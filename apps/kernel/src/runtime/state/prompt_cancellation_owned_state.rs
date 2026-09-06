//! Local prompt cancellation state transitions.

use super::owned::OwnedPromptCancellation;
use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn cancel_active_prompt_only(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<crate::session::PromptQueueItem, DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        let session = self.session_store.get_session(session_id)?;
        let cancelled = self
            .prompt_state_owner
            .cancel_active_prompt_only(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let provider_run_id = self
            .provider_store
            .get_run_for_agent(session_id, agent_id)
            .map(|run| run.id().to_string());
        self.record_cancelled_prompt_settlement(
            session_id,
            agent_id,
            &cancelled,
            provider_run_id.as_deref(),
        );
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        Ok(cancelled)
    }

    pub(super) fn finalize_local_prompt_cancellation_with_queued_advance(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: Option<&str>,
    ) -> Result<OwnedPromptCancellation, DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        let session = self.session_store.get_session(session_id)?;
        let prompt = self
            .prompt_state_owner
            .finalize_active_prompt_cancellation(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let cancellation_provider_run_id = provider_run_id.map(str::to_string).or_else(|| {
            self.provider_store
                .get_run_for_agent(session_id, agent_id)
                .map(|run| run.id().to_string())
        });
        self.record_cancelled_prompt_settlement(
            session_id,
            agent_id,
            &prompt,
            cancellation_provider_run_id.as_deref(),
        );
        let released_workflow_claim =
            match (prompt.workflow_run_id(), prompt.workflow_node_run_id()) {
                (Some(workflow_run_id), Some(workflow_node_run_id)) => self
                    .release_workflow_node_workspace_claim(
                        session_id,
                        workflow_run_id,
                        workflow_node_run_id,
                    ),
                _ => false,
            };
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        let released_claim = cancellation_provider_run_id
            .as_deref()
            .map(|provider_run_id| self.clear_prompt_activity(provider_run_id))
            .unwrap_or(false)
            || released_workflow_claim;
        let current_session = self.session_store.get_session(session_id)?;
        let hold_queued_prompts = current_session
            .metaagent_task(agent_id)
            .is_some_and(|task| task.status() == crate::session::MetaagentTaskStatus::Paused)
            && self
                .prompt_state_owner
                .peek_next_queued_prompt(&current_session, agent_id)
                .is_some_and(|prompt| {
                    prompt.prompt()
                        == crate::scheduler::prompt_injection::METAAGENT_EVENT_VISIBLE_PROMPT
                });
        let provider_account_available = if self
            .prompt_state_owner
            .peek_next_queued_prompt(&current_session, agent_id)
            .is_none()
        {
            true
        } else {
            self.provider_account_allows_queued_prompt_advance(
                session_id,
                &agent,
                "advance queued prompt after cancellation",
            )
        };
        let started_next = if provider_account_available
            && !hold_queued_prompts
            && self
                .prompt_state_owner
                .active_prompt_for_agent(&self.session_store.get_session(session_id)?, agent_id)
                .is_none()
        {
            let next_prompt = self
                .prompt_state_owner
                .peek_next_queued_prompt(&self.session_store.get_session(session_id)?, agent_id);
            if let (Some(provider_run_id), Some(next_prompt)) = (
                cancellation_provider_run_id.as_deref(),
                next_prompt.as_ref(),
            ) {
                let provider_run =
                    self.ensure_provider_run_in_session(session_id, provider_run_id)?;
                if provider_run.state() == crate::provider::ProviderRunState::Running {
                    let acquired_workflow_claim = match self
                        .ensure_workflow_prompt_workspace_claim(session_id, next_prompt)
                    {
                        Ok(acquired) => acquired,
                        Err(DaemonError::WorkspaceClaimConflict { .. }) => None,
                        Err(error) => return Err(error),
                    };
                    if next_prompt.workflow_run_id().is_some() && acquired_workflow_claim.is_none()
                    {
                        return Ok(OwnedPromptCancellation {
                            cancellation: crate::session::PromptCancellation {
                                prompt,
                                started_next: None,
                            },
                            released_claim,
                            dispatch: None,
                        });
                    }
                    let started_next = self
                        .prompt_state_owner
                        .activate_next_queued_prompt_with_prompt_id(
                            &self.session_store.get_session(session_id)?,
                            agent_id,
                            Some(next_prompt.id()),
                            self.session_store.reserve_prompt_id(),
                        )?;
                    if let Some(started_next) = started_next.as_ref() {
                        let source_attachment_id = self.promoted_prompt_source_attachment_id(
                            session_id,
                            started_next.source_attachment_id(),
                        )?;
                        let prompt_sent_at_ms = self.record_started_user_prompt(
                            session_id,
                            &source_attachment_id,
                            started_next,
                        )?;
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
                            started_next,
                            Some(prompt_sent_at_ms),
                        );
                    } else if acquired_workflow_claim == Some(true) {
                        self.release_workflow_node_workspace_claim(
                            session_id,
                            next_prompt.workflow_run_id().unwrap_or_default(),
                            next_prompt.workflow_node_run_id().unwrap_or_default(),
                        );
                    }
                    started_next
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        if let Some(started_next) = started_next.as_ref() {
            self.workflow_mark_prompt_started(session_id, started_next)?;
        }
        let (active_prompt, queued_prompts) = self
            .prompt_state_owner
            .state_parts(&self.session_store.get_session(session_id)?, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        if started_next.is_none() {
            self.sync_focused_provider_run_if_idle(session_id)?;
        }
        let dispatch = if let (Some(provider_run_id), Some(started_next)) = (
            cancellation_provider_run_id.as_deref(),
            started_next.as_ref(),
        ) {
            let provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;
            let source_attachment_id = self.promoted_prompt_source_attachment_id(
                session_id,
                started_next.source_attachment_id(),
            )?;
            if self
                .provider_store
                .run_uses_structured_prompt_io(&provider_run)
            {
                let prompt_with_handoff = self.prompt_with_pending_context_handoff(
                    session_id,
                    agent_id,
                    &source_attachment_id,
                    &provider_run,
                    started_next.prompt(),
                );
                let granted_skill_context =
                    self.granted_skill_hidden_context(session_id, agent_id, &prompt_with_handoff)?;
                let hidden_system_context = join_hidden_context(
                    started_next.hidden_system_context(),
                    &granted_skill_context,
                );
                let (source_client_id, _source_user_id) =
                    self.prompt_source_attribution(started_next);
                let mode = crate::prompt_assembly::provider_turn_mode_for_prompt(
                    agent_id,
                    self.agent_store.get_agent(agent_id)?.is_metaagent(),
                    source_client_id.as_deref(),
                    &hidden_system_context,
                );
                self.mark_active_prompt_delivery(
                    session_id,
                    agent_id,
                    started_next.id(),
                    crate::session::DurablePromptDeliveryPhase::Dispatching,
                    Some(provider_run_id.to_string()),
                    provider_run.provider_session_id().map(str::to_string),
                )?;
                self.provider_store.enqueue_structured_prompt_submit(
                    session_id.to_string(),
                    provider_run_id.to_string(),
                    agent_id.to_string(),
                    started_next.id().to_string(),
                    &provider_run,
                    &prompt_with_handoff,
                    &hidden_system_context,
                    started_next.attachments(),
                    mode,
                    false,
                )?;
                self.consume_pending_context_handoff(session_id, agent_id, &provider_run);
                self.note_prompt_started(provider_run_id);
                None
            } else {
                Some(crate::app::KernelPromptDispatch {
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
                    external_provider_turn_id: started_next
                        .external_provider_turn_id()
                        .map(str::to_string),
                    steering: false,
                })
            }
        } else {
            None
        };
        let _ = self.session_snapshot(session_id)?;
        Ok(OwnedPromptCancellation {
            cancellation: crate::session::PromptCancellation {
                prompt,
                started_next,
            },
            released_claim,
            dispatch,
        })
    }

    fn record_cancelled_prompt_settlement(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt: &crate::session::PromptQueueItem,
        provider_run_id: Option<&str>,
    ) {
        let settled_at_ms = crate::session::unix_epoch_ms();
        let archive_enabled = self
            .config_projection
            .snapshot()
            .user_config
            .history
            .archive
            .mode
            == crate::config::HistoryArchiveMode::External;
        self.operational_history_store.record_prompt_settlement(
            archive_enabled,
            session_id,
            agent_id,
            prompt.id(),
            provider_run_id,
            settled_at_ms,
            "cancelled",
        );
        self.completed_git_turn_snapshots.record_prompt_settlement(
            session_id,
            agent_id,
            provider_run_id.unwrap_or("provider-run-cancelled"),
            prompt,
            settled_at_ms,
            Some(prompt.created_at_ms()),
            crate::git_observer::CompletedTurnSettlementStatus::Cancelled,
        );
    }

    pub(super) fn cancel_local_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
    ) -> Result<Option<crate::app::KernelPromptCancellation>, DaemonError> {
        if !crate::scheduler::runtime::is_workflow_prompt_attachment(attachment_id) {
            let _ = self.ensure_attachment_in_session(session_id, attachment_id)?;
        }
        let target_agent = self.agent_store.get_agent(target_agent_id)?;
        if target_agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: target_agent_id.to_string(),
            });
        }
        if target_agent.remote_execution().is_some() {
            return Ok(None);
        }
        let session = self.session_store.get_session(session_id)?;
        let active_prompt = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, target_agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        if active_prompt.status() == crate::session::PromptStatus::Cancelling {
            let session = self.session_snapshot(session_id)?;
            return Ok(Some(crate::app::KernelPromptCancellation {
                cancellation: crate::session::PromptCancellation {
                    prompt: active_prompt,
                    started_next: None,
                },
                session,
                dispatch: None,
            }));
        }

        let provider_run = self
            .provider_run_projection
            .get_for_agent(session_id, target_agent_id)
            .or_else(|| {
                self.provider_store
                    .get_run_for_agent(session_id, target_agent_id)
            })
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?;
        let provider_run = self.ensure_provider_run_in_session(session_id, provider_run.id())?;

        let prompt = self
            .prompt_state_owner
            .begin_cancelling_active_prompt(&session, target_agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let (active_prompt, queued_prompts) = self
            .prompt_state_owner
            .state_parts(&session, target_agent_id);
        self.mirror_prompt_owner_agent_state(
            session_id,
            target_agent_id,
            active_prompt,
            queued_prompts,
        )?;
        self.note_prompt_settlement_requested(provider_run.id());
        let recipients = self.other_attachment_ids(session_id, attachment_id);
        self.record_notice(
            session_id,
            Some(provider_run.id()),
            recipients,
            format!(
                "Attachment `{}` requested cancellation of active prompt `{}` on provider run `{}`.",
                attachment_id,
                prompt.id(),
                provider_run.id()
            ),
        );
        let session = self.session_snapshot(session_id)?;

        Ok(Some(crate::app::KernelPromptCancellation {
            cancellation: crate::session::PromptCancellation {
                prompt,
                started_next: None,
            },
            session,
            dispatch: Some(crate::app::KernelPromptAbortDispatch {
                session_id: session_id.to_string(),
                provider_run_id: provider_run.id().to_string(),
                source_attachment_id: attachment_id.to_string(),
            }),
        }))
    }
}

fn join_hidden_context(first: &str, second: &str) -> String {
    match (first.trim(), second.trim()) {
        ("", "") => String::new(),
        (first, "") => first.to_string(),
        ("", second) => second.to_string(),
        (first, second) => format!("{first}\n\n{second}"),
    }
}
