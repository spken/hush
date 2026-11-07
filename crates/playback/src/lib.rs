//! librespot session login and audio playback.

use std::{path::PathBuf, sync::Arc};

use librespot_core::{
    Session, SessionConfig, SpotifyId, SpotifyUri, authentication::Credentials, cache::Cache,
};
use librespot_playback::{
    audio_backend,
    config::{AudioFormat, PlayerConfig},
    mixer::NoOpVolume,
    player::Player,
};
use model::TrackId;

/// Ways the librespot session or player can fail.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("librespot: {0}")]
    Librespot(#[from] librespot_core::Error),
    #[error("session login: {0}")]
    Login(Box<dyn std::error::Error + Send + Sync>),
    #[error("blocking task: {0}")]
    Task(#[from] tokio::task::JoinError),
}

/// Logs a session in with the reusable credentials librespot cached in `cache_dir`, or,
/// when there are none, with the access token `login` produces. `login` is only awaited
/// in that case.
pub async fn connect<E>(
    cache_dir: PathBuf,
    login: impl Future<Output = Result<String, E>>,
) -> Result<Session, Error>
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
        None => Credentials::with_access_token(login.await.map_err(|e| Error::Login(e.into()))?),
    };
    let session = Session::new(SessionConfig::default(), Some(cache));
    session.connect(credentials, true).await?;
    Ok(session)
}

/// Creates a player on the default audio device.
pub fn player(session: Session) -> Arc<Player> {
    // librespot lists its default backend (rodio) first.
    let (_, backend) = audio_backend::BACKENDS[0];
    Player::new(
        PlayerConfig::default(),
        session,
        Box::new(NoOpVolume),
        move || backend(None, AudioFormat::default()),
    )
}

/// Loads `track` into `player` and starts playing it.
pub fn play(player: &Player, track: TrackId) {
    let id = SpotifyId { id: track.0.0 };
    player.load(SpotifyUri::Track { id }, true, 0);
}
