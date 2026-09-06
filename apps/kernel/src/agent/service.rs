use crate::error::DaemonError;
use crate::extension::{ExtensionGrant, ExtensionKind, RemoteExtensionManifestSyncStatus};
use crate::provider::{
    AgentExecutionMode, AgentPermissionLevel, ExternalProviderImportMetadata, ProviderResumeState,
};
use crate::session::{RuntimeSession, SessionService, SessionStatus};

use super::{
    calculate_agent_layout, generate_agent_ref, recalculate_positions, AgentInstance, AgentState,
    AgentStore, AgentSubstituteProfile, CreateAgentRequest, GridPosition, RemoteAgentBinding,
};

#[derive(Debug, Clone)]
pub struct AgentService {
    store: AgentStore,
}

impl AgentService {
    pub fn new() -> Self {
        Self {
            store: AgentStore::new(),
        }
    }

    /// Create a new agent in a session
    pub fn create_agent(
        &mut self,
        request: CreateAgentRequest,
        sessions: &mut SessionService,
    ) -> Result<AgentInstance, DaemonError> {
        self.create_agents_with_operation(vec![request], sessions, "create agent")?
            .into_iter()
            .next()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "create agent",
                message: "single-agent create returned no agent".to_string(),
            })
    }

    pub fn create_agents(
        &mut self,
        requests: Vec<CreateAgentRequest>,
        sessions: &mut SessionService,
    ) -> Result<Vec<AgentInstance>, DaemonError> {
        self.create_agents_with_operation(requests, sessions, "create agents")
    }

    fn create_agents_with_operation(
        &mut self,
        requests: Vec<CreateAgentRequest>,
        sessions: &mut SessionService,
        operation: &'static str,
    ) -> Result<Vec<AgentInstance>, DaemonError> {
        let mut requests = requests;
        for request in &mut requests {
            request.alias = normalize_agent_alias(request.alias.take());
        }
        let Some(first) = requests.first() else {
            return Ok(Vec::new());
        };
        let session_id = first.session_id.clone();
        if requests
            .iter()
            .any(|request| request.session_id.as_str() != session_id)
        {
            return Err(DaemonError::LocalTransport {
                operation,
                message: "batch create requires all agents to target the same session".to_string(),
            });
        }

        let session = sessions.get_session(&session_id)?;
        if session.status() == SessionStatus::Ended {
            return Err(DaemonError::SessionOperationNotAllowed {
                session_id,
                status: session.status(),
                operation,
            });
        }

        let mut session_summary = self.store.session_summary(session.id());
        let current_count = session_summary.count;
        let new_count = requests.len();
        if current_count + new_count > session.max_agents() as usize {
            return Err(DaemonError::AgentLimitReached {
                session_id: session.id().to_string(),
                max_agents: session.max_agents(),
            });
        }

        for request in &requests {
            if request.role == crate::agent::AgentRole::Meta {
                return Err(DaemonError::LocalTransport {
                    operation,
                    message: "creating separate metaagents is deprecated; create a regular agent and send `/meta <task>` to enter meta mode".to_string(),
                });
            }
            if let Some(alias) = request.alias.as_deref() {
                if self.is_alias_taken(session.id(), alias) {
                    return Err(DaemonError::AgentAliasConflict {
                        session_id: session.id().to_string(),
                        alias: alias.to_string(),
                    });
                }
                let normalized = alias.trim().to_lowercase();
                if !session_summary.aliases.insert(normalized) {
                    return Err(DaemonError::AgentAliasConflict {
                        session_id: session.id().to_string(),
                        alias: alias.to_string(),
                    });
                }
            }
        }

        let agent_ids = self.store.next_agent_ids(new_count);
        let mut created_agents = Vec::with_capacity(new_count);
        for (agent_id, request) in agent_ids.into_iter().zip(requests.into_iter()) {
            let agent_ref = generate_agent_ref();
            let mut agent = AgentInstance::new(
                agent_id,
                agent_ref,
                request.session_id,
                request.alias,
                request.provider,
                request.model,
                request.effort,
                request.worktree_id,
                GridPosition::new(0, 0, 1, 1),
            );
            agent.set_owner_user_id(request.owner_user_id);
            agent.set_controlled_by_metaagent_id(request.controlled_by_metaagent_id);
            agent.set_role(request.role);
            agent.set_account_profile(request.account_profile);
            agent.set_execution_mode_override(request.execution_mode_override);
            agent.set_permission_level_override(request.permission_level_override);
            created_agents.push(agent);
        }

        let focused_agent_id = created_agents.last().map(|agent| agent.id().to_string());
        let created_agents = self.store.insert_session_batch_and_apply_layout(
            session.id(),
            created_agents,
            focused_agent_id.as_deref(),
        );
        sessions.set_focused_agent(session.id(), focused_agent_id)?;

        Ok(created_agents)
    }

    pub(crate) fn create_agent_for_session(
        &mut self,
        request: CreateAgentRequest,
        session: &RuntimeSession,
    ) -> Result<AgentInstance, DaemonError> {
        let mut request = request;
        request.alias = normalize_agent_alias(request.alias.take());
        if session.status() == SessionStatus::Ended {
            return Err(DaemonError::SessionOperationNotAllowed {
                session_id: request.session_id.clone(),
                status: session.status(),
                operation: "create agent",
            });
        }

        // Check max agents limit
        let current_count = self.store.count_by_session(&request.session_id);
        if current_count >= session.max_agents() as usize {
            return Err(DaemonError::AgentLimitReached {
                session_id: request.session_id.clone(),
                max_agents: session.max_agents(),
            });
        }

        // Validate alias uniqueness within session
        if let Some(ref alias) = request.alias {
            if self.is_alias_taken(&request.session_id, alias) {
                return Err(DaemonError::AgentAliasConflict {
                    session_id: request.session_id.clone(),
                    alias: alias.clone(),
                });
            }
        }

        // Calculate position for new agent
        let position = self.calculate_position_for_new_agent(&request.session_id);

        // Create agent
        let agent_ref = generate_agent_ref();
        let mut agent = AgentInstance::new(
            self.store.next_agent_id(),
            agent_ref,
            request.session_id,
            request.alias,
            request.provider,
            request.model,
            request.effort,
            request.worktree_id,
            position,
        );
        agent.set_owner_user_id(request.owner_user_id);
        agent.set_controlled_by_metaagent_id(request.controlled_by_metaagent_id);
        if request.role == crate::agent::AgentRole::Meta {
            return Err(DaemonError::LocalTransport {
                operation: "create agent",
                message: "creating separate metaagents is deprecated; create a regular agent and send `/meta <task>` to enter meta mode".to_string(),
            });
        }
        agent.set_role(request.role);
        agent.set_account_profile(request.account_profile);
        agent.set_execution_mode_override(request.execution_mode_override);
        agent.set_permission_level_override(request.permission_level_override);
        let agent_id = agent.id().to_string();
        let session_id = agent.session_id().to_string();

        self.store.insert(agent);

        self.store
            .apply_session_layout_and_focus(&session_id, Some(&agent_id));
        Ok(self
            .store
            .get(&agent_id)
            .cloned()
            .expect("new agent should be stored"))
    }

    pub(crate) fn restore_agent(&mut self, agent: AgentInstance) -> AgentInstance {
        self.store.insert_restored(agent)
    }

    pub(crate) fn materialize_publication_agent(
        &mut self,
        agent: AgentInstance,
        session_id: &str,
        owner_user_id: Option<&str>,
    ) -> AgentInstance {
        let mut agent = agent.materialized_for_publication_runtime(
            self.store.next_agent_id(),
            generate_agent_ref(),
            session_id,
        );
        if let Some(owner_user_id) = owner_user_id {
            agent.set_owner_user_id(owner_user_id);
        }
        self.store.insert(agent)
    }

    pub(crate) fn materialize_workflow_runtime_agent(
        &mut self,
        agent: AgentInstance,
        session_id: &str,
        worktree_id: &str,
    ) -> AgentInstance {
        // A workflow instance copy must never reuse the source agent's visible
        // alias; allocate a deterministic user-facing alias by appending the
        // next available numeric suffix (e.g. `pr-reviewer` -> `pr-reviewer-2`).
        // Runtime ids/refs stay internal and are freshly minted below.
        let copied_alias = agent
            .alias()
            .map(|source_alias| self.next_workflow_copy_alias(session_id, source_alias));
        let mut agent = agent.materialized_for_workflow_runtime(
            self.store.next_agent_id(),
            generate_agent_ref(),
            session_id,
            worktree_id,
        );
        if let Some(copied_alias) = copied_alias {
            agent.set_alias(Some(copied_alias));
        }
        self.store.insert(agent)
    }

    /// Allocate the next collision-free visible alias for a workflow instance
    /// copy of `base_alias` within `session_id`. Copies start at `-2` and skip
    /// any alias already present in the session (including the source alias and
    /// earlier copies).
    fn next_workflow_copy_alias(&self, session_id: &str, base_alias: &str) -> String {
        let taken: std::collections::HashSet<String> = self
            .store
            .get_by_session(session_id)
            .into_iter()
            .filter_map(|agent| agent.alias().map(str::to_string))
            .collect();
        let mut suffix: u64 = 2;
        loop {
            let candidate = format!("{base_alias}-{suffix}");
            if !taken.contains(&candidate) {
                return candidate;
            }
            suffix = suffix.saturating_add(1);
        }
    }

    pub(crate) fn remove_workflow_runtime_agent(
        &mut self,
        agent_id: &str,
    ) -> Option<AgentInstance> {
        let agent = self.store.get(agent_id)?;
        if agent.visible_in_freeform() || agent.is_processing() {
            return None;
        }
        self.store.remove(agent_id)
    }

    pub(crate) fn destroy_workflow_runtime_agent(
        &mut self,
        agent_id: &str,
        sessions: &mut SessionService,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .store
            .get(agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })?;
        if agent.visible_in_freeform() || agent.is_processing() {
            return Err(DaemonError::LocalTransport {
                operation: "cleanup workflow runtime instance",
                message: format!("runtime agent `{agent_id}` remained active"),
            });
        }
        self.destroy_agent(agent_id, sessions)
    }

    /// Create default agent for a new session
    pub fn create_default_agent(
        &mut self,
        session_id: &str,
        worktree_id: &str,
        provider: &str,
        sessions: &mut SessionService,
    ) -> Result<AgentInstance, DaemonError> {
        let request = CreateAgentRequest::new(session_id, provider).with_worktree(worktree_id);

        self.create_agent(request, sessions)
    }

    /// Destroy an agent
    pub fn destroy_agent(
        &mut self,
        agent_id: &str,
        sessions: &mut SessionService,
    ) -> Result<AgentInstance, DaemonError> {
        let agent =
            self.store
                .get(agent_id)
                .cloned()
                .ok_or_else(|| DaemonError::AgentNotFound {
                    agent_id: agent_id.to_string(),
                })?;

        let session_id = agent.session_id().to_string();
        let session_focused_agent_id = sessions
            .get_session(&session_id)?
            .focused_agent_id()
            .map(str::to_string);
        let was_focused = agent.state() == AgentState::Focused
            || session_focused_agent_id.as_deref() == Some(agent_id);

        // Remove the agent
        self.store.remove(agent_id);

        // Recalculate positions for remaining agents
        let mut remaining_agents: Vec<_> =
            self.store.get_by_session(&session_id).into_iter().collect();

        if !remaining_agents.is_empty() {
            recalculate_positions(&mut remaining_agents);

            // Update stored positions
            for agent in &remaining_agents {
                if let Some(stored) = self.store.get_mut(agent.id()) {
                    stored.set_position(agent.position().clone());
                }
            }

            let focus_is_stale_after_destroy =
                session_focused_agent_id
                    .as_deref()
                    .is_some_and(|focused_agent_id| {
                        !remaining_agents
                            .iter()
                            .any(|agent| agent.id() == focused_agent_id)
                    });

            // Runtime workflow copies have no Freeform pane. If canonical focus
            // disappears, prefer a visible agent or leave the session unfocused.
            if was_focused || focus_is_stale_after_destroy {
                let replacement = remaining_agents
                    .iter()
                    .find(|agent| agent.visible_in_freeform())
                    .map(|agent| agent.id().to_string());
                for agent in &remaining_agents {
                    if let Some(stored) = self.store.get_mut(agent.id()) {
                        stored.set_state(
                            stored
                                .state()
                                .with_focus(replacement.as_deref() == Some(stored.id())),
                        );
                    }
                }
                sessions.set_focused_agent(&session_id, replacement)?;
            }
        } else {
            // No agents left, clear focused agent
            sessions.set_focused_agent(&session_id, None)?;
        }

        Ok(agent)
    }

    pub(crate) fn repair_stale_session_focus(
        &mut self,
        session_id: &str,
        sessions: &mut SessionService,
    ) -> Result<bool, DaemonError> {
        let focused_agent_id = sessions
            .get_session(session_id)?
            .focused_agent_id()
            .map(str::to_string);
        let agents = self.store.get_by_session(session_id);
        let visible_agents = agents
            .iter()
            .filter(|agent| agent.visible_in_freeform())
            .collect::<Vec<_>>();
        let focus_is_valid = match focused_agent_id.as_deref() {
            Some(focused_agent_id) => visible_agents
                .iter()
                .any(|agent| agent.id() == focused_agent_id),
            None => visible_agents.is_empty(),
        };
        if focus_is_valid {
            return Ok(false);
        }

        let replacement = visible_agents.first().map(|agent| agent.id().to_string());
        for agent in agents {
            if let Some(stored) = self.store.get_mut(agent.id()) {
                stored.set_state(
                    stored
                        .state()
                        .with_focus(replacement.as_deref() == Some(stored.id())),
                );
            }
        }
        sessions.set_focused_agent(session_id, replacement)?;
        Ok(true)
    }

    /// Focus an agent (tap navigation)
    pub fn focus_agent(
        &mut self,
        session_id: &str,
        agent_id: &str,
        sessions: &mut SessionService,
    ) -> Result<AgentInstance, DaemonError> {
        let agents: Vec<_> = self.store.get_by_session(session_id);

        // Validate agent exists in session
        let _target_agent = agents
            .iter()
            .find(|a| a.id() == agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            })?;

        // Unfocus all other agents in session
        for agent in &agents {
            if agent.id() != agent_id && agent.state() == AgentState::Focused {
                if let Some(stored) = self.store.get_mut(agent.id()) {
                    stored.set_state(AgentState::Idle);
                }
            }
        }

        // Focus target agent
        if let Some(stored) = self.store.get_mut(agent_id) {
            stored.set_state(stored.state().with_focus(true));
        }

        sessions.set_focused_agent(session_id, Some(agent_id.to_string()))?;

        Ok(self.store.get(agent_id).cloned().unwrap())
    }

    /// Get next agent in session (for tap navigation)
    pub fn get_next_agent_in_session(
        &self,
        session_id: &str,
        current_agent_id: &str,
    ) -> Option<AgentInstance> {
        let agents = self.store.get_by_session(session_id);

        if let Some(current_index) = agents.iter().position(|a| a.id() == current_agent_id) {
            let next_index = (current_index + 1) % agents.len();
            agents.get(next_index).cloned()
        } else {
            agents.first().cloned()
        }
    }

    /// Cycle focus to next agent (tap navigation)
    pub fn cycle_focus(
        &mut self,
        session_id: &str,
        sessions: &mut SessionService,
    ) -> Result<Option<AgentInstance>, DaemonError> {
        let agents = self.store.get_by_session(session_id);

        if agents.is_empty() {
            return Ok(None);
        }

        // Canonical focus belongs to the session. Runtime states such as Error
        // and Working must remain visible while that agent is focused.
        let current_focused = sessions
            .get_session(session_id)?
            .focused_agent_id()
            .map(str::to_string);

        let next_agent_id = if let Some(current_id) = current_focused {
            self.get_next_agent_in_session(session_id, &current_id)
                .map(|a| a.id().to_string())
        } else {
            agents.first().map(|a| a.id().to_string())
        };

        if let Some(next_id) = next_agent_id {
            let agent = self.focus_agent(session_id, &next_id, sessions)?;
            Ok(Some(agent))
        } else {
            Ok(None)
        }
    }

    /// Update agent state
    pub fn set_agent_state(
        &mut self,
        agent_id: &str,
        state: AgentState,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .store
            .get_mut(agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })?;

        agent.set_state(state);
        Ok(agent.clone())
    }

    /// Set agent processing state
    pub fn set_agent_processing(
        &mut self,
        agent_id: &str,
        is_processing: bool,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .store
            .get_mut(agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })?;

        agent.set_processing(is_processing);
        Ok(agent.clone())
    }

    pub fn note_prompt_sent_at(
        &mut self,
        agent_id: &str,
        timestamp_ms: u64,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .store
            .get_mut(agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })?;
        agent.note_prompt_sent_at(timestamp_ms);
        Ok(agent.clone())
    }

    pub fn set_agent_runtime_profile(
        &mut self,
        agent_id: &str,
        provider: &str,
        model: Option<String>,
        effort: Option<String>,
        resume_state: ProviderResumeState,
    ) -> Result<AgentInstance, DaemonError> {
        self.set_agent_runtime_profile_with_account_profile(
            agent_id,
            provider,
            model,
            effort,
            None,
            resume_state,
        )
    }

    pub(crate) fn set_agent_provider_resume_state(
        &mut self,
        agent_id: &str,
        resume_state: ProviderResumeState,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .store
            .get_mut(agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })?;
        agent.set_provider_resume_state(resume_state);
        Ok(agent.clone())
    }

    pub fn set_agent_runtime_profile_with_account_profile(
        &mut self,
        agent_id: &str,
        provider: &str,
        model: Option<String>,
        effort: Option<String>,
        account_profile: Option<String>,
        resume_state: ProviderResumeState,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .store
            .get_mut(agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })?;
        agent.set_provider(provider.to_string());
        agent.set_model(model);
        agent.set_effort(effort);
        if account_profile.is_some() {
            agent.set_account_profile(account_profile);
        }
        if agent.active_substitute_index().is_none() {
            agent.set_primary_profile(
                provider.to_string(),
                agent.model().map(str::to_string),
                agent.effort().map(str::to_string),
            );
        }
        agent.set_provider_resume_state(resume_state);
        Ok(agent.clone())
    }

    pub fn set_remote_extension_manifest_sync(
        &mut self,
        agent_id: &str,
        status: Option<RemoteExtensionManifestSyncStatus>,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .store
            .get_mut(agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })?;
        agent.set_remote_extension_manifest_sync(status);
        Ok(agent.clone())
    }

    pub fn set_external_provider_import(
        &mut self,
        agent_id: &str,
        import: Option<ExternalProviderImportMetadata>,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .store
            .get_mut(agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })?;
        agent.set_external_provider_import(import);
        Ok(agent.clone())
    }

    pub fn update_agent_config(
        &mut self,
        agent_id: &str,
        execution_mode_override: Option<Option<AgentExecutionMode>>,
        permission_level_override: Option<Option<AgentPermissionLevel>>,
        workspace_id: Option<Option<String>>,
        worktree_id: Option<Option<String>>,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .store
            .get_mut(agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })?;
        if let Some(execution_mode_override) = execution_mode_override {
            agent.set_execution_mode_override(execution_mode_override);
        }
        if let Some(permission_level_override) = permission_level_override {
            agent.set_permission_level_override(permission_level_override);
        }
        if let Some(workspace_id) = workspace_id {
            agent.set_workspace_id(workspace_id);
        }
        if let Some(worktree_id) = worktree_id {
            agent.set_worktree_id(worktree_id);
        }
        Ok(agent.clone())
    }

    pub fn update_agent_profile(
        &mut self,
        agent_id: &str,
        provider: Option<String>,
        model: Option<String>,
        effort: Option<Option<String>>,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .store
            .get_mut(agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })?;
        if let Some(provider) = provider {
            agent.set_provider(provider);
        }
        if let Some(model) = model {
            agent.set_model(Some(model));
        }
        if let Some(effort) = effort {
            agent.set_effort(effort);
        }
        agent.set_primary_profile(
            agent.provider().to_string(),
            agent.model().map(str::to_string),
            agent.effort().map(str::to_string),
        );
        Ok(agent.clone())
    }

    pub fn set_agent_primary_profile_snapshot(
        &mut self,
        agent_id: &str,
        provider: &str,
        model: Option<String>,
        effort: Option<String>,
        account_profile: Option<String>,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .store
            .get_mut(agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })?;
        agent.set_primary_profile_snapshot(provider, model, effort, account_profile);
        Ok(agent.clone())
    }

    pub fn alias_agent(
        &mut self,
        agent_id: &str,
        alias: Option<String>,
    ) -> Result<AgentInstance, DaemonError> {
        let alias = normalize_agent_alias(alias);
        let session_id = self
            .store
            .get(agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })?
            .session_id()
            .to_string();
        if let Some(alias) = alias.as_deref() {
            if self.is_alias_taken_by_other(&session_id, agent_id, alias) {
                return Err(DaemonError::AgentAliasConflict {
                    session_id,
                    alias: alias.to_string(),
                });
            }
        }
        let agent = self
            .store
            .get_mut(agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })?;
        agent.set_alias(alias);
        Ok(agent.clone())
    }

    pub fn add_agent_substitute(
        &mut self,
        agent_id: &str,
        profile: AgentSubstituteProfile,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .store
            .get_mut(agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })?;
        agent.add_substitute(profile);
        Ok(agent.clone())
    }

    pub fn remove_agent_substitute(
        &mut self,
        agent_id: &str,
        index: usize,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .store
            .get_mut(agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })?;
        if agent.remove_substitute(index).is_none() {
            return Err(DaemonError::LocalTransport {
                operation: "remove agent substitute",
                message: format!("agent `{agent_id}` has no substitute at index {index}"),
            });
        }
        Ok(agent.clone())
    }

    pub fn move_agent_substitute(
        &mut self,
        agent_id: &str,
        from_index: usize,
        to_index: usize,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .store
            .get_mut(agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })?;
        if !agent.move_substitute(from_index, to_index) {
            return Err(DaemonError::LocalTransport {
                operation: "move agent substitute",
                message: format!(
                    "agent `{agent_id}` cannot move substitute {from_index} to {to_index}"
                ),
            });
        }
        Ok(agent.clone())
    }

    pub fn clear_agent_substitutes(
        &mut self,
        agent_id: &str,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .store
            .get_mut(agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })?;
        agent.clear_substitutes();
        Ok(agent.clone())
    }

    pub fn set_agent_substitution_timeout(
        &mut self,
        agent_id: &str,
        timeout_ms: Option<u64>,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .store
            .get_mut(agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })?;
        agent.set_substitution_timeout_ms(timeout_ms);
        Ok(agent.clone())
    }

    pub fn activate_agent_substitute(
        &mut self,
        agent_id: &str,
        index: usize,
        reason: impl Into<String>,
    ) -> Result<(AgentInstance, AgentSubstituteProfile), DaemonError> {
        let agent = self
            .store
            .get_mut(agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })?;
        let profile = agent.activate_substitute(index, reason).ok_or_else(|| {
            DaemonError::LocalTransport {
                operation: "activate agent substitute",
                message: format!("agent `{agent_id}` has no substitute at index {index}"),
            }
        })?;
        Ok((agent.clone(), profile))
    }

    pub fn deactivate_agent_substitute(
        &mut self,
        agent_id: &str,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .store
            .get_mut(agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })?;
        agent.deactivate_substitute();
        Ok(agent.clone())
    }

    pub fn bind_remote_execution(
        &mut self,
        agent_id: &str,
        remote_execution: RemoteAgentBinding,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .store
            .get_mut(agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })?;
        agent.set_remote_execution(Some(remote_execution));
        Ok(agent.clone())
    }

    pub fn set_remote_execution_active_worker_provider_run_id(
        &mut self,
        agent_id: &str,
        provider_run_id: Option<String>,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .store
            .get_mut(agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })?;
        agent.set_remote_execution_active_worker_provider_run_id(provider_run_id);
        Ok(agent.clone())
    }

    pub fn clear_remote_execution(&mut self, agent_id: &str) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .store
            .get_mut(agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })?;
        agent.set_remote_execution(None);
        Ok(agent.clone())
    }

    /// Get agent by ID
    pub fn get_agent(&self, agent_id: &str) -> Result<AgentInstance, DaemonError> {
        self.store
            .get(agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })
    }

    /// Get agent by reference
    pub fn get_agent_by_ref(&self, agent_ref: &str) -> Result<AgentInstance, DaemonError> {
        self.store
            .get_by_ref(agent_ref)
            .cloned()
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_ref.to_string(),
            })
    }

    /// Get all agents in a session
    pub fn get_session_agents(&self, session_id: &str) -> Vec<AgentInstance> {
        self.store.get_by_session(session_id)
    }

    #[cfg(test)]
    pub(crate) fn set_controlled_by_metaagent_id(
        &mut self,
        agent_id: &str,
        metaagent_id: Option<String>,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .store
            .get_mut(agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })?;
        agent.set_controlled_by_metaagent_id(metaagent_id);
        Ok(agent.clone())
    }

    pub(crate) fn activate_agent_meta_mode(
        &mut self,
        agent_id: &str,
        task_id: Option<String>,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .store
            .get_mut(agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })?;
        agent.activate_meta_mode(task_id);
        Ok(agent.clone())
    }

    pub(crate) fn deactivate_agent_meta_mode(
        &mut self,
        agent_id: &str,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .store
            .get_mut(agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })?;
        agent.deactivate_meta_mode();
        Ok(agent.clone())
    }

    pub fn list_agents(&self) -> Vec<AgentInstance> {
        self.store.list()
    }

    /// Get focused agent in session
    pub fn get_focused_agent(&self, session_id: &str) -> Option<AgentInstance> {
        self.store.focused_agent(session_id).cloned()
    }

    pub fn grant_mcp(
        &mut self,
        agent_ref: &str,
        name: String,
    ) -> Result<AgentInstance, DaemonError> {
        self.grant_extension(agent_ref, ExtensionGrant::new(ExtensionKind::Mcp, name))
    }

    pub fn grant_extension(
        &mut self,
        agent_ref: &str,
        grant: ExtensionGrant,
    ) -> Result<AgentInstance, DaemonError> {
        let agent_id = self.resolve_agent_id(agent_ref)?;
        let agent = self
            .store
            .get_mut(&agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_ref.to_string(),
            })?;
        agent.grant_extension(grant);
        Ok(agent.clone())
    }

    pub fn revoke_mcp(
        &mut self,
        agent_ref: &str,
        name: &str,
    ) -> Result<AgentInstance, DaemonError> {
        let agent_id = self.resolve_agent_id(agent_ref)?;
        let agent = self
            .store
            .get_mut(&agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_ref.to_string(),
            })?;
        agent.revoke_extension(ExtensionKind::Mcp, name);
        Ok(agent.clone())
    }

    pub fn grant_skill(
        &mut self,
        agent_ref: &str,
        name: String,
    ) -> Result<AgentInstance, DaemonError> {
        self.grant_extension(agent_ref, ExtensionGrant::new(ExtensionKind::Skill, name))
    }

    pub fn revoke_skill(
        &mut self,
        agent_ref: &str,
        name: &str,
    ) -> Result<AgentInstance, DaemonError> {
        let agent_id = self.resolve_agent_id(agent_ref)?;
        let agent = self
            .store
            .get_mut(&agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_ref.to_string(),
            })?;
        agent.revoke_extension(ExtensionKind::Skill, name);
        Ok(agent.clone())
    }

    pub fn revoke_extension(
        &mut self,
        agent_ref: &str,
        kind: ExtensionKind,
        name: &str,
    ) -> Result<AgentInstance, DaemonError> {
        let agent_id = self.resolve_agent_id(agent_ref)?;
        let agent = self
            .store
            .get_mut(&agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_ref.to_string(),
            })?;
        agent.revoke_extension(kind, name);
        Ok(agent.clone())
    }

    fn resolve_agent_id(&self, agent_ref: &str) -> Result<String, DaemonError> {
        if let Some(agent) = self.store.get(agent_ref) {
            return Ok(agent.id().to_string());
        }
        self.store
            .get_by_ref(agent_ref)
            .map(|agent| agent.id().to_string())
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_ref.to_string(),
            })
    }

    /// Remove all agents for a session (called when session ends)
    pub fn remove_session_agents(&mut self, session_id: &str) -> Vec<AgentInstance> {
        self.store.remove_by_session(session_id)
    }

    fn calculate_position_for_new_agent(&self, session_id: &str) -> GridPosition {
        let current_count = self.store.count_by_session(session_id);
        let positions = calculate_agent_layout(current_count + 1);

        positions
            .get(current_count)
            .cloned()
            .unwrap_or_else(|| GridPosition::new(0, 0, 1, 1))
    }

    fn is_alias_taken(&self, session_id: &str, alias: &str) -> bool {
        self.is_alias_taken_by_other(session_id, "", alias)
    }

    fn is_alias_taken_by_other(&self, session_id: &str, agent_id: &str, alias: &str) -> bool {
        let normalized = normalized_agent_alias_key(alias);
        self.store.get_by_session(session_id).iter().any(|agent| {
            agent.id() != agent_id
                && (normalized_agent_alias_key(agent.id()) == normalized
                    || normalized_agent_alias_key(agent.agent_ref()) == normalized
                    || agent
                        .alias()
                        .map(normalized_agent_alias_key)
                        .is_some_and(|candidate| candidate == normalized))
        })
    }

    pub fn store(&self) -> &AgentStore {
        &self.store
    }
}

