//! Win32 calls Slint and winit do not cover.

use std::{ffi::c_void, io, iter, ptr};

use slint::winit_030::{
    WinitWindowAccessor,
    winit::raw_window_handle::{HasWindowHandle, RawWindowHandle},
};
use souvlaki::{MediaControls, PlatformConfig};

use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE},
    System::Threading::CreateMutexW,
    UI::WindowsAndMessaging::{FindWindowW, IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow},
};

/// The window class Hush's main window registers, so a second launch can find it.
pub(crate) const WINDOW_CLASS: &str = "Hush";

const MUTEX_NAME: &str = "Local\\Hush.SingleInstance";

/// The per-user single-instance mutex, held until dropped.
#[derive(Debug)]
pub(crate) struct Instance(HANDLE);

impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: the handle came from `CreateMutexW` and is closed only here.
        unsafe { CloseHandle(self.0) };
    }
}

/// Claims the per-user single-instance mutex. Returns `None`, after bringing the running
/// instance's window to the front, when another Hush already holds it.
pub(crate) fn claim_single_instance() -> io::Result<Option<Instance>> {
    let name = wide(MUTEX_NAME);
    // SAFETY: `name` is NUL-terminated and outlives the call; null security attributes
    // are allowed.
    let handle = unsafe { CreateMutexW(ptr::null(), 0, name.as_ptr()) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    let instance = Instance(handle);
    // SAFETY: reads this thread's last error, which `CreateMutexW` just set.
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        focus_running();
        return Ok(None);
    }
    Ok(Some(instance))
}

fn focus_running() {
    let class = wide(WINDOW_CLASS);
    // SAFETY: `class` is NUL-terminated and outlives the call; a null title matches any.
    let window = unsafe { FindWindowW(class.as_ptr(), ptr::null()) };
    if window.is_null() {
        return;
    }
    // SAFETY: `window` came from `FindWindowW`; these calls fail harmlessly if it has
    // closed since.
    unsafe {
        if IsIconic(window) != 0 {
            ShowWindow(window, SW_RESTORE);
        }
        SetForegroundWindow(window);
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(iter::once(0)).collect()
}

/// The native window behind `window`, once the event loop has created it.
pub(crate) fn native_window(window: &slint::Window) -> Option<*mut c_void> {
    let hwnd =
        window.with_winit_window(|window| match window.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
            _ => None,
        })??;
    Some(ptr::with_exposed_provenance_mut(hwnd.cast_unsigned()))
}

/// Hush's entry in the Windows media overlay, tied to the native window `hwnd`; `None`,
/// after logging why, when Windows refuses one.
pub(crate) fn media_controls(hwnd: *mut c_void) -> Option<MediaControls> {
    let config = PlatformConfig {
        display_name: "Hush",
        dbus_name: "hush",
        hwnd: Some(hwnd),
    };
    MediaControls::new(config)
        .inspect_err(|error| tracing::warn!(%error, "media overlay unavailable"))
        .ok()
}
