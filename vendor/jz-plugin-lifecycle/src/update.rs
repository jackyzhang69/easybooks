//! Passive update notice and host-installed sync, run after a command has
//! finished. Never panics, never returns an error, never changes the
//! command's exit code. The only wait it can add is the bounded catalog GET
//! (at most [`CATALOG_WAIT`], at most once per [`CHECK_INTERVAL_SECS`], or
//! once per [`RETRY_AFTER_FAILURE_SECS`] after a failure).

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::install::{self, InstallRequest};

pub const UPDATE_NOTICE_SCHEMA: &str = "jz.plugin.update_notice.v1";
pub const MARKETPLACE_CATALOG_URL: &str =
    "https://raw.githubusercontent.com/jackyzhang69/plugins/main/.claude-plugin/marketplace.json";
pub const CHECK_INTERVAL_SECS: u64 = 86_400;
pub const RETRY_AFTER_FAILURE_SECS: u64 = 3_600;

/// Hard bound on the catalog GET. ureq's own timeout does not bound DNS, so
/// the GET runs on a spawned thread and is abandoned after this.
const CATALOG_WAIT: Duration = Duration::from_secs(2);
const CATALOG_MAX_BYTES: u64 = 2 << 20;

pub struct UpdateEnv<'a> {
    pub home: &'a Path,
    pub catalog_url: &'a str,
    pub now_secs: u64,
}

/// Numeric dot-part comparison. Any non-numeric part, or an empty string, is `false`.
pub fn version_less(left: &str, right: &str) -> bool {
    fn parts(version: &str) -> Option<Vec<u64>> {
        if version.is_empty() {
            return None;
        }
        version
            .split('.')
            .map(|part| part.parse::<u64>().ok())
            .collect()
    }
    let (Some(mut left), Some(mut right)) = (parts(left), parts(right)) else {
        return false;
    };
    let len = left.len().max(right.len());
    left.resize(len, 0);
    right.resize(len, 0);
    left < right
}

fn is_numeric_version(version: &str) -> bool {
    !version.is_empty() && version.split('.').all(|part| part.parse::<u64>().is_ok())
}

#[derive(Default)]
struct State {
    checked_at: Option<u64>,
    latest: Option<String>,
    notified_at: Option<u64>,
    notified_version: Option<String>,
}

impl State {
    fn load(path: &Path) -> State {
        let Some(Value::Object(map)) = fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        else {
            return State::default();
        };
        let text = |key: &str| map.get(key).and_then(Value::as_str).map(str::to_owned);
        State {
            checked_at: map.get("checked_at").and_then(Value::as_u64),
            latest: text("latest"),
            notified_at: map.get("notified_at").and_then(Value::as_u64),
            notified_version: text("notified_version"),
        }
    }

    fn save(&self, path: &Path) {
        let mut map = serde_json::Map::new();
        if let Some(checked_at) = self.checked_at {
            map.insert("checked_at".into(), checked_at.into());
        }
        if let Some(latest) = &self.latest {
            map.insert("latest".into(), latest.clone().into());
        }
        if let Some(notified_at) = self.notified_at {
            map.insert("notified_at".into(), notified_at.into());
        }
        if let Some(version) = &self.notified_version {
            map.insert("notified_version".into(), version.clone().into());
        }
        let Ok(bytes) = serde_json::to_vec(&Value::Object(map)) else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        // Write a sibling then rename: a link at `path` is replaced, never written through.
        let tmp = path.with_extension(format!("json.tmp-{}", std::process::id()));
        if fs::write(&tmp, bytes).is_ok() && fs::rename(&tmp, path).is_err() {
            let _ = fs::remove_file(&tmp);
        }
    }
}

/// `true` when `earlier` is at least `interval` before `now`; a timestamp
/// later than `now` counts as due.
fn is_due(earlier: Option<u64>, now: u64, interval: u64) -> bool {
    match earlier {
        None => true,
        Some(earlier) => earlier > now || now.saturating_sub(earlier) >= interval,
    }
}

/// GET the catalog without credentials and without following redirects.
/// `Ok(None)` when the catalog has no usable entry for `plugin_id`.
fn fetch_latest(url: &str, plugin_id: &str) -> Option<Option<String>> {
    let agent = ureq::AgentBuilder::new()
        .timeout(CATALOG_WAIT)
        .redirects(0)
        .build();
    let response = agent.get(url).call().ok()?;
    if response.status() != 200 {
        return None;
    }
    let mut body = Vec::new();
    response
        .into_reader()
        .take(CATALOG_MAX_BYTES)
        .read_to_end(&mut body)
        .ok()?;
    let catalog: Value = serde_json::from_slice(&body).ok()?;
    let latest = catalog
        .get("plugins")?
        .as_array()?
        .iter()
        .find(|entry| entry.get("name").and_then(Value::as_str) == Some(plugin_id))
        .and_then(|entry| entry.get("version")?.as_str())
        .filter(|version| is_numeric_version(version))
        .map(str::to_owned);
    Some(latest)
}

fn fetch_with_deadline(url: &str, plugin_id: &str) -> Option<Option<String>> {
    let (sender, receiver) = mpsc::channel();
    let (url, plugin_id) = (url.to_owned(), plugin_id.to_owned());
    std::thread::Builder::new()
        .name("jz-update-check".into())
        .spawn(move || {
            let _ = sender.send(fetch_latest(&url, &plugin_id));
        })
        .ok()?;
    // The thread is never joined: a hung DNS lookup must not hold the process.
    receiver.recv_timeout(CATALOG_WAIT).ok().flatten()
}

