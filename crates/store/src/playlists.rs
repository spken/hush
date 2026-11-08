use model::{List, PlaylistOrder, SpotifyId};

use crate::{Error, Store, entities::in_transaction, lists::list_id};

impl Store {
    /// Records when playlists last changed, as `(ID, Unix milliseconds)`, skipping those
    /// not stored. Returns whether any time changed.
    pub async fn save_modified(&self, modified: Vec<(SpotifyId, i64)>) -> Result<bool, Error> {
        self.call(move |conn| {
            in_transaction(conn, |conn| {
                let mut update = conn.prepare_cached(
                    "UPDATE playlist SET modified_at = ?2 WHERE id = ?1 AND modified_at IS NOT ?2",
                )?;
                let mut changed = false;
                for (id, at) in &modified {
                    changed |= update.execute((id.to_string(), at))? > 0;
                }
                Ok(changed)
            })
        })
        .await
    }

    /// The keys of the user's playlists and how many of them are pinned: the pinned ones
    /// come first, pinned last first, then the rest in `order`.
    pub async fn playlist_keys(&self, order: PlaylistOrder) -> Result<(Vec<u32>, u32), Error> {
        let order = match order {
            PlaylistOrder::Modified => "p.modified_at DESC NULLS LAST,",
            PlaylistOrder::Played => "p.played_at DESC,",
            PlaylistOrder::Name => "p.name COLLATE NOCASE,",
            PlaylistOrder::Creator => "p.owner COLLATE NOCASE, p.name COLLATE NOCASE,",
            PlaylistOrder::Custom => "",
        };
        let sql = format!(
            "SELECT l.entity_key, pin.seq IS NOT NULL FROM list_item l              JOIN playlist p ON p.key = l.entity_key              LEFT JOIN pinned pin ON pin.key = l.entity_key              WHERE l.list_id = ?1 ORDER BY pin.seq DESC NULLS LAST, {order} l.pos"
        );
        self.call(move |conn| {
            let mut pinned = 0;
            let keys = conn
                .prepare_cached(&sql)?
                .query_map([list_id(List::Playlists)], |row| {
                    Ok((row.get(0)?, row.get::<_, bool>(1)?))
                })?
                .map(|row| {
                    let (key, is_pinned) = row?;
                    pinned += u32::from(is_pinned);
                    Ok(key)
                })
                .collect::<rusqlite::Result<_>>()?;
            Ok((keys, pinned))
        })
        .await
    }

    /// Pins the playlist with key `key` above the others, or with `false` unpins it.
    pub async fn pin(&self, key: u32, pinned: bool) -> Result<(), Error> {
        self.call(move |conn| {
            if pinned {
                conn.prepare_cached(
                    "INSERT OR REPLACE INTO pinned (key, seq)                      VALUES (?1, (SELECT coalesce(max(seq), 0) + 1 FROM pinned))",
                )?
                .execute([key])?;
            } else {
                conn.prepare_cached("DELETE FROM pinned WHERE key = ?1")?
                    .execute([key])?;
            }
            Ok(())
        })
        .await
    }
}
