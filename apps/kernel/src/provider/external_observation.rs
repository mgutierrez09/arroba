use crate::history::SessionHistoryExternalObservation;
use crate::provider::ProviderRunTokenUsage;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ObservedExternalProviderTurn {
    pub(crate) role: ObservedExternalProviderTurnRole,
    pub(crate) text: String,
    pub(crate) provider_turn_id: Option<String>,
    pub(crate) observed_at_ms: Option<u64>,
}

impl ObservedExternalProviderTurn {
    pub(crate) fn stable_fallback_id(&self) -> String {
        format!(
            "observed-v1-{}-{:016x}",
            role_text(self.role),
            stable_observed_turn_hash(self.role, &self.text, self.observed_at_ms)
        )
    }

    pub(crate) fn provider_turn_id_or_fallback(&self) -> String {
        self.provider_turn_id
            .clone()
            .unwrap_or_else(|| self.stable_fallback_id())
    }

    pub(crate) fn external_merge_key(&self, provider: &str, provider_session_id: &str) -> String {
        crate::history::external_provider_observed_merge_key(
            provider,
            provider_session_id,
            &self.provider_turn_id_or_fallback(),
        )
    }
}

fn stable_observed_turn_hash(
    role: ObservedExternalProviderTurnRole,
    text: &str,
    observed_at_ms: Option<u64>,
) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    stable_observed_turn_hash_bytes(&mut hash, role_text(role).as_bytes());
    stable_observed_turn_hash_bytes(&mut hash, &[0]);
    stable_observed_turn_hash_bytes(&mut hash, text.as_bytes());
    stable_observed_turn_hash_bytes(&mut hash, &[0]);
    match observed_at_ms {
        Some(value) => {
            stable_observed_turn_hash_bytes(&mut hash, &[1]);
            stable_observed_turn_hash_bytes(&mut hash, &value.to_be_bytes());
        }
        None => stable_observed_turn_hash_bytes(&mut hash, &[0]),
    }
    hash
}

fn stable_observed_turn_hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub(crate) enum ObservedExternalProviderTurnRole {
    User,
    Assistant,
    Reasoning,
    Tool,
    Status,
}

impl ObservedExternalProviderTurnRole {
    pub(crate) fn as_str(self) -> &'static str {
        role_text(self)
    }

    pub(crate) fn session_history_kind(self) -> crate::history::SessionHistoryEntryKind {
        match self {
            Self::User => crate::history::SessionHistoryEntryKind::UserPrompt,
            Self::Assistant => crate::history::SessionHistoryEntryKind::ProviderOutput,
            Self::Reasoning => crate::history::SessionHistoryEntryKind::ProviderReasoning,
            Self::Tool => crate::history::SessionHistoryEntryKind::ProviderTool,
            Self::Status => crate::history::SessionHistoryEntryKind::ProviderStatus,
        }
    }
}

pub(crate) fn observed_role(role: Option<&str>) -> Option<ObservedExternalProviderTurnRole> {
    match role {
        Some("user") => Some(ObservedExternalProviderTurnRole::User),
        Some("assistant") => Some(ObservedExternalProviderTurnRole::Assistant),
        Some("reasoning") => Some(ObservedExternalProviderTurnRole::Reasoning),
        Some("tool") => Some(ObservedExternalProviderTurnRole::Tool),
        Some("status") => Some(ObservedExternalProviderTurnRole::Status),
        _ => None,
    }
}

pub(crate) fn clean_observed_turn_text(role: Option<&str>, text: String) -> Option<String> {
    match observed_role(role)? {
        ObservedExternalProviderTurnRole::User => clean_provider_prompt(text),
        ObservedExternalProviderTurnRole::Assistant => {
            let text = text.trim();
            (!text.is_empty()).then(|| text.to_string())
        }
        ObservedExternalProviderTurnRole::Reasoning
        | ObservedExternalProviderTurnRole::Tool
        | ObservedExternalProviderTurnRole::Status => {
            let text = text.trim();
            (!text.is_empty()).then(|| text.to_string())
        }
    }
}

