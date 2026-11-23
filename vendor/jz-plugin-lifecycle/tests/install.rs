use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use jz_plugin_lifecycle::install::{
    fresh_install, platform_id, sweep, InstallError, InstallRequest,
};

const ID: &str = "demo";
const BIN: &str = "demo-cli";

fn bin_file() -> String {
    if cfg!(windows) {
        format!("{BIN}.exe")
    } else {
        BIN.to_string()
    }
}

fn write(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

/// A valid package directory with `version`.
fn package(dir: &Path, version: &str) -> PathBuf {
    write(
        &dir.join("runtime-manifest.json"),
        format!("{{\"version\":\"{version}\"}}").as_bytes(),
    );
    write(
        &dir.join("bin").join(platform_id()).join(bin_file()),
        version.as_bytes(),
    );
    write(&dir.join("skills/demo/SKILL.md"), b"skill");
    dir.to_path_buf()
}

fn request<'a>(
    home: &'a Path,
    package_root: &'a Path,
    stale: &'a [PathBuf],
    host_roots: &'a [PathBuf],
) -> InstallRequest<'a> {
    InstallRequest {
        plugin_id: ID,
        binary_name: BIN,
        package_root,
        stale_caches: stale,
        home,
        host_cache_roots: host_roots,
    }
}

fn names(dir: &Path) -> BTreeSet<String> {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn manifest_version(dir: &Path) -> String {
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("runtime-manifest.json")).unwrap()).unwrap();
    value["version"].as_str().unwrap().to_string()
}

struct World {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
}

impl World {
    fn new() -> World {
        let tmp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(tmp.path()).unwrap();
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        World {
            _tmp: tmp,
            root,
            home,
        }
    }
    fn install_root(&self) -> PathBuf {
        self.home.join("plugins").join(ID)
    }
}

#[test]
fn fresh_install_leaves_only_current_and_keeps_user_state() {
    let w = World::new();
    let plugins = w.install_root();
    package(&plugins.join("current"), "1.0.0");
    write(&plugins.join("current/OLD_MARKER"), b"old");
    package(&plugins.join("1.2.10"), "1.2.10");
    write(&plugins.join(".previous-0.1.0-x/file"), b"x");
    write(&plugins.join("outgoing/file"), b"x");
    write(&plugins.join("incoming/file"), b"x");
    write(&w.home.join("token/user.json"), b"{\"secret\":1}");
    write(&w.home.join(ID).join("config.json"), b"{\"k\":2}");
    let stale = w.root.join("caches/regen");
    write(&stale.join("blob"), b"cache");
    let stale_file = w.root.join("caches/one.json");
    write(&stale_file, b"cache");
    let keep_cache = w.root.join("caches/keep");
    write(&keep_cache, b"keep");

    let pkg = package(&w.root.join("pkg"), "2.0.0");
    let stale_list = [stale.clone(), stale_file.clone()];
    let report = fresh_install(&request(&w.home, &pkg, &stale_list, &[])).unwrap();

    assert_eq!(report.version, "2.0.0");
    assert!(report.leftover.is_empty(), "{:?}", report.leftover);
    assert!(report.pending_cleanup.is_empty());
    // 1.2.10, .previous-*, the old current (now outgoing) and the two declared caches.
    assert_eq!(report.removed, 5);
    assert_eq!(names(&plugins), set(&["current"]));
    assert_eq!(manifest_version(&plugins.join("current")), "2.0.0");
    assert!(!plugins.join("current/OLD_MARKER").exists());
    assert_eq!(report.install_home, plugins.join("current"));
    assert!(report.binary.is_file());
    assert_eq!(
        fs::read(w.home.join("token/user.json")).unwrap(),
        b"{\"secret\":1}"
    );
    assert_eq!(
        fs::read(w.home.join(ID).join("config.json")).unwrap(),
        b"{\"k\":2}"
    );
    assert!(!stale.exists() && !stale_file.exists());
    assert!(keep_cache.exists(), "only declared caches are removed");
}

