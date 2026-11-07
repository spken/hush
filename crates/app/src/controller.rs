//! The Controller: the single owner of app state. It turns UI commands into device calls
//! and playback events into view updates.

use std::{path::PathBuf, sync::Arc, time::Instant};

use auth::TokenProvider;
use model::{Anchor, Command, Event};
use playback::Device;
use store::Store;
use tokio::{sync::mpsc, task::JoinHandle};
use tokio_util::sync::CancellationToken;
use webapi::{ApiGateway, Lane};

use crate::view::View;

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
    device: Device,
    view: View,
    anchor: Anchor,
}

async fn run(deps: Deps, mut commands: mpsc::Receiver<Command>) {
    let Deps {
        cache_dir,
        client_id,
        http,
        store,
        view,
    } = deps;
    let (api_events, mut service_events) = mpsc::channel(8);
    // The two browser logins share a callback port, so they must not overlap.
    let start = async {
        let gateway = web_api(client_id.as_deref(), http.clone(), store, api_events).await;
        let device = Device::start(cache_dir, auth::session_token(&http)).await;
        (gateway, device)
    };
    tokio::pin!(start);
    let (gateway, started) = loop {
        tokio::select! {
            started = &mut start => break started,
            // There is no device to act on yet; only a closed channel matters.
            command = commands.recv() => if command.is_none() { return },
        }
    };
    if let Some(gateway) = &gateway {
        view.web_api(gateway.metrics());
    }
    let (device, mut events) = match started {
        Ok(started) => started,
        Err(error) => {
            tracing::error!(%error, "could not start the Spotify Connect device");
            view.failed(&error);
            while commands.recv().await.is_some() {}
            return;
        }
    };
    tracing::info!("\"Hush\" is in the Spotify Connect device list");
    view.connected();

    let mut controller = Controller {
        device,
        view,
        anchor: Anchor::new(Instant::now()),
    };
    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break };
                controller.command(&command);
            }
            event = events.recv() => {
                let Some(event) = event else {
                    tracing::warn!("the player stopped");
                    break;
                };
                controller.event(event);
            }
            Some(event) = service_events.recv() => controller.event(event),
        }
    }
    if let Err(error) = controller.device.shutdown().await {
        tracing::warn!(%error, "the Spotify Connect device did not shut down cleanly");
    }
    drop(gateway);
}

/// Signs in to the Web API and starts its gateway; `None`, after logging why, when the
/// Web API is not configured or the sign-in fails.
async fn web_api(
    client_id: Option<&str>,
    http: reqwest::Client,
    store: Store,
    events: mpsc::Sender<Event>,
) -> Option<ApiGateway> {
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
        store,
        base_url: webapi::BASE_URL.into(),
        events,
    });
    match gateway.me(Lane::P1, CancellationToken::new()).await {
        Ok(profile) => tracing::info!(
            user = profile.display_name.as_deref().unwrap_or_default(),
            "signed in to the Web API"
        ),
        Err(error) => tracing::warn!(%error, "could not read the Spotify profile"),
    }
    Some(gateway)
}

impl Controller {
    fn command(&mut self, command: &Command) {
        let result = match *command {
            Command::PlayPause => self.device.play_pause(),
            Command::Next => self.device.next(),
            Command::Previous => self.device.previous(),
            Command::Seek { position_ms } => self.device.seek(position_ms).map(|()| {
                // Move the bar now instead of when the player confirms the seek.
                self.anchor
                    .apply(&Event::Seeked { position_ms }, Instant::now());
                self.view.progress(self.anchor);
            }),
            Command::SetVolume(volume) => self
                .device
                .set_volume(volume)
                .map(|()| self.view.volume(volume)),
        };
        if let Err(error) = result {
            tracing::warn!(%error, "playback command failed");
        }
    }

    fn event(&mut self, event: Event) {
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
        }
    }
}
