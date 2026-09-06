use super::*;

pub(crate) struct VaultUnlockGuard {
    path: Option<std::path::PathBuf>,
    lock_on_drop: bool,
}

impl VaultUnlockGuard {
    fn unlocked_for_operation(path: std::path::PathBuf) -> Self {
        Self {
            path: Some(path),
            lock_on_drop: true,
        }
    }

    fn unlocked_until_expiry() -> Self {
        Self {
            path: None,
            lock_on_drop: false,
        }
    }

    fn not_required() -> Self {
        Self {
            path: None,
            lock_on_drop: false,
        }
    }
}

impl Drop for VaultUnlockGuard {
    fn drop(&mut self) {
        if !self.lock_on_drop {
            return;
        }
        if let Some(path) = self.path.as_ref() {
            let _ = crate::secret::lock_chariox_encrypted_vault(path);
            let _ = crate::secret::clear_vault_secret_process_cache();
        }
    }
}

impl KernelRuntimeState {
    async fn await_vault_interaction(
        &self,
        session_id: &str,
        interaction: crate::session::RuntimeInteraction,
        operation: &'static str,
        interaction_name: &'static str,
    ) -> Result<super::PendingInteractionResolution, DaemonError> {
        let interaction_id = interaction.id().to_string();
        let timeout_sec = interaction.timeout_sec();
        let resolution_rx = self
            .create_runtime_interaction(session_id, interaction)
            .await?;
        if let Some(timeout_sec) = timeout_sec {
            let state = self.clone();
            let timeout_session_id = session_id.to_string();
            let timeout_interaction_id = interaction_id;
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(timeout_sec)).await;
                let _ = state
                    .timeout_runtime_interaction(&timeout_session_id, &timeout_interaction_id)
                    .await;
            });
        }
        resolution_rx
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation,
                message: format!(
                    "{interaction_name} interaction dropped before resolution: {error}"
                ),
            })
    }

    pub(crate) async fn ensure_vault_unlocked_for_command_context(
        &self,
        command: &crate::runtime::command::KernelCommand,
        session_id: Option<&str>,
        agent_id: Option<&str>,
        operation: &'static str,
    ) -> Result<VaultUnlockGuard, DaemonError> {
        let user_config = self.owned.config_projection.snapshot().user_config;
        if user_config.credential_vault.backend
            != crate::config::CredentialVaultBackend::CharioxEncrypted
        {
            return Ok(VaultUnlockGuard::not_required());
        }
        let vault_path = expand_vault_path(&user_config.credential_vault.path);
        if crate::secret::chariox_encrypted_vault_status(&vault_path)?.unlocked {
            return Ok(VaultUnlockGuard::unlocked_until_expiry());
        }
        let session_id = session_id
            .or(command.session_id.as_deref())
            .ok_or_else(|| DaemonError::LocalTransport {
                operation,
                message:
                    "encrypted Chariox vault access requires a session_id so the unlock popup can be shown"
                        .to_string(),
            })?;
        let agent_id = agent_id.or(command.agent_id.as_deref()).unwrap_or("vault");
        self.ensure_vault_unlocked_for_agent(session_id, agent_id, operation)
            .await
    }

    pub(super) async fn ensure_vault_unlocked_for_provider_run(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        operation: &'static str,
    ) -> Result<VaultUnlockGuard, DaemonError> {
        let agent_id =
            provider_run
                .agent_instance_id()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation,
                    message: "provider run is not bound to an agent for vault unlock".to_string(),
                })?;
        self.ensure_vault_unlocked_for_agent(provider_run.session_id(), agent_id, operation)
            .await
    }

    pub(super) async fn ensure_vault_unlocked_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        operation: &'static str,
    ) -> Result<VaultUnlockGuard, DaemonError> {
        let user_config = self.owned.config_projection.snapshot().user_config;
        let vault_config = user_config.credential_vault;
        if vault_config.backend != crate::config::CredentialVaultBackend::CharioxEncrypted {
            return Ok(VaultUnlockGuard::not_required());
        }
        let vault_path = expand_vault_path(&vault_config.path);
        let force_prompt = matches!(
            vault_config.unlock_policy,
            crate::config::CredentialVaultUnlockPolicy::Always
        );
        if !force_prompt && crate::secret::chariox_encrypted_vault_status(&vault_path)?.unlocked {
            return Ok(VaultUnlockGuard::unlocked_until_expiry());
        }
        let unlock_request_lock = vault_unlock_request_lock(&vault_path);
        let _dedupe_guard = unlock_request_lock.lock().await;
        if !force_prompt && crate::secret::chariox_encrypted_vault_status(&vault_path)?.unlocked {
            return Ok(VaultUnlockGuard::unlocked_until_expiry());
        }

        let resolution = self
            .await_vault_interaction(
                session_id,
                vault_passphrase_interaction(session_id, agent_id, operation),
                operation,
                "vault unlock",
            )
            .await?;
        let passphrase = vault_passphrase_from_resolution(resolution, operation)?;
        let choice_id = if matches!(
            vault_config.unlock_policy,
            crate::config::CredentialVaultUnlockPolicy::Ttl
        ) {
            vault_lease_choice_from_resolution(
                self.await_vault_interaction(
                    session_id,
                    vault_unlock_lease_interaction(session_id, agent_id, &vault_config),
                    operation,
                    "vault unlock lease",
                )
                .await?,
                operation,
            )?
        } else if matches!(
            vault_config.unlock_policy,
            crate::config::CredentialVaultUnlockPolicy::KernelInit
        ) {
            "unlock_kernel".to_string()
        } else {
            "unlock_operation".to_string()
        };
        let (lease, lock_after_operation) =
            unlock_lease_for_choice(&choice_id, &vault_config, force_prompt);
        let status =
            crate::secret::unlock_chariox_encrypted_vault(&vault_path, passphrase.as_str(), lease)?;
        crate::logging::info_with_fields(
            "credential_vault",
            "Chariox vault unlocked",
            serde_json::json!({
                "session_id": session_id,
                "agent_id": agent_id,
                "operation": operation,
                "path": status.path.display().to_string(),
                "expires_at_ms": status.expires_at_ms,
                "lock_after_operation": lock_after_operation,
            }),
        );
        if lock_after_operation {
            Ok(VaultUnlockGuard::unlocked_for_operation(vault_path))
        } else {
            Ok(VaultUnlockGuard::unlocked_until_expiry())
        }
    }

    pub(crate) async fn manage_credential_vault_unlock(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(crate::secret::CharioxVaultUnlockStatus, String), DaemonError> {
        let user_config = self.owned.config_projection.snapshot().user_config;
        let vault_config = user_config.credential_vault;
        if vault_config.backend != crate::config::CredentialVaultBackend::CharioxEncrypted {
            let status = crate::secret::chariox_encrypted_vault_status(expand_vault_path(
                &vault_config.path,
            ))?;
            return Ok((status, "not_required".to_string()));
        }
        let vault_path = expand_vault_path(&vault_config.path);
        let status = crate::secret::chariox_encrypted_vault_status(&vault_path)?;
        if !status.unlocked {
            let _guard = self
                .ensure_vault_unlocked_for_agent(session_id, agent_id, "credential_vault_manage")
                .await?;
            let status = crate::secret::chariox_encrypted_vault_status(&vault_path)?;
            return Ok((status, "unlocked".to_string()));
        }

        let resolution = self
            .await_vault_interaction(
                session_id,
                vault_manage_interaction(session_id, agent_id),
                "credential_vault_manage",
                "vault management",
            )
            .await?;
        let choice_id = resolution.choice_id.as_deref().unwrap_or("dismiss");
        apply_vault_manage_choice(&vault_path, choice_id, &vault_config)
    }
}

