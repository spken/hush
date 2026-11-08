use std::{
    path::Path,
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::Connection;
use tokio::sync::{mpsc, oneshot};

/// Schema changes in order; a database at `user_version` n has the first n applied.
const MIGRATIONS: [&str; 8] = [
    include_str!("../migrations/0001_http_cache.sql"),
    include_str!("../migrations/0002_library.sql"),
    include_str!("../migrations/0003_catalog.sql"),
    include_str!("../migrations/0004_search.sql"),
    include_str!("../migrations/0005_art.sql"),
    include_str!("../migrations/0006_settings.sql"),
    include_str!("../migrations/0007_played.sql"),
    include_str!("../migrations/0008_playlist_order.sql"),
];

/// Ways a database call can fail.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("SQLite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("database thread: {0}")]
    Thread(#[from] std::io::Error),
    #[error("the database thread stopped")]
    Stopped,
    #[error("settings: {0}")]
    Json(#[from] serde_json::Error),
}

type Job = Box<dyn FnOnce(&Connection) + Send>;

/// A handle on the database. The one connection lives on its own thread, which stops once
/// every handle is dropped.
#[derive(Debug, Clone)]
pub struct Store {
    tx: mpsc::Sender<Job>,
}

impl Store {
    /// Opens or creates the database at `path`, brings its schema up to date and starts its
    /// thread. Blocks while it does.
    pub fn open(path: &Path) -> Result<Self, Error> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA cache_size = -2048; PRAGMA mmap_size = 0;",
        )?;
        migrate(&conn)?;
        let (tx, mut rx) = mpsc::channel::<Job>(64);
        thread::Builder::new().name("store".into()).spawn(move || {
            while let Some(job) = rx.blocking_recv() {
                job(&conn);
            }
        })?;
        Ok(Self { tx })
    }

    /// Runs `query` on the database thread.
    pub(crate) async fn call<T: Send + 'static>(
        &self,
        query: impl FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    ) -> Result<T, Error> {
        let (reply, rx) = oneshot::channel();
        let job: Job = Box::new(move |conn| {
            // A caller that stopped waiting no longer needs the result.
            let _ = reply.send(query(conn));
        });
        self.tx.send(job).await.map_err(|_| Error::Stopped)?;
        Ok(rx.await.map_err(|_| Error::Stopped)??)
    }
}

fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let applied: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    for (version, sql) in (1u32..).zip(MIGRATIONS).skip(applied as usize) {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", version)?;
        tx.commit()?;
    }
    Ok(())
}

/// Seconds since the Unix epoch, the time format the tables use.
pub(crate) fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        })
}
