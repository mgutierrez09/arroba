use crate::error::DaemonError;
use crate::local::{
    DeleteCredentialSecretRequest, GetCredentialVaultStatusRequest, GetUserConfigRequest,
    GetUserConfigSchemaRequest, LocalDaemonRequest, LocalDaemonResponse,
    LockCredentialVaultRequest, ManageCredentialVaultRequest, SetCredentialSecretRequest,
    SetProviderAccountCredentialRequest, SetUserConfigValueRequest,
    SetWorkspaceLiveSyncModeRequest, UnsetUserConfigValueRequest, UserConfigMutationEffect,
};
use crate::runtime::command::{command_caller_user_id, KernelCommand};
use crate::runtime::projection::DaemonConfigProjectionStore;
use crate::runtime::state::{KernelRuntimeState, ProviderReloadTrigger};
use crate::runtime::user_config_policy::{
    summarize_provider_reload_outcomes, user_config_mutation_effects, UserConfigMutation,
};

pub(crate) async fn execute_user_config_request(
    config_projection: &DaemonConfigProjectionStore,
    runtime_state: &KernelRuntimeState,
    command: &KernelCommand,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::GetUserConfig(request) => {
            execute_get_user_config_request(config_projection, request).await
        }
        LocalDaemonRequest::GetUserConfigSchema(request) => {
            execute_get_user_config_schema_request(request).await
        }
        LocalDaemonRequest::SetUserConfigValue(request) => {
            execute_set_user_config_value_request(runtime_state, request).await
        }
        LocalDaemonRequest::SetWorkspaceLiveSyncMode(request) => {
            execute_set_workspace_live_sync_mode_request(runtime_state, command, request).await
        }
        LocalDaemonRequest::UnsetUserConfigValue(request) => {
            execute_unset_user_config_value_request(runtime_state, request).await
        }
        LocalDaemonRequest::SetCredentialSecret(request) => {
            execute_set_credential_secret_request(
                config_projection,
                runtime_state,
                command,
                request,
            )
            .await
        }
        LocalDaemonRequest::DeleteCredentialSecret(request) => {
            execute_delete_credential_secret_request(
                config_projection,
                runtime_state,
                command,
                request,
            )
            .await
        }
        LocalDaemonRequest::SetProviderAccountCredential(request) => {
            execute_set_provider_account_credential_request(
                config_projection,
                runtime_state,
                command,
                request,
            )
            .await
        }
        LocalDaemonRequest::GetCredentialVaultStatus(request) => {
            execute_get_credential_vault_status_request(config_projection, request).await
        }
        LocalDaemonRequest::LockCredentialVault(request) => {
            execute_lock_credential_vault_request(config_projection, request).await
        }
        LocalDaemonRequest::ManageCredentialVault(request) => {
            execute_manage_credential_vault_request(runtime_state, request).await
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "user config request",
            message: "unsupported user config request".to_string(),
        }),
    }
}

pub(crate) async fn execute_set_provider_account_credential_request(
    config_projection: &DaemonConfigProjectionStore,
    runtime_state: &KernelRuntimeState,
    command: &KernelCommand,
    request: SetProviderAccountCredentialRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let owner_user_id =
        runtime_state.provider_account_authority_owner_user_id(&command_caller_user_id(command));
    let provider = crate::provider::validate_provider_account_credential_input(
        &request.provider,
        &request.value,
    )?
    .to_string();
    let profile = runtime_state.provider_account_profile_registry().get(
        &owner_user_id,
        &provider,
        &request.account_profile,
    )?;
    let _vault_unlock = runtime_state
        .ensure_vault_unlocked_for_command_context(
            command,
            request.session_id.as_deref(),
            request.agent_id.as_deref(),
            "provider_account_credential_set",
        )
        .await?;
    let config = config_projection.snapshot();
    let stored = crate::provider::store_provider_account_credential(
        &config,
        &owner_user_id,
        &provider,
        &profile.profile_id,
        &request.value,
        request.overwrite,
    )?;
    runtime_state.record_waiting_room_change();
    Ok(LocalDaemonResponse::ProviderAccountCredentialStored {
        provider,
        account_profile: profile.profile_id,
        credential_id: stored.credential_id,
        replaced: stored.replaced,
    })
}

pub(crate) async fn execute_get_user_config_request(
    config_projection: &DaemonConfigProjectionStore,
    _request: GetUserConfigRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let config = config_projection.snapshot();
    Ok(LocalDaemonResponse::UserConfig {
        path: config.user_config_path().clone(),
        config: config.user_config,
    })
}

pub(crate) async fn execute_get_user_config_schema_request(
    _request: GetUserConfigSchemaRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    Ok(LocalDaemonResponse::UserConfigSchema {
        entries: crate::config::DaemonConfig::user_config_schema(),
    })
}

pub(crate) async fn execute_set_user_config_value_request(
    runtime_state: &KernelRuntimeState,
    request: SetUserConfigValueRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let (config, effects) = apply_user_config_mutation(
        runtime_state,
        UserConfigMutation::Set {
            path: request.path,
            value: request.value,
        },
    )
    .await?;
    Ok(LocalDaemonResponse::UserConfigUpdated {
        path: config.user_config_path().clone(),
        config: config.user_config,
        effects,
    })
}

