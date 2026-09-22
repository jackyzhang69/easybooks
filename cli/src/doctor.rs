//! `easybooks doctor` — local config check + backend round-trip + version +
//! cache freshness + optional upgrade check (contract §2).
//!
//! Output shape (exactly, contract §2):
//! {
//!   "binary_version": "0.1.0",
//!   "config": { "present": true, "path": "...", "base_url": "..." },
//!   "backend": { "reachable": true, "status": "ok" },
//!   "cache": { "location": "...|not_in_cache", "stale": false, "version": "...", "latest_available": "..." },
//!   "upgrade": { "checked": false, "upgrade_available": false }
//! }
//!
//! Flags:
//!   --no-fetch       pure local read; no network at all (config + cache only)
//!   --check-upgrade  one GitHub Tags API call (non-fatal)

use crate::config::{self, Config};
use crate::output;
use anyhow::Result;
use clap::Args;
use easybooks_cli::bootstrap;
use jz_plugin_common::doctor::CommonDoctor;
use std::time::Duration;

#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Pure local read: only report binary version + config presence + cache
    /// freshness. No network access. This is the form agents call at session
    /// start.
    #[arg(long)]
    pub no_fetch: bool,

    /// Check the plugin marketplace for a newer version. Hits GitHub's tags
    /// API with a 5-second timeout. Failure is non-fatal — the rest of doctor
    /// output is unchanged. Independent of `--no-fetch`.
    #[arg(long)]
    pub check_upgrade: bool,
}

pub fn run(args: DoctorArgs, base_url_arg: Option<String>) -> Result<()> {
    let self_version = env!("CARGO_PKG_VERSION");

    // Canonicalise so symlink-launched binaries report the same ancestor chain
    // as direct launches; the ancestor[3..6] cache walk relies on the real path.
    let self_exe = std::env::current_exe()
        .ok()
        .and_then(|p| std::fs::canonicalize(&p).ok().or(Some(p)));

    let cache = bootstrap::detect_cache_status(self_exe.as_deref(), self_version);

    // ---- config block (always local) -------------------------------------
    let cfg = Config::load(base_url_arg).ok();
    let config_present = config::config_path_buf()
        .map(|p| p.is_file())
        .unwrap_or(false);
    let config_block = match &cfg {
        Some(c) => serde_json::json!({
            "present": config_present,
            "path": config::config_path().unwrap_or_default(),
            "base_url": c.base_url,
            "api_key_masked": c.api_key_masked(),
        }),
        None => serde_json::json!({
            "present": config_present,
            "path": config::config_path().unwrap_or_default(),
            "base_url": serde_json::Value::Null,
            "api_key_masked": serde_json::Value::Null,
        }),
    };

    // ---- backend block (skipped under --no-fetch) -------------------------
    let backend_block = if args.no_fetch {
        serde_json::json!({ "reachable": false, "status": "skipped" })
    } else {
        backend_probe(cfg.as_ref())
    };

    // ---- upgrade block ----------------------------------------------------
    let upgrade_block = if args.check_upgrade {
        check_marketplace_upgrade(self_version)
    } else {
        serde_json::json!({ "checked": false, "upgrade_available": false })
    };

    let common_section = CommonDoctor::new("easybooks", self_version)
        .section()
        .map_err(|error| anyhow::anyhow!("common doctor section failed: {error}"))?;

    let payload = serde_json::json!({
        "binary_version": self_version,
        "binary_path": self_exe
            .as_deref()
            .and_then(|p| p.to_str())
            .map(bootstrap::redact_home)
            .unwrap_or_default(),
        "config": config_block,
        "common": common_section,
        "backend": backend_block,
        "cache": cache.to_json(),
        "upgrade": upgrade_block,
    });

    if !args.no_fetch {
        if backend_block.get("reachable").and_then(|v| v.as_bool()) == Some(true)
            && backend_block.get("status").and_then(|v| v.as_str()) == Some("ok")
        {
            crate::signals::emit_named("doctor_completed", "doctor", "succeeded", "doctor", None);
        } else if cfg.is_some() {
            crate::signals::emit_named("doctor_completed", "doctor", "failed", "doctor", None);
        }
    }

    output::print_json(&payload)
}

