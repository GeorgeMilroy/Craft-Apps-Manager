//! The download and install queue. One worker downloads jobs in order, one
//! at a time; a second installs finished downloads, so the next download
//! starts while the previous app is installing.

use crate::catalog::AppInfo;
use crate::github::Asset;
use crate::manager::Manager;
use crate::util::{self, human_bytes};
use imp::{install, uninstall};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Install,
    Update,
    Reinstall,
    Uninstall,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::Install => "Install",
            Action::Update => "Update",
            Action::Reinstall => "Reinstall",
            Action::Uninstall => "Uninstall",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Queued,
    Downloading,
    ReadyToInstall,
    Installing,
    Done(String),
    Failed(String),
    Cancelled,
}

impl Status {
    pub fn is_finished(&self) -> bool {
        matches!(self, Status::Done(_) | Status::Failed(_) | Status::Cancelled)
    }

    pub fn is_active(&self) -> bool {
        !self.is_finished()
    }
}

#[derive(Clone, Debug)]
pub struct Job {
    pub id: u64,
    pub app: AppInfo,
    pub action: Action,
    pub version: String,
    /// The installer to download. None for uninstalls, which skip the download.
    pub asset: Option<Asset>,
    pub checksums_url: Option<String>,
    pub status: Status,
    pub downloaded: u64,
    pub total: u64,
    /// Bytes per second.
    pub speed: f64,
    pub verified: bool,
    pub file: Option<PathBuf>,
    pub cancel: Arc<AtomicBool>,
    pub finished_at: Option<u64>,
}

impl Job {
    pub fn new(
        id: u64,
        app: AppInfo,
        action: Action,
        version: String,
        asset: Option<Asset>,
        checksums_url: Option<String>,
    ) -> Self {
        Job {
            id,
            app,
            action,
            version,
            total: asset.as_ref().map_or(0, |a| a.size),
            status: if asset.is_some() { Status::Queued } else { Status::ReadyToInstall },
            asset,
            checksums_url,
            downloaded: 0,
            speed: 0.0,
            verified: false,
            file: None,
            cancel: Arc::new(AtomicBool::new(false)),
            finished_at: None,
        }
    }
}

pub fn spawn(mgr: Arc<Manager>) {
    let m = mgr.clone();
    thread::Builder::new().name("downloads".into()).spawn(move || download_loop(m)).expect("spawn downloader");
    thread::Builder::new().name("installs".into()).spawn(move || install_loop(mgr)).expect("spawn installer");
}

fn download_loop(mgr: Arc<Manager>) {
    loop {
        let job = mgr.next_job(&Status::Queued, Status::Downloading);
        let result = download(&mgr, &job);
        let cancelled = job.cancel.load(Ordering::SeqCst);
        let failure = match &result {
            Err(e) if !cancelled => Some(Err(e.clone())),
            _ => None,
        };
        mgr.update_job(job.id, |j| match result {
            Ok((file, verified)) => {
                j.file = Some(file);
                j.verified = verified;
                j.status = Status::ReadyToInstall;
            }
            Err(_) if cancelled => j.status = Status::Cancelled,
            Err(e) => j.status = Status::Failed(e),
        });
        if let Some(failure) = failure {
            mgr.notify_job_result(&job, &failure);
        }
    }
}

fn install_loop(mgr: Arc<Manager>) {
    loop {
        let job = mgr.next_job(&Status::ReadyToInstall, Status::Installing);
        let result = match job.action {
            Action::Uninstall => uninstall(&mgr, &job),
            _ => install(&mgr, &job),
        };
        mgr.refresh_installed();
        if let Some(file) = &job.file {
            if !mgr.lock().settings.keep_installers {
                let _ = fs::remove_file(file);
            }
        }
        let status = match &result {
            Ok(message) => Status::Done(message.clone()),
            Err(error) => Status::Failed(error.clone()),
        };
        mgr.update_job(job.id, |j| j.status = status);
        mgr.notify_job_result(&job, &result);
    }
}

