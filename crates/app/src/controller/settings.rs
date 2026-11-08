//! The Controller's settings side: playback settings kept in the database and applied to
//! the device, the output devices Settings offers, and updating Hush.

use model::Playback;

use super::Controller;
use crate::{update, view::Update};

impl Controller {
    /// Applies `playback` to the device and keeps it.
    pub(super) async fn set_playback(&mut self, playback: Playback) -> Result<(), store::Error> {
        if let Some(device) = &self.device {
            device.apply(&playback);
        }
        self.store.save_playback(&playback).await?;
        self.playback = playback;
        Ok(())
    }

    /// Lists the output devices on the settings page, in the background.
    pub(super) fn list_outputs(&self) {
        let view = self.view.clone();
        let chosen = self.playback.output.clone();
        tokio::spawn(async move {
            match tokio::task::spawn_blocking(playback::output_devices).await {
                Ok(devices) => view.outputs(devices, chosen),
                Err(error) => tracing::warn!(%error, "output devices not listed"),
            }
        });
    }

    /// Stops Hush and starts it again, to run an installed update.
    pub(super) fn restart(&mut self) {
        self.restart = true;
        self.view.quit();
    }

    /// Installs the newest release when it is newer than this Hush, in the background.
    pub(super) fn update(&self) {
        let view = self.view.clone();
        let http = self.http.clone();
        view.update_state(Update::Checking);
        tokio::spawn(async move {
            let installed = async {
                let Some(release) = update::newer(&http).await? else {
                    return Ok(None);
                };
                view.update_state(Update::Downloading(release.version.clone()));
                update::install(&http, &release).await?;
                anyhow::Ok(Some(release.version))
            };
            match installed.await {
                Ok(None) => view.update_state(Update::Latest),
                Ok(Some(version)) => {
                    tracing::info!(%version, "update installed");
                    view.update_state(Update::Ready(version));
                }
                Err(error) => {
                    tracing::warn!(error = format!("{error:#}"), "update failed");
                    view.update_state(Update::Failed);
                }
            }
        });
    }
}
