//! Worker-confirmed profile changes and remote substitute selection.

use super::*;
use crate::agent::AgentInstance;
use crate::local::AgentSubstituteAction;

pub(super) fn same_execution_profile(a: &AgentInstance, b: &AgentInstance) -> bool {
    a.provider() == b.provider()
        && a.provider_account_profile() == b.provider_account_profile()
        && a.model() == b.model()
        && a.effort() == b.effort()
}

pub(super) fn substitute_target(
    agent: &AgentInstance,
    action: &AgentSubstituteAction,
) -> Result<Option<AgentInstance>, DaemonError> {
    let mut target = agent.clone();
    match action {
        AgentSubstituteAction::Activate { index, reason } => {
            target
                .activate_substitute(*index, reason.clone().unwrap_or_else(|| "manual".into()))
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "activate agent substitute",
                    message: format!("agent `{}` has no substitute at index {index}", agent.id()),
                })?;
        }
        AgentSubstituteAction::Primary {} => target.deactivate_substitute(),
        AgentSubstituteAction::Clear {} => target.clear_substitutes(),
        AgentSubstituteAction::Remove { index } => {
            target.remove_substitute(*index);
        }
        _ => return Ok(None),
    }
    Ok(Some(target))
}

impl KernelRuntimeState {
    pub(super) async fn confirm_remote_agent_profile(
        &self,
        agent_id: &str,
        update: &owned::OwnedRemoteAgentProfileUpdate,
    ) -> Result<(), DaemonError> {
        let mut config = self.config_snapshot().await;
        if let (Some(url), Some(token)) = (update.relay_url.clone(), update.relay_token.clone()) {
            config.apply_remote_relay_override(url, token);
        }
        self.ensure_remote_profile_account(agent_id, update, &config)
            .await?;
        let request = RelayPeerRequest::UpdateLeasedAgentProfile {
            leased_agent_id: update.leased_agent_id.clone(),
            provider: update.provider.clone(),
            account_profile: update.account_profile.clone(),
            model: update.model.clone(),
            effort: update.effort.clone(),
        };
        let response = self
            .send_remote_profile_request(&config, &update.worker_kernel_id, request)
            .await?;
        match response {
            RelayPeerResponse::LeasedAgentProfileUpdated { leased_agent } => {
                update.validate_worker_acknowledgement(agent_id, &leased_agent)
            }
            other => Err(DaemonError::LocalTransport {
                operation: "update remote leased agent profile",
                message: format!("unexpected remote profile response: {other:?}"),
            }),
        }
    }

    pub(super) async fn send_remote_profile_request(
        &self,
        config: &crate::config::DaemonConfig,
        worker_kernel_id: &str,
        request: RelayPeerRequest,
    ) -> Result<RelayPeerResponse, DaemonError> {
        let target = ClientTarget {
            daemon_id: Some(worker_kernel_id.to_string()),
            daemon_alias: None,
        };
        match self.connected_relay_state_for_config(config).await {
            Some(relay_state) => {
                crate::transport::relay_client::send_peer_request_via_connected_relay(
                    config,
                    &relay_state,
                    target,
                    request,
                )
                .await
            }
            None => {
                crate::transport::relay_client::send_peer_request_via_temporary_connection(
                    config, target, request,
                )
                .await
            }
        }
    }

    pub(super) async fn finish_remote_agent_profile_transition(
        &self,
        session_id: &str,
        agent_id: &str,
        claim: crate::runtime::prompt_state::AgentProfileTransitionClaim,
    ) -> Result<(), DaemonError> {
        if let Some(mut submission) = self
            .owned
            .finish_remote_profile_transition(session_id, agent_id, claim)?
        {
            self.finish_owned_prompt_submission_workflow_start(&mut submission)
                .await?;
            self.spawn_remote_prompt_projection_drain_if_needed(&submission);
            if let Some(dispatch) = submission.remote_dispatch.take() {
                self.spawn_remote_prompt_dispatch(dispatch);
            }
        }
        Ok(())
    }

