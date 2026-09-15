//! Shared A2A transport construction for the two host-owned CLI consumers.
//! Protocol requests, events and task semantics belong to the official SDK.
//! The caller supplies a freshly exchanged in-memory audience JWT; this module
//! does not load credentials, spawn a host agent, or execute peer instructions.

use a2a_client::{rest::RestTransport, A2AClient};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use std::time::Duration;

pub use a2a as protocol;

pub const SUPPORT_EXTENSION: &str = "https://jackyzhang.app/a2a/support/v1";

// A bounded HTTP operation is not a deadline on the remote agent's work. This
// applies only to nonstreaming requests; subscriptions retain their lifecycle.
pub(crate) async fn request<T>(
    future: impl std::future::Future<Output = Result<T, a2a::A2AError>>,
) -> Result<T, ClientError> {
    bounded_request(Duration::from_secs(30), future).await
}

async fn bounded_request<T>(
    duration: Duration,
    future: impl std::future::Future<Output = Result<T, a2a::A2AError>>,
) -> Result<T, ClientError> {
    tokio::time::timeout(duration, future)
        .await
        .map_err(|_| ClientError::TimedOut)?
        .map_err(|error| {
            // SDK retains typed ErrorInfo; pre-SDK account authentication
            // failures instead retain the SDK-generated HTTP status prefix.
            let denied = error.details.as_ref().is_some_and(|details| {
                details.iter().any(|detail| {
                    detail.type_url == "type.googleapis.com/google.rpc.ErrorInfo"
                        && matches!(
                            detail
                                .value
                                .get("reason")
                                .and_then(serde_json::Value::as_str),
                            Some("UNAUTHENTICATED" | "UNAUTHORIZED" | "PERMISSION_DENIED")
                        )
                })
            });
            if denied
                || error.message.starts_with("HTTP 401 ")
                || error.message.starts_with("HTTP 403 ")
                || error
                    .message
                    .starts_with("agent card fetch returned HTTP 401 ")
                || error
                    .message
                    .starts_with("agent card fetch returned HTTP 403 ")
            {
                return ClientError::AuthorizationRejected;
            }
            match error.code {
                a2a::error_code::INVALID_PARAMS | a2a::error_code::INVALID_REQUEST => {
                    ClientError::InvalidRequest
                }
                a2a::error_code::TASK_NOT_FOUND => ClientError::TaskNotFound,
                a2a::error_code::INTERNAL_ERROR => {
                    // This SDK discards HTTP status for parseable non-A2A error
                    // envelopes. Unknown internal errors therefore fail closed;
                    // only transport evidence or typed server errors are retried.
                    let temporary = error.message.starts_with("HTTP request failed:")
                        || error.message.starts_with("HTTP 5")
                        || error.message.starts_with("failed to fetch agent card:")
                        || error
                            .message
                            .starts_with("agent card fetch returned HTTP 5")
                        || error.details.as_ref().is_some_and(|details| {
                            details.iter().any(|detail| {
                                detail.type_url == "type.googleapis.com/google.rpc.ErrorInfo"
                                    && matches!(
                                        detail
                                            .value
                                            .get("reason")
                                            .and_then(serde_json::Value::as_str),
                                        Some("SERVER_ERROR" | "INTERNAL_ERROR")
                                    )
                            })
                        });
                    if temporary {
                        ClientError::RequestFailed
                    } else {
                        ClientError::Rejected
                    }
                }
                _ => ClientError::Rejected,
            }
        })
}

