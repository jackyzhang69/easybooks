//! Transaction recording (contract §2 "Record transactions"): the core
//! "drop a receipt/invoice" path. `income add` / `expense add` wrap a single
//! Entry; `tx import-json` is the batch boundary. All go through
//! POST /api/integrations/ingest/transactions (idempotent, §3).

use crate::client::ApiClient;
use crate::output;
use crate::signals::Operation;
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Map, Value};

/// Flags shared by `income add` / `expense add`. The CLI surface takes a
/// human-friendly `--amount <decimal>` (dollars), which we convert to the
/// integer `amount_cents` the Entry/backend contract expects.
pub struct AddArgs {
    pub amount: String,
    pub description: String,
    pub date: String,
    pub category: Option<String>,
    pub classification: Option<String>,
    pub source_system: Option<String>,
    pub source_id: Option<String>,
    pub notes: Option<String>,
    pub dry_run: bool,
}

/// `easybooks income add ...` / `easybooks expense add ...`
/// `entry_type` is "income" or "expense".
pub fn add(client: &ApiClient, entry_type: &str, args: AddArgs) -> Result<()> {
    let mut op = Operation::posting(entry_type, args.dry_run);
    op.summary(format!("1 {entry_type} entry"));
    op.context("operation", "add");
    op.context("entry_count", 1);
    op.context("has_category", args.category.is_some());
    op.context("has_source_id", args.source_id.is_some());
    op.step("validate", "posting_invalid", || {
        validate_date(&args.date)?;
        if let Some(c) = &args.classification {
            validate_classification(c)?;
        }
        Ok(())
    })?;
    let amount_cents = op.step("balance", "posting_invalid", || {
        parse_amount_cents(&args.amount)
    })?;
    op.detail("amount normalized to integer cents");

    let source_system = args
        .source_system
        .clone()
        .unwrap_or_else(|| "manual".to_string());

    // source_id is required for idempotency at the backend. When the user does
    // not supply one for an ad-hoc manual entry, synthesise a stable-ish id
    // from the entry's natural key so a re-run of the SAME command de-dupes
    // rather than double-recording.
    let source_id = args.source_id.clone().unwrap_or_else(|| {
        synth_source_id(entry_type, &args.date, amount_cents, &args.description)
    });

    let mut entry = Map::new();
    entry.insert("type".into(), json!(entry_type));
    entry.insert("amount_cents".into(), json!(amount_cents));
    entry.insert("description".into(), json!(args.description));
    entry.insert("date".into(), json!(args.date));
    if let Some(category) = &args.category {
        entry.insert("category_name".into(), json!(category));
    }
    if let Some(classification) = &args.classification {
        entry.insert("classification".into(), json!(classification));
    }
    entry.insert("source_id".into(), json!(source_id));
    // `notes` has no first-class Entry field; carry it in source_payload so it
    // round-trips into the stored row's payload without inventing a column.
    if let Some(notes) = &args.notes {
        entry.insert("source_payload".into(), json!({ "notes": notes }));
    }

    let mut body = Map::new();
    body.insert("source_system".into(), json!(source_system));
    body.insert("entries".into(), json!([Value::Object(entry.clone())]));

    if args.dry_run {
        // Validate + echo the resolved row without writing (contract §2).
        return output::print_json(&json!({
            "status": "dry_run",
            "would_post": "/api/integrations/ingest/transactions",
            "source_system": source_system,
            "entries": [Value::Object(entry)],
        }));
    }

    let resp = op.step("post", "posting_failed", || {
        client.post(
            "/api/integrations/ingest/transactions",
            &Value::Object(body),
        )
    })?;
    op.succeeded();
    output::print_json(&resp)
}

/// `easybooks tx import-json --json '<json>' [--dry-run]`
///
/// `<json>` is `{ "source_system":"...", "entries":[<Entry>...] }`. The user is
/// identified by the API key (`Authorization: Bearer`), so there is no owner id
/// in the body. We validate the envelope locally, then POST (or echo on dry-run).
pub fn import_json(client: &ApiClient, raw: &str, dry_run: bool) -> Result<()> {
    let mut op = Operation::posting("import", dry_run);
    let body = op.step("validate", "posting_invalid", || {
        build_import_body(raw, None)
    })?;
    finish_import(client, body, dry_run, op)
}

