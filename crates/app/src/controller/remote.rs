//! The Controller's remote side: what plays on another Spotify Connect device, read from
//! the Web API only when worth it, transport controls sent there, and moving playback
//! between devices.

use std::{mem, time::Duration};

use model::{Event, SpotifyId, Track, TrackId, web};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use webapi::Control;

use super::Controller;

/// How long after a command the remote device is read again, so it has acted on it.
const AFTER_COMMAND: Duration = Duration::from_secs(1);
/// How long after the estimated end of a track the next one is read.
const AFTER_TRACK_END: Duration = Duration::from_millis(1500);
/// How often the remote device is read while the window can be seen.
const HEARTBEAT: Duration = Duration::from_mins(1);
/// Reads, `AFTER_COMMAND` apart, that may find nothing after playback moved before the
/// heartbeat takes over: Spotify takes a moment to report the new device.
const SETTLE_READS: u8 = 4;

/// What the Controller knows about playback on other devices.
#[derive(Debug)]
pub(super) struct Remote {
    /// The active device, when it is not Hush, as last read.
    pub(super) device: Option<Box<str>>,
    /// Whether the window can be seen; nothing is read while it is minimized.
    pub(super) visible: bool,
    /// When to read the active device next.
    pub(super) poll_at: Option<Instant>,
    /// Whether a read is on its way.
    polling: bool,
    /// Reads left that retry soon when they find nothing.
    settling: u8,
    /// Whether playback moves back to Hush once the reads find no other device playing:
    /// Hush was playing when it lost its session or stopped being the active device.
    pub(super) resume: bool,
}

impl Remote {
    pub(super) fn new() -> Self {
        Self {
            device: None,
            visible: true,
            poll_at: None,
            polling: false,
            settling: 0,
            resume: false,
        }
    }
}

impl Controller {
    /// Whether transport controls go to another device through the Web API.
    pub(super) fn remote_active(&self) -> bool {
        !self.active && self.remote.device.is_some()
    }

    /// Reads the active device after `delay`, unless Hush is the active device, the window
    /// is minimized with nothing waiting to resume, or a sooner read is due.
    pub(super) fn poll_in(&mut self, delay: Duration) {
        if self.active || !(self.remote.visible || self.remote.resume) || self.library.is_none() {
            return;
        }
        let at = Instant::now() + delay;
        self.remote.poll_at = Some(self.remote.poll_at.map_or(at, |due| due.min(at)));
    }

    /// Reads the active device soon and keeps reading it until it reports what plays,
    /// after playback moved between devices.
    pub(super) fn settle(&mut self) {
        self.remote.settling = SETTLE_READS;
        self.poll_in(AFTER_COMMAND);
    }

    /// Hush stopped being the active device. Unless Hush moved playback away itself, which
    /// left reads settling, what played here resumes if no other device takes it.
    pub(super) fn deactivated(&mut self) {
        self.note("Hush stopped being the active device");
        self.remote.resume = self.anchor.is_playing() && self.remote.settling == 0;
        self.settle();
    }

    /// Reads what plays on the active device; the answer comes back as `Event::Polled`.
    pub(super) fn poll(&mut self) {
        self.remote.poll_at = None;
        let Some(library) = &self.library else { return };
        if self.active || self.remote.polling {
            return;
        }
        self.remote.polling = true;
        let gateway = library.gateway.clone();
        let events = self.events.clone();
        tokio::spawn(async move {
            let player = gateway
                .player(CancellationToken::new())
                .await
                .inspect_err(|error| tracing::warn!(%error, "remote player not read"));
            // Fails only once the Controller stopped, and then nothing waits for this.
            let _ = events.send(Event::Polled(player.ok().flatten())).await;
        });
    }

