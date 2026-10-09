//! Settings and the app's own bookkeeping, stored as JSON. On Windows they're
//! in the machine-wide C:\ProgramData\ArtCraftAppsManager folder (Program
//! Files is read-only without admin rights, so it can't hold them). On macOS
//! they're in ~/Library/Application Support/ArtCraftAppsManager.

use crate::catalog::AppInfo;
use crate::github::Release;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::{fs, io};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeChoice {
    /// Follow the system's light or dark mode.
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeChoice,
    pub start_at_login: bool,
    pub minimize_to_tray: bool,
    pub check_interval_hours: u32,
    pub notify_updates: bool,
    pub notify_installs: bool,
    pub auto_install_updates: bool,
    pub keep_installers: bool,
    /// Apps whose update notifications are turned off.
    pub muted: BTreeSet<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: ThemeChoice::System,
            start_at_login: false,
            minimize_to_tray: true,
            check_interval_hours: 4,
            notify_updates: true,
            notify_installs: true,
            auto_install_updates: false,
            keep_installers: false,
            muted: BTreeSet::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Store {
    /// Latest release per app, with the ETag of the list it came from.
    pub releases: BTreeMap<String, CachedRelease>,
    /// The last version each app was announced at, so each update notifies once.
    pub notified: BTreeMap<String, String>,
    pub discovered: Vec<AppInfo>,
    pub repos_etag: Option<String>,
    pub last_discovery: Option<u64>,
    pub last_check: Option<u64>,
    pub tray_hint_shown: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CachedRelease {
    pub etag: Option<String>,
    pub release: Option<Release>,
}

pub struct Paths {
    pub data: PathBuf,
    pub downloads: PathBuf,
    pub logs: PathBuf,
}

impl Paths {
    #[cfg(target_os = "macos")]
    pub fn new() -> Self {
        let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("/tmp"), PathBuf::from);
        let data = home.join("Library").join("Application Support").join("ArtCraftAppsManager");
        let paths = Paths { downloads: data.join("downloads"), logs: data.join("logs"), data };
        for dir in [&paths.data, &paths.downloads, &paths.logs] {
            let _ = fs::create_dir_all(dir);
        }
        paths
    }

    #[cfg(windows)]
    pub fn new() -> Self {
        let program_data = std::env::var_os("ProgramData").map_or_else(|| PathBuf::from(r"C:\ProgramData"), PathBuf::from);
        let data = program_data.join("ArtCraftAppsManager");
        let paths = Paths { downloads: data.join("downloads"), logs: data.join("logs"), data };
        let _ = fs::create_dir_all(&paths.data);
        paths.move_from(&program_data.join("CraftAppsManager"));
        for dir in [&paths.downloads, &paths.logs] {
            let _ = fs::create_dir_all(dir);
        }
        paths.migrate_from_app_data();
        paths
    }

    pub fn settings_file(&self) -> PathBuf {
        self.data.join("settings.json")
    }

    pub fn store_file(&self) -> PathBuf {
        self.data.join("state.json")
    }

    /// Versions 0.2 and 0.3, from before the rename to ArtCraft Apps Manager,
    /// used C:\ProgramData\CraftAppsManager. Moves its contents across, then
    /// removes it. Anything that can't be moved yet, such as a file that's in
    /// use, is left for the next start.
    #[cfg(windows)]
    fn move_from(&self, old: &Path) {
        let Ok(entries) = fs::read_dir(old) else {
            return;
        };
        let mut moved_all = true;
        for entry in entries.flatten() {
            let dest = self.data.join(entry.file_name());
            if !dest.exists() && fs::rename(entry.path(), &dest).is_err() {
                moved_all = false;
            }
        }
        if moved_all {
            let _ = fs::remove_dir_all(old);
        }
    }

    /// Versions before 0.2 kept their data in %LOCALAPPDATA%. Carries the
    /// settings and state over once, then removes the old folder.
    #[cfg(windows)]
    fn migrate_from_app_data(&self) {
        let Some(old) = std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("CraftAppsManager")) else {
            return;
        };
        if !old.is_dir() {
            return;
        }
        for (file, dest) in [("settings.json", self.settings_file()), ("state.json", self.store_file())] {
            if !dest.exists() && fs::copy(old.join(file), &dest).is_err() && old.join(file).exists() {
                return; // Keep the old folder if anything couldn't be copied.
            }
        }
        let _ = fs::remove_dir_all(old);
    }
}

/// Reads `path`, falling back to defaults if it's missing. A file that
/// can't be parsed is kept as `.bak` rather than silently overwritten.
pub fn load<T: DeserializeOwned + Default>(path: &Path) -> T {
    let Ok(text) = fs::read_to_string(path) else {
        return T::default();
    };
    serde_json::from_str(&text).unwrap_or_else(|_| {
        let _ = fs::copy(path, path.with_extension("json.bak"));
        T::default()
    })
}

pub fn save<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    fs::rename(tmp, path)
}
