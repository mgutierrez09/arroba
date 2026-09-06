//! Remote leased-prompt cancellation and completion runtime.
//!
//! This module owns relay calls that settle or cancel an already-admitted remote prompt.

use super::remote_prompt_worker_submission_runtime::remote_prompt_error_should_refresh_binding;
use super::*;

impl KernelRuntimeState {
    pub(super) async fn cancel_remote_agent_prompt_if_remote(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
    ) -> Result<Option<crate::app::KernelPromptCancellation>, DaemonError> {
        let owned = &self.owned;
        let Some(remote_execution) = owned
            .agent_store
            .get_agent(target_agent_id)?
            .remote_execution()
            .cloned()
        else {
            return Ok(None);
        };
        let prompt_already_cancelling = owned
            .prompt_state_owner
            .active_prompt_for_agent(
                &owned.session_store.get_session(session_id)?,
                target_agent_id,
            )
            .is_some_and(|prompt| prompt.status() == crate::session::PromptStatus::Cancelling);
        let cancellation_response = self
            .with_app_side_effect(|app| {
                let relay_config = app.relay_config_for_remote_execution(&remote_execution);
                app.block_on_relay_future(
                    crate::transport::relay_client::send_peer_request_via_temporary_connection(
                        &relay_config,
                        ClientTarget {
                            daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                            daemon_alias: None,
                        },
                        RelayPeerRequest::CancelLeasedPrompt {
                            leased_agent_id: remote_execution.leased_agent_id.clone(),
                        },
                    ),
                )
            })
            .await;
        match cancellation_response {
            Ok(RelayPeerResponse::LeasedPromptCancelled { .. }) => {
                if prompt_already_cancelling {
                    Ok(Some(
                        owned.finalize_remote_prompt_cancellation_after_worker_settled(
                            session_id,
                            target_agent_id,
                            attachment_id,
                        )?,
                    ))
                } else {
                    Ok(Some(owned.begin_remote_prompt_cancellation(
                        session_id,
                        target_agent_id,
                        attachment_id,
                    )?))
                }
            }
            Ok(other) => Err(DaemonError::LocalTransport {
                operation: "cancel remote prompt",
                message: format!("unexpected remote prompt cancellation response: {other:?}"),
            }),
            Err(error) if remote_prompt_completion_should_treat_as_settled(&error) => {
                crate::logging::warn_with_fields(
                    "daemon.remote_prompt_dispatch",
                    "remote prompt cancellation already settled on worker",
                    serde_json::json!({
                        "session_id": session_id,
                        "agent_id": target_agent_id,
                        "worker_kernel_id": remote_execution.worker_kernel_id,
                        "leased_agent_id": remote_execution.leased_agent_id,
                        "error": error.to_string(),
                    }),
                );
                Ok(Some(
                    owned.finalize_remote_prompt_cancellation_after_worker_settled(
                        session_id,
                        target_agent_id,
                        attachment_id,
                    )?,
                ))
            }
            Err(error) => Err(error),
        }
    }

