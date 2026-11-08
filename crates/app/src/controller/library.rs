//! The Controller's library side: sync scheduling and what the library page shows.

use model::{Entity, Event, List};
use tokio_util::sync::CancellationToken;

use super::Controller;

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
        report(self.show_open().await);
    }

    pub(super) async fn list_changed(&mut self, list: List) -> Result<(), store::Error> {
        match list {
            List::Playlists => self.show_playlists().await?,
            List::LikedSongs => self.show_liked().await?,
            List::SavedAlbums | List::Playlist(_) => {}
        }
        // A playlist's header comes from the playlist list.
        let header_changed = list == List::Playlists && matches!(self.open, List::Playlist(_));
        if list == self.open || header_changed {
            self.show_open().await?;
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

    /// Shows the open list's header and tracks; Liked Songs when the open playlist is gone.
    pub(super) async fn show_open(&mut self) -> Result<(), store::Error> {
        let playlist = match self.open {
            List::Playlist(key) => self.store.playlist_rows(vec![key]).await?.pop(),
            List::LikedSongs | List::SavedAlbums | List::Playlists => None,
        };
        if playlist.is_none() {
            self.open = List::LikedSongs;
        }
        let keys = self.store.list_keys(self.open).await?;
        self.view.open(playlist, keys);
        Ok(())
    }

    pub(super) async fn rows(&self, entity: Entity, keys: Vec<u32>) -> Result<(), store::Error> {
        match entity {
            Entity::Track => self.view.track_rows(self.store.track_rows(keys).await?),
            Entity::Playlist => self
                .view
                .playlist_rows(self.store.playlist_rows(keys).await?),
        }
        Ok(())
    }
}

/// Logs a failed library database read; the view keeps what it showed.
pub(super) fn report(result: Result<(), store::Error>) {
    if let Err(error) = result {
        tracing::warn!(%error, "library database read failed");
    }
}
