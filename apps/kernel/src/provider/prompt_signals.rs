use crate::terminal::TerminalOutputKind;

use super::launch_contract::ProviderResumeState;
use super::runtime_run::ProviderRunTokenUsage;

pub(crate) const PROVIDER_CONNECTION_RETRY_MERGE_KEY: &str = "__provider_connection_retry__";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderPromptChunk {
    pub kind: TerminalOutputKind,
    pub merge_key: Option<String>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderAssistantCompletion {
    pub message_id: String,
    pub completed_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProviderPromptSignalBatch {
    pub chunks: Vec<ProviderPromptChunk>,
    pub completions: Vec<ProviderAssistantCompletion>,
    pub prompt_completed: bool,
    pub terminal_failure: Option<String>,
    pub notices: Vec<String>,
    pub resolved_model: Option<String>,
    pub resolved_model_source: Option<&'static str>,
    pub resolved_variant: Option<String>,
    pub resolved_usage_tokens_total: Option<u64>,
    pub resolved_usage: Option<ProviderRunTokenUsage>,
    pub account_usage: Option<crate::account_profile::ProviderAccountUsageSnapshot>,
    pub resolved_resume_state: Option<ProviderResumeState>,
}

pub(crate) fn provider_retry_status(provider: &str, detail: Option<&str>) -> String {
    let detail = detail
        .map(str::trim)
        .filter(|detail| !detail.is_empty())
        .map(|detail| format!(" ({detail})"))
        .unwrap_or_default();
    format!("{provider} connection interrupted — retrying{detail}.")
}

pub(crate) fn classify_provider_terminal_failure_text(
    adapter_key: &str,
    text: &str,
) -> Option<String> {
    if !matches!(adapter_key, "claude" | "codex" | "opencode") {
        return None;
    }
    if let Some(failure) = classify_provider_substitutable_failure_text(adapter_key, text) {
        return Some(failure);
    }
    let normalized = text.to_lowercase();
    if provider_normalized_text_reports_resource_limit(&normalized) {
        return Some(format!(
            "Provider reported a resource limit: {}",
            compact_provider_error_snippet(text)
        ));
    }
    if adapter_key == "claude"
        && normalized.contains("dangerously-skip-permissions")
        && normalized.contains("cannot be used with root/sudo privileges")
    {
        return Some(format!(
            "Provider reported a terminal permission error: {}",
            compact_provider_error_snippet(text)
        ));
    }
    let fatal_model_error = normalized.contains("unsupported model")
        || normalized.contains("invalid model")
        || normalized.contains("model_not_found")
        || normalized.contains("model not found")
        || normalized.contains("model does not exist")
        || normalized.contains("model is not supported")
        || normalized.lines().any(|line| {
            line.contains("model")
                && (line.contains("http 400")
                    || line.contains("status 400")
                    || line.contains("400 bad request"))
        });
    if !fatal_model_error {
        return None;
    }
    Some(format!(
        "Provider reported a terminal model error: {}",
        compact_provider_error_snippet(text)
    ))
}

/// Classifies untrusted terminal output without treating ordinary assistant prose as a provider
/// failure. Structured provider error notifications use `classify_provider_terminal_failure_text`
/// directly because their provenance is already authoritative.
pub(crate) fn classify_provider_terminal_failure_output_text(
    adapter_key: &str,
    text: &str,
) -> Option<String> {
    text.lines().find_map(|line| {
        let normalized = line.trim().to_lowercase();
        let exact_provider_dialog = adapter_key == "claude"
            && claude_normalized_text_reports_resource_limit_dialog(&normalized);
        if !provider_normalized_text_has_error_frame(&normalized) && !exact_provider_dialog {
            return None;
        }
        classify_provider_terminal_failure_text(adapter_key, line)
    })
}

fn provider_normalized_text_has_error_frame(normalized: &str) -> bool {
    normalized.lines().any(|line| {
        let line = line.trim();
        line.starts_with("error:")
            || line.starts_with("error ")
            || line.starts_with("fatal:")
            || line.starts_with("fatal ")
            || line.starts_with("api error")
            || line.starts_with("codex error")
            || line.starts_with("opencode error")
            || line.starts_with("claude error")
            || (line.starts_with('{')
                && (line.contains("\"error\"") || line.contains("\"type\":\"error\"")))
    })
}

/// Only native failure hooks may supply this frame. Normal Stop output and transcript
/// prose are not failure signals, even if they contain identical words.
pub(crate) fn claude_native_stop_failure(event: &serde_json::Value) -> Option<String> {
    if event
        .get("hook_event_name")
        .and_then(serde_json::Value::as_str)
        != Some("StopFailure")
    {
        return None;
    }
    let error = event
        .get("error")
        .and_then(serde_json::Value::as_str)
        .filter(|code| {
            matches!(
                *code,
                "rate_limit"
                    | "billing_error"
                    | "authentication_failed"
                    | "oauth_org_not_allowed"
                    | "invalid_request"
                    | "model_not_found"
                    | "server_error"
                    | "max_output_tokens"
            )
        })
        .unwrap_or("unknown");
    let detail = ["last_assistant_message", "error_details"]
        .into_iter()
        .find_map(|key| {
            event
                .get(key)
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
        })
        .map(compact_provider_error_snippet)
        .unwrap_or_else(|| "Provider ended the turn with an API error".to_string());
    Some(format!("Claude StopFailure [{error}]: {detail}"))
}

pub(crate) fn classify_provider_substitutable_failure_text(
    adapter_key: &str,
    text: &str,
) -> Option<String> {
    // Prompt settlement receives the human-readable notice projected from the
    // authoritative provider failure. Remove only our exact framing so Claude's
    // anchored dialog matcher still recognizes the original limit response.
    let detail = text.trim();
    let detail = detail
        .strip_prefix("Provider prompt dispatch failed: ")
        .unwrap_or(detail);
    let detail = detail
        .strip_prefix("Provider reported a substitutable resource limit: ")
        .unwrap_or(detail);
    let normalized = detail.to_lowercase();
    let substitutable = match adapter_key {
        "codex" | "opencode" => provider_normalized_text_reports_resource_limit(&normalized),
        "claude" => {
            claude_normalized_text_reports_resource_limit_dialog(&normalized)
                || normalized.starts_with("claude stopfailure [rate_limit]: ")
                || normalized.starts_with("claude stopfailure [billing_error]: ")
        }
        _ => false,
    };
    if !substitutable {
        return None;
    }
    Some(format!(
        "Provider reported a substitutable resource limit: {}",
        compact_provider_error_snippet(detail)
    ))
}

fn provider_normalized_text_reports_resource_limit(normalized: &str) -> bool {
    let quota_or_billing = normalized.contains("insufficient_quota")
        || normalized.contains("quota exceeded")
        || normalized.contains("exceeded your current quota")
        || normalized.contains("billing hard limit")
        || normalized.contains("billing limit")
        || normalized.contains("insufficient balance")
        || normalized.contains("manage your billing")
        || normalized.contains("spend limit")
        || normalized.contains("usage limit")
        || normalized.contains("monthly limit")
        || normalized.contains("no credits")
        || normalized.contains("not enough credits")
        || normalized.contains("don't have usage credits")
        || normalized.contains("do not have usage credits")
        || normalized.contains("don’t have usage credits")
        || normalized.contains("don'thaveusagecredits")
        || normalized.contains("don’thaveusagecredits")
        || normalized.contains("donothaveusagecredits")
        || normalized.contains("credits exhausted")
        || normalized.contains("credit balance")
        || normalized.contains("out of credits");
    let rate_or_run_limit = normalized.contains("rate_limit_exceeded")
        || normalized.contains("rate limit exceeded")
        || normalized.contains("rate limited")
        || normalized.contains("too many requests")
        || normalized.contains("http 429")
        || normalized.contains("status 429")
        || normalized.contains("429 too many requests")
        || normalized.contains("run limit")
        || normalized.contains("runs limit")
        || normalized.contains("turn limit");
    quota_or_billing || rate_or_run_limit
}

fn claude_normalized_text_reports_resource_limit_dialog(normalized: &str) -> bool {
    normalized
        .trim_start()
        .starts_with("you've hit your usage limit")
        || normalized
            .trim_start()
            .starts_with("you have hit your usage limit")
        || normalized
            .trim_start()
            .starts_with("you've hit your session limit")
        || normalized
            .trim_start()
            .starts_with("you have hit your session limit")
        || (normalized
            .trim_start()
            .starts_with("fable 5 now uses usage credits")
            && (normalized.contains("don't have usage credits")
                || normalized.contains("don’t have usage credits")))
        || (normalized
            .trim_start()
            .starts_with("fable5nowusesusagecredits")
            && (normalized.contains("don'thaveusagecredits")
                || normalized.contains("don’thaveusagecredits")
                || normalized.contains("donothaveusagecredits")))
}

fn compact_provider_error_snippet(text: &str) -> String {
    let mut seen_lines = std::collections::BTreeSet::new();
    let mut snippet = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| seen_lines.insert((*line).to_string()))
        .flat_map(str::split_whitespace)
        .collect::<Vec<_>>()
        .join(" ");
    const MAX_CHARS: usize = 500;
    if snippet.chars().count() > MAX_CHARS {
        snippet = snippet.chars().take(MAX_CHARS).collect::<String>();
        snippet.push_str("...");
    }
    snippet
}

#[cfg(test)]
mod tests {
    use super::{
        classify_provider_substitutable_failure_text,
        classify_provider_terminal_failure_output_text, classify_provider_terminal_failure_text,
        claude_native_stop_failure, provider_retry_status,
    };

    #[test]
    fn retry_status_uses_one_provider_neutral_message_shape() {
        assert_eq!(
            provider_retry_status("Codex", Some("2/5")),
            "Codex connection interrupted — retrying (2/5)."
        );
        assert_eq!(
            provider_retry_status("OpenCode", None),
            "OpenCode connection interrupted — retrying."
        );
    }

    #[test]
    fn classifier_detects_provider_model_rejection_text() {
        let failure = classify_provider_terminal_failure_text(
            "codex",
            "Error: HTTP 400 Bad Request: unsupported model gpt-5.2-codex",
        )
        .expect("model rejection text should be classified");

        assert!(failure.contains("terminal model error"));
        assert!(failure.contains("gpt-5.2-codex"));
    }

    #[test]
    fn classifier_ignores_non_provider_text() {
        assert!(classify_provider_terminal_failure_text(
            "dev-stub",
            "unsupported model gpt-5.2-codex"
        )
        .is_none());
        assert!(
            classify_provider_terminal_failure_text("codex", "normal assistant output").is_none()
        );
    }

    #[test]
    fn classifier_ignores_reviewer_prose_about_unsupported_findings_and_schema_models() {
        let review = "Two reviewer findings are not supported by the code at this exact head.\n\
            The merge has conflicts in packages/db/prisma/models.prisma and schema.prisma.";

        assert!(classify_provider_terminal_failure_output_text("claude", review).is_none());

        let model_classifier_review = "The implementation treats any assistant prose containing \
            unsupported model, invalid model, model_not_found, or model not found as a terminal \
            provider error. Even a review discussing the HTTP 400 test would terminate the run.";
        assert!(
            classify_provider_terminal_failure_output_text("claude", model_classifier_review)
                .is_none(),
            "ordinary reviewer prose must not be reinterpreted as a provider transport error"
        );

        let quota_classifier_review = "The usage limit and insufficient_quota classifiers are \
            intentionally discussed in this review; that prose is not a provider billing error.";
        assert!(
            classify_provider_terminal_failure_output_text("claude", quota_classifier_review)
                .is_none(),
            "ordinary reviewer prose must not be reinterpreted as a provider quota error"
        );

        let framed_review = "Error: this classifier is intentionally under review.\n\
            The phrase unsupported model is reviewer prose, not a provider response.";
        assert!(
            classify_provider_terminal_failure_output_text("claude", framed_review).is_none(),
            "an unrelated framed line must not lend error provenance to another line"
        );

        assert!(classify_provider_terminal_failure_output_text(
            "codex",
            "Error: HTTP 400 Bad Request: unsupported model gpt-5.2-codex",
        )
        .is_some());
    }

    #[test]
    fn substitute_classifier_detects_shared_quota_and_limit_errors() {
        let codex_failure = classify_provider_substitutable_failure_text(
            "codex",
            "insufficient_quota: You exceeded your current quota.",
        )
        .expect("codex quota error should be substitutable");
        assert!(codex_failure.contains("substitutable resource limit"));

        let opencode_failure = classify_provider_substitutable_failure_text(
            "opencode",
            "OpenCode error: No credits available for this account",
        )
        .expect("opencode credit error should be substitutable");
        assert!(opencode_failure.contains("No credits"));

        let opencode_balance_failure = classify_provider_substitutable_failure_text(
            "opencode",
            "Insufficient balance. Manage your billing here: https://opencode.ai/workspace/wrk/billing",
        )
        .expect("opencode balance error should be substitutable");
        assert!(opencode_balance_failure.contains("Insufficient balance"));
    }

    #[test]
    fn substitute_classifier_detects_claude_usage_limit() {
        let failure = classify_provider_terminal_failure_text(
            "claude",
            "You've hit your usage limit. Your limit will reset later.",
        )
        .expect("Claude usage limit should be terminal");

        assert!(failure.contains("resource limit"));
        assert!(failure.contains("You've hit your usage limit"));
        let substitute_failure =
            classify_provider_substitutable_failure_text("claude", "You've hit your usage limit.")
                .expect("Claude usage limit should activate an available substitute");
        assert!(substitute_failure.contains("substitutable resource limit"));
    }

    #[test]
    fn claude_stop_failure_uses_authoritative_code_not_assistant_prose() {
        for code in [
            "rate_limit",
            "billing_error",
            "authentication_failed",
            "server_error",
            "unknown",
        ] {
            let event = serde_json::json!({
                "hook_event_name": "StopFailure",
                "error": code,
                "last_assistant_message": "You've hit your session limit · resets 4am"
            });
            let failure = claude_native_stop_failure(&event).unwrap();
            assert_eq!(
                classify_provider_substitutable_failure_text("claude", &failure).is_some(),
                matches!(code, "rate_limit" | "billing_error")
            );
            assert!(
                classify_provider_terminal_failure_output_text("claude", &failure).is_none(),
                "plain assistant text imitating a hook frame is not authoritative"
            );
        }
        for event_name in ["Stop", "SessionEnd", "assistant"] {
            assert!(claude_native_stop_failure(&serde_json::json!({
                "hook_event_name": event_name,
                "error": "rate_limit",
                "last_assistant_message": "You've hit your session limit"
            }))
            .is_none());
        }
        let failure = claude_native_stop_failure(&serde_json::json!({
            "hook_event_name": "StopFailure", "error": "rate_limit"
        }))
        .unwrap();
        assert!(
            classify_provider_substitutable_failure_text("claude", &failure).is_some(),
            "documented optional error text must not disable failure handling"
        );
        let failure = claude_native_stop_failure(&serde_json::json!({
            "hook_event_name": "StopFailure", "error": "billing_error", "error_details": "insufficient credit"
        })).unwrap();
        assert!(failure.ends_with("insufficient credit"));
    }

    #[test]
    fn substitute_classifier_detects_claude_session_limit_dialog() {
        let dialog = "You've hit your session limit · resets 3:50am (Europe/Madrid)";

        let terminal_failure = classify_provider_terminal_failure_output_text("claude", dialog)
            .expect("Claude's session-limit dialog should terminate the provider turn");
        assert!(terminal_failure.contains("substitutable resource limit"));
        assert!(terminal_failure.contains("session limit"));

        let substitute_failure = classify_provider_substitutable_failure_text("claude", dialog)
            .expect("Claude's session-limit dialog should advance the substitute chain");
        assert!(substitute_failure.contains("substitutable resource limit"));
    }

    #[test]
    fn substitute_classifier_preserves_claude_failure_through_kernel_notices() {
        let dialog = "You've hit your session limit · resets 10:40pm (Europe/Madrid)";
        let failure = classify_provider_substitutable_failure_text("claude", dialog).unwrap();
        for notice in [
            failure.clone(),
            format!("Provider prompt dispatch failed: {failure}"),
        ] {
            assert_eq!(
                classify_provider_substitutable_failure_text("claude", &notice),
                Some(failure.clone())
            );
        }
        assert!(classify_provider_substitutable_failure_text(
            "claude",
            &format!("The reviewer said: {failure}")
        )
        .is_none());
        assert!(
            classify_provider_terminal_failure_output_text(
                "claude",
                &format!("Provider prompt dispatch failed: {failure}")
            )
            .is_none(),
            "ordinary output must not impersonate an authoritative kernel notice"
        );
    }

    #[test]
    fn terminal_classifier_detects_claude_model_credit_dialog() {
        let failure = classify_provider_terminal_failure_text(
            "claude",
            "Fable 5 now uses usage credits. You don't have usage credits yet.\n\
             1. Set up usage credits on claude.ai\n\
             2. Switch to Sonnet 5 and continue",
        )
        .expect("Claude model credit dialog should be terminal");

        assert!(failure.contains("resource limit"));
        assert!(failure.contains("don't have usage credits"));
        assert!(classify_provider_substitutable_failure_text(
            "claude",
            "Fable 5 now uses usage credits. You don't have usage credits yet."
        )
        .is_some());

        assert!(classify_provider_terminal_failure_text(
            "claude",
            "Fable5nowusesusagecredits Youdon'thaveusagecreditsyet",
        )
        .is_some());
    }

    #[test]
    fn substitute_classifier_ignores_non_usage_claude_failures() {
        for text in [
            "Claude error: connection refused",
            "Claude error: unauthorized",
            "Claude error: unsupported model",
            "Claude error: HTTP 429 Too Many Requests",
        ] {
            assert!(classify_provider_substitutable_failure_text("claude", text).is_none());
        }
    }

    #[test]
    fn terminal_classifier_preserves_claude_root_permission_restriction() {
        let failure = classify_provider_terminal_failure_text(
            "claude",
            "Error: --dangerously-skip-permissions cannot be used with root/sudo privileges",
        )
        .expect("Claude root permission restriction should be terminal");

        assert!(failure.contains("terminal permission error"));
        assert!(failure.contains("--dangerously-skip-permissions"));
        assert!(failure.contains("root/sudo privileges"));
    }

    #[test]
    fn terminal_classifier_deduplicates_repeated_provider_lines() {
        let repeated = "--dangerously-skip-permissions cannot be used with root/sudo privileges";
        let failure =
            classify_provider_terminal_failure_text("claude", &format!("{repeated}\n{repeated}"))
                .expect("Claude root permission restriction should be terminal");

        assert_eq!(failure.matches(repeated).count(), 1, "{failure}");
    }

    #[test]
    fn substitute_classifier_ignores_model_auth_and_network_errors() {
        assert!(classify_provider_substitutable_failure_text(
            "codex",
            "HTTP 400 Bad Request: unsupported model gpt-5.2-codex"
        )
        .is_none());
        assert!(classify_provider_substitutable_failure_text(
            "opencode",
            "Authentication required. Please login."
        )
        .is_none());
        assert!(
            classify_provider_substitutable_failure_text("codex", "connection refused").is_none()
        );
    }
}
