use model::{PlaylistRow, TrackRow, web};
use rusqlite::{Connection, Row, ToSql};

use crate::{Error, Store};

type Params = Vec<Box<dyn ToSql>>;

impl Store {
    /// Adds or updates tracks and returns their keys, `None` for an entry without an ID or
    /// that is a podcast episode.
    pub async fn save_tracks(&self, tracks: Vec<web::Track>) -> Result<Vec<Option<u32>>, Error> {
        self.call(move |conn| {
            upsert(
                conn,
                "INSERT INTO track (id, name, artists, album, duration_ms) \
                 VALUES (?1, ?2, ?3, ?4, ?5) \
                 ON CONFLICT (id) DO UPDATE SET name = excluded.name, \
                 artists = excluded.artists, album = excluded.album, \
                 duration_ms = excluded.duration_ms RETURNING key",
                &tracks,
                |track| {
                    let id = track.id.as_deref();
                    let id = id.filter(|_| track.kind.as_deref() != Some("episode"))?;
                    let album = track.album.as_ref().and_then(|album| album.name.as_deref());
                    Some(vec![
                        Box::new(id.to_owned()),
                        Box::new(text(track.name.as_deref())),
                        Box::new(names(&track.artists)),
                        Box::new(text(album)),
                        Box::new(track.duration_ms.unwrap_or(0)),
                    ])
                },
            )
        })
        .await
    }

    /// Adds or updates albums and returns their keys, `None` for an entry without an ID.
    pub async fn save_albums(&self, albums: Vec<web::Album>) -> Result<Vec<Option<u32>>, Error> {
        self.call(move |conn| {
            upsert(
                conn,
                "INSERT INTO album (id, name, artists) VALUES (?1, ?2, ?3) \
                 ON CONFLICT (id) DO UPDATE SET name = excluded.name, \
                 artists = excluded.artists RETURNING key",
                &albums,
                |album| {
                    Some(vec![
                        Box::new(album.id.as_deref()?.to_owned()),
                        Box::new(text(album.name.as_deref())),
                        Box::new(names(&album.artists)),
                    ])
                },
            )
        })
        .await
    }

    /// Adds or updates playlists and returns their keys, `None` for an entry without an ID.
    pub async fn save_playlists(
        &self,
        playlists: Vec<web::Playlist>,
    ) -> Result<Vec<Option<u32>>, Error> {
        self.call(move |conn| {
            upsert(
                conn,
                "INSERT INTO playlist \
                 (id, name, owner_id, owner, collaborative, tracks, snapshot) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
                 ON CONFLICT (id) DO UPDATE SET name = excluded.name, \
                 owner_id = excluded.owner_id, owner = excluded.owner, \
                 collaborative = excluded.collaborative, tracks = excluded.tracks, \
                 snapshot = excluded.snapshot RETURNING key",
                &playlists,
                |playlist| {
                    let owner = playlist.owner.as_ref();
                    let tracks = playlist.items.as_ref().and_then(|items| items.total);
                    Some(vec![
                        Box::new(playlist.id.as_deref()?.to_owned()),
                        Box::new(text(playlist.name.as_deref())),
                        Box::new(text(owner.and_then(|owner| owner.id.as_deref()))),
                        Box::new(text(owner.and_then(|owner| owner.display_name.as_deref()))),
                        Box::new(playlist.collaborative.unwrap_or(false)),
                        Box::new(tracks.unwrap_or(0)),
                        Box::new(text(playlist.snapshot_id.as_deref())),
                    ])
                },
            )
        })
        .await
    }

    /// The rows of these tracks, in no particular order.
    pub async fn track_rows(&self, keys: Vec<u32>) -> Result<Vec<TrackRow>, Error> {
        self.call(move |conn| {
            rows(
                conn,
                "SELECT key, name, artists, duration_ms FROM track \
                 WHERE key IN (SELECT value FROM json_each(?1))",
                &keys,
                |row| {
                    Ok(TrackRow {
                        key: row.get(0)?,
                        name: boxed(row.get(1)?),
                        artists: boxed(row.get(2)?),
                        duration_ms: row.get(3)?,
                    })
                },
            )
        })
        .await
    }

    /// The rows of these playlists, in no particular order.
    pub async fn playlist_rows(&self, keys: Vec<u32>) -> Result<Vec<PlaylistRow>, Error> {
        self.call(move |conn| {
            rows(
                conn,
                "SELECT key, name, owner, tracks, \
                 EXISTS (SELECT 1 FROM sync_cursor WHERE list_id = key) FROM playlist \
                 WHERE key IN (SELECT value FROM json_each(?1))",
                &keys,
                |row| {
                    Ok(PlaylistRow {
                        key: row.get(0)?,
                        name: boxed(row.get(1)?),
                        owner: boxed(row.get(2)?),
                        tracks: row.get(3)?,
                        synced: row.get(4)?,
                    })
                },
            )
        })
        .await
    }
}

/// Runs the `INSERT … RETURNING key` statement `sql` for every item `params` gives
/// parameters for, in one transaction.
fn upsert<T>(
    conn: &Connection,
    sql: &str,
    items: &[T],
    params: impl Fn(&T) -> Option<Params>,
) -> rusqlite::Result<Vec<Option<u32>>> {
    let tx = conn.unchecked_transaction()?;
    let keys = {
        let mut statement = tx.prepare_cached(sql)?;
        items
            .iter()
            .map(|item| {
                params(item)
                    .map(|params| {
                        statement.query_row(rusqlite::params_from_iter(params), |row| row.get(0))
                    })
                    .transpose()
            })
            .collect::<rusqlite::Result<_>>()?
    };
    tx.commit()?;
    Ok(keys)
}

/// Runs `sql`, whose `?1` is the JSON array of `keys`, and maps every row it returns.
fn rows<T>(
    conn: &Connection,
    sql: &str,
    keys: &[u32],
    map: impl FnMut(&Row<'_>) -> rusqlite::Result<T>,
) -> rusqlite::Result<Vec<T>> {
    let keys: Vec<String> = keys.iter().map(u32::to_string).collect();
    conn.prepare_cached(sql)?
        .query_map([format!("[{}]", keys.join(","))], map)?
        .collect()
}

/// Artist names as one line, the way rows show them.
fn names(artists: &[web::Named]) -> String {
    let listed: Vec<&str> = (artists.iter())
        .filter_map(|artist| artist.name.as_deref())
        .collect();
    listed.join(", ")
}

fn text(value: Option<&str>) -> String {
    value.unwrap_or_default().to_owned()
}

fn boxed(text: String) -> Box<str> {
    text.into_boxed_str()
}
