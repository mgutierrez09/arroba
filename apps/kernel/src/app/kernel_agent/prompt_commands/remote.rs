use crate::agent::RemoteAgentBinding;
use crate::app::{DaemonApp, KernelRemotePromptDispatch};
use crate::error::DaemonError;
use crate::session::{PromptCancellation, PromptCompletion, PromptQueueItem};
use crate::transport::relay_client::{
    send_peer_request_via_temporary_connection,
    send_peer_request_via_temporary_connection_with_timeout,
};
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use chariox_relay::protocol::ClientTarget;

use super::super::KernelAgentService;
use super::completion::{KernelPromptCompletionAdmission, KernelPromptOwnerCompletion};

fn remote_workspace_live_sync_mode_for_agent(
    app: &DaemonApp,
    session_id: &str,
    agent_id: &str,
) -> Option<crate::config::WorkspaceLiveSyncMode> {
    let session = app.sessions().get_session(session_id).ok()?;
    let agent = app.agents().get_agent(agent_id).ok()?;
    Some(
        crate::provider::provider_workspace_live_sync_mode_for_session(
            agent.provider(),
            app.config(),
            Some(&session),
        ),
    )
}

fn remote_git_turn_context(
    dispatch: &KernelRemotePromptDispatch,
) -> crate::transport::relay_peer::RemoteGitTurnContext {
    crate::transport::relay_peer::RemoteGitTurnContext {
        home_session_id: dispatch.session_id.clone(),
        home_agent_id: dispatch.agent_id.clone(),
        home_prompt_id: dispatch.prompt_id.clone(),
        home_turn_id: dispatch.prompt_id.clone(),
        source_attachment_id: Some(dispatch.source_attachment_id.clone()),
        workspace_live_sync_mode: dispatch.workspace_live_sync_mode,
        prompt_origin: Some(dispatch.prompt_origin),
        external_provider: dispatch.external_provider.clone(),
        external_provider_session_id: dispatch.external_provider_session_id.clone(),
        external_provider_turn_id: dispatch.external_provider_turn_id.clone(),
        prompt_summary: crate::prompt_transcript::render_prompt_transcript(
            &dispatch.prompt,
            &dispatch.attachments,
        ),
    }
}

fn remote_dispatch_relay_config(
    app: &DaemonApp,
    dispatch: &KernelRemotePromptDispatch,
) -> crate::config::DaemonConfig {
    let mut config = app.config().clone();
    if let (Some(relay_url), Some(relay_token)) =
        (dispatch.relay_url.clone(), dispatch.relay_token.clone())
    {
        config.apply_remote_relay_override(relay_url, relay_token);
    }
    config
}

fn remote_git_turn_context_for_prompt(
    app: &DaemonApp,
    session_id: &str,
    agent_id: &str,
    prompt: &PromptQueueItem,
    home_prompt_id: &str,
) -> crate::transport::relay_peer::RemoteGitTurnContext {
    crate::transport::relay_peer::RemoteGitTurnContext {
        home_session_id: session_id.to_string(),
        home_agent_id: agent_id.to_string(),
        home_prompt_id: home_prompt_id.to_string(),
        home_turn_id: home_prompt_id.to_string(),
        source_attachment_id: Some(prompt.source_attachment_id().to_string()),
        workspace_live_sync_mode: remote_workspace_live_sync_mode_for_agent(
            app, session_id, agent_id,
        ),
        prompt_origin: Some(prompt.prompt_origin()),
        external_provider: prompt.external_provider().map(str::to_string),
        external_provider_session_id: prompt.external_provider_session_id().map(str::to_string),
        external_provider_turn_id: prompt.external_provider_turn_id().map(str::to_string),
        prompt_summary: crate::prompt_transcript::render_prompt_transcript(
            prompt.prompt(),
            prompt.attachments(),
        ),
    }
}

fn remote_prompt_error_is_already_settled(error: &DaemonError) -> bool {
    match error {
        DaemonError::NoActivePrompt { .. } => true,
        DaemonError::LocalTransport { message, .. } => {
            message.contains("no active prompt")
                || message.contains("NoActivePrompt")
                || message.contains("no_active_prompt")
        }
        _ => false,
    }
}

