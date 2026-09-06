//! Exact selected-account transfer before an existing worker profile changes.

use super::*;
use crate::account_profile::{
    ProviderAccountMaterializationState, ProviderAccountMaterializationStatus,
    ProviderAccountMaterializationTargetKind,
};
use crate::transport::relay_peer::RemoteProviderAccountSyncContext;

impl KernelRuntimeState {
    pub(super) async fn ensure_remote_profile_account(
        &self,
        agent_id: &str,
        update: &owned::OwnedRemoteAgentProfileUpdate,
        config: &crate::config::DaemonConfig,
    ) -> Result<(), DaemonError> {
        let Some(provider) = crate::provider::canonical_provider_family(&update.provider) else {
            return Ok(());
        };
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        let owner = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        let registry = &self.owned.provider_account_profiles;
        let target_kind = if self
            .owned
            .slice_store
            .resolve_by_worker_kernel_ref(&update.worker_kernel_id)
            .is_some()
        {
            ProviderAccountMaterializationTargetKind::Slice
        } else {
            ProviderAccountMaterializationTargetKind::Worker
        };
        let result = async {
            let mut materialization =
                registry.export_materialization(&owner, provider, &update.account_profile)?;
            // A resolved stable ID must cross both account and profile requests.
            if materialization.profile.profile_id != update.account_profile {
                return Err(DaemonError::LocalTransport {
                    operation: "materialize remote profile account",
                    message: "selected account identity changed; select the account again".into(),
                });
            }
            // Cloud-owner aliases are local registry details, not lease owners.
            materialization.profile.owner_user_id = agent.owner_user_id().to_string();
            let response = self
                .send_remote_profile_request(
                    config,
                    &update.worker_kernel_id,
                    RelayPeerRequest::EnsureRemoteProviderAccount {
                        context: RemoteProviderAccountSyncContext {
                            home_kernel_id: config.daemon_id.clone(),
                            home_session_id: agent.session_id().to_string(),
                            home_agent_id: agent_id.to_string(),
                            execution_lease_id: update.execution_lease_id.clone(),
                        },
                        materialization,
                    },
                )
                .await?;
            match response {
                RelayPeerResponse::RemoteProviderAccountEnsured {
                    provider: confirmed_provider,
                    account_profile,
                } if confirmed_provider == provider
                    && account_profile == update.account_profile =>
                {
                    Ok(())
                }
                _ => Err(DaemonError::LocalTransport {
                    operation: "materialize remote profile account",
                    message:
                        "worker did not confirm the selected provider account; profile unchanged"
                            .into(),
                }),
            }
        }
        .await;
        let status = registry.update_materialization_status(
            &owner,
            provider,
            &update.account_profile,
            ProviderAccountMaterializationStatus {
                target_kind,
                target_ref: update.worker_kernel_id.clone(),
                state: if result.is_ok() {
                    ProviderAccountMaterializationState::Materialized
                } else {
                    ProviderAccountMaterializationState::Error
                },
                observed_at_ms: crate::session::unix_epoch_ms(),
                last_error: result
                    .as_ref()
                    .err()
                    .map(|_| "selected account transfer failed; profile unchanged".to_string()),
            },
        );
        // Never destroy an existing lease or discard queued work on failure.
        result?;
        status?;
        Ok(())
    }
}
