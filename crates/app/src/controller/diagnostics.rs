//! The Controller's playback diagnostics for the debug overlay: which device plays, and
//! the last few times playback was interrupted or moved.

use std::collections::VecDeque;

use super::Controller;
use crate::{view::Note, windows};

/// How many notes the debug overlay keeps.
const NOTES: usize = 8;

/// The latest notes, newest first.
#[derive(Debug, Default)]
pub(super) struct Diagnostics(VecDeque<Note>);

impl Controller {
    /// Logs `what` happened to playback and shows it in the debug overlay.
    pub(super) fn note(&mut self, what: &str) {
        tracing::info!(what, "playback");
        let notes = &mut self.diagnostics.0;
        notes.truncate(NOTES - 1);
        notes.push_front(Note {
            at: windows::local_time().into(),
            what: what.into(),
        });
        self.show_diagnostics();
    }

    /// Shows the device that plays and the latest notes in the debug overlay.
    pub(super) fn show_diagnostics(&self) {
        let device = match (&self.device, &self.remote.device) {
            (None, _) => "None; reconnecting",
            (Some(_), _) if self.active => "Hush",
            (Some(_), Some(remote)) => remote,
            (Some(_), None) => "None",
        };
        self.view
            .diagnostics(device, self.diagnostics.0.iter().cloned().collect());
    }
}
