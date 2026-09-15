//! Typed readiness envelope (`jz.plugin.envelope.v1`).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const ENVELOPE_SCHEMA: &str = "jz.plugin.envelope.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ready,
    NeedsAgent,
    NeedsHuman,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sensitivity {
    Public,
    LocalState,
    LocalSensitive,
    Credential,
    Secret,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Materiality {
    ReadOnlyOrProductLocal,
    ReversibleHostAction,
    ThirdPartySoftwareChange,
    HostEntitlementChange,
    Destructive,
    ExternalWrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialClass {
    ThirdPartyInstallReplaceRepair,
    SignatureOrEntitlementChange,
    PrivilegeElevation,
    UserVisibleDeleteOrOverwrite,
    MoneyMovement,
    IrreversibleExternalWrite,
    MaterialOutsideRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentNeed {
    pub id: String,
    pub kind: String,
    pub instruction: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protected_channel: Option<String>,
    pub obtain: Vec<String>,
    pub accepts_multiple: bool,
    pub verify_by: Vec<String>,
    pub sensitivity: Sensitivity,
    pub materiality: Materiality,
    pub consent_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HumanAction {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material_class: Option<MaterialClass>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub schema: String,
    pub product: String,
    pub operation: String,
    pub status: Status,
    #[serde(default)]
    pub say_to_user: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human_action: Option<HumanAction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub needs: Vec<AgentNeed>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub continue_args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub support_code: Option<String>,
    pub offer_tell_jacky: bool,
    pub progress_fingerprint: String,
    pub no_progress: bool,
    pub dead_end: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub code: &'static str,
    pub message: String,
}

impl Envelope {
    pub fn ready(product: &str, operation: &str, progress_fingerprint: impl Into<String>) -> Self {
        Self {
            schema: ENVELOPE_SCHEMA.into(),
            product: product.into(),
            operation: operation.into(),
            status: Status::Ready,
            say_to_user: String::new(),
            human_action: None,
            needs: Vec::new(),
            continue_args: Vec::new(),
            resume_token: None,
            support_code: None,
            offer_tell_jacky: false,
            progress_fingerprint: progress_fingerprint.into(),
            no_progress: false,
            dead_end: false,
        }
    }

    pub fn needs_agent(
        product: &str,
        operation: &str,
        needs: Vec<AgentNeed>,
        continue_args: Vec<String>,
        resume_token: impl Into<String>,
        progress_fingerprint: impl Into<String>,
    ) -> Self {
        Self {
            schema: ENVELOPE_SCHEMA.into(),
            product: product.into(),
            operation: operation.into(),
            status: Status::NeedsAgent,
            say_to_user: String::new(),
            human_action: None,
            needs,
            continue_args,
            resume_token: Some(resume_token.into()),
            support_code: None,
            offer_tell_jacky: false,
            progress_fingerprint: progress_fingerprint.into(),
            no_progress: false,
            dead_end: false,
        }
    }

    pub fn needs_human(
        product: &str,
        operation: &str,
        say_to_user: impl Into<String>,
        human_action: HumanAction,
        continue_args: Vec<String>,
        resume_token: impl Into<String>,
        progress_fingerprint: impl Into<String>,
    ) -> Self {
        Self {
            schema: ENVELOPE_SCHEMA.into(),
            product: product.into(),
            operation: operation.into(),
            status: Status::NeedsHuman,
            say_to_user: say_to_user.into(),
            human_action: Some(human_action),
            needs: Vec::new(),
            continue_args,
            resume_token: Some(resume_token.into()),
            support_code: None,
            offer_tell_jacky: false,
            progress_fingerprint: progress_fingerprint.into(),
            no_progress: false,
            dead_end: false,
        }
    }

    pub fn blocked(
        product: &str,
        operation: &str,
        say_to_user: impl Into<String>,
        resume_token: impl Into<String>,
        progress_fingerprint: impl Into<String>,
        support_code: Option<String>,
    ) -> Self {
        Self {
            schema: ENVELOPE_SCHEMA.into(),
            product: product.into(),
            operation: operation.into(),
            status: Status::Blocked,
            say_to_user: say_to_user.into(),
            human_action: None,
            needs: Vec::new(),
            continue_args: Vec::new(),
            resume_token: Some(resume_token.into()),
            support_code,
            offer_tell_jacky: true,
            progress_fingerprint: progress_fingerprint.into(),
            no_progress: true,
            dead_end: true,
        }
    }

    pub fn validate(&self) -> Result<(), Vec<Violation>> {
        let mut violations = Vec::new();
        if self.schema != ENVELOPE_SCHEMA {
            violations.push(viol(
                "invalid-schema",
                "schema must be jz.plugin.envelope.v1",
            ));
        }
        if self.product.trim().is_empty() {
            violations.push(viol("missing-product", "product is required"));
        }
        if self.operation.trim().is_empty() {
            violations.push(viol("missing-operation", "operation is required"));
        }
        if self.progress_fingerprint.trim().is_empty() {
            violations.push(viol(
                "missing-progress-fingerprint",
                "progress_fingerprint is required",
            ));
        }

        match self.status {
            Status::Ready => validate_ready(self, &mut violations),
            Status::NeedsAgent => validate_needs_agent(self, &mut violations),
            Status::NeedsHuman => validate_needs_human(self, &mut violations),
            Status::Blocked => validate_blocked(self, &mut violations),
        }

        if !self.continue_args.is_empty() {
            validate_continue_args(self, &mut violations);
            scan_continue_args(self, &mut violations);
        }
        if !self.say_to_user.is_empty() {
            validate_say_to_user(&self.say_to_user, &mut violations);
            scan_public_text(&self.say_to_user, "say_to_user", &mut violations);
        }
        if let Some(code) = &self.support_code {
            scan_public_text(code, "support_code", &mut violations);
        }

        if violations.is_empty() {
            Ok(())
        } else {
            Err(violations)
        }
    }
}

fn validate_ready(envelope: &Envelope, violations: &mut Vec<Violation>) {
    if !envelope.say_to_user.is_empty() {
        violations.push(viol(
            "ready-nonempty-say-to-user",
            "ready forbids non-empty say_to_user",
        ));
    }
    if !envelope.continue_args.is_empty() {
        violations.push(viol(
            "ready-nonempty-continue-args",
            "ready forbids non-empty continue_args",
        ));
    }
    if !envelope.needs.is_empty() {
        violations.push(viol("ready-nonempty-needs", "ready forbids needs"));
    }
    if envelope.human_action.is_some() {
        violations.push(viol("ready-human-action", "ready forbids human_action"));
    }
    if envelope.support_code.is_some() {
        violations.push(viol("ready-support-code", "ready forbids support_code"));
    }
    if envelope.resume_token.is_some() {
        violations.push(viol("ready-resume-token", "ready forbids resume_token"));
    }
}

fn validate_needs_agent(envelope: &Envelope, violations: &mut Vec<Violation>) {
    if envelope.needs.is_empty() {
        violations.push(viol(
            "needs-agent-empty-needs",
            "needs_agent requires non-empty needs",
        ));
    }
    if envelope.continue_args.is_empty() {
        violations.push(viol(
            "needs-agent-empty-continue-args",
            "needs_agent requires non-empty continue_args",
        ));
    }
    if envelope.resume_token.as_deref().is_none_or(str::is_empty) {
        violations.push(viol(
            "needs-agent-missing-resume-token",
            "needs_agent requires resume_token",
        ));
    }
    if envelope.human_action.is_some() {
        violations.push(viol("two-states", "needs_agent forbids human_action"));
    }
    if !envelope.say_to_user.is_empty() {
        violations.push(viol(
            "needs-agent-say-to-user",
            "needs_agent forbids non-empty say_to_user",
        ));
    }
    if !envelope.needs.is_empty() && envelope.human_action.is_some() {
        violations.push(viol("two-states", "exactly one route per envelope"));
    }
    for need in &envelope.needs {
        if need.consent_required {
            violations.push(viol(
                "needs-agent-consent-required",
                "consent_required is forbidden on needs_agent",
            ));
        }
        if need.obtain.iter().any(|o| o == "ask_the_human") {
            violations.push(viol(
                "needs-agent-ask-the-human",
                "ask_the_human is forbidden in obtain",
            ));
        }
    }
}

fn validate_needs_human(envelope: &Envelope, violations: &mut Vec<Violation>) {
    if envelope.human_action.is_none() {
        violations.push(viol(
            "needs-human-missing-human-action",
            "needs_human requires human_action",
        ));
    }
    if envelope.say_to_user.trim().is_empty() {
        violations.push(viol(
            "needs_human-without-say_to_user",
            "needs_human requires non-empty say_to_user",
        ));
    }
    if envelope.continue_args.is_empty() {
        violations.push(viol(
            "needs-human-empty-continue-args",
            "needs_human requires non-empty continue_args",
        ));
    }
    if envelope.resume_token.as_deref().is_none_or(str::is_empty) {
        violations.push(viol(
            "needs-human-missing-resume-token",
            "needs_human requires resume_token",
        ));
    }
    if !envelope.needs.is_empty() {
        violations.push(viol(
            "needs_human-with-needs",
            "needs_human forbids non-empty needs",
        ));
    }
    if let Some(action) = &envelope.human_action {
        if action.kind == "consent" && action.material_class.is_none() {
            violations.push(viol(
                "needs-human-consent-material-class",
                "kind=consent requires material_class",
            ));
        }
    }
}

fn validate_blocked(envelope: &Envelope, violations: &mut Vec<Violation>) {
    if envelope.say_to_user.trim().is_empty() {
        violations.push(viol(
            "blocked-empty-say-to-user",
            "blocked requires non-empty say_to_user",
        ));
    }
    if envelope.resume_token.as_deref().is_none_or(str::is_empty) {
        violations.push(viol(
            "blocked-missing-resume-token",
            "blocked requires resume_token",
        ));
    }
    if !envelope.dead_end {
        violations.push(viol("blocked-dead-end", "blocked requires dead_end: true"));
    }
    if !envelope.continue_args.is_empty() {
        violations.push(viol(
            "blocked-continue-args",
            "blocked forbids non-empty continue_args",
        ));
    }
    if !envelope.needs.is_empty() {
        violations.push(viol("blocked-needs", "blocked forbids needs"));
    }
    if envelope.human_action.is_some() {
        violations.push(viol(
            "blocked-human-action",
            "blocked forbids continuable human_action",
        ));
    }
}

fn validate_continue_args(envelope: &Envelope, violations: &mut Vec<Violation>) {
    if let Some(token) = &envelope.resume_token {
        for arg in &envelope.continue_args {
            if arg == token || arg.contains(token) {
                violations.push(viol(
                    "resume-token-in-argv",
                    "resume_token must not appear in continue_args",
                ));
                return;
            }
        }
    }
}

fn validate_say_to_user(text: &str, violations: &mut Vec<Violation>) {
    if looks_like_command(text) {
        violations.push(viol(
            "command-in-say_to_user",
            "say_to_user must not contain shell commands",
        ));
    }
    if crate::http::looks_like_absolute_path(text) {
        violations.push(viol(
            "path-in-say_to_user",
            "say_to_user must not contain filesystem paths",
        ));
    }
    if looks_like_json(text) {
        violations.push(viol(
            "json-in-say_to_user",
            "say_to_user must not contain JSON",
        ));
    }
    if sentence_count(text) > 2 {
        violations.push(viol(
            "say-to-user-length",
            "say_to_user must be at most two sentences",
        ));
    }
}

fn scan_public_text(text: &str, field: &str, violations: &mut Vec<Violation>) {
    if crate::http::contains_forbidden_secret(text) {
        let code = match field {
            "support_code" => "secret-in-support_code",
            _ if field == "say_to_user" => "secret-in-say_to_user",
            _ => "secret-in-field",
        };
        violations.push(viol(
            code,
            format!("{field} must not contain secrets or credentials"),
        ));
    }
    if field != "support_code" && crate::http::looks_like_absolute_path(text) {
        violations.push(viol(
            "path-in-say_to_user",
            "say_to_user must not contain filesystem paths",
        ));
    }
}

fn scan_continue_args(envelope: &Envelope, violations: &mut Vec<Violation>) {
    for arg in &envelope.continue_args {
        if crate::http::contains_forbidden_secret(arg) {
            violations.push(viol(
                "secret-in-argv",
                "continue_args must not contain secrets or credentials",
            ));
            return;
        }
        if crate::http::looks_like_absolute_path(arg) {
            violations.push(viol(
                "path-in-argv",
                "continue_args must not contain filesystem paths",
            ));
            return;
        }
    }
}

fn looks_like_command(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    for phrase in ["npm install", "npm run", "formbro login"] {
        if contains_command_phrase(&lower, phrase) {
            return true;
        }
    }
    for token in ["cargo", "curl", "bash", "sh", "sudo"] {
        if contains_command_token(&lower, token) {
            return true;
        }
    }
    false
}

fn is_command_prefix_sep(ch: char) -> bool {
    ch.is_ascii_whitespace() || matches!(ch, ';' | '|' | '&' | '(' | '`')
}

fn is_command_suffix_sep(ch: char) -> bool {
    ch.is_ascii_whitespace() || matches!(ch, '-' | ';' | '|' | '&' | ')' | '`')
}

fn contains_command_token(haystack: &str, token: &str) -> bool {
    let mut from = 0;
    while let Some(rel) = haystack[from..].find(token) {
        let start = from + rel;
        let end = start + token.len();
        let before_ok = start == 0
            || haystack[..start]
                .chars()
                .next_back()
                .is_some_and(is_command_prefix_sep);
        let after_ok = end == haystack.len()
            || haystack[end..]
                .chars()
                .next()
                .is_some_and(is_command_suffix_sep);
        if before_ok && after_ok {
            return true;
        }
        from = start + 1;
    }
    false
}

fn contains_command_phrase(haystack: &str, phrase: &str) -> bool {
    let mut from = 0;
    while let Some(rel) = haystack[from..].find(phrase) {
        let start = from + rel;
        let before_ok = start == 0
            || haystack[..start]
                .chars()
                .next_back()
                .is_some_and(is_command_prefix_sep);
        if before_ok {
            return true;
        }
        from = start + 1;
    }
    false
}

fn looks_like_json(text: &str) -> bool {
    let trimmed = text.trim();
    (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
}

fn sentence_count(text: &str) -> usize {
    text.split(['.', '!', '?'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .count()
        .max(1)
}

fn viol(code: &'static str, message: impl Into<String>) -> Violation {
    Violation {
        code,
        message: message.into(),
    }
}

pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/fixtures/plugin-envelope-v1")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn violation_codes(result: Result<(), Vec<Violation>>) -> Vec<String> {
        result
            .err()
            .unwrap_or_default()
            .into_iter()
            .map(|v| v.code.to_string())
            .collect()
    }

    #[test]
    fn contract_fixtures_table_driven() {
        let dir = fixtures_dir();
        let mut entries: Vec<_> = fs::read_dir(&dir)
            .unwrap_or_else(|error| panic!("read fixtures dir {}: {error}", dir.display()))
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .collect();
        entries.sort();
        assert!(
            entries.len() >= 11,
            "expected at least 11 envelope fixtures, found {}",
            entries.len()
        );

        for path in entries {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let raw = fs::read_to_string(&path).unwrap();
            let envelope: Envelope = serde_json::from_str(&raw)
                .unwrap_or_else(|error| panic!("deserialize {name}: {error}"));
            let result = envelope.validate();
            if name.starts_with("invalid-") {
                assert!(
                    result.is_err(),
                    "expected invalid fixture {name} to fail validation"
                );
                let stem = name
                    .strip_prefix("invalid-")
                    .unwrap()
                    .strip_suffix(".json")
                    .unwrap();
                let codes = violation_codes(result);
                assert!(
                    codes
                        .iter()
                        .any(|code| code == stem || code.replace('_', "-") == stem),
                    "fixture {name} expected violation code {stem}, got {codes:?}"
                );
            } else {
                assert!(
                    result.is_ok(),
                    "valid fixture {name} failed: {:?}",
                    result.err()
                );
            }
        }
    }

    #[test]
    fn deny_unknown_top_level_fields() {
        let raw = r#"{
            "schema": "jz.plugin.envelope.v1",
            "product": "anychat",
            "operation": "send",
            "status": "ready",
            "say_to_user": "",
            "continue_args": [],
            "offer_tell_jacky": false,
            "progress_fingerprint": "sha256:abc",
            "no_progress": false,
            "dead_end": false,
            "extra": {}
        }"#;
        assert!(serde_json::from_str::<Envelope>(raw).is_err());
    }

    #[test]
    fn ordinary_english_is_not_a_shell_command() {
        for text in [
            "Automatic setup could not finish on this computer.",
            "Please refresh the status.",
            "The English version is available.",
            "Automatic setup could not complete on this computer.",
        ] {
            assert!(
                !looks_like_command(text),
                "ordinary prose must not be classified as a command: {text}"
            );
        }
    }

    #[test]
    fn real_command_text_is_still_rejected() {
        for text in [
            "Run sh setup.sh",
            "bash scripts/foo",
            "curl https://example.com",
            "sudo reboot",
            "cargo test",
            "Run npm install -g formbro-cli and then formbro login.",
            "npm run build",
        ] {
            assert!(
                looks_like_command(text),
                "command text must still be rejected: {text}"
            );
        }
    }
}
