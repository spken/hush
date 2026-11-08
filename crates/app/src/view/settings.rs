//! The settings page: playback settings and output devices from the Controller, changes
//! back to it.

use model::{Command, EQ_BANDS, OutputDevice, Playback};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use ui::{MainWindow, SettingsView, UpdateState};

use super::{View, int};
use crate::controller;

/// Where updating Hush stands.
#[derive(Debug)]
pub(crate) enum Update {
    Checking,
    /// This Hush is the newest release.
    Latest,
    /// Downloading this version.
    Downloading(Box<str>),
    /// This version is installed and runs once Hush restarts.
    Ready(Box<str>),
    Failed,
}

/// The equalizer presets: a name and a gain per band, in dB.
const PRESETS: [(&str, [f32; EQ_BANDS]); 7] = [
    ("Flat", [0.0; EQ_BANDS]),
    (
        "Bass Boost",
        [6.0, 5.0, 4.0, 2.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0],
    ),
    (
        "Treble Boost",
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.5, 2.0, 4.0, 5.0, 6.0],
    ),
    (
        "Vocal",
        [-2.0, -2.0, -1.0, 1.0, 3.0, 3.5, 3.0, 1.5, 0.0, -1.0],
    ),
    ("Rock", [4.0, 3.0, 1.5, 0.0, -1.0, -0.5, 1.0, 2.5, 3.5, 4.0]),
    (
        "Electronic",
        [5.0, 4.0, 1.5, 0.0, -1.5, 1.0, 0.5, 1.5, 4.0, 5.0],
    ),
    (
        "Acoustic",
        [3.0, 3.0, 2.0, 0.5, 1.0, 1.0, 2.0, 2.5, 2.0, 1.5],
    ),
];
/// The output entry that follows Windows' default device.
const SYSTEM_DEFAULT: &str = "System Default";
/// The preset entry shown for gains that match no preset.
const CUSTOM: &str = "Custom";

impl View {
    /// Shows the playback settings.
    pub(crate) fn playback(&self, playback: &Playback) {
        let playback = playback.clone();
        self.update(move |window| {
            let view = window.global::<SettingsView>();
            view.set_eq(playback.eq);
            view.set_preamp(playback.preamp_db);
            view.set_gains(ModelRc::new(VecModel::from(playback.gains_db.to_vec())));
            view.set_preset(preset(&playback.gains_db));
            view.set_normalisation(playback.normalisation);
            view.set_gapless(playback.gapless);
        });
    }

    /// Shows the saved Client ID.
    pub(crate) fn client_id(&self, client_id: &str) {
        let client_id = SharedString::from(client_id);
        self.update(move |window| window.global::<SettingsView>().set_client_id(client_id));
    }

    /// Shows where updating Hush stands.
    pub(crate) fn update_state(&self, update: Update) {
        self.update(move |window| {
            let view = window.global::<SettingsView>();
            let state = match update {
                Update::Checking => UpdateState::Checking,
                Update::Latest => UpdateState::Latest,
                Update::Downloading(version) => {
                    view.set_available(version.as_ref().into());
                    UpdateState::Downloading
                }
                Update::Ready(version) => {
                    view.set_available(version.as_ref().into());
                    UpdateState::Ready
                }
                Update::Failed => UpdateState::Failed,
            };
            view.set_update(state);
        });
    }

    /// Offers `devices` as outputs, with `chosen` selected, or the system default for
    /// `None`.
    pub(crate) fn outputs(&self, devices: Vec<OutputDevice>, chosen: Option<Box<str>>) {
        self.update(move |window| {
            let view = window.global::<SettingsView>();
            let selected = (chosen.as_ref())
                .and_then(|chosen| devices.iter().position(|device| &device.id == chosen))
                .map_or(0, |index| index + 1);
            let (ids, names): (Vec<SharedString>, Vec<SharedString>) = (devices.iter())
                .map(|device| (device.id.as_ref().into(), device.name.as_ref().into()))
                .unzip();
            let names: Vec<SharedString> = std::iter::once(SYSTEM_DEFAULT.into())
                .chain(names)
                .collect();
            view.set_output_ids(ModelRc::new(VecModel::from(ids)));
            view.set_outputs(ModelRc::new(VecModel::from(names)));
            view.set_output(int(u32::try_from(selected).unwrap_or(0)));
        });
    }
}

