//! Finding which apps are installed, at which version, and where: from
//! Windows' list of installed apps, or from the app bundles in the Mac's
//! Applications folders.

use crate::catalog::AppInfo;
use crate::github::is_newer;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Installed {
    pub version: String,
    pub source: Source,
    pub exe: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub enum Source {
    #[cfg(windows)]
    Msi { product_code: String },
    /// Installed by a non-MSI installer, such as an NSIS setup.exe.
    #[cfg(windows)]
    Program { uninstall: Option<String> },
    /// An app bundle, such as /Applications/PhotoCraft.app.
    #[cfg(target_os = "macos")]
    Bundle { path: PathBuf },
}

/// One installed program, as the platform lists it.
pub struct Entry {
    display_name: String,
    version: String,
    /// Installed the way this manager installs apps (from an MSI, or as an app
    /// bundle), which wins over other entries with the same name.
    preferred: bool,
    source: Source,
    location: Option<PathBuf>,
}

/// Reads the machine-wide (64- and 32-bit) and per-user uninstall entries.
#[cfg(windows)]
pub fn scan(_apps: &[AppInfo]) -> Vec<Entry> {
    use winreg::enums::*;
    use winreg::RegKey;
    const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";

    let views = [
        (HKEY_LOCAL_MACHINE, KEY_WOW64_64KEY),
        (HKEY_LOCAL_MACHINE, KEY_WOW64_32KEY),
        (HKEY_CURRENT_USER, 0),
    ];
    let mut out = Vec::new();
    for (hive, view) in views {
        let Ok(root) = RegKey::predef(hive).open_subkey_with_flags(UNINSTALL, KEY_READ | view) else {
            continue;
        };
        for key in root.enum_keys().flatten() {
            let Ok(k) = root.open_subkey_with_flags(&key, KEY_READ | view) else {
                continue;
            };
            let get = |name: &str| k.get_value::<String, _>(name).ok().filter(|v| !v.trim().is_empty());
            let (Some(display_name), Some(version)) = (get("DisplayName"), get("DisplayVersion")) else {
                continue;
            };
            let msi = k.get_value::<u32, _>("WindowsInstaller").unwrap_or(0) == 1;
            let source = if msi && key.starts_with('{') {
                Source::Msi { product_code: key }
            } else {
                Source::Program { uninstall: get("QuietUninstallString").or_else(|| get("UninstallString")) }
            };
            out.push(Entry {
                preferred: msi,
                location: get("InstallLocation").map(PathBuf::from),
                source,
                display_name,
                version,
            });
        }
    }
    out
}

/// Finds the catalog's apps among the bundles in /Applications and
/// ~/Applications. Only bundles named like an app have their version read.
#[cfg(target_os = "macos")]
pub fn scan(apps: &[AppInfo]) -> Vec<Entry> {
    let mut dirs = vec![PathBuf::from("/Applications")];
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join("Applications"));
    }
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(items) = std::fs::read_dir(&dir) else { continue };
        for path in items.flatten().map(|i| i.path()) {
            if path.extension().is_none_or(|x| x != "app") {
                continue;
            }
            let Some(name) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else { continue };
            if !apps.iter().any(|app| name_matches(app, &name)) {
                continue;
            }
            let Some(version) = bundle_version(&path) else { continue };
            out.push(Entry {
                display_name: name,
                version,
                preferred: true,
                source: Source::Bundle { path: path.clone() },
                location: Some(path),
            });
        }
    }
    out
}

/// The version in a bundle's Info.plist, which may be XML or binary.
#[cfg(target_os = "macos")]
fn bundle_version(bundle: &std::path::Path) -> Option<String> {
    let info = bundle.join("Contents").join("Info");
    ["CFBundleShortVersionString", "CFBundleVersion"].iter().find_map(|key| {
        let out = std::process::Command::new("/usr/bin/defaults").arg("read").arg(&info).arg(key).output().ok()?;
        let version = String::from_utf8(out.stdout).ok()?.trim().to_owned();
        (out.status.success() && !version.is_empty()).then_some(version)
    })
}

pub fn detect_all(apps: &[AppInfo], found: &[Entry]) -> HashMap<String, Installed> {
    apps.iter()
        .filter_map(|app| detect(app, found).map(|i| (app.id.clone(), i)))
        .collect()
}

fn detect(app: &AppInfo, found: &[Entry]) -> Option<Installed> {
    // Prefer what this manager installs (MSIs, app bundles), then the
    // newest version, in case an old name was left installed alongside.
    let mut entries: Vec<&Entry> = found.iter().filter(|e| name_matches(app, &e.display_name)).collect();
    entries.sort_by(|a, b| {
        b.preferred.cmp(&a.preferred).then_with(|| {
            if is_newer(&a.version, &b.version) {
                Ordering::Less
            } else if is_newer(&b.version, &a.version) {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        })
    });
    let e = entries.first()?;
    // On a Mac the bundle itself is what gets opened.
    #[cfg(target_os = "macos")]
    let exe = e.location.clone();
    #[cfg(windows)]
    let exe = find_installed_exe(app, e.location.clone());
    Some(Installed { version: e.version.clone(), source: e.source.clone(), exe })
}

fn normalize(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// "GridCraft" and "GridCraft 0.3.0" match GridCraft; "GridCraft Pro" doesn't.
fn name_matches(app: &AppInfo, display_name: &str) -> bool {
    let d = normalize(display_name);
    app.names().any(|n| {
        let n = normalize(n);
        d.strip_prefix(&n).is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit()))
    })
}

#[cfg(windows)]
fn find_installed_exe(app: &AppInfo, location: Option<PathBuf>) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = location.into_iter().collect();
    for var in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        if let Some(base) = std::env::var_os(var).map(PathBuf::from) {
            for name in app.names() {
                dirs.push(base.join(name));
                if var == "LOCALAPPDATA" {
                    dirs.push(base.join("Programs").join(name));
                }
            }
        }
    }
    dirs.iter().filter(|d| d.is_dir()).find_map(|d| find_exe_in(d, app))
}

/// The app's main executable in `dir`: `<id>.exe` or `<name>.exe` if present,
/// otherwise the first .exe that isn't a CLI tool or uninstaller.
#[cfg(windows)]
fn find_exe_in(dir: &std::path::Path, app: &AppInfo) -> Option<PathBuf> {
    let stems: Vec<String> = std::iter::once(app.id.clone())
        .chain(app.names().map(str::to_lowercase))
        .collect();
    let exes: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("exe")))
        .collect();
    let stem = |p: &PathBuf| p.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    exes.iter()
        .find(|p| stems.contains(&stem(p)))
        .or_else(|| {
            exes.iter().find(|p| {
                let s = stem(p);
                !s.ends_with("-cli") && !s.contains("uninstall") && !s.contains("updater")
            })
        })
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::builtin;

    #[test]
    fn matches_display_names() {
        let apps = builtin();
        let grid = apps.iter().find(|a| a.id == "gridcraft").unwrap();
        let pdf = apps.iter().find(|a| a.id == "pdfcraft").unwrap();
        assert!(name_matches(grid, "GridCraft"));
        assert!(name_matches(grid, "GridCraft 0.3.0"));
        assert!(!name_matches(grid, "GridCraft Pro"));
        assert!(name_matches(pdf, "PrintCraft"));
        assert!(!name_matches(pdf, "Adobe Acrobat"));
    }
}
