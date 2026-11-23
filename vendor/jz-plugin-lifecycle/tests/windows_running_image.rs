//! Windows-only proof that the shared install code can replace the package
//! its own process is running from, and never follows reparse points.
//!
//! The test binary copies itself into `current/bin/win32-x64/helper.exe`,
//! runs that copy as a child (`helper_entry`), and the child runs
//! `fresh_install` for a newer package while its own image is live inside the
//! old `current`. The coordinator executes this on a Windows host:
//! `cargo test --test windows_running_image`.
#![cfg(windows)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use jz_plugin_lifecycle::install::{fresh_install, sweep, InstallRequest};
use serde_json::{json, Value};

const ID: &str = "winlife";
const BIN: &str = "helper";
/// Set only on the child process; the parent test never touches its own environment.
const HELPER_WORKDIR: &str = "JZ_LIFECYCLE_HELPER_WORKDIR";

fn write(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn names(dir: &Path) -> BTreeSet<String> {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

/// Runs inside the child copy only; a no-op when started as an ordinary test.
#[test]
fn helper_entry() {
    let Some(workdir) = std::env::var_os(HELPER_WORKDIR) else {
        return;
    };
    let work = PathBuf::from(workdir);
    let home = work.join("home");
    let package = work.join("pkg-new");
    let result = fresh_install(&InstallRequest {
        plugin_id: ID,
        binary_name: BIN,
        package_root: &package,
        stale_caches: &[],
        home: &home,
        host_cache_roots: &[],
    });
    let report = match result {
        Ok(report) => json!({
            "ok": true,
            "version": report.version,
            "leftover": report.leftover.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>(),
            "pending_cleanup": report.pending_cleanup.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>(),
        }),
        Err(error) => json!({"ok": false, "error": error.to_string()}),
    };
    fs::write(work.join("report.json"), report.to_string()).unwrap();
}

#[test]
fn fresh_install_replaces_the_package_it_is_running_from() {
    let tmp = tempfile::tempdir().unwrap();
    let work = fs::canonicalize(tmp.path()).unwrap();
    let home = work.join("home");
    let plugins = home.join("plugins").join(ID);

    // Old package whose binary is this very test executable (a live image once started).
    let old_bin = plugins.join("current/bin/win32-x64/helper.exe");
    write(
        &plugins.join("current/runtime-manifest.json"),
        b"{\"version\":\"1.0.0\"}",
    );
    write(&old_bin, b"");
    fs::copy(std::env::current_exe().unwrap(), &old_bin).unwrap();
    write(&home.join("token/user.json"), b"secret");
    write(&home.join(ID).join("config.json"), b"cfg");

    // Newer package.
    let new = work.join("pkg-new");
    write(
        &new.join("runtime-manifest.json"),
        b"{\"version\":\"2.0.0\"}",
    );
    write(&new.join("bin/win32-x64/helper.exe"), b"new-image");

    let status = Command::new(&old_bin)
        .args(["--exact", "helper_entry", "--nocapture", "--test-threads=1"])
        .env(HELPER_WORKDIR, &work)
        .status()
        .unwrap();
    assert!(status.success(), "child failed: {status:?}");

    let report: Value =
        serde_json::from_slice(&fs::read(work.join("report.json")).unwrap()).unwrap();
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["version"], "2.0.0");
    assert_eq!(report["leftover"], json!([]), "{report}");
    let pending = report["pending_cleanup"].as_array().unwrap();
    assert_eq!(pending.len(), 1, "{report}");
    let pending_name = Path::new(pending[0].as_str().unwrap())
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(
        pending_name.starts_with(".running-") && pending_name.ends_with("helper.exe"),
        "unexpected pending entry {pending_name}"
    );

    // New current is live; the old image survives only as the pending entry.
    assert_eq!(
        fs::read(plugins.join("current/bin/win32-x64/helper.exe")).unwrap(),
        b"new-image"
    );
    assert_eq!(
        names(&plugins),
        BTreeSet::from(["current".to_string(), pending_name.clone()])
    );
    assert_eq!(fs::read(home.join("token/user.json")).unwrap(), b"secret");
    assert_eq!(fs::read(home.join(ID).join("config.json")).unwrap(), b"cfg");

    // The child has exited: the next sweep retires the old image.
    assert_eq!(sweep(&home, ID), Vec::<PathBuf>::new());
    assert_eq!(names(&plugins), BTreeSet::from(["current".to_string()]));
}

#[test]
fn junctions_are_removed_as_links_and_their_targets_survive() {
    let tmp = tempfile::tempdir().unwrap();
    let work = fs::canonicalize(tmp.path()).unwrap();
    let home = work.join("home");
    let plugins = home.join("plugins").join(ID);
    fs::create_dir_all(&plugins).unwrap();

    // Legacy layout: `current` is a junction to `1.2.10`, plus a junction escaping the root.
    write(
        &plugins.join("1.2.10/runtime-manifest.json"),
        b"{\"version\":\"1.2.10\"}",
    );
    let outside = work.join("outside");
    write(&outside.join("precious.txt"), b"keep");
    for (link, target) in [
        (plugins.join("current"), plugins.join("1.2.10")),
        (plugins.join("escape"), outside.clone()),
    ] {
        let status = Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(&link)
            .arg(&target)
            .output()
            .unwrap()
            .status;
        assert!(status.success(), "mklink /J {link:?} failed");
    }

    let new = work.join("pkg-new");
    write(
        &new.join("runtime-manifest.json"),
        b"{\"version\":\"2.0.0\"}",
    );
    write(&new.join("bin/win32-x64/helper.exe"), b"new-image");
    let report = fresh_install(&InstallRequest {
        plugin_id: ID,
        binary_name: BIN,
        package_root: &new,
        stale_caches: &[],
        home: &home,
        host_cache_roots: &[],
    })
    .unwrap();
    assert!(report.leftover.is_empty(), "{:?}", report.leftover);
    assert_eq!(names(&plugins), BTreeSet::from(["current".to_string()]));
    assert_eq!(fs::read(outside.join("precious.txt")).unwrap(), b"keep");
    assert_eq!(
        fs::read(plugins.join("current/runtime-manifest.json")).unwrap(),
        b"{\"version\":\"2.0.0\"}"
    );
}
