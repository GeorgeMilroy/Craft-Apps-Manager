//! Finding which apps are installed, at which version, and where.

use crate::catalog::AppInfo;
use crate::github::is_newer;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use winreg::enums::*;
use winreg::RegKey;

const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";

#[derive(Clone, Debug)]
pub struct Installed {
    pub version: String,
    pub source: Source,
    pub exe: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub enum Source {
    Msi { product_code: String },
    /// Installed by a non-MSI installer, such as an NSIS setup.exe.
    Program { uninstall: Option<String> },
}

/// One entry from the Windows "Installed apps" list.
pub struct RegEntry {
    key: String,
    display_name: String,
    version: String,
    location: Option<String>,
    uninstall: Option<String>,
    msi: bool,
}

/// Reads the machine-wide (64- and 32-bit) and per-user uninstall entries.
pub fn scan_registry() -> Vec<RegEntry> {
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
            out.push(RegEntry {
                msi: k.get_value::<u32, _>("WindowsInstaller").unwrap_or(0) == 1,
                location: get("InstallLocation"),
                uninstall: get("QuietUninstallString").or_else(|| get("UninstallString")),
                key,
                display_name,
                version,
            });
        }
    }
    out
}

pub fn detect_all(apps: &[AppInfo], registry: &[RegEntry]) -> HashMap<String, Installed> {
    apps.iter()
        .filter_map(|app| detect(app, registry).map(|i| (app.id.clone(), i)))
        .collect()
}

fn detect(app: &AppInfo, registry: &[RegEntry]) -> Option<Installed> {
    // Prefer MSI entries (what this manager installs), then the
    // newest version, in case an old name was left installed alongside.
    let mut entries: Vec<&RegEntry> = registry.iter().filter(|e| name_matches(app, &e.display_name)).collect();
    entries.sort_by(|a, b| {
        b.msi.cmp(&a.msi).then_with(|| {
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
    let source = if e.msi && e.key.starts_with('{') {
        Source::Msi { product_code: e.key.clone() }
    } else {
        Source::Program { uninstall: e.uninstall.clone() }
    };
    Some(Installed { version: e.version.clone(), source, exe: find_installed_exe(app, e.location.as_deref()) })
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

fn find_installed_exe(app: &AppInfo, location: Option<&str>) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = location.map(PathBuf::from).into_iter().collect();
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
fn find_exe_in(dir: &Path, app: &AppInfo) -> Option<PathBuf> {
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
