//! The Controller's library side: sync scheduling and what the library page shows.

use model::{Entity, Event, List, PlaylistOrder, Route, TrackOrder};
use playback::Device;
use tokio_util::sync::CancellationToken;

use super::Controller;

/// How many recently played playlists the now-playing page can show.
const RECENT: u32 = 12;
use crate::view::{Loading, Open};

impl Controller {
    /// Unless a sync runs, starts a library sync, which without `force` only runs when the
    /// last one is a day old, then reads when the playlists last changed from spclient.
    /// The second needs only the device, so it runs while the Web API is off.
    pub(super) fn sync(&mut self, force: bool) {
        if self.library.is_none() && force {
            tracing::info!("library sync needs the Web API");
        }
        let session = (self.device.as_ref())
            .filter(|_| self.spclient)
            .map(Device::session);
        if self.syncing.is_some() || (self.library.is_none() && session.is_none()) {
            return;
        }
        let cancel = CancellationToken::new();
        self.syncing = Some(cancel.clone());
        let library = self.library.clone();
        let store = self.store.clone();
        let events = self.events.clone();
        tokio::spawn(async move {
            if let Some(library) = &library
                && let Err(error) = catalog::sync(library, force, &cancel).await
                && !cancel.is_cancelled()
            {
                tracing::warn!(%error, "library sync failed");
            }
            // Sends fail only once the Controller stopped, and then nothing waits for them.
            if let Some(session) = session {
                match catalog::playlist_times(&store, &session).await {
                    Ok(true) => {
                        let _ = events.send(Event::ListChanged(List::Playlists)).await;
                    }
                    Ok(false) => {}
                    Err(error) => tracing::warn!(%error, "playlist times not read"),
                }
            }
            let _ = events.send(Event::Synced).await;
        });
    }

    /// Shows everything the library page reads from the database.
    pub(super) async fn show_library(&mut self) {
        match self.store.sorting().await {
            Ok(sorting) => self.sorting = sorting,
            Err(error) => tracing::warn!(%error, "sorting not read; using the defaults"),
        }
        report(self.show_playlists().await);
        report(self.show_liked().await);
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
            List::SavedAlbums
            | List::Playlist(_)
            | List::Album(_)
            | List::ArtistTop(_)
            | List::ArtistAlbums(_) => {}
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
        let (keys, pinned) = self.store.playlist_keys(self.sorting.playlists).await?;
        self.view.playlists(keys, pinned, self.sorting.playlists);
        Ok(())
    }

    /// Orders the library's playlists as `order` from now on.
    pub(super) async fn sort_playlists(
        &mut self,
        order: PlaylistOrder,
    ) -> Result<(), store::Error> {
        self.sorting.playlists = order;
        self.store.save_sorting(self.sorting).await?;
        self.show_playlists().await
    }

    /// Orders the songs of the open list as `order` from now on.
    pub(super) async fn sort_tracks(&mut self, order: TrackOrder) -> Result<(), store::Error> {
        self.sorting.tracks = order;
        self.store.save_sorting(self.sorting).await?;
        self.show_open().await
    }

    /// Pins the playlist with key `key` above the others, or with `false` unpins it.
    pub(super) async fn pin(&self, key: u32, pinned: bool) -> Result<(), store::Error> {
        self.store.pin(key, pinned).await?;
        self.show_playlists().await
    }

    async fn show_liked(&self) -> Result<(), store::Error> {
        self.view
            .liked(self.store.list_len(List::LikedSongs).await?);
        Ok(())
    }

    /// Shows the open list's header and entries; Liked Songs when the open playlist is gone.
    pub(super) async fn show_open(&mut self) -> Result<(), store::Error> {
        let open = match self.open {
            List::Playlist(key) => {
                (self.store.playlist_rows(vec![key]).await?.pop()).map(Open::Playlist)
            }
            List::LikedSongs
            | List::SavedAlbums
            | List::Playlists
            | List::Album(_)
            | List::ArtistTop(_)
            | List::ArtistAlbums(_) => None,
        };
        let open = open.unwrap_or_else(|| {
            self.open = List::LikedSongs;
            Open::LikedSongs
        });
        let keys = self
            .store
            .track_keys(self.open, self.sorting.tracks)
            .await?;
        self.view
            .open(open, keys, self.sorting.tracks, self.loading(self.open));
        Ok(())
    }

    /// Shows `list` on the library page.
    pub(super) async fn open_list(&mut self, list: List) {
        self.forget_pictures();
        self.open = list;
        report(self.show_open().await);
        self.fetch_open();
    }

    /// Fetches the open list when it is a playlist whose tracks may not be stored.
    pub(super) fn fetch_open(&mut self) {
        if let List::Playlist(_) = self.open {
            self.fetch(self.open);
        }
    }

    /// Loads the text of the rows with `keys` and fills it in wherever it was asked for,
    /// on its own task so that commands behind it are not held up by the database.
    pub(super) fn rows(&self, entity: Entity, keys: Vec<u32>) {
        let store = self.store.clone();
        let view = self.view.clone();
        tokio::spawn(async move {
            let shown = match entity {
                Entity::Track => store
                    .track_rows(keys)
                    .await
                    .map(|rows| view.track_rows(rows)),
                Entity::Album => store
                    .album_rows(keys)
                    .await
                    .map(|rows| view.album_rows(rows)),
                Entity::Playlist => store
                    .playlist_rows(keys)
                    .await
                    .map(|rows| view.playlist_rows(rows)),
                Entity::Artist => store
                    .artist_rows(keys)
                    .await
                    .map(|rows| view.artist_rows(rows)),
            };
            report(shown);
        });
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
