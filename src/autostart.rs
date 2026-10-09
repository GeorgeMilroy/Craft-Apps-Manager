//! Starting at sign-in: the per-user Run key on Windows, a LaunchAgent on
//! macOS. Both start the app with --minimized.

pub use imp::set;

#[cfg(windows)]
mod imp {
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
}

#[cfg(target_os = "macos")]
mod imp {
    use crate::BUNDLE_ID;
    use std::path::PathBuf;
    use std::{fs, io};

    fn agent_file() -> io::Result<PathBuf> {
        let home = std::env::var_os("HOME").ok_or_else(|| io::Error::other("HOME isn't set"))?;
        Ok(PathBuf::from(home).join("Library").join("LaunchAgents").join(format!("{BUNDLE_ID}.plist")))
    }

    /// Adds or removes ~/Library/LaunchAgents/<bundle id>.plist. Enabling
    /// always rewrites it, so the agent follows the app if it has been moved.
    pub fn set(enabled: bool) -> io::Result<()> {
        let file = agent_file()?;
        if !enabled {
            return match fs::remove_file(&file) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                other => other,
            };
        }
        let exe = std::env::current_exe()?;
        let exe = exe.display().to_string().replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{BUNDLE_ID}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    <string>--minimized</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>ProcessType</key>
  <string>Interactive</string>
</dict>
</plist>
"#
        );
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir)?;
        }
        if fs::read_to_string(&file).is_ok_and(|old| old == plist) {
            return Ok(());
        }
        fs::write(file, plist)
    }
}
