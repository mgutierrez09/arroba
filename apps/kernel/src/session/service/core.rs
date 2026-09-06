use super::*;
use crate::session::{WorkflowEventDeliveryReceipt, WorkflowPublicationSnapshot};
use std::path::Path;

mod event_publication;
mod publication;

impl SessionService {
    pub fn new(config: &DaemonConfig) -> Self {
        #[cfg(test)]
        let prompt_id_allocator = PromptIdAllocator::default();
        #[cfg(not(test))]
        let prompt_id_allocator =
            PromptIdAllocator::persistent(config.kernel_prompt_counter_path());
        Self {
            store: SessionStore::new(),
            room_environments: RoomEnvironmentRegistry::new(),
            projects: BTreeMap::new(),
            ephemeral_session_ids: BTreeSet::new(),
            host_machine_id: config.host_machine_id.clone(),
            host_daemon_id: config.daemon_id.clone(),
            event_environment_id: config.event_delivery_environment_id.clone(),
            prompt_id_allocator,
            next_workflow_number: 0,
            next_workflow_schema_number: 0,
            next_workflow_endpoint_number: 0,
            next_workflow_node_number: 0,
            next_workflow_edge_number: 0,
            next_workflow_node_run_number: 0,
            next_workflow_message_number: 0,
            next_workflow_watchdog_number: 0,
            next_workflow_publication_number: 0,
            next_workflow_event_binding_number: 0,
            next_workflow_prompt_queue_number: 0,
            next_workflow_queued_prompt_number: 0,
            next_agent_prompt_schedule_number: 0,
            max_workflow_queues_per_workflow: config.max_workflow_queues_per_workflow(),
            session_default_max_agents: config.session_default_max_agents(),
            workflow_default_max_concurrent: config.workflow_code_limits().max_concurrent.max(1),
            next_workspace_link_number: 0,
        }
    }

    pub fn create_session(
        &mut self,
        request: CreateSessionRequest,
    ) -> Result<RuntimeSession, DaemonError> {
        if request.metaagent {
            return Err(DaemonError::LocalTransport {
                operation: "create session",
                message:
                    "creating separate metaagent sessions is deprecated; create a regular session and send `/meta <task>` to enter meta mode"
                        .to_string(),
            });
        }
        let alias = match request.alias.as_ref() {
            Some(alias) if !alias.trim().is_empty() => {
                normalize_session_alias(Some(alias.clone()))?
            }
            _ if !request.hidden => Some(self.default_session_alias(&request.workspace_id)),
            _ => None,
        };
        if let Some(alias) = alias.as_deref() {
            self.ensure_alias_available(&request.workspace_id, alias)?;
        }
        let project_id = if request.hidden {
            None
        } else {
            Some(self.resolve_project_selection(&request)?)
        };
        let mut session = RuntimeSession::new(
            self.store.next_session_id(),
            alias,
            request.workspace_id,
            request.worktree_id,
            self.host_machine_id.clone(),
            self.host_daemon_id.clone(),
        );
        if let Some(project_id) = project_id {
            let assigned = session.assign_project_id(project_id);
            debug_assert!(assigned);
        }
        session.set_max_agents(self.session_default_max_agents);
        session.set_owner_user_id(request.owner_user_id);
        session.set_hidden(request.hidden);
        if let Some(agent_defaults) = request.agent_defaults {
            session.set_agent_defaults(agent_defaults);
        }
        if let Some(mode) = request.workspace_live_sync_mode {
            session.set_workspace_live_sync_mode(Some(mode));
        }

        Ok(self.store.insert(session))
    }

    pub(crate) fn create_ephemeral_session(
        &mut self,
        request: CreateSessionRequest,
    ) -> Result<RuntimeSession, DaemonError> {
        let session = self.create_session(request)?;
        self.ephemeral_session_ids.insert(session.id().to_string());
        Ok(session)
    }

    pub(crate) fn is_ephemeral_session(&self, session_id: &str) -> bool {
        self.ephemeral_session_ids.contains(session_id)
    }

    pub(crate) fn has_session(&self, session_id: &str) -> bool {
        self.store.get(session_id).is_some()
    }

    pub(crate) fn durable_sessions(&self) -> Vec<RuntimeSession> {
        self.store
            .list()
            .into_iter()
            .filter(|session| !self.is_ephemeral_session(session.id()))
            .map(|session| session.durable_runtime_snapshot())
            .collect()
    }

    pub(crate) fn all_session_ids(&self) -> Vec<String> {
        self.store.session_ids()
    }

