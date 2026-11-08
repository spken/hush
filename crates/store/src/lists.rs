use std::time::Duration;

use model::List;
use rusqlite::{Connection, OptionalExtension};

use crate::{Error, Store, db::unix_now};

/// One position in a list.
#[derive(Debug, Clone)]
pub struct Entry {
    pub key: u32,
    /// When Spotify says it was added, for lists that say.
    pub added_at: Option<Box<str>>,
}

/// When a list was last synced.
#[derive(Debug)]
pub struct Cursor {
    /// Time since the sync.
    pub age: Duration,
    /// For a playlist, the snapshot its tracks came from.
    pub snapshot: Option<Box<str>>,
}

/// A playlist whose stored tracks are missing or not from its current snapshot.
#[derive(Debug)]
pub struct StalePlaylist {
    pub key: u32,
    pub id: Box<str>,
    pub snapshot: Box<str>,
}

/// The `list_id` a list is stored under. A playlist's tracks use its key; the lists of
/// other entities add their key to a multiple of 2^32, which no key reaches.
fn list_id(list: List) -> i64 {
    const SPAN: i64 = 1 << 32;
    match list {
        List::LikedSongs => -1,
        List::SavedAlbums => -2,
        List::Playlists => -3,
        List::Playlist(key) => key.into(),
        List::Album(key) => SPAN + i64::from(key),
        List::ArtistTop(key) => 2 * SPAN + i64::from(key),
        List::ArtistAlbums(key) => 3 * SPAN + i64::from(key),
    }
}

impl Store {
    /// The keys of a list's entities, in order.
    pub async fn list_keys(&self, list: List) -> Result<Vec<u32>, Error> {
        self.call(move |conn| {
            conn.prepare_cached("SELECT entity_key FROM list_item WHERE list_id = ?1 ORDER BY pos")?
                .query_map([list_id(list)], |row| row.get(0))?
                .collect()
        })
        .await
    }

    /// How many entries a list has.
    pub async fn list_len(&self, list: List) -> Result<u32, Error> {
        self.call(move |conn| {
            conn.prepare_cached("SELECT count(*) FROM list_item WHERE list_id = ?1")?
                .query_row([list_id(list)], |row| row.get(0))
        })
        .await
    }

    /// The index of the first of `entries` that the list already holds, added at the same
    /// time.
    pub async fn first_known(
        &self,
        list: List,
        entries: Vec<Entry>,
    ) -> Result<Option<usize>, Error> {
        self.call(move |conn| {
            let mut known = conn.prepare_cached(
                "SELECT EXISTS (SELECT 1 FROM list_item \
                 WHERE list_id = ?1 AND entity_key = ?2 AND added_at IS ?3)",
            )?;
            for (index, entry) in entries.iter().enumerate() {
                let params = (list_id(list), entry.key, entry.added_at.as_deref());
                if known.query_row(params, |row| row.get(0))? {
                    return Ok(Some(index));
                }
            }
            Ok(None)
        })
        .await
    }

    /// Puts `entries` at the top of a list, moving the rest down.
    pub async fn prepend(&self, list: List, entries: Vec<Entry>) -> Result<(), Error> {
        self.call(move |conn| {
            let tx = conn.unchecked_transaction()?;
            tx.prepare_cached("UPDATE list_item SET pos = pos + ?2 WHERE list_id = ?1")?
                .execute((list_id(list), count(&entries)))?;
            insert(&tx, list, 0, &entries)?;
            tx.commit()
        })
        .await
    }

    /// Writes `entries` over a list from position `at` on.
    pub async fn overwrite(&self, list: List, at: u32, entries: Vec<Entry>) -> Result<(), Error> {
        self.call(move |conn| {
            let tx = conn.unchecked_transaction()?;
            tx.prepare_cached(
                "DELETE FROM list_item WHERE list_id = ?1 AND pos >= ?2 AND pos < ?2 + ?3",
            )?
            .execute((list_id(list), at, count(&entries)))?;
            insert(&tx, list, at, &entries)?;
            tx.commit()
        })
        .await
    }

    /// Drops a list's entries from position `len` on.
    pub async fn truncate(&self, list: List, len: u32) -> Result<(), Error> {
        self.call(move |conn| {
            conn.prepare_cached("DELETE FROM list_item WHERE list_id = ?1 AND pos >= ?2")?
                .execute((list_id(list), len))?;
            Ok(())
        })
        .await
    }

