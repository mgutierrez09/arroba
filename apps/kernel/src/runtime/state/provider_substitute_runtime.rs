//! Provider substitute activation after provider failures.

use super::*;

impl KernelRuntimeState {
    pub(super) async fn activate_next_agent_substitute_after_failure(
        &self,
        session_id: &str,
        agent_id: &str,
        reason: &str,
    ) -> Result<bool, DaemonError> {
        let (launch_request, runtime_init_delay_ms, agent) = {
            let owned = &self.owned;
            let current = owned.agent_store.get_agent(agent_id)?;
            if current.remote_execution().is_some() {
                return Ok(false);
            }
            let Some(substitute_index) = next_substitute_index(&current) else {
                return Ok(false);
            };
            let (agent, profile) = owned.agent_store.activate_agent_substitute(
                agent_id,
                substitute_index,
                reason.to_string(),
            )?;
            if let Some(kernel_id) = profile.kernel_id.as_deref() {
                let local_kernel_id = owned.config_projection.snapshot().daemon_id;
                if kernel_id != local_kernel_id {
                    return Err(DaemonError::LocalTransport {
                        operation: "activate agent substitute",
                        message: format!(
                            "remote substitute kernel `{kernel_id}` is not supported yet"
                        ),
                    });
                }
            }
            let provider = crate::provider::provider_id_for_launch(&profile.provider);
            let adapter_key = crate::provider::adapter_key_for_provider(provider);
            let account_profile = profile
                .account_profile
                .clone()
                .unwrap_or_else(|| "default".to_string());
            let config = owned.config_projection.snapshot();
            let mut launch_request = crate::provider::LaunchProviderRequest::new(
                session_id,
                adapter_key,
                provider,
                account_profile,
                profile.model.clone(),
            )
            .with_agent_id(agent_id)
            .with_owner_user_id(agent.owner_user_id().to_string())
            .with_variant(profile.variant.clone());
            if let Some(worktree_id) = profile.worktree_id.as_deref() {
                launch_request =
                    launch_request.with_working_directory(std::path::PathBuf::from(worktree_id));
            }
            launch_request = launch_request.with_workspace_live_sync_mode(
                crate::provider::provider_workspace_live_sync_mode_for_session(
                    provider,
                    &config,
                    owned.session_store.get_session(session_id).ok().as_ref(),
                ),
            );
            let launch_request = self
                .prepare_provider_launch_request_with_vault(
                    launch_request,
                    "activate agent substitute",
                )
                .await?;
            owned.record_notice(
                session_id,
                None,
                owned
                    .attachment_store
                    .list_session_attachment_ids(session_id),
                format!(
                    "Activating substitute {} for agent `{agent_id}` after {reason}.",
                    substitute_index
                ),
            );
            let _ = owned.session_snapshot(session_id)?;
            (launch_request, config.provider_runtime_init_delay_ms, agent)
        };

        self.append_agent_durable_event("agent.substitute_activated", &agent, None)
            .await?;
        self.spawn_provider_relaunch(launch_request, runtime_init_delay_ms, None, 0);
        Ok(true)
    }
}

fn next_substitute_index(agent: &crate::agent::AgentInstance) -> Option<usize> {
    let next = agent
        .active_substitute_index()
        .map(|index| index.saturating_add(1))
        .unwrap_or(0);
    (next < agent.substitutes().len()).then_some(next)
}
