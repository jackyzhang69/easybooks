//! Product Signals batch ingest (fail-open for callers — returns `Result`).

use crate::auth::{self, AuthError};
use crate::http::{self, HttpError};
use crate::identity::PluginIdentity;
use chrono::Utc;
use serde::Serialize;
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub event_id: String,
    #[serde(skip_serializing)]
    pub plugin_id: String,
    pub event_name: String,
    pub event_version: i32,
    #[serde(skip_serializing)]
    pub platform_user_id: String,
    pub actor_class: String,
    pub source: String,
    pub environment: String,
    pub occurred_at: String,
    pub idempotency_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feature_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_bucket: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub properties: HashMap<String, serde_json::Value>,
}

impl Event {
    pub fn new(
        plugin_id: impl Into<String>,
        event_name: impl Into<String>,
        platform_user_id: impl Into<String>,
        idempotency_key: impl Into<String>,
    ) -> Self {
        Self {
            event_id: uuid::Uuid::new_v4().to_string(),
            plugin_id: plugin_id.into(),
            event_name: event_name.into(),
            event_version: 1,
            platform_user_id: platform_user_id.into(),
            actor_class: "user".to_string(),
            source: "product_server".to_string(),
            environment: std::env::var("ENVIRONMENT").unwrap_or_else(|_| "production".into()),
            occurred_at: Utc::now().to_rfc3339(),
            idempotency_key: idempotency_key.into(),
            app_version: None,
            feature_id: None,
            outcome: None,
            duration_bucket: None,
            error_code: None,
            properties: HashMap::new(),
        }
    }
}

#[derive(Debug, Serialize)]
struct IngestBatchRequest<'a> {
    events: &'a [Event],
}

#[derive(Debug, Error)]
pub enum SignalsError {
    #[error("{0}")]
    Auth(#[from] AuthError),
    #[error("{0}")]
    Http(#[from] HttpError),
}

pub fn emit_batch(
    identity: &PluginIdentity,
    accountd_base: &str,
    events: &[Event],
) -> Result<(), SignalsError> {
    if events.is_empty() {
        return Ok(());
    }
    let base = accountd_base.trim_end_matches('/');
    let url = format!("{base}/v1/products/{}/events:batch", identity.plugin_id);
    let body = IngestBatchRequest { events };
    let _: http::Response<serde_json::Value> =
        auth::post_product(identity, accountd_base, &url, &body).map_err(|error| match error {
            AuthError::Http(http_error) => SignalsError::Http(http_error),
            other => SignalsError::Auth(other),
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{AuthMode, PluginIdentity};
    use crate::test_env;

    const IDENTITY: PluginIdentity = PluginIdentity {
        plugin_id: "anychat",
        aud: Some("anychat"),
        auth_mode: AuthMode::Exchange,
        product_scopes: &["read", "write"],
    };

    fn jwt(aud: &str, issuer: &str) -> String {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine as _;
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"ES256","typ":"JWT"}"#);
        let exp = chrono::Utc::now().timestamp() + 120;
        let payload = format!(r#"{{"aud":"{aud}","iss":"{issuer}","exp":{exp}}}"#);
        format!("{}.{}.sig", header, URL_SAFE_NO_PAD.encode(payload))
    }

    #[test]
    fn ingest_json_omits_identity_fields() {
        let event = Event::new(
            "anychat",
            "search_results_returned",
            "550e8400-e29b-41d4-a716-446655440000",
            "k1",
        );
        let value = serde_json::to_value(&event).unwrap();
        assert!(value.get("plugin_id").is_none());
        assert!(value.get("platform_user_id").is_none());
        assert_eq!(value["event_name"], "search_results_returned");
    }

    #[test]
    fn batch_body_field_names() {
        test_env::with_home("jz_signals", |_tmp, mut server| {
            let issuer = server.url();
            let product_jwt = jwt("anychat", &issuer);
            server
                .mock("POST", "/v1/token/exchange")
                .with_status(200)
                .with_body(format!(r#"{{"access_token":"{product_jwt}"}}"#))
                .create();
            let batch = server
                .mock("POST", "/v1/products/anychat/events:batch")
                .match_body(mockito::Matcher::Regex(
                    r#""event_name":"search_results_returned""#.into(),
                ))
                .match_body(mockito::Matcher::Regex(r#""idempotency_key":"k1""#.into()))
                .with_status(200)
                .with_body("{}")
                .create();
            let event = Event::new(
                "anychat",
                "search_results_returned",
                "550e8400-e29b-41d4-a716-446655440000",
                "k1",
            );
            emit_batch(&IDENTITY, &issuer, &[event]).unwrap();
            batch.assert();
        });
    }

    #[test]
    fn typed_error_on_4xx() {
        test_env::with_home("jz_signals", |_tmp, mut server| {
            let issuer = server.url();
            let product_jwt = jwt("anychat", &issuer);
            server
                .mock("POST", "/v1/token/exchange")
                .with_status(200)
                .with_body(format!(r#"{{"access_token":"{product_jwt}"}}"#))
                .create();
            server
                .mock("POST", "/v1/products/anychat/events:batch")
                .with_status(400)
                .with_body(r#"{"error":"bad event"}"#)
                .create();
            let event = Event::new(
                "anychat",
                "bad",
                "550e8400-e29b-41d4-a716-446655440000",
                "k2",
            );
            let err = emit_batch(&IDENTITY, &issuer, &[event]).unwrap_err();
            assert!(matches!(
                err,
                SignalsError::Http(HttpError::Status { code: 400, .. })
            ));
        });
    }

    #[test]
    fn typed_error_on_503() {
        test_env::with_home("jz_signals", |_tmp, mut server| {
            let issuer = server.url();
            let product_jwt = jwt("anychat", &issuer);
            server
                .mock("POST", "/v1/token/exchange")
                .with_status(200)
                .with_body(format!(r#"{{"access_token":"{product_jwt}"}}"#))
                .create();
            server
                .mock("POST", "/v1/products/anychat/events:batch")
                .with_status(503)
                .with_body("busy")
                .create();
            let event = Event::new(
                "anychat",
                "bad",
                "550e8400-e29b-41d4-a716-446655440000",
                "k3",
            );
            let err = emit_batch(&IDENTITY, &issuer, &[event]).unwrap_err();
            assert!(matches!(
                err,
                SignalsError::Http(HttpError::Status { code: 503, .. })
            ));
        });
    }
}
