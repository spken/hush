//! The Controller's settings side: playback settings kept in the database and applied to
//! the device, and the output devices Settings offers.

use model::Playback;

use super::Controller;

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
}
