use super::*;
#[cfg(test)]
use crate::session::PromptStatus;

impl SessionService {
    pub fn resolve_session_ref(
        &self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Result<RuntimeSession, DaemonError> {
        let normalized_ref = session_ref.trim().to_lowercase();
        if normalized_ref.is_empty() {
            return Err(DaemonError::SessionNotFound {
                session_id: normalized_ref,
            });
        }

        let all_sessions = self
            .store
            .visible_non_ended_sessions()
            .cloned()
            .collect::<Vec<_>>();
        let workspace_sessions = all_sessions
            .iter()
            .filter(|session| {
                workspace_id.is_none_or(|workspace| session.workspace_id() == workspace)
            })
            .cloned()
            .collect::<Vec<_>>();

        if let Some(session) = all_sessions
            .iter()
            .find(|session| session.id() == normalized_ref)
        {
            return Ok(session.clone());
        }
        if let Some(session) = workspace_sessions
            .iter()
            .find(|session| session.alias() == Some(normalized_ref.as_str()))
        {
            return Ok(session.clone());
        }

        let id_matches = all_sessions
            .iter()
            .filter(|session| session.id().starts_with(&normalized_ref))
            .cloned()
            .collect::<Vec<_>>();
        if id_matches.len() == 1 {
            return Ok(id_matches[0].clone());
        }
        if id_matches.len() > 1 {
            return Err(DaemonError::AmbiguousSessionRef {
                session_ref: normalized_ref,
                matches: id_matches
                    .into_iter()
                    .map(|session| describe_session_match(&session))
                    .collect(),
            });
        }

        let alias_matches = workspace_sessions
            .iter()
            .filter(|session| {
                session
                    .alias()
                    .is_some_and(|alias| alias.starts_with(normalized_ref.as_str()))
            })
            .cloned()
            .collect::<Vec<_>>();
        if alias_matches.len() == 1 {
            return Ok(alias_matches[0].clone());
        }
        if alias_matches.len() > 1 {
            return Err(DaemonError::AmbiguousSessionRef {
                session_ref: normalized_ref,
                matches: alias_matches
                    .into_iter()
                    .map(|session| describe_session_match(&session))
                    .collect(),
            });
        }

        Err(DaemonError::SessionNotFound {
            session_id: normalized_ref,
        })
    }

    pub fn transition_session(
        &mut self,
        session_id: &str,
        next_status: SessionStatus,
    ) -> Result<RuntimeSession, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;

        if !session.transition_to(next_status) {
            return Err(DaemonError::InvalidSessionTransition {
                session_id: session_id.to_string(),
                from: session.status(),
                to: next_status,
            });
        }

        session.touch();

        Ok(session.clone())
    }

    pub fn end_session(&mut self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        self.transition_session(session_id, SessionStatus::Ended)
    }

    pub fn delete_session(&mut self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        self.delete_session_with_project_cleanup(session_id)
            .map(|(session, _)| session)
    }

    pub(crate) fn delete_session_with_project_cleanup(
        &mut self,
        session_id: &str,
    ) -> Result<(RuntimeSession, Option<RuntimeProject>), DaemonError> {
        let deleted =
            self.store
                .remove(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        self.room_environments.remove(session_id);
        self.ephemeral_session_ids.remove(session_id);
        let project_id = deleted.project_id().to_string();
        let project_has_visible_sessions = self
            .store
            .list()
            .into_iter()
            .any(|session| !session.is_hidden() && session.project_id() == project_id.as_str());
        let removed_project = (!project_id.is_empty() && !project_has_visible_sessions)
            .then(|| self.projects.remove(&project_id))
            .flatten();
        Ok((deleted, removed_project))
    }

    pub fn add_attachment_to_session(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<RuntimeSession, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;

        if session.status() == SessionStatus::Ended {
            let _ = session.transition_to(SessionStatus::Parked);
        }

        session.add_attachment(attachment_id);
        session.touch();
        Ok(session.clone())
    }

    pub fn remove_attachment_from_session(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<(RuntimeSession, PromptDetachEffect), DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "detach")?;

        if !session.remove_attachment(attachment_id) {
            return Err(DaemonError::AttachmentNotInSession {
                session_id: session_id.to_string(),
                attachment_id: attachment_id.to_string(),
            });
        }

        session.touch();

        Ok((
            session.clone(),
            PromptDetachEffect {
                removed_active_prompt: false,
                removed_queued_prompt_count: 0,
            },
        ))
    }

