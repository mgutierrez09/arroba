use sha2::{Digest, Sha256};

use crate::config::DaemonConfig;
use crate::error::DaemonError;

use super::ProviderCredentialEnvironment;

const CLAUDE_OAUTH_TOKEN_ENV: &str = "CLAUDE_CODE_OAUTH_TOKEN";

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
}
