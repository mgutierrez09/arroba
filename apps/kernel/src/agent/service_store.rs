use std::sync::{Arc, Mutex, MutexGuard};

use crate::durable_state::DurableKernelStateStore;
use crate::error::DaemonError;
use crate::extension::{ExtensionGrant, ExtensionKind, RemoteExtensionManifestSyncStatus};
use crate::provider::{
    AgentExecutionMode, AgentPermissionLevel, ExternalProviderImportMetadata, ProviderResumeState,
};
use crate::session::{RuntimeSession, SessionService};

use super::{
    AgentInstance, AgentService, AgentState, AgentSubstituteProfile, CreateAgentRequest,
    RemoteAgentBinding,
};

#[derive(Debug, Clone)]
pub struct AgentServiceStore {
    inner: Arc<Mutex<AgentService>>,
}

#[derive(Debug)]
pub(crate) enum ProviderResumeClearOutcome {
    Cleared,
    AlreadyAbsent,
    Superseded { current_provider_session_id: String },
}

impl AgentServiceStore {
    pub fn new(service: AgentService) -> Self {
        Self {
            inner: Arc::new(Mutex::new(service)),
        }
    }

    pub fn read(&self) -> MutexGuard<'_, AgentService> {
        self.inner.lock().expect("agent service mutex poisoned")
    }

    pub fn write(&self) -> MutexGuard<'_, AgentService> {
        self.inner.lock().expect("agent service mutex poisoned")
    }

    pub fn create_agent(
        &self,
        request: CreateAgentRequest,
        sessions: &mut SessionService,
    ) -> Result<AgentInstance, DaemonError> {
        self.write().create_agent(request, sessions)
    }

    pub fn create_agents(
        &self,
        requests: Vec<CreateAgentRequest>,
        sessions: &mut SessionService,
    ) -> Result<Vec<AgentInstance>, DaemonError> {
        self.write().create_agents(requests, sessions)
    }

    pub(crate) fn create_agent_for_session(
        &self,
        request: CreateAgentRequest,
        session: &RuntimeSession,
    ) -> Result<AgentInstance, DaemonError> {
        self.write().create_agent_for_session(request, session)
    }

    pub(crate) fn restore_agent(&self, agent: AgentInstance) -> AgentInstance {
        self.write().restore_agent(agent)
    }

    pub(crate) fn materialize_publication_agent(
        &self,
        agent: AgentInstance,
        session_id: &str,
        owner_user_id: Option<&str>,
    ) -> AgentInstance {
        self.write()
            .materialize_publication_agent(agent, session_id, owner_user_id)
    }

    pub(crate) fn materialize_workflow_runtime_agent(
        &self,
        agent: AgentInstance,
        session_id: &str,
        worktree_id: &str,
    ) -> AgentInstance {
        self.write()
            .materialize_workflow_runtime_agent(agent, session_id, worktree_id)
    }

    pub(crate) fn remove_workflow_runtime_agent(&self, agent_id: &str) -> Option<AgentInstance> {
        self.write().remove_workflow_runtime_agent(agent_id)
    }

    pub(crate) fn destroy_workflow_runtime_agent(
        &self,
        agent_id: &str,
        sessions: &mut SessionService,
    ) -> Result<AgentInstance, DaemonError> {
        self.write()
            .destroy_workflow_runtime_agent(agent_id, sessions)
    }

    pub fn create_default_agent(
        &self,
        session_id: &str,
        worktree_id: &str,
        provider: &str,
        sessions: &mut SessionService,
    ) -> Result<AgentInstance, DaemonError> {
        self.write()
            .create_default_agent(session_id, worktree_id, provider, sessions)
    }

    pub fn destroy_agent(
        &self,
        agent_id: &str,
        sessions: &mut SessionService,
    ) -> Result<AgentInstance, DaemonError> {
        self.write().destroy_agent(agent_id, sessions)
    }

    pub fn focus_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        sessions: &mut SessionService,
    ) -> Result<AgentInstance, DaemonError> {
        self.write().focus_agent(session_id, agent_id, sessions)
    }

    pub(crate) fn repair_stale_session_focus(
        &self,
        session_id: &str,
        sessions: &mut SessionService,
    ) -> Result<bool, DaemonError> {
        self.write()
            .repair_stale_session_focus(session_id, sessions)
    }

    pub fn cycle_focus(
        &self,
        session_id: &str,
        sessions: &mut SessionService,
    ) -> Result<Option<AgentInstance>, DaemonError> {
        self.write().cycle_focus(session_id, sessions)
    }

    pub fn set_agent_state(
        &self,
        agent_id: &str,
        state: AgentState,
    ) -> Result<AgentInstance, DaemonError> {
        self.write().set_agent_state(agent_id, state)
    }

    pub(crate) fn mark_unexpected_provider_exit_error(
        &self,
        agent_id: &str,
        had_active_prompt: bool,
    ) -> Result<bool, DaemonError> {
        if !had_active_prompt || self.get_agent(agent_id)?.state() == AgentState::Error {
            return Ok(false);
        }
        self.set_agent_state(agent_id, AgentState::Error)?;
        Ok(true)
    }

    pub(crate) fn clear_local_prompt_error(&self, agent_id: &str) -> Result<bool, DaemonError> {
        if self.get_agent(agent_id)?.state() != AgentState::Error {
            return Ok(false);
        }
        self.set_agent_state(agent_id, AgentState::Idle)?;
        Ok(true)
    }

    pub fn set_agent_processing(
        &self,
        agent_id: &str,
        is_processing: bool,
    ) -> Result<AgentInstance, DaemonError> {
        self.write().set_agent_processing(agent_id, is_processing)
    }

    pub fn note_prompt_sent_at(
        &self,
        agent_id: &str,
        timestamp_ms: u64,
    ) -> Result<AgentInstance, DaemonError> {
        self.write().note_prompt_sent_at(agent_id, timestamp_ms)
    }

    pub fn set_agent_runtime_profile(
        &self,
        agent_id: &str,
        provider: &str,
        model: Option<String>,
        effort: Option<String>,
        resume_state: ProviderResumeState,
    ) -> Result<AgentInstance, DaemonError> {
        self.write()
            .set_agent_runtime_profile(agent_id, provider, model, effort, resume_state)
    }

    pub fn set_agent_runtime_profile_with_account_profile(
        &self,
        agent_id: &str,
        provider: &str,
        model: Option<String>,
        effort: Option<String>,
        account_profile: Option<String>,
        resume_state: ProviderResumeState,
    ) -> Result<AgentInstance, DaemonError> {
        self.write().set_agent_runtime_profile_with_account_profile(
            agent_id,
            provider,
            model,
            effort,
            account_profile,
            resume_state,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn set_agent_runtime_profile_durably(
        &self,
        durable_state_store: &DurableKernelStateStore,
        agent_id: &str,
        provider: &str,
        model: Option<String>,
        effort: Option<String>,
        account_profile: Option<String>,
        resume_state: ProviderResumeState,
        provider_run_id: Option<&str>,
        reason: Option<&str>,
    ) -> Result<AgentInstance, DaemonError> {
        let mut agents = self.write();
        let previous = agents.get_agent(agent_id)?;
        let updated = agents.set_agent_runtime_profile_with_account_profile(
            agent_id,
            provider,
            model,
            effort,
            account_profile,
            resume_state,
        )?;
        let mut payload = serde_json::json!({ "agent": &updated });
        if let Some(provider_run_id) = provider_run_id {
            payload["provider_run_id"] = serde_json::Value::String(provider_run_id.to_string());
        }
        if let Some(reason) = reason {
            payload["reason"] = serde_json::Value::String(reason.to_string());
        }
        if let Err(error) = durable_state_store.append_event(
            "agent.runtime_profile_updated",
            Some(updated.id().to_string()),
            payload,
        ) {
            agents.restore_agent(previous);
            return Err(error);
        }
        Ok(updated)
    }

    pub(crate) fn clear_provider_resume_state_durably_if_matches(
        &self,
        durable_state_store: &DurableKernelStateStore,
        agent_id: &str,
        provider: &str,
        expected_provider_session_id: &str,
        provider_run_id: &str,
        reason: &str,
    ) -> Result<ProviderResumeClearOutcome, DaemonError> {
        let mut agents = self.write();
        let previous = agents.get_agent(agent_id)?;
        let current_provider_session_id = previous
            .provider_resume_state()
            .provider_session_id(provider)
            .map(str::to_string);
        match current_provider_session_id.as_deref() {
            None => return Ok(ProviderResumeClearOutcome::AlreadyAbsent),
            Some(current) if current != expected_provider_session_id => {
                return Ok(ProviderResumeClearOutcome::Superseded {
                    current_provider_session_id: current.to_string(),
                });
            }
            Some(_) => {}
        }
        let replacement_resume_state = previous
            .provider_resume_state()
            .without_provider_session_id(provider);
        let updated = agents.set_agent_provider_resume_state(agent_id, replacement_resume_state)?;
        if let Err(error) = durable_state_store.append_event(
            "agent.runtime_profile_updated",
            Some(updated.id().to_string()),
            serde_json::json!({
                "agent": &updated,
                "provider_run_id": provider_run_id,
                "reason": reason,
            }),
        ) {
            agents.restore_agent(previous);
            return Err(error);
        }
        Ok(ProviderResumeClearOutcome::Cleared)
    }

    pub fn set_remote_extension_manifest_sync(
        &self,
        agent_id: &str,
        status: Option<RemoteExtensionManifestSyncStatus>,
    ) -> Result<AgentInstance, DaemonError> {
        self.write()
            .set_remote_extension_manifest_sync(agent_id, status)
    }

    pub fn set_external_provider_import(
        &self,
        agent_id: &str,
        import: Option<ExternalProviderImportMetadata>,
    ) -> Result<AgentInstance, DaemonError> {
        self.write().set_external_provider_import(agent_id, import)
    }

    pub fn update_agent_config(
        &self,
        agent_id: &str,
        execution_mode_override: Option<Option<AgentExecutionMode>>,
        permission_level_override: Option<Option<AgentPermissionLevel>>,
        workspace_id: Option<Option<String>>,
        worktree_id: Option<Option<String>>,
    ) -> Result<AgentInstance, DaemonError> {
        self.write().update_agent_config(
            agent_id,
            execution_mode_override,
            permission_level_override,
            workspace_id,
            worktree_id,
        )
    }

    pub fn update_agent_profile(
        &self,
        agent_id: &str,
        provider: Option<String>,
        model: Option<String>,
        effort: Option<Option<String>>,
    ) -> Result<AgentInstance, DaemonError> {
        self.write()
            .update_agent_profile(agent_id, provider, model, effort)
    }

    pub fn alias_agent(
        &self,
        agent_id: &str,
        alias: Option<String>,
    ) -> Result<AgentInstance, DaemonError> {
        self.write().alias_agent(agent_id, alias)
    }

    pub fn add_agent_substitute(
        &self,
        agent_id: &str,
        profile: AgentSubstituteProfile,
    ) -> Result<AgentInstance, DaemonError> {
        self.write().add_agent_substitute(agent_id, profile)
    }

    pub fn remove_agent_substitute(
        &self,
        agent_id: &str,
        index: usize,
    ) -> Result<AgentInstance, DaemonError> {
        self.write().remove_agent_substitute(agent_id, index)
    }

    pub fn move_agent_substitute(
        &self,
        agent_id: &str,
        from_index: usize,
        to_index: usize,
    ) -> Result<AgentInstance, DaemonError> {
        self.write()
            .move_agent_substitute(agent_id, from_index, to_index)
    }

    pub fn clear_agent_substitutes(&self, agent_id: &str) -> Result<AgentInstance, DaemonError> {
        self.write().clear_agent_substitutes(agent_id)
    }

    pub fn set_agent_primary_profile_snapshot(
        &self,
        agent_id: &str,
        provider: &str,
        model: Option<String>,
        effort: Option<String>,
        account_profile: Option<String>,
    ) -> Result<AgentInstance, DaemonError> {
        self.write().set_agent_primary_profile_snapshot(
            agent_id,
            provider,
            model,
            effort,
            account_profile,
        )
    }

    pub fn set_agent_substitution_timeout(
        &self,
        agent_id: &str,
        timeout_ms: Option<u64>,
    ) -> Result<AgentInstance, DaemonError> {
        self.write()
            .set_agent_substitution_timeout(agent_id, timeout_ms)
    }

    pub fn activate_agent_substitute(
        &self,
        agent_id: &str,
        index: usize,
        reason: impl Into<String>,
    ) -> Result<(AgentInstance, AgentSubstituteProfile), DaemonError> {
        self.write()
            .activate_agent_substitute(agent_id, index, reason)
    }

    pub fn deactivate_agent_substitute(
        &self,
        agent_id: &str,
    ) -> Result<AgentInstance, DaemonError> {
        self.write().deactivate_agent_substitute(agent_id)
    }

    pub fn bind_remote_execution(
        &self,
        agent_id: &str,
        remote_execution: RemoteAgentBinding,
    ) -> Result<AgentInstance, DaemonError> {
        self.write()
            .bind_remote_execution(agent_id, remote_execution)
    }

    pub fn set_remote_execution_active_worker_provider_run_id(
        &self,
        agent_id: &str,
        provider_run_id: Option<String>,
    ) -> Result<AgentInstance, DaemonError> {
        self.write()
            .set_remote_execution_active_worker_provider_run_id(agent_id, provider_run_id)
    }

    pub fn clear_remote_execution(&self, agent_id: &str) -> Result<AgentInstance, DaemonError> {
        self.write().clear_remote_execution(agent_id)
    }

    pub fn get_agent(&self, agent_id: &str) -> Result<AgentInstance, DaemonError> {
        self.read().get_agent(agent_id)
    }

    pub(crate) fn activate_agent_meta_mode(
        &self,
        agent_id: &str,
        task_id: Option<String>,
    ) -> Result<AgentInstance, DaemonError> {
        self.write().activate_agent_meta_mode(agent_id, task_id)
    }

    pub(crate) fn deactivate_agent_meta_mode(
        &self,
        agent_id: &str,
    ) -> Result<AgentInstance, DaemonError> {
        self.write().deactivate_agent_meta_mode(agent_id)
    }

    pub fn get_agent_by_ref(&self, agent_ref: &str) -> Result<AgentInstance, DaemonError> {
        self.read().get_agent_by_ref(agent_ref)
    }

    pub fn get_session_agents(&self, session_id: &str) -> Vec<AgentInstance> {
        self.read().get_session_agents(session_id)
    }

    pub fn list_agents(&self) -> Vec<AgentInstance> {
        self.read().list_agents()
    }

    pub fn get_focused_agent(&self, session_id: &str) -> Option<AgentInstance> {
        self.read().get_focused_agent(session_id)
    }

    pub fn grant_mcp(&self, agent_ref: &str, name: String) -> Result<AgentInstance, DaemonError> {
        self.write().grant_mcp(agent_ref, name)
    }

    pub fn grant_extension(
        &self,
        agent_ref: &str,
        grant: ExtensionGrant,
    ) -> Result<AgentInstance, DaemonError> {
        self.write().grant_extension(agent_ref, grant)
    }

    pub fn revoke_mcp(&self, agent_ref: &str, name: &str) -> Result<AgentInstance, DaemonError> {
        self.write().revoke_mcp(agent_ref, name)
    }

    pub fn grant_skill(&self, agent_ref: &str, name: String) -> Result<AgentInstance, DaemonError> {
        self.write().grant_skill(agent_ref, name)
    }

    pub fn revoke_skill(&self, agent_ref: &str, name: &str) -> Result<AgentInstance, DaemonError> {
        self.write().revoke_skill(agent_ref, name)
    }

    pub fn revoke_extension(
        &self,
        agent_ref: &str,
        kind: ExtensionKind,
        name: &str,
    ) -> Result<AgentInstance, DaemonError> {
        self.write().revoke_extension(agent_ref, kind, name)
    }

    pub fn remove_session_agents(&self, session_id: &str) -> Vec<AgentInstance> {
        self.write().remove_session_agents(session_id)
    }
}
