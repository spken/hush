//! The bridge between the Controller and the window: view-model updates in one direction,
//! UI callbacks in the other.

use std::{fmt, time::Instant};

use model::{Anchor, Command, Track};
use slint::{ComponentHandle, SharedString};
use ui::{Connection, Format, MainWindow, Player};

use crate::controller;

/// The Controller's handle on the window; every update runs on the UI thread.
pub(crate) struct View(slint::Weak<MainWindow>);

impl View {
    pub(crate) fn new(window: &MainWindow) -> Self {
        Self(window.as_weak())
    }

    /// The device is registered and ready to play.
    pub(crate) fn connected(&self) {
        self.update(|window| {
            window.global::<Player>().set_connection(Connection::Ready);
        });
    }

    /// The device could not start; `error` says why.
    pub(crate) fn failed(&self, error: &impl fmt::Display) {
        let error = SharedString::from(error.to_string());
        self.update(move |window| {
            let player = window.global::<Player>();
            player.set_connection(Connection::Failed);
            player.set_error(error);
        });
    }

    /// Shows the loaded track, or that nothing is loaded.
    pub(crate) fn track(&self, track: Option<Track>) {
        self.update(move |window| {
            let player = window.global::<Player>();
            player.set_has_track(track.is_some());
            if let Some(track) = track {
                player.set_title(track.name.as_ref().into());
                player.set_artists(track.artists.join(", ").into());
                player.set_duration_ms(int(track.duration_ms));
            }
        });
    }

    /// Shows the play state and, on every UI progress tick, the position derived from
    /// `anchor`.
    pub(crate) fn progress(&self, anchor: Anchor) {
        self.update(move |window| {
            let player = window.global::<Player>();
            player.set_playing(anchor.is_playing());
            let window = window.as_weak();
            let tick = move || {
                if let Some(window) = window.upgrade() {
                    let position_ms = anchor.position_at(Instant::now());
                    window.global::<Player>().set_position_ms(int(position_ms));
                }
            };
            tick();
            player.on_tick(tick);
        });
    }

    /// Shows the device volume, from 0 to `u16::MAX`.
    pub(crate) fn volume(&self, volume: u16) {
        self.update(move |window| {
            window.global::<Player>().set_volume(i32::from(volume));
        });
    }

    fn update(&self, change: impl FnOnce(MainWindow) + Send + 'static) {
        // Fails only after the event loop quit: the window is gone and needs no update.
        if let Err(error) = self.0.upgrade_in_event_loop(change) {
            tracing::debug!(%error, "view update dropped");
        }
    }
}

impl fmt::Debug for View {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("View").finish_non_exhaustive()
    }
}

/// Routes the window's callbacks to the Controller and to Rust-side formatting.
pub(crate) fn bind(window: &MainWindow, controller: &controller::Handle) {
    let player = window.global::<Player>();
    player.set_volume_max(i32::from(u16::MAX));
    let send = |command: fn() -> Command| {
        let controller = controller.clone();
        move || controller.send(command())
    };
    player.on_play_pause(send(|| Command::PlayPause));
    player.on_next(send(|| Command::Next));
    player.on_previous(send(|| Command::Previous));
    let seek = controller.clone();
    player.on_seek(move |position_ms| {
        seek.send(Command::Seek {
            position_ms: position_ms.max(0).unsigned_abs(),
        });
    });
    let set_volume = controller.clone();
    player.on_set_volume(move |volume| {
        set_volume.send(Command::SetVolume(
            u16::try_from(volume.max(0)).unwrap_or(u16::MAX),
        ));
    });

    window.global::<Format>().on_clock(|position_ms| {
        let seconds = position_ms.max(0) / 1000;
        format!("{}:{:02}", seconds / 60, seconds % 60).into()
    });

    let weak = window.as_weak();
    window.global::<ui::Debug>().on_sample(move || {
        let Some(window) = weak.upgrade() else { return };
        let Some(memory) = memory_stats::memory_stats() else {
            tracing::warn!("memory stats unavailable");
            return;
        };
        let debug = window.global::<ui::Debug>();
        debug.set_private_bytes(megabytes(memory.virtual_mem));
        debug.set_working_set(megabytes(memory.physical_mem));
    });
}

/// Milliseconds as a Slint `int`, saturating past ~24 days.
fn int(ms: u32) -> i32 {
    i32::try_from(ms).unwrap_or(i32::MAX)
}

/// Bytes in MiB with one decimal, the unit Task Manager calls MB.
fn megabytes(bytes: usize) -> SharedString {
    const MIB: usize = 1 << 20;
    format!("{}.{} MB", bytes / MIB, bytes * 10 / MIB % 10).into()
}