    pub(super) async fn complete_remote_agent_prompt_if_remote(
        &self,
        session_id: &str,
        target_agent_id: &str,
        owned_provider_run_id: Option<String>,
        next_queued_prompt: Option<&crate::session::PromptQueueItem>,
    ) -> Result<Option<crate::session::PromptCompletion>, DaemonError> {
        let owned = &self.owned;
        let Some(remote_execution) = owned
            .agent_store
            .get_agent(target_agent_id)?
            .remote_execution()
            .cloned()
        else {
            return Ok(None);
        };
        let completion_response = self
            .with_app_side_effect(|app| {
                let relay_config = app.relay_config_for_remote_execution(&remote_execution);
                app.block_on_relay_future(
                    crate::transport::relay_client::send_peer_request_via_temporary_connection(
                        &relay_config,
                        ClientTarget {
                            daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                            daemon_alias: None,
                        },
                        RelayPeerRequest::CompleteLeasedPrompt {
                            leased_agent_id: remote_execution.leased_agent_id.clone(),
                        },
                    ),
                )
            })
            .await;
        let (remote_provider_run_id, provider_diagnostic) = match completion_response {
            Ok(RelayPeerResponse::LeasedPromptCompleted {
                provider_run_id,
                provider_diagnostic,
                git_observations,
                workspace_live_sync_change,
                ..
            }) => {
                if let Err(error) = crate::git_observer::append_observations(
                    &owned.operational_history_store,
                    git_observations,
                ) {
                    crate::logging::warn_with_fields(
                        "daemon.git_observer",
                        "failed to append remote git observations",
                        serde_json::json!({
                            "session_id": session_id,
                            "agent_id": target_agent_id,
                            "error": error.to_string(),
                        }),
                    );
                }
                if let Some(change) = workspace_live_sync_change {
                    self.record_and_fanout_workspace_live_sync_change(
                        change,
                        Some(&remote_execution.worker_kernel_id),
                        Some(&remote_execution.worker_machine_id),
                    )
                    .await;
                }
                (
                    provider_run_id.unwrap_or_else(|| "remote-provider-run-completed".to_string()),
                    provider_diagnostic,
                )
            }
            Err(error) if remote_prompt_completion_should_treat_as_settled(&error) => {
                crate::logging::warn_with_fields(
                    "daemon.remote_prompt_dispatch",
                    "remote prompt completion already settled on worker",
                    serde_json::json!({
                        "session_id": session_id,
                        "agent_id": target_agent_id,
                        "worker_kernel_id": remote_execution.worker_kernel_id,
                        "leased_agent_id": remote_execution.leased_agent_id,
                        "error": error.to_string(),
                    }),
                );
                (
                    owned_provider_run_id
                        .clone()
                        .unwrap_or_else(|| "remote-provider-run-completed".to_string()),
                    None,
                )
            }
            Err(error)
                if remote_prompt_error_should_refresh_binding(&error)
                    && remote_prompt_completion_should_wait_for_binding_repair(
                        owned
                            .agent_store
                            .get_agent(target_agent_id)?
                            .remote_execution(),
                        &remote_execution,
                    ) =>
            {
                crate::logging::warn_with_fields(
                    "daemon.remote_prompt_dispatch",
                    "remote prompt completion waiting for stale binding repair",
                    serde_json::json!({
                        "session_id": session_id,
                        "agent_id": target_agent_id,
                        "worker_kernel_id": remote_execution.worker_kernel_id,
                        "leased_agent_id": remote_execution.leased_agent_id,
                        "error": error.to_string(),
                    }),
                );
                return Err(DaemonError::LocalTransport {
                    operation: "complete remote prompt",
                    message: "remote prompt worker binding is being repaired; retry completion"
                        .to_string(),
                });
            }
            Err(error) => return Err(error),
            Ok(other) => {
                return Err(DaemonError::LocalTransport {
                    operation: "complete remote prompt",
                    message: format!("unexpected remote prompt completion response: {other:?}"),
                });
            }
        };
        let completion = owned.complete_remote_prompt_owner(
            session_id,
            target_agent_id,
            &remote_provider_run_id,
            next_queued_prompt,
        )?;
        if completion.completed.workflow_run_id().is_some() {
            if let Some(diagnostic) = provider_diagnostic.as_deref() {
                let dispatches = owned.workflow_fail_provider_prompt(
                    session_id,
                    &completion.completed,
                    Some(&remote_provider_run_id),
                    diagnostic,
                )?;
                self.spawn_workflow_prompt_dispatches(dispatches);
            } else {
                let dispatches = owned.workflow_complete_prompt(
                    session_id,
                    &completion.completed,
                    Some(&remote_provider_run_id),
                )?;
                self.spawn_workflow_prompt_dispatches(dispatches);
            }
        }
        if let Some(started_next) = completion.started_next.as_ref() {
            let agent = self.owned.agent_store.get_agent(target_agent_id)?;
            let (remote_prompt, required_skills) = self
                .prepare_remote_prompt_skill_context(&agent, started_next.prompt())
                .await?;
            let (required_mcps, remote_extension_manifest) =
                self.remote_prompt_mcp_capabilities_for_agent(&agent)?;
            let attachments = self
                .with_app_side_effect(|app| {
                    app.serialize_remote_prompt_attachments(started_next.attachments())
                })
                .await?;
            let workflow_context = if crate::scheduler::runtime::is_workflow_prompt_attachment(
                started_next.source_attachment_id(),
            ) {
                Some(
                    self.with_app_side_effect(|app| {
                        crate::app::RemoteWorkflowTurnContextResolver::new(app)
                            .remote_workflow_turn_context_for_prompt(
                                session_id,
                                target_agent_id,
                                started_next,
                            )
                    })
                    .await?,
                )
            } else {
                None
            };
            let submit_result = self
                .with_app_side_effect(|app| {
                    app.ensure_remote_agent_binding_protocol(&remote_execution)?;
                    let relay_config = app.relay_config_for_remote_execution(&remote_execution);
                    app.block_on_relay_future(
                        crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
                            &relay_config,
                            ClientTarget {
                                daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                                daemon_alias: None,
                            },
                            RelayPeerRequest::SubmitLeasedPrompt {
                                leased_agent_id: remote_execution.leased_agent_id.clone(),
                                prompt: remote_prompt,
                                hidden_system_context: started_next.hidden_system_context().to_string(),
                                attachments,
                                workflow_context,
                                git_context: Some(remote_git_turn_context_for_prompt(
                                    app,
                                    session_id,
                                    target_agent_id,
                                    started_next,
                                )),
                                required_mcps,
                                required_skills,
                                remote_extension_manifest,
                                provider_launch_credential: None,
                            },
                            crate::transport::relay_client::LEASED_PROMPT_SUBMIT_RESPONSE_TIMEOUT,
                        ),
                    )
                })
                .await?;
            if let RelayPeerResponse::LeasedPromptSubmitted {
                provider_run_id, ..
            } = submit_result
            {
                owned.echo_promoted_queued_prompt_to_attachments(
                    session_id,
                    &provider_run_id,
                    started_next.id(),
                    started_next.source_attachment_id(),
                    started_next.prompt(),
                    started_next.attachments(),
                );
            }
        }
        Ok(Some(completion))
    }
}

