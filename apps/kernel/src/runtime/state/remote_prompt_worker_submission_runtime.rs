//! Relay worker submission and stale remote-agent binding refresh for remote prompts.

use super::*;

const REMOTE_PROMPT_TRANSPORT_RETRY_WINDOW: std::time::Duration =
    std::time::Duration::from_secs(30);

pub(super) async fn submit_remote_prompt_to_worker_with_binding_refresh(
    state: &KernelRuntimeState,
    dispatch: &mut crate::app::KernelRemotePromptDispatch,
    prompt: String,
    attachments: Vec<crate::transport::relay_peer::RelayPromptAttachment>,
    required_mcps: Vec<crate::transport::relay_peer::RequiredRemoteMcp>,
    required_skills: Option<Vec<crate::transport::relay_peer::RequiredRemoteSkill>>,
    remote_extension_manifest: crate::extension::RemoteExtensionManifest,
) -> Result<String, DaemonError> {
    let mut attempt = 0_u32;
    let transport_retry_started_at = tokio::time::Instant::now();
    loop {
        if let Some(error) = remote_prompt_dispatch_unavailable_slice_error(state, dispatch) {
            return Err(error);
        }
        if attempt > 0 && !remote_prompt_dispatch_is_current(state, dispatch) {
            return Err(DaemonError::NoActivePrompt {
                session_id: dispatch.session_id.clone(),
            });
        }
        let mut result = submit_remote_prompt_to_worker(
            state,
            dispatch,
            prompt.clone(),
            attachments.clone(),
            required_mcps.clone(),
            required_skills.clone(),
            remote_extension_manifest.clone(),
            "unexpected remote prompt response",
        )
        .await;
        if remote_prompt_dispatch_should_refresh_binding(&result) {
            result = match refresh_remote_prompt_binding(state, dispatch).await {
                Ok(()) => {
                    submit_remote_prompt_to_worker(
                        state,
                        dispatch,
                        prompt.clone(),
                        attachments.clone(),
                        required_mcps.clone(),
                        required_skills.clone(),
                        remote_extension_manifest.clone(),
                        "unexpected remote prompt response after binding refresh",
                    )
                    .await
                }
                Err(error) => Err(error),
            };
        }
        match result {
            Err(error) if remote_prompt_error_should_retry_transport(&error) => {
                attempt = attempt.saturating_add(1);
                if remote_prompt_transport_retry_window_expired(
                    transport_retry_started_at.elapsed(),
                ) {
                    return Err(DaemonError::LocalTransport {
                        operation: "submit remote prompt transport retry window",
                        message: format!(
                            "worker kernel `{}` remained unreachable for {}s while dispatching prompt `{}`: {error}",
                            dispatch.worker_kernel_id,
                            REMOTE_PROMPT_TRANSPORT_RETRY_WINDOW.as_secs(),
                            dispatch.prompt_id,
                        ),
                    });
                }
                if attempt == 1 || attempt % 12 == 0 {
                    crate::logging::warn_with_fields(
                        "daemon.remote_prompt_dispatch",
                        "remote prompt transport unavailable; retrying active prompt",
                        serde_json::json!({
                            "session_id": dispatch.session_id,
                            "agent_id": dispatch.agent_id,
                            "worker_kernel_id": dispatch.worker_kernel_id,
                            "leased_agent_id": dispatch.leased_agent_id,
                            "prompt_id": dispatch.prompt_id,
                            "attempt": attempt,
                            "error": error.to_string(),
                        }),
                    );
                }
                tokio::time::sleep(remote_prompt_transport_retry_delay(attempt)).await;
            }
            result => return result,
        }
    }
}

