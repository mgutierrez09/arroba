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
        request: crate::provider::LaunchProviderRequest,
        runtime_mcp_url: String,
    ) -> Result<crate::provider::LaunchProviderRequest, DaemonError> {
        let request = self.prepare_provider_launch_request_without_account_credentials(
            request,
            runtime_mcp_url,
        )?;
        self.attach_provider_account_credentials(request)
    }

    pub(super) fn prepare_workflow_provider_launch_request(
        &self,
        request: crate::provider::LaunchProviderRequest,
        runtime_mcp_url: String,
    ) -> Result<crate::provider::LaunchProviderRequest, DaemonError> {
        let request = self.prepare_provider_launch_request_without_account_credentials(
            request,
            runtime_mcp_url,
        )?;
        if self.provider_launch_request_uses_vaulted_account_credential(&request)? {
            return Ok(request);
        }
        self.attach_provider_account_credentials(request)
    }

    fn prepare_provider_launch_request_without_account_credentials(
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

    fn attach_provider_account_credentials(
        &self,
        mut request: crate::provider::LaunchProviderRequest,
    ) -> Result<crate::provider::LaunchProviderRequest, DaemonError> {
        if !request.provider_credential_env.is_empty() {
            return Ok(request);
        }
        if crate::provider::canonical_provider_family(&request.provider)
            .is_some_and(|provider| matches!(provider, "codex" | "claude" | "opencode"))
        {
            let config = self.config_projection.snapshot();
            let account_owner_user_id =
                crate::account_profile::provider_account_authority_owner_user_id(
                    &config,
                    &request.owner_user_id,
                );
            let provider_credential_env =
                crate::provider::resolve_provider_account_credentials_for_launch(
                    &config,
                    &self.provider_account_profiles,
                    &account_owner_user_id,
                    &request.provider,
                    &request.account_profile,
                    request.client_interface,
                )?;
            request = request.with_provider_credential_env(provider_credential_env);
        }
        Ok(request)
    }

    fn provider_launch_request_uses_vaulted_account_credential(
        &self,
        request: &crate::provider::LaunchProviderRequest,
    ) -> Result<bool, DaemonError> {
        let config = self.config_projection.snapshot();
        if config.user_config.credential_vault.backend
            != crate::config::CredentialVaultBackend::CharioxEncrypted
        {
            return Ok(false);
        }
        let account_owner_user_id =
            crate::account_profile::provider_account_authority_owner_user_id(
                &config,
                &request.owner_user_id,
            );
        crate::provider::provider_account_credential_uses_vault(
            &account_owner_user_id,
            &request.provider,
            &request.account_profile,
        )
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
    async fn unattended_claude_launch_requires_portable_or_vaulted_credentials() {
        let _env = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-owned-missing-claude-credential-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("test root should exist");
        std::env::set_var("CHARIOX_HOME", &root);
        let mut app = crate::app::DaemonApp::bootstrap(
            crate::config::DaemonConfig::for_tests()
                .with_session_history_root(root.join("session-history")),
        )
        .expect("daemon bootstrap");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                root.to_string_lossy(),
                root.to_string_lossy(),
            ))
            .expect("session should create");
        let profile = app
            .provider_account_profile_registry()
            .create_managed(
                crate::session::DEFAULT_LOCAL_USER_ID,
                "claude",
                "Missing Claude credential",
            )
            .expect("managed Claude profile should create");
        let claude_config_dir = std::path::PathBuf::from(
            app.provider_account_profile_registry()
                .resolve_environment(
                    crate::session::DEFAULT_LOCAL_USER_ID,
                    "claude",
                    &profile.profile_id,
                )
                .expect("Claude environment should resolve")["CLAUDE_CONFIG_DIR"]
                .clone(),
        );
        let agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "claude")
                    .with_account_profile(profile.profile_id.clone()),
            )
            .expect("Claude agent should create");
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "claude",
            "claude",
            &profile.profile_id,
            "claude-sonnet",
        )
        .with_agent_id(agent.id());
        let error = runtime
            .owned
            .prepare_provider_launch_request(
                request.clone(),
                "http://127.0.0.1:43120/mcp".to_string(),
            )
            .expect_err("unattended launch without credentials must fail before spawn");
        assert!(error
            .to_string()
            .contains("unattended Claude launch requires"));

        let foreground = runtime
            .owned
            .prepare_provider_launch_request(
                request
                    .clone()
                    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui),
                "http://127.0.0.1:43120/mcp".to_string(),
            )
            .expect("native Claude TUI must remain available for interactive sign-in");
        assert!(foreground.provider_credential_env.is_empty());

        std::fs::write(
            claude_config_dir.join(".credentials.json"),
            br#"{"claudeAiOauth":{"refreshToken":"portable-refresh-token"}}"#,
        )
        .expect("portable Claude credential fixture should write");
        let portable_launch = runtime
            .owned
            .prepare_provider_launch_request(request, "http://127.0.0.1:43120/mcp".to_string());
        if cfg!(target_os = "linux") {
            portable_launch.expect("Linux may use portable provider-native credentials");
        } else {
            portable_launch.expect_err(
                "macOS and Windows unattended launches must require a Chariox setup token",
            );
        }

        std::env::remove_var("CHARIOX_HOME");
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

    #[tokio::test]
    async fn vaulted_claude_launch_waits_for_chariox_unlock_interaction() {
        let _env = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-owned-vaulted-claude-launch-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("test root should exist");
        std::env::set_var("CHARIOX_HOME", &root);
        let vault_path = root.join("credentials.vault");
        let mut config = crate::config::DaemonConfig::for_tests()
            .with_session_history_root(root.join("session-history"));
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.path = vault_path.display().to_string();
        config.user_config.state.path = Some(root.join("state.db").display().to_string());
        config.user_config.history.operational.path =
            Some(root.join("operational.db").display().to_string());
        config.user_config.artifacts.operational.root =
            Some(root.join("artifacts").display().to_string());
        config.user_config.artifacts.operational.index_path =
            Some(root.join("artifacts.db").display().to_string());

        let mut app = crate::app::DaemonApp::bootstrap(config.clone()).expect("daemon bootstrap");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                root.to_string_lossy(),
                root.to_string_lossy(),
            ))
            .expect("session should create");
        let profile = app
            .provider_account_profile_registry()
            .create_managed(
                crate::session::DEFAULT_LOCAL_USER_ID,
                "claude",
                "Vaulted Claude",
            )
            .expect("managed Claude profile should create");
        let workflow_agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "claude")
                    .with_model("claude-sonnet")
                    .with_account_profile(profile.profile_id.clone()),
            )
            .expect("workflow Claude agent should create");
        crate::secret::unlock_chariox_encrypted_vault(
            &vault_path,
            "correct horse battery staple",
            crate::secret::VaultUnlockLease::KernelShutdown,
        )
        .expect("vault should initialize");
        crate::provider::store_provider_account_credential(
            &config,
            crate::session::DEFAULT_LOCAL_USER_ID,
            "claude",
            &profile.profile_id,
            "setup-token-secret",
            false,
        )
        .expect("provider credential should store");
        crate::secret::lock_chariox_encrypted_vault(&vault_path).expect("vault should lock");
        crate::secret::clear_vault_secret_process_cache().expect("secret cache should clear");

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let (workflow_provider_run_id, retired_provider_run_id) = runtime
            .owned
            .workflow_ensure_provider_run(
                session.id(),
                workflow_agent.id(),
                false,
                false,
                false,
                false,
                None,
            )
            .expect("locked vault must not block synchronous workflow admission");
        assert!(retired_provider_run_id.is_none());
        assert!(runtime
            .owned
            .take_pending_provider_launch_credentials(&workflow_provider_run_id)
            .is_empty());
        let workflow_run = runtime
            .owned
            .provider_store
            .get_run(&workflow_provider_run_id)
            .expect("workflow provider run should exist");
        let mut workflow_credentials = Box::pin(
            runtime.resolve_provider_account_credentials_for_run_with_vault(
                &workflow_run,
                "test vaulted workflow Claude launch",
            ),
        );
        tokio::select! {
            result = &mut workflow_credentials => panic!("workflow credential resolution completed before unlock: {result:?}"),
            _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {}
        }
        let interaction = runtime
            .owned
            .session_store
            .get_session(session.id())
            .expect("session should remain available")
            .active_interaction_for_agent(workflow_agent.id())
            .expect("workflow vault unlock interaction should be visible")
            .clone();
        runtime
            .resolve_runtime_interaction(
                session.id(),
                interaction.id(),
                "unlock_operation",
                Some("correct horse battery staple"),
            )
            .await
            .expect("workflow unlock interaction should resolve");
        assert_eq!(
            workflow_credentials
                .await
                .expect("workflow credentials should resolve after unlock")
                .iter()
                .collect::<Vec<_>>(),
            vec![("CLAUDE_CODE_OAUTH_TOKEN", "setup-token-secret")]
        );

        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "claude",
            "claude",
            &profile.profile_id,
            "claude-sonnet",
        )
        .with_agent_id(agent.id());
        let mut preparation = Box::pin(
            runtime
                .prepare_provider_launch_request_with_vault(request, "test vaulted Claude launch"),
        );
        tokio::select! {
            result = &mut preparation => panic!("launch preparation resolved before unlock: {result:?}"),
            _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {}
        }
        let pending_session = runtime
            .owned
            .session_store
            .get_session(session.id())
            .expect("session should remain available");
        let interaction = pending_session
            .active_interaction_for_agent(agent.id())
            .expect("vault unlock interaction should be visible")
            .clone();
        assert_eq!(interaction.title(), Some("Unlock Chariox Vault"));
        runtime
            .resolve_runtime_interaction(
                session.id(),
                interaction.id(),
                "unlock_operation",
                Some("correct horse battery staple"),
            )
            .await
            .expect("unlock interaction should resolve");
        let prepared = preparation
            .await
            .expect("launch should prepare after Chariox unlock");
        assert_eq!(
            prepared.provider_credential_env.iter().collect::<Vec<_>>(),
            vec![("CLAUDE_CODE_OAUTH_TOKEN", "setup-token-secret")]
        );
        assert!(
            !crate::secret::chariox_encrypted_vault_status(&vault_path)
                .expect("vault status should resolve")
                .unlocked
        );

        let mut remote_credential = Box::pin(runtime.resolve_remote_provider_launch_credential(
            session.id(),
            workflow_agent.id(),
            "test remote Claude launch",
        ));
        tokio::select! {
            result = &mut remote_credential => panic!("remote credential resolved before unlock: {result:?}"),
            _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {}
        }
        let interaction = runtime
            .owned
            .session_store
            .get_session(session.id())
            .expect("session should remain available")
            .active_interaction_for_agent(workflow_agent.id())
            .expect("remote launch vault interaction should be visible")
            .clone();
        runtime
            .resolve_runtime_interaction(
                session.id(),
                interaction.id(),
                "unlock_operation",
                Some("correct horse battery staple"),
            )
            .await
            .expect("remote launch unlock should resolve");
        let credential = remote_credential
            .await
            .expect("remote launch credential should resolve")
            .expect("Claude launch should carry a credential");
        assert_eq!(credential.provider, "claude");
        assert_eq!(credential.account_profile, profile.profile_id);
        assert!(!format!("{credential:?}").contains("setup-token-secret"));
        assert_eq!(
            credential.secret_input.into_zeroizing().as_str(),
            "setup-token-secret"
        );
        assert!(
            !crate::secret::chariox_encrypted_vault_status(&vault_path)
                .expect("vault status should resolve")
                .unlocked
        );

        std::env::remove_var("CHARIOX_HOME");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn mcp_continuation_retries_when_prompt_starts_during_vault_unlock() {
        let _env = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-mcp-continuation-vault-race-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("test root should exist");
        let previous_home = std::env::var_os("CHARIOX_HOME");
        let previous_claude_bin = std::env::var_os("CHARIOX_CLAUDE_BIN");
        std::env::set_var("CHARIOX_HOME", &root);
        std::env::set_var(
            "CHARIOX_CLAUDE_BIN",
            std::env::current_exe().expect("test executable should resolve"),
        );

        let vault_path = root.join("credentials.vault");
        let mut config = crate::config::DaemonConfig::for_tests()
            .with_session_history_root(root.join("session-history"));
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.path = vault_path.display().to_string();
        config.user_config.state.path = Some(root.join("state.db").display().to_string());
        config.user_config.history.operational.path =
            Some(root.join("operational.db").display().to_string());
        config.user_config.artifacts.operational.root =
            Some(root.join("artifacts").display().to_string());
        config.user_config.artifacts.operational.index_path =
            Some(root.join("artifacts.db").display().to_string());

        let mut app = crate::app::DaemonApp::bootstrap(config.clone()).expect("daemon bootstrap");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                root.to_string_lossy(),
                root.to_string_lossy(),
            ))
            .expect("session should create");
        let profile = app
            .provider_account_profile_registry()
            .create_managed(
                crate::session::DEFAULT_LOCAL_USER_ID,
                "claude",
                "Vaulted continuation Claude",
            )
            .expect("managed Claude profile should create");
        let agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "claude")
                    .with_model("claude-sonnet")
                    .with_account_profile(profile.profile_id.clone()),
            )
            .expect("Claude agent should create");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "mcp-continuation-vault-race-client",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("test client should attach");
        crate::secret::unlock_chariox_encrypted_vault(
            &vault_path,
            "correct horse battery staple",
            crate::secret::VaultUnlockLease::KernelShutdown,
        )
        .expect("vault should initialize");
        crate::provider::store_provider_account_credential(
            &config,
            crate::session::DEFAULT_LOCAL_USER_ID,
            "claude",
            &profile.profile_id,
            "setup-token-secret",
            false,
        )
        .expect("provider credential should store");

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "claude",
            "claude",
            &profile.profile_id,
            "claude-sonnet",
        )
        .with_agent_id(agent.id());
        let prepared = runtime
            .prepare_provider_launch_request_with_vault(request, "prepare test provider run")
            .await
            .expect("initial launch should prepare while vault is unlocked");
        let started = runtime
            .owned
            .provider_store
            .start_run_provider_only(prepared)
            .expect("test provider run should start");
        let running = runtime
            .owned
            .provider_store
            .mark_run_running(started.run().id())
            .expect("test provider run should become idle and running");
        runtime
            .owned
            .provider_run_projection
            .update(running.clone());
        crate::secret::lock_chariox_encrypted_vault(&vault_path).expect("vault should lock");
        crate::secret::clear_vault_secret_process_cache().expect("secret cache should clear");

        runtime.remember_pending_mcp_continuation(
            session.id(),
            agent.id(),
            attachment.id(),
            "playwright",
            "continue after granting playwright",
        );
        let first_unlock = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Some(interaction) = runtime
                    .owned
                    .session_store
                    .get_session(session.id())
                    .expect("session should remain available")
                    .active_interaction_for_agent(agent.id())
                {
                    break interaction.clone();
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("first vault unlock interaction should appear");
        let active_prompt = crate::session::PromptQueueItem::new(
            "prompt-started-during-unlock",
            attachment.id(),
            agent.id(),
            "new prompt",
            crate::session::PromptStatus::Running,
        )
        .with_prompt_origin(crate::session::PromptOrigin::External);
        app.lock()
            .await
            .prompt_owner_sync_external_active_prompt(session.id(), agent.id(), Some(active_prompt))
            .expect("active prompt should start during unlock");
        runtime
            .resolve_runtime_interaction(
                session.id(),
                first_unlock.id(),
                "unlock_operation",
                Some("correct horse battery staple"),
            )
            .await
            .expect("first vault unlock should resolve");

        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if runtime
                    .owned
                    .pending_mcp_continuations
                    .write()
                    .contains_key(agent.id())
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("deferred MCP continuation should remain pending");
        assert_eq!(
            runtime
                .owned
                .provider_store
                .get_run_for_agent(session.id(), agent.id())
                .expect("provider run should remain available")
                .id(),
            running.id()
        );

        app.lock()
            .await
            .prompt_owner_sync_external_active_prompt(session.id(), agent.id(), None)
            .expect("agent should become idle");
        let second_unlock = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Some(interaction) = runtime
                    .owned
                    .session_store
                    .get_session(session.id())
                    .expect("session should remain available")
                    .active_interaction_for_agent(agent.id())
                {
                    if interaction.title() == Some("Unlock Chariox Vault") {
                        break interaction.clone();
                    }
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("idle retry should request vault unlock again");
        runtime
            .resolve_runtime_interaction(
                session.id(),
                second_unlock.id(),
                "unlock_operation",
                Some("correct horse battery staple"),
            )
            .await
            .expect("retry vault unlock should resolve");
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let resumed = runtime
                    .owned
                    .operational_history_store
                    .query_events(crate::history::HistoryEventQuery {
                        session_id: Some(session.id().to_string()),
                        agent_id: Some(agent.id().to_string()),
                        text: Some("continue after granting playwright".to_string()),
                        limit: Some(10),
                        ..crate::history::HistoryEventQuery::default()
                    })
                    .expect("continuation history should remain queryable")
                    .into_iter()
                    .any(|event| {
                        event
                            .prompt_id
                            .as_deref()
                            .is_some_and(|prompt_id| prompt_id.starts_with("prompt-"))
                    });
                if resumed {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("MCP continuation should resume after the agent becomes idle");

        let _ = crate::secret::lock_chariox_encrypted_vault(&vault_path);
        let _ = crate::secret::clear_vault_secret_process_cache();
        restore_env("CHARIOX_HOME", previous_home);
        restore_env("CHARIOX_CLAUDE_BIN", previous_claude_bin);
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
