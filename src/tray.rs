//! The tray icon: in the notification area on Windows, in the menu bar on macOS.

pub use imp::{is_running, quit, refresh, spawn};

/// Runs on its own thread with its own Win32 message loop, because egui
/// stops running frames while the main window is hidden, and the tray has
/// to keep working then.
#[cfg(windows)]
mod imp {
    use crate::manager::Manager;
    use crate::{logo, platform, APP_NAME};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, GetSystemMetrics, PeekMessageW, PostThreadMessageW, TranslateMessage, MSG,
        PM_NOREMOVE, SM_CXSMICON, WM_APP, WM_USER,
    };

    static THREAD_ID: AtomicU32 = AtomicU32::new(0);
    const WM_REFRESH: u32 = WM_APP + 1;
    const WM_QUIT_APP: u32 = WM_APP + 2;

    pub fn spawn(mgr: Arc<Manager>) {
        let _ = std::thread::Builder::new().name("tray".into()).spawn(move || {
            if let Err(e) = run(&mgr) {
                eprintln!("Tray icon unavailable: {e}");
            }
            THREAD_ID.store(0, Ordering::SeqCst);
        });
    }

    pub fn is_running() -> bool {
        THREAD_ID.load(Ordering::SeqCst) != 0
    }

    /// Updates the icon, tooltip and menu from the current state.
    pub fn refresh() {
        post(WM_REFRESH);
    }

    /// Removes the icon and exits the process. Returns false if there's no tray to do it.
    pub fn quit() -> bool {
        post(WM_QUIT_APP)
    }

    fn post(message: u32) -> bool {
        let id = THREAD_ID.load(Ordering::SeqCst);
        id != 0 && unsafe { PostThreadMessageW(id, message, 0, 0) } != 0
    }

    fn make_icon(badge: bool) -> Result<Icon, String> {
        let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.clamp(16, 64) as u32;
        Icon::from_rgba(logo::rgba(size, badge), size, size).map_err(|e| e.to_string())
    }

    fn run(mgr: &Arc<Manager>) -> Result<(), String> {
        // Create this thread's message queue before anyone can post to it.
        let mut msg: MSG = unsafe { std::mem::zeroed() };
        unsafe { PeekMessageW(&mut msg, std::ptr::null_mut(), WM_USER, WM_USER, PM_NOREMOVE) };

        let open = MenuItem::new(format!("Open {APP_NAME}"), true, None);
        let install = MenuItem::new("Install updates", false, None);
        let check = MenuItem::new("Check for updates", true, None);
        let quit = MenuItem::new("Quit", true, None);
        let menu = Menu::new();
        menu.append_items(&[&open, &install, &check, &PredefinedMenuItem::separator(), &quit])
            .map_err(|e| e.to_string())?;

        let tray = TrayIconBuilder::new()
            .with_tooltip(APP_NAME)
            .with_icon(make_icon(false)?)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()
            .map_err(|e| e.to_string())?;
        THREAD_ID.store(unsafe { GetCurrentThreadId() }, Ordering::SeqCst);

        let mut shown_updates = None;
        let sync = |shown: &mut Option<usize>| {
            let updates = mgr.lock().updates().len();
            if *shown == Some(updates) {
                return;
            }
            *shown = Some(updates);
            let _ = tray.set_icon(make_icon(updates > 0).ok());
            let (tooltip, item) = match updates {
                0 => (APP_NAME.to_owned(), "Install updates".to_owned()),
                1 => (format!("{APP_NAME}\n1 update available"), "Install 1 update".to_owned()),
                n => (format!("{APP_NAME}\n{n} updates available"), format!("Install {n} updates")),
            };
            let _ = tray.set_tooltip(Some(tooltip));
            install.set_text(item);
            install.set_enabled(updates > 0);
        };
        sync(&mut shown_updates);

        loop {
            if unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) } <= 0 {
                return Ok(());
            }
            match (msg.hwnd.is_null(), msg.message) {
                (true, WM_REFRESH) => sync(&mut shown_updates),
                (true, WM_QUIT_APP) => {
                    drop(tray);
                    std::process::exit(0);
                }
                _ => unsafe {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                },
            }

            while let Ok(event) = TrayIconEvent::receiver().try_recv() {
                match event {
                    TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }
                    | TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } => {
                        platform::show_main_window();
                        mgr.changed();
                    }
                    _ => {}
                }
            }
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                if event.id == *open.id() {
                    platform::show_main_window();
                    mgr.changed();
                } else if event.id == *install.id() {
                    mgr.update_all();
                } else if event.id == *check.id() {
                    mgr.check_now();
                } else if event.id == *quit.id() {
                    mgr.save();
                    drop(tray);
                    std::process::exit(0);
                }
            }
        }
    }
}