pub(crate) fn text_from_content(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => {
            let parts = items
                .iter()
                .filter_map(|item| {
                    item.get("text")
                        .or_else(|| item.get("content"))
                        .or_else(|| item.get("value"))
                        .and_then(Value::as_str)
                })
                .collect::<Vec<_>>();
            (!parts.is_empty()).then(|| parts.join("\n"))
        }
        Value::Object(_) => value
            .get("text")
            .or_else(|| value.get("content"))
            .or_else(|| value.get("value"))
            .and_then(Value::as_str)
            .map(str::to_string),
        _ => None,
    }
}

pub(crate) fn clean_provider_prompt(prompt: String) -> Option<String> {
    // Decode before provider wrapper removal or whitespace compaction destroys
    // the frame's byte offsets. Its request is already the user-authored text.
    let request = strip_observed_account_handoff(&prompt);
    if request != prompt {
        return (!request.trim().is_empty()).then(|| request.trim().to_string());
    }
    let prompt = strip_observed_generated_prompt_context(prompt.trim()).trim();
    if prompt.is_empty()
        || prompt.starts_with("# AGENTS.md instructions")
        || prompt.starts_with("<environment_context>")
        || prompt.starts_with("<recommended_plugins>")
        || prompt.starts_with("<skills_instructions>")
        || prompt.starts_with("<apps_instructions>")
        || prompt.starts_with("<plugins_instructions>")
        || prompt.starts_with("Native provider execution is enabled")
    {
        return None;
    }
    let prompt = prompt
        .split("## My request for Codex:")
        .last()
        .unwrap_or(prompt)
        .split("## My request:")
        .last()
        .unwrap_or(prompt)
        .trim();
    (!prompt.is_empty()).then(|| compact_whitespace(prompt))
}

fn compact_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn role_text(role: ObservedExternalProviderTurnRole) -> &'static str {
    match role {
        ObservedExternalProviderTurnRole::User => "user",
        ObservedExternalProviderTurnRole::Assistant => "assistant",
        ObservedExternalProviderTurnRole::Reasoning => "reasoning",
        ObservedExternalProviderTurnRole::Tool => "tool",
        ObservedExternalProviderTurnRole::Status => "status",
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ExternalProviderObservationPolicy<'a> {
    provider: &'a str,
}

#[derive(Debug, Clone, Copy)]
struct ExternalProviderObservationSpec {
    provider: &'static str,
    requires_explicit_completion: bool,
    settling_status_prefixes: &'static [&'static str],
    settling_status_fragments: &'static [&'static str],
    passive_status_prefixes: &'static [&'static str],
    projects_token_usage: bool,
}

const EXTERNAL_PROVIDER_OBSERVATION_SPECS: &[ExternalProviderObservationSpec] = &[
    ExternalProviderObservationSpec {
        provider: "codex",
        requires_explicit_completion: true,
        settling_status_prefixes: &["codex task_complete", "codex event turn_aborted"],
        settling_status_fragments: &["\"type\":\"turn_aborted\"", "\"type\": \"turn_aborted\""],
        passive_status_prefixes: &["codex token_count"],
        projects_token_usage: true,
    },
    ExternalProviderObservationSpec {
        provider: "claude",
        requires_explicit_completion: false,
        settling_status_prefixes: &["claude message completed"],
        settling_status_fragments: &[],
        passive_status_prefixes: &["claude last-prompt", "claude ai-title"],
        projects_token_usage: false,
    },
    ExternalProviderObservationSpec {
        provider: "opencode",
        requires_explicit_completion: true,
        settling_status_prefixes: &["opencode message completed"],
        settling_status_fragments: &[],
        passive_status_prefixes: &[],
        projects_token_usage: false,
    },
];

impl<'a> ExternalProviderObservationPolicy<'a> {
    pub(crate) fn for_provider(provider: &'a str) -> Self {
        Self { provider }
    }

