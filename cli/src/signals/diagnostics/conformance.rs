//! Contract conformance against the shared Product Signals diagnostics
//! fixtures (`cli/tests/fixtures/product-signals-diagnostics`, copied from
//! jackyzhang69/platform @ 421e3112c). Each event is built from the same inputs
//! as `jz-plugin-common/tests/signals_diagnostics.rs`, but through EasyBooks'
//! own [`normalize`]. The crate builders (`with_error_message`, `with_metadata`,
//! `with_steps`, …) only insert raw values into `properties`, so the helpers
//! below do the same. No field is normalized: plugin id, event id, timestamps
//! and environment are pinned to the fixture values exactly as the crate test
//! pins them.

use super::{normalize, Step};
use jz_plugin_common::signals::Event;
use serde_json::{json, Map, Value};
use std::path::Path;

const ADVERSARIAL: &str = r#"/Users/alice/src /home/bob/src C:\Users\carol\src D:/Users/dave/src \/Users\/erin\/src \\Users\\frank\\src alice@example.com Authorization: Bearer standalonesecret Bearer alternate_secret sk-supersecretvalue ghp_0123456789abcdefghijklmnopqrstuvwx github_pat_shortsecret xoxb-1234567890-ABCDEFGHIJ AKIAIOSFODNN7EXAMPLE 0123456789abcdef0123456789abcdef abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789+/ xxx.yyy.zzz "api_key": "quoted secret value" dbpassword=hidden clientSecret=hidden serviceApiKey=hidden monkey=value"#;

fn event(name: &str) -> Event {
    let mut event = Event::new(
        "anypdf",
        name,
        "550e8400-e29b-41d4-a716-446655440000",
        "diagnostics-fixture",
    );
    event.event_id = "7a9f3d7a-9762-43a3-9eb8-bf1a5de3b080".into();
    event.occurred_at = "2026-08-24T12:00:00Z".into();
    event.environment = "test".into();
    event
}

fn with(mut event: Event, key: &str, value: impl Into<Value>) -> Event {
    event.properties.insert(key.into(), value.into());
    event
}

/// `ClientMetadata` serialization: unset fields are omitted.
fn metadata(fields: &[(&str, String)]) -> Value {
    Value::Object(
        fields
            .iter()
            .map(|(key, value)| ((*key).to_string(), Value::String(value.clone())))
            .collect(),
    )
}

/// `Event::with_steps`: keep the first 50, serialize each `Step`.
fn steps(mut steps: Vec<Step>) -> Value {
    steps.truncate(50);
    serde_json::to_value(steps).expect("steps serialize")
}

fn detailed(step: Step, detail: impl Into<String>) -> Step {
    Step {
        detail: Some(detail.into()),
        ..step
    }
}

