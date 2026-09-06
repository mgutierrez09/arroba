use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderProcessService, ProviderRuntimeBinding,
    RuntimeProviderRun,
};

use super::provider_activation::ProviderRunActivationState;
pub(crate) use super::provider_activation::StartedProviderLaunch;
use super::provider_launch_policy::failed_provider_resume_state_replacement;
use super::provider_liveness::clear_active_provider_run_session_pointer;
pub(crate) use super::provider_liveness::ProviderRunLivenessRuntime;
pub(crate) use super::provider_processes::ProviderProcessTracker;

impl DaemonApp {
    pub(crate) fn end_provider_run_for_workflow_context_flush(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let Some(run) = self.providers().get_run_for_agent(session_id, agent_id) else {
            return Ok(());
        };
        if run.state() == crate::provider::ProviderRunState::Ended {
            return Ok(());
        }
        let ended = self
            .providers()
            .terminate_run_provider_only(session_id, run.id())?
            .into_run();
        clear_active_provider_run_session_pointer(self, session_id, ended.id())?;
        let _ = ProviderProcessTracker::new(self).remove_run(ended.id());
        self.update_provider_run_projection(ended);
        Ok(())
    }

    pub(crate) fn start_provider_launch(
        &mut self,
        request: LaunchProviderRequest,
    ) -> Result<StartedProviderLaunch, DaemonError> {
        self.start_provider_launch_with_options(request, false, false)
    }

    #[cfg(test)]
    pub(crate) fn start_workflow_provider_launch(
        &mut self,
        request: LaunchProviderRequest,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        self.start_workflow_provider_launch_for_node(request, None)
    }

    pub(crate) fn start_workflow_provider_launch_for_node(
        &mut self,
        request: LaunchProviderRequest,
        workflow_node_run_id: Option<&str>,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let started = self.start_provider_launch_with_options(request, false, true)?;
        if let Some(workflow_node_run_id) = workflow_node_run_id {
            self.providers
                .mark_workflow_fresh_context(started.run.id(), workflow_node_run_id)?;
        }
        let binding = match ProviderProcessService::initialize_runtime_binding_with_credentials(
            &started.run,
            &started.provider_credential_env,
        ) {
            Ok(binding) => binding,
            Err(error) => {
                self.fail_provider_launch(&started, &error);
                return Err(error);
            }
        };
        if let Err(error) = self.finish_provider_launch(&started, binding) {
            self.fail_provider_launch(&started, &error);
            return Err(error);
        }
        self.providers.get_run(started.run.id())
    }

    fn start_provider_launch_with_lease(
        &mut self,
        request: LaunchProviderRequest,
        leased: bool,
    ) -> Result<StartedProviderLaunch, DaemonError> {
        self.start_provider_launch_with_options(request, leased, false)
    }

