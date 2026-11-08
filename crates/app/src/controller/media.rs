//! The Windows media overlay (volume flyout, lock screen) and the keyboard's media keys,
//! through the System Media Transport Controls.

use std::time::{Duration, Instant};

use model::{Anchor, Art, Command, Track};
use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition};
use tokio::sync::mpsc::WeakSender;

/// The cover size the overlay shows.
const COVER_PX: u32 = 300;

/// Hush's entry in the media overlay. Button presses arrive as `Command`s.
pub(super) struct Overlay {
    controls: MediaControls,
}

impl Overlay {
    /// Takes over `controls`, sending what their buttons ask for to `commands` for as long
    /// as the Controller runs.
    pub(super) fn new(mut controls: MediaControls, commands: WeakSender<Command>) -> Option<Self> {
        let attached = controls.attach(move |event| {
            let command = match event {
                MediaControlEvent::Play => Command::SetPlaying(true),
                MediaControlEvent::Pause | MediaControlEvent::Stop => Command::SetPlaying(false),
                MediaControlEvent::Toggle => Command::PlayPause,
                MediaControlEvent::Next => Command::Next,
                MediaControlEvent::Previous => Command::Previous,
                MediaControlEvent::SetPosition(MediaPosition(position)) => Command::Seek {
                    position_ms: u32::try_from(position.as_millis()).unwrap_or(u32::MAX),
                },
                _ => return,
            };
            if let Some(commands) = commands.upgrade()
                && let Err(error) = commands.try_send(command)
            {
                tracing::warn!(%error, "media key dropped");
            }
        });
        match attached {
            Ok(()) => Some(Self { controls }),
            Err(error) => {
                tracing::warn!(%error, "media overlay unavailable");
                None
            }
        }
    }

    /// Shows the loaded track, or that nothing is loaded.
    pub(super) fn track(&mut self, track: Option<&Track>) {
        let Some(track) = track else {
            self.playback(MediaPlayback::Stopped);
            return;
        };
        let artists = track.artists.join(", ");
        let metadata = MediaMetadata {
            title: Some(&track.name),
            artist: Some(&artists),
            album: None,
            cover_url: Art::url(track.art.as_str(), COVER_PX),
            duration: Some(Duration::from_millis(track.duration_ms.into())),
        };
        if let Err(error) = self.controls.set_metadata(metadata) {
            tracing::warn!(%error, "media overlay not updated");
        }
    }

    /// Shows whether the track plays, and where it is.
    pub(super) fn progress(&mut self, anchor: Anchor, now: Instant) {
        let progress = Some(MediaPosition(Duration::from_millis(
            anchor.position_at(now).into(),
        )));
        self.playback(if anchor.is_playing() {
            MediaPlayback::Playing { progress }
        } else {
            MediaPlayback::Paused { progress }
        });
    }

    fn playback(&mut self, playback: MediaPlayback) {
        if let Err(error) = self.controls.set_playback(playback) {
            tracing::warn!(%error, "media overlay not updated");
        }
    }
}
