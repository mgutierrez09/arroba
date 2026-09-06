//! Provider launch failure recovery.
//!
//! Owns notices, failed run cleanup, remote leased workflow failure forwarding, and automatic
//! substitute activation after a provider run fails before it becomes ready.

use super::*;

impl KernelRuntimeState {
    pub(crate) async fn fail_provider_launch(
        &self,
        started: &crate::app::StartedProviderLaunch,
        error: &DaemonError,
    ) {
        let _permit = self.provider_runtime_lanes.acquire(started.run.id()).await;
        self.fail_provider_launch_in_lane(started, error).await;
    }

    pub(super) async fn fail_provider_launch_in_lane(
        &self,
        started: &crate::app::StartedProviderLaunch,
        error: &DaemonError,
    ) {
        self.settle_provider_launch_failure_in_lane(started, error, None)
            .await;
    }

    pub(super) async fn retry_due_provider_launch_failures(&self, now_ms: u64) {
        for provider_run_id in self
            .owned
            .provider_launch_failure_retries
            .due_provider_run_ids(now_ms)
        {
            let _permit = self.provider_runtime_lanes.acquire(&provider_run_id).await;
            let Some(retry) = self
                .owned
                .provider_launch_failure_retries
                .take_due_for(&provider_run_id, now_ms)
            else {
                continue;
            };
            let started = retry.started.clone();
            let error = retry.error();
            self.settle_provider_launch_failure_in_lane(&started, &error, Some(retry))
                .await;
        }
    }

