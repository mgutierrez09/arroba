use crate::app::{DaemonApp, PromptActivityStore};
use crate::error::DaemonError;
use crate::provider::{
    classify_provider_terminal_failure_output_text, ProviderPromptSignalBatch, RuntimeProviderRun,
};
use crate::provider::{AgentEndpointMode, ProviderProcessServiceStore, ProviderRunState};
use crate::pty::PtyOutputChunk;
use crate::runtime::projection::AgentRuntimeProjectionStore;
use crate::terminal::{TerminalOutputKind, TerminalOutputRecord};

use super::provider_output_claude_native::{
    claude_native_recent_terminal_failure, ProviderOutputClaudeNativeBridge,
};
use super::provider_output_fanout::ProviderOutputFanout;
use super::provider_output_prompt_settlement::ProviderOutputPromptSettlement;
use super::provider_output_trace::ProviderOutputTrace;

mod background;
mod structured_store;
#[cfg(test)]
mod tests;
mod timeouts;

use background::pump_session_active_prompt_outputs;
pub(crate) use structured_store::{
    structured_output_batch_should_poll_immediately, StructuredOutputRecordStore,
    STRUCTURED_OUTPUT_EMPTY_POLL_BACKOFF_MS, STRUCTURED_OUTPUT_POLL_FAILURE_RETRY_LIMIT,
};
use timeouts::{reap_provider_first_output_timeouts, reap_provider_inactivity_timeouts};

pub(crate) struct ProviderOutputPumpRequest<'a> {
    pub(crate) session_id: &'a str,
    pub(crate) provider_run_id: &'a str,
    pub(crate) recipient_attachment_ids: Vec<String>,
    pub(crate) initial_liveness_already_checked: bool,
}

pub(crate) fn should_project_pty_output(
    has_active_prompt: bool,
    terminal_failure: Option<&str>,
    uses_transient_native_terminal: bool,
) -> bool {
    has_active_prompt || terminal_failure.is_some() || uses_transient_native_terminal
}

pub(crate) fn pump_terminal_output_for_attachment(
    app: &mut DaemonApp,
    session_id: &str,
    attachment_id: &str,
) -> Result<Vec<TerminalOutputRecord>, DaemonError> {
    reap_structured_prompt_jobs(app);
    reap_provider_first_output_timeouts(app, session_id)?;
    reap_provider_inactivity_timeouts(app, session_id)?;
    crate::app::KernelSessionReadService::new(app)
        .ensure_attachment_in_session(session_id, attachment_id)?;
    pump_session_active_prompt_outputs(app, session_id);
    Ok(app.terminal.drain_output_records(session_id, attachment_id))
}

pub(crate) fn reap_structured_prompt_jobs(app: &mut DaemonApp) {
    ProviderOutputStructuredPromptReaper::new(app).reap();
}

pub(crate) fn pump_active_prompt_outputs(app: &mut DaemonApp) -> Vec<String> {
    reap_structured_prompt_jobs(app);
    let sessions = app.sessions.list_sessions();
    let mut pumped_provider_run_ids = Vec::new();
    for session in sessions {
        if let Err(error) = reap_provider_first_output_timeouts(app, session.id()) {
            crate::logging::warn_with_fields(
                "daemon.provider_output",
                "provider first-output timeout reap failed",
                serde_json::json!({
                    "session_id": session.id(),
                    "error": error.to_string(),
                }),
            );
        }
        if let Err(error) = reap_provider_inactivity_timeouts(app, session.id()) {
            crate::logging::warn_with_fields(
                "daemon.provider_output",
                "provider inactivity timeout reap failed",
                serde_json::json!({
                    "session_id": session.id(),
                    "error": error.to_string(),
                }),
            );
        }
        pumped_provider_run_ids.extend(pump_session_active_prompt_outputs(app, session.id()));
    }
    pumped_provider_run_ids
}

pub(crate) struct ProviderOutputPump<'a> {
    context: ProviderOutputPumpContext<'a>,
}

