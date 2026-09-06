use std::path::Path;

use crate::agent::CreateAgentRequest;
use crate::agent::GitWorktreePlacement;
use crate::app::DaemonApp;
use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::error::DaemonError;
use crate::execution_lease::{ExecutionLease, LeasedAgent};
use crate::provider::ProviderRunState;
use crate::session::CreateSessionRequest;

mod git_observation;
mod mcp_availability;
mod native_provider;
mod projection;
mod prompt_attachments;
mod prompt_lifecycle;
mod provider_account;
mod provider_run;
mod relay_context;
mod skill_sync;

pub(crate) use projection::RemoteProviderFailure;
pub(crate) use prompt_lifecycle::PreparedLeasedProviderRun;

pub(crate) struct RemoteLeaseRuntime<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> RemoteLeaseRuntime<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn create_execution_lease(
        &mut self,
        home_kernel_id: &str,
        home_session_id: &str,
        home_agent_id: &str,
        home_agent_metaagent: bool,
        owner_user_id: &str,
    ) -> Result<ExecutionLease, DaemonError> {
        if !self.app.config.accept_remote_leases {
            return Err(DaemonError::RemoteLeasesDisabled {
                machine_id: self.app.config.host_machine_id.clone(),
            });
        }
        self.app.next_execution_lease_number = self.app.next_execution_lease_number.wrapping_add(1);
        let lease_id = format!(
            "lease-{:016x}",
            crate::session::unix_epoch_ms() ^ self.app.next_execution_lease_number.rotate_left(11)
        );
        let lease = ExecutionLease::new(
            lease_id.clone(),
            home_kernel_id.to_string(),
            home_session_id.to_string(),
            home_agent_id.to_string(),
            home_agent_metaagent,
            owner_user_id.to_string(),
            self.app.config.daemon_id.clone(),
            self.app.config.host_machine_id.clone(),
        );
        self.app.execution_leases.insert(lease_id, lease.clone());
        Ok(lease)
    }

    pub(crate) fn destroy_execution_lease(
        &mut self,
        lease_id: &str,
    ) -> Result<ExecutionLease, DaemonError> {
        self.app
            .leased_agents
            .retain(|_, agent| agent.lease_id != lease_id);
        self.app.execution_leases.remove(lease_id).ok_or_else(|| {
            DaemonError::ExecutionLeaseNotFound {
                lease_id: lease_id.to_string(),
            }
        })
    }

    pub(crate) fn create_leased_agent(
        &mut self,
        lease_id: &str,
        provider: &str,
        account_profile: &str,
        model: Option<String>,
        effort: Option<String>,
        execution_mode: Option<crate::provider::AgentExecutionMode>,
        permission_level: Option<crate::provider::AgentPermissionLevel>,
        workspace_live_sync_mode: Option<crate::config::WorkspaceLiveSyncMode>,
        worktree_id: Option<String>,
        worktree_placement: Option<GitWorktreePlacement>,
    ) -> Result<LeasedAgent, DaemonError> {
        let base_directory =
            std::env::current_dir().map_err(|error| DaemonError::LocalTransport {
                operation: "resolve leased agent working directory",
                message: error.to_string(),
            })?;
        self.create_leased_agent_from_base_directory(
            &base_directory,
            lease_id,
            provider,
            account_profile,
            model,
            effort,
            execution_mode,
            permission_level,
            workspace_live_sync_mode,
            worktree_id,
            worktree_placement,
        )
    }

    pub(crate) fn create_leased_agent_from_base_directory(
        &mut self,
        base_directory: &Path,
        lease_id: &str,
        provider: &str,
        account_profile: &str,
        model: Option<String>,
        effort: Option<String>,
        execution_mode: Option<crate::provider::AgentExecutionMode>,
        permission_level: Option<crate::provider::AgentPermissionLevel>,
        workspace_live_sync_mode: Option<crate::config::WorkspaceLiveSyncMode>,
        worktree_id: Option<String>,
        worktree_placement: Option<GitWorktreePlacement>,
    ) -> Result<LeasedAgent, DaemonError> {
        let lease = self
            .app
            .execution_leases
            .get(lease_id)
            .cloned()
            .ok_or_else(|| DaemonError::ExecutionLeaseNotFound {
                lease_id: lease_id.to_string(),
            })?;
        let adapter_key = crate::provider::adapter_key_for_provider(provider);
        if self.app.providers.registry().resolve(adapter_key).is_none() {
            return Err(DaemonError::ProviderAdapterNotFound {
                adapter_key: adapter_key.to_string(),
            });
        }
        let worktree = if let Some(placement) = worktree_placement {
            crate::git_worktree_placement::prepare_git_worktree(
                &placement,
                base_directory,
                worktree_id.as_deref(),
                "create remote git worktree",
            )?
        } else {
            match worktree_id {
                Some(worktree) => worktree,
                None => base_directory.display().to_string(),
            }
        };
        let worktree_path = Path::new(&worktree);
        if !worktree_path.exists() {
            return Err(DaemonError::LocalTransport {
                operation: "resolve leased agent working directory",
                message: format!("remote working directory `{worktree}` does not exist"),
            });
        }
        if !worktree_path.is_dir() {
            return Err(DaemonError::LocalTransport {
                operation: "resolve leased agent working directory",
                message: format!("remote working directory `{worktree}` is not a directory"),
            });
        }
        let workspace_id = format!("remote-lease:{}", lease.home_session_id);
        let existing_session = self
            .app
            .leased_agents
            .values()
            .filter(|agent| {
                self.app
                    .execution_leases
                    .get(&agent.lease_id)
                    .is_some_and(|existing_lease| {
                        existing_lease.home_session_id == lease.home_session_id
                    })
            })
            .filter_map(|agent| {
                self.app
                    .sessions
                    .get_session(&agent.backing_session_id)
                    .ok()
            })
            .find(|session| {
                session.workspace_id() == workspace_id
                    && session.worktree_id() == worktree
                    && session.owner_user_id() == lease.owner_user_id
            });
        let session = match existing_session {
            Some(session) => {
                if let Some(mode) = workspace_live_sync_mode {
                    if session.workspace_live_sync_mode() != Some(mode) {
                        self.app
                            .sessions
                            .write()
                            .set_workspace_live_sync_mode(session.id(), mode)?
                    } else {
                        session
                    }
                } else {
                    session
                }
            }
            None => {
                let mut request = CreateSessionRequest::new(workspace_id.clone(), worktree.clone())
                    .with_hidden(true)
                    .with_owner_user_id(lease.owner_user_id.clone());
                if let Some(mode) = workspace_live_sync_mode {
                    request = request.with_workspace_live_sync_mode(mode);
                }
                self.app.sessions.create_ephemeral_session(request)?
            }
        };
        let session_store = self.app.session_state_store();
        let attachment = {
            let mut sessions = session_store.write();
            self.app.attachments.attach(
                &mut sessions,
                AttachRequest::for_user(
                    session.id(),
                    format!("leased-agent:{}", lease.home_agent_id),
                    ClientCapabilityLevel::MessageTransport,
                    lease.owner_user_id.clone(),
                ),
            )?
        };
        let mut backing_agent = {
            let mut sessions = session_store.write();
            let mut request = CreateAgentRequest::new(session.id(), provider)
                .with_owner_user_id(lease.owner_user_id.clone())
                .with_account_profile(account_profile.to_string())
                .with_worktree(session.worktree_id());
            request.model = model.clone();
            request.effort = effort.clone();
            if let Some(execution_mode) = execution_mode {
                request = request.with_execution_mode_override(execution_mode);
            }
            if let Some(permission_level) = permission_level {
                request = request.with_permission_level_override(permission_level);
            }
            self.app.agents.create_agent(request, &mut sessions)?
        };
        if lease.home_agent_metaagent {
            backing_agent = self
                .app
                .agents_mut()
                .activate_agent_meta_mode(backing_agent.id(), None)?;
        }
        self.app.next_leased_agent_number = self.app.next_leased_agent_number.wrapping_add(1);
        let agent_id = format!(
            "leased-agent-{:016x}",
            crate::session::unix_epoch_ms() ^ self.app.next_leased_agent_number.rotate_left(13)
        );
        let agent = LeasedAgent::new(
            agent_id.clone(),
            lease_id.to_string(),
            lease.home_agent_id.clone(),
            provider.to_string(),
            account_profile.to_string(),
            model,
            effort,
            execution_mode,
            permission_level,
            session.id().to_string(),
            backing_agent.id().to_string(),
            attachment.id().to_string(),
        );
        self.app.leased_agents.insert(agent_id, agent.clone());
        Ok(agent)
    }

    pub(crate) fn destroy_leased_agent(
        &mut self,
        leased_agent_id: &str,
    ) -> Result<LeasedAgent, DaemonError> {
        let agent = self
            .app
            .leased_agents
            .remove(leased_agent_id)
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        self.app
            .leased_workflow_turns
            .retain(|_, binding| binding.leased_agent_id != leased_agent_id);
        let provider_runs = self
            .app
            .providers
            .list_runs()
            .into_iter()
            .filter(|run| {
                run.session_id() == agent.backing_session_id
                    && run.agent_instance_id() == Some(agent.backing_agent_id.as_str())
                    && run.state() != ProviderRunState::Ended
            })
            .collect::<Vec<_>>();
        for provider_run in provider_runs {
            let run_id = provider_run.id().to_string();
            let _ = crate::app::provider_runtime::ProviderProcessTracker::new(self.app)
                .remove_run(&run_id);
            if let Ok(outcome) = self
                .app
                .providers
                .terminate_run_provider_only(provider_run.session_id(), provider_run.id())
            {
                let _ = self
                    .app
                    .sessions
                    .set_active_provider_run(outcome.run().session_id(), None);
                self.app.update_provider_run_projection(outcome.into_run());
            }
        }
        let backing_session_still_used = self
            .app
            .leased_agents
            .values()
            .any(|candidate| candidate.backing_session_id == agent.backing_session_id);
        let session_store = self.app.session_state_store();
        let _ = {
            let mut sessions = session_store.write();
            self.app
                .attachments
                .detach(&mut sessions, &agent.backing_attachment_id)
        };
        let _ = {
            let mut sessions = session_store.write();
            self.app
                .agents
                .destroy_agent(&agent.backing_agent_id, &mut sessions)
        };
        if !backing_session_still_used {
            let _ = self.app.sessions.end_session(&agent.backing_session_id);
            let _ = self.app.sessions.delete_session(&agent.backing_session_id);
        }
        Ok(agent)
    }

    pub(crate) fn leased_workflow_event_capabilities_for_backing_prompt(
        &self,
        session_id: &str,
        agent_id: &str,
        backing_prompt_id: &str,
    ) -> Option<(bool, bool, bool)> {
        self.app
            .leased_workflow_turns
            .values()
            .find(|binding| {
                binding.backing_prompt_id == backing_prompt_id
                    && self
                        .app
                        .leased_agents
                        .get(&binding.leased_agent_id)
                        .is_some_and(|agent| {
                            agent.backing_session_id == session_id
                                && agent.backing_agent_id == agent_id
                        })
            })
            .map(|binding| {
                (
                    binding.context.event_reply_enabled,
                    binding.context.event_context_enabled,
                    binding.context.event_actions_enabled,
                )
            })
    }

    pub(crate) fn activate_leased_workflow_prompt(
        &mut self,
        backing_prompt_id: &str,
        provider_run_id: &str,
    ) {
        // The backing queue is keyed by the worker prompt id, and the binding
        // registry uses that same id as its collision-free identity. Resolve by
        // the value as a defensive measure so persisted/older state remains
        // promotable if its map key was generated by an earlier version.
        let Some(binding_key) = self
            .app
            .leased_workflow_turns
            .iter()
            .find(|(_, binding)| binding.backing_prompt_id == backing_prompt_id)
            .map(|(key, _)| key.clone())
        else {
            return;
        };
        let Some(binding) = self.app.leased_workflow_turns.get(&binding_key).cloned() else {
            return;
        };
        if let Some(agent) = self.app.leased_agents.get_mut(&binding.leased_agent_id) {
            if agent.active_home_prompt_id.as_deref() != Some(binding.home_prompt_id.as_str()) {
                agent.applied_home_steer_ids.clear();
                agent.replayable_completion = None;
            }
            agent.active_home_prompt_id = Some(binding.home_prompt_id.clone());
            agent.active_home_prompt_started_at_ms = Some(crate::session::unix_epoch_ms());
        }
        if let Some(binding) = self.app.leased_workflow_turns.get_mut(&binding_key) {
            binding.provider_run_id = provider_run_id.to_string();
        }
    }

    pub(crate) fn update_leased_agent_config(
        &mut self,
        leased_agent_id: &str,
        execution_mode: crate::provider::AgentExecutionMode,
        permission_level: crate::provider::AgentPermissionLevel,
    ) -> Result<LeasedAgent, DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        if self
            .app
            .prompt_owner_active_prompt_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )?
            .is_some()
        {
            return Err(DaemonError::LocalTransport {
                operation: "update leased agent config",
                message: format!(
                    "leased agent `{leased_agent_id}` has an active turn; update the config after it finishes"
                ),
            });
        }

        let config_changed = leased_agent.execution_mode != Some(execution_mode)
            || leased_agent.permission_level != Some(permission_level);
        if config_changed {
            if let Some(run) = self.app.providers.get_run_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            ) {
                match run.state() {
                    ProviderRunState::Starting
                    | ProviderRunState::Running
                    | ProviderRunState::Parked => {
                        let run_id = run.id().to_string();
                        let _ = crate::app::provider_runtime::ProviderProcessTracker::new(self.app)
                            .remove_run(&run_id);
                        if let Ok(outcome) = self
                            .app
                            .providers
                            .terminate_run_provider_only(run.session_id(), run.id())
                        {
                            let _ = self
                                .app
                                .sessions
                                .set_active_provider_run(outcome.run().session_id(), None);
                            self.app.update_provider_run_projection(outcome.into_run());
                        }
                    }
                    ProviderRunState::Ended => {
                        self.app.providers.clear_runtime(run.id());
                    }
                }
            }
        }

        let _ = self.app.agents.update_agent_config(
            &leased_agent.backing_agent_id,
            Some(Some(execution_mode)),
            Some(Some(permission_level)),
            None,
            None,
        )?;
        let updated = self
            .app
            .leased_agents
            .get_mut(leased_agent_id)
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        updated.execution_mode = Some(execution_mode);
        updated.permission_level = Some(permission_level);
        Ok(updated.clone())
    }

    pub(crate) fn update_leased_agent_profile(
        &mut self,
        leased_agent_id: &str,
        provider: String,
        account_profile: String,
        model: Option<String>,
        effort: Option<String>,
    ) -> Result<LeasedAgent, DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        let account_profile =
            self.resolve_leased_profile_account(&leased_agent, &provider, &account_profile)?;
        let profile_changed = leased_agent.provider != provider
            || leased_agent.account_profile != account_profile
            || leased_agent.model != model
            || leased_agent.effort != effort;
        // A delivery retry may confirm its profile after the original prompt started.
        // Confirmation is read-only; an actual change still requires an idle agent.
        if !profile_changed {
            let backing = self.app.agents.get_agent(&leased_agent.backing_agent_id)?;
            if backing.provider() != provider
                || backing.provider_account_profile() != account_profile
                || backing.model() != model.as_deref()
                || backing.effort() != effort.as_deref()
            {
                return Err(DaemonError::LocalTransport {
                    operation: "confirm leased agent profile",
                    message: "leased profile differs from the backing agent; rebind the remote agent before dispatch".to_string(),
                });
            }
            return Ok(leased_agent);
        }
        if self
            .app
            .prompt_owner_active_prompt_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )?
            .is_some()
            || self
                .app
                .prompt_owner_peek_next_queued_prompt(
                    &leased_agent.backing_session_id,
                    &leased_agent.backing_agent_id,
                )?
                .is_some()
        {
            return Err(DaemonError::LocalTransport {
                operation: "update leased agent profile",
                message: format!(
                    "leased agent `{leased_agent_id}` has an active turn or queued prompt; update the profile after pending work finishes"
                ),
            });
        }

        self.terminate_backing_provider_runtime(&leased_agent);
        let backing_agent = self.app.agents.get_agent(&leased_agent.backing_agent_id)?;
        let resume_state = backing_agent
            .provider_resume_state()
            .without_provider_session_id(backing_agent.provider())
            .without_provider_session_id(&provider);
        self.app
            .agents
            .set_agent_runtime_profile_with_account_profile(
                &leased_agent.backing_agent_id,
                &provider,
                model.clone(),
                effort.clone(),
                Some(account_profile.clone()),
                resume_state,
            )?;
        let updated = self
            .app
            .leased_agents
            .get_mut(leased_agent_id)
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        updated.provider = provider;
        updated.account_profile = account_profile;
        updated.model = model;
        updated.effort = effort;
        Ok(updated.clone())
    }

    pub(crate) fn update_leased_agent_meta_mode(
        &mut self,
        leased_agent_id: &str,
        active: bool,
    ) -> Result<LeasedAgent, DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        let backing_agent = self.app.agents.get_agent(&leased_agent.backing_agent_id)?;
        if self
            .app
            .prompt_owner_active_prompt_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )?
            .is_some()
        {
            return Err(DaemonError::LocalTransport {
                operation: "update leased agent meta mode",
                message: format!(
                    "leased agent `{leased_agent_id}` has an active turn; update meta mode after it finishes"
                ),
            });
        }

        let changed = backing_agent.is_metaagent() != active;
        if changed {
            if active {
                self.app
                    .agents_mut()
                    .activate_agent_meta_mode(&leased_agent.backing_agent_id, None)?;
            } else {
                self.app
                    .agents_mut()
                    .deactivate_agent_meta_mode(&leased_agent.backing_agent_id)?;
            }
            self.terminate_backing_provider_runtime(&leased_agent);
        }

        Ok(leased_agent)
    }

    pub(crate) fn update_leased_agent_remote_extension_manifest(
        &mut self,
        leased_agent_id: &str,
        remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    ) -> Result<(), DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        if let Some(run) = self.app.providers.get_run_for_agent(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        ) {
            let updated = self
                .app
                .providers
                .update_run_remote_extension_manifest(run.id(), remote_extension_manifest)?;
            self.app.update_provider_run_projection(updated);
        }
        Ok(())
    }

    fn terminate_backing_provider_runtime(&mut self, leased_agent: &LeasedAgent) {
        let Some(run) = self.app.providers.get_run_for_agent(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        ) else {
            return;
        };
        match run.state() {
            ProviderRunState::Starting | ProviderRunState::Running | ProviderRunState::Parked => {
                let run_id = run.id().to_string();
                let _ = crate::app::provider_runtime::ProviderProcessTracker::new(self.app)
                    .remove_run(&run_id);
                if let Ok(outcome) = self
                    .app
                    .providers
                    .terminate_run_provider_only(run.session_id(), run.id())
                {
                    let _ = self
                        .app
                        .sessions
                        .set_active_provider_run(outcome.run().session_id(), None);
                    self.app.update_provider_run_projection(outcome.into_run());
                }
            }
            ProviderRunState::Ended => {
                self.app.providers.clear_runtime(run.id());
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn execution_lease_count(&self) -> usize {
        self.app.execution_leases.len()
    }

    #[cfg(test)]
    pub(crate) fn leased_agent_count(&self) -> usize {
        self.app.leased_agents.len()
    }

    #[cfg(test)]
    pub(crate) fn leased_agent_snapshot_for_test(
        &self,
        leased_agent_id: &str,
    ) -> Option<LeasedAgent> {
        self.app.leased_agents.get(leased_agent_id).cloned()
    }

    #[cfg(test)]
    pub(crate) fn leased_workflow_turn_binding_for_test(
        &self,
        home_prompt_id: &str,
    ) -> Option<(String, String, bool)> {
        self.app
            .leased_workflow_turns
            .values()
            .find(|binding| binding.home_prompt_id == home_prompt_id)
            .map(|binding| {
                (
                    binding.backing_prompt_id.clone(),
                    binding.provider_run_id.clone(),
                    binding.context.event_reply_enabled,
                )
            })
    }

    #[cfg(test)]
    pub(crate) fn has_leased_workflow_turn_binding_for_test(&self, home_prompt_id: &str) -> bool {
        self.app
            .leased_workflow_turns
            .values()
            .any(|binding| binding.home_prompt_id == home_prompt_id)
    }

    #[cfg(test)]
    pub(crate) fn leased_workflow_turn_binding_count_for_test(
        &self,
        home_prompt_id: &str,
    ) -> usize {
        self.app
            .leased_workflow_turns
            .values()
            .filter(|binding| binding.home_prompt_id == home_prompt_id)
            .count()
    }

    #[cfg(test)]
    pub(crate) fn clear_active_home_prompt_projection_for_test(&mut self, leased_agent_id: &str) {
        if let Some(agent) = self.app.leased_agents.get_mut(leased_agent_id) {
            agent.active_home_prompt_id = None;
            agent.active_home_prompt_started_at_ms = None;
        }
    }

    #[cfg(test)]
    pub(crate) fn set_leased_agent_provider_for_test(
        &mut self,
        leased_agent_id: &str,
        provider: &str,
    ) {
        if let Some(agent) = self.app.leased_agents.get_mut(leased_agent_id) {
            agent.provider = provider.to_string();
        }
    }

    #[cfg(test)]
    pub(crate) fn push_projected_output_history_key_for_test(
        &mut self,
        leased_agent_id: &str,
        key: String,
    ) {
        if let Some(agent) = self.app.leased_agents.get_mut(leased_agent_id) {
            agent.projected_output_history_keys.push(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    mod profile_admission;

    #[test]
    fn leased_profile_update_rejects_missing_account_without_retiring_provider() {
        assert_leased_profile_account_rejection(false);
    }

    #[test]
    fn leased_profile_update_rejects_another_owners_account_without_retiring_provider() {
        assert_leased_profile_account_rejection(true);
    }

    fn assert_leased_profile_account_rejection(wrong_owner: bool) {
        let (mut app, leased_agent) = leased_agent_fixture(false);
        let requested_account = if wrong_owner {
            app.provider_account_profile_registry()
                .create_managed("unrelated-owner", "codex", "Unrelated")
                .unwrap()
                .profile_id
        } else {
            "missing-worker-account".into()
        };
        let run = app
            .providers_mut()
            .launch_run_detached(
                crate::provider::LaunchProviderRequest::new(
                    &leased_agent.backing_session_id,
                    "dev-stub",
                    "dev-stub",
                    "default",
                    "old-model",
                )
                .with_agent_id(&leased_agent.backing_agent_id),
            )
            .unwrap();
        let before = serde_json::to_value(
            app.agents()
                .get_agent(&leased_agent.backing_agent_id)
                .unwrap(),
        )
        .unwrap();
        let result = RemoteLeaseRuntime::new(&mut app).update_leased_agent_profile(
            &leased_agent.id,
            "codex".into(),
            requested_account.clone(),
            Some("new-model".into()),
            Some("low".into()),
        );
        assert!(
            result.is_err(),
            "worker must validate its account before accepting a profile change"
        );
        assert!(!result.unwrap_err().to_string().contains(&requested_account));
        assert_eq!(
            serde_json::to_value(
                app.agents()
                    .get_agent(&leased_agent.backing_agent_id)
                    .unwrap()
            )
            .unwrap(),
            before
        );
        assert_eq!(
            app.providers().get_run(run.id()).unwrap().state(),
            ProviderRunState::Running
        );
        assert_eq!(
            app.leased_agents
                .get(&leased_agent.id)
                .unwrap()
                .account_profile,
            leased_agent.account_profile
        );
    }

    #[test]
    fn leased_profile_update_resolves_default_to_lease_owners_stable_account() {
        let (mut app, leased_agent) = leased_agent_fixture(false);
        let profile = app
            .provider_account_profile_registry()
            .get(crate::session::DEFAULT_LOCAL_USER_ID, "codex", "default")
            .unwrap();
        let updated = RemoteLeaseRuntime::new(&mut app)
            .update_leased_agent_profile(
                &leased_agent.id,
                "codex".into(),
                "default".into(),
                Some("new-model".into()),
                Some("low".into()),
            )
            .unwrap();
        assert_ne!(updated.account_profile, "default");
        assert_eq!(updated.account_profile, profile.profile_id);
        assert_eq!(
            app.agents()
                .get_agent(&leased_agent.backing_agent_id)
                .unwrap()
                .provider_account_profile(),
            profile.profile_id
        );
    }

    #[test]
    fn leased_profile_confirmation_is_idempotent_during_an_active_prompt() {
        let (mut app, leased_agent) = leased_agent_fixture(false);
        let run = app
            .providers_mut()
            .launch_run_detached(
                crate::provider::LaunchProviderRequest::new(
                    &leased_agent.backing_session_id,
                    "dev-stub",
                    &leased_agent.provider,
                    &leased_agent.account_profile,
                    leased_agent.model.as_deref().unwrap(),
                )
                .with_agent_id(&leased_agent.backing_agent_id),
            )
            .unwrap();
        sync_active_prompt(&mut app, &leased_agent);
        let before = serde_json::to_value(&leased_agent).unwrap();
        let confirmed = RemoteLeaseRuntime::new(&mut app)
            .update_leased_agent_profile(
                &leased_agent.id,
                leased_agent.provider.clone(),
                leased_agent.account_profile.clone(),
                leased_agent.model.clone(),
                leased_agent.effort.clone(),
            )
            .expect("a retry may confirm the exact profile without interrupting its active turn");
        assert_eq!(serde_json::to_value(confirmed).unwrap(), before);
        assert_eq!(
            app.providers().get_run(run.id()).unwrap().state(),
            ProviderRunState::Running
        );
        assert_eq!(
            app.prompt_owner_active_prompt_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )
            .unwrap()
            .unwrap()
            .id(),
            "active-prompt"
        );
        for (provider, account, model, effort) in [
            (
                "other-provider".to_string(),
                leased_agent.account_profile.clone(),
                leased_agent.model.clone(),
                leased_agent.effort.clone(),
            ),
            (
                leased_agent.provider.clone(),
                "other-account".to_string(),
                leased_agent.model.clone(),
                leased_agent.effort.clone(),
            ),
            (
                leased_agent.provider.clone(),
                leased_agent.account_profile.clone(),
                Some("other-model".into()),
                leased_agent.effort.clone(),
            ),
            (
                leased_agent.provider.clone(),
                leased_agent.account_profile.clone(),
                leased_agent.model.clone(),
                Some("high".into()),
            ),
        ] {
            assert!(RemoteLeaseRuntime::new(&mut app).update_leased_agent_profile(
                &leased_agent.id, provider, account, model, effort,
            ).is_err(), "confirmation must never permit a changed profile during a turn");
            assert_eq!(
                serde_json::to_value(app.leased_agents.get(&leased_agent.id).unwrap()).unwrap(),
                before
            );
            assert_eq!(
                app.providers().get_run(run.id()).unwrap().state(),
                ProviderRunState::Running
            );
        }
    }

    #[test]
    fn leased_profile_confirmation_rejects_backing_agent_profile_drift() {
        let (mut app, leased_agent) = leased_agent_fixture(false);
        let backing = app
            .agents()
            .get_agent(&leased_agent.backing_agent_id)
            .unwrap();
        app.agents_mut()
            .set_agent_runtime_profile_with_account_profile(
                &leased_agent.backing_agent_id,
                &leased_agent.provider,
                leased_agent.model.clone(),
                leased_agent.effort.clone(),
                Some("different-account".into()),
                backing.provider_resume_state().clone(),
            )
            .unwrap();
        let result = RemoteLeaseRuntime::new(&mut app).update_leased_agent_profile(
            &leased_agent.id,
            leased_agent.provider.clone(),
            leased_agent.account_profile.clone(),
            leased_agent.model.clone(),
            leased_agent.effort.clone(),
        );
        assert!(
            result.is_err(),
            "confirmation must check the backing agent, not only the lease record"
        );
        assert_eq!(
            app.agents()
                .get_agent(&leased_agent.backing_agent_id)
                .unwrap()
                .provider_account_profile(),
            "different-account"
        );
    }

    #[test]
    fn leased_agent_config_update_ignores_legacy_processing_without_active_prompt() {
        let (mut app, leased_agent) = leased_agent_fixture(false);
        app.agents_mut()
            .set_agent_processing(&leased_agent.backing_agent_id, true)
            .expect("backing agent processing should update");

        let updated = RemoteLeaseRuntime::new(&mut app)
            .update_leased_agent_config(
                &leased_agent.id,
                crate::provider::AgentExecutionMode::Plan,
                crate::provider::AgentPermissionLevel::Required,
            )
            .expect("stale processing alone should not block leased config update");

        assert_eq!(
            updated.execution_mode,
            Some(crate::provider::AgentExecutionMode::Plan)
        );
        assert_eq!(
            updated.permission_level,
            Some(crate::provider::AgentPermissionLevel::Required)
        );
    }

    #[test]
    fn leased_agent_meta_mode_update_ignores_legacy_processing_without_active_prompt() {
        let (mut app, leased_agent) = leased_agent_fixture(false);
        app.agents_mut()
            .set_agent_processing(&leased_agent.backing_agent_id, true)
            .expect("backing agent processing should update");

        let updated = RemoteLeaseRuntime::new(&mut app)
            .update_leased_agent_meta_mode(&leased_agent.id, true)
            .expect("stale processing alone should not block leased meta-mode update");

        assert_eq!(updated.id, leased_agent.id);
        assert!(app
            .agents()
            .get_agent(&leased_agent.backing_agent_id)
            .expect("backing agent should exist")
            .is_metaagent());
    }

    #[test]
    fn leased_agent_config_update_still_blocks_active_prompt_owner() {
        let (mut app, leased_agent) = leased_agent_fixture(false);
        sync_active_prompt(&mut app, &leased_agent);

        let error = RemoteLeaseRuntime::new(&mut app)
            .update_leased_agent_config(
                &leased_agent.id,
                crate::provider::AgentExecutionMode::Plan,
                crate::provider::AgentPermissionLevel::Required,
            )
            .expect_err("active prompt ownership should block leased config update");

        assert_active_turn_error(error, "update leased agent config");
    }

    fn leased_agent_fixture(home_agent_metaagent: bool) -> (DaemonApp, LeasedAgent) {
        leased_agent_fixture_with_config(
            home_agent_metaagent,
            crate::config::DaemonConfig::for_tests(),
        )
    }

    fn leased_agent_fixture_with_config(
        home_agent_metaagent: bool,
        mut config: crate::config::DaemonConfig,
    ) -> (DaemonApp, LeasedAgent) {
        config.accept_remote_leases = true;
        let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
        let lease = RemoteLeaseRuntime::new(&mut app)
            .create_execution_lease(
                "home-kernel",
                "home-session",
                "home-agent",
                home_agent_metaagent,
                crate::session::DEFAULT_LOCAL_USER_ID,
            )
            .expect("execution lease should be created");
        let leased_agent = RemoteLeaseRuntime::new(&mut app)
            .create_leased_agent(
                &lease.id,
                "managed-dev-stub",
                "default",
                Some("sonnet".to_string()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .expect("leased agent should be created");
        (app, leased_agent)
    }

    fn sync_active_prompt(app: &mut DaemonApp, leased_agent: &LeasedAgent) {
        let prompt = crate::session::PromptQueueItem::new(
            "active-prompt",
            &leased_agent.backing_attachment_id,
            &leased_agent.backing_agent_id,
            "active prompt",
            crate::session::PromptStatus::Running,
        );
        app.prompt_owner_sync_external_active_prompt(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
            Some(prompt),
        )
        .expect("active prompt should sync");
    }

    fn assert_active_turn_error(error: DaemonError, operation: &'static str) {
        match error {
            DaemonError::LocalTransport {
                operation: actual,
                message,
            } => {
                assert_eq!(actual, operation);
                assert!(message.contains("has an active turn"));
            }
            other => panic!("expected active turn error, got {other:?}"),
        }
    }
}