impl<'a> KernelAgentService<'a> {
    pub(super) fn cancel_remote_active_prompt(
        &mut self,
        session_id: &str,
        agent_id: &str,
        attachment_id: Option<&str>,
        active_prompt: &PromptQueueItem,
        remote_execution: RemoteAgentBinding,
    ) -> Result<PromptCancellation, DaemonError> {
        let relay_config = self
            .app
            .relay_config_for_remote_execution(&remote_execution);
        let cancellation_response =
            self.app
                .block_on_relay_future(send_peer_request_via_temporary_connection(
                    &relay_config,
                    ClientTarget {
                        daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                        daemon_alias: None,
                    },
                    RelayPeerRequest::CancelLeasedPrompt {
                        leased_agent_id: remote_execution.leased_agent_id.clone(),
                    },
                ));
        match cancellation_response {
            Ok(RelayPeerResponse::LeasedPromptCancelled { .. }) => {}
            Ok(other) => {
                return Err(DaemonError::LocalTransport {
                    operation: "cancel remote prompt",
                    message: format!("unexpected remote prompt cancellation response: {other:?}"),
                });
            }
            Err(error) if remote_prompt_error_is_already_settled(&error) => {
                crate::logging::warn_with_fields(
                    "daemon.remote_prompt_dispatch",
                    "remote prompt cancellation already settled on worker",
                    serde_json::json!({
                        "session_id": session_id,
                        "agent_id": agent_id,
                        "worker_kernel_id": remote_execution.worker_kernel_id,
                        "leased_agent_id": remote_execution.leased_agent_id,
                        "error": error.to_string(),
                    }),
                );
                return self.finalize_active_prompt_cancellation(session_id, agent_id, None);
            }
            Err(error) => return Err(error),
        };
        let prompt = self
            .app
            .prompt_owner_begin_cancelling_active_prompt(session_id, agent_id)?;
        let recipients = match attachment_id {
            Some(attachment_id) => self.app.other_attachment_ids(session_id, attachment_id),
            None => self.app.attachments.list_session_attachment_ids(session_id),
        };
        let message = match attachment_id {
            Some(attachment_id) => format!(
                "Attachment `{attachment_id}` requested cancellation of active remote prompt `{}` on worker kernel `{}`.",
                active_prompt.id(),
                remote_execution.worker_kernel_id
            ),
            None => format!(
                "Chariox requested cancellation of active remote prompt `{}` on worker kernel `{}`.",
                active_prompt.id(),
                remote_execution.worker_kernel_id
            ),
        };
        self.app
            .record_notice(session_id, None, recipients, message);
        crate::app::KernelSessionReadService::new(self.app).session_snapshot(session_id)?;
        Ok(PromptCancellation {
            prompt,
            started_next: None,
        })
    }

