//! The Controller: the single owner of app state. It turns UI commands into device,
//! library and catalog calls, and playback and service events into view updates.

mod library;
mod pages;
mod search;

use std::{
    collections::HashMap,
    future,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use auth::TokenProvider;
use catalog::{CatalogService, Library};
use model::{Anchor, Command, Event, List, Route};
use playback::{Device, Events};
use store::Store;
use tokio::{sync::mpsc, task::JoinHandle, time};
use tokio_util::sync::CancellationToken;
use webapi::{ApiGateway, Lane};

use self::{library::report, search::Search};
use crate::view::{Loading, View};

/// How often the Controller checks whether the daily library sync is due. The check reads
/// the wall clock, so a sync missed while the computer slept runs soon after it wakes.
const SYNC_CHECK: Duration = Duration::from_mins(10);

/// What the Controller is built from.
#[derive(Debug)]
pub(crate) struct Deps {
    /// Where librespot keeps its reusable credentials.
    pub(crate) cache_dir: PathBuf,
    /// Hush's Spotify app, for the Web API; without it the Web API stays off.
    pub(crate) client_id: Option<Box<str>>,
    /// Whether the catalog may come from spclient; off leaves it to the Web API.
    pub(crate) spclient: bool,
    pub(crate) http: reqwest::Client,
    pub(crate) store: Store,
    pub(crate) view: View,
}

/// Sends commands to the Controller, which stops once every handle is dropped.
#[derive(Debug, Clone)]
pub(crate) struct Handle {
    tx: mpsc::Sender<Command>,
}

impl Handle {
    /// Starts the Controller on the current tokio runtime.
    pub(crate) fn spawn(deps: Deps) -> (Self, JoinHandle<()>) {
        let (tx, rx) = mpsc::channel(64);
        (Self { tx }, tokio::spawn(run(deps, rx)))
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
    route: Route,
    /// The pages Back returns to, oldest first.
    history: Vec<Route>,
    search: Search,
}

async fn run(deps: Deps, mut commands: mpsc::Receiver<Command>) {
    let Deps {
        cache_dir,
        client_id,
        spclient,
        http,
        store,
        view,
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
        route: Route::NowPlaying,
        history: Vec::new(),
        search: Search::new(),
    };
    controller.show_library().await;
    controller.show_search();

    // The two browser logins share a callback port, so they must not overlap.
    let start = async {
        let library = web_api(client_id.as_deref(), http.clone(), store, events).await;
        let device = Device::start(cache_dir, auth::session_token(&http)).await;
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
                player = controller.started(library, device, spclient).await;
            }
            command = commands.recv() => {
                let Some(command) = command else { break };
                controller.command(command).await;
            }
            event = next(&mut player) => {
                let Some(event) = event else {
                    tracing::warn!("the player stopped");
                    break;
                };
                controller.event(event).await;
            }
            Some(event) = service_events.recv() => controller.event(event).await,
            _ = sync_check.tick() => controller.sync(false),
        }
    }
    if let Some(syncing) = controller.syncing {
        syncing.cancel();
    }
    if let Some(device) = controller.device
        && let Err(error) = device.shutdown().await
    {
        tracing::warn!(%error, "the Spotify Connect device did not shut down cleanly");
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
        tracing::warn!("HUSH_CLIENT_ID is not set; the Web API is off");
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
    /// Takes in the Web API and the device once they have started, starts the catalog on
    /// what is available, and returns the device's events.
    async fn started(
        &mut self,
        library: Option<Library>,
        device: Result<(Device, Events), playback::Error>,
        spclient: bool,
    ) -> Option<Events> {
        let gateway = library.as_ref().map(|library| library.gateway.clone());
        if let Some(library) = library {
            self.view.web_api(library.gateway.metrics());
            self.library = Some(library);
            self.sync(false);
        }
        let (session, events) = match device {
            Ok((device, events)) => {
                tracing::info!("\"Hush\" is in the Spotify Connect device list");
                self.view.connected();
                let session = spclient.then(|| device.session());
                self.device = Some(device);
                (session, Some(events))
            }
            Err(error) => {
                tracing::error!(%error, "could not start the Spotify Connect device");
                self.view.failed(&error);
                (None, None)
            }
        };
        if !spclient {
            tracing::info!("HUSH_CATALOG is web-api; spclient is off for the catalog");
        }
        self.catalog = if session.is_none() && gateway.is_none() {
            tracing::error!("no catalog source; album and artist pages stay empty");
            Catalog::Off
        } else {
            Catalog::Ready(CatalogService::spawn(catalog::Deps {
                store: self.store.clone(),
                session,
                gateway,
            }))
        };
        report(self.show_open().await);
        self.fetch_open();
        self.show_route().await;
        events
    }

    async fn command(&mut self, command: Command) {
        match command {
            Command::PlayPause => {
                self.playback(Device::play_pause);
            }
            Command::Next => {
                self.playback(Device::next);
            }
            Command::Previous => {
                self.playback(Device::previous);
            }
            Command::Seek { position_ms } => {
                if self.playback(|device| device.seek(position_ms)) {
                    // Move the bar now instead of when the player confirms the seek.
                    let seeked = Event::Seeked { position_ms };
                    self.anchor.apply(&seeked, Instant::now());
                    self.view.progress(self.anchor);
                }
            }
            Command::SetVolume(volume) => {
                if self.playback(|device| device.set_volume(volume)) {
                    self.view.volume(volume);
                }
            }
            Command::Play {
                list,
                start,
                shuffle,
            } => report(self.play(list, start, shuffle).await),
            Command::Navigate(route) => self.navigate(route).await,
            Command::Back => self.back().await,
            Command::OpenList(list) => {
                self.open = list;
                report(self.show_open().await);
                self.fetch_open();
            }
            Command::RowsVisible { entity, keys } => report(self.rows(entity, keys).await),
            Command::SyncLibrary => self.sync(true),
            Command::Search { query, now } => self.search(&query, now).await,
            Command::SearchMore(entity) => self.search_more(entity),
            Command::PlayFound(start) => report(self.play_found(start).await),
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
            Event::TrackChanged(track) => self.view.track(Some(track)),
            Event::Stopped => {
                self.view.track(None);
                self.view.progress(self.anchor);
            }
            Event::Volume(volume) => self.view.volume(volume),
            Event::Active(active) => self.active = active,
            Event::Degraded(reason) => tracing::warn!(?reason, "running degraded"),
            Event::Playing { .. } | Event::Paused { .. } | Event::Seeked { .. } => {
                self.view.progress(self.anchor);
            }
            Event::Syncing { done, total } => self.view.syncing(Some((done, total))),
            Event::Synced => {
                self.syncing = None;
                self.view.syncing(None);
            }
            Event::ListChanged(list) => report(self.list_changed(list).await),
            Event::Fetched { list, outcome } => report(self.fetched(list, outcome).await),
            Event::Searched { query, more, found } => self.searched(query, more, found),
        }
    }
}
