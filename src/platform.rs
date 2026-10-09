//! Platform helpers: single instance, showing the main window from other
//! threads, opening links and folders, the machine's architecture, and the
//! wording that differs between Windows and macOS.

pub use imp::*;

/// Picks the Windows or the macOS wording.
pub const fn os_text(windows: &'static str, macos: &'static str) -> &'static str {
    if cfg!(target_os = "macos") {
        macos
    } else {
        windows
    }
}

pub const OS_NAME: &str = os_text("Windows", "macOS");

#[cfg(windows)]
mod imp {
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

    /// Only macOS has a Dock icon to hide while the app is in the tray.
    pub fn set_in_dock(_visible: bool) {}

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
}

#[cfg(target_os = "macos")]
mod imp {
    use crate::github::Arch;
    use objc2::rc::Retained;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSView, NSWindow};
    use std::ffi::OsStr;
    use std::fs::{File, TryLockError};
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::OnceLock;

    /// The window's NSView. Only ever dereferenced on the main thread.
    static MAIN_VIEW: AtomicUsize = AtomicUsize::new(0);
    static INSTANCE_LOCK: OnceLock<File> = OnceLock::new();

    /// Returns false if another copy of the app is already running for this
    /// user. The lock file stays locked for the life of the process.
    pub fn claim_single_instance(name: &str) -> bool {
        let Ok(file) = File::create(std::env::temp_dir().join(format!("{name}.lock"))) else {
            return true;
        };
        match file.try_lock() {
            Ok(()) => {
                let _ = INSTANCE_LOCK.set(file);
                true
            }
            Err(TryLockError::WouldBlock) => false,
            Err(_) => true,
        }
    }

    /// Opening the app again from Finder or Launchpad reaches the running copy
    /// through macOS, so a second process has nothing to do.
    pub fn activate_existing_window(_title: &str) {}

    pub fn set_app_user_model_id(_id: &str) {}

    pub fn set_main_window(cc: &eframe::CreationContext<'_>) {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(handle) = cc.window_handle() {
            if let RawWindowHandle::AppKit(h) = handle.as_raw() {
                MAIN_VIEW.store(h.ns_view.as_ptr() as usize, Ordering::SeqCst);
            }
        }
    }

    fn main_window(_mtm: MainThreadMarker) -> Option<Retained<NSWindow>> {
        let view = MAIN_VIEW.load(Ordering::SeqCst) as *const NSView;
        // SAFETY: the view belongs to eframe's window, which lasts as long as
        // the app, and the marker proves this is the main thread.
        unsafe { view.as_ref() }?.window()
    }

    /// Shows and focuses the main window and puts the app back in the Dock.
    /// Safe to call from any thread: the work runs on the main thread, and the
    /// UI notices the change on its next frame and syncs its own state.
    pub fn show_main_window() {
        dispatch2::DispatchQueue::main().exec_async(|| {
            let Some(mtm) = MainThreadMarker::new() else { return };
            dock(mtm, true);
            if let Some(window) = main_window(mtm) {
                window.makeKeyAndOrderFront(None);
            }
            #[allow(deprecated)] // Its replacement, activate(), needs macOS 14.
            NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
        });
    }

    pub fn is_main_window_visible() -> bool {
        MainThreadMarker::new().and_then(main_window).is_some_and(|w| w.isVisible())
    }

    /// Hides the Dock icon while the app lives only in the menu bar, and
    /// brings it back with the window. Call on the main thread.
    pub fn set_in_dock(visible: bool) {
        if let Some(mtm) = MainThreadMarker::new() {
            dock(mtm, visible);
        }
    }

    fn dock(mtm: MainThreadMarker, visible: bool) {
        let policy = if visible {
            NSApplicationActivationPolicy::Regular
        } else {
            NSApplicationActivationPolicy::Accessory
        };
        let _ = NSApplication::sharedApplication(mtm).setActivationPolicy(policy);
    }

    /// Opens a URL in the default browser, or a folder in Finder.
    pub fn open(target: impl AsRef<OsStr>) {
        if let Ok(mut child) = Command::new("/usr/bin/open").arg(target).spawn() {
            std::thread::spawn(move || child.wait());
        }
    }

    /// The Mac's architecture, even when this build runs under Rosetta.
    pub fn native_arch() -> Arch {
        let translated = || {
            Command::new("/usr/sbin/sysctl")
                .args(["-in", "sysctl.proc_translated"])
                .output()
                .is_ok_and(|o| o.stdout.starts_with(b"1"))
        };
        if cfg!(target_arch = "aarch64") || translated() {
            Arch::Arm64
        } else {
            Arch::X64
        }
    }
}
