//! User-plane pair-session client (`contracts/pair-session-v1.md`).
//!
//! Admin HTTP stays in plugin-admin. This module never prints tokens.

use crate::auth::{self, AuthError};
use crate::identity::PluginIdentity;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use thiserror::Error;

pub const MAX_MESSAGE_RUNES: usize = 2_000;
const MAX_WAIT_SECONDS: u64 = 25;
const RETRY_BACKOFF: [Duration; 4] = [
    Duration::from_millis(250),
    Duration::from_millis(500),
    Duration::from_secs(1),
    Duration::from_secs(2),
];

#[derive(Debug, Error)]
pub enum PairError {
    #[error("{0}")]
    Auth(AuthError),
    #[error("pair session not found")]
    NotFound,
    #[error("pair session conflict")]
    Conflict,
    #[error("pair session expired")]
    Expired,
    #[error("pair session closed")]
    Closed,
    #[error("pair receive timed out")]
    Timeout,
    #[error("{0}")]
    Rejected(String),
}

fn map_status(err: AuthError) -> PairError {
    match err {
        error @ AuthError::Http(crate::http::HttpError::Status {
            code: 408 | 425 | 429 | 500..=599,
            ..
        }) => PairError::Auth(error),
        AuthError::Http(crate::http::HttpError::Status { code: 404, .. }) => PairError::NotFound,
        AuthError::Http(crate::http::HttpError::Status {
            code: 409,
            body_excerpt,
        }) => {
            if body_excerpt.contains("expired") {
                PairError::Expired
            } else if body_excerpt.contains("closed") {
                PairError::Closed
            } else {
                PairError::Conflict
            }
        }
        AuthError::Http(crate::http::HttpError::Status { body_excerpt, .. }) => {
            PairError::Rejected(body_excerpt)
        }
        other => PairError::Auth(other),
    }
}

impl PairError {
    /// Network overload and a dropped connection may be retried with the
    /// same idempotency key. Auth, contract, identity and terminal-session
    /// errors must return to the calling agent immediately.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            PairError::Auth(AuthError::Http(
                crate::http::HttpError::Transport(_)
                    | crate::http::HttpError::Status {
                        code: 408 | 425 | 429 | 500..=599,
                        ..
                    }
            ))
        )
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PairSession {
    pub id: String,
    pub product: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairMessage {
    pub id: String,
    pub session_id: String,
    pub from_role: String,
    pub kind: String,
    pub body: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Items<T> {
    items: Vec<T>,
}

fn product_url(identity: &PluginIdentity, accountd_base: &str, suffix: &str) -> String {
    let base = accountd_base.trim_end_matches('/');
    format!("{base}/v1/products/{}{suffix}", identity.plugin_id)
}

fn validate_session_response(
    identity: &PluginIdentity,
    session: &PairSession,
) -> Result<(), PairError> {
    validate_identifier(&session.id)?;
    if session.product != identity.plugin_id {
        return Err(PairError::Rejected("pair product is invalid".into()));
    }
    if !matches!(
        session.status.as_str(),
        "waiting" | "open" | "closed" | "expired"
    ) {
        return Err(PairError::Rejected("pair session status is invalid".into()));
    }
    Ok(())
}

/// Validate a server envelope before handing it to a host agent. The body is
/// still untrusted data; this only prevents malformed identifiers or kinds
/// from crossing the client boundary.
pub fn validate_message(message: &PairMessage, expected_session_id: &str) -> Result<(), PairError> {
    validate_identifier(&message.id)?;
    validate_identifier(&message.session_id)?;
    validate_identifier(expected_session_id)?;
    if message.session_id != expected_session_id {
        return Err(PairError::Rejected(
            "pair message session is invalid".into(),
        ));
    }
    if !matches!(message.from_role.as_str(), "user" | "admin") {
        return Err(PairError::Rejected("pair message role is invalid".into()));
    }
    if !matches!(
        message.kind.as_str(),
        "snapshot"
            | "result"
            | "ask_run"
            | "ask_continue"
            | "ask_say"
            | "ask_human"
            | "diagnosis"
            | "message"
    ) || !message.body.is_object()
    {
        return Err(PairError::Rejected(
            "pair message envelope is invalid".into(),
        ));
    }
    if message.kind == "message" {
        let object = message
            .body
            .as_object()
            .filter(|object| {
                object
                    .keys()
                    .all(|key| matches!(key.as_str(), "text" | "phase" | "reply_to"))
            })
            .ok_or_else(|| PairError::Rejected("message body is invalid".into()))?;
        let text = object
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| PairError::Rejected("message body is invalid".into()))?;
        validate_message_text(text)?;
        if let Some(phase) = object.get("phase") {
            validate_phase(phase.as_str().unwrap_or(""))?;
        }
        if let Some(reply) = object.get("reply_to") {
            validate_identifier(reply.as_str().unwrap_or(""))?;
        }
    }
    Ok(())
}

/// Conversation metadata conveys observations, never remote execution authority.
pub fn validate_phase(phase: &str) -> Result<(), PairError> {
    if matches!(
        phase,
        "question" | "progress" | "answer" | "needs_human" | "offline"
    ) {
        Ok(())
    } else {
        Err(PairError::Rejected("invalid conversation phase".into()))
    }
}

pub fn conversation_body(
    text: &str,
    phase: &str,
    reply_to: Option<&str>,
) -> Result<Value, PairError> {
    validate_message_text(text)?;
    validate_phase(phase)?;
    let mut body = json!({"text": text, "phase": phase});
    if let Some(id) = reply_to {
        validate_identifier(id)?;
        body["reply_to"] = json!(id);
    }
    Ok(body)
}

pub fn validate_identifier(value: &str) -> Result<(), PairError> {
    if value.len() != 32 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(PairError::Rejected("pair identifier is invalid".into()));
    }
    Ok(())
}

