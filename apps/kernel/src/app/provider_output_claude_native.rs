use std::fs;
use std::path::PathBuf;

use serde_json::Value;

use super::provider_output_fanout::ProviderOutputFanout;
use crate::app::DaemonApp;
use crate::app::KernelPromptDispatch;
use crate::error::DaemonError;
use crate::provider::{
    ProviderNativeInteractionBridge, ProviderPromptSignalBatch, ProviderResumeState,
    RuntimeProviderRun,
};
use crate::session::{
    unix_epoch_ms, PromptAttachment, RuntimeInteraction, RuntimeInteractionChoice,
    RuntimeInteractionChoiceStyle, RuntimeInteractionKind, RuntimeInteractionLevel,
};
use crate::terminal::TerminalOutputKind;

mod attachments;
mod permission;
#[cfg(test)]
mod tests;
mod transcript;

use attachments::{
    extract_claude_native_prompt_attachments, format_claude_attachment_context,
    format_claude_native_attachment_prompt_suffix, join_claude_context,
};
use permission::{
    append_claude_headless_debug, claude_headless_bypass_confirmation_visible,
    claude_headless_bypass_selection_pending, claude_headless_composer_visible,
    claude_headless_prompt_waiting_in_composer, claude_headless_workspace_trust_visible,
    claude_native_marker, claude_permission_recent_file, claude_rendered_permission_visible,
    claude_yolo_rendered_permission_confirmation_pending, clear_claude_hook_permission_tombstone,
    clear_claude_permission_recent, clear_claude_yolo_rendered_permission_confirmation,
    extract_native_hidden_instructions, format_claude_permission_message,
    mark_claude_yolo_rendered_permission_confirmed, normalize_claude_visible_prompt_for_headless,
    read_claude_headless_submit_retry, redact_native_hidden_instructions,
    should_bridge_claude_permission, take_claude_permission_inputs,
    take_matching_claude_hook_permission_tombstone, timestamp_millis,
    update_claude_permission_recent, write_claude_headless_bypass_selection_marker,
    write_claude_headless_startup_wait_marker, write_claude_headless_submit_retry,
    write_claude_hook_context_response, write_claude_hook_permission_tombstone,
    write_claude_native_marker, write_claude_permission_input, write_claude_permission_response,
};
#[cfg(test)]
use transcript::drain_claude_transcript_file;
use transcript::{
    drain_claude_transcript_file_since, known_claude_transcript_paths,
    load_claude_transcript_cursor, save_claude_transcript_cursor,
};

const CLAUDE_ATTACHMENT_CONTEXT_BYTES: usize = 64 * 1024;
const CLAUDE_TRANSCRIPT_STOP_DRAIN_MS: u64 = 300;
const CLAUDE_TRANSCRIPT_STOP_DRAIN_MARKER_PREFIX: &str = "stop-draining:";

/// Delay between writing a prompt's visible text into the provider PTY and
/// sending the Enter keystroke, giving the terminal time to register the
/// (possibly multi-line, bracket-pasted) text before it is submitted. This
/// wait is taken between short app-lock holds by the async dispatch retry
/// loop rather than by sleeping inside the lock.
const CLAUDE_SUBMIT_DELAY_MS: u64 = 250;

struct ClaudeNativePromptInjection<'a> {
    id: &'a str,
    prompt: &'a str,
    hidden_system_context: &'a str,
    attachments: &'a [PromptAttachment],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClaudeNativeDispatchAttempt {
    Completed,
    AwaitingInjection,
}

const CLAUDE_HEADLESS_SUBMIT_RETRY_LIMIT: u8 = 10;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ClaudeNativeProcessOutcome {
    /// A managed Claude run reported Stop/SessionEnd this pass. The caller
    /// should drain its transcript once more after a short delay taken off the
    /// app lock, capturing the final assistant flush before settlement.
    pub(crate) needs_deferred_transcript_drain: bool,
    /// Provider-native failure, settled by the caller's normal failure/substitution path.
    pub(crate) terminal_failure: Option<String>,
}

pub(crate) struct ProviderOutputClaudeNativeBridge<'a> {
    app: &'a mut DaemonApp,
}

pub(crate) fn claude_native_recent_terminal_failure(
    provider_run: &RuntimeProviderRun,
) -> Option<String> {
    let context_file = provider_run
        .pty_env()
        .get("CHARIOX_CLAUDE_NATIVE_CONTEXT")?;
    let recent_failure_file = claude_permission_recent_file(context_file)?;
    let recent_failure = fs::read_to_string(recent_failure_file).ok()?;
    crate::provider::classify_provider_terminal_failure_text(
        provider_run.adapter_key(),
        &recent_failure,
    )
}

fn claude_native_history_source_attachment_id(
    app: &DaemonApp,
    session_id: &str,
    provider_run_id: &str,
    fallback_attachment_id: &str,
) -> String {
    app.terminal()
        .input_records()
        .into_iter()
        .rev()
        .find(|record| record.session_id == session_id && record.provider_run_id == provider_run_id)
        .map(|record| record.source_attachment_id)
        .unwrap_or_else(|| fallback_attachment_id.to_string())
}

fn claude_headless_prompt_input(
    prompt: &ClaudeNativePromptInjection<'_>,
    context_file: &str,
) -> String {
    let native_attachment_suffix =
        format_claude_native_attachment_prompt_suffix(prompt.attachments, context_file);
    let visible = redact_native_hidden_instructions(prompt.prompt)
        .trim()
        .to_string();
    normalize_claude_visible_prompt_for_headless(&join_claude_context([
        native_attachment_suffix,
        visible,
    ]))
}

fn claude_headless_prompt_matches(expected: &str, observed: &str) -> bool {
    let normalize = |value: &str| value.replace("\r\n", "\n").replace('\r', "\n");
    normalize(expected).trim() == normalize(observed).trim()
}

fn claude_native_prompt_is_internal_control(prompt: &str) -> bool {
    crate::provider::ExternalProviderObservationPolicy::for_provider("claude")
        .user_prompt_is_internal_control(prompt)
}

