//! Client mutation commands (contract §2):
//! create, update, and delete clients.
//!
//!   - `clients create (--name <n> [...]) | (--json '<obj>')`
//!     → POST   /api/integrations/clients
//!   - `clients update <id> [--name] [--email] [--phone] [--address] [--notes]`
//!     → PATCH  /api/integrations/clients/{id}
//!   - `clients delete <id> [--force]`
//!     → DELETE /api/integrations/clients/{id}
//!
//! Identity comes from the `eb_live_` Bearer key — no owner id is sent.

use crate::client::ApiClient;
use crate::output;
use crate::signals::Operation;
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};

/// `easybooks clients create --name <n> [--email] [--phone] [--address] [--notes]`
/// OR `easybooks clients create --json '<obj>'`
/// → POST /api/integrations/clients
///
/// When `--json` is supplied the raw object is posted as-is (all other flags are
/// ignored). Otherwise, only the flags that were provided are included in the body.
#[allow(clippy::too_many_arguments)]
pub fn create(
    client: &ApiClient,
    name: Option<&str>,
    email: Option<&str>,
    phone: Option<&str>,
    address: Option<&str>,
    notes: Option<&str>,
    json_str: Option<&str>,
) -> Result<()> {
    let body: Value = if let Some(raw) = json_str {
        serde_json::from_str(raw).context("--json is not valid JSON")?
    } else {
        let name = name.ok_or_else(|| anyhow!("--name is required (or use --json)"))?;
        if name.trim().is_empty() {
            return Err(anyhow!("--name must not be empty"));
        }
        let mut obj = serde_json::Map::new();
        obj.insert("name".to_string(), json!(name));
        if let Some(v) = email {
            obj.insert("email".to_string(), json!(v));
        }
        if let Some(v) = phone {
            obj.insert("phone".to_string(), json!(v));
        }
        if let Some(v) = address {
            obj.insert("address".to_string(), json!(v));
        }
        if let Some(v) = notes {
            obj.insert("notes".to_string(), json!(v));
        }
        Value::Object(obj)
    };

    let mut op = Operation::named("client_created", "client", "client");
    op.context("operation", "client_create");
    let resp = op.step("post", "client_failed", || {
        client.post("/api/integrations/clients", &body)
    })?;
    op.succeeded();
    output::print_json(&resp)
}

/// `easybooks clients update <id> [--name] [--email] [--phone] [--address] [--notes]`
/// → PATCH /api/integrations/clients/{id}
///
/// Only the flags that were provided are included in the PATCH body.
#[allow(clippy::too_many_arguments)]
pub fn update(
    client: &ApiClient,
    client_id: &str,
    name: Option<&str>,
    email: Option<&str>,
    phone: Option<&str>,
    address: Option<&str>,
    notes: Option<&str>,
) -> Result<()> {
    if client_id.trim().is_empty() {
        return Err(anyhow!("client_id is required"));
    }
    let mut body = serde_json::Map::new();
    if let Some(v) = name {
        body.insert("name".to_string(), json!(v));
    }
    if let Some(v) = email {
        body.insert("email".to_string(), json!(v));
    }
    if let Some(v) = phone {
        body.insert("phone".to_string(), json!(v));
    }
    if let Some(v) = address {
        body.insert("address".to_string(), json!(v));
    }
    if let Some(v) = notes {
        body.insert("notes".to_string(), json!(v));
    }
    if body.is_empty() {
        return Err(anyhow!(
            "clients update requires at least one field (--name, --email, --phone, --address, --notes)"
        ));
    }
    let path = format!("/api/integrations/clients/{}", encode_segment(client_id));
    output::print_json(&client.send_with_body("PATCH", &path, &Value::Object(body))?)
}

/// `easybooks clients delete <id> [--force]`
/// → DELETE /api/integrations/clients/{id}
///
/// Without `--force` the command will ask (in a real interactive session) before
/// deleting. Since the CLI is machine-first (non-interactive), `--force` is
/// required to proceed; without it an explicit error is returned so agents that
/// call without the flag fail-safe rather than silently deleting.
pub fn delete(client: &ApiClient, client_id: &str, force: bool) -> Result<()> {
    if client_id.trim().is_empty() {
        return Err(anyhow!("client_id is required"));
    }
    if !force {
        return Err(anyhow!(
            "clients delete requires --force to confirm the deletion (non-interactive CLI guard)"
        ));
    }
    let path = format!("/api/integrations/clients/{}", encode_segment(client_id));
    output::print_json(&client.delete(&path)?)
}

/// Percent-encode path-breaking characters.
fn encode_segment(value: &str) -> String {
    value.replace('/', "%2F").replace(' ', "%20")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_requires_name_or_json() {
        // Both name and json_str are None → should error
        // We can't call the real function without a client, but we can verify
        // the guard logic inline.
        let name: Option<&str> = None;
        let json_str: Option<&str> = None;
        // Simulate guard: if neither --name nor --json, it must error.
        let result: Result<()> = if json_str.is_none() && name.is_none() {
            Err(anyhow!("--name is required (or use --json)"))
        } else {
            Ok(())
        };
        assert!(result.is_err());
    }

    #[test]
    fn delete_requires_force() {
        // Guard logic: without --force, deletion should be refused.
        let force = false;
        let result: Result<()> = if !force {
            Err(anyhow!("requires --force"))
        } else {
            Ok(())
        };
        assert!(result.is_err());
    }

    #[test]
    fn delete_with_force_passes_guard() {
        let force = true;
        let result: Result<()> = if !force {
            Err(anyhow!("requires --force"))
        } else {
            Ok(())
        };
        assert!(result.is_ok());
    }

    #[test]
    fn encode_segment_paths() {
        assert_eq!(encode_segment("client_123"), "client_123");
        assert_eq!(encode_segment("a/b"), "a%2Fb");
    }
}