pub fn validate_idempotency_key(value: &str) -> Result<(), PairError> {
    if value.is_empty()
        || value.chars().count() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
        || crate::http::contains_forbidden_secret(value)
    {
        return Err(PairError::Rejected("idempotency key is invalid".into()));
    }
    Ok(())
}

/// Validate the only free-text transport body. The server repeats these
/// checks as the canonical boundary; the client fails closed before putting
/// user input on the wire.
pub fn validate_message_text(value: &str) -> Result<(), PairError> {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    if trimmed.is_empty()
        || trimmed.chars().count() > MAX_MESSAGE_RUNES
        || crate::http::contains_forbidden_secret(trimmed)
        || contains_remote_authority(trimmed)
        || lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("www.")
        || trimmed.split_whitespace().any(|part| {
            crate::http::looks_like_absolute_path(part.trim_matches(|c: char| {
                matches!(c, ',' | '.' | ':' | ';' | ')' | ']' | '}' | '"' | '\'')
            }))
        })
    {
        return Err(PairError::Rejected(
            "message is outside the pair support contract".into(),
        ));
    }
    Ok(())
}

/// Message text is an untrusted request or observation. Keep explicit
/// execution channels, continuation tokens and shell syntax out of this
/// role-neutral kind while leaving ordinary requests for supported outcomes
/// available to the receiving host agent.
pub fn contains_remote_authority(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    [
        "resume_token",
        "resume token",
        "argv",
        "command line",
        "shell command",
        "execute command",
        "command execution",
        "execute code",
        "code execution",
        "run code",
        "run a command",
        "remote command",
        "remote execution",
        "powershell",
        "cmd.exe",
        "bash -c",
        "sh -c",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
        || value.contains("$(")
        || value.contains('`')
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionRequest {
    pub id: String,
    pub product: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub created_at: i64,
    pub expires_at: i64,
}

impl ConnectionRequest {
    pub fn validate(&self, product: &str) -> Result<(), PairError> {
        validate_identifier(&self.id)?;
        if self.product != product
            || !matches!(self.status.as_str(), "pending" | "approved" | "expired")
            || self.created_at <= 0
            || self.expires_at <= self.created_at
        {
            return Err(PairError::Rejected(
                "connection request response is invalid".into(),
            ));
        }
        if let Some(id) = &self.session_id {
            validate_identifier(id)?;
        }
        if (self.status == "approved" && self.session_id.is_none())
            || (self.status == "pending" && self.session_id.is_some())
        {
            return Err(PairError::Rejected(
                "connection request response is invalid".into(),
            ));
        }
        Ok(())
    }
}

pub fn request_connection(
    identity: &PluginIdentity,
    base: &str,
    key: &str,
) -> Result<ConnectionRequest, PairError> {
    validate_idempotency_key(key)?;
    let url = product_url(identity, base, "/pair/requests");
    let response = auth::post_product_with_timeout::<Value, ConnectionRequest>(
        identity,
        base,
        &url,
        &json!({"user_confirmed":true,"request_key":key}),
        Duration::from_secs(30),
    )
    .map_err(map_status)?;
    response.body.validate(identity.plugin_id)?;
    Ok(response.body)
}

pub fn wait_connection(
    identity: &PluginIdentity,
    base: &str,
    id: &str,
    seconds: u64,
) -> Result<ConnectionRequest, PairError> {
    validate_identifier(id)?;
    if seconds > 25 {
        return Err(PairError::Rejected("wait duration is invalid".into()));
    }
    let url = product_url(
        identity,
        base,
        &format!("/pair/requests/{id}/status?wait_seconds={seconds}"),
    );
    let response = auth::get_product_with_timeout::<ConnectionRequest>(
        identity,
        base,
        &url,
        Duration::from_secs(seconds + 5),
    )
    .map_err(map_status)?;
    response.body.validate(identity.plugin_id)?;
    if response.body.id != id {
        return Err(PairError::Rejected(
            "connection request response is invalid".into(),
        ));
    }
    Ok(response.body)
}

pub fn join(
    identity: &PluginIdentity,
    accountd_base: &str,
    code: &str,
) -> Result<PairSession, PairError> {
    let url = product_url(identity, accountd_base, "/pair/join");
    let body = json!({ "code": code, "user_confirmed": true });
    match auth::post_product_with_timeout::<Value, PairSession>(
        identity,
        accountd_base,
        &url,
        &body,
        Duration::from_secs(30),
    ) {
        Ok(resp) => {
            validate_session_response(identity, &resp.body)?;
            Ok(resp.body)
        }
        Err(err) => Err(map_status(err)),
    }
}

pub fn current(identity: &PluginIdentity, accountd_base: &str) -> Result<PairSession, PairError> {
    let url = product_url(identity, accountd_base, "/pair");
    match auth::get_product_with_timeout::<PairSession>(
        identity,
        accountd_base,
        &url,
        Duration::from_secs(30),
    ) {
        Ok(resp) => {
            validate_session_response(identity, &resp.body)?;
            Ok(resp.body)
        }
        Err(err) => Err(map_status(err)),
    }
}

pub fn post(
    identity: &PluginIdentity,
    accountd_base: &str,
    session_id: &str,
    kind: &str,
    body: Value,
    idempotency_key: &str,
) -> Result<PairMessage, PairError> {
    validate_identifier(session_id)?;
    validate_idempotency_key(idempotency_key)?;
    if kind == "message" {
        let object = body
            .as_object()
            .filter(|object| {
                object
                    .keys()
                    .all(|key| matches!(key.as_str(), "text" | "phase" | "reply_to"))
            })
            .ok_or_else(|| PairError::Rejected("message body is invalid".into()))?;
        let text = object
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| PairError::Rejected("message body is invalid".into()))?;
        validate_message_text(text)?;
        if let Some(phase) = object.get("phase") {
            validate_phase(phase.as_str().unwrap_or(""))?;
        }
        if let Some(reply) = object.get("reply_to") {
            validate_identifier(reply.as_str().unwrap_or(""))?;
        }
    }
    let url = product_url(
        identity,
        accountd_base,
        &format!("/pair/{session_id}/messages"),
    );
    let payload = json!({
        "kind": kind,
        "body": body,
        "idempotency_key": idempotency_key,
    });
    match auth::post_product_with_timeout::<Value, PairMessage>(
        identity,
        accountd_base,
        &url,
        &payload,
        Duration::from_secs(30),
    ) {
        Ok(resp) => {
            validate_message(&resp.body, session_id)?;
            Ok(resp.body)
        }
        Err(err) => Err(map_status(err)),
    }
}

pub fn unread(
    identity: &PluginIdentity,
    accountd_base: &str,
    session_id: &str,
) -> Result<Vec<PairMessage>, PairError> {
    validate_identifier(session_id)?;
    let url = product_url(
        identity,
        accountd_base,
        &format!("/pair/{session_id}/messages?unread=true&limit=20"),
    );
    match auth::get_product_with_timeout::<Items<PairMessage>>(
        identity,
        accountd_base,
        &url,
        Duration::from_secs(30),
    ) {
        Ok(resp) => {
            for message in &resp.body.items {
                validate_message(message, session_id)?;
            }
            Ok(resp.body.items)
        }
        Err(err) => Err(map_status(err)),
    }
}

/// Bounded long receive. Storage and receiver-side acknowledgement remain the
/// existing pair contract; waiting never consumes a message.
pub fn wait_unread(
    identity: &PluginIdentity,
    accountd_base: &str,
    session_id: &str,
) -> Result<Vec<PairMessage>, PairError> {
    wait_unread_for(identity, accountd_base, session_id, MAX_WAIT_SECONDS)
}

/// One bounded accountd long-poll. The caller owns the overall deadline and
/// may renew this request; a 25-second server cap prevents an unbounded HTTP
/// request while keeping idle agents out of a busy poll loop.
pub fn wait_unread_for(
    identity: &PluginIdentity,
    accountd_base: &str,
    session_id: &str,
    wait_seconds: u64,
) -> Result<Vec<PairMessage>, PairError> {
    validate_identifier(session_id)?;
    if !(1..=MAX_WAIT_SECONDS).contains(&wait_seconds) {
        return Err(PairError::Rejected("wait duration is invalid".into()));
    }
    let url = product_url(
        identity,
        accountd_base,
        &format!("/pair/{session_id}/messages?unread=true&limit=20&wait_seconds={wait_seconds}"),
    );
    match auth::get_product_with_timeout::<Items<PairMessage>>(
        identity,
        accountd_base,
        &url,
        Duration::from_secs(wait_seconds + 5),
    ) {
        Ok(resp) => {
            for message in &resp.body.items {
                validate_message(message, session_id)?;
            }
            Ok(resp.body.items)
        }
        Err(err) => Err(map_status(err)),
    }
}

#[derive(Debug, Clone)]
pub enum WaitOutcome {
    Messages(Vec<PairMessage>),
    TimedOut,
    Closed(PairSession),
    Expired(PairSession),
}

/// Keep the network wait inside the Rust transport. The calling host agent
/// receives one actionable result and decides what local tools or permissions
/// are appropriate; this function never starts an agent or interprets text.
pub fn wait_unread_until(
    identity: &PluginIdentity,
    accountd_base: &str,
    session_id: &str,
    overall_timeout: Duration,
    allow_waiting: bool,
) -> Result<WaitOutcome, PairError> {
    validate_identifier(session_id)?;
    let deadline = Instant::now()
        .checked_add(overall_timeout)
        .ok_or(PairError::Timeout)?;
    let mut retry = 0usize;
    loop {
        let session = match get(identity, accountd_base, session_id) {
            Ok(session) => session,
            Err(error) if error.is_retryable() => {
                if !sleep_before_deadline(deadline, retry) {
                    return Ok(WaitOutcome::TimedOut);
                }
                retry = retry.saturating_add(1);
                continue;
            }
            Err(PairError::Closed) => {
                return Ok(WaitOutcome::Closed(PairSession {
                    id: session_id.into(),
                    product: identity.plugin_id.into(),
                    status: "closed".into(),
                    ..PairSession::default()
                }))
            }
            Err(PairError::Expired) => {
                return Ok(WaitOutcome::Expired(PairSession {
                    id: session_id.into(),
                    product: identity.plugin_id.into(),
                    status: "expired".into(),
                    ..PairSession::default()
                }))
            }
            Err(error) => return Err(error),
        };
        match session.status.as_str() {
            "closed" => return Ok(WaitOutcome::Closed(session)),
            "expired" => return Ok(WaitOutcome::Expired(session)),
            "waiting" if !allow_waiting => return Err(PairError::Conflict),
            "waiting" | "open" => {}
            _ => return Err(PairError::Rejected("pair session status is invalid".into())),
        }
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Ok(WaitOutcome::TimedOut);
        };
        if remaining.is_zero() {
            return Ok(WaitOutcome::TimedOut);
        }
        // The status request consumes a fraction of a second. A one-second
        // caller deadline must still inspect the mailbox at least once.
        let wait_seconds = remaining.as_secs().clamp(1, MAX_WAIT_SECONDS);
        match wait_unread_for(identity, accountd_base, session_id, wait_seconds) {
            Ok(items) if !items.is_empty() => return Ok(WaitOutcome::Messages(items)),
            Ok(_) => {
                retry = 0;
                if Instant::now() >= deadline {
                    return Ok(WaitOutcome::TimedOut);
                }
            }
            Err(error) if error.is_retryable() => {
                if !sleep_before_deadline(deadline, retry) {
                    return Ok(WaitOutcome::TimedOut);
                }
                retry = retry.saturating_add(1);
            }
            Err(PairError::Closed) => {
                return Ok(WaitOutcome::Closed(terminal_session(session, "closed")))
            }
            Err(PairError::Expired) => {
                return Ok(WaitOutcome::Expired(terminal_session(session, "expired")))
            }
            Err(error) => return Err(error),
        }
    }
}

