//! The library page: list keys and row text from the Controller, list choices and
//! visible rows back to it.

use model::{
    AlbumKind, AlbumRow, Command, Entity, List, PlaylistOrder, PlaylistRow, TrackOrder, TrackRow,
};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use ui::{KeyListModel, Library, MainWindow};

use super::{Loading, View, clock, count, int, thousands, with_list};
use crate::controller;

/// What the library page shows beside its list of lists.
#[derive(Debug)]
pub(crate) enum Open {
    LikedSongs,
    Playlist(PlaylistRow),
}

/// The orders the library offers for its playlists, as its sort menu names them.
const ORDERS: [(PlaylistOrder, &str); 5] = [
    (PlaylistOrder::Modified, "Recently Modified"),
    (PlaylistOrder::Played, "Recently Played"),
    (PlaylistOrder::Name, "Alphabetical"),
    (PlaylistOrder::Creator, "Creator"),
    (PlaylistOrder::Custom, "Custom Order"),
];

/// The orders the library offers for the songs of a list, as its sort menu names them.
const TRACK_ORDERS: [(TrackOrder, &str); 5] = [
    (TrackOrder::Custom, "Custom Order"),
    (TrackOrder::NewestAdded, "Recently Added"),
    (TrackOrder::OldestAdded, "Least Recently Added"),
    (TrackOrder::Title, "Title"),
    (TrackOrder::Artist, "Artist"),
];

impl View {
    /// Shows the user's playlists, the first `pinned` of them pinned and the rest in
    /// `order`.
    pub(crate) fn playlists(&self, keys: Vec<u32>, pinned: u32, order: PlaylistOrder) {
        let order = index(&ORDERS, order);
        self.update(move |window| {
            let library = window.global::<Library>();
            library.set_pinned_count(int(pinned));
            library.set_order(order);
            with_list(&library.get_playlists(), |list| list.set_keys(keys));
        });
    }

    /// Shows the playlists played last, most recent first.
    pub(crate) fn recent(&self, keys: Vec<u32>) {
        self.update(move |window| {
            with_list(&window.global::<Library>().get_recent(), |list| {
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

    /// Shows `open` beside the list of lists: its header and its tracks' keys in `order`,
    /// which `loading` says may still be on their way.
    pub(crate) fn open(&self, open: Open, keys: Vec<u32>, order: TrackOrder, loading: Loading) {
        let order = index(&TRACK_ORDERS, order);
        self.update(move |window| {
            let library = window.global::<Library>();
            library.set_track_order(order);
            let entries = u32::try_from(keys.len()).unwrap_or(u32::MAX);
            let mut art = SharedString::default();
            let (key, title, detail, empty) = match open {
                Open::LikedSongs => (
                    0,
                    "Liked Songs".into(),
                    count(entries, "song", "songs"),
                    "Songs you like on Spotify appear here.",
                ),
                Open::Playlist(playlist) => {
                    let songs = if playlist.synced {
                        entries
                    } else {
                        playlist.tracks
                    };
                    art = playlist.art.as_str().into();
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
            library.set_open_art(art);
            library.set_open_empty(empty.into());
            with_list(&library.get_tracks(), |list| list.set_keys(keys));
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
                        art: row.art.as_str().into(),
                    };
                    (row.key, track)
                })
                .collect();
            for model in [
                window.global::<Library>().get_tracks(),
                window.global::<ui::AlbumView>().get_tracks(),
                window.global::<ui::ArtistView>().get_top_tracks(),
                window.global::<ui::SearchView>().get_songs(),
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
                        art: row.art.as_str().into(),
                    };
                    (row.key, album)
                })
                .collect();
            for model in [
                window.global::<ui::ArtistView>().get_albums(),
                window.global::<ui::SearchView>().get_albums(),
            ] {
                with_list(&model, |list| list.fill(rows.iter().cloned()));
            }
        });
    }

    /// Fills in loaded playlist rows wherever they were asked for.
    pub(crate) fn playlist_rows(&self, rows: Vec<PlaylistRow>) {
        self.update(move |window| {
            let rows: Vec<(u32, ui::PlaylistRow)> = (rows.into_iter())
                .map(|row| {
                    let detail = format!("{} · {}", row.owner, count(row.tracks, "song", "songs"));
                    let playlist = ui::PlaylistRow {
                        key: int(row.key),
                        name: row.name.as_ref().into(),
                        detail: detail.into(),
                        art: row.art.as_str().into(),
                    };
                    (row.key, playlist)
                })
                .collect();
            for model in [
                window.global::<Library>().get_playlists(),
                window.global::<Library>().get_recent(),
                window.global::<ui::SearchView>().get_playlists(),
            ] {
                with_list(&model, |list| list.fill(rows.iter().cloned()));
            }
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

/// Where `order` is in a sort menu's `orders`, as a Slint `int`.
fn index<T: Copy + PartialEq>(orders: &[(T, &str)], order: T) -> i32 {
    (orders.iter())
        .position(|(each, _)| *each == order)
        .and_then(|index| i32::try_from(index).ok())
        .unwrap_or(0)
}

/// The names in a sort menu's `orders`, for Slint.
fn names<T>(orders: &[(T, &str)]) -> ModelRc<SharedString> {
    let names: Vec<SharedString> = orders.iter().map(|&(_, name)| name.into()).collect();
    ModelRc::new(VecModel::from(names))
}

/// The order at `index` of a sort menu's `orders`, if there is one.
fn chosen<T: Copy>(orders: &[(T, &str)], index: i32) -> Option<T> {
    let index = usize::try_from(index).ok()?;
    orders.get(index).map(|&(order, _)| order)
}

/// The library list `key` names on the library page: 0 Liked Songs, else a playlist.
fn list(key: i32) -> List {
    match u32::try_from(key) {
        Ok(key) if key > 0 => List::Playlist(key),
        _ => List::LikedSongs,
    }
}

/// Gives the library page its list models and routes its callbacks to the Controller.
pub(super) fn bind(window: &MainWindow, controller: &controller::Handle) {
    let library = window.global::<Library>();
    library.set_tracks(ModelRc::from(KeyListModel::new(super::load(
        controller,
        Entity::Track,
    ))));
    library.set_playlists(ModelRc::from(KeyListModel::new(super::load(
        controller,
        Entity::Playlist,
    ))));
    library.set_recent(ModelRc::from(KeyListModel::new(super::load(
        controller,
        Entity::Playlist,
    ))));
    library.set_orders(names(&ORDERS));
    library.set_track_orders(names(&TRACK_ORDERS));
    let open = controller.clone();
    library.on_open(move |key| open.send(Command::OpenList(list(key))));
    let sort = controller.clone();
    library.on_sort(move |index| {
        if let Some(order) = chosen(&ORDERS, index) {
            sort.send(Command::SortPlaylists(order));
        }
    });
    let sort_tracks = controller.clone();
    library.on_sort_tracks(move |index| {
        if let Some(order) = chosen(&TRACK_ORDERS, index) {
            sort_tracks.send(Command::SortTracks(order));
        }
    });
    let pin = controller.clone();
    library.on_pin(move |key, pinned| {
        if let Ok(key) = u32::try_from(key) {
            pin.send(Command::PinPlaylist { key, pinned });
        }
    });
    library.on_play(super::play(controller, list));
    let sync = controller.clone();
    library.on_sync(move || sync.send(Command::SyncLibrary));
}