pub(crate) async fn execute_set_workspace_live_sync_mode_request(
    runtime_state: &KernelRuntimeState,
    command: &KernelCommand,
    request: SetWorkspaceLiveSyncModeRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let session_id = request.session_id.clone();
    let session = runtime_state.set_workspace_live_sync_mode(
        &session_id,
        request.mode,
        &command_caller_user_id(command),
        Some(command),
    )?;
    let outcomes = runtime_state
        .apply_provider_reload_policy(ProviderReloadTrigger::SessionWorkspaceLiveSyncModeChanged {
            session_id,
        })
        .await?;
    let summary = summarize_provider_reload_outcomes(&outcomes);
    let message = if summary.reloaded == 0 && summary.deferred == 0 {
        "session workspace live sync mode updated; no running provider needed reload".to_string()
    } else {
        format!(
            "session workspace live sync mode updated; provider reloads: {} reloaded, {} deferred, {} unaffected",
            summary.reloaded, summary.deferred, summary.unaffected
        )
    };
    Ok(LocalDaemonResponse::WorkspaceLiveSyncModeUpdated {
        session,
        effects: vec![UserConfigMutationEffect {
            kind: "provider_reload".to_string(),
            path: "session.workspace_live_sync_mode".to_string(),
            message,
            provider_reload: Some(summary),
        }],
    })
}

pub(crate) async fn execute_unset_user_config_value_request(
    runtime_state: &KernelRuntimeState,
    request: UnsetUserConfigValueRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let (config, effects) = apply_user_config_mutation(
        runtime_state,
        UserConfigMutation::Unset { path: request.path },
    )
    .await?;
    Ok(LocalDaemonResponse::UserConfigUpdated {
        path: config.user_config_path().clone(),
        config: config.user_config,
        effects,
    })
}

async fn apply_user_config_mutation(
    runtime_state: &KernelRuntimeState,
    mutation: UserConfigMutation,
) -> Result<(crate::config::DaemonConfig, Vec<UserConfigMutationEffect>), DaemonError> {
    let changed_path = match &mutation {
        UserConfigMutation::Set { path, .. } | UserConfigMutation::Unset { path } => {
            path.trim().to_string()
        }
    };
    let config = match mutation {
        UserConfigMutation::Set { path, value } => {
            runtime_state.set_user_config_value(path, value).await?
        }
        UserConfigMutation::Unset { path } => runtime_state.unset_user_config_value(path).await?,
    };
    let effects = user_config_mutation_effects(runtime_state, &changed_path).await?;
    Ok((config, effects))
}

pub(crate) async fn execute_set_credential_secret_request(
    config_projection: &DaemonConfigProjectionStore,
    runtime_state: &KernelRuntimeState,
    command: &KernelCommand,
    request: SetCredentialSecretRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let _vault_unlock = runtime_state
        .ensure_vault_unlocked_for_command_context(
            command,
            request.session_id.as_deref(),
            request.agent_id.as_deref(),
            "credential_secret_set",
        )
        .await?;
    let user_config = config_projection.snapshot().user_config;
    let service = crate::secret::RuntimeSecretService::with_vault_config(
        Vec::new(),
        &user_config.credential_vault,
    )?;
    service.set_vault_secret(&request.key, &request.value)?;
    Ok(LocalDaemonResponse::CredentialSecretStored { key: request.key })
}

pub(crate) async fn execute_delete_credential_secret_request(
    config_projection: &DaemonConfigProjectionStore,
    runtime_state: &KernelRuntimeState,
    command: &KernelCommand,
    request: DeleteCredentialSecretRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let _vault_unlock = runtime_state
        .ensure_vault_unlocked_for_command_context(
            command,
            request.session_id.as_deref(),
            request.agent_id.as_deref(),
            "credential_secret_delete",
        )
        .await?;
    let user_config = config_projection.snapshot().user_config;
    let service = crate::secret::RuntimeSecretService::with_vault_config(
        Vec::new(),
        &user_config.credential_vault,
    )?;
    service.delete_vault_secret(&request.key)?;
    Ok(LocalDaemonResponse::CredentialSecretDeleted { key: request.key })
}

pub(crate) async fn execute_get_credential_vault_status_request(
    config_projection: &DaemonConfigProjectionStore,
    _request: GetCredentialVaultStatusRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let status = credential_vault_status(config_projection)?;
    Ok(LocalDaemonResponse::CredentialVaultStatus { status })
}

pub(crate) async fn execute_lock_credential_vault_request(
    config_projection: &DaemonConfigProjectionStore,
    _request: LockCredentialVaultRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let user_config = config_projection.snapshot().user_config;
    let path = user_config.credential_vault.path.clone();
    crate::secret::lock_chariox_encrypted_vault(&path)?;
    crate::secret::clear_vault_secret_process_cache()?;
    let status = crate::secret::chariox_encrypted_vault_status(&path)?;
    Ok(LocalDaemonResponse::CredentialVaultLocked { status })
}

pub(crate) async fn execute_manage_credential_vault_request(
    runtime_state: &KernelRuntimeState,
    request: ManageCredentialVaultRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let (status, action) = runtime_state
        .manage_credential_vault_unlock(
            &request.session_id,
            request.agent_id.as_deref().unwrap_or("vault"),
        )
        .await?;
    Ok(LocalDaemonResponse::CredentialVaultManaged { status, action })
}

fn credential_vault_status(
    config_projection: &DaemonConfigProjectionStore,
) -> Result<crate::secret::CharioxVaultUnlockStatus, DaemonError> {
    let user_config = config_projection.snapshot().user_config;
    crate::secret::chariox_encrypted_vault_status(&user_config.credential_vault.path)
}
