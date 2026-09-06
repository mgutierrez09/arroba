use std::path::PathBuf;

use crate::account_profile::{
    ProviderAccountProfile, ProviderAccountProfileRegistry, ProviderAccountUsageAvailability,
};
use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::{LaunchProviderRequest, RuntimeMcpBinding};

use super::provider_launch_policy::{
    apply_metaagent_launch_policy, default_provider_env_remove, generate_runtime_mcp_auth_token,
    granted_mcp_servers_for_agent_launch, registered_workflow_runtime_worktree_root,
    resolve_mcp_credentials_for_launch, sanitize_resume_state_for_launch,
};

const PROVIDER_USAGE_REFRESH_RETRY_AFTER_MS: u64 = 5 * 60 * 1_000;

fn provider_usage_refresh_due_for_launch(
    usage: &crate::account_profile::ProviderAccountUsageSnapshot,
    now_ms: u64,
) -> bool {
    let usage = usage.clone().reconciled_freshness(now_ms);
    match usage.availability {
        ProviderAccountUsageAvailability::Stale => true,
        ProviderAccountUsageAvailability::Unavailable | ProviderAccountUsageAvailability::Error => {
            usage.observed_at_ms.is_none_or(|observed_at_ms| {
                now_ms.saturating_sub(observed_at_ms) >= PROVIDER_USAGE_REFRESH_RETRY_AFTER_MS
            })
        }
        ProviderAccountUsageAvailability::Available | ProviderAccountUsageAvailability::Partial => {
            false
        }
    }
}

