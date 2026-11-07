//! Hush: a native Windows Spotify client.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod controller;
mod view;
#[allow(unsafe_code)]
mod windows;

use std::{env, path::PathBuf};

use anyhow::Context;
use slint::{
    ComponentHandle,
    winit_030::winit::{platform::windows::WindowAttributesExtWindows, window::Theme},
};

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();
    if !windows::claim_single_instance().context("single-instance mutex")? {
        tracing::info!("Hush is already running");
        return Ok(());
    }

    let cache_dir =
        PathBuf::from(env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is not set")?).join("Hush");
    // The OAuth token exchange must not follow redirects.
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(4)
        .enable_all()
        .build()?;

    // A dark theme gives the native title bar its dark mode.
    slint::BackendSelector::new()
        .with_winit_window_attributes_hook(|attributes| {
            attributes
                .with_theme(Some(Theme::Dark))
                .with_class_name(windows::WINDOW_CLASS)
        })
        .select()?;
    let window = ui::MainWindow::new()?;
    let deps = controller::Deps {
        cache_dir,
        http,
        view: view::View::new(&window),
    };
    let (controller, task) = {
        let _runtime = runtime.enter();
        controller::Handle::spawn(deps)
    };
    view::bind(&window, &controller);
    drop(controller);

    window.run()?;
    // The window's callbacks hold the last command senders; dropping it stops the
    // Controller, which removes the device from Spotify Connect.
    drop(window);
    runtime.block_on(task).context("controller task")?;
    Ok(())
}
