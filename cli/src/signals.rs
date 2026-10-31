//! Product Signals fail-open emitter for EasyBooks.
//!
//! Emits `posting_completed` / `invoice_created` (plus a few `emit_named`
//! events) to accountd. Posting and invoice events carry bounded reserved
//! diagnostics: client metadata, timed steps, count-only context and, on
//! failure, the redacted error message/report. Never blocks or fails the
//! user's bookkeeping command.

mod diagnostics;

use crate::config;
use crate::identity::PLUGIN_IDENTITY;
use diagnostics::Step;
use jz_plugin_common::signals::{self, Event};
use serde_json::{Map, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU64 = AtomicU64::new(1);

const PLUGIN_ID: &str = "easybooks";
const EVENT_NAME: &str = "posting_completed";
const PLATFORM_USER_UNKNOWN: &str = "00000000-0000-0000-0000-000000000000";

fn disabled() -> bool {
    if std::env::var("EASYBOOKS_PORTAL_OFFLINE")
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        return true;
    }
    if let Ok(v) = std::env::var("EASYBOOKS_PRODUCT_SIGNALS_ENABLED") {
        return v == "0" || v.eq_ignore_ascii_case("false");
    }
    false
}

fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    let h = rem / 3600;
    let m = (rem % 3600) / 60;
    let s = rem % 60;
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i32 + era as i32 * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Reserved diagnostics for one event. Only counts/kinds belong in
/// `input_summary` and `context`: never amounts, customer/vendor names,
/// descriptions or account numbers.
#[derive(Default)]
struct Diagnostics<'a> {
    error: Option<&'a anyhow::Error>,
    steps: &'a [Step],
    input_summary: Option<&'a str>,
    context: Option<&'a Map<String, Value>>,
}

pub(crate) fn emit_named(
    event_name: &str,
    key: &str,
    outcome: &str,
    feature: &str,
    error_code: Option<&str>,
) {
    if disabled() {
        return;
    }
    emit(build_event(
        event_name,
        key,
        outcome,
        feature,
        error_code,
        Diagnostics::default(),
    ));
}

fn emit(event: Event) {
    let _ = signals::emit_batch(&PLUGIN_IDENTITY, &config::resolve_accountd_url(), &[event]);
}

fn build_event(
    event_name: &str,
    key: &str,
    outcome: &str,
    feature: &str,
    error_code: Option<&str>,
    diag: Diagnostics<'_>,
) -> Event {
    let now = now_rfc3339();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let idempotency_key = format!("easybooks:{key}:{now}:{seq}");
    let mut event = Event::new(
        PLUGIN_ID,
        event_name,
        PLATFORM_USER_UNKNOWN,
        idempotency_key,
    );
    event.occurred_at = now;
    event.outcome = Some(outcome.to_string());
    event.feature_id = Some(feature.to_string());
    event.app_version = Some(env!("CARGO_PKG_VERSION").to_string());
    if let Some(code) = error_code {
        event.error_code = Some(code.to_string());
    }
    let props = &mut event.properties;
    let mut metadata = diagnostics::metadata(env!("CARGO_PKG_VERSION"));
    if let (Some(summary), Value::Object(map)) = (diag.input_summary, &mut metadata) {
        map.insert("input_summary".into(), summary.into());
    }
    props.insert("metadata".into(), metadata);
    if let Some(error) = diag.error {
        props.insert(
            "error_message".into(),
            diagnostics::error_message(error).into(),
        );
        props.insert("error_stack".into(), diagnostics::error_stack(error).into());
    }
    if let Some(context) = diag.context {
        props.insert("context".into(), Value::Object(context.clone()));
    }
    if !diag.steps.is_empty() {
        if let Ok(steps) = serde_json::to_value(diag.steps) {
            props.insert("steps".into(), steps);
        }
    }
    diagnostics::normalize(&mut event);
    event
}

#[derive(Clone, Copy)]
enum OperationKind {
    Posting,
    Invoice,
}