/// Downloads the job's asset, checking its size and SHA-256. Returns the
/// file and whether a checksum was available to verify against.
fn download(mgr: &Manager, job: &Job) -> Result<(PathBuf, bool), String> {
    let asset = job.asset.as_ref().ok_or("Nothing to download.")?;
    let name: String = asset.name.chars().filter(|c| !matches!(c, '/' | '\\' | ':')).collect();
    let dest = mgr.paths.downloads.join(&name);
    let part = mgr.paths.downloads.join(format!("{name}.part"));
    let expected = expected_sha256(mgr, asset, job.checksums_url.as_deref());

    let resp = mgr
        .http
        .get(&asset.browser_download_url)
        .call()
        .map_err(|e| format!("Download failed: {e}"))?;
    let total = resp.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(asset.size);
    let mut reader = resp.into_reader();
    let file = File::create(&part).map_err(|e| format!("Couldn't write to {}: {e}", part.display()))?;
    let mut file = io::BufWriter::new(file);

    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    let mut speed = 0f64;
    let (mut last_at, mut last_done) = (Instant::now(), 0u64);
    let result = loop {
        if job.cancel.load(Ordering::SeqCst) {
            break Err("Cancelled.".to_owned());
        }
        let n = match reader.read(&mut buf) {
            Ok(0) => break Ok(()),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => break Err(format!("Download interrupted: {e}")),
        };
        if let Err(e) = file.write_all(&buf[..n]) {
            break Err(format!("Couldn't save the download: {e}"));
        }
        hasher.update(&buf[..n]);
        done += n as u64;

        let elapsed = last_at.elapsed().as_secs_f64();
        if elapsed >= 0.25 {
            let current = (done - last_done) as f64 / elapsed;
            speed = if speed == 0.0 { current } else { speed * 0.7 + current * 0.3 };
            (last_at, last_done) = (Instant::now(), done);
            mgr.update_job(job.id, |j| {
                j.downloaded = done;
                j.total = total;
                j.speed = speed;
            });
        }
    };
    let result = result.and_then(|()| file.flush().map_err(|e| format!("Couldn't save the download: {e}")));
    drop(file);
    let fail = |e: String| {
        let _ = fs::remove_file(&part);
        Err(e)
    };
    if let Err(e) = result {
        return fail(e);
    }
    mgr.update_job(job.id, |j| j.downloaded = done);

    if asset.size > 0 && done != asset.size {
        return fail(format!(
            "The download was incomplete ({} of {}). Try again.",
            human_bytes(done),
            human_bytes(asset.size)
        ));
    }
    if let Some(expected) = &expected {
        if !expected.eq_ignore_ascii_case(&util::to_hex(&hasher.finalize())) {
            return fail("The download didn't match its published SHA-256 checksum, so it was discarded. Try again.".into());
        }
    }
    let _ = fs::remove_file(&dest);
    fs::rename(&part, &dest).map_err(|e| format!("Couldn't save the download: {e}"))?;
    Ok((dest, expected.is_some()))
}

/// The asset's checksum from GitHub's metadata, or failing that from the
/// release's SHA256SUMS.txt.
fn expected_sha256(mgr: &Manager, asset: &Asset, checksums_url: Option<&str>) -> Option<String> {
    if let Some(hex) = asset.digest.as_deref().and_then(|d| d.strip_prefix("sha256:")) {
        return Some(hex.to_owned());
    }
    let text = mgr.http.get(checksums_url?).call().ok()?.into_string().ok()?;
    text.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?.trim_start_matches('*');
        (name == asset.name).then(|| hash.to_owned())
    })
}

/// Installs with Windows Installer, which asks for admin permission itself.
#[cfg(windows)]
mod imp {
    use super::{Action, Job};
    use crate::installed::Source;
    use crate::manager::Manager;
    use std::ffi::OsStr;
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    pub fn install(mgr: &Manager, job: &Job) -> Result<String, String> {
        let file = job.file.as_deref().ok_or("The download is missing.")?;
        let log = mgr.paths.logs.join(format!("{}-{}.log", job.app.id, job.version));
        // A same-version reinstall has to be a repair; /i would do nothing.
        let mode = if job.action == Action::Reinstall { "/fvamus" } else { "/i" };
        let code = msiexec([OsStr::new(mode), file.as_os_str(), "/passive".as_ref(), "/norestart".as_ref(), "/l*v".as_ref(), log.as_os_str()])?;
        let restart = msi_outcome(code, &log)?;
        let (name, version) = (&job.app.name, &job.version);
        let mut message = match job.action {
            Action::Update => format!("{name} updated to {version}"),
            Action::Reinstall => format!("{name} {version} reinstalled"),
            _ => format!("{name} {version} installed"),
        };
        if restart {
            message.push_str(". Restart Windows to finish.");
        }
        Ok(message)
    }

