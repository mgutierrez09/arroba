use std::path::PathBuf;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::{LaunchProviderRequest, RuntimeMcpBinding};

use super::provider_launch_policy::{
    apply_metaagent_launch_policy, default_provider_env_remove, generate_runtime_mcp_auth_token,
    granted_mcp_servers_for_agent_launch, resolve_mcp_credentials_for_launch,
    sanitize_resume_state_for_launch,
};

impl DaemonApp {
    pub(crate) fn prepare_app_provider_launch_request(
        &self,
        mut request: LaunchProviderRequest,
        operation: &'static str,
    ) -> Result<LaunchProviderRequest, DaemonError> {
        let session = self.sessions.get_session(&request.session_id)?;
        if request.agent_id.is_none() {
            request.agent_id = session.focused_agent_id().map(str::to_string);
        }
        let agent = request
            .agent_id
            .as_deref()
            .and_then(|agent_id| self.agents.get_agent(agent_id).ok());
        if let Some(agent) = agent.as_ref() {
            if agent.session_id() != session.id() {
                return Err(DaemonError::AgentNotInSession {
                    session_id: session.id().to_string(),
                    agent_id: agent.id().to_string(),
                });
            }
            if agent.remote_execution().is_some() {
                return Err(DaemonError::LocalTransport {
                    operation,
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
                    &self.config,
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
            let provider_credential_env = crate::provider::resolve_provider_account_credentials(
                &self.config,
                &account_owner_user_id,
                &request.provider,
                &profile.profile_id,
            )?;
            request.account_profile = profile.profile_id;
            request = request
                .with_provider_account_env(provider_account_env)
                .with_provider_credential_env(provider_credential_env);
        }
        if request.resume_state.is_none() {
            if let Some(agent) = agent.as_ref() {
                let resume_state = sanitize_resume_state_for_launch(&request, agent);
                if !resume_state.is_empty() {
                    request = request.with_resume_state(resume_state);
                }
            }
        }
        if request.working_directory.is_none() {
            let working_directory = agent
                .as_ref()
                .and_then(|agent| agent.worktree_id().map(PathBuf::from))
                .unwrap_or_else(|| PathBuf::from(session.worktree_id()));
            request = request.with_working_directory(working_directory);
        }
        if request.uses_workspace_live_sync() && request.workspace_live_sync_roots.is_empty() {
            let workspace_live_sync_roots = crate::app::workspace_live_sync_protected_roots(
                &session,
                request.working_directory.as_deref(),
                &self.config.host_machine_id,
                &self.config.daemon_id,
            );
            request = request.with_workspace_live_sync_roots(workspace_live_sync_roots);
        }
        if crate::provider::managed_provider_isolation_required()
            && !session.project_id().is_empty()
        {
            let project = self.sessions.get_project(session.project_id())?;
            let mut roots = project
                .workspace_ids()
                .iter()
                .filter(|workspace| !workspace.trim().is_empty())
                .map(PathBuf::from)
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
                    self.providers
                        .get_session_run_for_provider(&request.session_id, &request.provider)
                        .and_then(|run| run.runtime_mcp_auth_token().map(str::to_string))
                })
                .flatten();
            request = request.with_runtime_mcp_binding(RuntimeMcpBinding::new(
                self.config.runtime_mcp_url(),
                shared_auth_token.unwrap_or_else(generate_runtime_mcp_auth_token),
            ));
        }
        if request.provider_env_remove.is_empty() {
            request = request.with_provider_env_remove(default_provider_env_remove(&self.config));
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
                request = request.with_mcp_servers(granted_mcp_servers_for_agent_launch(
                    operation, &session, agent,
                )?);
            }
        }
        let mcp_servers = std::mem::take(&mut request.mcp_servers);
        request = request.with_mcp_servers(resolve_mcp_credentials_for_launch(
            &self.config,
            mcp_servers,
        )?);
        request = apply_metaagent_launch_policy(request, agent.as_ref());
        Ok(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::CreateAgentRequest;
    use crate::provider::LaunchProviderRequest;
    use crate::provider::{AgentExecutionMode, AgentPermissionLevel};
    use crate::session::CreateSessionRequest;

    #[test]
    fn app_launch_preparation_scopes_workspace_live_sync_roots_to_selected_repo_and_local_links() {
        let base = std::env::temp_dir().join(format!(
            "chariox-app-live-sync-root-scope-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let selected = base.join("selected");
        let selected_child = selected.join("src");
        let sibling = base.join("sibling");
        let attached = base.join("attached");
        std::fs::create_dir_all(&selected_child).expect("selected repo fixture should exist");
        std::fs::create_dir_all(&sibling).expect("sibling repo fixture should exist");
        std::fs::create_dir_all(&attached).expect("attached repo fixture should exist");
        run_git_init(&selected);
        run_git_init(&sibling);
        run_git_init(&attached);

        let app =
            DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).expect("daemon boot");
        let session = app
            .sessions_mut()
            .create_session(CreateSessionRequest::new(
                selected.to_string_lossy(),
                selected_child.to_string_lossy(),
            ))
            .expect("session should be created");
        app.sessions_mut()
            .set_workspace_live_sync_mode(
                session.id(),
                crate::config::WorkspaceLiveSyncMode::Tracked,
            )
            .expect("workspace live sync mode should update");
        app.sessions_mut()
            .create_workspace_link(session.id(), "shared".to_string(), "local".to_string())
            .expect("workspace link should be created");
        app.sessions_mut()
            .attach_workspace_link(
                session.id(),
                "shared",
                "local".to_string(),
                "machine-test".to_string(),
                "daemon-test".to_string(),
                attached.to_string_lossy().to_string(),
                None,
                None,
            )
            .expect("local workspace link should attach");
        app.sessions_mut()
            .attach_workspace_link(
                session.id(),
                "shared",
                "peer".to_string(),
                "remote-machine".to_string(),
                "remote-daemon".to_string(),
                "/remote/repo".to_string(),
                None,
                None,
            )
            .expect("remote workspace link should attach");

        let prepared = app
            .prepare_app_provider_launch_request(
                LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "dev-stub",
                    "default",
                    "model",
                )
                .with_workspace_live_sync_mode(crate::config::WorkspaceLiveSyncMode::Tracked),
                "test.launch",
            )
            .expect("provider launch should prepare");

        let canonical_selected = selected
            .canonicalize()
            .expect("selected repo should canonicalize");
        let _ = std::fs::remove_dir_all(&base);
        assert_eq!(
            prepared.workspace_live_sync_roots,
            vec![canonical_selected, attached]
        );
    }

    #[test]
    fn app_launch_preparation_preserves_metaagent_mode_and_permission_without_user_mcps() {
        let mut app =
            DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).expect("daemon boot");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let metaagent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_execution_mode_override(AgentExecutionMode::Build)
                    .with_permission_level_override(AgentPermissionLevel::Yolo),
            )
            .expect("metaagent should spawn");
        let metaagent = app
            .agents_mut()
            .activate_agent_meta_mode(metaagent.id(), None)
            .expect("agent should enter meta mode");

        let prepared = app
            .prepare_app_provider_launch_request(
                LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "dev-stub",
                    "default",
                    "model",
                )
                .with_agent_id(metaagent.id())
                .with_execution_mode(AgentExecutionMode::Build)
                .with_permission_level(AgentPermissionLevel::Yolo)
                .with_mcp_servers(vec![
                    crate::mcp::CharioxMcpServerConfig::streamable_http(
                        "worker-tool",
                        "http://127.0.0.1/mcp",
                    ),
                ]),
                "test.launch",
            )
            .expect("provider launch should prepare");

