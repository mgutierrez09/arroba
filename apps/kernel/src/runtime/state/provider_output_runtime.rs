//! Provider output pumping and terminal snapshot orchestration.
//!
//! These methods bridge owned runtime state with provider processes/endpoints and translate
//! provider runtime events back into prompt/session mutations.

use super::*;

const PTY_PROMPT_SETTLE_QUIET_FOR: std::time::Duration = std::time::Duration::from_millis(50);

impl KernelRuntimeState {
    pub(super) async fn pump_owned_provider_output(
        &self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
        initial_liveness_already_checked: bool,
    ) -> Result<Vec<crate::terminal::TerminalOutputRecord>, DaemonError> {
        let owned = &self.owned;
        owned.reap_structured_prompt_jobs();
        self.reap_provider_first_output_timeouts(session_id).await?;
        self.reap_provider_inactivity_timeouts(session_id).await?;
        let mut provider_run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        let uses_structured_prompt_io = provider_run_uses_structured_output_pump(&provider_run);
        if !initial_liveness_already_checked
            && uses_structured_prompt_io
            && self
                .reconcile_provider_run_exit(session_id, provider_run_id)
                .await?
        {
            return Ok(owned
                .structured_output_records
                .take_and_stop_polling(provider_run_id));
        }
        if provider_run.state() == crate::provider::ProviderRunState::Ended {
            return Ok(owned
                .structured_output_records
                .take_and_stop_polling(provider_run_id));
        }
        if provider_run.state() == crate::provider::ProviderRunState::Parked {
            if !owned.provider_run_has_active_prompt(session_id, &provider_run)? {
                return Ok(owned
                    .structured_output_records
                    .take_and_stop_polling(provider_run_id));
            }
            provider_run = owned.provider_store.resume_run_detached(provider_run_id)?;
            owned.provider_run_projection.update(provider_run.clone());
            crate::logging::warn_with_fields(
                "daemon.provider",
                "resumed parked provider run that still had an active prompt",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                    "agent_id": provider_run.agent_instance_id(),
                }),
            );
        }

        if uses_structured_prompt_io {
            return self
                .pump_owned_structured_provider_output(
                    session_id,
                    provider_run_id,
                    recipient_attachment_ids,
                )
                .await;
        }

        if crate::provider::provider_run_uses_claude_native_bridge(&provider_run) {
            let provider_run = provider_run.clone();
            let outcome = self
                .with_app_side_effect(|app| {
                    app.process_claude_native_bridge_for_runtime(
                        session_id,
                        provider_run_id,
                        &provider_run,
                    )
                })
                .await?;
            if let Some(message) = outcome.terminal_failure {
                let run = owned
                    .provider_store
                    .record_terminal_diagnostic(provider_run_id, message.clone())?;
                owned.provider_run_projection.update(run);
                self.fail_owned_provider_prompt(session_id, provider_run_id, &message, true)
                    .await?;
                return Ok(Vec::new());
            }
            if outcome.needs_deferred_transcript_drain {
                // The final Claude transcript flush can trail the Stop event;
                // wait for it off the app lock so the whole daemon stays
                // responsive, then drain once more before settling.
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                self.with_app_side_effect(|app| {
                    app.finish_deferred_claude_native_stop_for_runtime(
                        session_id,
                        provider_run_id,
                        &provider_run,
                    )
                })
                .await?;
            }
        }

        let mut chunks = match self
            .with_app_side_effect(|app| app.drain_provider_pty_output_for_runtime(provider_run_id))
            .await
        {
            Ok(chunks) => chunks,
            Err(error) => {
                if provider_run.state() == crate::provider::ProviderRunState::Starting
                    && matches!(error, DaemonError::PtyProcessNotFound { .. })
                {
                    return Ok(Vec::new());
                }
                if self
                    .reconcile_provider_run_exit(session_id, provider_run_id)
                    .await?
                {
                    return Ok(owned
                        .structured_output_records
                        .take_and_stop_polling(provider_run_id));
                }
                return Err(error);
            }
        };
        if crate::provider::provider_run_uses_claude_native_bridge(&provider_run) {
            let rendered = chunks
                .iter()
                .map(|chunk| String::from_utf8_lossy(&chunk.bytes))
                .collect::<String>();
            if !rendered.is_empty() {
                let provider_run = provider_run.clone();
                self.with_app_side_effect(|app| {
                    app.process_claude_native_terminal_output_bridge_for_runtime(
                        session_id,
                        provider_run_id,
                        &provider_run,
                        &rendered,
                    )
                })
                .await?;
            }
        }
        let uses_transient_native_terminal =
            provider_run_uses_transient_native_terminal(&provider_run);
        if !chunks.is_empty() {
            if crate::provider::provider_run_is_claude_headless(&provider_run)
                || uses_transient_native_terminal
            {
                owned.note_prompt_output(provider_run_id);
            } else {
                owned.note_prompt_response_content(provider_run_id);
            }
            owned
                .schedule_provider_output_check_after(provider_run_id, PTY_PROMPT_SETTLE_QUIET_FOR);
        }
        let terminal_failure = crate::provider::classify_provider_terminal_failure_output_text(
            provider_run.adapter_key(),
            &chunks
                .iter()
                .map(|chunk| String::from_utf8_lossy(&chunk.bytes))
                .collect::<String>(),
        );
        let has_active_prompt = owned.provider_run_has_active_prompt(session_id, &provider_run)?;
        if !crate::app::provider_output::should_project_pty_output(
            has_active_prompt,
            terminal_failure.as_deref(),
            uses_transient_native_terminal,
        ) {
            chunks.clear();
        }
        let records = if crate::provider::provider_run_is_claude_headless(&provider_run) {
            Vec::new()
        } else {
            let agent_id = provider_run.agent_instance_id().map(str::to_string);
            let prompt_metadata =
                owned.active_prompt_transcript_metadata_for_agent(session_id, agent_id.as_deref());
            let mut history_entries = Vec::with_capacity(chunks.len());
            let terminal_outputs = chunks
                .into_iter()
                .map(|chunk| {
                    let history_text = (!uses_transient_native_terminal)
                        .then(|| String::from_utf8_lossy(&chunk.bytes).into_owned());
                    if let Some(history_text) = history_text.as_ref() {
                        history_entries.push(
                            crate::history::SessionHistoryEntry::provider_output(
                                session_id,
                                provider_run_id,
                                agent_id.as_deref(),
                                crate::terminal::TerminalOutputKind::ProviderOutput,
                                None,
                                history_text.clone(),
                            )
                            .with_prompt_origin(prompt_metadata.prompt_origin)
                            .with_source_attachment_id(
                                prompt_metadata.source_attachment_id.clone(),
                            ),
                        );
                    }
                    super::prompt_transcript_owned_state::TerminalOutputBatchAppend {
                        provider_run_id: provider_run_id.to_string(),
                        agent_id: agent_id.clone(),
                        kind: if uses_transient_native_terminal {
                            crate::terminal::TerminalOutputKind::ProviderTerminal
                        } else {
                            crate::terminal::TerminalOutputKind::ProviderOutput
                        },
                        merge_key: None,
                        bytes: chunk.bytes,
                    }
                })
                .collect::<Vec<_>>();
            let records = owned.fan_out_terminal_outputs_to_recipients(
                session_id,
                recipient_attachment_ids,
                terminal_outputs,
            );
            owned.append_history_entries(session_id, history_entries);
            records
        };
        if let Some(message) = terminal_failure {
            let run = owned
                .provider_store
                .record_terminal_diagnostic(provider_run_id, message.clone())?;
            owned.provider_run_projection.update(run);
            self.fail_owned_provider_prompt(session_id, provider_run_id, &message, true)
                .await?;
            return Ok(records);
        }
        // Settlement can include cancellation, queue advancement, and remote
        // substitute reconciliation. Keep that future off enclosing
        // launch/dispatch/output stack frames.
        if !Box::pin(self.reconcile_provider_run_exit(session_id, provider_run_id)).await? {
            if records.is_empty() {
                let _ = self
                    .settle_owned_pty_prompt_if_quiet(session_id, provider_run_id)
                    .await?;
                owned.ensure_provider_output_timeout_scheduled(provider_run_id);
            }
        }
        Ok(records)
    }

    async fn settle_owned_pty_prompt_if_quiet(
        &self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<crate::app::ProviderRunExitSessionSummary, DaemonError> {
        let owned = &self.owned;
        let provider_run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        if !provider_run_allows_quiet_pty_settlement(&provider_run)
            || !owned
                .prompt_output_quiet_after_response(provider_run_id, PTY_PROMPT_SETTLE_QUIET_FOR)
        {
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: false,
                cancelled_prompt: false,
                started_next_prompt: false,
            });
        }
        let Some(agent_id) = provider_run.agent_instance_id() else {
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: false,
                cancelled_prompt: false,
                started_next_prompt: false,
            });
        };
        let session = owned.session_store.get_session(session_id)?;
        let Some(active_prompt) = owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
        else {
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: false,
                cancelled_prompt: false,
                started_next_prompt: false,
            });
        };
        if active_prompt.status() != crate::session::PromptStatus::Cancelling {
            if let (Some(workflow_run_id), Some(workflow_node_run_id)) = (
                active_prompt.workflow_run_id(),
                active_prompt.workflow_node_run_id(),
            ) {
                if !owned.workflow_prompt_has_completion_output(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                    provider_run_id,
                ) {
                    return Ok(crate::app::ProviderRunExitSessionSummary {
                        had_active_prompt: true,
                        cancelled_prompt: false,
                        started_next_prompt: false,
                    });
                }
            }
        }
        self.settle_owned_provider_prompt(session_id, provider_run_id, true, false, false)
            .await
    }

    pub(crate) async fn pump_terminal_output_with_snapshot(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<
        (
            Vec<crate::terminal::TerminalOutputRecord>,
            Option<crate::session::RuntimeSession>,
        ),
        DaemonError,
    > {
        let owned = &self.owned;
        let projected_session = owned.session_projection.get(session_id);
        owned.reap_structured_prompt_jobs();
        self.reap_provider_first_output_timeouts(session_id).await?;
        self.reap_provider_inactivity_timeouts(session_id).await?;
        owned.ensure_attachment_in_session(session_id, attachment_id)?;
        self.spawn_workflow_prompt_dispatches(owned.workflow_retry_blocked_claims());
        let session = owned.session_store.get_session(session_id)?;
        let provider_run_ids = provider_run_ids_for_owned_output_pump(owned, &session);
        let recipient_attachment_ids = owned
            .attachment_store
            .list_session_attachment_ids(session_id);
        for provider_run_id in &provider_run_ids {
            let result = self
                .pump_owned_provider_output(
                    session_id,
                    provider_run_id,
                    recipient_attachment_ids.clone(),
                    false,
                )
                .await;
            if let Err(error) = result {
                if matches!(error, DaemonError::ProviderRunNotFound { .. })
                    && owned
                        .provider_run_projection
                        .get(provider_run_id)
                        .is_some_and(|run| run.session_id() == session_id)
                {
                    continue;
                }
                return Err(error);
            }
            self.observe_git_after_provider_activity_if_pending(provider_run_id)
                .await;
        }
        self.drain_active_remote_prompt_projections_for_session(&session)
            .await?;
        let records = owned
            .terminal_stream
            .drain_output_records(session_id, attachment_id);
        let session = owned
            .session_snapshot_without_projection_update(session_id)
            .ok()
            .filter(|session| projected_session.as_ref() != Some(session));
        for provider_run_id in &provider_run_ids {
            self.observe_git_after_provider_activity_if_pending(provider_run_id)
                .await;
        }
        Ok((records, session))
    }

    async fn reap_provider_first_output_timeouts(
        &self,
        session_id: &str,
    ) -> Result<(), DaemonError> {
        let timed_out = first_output_timeout_candidates(&self.owned, session_id);
        for timeout in timed_out {
            let diagnostic =
                crate::app::provider_first_output_timeout_diagnostic(timeout.elapsed_ms);
            let run = self
                .owned
                .provider_store
                .record_terminal_diagnostic(&timeout.provider_run_id, diagnostic.clone())?;
            self.owned.provider_run_projection.update(run);
            let recipients = self
                .owned
                .attachment_store
                .list_session_attachment_ids(session_id);
            self.owned.record_notice(
                session_id,
                Some(&timeout.provider_run_id),
                recipients,
                diagnostic.clone(),
            );
            crate::logging::warn_with_fields(
                "daemon.provider",
                "provider prompt produced no first output before timeout",
                serde_json::json!({
                    "session_id": session_id,
                    "agent_id": timeout.agent_id,
                    "provider_run_id": timeout.provider_run_id,
                    "elapsed_ms": timeout.elapsed_ms,
                }),
            );
            self.fail_owned_provider_prompt(
                session_id,
                &timeout.provider_run_id,
                &diagnostic,
                true,
            )
            .await?;
        }
        Ok(())
    }

    pub(super) async fn reap_provider_inactivity_timeouts(
        &self,
        session_id: &str,
    ) -> Result<(), DaemonError> {
        let timed_out = inactivity_timeout_candidates(&self.owned, session_id);
        for timeout in timed_out {
            let diagnostic = crate::app::provider_inactivity_timeout_diagnostic(timeout.elapsed_ms);
            let run = self
                .owned
                .provider_store
                .record_terminal_diagnostic(&timeout.provider_run_id, diagnostic.clone())?;
            self.owned.provider_run_projection.update(run);
            let recipients = self
                .owned
                .attachment_store
                .list_session_attachment_ids(session_id);
            self.owned.record_notice(
                session_id,
                Some(&timeout.provider_run_id),
                recipients,
                diagnostic.clone(),
            );
            crate::logging::warn_with_fields(
                "daemon.provider",
                "provider prompt produced no output after prior activity before timeout",
                serde_json::json!({
                    "session_id": session_id,
                    "agent_id": timeout.agent_id,
                    "provider_run_id": timeout.provider_run_id,
                    "elapsed_ms": timeout.elapsed_ms,
                }),
            );
            self.fail_owned_provider_prompt(
                session_id,
                &timeout.provider_run_id,
                &diagnostic,
                true,
            )
            .await?;
        }
        Ok(())
    }
}