/// Build the ingest body from a raw JSON envelope.
///
/// `default_source_system` (used by `gmail record`) supplies a fallback when
/// the JSON omits `source_system`. Identity comes from the user's API key
/// (`Authorization: Bearer`), so no owner id is injected into the body.
pub fn build_import_body(
    raw: &str,
    default_source_system: Option<&str>,
) -> Result<Map<String, Value>> {
    let value: Value = serde_json::from_str(raw).context("--json is not valid JSON")?;
    let obj = value
        .as_object()
        .ok_or_else(|| anyhow!("--json must be a JSON object with `entries`"))?;

    let mut body: Map<String, Value> = obj.clone();

    // source_system: keep explicit; else default; else error.
    if !body.contains_key("source_system") {
        match default_source_system {
            Some(ds) => {
                body.insert("source_system".into(), json!(ds));
            }
            None => return Err(anyhow!("`source_system` is required in --json")),
        }
    }

    // entries must be a non-empty array.
    let entries = body
        .get("entries")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("`entries` must be a JSON array"))?;
    if entries.is_empty() {
        return Err(anyhow!("`entries` is empty — nothing to record"));
    }
    for (i, e) in entries.iter().enumerate() {
        validate_entry(e, i)?;
    }

    Ok(body)
}

/// POST the assembled import body, or echo it under --dry-run. `op` already
/// holds the caller's `validate` step; this records `balance` and `post`.
pub fn finish_import(
    client: &ApiClient,
    body: Map<String, Value>,
    dry_run: bool,
    mut op: Operation,
) -> Result<()> {
    if dry_run {
        return output::print_json(&json!({
            "status": "dry_run",
            "would_post": "/api/integrations/ingest/transactions",
            "body": Value::Object(body),
        }));
    }
    let (income, expense) = op.step("balance", "posting_invalid", || Ok(entry_kinds(&body)))?;
    let count = income + expense;
    op.detail(format!("{income} income, {expense} expense"));
    op.summary(format!(
        "{count} entries ({income} income, {expense} expense)"
    ));
    op.context("operation", "import");
    op.context("entry_count", count);
    op.context("income_count", income);
    op.context("expense_count", expense);
    op.context("source_system_kind", source_system_kind(&body));
    let resp = op.step("post", "posting_failed", || {
        client.post(
            "/api/integrations/ingest/transactions",
            &Value::Object(body),
        )
    })?;
    op.succeeded();
    output::print_json(&resp)
}

/// (income, expense) entry counts of a validated import body.
fn entry_kinds(body: &Map<String, Value>) -> (usize, usize) {
    let entries = body.get("entries").and_then(Value::as_array);
    let kinds = entries.into_iter().flatten();
    kinds.fold((0, 0), |(income, expense), entry| {
        match entry.get("type").and_then(Value::as_str) {
            Some("income") => (income + 1, expense),
            _ => (income, expense + 1),
        }
    })
}

/// Bounded kind of the free-form `source_system` (never the raw value).
fn source_system_kind(body: &Map<String, Value>) -> &'static str {
    match body.get("source_system").and_then(Value::as_str) {
        Some("manual") => "manual",
        Some("gmail") => "gmail",
        _ => "other",
    }
}

// --- validation helpers ----------------------------------------------------

fn validate_entry(e: &Value, index: usize) -> Result<()> {
    let obj = e
        .as_object()
        .ok_or_else(|| anyhow!("entries[{}] is not an object", index))?;

    let ty = obj
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("entries[{}] missing string `type`", index))?;
    if ty != "income" && ty != "expense" {
        return Err(anyhow!(
            "entries[{}].type must be \"income\" or \"expense\" (got {:?})",
            index,
            ty
        ));
    }
    if !obj
        .get("amount_cents")
        .map(|v| v.is_i64() || v.is_u64())
        .unwrap_or(false)
    {
        return Err(anyhow!("entries[{}] missing integer `amount_cents`", index));
    }
    if obj.get("description").and_then(|v| v.as_str()).is_none() {
        return Err(anyhow!("entries[{}] missing string `description`", index));
    }
    match obj.get("date").and_then(|v| v.as_str()) {
        Some(d) => validate_date(d).with_context(|| format!("entries[{}].date", index))?,
        None => return Err(anyhow!("entries[{}] missing string `date`", index)),
    }
    if obj.get("source_id").and_then(|v| v.as_str()).is_none() {
        return Err(anyhow!(
            "entries[{}] missing string `source_id` (required for idempotency)",
            index
        ));
    }
    if let Some(c) = obj.get("classification").and_then(|v| v.as_str()) {
        validate_classification(c).with_context(|| format!("entries[{}]", index))?;
    }
    Ok(())
}