    pub fn uninstall(mgr: &Manager, job: &Job) -> Result<String, String> {
        let name = &job.app.name;
        let installed = mgr.lock().installed.get(&job.app.id).cloned().ok_or_else(|| format!("{name} isn't installed."))?;
        match installed.source {
            Source::Msi { product_code } => {
                let log = mgr.paths.logs.join(format!("{}-uninstall.log", job.app.id));
                let code = msiexec([OsStr::new("/x"), product_code.as_ref(), "/passive".as_ref(), "/norestart".as_ref(), "/l*v".as_ref(), log.as_os_str()])?;
                msi_outcome(code, &log)?;
            }
            Source::Program { uninstall: Some(command) } => {
                // Registry uninstall strings are command lines, so run them as such.
                let status = Command::new("cmd.exe")
                    .raw_arg(format!("/S /C \"{command}\""))
                    .creation_flags(CREATE_NO_WINDOW)
                    .status()
                    .map_err(|e| format!("Couldn't start the uninstaller: {e}"))?;
                if !status.success() {
                    return Err(format!("The uninstaller failed (exit code {}).", status.code().unwrap_or(-1)));
                }
            }
            Source::Program { uninstall: None } => {
                return Err(format!("{name} has no registered uninstaller. Remove it from Windows Settings > Apps."));
            }
        }
        Ok(format!("{name} uninstalled"))
    }

    fn msiexec<I, S>(args: I) -> Result<i32, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let exe = std::env::var_os("SystemRoot")
            .map(|root| PathBuf::from(root).join("System32").join("msiexec.exe"))
            .unwrap_or_else(|| PathBuf::from("msiexec.exe"));
        let status = Command::new(exe)
            .args(args)
            .status()
            .map_err(|e| format!("Couldn't start Windows Installer: {e}"))?;
        Ok(status.code().unwrap_or(-1))
    }

    /// Interprets a Windows Installer exit code. Ok(true) means a restart is needed.
    fn msi_outcome(code: i32, log: &Path) -> Result<bool, String> {
        match code {
            0 => Ok(false),
            1641 | 3010 => Ok(true),
            1602 => Err("Cancelled: the installer was closed or admin permission was declined.".into()),
            1618 => Err("Another installation is already running. Try again when it finishes.".into()),
            1625 | 1643 => Err("Blocked by a system policy on this PC.".into()),
            1638 => Err("A different version is already installed. Uninstall it first.".into()),
            _ => Err(format!("Windows Installer failed (error {code}). Log: {}", log.display())),
        }
    }
}

/// Copies apps out of their disk images into /Applications. macOS asks for an
/// administrator's password when the user can't write there.
#[cfg(target_os = "macos")]
mod imp {
    use super::{Action, Job};
    use crate::installed::Source;
    use crate::manager::Manager;
    use std::fs;
    use std::path::Path;
    use std::process::{Command, Output};

    const APPLICATIONS: &str = "/Applications";

    pub fn install(mgr: &Manager, job: &Job) -> Result<String, String> {
        let file = job.file.as_deref().ok_or("The download is missing.")?;
        let mount = mgr.paths.downloads.join(format!("mount-{}", job.id));
        fs::create_dir_all(&mount).map_err(|e| format!("Couldn't open the disk image: {e}"))?;
        let attach = Command::new("/usr/bin/hdiutil")
            .arg("attach")
            .arg(file)
            .arg("-mountpoint")
            .arg(&mount)
            .args(["-nobrowse", "-noautoopen", "-readonly"])
            .output();
        if let Err(e) = checked(attach) {
            let _ = fs::remove_dir(&mount);
            return Err(format!("Couldn't open the disk image: {e}"));
        }
        let result = copy_app(&mount, job);
        let _ = Command::new("/usr/bin/hdiutil").args(["detach", "-force"]).arg(&mount).output();
        let _ = fs::remove_dir(&mount);
        result?;

        let (name, version) = (&job.app.name, &job.version);
        Ok(match job.action {
            Action::Update => format!("{name} updated to {version}"),
            Action::Reinstall => format!("{name} {version} reinstalled"),
            _ => format!("{name} {version} installed"),
        })
    }

