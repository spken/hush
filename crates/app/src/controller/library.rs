//! The Controller's library side: sync scheduling and what the library page shows.

use model::{Entity, Event, List, Route};
use tokio_util::sync::CancellationToken;

use super::Controller;

/// How many recently played playlists the now-playing page can show.
const RECENT: u32 = 12;
use crate::view::{Loading, Open};

impl Controller {
    /// Starts a library sync unless one is running; without `force` it only syncs when the
    /// last sync is a day old.
    pub(super) fn sync(&mut self, force: bool) {
        let Some(library) = &self.library else {
            if force {
                tracing::info!("library sync needs the Web API");
            }
            return;
        };
        if self.syncing.is_some() {
            return;
        }
        let cancel = CancellationToken::new();
        self.syncing = Some(cancel.clone());
        let library = library.clone();
        tokio::spawn(async move {
            if let Err(error) = catalog::sync(&library, force, &cancel).await
                && !cancel.is_cancelled()
            {
                tracing::warn!(%error, "library sync failed");
            }
            // Fails only once the Controller stopped, and then nothing waits for this.
            let _ = library.events.send(Event::Synced).await;
        });
    }

    /// Shows everything the library page reads from the database.
    pub(super) async fn show_library(&mut self) {
        report(self.show_playlists().await);
        report(self.show_liked().await);
        report(self.show_saved_albums().await);
        report(self.show_open().await);
        self.show_recent();
    }

    /// Records the playlists in Spotify's listening history, then shows the ones played
    /// last. The history holds only the last 50 tracks, so the database remembers older
    /// plays.
    pub(super) fn show_recent(&self) {
        let gateway = (self.library.as_ref()).map(|library| library.gateway.clone());
        let store = self.store.clone();
        let view = self.view.clone();
        tokio::spawn(async move {
            if let Some(gateway) = gateway {
                match gateway.recently_played(CancellationToken::new()).await {
                    Ok(history) => {
                        if let Err(error) = store.save_played(history.playlists()).await {
                            tracing::warn!(%error, "listening history not stored");
                        }
                    }
                    Err(error) => tracing::warn!(%error, "listening history not read"),
                }
            }
            match store.recent_playlists(RECENT).await {
                Ok(keys) => view.recent(keys),
                Err(error) => tracing::warn!(%error, "recent playlists not read"),
            }
        });
    }

    /// Shows a list that changed in the database wherever it is on show.
    pub(super) async fn list_changed(&mut self, list: List) -> Result<(), store::Error> {
        match list {
            List::Playlists => self.show_playlists().await?,
            List::LikedSongs => self.show_liked().await?,
            List::SavedAlbums => self.show_saved_albums().await?,
            List::Playlist(_) | List::Album(_) | List::ArtistTop(_) | List::ArtistAlbums(_) => {}
        }
        // A playlist's header comes from the playlist list.
        let header_changed = list == List::Playlists && matches!(self.open, List::Playlist(_));
        if list == self.open || header_changed {
            self.show_open().await?;
        }
        let on_show = match (list, self.route) {
            (List::Album(key), Route::Album(open))
            | (List::ArtistTop(key) | List::ArtistAlbums(key), Route::Artist(open)) => key == open,
            _ => false,
        };
        if on_show {
            self.show_page().await?;
        }
        Ok(())
    }

    async fn show_playlists(&self) -> Result<(), store::Error> {
        self.view
            .playlists(self.store.list_keys(List::Playlists).await?);
        Ok(())
    }

    async fn show_liked(&self) -> Result<(), store::Error> {
        self.view
            .liked(self.store.list_len(List::LikedSongs).await?);
        Ok(())
    }

    async fn show_saved_albums(&self) -> Result<(), store::Error> {
        self.view
            .saved_albums(self.store.list_keys(List::SavedAlbums).await?);
        Ok(())
    }

    /// Shows the open list's header and entries; Liked Songs when the open playlist is gone.
    pub(super) async fn show_open(&mut self) -> Result<(), store::Error> {
        let open = match self.open {
            List::Playlist(key) => {
                (self.store.playlist_rows(vec![key]).await?.pop()).map(Open::Playlist)
            }
            List::SavedAlbums => Some(Open::SavedAlbums),
            List::LikedSongs
            | List::Playlists
            | List::Album(_)
            | List::ArtistTop(_)
            | List::ArtistAlbums(_) => None,
        };
        let open = open.unwrap_or_else(|| {
            self.open = List::LikedSongs;
            Open::LikedSongs
        });
        let keys = self.store.list_keys(self.open).await?;
        self.view.open(open, keys, self.loading(self.open));
        Ok(())
    }

    /// Fetches the open list when it is a playlist whose tracks may not be stored.
    pub(super) fn fetch_open(&mut self) {
        if let List::Playlist(_) = self.open {
            self.fetch(self.open);
        }
    }

    pub(super) async fn rows(&self, entity: Entity, keys: Vec<u32>) -> Result<(), store::Error> {
        match entity {
            Entity::Track => self.view.track_rows(self.store.track_rows(keys).await?),
            Entity::Album => self.view.album_rows(self.store.album_rows(keys).await?),
            Entity::Playlist => self
                .view
                .playlist_rows(self.store.playlist_rows(keys).await?),
            Entity::Artist => self.view.artist_rows(self.store.artist_rows(keys).await?),
        }
        Ok(())
    }

    /// Whether `list` may still be on its way from the catalog.
    pub(super) fn loading(&self, list: List) -> Loading {
        match (self.fetches.get(&list), &self.catalog) {
            (Some(&loading), _) => loading,
            (None, super::Catalog::Starting) => Loading::Pending,
            (None, super::Catalog::Off) => Loading::Failed,
            (None, super::Catalog::Ready(_)) => Loading::Done,
        }
    }
}

/// Logs a failed library database read; the view keeps what it showed.
pub(super) fn report(result: Result<(), store::Error>) {
    if let Err(error) = result {
        tracing::warn!(%error, "library database read failed");
    }
}
