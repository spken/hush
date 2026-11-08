use std::time::Duration;

use model::{
    Event, List,
    web::{Added, Page, Playlist},
};
use serde::de::DeserializeOwned;
use store::{Entry, Store};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use webapi::{ApiGateway, Lane};

/// Entries per Web API page, the most Spotify allows.
const PAGE: u32 = 50;
/// How old the last sync may get before the next one is due.
const SYNC_EVERY: Duration = Duration::from_hours(24);
const LIBRARY: [List; 3] = [List::LikedSongs, List::SavedAlbums, List::Playlists];

/// Ways a library sync can fail.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Web(#[from] webapi::Error),
    #[error("library database: {0}")]
    Store(#[from] store::Error),
}

/// What library sync reads from and writes to.
#[derive(Debug, Clone)]
pub struct Library {
    pub gateway: ApiGateway,
    pub store: Store,
    /// The signed-in user's Spotify ID, which tells their own playlists apart.
    pub user_id: Box<str>,
    /// Where progress and list changes go.
    pub events: mpsc::Sender<Event>,
}

/// Brings the library in the database up to date with Spotify, unless it was synced in
/// the last day and `force` is off.
pub async fn sync(library: &Library, force: bool, cancel: &CancellationToken) -> Result<(), Error> {
    if !force && !due(&library.store).await? {
        return Ok(());
    }
    let sync = Sync { library, cancel };
    sync.emit(Event::Syncing { done: 0, total: 0 }).await;
    sync.saved::<Added<_>>(List::LikedSongs, "/me/tracks", &Store::save_tracks)
        .await?;
    sync.saved::<Added<_>>(List::SavedAlbums, "/me/albums", &Store::save_albums)
        .await?;
    sync.playlists().await?;
    tracing::info!("library synced");
    Ok(())
}

