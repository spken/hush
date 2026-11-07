//! Registers "Hush" as a Spotify Connect device, then logs its playback events and, every
//! 250 ms while it moves, the interpolated position until Ctrl+C. Enter toggles a +6 dB
//! boost at 1 kHz. Logs memory use on exit.
//!
//! `cargo run -p playback --example m2`

use std::{
    env,
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::Context;
use model::Anchor;
use playback::{Device, EQ_BANDS_HZ};
use tokio::io::{AsyncBufReadExt, BufReader};

const TICK: Duration = Duration::from_millis(250);
const BOOST_HZ: u16 = 1000;
const BOOST_DB: f64 = 6.0;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();
    let cache_dir =
        PathBuf::from(env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is not set")?).join("Hush");
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;

    let (device, mut events) = Device::start(cache_dir, auth::session_token(&http)).await?;
    tracing::info!("\"Hush\" is in the Spotify Connect device list");

    let mut anchor = Anchor::new(Instant::now());
    let mut boosted = false;
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut logged = None;
    let mut tick = tokio::time::interval(TICK);
    let stop = tokio::signal::ctrl_c();
    tokio::pin!(stop);
    loop {
        tokio::select! {
            event = events.recv() => {
                let Some(event) = event else { break };
                anchor.apply(&event, Instant::now());
                tracing::info!(?event, "player event");
            }
            _ = tick.tick() => {
                let position_ms = anchor.position_at(Instant::now());
                if logged.replace(position_ms) != Some(position_ms) {
                    tracing::info!(position = clock(position_ms));
                }
            }
            Ok(Some(_)) = lines.next_line() => {
                boosted = !boosted;
                let gains = EQ_BANDS_HZ.map(|hz| if boosted && hz == BOOST_HZ { BOOST_DB } else { 0.0 });
                device.set_eq(0.0, &gains);
                tracing::info!(boosted, "1 kHz boost");
            }
            _ = &mut stop => break,
        }
    }
    device.shutdown().await?;

    let memory = memory_stats::memory_stats().context("memory stats unavailable")?;
    tracing::info!(
        working_set_mb = memory.physical_mem / 1_000_000,
        private_mb = memory.virtual_mem / 1_000_000,
        "device stopped"
    );
    Ok(())
}

/// Formats a position like the Spotify clients do, with milliseconds.
fn clock(position_ms: u32) -> String {
    let seconds = position_ms / 1000;
    format!(
        "{}:{:02}.{:03}",
        seconds / 60,
        seconds % 60,
        position_ms % 1000
    )
}
