//! Playback on other Spotify Connect devices: which one plays, and the device picker.

use model::web;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use ui::{DeviceRow, Player};

use super::View;

impl View {
    /// Shows that playback is on the device named `name`, or on Hush for `None`.
    pub(crate) fn remote(&self, name: Option<Box<str>>) {
        self.update(move |window| {
            let name = name.as_deref().map_or_else(SharedString::new, Into::into);
            window.global::<Player>().set_remote_device(name);
        });
    }

    /// Lists `devices` in the device picker; the one with ID `own` is Hush.
    pub(crate) fn devices(&self, devices: Vec<web::Device>, own: Option<Box<str>>) {
        self.update(move |window| {
            let rows: Vec<DeviceRow> = (devices.into_iter())
                .filter_map(|device| {
                    let id = device.id?;
                    let detail = if own.as_ref() == Some(&id) {
                        "This computer".into()
                    } else {
                        device.kind.unwrap_or_default()
                    };
                    Some(DeviceRow {
                        id: id.as_ref().into(),
                        name: device.name.unwrap_or_default().as_ref().into(),
                        detail: detail.as_ref().into(),
                        active: device.is_active.unwrap_or(false),
                    })
                })
                .collect();
            window
                .global::<Player>()
                .set_devices(ModelRc::new(VecModel::from(rows)));
        });
    }
}
