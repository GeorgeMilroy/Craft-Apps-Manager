//! Periodic update checks against GitHub.

use crate::catalog::{AppInfo, EXCLUDED, OWNER};
use crate::github::{self, Client, Fetch, Release};
use crate::jobs::Action;
use crate::manager::Manager;
use crate::settings::{self, CachedRelease};
use crate::util::now_unix;
use crate::{icons, installed, notify};
use std::collections::HashMap;
use std::io::Read;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// Until the first check of a session, data older than this is refreshed
/// rather than waiting out the full interval. Restarting the app within this
/// window reuses the cached results, which saves GitHub requests.
const STARTUP_REFRESH_SECS: u64 = 30 * 60;
/// Looking for new apps costs a request, so it's done at most daily.
const DISCOVERY_INTERVAL_SECS: u64 = 24 * 3600;

/// Checks when one is due or requested on `rx`.
pub fn spawn(mgr: Arc<Manager>, rx: Receiver<()>) {
    thread::Builder::new()
        .name("update-check".into())
        .spawn(move || {
            let mut retry_at: Option<u64> = None;
            let mut checked = false;
            loop {
                let due = {
                    let st = mgr.lock();
                    let mut interval = u64::from(st.settings.check_interval_hours.max(1)) * 3600;
                    if !checked {
                        interval = interval.min(STARTUP_REFRESH_SECS);
                    }
                    st.store.last_check.map_or(0, |t| t + interval)
                };
                let due = retry_at.map_or(due, |r| r.min(due));
                let now = now_unix();
                if due > now {
                    // Wake at least once a minute, since timers can stall while the PC sleeps.
                    match rx.recv_timeout(Duration::from_secs((due - now).min(60))) {
                        Ok(()) => {}
                        Err(RecvTimeoutError::Timeout) => continue,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                while rx.try_recv().is_ok() {}
                // After a failed check (offline at sign-in, say), try again in 5 minutes.
                retry_at = (!run(&mgr)).then(|| now_unix() + 300);
                checked = true;
            }
        })
        .expect("couldn't start the update checker");
}

/// Returns false if no app could be checked.
fn run(mgr: &Manager) -> bool {
    let (catalog, etags, repos_etag, discover_due) = {
        let mut st = mgr.lock();
        st.checking = true;
        let etags: HashMap<String, Option<String>> =
            st.store.releases.iter().map(|(id, c)| (id.clone(), c.etag.clone())).collect();
        let discover_due = st.store.last_discovery.is_none_or(|t| now_unix() >= t + DISCOVERY_INTERVAL_SECS);
        (st.catalog.clone(), etags, st.store.repos_etag.clone(), discover_due)
    };
    mgr.changed();

    let client = Client::new(mgr.http.clone());
    let (repos_etag, new_apps) = if discover_due {
        discover(&client, &catalog, repos_etag.as_deref())
    } else {
        (None, Vec::new())
    };
    for app in &new_apps {
        fetch_icon(mgr, &app.id);
    }
    let apps: Vec<AppInfo> = catalog.into_iter().chain(new_apps.iter().cloned()).collect();

    let results: Vec<(String, Result<Fetch<Vec<Release>>, String>)> = thread::scope(|s| {
        let handles: Vec<_> = apps
            .iter()
            .map(|app| {
                let etag = etags.get(&app.id).cloned().flatten();
                let client = &client;
                s.spawn(move || (app.id.clone(), client.releases(&app.id, etag.as_deref())))
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    });
    let found = installed::scan(&apps);

    let mut guard = mgr.lock();
    let st = &mut *guard;
    if discover_due {
        st.store.last_discovery = Some(now_unix());
    }
    if let Some(etag) = repos_etag {
        st.store.repos_etag = Some(etag);
    }
    for app in new_apps {
        st.store.discovered.push(app.clone());
        st.catalog.push(app);
    }

    let mut first_error = None;
    let mut failures = 0;
    for (id, result) in results.iter() {
        match result {
            Ok(Fetch::Fresh { value, etag }) => {
                let Some(app) = apps.iter().find(|a| &a.id == id) else { continue };
                let release = github::latest_release(app, value);
                st.store.releases.insert(id.clone(), CachedRelease { etag: etag.clone(), release });
                st.check_errors.remove(id);
            }
            Ok(Fetch::NotModified) => {
                st.check_errors.remove(id);
            }
            Err(e) => {
                failures += 1;
                first_error.get_or_insert_with(|| e.clone());
                st.check_errors.insert(id.clone(), e.clone());
            }
        }
    }
    let ok = results.is_empty() || failures < results.len();
    st.check_error = if ok { None } else { first_error };
    if ok {
        st.store.last_check = Some(now_unix());
    }
    st.installed = installed::detect_all(&st.catalog, &found);

    // Updates not announced yet. Each version is announced once, even if muted.
    let mut fresh = Vec::new();
    for id in st.updates() {
        let Some(latest) = st.latest(&id).map(Release::version) else { continue };
        if st.store.notified.get(&id) == Some(&latest) {
            continue;
        }
        st.store.notified.insert(id.clone(), latest.clone());
        if !st.settings.muted.contains(&id) {
            let name = st.app(&id).map(|a| a.name.clone()).unwrap_or_default();
            let from = st.installed.get(&id).map(|i| i.version.clone()).unwrap_or_default();
            fresh.push((name, from, latest));
        }
    }

    let auto = st.settings.auto_install_updates;
    if auto {
        for id in st.updates() {
            let _ = st.enqueue(&id, Action::Update, mgr.arch);
        }
    }
    let notify_updates = st.settings.notify_updates;
    st.checking = false;
    let _ = settings::save(&mgr.paths.store_file(), &st.store);
    drop(guard);

    if notify_updates && !fresh.is_empty() {
        let (title, body) = update_message(&fresh, auto);
        notify::show(title, body);
    }
    mgr.changed();
    ok
}

fn update_message(updates: &[(String, String, String)], auto: bool) -> (String, String) {
    let next = if auto { "Installing now." } else { "Open ArtCraft Apps Manager to update." };
    if let [(name, from, to)] = updates {
        return (format!("{name} {to} is available"), format!("You have {from}. {next}"));
    }
    let names: Vec<String> = updates.iter().map(|(name, _, to)| format!("{name} {to}")).collect();
    (format!("{} app updates are available", updates.len()), format!("{}. {next}", names.join(", ")))
}

/// Finds storytold repositories named "*craft" that aren't in the catalog
/// yet but publish installers for this platform. Returns the repo list's
/// ETag and any new apps. Costs one request, plus one per candidate repository.
fn discover(client: &Client, catalog: &[AppInfo], etag: Option<&str>) -> (Option<String>, Vec<AppInfo>) {
    let Ok(Fetch::Fresh { value: repos, etag }) = client.org_repos(etag) else {
        return (None, Vec::new());
    };
    let new_apps = repos
        .iter()
        .filter(|r| !r.fork && !r.archived && r.name.to_ascii_lowercase().ends_with("craft"))
        .filter(|r| !catalog.iter().any(|a| a.id.eq_ignore_ascii_case(&r.name)))
        .filter(|r| !EXCLUDED.iter().any(|x| x.eq_ignore_ascii_case(&r.name)))
        .filter_map(|r| {
            let probe = AppInfo::discovered(&r.name, &r.name, "", None);
            let Ok(Fetch::Fresh { value, .. }) = client.releases(&r.name, None) else { return None };
            let release = github::latest_release(&probe, &value)?;
            // Release names look like "GridCraft v0.3.0".
            let name = release
                .name
                .as_deref()
                .and_then(|n| n.split(" v").next())
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| r.name.clone());
            let description = r.description.as_deref().unwrap_or("");
            Some(AppInfo::discovered(&r.name, &name, description, r.homepage.clone()))
        })
        .collect();
    (etag, new_apps)
}

/// Saves a discovered app's icon from the place every Craft app keeps it.
/// Best effort: without it the app shows a plain tile.
fn fetch_icon(mgr: &Manager, id: &str) {
    let url = format!(
        "https://raw.githubusercontent.com/{OWNER}/{id}/HEAD/assets/app-icon/hicolor/128x128/apps/ai.storyteller.{id}.png"
    );
    let Ok(resp) = mgr.http.get(&url).call() else { return };
    let mut bytes = Vec::new();
    if resp.into_reader().take(4 << 20).read_to_end(&mut bytes).is_ok() && bytes.starts_with(b"\x89PNG") {
        let path = icons::downloaded_path(&mgr.paths.data, id);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, bytes);
    }
}
