use chariox_relay::protocol::ClientTarget;

use crate::error::DaemonError;
use crate::runtime::state::KernelRuntimeState;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

mod credential_support;
use credential_support::*;

impl KernelRuntimeState {
    pub(super) async fn dispatch_credential_runtime_tool_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let user_config = self.owned.config_projection.snapshot().user_config;
        let credentials = crate::credential::load_user_credentials()?;
        let service = crate::secret::RuntimeSecretService::with_vault_config(
            credentials,
            &user_config.credential_vault,
        )?;
        match tool_name {
            crate::transport::runtime_tools::LIST_CREDENTIAL_HANDLES_TOOL => {
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "credentials": service.list_handles()
                    }),
                })
            }
            crate::transport::runtime_tools::CREATE_GENERATED_CREDENTIAL_TOOL => {
                self.ensure_agent_can_manage_user_vault(provider_run)?;
                let _vault_unlock = self
                    .ensure_vault_unlocked_for_provider_run(
                        provider_run,
                        "runtime_tool_create_generated_credential",
                    )
                    .await?;
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::CreateGeneratedCredentialArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_create_generated_credential",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let generator = args.generator.unwrap_or(
                    crate::transport::runtime_tools::GeneratedCredentialSecretGeneratorArgs {
                        kind: "password".to_string(),
                        length: 32,
                        symbols: true,
                        avoid_ambiguous: false,
                    },
                );
                let secret = zeroize::Zeroizing::new(generate_credential_secret(&generator)?);
                let credential = stamp_runtime_credential_metadata(
                    credential_from_runtime_input(args.credential)?,
                    provider_run,
                );
                let registry = crate::credential::CharioxCredentialRegistry::user()?;
                let result = service.upsert_vault_backed_credential_with_secret(
                    &registry,
                    credential,
                    secret.as_str(),
                    args.overwrite,
                )?;
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "credential_id": result.credential_id,
                        "stored": result.stored,
                        "generated": true,
                    }),
                })
            }
            crate::transport::runtime_tools::REQUEST_CREDENTIAL_SECRET_TOOL => {
                self.ensure_agent_can_manage_user_vault(provider_run)?;
                let _vault_unlock = self
                    .ensure_vault_unlocked_for_provider_run(
                        provider_run,
                        "runtime_tool_request_credential_secret",
                    )
                    .await?;
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::RequestCredentialSecretArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_request_credential_secret",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let credential = stamp_runtime_credential_metadata(
                    credential_from_runtime_input(args.credential)?,
                    provider_run,
                );
                match &credential.source {
                    crate::config::UserCredentialSourceConfig::Vault { .. } => {}
                    crate::config::UserCredentialSourceConfig::Env { .. }
                    | crate::config::UserCredentialSourceConfig::File { .. } => {
                        return Err(DaemonError::LocalTransport {
                            operation: "runtime_tool_request_credential_secret",
                            message: "runtime-created credentials must use a vault source"
                                .to_string(),
                        });
                    }
                }
                if let Some(max_length) = args.prompt.max_length {
                    if max_length < args.prompt.min_length.unwrap_or(1) {
                        return Err(DaemonError::LocalTransport {
                            operation: "runtime_tool_request_credential_secret",
                            message:
                                "prompt max_length must be greater than or equal to min_length"
                                    .to_string(),
                        });
                    }
                }
                let interaction = crate::session::RuntimeInteraction::new(
                    format!(
                        "credential-secret-{}-{}",
                        provider_run.agent_instance_id().unwrap_or("agent"),
                        crate::session::unix_epoch_ms()
                    ),
                    provider_run.agent_instance_id().ok_or_else(|| {
                        DaemonError::LocalTransport {
                            operation: "runtime_tool_request_credential_secret",
                            message: "provider run is not bound to an agent".to_string(),
                        }
                    })?,
                    crate::session::RuntimeInteractionKind::Choice,
                    crate::session::RuntimeInteractionLevel::Critical,
                    args.prompt.title.clone(),
                    args.prompt.message.clone(),
                    vec![crate::session::RuntimeInteractionChoice::new(
                        "cancel",
                        "Cancel",
                        "cancel",
                        Some(crate::session::RuntimeInteractionChoiceStyle::Danger),
                    )],
                    Some(crate::session::RuntimeInteractionCustomChoice::secret(
                        "secret",
                        "Secret",
                        args.prompt.placeholder.clone(),
                        args.prompt.min_length,
                        args.prompt.max_length,
                    )),
                    args.prompt.timeout_sec,
                    Some("cancel".to_string()),
                );
                let interaction_id = interaction.id().to_string();
                let session_id = provider_run.session_id().to_string();
                let timeout_sec = interaction.timeout_sec();
                let remote_target = self
                    .with_app_side_effect(|app| {
                        let mut runtime = crate::app::RemoteLeaseRuntime::new(app);
                        runtime.native_interaction_context_for_backing_agent(
                            provider_run.session_id(),
                            provider_run.agent_instance_id().unwrap_or(""),
                            provider_run.id(),
                        )
                    })
                    .await;
                let resolution = if let Some((target_daemon_id, context)) = remote_target {
                    let response = self
                        .with_app_side_effect(|app| {
                            app.block_on_relay_future(
                                crate::transport::relay_client::send_peer_request_via_temporary_connection(
                                    app.config(),
                                    ClientTarget {
                                        daemon_id: Some(target_daemon_id.clone()),
                                        daemon_alias: None,
                                    },
                                    RelayPeerRequest::ForwardNativeInteraction {
                                        context: context.clone(),
                                        interaction: interaction.clone(),
                                    },
                                ),
                            )
                        })
                        .await?;
                    match response {
                        RelayPeerResponse::NativeInteractionResolved { resolution } => resolution,
                        other => {
                            return Err(DaemonError::LocalTransport {
                                operation: "runtime_tool_request_credential_secret",
                                message: format!(
                                    "unexpected relay response for remote credential secret interaction: {other:?}"
                                ),
                            });
                        }
                    }
                } else {
                    let resolution_rx = self
                        .create_runtime_interaction(&session_id, interaction)
                        .await?;
                    if let Some(timeout_sec) = timeout_sec {
                        let state = self.clone();
                        let timeout_session_id = session_id.clone();
                        let timeout_interaction_id = interaction_id.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_secs(timeout_sec)).await;
                            let _ = state
                                .timeout_runtime_interaction(
                                    &timeout_session_id,
                                    &timeout_interaction_id,
                                )
                                .await;
                        });
                    }
                    let resolution =
                        resolution_rx
                            .await
                            .map_err(|error| DaemonError::LocalTransport {
                                operation: "runtime_tool_request_credential_secret",
                                message: format!(
                                "credential secret interaction dropped before resolution: {error}"
                            ),
                            })?;
                    crate::provider::ProviderNativeInteractionResolution {
                        status: resolution.status.to_string(),
                        choice_id: resolution.choice_id,
                        reply: resolution.reply,
                    }
                };
                if resolution.status == "timed_out" {
                    return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                        ok: true,
                        payload: serde_json::json!({
                            "credential_id": credential.id,
                            "status": "timed_out",
                        }),
                    });
                }
                if resolution.choice_id.as_deref() != Some("secret") {
                    return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                        ok: true,
                        payload: serde_json::json!({
                            "credential_id": credential.id,
                            "status": "cancelled",
                        }),
                    });
                }
                let secret = zeroize::Zeroizing::new(resolution.reply.ok_or_else(|| {
                    DaemonError::LocalTransport {
                        operation: "runtime_tool_request_credential_secret",
                        message: "credential secret interaction resolved without a secret"
                            .to_string(),
                    }
                })?);
                let registry = crate::credential::CharioxCredentialRegistry::user()?;
                let result = service.upsert_vault_backed_credential_with_secret(
                    &registry,
                    credential,
                    secret.as_str(),
                    args.overwrite,
                )?;
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "credential_id": result.credential_id,
                        "status": "stored",
                    }),
                })
            }
            crate::transport::runtime_tools::HTTP_REQUEST_WITH_CREDENTIAL_TOOL => {
                let _vault_unlock = self
                    .ensure_vault_unlocked_for_provider_run(
                        provider_run,
                        "runtime_tool_http_request_with_credential",
                    )
                    .await?;
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::HttpRequestWithCredentialArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_http_request_with_credential",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let request = crate::secret::CredentialHttpRequest {
                    credential_id: args.credential_id,
                    method: args.method,
                    url: args.url,
                    headers: args.headers,
                    body_text: args.body_text,
                    body_json: args.body_json,
                    timeout_ms: args.timeout_ms,
                    max_response_bytes: args.max_response_bytes,
                };
                let response = tokio::task::spawn_blocking(move || {
                    service.http_request_with_credential(request)
                })
                .await
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_http_request_with_credential",
                    message: error.to_string(),
                })??;
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::to_value(response).map_err(|error| {
                        DaemonError::LocalTransport {
                            operation: "runtime_tool_http_request_with_credential",
                            message: error.to_string(),
                        }
                    })?,
                })
            }
            crate::transport::runtime_tools::SEND_SECRET_TO_TERMINAL_TOOL => {
                let _vault_unlock = self
                    .ensure_vault_unlocked_for_provider_run(
                        provider_run,
                        "runtime_tool_send_secret_to_terminal",
                    )
                    .await?;
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::SendSecretToTerminalArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_send_secret_to_terminal",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let mut input = service.terminal_secret_input(&args.credential_id)?;
                if args.append_newline {
                    input.push('\n');
                }
                let provider_run_id = provider_run.id().to_string();
                self.with_app_side_effect(move |app| {
                    app.write_provider_pty_input_for_runtime(&provider_run_id, input.as_bytes())
                })
                .await?;
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "submitted": true,
                        "credential_id": args.credential_id,
                        "target": "current_provider_run",
                    }),
                })
            }
            crate::transport::runtime_tools::MANAGE_CREDENTIAL_VAULT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManageCredentialVaultArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_manage_credential_vault",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let action = args.action.as_deref().unwrap_or("popup");
                if matches!(action, "lock" | "popup") {
                    self.ensure_agent_can_manage_user_vault(provider_run)?;
                }
                let user_config = self.owned.config_projection.snapshot().user_config;
                let vault_path = user_config.credential_vault.path.clone();
                match action {
                    "status" => {
                        let status = crate::secret::chariox_encrypted_vault_status(&vault_path)?;
                        Ok(crate::transport::runtime_tools::RuntimeToolResult {
                            ok: true,
                            payload: serde_json::json!({
                                "action": "status",
                                "status": status,
                            }),
                        })
                    }
                    "lock" => {
                        crate::secret::lock_chariox_encrypted_vault(&vault_path)?;
                        crate::secret::clear_vault_secret_process_cache()?;
                        let status = crate::secret::chariox_encrypted_vault_status(&vault_path)?;
                        Ok(crate::transport::runtime_tools::RuntimeToolResult {
                            ok: true,
                            payload: serde_json::json!({
                                "action": "locked",
                                "status": status,
                            }),
                        })
                    }
                    "popup" => {
                        let agent_id = provider_run.agent_instance_id().ok_or_else(|| {
                            DaemonError::LocalTransport {
                                operation: "runtime_tool_manage_credential_vault",
                                message: "provider run is not bound to an agent".to_string(),
                            }
                        })?;
                        let (status, action) = self
                            .manage_credential_vault_unlock(provider_run.session_id(), agent_id)
                            .await?;
                        Ok(crate::transport::runtime_tools::RuntimeToolResult {
                            ok: true,
                            payload: serde_json::json!({
                                "action": action,
                                "status": status,
                            }),
                        })
                    }
                    other => Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_manage_credential_vault",
                        message: format!(
                            "unsupported vault action `{other}`; expected status, lock, or popup"
                        ),
                    }),
                }
            }
            crate::transport::runtime_tools::REQUEST_POPUP_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::RequestPopupArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_request_popup",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                if args.choices.len() < 2 {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_request_popup",
                        message: "popup interactions require at least two choices".to_string(),
                    });
                }
                let choices = args
                    .choices
                    .into_iter()
                    .map(|choice| {
                        crate::session::RuntimeInteractionChoice::new(
                            choice.id,
                            choice.label,
                            choice.reply,
                            choice.style,
                        )
                    })
                    .collect::<Vec<_>>();
                let custom_choice = args.custom_choice.map(|choice| {
                    crate::session::RuntimeInteractionCustomChoice::new(
                        choice.id,
                        choice.label,
                        choice.placeholder,
                        choice.min_length,
                        choice.max_length,
                    )
                });
                if let Some(custom_choice) = custom_choice.as_ref() {
                    if choices
                        .iter()
                        .any(|choice| choice.id() == custom_choice.id())
                    {
                        return Err(DaemonError::LocalTransport {
                            operation: "runtime_tool_request_popup",
                            message: format!(
                                "custom_choice id `{}` duplicates a fixed choice",
                                custom_choice.id()
                            ),
                        });
                    }
                    if let Some(max_length) = custom_choice.max_length() {
                        if max_length < custom_choice.min_length() {
                            return Err(DaemonError::LocalTransport {
                                operation: "runtime_tool_request_popup",
                                message: "custom_choice max_length must be greater than or equal to min_length".to_string(),
                            });
                        }
                    }
                }
                let default_choice_id = args.default_on_timeout.clone();
                if let Some(default_choice_id) = default_choice_id.as_deref() {
                    if !choices
                        .iter()
                        .any(|choice| choice.id() == default_choice_id)
                    {
                        return Err(DaemonError::LocalTransport {
                            operation: "runtime_tool_request_popup",
                            message: format!(
                                "default_on_timeout choice `{default_choice_id}` is not defined"
                            ),
                        });
                    }
                }
                let interaction = crate::session::RuntimeInteraction::new(
                    format!(
                        "interaction-{}-{}",
                        provider_run.agent_instance_id().unwrap_or("agent"),
                        crate::session::unix_epoch_ms()
                    ),
                    provider_run.agent_instance_id().ok_or_else(|| {
                        DaemonError::LocalTransport {
                            operation: "runtime_tool_request_popup",
                            message: "provider run is not bound to an agent".to_string(),
                        }
                    })?,
                    crate::session::RuntimeInteractionKind::Choice,
                    args.level
                        .unwrap_or(crate::session::RuntimeInteractionLevel::Info),
                    args.title,
                    args.message,
                    choices,
                    custom_choice,
                    args.timeout_sec,
                    default_choice_id.clone(),
                );
                let interaction_id = interaction.id().to_string();
                let session_id = provider_run.session_id().to_string();
                let timeout_sec = interaction.timeout_sec();
                let remote_target = self
                    .with_app_side_effect(|app| {
                        let mut runtime = crate::app::RemoteLeaseRuntime::new(app);
                        runtime.native_interaction_context_for_backing_agent(
                            provider_run.session_id(),
                            provider_run.agent_instance_id().unwrap_or(""),
                            provider_run.id(),
                        )
                    })
                    .await;
                if let Some((target_daemon_id, context)) = remote_target {
                    let response = self
                        .with_app_side_effect(|app| {
                            app.block_on_relay_future(
                                crate::transport::relay_client::send_peer_request_via_temporary_connection(
                                    app.config(),
                                    ClientTarget {
                                        daemon_id: Some(target_daemon_id.clone()),
                                        daemon_alias: None,
                                    },
                                    RelayPeerRequest::ForwardNativeInteraction {
                                        context: context.clone(),
                                        interaction: interaction.clone(),
                                    },
                                ),
                            )
                        })
                        .await?;
                    let resolution = match response {
                        RelayPeerResponse::NativeInteractionResolved { resolution } => resolution,
                        other => {
                            return Err(DaemonError::LocalTransport {
                                operation: "runtime_tool_request_popup",
                                message: format!(
                                    "unexpected relay response for remote popup interaction: {other:?}"
                                ),
                            });
                        }
                    };
                    return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                        ok: true,
                        payload: serde_json::json!({
                            "interaction_id": interaction_id,
                            "status": resolution.status,
                            "choice_id": resolution.choice_id,
                            "reply": resolution.reply,
                        }),
                    });
                }
                let resolution_rx = self
                    .create_runtime_interaction(&session_id, interaction)
                    .await?;
                if let Some(timeout_sec) = timeout_sec {
                    let state = self.clone();
                    let timeout_session_id = session_id.clone();
                    let timeout_interaction_id = interaction_id.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_secs(timeout_sec)).await;
                        let _ = state
                            .timeout_runtime_interaction(
                                &timeout_session_id,
                                &timeout_interaction_id,
                            )
                            .await;
                    });
                }
                let resolution =
                    resolution_rx
                        .await
                        .map_err(|error| DaemonError::LocalTransport {
                            operation: "runtime_tool_request_popup",
                            message: format!(
                                "popup interaction dropped before resolution: {error}"
                            ),
                        })?;
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "interaction_id": interaction_id,
                        "status": resolution.status,
                        "choice_id": resolution.choice_id,
                        "reply": resolution.reply,
                    }),
                })
            }
            _ => Err(DaemonError::LocalTransport {
                operation: "dispatch_credential_runtime_tool_call",
                message: format!("unknown credential runtime tool `{tool_name}`"),
            }),
        }
    }

    pub(crate) async fn dispatch_forwarded_home_credential_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteExtensionInvocationContext,
        tool_name: String,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let agent = self.authorize_home_credential_context(&context)?;
        match tool_name.as_str() {
            crate::transport::runtime_tools::LIST_CREDENTIAL_HANDLES_TOOL => {
                let service = self.home_runtime_secret_service()?;
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "credentials": service.list_handles()
                    }),
                })
            }
            crate::transport::runtime_tools::CREATE_GENERATED_CREDENTIAL_TOOL => {
                self.ensure_agent_can_manage_user_vault_for_agent(
                    &context.home_session_id,
                    &agent,
                )?;
                let _vault_unlock = self
                    .ensure_vault_unlocked_for_agent(
                        &context.home_session_id,
                        agent.id(),
                        "runtime_tool_create_generated_credential",
                    )
                    .await?;
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::CreateGeneratedCredentialArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_create_generated_credential",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let generator = args.generator.unwrap_or(
                    crate::transport::runtime_tools::GeneratedCredentialSecretGeneratorArgs {
                        kind: "password".to_string(),
                        length: 32,
                        symbols: true,
                        avoid_ambiguous: false,
                    },
                );
                let secret = zeroize::Zeroizing::new(generate_credential_secret(&generator)?);
                let service = self.home_runtime_secret_service()?;
                let credential = stamp_runtime_credential_metadata_for_agent(
                    credential_from_runtime_input(args.credential)?,
                    Some(agent.id()),
                    &context.home_session_id,
                    agent.primary_provider(),
                    Some(&context.worker_provider_run_id),
                );
                let registry = crate::credential::CharioxCredentialRegistry::user()?;
                let result = service.upsert_vault_backed_credential_with_secret(
                    &registry,
                    credential,
                    secret.as_str(),
                    args.overwrite,
                )?;
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "credential_id": result.credential_id,
                        "stored": result.stored,
                        "generated": true,
                    }),
                })
            }
            crate::transport::runtime_tools::REQUEST_CREDENTIAL_SECRET_TOOL => {
                let service = self.home_runtime_secret_service()?;
                self.dispatch_forwarded_home_request_credential_secret(
                    &context, &agent, service, arguments,
                )
                .await
            }
            crate::transport::runtime_tools::HTTP_REQUEST_WITH_CREDENTIAL_TOOL => {
                let _vault_unlock = self
                    .ensure_vault_unlocked_for_agent(
                        &context.home_session_id,
                        agent.id(),
                        "runtime_tool_http_request_with_credential",
                    )
                    .await?;
                let service = self.home_runtime_secret_service()?;
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::HttpRequestWithCredentialArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_http_request_with_credential",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let request = crate::secret::CredentialHttpRequest {
                    credential_id: args.credential_id,
                    method: args.method,
                    url: args.url,
                    headers: args.headers,
                    body_text: args.body_text,
                    body_json: args.body_json,
                    timeout_ms: args.timeout_ms,
                    max_response_bytes: args.max_response_bytes,
                };
                let response = tokio::task::spawn_blocking(move || {
                    service.http_request_with_credential(request)
                })
                .await
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_http_request_with_credential",
                    message: error.to_string(),
                })??;
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::to_value(response).map_err(|error| {
                        DaemonError::LocalTransport {
                            operation: "runtime_tool_http_request_with_credential",
                            message: error.to_string(),
                        }
                    })?,
                })
            }
            crate::transport::runtime_tools::SEND_SECRET_TO_TERMINAL_TOOL => {
                Err(DaemonError::LocalTransport {
                    operation: "home credential proxy",
                    message:
                        "send_secret_to_terminal must resolve the secret on home and inject on the worker"
                            .to_string(),
                })
            }
            crate::transport::runtime_tools::PASTE_SECRET_TO_COMPUTER_TOOL => {
                self.dispatch_forwarded_home_computer_secret_input_tool(
                    &context, &agent, arguments,
                )
                .await
            }
            crate::transport::runtime_tools::MANAGE_CREDENTIAL_VAULT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManageCredentialVaultArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_manage_credential_vault",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let action = args.action.as_deref().unwrap_or("popup");
                if matches!(action, "lock" | "popup") {
                    self.ensure_agent_can_manage_user_vault_for_agent(
                        &context.home_session_id,
                        &agent,
                    )?;
                }
                let user_config = self.owned.config_projection.snapshot().user_config;
                let vault_path = user_config.credential_vault.path.clone();
                match action {
                    "status" => {
                        let status = crate::secret::chariox_encrypted_vault_status(&vault_path)?;
                        Ok(crate::transport::runtime_tools::RuntimeToolResult {
                            ok: true,
                            payload: serde_json::json!({
                                "action": "status",
                                "status": status,
                            }),
                        })
                    }
                    "lock" => {
                        crate::secret::lock_chariox_encrypted_vault(&vault_path)?;
                        crate::secret::clear_vault_secret_process_cache()?;
                        let status = crate::secret::chariox_encrypted_vault_status(&vault_path)?;
                        Ok(crate::transport::runtime_tools::RuntimeToolResult {
                            ok: true,
                            payload: serde_json::json!({
                                "action": "locked",
                                "status": status,
                            }),
                        })
                    }
                    "popup" => {
                        let (status, action) = self
                            .manage_credential_vault_unlock(&context.home_session_id, agent.id())
                            .await?;
                        Ok(crate::transport::runtime_tools::RuntimeToolResult {
                            ok: true,
                            payload: serde_json::json!({
                                "action": action,
                                "status": status,
                            }),
                        })
                    }
                    other => Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_manage_credential_vault",
                        message: format!(
                            "unsupported vault action `{other}`; expected status, lock, or popup"
                        ),
                    }),
                }
            }
            crate::transport::runtime_tools::REQUEST_POPUP_TOOL => Err(DaemonError::LocalTransport {
                operation: "home credential proxy",
                message: "request_popup is not a credential vault operation".to_string(),
            }),
            _ => Err(DaemonError::LocalTransport {
                operation: "home credential proxy",
                message: format!("unknown credential runtime tool `{tool_name}`"),
            }),
        }
    }

    pub(crate) async fn resolve_forwarded_home_credential_secret(
        &self,
        context: crate::transport::relay_peer::RemoteExtensionInvocationContext,
        credential_id: String,
        injection: crate::transport::relay_peer::RemoteCredentialSecretInjection,
    ) -> Result<(String, String), DaemonError> {
        let agent = self.authorize_home_credential_context(&context)?;
        let service = self.home_runtime_secret_service()?;
        match &injection {
            crate::transport::relay_peer::RemoteCredentialSecretInjection::Browser {
                target_url,
            } => {
                service.validate_browser_secret_input_for_target_url(&credential_id, target_url)?
            }
            crate::transport::relay_peer::RemoteCredentialSecretInjection::Pty => {
                service.validate_terminal_secret_input(&credential_id)?
            }
            crate::transport::relay_peer::RemoteCredentialSecretInjection::Computer => {
                service.validate_computer_secret_input(&credential_id)?
            }
        };
        if matches!(
            &injection,
            crate::transport::relay_peer::RemoteCredentialSecretInjection::Computer
        ) {
            self.ensure_computer_secret_input_approved(
                &context.home_session_id,
                agent.id(),
                &credential_id,
            )
            .await?;
        }
        let _vault_unlock = self
            .ensure_vault_unlocked_for_agent(
                &context.home_session_id,
                agent.id(),
                "home_credential_secret_resolve",
            )
            .await?;
        let secret_input = match injection {
            crate::transport::relay_peer::RemoteCredentialSecretInjection::Browser {
                target_url,
            } => service.browser_secret_input_for_target_url(&credential_id, &target_url)?,
            crate::transport::relay_peer::RemoteCredentialSecretInjection::Pty => {
                service.terminal_secret_input(&credential_id)?
            }
            crate::transport::relay_peer::RemoteCredentialSecretInjection::Computer => {
                service.computer_secret_input(&credential_id)?
            }
        };
        Ok((credential_id, secret_input))
    }

    async fn dispatch_forwarded_home_request_credential_secret(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        agent: &crate::agent::AgentInstance,
        service: crate::secret::RuntimeSecretService,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        self.ensure_agent_can_manage_user_vault_for_agent(&context.home_session_id, agent)?;
        let _vault_unlock = self
            .ensure_vault_unlocked_for_agent(
                &context.home_session_id,
                agent.id(),
                "runtime_tool_request_credential_secret",
            )
            .await?;
        let args = serde_json::from_value::<
            crate::transport::runtime_tools::RequestCredentialSecretArgs,
        >(arguments)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "runtime_tool_request_credential_secret",
            message: format!("invalid tool arguments: {error}"),
        })?;
        let credential = stamp_runtime_credential_metadata_for_agent(
            credential_from_runtime_input(args.credential)?,
            Some(agent.id()),
            &context.home_session_id,
            agent.primary_provider(),
            Some(&context.worker_provider_run_id),
        );
        match &credential.source {
            crate::config::UserCredentialSourceConfig::Vault { .. } => {}
            crate::config::UserCredentialSourceConfig::Env { .. }
            | crate::config::UserCredentialSourceConfig::File { .. } => {
                return Err(DaemonError::LocalTransport {
                    operation: "runtime_tool_request_credential_secret",
                    message: "runtime-created credentials must use a vault source".to_string(),
                });
            }
        }
        if let Some(max_length) = args.prompt.max_length {
            if max_length < args.prompt.min_length.unwrap_or(1) {
                return Err(DaemonError::LocalTransport {
                    operation: "runtime_tool_request_credential_secret",
                    message: "prompt max_length must be greater than or equal to min_length"
                        .to_string(),
                });
            }
        }
        let interaction = crate::session::RuntimeInteraction::new(
            format!(
                "credential-secret-{}-{}",
                agent.id(),
                crate::session::unix_epoch_ms()
            ),
            agent.id(),
            crate::session::RuntimeInteractionKind::Choice,
            crate::session::RuntimeInteractionLevel::Critical,
            args.prompt.title.clone(),
            args.prompt.message.clone(),
            vec![crate::session::RuntimeInteractionChoice::new(
                "cancel",
                "Cancel",
                "cancel",
                Some(crate::session::RuntimeInteractionChoiceStyle::Danger),
            )],
            Some(crate::session::RuntimeInteractionCustomChoice::secret(
                "secret",
                "Secret",
                args.prompt.placeholder.clone(),
                args.prompt.min_length,
                args.prompt.max_length,
            )),
            args.prompt.timeout_sec,
            Some("cancel".to_string()),
        );
        let interaction_id = interaction.id().to_string();
        let timeout_sec = interaction.timeout_sec();
        let resolution_rx = self
            .create_runtime_interaction(&context.home_session_id, interaction)
            .await?;
        if let Some(timeout_sec) = timeout_sec {
            let state = self.clone();
            let timeout_session_id = context.home_session_id.clone();
            let timeout_interaction_id = interaction_id;
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(timeout_sec)).await;
                let _ = state
                    .timeout_runtime_interaction(&timeout_session_id, &timeout_interaction_id)
                    .await;
            });
        }
        let resolution = resolution_rx
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "runtime_tool_request_credential_secret",
                message: format!(
                    "credential secret interaction dropped before resolution: {error}"
                ),
            })?;
        if resolution.status.to_string() == "timed_out" {
            return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({
                    "credential_id": credential.id,
                    "status": "timed_out",
                }),
            });
        }
        if resolution.choice_id.as_deref() != Some("secret") {
            return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({
                    "credential_id": credential.id,
                    "status": "cancelled",
                }),
            });
        }
        let secret = zeroize::Zeroizing::new(resolution.reply.ok_or_else(|| {
            DaemonError::LocalTransport {
                operation: "runtime_tool_request_credential_secret",
                message: "credential secret interaction resolved without a secret".to_string(),
            }
        })?);
        let registry = crate::credential::CharioxCredentialRegistry::user()?;
        let result = service.upsert_vault_backed_credential_with_secret(
            &registry,
            credential,
            secret.as_str(),
            args.overwrite,
        )?;
        Ok(crate::transport::runtime_tools::RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "credential_id": result.credential_id,
                "status": "stored",
            }),
        })
    }

    fn authorize_home_credential_context(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        super::home_extension_authorizer::authorize_remote_home_context(
            self,
            context,
            "home credential proxy",
        )
    }

    pub(super) fn home_runtime_secret_service(
        &self,
    ) -> Result<crate::secret::RuntimeSecretService, DaemonError> {
        let user_config = self.owned.config_projection.snapshot().user_config;
        let credentials = crate::credential::load_user_credentials()?;
        crate::secret::RuntimeSecretService::with_vault_config(
            credentials,
            &user_config.credential_vault,
        )
    }
}