    pub(crate) fn archive_terminal_workflow_runs(
        &mut self,
        session_id: &str,
    ) -> Result<Vec<WorkflowRun>, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        Ok(session.archive_terminal_workflow_runs())
    }

    pub(crate) fn restore_active_workflow_runs(
        &mut self,
        session_id: &str,
        workflow_runs: Vec<WorkflowRun>,
    ) -> Result<(), DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        session.restore_active_workflow_runs(workflow_runs);
        Ok(())
    }

    pub(crate) fn restore_workflow_hot_state(
        &mut self,
        session_id: &str,
        state: DurableWorkflowHotState,
    ) -> Result<(), DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        session.restore_durable_workflow_hot_state(state);
        Ok(())
    }

    pub(crate) fn restore_workflow_event_delivery_receipts(
        &mut self,
        session_id: &str,
        receipts: Vec<WorkflowEventDeliveryReceipt>,
    ) -> Result<(), DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        session.restore_workflow_event_delivery_receipts(receipts);
        Ok(())
    }

    pub(crate) fn durable_projects(&self) -> Vec<RuntimeProject> {
        self.projects.values().cloned().collect()
    }

    fn default_session_alias(&self, workspace_id: &str) -> String {
        let base = default_session_alias_base(workspace_id);
        let mut number = self
            .store
            .list()
            .iter()
            .filter(|session| !session.is_hidden() && session.workspace_id() == workspace_id)
            .count()
            + 1;
        loop {
            let alias = format!("{base}-{number}");
            if self.store.visible_non_ended_sessions().all(|session| {
                session.workspace_id() != workspace_id || session.alias() != Some(alias.as_str())
            }) {
                return alias;
            }
            number += 1;
        }
    }

    pub(crate) fn restore_session(&mut self, session: RuntimeSession) -> RuntimeSession {
        self.restore_session_with_default_project_name_hint(session, None)
    }

    pub(crate) fn commit_publication_runtime_configuration(
        &mut self,
        session: RuntimeSession,
    ) -> Result<RuntimeSession, DaemonError> {
        let current = self
            .store
            .get(session.id())
            .ok_or_else(|| DaemonError::SessionNotFound {
                session_id: session.id().to_string(),
            })?;
        if !current.is_hidden()
            || current.owner_user_id() != session.owner_user_id()
            || current.host_daemon_id() != session.host_daemon_id()
        {
            return Err(DaemonError::LocalTransport {
                operation: "commit publication runtime configuration",
                message: "publication runtime session ownership changed".to_string(),
            });
        }
        Ok(self.store.insert(session))
    }

    pub(crate) fn restore_session_with_default_project_name_hint(
        &mut self,
        mut session: RuntimeSession,
        default_project_name_hint: Option<&str>,
    ) -> RuntimeSession {
        session.purge_unsupported_workflow_publications();
        if session.is_hidden() {
            session.clear_project_id_for_hidden_restore();
        } else if session.project_id().is_empty() {
            let project_id = self.ensure_default_project(
                session.owner_user_id(),
                session.workspace_id(),
                default_project_name_hint,
            );
            let assigned = session.assign_project_id(project_id);
            debug_assert!(assigned);
        } else if !self.projects.contains_key(session.project_id()) {
            let name = self.unique_project_name(
                session.owner_user_id(),
                &default_project_name(session.workspace_id()),
                None,
            );
            let project = RuntimeProject::new(
                session.project_id().to_string(),
                session.owner_user_id().to_string(),
                session.workspace_id().to_string(),
                name,
                RuntimeProjectKind::Default,
            );
            self.projects.insert(project.id().to_string(), project);
        }
        self.store.insert(session)
    }

    pub(crate) fn restore_projects(&mut self, projects: Vec<RuntimeProject>) {
        for mut project in projects {
            project.normalize_workspace_ids();
            self.projects.insert(project.id().to_string(), project);
        }
    }

    pub fn list_projects(
        &self,
        owner_user_id: &str,
        include_archived: bool,
    ) -> Vec<RuntimeProject> {
        self.projects
            .values()
            .filter(|project| project.owner_user_id() == owner_user_id)
            .filter(|project| include_archived || project.status() == RuntimeProjectStatus::Active)
            .cloned()
            .collect()
    }

    pub fn list_visible_projects(
        &self,
        caller_user_id: &str,
        include_archived: bool,
    ) -> Vec<RuntimeProject> {
        let visible_project_ids = self
            .store
            .list()
            .into_iter()
            .filter(|session| !session.is_hidden())
            .filter(|session| session.has_member(caller_user_id))
            .map(|session| session.project_id().to_string())
            .collect::<BTreeSet<_>>();
        self.projects
            .values()
            .filter(|project| visible_project_ids.contains(project.id()))
            .filter(|project| include_archived || project.status() == RuntimeProjectStatus::Active)
            .cloned()
            .collect()
    }

    pub(crate) fn remove_projects_without_visible_sessions(&mut self) -> Vec<RuntimeProject> {
        let visible_project_ids = self
            .store
            .list()
            .into_iter()
            .filter(|session| !session.is_hidden())
            .map(|session| session.project_id().to_string())
            .collect::<BTreeSet<_>>();
        let empty_project_ids = self
            .projects
            .keys()
            .filter(|project_id| !visible_project_ids.contains(project_id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        empty_project_ids
            .into_iter()
            .filter_map(|project_id| self.projects.remove(&project_id))
            .collect()
    }

    pub(crate) fn reconcile_duplicate_project_names(&mut self) -> Vec<RuntimeProject> {
        let visible_session_counts = self
            .store
            .list()
            .into_iter()
            .filter(|session| !session.is_hidden() && !session.project_id().is_empty())
            .fold(BTreeMap::<String, usize>::new(), |mut counts, session| {
                *counts.entry(session.project_id().to_string()).or_default() += 1;
                counts
            });
        let mut project_groups = BTreeMap::<(String, String), Vec<String>>::new();
        let mut occupied_names = BTreeMap::<String, BTreeSet<String>>::new();
        for project in self.projects.values() {
            let owner_user_id = project.owner_user_id().to_string();
            let name_key = project_name_key(project.name());
            project_groups
                .entry((owner_user_id.clone(), name_key.clone()))
                .or_default()
                .push(project.id().to_string());
            occupied_names
                .entry(owner_user_id)
                .or_default()
                .insert(name_key);
        }

        let mut renames = Vec::new();
        for ((owner_user_id, _), mut project_ids) in project_groups {
            if project_ids.len() < 2 {
                continue;
            }
            project_ids.sort_by(|left_id, right_id| {
                let left = self
                    .projects
                    .get(left_id)
                    .expect("grouped project should exist");
                let right = self
                    .projects
                    .get(right_id)
                    .expect("grouped project should exist");
                visible_session_counts
                    .get(right_id)
                    .copied()
                    .unwrap_or_default()
                    .cmp(
                        &visible_session_counts
                            .get(left_id)
                            .copied()
                            .unwrap_or_default(),
                    )
                    .then_with(|| left.created_at_ms().cmp(&right.created_at_ms()))
                    .then_with(|| left_id.cmp(right_id))
            });
            let base_name = self
                .projects
                .get(&project_ids[0])
                .expect("canonical project should exist")
                .name()
                .to_string();
            let occupied = occupied_names.entry(owner_user_id).or_default();
            for project_id in project_ids.into_iter().skip(1) {
                let name = unique_project_name_from_keys(&base_name, occupied);
                occupied.insert(project_name_key(&name));
                renames.push((project_id, name));
            }
        }

        renames
            .into_iter()
            .filter_map(|(project_id, name)| {
                let project = self.projects.get_mut(&project_id)?;
                project.rename(name);
                Some(project.clone())
            })
            .collect()
    }

    pub(crate) fn migrate_default_project_workspace(
        &mut self,
        session_id: &str,
        workspace_id: &str,
        default_project_name_hint: Option<&str>,
        replaced_project_ids: &BTreeSet<String>,
    ) -> Result<Option<RuntimeSession>, DaemonError> {
        let current = self.get_session(session_id)?;
        if current.is_hidden() || current.workspace_id() == workspace_id {
            return Ok(None);
        }
        let source_project = self.get_project(current.project_id())?;
        if source_project.kind() != RuntimeProjectKind::Default {
            return Ok(None);
        }

        let target_project_id = if let Some(project_id) = self
            .projects
            .values()
            .find(|project| {
                project.owner_user_id() == current.owner_user_id()
                    && project.workspace_id() == workspace_id
                    && project.kind() == RuntimeProjectKind::Default
            })
            .map(|project| project.id().to_string())
        {
            if self
                .projects
                .get(&project_id)
                .is_some_and(|project| project.status() == RuntimeProjectStatus::Archived)
            {
                self.projects
                    .get_mut(&project_id)
                    .expect("selected default project should exist")
                    .restore();
            }
            project_id
        } else {
            let project_id = default_project_id(current.owner_user_id(), workspace_id);
            let desired_name = default_project_name_hint
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| default_project_name(workspace_id));
            let name = self.unique_project_name_excluding(
                current.owner_user_id(),
                &desired_name,
                replaced_project_ids,
            );
            let project = RuntimeProject::new(
                project_id.clone(),
                current.owner_user_id().to_string(),
                workspace_id.to_string(),
                name,
                RuntimeProjectKind::Default,
            );
            self.projects.insert(project_id.clone(), project);
            project_id
        };
        let alias = self.migrated_session_alias(&current, workspace_id);
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        session.migrate_default_project_scope(workspace_id, target_project_id, alias);
        Ok(Some(session.clone()))
    }

    pub fn get_project(&self, project_id: &str) -> Result<RuntimeProject, DaemonError> {
        self.projects.get(project_id).cloned().ok_or_else(|| {
            project_error(
                "project.get",
                format!("project `{project_id}` was not found"),
            )
        })
    }

    pub fn sessions_in_project(&self, project_id: &str) -> Vec<RuntimeSession> {
        self.store
            .list()
            .into_iter()
            .filter(|session| session.project_id() == project_id)
            .collect()
    }

    pub fn rename_project(
        &mut self,
        project_id: &str,
        name: String,
        caller_user_id: &str,
    ) -> Result<RuntimeProject, DaemonError> {
        let name = normalize_project_name(&name)?;
        self.ensure_project_owner(project_id, caller_user_id, "project.rename")?;
        self.ensure_project_name_available(
            caller_user_id,
            &name,
            Some(project_id),
            "project.rename",
        )?;
        let project = self.project_mut_for_owner(project_id, caller_user_id, "project.rename")?;
        project.rename(name);
        Ok(project.clone())
    }

    pub fn update_project_workspaces(
        &mut self,
        project_id: &str,
        workspace_ids: Vec<String>,
        caller_user_id: &str,
    ) -> Result<RuntimeProject, DaemonError> {
        let workspace_ids = normalize_project_workspace_ids(workspace_ids)?;
        let project =
            self.ensure_project_owner(project_id, caller_user_id, "project.workspaces.update")?;
        if project.kind() != RuntimeProjectKind::Named {
            return Err(project_error(
                "project.workspaces.update",
                format!("default project `{project_id}` has immutable Workspace membership"),
            ));
        }
        let project =
            self.project_mut_for_owner(project_id, caller_user_id, "project.workspaces.update")?;
        project.replace_workspace_ids(workspace_ids);
        Ok(project.clone())
    }

    pub fn archive_project(
        &mut self,
        project_id: &str,
        caller_user_id: &str,
    ) -> Result<RuntimeProject, DaemonError> {
        let project = self.project_mut_for_owner(project_id, caller_user_id, "project.archive")?;
        project.archive();
        Ok(project.clone())
    }

    pub fn restore_project_status(
        &mut self,
        project_id: &str,
        caller_user_id: &str,
    ) -> Result<RuntimeProject, DaemonError> {
        let project = self.project_mut_for_owner(project_id, caller_user_id, "project.restore")?;
        project.restore();
        Ok(project.clone())
    }

    pub fn delete_project_record(
        &mut self,
        project_id: &str,
        caller_user_id: &str,
    ) -> Result<RuntimeProject, DaemonError> {
        self.ensure_project_owner(project_id, caller_user_id, "project.delete")?;
        self.projects.remove(project_id).ok_or_else(|| {
            project_error(
                "project.delete",
                format!("project `{project_id}` was not found"),
            )
        })
    }

    pub fn ensure_project_owner(
        &self,
        project_id: &str,
        caller_user_id: &str,
        operation: &'static str,
    ) -> Result<RuntimeProject, DaemonError> {
        let project = self.get_project(project_id)?;
        if project.owner_user_id() != caller_user_id {
            return Err(project_error(
                operation,
                format!("user `{caller_user_id}` does not own project `{project_id}`"),
            ));
        }
        Ok(project)
    }

    fn project_mut_for_owner(
        &mut self,
        project_id: &str,
        caller_user_id: &str,
        operation: &'static str,
    ) -> Result<&mut RuntimeProject, DaemonError> {
        self.ensure_project_owner(project_id, caller_user_id, operation)?;
        self.projects.get_mut(project_id).ok_or_else(|| {
            project_error(operation, format!("project `{project_id}` was not found"))
        })
    }

    fn resolve_project_selection(
        &mut self,
        request: &CreateSessionRequest,
    ) -> Result<String, DaemonError> {
        match &request.project_selection {
            SessionProjectSelection::Default => {
                let project_id = self.ensure_default_project(
                    &request.owner_user_id,
                    &request.workspace_id,
                    request.default_project_name_hint.as_deref(),
                );
                if self
                    .projects
                    .get(&project_id)
                    .is_some_and(|project| project.status() == RuntimeProjectStatus::Archived)
                {
                    return Err(project_error(
                        "session.create",
                        format!(
                            "default project `{project_id}` is archived; restore it before creating a session"
                        ),
                    ));
                }
                Ok(project_id)
            }
            SessionProjectSelection::Existing { project_id } => {
                let project = self.ensure_project_owner(
                    project_id,
                    &request.owner_user_id,
                    "session.create",
                )?;
                if !project.contains_workspace(&request.workspace_id) {
                    return Err(project_error(
                        "session.create",
                        format!(
                            "project `{project_id}` does not include Workspace `{}`",
                            request.workspace_id,
                        ),
                    ));
                }
                if project.status() != RuntimeProjectStatus::Active {
                    return Err(project_error(
                        "session.create",
                        format!(
                            "project `{project_id}` is archived; restore it before creating a session"
                        ),
                    ));
                }
                Ok(project_id.clone())
            }
            SessionProjectSelection::New => {
                let number = self.next_named_project_number(&request.owner_user_id);
                let id = self.next_named_project_id(
                    &request.owner_user_id,
                    &request.workspace_id,
                    number,
                );
                let project = RuntimeProject::new(
                    id.clone(),
                    request.owner_user_id.clone(),
                    request.workspace_id.clone(),
                    format!("Project-{number}"),
                    RuntimeProjectKind::Named,
                );
                self.projects.insert(id.clone(), project);
                Ok(id)
            }
        }
    }

    fn ensure_default_project(
        &mut self,
        owner_user_id: &str,
        workspace_id: &str,
        name_hint: Option<&str>,
    ) -> String {
        if let Some(project) = self.projects.values().find(|project| {
            project.owner_user_id() == owner_user_id
                && project.workspace_id() == workspace_id
                && project.kind() == RuntimeProjectKind::Default
        }) {
            return project.id().to_string();
        }
        let id = default_project_id(owner_user_id, workspace_id);
        let name = name_hint
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| default_project_name(workspace_id));
        let name = self.unique_project_name(owner_user_id, &name, None);
        let project = RuntimeProject::new(
            id.clone(),
            owner_user_id.to_string(),
            workspace_id.to_string(),
            name,
            RuntimeProjectKind::Default,
        );
        self.projects.insert(id.clone(), project);
        id
    }

    fn next_named_project_number(&self, owner_user_id: &str) -> u64 {
        self.projects
            .values()
            .filter(|project| project.owner_user_id() == owner_user_id)
            .filter_map(|project| project.name().strip_prefix("Project-"))
            .filter_map(|number| number.parse::<u64>().ok())
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }

    fn unique_project_name(
        &self,
        owner_user_id: &str,
        desired_name: &str,
        excluding_project_id: Option<&str>,
    ) -> String {
        let excluded_project_ids = excluding_project_id
            .map(str::to_string)
            .into_iter()
            .collect::<BTreeSet<_>>();
        self.unique_project_name_excluding(owner_user_id, desired_name, &excluded_project_ids)
    }

    fn unique_project_name_excluding(
        &self,
        owner_user_id: &str,
        desired_name: &str,
        excluded_project_ids: &BTreeSet<String>,
    ) -> String {
        let occupied = self
            .projects
            .values()
            .filter(|project| project.owner_user_id() == owner_user_id)
            .filter(|project| !excluded_project_ids.contains(project.id()))
            .map(|project| project_name_key(project.name()))
            .collect::<BTreeSet<_>>();
        unique_project_name_from_keys(desired_name, &occupied)
    }

    fn migrated_session_alias(
        &self,
        session: &RuntimeSession,
        workspace_id: &str,
    ) -> Option<String> {
        let alias = session.alias()?.to_string();
        if session.status() == SessionStatus::Ended
            || self.store.visible_non_ended_sessions().all(|candidate| {
                candidate.id() == session.id()
                    || candidate.workspace_id() != workspace_id
                    || candidate.alias() != Some(alias.as_str())
            })
        {
            return Some(alias);
        }
        let mut suffix = 2_u64;
        loop {
            let candidate_alias = format!("{alias}-{suffix}");
            if self.store.visible_non_ended_sessions().all(|candidate| {
                candidate.id() == session.id()
                    || candidate.workspace_id() != workspace_id
                    || candidate.alias() != Some(candidate_alias.as_str())
            }) {
                return Some(candidate_alias);
            }
            suffix = suffix.saturating_add(1);
        }
    }

    fn ensure_project_name_available(
        &self,
        owner_user_id: &str,
        name: &str,
        excluding_project_id: Option<&str>,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        let name_key = project_name_key(name);
        let conflict = self.projects.values().any(|project| {
            project.owner_user_id() == owner_user_id
                && Some(project.id()) != excluding_project_id
                && project_name_key(project.name()) == name_key
        });
        if conflict {
            return Err(project_error(
                operation,
                format!("project name `{name}` already exists"),
            ));
        }
        Ok(())
    }

    fn next_named_project_id(
        &self,
        owner_user_id: &str,
        workspace_id: &str,
        number: u64,
    ) -> String {
        let mut salt = number;
        loop {
            let candidate = format!(
                "project-{:016x}",
                stable_hash(&format!("{owner_user_id}\0{workspace_id}\0{salt}"))
            );
            if !self.projects.contains_key(&candidate) {
                return candidate;
            }
            salt = salt.saturating_add(1);
        }
    }

    pub(crate) fn remove_restored_session(&mut self, session_id: &str) -> Option<RuntimeSession> {
        self.ephemeral_session_ids.remove(session_id);
        self.room_environments.remove(session_id);
        self.store.remove(session_id)
    }

    pub(crate) fn replace_publication_runtime_workflows(
        &mut self,
        session_id: &str,
        workflows: Vec<WorkflowDefinition>,
        workflow_prompt_queues: Vec<WorkflowPromptQueueDefinition>,
        workflow_watchdogs: Vec<WorkflowWatchdogDefinition>,
    ) -> Result<RuntimeSession, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        session.replace_publication_runtime_workflows(
            workflows,
            workflow_prompt_queues,
            workflow_watchdogs,
        );
        let first_agent_id = session
            .workflows()
            .first()
            .and_then(|workflow| workflow.nodes().first())
            .map(|node| node.agent_id().to_string());
        session.set_focused_agent(first_agent_id);
        Ok(session.clone())
    }

    pub(crate) fn restore_workflow_publication(
        &mut self,
        session_id: &str,
        publication: WorkflowPublicationDefinition,
        source_snapshot: Option<WorkflowPublicationSnapshot>,
    ) -> Result<WorkflowPublicationDefinition, DaemonError> {
        if publication.session_id() != session_id {
            return Err(DaemonError::LocalTransport {
                operation: "restore workflow publication",
                message: format!(
                    "publication `{}` belongs to session `{}` instead of `{session_id}`",
                    publication.id(),
                    publication.session_id()
                ),
            });
        }
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        Ok(session.create_workflow_publication(publication, source_snapshot))
    }

    pub fn get_session(&self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        self.store
            .get(session_id)
            .cloned()
            .ok_or_else(|| DaemonError::SessionNotFound {
                session_id: session_id.to_string(),
            })
    }

    pub fn list_session_members(
        &self,
        session_id: &str,
    ) -> Result<(Vec<SessionMember>, Vec<SessionInvite>), DaemonError> {
        let session = self.get_session(session_id)?;
        Ok((session.members().to_vec(), session.invites().to_vec()))
    }

    pub fn create_workspace_link(
        &mut self,
        session_id: &str,
        name: String,
        created_by_user_id: String,
    ) -> Result<(RuntimeSession, WorkspaceLinkDefinition), DaemonError> {
        let normalized_name = normalize_workspace_link_name(&name)?;
        let link_id = self.next_workspace_link_id();
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        ensure_workspace_link_name_available(session, &normalized_name)?;
        let link = WorkspaceLinkDefinition::new(
            link_id,
            session_id.to_string(),
            normalized_name,
            created_by_user_id,
        );
        let link = session.create_workspace_link(link);
        session.touch();
        Ok((session.clone(), link))
    }

    pub fn list_workspace_links(
        &self,
        session_id: &str,
    ) -> Result<Vec<WorkspaceLinkDefinition>, DaemonError> {
        Ok(self.get_session(session_id)?.workspace_links().to_vec())
    }

    pub fn set_workspace_live_sync_mode(
        &mut self,
        session_id: &str,
        mode: crate::config::WorkspaceLiveSyncMode,
    ) -> Result<RuntimeSession, DaemonError> {
        let session =
            self.get_session_mut_for_operation(session_id, "set workspace live sync mode")?;
        session.set_workspace_live_sync_mode(Some(mode));
        Ok(session.clone())
    }

    pub fn resolve_workspace_link_ref(
        &self,
        session_id: &str,
        link_ref: &str,
    ) -> Result<WorkspaceLinkDefinition, DaemonError> {
        let session = self.get_session(session_id)?;
        resolve_workspace_link_ref_in_session(&session, link_ref).cloned()
    }

    pub fn attach_workspace_link(
        &mut self,
        session_id: &str,
        link_ref: &str,
        user_id: String,
        machine_id: String,
        kernel_id: String,
        repo_root: String,
        branch: Option<String>,
        repo_fingerprint: Option<String>,
    ) -> Result<
        (
            RuntimeSession,
            WorkspaceLinkDefinition,
            WorkspaceLinkAttachment,
        ),
        DaemonError,
    > {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let link_id = resolve_workspace_link_ref_in_session(session, link_ref)?
            .link_id()
            .to_string();
        let attachment = WorkspaceLinkAttachment::new(
            link_id.clone(),
            user_id,
            machine_id,
            kernel_id,
            repo_root,
            branch,
            repo_fingerprint,
        );
        let link =
            session
                .workspace_link_mut(&link_id)
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "attach workspace link",
                    message: format!("workspace link `{link_ref}` was not found"),
                })?;
        let attachment = link.attach(attachment);
        let link = link.clone();
        session.touch();
        Ok((session.clone(), link, attachment))
    }

    pub fn detach_workspace_link(
        &mut self,
        session_id: &str,
        link_ref: &str,
        user_id: String,
        repo_root: Option<&Path>,
    ) -> Result<
        (
            RuntimeSession,
            WorkspaceLinkDefinition,
            Vec<WorkspaceLinkAttachment>,
        ),
        DaemonError,
    > {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let link_id = resolve_workspace_link_ref_in_session(session, link_ref)?
            .link_id()
            .to_string();
        let link =
            session
                .workspace_link_mut(&link_id)
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "detach workspace link",
                    message: format!("workspace link `{link_ref}` was not found"),
                })?;
        let detached = link.detach(&user_id, repo_root);
        let link = link.clone();
        session.touch();
        Ok((session.clone(), link, detached))
    }

    fn next_workspace_link_id(&mut self) -> String {
        self.next_workspace_link_number += 1;
        format!("workspace-link-{}", self.next_workspace_link_number)
    }

    pub fn create_session_invite(
        &mut self,
        session_id: &str,
        invite_id: String,
        created_by_user_id: String,
        expires_at_ms: Option<u64>,
        max_uses: Option<u32>,
        collaboration_level: CollaborationLevel,
    ) -> Result<(RuntimeSession, SessionInvite), DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        if !session.has_member(&created_by_user_id) {
            return Err(DaemonError::LocalTransport {
                operation: "create session invite",
                message: format!(
                    "user `{created_by_user_id}` is not a member of session `{session_id}`"
                ),
            });
        }
        let invite = SessionInvite::new(
            invite_id,
            session_id,
            created_by_user_id,
            unix_epoch_ms(),
            expires_at_ms,
            max_uses,
            collaboration_level,
        );
        let invite = session.add_invite(invite);
        session.touch();
        Ok((session.clone(), invite))
    }

    pub fn join_session_invite(
        &mut self,
        session_id: &str,
        invite_id: &str,
        user_id: String,
        now_ms: u64,
    ) -> Result<(RuntimeSession, SessionMember), DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let (invited_by_user_id, collaboration_level) = {
            let invite =
                session
                    .invite_mut(invite_id)
                    .ok_or_else(|| DaemonError::LocalTransport {
                        operation: "join session invite",
                        message: format!("session invite `{invite_id}` was not found"),
                    })?;
            if invite.session_id() != session_id {
                return Err(DaemonError::LocalTransport {
                    operation: "join session invite",
                    message: "session invite target does not match the local session".to_string(),
                });
            }
            if invite.is_revoked() {
                return Err(DaemonError::LocalTransport {
                    operation: "join session invite",
                    message: "session invite is revoked".to_string(),
                });
            }
            if invite.is_expired(now_ms) {
                return Err(DaemonError::LocalTransport {
                    operation: "join session invite",
                    message: "session invite is expired".to_string(),
                });
            }
            if invite.is_exhausted() {
                return Err(DaemonError::LocalTransport {
                    operation: "join session invite",
                    message: "session invite has no uses remaining".to_string(),
                });
            }
            invite.mark_used();
            (
                Some(invite.created_by_user_id().to_string()),
                invite.collaboration_level(),
            )
        };
        let member = session.add_member(user_id, invited_by_user_id, collaboration_level);
        session.touch();
        Ok((session.clone(), member))
    }

    pub fn revoke_session_invite(
        &mut self,
        session_id: &str,
        invite_id: &str,
    ) -> Result<(RuntimeSession, SessionInvite), DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let invite = session
            .invite_mut(invite_id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "revoke session invite",
                message: format!("session invite `{invite_id}` was not found"),
            })?;
        invite.revoke(unix_epoch_ms());
        let invite = invite.clone();
        session.touch();
        Ok((session.clone(), invite))
    }

    pub fn list_sessions(&self) -> Vec<RuntimeSession> {
        self.store.visible_non_ended_sessions().cloned().collect()
    }

    pub fn list_non_ended_sessions_including_hidden(&self) -> Vec<RuntimeSession> {
        self.store.non_ended_sessions().cloned().collect()
    }

    pub fn list_all_sessions(&self) -> Vec<RuntimeSession> {
        self.store.list()
    }

    pub fn list_workflows(&self, session_id: &str) -> Result<Vec<WorkflowDefinition>, DaemonError> {
        Ok(self.get_session(session_id)?.workflows().to_vec())
    }

    pub fn resolve_workflow_ref(
        &self,
        session_id: &str,
        workflow_ref: &str,
    ) -> Result<WorkflowDefinition, DaemonError> {
        let normalized_ref = workflow_ref.trim().to_lowercase();
        let session = self.get_session(session_id)?;
        let workflows = session.workflows();
        if let Some(workflow) = workflows
            .iter()
            .find(|workflow| workflow.id() == normalized_ref)
        {
            return Ok(workflow.clone());
        }
        if let Some(workflow) = workflows
            .iter()
            .find(|workflow| workflow.alias() == Some(normalized_ref.as_str()))
        {
            return Ok(workflow.clone());
        }
        let id_matches = workflows
            .iter()
            .filter(|workflow| workflow.id().starts_with(&normalized_ref))
            .cloned()
            .collect::<Vec<_>>();
        if id_matches.len() == 1 {
            return Ok(id_matches[0].clone());
        }
        let alias_matches = workflows
            .iter()
            .filter(|workflow| {
                workflow
                    .alias()
                    .is_some_and(|alias| alias.starts_with(normalized_ref.as_str()))
            })
            .cloned()
            .collect::<Vec<_>>();
        if alias_matches.len() == 1 {
            return Ok(alias_matches[0].clone());
        }
        Err(DaemonError::WorkflowNotFound {
            session_id: session_id.to_string(),
            workflow_id: workflow_ref.to_string(),
        })
    }

    pub fn create_workflow(
        &mut self,
        session_id: &str,
        alias: Option<String>,
    ) -> Result<WorkflowDefinition, DaemonError> {
        self.create_workflow_controlled_by_metaagent(session_id, alias, None)
    }

    pub fn create_workflow_controlled_by_metaagent(
        &mut self,
        session_id: &str,
        alias: Option<String>,
        controlled_by_metaagent_id: Option<String>,
    ) -> Result<WorkflowDefinition, DaemonError> {
        self.create_workflow_controlled_by_metaagent_with_alias_base(
            session_id,
            alias,
            "workflow",
            controlled_by_metaagent_id,
        )
    }

    pub(super) fn create_workflow_controlled_by_metaagent_with_alias_base(
        &mut self,
        session_id: &str,
        alias: Option<String>,
        default_base: &str,
        controlled_by_metaagent_id: Option<String>,
    ) -> Result<WorkflowDefinition, DaemonError> {
        let alias = self.workflow_alias_for_create(session_id, alias, default_base)?;
        let workflow = match controlled_by_metaagent_id {
            Some(metaagent_id) => WorkflowDefinition::new_controlled_by_metaagent(
                self.next_workflow_id(),
                alias,
                metaagent_id,
            ),
            None => WorkflowDefinition::new(self.next_workflow_id(), alias),
        };
        let workflow = workflow.with_max_concurrent(self.workflow_default_max_concurrent);
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        Ok(session.create_workflow(workflow))
    }

    pub(super) fn workflow_alias_for_create(
        &self,
        session_id: &str,
        alias: Option<String>,
        default_base: &str,
    ) -> Result<Option<String>, DaemonError> {
        let alias = match alias {
            Some(alias) if !alias.trim().is_empty() => normalize_workflow_alias(Some(alias))?,
            _ => Some(self.default_workflow_alias(session_id, default_base)?),
        };
        if let Some(alias) = alias.as_deref() {
            self.ensure_workflow_alias_available(session_id, alias)?;
        }
        Ok(alias)
    }

    fn default_workflow_alias(&self, session_id: &str, base: &str) -> Result<String, DaemonError> {
        let base = default_workflow_alias_base(base);
        let session = self
            .store
            .get(session_id)
            .ok_or_else(|| DaemonError::SessionNotFound {
                session_id: session_id.to_string(),
            })?;
        let mut number = session
            .workflows()
            .iter()
            .filter(|workflow| {
                workflow
                    .alias()
                    .is_some_and(|alias| workflow_alias_uses_base(alias, &base))
            })
            .count()
            + 1;
        loop {
            let alias = format!("{base}-{number}");
            if session
                .workflows()
                .iter()
                .all(|workflow| workflow.alias() != Some(alias.as_str()))
            {
                return Ok(alias);
            }
            number += 1;
        }
    }

    pub fn assign_workflow_alias(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        alias: String,
    ) -> Result<WorkflowDefinition, DaemonError> {
        let workflow_id = self
            .resolve_workflow_ref(session_id, workflow_ref)?
            .id()
            .to_string();
        let alias = normalize_workflow_alias(Some(alias))?.ok_or_else(|| {
            DaemonError::InvalidWorkflowAlias {
                alias: String::new(),
                message: "alias cannot be empty",
            }
        })?;
        self.ensure_workflow_alias_available_for_update(session_id, &workflow_id, &alias)?;
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow =
            session
                .workflow_mut(&workflow_id)
                .ok_or_else(|| DaemonError::WorkflowNotFound {
                    session_id: session_id.to_string(),
                    workflow_id: workflow_id.to_string(),
                })?;
        workflow.set_alias(Some(alias));
        Ok(workflow.clone())
    }

    pub fn set_workflow_flush_agent_context_before_run(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        value: bool,
    ) -> Result<WorkflowDefinition, DaemonError> {
        let workflow_id = self
            .resolve_workflow_ref(session_id, workflow_ref)?
            .id()
            .to_string();
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow =
            session
                .workflow_mut(&workflow_id)
                .ok_or_else(|| DaemonError::WorkflowNotFound {
                    session_id: session_id.to_string(),
                    workflow_id: workflow_id.clone(),
                })?;
        workflow.set_flush_agent_context_before_run(value);
        Ok(workflow.clone())
    }

    pub fn set_workflow_run_output_schema_ref(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        value: Option<String>,
    ) -> Result<WorkflowDefinition, DaemonError> {
        let workflow_id = self
            .resolve_workflow_ref(session_id, workflow_ref)?
            .id()
            .to_string();
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow =
            session
                .workflow_mut(&workflow_id)
                .ok_or_else(|| DaemonError::WorkflowNotFound {
                    session_id: session_id.to_string(),
                    workflow_id: workflow_id.clone(),
                })?;
        workflow.set_run_output_schema_ref(value);
        Ok(workflow.clone())
    }

    pub fn assign_session_alias(
        &mut self,
        session_id: &str,
        alias: String,
    ) -> Result<RuntimeSession, DaemonError> {
        let alias = normalize_session_alias(Some(alias))?.ok_or_else(|| {
            DaemonError::InvalidSessionAlias {
                alias: String::new(),
                message: "alias cannot be empty",
            }
        })?;

        let session = self.get_session(session_id)?;
        self.ensure_session_alias_available_for_update(
            session.workspace_id(),
            session.id(),
            &alias,
        )?;

        let session = self.get_session_mut_for_operation(session_id, "assign alias")?;
        session.set_alias(Some(alias));
        Ok(session.clone())
    }

    pub fn resolve_workflow_endpoint_ref(
        &self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
    ) -> Result<WorkflowEndpointDefinition, DaemonError> {
        let workflow = self.resolve_workflow_ref(session_id, workflow_ref)?;
        let normalized_ref = endpoint_ref.trim().to_lowercase();
        if let Some(endpoint) = workflow
            .endpoints()
            .iter()
            .find(|endpoint| endpoint.id() == normalized_ref)
        {
            return Ok(endpoint.clone());
        }
        if let Some(endpoint) = workflow
            .endpoints()
            .iter()
            .find(|endpoint| endpoint.alias() == Some(normalized_ref.as_str()))
        {
            return Ok(endpoint.clone());
        }
        let id_matches = workflow
            .endpoints()
            .iter()
            .filter(|endpoint| endpoint.id().starts_with(&normalized_ref))
            .cloned()
            .collect::<Vec<_>>();
        if id_matches.len() == 1 {
            return Ok(id_matches[0].clone());
        }
        let alias_matches = workflow
            .endpoints()
            .iter()
            .filter(|endpoint| {
                endpoint
                    .alias()
                    .is_some_and(|alias| alias.starts_with(normalized_ref.as_str()))
            })
            .cloned()
            .collect::<Vec<_>>();
        if alias_matches.len() == 1 {
            return Ok(alias_matches[0].clone());
        }
        Err(DaemonError::WorkflowEndpointNotFound {
            session_id: session_id.to_string(),
            workflow_id: workflow.id().to_string(),
            endpoint_id: endpoint_ref.to_string(),
        })
    }

    pub fn list_workflow_runs(
        &self,
        session_id: &str,
        workflow_ref: Option<&str>,
    ) -> Result<Vec<WorkflowRun>, DaemonError> {
        let workflow_id = workflow_ref
            .map(|reference| self.resolve_workflow_ref(session_id, reference))
            .transpose()?
            .map(|workflow| workflow.id().to_string());
        let session = self.get_session(session_id)?;
        Ok(session
            .workflow_runs()
            .iter()
            .filter(|workflow_run| {
                workflow_id
                    .as_deref()
                    .is_none_or(|id| workflow_run.workflow_id() == id)
            })
            .cloned()
            .collect())
    }

    pub fn resolve_workflow_run_ref(
        &self,
        session_id: &str,
        workflow_run_ref: &str,
    ) -> Result<WorkflowRun, DaemonError> {
        let normalized_ref = workflow_run_ref.trim().to_lowercase();
        let session = self.get_session(session_id)?;
        let workflow_runs = session.workflow_runs();
        if let Some(workflow_run) = workflow_runs
            .iter()
            .find(|workflow_run| workflow_run.id() == normalized_ref)
        {
            return Ok(workflow_run.clone());
        }
        let id_matches = workflow_runs
            .iter()
            .filter(|workflow_run| workflow_run.id().starts_with(&normalized_ref))
            .cloned()
            .collect::<Vec<_>>();
        if id_matches.len() == 1 {
            return Ok(id_matches[0].clone());
        }
        Err(DaemonError::WorkflowRunNotFound {
            session_id: session_id.to_string(),
            workflow_run_id: workflow_run_ref.to_string(),
        })
    }

    pub fn resolve_workflow_prompt_queue_ref(
        &self,
        session_id: &str,
        workflow_id: &str,
        queue_ref: &str,
    ) -> Result<String, DaemonError> {
        let normalized_ref = queue_ref.trim().to_lowercase();
        let session = self.get_session(session_id)?;
        if let Some(queue) = session.workflow_prompt_queues().iter().find(|queue| {
            queue.workflow_id() == workflow_id
                && (queue.id() == normalized_ref || queue.alias() == normalized_ref)
        }) {
            return Ok(queue.id().to_string());
        }
        let matches = session
            .workflow_prompt_queues()
            .iter()
            .filter(|queue| {
                queue.workflow_id() == workflow_id
                    && (queue.id().starts_with(&normalized_ref)
                        || queue.alias().starts_with(&normalized_ref))
            })
            .map(|queue| queue.id().to_string())
            .collect::<Vec<_>>();
        if matches.len() == 1 {
            return Ok(matches[0].clone());
        }
        Err(DaemonError::InvalidWorkflowGraphReference {
            session_id: session_id.to_string(),
            workflow_id: workflow_id.to_string(),
            reference: queue_ref.to_string(),
            message: "workflow prompt queue was not found",
        })
    }

    pub fn resolve_queued_workflow_prompt_ref(
        &self,
        session_id: &str,
        queue_item_ref: &str,
    ) -> Result<String, DaemonError> {
        let normalized_ref = queue_item_ref.trim().to_lowercase();
        let session = self.get_session(session_id)?;
        if let Some(queued_prompt) = session
            .workflow_queued_prompts()
            .iter()
            .find(|queued_prompt| queued_prompt.id() == normalized_ref)
        {
            return Ok(queued_prompt.id().to_string());
        }
        let id_matches = session
            .workflow_queued_prompts()
            .iter()
            .filter(|queued_prompt| queued_prompt.id().starts_with(&normalized_ref))
            .map(|queued_prompt| queued_prompt.id().to_string())
            .collect::<Vec<_>>();
        if id_matches.len() == 1 {
            return Ok(id_matches[0].clone());
        }
        Err(DaemonError::InvalidWorkflowGraphReference {
            session_id: session_id.to_string(),
            workflow_id: normalized_ref.clone(),
            reference: queue_item_ref.to_string(),
            message: "queued workflow prompt was not found",
        })
    }
}

