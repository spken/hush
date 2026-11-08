use model::{
    AlbumKind, AlbumRow, Art, ArtistRow, Entity, List, PlaylistRow, SpotifyId, TrackRow,
    catalog::{Album, Artist, Credit, Track},
    web,
};
use rusqlite::{Connection, Row};

use crate::{
    Error, Store,
    lists::{Entry, mark_synced, replace_list},
};

impl Store {
    /// Adds or updates tracks and returns their keys, in order.
    pub async fn save_tracks(&self, tracks: Vec<Track>) -> Result<Vec<u32>, Error> {
        self.call(move |conn| in_transaction(conn, |conn| upsert_tracks(conn, &tracks)))
            .await
    }

    /// Adds or updates albums, not their tracks, and returns their keys, in order.
    pub async fn save_albums(&self, albums: Vec<Album>) -> Result<Vec<u32>, Error> {
        self.call(move |conn| in_transaction(conn, |conn| upsert_albums(conn, &albums)))
            .await
    }

    /// Adds or renames artists and returns their keys, in order.
    pub async fn save_artists(&self, artists: Vec<Credit>) -> Result<Vec<u32>, Error> {
        self.call(move |conn| {
            in_transaction(conn, |conn| {
                (artists.iter())
                    .map(|artist| upsert_artist(conn, artist))
                    .collect()
            })
        })
        .await
    }

    /// Adds or updates playlists, which all have an ID, and returns their keys, in order.
    pub async fn save_playlists(&self, playlists: Vec<web::Playlist>) -> Result<Vec<u32>, Error> {
        self.call(move |conn| {
            in_transaction(conn, |conn| {
                let mut statement = conn.prepare_cached(
                    "INSERT INTO playlist \
                     (id, name, owner_id, owner, collaborative, tracks, snapshot, art) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
                     ON CONFLICT (id) DO UPDATE SET name = excluded.name, \
                     owner_id = excluded.owner_id, owner = excluded.owner, \
                     collaborative = excluded.collaborative, tracks = excluded.tracks, \
                     snapshot = excluded.snapshot, \
                     art = iif(excluded.art = '', art, excluded.art) RETURNING key",
                )?;
                (playlists.iter())
                    .map(|playlist| {
                        let owner = playlist.owner.as_ref();
                        let tracks = playlist.items.as_ref().and_then(|items| items.total);
                        let art = web::Image::art(playlist.images.as_deref().unwrap_or_default());
                        let params = (
                            text(playlist.id.as_deref()),
                            text(playlist.name.as_deref()),
                            text(owner.and_then(|owner| owner.id.as_deref())),
                            text(owner.and_then(|owner| owner.display_name.as_deref())),
                            playlist.collaborative.unwrap_or(false),
                            tracks.unwrap_or(0),
                            text(playlist.snapshot_id.as_deref()),
                            art.as_str(),
                        );
                        statement.query_row(params, |row| row.get(0))
                    })
                    .collect()
            })
        })
        .await
    }

    /// Stores an album with its tracks, in order.
    pub async fn save_album(&self, album: Album) -> Result<(), Error> {
        self.call(move |conn| {
            in_transaction(conn, |conn| {
                let key = upsert_albums(conn, std::slice::from_ref(&album))?[0];
                let tracks = upsert_tracks(conn, &album.tracks)?;
                replace_list(conn, List::Album(key), &entries(tracks))?;
                mark_synced(conn, List::Album(key), None)
            })
        })
        .await
    }

    /// Stores the artist with key `key`: its name, portrait, top tracks and albums.
    pub async fn save_artist(&self, key: u32, artist: Artist) -> Result<(), Error> {
        self.call(move |conn| {
            in_transaction(conn, |conn| {
                conn.prepare_cached(
                    "UPDATE artist SET name = ?2, art = iif(?3 = '', art, ?3) WHERE key = ?1",
                )?
                .execute((key, &*artist.name, artist.art.as_str()))?;
                let top = upsert_tracks(conn, &artist.top_tracks)?;
                replace_list(conn, List::ArtistTop(key), &entries(top))?;
                let albums = upsert_albums(conn, &artist.albums)?;
                replace_list(conn, List::ArtistAlbums(key), &entries(albums))?;
                mark_synced(conn, List::ArtistTop(key), None)
            })
        })
        .await
    }

