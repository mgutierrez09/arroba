//! Startup reconciliation for durable prompt work left active by a previous kernel process.

use super::*;

/// Prefix stamped onto the `source_attachment_id` of every kernel-internal
/// restart-recovery dispatch. It identifies an envelope that carries provider
/// resume text (not user input) so downstream fanout can suppress it.
pub(crate) const KERNEL_RECOVERY_ATTACHMENT_PREFIX: &str = "kernel-recovery:";

/// Whether an attachment id belongs to a kernel-internal restart-recovery
/// dispatch. Kept as a shared helper so every fanout/persistence boundary
/// checks the same marker.
pub(crate) fn is_internal_recovery_prompt_attachment(attachment_id: &str) -> bool {
    attachment_id.starts_with(KERNEL_RECOVERY_ATTACHMENT_PREFIX)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DurableRestartRecoverySummary {
    pub(crate) cancelled_local_prompts_finalized: usize,
    pub(crate) accepted_local_redispatched: usize,
    pub(crate) uncertain_original_redispatched: usize,
    pub(crate) provider_continuations_dispatched: usize,
    pub(crate) remote_reconciliations_started: usize,
    pub(crate) uncertain_local_prompts_preserved: usize,
    pub(crate) transcript_recovery_pending: usize,
    pub(crate) queued_local_prompts_started: usize,
    pub(crate) orphaned_workflow_prompts_finalized: usize,
    pub(crate) failed_reconciliations: usize,
}

pub(super) enum UncertainLocalRecoveryOutcome {
    OriginalRedispatched,
    ContinuationDispatched,
    Preserved,
    TranscriptPending,
}

pub(super) enum CancelledLocalPromptRestartOutcome {
    Finalized,
    Recovered(UncertainLocalRecoveryOutcome),
}

type DurableRestartRecoveryTarget = (String, String, String);
type DurableRestartDispatchTarget = (String, String, String, String);
const TRANSCRIPT_OBSERVATION_ATTEMPTS_BEFORE_REDISPATCH: u32 = 9;

pub(crate) struct DurableRestartRecoveryTask(tokio::task::JoinHandle<()>);

impl Drop for DurableRestartRecoveryTask {
    fn drop(&mut self) {
        // In particular, a publication which never activates must not retain
        // its runtime and exclusive state lease after the listener stops.
        self.0.abort();
    }
}

fn should_rearm_unobserved_dispatches(
    attempt: u32,
    transcript_recovery_pending: usize,
    pending_dispatch_targets: usize,
) -> bool {
    transcript_recovery_pending > 0
        && pending_dispatch_targets > 0
        && attempt >= TRANSCRIPT_OBSERVATION_ATTEMPTS_BEFORE_REDISPATCH
}

impl KernelRuntimeState {
    pub(crate) fn spawn_durable_restart_recovery(&self) -> DurableRestartRecoveryTask {
        // Recovery belongs only to work that survived this kernel restart.
        // Keep that identity set fixed across the retry window so prompts
        // accepted after startup can never be mistaken for orphaned work.
        let recovery_targets = self.durable_restart_recovery_targets();
        let dispatch_targets = self.durable_restart_dispatch_targets(&recovery_targets);
        let queued_recovery_targets = self.durable_restart_queued_recovery_targets();
        crate::logging::info_with_fields(
            "durable_state.recovery",
            "captured durable restart recovery targets",
            serde_json::json!({
                "active_prompt_targets": recovery_targets.len(),
                "unobserved_dispatch_targets": dispatch_targets.len(),
                "queued_publication_targets": queued_recovery_targets.len(),
            }),
        );
        let state = self.clone();
        DurableRestartRecoveryTask(tokio::spawn(async move {
            state.owned.publication_activation.wait().await;
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            let mut attempt = 0_u32;
            let mut pending_dispatch_targets = dispatch_targets;
            let summary = loop {
                let mut summary = state
                    .recover_durable_runtime_after_restart_targets(
                        &recovery_targets,
                        &queued_recovery_targets,
                    )
                    .await;
                if should_rearm_unobserved_dispatches(
                    attempt,
                    summary.transcript_recovery_pending,
                    pending_dispatch_targets.len(),
                ) {
                    match state
                        .rearm_unobserved_restart_recovery_dispatches(&mut pending_dispatch_targets)
                    {
                        Ok(rearmed) if rearmed > 0 => {
                            crate::logging::warn_with_fields(
                                "durable_state.recovery",
                                "rearmed unobserved restart recovery dispatches",
                                serde_json::json!({
                                    "rearmed_prompt_count": rearmed,
                                    "observation_attempts": attempt.saturating_add(1),
                                }),
                            );
                        }
                        Ok(_) => {}
                        Err(error) => {
                            summary.failed_reconciliations =
                                summary.failed_reconciliations.saturating_add(1);
                            crate::logging::error_with_fields(
                                "durable_state.recovery",
                                "failed to durably rearm unobserved restart recovery dispatch",
                                serde_json::json!({
                                    "error": error.to_string(),
                                    "observation_attempts": attempt.saturating_add(1),
                                }),
                            );
                        }
                    }
                }
                if (summary.transcript_recovery_pending == 0 && summary.failed_reconciliations == 0)
                    || attempt >= 299
                {
                    break summary;
                }
                attempt = attempt.saturating_add(1);
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            };
            crate::logging::info_with_fields(
                "durable_state.recovery",
                "reconciled durable runtime work after kernel restart",
                serde_json::json!({
                    "cancelled_local_prompts_finalized": summary.cancelled_local_prompts_finalized,
                    "accepted_local_redispatched": summary.accepted_local_redispatched,
                    "uncertain_original_redispatched": summary.uncertain_original_redispatched,
                    "provider_continuations_dispatched": summary.provider_continuations_dispatched,
                    "remote_reconciliations_started": summary.remote_reconciliations_started,
                    "uncertain_local_prompts_preserved": summary.uncertain_local_prompts_preserved,
                    "transcript_recovery_pending": summary.transcript_recovery_pending,
                    "queued_local_prompts_started": summary.queued_local_prompts_started,
                    "orphaned_workflow_prompts_finalized": summary.orphaned_workflow_prompts_finalized,
                    "failed_reconciliations": summary.failed_reconciliations,
                }),
            );
        }))
    }

    fn rearm_unobserved_restart_recovery_dispatches(
        &self,
        dispatch_targets: &mut BTreeSet<DurableRestartDispatchTarget>,
    ) -> Result<usize, DaemonError> {
        let mut rearmed = 0;
        let candidates = dispatch_targets.iter().cloned().collect::<Vec<_>>();
        for (session_id, agent_id, prompt_id, operation_id) in candidates {
            let transition = self.owned.compare_and_mark_active_prompt_recovery_phase(
                &session_id,
                &agent_id,
                &prompt_id,
                &operation_id,
                crate::session::DurablePromptDeliveryPhase::Dispatching,
                crate::session::DurablePromptDeliveryPhase::Accepted,
            )?;
            dispatch_targets.remove(&(session_id, agent_id, prompt_id, operation_id));
            if transition.is_some() {
                rearmed += 1;
            }
        }
        Ok(rearmed)
    }

    fn durable_restart_dispatch_targets(
        &self,
        recovery_targets: &BTreeSet<DurableRestartRecoveryTarget>,
    ) -> BTreeSet<DurableRestartDispatchTarget> {
        recovery_targets
            .iter()
            .filter_map(|(session_id, agent_id, prompt_id)| {
                let session = self.owned.session_store.get_session(session_id).ok()?;
                let prompt = self
                    .owned
                    .prompt_state_owner
                    .active_prompt_for_agent(&session, agent_id)?;
                if prompt.id() != prompt_id
                    || prompt.durable_recovery_phase()
                        != Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
                {
                    return None;
                }
                Some((
                    session_id.clone(),
                    agent_id.clone(),
                    prompt_id.clone(),
                    prompt.durable_recovery_operation_id()?.to_string(),
                ))
            })
            .collect()
    }

    pub(crate) async fn recover_durable_runtime_after_restart(
        &self,
    ) -> DurableRestartRecoverySummary {
        let recovery_targets = self.durable_restart_recovery_targets();
        let queued_recovery_targets = self.durable_restart_queued_recovery_targets();
        self.recover_durable_runtime_after_restart_targets(
            &recovery_targets,
            &queued_recovery_targets,
        )
        .await
    }

    fn durable_restart_recovery_targets(&self) -> BTreeSet<DurableRestartRecoveryTarget> {
        let mut targets = BTreeSet::new();
        for session in self.owned.session_store.list_all_sessions() {
            for (agent_id, prompt_state) in session.prompt_states() {
                let Some(prompt) = prompt_state.active_prompt() else {
                    continue;
                };
                let Ok(agent) = self.owned.agent_store.get_agent(agent_id) else {
                    continue;
                };
                let local_workspace_available = agent.remote_execution().is_some()
                    || std::path::Path::new(
                        agent.worktree_id().unwrap_or_else(|| session.worktree_id()),
                    )
                    .exists();
                if !local_workspace_available {
                    continue;
                }
                targets.insert((
                    session.id().to_string(),
                    agent_id.to_string(),
                    prompt.id().to_string(),
                ));
            }
        }
        targets
    }

    fn durable_restart_queued_recovery_targets(&self) -> BTreeSet<DurableRestartRecoveryTarget> {
        self.owned
            .session_store
            .list_all_sessions()
            .into_iter()
            .flat_map(|session| {
                session
                    .prompt_states()
                    .iter()
                    .filter(|(_, prompt_state)| prompt_state.active_prompt().is_none())
                    .filter_map(|(agent_id, prompt_state)| {
                        let prompt = prompt_state.queued_prompts().front()?;
                        recoverable_queued_publication_prompt(&session, prompt).then(|| {
                            (
                                session.id().to_string(),
                                agent_id.to_string(),
                                prompt.id().to_string(),
                            )
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    async fn recover_durable_runtime_after_restart_targets(
        &self,
        recovery_targets: &BTreeSet<DurableRestartRecoveryTarget>,
        queued_recovery_targets: &BTreeSet<DurableRestartRecoveryTarget>,
    ) -> DurableRestartRecoverySummary {
        let mut summary = DurableRestartRecoverySummary::default();
        if !self.owned.publication_activation.is_active() {
            return summary;
        }
        let sessions = self.owned.session_store.list_all_sessions();
        // Publication work is autonomous and already durably admitted. Resume it before
        // transcript reconciliation for unrelated interactive sessions, which may require slow
        // provider scans or retries.
        for (session_id, agent_id, prompt_id) in queued_recovery_targets {
            match self
                .recover_queued_local_prompt_after_restart(session_id, agent_id, prompt_id, true)
            {
                Ok(Some(dispatches)) => {
                    summary.queued_local_prompts_started += 1;
                    self.spawn_workflow_prompt_dispatches(dispatches);
                }
                Ok(None) => {}
                Err(error) => {
                    summary.failed_reconciliations += 1;
                    log_restart_recovery_failure(session_id, agent_id, prompt_id, &error);
                }
            }
        }
        for session in &sessions {
            for (agent_id, prompt_state) in session.prompt_states() {
                let Some(prompt) = prompt_state.active_prompt().cloned() else {
                    continue;
                };
                if !recovery_targets.contains(&(
                    session.id().to_string(),
                    agent_id.to_string(),
                    prompt.id().to_string(),
                )) {
                    continue;
                }
                let delivery_phase = prompt.durable_delivery_phase();
                let agent = match self.owned.agent_store.get_agent(agent_id) {
                    Ok(agent) => agent,
                    Err(error) => {
                        summary.failed_reconciliations += 1;
                        log_restart_recovery_failure(session.id(), agent_id, prompt.id(), &error);
                        continue;
                    }
                };
                if prompt.workflow_run_id().is_some()
                    && self
                        .owned
                        .session_store
                        .read()
                        .resolve_workflow_run_ref(
                            session.id(),
                            prompt.workflow_run_id().expect("checked above"),
                        )
                        .is_err()
                {
                    match self
                        .finalize_orphaned_workflow_prompt_after_restart(
                            session.id(),
                            agent_id,
                            &prompt,
                        )
                        .await
                    {
                        Ok(()) => summary.orphaned_workflow_prompts_finalized += 1,
                        Err(error) => {
                            summary.failed_reconciliations += 1;
                            log_restart_recovery_failure(
                                session.id(),
                                agent_id,
                                prompt.id(),
                                &error,
                            );
                        }
                    }
                    continue;
                }
                if agent.remote_execution().is_some() {
                    match self
                        .recover_remote_prompt_after_kernel_restart(
                            session.id(),
                            agent_id,
                            delivery_phase,
                            prompt.durable_delivery_provider_run_id(),
                        )
                        .await
                    {
                        Ok(true) => summary.remote_reconciliations_started += 1,
                        Ok(false) => summary.failed_reconciliations += 1,
                        Err(error) => {
                            summary.failed_reconciliations += 1;
                            log_restart_recovery_failure(
                                session.id(),
                                agent_id,
                                prompt.id(),
                                &error,
                            );
                        }
                    }
                    continue;
                }
                if prompt.status() == crate::session::PromptStatus::Cancelling {
                    match self
                        .finalize_cancelled_local_prompt_after_restart(
                            session.id(),
                            agent_id,
                            &prompt,
                        )
                        .await
                    {
                        Ok(CancelledLocalPromptRestartOutcome::Finalized) => {
                            summary.cancelled_local_prompts_finalized += 1;
                        }
                        Ok(CancelledLocalPromptRestartOutcome::Recovered(
                            UncertainLocalRecoveryOutcome::OriginalRedispatched,
                        )) => {
                            summary.uncertain_original_redispatched += 1;
                        }
                        Ok(CancelledLocalPromptRestartOutcome::Recovered(
                            UncertainLocalRecoveryOutcome::ContinuationDispatched,
                        )) => {
                            summary.provider_continuations_dispatched += 1;
                        }
                        Ok(CancelledLocalPromptRestartOutcome::Recovered(
                            UncertainLocalRecoveryOutcome::Preserved,
                        )) => {
                            summary.uncertain_local_prompts_preserved += 1;
                        }
                        Ok(CancelledLocalPromptRestartOutcome::Recovered(
                            UncertainLocalRecoveryOutcome::TranscriptPending,
                        )) => {
                            summary.transcript_recovery_pending += 1;
                        }
                        Err(error) => {
                            summary.failed_reconciliations += 1;
                            log_restart_recovery_failure(
                                session.id(),
                                agent_id,
                                prompt.id(),
                                &error,
                            );
                        }
                    }
                    continue;
                }
                match delivery_phase {
                    Some(crate::session::DurablePromptDeliveryPhase::Accepted) => match self
                        .redispatch_local_prompt(session.id(), agent_id, &prompt)
                        .await
                    {
                        Ok(()) => summary.accepted_local_redispatched += 1,
                        Err(error) => {
                            summary.failed_reconciliations += 1;
                            log_restart_recovery_failure(
                                session.id(),
                                agent_id,
                                prompt.id(),
                                &error,
                            );
                        }
                    },
                    Some(
                        crate::session::DurablePromptDeliveryPhase::Dispatching
                        | crate::session::DurablePromptDeliveryPhase::Delivered,
                    ) => match self
                        .reconcile_uncertain_local_prompt(
                            session.id(),
                            &agent,
                            &prompt,
                            delivery_phase.expect("matched delivery phase"),
                        )
                        .await
                    {
                        Ok(UncertainLocalRecoveryOutcome::OriginalRedispatched) => {
                            summary.uncertain_original_redispatched += 1;
                        }
                        Ok(UncertainLocalRecoveryOutcome::ContinuationDispatched) => {
                            summary.provider_continuations_dispatched += 1;
                        }
                        Ok(UncertainLocalRecoveryOutcome::Preserved) => {
                            summary.uncertain_local_prompts_preserved += 1;
                        }
                        Ok(UncertainLocalRecoveryOutcome::TranscriptPending) => {
                            summary.transcript_recovery_pending += 1;
                        }
                        Err(error) => {
                            summary.failed_reconciliations += 1;
                            log_restart_recovery_failure(
                                session.id(),
                                agent_id,
                                prompt.id(),
                                &error,
                            );
                        }
                    },
                    None => summary.uncertain_local_prompts_preserved += 1,
                }
            }
        }
        for session in sessions {
            self.spawn_workflow_prompt_dispatches(
                self.owned
                    .workflow_maybe_start_next_queued_prompt(session.id()),
            );
        }
        // Workspace claims are process-local, while blocked workflow nodes are durable.  After
        // a restart the old claim cannot still be held, so retry those nodes explicitly instead
        // of leaving event-delivery runs parked in `BlockedOnWorkspaceClaim` forever.
        self.spawn_workflow_prompt_dispatches(self.owned.workflow_retry_blocked_claims());
        summary
    }

    async fn finalize_orphaned_workflow_prompt_after_restart(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<(), DaemonError> {
        let session_id_owned = session_id.to_string();
        let agent_id_owned = agent_id.to_string();
        let prompt_id = prompt.id().to_string();
        self.with_app_side_effect(move |app| {
            let cancelled =
                app.prompt_owner_cancel_active_prompt_only(&session_id_owned, &agent_id_owned)?;
            crate::logging::warn_with_fields(
                "durable_state.recovery",
                "finalized workflow prompt whose run was not durable",
                serde_json::json!({
                    "session_id": session_id_owned,
                    "agent_id": agent_id_owned,
                    "prompt_id": prompt_id,
                    "cancelled_prompt_id": cancelled.id(),
                }),
            );
            Ok(())
        })
        .await?;

        // The workflow run already exists; only its provider prompt was orphaned.
        // Recover the next provider prompt directly instead of asking the workflow
        // queue to create another run.  The latter leaves a Ready node stranded
        // because the invocation was already claimed before the restart.
        let queued_prompt_id = self
            .owned
            .session_store
            .get_session(session_id)
            .ok()
            .and_then(|session| {
                self.owned
                    .prompt_state_owner
                    .state_parts(&session, agent_id)
                    .1
                    .front()
                    .map(|prompt| prompt.id().to_string())
            });
        if let Some(queued_prompt_id) = queued_prompt_id {
            if let Some(dispatches) = self.recover_queued_local_prompt_after_restart(
                session_id,
                agent_id,
                &queued_prompt_id,
                false,
            )? {
                self.spawn_workflow_prompt_dispatches(dispatches);
            }
        }
        Ok(())
    }

    #[allow(clippy::result_large_err)]
    fn recover_queued_local_prompt_after_restart(
        &self,
        session_id: &str,
        agent_id: &str,
        expected_prompt_id: &str,
        require_publication: bool,
    ) -> Result<Option<WorkflowPromptDispatches>, DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        if self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .is_some()
        {
            return Ok(None);
        }
        let Some(prompt) = self
            .owned
            .prompt_state_owner
            .state_parts(&session, agent_id)
            .1
            .front()
            .cloned()
        else {
            return Ok(None);
        };
        if prompt.id() != expected_prompt_id {
            return Ok(None);
        }
        if require_publication && !recoverable_queued_publication_prompt(&session, &prompt) {
            return Ok(None);
        }
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if agent.remote_execution().is_some() {
            return Ok(None);
        }

        let (event_reply_enabled, event_context_enabled, event_actions_enabled) = self
            .owned
            .workflow_event_capabilities_for_prompt(session_id, &prompt)?;
        let fresh_context = self
            .owned
            .workflow_prompt_requires_fresh_provider_context(session_id, agent_id, &prompt)?;
        let (provider_run_id, retired_provider_run_id) = self.owned.workflow_ensure_provider_run(
            session_id,
            agent_id,
            event_reply_enabled,
            event_context_enabled,
            event_actions_enabled,
            fresh_context,
            prompt.workflow_node_run_id(),
        )?;
        let provider_run = self
            .owned
            .ensure_provider_run_in_session(session_id, &provider_run_id)?;
        let mut dispatches = WorkflowPromptDispatches::default();
        if let Some(retired_provider_run_id) = retired_provider_run_id {
            dispatches
                .retire_provider_before_launch(provider_run_id.clone(), retired_provider_run_id);
        }
        match provider_run.state() {
            crate::provider::ProviderRunState::Starting => {
                dispatches.starting_provider_runs.push(provider_run_id);
            }
            crate::provider::ProviderRunState::Running => {
                if let Some(dispatch) = self.owned.advance_next_queued_prompt_dispatch(
                    session_id,
                    agent_id,
                    &provider_run_id,
                )? {
                    dispatches.local.push(dispatch);
                } else {
                    return Ok(None);
                }
            }
            crate::provider::ProviderRunState::Parked
            | crate::provider::ProviderRunState::Ended => {
                return Err(DaemonError::InvalidProviderRunState {
                    provider_run_id,
                    state: provider_run.state(),
                    operation: "recover queued prompt after restart",
                });
            }
        }
        Ok(Some(dispatches))
    }

    pub(super) async fn finalize_cancelled_local_prompt_after_restart(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<CancelledLocalPromptRestartOutcome, DaemonError> {
        let failed_delivery = prompt
            .durable_delivery_provider_run_id()
            .zip(prompt.durable_delivery_provider_session_id())
            .filter(|_| prompt.durable_delivery_failure_pending())
            .map(|(run_id, session_id)| (run_id.to_string(), session_id.to_string()));
        if let Some((failed_provider_run_id, failed_provider_session_id)) = failed_delivery.as_ref()
        {
            let agent = self.owned.agent_store.get_agent(agent_id)?;
            let adapter_key = crate::provider::adapter_key_for_provider(agent.provider());
            match self.clear_provider_resume_state_for_identity(
                session_id,
                agent_id,
                adapter_key,
                failed_provider_session_id,
                failed_provider_run_id,
                "unresponsive_provider_resume_state_cleared_after_restart",
            )? {
                crate::agent::ProviderResumeClearOutcome::Cleared
                | crate::agent::ProviderResumeClearOutcome::AlreadyAbsent => {}
                crate::agent::ProviderResumeClearOutcome::Superseded {
                    current_provider_session_id,
                } => {
                    let restored = self
                        .owned
                        .restore_active_prompt_after_resume_superseded(
                            session_id,
                            agent_id,
                            prompt.id(),
                            failed_provider_run_id,
                            failed_provider_session_id,
                            &current_provider_session_id,
                        )?
                        .ok_or_else(|| DaemonError::LocalTransport {
                            operation: "recover failed prompt delivery",
                            message: format!(
                                "prompt `{}` changed before superseding provider resume `{current_provider_session_id}` could be reconciled",
                                prompt.id()
                            ),
                        })?;
                    let outcome = self
                        .reconcile_uncertain_local_prompt(
                            session_id,
                            &agent,
                            &restored,
                            crate::session::DurablePromptDeliveryPhase::Dispatching,
                        )
                        .await?;
                    return Ok(CancelledLocalPromptRestartOutcome::Recovered(outcome));
                }
            }
            self.retire_owned_provider_run_after_terminal_failure(
                session_id,
                failed_provider_run_id,
            )
            .await;
        }
        let provider_run_id = if failed_delivery.is_some() {
            None
        } else {
            self.owned
                .provider_store
                .get_run_for_agent(session_id, agent_id)
                .map(|run| run.id().to_string())
        };
        let cancellation = self
            .owned
            .finalize_local_prompt_cancellation_with_queued_advance(
                session_id,
                agent_id,
                provider_run_id.as_deref(),
            )?;
        self.owned
            .workflow_cancel_prompt(session_id, &cancellation.cancellation.prompt)?;
        if cancellation.released_claim {
            self.spawn_workflow_prompt_dispatches(self.owned.workflow_retry_blocked_claims());
        }
        if let Some(dispatch) = cancellation.dispatch {
            if let Err(error) = self
                .enqueue_prompt_dispatch_after_liveness(&dispatch, &self.owned)
                .await
            {
                let _ = self.fail_prompt_dispatch(dispatch, error).await;
            }
        }
        if failed_delivery.is_some()
            && self
                .owned
                .prompt_state_owner
                .peek_next_queued_prompt(
                    &self.owned.session_store.get_session(session_id)?,
                    agent_id,
                )
                .is_some()
        {
            let session_id_owned = session_id.to_string();
            let agent_id_owned = agent_id.to_string();
            let replacement_provider_run_id = self
                .with_app_side_effect(move |app| {
                    app.ensure_prompt_provider_run_for_agent(&session_id_owned, &agent_id_owned)
                })
                .await?;
            if let Some(dispatch) = self.owned.advance_next_queued_prompt_dispatch(
                session_id,
                agent_id,
                &replacement_provider_run_id,
            )? {
                self.spawn_prompt_dispatch(dispatch, self.provider_runtime_lanes.clone());
            }
        }
        Ok(CancelledLocalPromptRestartOutcome::Finalized)
    }

    async fn redispatch_local_prompt(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<(), DaemonError> {
        let session_id_owned = session_id.to_string();
        let agent_id_owned = agent_id.to_string();
        let provider_run_id = self
            .with_app_side_effect(move |app| {
                app.ensure_prompt_provider_run_for_agent(&session_id_owned, &agent_id_owned)
            })
            .await?;
        let dispatch = crate::app::KernelPromptDispatch {
            session_id: session_id.to_string(),
            provider_run_id,
            agent_id: agent_id.to_string(),
            prompt_id: prompt.id().to_string(),
            target_active_prompt_id: None,
            source_attachment_id: prompt.source_attachment_id().to_string(),
            prompt: prompt.prompt().to_string(),
            hidden_system_context: prompt.hidden_system_context().to_string(),
            attachments: prompt.attachments().to_vec(),
            prompt_origin: prompt.prompt_origin(),
            external_provider: prompt.external_provider().map(str::to_string),
            external_provider_session_id: prompt.external_provider_session_id().map(str::to_string),
            external_provider_turn_id: prompt.external_provider_turn_id().map(str::to_string),
            steering: false,
        };
        self.enqueue_prompt_dispatch(&dispatch).await
    }

    async fn reconcile_uncertain_local_prompt(
        &self,
        session_id: &str,
        agent: &crate::agent::AgentInstance,
        prompt: &crate::session::PromptQueueItem,
        delivery_phase: crate::session::DurablePromptDeliveryPhase,
    ) -> Result<UncertainLocalRecoveryOutcome, DaemonError> {
        let mut prompt = prompt.clone();
        let adapter_key = crate::provider::adapter_key_for_provider(agent.provider());
        if adapter_key == "dev-stub" {
            self.redispatch_local_prompt(session_id, agent.id(), &prompt)
                .await?;
            return Ok(UncertainLocalRecoveryOutcome::OriginalRedispatched);
        }
        if !crate::provider::ExternalProviderObservationPolicy::for_provider(adapter_key)
            .is_configured()
        {
            return Ok(UncertainLocalRecoveryOutcome::Preserved);
        }
        let existing_recovery_operation =
            prompt.durable_recovery_operation_id().map(str::to_string);
        let session = self.owned.session_store.get_session(session_id).ok();
        let workflow_rendered_prompt = session
            .as_ref()
            .and_then(|session| workflow_turn_rendered_prompt(session, &prompt));
        let recovery_material =
            restart_recovery_prompt_material(&prompt, workflow_rendered_prompt.as_deref());
        let prompt_text = recovery_material.transcript_match_text.clone();
        let worktree_path = agent.worktree_id().map(str::to_string).or_else(|| {
            session
                .as_ref()
                .map(|session| session.worktree_id().to_string())
        });
        let mut matched = None;
        for scan_attempt in 0..5 {
            let adapter_key_owned = adapter_key.to_string();
            let prompt_text = prompt_text.clone();
            let worktree_path = worktree_path.clone();
            let recovery_operation_for_scan = existing_recovery_operation.clone();
            matched = tokio::task::spawn_blocking(move || {
                crate::app::find_external_provider_prompt_recovery_match(
                    &adapter_key_owned,
                    &prompt_text,
                    worktree_path.as_deref(),
                    recovery_operation_for_scan.as_deref(),
                )
            })
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "scan provider transcript for restart recovery",
                message: error.to_string(),
            })?;
            if matched.is_some() || scan_attempt == 4 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        let prompt_provider_session_id = prompt
            .durable_delivery_provider_session_id()
            .map(str::to_string);
        let agent_provider_session_id = agent
            .provider_resume_state()
            .provider_session_id(adapter_key)
            .map(str::to_string);
        let provider_session_id = preferred_restart_recovery_provider_session_id(
            matched
                .as_ref()
                .map(|matched| matched.provider_session_id.as_str()),
            prompt_provider_session_id.as_deref(),
            agent_provider_session_id.as_deref(),
            prompt.workflow_run_id().is_some(),
        );
        let Some(provider_session_id) = provider_session_id else {
            // Dispatching means the provider call may already have succeeded.
            // Without a durable provider identity or transcript observation,
            // replaying the original prompt could repeat external side effects.
            // Accepted is the only phase known to be safe for raw redispatch.
            return Ok(UncertainLocalRecoveryOutcome::TranscriptPending);
        };
        if prompt.durable_delivery_provider_session_id() != Some(provider_session_id.as_str()) {
            let Some(provider_run_id) = prompt.durable_delivery_provider_run_id() else {
                // The newer provider session may be authoritative, but without the
                // durable run identity we cannot safely rewrite the prompt's delivery
                // record. Keep the recovery pending until transcript observation or
                // durable state supplies enough identity to reconcile it.
                return Ok(UncertainLocalRecoveryOutcome::TranscriptPending);
            };
            prompt = self.owned.mark_active_prompt_delivery(
                session_id,
                agent.id(),
                prompt.id(),
                delivery_phase,
                Some(provider_run_id.to_string()),
                Some(provider_session_id.clone()),
            )?;
        }
        if let Some(operation_id) = existing_recovery_operation.as_deref() {
            let operation_observed = matched
                .as_ref()
                .is_some_and(|matched| matched.recovery_operation_observed);
            if operation_observed {
                self.owned.mark_active_prompt_recovery_phase(
                    session_id,
                    agent.id(),
                    prompt.id(),
                    operation_id,
                    crate::session::DurablePromptDeliveryPhase::Delivered,
                )?;
            } else if prompt.durable_recovery_phase()
                != Some(crate::session::DurablePromptDeliveryPhase::Accepted)
            {
                return Ok(UncertainLocalRecoveryOutcome::TranscriptPending);
            }
        }
        self.persist_recovered_provider_session(agent, adapter_key, &provider_session_id)?;
        let recovery_prompt =
            self.owned
                .begin_active_prompt_recovery(session_id, agent.id(), prompt.id())?;
        let operation_id = recovery_prompt
            .durable_recovery_operation_id()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "begin provider restart continuation",
                message: "recovery operation did not receive an id".to_string(),
            })?
            .to_string();
        self.owned.mark_active_prompt_recovery_phase(
            session_id,
            agent.id(),
            prompt.id(),
            &operation_id,
            crate::session::DurablePromptDeliveryPhase::Dispatching,
        )?;
        let session_id_owned = session_id.to_string();
        let agent_id_owned = agent.id().to_string();
        let provider_run_id = match self
            .with_app_side_effect(move |app| {
                app.ensure_prompt_provider_run_for_agent(&session_id_owned, &agent_id_owned)
            })
            .await
        {
            Ok(provider_run_id) => provider_run_id,
            Err(error) => {
                let _ = self.owned.mark_active_prompt_recovery_phase(
                    session_id,
                    agent.id(),
                    prompt.id(),
                    &operation_id,
                    crate::session::DurablePromptDeliveryPhase::Accepted,
                );
                return Err(error);
            }
        };
        let structured = self
            .owned
            .provider_store
            .get_run(&provider_run_id)
            .is_ok_and(|run| {
                self.owned
                    .provider_store
                    .run_uses_structured_prompt_io(&run)
            });
        let dispatch = crate::app::KernelPromptDispatch {
            session_id: session_id.to_string(),
            provider_run_id,
            agent_id: agent.id().to_string(),
            prompt_id: prompt.id().to_string(),
            target_active_prompt_id: None,
            source_attachment_id: format!("{KERNEL_RECOVERY_ATTACHMENT_PREFIX}{operation_id}"),
            prompt: provider_restart_continuation_prompt(&operation_id),
            hidden_system_context: recovery_material.hidden_system_context,
            attachments: recovery_material.attachments,
            prompt_origin: crate::session::PromptOrigin::Chariox,
            external_provider: None,
            external_provider_session_id: None,
            external_provider_turn_id: None,
            steering: false,
        };
        if let Err(error) = self.enqueue_prompt_dispatch(&dispatch).await {
            let _ = self.owned.mark_active_prompt_recovery_phase(
                session_id,
                agent.id(),
                prompt.id(),
                &operation_id,
                crate::session::DurablePromptDeliveryPhase::Accepted,
            );
            return Err(error);
        }
        if !structured {
            self.owned.mark_active_prompt_recovery_phase(
                session_id,
                agent.id(),
                prompt.id(),
                &operation_id,
                crate::session::DurablePromptDeliveryPhase::Delivered,
            )?;
        }
        Ok(UncertainLocalRecoveryOutcome::ContinuationDispatched)
    }

    fn persist_recovered_provider_session(
        &self,
        agent: &crate::agent::AgentInstance,
        adapter_key: &str,
        provider_session_id: &str,
    ) -> Result<(), DaemonError> {
        if agent
            .provider_resume_state()
            .provider_session_id(adapter_key)
            == Some(provider_session_id)
        {
            return Ok(());
        }
        let mut resume_state = agent.provider_resume_state().clone();
        if !resume_state.set_provider_session_id(adapter_key, provider_session_id.to_string()) {
            return Err(DaemonError::LocalTransport {
                operation: "persist provider restart session",
                message: format!("provider `{adapter_key}` has no resumable session identity"),
            });
        }
        self.owned.agent_store.set_agent_runtime_profile_durably(
            &self.owned.durable_state_store,
            agent.id(),
            agent.provider(),
            agent.model().map(str::to_string),
            agent.effort().map(str::to_string),
            Some(agent.provider_account_profile().to_string()),
            resume_state,
            None,
            Some("provider_restart_transcript_reconciled"),
        )?;
        Ok(())
    }
}

fn preferred_restart_recovery_provider_session_id(
    observed_session_id: Option<&str>,
    dispatch_session_id: Option<&str>,
    durable_agent_session_id: Option<&str>,
    workflow_prompt: bool,
) -> Option<String> {
    if workflow_prompt {
        observed_session_id.or(dispatch_session_id)
    } else {
        durable_agent_session_id
            .or(dispatch_session_id)
            .or(observed_session_id)
    }
    .map(str::to_string)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RestartRecoveryPromptMaterial {
    transcript_match_text: String,
    hidden_system_context: String,
    attachments: Vec<crate::session::PromptAttachment>,
}

fn restart_recovery_prompt_material(
    prompt: &crate::session::PromptQueueItem,
    workflow_rendered_prompt: Option<&str>,
) -> RestartRecoveryPromptMaterial {
    let transcript_match_text = workflow_rendered_prompt
        .unwrap_or_else(|| prompt.prompt())
        .to_string();
    let hidden_system_context = match workflow_rendered_prompt {
        Some(rendered_prompt) => join_restart_recovery_context(
            prompt.hidden_system_context(),
            &format!(
                "Authoritative durable workflow turn interrupted by a kernel restart:\n\n{rendered_prompt}"
            ),
        ),
        None => prompt.hidden_system_context().to_string(),
    };
    RestartRecoveryPromptMaterial {
        transcript_match_text,
        hidden_system_context,
        attachments: prompt.attachments().to_vec(),
    }
}

fn workflow_turn_rendered_prompt(
    session: &crate::session::RuntimeSession,
    prompt: &crate::session::PromptQueueItem,
) -> Option<String> {
    let workflow_run_id = prompt.workflow_run_id()?;
    let workflow_node_run_id = prompt.workflow_node_run_id()?;
    session
        .workflow_runs()
        .iter()
        .find(|run| run.id() == workflow_run_id)?
        .node_runs()
        .iter()
        .find(|node_run| node_run.id() == workflow_node_run_id)?
        .turn_envelope()?
        .rendered_prompt()
        .map(str::to_string)
}

fn join_restart_recovery_context(existing: &str, recovery: &str) -> String {
    match existing.trim() {
        "" => recovery.to_string(),
        existing => format!("{existing}\n\n{recovery}"),
    }
}

fn recoverable_queued_publication_prompt(
    session: &crate::session::RuntimeSession,
    prompt: &crate::session::PromptQueueItem,
) -> bool {
    if !std::path::Path::new(session.worktree_id()).exists() {
        return false;
    }
    let (Some(workflow_run_id), Some(workflow_node_run_id)) =
        (prompt.workflow_run_id(), prompt.workflow_node_run_id())
    else {
        return false;
    };
    session
        .workflow_runs()
        .iter()
        .find(|run| run.id() == workflow_run_id)
        .is_some_and(|run| {
            run.publication_invocation().is_some_and(|invocation| {
                session.workflow_publications().iter().any(|publication| {
                    publication.id() == invocation.publication_id && publication.enabled()
                })
            }) && matches!(
                run.status(),
                crate::session::WorkflowRunStatus::Created
                    | crate::session::WorkflowRunStatus::Running
                    | crate::session::WorkflowRunStatus::Waiting
            ) && run
                .node_runs()
                .iter()
                .find(|node_run| node_run.id() == workflow_node_run_id)
                .is_some_and(|node_run| {
                    !matches!(
                        node_run.status(),
                        crate::session::WorkflowNodeRunStatus::Completed
                            | crate::session::WorkflowNodeRunStatus::Failed
                            | crate::session::WorkflowNodeRunStatus::Stopped
                    )
                })
        })
}

fn provider_restart_continuation_prompt(operation_id: &str) -> String {
    format!(
        "[Chariox recovery operation {operation_id}] Continue the active task from the current provider session state. Do not repeat completed tool calls or external side effects. If the task already completed, return its final response from the existing results."
    )
}

fn log_restart_recovery_failure(
    session_id: &str,
    agent_id: &str,
    prompt_id: &str,
    error: &DaemonError,
) {
    crate::logging::warn_with_fields(
        "durable_state.recovery",
        "durable prompt restart reconciliation failed",
        serde_json::json!({
            "session_id": session_id,
            "agent_id": agent_id,
            "prompt_id": prompt_id,
            "error": error.to_string(),
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::CreateAgentRequest;
    use crate::app::KernelSessionService;
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::config::DaemonConfig;
    use crate::provider::LaunchProviderRequest;
    use crate::session::{CreateSessionRequest, PromptQueueItem, PromptStatus};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    fn runtime_with_active_prompt(
        delivery_phase: crate::session::DurablePromptDeliveryPhase,
    ) -> (KernelRuntimeState, String, String, String) {
        runtime_with_active_prompt_in_worktree(
            delivery_phase,
            std::env::current_dir()
                .expect("test workspace should resolve")
                .to_string_lossy()
                .as_ref(),
        )
    }

    fn runtime_with_active_prompt_in_worktree(
        delivery_phase: crate::session::DurablePromptDeliveryPhase,
        worktree: &str,
    ) -> (KernelRuntimeState, String, String, String) {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "workspace-restart-recovery",
                worktree,
            ))
            .expect("session should create");
        let agent = KernelSessionService::new(&mut app)
            .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_worktree(worktree))
            .expect("agent should create");
        let attachment = KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "attachment-restart-recovery",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let provider_run = app
            .launch_provider(
                LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "dev-stub",
                    "default",
                    "sonnet",
                )
                .with_agent_id(agent.id()),
            )
            .expect("provider should launch");
        let prompt = PromptQueueItem::new(
            "pending-restart-recovery",
            attachment.id(),
            agent.id(),
            "continue after restart",
            PromptStatus::Queued,
        )
        .with_durable_operation("command-restart-recovery", "fingerprint-restart-recovery");
        let outcome = app
            .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
            .expect("prompt should be accepted");
        let prompt = match outcome {
            crate::session::PromptSubmissionOutcome::Started { prompt } => prompt,
            crate::session::PromptSubmissionOutcome::Queued { .. } => {
                panic!("prompt should start")
            }
        };
        app.mark_active_prompt_delivery(
            session.id(),
            agent.id(),
            prompt.id(),
            delivery_phase,
            (delivery_phase != crate::session::DurablePromptDeliveryPhase::Accepted)
                .then(|| provider_run.id().to_string()),
            None,
        )
        .expect("delivery phase should persist");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let prompt_id = prompt.id().to_string();
        app.attachments().remove_session_attachments(&session_id);
        let mut restored = app
            .sessions()
            .get_session(&session_id)
            .expect("session should load before simulated restart");
        restored.reconcile_after_kernel_restart();
        app.sessions_mut().restore_session(restored);
        let app = Arc::new(Mutex::new(app));
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            app,
            crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
        );
        (router.runtime_state(), session_id, agent_id, prompt_id)
    }

    #[test]
    fn workflow_restart_recovery_never_uses_an_agent_level_session() {
        assert_eq!(
            preferred_restart_recovery_provider_session_id(
                None,
                Some("prompt-session"),
                Some("stale-agent-session"),
                true,
            ),
            Some("prompt-session".to_string())
        );
        assert_eq!(
            preferred_restart_recovery_provider_session_id(
                None,
                None,
                Some("stale-agent-session"),
                true,
            ),
            None
        );
        assert_eq!(
            preferred_restart_recovery_provider_session_id(
                Some("observed-session"),
                Some("prompt-session"),
                Some("stale-agent-session"),
                true,
            ),
            Some("observed-session".to_string())
        );
    }

    #[test]
    fn interactive_restart_recovery_prefers_the_durable_agent_session() {
        assert_eq!(
            preferred_restart_recovery_provider_session_id(
                Some("observed-session"),
                Some("dispatch-session"),
                Some("acknowledged-agent-session"),
                false,
            ),
            Some("acknowledged-agent-session".to_string())
        );
    }

    #[test]
    fn workflow_restart_recovery_preserves_durable_context_and_attachments() {
        let attachment = crate::session::PromptAttachment::new(
            "file:///tmp/review.patch",
            "text/plain",
            Some("review.patch".to_string()),
        );
        let prompt = PromptQueueItem::new(
            "pending-workflow-recovery",
            "workflow-run:test",
            "agent-1",
            "raw queued prompt",
            PromptStatus::Running,
        )
        .with_hidden_system_context("existing hidden context")
        .with_attachments(vec![attachment.clone()]);

        let material =
            restart_recovery_prompt_material(&prompt, Some("authoritative rendered workflow turn"));

        assert_eq!(
            material.transcript_match_text,
            "authoritative rendered workflow turn"
        );
        assert!(material
            .hidden_system_context
            .starts_with("existing hidden context\n\n"));
        assert!(material
            .hidden_system_context
            .contains("authoritative rendered workflow turn"));
        assert_eq!(material.attachments, vec![attachment]);
    }

    fn runtime_with_queued_metaagent_task() -> (KernelRuntimeState, String, String) {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "workspace-restart-meta-queue",
                "worktree-restart-meta-queue",
            ))
            .expect("session should create");
        let attachment = KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "attachment-restart-meta-queue",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.launch_provider(
            LaunchProviderRequest::new(session.id(), "dev-stub", "dev-stub", "default", "sonnet")
                .with_agent_id(agent.id()),
        )
        .expect("provider should launch");
        app.sessions_mut()
            .enqueue_metaagent_task(
                session.id(),
                agent.id(),
                attachment.id(),
                "resume queued Meta work",
                Vec::new(),
            )
            .expect("Meta task should queue");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            app,
            crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
        );
        (router.runtime_state(), session_id, agent_id)
    }

    fn runtime_with_queued_prompt() -> (KernelRuntimeState, String, String) {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let workspace = std::env::current_dir().expect("test workspace should resolve");
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                workspace.to_string_lossy(),
                workspace.to_string_lossy(),
            ))
            .expect("session should create");
        let mut session_with_agents = session.clone();
        session_with_agents.set_agents(vec![agent.clone()]);
        app.sessions_mut().restore_session(session_with_agents);
        let _attachment = KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "attachment-restart-prompt-queue",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let workflow = app
            .sessions_mut()
            .create_workflow(session.id(), Some("restart-publication".to_string()))
            .expect("workflow should create");
        let node = app
            .sessions_mut()
            .add_workflow_node(session.id(), workflow.id(), agent.id())
            .expect("workflow node should create");
        let endpoint = app
            .sessions_mut()
            .create_workflow_endpoint(
                session.id(),
                workflow.id(),
                node.id(),
                Some("entry".to_string()),
            )
            .expect("workflow endpoint should create");
        let publication = app
            .sessions_mut()
            .create_workflow_publication(
                session.id(),
                workflow.id(),
                endpoint.id(),
                Some("default".to_string()),
                Some("restart-publication".to_string()),
                Some(crate::session::WORKFLOW_PUBLICATION_KIND_INGRESS.to_string()),
                Some("/restart".to_string()),
                vec!["POST".to_string()],
                None,
                None,
                None,
                None,
                Some("async".to_string()),
                None,
                None,
                "local".to_string(),
            )
            .expect("workflow publication should create");
        let publication_invocation = crate::session::WorkflowPublicationInvocationEnvelope {
            publication_id: publication.id().to_string(),
            hook_id: None,
            invocation_id: "invocation-restart".to_string(),
            transport: "event".to_string(),
            endpoint_id: endpoint.id().to_string(),
            queue_ref: Some("default".to_string()),
            input: serde_json::json!({"prompt": "resume queued work"}),
            artifacts: Vec::new(),
            mode: None,
            caller: serde_json::json!({"type": "event"}),
        };
        let workflow_run = app
            .sessions_mut()
            .invoke_workflow_endpoint_with_publication_invocation(
                session.id(),
                workflow.id(),
                endpoint.id(),
                Some("resume queued work".to_string()),
                Some(publication_invocation),
            )
            .expect("published workflow run should create");
        let node_run_id = workflow_run.node_runs()[0].id().to_string();
        app.sessions_mut()
            .prepare_workflow_turn(
                session.id(),
                workflow_run.id(),
                &node_run_id,
                format!("workflow-ack:{node_run_id}"),
                "resume queued work".to_string(),
                None,
                None,
            )
            .expect("workflow turn should prepare");
        let prompt = PromptQueueItem::new(
            "pending-restart-prompt-queue",
            crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
            agent.id(),
            "resume queued work",
            PromptStatus::Queued,
        )
        .with_workflow_context(workflow_run.id(), &node_run_id);
        let outcome = app
            .prompt_owner_submit_prepared_prompt(session.id(), prompt, true)
            .expect("prompt should queue");
        assert!(matches!(
            outcome,
            crate::session::PromptSubmissionOutcome::Queued { .. }
        ));
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            app,
            crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
        );
        (router.runtime_state(), session_id, agent_id)
    }

    #[tokio::test]
    async fn queued_metaagent_task_starts_after_restart_without_an_active_prompt() {
        let (runtime, session_id, agent_id) = runtime_with_queued_metaagent_task();

        let summary = runtime.recover_durable_runtime_after_restart().await;

        assert_eq!(summary, DurableRestartRecoverySummary::default());
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let session = runtime
                    .owned
                    .session_store
                    .get_session(&session_id)
                    .expect("session should remain available");
                if session.queued_metaagent_tasks().is_empty()
                    && session.metaagent_task(&agent_id).is_some_and(|task| {
                        task.status() == crate::session::MetaagentTaskStatus::Active
                    })
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("queued Meta task should restart");
    }

    #[tokio::test]
    async fn queued_local_prompt_starts_provider_after_restart() {
        let (runtime, session_id, agent_id) = runtime_with_queued_prompt();

        let summary = runtime.recover_durable_runtime_after_restart().await;

        assert_eq!(summary.queued_local_prompts_started, 1);
        assert_eq!(summary.failed_reconciliations, 0);
        let run = runtime
            .owned
            .provider_store
            .get_run_for_agent(&session_id, &agent_id)
            .expect("queued prompt recovery should launch its provider");
        assert!(matches!(
            run.state(),
            crate::provider::ProviderRunState::Starting
                | crate::provider::ProviderRunState::Running
        ));
    }

    #[tokio::test]
    async fn accepted_prompt_is_redispatched_after_restart() {
        let (runtime, session_id, agent_id, prompt_id) =
            runtime_with_active_prompt(crate::session::DurablePromptDeliveryPhase::Accepted);

        let summary = runtime.recover_durable_runtime_after_restart().await;

        assert_eq!(summary.accepted_local_redispatched, 1);
        assert_eq!(summary.failed_reconciliations, 0);
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should load");
        let prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .expect("prompt should remain active");
        assert_eq!(prompt.id(), prompt_id);
        assert_eq!(
            prompt.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Delivered)
        );
    }

    #[tokio::test]
    async fn restart_recovery_retry_ignores_prompt_outside_startup_snapshot() {
        let (runtime, session_id, agent_id, prompt_id) =
            runtime_with_active_prompt(crate::session::DurablePromptDeliveryPhase::Accepted);

        let summary = runtime
            .recover_durable_runtime_after_restart_targets(&BTreeSet::new(), &BTreeSet::new())
            .await;

        assert_eq!(summary, DurableRestartRecoverySummary::default());
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should load");
        let prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .expect("post-startup prompt should remain active");
        assert_eq!(prompt.id(), prompt_id);
        assert_eq!(
            prompt.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Accepted)
        );
    }

    #[tokio::test]
    async fn restart_recovery_skips_local_prompt_when_its_workspace_is_gone() {
        let missing_worktree = std::env::temp_dir().join(format!(
            "chariox-missing-restart-recovery-{}",
            std::process::id()
        ));
        let (runtime, session_id, agent_id, prompt_id) = runtime_with_active_prompt_in_worktree(
            crate::session::DurablePromptDeliveryPhase::Delivered,
            missing_worktree.to_string_lossy().as_ref(),
        );

        let summary = runtime.recover_durable_runtime_after_restart().await;

        assert_eq!(summary, DurableRestartRecoverySummary::default());
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should remain available");
        let prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .expect("unrecoverable prompt should remain preserved");
        assert_eq!(prompt.id(), prompt_id);
    }

    #[tokio::test]
    async fn uncertain_dev_stub_prompt_is_redispatched_after_restart() {
        let (runtime, session_id, agent_id, prompt_id) =
            runtime_with_active_prompt(crate::session::DurablePromptDeliveryPhase::Dispatching);

        let summary = runtime.recover_durable_runtime_after_restart().await;

        assert_eq!(summary.accepted_local_redispatched, 0);
        assert_eq!(summary.uncertain_original_redispatched, 1);
        assert_eq!(summary.uncertain_local_prompts_preserved, 0);
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should load");
        let prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .expect("redispatched prompt should remain active");
        assert_eq!(prompt.id(), prompt_id);
        assert_eq!(
            prompt.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Delivered)
        );
    }

    #[tokio::test]
    async fn cancelling_local_prompt_is_finalized_instead_of_resumed_after_restart() {
        let (runtime, session_id, agent_id, _prompt_id) =
            runtime_with_active_prompt(crate::session::DurablePromptDeliveryPhase::Delivered);
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should load");
        runtime
            .owned
            .prompt_state_owner
            .begin_cancelling_active_prompt(&session, &agent_id)
            .expect("prompt should begin cancelling");
        let (active_prompt, queued_prompts) = runtime
            .owned
            .prompt_state_owner
            .state_parts(&session, &agent_id);
        runtime
            .owned
            .mirror_prompt_owner_agent_state(&session_id, &agent_id, active_prompt, queued_prompts)
            .expect("cancelling state should persist");

        let summary = runtime.recover_durable_runtime_after_restart().await;

        assert_eq!(summary.cancelled_local_prompts_finalized, 1);
        assert_eq!(summary.provider_continuations_dispatched, 0);
        assert_eq!(summary.failed_reconciliations, 0);
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should remain available");
        assert!(
            runtime
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, &agent_id)
                .is_none(),
            "cancelled prompt must not be resumed after restart"
        );
    }

    #[tokio::test]
    async fn failed_delivery_intent_recovers_after_fresh_bootstrap_without_provider_run() {
        let config = DaemonConfig::for_tests();
        let worktree = std::env::current_dir().expect("test workspace should resolve");
        let (session_id, agent_id, provider_run_id, prompt_id) = {
            let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
            let (session, agent) = KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new(
                    worktree.to_string_lossy(),
                    worktree.to_string_lossy(),
                ))
                .expect("session should create");
            let attachment = KernelSessionService::new(&mut app)
                .attach(AttachRequest::new(
                    session.id(),
                    "attachment-fresh-bootstrap-failed-delivery",
                    ClientCapabilityLevel::FullTerminal,
                ))
                .expect("attachment should attach");
            let updated_agent = app
                .agents_mut()
                .set_agent_runtime_profile_with_account_profile(
                    agent.id(),
                    "codex",
                    Some("gpt-5.5".to_string()),
                    None,
                    Some("default".to_string()),
                    crate::provider::ProviderResumeState::from_codex_thread_id(
                        "codex-thread-failed-before-restart",
                    ),
                )
                .expect("failed resume should seed");
            app.durable_state_store()
                .append_event(
                    "agent.runtime_profile_updated",
                    Some(updated_agent.id().to_string()),
                    serde_json::json!({
                        "agent": &updated_agent,
                        "reason": "test_failed_delivery_resume_seeded",
                    }),
                )
                .expect("failed resume should persist");
            let request =
                LaunchProviderRequest::new(session.id(), "codex", "codex", "default", "gpt-5.5")
                    .with_agent_id(agent.id())
                    .with_resume_state(crate::provider::ProviderResumeState::from_codex_thread_id(
                        "codex-thread-failed-before-restart",
                    ));
            let mut provider_run = crate::provider::RuntimeProviderRun::new(
                "provider-run-failed-before-restart",
                &request,
                crate::provider::ProviderLaunchResult {
                    endpoint_mode: crate::provider::AgentEndpointMode::External,
                    process_label: "test-codex".to_string(),
                    pty_target: None,
                    pty_program: None,
                    pty_args: Vec::new(),
                    pty_env: Default::default(),
                    pty_env_remove: Vec::new(),
                    working_directory: None,
                    structured_endpoint: Some("test-codex-runtime".to_string()),
                },
            );
            provider_run.mark_running();
            app.providers_mut()
                .insert_run_for_test(provider_run.clone());
            app.sessions_mut()
                .set_active_provider_run(session.id(), Some(provider_run.id().to_string()))
                .expect("failed run should be active");
            app.update_provider_run_projection(provider_run.clone());
            let prompt = PromptQueueItem::new(
                "prompt-failed-before-restart",
                attachment.id(),
                agent.id(),
                "do not replay this failed delivery",
                PromptStatus::Queued,
            );
            let prompt = match app
                .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
                .expect("prompt should start")
            {
                crate::session::PromptSubmissionOutcome::Started { prompt } => prompt,
                crate::session::PromptSubmissionOutcome::Queued { .. } => {
                    panic!("prompt should start immediately")
                }
            };
            app.mark_active_prompt_delivery(
                session.id(),
                agent.id(),
                prompt.id(),
                crate::session::DurablePromptDeliveryPhase::Dispatching,
                Some(provider_run.id().to_string()),
                Some("codex-thread-failed-before-restart".to_string()),
            )
            .expect("dispatch identity should persist");
            let session_id = session.id().to_string();
            let agent_id = agent.id().to_string();
            let provider_run_id = provider_run.id().to_string();
            let prompt_id = prompt.id().to_string();
            let app = Arc::new(Mutex::new(app));
            let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
                app,
                crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
            );
            let runtime = router.runtime_state();
            let active_status = runtime
                .owned
                .session_store
                .get_session(&session_id)
                .expect("session should remain available")
                .active_prompt_for_agent(&agent_id)
                .expect("prompt should remain active")
                .status();
            runtime
                .owned
                .compare_and_mark_active_prompt_delivery_failure(
                    &session_id,
                    &agent_id,
                    &prompt_id,
                    &provider_run_id,
                    "codex-thread-failed-before-restart",
                    (active_status, PromptStatus::Cancelling),
                )
                .expect("failure intent should persist")
                .expect("failure intent should match");
            (session_id, agent_id, provider_run_id, prompt_id)
        };

        let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
        assert!(
            app.providers().get_run(&provider_run_id).is_err(),
            "process-local provider runs must not survive a fresh bootstrap",
        );
        assert_eq!(
            app.sessions()
                .get_session(&session_id)
                .expect("session should restore")
                .active_provider_run_id(),
            None,
        );
        let app = Arc::new(Mutex::new(app));
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            app,
            crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
        );
        let runtime = router.runtime_state();
        let restored_session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should restore");
        let restored_prompt = restored_session
            .active_prompt_for_agent(&agent_id)
            .expect("failure intent should restore")
            .clone();
        assert_eq!(restored_prompt.id(), prompt_id);
        assert_eq!(restored_prompt.status(), PromptStatus::Cancelling);
        assert!(restored_prompt.durable_delivery_failure_pending());

        let summary = runtime.recover_durable_runtime_after_restart().await;

        assert_eq!(summary.failed_reconciliations, 0);
        assert_eq!(summary.cancelled_local_prompts_finalized, 1);
        assert!(runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should remain available")
            .active_prompt_for_agent(&agent_id)
            .is_none(),);
        assert_eq!(
            runtime
                .owned
                .agent_store
                .get_agent(&agent_id)
                .expect("agent should remain available")
                .provider_resume_state()
                .codex_thread_id(),
            None,
        );
    }

    async fn assert_restart_prefers_durable_agent_session_over_prompt_session(
        delivery_phase: crate::session::DurablePromptDeliveryPhase,
        persist_provider_run_id: bool,
    ) {
        let config = DaemonConfig::for_tests();
        let worktree = std::env::current_dir().expect("test workspace should resolve");
        let (session_id, agent_id, prompt_id) = {
            let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
            let (session, agent) = KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new(
                    worktree.to_string_lossy(),
                    worktree.to_string_lossy(),
                ))
                .expect("session should create");
            let attachment = KernelSessionService::new(&mut app)
                .attach(AttachRequest::new(
                    session.id(),
                    "attachment-structured-ack-crash-prefix",
                    ClientCapabilityLevel::FullTerminal,
                ))
                .expect("attachment should attach");
            let updated_agent = app
                .agents_mut()
                .set_agent_runtime_profile_with_account_profile(
                    agent.id(),
                    "codex",
                    Some("gpt-5.5".to_string()),
                    None,
                    Some("default".to_string()),
                    crate::provider::ProviderResumeState::from_codex_thread_id(
                        "codex-thread-acknowledged-s2",
                    ),
                )
                .expect("acknowledged resume should seed");
            app.durable_state_store()
                .append_event(
                    "agent.runtime_profile_updated",
                    Some(updated_agent.id().to_string()),
                    serde_json::json!({
                        "agent": &updated_agent,
                        "reason": "test_structured_ack_crash_prefix",
                    }),
                )
                .expect("acknowledged resume should persist");
            let mut prompt = PromptQueueItem::new(
                "prompt-structured-ack-crash-prefix",
                attachment.id(),
                agent.id(),
                "continue the acknowledged provider session",
                PromptStatus::Queued,
            );
            let recovery_operation_id = prompt.begin_durable_recovery_operation();
            assert!(prompt.mark_durable_recovery_phase(
                &recovery_operation_id,
                crate::session::DurablePromptDeliveryPhase::Dispatching,
            ));
            let prompt = match app
                .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
                .expect("prompt should start")
            {
                crate::session::PromptSubmissionOutcome::Started { prompt } => prompt,
                crate::session::PromptSubmissionOutcome::Queued { .. } => {
                    panic!("prompt should start immediately")
                }
            };
            app.mark_active_prompt_delivery(
                session.id(),
                agent.id(),
                prompt.id(),
                delivery_phase,
                persist_provider_run_id
                    .then(|| "provider-run-structured-ack-crash-prefix".to_string()),
                Some("codex-thread-dispatch-s1".to_string()),
            )
            .expect("dispatch identity should persist");
            (
                session.id().to_string(),
                agent.id().to_string(),
                prompt.id().to_string(),
            )
        };

        let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
        let restored_agent = app
            .agents()
            .get_agent(&agent_id)
            .expect("agent should restore");
        assert_eq!(
            restored_agent.provider_resume_state().codex_thread_id(),
            Some("codex-thread-acknowledged-s2"),
        );
        let restored_session = app
            .sessions()
            .get_session(&session_id)
            .expect("session should restore");
        assert_eq!(
            restored_session
                .active_prompt_for_agent(&agent_id)
                .expect("prompt should restore")
                .durable_delivery_provider_session_id(),
            Some("codex-thread-dispatch-s1"),
        );
        let app = Arc::new(Mutex::new(app));
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            app,
            crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
        );
        let runtime = router.runtime_state();

        let summary = runtime.recover_durable_runtime_after_restart().await;

        assert_eq!(summary.failed_reconciliations, 0);
        assert_eq!(summary.transcript_recovery_pending, 1);
        let restored_agent = runtime
            .owned
            .agent_store
            .get_agent(&agent_id)
            .expect("agent should remain available");
        assert_eq!(
            restored_agent.provider_resume_state().codex_thread_id(),
            Some("codex-thread-acknowledged-s2"),
            "restart recovery must not downgrade the durable acknowledgement to dispatch S1",
        );
        let restored_session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should remain available");
        let restored_prompt = restored_session
            .active_prompt_for_agent(&agent_id)
            .expect("uncertain prompt should remain active");
        assert_eq!(restored_prompt.id(), prompt_id);
        assert_eq!(
            restored_prompt.durable_delivery_provider_session_id(),
            if persist_provider_run_id {
                Some("codex-thread-acknowledged-s2")
            } else {
                Some("codex-thread-dispatch-s1")
            },
            "the prompt may converge only when its durable provider run identifies the delivery",
        );
        assert_eq!(
            restored_prompt.durable_delivery_phase(),
            Some(delivery_phase),
            "reconciling the session identity must preserve the established delivery phase",
        );
    }

    #[tokio::test]
    async fn structured_ack_restart_prefers_durable_agent_session_over_dispatch_session() {
        assert_restart_prefers_durable_agent_session_over_prompt_session(
            crate::session::DurablePromptDeliveryPhase::Dispatching,
            true,
        )
        .await;
    }

    #[tokio::test]
    async fn structured_output_restart_updates_delivered_prompt_to_durable_agent_session() {
        assert_restart_prefers_durable_agent_session_over_prompt_session(
            crate::session::DurablePromptDeliveryPhase::Delivered,
            true,
        )
        .await;
    }

    #[tokio::test]
    async fn restart_session_mismatch_without_a_provider_run_remains_pending() {
        assert_restart_prefers_durable_agent_session_over_prompt_session(
            crate::session::DurablePromptDeliveryPhase::Dispatching,
            false,
        )
        .await;
    }

    #[tokio::test]
    async fn restart_recovery_preserves_superseding_resume_as_uncertain_delivery() {
        let _environment = crate::env_lock::lock();
        let mut app = crate::test_support::bootstrap_authenticated_app(DaemonConfig::for_tests())
            .expect("daemon should boot");
        let worktree = std::env::current_dir().expect("test workspace should resolve");
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                worktree.to_string_lossy(),
                worktree.to_string_lossy(),
            ))
            .expect("session should create");
        let attachment = KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "attachment-restart-superseded-resume",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let updated_agent = app
            .agents_mut()
            .set_agent_runtime_profile_with_account_profile(
                agent.id(),
                "codex",
                Some("current-model".to_string()),
                None,
                Some("default".to_string()),
                crate::provider::ProviderResumeState::from_codex_thread_id("codex-thread-current"),
            )
            .expect("superseding resume should update");
        app.durable_state_store()
            .append_event(
                "agent.runtime_profile_updated",
                Some(updated_agent.id().to_string()),
                serde_json::json!({
                    "agent": &updated_agent,
                    "reason": "test_superseding_resume_seeded",
                }),
            )
            .expect("superseding resume should persist");
        let request =
            LaunchProviderRequest::new(session.id(), "codex", "codex", "default", "gpt-5.5")
                .with_agent_id(agent.id())
                .with_resume_state(crate::provider::ProviderResumeState::from_codex_thread_id(
                    "codex-thread-current",
                ));
        let mut provider_run = crate::provider::RuntimeProviderRun::new(
            "provider-run-restart-superseded-resume",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::External,
                process_label: "test-codex".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: Default::default(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: Some("test-codex-runtime".to_string()),
            },
        );
        provider_run.mark_running();
        app.providers_mut()
            .insert_run_for_test(provider_run.clone());
        app.sessions_mut()
            .set_active_provider_run(session.id(), Some(provider_run.id().to_string()))
            .expect("failed provider run should be active");
        app.update_provider_run_projection(provider_run.clone());
        let prompt = PromptQueueItem::new(
            "prompt-restart-superseded-resume",
            attachment.id(),
            agent.id(),
            "recover without replaying this prompt",
            PromptStatus::Queued,
        );
        let prompt = match app
            .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
            .expect("prompt should start")
        {
            crate::session::PromptSubmissionOutcome::Started { prompt } => prompt,
            crate::session::PromptSubmissionOutcome::Queued { .. } => {
                panic!("prompt should start immediately")
            }
        };
        app.mark_active_prompt_delivery(
            session.id(),
            agent.id(),
            prompt.id(),
            crate::session::DurablePromptDeliveryPhase::Dispatching,
            Some(provider_run.id().to_string()),
            Some("codex-thread-failed".to_string()),
        )
        .expect("dispatch identity should persist");
        let app = Arc::new(Mutex::new(app));
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            app,
            crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
        );
        let runtime = router.runtime_state();
        let active = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(
                &runtime
                    .owned
                    .session_store
                    .get_session(session.id())
                    .expect("session should remain available"),
                agent.id(),
            )
            .expect("prompt should remain active");
        runtime
            .owned
            .compare_and_mark_active_prompt_delivery_failure(
                session.id(),
                agent.id(),
                active.id(),
                provider_run.id(),
                "codex-thread-failed",
                (active.status(), PromptStatus::Cancelling),
            )
            .expect("failure intent should persist")
            .expect("failure intent should match the active prompt");

        let summary = runtime.recover_durable_runtime_after_restart().await;

        assert_eq!(summary.failed_reconciliations, 0);
        assert_eq!(summary.cancelled_local_prompts_finalized, 0);
        assert_eq!(summary.uncertain_local_prompts_preserved, 0);
        assert_eq!(summary.provider_continuations_dispatched, 1);
        let session_state = runtime
            .owned
            .session_store
            .get_session(session.id())
            .expect("session should remain available");
        let prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session_state, agent.id())
            .expect("uncertain prompt should remain active");
        assert_eq!(prompt.status(), PromptStatus::Dispatching);
        assert_eq!(
            prompt.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Dispatching),
        );
        assert_eq!(
            prompt.durable_delivery_provider_session_id(),
            Some("codex-thread-current"),
        );
        assert!(!prompt.durable_delivery_failure_pending());
        assert_eq!(
            runtime
                .owned
                .provider_store
                .get_run(provider_run.id())
                .expect("superseded provider run should remain recoverable")
                .state(),
            crate::provider::ProviderRunState::Running,
        );
    }

    #[test]
    fn recovery_operation_reuses_accepted_generation_and_advances_after_delivery() {
        let mut prompt = PromptQueueItem::new(
            "prompt-1",
            "attachment-1",
            "agent-1",
            "recover",
            PromptStatus::Running,
        );

        let first = prompt.begin_durable_recovery_operation();
        assert_eq!(prompt.begin_durable_recovery_operation(), first);
        assert!(prompt.mark_durable_recovery_phase(
            &first,
            crate::session::DurablePromptDeliveryPhase::Delivered,
        ));
        let second = prompt.begin_durable_recovery_operation();

        assert_eq!(first, "chariox-recovery:prompt-1:1");
        assert_eq!(second, "chariox-recovery:prompt-1:2");
        assert_eq!(
            prompt.durable_recovery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Accepted)
        );
    }

    #[test]
    fn restart_recovery_rearms_only_on_the_tenth_pending_observation() {
        assert!(!should_rearm_unobserved_dispatches(8, 1, 1));
        assert!(should_rearm_unobserved_dispatches(9, 1, 1));
        assert!(should_rearm_unobserved_dispatches(10, 1, 1));
        assert!(!should_rearm_unobserved_dispatches(9, 0, 1));
        assert!(!should_rearm_unobserved_dispatches(9, 1, 0));
    }

    #[test]
    fn restart_recovery_rearms_unobserved_dispatch_with_the_same_operation() {
        let (runtime, session_id, agent_id, prompt_id) =
            runtime_with_active_prompt(crate::session::DurablePromptDeliveryPhase::Delivered);
        let recovery = runtime
            .owned
            .begin_active_prompt_recovery(&session_id, &agent_id, &prompt_id)
            .expect("recovery operation should begin");
        let operation_id = recovery
            .durable_recovery_operation_id()
            .expect("recovery operation id should exist")
            .to_string();
        runtime
            .owned
            .mark_active_prompt_recovery_phase(
                &session_id,
                &agent_id,
                &prompt_id,
                &operation_id,
                crate::session::DurablePromptDeliveryPhase::Dispatching,
            )
            .expect("recovery operation should enter dispatching");
        let recovery_targets = runtime.durable_restart_recovery_targets();
        let mut dispatch_targets = runtime.durable_restart_dispatch_targets(&recovery_targets);

        assert_eq!(
            runtime
                .rearm_unobserved_restart_recovery_dispatches(&mut dispatch_targets)
                .expect("rearm should persist"),
            1
        );
        assert!(dispatch_targets.is_empty(), "successful target must retire");

        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should load");
        let prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .expect("active prompt should remain available");
        assert_eq!(
            prompt.durable_recovery_operation_id(),
            Some(operation_id.as_str())
        );
        assert_eq!(
            prompt.durable_recovery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Accepted)
        );
    }

    #[test]
    fn restart_recovery_does_not_rearm_an_operation_dispatched_after_startup() {
        let (runtime, session_id, agent_id, prompt_id) =
            runtime_with_active_prompt(crate::session::DurablePromptDeliveryPhase::Delivered);
        let recovery = runtime
            .owned
            .begin_active_prompt_recovery(&session_id, &agent_id, &prompt_id)
            .expect("recovery operation should begin");
        let operation_id = recovery
            .durable_recovery_operation_id()
            .expect("recovery operation id should exist")
            .to_string();
        let recovery_targets = runtime.durable_restart_recovery_targets();
        let mut dispatch_targets = runtime.durable_restart_dispatch_targets(&recovery_targets);
        assert!(dispatch_targets.is_empty());
        runtime
            .owned
            .mark_active_prompt_recovery_phase(
                &session_id,
                &agent_id,
                &prompt_id,
                &operation_id,
                crate::session::DurablePromptDeliveryPhase::Dispatching,
            )
            .expect("startup recovery should dispatch");

        assert_eq!(
            runtime
                .rearm_unobserved_restart_recovery_dispatches(&mut dispatch_targets)
                .expect("empty startup target set should be a no-op"),
            0
        );
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should load");
        let prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .expect("active prompt should remain available");
        assert_eq!(
            prompt.durable_recovery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
        );
    }

    #[test]
    fn restart_recovery_does_not_downgrade_a_delivered_operation() {
        let (runtime, session_id, agent_id, prompt_id) =
            runtime_with_active_prompt(crate::session::DurablePromptDeliveryPhase::Delivered);
        let recovery = runtime
            .owned
            .begin_active_prompt_recovery(&session_id, &agent_id, &prompt_id)
            .expect("recovery operation should begin");
        let operation_id = recovery
            .durable_recovery_operation_id()
            .expect("recovery operation id should exist")
            .to_string();
        runtime
            .owned
            .mark_active_prompt_recovery_phase(
                &session_id,
                &agent_id,
                &prompt_id,
                &operation_id,
                crate::session::DurablePromptDeliveryPhase::Dispatching,
            )
            .expect("recovery operation should enter dispatching");
        let recovery_targets = runtime.durable_restart_recovery_targets();
        let mut dispatch_targets = runtime.durable_restart_dispatch_targets(&recovery_targets);
        runtime
            .owned
            .mark_active_prompt_recovery_phase(
                &session_id,
                &agent_id,
                &prompt_id,
                &operation_id,
                crate::session::DurablePromptDeliveryPhase::Delivered,
            )
            .expect("delivery acknowledgement should win");

        assert_eq!(
            runtime
                .rearm_unobserved_restart_recovery_dispatches(&mut dispatch_targets)
                .expect("delivered acknowledgement should be a no-op"),
            0
        );
        assert!(dispatch_targets.is_empty(), "delivered target must retire");
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should load");
        let prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .expect("active prompt should remain available");
        assert_eq!(
            prompt.durable_recovery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Delivered)
        );
    }

    #[test]
    fn restart_recovery_rolls_back_rearm_when_durable_append_fails() {
        let (runtime, session_id, agent_id, prompt_id) =
            runtime_with_active_prompt(crate::session::DurablePromptDeliveryPhase::Delivered);
        let recovery = runtime
            .owned
            .begin_active_prompt_recovery(&session_id, &agent_id, &prompt_id)
            .expect("recovery operation should begin");
        let operation_id = recovery
            .durable_recovery_operation_id()
            .expect("recovery operation id should exist")
            .to_string();
        runtime
            .owned
            .mark_active_prompt_recovery_phase(
                &session_id,
                &agent_id,
                &prompt_id,
                &operation_id,
                crate::session::DurablePromptDeliveryPhase::Dispatching,
            )
            .expect("recovery operation should enter dispatching");
        let recovery_targets = runtime.durable_restart_recovery_targets();
        let mut dispatch_targets = runtime.durable_restart_dispatch_targets(&recovery_targets);
        let durable_path = runtime.owned.durable_state_store.path().to_path_buf();
        let connection = rusqlite::Connection::open(&durable_path)
            .expect("durable database should open for failure injection");
        connection
            .execute_batch(
                "CREATE TRIGGER fail_restart_recovery_rearm
                 BEFORE INSERT ON durable_state_events
                 WHEN NEW.kind = 'session.prompt_state.updated'
                 BEGIN
                   SELECT RAISE(FAIL, 'injected restart recovery persistence failure');
                 END;",
            )
            .expect("failure trigger should install");

        let result = runtime.rearm_unobserved_restart_recovery_dispatches(&mut dispatch_targets);

        connection
            .execute_batch("DROP TRIGGER fail_restart_recovery_rearm;")
            .expect("failure trigger should be removed");
        assert!(result.is_err(), "failed persistence must propagate");
        assert_eq!(
            dispatch_targets.len(),
            1,
            "failed target must remain eligible for a later retry"
        );
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should load");
        let owner_prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .expect("owner prompt should remain active");
        let mirrored_prompt = session
            .active_prompt_for_agent(&agent_id)
            .expect("session mirror should remain active");
        assert_eq!(
            owner_prompt.durable_recovery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
        );
        assert_eq!(
            mirrored_prompt.durable_recovery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
        );

        let mut last_payload = runtime
            .owned
            .durable_state_store
            .load_events_after(0)
            .expect("durable events should load")
            .into_iter()
            .rev()
            .find(|event| {
                event.kind == crate::durable_prompt_state::DURABLE_PROMPT_STATE_EVENT_KIND
                    && event.subject_id.as_deref() == Some(session_id.as_str())
            })
            .map(|event| {
                serde_json::from_value::<
                        crate::durable_prompt_state::DurablePromptStateEventPayload,
                    >(event.payload)
                    .expect("durable prompt payload should decode")
            })
            .expect("durable prompt event should exist");
        last_payload.restore_private_states();
        assert_eq!(
            last_payload
                .active_prompt
                .as_ref()
                .and_then(crate::session::PromptQueueItem::durable_recovery_phase),
            Some(crate::session::DurablePromptDeliveryPhase::Dispatching),
            "failed rearm must leave the durable operation undispatched"
        );

        assert!(should_rearm_unobserved_dispatches(
            TRANSCRIPT_OBSERVATION_ATTEMPTS_BEFORE_REDISPATCH + 1,
            1,
            dispatch_targets.len(),
        ));
        assert_eq!(
            runtime
                .rearm_unobserved_restart_recovery_dispatches(&mut dispatch_targets)
                .expect("rearm should retry after durable storage recovers"),
            1
        );
        assert!(
            dispatch_targets.is_empty(),
            "successful retry must retire the frozen target"
        );
        runtime
            .owned
            .mark_active_prompt_recovery_phase(
                &session_id,
                &agent_id,
                &prompt_id,
                &operation_id,
                crate::session::DurablePromptDeliveryPhase::Dispatching,
            )
            .expect("redispatch should return the same operation to dispatching");
        assert!(!should_rearm_unobserved_dispatches(
            TRANSCRIPT_OBSERVATION_ATTEMPTS_BEFORE_REDISPATCH + 2,
            1,
            dispatch_targets.len(),
        ));
    }

    #[tokio::test]
    async fn internal_recovery_prompt_is_not_recorded_as_user_terminal_input() {
        let (runtime, session_id, agent_id, prompt_id) =
            runtime_with_active_prompt(crate::session::DurablePromptDeliveryPhase::Dispatching);
        let provider_run_id = runtime
            .owned
            .provider_store
            .get_run_for_agent(&session_id, &agent_id)
            .expect("provider run should exist")
            .id()
            .to_string();
        let operation_id = "chariox-recovery:prompt-hidden:1";
        let dispatch = crate::app::KernelPromptDispatch {
            session_id: session_id.clone(),
            provider_run_id: provider_run_id.clone(),
            agent_id: agent_id.clone(),
            prompt_id,
            target_active_prompt_id: None,
            source_attachment_id: format!("{KERNEL_RECOVERY_ATTACHMENT_PREFIX}{operation_id}"),
            prompt: provider_restart_continuation_prompt(operation_id),
            hidden_system_context: String::new(),
            attachments: Vec::new(),
            prompt_origin: crate::session::PromptOrigin::Chariox,
            external_provider: None,
            external_provider_session_id: None,
            external_provider_turn_id: None,
            steering: false,
        };

        runtime
            .enqueue_prompt_dispatch(&dispatch)
            .await
            .expect("internal continuation should dispatch");

        assert!(runtime
            .owned
            .terminal_stream
            .input_records()
            .iter()
            .all(|record| !String::from_utf8_lossy(&record.bytes).contains(operation_id)));
    }

    #[tokio::test]
    async fn internal_recovery_prompt_is_not_echoed_to_other_attachments() {
        // The dispatch fanout is the boundary where a recovery envelope would
        // become user-visible terminal output on subscribed attachments. The
        // local dispatch runtime guards its call site, but remote-lease
        // dispatchers also invoke this helper and any future caller could
        // regress the invariant. Assert the fanout helper itself refuses to
        // surface a `kernel-recovery:*` envelope regardless of caller.
        let (runtime, session_id, agent_id, _prompt_id) =
            runtime_with_active_prompt(crate::session::DurablePromptDeliveryPhase::Dispatching);
        let provider_run_id = runtime
            .owned
            .provider_store
            .get_run_for_agent(&session_id, &agent_id)
            .expect("provider run should exist")
            .id()
            .to_string();
        let observer_attachment_id = runtime
            .with_app_side_effect(|app| {
                crate::app::KernelSessionService::new(app)
                    .attach(AttachRequest::new(
                        &session_id,
                        "attachment-restart-recovery-observer",
                        ClientCapabilityLevel::FullTerminal,
                    ))
                    .expect("observer attachment should attach")
                    .id()
                    .to_string()
            })
            .await;
        let operation_id = "chariox-recovery:prompt-hidden:1";
        let recovery_source_attachment =
            format!("{KERNEL_RECOVERY_ATTACHMENT_PREFIX}{operation_id}");
        let recovery_prompt = provider_restart_continuation_prompt(operation_id);

        runtime.owned.echo_prompt_to_other_attachments(
            &session_id,
            &provider_run_id,
            "prompt-hidden",
            &recovery_source_attachment,
            &recovery_prompt,
            &[],
        );

        let leaked_records: Vec<_> = runtime
            .owned
            .terminal_stream
            .output_records()
            .into_iter()
            .filter(|record| {
                record
                    .recipient_attachment_ids
                    .iter()
                    .any(|id| id == &observer_attachment_id)
                    && String::from_utf8_lossy(&record.bytes).contains(operation_id)
            })
            .collect();
        assert!(
            leaked_records.is_empty(),
            "kernel-recovery envelope must never be echoed to other attachments; leaked records = {leaked_records:#?}",
        );
    }
}