    /// Shows what the active device plays, and plans the next read. Playback waiting to
    /// resume moves back to Hush once the reads after it stopped find no other device.
    pub(super) fn polled(&mut self, player: Option<web::Player>) {
        self.remote.polling = false;
        if self.active {
            return;
        }
        let device = player.as_ref().and_then(|player| player.device.as_ref());
        let own: Option<Box<str>> = self.device.as_ref().map(|device| device.id().into());
        let device = device.filter(|device| device.id.as_deref() != own.as_deref());
        self.remote.device = device.and_then(|device| device.name.clone());
        self.view.remote(self.remote.device.clone());
        let moved = self.remote.device.is_some() && mem::take(&mut self.remote.resume);
        match &self.remote.device {
            Some(name) if moved => {
                let note = format!("Playback moved to {name}");
                self.note(&note);
            }
            _ => self.show_diagnostics(),
        }
        let track = (player.as_ref())
            .filter(|_| self.remote.device.is_some())
            .and_then(|player| track(player.item.as_ref()?));
        let Some((player, track)) = player.zip(track) else {
            if self.remote.settling > 0 {
                self.remote.settling -= 1;
                self.poll_in(AFTER_COMMAND);
            } else {
                if mem::take(&mut self.remote.resume) {
                    self.note("No other device playing; resuming on Hush");
                    if let Some(own) = own {
                        self.move_playback(own, true);
                    }
                }
                self.poll_in(HEARTBEAT);
            }
            // The track stays on screen while it may still resume here.
            if moved {
                self.show_stopped();
            } else if !self.remote.resume {
                self.view.track(None);
            }
            return;
        };
        self.remote.settling = 0;
        let position_ms = player.progress_ms.unwrap_or(0);
        let playing = player.is_playing.unwrap_or(false);
        let event = if playing {
            Event::Playing { position_ms }
        } else {
            Event::Paused { position_ms }
        };
        self.anchor.apply(&event, std::time::Instant::now());
        if let Some(shuffle) = player.shuffle_state {
            self.view.shuffle(shuffle);
        }
        if let Some(volume) = player.device.and_then(|device| device.volume_percent) {
            self.view.volume(percent_to_volume(volume));
        }
        let left = Duration::from_millis(track.duration_ms.saturating_sub(position_ms).into());
        self.show_backdrop(&track.art);
        self.view.track(Some(track));
        self.view.progress(self.anchor);
        self.read_queue(Duration::ZERO);
        self.poll_in(HEARTBEAT);
        if playing {
            self.poll_in(left + AFTER_TRACK_END);
        }
    }

    /// Sends `control` to the active device, then reads it again.
    pub(super) fn control_remote(&mut self, control: Control) {
        let Some(library) = &self.library else { return };
        let gateway = library.gateway.clone();
        tokio::spawn(async move {
            if let Err(error) = gateway.control(control, CancellationToken::new()).await {
                tracing::warn!(%error, ?control, "remote control failed");
            }
        });
        self.poll_in(AFTER_COMMAND);
    }

    /// Lists the Spotify Connect devices for the device picker.
    pub(super) fn list_devices(&self) {
        let Some(library) = &self.library else { return };
        let gateway = library.gateway.clone();
        let view = self.view.clone();
        let own = self.device.as_ref().map(|device| device.id().into());
        tokio::spawn(async move {
            match gateway.devices(CancellationToken::new()).await {
                Ok(devices) => view.devices(devices.devices, own),
                Err(error) => tracing::warn!(%error, "devices not listed"),
            }
        });
    }

    /// Moves playback to the device with ID `id`, playing there only if it plays now: Hush
    /// itself through Spotify Connect, at no request, or another device through the Web API.
    pub(super) fn transfer(&mut self, id: &str) {
        if self.device.as_ref().is_some_and(|device| device.id() == id) {
            self.playback(playback::Device::activate);
            return;
        }
        self.move_playback(id.into(), self.anchor.is_playing());
        self.settle();
    }

    /// Moves playback to the device with ID `id` through the Web API, playing there when
    /// `play` is set.
    fn move_playback(&self, id: Box<str>, play: bool) {
        let Some(library) = &self.library else { return };
        let gateway = library.gateway.clone();
        tokio::spawn(async move {
            if let Err(error) = gateway.transfer(&id, play, CancellationToken::new()).await {
                tracing::warn!(%error, "playback not moved");
            }
        });
    }
}

/// A Web API track as the now-playing view shows it; `None` for an episode or an ad.
fn track(item: &web::Track) -> Option<Track> {
    let id: SpotifyId = item.id.as_deref()?.parse().ok()?;
    Some(Track {
        id: TrackId(id),
        name: item.name.clone().unwrap_or_default(),
        artists: (item.artists.iter())
            .filter_map(|artist| artist.name.clone())
            .collect(),
        duration_ms: item.duration_ms.unwrap_or(0),
        art: (item.album.as_ref())
            .map(|album| web::Image::art(&album.images))
            .unwrap_or_default(),
    })
}

/// A Web API volume in percent as a device volume, from 0 to `u16::MAX`.
fn percent_to_volume(percent: u32) -> u16 {
    u16::try_from(u32::from(u16::MAX) * percent.min(100) / 100).unwrap_or(u16::MAX)
}

/// A device volume, from 0 to `u16::MAX`, in percent.
pub(super) fn volume_to_percent(volume: u16) -> u8 {
    u8::try_from(u32::from(volume) * 100 / u32::from(u16::MAX)).unwrap_or(100)
}
