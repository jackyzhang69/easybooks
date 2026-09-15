//! Thin ureq wrapper — every non-2xx is an error; body excerpts redact secrets.

use serde::de::DeserializeOwned;
use serde::Serialize;
use thiserror::Error;
use ureq::{Agent, AgentBuilder};

const REDACTED: &str = "[REDACTED]";

#[derive(Debug, Error)]
pub enum HttpError {
    #[error("HTTP {code}: {body_excerpt}")]
    Status { code: u16, body_excerpt: String },
    #[error("network error: {0}")]
    Transport(String),
    #[error("response decode failed: {0}")]
    Decode(String),
}

#[derive(Debug, Clone)]
pub struct Response<T> {
    pub status: u16,
    pub body: T,
}

pub fn agent() -> Agent {
    AgentBuilder::new().redirects(0).build()
}

/// Build an agent for a bounded operation such as the pair mailbox long poll.
/// The default agent keeps the historical timeout behavior for ordinary
/// product calls; callers that need a deadline opt in here.
pub fn agent_with_timeouts(read_timeout: std::time::Duration) -> Agent {
    AgentBuilder::new()
        .redirects(0)
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout_read(read_timeout)
        .timeout_write(std::time::Duration::from_secs(10))
        .build()
}

pub fn send_json<T, B>(
    method: &str,
    url: &str,
    bearer: Option<&str>,
    body: Option<&B>,
) -> Result<Response<T>, HttpError>
where
    T: DeserializeOwned,
    B: Serialize,
{
    send_json_with_agent(&agent(), method, url, bearer, body)
}

/// Send JSON using a caller-selected HTTP agent. This keeps bounded long-poll
/// timeouts local to the operation that needs them.
pub fn send_json_with_agent<T, B>(
    client: &Agent,
    method: &str,
    url: &str,
    bearer: Option<&str>,
    body: Option<&B>,
) -> Result<Response<T>, HttpError>
where
    T: DeserializeOwned,
    B: Serialize,
{
    let mut request = client.request(method, url);
    request = request
        .set("Accept", "application/json")
        .set("Accept-Encoding", "identity");
    if let Some(token) = bearer {
        request = request.set("Authorization", &format!("Bearer {token}"));
    }
    let response = match body {
        Some(value) => request
            .set("Content-Type", "application/json")
            .send_json(value),
        None => request.call(),
    }
    .map_err(map_ureq_error)?;
    let status = response.status();
    let text = response
        .into_string()
        .map_err(|error| HttpError::Transport(format!("response body read failed: {error}")))?;
    if !(200..300).contains(&status) {
        return Err(HttpError::Status {
            code: status,
            body_excerpt: redact_body_excerpt(&text),
        });
    }
    let parsed: T = serde_json::from_str(&text).map_err(|error| {
        HttpError::Decode(format!(
            "JSON decode: {error}; excerpt={}",
            redact_body_excerpt(&text)
        ))
    })?;
    Ok(Response {
        status,
        body: parsed,
    })
}

/// Send JSON with a bounded read timeout. Used by the pair wait loop only.
pub fn send_json_with_timeout<T, B>(
    method: &str,
    url: &str,
    bearer: Option<&str>,
    body: Option<&B>,
    read_timeout: std::time::Duration,
) -> Result<Response<T>, HttpError>
where
    T: DeserializeOwned,
    B: Serialize,
{
    send_json_with_agent(
        &agent_with_timeouts(read_timeout),
        method,
        url,
        bearer,
        body,
    )
}

pub fn send_json_value(
    method: &str,
    url: &str,
    bearer: Option<&str>,
    body: Option<&serde_json::Value>,
) -> Result<Response<serde_json::Value>, HttpError> {
    send_json(method, url, bearer, body)
}

pub fn post_json_value(
    url: &str,
    bearer: Option<&str>,
    body: &serde_json::Value,
) -> Result<Response<serde_json::Value>, HttpError> {
    send_json_value("POST", url, bearer, Some(body))
}

