use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use crate::account_profile::{ProviderAccountAuthState, ProviderAccountProfileRegistry};
use crate::error::DaemonError;
use crate::local::GetProviderAuthStatusRequest;

const PUBLICATION_PROVIDER_ACCOUNT_BINDINGS: &str = "CHARIOX_PUBLICATION_PROVIDER_ACCOUNT_BINDINGS";
const CREDENTIAL_BINDINGS_ROOT: &str = "/home/chariox/.credential-bindings";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicationProviderAccountBindings {
    schema_version: u32,
    #[serde(default)]
    defaults: Vec<PublicationProviderDefaultAccount>,
    accounts: Vec<PublicationProviderAccountBinding>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicationProviderDefaultAccount {
    provider: String,
    account_profile: String,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicationProviderAccountBinding {
    provider: String,
    account_profile: String,
    label: String,
    home: PathBuf,
}

pub(crate) fn materialize_publication_provider_accounts(
    registry: &ProviderAccountProfileRegistry,
    owner_user_id: &str,
) -> Result<(), DaemonError> {
    let Some(source) = std::env::var_os(PUBLICATION_PROVIDER_ACCOUNT_BINDINGS) else {
        return Ok(());
    };
    let bindings = validated_bindings(source.to_string_lossy().as_bytes())?;
    materialize_validated_bindings(registry, owner_user_id, &bindings)?;
    refresh_materialized_provider_account_auth(registry, owner_user_id, &bindings)
}

fn refresh_materialized_provider_account_auth(
    registry: &ProviderAccountProfileRegistry,
    owner_user_id: &str,
    bindings: &PublicationProviderAccountBindings,
) -> Result<(), DaemonError> {
    for binding in &bindings.accounts {
        let result = crate::local::provider_requests::provider_auth_status_response(
            registry,
            owner_user_id,
            GetProviderAuthStatusRequest {
                provider: binding.provider.clone(),
                account_profile: binding.account_profile.clone(),
            },
        );
        if let Err(error) = result {
            registry.update_observation(
                owner_user_id,
                &binding.provider,
                &binding.account_profile,
                ProviderAccountAuthState::Error,
                None,
                None,
                None,
                None,
            )?;
            crate::logging::warn_with_fields(
                "daemon.publication_provider_accounts",
                "publication provider account auth refresh failed",
                serde_json::json!({
                    "provider": binding.provider,
                    "account_profile": binding.account_profile,
                    "error": error.to_string(),
                }),
            );
        }
    }
    Ok(())
}

fn materialize_validated_bindings(
    registry: &ProviderAccountProfileRegistry,
    owner_user_id: &str,
    bindings: &PublicationProviderAccountBindings,
) -> Result<(), DaemonError> {
    for binding in &bindings.accounts {
        registry.materialize_deployment_profile(
            owner_user_id,
            &binding.provider,
            &binding.account_profile,
            &binding.label,
            &binding.home,
        )?;
    }
    for default in &bindings.defaults {
        if bindings.accounts.iter().any(|account| {
            account
                .provider
                .trim()
                .eq_ignore_ascii_case(default.provider.trim())
                && account.account_profile.trim() == default.account_profile.trim()
        }) {
            registry.set_default(owner_user_id, &default.provider, &default.account_profile)?;
        }
    }
    Ok(())
}

fn validated_bindings(source: &[u8]) -> Result<PublicationProviderAccountBindings, DaemonError> {
    let bindings: PublicationProviderAccountBindings =
        serde_json::from_slice(source).map_err(|_| invalid_bindings())?;
    if bindings.schema_version != 1
        || bindings.defaults.len() > 64
        || bindings.accounts.len() > 256
        || bindings.defaults.iter().any(|default| {
            default.provider.trim().is_empty() || default.account_profile.trim().is_empty()
        })
    {
        return Err(invalid_bindings());
    }
    let mut unique = BTreeMap::<(String, String), PublicationProviderAccountBinding>::new();
    for binding in bindings.accounts {
        validate_binding(&binding)?;
        let key = (
            binding.provider.trim().to_lowercase(),
            binding.account_profile.trim().to_string(),
        );
        if let Some(existing) = unique.get(&key) {
            if existing.home != binding.home || existing.label != binding.label {
                return Err(invalid_bindings());
            }
            continue;
        }
        unique.insert(key, binding);
    }
    Ok(PublicationProviderAccountBindings {
        schema_version: bindings.schema_version,
        defaults: bindings.defaults,
        accounts: unique.into_values().collect(),
    })
}

fn validate_binding(binding: &PublicationProviderAccountBinding) -> Result<(), DaemonError> {
    if binding.provider.trim().is_empty()
        || binding.account_profile.trim().is_empty()
        || binding.label.trim().is_empty()
        || !binding.home.is_absolute()
        || !binding
            .home
            .starts_with(Path::new(CREDENTIAL_BINDINGS_ROOT))
        || binding
            .home
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(invalid_bindings());
    }
    Ok(())
}

fn invalid_bindings() -> DaemonError {
    DaemonError::InvalidConfig {
        field: "publication_provider_account_bindings",
        message: "provider account bindings are invalid",
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        materialize_validated_bindings, refresh_materialized_provider_account_auth,
        validated_bindings, PublicationProviderAccountBinding, PublicationProviderAccountBindings,
        PublicationProviderDefaultAccount,
    };
    use crate::account_profile::{ProviderAccountAuthState, ProviderAccountProfileRegistry};

    #[test]
    fn provider_account_materialization_manifest_has_no_agent_scope() {
        let bindings = validated_bindings(
            br#"{
              "schema_version": 1,
              "defaults": [{"provider":"codex","account_profile":"main"}],
              "accounts": [{
                "provider":"codex",
                "account_profile":"secondary",
                "label":"Codex secondary",
                "home":"/home/chariox/.credential-bindings/001/home"
              }]
            }"#,
        )
        .expect("valid deployment account manifest");

        assert_eq!(bindings.accounts.len(), 1);
        assert_eq!(bindings.accounts[0].account_profile, "secondary");
    }

    #[test]
    fn provider_account_materialization_rejects_obsolete_agent_scope() {
        assert!(validated_bindings(
            br#"{
              "schema_version": 1,
              "defaults": [],
              "accounts": [{
                "agent_id":"agent-1",
                "provider":"codex",
                "account_profile":"secondary",
                "label":"Codex secondary",
                "home":"/home/chariox/.credential-bindings/001/home"
              }]
            }"#,
        )
        .is_err());
    }

    #[test]
    fn publication_default_account_is_registered_as_default() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "chariox-publication-provider-default-{}-{unique}",
            std::process::id()
        ));
        let source_home = root.join("source-home");
        fs::create_dir_all(source_home.join(".codex")).expect("create source profile");
        fs::write(source_home.join(".codex/auth.json"), "{}").expect("write source credential");
        let registry = ProviderAccountProfileRegistry::open(root.join("registry.json"))
            .expect("open registry");
        let bindings = PublicationProviderAccountBindings {
            schema_version: 1,
            defaults: vec![PublicationProviderDefaultAccount {
                provider: "codex".to_string(),
                account_profile: "profile-codex".to_string(),
            }],
            accounts: vec![PublicationProviderAccountBinding {
                provider: "codex".to_string(),
                account_profile: "profile-codex".to_string(),
                label: "Codex deployment".to_string(),
                home: source_home,
            }],
        };

        materialize_validated_bindings(&registry, "local", &bindings)
            .expect("materialize publication accounts");

        let profile = registry
            .get("local", "codex", "profile-codex")
            .expect("resolve publication account");
        assert!(profile.is_default);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn publication_claude_account_is_authenticated_before_prompt_admission() {
        let _guard = crate::env_lock::lock();
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "chariox-publication-claude-auth-{}-{unique}",
            std::process::id()
        ));
        let source_home = root.join("source-home");
        fs::create_dir_all(source_home.join(".claude")).expect("create source profile");
        fs::write(
            source_home.join(".claude/.credentials.json"),
            r#"{"claudeAiOauth":{"refreshToken":"portable-refresh-token"}}"#,
        )
        .expect("write source credential");

        let claude = root.join("claude");
        fs::write(
            &claude,
            r#"#!/bin/sh
set -eu
if [ "$#" -ge 3 ] && [ "$1" = "auth" ] && [ "$2" = "status" ] && [ "$3" = "--json" ]; then
  printf '%s\n' '{"loggedIn":true,"authMethod":"claude.ai","email":"deployment@example.test","subscriptionType":"pro"}'
  exit 0
fi
if [ "$#" -ge 1 ] && [ "$1" = "--version" ]; then
  printf '%s\n' 'claude 1.2.3'
  exit 0
fi
exit 2
"#,
        )
        .expect("write Claude fixture");
        let mut permissions = fs::metadata(&claude)
            .expect("read Claude fixture metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&claude, permissions).expect("make Claude fixture executable");
        std::env::set_var("CHARIOX_CLAUDE_BIN", &claude);

        let registry = ProviderAccountProfileRegistry::open(root.join("registry.json"))
            .expect("open registry");
        let bindings = PublicationProviderAccountBindings {
            schema_version: 1,
            defaults: vec![PublicationProviderDefaultAccount {
                provider: "claude".to_string(),
                account_profile: "profile-claude".to_string(),
            }],
            accounts: vec![PublicationProviderAccountBinding {
                provider: "claude".to_string(),
                account_profile: "profile-claude".to_string(),
                label: "Claude deployment".to_string(),
                home: source_home,
            }],
        };

        materialize_validated_bindings(&registry, "local", &bindings)
            .expect("materialize publication accounts");
        assert!(registry
            .require_authenticated(
                "local",
                "claude",
                "profile-claude",
                Some("sonnet"),
                "submit prompt",
            )
            .is_err());

        refresh_materialized_provider_account_auth(&registry, "local", &bindings)
            .expect("refresh deployment account auth");

        registry
            .require_authenticated(
                "local",
                "claude",
                "profile-claude",
                Some("sonnet"),
                "submit prompt",
            )
            .expect("authenticated deployment account should pass prompt admission");

        fs::write(
            &claude,
            r#"#!/bin/sh
set -eu
if [ "$#" -ge 3 ] && [ "$1" = "auth" ] && [ "$2" = "status" ] && [ "$3" = "--json" ]; then
  printf '%s\n' '{"loggedIn":false}'
  exit 0
fi
if [ "$#" -ge 1 ] && [ "$1" = "--version" ]; then
  printf '%s\n' 'claude 1.2.3'
  exit 0
fi
exit 2
"#,
        )
        .expect("replace Claude fixture with unauthenticated response");
        refresh_materialized_provider_account_auth(&registry, "local", &bindings)
            .expect("record unauthenticated deployment account");
        assert!(registry
            .require_authenticated(
                "local",
                "claude",
                "profile-claude",
                Some("sonnet"),
                "submit prompt",
            )
            .is_err());

        fs::write(
            &claude,
            r#"#!/bin/sh
set -eu
if [ "$#" -ge 3 ] && [ "$1" = "auth" ] && [ "$2" = "status" ] && [ "$3" = "--json" ]; then
  printf '%s\n' 'not-json'
  exit 0
fi
if [ "$#" -ge 1 ] && [ "$1" = "--version" ]; then
  printf '%s\n' 'claude 1.2.3'
  exit 0
fi
exit 2
"#,
        )
        .expect("replace Claude fixture with operational failure");
        refresh_materialized_provider_account_auth(&registry, "local", &bindings)
            .expect("operational auth failure must not abort publication startup");
        assert_eq!(
            registry
                .get("local", "claude", "profile-claude")
                .expect("resolve failed deployment account")
                .auth_state,
            ProviderAccountAuthState::Error
        );

        std::env::remove_var("CHARIOX_CLAUDE_BIN");
        let _ = fs::remove_dir_all(root);
    }
}
