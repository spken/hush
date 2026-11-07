use std::time::Duration;

use rusqlite::OptionalExtension;

use crate::{Error, Store, db::unix_now};

/// A Web API response kept for reuse.
#[derive(Debug)]
pub struct CachedResponse {
    pub body: Vec<u8>,
    /// The `ETag` Spotify sent with it, to revalidate it with.
    pub etag: Option<Box<str>>,
    /// Whether it is still within its time to live.
    pub fresh: bool,
}

impl Store {
    /// The response stored for `url`, fresh or not.
    pub async fn cached_response(&self, url: String) -> Result<Option<CachedResponse>, Error> {
        self.call(move |conn| {
            conn.prepare_cached(
                "SELECT body, etag, expires_at > ?2 FROM http_cache WHERE url = ?1",
            )?
            .query_row((&url, unix_now()), |row| {
                Ok(CachedResponse {
                    body: row.get(0)?,
                    etag: row.get::<_, Option<String>>(1)?.map(String::into_boxed_str),
                    fresh: row.get(2)?,
                })
            })
            .optional()
        })
        .await
    }

    /// Stores `body` as the response for `url`, fresh for `ttl`.
    pub async fn store_response(
        &self,
        url: String,
        etag: Option<Box<str>>,
        body: impl AsRef<[u8]> + Send + 'static,
        ttl: Duration,
    ) -> Result<(), Error> {
        self.call(move |conn| {
            let ttl = i64::try_from(ttl.as_secs()).unwrap_or(i64::MAX);
            conn.prepare_cached(
                "INSERT OR REPLACE INTO http_cache (url, etag, body, expires_at) \
                 VALUES (?1, ?2, ?3, ?4)",
            )?
            .execute((
                &url,
                etag.as_deref(),
                body.as_ref(),
                unix_now().saturating_add(ttl),
            ))?;
            Ok(())
        })
        .await
    }
}
