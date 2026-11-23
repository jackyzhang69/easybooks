//! `doctor --repair-install` end to end, always against a throwaway home and
//! host caches passed to the child process only (the real home is never touched).

#![cfg(not(windows))]

use assert_cmd::Command;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn write(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

#[test]
fn repair_install_leaves_only_current_and_keeps_user_state() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let hostcache = temp.path().join("hostcache");
    let othercache = temp.path().join("othercache");
    let user_home = temp.path().join("userhome");
    let platform = jz_plugin_lifecycle::install::platform_id();
    let version = env!("CARGO_PKG_VERSION");

    // Old installed state and user data.
    write(&home.join("plugins/easybooks/0.0.1/old.txt"), "old");
    write(&home.join("plugins/easybooks/.previous-x/old.txt"), "old");
    write(&home.join("token/user.json"), "{\"token\":\"keep\"}");
    write(&home.join("easybooks/config.json"), "{\"keep\":true}");
    write(&hostcache.join("easybooks/0.0.1/old.txt"), "old");
    write(&othercache.join("easybooks/0.0.1/old.txt"), "old");

    // The new package, supplied by the first host cache. The manifest version
    // is an integer envelope; the product version comes from plugin.json.
    let package = hostcache.join("easybooks").join(version);
    write(
        &package.join("runtime-manifest.json"),
        &fs::read_to_string("../plugin-metadata/runtime-manifest.json").unwrap(),
    );
    write(
        &package.join(".claude-plugin/plugin.json"),
        &format!("{{\"version\":\"{version}\"}}"),
    );
    let binary = package.join("bin").join(platform).join("easybooks");
    fs::create_dir_all(binary.parent().unwrap()).unwrap();
    fs::copy(assert_cmd::cargo::cargo_bin("easybooks"), &binary).unwrap();

    let roots = format!(
        "{}:{}",
        hostcache.join("easybooks").display(),
        othercache.join("easybooks").display()
    );
    let output = Command::new(&binary)
        .args(["doctor", "--repair-install"])
        .env("JACKYZHANG_APP_HOME", &home)
        .env("JZ_PLUGIN_HOST_CACHE_ROOTS", roots)
        .env("JZ_PLUGIN_UPDATE_CHECK", "off")
        .env("HOME", &user_home)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["ok"], true);
    assert_eq!(report["version"], version);
    assert_eq!(report["leftover"], serde_json::json!([]));
    assert!(report.get("previous_tree_preserved").is_none());

    let installed: Vec<_> = fs::read_dir(home.join("plugins/easybooks"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(installed, ["current"]);
    assert!(home
        .join("plugins/easybooks/current/runtime-manifest.json")
        .is_file());
    assert_eq!(
        fs::read_to_string(home.join("token/user.json")).unwrap(),
        "{\"token\":\"keep\"}"
    );
    assert!(home.join("easybooks/config.json").is_file());
    assert!(!hostcache.join("easybooks/0.0.1").exists());
    assert!(hostcache.join("easybooks").join(version).exists());
    assert!(othercache.join("easybooks/0.0.1").exists());
}
