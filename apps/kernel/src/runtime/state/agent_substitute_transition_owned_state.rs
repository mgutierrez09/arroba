//! Apply the same idle-only execution boundary to manual substitute changes as profile edits.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn prepare_agent_substitute_transition(
        &self,
        agent: &crate::agent::AgentInstance,
        action: &crate::local::AgentSubstituteAction,
    ) -> Result<Option<String>, DaemonError> {
        use crate::local::AgentSubstituteAction;
        let mut target = agent.clone();
        match action {
            AgentSubstituteAction::Activate { index, .. } => {
                if target.activate_substitute(*index, "manual").is_none() {
                    return Err(DaemonError::LocalTransport {
                        operation: "activate agent substitute",
                        message: format!(
                            "agent `{}` has no substitute at index {index}",
                            agent.id()
                        ),
                    });
                }
            }
            AgentSubstituteAction::Primary {} => target.deactivate_substitute(),
            AgentSubstituteAction::Clear {} => target.clear_substitutes(),
            AgentSubstituteAction::Remove { index } => {
                target.remove_substitute(*index);
            }
            _ => return Ok(None),
        }
        let identity_changed = target.provider() != agent.provider()
            || target.model() != agent.model()
            || target.provider_account_profile() != agent.provider_account_profile();
        if !identity_changed && target.effort() == agent.effort() {
            return Ok(None);
        }
        let session = self.session_store.get_session(agent.session_id())?;
        if self
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent.id())
            .is_some()
        {
            return Err(DaemonError::LocalTransport {
                operation: "update agent substitutes",
                message: format!(
                    "agent `{}` has an active turn; switch substitutes after it finishes",
                    agent.id()
                ),
            });
        }
        self.ensure_agent_config_not_provider_native_tui(
            agent.session_id(),
            agent.id(),
            "update agent substitutes",
        )?;
        if agent.remote_execution().is_some() {
            return Err(DaemonError::LocalTransport {
                operation: "update agent substitutes",
                message: "switch the remote agent's provider through its profile configuration"
                    .to_string(),
            });
        }
        // An account may have been removed since this substitute or starter was saved.
        // Validate before retiring its current run or changing any selection.
        if crate::provider::canonical_provider_family(target.provider()).is_some() {
            let owner = crate::account_profile::provider_account_authority_owner_user_id(
                &self.config_projection.snapshot(),
                agent.owner_user_id(),
            );
            self.provider_account_profiles.get(
                &owner,
                target.provider(),
                target.provider_account_profile(),
            ).map_err(|_| DaemonError::LocalTransport {
                operation: "update agent substitutes",
                message: format!("the saved {} account is unavailable; choose an available account before switching", target.provider()),
            })?;
        }
        let mut retired = None;
        if let Some(run) = self
            .provider_store
            .get_run_for_agent(agent.session_id(), agent.id())
        {
            if run.state() != crate::provider::ProviderRunState::Ended {
                if identity_changed {
                    self.prepare_agent_profile_context_handoff(
                        &run,
                        target.provider(),
                        target.provider_account_profile(),
                        target.model(),
                    );
                }
                let ended = self
                    .provider_store
                    .terminate_run_provider_only(agent.session_id(), run.id())?
                    .into_run();
                self.clear_active_provider_run_session_pointer(agent.session_id(), ended.id())?;
                retired = Some(ended.id().to_string());
                self.provider_run_projection.update(ended);
            }
        }
        if identity_changed {
            let resume = agent
                .provider_resume_state()
                .clone()
                .without_provider_session_id(agent.provider())
                .without_provider_session_id(target.provider());
            self.agent_store
                .write()
                .set_agent_provider_resume_state(agent.id(), resume)?;
        }
        Ok(retired)
    }
}