async fn refresh_remote_prompt_binding(
    state: &KernelRuntimeState,
    dispatch: &mut crate::app::KernelRemotePromptDispatch,
) -> Result<(), DaemonError> {
    crate::logging::warn_with_fields(
        "daemon.remote_prompt_dispatch",
        "remote prompt lease stale; refreshing binding",
        serde_json::json!({
            "session_id": dispatch.session_id,
            "agent_id": dispatch.agent_id,
            "worker_kernel_id": dispatch.worker_kernel_id,
            "leased_agent_id": dispatch.leased_agent_id,
        }),
    );
    let agent = state
        .with_app_side_effect(|app| app.refresh_remote_agent_binding(&dispatch.agent_id))
        .await?;
    let Some(remote_execution) = agent.remote_execution().cloned() else {
        return Err(DaemonError::LocalTransport {
            operation: "refresh remote prompt binding",
            message: format!(
                "agent `{}` did not have remote execution after binding refresh",
                dispatch.agent_id
            ),
        });
    };
    dispatch.worker_kernel_id = remote_execution.worker_kernel_id;
    dispatch.leased_agent_id = remote_execution.leased_agent_id;
    dispatch.relay_url = remote_execution.relay_url;
    dispatch.relay_token = remote_execution.relay_token;
    Ok(())
}

fn remote_prompt_dispatch_is_current(
    state: &KernelRuntimeState,
    dispatch: &crate::app::KernelRemotePromptDispatch,
) -> bool {
    let Ok(session) = state.owned.session_store.get_session(&dispatch.session_id) else {
        return false;
    };
    state
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &dispatch.agent_id)
        .is_some_and(|prompt| prompt.id() == dispatch.prompt_id)
}

pub(super) fn remote_prompt_transport_retry_delay(attempt: u32) -> std::time::Duration {
    let multiplier = 1_u64 << attempt.saturating_sub(1).min(3);
    std::time::Duration::from_millis(250_u64.saturating_mul(multiplier))
}

fn remote_prompt_transport_retry_window_expired(elapsed: std::time::Duration) -> bool {
    elapsed >= REMOTE_PROMPT_TRANSPORT_RETRY_WINDOW
}

pub(super) fn remote_prompt_unavailable_slice_error(
    slice_store: &crate::slice::SliceStore,
    remote_execution: &crate::agent::RemoteAgentBinding,
    session_id: &str,
    agent_id: &str,
) -> Option<DaemonError> {
    let slice = slice_store
        .list_by_session(session_id)
        .into_iter()
        .find(|slice| {
            slice
                .agent_ids
                .iter()
                .any(|candidate| candidate == agent_id)
        })
        .or_else(|| slice_store.resolve_by_worker_kernel_ref(&remote_execution.worker_kernel_id))
        .or_else(|| {
            slice_store.resolve_by_worker_kernel_ref(&remote_execution.worker_machine_id)
        })?;
    if remote_prompt_slice_status_allows_transport_retry(&slice.status) {
        return None;
    }
    let status = format!("{:?}", slice.status).to_ascii_lowercase();
    Some(DaemonError::LocalTransport {
        operation: "submit remote prompt to unavailable slice",
        message: format!(
            "agent `{agent_id}` is deployed in {status} slice `{}`; start the slice before sending a prompt (worker kernel `{}` is unreachable)",
            slice.name, remote_execution.worker_kernel_id
        ),
    })
}

fn remote_prompt_dispatch_unavailable_slice_error(
    state: &KernelRuntimeState,
    dispatch: &crate::app::KernelRemotePromptDispatch,
) -> Option<DaemonError> {
    let agent = state.owned.agent_store.get_agent(&dispatch.agent_id).ok()?;
    let remote_execution = agent.remote_execution()?;
    remote_prompt_unavailable_slice_error(
        &state.owned.slice_store,
        remote_execution,
        &dispatch.session_id,
        &dispatch.agent_id,
    )
}

fn remote_prompt_slice_status_allows_transport_retry(status: &crate::slice::SliceStatus) -> bool {
    matches!(
        status,
        crate::slice::SliceStatus::Starting | crate::slice::SliceStatus::Running
    )
}

