use super::*;

const STRUCTURED_PROMPT_SETTLE_QUIET_FOR: std::time::Duration =
    std::time::Duration::from_millis(50);
const WORKFLOW_MISSING_OUTPUT_SETTLE_QUIET_FOR: std::time::Duration =
    std::time::Duration::from_millis(
        crate::app::provider_output::STRUCTURED_OUTPUT_EMPTY_POLL_BACKOFF_MS + 50,
    );

impl KernelRuntimeState {
    pub(super) async fn settle_owned_provider_prompt(
        &self,
        session_id: &str,
        provider_run_id: &str,
        prompt_completed: bool,
        saw_settlement_blocking_activity: bool,
        force: bool,
    ) -> Result<crate::app::ProviderRunExitSessionSummary, DaemonError> {
        let owned = &self.owned;
        let provider_run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        let agent_id = provider_run
            .agent_instance_id()
            .map(str::to_string)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: "provider run has no agent".to_string(),
            })?;
        if owned
            .provider_store
            .get_run_for_agent(session_id, &agent_id)
            .is_some_and(|current| {
                current.id() != provider_run_id
                    && current.state() != crate::provider::ProviderRunState::Ended
            })
        {
            if owned.clear_prompt_activity(provider_run_id) {
                self.spawn_workflow_prompt_dispatches(owned.workflow_retry_blocked_claims());
            }
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: false,
                cancelled_prompt: false,
                started_next_prompt: false,
            });
        }
        let active_prompt = owned
            .prompt_state_owner
            .active_prompt_for_agent(&owned.session_store.get_session(session_id)?, &agent_id);
        let Some(active_prompt) = active_prompt else {
            if !force && !prompt_completed {
                crate::logging::debug_with_fields(
                    "daemon.provider",
                    "settle provider prompt skipped without completion signal",
                    serde_json::json!({
                        "session_id": session_id,
                        "provider_run_id": provider_run_id,
                        "agent_id": agent_id,
                        "prompt_completed": prompt_completed,
                        "force": force,
                    }),
                );
                return Ok(crate::app::ProviderRunExitSessionSummary {
                    had_active_prompt: false,
                    cancelled_prompt: false,
                    started_next_prompt: false,
                });
            }
            crate::logging::debug_with_fields(
                "daemon.provider",
                "settle provider prompt found no active prompt",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                    "agent_id": agent_id,
                    "prompt_completed": prompt_completed,
                    "force": force,
                }),
            );
            if owned.clear_prompt_activity(provider_run_id) {
                self.spawn_workflow_prompt_dispatches(owned.workflow_retry_blocked_claims());
            }
            self.observe_git_after_provider_activity_if_pending(provider_run_id)
                .await;
            let _ = owned.sync_focused_provider_run_if_idle(session_id);
            let _ = owned.session_snapshot(session_id);
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: false,
                cancelled_prompt: false,
                started_next_prompt: false,
            });
        };
        if active_prompt.is_external() {
            crate::logging::debug_with_fields(
                "daemon.provider",
                "settle provider prompt ignored external active prompt",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                    "agent_id": agent_id,
                    "prompt_id": active_prompt.id(),
                    "prompt_completed": prompt_completed,
                    "force": force,
                }),
            );
            if owned.clear_prompt_activity(provider_run_id) {
                self.spawn_workflow_prompt_dispatches(owned.workflow_retry_blocked_claims());
            }
            self.observe_git_after_provider_activity_if_pending(provider_run_id)
                .await;
            let _ = owned.sync_focused_provider_run_if_idle(session_id);
            let _ = owned.session_snapshot(session_id);
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: false,
                cancelled_prompt: false,
                started_next_prompt: false,
            });
        }
        if !force && active_prompt.delivery_pending() {
            owned.schedule_provider_output_check_after(
                provider_run_id,
                STRUCTURED_PROMPT_SETTLE_QUIET_FOR,
            );
            crate::logging::debug_with_fields(
                "daemon.provider",
                "provider settlement ignored before prompt delivery acknowledgement",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                    "agent_id": agent_id,
                    "prompt_id": active_prompt.id(),
                    "prompt_completed": prompt_completed,
                }),
            );
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: true,
                cancelled_prompt: false,
                started_next_prompt: false,
            });
        }

        if prompt_completed {
            owned.mark_prompt_completion_recorded(provider_run_id);
        }
        let completion_recorded = owned.prompt_completion_recorded(provider_run_id);
        let settlement_pending = owned.prompt_completion_settlement_pending(provider_run_id);
        let codex_provider = provider_run.adapter_key() == "codex";
        if !force && codex_provider && !prompt_completed {
            owned.schedule_provider_output_check_after(
                provider_run_id,
                STRUCTURED_PROMPT_SETTLE_QUIET_FOR,
            );
            crate::logging::debug_with_fields(
                "daemon.provider",
                "codex prompt settlement waits for authoritative turn completion",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                    "agent_id": agent_id,
                    "prompt_id": active_prompt.id(),
                    "completion_recorded": completion_recorded,
                    "settlement_pending": settlement_pending,
                }),
            );
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: true,
                cancelled_prompt: false,
                started_next_prompt: false,
            });
        }
        if !force && !codex_provider && (prompt_completed || settlement_pending) {
            let quiet_after_response = owned.prompt_output_quiet_after_response(
                provider_run_id,
                STRUCTURED_PROMPT_SETTLE_QUIET_FOR,
            );
            if !settlement_pending || saw_settlement_blocking_activity || !quiet_after_response {
                owned.note_prompt_settlement_requested(provider_run_id);
                let _ = owned.session_snapshot(session_id);
                crate::logging::debug_with_fields(
                    "daemon.provider",
                    "provider completion is draining final output",
                    serde_json::json!({
                        "session_id": session_id,
                        "provider_run_id": provider_run_id,
                        "agent_id": agent_id,
                        "prompt_id": active_prompt.id(),
                        "settlement_pending": settlement_pending,
                        "saw_settlement_blocking_activity": saw_settlement_blocking_activity,
                        "quiet_after_response": quiet_after_response,
                    }),
                );
                return Ok(crate::app::ProviderRunExitSessionSummary {
                    had_active_prompt: true,
                    cancelled_prompt: false,
                    started_next_prompt: false,
                });
            }
        }
        let is_workflow_prompt = active_prompt.workflow_run_id().is_some();
        if is_workflow_prompt && !force && !prompt_completed && !settlement_pending {
            if completion_recorded {
                owned.note_prompt_settlement_requested(provider_run_id);
                let _ = owned.session_snapshot(session_id);
            }
            crate::logging::debug_with_fields(
                "daemon.provider",
                "workflow provider prompt settlement skipped until provider completion",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                    "agent_id": agent_id,
                    "prompt_id": active_prompt.id(),
                    "settlement_pending": settlement_pending,
                    "saw_settlement_blocking_activity": saw_settlement_blocking_activity,
                    "completion_recorded": completion_recorded,
                }),
            );
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: true,
                cancelled_prompt: false,
                started_next_prompt: false,
            });
        }
        if !force && !prompt_completed && !settlement_pending && completion_recorded {
            owned.note_prompt_settlement_requested(provider_run_id);
            let _ = owned.session_snapshot(session_id);
            if saw_settlement_blocking_activity {
                crate::logging::debug_with_fields(
                    "daemon.provider",
                    "provider completion is draining final output",
                    serde_json::json!({
                        "session_id": session_id,
                        "provider_run_id": provider_run_id,
                        "agent_id": agent_id,
                        "prompt_id": active_prompt.id(),
                    }),
                );
                return Ok(crate::app::ProviderRunExitSessionSummary {
                    had_active_prompt: true,
                    cancelled_prompt: false,
                    started_next_prompt: false,
                });
            }
        }
        if !force && !prompt_completed && !settlement_pending && !completion_recorded {
            crate::logging::debug_with_fields(
                "daemon.provider",
                "settle provider prompt skipped until provider completion",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                    "agent_id": agent_id,
                    "active_prompt_status": active_prompt.status(),
                }),
            );
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: true,
                cancelled_prompt: false,
                started_next_prompt: false,
            });
        }

        if !force && !prompt_completed && completion_recorded && saw_settlement_blocking_activity {
            owned.note_prompt_settlement_requested(provider_run_id);
            let _ = owned.session_snapshot(session_id);
            crate::logging::debug_with_fields(
                "daemon.provider",
                "provider completion is draining final output",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                    "agent_id": agent_id,
                    "prompt_id": active_prompt.id(),
                }),
            );
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: true,
                cancelled_prompt: false,
                started_next_prompt: false,
            });
        }

        if active_prompt.status() == crate::session::PromptStatus::Cancelling {
            if !force && completion_recorded && saw_settlement_blocking_activity {
                owned.note_prompt_settlement_requested(provider_run_id);
                let _ = owned.session_snapshot(session_id);
                return Ok(crate::app::ProviderRunExitSessionSummary {
                    had_active_prompt: true,
                    cancelled_prompt: false,
                    started_next_prompt: false,
                });
            }
            let cancellation = owned.finalize_local_prompt_cancellation_with_queued_advance(
                session_id,
                &agent_id,
                Some(provider_run_id),
            )?;
            owned.workflow_cancel_prompt(session_id, &cancellation.cancellation.prompt)?;
            if cancellation.released_claim {
                self.spawn_workflow_prompt_dispatches(owned.workflow_retry_blocked_claims());
            }
            if let Some(dispatch) = cancellation.dispatch {
                if let Err(error) = self
                    .enqueue_prompt_dispatch_after_liveness(&dispatch, owned)
                    .await
                {
                    let _ = self.fail_prompt_dispatch(dispatch, error).await;
                }
            }
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: true,
                cancelled_prompt: true,
                started_next_prompt: cancellation.cancellation.started_next.is_some(),
            });
        }

        if !force && !codex_provider && (prompt_completed || settlement_pending) {
            if let (Some(workflow_run_id), Some(workflow_node_run_id)) = (
                active_prompt.workflow_run_id(),
                active_prompt.workflow_node_run_id(),
            ) {
                if !owned.workflow_prompt_has_completion_output(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                    provider_run_id,
                ) && !owned.prompt_output_quiet_after_response(
                    provider_run_id,
                    WORKFLOW_MISSING_OUTPUT_SETTLE_QUIET_FOR,
                ) {
                    owned.note_prompt_settlement_requested(provider_run_id);
                    owned.schedule_provider_output_check_when_quiet(
                        provider_run_id,
                        WORKFLOW_MISSING_OUTPUT_SETTLE_QUIET_FOR,
                    );
                    let _ = owned.session_snapshot(session_id);
                    return Ok(crate::app::ProviderRunExitSessionSummary {
                        had_active_prompt: true,
                        cancelled_prompt: false,
                        started_next_prompt: false,
                    });
                }
            }
        }
        // A provider can terminate at the same time that it reports the active prompt as
        // complete (for example, when a managed provider socket is replaced during recovery).
        // Keep this fact so the common completion path can create a replacement run for queued
        // work instead of leaving the queue parked behind the ended run.
        let provider_run_was_running =
            provider_run.state() == crate::provider::ProviderRunState::Running;
        let next_queued_prompt_candidate = if provider_run_was_running {
            owned
                .prompt_state_owner
                .peek_next_queued_prompt(&owned.session_store.get_session(session_id)?, &agent_id)
        } else {
            None
        };
        // An ordinary provider has already discovered the reduced MCP surface.
        // Do not promote a workflow prompt onto it: complete the current turn
        // first, then the app-level queue path will replace the provider with a
        // workflow-scoped run before dispatching the queued prompt.
        let next_queued_workflow_event_capabilities = next_queued_prompt_candidate
            .as_ref()
            .filter(|prompt| {
                crate::scheduler::runtime::is_workflow_prompt_attachment(
                    prompt.source_attachment_id(),
                )
            })
            .map(|prompt| owned.workflow_event_capabilities_for_prompt(session_id, prompt))
            .transpose()?;
        let next_queued_workflow_requires_fresh_context = next_queued_prompt_candidate
            .as_ref()
            .filter(|prompt| {
                crate::scheduler::runtime::is_workflow_prompt_attachment(
                    prompt.source_attachment_id(),
                )
            })
            .map(|prompt| {
                owned.workflow_prompt_requires_fresh_provider_context(session_id, &agent_id, prompt)
            })
            .transpose()?
            .unwrap_or(false);
        let defer_queued_workflow_prompt =
            next_queued_prompt_candidate.as_ref().is_some_and(|prompt| {
                crate::scheduler::runtime::is_workflow_prompt_attachment(
                    prompt.source_attachment_id(),
                ) && (next_queued_workflow_requires_fresh_context
                    || !provider_run.workflow_tools_enabled()
                    || next_queued_workflow_event_capabilities.is_some_and(
                        |(reply, context, actions)| {
                            provider_run.workflow_event_reply_enabled() != reply
                                || provider_run.workflow_event_context_enabled() != context
                                || provider_run.workflow_event_actions_enabled() != actions
                        },
                    ))
            });
        let next_queued_prompt = (!defer_queued_workflow_prompt)
            .then_some(next_queued_prompt_candidate)
            .flatten();
        // Removing the active prompt is not the end of workflow settlement: the
        // path below still performs asynchronous post-provider work before it
        // records the workflow completion. Mark the run first so live orphan
        // reconciliation cannot stop a real run during that gap.
        let settling_workflow_run_id = active_prompt.workflow_run_id().map(str::to_string);
        if let Some(workflow_run_id) = settling_workflow_run_id.as_deref() {
            owned
                .session_store
                .write()
                .mark_workflow_run_settling(session_id, workflow_run_id)?;
        }
        let completion = if let Some(next_queued_prompt) = next_queued_prompt.as_ref() {
            owned.complete_local_prompt_with_queued_advance_if_matches(
                session_id,
                &agent_id,
                Some(provider_run_id),
                next_queued_prompt,
                Some(active_prompt.id()),
            )
        } else {
            owned.complete_local_prompt_without_advance_if_matches(
                session_id,
                &agent_id,
                Some(provider_run_id),
                Some(active_prompt.id()),
            )
        };
        let completion = match completion {
            Ok(completion) => completion,
            Err(error) => {
                if let Some(workflow_run_id) = settling_workflow_run_id.as_deref() {
                    owned
                        .session_store
                        .write()
                        .clear_workflow_run_settling(session_id, workflow_run_id)?;
                }
                return Err(error);
            }
        };
        let Some(completion) = completion else {
            if let Some(workflow_run_id) = settling_workflow_run_id.as_deref() {
                owned
                    .session_store
                    .write()
                    .clear_workflow_run_settling(session_id, workflow_run_id)?;
            }
            crate::logging::debug_with_fields(
                "daemon.provider",
                "ignored stale provider settlement after active prompt changed",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                    "agent_id": agent_id,
                    "expected_prompt_id": active_prompt.id(),
                }),
            );
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: true,
                cancelled_prompt: false,
                started_next_prompt: false,
            });
        };
        self.observe_git_after_prompt_completion(provider_run_id, &completion.completion.completed)
            .await;
        crate::logging::debug_with_fields(
            "daemon.provider",
            "settled provider prompt",
            serde_json::json!({
                "session_id": session_id,
                "provider_run_id": provider_run_id,
                "agent_id": agent_id,
                "prompt_completed": prompt_completed,
                "force": force,
                "started_next": completion.completion.started_next.is_some(),
                "released_claim": completion.released_claim,
            }),
        );
        if completion.completion.completed.workflow_run_id().is_some() {
            let workflow_completion = owned.workflow_complete_prompt(
                session_id,
                &completion.completion.completed,
                Some(provider_run_id),
            );
            if let Some(workflow_run_id) = settling_workflow_run_id.as_deref() {
                owned
                    .session_store
                    .write()
                    .clear_workflow_run_settling(session_id, workflow_run_id)?;
            }
            let mut dispatches = workflow_completion?;
            // Completion persisted while the settlement reservation still held
            // the terminal run. Archive only after its claim cleanup has ended.
            owned
                .persist_workflow_runtime_session(session_id, "workflow_provider_prompt_settled")?;
            if completion.released_claim {
                dispatches.extend(owned.workflow_retry_blocked_claims());
            }
            if let Some(started_next) = completion.completion.started_next.as_ref() {
                if crate::scheduler::runtime::is_workflow_prompt_attachment(
                    started_next.source_attachment_id(),
                ) {
                    // Mark the envelope as dispatched before the provider can observe
                    // the promoted prompt. This keeps workflow acknowledgement aligned
                    // with the normal completion-promotion path.
                    owned.workflow_mark_prompt_started(session_id, started_next)?;
                }
            }
            let reuses_provider_run = dispatches
                .local
                .iter()
                .any(|dispatch| dispatch.provider_run_id == provider_run_id);
            if completion.completion.started_next.is_none() && !reuses_provider_run {
                if let Ok(outcome) = owned
                    .provider_store
                    .terminate_run_provider_only(session_id, provider_run_id)
                {
                    let retired_provider_run_id = outcome.run().id().to_string();
                    let _ = owned.clear_active_provider_run_session_pointer(
                        session_id,
                        &retired_provider_run_id,
                    );
                    owned.provider_run_projection.update(outcome.into_run());
                    let (_, process_key) = self
                        .with_app_side_effect(|app| {
                            crate::app::ProviderLaunchProcessRuntime::new(app)
                                .remove_run(&retired_provider_run_id)
                        })
                        .await
                        .unwrap_or((false, None));
                    owned.remove_provider_process_tracking_for_run(
                        &retired_provider_run_id,
                        process_key,
                    );
                    owned
                        .connector_adapter_processes
                        .shutdown_run(&retired_provider_run_id)
                        .await;
                }
            }
            self.spawn_workflow_prompt_dispatches(dispatches);
        }
        if completion.released_claim && completion.completion.completed.workflow_run_id().is_none()
        {
            self.spawn_workflow_prompt_dispatches(owned.workflow_retry_blocked_claims());
        }
        self.inject_metaagent_turn_completion_event(session_id, &agent_id, &completion.completion)?;
        self.inject_orphaned_metaagent_task_event_after_turn(
            session_id,
            &agent_id,
            &completion.completion,
        )?;
        if let Some(dispatch) = completion.dispatch {
            if let Err(error) = self
                .enqueue_prompt_dispatch_after_liveness(&dispatch, owned)
                .await
            {
                let _ = self.fail_prompt_dispatch(dispatch, error).await;
            }
        } else if let Some(workflow_run_id) = settling_workflow_run_id.as_deref() {
            owned
                .session_store
                .write()
                .clear_workflow_run_settling(session_id, workflow_run_id)?;
        }
        if defer_queued_workflow_prompt {
            let session_id_for_queue = session_id.to_string();
            let agent_id_for_queue = agent_id.clone();
            match self
                .with_app_side_effect(move |app| {
                    app.advance_next_queued_prompt(&session_id_for_queue, &agent_id_for_queue)
                })
                .await
            {
                Ok(Some(_)) => {}
                Ok(None) => {}
                Err(error) => {
                    self.owned.record_notice(
                        session_id,
                        Some(provider_run_id),
                        self.owned
                            .attachment_store
                            .list_session_attachment_ids(session_id),
                        format!(
                            "Queued workflow prompt remained pending while preparing its provider context: {error}"
                        ),
                    );
                }
            }
        }
        if completion.completion.started_next.is_none() && !provider_run_was_running {
            let session_id_for_queue = session_id.to_string();
            let agent_id_for_queue = agent_id.clone();
            let agent_id_for_log = agent_id_for_queue.clone();
            match self
                .with_app_side_effect(move |app| {
                    app.advance_next_queued_prompt(&session_id_for_queue, &agent_id_for_queue)
                })
                .await
            {
                Ok(Some(_started_next)) => {
                    crate::logging::info_with_fields(
                        "daemon.provider",
                        "advanced queued prompt after terminal provider recovery",
                        serde_json::json!({
                            "session_id": session_id,
                            "agent_id": agent_id_for_log,
                            "ended_provider_run_id": provider_run_id,
                        }),
                    );
                }
                Ok(None) => {}
                Err(error) => {
                    self.owned.record_notice(
                        session_id,
                        Some(provider_run_id),
                        self.owned
                            .attachment_store
                            .list_session_attachment_ids(session_id),
                        format!(
                            "Queued prompt remained pending after terminal provider recovery: {error}"
                        ),
                    );
                }
            }
        }
        self.spawn_workflow_prompt_dispatches(
            owned.workflow_maybe_start_next_queued_prompt(session_id),
        );
        let state = self.clone();
        let session_id_for_continuation = session_id.to_string();
        let agent_id_for_continuation = agent_id.clone();
        let provider_run_id_for_continuation = provider_run_id.to_string();
        let continuation: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> =
            Box::pin(async move {
                if let Err(error) = state
                    .run_pending_mcp_continuation_after_completion(
                        &session_id_for_continuation,
                        &agent_id_for_continuation,
                    )
                    .await
                {
                    crate::logging::warn_with_fields(
                        "daemon.provider",
                        "pending MCP continuation failed",
                        serde_json::json!({
                            "session_id": session_id_for_continuation,
                            "agent_id": agent_id_for_continuation,
                            "error": error.to_string(),
                        }),
                    );
                }
                if let Err(error) = state
                    .owned
                    .park_detached_idle_provider_run(&session_id_for_continuation)
                {
                    crate::logging::warn_with_fields(
                        "daemon.session",
                        "failed to park detached provider after prompt completion",
                        serde_json::json!({
                            "session_id": session_id_for_continuation,
                            "provider_run_id": provider_run_id_for_continuation,
                            "error": error.to_string(),
                        }),
                    );
                }
            });
        tokio::spawn(continuation);
        Ok(crate::app::ProviderRunExitSessionSummary {
            had_active_prompt: true,
            cancelled_prompt: false,
            started_next_prompt: completion.completion.started_next.is_some(),
        })
    }
}
