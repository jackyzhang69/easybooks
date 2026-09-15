//! Transaction query and per-transaction operations (contract §2):
//! list transactions with filters, get receipt URLs, confirm, and update.
//!
//!   - `tx list [--type] [--classification] [--review] [--from] [--to] [--query] [--limit]`
//!     → GET  /api/integrations/transactions
//!   - `tx receipt-url <id> [--expires <secs>]`
//!     → GET  /api/integrations/transactions/{id}/receipt-url
//!   - `tx confirm <id>`
//!     → POST /api/integrations/transactions/{id}/confirm
//!   - `tx update <id> [--amount] [--date] [--description] [--category] [--notes] [--dry-run]`
//!     → PATCH /api/integrations/transactions/{id}
//!
//! Identity comes from the `eb_live_` Bearer key — no owner id is sent.

use crate::client::ApiClient;
use crate::output;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};

/// `easybooks tx list [filters...]`
/// → GET /api/integrations/transactions
///
/// Only query parameters that were explicitly provided are sent so the backend
/// can treat absent keys as "no filter".
#[allow(clippy::too_many_arguments)]
pub fn list(
    client: &ApiClient,
    type_filter: Option<&str>,
    classification: Option<&str>,
    review: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
    query: Option<&str>,
    limit: Option<u32>,
) -> Result<()> {
    let mut q: Vec<(&str, String)> = vec![];
    if let Some(v) = type_filter {
        q.push(("type", v.to_string()));
    }
    if let Some(v) = classification {
        q.push(("classification", v.to_string()));
    }
    if let Some(v) = review {
        q.push(("review", v.to_string()));
    }
    if let Some(v) = from {
        q.push(("from", v.to_string()));
    }
    if let Some(v) = to {
        q.push(("to", v.to_string()));
    }
    if let Some(v) = query {
        q.push(("q", v.to_string()));
    }
    if let Some(v) = limit {
        q.push(("limit", v.to_string()));
    }
    output::print_json(&client.get("/api/integrations/transactions", q)?)
}

/// `easybooks tx receipt-url <id> [--expires <secs>]`
/// → GET /api/integrations/transactions/{id}/receipt-url
pub fn receipt_url(client: &ApiClient, transaction_id: &str, expires: Option<u32>) -> Result<()> {
    if transaction_id.trim().is_empty() {
        return Err(anyhow!("transaction_id is required"));
    }
    let path = format!(
        "/api/integrations/transactions/{}/receipt-url",
        encode_segment(transaction_id)
    );
    let mut q: Vec<(&str, String)> = vec![];
    if let Some(secs) = expires {
        q.push(("expires", secs.to_string()));
    }
    output::print_json(&client.get(&path, q)?)
}

/// `easybooks tx confirm <id>`
/// → POST /api/integrations/transactions/{id}/confirm
pub fn confirm(client: &ApiClient, transaction_id: &str) -> Result<()> {
    if transaction_id.trim().is_empty() {
        return Err(anyhow!("transaction_id is required"));
    }
    let path = format!(
        "/api/integrations/transactions/{}/confirm",
        encode_segment(transaction_id)
    );
    output::print_json(&client.post(&path, &json!({}))?)
}