/// Parse a decimal dollar amount into integer cents. Accepts a leading sign,
/// optional `$`, and up to two fractional digits. Rejects junk so a typo never
/// silently records the wrong figure.
pub fn parse_amount_cents(raw: &str) -> Result<i64> {
    let s = raw.trim().trim_start_matches('$').replace(',', "");
    let (sign, digits) = match s.strip_prefix('-') {
        Some(rest) => (-1i64, rest),
        None => (1i64, s.strip_prefix('+').unwrap_or(&s)),
    };
    if digits.is_empty() {
        return Err(anyhow!("--amount is empty"));
    }
    let mut parts = digits.splitn(2, '.');
    let whole = parts.next().unwrap_or("");
    let frac = parts.next().unwrap_or("");
    if whole.is_empty() && frac.is_empty() {
        return Err(anyhow!("--amount \"{}\" is not a number", raw));
    }
    if !whole.chars().all(|c| c.is_ascii_digit()) || !frac.chars().all(|c| c.is_ascii_digit()) {
        return Err(anyhow!("--amount \"{}\" is not a valid decimal", raw));
    }
    if frac.len() > 2 {
        return Err(anyhow!(
            "--amount \"{}\" has more than 2 decimal places",
            raw
        ));
    }
    let dollars: i64 = if whole.is_empty() {
        0
    } else {
        whole
            .parse()
            .map_err(|_| anyhow!("--amount \"{}\" is too large", raw))?
    };
    let cents_frac: i64 = match frac.len() {
        0 => 0,
        1 => frac.parse::<i64>().unwrap_or(0) * 10,
        _ => frac.parse::<i64>().unwrap_or(0),
    };
    Ok(sign * (dollars * 100 + cents_frac))
}

/// Validate a `YYYY-MM-DD` date string (shape + plausible ranges). We keep this
/// dependency-free (no chrono) — the backend is the authority on real calendar
/// validity; this just catches obvious mistakes before a network round-trip.
fn validate_date(d: &str) -> Result<()> {
    let parts: Vec<&str> = d.split('-').collect();
    let bad = || anyhow!("date \"{}\" must be YYYY-MM-DD", d);
    if parts.len() != 3 {
        return Err(bad());
    }
    if parts[0].len() != 4 || parts[1].len() != 2 || parts[2].len() != 2 {
        return Err(bad());
    }
    let nums: Vec<u32> = parts
        .iter()
        .map(|p| p.parse::<u32>().map_err(|_| bad()))
        .collect::<Result<_>>()?;
    let (_, m, day) = (nums[0], nums[1], nums[2]);
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) {
        return Err(anyhow!("date \"{}\" has an out-of-range month or day", d));
    }
    Ok(())
}

fn validate_classification(c: &str) -> Result<()> {
    if c == "business" || c == "mixed" || c == "personal" {
        Ok(())
    } else {
        Err(anyhow!(
            "classification must be \"business\", \"mixed\", or \"personal\" (got {:?})",
            c
        ))
    }
}

/// Deterministic synthetic source_id for a manual single entry that lacked one,
/// so re-running the identical command de-dupes at the backend's
/// (user_id, source_system, source_id) unique key. Not cryptographic — just a
/// stable fingerprint of the entry's natural fields.
fn synth_source_id(entry_type: &str, date: &str, amount_cents: i64, description: &str) -> String {
    let mut hash: u64 = 1469598103934665603; // FNV-1a offset basis
    for byte in format!("{entry_type}|{date}|{amount_cents}|{description}").bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    format!("manual-{:016x}", hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amount_parsing() {
        assert_eq!(parse_amount_cents("120").unwrap(), 12000);
        assert_eq!(parse_amount_cents("120.00").unwrap(), 12000);
        assert_eq!(parse_amount_cents("120.5").unwrap(), 12050);
        assert_eq!(parse_amount_cents("$1,234.56").unwrap(), 123456);
        assert_eq!(parse_amount_cents("-9.99").unwrap(), -999);
        assert_eq!(parse_amount_cents("0.07").unwrap(), 7);
        assert!(parse_amount_cents("abc").is_err());
        assert!(parse_amount_cents("1.234").is_err());
    }

    #[test]
    fn date_validation() {
        assert!(validate_date("2026-05-01").is_ok());
        assert!(validate_date("2026-13-01").is_err());
        assert!(validate_date("2026-5-1").is_err());
        assert!(validate_date("not-a-date").is_err());
    }

    #[test]
    fn synth_source_id_is_stable() {
        let a = synth_source_id("expense", "2026-05-01", 12000, "Software");
        let b = synth_source_id("expense", "2026-05-01", 12000, "Software");
        let c = synth_source_id("expense", "2026-05-01", 12001, "Software");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