fn remote_prompt_completion_should_treat_as_settled(error: &DaemonError) -> bool {
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

fn remote_prompt_completion_should_wait_for_binding_repair(
    current_binding: Option<&crate::agent::RemoteAgentBinding>,
    attempted_binding: &crate::agent::RemoteAgentBinding,
) -> bool {
    let Some(current_binding) = current_binding else {
        return false;
    };
    current_binding.leased_agent_id != attempted_binding.leased_agent_id
        || current_binding.active_worker_provider_run_id.is_none()
}

fn remote_git_turn_context_for_prompt(
    app: &crate::app::DaemonApp,
    session_id: &str,
    agent_id: &str,
    prompt: &crate::session::PromptQueueItem,
) -> crate::transport::relay_peer::RemoteGitTurnContext {
    let workspace_live_sync_mode =
        app.sessions()
            .get_session(session_id)
            .ok()
            .and_then(|session| {
                app.agents().get_agent(agent_id).ok().map(|agent| {
                    crate::provider::provider_workspace_live_sync_mode_for_session(
                        agent.provider(),
                        app.config(),
                        Some(&session),
                    )
                })
            });
    crate::transport::relay_peer::RemoteGitTurnContext {
        home_session_id: session_id.to_string(),
        home_agent_id: agent_id.to_string(),
        home_prompt_id: prompt.id().to_string(),
        home_turn_id: prompt.id().to_string(),
        source_attachment_id: Some(prompt.source_attachment_id().to_string()),
        workspace_live_sync_mode,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(
        leased_agent_id: &str,
        active_run: Option<&str>,
    ) -> crate::agent::RemoteAgentBinding {
        crate::agent::RemoteAgentBinding {
            worker_kernel_id: "worker-kernel".to_string(),
            worker_machine_id: "worker-machine".to_string(),
            execution_lease_id: "lease".to_string(),
            leased_agent_id: leased_agent_id.to_string(),
            active_worker_provider_run_id: active_run.map(str::to_string),
            relay_url: None,
            relay_token: None,
            relay_peer_protocol_version: Some(
                crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
            ),
        }
    }

    #[test]
    fn remote_completion_waits_when_binding_has_no_active_worker_run() {
        let attempted = binding("leased-agent-old", None);

        assert!(remote_prompt_completion_should_wait_for_binding_repair(
            Some(&attempted),
            &attempted,
        ));
    }

    #[test]
    fn remote_completion_waits_when_binding_already_changed() {
        let attempted = binding("leased-agent-old", Some("worker-run-old"));
        let current = binding("leased-agent-new", Some("worker-run-new"));

        assert!(remote_prompt_completion_should_wait_for_binding_repair(
            Some(&current),
            &attempted,
        ));
    }

    #[test]
    fn remote_completion_does_not_wait_when_submitted_binding_matches() {
        let attempted = binding("leased-agent-old", Some("worker-run-old"));

        assert!(!remote_prompt_completion_should_wait_for_binding_repair(
            Some(&attempted),
            &attempted,
        ));
    }
}
