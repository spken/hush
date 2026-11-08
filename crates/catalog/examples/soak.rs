//! A long run of Hush's services the way a day of use drives them, logging Web API 429s and
//! memory every minute: the device stays registered, the library syncs when due, the
//! active device is read every minute (the remote heartbeat), and an artist and its albums
//! are browsed every 10 minutes. Runs on a copy of Hush's database, with the stored logins,
//! so run Hush once first, and close it: the device here is "Hush" too.
//!
//! `cargo run --release -p catalog --example soak -- [minutes, default 1440]`

use std::{env, fs, path::PathBuf, sync::Arc, time::Duration};

use anyhow::Context as _;
use auth::TokenProvider;
use catalog::{CatalogService, Deps, Library};
use model::{List, Playback, catalog::Credit};
use playback::Device;
use store::Store;
use tokio::{sync::mpsc, time};
use tokio_util::sync::CancellationToken;
use webapi::{ApiGateway, Lane};

const MINUTE: Duration = Duration::from_mins(1);
const BROWSE_EVERY: u32 = 10;
/// Artists browsed in turn: Radiohead, Daft Punk, Kendrick Lamar, Miles Davis.
const ARTISTS: [&str; 4] = [
    "4Z8W4fKeB5YxbusRsdQVPb",
    "4tZwfgrHOc3mvqYlEYSvVi",
    "2YZyLoL8N0Wb9xBt1NhZWg",
    "0kbYTNQb4Pb1rPbbaF0pT4",
];

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();
    dotenvy::dotenv().context(".env")?;
    let minutes: u32 = env::args().nth(1).map_or(Ok(1440), |arg| arg.parse())?;
    let client_id = env::var("HUSH_CLIENT_ID").context("HUSH_CLIENT_ID is not set")?;
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let tokens = Arc::new(TokenProvider::new(&client_id, http.clone()).await?);
    let data =
        PathBuf::from(env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is not set")?).join("Hush");
    let db = env::temp_dir().join("hush-soak.db");
    fs::copy(data.join("hush.db"), &db).context("copy Hush's database; is Hush closed?")?;
    let store = Store::open(&db)?;
    let (device, _events) =
        Device::start(data, auth::session_token(&http), &Playback::default()).await?;

    let (events, mut service_events) = mpsc::channel(64);
    let gateway = ApiGateway::spawn(webapi::Deps {
        http,
        tokens,
        store: store.clone(),
        base_url: webapi::BASE_URL.into(),
        events: events.clone(),
    });
    // Sync progress and list changes are not needed here.
    tokio::spawn(async move { while service_events.recv().await.is_some() {} });
    let user_id = gateway
        .me(Lane::P1, CancellationToken::new())
        .await?
        .id
        .context("profile has no ID")?;
    let library = Library {
        gateway: gateway.clone(),
        store: store.clone(),
        user_id,
        events,
    };
    let catalog = CatalogService::spawn(Deps {
        store: store.clone(),
        session: Some(device.session()),
        gateway: Some(gateway.clone()),
    });

    let mut first_kib = None;
    let mut peak_kib = 0;
    let mut tick = time::interval(MINUTE);
    for minute in 0..=minutes {
        tick.tick().await;
        if let Err(error) = catalog::sync(&library, false, &CancellationToken::new()).await {
            tracing::warn!(%error, "library sync failed");
        }
        if let Err(error) = gateway.player(CancellationToken::new()).await {
            tracing::warn!(%error, "player not read");
        }
        if minute % BROWSE_EVERY == 0 {
            let artist = ARTISTS[(minute / BROWSE_EVERY) as usize % ARTISTS.len()];
            if let Err(error) = browse(&catalog, &store, artist).await {
                tracing::warn!(%error, artist, "browse failed");
            }
        }
        let memory = memory_stats::memory_stats().context("memory stats unavailable")?;
        let private_kib = i64::try_from(memory.virtual_mem / 1024).unwrap_or(i64::MAX);
        let first = *first_kib.get_or_insert(private_kib);
        peak_kib = peak_kib.max(private_kib);
        let metrics = gateway.metrics().borrow().clone();
        tracing::info!(
            minute,
            private_kib,
            drift_kib = private_kib - first,
            requests = metrics.requests,
            rate_limited = metrics.rate_limited,
            quota_exceeded = metrics.quota_exceeded,
            "soak"
        );
    }
    tracing::info!(peak_kib, "soak done");
    device.shutdown().await?;
    Ok(())
}

/// Opens an artist page and its first albums the way the app does.
async fn browse(catalog: &CatalogService, store: &Store, artist: &str) -> anyhow::Result<()> {
    let credit = Credit {
        id: artist.parse()?,
        name: "".into(),
        art: model::Art::default(),
    };
    let key = store
        .save_artists(vec![credit])
        .await?
        .pop()
        .context("artist")?;
    catalog
        .fetch(List::ArtistTop(key), CancellationToken::new())
        .await?;
    for album in store
        .list_keys(List::ArtistAlbums(key))
        .await?
        .into_iter()
        .take(4)
    {
        catalog
            .fetch(List::Album(album), CancellationToken::new())
            .await?;
    }
    Ok(())
}
