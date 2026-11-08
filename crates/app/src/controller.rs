//! The Controller: the single owner of app state. It turns UI commands into device and
//! library calls, and playback and library events into view updates.

mod library;

use std::{
    future,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use auth::TokenProvider;
use catalog::Library;
use model::{Anchor, Command, Event, List};
use playback::{Device, Events};
use store::Store;
use tokio::{sync::mpsc, task::JoinHandle, time};
use tokio_util::sync::CancellationToken;
use webapi::{ApiGateway, Lane};

use self::library::report;
use crate::view::View;

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

struct Controller {
    view: View,
    store: Store,
    anchor: Anchor,
    device: Option<Device>,
    /// Set once the Web API is signed in.
    library: Option<Library>,
    /// Cancels the library sync while one runs.
    syncing: Option<CancellationToken>,
    /// The list whose tracks are on show.
    open: List,
}

async fn run(deps: Deps, mut commands: mpsc::Receiver<Command>) {
    let Deps {
        cache_dir,
        client_id,
        http,
        store,
        view,
    } = deps;
    let (events, mut service_events) = mpsc::channel(64);
    let mut controller = Controller {
        view,
        store: store.clone(),
        anchor: Anchor::new(Instant::now()),
        device: None,
        library: None,
        syncing: None,
        open: List::LikedSongs,
    };
    controller.show_library().await;

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
                player = controller.started(library, device);
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
    /// Takes in the Web API and the device once they have started, and returns the
    /// device's events.
    fn started(
        &mut self,
        library: Option<Library>,
        device: Result<(Device, Events), playback::Error>,
    ) -> Option<Events> {
        if let Some(library) = library {
            self.view.web_api(library.gateway.metrics());
            self.library = Some(library);
            self.sync(false);
        }
        match device {
            Ok((device, events)) => {
                tracing::info!("\"Hush\" is in the Spotify Connect device list");
                self.view.connected();
                self.device = Some(device);
                Some(events)
            }
            Err(error) => {
                tracing::error!(%error, "could not start the Spotify Connect device");
                self.view.failed(&error);
                None
            }
        }
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
            Command::OpenList(list) => {
                self.open = list;
                report(self.show_open().await);
            }
            Command::RowsVisible { entity, keys } => report(self.rows(entity, keys).await),
            Command::SyncLibrary => self.sync(true),
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
        }
    }
}
