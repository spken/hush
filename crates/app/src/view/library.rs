//! The library page: list keys and row text from the Controller, list choices and
//! visible rows back to it.

use model::{Command, Entity, List, PlaylistRow, TrackRow};
use slint::{ComponentHandle, Model, ModelRc};
use ui::{KeyListModel, Library, MainWindow};

use super::{View, clock, int};
use crate::controller;

impl View {
    /// Shows the user's playlists.
    pub(crate) fn playlists(&self, keys: Vec<u32>) {
        self.update(move |window| {
            with_list(&window.global::<Library>().get_playlists(), |list| {
                list.set_keys(keys);
            });
        });
    }

    /// Shows how many Liked Songs there are.
    pub(crate) fn liked(&self, songs: u32) {
        self.update(move |window| {
            window
                .global::<Library>()
                .set_liked_detail(count(songs).into());
        });
    }

    /// Opens a list's tracks under its header: `playlist`'s, or Liked Songs for `None`.
    pub(crate) fn open(&self, playlist: Option<PlaylistRow>, keys: Vec<u32>) {
        self.update(move |window| {
            let library = window.global::<Library>();
            let songs = u32::try_from(keys.len()).unwrap_or(u32::MAX);
            let (key, title, detail, empty) = match playlist {
                Some(playlist) if playlist.synced => (
                    playlist.key,
                    playlist.name,
                    format!("{} · {}", playlist.owner, count(songs)),
                    "This playlist has no songs.",
                ),
                Some(playlist) => (
                    playlist.key,
                    playlist.name,
                    format!("{} · {}", playlist.owner, count(playlist.tracks)),
                    "Hush syncs the songs of playlists you own or collaborate on.",
                ),
                None => (
                    0,
                    "Liked Songs".into(),
                    count(songs),
                    "Songs you like on Spotify appear here.",
                ),
            };
            library.set_open_key(int(key));
            library.set_open_title(title.as_ref().into());
            library.set_open_detail(detail.into());
            library.set_open_empty(empty.into());
            with_list(&library.get_tracks(), |list| list.set_keys(keys));
        });
    }

    /// Fills in loaded track rows.
    pub(crate) fn track_rows(&self, rows: Vec<TrackRow>) {
        self.update(move |window| {
            with_list(&window.global::<Library>().get_tracks(), |list| {
                list.fill(rows.into_iter().map(|row| {
                    let track = ui::TrackRow {
                        title: row.name.as_ref().into(),
                        artists: row.artists.as_ref().into(),
                        album: row.album.as_ref().into(),
                        duration: clock(row.duration_ms).into(),
                    };
                    (row.key, track)
                }));
            });
        });
    }

    /// Fills in loaded playlist rows.
    pub(crate) fn playlist_rows(&self, rows: Vec<PlaylistRow>) {
        self.update(move |window| {
            with_list(&window.global::<Library>().get_playlists(), |list| {
                list.fill(rows.into_iter().map(|row| {
                    let playlist = ui::PlaylistRow {
                        key: int(row.key),
                        name: row.name.as_ref().into(),
                        detail: format!("{} · {}", row.owner, count(row.tracks)).into(),
                    };
                    (row.key, playlist)
                }));
            });
        });
    }

    /// Shows library sync progress, as `(done, total)` with 0 / 0 while unknown, or that no
    /// sync runs.
    pub(crate) fn syncing(&self, progress: Option<(u32, u32)>) {
        self.update(move |window| {
            let library = window.global::<Library>();
            library.set_syncing(progress.is_some());
            let status = match progress {
                Some((done, total)) if total > 0 => {
                    format!(
                        "Syncing library… {} / {}",
                        thousands(done),
                        thousands(total)
                    )
                }
                _ => "Syncing library…".to_owned(),
            };
            library.set_sync_status(status.into());
        });
    }
}

/// Gives the library page its list models and routes its callbacks to the Controller.
pub(super) fn bind(window: &MainWindow, controller: &controller::Handle) {
    let library = window.global::<Library>();
    let load = |entity: Entity| {
        let controller = controller.clone();
        move |keys| controller.send(Command::RowsVisible { entity, keys })
    };
    library.set_tracks(ModelRc::from(KeyListModel::new(load(Entity::Track))));
    library.set_playlists(ModelRc::from(KeyListModel::new(load(Entity::Playlist))));
    let open = controller.clone();
    library.on_open(move |key| {
        let list = u32::try_from(key)
            .ok()
            .filter(|&key| key != 0)
            .map_or(List::LikedSongs, List::Playlist);
        open.send(Command::OpenList(list));
    });
    let sync = controller.clone();
    library.on_sync(move || sync.send(Command::SyncLibrary));
}

/// Runs `change` on the key list behind `model`, which `bind` installed.
fn with_list<T: Clone + Default + 'static>(
    model: &ModelRc<T>,
    change: impl FnOnce(&KeyListModel<T>),
) {
    if let Some(list) = model.as_any().downcast_ref::<KeyListModel<T>>() {
        change(list);
    } else {
        tracing::error!("a library list has no key list model");
    }
}

/// "1 song", "1,234 songs".
fn count(songs: u32) -> String {
    match songs {
        1 => "1 song".to_owned(),
        songs => format!("{} songs", thousands(songs)),
    }
}

/// A number with thousands separators.
fn thousands(number: u32) -> String {
    let digits = number.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}