impl<'a> ProviderOutputPump<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self {
            context: ProviderOutputPumpContext::new(app),
        }
    }

    pub(crate) fn pump_provider_output(
        &mut self,
        request: ProviderOutputPumpRequest<'_>,
    ) -> Result<Vec<TerminalOutputRecord>, DaemonError> {
        self.context.reap_structured_prompt_jobs();
        self.context
            .reap_provider_first_output_timeouts(request.session_id)?;
        self.context
            .reap_provider_inactivity_timeouts(request.session_id)?;
        let mut provider_run = self
            .context
            .ensure_provider_run_in_session(request.session_id, request.provider_run_id)?;
        let uses_structured_prompt_io = self.context.run_uses_structured_prompt_io(&provider_run);
        if !request.initial_liveness_already_checked
            && uses_structured_prompt_io
            && self
                .context
                .reconcile_provider_run_exit(request.session_id, request.provider_run_id)?
        {
            return Ok(self
                .context
                .pending_structured_output_records
                .take_and_stop_polling(request.provider_run_id));
        }
        if provider_run.state() == ProviderRunState::Ended {
            return Ok(self
                .context
                .pending_structured_output_records
                .take_and_stop_polling(request.provider_run_id));
        }
        if provider_run.state() == ProviderRunState::Parked {
            if !self
                .context
                .provider_run_has_active_prompt(request.session_id, &provider_run)?
            {
                return Ok(self
                    .context
                    .pending_structured_output_records
                    .take_and_stop_polling(request.provider_run_id));
            }
            provider_run = self
                .context
                .resume_detached_provider_run(request.provider_run_id)?;
            crate::logging::warn_with_fields(
                "daemon.provider_output",
                "resumed parked provider run that still had an active prompt",
                serde_json::json!({
                    "session_id": request.session_id,
                    "provider_run_id": request.provider_run_id,
                    "agent_id": provider_run.agent_instance_id(),
                }),
            );
        }

        if uses_structured_prompt_io {
            return self.context.pump_structured_output(
                request.session_id,
                request.provider_run_id,
                request.recipient_attachment_ids,
            );
        }
        if crate::provider::provider_run_uses_claude_native_bridge(&provider_run) {
            if let Some(message) = self.context.process_claude_native_tui_bridge(
                request.session_id,
                request.provider_run_id,
                &provider_run,
            )? {
                let run = self
                    .context
                    .provider_store
                    .record_terminal_diagnostic(request.provider_run_id, message.clone())?;
                self.context.app.update_provider_run_projection(run);
                self.context.fail_prompt_for_terminal_failure(
                    request.session_id,
                    request.provider_run_id,
                    &message,
                )?;
                return Ok(Vec::new());
            }
        }

        let mut chunks = match self.context.drain_pty_output(request.provider_run_id) {
            Ok(chunks) => chunks,
            Err(error) => {
                if self
                    .context
                    .reconcile_provider_run_exit(request.session_id, request.provider_run_id)?
                {
                    return Ok(self
                        .context
                        .pending_structured_output_records
                        .take_and_stop_polling(request.provider_run_id));
                }
                return Err(error);
            }
        };
        if crate::provider::provider_run_uses_claude_native_bridge(&provider_run) {
            let rendered = chunks
                .iter()
                .map(|chunk| String::from_utf8_lossy(&chunk.bytes))
                .collect::<String>();
            self.context.process_claude_native_terminal_output_bridge(
                request.session_id,
                request.provider_run_id,
                &provider_run,
                &rendered,
            )?;
        }
        let terminal_failure = classify_provider_terminal_failure_output_text(
            provider_run.adapter_key(),
            &chunks
                .iter()
                .map(|chunk| String::from_utf8_lossy(&chunk.bytes))
                .collect::<String>(),
        )
        .or_else(|| {
            if crate::provider::provider_run_uses_claude_native_bridge(&provider_run) {
                claude_native_recent_terminal_failure(&provider_run)
            } else {
                None
            }
        });
        let has_active_prompt = self
            .context
            .provider_run_has_active_prompt(request.session_id, &provider_run)?;
        let uses_transient_native_terminal =
            crate::provider::provider_run_uses_claude_native_bridge(&provider_run)
                && !crate::provider::provider_run_is_claude_headless(&provider_run);
        if !should_project_pty_output(
            has_active_prompt,
            terminal_failure.as_deref(),
            uses_transient_native_terminal,
        ) {
            chunks.clear();
        }
        if !chunks.is_empty() {
            if crate::provider::provider_run_is_claude_headless(&provider_run)
                || uses_transient_native_terminal
            {
                self.context.note_prompt_output(request.provider_run_id);
            } else {
                self.context
                    .note_prompt_response_content(request.provider_run_id);
            }
        }

        let records = if crate::provider::provider_run_is_claude_headless(&provider_run) {
            Vec::new()
        } else {
            chunks
                .into_iter()
                .map(|chunk| {
                    if uses_transient_native_terminal {
                        self.context.fan_out_terminal_output(
                            request.session_id,
                            request.provider_run_id,
                            TerminalOutputKind::ProviderTerminal,
                            None,
                            request.recipient_attachment_ids.clone(),
                            &chunk.bytes,
                        )
                    } else {
                        self.context.fan_out_provider_output(
                            request.session_id,
                            request.provider_run_id,
                            request.recipient_attachment_ids.clone(),
                            &chunk.bytes,
                        )
                    }
                })
                .collect::<Vec<_>>()
        };
        if let Some(message) = terminal_failure {
            let run = self
                .context
                .provider_store
                .record_terminal_diagnostic(request.provider_run_id, message.clone())?;
            self.context.app.update_provider_run_projection(run);
            self.context.fail_prompt_for_terminal_failure(
                request.session_id,
                request.provider_run_id,
                &message,
            )?;
            return Ok(records);
        }
        self.context
            .reconcile_provider_run_exit(request.session_id, request.provider_run_id)?;
        if records.is_empty() {
            self.context
                .settle_pty_prompt_if_quiet(request.session_id, request.provider_run_id)?;
        }

        Ok(records)
    }
}