fn claude_headless_dispatch_matches_prompt(
    context_file: &str,
    dispatch_prompt_id: &str,
    observed_prompt: &str,
) -> bool {
    let retry = read_claude_headless_submit_retry(context_file);
    retry.prompt_id == dispatch_prompt_id
        && !retry.visible_prompt.is_empty()
        && claude_headless_prompt_matches(&retry.visible_prompt, observed_prompt)
}

fn acknowledge_claude_headless_dispatch_from_hook_events(
    context_file: &str,
    events_file: &str,
    dispatch_prompt_id: &str,
) {
    let raw = fs::read_to_string(events_file).unwrap_or_default();
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if event.get("hook_event_name").and_then(Value::as_str) != Some("UserPromptSubmit") {
            continue;
        }
        let Some(observed_prompt) = event
            .get("prompt")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|prompt| !prompt.is_empty())
        else {
            continue;
        };
        if !claude_native_prompt_is_internal_control(observed_prompt)
            && claude_headless_dispatch_matches_prompt(
                context_file,
                dispatch_prompt_id,
                observed_prompt,
            )
        {
            write_claude_native_marker(context_file, &format!("accepted:{dispatch_prompt_id}"));
            return;
        }
    }
}

fn acknowledge_claude_headless_steering_enqueue(
    context_file: &str,
    active_prompt_id: Option<&str>,
    enqueued_prompts: &[String],
) {
    let Some(active_prompt_id) = active_prompt_id else {
        return;
    };
    let Some(marker) = claude_native_marker(context_file) else {
        return;
    };
    let Some(dispatch_prompt_id) = marker.strip_prefix("injected:") else {
        return;
    };
    if dispatch_prompt_id.is_empty() || dispatch_prompt_id == active_prompt_id {
        return;
    }
    if enqueued_prompts.iter().any(|prompt| {
        claude_headless_dispatch_matches_prompt(context_file, dispatch_prompt_id, prompt)
    }) {
        write_claude_native_marker(context_file, &format!("accepted:{dispatch_prompt_id}"));
    }
}