async fn submit_remote_prompt_to_worker(
    state: &KernelRuntimeState,
    dispatch: &crate::app::KernelRemotePromptDispatch,
    prompt: String,
    attachments: Vec<crate::transport::relay_peer::RelayPromptAttachment>,
    required_mcps: Vec<crate::transport::relay_peer::RequiredRemoteMcp>,
    required_skills: Option<Vec<crate::transport::relay_peer::RequiredRemoteSkill>>,
    remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    unexpected_response_message: &'static str,
) -> Result<String, DaemonError> {
    let agent = state.owned.agent_store.get_agent(&dispatch.agent_id)?;
    let remote_execution = agent
        .remote_execution()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "dispatch remote agent prompt",
            message: format!("agent `{}` lost its remote binding", dispatch.agent_id),
        })?;
    if !remote_execution.relay_peer_protocol_compatible() {
        return Err(DaemonError::LocalTransport {
            operation: "dispatch remote agent prompt",
            message: format!(
                "remote worker `{}` has an incompatible or legacy relay peer protocol {:?}; rebind the remote agent before dispatch (current protocol {})",
                remote_execution.worker_kernel_id,
                remote_execution.relay_peer_protocol_version,
                crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
            ),
        });
    }
    let config = remote_dispatch_relay_config(state.config_snapshot().await, dispatch);
    let target = ClientTarget {
        daemon_id: Some(dispatch.worker_kernel_id.clone()),
        daemon_alias: None,
    };
    let request = RelayPeerRequest::SubmitLeasedPrompt {
        leased_agent_id: dispatch.leased_agent_id.clone(),
        expected_profile: crate::transport::relay_peer::RelayAgentExecutionProfile::from(&agent),
        prompt,
        hidden_system_context: dispatch.hidden_system_context.clone(),
        attachments,
        workflow_context: dispatch.workflow_context.clone(),
        git_context: Some(remote_git_turn_context(dispatch)),
        required_mcps,
        required_skills,
        remote_extension_manifest,
    };
    let response = match state.connected_relay_state_for_config(&config).await {
        Some(relay_state) => {
            crate::transport::relay_client::send_peer_request_via_connected_relay_with_timeout(
                &config,
                &relay_state,
                target,
                request,
                crate::transport::relay_client::LEASED_PROMPT_SUBMIT_RESPONSE_TIMEOUT,
            )
            .await
        }
        None => {
            crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
                &config,
                target,
                request,
                crate::transport::relay_client::LEASED_PROMPT_SUBMIT_RESPONSE_TIMEOUT,
            )
            .await
        }
    };
    match response {
        Ok(RelayPeerResponse::LeasedPromptSubmitted {
            provider_run_id, ..
        }) => Ok(provider_run_id),
        Ok(other) => Err(DaemonError::LocalTransport {
            operation: "submit remote prepared prompt",
            message: format!("{unexpected_response_message}: {other:?}"),
        }),
        Err(error) => Err(error),
    }
}

fn remote_prompt_dispatch_should_refresh_binding(result: &Result<String, DaemonError>) -> bool {
    let Err(error) = result else {
        return false;
    };
    remote_prompt_error_should_refresh_binding(error)
}

pub(super) fn remote_prompt_error_should_refresh_binding(error: &DaemonError) -> bool {
    match error {
        DaemonError::LeasedAgentNotFound { .. } | DaemonError::ExecutionLeaseNotFound { .. } => {
            true
        }
        DaemonError::LocalTransport { message, .. } => {
            message.contains("leased agent") && message.contains("was not found")
                || message.contains("execution lease") && message.contains("was not found")
                || message.contains("leased_agent_not_found")
                || message.contains("execution_lease_not_found")
        }
        _ => false,
    }
}

