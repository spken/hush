use std::{fmt, sync::Arc, time::Duration};

use kit::{Coalescer, Req};
use librespot_core::Session;
use model::{Entity, List};
use store::Store;
use tokio::{
    sync::{Semaphore, mpsc},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;
use webapi::ApiGateway;

use crate::{
    Error,
    source::{CatalogSource, Fetch},
    spclient::Spclient,
    web_api::WebApi,
};

/// Catalog lookups that may run at once, to stay polite to spclient.
const CONCURRENCY: usize = 6;
/// How long an artist's top tracks and albums are shown before they are fetched again.
const ARTIST_FRESH_FOR: Duration = Duration::from_hours(7 * 24);

/// What the catalog service is built from.
pub struct Deps {
    pub store: Store,
    /// The librespot session, for spclient; `None` to use the Web API only.
    pub session: Option<Session>,
    /// The Web API, the fallback source.
    pub gateway: Option<ApiGateway>,
}

impl fmt::Debug for Deps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Deps")
            .field("spclient", &self.session.is_some())
            .field("web_api", &self.gateway.is_some())
            .finish_non_exhaustive()
    }
}

/// Sends requests to the catalog actor, which stops once every handle is dropped.
#[derive(Debug, Clone)]
pub struct CatalogService {
    tx: mpsc::Sender<Msg>,
}

#[derive(Debug)]
enum Msg {
    Fetch(Req<Result<bool, Error>>, List),
}

struct Ctx {
    store: Store,
    /// Tried in order until one answers.
    sources: Vec<Box<dyn CatalogSource>>,
    permits: Semaphore,
}

impl CatalogService {
    /// Starts the catalog actor on the current tokio runtime.
    pub fn spawn(deps: Deps) -> Self {
        let mut sources: Vec<Box<dyn CatalogSource>> = Vec::new();
        sources.extend(
            (deps.session)
                .map(|session| Box::new(Spclient::new(session)) as Box<dyn CatalogSource>),
        );
        sources.extend(
            (deps.gateway).map(|gateway| Box::new(WebApi::new(gateway)) as Box<dyn CatalogSource>),
        );
        let ctx = Ctx {
            store: deps.store,
            sources,
            permits: Semaphore::new(CONCURRENCY),
        };
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(run(Arc::new(ctx), rx));
        Self { tx }
    }

    /// Makes sure `list` is in the database: an album's tracks, an artist's top tracks and
    /// albums, or a playlist's tracks. Library lists are left to library sync. Returns
    /// whether it stored anything.
    pub async fn fetch(&self, list: List, cancel: CancellationToken) -> Result<bool, Error> {
        kit::call(&self.tx, cancel, |req| Msg::Fetch(req, list)).await
    }
}

async fn run(ctx: Arc<Ctx>, mut rx: mpsc::Receiver<Msg>) {
    let mut waiting = Coalescer::default();
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            message = rx.recv() => match message {
                Some(Msg::Fetch(req, list)) => {
                    if !req.is_cancelled() && waiting.join(list, req) {
                        let ctx = ctx.clone();
                        tasks.spawn(async move { (list, fetch(&ctx, list).await) });
                    }
                }
                None => break,
            },
            Some(done) = tasks.join_next() => match done {
                Ok((list, result)) => waiting.finish(&list, result),
                Err(error) => tracing::error!(%error, "catalog task failed"),
            },
        }
    }
}

/// Looks `list` up in the database, then in each source in turn, and stores what a source
/// returns. Returns whether it stored anything.
async fn fetch(ctx: &Ctx, list: List) -> Result<bool, Error> {
    let store = &ctx.store;
    match list {
        List::Album(key) => {
            if store.cursor(list).await?.is_some() {
                return Ok(false);
            }
            let id = store
                .id(Entity::Album, key)
                .await?
                .ok_or(Error::Unavailable)?;
            store
                .save_album(ctx.first(|source| source.album(id)).await?)
                .await?;
        }
        List::ArtistTop(key) | List::ArtistAlbums(key) => {
            let stored = store.cursor(List::ArtistTop(key)).await?;
            if stored
                .as_ref()
                .is_some_and(|cursor| cursor.age < ARTIST_FRESH_FOR)
            {
                return Ok(false);
            }
            let id = (store.id(Entity::Artist, key).await?).ok_or(Error::Unavailable)?;
            match ctx.first(|source| source.artist(id)).await {
                Ok(artist) => store.save_artist(key, artist).await?,
                // The stored artist is a week old, not wrong.
                Err(error) if stored.is_some() => {
                    tracing::warn!(%error, "artist not refreshed; showing the stored one");
                    return Ok(false);
                }
                Err(error) => return Err(error),
            }
        }
        List::Playlist(key) => {
            let Some(playlist) = store.stale_playlist(key).await? else {
                return Ok(false);
            };
            let id = playlist.id.parse().map_err(|_| Error::Unavailable)?;
            let tracks = ctx.first(|source| source.playlist(id)).await?;
            store
                .save_playlist_tracks(key, tracks, playlist.snapshot)
                .await?;
        }
        List::LikedSongs | List::SavedAlbums | List::Playlists => return Ok(false),
    }
    Ok(true)
}

impl Ctx {
    /// What the first source that answers returns, or the last source's error.
    async fn first<T>(
        &self,
        get: impl for<'a> Fn(&'a dyn CatalogSource) -> Fetch<'a, T>,
    ) -> Result<T, Error> {
        let _permit = self
            .permits
            .acquire()
            .await
            .map_err(|_| Error::Unavailable)?;
        let mut failed = Error::Unavailable;
        for source in &self.sources {
            match get(source.as_ref()).await {
                Ok(found) => return Ok(found),
                Err(error) => {
                    tracing::warn!(%error, "catalog source failed");
                    failed = error;
                }
            }
        }
        Err(failed)
    }
}