    pub(crate) fn configured_provider_ids() -> impl Iterator<Item = &'static str> {
        EXTERNAL_PROVIDER_OBSERVATION_SPECS
            .iter()
            .map(|spec| spec.provider)
    }

    pub(crate) fn is_configured(self) -> bool {
        self.spec().is_some()
    }

    fn spec(self) -> Option<&'static ExternalProviderObservationSpec> {
        let provider = self.provider.trim();
        EXTERNAL_PROVIDER_OBSERVATION_SPECS
            .iter()
            .find(|spec| provider.eq_ignore_ascii_case(spec.provider))
    }

    pub(crate) fn uses_explicit_completion(self) -> bool {
        self.spec()
            .is_some_and(|spec| spec.requires_explicit_completion)
    }

    pub(crate) fn user_prompt_is_internal_control(self, text: &str) -> bool {
        if !self.provider.trim().eq_ignore_ascii_case("claude") {
            return false;
        }
        let text = text.trim();
        text == "[Request interrupted by user]"
            || (text.starts_with("<task-notification>") && text.ends_with("</task-notification>"))
    }

    pub(crate) fn status_settles(self, text: &str) -> bool {
        self.spec().is_some_and(|spec| {
            spec.settling_status_prefixes
                .iter()
                .any(|prefix| status_text_starts_with(text, prefix))
                || spec
                    .settling_status_fragments
                    .iter()
                    .any(|fragment| text.contains(fragment))
        })
    }

    pub(crate) fn status_is_passive_telemetry(self, text: &str) -> bool {
        self.spec().is_some_and(|spec| {
            spec.passive_status_prefixes
                .iter()
                .any(|prefix| status_text_starts_with(text, prefix))
        })
    }

    pub(crate) fn status_usage(self, text: &str) -> Option<ProviderRunTokenUsage> {
        if !self.spec().is_some_and(|spec| spec.projects_token_usage) {
            return None;
        }
        let (header, payload) = text.split_once('\n')?;
        if !header.trim().eq_ignore_ascii_case("codex token_count") {
            return None;
        }
        let payload: serde_json::Value = serde_json::from_str(payload).ok()?;
        let context_tokens = first_u64_path(
            &payload,
            &[
                &["info", "total_token_usage", "total_tokens"],
                &["total_token_usage", "total_tokens"],
                &["info", "totalTokenUsage", "totalTokens"],
                &["totalTokenUsage", "totalTokens"],
                &["last", "total_tokens"],
                &["last", "totalTokens"],
            ],
        );
        let context_window = first_u64_path(
            &payload,
            &[
                &["info", "model_context_window"],
                &["info", "modelContextWindow"],
                &["model_context_window"],
                &["modelContextWindow"],
            ],
        );
        let context_tokens_with_window = match (context_tokens, context_window) {
            (Some(tokens), Some(window)) if tokens <= window => Some(tokens),
            _ => None,
        };
        (context_tokens.is_some() || context_window.is_some()).then_some(ProviderRunTokenUsage {
            total_tokens: context_tokens,
            last_tokens: context_tokens,
            context_tokens: context_tokens_with_window,
            context_window,
        })
    }

    pub(crate) fn turn_is_passive_telemetry(self, turn: &ObservedExternalProviderTurn) -> bool {
        turn.role == ObservedExternalProviderTurnRole::Status
            && self.status_is_passive_telemetry(&turn.text)
    }

    pub(crate) fn latest_effective_turn_settles(
        self,
        turns: &[ObservedExternalProviderTurn],
    ) -> bool {
        let Some(latest) = turns
            .iter()
            .rev()
            .find(|turn| !self.turn_is_passive_telemetry(turn))
            .or_else(|| turns.last())
        else {
            return false;
        };
        match latest.role {
            ObservedExternalProviderTurnRole::Status => self.status_settles(&latest.text),
            ObservedExternalProviderTurnRole::Assistant
            | ObservedExternalProviderTurnRole::User
            | ObservedExternalProviderTurnRole::Reasoning
            | ObservedExternalProviderTurnRole::Tool => false,
        }
    }

    pub(crate) fn observation_for_turn(
        self,
        turn: &ObservedExternalProviderTurn,
    ) -> Option<SessionHistoryExternalObservation> {
        SessionHistoryExternalObservation {
            settles_active_prompt: turn.role == ObservedExternalProviderTurnRole::Status
                && self.status_settles(&turn.text),
            passive_telemetry: self.turn_is_passive_telemetry(turn),
        }
        .useful()
    }
}