pub(super) fn remote_prompt_error_should_retry_transport(error: &DaemonError) -> bool {
    let DaemonError::LocalTransport { operation, message } = error else {
        return false;
    };
    if matches!(
        *operation,
        "connect temporary relay peer socket"
            | "write temporary relay register"
            | "write temporary relay peer request"
    ) {
        return true;
    }
    let message = message.to_ascii_lowercase();
    let transient_message = [
        "target daemon is not connected to relay",
        "relay is not connected",
        "relay peer request was cancelled",
        "timed out waiting for relay peer response",
        "not currently visible on relay",
        "did not appear on relay",
        "connection reset",
        "connection refused",
        "connection closed",
        "closed without",
        "broken pipe",
        "temporarily unavailable",
        "websocket",
    ]
    .iter()
    .any(|candidate| message.contains(candidate));
    transient_message
        && matches!(
            *operation,
            "send relay peer request"
                | "read relay peer response"
                | "get_live_kernel"
                | "relay_metadata_query"
                | "connect relay metadata socket"
                | "write relay metadata request"
                | "read relay metadata response"
        )
}

fn remote_git_turn_context(
    dispatch: &crate::app::KernelRemotePromptDispatch,
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
    mut config: crate::config::DaemonConfig,
    dispatch: &crate::app::KernelRemotePromptDispatch,
) -> crate::config::DaemonConfig {
    if let (Some(relay_url), Some(relay_token)) =
        (dispatch.relay_url.clone(), dispatch.relay_token.clone())
    {
        config.apply_remote_relay_override(relay_url, relay_token);
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_prompt_dispatch_does_not_refresh_binding_after_worker_timeout() {
        let result = Err(DaemonError::LocalTransport {
            operation: "submit remote prepared prompt",
            message: "remote prompt dispatch timed out waiting for worker response".to_string(),
        });

        assert!(!remote_prompt_dispatch_should_refresh_binding(&result));
    }

    #[test]
    fn remote_prompt_dispatch_refreshes_binding_for_missing_lease_errors() {
        let result = Err(DaemonError::LocalTransport {
            operation: "submit remote prepared prompt",
            message: "leased_agent_not_found".to_string(),
        });

        assert!(remote_prompt_dispatch_should_refresh_binding(&result));
    }

    #[test]
    fn remote_prompt_dispatch_retries_disconnected_relay_targets() {
        let error = DaemonError::LocalTransport {
            operation: "read relay peer response",
            message: "target daemon is not connected to relay".to_string(),
        };

        assert!(remote_prompt_error_should_retry_transport(&error));
    }

    #[test]
    fn remote_prompt_transport_retry_window_is_bounded() {
        assert!(!remote_prompt_transport_retry_window_expired(
            REMOTE_PROMPT_TRANSPORT_RETRY_WINDOW - std::time::Duration::from_millis(1),
        ));
        assert!(remote_prompt_transport_retry_window_expired(
            REMOTE_PROMPT_TRANSPORT_RETRY_WINDOW,
        ));
    }

    #[test]
    fn remote_prompt_dispatch_does_not_retry_stopped_slice_forever() {
        assert!(!remote_prompt_slice_status_allows_transport_retry(
            &crate::slice::SliceStatus::Stopped,
        ));
        assert!(!remote_prompt_slice_status_allows_transport_retry(
            &crate::slice::SliceStatus::Stopping,
        ));
        assert!(!remote_prompt_slice_status_allows_transport_retry(
            &crate::slice::SliceStatus::Unhealthy,
        ));
        assert!(remote_prompt_slice_status_allows_transport_retry(
            &crate::slice::SliceStatus::Starting,
        ));
        assert!(remote_prompt_slice_status_allows_transport_retry(
            &crate::slice::SliceStatus::Running,
        ));
    }

    #[test]
    fn leased_prompt_submit_timeout_covers_codex_mcp_retry_window() {
        assert!(
            crate::transport::relay_client::LEASED_PROMPT_SUBMIT_RESPONSE_TIMEOUT
                > std::time::Duration::from_secs(180)
        );
    }
}
