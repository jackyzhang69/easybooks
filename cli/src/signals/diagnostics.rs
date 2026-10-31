//! Bounded reserved Product Signals properties (accountd contract
//! `product-signals-v1.md`, "reserved fields").
//!
//! Local mirror of `jz-plugin-common` `signals::diagnostics` (platform PR #159,
//! including the bounded scrub/truncate loop fix 3ed0bef). The vendored crate
//! pin cannot move yet: the newer crate also changes product-JWT validation,
//! which is outside this module. Delete this file once the pin moves and use the
//! crate's `Event::with_*` builders instead.
//!
//! On top of the shared secret/path scrub, error text gets an EasyBooks-only
//! pass that removes quoted user values, money amounts and long digit runs, so
//! amounts, customer/vendor names, descriptions and account numbers never leave
//! the machine through `error_message` / `error_stack`.

use jz_plugin_common::signals::Event;
use regex::{Captures, Regex};
use serde::Serialize;
use serde_json::{Map, Value};
use std::sync::LazyLock;

#[cfg(test)]
mod conformance;

const RESERVED: [&str; 5] = [
    "error_message",
    "error_stack",
    "metadata",
    "context",
    "steps",
];
const METADATA: [&str; 9] = [
    "os",
    "os_version",
    "arch",
    "plugin_version",
    "client_version",
    "host_app",
    "host_app_version",
    "locale",
    "input_summary",
];
const MAX_STEPS: usize = 50;
const MAX_CAUSES: usize = 16;

/// One timed sub-operation. Invalid durations are dropped, never sent.
#[derive(Debug, Clone, Serialize)]
pub struct Step {
    pub name: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Step {
    pub fn new(name: impl Into<String>, status: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: status.into(),
            duration_ms: None,
            error_code: None,
            detail: None,
        }
    }
    pub fn with_duration_ms(mut self, duration_ms: f64) -> Self {
        self.duration_ms = (duration_ms.is_finite() && duration_ms >= 0.0).then_some(duration_ms);
        self
    }
    pub fn with_error_code(mut self, code: impl Into<String>) -> Self {
        self.error_code = Some(code.into());
        self
    }
}

/// Client facts: no subprocess, network or panic. Unknown facts stay unset
/// (never null). Host app is not detectable from the CLI, so it is omitted.
pub(super) fn metadata(plugin_version: &str) -> Value {
    let mut map = Map::new();
    let os = match std::env::consts::OS {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    };
    map.insert("os".into(), os.into());
    if let Some(version) = os_version() {
        map.insert("os_version".into(), version.into());
    }
    map.insert("arch".into(), std::env::consts::ARCH.into());
    map.insert("plugin_version".into(), plugin_version.into());
    if let Some(locale) = locale() {
        map.insert("locale".into(), locale.into());
    }
    Value::Object(map)
}

fn os_version() -> Option<String> {
    use std::io::Read;
    let path = if cfg!(target_os = "macos") {
        "/System/Library/CoreServices/SystemVersion.plist"
    } else if cfg!(target_os = "linux") {
        "/proc/sys/kernel/osrelease"
    } else {
        return None;
    };
    let mut content = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(8192)
        .read_to_string(&mut content)
        .ok()?;
    if cfg!(target_os = "macos") {
        let (_, tail) = content.split_once("<key>ProductVersion</key>")?;
        let (_, tail) = tail.split_once("<string>")?;
        Some(tail.split_once("</string>")?.0.trim().into())
    } else {
        Some(content.trim().into())
    }
}

/// POSIX locale env (`en_CA.UTF-8` -> `en-CA`); `C`/`POSIX`/unset are omitted.
fn locale() -> Option<String> {
    let raw = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|key| std::env::var(key).ok().filter(|v| !v.trim().is_empty()))?;
    let tag = raw.split(['.', '@']).next()?.trim().replace('_', "-");
    let valid = !tag.is_empty()
        && tag.len() <= 35
        && tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    (valid && tag != "C" && tag != "POSIX").then_some(tag)
}

/// Display plus at most 16 source links, EasyBooks-redacted.
pub(super) fn error_message(error: &anyhow::Error) -> String {
    let mut message = error.to_string();
    for cause in error.chain().skip(1).take(MAX_CAUSES) {
        message.push_str(": ");
        message.push_str(&cause.to_string());
    }
    domain_redact(&message)
}