fn default_session_alias_base(workspace_id: &str) -> String {
    let repo_name = Path::new(workspace_id)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(workspace_id);
    default_alias_base(repo_name, "workspace")
}

fn default_workflow_alias_base(base: &str) -> String {
    default_alias_base(base, "workflow")
}

fn default_alias_base(input: &str, fallback: &str) -> String {
    let mut base = String::new();
    let mut previous_separator = false;
    for char in input.trim().to_lowercase().chars() {
        if char.is_ascii_lowercase() || char.is_ascii_digit() || char == '_' {
            base.push(char);
            previous_separator = false;
        } else if char == '-' || char.is_ascii_whitespace() {
            if !previous_separator && !base.is_empty() {
                base.push('-');
                previous_separator = true;
            }
        } else if !previous_separator && !base.is_empty() {
            base.push('-');
            previous_separator = true;
        }
    }
    while base.ends_with('-') {
        base.pop();
    }
    if base.is_empty() {
        fallback.to_string()
    } else {
        base
    }
}

fn workflow_alias_uses_base(alias: &str, base: &str) -> bool {
    let Some(suffix) = alias.strip_prefix(base) else {
        return false;
    };
    suffix.strip_prefix('-').is_some_and(|number| {
        !number.is_empty() && number.chars().all(|char| char.is_ascii_digit())
    })
}

