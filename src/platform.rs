//! Win32 helpers: single instance, showing the main window from other
//! threads, opening links and folders, and the machine's architecture.

use crate::github::Arch;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicIsize, Ordering};
use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HWND};
use windows_sys::Win32::System::SystemInformation::{
    IMAGE_FILE_MACHINE, IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_ARM64, IMAGE_FILE_MACHINE_I386,
};
use windows_sys::Win32::System::Threading::{CreateMutexW, GetCurrentProcess, IsWow64Process2};
use windows_sys::Win32::UI::Shell::{SetCurrentProcessExplicitAppUserModelID, ShellExecuteW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FindWindowW, IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW, SW_SHOWNORMAL,
};

static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);

fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

/// Returns false if another copy of the app is already running.
pub fn claim_single_instance(name: &str) -> bool {
    let name = wide(format!("Local\\{name}.SingleInstance"));
    // The handle is deliberately never closed: it marks this process for its lifetime.
    unsafe {
        CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        GetLastError() != ERROR_ALREADY_EXISTS
    }
}

/// Brings the window of an already-running copy to the front.
pub fn activate_existing_window(title: &str) {
    let title = wide(title);
    let hwnd = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
    if !hwnd.is_null() {
        show_window(hwnd);
    }
}

pub fn set_app_user_model_id(id: &str) {
    let id = wide(id);
    unsafe {
        SetCurrentProcessExplicitAppUserModelID(id.as_ptr());
    }
}

pub fn set_main_window(cc: &eframe::CreationContext<'_>) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    if let Ok(handle) = cc.window_handle() {
        if let RawWindowHandle::Win32(h) = handle.as_raw() {
            MAIN_HWND.store(h.hwnd.get(), Ordering::SeqCst);
        }
    }
}

fn main_hwnd() -> HWND {
    MAIN_HWND.load(Ordering::SeqCst) as HWND
}

/// Shows and focuses the main window. Safe to call from any thread; the UI
/// thread notices the change on its next frame and syncs its own state.
pub fn show_main_window() {
    let hwnd = main_hwnd();
    if !hwnd.is_null() {
        show_window(hwnd);
    }
}

fn show_window(hwnd: HWND) {
    unsafe {
        ShowWindow(hwnd, if IsIconic(hwnd) != 0 { SW_RESTORE } else { SW_SHOW });
        SetForegroundWindow(hwnd);
    }
}

pub fn is_main_window_visible() -> bool {
    let hwnd = main_hwnd();
    !hwnd.is_null() && unsafe { IsWindowVisible(hwnd) } != 0
}

/// Opens a URL in the default browser, or a folder in Explorer.
pub fn open(target: impl AsRef<OsStr>) {
    let verb = wide("open");
    let target = wide(target);
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// The machine's native architecture, even when this build runs emulated.
pub fn native_arch() -> Arch {
    let mut process: IMAGE_FILE_MACHINE = 0;
    let mut native: IMAGE_FILE_MACHINE = 0;
    let ok = unsafe { IsWow64Process2(GetCurrentProcess(), &mut process, &mut native) } != 0;
    match native {
        IMAGE_FILE_MACHINE_ARM64 if ok => Arch::Arm64,
        IMAGE_FILE_MACHINE_AMD64 if ok => Arch::X64,
        IMAGE_FILE_MACHINE_I386 if ok => Arch::X86,
        _ if cfg!(target_arch = "aarch64") => Arch::Arm64,
        _ if cfg!(target_arch = "x86") => Arch::X86,
        _ => Arch::X64,
    }
}
