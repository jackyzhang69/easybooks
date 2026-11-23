//! Fresh install: write a complete new tree, switch `current` to it, then
//! delete everything else under `plugins/<plugin_id>/`, the product's
//! declared stale caches and older version directories in the host plugin
//! cache that supplied the package. No backup is kept.
//!
//! Safety rules the code below enforces:
//! * every existence check is `symlink_metadata`; a link (Unix symlink, or on
//!   Windows any reparse point: symlink, junction, mount point) is removed as
//!   a link and its target is never entered;
//! * `plugins/` and `plugins/<plugin_id>/` must be real directories, so a
//!   linked root cannot redirect deletion elsewhere;
//! * `plugin_id`, `binary_name` and host-cache version names are validated as
//!   single path components before they are joined to any path;
//! * declared stale-cache paths are validated before anything is changed and
//!   can never name the platform home, a direct child of it (token, runtime
//!   or another plugin's directory), `plugins/`, the package or a host cache;
//! * install and sweep exclude each other through `plugins/<id>/.install.lock`
//!   (non-blocking `create_new`, never waited on, so there is nothing to
//!   deadlock on);
//! * nothing here panics: all failures are values.

use std::fmt;
use std::fs::{self, Metadata, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::update::version_less;

const LOCK_NAME: &str = ".install.lock";
const LOCK_STALE_AFTER: Duration = Duration::from_secs(600);
const RUNNING_PREFIX: &str = ".running-";
const STALE_OUTGOING_PREFIX: &str = ".outgoing-stale-";
const TOP_LEVEL_SKIPPED: [&str; 3] = ["incoming", "outgoing", "current"];
const MAX_DEPTH: usize = 64;
const MANIFEST_MAX_BYTES: u64 = 1 << 20;

pub struct InstallRequest<'a> {
    pub plugin_id: &'a str,
    pub binary_name: &'a str,
    pub package_root: &'a Path,
    pub stale_caches: &'a [PathBuf],
    pub home: &'a Path,
    pub host_cache_roots: &'a [PathBuf],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallReport {
    pub version: String,
    /// `<home>/plugins/<plugin_id>/current`.
    pub install_home: PathBuf,
    pub binary: PathBuf,
    /// Entries removed in steps 5-7 (top-level entries, declared caches, old host versions).
    pub removed: usize,
    /// Paths that could not be removed and are not the running image.
    pub leftover: Vec<PathBuf>,
    /// Paths that could not be removed because they are, or contain, the running image.
    pub pending_cleanup: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallError {
    NoHome(String),
    Busy,
    InvalidPackage(String),
    CredentialInPackage,
    SwitchFailed(String),
    Io(String),
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstallError::NoHome(msg) => write!(f, "no platform home: {msg}"),
            InstallError::Busy => f.write_str("another install is in progress"),
            InstallError::InvalidPackage(msg) => write!(f, "invalid package: {msg}"),
            InstallError::CredentialInPackage => f.write_str("package contains a credential"),
            InstallError::SwitchFailed(msg) => write!(f, "could not switch current: {msg}"),
            InstallError::Io(msg) => write!(f, "install i/o error: {msg}"),
        }
    }
}

impl std::error::Error for InstallError {}

fn io_err(context: &str, error: &io::Error) -> InstallError {
    InstallError::Io(format!("{context}: {error}"))
}

pub fn platform_id() -> &'static str {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "darwin-arm64"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "darwin-x64"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "linux-x64"
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "win32-x64"
    } else {
        "unknown"
    }
}

fn binary_file_name(binary_name: &str) -> String {
    if cfg!(windows) && !binary_name.ends_with(".exe") {
        format!("{binary_name}.exe")
    } else {
        binary_name.to_string()
    }
}

/// The package directory the running executable belongs to: the nearest
/// parent (at most five levels) holding `runtime-manifest.json`.
pub fn package_root_from_exe() -> Result<PathBuf, InstallError> {
    let exe = std::env::current_exe()
        .and_then(fs::canonicalize)
        .map_err(|error| io_err("resolving the running executable", &error))?;
    let mut dir = exe.parent();
    for _ in 0..5 {
        let Some(candidate) = dir else { break };
        if is_regular_file(&candidate.join("runtime-manifest.json")) {
            return Ok(candidate.to_path_buf());
        }
        dir = candidate.parent();
    }
    Err(InstallError::InvalidPackage(
        "no runtime-manifest.json above the running executable".into(),
    ))
}

