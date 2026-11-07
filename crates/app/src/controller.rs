//! The Controller: the single owner of app state. It turns UI commands into device calls
//! and playback events into view updates.

use std::{path::PathBuf, time::Instant};

use model::{Anchor, Command, Event};
use playback::Device;
use tokio::{sync::mpsc, task::JoinHandle};

use crate::view::View;

/// What the Controller is built from.
#[derive(Debug)]
pub(crate) struct Deps {
    /// Where librespot keeps its reusable credentials.
    pub(crate) cache_dir: PathBuf,
    pub(crate) http: reqwest::Client,
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
        http,
        view,
    } = deps;
    let start = Device::start(cache_dir, auth::session_token(&http));
    tokio::pin!(start);
    let (device, mut events) = loop {
        tokio::select! {
            started = &mut start => match started {
                Ok(started) => break started,
                Err(error) => {
                    tracing::error!(%error, "could not start the Spotify Connect device");
                    view.failed(&error);
                    while commands.recv().await.is_some() {}
                    return;
                }
            },
            // There is no device to act on yet; only a closed channel matters.
            command = commands.recv() => if command.is_none() { return },
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
        }
    }
    if let Err(error) = controller.device.shutdown().await {
        tracing::warn!(%error, "the Spotify Connect device did not shut down cleanly");
    }
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
            Event::Playing { .. } | Event::Paused { .. } | Event::Seeked { .. } => {
                self.view.progress(self.anchor);
            }
        }
    }
}
