//! Workflow prompt cancellation and provider-failure transitions.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_cancel_prompt(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<(), DaemonError> {
        let (Some(workflow_run_id), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        else {
            return Ok(());
        };
        let already_interrupted = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_id)
            .is_ok_and(|run| {
                matches!(
                    run.status(),
                    crate::session::WorkflowRunStatus::Paused
                        | crate::session::WorkflowRunStatus::Stopped
                )
            });
        if already_interrupted {
            let _ = self.release_workflow_node_workspace_claim(
                session_id,
                workflow_run_id,
                workflow_node_run_id,
            );
            let _ = self.session_snapshot(session_id)?;
            return Ok(());
        }
        let workflow_run = self.session_store.write().stop_workflow_node_run(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        )?;
        let _ = self.release_workflow_node_workspace_claim(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        );
        self.workflow_record_failure(
            session_id,
            workflow_run_id,
            &crate::session::WorkflowFailureEvent::new(
                crate::session::WorkflowFailureKind::RunStopped,
                workflow_node_run_id,
                Vec::new(),
                "workflow node run was stopped before validated completion",
            ),
        );
        self.record_notice(
            session_id,
            None,
            self.attachment_store
                .list_session_attachment_ids(session_id),
            format!("Workflow run `{}` was stopped.", workflow_run.id()),
        );
        self.workflow_maybe_start_next_queued_prompt(session_id);
        self.persist_workflow_runtime_session(session_id, "workflow_prompt_cancelled")?;
        Ok(())
    }

    pub(super) fn workflow_fail_provider_prompt(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
        provider_run_id: Option<&str>,
        message: &str,
    ) -> Result<WorkflowPromptDispatches, DaemonError> {
        if prompt.workflow_run_id().is_none() || prompt.workflow_node_run_id().is_none() {
            return Ok(WorkflowPromptDispatches::default());
        }
        self.workflow_fail_provider_prompt_state(session_id, prompt, provider_run_id, message)?;
        let dispatches = self.workflow_maybe_start_next_queued_prompt(session_id);
        self.persist_workflow_runtime_session(session_id, "workflow_provider_prompt_failed")?;
        Ok(dispatches)
    }

    pub(super) fn workflow_fail_provider_prompt_without_queue_advance(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
        provider_run_id: Option<&str>,
        message: &str,
    ) -> Result<bool, DaemonError> {
        if prompt.workflow_run_id().is_none() || prompt.workflow_node_run_id().is_none() {
            return Ok(false);
        }
        let released_claim =
            self.workflow_fail_provider_prompt_state(session_id, prompt, provider_run_id, message)?;
        self.persist_workflow_runtime_session(session_id, "workflow_provider_prompt_failed")?;
        Ok(released_claim)
    }

    fn workflow_fail_provider_prompt_state(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
        provider_run_id: Option<&str>,
        message: &str,
    ) -> Result<bool, DaemonError> {
        let (Some(workflow_run_id), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        else {
            return Ok(false);
        };
        self.workflow_record_failure(
            session_id,
            workflow_run_id,
            &crate::session::WorkflowFailureEvent::new(
                crate::session::WorkflowFailureKind::ProviderFailure,
                workflow_node_run_id,
                Vec::new(),
                message,
            ),
        );
        let workflow_run = self.session_store.write().fail_workflow_node_run(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        )?;
        let released_claim = self.release_workflow_node_workspace_claim(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        );
        self.record_notice(
            session_id,
            provider_run_id,
            self.attachment_store
                .list_session_attachment_ids(session_id),
            format!(
                "Workflow run `{}` failed after provider turn failure: {}",
                workflow_run.id(),
                message
            ),
        );
        Ok(released_claim)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::KernelSessionService;
    use crate::config::DaemonConfig;
    use crate::session::{CreateSessionRequest, PromptQueueItem, PromptStatus};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    async fn owned_runtime_state(app: &Arc<Mutex<DaemonApp>>) -> KernelRuntimeState {
        let (
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        ) = {
            let app = app.lock().await;
            (
                app.config_projection_store(),
                app.session_state_store(),
                app.agents().clone(),
                app.attachments().clone(),
                app.providers().clone(),
                app.provider_process_tracking_store(),
                app.slices(),
                app.session_state_projection_store(),
                app.provider_run_projection_store(),
                app.operational_history_store(),
                app.durable_state_store(),
                app.prompt_state_owner(),
                app.active_turn_store(),
                app.prompt_activity_store(),
                app.prompt_workspace_claim_store(),
                app.structured_output_record_store(),
                app.terminal_stream_store(),
                app.workflow_design_event_store(),
                app.metaagent_event_store(),
                app.workspace_coordinator(),
            )
        };
        KernelRuntimeState::new_with_owned_state(
            Arc::clone(app),
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        )
    }

    #[tokio::test]
    async fn workflow_provider_failure_persists_terminal_run_state_for_restart() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "workspace-workflow-failure",
                "worktree-workflow-failure",
            ))
            .expect("session should create");
        let workflow = app
            .sessions_mut()
            .create_workflow(session.id(), Some("failure-test".to_string()))
            .expect("workflow should create");
        let node = app
            .sessions_mut()
            .add_workflow_node(session.id(), workflow.id(), agent.id())
            .expect("node should create");
        let endpoint = app
            .sessions_mut()
            .create_workflow_endpoint(
                session.id(),
                workflow.id(),
                node.id(),
                Some("entry".to_string()),
            )
            .expect("endpoint should create");
        let workflow_run = app
            .sessions_mut()
            .invoke_workflow_endpoint(
                session.id(),
                workflow.id(),
                endpoint.id(),
                Some("fail visibly".to_string()),
            )
            .expect("workflow run should create");
        let node_run_id = workflow_run.node_runs()[0].id().to_string();
        let prompt = PromptQueueItem::new(
            "prompt-failure",
            "attachment-failure",
            agent.id(),
            "fail visibly",
            PromptStatus::Running,
        )
        .with_workflow_context(workflow_run.id(), &node_run_id);
        app.sessions_mut()
            .enqueue_workflow_prompt(
                session.id(),
                workflow.id(),
                endpoint.id(),
                Some("review the next exact revision".to_string()),
                None,
                crate::session::WorkflowQueuedPromptSource::Manual,
                None,
            )
            .expect("a subsequent workflow invocation should queue");
        let session_id = session.id().to_string();
        let workflow_run_id = workflow_run.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;

        let dispatches = runtime
            .owned
            .workflow_fail_provider_prompt(&session_id, &prompt, None, "provider unavailable")
            .expect("provider failure should settle workflow");
        assert!(
            !dispatches.is_empty(),
            "the failure transition must return the queued invocation's provider launch to its runtime caller: local={}, remote={}, starting_runs={}, starting_meta={}, admitted={}",
            dispatches.local.len(),
            dispatches.remote.len(),
            dispatches.starting_provider_runs.len(),
            dispatches.starting_metaagent_tasks.len(),
            dispatches.admitted_workflow_prompt,
        );
        assert_eq!(dispatches.starting_provider_runs.len(), 1);

        runtime
            .owned
            .durable_state_store
            .load_events_by_kind("workflow.runtime.updated")
            .expect("durable workflow events should load")
            .into_iter()
            .rev()
            .find(|event| {
                event.subject_id.as_deref() == Some(session_id.as_str())
                    && event
                        .payload
                        .get("reason")
                        .and_then(serde_json::Value::as_str)
                        == Some("workflow_provider_prompt_failed")
            })
            .expect("workflow provider failure should persist a bounded transition");
        let durable_run = runtime
            .owned
            .durable_state_store
            .resolve_workflow_run(session.host_daemon_id(), &session_id, &workflow_run_id)
            .expect("durable workflow run should load")
            .expect("durable workflow run should exist");
        assert_eq!(
            durable_run.status(),
            crate::session::WorkflowRunStatus::Failed,
        );
    }
}