fn vault_passphrase_interaction(
    session_id: &str,
    agent_id: &str,
    operation: &'static str,
) -> crate::session::RuntimeInteraction {
    let choices = vec![crate::session::RuntimeInteractionChoice::new(
        "cancel",
        "Cancel",
        "cancel",
        Some(crate::session::RuntimeInteractionChoiceStyle::Danger),
    )];
    crate::session::RuntimeInteraction::new(
        format!(
            "vault-unlock-{}-{}",
            agent_id,
            crate::session::unix_epoch_ms()
        ),
        agent_id,
        crate::session::RuntimeInteractionKind::Choice,
        crate::session::RuntimeInteractionLevel::Critical,
        Some("Unlock Chariox Vault".to_string()),
        format!(
            "Agent `{agent_id}` in session `{session_id}` needs Chariox Vault access for `{operation}`. Enter the vault passphrase."
        ),
        choices,
        Some(crate::session::RuntimeInteractionCustomChoice::secret(
            "passphrase",
            "Vault passphrase",
            Some("Passphrase".to_string()),
            Some(1),
            Some(512),
        )),
        Some(300),
        Some("cancel".to_string()),
    )
}

fn vault_unlock_lease_interaction(
    session_id: &str,
    agent_id: &str,
    config: &crate::config::UserCredentialVaultConfig,
) -> crate::session::RuntimeInteraction {
    crate::session::RuntimeInteraction::new(
        format!(
            "vault-unlock-lease-{}-{}",
            agent_id,
            crate::session::unix_epoch_ms()
        ),
        agent_id,
        crate::session::RuntimeInteractionKind::Choice,
        crate::session::RuntimeInteractionLevel::Critical,
        Some("Choose Vault Unlock Duration".to_string()),
        format!("Choose how long to keep the Chariox Vault unlocked for session `{session_id}`."),
        vec![
            crate::session::RuntimeInteractionChoice::new(
                "unlock_operation",
                "This operation",
                "operation",
                Some(crate::session::RuntimeInteractionChoiceStyle::Primary),
            ),
            crate::session::RuntimeInteractionChoice::new(
                "unlock_default_ttl",
                format!(
                    "{} minutes",
                    config.default_ttl_minutes.min(config.max_ttl_minutes)
                ),
                "ttl_default",
                Some(crate::session::RuntimeInteractionChoiceStyle::Primary),
            ),
            crate::session::RuntimeInteractionChoice::new(
                "unlock_60m",
                "1 hour",
                "ttl_60",
                Some(crate::session::RuntimeInteractionChoiceStyle::Secondary),
            ),
            crate::session::RuntimeInteractionChoice::new(
                "unlock_kernel",
                "Until kernel shutdown",
                "kernel_shutdown",
                Some(crate::session::RuntimeInteractionChoiceStyle::Secondary),
            ),
            crate::session::RuntimeInteractionChoice::new(
                "cancel",
                "Cancel",
                "cancel",
                Some(crate::session::RuntimeInteractionChoiceStyle::Danger),
            ),
        ],
        None,
        Some(300),
        Some("cancel".to_string()),
    )
}