    /// Copies the image's app next to the installed one, then swaps it in, so
    /// a failed copy never leaves a half-installed app behind.
    fn copy_app(mount: &Path, job: &Job) -> Result<(), String> {
        let app = fs::read_dir(mount)
            .map_err(|e| format!("Couldn't read the disk image: {e}"))?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|x| x == "app") && !p.is_symlink())
            .ok_or("The disk image doesn't contain an app.")?;
        let bundle = app.file_name().ok_or("The disk image doesn't contain an app.")?;
        let dest = Path::new(APPLICATIONS).join(bundle);
        let name = bundle.to_string_lossy();
        let staged = Path::new(APPLICATIONS).join(format!(".{name}.new-{}", job.id));
        let old = Path::new(APPLICATIONS).join(format!(".{name}.old-{}", job.id));

        if can_write(Path::new(APPLICATIONS)) {
            let _ = fs::remove_dir_all(&staged);
            checked(Command::new("/usr/bin/ditto").arg(&app).arg(&staged).output())
                .map_err(|e| format!("Couldn't copy {name} to Applications: {e}"))?;
            let swapped = (|| {
                if dest.exists() {
                    fs::rename(&dest, &old)?;
                }
                fs::rename(&staged, &dest)
            })();
            if let Err(e) = swapped {
                if !dest.exists() && old.exists() {
                    let _ = fs::rename(&old, &dest);
                }
                let _ = fs::remove_dir_all(&staged);
                return Err(format!("Couldn't replace {name} in Applications: {e}"));
            }
            let _ = fs::remove_dir_all(&old);
            Ok(())
        } else {
            let (app, dest, staged) = (quote(&app), quote(&dest), quote(&staged));
            as_admin(
                &format!(
                    "/bin/rm -rf {staged} && /usr/bin/ditto {app} {staged} && /bin/rm -rf {dest} && /bin/mv {staged} {dest}"
                ),
                &format!("install {}", job.app.name),
            )
        }
    }

    pub fn uninstall(mgr: &Manager, job: &Job) -> Result<String, String> {
        let name = &job.app.name;
        let installed = mgr.lock().installed.get(&job.app.id).cloned().ok_or_else(|| format!("{name} isn't installed."))?;
        let Source::Bundle { path } = installed.source;
        if trash(&path).is_err() {
            // Moving it needs permissions the user doesn't have, so it's deleted instead.
            as_admin(&format!("/bin/rm -rf {}", quote(&path)), &format!("uninstall {name}"))?;
            return Ok(format!("{name} uninstalled"));
        }
        Ok(format!("{name} moved to the Trash"))
    }

    fn trash(path: &Path) -> Result<(), String> {
        use objc2_foundation::{NSFileManager, NSString, NSURL};
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        NSFileManager::defaultManager()
            .trashItemAtURL_resultingItemURL_error(&url, None)
            .map_err(|e| e.localizedDescription().to_string())
    }

    /// Whether this user can create files in `dir`, as admins usually can in /Applications.
    fn can_write(dir: &Path) -> bool {
        let probe = dir.join(format!(".artcraft-apps-manager-{}", std::process::id()));
        let ok = fs::File::create(&probe).is_ok();
        let _ = fs::remove_file(&probe);
        ok
    }

    /// Runs a shell command as root, after macOS asks for an administrator's
    /// name and password, explaining that the app wants to `purpose`.
    fn as_admin(command: &str, purpose: &str) -> Result<(), String> {
        let applescript = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!(
            "do shell script \"{}\" with prompt \"{} wants to {}.\" with administrator privileges",
            applescript(command),
            crate::APP_NAME,
            applescript(purpose),
        );
        match Command::new("/usr/bin/osascript").arg("-e").arg(script).output() {
            Ok(out) if out.status.success() => Ok(()),
            // -128 is "User canceled".
            Ok(out) if String::from_utf8_lossy(&out.stderr).contains("-128") => {
                Err("Cancelled: administrator permission was declined.".into())
            }
            other => Err(format!("Couldn't {purpose}: {}", checked(other).err().unwrap_or_default())),
        }
    }

    /// A path quoted for the shell.
    fn quote(path: &Path) -> String {
        format!("'{}'", path.to_string_lossy().replace('\'', r"'\''"))
    }

    /// The command's error output if it failed.
    fn checked(output: std::io::Result<Output>) -> Result<(), String> {
        let out = output.map_err(|e| e.to_string())?;
        if out.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        Err(if stderr.is_empty() { format!("exit code {}", out.status.code().unwrap_or(-1)) } else { stderr })
    }
}
