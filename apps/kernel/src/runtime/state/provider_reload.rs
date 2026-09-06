//! Provider reload policy for launch-time runtime changes.
//!
//! This module owns the shared decision and relaunch path for changes that require a provider
//! process to be started with different launch inputs.

use super::*;

#[derive(Debug, Clone)]
pub(crate) enum ProviderReloadTrigger {
    AgentMcpChanged {
        session_id: String,
        agent_id: String,
        name: String,
    },
    SessionWorkspaceLiveSyncModeChanged {
        session_id: String,
    },
    UserConfigChanged {
        path: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderReloadOutcome {
    Unaffected,
    Reloaded,
    Deferred,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderLaunchFingerprint {
    runtime_mcp_server_url: Option<String>,
    mcp_servers: Vec<crate::mcp::CharioxMcpServerConfig>,
    provider_env_remove: Vec<String>,
    provider_config_overrides: std::collections::BTreeMap<String, serde_json::Value>,
    write_access_mode: crate::provider::ProviderWriteAccessMode,
    execution_mode: crate::provider::AgentExecutionMode,
    permission_level: crate::provider::AgentPermissionLevel,
}

impl ProviderLaunchFingerprint {
    fn from_run(run: &crate::provider::RuntimeProviderRun) -> Self {
        Self {
            runtime_mcp_server_url: run.runtime_mcp_server_url().map(str::to_string),
            mcp_servers: run.mcp_servers().to_vec(),
            provider_env_remove: run.pty_env_remove().to_vec(),
            provider_config_overrides: run.provider_config_overrides().clone(),
            write_access_mode: run.write_access_mode(),
            execution_mode: run.execution_mode(),
            permission_level: run.permission_level(),
        }
    }

    fn from_request(request: &crate::provider::LaunchProviderRequest) -> Self {
        Self {
            runtime_mcp_server_url: request
                .runtime_mcp_binding
                .as_ref()
                .map(|binding| binding.server_url.clone()),
            mcp_servers: request.mcp_servers.clone(),
            provider_env_remove: request.provider_env_remove.clone(),
            provider_config_overrides: request.provider_config_overrides.clone(),
            write_access_mode: request.write_access_mode,
            execution_mode: request.execution_mode.unwrap_or_default(),
            permission_level: request.permission_level.unwrap_or_default(),
        }
    }
}

impl KernelRuntimeState {
    pub(crate) async fn apply_provider_reload_policy(
        &self,
        trigger: ProviderReloadTrigger,
    ) -> Result<Vec<ProviderReloadOutcome>, DaemonError> {
        let mut outcomes = Vec::new();
        match trigger {
            ProviderReloadTrigger::AgentMcpChanged {
                session_id,
                agent_id,
                name,
            } => {
                let reason = format!("MCP `{name}`");
                outcomes.push(
                    self.reload_agent_provider_for_policy(&session_id, &agent_id, &reason)
                        .await?,
                );
            }
            ProviderReloadTrigger::SessionWorkspaceLiveSyncModeChanged { session_id } => {
                let session = self.owned.session_store.get_session(&session_id)?;
                let runs = self.owned.provider_store.list_runs();
                let agent_ids = active_agent_provider_run_ids_for_session(&runs, session.id());
                for agent_id in agent_ids {
                    outcomes.push(
                        self.reload_agent_provider_for_policy(
                            session.id(),
                            &agent_id,
                            "session workspace live sync mode",
                        )
                        .await?,
                    );
                }
                if outcomes.is_empty() {
                    outcomes.push(ProviderReloadOutcome::Unaffected);
                }
            }
            ProviderReloadTrigger::UserConfigChanged { path } => {
                if !user_config_path_requires_provider_reload(&path) {
                    outcomes.push(ProviderReloadOutcome::Unaffected);
                    return Ok(outcomes);
                }
                for run in self.owned.provider_store.list_runs() {
                    let Some(agent_id) = run.agent_instance_id().map(str::to_string) else {
                        continue;
                    };
                    if !matches!(
                        run.state(),
                        crate::provider::ProviderRunState::Running
                            | crate::provider::ProviderRunState::Starting
                    ) {
                        continue;
                    }
                    let reason = format!("config `{path}`");
                    outcomes.push(
                        self.reload_agent_provider_for_policy(run.session_id(), &agent_id, &reason)
                            .await?,
                    );
                }
                if outcomes.is_empty() {
                    outcomes.push(ProviderReloadOutcome::Unaffected);
                }
            }
        }
        Ok(outcomes)
    }

    pub(super) async fn reload_agent_provider_for_policy(
        &self,
        session_id: &str,
        agent_id: &str,
        reason: &str,
    ) -> Result<ProviderReloadOutcome, DaemonError> {
        match self
            .reload_agent_provider_if_idle(session_id, agent_id, reason)
            .await?
        {
            ProviderReloadOutcome::Deferred => {
                self.remember_pending_provider_reload(session_id, agent_id, reason);
                Ok(ProviderReloadOutcome::Deferred)
            }
            outcome => Ok(outcome),
        }
    }

    pub(super) async fn reload_agent_provider_if_idle(
        &self,
        session_id: &str,
        agent_id: &str,
        reason: &str,
    ) -> Result<ProviderReloadOutcome, DaemonError> {
        let (launch_request, runtime_init_delay_ms, terminated_run_id) = {
            let owned = &self.owned;
            if owned
                .prompt_state_owner
                .active_prompt_for_agent(&owned.session_store.get_session(session_id)?, agent_id)
                .is_some()
            {
                if let Some(run) = owned.provider_store.get_run_for_agent(session_id, agent_id) {
                    owned.record_notice(
                        session_id,
                        Some(run.id()),
                        owned.attachment_store.list_session_attachment_ids(session_id),
                        format!(
                            "Provider reload for {reason} is pending until agent `{agent_id}` is idle."
                        ),
                    );
                }
                return Ok(ProviderReloadOutcome::Deferred);
            }
            let Some(run) = owned.provider_store.get_run_for_agent(session_id, agent_id) else {
                return Ok(ProviderReloadOutcome::Unaffected);
            };
            if !crate::provider::provider_run_supports_policy_reload(&run) {
                return Ok(ProviderReloadOutcome::Unaffected);
            }
            let config = owned.config_projection.snapshot();
            let durable_resume_state = owned
                .agent_store
                .get_agent(agent_id)?
                .provider_resume_state()
                .clone();
            let mut launch_request =
                policy_reload_launch_request(&run, agent_id, durable_resume_state);
            launch_request = launch_request.with_workspace_live_sync_mode(
                crate::provider::provider_workspace_live_sync_mode_for_session(
                    run.provider(),
                    &config,
                    owned.session_store.get_session(session_id).ok().as_ref(),
                ),
            );
            let launch_request = self
                .prepare_provider_launch_request_with_vault(launch_request, "reload provider run")
                .await?;
            let has_active_prompt = owned
                .prompt_state_owner
                .active_prompt_for_agent(&owned.session_store.get_session(session_id)?, agent_id)
                .is_some();
            let current_run = owned.provider_store.get_run_for_agent(session_id, agent_id);
            if current_run.is_none() {
                return Ok(ProviderReloadOutcome::Unaffected);
            }
            if !provider_reload_snapshot_is_still_current(
                run.id(),
                current_run.as_ref(),
                has_active_prompt,
            ) {
                if !has_active_prompt {
                    return Ok(ProviderReloadOutcome::Deferred);
                }
                owned.record_notice(
                    session_id,
                    Some(run.id()),
                    owned
                        .attachment_store
                        .list_session_attachment_ids(session_id),
                    format!(
                        "Provider reload for {reason} is pending until agent `{agent_id}` is idle."
                    ),
                );
                return Ok(ProviderReloadOutcome::Deferred);
            }
            let run = current_run.expect("current provider run was checked above");
            if ProviderLaunchFingerprint::from_run(&run)
                == ProviderLaunchFingerprint::from_request(&launch_request)
            {
                return Ok(ProviderReloadOutcome::Unaffected);
            }

            let mut terminated_run_id = None;
            if run.state() != crate::provider::ProviderRunState::Ended {
                terminated_run_id = Some(run.id().to_string());
                let outcome = owned
                    .provider_store
                    .terminate_run_provider_only(session_id, run.id())?;
                owned.clear_active_provider_run_session_pointer(session_id, outcome.run().id())?;
                owned.provider_run_projection.update(outcome.into_run());
            }
            owned.record_notice(
                session_id,
                None,
                owned
                    .attachment_store
                    .list_session_attachment_ids(session_id),
                format!(
                    "Reloading provider conversation for agent `{agent_id}` after {reason} changed."
                ),
            );
            (
                launch_request,
                config.provider_runtime_init_delay_ms,
                terminated_run_id,
            )
        };

        self.spawn_provider_relaunch(
            launch_request,
            runtime_init_delay_ms,
            terminated_run_id,
            12_000,
        );
        Ok(ProviderReloadOutcome::Reloaded)
    }
}

fn provider_reload_snapshot_is_still_current(
    expected_run_id: &str,
    current_run: Option<&crate::provider::RuntimeProviderRun>,
    has_active_prompt: bool,
) -> bool {
    !has_active_prompt && current_run.is_some_and(|run| run.id() == expected_run_id)
}

fn policy_reload_launch_request(
    run: &crate::provider::RuntimeProviderRun,
    agent_id: &str,
    durable_resume_state: crate::provider::ProviderResumeState,
) -> crate::provider::LaunchProviderRequest {
    crate::provider::LaunchProviderRequest::new(
        run.session_id(),
        run.adapter_key(),
        run.provider(),
        run.account_profile(),
        run.model(),
    )
    .with_agent_id(agent_id)
    .with_owner_user_id(run.owner_user_id().to_string())
    .with_variant(run.variant().map(str::to_string))
    .with_resume_state(durable_resume_state)
}

fn user_config_path_requires_provider_reload(path: &str) -> bool {
    path == "providers.workspace_live_sync"
}

fn active_agent_provider_run_ids_for_session(
    runs: &[crate::provider::RuntimeProviderRun],
    session_id: &str,
) -> std::collections::BTreeSet<String> {
    runs.iter()
        .filter(|run| {
            run.session_id() == session_id
                && matches!(
                    run.state(),
                    crate::provider::ProviderRunState::Running
                        | crate::provider::ProviderRunState::Starting
                )
        })
        .filter_map(|run| run.agent_instance_id().map(str::to_string))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::provider::{
        AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, ProviderResumeState,
    };

    use super::{
        active_agent_provider_run_ids_for_session, policy_reload_launch_request,
        provider_reload_snapshot_is_still_current,
    };

    #[test]
    fn provider_reload_uses_only_durable_resume_state() {
        let run = provider_run_with_resume_state(
            "run-claude",
            "session-1",
            "agent-1",
            ProviderResumeState::from_claude_session_id("unconfirmed-runtime-session"),
        );

        let fresh_request =
            policy_reload_launch_request(&run, "agent-1", ProviderResumeState::default());
        assert!(
            fresh_request.resume_state.is_none(),
            "a provider-generated session id must not be resumed before the first turn confirms it"
        );

        let confirmed_request = policy_reload_launch_request(
            &run,
            "agent-1",
            ProviderResumeState::from_claude_session_id("confirmed-session"),
        );
        assert_eq!(
            confirmed_request
                .resume_state
                .as_ref()
                .and_then(ProviderResumeState::claude_session_id),
            Some("confirmed-session")
        );
    }

    #[test]
    fn provider_reload_policy_selects_active_agent_runs_for_session_mode_changes() {
        let mut running = provider_run("run-1", "session-1", Some("agent-1"));
        running.mark_running();
        let starting = provider_run("run-2", "session-1", Some("agent-2"));
        let mut ended = provider_run("run-3", "session-1", Some("agent-3"));
        ended.mark_ended();
        let mut other_session = provider_run("run-4", "session-2", Some("agent-4"));
        other_session.mark_running();
        let mut session_run = provider_run("run-5", "session-1", None);
        session_run.mark_running();

        assert_eq!(
            active_agent_provider_run_ids_for_session(
                &[running, starting, ended, other_session, session_run],
                "session-1",
            ),
            ["agent-1".to_string(), "agent-2".to_string()]
                .into_iter()
                .collect()
        );
    }

    #[test]
    fn provider_reload_revalidates_idle_run_identity_after_async_preparation() {
        let expected = provider_run("expected", "session-1", Some("agent-1"));
        let replacement = provider_run("replacement", "session-1", Some("agent-1"));

        assert!(provider_reload_snapshot_is_still_current(
            expected.id(),
            Some(&expected),
            false,
        ));
        assert!(!provider_reload_snapshot_is_still_current(
            expected.id(),
            Some(&expected),
            true,
        ));
        assert!(!provider_reload_snapshot_is_still_current(
            expected.id(),
            Some(&replacement),
            false,
        ));
        assert!(!provider_reload_snapshot_is_still_current(
            expected.id(),
            None,
            false,
        ));
    }

    fn provider_run(
        run_id: &str,
        session_id: &str,
        agent_id: Option<&str>,
    ) -> crate::provider::RuntimeProviderRun {
        let mut request =
            LaunchProviderRequest::new(session_id, "codex", "codex", "default", "gpt-5.2");
        if let Some(agent_id) = agent_id {
            request = request.with_agent_id(agent_id);
        }
        crate::provider::RuntimeProviderRun::new(
            run_id,
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: "codex:test".to_string(),
                pty_target: None,
                pty_program: Some("codex".to_string()),
                pty_args: Vec::new(),
                pty_env: BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: Some("ws://127.0.0.1:1".to_string()),
            },
        )
    }

    fn provider_run_with_resume_state(
        run_id: &str,
        session_id: &str,
        agent_id: &str,
        resume_state: ProviderResumeState,
    ) -> crate::provider::RuntimeProviderRun {
        crate::provider::RuntimeProviderRun::new(
            run_id,
            &LaunchProviderRequest::new(session_id, "claude", "claude-p", "default", "haiku")
                .with_agent_id(agent_id)
                .with_resume_state(resume_state),
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::External,
                process_label: "claude".to_string(),
                pty_target: None,
                pty_program: Some("claude".to_string()),
                pty_args: Vec::new(),
                pty_env: BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: Some("stdio://claude".to_string()),
            },
        )
    }
}