/// anyhow's report: message, numbered `Caused by:` chain, and the backtrace
/// when `RUST_BACKTRACE`/`RUST_LIB_BACKTRACE` captured one.
pub(super) fn error_stack(error: &anyhow::Error) -> String {
    domain_redact(&format!("{error:?}"))
}

struct Domain {
    remote_error: Regex,
    http_message: Regex,
    quoted: Regex,
    labeled: Regex,
    money: Regex,
    decimal: Regex,
    grouped: Regex,
    digits: Regex,
}
static DOMAIN: LazyLock<Domain> = LazyLock::new(|| {
    fn re(pattern: &str) -> Regex {
        Regex::new(pattern).expect("constant easybooks regex")
    }
    Domain {
        // Formats produced by `crate::client` for backend failures. Server text
        // may span lines; it ends at the client's own suffix, at anyhow's
        // report sections, or at the end of the text.
        remote_error: re(r"(?s)(remote_error: ).*( \(HTTP \d{3}\))"),
        http_message: re(
            r"(?s)(HTTP \d{3}(?: [^:\n)]*\)?)?: ).*?(\n\nCaused by:|\n\nStack backtrace:|\z)",
        ),
        quoted: re(r#""(?:\\.|[^"\\])*""#),
        labeled: re(
            r#"(?i)((?:amount|total|subtotal|balance|price|tax|quantity)[A-Za-z_]*["']?[ \t]*[:=]?[ \t]*)[$€£]?-?\d[\d.,]*"#,
        ),
        money: re(r"[$€£][ \t]?-?\d[\d,]*(?:\.\d+)?"),
        decimal: re(r"\d+(?:\.\d+)+"),
        grouped: re(r"\d{1,3}(?:,\d{3})+(?:\.\d+)?"),
        digits: re(r"\d{6,}"),
    }
});

const SERVER_MESSAGE: &str = "[server message redacted]";

/// Backend error text is free-form and may echo entry/invoice/customer data,
/// so only the HTTP status survives. Local error text is template-built:
/// quoted user values (descriptions, names, raw amounts echoed by `{:?}`),
/// money, single-dot decimals and 6+ digit runs (account numbers, large cents)
/// are removed; dotted versions/IPs (two or more dots) and short integers stay.
fn domain_redact(input: &str) -> String {
    let d = &*DOMAIN;
    // Secrets first: digit redaction would otherwise split a token (e.g.
    // `ghp_0123456789abc…`) so the shared family patterns no longer match it.
    let scrubbed = scrub(input);
    let value = d.remote_error.replace_all(&scrubbed, |c: &Captures<'_>| {
        format!("{}{SERVER_MESSAGE}{}", &c[1], &c[2])
    });
    let value = d.http_message.replace_all(&value, |c: &Captures<'_>| {
        format!("{}{SERVER_MESSAGE}{}", &c[1], &c[2])
    });
    let value = d.quoted.replace_all(&value, "\"[redacted]\"");
    let value = d
        .labeled
        .replace_all(&value, |c: &Captures<'_>| format!("{}[amount]", &c[1]));
    let value = d.money.replace_all(&value, "[amount]");
    let value = d.grouped.replace_all(&value, "[amount]");
    let value = d.decimal.replace_all(&value, |c: &Captures<'_>| {
        if c[0].matches('.').count() == 1 {
            "[amount]".to_string()
        } else {
            c[0].to_string()
        }
    });
    d.digits.replace_all(&value, "[number]").into_owned()
}

/// Replace reserved diagnostics in `event.properties` with their bounded,
/// scrubbed form: exactly what accountd stores, so it never has to reject.
pub(super) fn normalize(event: &mut Event) {
    let error = event
        .error_code
        .as_ref()
        .is_some_and(|s| !s.trim().is_empty())
        || event.event_name.to_ascii_lowercase().ends_with("_failed")
        || event.outcome.as_deref() == Some("failed");
    let raw = std::mem::take(&mut event.properties);
    for (key, value) in raw {
        let normalized = match key.as_str() {
            "error_message" if error => string(&value, 2048),
            "error_stack" if error => string(&value, 8192),
            "metadata" => metadata_value(&value),
            "context" => context(&value),
            "steps" => steps(&value),
            other if RESERVED.iter().any(|name| other.eq_ignore_ascii_case(name)) => None,
            _ => Some(value),
        };
        if let Some(value) = normalized {
            event.properties.insert(key, value);
        }
    }
}