fn fixtures() -> Vec<(&'static str, Event)> {
    let redaction = event("form_fill_failed");
    let redaction = with(redaction, "error_message", ADVERSARIAL);
    let redaction = with(redaction, "error_stack", ADVERSARIAL);
    let redaction = with(
        redaction,
        "metadata",
        metadata(&[
            ("os", ADVERSARIAL.into()),
            ("os_version", "27.0".into()),
            ("arch", "arm64".into()),
            ("plugin_version", "1.0.0".into()),
            ("client_version", "2.0.0".into()),
            ("host_app", "Desktop".into()),
            ("host_app_version", "3.0.0".into()),
            ("locale", "en-CA".into()),
            ("input_summary", ADVERSARIAL.into()),
        ]),
    );
    let redaction = with(
        redaction,
        "context",
        json!({"operation": ADVERSARIAL, "dbPassword": {"secret": "subtreeprivate"},
        "nested": {"cookie": ["subtreeprivate"], "safe": [true, 2.5, ADVERSARIAL, null]},
        "APİKEY": "unicodeprivate",
        "alice@example.com": "key normalized"}),
    );
    let redaction = with(
        redaction,
        "steps",
        steps(vec![detailed(
            Step::new(ADVERSARIAL, "failed")
                .with_duration_ms(12.5)
                .with_error_code(ADVERSARIAL),
            ADVERSARIAL,
        )]),
    );

    let oversized = event("form_fill_failed");
    let oversized = with(
        oversized,
        "error_message",
        format!("token=boundarysecret {}", "界".repeat(2000)),
    );
    let oversized = with(oversized, "error_stack", "界".repeat(4000));
    let oversized = with(
        oversized,
        "metadata",
        metadata(&[
            ("os", "界".repeat(100)),
            ("input_summary", "界".repeat(1000)),
        ]),
    );
    let oversized = with(
        oversized,
        "steps",
        steps(
            (0..60)
                .map(|i| {
                    detailed(
                        Step::new(format!("s{i}{}", "界".repeat(100)), "failed".repeat(10))
                            .with_error_code("e".repeat(200))
                            .with_duration_ms(0.0),
                        "界".repeat(500),
                    )
                })
                .collect(),
        ),
    );

    let mut object = Map::new();
    for i in 0..100 {
        object.insert(format!("k{i:03}"), json!("<>&\u{2028}界".repeat(300)));
    }
    let budget = with(event("form_fill_completed"), "context", object);

    let structure = with(
        event("form_fill_completed"),
        "context",
        json!({
            "deep": {"two": [{"four": {"drop": "too deep"}, "scalar": "retained"}]},
            "items": (0..70).collect::<Vec<_>>(),
            "null": null,
            "a".repeat(129): "first collision",
            format!("{}b", "a".repeat(128)): "second collision",
            "authToken": {"very": {"deep": {"secret": ["subtreeprivate"]}}}
        }),
    );

    let malformed = event("form_fill_failed");
    let malformed = with(
        malformed,
        "metadata",
        json!({"unknown": "discard", "os": null, "arch": "arm64"}),
    );
    let malformed = with(
        malformed,
        "steps",
        json!([
            {"name": "", "status": "failed"}, {"name": "negative", "status": "failed", "duration_ms": -1, "unknown": true},
            {"name": "ok", "status": "failed", "error_code": "", "detail": null},
            {"name": "wrong", "status": null}
        ]),
    );
    let malformed = with(malformed, "ERROR_STACK", "discard noncanonical alias");

    let success = event("form_fill_completed");
    let success = with(success, "error_message", ADVERSARIAL);
    let success = with(success, "error_stack", ADVERSARIAL);
    let success = with(success, "context", json!({"count": 12}));

    let mut outcome = with(
        event("form_fill_completed"),
        "error_message",
        "failed by outcome",
    );
    outcome.outcome = Some("failed".into());
    let mut code = with(
        event("form_fill_completed"),
        "error_message",
        "failed by code",
    );
    code.error_code = Some("execution_failed".into());

    let keys = with(
        event("form_fill_completed"),
        "context",
        (0..100)
            .map(|i| (format!("k{i:03}"), json!(i)))
            .collect::<Map<_, _>>(),
    );

    let whitespace = with(
        event("form_fill_failed"),
        "error_message",
        "token=private\u{a0}tail file:///Users/alice/private passwd=hidden",
    );

    // Each cap cuts through an inserted "[redacted]" / "<email>" / "file://<redacted>".
    let cut = event("form_fill_failed");
    let cut = with(cut, "error_message", format!("{}token=x", "a".repeat(2041)));
    let cut = with(
        cut,
        "error_stack",
        format!("{} alice@example.com", "s".repeat(8188)),
    );
    let cut = with(
        cut,
        "metadata",
        metadata(&[("os", format!("{} file:///Users/alice/x", "o".repeat(120)))]),
    );
    let cut = with(
        cut,
        "context",
        json!({"value": format!("{}secret=v", "c".repeat(1015))}),
    );
    let cut = with(
        cut,
        "steps",
        steps(vec![detailed(
            Step::new("cut", "failed"),
            format!("{} Bearer abcdef", "d".repeat(1019)),
        )]),
    );

    vec![
        ("redaction", redaction),
        ("oversized", oversized),
        ("context-budget", budget),
        ("context-structure", structure),
        ("invalid-values", malformed),
        ("success", success),
        ("outcome-failed", outcome),
        ("code-failed", code),
        ("context-keys", keys),
        ("whitespace", whitespace),
        ("replacement-cut", cut),
    ]
}

#[test]
fn diagnostics_match_shared_contract_fixtures() {
    let directory =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/product-signals-diagnostics");
    let cases = fixtures();
    assert_eq!(cases.len(), 11);
    for (name, mut event) in cases {
        normalize(&mut event);
        let actual = serde_json::to_value(&event).expect("event serializes");
        let path = directory.join(format!("{name}.json"));
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let expected: Value = serde_json::from_str(&raw).expect("fixture is JSON");
        assert_eq!(actual, expected, "fixture {name}");
    }
}