fn normalize_agent_alias(alias: Option<String>) -> Option<String> {
    alias.and_then(|alias| {
        let alias = alias.trim();
        (!alias.is_empty()).then(|| alias.to_string())
    })
}

fn normalized_agent_alias_key(alias: &str) -> String {
    alias.trim().to_lowercase()
}

impl Default for AgentService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod workflow_copy_alias_tests {
    use super::*;
    use crate::config::DaemonConfig;
    use crate::provider::{AgentExecutionMode, AgentPermissionLevel};
    use crate::session::CreateSessionRequest;

    fn insert_source(service: &mut AgentService, session_id: &str, alias: &str) -> AgentInstance {
        let id = service.store.next_agent_id();
        let mut agent = AgentInstance::new(
            id,
            generate_agent_ref(),
            session_id,
            Some(alias.to_string()),
            "codex",
            Some("gpt-5-codex".to_string()),
            Some("high".to_string()),
            Some("wt-source".to_string()),
            GridPosition::new(0, 0, 1, 1),
        );
        agent.set_execution_mode_override(Some(AgentExecutionMode::Build));
        agent.set_permission_level_override(Some(AgentPermissionLevel::Yolo));
        agent.set_account_profile(Some("acct-primary".to_string()));
        agent.grant_mcp("home_browser");
        agent.grant_skill("dataviz");
        service.store.insert(agent)
    }