/// Host plugin-cache bases for `plugin_id`: `JZ_PLUGIN_HOST_CACHE_ROOTS`
/// entries when set, else the Claude Code and Codex defaults. Only product
/// entry points call this; tests pass explicit roots.
pub fn default_host_cache_roots(plugin_id: &str) -> Vec<PathBuf> {
    if let Some(raw) = std::env::var_os("JZ_PLUGIN_HOST_CACHE_ROOTS") {
        if !raw.is_empty() {
            return std::env::split_paths(&raw)
                .filter(|path| !path.as_os_str().is_empty())
                .collect();
        }
    }
    let home = crate::home::home_dir();
    let base = |env_name: &str, fallback_dir: &str| -> Option<PathBuf> {
        let configured = std::env::var_os(env_name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        configured
            .or_else(|| home.as_ref().map(|home| home.join(fallback_dir)))
            .map(|base| base.join("plugins/cache/jacky-plugins").join(plugin_id))
    };
    [
        base("CLAUDE_CONFIG_DIR", ".claude"),
        base("CODEX_HOME", ".codex"),
    ]
    .into_iter()
    .flatten()
    .collect()
}

// ---------------------------------------------------------------- path helpers

fn lstat(path: &Path) -> io::Result<Option<Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(meta) => Ok(Some(meta)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// A Unix symlink, or on Windows any reparse point.
fn is_link(meta: &Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

fn is_real_dir(path: &Path) -> bool {
    matches!(lstat(path), Ok(Some(meta)) if meta.is_dir() && !is_link(&meta))
}

fn is_regular_file(path: &Path) -> bool {
    matches!(lstat(path), Ok(Some(meta)) if meta.is_file() && !is_link(&meta))
}

/// Existing in any form, or unreadable (treated as existing: never assume absence).
fn exists_in_any_form(path: &Path) -> bool {
    !matches!(lstat(path), Ok(None))
}

fn canon(path: &Path) -> Option<PathBuf> {
    fs::canonicalize(path).ok()
}

fn running_exe() -> Option<PathBuf> {
    std::env::current_exe().ok().and_then(|exe| canon(&exe))
}

fn valid_plugin_id(id: &str) -> bool {
    let mut chars = id.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_alphanumeric())
        && id.len() <= 64
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn valid_binary_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':', '\0'])
        && Path::new(name).components().count() == 1
}

/// `N.N.N` exactly, all numeric.
fn is_triple_version(name: &str) -> bool {
    let parts: Vec<&str> = name.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
}

fn read_small_json(path: &Path) -> Option<serde_json::Value> {
    let meta = lstat(path).ok()??;
    if !meta.is_file() || is_link(&meta) || meta.len() > MANIFEST_MAX_BYTES {
        return None;
    }
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn triple_at(value: &serde_json::Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)?
        .as_str()
        .filter(|v| is_triple_version(v))
        .map(str::to_owned)
}

/// The package's product version (`N.N.N`). Products use `runtime-manifest.json`
/// `version` either as the product version or as an integer format envelope
/// (formbro, easybooks, anychat, anydoc: `3`/`4`/`1`), so the first product
/// version found wins: manifest `version`, `plugin_version`, `binary.version`,
/// then `.claude-plugin/plugin.json` `version` — the same fields the official
/// publisher binds to the release identity.
pub(crate) fn package_version(dir: &Path) -> Option<String> {
    let manifest = read_small_json(&dir.join("runtime-manifest.json"))?;
    ["/version", "/plugin_version", "/binary/version"]
        .iter()
        .find_map(|pointer| triple_at(&manifest, pointer))
        .or_else(|| {
            let plugin = read_small_json(&dir.join(".claude-plugin").join("plugin.json"))?;
            triple_at(&plugin, "/version")
        })
}

fn validate_package(dir: &Path, binary_name: &str) -> Result<String, InstallError> {
    let version = package_version(dir).ok_or_else(|| {
        InstallError::InvalidPackage(
            "runtime-manifest.json (or .claude-plugin/plugin.json) must carry an N.N.N product version".into(),
        )
    })?;
    let binary = dir
        .join("bin")
        .join(platform_id())
        .join(binary_file_name(binary_name));
    if !is_regular_file(&binary) {
        return Err(InstallError::InvalidPackage(format!(
            "bin/{}/{} is missing",
            platform_id(),
            binary_file_name(binary_name)
        )));
    }
    Ok(version)
}