fn normalize_workspace_link_name(name: &str) -> Result<String, DaemonError> {
    let normalized = name.trim().to_lowercase();
    if normalized.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "create workspace link",
            message: "workspace link name cannot be empty".to_string(),
        });
    }
    if !normalized
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.')
    {
        return Err(DaemonError::LocalTransport {
            operation: "create workspace link",
            message: "workspace link name may only contain letters, numbers, '-', '_' or '.'"
                .to_string(),
        });
    }
    Ok(normalized)
}

fn ensure_workspace_link_name_available(
    session: &RuntimeSession,
    name: &str,
) -> Result<(), DaemonError> {
    if session
        .workspace_links()
        .iter()
        .any(|link| link.name() == name)
    {
        Err(DaemonError::LocalTransport {
            operation: "create workspace link",
            message: format!("workspace link `{name}` already exists"),
        })
    } else {
        Ok(())
    }
}

fn resolve_workspace_link_ref_in_session<'a>(
    session: &'a RuntimeSession,
    link_ref: &str,
) -> Result<&'a WorkspaceLinkDefinition, DaemonError> {
    let normalized_ref = link_ref.trim().to_lowercase();
    if normalized_ref.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "resolve workspace link",
            message: "workspace link reference cannot be empty".to_string(),
        });
    }
    if let Some(link) = session
        .workspace_links()
        .iter()
        .find(|link| link.link_id() == normalized_ref || link.name() == normalized_ref)
    {
        return Ok(link);
    }
    let matches = session
        .workspace_links()
        .iter()
        .filter(|link| {
            link.link_id().starts_with(&normalized_ref) || link.name().starts_with(&normalized_ref)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [link] => Ok(*link),
        [] => Err(DaemonError::LocalTransport {
            operation: "resolve workspace link",
            message: format!("workspace link `{normalized_ref}` was not found"),
        }),
        _ => Err(DaemonError::LocalTransport {
            operation: "resolve workspace link",
            message: format!("workspace link `{normalized_ref}` is ambiguous"),
        }),
    }
}

