//! Platform home paths. Same semantics as `jz-plugin-common::home`
//! (`platform_home`): `JACKYZHANG_APP_HOME` when non-empty, else
//! `$HOME/.jackyzhang.app`, else `%USERPROFILE%\.jackyzhang.app`.

use std::io;
use std::path::PathBuf;

pub const PLATFORM_HOME_ENV: &str = "JACKYZHANG_APP_HOME";

/// The platform home. Only product entry points call this; crate tests pass
/// temp paths instead.
pub fn platform_home() -> io::Result<PathBuf> {
    if let Ok(raw) = std::env::var(PLATFORM_HOME_ENV) {
        if !raw.is_empty() {
            return Ok(PathBuf::from(raw));
        }
    }
    home_dir()
        .map(|home| home.join(".jackyzhang.app"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no home directory"))
}

/// `<home>/plugins/<plugin_id>`: the canonical public package root.
pub fn install_root(plugin_id: &str) -> io::Result<PathBuf> {
    Ok(platform_home()?.join("plugins").join(plugin_id))
}

/// `<home>/<plugin_id>`: runtime state and caches, never package content.
pub fn runtime_dir(plugin_id: &str) -> io::Result<PathBuf> {
    Ok(platform_home()?.join(plugin_id))
}

pub(crate) fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
}
