use std::path::PathBuf;

use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::provider::ProviderRunState;
use crate::runtime::state::KernelRuntimeState;

pub(crate) async fn execute_provider_account_request(
    runtime_state: &KernelRuntimeState,
    owner_user_id: &str,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let owner_user_id = runtime_state.provider_account_authority_owner_user_id(owner_user_id);
    let invalidates_catalog = !matches!(
        request,
        LocalDaemonRequest::ListProviderAccountProfiles(_)
            | LocalDaemonRequest::GetProviderAccountProfile(_)
    );
    let registry = runtime_state.provider_account_profile_registry().clone();
    let runtime_for_checks = runtime_state.clone();
    let response = tokio::task::spawn_blocking(move || match request {
        LocalDaemonRequest::ListProviderAccountProfiles(request) => {
            Ok(LocalDaemonResponse::ProviderAccountProfilesListed {
                profiles: registry.list(&owner_user_id, request.provider.as_deref())?,
            })
        }
        LocalDaemonRequest::GetProviderAccountProfile(request) => {
            Ok(LocalDaemonResponse::ProviderAccountProfile {
                profile: registry.get(
                    &owner_user_id,
                    &request.provider,
                    &request.account_profile,
                )?,
            })
        }
        LocalDaemonRequest::CreateProviderAccountProfile(request) => {
            Ok(LocalDaemonResponse::ProviderAccountProfile {
                profile: registry.create_managed(
                    &owner_user_id,
                    &request.provider,
                    &request.label,
                )?,
            })
        }
        LocalDaemonRequest::LinkProviderAccountProfile(request) => {
            Ok(LocalDaemonResponse::ProviderAccountProfile {
                profile: registry.link_existing(
                    &owner_user_id,
                    &request.provider,
                    &request.label,
                    &PathBuf::from(request.path),
                )?,
            })
        }
        LocalDaemonRequest::ImportNativeProviderAccountProfile(request) => {
            let native_owner = runtime_for_checks
                .provider_account_authority_owner_user_id(crate::session::DEFAULT_LOCAL_USER_ID);
            if owner_user_id != native_owner {
                return Err(DaemonError::LocalTransport {
                    operation: "import native account profile",
                    message:
                        "only the provider-account authority owner may import a host-native account"
                            .to_string(),
                });
            }
            let home = std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "import native account profile",
                    message: "the kernel host HOME must be an absolute path".to_string(),
                })?;
            Ok(LocalDaemonResponse::ProviderAccountProfile {
                profile: registry.import_native_default(
                    &owner_user_id,
                    &request.provider,
                    &home,
                )?,
            })
        }
        LocalDaemonRequest::RenameProviderAccountProfile(request) => {
            Ok(LocalDaemonResponse::ProviderAccountProfile {
                profile: registry.rename(
                    &owner_user_id,
                    &request.provider,
                    &request.account_profile,
                    &request.label,
                )?,
            })
        }
        LocalDaemonRequest::SetDefaultProviderAccountProfile(request) => {
            Ok(LocalDaemonResponse::ProviderAccountProfile {
                profile: registry.set_default(
                    &owner_user_id,
                    &request.provider,
                    &request.account_profile,
                )?,
            })
        }
        LocalDaemonRequest::RefreshProviderAccountProfile(request) => {
            Ok(LocalDaemonResponse::ProviderAccountProfile {
                profile:
                    crate::local::provider_requests::refresh_provider_account_profile_response(
                        &registry,
                        &owner_user_id,
                        &request.provider,
                        &request.account_profile,
                    )?,
            })
        }
        LocalDaemonRequest::RemoveProviderAccountProfile(request) => {
            ensure_profile_idle(
                &runtime_for_checks,
                &registry,
                &owner_user_id,
                &request.provider,
                &request.account_profile,
            )?;
            invalidate_profile_endpoint(
                &owner_user_id,
                &request.provider,
                &request.account_profile,
            );
            Ok(LocalDaemonResponse::ProviderAccountProfileRemoved {
                profile: registry.remove_registration(
                    &owner_user_id,
                    &request.provider,
                    &request.account_profile,
                )?,
            })
        }
        LocalDaemonRequest::DeleteProviderAccountProfileData(request) => {
            ensure_profile_idle(
                &runtime_for_checks,
                &registry,
                &owner_user_id,
                &request.provider,
                &request.account_profile,
            )?;
            invalidate_profile_endpoint(
                &owner_user_id,
                &request.provider,
                &request.account_profile,
            );
            Ok(LocalDaemonResponse::ProviderAccountProfileDataDeleted {
                profile: registry.delete_managed_profile_data(
                    &owner_user_id,
                    &request.provider,
                    &request.account_profile,
                    &request.confirmation_profile_id,
                )?,
            })
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "provider account request",
            message: "unsupported provider account request".to_string(),
        }),
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "provider account request",
        message: error.to_string(),
    })??;
    if invalidates_catalog {
        runtime_state
            .with_app_side_effect(|app| app.invalidate_provider_catalog_cache())
            .await;
        runtime_state.record_waiting_room_change();
    }
    Ok(response)
}

