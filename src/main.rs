#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod catalog;
mod check;
mod github;
mod icons;
mod installed;
mod jobs;
mod logo;
mod manager;
mod notify;
mod platform;
mod settings;
mod theme;
mod tray;
mod ui;
mod util;

use eframe::egui;

pub const APP_NAME: &str = "ArtCraft Apps Manager";
/// AppUserModelID: groups the taskbar button and identifies our notifications.
/// It predates the rename to ArtCraft Apps Manager and is kept, along with the
/// single-instance lock named after it, so old and new versions recognise each other.
pub const APP_ID: &str = "ArtCraft.CraftAppsManager";
/// The Mac app's bundle identifier, also used for its notifications and its
/// LaunchAgent. Never change it: macOS keys permissions and settings to it.
#[cfg(target_os = "macos")]
pub const BUNDLE_ID: &str = "com.mecorelabs.artcraftappsmanager";

fn main() -> eframe::Result<()> {
    // Passed by the sign-in startup entry, so the window doesn't pop up at login.
    let start_hidden = std::env::args().skip(1).any(|a| a == "--minimized");

    if !platform::claim_single_instance(APP_ID) {
        if !start_hidden {
            platform::activate_existing_window(APP_NAME);
        }
        return Ok(());
    }
    platform::set_app_user_model_id(APP_ID);

    let mgr = manager::Manager::load();
    notify::register(&mgr.paths);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_inner_size([460.0, 660.0])
            .with_min_inner_size([400.0, 440.0])
            .with_icon(ui::window_icon()),
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| {
            platform::set_main_window(cc);
            mgr.start(cc.egui_ctx.clone());
            Ok(Box::new(ui::ManagerApp::new(cc, mgr, start_hidden)))
        }),
    )
}