fn vault_passphrase_from_resolution(
    resolution: super::PendingInteractionResolution,
    operation: &'static str,
) -> Result<zeroize::Zeroizing<String>, DaemonError> {
    if resolution.status == "timed_out" {
        return Err(DaemonError::LocalTransport {
            operation,
            message: "Chariox vault unlock timed out".to_string(),
        });
    }
    match resolution.choice_id.as_deref() {
        None | Some("cancel") => Err(DaemonError::LocalTransport {
            operation,
            message: "Chariox vault unlock was cancelled".to_string(),
        }),
        Some("passphrase") => resolution
            .reply
            .map(zeroize::Zeroizing::new)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation,
                message: "Chariox vault unlock resolved without a passphrase".to_string(),
            }),
        Some(_) => Err(DaemonError::LocalTransport {
            operation,
            message: "Chariox vault unlock resolved without a passphrase".to_string(),
        }),
    }
}

fn vault_lease_choice_from_resolution(
    resolution: super::PendingInteractionResolution,
    operation: &'static str,
) -> Result<String, DaemonError> {
    if resolution.status == "timed_out" {
        return Err(DaemonError::LocalTransport {
            operation,
            message: "Chariox vault unlock duration selection timed out".to_string(),
        });
    }
    match resolution.choice_id.as_deref() {
        None | Some("cancel") => Err(DaemonError::LocalTransport {
            operation,
            message: "Chariox vault unlock was cancelled".to_string(),
        }),
        Some(
            choice_id
            @ ("unlock_operation" | "unlock_default_ttl" | "unlock_60m" | "unlock_kernel"),
        ) => Ok(choice_id.to_string()),
        Some(_) => Err(DaemonError::LocalTransport {
            operation,
            message: "Chariox vault unlock resolved with an invalid duration".to_string(),
        }),
    }
}

