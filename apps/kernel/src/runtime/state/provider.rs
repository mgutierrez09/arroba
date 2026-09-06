//! Provider-run launch, liveness, and process-tracking mutations.
//!
//! This module keeps the owned provider registry coherent with sessions, including active-run
//! launch/finish transitions, liveness reconciliation, and provider-output bookkeeping.

use super::owned::OwnedProviderRunExit;
use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn reusable_native_tui_run_for_launch(
        &self,
        request: &crate::provider::LaunchProviderRequest,
    ) -> Result<Option<crate::provider::RuntimeProviderRun>, DaemonError> {
        if request.client_interface != crate::provider::ProviderClientInterface::NativeTui {
            return Ok(None);
        }
        let Some(agent_id) = request.agent_id.as_deref() else {
            return Ok(None);
        };
        let Some(run) = self
            .provider_store
            .list_runs()
            .into_iter()
            .filter(|run| {
                run.session_id() == request.session_id
                    && run.agent_instance_id() == Some(agent_id)
                    && run.client_interface() == crate::provider::ProviderClientInterface::NativeTui
                    && matches!(
                        run.state(),
                        crate::provider::ProviderRunState::Starting
                            | crate::provider::ProviderRunState::Running
                    )
            })
            .max_by(|left, right| left.active_selection_cmp(right))
        else {
            return Ok(None);
        };

        if !request.matches_existing_run_selection(&run) {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: run.id().to_string(),
                state: run.state(),
                operation: "launch native TUI provider run with different parameters",
            });
        }

        self.session_store
            .set_active_provider_run(&request.session_id, Some(run.id().to_string()))?;
        self.provider_run_projection.update(run.clone());
        Ok(Some(run))
    }

    pub(super) fn start_provider_launch(
        &self,
        mut request: crate::provider::LaunchProviderRequest,
    ) -> Result<crate::app::StartedProviderLaunch, DaemonError> {
        let session_id = request.session_id.clone();
        let active_run_id = self
            .session_store
            .get_session(&session_id)?
            .active_provider_run_id()
            .map(str::to_owned);
        let previous_active_run_id = active_run_id;

        if let Some(active_run_id) = previous_active_run_id.as_deref() {
            let (active_run, locally_owned) =
                self.provider_run_for_activation(&session_id, active_run_id)?;
            if locally_owned && active_run.agent_instance_id() == request.agent_id.as_deref() {
                match active_run.state() {
                    crate::provider::ProviderRunState::Ended => {
                        self.session_store
                            .set_active_provider_run(&session_id, None)?;
                        self.provider_store.clear_runtime(active_run_id);
                    }
                    crate::provider::ProviderRunState::Starting => {
                        if active_run.client_interface().is_chariox() {
                            let outcome = self
                                .provider_store
                                .terminate_run_provider_only(&session_id, active_run_id)?;
                            self.clear_active_provider_run_session_pointer(
                                &session_id,
                                outcome.run().id(),
                            )?;
                            self.provider_run_projection.update(outcome.into_run());
                        }
                    }
                    crate::provider::ProviderRunState::Running => {
                        if active_run.client_interface().is_chariox()
                            && !self.provider_run_has_active_prompt(&session_id, &active_run)?
                        {
                            let outcome = self
                                .provider_store
                                .park_run_provider_only(&session_id, active_run_id)?;
                            self.clear_active_provider_run_session_pointer(
                                &session_id,
                                outcome.run().id(),
                            )?;
                            self.provider_run_projection.update(outcome.into_run());
                        }
                    }
                    crate::provider::ProviderRunState::Parked => {
                        self.session_store
                            .set_active_provider_run(&session_id, None)?;
                    }
                }
            }
        }

        let provider_credential_env = std::mem::take(&mut request.provider_credential_env);
        let outcome = self.provider_store.start_run_provider_only(request)?;
        self.session_store
            .set_active_provider_run(&session_id, Some(outcome.run().id().to_string()))?;
        Ok(crate::app::StartedProviderLaunch {
            run: outcome.into_run(),
            previous_active_run_id,
            provider_credential_env,
        })
    }

    pub(super) fn resume_provider_run_for_session(
        &self,
        session_id: &str,
        run_id: &str,
    ) -> Result<crate::provider::RuntimeProviderRun, DaemonError> {
        let active_run_id = self
            .session_store
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_owned);

        if let Some(active_run_id) = active_run_id.as_deref() {
            if active_run_id != run_id {
                let (active_run, locally_owned) =
                    self.provider_run_for_activation(session_id, active_run_id)?;
                if locally_owned {
                    match active_run.state() {
                        crate::provider::ProviderRunState::Running => {
                            if active_run.client_interface().is_chariox()
                                && !self.provider_run_has_active_prompt(session_id, &active_run)?
                            {
                                let outcome = self
                                    .provider_store
                                    .park_run_provider_only(session_id, active_run_id)?;
                                self.clear_active_provider_run_session_pointer(
                                    session_id,
                                    outcome.run().id(),
                                )?;
                                self.provider_run_projection.update(outcome.into_run());
                            }
                        }
                        crate::provider::ProviderRunState::Starting => {
                            let outcome = self
                                .provider_store
                                .terminate_run_provider_only(session_id, active_run_id)?;
                            self.clear_active_provider_run_session_pointer(
                                session_id,
                                outcome.run().id(),
                            )?;
                            self.provider_run_projection.update(outcome.into_run());
                        }
                        crate::provider::ProviderRunState::Parked
                        | crate::provider::ProviderRunState::Ended => {
                            self.session_store
                                .set_active_provider_run(session_id, None)?;
                        }
                    }
                }
            }
        }

        let (target_run, locally_owned) = self.provider_run_for_activation(session_id, run_id)?;
        if !locally_owned {
            if !matches!(
                target_run.state(),
                crate::provider::ProviderRunState::Starting
                    | crate::provider::ProviderRunState::Running
            ) {
                return Err(DaemonError::InvalidProviderRunState {
                    provider_run_id: run_id.to_string(),
                    state: target_run.state(),
                    operation: "restore projected provider run",
                });
            }
            self.session_store
                .set_active_provider_run(session_id, Some(run_id.to_string()))?;
            let _ = self.session_snapshot(session_id)?;
            return Ok(target_run);
        }

        let outcome = self
            .provider_store
            .resume_run_provider_only(session_id, run_id)?;
        self.session_store
            .set_active_provider_run(session_id, Some(outcome.run().id().to_string()))?;
        let run = outcome.into_run();
        self.provider_run_projection.update(run.clone());
        Ok(run)
    }

    fn provider_run_for_activation(
        &self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<(crate::provider::RuntimeProviderRun, bool), DaemonError> {
        let result = match self.provider_store.get_run(provider_run_id) {
            Ok(run) => Ok((run, true)),
            Err(DaemonError::ProviderRunNotFound { .. }) => self
                .provider_run_projection
                .get(provider_run_id)
                .map(|run| (run, false))
                .ok_or_else(|| DaemonError::ProviderRunNotFound {
                    provider_run_id: provider_run_id.to_string(),
                }),
            Err(error) => Err(error),
        };
        result.and_then(|(run, locally_owned)| {
            if run.session_id() == session_id {
                Ok((run, locally_owned))
            } else {
                Err(DaemonError::ProviderRunNotInSession {
                    session_id: session_id.to_string(),
                    provider_run_id: provider_run_id.to_string(),
                })
            }
        })
    }

    pub(super) fn finish_provider_launch_success(
        &self,
        started: &crate::app::StartedProviderLaunch,
        binding: Option<crate::provider::ProviderRuntimeBinding>,
    ) -> Result<crate::provider::RuntimeProviderRun, DaemonError> {
        let previous_active_run = started
            .previous_active_run_id
            .as_deref()
            .and_then(|run_id| self.provider_store.get_run(run_id).ok());
        if let Some(binding) = binding {
            self.provider_store
                .apply_runtime_binding(started.run.id(), binding)?;
        }
        let run = self.provider_store.mark_run_running(started.run.id())?;
        self.session_store
            .set_active_provider_run(run.session_id(), Some(run.id().to_string()))?;
        let _ = self.session_snapshot(run.session_id())?;
        crate::logging::info_with_fields(
            "daemon.app",
            "initializing provider runtime",
            serde_json::json!({
                "provider_run_id": run.id(),
                "session_id": run.session_id(),
            }),
        );
        crate::logging::info_with_fields(
            "daemon.app",
            "provider runtime initialized successfully",
            serde_json::json!({
                "provider_run_id": run.id(),
                "session_id": run.session_id(),
            }),
        );
        let _ = self.provider_store.record_run_activity(run.id());
        if let Some(agent_id) = run.agent_instance_id() {
            self.agent_store.set_agent_runtime_profile_durably(
                &self.durable_state_store,
                agent_id,
                run.provider(),
                Some(run.model().to_string()),
                run.variant().map(str::to_string),
                Some(run.account_profile().to_string()),
                run.resume_state().clone(),
                Some(run.id()),
                None,
            )?;
        }
        if let Some(previous_active_run) = previous_active_run.as_ref() {
            self.prepare_provider_switch_context_handoff(previous_active_run, &run);
        }
        self.provider_run_projection.update(run.clone());
        Ok(run)
    }

    pub(super) fn ensure_provider_run_in_session(
        &self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<crate::provider::RuntimeProviderRun, DaemonError> {
        let provider_run = self.provider_store.get_run(provider_run_id)?;
        if provider_run.session_id() != session_id {
            return Err(DaemonError::ProviderRunNotInSession {
                session_id: session_id.to_string(),
                provider_run_id: provider_run_id.to_string(),
            });
        }
        Ok(provider_run)
    }

    pub(super) fn remove_provider_process_tracking_for_run(
        &self,
        provider_run_id: &str,
        pty_process_key: Option<String>,
    ) {
        self.workspace_identity_monitor
            .remove_provider_run(provider_run_id);
        let process_key = self
            .provider_process_tracking
            .read()
            .run_processes
            .get(provider_run_id)
            .cloned()
            .or(pty_process_key);
        let Some(process_key) = process_key else {
            return;
        };
        let mut tracking = self.provider_process_tracking.write();
        tracking.run_processes.remove(provider_run_id);
        let should_remove_entry = if let Some(entry) = tracking.processes.get_mut(&process_key) {
            entry
                .owner_provider_run_ids
                .retain(|id| id != provider_run_id);
            entry.owner_provider_run_ids.is_empty()
        } else {
            false
        };
        if should_remove_entry {
            tracking.processes.remove(&process_key);
        }
    }

    pub(super) fn reconcile_provider_run_liveness_provider_phase(
        &self,
        session_id: &str,
        provider_run_id: &str,
        process_running: Option<bool>,
    ) -> Result<Option<OwnedProviderRunExit>, DaemonError> {
        let provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;
        let _ = provider_run
            .agent_instance_id()
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: "provider run has no agent".to_string(),
            })?;
        let reconciliation = self.provider_store.reconcile_run_liveness_provider_only(
            session_id,
            provider_run_id,
            process_running,
        )?;
        match reconciliation {
            crate::provider::ProviderRunLivenessReconciliation::AlreadyEnded(run) => {
                self.clear_active_provider_run_session_pointer(session_id, provider_run_id)?;
                self.provider_run_projection.update(run.clone());
                Ok(Some(OwnedProviderRunExit {
                    ended_run: run,
                    already_ended: true,
                }))
            }
            crate::provider::ProviderRunLivenessReconciliation::NewlyEnded(run) => {
                self.clear_active_provider_run_session_pointer(session_id, provider_run_id)?;
                self.provider_run_projection.update(run.clone());
                Ok(Some(OwnedProviderRunExit {
                    ended_run: run,
                    already_ended: false,
                }))
            }
            crate::provider::ProviderRunLivenessReconciliation::ExternalEndpoint(_)
            | crate::provider::ProviderRunLivenessReconciliation::StillRunning(_) => Ok(None),
        }
    }
}