    #[test]
    fn sequential_copies_receive_incrementing_numeric_suffixes() {
        let mut service = AgentService::new();
        let source = insert_source(&mut service, "s1", "pr-reviewer");

        let copy_a = service.materialize_workflow_runtime_agent(source.clone(), "s1", "wt-a");
        let copy_b = service.materialize_workflow_runtime_agent(source.clone(), "s1", "wt-b");

        assert_eq!(copy_a.alias(), Some("pr-reviewer-2"));
        assert_eq!(copy_b.alias(), Some("pr-reviewer-3"));
        // Runtime ids/refs stay internal and are freshly minted per copy.
        assert_ne!(copy_a.id(), source.id());
        assert_ne!(copy_a.agent_ref(), source.agent_ref());
        assert_ne!(copy_a.id(), copy_b.id());
        assert!(!copy_a.visible_in_freeform());
    }

    #[test]
    fn copies_skip_pre_existing_alias_collisions() {
        let mut service = AgentService::new();
        let source = insert_source(&mut service, "s2", "pr-reviewer");
        // A pre-existing agent already occupies the first suffixed alias.
        let _collision = insert_source(&mut service, "s2", "pr-reviewer-2");

        let copy = service.materialize_workflow_runtime_agent(source, "s2", "wt");

        assert_eq!(copy.alias(), Some("pr-reviewer-3"));
    }

