use sha2::{Digest, Sha256};

use crate::config::DaemonConfig;
use crate::error::DaemonError;

use super::ProviderCredentialEnvironment;

const CLAUDE_OAUTH_TOKEN_ENV: &str = "CLAUDE_CODE_OAUTH_TOKEN";

#[derive(Debug)]
pub(crate) struct StoredProviderAccountCredential {
    pub(crate) credential_id: String,
    pub(crate) replaced: bool,
}

/// Stable handle for the Chariox-vault credential assigned to one provider
/// account. The handle contains no provider secret or host-local path.
pub(crate) fn provider_account_credential_id(
    owner_user_id: &str,
    provider: &str,
    profile_id: &str,
) -> String {
    let identity = format!(
        "{}\0{}\0{}",
        owner_user_id.trim(),
        provider.trim().to_ascii_lowercase(),
        profile_id.trim()
    );
    let digest = Sha256::digest(identity.as_bytes());
    format!("provider-account-{}-{digest:x}", canonical_label(provider))[..64].to_string()
}

pub(crate) fn resolve_provider_account_credentials(
    config: &DaemonConfig,
    owner_user_id: &str,
    provider: &str,
    profile_id: &str,
) -> Result<ProviderCredentialEnvironment, DaemonError> {
    let credential_id = provider_account_credential_id(owner_user_id, provider, profile_id);
    let credentials = crate::credential::load_user_credentials()?;
    if !credentials
        .iter()
        .any(|credential| credential.id == credential_id)
    {
        return Ok(ProviderCredentialEnvironment::default());
    }

    let env_name = match crate::provider::canonical_provider_family(provider) {
        Some("claude") => CLAUDE_OAUTH_TOKEN_ENV,
        _ => {
            return Err(DaemonError::InvalidConfig {
                field: "provider account credential",
                message: "vault-backed provider launch credentials are currently supported only for Claude",
            });
        }
    };
    let service = crate::secret::RuntimeSecretService::with_vault_config(
        credentials,
        &config.user_config.credential_vault,
    )?;
    let mut environment = ProviderCredentialEnvironment::default();
    environment.insert(env_name, service.provider_secret_input(&credential_id)?);
    Ok(environment)
}

pub(crate) fn resolve_provider_account_credentials_for_launch(
    config: &DaemonConfig,
    profiles: &crate::account_profile::ProviderAccountProfileRegistry,
    owner_user_id: &str,
    provider: &str,
    profile_id: &str,
    client_interface: crate::provider::ProviderClientInterface,
) -> Result<ProviderCredentialEnvironment, DaemonError> {
    let environment =
        resolve_provider_account_credentials(config, owner_user_id, provider, profile_id)?;
    if !environment.is_empty()
        || crate::provider::canonical_provider_family(provider) != Some("claude")
        || client_interface == crate::provider::ProviderClientInterface::NativeTui
        || (portable_claude_credentials_authorize_unattended_launch()
            && profiles.has_portable_claude_credentials(owner_user_id, profile_id)?)
    {
        return Ok(environment);
    }
    Err(DaemonError::InvalidConfig {
        field: "provider account credential",
        message: unattended_claude_credential_error_message(),
    })
}

fn portable_claude_credentials_authorize_unattended_launch() -> bool {
    std::env::consts::OS == "linux"
}

fn unattended_claude_credential_error_message() -> &'static str {
    if portable_claude_credentials_authorize_unattended_launch() {
        "unattended Claude launch requires a Chariox-vault setup token or a portable provider-native credential; use `provider setup-token claude <account-profile>` or launch the native Claude TUI to sign in interactively"
    } else {
        "unattended Claude launch requires a Chariox-vault setup token on this platform; use `provider setup-token claude <account-profile>` or launch the native Claude TUI to sign in interactively"
    }
}

pub(crate) fn provider_account_credential_uses_vault(
    owner_user_id: &str,
    provider: &str,
    profile_id: &str,
) -> Result<bool, DaemonError> {
    let credential_id = provider_account_credential_id(owner_user_id, provider, profile_id);
    Ok(crate::credential::load_user_credentials()?
        .iter()
        .find(|credential| credential.id == credential_id)
        .is_some_and(|credential| {
            matches!(
                credential.source,
                crate::config::UserCredentialSourceConfig::Vault { .. }
            )
        }))
}