// ------------------------------------------------------------------- deletion

fn remove_link(path: &Path, meta: &Metadata) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
        if meta.file_attributes() & FILE_ATTRIBUTE_DIRECTORY != 0 {
            return fs::remove_dir(path);
        }
    }
    let _ = meta;
    fs::remove_file(path)
}

#[cfg(windows)]
fn clear_readonly(path: &Path) {
    if let Ok(meta) = fs::symlink_metadata(path) {
        let mut permissions = meta.permissions();
        if permissions.readonly() {
            permissions.set_readonly(false);
            let _ = fs::set_permissions(path, permissions);
        }
    }
}

#[cfg(not(windows))]
fn clear_readonly(_path: &Path) {}

fn remove_file_retry(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if cfg!(windows) && error.kind() == io::ErrorKind::PermissionDenied => {
            clear_readonly(path);
            fs::remove_file(path)
        }
        other => other,
    }
}

/// Remove one entry without ever following a link.
fn remove_entry(path: &Path) -> io::Result<()> {
    remove_entry_at(path, 0)
}

fn remove_entry_at(path: &Path, depth: usize) -> io::Result<()> {
    let Some(meta) = lstat(path)? else {
        return Ok(());
    };
    if is_link(&meta) {
        return remove_link(path, &meta);
    }
    if !meta.is_dir() {
        return remove_file_retry(path);
    }
    if depth >= MAX_DEPTH {
        return Err(io::Error::other("directory tree too deep"));
    }
    let mut first_error = None;
    for entry in fs::read_dir(path)? {
        match entry {
            Ok(entry) => {
                if let Err(error) = remove_entry_at(&entry.path(), depth + 1) {
                    first_error.get_or_insert(error);
                }
            }
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => fs::remove_dir(path),
    }
}

#[derive(Default)]
struct Outcome {
    removed: usize,
    leftover: Vec<PathBuf>,
    pending: Vec<PathBuf>,
}

impl Outcome {
    /// Delete `path` (missing = success). A failure is `pending` when the path
    /// is a `.running-*` file or contains the running image, else `leftover`.
    fn delete(&mut self, path: &Path, exe: Option<&Path>) {
        let meta = match lstat(path) {
            Ok(None) => return,
            Ok(Some(meta)) => meta,
            Err(_) => {
                self.leftover.push(path.to_path_buf());
                return;
            }
        };
        let running_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(RUNNING_PREFIX));
        let holds_exe = !is_link(&meta)
            && matches!((exe, canon(path)), (Some(exe), Some(real)) if exe.starts_with(&real));
        match remove_entry(path) {
            Ok(()) => self.removed += 1,
            Err(_) if running_name || holds_exe => self.pending.push(path.to_path_buf()),
            Err(_) => self.leftover.push(path.to_path_buf()),
        }
    }

    fn finish(mut self) -> (usize, Vec<PathBuf>, Vec<PathBuf>) {
        self.pending.retain(|path| exists_in_any_form(path));
        self.pending.sort();
        self.pending.dedup();
        self.leftover.sort();
        self.leftover.dedup();
        (self.removed, self.leftover, self.pending)
    }
}

/// Delete every entry of `root` except `current` and the install lock.
fn delete_all_but_current(root: &Path, exe: Option<&Path>, outcome: &mut Outcome) {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => {
            outcome.leftover.push(root.to_path_buf());
            return;
        }
    };
    for entry in entries {
        match entry {
            Ok(entry) => {
                let name = entry.file_name();
                if name == "current" || name == LOCK_NAME {
                    continue;
                }
                outcome.delete(&entry.path(), exe);
            }
            Err(_) => outcome.leftover.push(root.to_path_buf()),
        }
    }
}

// -------------------------------------------------------------------- locking

struct InstallLock {
    path: PathBuf,
}

impl Drop for InstallLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn lock_is_stale(meta: &Metadata) -> bool {
    match meta.modified() {
        Ok(modified) => match SystemTime::now().duration_since(modified) {
            Ok(age) => age >= LOCK_STALE_AFTER,
            // A modification time in the future is clock skew; far ahead is stale.
            Err(skew) => skew.duration() > LOCK_STALE_AFTER,
        },
        Err(_) => false,
    }
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("{}-{nanos}", std::process::id())
}

