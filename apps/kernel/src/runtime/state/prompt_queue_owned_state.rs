//! Prompt queue mirror and advancement mutations.
//!
//! This module owns synchronizing prompt-owner state back into sessions and advancing queued
//! prompts onto an existing provider run.

use super::*;

pub(super) struct RemoteQueuedPromptSteerPreparation {
    pub(super) agent: crate::agent::AgentInstance,
    pub(super) remote_execution: crate::agent::RemoteAgentBinding,
    pub(super) target_active_prompt_id: String,
    pub(super) prompt: crate::session::PromptQueueItem,
}

struct QueuedPromptSteerContext {
    session: crate::session::RuntimeSession,
    agent: crate::agent::AgentInstance,
    active_prompt: crate::session::PromptQueueItem,
    provider_run_id: String,
    queued_prompt: crate::session::PromptQueueItem,
}

impl KernelRuntimeOwnedState {
    pub(super) fn provider_account_allows_queued_prompt_advance(
        &self,
        session_id: &str,
        agent: &crate::agent::AgentInstance,
        operation: &'static str,
    ) -> bool {
        let error = self.provider_account_profiles.require_agent_authenticated(
            &self.config_projection.snapshot(),
            agent,
            operation,
        );
        let Err(error) = error else {
            return true;
        };
        crate::logging::warn_with_fields(
            "daemon.prompt_queue",
            "deferred queued prompt because its provider account is unavailable",
            serde_json::json!({
                "session_id": session_id,
                "agent_id": agent.id(),
                "error": error.to_string(),
            }),
        );
        false
    }

    pub(super) fn prompt_source_attribution(
        &self,
        prompt: &crate::session::PromptQueueItem,
    ) -> (Option<String>, Option<String>) {
        let source_attachment = self
            .attachment_store
            .get_attachment(prompt.source_attachment_id())
            .ok();
        let source_client_id = prompt.source_client_id().map(str::to_string).or_else(|| {
            source_attachment
                .as_ref()
                .map(|attachment| attachment.client_id().to_string())
        });
        let source_user_id = prompt.source_user_id().map(str::to_string).or_else(|| {
            source_attachment
                .as_ref()
                .map(|attachment| attachment.owner_user_id().to_string())
        });
        (source_client_id, source_user_id)
    }

    pub(super) fn active_prompt_source_attribution(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(Option<String>, Option<String>), DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let prompt = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        Ok(self.prompt_source_attribution(&prompt))
    }

