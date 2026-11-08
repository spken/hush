//! The Controller's search side: library matches on every change of the search text,
//! Spotify's catalog after a pause in typing, and recent catalog results kept in memory.

use std::time::{Duration, Instant};

use kit::{ByteLru, Weigh};
use model::{Context, Entity, Event, Hits};
use tokio::time;
use tokio_util::sync::CancellationToken;

use super::Controller;
use crate::view::Loading;

/// How long typing must pause before Spotify's catalog is searched.
const PAUSE: Duration = Duration::from_millis(300);
/// Shorter text searches nothing.
const MIN_CHARS: usize = 2;
/// Library matches shown of each kind, above the catalog's.
const LIBRARY_HITS: u32 = 5;
/// How long catalog results are reused for the same query.
const FRESH_FOR: Duration = Duration::from_mins(10);
/// Memory for cached catalog results: about 200 queries' first pages.
const CACHE_BYTES: u64 = 64 * 1024;
const CACHED_BYTES: u64 = 320;
/// Spotify serves search results up to this offset.
const LAST_OFFSET: u32 = 1000;
/// Every kind of result in `Entity` order, so `entity as usize` indexes per-kind arrays.
const KINDS: [Entity; 4] = [
    Entity::Track,
    Entity::Album,
    Entity::Artist,
    Entity::Playlist,
];

/// What the search page shows, and the catalog requests behind it.
pub(super) struct Search {
    /// The search text with its spaces tidied; empty when too short to search for.
    query: Box<str>,
    /// `query` in lower case, which catalog results are asked for and cached by.
    key: Box<str>,
    library: [Vec<u32>; 4],
    catalog: [Hits; 4],
    /// Whether the catalog's first page for `key` is on its way.
    loading: Loading,
    /// Cancels the catalog requests for `key`.
    cancel: CancellationToken,
    cache: ByteLru<Box<str>, Cached>,
}

impl Search {
    pub(super) fn new() -> Self {
        Self {
            query: Box::default(),
            key: Box::default(),
            library: Default::default(),
            catalog: Default::default(),
            loading: Loading::Done,
            cancel: CancellationToken::new(),
            cache: ByteLru::new(CACHE_BYTES, CACHED_BYTES),
        }
    }

    /// The keys the page lists for one kind: library matches, then the catalog's others.
    fn shown(&self, entity: Entity) -> Vec<u32> {
        let library = &self.library[entity as usize];
        let catalog = &self.catalog[entity as usize].keys;
        let mut keys = library.clone();
        keys.extend(catalog.iter().filter(|key| !library.contains(key)));
        keys
    }

    /// Stops the catalog requests for the old query.
    fn restart(&mut self) {
        self.cancel.cancel();
        self.cancel = CancellationToken::new();
    }
}

/// Catalog results for one query.
#[derive(Clone)]
struct Cached {
    at: Instant,
    catalog: [Hits; 4],
}

impl Weigh for Cached {
    fn bytes(&self) -> u64 {
        let keys: usize = self.catalog.iter().map(|hits| hits.keys.capacity()).sum();
        u64::try_from(size_of::<Self>() + keys * size_of::<u32>()).unwrap_or(u64::MAX)
    }
}

impl Controller {
    /// Searches for `text`: the library at once, the catalog from the cache or after a pause
    /// in typing. `now` skips the pause, as Enter does.
    pub(super) async fn search(&mut self, text: &str, now: bool) {
        let query = tidy(text);
        let key: Box<str> = query.to_lowercase().into();
        if key == self.search.key {
            if now && self.search.loading == Loading::Pending {
                self.search.restart();
                self.request_catalog(KINDS.to_vec(), 0, false);
            }
            return;
        }
        self.search.restart();
        self.search.query = query;
        self.search.key = key;
        self.search.library = Default::default();
        self.search.catalog = Default::default();
        self.search.loading = Loading::Done;
        if !self.search.key.is_empty() {
            match self
                .store
                .search_library(&self.search.key, LIBRARY_HITS)
                .await
            {
                Ok(found) => {
                    for (entity, keys) in found {
                        self.search.library[entity as usize] = keys;
                    }
                }
                Err(error) => tracing::warn!(%error, "library search failed"),
            }
            let cached = (self.search.cache.get(&self.search.key))
                .filter(|cached| cached.at.elapsed() < FRESH_FOR);
            match cached {
                Some(cached) => self.search.catalog.clone_from(&cached.catalog),
                None => self.request_catalog(KINDS.to_vec(), 0, !now),
            }
        }
        self.show_search();
    }