#[test]
fn a_second_install_of_the_same_package_is_idempotent() {
    let w = World::new();
    let pkg = package(&w.root.join("pkg"), "2.0.0");
    fresh_install(&request(&w.home, &pkg, &[], &[])).unwrap();
    let again = fresh_install(&request(&w.home, &pkg, &[], &[])).unwrap();
    assert!(again.leftover.is_empty());
    assert_eq!(names(&w.install_root()), set(&["current"]));
}

#[test]
fn running_from_the_installed_copy_only_cleans_up_around_it() {
    let w = World::new();
    let plugins = w.install_root();
    let current = package(&plugins.join("current"), "2.0.0");
    write(&plugins.join("1.2.10/file"), b"x");
    write(&plugins.join(".previous-x/file"), b"x");
    let report = fresh_install(&request(&w.home, &current, &[], &[])).unwrap();
    assert_eq!(report.version, "2.0.0");
    assert_eq!(names(&plugins), set(&["current"]));
    assert!(plugins.join("current/skills/demo/SKILL.md").is_file());
}

#[test]
fn invalid_packages_leave_the_old_current_untouched() {
    let w = World::new();
    let plugins = w.install_root();
    package(&plugins.join("current"), "1.0.0");

    let no_manifest = w.root.join("no-manifest");
    write(
        &no_manifest.join("bin").join(platform_id()).join(bin_file()),
        b"x",
    );
    assert!(matches!(
        fresh_install(&request(&w.home, &no_manifest, &[], &[])),
        Err(InstallError::InvalidPackage(_))
    ));

    let no_binary = w.root.join("no-binary");
    write(
        &no_binary.join("runtime-manifest.json"),
        b"{\"version\":\"2.0.0\"}",
    );
    assert!(matches!(
        fresh_install(&request(&w.home, &no_binary, &[], &[])),
        Err(InstallError::InvalidPackage(_))
    ));

    let bad_version = package(&w.root.join("bad-version"), "2.0.0");
    write(
        &bad_version.join("runtime-manifest.json"),
        b"{\"version\":7}",
    );
    assert!(matches!(
        fresh_install(&request(&w.home, &bad_version, &[], &[])),
        Err(InstallError::InvalidPackage(_))
    ));

    assert_eq!(manifest_version(&plugins.join("current")), "1.0.0");
    assert_eq!(
        names(&plugins),
        set(&["current"]),
        "incoming and lock are cleaned up"
    );
}

#[test]
fn an_integer_format_envelope_takes_the_product_version_from_plugin_json() {
    let w = World::new();
    let plugins = w.install_root();
    package(&plugins.join("current"), "1.0.0");
    let pkg = package(&w.root.join("pkg"), "2.0.0");
    write(&pkg.join("runtime-manifest.json"), b"{\"version\":3}");
    write(
        &pkg.join(".claude-plugin/plugin.json"),
        b"{\"name\":\"x\",\"version\":\"2.0.0\"}",
    );
    let report =
        fresh_install(&request(&w.home, &pkg, &[], &[])).expect("envelope package installs");
    assert_eq!(report.version, "2.0.0");

    let plugin_version = package(&w.root.join("pkg2"), "3.0.0");
    write(
        &plugin_version.join("runtime-manifest.json"),
        b"{\"version\":4,\"plugin_version\":\"3.0.0\"}",
    );
    let report = fresh_install(&request(&w.home, &plugin_version, &[], &[]))
        .expect("plugin_version installs");
    assert_eq!(report.version, "3.0.0");
}

#[test]
fn a_package_carrying_a_token_is_refused() {
    let w = World::new();
    let plugins = w.install_root();
    package(&plugins.join("current"), "1.0.0");
    let pkg = package(&w.root.join("pkg"), "2.0.0");
    write(&pkg.join("token/user.json"), b"secret");
    assert_eq!(
        fresh_install(&request(&w.home, &pkg, &[], &[])),
        Err(InstallError::CredentialInPackage)
    );
    assert_eq!(manifest_version(&plugins.join("current")), "1.0.0");
    assert_eq!(names(&plugins), set(&["current"]));
}

