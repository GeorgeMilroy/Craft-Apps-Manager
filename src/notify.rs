//! Windows toast notifications, and macOS Notification Center ones.

use crate::settings::Paths;

/// Registers this app's AppUserModelID so its toasts show under its own name
/// and icon. Unpackaged apps have to do this themselves.
#[cfg(windows)]
pub fn register(paths: &Paths) {
    use crate::{logo, APP_ID, APP_NAME};
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let icon_path = paths.data.join("icon.ico");
    let _ = std::fs::write(&icon_path, logo::ICO);
    let key = format!(r"Software\Classes\AppUserModelId\{APP_ID}");
    if let Ok((key, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(key) {
        let _ = key.set_value("DisplayName", &APP_NAME);
        let _ = key.set_value("IconUri", &icon_path.display().to_string());
    }
}

/// Sends notifications as this app's bundle, so they show its name and icon.
#[cfg(target_os = "macos")]
pub fn register(_paths: &Paths) {
    let _ = notify_rust::set_application(crate::BUNDLE_ID);
}

/// Shows a notification without blocking the caller.
pub fn show(title: impl Into<String>, body: impl Into<String>) {
    let (title, body) = (title.into(), body.into());
    std::thread::spawn(move || {
        let mut notification = notify_rust::Notification::new();
        notification.summary(&title).body(&body);
        #[cfg(windows)]
        notification.app_id(crate::APP_ID);
        let _ = notification.show();
    });
}