fn notice_line(plugin_id: &str, current: &str, latest: &str) -> String {
    let quote = |text: &str| serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into());
    format!(
        "{{\"schema\":{},\"plugin_id\":{},\"current\":{},\"latest\":{},\"say_to_user\":{}}}",
        quote(UPDATE_NOTICE_SCHEMA),
        quote(plugin_id),
        quote(current),
        quote(latest),
        quote("A newer version of this plugin is available. I can update it whenever you like; nothing is blocked."),
    )
}

/// Check the marketplace catalog (at most daily) and write at most one notice
/// line to `out`. Every error is swallowed.
pub fn run_check(plugin_id: &str, version: &str, env: &UpdateEnv, out: &mut dyn Write) {
    let state_path = env.home.join(plugin_id).join("update-check.json");
    let mut state = State::load(&state_path);
    let now = env.now_secs;

    if is_due(state.checked_at, now, CHECK_INTERVAL_SECS) {
        match fetch_with_deadline(env.catalog_url, plugin_id) {
            Some(latest) => {
                state.latest = latest;
                state.checked_at = Some(now);
            }
            None => {
                state.checked_at = Some(
                    now.saturating_sub(CHECK_INTERVAL_SECS)
                        .saturating_add(RETRY_AFTER_FAILURE_SECS),
                );
            }
        }
        state.save(&state_path);
    }

    let Some(latest) = state.latest.clone() else {
        return;
    };
    let announce = version_less(version, &latest)
        && (state.notified_version.as_deref() != Some(latest.as_str())
            || is_due(state.notified_at, now, CHECK_INTERVAL_SECS));
    if !announce {
        return;
    }
    if writeln!(out, "{}", notice_line(plugin_id, version, &latest)).is_ok() {
        let _ = out.flush();
        state.notified_at = Some(now);
        state.notified_version = Some(latest);
        state.save(&state_path);
    }
}

fn skip_update_work() -> bool {
    cfg!(debug_assertions)
        || std::env::var_os("CI").is_some_and(|value| !value.is_empty())
        || std::env::var("JZ_PLUGIN_UPDATE_CHECK").is_ok_and(|value| value == "off")
}

/// `<pkg>` when the running executable is exactly
/// `<pkg>/bin/<platform>/<binary>[.exe]` and `<pkg>/runtime-manifest.json` exists.
fn host_package_of_running_exe(binary_name: &str) -> Option<PathBuf> {
    let exe = fs::canonicalize(std::env::current_exe().ok()?).ok()?;
    let expected = if cfg!(windows) && !binary_name.ends_with(".exe") {
        format!("{binary_name}.exe")
    } else {
        binary_name.to_string()
    };
    if exe.file_name()?.to_str()? != expected {
        return None;
    }
    let platform_dir = exe.parent()?;
    if platform_dir.file_name()?.to_str()? != install::platform_id() {
        return None;
    }
    let bin_dir = platform_dir.parent()?;
    if bin_dir.file_name()?.to_str()? != "bin" {
        return None;
    }
    let package = bin_dir.parent()?;
    package
        .join("runtime-manifest.json")
        .is_file()
        .then(|| package.to_path_buf())
}

/// Bring `plugins/<id>/current` up to a host-installed newer package, else
/// sweep leftovers. Result ignored: neither may affect the command.
fn sync_or_sweep(plugin_id: &str, binary_name: &str, stale_caches: &[PathBuf], home: &Path) {
    let root = home.join("plugins").join(plugin_id);
    let host_newer = host_package_of_running_exe(binary_name).filter(|package| {
        let outside_root = match (fs::canonicalize(package), fs::canonicalize(&root)) {
            (Ok(package), Ok(root)) => !package.starts_with(root),
            (Ok(_), Err(_)) => true,
            _ => false,
        };
        outside_root
            && match (
                install::package_version(&root.join("current")),
                install::package_version(package),
            ) {
                (Some(installed), Some(offered)) => version_less(&installed, &offered),
                _ => false,
            }
    });
    match host_newer {
        Some(package) => {
            let host_roots = install::default_host_cache_roots(plugin_id);
            let _ = install::fresh_install(&InstallRequest {
                plugin_id,
                binary_name,
                package_root: &package,
                stale_caches,
                home,
                host_cache_roots: &host_roots,
            });
        }
        None => {
            let _ = install::sweep(home, plugin_id);
        }
    }
}

/// Call once on the normal CLI exit path, after output is written. Skipped in
/// debug builds, when `CI` is non-empty and when `JZ_PLUGIN_UPDATE_CHECK=off`.
pub fn after_command(plugin_id: &str, version: &str, binary_name: &str, stale_caches: &[PathBuf]) {
    if skip_update_work() {
        return;
    }
    // Belt and braces: an unexpected panic here must never reach the user's command.
    let _ = std::panic::catch_unwind(|| {
        let Ok(home) = crate::home::platform_home() else {
            return;
        };
        sync_or_sweep(plugin_id, binary_name, stale_caches, &home);
        let Ok(elapsed) = SystemTime::now().duration_since(UNIX_EPOCH) else {
            return;
        };
        let now_secs = elapsed.as_secs();
        let env = UpdateEnv {
            home: &home,
            catalog_url: MARKETPLACE_CATALOG_URL,
            now_secs,
        };
        run_check(plugin_id, version, &env, &mut std::io::stderr());
    });
}
