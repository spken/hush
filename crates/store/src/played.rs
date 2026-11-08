use model::SpotifyId;

use crate::{Error, Store, entities::in_transaction};

impl Store {
    /// Records when playlists were last played, as `(ID, ISO 8601 time)`. Playlists that
    /// are not stored are skipped, and an older time never replaces a newer one.
    pub async fn save_played(&self, played: Vec<(SpotifyId, Box<str>)>) -> Result<(), Error> {
        self.call(move |conn| {
            in_transaction(conn, |conn| {
                let mut update = conn.prepare_cached(
                    "UPDATE playlist SET played_at = max(played_at, ?2) WHERE id = ?1",
                )?;
                for (id, at) in &played {
                    update.execute((id.to_string(), &**at))?;
                }
                Ok(())
            })
        })
        .await
    }

    /// The keys of the `limit` playlists played last, most recent first.
    pub async fn recent_playlists(&self, limit: u32) -> Result<Vec<u32>, Error> {
        self.call(move |conn| {
            conn.prepare_cached(
                "SELECT key FROM playlist WHERE played_at != '' \
                 ORDER BY played_at DESC LIMIT ?1",
            )?
            .query_map([limit], |row| row.get(0))?
            .collect()
        })
        .await
    }
}