        assert_eq!(prepared.execution_mode, Some(AgentExecutionMode::Build));
        assert_eq!(prepared.permission_level, Some(AgentPermissionLevel::Yolo));
        assert_eq!(
            prepared
                .provider_config_overrides
                .get("features.multi_agent"),
            Some(&serde_json::json!(false)),
            "Meta mode should disable provider-native Codex multi-agent tools"
        );
        assert_eq!(
            prepared
                .provider_config_overrides
                .get("chariox.metaagent_tools_only"),
            Some(&serde_json::json!(true)),
            "Meta mode should expose only Chariox orchestration tools to provider-native runtimes"
        );
        assert!(
            prepared.mcp_servers.is_empty(),
            "metaagent provider runs should not receive user MCP servers"
        );
    }

    #[test]
    fn app_launch_preparation_allows_hidden_remote_lease_session_under_managed_isolation() {
        let root = std::env::temp_dir().join(format!(
            "chariox-hidden-provider-launch-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&root).expect("hidden session workspace should exist");
        run_git_init(&root);
        let app =
            DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).expect("daemon boot");
        let session = app
            .sessions_mut()
            .create_ephemeral_session(
                CreateSessionRequest::new(root.to_string_lossy(), root.to_string_lossy())
                    .with_hidden(true),
            )
            .expect("hidden remote lease session should create");
        assert!(session.project_id().is_empty());

        let _env = crate::env_lock::lock();
        let previous_isolation = std::env::var_os(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV);
        std::env::set_var(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV, "1");
        let prepared = app.prepare_app_provider_launch_request(
            LaunchProviderRequest::new(session.id(), "dev-stub", "dev-stub", "default", "model"),
            "test.launch",
        );
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

    fn run_git_init(path: &std::path::Path) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("init")
            .arg("-b")
            .arg("main")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("git init should run");
        assert!(
            status.success(),
            "git init should succeed in {}",
            path.display()
        );
    }
}