fn invalidate_profile_endpoint(owner_user_id: &str, provider: &str, account_profile: &str) {
    match crate::provider::canonical_provider_family(provider) {
        Some("codex") => {
            crate::provider::invalidate_codex_account_endpoint(owner_user_id, account_profile)
        }
        Some("opencode") => {
            crate::provider::invalidate_opencode_account_endpoint(owner_user_id, account_profile)
        }
        _ => {}
    }
}

pub(crate) fn ensure_profile_idle(
    runtime_state: &KernelRuntimeState,
    registry: &crate::account_profile::ProviderAccountProfileRegistry,
    owner_user_id: &str,
    provider: &str,
    profile_id: &str,
) -> Result<(), DaemonError> {
    let profile = registry.get(owner_user_id, provider, profile_id)?;
    let bound_agents = bound_agent_labels(
        &runtime_state.list_session_snapshots(),
        &profile.provider,
        |agent_owner_user_id| {
            runtime_state.provider_account_authority_owner_user_id(agent_owner_user_id)
                == owner_user_id
        },
        |account_profile| {
            account_profile_reference_matches(
                registry,
                owner_user_id,
                &profile.provider,
                account_profile,
                &profile.profile_id,
            )
        },
    );
    if !bound_agents.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "mutate provider account profile",
            message: format!(
                "account profile `{}` is assigned to agent(s) {}; choose another account for those agents before removing or deleting it",
                profile.label,
                bound_agents.join(", "),
            ),
        });
    }
    let active = runtime_state
        .provider_runs_for_external_session_attachment()
        .into_iter()
        .any(|run| {
            let run_owner_user_id =
                runtime_state.provider_account_authority_owner_user_id(run.owner_user_id());
            run_owner_user_id == owner_user_id
                && crate::provider::canonical_provider_family(run.provider())
                    == crate::provider::canonical_provider_family(provider)
                && account_profile_reference_matches(
                    registry,
                    &run_owner_user_id,
                    &profile.provider,
                    run.account_profile(),
                    &profile.profile_id,
                )
                && run.state() != ProviderRunState::Ended
        });
    if active {
        return Err(DaemonError::LocalTransport {
            operation: "mutate provider account profile",
            message: format!(
                "account profile `{}` has an active provider run; end the run before removing or deleting it",
                profile.label
            ),
        });
    }
    if runtime_state
        .provider_login_process_store()
        .has_running_for_profile(owner_user_id, &profile.provider, &profile.profile_id)
    {
        return Err(DaemonError::LocalTransport {
            operation: "mutate provider account profile",
            message: format!(
                "account profile `{}` has an active provider authentication workflow; cancel it before removing or deleting the profile",
                profile.label
            ),
        });
    }
    Ok(())
}

fn bound_agent_labels(
    sessions: &[crate::session::RuntimeSession],
    provider: &str,
    owner_matches: impl Fn(&str) -> bool,
    account_profile_matches: impl Fn(&str) -> bool,
) -> Vec<String> {
    let provider_family = crate::provider::canonical_provider_family(provider);
    let mut labels = sessions
        .iter()
        .flat_map(|session| session.agents())
        .filter(|agent| {
            owner_matches(agent.owner_user_id())
                && agent_provider_account_references(agent).any(|(provider, account_profile)| {
                    crate::provider::canonical_provider_family(provider) == provider_family
                        && account_profile_matches(account_profile)
                })
        })
        .map(|agent| agent.alias().unwrap_or(agent.id()).to_string())
        .collect::<Vec<_>>();
    labels.sort();
    labels.dedup();
    labels
}

fn agent_provider_account_references(
    agent: &crate::agent::AgentInstance,
) -> impl Iterator<Item = (&str, &str)> {
    std::iter::once((agent.provider(), agent.provider_account_profile()))
        .chain(std::iter::once((
            agent.primary_provider(),
            agent.primary_account_profile().unwrap_or("default"),
        )))
        .chain(agent.substitutes().iter().map(|profile| {
            (
                profile.provider.as_str(),
                profile.account_profile.as_deref().unwrap_or("default"),
            )
        }))
}

