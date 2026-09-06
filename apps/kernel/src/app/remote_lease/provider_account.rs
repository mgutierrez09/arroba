use crate::account_profile::{ProviderAccountMaterialization, ProviderAccountProfile};
use crate::error::DaemonError;
use crate::transport::relay_peer::RemoteProviderAccountSyncContext;

use super::RemoteLeaseRuntime;

impl RemoteLeaseRuntime<'_> {
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
        if crate::provider::canonical_provider_family(&materialization.profile.provider)
            == Some("claude")
            && materialization
                .files
                .iter()
                .any(|file| file.relative_path == ".credentials.json")
        {
            return Err(DaemonError::LocalTransport {
                operation: "ensure remote provider account",
                message: "Claude provider credentials cannot be materialized on a remote worker; use the kernel-managed Chariox-vault setup-token launch path".to_string(),
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

        let claude_with_refresh_credential = ProviderAccountMaterialization {
            profile: crate::account_profile::ProviderAccountReplicaMetadata {
                owner_user_id: "owner-a".to_string(),
                provider: "claude".to_string(),
                profile_id: "work".to_string(),
                label: "Work".to_string(),
                origin: crate::account_profile::ProviderAccountProfileOrigin::CharioxCreated,
                is_default: false,
            },
            files: vec![crate::account_profile::ProviderAccountMaterializationFile {
                relative_path: ".credentials.json".to_string(),
                contents_base64: "bmV2ZXItbG9nLXRoaXM=".to_string(),
            }],
            generated_at_ms: 1,
        };
        let error = RemoteLeaseRuntime::new(&mut app)
            .ensure_remote_provider_account(context.clone(), claude_with_refresh_credential)
            .expect_err("remote Claude refresh credentials must be rejected");
        assert!(error.to_string().contains("setup-token launch path"));
        assert!(!error.to_string().contains("bmV2ZXItbG9nLXRoaXM"));

        let mut wrong_owner = materialization;
        wrong_owner.profile.owner_user_id = "owner-b".to_string();
        assert!(RemoteLeaseRuntime::new(&mut app)
            .ensure_remote_provider_account(context, wrong_owner)
            .is_err());
        let _ = std::fs::remove_dir_all(root);
    }
}
