use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::SystemTime,
};

/// The most the cache may hold before its least recently used files are removed.
const CAP: u64 = 300 * 1024 * 1024;
/// What a trim leaves, so the next one is not due straight away.
const TRIM_TO: u64 = CAP / 10 * 9;

/// Encoded pictures by URL, one file each, never revalidated. A file's modified time is
/// when it was last used.
#[derive(Debug)]
pub(crate) struct Disk {
    state: Arc<State>,
}

#[derive(Debug)]
struct State {
    dir: PathBuf,
    /// Bytes on disk, as of the last trim plus what was written since.
    used: AtomicU64,
    trimming: AtomicBool,
}

impl Disk {
    /// The cache in `dir`, measured and trimmed in the background.
    pub(crate) fn open(dir: PathBuf) -> Self {
        let state = Arc::new(State {
            dir,
            used: AtomicU64::new(0),
            trimming: AtomicBool::new(true),
        });
        let first = Arc::clone(&state);
        tokio::task::spawn_blocking(move || {
            if let Err(error) = fs::create_dir_all(&first.dir) {
                tracing::warn!(%error, "image cache folder not created");
            }
            first.trim();
        });
        Self { state }
    }

    /// The cached bytes for `url`, marking them as used.
    pub(crate) async fn read(&self, url: &str) -> Option<Vec<u8>> {
        let path = self.state.path(url);
        let bytes = tokio::fs::read(&path).await.ok()?;
        let touched = tokio::task::spawn_blocking(move || {
            fs::File::options()
                .write(true)
                .open(&path)?
                .set_modified(SystemTime::now())
        })
        .await;
        if let Ok(Err(error)) = touched {
            tracing::debug!(%error, "image cache file not touched");
        }
        Some(bytes)
    }

    /// Caches `bytes` for `url`, trimming the cache when it grows past its cap.
    pub(crate) async fn write(&self, url: &str, bytes: &[u8]) {
        let path = self.state.path(url);
        let partial = path.with_extension("part");
        let written = async {
            tokio::fs::write(&partial, bytes).await?;
            tokio::fs::rename(&partial, &path).await
        };
        if let Err(error) = written.await {
            tracing::warn!(%error, "image not cached");
            return;
        }
        let used = self
            .state
            .used
            .fetch_add(bytes.len() as u64, Ordering::Relaxed);
        if used > CAP && !self.state.trimming.swap(true, Ordering::AcqRel) {
            let state = Arc::clone(&self.state);
            tokio::task::spawn_blocking(move || state.trim());
        }
    }
}

impl State {
    fn path(&self, url: &str) -> PathBuf {
        self.dir.join(format!("{:016x}", fnv1a(url)))
    }

    /// Measures the cache and removes the least recently used files while it is over its
    /// cap.
    fn trim(&self) {
        match files(&self.dir) {
            Ok(mut files) => {
                let mut used: u64 = files.iter().map(|(_, len, _)| len).sum();
                if used > CAP {
                    files.sort_unstable_by_key(|(used_at, _, _)| *used_at);
                    for (_, len, path) in files {
                        if used <= TRIM_TO {
                            break;
                        }
                        match fs::remove_file(&path) {
                            Ok(()) => used -= len,
                            Err(error) => tracing::warn!(%error, "image cache file not removed"),
                        }
                    }
                }
                self.used.store(used, Ordering::Relaxed);
            }
            Err(error) => tracing::warn!(%error, "image cache not measured"),
        }
        self.trimming.store(false, Ordering::Release);
    }
}

/// The cache's finished files: when each was last used, its length and its path.
fn files(dir: &Path) -> io::Result<Vec<(SystemTime, u64, PathBuf)>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().is_some() {
            continue;
        }
        let meta = fs::metadata(&path)?;
        files.push((meta.modified()?, meta.len(), path));
    }
    Ok(files)
}

/// A stable 64-bit hash: file names must not change between builds.
fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}