#[test]
fn ids_and_binary_names_cannot_escape() {
    let w = World::new();
    let pkg = package(&w.root.join("pkg"), "2.0.0");
    for id in ["../x", "a/b", "", ".hidden", "a b", "..", "x\\y"] {
        let mut req = request(&w.home, &pkg, &[], &[]);
        req.plugin_id = id;
        assert!(
            matches!(fresh_install(&req), Err(InstallError::InvalidPackage(_))),
            "{id:?}"
        );
        assert!(sweep(&w.home, id).is_empty());
    }
    for bin in ["", "..", "a/b", "a\\b", "c:x"] {
        let mut req = request(&w.home, &pkg, &[], &[]);
        req.binary_name = bin;
        assert!(
            matches!(fresh_install(&req), Err(InstallError::InvalidPackage(_))),
            "{bin:?}"
        );
    }
    assert!(
        !w.home.join("plugins").exists(),
        "refusals happen before anything is created"
    );
}

#[test]
fn unsafe_stale_cache_paths_are_refused_before_any_change() {
    let w = World::new();
    let plugins = w.install_root();
    package(&plugins.join("current"), "1.0.0");
    write(&w.home.join("token/user.json"), b"secret");
    write(&w.home.join(ID).join("config.json"), b"cfg");
    write(&w.home.join("other/config.json"), b"cfg");
    let pkg = package(&w.root.join("pkg"), "2.0.0");
    let host = w.root.join("host").join(ID);
    fs::create_dir_all(host.join("0.1.0")).unwrap();
    let host_roots = [host.clone()];

    let bad: Vec<PathBuf> = vec![
        w.home.clone(),
        w.root.clone(),
        PathBuf::from("/"),
        w.home.join("token"),
        w.home.join("token/user.json"),
        w.home.join(ID),      // the whole runtime dir
        w.home.join("other"), // another plugin's runtime dir
        w.home.join("plugins"),
        plugins.clone(),
        plugins.join("current"),
        plugins.join("current/bin"),
        pkg.clone(),
        pkg.join("skills"),
        host.clone(),
        host.join("0.1.0"),
        w.home.join(ID).join("..").join("token"),
        PathBuf::from("relative/cache"),
        PathBuf::new(),
    ];
    for path in bad {
        let list = [path.clone()];
        let result = fresh_install(&request(&w.home, &pkg, &list, &host_roots));
        assert!(
            matches!(result, Err(InstallError::Io(_))),
            "{path:?} -> {result:?}"
        );
    }
    assert_eq!(manifest_version(&plugins.join("current")), "1.0.0");
    assert_eq!(fs::read(w.home.join("token/user.json")).unwrap(), b"secret");
    assert_eq!(
        fs::read(w.home.join(ID).join("config.json")).unwrap(),
        b"cfg"
    );
    assert!(host.join("0.1.0").is_dir());
}

#[test]
fn a_stale_cache_under_a_linked_parent_cannot_reach_the_token_dir() {
    #[cfg(unix)]
    {
        let w = World::new();
        write(&w.home.join("token/user.json"), b"secret");
        let pkg = package(&w.root.join("pkg"), "2.0.0");
        let caches = w.root.join("caches");
        fs::create_dir_all(&caches).unwrap();
        std::os::unix::fs::symlink(w.home.join("token"), caches.join("sneaky")).unwrap();
        let stale = [caches.join("sneaky").join("user.json")];
        assert!(matches!(
            fresh_install(&request(&w.home, &pkg, &stale, &[])),
            Err(InstallError::Io(_))
        ));
        assert_eq!(fs::read(w.home.join("token/user.json")).unwrap(), b"secret");
    }
}

#[test]
fn linked_current_is_converted_and_link_targets_are_never_entered() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let w = World::new();
        let plugins = w.install_root();
        package(&plugins.join("1.2.10"), "1.2.10");
        symlink("1.2.10", plugins.join("current")).unwrap();
        // Links inside the install root that point outside it.
        let outside = w.root.join("outside");
        write(&outside.join("precious.txt"), b"keep");
        symlink(&outside, plugins.join("escape")).unwrap();
        symlink(outside.join("precious.txt"), plugins.join("escape-file")).unwrap();

        let pkg = package(&w.root.join("pkg"), "2.0.0");
        let report = fresh_install(&request(&w.home, &pkg, &[], &[])).unwrap();
        assert!(report.leftover.is_empty());
        assert_eq!(names(&plugins), set(&["current"]));
        let meta = fs::symlink_metadata(plugins.join("current")).unwrap();
        assert!(
            meta.is_dir() && !meta.file_type().is_symlink(),
            "current is a real directory now"
        );
        assert_eq!(manifest_version(&plugins.join("current")), "2.0.0");
        assert_eq!(fs::read(outside.join("precious.txt")).unwrap(), b"keep");
    }
}