struct ProviderOutputPumpContext<'a> {
    app: &'a mut DaemonApp,
    provider_store: ProviderProcessServiceStore,
    pending_structured_output_records: StructuredOutputRecordStore,
    active_turns: crate::app::ActiveTurnStore,
    prompt_activity: PromptActivityStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
}

struct ProviderOutputRecipientResolver<'a> {
    app: &'a DaemonApp,
}

impl<'a> ProviderOutputRecipientResolver<'a> {
    fn new(app: &'a DaemonApp) -> Self {
        Self { app }
    }

    fn session_attachment_ids(&self, session_id: &str) -> Vec<String> {
        self.app.attachments.list_session_attachment_ids(session_id)
    }
}

struct ProviderOutputLiveness<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> ProviderOutputLiveness<'a> {
    fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    fn reconcile_exit(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        super::provider_runtime::ProviderRunLivenessRuntime::new(self.app)
            .reconcile_provider_run_exit(session_id, provider_run_id)
    }
}

struct ProviderOutputPtyDrain<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> ProviderOutputPtyDrain<'a> {
    fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    fn drain_output(&mut self, provider_run_id: &str) -> Result<Vec<PtyOutputChunk>, DaemonError> {
        self.app.pty.drain_output(provider_run_id)
    }
}

struct ProviderOutputStructuredPromptReaper<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> ProviderOutputStructuredPromptReaper<'a> {
    fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    fn reap(&mut self) {
        self.app.reap_structured_prompt_jobs();
    }
}

impl<'a> ProviderOutputPumpContext<'a> {
    fn new(app: &'a mut DaemonApp) -> Self {
        Self {
            provider_store: app.providers.clone(),
            pending_structured_output_records: app.pending_structured_output_records.clone(),
            active_turns: app.active_turns.clone(),
            prompt_activity: app.prompt_activity.clone(),
            agent_runtime_projection: app.agent_runtime_projection_store(),
            app,
        }
    }

    fn reap_structured_prompt_jobs(&mut self) {
        reap_structured_prompt_jobs(self.app);
    }

    fn reap_provider_first_output_timeouts(&mut self, session_id: &str) -> Result<(), DaemonError> {
        reap_provider_first_output_timeouts(self.app, session_id)
    }

    fn reap_provider_inactivity_timeouts(&mut self, session_id: &str) -> Result<(), DaemonError> {
        reap_provider_inactivity_timeouts(self.app, session_id)
    }

    fn reconcile_provider_run_exit(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        ProviderOutputLiveness::new(self.app).reconcile_exit(session_id, provider_run_id)
    }

    fn ensure_provider_run_in_session(
        &self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let provider_run = self.provider_store.get_run(provider_run_id)?;
        if provider_run.session_id() != session_id {
            return Err(DaemonError::ProviderRunNotInSession {
                session_id: session_id.to_string(),
                provider_run_id: provider_run_id.to_string(),
            });
        }
        Ok(provider_run)
    }

    fn run_uses_structured_prompt_io(&self, provider_run: &RuntimeProviderRun) -> bool {
        self.provider_store
            .run_uses_structured_prompt_io(provider_run)
    }

    fn provider_run_has_active_prompt(
        &self,
        session_id: &str,
        provider_run: &RuntimeProviderRun,
    ) -> Result<bool, DaemonError> {
        self.app
            .provider_run_has_active_prompt(session_id, provider_run)
    }

    fn resume_detached_provider_run(
        &mut self,
        provider_run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let run = self.provider_store.resume_run_detached(provider_run_id)?;
        self.app.update_provider_run_projection(run.clone());
        Ok(run)
    }

