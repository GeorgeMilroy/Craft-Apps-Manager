//! Starting with Windows, via the per-user Run key.

use std::io;
use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "ArtCraftAppsManager";
/// The entry's name before the rename to ArtCraft Apps Manager.
const OLD_VALUE: &str = "CraftAppsManager";

/// Adds or removes the Run entry. Enabling always rewrites it, so the entry
/// follows the executable if it has been moved.
pub fn set(enabled: bool) -> io::Result<()> {
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN)?;
    let _ = key.delete_value(OLD_VALUE);
    if enabled {
        let exe = std::env::current_exe()?;
        key.set_value(VALUE, &format!("\"{}\" --minimized", exe.display()))
    } else {
        match key.delete_value(VALUE) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }
}