/// Records the timed sub-steps of one posting (`posting_completed`) or invoice
/// create (`invoice_created`) and emits its single outcome event: from the
/// first failing [`Operation::step`], or from [`Operation::succeeded`].
/// Dry runs never emit.
pub struct Operation {
    kind: OperationKind,
    feature: String,
    dry_run: bool,
    steps: Vec<Step>,
    input_summary: Option<String>,
    context: Map<String, Value>,
}

impl Operation {
    pub fn posting(feature: &str, dry_run: bool) -> Self {
        Self::new(OperationKind::Posting, feature, dry_run)
    }

    pub fn invoice(dry_run: bool) -> Self {
        Self::new(OperationKind::Invoice, "invoice", dry_run)
    }

    fn new(kind: OperationKind, feature: &str, dry_run: bool) -> Self {
        Self {
            kind,
            feature: feature.to_string(),
            dry_run,
            steps: Vec::new(),
            input_summary: None,
            context: Map::new(),
        }
    }

    /// Counts/kinds only.
    pub fn summary(&mut self, summary: impl Into<String>) {
        self.input_summary = Some(summary.into());
    }

    /// Counts/kinds/flags only.
    pub fn context(&mut self, key: &str, value: impl Into<Value>) {
        self.context.insert(key.to_string(), value.into());
    }

    /// Run and time one sub-step. On error the step is recorded as failed
    /// with `error_code` and the operation's failure event is emitted with
    /// the error's message and report.
    pub fn step<T>(
        &mut self,
        name: &str,
        error_code: &str,
        run: impl FnOnce() -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let started = Instant::now();
        let result = run();
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        match &result {
            Ok(_) => self.push(Step::new(name, "succeeded").with_duration_ms(elapsed_ms)),
            Err(error) => {
                self.push(
                    Step::new(name, "failed")
                        .with_duration_ms(elapsed_ms)
                        .with_error_code(error_code),
                );
                self.emit("failed", Some(error_code), Some(error));
            }
        }
        result
    }

    /// Attach a count/kind-only detail to the most recent step.
    pub fn detail(&mut self, detail: impl Into<String>) {
        if let Some(step) = self.steps.last_mut() {
            step.detail = Some(detail.into());
        }
    }

    pub fn succeeded(self) {
        self.emit("succeeded", None, None);
    }

    fn push(&mut self, step: Step) {
        if self.steps.len() < 50 {
            self.steps.push(step);
        }
    }

    fn emit(&self, outcome: &str, error_code: Option<&str>, error: Option<&anyhow::Error>) {
        if self.dry_run || disabled() {
            return;
        }
        emit(self.event(outcome, error_code, error));
    }

