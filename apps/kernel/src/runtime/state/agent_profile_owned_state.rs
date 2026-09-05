//! Agent profile, alias, and substitute administration.
//!
//! Execution-mode and permission overrides live in `agent_config_owned_state`; capability grants
//! live in `capability_owned_state`.

use super::*;

impl owned::OwnedRemoteAgentProfileUpdate {
    pub(super) fn validate_worker_acknowledgement(
        &self,
        home_agent_id: &str,
        leased_agent: &crate::execution_lease::LeasedAgent,
    ) -> Result<(), DaemonError> {
        if leased_agent.id != self.leased_agent_id
            || leased_agent.lease_id != self.execution_lease_id
            || leased_agent.home_agent_id != home_agent_id
            || leased_agent.provider != self.provider
            || leased_agent.account_profile != self.account_profile
            || leased_agent.model != self.model
            || leased_agent.effort != self.effort
        {
            return Err(DaemonError::LocalTransport {
                operation: "update remote leased agent profile",
                message: "the worker acknowledgement does not match the requested agent, lease, or provider profile; the home profile was not changed".to_string(),
            });
        }
        Ok(())
    }
}

impl KernelRuntimeOwnedState {
    pub(super) fn update_agent_profile(
        &self,
        session_id: &str,
        agent_id: &str,
        caller_user_id: &str,
        provider: Option<String>,
        account_profile: Option<String>,
        model: Option<String>,
        effort: Option<Option<String>>,
    ) -> Result<owned::OwnedAgentProfileUpdate, DaemonError> {
        let provider = provider
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let model = model
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let effort = effort.map(|value| {
            value
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        });
        let agent = self.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        self.ensure_agent_owner(agent_id, caller_user_id, "update agent profile")?;
        let session = self.session_store.get_session(session_id)?;
        if self
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .is_some()
        {
            return Err(DaemonError::LocalTransport {
                operation: "update agent profile",
                message: format!(
                    "agent `{agent_id}` has an active turn; update the profile after it finishes"
                ),
            });
        }
        self.ensure_agent_config_not_provider_native_tui(
            session_id,
            agent_id,
            "update agent profile",
        )?;
        // While a substitute is active, profile edits retarget the stored
        // primary snapshot; unspecified fields resolve against that snapshot,
        // not the running substitute.
        let editing_substituted_primary = agent.active_substitute_index().is_some();
        let base_provider = if editing_substituted_primary {
            agent.primary_provider()
        } else {
            agent.provider()
        };
        let base_model = if editing_substituted_primary {
            agent.primary_model()
        } else {
            agent.model()
        };
        let base_effort = if editing_substituted_primary {
            agent.primary_effort()
        } else {
            agent.effort()
        };
        let base_account_profile = if editing_substituted_primary {
            agent.primary_account_profile().unwrap_or("default")
        } else {
            agent.provider_account_profile()
        };
        let target_provider = provider.as_deref().unwrap_or(base_provider).to_string();
        let target_model = model.as_deref().or(base_model).map(str::to_string);
        let requested_account_profile = account_profile.as_deref().unwrap_or(base_account_profile);
        let target_account_profile = if crate::provider::canonical_provider_family(&target_provider)
            .is_some_and(|provider| matches!(provider, "codex" | "claude" | "opencode"))
        {
            let account_owner_user_id =
                crate::account_profile::provider_account_authority_owner_user_id(
                    &self.config_projection.snapshot(),
                    agent.owner_user_id(),
                );
            self.provider_account_profiles
                .get(
                    &account_owner_user_id,
                    &target_provider,
                    requested_account_profile,
                )?
                .profile_id
        } else {
            requested_account_profile.to_string()
        };
        let target_effort = match effort.as_ref() {
            Some(value) => value.as_deref(),
            None => base_effort,
        };
        if editing_substituted_primary {
            // The running substitute is left untouched and returning to
            // primary lands on the edited values.
            let primary_changed = target_provider != agent.primary_provider()
                || target_model.as_deref() != agent.primary_model()
                || target_account_profile != agent.primary_account_profile().unwrap_or("default")
                || target_effort != agent.primary_effort();
            let agent = if primary_changed {
                self.agent_store.set_agent_primary_profile_snapshot(
                    agent_id,
                    &target_provider,
                    target_model,
                    target_effort.map(str::to_string),
                    Some(target_account_profile),
                )?
            } else {
                agent
            };
            return Ok(owned::OwnedAgentProfileUpdate {
                agent,
                terminated_run_ids: Vec::new(),
                remote_update: None,
            });
        }
        let provider_model_or_account_changed = target_provider != agent.provider()
            || target_model.as_deref() != agent.model()
            || target_account_profile != agent.provider_account_profile();
        if !provider_model_or_account_changed && target_effort == agent.effort() {
            return Ok(owned::OwnedAgentProfileUpdate {
                agent,
                terminated_run_ids: Vec::new(),
                remote_update: None,
            });
        }
        let remote_update =
            agent
                .remote_execution()
                .map(|binding| owned::OwnedRemoteAgentProfileUpdate {
                    worker_kernel_id: binding.worker_kernel_id.clone(),
                    execution_lease_id: binding.execution_lease_id.clone(),
                    leased_agent_id: binding.leased_agent_id.clone(),
                    relay_url: binding.relay_url.clone(),
                    relay_token: binding.relay_token.clone(),
                    provider: target_provider.clone(),
                    account_profile: target_account_profile.clone(),
                    model: target_model.clone(),
                    effort: target_effort.map(str::to_string),
                });
        let mut terminated_run_ids = Vec::new();
        if remote_update.is_none() {
            if let Some(run) = self.provider_store.get_run_for_agent(session_id, agent_id) {
                match run.state() {
                    crate::provider::ProviderRunState::Starting
                    | crate::provider::ProviderRunState::Running
                    | crate::provider::ProviderRunState::Parked => {
                        if provider_model_or_account_changed {
                            self.prepare_agent_profile_context_handoff(
                                &run,
                                &target_provider,
                                &target_account_profile,
                                target_model.as_deref(),
                            );
                        }
                        let outcome = self
                            .provider_store
                            .terminate_run_provider_only(session_id, run.id())?;
                        self.clear_active_provider_run_session_pointer(
                            session_id,
                            outcome.run().id(),
                        )?;
                        let ended = outcome.into_run();
                        terminated_run_ids.push(ended.id().to_string());
                        self.provider_run_projection.update(ended);
                    }
                    crate::provider::ProviderRunState::Ended => {
                        self.provider_store.clear_runtime(run.id());
                    }
                }
            }
        }
        let agent = if remote_update.is_some() {
            agent
        } else {
            let mut resume_state = agent.provider_resume_state().clone();
            if provider_model_or_account_changed {
                resume_state = resume_state
                    .without_provider_session_id(agent.provider())
                    .without_provider_session_id(&target_provider);
            }
            let agent = self
                .agent_store
                .set_agent_runtime_profile_with_account_profile(
                    agent_id,
                    &target_provider,
                    target_model,
                    target_effort.map(str::to_string),
                    Some(target_account_profile),
                    resume_state,
                )?;
            let _ = self.session_snapshot(session_id)?;
            agent
        };
        Ok(owned::OwnedAgentProfileUpdate {
            agent,
            terminated_run_ids,
            remote_update,
        })
    }

