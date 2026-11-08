use std::{fmt, path::PathBuf, sync::Arc};

use arc_swap::ArcSwap;
use librespot_connect::{
    ConnectConfig, LoadContextOptions, LoadRequest, LoadRequestOptions, Options, PlayingTrack,
    Spirc,
};
use librespot_core::{
    Session, SessionConfig, authentication::Credentials, cache::Cache, config::DeviceType,
};
use librespot_playback::{
    config::{Bitrate, PlayerConfig},
    mixer::{Mixer, MixerConfig, NoOpVolume, softmixer::SoftMixer},
    player::{Player, PlayerEventChannel},
};
use model::{Context, Event, Playback, SpotifyId, TrackId};
use tokio::task::JoinHandle;

use crate::{
    Error,
    eq::{self, Coefficients},
    event,
    sink::{EqSink, OutputChoice},
};

const NAME: &str = "Hush";

/// Hush's Spotify Connect device: a librespot player that Hush and other Spotify clients
/// control.
pub struct Device {
    spirc: Spirc,
    session: Session,
    task: JoinHandle<()>,
    eq: Arc<ArcSwap<Coefficients>>,
    output: OutputChoice,
}

/// The device's playback events, in the order they happened.
pub struct Events(PlayerEventChannel);

impl Device {
    /// Logs a session in and registers the device as "Hush", playing as `settings` say.
    /// The session uses the reusable credentials librespot cached in `cache_dir`, or, when
    /// there are none, the access token `login` produces; `login` is only awaited in that
    /// case. Normalisation and gapless are fixed for the device's life. Given an `id`, the
    /// device keeps it, so Spotify sees a reconnect as the same device; otherwise it gets a
    /// new one.
    pub async fn start<E>(
        cache_dir: PathBuf,
        login: impl Future<Output = Result<String, E>>,
        settings: &Playback,
        id: Option<&str>,
    ) -> Result<(Self, Events), Error>
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        let (cache, cached) = tokio::task::spawn_blocking(move || {
            let cache = Cache::new(Some(cache_dir), None, None, None)?;
            let cached = cache.credentials();
            Ok::<_, librespot_core::Error>((cache, cached))
        })
        .await??;
        let credentials = match cached {
            Some(credentials) => credentials,
            None => {
                Credentials::with_access_token(login.await.map_err(|e| Error::Login(e.into()))?)
            }
        };
        let mut config = SessionConfig::default();
        if let Some(id) = id {
            id.clone_into(&mut config.device_id);
        }
        let session = Session::new(config, Some(cache));

        let eq = Arc::new(ArcSwap::from_pointee(equalizer(settings)));
        let output: OutputChoice = Arc::new(ArcSwap::from_pointee(settings.output.clone()));
        let mixer = Arc::new(SoftMixer::open(MixerConfig::default())?);
        // The player leaves volume alone; `EqSink` applies it after the EQ.
        let (sink_eq, sink_output, volume) = (eq.clone(), output.clone(), mixer.get_soft_volume());
        let config = PlayerConfig {
            bitrate: Bitrate::Bitrate320,
            normalisation: settings.normalisation,
            gapless: settings.gapless,
            ditherer: None,
            ..PlayerConfig::default()
        };
        let player = Player::new(config, session.clone(), Box::new(NoOpVolume), move || {
            Box::new(EqSink::new(sink_eq, volume, sink_output))
        });
        let events = Events(player.get_player_event_channel());

