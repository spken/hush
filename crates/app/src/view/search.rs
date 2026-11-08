//! The search page: result keys and row text from the Controller, search text and choices
//! back to it.

use model::{ArtistRow, Command, Entity};
use slint::{ComponentHandle, ModelRc};
use ui::{KeyListModel, MainWindow, ResultKind, SearchView};

use super::{Loading, View, int, with_list};
use crate::controller;

impl View {
    /// Shows what a search for `query` found: per kind of result, in `Entity` order, the
    /// keys and whether Spotify has more; `loading` says whether Spotify's are on their way.
    /// An empty `query` searches nothing.
    pub(crate) fn search(&self, query: Box<str>, kinds: [(Vec<u32>, bool); 4], loading: Loading) {
        self.update(move |window| {
            let view = window.global::<SearchView>();
            let empty = kinds.iter().all(|(keys, _)| keys.is_empty());
            let status = match loading {
                _ if query.is_empty() => {
                    "Find songs, artists, albums and playlists: your library first, then all \
                     of Spotify."
                        .to_owned()
                }
                Loading::Pending if empty => "Searching…".to_owned(),
                Loading::Pending => "Searching Spotify…".to_owned(),
                Loading::Failed if empty => {
                    "Couldn’t search Spotify. Check your connection.".to_owned()
                }
                Loading::Failed => "Couldn’t reach Spotify; showing your library.".to_owned(),
                Loading::Done if empty => format!("Nothing found for “{query}”."),
                Loading::Done => String::new(),
            };
            view.set_status(status.into());
            let [
                (tracks, more_tracks),
                (albums, more_albums),
                (artists, more_artists),
                (playlists, more_playlists),
            ] = kinds;
            view.set_more_songs(more_tracks);
            view.set_more_albums(more_albums);
            view.set_more_artists(more_artists);
            view.set_more_playlists(more_playlists);
            with_list(&view.get_songs(), |list| list.set_keys(tracks));
            with_list(&view.get_albums(), |list| list.set_keys(albums));
            with_list(&view.get_artists(), |list| list.set_keys(artists));
            with_list(&view.get_playlists(), |list| list.set_keys(playlists));
        });
    }

    /// Fills in loaded artist rows.
    pub(crate) fn artist_rows(&self, rows: Vec<ArtistRow>) {
        self.update(move |window| {
            with_list(&window.global::<SearchView>().get_artists(), |list| {
                list.fill(rows.into_iter().map(|row| {
                    let artist = ui::ArtistRow {
                        key: int(row.key),
                        name: row.name.as_ref().into(),
                    };
                    (row.key, artist)
                }));
            });
        });
    }
}

/// Gives the search page its list models and routes its callbacks to the Controller.
pub(super) fn bind(window: &MainWindow, controller: &controller::Handle) {
    let search = window.global::<SearchView>();
    search.set_songs(ModelRc::from(KeyListModel::new(super::load(
        controller,
        Entity::Track,
    ))));
    search.set_artists(ModelRc::from(KeyListModel::new(super::load(
        controller,
        Entity::Artist,
    ))));
    search.set_albums(ModelRc::from(KeyListModel::new(super::load(
        controller,
        Entity::Album,
    ))));
    search.set_playlists(ModelRc::from(KeyListModel::new(super::load(
        controller,
        Entity::Playlist,
    ))));
    let send = |now: bool| {
        let controller = controller.clone();
        move |text: slint::SharedString| {
            controller.send(Command::Search {
                query: text.as_str().into(),
                now,
            });
        }
    };
    search.on_edited(send(false));
    search.on_accepted(send(true));
    let more = controller.clone();
    search.on_more(move |kind| {
        more.send(Command::SearchMore(match kind {
            ResultKind::Songs => Entity::Track,
            ResultKind::Artists => Entity::Artist,
            ResultKind::Albums => Entity::Album,
            ResultKind::Playlists => Entity::Playlist,
        }));
    });
    let play = controller.clone();
    search.on_play(move |key| {
        if let Ok(key) = u32::try_from(key) {
            play.send(Command::PlayFound(key));
        }
    });
}