    fn event(
        &self,
        outcome: &str,
        error_code: Option<&str>,
        error: Option<&anyhow::Error>,
    ) -> Event {
        let (event_name, key) = match self.kind {
            OperationKind::Posting => (EVENT_NAME, "posting"),
            OperationKind::Invoice => ("invoice_created", "invoice"),
        };
        let diag = Diagnostics {
            error,
            steps: &self.steps,
            input_summary: self.input_summary.as_deref(),
            context: (!self.context.is_empty()).then_some(&self.context),
        };
        build_event(event_name, key, outcome, &self.feature, error_code, diag)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn civil_epoch() {
        assert_eq!(super::civil_from_days(0), (1970, 1, 1));
        assert_eq!(super::civil_from_days(1), (1970, 1, 2));
    }

    #[test]
    fn emit_disabled_is_noop() {
        std::env::set_var("EASYBOOKS_PRODUCT_SIGNALS_ENABLED", "0");
        assert!(super::disabled());
    }

    use super::{build_event, diagnostics, Diagnostics, Operation, Step};
    use anyhow::anyhow;
    use serde_json::{json, Map, Value};

    const ADVERSARIAL: &str = r#"open /Users/jacky/Books/ledger.db and C:\Users\bob\AppData\x and \/Users\/erin\/y for bob@example.com; Authorization: Bearer abcdef0123456789secret; sk-supersecretvalue ghp_0123456789abcdefghijklmnopqrstuvwx github_pat_11ABCDEFG0123456789 xoxb-1234567890-ABCDEFGHIJ AKIAIOSFODNN7EXAMPLE eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJqYWNreSJ9.c2lnbmF0dXJlc2VjcmV0 password=hunter2 dbPassword=hidden1 serviceApiKey=hidden2 0123456789abcdef0123456789abcdef --amount "1,234.56" vendor "Acme Plumbing Ltd" total=2260.00 $120.00 acct 987654321"#;
    const LEAKS: &[&str] = &[
        "jacky",
        "bob",
        "erin",
        "example.com",
        "abcdef0123456789secret",
        "supersecretvalue",
        "ghp_0123",
        "abcdefghijklmnopqrstuvwx",
        "ABCDEFGHIJ",
        "github_pat_11",
        "xoxb-123",
        "AKIAIOSFODNN7",
        "eyJhbGci",
        "hunter2",
        "hidden1",
        "hidden2",
        "0123456789abcdef0123",
        "1,234",
        "Acme",
        "2260",
        "120.00",
        "987654321",
    ];
    const RESERVED: &[&str] = &[
        "error_message",
        "error_stack",
        "metadata",
        "context",
        "steps",
    ];
    const METADATA: &[&str] = &[
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

    fn quiet() {
        // Operation::step emits on failure; keep unit tests off the network.
        std::env::set_var("EASYBOOKS_PRODUCT_SIGNALS_ENABLED", "0");
    }

    fn failing_error() -> anyhow::Error {
        anyhow!("{ADVERSARIAL}").context(
            "Tell Jacky remote_error: {\"error\":\"customer Jane Doe rejected\"} (HTTP 422)",
        )
    }

    /// Go `json.Marshal` byte length (escapes `<>&`, U+2028, U+2029).
    fn go_len(value: &Value) -> usize {
        let json = serde_json::to_string(value).unwrap();
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

    fn assert_clean_string(value: &Value, cap: usize, what: &str) -> String {
        let s = value
            .as_str()
            .unwrap_or_else(|| panic!("{what} must be a string: {value}"));
        assert!(s.len() <= cap, "{what} exceeds {cap} bytes: {}", s.len());
        assert_eq!(diagnostics::text(s, cap), s, "{what} would still scrub");
        for leak in LEAKS {
            assert!(!s.contains(leak), "{what} leaks {leak}: {s}");
        }
        s.to_string()
    }

    fn assert_no_nulls(value: &Value, path: &str) {
        match value {
            Value::Null => panic!("null at {path}"),
            Value::Array(items) => items
                .iter()
                .enumerate()
                .for_each(|(i, v)| assert_no_nulls(v, &format!("{path}[{i}]"))),
            Value::Object(map) => map
                .iter()
                .for_each(|(k, v)| assert_no_nulls(v, &format!("{path}.{k}"))),
            _ => {}
        }
    }

    fn sensitive(key: &str) -> bool {
        let lower = key.to_lowercase();
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

    fn check_context(value: &Value, depth: usize, keys: &mut usize) {
        match value {
            Value::Object(map) => {
                assert!(depth <= 4, "context deeper than 4");
                for (key, child) in map {
                    *keys += 1;
                    assert_clean_string(&Value::String(key.clone()), 128, "context key");
                    if sensitive(key) {
                        assert_eq!(child, "[redacted]", "sensitive key {key} not redacted");
                    } else {
                        check_context(child, depth + 1, keys);
                    }
                }
            }
            Value::Array(items) => {
                assert!(depth <= 4, "context deeper than 4");
                assert!(items.len() <= 50, "context array over 50");
                items.iter().for_each(|v| check_context(v, depth + 1, keys));
            }
            Value::String(_) => {
                assert_clean_string(value, 1024, "context string");
            }
            Value::Number(n) => assert!(n.as_f64().is_some_and(f64::is_finite)),
            Value::Bool(_) => {}
            Value::Null => panic!("context null"),
        }
    }

    /// Every accountd reserved-field rule (contracts/product-signals-v1.md).
    fn assert_accepted(event: &Value) {
        assert_no_nulls(event, "$");
        let is_error = event.get("error_code").is_some_and(|c| c != "")
            || event["event_name"].as_str().unwrap().ends_with("_failed")
            || event.get("outcome").is_some_and(|o| o == "failed");
        let Some(props) = event.get("properties") else {
            return;
        };
        let props = props.as_object().expect("properties object");
        for (key, value) in props {
            assert!(
                RESERVED.contains(&key.as_str()),
                "unregistered property {key}"
            );
            match key.as_str() {
                "error_message" | "error_stack" => {
                    assert!(is_error, "{key} on a non-error event");
                    let cap = if key == "error_message" { 2048 } else { 8192 };
                    assert!(!assert_clean_string(value, cap, key).is_empty());
                }
                "metadata" => {
                    for (k, v) in value.as_object().expect("metadata object") {
                        assert!(METADATA.contains(&k.as_str()), "unknown metadata {k}");
                        let cap = if k == "input_summary" { 512 } else { 128 };
                        assert_clean_string(v, cap, k);
                    }
                }
                "context" => {
                    assert!(value.is_object(), "context root must be an object");
                    let mut keys = 0;
                    check_context(value, 1, &mut keys);
                    assert!(keys <= 64, "context has {keys} keys");
                    assert!(go_len(value) <= 16 * 1024, "context over 16 KiB");
                }
                "steps" => {
                    let steps = value.as_array().expect("steps array");
                    assert!(steps.len() <= 50, "{} steps", steps.len());
                    for step in steps {
                        let step = step.as_object().expect("step object");
                        for (k, v) in step {
                            match k.as_str() {
                                "name" => assert!(!assert_clean_string(v, 128, k).is_empty()),
                                "status" => assert!(!assert_clean_string(v, 32, k).is_empty()),
                                "error_code" => {
                                    assert_clean_string(v, 128, k);
                                }
                                "detail" => {
                                    assert_clean_string(v, 1024, k);
                                }
                                "duration_ms" => {
                                    assert!(v.as_f64().is_some_and(|n| n.is_finite() && n >= 0.0))
                                }
                                other => panic!("unknown step key {other}"),
                            }
                        }
                        assert!(step.contains_key("name") && step.contains_key("status"));
                    }
                }
                _ => unreachable!(),
            }
        }
    }

    fn posting_failure() -> Value {
        quiet();
        let mut op = Operation::posting("expense", false);
        op.summary("1 expense entry");
        op.context("operation", "add");
        op.context("entry_count", 1);
        op.step("validate", "posting_invalid", || Ok(())).unwrap();
        let err = op
            .step("post", "posting_failed", || -> anyhow::Result<()> {
                Err(failing_error())
            })
            .unwrap_err();
        serde_json::to_value(op.event("failed", Some("posting_failed"), Some(&err))).unwrap()
    }

    fn posting_success() -> Value {
        quiet();
        let mut op = Operation::posting("import", false);
        op.step("validate", "posting_invalid", || Ok(())).unwrap();
        op.step("balance", "posting_invalid", || Ok(())).unwrap();
        op.detail("2 income, 1 expense");
        op.summary("3 entries (2 income, 1 expense)");
        op.context("operation", "import");
        op.context("entry_count", 3);
        op.context("source_system_kind", "other");
        op.step("post", "posting_failed", || Ok(())).unwrap();
        serde_json::to_value(op.event("succeeded", None, None)).unwrap()
    }

    fn invoice_failure() -> Value {
        quiet();
        let mut op = Operation::invoice(false);
        op.context("operation", "invoice_create");
        let err = op
            .step("build", "invoice_invalid", || -> anyhow::Result<()> {
                Err(anyhow!("expected value at line 1 column 7")
                    .context(format!("--json is not valid JSON near {ADVERSARIAL}")))
            })
            .unwrap_err();
        serde_json::to_value(op.event("failed", Some("invoice_invalid"), Some(&err))).unwrap()
    }

    fn invoice_success() -> Value {
        quiet();
        let mut op = Operation::invoice(false);
        op.context("operation", "invoice_create");
        op.step("build", "invoice_invalid", || Ok(())).unwrap();
        op.detail("3 line items, client by client_id");
        op.summary("3 line items; client by client_id");
        op.context("item_count", 3);
        op.step("number", "invoice_invalid", || Ok(())).unwrap();
        op.detail("server-assigned number; idempotent on source_id");
        op.step("persist", "invoice_failed", || Ok(())).unwrap();
        op.context("created", true);
        serde_json::to_value(op.event("succeeded", None, None)).unwrap()
    }

    fn step_names(event: &Value) -> Vec<&str> {
        event["properties"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn failure_event_carries_redacted_error_metadata_and_steps() {
        let event = posting_failure();
        assert_accepted(&event);
        assert_eq!(event["event_name"], "posting_completed");
        assert_eq!(event["outcome"], "failed");
        assert_eq!(event["error_code"], "posting_failed");
        let props = &event["properties"];
        let message = props["error_message"].as_str().unwrap();
        assert!(
            message.starts_with("Tell Jacky remote_error: "),
            "{message}"
        );
        assert!(message.contains("open ~/Books/ledger.db"), "{message}");
        assert!(message.contains("<email>") && message.contains("[redacted]"));
        assert!(message.contains("[amount]") && message.contains("[number]"));
        assert!(!message.contains("Jane Doe"), "{message}");
        let stack = props["error_stack"].as_str().unwrap();
        assert!(stack.contains("Caused by:"), "{stack}");
        let metadata = &props["metadata"];
        assert_eq!(metadata["plugin_version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(metadata["arch"], std::env::consts::ARCH);
        assert!(metadata["os"].is_string());
        assert_eq!(metadata["input_summary"], "1 expense entry");
        assert_eq!(
            props["context"],
            json!({"operation": "add", "entry_count": 1})
        );
        assert_eq!(step_names(&event), ["validate", "post"]);
        let post = &props["steps"][1];
        assert_eq!(post["status"], "failed");
        assert_eq!(post["error_code"], "posting_failed");
        assert!(post["duration_ms"].as_f64().unwrap() >= 0.0);
    }

    #[test]
    fn success_events_have_timed_steps_and_no_error_fields() {
        let posting = posting_success();
        assert_accepted(&posting);
        assert_eq!(step_names(&posting), ["validate", "balance", "post"]);
        assert_eq!(
            posting["properties"]["steps"][1]["detail"],
            "2 income, 1 expense"
        );
        let invoice = invoice_success();
        assert_accepted(&invoice);
        assert_eq!(invoice["event_name"], "invoice_created");
        assert_eq!(step_names(&invoice), ["build", "number", "persist"]);
        for event in [&posting, &invoice] {
            assert_eq!(event["outcome"], "succeeded");
            assert!(event.get("error_code").is_none());
            let props = event["properties"].as_object().unwrap();
            assert!(!props.contains_key("error_message") && !props.contains_key("error_stack"));
            for step in props["steps"].as_array().unwrap() {
                assert_eq!(step["status"], "succeeded");
                assert!(step["duration_ms"].as_f64().unwrap() >= 0.0);
            }
        }
    }

    #[test]
    fn error_fields_are_dropped_from_non_error_events() {
        quiet();
        let err = failing_error();
        let diag = Diagnostics {
            error: Some(&err),
            ..Diagnostics::default()
        };
        let event = build_event(
            "posting_completed",
            "posting",
            "succeeded",
            "income",
            None,
            diag,
        );
        let value = serde_json::to_value(event).unwrap();
        assert_accepted(&value);
        assert!(value["properties"].get("error_message").is_none());
        assert!(value["properties"].get("error_stack").is_none());
    }

    /// Every reserved field over its cap; the message cap cuts the
    /// `token=[redacted]` marker (scrub-loop regression input).
    fn oversize_failure() -> Value {
        quiet();
        let err = anyhow!("{}", "x".repeat(9000))
            .context(format!("{} token=secretvalue", "X".repeat(2035)));
        let mut context = Map::new();
        for i in 0..100 {
            context.insert(format!("k{i:03}"), json!("v".repeat(3000)));
        }
        context.insert("deep".into(), json!({"a": {"b": {"c": {"d": {"e": 1}}}}}));
        context.insert("list".into(), json!((0..200).collect::<Vec<_>>()));
        context.insert("api_token".into(), json!({"nested": "sk-supersecretvalue"}));
        context.insert("z".repeat(500), json!(null));
        let steps: Vec<Step> = (0..80)
            .map(|i| {
                let mut step = Step::new(format!("{i}{}", "n".repeat(300)), "s".repeat(100))
                    .with_duration_ms(if i % 2 == 0 { 1.5 } else { f64::NAN })
                    .with_error_code("e".repeat(300));
                step.detail = Some("d".repeat(5000));
                step
            })
            .collect();
        let summary = "s".repeat(2000);
        let diag = Diagnostics {
            error: Some(&err),
            steps: &steps,
            input_summary: Some(&summary),
            context: Some(&context),
        };
        let event = build_event(
            "posting_completed",
            "posting",
            "failed",
            "income",
            Some("posting_failed"),
            diag,
        );
        serde_json::to_value(event).unwrap()
    }

    #[test]
    fn oversize_diagnostics_are_capped_not_rejected() {
        let value = oversize_failure();
        assert_accepted(&value);
        assert!(!value.to_string().contains("secretvalue"));
        let props = &value["properties"];
        assert_eq!(props["error_message"].as_str().unwrap().len(), 2048);
        assert_eq!(props["error_stack"].as_str().unwrap().len(), 8192);
        assert_eq!(
            props["metadata"]["input_summary"].as_str().unwrap().len(),
            512
        );
        assert_eq!(props["steps"].as_array().unwrap().len(), 50);
        assert!(props["steps"][1].get("duration_ms").is_none());
    }

    #[test]
    fn operation_keeps_at_most_fifty_steps() {
        quiet();
        let mut op = Operation::posting("import", false);
        for _ in 0..60 {
            op.step("validate", "posting_invalid", || Ok(())).unwrap();
        }
        let value = serde_json::to_value(op.event("succeeded", None, None)).unwrap();
        assert_eq!(value["properties"]["steps"].as_array().unwrap().len(), 50);
    }

    /// Representative payloads for the real Go validator. Writes one event
    /// object per file only when EASYBOOKS_SIGNALS_PAYLOAD_DIR is set.
    #[test]
    fn representative_payloads_are_accepted() {
        let payloads = [
            ("posting_failed_adversarial", posting_failure()),
            ("posting_succeeded_import", posting_success()),
            ("invoice_failed_build_adversarial", invoice_failure()),
            ("invoice_succeeded", invoice_success()),
            ("posting_failed_oversize", oversize_failure()),
        ];
        for (_, payload) in &payloads {
            assert_accepted(payload);
        }
        if let Ok(dir) = std::env::var("EASYBOOKS_SIGNALS_PAYLOAD_DIR") {
            std::fs::create_dir_all(&dir).unwrap();
            for (name, payload) in &payloads {
                let path = std::path::Path::new(&dir).join(format!("{name}.json"));
                std::fs::write(path, serde_json::to_string_pretty(payload).unwrap()).unwrap();
            }
        }
    }
}
