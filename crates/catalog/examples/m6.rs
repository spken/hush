//! Checks the catalog's exit criteria on a scratch database: 5 artist pages and 20 album
//! pages from spclient cost no Web API requests; with spclient off the same pages load
//! from the Web API; and Play on an album and a playlist starts them on Hush. Uses the
//! stored logins, so run Hush once first, and close it: the device here is "Hush" too.
//!
//! `cargo run -p catalog --example m6`

use std::{env, fs, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context as _, ensure};
use auth::TokenProvider;
use catalog::{CatalogService, Deps};
use model::{
    AlbumKind, Context, Entity, Event, List, SpotifyId,
    catalog::{Album, Credit},
};
use playback::{Device, Events};
use store::Store;
use tokio::{sync::mpsc, time};
use tokio_util::sync::CancellationToken;
use webapi::ApiGateway;

/// Radiohead, Daft Punk, Kendrick Lamar, Miles Davis and Fleetwood Mac.
const ARTISTS: [&str; 5] = [
    "4Z8W4fKeB5YxbusRsdQVPb",
    "4tZwfgrHOc3mvqYlEYSvVi",
    "2YZyLoL8N0Wb9xBt1NhZWg",
    "0kbYTNQb4Pb1rPbbaF0pT4",
    "08GQAI4eElDnROBrJRGE0X",
];
const ALBUMS_PER_ARTIST: usize = 4;
/// OK Computer.
const ALBUM: &str = "6dVIqQ8qmQ5GBnJ9shOYGE";
/// Today's Top Hits, an editorial playlist.
const PLAYLIST: &str = "37i9dQZF1DXcBWIGoYBM5M";

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();
    dotenvy::dotenv().context(".env")?;
    let client_id = env::var("HUSH_CLIENT_ID").context("HUSH_CLIENT_ID is not set")?;
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let tokens = Arc::new(TokenProvider::new(&client_id, http.clone()).await?);
    let cache_dir =
        PathBuf::from(env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is not set")?).join("Hush");
    let (device, mut events) = Device::start(cache_dir, auth::session_token(&http)).await?;

    let store = scratch("hush-m6-spclient.db")?;
    let gateway = spawn_gateway(&http, &tokens, &store);
    let catalog = CatalogService::spawn(Deps {
        store: store.clone(),
        session: Some(device.session()),
        gateway: Some(gateway.clone()),
    });
    let pages = browse(&catalog, &store).await?;
    let requests = gateway.metrics().borrow().requests;
    tracing::info!(pages, requests, "spclient catalog");
    ensure!(
        requests == 0,
        "spclient pages cost {requests} Web API requests"
    );
    tracing::info!("pass: 5 artist and 20 album pages cost 0 Web API requests");

    let store = scratch("hush-m6-web-api.db")?;
    let gateway = spawn_gateway(&http, &tokens, &store);
    let catalog = CatalogService::spawn(Deps {
        store: store.clone(),
        session: None,
        gateway: Some(gateway.clone()),
    });
    let pages = browse(&catalog, &store).await?;
    let requests = gateway.metrics().borrow().requests;
    tracing::info!(pages, requests, "Web API catalog");
    tracing::info!("pass: with spclient off the same pages load from the Web API");

    device.activate()?;
    for context in [
        Context::Album(ALBUM.parse()?),
        Context::Playlist(PLAYLIST.parse()?),
    ] {
        device.play(context.clone(), None, false)?;
        let track = started(&mut events).await?;
        tracing::info!(?context, track, "playing on Hush");
    }
    tracing::info!("pass: Play started an album and a playlist on Hush");
    device.shutdown().await?;
    Ok(())
}

fn scratch(name: &str) -> anyhow::Result<Store> {
    let db = env::temp_dir().join(name);
    if db.exists() {
        fs::remove_file(&db)?;
    }
    Ok(Store::open(&db)?)
}

fn spawn_gateway(http: &reqwest::Client, tokens: &Arc<TokenProvider>, store: &Store) -> ApiGateway {
    let (events, _) = mpsc::channel(8);
    ApiGateway::spawn(webapi::Deps {
        http: http.clone(),
        tokens: tokens.clone(),
        store: store.clone(),
        base_url: webapi::BASE_URL.into(),
        events,
    })
}

/// Opens 5 artist pages and 4 albums from each, the way the app does, and returns how
/// many pages loaded.
async fn browse(catalog: &CatalogService, store: &Store) -> anyhow::Result<usize> {
    let mut pages = 0;
    for id in ARTISTS {
        let key = artist_key(store, id.parse()?).await?;
        catalog
            .fetch(List::ArtistTop(key), CancellationToken::new())
            .await?;
        let albums = store.list_keys(List::ArtistAlbums(key)).await?;
        ensure!(!albums.is_empty(), "artist {id} has no albums");
        pages += 1;
        for album in albums.into_iter().take(ALBUMS_PER_ARTIST) {
            catalog
                .fetch(List::Album(album), CancellationToken::new())
                .await?;
            let tracks = store.list_len(List::Album(album)).await?;
            ensure!(tracks > 0, "album {album} has no tracks");
            pages += 1;
        }
    }
    Ok(pages)
}

/// Stores the artist `id` the way a track or album credit does, and returns its key.
async fn artist_key(store: &Store, id: SpotifyId) -> anyhow::Result<u32> {
    let credit = Album {
        id,
        name: "".into(),
        artists: vec![Credit {
            id,
            name: "".into(),
        }],
        year: None,
        kind: AlbumKind::Album,
        tracks: Vec::new(),
    };
    let keys = store.save_albums(vec![credit]).await?;
    let row = store.album_rows(keys).await?.pop().context("seed album")?;
    let key = row.artist_key.context("seed artist")?;
    ensure!(store.id(Entity::Artist, key).await? == Some(id));
    Ok(key)
}

/// Waits for the next track Hush loads, and returns its name.
async fn started(events: &mut Events) -> anyhow::Result<Box<str>> {
    time::timeout(Duration::from_secs(20), async {
        loop {
            if let Event::TrackChanged(track) = events.recv().await.context("the player stopped")? {
                return Ok(track.name);
            }
        }
    })
    .await
    .context("nothing started playing within 20 s")?
}
