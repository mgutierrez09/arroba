use super::*;

pub(crate) enum ProviderLaunchStartOutcome {
    Reused(crate::provider::RuntimeProviderRun),
    Started(crate::app::StartedProviderLaunch, u64),
}

impl KernelRuntimeState {
    pub(crate) async fn launch_provider_for_remote_lease_detached(
        &self,
        launch_request: crate::provider::LaunchProviderRequest,
    ) -> Result<crate::provider::RuntimeProviderRun, DaemonError> {
        let runtime_init_delay_ms;
        let started = {
            let owned = &self.owned;
            let config = owned.config_projection.snapshot();
            let launch_request = self
                .prepare_provider_launch_request_with_vault(
                    launch_request,
                    "launch remote lease provider run",
                )
                .await?;
            crate::logging::info_with_fields(
                "daemon.app",
                "launching remote lease provider run",
                serde_json::json!({
                    "adapter_key": launch_request.adapter_key.clone(),
                    "agent_id": launch_request.agent_id.clone(),
                    "provider": launch_request.provider.clone(),
                    "session_id": launch_request.session_id.clone(),
                }),
            );
            let started = owned.start_provider_launch(launch_request)?;
            let run = started.run.clone();
            // Lease identity must be visible before the provider's first MCP
            // request, which can arrive immediately after the PTY is spawned.
            owned
                .provider_run_projection
                .mark_leased_provider_run(run.id());
            if let Some(previous_active_run_id) = started.previous_active_run_id.as_deref() {
                if let Ok(previous_run) = owned.provider_store.get_run(previous_active_run_id) {
                    owned.provider_run_projection.update(previous_run);
                }
            }
            crate::logging::info_with_fields(
                "daemon.app",
                "prepared remote lease provider run endpoint metadata",
                serde_json::json!({
                    "provider_run_id": run.id(),
                    "endpoint_mode": run.endpoint_mode().to_string(),
                    "session_id": run.session_id(),
                    "provider": run.provider(),
                }),
            );
            if let Err(error) = self
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app)
                        .spawn_for_launch_with_credentials(&run, &started.provider_credential_env)
                })
                .await
            {
                crate::logging::error_with_fields(
                    "daemon.app",
                    "PTY spawn failed for remote lease provider run",
                    serde_json::json!({
                        "provider_run_id": run.id(),
                        "session_id": run.session_id(),
                        "error": error.to_string(),
                    }),
                );
                if let Ok(outcome) = owned
                    .provider_store
                    .terminate_run_provider_only(run.session_id(), run.id())
                {
                    let _ = owned.clear_active_provider_run_session_pointer(
                        run.session_id(),
                        outcome.run().id(),
                    );
                    owned.provider_run_projection.update(outcome.into_run());
                }
                return Err(error);
            }
            owned.provider_run_projection.update(run);
            runtime_init_delay_ms = config.provider_runtime_init_delay_ms;
            started
        };

        let accepted = started.run.clone();
        let state = self.clone();
        tokio::spawn(async move {
            if runtime_init_delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(runtime_init_delay_ms)).await;
            }
            let run = started.run.clone();
            let provider_credential_env = started.provider_credential_env.clone();
            let binding = tokio::task::spawn_blocking(move || {
                crate::provider::ProviderProcessService::initialize_runtime_binding_with_credentials(
                    &run,
                    &provider_credential_env,
                )
            })
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "initialize remote lease provider runtime",
                message: error.to_string(),
            });

            match binding {
                Ok(Ok(binding)) => state.finish_provider_launch(&started, binding).await,
                Ok(Err(error)) => state.fail_provider_launch(&started, &error).await,
                Err(error) => state.fail_provider_launch(&started, &error).await,
            }
        });
        Ok(accepted)
    }

    pub(crate) async fn launch_remote_native_provider_run(
        &self,
        request: &crate::local::LaunchProviderRunRequest,
        caller_user_id: &str,
    ) -> Result<Option<crate::local::LocalDaemonResponse>, DaemonError> {
        if !request.native_tui {
            return Ok(None);
        }
        let session = self.owned.session_store.get_session(&request.session_id)?;
        let agent_id = request
            .agent_id
            .clone()
            .or_else(|| session.focused_agent_id().map(str::to_string))
            .or_else(|| {
                self.owned
                    .agent_store
                    .get_focused_agent(&request.session_id)
                    .map(|agent| agent.id().to_string())
            });
        let Some(agent_id) = agent_id else {
            return Ok(None);
        };
        let agent = self.owned.agent_store.get_agent(&agent_id)?;
        if agent.owner_user_id() != caller_user_id {
            return Err(DaemonError::OwnershipAccessDenied {
                user_id: caller_user_id.to_string(),
                owner_user_id: agent.owner_user_id().to_string(),
                resource: format!("provider run for agent `{agent_id}`"),
                operation: "launch provider run",
            });
        }
        let Some(remote_execution) = agent.remote_execution().cloned() else {
            return Ok(None);
        };
        let required_mcps = self.required_remote_mcps_for_native_provider_launch(&agent)?;
        let required_skills = self.required_remote_skills_for_native_provider_launch(&agent)?;
        let remote_extension_manifest = self
            .remote_extension_manifest_for_agent(&agent)?
            .without_mcp_tools();
        if !required_mcps.is_empty() {
            self.ensure_remote_mcp_requirements_available_for_agent(&agent, required_mcps.clone())
                .await?;
        }
        if self.remote_agent_is_home_managed_slice(&agent) {
            self.ensure_remote_skill_packages_for_agent(&agent).await?;
        }
        let mut relay_config = self.owned.config_projection.snapshot();
        if let (Some(relay_url), Some(relay_token)) = (
            remote_execution.relay_url.clone(),
            remote_execution.relay_token.clone(),
        ) {
            relay_config.apply_remote_relay_override(relay_url, relay_token);
        }
        let leased_agent_id = remote_execution.leased_agent_id.clone();
        let response = crate::transport::relay_client::send_peer_request_via_temporary_connection(
            &relay_config,
            ClientTarget {
                daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                daemon_alias: None,
            },
            RelayPeerRequest::LaunchLeasedNativeProviderRun {
                leased_agent_id: leased_agent_id.clone(),
                adapter_key: crate::provider::adapter_key_for_provider(&request.adapter_key)
                    .to_string(),
                provider: request.provider.clone(),
                account_profile: request.account_profile.clone(),
                model: request.model.clone(),
                variant: request.variant.clone(),
                structured_endpoint: request.structured_endpoint.clone(),
                provider_session_id: request.provider_session_id.clone(),
                required_mcps,
                required_skills: Some(required_skills),
                remote_extension_manifest,
            },
        )
        .await?;
        match response {
            RelayPeerResponse::LeasedNativeProviderRunLaunched { provider_run } => {
                let home_agent_id = agent_id.clone();
                let (worker_provider_run_id, projected_run) = provider_run
                    .project_leased_for_home_agent(
                        &leased_agent_id,
                        request.session_id.clone(),
                        agent_id,
                    );
                let _ = self
                    .owned
                    .agent_store
                    .set_remote_execution_active_worker_provider_run_id(
                        &home_agent_id,
                        Some(worker_provider_run_id),
                    )?;
                self.owned
                    .provider_run_projection
                    .update(projected_run.clone());
                self.owned.session_store.set_active_provider_run(
                    &request.session_id,
                    Some(projected_run.id().to_string()),
                )?;
                let _ = self.owned.session_snapshot(&request.session_id)?;
                Ok(Some(
                    crate::local::LocalDaemonResponse::ProviderRunLaunched {
                        provider_run: projected_run,
                    },
                ))
            }
            other => Err(DaemonError::LocalTransport {
                operation: "launch remote native provider run",
                message: format!("unexpected remote native provider launch response: {other:?}"),
            }),
        }
    }

    pub(crate) async fn start_provider_launch(
        &self,
        request: crate::local::LaunchProviderRunRequest,
        caller_user_id: String,
    ) -> Result<ProviderLaunchStartOutcome, DaemonError> {
        let launch_request = self
            .owned
            .launch_provider_request_from_local_request(request);
        {
            let owned = &self.owned;
            if launch_request.owner_user_id != caller_user_id {
                return Err(DaemonError::OwnershipAccessDenied {
                    user_id: caller_user_id,
                    owner_user_id: launch_request.owner_user_id.clone(),
                    resource: format!(
                        "provider run for agent `{}`",
                        launch_request.agent_id.as_deref().unwrap_or("<focused>")
                    ),
                    operation: "launch provider run",
                });
            }
            let config = owned.config_projection.snapshot();
            let launch_request = self
                .prepare_provider_launch_request_with_vault(launch_request, "launch provider run")
                .await?;
            if let Some(run) = owned.reusable_native_tui_run_for_launch(&launch_request)? {
                return Ok(ProviderLaunchStartOutcome::Reused(run));
            }
            crate::logging::info_with_fields(
                "daemon.app",
                "launching provider run",
                serde_json::json!({
                    "adapter_key": launch_request.adapter_key.clone(),
                    "agent_id": launch_request.agent_id.clone(),
                    "provider": launch_request.provider.clone(),
                    "session_id": launch_request.session_id.clone(),
                }),
            );
            let started = owned.start_provider_launch(launch_request)?;
            let run = started.run.clone();
            if let Some(previous_active_run_id) = started.previous_active_run_id.as_deref() {
                if let Ok(previous_run) = owned.provider_store.get_run(previous_active_run_id) {
                    owned.provider_run_projection.update(previous_run);
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
            if let Err(error) = self
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app)
                        .spawn_for_launch_with_credentials(&run, &started.provider_credential_env)
                })
                .await
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
                if let Ok(outcome) = owned
                    .provider_store
                    .terminate_run_provider_only(run.session_id(), run.id())
                {
                    let _ = owned.clear_active_provider_run_session_pointer(
                        run.session_id(),
                        outcome.run().id(),
                    );
                    owned.provider_run_projection.update(outcome.into_run());
                }
                if let Some(previous_active_run_id) = started.previous_active_run_id.as_deref() {
                    let recipients = owned
                        .attachment_store
                        .list_session_attachment_ids(run.session_id());
                    match owned
                        .resume_provider_run_for_session(run.session_id(), previous_active_run_id)
                    {
                        Ok(resumed_run) => {
                            owned.record_notice(
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
                            owned.record_notice(
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
            owned.provider_run_projection.update(run);
            Ok(ProviderLaunchStartOutcome::Started(
                started,
                config.provider_runtime_init_delay_ms,
            ))
        }
    }

    pub(crate) async fn finish_provider_launch(
        &self,
        started: &crate::app::StartedProviderLaunch,
        binding: Option<crate::provider::ProviderRuntimeBinding>,
    ) {
        let _permit = self.provider_runtime_lanes.acquire(started.run.id()).await;
        self.finish_provider_launch_in_lane(started, binding).await;
    }

    async fn finish_provider_launch_in_lane(
        &self,
        started: &crate::app::StartedProviderLaunch,
        binding: Option<crate::provider::ProviderRuntimeBinding>,
    ) {
        if let Ok(run) = self.owned.provider_store.get_run(started.run.id()) {
            if provider_launch_completion_is_stale(run.state()) {
                crate::logging::info_with_fields(
                    "daemon.provider",
                    "ignoring stale provider runtime launch completion",
                    serde_json::json!({
                        "provider_run_id": run.id(),
                        "session_id": run.session_id(),
                        "state": format!("{:?}", run.state()),
                    }),
                );
                return;
            }
        }
        let mut retry_metaagent_event_dispatches = WorkflowPromptDispatches::default();
        {
            let owned = &self.owned;
            let result = owned.finish_provider_launch_success(started, binding);
            match result {
                Ok(run) => {
                    if let Some(agent_id) = run.agent_instance_id() {
                        match owned.advance_next_queued_prompt_dispatch(
                            run.session_id(),
                            agent_id,
                            run.id(),
                        ) {
                            Ok(Some(dispatch)) => {
                                if let Err(error) = self.enqueue_prompt_dispatch(&dispatch).await {
                                    let _ = self.fail_prompt_dispatch(dispatch, error).await;
                                }
                            }
                            Ok(None) => {}
                            Err(error) => {
                                self.fail_provider_launch_in_lane(started, &error).await;
                                return;
                            }
                        }
                        match owned.retry_pending_metaagent_event_prompts_for_provider_run(&run) {
                            Ok(dispatches) => {
                                retry_metaagent_event_dispatches = dispatches;
                            }
                            Err(error) => {
                                self.fail_provider_launch_in_lane(started, &error).await;
                                return;
                            }
                        }
                        let _ = owned.session_snapshot(run.session_id());
                    }
                }
                Err(error) => {
                    self.fail_provider_launch_in_lane(started, &error).await;
                }
            }
        }
        self.spawn_workflow_prompt_dispatches(retry_metaagent_event_dispatches);
    }
}

fn provider_launch_completion_is_stale(state: crate::provider::ProviderRunState) -> bool {
    state != crate::provider::ProviderRunState::Starting
}

#[cfg(test)]
mod tests {
    use super::provider_launch_completion_is_stale;
    use crate::provider::ProviderRunState;

    #[test]
    fn duplicate_or_cancelled_provider_launch_completion_is_stale() {
        assert!(!provider_launch_completion_is_stale(
            ProviderRunState::Starting
        ));
        assert!(provider_launch_completion_is_stale(
            ProviderRunState::Running
        ));
        assert!(provider_launch_completion_is_stale(
            ProviderRunState::Parked
        ));
        assert!(provider_launch_completion_is_stale(ProviderRunState::Ended));
    }
}
