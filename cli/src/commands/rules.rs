//! Auto-categorization rules (contract §2) — a QB Bank Rules inspired command
//! group that lets the agent manage the user's classification rules and apply
//! them across recorded transactions.
//!
//!   - `rules list`                → GET    /api/integrations/rules
//!   - `rules show <id>`           → GET    /api/integrations/rules/{id}
//!   - `rules create --json <j>`   → POST   /api/integrations/rules
//!   - `rules delete <id>`         → DELETE /api/integrations/rules/{id}
//!   - `rules enable <id>`         → PATCH  /api/integrations/rules/{id} {enabled:true}
//!   - `rules disable <id>`        → PATCH  /api/integrations/rules/{id} {enabled:false}
//!   - `rules apply --scope <all|unclassified|selected> [--ids …] [--rule-ids …]
//!      [--only-auto-apply] [--commit]`
//!     → POST   /api/integrations/rules/apply
//!
//! Every call goes through the bundled `ApiClient` so it carries the user's
//! `eb_live_` Bearer key exactly like every other read/write. Identity comes
//! from the key, so no owner id is sent in the body or path.

use crate::client::ApiClient;
use crate::output;
use crate::signals::Operation;
use anyhow::{anyhow, Context, Result};
use serde_json::json;

/// `easybooks rules list` → GET /api/integrations/rules
pub fn list(client: &ApiClient) -> Result<()> {
    output::print_json(&client.get("/api/integrations/rules", vec![])?)
}

/// `easybooks rules show <rule_id>` → GET /api/integrations/rules/{id}
pub fn show(client: &ApiClient, rule_id: &str) -> Result<()> {
    if rule_id.trim().is_empty() {
        return Err(anyhow!("rule_id is required"));
    }
    let path = format!("/api/integrations/rules/{}", encode_segment(rule_id));
    output::print_json(&client.get(&path, vec![])?)
}

/// `easybooks rules create --json '<rule json>'` → POST /api/integrations/rules
///
/// The `--json` payload is parsed into a `serde_json::Value` and posted as-is so
/// the rule schema stays owned by the backend; a parse failure surfaces a clear
/// "invalid --json rule payload" error before any round-trip.
pub fn create(client: &ApiClient, json_str: &str) -> Result<()> {
    let body: serde_json::Value =
        serde_json::from_str(json_str).context("invalid --json rule payload")?;
    let mut op = Operation::named("rule_created", "rule", "rule");
    op.context("operation", "rule_create");
    let resp = op.step("post", "rule_failed", || {
        client.post("/api/integrations/rules", &body)
    })?;
    op.succeeded();
    output::print_json(&resp)
}

/// `easybooks rules delete <rule_id>` → DELETE /api/integrations/rules/{id}
pub fn delete(client: &ApiClient, rule_id: &str) -> Result<()> {
    if rule_id.trim().is_empty() {
        return Err(anyhow!("rule_id is required"));
    }
    let path = format!("/api/integrations/rules/{}", encode_segment(rule_id));
    output::print_json(&client.delete(&path)?)
}

/// `easybooks rules enable <rule_id>`
/// → PATCH /api/integrations/rules/{id} {enabled:true}
pub fn enable(client: &ApiClient, rule_id: &str) -> Result<()> {
    set_enabled(client, rule_id, true)
}

/// `easybooks rules disable <rule_id>`
/// → PATCH /api/integrations/rules/{id} {enabled:false}
pub fn disable(client: &ApiClient, rule_id: &str) -> Result<()> {
    set_enabled(client, rule_id, false)
}

/// Shared PATCH for enable/disable — flips only the `enabled` flag.
fn set_enabled(client: &ApiClient, rule_id: &str, enabled: bool) -> Result<()> {
    if rule_id.trim().is_empty() {
        return Err(anyhow!("rule_id is required"));
    }
    let body = json!({ "enabled": enabled });
    let path = format!("/api/integrations/rules/{}", encode_segment(rule_id));
    output::print_json(&client.send_with_body("PATCH", &path, &body)?)
}

/// `easybooks rules apply --scope <all|unclassified|selected> [--ids a,b]
/// [--rule-ids r1,r2] [--only-auto-apply] [--commit]`
/// → POST /api/integrations/rules/apply
///
/// Without `--commit` the backend returns a dry-run preview (`preview` /
/// `total_scanned`); with `--commit` it persists the matches (`committed`). The
/// `--ids` / `--rule-ids` lists are comma-split into trimmed, non-empty arrays
/// and only included when present.
pub fn apply(
    client: &ApiClient,
    scope: &str,
    ids: Option<&str>,
    rule_ids: Option<&str>,
    only_auto_apply: bool,
    commit: bool,
) -> Result<()> {
    validate_scope(scope)?;

    let mut body = json!({
        "scope": scope,
        "commit": commit,
        "only_auto_apply": only_auto_apply,
    });
    let obj = body
        .as_object_mut()
        .expect("body is constructed as a JSON object");

    if let Some(raw) = ids {
        let transaction_ids = split_csv(raw);
        if !transaction_ids.is_empty() {
            obj.insert("transaction_ids".to_string(), json!(transaction_ids));
        }
    }
    if let Some(raw) = rule_ids {
        let rule_ids = split_csv(raw);
        if !rule_ids.is_empty() {
            obj.insert("rule_ids".to_string(), json!(rule_ids));
        }
    }

    output::print_json(&client.post("/api/integrations/rules/apply", &body)?)
}

/// Validate the apply scope is one of the three supported selectors.
fn validate_scope(scope: &str) -> Result<()> {
    match scope {
        "all" | "unclassified" | "selected" => Ok(()),
        _ => Err(anyhow!(
            "--scope must be one of all|unclassified|selected (got {:?})",
            scope
        )),
    }
}

/// Split a comma-separated list into trimmed, non-empty strings. Empty segments
/// (e.g. trailing commas or whitespace-only fields) are dropped.
fn split_csv(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// Percent-encode the characters that would break a path segment. The rule id
/// is normally a uuid, but we stay defensive (mirrors `tx_ops::encode_segment`).
fn encode_segment(value: &str) -> String {
    value.replace('/', "%2F").replace(' ', "%20")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_validation() {
        assert!(validate_scope("all").is_ok());
        assert!(validate_scope("unclassified").is_ok());
        assert!(validate_scope("selected").is_ok());
        assert!(validate_scope("All").is_err());
        assert!(validate_scope("everything").is_err());
        assert!(validate_scope("").is_err());
    }

    #[test]
    fn csv_split_trims_and_drops_empties() {
        assert_eq!(split_csv("a,b,c"), vec!["a", "b", "c"]);
        assert_eq!(split_csv(" a , b ,c "), vec!["a", "b", "c"]);
        assert_eq!(split_csv("a,,b,"), vec!["a", "b"]);
        assert_eq!(split_csv(""), Vec::<String>::new());
        assert_eq!(split_csv("  ,  "), Vec::<String>::new());
        assert_eq!(split_csv("solo"), vec!["solo"]);
    }

    #[test]
    fn encode_segment_escapes_path_breakers() {
        assert_eq!(encode_segment("rule_123"), "rule_123");
        assert_eq!(encode_segment("a/b"), "a%2Fb");
        assert_eq!(encode_segment("a b"), "a%20b");
    }
}