    pub fn set_active_provider_run(
        &mut self,
        session_id: &str,
        provider_run_id: Option<String>,
    ) -> Result<RuntimeSession, DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "set active provider run")?;

        session.set_active_provider_run(provider_run_id);

        let target_status = if session.active_provider_run_id().is_some() {
            SessionStatus::Active
        } else if session.status() == SessionStatus::Active {
            SessionStatus::Parked
        } else {
            session.status()
        };

        let _ = session.transition_to(target_status);
        session.touch();
        Ok(session.clone())
    }

    pub fn set_focused_agent(
        &mut self,
        session_id: &str,
        agent_id: Option<String>,
    ) -> Result<RuntimeSession, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;

        session.set_focused_agent(agent_id);
        session.touch();
        Ok(session.clone())
    }

    pub fn note_agent_output_sequence(
        &mut self,
        session_id: &str,
        agent_id: &str,
        sequence: u64,
    ) -> Result<RuntimeSession, DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "note agent output")?;
        session.note_agent_output_sequence(agent_id, sequence);
        session.touch();
        Ok(session.clone())
    }

    pub fn upsert_external_provider_import(
        &mut self,
        session_id: &str,
        import: crate::provider::ExternalProviderImportMetadata,
    ) -> Result<RuntimeSession, DaemonError> {
        let session = self.get_session_mut_for_operation(
            session_id,
            "upsert external provider import metadata",
        )?;
        session.upsert_external_provider_import(import);
        session.touch();
        Ok(session.clone())
    }

    pub fn record_workflow_node_thinking_trace_for_node_run(
        &mut self,
        session_id: &str,
        workflow_node_run_id: &str,
        message: impl Into<String>,
    ) -> Result<Option<RuntimeSession>, DaemonError> {
        let message = message.into();
        if message.trim().is_empty() {
            return Ok(None);
        }
        let session = self.get_session_mut_for_operation(session_id, "record workflow thinking")?;
        let Some(node_run) = session.workflow_node_run_mut(workflow_node_run_id) else {
            return Ok(None);
        };
        if node_run.add_thinking_trace(message).is_none() {
            return Ok(None);
        }
        session.touch();
        Ok(Some(session.clone()))
    }

    pub fn ensure_metaagent_task(
        &mut self,
        session_id: &str,
        metaagent_id: &str,
        task_markdown: impl Into<String>,
    ) -> Result<RuntimeSession, DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "ensure metaagent task")?;
        session.ensure_metaagent_task(metaagent_id, task_markdown);
        session.touch();
        Ok(session.clone())
    }

    pub fn enqueue_metaagent_task(
        &mut self,
        session_id: &str,
        metaagent_id: &str,
        source_attachment_id: &str,
        task_markdown: impl Into<String>,
        attachments: Vec<crate::session::PromptAttachment>,
    ) -> Result<crate::session::QueuedMetaagentTask, DaemonError> {
        let id = format!("session-task:{}", self.reserve_prompt_id());
        let task = crate::session::QueuedMetaagentTask::new(
            id,
            metaagent_id,
            source_attachment_id,
            task_markdown,
            attachments,
        );
        let session = self.get_session_mut_for_operation(session_id, "enqueue metaagent task")?;
        let task = session.enqueue_metaagent_task(task);
        session.touch();
        Ok(task)
    }

    pub fn pop_next_queued_metaagent_task(
        &mut self,
        session_id: &str,
    ) -> Result<Option<crate::session::QueuedMetaagentTask>, DaemonError> {
        let session =
            self.get_session_mut_for_operation(session_id, "start queued metaagent task")?;
        if session.has_active_session_task() {
            return Ok(None);
        }
        let task = session.pop_next_queued_metaagent_task();
        if task.is_some() {
            session.touch();
        }
        Ok(task)
    }

    pub fn requeue_metaagent_task_front(
        &mut self,
        session_id: &str,
        task: crate::session::QueuedMetaagentTask,
    ) -> Result<RuntimeSession, DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "requeue metaagent task")?;
        session.requeue_metaagent_task_front(task);
        session.touch();
        Ok(session.clone())
    }

    pub fn start_metaagent_task_if_needed(
        &mut self,
        session_id: &str,
        metaagent_id: &str,
        task_markdown: impl Into<String>,
    ) -> Result<Option<RuntimeSession>, DaemonError> {
        let session =
            self.get_session_mut_for_operation(session_id, "start metaagent task if needed")?;
        if session
            .start_metaagent_task_if_needed(metaagent_id, task_markdown)
            .is_none()
        {
            return Ok(None);
        }
        session.touch();
        Ok(Some(session.clone()))
    }

    pub fn start_or_update_metaagent_task(
        &mut self,
        session_id: &str,
        metaagent_id: &str,
        task_markdown: impl Into<String>,
    ) -> Result<RuntimeSession, DaemonError> {
        let session =
            self.get_session_mut_for_operation(session_id, "start or update metaagent task")?;
        session.start_or_update_metaagent_task(metaagent_id, task_markdown);
        session.touch();
        Ok(session.clone())
    }

    pub fn update_metaagent_task_markdown(
        &mut self,
        session_id: &str,
        metaagent_id: &str,
        task_markdown: impl Into<String>,
    ) -> Result<RuntimeSession, DaemonError> {
        let session =
            self.get_session_mut_for_operation(session_id, "update metaagent task markdown")?;
        session.update_metaagent_task_markdown(metaagent_id, task_markdown);
        session.touch();
        Ok(session.clone())
    }

    pub fn update_metaagent_plan_markdown(
        &mut self,
        session_id: &str,
        metaagent_id: &str,
        plan_markdown: impl Into<String>,
    ) -> Result<RuntimeSession, DaemonError> {
        let session =
            self.get_session_mut_for_operation(session_id, "update metaagent plan markdown")?;
        session.update_metaagent_plan_markdown(metaagent_id, plan_markdown);
        session.touch();
        Ok(session.clone())
    }

    pub fn set_metaagent_task_status(
        &mut self,
        session_id: &str,
        metaagent_id: &str,
        status: crate::session::MetaagentTaskStatus,
    ) -> Result<RuntimeSession, DaemonError> {
        let session =
            self.get_session_mut_for_operation(session_id, "set metaagent task status")?;
        session
            .set_metaagent_task_status(metaagent_id, status)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "set_metaagent_task_status",
                message: format!("metaagent task for `{metaagent_id}` does not exist"),
            })?;
        session.touch();
        Ok(session.clone())
    }

    pub fn complete_metaagent_task(
        &mut self,
        session_id: &str,
        metaagent_id: &str,
        summary: Option<String>,
    ) -> Result<RuntimeSession, DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "complete metaagent task")?;
        session
            .complete_metaagent_task(metaagent_id, summary)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "complete_metaagent_task",
                message: format!("metaagent task for `{metaagent_id}` does not exist"),
            })?;
        session.touch();
        Ok(session.clone())
    }

    pub fn block_metaagent_task(
        &mut self,
        session_id: &str,
        metaagent_id: &str,
        reason: impl Into<String>,
    ) -> Result<RuntimeSession, DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "block metaagent task")?;
        session
            .block_metaagent_task(metaagent_id, reason)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "block_metaagent_task",
                message: format!("metaagent task for `{metaagent_id}` does not exist"),
            })?;
        session.touch();
        Ok(session.clone())
    }

    pub fn abort_metaagent_task(
        &mut self,
        session_id: &str,
        metaagent_id: &str,
        reason: Option<String>,
    ) -> Result<RuntimeSession, DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "abort metaagent task")?;
        session
            .abort_metaagent_task(metaagent_id, reason)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "abort_metaagent_task",
                message: format!("metaagent task for `{metaagent_id}` does not exist"),
            })?;
        session.touch();
        Ok(session.clone())
    }

    #[cfg(test)]
    pub(crate) fn submit_prompt(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        target_agent_id: &str,
        prompt: impl Into<String>,
        attachments: Vec<PromptAttachment>,
    ) -> Result<(RuntimeSession, PromptSubmissionOutcome), DaemonError> {
        let prompt_id = self.next_prompt_id();
        let prompt = PromptQueueItem::new(
            prompt_id,
            attachment_id,
            target_agent_id,
            prompt,
            PromptStatus::Queued,
        )
        .with_attachments(attachments);
        let session = self.get_session_mut_for_operation(session_id, "submit prompt")?;

        if !session.has_attachment(attachment_id) {
            return Err(DaemonError::AttachmentNotInSession {
                session_id: session_id.to_string(),
                attachment_id: attachment_id.to_string(),
            });
        }

        let outcome = session.submit_prompt(prompt);
        Ok((session.clone(), outcome))
    }

    pub(crate) fn mirror_agent_prompt_state(
        &mut self,
        session_id: &str,
        agent_id: &str,
        active_prompt: Option<super::PromptQueueItem>,
        queued_prompts: std::collections::VecDeque<super::PromptQueueItem>,
    ) -> Result<RuntimeSession, DaemonError> {
        let session =
            self.get_session_mut_for_operation(session_id, "mirror prompt owner state")?;
        session.mirror_agent_prompt_state(agent_id, active_prompt, queued_prompts);
        Ok(session.clone())
    }

    pub(crate) fn note_prompt_sent(
        &mut self,
        session_id: &str,
        agent_id: &str,
        timestamp_ms: u64,
    ) -> Result<RuntimeSession, DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "note prompt sent")?;
        session.note_prompt_sent_at(agent_id, timestamp_ms);
        Ok(session.clone())
    }

    #[cfg(test)]
    pub(crate) fn cancel_active_prompt(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(RuntimeSession, PromptQueueItem), DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "cancel prompt")?;
        let cancelled = session.cancel_active_prompt_only(agent_id).ok_or_else(|| {
            DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            }
        })?;
        Ok((session.clone(), cancelled))
    }

    #[cfg(test)]
    pub(crate) fn complete_active_prompt(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(RuntimeSession, super::PromptQueueItem), DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "complete prompt")?;
        let completed = session
            .complete_active_prompt_only(agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        Ok((session.clone(), completed))
    }

    #[cfg(test)]
    pub(crate) fn complete_active_prompt_only(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(RuntimeSession, super::PromptQueueItem), DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "complete prompt")?;
        let completed = session
            .complete_active_prompt_only(agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        Ok((session.clone(), completed))
    }

    #[cfg(test)]
    pub(crate) fn activate_next_queued_prompt(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(RuntimeSession, Option<super::PromptQueueItem>), DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "activate next prompt")?;
        let next = session
            .pop_next_queued_prompt(agent_id)
            .map(|prompt| session.activate_prompt(prompt));
        Ok((session.clone(), next))
    }

    #[cfg(test)]
    pub(crate) fn activate_expected_next_queued_prompt(
        &mut self,
        session_id: &str,
        agent_id: &str,
        expected_prompt_id: &str,
    ) -> Result<(RuntimeSession, Option<super::PromptQueueItem>), DaemonError> {
        let session =
            self.get_session_mut_for_operation(session_id, "activate expected next prompt")?;
        let Some(peeked) = session.peek_next_queued_prompt(agent_id) else {
            return Ok((session.clone(), None));
        };
        if peeked.id() != expected_prompt_id {
            return Err(DaemonError::LocalTransport {
                operation: "activate expected queued prompt",
                message: format!(
                    "expected queued prompt `{}` but prompt queue front was `{}`",
                    expected_prompt_id,
                    peeked.id()
                ),
            });
        }
        let next = session
            .pop_next_queued_prompt(agent_id)
            .map(|prompt| session.activate_prompt(prompt));
        Ok((session.clone(), next))
    }

    pub fn update_config(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        values: BTreeMap<String, String>,
        _requires_idle: bool,
    ) -> Result<(RuntimeSession, SessionConfigState), DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "update config")?;

        if !session.has_attachment(attachment_id) {
            return Err(DaemonError::AttachmentNotInSession {
                session_id: session_id.to_string(),
                attachment_id: attachment_id.to_string(),
            });
        }

        session.apply_config_changes(values, attachment_id);
        Ok((session.clone(), session.config_state().clone()))
    }

    pub fn store(&self) -> &SessionStore {
        &self.store
    }

    pub fn active_session_count(&self) -> usize {
        self.store.active_session_count()
    }

    pub(super) fn ensure_alias_available(
        &self,
        workspace_id: &str,
        alias: &str,
    ) -> Result<(), DaemonError> {
        if self
            .store
            .visible_non_ended_sessions()
            .any(|session| session.workspace_id() == workspace_id && session.alias() == Some(alias))
        {
            return Err(DaemonError::SessionAliasConflict {
                workspace_id: workspace_id.to_string(),
                alias: alias.to_string(),
            });
        }
        Ok(())
    }

    pub(super) fn ensure_workflow_alias_available(
        &self,
        session_id: &str,
        alias: &str,
    ) -> Result<(), DaemonError> {
        let session = self.get_session(session_id)?;
        if session
            .workflows()
            .iter()
            .any(|workflow| workflow.alias() == Some(alias))
        {
            return Err(DaemonError::WorkflowAliasConflict {
                session_id: session_id.to_string(),
                alias: alias.to_string(),
            });
        }
        Ok(())
    }

    pub(super) fn ensure_workflow_alias_available_for_update(
        &self,
        session_id: &str,
        workflow_id: &str,
        alias: &str,
    ) -> Result<(), DaemonError> {
        let session = self.get_session(session_id)?;
        if session
            .workflows()
            .iter()
            .any(|workflow| workflow.id() != workflow_id && workflow.alias() == Some(alias))
        {
            return Err(DaemonError::WorkflowAliasConflict {
                session_id: session_id.to_string(),
                alias: alias.to_string(),
            });
        }
        Ok(())
    }

    pub(super) fn ensure_session_alias_available_for_update(
        &self,
        workspace_id: &str,
        session_id: &str,
        alias: &str,
    ) -> Result<(), DaemonError> {
        if self.store.visible_non_ended_sessions().any(|session| {
            session.id() != session_id
                && session.workspace_id() == workspace_id
                && session.alias() == Some(alias)
        }) {
            return Err(DaemonError::SessionAliasConflict {
                workspace_id: workspace_id.to_string(),
                alias: alias.to_string(),
            });
        }
        Ok(())
    }

    pub(super) fn ensure_workflow_endpoint_alias_available(
        &self,
        session_id: &str,
        workflow_id: &str,
        alias: &str,
    ) -> Result<(), DaemonError> {
        let session = self.get_session(session_id)?;
        let workflow =
            session
                .workflow(workflow_id)
                .ok_or_else(|| DaemonError::WorkflowNotFound {
                    session_id: session_id.to_string(),
                    workflow_id: workflow_id.to_string(),
                })?;
        if workflow
            .endpoints()
            .iter()
            .any(|endpoint| endpoint.alias() == Some(alias))
        {
            return Err(DaemonError::WorkflowEndpointAliasConflict {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.to_string(),
                alias: alias.to_string(),
            });
        }
        Ok(())
    }

    pub(super) fn ensure_workflow_endpoint_alias_available_for_update(
        &self,
        session_id: &str,
        workflow_id: &str,
        endpoint_id: &str,
        alias: &str,
    ) -> Result<(), DaemonError> {
        let session = self.get_session(session_id)?;
        let workflow =
            session
                .workflow(workflow_id)
                .ok_or_else(|| DaemonError::WorkflowNotFound {
                    session_id: session_id.to_string(),
                    workflow_id: workflow_id.to_string(),
                })?;
        if workflow
            .endpoints()
            .iter()
            .any(|endpoint| endpoint.id() != endpoint_id && endpoint.alias() == Some(alias))
        {
            return Err(DaemonError::WorkflowEndpointAliasConflict {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.to_string(),
                alias: alias.to_string(),
            });
        }
        Ok(())
    }

    pub(super) fn get_session_mut_for_operation(
        &mut self,
        session_id: &str,
        operation: &'static str,
    ) -> Result<&mut RuntimeSession, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;

        if session.status() == SessionStatus::Ended {
            return Err(DaemonError::SessionOperationNotAllowed {
                session_id: session_id.to_string(),
                status: session.status(),
                operation,
            });
        }

        session.touch();
        Ok(session)
    }

    pub(crate) fn reserve_prompt_id(&self) -> String {
        self.prompt_id_allocator.next_prompt_id()
    }

    pub(crate) fn prompt_id_allocator(&self) -> PromptIdAllocator {
        self.prompt_id_allocator.clone()
    }

    #[cfg(test)]
    pub(super) fn next_prompt_id(&self) -> String {
        self.reserve_prompt_id()
    }
}