    pub(super) async fn update_remote_agent_substitute(
        &self,
        original: AgentInstance,
        action: AgentSubstituteAction,
        target: AgentInstance,
    ) -> Result<AgentInstance, DaemonError> {
        let session_id = original.session_id().to_string();
        let agent_id = original.id().to_string();
        let claim = self
            .owned
            .prompt_state_owner
            .claim_idle_agent_profile_transition(
                &self.owned.session_store.get_session(&session_id)?,
                &agent_id,
            )?;
        let result = self
            .apply_remote_agent_substitute(original, action, target)
            .await;
        let finish = self
            .finish_remote_agent_profile_transition(&session_id, &agent_id, claim)
            .await;
        if result.is_ok() {
            finish?;
        }
        result
    }

    pub(super) async fn update_remote_agent_substitute_with_claim(
        &self,
        original: AgentInstance,
        action: AgentSubstituteAction,
        target: AgentInstance,
        claim: crate::runtime::prompt_state::AgentProfileTransitionClaim,
    ) -> Result<AgentInstance, DaemonError> {
        let session_id = original.session_id().to_string();
        let agent_id = original.id().to_string();
        let result = self
            .apply_remote_agent_substitute(original, action, target)
            .await;
        let finish = self
            .finish_remote_agent_profile_transition(&session_id, &agent_id, claim)
            .await;
        if result.is_ok() {
            finish?;
        }
        result
    }

    async fn apply_remote_agent_substitute(
        &self,
        original: AgentInstance,
        action: AgentSubstituteAction,
        target: AgentInstance,
    ) -> Result<AgentInstance, DaemonError> {
        let session_id = original.session_id();
        let agent_id = original.id();
        self.owned.ensure_agent_config_not_provider_native_tui(
            session_id,
            agent_id,
            "update agent substitutes",
        )?;
        // Resolve only this explicitly selected account, never another fallback.
        let account = if crate::provider::canonical_provider_family(target.provider()).is_some() {
            let owner = crate::account_profile::provider_account_authority_owner_user_id(
                &self.owned.config_projection.snapshot(),
                original.owner_user_id(),
            );
            self.owned
                .provider_account_profiles
                .get(&owner, target.provider(), target.provider_account_profile())?
                .profile_id
        } else {
            target.provider_account_profile().to_string()
        };
        let binding = original
            .remote_execution()
            .expect("remote substitute requires a worker binding");
        let update = owned::OwnedRemoteAgentProfileUpdate {
            worker_kernel_id: binding.worker_kernel_id.clone(),
            execution_lease_id: binding.execution_lease_id.clone(),
            leased_agent_id: binding.leased_agent_id.clone(),
            relay_url: binding.relay_url.clone(),
            relay_token: binding.relay_token.clone(),
            provider: target.provider().to_string(),
            account_profile: account.clone(),
            model: target.model().map(str::to_string),
            effort: target.effort().map(str::to_string),
        };
        self.confirm_remote_agent_profile(agent_id, &update).await?;
        let agent = {
            let mut agents = self.owned.agent_store.write();
            let current = agents.get_agent(agent_id)?;
            // List edits may happen during I/O. Never apply an index against a
            // changed list or overwrite a newer starter, binding, or owner.
            if !same_execution_profile(&current, &original)
                || current.primary_provider() != original.primary_provider()
                || current.primary_model() != original.primary_model()
                || current.primary_effort() != original.primary_effort()
                || current.primary_account_profile() != original.primary_account_profile()
                || current.substitutes() != original.substitutes()
                || current.active_substitute_index() != original.active_substitute_index()
                || current.remote_execution() != original.remote_execution()
                || current.owner_user_id() != original.owner_user_id()
                || current.session_id() != original.session_id()
            {
                return Err(DaemonError::LocalTransport {
                    operation: "update agent substitutes",
                    message: "agent configuration changed while the worker confirmed the switch; retry with the current configuration".into(),
                });
            }
            // Apply to the current agent so unrelated edits and activity survive.
            let mut committed =
                substitute_target(&current, &action)?.expect("execution-changing action");
            committed.set_account_profile(Some(account));
            committed.set_provider_resume_state(
                current
                    .provider_resume_state()
                    .without_provider_session_id(current.provider())
                    .without_provider_session_id(committed.provider()),
            );
            committed.set_remote_execution_active_worker_provider_run_id(None);
            agents.restore_agent(committed)
        };
        self.owned.session_snapshot(session_id)?;
        self.append_agent_durable_event("agent.updated", &agent, None)
            .await?;
        self.invalidate_workflow_copies_after_source_agent_change(session_id, agent_id)?;
        Ok(agent)
    }
}
