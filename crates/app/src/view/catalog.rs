//! Navigation and the album and artist pages.

use model::{AlbumRow, ArtistRow, Command, Entity, List, Route};
use slint::{ComponentHandle, ModelRc};
use ui::{AlbumView, ArtistView, KeyListModel, MainWindow, Nav, Page};

use super::{Loading, View, count, int, library::kind, with_list};
use crate::controller;

impl View {
    /// Shows the page `route` names, and whether Back leads anywhere.
    pub(crate) fn route(&self, route: Route, can_back: bool) {
        self.update(move |window| {
            let nav = window.global::<Nav>();
            nav.set_page(match route {
                Route::NowPlaying => Page::NowPlaying,
                Route::Search => Page::Search,
                Route::Library => Page::Library,
                Route::Album(_) => Page::Album,
                Route::Artist(_) => Page::Artist,
            });
            nav.set_can_back(can_back);
        });
    }

    /// Shows an album and its tracks' keys, which `loading` says may still be on their way.
    pub(crate) fn album(&self, album: AlbumRow, keys: Vec<u32>, loading: Loading) {
        self.update(move |window| {
            let view = window.global::<AlbumView>();
            let songs = u32::try_from(keys.len()).unwrap_or(u32::MAX);
            let mut detail: Vec<String> = album.year.iter().map(u16::to_string).collect();
            if songs > 0 {
                detail.push(count(songs, "song", "songs"));
            }
            view.set_key(int(album.key));
            view.set_label(kind(album.kind).to_uppercase().into());
            view.set_title(album.name.as_ref().into());
            view.set_artists(album.artists.as_ref().into());
            view.set_artist_key(album.artist_key.map_or(0, int));
            view.set_detail(detail.join(" · ").into());
            view.set_art(album.art.as_str().into());
            view.set_status(status(songs == 0, loading, "album").into());
            with_list(&view.get_tracks(), |list| list.set_keys(keys));
        });
    }

    /// Shows an artist, with the keys of its top tracks and albums, which `loading` says
    /// may still be on their way.
    pub(crate) fn artist(
        &self,
        artist: ArtistRow,
        (top, albums): (Vec<u32>, Vec<u32>),
        loading: Loading,
    ) {
        self.update(move |window| {
            let view = window.global::<ArtistView>();
            view.set_key(int(artist.key));
            view.set_name(artist.name.as_ref().into());
            view.set_art(artist.art.as_str().into());
            let empty = top.is_empty() && albums.is_empty();
            view.set_status(status(empty, loading, "artist").into());
            with_list(&view.get_top_tracks(), |list| list.set_keys(top));
            with_list(&view.get_albums(), |list| list.set_keys(albums));
        });
    }
}

/// What a catalog page says while it has nothing to show: `what` is "album" or "artist".
fn status(empty: bool, loading: Loading, what: &str) -> String {
    match loading {
        _ if !empty => String::new(),
        Loading::Pending => "Loading…".to_owned(),
        Loading::Failed => format!("Couldn’t load this {what}. Check your connection."),
        Loading::Done => format!("Spotify has nothing for this {what}."),
    }
}

/// Gives the album and artist pages their list models and routes navigation and their
/// callbacks to the Controller.
pub(super) fn bind(window: &MainWindow, controller: &controller::Handle) {
    let nav = window.global::<Nav>();
    let send = |route: fn(i32) -> Option<Route>| {
        let controller = controller.clone();
        move |key| {
            if let Some(route) = route(key) {
                controller.send(Command::Navigate(route));
            }
        }
    };
    nav.on_open_album(send(|key| u32::try_from(key).ok().map(Route::Album)));
    nav.on_open_artist(send(|key| u32::try_from(key).ok().map(Route::Artist)));
    let go = controller.clone();
    nav.on_go(move |page| {
        let route = match page {
            Page::Library | Page::Album | Page::Artist => Route::Library,
            Page::NowPlaying => Route::NowPlaying,
            Page::Search => Route::Search,
        };
        go.send(Command::Navigate(route));
    });
    let back = controller.clone();
    nav.on_back(move || back.send(Command::Back));

    let album = window.global::<AlbumView>();
    album.set_tracks(ModelRc::from(KeyListModel::new(super::load(
        controller,
        Entity::Track,
    ))));
    album.on_play(super::play(controller, |key| {
        List::Album(key.max(0).unsigned_abs())
    }));
    let artist = window.global::<ArtistView>();
    artist.set_top_tracks(ModelRc::from(KeyListModel::new(super::load(
        controller,
        Entity::Track,
    ))));
    artist.set_albums(ModelRc::from(KeyListModel::new(super::load(
        controller,
        Entity::Album,
    ))));
    artist.on_play(super::play(controller, |key| {
        List::ArtistTop(key.max(0).unsigned_abs())
    }));
}