fn string(value: &Value, cap: usize) -> Option<Value> {
    let clean = text(value.as_str()?, cap);
    (!clean.is_empty()).then_some(Value::String(clean))
}
fn metadata_value(raw: &Value) -> Option<Value> {
    let object = raw.as_object()?;
    let result: Map<String, Value> = METADATA
        .iter()
        .filter_map(|key| {
            let cap = if *key == "input_summary" { 512 } else { 128 };
            Some(((*key).into(), string(object.get(*key)?, cap)?))
        })
        .collect();
    (!result.is_empty()).then_some(Value::Object(result))
}
fn sensitive(key: &str) -> bool {
    // Go strings.ToLower uses simple one-codepoint mappings.
    let lower: String = key
        .chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect();
    [
        "password",
        "passwd",
        "token",
        "secret",
        "api_key",
        "api-key",
        "apikey",
        "authorization",
        "auth",
        "private_key",
        "private-key",
        "credential",
        "cookie",
    ]
    .iter()
    .any(|part| lower.contains(part))
}
fn context(raw: &Value) -> Option<Value> {
    raw.as_object()?;
    let mut keys = 0;
    context_value(raw, 1, &mut keys)
}
/// Byte length of Go's `json.Marshal` (escapes `<>&`, U+2028, U+2029).
fn go_json_bytes(value: &Value) -> usize {
    let json = serde_json::to_string(value).unwrap_or_default();
    json.len()
        + json
            .chars()
            .map(|c| match c {
                '<' | '>' | '&' => 5,
                '\u{2028}' | '\u{2029}' => 3,
                _ => 0,
            })
            .sum::<usize>()
}
fn context_value(raw: &Value, depth: usize, keys: &mut usize) -> Option<Value> {
    match raw {
        Value::Object(object) if depth <= 4 => {
            let mut result = Map::new();
            let mut bytes = 2;
            for (key, value) in object {
                if *keys >= 64 {
                    break;
                }
                let clean = text(key, 128);
                if result.contains_key(&clean) {
                    continue;
                }
                *keys += 1;
                let child = if sensitive(key) || sensitive(&clean) {
                    Some(Value::String("[redacted]".into()))
                } else {
                    context_value(value, depth + 1, keys)
                };
                if let Some(child) = child {
                    let added = go_json_bytes(&Value::String(clean.clone()))
                        + 1
                        + go_json_bytes(&child)
                        + usize::from(!result.is_empty());
                    if bytes + added > 16 * 1024 {
                        break;
                    }
                    bytes += added;
                    result.insert(clean, child);
                }
            }
            Some(Value::Object(result))
        }
        Value::Array(array) if depth <= 4 => {
            let mut result = Vec::new();
            let mut bytes = 2;
            for child in array.iter().take(50) {
                if let Some(child) = context_value(child, depth + 1, keys) {
                    let added = go_json_bytes(&child) + usize::from(!result.is_empty());
                    if bytes + added > 16 * 1024 {
                        break;
                    }
                    bytes += added;
                    result.push(child);
                }
            }
            Some(Value::Array(result))
        }
        Value::String(value) => Some(Value::String(text(value, 1024))),
        Value::Bool(_) => Some(raw.clone()),
        Value::Number(number) if number.as_f64().is_some_and(f64::is_finite) => Some(raw.clone()),
        _ => None,
    }
}
fn steps(raw: &Value) -> Option<Value> {
    let array = raw.as_array()?;
    Some(Value::Array(
        array
            .iter()
            .filter_map(|raw| {
                let mut step = Map::new();
                step.insert("name".into(), string(raw.get("name")?, 128)?);
                step.insert("status".into(), string(raw.get("status")?, 32)?);
                for (key, cap) in [("error_code", 128), ("detail", 1024)] {
                    if let Some(value) = raw.get(key).and_then(|v| string(v, cap)) {
                        step.insert(key.into(), value);
                    }
                }
                if let Some(value) = raw
                    .get("duration_ms")
                    .filter(|v| v.as_f64().is_some_and(|n| n.is_finite() && n >= 0.0))
                {
                    step.insert("duration_ms".into(), value.clone());
                }
                Some(Value::Object(step))
            })
            .take(MAX_STEPS)
            .collect(),
    ))
}

