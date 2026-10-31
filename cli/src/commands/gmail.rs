//! Gmail commands (contract §2 "Gmail").
//!
//! v1: the agent reads candidate receipts/invoices via the connected Gmail MCP
//! and RECORDS what it extracted through the CLI. `gmail record` is an alias of
//! `tx import-json` with `source_system` defaulted to `gmail`; each entry's
//! `source_id` MUST be the Gmail message id, which guarantees idempotency (no
//! double-recording on re-scan).
//!
//! v2 (documented, not built): native Gmail OAuth in the CLI so `gmail sync`
//! can pull candidates headless for cron. This module is the deliberate seam.

use crate::client::ApiClient;
use crate::commands::transactions;
use crate::output;
use crate::signals::Operation;
use anyhow::{anyhow, Result};
use serde_json::json;

/// `easybooks gmail record --json '<json>' [--dry-run]`
///
/// Alias of `tx import-json` with `source_system` defaulted to `gmail`. We
/// additionally enforce that every entry's `source_id` looks like a Gmail
/// message id (non-empty), because that id is the idempotency key that stops a
/// re-scan from recording the same receipt twice.
pub fn record(client: &ApiClient, raw: &str, dry_run: bool) -> Result<()> {
    let mut op = Operation::posting("import", dry_run);
    let body = op.step("validate", "posting_invalid", || {
        let body = transactions::build_import_body(raw, Some("gmail"))?;

        // Guard: each entry must carry a source_id (the Gmail message id). The
        // shared validator already requires a non-empty source_id, but we restate
        // the intent here with a Gmail-specific message so the agent fixes the
        // right thing.
        if let Some(entries) = body.get("entries").and_then(|v| v.as_array()) {
            for (i, e) in entries.iter().enumerate() {
                let sid = e.get("source_id").and_then(|v| v.as_str()).unwrap_or("");
                if sid.trim().is_empty() {
                    return Err(anyhow!(
                        "entries[{}].source_id must be the Gmail message id (got empty) — \
                         this is what prevents double-recording on re-scan",
                        i
                    ));
                }
            }
        }
        Ok(body)
    })?;

    transactions::finish_import(client, body, dry_run, op)
}

/// `easybooks gmail sync` — v1 STUB. Native OAuth sync ships in v2; for now the
/// agent reads Gmail via the MCP and records with `gmail record`.
pub fn sync() -> Result<()> {
    output::print_json(&json!({
        "status": "not_implemented_v1",
        "hint": "In v1, read Gmail via the Gmail MCP and record with `easybooks gmail record`. Native OAuth sync ships in v2."
    }))
}