async fn due(store: &Store) -> Result<bool, Error> {
    for list in LIBRARY {
        let cursor = store.cursor(list).await?;
        if cursor.is_none_or(|cursor| cursor.age >= SYNC_EVERY) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// An entry of a Web API list: the item it holds and when it was added.
trait Listed: DeserializeOwned {
    type Item;
    fn split(self) -> (Option<Box<str>>, Option<Self::Item>);
}

impl<T: DeserializeOwned> Listed for Added<T> {
    type Item = T;
    fn split(self) -> (Option<Box<str>>, Option<T>) {
        (self.added_at, self.item)
    }
}

impl Listed for Playlist {
    type Item = Self;
    fn split(self) -> (Option<Box<str>>, Option<Self>) {
        (None, Some(self))
    }
}

/// Saves a page's items and returns their keys, `None` for items it skipped.
trait Save<T>: AsyncFn(&Store, Vec<T>) -> Result<Vec<Option<u32>>, store::Error> {}

impl<T, F: AsyncFn(&Store, Vec<T>) -> Result<Vec<Option<u32>>, store::Error>> Save<T> for F {}

/// A page that was read: its entries, the list's total and how many entries Spotify sent.
struct Read {
    entries: Vec<Entry>,
    total: u32,
    sent: u32,
}

struct Sync<'a> {
    library: &'a Library,
    cancel: &'a CancellationToken,
}

impl Sync<'_> {
    /// Syncs a newest-first list: reads it until an entry already stored and puts what is
    /// new on top. When the stored length then differs from Spotify's, something was
    /// removed, and the whole list is read again.
    async fn saved<E: Listed>(
        &self,
        list: List,
        path: &str,
        save: &impl Save<E::Item>,
    ) -> Result<(), Error> {
        let store = &self.library.store;
        if store.list_len(list).await? == 0 {
            self.reread::<E>(list, path, save).await?;
        } else {
            let mut new = Vec::new();
            let mut offset = 0;
            let total = loop {
                let read = self.page::<E>(path, offset, save).await?;
                match store.first_known(list, read.entries.clone()).await? {
                    Some(known) => {
                        new.extend(read.entries.into_iter().take(known));
                        break read.total;
                    }
                    None => new.extend(read.entries),
                }
                offset += read.sent;
                if read.sent < PAGE || offset >= read.total {
                    break read.total;
                }
            };
            if !new.is_empty() {
                tracing::info!(?list, added = new.len(), "library list grew");
                store.prepend(list, new).await?;
                self.emit(Event::ListChanged(list)).await;
            }
            if store.list_len(list).await? != total {
                tracing::info!(?list, total, "library list out of step; reading all of it");
                self.reread::<E>(list, path, save).await?;
            }
        }
        store.set_cursor(list, None).await?;
        Ok(())
    }

    /// Syncs the playlist list, then the tracks of every playlist the user may read whose
    /// snapshot changed.
    async fn playlists(&self) -> Result<(), Error> {
        let store = &self.library.store;
        self.reread::<Playlist>(List::Playlists, "/me/playlists", &Store::save_playlists)
            .await?;
        store.set_cursor(List::Playlists, None).await?;
        for playlist in store.stale_playlists(self.library.user_id.clone()).await? {
            let list = List::Playlist(playlist.key);
            let path = format!("/playlists/{}/items", playlist.id);
            match self
                .reread::<Added<_>>(list, &path, &Store::save_tracks)
                .await
            {
                // A playlist someone else owns and made collaborative without the user in it.
                Err(Error::Web(webapi::Error::Status(403))) => {
                    tracing::info!(id = &*playlist.id, "playlist tracks not readable");
                }
                result => result?,
            }
            store.set_cursor(list, Some(playlist.snapshot)).await?;
        }
        Ok(())
    }

    /// Reads all of a list and writes it over the stored one, page by page.
    async fn reread<E: Listed>(
        &self,
        list: List,
        path: &str,
        save: &impl Save<E::Item>,
    ) -> Result<(), Error> {
        let store = &self.library.store;
        let (mut offset, mut len) = (0, 0);
        loop {
            let read = self.page::<E>(path, offset, save).await?;
            let written = u32::try_from(read.entries.len()).unwrap_or(u32::MAX);
            store.overwrite(list, len, read.entries).await?;
            len += written;
            offset += read.sent;
            let finished = read.sent < PAGE || offset >= read.total;
            if finished {
                store.truncate(list, len).await?;
            }
            self.emit(Event::Syncing {
                done: offset.min(read.total),
                total: read.total,
            })
            .await;
            self.emit(Event::ListChanged(list)).await;
            if finished {
                return Ok(());
            }
        }
    }

    /// Reads the page of `path` at `offset` and saves its items.
    async fn page<E: Listed>(
        &self,
        path: &str,
        offset: u32,
        save: &impl Save<E::Item>,
    ) -> Result<Read, Error> {
        let page: Page<E> = self
            .library
            .gateway
            .get(
                &format!("{path}?limit={PAGE}&offset={offset}"),
                Lane::P3,
                self.cancel.clone(),
            )
            .await?;
        let sent = u32::try_from(page.items.len()).unwrap_or(u32::MAX);
        let (added, items): (Vec<_>, Vec<_>) = page
            .items
            .into_iter()
            .filter_map(|entry| match entry.split() {
                (added_at, Some(item)) => Some((added_at, item)),
                (_, None) => None,
            })
            .unzip();
        let keys = save(&self.library.store, items).await?;
        let entries = keys
            .into_iter()
            .zip(added)
            .filter_map(|(key, added_at)| {
                Some(Entry {
                    key: key?,
                    added_at,
                })
            })
            .collect();
        Ok(Read {
            entries,
            total: page.total.unwrap_or(0),
            sent,
        })
    }

    async fn emit(&self, event: Event) {
        // Fails only once the Controller stopped, and then nothing needs the event.
        let _ = self.library.events.send(event).await;
    }
}