/// A2A metadata uses protobuf Struct numbers, which may arrive as 2.0 rather
/// than 2. Preserve the exact safe integer contract when forming acknowledgements.
pub fn delivery_version(event: &a2a::StreamResponse) -> Result<u64, ClientError> {
    let metadata = match event {
        a2a::StreamResponse::Task(value) => &value.metadata,
        a2a::StreamResponse::Message(value) => &value.metadata,
        a2a::StreamResponse::StatusUpdate(value) => &value.metadata,
        a2a::StreamResponse::ArtifactUpdate(value) => &value.metadata,
    };
    let version = metadata
        .as_ref()
        .and_then(|m| m.get(SUPPORT_EXTENSION))
        .and_then(|m| m.get("version"))
        .and_then(serde_json::Value::as_f64)
        .ok_or(ClientError::InvalidResponse)?;
    if !version.is_finite()
        || version <= 0.0
        || version > (1u64 << 53) as f64
        || version.fract() != 0.0
    {
        return Err(ClientError::InvalidResponse);
    }
    Ok(version as u64)
}

/// Support-specific controls remain A2A messages; tasks/events retain SDK types.
fn validate_card(card: &a2a::AgentCard, endpoint: &str) -> Result<(), ClientError> {
    let value = serde_json::to_value(card).map_err(|_| ClientError::InvalidResponse)?;
    let interfaces = value["supportedInterfaces"]
        .as_array()
        .ok_or(ClientError::InvalidResponse)?;
    if !interfaces.iter().any(|item| {
        item["url"]
            .as_str()
            .is_some_and(|url| url.trim_end_matches('/') == endpoint.trim_end_matches('/'))
            && item["protocolVersion"] == "1.0"
            && item["protocolBinding"] == "HTTP+JSON"
    }) {
        return Err(ClientError::InvalidResponse);
    }
    let capabilities = &value["capabilities"];
    let extensions = capabilities["extensions"]
        .as_array()
        .ok_or(ClientError::InvalidResponse)?;
    if capabilities["streaming"] != true
        || !extensions.iter().any(|e| {
            e["uri"] == SUPPORT_EXTENSION && e["required"] == true && e["params"]["version"] == "1"
        })
        || extensions
            .iter()
            .any(|e| e["required"] == true && e["uri"] != SUPPORT_EXTENSION)
    {
        return Err(ClientError::InvalidResponse);
    }
    Ok(())
}

/// Authentication and token refresh stay with each CLI's existing identity owner.
pub struct SupportClient {
    pub sdk: A2AClient<RestTransport>,
    context: String,
    endpoint: String,
    resolver: a2a_client::agent_card::AgentCardResolver,
    verified: std::sync::atomic::AtomicBool,
}

impl SupportClient {
    pub async fn verify(&self) -> Result<(), ClientError> {
        if self.verified.load(std::sync::atomic::Ordering::Acquire) {
            return Ok(());
        }
        let card = request(self.resolver.resolve(&self.endpoint)).await?;
        validate_card(&card, &self.endpoint)?;
        self.verified
            .store(true, std::sync::atomic::Ordering::Release);
        Ok(())
    }

    pub async fn receiver_presence(
        &self,
        phase: &str,
        generation: &str,
    ) -> Result<bool, ClientError> {
        if !matches!(phase, "receiving" | "working") {
            return Err(ClientError::InvalidRequest);
        }
        crate::pair::validate_identifier(generation).map_err(|_| ClientError::InvalidRequest)?;
        let value=self.control("", "Receiver transport state", &uuid::Uuid::new_v4().to_string(), serde_json::json!({"operation":"receiver_presence","phase":phase,"generation":generation})).await?;
        value["accepted"]
            .as_bool()
            .ok_or(ClientError::InvalidResponse)
    }