/// The menu bar icon. AppKit only allows it on the main thread, so it's made
/// there when the app starts, its menu's actions run there, and refreshes are
/// sent there from the other threads.
#[cfg(target_os = "macos")]
mod imp {
    use crate::manager::Manager;
    use crate::{logo, platform, APP_NAME};
    use dispatch2::DispatchQueue;
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

    struct Tray {
        icon: TrayIcon,
        install: MenuItem,
        mgr: Arc<Manager>,
        shown_updates: Option<usize>,
    }

    thread_local! {
        static TRAY: RefCell<Option<Tray>> = const { RefCell::new(None) };
    }
    static RUNNING: AtomicBool = AtomicBool::new(false);

    /// Creates the icon. Called on the main thread, as the app starts.
    pub fn spawn(mgr: Arc<Manager>) {
        if let Err(e) = create(mgr) {
            eprintln!("Menu bar icon unavailable: {e}");
        }
    }

    pub fn is_running() -> bool {
        RUNNING.load(Ordering::SeqCst)
    }

    /// Updates the icon, tooltip and menu from the current state.
    pub fn refresh() {
        if is_running() {
            DispatchQueue::main().exec_async(sync);
        }
    }

    /// Removes the icon and exits the process. Returns false if there's no icon to do it.
    pub fn quit() -> bool {
        if !is_running() {
            return false;
        }
        DispatchQueue::main().exec_async(quit_now);
        true
    }

    fn quit_now() {
        TRAY.with(|t| drop(t.borrow_mut().take()));
        std::process::exit(0);
    }

    fn make_icon(badge: bool) -> Result<Icon, String> {
        // The menu bar is 22 points tall; this suits it at 2x.
        let size = 44;
        Icon::from_rgba(logo::rgba(size, badge), size, size).map_err(|e| e.to_string())
    }

    fn create(mgr: Arc<Manager>) -> Result<(), String> {
        let open = MenuItem::new(format!("Open {APP_NAME}"), true, None);
        let install = MenuItem::new("Install updates", false, None);
        let check = MenuItem::new("Check for updates", true, None);
        let quit = MenuItem::new(format!("Quit {APP_NAME}"), true, None);
        let menu = Menu::new();
        menu.append_items(&[&open, &install, &check, &PredefinedMenuItem::separator(), &quit])
            .map_err(|e| e.to_string())?;
        let icon = TrayIconBuilder::new()
            .with_tooltip(APP_NAME)
            .with_icon(make_icon(false)?)
            .with_menu(Box::new(menu))
            .build()
            .map_err(|e| e.to_string())?;

        let ids = (open.id().clone(), install.id().clone(), check.id().clone(), quit.id().clone());
        let m = mgr.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let (open, install, check, quit) = &ids;
            if event.id == *open {
                platform::show_main_window();
                m.changed();
            } else if event.id == *install {
                m.update_all();
            } else if event.id == *check {
                m.check_now();
            } else if event.id == *quit {
                m.save();
                quit_now();
            }
        }));

        TRAY.with(|t| *t.borrow_mut() = Some(Tray { icon, install, mgr, shown_updates: None }));
        RUNNING.store(true, Ordering::SeqCst);
        sync();
        Ok(())
    }

    fn sync() {
        TRAY.with(|t| {
            let mut t = t.borrow_mut();
            let Some(tray) = t.as_mut() else { return };
            let updates = tray.mgr.lock().updates().len();
            if tray.shown_updates == Some(updates) {
                return;
            }
            tray.shown_updates = Some(updates);
            let _ = tray.icon.set_icon(make_icon(updates > 0).ok());
            let (tooltip, item) = match updates {
                0 => (APP_NAME.to_owned(), "Install updates".to_owned()),
                1 => (format!("{APP_NAME}: 1 update available"), "Install 1 update".to_owned()),
                n => (format!("{APP_NAME}: {n} updates available"), format!("Install {n} updates")),
            };
            let _ = tray.icon.set_tooltip(Some(tooltip));
            tray.install.set_text(item);
            tray.install.set_enabled(updates > 0);
        });
    }
}
