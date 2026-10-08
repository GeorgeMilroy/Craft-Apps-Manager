//! GitHub releases: fetching them, and picking the right Windows download.

use crate::catalog::{AppInfo, OWNER};
use crate::util::now_unix;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const API: &str = "https://api.github.com";
pub const USER_AGENT: &str = concat!("ArtCraftAppsManager/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    pub html_url: String,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

impl Release {
    pub fn version(&self) -> String {
        extract_version(&self.tag_name)
            .or_else(|| self.name.as_deref().and_then(extract_version))
            .unwrap_or_else(|| self.tag_name.clone())
    }

    pub fn date(&self) -> &str {
        self.published_at.as_deref().and_then(|d| d.get(..10)).unwrap_or("")
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub browser_download_url: String,
    /// `sha256:<hex>`, provided by GitHub for assets uploaded since mid-2025.
    #[serde(default)]
    pub digest: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Repo {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub fork: bool,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arch {
    X64,
    Arm64,
    X86,
}

impl Arch {
    pub fn label(self) -> &'static str {
        match self {
            Arch::X64 => "x64",
            Arch::Arm64 => "ARM64",
            Arch::X86 => "x86",
        }
    }

    fn tokens(self) -> &'static [&'static str] {
        match self {
            Arch::X64 => &["x64", "amd64"],
            Arch::Arm64 => &["arm64", "aarch64"],
            Arch::X86 => &["x86", "i686", "win32"],
        }
    }

    /// Builds that run on a machine of this architecture, best first.
    fn runnable(self) -> &'static [Arch] {
        match self {
            Arch::X64 => &[Arch::X64, Arch::X86],
            Arch::Arm64 => &[Arch::Arm64, Arch::X64, Arch::X86],
            Arch::X86 => &[Arch::X86],
        }
    }
}

pub enum Fetch<T> {
    Fresh { value: T, etag: Option<String> },
    NotModified,
}

/// Unauthenticated GitHub API client. GitHub allows 60 requests an hour per
/// IP address, and without sign-in even "not modified" answers count.
pub struct Client {
    agent: ureq::Agent,
}

pub fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .user_agent(USER_AGENT)
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(60))
        .try_proxy_from_env(true)
        .build()
}

impl Client {
    pub fn new(agent: ureq::Agent) -> Self {
        Client { agent }
    }

    /// Recent releases of `repo`, newest first. Passing the ETag from the
    /// last call lets GitHub answer "not modified" instead of resending an
    /// unchanged list.
    pub fn releases(&self, repo: &str, etag: Option<&str>) -> Result<Fetch<Vec<Release>>, String> {
        self.get_json(&format!("{API}/repos/{OWNER}/{repo}/releases?per_page=8"), etag)
    }

    pub fn org_repos(&self, etag: Option<&str>) -> Result<Fetch<Vec<Repo>>, String> {
        self.get_json(&format!("{API}/orgs/{OWNER}/repos?per_page=100"), etag)
    }

    fn get_json<T: DeserializeOwned>(&self, url: &str, etag: Option<&str>) -> Result<Fetch<T>, String> {
        let mut req = self
            .agent
            .get(url)
            .set("Accept", "application/vnd.github+json")
            .set("X-GitHub-Api-Version", "2022-11-28");
        if let Some(etag) = etag {
            req = req.set("If-None-Match", etag);
        }
        match req.call() {
            Ok(resp) if resp.status() == 304 => Ok(Fetch::NotModified),
            Ok(resp) => {
                let etag = resp.header("ETag").map(str::to_owned);
                let value = resp
                    .into_json()
                    .map_err(|e| format!("Unexpected response from GitHub: {e}"))?;
                Ok(Fetch::Fresh { value, etag })
            }
            Err(ureq::Error::Status(code, resp)) => Err(status_error(code, &resp)),
            Err(ureq::Error::Transport(t)) => Err(format!("Couldn't reach GitHub: {t}")),
        }
    }
}

fn status_error(code: u16, resp: &ureq::Response) -> String {
    if matches!(code, 403 | 429) && resp.header("x-ratelimit-remaining") == Some("0") {
        let resets = resp
            .header("x-ratelimit-reset")
            .and_then(|r| r.parse::<u64>().ok())
            .map(|r| format!(" for the next {} min", r.saturating_sub(now_unix()) / 60 + 1))
            .unwrap_or_default();
        return format!(
            "GitHub's hourly request limit for this network is used up{resets}. The app will check again automatically."
        );
    }
    match code {
        404 => "Not found on GitHub (HTTP 404).".to_owned(),
        _ => format!("GitHub returned HTTP {code}."),
    }
}

/// The newest stable release of `app` with a Windows installer (MSI).
pub fn latest_release(app: &AppInfo, releases: &[Release]) -> Option<Release> {
    releases
        .iter()
        .find(|r| !r.draft && !r.prerelease && r.assets.iter().any(|a| is_windows_asset(app, &a.name)))
        .cloned()
}

/// The MSI best suited to a machine of `arch`.
pub fn pick_asset<'a>(app: &AppInfo, release: &'a Release, arch: Arch) -> Option<&'a Asset> {
    arch.runnable().iter().find_map(|want| {
        release
            .assets
            .iter()
            .find(|a| is_windows_asset(app, &a.name) && asset_arch(&a.name).unwrap_or(Arch::X64) == *want)
    })
}

pub fn checksums_asset(release: &Release) -> Option<&Asset> {
    release.assets.iter().find(|a| a.name.eq_ignore_ascii_case("SHA256SUMS.txt"))
}

