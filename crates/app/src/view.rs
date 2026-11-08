//! The bridge between the Controller and the window: view-model updates in one direction,
//! UI callbacks in the other.

mod art;
mod catalog;
mod devices;
mod library;
mod queue;
mod search;
mod settings;

use std::{cell::Cell, fmt, time::Instant};

use model::{Anchor, Command, Degraded, Entity, List, Track};
use slint::{
    ComponentHandle, Model, ModelRc, SharedString, VecModel,
    winit_030::{EventResult, WinitWindowAccessor, winit::event::WindowEvent},
};
use tokio::sync::watch;
pub(crate) use ui::Note;
use ui::{Connection, EndpointRequests, Format, KeyListModel, MainWindow, Player};
use webapi::Metrics;

use crate::controller;

pub(crate) use self::{library::Open, settings::Update};

/// Whether a list whose entries are not all stored is being fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Loading {
    /// Nothing is missing.
    Done,
    Pending,
    /// Every catalog source failed.
    Failed,
}

/// The Controller's handle on the window; every update runs on the UI thread.
#[derive(Clone)]
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

    /// The device lost its session and is reconnecting.
    pub(crate) fn connecting(&self) {
        self.update(|window| {
            let player = window.global::<Player>();
            player.set_connection(Connection::Connecting);
            player.set_has_track(false);
        });
    }

    /// Says what runs degraded, if anything.
    pub(crate) fn degraded(&self, reasons: &[Degraded]) {
        let notice: Vec<&str> = (reasons.iter())
            .map(|reason| match reason {
                Degraded::QuotaExceeded => {
                    "Spotify’s request limit was reached; showing saved data for now."
                }
                Degraded::Disconnected => {
                    "Can’t reach Spotify. Reconnecting; what’s saved stays available."
                }
            })
            .collect();
        let notice = SharedString::from(notice.join(" "));
        self.update(move |window| window.global::<Player>().set_notice(notice));
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
                player.set_art(track.art.as_str().into());
            }
        });
    }

    /// Shows the play state and the position derived from `anchor`, moved on once a second
    /// as the track reaches each whole second.
    pub(crate) fn progress(&self, anchor: Anchor) {
        self.update(move |window| {
            let player = window.global::<Player>();
            player.set_playing(anchor.is_playing());
            let window = window.as_weak();
            let tick = move || {
                if let Some(window) = window.upgrade() {
                    let (position_ms, next_ms) = whole_second(anchor.position_at(Instant::now()));
                    let player = window.global::<Player>();
                    player.set_position_ms(int(position_ms));
                    player.set_next_tick(i64::from(next_ms));
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

    /// Shows whether shuffle is on.
    pub(crate) fn shuffle(&self, shuffle: bool) {
        self.update(move |window| window.global::<Player>().set_shuffle(shuffle));
    }

    /// Shows the device that plays and the latest playback notes, newest first, in the
    /// debug overlay.
    pub(crate) fn diagnostics(&self, device: &str, notes: Vec<Note>) {
        let device = SharedString::from(device);
        self.update(move |window| {
            let debug = window.global::<ui::Debug>();
            debug.set_playing_on(device);
            debug.set_playback_notes(ModelRc::new(VecModel::from(notes)));
        });
    }

    /// Shows the Web API gateway's counts in the debug overlay from now on.
    pub(crate) fn web_api(&self, metrics: watch::Receiver<Metrics>) {
        self.update(move |window| {
            let weak = window.as_weak();
            window.global::<ui::Debug>().on_sample(move || {
                if let Some(window) = weak.upgrade() {
                    sample(&window, Some(&metrics.borrow().clone()));
                }
            });
        });
    }

    /// Closes the window, which stops Hush.
    pub(crate) fn quit(&self) {
        self.update(|_| {
            if let Err(error) = slint::quit_event_loop() {
                tracing::warn!(%error, "window not closed");
            }
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
    let list = controller.clone();
    player.on_list_devices(move || list.send(Command::ListDevices));
    let shuffle = controller.clone();
    player.on_set_shuffle(move |on| shuffle.send(Command::SetShuffle(on)));
    let transfer = controller.clone();
    player.on_transfer(move |id| transfer.send(Command::Transfer(id.as_str().into())));
    window_state(window, controller);
    player.on_set_volume(move |volume| {
        set_volume.send(Command::SetVolume(
            u16::try_from(volume.max(0)).unwrap_or(u16::MAX),
        ));
    });

    window
        .global::<Format>()
        .on_clock(|position_ms| clock(position_ms.max(0).unsigned_abs()).into());
    library::bind(window, controller);
    catalog::bind(window, controller);
    search::bind(window, controller);
    queue::bind(window, controller);
    art::bind(window, controller);
    settings::bind(window, controller);

    let weak = window.as_weak();
    window.global::<ui::Debug>().on_sample(move || {
        if let Some(window) = weak.upgrade() {
            sample(&window, None);
        }
    });
}

/// Fills the debug overlay with memory use and, once the Web API is up, its counts.
fn sample(window: &MainWindow, web_api: Option<&Metrics>) {
    let debug = window.global::<ui::Debug>();
    if let Some(memory) = memory_stats::memory_stats() {
        debug.set_private_bytes(megabytes(memory.virtual_mem));
        debug.set_working_set(megabytes(memory.physical_mem));
    } else {
        tracing::warn!("memory stats unavailable");
    }
    let Some(metrics) = web_api else { return };
    debug.set_web_api_requests(int(metrics.requests));
    let lookups = u64::from(metrics.cache_hits) + u64::from(metrics.cache_misses);
    if let Some(rate) = (u64::from(metrics.cache_hits) * 100).checked_div(lookups) {
        debug.set_cache_hit_rate(format!("{rate}%").into());
    }
    debug.set_rate_limited(int(metrics.rate_limited));
    debug.set_quota_exceeded(int(metrics.quota_exceeded));
    let this_minute: Vec<EndpointRequests> = metrics
        .this_minute
        .iter()
        .map(|(endpoint, requests)| EndpointRequests {
            endpoint: endpoint.as_ref().into(),
            requests: int(*requests),
        })
        .collect();
    debug.set_this_minute(ModelRc::new(VecModel::from(this_minute)));
}

/// A list model callback that asks the Controller for the rows of `entity` it needs.
fn load(controller: &controller::Handle, entity: Entity) -> impl Fn(Vec<u32>) + 'static {
    let controller = controller.clone();
    move |keys| controller.send(Command::RowsVisible { entity, keys })
}

/// A `play(list key, start key, shuffle)` callback for the list `list` maps a key to; a
/// start key of 0 plays from the top.
fn play(
    controller: &controller::Handle,
    list: fn(i32) -> List,
) -> impl Fn(i32, i32, bool) + 'static {
    let controller = controller.clone();
    move |key, start, shuffle| {
        controller.send(Command::Play {
            list: list(key),
            start: u32::try_from(start).ok().filter(|&start| start != 0),
            shuffle,
        });
    }
}

/// Runs `change` on the key list behind `model`, which `bind` installed.
fn with_list<T: Clone + Default + 'static>(
    model: &ModelRc<T>,
    change: impl FnOnce(&KeyListModel<T>),
) {
    if let Some(list) = model.as_any().downcast_ref::<KeyListModel<T>>() {
        change(list);
    } else {
        tracing::error!("a list has no key list model");
    }
}

/// "1 song", "1,234 songs".
fn count(number: u32, one: &str, many: &str) -> String {
    match number {
        1 => format!("1 {one}"),
        number => format!("{} {many}", thousands(number)),
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

/// The whole second a track is at, in milliseconds, and how many milliseconds until the
/// next one. A tick that fires a little early still shows the second it was meant for.
fn whole_second(position_ms: u32) -> (u32, u32) {
    const EARLY_MS: u32 = 50;
    let second_ms = position_ms.saturating_add(EARLY_MS) / 1000 * 1000;
    (
        second_ms,
        second_ms.saturating_add(1000).saturating_sub(position_ms),
    )
}

/// A position or length in milliseconds as "m:ss".
fn clock(ms: u32) -> String {
    let seconds = ms / 1000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// A count, key or milliseconds as a Slint `int`, saturating at `i32::MAX`.
fn int(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// Bytes in MiB with one decimal, the unit Task Manager calls MB.
fn megabytes(bytes: usize) -> SharedString {
    const MIB: usize = 1 << 20;
    format!("{}.{} MB", bytes / MIB, bytes * 10 / MIB % 10).into()
}

/// Tells the Controller when the window gains the focus and when it is minimized or shown
/// again, which decides when another device's playback is read.
fn window_state(window: &MainWindow, controller: &controller::Handle) {
    let controller = controller.clone();
    let visible = Cell::new(true);
    window.window().on_winit_window_event(move |_, event| {
        match event {
            WindowEvent::Focused(true) => controller.send(Command::WindowFocused),
            // Windows reports a minimized window as resized to nothing.
            WindowEvent::Resized(size) => {
                let shown = size.width > 0 && size.height > 0;
                if visible.replace(shown) != shown {
                    controller.send(Command::WindowVisible(shown));
                }
            }
            _ => {}
        }
        EventResult::Propagate
    });
}