fn terminal_session(mut session: PairSession, status: &str) -> PairSession {
    session.status = status.to_owned();
    session
}

fn sleep_before_deadline(deadline: Instant, attempt: usize) -> bool {
    let delay = RETRY_BACKOFF[attempt.min(RETRY_BACKOFF.len() - 1)];
    let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
        return false;
    };
    if remaining.is_zero() {
        return false;
    }
    std::thread::sleep(delay.min(remaining));
    Instant::now() < deadline
}

pub fn get(
    identity: &PluginIdentity,
    accountd_base: &str,
    session_id: &str,
) -> Result<PairSession, PairError> {
    validate_identifier(session_id)?;
    let url = product_url(identity, accountd_base, &format!("/pair/{session_id}"));
    match auth::get_product_with_timeout::<PairSession>(
        identity,
        accountd_base,
        &url,
        Duration::from_secs(30),
    ) {
        Ok(resp) => {
            validate_session_response(identity, &resp.body)?;
            Ok(resp.body)
        }
        Err(err) => Err(map_status(err)),
    }
}

pub fn mark_read(
    identity: &PluginIdentity,
    accountd_base: &str,
    session_id: &str,
    message_id: &str,
) -> Result<(), PairError> {
    validate_identifier(session_id)?;
    validate_identifier(message_id)?;
    let url = product_url(
        identity,
        accountd_base,
        &format!("/pair/{session_id}/messages/{message_id}/read"),
    );
    match auth::post_product_empty(identity, accountd_base, &url, &json!({})) {
        Ok(_) => Ok(()),
        Err(err) => Err(map_status(err)),
    }
}