    #[test]
    fn multiple_agents_get_independent_suffix_sequences() {
        let mut service = AgentService::new();
        let reviewer = insert_source(&mut service, "s3", "pr-reviewer");
        let tester = insert_source(&mut service, "s3", "tester");

        let reviewer_copy = service.materialize_workflow_runtime_agent(reviewer, "s3", "wt");
        let tester_copy = service.materialize_workflow_runtime_agent(tester, "s3", "wt");

        assert_eq!(reviewer_copy.alias(), Some("pr-reviewer-2"));
        assert_eq!(tester_copy.alias(), Some("tester-2"));
    }

    #[test]
    fn copy_preserves_provider_model_effort_permissions_and_extensions() {
        let mut service = AgentService::new();
        let source = insert_source(&mut service, "s4", "pr-reviewer");

        let copy = service.materialize_workflow_runtime_agent(source.clone(), "s4", "wt");

        assert_eq!(copy.provider(), source.provider());
        assert_eq!(copy.model(), source.model());
        assert_eq!(copy.effort(), source.effort());
        assert_eq!(
            copy.execution_mode_override(),
            source.execution_mode_override()
        );
        assert_eq!(
            copy.permission_level_override(),
            source.permission_level_override()
        );
        assert_eq!(copy.account_profile(), source.account_profile());
        assert_eq!(copy.extension_grants(), source.extension_grants());
        // The copy is pinned to its own instance worktree, not the source's.
        assert_eq!(copy.worktree_id(), Some("wt"));
        assert_ne!(copy.worktree_id(), source.worktree_id());
        // The visible alias is the only identity that changes.
        assert_ne!(copy.alias(), source.alias());
    }