fn account_profile_reference_matches(
    registry: &crate::account_profile::ProviderAccountProfileRegistry,
    owner_user_id: &str,
    provider: &str,
    account_profile: &str,
    target_profile_id: &str,
) -> bool {
    registry
        .get(owner_user_id, provider, account_profile)
        .is_ok_and(|profile| profile.profile_id == target_profile_id)
}

#[cfg(test)]
mod tests {
    use super::{account_profile_reference_matches, bound_agent_labels};
    use crate::agent::{AgentInstance, AgentSubstituteProfile, GridPosition};
    use crate::session::RuntimeSession;

    #[test]
    fn account_binding_detection_covers_provider_families_and_agent_owners() {
        let mut session = RuntimeSession::new(
            "session-a",
            Some("accounts".to_string()),
            "workspace-a",
            "worktree-a",
            "machine-a",
            "kernel-a",
        );
        let mut bound = AgentInstance::new(
            "agent-bound",
            "agent-bound",
            session.id(),
            Some("reviewer".to_string()),
            "claude-headless",
            Some("opus".to_string()),
            Some("high".to_string()),
            None,
            GridPosition::new(0, 0, 1, 1),
        );
        bound.set_owner_user_id("owner-a");
        bound.set_account_profile(Some("secondary".to_string()));
        let mut other_owner = bound.clone();
        other_owner.set_alias(Some("other-owner".to_string()));
        other_owner.set_owner_user_id("owner-b");
        let mut other_account = bound.clone();
        other_account.set_alias(Some("other-account".to_string()));
        other_account.set_account_profile(Some("default".to_string()));
        session.set_agents(vec![bound, other_owner, other_account]);

        assert_eq!(
            bound_agent_labels(
                &[session],
                "claude",
                |owner| owner == "owner-a",
                |account_profile| account_profile == "secondary",
            ),
            vec!["reviewer".to_string()],
        );
    }

    #[test]
    fn account_binding_detection_covers_saved_starter_and_inactive_substitutes() {
        let mut session = RuntimeSession::new(
            "session-a",
            Some("accounts".to_string()),
            "workspace-a",
            "worktree-a",
            "machine-a",
            "kernel-a",
        );
        let mut agent = AgentInstance::new(
            "agent-bound",
            "agent-bound",
            session.id(),
            Some("reviewer".to_string()),
            "codex",
            Some("gpt-5.6".to_string()),
            Some("high".to_string()),
            None,
            GridPosition::new(0, 0, 1, 1),
        );
        agent.set_owner_user_id("owner-a");
        agent.set_account_profile(Some("starter-account".to_string()));
        agent.add_substitute(
            AgentSubstituteProfile::new("opencode", "deepseek-v4-pro", Some("high".into()))
                .with_account_profile(Some("substitute-account".to_string())),
        );
        agent.activate_substitute(0, "manual");
        session.set_agents(vec![agent]);

        assert_eq!(
            bound_agent_labels(
                &[session.clone()],
                "codex",
                |owner| owner == "owner-a",
                |account_profile| account_profile == "starter-account",
            ),
            vec!["reviewer".to_string()],
            "the saved starter remains an account binding while a substitute is active",
        );
        assert_eq!(
            bound_agent_labels(
                &[session],
                "opencode",
                |owner| owner == "owner-a",
                |account_profile| account_profile == "substitute-account",
            ),
            vec!["reviewer".to_string()],
            "every configured substitute remains an account binding",
        );
    }

    #[test]
    fn account_binding_detection_resolves_default_alias_to_stable_id() {
        let root = std::env::temp_dir().join(format!(
            "chariox-provider-account-binding-test-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let home = root.join("home");
        std::fs::create_dir_all(&home).unwrap();
        let registry = crate::account_profile::ProviderAccountProfileRegistry::open(
            root.join("accounts.json"),
        )
        .unwrap();
        let profile = registry
            .migrate_effective_defaults("owner-a", &home)
            .unwrap()
            .into_iter()
            .find(|profile| profile.provider == "codex")
            .unwrap();

        assert!(account_profile_reference_matches(
            &registry,
            "owner-a",
            "codex",
            "default",
            &profile.profile_id,
        ));
        assert!(account_profile_reference_matches(
            &registry,
            "owner-a",
            "codex",
            &profile.profile_id,
            &profile.profile_id,
        ));
        assert!(!account_profile_reference_matches(
            &registry,
            "owner-a",
            "codex",
            "missing",
            &profile.profile_id,
        ));
        let _ = std::fs::remove_dir_all(root);
    }
}