    fn pump_structured_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
    ) -> Result<Vec<TerminalOutputRecord>, DaemonError> {
        let mut provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;
        if provider_run.state() == ProviderRunState::Parked {
            if !self.provider_run_has_active_prompt(session_id, &provider_run)? {
                return Ok(self
                    .pending_structured_output_records
                    .take_and_stop_polling(provider_run_id));
            }
            provider_run = self.resume_detached_provider_run(provider_run_id)?;
        }
        if provider_run.endpoint_mode() != AgentEndpointMode::External {
            if let Err(error) = self.drain_pty_output(provider_run_id) {
                if self.reconcile_provider_run_exit(session_id, provider_run_id)? {
                    return Ok(self
                        .pending_structured_output_records
                        .take_and_stop_polling(provider_run_id));
                }
                if !matches!(error, DaemonError::PtyProcessNotFound { .. }) {
                    return Err(error);
                }
            }
        }
        let mut records = self.pending_structured_output_records.take(provider_run_id);
        records.extend(self.drain_finished_structured_output_jobs_for_run(
            session_id,
            provider_run_id,
            recipient_attachment_ids.clone(),
        )?);
        if crate::transport::flow_control::prompt_completion_settlement_pending(
            self.app,
            provider_run_id,
        ) {
            self.settle_structured_prompt_completion(session_id, provider_run_id, false, false)?;
            if !self.provider_run_has_active_prompt(session_id, &provider_run)? {
                self.pending_structured_output_records
                    .stop_polling(provider_run_id);
                return Ok(records);
            }
        }
        if self
            .pending_structured_output_records
            .poll_due(provider_run_id, crate::session::unix_epoch_ms())
        {
            let active_prompt = provider_run
                .agent_instance_id()
                .map(str::to_string)
                .and_then(|agent_id| {
                    self.app
                        .prompt_owner_active_prompt_for_agent(session_id, &agent_id)
                        .ok()
                        .flatten()
                });
            if active_prompt
                .as_ref()
                .is_some_and(|prompt| prompt.delivery_pending())
            {
                self.pending_structured_output_records
                    .schedule_after_empty_poll(
                        provider_run_id.to_string(),
                        crate::session::unix_epoch_ms(),
                    );
                return Ok(records);
            }
            match self
                .provider_store
                .enqueue_structured_output_poll(provider_run_id)?
            {
                true => {
                    let prompt_id = active_prompt.map(|prompt| prompt.id().to_string());
                    self.pending_structured_output_records
                        .mark_poll_enqueued(provider_run_id, prompt_id);
                }
                false => self
                    .pending_structured_output_records
                    .schedule_after_empty_poll(
                        provider_run_id.to_string(),
                        crate::session::unix_epoch_ms(),
                    ),
            }
        }
        Ok(records)
    }

    fn drain_finished_structured_output_jobs_for_run(
        &mut self,
        requested_session_id: &str,
        requested_provider_run_id: &str,
        requested_recipient_attachment_ids: Vec<String>,
    ) -> Result<Vec<TerminalOutputRecord>, DaemonError> {
        let mut requested_records = Vec::new();
        for finished in self
            .provider_store
            .drain_finished_structured_output_poll_jobs()
        {
            let settlement_retry_attempt = finished.settlement_retry_attempt;
            let provider_run_id = finished.provider_run_id.clone();
            let polled_prompt_id = self
                .pending_structured_output_records
                .take_in_flight_prompt_id(&provider_run_id);
            let is_requested_run = provider_run_id == requested_provider_run_id;
            let now_ms = crate::session::unix_epoch_ms();
            let poll_result = match finished.result {
                Ok(Some(poll_result)) => {
                    self.pending_structured_output_records
                        .mark_poll_succeeded(&provider_run_id);
                    poll_result
                }
                Ok(None) => {
                    self.pending_structured_output_records
                        .mark_poll_succeeded(&provider_run_id);
                    self.pending_structured_output_records
                        .schedule_after_empty_poll(provider_run_id, now_ms);
                    continue;
                }
                Err(error) => {
                    let reconcile_result = if is_requested_run {
                        self.reconcile_provider_run_exit(
                            requested_session_id,
                            requested_provider_run_id,
                        )
                    } else {
                        self.provider_store
                            .get_run(&provider_run_id)
                            .and_then(|run| {
                                let session_id = run.session_id().to_string();
                                self.reconcile_provider_run_exit(&session_id, &provider_run_id)
                            })
                    };
                    match reconcile_result {
                        Ok(true) => {
                            self.pending_structured_output_records
                                .stop_polling(&provider_run_id);
                            continue;
                        }
                        Ok(false) => {
                            let retry_attempt = self
                                .pending_structured_output_records
                                .schedule_after_poll_failure(&provider_run_id, now_ms);
                            if retry_attempt.is_none() {
                                crate::logging::error_with_fields(
                                    "daemon.app",
                                    "structured output polling abandoned after repeated failures",
                                    serde_json::json!({
                                        "session_id": if is_requested_run {
                                            Some(requested_session_id)
                                        } else {
                                            None
                                        },
                                        "provider_run_id": provider_run_id,
                                        "retry_limit": STRUCTURED_OUTPUT_POLL_FAILURE_RETRY_LIMIT,
                                        "error": error.to_string(),
                                    }),
                                );
                                if is_requested_run {
                                    return Err(error);
                                }
                                continue;
                            }
                            crate::logging::warn_with_fields(
                                "daemon.app",
                                "structured output poll failed; retry scheduled",
                                serde_json::json!({
                                    "session_id": if is_requested_run {
                                        Some(requested_session_id)
                                    } else {
                                        None
                                    },
                                    "provider_run_id": provider_run_id,
                                    "retry_attempt": retry_attempt,
                                    "error": error.to_string(),
                                }),
                            );
                            continue;
                        }
                        Err(reconcile_error) if is_requested_run => return Err(reconcile_error),
                        Err(reconcile_error) => {
                            let retry_attempt = self
                                .pending_structured_output_records
                                .schedule_after_poll_failure(&provider_run_id, now_ms);
                            let message = if retry_attempt.is_some() {
                                "background structured output poll reconciliation failed; retry scheduled"
                            } else {
                                "background structured output poll reconciliation abandoned after repeated failures"
                            };
                            crate::logging::error_with_fields(
                                "daemon.app",
                                message,
                                serde_json::json!({
                                    "provider_run_id": provider_run_id,
                                    "retry_attempt": retry_attempt,
                                    "retry_limit": STRUCTURED_OUTPUT_POLL_FAILURE_RETRY_LIMIT,
                                    "error": reconcile_error.to_string(),
                                }),
                            );
                            continue;
                        }
                    }
                }
            };
            let provider_run = match self.provider_store.get_run(&provider_run_id) {
                Ok(run) => run,
                Err(_) => {
                    self.pending_structured_output_records
                        .clear(&provider_run_id);
                    continue;
                }
            };
            let session_id = provider_run.session_id().to_string();
            let active_prompt = provider_run
                .agent_instance_id()
                .map(str::to_string)
                .and_then(|agent_id| {
                    self.app
                        .prompt_owner_active_prompt_for_agent(&session_id, &agent_id)
                        .ok()
                        .flatten()
                });
            let active_prompt_id = active_prompt.as_ref().map(|prompt| prompt.id().to_string());
            let active_prompt_is_dispatching = active_prompt
                .as_ref()
                .is_some_and(|prompt| prompt.delivery_pending());
            if polled_prompt_id != active_prompt_id || active_prompt_is_dispatching {
                crate::logging::debug_with_fields(
                    "daemon.provider",
                    "discarding stale structured output poll before prompt delivery",
                    serde_json::json!({
                        "provider_run_id": provider_run_id,
                        "polled_prompt_id": polled_prompt_id,
                        "active_prompt_id": active_prompt_id,
                        "active_prompt_is_dispatching": active_prompt_is_dispatching,
                    }),
                );
                self.pending_structured_output_records
                    .schedule_next_poll(provider_run_id, now_ms);
                continue;
            }
            let recipient_attachment_ids = if is_requested_run {
                requested_recipient_attachment_ids.clone()
            } else {
                self.recipient_attachment_ids_for_session(&session_id)
            };
            let next_poll_due_at_ms =
                if structured_output_batch_should_poll_immediately(&poll_result) {
                    now_ms
                } else {
                    now_ms.saturating_add(STRUCTURED_OUTPUT_EMPTY_POLL_BACKOFF_MS)
                };
            let retry_poll_result = poll_result.clone();
            let poll_result = match self.prepare_structured_output_batch(
                &session_id,
                &provider_run_id,
                poll_result,
            ) {
                Ok(poll_result) => poll_result,
                Err(error) if crate::durable_state::is_retryable_durable_write_error(&error) => {
                    self.pending_structured_output_records
                        .mark_poll_enqueued(&provider_run_id, polled_prompt_id.clone());
                    self.provider_store
                        .schedule_finished_structured_output_poll_retry(
                            crate::provider::FinishedProviderOutputPollJob {
                                provider_run_id: provider_run_id.clone(),
                                result: Ok(Some(retry_poll_result)),
                                settlement_retry_attempt,
                            },
                        );
                    crate::logging::warn_with_fields(
                        "durable_state.recovery",
                        "deferred app structured output resume persistence after durable write failure",
                        serde_json::json!({
                            "provider_run_id": &provider_run_id,
                            "settlement_retry_attempt": settlement_retry_attempt.saturating_add(1),
                            "error": error.to_string(),
                        }),
                    );
                    continue;
                }
                Err(error) => return Err(error),
            };
            let records = self.apply_prepared_structured_output_batch(
                &session_id,
                &provider_run_id,
                recipient_attachment_ids,
                poll_result,
            )?;
            if is_requested_run {
                requested_records.extend(records);
            } else {
                self.pending_structured_output_records
                    .append(provider_run_id.clone(), records);
            }
            self.pending_structured_output_records
                .schedule_next_poll(provider_run_id, next_poll_due_at_ms);
        }
        Ok(requested_records)
    }

    fn prepare_structured_output_batch(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        poll_result: ProviderPromptSignalBatch,
    ) -> Result<ProviderPromptSignalBatch, DaemonError> {
        self.trace_structured_poll_batch(
            session_id,
            provider_run_id,
            "structured_poll_batch_received",
            &poll_result,
        );
        let projected_provider_run = self
            .provider_store
            .preview_structured_output_metadata(provider_run_id, &poll_result)?;
        self.persist_resolved_resume_state(&projected_provider_run, &poll_result)?;
        Ok(poll_result)
    }

    fn apply_prepared_structured_output_batch(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
        poll_result: ProviderPromptSignalBatch,
    ) -> Result<Vec<TerminalOutputRecord>, DaemonError> {
        self.provider_store
            .apply_structured_output_metadata(provider_run_id, &poll_result)?;
        let provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;
        self.mark_resolved_external_provider_session_attached(&provider_run);
        self.app
            .update_provider_run_projection(provider_run.clone());
        let terminal_sink = ProviderOutputFanout::new(self.app);
        for notice in &poll_result.notices {
            terminal_sink.record_notice(
                session_id,
                Some(provider_run_id),
                recipient_attachment_ids.clone(),
                notice.to_string(),
            );
        }
        let saw_response_content = poll_result.chunks.iter().any(|chunk| {
            matches!(
                chunk.kind,
                TerminalOutputKind::ProviderOutput | TerminalOutputKind::ProviderReasoning
            )
        });
        let saw_runtime_activity = poll_result.chunks.iter().any(|chunk| {
            matches!(
                chunk.kind,
                TerminalOutputKind::ProviderOutput
                    | TerminalOutputKind::ProviderReasoning
                    | TerminalOutputKind::ProviderTool
                    | TerminalOutputKind::ProviderStatus
            )
        });
        let saw_settlement_blocking_activity = poll_result.chunks.iter().any(|chunk| {
            matches!(
                chunk.kind,
                TerminalOutputKind::ProviderOutput
                    | TerminalOutputKind::ProviderReasoning
                    | TerminalOutputKind::ProviderTool
            )
        });
        for chunk in &poll_result.chunks {
            if chunk.kind == TerminalOutputKind::ProviderTool {
                crate::transport::flow_control::note_prompt_tool_output(
                    self.app,
                    provider_run_id,
                    chunk.merge_key.as_deref(),
                    &chunk.bytes,
                );
            }
        }
        if saw_response_content {
            self.note_prompt_response_content(provider_run_id);
        } else if saw_runtime_activity {
            self.note_prompt_output(provider_run_id);
        }
        let completions = poll_result.completions;
        let prompt_completed = poll_result.prompt_completed;
        let terminal_failure = poll_result.terminal_failure.clone();
        if let Some(message) = terminal_failure.as_deref() {
            let run = self
                .provider_store
                .record_terminal_diagnostic(provider_run_id, message.to_string())?;
            self.app.update_provider_run_projection(run);
        }
        let records: Vec<TerminalOutputRecord> = poll_result
            .chunks
            .into_iter()
            .filter_map(|chunk| {
                let record = self.fan_out_terminal_output(
                    session_id,
                    provider_run_id,
                    chunk.kind,
                    chunk.merge_key,
                    recipient_attachment_ids.clone(),
                    &chunk.bytes,
                );
                if record.pending_recipient_attachment_ids.is_empty() && record.bytes.is_empty() {
                    None
                } else {
                    Some(record)
                }
            })
            .collect();
        self.trace_terminal_records(
            session_id,
            provider_run_id,
            "structured_poll_records_fanned_out",
            &records,
        );
        for completion in &completions {
            terminal_sink.record_assistant_message_completion(
                session_id,
                provider_run_id,
                recipient_attachment_ids.clone(),
                &completion.message_id,
                completion.completed_at_ms,
            );
            self.mark_prompt_completion_recorded(provider_run_id);
        }
        let exited = self.reconcile_provider_run_exit(session_id, provider_run_id)?;
        if exited {
            self.trace_prompt_state(
                session_id,
                provider_run_id,
                "structured_poll_provider_exited",
            );
            return Ok(records);
        }
        if let Some(message) = terminal_failure {
            self.fail_prompt_for_terminal_failure(session_id, provider_run_id, &message)?;
            self.trace_prompt_state(
                session_id,
                provider_run_id,
                "structured_poll_terminal_failure_settled",
            );
            return Ok(records);
        }
        let should_trace_settlement =
            prompt_completed || saw_settlement_blocking_activity || !records.is_empty();
        if should_trace_settlement {
            self.trace_prompt_state(
                session_id,
                provider_run_id,
                "structured_poll_before_settlement",
            );
        }
        let settlement = self.settle_structured_prompt_completion(
            session_id,
            provider_run_id,
            prompt_completed,
            saw_settlement_blocking_activity,
        );
        if should_trace_settlement || settlement.is_err() {
            self.trace_prompt_state(
                session_id,
                provider_run_id,
                if settlement.is_ok() {
                    "structured_poll_after_settlement"
                } else {
                    "structured_poll_settlement_error"
                },
            );
        }
        settlement?;
        Ok(records)
    }

    fn persist_resolved_resume_state(
        &mut self,
        provider_run: &RuntimeProviderRun,
        poll_result: &ProviderPromptSignalBatch,
    ) -> Result<(), DaemonError> {
        let Some(resume_state) = poll_result.resolved_resume_state.as_ref() else {
            return Ok(());
        };
        let Some(agent_id) = provider_run.agent_instance_id() else {
            return Ok(());
        };
        let durable_state_store = self.app.durable_state_store();
        self.app.agents.set_agent_runtime_profile_durably(
            &durable_state_store,
            agent_id,
            provider_run.provider(),
            Some(provider_run.model().to_string()),
            provider_run.variant().map(str::to_string),
            None,
            resume_state.clone(),
            Some(provider_run.id()),
            None,
        )?;
        let _ = crate::app::KernelSessionReadService::new(self.app)
            .session_snapshot(provider_run.session_id())?;
        Ok(())
    }

    fn mark_resolved_external_provider_session_attached(&self, provider_run: &RuntimeProviderRun) {
        let Some(agent_id) = provider_run.agent_instance_id() else {
            return;
        };
        self.app
            .external_provider_sessions
            .mark_provider_run_attached(
                provider_run.adapter_key(),
                provider_run.account_profile(),
                provider_run.provider_session_id(),
                provider_run.resume_state(),
                provider_run.session_id(),
                agent_id,
            );
    }

    fn trace_structured_poll_batch(
        &self,
        session_id: &str,
        provider_run_id: &str,
        source: &str,
        poll_result: &ProviderPromptSignalBatch,
    ) {
        self.trace()
            .structured_poll_batch(session_id, provider_run_id, source, poll_result);
    }

    fn trace_terminal_records(
        &self,
        session_id: &str,
        provider_run_id: &str,
        source: &str,
        records: &[TerminalOutputRecord],
    ) {
        self.trace()
            .terminal_records(session_id, provider_run_id, source, records);
    }

    fn trace_prompt_state(&self, session_id: &str, provider_run_id: &str, source: &str) {
        self.trace()
            .prompt_state_turn(session_id, provider_run_id, source);
    }

    fn trace(&self) -> ProviderOutputTrace {
        ProviderOutputTrace::new(
            self.app,
            self.provider_store.clone(),
            self.active_turns.clone(),
            self.prompt_activity.clone(),
        )
    }

    fn drain_pty_output(
        &mut self,
        provider_run_id: &str,
    ) -> Result<Vec<PtyOutputChunk>, DaemonError> {
        ProviderOutputPtyDrain::new(self.app).drain_output(provider_run_id)
    }

    fn process_claude_native_tui_bridge(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        provider_run: &RuntimeProviderRun,
    ) -> Result<Option<String>, DaemonError> {
        // The interactive TUI pump revisits transcripts on its own cadence, so
        // the deferred-drain hint is not needed on this path.
        ProviderOutputClaudeNativeBridge::new(self.app)
            .process(
                session_id,
                provider_run_id,
                provider_run,
                self.provider_store.native_interaction_bridge(),
            )
            .map(|outcome| outcome.terminal_failure)
    }

    fn process_claude_native_terminal_output_bridge(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        provider_run: &RuntimeProviderRun,
        rendered: &str,
    ) -> Result<(), DaemonError> {
        ProviderOutputClaudeNativeBridge::new(self.app).process_terminal_output(
            session_id,
            provider_run_id,
            provider_run,
            self.provider_store.native_interaction_bridge(),
            rendered,
        )
    }

    fn recipient_attachment_ids_for_session(&self, session_id: &str) -> Vec<String> {
        ProviderOutputRecipientResolver::new(self.app).session_attachment_ids(session_id)
    }

    fn note_prompt_output(&mut self, provider_run_id: &str) {
        crate::transport::flow_control::note_prompt_output(self.app, provider_run_id);
    }

    fn note_prompt_response_content(&mut self, provider_run_id: &str) {
        crate::transport::flow_control::note_prompt_response_content(self.app, provider_run_id);
    }

    fn mark_prompt_completion_recorded(&self, provider_run_id: &str) {
        if let Some(state) = self.prompt_activity.write().get_mut(provider_run_id) {
            state.completion_recorded = true;
        }
    }

    fn settle_structured_prompt_completion(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        prompt_completed: bool,
        saw_settlement_blocking_activity: bool,
    ) -> Result<(), DaemonError> {
        self.prompt_settlement().settle_structured_completion(
            session_id,
            provider_run_id,
            prompt_completed,
            saw_settlement_blocking_activity,
        )
    }

    fn settle_pty_prompt_if_quiet(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<(), DaemonError> {
        self.prompt_settlement()
            .settle_pty_if_quiet(session_id, provider_run_id)
    }

    fn fail_prompt_for_terminal_failure(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        message: &str,
    ) -> Result<(), DaemonError> {
        self.prompt_settlement()
            .fail_for_terminal_failure(session_id, provider_run_id, message)
    }

    fn prompt_settlement(&mut self) -> ProviderOutputPromptSettlement<'_> {
        ProviderOutputPromptSettlement::new(
            self.app,
            self.provider_store.clone(),
            self.active_turns.clone(),
            self.prompt_activity.clone(),
            self.agent_runtime_projection.clone(),
        )
    }

    fn fan_out_provider_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> TerminalOutputRecord {
        self.fan_out_terminal_output(
            session_id,
            provider_run_id,
            TerminalOutputKind::ProviderOutput,
            None,
            recipient_attachment_ids,
            bytes,
        )
    }

    fn fan_out_terminal_output(
        &self,
        session_id: &str,
        provider_run_id: &str,
        kind: TerminalOutputKind,
        merge_key: Option<String>,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> TerminalOutputRecord {
        ProviderOutputFanout::new(self.app).fan_out(
            session_id,
            provider_run_id,
            kind,
            merge_key,
            recipient_attachment_ids,
            bytes,
        )
    }
}

