use model::{Playback, Sorting};
use rusqlite::OptionalExtension;
use serde::{Serialize, de::DeserializeOwned};

use crate::{Error, Store};

/// The `settings` row the playback settings live in.
const PLAYBACK: &str = "playback";
/// The `settings` row holding how the library orders playlists and songs.
const SORTING: &str = "sorting";
/// The `settings` row holding the Client ID of the user's Spotify app.
const CLIENT_ID: &str = "client_id";

impl Store {
    /// The playback settings, or the defaults when none were saved or they no longer read.
    pub async fn playback(&self) -> Result<Playback, Error> {
        Ok(self.setting(PLAYBACK).await?.unwrap_or_default())
    }

    /// Saves the playback settings.
    pub async fn save_playback(&self, playback: &Playback) -> Result<(), Error> {
        self.save_setting(PLAYBACK, playback).await
    }

    /// How the library orders playlists and songs, or the defaults when none was saved.
    pub async fn sorting(&self) -> Result<Sorting, Error> {
        Ok(self.setting(SORTING).await?.unwrap_or_default())
    }

    /// Saves how the library orders playlists and songs.
    pub async fn save_sorting(&self, sorting: Sorting) -> Result<(), Error> {
        self.save_setting(SORTING, &sorting).await
    }

    /// The Client ID of the user's Spotify app, if one was saved.
    pub async fn client_id(&self) -> Result<Option<Box<str>>, Error> {
        self.setting(CLIENT_ID).await
    }

    /// Saves the Client ID of the user's Spotify app.
    pub async fn save_client_id(&self, client_id: &str) -> Result<(), Error> {
        self.save_setting(CLIENT_ID, client_id).await
    }

    /// The setting saved as `name`, or `None` when none was saved or it no longer reads.
    async fn setting<T: DeserializeOwned>(&self, name: &'static str) -> Result<Option<T>, Error> {
        let saved: Option<String> = self
            .call(move |conn| {
                conn.prepare_cached("SELECT value FROM settings WHERE name = ?1")?
                    .query_row([name], |row| row.get(0))
                    .optional()
            })
            .await?;
        Ok(saved.and_then(|json| {
            serde_json::from_str(&json)
                .inspect_err(|error| tracing::warn!(%error, name, "setting unreadable"))
                .ok()
        }))
    }

    async fn save_setting<T: Serialize + ?Sized>(
        &self,
        name: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        let json = serde_json::to_string(value)?;
        self.call(move |conn| {
            conn.prepare_cached(
                "INSERT INTO settings (name, value) VALUES (?1, ?2) \
                 ON CONFLICT (name) DO UPDATE SET value = excluded.value",
            )?
            .execute((name, json))
            .map(drop)
        })
        .await
    }
}
