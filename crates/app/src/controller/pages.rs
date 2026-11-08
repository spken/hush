//! The Controller's catalog side: navigation, album and artist pages, catalog lookups and
//! playing lists on Hush.

use model::{Context, Entity, Event, Fetch, List, Route, TrackId};
use tokio_util::sync::CancellationToken;

use super::{Catalog, Controller, library::report};
use crate::view::Loading;

/// Pages Back can return through; older ones are forgotten.
const HISTORY: usize = 50;

impl Controller {
    /// Shows `route`. Album and artist pages remember the page they were opened from;
    /// top-level pages start the history afresh.
    pub(super) async fn navigate(&mut self, route: Route) {
        match route {
            Route::NowPlaying | Route::Library | Route::Search => self.history.clear(),
            Route::Album(_) | Route::Artist(_) if route == self.route => return,
            Route::Album(_) | Route::Artist(_) => {
                if self.history.len() == HISTORY {
                    self.history.remove(0);
                }
                self.history.push(self.route);
            }
        }
        self.route = route;
        self.show_route().await;
    }

    pub(super) async fn back(&mut self) {
        if let Some(route) = self.history.pop() {
            self.route = route;
            self.show_route().await;
        }
    }

    /// Shows the current page and fetches what it is missing.
    pub(super) async fn show_route(&mut self) {
        self.view.route(self.route, !self.history.is_empty());
        let list = match self.route {
            Route::Album(key) => List::Album(key),
            Route::Artist(key) => List::ArtistTop(key),
            Route::NowPlaying | Route::Library | Route::Search => return,
        };
        self.fetch(list);
        report(self.show_page().await);
    }

    /// Shows the current album or artist page as the database has it.
    pub(super) async fn show_page(&self) -> Result<(), store::Error> {
        match self.route {
            Route::Album(key) => {
                let Some(album) = self.store.album_rows(vec![key]).await?.pop() else {
                    tracing::warn!(key, "no such album");
                    return Ok(());
                };
                let list = List::Album(key);
                let keys = self.store.list_keys(list).await?;
                self.view.album(album, keys, self.loading(list));
            }
            Route::Artist(key) => {
                let Some(name) = self.store.artist_name(key).await? else {
                    tracing::warn!(key, "no such artist");
                    return Ok(());
                };
                let lists = (
                    self.store.list_keys(List::ArtistTop(key)).await?,
                    self.store.list_keys(List::ArtistAlbums(key)).await?,
                );
                let loading = self.loading(List::ArtistTop(key));
                self.view.artist(key, name, lists, loading);
            }
            Route::NowPlaying | Route::Library | Route::Search => {}
        }
        Ok(())
    }

    /// Asks the catalog for `list` unless a lookup for it is running; the outcome comes
    /// back as `Event::Fetched`.
    pub(super) fn fetch(&mut self, list: List) {
        let Catalog::Ready(catalog) = &self.catalog else {
            return;
        };
        if self.fetches.get(&list) == Some(&Loading::Pending) {
            return;
        }
        self.fetches.insert(list, Loading::Pending);
        let catalog = catalog.clone();
        let events = self.events.clone();
        tokio::spawn(async move {
            let outcome = match catalog.fetch(list, CancellationToken::new()).await {
                Ok(true) => Fetch::Changed,
                Ok(false) => Fetch::Unchanged,
                Err(error) => {
                    tracing::warn!(%error, ?list, "catalog lookup failed");
                    Fetch::Failed
                }
            };
            // Fails only once the Controller stopped, and then nothing waits for this.
            let _ = events.send(Event::Fetched { list, outcome }).await;
        });
    }

    pub(super) async fn fetched(&mut self, list: List, outcome: Fetch) -> Result<(), store::Error> {
        match outcome {
            Fetch::Failed => {
                self.fetches.insert(list, Loading::Failed);
            }
            Fetch::Changed | Fetch::Unchanged => {
                self.fetches.remove(&list);
            }
        }
        // A failure changes what an empty page says; a change, what it lists.
        if outcome != Fetch::Unchanged {
            self.list_changed(list).await?;
        }
        Ok(())
    }

    /// Plays `list` on Hush from the track with key `start`, or from its top, making Hush
    /// the active device first.
    pub(super) async fn play(
        &mut self,
        list: List,
        start: Option<u32>,
        shuffle: bool,
    ) -> Result<(), store::Error> {
        let context = match list {
            List::LikedSongs => Some(Context::LikedSongs),
            List::Playlist(key) => {
                (self.store.id(Entity::Playlist, key).await?).map(Context::Playlist)
            }
            List::Album(key) => self.store.id(Entity::Album, key).await?.map(Context::Album),
            List::ArtistTop(key) | List::ArtistAlbums(key) => self
                .store
                .id(Entity::Artist, key)
                .await?
                .map(Context::Artist),
            List::SavedAlbums | List::Playlists => None,
        };
        let Some(context) = context else {
            tracing::warn!(?list, "nothing to play");
            return Ok(());
        };
        self.play_context(context, start, shuffle).await
    }

    /// Plays `context` on Hush from the track with key `start`, or from its top, making
    /// Hush the active device first.
    pub(super) async fn play_context(
        &mut self,
        context: Context,
        start: Option<u32>,
        shuffle: bool,
    ) -> Result<(), store::Error> {
        let start = match start {
            Some(key) => self.store.id(Entity::Track, key).await?.map(TrackId),
            None => None,
        };
        let active = self.active;
        self.playback(|device| {
            if !active {
                device.activate()?;
            }
            device.play(context, start, shuffle)
        });
        Ok(())
    }
}
