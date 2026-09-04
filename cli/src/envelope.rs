//! Typed readiness envelope (`jz.plugin.envelope.v1`) for ordinary product paths.

use jz_plugin_common::envelope::{
    AgentNeed, Envelope, Materiality, Sensitivity, ENVELOPE_SCHEMA,
};
use sha2::{Digest, Sha256};

pub const PRODUCT: &str = "easybooks";

pub fn operation_from_args(args: &[String]) -> String {
    args.first()
        .cloned()
        .unwrap_or_else(|| "unknown".into())
}

pub fn progress_fingerprint(status: &str) -> String {
    hex::encode(Sha256::digest(
        format!("{PRODUCT}-readiness-v1|{status}").as_bytes(),
    ))
}

fn resume_token(progress_fingerprint: &str) -> String {
    hex::encode(Sha256::digest(
        format!("{PRODUCT}|connection|{progress_fingerprint}").as_bytes(),
    ))
}

fn portal_connection_need() -> AgentNeed {
    AgentNeed {
        id: "portal_connected".into(),
        kind: "product_capability".into(),
        instruction: "Follow the EasyBooks connect contract: reuse a governed credential when available, otherwise obtain the human sign-in input, run login with token on stdin, then re-invoke the exact continuation.".into(),
        protected_channel: None,
        obtain: vec!["agent_follow_product_connect_contract".into()],
        accepts_multiple: false,
        verify_by: vec!["product_connection_reinspection".into()],
        sensitivity: Sensitivity::Credential,
        materiality: Materiality::ReadOnlyOrProductLocal,
        consent_required: false,
    }
}

pub fn connection_not_ready(operation: &str, continue_args: Vec<String>) -> Envelope {
    let progress = progress_fingerprint("connection_unready");
    Envelope::needs_agent(
        PRODUCT,
        operation,
        vec![portal_connection_need()],
        continue_args,
        resume_token(&progress),
        progress,
    )
}

pub fn ready(operation: &str) -> Envelope {
    Envelope::ready(PRODUCT, operation, progress_fingerprint("ready"))
}

pub fn print_stdout(envelope: &Envelope) -> anyhow::Result<()> {
    envelope
        .validate()
        .map_err(|violations| anyhow::anyhow!("envelope validation failed: {violations:?}"))?;
    println!("{}", serde_json::to_string_pretty(envelope)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use jz_plugin_common::envelope::Status;

    #[test]
    fn connection_not_ready_validates() {
        let envelope = connection_not_ready(
            "categories list",
            vec!["categories".into(), "list".into(), "--json".into()],
        );
        envelope.validate().expect("valid needs_agent envelope");
        assert_eq!(envelope.schema, ENVELOPE_SCHEMA);
        assert_eq!(envelope.status, Status::NeedsAgent);
    }

    #[test]
    fn ready_envelope_is_silent() {
        let envelope = ready("categories list");
        envelope.validate().expect("valid ready envelope");
        assert!(envelope.say_to_user.is_empty());
    }
}