/// Send a request that must return HTTP 204 with an empty body. A malformed or
/// unexpected successful response is an error; it is never treated as an ack.
pub fn send_empty_value(
    method: &str,
    url: &str,
    bearer: Option<&str>,
    body: Option<&serde_json::Value>,
) -> Result<Response<()>, HttpError> {
    send_empty_value_with_agent(&agent(), method, url, bearer, body)
}

/// Send an empty-response request with an operation-local timeout. A pair
/// acknowledgement is a bounded network operation just like a long poll.
pub fn send_empty_value_with_timeout(
    method: &str,
    url: &str,
    bearer: Option<&str>,
    body: Option<&serde_json::Value>,
    read_timeout: std::time::Duration,
) -> Result<Response<()>, HttpError> {
    send_empty_value_with_agent(
        &agent_with_timeouts(read_timeout),
        method,
        url,
        bearer,
        body,
    )
}

fn send_empty_value_with_agent(
    client: &Agent,
    method: &str,
    url: &str,
    bearer: Option<&str>,
    body: Option<&serde_json::Value>,
) -> Result<Response<()>, HttpError> {
    let mut request = client.request(method, url);
    request = request
        .set("Accept", "application/json")
        .set("Accept-Encoding", "identity");
    if let Some(token) = bearer {
        request = request.set("Authorization", &format!("Bearer {token}"));
    }
    let response = match body {
        Some(value) => request
            .set("Content-Type", "application/json")
            .send_json(value),
        None => request.call(),
    }
    .map_err(map_ureq_error)?;
    let status = response.status();
    if status == 204
        && (response.header("Transfer-Encoding").is_some()
            || response
                .header("Content-Length")
                .is_some_and(|length| length.parse::<u64>() != Ok(0)))
    {
        return Err(HttpError::Decode(
            "acknowledgement has unexpected body framing".into(),
        ));
    }
    let text = response
        .into_string()
        .map_err(|error| HttpError::Transport(format!("response body read failed: {error}")))?;
    if status != 204 {
        return Err(HttpError::Status {
            code: status,
            body_excerpt: redact_body_excerpt(&text),
        });
    }
    if !text.is_empty() {
        return Err(HttpError::Decode(
            "expected an empty HTTP 204 response".into(),
        ));
    }
    Ok(Response { status, body: () })
}

fn map_ureq_error(error: ureq::Error) -> HttpError {
    match error {
        ureq::Error::Status(code, response) => {
            let text = response.into_string().unwrap_or_default();
            HttpError::Status {
                code,
                body_excerpt: redact_body_excerpt(&text),
            }
        }
        ureq::Error::Transport(transport) => {
            HttpError::Transport(format!("{:?}", transport.kind()))
        }
    }
}

pub fn redact_body_excerpt(text: &str) -> String {
    let mut out = text.chars().take(512).collect::<String>();
    if text.chars().count() > 512 {
        out.push('…');
    }
    redact_secrets(&out)
}

/// Shared §4 scanner: secrets/tokens in envelope-facing text.
pub fn contains_forbidden_secret(text: &str) -> bool {
    if text.contains("jz_") || text.contains("Bearer ") {
        return true;
    }
    if text.contains("eyJ") {
        return true;
    }
    contains_jwt_shape(text)
}

pub fn looks_like_absolute_path(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.starts_with('/')
        || trimmed.starts_with("~/")
        || trimmed.starts_with("./")
        || trimmed.starts_with("../")
        || trimmed.starts_with("\\\\")
        || trimmed
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
            && trimmed.get(1..=2) == Some(":\\")
}

pub fn redact_secrets(input: &str) -> String {
    let mut out = input.to_string();
    while let Some(start) = out.find("Bearer ") {
        let rest = &out[start + 7..];
        if rest.starts_with(REDACTED) {
            break;
        }
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '"')
            .unwrap_or(rest.len());
        if end == 0 {
            break;
        }
        out.replace_range(start..start + 7 + end, "Bearer [REDACTED]");
    }
    for prefix in ["jz_", "eyJ"] {
        while let Some(idx) = out.find(prefix) {
            let tail = &out[idx..];
            let len = tail
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '"' && *c != '\'' && *c != ',')
                .count();
            if len == 0 {
                break;
            }
            out.replace_range(idx..idx + len, REDACTED);
        }
    }
    redact_jwt_shapes(&mut out);
    out
}