    pub async fn presence(
        &self,
        phase: Option<&str>,
        generation: Option<&str>,
        acknowledged: Option<u64>,
    ) -> Result<serde_json::Value, ClientError> {
        let mut control = serde_json::json!({"operation":"presence"});
        if let Some(version) = acknowledged {
            if phase.is_some() || generation.is_some() || version == 0 || version > 1u64 << 53 {
                return Err(ClientError::InvalidRequest);
            }
            control = serde_json::json!({"operation":"presence_ack","version":version});
        } else if let Some(phase) = phase {
            if !matches!(phase, "receiving" | "working" | "offline") {
                return Err(ClientError::InvalidRequest);
            }
            let generation = generation.ok_or(ClientError::InvalidRequest)?;
            crate::pair::validate_identifier(generation)
                .map_err(|_| ClientError::InvalidRequest)?;
            control["phase"] = phase.into();
            control["generation"] = generation.into();
        } else if generation.is_some() {
            return Err(ClientError::InvalidRequest);
        }
        self.control(
            "",
            "Support transport state",
            &uuid::Uuid::new_v4().to_string(),
            control,
        )
        .await
    }
    pub fn new(endpoint: &str, bearer: &str, context: &str) -> Result<Self, ClientError> {
        crate::pair::validate_identifier(context).map_err(|_| ClientError::InvalidRequest)?;
        Ok(Self {
            sdk: client(endpoint, bearer)?,
            context: context.into(),
            endpoint: endpoint.trim_end_matches('/').into(),
            resolver: a2a_client::agent_card::AgentCardResolver::new(Some(
                transport_http(endpoint, bearer)?.0,
            )),
            verified: std::sync::atomic::AtomicBool::new(false),
        })
    }

    pub async fn submit(
        &self,
        text: &str,
        message_id: &str,
        task: Option<&str>,
    ) -> Result<a2a::Task, ClientError> {
        self.verify().await?;
        let mut request = self.request(text, message_id)?;
        request.message.task_id = task.map(str::to_owned);
        match crate::pair_a2a::request(self.sdk.send_message(&request)).await? {
            a2a::SendMessageResponse::Task(task) => Ok(task),
            _ => Err(ClientError::InvalidResponse),
        }
    }

    pub async fn publish(
        &self,
        task: &str,
        text: &str,
        message_id: &str,
        phase: &str,
    ) -> Result<(), ClientError> {
        crate::pair::validate_phase(phase).map_err(|_| ClientError::InvalidRequest)?;
        let response = self
            .control(
                task,
                text,
                message_id,
                serde_json::json!({"operation":"publish","phase":phase}),
            )
            .await?;
        if response.get("accepted") != Some(&serde_json::Value::Bool(true)) {
            return Err(ClientError::InvalidResponse);
        }
        Ok(())
    }

    pub async fn pending(&self, task: &str) -> Result<Vec<a2a::StreamResponse>, ClientError> {
        let response = self
            .control(
                task,
                "Receive pending support events",
                &uuid::Uuid::new_v4().to_string(),
                serde_json::json!({"operation":"receive"}),
            )
            .await?;
        serde_json::from_value(
            response
                .get("events")
                .cloned()
                .ok_or(ClientError::InvalidResponse)?,
        )
        .map_err(|_| ClientError::InvalidResponse)
    }

    pub async fn acknowledge(&self, task: &str, version: u64) -> Result<(), ClientError> {
        if version == 0 || version > (1u64 << 53) {
            return Err(ClientError::InvalidRequest);
        }
        let response = self
            .control(
                task,
                "Acknowledge handled support events",
                &uuid::Uuid::new_v4().to_string(),
                serde_json::json!({"operation":"acknowledge","version":version}),
            )
            .await?;
        if response.get("accepted") != Some(&serde_json::Value::Bool(true)) {
            return Err(ClientError::InvalidResponse);
        }
        Ok(())
    }

    fn request(
        &self,
        text: &str,
        message_id: &str,
    ) -> Result<a2a::SendMessageRequest, ClientError> {
        crate::pair::validate_message_text(text).map_err(|_| ClientError::InvalidRequest)?;
        crate::pair::validate_idempotency_key(message_id)
            .map_err(|_| ClientError::InvalidRequest)?;
        let mut message = a2a::Message::new(a2a::Role::User, vec![a2a::Part::text(text)]);
        message.message_id = message_id.into();
        message.context_id = Some(self.context.clone());
        Ok(a2a::SendMessageRequest {
            message,
            configuration: Some(a2a::SendMessageConfiguration {
                accepted_output_modes: None,
                task_push_notification_config: None,
                history_length: None,
                return_immediately: Some(true),
            }),
            metadata: None,
            tenant: None,
        })
    }