/// Non-blocking: a held lock is `Busy`, never waited on. A lock older than ten
/// minutes belongs to a crashed install and is replaced.
fn acquire_lock(root: &Path) -> Result<InstallLock, InstallError> {
    let path = root.join(LOCK_NAME);
    for _attempt in 0..2 {
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                let _ = writeln!(file, "{}", std::process::id());
                return Ok(InstallLock { path });
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let Ok(Some(meta)) = lstat(&path) else {
                    // Vanished between create_new and lstat: retry once.
                    continue;
                };
                if !lock_is_stale(&meta) {
                    return Err(InstallError::Busy);
                }
                // Move the stale lock aside atomically, then confirm what we moved
                // was really stale (another process may have replaced it first).
                let aside = root.join(format!("{LOCK_NAME}.stale-{}", unique_suffix()));
                if fs::rename(&path, &aside).is_err() {
                    return Err(InstallError::Busy);
                }
                let moved_was_stale =
                    matches!(lstat(&aside), Ok(Some(moved)) if lock_is_stale(&moved));
                if !moved_was_stale {
                    // We displaced a live lock: put it back if nobody took its place.
                    let _ = fs::hard_link(&aside, &path);
                    let _ = fs::remove_file(&aside);
                    return Err(InstallError::Busy);
                }
                let _ = remove_entry(&aside);
            }
            Err(error) => return Err(io_err("taking the install lock", &error)),
        }
    }
    Err(InstallError::Busy)
}

// ------------------------------------------------------------------- guards

/// `plugins/` and `plugins/<id>/` must be real directories; created when absent.
fn ensure_roots(home: &Path, plugins: &Path, root: &Path) -> Result<(), InstallError> {
    fs::create_dir_all(home).map_err(|error| io_err("creating the platform home", &error))?;
    for dir in [plugins, root] {
        match lstat(dir) {
            Ok(None) => {
                fs::create_dir(dir).map_err(|error| io_err("creating the install root", &error))?
            }
            Ok(Some(_)) => {}
            Err(error) => return Err(io_err("inspecting the install root", &error)),
        }
        if !is_real_dir(dir) {
            return Err(InstallError::Io(format!(
                "{} must be a real directory, not a link",
                dir.display()
            )));
        }
    }
    Ok(())
}

/// Absolute path forms to test a candidate against: lexical, and the
/// canonical parent joined to its file name (so a linked parent cannot hide
/// the real location).
fn path_forms(path: &Path) -> Vec<PathBuf> {
    let mut forms = vec![path.to_path_buf()];
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
        if let Some(real_parent) = canon(parent) {
            let resolved = real_parent.join(name);
            if resolved != path {
                forms.push(resolved);
            }
        }
    }
    forms
}

fn stale_cache_is_safe(
    candidate: &Path,
    home: &Path,
    package: &Path,
    host_roots: &[PathBuf],
) -> bool {
    if !candidate.is_absolute()
        || candidate
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        || !matches!(
            candidate.components().next_back(),
            Some(Component::Normal(_))
        )
    {
        return false;
    }
    let mut homes = vec![home.to_path_buf()];
    homes.extend(canon(home));
    let mut packages = vec![package.to_path_buf()];
    packages.extend(canon(package));
    let mut bases: Vec<PathBuf> = host_roots.to_vec();
    bases.extend(host_roots.iter().filter_map(|base| canon(base)));

    for form in path_forms(candidate) {
        for home in &homes {
            // The home itself or an ancestor of it; a direct child of it (token,
            // runtime dir, another plugin's dir); the whole plugins/ tree; the token dir.
            if home.starts_with(&form)
                || form.parent() == Some(home.as_path())
                || form.starts_with(home.join("plugins"))
                || form.starts_with(home.join("token"))
            {
                return false;
            }
        }
        for package in &packages {
            if package.starts_with(&form) || form.starts_with(package) {
                return false;
            }
        }
        for base in &bases {
            if base.starts_with(&form) || form.starts_with(base) {
                return false;
            }
        }
    }
    true
}

// ------------------------------------------------------------------- copying