fn redact_jwt_shapes(out: &mut String) {
    loop {
        let chars: Vec<char> = out.chars().collect();
        let mut replaced = false;
        for index in 0..chars.len() {
            if let Some(len) = jwt_token_len(&chars[index..]) {
                let token: String = chars[index..index + len].iter().collect();
                if !token.contains(REDACTED) {
                    out.replace_range(index..index + len, REDACTED);
                    replaced = true;
                    break;
                }
            }
        }
        if !replaced {
            break;
        }
    }
}

fn jwt_token_len(chars: &[char]) -> Option<usize> {
    let mut dots = 0;
    let mut length = 0;
    for &ch in chars {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            length += 1;
            continue;
        }
        if ch == '.' && dots < 2 {
            dots += 1;
            length += 1;
            continue;
        }
        break;
    }
    if dots == 2 && length > 6 {
        Some(length)
    } else {
        None
    }
}

fn contains_jwt_shape(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    (0..chars.len()).any(|index| jwt_token_len(&chars[index..]).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_jz_token_in_body_excerpt() {
        let body = r#"{"error":"bad","token":"jz_abc123secret"}"#;
        let excerpt = redact_body_excerpt(body);
        assert!(!excerpt.contains("jz_abc123secret"));
        assert!(excerpt.contains(REDACTED));
    }

    #[test]
    fn http_500_redacts_token_in_error() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/fail")
            .with_status(500)
            .with_body(r#"{"detail":"refused jz_supersecret_token"}"#)
            .create();
        let url = format!("{}/fail", server.url());
        let err = send_json_value("GET", &url, None, None).unwrap_err();
        mock.assert();
        match err {
            HttpError::Status { body_excerpt, .. } => {
                assert!(!body_excerpt.contains("jz_supersecret_token"));
            }
            other => panic!("expected status error, got {other:?}"),
        }
    }

    #[test]
    fn redacts_bearer_and_jwt_shapes_from_display() {
        let jwt = "abc.def.ghi";
        let body = format!(r#"{{"authorization":"Bearer eyJsecret.payload.sig","other":"{jwt}"}}"#);
        let err = HttpError::Status {
            code: 500,
            body_excerpt: redact_body_excerpt(&body),
        };
        let rendered = err.to_string();
        assert!(!rendered.contains("eyJsecret"));
        assert!(!rendered.contains("Bearer eyJ"));
        assert!(!rendered.contains("abc.def.ghi"));
        if let HttpError::Status { body_excerpt, .. } = err {
            assert!(!body_excerpt.contains("eyJsecret"));
            assert!(!body_excerpt.contains("abc.def.ghi"));
        }
    }

    #[test]
    fn and_or_is_not_treated_as_path() {
        assert!(!looks_like_absolute_path("Choose Continue and/or Cancel."));
    }

    #[test]
    fn empty_ack_requires_exact_204_without_a_body() {
        let mut server = mockito::Server::new();
        let ok = server.mock("POST", "/read").with_status(204).create();
        let url = format!("{}/read", server.url());
        let response = send_empty_value("POST", &url, None, Some(&serde_json::json!({})))
            .expect("204 acknowledgement");
        assert_eq!(response.status, 204);
        ok.assert();

        let bad = server
            .mock("POST", "/read-body")
            .with_status(200)
            .with_body("unexpected")
            .create();
        let url = format!("{}/read-body", server.url());
        assert!(matches!(
            send_empty_value("POST", &url, None, Some(&serde_json::json!({}))),
            Err(HttpError::Status { code: 200, .. })
        ));
        bad.assert();

        let bad_body = server
            .mock("POST", "/read-204-body")
            .with_status(204)
            .with_header("content-length", "10")
            .with_body("unexpected")
            .create();
        let url = format!("{}/read-204-body", server.url());
        assert!(matches!(
            send_empty_value("POST", &url, None, Some(&serde_json::json!({}))),
            Err(HttpError::Decode(_))
        ));
        bad_body.assert();
    }
}
