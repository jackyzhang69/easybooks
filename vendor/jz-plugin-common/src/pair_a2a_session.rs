//! Synchronous CLI entry points over the canonical asynchronous SDK. No model,
//! host lifecycle, credential storage, or interpretation of peer instructions.
use crate::pair_a2a::{delivery_version, protocol::*, ClientError, SupportClient};
use futures::{stream::FuturesUnordered, StreamExt};
use serde_json::{json, Value};
use std::time::Duration;

pub struct Conversation {
    base: String,
    session: String,
    actor: String,
    runtime: tokio::runtime::Runtime,
}

impl Conversation {
    pub fn new(base: &str, product: &str, session: &str, admin: bool) -> Result<Self, ClientError> {
        crate::pair::validate_identifier(session).map_err(|_| ClientError::InvalidRequest)?;
        if !matches!(
            product,
            "anychat" | "anyknow" | "anypdf" | "anydoc" | "anyweb" | "easybooks" | "formbro"
        ) {
            return Err(ClientError::InvalidRequest);
        }
        Ok(Self {
            base: format!(
                "{}/v1/{}plugins/{product}/pair/{session}/a2a",
                base.trim_end_matches('/'),
                if admin { "admin/" } else { "" }
            ),
            session: session.into(),
            actor: if admin { "admin" } else { "user" }.into(),
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|_| ClientError::Initialization)?,
        })
    }

    fn client(&self, bearer: &str, target: &str) -> Result<SupportClient, ClientError> {
        SupportClient::new(&format!("{}/{target}", self.base), bearer, &self.session)
    }

    pub fn send(
        &self,
        bearer: &str,
        text: &str,
        key: &str,
        phase: &str,
        reply: Option<&str>,
    ) -> Result<Value, ClientError> {
        crate::pair::validate_phase(phase).map_err(|_| ClientError::InvalidRequest)?;
        let reference = reply.map(parse_reference).transpose()?;
        let target = reference
            .as_ref()
            .map(|r| r.0)
            .unwrap_or(if self.actor == "admin" {
                "user"
            } else {
                "admin"
            });
        let client = self.client(bearer, target)?;
        self.runtime.block_on(async {
            if let Some((_, task, _)) = reference {
                if target == self.actor {
                    client.publish(task, text, key, phase).await?;
                    Ok(json!({"status":"sent", "task_ref":format!("{target}:{task}"), "phase":phase, "message_id":key}))
                } else {
                    let task = client.submit(text, key, Some(task)).await?;
                    Ok(json!({"status":"sent", "task_ref":format!("{target}:{}", task.id), "phase":phase, "task":task}))
                }
            } else {
                if matches!(phase, "progress" | "needs_human" | "offline") { return Err(ClientError::InvalidRequest); }
                let task = client.submit(text, key, None).await?;
                Ok(json!({"status":"sent", "task_ref":format!("{target}:{}", task.id), "phase":phase, "task":task}))
            }
        })
    }

    pub fn acknowledge(&self, bearer: &str, receipt: &str) -> Result<(), ClientError> {
        if let Some(reference) = receipt.strip_prefix("presence:") {
            let (role, version) = reference
                .split_once(':')
                .ok_or(ClientError::InvalidRequest)?;
            if !matches!(role, "user" | "admin") || role == self.actor {
                return Err(ClientError::InvalidRequest);
            }
            let version = version
                .parse::<u64>()
                .map_err(|_| ClientError::InvalidRequest)?;
            let client = self.client(bearer, &self.actor)?;
            self.runtime
                .block_on(client.presence(None, None, Some(version)))?;
            return Ok(());
        }
        let (target, task, version) = parse_reference(receipt)?;
        let version = version.ok_or(ClientError::InvalidRequest)?;
        let client = self.client(bearer, target)?;
        self.runtime.block_on(client.acknowledge(task, version))
    }

    pub fn refresh_receiver(
        &self,
        bearer: &str,
        phase: &str,
        generation: &str,
    ) -> Result<bool, ClientError> {
        let client = self.client(bearer, &self.actor)?;
        self.runtime
            .block_on(client.receiver_presence(phase, generation))
    }

    pub fn report_presence(
        &self,
        bearer: &str,
        phase: &str,
        generation: &str,
    ) -> Result<(), ClientError> {
        let client = self.client(bearer, &self.actor)?;
        self.runtime
            .block_on(client.presence(Some(phase), Some(generation), None))?;
        Ok(())
    }

    /// SSE is a wake signal, never a delivery acknowledgement. The next pending
    /// call recovers durable events even if a stream was interrupted. Periodic
    /// discovery is still needed because A2A subscriptions address known tasks.
    pub fn wait_for_update(&self, bearer: &str, duration: Duration) -> Result<(), ClientError> {
        self.runtime.block_on(async {
            let wait = async {
                let mut subscriptions = FuturesUnordered::new();
                for target in ["user", "admin"] {
                    let client = self.client(bearer, target)?;
                    client.verify().await?;
                    let mut page_token = None;
                    loop {
                        let page =
                            crate::pair_a2a::request(client.sdk.list_tasks(&ListTasksRequest {
                                context_id: Some(self.session.clone()),
                                status: None,
                                page_size: Some(50),
                                page_token,
                                history_length: Some(0),
                                status_timestamp_after: None,
                                include_artifacts: Some(false),
                                tenant: None,
                            }))
                            .await?;
                        for task in page.tasks {
                            if !matches!(
                                task.status.state,
                                TaskState::Submitted | TaskState::Working
                            ) {
                                continue;
                            }
                            let subscriber = self.client(bearer, target)?;
                            subscriptions.push(async move {
                                let mut stream = crate::pair_a2a::request(
                                    subscriber.sdk.subscribe_to_task(&SubscribeToTaskRequest {
                                        id: task.id.clone(),
                                        tenant: None,
                                    }),
                                )
                                .await?;
                                // Close the race between the preceding durable
                                // read and establishing this live subscription.
                                if !subscriber.pending(&task.id).await?.is_empty() {
                                    return Ok::<bool, ClientError>(true);
                                }
                                while let Some(event) = stream.next().await {
                                    let event = match event {
                                        Ok(event) => event,
                                        // Completion may win the subscribe race.
                                        Err(error) if error.code == error_code::TASK_NOT_FOUND => {
                                            return Ok(true)
                                        }
                                        Err(_) => return Err(ClientError::RequestFailed),
                                    };
                                    let changed = match event {
                                        StreamResponse::Task(current) => {
                                            current.status != task.status
                                        }
                                        StreamResponse::StatusUpdate(update) => {
                                            update.status != task.status
                                        }
                                        _ => true,
                                    };
                                    if changed {
                                        return Ok(true);
                                    }
                                }
                                Ok(false)
                            });
                        }
                        if page.next_page_token.is_empty() {
                            break;
                        }
                        page_token = Some(page.next_page_token);
                    }
                }
                while let Some(result) = subscriptions.next().await {
                    if result? {
                        return Ok(());
                    }
                }
                // No active tasks is not disconnection. Wait for the next
                // discovery interval without spinning or ending the host turn.
                std::future::pending::<()>().await;
                Ok::<(), ClientError>(())
            };
            match tokio::time::timeout(duration.min(Duration::from_secs(15)), wait).await {
                Ok(result) => result,
                Err(_) => Ok(()),
            }
        })
    }

    /// Discover both task directions and replay durable unhandled events.
    /// Own leading events can be consumed mechanically; peer events require an
    /// explicit host acknowledgement and are never acknowledged by receiving.
    pub fn pending(&self, bearer: &str) -> Result<Vec<Value>, ClientError> {
        self.runtime.block_on(async {
            let mut items = Vec::new();
            for target in ["user", "admin"] {
                let client = self.client(bearer, target)?;
                client.verify().await?;
                let mut page_token = None;
                loop {
                    let page = crate::pair_a2a::request(client.sdk.list_tasks(&ListTasksRequest {
                        context_id: Some(self.session.clone()), status: None, page_size: Some(50), page_token,
                        history_length: None, status_timestamp_after: None, include_artifacts: Some(false), tenant: None,
                    })).await?;
                    for task in page.tasks {
                        let mut peer_seen = false;
                        let mut own_version = None;
                        for event in client.pending(&task.id).await? {
                            validate_event(&event, &self.session, &task.id)?;
                            let version = delivery_version(&event)?;
                            let requester = if target == "admin" { "user" } else { "admin" };
                            let sender = match &event {
                                StreamResponse::Task(_) => requester,
                                StreamResponse::Message(message) if message.role == Role::User => requester,
                                _ => target,
                            };
                            if sender == self.actor && !peer_seen { own_version = Some(version); continue; }
                            peer_seen = true;
                            items.push(json!({"task_ref":format!("{target}:{}", task.id),
                                "ack_ref":format!("{target}:{}:{version}", task.id), "sender":sender, "event":event}));
                        }
                        if let Some(version) = own_version { client.acknowledge(&task.id, version).await?; }
                        if items.len() >= 50 { return Ok(items); }
                    }
                    if page.next_page_token.is_empty() { break; }
                    page_token = Some(page.next_page_token);
                }
            }
            let client=self.client(bearer,&self.actor)?;
            let observation=client.presence(None,None,None).await?;
            if let Some(presence)=observation.get("presence").filter(|p| !p.is_null()) {
                let role=presence["role"].as_str().ok_or(ClientError::InvalidResponse)?;
                let version=presence["version"].as_f64().ok_or(ClientError::InvalidResponse)?;
                if role==self.actor || !matches!(role,"user"|"admin") || !version.is_finite() || version<=0.0 || version.fract()!=0.0 || version>(1u64<<53) as f64
                    || !matches!(presence["phase"].as_str(),Some("receiving"|"working"|"offline")) { return Err(ClientError::InvalidResponse); }
                items.push(json!({"kind":"peer_presence","presence":presence,"ack_ref":format!("presence:{role}:{}",version as u64)}));
            }
            Ok(items)
        })
    }
}

