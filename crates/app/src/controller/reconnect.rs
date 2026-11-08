//! The Controller's recovery side: the Spotify Connect device started again after its
//! session is lost, the catalog rebuilt on the new session, and degraded modes shown.

use std::io;

use catalog::CatalogService;
use model::Degraded;
use playback::{Device, Events};
use tokio::{
    task::{JoinError, JoinHandle},
    time,
};

use super::{Catalog, Controller};

impl Controller {
    /// Starts the catalog on the sources available: spclient through the device's session
    /// unless switched off, and the Web API once signed in.
    pub(super) fn start_catalog(&mut self) {
        let session = (self.device.as_ref())
            .filter(|_| self.spclient)
            .map(Device::session);
        let gateway = (self.library.as_ref()).map(|library| library.gateway.clone());
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
    }

    /// The device lost its session: playback stops here, and pages come from the database
    /// and the Web API until it reconnects.
    pub(super) fn disconnected(&mut self) {
        tracing::warn!("the Spotify Connect device lost its session; reconnecting");
        self.device = None;
        self.active = false;
        self.view.connecting();
        self.start_catalog();
        self.degrade(Degraded::Disconnected, true);
        self.attempts = 0;
        self.reconnecting = Some(self.reconnect());
    }

    /// Starts the device again after a delay that grows with each attempt, with the
    /// credentials librespot stored; it never opens the browser.
    pub(super) fn reconnect(&self) -> JoinHandle<Result<(Device, Events), playback::Error>> {
        let cache_dir = self.cache_dir.clone();
        let playback = self.playback.clone();
        let delay = kit::backoff(self.attempts);
        tokio::spawn(async move {
            time::sleep(delay).await;
            let login = async { Err::<String, _>(io::Error::other("no stored credentials")) };
            Device::start(cache_dir, login, &playback).await
        })
    }

    /// Takes in a finished reconnect: the device's events when it is back, or `None` after
    /// trying again later.
    pub(super) async fn reconnect_done(
        &mut self,
        joined: Result<Result<(Device, Events), playback::Error>, JoinError>,
    ) -> Option<Events> {
        self.reconnecting = None;
        match joined {
            Ok(Ok((device, events))) => {
                self.reconnected(device).await;
                Some(events)
            }
            failed => {
                if let Ok(Err(error)) = failed {
                    let attempts = self.attempts;
                    tracing::warn!(%error, attempts, "Spotify Connect device not reconnected");
                }
                self.attempts += 1;
                self.reconnecting = Some(self.reconnect());
                None
            }
        }
    }

    /// The device is back: it takes over playback commands and the catalog's spclient side.
    async fn reconnected(&mut self, device: Device) {
        tracing::info!("the Spotify Connect device reconnected");
        device.apply(&self.playback);
        self.device = Some(device);
        self.view.connected();
        self.start_catalog();
        self.degrade(Degraded::Disconnected, false);
        self.show_route().await;
    }

    /// Notes that `reason` started, or with `false` ended, and shows what still runs
    /// degraded.
    pub(super) fn degrade(&mut self, reason: Degraded, degraded: bool) {
        self.degraded.retain(|&other| other != reason);
        if degraded {
            tracing::warn!(?reason, "running degraded");
            self.degraded.push(reason);
        } else {
            tracing::info!(?reason, "recovered");
        }
        self.view.degraded(&self.degraded);
    }
}
