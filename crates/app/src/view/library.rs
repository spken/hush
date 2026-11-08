//! The library page: list keys and row text from the Controller, list choices and
//! visible rows back to it.

use model::{AlbumKind, AlbumRow, Command, Entity, List, PlaylistRow, TrackRow};
use slint::{ComponentHandle, ModelRc};
use ui::{KeyListModel, Library, MainWindow};

use super::{Loading, View, clock, count, int, thousands, with_list};
use crate::controller;

/// What the library page shows beside its list of lists.
#[derive(Debug)]
pub(crate) enum Open {
    LikedSongs,
    SavedAlbums,
    Playlist(PlaylistRow),
}

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
                .set_liked_detail(count(songs, "song", "songs").into());
        });
    }

    /// Shows the saved albums.
    pub(crate) fn saved_albums(&self, keys: Vec<u32>) {
        self.update(move |window| {
            let library = window.global::<Library>();
            let albums = u32::try_from(keys.len()).unwrap_or(u32::MAX);
            library.set_albums_detail(count(albums, "album", "albums").into());
            with_list(&library.get_albums(), |list| list.set_keys(keys));
        });
    }

    /// Shows `open` beside the list of lists: its header and its tracks' keys, which
    /// `loading` says may still be on their way.
    pub(crate) fn open(&self, open: Open, keys: Vec<u32>, loading: Loading) {
        self.update(move |window| {
            let library = window.global::<Library>();
            let entries = u32::try_from(keys.len()).unwrap_or(u32::MAX);
            let (key, title, detail, empty) = match open {
                Open::LikedSongs => (
                    0,
                    "Liked Songs".into(),
                    count(entries, "song", "songs"),
                    "Songs you like on Spotify appear here.",
                ),
                Open::SavedAlbums => (
                    -1,
                    "Albums".into(),
                    count(entries, "album", "albums"),
                    "Albums you save on Spotify appear here.",
                ),
                Open::Playlist(playlist) => {
                    let songs = if playlist.synced {
                        entries
                    } else {
                        playlist.tracks
                    };
                    let empty = match loading {
                        Loading::Pending => "Loading songs…",
                        Loading::Failed => "Couldn’t load this playlist. Check your connection.",
                        Loading::Done => "This playlist has no songs.",
                    };
                    (
                        int(playlist.key),
                        playlist.name,
                        format!("{} · {}", playlist.owner, count(songs, "song", "songs")),
                        empty,
                    )
                }
            };
            library.set_open_key(key);
            library.set_open_title(title.as_ref().into());
            library.set_open_detail(detail.into());
            library.set_open_empty(empty.into());
            if key != -1 {
                with_list(&library.get_tracks(), |list| list.set_keys(keys));
            }
        });
    }

    /// Fills in loaded track rows wherever they were asked for.
    pub(crate) fn track_rows(&self, rows: Vec<TrackRow>) {
        self.update(move |window| {
            let rows: Vec<(u32, ui::TrackRow)> = (rows.into_iter())
                .map(|row| {
                    let track = ui::TrackRow {
                        key: int(row.key),
                        title: row.name.as_ref().into(),
                        artists: row.artists.as_ref().into(),
                        artist_key: row.artist_key.map_or(0, int),
                        duration: clock(row.duration_ms).into(),
                    };
                    (row.key, track)
                })
                .collect();
            for model in [
                window.global::<Library>().get_tracks(),
                window.global::<ui::AlbumView>().get_tracks(),
                window.global::<ui::ArtistView>().get_top_tracks(),
            ] {
                with_list(&model, |list| list.fill(rows.iter().cloned()));
            }
        });
    }

    /// Fills in loaded album rows wherever they were asked for.
    pub(crate) fn album_rows(&self, rows: Vec<AlbumRow>) {
        self.update(move |window| {
            let rows: Vec<(u32, ui::AlbumRow)> = (rows.into_iter())
                .map(|row| {
                    let album = ui::AlbumRow {
                        key: int(row.key),
                        name: row.name.as_ref().into(),
                        artists: row.artists.as_ref().into(),
                        year: row
                            .year
                            .map(|year| year.to_string())
                            .unwrap_or_default()
                            .into(),
                        kind: kind(row.kind).into(),
                    };
                    (row.key, album)
                })
                .collect();
            for model in [
                window.global::<Library>().get_albums(),
                window.global::<ui::ArtistView>().get_albums(),
            ] {
                with_list(&model, |list| list.fill(rows.iter().cloned()));
            }
        });
    }

    /// Fills in loaded playlist rows.
    pub(crate) fn playlist_rows(&self, rows: Vec<PlaylistRow>) {
        self.update(move |window| {
            with_list(&window.global::<Library>().get_playlists(), |list| {
                list.fill(rows.into_iter().map(|row| {
                    let detail = format!("{} · {}", row.owner, count(row.tracks, "song", "songs"));
                    let playlist = ui::PlaylistRow {
                        key: int(row.key),
                        name: row.name.as_ref().into(),
                        detail: detail.into(),
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

/// An album kind as rows name it.
pub(super) fn kind(kind: AlbumKind) -> &'static str {
    match kind {
        AlbumKind::Album => "Album",
        AlbumKind::Single => "Single",
        AlbumKind::Compilation => "Compilation",
    }
}

/// The library list `key` names on the library page: 0 Liked Songs, -1 the saved albums,
/// else a playlist.
fn list(key: i32) -> List {
    match u32::try_from(key) {
        Ok(0) => List::LikedSongs,
        Ok(key) => List::Playlist(key),
        Err(_) => List::SavedAlbums,
    }
}

/// Gives the library page its list models and routes its callbacks to the Controller.
pub(super) fn bind(window: &MainWindow, controller: &controller::Handle) {
    let library = window.global::<Library>();
    library.set_tracks(ModelRc::from(KeyListModel::new(super::load(
        controller,
        Entity::Track,
    ))));
    library.set_albums(ModelRc::from(KeyListModel::new(super::load(
        controller,
        Entity::Album,
    ))));
    library.set_playlists(ModelRc::from(KeyListModel::new(super::load(
        controller,
        Entity::Playlist,
    ))));
    let open = controller.clone();
    library.on_open(move |key| open.send(Command::OpenList(list(key))));
    library.on_play(super::play(controller, list));
    let sync = controller.clone();
    library.on_sync(move || sync.send(Command::SyncLibrary));
}