    /// Stores the tracks of the playlist with key `key`, as of `snapshot`.
    pub async fn save_playlist_tracks(
        &self,
        key: u32,
        tracks: Vec<Track>,
        snapshot: Box<str>,
    ) -> Result<(), Error> {
        self.call(move |conn| {
            in_transaction(conn, |conn| {
                let tracks = upsert_tracks(conn, &tracks)?;
                replace_list(conn, List::Playlist(key), &entries(tracks))?;
                mark_synced(conn, List::Playlist(key), Some(&snapshot))
            })
        })
        .await
    }

    /// The Spotify ID behind a key, if the key exists.
    pub async fn id(&self, entity: Entity, key: u32) -> Result<Option<SpotifyId>, Error> {
        Ok(self.ids(entity, vec![key]).await?.pop())
    }

    /// The Spotify IDs behind keys, in order; keys that do not exist are left out.
    pub async fn ids(&self, entity: Entity, keys: Vec<u32>) -> Result<Vec<SpotifyId>, Error> {
        let table = match entity {
            Entity::Track => "track",
            Entity::Album => "album",
            Entity::Artist => "artist",
            Entity::Playlist => "playlist",
        };
        let ids: Vec<String> = self
            .call(move |conn| {
                rows(
                    conn,
                    &format!(
                        "SELECT e.id FROM json_each(?1) j JOIN {table} e ON e.key = j.value \
                         ORDER BY j.key"
                    ),
                    &keys,
                    |row| row.get(0),
                )
            })
            .await?;
        Ok(ids.iter().filter_map(|id| id.parse().ok()).collect())
    }