    async fn control(
        &self,
        task: &str,
        text: &str,
        message_id: &str,
        control: serde_json::Value,
    ) -> Result<serde_json::Value, ClientError> {
        self.verify().await?;
        let mut request = self.request(text, message_id)?;
        if !task.is_empty() {
            crate::pair::validate_idempotency_key(task).map_err(|_| ClientError::InvalidRequest)?;
            request.message.reference_task_ids = Some(vec![task.into()]);
        }
        request.metadata = Some(std::collections::HashMap::from([(
            SUPPORT_EXTENSION.into(),
            control,
        )]));
        match crate::pair_a2a::request(self.sdk.send_message(&request)).await? {
            a2a::SendMessageResponse::Message(message) if message.parts.len() == 1 => {
                match &message.parts[0].content {
                    a2a::PartContent::Data(value) => Ok(value.clone()),
                    _ => Err(ClientError::InvalidResponse),
                }
            }
            _ => Err(ClientError::InvalidResponse),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("support authorization was rejected; reconnect the account before retrying")]
    AuthorizationRejected,
    #[error("support HTTP request timed out; remote task state is unchanged")]
    TimedOut,
    #[error("support task was not found")]
    TaskNotFound,
    #[error("support request was rejected")]
    Rejected,
    #[error("invalid support request")]
    InvalidRequest,
    #[error("support request failed")]
    RequestFailed,
    #[error("invalid support response")]
    InvalidResponse,
    #[error("invalid support service URL")]
    InvalidEndpoint,
    #[error("invalid support authorization header")]
    InvalidAuthorization,
    #[error("support HTTP client initialization failed")]
    Initialization,
}

impl ClientError {
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::TimedOut | Self::RequestFailed)
    }
}

/// Use one SDK REST binding and never forward authorization through redirects.
/// HTTP is restricted to loopback for isolated acceptance fixtures.
pub fn client(endpoint: &str, bearer: &str) -> Result<A2AClient<RestTransport>, ClientError> {
    let (http, url) = transport_http(endpoint, bearer)?;
    Ok(A2AClient::new(RestTransport::new(http, url.to_string())))
}