fn unlock_lease_for_choice(
    choice_id: &str,
    config: &crate::config::UserCredentialVaultConfig,
    force_operation: bool,
) -> (crate::secret::VaultUnlockLease, bool) {
    if force_operation {
        return (crate::secret::VaultUnlockLease::Operation, true);
    }
    if matches!(
        config.unlock_policy,
        crate::config::CredentialVaultUnlockPolicy::KernelInit
    ) {
        return (crate::secret::VaultUnlockLease::KernelShutdown, false);
    }
    match choice_id {
        "unlock_operation" => (crate::secret::VaultUnlockLease::Operation, true),
        "unlock_60m" => (
            crate::secret::VaultUnlockLease::TtlMinutes(60.min(config.max_ttl_minutes)),
            false,
        ),
        "unlock_kernel" => (crate::secret::VaultUnlockLease::KernelShutdown, false),
        "passphrase" | "unlock_default_ttl" => (
            crate::secret::VaultUnlockLease::TtlMinutes(
                config
                    .default_ttl_minutes
                    .min(config.max_ttl_minutes)
                    .max(1),
            ),
            false,
        ),
        _ => (crate::secret::VaultUnlockLease::Operation, true),
    }
}

fn vault_manage_interaction(
    session_id: &str,
    agent_id: &str,
) -> crate::session::RuntimeInteraction {
    crate::session::RuntimeInteraction::new(
        format!(
            "vault-manage-{}-{}",
            agent_id,
            crate::session::unix_epoch_ms()
        ),
        agent_id,
        crate::session::RuntimeInteractionKind::Choice,
        crate::session::RuntimeInteractionLevel::Info,
        Some("Chariox Vault Unlocked".to_string()),
        format!(
            "The Chariox Vault is unlocked for session `{session_id}`. Extend the unlock window or lock it now."
        ),
        vec![
            crate::session::RuntimeInteractionChoice::new(
                "extend_30m",
                "Extend 30 minutes",
                "extend_30m",
                Some(crate::session::RuntimeInteractionChoiceStyle::Primary),
            ),
            crate::session::RuntimeInteractionChoice::new(
                "extend_60m",
                "Extend 1 hour",
                "extend_60m",
                Some(crate::session::RuntimeInteractionChoiceStyle::Secondary),
            ),
            crate::session::RuntimeInteractionChoice::new(
                "lock_now",
                "Lock now",
                "lock_now",
                Some(crate::session::RuntimeInteractionChoiceStyle::Danger),
            ),
            crate::session::RuntimeInteractionChoice::new(
                "dismiss",
                "Dismiss",
                "dismiss",
                Some(crate::session::RuntimeInteractionChoiceStyle::Secondary),
            ),
        ],
        None,
        Some(120),
        Some("dismiss".to_string()),
    )
}

fn apply_vault_manage_choice(
    vault_path: &std::path::Path,
    choice_id: &str,
    config: &crate::config::UserCredentialVaultConfig,
) -> Result<(crate::secret::CharioxVaultUnlockStatus, String), DaemonError> {
    match choice_id {
        "extend_30m" => {
            let status = crate::secret::extend_chariox_encrypted_vault(
                vault_path,
                crate::secret::VaultUnlockLease::TtlMinutes(30.min(config.max_ttl_minutes)),
            )?;
            Ok((status, "extended_30m".to_string()))
        }
        "extend_60m" => {
            let status = crate::secret::extend_chariox_encrypted_vault(
                vault_path,
                crate::secret::VaultUnlockLease::TtlMinutes(60.min(config.max_ttl_minutes)),
            )?;
            Ok((status, "extended_60m".to_string()))
        }
        "lock_now" => {
            crate::secret::lock_chariox_encrypted_vault(vault_path)?;
            crate::secret::clear_vault_secret_process_cache()?;
            let status = crate::secret::chariox_encrypted_vault_status(vault_path)?;
            Ok((status, "locked".to_string()))
        }
        _ => {
            let status = crate::secret::chariox_encrypted_vault_status(vault_path)?;
            Ok((status, "dismissed".to_string()))
        }
    }
}

