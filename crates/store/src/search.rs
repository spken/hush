use model::{Entity, List};
use rusqlite::{Connection, Params};

use crate::{Error, Store, lists::list_id};

/// The keys of entities in a library list, as `?3`.
const IN_LIST: &str = "SELECT entity_key FROM list_item WHERE list_id = ?3";
/// The first artists of the tracks in list `?3` and the albums in list `?4`.
const ARTISTS_IN_LISTS: &str = "SELECT t.artist_key FROM list_item l \
     JOIN track t ON t.key = l.entity_key WHERE l.list_id = ?3 \
     UNION SELECT a.artist_key FROM list_item l \
     JOIN album a ON a.key = l.entity_key WHERE l.list_id = ?4";

impl Store {
    /// Library entries whose text has every word of `query` as a word start, best match
    /// first, at most `limit` of each kind: Liked Songs, saved albums, the user's playlists,
    /// and the first artists of liked songs and saved albums.
    pub async fn search_library(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<(Entity, Vec<u32>)>, Error> {
        let Some(words) = fts_query(query) else {
            return Ok(Vec::new());
        };
        self.call(move |conn| {
            let liked = list_id(List::LikedSongs);
            let saved = list_id(List::SavedAlbums);
            let playlists = list_id(List::Playlists);
            Ok(vec![
                (
                    Entity::Track,
                    keys(conn, "track_fts", IN_LIST, (&words, limit, liked))?,
                ),
                (
                    Entity::Album,
                    keys(conn, "album_fts", IN_LIST, (&words, limit, saved))?,
                ),
                (
                    Entity::Artist,
                    keys(
                        conn,
                        "artist_fts",
                        ARTISTS_IN_LISTS,
                        (&words, limit, liked, saved),
                    )?,
                ),
                (
                    Entity::Playlist,
                    keys(conn, "playlist_fts", IN_LIST, (&words, limit, playlists))?,
                ),
            ])
        })
        .await
    }
}

/// The keys of the rows of the full-text index `index` that match `?1` and whose key
/// `library` selects, best match first, at most `?2`.
fn keys(
    conn: &Connection,
    index: &str,
    library: &str,
    params: impl Params,
) -> rusqlite::Result<Vec<u32>> {
    // The `+` keeps SQLite from looking up every library key in the index one by one,
    // which took ~100 ms for 4,400 liked songs; the match drives the query instead.
    conn.prepare_cached(&format!(
        "SELECT rowid FROM {index} WHERE {index} MATCH ?1 AND +rowid IN ({library}) \
         ORDER BY rank LIMIT ?2"
    ))?
    .query_map(params, |row| row.get(0))?
    .collect()
}

/// An FTS5 query for rows with every word of `text` as a word start; `None` when `text`
/// has no letters or digits.
fn fts_query(text: &str) -> Option<String> {
    let words: Vec<String> = (text.split_whitespace())
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .map(|word| format!("\"{}\"*", word.replace('"', "\"\"")))
        .collect();
    (!words.is_empty()).then(|| words.join(" "))
}
