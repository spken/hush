use model::Playback;
use rusqlite::OptionalExtension;

use crate::{Error, Store};

/// The `settings` row the playback settings live in.
const PLAYBACK: &str = "playback";

impl Store {
    /// The playback settings, or the defaults when none were saved or they no longer read.
    pub async fn playback(&self) -> Result<Playback, Error> {
        let saved: Option<String> = self
            .call(|conn| {
                conn.prepare_cached("SELECT value FROM settings WHERE name = ?1")?
                    .query_row([PLAYBACK], |row| row.get(0))
                    .optional()
            })
            .await?;
        Ok(saved
            .and_then(|json| {
                serde_json::from_str(&json)
                    .inspect_err(|error| tracing::warn!(%error, "playback settings unreadable"))
                    .ok()
            })
            .unwrap_or_default())
    }

    /// Saves the playback settings.
    pub async fn save_playback(&self, playback: &Playback) -> Result<(), Error> {
        let json = serde_json::to_string(playback)?;
        self.call(move |conn| {
            conn.prepare_cached(
                "INSERT INTO settings (name, value) VALUES (?1, ?2) \
                 ON CONFLICT (name) DO UPDATE SET value = excluded.value",
            )?
            .execute((PLAYBACK, json))
            .map(drop)
        })
        .await
    }
}