fn provider_run_uses_transient_native_terminal(
    provider_run: &crate::provider::RuntimeProviderRun,
) -> bool {
    crate::provider::provider_run_uses_claude_native_bridge(provider_run)
        && !crate::provider::provider_run_is_claude_headless(provider_run)
}

pub(super) fn provider_run_allows_quiet_pty_settlement(
    provider_run: &crate::provider::RuntimeProviderRun,
) -> bool {
    !provider_run_uses_structured_output_pump(provider_run)
        && !crate::provider::provider_run_uses_claude_native_bridge(provider_run)
}

pub(super) fn provider_run_uses_structured_output_pump(
    provider_run: &crate::provider::RuntimeProviderRun,
) -> bool {
    crate::provider::provider_run_uses_structured_prompt_io(provider_run)
}

fn first_output_timeout_candidates(
    owned: &KernelRuntimeOwnedState,
    session_id: &str,
) -> Vec<crate::app::ProviderFirstOutputTimeoutCandidate> {
    let prompt_activity = owned.prompt_activity.read().clone();
    let active_turns = owned.active_turns.snapshot();
    let Ok(session) = owned.session_store.get_session(session_id) else {
        return Vec::new();
    };
    crate::app::provider_first_output_timeout_candidates(
        session_id,
        active_turns.into_values(),
        &prompt_activity,
        |turn| {
            owned
                .provider_store
                .get_run(&turn.provider_run_id)
                .is_ok_and(|run| {
                    run.session_id() == session_id
                        && run.agent_instance_id() == Some(turn.agent_id.as_str())
                        && run.terminal_diagnostic().is_none()
                        && matches!(
                            run.state(),
                            crate::provider::ProviderRunState::Starting
                                | crate::provider::ProviderRunState::Running
                                | crate::provider::ProviderRunState::Parked
                        )
                })
        },
        |turn| {
            owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, &turn.agent_id)
                .is_some_and(|prompt| prompt.id() == turn.prompt_id)
        },
    )
}

