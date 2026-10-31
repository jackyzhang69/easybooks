//! Invoice commands (contract §2 "Invoices").
//!   - `invoice create --json '<json>' [--dry-run]` → POST /api/integrations/ingest/invoice
//!   - `invoice send <invoice_id>`                  → POST /api/integrations/invoice/:id/send
//!   - `invoice get <id>`                           → GET  /api/integrations/invoices/{id}
//!   - `invoice mark <id> --status <paid|unpaid>`   → POST /api/integrations/invoice/{id}/status
//!   - `invoice pdf <id> [--out <path>]`            → GET  /api/integrations/invoice/{id}/pdf
//!   - `invoice stats [--year <YYYY>]`              → GET  /api/integrations/invoices/stats

use crate::client::ApiClient;
use crate::output;
use crate::signals::Operation;
use anyhow::{anyhow, Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use serde_json::{json, Value};

/// `easybooks invoice create --json '<json>' [--dry-run]`
///
/// `<json>` = `{ "client": {...} | {"client_id":uuid}, "issue_date", "due_date",
/// "tax_rate?", "items":[{description,quantity,unit_price}], "notes?",
/// "payment_details?", "source_id?" }`. Subtotal/tax/total are computed
/// server-side; we only validate the envelope + inject the configured owner.
pub fn create(client: &ApiClient, raw: &str, dry_run: bool) -> Result<()> {
    let mut op = Operation::invoice(dry_run);
    op.context("operation", "invoice_create");
    let body = op.step("build", "invoice_invalid", || {
        let value: Value = serde_json::from_str(raw).context("--json is not valid JSON")?;
        let obj = value
            .as_object()
            .ok_or_else(|| anyhow!("--json must be a JSON object"))?;

        // Minimal local validation — the backend is authoritative, but we catch
        // obvious mistakes before a round-trip.
        if obj.get("client").is_none() {
            return Err(anyhow!("--json missing `client` (object or {{client_id}})"));
        }
        if obj.get("issue_date").and_then(|v| v.as_str()).is_none() {
            return Err(anyhow!("--json missing string `issue_date` (YYYY-MM-DD)"));
        }
        if obj.get("due_date").and_then(|v| v.as_str()).is_none() {
            return Err(anyhow!("--json missing string `due_date` (YYYY-MM-DD)"));
        }
        match obj.get("items").and_then(|v| v.as_array()) {
            Some(items) if !items.is_empty() => {}
            _ => return Err(anyhow!("--json `items` must be a non-empty array")),
        }

        Ok(obj.clone())
    })?;
    let items = body
        .get("items")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let client_ref = if body
        .get("client")
        .and_then(|c| c.get("client_id"))
        .is_some()
    {
        "client_id"
    } else {
        "inline"
    };
    op.detail(format!("{items} line items, client by {client_ref}"));
    op.summary(format!("{items} line items; client by {client_ref}"));
    op.context("item_count", items);
    op.context("client_ref", client_ref);
    op.context("has_tax_rate", body.contains_key("tax_rate"));
    op.context("has_notes", body.contains_key("notes"));

    // Invoice numbers are server-generated; record which idempotency key the
    // server will use for this create.
    let keyed_by = if body.contains_key("source_id") {
        "source_id"
    } else {
        "invoice_number"
    };
    op.step("number", "invoice_invalid", || Ok(()))?;
    op.detail(format!("server-assigned number; idempotent on {keyed_by}"));
    op.context("idempotency_key_kind", keyed_by);

    if dry_run {
        return output::print_json(&json!({
            "status": "dry_run",
            "would_post": "/api/integrations/ingest/invoice",
            "body": Value::Object(body),
        }));
    }

    let resp = op.step("persist", "invoice_failed", || {
        client.post("/api/integrations/ingest/invoice", &Value::Object(body))
    })?;
    if let Some(created) = resp.get("created").and_then(Value::as_bool) {
        op.detail(if created {
            "created"
        } else {
            "existing invoice matched"
        });
        op.context("created", created);
    }
    op.context("number_assigned", resp.get("invoice_number").is_some());
    op.succeeded();
    output::print_json(&resp)
}

/// `easybooks invoice send <invoice_id>` → proxy to the integration send route
/// `POST /api/integrations/invoice/:id/send`. That route authenticates with the
/// integration key (which the client always sends) and resolves the owner user
/// id from the body/header/env, so the legacy `x-user-id` header is not needed.
/// Output is passed through unchanged.
pub fn send(client: &ApiClient, invoice_id: &str) -> Result<()> {
    let path = format!(
        "/api/integrations/invoice/{}/send",
        encode_segment(invoice_id)
    );
    match client.post(&path, &json!({})) {
        Ok(resp) => {
            crate::signals::emit_named("invoice_sent", "invoice", "succeeded", "invoice", None);
            output::print_json(&resp)
        }
        Err(err) => {
            crate::signals::emit_named(
                "invoice_sent",
                "invoice",
                "failed",
                "invoice",
                Some("invoice_failed"),
            );
            Err(err)
        }
    }
}

/// `easybooks invoice get <id>`
/// → GET /api/integrations/invoices/{id}
pub fn get(client: &ApiClient, invoice_id: &str) -> Result<()> {
    if invoice_id.trim().is_empty() {
        return Err(anyhow!("invoice_id is required"));
    }
    let path = format!("/api/integrations/invoices/{}", encode_segment(invoice_id));
    output::print_json(&client.get(&path, vec![])?)
}

/// `easybooks invoice mark <id> --status <paid|unpaid>`
/// → POST /api/integrations/invoice/{id}/status  body {"status": ...}
pub fn mark(client: &ApiClient, invoice_id: &str, status: &str) -> Result<()> {
    if invoice_id.trim().is_empty() {
        return Err(anyhow!("invoice_id is required"));
    }
    validate_invoice_status(status)?;
    let path = format!(
        "/api/integrations/invoice/{}/status",
        encode_segment(invoice_id)
    );
    let body = json!({ "status": status });
    match client.post(&path, &body) {
        Ok(resp) => {
            crate::signals::emit_named("invoice_marked", "invoice", "succeeded", "invoice", None);
            output::print_json(&resp)
        }
        Err(err) => {
            crate::signals::emit_named(
                "invoice_marked",
                "invoice",
                "failed",
                "invoice",
                Some("invoice_failed"),
            );
            Err(err)
        }
    }
}

/// `easybooks invoice pdf <id> [--out <path>]`
/// → GET /api/integrations/invoice/{id}/pdf
///
/// Decodes `response.content_base64` from the JSON response and writes the bytes
/// to `--out` (defaults to `./<filename>` from the response). Prints
/// `{"saved": "<path>"}` on success.
pub fn pdf(client: &ApiClient, invoice_id: &str, out: Option<&str>) -> Result<()> {
    if invoice_id.trim().is_empty() {
        return Err(anyhow!("invoice_id is required"));
    }
    let path = format!(
        "/api/integrations/invoice/{}/pdf",
        encode_segment(invoice_id)
    );
    let resp = client.get(&path, vec![])?;

    // Extract the base64-encoded PDF content from the response.
    let content_b64 = resp
        .get("content_base64")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("response missing `content_base64` field"))?;

    let bytes = BASE64
        .decode(content_b64)
        .context("failed to decode content_base64 from response")?;

    // Determine the output path: --out flag, else filename from response, else default.
    let save_path = if let Some(p) = out {
        p.to_string()
    } else {
        let filename = resp
            .get("filename")
            .and_then(|v| v.as_str())
            .unwrap_or("invoice.pdf");
        format!("./{}", filename)
    };

    std::fs::write(&save_path, &bytes)
        .with_context(|| format!("failed to write PDF to {:?}", save_path))?;

    output::print_json(&json!({ "saved": save_path }))
}