const MAX_PROJECT_NAME_CHARS: usize = 120;
const MAX_PROJECT_WORKSPACES: usize = 32;
const MAX_PROJECT_WORKSPACE_ID_BYTES: usize = 4 * 1024;

fn normalize_project_workspace_ids(workspace_ids: Vec<String>) -> Result<Vec<String>, DaemonError> {
    if workspace_ids.is_empty() || workspace_ids.len() > MAX_PROJECT_WORKSPACES {
        return Err(project_error(
            "project.workspaces.update",
            format!("a project must contain between 1 and {MAX_PROJECT_WORKSPACES} Workspaces"),
        ));
    }
    let mut seen = BTreeSet::new();
    for workspace_id in &workspace_ids {
        if workspace_id.trim().is_empty() || workspace_id.len() > MAX_PROJECT_WORKSPACE_ID_BYTES {
            return Err(project_error(
                "project.workspaces.update",
                "project Workspace identifiers must be non-empty and bounded".to_string(),
            ));
        }
        if !seen.insert(workspace_id.as_str()) {
            return Err(project_error(
                "project.workspaces.update",
                format!("project Workspace `{workspace_id}` is duplicated"),
            ));
        }
    }
    Ok(workspace_ids)
}

fn normalize_project_name(name: &str) -> Result<String, DaemonError> {
    let normalized = name.trim();
    if normalized.is_empty() {
        return Err(project_error(
            "project.rename",
            "project name cannot be empty".to_string(),
        ));
    }
    if normalized.chars().count() > MAX_PROJECT_NAME_CHARS {
        return Err(project_error(
            "project.rename",
            "project name cannot exceed 120 characters".to_string(),
        ));
    }
    Ok(normalized.to_string())
}