    #[test]
    fn durable_reconstruction_continues_the_suffix_sequence() {
        // Simulate a restart where an earlier copy (pr-reviewer-2) has been
        // restored into the session store from durable state. A fresh copy must
        // continue from the next available suffix rather than reuse it.
        let mut service = AgentService::new();
        let source = insert_source(&mut service, "s5", "pr-reviewer");

        let restored = service.materialize_workflow_runtime_agent(source.clone(), "s5", "wt");
        assert_eq!(restored.alias(), Some("pr-reviewer-2"));

        let next = service.materialize_workflow_runtime_agent(source, "s5", "wt");
        assert_eq!(next.alias(), Some("pr-reviewer-3"));
    }

    #[test]
    fn agents_without_a_source_alias_stay_unaliased() {
        let mut service = AgentService::new();
        let id = service.store.next_agent_id();
        let source = AgentInstance::new(
            id,
            generate_agent_ref(),
            "s6",
            None,
            "codex",
            None,
            None,
            Some("wt".to_string()),
            GridPosition::new(0, 0, 1, 1),
        );
        let source = service.store.insert(source);

        let copy = service.materialize_workflow_runtime_agent(source, "s6", "wt");

        assert_eq!(copy.alias(), None);
    }

    #[test]
    fn destroying_a_focused_runtime_copy_repairs_canonical_session_focus() {
        let mut service = AgentService::new();
        let mut sessions = SessionService::new(&DaemonConfig::for_tests());
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let source = insert_source(&mut service, session.id(), "reviewer");
        let copy = service.materialize_workflow_runtime_agent(
            source.clone(),
            session.id(),
            "runtime-worktree",
        );
        service
            .focus_agent(session.id(), copy.id(), &mut sessions)
            .expect("runtime copy should focus");

        service
            .destroy_workflow_runtime_agent(copy.id(), &mut sessions)
            .expect("idle runtime copy should be destroyed");

        assert_eq!(
            sessions
                .get_session(session.id())
                .expect("session should remain")
                .focused_agent_id(),
            Some(source.id())
        );
        assert_eq!(
            service
                .get_focused_agent(session.id())
                .expect("remaining source should focus")
                .id(),
            source.id()
        );
    }