/// `easybooks invoice stats [--year <YYYY>]`
/// → GET /api/integrations/invoices/stats
pub fn stats(client: &ApiClient, year: Option<u32>) -> Result<()> {
    let mut q: Vec<(&str, String)> = vec![];
    if let Some(y) = year {
        q.push(("year", y.to_string()));
    }
    output::print_json(&client.get("/api/integrations/invoices/stats", q)?)
}

fn validate_invoice_status(status: &str) -> Result<()> {
    match status {
        "paid" | "unpaid" => Ok(()),
        _ => Err(anyhow!("--status must be paid|unpaid (got {:?})", status)),
    }
}

fn encode_segment(value: &str) -> String {
    value.replace('/', "%2F").replace(' ', "%20")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invoice_status_validation() {
        assert!(validate_invoice_status("paid").is_ok());
        assert!(validate_invoice_status("unpaid").is_ok());
        assert!(validate_invoice_status("Paid").is_err());
        assert!(validate_invoice_status("pending").is_err());
        assert!(validate_invoice_status("").is_err());
    }

    #[test]
    fn encode_segment_handles_specials() {
        assert_eq!(encode_segment("inv_123"), "inv_123");
        assert_eq!(encode_segment("a/b"), "a%2Fb");
        assert_eq!(encode_segment("a b"), "a%20b");
    }
}