fn project_name_key(name: &str) -> String {
    name.trim().to_lowercase()
}

fn unique_project_name_from_keys(desired_name: &str, occupied: &BTreeSet<String>) -> String {
    let base = truncate_project_name(desired_name.trim(), MAX_PROJECT_NAME_CHARS);
    let base = if base.is_empty() {
        "Project".to_string()
    } else {
        base
    };
    if !occupied.contains(&project_name_key(&base)) {
        return base;
    }
    let mut suffix_number = 2_u64;
    loop {
        let suffix = format!(" ({suffix_number})");
        let prefix = truncate_project_name(
            &base,
            MAX_PROJECT_NAME_CHARS.saturating_sub(suffix.chars().count()),
        );
        let candidate = format!("{prefix}{suffix}");
        if !occupied.contains(&project_name_key(&candidate)) {
            return candidate;
        }
        suffix_number = suffix_number.saturating_add(1);
    }
}

fn truncate_project_name(name: &str, max_chars: usize) -> String {
    name.chars().take(max_chars).collect()
}

fn project_error(operation: &'static str, message: String) -> DaemonError {
    DaemonError::LocalTransport { operation, message }
}

fn default_project_id(owner_user_id: &str, workspace_id: &str) -> String {
    format!(
        "project-default-{:016x}",
        stable_hash(&format!("{owner_user_id}\0{workspace_id}"))
    )
}

fn default_project_name(workspace_id: &str) -> String {
    let trimmed = workspace_id.trim().trim_end_matches(['/', '\\']);
    std::path::Path::new(trimmed)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("Project")
        .to_string()
}

fn stable_hash(value: &str) -> u64 {
    value
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
}