    #[test]
    fn stale_restored_focus_repairs_to_a_visible_agent() {
        let mut service = AgentService::new();
        let mut sessions = SessionService::new(&DaemonConfig::for_tests());
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let source = insert_source(&mut service, session.id(), "reviewer");
        let copy = service.materialize_workflow_runtime_agent(
            source.clone(),
            session.id(),
            "runtime-worktree",
        );
        sessions
            .set_focused_agent(session.id(), Some(copy.id().to_string()))
            .expect("hidden runtime focus should be stored");

        assert!(service
            .repair_stale_session_focus(session.id(), &mut sessions)
            .expect("stale focus should repair"));
        assert_eq!(
            sessions
                .get_session(session.id())
                .expect("session should remain")
                .focused_agent_id(),
            Some(source.id())
        );
        assert_eq!(
            service
                .get_agent(source.id())
                .expect("source should remain")
                .state(),
            AgentState::Focused
        );
        assert_ne!(
            service
                .get_agent(copy.id())
                .expect("copy should remain")
                .state(),
            AgentState::Focused
        );
        assert!(!service
            .repair_stale_session_focus(session.id(), &mut sessions)
            .expect("valid focus should stay unchanged"));
    }

    #[test]
    fn destroying_a_focused_runtime_copy_skips_remaining_hidden_copies() {
        let mut service = AgentService::new();
        let mut sessions = SessionService::new(&DaemonConfig::for_tests());
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let first_hidden = insert_source(&mut service, session.id(), "legacy-runtime");
        service
            .store
            .get_mut(first_hidden.id())
            .expect("first runtime agent should exist")
            .set_visible_in_freeform(false);
        let second_hidden = service.materialize_workflow_runtime_agent(
            first_hidden.clone(),
            session.id(),
            "runtime-worktree-2",
        );
        let visible = insert_source(&mut service, session.id(), "reviewer");
        service
            .focus_agent(session.id(), first_hidden.id(), &mut sessions)
            .expect("runtime agent should focus for the legacy-state fixture");

        service
            .destroy_workflow_runtime_agent(first_hidden.id(), &mut sessions)
            .expect("focused runtime copy should be destroyed");

        assert_eq!(
            sessions
                .get_session(session.id())
                .expect("session should remain")
                .focused_agent_id(),
            Some(visible.id())
        );
        assert_eq!(
            service
                .get_agent(visible.id())
                .expect("visible agent should remain")
                .state(),
            AgentState::Focused
        );
        assert_ne!(
            service
                .get_agent(second_hidden.id())
                .expect("hidden copy should remain")
                .state(),
            AgentState::Focused
        );
    }

    #[test]
    fn repair_clears_focus_when_only_hidden_runtime_agents_remain() {
        let mut service = AgentService::new();
        let mut sessions = SessionService::new(&DaemonConfig::for_tests());
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let hidden = insert_source(&mut service, session.id(), "legacy-runtime");
        service
            .store
            .get_mut(hidden.id())
            .expect("runtime agent should exist")
            .set_visible_in_freeform(false);
        service
            .focus_agent(session.id(), hidden.id(), &mut sessions)
            .expect("legacy hidden focus should be stored");

        assert!(service
            .repair_stale_session_focus(session.id(), &mut sessions)
            .expect("hidden-only focus should repair"));
        assert_eq!(
            sessions
                .get_session(session.id())
                .expect("session should remain")
                .focused_agent_id(),
            None
        );
        assert_ne!(
            service
                .get_agent(hidden.id())
                .expect("hidden agent should remain")
                .state(),
            AgentState::Focused
        );
    }
}