impl DaemonApp {
    pub(crate) fn process_claude_native_bridge_for_runtime(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        provider_run: &RuntimeProviderRun,
    ) -> Result<crate::app::ClaudeNativeProcessOutcome, DaemonError> {
        let native_interaction_bridge = self.providers.native_interaction_bridge();
        ProviderOutputClaudeNativeBridge::new(self).process(
            session_id,
            provider_run_id,
            provider_run,
            native_interaction_bridge,
        )
    }

    pub(crate) fn finish_deferred_claude_native_stop_for_runtime(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        provider_run: &RuntimeProviderRun,
    ) -> Result<(), DaemonError> {
        ProviderOutputClaudeNativeBridge::new(self).finish_deferred_stop(
            session_id,
            provider_run_id,
            provider_run,
        )
    }

    pub(crate) fn process_claude_native_prompt_dispatch_attempt_for_runtime(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        provider_run: &RuntimeProviderRun,
        dispatch: &crate::app::KernelPromptDispatch,
    ) -> Result<crate::app::ClaudeNativeDispatchAttempt, DaemonError> {
        ProviderOutputClaudeNativeBridge::new(self).process_prompt_dispatch_attempt(
            session_id,
            provider_run_id,
            provider_run,
            dispatch,
        )
    }

    pub(crate) fn process_claude_native_terminal_output_bridge_for_runtime(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        provider_run: &RuntimeProviderRun,
        rendered: &str,
    ) -> Result<(), DaemonError> {
        let native_interaction_bridge = self.providers.native_interaction_bridge();
        ProviderOutputClaudeNativeBridge::new(self).process_terminal_output(
            session_id,
            provider_run_id,
            provider_run,
            native_interaction_bridge,
            rendered,
        )
    }
}