    /// When a list was last synced, if ever.
    pub async fn cursor(&self, list: List) -> Result<Option<Cursor>, Error> {
        self.call(move |conn| {
            conn.prepare_cached(
                "SELECT ?2 - synced_at, snapshot FROM sync_cursor WHERE list_id = ?1",
            )?
            .query_row((list_id(list), unix_now()), |row| {
                Ok(Cursor {
                    age: Duration::from_secs(row.get::<_, i64>(0)?.try_into().unwrap_or(0)),
                    snapshot: row.get::<_, Option<String>>(1)?.map(String::into_boxed_str),
                })
            })
            .optional()
        })
        .await
    }

    /// Records that a list was synced just now, for a playlist from `snapshot`.
    pub async fn set_cursor(&self, list: List, snapshot: Option<Box<str>>) -> Result<(), Error> {
        self.call(move |conn| mark_synced(conn, list, snapshot.as_deref()))
            .await
    }

    /// The playlist with key `key`, if its stored tracks are missing or older than its
    /// snapshot.
    pub async fn stale_playlist(&self, key: u32) -> Result<Option<StalePlaylist>, Error> {
        self.call(move |conn| {
            conn.prepare_cached(
                "SELECT p.key, p.id, p.snapshot FROM playlist p                  LEFT JOIN sync_cursor c ON c.list_id = p.key                  WHERE p.key = ?1 AND c.snapshot IS NOT p.snapshot",
            )?
            .query_row([key], stale)
            .optional()
        })
        .await
    }

    /// The playlists in the user's list whose tracks `user_id` may read and whose stored
    /// tracks are missing or from an older snapshot, in list order.
    pub async fn stale_playlists(&self, user_id: Box<str>) -> Result<Vec<StalePlaylist>, Error> {
        self.call(move |conn| {
            conn.prepare_cached(
                "SELECT p.key, p.id, p.snapshot FROM list_item l \
                 JOIN playlist p ON p.key = l.entity_key \
                 LEFT JOIN sync_cursor c ON c.list_id = p.key \
                 WHERE l.list_id = ?1 AND (p.owner_id = ?2 OR p.collaborative) \
                 AND c.snapshot IS NOT p.snapshot \
                 ORDER BY l.pos",
            )?
            .query_map((list_id(List::Playlists), &*user_id), stale)?
            .collect()
        })
        .await
    }
}

fn stale(row: &rusqlite::Row<'_>) -> rusqlite::Result<StalePlaylist> {
    Ok(StalePlaylist {
        key: row.get(0)?,
        id: row.get::<_, String>(1)?.into_boxed_str(),
        snapshot: row.get::<_, String>(2)?.into_boxed_str(),
    })
}

/// Records that a list was synced just now, for a playlist from `snapshot`.
pub(crate) fn mark_synced(
    conn: &Connection,
    list: List,
    snapshot: Option<&str>,
) -> rusqlite::Result<()> {
    conn.prepare_cached(
        "INSERT OR REPLACE INTO sync_cursor (list_id, synced_at, snapshot) VALUES (?1, ?2, ?3)",
    )?
    .execute((list_id(list), unix_now(), snapshot))?;
    Ok(())
}

/// Makes `entries` the whole of a list.
pub(crate) fn replace_list(
    conn: &Connection,
    list: List,
    entries: &[Entry],
) -> rusqlite::Result<()> {
    conn.prepare_cached("DELETE FROM list_item WHERE list_id = ?1")?
        .execute([list_id(list)])?;
    insert(conn, list, 0, entries)
}

fn insert(conn: &Connection, list: List, at: u32, entries: &[Entry]) -> rusqlite::Result<()> {
    let mut insert = conn.prepare_cached(
        "INSERT INTO list_item (list_id, pos, entity_key, added_at) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (pos, entry) in (at..).zip(entries) {
        insert.execute((list_id(list), pos, entry.key, entry.added_at.as_deref()))?;
    }
    Ok(())
}

fn count(entries: &[Entry]) -> u32 {
    u32::try_from(entries.len()).unwrap_or(u32::MAX)
}