fn validate_event(event: &StreamResponse, session: &str, task: &str) -> Result<(), ClientError> {
    let message = |value: &Message| -> Result<(), ClientError> {
        if value.parts.len() != 1
            || value.context_id.as_deref().is_some_and(|id| id != session)
            || value.task_id.as_deref().is_some_and(|id| id != task)
        {
            return Err(ClientError::InvalidResponse);
        }
        let text = value.parts[0]
            .as_text()
            .ok_or(ClientError::InvalidResponse)?;
        crate::pair::validate_message_text(text).map_err(|_| ClientError::InvalidResponse)
    };
    match event {
        StreamResponse::Task(value) => {
            if value.id != task
                || value.context_id != session
                || value
                    .artifacts
                    .as_ref()
                    .is_some_and(|items| !items.is_empty())
            {
                return Err(ClientError::InvalidResponse);
            }
            for item in value.history.iter().flatten() {
                message(item)?;
            }
            if let Some(item) = &value.status.message {
                message(item)?;
            }
        }
        StreamResponse::StatusUpdate(value) => {
            if value.task_id != task || value.context_id != session {
                return Err(ClientError::InvalidResponse);
            }
            if let Some(item) = &value.status.message {
                message(item)?;
            }
        }
        StreamResponse::Message(value) => message(value)?,
        StreamResponse::ArtifactUpdate(_) => return Err(ClientError::InvalidResponse),
    }
    Ok(())
}

