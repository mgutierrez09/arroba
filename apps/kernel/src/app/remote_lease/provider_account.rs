use crate::account_profile::{ProviderAccountMaterialization, ProviderAccountProfile};
use crate::error::DaemonError;
use crate::transport::relay_peer::RemoteProviderAccountSyncContext;

use super::RemoteLeaseRuntime;

impl RemoteLeaseRuntime<'_> {
    pub(super) fn resolve_leased_profile_account(
        &self,
        leased_agent: &crate::execution_lease::LeasedAgent,
        provider: &str,
        account_profile: &str,
    ) -> Result<String, DaemonError> {
        if crate::provider::canonical_provider_family(provider).is_none() {
            return Ok(account_profile.to_string());
        }
        let lease = self
            .app
            .execution_leases
            .get(&leased_agent.lease_id)
            .ok_or_else(|| DaemonError::ExecutionLeaseNotFound {
                lease_id: leased_agent.lease_id.clone(),
            })?;
        // Replicas are registered under the lease owner, not the worker's local
        // user or another lease's owner. Resolve aliases before changing a run.
        self.app.provider_account_profile_registry()
            .get(&lease.owner_user_id, provider, account_profile)
            .map(|profile| profile.profile_id)
            .map_err(|_| DaemonError::LocalTransport {
                operation: "update leased agent profile",
                message: format!("the selected {provider} account is unavailable on the worker; connect or select an available account before changing the profile"),
            })
    }

    pub(crate) fn ensure_remote_provider_account(
        &mut self,
        context: RemoteProviderAccountSyncContext,
        materialization: ProviderAccountMaterialization,
    ) -> Result<ProviderAccountProfile, DaemonError> {
        let lease = self
            .app
            .execution_leases
            .get(&context.execution_lease_id)
            .cloned()
            .ok_or_else(|| DaemonError::ExecutionLeaseNotFound {
                lease_id: context.execution_lease_id.clone(),
            })?;
        if lease.home_kernel_id != context.home_kernel_id
            || lease.home_session_id != context.home_session_id
            || lease.home_agent_id != context.home_agent_id
        {
            return Err(DaemonError::LocalTransport {
                operation: "ensure remote provider account",
                message:
                    "provider-account materialization context does not match the execution lease"
                        .to_string(),
            });
        }
        if lease.owner_user_id != materialization.profile.owner_user_id {
            return Err(DaemonError::LocalTransport {
                operation: "ensure remote provider account",
                message:
                    "provider-account materialization owner does not match the execution lease"
                        .to_string(),
            });
        }

        let profile = self
            .app
            .provider_account_profile_registry()
            .materialize_replica(&lease.owner_user_id, &materialization)?;
        self.app.durable_state_store().append_event(
            "provider_account.materialized",
            Some(lease.id),
            serde_json::json!({
                "owner_user_id": profile.owner_user_id,
                "provider": profile.provider,
                "profile_id": profile.profile_id,
                "source_home_kernel_id": context.home_kernel_id,
            }),
        )?;
        Ok(profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn materialization_requires_matching_lease_owner_and_creates_profile_root() {
        let root = std::env::temp_dir().join(format!(
            "chariox-remote-account-materialization-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut config = crate::config::DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        config.user_config.state.path = Some(root.join("state.db").display().to_string());
        let mut app = crate::DaemonApp::bootstrap(config).unwrap();
        let lease = RemoteLeaseRuntime::new(&mut app)
            .create_execution_lease(
                "home-kernel",
                "home-session",
                "home-agent",
                false,
                "owner-a",
            )
            .unwrap();
        let context = RemoteProviderAccountSyncContext {
            home_kernel_id: "home-kernel".to_string(),
            home_session_id: "home-session".to_string(),
            home_agent_id: "home-agent".to_string(),
            execution_lease_id: lease.id.clone(),
        };
        let materialization = ProviderAccountMaterialization {
            profile: crate::account_profile::ProviderAccountReplicaMetadata {
                owner_user_id: "owner-a".to_string(),
                provider: "codex".to_string(),
                profile_id: "work".to_string(),
                label: "Work".to_string(),
                origin: crate::account_profile::ProviderAccountProfileOrigin::CharioxCreated,
                is_default: false,
            },
            files: Vec::new(),
            generated_at_ms: 1,
        };

        let profile = RemoteLeaseRuntime::new(&mut app)
            .ensure_remote_provider_account(context.clone(), materialization.clone())
            .unwrap();
        assert_eq!(profile.profile_id, "work");
        let environment = app
            .provider_account_profile_registry()
            .resolve_environment("owner-a", "codex", "work")
            .unwrap();
        assert!(std::path::Path::new(&environment["CODEX_HOME"])
            .join("config.toml")
            .exists());

        let mut wrong_owner = materialization;
        wrong_owner.profile.owner_user_id = "owner-b".to_string();
        assert!(RemoteLeaseRuntime::new(&mut app)
            .ensure_remote_provider_account(context, wrong_owner)
            .is_err());
        let _ = std::fs::remove_dir_all(root);
    }
}
