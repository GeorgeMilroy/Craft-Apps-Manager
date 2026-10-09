//! Shared state, and the operations the window, the tray and the background
//! workers perform on it.

use crate::catalog::{self, AppInfo};
use crate::github::{self, Arch, Release};
use crate::installed::{self, Installed};
use crate::jobs::{self, Action, Job, Status};
use crate::settings::{self, Paths, Settings, Store};
use crate::{autostart, check, notify, platform, tray, util};
use eframe::egui;
use std::collections::{BTreeMap, HashMap};
use std::process::Command;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};

pub struct State {
    pub settings: Settings,
    pub store: Store,
    pub catalog: Vec<AppInfo>,
    pub installed: HashMap<String, Installed>,
    /// Per-app errors from the last update check.
    pub check_errors: BTreeMap<String, String>,
    /// Set when the last check failed for every app (offline, rate limited).
    pub check_error: Option<String>,
    pub checking: bool,
    pub jobs: Vec<Job>,
    next_job_id: u64,
}

impl State {
    pub fn app(&self, id: &str) -> Option<&AppInfo> {
        self.catalog.iter().find(|a| a.id == id)
    }

    pub fn latest(&self, id: &str) -> Option<&Release> {
        self.store.releases.get(id)?.release.as_ref()
    }

    pub fn has_update(&self, id: &str) -> bool {
        match (self.installed.get(id), self.latest(id)) {
            (Some(i), Some(r)) => github::is_newer(&r.version(), &i.version),
            _ => false,
        }
    }

    /// Installed apps with a newer release, in catalog order.
    pub fn updates(&self) -> Vec<String> {
        self.catalog.iter().filter(|a| self.has_update(&a.id)).map(|a| a.id.clone()).collect()
    }

    pub fn active_job(&self, app_id: &str) -> Option<&Job> {
        self.jobs.iter().find(|j| j.app.id == app_id && j.status.is_active())
    }

    pub fn enqueue(&mut self, app_id: &str, action: Action, arch: Arch) -> Result<(), String> {
        let app = self.app(app_id).ok_or("Unknown app.")?.clone();
        if self.active_job(app_id).is_some() {
            return Err(format!("{} is already in the queue.", app.name));
        }
        let id = self.next_job_id;
        self.next_job_id += 1;

        if action == Action::Uninstall {
            let installed = self.installed.get(app_id).ok_or_else(|| format!("{} isn't installed.", app.name))?;
            self.jobs.push(Job::new(id, app, action, installed.version.clone(), None, None));
            return Ok(());
        }

        let release = self
            .latest(app_id)
            .ok_or_else(|| format!("No release information for {} yet. Check for updates first.", app.name))?;
        let version = release.version();
        let asset = github::pick_asset(&app, release, arch)
            .cloned()
            .ok_or_else(|| format!("{} {version} has no installer for {} {}.", app.name, platform::OS_NAME, arch.label()))?;
        let checksums = github::checksums_asset(release).map(|a| a.browser_download_url.clone());
        self.jobs.push(Job::new(id, app, action, version, Some(asset), checksums));
        Ok(())
    }
}

pub struct Manager {
    state: Mutex<State>,
    jobs_cv: Condvar,
    pub paths: Paths,
    pub arch: Arch,
    pub http: ureq::Agent,
    ctx: OnceLock<egui::Context>,
    check_tx: OnceLock<Sender<()>>,
}