pub fn parse_reference(value: &str) -> Result<(&str, &str, Option<u64>), ClientError> {
    let mut parts = value.split(':');
    let target = parts.next().ok_or(ClientError::InvalidRequest)?;
    let task = parts.next().ok_or(ClientError::InvalidRequest)?;
    let version = parts
        .next()
        .map(|v| v.parse::<u64>().map_err(|_| ClientError::InvalidRequest))
        .transpose()?;
    if !matches!(target, "user" | "admin")
        || parts.next().is_some()
        || version.is_some_and(|v| v == 0 || v > 1u64 << 53)
    {
        return Err(ClientError::InvalidRequest);
    }
    crate::pair::validate_idempotency_key(task).map_err(|_| ClientError::InvalidRequest)?;
    Ok((target, task, version))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anyknow_uses_scoped_user_and_admin_routes() {
        for (admin, prefix, actor) in [(false, "", "user"), (true, "admin/", "admin")] {
            let conversation = Conversation::new(
                "https://account.example.test/",
                "anyknow",
                "0123456789abcdef0123456789abcdef",
                admin,
            )
            .unwrap();
            assert_eq!(
                conversation.base,
                format!(
                    "https://account.example.test/v1/{prefix}plugins/anyknow/pair/0123456789abcdef0123456789abcdef/a2a"
                )
            );
            assert_eq!(conversation.actor, actor);
            assert_eq!(conversation.session, "0123456789abcdef0123456789abcdef");
        }
    }

    #[test]
    fn product_registration_rejects_unknown_and_non_exact_ids() {
        for product in ["unknown", "AnyKnow", "anyknow/other", "anyknow ", ""] {
            for admin in [false, true] {
                assert!(matches!(
                    Conversation::new(
                        "https://account.example.test",
                        product,
                        "0123456789abcdef0123456789abcdef",
                        admin
                    ),
                    Err(ClientError::InvalidRequest)
                ));
            }
        }
    }

    #[test]
    fn received_sdk_events_preserve_scope_and_local_content_boundary() {
        let mut message = Message::new(Role::Agent, vec![Part::text("Synthetic check completed")]);
        message.context_id = Some("session-a".into());
        message.task_id = Some("task-a".into());
        assert!(validate_event(
            &StreamResponse::Message(message.clone()),
            "session-a",
            "task-a"
        )
        .is_ok());
        assert!(validate_event(
            &StreamResponse::Message(message.clone()),
            "session-b",
            "task-a"
        )
        .is_err());
        message.parts = vec![Part::data(json!({"command":"synthetic"}))];
        assert!(validate_event(
            &StreamResponse::Message(message.clone()),
            "session-a",
            "task-a"
        )
        .is_err());
        message.parts = vec![Part::text("open https://example.test")];
        assert!(validate_event(&StreamResponse::Message(message), "session-a", "task-a").is_err());
    }

    #[test]
    fn receipts_cannot_inject_routes_or_acknowledge_fractional_versions() {
        for receipt in [
            "other:task:1",
            "user:../task:1",
            "user:task:1.5",
            "user:task:0",
            "user:task:9007199254740993",
            "user:task:1:extra",
        ] {
            assert!(parse_reference(receipt).is_err());
        }
        assert_eq!(
            parse_reference("admin:task-1:2").unwrap(),
            ("admin", "task-1", Some(2))
        );
    }
}