fn inactivity_timeout_candidates(
    owned: &KernelRuntimeOwnedState,
    session_id: &str,
) -> Vec<crate::app::ProviderInactivityTimeoutCandidate> {
    let prompt_activity = owned.prompt_activity.read().clone();
    let active_turns = owned.active_turns.snapshot();
    let Ok(session) = owned.session_store.get_session(session_id) else {
        return Vec::new();
    };
    crate::app::provider_inactivity_timeout_candidates(
        session_id,
        active_turns.into_values(),
        &prompt_activity,
        |turn| {
            owned
                .provider_store
                .get_run(&turn.provider_run_id)
                .is_ok_and(|run| {
                    run.session_id() == session_id
                        && run.agent_instance_id() == Some(turn.agent_id.as_str())
                        && run.terminal_diagnostic().is_none()
                        && matches!(
                            run.state(),
                            crate::provider::ProviderRunState::Starting
                                | crate::provider::ProviderRunState::Running
                                | crate::provider::ProviderRunState::Parked
                        )
                })
        },
        |turn| {
            owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, &turn.agent_id)
                .is_some_and(|prompt| prompt.id() == turn.prompt_id)
        },
    )
}

pub(super) fn provider_run_ids_for_owned_output_pump(
    owned: &KernelRuntimeOwnedState,
    session: &crate::session::RuntimeSession,
) -> BTreeSet<String> {
    let mut provider_run_ids = BTreeSet::new();
    if let Some(provider_run_id) = session.active_provider_run_id().filter(|run_id| {
        owned
            .provider_store
            .get_run(run_id)
            .is_ok_and(|run| provider_run_requires_owned_output_pump(owned, session, &run))
    }) {
        provider_run_ids.insert(provider_run_id.to_string());
    }
    for agent_id in owned.prompt_state_owner.active_prompt_agent_ids(session) {
        if let Some(provider_run_id) = owned
            .provider_store
            .get_run_for_agent(session.id(), &agent_id)
            .map(|run| run.id().to_string())
        {
            provider_run_ids.insert(provider_run_id);
        }
    }
    provider_run_ids.extend(
        owned
            .provider_store
            .list_runs()
            .into_iter()
            .filter(|run| run.session_id() == session.id())
            .filter(|run| {
                !run.client_interface().is_chariox()
                    && matches!(
                        run.state(),
                        crate::provider::ProviderRunState::Starting
                            | crate::provider::ProviderRunState::Running
                    )
            })
            .map(|run| run.id().to_string()),
    );
    provider_run_ids.extend(
        owned
            .git_turn_snapshots
            .provider_run_ids_for_session(session.id()),
    );
    provider_run_ids
}

fn provider_run_requires_owned_output_pump(
    owned: &KernelRuntimeOwnedState,
    session: &crate::session::RuntimeSession,
    run: &crate::provider::RuntimeProviderRun,
) -> bool {
    if run.state() == crate::provider::ProviderRunState::Starting {
        return true;
    }
    if !run.client_interface().is_chariox()
        && matches!(
            run.state(),
            crate::provider::ProviderRunState::Starting
                | crate::provider::ProviderRunState::Running
        )
    {
        return true;
    }
    run.agent_instance_id().is_some_and(|agent_id| {
        owned
            .prompt_state_owner
            .active_prompt_for_agent_snapshot(session, agent_id)
            .is_some()
    })
}