impl Manager {
    pub fn load() -> Arc<Self> {
        let paths = Paths::new();
        let settings: Settings = settings::load(&paths.settings_file());
        let mut store: Store = settings::load(&paths.store_file());

        let mut catalog = catalog::builtin();
        for app in &store.discovered {
            if !catalog.iter().any(|a| a.id == app.id) {
                catalog.push(app.clone());
            }
        }
        // Drop cached data for apps no longer listed (such as ArtCraft).
        store.releases.retain(|id, _| catalog.iter().any(|a| &a.id == id));
        let installed = installed::detect_all(&catalog, &installed::scan(&catalog));

        // Partial files from downloads cut short by the last exit.
        if let Ok(entries) = std::fs::read_dir(&paths.downloads) {
            for e in entries.flatten() {
                if e.path().extension().is_some_and(|x| x == "part") {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        // Keeps the startup entry pointing at this executable.
        let _ = autostart::set(settings.start_at_login);

        Arc::new(Manager {
            state: Mutex::new(State {
                settings,
                store,
                catalog,
                installed,
                check_errors: BTreeMap::new(),
                check_error: None,
                checking: false,
                jobs: Vec::new(),
                next_job_id: 1,
            }),
            jobs_cv: Condvar::new(),
            paths,
            arch: crate::platform::native_arch(),
            http: github::agent(),
            ctx: OnceLock::new(),
            check_tx: OnceLock::new(),
        })
    }

    /// Starts the update checker, the download and install workers, and the tray icon.
    pub fn start(self: &Arc<Self>, ctx: egui::Context) {
        let _ = self.ctx.set(ctx);
        let (tx, rx) = mpsc::channel();
        let _ = self.check_tx.set(tx);
        check::spawn(self.clone(), rx);
        jobs::spawn(self.clone());
        tray::spawn(self.clone());
    }

    pub fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Call after changing state: repaints the window and refreshes the tray.
    pub fn changed(&self) {
        if let Some(ctx) = self.ctx.get() {
            ctx.request_repaint();
        }
        tray::refresh();
    }

    pub fn check_now(&self) {
        if let Some(tx) = self.check_tx.get() {
            let _ = tx.send(());
        }
    }

    pub fn enqueue(&self, app_id: &str, action: Action) -> Result<(), String> {
        let result = self.lock().enqueue(app_id, action, self.arch);
        self.jobs_cv.notify_all();
        self.changed();
        result
    }

    pub fn update_all(&self) {
        {
            let mut st = self.lock();
            for id in st.updates() {
                let _ = st.enqueue(&id, Action::Update, self.arch);
            }
        }
        self.jobs_cv.notify_all();
        self.changed();
    }

    /// Blocks until a job is in `from`, moves it to `to` and returns a copy.
    pub fn next_job(&self, from: &Status, to: Status) -> Job {
        let mut st = self.lock();
        loop {
            if let Some(job) = st.jobs.iter_mut().find(|j| j.status == *from) {
                job.status = to;
                let job = job.clone();
                drop(st);
                self.changed();
                return job;
            }
            st = self.jobs_cv.wait(st).unwrap_or_else(PoisonError::into_inner);
        }
    }

    pub fn update_job(&self, id: u64, f: impl FnOnce(&mut Job)) {
        {
            let mut st = self.lock();
            if let Some(job) = st.jobs.iter_mut().find(|j| j.id == id) {
                f(job);
                if job.status.is_finished() && job.finished_at.is_none() {
                    job.finished_at = Some(util::now_unix());
                }
            }
        }
        self.jobs_cv.notify_all();
        self.changed();
    }

    pub fn cancel(&self, id: u64) {
        {
            let mut st = self.lock();
            let keep = st.settings.keep_installers;
            let Some(job) = st.jobs.iter_mut().find(|j| j.id == id) else { return };
            match job.status {
                Status::Queued | Status::ReadyToInstall => {
                    if let (Some(file), false) = (&job.file, keep) {
                        let _ = std::fs::remove_file(file);
                    }
                    job.status = Status::Cancelled;
                    job.finished_at = Some(util::now_unix());
                }
                // The downloader notices, cleans up and marks the job cancelled.
                Status::Downloading => job.cancel.store(true, std::sync::atomic::Ordering::SeqCst),
                _ => {}
            }
        }
        self.jobs_cv.notify_all();
        self.changed();
    }

    pub fn clear_finished(&self) {
        self.lock().jobs.retain(|j| j.status.is_active());
        self.changed();
    }

    pub fn refresh_installed(&self) {
        let catalog = self.lock().catalog.clone();
        let found = installed::detect_all(&catalog, &installed::scan(&catalog));
        self.lock().installed = found;
    }

    pub fn launch(&self, app_id: &str) -> Result<(), String> {
        let exe = self
            .lock()
            .installed
            .get(app_id)
            .and_then(|i| i.exe.clone())
            .ok_or("Couldn't find the app's program file.")?;
        // On a Mac, `exe` is the app bundle, which only `open` can start.
        #[cfg(target_os = "macos")]
        let mut command = {
            let mut c = Command::new("/usr/bin/open");
            c.arg(&exe);
            c
        };
        #[cfg(windows)]
        let mut command = {
            let mut c = Command::new(&exe);
            c.current_dir(exe.parent().unwrap_or(std::path::Path::new(".")));
            c
        };
        let child = command.spawn().map_err(|e| format!("Couldn't start {}: {e}", exe.display()))?;
        // `open` exits once the app is running; waiting for it stops it lingering as a zombie.
        #[cfg(target_os = "macos")]
        std::thread::spawn(move || {
            let mut child = child;
            child.wait()
        });
        #[cfg(windows)]
        drop(child);
        Ok(())
    }

    pub fn set_muted(&self, app_id: &str, muted: bool) {
        let mut settings = self.lock().settings.clone();
        if muted {
            settings.muted.insert(app_id.to_owned());
        } else {
            settings.muted.remove(app_id);
        }
        let _ = self.update_settings(settings);
    }

    pub fn update_settings(&self, new: Settings) -> Result<(), String> {
        let old_autostart = self.lock().settings.start_at_login;
        if new.start_at_login != old_autostart {
            autostart::set(new.start_at_login).map_err(|e| format!("Couldn't change the sign-in setting: {e}"))?;
        }
        {
            let mut st = self.lock();
            st.settings = new;
            let _ = settings::save(&self.paths.settings_file(), &st.settings);
        }
        self.changed();
        Ok(())
    }

    pub fn save_store(&self) {
        let st = self.lock();
        let _ = settings::save(&self.paths.store_file(), &st.store);
    }

    pub fn save(&self) {
        let st = self.lock();
        let _ = settings::save(&self.paths.settings_file(), &st.settings);
        let _ = settings::save(&self.paths.store_file(), &st.store);
    }

    /// Saves and exits. The tray thread does the exit so its icon is removed
    /// first; otherwise a dead icon lingers until the mouse passes over it.
    pub fn quit(&self) {
        self.save();
        if !tray::quit() {
            std::process::exit(0);
        }
    }

    pub fn notify_job_result(&self, job: &Job, result: &Result<String, String>) {
        if !self.lock().settings.notify_installs {
            return;
        }
        match result {
            Ok(message) => notify::show(message.clone(), ""),
            Err(error) => notify::show(
                format!("{} {} failed", job.app.name, job.action.label().to_lowercase()),
                error.clone(),
            ),
        }
    }
}
