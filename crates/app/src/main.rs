//! Hush: a native Windows Spotify client.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod controller;
mod view;
#[allow(unsafe_code)]
mod windows;

use std::{env, fs, path::PathBuf};

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

    if let Err(error) = dotenvy::dotenv()
        && !error.not_found()
    {
        tracing::warn!(%error, ".env not loaded");
    }
    let client_id = env::var("HUSH_CLIENT_ID")
        .ok()
        .filter(|id| !id.is_empty())
        .map(String::into_boxed_str);
    // `HUSH_CATALOG=web-api` leaves album, artist and playlist pages to the Web API.
    let spclient = env::var("HUSH_CATALOG").map_or(true, |source| source != "web-api");
    let cache_dir =
        PathBuf::from(env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is not set")?).join("Hush");
    fs::create_dir_all(&cache_dir).context("data folder")?;
    let store = store::Store::open(&cache_dir.join("hush.db")).context("database")?;
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
        client_id,
        spclient,
        http,
        store,
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
