use serde_json::Value;

use crate::session::unix_epoch_ms;
use crate::terminal::TerminalOutputKind;

use super::super::{
    ProviderAssistantCompletion, ProviderPromptChunk, ProviderPromptSignalBatch,
    ProviderResumeState, ProviderRunTokenUsage,
};
use super::usage::merge_claude_account_usage;
use super::ClaudeRuntimeState;

pub(super) fn apply_claude_message(
    provider_run_id: &str,
    state: &mut ClaudeRuntimeState,
    value: Value,
    batch: &mut ProviderPromptSignalBatch,
) {
    let Some(kind) = value.get("type").and_then(Value::as_str) else {
        return;
    };
    match kind {
        "system" => apply_system_message(state, &value, batch),
        "stream_event" => apply_stream_event(provider_run_id, state, &value, batch),
        "assistant" => apply_assistant_message(provider_run_id, state, &value, batch),
        "result" => apply_result_message(state, &value, batch),
        "rate_limit_event" => apply_rate_limit_event(&value, batch),
        _ => {}
    }
}

fn apply_system_message(
    state: &mut ClaudeRuntimeState,
    value: &Value,
    batch: &mut ProviderPromptSignalBatch,
) {
    if let Some(session_id) =
        string_field(value, "session_id").or_else(|| string_field(value, "sessionId"))
    {
        record_claude_session_id(state, batch, session_id);
    }
    if batch.resolved_model.is_none() {
        if let Some(model) = string_field(value, "model") {
            batch.resolved_model = Some(format_claude_model(&model));
            batch.resolved_model_source = Some("claude.system");
        }
    }
}

fn apply_stream_event(
    provider_run_id: &str,
    state: &mut ClaudeRuntimeState,
    value: &Value,
    batch: &mut ProviderPromptSignalBatch,
) {
    let event = value.get("event").unwrap_or(value);
    let event_kind = event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if event_kind == "rate_limit_event" {
        apply_rate_limit_event(event, batch);
        return;
    }
    if event_kind == "message_start" {
        state.active_stream_message_id = event
            .get("message")
            .and_then(|message| message.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(model) = event
            .get("message")
            .and_then(|message| message.get("model"))
            .and_then(Value::as_str)
        {
            batch.resolved_model = Some(format_claude_model(model));
            batch.resolved_model_source = Some("claude.stream_event");
        }
    }
    if event_kind == "content_block_start" {
        if let Some(block) = event.get("content_block") {
            let block_kind = block.get("type").and_then(Value::as_str).unwrap_or("text");
            if let Some(text) = claude_block_text(block, block_kind) {
                emit_authoritative_text(
                    provider_run_id,
                    state,
                    batch,
                    &claude_stream_block_key(state, event, block_kind),
                    block_kind,
                    text,
                );
            }
        }
    }
    if event_kind == "content_block_delta" {
        let Some(delta) = event.get("delta") else {
            return;
        };
        match delta
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "text_delta" => {
                if let Some(text) = delta.get("text").and_then(Value::as_str) {
                    emit_stream_text_delta(
                        provider_run_id,
                        state,
                        batch,
                        &claude_stream_block_key(state, event, "text"),
                        "text",
                        text,
                    );
                }
            }
            "thinking_delta" => {
                if let Some(text) = delta
                    .get("thinking")
                    .or_else(|| delta.get("text"))
                    .and_then(Value::as_str)
                {
                    emit_stream_text_delta(
                        provider_run_id,
                        state,
                        batch,
                        &claude_stream_block_key(state, event, "thinking"),
                        "thinking",
                        text,
                    );
                }
            }
            _ => {}
        }
    }
    if event_kind == "message_delta" {
        if let Some(usage) = event
            .get("usage")
            .or_else(|| {
                event
                    .get("message")
                    .and_then(|message| message.get("usage"))
            })
            .and_then(usage_from_value)
        {
            batch.resolved_usage_tokens_total = usage.total_tokens;
            batch.resolved_usage = Some(usage);
        }
    }
}

