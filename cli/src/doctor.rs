//! `easybooks doctor` — local config check + backend round-trip + version +
//! cache freshness (contract §2). `--repair-install` is handled before this
//! module runs (see `lifecycle.rs`).
//!
//! Output shape (exactly, contract §2):
//! {
//!   "binary_version": "0.1.0",
//!   "config": { "present": true, "path": "...", "base_url": "..." },
//!   "backend": { "reachable": true, "status": "ok" },
//!   "cache": { "location": "...|not_in_cache", "stale": false, "version": "...", "latest_available": "..." }
//! }
//!
//! Flags:
//!   --no-fetch        pure local read; no network at all (config + cache only)
//!   --repair-install  reinstall this package as the only installed copy

use crate::config::{self, Config};
use crate::output;
use anyhow::Result;
use clap::Args;
use easybooks_cli::bootstrap;
use jz_plugin_common::doctor::CommonDoctor;
use std::time::Instant;

#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Pure local read: only report binary version + config presence + cache
    /// freshness. No network access. This is the form agents call at session
    /// start.
    #[arg(long)]
    pub no_fetch: bool,

    /// Reinstall this package as the only installed copy: switch
    /// `~/.jackyzhang.app/plugins/easybooks/current` to it, then delete every
    /// other tree and older versions in the host plugin cache. Login, settings
    /// and user data are kept. Run it with the binary from the newly installed
    /// package. Prints JSON; works offline and without a login.
    #[arg(long, conflicts_with = "no_fetch")]
    pub repair_install: bool,
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
    let probe_started = Instant::now();
    let backend_block = if args.no_fetch {
        serde_json::json!({ "reachable": false, "status": "skipped" })
    } else {
        backend_probe(cfg.as_ref())
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
    });

    if !args.no_fetch {
        let status = backend_block
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let reachable = backend_block.get("reachable").and_then(|v| v.as_bool()) == Some(true);
        let mut op = crate::signals::Operation::named("doctor_completed", "doctor", "doctor");
        op.context("operation", "doctor");
        op.context("backend_status", status);
        op.context("backend_reachable", reachable);
        if reachable && status == "ok" {
            op.record("backend_probe", probe_started, None);
            op.succeeded();
        } else if cfg.is_some() {
            let code = format!("backend_{status}");
            op.record("backend_probe", probe_started, Some(&code));
            let hint = backend_block
                .get("hint")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            op.failed(
                &code,
                &anyhow::anyhow!("backend probe failed ({status}): {hint}"),
            );
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