    fn start_provider_launch_with_options(
        &mut self,
        mut request: LaunchProviderRequest,
        leased: bool,
        workflow_tools_enabled: bool,
    ) -> Result<StartedProviderLaunch, DaemonError> {
        request = self.prepare_app_provider_launch_request(request, "launch provider run")?;
        crate::logging::info_with_fields(
            "daemon.app",
            "launching provider run",
            serde_json::json!({
                "adapter_key": request.adapter_key.clone(),
                "agent_id": request.agent_id.clone(),
                "provider": request.provider.clone(),
                "session_id": request.session_id.clone(),
            }),
        );
        let request_session_id = request.session_id.clone();
        let recipients = self
            .attachments
            .list_session_attachment_ids(&request_session_id);
        let started = ProviderRunActivationState::start_provider_run_for_session(self, request)?;
        let run = if workflow_tools_enabled {
            self.providers.enable_workflow_tools(started.run.id())?
        } else {
            started.run.clone()
        };
        // Publish lease identity before spawning the provider process. The provider's
        // first MCP tools/list can happen during PTY startup, before launch_provider
        // returns to the caller.
        if leased {
            self.mark_leased_provider_run(run.id());
            #[cfg(test)]
            self.provider_run_projection_store()
                .notify_leased_provider_run_pre_spawn(run.id());
        }
        if let Some(previous_active_run_id) = started.previous_active_run_id.as_deref() {
            if let Ok(previous_run) = self.providers.get_run(previous_active_run_id) {
                self.update_provider_run_projection(previous_run);
            }
        }
        crate::logging::info_with_fields(
            "daemon.app",
            "prepared provider run endpoint metadata",
            serde_json::json!({
                "provider_run_id": run.id(),
                "endpoint_mode": run.endpoint_mode().to_string(),
                "session_id": run.session_id(),
                "provider": run.provider(),
            }),
        );
        if run.endpoint_mode() == AgentEndpointMode::Managed {
            if let Err(error) = self
                .pty
                .spawn_for_run_with_credentials(&run, &started.provider_credential_env)
            {
                crate::logging::error_with_fields(
                    "daemon.app",
                    "PTY spawn failed for provider run",
                    serde_json::json!({
                        "provider_run_id": run.id(),
                        "session_id": run.session_id(),
                        "error": error.to_string(),
                    }),
                );
                if let Ok(outcome) = self
                    .providers
                    .terminate_run_provider_only(run.session_id(), run.id())
                {
                    clear_active_provider_run_session_pointer(
                        self,
                        run.session_id(),
                        outcome.run().id(),
                    )?;
                    self.update_provider_run_projection(outcome.into_run());
                }
                if let Some(previous_active_run_id) = started.previous_active_run_id.as_deref() {
                    match ProviderRunActivationState::resume_provider_run_for_session(
                        self,
                        run.session_id(),
                        previous_active_run_id,
                    ) {
                        Ok(resumed_run) => {
                            self.record_notice(
                                run.session_id(),
                                Some(resumed_run.id()),
                                recipients,
                                format!(
                                    "Provider switch failed for session `{}`. Chariox resumed the previous provider run `{}` automatically.",
                                    run.session_id(),
                                    resumed_run.id()
                                ),
                            );
                        }
                        Err(resume_error) => {
                            self.record_notice(
                                run.session_id(),
                                None,
                                recipients,
                                format!(
                                    "Provider switch failed for session `{}` and Chariox could not resume the previous provider run: {}",
                                    run.session_id(),
                                    resume_error
                                ),
                            );
                        }
                    }
                }
                return Err(error);
            }
            ProviderProcessTracker::new(self).register_managed_run(&run)?;
        }
        self.update_provider_run_projection(run.clone());
        Ok(started)
    }

    pub(crate) fn finish_provider_launch(
        &mut self,
        started: &StartedProviderLaunch,
        binding: Option<ProviderRuntimeBinding>,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        if let Some(binding) = binding {
            self.providers
                .apply_runtime_binding(started.run.id(), binding)?;
        }
        self.finish_provider_launch_success(&started.run)
    }

