//! Hush's Spotify Connect device: librespot session, Spirc and player.

mod device;
mod event;

pub use device::{Device, Events};

/// Ways the librespot session or player can fail.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("librespot: {0}")]
    Librespot(#[from] librespot_core::Error),
    #[error("session login: {0}")]
    Login(Box<dyn std::error::Error + Send + Sync>),
    #[error("task: {0}")]
    Task(#[from] tokio::task::JoinError),
}