/// Round-trip `GET /api/integrations/whoami` with a short timeout. All failure
/// modes return `{ reachable:false, status:"...", hint:"..." }`; never throws.
fn backend_probe(cfg: Option<&Config>) -> serde_json::Value {
    let Some(cfg) = cfg else {
        return serde_json::json!({
            "reachable": false,
            "status": "no_config",
            "hint": "run `easybooks login --token-stdin` first",
        });
    };
    let client = match crate::client::ApiClient::from_config(cfg) {
        Ok(c) => c,
        Err(e) => {
            return serde_json::json!({
                "reachable": false,
                "status": "client_init_failed",
                "hint": format!("http client init failed: {}", e),
            })
        }
    };
    match client.get("/api/integrations/whoami", vec![]) {
        Ok(body) => serde_json::json!({
            "reachable": true,
            "status": "ok",
            "auth_kind": "exchange",
            "whoami": body,
        }),
        Err(e) => {
            let msg = format!("{e:#}");
            let lower = msg.to_lowercase();
            if lower.contains("401") || lower.contains("unauthorized") || lower.contains("403") {
                serde_json::json!({
                    "reachable": true,
                    "status": "unauthorized",
                    "hint": "token rejected; re-run easybooks login --token-stdin",
                })
            } else {
                serde_json::json!({
                    "reachable": false,
                    "status": "error",
                    "hint": msg,
                })
            }
        }
    }
}

fn check_marketplace_upgrade(self_version: &str) -> serde_json::Value {
    const TAGS_URL: &str =
        "https://api.github.com/repos/jackyzhang69/easybooks/tags?per_page=100";
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .user_agent(concat!("easybooks-cli/", env!("CARGO_PKG_VERSION")))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return serde_json::json!({
                "checked": false,
                "upgrade_available": false,
                "check_failed_reason": format!("http client init: {}", e),
            })
        }
    };
    let resp = match client.get(TAGS_URL).send() {
        Ok(r) => r,
        Err(e) => {
            return serde_json::json!({
                "checked": false,
                "upgrade_available": false,
                "check_failed_reason": format!("network: {}", e),
            })
        }
    };
    if !resp.status().is_success() {
        return serde_json::json!({
            "checked": false,
            "upgrade_available": false,
            "check_failed_reason": format!("http {}", resp.status().as_u16()),
        });
    }
    let body: serde_json::Value = match resp.json() {
        Ok(v) => v,
        Err(e) => {
            return serde_json::json!({
                "checked": false,
                "upgrade_available": false,
                "check_failed_reason": format!("json parse: {}", e),
            })
        }
    };
    let arr = match body.as_array() {
        Some(a) => a,
        None => {
            return serde_json::json!({
                "checked": false,
                "upgrade_available": false,
                "check_failed_reason": "unexpected response shape (not an array)",
            })
        }
    };
    let latest = match latest_easybooks_plugin_version(arr) {
        Some(version) => version,
        None => {
            return serde_json::json!({
                "checked": false,
                "upgrade_available": false,
                "check_failed_reason": "no EasyBooks plugin-v<semver> tags found in response",
            })
        }
    };
    let self_parsed = bootstrap::resolve::parse_strict_semver(self_version).unwrap_or_default();
    let latest_parsed = bootstrap::resolve::parse_strict_semver(&latest).unwrap_or_default();
    let available = latest_parsed > self_parsed;
    serde_json::json!({
        "checked": true,
        "current": self_version,
        "latest": latest,
        "upgrade_available": available,
    })
}

fn latest_easybooks_plugin_version(tags: &[serde_json::Value]) -> Option<String> {
    let mut versions: Vec<(Vec<u32>, String)> = tags
        .iter()
        .filter_map(|t| t.get("name").and_then(|v| v.as_str()))
        .filter_map(|name| name.strip_prefix("plugin-v"))
        .map(str::to_string)
        .filter_map(|n| bootstrap::resolve::parse_strict_semver(&n).map(|p| (p, n)))
        .collect();
    versions.sort_by(|a, b| a.0.cmp(&b.0));
    versions.pop().map(|(_, version)| version)
}

#[cfg(test)]
mod tests {
    use super::latest_easybooks_plugin_version;
    use serde_json::json;

    #[test]
    fn upgrade_tag_matrix_isolates_easybooks_releases() {
        let tags = json!([
            {"name": "v1.5.21"},
            {"name": "desktop-v2.1.20"},
            {"name": "plugin-v0.5.4"},
            {"name": "plugin-v0.5.6"},
            {"name": "plugin-v0.5.5"},
            {"name": "plugin-v0.5.7-rc1"},
            {"name": "plugin-vjunk"}
        ]);
        assert_eq!(
            latest_easybooks_plugin_version(tags.as_array().unwrap()),
            Some("0.5.6".to_string())
        );
    }

    #[test]
    fn upgrade_tag_matrix_rejects_unrelated_tags() {
        let tags = json!([{"name": "v9.9.9"}, {"name": "plugin-v0.5.7-rc1"}]);
        assert_eq!(
            latest_easybooks_plugin_version(tags.as_array().unwrap()),
            None
        );
    }
}
