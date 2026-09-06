use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn launch_provider_request_from_local_request(
        &self,
        request: crate::local::LaunchProviderRunRequest,
    ) -> crate::provider::LaunchProviderRequest {
        let adapter_key =
            crate::provider::adapter_key_for_provider(&request.adapter_key).to_string();
        let mut launch_request = crate::provider::LaunchProviderRequest::new(
            request.session_id.clone(),
            adapter_key,
            request.provider,
            request.account_profile,
            request.model,
        )
        .with_variant(request.variant);
        if let Some(endpoint) = request.structured_endpoint {
            launch_request = launch_request.with_structured_endpoint(endpoint);
        }
        if request.native_tui {
            launch_request = launch_request
                .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
        }
        if let Some(provider_session_id) = request.provider_session_id {
            let resume_state = crate::provider::ProviderResumeState::from_external_provider_session(
                &launch_request.adapter_key,
                provider_session_id,
            );
            if !resume_state.is_empty() {
                launch_request = launch_request.with_resume_state(resume_state);
            }
        }
        let config = self.config_projection.snapshot();
        let session = self.session_store.get_session(&request.session_id).ok();
        let workspace_live_sync_mode =
            crate::provider::provider_workspace_live_sync_mode_for_session(
                &launch_request.provider,
                &config,
                session.as_ref(),
            );
        launch_request = launch_request.with_workspace_live_sync_mode(workspace_live_sync_mode);
        if let Some(agent_id) = request.agent_id.clone().or_else(|| {
            self.session_store
                .get_session(&request.session_id)
                .ok()
                .and_then(|session| session.focused_agent_id().map(str::to_string))
                .or_else(|| {
                    self.agent_store
                        .get_focused_agent(&request.session_id)
                        .map(|agent| agent.id().to_string())
                })
        }) {
            launch_request = if let Ok(agent) = self.agent_store.get_agent(&agent_id) {
                let session = self.session_store.get_session(&request.session_id).ok();
                let effective_config = session
                    .as_ref()
                    .map(|session| {
                        crate::session::effective_agent_execution_config(session, Some(&agent))
                    })
                    .unwrap_or_default();
                launch_request
                    .with_agent_id(agent_id)
                    .with_owner_user_id(agent.owner_user_id().to_string())
                    .with_execution_mode(effective_config.mode)
                    .with_permission_level(effective_config.permission_level)
            } else {
                launch_request.with_agent_id(agent_id)
            };
        } else {
            let session = self.session_store.get_session(&request.session_id).ok();
            let effective_config = session
                .as_ref()
                .map(|session| crate::session::effective_agent_execution_config(session, None))
                .unwrap_or_default();
            launch_request = launch_request
                .with_execution_mode(effective_config.mode)
                .with_permission_level(effective_config.permission_level);
        }
        launch_request
    }

    pub(super) fn prepare_provider_launch_request(
        &self,
        mut request: crate::provider::LaunchProviderRequest,
        runtime_mcp_url: String,
    ) -> Result<crate::provider::LaunchProviderRequest, DaemonError> {
        request.adapter_key =
            crate::provider::adapter_key_for_provider(&request.adapter_key).to_string();
        let session = self.session_store.get_session(&request.session_id)?;
        let config = self.config_projection.snapshot();
        if request.agent_id.is_none() {
            request.agent_id = self
                .session_store
                .get_session(&request.session_id)?
                .focused_agent_id()
                .map(str::to_string)
                .or_else(|| {
                    self.agent_store
                        .get_focused_agent(&request.session_id)
                        .map(|agent| agent.id().to_string())
                });
        }
        let agent = request
            .agent_id
            .as_deref()
            .and_then(|agent_id| self.agent_store.get_agent(agent_id).ok());
        if let Some(agent) = agent.as_ref() {
            if agent.session_id() != session.id() {
                return Err(DaemonError::AgentNotInSession {
                    session_id: session.id().to_string(),
                    agent_id: agent.id().to_string(),
                });
            }
            if agent.remote_execution().is_some() {
                return Err(DaemonError::LocalTransport {
                    operation: "launch provider run",
                    message: format!(
                        "agent `{}` is remote-backed and must launch its provider on the worker kernel",
                        agent.id()
                    ),
                });
            }
            request = request.with_owner_user_id(agent.owner_user_id().to_string());
        } else {
            request = request.with_owner_user_id(session.owner_user_id().to_string());
        }
        if crate::provider::canonical_provider_family(&request.provider)
            .is_some_and(|provider| matches!(provider, "codex" | "claude" | "opencode"))
        {
            let account_owner_user_id =
                crate::account_profile::provider_account_authority_owner_user_id(
                    &config,
                    &request.owner_user_id,
                );
            let profile = self.provider_account_profiles.get(
                &account_owner_user_id,
                &request.provider,
                &request.account_profile,
            )?;
            let provider_account_env = self.provider_account_profiles.resolve_environment(
                &account_owner_user_id,
                &request.provider,
                &profile.profile_id,
            )?;
            request.account_profile = profile.profile_id;
            request = request.with_provider_account_env(provider_account_env);
        }
        let effective_config =
            crate::session::effective_agent_execution_config(&session, agent.as_ref());
        if request.execution_mode.is_none() {
            request = request.with_execution_mode(effective_config.mode);
        }
        if request.permission_level.is_none() {
            request = request.with_permission_level(effective_config.permission_level);
        }
        if request.resume_state.is_none() {
            if let Some(agent) = agent.as_ref() {
                let resume_state = crate::app::sanitize_resume_state_for_launch(&request, agent);
                if !resume_state.is_empty() {
                    request = request.with_resume_state(resume_state);
                }
            }
        }
        if request.working_directory.is_none() {
            let agent_worktree = agent
                .as_ref()
                .and_then(|agent| agent.worktree_id().map(std::path::PathBuf::from));
            request.working_directory = Some(
                agent_worktree.unwrap_or_else(|| std::path::PathBuf::from(session.worktree_id())),
            );
        }
        if request.uses_workspace_live_sync() && request.workspace_live_sync_roots.is_empty() {
            let workspace_live_sync_roots = crate::app::workspace_live_sync_protected_roots(
                &session,
                request.working_directory.as_deref(),
                &config.host_machine_id,
                &config.daemon_id,
            );
            request = request.with_workspace_live_sync_roots(workspace_live_sync_roots);
        }
        if crate::provider::managed_provider_isolation_required()
            && !session.project_id().is_empty()
        {
            let project = self.session_store.get_project(session.project_id())?;
            let mut roots = project
                .workspace_ids()
                .iter()
                .filter(|workspace| !workspace.trim().is_empty())
                .map(std::path::PathBuf::from)
                .collect::<Vec<_>>();
            if let Some(root) = crate::app::registered_workflow_runtime_worktree_root(
                &session,
                request.agent_id.as_deref(),
                request.working_directory.as_deref(),
            ) {
                if !roots.iter().any(|existing| existing == &root) {
                    roots.push(root);
                }
            }
            for root in std::mem::take(&mut request.workspace_live_sync_roots) {
                if !roots.iter().any(|existing| existing == &root) {
                    roots.push(root);
                }
            }
            request = request.with_workspace_live_sync_roots(roots);
        }
        if request.runtime_mcp_binding.is_none() {
            let shared_auth_token = request
                .agent_id
                .is_none()
                .then(|| {
                    self.provider_store
                        .get_session_run_for_provider(&request.session_id, &request.provider)
                        .and_then(|run| run.runtime_mcp_auth_token().map(str::to_string))
                })
                .flatten();
            request = request.with_runtime_mcp_binding(crate::provider::RuntimeMcpBinding::new(
                runtime_mcp_url,
                shared_auth_token.unwrap_or_else(crate::app::generate_runtime_mcp_auth_token),
            ));
        }
        if request.provider_env_remove.is_empty() {
            request =
                request.with_provider_env_remove(crate::app::default_provider_env_remove(&config));
        }
        for name in crate::account_profile::provider_auth_env_vars(&request.provider) {
            if !request
                .provider_env_remove
                .iter()
                .any(|existing| existing == name)
            {
                request.provider_env_remove.push((*name).to_string());
            }
        }
        if request.mcp_servers.is_empty() {
            if let Some(agent) = agent.as_ref() {
                request =
                    request.with_mcp_servers(crate::app::granted_mcp_servers_for_agent_launch(
                        "provider.launch.mcps",
                        &session,
                        agent,
                    )?);
            }
        }
        let mcp_servers = std::mem::take(&mut request.mcp_servers);
        request = request.with_mcp_servers(crate::app::resolve_mcp_credentials_for_launch(
            &config,
            mcp_servers,
        )?);
        request = crate::app::apply_metaagent_launch_policy(request, agent.as_ref());
        Ok(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn owned_launch_preparation_preserves_provider_account_and_project_repositories() {
        let root = std::env::temp_dir().join(format!(
            "chariox-owned-provider-launch-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let primary = root.join("primary");
        let supporting = root.join("supporting");
        std::fs::create_dir_all(&primary).expect("primary workspace");
        std::fs::create_dir_all(&supporting).expect("supporting workspace");

        let mut app = crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(
                crate::session::CreateSessionRequest::new(
                    primary.to_string_lossy(),
                    primary.to_string_lossy(),
                )
                .with_project_selection(crate::session::SessionProjectSelection::New),
            )
            .expect("session");
        app.sessions_mut()
            .update_project_workspaces(
                session.project_id(),
                vec![
                    primary.to_string_lossy().into_owned(),
                    supporting.to_string_lossy().into_owned(),
                ],
                crate::session::DEFAULT_LOCAL_USER_ID,
            )
            .expect("project workspaces");
        let registry = app.provider_account_profile_registry();
        let profile = registry
            .create_managed(
                crate::session::DEFAULT_LOCAL_USER_ID,
                "codex",
                "Owned launch test",
            )
            .expect("managed Codex profile");
        let environment = registry
            .resolve_environment(
                crate::session::DEFAULT_LOCAL_USER_ID,
                "codex",
                &profile.profile_id,
            )
            .expect("profile environment");
        std::fs::write(
            Path::new(&environment["CODEX_HOME"]).join("auth.json"),
            br#"{"tokens":{"access_token":"test"}}"#,
        )
        .expect("Codex credential fixture");

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "codex",
            "codex",
            profile.profile_id,
            "gpt-5.6-luna",
        )
        .with_agent_id(agent.id());

        let _env = crate::env_lock::lock();
        let previous_isolation = std::env::var_os(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV);
        std::env::set_var(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV, "1");
        let prepared = runtime
            .owned
            .prepare_provider_launch_request(request, "http://127.0.0.1:43120/mcp".to_string())
            .expect("owned launch preparation");
        match previous_isolation {
            Some(value) => {
                std::env::set_var(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV, value)
            }
            None => std::env::remove_var(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV),
        }

        assert_eq!(prepared.provider_account_env, environment);
        assert_eq!(
            prepared.workspace_live_sync_roots,
            vec![primary.clone(), supporting.clone()]
        );
        for name in ["OPENAI_API_KEY", "CODEX_API_KEY"] {
            assert!(prepared
                .provider_env_remove
                .iter()
                .any(|existing| existing == name));
        }

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn owned_managed_launch_allows_only_the_registered_pool_instance_worktree() {
        let root = std::env::temp_dir().join(format!(
            "chariox-owned-managed-pool-launch-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let primary = root.join("primary");
        let supporting = root.join("supporting");
        let instance_worktree = root.join("workflow-runtime").join("instance-2");
        let unregistered_worktree = root.join("workflow-runtime").join("unregistered");
        std::fs::create_dir_all(&primary).expect("primary workspace");
        std::fs::create_dir_all(&supporting).expect("supporting workspace");
        std::fs::create_dir_all(&instance_worktree).expect("runtime instance worktree");
        std::fs::create_dir_all(&unregistered_worktree).expect("unregistered worktree");

        let mut app = crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap");
        let (session, source_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(
                crate::session::CreateSessionRequest::new(
                    primary.to_string_lossy(),
                    primary.to_string_lossy(),
                )
                .with_project_selection(crate::session::SessionProjectSelection::New),
            )
            .expect("session");
        app.sessions_mut()
            .update_project_workspaces(
                session.project_id(),
                vec![
                    primary.to_string_lossy().into_owned(),
                    supporting.to_string_lossy().into_owned(),
                ],
                crate::session::DEFAULT_LOCAL_USER_ID,
            )
            .expect("project workspaces");
        let runtime_agent = app.agents().clone().materialize_workflow_runtime_agent(
            source_agent.clone(),
            session.id(),
            &instance_worktree.to_string_lossy(),
        );
        let unregistered_agent = app.agents().clone().materialize_workflow_runtime_agent(
            source_agent,
            session.id(),
            &unregistered_worktree.to_string_lossy(),
        );
        app.sessions_mut()
            .register_workflow_runtime_instance(
                session.id(),
                crate::session::WorkflowEndpointRuntimeInstance::new(
                    "instance-2",
                    "workflow-1",
                    "endpoint-1",
                    1,
                    2,
                    false,
                    std::collections::BTreeMap::from([(
                        "node-1".to_string(),
                        runtime_agent.id().to_string(),
                    )]),
                    instance_worktree.to_string_lossy(),
                ),
            )
            .expect("runtime instance should register");

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "dev-stub",
            "default",
            "model",
        )
        .with_agent_id(runtime_agent.id());

        let _env = crate::env_lock::lock();
        let previous_isolation = std::env::var_os(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV);
        std::env::set_var(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV, "1");
        let prepared = runtime
            .owned
            .prepare_provider_launch_request(request, "http://127.0.0.1:43120/mcp".to_string());
        let unregistered = runtime.owned.prepare_provider_launch_request(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "model",
            )
            .with_agent_id(unregistered_agent.id()),
            "http://127.0.0.1:43120/mcp".to_string(),
        );
        let traversal = runtime.owned.prepare_provider_launch_request(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "model",
            )
            .with_agent_id(runtime_agent.id())
            .with_working_directory(instance_worktree.join("..").join("unregistered")),
            "http://127.0.0.1:43120/mcp".to_string(),
        );
        #[cfg(unix)]
        let symlink_escape = {
            let link = instance_worktree.join("outside-link");
            std::os::unix::fs::symlink(&unregistered_worktree, &link)
                .expect("escape symlink fixture");
            runtime.owned.prepare_provider_launch_request(
                crate::provider::LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "dev-stub",
                    "default",
                    "model",
                )
                .with_agent_id(runtime_agent.id())
                .with_working_directory(link),
                "http://127.0.0.1:43120/mcp".to_string(),
            )
        };
        restore_env(
            crate::provider::MANAGED_PROVIDER_ISOLATION_ENV,
            previous_isolation,
        );
        let prepared = prepared.expect("managed pool clone launch should prepare");
        let unregistered = unregistered.expect("unregistered hidden launch should prepare");
        let traversal = traversal.expect("traversal launch should prepare without expanding roots");
        #[cfg(unix)]
        let symlink_escape =
            symlink_escape.expect("symlink escape should prepare without expanding roots");

        assert_eq!(
            prepared.workspace_live_sync_roots,
            vec![primary, supporting, instance_worktree],
            "the exact registered runtime worktree must join the managed allowlist"
        );
        assert_eq!(
            unregistered.workspace_live_sync_roots,
            vec![root.join("primary"), root.join("supporting")],
            "an unregistered hidden agent must not expand the managed allowlist"
        );
        assert_eq!(
            traversal.workspace_live_sync_roots,
            vec![root.join("primary"), root.join("supporting")],
            "a lexical traversal outside the registered worktree must not expand the managed allowlist"
        );
        #[cfg(unix)]
        assert_eq!(
            symlink_escape.workspace_live_sync_roots,
            vec![root.join("primary"), root.join("supporting")],
            "a symlink escape outside the registered worktree must not expand the managed allowlist"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn owned_launch_preparation_allows_hidden_remote_lease_session_under_managed_isolation() {
        let root = std::env::temp_dir().join(format!(
            "chariox-owned-hidden-provider-launch-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&root).expect("hidden session workspace should exist");
        let app = crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap");
        let session = app
            .sessions_mut()
            .create_ephemeral_session(
                crate::session::CreateSessionRequest::new(
                    root.to_string_lossy(),
                    root.to_string_lossy(),
                )
                .with_hidden(true),
            )
            .expect("hidden remote lease session should create");
        assert!(session.project_id().is_empty());

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "dev-stub",
            "default",
            "model",
        );

        let _env = crate::env_lock::lock();
        let previous_isolation = std::env::var_os(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV);
        std::env::set_var(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV, "1");
        let prepared = runtime
            .owned
            .prepare_provider_launch_request(request, "http://127.0.0.1:43120/mcp".to_string());
        restore_env(
            crate::provider::MANAGED_PROVIDER_ISOLATION_ENV,
            previous_isolation,
        );
        let prepared = prepared.expect("hidden provider launch should prepare");
        assert_eq!(prepared.working_directory.as_deref(), Some(root.as_path()));
        assert!(prepared.workspace_live_sync_roots.is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    fn restore_env(name: &str, previous: Option<std::ffi::OsString>) {
        match previous {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }

    async fn owned_runtime_state(app: &Arc<Mutex<crate::app::DaemonApp>>) -> KernelRuntimeState {
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
}