    async fn settle_provider_launch_failure_in_lane(
        &self,
        started: &crate::app::StartedProviderLaunch,
        error: &DaemonError,
        retry: Option<crate::app::ProviderLaunchFailureRetry>,
    ) {
        let mut advance_workflow_queue = false;
        {
            let owned = &self.owned;
            let diagnostic = format!(
                "Provider launch `{}` failed before it became ready: {}",
                started.run.id(),
                error
            );
            if retry.is_none() {
                crate::logging::error_with_fields(
                    "daemon.app",
                    "provider runtime initialization failed",
                    serde_json::json!({
                        "provider_run_id": started.run.id(),
                        "session_id": started.run.session_id(),
                        "error": error.to_string(),
                    }),
                );
                let recipients = owned
                    .attachment_store
                    .list_session_attachment_ids(started.run.session_id());
                owned.record_notice(
                    started.run.session_id(),
                    Some(started.run.id()),
                    recipients,
                    diagnostic.clone(),
                );
                if let Ok(run) = owned
                    .provider_store
                    .record_terminal_diagnostic(started.run.id(), diagnostic.clone())
                {
                    owned.provider_run_projection.update(run);
                }
            }
            match clear_failed_provider_resume_state_for_runtime(owned, started, error) {
                Ok(true) => {
                    owned
                        .provider_launch_failure_retries
                        .clear(started.run.id());
                }
                Ok(false) => {
                    crate::logging::warn_with_fields(
                        "durable_state.recovery",
                        "provider launch failure was superseded by a newer durable resume",
                        serde_json::json!({
                            "provider_run_id": started.run.id(),
                            "session_id": started.run.session_id(),
                        }),
                    );
                    owned
                        .provider_launch_failure_retries
                        .clear(started.run.id());
                }
                Err(clear_error) => {
                    crate::logging::error_with_fields(
                        "durable_state.recovery",
                        "failed to durably clear invalid provider resume",
                        serde_json::json!({
                            "provider_run_id": started.run.id(),
                            "session_id": started.run.session_id(),
                            "error": clear_error.to_string(),
                        }),
                    );
                    if crate::durable_state::is_retryable_durable_write_error(&clear_error) {
                        let retry_attempt = retry.as_ref().map(|retry| retry.attempt());
                        let outcome = if let Some(retry) = retry {
                            owned
                                .provider_launch_failure_retries
                                .reschedule(retry, crate::session::unix_epoch_ms())
                        } else if owned.provider_launch_failure_retries.schedule_initial(
                            started,
                            error,
                            crate::session::unix_epoch_ms(),
                        ) {
                            crate::app::ProviderLaunchFailureRetryScheduleOutcome::Scheduled
                        } else {
                            crate::app::ProviderLaunchFailureRetryScheduleOutcome::AlreadyScheduled
                        };
                        match outcome {
                            crate::app::ProviderLaunchFailureRetryScheduleOutcome::Scheduled => {
                                owned.runtime_projection_changes.record_change();
                            }
                            crate::app::ProviderLaunchFailureRetryScheduleOutcome::AlreadyScheduled => {}
                            crate::app::ProviderLaunchFailureRetryScheduleOutcome::Exhausted => {
                                let diagnostic = format!(
                                    "Provider launch `{}` cleanup exhausted {} durable-write attempts; operator intervention is required",
                                    started.run.id(),
                                    retry_attempt.unwrap_or_default(),
                                );
                                crate::logging::error_with_fields(
                                    "durable_state.recovery",
                                    "provider launch failure cleanup retry budget exhausted",
                                    serde_json::json!({
                                        "provider_run_id": started.run.id(),
                                        "session_id": started.run.session_id(),
                                        "attempts": retry_attempt,
                                    }),
                                );
                                let recipients = owned
                                    .attachment_store
                                    .list_session_attachment_ids(started.run.session_id());
                                owned.record_notice(
                                    started.run.session_id(),
                                    Some(started.run.id()),
                                    recipients,
                                    diagnostic.clone(),
                                );
                                if let Ok(run) = owned
                                    .provider_store
                                    .record_terminal_diagnostic(started.run.id(), diagnostic)
                                {
                                    owned.provider_run_projection.update(run);
                                }
                            }
                        }
                    }
                    return;
                }
            }
            let durable_active_prompt = started.run.agent_instance_id().and_then(|agent_id| {
                let session = owned
                    .session_store
                    .get_session(started.run.session_id())
                    .ok()?;
                owned
                    .prompt_state_owner
                    .active_prompt_for_agent(&session, agent_id)
                    .filter(|prompt| prompt.durable_delivery_phase().is_some())
            });
            let leased_context = self
                .with_app_side_effect(|app| {
                    crate::app::RemoteLeaseRuntime::new(app)
                        .leased_workflow_turn_context_for_provider_run(started.run.id())
                })
                .await;
            if leased_context.is_some() {
                if let Some(agent_id) = started.run.agent_instance_id() {
                    if let Err(error) = self
                        .settle_leased_workflow_provider_failure(
                            started.run.session_id(),
                            agent_id,
                            started.run.id(),
                        )
                        .await
                    {
                        crate::logging::warn_with_fields(
                            "daemon.provider",
                            "leased provider launch failure settlement failed",
                            serde_json::json!({
                                "provider_run_id": started.run.id(),
                                "error": error.to_string(),
                            }),
                        );
                    }
                }
            } else if durable_active_prompt.is_some() {
                crate::logging::warn_with_fields(
                    "durable_state.recovery",
                    "preserving durable prompt after provider launch failure",
                    serde_json::json!({
                        "session_id": started.run.session_id(),
                        "agent_id": started.run.agent_instance_id(),
                        "provider_run_id": started.run.id(),
                    }),
                );
            } else if let Some(agent_id) = started.run.agent_instance_id() {
                if let Ok(session) = owned.session_store.get_session(started.run.session_id()) {
                    if let Some(active_prompt) = owned
                        .prompt_state_owner
                        .active_prompt_for_agent(&session, agent_id)
                    {
                        let _ = self.inject_metaagent_turn_failure_event(
                            started.run.session_id(),
                            agent_id,
                            &active_prompt,
                            Some(started.run.id()),
                            &diagnostic,
                        );
                        if active_prompt.workflow_run_id().is_some() {
                            if owned
                                .workflow_fail_provider_prompt_without_queue_advance(
                                    started.run.session_id(),
                                    &active_prompt,
                                    Some(started.run.id()),
                                    &diagnostic,
                                )
                                .is_ok()
                            {
                                advance_workflow_queue = true;
                            }
                        }
                        let _ = owned.complete_local_prompt_without_advance(
                            started.run.session_id(),
                            agent_id,
                            Some(started.run.id()),
                        );
                    } else if let Ok(queued_workflow_prompts) = owned
                        .take_queued_workflow_prompts_for_agent(started.run.session_id(), agent_id)
                    {
                        for prompt in queued_workflow_prompts {
                            let _ = self.inject_metaagent_turn_failure_event(
                                started.run.session_id(),
                                agent_id,
                                &prompt,
                                Some(started.run.id()),
                                &diagnostic,
                            );
                            if owned
                                .workflow_fail_provider_prompt_without_queue_advance(
                                    started.run.session_id(),
                                    &prompt,
                                    Some(started.run.id()),
                                    &diagnostic,
                                )
                                .is_ok()
                            {
                                advance_workflow_queue = true;
                            }
                        }
                    }
                }
            }
            let (_, process_key) = self
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(started.run.id())
                })
                .await
                .unwrap_or((false, None));
            owned.remove_provider_process_tracking_for_run(started.run.id(), process_key);
            owned.provider_store.clear_runtime(started.run.id());
            if let Ok(outcome) = owned
                .provider_store
                .terminate_run_provider_only(started.run.session_id(), started.run.id())
            {
                let _ = owned.clear_active_provider_run_session_pointer(
                    started.run.session_id(),
                    outcome.run().id(),
                );
                owned.provider_run_projection.update(outcome.into_run());
            }
            if let Some(previous_active_run_id) = started.previous_active_run_id.as_deref() {
                let _ = owned.resume_provider_run_for_session(
                    started.run.session_id(),
                    previous_active_run_id,
                );
            }
            if advance_workflow_queue {
                self.spawn_workflow_prompt_dispatches(
                    owned.workflow_maybe_start_next_queued_prompt(started.run.session_id()),
                );
            }
            let _ = owned.session_snapshot(started.run.session_id());
        }
        if let (Some(agent_id), Some(reason)) = (
            started.run.agent_instance_id(),
            crate::provider::classify_provider_substitutable_failure_text(
                started.run.adapter_key(),
                &error.to_string(),
            ),
        ) {
            if let Err(substitute_error) = self
                .activate_next_agent_substitute_after_failure(
                    started.run.session_id(),
                    agent_id,
                    &reason,
                )
                .await
            {
                crate::logging::warn_with_fields(
                    "daemon.provider",
                    "automatic substitute activation after launch failure failed",
                    serde_json::json!({
                        "session_id": started.run.session_id(),
                        "agent_id": agent_id,
                        "provider_run_id": started.run.id(),
                        "error": substitute_error.to_string(),
                    }),
                );
            }
        }
    }
}