impl<'a> ProviderOutputClaudeNativeBridge<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn process(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        provider_run: &RuntimeProviderRun,
        native_interaction_bridge: Option<std::sync::Arc<dyn ProviderNativeInteractionBridge>>,
    ) -> Result<ClaudeNativeProcessOutcome, DaemonError> {
        let mut outcome = ClaudeNativeProcessOutcome::default();
        let Some(agent_id) = provider_run.agent_instance_id().map(str::to_string) else {
            return Ok(outcome);
        };
        let Some(events_file) = provider_run.pty_env().get("CHARIOX_CLAUDE_NATIVE_EVENTS") else {
            return Ok(outcome);
        };
        let Some(context_file) = provider_run.pty_env().get("CHARIOX_CLAUDE_NATIVE_CONTEXT") else {
            return Ok(outcome);
        };

        let resolving_permission = claude_native_marker(context_file)
            .as_deref()
            .is_some_and(|value| value.starts_with("permission:"));
        let resolved_prompt_marker = if resolving_permission {
            self.app
                .prompt_owner_active_prompt_for_agent(session_id, &agent_id)?
                .map(|prompt| format!("permission-resolved:{}", prompt.id()))
        } else {
            None
        };
        for input in take_claude_permission_inputs(context_file) {
            self.app
                .write_provider_pty_input_for_runtime(provider_run_id, &input)?;
            if resolving_permission {
                write_claude_native_marker(
                    context_file,
                    resolved_prompt_marker.as_deref().unwrap_or_default(),
                );
            }
        }
        if let Some(settled) = self.process_pending_claude_stop(
            session_id,
            provider_run_id,
            &agent_id,
            context_file,
            provider_run.provider() == "claude-headless",
            false,
        )? {
            outcome.needs_deferred_transcript_drain = !settled;
            return Ok(outcome);
        }
        self.inject_pending_prompt(
            session_id,
            provider_run_id,
            &agent_id,
            context_file,
            provider_run,
        )?;
        self.drain_known_claude_transcripts(session_id, provider_run_id, context_file)?;
        self.process_claude_account_usage(provider_run)?;

        let events_path = std::path::Path::new(events_file);
        let raw = fs::read_to_string(events_path).unwrap_or_default();
        if raw.trim().is_empty() {
            return Ok(outcome);
        }
        let _ = fs::write(events_path, "");
        let runtime_attachment_id = self
            .app
            .attachments
            .list_session_attachment_ids(session_id)
            .into_iter()
            .next();

        for line in raw.lines().filter(|line| !line.trim().is_empty()) {
            let Ok(event) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if let Some(transcript_path) = event
                .get("transcript_path")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                self.drain_claude_transcript(
                    session_id,
                    provider_run_id,
                    context_file,
                    transcript_path,
                )?;
            }
            let event_name = event
                .get("hook_event_name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if event_name == "UserPromptSubmit" {
                clear_claude_hook_permission_tombstone(context_file);
                let Some(prompt) = event
                    .get("prompt")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|prompt| !prompt.is_empty())
                else {
                    continue;
                };
                // Claude emits this synthetic UserPromptSubmit hook when its
                // process is interrupted. It is provider control state, not a
                // user prompt, and must never create a zero-duration turn.
                if claude_native_prompt_is_internal_control(prompt) {
                    continue;
                }
                let active_prompt = self
                    .app
                    .prompt_owner_active_prompt_for_agent(session_id, &agent_id)?;
                let marker = claude_native_marker(context_file);
                if let Some(dispatch_prompt_id) =
                    marker.as_deref().and_then(claude_native_dispatch_prompt_id)
                {
                    // Idle submissions acknowledge through UserPromptSubmit.
                    // A busy Claude run records steering as a native queue
                    // enqueue instead, but some versions also emit this hook;
                    // only accept that steering hook when its prompt matches
                    // the exact text Chariox injected. Consume mismatched/stale
                    // hook events while a Chariox prompt is active, and also
                    // consume an exact late acknowledgement after cancellation
                    // so it cannot resurrect the cancelled managed turn as an
                    // external prompt.
                    let matches_managed_dispatch = active_prompt
                        .as_ref()
                        .is_some_and(|active_prompt| dispatch_prompt_id == active_prompt.id())
                        || claude_headless_dispatch_matches_prompt(
                            context_file,
                            dispatch_prompt_id,
                            prompt,
                        );
                    if matches_managed_dispatch {
                        write_claude_native_marker(
                            context_file,
                            &format!("accepted:{dispatch_prompt_id}"),
                        );
                        continue;
                    }
                    if active_prompt.is_some() {
                        continue;
                    }
                }
                if let Some(request_id) =
                    event.get("hook_context_request_id").and_then(Value::as_str)
                {
                    let context =
                        self.claude_native_prompt_context(session_id, &agent_id, prompt)?;
                    crate::provider::ensure_claude_native_hidden_context_fits(
                        provider_run_id,
                        &context,
                    )?;
                    fs::write(context_file, &context).map_err(|error| {
                        DaemonError::ProviderProtocol {
                            provider_run_id: provider_run_id.to_string(),
                            operation: "claude_hidden_context_write",
                            message: error.to_string(),
                        }
                    })?;
                    write_claude_hook_context_response(context_file, request_id, &context);
                }
                let Some(runtime_attachment_id) = runtime_attachment_id.as_deref() else {
                    continue;
                };
                let history_source_attachment_id = claude_native_history_source_attachment_id(
                    self.app,
                    session_id,
                    provider_run_id,
                    runtime_attachment_id,
                );
                let attachments = extract_claude_native_prompt_attachments(
                    prompt,
                    provider_run.working_directory().map(PathBuf::as_path),
                );
                let outcome = self.app.record_native_prompt_started_with_attachments(
                    session_id,
                    runtime_attachment_id,
                    &history_source_attachment_id,
                    &agent_id,
                    prompt,
                    attachments,
                )?;
                if let crate::session::PromptSubmissionOutcome::Started { prompt } = outcome {
                    write_claude_native_marker(context_file, &format!("native:{}", prompt.id()));
                }
            } else if event_name == "StopFailure" {
                outcome.terminal_failure = crate::provider::claude_native_stop_failure(&event);
                return Ok(outcome);
            } else if matches!(event_name, "Stop" | "SessionEnd") {
                // Stop is the authoritative settlement signal for every
                // managed Claude interface. Drain now and once more after a
                // short off-lock delay because the final transcript flush can
                // trail the hook event.
                self.drain_known_claude_transcripts(session_id, provider_run_id, context_file)?;
                if let Some(active_prompt) = self
                    .app
                    .prompt_owner_active_prompt_for_agent(session_id, &agent_id)?
                {
                    write_claude_native_marker(
                        context_file,
                        &format!(
                            "{CLAUDE_TRANSCRIPT_STOP_DRAIN_MARKER_PREFIX}{}:{}",
                            active_prompt.id(),
                            unix_epoch_ms()
                        ),
                    );
                    outcome.needs_deferred_transcript_drain = true;
                } else {
                    let _ = fs::write(context_file, "");
                    write_claude_native_marker(context_file, "");
                }
                continue;
            } else if matches!(event_name, "PreToolUse" | "PermissionRequest") {
                self.resolve_permission_event(
                    session_id,
                    provider_run_id,
                    &agent_id,
                    context_file,
                    provider_run,
                    native_interaction_bridge.clone(),
                    &event,
                )?;
            }
        }
        self.drain_known_claude_transcripts(session_id, provider_run_id, context_file)?;
        Ok(outcome)
    }

    fn process_claude_account_usage(
        &mut self,
        provider_run: &RuntimeProviderRun,
    ) -> Result<(), DaemonError> {
        let Some(path) = provider_run.pty_env().get("CHARIOX_CLAUDE_USAGE_FILE") else {
            return Ok(());
        };
        let path = std::path::Path::new(path);
        let consumed_path = path.with_extension("consuming");
        if fs::rename(path, &consumed_path).is_err() {
            return Ok(());
        }
        let raw = fs::read_to_string(&consumed_path).unwrap_or_default();
        let _ = fs::remove_file(&consumed_path);
        if raw.trim().is_empty() {
            return Ok(());
        }
        let Ok(value) = serde_json::from_str(&raw) else {
            return Ok(());
        };
        let Some(snapshot) = crate::provider::claude_status_line_usage_snapshot(&value) else {
            return Ok(());
        };
        self.app.provider_account_profiles.update_usage(
            provider_run.owner_user_id(),
            provider_run.provider(),
            provider_run.account_profile(),
            snapshot,
        )?;
        Ok(())
    }

    pub(crate) fn finish_deferred_stop(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        provider_run: &RuntimeProviderRun,
    ) -> Result<(), DaemonError> {
        let Some(agent_id) = provider_run.agent_instance_id() else {
            return Ok(());
        };
        let Some(context_file) = provider_run.pty_env().get("CHARIOX_CLAUDE_NATIVE_CONTEXT") else {
            return Ok(());
        };
        let _ = self.process_pending_claude_stop(
            session_id,
            provider_run_id,
            agent_id,
            context_file,
            provider_run.provider() == "claude-headless",
            true,
        )?;
        Ok(())
    }

    fn process_pending_claude_stop(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        context_file: &str,
        mark_next_headless_prompt_ready: bool,
        force: bool,
    ) -> Result<Option<bool>, DaemonError> {
        let Some((prompt_id, stopped_at_ms)) = claude_transcript_stop_drain_marker(context_file)
        else {
            return Ok(None);
        };
        self.drain_known_claude_transcripts(session_id, provider_run_id, context_file)?;
        if !force && unix_epoch_ms().saturating_sub(stopped_at_ms) < CLAUDE_TRANSCRIPT_STOP_DRAIN_MS
        {
            return Ok(Some(false));
        }
        let active_prompt = self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, agent_id)?;
        if active_prompt
            .as_ref()
            .is_some_and(|active_prompt| active_prompt.id() != prompt_id)
        {
            crate::logging::warn_with_fields(
                "daemon.claude_headless",
                "ignored stale deferred stop settlement",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                    "agent_id": agent_id,
                    "stopped_prompt_id": prompt_id,
                    "active_prompt_id": active_prompt.as_ref().map(|prompt| prompt.id()),
                }),
            );
            write_claude_native_marker(context_file, "");
            return Ok(Some(true));
        }
        self.complete_native_prompt_after_stop(
            session_id,
            provider_run_id,
            agent_id,
            context_file,
            mark_next_headless_prompt_ready,
        )?;
        Ok(Some(true))
    }

    fn complete_native_prompt_after_stop(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        context_file: &str,
        mark_next_headless_prompt_ready: bool,
    ) -> Result<(), DaemonError> {
        if self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, agent_id)?
            .is_some()
        {
            self.app
                .complete_active_prompt(session_id, agent_id, Some(provider_run_id))?;
        }
        let _ = fs::write(context_file, "");
        write_claude_native_marker(context_file, "");
        if mark_next_headless_prompt_ready {
            if let Some(next_prompt) = self
                .app
                .prompt_owner_active_prompt_for_agent(session_id, agent_id)?
            {
                crate::logging::debug_with_fields(
                    "daemon.claude_headless",
                    "marked post-stop queued prompt ready",
                    serde_json::json!({
                        "session_id": session_id,
                        "provider_run_id": provider_run_id,
                        "agent_id": agent_id,
                        "prompt_id": next_prompt.id(),
                    }),
                );
                write_claude_native_marker(
                    context_file,
                    &format!("post-stop-ready:{}", next_prompt.id()),
                );
            }
        }
        Ok(())
    }

    fn drain_known_claude_transcripts(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        context_file: &str,
    ) -> Result<(), DaemonError> {
        let paths = known_claude_transcript_paths(context_file);
        for path in paths {
            self.drain_claude_transcript(session_id, provider_run_id, context_file, &path)?;
        }
        Ok(())
    }

    fn drain_claude_transcript(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        context_file: &str,
        transcript_path: &str,
    ) -> Result<(), DaemonError> {
        let minimum_timestamp_ms = self
            .app
            .providers
            .get_run(provider_run_id)
            .ok()
            .and_then(|run| run.agent_instance_id().map(str::to_string))
            .and_then(|agent_id| {
                self.app
                    .prompt_owner_active_prompt_for_agent(session_id, &agent_id)
                    .ok()
                    .flatten()
            })
            .map(|prompt| prompt.created_at_ms())
            .or_else(|| {
                self.app
                    .providers
                    .get_run(provider_run_id)
                    .ok()
                    .map(|run| run.started_at_ms())
            });
        let mut cursor = load_claude_transcript_cursor(context_file);
        let drain =
            drain_claude_transcript_file_since(transcript_path, &mut cursor, minimum_timestamp_ms);
        save_claude_transcript_cursor(context_file, &cursor);
        let active_prompt_id = self
            .app
            .providers
            .get_run(provider_run_id)
            .ok()
            .and_then(|run| run.agent_instance_id().map(str::to_string))
            .and_then(|agent_id| {
                self.app
                    .prompt_owner_active_prompt_for_agent(session_id, &agent_id)
                    .ok()
                    .flatten()
            })
            .map(|prompt| prompt.id().to_string());
        acknowledge_claude_headless_steering_enqueue(
            context_file,
            active_prompt_id.as_deref(),
            &drain.enqueued_prompts,
        );
        if drain.chunks.is_empty()
            && drain.assistant_message_ids.is_empty()
            && drain.session_id.is_none()
            && drain.model.is_none()
        {
            return Ok(());
        }

        let mut metadata = ProviderPromptSignalBatch::default();
        if let Some(session_id) = drain.session_id {
            metadata.resolved_resume_state =
                Some(ProviderResumeState::from_claude_session_id(session_id));
        }
        if let Some(model) = drain.model {
            metadata.resolved_model = Some(model);
            metadata.resolved_model_source = Some("claude.transcript");
        }
        if metadata.resolved_resume_state.is_some() || metadata.resolved_model.is_some() {
            self.app
                .providers
                .apply_structured_output_metadata(provider_run_id, &metadata)?;
            if let Ok(run) = self.app.providers.get_run(provider_run_id) {
                self.app.update_provider_run_projection(run);
            }
        }

        let recipient_attachment_ids = self.app.attachments.list_session_attachment_ids(session_id);
        let fanout = ProviderOutputFanout::new(self.app);
        let mut saw_response_content = false;
        let mut saw_runtime_activity = false;
        for chunk in drain.chunks {
            if chunk.text.is_empty() {
                continue;
            }
            if chunk.kind == TerminalOutputKind::ProviderTool {
                crate::transport::flow_control::note_prompt_tool_output(
                    self.app,
                    provider_run_id,
                    Some(&chunk.merge_key_suffix),
                    chunk.text.as_bytes(),
                );
            }
            if matches!(
                chunk.kind,
                TerminalOutputKind::ProviderOutput | TerminalOutputKind::ProviderReasoning
            ) {
                saw_response_content = true;
            }
            if matches!(
                chunk.kind,
                TerminalOutputKind::ProviderOutput
                    | TerminalOutputKind::ProviderReasoning
                    | TerminalOutputKind::ProviderTool
                    | TerminalOutputKind::ProviderStatus
            ) {
                saw_runtime_activity = true;
            }
            fanout.fan_out(
                session_id,
                provider_run_id,
                chunk.kind,
                Some(format!(
                    "claude-transcript:{provider_run_id}:{}",
                    chunk.merge_key_suffix
                )),
                recipient_attachment_ids.clone(),
                chunk.text.as_bytes(),
            );
        }
        if saw_response_content {
            crate::transport::flow_control::note_prompt_response_content(self.app, provider_run_id);
        } else if saw_runtime_activity {
            crate::transport::flow_control::note_prompt_output(self.app, provider_run_id);
        }
        for message_id in drain.assistant_message_ids {
            ProviderOutputFanout::new(self.app).record_assistant_message_completion(
                session_id,
                provider_run_id,
                recipient_attachment_ids.clone(),
                &message_id,
                unix_epoch_ms(),
            );
        }
        Ok(())
    }

    pub(crate) fn process_terminal_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        provider_run: &RuntimeProviderRun,
        native_interaction_bridge: Option<std::sync::Arc<dyn ProviderNativeInteractionBridge>>,
        rendered: &str,
    ) -> Result<(), DaemonError> {
        let Some(context_file) = provider_run.pty_env().get("CHARIOX_CLAUDE_NATIVE_CONTEXT") else {
            return Ok(());
        };
        let visible = claude_rendered_permission_visible(rendered);
        if !rendered.is_empty() {
            if provider_run.provider() == "claude-headless" {
                append_claude_headless_debug(context_file, "pty", rendered);
            }
            self.drain_known_claude_transcripts(session_id, provider_run_id, context_file)?;
        }
        if provider_run.provider() == "claude-headless" {
            let recent = update_claude_permission_recent(context_file, rendered);
            if claude_headless_bypass_selection_pending(context_file) {
                append_claude_headless_debug(
                    context_file,
                    "auto_confirm_enter",
                    "bypass_permissions",
                );
                self.app
                    .write_provider_pty_input_for_runtime(provider_run_id, b"\r")?;
                write_claude_headless_startup_wait_marker(context_file);
                clear_claude_permission_recent(context_file);
                return Ok(());
            }
            if claude_headless_workspace_trust_visible(&recent) {
                append_claude_headless_debug(context_file, "auto_confirm", "workspace_trust");
                self.app
                    .write_provider_pty_input_for_runtime(provider_run_id, b"\r")?;
                write_claude_headless_startup_wait_marker(context_file);
                clear_claude_permission_recent(context_file);
                return Ok(());
            }
            if claude_headless_bypass_confirmation_visible(&recent) {
                append_claude_headless_debug(context_file, "auto_confirm", "bypass_permissions");
                self.app
                    .write_provider_pty_input_for_runtime(provider_run_id, b"\x1b[B")?;
                write_claude_headless_bypass_selection_marker(context_file);
                clear_claude_permission_recent(context_file);
                return Ok(());
            }
        }
        if claude_native_marker(context_file)
            .as_deref()
            .is_some_and(|value| value.starts_with("permission:"))
        {
            clear_claude_permission_recent(context_file);
            return Ok(());
        }
        let recent = if visible {
            rendered.to_string()
        } else {
            update_claude_permission_recent(context_file, rendered)
        };
        if !visible && !claude_rendered_permission_visible(&recent) {
            clear_claude_yolo_rendered_permission_confirmation(context_file);
            return Ok(());
        }
        if take_matching_claude_hook_permission_tombstone(context_file, &recent) {
            clear_claude_permission_recent(context_file);
            return Ok(());
        }
        if provider_run.permission_level() == crate::provider::AgentPermissionLevel::Yolo {
            if claude_yolo_rendered_permission_confirmation_pending(context_file, &recent) {
                clear_claude_permission_recent(context_file);
                return Ok(());
            }
            append_claude_headless_debug(context_file, "auto_confirm", "yolo_permission");
            self.app
                .write_provider_pty_input_for_runtime(provider_run_id, b"\r")?;
            mark_claude_yolo_rendered_permission_confirmed(context_file, &recent);
            clear_claude_permission_recent(context_file);
            return Ok(());
        }
        let Some(bridge) = native_interaction_bridge else {
            return Ok(());
        };
        let Some(agent_id) = provider_run.agent_instance_id().map(str::to_string) else {
            return Ok(());
        };
        let interaction_id = format!(
            "claude-rendered-permission-{provider_run_id}-{}",
            timestamp_millis()
        );
        write_claude_native_marker(context_file, &format!("permission:{interaction_id}"));
        clear_claude_permission_recent(context_file);
        let interaction = RuntimeInteraction::new(
            interaction_id.clone(),
            agent_id,
            RuntimeInteractionKind::Permission,
            RuntimeInteractionLevel::Warning,
            Some("Approve Claude Code tool?".to_string()),
            "Claude Code is showing a native tool permission prompt.",
            vec![
                RuntimeInteractionChoice::new(
                    "allow_once",
                    "Allow once",
                    "allow",
                    Some(RuntimeInteractionChoiceStyle::Primary),
                ),
                RuntimeInteractionChoice::new(
                    "deny",
                    "Deny",
                    "deny",
                    Some(RuntimeInteractionChoiceStyle::Danger),
                ),
            ],
            None,
            Some(300),
            Some("deny".to_string()),
        );
        let session_id = session_id.to_string();
        let context_file = context_file.to_string();
        std::thread::spawn(move || {
            let input = match bridge.request_blocking(&session_id, interaction) {
                Ok(resolution)
                    if resolution.reply.as_deref() == Some("allow")
                        || resolution.choice_id.as_deref() == Some("allow_once") =>
                {
                    b"\r".to_vec()
                }
                Ok(_) => vec![0x03],
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "daemon.provider_output",
                        "Claude rendered permission bridge failed",
                        serde_json::json!({
                            "session_id": session_id,
                            "interaction_id": interaction_id,
                            "error": error.to_string(),
                        }),
                    );
                    vec![0x03]
                }
            };
            write_claude_permission_input(&context_file, &interaction_id, &input);
        });
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_permission_event(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        context_file: &str,
        provider_run: &RuntimeProviderRun,
        native_interaction_bridge: Option<std::sync::Arc<dyn ProviderNativeInteractionBridge>>,
        event: &Value,
    ) -> Result<(), DaemonError> {
        if !should_bridge_claude_permission(event) {
            return Ok(());
        }
        let Some(request_id) = event
            .get("hook_context_request_id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
        else {
            return Ok(());
        };
        if provider_run.permission_level() == crate::provider::AgentPermissionLevel::Yolo {
            write_claude_hook_permission_tombstone(context_file, event);
            clear_claude_permission_recent(context_file);
            write_claude_permission_response(
                context_file,
                request_id,
                true,
                "Allowed by Chariox yolo permission policy.",
            );
            return Ok(());
        }
        let Some(bridge) = native_interaction_bridge else {
            return Ok(());
        };
        let tool_name = event
            .get("tool_name")
            .and_then(Value::as_str)
            .unwrap_or("tool");
        let interaction_id = format!("claude-native-permission-{provider_run_id}-{request_id}");
        write_claude_native_marker(context_file, &format!("permission:{interaction_id}"));
        write_claude_hook_permission_tombstone(context_file, event);
        clear_claude_permission_recent(context_file);
        let interaction = RuntimeInteraction::new(
            interaction_id,
            agent_id.to_string(),
            RuntimeInteractionKind::Permission,
            RuntimeInteractionLevel::Warning,
            Some(format!("Approve Claude Code {tool_name}?")),
            format_claude_permission_message(event),
            vec![
                RuntimeInteractionChoice::new(
                    "allow_once",
                    "Allow once",
                    "allow",
                    Some(RuntimeInteractionChoiceStyle::Primary),
                ),
                RuntimeInteractionChoice::new(
                    "deny",
                    "Deny",
                    "deny",
                    Some(RuntimeInteractionChoiceStyle::Danger),
                ),
            ],
            None,
            Some(300),
            Some("deny".to_string()),
        );
        let session_id = session_id.to_string();
        let context_file = context_file.to_string();
        let request_id = request_id.to_string();
        std::thread::spawn(
            move || match bridge.request_blocking(&session_id, interaction) {
                Ok(resolution) => {
                    let allowed = resolution.reply.as_deref() == Some("allow")
                        || resolution.choice_id.as_deref() == Some("allow_once");
                    write_claude_permission_response(
                        &context_file,
                        &request_id,
                        allowed,
                        if allowed {
                            "Approved through Chariox."
                        } else if resolution.status == "timed_out" {
                            "Timed out waiting for Chariox approval."
                        } else {
                            "Denied through Chariox."
                        },
                    );
                }
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "daemon.provider_output",
                        "Claude native permission bridge failed",
                        serde_json::json!({
                            "session_id": session_id,
                            "request_id": request_id,
                            "error": error.to_string(),
                        }),
                    );
                    write_claude_permission_response(
                        &context_file,
                        &request_id,
                        false,
                        "Chariox permission bridge failed.",
                    );
                }
            },
        );
        Ok(())
    }

    /// One injection attempt for a prompt dispatch. Claude-headless confirms
    /// injection asynchronously through the context-file marker, so the caller
    /// retries `AwaitingInjection` outcomes off the app lock instead of this
    /// method sleeping while the whole daemon is blocked.
    pub(crate) fn process_prompt_dispatch_attempt(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        provider_run: &RuntimeProviderRun,
        dispatch: &KernelPromptDispatch,
    ) -> Result<ClaudeNativeDispatchAttempt, DaemonError> {
        let Some(agent_id) = provider_run.agent_instance_id().map(str::to_string) else {
            return Ok(ClaudeNativeDispatchAttempt::Completed);
        };
        let Some(context_file) = provider_run.pty_env().get("CHARIOX_CLAUDE_NATIVE_CONTEXT") else {
            return Ok(ClaudeNativeDispatchAttempt::Completed);
        };
        let prompt = ClaudeNativePromptInjection {
            id: &dispatch.prompt_id,
            prompt: &dispatch.prompt,
            hidden_system_context: &dispatch.hidden_system_context,
            attachments: &dispatch.attachments,
        };
        self.inject_prompt(
            session_id,
            provider_run_id,
            &agent_id,
            context_file,
            provider_run,
            &prompt,
        )?;
        if provider_run.provider() == "claude-headless" {
            if let Some(events_file) = provider_run.pty_env().get("CHARIOX_CLAUDE_NATIVE_EVENTS") {
                // Workflow dispatch does not have a terminal client polling
                // provider output. Observe the exact UserPromptSubmit hook
                // here as well so successful headless injection does not rely
                // on an unrelated output-pump request. Leave the event file
                // intact for the normal bridge to drain transcript and Stop.
                acknowledge_claude_headless_dispatch_from_hook_events(
                    context_file,
                    events_file,
                    prompt.id,
                );
            }
        }
        // Native TUI injection completes once Enter reaches the provider. A
        // headless run must additionally acknowledge UserPromptSubmit; an
        // `injected` marker only proves that bytes were written to the PTY and
        // is not enough to leave the turn running indefinitely if Claude
        // dropped them during cold startup.
        let marker = claude_native_marker(context_file);
        let completed_marker = if provider_run.provider() == "claude-headless" {
            format!("accepted:{}", prompt.id)
        } else {
            format!("injected:{}", prompt.id)
        };
        if marker.as_deref() == Some(completed_marker.as_str()) {
            return Ok(ClaudeNativeDispatchAttempt::Completed);
        }
        Ok(ClaudeNativeDispatchAttempt::AwaitingInjection)
    }

    fn claude_native_prompt_context(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt: &str,
    ) -> Result<String, DaemonError> {
        let agent = self.app.agents.get_agent(agent_id)?;
        let session = self.app.sessions.get_session(session_id)?;
        let skill_grants = agent.skill_grants();
        crate::skill::format_granted_skill_prompt_context(
            agent.agent_ref(),
            &skill_grants,
            session.workspace_id(),
            prompt,
        )
    }

    fn inject_pending_prompt(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        context_file: &str,
        provider_run: &RuntimeProviderRun,
    ) -> Result<(), DaemonError> {
        let Some(prompt) = self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, agent_id)?
        else {
            return Ok(());
        };
        if claude_native_marker(context_file)
            .as_deref()
            .and_then(claude_native_dispatch_prompt_id)
            .is_some_and(|dispatch_prompt_id| dispatch_prompt_id != prompt.id())
        {
            // The output pump also revisits pending injection. Do not let it
            // overwrite a concurrent steering dispatch, whose injection id is
            // intentionally different from the active turn's prompt id.
            return Ok(());
        }
        let prompt = ClaudeNativePromptInjection {
            id: prompt.id(),
            prompt: prompt.prompt(),
            hidden_system_context: prompt.hidden_system_context(),
            attachments: prompt.attachments(),
        };
        self.inject_prompt(
            session_id,
            provider_run_id,
            agent_id,
            context_file,
            provider_run,
            &prompt,
        )
    }

    fn inject_prompt(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        context_file: &str,
        provider_run: &RuntimeProviderRun,
        prompt: &ClaudeNativePromptInjection<'_>,
    ) -> Result<(), DaemonError> {
        let mut marker = claude_native_marker(context_file);
        if marker
            .as_deref()
            .is_some_and(|value| value.starts_with("permission:"))
        {
            return Ok(());
        }
        let force_post_stop_ready = provider_run.provider() == "claude-headless"
            && marker
                .as_deref()
                .is_some_and(|value| value == format!("post-stop-ready:{}", prompt.id));
        if force_post_stop_ready {
            crate::logging::debug_with_fields(
                "daemon.claude_headless",
                "forcing post-stop queued prompt injection",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                    "agent_id": agent_id,
                    "prompt_id": prompt.id,
                }),
            );
            write_claude_native_marker(context_file, "");
            marker = None;
        }
        // A prior injection wrote the visible text and marked `submit-wait`;
        // submit the Enter keystroke once the PTY-settle delay has elapsed.
        // The wait itself happens off the app lock: the async dispatch retry
        // loop (and the output pump for `process`) revisit this until the
        // delay passes, so the daemon is never blocked mid-injection.
        match submit_wait_state(marker.as_deref(), prompt.id, unix_epoch_ms()) {
            SubmitWaitState::Waiting => {
                append_claude_headless_debug(context_file, "submit_wait", prompt.id);
                return Ok(());
            }
            SubmitWaitState::ReadyToSubmit => {
                append_claude_headless_debug(context_file, "submit_enter", prompt.id);
                self.app
                    .write_provider_pty_input_for_runtime(provider_run_id, b"\r")?;
                write_claude_native_marker(context_file, &format!("injected:{}", prompt.id));
                if provider_run.provider() == "claude-headless" {
                    let retry = read_claude_headless_submit_retry(context_file);
                    let count = if retry.prompt_id == prompt.id {
                        retry.count
                    } else {
                        0
                    };
                    let visible_prompt =
                        if retry.prompt_id == prompt.id && !retry.visible_prompt.is_empty() {
                            retry.visible_prompt
                        } else {
                            claude_headless_prompt_input(prompt, context_file)
                        };
                    write_claude_headless_submit_retry(
                        context_file,
                        prompt.id,
                        count,
                        unix_epoch_ms(),
                        &visible_prompt,
                    );
                }
                return Ok(());
            }
            SubmitWaitState::NotSubmitWait => {}
        }
        let prompt_typed_for_headless = provider_run.provider() == "claude-headless"
            && marker.as_deref() == Some(&format!("typed:{}", prompt.id));
        if provider_run.provider() == "claude-headless"
            && claude_headless_bypass_selection_pending(context_file)
        {
            append_claude_headless_debug(context_file, "bypass_selection_wait", prompt.id);
            return Ok(());
        }
        if let Some(started_at_ms) = marker
            .as_deref()
            .and_then(|value| value.strip_prefix("startup-wait:"))
            .and_then(|value| value.parse::<u64>().ok())
        {
            if unix_epoch_ms().saturating_sub(started_at_ms) < 2_500 {
                append_claude_headless_debug(context_file, "startup_wait", prompt.id);
                return Ok(());
            }
            write_claude_native_marker(context_file, "");
            marker = None;
        }
        if provider_run.provider() == "claude-headless"
            && marker.is_none()
            && unix_epoch_ms().saturating_sub(provider_run.started_at_ms()) < 4_000
        {
            append_claude_headless_debug(context_file, "inject_wait", prompt.id);
            return Ok(());
        }
        if provider_run.provider() == "claude-headless" {
            let recent = claude_permission_recent_file(context_file)
                .and_then(|path| fs::read_to_string(path).ok())
                .unwrap_or_default();
            if claude_headless_workspace_trust_visible(&recent) {
                append_claude_headless_debug(
                    context_file,
                    "inject_auto_confirm",
                    "workspace_trust",
                );
                self.app
                    .write_provider_pty_input_for_runtime(provider_run_id, b"\r")?;
                write_claude_headless_startup_wait_marker(context_file);
                clear_claude_permission_recent(context_file);
                return Ok(());
            }
            if claude_headless_bypass_confirmation_visible(&recent) {
                append_claude_headless_debug(
                    context_file,
                    "inject_auto_confirm",
                    "bypass_permissions",
                );
                self.app
                    .write_provider_pty_input_for_runtime(provider_run_id, b"\x1b[B")?;
                write_claude_headless_bypass_selection_marker(context_file);
                clear_claude_permission_recent(context_file);
                return Ok(());
            }
            if !force_post_stop_ready
                && !prompt_typed_for_headless
                && !claude_headless_composer_visible(&recent)
                && unix_epoch_ms().saturating_sub(provider_run.started_at_ms()) < 4_000
            {
                append_claude_headless_debug(context_file, "inject_wait_composer", prompt.id);
                return Ok(());
            }
        }
        if marker.as_deref() == Some(&format!("typed:{}", prompt.id)) {
            append_claude_headless_debug(context_file, "inject_enter", prompt.id);
            self.app
                .write_provider_pty_input_for_runtime(provider_run_id, b"\r")?;
            write_claude_native_marker(context_file, &format!("injected:{}", prompt.id));
            if provider_run.provider() == "claude-headless" {
                let visible_prompt = claude_headless_prompt_input(prompt, context_file);
                write_claude_headless_submit_retry(
                    context_file,
                    prompt.id,
                    0,
                    unix_epoch_ms(),
                    &visible_prompt,
                );
            }
            return Ok(());
        }
        if provider_run.provider() == "claude-headless"
            && marker.as_deref() == Some(&format!("injected:{}", prompt.id))
        {
            let retry = read_claude_headless_submit_retry(context_file);
            let now = unix_epoch_ms();
            let recent = claude_permission_recent_file(context_file)
                .and_then(|path| fs::read_to_string(path).ok())
                .unwrap_or_default();
            let count = if retry.prompt_id == prompt.id {
                retry.count
            } else {
                0
            };
            let last_attempt_ms = if retry.prompt_id == prompt.id {
                retry.last_attempt_ms
            } else {
                0
            };
            let visible_prompt = if retry.prompt_id == prompt.id && !retry.visible_prompt.is_empty()
            {
                retry.visible_prompt.clone()
            } else {
                claude_headless_prompt_input(prompt, context_file)
            };
            if count < CLAUDE_HEADLESS_SUBMIT_RETRY_LIMIT
                && now.saturating_sub(last_attempt_ms) >= 2_000
                && claude_headless_prompt_waiting_in_composer(
                    &recent,
                    redact_native_hidden_instructions(prompt.prompt).trim(),
                )
            {
                append_claude_headless_debug(
                    context_file,
                    "inject_enter_retry",
                    &format!("{}:{}", prompt.id, count + 1),
                );
                self.app
                    .write_provider_pty_input_for_runtime(provider_run_id, b"\r")?;
                write_claude_headless_submit_retry(
                    context_file,
                    prompt.id,
                    count + 1,
                    now,
                    &visible_prompt,
                );
            } else if count < CLAUDE_HEADLESS_SUBMIT_RETRY_LIMIT
                && now.saturating_sub(last_attempt_ms) >= 2_000
                && claude_headless_composer_visible(&recent)
            {
                // Claude can drop both the pasted text and Enter while its
                // cold-start composer is still taking ownership of the PTY.
                // UserPromptSubmit changes the marker to `accepted` before
                // this grace period expires on successful submissions, so an
                // idle composer with no acknowledgement is safe to retype.
                let input = claude_headless_prompt_input(prompt, context_file);
                append_claude_headless_debug(
                    context_file,
                    "inject_prompt_retry",
                    &format!("{}:{}", prompt.id, count + 1),
                );
                self.app
                    .write_provider_pty_input_for_runtime(provider_run_id, input.as_bytes())?;
                write_claude_headless_submit_retry(context_file, prompt.id, count + 1, now, &input);
                write_claude_native_marker(
                    context_file,
                    &format!("submit-wait:{}:{}", prompt.id, now),
                );
            }
            return Ok(());
        }
        if marker
            .as_deref()
            .is_some_and(|value| value.ends_with(prompt.id))
        {
            return Ok(());
        }
        let native_attachment_suffix =
            format_claude_native_attachment_prompt_suffix(prompt.attachments, context_file);
        let visible = redact_native_hidden_instructions(prompt.prompt)
            .trim()
            .to_string();
        let native_hidden = extract_native_hidden_instructions(prompt.prompt);
        let attachment_context = format_claude_attachment_context(prompt.attachments, context_file);
        let hidden_context = if provider_run.provider() == "claude-headless" {
            let envelope = crate::prompt_assembly::PromptAssemblyService::from_env()?
                .assemble_provider_turn(
                    provider_run,
                    &visible,
                    Some(prompt.hidden_system_context),
                    prompt.attachments.to_vec(),
                    crate::prompt_assembly::PromptAssemblyMode::NormalProviderTurn,
                )?;
            let skill_context =
                self.claude_native_prompt_context(session_id, agent_id, &visible)?;
            join_claude_context([
                envelope.hidden_system_context,
                skill_context,
                native_hidden,
                attachment_context,
            ])
        } else {
            let scheduled_hidden =
                crate::prompt_assembly::strip_prompt_manifest_entries(prompt.hidden_system_context);
            join_claude_context([scheduled_hidden, native_hidden, attachment_context])
        };
        crate::provider::ensure_claude_native_hidden_context_fits(
            provider_run_id,
            &hidden_context,
        )?;
        fs::write(context_file, hidden_context).map_err(|error| DaemonError::ProviderProtocol {
            provider_run_id: provider_run_id.to_string(),
            operation: "claude_hidden_context_write",
            message: error.to_string(),
        })?;
        let visible = join_claude_context([native_attachment_suffix, visible]);
        if !visible.is_empty() {
            let input = if provider_run.provider() == "claude-headless" {
                normalize_claude_visible_prompt_for_headless(&visible)
            } else {
                visible.clone()
            };
            append_claude_headless_debug(context_file, "inject_prompt", &input);
            self.app
                .write_provider_pty_input_for_runtime(provider_run_id, input.as_bytes())?;
            let written_at_ms = unix_epoch_ms();
            if provider_run.provider() == "claude-headless" {
                write_claude_headless_submit_retry(
                    context_file,
                    prompt.id,
                    0,
                    written_at_ms,
                    &input,
                );
            }
            // Defer the Enter keystroke: mark `submit-wait` with the write
            // time so a later pass (off the app lock) submits it once the PTY
            // has had CLAUDE_SUBMIT_DELAY_MS to register the pasted text.
            write_claude_native_marker(
                context_file,
                &format!("submit-wait:{}:{written_at_ms}", prompt.id),
            );
        } else {
            append_claude_headless_debug(context_file, "inject_empty", prompt.id);
            write_claude_native_marker(context_file, &format!("injected:{}", prompt.id));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubmitWaitState {
    NotSubmitWait,
    Waiting,
    ReadyToSubmit,
}

fn claude_native_dispatch_prompt_id(marker: &str) -> Option<&str> {
    for prefix in ["typed:", "injected:", "accepted:"] {
        if let Some(prompt_id) = marker.strip_prefix(prefix) {
            return (!prompt_id.is_empty()).then_some(prompt_id);
        }
    }
    marker
        .strip_prefix("submit-wait:")
        .and_then(|rest| rest.rsplit_once(':'))
        .map(|(prompt_id, _)| prompt_id)
        .filter(|prompt_id| !prompt_id.is_empty())
}

/// Decide whether a deferred Enter keystroke is due for the given prompt,
/// based on a `submit-wait:{prompt_id}:{written_at_ms}` marker. An
/// unparseable timestamp submits immediately rather than stalling forever.
fn submit_wait_state(marker: Option<&str>, prompt_id: &str, now_ms: u64) -> SubmitWaitState {
    let Some(rest) = marker.and_then(|value| value.strip_prefix("submit-wait:")) else {
        return SubmitWaitState::NotSubmitWait;
    };
    let Some((marked_prompt_id, started_at)) = rest.rsplit_once(':') else {
        return SubmitWaitState::NotSubmitWait;
    };
    if marked_prompt_id != prompt_id {
        return SubmitWaitState::NotSubmitWait;
    }
    match started_at.parse::<u64>() {
        Ok(started_at_ms) if now_ms.saturating_sub(started_at_ms) < CLAUDE_SUBMIT_DELAY_MS => {
            SubmitWaitState::Waiting
        }
        _ => SubmitWaitState::ReadyToSubmit,
    }
}

fn claude_transcript_stop_drain_marker(context_file: &str) -> Option<(String, u64)> {
    let marker = claude_native_marker(context_file)?;
    let payload = marker.strip_prefix(CLAUDE_TRANSCRIPT_STOP_DRAIN_MARKER_PREFIX)?;
    let (prompt_id, stopped_at_ms) = payload.rsplit_once(':')?;
    let stopped_at_ms = stopped_at_ms.parse::<u64>().ok()?;
    (!prompt_id.trim().is_empty()).then(|| (prompt_id.to_string(), stopped_at_ms))
}