    pub(super) fn finish_compat_remote_prompt_dispatch(
        &mut self,
        dispatch: Option<KernelRemotePromptDispatch>,
    ) -> Result<(), DaemonError> {
        let Some(dispatch) = dispatch else {
            return Ok(());
        };
        self.app.mark_active_prompt_delivery(
            &dispatch.session_id,
            &dispatch.agent_id,
            &dispatch.prompt_id,
            crate::session::DurablePromptDeliveryPhase::Dispatching,
            None,
            None,
        )?;
        let attachments = self
            .app
            .serialize_remote_prompt_attachments(&dispatch.attachments)?;
        let agent = self.app.agents().get_agent(&dispatch.agent_id)?;
        let remote_execution =
            agent
                .remote_execution()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "submit remote prepared prompt",
                    message: format!("agent `{}` lost its remote binding", dispatch.agent_id),
                })?;
        self.app
            .ensure_remote_agent_binding_protocol(remote_execution)?;
        let (required_mcps, required_skills, remote_extension_manifest) =
            self.app.remote_prompt_capabilities_for_agent(&agent)?;
        let relay_config = remote_dispatch_relay_config(self.app, &dispatch);
        let result = match self.app.block_on_relay_future(
            send_peer_request_via_temporary_connection_with_timeout(
                &relay_config,
                ClientTarget {
                    daemon_id: Some(dispatch.worker_kernel_id.clone()),
                    daemon_alias: None,
                },
                RelayPeerRequest::SubmitLeasedPrompt {
                    leased_agent_id: dispatch.leased_agent_id.clone(),
                    prompt: dispatch.prompt.clone(),
                    hidden_system_context: dispatch.hidden_system_context.clone(),
                    attachments,
                    workflow_context: dispatch.workflow_context.clone(),
                    git_context: Some(remote_git_turn_context(&dispatch)),
                    required_mcps,
                    required_skills,
                    remote_extension_manifest,
                    provider_launch_credential: None,
                },
                crate::transport::relay_client::LEASED_PROMPT_SUBMIT_RESPONSE_TIMEOUT,
            ),
        ) {
            Ok(RelayPeerResponse::LeasedPromptSubmitted {
                provider_run_id, ..
            }) => Ok(provider_run_id),
            Ok(other) => Err(DaemonError::LocalTransport {
                operation: "submit remote prepared prompt",
                message: format!("unexpected remote prompt response: {other:?}"),
            }),
            Err(error) => Err(error),
        };
        self.app
            .finish_kernel_remote_prompt_dispatch(dispatch, result)
    }

    pub(super) fn complete_remote_prompt_from_admission(
        &mut self,
        admission: KernelPromptCompletionAdmission,
    ) -> Result<KernelPromptOwnerCompletion, DaemonError> {
        let KernelPromptCompletionAdmission::Remote {
            session_id,
            agent_id,
            remote_execution,
            next_queued_prompt,
        } = admission
        else {
            return Err(DaemonError::LocalTransport {
                operation: "complete prompt admission",
                message: "expected remote prompt completion admission".to_string(),
            });
        };

        let relay_config = self
            .app
            .relay_config_for_remote_execution(&remote_execution);
        let completion_response =
            self.app
                .block_on_relay_future(send_peer_request_via_temporary_connection(
                    &relay_config,
                    ClientTarget {
                        daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                        daemon_alias: None,
                    },
                    RelayPeerRequest::CompleteLeasedPrompt {
                        leased_agent_id: remote_execution.leased_agent_id.clone(),
                    },
                ));
        let remote_provider_run_id = match completion_response {
            Ok(response) => match response {
                RelayPeerResponse::LeasedPromptCompleted {
                    provider_run_id,
                    git_observations,
                    workspace_live_sync_change,
                    ..
                } => {
                    let _ = crate::git_observer::append_observations(
                        &self.app.operational_history_store(),
                        git_observations,
                    )?;
                    if let Some(change) = workspace_live_sync_change {
                        self.app.fanout_remote_workspace_live_sync_change(
                            change,
                            Some(&remote_execution.worker_kernel_id),
                        );
                    }
                    provider_run_id
                }
                other => {
                    return Err(DaemonError::LocalTransport {
                        operation: "complete remote prompt",
                        message: format!("unexpected remote prompt completion response: {other:?}"),
                    });
                }
            },
            Err(error) if remote_prompt_error_is_already_settled(&error) => {
                crate::logging::warn_with_fields(
                    "daemon.remote_prompt_dispatch",
                    "remote prompt completion already settled on worker",
                    serde_json::json!({
                        "session_id": session_id,
                        "agent_id": agent_id,
                        "worker_kernel_id": remote_execution.worker_kernel_id,
                        "leased_agent_id": remote_execution.leased_agent_id,
                        "error": error.to_string(),
                    }),
                );
                None
            }
            Err(error) => return Err(error),
        };
        let completed = self
            .app
            .prompt_owner_complete_active_prompt_only(&session_id, &agent_id)?;
        let _ = self
            .app
            .agents()
            .set_remote_execution_active_worker_provider_run_id(&agent_id, None)?;
        Ok(KernelPromptOwnerCompletion {
            session_id,
            agent_id,
            completed,
            provider_run_id: None,
            remote_execution: Some(remote_execution),
            remote_provider_run_id,
            next_queued_prompt,
            settlement_status: crate::git_observer::CompletedTurnSettlementStatus::Completed,
        })
    }

    pub(super) fn finish_remote_prompt_completion(
        &mut self,
        completion: KernelPromptOwnerCompletion,
    ) -> Result<PromptCompletion, DaemonError> {
        let remote_provider_run_id = remote_completion_provider_run_id(
            completion.remote_execution.as_ref(),
            completion.remote_provider_run_id.as_deref(),
        );
        let settled_at_ms = crate::session::unix_epoch_ms();
        let started_at_ms = self
            .app
            .active_turn_store()
            .get(&remote_provider_run_id)
            .map(|turn| turn.started_at_ms)
            .or(Some(completion.completed.created_at_ms()));
        self.app
            .operational_history_store()
            .record_prompt_settlement(
                self.app.history_archive_enabled(),
                &completion.session_id,
                &completion.agent_id,
                completion.completed.id(),
                Some(&remote_provider_run_id),
                settled_at_ms,
                completion.settlement_status.as_str(),
            );
        self.app
            .completed_git_turn_snapshot_store()
            .record_prompt_settlement(
                &completion.session_id,
                &completion.agent_id,
                &remote_provider_run_id,
                &completion.completed,
                settled_at_ms,
                started_at_ms,
                completion.settlement_status,
            );
        let recipient_attachment_ids = self
            .app
            .attachments
            .list_session_attachment_ids(&completion.session_id);
        self.record_assistant_message_completion(
            &completion.session_id,
            &remote_provider_run_id,
            recipient_attachment_ids,
            &format!("prompt-complete:{}", completion.completed.id()),
            settled_at_ms,
        );
        let started_next = if self
            .app
            .prompt_owner_active_prompt_for_agent(&completion.session_id, &completion.agent_id)?
            .is_none()
        {
            let remote_execution = completion.remote_execution.as_ref().ok_or_else(|| {
                DaemonError::LocalTransport {
                    operation: "complete remote prompt",
                    message: "missing remote execution binding".to_string(),
                }
            })?;
            self.advance_next_queued_prompt_remote(
                &completion.session_id,
                &completion.agent_id,
                &remote_execution.worker_kernel_id,
                &remote_execution.leased_agent_id,
                remote_execution.relay_url.as_deref(),
                remote_execution.relay_token.as_deref(),
                completion.next_queued_prompt.as_ref(),
            )?
        } else {
            None
        };
        if started_next.is_none() {
            self.app
                .sync_focused_provider_run_if_idle(&completion.session_id)?;
        }
        crate::app::KernelSessionReadService::new(self.app)
            .session_snapshot(&completion.session_id)?;

        let prompt_completion = PromptCompletion {
            completed: completion.completed,
            started_next,
        };
        self.inject_orphaned_metaagent_task_event_after_turn(
            &completion.agent_id,
            &prompt_completion,
        )?;
        Ok(prompt_completion)
    }

    pub(crate) fn advance_next_queued_prompt_remote(
        &mut self,
        session_id: &str,
        agent_id: &str,
        worker_kernel_id: &str,
        leased_agent_id: &str,
        relay_url: Option<&str>,
        relay_token: Option<&str>,
        expected_next: Option<&PromptQueueItem>,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        let mut relay_config = self.app.config().clone();
        if let (Some(relay_url), Some(relay_token)) = (relay_url, relay_token) {
            relay_config
                .apply_remote_relay_override(relay_url.to_string(), relay_token.to_string());
        }
        loop {
            let next_candidate =
                self.next_queued_prompt_candidate(session_id, agent_id, expected_next)?;
            let Some(peeked) = next_candidate else {
                return Ok(None);
            };
            let is_workflow_prompt = crate::app::workflow_runtime::is_workflow_prompt_source(
                peeked.source_attachment_id(),
            );
            if let Err(error) = crate::app::KernelSessionReadService::new(self.app)
                .ensure_attachment_in_session(session_id, peeked.source_attachment_id())
            {
                if !is_workflow_prompt {
                    self.app.record_notice(
                        session_id,
                        None,
                        self.app.attachments.list_session_attachment_ids(session_id),
                        format!(
                            "Skipped queued prompt `{}` because its source attachment is no longer active: {}",
                            peeked.id(),
                            error
                        ),
                    );
                    let _ = self.activate_next_queued_prompt_for_mirror(
                        session_id,
                        agent_id,
                        expected_next,
                    )?;
                    continue;
                }
            }
            let agent = self.app.agents().get_agent(agent_id)?;
            let remote_execution =
                agent
                    .remote_execution()
                    .ok_or_else(|| DaemonError::LocalTransport {
                        operation: "advance remote queued prompt",
                        message: format!("agent `{agent_id}` lost its remote binding"),
                    })?;
            self.app
                .ensure_remote_agent_binding_protocol(remote_execution)?;
            let (required_mcps, required_skills, remote_extension_manifest) =
                self.app.remote_prompt_capabilities_for_agent(&agent)?;
            let home_prompt_id = self.app.sessions_mut().reserve_prompt_id();
            let response = self.app.block_on_relay_future(
                send_peer_request_via_temporary_connection_with_timeout(
                    &relay_config,
                    ClientTarget {
                        daemon_id: Some(worker_kernel_id.to_string()),
                        daemon_alias: None,
                    },
                    RelayPeerRequest::SubmitLeasedPrompt {
                        leased_agent_id: leased_agent_id.to_string(),
                        prompt: peeked.prompt().to_string(),
                        hidden_system_context: peeked.hidden_system_context().to_string(),
                        attachments: self
                            .app
                            .serialize_remote_prompt_attachments(peeked.attachments())?,
                        workflow_context: if is_workflow_prompt {
                            Some(
                                crate::app::RemoteWorkflowTurnContextResolver::new(self.app)
                                    .remote_workflow_turn_context_for_prompt(
                                        session_id, agent_id, &peeked,
                                    )?,
                            )
                        } else {
                            None
                        },
                        git_context: Some(remote_git_turn_context_for_prompt(
                            self.app,
                            session_id,
                            agent_id,
                            &peeked,
                            &home_prompt_id,
                        )),
                        required_mcps,
                        required_skills,
                        remote_extension_manifest,
                        provider_launch_credential: None,
                    },
                    crate::transport::relay_client::LEASED_PROMPT_SUBMIT_RESPONSE_TIMEOUT,
                ),
            );
            let remote_provider_run_id = match response {
                Ok(RelayPeerResponse::LeasedPromptSubmitted {
                    provider_run_id, ..
                }) => provider_run_id,
                Ok(other) => {
                    return Err(DaemonError::LocalTransport {
                        operation: "advance remote queued prompt",
                        message: format!("unexpected remote prompt response: {other:?}"),
                    });
                }
                Err(error) => return Err(error),
            };
            let (_session, next_candidate) = self
                .activate_next_queued_prompt_for_mirror_with_prompt_id(
                    session_id,
                    agent_id,
                    Some(&peeked),
                    home_prompt_id,
                )?;
            let Some(active) = next_candidate else {
                continue;
            };
            let _ = self
                .app
                .agents()
                .set_remote_execution_active_worker_provider_run_id(
                    agent_id,
                    Some(remote_provider_run_id.clone()),
                )?;
            let active = self.finish_promoted_queued_prompt_start(
                session_id,
                &remote_provider_run_id,
                agent_id,
                active.id(),
            )?;
            return Ok(Some(active));
        }
    }
}