    pub(super) fn commit_remote_agent_profile_update(
        &self,
        session_id: &str,
        agent_id: &str,
        provider: String,
        account_profile: String,
        model: Option<String>,
        effort: Option<String>,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        let session = self.session_store.get_session(session_id)?;
        if self
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .is_some()
        {
            return Err(DaemonError::LocalTransport {
                operation: "commit remote agent profile",
                message: format!(
                    "agent `{agent_id}` has an active turn; update the profile after it finishes"
                ),
            });
        }
        if crate::provider::canonical_provider_family(&provider).is_some() {
            let account_owner_user_id =
                crate::account_profile::provider_account_authority_owner_user_id(
                    &self.config_projection.snapshot(),
                    agent.owner_user_id(),
                );
            let confirmed = self.provider_account_profiles.get(
                &account_owner_user_id,
                &provider,
                &account_profile,
            )?;
            if confirmed.profile_id != account_profile {
                return Err(DaemonError::LocalTransport {
                    operation: "commit remote agent profile",
                    message: "selected account identity changed while the worker confirmed the profile; select the account again".into(),
                });
            }
        }
        let resume_state = agent
            .provider_resume_state()
            .without_provider_session_id(agent.provider())
            .without_provider_session_id(&provider);
        self.agent_store
            .set_agent_runtime_profile_with_account_profile(
                agent_id,
                &provider,
                model,
                effort,
                Some(account_profile),
                resume_state,
            )?;
        self.agent_store
            .set_remote_execution_active_worker_provider_run_id(agent_id, None)?;
        let agent = self.agent_store.get_agent(agent_id)?;
        let _ = self.session_snapshot(session_id)?;
        Ok(agent)
    }

