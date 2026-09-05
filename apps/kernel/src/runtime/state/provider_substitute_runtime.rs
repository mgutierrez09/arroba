//! Provider substitute activation after provider failures.

use super::*;

impl KernelRuntimeState {
    pub(super) async fn activate_next_agent_substitute_after_failure(
        &self,
        session_id: &str,
        agent_id: &str,
        reason: &str,
    ) -> Result<bool, DaemonError> {
        self.activate_next_agent_substitute_after_failure_with_claim(
            session_id, agent_id, reason, None,
        )
        .await
    }

    pub(super) async fn activate_next_agent_substitute_after_failure_with_claim(
        &self,
        session_id: &str,
        agent_id: &str,
        reason: &str,
        profile_transition: Option<crate::runtime::prompt_state::AgentProfileTransitionClaim>,
    ) -> Result<bool, DaemonError> {
        let current = self.owned.agent_store.get_agent(agent_id)?;
        let Some(substitute_index) = self.owned.next_available_substitute_index(&current)? else {
            return Ok(false);
        };
        if current.remote_execution().is_some() {
            let action = crate::local::AgentSubstituteAction::Activate {
                index: substitute_index,
                reason: Some(reason.to_string()),
            };
            let target = super::remote_agent_profile_runtime::substitute_target(&current, &action)?
                .expect("activation selects a substitute profile");
            if let Some(claim) = profile_transition {
                self.update_remote_agent_substitute_with_claim(current, action, target, claim)
                    .await?;
            } else {
                self.update_remote_agent_substitute(current, action, target)
                    .await?;
            }
            return Ok(true);
        }
        drop(profile_transition);
        let (launch_request, runtime_init_delay_ms, agent) = {
            let owned = &self.owned;
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
            let launch_request =
                owned.prepare_provider_launch_request(launch_request, config.runtime_mcp_url())?;
            owned.record_notice(
                session_id,
                None,
                owned
                    .attachment_store
                    .list_session_attachment_ids(session_id),
                format!(
                    "Activating substitute {} for agent `{agent_id}` after {reason}.",
                    substitute_index + 1
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

impl KernelRuntimeOwnedState {
    fn next_available_substitute_index(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Result<Option<usize>, DaemonError> {
        let next = agent
            .active_substitute_index()
            .map(|index| index.saturating_add(1))
            .unwrap_or(0);
        let config = self.config_projection.snapshot();
        let owner = crate::account_profile::provider_account_authority_owner_user_id(
            &config,
            agent.owner_user_id(),
        );
        let now_ms = crate::session::unix_epoch_ms();
        for (index, candidate) in agent.substitutes().iter().enumerate().skip(next) {
            if crate::provider::canonical_provider_family(&candidate.provider).is_some() {
                let Some(account) = self.provider_account_profiles.find(
                    &owner,
                    &candidate.provider,
                    candidate.account_profile.as_deref().unwrap_or("default"),
                )?
                else {
                    self.record_notice(
                        agent.session_id(),
                        None,
                        self.attachment_store.list_session_attachment_ids(agent.session_id()),
                        format!("Skipping substitute {} for agent `{}`: its saved {} account is no longer available.", index + 1, agent.id(), candidate.provider),
                    );
                    continue;
                };
                if account.has_confirmed_exhaustion(&candidate.model, now_ms) {
                    self.record_notice(
                        agent.session_id(),
                        None,
                        self.attachment_store.list_session_attachment_ids(agent.session_id()),
                        format!("Skipping substitute {} for agent `{}`: account `{}` has exhausted capacity for `{}`.", index + 1, agent.id(), account.label, candidate.model),
                    );
                    continue;
                }
            }
            return Ok(Some(index));
        }
        Ok(None)
    }
}