fn refresh_provider_usage_before_launch(
    registry: &ProviderAccountProfileRegistry,
    owner_user_id: &str,
    provider: &str,
    account_profile: &str,
    now_ms: u64,
    refresh: impl FnOnce() -> Result<ProviderAccountProfile, DaemonError>,
) -> Result<(), DaemonError> {
    let profile = registry.get(owner_user_id, provider, account_profile)?;
    if provider_usage_refresh_due_for_launch(&profile.usage, now_ms) {
        if let Some(_refresh_lease) = registry.acquire_usage_refresh_attempt(
            owner_user_id,
            provider,
            &profile.profile_id,
            now_ms,
            PROVIDER_USAGE_REFRESH_RETRY_AFTER_MS,
        )? {
            // Reporting is optional for providers and plans that expose no stable usage API.
            // Keep unknown capacity honest rather than blocking work when refresh itself fails.
            if refresh().is_err() {
                crate::logging::warn_with_fields(
                    "daemon.provider",
                    "provider usage refresh before launch was unavailable",
                    serde_json::json!({
                        "provider": provider,
                        "account_profile": account_profile,
                    }),
                );
            }
        }
    }
    Ok(())
}

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
            let profile = if request.client_interface.is_chariox() {
                refresh_provider_usage_before_launch(
                    &self.provider_account_profiles,
                    &account_owner_user_id,
                    &request.provider,
                    &request.account_profile,
                    crate::session::unix_epoch_ms(),
                    || {
                        crate::local::provider_requests::refresh_provider_account_profile_response(
                            &self.provider_account_profiles,
                            &account_owner_user_id,
                            &request.provider,
                            &request.account_profile,
                        )
                    },
                )?;
                self.provider_account_profiles.require_authenticated(
                    &account_owner_user_id,
                    &request.provider,
                    &request.account_profile,
                    Some(&request.model),
                    operation,
                )?
            } else {
                self.provider_account_profiles.get(
                    &account_owner_user_id,
                    &request.provider,
                    &request.account_profile,
                )?
            };
            let provider_account_env = self.provider_account_profiles.resolve_environment(
                &account_owner_user_id,
                &request.provider,
                &profile.profile_id,
            )?;
            request.account_profile = profile.profile_id;
            request = request.with_provider_account_env(provider_account_env);
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
            if let Some(root) = registered_workflow_runtime_worktree_root(
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
    use crate::account_profile::{
        ProviderAccountAuthState, ProviderAccountUsageMeter, ProviderAccountUsageMeterKind,
        ProviderAccountUsageMeterScope, ProviderAccountUsageMeterState,
        ProviderAccountUsageSnapshot,
    };
    use crate::agent::CreateAgentRequest;
    use crate::provider::LaunchProviderRequest;
    use crate::provider::{AgentExecutionMode, AgentPermissionLevel};
    use crate::session::CreateSessionRequest;

    #[test]
    fn launch_refreshes_unobserved_usage_before_applying_the_capacity_gate() {
        let root = std::env::temp_dir().join(format!(
            "chariox-provider-launch-usage-refresh-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let registry = ProviderAccountProfileRegistry::open(root.join("profiles.json"))
            .expect("profile registry should open");
        let profiles = registry
            .migrate_effective_defaults("owner-a", &root.join("home"))
            .expect("default profiles should migrate");
        let profile = profiles
            .into_iter()
            .find(|profile| profile.provider == "opencode")
            .expect("OpenCode default should exist");
        registry
            .update_observation(
                "owner-a",
                "opencode",
                &profile.profile_id,
                ProviderAccountAuthState::Authenticated,
                None,
                None,
                None,
                None,
            )
            .expect("profile should be authenticated");
        let now_ms = crate::session::unix_epoch_ms();
        let mut refresh_count = 0;

        refresh_provider_usage_before_launch(
            &registry,
            "owner-a",
            "opencode",
            &profile.profile_id,
            now_ms,
            || {
                refresh_count += 1;
                registry.update_usage(
                    "owner-a",
                    "opencode",
                    &profile.profile_id,
                    ProviderAccountUsageSnapshot {
                        profile_id: profile.profile_id.clone(),
                        provider: "opencode".to_string(),
                        availability: ProviderAccountUsageAvailability::Available,
                        meters: vec![ProviderAccountUsageMeter {
                            meter_id: "go/monthly".to_string(),
                            label: "OpenCode Go monthly".to_string(),
                            service_id: Some("opencode-go".to_string()),
                            kind: ProviderAccountUsageMeterKind::RollingLimit,
                            scope: ProviderAccountUsageMeterScope::Plan,
                            used_percent: Some(100.0),
                            used: None,
                            remaining: None,
                            total: None,
                            unit: None,
                            window_duration_minutes: None,
                            resets_at_ms: Some(now_ms + 60_000),
                            state: ProviderAccountUsageMeterState::Exhausted,
                            source: "test.go_usage".to_string(),
                            observed_at_ms: now_ms,
                        }],
                        observed_at_ms: Some(now_ms),
                        source: "test.go_usage".to_string(),
                        management_url: None,
                    },
                )
            },
        )
        .expect("launch refresh should complete");

        assert_eq!(refresh_count, 1);
        let blocked = registry
            .require_authenticated(
                "owner-a",
                "opencode",
                &profile.profile_id,
                Some("opencode-go/deepseek-v4"),
                "test.launch",
            )
            .expect_err("fresh exhausted Go usage should block the launch");
        assert!(blocked.to_string().contains("usage is exhausted"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn concurrent_launch_waits_for_in_flight_usage_refresh_before_capacity_gate() {
        use std::sync::mpsc;
        use std::time::Duration;

        let root = std::env::temp_dir().join(format!(
            "chariox-provider-launch-concurrent-usage-refresh-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let registry = ProviderAccountProfileRegistry::open(root.join("profiles.json"))
            .expect("profile registry should open");
        let profile = registry
            .migrate_effective_defaults("owner-a", &root.join("home"))
            .expect("default profiles should migrate")
            .into_iter()
            .find(|profile| profile.provider == "opencode")
            .expect("OpenCode default should exist");
        registry
            .update_observation(
                "owner-a",
                "opencode",
                &profile.profile_id,
                ProviderAccountAuthState::Authenticated,
                None,
                None,
                None,
                None,
            )
            .expect("profile should be authenticated");

        let now_ms = crate::session::unix_epoch_ms();
        let profile_id = profile.profile_id.clone();
        let first_registry = registry.clone();
        let first_profile_id = profile_id.clone();
        let (refresh_started_tx, refresh_started_rx) = mpsc::channel();
        let (release_refresh_tx, release_refresh_rx) = mpsc::channel();
        let first = std::thread::spawn(move || {
            refresh_provider_usage_before_launch(
                &first_registry,
                "owner-a",
                "opencode",
                &first_profile_id,
                now_ms,
                || {
                    refresh_started_tx
                        .send(())
                        .expect("test should observe the in-flight refresh");
                    release_refresh_rx
                        .recv()
                        .expect("test should release the in-flight refresh");
                    first_registry.update_usage(
                        "owner-a",
                        "opencode",
                        &first_profile_id,
                        ProviderAccountUsageSnapshot {
                            profile_id: first_profile_id.clone(),
                            provider: "opencode".to_string(),
                            availability: ProviderAccountUsageAvailability::Available,
                            meters: vec![ProviderAccountUsageMeter {
                                meter_id: "go/monthly".to_string(),
                                label: "OpenCode Go monthly".to_string(),
                                service_id: Some("opencode-go".to_string()),
                                kind: ProviderAccountUsageMeterKind::RollingLimit,
                                scope: ProviderAccountUsageMeterScope::Plan,
                                used_percent: Some(100.0),
                                used: None,
                                remaining: None,
                                total: None,
                                unit: None,
                                window_duration_minutes: None,
                                resets_at_ms: Some(now_ms + 60_000),
                                state: ProviderAccountUsageMeterState::Exhausted,
                                source: "test.go_usage".to_string(),
                                observed_at_ms: now_ms,
                            }],
                            observed_at_ms: Some(now_ms),
                            source: "test.go_usage".to_string(),
                            management_url: None,
                        },
                    )
                },
            )
            .expect("first launch refresh should complete");
        });
        refresh_started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("first launch should enter its refresh");

        let second_registry = registry.clone();
        let second_profile_id = profile_id.clone();
        let (second_started_tx, second_started_rx) = mpsc::channel();
        let (second_finished_tx, second_finished_rx) = mpsc::channel();
        let second = std::thread::spawn(move || {
            second_started_tx
                .send(())
                .expect("test should observe the second launch");
            refresh_provider_usage_before_launch(
                &second_registry,
                "owner-a",
                "opencode",
                &second_profile_id,
                now_ms + 1,
                || panic!("the concurrent launch must share the in-flight refresh"),
            )
            .expect("second launch refresh coordination should complete");
            let blocked = second_registry
                .require_authenticated(
                    "owner-a",
                    "opencode",
                    &second_profile_id,
                    Some("opencode-go/deepseek-v4"),
                    "test.concurrent_launch",
                )
                .is_err();
            second_finished_tx
                .send(blocked)
                .expect("test should observe the second launch outcome");
        });
        second_started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("second launch should start");

        if let Ok(blocked) = second_finished_rx.recv_timeout(Duration::from_millis(250)) {
            release_refresh_tx
                .send(())
                .expect("first refresh should be released for cleanup");
            first.join().expect("first launch should join");
            second.join().expect("second launch should join");
            panic!(
                "second launch crossed the capacity gate before the in-flight refresh completed; blocked={blocked}"
            );
        }

        release_refresh_tx
            .send(())
            .expect("first refresh should be released");
        assert!(
            second_finished_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("second launch should finish after the refresh"),
            "second launch should apply the exhausted result from the shared refresh"
        );
        first.join().expect("first launch should join");
        second.join().expect("second launch should join");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn launch_does_not_retry_a_recent_unavailable_usage_observation() {
        let now_ms = crate::session::unix_epoch_ms();
        let usage = ProviderAccountUsageSnapshot {
            profile_id: "profile-1".to_string(),
            provider: "opencode".to_string(),
            availability: ProviderAccountUsageAvailability::Unavailable,
            meters: Vec::new(),
            observed_at_ms: Some(now_ms),
            source: "provider_not_observed".to_string(),
            management_url: None,
        };

        assert!(!provider_usage_refresh_due_for_launch(&usage, now_ms));
        assert!(provider_usage_refresh_due_for_launch(
            &usage,
            now_ms + PROVIDER_USAGE_REFRESH_RETRY_AFTER_MS
        ));
    }

    #[test]
    fn launch_rate_limits_failed_usage_refreshes_per_account() {
        let root = std::env::temp_dir().join(format!(
            "chariox-provider-launch-usage-refresh-rate-limit-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let registry = ProviderAccountProfileRegistry::open(root.join("profiles.json"))
            .expect("profile registry should open");
        let profile = registry
            .migrate_effective_defaults("owner-a", &root.join("home"))
            .expect("default profiles should migrate")
            .into_iter()
            .find(|profile| profile.provider == "opencode")
            .expect("OpenCode default should exist");
        registry
            .update_observation(
                "owner-a",
                "opencode",
                &profile.profile_id,
                ProviderAccountAuthState::Authenticated,
                None,
                None,
                None,
                None,
            )
            .expect("profile should be authenticated");
        let now_ms = crate::session::unix_epoch_ms();
        let mut refresh_count = 0;
        let mut refresh = || {
            refresh_count += 1;
            Err(DaemonError::LocalTransport {
                operation: "test.usage_refresh",
                message: "temporarily unavailable".to_string(),
            })
        };

        refresh_provider_usage_before_launch(
            &registry,
            "owner-a",
            "opencode",
            &profile.profile_id,
            now_ms,
            &mut refresh,
        )
        .expect("first failed refresh should not block launch preparation");
        refresh_provider_usage_before_launch(
            &registry,
            "owner-a",
            "opencode",
            &profile.profile_id,
            now_ms + 1_000,
            &mut refresh,
        )
        .expect("rate-limited refresh should not block launch preparation");

        assert_eq!(refresh_count, 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn app_launch_preparation_scopes_workspace_live_sync_roots_to_selected_repo_and_local_links() {
        let _env = crate::env_lock::lock();
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