pub(crate) fn store_provider_account_credential(
    config: &DaemonConfig,
    owner_user_id: &str,
    provider: &str,
    profile_id: &str,
    secret: &str,
    overwrite: bool,
) -> Result<StoredProviderAccountCredential, DaemonError> {
    let provider = validate_provider_account_credential_input(provider, secret)?;
    let secret = secret.trim();
    let credential_id = provider_account_credential_id(owner_user_id, provider, profile_id);
    let registry = crate::credential::CharioxCredentialRegistry::user()?;
    let existing = registry.get(&credential_id)?;
    let replaced = existing.is_some();
    let now_ms = crate::session::unix_epoch_ms();
    let created_at_ms = existing
        .as_ref()
        .and_then(|credential| credential.metadata.as_ref())
        .and_then(|metadata| metadata.created_at_ms)
        .unwrap_or(now_ms);
    let credential = crate::config::UserCredentialConfig {
        id: credential_id.clone(),
        description: Some(format!("{provider} account profile {profile_id}")),
        source: crate::config::UserCredentialSourceConfig::Vault {
            key: credential_id.clone(),
        },
        allowed_hosts: Vec::new(),
        allowed_uses: vec![crate::config::UserCredentialUse::Provider],
        injection: crate::config::UserCredentialInjectionConfig::Provider,
        metadata: Some(crate::config::UserCredentialMetadataConfig {
            created_by_kind: Some("provider_account_profile".to_string()),
            created_by_id: Some(profile_id.to_string()),
            session_id: None,
            provider: Some(provider.to_string()),
            provider_run_id: None,
            vault_key: Some(credential_id.clone()),
            created_at_ms: Some(created_at_ms),
            updated_at_ms: Some(now_ms),
        }),
    };
    let service = crate::secret::RuntimeSecretService::with_vault_config(
        Vec::new(),
        &config.user_config.credential_vault,
    )?;
    service.upsert_vault_backed_credential_with_secret(&registry, credential, secret, overwrite)?;
    Ok(StoredProviderAccountCredential {
        credential_id,
        replaced,
    })
}

pub(crate) fn validate_provider_account_credential_input(
    provider: &str,
    secret: &str,
) -> Result<&'static str, DaemonError> {
    let provider =
        crate::provider::canonical_provider_family(provider).ok_or(DaemonError::InvalidConfig {
            field: "provider account credential",
            message: "unsupported provider",
        })?;
    if provider != "claude" {
        return Err(DaemonError::InvalidConfig {
            field: "provider account credential",
            message: "vault-backed account credentials are currently supported only for Claude",
        });
    }
    if secret.trim().is_empty() {
        return Err(DaemonError::InvalidConfig {
            field: "provider account credential",
            message: "Claude setup token must not be empty",
        });
    }
    Ok(provider)
}