        let connect = ConnectConfig {
            name: NAME.to_owned(),
            device_type: DeviceType::Computer,
            ..ConnectConfig::default()
        };
        let (spirc, run) = Spirc::new(connect, session.clone(), credentials, player, mixer).await?;
        let task = tokio::spawn(run);
        let device = Self {
            spirc,
            session,
            task,
            eq,
            output,
        };
        Ok((device, events))
    }

    /// The device's Spotify Connect ID, as the Web API lists it.
    pub fn id(&self) -> &str {
        self.session.device_id()
    }

    /// The device's logged-in librespot session, for metadata lookups.
    pub fn session(&self) -> Session {
        self.session.clone()
    }

    /// Applies the equalizer and output device of `settings`. An equalizer change is heard
    /// within ~120 ms, ramped so it does not click; a new device takes over with the next
    /// audio written.
    pub fn apply(&self, settings: &Playback) {
        self.eq.store(Arc::new(equalizer(settings)));
        if **self.output.load() != settings.output {
            self.output.store(Arc::new(settings.output.clone()));
        }
    }

    /// Makes Hush the active Spotify Connect device, so it can play. Does nothing while it
    /// already is.
    pub fn activate(&self) -> Result<(), Error> {
        Ok(self.spirc.activate()?)
    }

    /// Plays `context` on Hush, which must be active: from the track `start`, or from its
    /// top; shuffled or in order.
    pub fn play(
        &self,
        context: Context,
        start: Option<TrackId>,
        shuffle: bool,
    ) -> Result<(), Error> {
        let options = LoadRequestOptions {
            start_playing: true,
            seek_to: 0,
            context_options: Some(LoadContextOptions::Options(Options {
                shuffle,
                ..Options::default()
            })),
            playing_track: start.map(|TrackId(id)| PlayingTrack::Uri(uri("track", id))),
        };
        let request = match context {
            Context::LikedSongs => LoadRequest::from_context_uri(
                format!("spotify:user:{}:collection", self.session.username()),
                options,
            ),
            Context::Album(id) => LoadRequest::from_context_uri(uri("album", id), options),
            Context::Artist(id) => LoadRequest::from_context_uri(uri("artist", id), options),
            Context::Playlist(id) => LoadRequest::from_context_uri(uri("playlist", id), options),
            Context::Tracks(ids) => {
                LoadRequest::from_tracks(ids.iter().map(|&id| uri("track", id)).collect(), options)
            }
        };
        Ok(self.spirc.load(request)?)
    }

    /// Pauses when playing, resumes when paused. Like every transport control, it does
    /// nothing while another device is active.
    pub fn play_pause(&self) -> Result<(), Error> {
        Ok(self.spirc.play_pause()?)
    }

    /// Skips to the next track.
    pub fn next(&self) -> Result<(), Error> {
        Ok(self.spirc.next()?)
    }

    /// Restarts the track, or goes to the previous one within its first 3 seconds.
    pub fn previous(&self) -> Result<(), Error> {
        Ok(self.spirc.prev()?)
    }

    /// Jumps to `position_ms` in the current track.
    pub fn seek(&self, position_ms: u32) -> Result<(), Error> {
        Ok(self.spirc.set_position_ms(position_ms)?)
    }

    /// Sets the volume, from 0 to `u16::MAX`, and reports it to Spotify Connect.
    pub fn set_volume(&self, volume: u16) -> Result<(), Error> {
        Ok(self.spirc.set_volume(volume)?)
    }

    /// Turns shuffle on, or with `false` off.
    pub fn set_shuffle(&self, shuffle: bool) -> Result<(), Error> {
        Ok(self.spirc.shuffle(shuffle)?)
    }

    /// Pauses playback, removes the device from Spotify Connect and waits for it to stop.
    pub async fn shutdown(self) -> Result<(), Error> {
        self.spirc.shutdown()?;
        Ok(self.task.await?)
    }
}

impl fmt::Debug for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Device").finish_non_exhaustive()
    }
}

impl Events {
    /// Waits for the next event Hush uses; `None` once the player has shut down.
    pub async fn recv(&mut self) -> Option<Event> {
        loop {
            if let Some(event) = event::map(self.0.recv().await?) {
                return Some(event);
            }
        }
    }
}

impl fmt::Debug for Events {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Events").finish_non_exhaustive()
    }
}

/// The Spotify URI of the `kind` entity with ID `id`.
fn uri(kind: &str, id: SpotifyId) -> String {
    format!("spotify:{kind}:{id}")
}

/// The equalizer `settings` ask for; flat when it is off.
fn equalizer(settings: &Playback) -> Coefficients {
    if settings.eq {
        eq::coefficients(settings.preamp_db.into(), &settings.gains_db.map(f64::from))
    } else {
        eq::coefficients(0.0, &[0.0; _])
    }
}