fn tokens(name: &str) -> Vec<String> {
    name.to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

fn is_windows_asset(app: &AppInfo, name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    // Repos can ship more than one product (CLI tools, web builds, renamed apps).
    let ours = std::iter::once(app.id.as_str())
        .chain(app.names())
        .any(|n| lower.starts_with(&n.to_ascii_lowercase()));
    let t = tokens(name);
    ours && lower.ends_with(".msi") && !t.iter().any(|t| t == "cli" || t == "web")
}

fn asset_arch(name: &str) -> Option<Arch> {
    let t = tokens(name);
    [Arch::Arm64, Arch::X64, Arch::X86]
        .into_iter()
        .find(|arch| arch.tokens().iter().any(|tok| t.iter().any(|x| x == tok)))
}

/// Pulls "0.41.0" out of tags like "artcraft-v0.41.0", "v0.3.0" or "0.3.0".
pub fn extract_version(s: &str) -> Option<String> {
    let b = s.as_bytes();
    (0..b.len())
        .find(|&i| b[i].is_ascii_digit() && (i == 0 || matches!(b[i - 1], b'v' | b'V' | b'-' | b'_' | b' ')))
        .map(|i| {
            s[i..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
                .collect()
        })
}

fn parse_version(s: &str) -> Option<semver::Version> {
    let v = extract_version(s)?;
    if let Ok(v) = semver::Version::parse(&v) {
        return Some(v);
    }
    // Lenient fallback for "1.2" or four-part MSI versions like "1.2.3.0".
    let mut parts = v.split('.').map(|p| p.parse::<u64>().ok());
    let major = parts.next().flatten()?;
    let minor = parts.next().flatten().unwrap_or(0);
    let patch = parts.next().flatten().unwrap_or(0);
    Some(semver::Version::new(major, minor, patch))
}

/// Whether `latest` is a newer version than `installed`.
pub fn is_newer(latest: &str, installed: &str) -> bool {
    match (parse_version(latest), parse_version(installed)) {
        (Some(a), Some(b)) => a > b,
        _ => latest.trim() != installed.trim(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::builtin;

    fn asset(name: &str) -> Asset {
        Asset { name: name.into(), size: 1, browser_download_url: String::new(), digest: None }
    }

    fn release(tag: &str, names: &[&str]) -> Release {
        Release {
            tag_name: tag.into(),
            name: None,
            body: None,
            html_url: String::new(),
            published_at: None,
            draft: false,
            prerelease: false,
            assets: names.iter().map(|n| asset(n)).collect(),
        }
    }

    fn app(id: &str) -> AppInfo {
        builtin().into_iter().find(|a| a.id == id).unwrap()
    }

    #[test]
    fn versions() {
        assert_eq!(extract_version("artcraft-v0.41.0").as_deref(), Some("0.41.0"));
        assert_eq!(extract_version("v0.3.0").as_deref(), Some("0.3.0"));
        assert_eq!(extract_version("0.3.0").as_deref(), Some("0.3.0"));
        assert!(is_newer("0.5.0", "0.4.0"));
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(!is_newer("0.3.0", "0.3.0.0"));
        assert!(!is_newer("0.3.0", "0.4.0"));
    }

    #[test]
    fn picks_windows_assets() {
        let grid = app("gridcraft");
        let r = release(
            "v0.3.0",
            &[
                "gridcraft-0.3.0-linux-x86_64.tar.gz",
                "gridcraft-cli-0.3.0-macos-universal.zip",
                "gridcraft-0.3.0-windows-arm64.msi",
                "gridcraft-0.3.0-windows-x64.msi",
                "gridcraft-0.3.0-windows-x86.msi",
                "gridcraft-0.3.0-windows-x64-portable.zip",
                "gridcraft-web-0.3.0.zip",
            ],
        );
        let pick = |arch| pick_asset(&grid, &r, arch).map(|a| a.name.as_str());
        assert_eq!(pick(Arch::X64), Some("gridcraft-0.3.0-windows-x64.msi"));
        assert_eq!(pick(Arch::Arm64), Some("gridcraft-0.3.0-windows-arm64.msi"));
        assert_eq!(pick(Arch::X86), Some("gridcraft-0.3.0-windows-x86.msi"));

        // ArtCraft isn't in the catalog, but its naming style is worth covering.
        let art = AppInfo::discovered("artcraft", "ArtCraft", "", None);
        let r = release(
            "artcraft-v0.41.0",
            &["ArtCraft_0.41.0_universal.dmg", "ArtCraft_0.41.0_x64-setup.exe", "ArtCraft_0.41.0_x64_en-US.msi"],
        );
        assert_eq!(pick_asset(&art, &r, Arch::X64).map(|a| a.name.as_str()), Some("ArtCraft_0.41.0_x64_en-US.msi"));
        // ArtCraft has no ARM64 build, so ARM64 machines get the emulated x64 one.
        assert_eq!(pick_asset(&art, &r, Arch::Arm64).map(|a| a.name.as_str()), Some("ArtCraft_0.41.0_x64_en-US.msi"));
        // The setup.exe is ignored: only MSIs are installed.
        let setup_only = release("artcraft-v0.42.0", &["ArtCraft_0.42.0_x64-setup.exe"]);
        assert!(latest_release(&art, &[setup_only]).is_none());
        assert_eq!(r.version(), "0.41.0");
    }

    #[test]
    fn skips_releases_of_other_products() {
        let pdf = app("pdfcraft");
        let old = release("v0.2.1", &["printcraft-0.2.1-windows-x64.msi"]);
        let other = release("v0.1.0", &["somethingelse-0.1.0-windows-x64.msi"]);
        assert!(latest_release(&pdf, &[other.clone(), old]).is_some());
        assert!(latest_release(&pdf, &[other]).is_none());
    }
}