#[test]
fn a_current_link_pointing_outside_is_moved_not_followed() {
    #[cfg(unix)]
    {
        let w = World::new();
        let plugins = w.install_root();
        fs::create_dir_all(&plugins).unwrap();
        let outside = package(&w.root.join("outside"), "0.0.1");
        write(&outside.join("user-data.txt"), b"keep");
        std::os::unix::fs::symlink(&outside, plugins.join("current")).unwrap();
        let pkg = package(&w.root.join("pkg"), "2.0.0");
        fresh_install(&request(&w.home, &pkg, &[], &[])).unwrap();
        assert_eq!(fs::read(outside.join("user-data.txt")).unwrap(), b"keep");
        assert_eq!(manifest_version(&plugins.join("current")), "2.0.0");
    }
}

#[test]
fn a_linked_install_root_is_refused_and_not_swept() {
    #[cfg(unix)]
    {
        let w = World::new();
        let outside = w.root.join("outside");
        package(&outside.join("current"), "1.0.0");
        write(&outside.join("victim.txt"), b"keep");
        fs::create_dir_all(w.home.join("plugins")).unwrap();
        std::os::unix::fs::symlink(&outside, w.install_root()).unwrap();
        let pkg = package(&w.root.join("pkg"), "2.0.0");
        assert!(matches!(
            fresh_install(&request(&w.home, &pkg, &[], &[])),
            Err(InstallError::Io(_))
        ));
        assert!(sweep(&w.home, ID).is_empty());
        assert_eq!(fs::read(outside.join("victim.txt")).unwrap(), b"keep");

        // `plugins` itself linked.
        let w2 = World::new();
        let outside2 = w2.root.join("outside2");
        package(&outside2.join(ID).join("current"), "1.0.0");
        write(&outside2.join(ID).join("victim.txt"), b"keep");
        std::os::unix::fs::symlink(&outside2, w2.home.join("plugins")).unwrap();
        assert!(matches!(
            fresh_install(&request(&w2.home, &pkg, &[], &[])),
            Err(InstallError::Io(_))
        ));
        assert!(sweep(&w2.home, ID).is_empty());
        assert_eq!(
            fs::read(outside2.join(ID).join("victim.txt")).unwrap(),
            b"keep"
        );
    }
}

#[test]
fn links_inside_a_package_are_not_copied() {
    #[cfg(unix)]
    {
        let w = World::new();
        let pkg = package(&w.root.join("pkg"), "2.0.0");
        let outside = w.root.join("outside");
        write(&outside.join("secret.txt"), b"s");
        std::os::unix::fs::symlink(&outside, pkg.join("dirlink")).unwrap();
        std::os::unix::fs::symlink(outside.join("secret.txt"), pkg.join("filelink")).unwrap();
        // A token that is only a link is not a credential in the package, and is not copied.
        std::os::unix::fs::symlink(&outside, pkg.join("token")).unwrap();
        fresh_install(&request(&w.home, &pkg, &[], &[])).unwrap();
        let current = w.install_root().join("current");
        assert!(!current.join("dirlink").exists() && !current.join("filelink").exists());
        assert!(fs::symlink_metadata(current.join("token")).is_err());
        assert_eq!(fs::read(outside.join("secret.txt")).unwrap(), b"s");
    }
}