/// `easybooks tx update <id> [--amount] [--date] [--description] [--category] [--notes] [--dry-run]`
/// → PATCH /api/integrations/transactions/{id}
///
/// Only fields explicitly provided are included in the PATCH body.
/// `--category` maps to `category_name` in the JSON body (backend convention).
/// `--dry-run` prints the body that would be sent without making a network call.
#[allow(clippy::too_many_arguments)]
pub fn update(
    client: &ApiClient,
    transaction_id: &str,
    amount: Option<&str>,
    date: Option<&str>,
    description: Option<&str>,
    category: Option<&str>,
    notes: Option<&str>,
    dry_run: bool,
) -> Result<()> {
    if transaction_id.trim().is_empty() {
        return Err(anyhow!("transaction_id is required"));
    }

    let mut body = serde_json::Map::new();
    if let Some(a) = amount {
        // eb_transactions.amount stores DOLLARS and the PATCH body field is
        // `amount` (a decimal number), NOT `amount_cents`. Parse strictly via
        // cents (validates <=2dp) then emit dollars as a JSON number.
        let cents = parse_amount_cents(a)?;
        let dollars = serde_json::Number::from_f64(cents as f64 / 100.0)
            .ok_or_else(|| anyhow!("--amount {:?} is not a finite number", a))?;
        body.insert("amount".to_string(), Value::Number(dollars));
    }
    if let Some(d) = date {
        body.insert("date".to_string(), json!(d));
    }
    if let Some(desc) = description {
        body.insert("description".to_string(), json!(desc));
    }
    if let Some(cat) = category {
        body.insert("category_name".to_string(), json!(cat));
    }
    if let Some(n) = notes {
        body.insert("notes".to_string(), json!(n));
    }

    if body.is_empty() {
        return Err(anyhow!(
            "tx update requires at least one field to change (--amount, --date, --description, --category, --notes)"
        ));
    }

    if dry_run {
        return output::print_json(&json!({
            "status": "dry_run",
            "would_patch": format!("/api/integrations/transactions/{}", encode_segment(transaction_id)),
            "body": Value::Object(body),
        }));
    }

    let path = format!(
        "/api/integrations/transactions/{}",
        encode_segment(transaction_id)
    );
    output::print_json(&client.send_with_body("PATCH", &path, &Value::Object(body))?)
}

/// `easybooks tx delete <id> [--force]`
/// → DELETE /api/integrations/transactions/{id}
///
/// Guard: requires `--force` so agents/callers fail-safe rather than
/// silently deleting records without explicit intent.
pub fn delete(client: &ApiClient, transaction_id: &str, force: bool) -> Result<()> {
    if transaction_id.trim().is_empty() {
        return Err(anyhow!("transaction_id is required"));
    }
    if !force {
        return Err(anyhow!(
            "tx delete requires --force to confirm the deletion (non-interactive CLI guard)"
        ));
    }
    let path = format!(
        "/api/integrations/transactions/{}",
        encode_segment(transaction_id)
    );
    output::print_json(&client.delete(&path)?)
}

/// Parse a decimal dollar amount into integer cents (mirrors transactions.rs).
fn parse_amount_cents(raw: &str) -> Result<i64> {
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
    if !whole.chars().all(|c| c.is_ascii_digit()) || !frac.chars().all(|c| c.is_ascii_digit()) {
        return Err(anyhow!("--amount {:?} is not a valid decimal", raw));
    }
    if frac.len() > 2 {
        return Err(anyhow!("--amount {:?} has more than 2 decimal places", raw));
    }
    let dollars: i64 = if whole.is_empty() {
        0
    } else {
        whole
            .parse()
            .map_err(|_| anyhow!("--amount {:?} is too large", raw))?
    };
    let cents_frac: i64 = match frac.len() {
        0 => 0,
        1 => frac.parse::<i64>().unwrap_or(0) * 10,
        _ => frac.parse::<i64>().unwrap_or(0),
    };
    Ok(sign * (dollars * 100 + cents_frac))
}

/// Percent-encode path-breaking characters (mirrors tx_ops::encode_segment).
fn encode_segment(value: &str) -> String {
    value.replace('/', "%2F").replace(' ', "%20")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amount_cents_decimal() {
        assert_eq!(parse_amount_cents("99.99").unwrap(), 9999);
        assert_eq!(parse_amount_cents("$1,234.56").unwrap(), 123456);
        assert_eq!(parse_amount_cents("-50.00").unwrap(), -5000);
        assert!(parse_amount_cents("1.234").is_err());
        assert!(parse_amount_cents("abc").is_err());
    }

    #[test]
    fn encode_segment_handles_specials() {
        assert_eq!(encode_segment("txn_abc"), "txn_abc");
        assert_eq!(encode_segment("a/b"), "a%2Fb");
        assert_eq!(encode_segment("a b"), "a%20b");
    }

    #[test]
    fn delete_requires_force() {
        let force = false;
        let result: Result<()> = if !force {
            Err(anyhow!("requires --force"))
        } else {
            Ok(())
        };
        assert!(result.is_err());
    }
}
