//! Windows toast notifications.

use crate::settings::Paths;
use crate::{logo, APP_ID, APP_NAME};
use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

/// Registers this app's AppUserModelID so its toasts show under its own name
/// and icon. Unpackaged apps have to do this themselves.
pub fn register(paths: &Paths) {
    let icon_path = paths.data.join("icon.ico");
    let _ = std::fs::write(&icon_path, logo::ICO);
    let key = format!(r"Software\Classes\AppUserModelId\{APP_ID}");
    if let Ok((key, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(key) {
        let _ = key.set_value("DisplayName", &APP_NAME);
        let _ = key.set_value("IconUri", &icon_path.display().to_string());
    }
}

/// Shows a toast without blocking the caller.
pub fn show(title: impl Into<String>, body: impl Into<String>) {
    let (title, body) = (title.into(), body.into());
    std::thread::spawn(move || {
        let _ = notify_rust::Notification::new().summary(&title).body(&body).app_id(APP_ID).show();
    });
}