fn vault_unlock_request_lock(path: &std::path::Path) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    static LOCKS: std::sync::OnceLock<
        std::sync::Mutex<
            std::collections::BTreeMap<std::path::PathBuf, std::sync::Arc<tokio::sync::Mutex<()>>>,
        >,
    > = std::sync::OnceLock::new();
    let path = expand_vault_path(&path.to_string_lossy());
    let mut locks = LOCKS
        .get_or_init(|| std::sync::Mutex::new(std::collections::BTreeMap::new()))
        .lock()
        .expect("vault unlock request lock map poisoned");
    locks
        .entry(path)
        .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

fn expand_vault_path(path: &str) -> std::path::PathBuf {
    if path == "~" {
        if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
            return home;
        }
    }
    if let Some(suffix) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
            return home.join(suffix);
        }
    }
    std::path::PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolution(
        choice_id: Option<&str>,
        reply: Option<&str>,
    ) -> super::super::PendingInteractionResolution {
        super::super::PendingInteractionResolution {
            status: "answered",
            choice_id: choice_id.map(ToOwned::to_owned),
            reply: reply.map(ToOwned::to_owned),
        }
    }

    #[test]
    fn vault_passphrase_prompt_cannot_encode_an_unlock_lease_as_the_secret() {
        let interaction = vault_passphrase_interaction("session-1", "agent-1", "test");

        assert_eq!(
            interaction
                .choices()
                .iter()
                .map(|choice| choice.id())
                .collect::<Vec<_>>(),
            vec!["cancel"]
        );
        let custom_choice = interaction
            .custom_choice()
            .expect("vault passphrase prompt should expose secret input");
        assert_eq!(custom_choice.id(), "passphrase");
        assert_eq!(
            custom_choice.input_kind(),
            crate::session::RuntimeInteractionInputKind::Secret
        );

        let error = vault_passphrase_from_resolution(
            resolution(Some("unlock_60m"), Some("ttl_60")),
            "test",
        )
        .expect_err("a lease choice must never become a vault passphrase");
        assert!(matches!(
            error,
            DaemonError::LocalTransport { message, .. }
                if message == "Chariox vault unlock resolved without a passphrase"
        ));
    }

    #[test]
    fn vault_passphrase_prompt_accepts_only_the_secret_custom_choice() {
        let passphrase = vault_passphrase_from_resolution(
            resolution(Some("passphrase"), Some("correct horse battery staple")),
            "test",
        )
        .expect("secret custom choice should resolve the vault passphrase");

        assert_eq!(passphrase.as_str(), "correct horse battery staple");
    }

    #[test]
    fn vault_duration_prompt_is_a_separate_non_secret_interaction() {
        let config = crate::config::UserCredentialVaultConfig::default();
        let interaction = vault_unlock_lease_interaction("session-1", "agent-1", &config);

        assert!(interaction.custom_choice().is_none());
        assert_eq!(
            interaction
                .choices()
                .iter()
                .map(|choice| choice.id())
                .collect::<Vec<_>>(),
            vec![
                "unlock_operation",
                "unlock_default_ttl",
                "unlock_60m",
                "unlock_kernel",
                "cancel",
            ]
        );
        assert_eq!(
            vault_lease_choice_from_resolution(
                resolution(Some("unlock_60m"), Some("ttl_60")),
                "test",
            )
            .expect("known duration should resolve"),
            "unlock_60m"
        );
        assert!(vault_lease_choice_from_resolution(
            resolution(Some("passphrase"), Some("secret")),
            "test",
        )
        .is_err());
    }
}