#[test]
fn a_package_that_contains_the_install_root_is_refused() {
    let w = World::new();
    let plugins = w.install_root();
    package(&plugins.join("current"), "1.0.0");
    // The platform home itself, or the plugins dir, used as a package root.
    for bad in [w.home.clone(), w.home.join("plugins"), plugins.clone()] {
        write(
            &bad.join("runtime-manifest.json"),
            b"{\"version\":\"9.0.0\"}",
        );
        let result = fresh_install(&request(&w.home, &bad, &[], &[]));
        assert!(
            matches!(result, Err(InstallError::InvalidPackage(_))),
            "{bad:?} -> {result:?}"
        );
    }
    // Scratch space inside the install root.
    let incoming = package(&plugins.join("incoming"), "9.0.0");
    assert!(matches!(
        fresh_install(&request(&w.home, &incoming, &[], &[])),
        Err(InstallError::InvalidPackage(_))
    ));
    assert_eq!(manifest_version(&plugins.join("current")), "1.0.0");
}

#[test]
fn host_cache_pruning_is_limited_to_the_supplying_host() {
    let w = World::new();
    let a = w.root.join("hostA").join(ID);
    let b = w.root.join("hostB").join(ID);
    for dir in ["0.9.0", "9.9.9", "1.0", "notes", "0.9.0-beta"] {
        write(&a.join(dir).join("f"), b"x");
    }
    write(&b.join("0.9.0/f"), b"x");
    let pkg = package(&a.join("2.0.0"), "2.0.0");
    let roots = [b.clone(), a.clone()];
    let report = fresh_install(&request(&w.home, &pkg, &[], &roots)).unwrap();
    assert!(report.leftover.is_empty());
    assert_eq!(
        names(&a),
        set(&["2.0.0", "9.9.9", "1.0", "notes", "0.9.0-beta"])
    );
    assert!(
        b.join("0.9.0/f").is_file(),
        "another host's cache is never touched"
    );
    assert!(pkg.join("runtime-manifest.json").is_file());
}

#[test]
fn host_cache_is_untouched_when_the_package_came_from_elsewhere() {
    let w = World::new();
    let a = w.root.join("hostA").join(ID);
    write(&a.join("0.9.0/f"), b"x");
    let pkg = package(&w.root.join("pkg"), "2.0.0");
    fresh_install(&request(&w.home, &pkg, &[], std::slice::from_ref(&a))).unwrap();
    assert!(a.join("0.9.0/f").is_file());
}

#[test]
fn host_cache_links_and_the_package_directory_are_never_deleted() {
    #[cfg(unix)]
    {
        let w = World::new();
        let a = w.root.join("hostA").join(ID);
        let outside = w.root.join("outside");
        write(&outside.join("keep"), b"k");
        fs::create_dir_all(&a).unwrap();
        std::os::unix::fs::symlink(&outside, a.join("0.5.0")).unwrap();
        // A lower-numbered directory that actually holds the package being installed.
        let pkg = package(&a.join("0.8.0"), "2.0.0");
        fresh_install(&request(&w.home, &pkg, &[], std::slice::from_ref(&a))).unwrap();
        assert!(outside.join("keep").is_file());
        assert!(a.join("0.5.0").exists());
        assert!(pkg.join("runtime-manifest.json").is_file());
    }
}

#[test]
fn sweep_leaves_a_linked_current_and_a_missing_current_alone() {
    #[cfg(unix)]
    {
        let w = World::new();
        let plugins = w.install_root();
        package(&plugins.join("1.2.10"), "1.2.10");
        std::os::unix::fs::symlink("1.2.10", plugins.join("current")).unwrap();
        write(&plugins.join("outgoing/f"), b"x");
        assert!(sweep(&w.home, ID).is_empty());
        assert_eq!(names(&plugins), set(&["1.2.10", "current", "outgoing"]));
    }
    let w = World::new();
    let plugins = w.install_root();
    write(&plugins.join("incoming/f"), b"x");
    assert!(sweep(&w.home, ID).is_empty());
    assert_eq!(
        names(&plugins),
        set(&["incoming"]),
        "mid-switch state is not swept"
    );
    assert!(sweep(&w.root.join("nowhere"), ID).is_empty());
}

