//! Home-side recovery after an authoritative worker failure completion.

use super::*;

impl KernelRuntimeState {
    pub(super) async fn finish_remote_provider_failure(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
        completions: &[crate::session::PromptCompletion],
        failure: crate::app::RemoteProviderFailure,
    ) -> Result<(), DaemonError> {
        // Projection has settled the failed turn and reserved agent admission.
        // Release its workspace before worker I/O can finish that reservation
        // and dispatch queued work on the substitute.
        for completion in completions {
            if let (Some(run_id), Some(node_id)) = (
                completion.completed.workflow_run_id(),
                completion.completed.workflow_node_run_id(),
            ) {
                self.owned
                    .release_workflow_node_workspace_claim(session_id, run_id, node_id);
            }
        }
        let substituted = if let Some(reason) =
            crate::provider::classify_provider_substitutable_failure_text(
                &failure.adapter_key,
                &failure.message,
            ) {
            self.activate_substitute_after_provider_failure(
                session_id,
                agent_id,
                provider_run_id,
                &reason,
                Some(failure.profile_transition),
            )
            .await
        } else {
            false
        };
        if substituted {
            let dispatches = self
                .owned
                .workflow_maybe_start_next_queued_prompt(session_id);
            self.owned.persist_workflow_runtime_session(
                session_id,
                "remote_workflow_provider_prompt_failed",
            )?;
            self.spawn_workflow_prompt_dispatches(dispatches);
        } else {
            // Preserve queued invocations if no substitute was available or
            // worker confirmation failed. Never replay the failed invocation.
            self.owned.persist_workflow_runtime_session(
                session_id,
                "remote_workflow_provider_prompt_failed",
            )?;
        }
        Ok(())
    }
}
