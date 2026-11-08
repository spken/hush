//! The queue panel: what plays next from the Controller, additions and the panel opening
//! or closing back to it.

use model::{Command, web};
use slint::{ComponentHandle, ModelRc, VecModel};
use ui::{MainWindow, Queue, TrackRow};

use super::{View, clock};
use crate::controller;

impl View {
    /// Shows `tracks` as what plays next. They are not in the library, so their rows have
    /// no keys.
    pub(crate) fn queue(&self, tracks: Vec<web::Track>) {
        self.update(move |window| {
            let rows: Vec<TrackRow> = (tracks.into_iter())
                .map(|track| {
                    let artists: Vec<&str> = (track.artists.iter())
                        .filter_map(|artist| artist.name.as_deref())
                        .collect();
                    TrackRow {
                        key: 0,
                        title: track.name.as_deref().unwrap_or_default().into(),
                        artists: artists.join(", ").into(),
                        artist_key: 0,
                        duration: clock(track.duration_ms.unwrap_or(0)).into(),
                        art: (track.album.as_ref())
                            .map(|album| web::Image::art(&album.images))
                            .unwrap_or_default()
                            .as_str()
                            .into(),
                    }
                })
                .collect();
            window
                .global::<Queue>()
                .set_tracks(ModelRc::new(VecModel::from(rows)));
        });
    }
}

pub(super) fn bind(window: &MainWindow, controller: &controller::Handle) {
    let queue = window.global::<Queue>();
    let add = controller.clone();
    queue.on_add(move |key| {
        if let Ok(key) = u32::try_from(key) {
            add.send(Command::Queue(key));
        }
    });
    let show = controller.clone();
    queue.on_show(move |open| show.send(Command::ShowQueue(open)));
}