    pub(super) fn alias_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        caller_user_id: &str,
        alias: Option<String>,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        self.ensure_agent_owner(agent_id, caller_user_id, "alias agent")?;
        self.agent_store.alias_agent(agent_id, alias)
    }

    pub(super) fn update_agent_substitutes(
        &self,
        session_id: &str,
        agent_id: &str,
        caller_user_id: &str,
        action: crate::local::AgentSubstituteAction,
    ) -> Result<(crate::agent::AgentInstance, Option<String>), DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        self.ensure_agent_owner(agent_id, caller_user_id, "update agent substitutes")?;
        let retired_run = self.prepare_agent_substitute_transition(&agent, &action)?;
        let updated = match action {
            crate::local::AgentSubstituteAction::Add {
                provider,
                model,
                variant,
                account_profile,
                kernel_id,
                worktree_id,
            } => {
                let provider = provider.trim().to_string();
                let kernel_id = kernel_id.and_then(|value| {
                    let trimmed = value.trim();
                    (!trimmed.is_empty()).then(|| trimmed.to_string())
                });
                let worktree_id = worktree_id.and_then(|value| {
                    let trimmed = value.trim();
                    (!trimmed.is_empty()).then(|| trimmed.to_string())
                });
                if let Some(kernel_id) = kernel_id.as_deref() {
                    let local_kernel_id = self.config_projection.snapshot().daemon_id;
                    if kernel_id != local_kernel_id {
                        return Err(DaemonError::LocalTransport {
                            operation: "add agent substitute",
                            message: format!(
                                "remote substitute kernel `{kernel_id}` is not supported yet"
                            ),
                        });
                    }
                }
                // Authority seam: a substitute must bind a real stable account
                // profile from the kernel account inventory. An omitted alias
                // resolves the provider's current default to its stable
                // profile_id now, so the substitute never follows a later
                // default change and can never launch via the literal default
                // sentinel. Unregistered providers (no inventory family) keep
                // historical pass-through semantics.
                let requested_account = account_profile
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                let account_profile = if crate::provider::canonical_provider_family(&provider)
                    .is_some_and(|family| matches!(family, "codex" | "claude" | "opencode"))
                {
                    let requested = requested_account.unwrap_or("default");
                    let account_owner_user_id =
                        crate::account_profile::provider_account_authority_owner_user_id(
                            &self.config_projection.snapshot(),
                            agent.owner_user_id(),
                        );
                    match self.provider_account_profiles.get(
                        &account_owner_user_id,
                        &provider,
                        requested,
                    ) {
                        Ok(profile) => Some(profile.profile_id),
                        Err(_error) if requested_account.is_none() => {
                            return Err(DaemonError::LocalTransport {
                                operation: "add agent substitute",
                                message: format!(
                                    "no usable account profile is registered for `{provider}`; \
                                     register one first or bind an explicit account alias"
                                ),
                            });
                        }
                        Err(_) => {
                            // Never echo stable internal profile IDs (or the
                            // literal default sentinel) back to users. If the
                            // selection matches an account registered under a
                            // different provider, surface its public label as
                            // a hint instead.
                            let cross_provider_hint = self
                                .provider_account_profiles
                                .list(&account_owner_user_id, None)
                                .ok()
                                .and_then(|profiles| {
                                    profiles
                                        .iter()
                                        .find(|profile| profile.profile_id == requested)
                                        .map(|profile| {
                                            format!(
                                                " The account alias “{}” is registered for {}.",
                                                profile.label, profile.provider
                                            )
                                        })
                                })
                                .unwrap_or_default();
                            return Err(DaemonError::LocalTransport {
                                operation: "add agent substitute",
                                message: format!(
                                    "no {provider} account matches that selection{cross_provider_hint}; \
                                     choose an available account alias"
                                ),
                            });
                        }
                    }
                } else {
                    requested_account.map(str::to_string)
                };
                self.agent_store.add_agent_substitute(
                    agent_id,
                    crate::agent::AgentSubstituteProfile::new(provider, model, variant)
                        .with_account_profile(account_profile)
                        .with_kernel_id(kernel_id)
                        .with_worktree_id(worktree_id),
                )
            }
            crate::local::AgentSubstituteAction::Remove { index } => {
                self.agent_store.remove_agent_substitute(agent_id, index)
            }
            crate::local::AgentSubstituteAction::Move {
                from_index,
                to_index,
            } => self
                .agent_store
                .move_agent_substitute(agent_id, from_index, to_index),
            crate::local::AgentSubstituteAction::Clear {} => {
                self.agent_store.clear_agent_substitutes(agent_id)
            }
            crate::local::AgentSubstituteAction::SetTimeout { timeout_ms } => self
                .agent_store
                .set_agent_substitution_timeout(agent_id, timeout_ms),
            crate::local::AgentSubstituteAction::Activate { index, reason } => self
                .agent_store
                .activate_agent_substitute(
                    agent_id,
                    index,
                    reason.unwrap_or_else(|| "manual".to_string()),
                )
                .map(|(agent, _profile)| agent),
            crate::local::AgentSubstituteAction::Primary {} => {
                self.agent_store.deactivate_agent_substitute(agent_id)
            }
        }?;
        Ok((updated, retired_run))
    }
}