fn copy_tree(src: &Path, dst: &Path, top_level: bool, depth: usize) -> io::Result<()> {
    if depth >= MAX_DEPTH {
        return Err(io::Error::other("package tree too deep"));
    }
    fs::create_dir(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        if top_level
            && name
                .to_str()
                .is_some_and(|n| TOP_LEVEL_SKIPPED.contains(&n))
        {
            continue;
        }
        let meta = fs::symlink_metadata(entry.path())?;
        if is_link(&meta) {
            continue;
        }
        let target = dst.join(&name);
        if meta.is_dir() {
            copy_tree(&entry.path(), &target, false, depth + 1)?;
        } else if meta.is_file() {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

// ----------------------------------------------------------- running image

/// Windows only: the running image is renamed out of `current` so the old tree
/// can move. Restored when the switch fails.
struct MovedImage {
    from: PathBuf,
    to: PathBuf,
}

impl MovedImage {
    fn restore(self) {
        let _ = fs::rename(&self.to, &self.from);
    }
}

fn move_running_image(root: &Path, current: &Path, exe: Option<&Path>) -> Option<MovedImage> {
    if !cfg!(windows) {
        return None;
    }
    let exe = exe?;
    let current_real = canon(current)?;
    if !exe.starts_with(&current_real) {
        return None;
    }
    let name = exe.file_name()?.to_str()?;
    let to = root.join(format!("{RUNNING_PREFIX}{}-{name}", std::process::id()));
    fs::rename(exe, &to).ok()?;
    Some(MovedImage {
        from: exe.to_path_buf(),
        to,
    })
}

/// Clear a leftover `outgoing`; when it cannot be removed (a live image
/// inside it), move it aside so the switch can proceed and a later sweep retries.
fn clear_outgoing(root: &Path, outgoing: &Path) -> Result<(), InstallError> {
    if !exists_in_any_form(outgoing) {
        return Ok(());
    }
    if remove_entry(outgoing).is_ok() {
        return Ok(());
    }
    let aside = root.join(format!("{STALE_OUTGOING_PREFIX}{}", unique_suffix()));
    fs::rename(outgoing, &aside)
        .map_err(|error| io_err("clearing a leftover outgoing tree", &error))
}

// ------------------------------------------------------------------- install

pub fn fresh_install(req: &InstallRequest) -> Result<InstallReport, InstallError> {
    if !valid_plugin_id(req.plugin_id) {
        return Err(InstallError::InvalidPackage("invalid plugin id".into()));
    }
    if !valid_binary_name(req.binary_name) {
        return Err(InstallError::InvalidPackage("invalid binary name".into()));
    }
    let package = canon(req.package_root)
        .ok_or_else(|| InstallError::InvalidPackage("package root is not readable".into()))?;
    for stale in req.stale_caches {
        if !stale_cache_is_safe(stale, req.home, req.package_root, req.host_cache_roots) {
            return Err(InstallError::Io(format!(
                "refused unsafe stale cache path {}",
                stale.display()
            )));
        }
    }

    let plugins = req.home.join("plugins");
    let root = plugins.join(req.plugin_id);
    ensure_roots(req.home, &plugins, &root)?;
    let _lock = acquire_lock(&root)?;

    let root_real = canon(&root).ok_or_else(|| InstallError::Io("install root vanished".into()))?;
    if root_real.starts_with(&package) {
        return Err(InstallError::InvalidPackage(
            "package root contains the install root".into(),
        ));
    }
    if let Ok(relative) = package.strip_prefix(&root_real) {
        let first = relative.components().next();
        let scratch = matches!(first, Some(Component::Normal(name)) if name.to_str().is_some_and(|n| {
            n == "incoming" || n == "outgoing" || n.starts_with('.')
        }));
        if scratch {
            return Err(InstallError::InvalidPackage(
                "package root lives in install scratch space".into(),
            ));
        }
    }

    let exe = running_exe();
    let current = root.join("current");
    let in_place = is_real_dir(&current) && canon(&current).as_deref() == Some(package.as_path());

    let version = if in_place {
        validate_package(&current, req.binary_name)?
    } else {
        let incoming = root.join("incoming");
        let outgoing = root.join("outgoing");
        let incoming_fail = |error: InstallError| {
            let _ = remove_entry(&incoming);
            error
        };
        remove_entry(&incoming)
            .map_err(|error| io_err("clearing a leftover incoming tree", &error))?;
        if let Err(error) = copy_tree(&package, &incoming, true, 0) {
            return Err(incoming_fail(io_err("copying the package", &error)));
        }
        if exists_in_any_form(&incoming.join("token")) {
            return Err(incoming_fail(InstallError::CredentialInPackage));
        }
        let version = validate_package(&incoming, req.binary_name).map_err(incoming_fail)?;

        let had_current = exists_in_any_form(&current);
        let mut moved = None;
        if had_current {
            if let Err(error) = clear_outgoing(&root, &outgoing) {
                return Err(incoming_fail(error));
            }
            moved = move_running_image(&root, &current, exe.as_deref());
            if let Err(error) = fs::rename(&current, &outgoing) {
                if let Some(moved) = moved {
                    moved.restore();
                }
                return Err(incoming_fail(InstallError::SwitchFailed(error.to_string())));
            }
        }
        if let Err(error) = fs::rename(&incoming, &current) {
            let mut detail = error.to_string();
            if had_current {
                match fs::rename(&outgoing, &current) {
                    Ok(()) => {
                        if let Some(moved) = moved {
                            moved.restore();
                        }
                    }
                    Err(rollback) => {
                        detail = format!("{detail}; rolling back also failed: {rollback}");
                    }
                }
            }
            return Err(incoming_fail(InstallError::SwitchFailed(detail)));
        }
        // On Windows the old running image now lives only as `.running-*`; step 5 reports it.
        version
    };

    let mut outcome = Outcome::default();
    delete_all_but_current(&root, exe.as_deref(), &mut outcome);
    for stale in req.stale_caches {
        outcome.delete(stale, exe.as_deref());
    }
    prune_host_cache(
        req,
        &package,
        &version,
        &root_real,
        exe.as_deref(),
        &mut outcome,
    );

    let (removed, leftover, pending_cleanup) = outcome.finish();
    Ok(InstallReport {
        version,
        binary: current
            .join("bin")
            .join(platform_id())
            .join(binary_file_name(req.binary_name)),
        install_home: current,
        removed,
        leftover,
        pending_cleanup,
    })
}

/// Delete older `N.N.N` directories in the one host cache base that supplied
/// `package`. Never another base, never equal or higher versions, never
/// non-version names, never links, never the running image or the package.
fn prune_host_cache(
    req: &InstallRequest,
    package: &Path,
    version: &str,
    root_real: &Path,
    exe: Option<&Path>,
    outcome: &mut Outcome,
) {
    let home_real = canon(req.home);
    for base in req.host_cache_roots {
        let Some(base_real) = canon(base) else {
            continue;
        };
        if !package.starts_with(&base_real) || package == base_real {
            continue;
        }
        let overlaps_platform = base_real.starts_with(root_real)
            || root_real.starts_with(&base_real)
            || home_real
                .as_ref()
                .is_some_and(|home| home.starts_with(&base_real));
        if overlaps_platform {
            return;
        }
        let Ok(entries) = fs::read_dir(&base_real) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if !is_triple_version(name) || !version_less(name, version) {
                continue;
            }
            let path = entry.path();
            if !is_real_dir(&path) {
                continue;
            }
            let Some(real) = canon(&path) else { continue };
            if package.starts_with(&real) || exe.is_some_and(|exe| exe.starts_with(&real)) {
                continue;
            }
            outcome.delete(&path, exe);
        }
        return;
    }
}

/// Remove everything under `plugins/<plugin_id>/` except `current` when
/// `current` is a real directory and no install holds the lock. Returns the
/// paths that could not be removed. Does nothing for a missing or linked
/// `current` (an install is mid-switch, or the legacy linked layout is still
/// live; only `fresh_install` converts it).
pub fn sweep(home: &Path, plugin_id: &str) -> Vec<PathBuf> {
    if !valid_plugin_id(plugin_id) {
        return Vec::new();
    }
    let plugins = home.join("plugins");
    let root = plugins.join(plugin_id);
    if !is_real_dir(&plugins) || !is_real_dir(&root) || !is_real_dir(&root.join("current")) {
        return Vec::new();
    }
    let has_other_entries = match fs::read_dir(&root) {
        Ok(entries) => entries.flatten().any(|entry| {
            let name = entry.file_name();
            name != "current" && name != LOCK_NAME
        }),
        Err(_) => false,
    };
    if !has_other_entries {
        return Vec::new();
    }
    let Ok(_lock) = acquire_lock(&root) else {
        return Vec::new();
    };
    if !is_real_dir(&root.join("current")) {
        return Vec::new();
    }
    let exe = running_exe();
    let mut outcome = Outcome::default();
    delete_all_but_current(&root, exe.as_deref(), &mut outcome);
    let (_removed, mut leftover, pending) = outcome.finish();
    leftover.extend(pending);
    leftover
}