#[test]
fn sweep_removes_everything_but_current() {
    let w = World::new();
    let plugins = w.install_root();
    package(&plugins.join("current"), "2.0.0");
    write(&plugins.join("outgoing/f"), b"x");
    write(&plugins.join(".previous-1/f"), b"x");
    write(&plugins.join("stray.txt"), b"x");
    write(&w.home.join("token/user.json"), b"t");
    assert!(sweep(&w.home, ID).is_empty());
    assert_eq!(names(&plugins), set(&["current"]));
    assert!(plugins.join("current/runtime-manifest.json").is_file());
    assert!(w.home.join("token/user.json").is_file());
    // Nothing left to do: no lock churn, still clean.
    assert!(sweep(&w.home, ID).is_empty());
    assert_eq!(names(&plugins), set(&["current"]));
}

#[test]
fn sweep_does_not_remove_links_targets() {
    #[cfg(unix)]
    {
        let w = World::new();
        let plugins = w.install_root();
        package(&plugins.join("current"), "2.0.0");
        let outside = w.root.join("outside");
        write(&outside.join("keep"), b"k");
        std::os::unix::fs::symlink(&outside, plugins.join("loop")).unwrap();
        assert!(sweep(&w.home, ID).is_empty());
        assert_eq!(names(&plugins), set(&["current"]));
        assert!(outside.join("keep").is_file());
    }
}

#[test]
fn a_fresh_lock_blocks_install_and_sweep_and_survives() {
    let w = World::new();
    let plugins = w.install_root();
    package(&plugins.join("current"), "1.0.0");
    write(&plugins.join(".install.lock"), b"123");
    write(&plugins.join("outgoing/f"), b"x");
    let pkg = package(&w.root.join("pkg"), "2.0.0");
    assert_eq!(
        fresh_install(&request(&w.home, &pkg, &[], &[])),
        Err(InstallError::Busy)
    );
    assert!(sweep(&w.home, ID).is_empty());
    assert_eq!(
        names(&plugins),
        set(&[".install.lock", "current", "outgoing"])
    );
    assert_eq!(manifest_version(&plugins.join("current")), "1.0.0");
}

#[test]
fn a_stale_lock_is_replaced_and_removed() {
    let w = World::new();
    let plugins = w.install_root();
    package(&plugins.join("current"), "1.0.0");
    let lock = plugins.join(".install.lock");
    write(&lock, b"123");
    fs::OpenOptions::new()
        .write(true)
        .open(&lock)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(11 * 60))
        .unwrap();
    let pkg = package(&w.root.join("pkg"), "2.0.0");
    fresh_install(&request(&w.home, &pkg, &[], &[])).unwrap();
    assert_eq!(names(&plugins), set(&["current"]));
    assert_eq!(manifest_version(&plugins.join("current")), "2.0.0");
}

#[test]
fn concurrent_installs_and_sweeps_neither_fail_nor_corrupt() {
    let w = World::new();
    let plugins = w.install_root();
    package(&plugins.join("current"), "1.0.0");
    let pkgs: Vec<PathBuf> = ["2.0.0", "3.0.0"]
        .iter()
        .map(|v| package(&w.root.join(format!("pkg-{v}")), v))
        .collect();
    let home = w.home.clone();
    std::thread::scope(|scope| {
        for pkg in &pkgs {
            let home = &home;
            scope.spawn(move || {
                for _ in 0..8 {
                    match fresh_install(&request(home, pkg, &[], &[])) {
                        Ok(report) => assert!(report.leftover.is_empty(), "{:?}", report.leftover),
                        Err(InstallError::Busy) => {}
                        Err(other) => panic!("unexpected install error: {other:?}"),
                    }
                }
            });
        }
        let home = &home;
        scope.spawn(move || {
            for _ in 0..40 {
                assert!(sweep(home, ID).is_empty());
            }
        });
    });
    let current = plugins.join("current");
    let version = manifest_version(&current);
    assert!(
        ["1.0.0", "2.0.0", "3.0.0"].contains(&version.as_str()),
        "{version}"
    );
    assert!(current
        .join("bin")
        .join(platform_id())
        .join(bin_file())
        .is_file());
    assert!(sweep(&w.home, ID).is_empty());
    assert_eq!(names(&plugins), set(&["current"]));
}