fn transport_http(
    endpoint: &str,
    bearer: &str,
) -> Result<(reqwest::Client, reqwest::Url), ClientError> {
    let url = reqwest::Url::parse(endpoint).map_err(|_| ClientError::InvalidEndpoint)?;
    let loopback = url.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ClientError::InvalidEndpoint);
    }
    if bearer.is_empty() {
        return Err(ClientError::InvalidAuthorization);
    }
    let mut auth = HeaderValue::from_str(&format!("Bearer {bearer}"))
        .map_err(|_| ClientError::InvalidAuthorization)?;
    auth.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, auth);
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .default_headers(headers)
        .build()
        .map_err(|_| ClientError::Initialization)?;
    Ok((http, url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bounds_stalled_http_without_changing_task_state_or_exposing_remote_error() {
        let result = bounded_request::<()>(Duration::from_millis(10), std::future::pending()).await;
        assert!(matches!(result, Err(ClientError::TimedOut)));
        let error = bounded_request::<()>(Duration::from_secs(1), async {
            Err(a2a::A2AError::new(
                a2a::error_code::INVALID_PARAMS,
                "synthetic-private-server-detail",
            ))
        })
        .await
        .unwrap_err();
        assert!(!error.is_retryable());
        assert_eq!(error.to_string(), "invalid support request");
    }

    #[tokio::test]
    async fn sdk_authorization_failures_are_not_network_retries() {
        let mut server = mockito::Server::new_async().await;
        let denied = server
            .mock("POST", "/message:send")
            .with_status(401)
            .with_body(r#"{"error":{"code":"invalid_token","message":"synthetic-private-detail"}}"#)
            .create_async()
            .await;
        let client = SupportClient::new(
            &server.url(),
            "synthetic-auth",
            "0123456789abcdef0123456789abcdef",
        )
        .unwrap();
        let outgoing = client.request("Synthetic request", "request-1").unwrap();
        let error = request(client.sdk.send_message(&outgoing))
            .await
            .unwrap_err();
        assert!(matches!(error, ClientError::Rejected));
        assert!(!error.is_retryable());
        denied.assert_async().await;
        let error = bounded_request::<()>(Duration::from_secs(1), async {
            Err(
                a2a::A2AError::internal("synthetic-private-detail").with_details(vec![
                    a2a::TypedDetail::error_info("UNAUTHORIZED", "a2a-protocol.org", None),
                ]),
            )
        })
        .await
        .unwrap_err();
        assert!(matches!(error, ClientError::AuthorizationRejected));
    }

    #[tokio::test]
    async fn discovery_rejects_unauthorized_before_sending_work() {
        let mut server = mockito::Server::new_async().await;
        let denied = server
            .mock("GET", "/.well-known/agent-card.json")
            .with_status(401)
            .create_async()
            .await;
        let work = server
            .mock("POST", "/message:send")
            .expect(0)
            .create_async()
            .await;
        let client = SupportClient::new(
            &server.url(),
            "synthetic-auth",
            "0123456789abcdef0123456789abcdef",
        )
        .unwrap();
        assert!(matches!(
            client.submit("synthetic", "request-1", None).await,
            Err(ClientError::AuthorizationRejected)
        ));
        denied.assert_async().await;
        work.assert_async().await;
    }

    #[test]
    fn discovery_rejects_incompatible_capabilities() {
        let endpoint = "https://example.com/pair/a2a/user";
        let value = serde_json::json!({
          "name":"Synthetic", "version":"1", "description":"test", "skills":[],
          "defaultInputModes":["text/plain"], "defaultOutputModes":["text/plain"],
          "supportedInterfaces":[{"url":endpoint,"protocolVersion":"1.0","protocolBinding":"HTTP+JSON"}],
          "capabilities":{"streaming":true,"extensions":[{"uri":SUPPORT_EXTENSION,"required":true,"params":{"version":"1"}}]}
        });
        let card: a2a::AgentCard = serde_json::from_value(value.clone()).unwrap();
        assert!(validate_card(&card, endpoint).is_ok());
        for (pointer, replacement) in [
            (
                "/supportedInterfaces/0/url",
                serde_json::json!("https://other.invalid"),
            ),
            (
                "/supportedInterfaces/0/protocolVersion",
                serde_json::json!("0.3"),
            ),
            ("/capabilities/streaming", serde_json::json!(false)),
            (
                "/capabilities/extensions/0/params/version",
                serde_json::json!("2"),
            ),
            (
                "/capabilities/extensions/0/uri",
                serde_json::json!("https://unknown.invalid/extension"),
            ),
        ] {
            let mut changed = value.clone();
            *changed.pointer_mut(pointer).unwrap() = replacement;
            let card: a2a::AgentCard = serde_json::from_value(changed).unwrap();
            assert!(
                validate_card(&card, endpoint).is_err(),
                "accepted incompatible {pointer}"
            );
        }
    }

    #[test]
    fn rejects_nonlocal_plaintext_and_url_credentials_without_echoing_input() {
        for endpoint in [
            "http://example.com",
            "https://name:synthetic-secret@example.com",
            "https://example.com?token=synthetic-secret",
        ] {
            let error = client(endpoint, "synthetic-jwt")
                .err()
                .expect("must reject");
            assert_eq!(error.to_string(), "invalid support service URL");
        }
        let error = client("https://example.com", "synthetic\r\ninjected: value")
            .err()
            .expect("must reject");
        assert_eq!(error.to_string(), "invalid support authorization header");
    }
}
