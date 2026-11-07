//! Logs in, forces a token refresh, plays one track to the end on the default audio
//! device, then logs memory use.
//!
//! `cargo run -p playback --example m1 [-- spotify:track:<id>]`

use std::{env, path::PathBuf};

use anyhow::Context;
use auth::TokenProvider;
use model::TrackId;

const TRACK: &str = "spotify:track:4uLU6hMCjMI75M1A2tKUQC";

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();
    dotenvy::dotenv().ok();
    let client_id = env::var("HUSH_CLIENT_ID").context("HUSH_CLIENT_ID is not set")?;
    let track: TrackId = env::args().nth(1).as_deref().unwrap_or(TRACK).parse()?;
    let cache_dir =
        PathBuf::from(env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is not set")?).join("Hush");

    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let tokens = TokenProvider::new(&client_id, http.clone()).await?;
    tokens.expire().await;
    tokens.access_token().await.context("forced refresh")?;

    let session = playback::connect(cache_dir, auth::session_token(&http)).await?;
    tracing::info!(user = session.username(), "session ready");

    let player = playback::player(session);
    playback::play(&player, track);
    player.await_end_of_track().await;

    let memory = memory_stats::memory_stats().context("memory stats unavailable")?;
    tracing::info!(
        working_set_mb = memory.physical_mem / 1_000_000,
        private_mb = memory.virtual_mem / 1_000_000,
        "track ended"
    );
    Ok(())
}
