//! Local host lifecycle bridge. No model calls, background process, transcript
//! inspection, remote instructions, credentials, or business execution.
use crate::{home::Home, pair};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs, io,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Serialize, Deserialize)]
struct Receipt {
    session: String,
    product: String,
    phase: String,
    expires_at: i64,
    revision: String,
}

#[derive(Default, Serialize, Deserialize)]
struct Binding {
    receipt: String,
    bound_revision: Option<String>,
    continued_revision: Option<String>,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid pair host state")
}

fn directory(plugin: &str) -> io::Result<PathBuf> {
    if !matches!(plugin, "anychat" | "plugin-admin") {
        return Err(invalid());
    }
    let home = Home::resolve()?;
    crate::home::guard_credential_paths(&home)?;
    let dir = home.ensure_plugin_runtime(plugin)?.join("pair-host");
    crate::home::reject_symlink(&dir)?;
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// A receipt is generated only by a successful local CLI action. Hook input
/// cannot manufacture a binding by embedding an arbitrary peer message.
pub fn decorate(
    plugin: &str,
    product: &str,
    session: &str,
    phase: &str,
    expires_at: i64,
    output: Value,
) -> io::Result<Value> {
    crate::pair_receiver::retire_current(plugin)?;
    decorate_generation(plugin, product, session, phase, expires_at, output, None)
}

pub fn decorate_generation(
    plugin: &str,
    product: &str,
    session: &str,
    phase: &str,
    expires_at: i64,
    mut output: Value,
    generation: Option<&str>,
) -> io::Result<Value> {
    pair::validate_identifier(session).map_err(|_| invalid())?;
    if !matches!(
        phase,
        "active" | "needs_human" | "offline" | "closed" | "expired"
    ) {
        return Err(invalid());
    }
    if !matches!(
        product,
        "anychat" | "anyknow" | "anypdf" | "anydoc" | "anyweb" | "easybooks" | "formbro"
    ) {
        return Err(invalid());
    }
    let dir = directory(plugin)?;
    let revision = generation
        .map(str::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
    pair::validate_identifier(&revision).map_err(|_| invalid())?;
    let key = format!("{product}-{session}");
    let path = dir.join(format!("{key}.json"));
    let receipt = Receipt {
        session: session.into(),
        product: product.into(),
        phase: phase.into(),
        expires_at,
        revision: revision.clone(),
    };
    crate::home::write_private_file(&path, &serde_json::to_vec(&receipt)?)?;
    output.as_object_mut().ok_or_else(invalid)?.insert(
        "pair_host".into(),
        json!({"receipt":key,"revision":revision}),
    );
    Ok(output)
}

fn read_receipt(dir: &std::path::Path, key: &str) -> io::Result<Receipt> {
    if key.len() > 50 || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err(invalid());
    }
    let path = dir.join(format!("{key}.json"));
    crate::home::reject_symlink(&path)?;
    let receipt: Receipt = serde_json::from_slice(&fs::read(path)?).map_err(|_| invalid())?;
    pair::validate_identifier(&receipt.session).map_err(|_| invalid())?;
    pair::validate_identifier(&receipt.revision).map_err(|_| invalid())?;
    if key != format!("{}-{}", receipt.product, receipt.session)
        || !matches!(
            receipt.product.as_str(),
            "anychat" | "anyknow" | "anypdf" | "anydoc" | "anyweb" | "easybooks" | "formbro"
        )
        || !matches!(
            receipt.phase.as_str(),
            "active" | "needs_human" | "offline" | "closed" | "expired"
        )
    {
        return Err(invalid());
    }
    Ok(receipt)
}

pub fn current_generation(
    plugin: &str,
    product: &str,
    session: &str,
) -> io::Result<Option<String>> {
    pair::validate_identifier(session).map_err(|_| invalid())?;
    let dir = directory(plugin)?;
    match read_receipt(&dir, &format!("{product}-{session}")) {
        Ok(receipt) => Ok(Some(receipt.revision)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

// Inspect only the host's actual tool-output wrappers and top-level CLI JSON
// records, never recurse into message bodies or arbitrary tool input.
fn marker(value: &Value) -> Option<&Value> {
    value.get("pair_host")
}

fn output_markers(value: &Value) -> Vec<Value> {
    if let Some(m) = marker(value) {
        return vec![m.clone()];
    }
    if let Some(text) = value.as_str() {
        return text
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter_map(|v| marker(&v).cloned())
            .collect();
    }
    for key in ["stdout", "output"] {
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            return output_markers(&json!(text));
        }
    }
    if let Some(content) = value.get("content").and_then(Value::as_array) {
        return content
            .iter()
            .filter_map(|v| v.get("text"))
            .flat_map(output_markers)
            .collect();
    }
    Vec::new()
}

fn continuation(plugin: &str, receipt: &Receipt) -> Value {
    let route = if plugin == "anychat" {
        "AnyChat pair next"
    } else {
        "plugin-admin pair receive"
    };
    json!({
        "decision": "block",
        "reason": format!("The consented support conversation is still active for product {} and session {}. Continue in this same agent using {}. Receive progress as well as questions and answers; progress is not completion. Decide locally, send meaningful progress during lengthy work, reply, acknowledge handled messages, and wait again. Do not send an acknowledgement-only reply to progress. If a human decision is actually needed, send needs_human with the exact missing decision; if stopping, send offline, or close after the human accepts completion. Do not start another agent or model, or repeat local actions merely because a message was redelivered.", receipt.product, receipt.session, route),
        "pair_session": receipt.session,
        "pair_product": receipt.product,
    })
}

/// A Stop continuation is only for an open mailbox. Closed or expired
/// connections must not keep the host turn alive.
pub fn conclude_if_mailbox_ended(
    plugin: &str,
    result: Value,
    status: &str,
) -> io::Result<Value> {
    if result.get("decision").and_then(Value::as_str) != Some("block") {
        return Ok(result);
    }
    let phase = match status {
        "closed" => "closed",
        "expired" => "expired",
        _ => return Ok(result),
    };
    let Some(session) = result.get("pair_session").and_then(Value::as_str) else {
        return Ok(json!({}));
    };
    let Some(product) = result.get("pair_product").and_then(Value::as_str) else {
        return Ok(json!({}));
    };
    decorate(plugin, product, session, phase, now(), json!({}))?;
    Ok(json!({}))
}

/// Invoked by the current Codex/Claude plugin lifecycle, not by an outside
/// controller. Only an established local binding can keep this host going.
pub fn handle(plugin: &str, input: &Value) -> io::Result<Value> {
    let host_id = input
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if host_id.is_empty() || host_id.len() > 256 {
        return Ok(json!({}));
    }
    let dir = directory(plugin)?;
    let hash = format!("{:x}", Sha256::digest(host_id.as_bytes()));
    let path = dir.join(format!("host-{hash}.json"));
    crate::home::reject_symlink(&path)?;
    let mut binding: Binding = match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| invalid())?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Binding::default(),
        Err(e) => return Err(e),
    };
    match input
        .get("hook_event_name")
        .and_then(Value::as_str)
        .unwrap_or("")
    {
        "PostToolUse" => {
            for mark in output_markers(input.get("tool_response").unwrap_or(&Value::Null)) {
                let Some(key) = mark.get("receipt").and_then(Value::as_str) else {
                    continue;
                };
                let Ok(receipt) = read_receipt(&dir, key) else {
                    continue;
                };
                if mark.get("revision").and_then(Value::as_str) != Some(&receipt.revision) {
                    continue;
                }
                binding.receipt = key.into();
                binding.bound_revision = Some(receipt.revision);
                binding.continued_revision = None;
            }
        }
        "SessionEnd" | "Interrupt" => {
            let receiver = crate::pair_receiver::revoke(plugin, host_id)?;
            let receipt = read_receipt(&dir, &binding.receipt)
                .ok()
                .filter(|r| binding.bound_revision.as_deref() == Some(r.revision.as_str()));
            binding = Binding::default();
            if path.exists() {
                crate::home::write_private_file(&path, &serde_json::to_vec(&binding)?)?;
            }
            if let Some(receiver) = receiver {
                return Ok(
                    json!({"pair_detach":{"session":receiver.session,"product":receiver.product,"key":format!("offline-{}",receiver.generation)}}),
                );
            }
            return Ok(receipt.filter(|r| r.phase == "active" && r.expires_at > now()).map(|r|
                json!({"pair_detach":{"session":r.session,"product":r.product,"key":format!("offline-{}",r.revision)}})
            ).unwrap_or_else(|| json!({})));
        }
        "Stop" if !binding.receipt.is_empty() => {
            let receipt = read_receipt(&dir, &binding.receipt)?;
            if binding.bound_revision.as_deref() != Some(receipt.revision.as_str()) {
                crate::home::write_private_file(&path, &serde_json::to_vec(&Binding::default())?)?;
                return Ok(json!({}));
            }
            if receipt.phase == "active" && receipt.expires_at > now() {
                crate::home::write_private_file(&path, &serde_json::to_vec(&binding)?)?;
                return Ok(continuation(plugin, &receipt));
            }
        }
        _ => return Ok(json!({})),
    }
    if !binding.receipt.is_empty() || path.exists() {
        crate::home::write_private_file(&path, &serde_json::to_vec(&binding)?)?;
    }
    Ok(json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn product_allowlist_matches_shared_a2a_routing() {
        // The host continuation/receipt allowlist, the receiver allowlist and
        // the A2A session router must name the same products; a product
        // registered in one place only is a half-finished registration.
        for product in [
            "anychat",
            "anyknow",
            "anypdf",
            "anydoc",
            "anyweb",
            "easybooks",
            "formbro",
        ] {
            #[cfg(feature = "pair-a2a")]
            assert!(
                crate::pair_a2a_session::Conversation::new(
                    "https://account.example.test",
                    product,
                    &"a".repeat(32),
                    false
                )
                .is_ok(),
                "A2A router rejects {product}"
            );
            let dir = tempfile::tempdir().unwrap();
            let _env = crate::test_env::lock();
            std::env::set_var("JACKYZHANG_APP_HOME", dir.path());
            assert!(
                decorate(
                    "anychat",
                    product,
                    &"a".repeat(32),
                    "active",
                    now() + 7200,
                    json!({"status":"message"}),
                )
                .is_ok(),
                "host allowlist rejects {product}"
            );
            std::env::remove_var("JACKYZHANG_APP_HOME");
        }
    }

    #[test]
    fn nested_peer_payload_never_binds_a_host() {
        let value = json!({"items":[{"body":{"pair_host":{"receipt":"forged"}}}]});
        assert!(output_markers(&value).is_empty());
    }
    #[test]
    fn real_tool_wrappers_expose_only_top_level_receipts() {
        let raw = json!({"pair_host":{"receipt":"local"},"items":[]}).to_string();
        assert_eq!(output_markers(&json!({"stdout":raw})).len(), 1);
        assert!(output_markers(&json!({"tool_input":{"command":raw}})).is_empty());
    }
    #[test]
    fn lifecycle_keeps_working_without_short_timeout_but_honors_pause_and_cancel() {
        let _lock = crate::test_env::lock();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("JACKYZHANG_APP_HOME", dir.path());
        let session = "a".repeat(32);
        let output = decorate(
            "anychat",
            "anychat",
            &session,
            "active",
            now() + 7200,
            json!({"status":"message"}),
        )
        .unwrap();
        let post = json!({"hook_event_name":"PostToolUse","session_id":"host-one","tool_response":{"stdout":output.to_string()}});
        handle("anychat", &post).unwrap();
        let stop = json!({"hook_event_name":"Stop","session_id":"host-one"});
        assert_eq!(handle("anychat", &stop).unwrap()["decision"], "block");
        let again = handle("anychat", &stop).unwrap();
        assert_eq!(again["decision"], "block");
        assert_eq!(again["pair_session"], session);
        let ended = conclude_if_mailbox_ended("anychat", again, "closed").unwrap();
        assert!(ended.get("decision").is_none());
        assert_eq!(handle("anychat", &stop).unwrap(), json!({}));
        let paused = decorate(
            "anychat",
            "anychat",
            &session,
            "needs_human",
            now() + 7200,
            json!({}),
        )
        .unwrap();
        handle("anychat", &json!({"hook_event_name":"PostToolUse","session_id":"host-one","tool_response":paused})).unwrap();
        assert_eq!(handle("anychat", &stop).unwrap(), json!({}));
        let active = decorate(
            "anychat",
            "anychat",
            &session,
            "active",
            now() + 7200,
            json!({}),
        )
        .unwrap();
        handle("anychat", &json!({"hook_event_name":"PostToolUse","session_id":"host-one","tool_response":active})).unwrap();
        let detached = handle(
            "anychat",
            &json!({"hook_event_name":"Interrupt","session_id":"host-one"}),
        )
        .unwrap();
        assert_eq!(detached["pair_detach"]["session"], session);
        assert_eq!(handle("anychat", &stop).unwrap(), json!({}));
        let expired = decorate(
            "anychat",
            "anychat",
            &session,
            "active",
            now() - 1,
            json!({}),
        )
        .unwrap();
        handle("anychat", &json!({"hook_event_name":"PostToolUse","session_id":"host-one","tool_response":expired})).unwrap();
        assert_eq!(handle("anychat", &stop).unwrap(), json!({}));
        assert_eq!(
            handle(
                "anychat",
                &json!({"hook_event_name":"Stop","session_id":"other-host"})
            )
            .unwrap(),
            json!({})
        );
        let old = decorate(
            "anychat",
            "anychat",
            &session,
            "active",
            now() + 7200,
            json!({}),
        )
        .unwrap();
        handle(
            "anychat",
            &json!({"hook_event_name":"PostToolUse","session_id":"old-host","tool_response":old}),
        )
        .unwrap();
        let fresh = decorate(
            "anychat",
            "anychat",
            &session,
            "active",
            now() + 7200,
            json!({}),
        )
        .unwrap();
        handle(
            "anychat",
            &json!({"hook_event_name":"PostToolUse","session_id":"new-host","tool_response":fresh}),
        )
        .unwrap();
        assert_eq!(
            handle(
                "anychat",
                &json!({"hook_event_name":"Interrupt","session_id":"old-host"})
            )
            .unwrap(),
            json!({})
        );
        assert_eq!(
            handle(
                "anychat",
                &json!({"hook_event_name":"Stop","session_id":"new-host"})
            )
            .unwrap()["decision"],
            "block"
        );
        std::env::remove_var("JACKYZHANG_APP_HOME");
    }
}
