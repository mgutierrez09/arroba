//! Inbound relay peer request dispatch for leased runtimes and forwarded tools.

use std::sync::Arc;

use base64::Engine;
use chariox_relay::protocol::{EncryptedRelayPayload, RelayCallerIdentity};
use tokio::sync::RwLock;

use crate::runtime::router::CommandRouter;
use crate::transport::relay_crypto;
use crate::transport::relay_peer::{
    RelayPeerRequest, RelayPeerResponse, RELAY_PEER_PROTOCOL_VERSION,
};

use super::daemon_requests::RelayRequestOutcome;
use super::peer_events::emit_leased_projection_event;
use super::request_errors::{map_relay_error, relay_error};
use super::sender_identity::{
    require_bound_daemon_sender, require_bound_kernel_sender, require_bound_managed_context_sender,
    validate_optional_daemon_sender,
};
use super::{RelayClientState, RelayOutgoingSender};

pub(super) async fn handle_daemon_peer_request(
    router: &Arc<CommandRouter>,
    state: &Arc<RwLock<RelayClientState>>,
    outgoing_tx: &RelayOutgoingSender,
    from_daemon_id: &str,
    caller_identity: Option<RelayCallerIdentity>,
    encrypted_request: EncryptedRelayPayload,
) -> RelayRequestOutcome {
    if let Err(error) =
        validate_optional_daemon_sender(caller_identity.as_ref(), &encrypted_request)
    {
        return RelayRequestOutcome {
            encrypted_response: None,
            error: Some(error),
        };
    }
    let (request, requester_public_key, daemon_private_key, daemon_id) = {
        let daemon_private_key = router.relay_private_key();
        let daemon_id = router.relay_daemon_id();
        let decrypted = match relay_crypto::decrypt_payload_for_private_key(
            &daemon_private_key,
            &encrypted_request,
        ) {
            Ok(payload) => payload,
            Err(error) => {
                return RelayRequestOutcome {
                    encrypted_response: None,
                    error: Some(relay_error(
                        "invalid_request",
                        &format!("invalid relay peer request payload: {error}"),
                        false,
                    )),
                };
            }
        };
        let request = match serde_json::from_slice::<RelayPeerRequest>(&decrypted.plaintext) {
            Ok(request) => request,
            Err(error) => {
                return RelayRequestOutcome {
                    encrypted_response: None,
                    error: Some(relay_error(
                        "invalid_request",
                        &format!("invalid relay peer request payload: {error}"),
                        false,
                    )),
                };
            }
        };
        (
            request,
            decrypted.sender_public_key,
            daemon_private_key,
            daemon_id,
        )
    };
    let managed_context_caller = if managed_context_request(&request) {
        let identity = match require_bound_managed_context_sender(
            caller_identity.as_ref(),
            &encrypted_request,
        ) {
            Ok(identity) => identity.clone(),
            Err(error) => {
                return encrypt_peer_response(
                    &daemon_private_key,
                    &requester_public_key,
                    managed_context_failure_from_relay(&error),
                )
            }
        };
        let source_kernel_id = stable_peer_daemon_id(from_daemon_id);
        if source_kernel_id.trim().is_empty() {
            return encrypt_peer_response(
                &daemon_private_key,
                &requester_public_key,
                RelayPeerResponse::ManagedContextImportFailed {
                    code: "unauthorized".to_string(),
                    retryable: false,
                },
            );
        }
        Some((identity, source_kernel_id.to_string()))
    } else {
        None
    };
    if !from_daemon_id.trim().is_empty()
        && !matches!(
            &request,
            RelayPeerRequest::RefreshManagedSliceRelayToken { .. }
        )
    {
        state.write().await.remember_peer_public_key(
            stable_peer_daemon_id(from_daemon_id),
            requester_public_key.clone(),
        );
    }

    let response = match request {
        RelayPeerRequest::RoomBrowserController {
            session_id,
            slice_id,
            command,
        } => {
            match router
                .relay_room_browser_controller(
                    stable_peer_daemon_id(from_daemon_id),
                    &requester_public_key,
                    &session_id,
                    &slice_id,
                    command,
                )
                .await
            {
                Ok(result) => RelayPeerResponse::RoomBrowserController {
                    session_id,
                    slice_id,
                    result,
                },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    }
                }
            }
        }
        RelayPeerRequest::OpenRoomDisplay {
            session_id,
            slice_id,
            viewer_public_key,
        } => {
            match router
                .relay_open_room_display(
                    stable_peer_daemon_id(from_daemon_id),
                    &requester_public_key,
                    &session_id,
                    &slice_id,
                    viewer_public_key,
                )
                .await
            {
                Ok(endpoint) => RelayPeerResponse::RoomDisplayOpened {
                    session_id,
                    slice_id,
                    endpoint,
                },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    }
                }
            }
        }
        RelayPeerRequest::CaptureRoomScreenshot {
            session_id,
            slice_id,
        } => {
            match router
                .relay_capture_room_screenshot(
                    stable_peer_daemon_id(from_daemon_id),
                    &requester_public_key,
                    &session_id,
                    &slice_id,
                )
                .await
            {
                Ok(artifact) => RelayPeerResponse::RoomScreenshotCaptured {
                    session_id,
                    slice_id,
                    artifact,
                },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    }
                }
            }
        }
        RelayPeerRequest::ReadRoomScreenshotChunk {
            session_id,
            slice_id,
            artifact_id,
            offset,
            max_bytes,
        } => {
            match router.relay_read_room_screenshot_chunk(
                stable_peer_daemon_id(from_daemon_id),
                &requester_public_key,
                &session_id,
                &slice_id,
                &artifact_id,
                offset,
                max_bytes,
            ) {
                Ok(chunk) => RelayPeerResponse::RoomScreenshotChunk {
                    session_id,
                    slice_id,
                    chunk,
                },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    }
                }
            }
        }
        RelayPeerRequest::ObserveRoomComputer {
            session_id,
            slice_id,
            call,
        } => {
            match router
                .relay_observe_room_computer(
                    stable_peer_daemon_id(from_daemon_id),
                    &requester_public_key,
                    &session_id,
                    &slice_id,
                    call,
                )
                .await
            {
                Ok(result) => RelayPeerResponse::RoomComputerObserved {
                    session_id,
                    slice_id,
                    result: crate::transport::relay_peer::RemoteRoomComputerObservationResult(
                        result,
                    ),
                },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    }
                }
            }
        }
        RelayPeerRequest::Ping { value } => RelayPeerResponse::Pong { value, daemon_id },
        RelayPeerRequest::InstallManagedSliceRelayToken {
            slice_id,
            owner_kernel_id,
            owner_machine_id,
            activation_nonce,
            relay_token,
            expires_at_ms,
            relay_recovery_token,
            recovery_expires_at_ms,
        } => {
            let identity =
                require_bound_daemon_sender(caller_identity.as_ref(), &encrypted_request);
            let authorized = identity.is_ok_and(|identity| {
                stable_peer_daemon_id(from_daemon_id) == owner_kernel_id
                    && match identity.subject_kind {
                        chariox_relay::auth::RelaySubjectKind::Kernel => {
                            identity.subject == owner_kernel_id
                        }
                        chariox_relay::auth::RelaySubjectKind::Machine => {
                            identity.subject == owner_machine_id
                        }
                        _ => false,
                    }
            });
            if !authorized {
                RelayPeerResponse::ManagedSliceRelayTokenFailed {
                    code: "unauthorized".to_string(),
                    retryable: false,
                }
            } else {
                match router
                    .install_managed_slice_relay_token(
                        crate::runtime::router::ManagedSliceRelayTokenInstallRequest {
                            slice_id: slice_id.clone(),
                            owner_kernel_id: owner_kernel_id.clone(),
                            owner_machine_id: owner_machine_id.clone(),
                            relay_token: relay_token.into_inner(),
                            expires_at_ms,
                            relay_recovery_token: relay_recovery_token.into_inner(),
                            recovery_expires_at_ms,
                            owner_public_key: requester_public_key.clone(),
                        },
                    )
                    .await
                {
                    Ok(()) => {
                        state
                            .write()
                            .await
                            .stage_managed_slice_activation_confirmation(
                            super::connection_state::PendingManagedSliceActivationConfirmation::new(
                                slice_id.clone(),
                                owner_kernel_id.clone(),
                                requester_public_key.clone(),
                                daemon_id.clone(),
                                activation_nonce.clone(),
                            ),
                        );
                        RelayPeerResponse::ManagedSliceRelayTokenInstalled {
                            slice_id,
                            activation_nonce,
                            relay_peer_protocol_version: RELAY_PEER_PROTOCOL_VERSION,
                        }
                    }
                    Err(error) => managed_slice_token_failure_response(&error),
                }
            }
        }
        RelayPeerRequest::ConfirmManagedSliceRelayToken {
            slice_id,
            owner_kernel_id,
            worker_kernel_id,
            activation_nonce,
        } => {
            let identity =
                require_bound_kernel_sender(caller_identity.as_ref(), &encrypted_request);
            let authorized = match identity {
                Ok(identity)
                    if daemon_id == owner_kernel_id
                        && stable_peer_daemon_id(from_daemon_id) == worker_kernel_id =>
                {
                    state.write().await.confirm_managed_slice_relay_activation(
                        &slice_id,
                        &worker_kernel_id,
                        &identity.subject,
                        &requester_public_key,
                        &activation_nonce,
                    )
                }
                _ => false,
            };
            if authorized {
                RelayPeerResponse::ManagedSliceRelayTokenActivated {
                    slice_id,
                    activation_nonce,
                    relay_peer_protocol_version: RELAY_PEER_PROTOCOL_VERSION,
                }
            } else {
                RelayPeerResponse::ManagedSliceRelayTokenFailed {
                    code: "unauthorized".to_string(),
                    retryable: false,
                }
            }
        }
        RelayPeerRequest::RefreshManagedSliceRelayToken {
            slice_id,
            owner_kernel_id,
            worker_kernel_id,
        } => {
            let stable_worker_kernel_id = stable_peer_daemon_id(from_daemon_id);
            let identity = caller_identity.as_ref();
            let worker_identity = if stable_worker_kernel_id != worker_kernel_id {
                None
            } else if let Ok(bound) = require_bound_kernel_sender(identity, &encrypted_request) {
                Some(bound.subject.clone())
            } else {
                // The narrow bootstrap token is deliberately unkeyed. The router
                // claims this daemon id and encrypted sender key before it asks
                // Cloud for the key-bound runtime and recovery tokens.
                scoped_unbound_kernel_subject(identity)
            };
            if let Some(worker_relay_subject) = worker_identity {
                match router
                    .refresh_managed_slice_relay_token(
                        &slice_id,
                        &owner_kernel_id,
                        &worker_kernel_id,
                        &worker_relay_subject,
                        &requester_public_key,
                    )
                    .await
                {
                    Ok((
                        relay_token,
                        expires_at_ms,
                        relay_recovery_token,
                        recovery_expires_at_ms,
                    )) => RelayPeerResponse::ManagedSliceRelayTokenRefreshed {
                        slice_id,
                        relay_token: crate::transport::relay_peer::RelayManagedSliceToken::new(
                            relay_token,
                        ),
                        expires_at_ms,
                        relay_recovery_token:
                            crate::transport::relay_peer::RelayManagedSliceToken::new(
                                relay_recovery_token,
                            ),
                        recovery_expires_at_ms,
                        relay_peer_protocol_version: RELAY_PEER_PROTOCOL_VERSION,
                    },
                    Err(error) => managed_slice_token_failure_response(&error),
                }
            } else {
                RelayPeerResponse::ManagedSliceRelayTokenFailed {
                    code: "unauthorized".to_string(),
                    retryable: false,
                }
            }
        }
        RelayPeerRequest::CreateExecutionLease {
            home_kernel_id,
            home_session_id,
            home_agent_id,
            home_agent_metaagent,
            owner_user_id,
        } => {
            let lease = router
                .relay_create_execution_lease(
                    &home_kernel_id,
                    &home_session_id,
                    &home_agent_id,
                    home_agent_metaagent,
                    &owner_user_id,
                )
                .await;
            match lease {
                Ok(lease) => RelayPeerResponse::ExecutionLeaseCreated {
                    lease,
                    relay_peer_protocol_version: RELAY_PEER_PROTOCOL_VERSION,
                },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::DestroyExecutionLease { lease_id } => {
            let destroyed = router.relay_destroy_execution_lease(&lease_id).await;
            match destroyed {
                Ok(_) => RelayPeerResponse::ExecutionLeaseDestroyed { lease_id },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::SpawnLeasedAgent {
            lease_id,
            provider,
            account_profile,
            model,
            effort,
            execution_mode,
            permission_level,
            workspace_live_sync_mode,
            worktree_id,
            worktree_placement,
        } => {
            let leased_agent = router
                .relay_create_leased_agent(
                    &lease_id,
                    &provider,
                    &account_profile,
                    model,
                    effort,
                    execution_mode,
                    permission_level,
                    workspace_live_sync_mode,
                    worktree_id,
                    worktree_placement,
                )
                .await;
            match leased_agent {
                Ok(leased_agent) => RelayPeerResponse::LeasedAgentSpawned { leased_agent },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::DestroyLeasedAgent { leased_agent_id } => {
            let destroyed = router.relay_destroy_leased_agent(&leased_agent_id).await;
            match destroyed {
                Ok(_) => RelayPeerResponse::LeasedAgentDestroyed { leased_agent_id },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::UpdateLeasedAgentConfig {
            leased_agent_id,
            execution_mode,
            permission_level,
        } => {
            let updated = router
                .relay_update_leased_agent_config(
                    &leased_agent_id,
                    execution_mode,
                    permission_level,
                )
                .await;
            match updated {
                Ok(leased_agent) => RelayPeerResponse::LeasedAgentConfigUpdated { leased_agent },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::UpdateLeasedAgentProfile {
            leased_agent_id,
            provider,
            account_profile,
            model,
            effort,
        } => {
            let updated = router
                .relay_update_leased_agent_profile(
                    &leased_agent_id,
                    provider,
                    account_profile,
                    model,
                    effort,
                )
                .await;
            match updated {
                Ok(leased_agent) => RelayPeerResponse::LeasedAgentProfileUpdated { leased_agent },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::UpdateLeasedAgentMetaMode {
            leased_agent_id,
            active,
        } => {
            let updated = router
                .relay_update_leased_agent_meta_mode(&leased_agent_id, active)
                .await;
            match updated {
                Ok(leased_agent) => RelayPeerResponse::LeasedAgentMetaModeUpdated { leased_agent },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::UpdateLeasedAgentRemoteExtensionManifest {
            leased_agent_id,
            remote_extension_manifest,
        } => {
            let updated = router
                .relay_update_leased_agent_remote_extension_manifest(
                    &leased_agent_id,
                    remote_extension_manifest,
                )
                .await;
            match updated {
                Ok(()) => {
                    RelayPeerResponse::LeasedAgentRemoteExtensionManifestUpdated { leased_agent_id }
                }
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::LaunchLeasedNativeProviderRun {
            leased_agent_id,
            adapter_key,
            provider,
            account_profile,
            model,
            variant,
            structured_endpoint,
            provider_session_id,
            required_mcps,
            required_skills,
            remote_extension_manifest,
            provider_launch_credential,
        } => {
            let launched = router
                .relay_launch_leased_native_provider_run(
                    &leased_agent_id,
                    &adapter_key,
                    &provider,
                    &account_profile,
                    &model,
                    variant,
                    structured_endpoint,
                    provider_session_id,
                    required_mcps,
                    required_skills,
                    remote_extension_manifest,
                    provider_launch_credential,
                )
                .await;
            match launched {
                Ok(provider_run) => {
                    RelayPeerResponse::LeasedNativeProviderRunLaunched { provider_run }
                }
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::SendLeasedNativeProviderInput {
            leased_agent_id,
            provider_run_id,
            attachment_id,
            data_base64,
        } => {
            let sent = router
                .relay_send_leased_native_provider_input(
                    &leased_agent_id,
                    &provider_run_id,
                    &attachment_id,
                    &data_base64,
                )
                .await;
            match sent {
                Ok(byte_count) => RelayPeerResponse::LeasedNativeProviderInputSent { byte_count },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::ResizeLeasedProviderTerminal {
            leased_agent_id,
            provider_run_id,
            cols,
            rows,
        } => {
            let resized = router
                .relay_resize_leased_provider_terminal(
                    &leased_agent_id,
                    &provider_run_id,
                    cols,
                    rows,
                )
                .await;
            match resized {
                Ok(()) => RelayPeerResponse::LeasedProviderTerminalResized {
                    provider_run_id,
                    cols,
                    rows,
                },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::SubmitLeasedPrompt {
            leased_agent_id,
            prompt,
            hidden_system_context,
            attachments,
            workflow_context,
            git_context,
            required_mcps,
            required_skills,
            remote_extension_manifest,
            provider_launch_credential,
        } => {
            let submitted = router
                .relay_submit_leased_prompt(
                    &leased_agent_id,
                    &prompt,
                    &hidden_system_context,
                    attachments,
                    workflow_context,
                    git_context,
                    required_mcps,
                    required_skills,
                    remote_extension_manifest,
                    provider_launch_credential,
                )
                .await;
            match submitted {
                Ok((provider_run_id, outcome)) => {
                    if let Err(error) = emit_leased_projection_event(
                        router,
                        state,
                        outgoing_tx,
                        &leased_agent_id,
                        &provider_run_id,
                        true,
                    )
                    .await
                    {
                        crate::logging::warn_with_fields(
                            "daemon.relay",
                            "failed to emit leased runtime projection after submit",
                            serde_json::json!({
                                "leased_agent_id": leased_agent_id,
                                "provider_run_id": provider_run_id,
                                "error": error.to_string(),
                            }),
                        );
                    }
                    RelayPeerResponse::LeasedPromptSubmitted {
                        provider_run_id,
                        outcome,
                    }
                }
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::SteerLeasedPrompt {
            leased_agent_id,
            steer_id,
            target_home_prompt_id,
            prompt,
            hidden_system_context,
            attachments,
            required_skills,
        } => {
            let steered = router
                .relay_steer_leased_prompt(
                    &leased_agent_id,
                    &steer_id,
                    &target_home_prompt_id,
                    &prompt,
                    &hidden_system_context,
                    attachments,
                    required_skills,
                )
                .await;
            match steered {
                Ok((provider_run_id, replayed)) => {
                    if let Err(error) = emit_leased_projection_event(
                        router,
                        state,
                        outgoing_tx,
                        &leased_agent_id,
                        &provider_run_id,
                        true,
                    )
                    .await
                    {
                        crate::logging::warn_with_fields(
                            "daemon.relay",
                            "failed to emit leased runtime projection after steer",
                            serde_json::json!({
                                "leased_agent_id": leased_agent_id,
                                "provider_run_id": provider_run_id,
                                "steer_id": steer_id,
                                "error": error.to_string(),
                            }),
                        );
                    }
                    RelayPeerResponse::LeasedPromptSteered {
                        provider_run_id,
                        steer_id,
                        replayed,
                    }
                }
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::DrainLeasedRuntimeProjection {
            leased_agent_id,
            provider_run_id,
            pump_output,
        } => {
            let drained = router
                .relay_drain_leased_runtime_projection(
                    &leased_agent_id,
                    &provider_run_id,
                    pump_output,
                    true,
                )
                .await;
            match drained {
                Ok(event) => RelayPeerResponse::LeasedRuntimeProjectionDrained {
                    event: event.map(|(_target_daemon_id, event)| event),
                },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::CompleteLeasedPrompt { leased_agent_id } => {
            let completion = router.relay_complete_leased_prompt(&leased_agent_id).await;
            match completion {
                Ok(completion) => {
                    let provider_run_id = router
                        .relay_leased_agent_provider_run_id(&leased_agent_id)
                        .await
                        .ok()
                        .flatten();
                    let provider_diagnostic =
                        if let Some(provider_run_id) = provider_run_id.as_deref() {
                            router
                                .relay_provider_run_terminal_diagnostic(provider_run_id)
                                .await
                                .ok()
                                .flatten()
                        } else {
                            None
                        };
                    let (git_observations, workspace_live_sync_change) =
                        if let Some(provider_run_id) = provider_run_id.as_deref() {
                            router
                                .relay_observe_leased_git_after(&leased_agent_id, provider_run_id)
                                .await
                                .unwrap_or_default()
                        } else {
                            (Vec::new(), None)
                        };
                    RelayPeerResponse::LeasedPromptCompleted {
                        provider_run_id,
                        provider_diagnostic,
                        git_observations,
                        workspace_live_sync_change,
                        completion,
                    }
                }
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::ObserveLeasedGitAfter {
            leased_agent_id,
            provider_run_id,
        } => match router
            .relay_observe_leased_git_after(&leased_agent_id, &provider_run_id)
            .await
        {
            Ok((git_observations, workspace_live_sync_change)) => {
                RelayPeerResponse::LeasedGitObserved {
                    provider_run_id,
                    git_observations,
                    workspace_live_sync_change,
                }
            }
            Err(error) => {
                return RelayRequestOutcome {
                    encrypted_response: None,
                    error: Some(map_relay_error(&error)),
                };
            }
        },
        RelayPeerRequest::CancelLeasedPrompt { leased_agent_id } => {
            let cancellation = router.relay_cancel_leased_prompt(&leased_agent_id).await;
            match cancellation {
                Ok(cancellation) => RelayPeerResponse::LeasedPromptCancelled { cancellation },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::ForwardWorkflowRuntimeTool {
            context,
            tool_name,
            arguments,
        } => {
            let handled = router
                .dispatch_forwarded_workflow_runtime_tool_call(context, tool_name, arguments)
                .await;
            match handled {
                Ok(result) => RelayPeerResponse::WorkflowRuntimeToolHandled { result },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::ForwardWorkflowProviderFailure { context, message } => {
            let handled = router
                .dispatch_forwarded_workflow_provider_failure(context, message)
                .await;
            match handled {
                Ok(()) => RelayPeerResponse::WorkflowProviderFailureHandled,
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::ForwardWorkspaceLiveSyncRuntimeTool {
            context,
            metadata,
            tool_name,
            arguments,
            artifact_states,
        } => {
            let handled = router
                .dispatch_forwarded_workspace_live_sync_runtime_tool_call(
                    context,
                    metadata,
                    tool_name,
                    arguments,
                    artifact_states,
                )
                .await;
            match handled {
                Ok((result, final_artifact_states)) => {
                    RelayPeerResponse::WorkspaceLiveSyncRuntimeToolHandled {
                        result,
                        final_artifact_states,
                    }
                }
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::FinalizeWorkspaceLiveSyncRuntimeTool {
            context,
            metadata,
            tool_name,
            arguments,
            initial_artifact_states,
            final_artifact_states,
        } => {
            let finalized = router
                .finalize_forwarded_workspace_live_sync_runtime_tool_call(
                    context,
                    metadata,
                    tool_name,
                    arguments,
                    initial_artifact_states,
                    final_artifact_states,
                )
                .await;
            match finalized {
                Ok(()) => RelayPeerResponse::WorkspaceLiveSyncRuntimeToolFinalized,
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::ForwardCapabilityRuntimeTool {
            context,
            tool_name,
            arguments,
        } => {
            let handled = router
                .dispatch_forwarded_capability_runtime_tool_call(context, tool_name, arguments)
                .await;
            match handled {
                Ok((result, skill_package, remote_extension_manifest)) => {
                    RelayPeerResponse::CapabilityRuntimeToolHandled {
                        result,
                        skill_package,
                        remote_extension_manifest,
                    }
                }
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::ForwardMetaRuntimeTool {
            context,
            tool_name,
            arguments,
        } => {
            let handled = router
                .dispatch_forwarded_meta_runtime_tool_call(context, tool_name, arguments)
                .await;
            match handled {
                Ok(result) => RelayPeerResponse::MetaRuntimeToolHandled { result },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::ForwardRoomBrowserRuntimeTool { context, call } => {
            let handled = router
                .dispatch_forwarded_room_browser_runtime_tool_call(
                    stable_peer_daemon_id(from_daemon_id),
                    context,
                    call,
                )
                .await;
            match handled {
                Ok(result) => RelayPeerResponse::RoomBrowserRuntimeToolHandled {
                    result: crate::transport::relay_peer::RemoteRoomBrowserRuntimeToolResult(
                        result,
                    ),
                },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::InvokeHomeExtensionTool {
            context,
            metadata,
            tool,
            arguments,
        } => {
            let handled = router
                .dispatch_forwarded_home_extension_tool_call(context, metadata, tool, arguments)
                .await;
            match handled {
                Ok(result) => RelayPeerResponse::HomeExtensionToolHandled { result },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::InvokeHomeMcpProxy {
            context,
            metadata,
            name,
            tool,
            payload,
        } => {
            let handled = router
                .dispatch_forwarded_home_mcp_proxy_call(context, metadata, name, tool, payload)
                .await;
            match handled {
                Ok(response) => RelayPeerResponse::HomeMcpProxyHandled { response },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::CancelHomeExtensionInvocation { context, metadata } => {
            let invocation_id = metadata.invocation_id.clone();
            let cancelled = router
                .cancel_forwarded_home_extension_invocation(context, metadata)
                .await;
            match cancelled {
                Ok(cancelled) => RelayPeerResponse::HomeExtensionInvocationCancelled {
                    invocation_id,
                    cancelled,
                },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::InvokeHomeCredentialTool {
            context,
            tool_name,
            arguments,
        } => {
            let handled = router
                .dispatch_forwarded_home_credential_tool_call(context, tool_name, arguments)
                .await;
            match handled {
                Ok(result) => RelayPeerResponse::HomeCredentialToolHandled { result },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::ResolveHomeCredentialSecret {
            context,
            credential_id,
            injection,
        } => {
            let resolved = router
                .resolve_forwarded_home_credential_secret(context, credential_id, injection)
                .await;
            match resolved {
                Ok((credential_id, secret_input)) => {
                    RelayPeerResponse::HomeCredentialSecretResolved {
                        credential_id,
                        secret_input:
                            crate::transport::relay_peer::RemoteCredentialSecretInput::new(
                                secret_input,
                            ),
                    }
                }
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::ApplyWorkspaceLiveSyncChange { context, change } => {
            let applied = router
                .relay_apply_workspace_live_sync_change(context, change)
                .await;
            match applied {
                Ok(target_result) => {
                    RelayPeerResponse::WorkspaceLiveSyncChangeApplied { target_result }
                }
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::ForwardNativeInteraction {
            context,
            interaction,
        } => {
            let handled = router
                .relay_forward_native_interaction(context, interaction)
                .await;
            match handled {
                Ok(resolution) => RelayPeerResponse::NativeInteractionResolved { resolution },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::EnsureRemoteSkillPackages { context, packages } => {
            let ensured = router
                .relay_ensure_remote_skill_packages(context, packages)
                .await;
            match ensured {
                Ok(materialized) => RelayPeerResponse::RemoteSkillPackagesEnsured { materialized },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::EnsureRemoteProviderAccount {
            context,
            materialization,
        } => {
            let ensured = router
                .relay_ensure_remote_provider_account(context, materialization)
                .await;
            match ensured {
                Ok(profile) => RelayPeerResponse::RemoteProviderAccountEnsured {
                    provider: profile.provider,
                    account_profile: profile.profile_id,
                },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::CheckRemoteMcpAvailability {
            context,
            required_mcps,
        } => {
            let checked = router
                .relay_check_remote_mcp_availability(context, required_mcps)
                .await;
            match checked {
                Ok(results) => RelayPeerResponse::RemoteMcpAvailabilityChecked { results },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::ArmManagedContextImport {
            context_id,
            plan_digest,
            target_environment_id,
            target_kernel_id,
            target_key_thumbprint,
            capability,
            archive_sha256,
            archive_size_bytes,
        } => {
            let (identity, source_kernel_id) = managed_context_caller
                .clone()
                .expect("managed context caller checked before dispatch");
            let result = router
                .relay_arm_managed_context_import(
                    crate::runtime::router::RelayManagedContextArmRequest {
                        identity,
                        source_kernel_id,
                        context_id,
                        plan_digest,
                        target_environment_id,
                        target_kernel_id,
                        target_key_thumbprint,
                        capability: capability.into_inner(),
                        archive_sha256,
                        archive_size_bytes,
                    },
                )
                .await;
            match result {
                Ok(response) => response,
                Err(error) => managed_context_failure_response(&error),
            }
        }
        RelayPeerRequest::BeginManagedContextImport {
            transfer_id,
            capability,
        } => {
            let (identity, source_kernel_id) = managed_context_caller
                .clone()
                .expect("managed context caller checked before dispatch");
            let result = router
                .relay_begin_managed_context_import(
                    identity,
                    source_kernel_id,
                    transfer_id,
                    capability.into_inner(),
                )
                .await;
            match result {
                Ok(response) => response,
                Err(error) => managed_context_failure_response(&error),
            }
        }
        RelayPeerRequest::UploadManagedContextChunk {
            transfer_id,
            capability,
            offset,
            data_base64,
            chunk_sha256,
        } => {
            const MAX_ENCODED_CHUNK_BYTES: usize =
                crate::managed_context::transfer::MAX_TRANSFER_CHUNK_BYTES.div_ceil(3) * 4;
            let data_base64 = data_base64.into_inner();
            let bytes = if data_base64.len() > MAX_ENCODED_CHUNK_BYTES {
                None
            } else {
                match base64::engine::general_purpose::STANDARD.decode(data_base64) {
                    Ok(bytes)
                        if !bytes.is_empty()
                            && bytes.len()
                                <= crate::managed_context::transfer::MAX_TRANSFER_CHUNK_BYTES =>
                    {
                        Some(bytes)
                    }
                    _ => None,
                }
            };
            let Some(bytes) = bytes else {
                return encrypt_peer_response(
                    &daemon_private_key,
                    &requester_public_key,
                    RelayPeerResponse::ManagedContextImportFailed {
                        code: "invalid_request".to_string(),
                        retryable: false,
                    },
                );
            };
            let (identity, source_kernel_id) = managed_context_caller
                .clone()
                .expect("managed context caller checked before dispatch");
            let result = router
                .relay_upload_managed_context_chunk(
                    crate::runtime::router::RelayManagedContextChunkRequest {
                        identity,
                        source_kernel_id,
                        transfer_id,
                        capability: capability.into_inner(),
                        offset,
                        bytes,
                        chunk_sha256,
                    },
                )
                .await;
            match result {
                Ok(response) => response,
                Err(error) => managed_context_failure_response(&error),
            }
        }
        RelayPeerRequest::FinalizeManagedContextImport {
            transfer_id,
            capability,
        } => {
            let (identity, source_kernel_id) = managed_context_caller
                .clone()
                .expect("managed context caller checked before dispatch");
            let result = router
                .relay_finalize_managed_context_import(
                    identity,
                    source_kernel_id,
                    transfer_id,
                    capability.into_inner(),
                )
                .await;
            match result {
                Ok(response) => response,
                Err(error) => managed_context_failure_response(&error),
            }
        }
        RelayPeerRequest::GetManagedContextImportStatus {
            transfer_id,
            capability,
        } => {
            let (identity, source_kernel_id) = managed_context_caller
                .clone()
                .expect("managed context caller checked before dispatch");
            let result = router
                .relay_get_managed_context_import_status(
                    identity,
                    source_kernel_id,
                    transfer_id,
                    capability.into_inner(),
                )
                .await;
            match result {
                Ok(response) => response,
                Err(error) => managed_context_failure_response(&error),
            }
        }
    };
    encrypt_peer_response(&daemon_private_key, &requester_public_key, response)
}

fn managed_context_failure_response(error: &crate::error::DaemonError) -> RelayPeerResponse {
    let projected = map_relay_error(error);
    RelayPeerResponse::ManagedContextImportFailed {
        code: projected.code,
        retryable: projected.retryable,
    }
}

fn managed_slice_token_failure_response(error: &crate::error::DaemonError) -> RelayPeerResponse {
    let projected = map_relay_error(error);
    RelayPeerResponse::ManagedSliceRelayTokenFailed {
        code: projected.code,
        retryable: projected.retryable,
    }
}

fn managed_context_failure_from_relay(
    error: &chariox_relay::protocol::RelayError,
) -> RelayPeerResponse {
    RelayPeerResponse::ManagedContextImportFailed {
        code: error.code.clone(),
        retryable: error.retryable,
    }
}

fn encrypt_peer_response(
    daemon_private_key: &str,
    requester_public_key: &str,
    response: RelayPeerResponse,
) -> RelayRequestOutcome {
    let plaintext = match serde_json::to_vec(&response) {
        Ok(bytes) => bytes,
        Err(error) => {
            return RelayRequestOutcome {
                encrypted_response: None,
                error: Some(relay_error(
                    "relay_request_failed",
                    &format!("failed to serialize relay peer response: {error}"),
                    false,
                )),
            };
        }
    };
    match relay_crypto::encrypt_payload_for_peer(
        daemon_private_key,
        requester_public_key,
        &plaintext,
    ) {
        Ok(encrypted_response) => RelayRequestOutcome {
            encrypted_response: Some(encrypted_response),
            error: None,
        },
        Err(error) => RelayRequestOutcome {
            encrypted_response: None,
            error: Some(relay_error(
                "relay_request_failed",
                &format!("failed to encrypt relay peer response: {error}"),
                false,
            )),
        },
    }
}

fn stable_peer_daemon_id(from_daemon_id: &str) -> &str {
    from_daemon_id
        .split_once(":peer-tmp:daemon-peer-tmp-")
        .map_or(from_daemon_id, |(daemon_id, _)| daemon_id)
}

fn scoped_unbound_kernel_subject(identity: Option<&RelayCallerIdentity>) -> Option<String> {
    identity
        .filter(|identity| {
            identity.subject_kind == chariox_relay::auth::RelaySubjectKind::Kernel
                && identity.public_key_thumbprint.is_none()
                && identity.token_id.is_some()
                && identity.expires_at_ms > crate::session::unix_epoch_ms()
        })
        .map(|identity| identity.subject.clone())
}

fn managed_context_request(request: &RelayPeerRequest) -> bool {
    matches!(
        request,
        RelayPeerRequest::ArmManagedContextImport { .. }
            | RelayPeerRequest::BeginManagedContextImport { .. }
            | RelayPeerRequest::UploadManagedContextChunk { .. }
            | RelayPeerRequest::FinalizeManagedContextImport { .. }
            | RelayPeerRequest::GetManagedContextImportStatus { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use base64::Engine;
    use chariox_relay::auth::RelaySubjectKind;
    use sha2::{Digest, Sha256};
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::process::Command;
    use tokio::sync::Mutex;

    use crate::config::PersistedCloudRelayProfile;
    use crate::error::DaemonError;
    use crate::managed_bootstrap::{ConfirmedManagedKernelRegistration, ManagedKernelContextPlan};
    use crate::managed_context::development::{
        export_development_context, DevelopmentContextExportRequest, DevelopmentRepositoryRole,
        DevelopmentRepositorySelection,
    };
    use crate::managed_context::kernel::{
        KernelContextCompatibility, KernelContextPayload, KernelContextSnapshot,
    };
    use crate::managed_context::package::{
        apply_managed_context_package, export_managed_context_package,
        ManagedContextPackageApplicationRequest, ManagedContextPackageBinding,
        ManagedContextPackageDevelopment, ManagedContextPackageExportRequest,
        ManagedContextPackageKernel, ManagedContextPackageProviderAccounts,
    };
    use crate::runtime::terminal_pairings::public_key_thumbprint;
    use crate::secret::{
        export_transferred_vault_snapshot, lock_chariox_encrypted_vault,
        unlock_chariox_encrypted_vault, VaultUnlockLease,
    };
    use crate::transport::relay_peer::{
        RelayManagedContextCapability, RelayManagedContextChunk, RelayManagedContextTransferPhase,
    };
    use crate::{DaemonApp, DaemonConfig};

    fn scoped_kernel_identity(
        public_key_thumbprint: Option<String>,
        expires_at_ms: u64,
    ) -> RelayCallerIdentity {
        RelayCallerIdentity {
            realm_id: "realm-1".to_string(),
            subject: "source-kernel-1".to_string(),
            subject_kind: RelaySubjectKind::Kernel,
            expires_at_ms,
            token_id: Some("token-1".to_string()),
            user_id: Some("user-1".to_string()),
            public_key_thumbprint,
        }
    }

    fn scoped_machine_identity(
        subject: &str,
        public_key_thumbprint: Option<String>,
    ) -> RelayCallerIdentity {
        RelayCallerIdentity {
            realm_id: "realm-1".to_string(),
            subject: subject.to_string(),
            subject_kind: RelaySubjectKind::Machine,
            expires_at_ms: u64::MAX,
            token_id: Some("token-1".to_string()),
            user_id: Some("user-1".to_string()),
            public_key_thumbprint,
        }
    }

    #[test]
    fn slice_bootstrap_requires_a_scoped_live_unbound_kernel_identity() {
        let live = scoped_kernel_identity(None, u64::MAX);
        assert_eq!(
            scoped_unbound_kernel_subject(Some(&live)).as_deref(),
            Some("source-kernel-1")
        );

        let mut legacy = live.clone();
        legacy.expires_at_ms = 0;
        legacy.token_id = None;
        assert!(scoped_unbound_kernel_subject(Some(&legacy)).is_none());

        let mut expired = live.clone();
        expired.expires_at_ms = 1;
        assert!(scoped_unbound_kernel_subject(Some(&expired)).is_none());

        let mut bound = live;
        bound.public_key_thumbprint = Some("bound-key".to_string());
        assert!(scoped_unbound_kernel_subject(Some(&bound)).is_none());
    }

    #[tokio::test]
    async fn peer_handler_rejects_invalid_scoped_kernel_identity_before_decryption() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("test daemon should bootstrap"),
        ));
        let router = Arc::new(CommandRouter::with_interactive_capacity(app, 1));
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        let (outgoing_tx, _priority_rx, _event_rx) = RelayOutgoingSender::channel(1);
        let malformed_request = EncryptedRelayPayload {
            sender_public_key: "request-sender-public-key".to_string(),
            nonce: "not-valid-base64".to_string(),
            ciphertext: "not-valid-base64".to_string(),
        };
        let identities = [
            scoped_kernel_identity(
                Some(public_key_thumbprint("different-public-key")),
                u64::MAX,
            ),
            scoped_kernel_identity(None, 1),
        ];

        for identity in identities {
            let outcome = handle_daemon_peer_request(
                &router,
                &state,
                &outgoing_tx,
                "source-kernel-1",
                Some(identity),
                malformed_request.clone(),
            )
            .await;
            let error = outcome
                .error
                .expect("invalid scoped identity should be rejected");
            assert_eq!(error.code, "unauthorized");
            assert!(!error.retryable);
            assert!(outcome.encrypted_response.is_none());
        }
    }

    #[tokio::test]
    async fn managed_context_peer_request_requires_a_bound_kernel_identity() {
        let app =
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("test daemon should bootstrap");
        let target_public_key = app.config().relay_public_key.clone();
        let app = Arc::new(Mutex::new(app));
        let router = Arc::new(CommandRouter::with_interactive_capacity(app, 1));
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        let (outgoing_tx, _priority_rx, _event_rx) = RelayOutgoingSender::channel(1);
        let request = RelayPeerRequest::ArmManagedContextImport {
            context_id: "context-1".to_string(),
            plan_digest: format!("sha256:{}", "f".repeat(64)),
            target_environment_id: "environment-1".to_string(),
            target_kernel_id: "target-kernel-1".to_string(),
            target_key_thumbprint: "a".repeat(64),
            capability: RelayManagedContextCapability::new("c".repeat(43)),
            archive_sha256: "b".repeat(64),
            archive_size_bytes: 42,
        };
        let source_private_key = relay_crypto::generate_private_key_base64();
        let encrypted_request = relay_crypto::encrypt_payload_for_peer(
            &source_private_key,
            &target_public_key,
            &serde_json::to_vec(&request).expect("serialize managed context request"),
        )
        .expect("encrypt managed context request");

        let outcome = handle_daemon_peer_request(
            &router,
            &state,
            &outgoing_tx,
            "source-kernel-1",
            None,
            encrypted_request,
        )
        .await;
        assert!(outcome.error.is_none());
        let encrypted_response = outcome
            .encrypted_response
            .expect("identity rejection should stay encrypted");
        let decrypted =
            relay_crypto::decrypt_payload_for_private_key(&source_private_key, &encrypted_response)
                .expect("decrypt identity rejection");
        let response: RelayPeerResponse =
            serde_json::from_slice(&decrypted.plaintext).expect("decode identity rejection");
        assert!(matches!(
            response,
            RelayPeerResponse::ManagedContextImportFailed {
                ref code,
                retryable: false,
            } if code == "unauthorized"
        ));
    }

    #[tokio::test]
    async fn managed_slice_activation_confirmation_requires_bound_runtime_identity_and_nonce() {
        let config = DaemonConfig::for_tests();
        let owner_kernel_id = config.daemon_id.clone();
        let owner_public_key = config.relay_public_key.clone();
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config).expect("test daemon should bootstrap"),
        ));
        let router = Arc::new(CommandRouter::with_interactive_capacity(app, 1));
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        let (outgoing_tx, _priority_rx, _event_rx) = RelayOutgoingSender::channel(1);
        let peer_harness = ManagedPeerRequestHarness {
            router: &router,
            state: &state,
            outgoing_tx: &outgoing_tx,
        };
        let worker_private_key = relay_crypto::generate_private_key_base64();
        let worker_public_key =
            relay_crypto::public_key_from_private_key_base64(&worker_private_key)
                .expect("worker public key");
        state.write().await.begin_managed_slice_relay_activation(
            "slice-1".to_string(),
            "source-kernel-1".to_string(),
            "slice:dev".to_string(),
            worker_public_key.clone(),
            "activation-1".to_string(),
        );
        let request = |activation_nonce: &str| RelayPeerRequest::ConfirmManagedSliceRelayToken {
            slice_id: "slice-1".to_string(),
            owner_kernel_id: owner_kernel_id.clone(),
            worker_kernel_id: "source-kernel-1".to_string(),
            activation_nonce: activation_nonce.to_string(),
        };

        let mut unbound_identity = scoped_kernel_identity(None, u64::MAX);
        unbound_identity.subject = "slice:dev".to_string();
        let unbound = send_managed_peer_request(
            &peer_harness,
            "source-kernel-1",
            &unbound_identity,
            &worker_private_key,
            &owner_public_key,
            request("activation-1"),
        )
        .await;
        assert!(matches!(
            unbound,
            RelayPeerResponse::ManagedSliceRelayTokenFailed {
                ref code,
                retryable: false,
            } if code == "unauthorized"
        ));

        let mut bound_identity =
            scoped_kernel_identity(Some(public_key_thumbprint(&worker_public_key)), u64::MAX);
        bound_identity.subject = "slice:dev".to_string();
        let wrong_nonce = send_managed_peer_request(
            &peer_harness,
            "source-kernel-1",
            &bound_identity,
            &worker_private_key,
            &owner_public_key,
            request("activation-2"),
        )
        .await;
        assert!(matches!(
            wrong_nonce,
            RelayPeerResponse::ManagedSliceRelayTokenFailed {
                ref code,
                retryable: false,
            } if code == "unauthorized"
        ));
        assert!(!state
            .read()
            .await
            .managed_slice_relay_activation_confirmed("slice-1", "activation-1"));

        let confirmed = send_managed_peer_request(
            &peer_harness,
            "source-kernel-1",
            &bound_identity,
            &worker_private_key,
            &owner_public_key,
            request("activation-1"),
        )
        .await;
        assert!(matches!(
            confirmed,
            RelayPeerResponse::ManagedSliceRelayTokenActivated {
                ref slice_id,
                ref activation_nonce,
                relay_peer_protocol_version: RELAY_PEER_PROTOCOL_VERSION,
            } if slice_id == "slice-1" && activation_nonce == "activation-1"
        ));
        assert!(state
            .read()
            .await
            .managed_slice_relay_activation_confirmed("slice-1", "activation-1"));
    }

    #[tokio::test]
    async fn managed_slice_bootstrap_exchange_binds_the_first_worker_key() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind Cloud token fixture");
        let address = listener.local_addr().expect("Cloud token fixture address");
        let fixture = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.expect("accept token request");
                let mut request = Vec::new();
                loop {
                    let mut chunk = [0_u8; 4096];
                    let read = tokio::io::AsyncReadExt::read(&mut stream, &mut chunk)
                        .await
                        .expect("read token request");
                    assert!(read > 0, "token request ended before its body");
                    request.extend_from_slice(&chunk[..read]);
                    let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n")
                    else {
                        continue;
                    };
                    let headers = String::from_utf8_lossy(&request[..header_end]);
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|value| value.trim().parse::<usize>().ok())
                        })
                        .expect("token request content length");
                    if request.len() >= header_end + 4 + content_length {
                        break;
                    }
                }
                let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
                    serde_json::to_vec(&serde_json::json!({
                        "exp": crate::session::unix_epoch_ms() / 1_000 + 3_600,
                    }))
                    .expect("encode fixture token claims"),
                );
                let body = format!(
                    r#"{{"token":"header.{claims}.signature","expiresAt":"2099-01-01T00:00:00Z"}}"#
                );
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body,
                );
                tokio::io::AsyncWriteExt::write_all(&mut stream, response.as_bytes())
                    .await
                    .expect("write token response");
            }
        });

        let root = std::env::temp_dir().join(format!(
            "chariox-slice-bootstrap-exchange-{}-{}",
            std::process::id(),
            rand::random::<u64>(),
        ));
        let mut config = DaemonConfig::for_tests();
        config = config.with_session_history_root(root.join("sessions"));
        config.user_config.history.operational.path =
            Some(root.join("operational.db").display().to_string());
        config.user_config.artifacts.operational.root =
            Some(root.join("artifacts").display().to_string());
        config.user_config.artifacts.operational.index_path =
            Some(root.join("artifacts.db").display().to_string());
        config.user_config.state.path = Some(root.join("kernel/state.db").display().to_string());
        let owner_kernel_id = config.daemon_id.clone();
        let owner_public_key = config.relay_public_key.clone();
        config.cloud_relay = Some(test_cloud_profile(
            format!("http://{address}"),
            config.host_machine_id.clone(),
        ));
        let app = DaemonApp::bootstrap(config).expect("test daemon should bootstrap");
        let state = app.relay_client_state();
        let app = Arc::new(Mutex::new(app));
        let router = Arc::new(CommandRouter::with_interactive_capacity(app, 1));
        let slice = router
            .runtime_state()
            .create_slice(crate::local::CreateSliceRequest {
                name: "bootstrap-exchange".to_string(),
                backend: crate::slice::SliceBackendKind::LocalDocker,
                os: "linux".to_string(),
                display_mode: crate::slice::SliceDisplayMode::Headless,
                display_backend: crate::slice::SliceDisplayBackend::default(),
                workspace_id: None,
                worktree_id: None,
                workspace_mount: None,
                development: None,
                worker_kernel_ref: Some("slice:bootstrap-exchange".to_string()),
                display_url: None,
                provider_auth: Vec::new(),
                from_saved_state: None,
                base: Some(crate::local::SliceCreateBase::Clean),
            })
            .await
            .expect("slice should create");
        router
            .runtime_state()
            .mark_slice_starting(
                &slice.id,
                crate::slice::SliceRelayEndpoint {
                    url: "wss://relay.example.test".to_string(),
                    private: false,
                },
            )
            .expect("slice should enter starting state");

        let (outgoing_tx, _priority_rx, _event_rx) = RelayOutgoingSender::channel(1);
        let peer_harness = ManagedPeerRequestHarness {
            router: &router,
            state: &state,
            outgoing_tx: &outgoing_tx,
        };
        let worker_kernel_id = "kernel-bootstrap-worker";
        let worker_private_key = relay_crypto::generate_private_key_base64();
        let worker_public_key =
            relay_crypto::public_key_from_private_key_base64(&worker_private_key)
                .expect("worker public key");
        let mut bootstrap_identity = scoped_kernel_identity(None, u64::MAX);
        bootstrap_identity.subject = slice.worker_kernel_ref.clone();

        let conflicting_worker_id = "kernel-conflicting-worker";
        state
            .write()
            .await
            .remember_peer_public_key(conflicting_worker_id, "already-bound-key".to_string());
        let conflicting_private_key = relay_crypto::generate_private_key_base64();
        let rejected = send_managed_peer_request(
            &peer_harness,
            conflicting_worker_id,
            &bootstrap_identity,
            &conflicting_private_key,
            &owner_public_key,
            RelayPeerRequest::RefreshManagedSliceRelayToken {
                slice_id: slice.id.clone(),
                owner_kernel_id: owner_kernel_id.clone(),
                worker_kernel_id: conflicting_worker_id.to_string(),
            },
        )
        .await;
        assert!(matches!(
            rejected,
            RelayPeerResponse::ManagedSliceRelayTokenFailed { .. }
        ));
        assert!(
            router
                .runtime_state()
                .resolve_slice(&slice.id)
                .expect("slice should remain present")
                .worker_kernel_id
                .is_none(),
            "a rejected key claim must not durably bind the slice worker"
        );

        let response = send_managed_peer_request(
            &peer_harness,
            worker_kernel_id,
            &bootstrap_identity,
            &worker_private_key,
            &owner_public_key,
            RelayPeerRequest::RefreshManagedSliceRelayToken {
                slice_id: slice.id.clone(),
                owner_kernel_id,
                worker_kernel_id: worker_kernel_id.to_string(),
            },
        )
        .await;

        assert!(
            matches!(
                &response,
                RelayPeerResponse::ManagedSliceRelayTokenRefreshed {
                    slice_id,
                    relay_peer_protocol_version: RELAY_PEER_PROTOCOL_VERSION,
                    ..
                } if slice_id == &slice.id
            ),
            "unexpected bootstrap exchange response: {response:?}"
        );
        let claimed = router
            .runtime_state()
            .resolve_slice(&slice.id)
            .expect("slice should remain present");
        assert_eq!(claimed.worker_kernel_id.as_deref(), Some(worker_kernel_id));
        assert_eq!(
            state
                .read()
                .await
                .peer_public_key(worker_kernel_id)
                .as_deref(),
            Some(worker_public_key.as_str())
        );

        let replacement_private_key = relay_crypto::generate_private_key_base64();
        let replacement_public_key =
            relay_crypto::public_key_from_private_key_base64(&replacement_private_key)
                .expect("replacement public key");
        let replay = send_managed_peer_request(
            &peer_harness,
            worker_kernel_id,
            &bootstrap_identity,
            &replacement_private_key,
            &owner_public_key,
            RelayPeerRequest::RefreshManagedSliceRelayToken {
                slice_id: slice.id.clone(),
                owner_kernel_id: router.relay_daemon_id(),
                worker_kernel_id: worker_kernel_id.to_string(),
            },
        )
        .await;
        assert!(matches!(
            replay,
            RelayPeerResponse::ManagedSliceRelayTokenFailed {
                retryable: true,
                ..
            }
        ));
        assert_ne!(replacement_public_key, worker_public_key);
        assert_eq!(
            state
                .read()
                .await
                .peer_public_key(worker_kernel_id)
                .as_deref(),
            Some(worker_public_key.as_str())
        );

        fixture.await.expect("Cloud token fixture should finish");
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn managed_context_peer_request_accepts_selected_machine_and_kernel_binding() {
        let root = std::env::temp_dir().join(format!(
            "chariox-managed-machine-peer-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let mut config = DaemonConfig::for_tests();
        config = config.with_session_history_root(root.join("sessions"));
        config.user_config.history.operational.path =
            Some(root.join("operational.db").display().to_string());
        config.user_config.artifacts.operational.root =
            Some(root.join("artifacts").display().to_string());
        config.user_config.artifacts.operational.index_path =
            Some(root.join("artifacts.db").display().to_string());
        config.user_config.state.path = Some(root.join("kernel/state.db").display().to_string());
        let target_kernel_id = config.daemon_id.clone();
        let target_machine_id = config.host_machine_id.clone();
        config.cloud_relay = Some(test_cloud_profile(
            "http://127.0.0.1:1".to_string(),
            target_machine_id.clone(),
        ));
        let target_public_key = config.relay_public_key.clone();
        let target_key_thumbprint = public_key_thumbprint(&target_public_key);

        let source_private_key = relay_crypto::generate_private_key_base64();
        let source_public_key =
            relay_crypto::public_key_from_private_key_base64(&source_private_key)
                .expect("source public key");
        let source_key_thumbprint = public_key_thumbprint(&source_public_key);
        let source_kernel_id = "source-kernel-1";
        let source_machine_id = "source-machine-test";
        let identity =
            scoped_machine_identity(source_machine_id, Some(source_key_thumbprint.clone()));
        let context_plan = ManagedKernelContextPlan::source_project_for_tests(
            "context-machine-source",
            "realm-1",
            source_kernel_id,
            &source_key_thumbprint,
            "project-machine-source",
        );
        let plan = context_plan.package_binding();
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config).expect("managed target daemon should bootstrap"),
        ));
        let router = Arc::new(
            CommandRouter::with_interactive_capacity(app, 1).with_managed_kernel_registration(
                ConfirmedManagedKernelRegistration {
                    environment_id: "environment-machine-source".to_string(),
                    machine_id: target_machine_id,
                    kernel_id: target_kernel_id.clone(),
                    context_plan: Some(context_plan),
                },
            ),
        );
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        let (outgoing_tx, _priority_rx, _event_rx) = RelayOutgoingSender::channel(1);
        let peer_harness = ManagedPeerRequestHarness {
            router: &router,
            state: &state,
            outgoing_tx: &outgoing_tx,
        };
        let request = RelayPeerRequest::ArmManagedContextImport {
            context_id: plan.context_id,
            plan_digest: plan.plan_digest,
            target_environment_id: "environment-machine-source".to_string(),
            target_kernel_id,
            target_key_thumbprint,
            capability: RelayManagedContextCapability::new("c".repeat(43)),
            archive_sha256: "b".repeat(64),
            archive_size_bytes: 42,
        };

        let mut wrong_machine = identity.clone();
        wrong_machine.subject = "other-machine".to_string();
        let mut wrong_realm = identity.clone();
        wrong_realm.realm_id = "other-realm".to_string();
        let mut wrong_owner = identity.clone();
        wrong_owner.user_id = Some("other-user".to_string());
        let mut wrong_key = identity.clone();
        wrong_key.public_key_thumbprint = Some(public_key_thumbprint("other-key"));
        for (peer_kernel_id, rejected_identity) in [
            ("other-kernel", identity.clone()),
            (source_kernel_id, wrong_machine),
            (source_kernel_id, wrong_realm),
            (source_kernel_id, wrong_owner),
        ] {
            let response = send_managed_peer_request(
                &peer_harness,
                peer_kernel_id,
                &rejected_identity,
                &source_private_key,
                &target_public_key,
                request.clone(),
            )
            .await;
            assert!(matches!(
                response,
                RelayPeerResponse::ManagedContextImportFailed {
                    ref code,
                    retryable: false,
                } if code == "unauthorized"
            ));
        }
        let mismatched_key_request = relay_crypto::encrypt_payload_for_peer(
            &source_private_key,
            &target_public_key,
            &serde_json::to_vec(&request).expect("serialize mismatched-key request"),
        )
        .expect("encrypt mismatched-key request");
        let mismatched_key_outcome = handle_daemon_peer_request(
            &router,
            &state,
            &outgoing_tx,
            source_kernel_id,
            Some(wrong_key),
            mismatched_key_request,
        )
        .await;
        assert!(matches!(
            mismatched_key_outcome.error,
            Some(ref error) if error.code == "unauthorized" && !error.retryable
        ));
        assert!(mismatched_key_outcome.encrypted_response.is_none());

        let armed = send_managed_peer_request(
            &peer_harness,
            source_kernel_id,
            &identity,
            &source_private_key,
            &target_public_key,
            request,
        )
        .await;
        let (transfer_id, capability) = match armed {
            RelayPeerResponse::ManagedContextImportArmed {
                transfer_id,
                capability,
                ..
            } => (transfer_id, capability),
            response => panic!("machine-bound arm failed: {response:?}"),
        };
        let begun = send_managed_peer_request(
            &peer_harness,
            source_kernel_id,
            &identity,
            &source_private_key,
            &target_public_key,
            RelayPeerRequest::BeginManagedContextImport {
                transfer_id: transfer_id.clone(),
                capability: capability.clone(),
            },
        )
        .await;
        assert!(matches!(
            begun,
            RelayPeerResponse::ManagedContextImportStatus { ref status }
                if status.transfer_id == transfer_id
                    && status.phase == RelayManagedContextTransferPhase::Receiving
                    && status.accepted_bytes == 0
        ));
        let status = send_managed_peer_request(
            &peer_harness,
            source_kernel_id,
            &identity,
            &source_private_key,
            &target_public_key,
            RelayPeerRequest::GetManagedContextImportStatus {
                transfer_id: transfer_id.clone(),
                capability,
            },
        )
        .await;
        assert!(matches!(
            status,
            RelayPeerResponse::ManagedContextImportStatus { ref status }
                if status.transfer_id == transfer_id
                    && status.phase == RelayManagedContextTransferPhase::Receiving
        ));

        drop(router);
        fs::remove_dir_all(root).expect("remove machine-bound transfer fixture");
    }

    #[test]
    fn managed_context_failure_projection_does_not_expose_internal_details() {
        let error = crate::error::DaemonError::ManagedContext {
            code: "invalid_managed_context",
            operation: "import fixture",
            message: "/private/workspace/path and git stderr canary".to_string(),
            retryable: false,
        };
        let response = managed_context_failure_response(&error);
        let serialized = serde_json::to_string(&response).expect("serialize failure response");
        assert!(serialized.contains("invalid_managed_context"));
        assert!(!serialized.contains("/private/workspace/path"));
        assert!(!serialized.contains("git stderr canary"));
    }

    #[test]
    fn encrypted_managed_context_peer_transfer_imports_repository_kernel_context_and_vault() {
        std::thread::Builder::new()
            .name("managed-context-peer-transfer".to_string())
            .stack_size(crate::runtime_transport::KERNEL_RUNTIME_THREAD_STACK_SIZE)
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("managed context test runtime")
                    .block_on(async {
                        for source_subject_kind in
                            [RelaySubjectKind::Kernel, RelaySubjectKind::Machine]
                        {
                            encrypted_managed_context_peer_transfer_imports_repository_kernel_context_and_vault_inner(
                                source_subject_kind,
                            )
                            .await;
                        }
                    });
            })
            .expect("managed context test thread")
            .join()
            .unwrap_or_else(|error| std::panic::resume_unwind(error));
    }

    async fn encrypted_managed_context_peer_transfer_imports_repository_kernel_context_and_vault_inner(
        source_subject_kind: RelaySubjectKind,
    ) {
        let _env_guard = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-managed-peer-import-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let repository = root.join("source-repository");
        fs::create_dir_all(&repository).expect("create source repository");
        git(&repository, &["init", "-b", "main"]);
        git(
            &repository,
            &["config", "user.email", "tests@chariox.local"],
        );
        git(&repository, &["config", "user.name", "Chariox Tests"]);
        fs::write(repository.join("tracked.txt"), "managed context\n").expect("write source file");
        git(&repository, &["add", "tracked.txt"]);
        git(&repository, &["commit", "-m", "initial"]);
        let archive_path = root.join("development.tar.gz");
        let exported = export_development_context(DevelopmentContextExportRequest {
            project_id: "project-managed-peer".to_string(),
            repositories: vec![DevelopmentRepositorySelection {
                workspace_id: "workspace-primary".to_string(),
                worktree_id: None,
                worktree_path: repository.clone(),
                role: DevelopmentRepositoryRole::Primary,
            }],
            archive_path: archive_path.clone(),
        })
        .expect("export managed peer context");

        let source_private_key = relay_crypto::generate_private_key_base64();
        let source_public_key =
            relay_crypto::public_key_from_private_key_base64(&source_private_key)
                .expect("source public key");
        let source_key_thumbprint = public_key_thumbprint(&source_public_key);
        let source_kernel_id = "source-kernel-1";
        let identity = match source_subject_kind {
            RelaySubjectKind::Kernel => {
                scoped_kernel_identity(Some(source_key_thumbprint.clone()), u64::MAX)
            }
            RelaySubjectKind::Machine => {
                scoped_machine_identity("source-machine-test", Some(source_key_thumbprint.clone()))
            }
            _ => unreachable!("managed context test source must be a daemon identity"),
        };
        let context_id = "context-managed-peer".to_string();
        let transfer_state_path = root.join("kernel/managed-context-transfers/state.json");
        let retirement_state_backup = transfer_state_path.with_extension("retirement-backup");
        let (cloud_api_url, completion_fixture) =
            context_completion_fixture(transfer_state_path.clone());

        let mut config = DaemonConfig::for_tests();
        config = config.with_session_history_root(root.join("sessions"));
        config.user_config.history.operational.path =
            Some(root.join("operational.db").display().to_string());
        config.user_config.artifacts.operational.root =
            Some(root.join("artifacts").display().to_string());
        config.user_config.artifacts.operational.index_path =
            Some(root.join("artifacts.db").display().to_string());
        config.user_config.state.path = Some(root.join("kernel/state.db").display().to_string());
        let target_kernel_id = config.daemon_id.clone();
        let target_machine_id = config.host_machine_id.clone();
        config.cloud_relay = Some(test_cloud_profile(cloud_api_url, target_machine_id.clone()));
        let restart_config = config.clone();
        let target_public_key = config.relay_public_key.clone();
        let target_private_key = config.relay_private_key.clone();
        let target_key_thumbprint = public_key_thumbprint(&target_public_key);
        let context_plan = ManagedKernelContextPlan::source_project_for_tests(
            &context_id,
            "realm-1",
            source_kernel_id,
            &source_key_thumbprint,
            "project-managed-peer",
        );
        let plan_binding = context_plan.package_binding();
        let plan_digest = plan_binding.plan_digest.clone();
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config).expect("managed target daemon should bootstrap"),
        ));
        let router = Arc::new(
            CommandRouter::with_interactive_capacity(app.clone(), 1)
                .with_managed_kernel_registration(ConfirmedManagedKernelRegistration {
                    environment_id: "environment-managed-1".to_string(),
                    machine_id: target_machine_id.clone(),
                    kernel_id: target_kernel_id.clone(),
                    context_plan: Some(context_plan),
                }),
        );
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        let (outgoing_tx, _priority_rx, _event_rx) = RelayOutgoingSender::channel(1);
        let peer_harness = ManagedPeerRequestHarness {
            router: &router,
            state: &state,
            outgoing_tx: &outgoing_tx,
        };
        let source_vault_path = root.join("source-vault.json");
        let target_vault_path = root.join("target-vault.json");
        let capability_root = root.join("target-capabilities");
        let _capability_env = ScopedEnv::set(
            "CHARIOX_CAPABILITY_ISOLATION_ROOT",
            capability_root.as_os_str(),
        );
        let _vault_env =
            ScopedEnv::set("CHARIOX_MANAGED_VAULT_PATH", target_vault_path.as_os_str());
        unlock_chariox_encrypted_vault(
            &source_vault_path,
            "managed-peer-passphrase",
            VaultUnlockLease::KernelShutdown,
        )
        .expect("unlock source Vault");
        crate::secret::set_chariox_encrypted_vault_secret_for_test(
            source_vault_path.clone(),
            "managed-peer",
            "token",
            "managed-peer-secret-canary",
        )
        .expect("store source Vault canary");
        let transferred_vault = export_transferred_vault_snapshot(
            &source_vault_path,
            &context_id,
            source_kernel_id,
            &source_private_key,
            &target_kernel_id,
            &target_public_key,
        )
        .expect("export transferred Vault");
        let payload = KernelContextPayload {
            schema_version: crate::managed_context::kernel::KERNEL_CONTEXT_SCHEMA_VERSION,
            context_id: context_id.clone(),
            source_kernel_id: source_kernel_id.to_string(),
            source_key_thumbprint: source_key_thumbprint.clone(),
            target_kernel_id: target_kernel_id.clone(),
            target_key_thumbprint: target_key_thumbprint.clone(),
            compatibility: KernelContextCompatibility {
                source_kernel_version: env!("CARGO_PKG_VERSION").to_string(),
                local_daemon_protocol_version: crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
                relay_peer_protocol_version: RELAY_PEER_PROTOCOL_VERSION,
            },
            extensions: Vec::new(),
            dependencies: Vec::new(),
            vault: transferred_vault,
        };
        let snapshot_sha256 = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&payload).expect("serialize kernel payload"))
        );
        let kernel_context = KernelContextSnapshot {
            payload,
            snapshot_sha256: snapshot_sha256.clone(),
        };
        let package = export_managed_context_package(ManagedContextPackageExportRequest {
            plan: plan_binding,
            target_environment_id: "environment-managed-1".to_string(),
            source_kernel_id: source_kernel_id.to_string(),
            source_key_thumbprint: source_key_thumbprint.clone(),
            target_kernel_id: target_kernel_id.clone(),
            target_key_thumbprint: target_key_thumbprint.clone(),
            development: ManagedContextPackageDevelopment::FromSource {
                archive_path: archive_path.clone(),
                archive_sha256: exported.archive_sha256.clone(),
            },
            kernel_context: ManagedContextPackageKernel::FromKernel(Box::new(kernel_context)),
            provider_accounts: ManagedContextPackageProviderAccounts::None,
            git_credentials:
                crate::managed_context::package::ManagedContextPackageGitCredentials::None,
            package_path: root.join("context.chariox"),
        })
        .expect("compose managed context package");

        let wrong_context_error = router
            .relay_arm_managed_context_import(
                crate::runtime::router::RelayManagedContextArmRequest {
                    identity: identity.clone(),
                    source_kernel_id: source_kernel_id.to_string(),
                    context_id: "wrong-context".to_string(),
                    plan_digest: plan_digest.clone(),
                    target_environment_id: "environment-managed-1".to_string(),
                    target_kernel_id: target_kernel_id.clone(),
                    target_key_thumbprint: target_key_thumbprint.clone(),
                    capability: "w".repeat(43),
                    archive_sha256: package.package_sha256.clone(),
                    archive_size_bytes: package.package_size_bytes,
                },
            )
            .await
            .expect_err("Cloud context ID must bind the import");
        assert!(matches!(
            wrong_context_error,
            DaemonError::ManagedContext {
                code: "unauthorized",
                ..
            }
        ));
        let mut wrong_source_identity = identity.clone();
        wrong_source_identity.subject = "other-source-kernel".to_string();
        let wrong_source_error = router
            .relay_arm_managed_context_import(
                crate::runtime::router::RelayManagedContextArmRequest {
                    identity: wrong_source_identity,
                    source_kernel_id: source_kernel_id.to_string(),
                    context_id: context_id.clone(),
                    plan_digest: plan_digest.clone(),
                    target_environment_id: "environment-managed-1".to_string(),
                    target_kernel_id: target_kernel_id.clone(),
                    target_key_thumbprint: target_key_thumbprint.clone(),
                    capability: "w".repeat(43),
                    archive_sha256: package.package_sha256.clone(),
                    archive_size_bytes: package.package_size_bytes,
                },
            )
            .await
            .expect_err("Cloud source kernel must bind the import");
        assert!(matches!(
            wrong_source_error,
            DaemonError::ManagedContext {
                code: "unauthorized",
                ..
            }
        ));

        let armed = send_managed_peer_request(
            &peer_harness,
            source_kernel_id,
            &identity,
            &source_private_key,
            &target_public_key,
            RelayPeerRequest::ArmManagedContextImport {
                context_id: context_id.clone(),
                plan_digest: plan_digest.clone(),
                target_environment_id: "environment-managed-1".to_string(),
                target_kernel_id: target_kernel_id.clone(),
                target_key_thumbprint: target_key_thumbprint.clone(),
                capability: RelayManagedContextCapability::new("c".repeat(43)),
                archive_sha256: package.package_sha256.clone(),
                archive_size_bytes: package.package_size_bytes,
            },
        )
        .await;
        let (transfer_id, capability, max_chunk_bytes) = match armed {
            RelayPeerResponse::ManagedContextImportArmed {
                transfer_id,
                capability,
                max_chunk_bytes,
                relay_peer_protocol_version,
                ..
            } => {
                assert_eq!(relay_peer_protocol_version, RELAY_PEER_PROTOCOL_VERSION);
                (transfer_id, capability.into_inner(), max_chunk_bytes)
            }
            response => panic!("unexpected arm response: {response:?}"),
        };
        send_managed_peer_request(
            &peer_harness,
            source_kernel_id,
            &identity,
            &source_private_key,
            &target_public_key,
            RelayPeerRequest::BeginManagedContextImport {
                transfer_id: transfer_id.clone(),
                capability: RelayManagedContextCapability::new(capability.clone()),
            },
        )
        .await;
        let archive = fs::read(&package.package_path).expect("read exported package");
        let mut offset = 0_u64;
        for chunk in archive.chunks(max_chunk_bytes) {
            send_managed_peer_request(
                &peer_harness,
                source_kernel_id,
                &identity,
                &source_private_key,
                &target_public_key,
                RelayPeerRequest::UploadManagedContextChunk {
                    transfer_id: transfer_id.clone(),
                    capability: RelayManagedContextCapability::new(capability.clone()),
                    offset,
                    data_base64: RelayManagedContextChunk::new(
                        base64::engine::general_purpose::STANDARD.encode(chunk),
                    ),
                    chunk_sha256: format!("{:x}", Sha256::digest(chunk)),
                },
            )
            .await;
            offset += chunk.len() as u64;
        }
        let unavailable = send_managed_peer_request(
            &peer_harness,
            source_kernel_id,
            &identity,
            &source_private_key,
            &target_public_key,
            RelayPeerRequest::FinalizeManagedContextImport {
                transfer_id: transfer_id.clone(),
                capability: RelayManagedContextCapability::new(capability.clone()),
            },
        )
        .await;
        assert!(matches!(
            unavailable,
            RelayPeerResponse::ManagedContextImportFailed {
                ref code,
                retryable: true,
            } if code == "managed_context_cloud_completion_unavailable"
        ));
        let projects = router.runtime_state().list_projects("user-1", true).await;
        assert!(
            projects
                .iter()
                .any(|project| project.id() == "project-managed-peer"),
            "managed-context import must publish its Project before Cloud can expose readiness"
        );
        let completed = send_managed_peer_request(
            &peer_harness,
            source_kernel_id,
            &identity,
            &source_private_key,
            &target_public_key,
            RelayPeerRequest::FinalizeManagedContextImport {
                transfer_id: transfer_id.clone(),
                capability: RelayManagedContextCapability::new(capability.clone()),
            },
        )
        .await;
        let receipt = match completed {
            RelayPeerResponse::ManagedContextImportStatus { status } => {
                assert_eq!(status.phase, RelayManagedContextTransferPhase::Consumed);
                assert_eq!(status.accepted_bytes, archive.len() as u64);
                status.receipt.expect("completed import receipt")
            }
            response => panic!("unexpected finalize response: {response:?}"),
        };
        assert_eq!(receipt.transfer_id, transfer_id);
        assert_eq!(receipt.plan_digest, plan_digest);
        assert!(matches!(
            receipt.kernel_context,
            crate::transport::relay_peer::RelayManagedKernelContextImportReceipt::FromKernel {
                snapshot_sha256: ref imported_snapshot_sha256,
                extension_count: 0,
                dependency_count: 0,
                ..
            } if imported_snapshot_sha256 == &snapshot_sha256
        ));
        assert!(capability_root.join("kernel-context-import.json").is_file());
        assert_eq!(
            crate::secret::get_chariox_encrypted_vault_secret_for_test(
                target_vault_path.clone(),
                "managed-peer",
                "token",
            )
            .expect("read imported Vault canary"),
            "managed-peer-secret-canary"
        );
        let crate::transport::relay_peer::RelayManagedDevelopmentContextImportReceipt::FromSource {
            project_id,
            repositories,
            ..
        } = receipt.development
        else {
            panic!("selected development context became Empty")
        };
        assert_eq!(project_id, "project-managed-peer");
        assert_eq!(repositories.len(), 1);
        assert_eq!(
            fs::read_to_string(
                std::path::Path::new(&repositories[0].destination_path).join("tracked.txt")
            )
            .expect("read imported repository file"),
            "managed context\n"
        );
        let replayed = send_managed_peer_request(
            &peer_harness,
            source_kernel_id,
            &identity,
            &source_private_key,
            &target_public_key,
            RelayPeerRequest::FinalizeManagedContextImport {
                transfer_id,
                capability: RelayManagedContextCapability::new(capability),
            },
        )
        .await;
        assert!(matches!(
            replayed,
            RelayPeerResponse::ManagedContextImportStatus {
                status: crate::transport::relay_peer::RelayManagedContextTransferStatus {
                    phase: RelayManagedContextTransferPhase::Consumed,
                    ..
                }
            }
        ));

        let terminal_context_id = "context-managed-peer-terminal".to_string();
        let terminal_archive_path = root.join("terminal-development.tar.gz");
        let terminal_exported = export_development_context(DevelopmentContextExportRequest {
            project_id: "project-managed-peer-terminal".to_string(),
            repositories: vec![DevelopmentRepositorySelection {
                workspace_id: "workspace-primary".to_string(),
                worktree_id: None,
                worktree_path: repository,
                role: DevelopmentRepositoryRole::Primary,
            }],
            archive_path: terminal_archive_path.clone(),
        })
        .expect("export terminal managed peer context");
        let terminal_capability_root = root.join("terminal-capabilities");
        let terminal_vault_path = root.join("terminal-vault.json");
        let terminal_vault_envelope_path = terminal_vault_path.with_file_name(format!(
            "terminal-vault.json.managed-context-key-{:x}.json",
            Sha256::digest(target_kernel_id.as_bytes())
        ));
        let _terminal_capability_env = ScopedEnv::set(
            "CHARIOX_CAPABILITY_ISOLATION_ROOT",
            terminal_capability_root.as_os_str(),
        );
        let _terminal_vault_env = ScopedEnv::set(
            "CHARIOX_MANAGED_VAULT_PATH",
            terminal_vault_path.as_os_str(),
        );
        let terminal_plan = ManagedKernelContextPlan::source_project_for_tests(
            &terminal_context_id,
            "realm-1",
            source_kernel_id,
            &source_key_thumbprint,
            "project-managed-peer-terminal",
        );
        let terminal_plan_binding = terminal_plan.package_binding();
        let terminal_plan_digest = terminal_plan_binding.plan_digest.clone();
        let terminal_vault = export_transferred_vault_snapshot(
            &source_vault_path,
            &terminal_context_id,
            source_kernel_id,
            &source_private_key,
            &target_kernel_id,
            &target_public_key,
        )
        .expect("export terminal transferred Vault");
        let terminal_payload = KernelContextPayload {
            schema_version: crate::managed_context::kernel::KERNEL_CONTEXT_SCHEMA_VERSION,
            context_id: terminal_context_id.clone(),
            source_kernel_id: source_kernel_id.to_string(),
            source_key_thumbprint: source_key_thumbprint.clone(),
            target_kernel_id: target_kernel_id.clone(),
            target_key_thumbprint: target_key_thumbprint.clone(),
            compatibility: KernelContextCompatibility {
                source_kernel_version: env!("CARGO_PKG_VERSION").to_string(),
                local_daemon_protocol_version: crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
                relay_peer_protocol_version: RELAY_PEER_PROTOCOL_VERSION,
            },
            extensions: Vec::new(),
            dependencies: Vec::new(),
            vault: terminal_vault,
        };
        let terminal_snapshot_sha256 = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&terminal_payload).expect("serialize terminal kernel payload")
            )
        );
        let terminal_package = export_managed_context_package(ManagedContextPackageExportRequest {
            plan: terminal_plan_binding,
            target_environment_id: "environment-managed-1".to_string(),
            source_kernel_id: source_kernel_id.to_string(),
            source_key_thumbprint: source_key_thumbprint.clone(),
            target_kernel_id: target_kernel_id.clone(),
            target_key_thumbprint: target_key_thumbprint.clone(),
            development: ManagedContextPackageDevelopment::FromSource {
                archive_path: terminal_archive_path,
                archive_sha256: terminal_exported.archive_sha256,
            },
            kernel_context: ManagedContextPackageKernel::FromKernel(Box::new(
                KernelContextSnapshot {
                    payload: terminal_payload,
                    snapshot_sha256: terminal_snapshot_sha256,
                },
            )),
            provider_accounts: ManagedContextPackageProviderAccounts::None,
            git_credentials:
                crate::managed_context::package::ManagedContextPackageGitCredentials::None,
            package_path: root.join("terminal-context.chariox"),
        })
        .expect("compose terminal managed context package");
        let terminal_router = Arc::new(
            CommandRouter::with_interactive_capacity(app.clone(), 1)
                .with_managed_kernel_registration(ConfirmedManagedKernelRegistration {
                    environment_id: "environment-managed-1".to_string(),
                    machine_id: target_machine_id.clone(),
                    kernel_id: target_kernel_id.clone(),
                    context_plan: Some(terminal_plan),
                }),
        );
        let terminal_peer_harness = ManagedPeerRequestHarness {
            router: &terminal_router,
            state: &state,
            outgoing_tx: &outgoing_tx,
        };
        let terminal_armed = send_managed_peer_request(
            &terminal_peer_harness,
            source_kernel_id,
            &identity,
            &source_private_key,
            &target_public_key,
            RelayPeerRequest::ArmManagedContextImport {
                context_id: terminal_context_id.clone(),
                plan_digest: terminal_plan_digest.clone(),
                target_environment_id: "environment-managed-1".to_string(),
                target_kernel_id: target_kernel_id.clone(),
                target_key_thumbprint: target_key_thumbprint.clone(),
                capability: RelayManagedContextCapability::new("t".repeat(43)),
                archive_sha256: terminal_package.package_sha256.clone(),
                archive_size_bytes: terminal_package.package_size_bytes,
            },
        )
        .await;
        let (terminal_transfer_id, terminal_capability, terminal_max_chunk_bytes) =
            match terminal_armed {
                RelayPeerResponse::ManagedContextImportArmed {
                    transfer_id,
                    capability,
                    max_chunk_bytes,
                    ..
                } => (transfer_id, capability.into_inner(), max_chunk_bytes),
                response => panic!("unexpected terminal arm response: {response:?}"),
            };
        send_managed_peer_request(
            &terminal_peer_harness,
            source_kernel_id,
            &identity,
            &source_private_key,
            &target_public_key,
            RelayPeerRequest::BeginManagedContextImport {
                transfer_id: terminal_transfer_id.clone(),
                capability: RelayManagedContextCapability::new(terminal_capability.clone()),
            },
        )
        .await;
        let terminal_archive =
            fs::read(&terminal_package.package_path).expect("read terminal package");
        let mut terminal_offset = 0_u64;
        for chunk in terminal_archive.chunks(terminal_max_chunk_bytes) {
            send_managed_peer_request(
                &terminal_peer_harness,
                source_kernel_id,
                &identity,
                &source_private_key,
                &target_public_key,
                RelayPeerRequest::UploadManagedContextChunk {
                    transfer_id: terminal_transfer_id.clone(),
                    capability: RelayManagedContextCapability::new(terminal_capability.clone()),
                    offset: terminal_offset,
                    data_base64: RelayManagedContextChunk::new(
                        base64::engine::general_purpose::STANDARD.encode(chunk),
                    ),
                    chunk_sha256: format!("{:x}", Sha256::digest(chunk)),
                },
            )
            .await;
            terminal_offset += chunk.len() as u64;
        }
        let retirement_unavailable = send_managed_peer_request(
            &terminal_peer_harness,
            source_kernel_id,
            &identity,
            &source_private_key,
            &target_public_key,
            RelayPeerRequest::FinalizeManagedContextImport {
                transfer_id: terminal_transfer_id.clone(),
                capability: RelayManagedContextCapability::new(terminal_capability.clone()),
            },
        )
        .await;
        assert!(matches!(
            retirement_unavailable,
            RelayPeerResponse::ManagedContextImportFailed {
                ref code,
                retryable: true,
            } if code == "managed_context_import_unavailable"
        ));
        assert!(!terminal_capability_root.exists());
        assert!(!terminal_vault_path.exists());
        assert!(!terminal_vault_envelope_path.exists());
        fs::remove_dir(&transfer_state_path).expect("remove blocked transfer state path");
        fs::rename(&retirement_state_backup, &transfer_state_path)
            .expect("restore transfer state after retirement failure");

        let terminal = send_managed_peer_request(
            &terminal_peer_harness,
            source_kernel_id,
            &identity,
            &source_private_key,
            &target_public_key,
            RelayPeerRequest::FinalizeManagedContextImport {
                transfer_id: terminal_transfer_id.clone(),
                capability: RelayManagedContextCapability::new(terminal_capability.clone()),
            },
        )
        .await;
        assert!(matches!(
            terminal,
            RelayPeerResponse::ManagedContextImportFailed {
                ref code,
                retryable: false,
            } if code == "managed_context_cloud_completion_rejected"
        ));
        assert!(!terminal_capability_root.exists());
        assert!(!terminal_vault_path.exists());
        assert!(!terminal_vault_envelope_path.exists());
        let projects = terminal_router
            .runtime_state()
            .list_projects("user-1", true)
            .await;
        assert!(
            projects
                .iter()
                .all(|project| project.id() != "project-managed-peer-terminal"),
            "terminal Cloud rejection must roll back its provisional managed-context Project"
        );
        assert!(
            projects
                .iter()
                .any(|project| project.id() == "project-managed-peer"),
            "terminal rollback must preserve a Project from an earlier completed import"
        );
        let terminal_replay = send_managed_peer_request(
            &terminal_peer_harness,
            source_kernel_id,
            &identity,
            &source_private_key,
            &target_public_key,
            RelayPeerRequest::GetManagedContextImportStatus {
                transfer_id: terminal_transfer_id.clone(),
                capability: RelayManagedContextCapability::new(terminal_capability.clone()),
            },
        )
        .await;
        assert!(matches!(
            terminal_replay,
            RelayPeerResponse::ManagedContextImportFailed {
                ref code,
                retryable: false,
            } if code == "managed_context_cloud_completion_rejected"
        ));
        drop(terminal_router);
        let reopened = crate::managed_context::transfer::ManagedContextTransferStore::open(
            root.join("kernel/managed-context-transfers"),
        )
        .expect("reopen managed context transfer store");
        let reopened_status = reopened
            .get_status(
                &terminal_transfer_id,
                &terminal_capability,
                &crate::managed_context::transfer::ManagedContextTransferCaller {
                    kernel_id: source_kernel_id.to_string(),
                    key_thumbprint: source_key_thumbprint.clone(),
                    owner_user_id: identity.user_id.clone().expect("source owner"),
                    realm_id: identity.realm_id.clone(),
                    target_environment_id: "environment-managed-1".to_string(),
                    target_kernel_id: target_kernel_id.clone(),
                    target_key_thumbprint: target_key_thumbprint.clone(),
                },
                crate::session::unix_epoch_ms(),
            )
            .expect("read reopened terminal transfer");
        assert_eq!(
            reopened_status.phase,
            crate::managed_context::transfer::ManagedContextTransferPhase::Failed
        );
        assert_eq!(
            reopened_status.failure_code.as_deref(),
            Some("managed_context_cloud_completion_rejected")
        );
        assert!(!terminal_capability_root.exists());
        assert!(!terminal_vault_path.exists());
        assert!(!terminal_vault_envelope_path.exists());

        let completion_requests = completion_fixture.join().expect("Cloud completion fixture");
        assert_eq!(completion_requests.len(), 4);
        assert_eq!(completion_requests[0], completion_requests[1]);
        let completion_request = &completion_requests[1];
        assert_eq!(completion_request["accountId"], "account-1");
        assert_eq!(completion_request["environmentId"], "environment-managed-1");
        assert_eq!(completion_request["machineId"], target_machine_id);
        assert_eq!(completion_request["kernelId"], target_kernel_id);
        assert_eq!(
            completion_request["machineCredential"],
            test_machine_credential()
        );
        assert_eq!(completion_request["contextId"], context_id);
        assert_eq!(completion_request["planDigest"], plan_digest);
        assert_eq!(
            completion_request["contextManifestDigest"],
            format!("sha256:{}", receipt.receipt_sha256)
        );
        assert_eq!(completion_requests[2]["contextId"], terminal_context_id);
        assert_eq!(completion_requests[2]["planDigest"], terminal_plan_digest);
        assert_eq!(completion_requests[2], completion_requests[3]);

        let recovery_context_id = "context-managed-peer-recovery".to_string();
        let recovery_capability_root = root.join("recovery-capabilities");
        let recovery_vault_path = root.join("recovery-vault.json");
        let recovery_vault_envelope_path = recovery_vault_path.with_file_name(format!(
            "recovery-vault.json.managed-context-key-{:x}.json",
            Sha256::digest(target_kernel_id.as_bytes())
        ));
        let _recovery_capability_env = ScopedEnv::set(
            "CHARIOX_CAPABILITY_ISOLATION_ROOT",
            recovery_capability_root.as_os_str(),
        );
        let _recovery_vault_env = ScopedEnv::set(
            "CHARIOX_MANAGED_VAULT_PATH",
            recovery_vault_path.as_os_str(),
        );
        let recovery_plan = ManagedKernelContextPlan::source_project_for_tests(
            &recovery_context_id,
            "realm-1",
            source_kernel_id,
            &source_key_thumbprint,
            "project-managed-peer",
        );
        let recovery_plan_binding = recovery_plan.package_binding();
        let recovery_plan_digest = recovery_plan_binding.plan_digest.clone();
        let recovery_vault = export_transferred_vault_snapshot(
            &source_vault_path,
            &recovery_context_id,
            source_kernel_id,
            &source_private_key,
            &target_kernel_id,
            &target_public_key,
        )
        .expect("export recovery transferred Vault");
        let recovery_payload = KernelContextPayload {
            schema_version: crate::managed_context::kernel::KERNEL_CONTEXT_SCHEMA_VERSION,
            context_id: recovery_context_id.clone(),
            source_kernel_id: source_kernel_id.to_string(),
            source_key_thumbprint: source_key_thumbprint.clone(),
            target_kernel_id: target_kernel_id.clone(),
            target_key_thumbprint: target_key_thumbprint.clone(),
            compatibility: KernelContextCompatibility {
                source_kernel_version: env!("CARGO_PKG_VERSION").to_string(),
                local_daemon_protocol_version: crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
                relay_peer_protocol_version: RELAY_PEER_PROTOCOL_VERSION,
            },
            extensions: Vec::new(),
            dependencies: Vec::new(),
            vault: recovery_vault,
        };
        let recovery_snapshot_sha256 = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&recovery_payload).expect("serialize recovery kernel payload")
            )
        );
        let recovery_package = export_managed_context_package(ManagedContextPackageExportRequest {
            plan: recovery_plan_binding,
            target_environment_id: "environment-managed-1".to_string(),
            source_kernel_id: source_kernel_id.to_string(),
            source_key_thumbprint: source_key_thumbprint.clone(),
            target_kernel_id: target_kernel_id.clone(),
            target_key_thumbprint: target_key_thumbprint.clone(),
            development: ManagedContextPackageDevelopment::FromSource {
                archive_path: root.join("development.tar.gz"),
                archive_sha256: exported.archive_sha256,
            },
            kernel_context: ManagedContextPackageKernel::FromKernel(Box::new(
                KernelContextSnapshot {
                    payload: recovery_payload,
                    snapshot_sha256: recovery_snapshot_sha256,
                },
            )),
            provider_accounts: ManagedContextPackageProviderAccounts::None,
            git_credentials:
                crate::managed_context::package::ManagedContextPackageGitCredentials::None,
            package_path: root.join("recovery-context.chariox"),
        })
        .expect("compose recovery managed context package");
        let recovery_router = Arc::new(
            CommandRouter::with_interactive_capacity(app.clone(), 1)
                .with_managed_kernel_registration(ConfirmedManagedKernelRegistration {
                    environment_id: "environment-managed-1".to_string(),
                    machine_id: target_machine_id.clone(),
                    kernel_id: target_kernel_id.clone(),
                    context_plan: Some(recovery_plan.clone()),
                }),
        );
        let recovery_peer_harness = ManagedPeerRequestHarness {
            router: &recovery_router,
            state: &state,
            outgoing_tx: &outgoing_tx,
        };
        let recovery_armed = send_managed_peer_request(
            &recovery_peer_harness,
            source_kernel_id,
            &identity,
            &source_private_key,
            &target_public_key,
            RelayPeerRequest::ArmManagedContextImport {
                context_id: recovery_context_id,
                plan_digest: recovery_plan_digest,
                target_environment_id: "environment-managed-1".to_string(),
                target_kernel_id: target_kernel_id.clone(),
                target_key_thumbprint: target_key_thumbprint.clone(),
                capability: RelayManagedContextCapability::new("r".repeat(43)),
                archive_sha256: recovery_package.package_sha256.clone(),
                archive_size_bytes: recovery_package.package_size_bytes,
            },
        )
        .await;
        let (recovery_transfer_id, recovery_capability, recovery_max_chunk_bytes) =
            match recovery_armed {
                RelayPeerResponse::ManagedContextImportArmed {
                    transfer_id,
                    capability,
                    max_chunk_bytes,
                    ..
                } => (transfer_id, capability.into_inner(), max_chunk_bytes),
                response => panic!("unexpected recovery arm response: {response:?}"),
            };
        send_managed_peer_request(
            &recovery_peer_harness,
            source_kernel_id,
            &identity,
            &source_private_key,
            &target_public_key,
            RelayPeerRequest::BeginManagedContextImport {
                transfer_id: recovery_transfer_id.clone(),
                capability: RelayManagedContextCapability::new(recovery_capability.clone()),
            },
        )
        .await;
        let recovery_archive =
            fs::read(&recovery_package.package_path).expect("read recovery package");
        let mut recovery_offset = 0_u64;
        for chunk in recovery_archive.chunks(recovery_max_chunk_bytes) {
            send_managed_peer_request(
                &recovery_peer_harness,
                source_kernel_id,
                &identity,
                &source_private_key,
                &target_public_key,
                RelayPeerRequest::UploadManagedContextChunk {
                    transfer_id: recovery_transfer_id.clone(),
                    capability: RelayManagedContextCapability::new(recovery_capability.clone()),
                    offset: recovery_offset,
                    data_base64: RelayManagedContextChunk::new(
                        base64::engine::general_purpose::STANDARD.encode(chunk),
                    ),
                    chunk_sha256: format!("{:x}", Sha256::digest(chunk)),
                },
            )
            .await;
            recovery_offset += chunk.len() as u64;
        }
        let recovery_caller = crate::managed_context::transfer::ManagedContextTransferCaller {
            kernel_id: source_kernel_id.to_string(),
            key_thumbprint: source_key_thumbprint.clone(),
            owner_user_id: identity.user_id.clone().expect("source owner"),
            realm_id: identity.realm_id.clone(),
            target_environment_id: "environment-managed-1".to_string(),
            target_kernel_id: target_kernel_id.clone(),
            target_key_thumbprint: target_key_thumbprint.clone(),
        };
        let recovery_store = {
            let guard = app.clone().lock_owned().await;
            guard.managed_context_transfer_store()
        };
        let recovery_ready = match recovery_store
            .prepare_and_claim_import(
                &recovery_transfer_id,
                &recovery_capability,
                &recovery_caller,
                crate::session::unix_epoch_ms(),
            )
            .expect("claim recovery import before simulated crash")
        {
            crate::managed_context::transfer::ManagedContextImportClaim::Claimed(ready) => ready,
            other => panic!("unexpected recovery claim: {other:?}"),
        };
        apply_managed_context_package(ManagedContextPackageApplicationRequest {
            transfer_id: recovery_ready.transfer_id,
            package_path: recovery_ready.archive_path,
            expected_package_sha256: recovery_ready.archive_sha256,
            expected_binding: ManagedContextPackageBinding {
                plan: recovery_ready.plan,
                target_environment_id: recovery_ready.target_environment_id,
                source_kernel_id: recovery_ready.source_kernel_id,
                source_key_thumbprint: recovery_ready.source_key_thumbprint,
                target_kernel_id: recovery_ready.target_kernel_id,
                target_key_thumbprint: recovery_ready.target_key_thumbprint,
            },
            development_destination_root: recovery_ready.destination_root,
            target_private_key,
            provider_account_target: None,
            git_credential_target: None,
        })
        .expect("publish recovery context before simulated crash");
        recovery_store
            .release_import(&recovery_transfer_id)
            .expect("release recovery owner for simulated crash");
        assert!(recovery_capability_root.exists());
        assert!(recovery_vault_path.exists());
        assert!(recovery_vault_envelope_path.exists());
        drop(recovery_store);
        drop(recovery_router);
        drop(router);
        drop(reopened);
        drop(app);

        let mismatched_plan = ManagedKernelContextPlan::source_project_for_tests(
            "context-managed-peer-rebound",
            "realm-1",
            source_kernel_id,
            &source_key_thumbprint,
            "project-managed-peer",
        );
        let restarted_app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(restart_config)
                .expect("managed target daemon should reopen interrupted import"),
        ));
        let restarted_router = Arc::new(
            CommandRouter::with_interactive_capacity(restarted_app.clone(), 1)
                .with_managed_kernel_registration(ConfirmedManagedKernelRegistration {
                    environment_id: "environment-managed-1".to_string(),
                    machine_id: target_machine_id.clone(),
                    kernel_id: target_kernel_id.clone(),
                    context_plan: Some(mismatched_plan),
                }),
        );
        let restarted_peer_harness = ManagedPeerRequestHarness {
            router: &restarted_router,
            state: &state,
            outgoing_tx: &outgoing_tx,
        };
        let rebound = send_managed_peer_request(
            &restarted_peer_harness,
            source_kernel_id,
            &identity,
            &source_private_key,
            &target_public_key,
            RelayPeerRequest::FinalizeManagedContextImport {
                transfer_id: recovery_transfer_id.clone(),
                capability: RelayManagedContextCapability::new(recovery_capability.clone()),
            },
        )
        .await;
        assert!(matches!(
            rebound,
            RelayPeerResponse::ManagedContextImportFailed {
                ref code,
                retryable: false,
            } if code == "unauthorized"
        ));
        assert!(!recovery_capability_root.exists());
        assert!(!recovery_vault_path.exists());
        assert!(!recovery_vault_envelope_path.exists());
        drop(restarted_router);
        drop(restarted_app);
        let recovery_reopened =
            crate::managed_context::transfer::ManagedContextTransferStore::open(
                root.join("kernel/managed-context-transfers"),
            )
            .expect("reopen recovered transfer store");
        let recovery_status = recovery_reopened
            .get_status(
                &recovery_transfer_id,
                &recovery_capability,
                &recovery_caller,
                crate::session::unix_epoch_ms(),
            )
            .expect("read rebound transfer failure after restart");
        assert_eq!(
            recovery_status.phase,
            crate::managed_context::transfer::ManagedContextTransferPhase::Failed
        );
        assert!(!recovery_capability_root.exists());
        assert!(!recovery_vault_path.exists());
        assert!(!recovery_vault_envelope_path.exists());
        lock_chariox_encrypted_vault(&source_vault_path).expect("lock source Vault");
        lock_chariox_encrypted_vault(&target_vault_path).expect("lock target Vault");
        fs::remove_dir_all(root).expect("remove managed peer fixture");
    }

    struct ScopedEnv {
        name: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl ScopedEnv {
        fn set(name: &'static str, value: &std::ffi::OsStr) -> Self {
            let previous = std::env::var_os(name);
            std::env::set_var(name, value);
            Self { name, previous }
        }
    }

    impl Drop for ScopedEnv {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => std::env::set_var(self.name, value),
                None => std::env::remove_var(self.name),
            }
        }
    }

    struct ManagedPeerRequestHarness<'a> {
        router: &'a Arc<CommandRouter>,
        state: &'a Arc<RwLock<RelayClientState>>,
        outgoing_tx: &'a RelayOutgoingSender,
    }

    async fn send_managed_peer_request(
        harness: &ManagedPeerRequestHarness<'_>,
        source_kernel_id: &str,
        identity: &RelayCallerIdentity,
        source_private_key: &str,
        target_public_key: &str,
        request: RelayPeerRequest,
    ) -> RelayPeerResponse {
        let encrypted_request = relay_crypto::encrypt_payload_for_peer(
            source_private_key,
            target_public_key,
            &serde_json::to_vec(&request).expect("serialize peer request"),
        )
        .expect("encrypt peer request");
        let outcome = handle_daemon_peer_request(
            harness.router,
            harness.state,
            harness.outgoing_tx,
            source_kernel_id,
            Some(identity.clone()),
            encrypted_request,
        )
        .await;
        if let Some(error) = outcome.error {
            panic!("managed peer request failed: {error:?}");
        }
        let encrypted_response = outcome
            .encrypted_response
            .expect("managed peer encrypted response");
        let decrypted =
            relay_crypto::decrypt_payload_for_private_key(source_private_key, &encrypted_response)
                .expect("decrypt peer response");
        serde_json::from_slice(&decrypted.plaintext).expect("decode peer response")
    }

    fn context_completion_fixture(
        transfer_state_path: std::path::PathBuf,
    ) -> (String, std::thread::JoinHandle<Vec<serde_json::Value>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind Cloud completion fixture");
        let address = listener.local_addr().expect("Cloud completion address");
        let fixture = std::thread::spawn(move || {
            let mut bodies = Vec::new();
            for attempt in 0..4 {
                let (mut stream, _) = listener.accept().expect("accept Cloud completion");
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .expect("Cloud completion read timeout");
                let mut request = Vec::new();
                let mut chunk = [0_u8; 4096];
                let (header_end, content_length) = loop {
                    let read = stream.read(&mut chunk).expect("read Cloud completion");
                    assert!(read > 0, "Cloud completion request ended before its body");
                    request.extend_from_slice(&chunk[..read]);
                    let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n")
                    else {
                        continue;
                    };
                    let headers = String::from_utf8_lossy(&request[..header_end]);
                    assert!(headers.starts_with("POST /v1/managed-kernels/context/complete "));
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|value| value.trim().parse::<usize>().ok())
                        })
                        .expect("Cloud completion content length");
                    if request.len() < header_end + 4 + content_length {
                        continue;
                    }
                    break (header_end, content_length);
                };
                let body = serde_json::from_slice::<serde_json::Value>(
                    &request[header_end + 4..header_end + 4 + content_length],
                )
                .expect("decode Cloud completion request");
                if attempt == 2 {
                    let backup = transfer_state_path.with_extension("retirement-backup");
                    fs::rename(&transfer_state_path, &backup)
                        .expect("move transfer state before retirement failure");
                    fs::create_dir(&transfer_state_path)
                        .expect("block transfer state replacement during retirement");
                }
                let (status, response) = match attempt {
                    0 => (
                        "200 OK",
                        b"{\"ready\":true,\"observedState\":\"ready\"".to_vec(),
                    ),
                    1 => (
                        "200 OK",
                        serde_json::to_vec(&serde_json::json!({
                            "ready": true,
                            "observedState": "ready",
                            "contextManifestDigest": body["contextManifestDigest"],
                            "forwardCompatibleField": true,
                        }))
                        .expect("encode successful Cloud completion response"),
                    ),
                    _ => (
                        "409 Conflict",
                        serde_json::to_vec(&serde_json::json!({
                            "error": {
                                "message": "context completion conflicts with Cloud state",
                                "code": "identity_conflict",
                            },
                        }))
                        .expect("encode rejected Cloud completion response"),
                    ),
                };
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    response.len()
                )
                .expect("write Cloud completion headers");
                stream
                    .write_all(&response)
                    .expect("write Cloud completion response");
                bodies.push(body);
            }
            bodies
        });
        (format!("http://{address}"), fixture)
    }

    fn test_cloud_profile(api_url: String, machine_id: String) -> PersistedCloudRelayProfile {
        PersistedCloudRelayProfile {
            api_url,
            email: "user@example.test".to_string(),
            account_id: "account-1".to_string(),
            user_id: "user-1".to_string(),
            account_slug: "account".to_string(),
            realm_id: "realm-1".to_string(),
            relay_url: "wss://relay.example.test".to_string(),
            issuer_id: "issuer-1".to_string(),
            client_id: None,
            client_alias: None,
            machine_id: Some(machine_id),
            machine_alias: Some("Managed test".to_string()),
            machine_credential: Some(test_machine_credential()),
            cloud_session_token: None,
            cloud_session_expires_at_ms: None,
            token_expires_at_ms: None,
        }
    }

    fn test_machine_credential() -> String {
        format!("mcred_{}", "m".repeat(43))
    }

    fn git(path: &std::path::Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(path)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