/// The index of the preset `gains` match in the preset list, or of "Custom" after it.
fn preset(gains: &[f32; EQ_BANDS]) -> i32 {
    let index = (PRESETS.iter())
        .position(|(_, preset)| {
            (preset.iter().zip(gains)).all(|(preset, gain)| (preset - gain).abs() < 0.01)
        })
        .unwrap_or(PRESETS.len());
    i32::try_from(index).unwrap_or(0)
}

/// The playback settings the page shows.
fn shown(view: &SettingsView) -> Playback {
    let mut gains_db = [0.0; EQ_BANDS];
    for (gain, shown) in gains_db.iter_mut().zip(view.get_gains().iter()) {
        *gain = shown;
    }
    let output = usize::try_from(view.get_output() - 1)
        .ok()
        .and_then(|index| view.get_output_ids().row_data(index))
        .map(|id| id.as_str().into());
    Playback {
        eq: view.get_eq(),
        preamp_db: view.get_preamp(),
        gains_db,
        normalisation: view.get_normalisation(),
        gapless: view.get_gapless(),
        output,
    }
}

/// Gives the settings page its preset names and routes its changes to the Controller.
pub(super) fn bind(window: &MainWindow, controller: &controller::Handle) {
    let view = window.global::<SettingsView>();
    let presets: Vec<SharedString> = (PRESETS.iter().map(|(name, _)| *name))
        .chain([CUSTOM])
        .map(SharedString::from)
        .collect();
    view.set_presets(ModelRc::new(VecModel::from(presets)));
    view.set_bands(ModelRc::new(VecModel::from(
        playback::EQ_BANDS_HZ
            .iter()
            .map(|&hz| band(hz))
            .collect::<Vec<_>>(),
    )));

    let client_id = controller.clone();
    view.on_client_id_edited(move |text| {
        client_id.send(Command::SetClientId(text.as_str().into()));
    });
    view.set_version(env!("CARGO_PKG_VERSION").into());
    let update = controller.clone();
    view.on_install_update(move || update.send(Command::Update));
    let restart = controller.clone();
    view.on_restart(move || restart.send(Command::Restart));

    let weak = window.as_weak();
    let controller = controller.clone();
    let send = move |change: &dyn Fn(&SettingsView)| {
        if let Some(window) = weak.upgrade() {
            let view = window.global::<SettingsView>();
            change(&view);
            view.set_preset(preset(&shown(&view).gains_db));
            controller.send(Command::SetPlayback(shown(&view)));
        }
    };
    let send = std::rc::Rc::new(send);
    let changed = std::rc::Rc::clone(&send);
    view.on_changed(move || changed(&|_| {}));
    let gain = std::rc::Rc::clone(&send);
    view.on_set_gain(move |band, db| {
        gain(&|view| {
            if let Ok(band) = usize::try_from(band) {
                view.get_gains().set_row_data(band, db);
            }
        });
    });
    view.on_choose_preset(move |index| {
        send(&|view| {
            if let Some((_, gains)) = usize::try_from(index)
                .ok()
                .and_then(|index| PRESETS.get(index))
            {
                view.set_gains(ModelRc::new(VecModel::from(gains.to_vec())));
            }
        });
    });
}

/// A band's centre frequency as its label: "31", "1k", "16k".
fn band(hz: u16) -> SharedString {
    if hz >= 1000 {
        format!("{}k", hz / 1000).into()
    } else {
        hz.to_string().into()
    }
}