fn remote_completion_provider_run_id(
    remote_execution: Option<&RemoteAgentBinding>,
    completed_worker_provider_run_id: Option<&str>,
) -> String {
    let Some(remote_execution) = remote_execution else {
        return completed_worker_provider_run_id
            .unwrap_or("remote-provider-run-completed")
            .to_string();
    };
    let worker_provider_run_id = completed_worker_provider_run_id
        .or(remote_execution.active_worker_provider_run_id.as_deref());
    worker_provider_run_id
        .map(|worker_provider_run_id| {
            crate::provider::projected_leased_provider_run_id(
                &remote_execution.leased_agent_id,
                worker_provider_run_id,
            )
        })
        .unwrap_or_else(|| "remote-provider-run-completed".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::config::DaemonConfig;
    use crate::session::{CreateSessionRequest, PromptStatus, PromptSubmissionOutcome};

    #[test]
    fn remote_completion_uses_the_home_projected_provider_run_id() {
        let binding = RemoteAgentBinding {
            worker_kernel_id: "worker-kernel".to_string(),
            worker_machine_id: "slice:slice-1".to_string(),
            leased_agent_id: "leased-agent-1".to_string(),
            execution_lease_id: "lease-1".to_string(),
            active_worker_provider_run_id: Some("worker-run-old".to_string()),
            relay_url: None,
            relay_token: None,
            relay_peer_protocol_version: Some(
                crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
            ),
        };

        assert_eq!(
            remote_completion_provider_run_id(Some(&binding), Some("worker-run-1")),
            "leased:leased-agent-1:worker-run-1",
        );
    }

    #[test]
    fn queued_remote_prompt_persists_reserved_home_prompt_after_activation() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "client-1",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let first_prompt_id = app.sessions_mut().reserve_prompt_id();
        let first = PromptQueueItem::new(
            first_prompt_id,
            attachment.id(),
            agent.id(),
            "first",
            PromptStatus::Queued,
        );
        assert!(matches!(
            app.prompt_owner_submit_prepared_prompt(session.id(), first, false)
                .expect("first prompt should submit"),
            PromptSubmissionOutcome::Started { .. }
        ));
        let second_prompt_id = app.sessions_mut().reserve_prompt_id();
        let second = PromptQueueItem::new(
            second_prompt_id,
            attachment.id(),
            agent.id(),
            "second",
            PromptStatus::Queued,
        );
        let PromptSubmissionOutcome::Queued { prompt: queued } = app
            .prompt_owner_submit_prepared_prompt(session.id(), second, false)
            .expect("second prompt should queue")
        else {
            panic!("second prompt should remain queued")
        };
        app.prompt_owner_complete_active_prompt_only(session.id(), agent.id())
            .expect("first prompt should complete without promoting the queue");
        let peeked = app
            .prompt_owner_peek_next_queued_prompt(session.id(), agent.id())
            .expect("queued prompt should load")
            .expect("second prompt should remain queued");
        assert_eq!(peeked.id(), queued.id());

        let canonical_prompt_id = app.sessions_mut().reserve_prompt_id();
        let context = remote_git_turn_context_for_prompt(
            &app,
            session.id(),
            agent.id(),
            &peeked,
            &canonical_prompt_id,
        );
        let (_session, active) = KernelAgentService::new(&mut app)
            .activate_next_queued_prompt_for_mirror_with_prompt_id(
                session.id(),
                agent.id(),
                Some(&peeked),
                canonical_prompt_id,
            )
            .expect("queued prompt should activate with the reserved id");
        let active = active.expect("queued prompt should become active");
        let expected_merge_key = format!("prompt:{}", active.id());
        assert_eq!(
            app.operational_history_store()
                .load_session_events(session.id(), Some(agent.id()))
                .expect("operational history should load before delivery finishes")
                .into_iter()
                .filter(|event| {
                    event
                        .metadata
                        .get("merge_key")
                        .and_then(serde_json::Value::as_str)
                        == Some(expected_merge_key.as_str())
                })
                .count(),
            0,
            "activation alone must not persist a prompt that has not finished delivery"
        );
        let active = KernelAgentService::new(&mut app)
            .finish_promoted_queued_prompt_start(
                session.id(),
                "worker-provider-run-2",
                agent.id(),
                active.id(),
            )
            .expect("queued remote prompt should finish activation");

        assert_ne!(queued.id(), active.id());
        assert_eq!(context.home_prompt_id, active.id());
        assert_eq!(context.home_turn_id, active.id());
        assert_eq!(active.status(), PromptStatus::Running);
        let history = app
            .load_session_history_entries(&session, Some(agent.id()))
            .expect("promoted prompt history should load");
        let entry = history
            .iter()
            .find(|entry| entry.text.contains("second"))
            .expect("promoted prompt should persist in home history");
        assert_eq!(entry.source_attachment_id.as_deref(), Some(attachment.id()));
        assert_eq!(
            entry.merge_key.as_deref(),
            Some(expected_merge_key.as_str())
        );
        assert_eq!(
            entry.prompt_origin,
            Some(crate::session::PromptOrigin::Chariox)
        );
        assert_eq!(
            app.operational_history_store()
                .load_session_events(session.id(), Some(agent.id()))
                .expect("operational history should load after delivery finishes")
                .into_iter()
                .filter(|event| {
                    event
                        .metadata
                        .get("merge_key")
                        .and_then(serde_json::Value::as_str)
                        == Some(expected_merge_key.as_str())
                })
                .count(),
            1,
            "successful remote promotion must persist one canonical prompt event"
        );
        assert!(app
            .agents()
            .get_agent(agent.id())
            .expect("agent should load")
            .last_prompt_sent_at_ms()
            .is_some());
    }
}