pub fn close(
    identity: &PluginIdentity,
    accountd_base: &str,
    session_id: &str,
    reason: &str,
) -> Result<PairSession, PairError> {
    validate_identifier(session_id)?;
    let url = product_url(
        identity,
        accountd_base,
        &format!("/pair/{session_id}/close"),
    );
    let body = json!({ "reason": reason });
    match auth::post_product_with_timeout::<Value, PairSession>(
        identity,
        accountd_base,
        &url,
        &body,
        Duration::from_secs(30),
    ) {
        Ok(resp) => {
            validate_session_response(identity, &resp.body)?;
            Ok(resp.body)
        }
        Err(err) => Err(map_status(err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{AuthMode, PluginIdentity};
    use crate::test_env;
    use serde_json::json;

    const IDENTITY: PluginIdentity = PluginIdentity {
        plugin_id: "anychat",
        aud: Some("anychat"),
        auth_mode: AuthMode::Exchange,
        product_scopes: &["read", "write"],
    };

    fn make_test_jwt(aud: &str, issuer: &str) -> String {
        use base64::Engine as _;
        use std::time::{SystemTime, UNIX_EPOCH};
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"alg":"ES256","typ":"JWT"}"#);
        let exp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs_f64()
            + 120.0;
        let payload = format!(r#"{{"aud":"{aud}","iss":"{issuer}","exp":{exp}}}"#);
        let body = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload);
        format!("{header}.{body}.signature")
    }

    #[test]
    fn join_posts_confirmed_code() {
        test_env::with_home("jz_test_pair", |_, mut server| {
            let issuer = server.url();
            let jwt = make_test_jwt("anychat", &issuer);
            let _ex = server
                .mock("POST", "/v1/token/exchange")
                .with_status(200)
                .with_body(format!(r#"{{"access_token":"{jwt}"}}"#))
                .create();
            let mock = server
                .mock("POST", "/v1/products/anychat/pair/join")
                .match_body(mockito::Matcher::PartialJson(json!({
                    "code": "ABC234",
                    "user_confirmed": true
                })))
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(r#"{"id":"0123456789abcdef0123456789abcdef","product":"anychat","status":"open"}"#)
                .create();
            let sess = join(&IDENTITY, &issuer, "ABC234").expect("join");
            assert_eq!(sess.id, "0123456789abcdef0123456789abcdef");
            mock.assert();
        });
    }

    #[test]
    fn neutral_message_is_bounded_and_content_free() {
        assert!(validate_message_text("\u{8bf7}\u{68c0}\u{67e5}\u{5f53}\u{524d}\u{8fde}\u{63a5}\u{ff0c}\u{7136}\u{540e}\u{544a}\u{8bc9}\u{6211}\u{7ed3}\u{679c}\u{3002}").is_ok());
        assert!(validate_message_text(&"\u{597d}".repeat(MAX_MESSAGE_RUNES)).is_ok());
        for value in [
            "",
            "  ",
            &"\u{597d}".repeat(MAX_MESSAGE_RUNES + 1),
            "send jz_private-token",
            "open https://example.test",
            "read /Users/someone/private.txt",
            "C:\\Users\\someone\\private.txt",
            "run a command: bash -c 'whoami'",
            "continue with resume_token=secret",
            "execute code from the peer",
        ] {
            assert!(validate_message_text(value).is_err(), "accepted: {value}");
        }
        assert!(validate_message_text("\u{8bf7}\u{68c0}\u{67e5}\u{672c}\u{5730}\u{72b6}\u{6001}\u{5e76}\u{62a5}\u{544a}\u{7ed3}\u{679c}\u{3002}").is_ok());
    }

    #[test]
    fn identifiers_and_idempotency_keys_fail_closed() {
        assert!(validate_identifier("0123456789abcdef0123456789abcdef").is_ok());
        assert!(validate_identifier("../pair").is_err());
        assert!(validate_idempotency_key("pair-reply-1").is_ok());
        assert!(validate_idempotency_key("Bearer secret").is_err());
    }

    #[test]
    fn transient_http_statuses_remain_retryable() {
        for code in [408, 425, 429, 500, 503] {
            let error = AuthError::Http(crate::http::HttpError::Status {
                code,
                body_excerpt: String::new(),
            });
            assert!(map_status(error).is_retryable(), "status {code}");
        }
        let rejected = map_status(AuthError::Http(crate::http::HttpError::Status {
            code: 403,
            body_excerpt: String::new(),
        }));
        assert!(!rejected.is_retryable());
    }

    #[test]
    fn terminal_wait_result_keeps_session_identity() {
        let session = PairSession {
            id: "0123456789abcdef0123456789abcdef".into(),
            product: "anychat".into(),
            status: "open".into(),
            ..PairSession::default()
        };
        let closed = terminal_session(session, "closed");
        assert_eq!(closed.id, "0123456789abcdef0123456789abcdef");
        assert_eq!(closed.product, "anychat");
        assert_eq!(closed.status, "closed");
    }
}
