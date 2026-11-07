use std::{fmt, path::PathBuf, sync::Arc};

use arc_swap::ArcSwap;
use librespot_connect::{ConnectConfig, Spirc};
use librespot_core::{
    Session, SessionConfig, authentication::Credentials, cache::Cache, config::DeviceType,
};
use librespot_playback::{
    config::{Bitrate, PlayerConfig},
    mixer::{Mixer, MixerConfig, NoOpVolume, softmixer::SoftMixer},
    player::{Player, PlayerEventChannel},
};
use model::Event;
use tokio::task::JoinHandle;

use crate::{
    EQ_BANDS_HZ, Error,
    eq::{self, Coefficients},
    event,
    sink::EqSink,
};

const NAME: &str = "Hush";

/// Hush's Spotify Connect device: a librespot player that Hush and other Spotify clients
/// control.
pub struct Device {
    spirc: Spirc,
    task: JoinHandle<()>,
    eq: Arc<ArcSwap<Coefficients>>,
}

/// The device's playback events, in the order they happened.
pub struct Events(PlayerEventChannel);

impl Device {
    /// Logs a session in and registers the device as "Hush". The session uses the reusable
    /// credentials librespot cached in `cache_dir`, or, when there are none, the access
    /// token `login` produces; `login` is only awaited in that case.
    pub async fn start<E>(
        cache_dir: PathBuf,
        login: impl Future<Output = Result<String, E>>,
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
        let session = Session::new(SessionConfig::default(), Some(cache));

        let eq = Arc::new(ArcSwap::from_pointee(eq::coefficients(0.0, &[0.0; _])));
        let mixer = Arc::new(SoftMixer::open(MixerConfig::default())?);
        // The player leaves volume alone; `EqSink` applies it after the EQ.
        let (sink_eq, volume) = (eq.clone(), mixer.get_soft_volume());
        let config = PlayerConfig {
            bitrate: Bitrate::Bitrate320,
            normalisation: true,
            ditherer: None,
            ..PlayerConfig::default()
        };
        let player = Player::new(config, session.clone(), Box::new(NoOpVolume), move || {
            Box::new(EqSink::new(sink_eq, volume))
        });
        let events = Events(player.get_player_event_channel());

        let connect = ConnectConfig {
            name: NAME.to_owned(),
            device_type: DeviceType::Computer,
            ..ConnectConfig::default()
        };
        let (spirc, run) = Spirc::new(connect, session, credentials, player, mixer).await?;
        let task = tokio::spawn(run);
        Ok((Self { spirc, task, eq }, events))
    }

    /// Sets the equalizer: a preamp and one gain per band in `EQ_BANDS_HZ`, all in dB.
    /// The change is heard within ~120 ms, ramped so it does not click.
    pub fn set_eq(&self, preamp_db: f64, gains_db: &[f64; EQ_BANDS_HZ.len()]) {
        self.eq
            .store(Arc::new(eq::coefficients(preamp_db, gains_db)));
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