fn apply_rate_limit_event(value: &Value, batch: &mut ProviderPromptSignalBatch) {
    use crate::account_profile::{
        ProviderAccountUsageAvailability, ProviderAccountUsageMeter, ProviderAccountUsageMeterKind,
        ProviderAccountUsageMeterScope, ProviderAccountUsageMeterState,
        ProviderAccountUsageSnapshot,
    };

    let info = value
        .get("rate_limit_info")
        .or_else(|| value.get("rateLimitInfo"))
        .unwrap_or(value);
    let observed_at_ms = unix_epoch_ms();
    let limit_type = string_field(info, "rate_limit_type")
        .or_else(|| string_field(info, "rateLimitType"))
        .unwrap_or_else(|| "account".to_string());
    let utilization = info
        .get("utilization")
        .and_then(Value::as_f64)
        .map(|value| if value <= 1.0 { value * 100.0 } else { value });
    let status = string_field(info, "status").unwrap_or_else(|| "unknown".to_string());
    let overage_status =
        string_field(info, "overage_status").or_else(|| string_field(info, "overageStatus"));
    let exhausted = status.eq_ignore_ascii_case("rejected")
        || status.eq_ignore_ascii_case("exhausted")
        || overage_status
            .as_deref()
            .is_some_and(|status| status.eq_ignore_ascii_case("exhausted"));
    let state = if exhausted || utilization.is_some_and(|value| value >= 100.0) {
        ProviderAccountUsageMeterState::Exhausted
    } else if utilization.is_some_and(|value| value >= 80.0) {
        ProviderAccountUsageMeterState::Warning
    } else if utilization.is_some() || status.eq_ignore_ascii_case("allowed") {
        ProviderAccountUsageMeterState::Healthy
    } else {
        ProviderAccountUsageMeterState::Unknown
    };
    let window_duration_minutes = match limit_type.as_str() {
        "five_hour" | "five-hour" => Some(5 * 60),
        "seven_day" | "seven-day" => Some(7 * 24 * 60),
        _ => info
            .get("window_duration_minutes")
            .or_else(|| info.get("windowDurationMinutes"))
            .and_then(Value::as_u64),
    };
    let resets_at_ms = info
        .get("resets_at")
        .or_else(|| info.get("resetsAt"))
        .and_then(super::usage::timestamp_ms);
    let scope = if limit_type.to_ascii_lowercase().contains("model") {
        ProviderAccountUsageMeterScope::Model
    } else {
        ProviderAccountUsageMeterScope::Account
    };
    let snapshot = ProviderAccountUsageSnapshot {
        // The run-owning kernel replaces this placeholder with the selected
        // stable profile ID before persisting it.
        profile_id: String::new(),
        provider: "claude".to_string(),
        availability: ProviderAccountUsageAvailability::Available,
        meters: vec![ProviderAccountUsageMeter {
            meter_id: format!("rate_limit/{limit_type}"),
            label: match limit_type.as_str() {
                "five_hour" | "five-hour" => "5-hour".to_string(),
                "seven_day" | "seven-day" => "Weekly".to_string(),
                _ => limit_type.replace(['_', '-'], " "),
            },
            service_id: None,
            kind: ProviderAccountUsageMeterKind::RollingLimit,
            scope,
            used_percent: utilization,
            used: None,
            remaining: None,
            total: None,
            unit: None,
            window_duration_minutes,
            resets_at_ms,
            state,
            source: "claude.rate_limit_event".to_string(),
            observed_at_ms,
        }],
        observed_at_ms: Some(observed_at_ms),
        source: "claude.rate_limit_event".to_string(),
        management_url: Some("https://claude.ai/settings/usage".to_string()),
    };
    merge_claude_account_usage(&mut batch.account_usage, snapshot);
}

