//! Install and update lifecycle glue around `jz_plugin_lifecycle`: the
//! `doctor --repair-install` handler and the post-command update step.

use anyhow::Result;
use jz_plugin_lifecycle::{home, install};
use serde_json::{json, Value};
use std::path::PathBuf;

const PLUGIN_ID: &str = "easybooks";
const BINARY_NAME: &str = "easybooks";

/// Regenerable caches that install and update delete. EasyBooks keeps none:
/// everything under `~/.jackyzhang.app/easybooks/` is user state.
const STALE_CACHES: &[PathBuf] = &[];

fn paths_json(paths: &[PathBuf]) -> Vec<String> {
    paths.iter().map(|p| p.display().to_string()).collect()
}

fn error_code(error: &install::InstallError) -> &'static str {
    match error {
        install::InstallError::NoHome(_) => "no_home",
        install::InstallError::Busy => "install_busy",
        install::InstallError::InvalidPackage(_) => "invalid_package",
        install::InstallError::CredentialInPackage => "credential_in_package",
        install::InstallError::SwitchFailed(_) => "switch_failed",
        install::InstallError::Io(_) => "io_error",
    }
}

/// Outcome JSON for `doctor --repair-install` and whether it counts as success.
fn repair_install_payload(
    result: Result<install::InstallReport, install::InstallError>,
) -> (Value, bool) {
    match result {
        Ok(report) => {
            let ok = report.leftover.is_empty();
            let mut payload = json!({
                "ok": ok,
                "command": "doctor",
                "repair_install": true,
                "product": PLUGIN_ID,
                "version": report.version,
                "install_home": report.install_home.display().to_string(),
                "removed": report.removed,
                "leftover": paths_json(&report.leftover),
                "pending_cleanup": paths_json(&report.pending_cleanup),
            });
            if !ok {
                payload["code"] = json!("old_tree_not_removed");
            }
            (payload, ok)
        }
        Err(error) => (
            json!({
                "ok": false,
                "command": "doctor",
                "repair_install": true,
                "product": PLUGIN_ID,
                "code": error_code(&error),
                "message": error.to_string(),
            }),
            false,
        ),
    }
}

/// `easybooks doctor --repair-install`: make the package this binary belongs
/// to the only installed copy. Needs no login and no config. Exits 1 after
/// printing the JSON on failure.
pub fn repair_install() -> Result<()> {
    let result = install::package_root_from_exe().and_then(|package_root| {
        let platform_home =
            home::platform_home().map_err(|e| install::InstallError::NoHome(e.to_string()))?;
        let host_cache_roots = install::default_host_cache_roots(PLUGIN_ID);
        install::fresh_install(&install::InstallRequest {
            plugin_id: PLUGIN_ID,
            binary_name: BINARY_NAME,
            package_root: &package_root,
            stale_caches: STALE_CACHES,
            home: &platform_home,
            host_cache_roots: &host_cache_roots,
        })
    });
    let (payload, ok) = repair_install_payload(result);
    crate::output::print_json(&payload)?;
    if !ok {
        std::process::exit(1);
    }
    Ok(())
}

/// Daily update notice and host-install sync; never changes a command's result.
pub fn after_command() {
    jz_plugin_lifecycle::update::after_command(
        PLUGIN_ID,
        env!("CARGO_PKG_VERSION"),
        BINARY_NAME,
        STALE_CACHES,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(leftover: Vec<PathBuf>) -> install::InstallReport {
        install::InstallReport {
            version: "0.5.28".into(),
            install_home: PathBuf::from("/h/plugins/easybooks/current"),
            binary: PathBuf::from("/h/plugins/easybooks/current/bin/linux-x64/easybooks"),
            removed: 2,
            leftover,
            pending_cleanup: vec![],
        }
    }

    #[test]
    fn leftover_makes_repair_install_fail_with_code() {
        let (payload, ok) = repair_install_payload(Ok(report(vec![PathBuf::from(
            "/h/plugins/easybooks/stuck",
        )])));
        assert!(!ok);
        assert_eq!(payload["code"], "old_tree_not_removed");
        assert_eq!(payload["leftover"][0], "/h/plugins/easybooks/stuck");
        assert_eq!(payload["removed"], 2);
    }

    #[test]
    fn clean_repair_install_reports_ok_without_backup_fields() {
        let (payload, ok) = repair_install_payload(Ok(report(vec![])));
        assert!(ok);
        assert_eq!(payload["ok"], true);
        assert_eq!(payload["leftover"], json!([]));
        assert!(payload.get("previous_tree_preserved").is_none());
    }

    #[test]
    fn invalid_package_reports_code_and_not_ok() {
        let (payload, ok) =
            repair_install_payload(Err(install::InstallError::InvalidPackage("x".into())));
        assert!(!ok);
        assert_eq!(payload["code"], "invalid_package");
    }
}