fn canonical_label(provider: &str) -> &'static str {
    crate::provider::canonical_provider_family(provider).unwrap_or("provider")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_handle_is_stable_and_registry_safe() {
        let first = provider_account_credential_id("local", "Claude", "Work account");
        let second = provider_account_credential_id("local", "claude", "Work account");

        assert_eq!(first, second);
        assert!(first.starts_with("provider-account-claude-"));
        assert!(first
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')));
        assert_eq!(first.len(), 64);
    }

    #[test]
    fn distinct_account_profiles_have_distinct_credential_handles() {
        assert_ne!(
            provider_account_credential_id("local", "claude", "personal"),
            provider_account_credential_id("local", "claude", "work")
        );
    }

    #[test]
    fn portable_claude_credentials_authorize_unattended_launch_only_on_linux() {
        assert_eq!(
            portable_claude_credentials_authorize_unattended_launch(),
            cfg!(target_os = "linux")
        );
        let message = unattended_claude_credential_error_message();
        assert_eq!(
            message.contains("portable provider-native credential"),
            cfg!(target_os = "linux")
        );
        assert!(message.contains("Chariox-vault setup token"));
    }

    #[test]
    fn credential_input_validation_rejects_invalid_requests_before_storage() {
        let unsupported = validate_provider_account_credential_input("codex", "token")
            .expect_err("non-Claude credentials should be rejected");
        assert!(unsupported.to_string().contains("only for Claude"));
        let empty = validate_provider_account_credential_input("claude", "  ")
            .expect_err("empty Claude credentials should be rejected");
        assert!(empty.to_string().contains("must not be empty"));
        assert_eq!(
            validate_provider_account_credential_input("Claude", " token ")
                .expect("valid Claude credential should pass"),
            "claude"
        );
    }

    #[test]
    fn vault_requirement_is_derived_from_the_registered_credential_source() {
        let _guard = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-provider-account-vault-requirement-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::env::set_var("CHARIOX_HOME", &root);

        let credential_id = provider_account_credential_id("local", "claude", "work");
        assert!(
            !provider_account_credential_uses_vault("local", "claude", "work")
                .expect("missing credential should not require vault access")
        );
        crate::credential::CharioxCredentialRegistry::user()
            .expect("credential registry should resolve")
            .upsert(crate::config::UserCredentialConfig {
                id: credential_id.clone(),
                description: None,
                source: crate::config::UserCredentialSourceConfig::Vault { key: credential_id },
                allowed_hosts: Vec::new(),
                allowed_uses: vec![crate::config::UserCredentialUse::Provider],
                injection: crate::config::UserCredentialInjectionConfig::Provider,
                metadata: None,
            })
            .expect("provider credential should register");
        assert!(
            provider_account_credential_uses_vault("local", "claude", "work")
                .expect("vault-backed credential should require vault access")
        );

        std::env::remove_var("CHARIOX_HOME");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn configured_provider_credential_resolves_only_into_secret_environment() {
        let _guard = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-provider-account-credential-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::env::set_var("CHARIOX_HOME", &root);
        std::env::set_var("CHARIOX_TEST_CLAUDE_SETUP_TOKEN", "setup-token-secret");

        let credential_id = provider_account_credential_id("local", "claude", "work");
        crate::credential::CharioxCredentialRegistry::user()
            .expect("credential registry should resolve")
            .upsert(crate::config::UserCredentialConfig {
                id: credential_id,
                description: None,
                source: crate::config::UserCredentialSourceConfig::Env {
                    name: "CHARIOX_TEST_CLAUDE_SETUP_TOKEN".to_string(),
                },
                allowed_hosts: Vec::new(),
                allowed_uses: vec![crate::config::UserCredentialUse::Provider],
                injection: crate::config::UserCredentialInjectionConfig::Provider,
                metadata: None,
            })
            .expect("provider credential should register");

        let environment = resolve_provider_account_credentials(
            &crate::config::DaemonConfig::for_tests(),
            "local",
            "claude",
            "work",
        )
        .expect("provider credential should resolve");
        let values = environment.iter().collect::<Vec<_>>();
        assert_eq!(values, vec![(CLAUDE_OAUTH_TOKEN_ENV, "setup-token-secret")]);
        assert!(!format!("{environment:?}").contains("setup-token-secret"));

        std::env::remove_var("CHARIOX_TEST_CLAUDE_SETUP_TOKEN");
        std::env::remove_var("CHARIOX_HOME");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn store_provider_credential_uses_vault_source_and_provider_policy() {
        let _guard = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-provider-account-store-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::env::set_var("CHARIOX_HOME", &root);
        std::env::set_var("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT", "1");
        let mut config = crate::config::DaemonConfig::for_tests();
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::ProcessMemory;

        let stored = store_provider_account_credential(
            &config,
            "local",
            "claude",
            "work",
            "  setup-token-secret  ",
            false,
        )
        .expect("provider credential should store");
        assert!(!stored.replaced);
        let credential = crate::credential::CharioxCredentialRegistry::user()
            .expect("registry should open")
            .get(&stored.credential_id)
            .expect("credential should read")
            .expect("credential should exist");
        assert_eq!(
            credential.source,
            crate::config::UserCredentialSourceConfig::Vault {
                key: stored.credential_id.clone()
            }
        );
        assert_eq!(
            credential.allowed_uses,
            vec![crate::config::UserCredentialUse::Provider]
        );
        assert_eq!(
            credential.injection,
            crate::config::UserCredentialInjectionConfig::Provider
        );
        let resolved = resolve_provider_account_credentials(&config, "local", "claude", "work")
            .expect("stored credential should resolve");
        assert_eq!(
            resolved.iter().collect::<Vec<_>>(),
            vec![(CLAUDE_OAUTH_TOKEN_ENV, "setup-token-secret")]
        );
        let duplicate = store_provider_account_credential(
            &config,
            "local",
            "claude",
            "work",
            "replacement-token",
            false,
        )
        .expect_err("replacement should require explicit overwrite");
        assert!(duplicate.to_string().contains("overwrite=true"));
        let replacement = store_provider_account_credential(
            &config,
            "local",
            "claude",
            "work",
            "replacement-token",
            true,
        )
        .expect("explicit replacement should succeed");
        assert!(replacement.replaced);
        let resolved = resolve_provider_account_credentials(&config, "local", "claude", "work")
            .expect("replacement should resolve");
        assert_eq!(
            resolved.iter().collect::<Vec<_>>(),
            vec![(CLAUDE_OAUTH_TOKEN_ENV, "replacement-token")]
        );

        std::env::remove_var("CHARIOX_HOME");
        std::env::remove_var("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT");
        let _ = std::fs::remove_dir_all(root);
    }
}