    /// Asks the catalog for the next page of one kind of result.
    pub(super) fn search_more(&mut self, entity: Entity) {
        let hits = &self.search.catalog[entity as usize];
        if has_more(hits) {
            self.request_catalog(vec![entity], hits.next, false);
        }
    }

    /// Takes in catalog results, if they are for the current query.
    pub(super) fn searched(
        &mut self,
        query: Box<str>,
        more: bool,
        found: Option<Vec<(Entity, Hits)>>,
    ) {
        if query != self.search.key {
            return;
        }
        let Some(found) = found else {
            if !more {
                self.search.loading = Loading::Failed;
                self.show_search();
            }
            return;
        };
        for (entity, hits) in found {
            let shown = &mut self.search.catalog[entity as usize];
            if more {
                let new: Vec<u32> = (hits.keys.into_iter())
                    .filter(|key| !shown.keys.contains(key))
                    .collect();
                shown.keys.extend(new);
                shown.next = shown.next.max(hits.next);
                shown.total = hits.total;
            } else {
                *shown = hits;
            }
        }
        self.search.loading = Loading::Done;
        // Further pages join the cached first page without making it fresh again.
        let at = (self.search.cache.get(&query))
            .filter(|_| more)
            .map_or_else(Instant::now, |cached| cached.at);
        let catalog = self.search.catalog.clone();
        self.search.cache.insert(query, Cached { at, catalog });
        self.show_search();
    }

    /// Plays the songs the search page lists, from the one with key `start`.
    pub(super) async fn play_found(&mut self, start: u32) -> Result<(), store::Error> {
        let songs = self.search.shown(Entity::Track);
        let ids = self.store.ids(Entity::Track, songs).await?;
        self.play_context(Context::Tracks(ids.into()), Some(start), false)
            .await
    }

    /// Searches the catalog for the current query: `kinds` from `offset`, after the typing
    /// pause when `pause` is set. The results come back as `Event::Searched`.
    fn request_catalog(&mut self, kinds: Vec<Entity>, offset: u32, pause: bool) {
        let more = offset > 0;
        let Some(library) = &self.library else {
            if !more {
                self.search.loading = Loading::Failed;
            }
            return;
        };
        if !more {
            self.search.loading = Loading::Pending;
        }
        let gateway = library.gateway.clone();
        let store = self.store.clone();
        let events = self.events.clone();
        let query = self.search.key.clone();
        let cancel = self.search.cancel.clone();
        tokio::spawn(async move {
            if pause {
                tokio::select! {
                    () = cancel.cancelled() => return,
                    () = time::sleep(PAUSE) => {}
                }
            }
            let found =
                match catalog::search(&gateway, &store, &query, &kinds, offset, cancel.clone())
                    .await
                {
                    Ok(found) => Some(found),
                    Err(_) if cancel.is_cancelled() => return,
                    Err(error) => {
                        tracing::warn!(%error, "catalog search failed");
                        None
                    }
                };
            // Fails only once the Controller stopped, and then nothing waits for this.
            let _ = events.send(Event::Searched { query, more, found }).await;
        });
    }

    /// Shows the search page as it stands.
    pub(super) fn show_search(&self) {
        let kinds = KINDS.map(|entity| {
            let more = has_more(&self.search.catalog[entity as usize]);
            (self.search.shown(entity), more)
        });
        let query = self.search.query.clone();
        self.view.search(query, kinds, self.search.loading);
    }
}

/// Whether Spotify has results of a kind beyond those shown.
fn has_more(hits: &Hits) -> bool {
    hits.next < hits.total.min(LAST_OFFSET)
}

/// `text` without spaces at its ends and with inner runs of them as one; empty when too
/// short to search for.
fn tidy(text: &str) -> Box<str> {
    let query = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if query.chars().count() < MIN_CHARS {
        Box::default()
    } else {
        query.into()
    }
}