fn apply_assistant_message(
    provider_run_id: &str,
    state: &mut ClaudeRuntimeState,
    value: &Value,
    batch: &mut ProviderPromptSignalBatch,
) {
    let message = value.get("message").unwrap_or(value);
    if batch.resolved_model.is_none() {
        if let Some(model) = message.get("model").and_then(Value::as_str) {
            batch.resolved_model = Some(format_claude_model(model));
            batch.resolved_model_source = Some("claude.assistant");
        }
    }
    if let Some(usage) = message.get("usage").and_then(usage_from_value) {
        batch.resolved_usage_tokens_total = usage.total_tokens;
        batch.resolved_usage = Some(usage);
    }
    if let Some(content) = message.get("content").and_then(Value::as_array) {
        for (index, block) in content.iter().enumerate() {
            let block_kind = block
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let Some(text) = claude_block_text(block, block_kind) else {
                continue;
            };
            let key = claude_assistant_block_key(state, message, block_kind, index);
            emit_authoritative_text(provider_run_id, state, batch, &key, block_kind, text);
            if state
                .emitted_text_by_block
                .get(&key)
                .is_some_and(|emitted| emitted == text)
            {
                state.completed_text_blocks.insert(key);
            }
        }
    }
}

fn apply_result_message(
    state: &mut ClaudeRuntimeState,
    value: &Value,
    batch: &mut ProviderPromptSignalBatch,
) {
    if let Some(session_id) = string_field(value, "session_id") {
        record_claude_session_id(state, batch, session_id);
    }
    if let Some(usage) = value.get("usage").and_then(usage_from_value) {
        batch.resolved_usage_tokens_total = usage.total_tokens;
        batch.resolved_usage = Some(usage);
    }
    let subtype = value
        .get("subtype")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let is_error = value
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(subtype != "success");
    if is_error {
        batch.terminal_failure = Some(
            value
                .get("error")
                .and_then(Value::as_str)
                .or_else(|| value.get("result").and_then(Value::as_str))
                .unwrap_or("Claude Code reported an error")
                .to_string(),
        );
    }
    let message_id = state
        .session_id
        .as_ref()
        .map(|session_id| format!("claude:{session_id}:{}", state.result_number))
        .unwrap_or_else(|| format!("claude:result:{}", state.result_number));
    state.result_number += 1;
    batch.completions.push(ProviderAssistantCompletion {
        message_id,
        completed_at_ms: unix_epoch_ms(),
    });
    batch.prompt_completed = true;
    state.active_turn_id = None;
}

fn record_claude_session_id(
    state: &mut ClaudeRuntimeState,
    batch: &mut ProviderPromptSignalBatch,
    session_id: String,
) {
    if state.session_id.as_deref() != Some(session_id.as_str()) {
        state.session_id = Some(session_id.clone());
    }
    if state.active_prompt_message.is_some() {
        batch.resolved_resume_state = Some(ProviderResumeState::from_claude_session_id(session_id));
    }
}

fn emit_stream_text_delta(
    provider_run_id: &str,
    state: &mut ClaudeRuntimeState,
    batch: &mut ProviderPromptSignalBatch,
    key: &str,
    block_kind: &str,
    text: &str,
) {
    if text.is_empty() || state.completed_text_blocks.contains(key) {
        return;
    }
    state
        .emitted_text_by_block
        .entry(key.to_string())
        .or_default()
        .push_str(text);
    match block_kind {
        "thinking" => push_reasoning_chunk(provider_run_id, batch, text),
        _ => push_text_chunk(provider_run_id, batch, text),
    }
}

fn emit_authoritative_text(
    provider_run_id: &str,
    state: &mut ClaudeRuntimeState,
    batch: &mut ProviderPromptSignalBatch,
    key: &str,
    block_kind: &str,
    text: &str,
) {
    if text.is_empty() {
        return;
    }
    let emitted = state
        .emitted_text_by_block
        .entry(key.to_string())
        .or_default();
    if text == emitted.as_str() || emitted.starts_with(text) {
        return;
    }
    let Some(suffix) = text.strip_prefix(emitted.as_str()) else {
        crate::logging::debug_with_fields(
            "daemon.provider.claude",
            "Claude assistant snapshot did not match the streamed prefix",
            serde_json::json!({
                "block_key": key,
                "streamed_len": emitted.len(),
                "completed_len": text.len(),
            }),
        );
        return;
    };
    match block_kind {
        "thinking" => push_reasoning_chunk(provider_run_id, batch, suffix),
        _ => push_text_chunk(provider_run_id, batch, suffix),
    }
    *emitted = text.to_string();
}

