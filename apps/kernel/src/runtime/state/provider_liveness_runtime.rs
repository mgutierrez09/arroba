//! Provider process liveness reconciliation and unexpected-exit settlement.

use super::*;

impl KernelRuntimeState {
    pub(super) async fn reconcile_provider_run_exit(
        &self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        let owned = &self.owned;

        if let Some(exit) = owned.reconcile_provider_run_liveness_provider_phase(
            session_id,
            provider_run_id,
            None,
        )? {
            let (_, process_key) = self
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(provider_run_id)
                })
                .await
                .unwrap_or((false, None));
            owned.remove_provider_process_tracking_for_run(provider_run_id, process_key);
            self.owned
                .connector_adapter_processes
                .shutdown_run(provider_run_id)
                .await;
            let session_outcome = self
                .settle_owned_provider_prompt(session_id, provider_run_id, false, false, true)
                .await?;
            if session_outcome.had_active_prompt && !session_outcome.cancelled_prompt {
                let recipients = owned
                    .attachment_store
                    .list_session_attachment_ids(session_id);
                owned.record_notice(
                    session_id,
                    Some(provider_run_id),
                    recipients,
                    format!(
                        "Provider run `{}` for `{}` was already ended during liveness reconciliation. {}",
                        provider_run_id,
                        exit.ended_run.provider(),
                        if session_outcome.started_next_prompt {
                            "The active prompt was closed and Chariox advanced the queued backlog onto the next available provider run."
                        } else {
                            "The active prompt was closed without starting the queued backlog."
                        }
                    ),
                );
            }
            return Ok(exit.already_ended);
        }

        let process_running = self
            .with_app_side_effect(|app| {
                crate::app::ProviderLaunchProcessRuntime::new(app).poll_running(provider_run_id)
            })
            .await?;
        let Some(exit) = owned.reconcile_provider_run_liveness_provider_phase(
            session_id,
            provider_run_id,
            Some(process_running),
        )?
        else {
            return Ok(false);
        };
        let (_, process_key) = self
            .with_app_side_effect(|app| {
                crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(provider_run_id)
            })
            .await
            .unwrap_or((false, None));
        owned.remove_provider_process_tracking_for_run(provider_run_id, process_key);
        self.owned
            .connector_adapter_processes
            .shutdown_run(provider_run_id)
            .await;
        if exit.already_ended {
            let _ = self
                .settle_owned_provider_prompt(session_id, provider_run_id, false, false, true)
                .await?;
            return Ok(true);
        }

        let agent_id =
            exit.ended_run
                .agent_instance_id()
                .ok_or_else(|| DaemonError::AgentNotFound {
                    agent_id: "provider run has no agent".to_string(),
                })?;
        let session_outcome = self
            .settle_unexpected_provider_run_exit(session_id, provider_run_id, agent_id)
            .await?;
        if session_outcome.cancelled_prompt {
            return Ok(true);
        }
        let recipients = owned
            .attachment_store
            .list_session_attachment_ids(session_id);
        owned.record_notice(
            session_id,
            Some(provider_run_id),
            recipients,
            format!(
                "Provider run `{}` for `{}` ended unexpectedly. {}",
                provider_run_id,
                exit.ended_run.provider(),
                if session_outcome.had_active_prompt {
                    if session_outcome.started_next_prompt {
                        "The active prompt was closed and Chariox advanced the queued backlog onto the next available provider run."
                    } else {
                        "The active prompt was closed without starting the queued backlog."
                    }
                } else {
                    "No active prompt was running."
                }
            ),
        );
        Ok(true)
    }

    pub(super) async fn settle_unexpected_provider_run_exit(
        &self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
    ) -> Result<crate::app::ProviderRunExitSessionSummary, DaemonError> {
        let session_outcome = self
            .settle_owned_provider_prompt(session_id, provider_run_id, false, false, true)
            .await?;
        if !session_outcome.cancelled_prompt
            && self
                .owned
                .agent_store
                .mark_unexpected_provider_exit_error(agent_id, session_outcome.had_active_prompt)?
        {
            let _ = self.owned.session_snapshot(session_id)?;
        }
        Ok(session_outcome)
    }
}