// --- shared scrub (same order and patterns as accountd/signals/scrub.go) ----

struct Patterns {
    authorization: Regex,
    assignment: Regex,
    bearer: Regex,
    jwt: Regex,
    family: Regex,
    long_base64: Regex,
    long_url: Regex,
    home: Regex,
    file: Regex,
    email: Regex,
}
static PATTERNS: LazyLock<Patterns> = LazyLock::new(|| {
    fn re(pattern: &str) -> Regex {
        Regex::new(pattern).expect("constant diagnostic regex")
    }
    Patterns {
        authorization: re(
            r"(?i)(authorization[ \t\n\f\r]*[:=][ \t\n\f\r]*)(bearer[ \t\n\f\r]+)?[A-Za-z0-9._\-+/=]+",
        ),
        assignment: re(
            r#"(?i)((?:(?-u:\b)|["'])(?:[A-Za-z0-9_.-]*(?:authorization|bearer|token|secret|password|passwd|credential)[A-Za-z0-9_.-]*|[A-Za-z0-9_.-]*(?:api[_-]?key|private[_-]?key)|key)["']?[ \t\n\f\r]*[:=][ \t\n\f\r]*)("(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|[^ \t\n\f\r"']+)"#,
        ),
        bearer: re(r"(?i)(?-u:\b)bearer[ \t\n\f\r]+[A-Za-z0-9._~+/_-]+=*"),
        jwt: re(r"(?-u:\b)[A-Za-z0-9_-]{3,}\.[A-Za-z0-9_-]{3,}\.[A-Za-z0-9_-]{3,}(?-u:\b)"),
        family: re(
            r"(?i)(?-u:\b)(?:jz_|fb_|sk-|ghp_|github_pat_|xox[a-z0-9-]*-|akia)[A-Za-z0-9._/+=-]{6,}",
        ),
        long_base64: re(r"(?-u:\b)[A-Za-z0-9+/]{32,}(?-u:\b)"),
        long_url: re(r"(?-u:\b)[A-Za-z0-9_-]{32,}(?-u:\b)"),
        home: re(
            r#"(?i)(?:[A-Za-z]:)?(?:\\*/|\\+)(?:Users|home)(?:\\*/|\\+)[^/\\ \t\n\f\r'"<>]+(?:\\*/|\\+)?"#,
        ),
        file: re(r#"(?i)file://[^ \t\n\f\r'"<>]+"#),
        email: re(r"(?i)(?-u:\b)[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}(?-u:\b)"),
    }
});

fn scrub(input: &str) -> String {
    let p = &*PATTERNS;
    let value = p
        .authorization
        .replace_all(input, |c: &Captures<'_>| format!("{}[redacted]", &c[1]));
    let value = p.assignment.replace_all(&value, |c: &Captures<'_>| {
        let value = &c[2];
        if value.starts_with(['\'', '"']) {
            format!("{}{}[redacted]{}", &c[1], &value[..1], &value[..1])
        } else {
            format!("{}[redacted]", &c[1])
        }
    });
    let value = p.bearer.replace_all(&value, "[redacted]");
    let value = p.jwt.replace_all(&value, "[redacted]");
    let value = p.family.replace_all(&value, "[redacted]");
    fn long(c: &Captures<'_>) -> String {
        let mut seen = [false; 256];
        let mut distinct = 0;
        for byte in c[0].bytes() {
            if !seen[byte as usize] {
                seen[byte as usize] = true;
                distinct += 1;
            }
        }
        if distinct >= 8 {
            "[redacted]".into()
        } else {
            c[0].into()
        }
    }
    let value = p.long_base64.replace_all(&value, long);
    let value = p.long_url.replace_all(&value, long);
    let value = p.home.replace_all(&value, "~/");
    let value = p.file.replace_all(&value, "file://<redacted>");
    p.email.replace_all(&value, "<email>").into_owned()
}

fn truncate(mut value: String, cap: usize) -> String {
    let mut end = value.len().min(cap);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    value
}

/// Scrub before UTF-8-safe truncation, returning `v` with
/// `truncate(scrub(v)) == v` (accountd's acceptance check). A cap can cut a
/// replacement (`token=[redacted]` -> `token=[`), so the comparison runs after
/// truncation and iterations are bounded; without a fixed point the value is
/// dropped rather than ever emitting text that still scrubs.
pub(super) fn text(input: &str, cap: usize) -> String {
    let mut clean = truncate(scrub(input), cap);
    for _ in 0..8 {
        let next = truncate(scrub(&clean), cap);
        if next == clean {
            return clean;
        }
        clean = next;
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    fn bounded(input: String, cap: usize) -> String {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(text(&input, cap));
        });
        receiver
            .recv_timeout(Duration::from_secs(10))
            .unwrap_or_else(|_| panic!("text did not terminate for cap {cap}"))
    }

    #[test]
    fn cap_cutting_a_redaction_marker_terminates() {
        // Regression: 055a51e's loop compared before truncating and spun forever.
        let input = format!("{} token=secretvalue", "X".repeat(2035));
        let clean = bounded(input, 2048);
        assert!(clean.len() <= 2048);
        assert!(!clean.contains("secretvalue"));
        assert_eq!(bounded(clean.clone(), 2048), clean);
        assert_eq!(bounded("token=x".into(), 7), "token=[");
    }

    #[test]
    fn every_cap_is_bounded_idempotent_and_secret_free() {
        let corpus = [
            r#"/Users/jacky/a C:\Users\bob\b \/Users\/erin\/c bob@example.com Authorization: Bearer abcdefsecret sk-supersecretvalue ghp_0123456789abcdefghijklmnopqrstuvwx eyJhbGciOi.eyJzdWIiOiIx.c2lnbmF0dXJl password=hunter2 dbPassword=hidden serviceApiKey=hidden"#,
            "界token=界 Authorization: Bearer 界abc key=\"界\" a@b.co 界/home/x/界",
        ];
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            for input in corpus {
                for cap in 0..=input.len() + 16 {
                    let clean = text(input, cap);
                    assert!(clean.len() <= cap, "cap {cap} exceeded");
                    assert_eq!(text(&clean, cap), clean, "not idempotent at cap {cap}");
                    for secret in ["jacky", "bob", "erin", "hunter2", "hidden", "supersecret"] {
                        assert!(!clean.contains(secret), "{secret} survived at cap {cap}");
                    }
                }
            }
            let _ = sender.send(());
        });
        receiver
            .recv_timeout(Duration::from_secs(120))
            .expect("cap sweep panicked or did not terminate");
    }

    #[test]
    fn domain_redaction_removes_amounts_names_and_account_numbers() {
        let local = r#"--amount "1,234.567" is not a valid decimal; vendor "Acme Plumbing Ltd" total=2260.00 $99 1,250 acct 123456789 at 127.0.0.1:8080 v0.5.26 entries[3] line 1"#;
        let clean = domain_redact(local);
        for leaked in ["1,234", "Acme", "2260", "99", "1,250", "123456789"] {
            assert!(!clean.contains(leaked), "{leaked} survived in {clean}");
        }
        for kept in ["127.0.0.1:8080", "0.5.26", "entries[3]", "line 1"] {
            assert!(clean.contains(kept), "{kept} lost in {clean}");
        }
    }

    #[test]
    fn server_messages_keep_only_the_http_status() {
        let cases = [
            (
                "HTTP 422 Unprocessable Entity: vendor Acme Plumbing owes 12",
                "HTTP 422 Unprocessable Entity: [server message redacted]",
            ),
            (
                "Tell Jacky remote_error: client Jane Doe unknown (HTTP 404)",
                "Tell Jacky remote_error: [server message redacted] (HTTP 404)",
            ),
            (
                "parsing JSON response (HTTP 502 Bad Gateway): <html>Jane</html>",
                "parsing JSON response (HTTP 502 Bad Gateway): [server message redacted]",
            ),
            (
                "HTTP 400 Bad Request: Validation failed:\nvendor Acme Plumbing Ltd unknown\n\nStack backtrace:\n   0: easybooks::client",
                "HTTP 400 Bad Request: [server message redacted]\n\nStack backtrace:\n   0: easybooks::client",
            ),
            (
                "Tell Jacky remote_error: line one\nclient Jane Doe (HTTP 409)",
                "Tell Jacky remote_error: [server message redacted] (HTTP 409)",
            ),
        ];
        for (raw, expected) in cases {
            assert_eq!(domain_redact(raw), expected);
        }
    }
}