fn status_text_starts_with(text: &str, prefix: &str) -> bool {
    let text = text.trim_start();
    let Some(header) = text.get(..prefix.len()) else {
        return false;
    };
    if !header.eq_ignore_ascii_case(prefix) {
        return false;
    }
    text[prefix.len()..]
        .chars()
        .next()
        .is_none_or(char::is_whitespace)
}

fn first_u64_path(value: &serde_json::Value, paths: &[&[&str]]) -> Option<u64> {
    paths.iter().find_map(|path| read_u64_path(value, path))
}

fn read_u64_path(value: &serde_json::Value, path: &[&str]) -> Option<u64> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    current.as_u64()
}

pub(crate) fn normalized_observed_prompt_text(text: &str) -> Option<String> {
    let without_account_handoff = strip_observed_account_handoff(text);
    let without_provider_attachment_suffix =
        strip_observed_provider_attachment_suffix(without_account_handoff);
    let without_attachments = strip_observed_attachment_markup(without_provider_attachment_suffix);
    let normalized = strip_observed_generated_prompt_context(&without_attachments)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!normalized.is_empty()).then_some(normalized)
}

fn strip_observed_account_handoff(text: &str) -> &str {
    if let Some((request, suffix)) = super::account_handoff::decode_account_handoff(text.trim()) {
        return if strip_observed_provider_attachment_suffix(suffix)
            .trim()
            .is_empty()
        {
            request
        } else {
            text
        };
    }
    let Some(context) = text.trim().strip_prefix("<chariox_context_handoff>") else {
        return text;
    };
    let Some((_, remainder)) = context.split_once("</chariox_context_handoff>") else {
        return text;
    };
    let Some(transition) = remainder
        .trim_start()
        .strip_prefix("Provider/account switch:")
    else {
        return text;
    };
    let Some((_, request)) = transition.split_once("<user_request>") else {
        return text;
    };
    // Historical, unframed records can only be decoded when unambiguous.
    // New provider prompts use byte-counted framing above.
    let Some((request, suffix)) = request.split_once("</user_request>") else {
        return text;
    };
    if !suffix.contains("</user_request>")
        && strip_observed_provider_attachment_suffix(suffix)
            .trim()
            .is_empty()
    {
        return request;
    }
    text
}

fn strip_observed_provider_attachment_suffix(text: &str) -> &str {
    for (start, _) in text.match_indices("Attachment:") {
        let header = text[start..].lines().next().unwrap_or_default();
        let Some(description) = header.strip_prefix("Attachment: ") else {
            continue;
        };
        let Some((label_and_mime, url)) = description.rsplit_once(") at ") else {
            continue;
        };
        let Some((label, mime)) = label_and_mime.rsplit_once(" (") else {
            continue;
        };
        if label.trim().is_empty() || mime.trim().is_empty() || url.trim().is_empty() {
            continue;
        }
        return text[..start].trim_end();
    }
    text
}

fn strip_observed_generated_prompt_context(text: &str) -> &str {
    text.find("<runtime-instructions>")
        .map(|index| &text[..index])
        .unwrap_or(text)
}

fn strip_observed_attachment_markup(text: &str) -> String {
    ["image", "file"]
        .into_iter()
        .fold(text.to_string(), |current, tag| {
            strip_observed_attachment_tag_blocks(&current, tag)
        })
}

fn strip_observed_attachment_tag_blocks(text: &str, tag: &str) -> String {
    let open_prefix = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut output = String::new();
    let mut remaining = text;
    while let Some(start) = remaining.find(&open_prefix) {
        output.push_str(&remaining[..start]);
        let after_open = &remaining[start..];
        let Some(end) = after_open.find(&close) else {
            output.push_str(after_open);
            return output;
        };
        remaining = &after_open[end + close.len()..];
    }
    output.push_str(remaining);
    output
}

#[cfg(test)]
mod tests;
