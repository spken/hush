//! Win32 calls Slint and winit do not cover.

use std::{io, iter, ptr};

use windows_sys::Win32::{
    Foundation::{ERROR_ALREADY_EXISTS, GetLastError},
    System::Threading::CreateMutexW,
    UI::WindowsAndMessaging::{FindWindowW, IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow},
};

/// The window class Hush's main window registers, so a second launch can find it.
pub(crate) const WINDOW_CLASS: &str = "Hush";

const MUTEX_NAME: &str = "Local\\Hush.SingleInstance";

/// Claims the per-user single-instance mutex. Returns `false`, after bringing the running
/// instance's window to the front, when another Hush already holds it.
pub(crate) fn claim_single_instance() -> io::Result<bool> {
    let name = wide(MUTEX_NAME);
    // SAFETY: `name` is NUL-terminated and outlives the call; null security attributes
    // are allowed. The handle is never closed: the mutex must live as long as the process.
    let handle = unsafe { CreateMutexW(ptr::null(), 0, name.as_ptr()) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: reads this thread's last error, which `CreateMutexW` just set.
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        focus_running();
        return Ok(false);
    }
    Ok(true)
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