    fn ensure_queued_prompt_manually_mutable(
        &self,
        session: &crate::session::RuntimeSession,
        agent_id: &str,
        prompt_id: &str,
        operation: &'static str,
        action: &str,
    ) -> Result<(), DaemonError> {
        let (_, queued_prompts) = self.prompt_state_owner.state_parts(session, agent_id);
        let prompt = queued_prompts
            .iter()
            .find(|prompt| prompt.id() == prompt_id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation,
                message: format!(
                    "queued prompt `{prompt_id}` was not found for agent `{agent_id}`"
                ),
            })?;
        if prompt.workflow_run_id().is_some() {
            return Err(DaemonError::LocalTransport {
                operation,
                message: format!("workflow queued prompts cannot be {action} manually"),
            });
        }
        Ok(())
    }

    pub(super) fn promoted_prompt_source_attachment_id(
        &self,
        session_id: &str,
        source_attachment_id: &str,
    ) -> Result<String, DaemonError> {
        let _ = self.session_store.get_session(session_id)?;
        Ok(source_attachment_id.to_string())
    }

    pub(super) fn mirror_prompt_owner_agent_state(
        &self,
        session_id: &str,
        agent_id: &str,
        active_prompt: Option<crate::session::PromptQueueItem>,
        queued_prompts: std::collections::VecDeque<crate::session::PromptQueueItem>,
    ) -> Result<(), DaemonError> {
        let session = self.session_store.mirror_agent_prompt_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        )?;
        self.persist_prompt_session_state(&session, agent_id)?;
        self.provider_process_projection.invalidate();
        let _ = self.session_snapshot(session_id)?;
        Ok(())
    }

    pub(super) fn persist_prompt_session_state(
        &self,
        session: &crate::session::RuntimeSession,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        crate::durable_prompt_state::append_durable_prompt_state_event(
            &self.durable_state_store,
            session,
            agent_id,
        )
    }

    pub(super) fn mark_active_prompt_delivery(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        phase: crate::session::DurablePromptDeliveryPhase,
        provider_run_id: Option<String>,
        provider_session_id: Option<String>,
    ) -> Result<crate::session::PromptQueueItem, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let previous = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let prompt = self.prompt_state_owner.mark_active_prompt_delivery(
            &session,
            agent_id,
            prompt_id,
            phase,
            provider_run_id,
            provider_session_id,
        )?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        if let Err(error) = self.mirror_prompt_owner_agent_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        ) {
            let _ = self
                .prompt_state_owner
                .replace_active_prompt_if_matches(&session, agent_id, &prompt, previous);
            let (active_prompt, queued_prompts) =
                self.prompt_state_owner.state_parts(&session, agent_id);
            self.session_store.mirror_agent_prompt_state(
                session_id,
                agent_id,
                active_prompt,
                queued_prompts,
            )?;
            return Err(error);
        }
        Ok(prompt)
    }

    pub(super) fn compare_and_mark_active_prompt_delivery_failure(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        provider_run_id: &str,
        provider_session_id: &str,
        status_transition: (crate::session::PromptStatus, crate::session::PromptStatus),
    ) -> Result<Option<crate::session::PromptQueueItem>, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let prompt = self
            .prompt_state_owner
            .compare_and_mark_active_prompt_delivery_failure(
                &session,
                agent_id,
                prompt_id,
                provider_run_id,
                provider_session_id,
                status_transition,
            );
        if prompt.is_none() {
            return Ok(None);
        }
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        if let Err(error) = self.mirror_prompt_owner_agent_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        ) {
            let _ = self
                .prompt_state_owner
                .compare_and_mark_active_prompt_delivery_failure(
                    &session,
                    agent_id,
                    prompt_id,
                    provider_run_id,
                    provider_session_id,
                    (status_transition.1, status_transition.0),
                );
            let (active_prompt, queued_prompts) =
                self.prompt_state_owner.state_parts(&session, agent_id);
            self.session_store.mirror_agent_prompt_state(
                session_id,
                agent_id,
                active_prompt,
                queued_prompts,
            )?;
            return Err(error);
        }
        Ok(prompt)
    }

    pub(super) fn restore_active_prompt_after_resume_superseded(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        provider_run_id: &str,
        failed_provider_session_id: &str,
        current_provider_session_id: &str,
    ) -> Result<Option<crate::session::PromptQueueItem>, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let Some((previous, restored)) = self
            .prompt_state_owner
            .compare_and_restore_active_prompt_after_resume_superseded(
                &session,
                agent_id,
                prompt_id,
                provider_run_id,
                failed_provider_session_id,
                current_provider_session_id,
            )
        else {
            return Ok(None);
        };
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        if let Err(error) = self.mirror_prompt_owner_agent_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        ) {
            let _ = self
                .prompt_state_owner
                .replace_active_prompt_if_matches(&session, agent_id, &restored, previous);
            let (active_prompt, queued_prompts) =
                self.prompt_state_owner.state_parts(&session, agent_id);
            self.session_store.mirror_agent_prompt_state(
                session_id,
                agent_id,
                active_prompt,
                queued_prompts,
            )?;
            return Err(error);
        }
        Ok(Some(restored))
    }

    pub(super) fn begin_active_prompt_recovery(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
    ) -> Result<crate::session::PromptQueueItem, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let prompt = self
            .prompt_state_owner
            .begin_active_prompt_recovery(&session, agent_id, prompt_id)?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        Ok(prompt)
    }

    pub(super) fn mark_active_prompt_recovery_phase(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        operation_id: &str,
        phase: crate::session::DurablePromptDeliveryPhase,
    ) -> Result<crate::session::PromptQueueItem, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let prompt = self.prompt_state_owner.mark_active_prompt_recovery_phase(
            &session,
            agent_id,
            prompt_id,
            operation_id,
            phase,
        )?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        Ok(prompt)
    }

    pub(super) fn compare_and_mark_active_prompt_recovery_phase(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        operation_id: &str,
        expected_phase: crate::session::DurablePromptDeliveryPhase,
        next_phase: crate::session::DurablePromptDeliveryPhase,
    ) -> Result<Option<crate::session::PromptQueueItem>, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let prompt = self
            .prompt_state_owner
            .compare_and_mark_active_prompt_recovery_phase(
                &session,
                agent_id,
                prompt_id,
                operation_id,
                expected_phase,
                next_phase,
            )?;
        if prompt.is_some() {
            let (active_prompt, queued_prompts) =
                self.prompt_state_owner.state_parts(&session, agent_id);
            if let Err(error) = self.mirror_prompt_owner_agent_state(
                session_id,
                agent_id,
                active_prompt,
                queued_prompts,
            ) {
                // The durable append is the commit boundary for this transition. Restore the
                // owner state only while it still contains our Accepted phase; a concurrent
                // delivery acknowledgement must win. Then repair the in-memory session mirror
                // from whichever owner state is current without attempting a second durable
                // write. The failed transaction left the durable phase at `expected_phase`.
                let _ = self
                    .prompt_state_owner
                    .compare_and_mark_active_prompt_recovery_phase(
                        &session,
                        agent_id,
                        prompt_id,
                        operation_id,
                        next_phase,
                        expected_phase,
                    );
                let (active_prompt, queued_prompts) =
                    self.prompt_state_owner.state_parts(&session, agent_id);
                self.session_store.mirror_agent_prompt_state(
                    session_id,
                    agent_id,
                    active_prompt,
                    queued_prompts,
                )?;
                return Err(error);
            }
        }
        Ok(prompt)
    }

    pub(super) fn mirror_prompt_owner_session_state(
        &self,
        session_id: &str,
    ) -> Result<(), DaemonError> {
        let mut agent_ids = self
            .agent_store
            .get_session_agents(session_id)
            .into_iter()
            .map(|agent| agent.id().to_string())
            .collect::<Vec<_>>();
        let session = self.session_store.get_session(session_id)?;
        agent_ids.extend(session.prompt_states().keys().cloned());
        agent_ids.sort();
        agent_ids.dedup();
        for agent_id in agent_ids {
            let (active_prompt, queued_prompts) =
                self.prompt_state_owner.state_parts(&session, &agent_id);
            self.mirror_prompt_owner_agent_state(
                session_id,
                &agent_id,
                active_prompt,
                queued_prompts,
            )?;
        }
        Ok(())
    }

    pub(super) fn remove_queued_prompts_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<usize, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let removed = self
            .prompt_state_owner
            .remove_queued_prompts_for_agent(&session, agent_id);
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        Ok(removed)
    }

    pub(super) fn take_queued_workflow_prompts_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Vec<crate::session::PromptQueueItem>, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let (_, queued_prompts) = self.prompt_state_owner.state_parts(&session, agent_id);
        let workflow_prompts = queued_prompts
            .into_iter()
            .filter(|prompt| prompt.workflow_run_id().is_some())
            .collect::<Vec<_>>();
        if workflow_prompts.is_empty() {
            return Ok(workflow_prompts);
        }
        for prompt in &workflow_prompts {
            let _ = self
                .prompt_state_owner
                .remove_queued_prompt(&session, agent_id, prompt.id());
        }
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        Ok(workflow_prompts)
    }

    pub(super) fn remove_queued_metaagent_event_prompts_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        source_attachment_id: &str,
    ) -> Result<usize, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let removed = self
            .prompt_state_owner
            .remove_queued_metaagent_event_prompts_for_agent(
                &session,
                agent_id,
                source_attachment_id,
            );
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        Ok(removed)
    }

    pub(super) fn activate_next_queued_prompt_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        expected_prompt_id: Option<&str>,
    ) -> Result<Option<crate::session::PromptQueueItem>, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        if self
            .prompt_state_owner
            .peek_next_queued_prompt(&session, agent_id)
            .is_none()
        {
            return Ok(None);
        }
        let agent = self.agent_store.get_agent(agent_id)?;
        if !self.provider_account_allows_queued_prompt_advance(
            session_id,
            &agent,
            "activate queued prompt",
        ) {
            return Ok(None);
        }
        let prompt = self
            .prompt_state_owner
            .activate_next_queued_prompt_with_prompt_id(
                &session,
                agent_id,
                expected_prompt_id,
                self.session_store.reserve_prompt_id(),
            )?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        Ok(prompt)
    }

    pub(super) fn advance_next_queued_prompt_dispatch(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
    ) -> Result<Option<crate::app::KernelPromptDispatch>, DaemonError> {
        let (session, next_prompt) = loop {
            let session = self.session_store.get_session(session_id)?;
            let Some(next_prompt) = self
                .prompt_state_owner
                .peek_next_queued_prompt(&session, agent_id)
            else {
                return Ok(None);
            };
            let workflow_is_runnable =
                next_prompt.workflow_run_id().is_none_or(|workflow_run_id| {
                    session
                        .workflow_runs()
                        .iter()
                        .find(|workflow_run| workflow_run.id() == workflow_run_id)
                        .is_some_and(|workflow_run| {
                            matches!(
                                workflow_run.status(),
                                crate::session::WorkflowRunStatus::Created
                                    | crate::session::WorkflowRunStatus::Running
                                    | crate::session::WorkflowRunStatus::Waiting
                            )
                        })
                });
            if workflow_is_runnable {
                break (session, next_prompt);
            }
            let removed =
                self.prompt_state_owner
                    .remove_queued_prompt(&session, agent_id, next_prompt.id());
            if removed.is_none() {
                continue;
            }
            let (active_prompt, queued_prompts) =
                self.prompt_state_owner.state_parts(&session, agent_id);
            self.mirror_prompt_owner_agent_state(
                session_id,
                agent_id,
                active_prompt,
                queued_prompts,
            )?;
        };
        let agent = self.agent_store.get_agent(agent_id)?;
        if !self.provider_account_allows_queued_prompt_advance(
            session_id,
            &agent,
            "advance queued prompt",
        ) {
            return Ok(None);
        }
        if self
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .is_some_and(|prompt| prompt.is_external())
        {
            return Ok(None);
        }
        let provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;
        if provider_run.state() != crate::provider::ProviderRunState::Running {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: provider_run_id.to_string(),
                state: provider_run.state(),
                operation: "advance queued prompt",
            });
        }
        let acquired_workflow_claim =
            match self.ensure_workflow_prompt_workspace_claim(session_id, &next_prompt) {
                Ok(acquired) => acquired,
                Err(DaemonError::WorkspaceClaimConflict { .. }) => return Ok(None),
                Err(error) => return Err(error),
            };
        let started_next = self
            .prompt_state_owner
            .activate_next_queued_prompt_with_prompt_id(
                &session,
                agent_id,
                Some(next_prompt.id()),
                self.session_store.reserve_prompt_id(),
            );
        let started_next = match started_next {
            Ok(Some(prompt)) => prompt,
            Ok(None) => {
                if acquired_workflow_claim == Some(true) {
                    self.release_workflow_node_workspace_claim(
                        session_id,
                        next_prompt.workflow_run_id().unwrap_or_default(),
                        next_prompt.workflow_node_run_id().unwrap_or_default(),
                    );
                }
                return Err(DaemonError::LocalTransport {
                    operation: "advance queued prompt",
                    message: format!(
                        "expected queued prompt `{}` but no queued prompt was available",
                        next_prompt.id()
                    ),
                });
            }
            Err(error) => {
                if acquired_workflow_claim == Some(true) {
                    self.release_workflow_node_workspace_claim(
                        session_id,
                        next_prompt.workflow_run_id().unwrap_or_default(),
                        next_prompt.workflow_node_run_id().unwrap_or_default(),
                    );
                }
                return Err(error);
            }
        };
        let source_attachment_id = self.promoted_prompt_source_attachment_id(
            session_id,
            started_next.source_attachment_id(),
        )?;
        let prompt_sent_at_ms =
            self.record_started_user_prompt(session_id, &source_attachment_id, &started_next)?;
        self.echo_promoted_queued_prompt_to_attachments(
            session_id,
            provider_run_id,
            started_next.id(),
            &source_attachment_id,
            started_next.prompt(),
            started_next.attachments(),
        );
        self.capture_git_turn_snapshot_for_started_prompt(
            &session,
            agent_id,
            &provider_run,
            &started_next,
            Some(prompt_sent_at_ms),
        );
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        self.workflow_mark_prompt_started(session_id, &started_next)?;
        let _ = self.session_snapshot(session_id)?;
        Ok(Some(crate::app::KernelPromptDispatch {
            session_id: session_id.to_string(),
            provider_run_id: provider_run_id.to_string(),
            agent_id: agent_id.to_string(),
            prompt_id: started_next.id().to_string(),
            target_active_prompt_id: None,
            source_attachment_id,
            prompt: started_next.prompt().to_string(),
            hidden_system_context: started_next.hidden_system_context().to_string(),
            attachments: started_next.attachments().to_vec(),
            prompt_origin: started_next.prompt_origin(),
            external_provider: started_next.external_provider().map(str::to_string),
            external_provider_session_id: started_next
                .external_provider_session_id()
                .map(str::to_string),
            external_provider_turn_id: started_next.external_provider_turn_id().map(str::to_string),
            steering: false,
        }))
    }

    fn queued_prompt_steer_context(
        &self,
        session_id: &str,
        agent_id: &str,
        attachment_id: &str,
        prompt_id: &str,
    ) -> Result<QueuedPromptSteerContext, DaemonError> {
        let _ = self.ensure_attachment_in_session(session_id, attachment_id)?;
        let target_agent = self.agent_store.get_agent(agent_id)?;
        if target_agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        let session = self.session_store.get_session(session_id)?;
        let Some(active_prompt) = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
        else {
            return Err(DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            });
        };
        if active_prompt.is_external() {
            return Err(DaemonError::LocalTransport {
                operation: "steer queued prompt",
                message: "queued prompts cannot be steered into externally started provider turns"
                    .to_string(),
            });
        }
        let provider_run_id = if let Some(remote_execution) = target_agent.remote_execution() {
            if let Some(provider_run) = self
                .provider_run_projection
                .get_for_agent(session_id, agent_id)
            {
                if provider_run.session_id() != session_id {
                    return Err(DaemonError::ProviderRunNotInSession {
                        session_id: session_id.to_string(),
                        provider_run_id: provider_run.id().to_string(),
                    });
                }
                if provider_run.state() != crate::provider::ProviderRunState::Running {
                    return Err(DaemonError::InvalidProviderRunState {
                        provider_run_id: provider_run.id().to_string(),
                        state: provider_run.state(),
                        operation: "steer queued prompt",
                    });
                }
            }
            remote_execution
                .active_worker_provider_run_id
                .clone()
                .ok_or_else(|| DaemonError::NoActiveProviderRun {
                    session_id: session_id.to_string(),
                })?
        } else {
            let provider_run = self
                .provider_store
                .get_run_for_agent(session_id, agent_id)
                .ok_or_else(|| DaemonError::NoActiveProviderRun {
                    session_id: session_id.to_string(),
                })?;
            let provider_run =
                self.ensure_provider_run_in_session(session_id, provider_run.id())?;
            if provider_run.state() != crate::provider::ProviderRunState::Running {
                return Err(DaemonError::InvalidProviderRunState {
                    provider_run_id: provider_run.id().to_string(),
                    state: provider_run.state(),
                    operation: "steer queued prompt",
                });
            }
            provider_run.id().to_string()
        };
        let (_, queued_prompts) = self.prompt_state_owner.state_parts(&session, agent_id);
        let queued_prompt = queued_prompts
            .iter()
            .find(|prompt| prompt.id() == prompt_id)
            .cloned()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "steer queued prompt",
                message: format!(
                    "queued prompt `{prompt_id}` was not found for agent `{agent_id}`"
                ),
            })?;
        if queued_prompt.workflow_run_id().is_some() {
            return Err(DaemonError::LocalTransport {
                operation: "steer queued prompt",
                message: "workflow queued prompts cannot be steered manually".to_string(),
            });
        }
        Ok(QueuedPromptSteerContext {
            session,
            agent: target_agent,
            active_prompt,
            provider_run_id,
            queued_prompt,
        })
    }

    pub(super) fn prepare_remote_queued_prompt_steer(
        &self,
        session_id: &str,
        agent_id: &str,
        attachment_id: &str,
        prompt_id: &str,
    ) -> Result<Option<RemoteQueuedPromptSteerPreparation>, DaemonError> {
        let context =
            self.queued_prompt_steer_context(session_id, agent_id, attachment_id, prompt_id)?;
        let Some(remote_execution) = context.agent.remote_execution().cloned() else {
            return Ok(None);
        };
        Ok(Some(RemoteQueuedPromptSteerPreparation {
            agent: context.agent,
            remote_execution,
            target_active_prompt_id: context.active_prompt.id().to_string(),
            prompt: context.queued_prompt,
        }))
    }

    fn commit_queued_prompt_steer(
        &self,
        session_id: &str,
        agent_id: &str,
        attachment_id: &str,
        prompt_id: &str,
        context: QueuedPromptSteerContext,
        dispatch_locally: bool,
    ) -> Result<crate::app::KernelQueuedPromptSteer, DaemonError> {
        let target_active_prompt_id = context.active_prompt.id().to_string();
        let source_attachment_id = self.promoted_prompt_source_attachment_id(
            session_id,
            context.queued_prompt.source_attachment_id(),
        )?;
        self.append_steering_prompt_history(
            session_id,
            &context.provider_run_id,
            &target_active_prompt_id,
            &source_attachment_id,
            agent_id,
            context.queued_prompt.id(),
            context.queued_prompt.prompt(),
            context.queued_prompt.attachments(),
        )?;
        let prompt = self
            .prompt_state_owner
            .remove_queued_prompt(&context.session, agent_id, prompt_id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "steer queued prompt",
                message: format!(
                    "queued prompt `{prompt_id}` was not found for agent `{agent_id}`"
                ),
            })?;
        let (active_prompt, queued_prompts) = self
            .prompt_state_owner
            .state_parts(&context.session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        self.echo_steering_prompt_to_other_attachments(
            session_id,
            &context.provider_run_id,
            agent_id,
            prompt.id(),
            &source_attachment_id,
            attachment_id,
            prompt.prompt(),
            prompt.attachments(),
            prompt.prompt_origin(),
        );
        self.record_notice(
            session_id,
            Some(&context.provider_run_id),
            self.other_attachment_ids(session_id, attachment_id),
            format!(
                "Attachment `{}` steered queued prompt `{}` to agent `{}`.",
                attachment_id,
                prompt.id(),
                agent_id
            ),
        );
        let session = self.session_snapshot(session_id)?;
        let dispatch = dispatch_locally.then(|| crate::app::KernelPromptDispatch {
            session_id: session_id.to_string(),
            provider_run_id: context.provider_run_id,
            agent_id: agent_id.to_string(),
            prompt_id: prompt.id().to_string(),
            target_active_prompt_id: Some(target_active_prompt_id),
            source_attachment_id,
            prompt: prompt.prompt().to_string(),
            hidden_system_context: prompt.hidden_system_context().to_string(),
            attachments: prompt.attachments().to_vec(),
            prompt_origin: prompt.prompt_origin(),
            external_provider: prompt.external_provider().map(str::to_string),
            external_provider_session_id: prompt.external_provider_session_id().map(str::to_string),
            external_provider_turn_id: prompt.external_provider_turn_id().map(str::to_string),
            steering: true,
        });
        Ok(crate::app::KernelQueuedPromptSteer {
            dispatch,
            prompt,
            session,
        })
    }

    pub(super) fn steer_queued_prompt(
        &self,
        session_id: &str,
        agent_id: &str,
        attachment_id: &str,
        prompt_id: &str,
    ) -> Result<Option<crate::app::KernelQueuedPromptSteer>, DaemonError> {
        let context =
            self.queued_prompt_steer_context(session_id, agent_id, attachment_id, prompt_id)?;
        if context.agent.remote_execution().is_some() {
            return Ok(None);
        }
        self.commit_queued_prompt_steer(
            session_id,
            agent_id,
            attachment_id,
            prompt_id,
            context,
            true,
        )
        .map(Some)
    }

    pub(super) fn finish_remote_queued_prompt_steer(
        &self,
        session_id: &str,
        agent_id: &str,
        attachment_id: &str,
        prompt_id: &str,
        target_active_prompt_id: &str,
    ) -> Result<crate::app::KernelQueuedPromptSteer, DaemonError> {
        let context =
            self.queued_prompt_steer_context(session_id, agent_id, attachment_id, prompt_id)?;
        if context.agent.remote_execution().is_none() {
            return Err(DaemonError::LocalTransport {
                operation: "steer remote queued prompt",
                message: format!("agent `{agent_id}` is no longer remote"),
            });
        }
        if context.active_prompt.id() != target_active_prompt_id {
            return Err(DaemonError::LocalTransport {
                operation: "steer remote queued prompt",
                message: format!(
                    "active prompt changed from `{target_active_prompt_id}` to `{}` before the remote steer committed",
                    context.active_prompt.id()
                ),
            });
        }
        self.commit_queued_prompt_steer(
            session_id,
            agent_id,
            attachment_id,
            prompt_id,
            context,
            false,
        )
    }

    pub(super) fn cancel_queued_prompt(
        &self,
        session_id: &str,
        agent_id: &str,
        attachment_id: &str,
        prompt_id: &str,
    ) -> Result<crate::app::KernelQueuedPromptCancellation, DaemonError> {
        let _ = self.ensure_attachment_in_session(session_id, attachment_id)?;
        let target_agent = self.agent_store.get_agent(agent_id)?;
        if target_agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        let session = self.session_store.get_session(session_id)?;
        self.ensure_queued_prompt_manually_mutable(
            &session,
            agent_id,
            prompt_id,
            "cancel queued prompt",
            "cancelled",
        )?;
        let mut prompt = self
            .prompt_state_owner
            .remove_queued_prompt(&session, agent_id, prompt_id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "cancel queued prompt",
                message: format!(
                    "queued prompt `{prompt_id}` was not found for agent `{agent_id}`"
                ),
            })?;
        prompt.set_status(crate::session::PromptStatus::Cancelled);
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        self.record_notice(
            session_id,
            None,
            self.other_attachment_ids(session_id, attachment_id),
            format!(
                "Attachment `{}` cancelled queued prompt `{}` for agent `{}`.",
                attachment_id,
                prompt.id(),
                agent_id
            ),
        );
        let session = self.session_snapshot(session_id)?;
        Ok(crate::app::KernelQueuedPromptCancellation { prompt, session })
    }

    pub(super) fn update_queued_prompt(
        &self,
        session_id: &str,
        agent_id: &str,
        attachment_id: &str,
        prompt_id: &str,
        prompt_text: &str,
    ) -> Result<crate::app::KernelQueuedPromptUpdate, DaemonError> {
        let _ = self.ensure_attachment_in_session(session_id, attachment_id)?;
        let target_agent = self.agent_store.get_agent(agent_id)?;
        if target_agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        let session = self.session_store.get_session(session_id)?;
        self.ensure_queued_prompt_manually_mutable(
            &session,
            agent_id,
            prompt_id,
            "update queued prompt",
            "updated",
        )?;
        let prompt = self
            .prompt_state_owner
            .update_queued_prompt(&session, agent_id, prompt_id, prompt_text)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "update queued prompt",
                message: format!(
                    "queued prompt `{prompt_id}` was not found for agent `{agent_id}`"
                ),
            })?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        self.record_notice(
            session_id,
            None,
            self.other_attachment_ids(session_id, attachment_id),
            format!(
                "Attachment `{}` updated queued prompt `{}` for agent `{}`.",
                attachment_id,
                prompt.id(),
                agent_id
            ),
        );
        let session = self.session_snapshot(session_id)?;
        Ok(crate::app::KernelQueuedPromptUpdate { prompt, session })
    }
}