    /// The rows of these artists, in no particular order.
    pub async fn artist_rows(&self, keys: Vec<u32>) -> Result<Vec<ArtistRow>, Error> {
        self.call(move |conn| {
            rows(
                conn,
                "SELECT key, name, art FROM artist \
                 WHERE key IN (SELECT value FROM json_each(?1))",
                &keys,
                |row| {
                    Ok(ArtistRow {
                        key: row.get(0)?,
                        name: boxed(row.get(1)?),
                        art: Art::from_stored(row.get(2)?),
                    })
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
                "SELECT key, name, artists, artist_key, duration_ms, art FROM track \
                 WHERE key IN (SELECT value FROM json_each(?1))",
                &keys,
                |row| {
                    Ok(TrackRow {
                        key: row.get(0)?,
                        name: boxed(row.get(1)?),
                        artists: boxed(row.get(2)?),
                        artist_key: row.get(3)?,
                        duration_ms: row.get(4)?,
                        art: Art::from_stored(row.get(5)?),
                    })
                },
            )
        })
        .await
    }

    /// The rows of these albums, in no particular order.
    pub async fn album_rows(&self, keys: Vec<u32>) -> Result<Vec<AlbumRow>, Error> {
        self.call(move |conn| {
            rows(
                conn,
                "SELECT key, name, artists, artist_key, year, kind, art FROM album \
                 WHERE key IN (SELECT value FROM json_each(?1))",
                &keys,
                |row| {
                    Ok(AlbumRow {
                        key: row.get(0)?,
                        name: boxed(row.get(1)?),
                        artists: boxed(row.get(2)?),
                        artist_key: row.get(3)?,
                        year: row.get(4)?,
                        kind: kind_from(row.get(5)?),
                        art: Art::from_stored(row.get(6)?),
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
                 EXISTS (SELECT 1 FROM sync_cursor WHERE list_id = key), art FROM playlist \
                 WHERE key IN (SELECT value FROM json_each(?1))",
                &keys,
                |row| {
                    Ok(PlaylistRow {
                        key: row.get(0)?,
                        name: boxed(row.get(1)?),
                        owner: boxed(row.get(2)?),
                        tracks: row.get(3)?,
                        synced: row.get(4)?,
                        art: Art::from_stored(row.get(5)?),
                    })
                },
            )
        })
        .await
    }
}

fn in_transaction<T>(
    conn: &Connection,
    work: impl FnOnce(&Connection) -> rusqlite::Result<T>,
) -> rusqlite::Result<T> {
    let tx = conn.unchecked_transaction()?;
    let result = work(&tx)?;
    tx.commit()?;
    Ok(result)
}

fn upsert_tracks(conn: &Connection, tracks: &[Track]) -> rusqlite::Result<Vec<u32>> {
    let mut statement = conn.prepare_cached(
        "INSERT INTO track (id, name, artists, artist_key, duration_ms, art) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT (id) DO UPDATE SET name = excluded.name, artists = excluded.artists, \
         artist_key = excluded.artist_key, duration_ms = excluded.duration_ms, \
         art = iif(excluded.art = '', art, excluded.art) RETURNING key",
    )?;
    (tracks.iter())
        .map(|track| {
            let artist = first_artist(conn, &track.artists)?;
            let params = (
                track.id.to_string(),
                &*track.name,
                names(&track.artists),
                artist,
                track.duration_ms,
                track.art.as_str(),
            );
            statement.query_row(params, |row| row.get(0))
        })
        .collect()
}

fn upsert_albums(conn: &Connection, albums: &[Album]) -> rusqlite::Result<Vec<u32>> {
    let mut statement = conn.prepare_cached(
        "INSERT INTO album (id, name, artists, artist_key, year, kind, art) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
         ON CONFLICT (id) DO UPDATE SET name = excluded.name, artists = excluded.artists, \
         artist_key = excluded.artist_key, year = excluded.year, kind = excluded.kind, \
         art = iif(excluded.art = '', art, excluded.art) RETURNING key",
    )?;
    (albums.iter())
        .map(|album| {
            let artist = first_artist(conn, &album.artists)?;
            let params = (
                album.id.to_string(),
                &*album.name,
                names(&album.artists),
                artist,
                album.year,
                kind_to(album.kind),
                album.art.as_str(),
            );
            statement.query_row(params, |row| row.get(0))
        })
        .collect()
}

/// Adds or renames the first credited artist and returns its key.
fn first_artist(conn: &Connection, artists: &[Credit]) -> rusqlite::Result<Option<u32>> {
    (artists.first())
        .map(|artist| upsert_artist(conn, artist))
        .transpose()
}

fn upsert_artist(conn: &Connection, artist: &Credit) -> rusqlite::Result<u32> {
    conn.prepare_cached(
        "INSERT INTO artist (id, name, art) VALUES (?1, ?2, ?3) \
         ON CONFLICT (id) DO UPDATE SET name = excluded.name, \
         art = iif(excluded.art = '', art, excluded.art) RETURNING key",
    )?
    .query_row(
        (artist.id.to_string(), &*artist.name, artist.art.as_str()),
        |row| row.get(0),
    )
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

fn entries(keys: Vec<u32>) -> Vec<Entry> {
    (keys.into_iter())
        .map(|key| Entry {
            key,
            added_at: None,
        })
        .collect()
}

fn kind_to(kind: AlbumKind) -> u8 {
    match kind {
        AlbumKind::Album => 0,
        AlbumKind::Single => 1,
        AlbumKind::Compilation => 2,
    }
}

fn kind_from(kind: u8) -> AlbumKind {
    match kind {
        1 => AlbumKind::Single,
        2 => AlbumKind::Compilation,
        _ => AlbumKind::Album,
    }
}

/// Artist names as one line, the way rows show them.
fn names(artists: &[Credit]) -> String {
    let listed: Vec<&str> = artists.iter().map(|artist| &*artist.name).collect();
    listed.join(", ")
}

fn text(value: Option<&str>) -> String {
    value.unwrap_or_default().to_owned()
}

fn boxed(text: String) -> Box<str> {
    text.into_boxed_str()
}
