//! The Controller: the single owner of app state. It turns UI commands into device,
//! library and catalog calls, and playback and service events into view updates.

mod diagnostics;
mod library;
mod media;
mod pages;
mod reconnect;
mod remote;
mod search;
mod settings;

use std::{
    collections::HashMap,
    future,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use auth::TokenProvider;
use catalog::{CatalogService, Library};
use images::Images;
use model::{Anchor, Art, Command, Degraded, Event, List, PictureKey, Playback, Route, Sorting};
use playback::{Device, Events};
use store::Store;
use tokio::{
    sync::{
        mpsc::{self, Receiver, WeakSender},
        oneshot,
    },
    task::JoinHandle,
    time,
};
use tokio_util::sync::CancellationToken;
use webapi::{ApiGateway, Control, Lane};

use self::{
    diagnostics::Diagnostics,
    library::report,
    media::Overlay,
    remote::{Remote, volume_to_percent},
    search::Search,
};
use crate::view::{Loading, View};

/// How often the Controller checks whether the daily library sync is due. The check reads
/// the wall clock, so a sync missed while the computer slept runs soon after it wakes.
const SYNC_CHECK: Duration = Duration::from_mins(10);
/// The size of the canvas's blurred copy of the cover; Slint stretches it over the window.
const BACKDROP_PX: u32 = 384;

/// What the Controller is built from.
#[derive(Debug)]
pub(crate) struct Deps {
    /// Where librespot keeps its reusable credentials.
    pub(crate) cache_dir: PathBuf,
    /// Whether the catalog may come from spclient; off leaves it to the Web API.
    pub(crate) spclient: bool,
    pub(crate) http: reqwest::Client,
    pub(crate) store: Store,
    pub(crate) view: View,
    /// The window's entry in the Windows media overlay, once the window exists, if it
    /// could be created.
    pub(crate) media: oneshot::Receiver<Option<souvlaki::MediaControls>>,
}

/// Sends commands to the Controller, which stops once every handle is dropped.
#[derive(Debug, Clone)]
pub(crate) struct Handle {
    tx: mpsc::Sender<Command>,
}

impl Handle {
    /// Starts the Controller on the current tokio runtime; its task ends with whether Hush
    /// should start again.
    pub(crate) fn spawn(deps: Deps) -> (Self, JoinHandle<bool>) {
        let (tx, rx) = mpsc::channel(64);
        let weak = tx.downgrade();
        (Self { tx }, tokio::spawn(run(deps, rx, weak)))
    }

    /// Queues `command` without waiting, so the UI thread can call it.
    pub(crate) fn send(&self, command: Command) {
        if let Err(error) = self.tx.try_send(command) {
            tracing::warn!(%error, "command dropped");
        }
    }
}

/// Where catalog pages come from.
#[derive(Debug)]
enum Catalog {
    /// The device and the Web API are still starting.
    Starting,
    Ready(CatalogService),
    /// Neither spclient nor the Web API is available.
    Off,
}

struct Controller {
    view: View,
    store: Store,
    /// Where spawned work reports back.
    events: mpsc::Sender<Event>,
    anchor: Anchor,
    device: Option<Device>,
    /// Whether Hush is the active Spotify Connect device.
    active: bool,
    /// Set once the Web API is signed in.
    library: Option<Library>,
    catalog: Catalog,
    /// Catalog lookups that are running or failed, by list.
    fetches: HashMap<List, Loading>,
    /// Cancels the library sync while one runs.
    syncing: Option<CancellationToken>,
    /// The list the library page shows.
    open: List,
    /// How the library page orders playlists and songs, as saved.
    sorting: Sorting,
    route: Route,
    /// The pages Back returns to, oldest first.
    history: Vec<Route>,
    search: Search,
    images: Images,
    /// Cancels the picture loads for what the window showed before it last changed.
    pictures: CancellationToken,
    /// Cancels loading the backdrop of the track before.
    backdrop: CancellationToken,
    overlay: Option<Overlay>,
    /// The playback settings, as saved.
    playback: Playback,
    remote: Remote,
    /// What runs degraded, at most one of each.
    degraded: Vec<Degraded>,
    /// Where librespot keeps its reusable credentials, for reconnecting.
    cache_dir: PathBuf,
    /// Whether the catalog may come from spclient.
    spclient: bool,
    /// Starts the device again after it failed or lost its session.
    reconnecting: Option<JoinHandle<Result<(Device, Events), playback::Error>>>,
    /// Reconnects tried since the device last ran.
    attempts: u32,
    /// The ID the device had before it lost its session, which it keeps when it is back.
    device_id: Option<Box<str>>,
    diagnostics: Diagnostics,
    http: reqwest::Client,
    /// Whether Hush starts again once it stops, to run an installed update.
    restart: bool,
}

/// Runs the Controller until every `Handle` is dropped, then says whether Hush should start
/// again; `own` lets the media overlay send commands without keeping it alive.
async fn run(deps: Deps, mut commands: Receiver<Command>, own: WeakSender<Command>) -> bool {
    let Deps {
        cache_dir,
        spclient,
        http,
        store,
        view,
        media,
    } = deps;
    let (events, mut service_events) = mpsc::channel(64);
    let mut controller = Controller {
        view,
        store: store.clone(),
        events: events.clone(),
        anchor: Anchor::new(Instant::now()),
        device: None,
        active: false,
        library: None,
        catalog: Catalog::Starting,
        fetches: HashMap::new(),
        syncing: None,
        open: List::LikedSongs,
        sorting: Sorting::default(),
        route: Route::NowPlaying,
        history: Vec::new(),
        search: Search::new(),
        images: Images::new(http.clone(), cache_dir.join("images")),
        pictures: CancellationToken::new(),
        backdrop: CancellationToken::new(),
        overlay: None,
        playback: Playback::default(),
        remote: Remote::new(),
        degraded: Vec::new(),
        cache_dir: cache_dir.clone(),
        spclient,
        reconnecting: None,
        attempts: 0,
        device_id: None,
        diagnostics: Diagnostics::default(),
        http: http.clone(),
        restart: false,
    };
    match store.playback().await {
        Ok(playback) => controller.playback = playback,
        Err(error) => tracing::warn!(%error, "playback settings not read; using defaults"),
    }
    controller.view.playback(&controller.playback);
    let client_id = controller.client_id().await;
    controller.show_library().await;
    controller.show_search();
    if let Ok(Some(controls)) = media.await {
        controller.overlay = Overlay::new(controls, own);
    }

    // The two browser logins share a callback port, so they must not overlap.
    let playback = controller.playback.clone();
    let start = async {
        let library = web_api(client_id.as_deref(), http.clone(), store, events).await;
        let device = Device::start(cache_dir, auth::session_token(&http), &playback, None).await;
        (library, device)
    };
    tokio::pin!(start);
    let mut started = false;
    let mut player = None;
    let mut sync_check = time::interval(SYNC_CHECK);
    loop {
        tokio::select! {
            (library, device) = &mut start, if !started => {
                started = true;
                player = controller.started(library, device).await;
            }
            command = commands.recv() => {
                let Some(command) = command else { break };
                controller.command(command).await;
            }
            event = next(&mut player) => {
                if let Some(event) = event {
                    controller.event(event).await;
                } else {
                    player = None;
                    controller.disconnected();
                }
            }
            joined = joined(&mut controller.reconnecting) => {
                if let Some(events) = controller.reconnect_done(joined).await {
                    player = Some(events);
                }
            }
            Some(event) = service_events.recv() => controller.event(event).await,
            () = time::sleep_until(controller.remote.poll_at.unwrap_or_else(time::Instant::now)),
                if controller.remote.poll_at.is_some() => controller.poll(),
            _ = sync_check.tick() => controller.sync(false),
        }
    }
    controller.stop().await
}

/// The outcome of the reconnect running, if one is.
async fn joined<T>(task: &mut Option<JoinHandle<T>>) -> Result<T, tokio::task::JoinError> {
    match task {
        Some(task) => task.await,
        None => future::pending().await,
    }
}

/// The next player event, once the device has started.
async fn next(player: &mut Option<Events>) -> Option<Event> {
    match player {
        Some(events) => events.recv().await,
        None => future::pending().await,
    }
}

/// Signs in to the Web API and starts its gateway; `None`, after logging why, when the
/// Web API is not configured or the sign-in or profile read fails.
async fn web_api(
    client_id: Option<&str>,
    http: reqwest::Client,
    store: Store,
    events: mpsc::Sender<Event>,
) -> Option<Library> {
    let Some(client_id) = client_id else {
        tracing::warn!("no Client ID is set in Settings; the Web API is off");
        return None;
    };
    let tokens = match TokenProvider::new(client_id, http.clone()).await {
        Ok(tokens) => tokens,
        Err(error) => {
            tracing::error!(%error, "could not sign in to the Web API");
            return None;
        }
    };
    let gateway = ApiGateway::spawn(webapi::Deps {
        http,
        tokens: Arc::new(tokens),
        store: store.clone(),
        base_url: webapi::BASE_URL.into(),
        events: events.clone(),
    });
    let profile = match gateway.me(Lane::P1, CancellationToken::new()).await {
        Ok(profile) => profile,
        Err(error) => {
            tracing::error!(%error, "could not read the Spotify profile; the Web API is off");
            return None;
        }
    };
    tracing::info!(
        user = profile.display_name.as_deref().unwrap_or_default(),
        "signed in to the Web API"
    );
    let Some(user_id) = profile.id else {
        tracing::error!("the Spotify profile has no user ID; the Web API is off");
        return None;
    };
    Some(Library {
        gateway,
        store,
        user_id,
        events,
    })
}

impl Controller {
    /// Cancels the library sync and shuts the device down; returns whether Hush should
    /// start again.
    async fn stop(self) -> bool {
        if let Some(syncing) = self.syncing {
            syncing.cancel();
        }
        if let Some(device) = self.device
            && let Err(error) = device.shutdown().await
        {
            tracing::warn!(%error, "the Spotify Connect device did not shut down cleanly");
        }
        self.restart
    }

    /// Shows the saved Client ID and returns it, or `None` when none is set.
    async fn client_id(&self) -> Option<Box<str>> {
        let client_id = self.store.client_id().await.unwrap_or_else(|error| {
            tracing::warn!(%error, "Client ID not read; the Web API is off");
            None
        });
        self.view
            .client_id(client_id.as_deref().unwrap_or_default());
        client_id.filter(|id| !id.is_empty())
    }

    /// Takes in the Web API and the device once they have started, starts the catalog on
    /// what is available, and returns the device's events.
    async fn started(
        &mut self,
        library: Option<Library>,
        device: Result<(Device, Events), playback::Error>,
    ) -> Option<Events> {
        let events = match device {
            Ok((device, events)) => {
                tracing::info!("\"Hush\" is in the Spotify Connect device list");
                self.view.connected();
                // Settings changed while the device started.
                device.apply(&self.playback);
                self.device = Some(device);
                Some(events)
            }
            Err(error) => {
                tracing::error!(%error, "could not start the Spotify Connect device");
                self.view.failed(&error);
                self.reconnecting = Some(self.reconnect());
                None
            }
        };
        if !self.spclient {
            tracing::info!("HUSH_CATALOG is web-api; spclient is off for the catalog");
        }
        // After the device, so the sync can read playlist times from its session.
        if let Some(library) = library {
            self.view.web_api(library.gateway.metrics());
            self.library = Some(library);
        }
        self.sync(false);
        self.start_catalog();
        report(self.show_open().await);
        self.fetch_open();
        self.show_route().await;
        self.show_diagnostics();
        // Another device may be playing already.
        self.poll_in(Duration::ZERO);
        events
    }

    async fn command(&mut self, command: Command) {
        match command {
            Command::PlayPause => {
                let playing = !self.anchor.is_playing();
                self.set_playing(playing);
            }
            Command::SetPlaying(playing) => {
                if self.anchor.is_playing() != playing {
                    self.set_playing(playing);
                }
            }
            Command::Next if self.remote_active() => self.control_remote(Control::Next),
            Command::Next => {
                self.playback(Device::next);
            }
            Command::Previous if self.remote_active() => self.control_remote(Control::Previous),
            Command::Previous => {
                self.playback(Device::previous);
            }
            Command::Seek { position_ms } => {
                let sent = if self.remote_active() {
                    self.control_remote(Control::Seek(position_ms));
                    true
                } else {
                    self.playback(|device| device.seek(position_ms))
                };
                if sent {
                    // Move the bar now instead of when the player confirms the seek.
                    let seeked = Event::Seeked { position_ms };
                    self.anchor.apply(&seeked, Instant::now());
                    self.view.progress(self.anchor);
                }
            }
            Command::SetVolume(volume) => {
                let sent = if self.remote_active() {
                    self.control_remote(Control::Volume(volume_to_percent(volume)));
                    true
                } else {
                    self.playback(|device| device.set_volume(volume))
                };
                if sent {
                    self.view.volume(volume);
                }
            }
            Command::SetShuffle(shuffle) => {
                let sent = if self.remote_active() {
                    self.control_remote(Control::Shuffle(shuffle));
                    true
                } else {
                    self.playback(|device| device.set_shuffle(shuffle))
                };
                if sent {
                    self.view.shuffle(shuffle);
                }
            }
            Command::WindowFocused => self.poll_in(Duration::ZERO),
            Command::WindowVisible(visible) => {
                self.remote.visible = visible;
                if visible {
                    self.poll_in(Duration::ZERO);
                } else if !self.remote.resume {
                    self.remote.poll_at = None;
                }
            }
            Command::ListDevices => self.list_devices(),
            Command::Transfer(id) => self.transfer(&id),
            Command::Play {
                list,
                start,
                shuffle,
            } => report(self.play(list, start, shuffle).await),
            Command::Navigate(route) => self.navigate(route).await,
            Command::Back => self.back().await,
            Command::OpenList(list) => self.open_list(list).await,
            Command::SortPlaylists(order) => report(self.sort_playlists(order).await),
            Command::SortTracks(order) => report(self.sort_tracks(order).await),
            Command::PinPlaylist { key, pinned } => report(self.pin(key, pinned).await),
            Command::RowsVisible { entity, keys } => self.rows(entity, keys),
            Command::SyncLibrary => self.sync(true),
            Command::Search { query, now } => self.search(&query, now).await,
            Command::SearchMore(entity) => self.search_more(entity),
            Command::PlayFound(start) => report(self.play_found(start).await),
            Command::SetPlayback(playback) => report(self.set_playback(playback).await),
            Command::SetClientId(client_id) => {
                report(self.store.save_client_id(client_id.trim()).await);
            }
            Command::LoadPictures(keys) => {
                for key in keys {
                    self.load_picture(key);
                }
            }
            Command::Update => self.update(),
            Command::Restart => self.restart(),
        }
    }

    /// Resumes, or with `false` pauses, wherever playback is.
    fn set_playing(&mut self, playing: bool) {
        if self.remote_active() {
            self.control_remote(if playing {
                Control::Play
            } else {
                Control::Pause
            });
            let position_ms = self.anchor.position_at(Instant::now());
            let event = if playing {
                Event::Playing { position_ms }
            } else {
                Event::Paused { position_ms }
            };
            self.anchor.apply(&event, Instant::now());
            self.view.progress(self.anchor);
        } else {
            self.playback(Device::play_pause);
        }
    }

    /// Runs a playback command on the device; `false` when there is no device or it failed.
    fn playback(&self, command: impl FnOnce(&Device) -> Result<(), playback::Error>) -> bool {
        let Some(device) = &self.device else {
            tracing::debug!("no Spotify Connect device yet; command dropped");
            return false;
        };
        command(device)
            .inspect_err(|error| tracing::warn!(%error, "playback command failed"))
            .is_ok()
    }

    async fn event(&mut self, event: Event) {
        self.anchor.apply(&event, Instant::now());
        match event {
            Event::TrackChanged(track) => {
                self.show_backdrop(&track.art);
                if let Some(overlay) = &mut self.overlay {
                    overlay.track(Some(&track));
                }
                self.view.track(Some(track));
            }
            Event::Stopped => {
                self.note("Player stopped");
                // The track stays on screen while it may still resume here.
                if !self.remote.resume {
                    self.show_stopped();
                }
                self.view.progress(self.anchor);
            }
            Event::Volume(volume) => self.view.volume(volume),
            Event::Shuffle(shuffle) => self.view.shuffle(shuffle),
            Event::Active(active) => {
                self.active = active;
                if active {
                    self.note("Hush became the active device");
                    self.remote.resume = false;
                    self.remote.device = None;
                    self.remote.poll_at = None;
                    self.view.remote(None);
                } else {
                    self.deactivated();
                }
            }
            Event::Polled(player) => self.polled(player),
            Event::Degraded(reason) => self.degrade(reason, true),
            Event::Recovered(reason) => self.degrade(reason, false),
            Event::Playing { .. } | Event::Paused { .. } | Event::Seeked { .. } => {
                if let Some(overlay) = &mut self.overlay {
                    overlay.progress(self.anchor, Instant::now());
                }
                self.view.progress(self.anchor);
            }
            Event::Syncing { done, total } => self.view.syncing(Some((done, total))),
            Event::Synced => {
                self.syncing = None;
                self.view.syncing(None);
                self.show_recent();
            }
            Event::ListChanged(list) => report(self.list_changed(list).await),
            Event::Fetched { list, outcome } => report(self.fetched(list, outcome).await),
            Event::Searched { query, more, found } => self.searched(query, more, found),
        }
    }
}

impl Controller {
    /// Loads a picture for the UI, until what the window shows changes.
    fn load_picture(&self, key: PictureKey) {
        let images = self.images.clone();
        let view = self.view.clone();
        let cancel = self.pictures.clone();
        tokio::spawn(async move {
            match images.load(&key.url, key.px, key.radius, &cancel).await {
                Ok(picture) => view.picture(key, Some(picture), false),
                Err(images::Error::Cancelled) => view.picture(key, None, true),
                Err(error) => {
                    tracing::warn!(%error, url = &*key.url, "picture not loaded");
                    view.picture(key, None, false);
                }
            }
        });
    }

    /// Shows that nothing is loaded, with the playlists played last.
    fn show_stopped(&mut self) {
        self.show_backdrop(&Art::default());
        if let Some(overlay) = &mut self.overlay {
            overlay.track(None);
        }
        self.view.track(None);
        self.show_recent();
    }

    /// Floods the canvas with the cover `art` describes, or clears it when there is none.
    fn show_backdrop(&mut self, art: &Art) {
        self.backdrop.cancel();
        self.backdrop = CancellationToken::new();
        // The smallest size: it is blurred anyway.
        let Some(url) = Art::url(art.as_str(), 1) else {
            self.view.backdrop(None);
            return;
        };
        let url: Box<str> = url.into();
        let images = self.images.clone();
        let view = self.view.clone();
        let cancel = self.backdrop.clone();
        tokio::spawn(async move {
            match images.blurred(&url, BACKDROP_PX, &cancel).await {
                Ok(picture) => view.backdrop(Some(picture)),
                Err(images::Error::Cancelled) => {}
                Err(error) => {
                    tracing::warn!(%error, url = &*url, "backdrop not loaded");
                    view.backdrop(None);
                }
            }
        });
    }

    /// Cancels the picture loads still running for what the window showed.
    fn forget_pictures(&mut self) {
        self.pictures.cancel();
        self.pictures = CancellationToken::new();
    }
}
