//! Remote-agent prompt ownership transitions.
//!
//! This module owns prompt queue state for agents leased to remote kernels. Local provider prompt
//! lifecycle remains in `prompt`.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn advance_next_queued_remote_prompt_dispatch(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Option<crate::app::KernelPromptSubmission>, DaemonError> {
        self.prepare_queued_remote_prompt_dispatch(session_id, agent_id, None)
    }

    pub(super) fn finish_remote_profile_transition(
        &self,
        session_id: &str,
        agent_id: &str,
        claim: crate::runtime::prompt_state::AgentProfileTransitionClaim,
    ) -> Result<Option<crate::app::KernelPromptSubmission>, DaemonError> {
        self.prepare_queued_remote_prompt_dispatch(session_id, agent_id, Some(claim))
    }

    fn prepare_queued_remote_prompt_dispatch(
        &self,
        session_id: &str,
        agent_id: &str,
        profile_transition: Option<crate::runtime::prompt_state::AgentProfileTransitionClaim>,
    ) -> Result<Option<crate::app::KernelPromptSubmission>, DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        let Some(remote_execution) = agent.remote_execution().cloned() else {
            return Ok(None);
        };
        let session = self.session_store.get_session(session_id)?;
        let next_prompt = if profile_transition.is_none() {
            let Some(prompt) = self
                .prompt_state_owner
                .peek_next_queued_prompt(&session, agent_id)
            else {
                return Ok(None);
            };
            Some(prompt)
        } else {
            None
        };
        if !self.provider_account_allows_queued_prompt_advance(
            session_id,
            &agent,
            "advance remote queued prompt",
        ) {
            return Ok(None);
        }
        let started = if let Some(claim) = profile_transition {
            claim.finish_and_activate_next(
                &session,
                agent_id,
                self.session_store.reserve_prompt_id(),
            )?
        } else {
            let next_prompt = next_prompt.expect("ordinary queue advance checked its queue front");
            self.prompt_state_owner
                .activate_next_queued_prompt_with_prompt_id(
                    &session,
                    agent_id,
                    Some(next_prompt.id()),
                    self.session_store.reserve_prompt_id(),
                )?
        };
        let Some(started) = started else {
            return Ok(None);
        };
        let _ =
            self.record_started_user_prompt(session_id, started.source_attachment_id(), &started)?;
        self.persist_prompt_session_state(&self.session_store.get_session(session_id)?, agent_id)?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        let remote_dispatch = crate::app::KernelRemotePromptDispatch {
            session_id: session_id.to_string(),
            agent_id: agent_id.to_string(),
            prompt_id: started.id().to_string(),
            worker_kernel_id: remote_execution.worker_kernel_id,
            leased_agent_id: remote_execution.leased_agent_id,
            relay_url: remote_execution.relay_url,
            relay_token: remote_execution.relay_token,
            source_attachment_id: started.source_attachment_id().to_string(),
            prompt: started.prompt().to_string(),
            hidden_system_context: started.hidden_system_context().to_string(),
            attachments: started.attachments().to_vec(),
            workspace_live_sync_mode: Some(
                crate::provider::provider_workspace_live_sync_mode_for_session(
                    agent.provider(),
                    &self.config_projection.snapshot(),
                    Some(&session),
                ),
            ),
            prompt_origin: started.prompt_origin(),
            external_provider: started.external_provider().map(str::to_string),
            external_provider_session_id: started
                .external_provider_session_id()
                .map(str::to_string),
            external_provider_turn_id: started.external_provider_turn_id().map(str::to_string),
            workflow_context: None,
        };
        let session = self.session_snapshot(session_id)?;
        Ok(Some(crate::app::KernelPromptSubmission {
            outcome: crate::session::PromptSubmissionOutcome::Started { prompt: started },
            session,
            dispatch: None,
            remote_dispatch: Some(remote_dispatch),
        }))
    }

    pub(super) fn submit_remote_prepared_prompt(
        &self,
        prepared: &crate::app::KernelPreparedPromptSubmission,
    ) -> Result<Option<crate::app::KernelPromptSubmission>, DaemonError> {
        let session_id = prepared.session_id.clone();
        let attachment_id = prepared.prompt.source_attachment_id().to_string();
        let source_attachment =
            if crate::scheduler::runtime::is_workflow_prompt_attachment(&attachment_id) {
                None
            } else {
                Some(self.ensure_attachment_in_session(&session_id, &attachment_id)?)
            };
        let target_agent_id = prepared.prompt.target_agent_id().to_string();
        let target_agent = self.agent_store.get_agent(&target_agent_id)?;
        if target_agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id,
                agent_id: target_agent_id,
            });
        }
        let Some(remote_execution) = target_agent.remote_execution().cloned() else {
            return Ok(None);
        };
        self.provider_account_profiles.require_agent_authenticated(
            &self.config_projection.snapshot(),
            &target_agent,
            "submit remote prompt",
        )?;
        if target_agent.state() == crate::agent::AgentState::Error {
            let _ = self
                .agent_store
                .set_agent_state(&target_agent_id, crate::agent::AgentState::Idle)?;
        }
        let session = self.session_store.get_session(&session_id)?;
        let queued_while_active = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, &target_agent_id)
            .is_some();
        let will_queue = prepared.force_queue || queued_while_active;
        let prompt = if let Some(source_attachment) = source_attachment.as_ref() {
            prepared.prompt.clone().with_source_attribution(
                source_attachment.client_id(),
                source_attachment.owner_user_id(),
            )
        } else {
            prepared.prompt.clone()
        };
        let prompt = if will_queue {
            prompt
        } else {
            prompt.with_id(self.session_store.reserve_prompt_id())
        };
        let outcome = self.prompt_state_owner.submit_prepared_prompt(
            &session,
            prompt,
            prepared.force_queue,
        )?;
        let outcome_agent_id = match &outcome {
            crate::session::PromptSubmissionOutcome::Started { prompt }
            | crate::session::PromptSubmissionOutcome::Queued { prompt } => {
                prompt.target_agent_id().to_string()
            }
        };
        let (active_prompt, queued_prompts) = self
            .prompt_state_owner
            .state_parts(&session, &outcome_agent_id);
        self.mirror_prompt_owner_agent_state(
            &session_id,
            &outcome_agent_id,
            active_prompt,
            queued_prompts,
        )?;
        let remote_dispatch = match &outcome {
            crate::session::PromptSubmissionOutcome::Started { prompt } => {
                let _ = self.record_started_user_prompt(
                    &session_id,
                    prompt.source_attachment_id(),
                    prompt,
                )?;
                self.persist_prompt_session_state(
                    &self.session_store.get_session(&session_id)?,
                    &outcome_agent_id,
                )?;
                Some(crate::app::KernelRemotePromptDispatch {
                    session_id: session_id.clone(),
                    agent_id: target_agent_id.clone(),
                    prompt_id: prompt.id().to_string(),
                    worker_kernel_id: remote_execution.worker_kernel_id,
                    leased_agent_id: remote_execution.leased_agent_id,
                    relay_url: remote_execution.relay_url,
                    relay_token: remote_execution.relay_token,
                    source_attachment_id: prompt.source_attachment_id().to_string(),
                    prompt: prompt.prompt().to_string(),
                    hidden_system_context: prompt.hidden_system_context().to_string(),
                    attachments: prompt.attachments().to_vec(),
                    workspace_live_sync_mode: Some(
                        crate::provider::provider_workspace_live_sync_mode_for_session(
                            target_agent.provider(),
                            &self.config_projection.snapshot(),
                            Some(&session),
                        ),
                    ),
                    prompt_origin: prompt.prompt_origin(),
                    external_provider: prompt.external_provider().map(str::to_string),
                    external_provider_session_id: prompt
                        .external_provider_session_id()
                        .map(str::to_string),
                    external_provider_turn_id: prompt
                        .external_provider_turn_id()
                        .map(str::to_string),
                    workflow_context: None,
                })
            }
            crate::session::PromptSubmissionOutcome::Queued { prompt } => {
                self.record_notice_for_agent(
                    &session_id,
                    None,
                    Some(&target_agent_id),
                    self.other_attachment_ids(&session_id, prompt.source_attachment_id()),
                    format!(
                        "Attachment `{}` queued prompt `{}` for agent `{}`.",
                        prompt.source_attachment_id(),
                        prompt.id(),
                        target_agent_id
                    ),
                );
                None
            }
        };
        let session = if prepared.refresh_projection {
            self.session_snapshot(&session_id)?
        } else {
            self.session_snapshot_without_projection_update(&session_id)?
        };
        Ok(Some(crate::app::KernelPromptSubmission {
            outcome,
            session,
            dispatch: None,
            remote_dispatch,
        }))
    }

    pub(super) fn complete_remote_prompt_owner(
        &self,
        session_id: &str,
        agent_id: &str,
        remote_provider_run_id: &str,
        next_queued_prompt: Option<&crate::session::PromptQueueItem>,
    ) -> Result<crate::session::PromptCompletion, DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        let _ = self
            .agent_store
            .set_remote_execution_active_worker_provider_run_id(agent_id, None)?;
        let session = self.session_store.get_session(session_id)?;
        let completed = self
            .prompt_state_owner
            .complete_active_prompt_only(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
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
            completed.id(),
            Some(remote_provider_run_id),
            settled_at_ms,
            "completed",
        );
        let recipient_attachment_ids = self
            .attachment_store
            .list_session_attachment_ids(session_id);
        self.record_assistant_message_completion(
            session_id,
            remote_provider_run_id,
            recipient_attachment_ids,
            &format!("prompt-complete:{}", completed.id()),
            settled_at_ms,
        );
        let started_next = if let Some(expected_next) = next_queued_prompt {
            let active = self
                .prompt_state_owner
                .activate_next_queued_prompt_with_prompt_id(
                    &session,
                    agent_id,
                    Some(expected_next.id()),
                    self.session_store.reserve_prompt_id(),
                )?;
            if let Some(active_prompt) = active.as_ref() {
                let _ = self.record_started_user_prompt(
                    session_id,
                    active_prompt.source_attachment_id(),
                    active_prompt,
                )?;
            }
            active
        } else {
            None
        };
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        let _ = self.session_snapshot(session_id)?;
        Ok(crate::session::PromptCompletion {
            completed,
            started_next,
        })
    }

    pub(super) fn begin_remote_prompt_cancellation(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
    ) -> Result<crate::app::KernelPromptCancellation, DaemonError> {
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
        let session = self.session_store.get_session(session_id)?;
        let active_prompt = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, target_agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        if active_prompt.status() == crate::session::PromptStatus::Cancelling {
            let session = self.session_snapshot(session_id)?;
            return Ok(crate::app::KernelPromptCancellation {
                cancellation: crate::session::PromptCancellation {
                    prompt: active_prompt,
                    started_next: None,
                },
                session,
                dispatch: None,
            });
        }
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
        let worker_kernel_id = target_agent
            .remote_execution()
            .map(|remote| remote.worker_kernel_id.clone())
            .unwrap_or_else(|| "remote".to_string());
        self.record_notice(
            session_id,
            None,
            self.other_attachment_ids(session_id, attachment_id),
            format!(
                "Attachment `{attachment_id}` requested cancellation of active remote prompt `{}` on worker kernel `{}`.",
                prompt.id(),
                worker_kernel_id
            ),
        );
        let session = self.session_snapshot(session_id)?;
        Ok(crate::app::KernelPromptCancellation {
            cancellation: crate::session::PromptCancellation {
                prompt,
                started_next: None,
            },
            session,
            dispatch: None,
        })
    }

    pub(super) fn finalize_remote_prompt_cancellation_after_worker_settled(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
    ) -> Result<crate::app::KernelPromptCancellation, DaemonError> {
        let _ =
            self.begin_remote_prompt_cancellation(session_id, target_agent_id, attachment_id)?;
        let cancellation = self.finalize_local_prompt_cancellation_with_queued_advance(
            session_id,
            target_agent_id,
            None,
        )?;
        if cancellation.cancellation.prompt.workflow_run_id().is_some() {
            self.workflow_cancel_prompt(session_id, &cancellation.cancellation.prompt)?;
        }
        let session = self.session_snapshot(session_id)?;
        Ok(crate::app::KernelPromptCancellation {
            cancellation: cancellation.cancellation,
            session,
            dispatch: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::Mutex;

    use super::*;
    use crate::agent::RemoteAgentBinding;
    use crate::app::{DaemonApp, KernelPreparedPromptSubmission, KernelSessionService};
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::session::{CreateSessionRequest, PromptQueueItem, PromptStatus};

    async fn owned_runtime_state(app: &Arc<Mutex<DaemonApp>>) -> KernelRuntimeState {
        let (
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        ) = {
            let app_locked = app.lock().await;
            (
                app_locked.config_projection_store(),
                app_locked.session_state_store(),
                app_locked.agents().clone(),
                app_locked.attachments().clone(),
                app_locked.providers().clone(),
                app_locked.provider_process_tracking_store(),
                app_locked.slices(),
                app_locked.session_state_projection_store(),
                app_locked.provider_run_projection_store(),
                app_locked.operational_history_store(),
                app_locked.durable_state_store(),
                app_locked.prompt_state_owner(),
                app_locked.active_turn_store(),
                app_locked.prompt_activity_store(),
                app_locked.prompt_workspace_claim_store(),
                app_locked.structured_output_record_store(),
                app_locked.terminal_stream_store(),
                app_locked.workflow_design_event_store(),
                app_locked.metaagent_event_store(),
                app_locked.workspace_coordinator(),
            )
        };
        KernelRuntimeState::new_with_owned_state(
            Arc::clone(app),
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        )
    }

    #[tokio::test]
    async fn remote_completion_with_queued_prompt_projects_combined_transition() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let attachment = KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "client-remote-queue",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.agents
            .bind_remote_execution(
                agent.id(),
                RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel-1".to_string(),
                    worker_machine_id: "worker-machine-1".to_string(),
                    execution_lease_id: "lease-1".to_string(),
                    leased_agent_id: "leased-agent-1".to_string(),
                    active_worker_provider_run_id: Some("worker-run-1".to_string()),
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .expect("agent should bind to remote execution");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment_id = attachment.id().to_string();
        let projection_store = app.session_state_projection_store();
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;

        let first = PromptQueueItem::new(
            "pending:first",
            &attachment_id,
            &agent_id,
            "first remote prompt",
            PromptStatus::Queued,
        );
        let first_submission = runtime
            .owned
            .submit_remote_prepared_prompt(&KernelPreparedPromptSubmission {
                session_id: session_id.clone(),
                prompt: first,
                force_queue: false,
                refresh_projection: true,
            })
            .expect("first remote prompt should submit")
            .expect("remote prompt should be handled");
        let active_prompt_id = match first_submission.outcome {
            crate::session::PromptSubmissionOutcome::Started { prompt } => prompt.id().to_string(),
            crate::session::PromptSubmissionOutcome::Queued { .. } => {
                panic!("first remote prompt should start")
            }
        };
        let queued = PromptQueueItem::new(
            "queued:second",
            &attachment_id,
            &agent_id,
            "second remote prompt",
            PromptStatus::Queued,
        );
        let queued_submission = runtime
            .owned
            .submit_remote_prepared_prompt(&KernelPreparedPromptSubmission {
                session_id: session_id.clone(),
                prompt: queued,
                force_queue: false,
                refresh_projection: true,
            })
            .expect("second remote prompt should submit")
            .expect("remote prompt should be handled");
        let queued_prompt = match queued_submission.outcome {
            crate::session::PromptSubmissionOutcome::Queued { prompt } => prompt,
            crate::session::PromptSubmissionOutcome::Started { .. } => {
                panic!("second remote prompt should queue")
            }
        };
        let before_completion_sequence = projection_store.session_change_sequence(&session_id);

        let completion = runtime
            .owned
            .complete_remote_prompt_owner(
                &session_id,
                &agent_id,
                "worker-run-1",
                Some(&queued_prompt),
            )
            .expect("remote prompt completion should advance queue");

        assert_eq!(completion.completed.id(), active_prompt_id);
        let started_next = completion
            .started_next
            .expect("queued remote prompt should start");
        assert_ne!(started_next.id(), queued_prompt.id());
        assert_eq!(started_next.prompt(), queued_prompt.prompt());
        assert!(
            projection_store.session_change_sequence(&session_id) > before_completion_sequence,
            "remote queued advancement should refresh the session projection"
        );
        let projected = projection_store
            .get(&session_id)
            .expect("session projection should refresh");
        let active = projected
            .active_prompt_for_agent(&agent_id)
            .expect("next remote prompt should project as active");
        assert_eq!(active.id(), started_next.id());
        assert_eq!(active.prompt(), "second remote prompt");
        assert!(projected
            .queued_prompts_for_agent(&agent_id)
            .is_some_and(|queue| queue.is_empty()));
    }

    #[tokio::test]
    async fn stopped_slice_prompt_settles_with_one_visible_durable_error() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "workspace-stopped-slice",
                "worktree-stopped-slice",
            ))
            .expect("session should be created");
        let attachment = KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "client-stopped-slice",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.agents
            .bind_remote_execution(
                agent.id(),
                RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel-stopped".to_string(),
                    worker_machine_id: "worker-machine-stopped".to_string(),
                    execution_lease_id: "lease-stopped".to_string(),
                    leased_agent_id: "leased-agent-stopped".to_string(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .expect("agent should bind to remote execution");
        let slice = app
            .slices()
            .create(
                "owner-kernel",
                "owner-machine",
                crate::slice::CreateSliceInput {
                    name: "stopped-slice".to_string(),
                    backend: crate::slice::SliceBackendKind::LocalDocker,
                    os: "linux".to_string(),
                    display_mode: crate::slice::SliceDisplayMode::Headless,
                    workspace_id: Some("workspace-stopped-slice".to_string()),
                    worktree_id: Some("worktree-stopped-slice".to_string()),
                    workspace_mount: None,
                    development: None,
                    worker_kernel_ref: Some("slice:stopped-slice".to_string()),
                    display_url: None,
                    provider_auth: Vec::new(),
                    from_saved_state: None,
                    now_ms: 1,
                },
            )
            .expect("slice should be created stopped");
        app.slices()
            .attach_agent(&slice.id, session.id(), agent.id(), 2)
            .expect("agent should attach to slice");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;

        let submission = runtime
            .owned
            .submit_remote_prepared_prompt(&KernelPreparedPromptSubmission {
                session_id: session_id.clone(),
                prompt: PromptQueueItem::new(
                    "pending:stopped",
                    attachment.id(),
                    &agent_id,
                    "prompt for stopped slice",
                    PromptStatus::Queued,
                ),
                force_queue: false,
                refresh_projection: true,
            })
            .expect("stopped slice prompt should be admitted locally")
            .expect("remote prompt should be handled");
        let mut dispatch = submission
            .remote_dispatch
            .expect("admitted stopped-slice prompt should reach remote dispatch settlement");
        let dispatch_error =
            super::remote_prompt_worker_submission_runtime::submit_remote_prompt_to_worker_with_binding_refresh(
                &runtime,
                &mut dispatch,
                "prompt for stopped slice".to_string(),
                Vec::new(),
                Vec::new(),
                None,
                crate::extension::RemoteExtensionManifest::default(),
            )
            .await
            .expect_err("stopped slice must fail before relay transport");

        assert!(dispatch_error
            .to_string()
            .contains("stopped slice `stopped-slice`"));
        runtime
            .finish_remote_prompt_dispatch(dispatch, Err(dispatch_error))
            .await
            .expect_err("authoritative dispatch failure must remain visible to the caller");
        assert_eq!(
            runtime
                .owned
                .slice_store
                .resolve(&slice.id)
                .expect("slice should remain available")
                .status,
            crate::slice::SliceStatus::Stopped,
        );
        let output_records = runtime
            .owned
            .terminal_stream
            .drain_output_records(&session_id, attachment.id());
        let errors = output_records
            .iter()
            .filter(|record| record.kind == crate::terminal::TerminalOutputKind::ProviderError)
            .collect::<Vec<_>>();
        assert_eq!(errors.len(), 1, "live trace must show the rejection once");
        assert!(String::from_utf8_lossy(&errors[0].bytes).contains("stopped slice `stopped-slice`"));
        let durable_events = runtime
            .owned
            .operational_history_store
            .load_session_events(&session_id, Some(&agent_id))
            .expect("durable agent history should load");
        assert_eq!(
            durable_events
                .iter()
                .filter(|event| event.kind == crate::history::HistoryEventKind::ProviderError)
                .count(),
            1,
            "refresh history must contain exactly one visible error",
        );
        assert!(!durable_events.iter().any(|event| {
            event.kind == crate::history::HistoryEventKind::Notice
                && event
                    .content
                    .as_deref()
                    .is_some_and(|content| content.contains("Remote prompt dispatch failed"))
        }));
        assert!(runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(
                &runtime
                    .owned
                    .session_store
                    .get_session(&session_id)
                    .expect("session should remain available"),
                &agent_id,
            )
            .is_none());
        assert_eq!(
            runtime
                .owned
                .agent_store
                .get_agent(&agent_id)
                .expect("agent should remain available")
                .state(),
            crate::agent::AgentState::Error,
        );
        let refreshed = runtime
            .owned
            .session_snapshot(&session_id)
            .expect("failed stopped-slice session should remain refreshable");
        let refreshed_agent = refreshed
            .agents()
            .iter()
            .find(|candidate| candidate.id() == agent_id)
            .expect("remote agent should remain projected after refresh");
        assert_eq!(refreshed_agent.state(), crate::agent::AgentState::Error);
        assert_eq!(
            refreshed_agent
                .remote_execution()
                .map(|remote| remote.worker_kernel_id.as_str()),
            Some("worker-kernel-stopped"),
        );
    }

    #[tokio::test]
    async fn worker_settled_remote_cancellation_clears_the_home_active_prompt() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "workspace-cancel",
                "worktree-cancel",
            ))
            .expect("session should be created");
        let attachment = KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "client-remote-cancel",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.agents
            .bind_remote_execution(
                agent.id(),
                RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel-cancel".to_string(),
                    worker_machine_id: "worker-machine-cancel".to_string(),
                    execution_lease_id: "lease-cancel".to_string(),
                    leased_agent_id: "leased-agent-cancel".to_string(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .expect("agent should bind to remote execution");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment_id = attachment.id().to_string();
        let prompt = PromptQueueItem::new(
            "pending:cancel",
            &attachment_id,
            &agent_id,
            "stale remote prompt",
            PromptStatus::Queued,
        );
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        runtime
            .owned
            .submit_remote_prepared_prompt(&KernelPreparedPromptSubmission {
                session_id: session_id.clone(),
                prompt,
                force_queue: false,
                refresh_projection: true,
            })
            .expect("remote prompt should submit")
            .expect("remote prompt should be handled");

        let cancellation = runtime
            .owned
            .finalize_remote_prompt_cancellation_after_worker_settled(
                &session_id,
                &agent_id,
                &attachment_id,
            )
            .expect("settled worker cancellation should finalize at home");

        assert_eq!(
            cancellation.cancellation.prompt.status(),
            PromptStatus::Cancelled
        );
        assert!(runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(
                &runtime
                    .owned
                    .session_store
                    .get_session(&session_id)
                    .expect("session should remain available"),
                &agent_id,
            )
            .is_none());
        assert_eq!(
            runtime
                .owned
                .agent_store
                .get_agent(&agent_id)
                .expect("agent should remain available")
                .state(),
            crate::agent::AgentState::Focused
        );
    }
}