fn clear_failed_provider_resume_state_for_runtime(
    owned: &KernelRuntimeOwnedState,
    started: &crate::app::StartedProviderLaunch,
    error: &DaemonError,
) -> Result<bool, DaemonError> {
    if crate::app::failed_provider_resume_state_replacement(&started.run, error).is_none() {
        return Ok(true);
    }
    let Some(agent_id) = started.run.agent_instance_id() else {
        return Ok(true);
    };
    let provider = started.run.adapter_key();
    let Some(stale_provider_session_id) = started
        .run
        .resume_state()
        .provider_session_id(provider)
        .map(str::to_string)
    else {
        return Ok(true);
    };
    match owned
        .agent_store
        .clear_provider_resume_state_durably_if_matches(
            &owned.durable_state_store,
            agent_id,
            provider,
            &stale_provider_session_id,
            started.run.id(),
            "failed_provider_resume_state_cleared",
        )? {
        crate::agent::ProviderResumeClearOutcome::Cleared => {}
        crate::agent::ProviderResumeClearOutcome::AlreadyAbsent => return Ok(true),
        crate::agent::ProviderResumeClearOutcome::Superseded { .. } => return Ok(false),
    }
    owned.record_notice(
        started.run.session_id(),
        Some(started.run.id()),
        owned
            .attachment_store
            .list_session_attachment_ids(started.run.session_id()),
        crate::provider::provider_resume_failure_notice(provider, &stale_provider_session_id)
            .unwrap_or_else(|| {
                format!(
                    "Provider session `{stale_provider_session_id}` is no longer available. Chariox cleared it from the agent profile so the next prompt can start a new durable provider session."
                )
            }),
    );
    Ok(true)
}