    pub(crate) fn fail_provider_launch(
        &mut self,
        started: &StartedProviderLaunch,
        error: &DaemonError,
    ) {
        crate::logging::error_with_fields(
            "daemon.app",
            "provider runtime initialization failed",
            serde_json::json!({
                "provider_run_id": started.run.id(),
                "session_id": started.run.session_id(),
                "error": error.to_string(),
            }),
        );
        let recipients = self
            .attachments
            .list_session_attachment_ids(started.run.session_id());
        self.record_notice(
            started.run.session_id(),
            Some(started.run.id()),
            recipients,
            format!(
                "Provider launch `{}` failed before it became ready: {}",
                started.run.id(),
                error
            ),
        );
        let diagnostic = format!(
            "Provider launch `{}` failed before it became ready: {}",
            started.run.id(),
            error
        );
        if let Ok(run) = self
            .providers
            .record_terminal_diagnostic(started.run.id(), diagnostic.clone())
        {
            self.update_provider_run_projection(run);
        }
        match self.clear_failed_provider_resume_state(started, error) {
            Ok(true) => {
                self.provider_launch_failure_retries.clear(started.run.id());
            }
            Ok(false) => {
                crate::logging::warn_with_fields(
                    "daemon.app",
                    "provider launch failure was superseded by a newer durable resume",
                    serde_json::json!({
                        "provider_run_id": started.run.id(),
                        "session_id": started.run.session_id(),
                    }),
                );
                self.provider_launch_failure_retries.clear(started.run.id());
            }
            Err(clear_error) => {
                crate::logging::error_with_fields(
                    "durable_state.recovery",
                    "failed to durably clear invalid provider resume",
                    serde_json::json!({
                        "provider_run_id": started.run.id(),
                        "session_id": started.run.session_id(),
                        "error": clear_error.to_string(),
                    }),
                );
                if crate::durable_state::is_retryable_durable_write_error(&clear_error) {
                    self.provider_launch_failure_retries.schedule_initial(
                        started,
                        error,
                        crate::session::unix_epoch_ms(),
                    );
                }
                return;
            }
        }
        if let Some(agent_id) = started.run.agent_instance_id() {
            if let Ok(Some(active_prompt)) =
                self.prompt_owner_active_prompt_for_agent(started.run.session_id(), agent_id)
            {
                if active_prompt.durable_delivery_phase().is_none() {
                    if active_prompt.workflow_run_id().is_some() {
                        let _ = crate::scheduler::runtime::on_workflow_provider_failure(
                            self,
                            started.run.session_id(),
                            &active_prompt,
                            Some(started.run.id()),
                            &diagnostic,
                        );
                    }
                    let _ = self.complete_active_prompt(
                        started.run.session_id(),
                        agent_id,
                        Some(started.run.id()),
                    );
                }
            }
        }
        let _ = ProviderProcessTracker::new(self).remove_run(started.run.id());
        self.providers.clear_runtime(started.run.id());
        if let Ok(outcome) = self
            .providers
            .terminate_run_provider_only(started.run.session_id(), started.run.id())
        {
            clear_active_provider_run_session_pointer(
                self,
                started.run.session_id(),
                outcome.run().id(),
            )
            .ok();
            self.update_provider_run_projection(outcome.into_run());
        }
        if let Some(previous_active_run_id) = started.previous_active_run_id.as_deref() {
            let _ = ProviderRunActivationState::resume_provider_run_for_session(
                self,
                started.run.session_id(),
                previous_active_run_id,
            );
        }
        let _ = crate::app::KernelSessionReadService::new(self)
            .session_snapshot(started.run.session_id());
    }

    fn clear_failed_provider_resume_state(
        &mut self,
        started: &StartedProviderLaunch,
        error: &DaemonError,
    ) -> Result<bool, DaemonError> {
        if failed_provider_resume_state_replacement(&started.run, error).is_none() {
            return Ok(true);
        }
        let Some(agent_id) = started.run.agent_instance_id() else {
            return Ok(true);
        };
        let provider = started.run.adapter_key();
        let Some(stale_provider_session_id) = started
            .run
            .resume_state()
            .provider_session_id(provider)
            .map(str::to_string)
        else {
            return Ok(true);
        };
        match self.agents.clear_provider_resume_state_durably_if_matches(
            &self.durable_state,
            agent_id,
            provider,
            &stale_provider_session_id,
            started.run.id(),
            "failed_provider_resume_state_cleared",
        )? {
            crate::agent::ProviderResumeClearOutcome::Cleared => {}
            crate::agent::ProviderResumeClearOutcome::AlreadyAbsent => return Ok(true),
            crate::agent::ProviderResumeClearOutcome::Superseded { .. } => return Ok(false),
        }
        self.record_notice(
            started.run.session_id(),
            Some(started.run.id()),
            self.attachments
                .list_session_attachment_ids(started.run.session_id()),
            crate::provider::provider_resume_failure_notice(provider, &stale_provider_session_id)
                .unwrap_or_else(|| {
                    format!(
                        "Provider session `{stale_provider_session_id}` is no longer available. Chariox cleared it from the agent profile so the next prompt can start a new durable provider session."
                    )
                }),
        );
        Ok(true)
    }