fn claude_stream_block_key(state: &ClaudeRuntimeState, event: &Value, block_kind: &str) -> String {
    let index = event.get("index").and_then(Value::as_u64).unwrap_or(0);
    claude_scoped_block_key(state.active_stream_message_id.as_deref(), block_kind, index)
}

fn claude_assistant_block_key(
    state: &mut ClaudeRuntimeState,
    message: &Value,
    block_kind: &str,
    index: usize,
) -> String {
    let message_id = message.get("id").and_then(Value::as_str);
    let key = claude_scoped_block_key(message_id, block_kind, index as u64);
    if message_id.is_some() && !state.emitted_text_by_block.contains_key(&key) {
        let legacy_key = claude_scoped_block_key(None, block_kind, index as u64);
        if let Some(emitted) = state.emitted_text_by_block.remove(&legacy_key) {
            state.emitted_text_by_block.insert(key.clone(), emitted);
        }
        if state.completed_text_blocks.remove(&legacy_key) {
            state.completed_text_blocks.insert(key.clone());
        }
    }
    key
}

fn claude_scoped_block_key(message_id: Option<&str>, block_kind: &str, index: u64) -> String {
    match message_id {
        Some(message_id) => format!("message:{message_id}:{block_kind}:{index}"),
        None => format!("legacy:{block_kind}:{index}"),
    }
}

fn claude_block_text<'a>(block: &'a Value, block_kind: &str) -> Option<&'a str> {
    match block_kind {
        "thinking" => block
            .get("thinking")
            .or_else(|| block.get("text"))
            .and_then(Value::as_str),
        _ => block.get("text").and_then(Value::as_str),
    }
}

fn push_text_chunk(provider_run_id: &str, batch: &mut ProviderPromptSignalBatch, text: &str) {
    if text.is_empty() {
        return;
    }
    batch.chunks.push(ProviderPromptChunk {
        kind: TerminalOutputKind::ProviderOutput,
        merge_key: Some(format!("claude:{provider_run_id}:assistant")),
        bytes: text.as_bytes().to_vec(),
    });
}

fn push_reasoning_chunk(provider_run_id: &str, batch: &mut ProviderPromptSignalBatch, text: &str) {
    if text.is_empty() {
        return;
    }
    batch.chunks.push(ProviderPromptChunk {
        kind: TerminalOutputKind::ProviderReasoning,
        merge_key: Some(format!("claude:{provider_run_id}:reasoning")),
        bytes: text.as_bytes().to_vec(),
    });
}

fn string_field(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn format_claude_model(model: &str) -> String {
    let model = model.trim();
    if model.starts_with("claude/") {
        model.to_string()
    } else {
        format!("claude/{model}")
    }
}

fn usage_from_value(value: &Value) -> Option<ProviderRunTokenUsage> {
    let input = u64_field(value, "input_tokens")
        .or_else(|| u64_field(value, "input"))
        .unwrap_or_default();
    let output = u64_field(value, "output_tokens")
        .or_else(|| u64_field(value, "output"))
        .unwrap_or_default();
    let cache_create = u64_field(value, "cache_creation_input_tokens").unwrap_or_default();
    let cache_read = u64_field(value, "cache_read_input_tokens").unwrap_or_default();
    let total = u64_field(value, "total_tokens")
        .unwrap_or_else(|| input + output + cache_create + cache_read);
    (total > 0).then_some(ProviderRunTokenUsage {
        total_tokens: Some(total),
        last_tokens: Some(output),
        context_tokens: Some(input + cache_create + cache_read),
        context_window: None,
    })
}

fn u64_field(value: &Value, field: &str) -> Option<u64> {
    value.get(field).and_then(Value::as_u64)
}