    fn finish_provider_launch_success(
        &mut self,
        run: &RuntimeProviderRun,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let run = self.providers.mark_run_running(run.id())?;
        self.sessions
            .set_active_provider_run(run.session_id(), Some(run.id().to_string()))?;
        crate::app::KernelSessionReadService::new(self).session_snapshot(run.session_id())?;
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
        let _ = self.providers.record_run_activity(run.id());
        if let Some(agent_id) = run.agent_instance_id() {
            self.agents.set_agent_runtime_profile_durably(
                &self.durable_state,
                agent_id,
                run.provider(),
                Some(run.model().to_string()),
                run.variant().map(str::to_string),
                Some(run.account_profile().to_string()),
                run.resume_state().clone(),
                Some(run.id()),
                None,
            )?;
            let _ = self.advance_next_queued_prompt(run.session_id(), agent_id)?;
            crate::app::KernelSessionReadService::new(self).session_snapshot(run.session_id())?;
        }
        self.update_provider_run_projection(run.clone());
        Ok(run)
    }

    pub fn launch_provider(
        &mut self,
        request: LaunchProviderRequest,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        self.launch_provider_with_lease(request, false)
    }

    pub(crate) fn launch_leased_provider(
        &mut self,
        request: LaunchProviderRequest,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        self.launch_provider_with_lease(request, true)
    }

    fn launch_provider_with_lease(
        &mut self,
        request: LaunchProviderRequest,
        leased: bool,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let prepared =
            self.prepare_app_provider_launch_request(request.clone(), "launch provider run")?;
        if let Some(run) =
            ProviderRunActivationState::reusable_native_tui_run_for_launch(self, &prepared)?
        {
            if leased {
                self.mark_leased_provider_run(run.id());
            }
            return Ok(run);
        }
        let started = if leased {
            self.start_provider_launch_with_lease(request, true)?
        } else {
            self.start_provider_launch(request)?
        };
        let binding = match ProviderProcessService::initialize_runtime_binding_with_credentials(
            &started.run,
            &started.provider_credential_env,
        ) {
            Ok(binding) => binding,
            Err(error) => {
                self.fail_provider_launch(&started, &error);
                return Err(error);
            }
        };
        if let Err(error) = self.finish_provider_launch(&started, binding) {
            self.fail_provider_launch(&started, &error);
            return Err(error);
        }
        self.providers.get_run(started.run.id())
    }

    pub(crate) fn launch_provider_detached(
        &mut self,
        request: LaunchProviderRequest,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        self.launch_provider_detached_with_options(request, false)
    }

    fn launch_provider_detached_with_options(
        &mut self,
        mut request: LaunchProviderRequest,
        workflow_tools_enabled: bool,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        request = self.prepare_app_provider_launch_request(request, "launch provider run")?;
        let provider_credential_env = std::mem::take(&mut request.provider_credential_env);
        let initial_run = self.providers.launch_run_detached(request)?;
        let run_id = initial_run.id().to_string();
        let launch_result = (|| {
            let mut run = initial_run.clone();
            if workflow_tools_enabled {
                run = self.providers.enable_workflow_tools(run.id())?;
            }
            self.update_provider_run_projection(run.clone());
            if run.endpoint_mode() == AgentEndpointMode::Managed {
                self.pty
                    .spawn_for_run_with_credentials(&run, &provider_credential_env)?;
                ProviderProcessTracker::new(self).register_managed_run(&run)?;
            }
            self.providers
                .initialize_runtime_with_credentials(&run, &provider_credential_env)?;
            let run = self.providers.get_run(run.id())?;
            self.sessions
                .set_active_provider_run(run.session_id(), Some(run.id().to_string()))?;
            let _ = self.providers.record_run_activity(run.id());
            if let Some(agent_id) = run.agent_instance_id() {
                self.agents.set_agent_runtime_profile_durably(
                    &self.durable_state,
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
            self.update_provider_run_projection(run.clone());
            Ok(run)
        })();
        if let Err(error) = &launch_result {
            let started = StartedProviderLaunch {
                run: self
                    .providers
                    .get_run(&run_id)
                    .unwrap_or_else(|_| initial_run.clone()),
                previous_active_run_id: None,
                provider_credential_env: Default::default(),
            };
            self.fail_provider_launch(&started, error);
        }
        launch_result
    }
}

#[cfg(test)]
mod tests;
