use rand::distributions::{Alphanumeric, DistString};
use std::path::{Path, PathBuf};

use crate::agent::AgentInstance;
use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::mcp::CharioxMcpServerConfig;
use crate::provider::{
    normalize_provider_resume_model, LaunchProviderRequest, ProviderResumeState, RuntimeProviderRun,
};
use crate::session::RuntimeSession;

pub(crate) fn default_provider_env_remove(config: &DaemonConfig) -> Vec<String> {
    let credentials = crate::credential::load_user_credentials().unwrap_or_default();
    let _ = config;
    let mut names = crate::secret::RuntimeSecretService::credential_env_names_from(&credentials)
        .into_iter()
        .collect::<Vec<_>>();
    for name in crate::provider::managed_provider_control_env_remove() {
        if !names.iter().any(|existing| existing == &name) {
            names.push(name);
        }
    }
    names
}

pub(crate) fn resolve_mcp_credentials_for_launch(
    config: &DaemonConfig,
    servers: Vec<CharioxMcpServerConfig>,
) -> Result<Vec<CharioxMcpServerConfig>, DaemonError> {
    if servers.is_empty() {
        return Ok(servers);
    }
    let credentials = crate::credential::load_user_credentials()?;
    let service = crate::secret::RuntimeSecretService::with_vault_config(
        credentials,
        &config.user_config.credential_vault,
    )?;
    servers
        .into_iter()
        .map(|server| server.resolve_credential_bindings(&service))
        .collect()
}

pub(crate) fn sanitize_resume_state_for_launch(
    request: &LaunchProviderRequest,
    agent: &AgentInstance,
) -> ProviderResumeState {
    let resume_state = agent.provider_resume_state().clone();
    let requested_variant = request
        .variant
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    let agent_variant = agent.effort().filter(|value| !value.trim().is_empty());
    let requested_model =
        normalize_provider_resume_model(&request.adapter_key, request.model.as_str());
    let agent_model = agent
        .model()
        .map(|model| normalize_provider_resume_model(&request.adapter_key, model));
    let model_changed = agent_model
        .as_deref()
        .is_some_and(|model| model != requested_model);
    let variant_changed = agent_variant != requested_variant;
    let model_or_variant_changed = model_changed || variant_changed;
    if !model_or_variant_changed {
        return resume_state;
    }

    resume_state.without_provider_session_id(&request.adapter_key)
}

pub(crate) fn granted_mcp_servers_for_agent_launch(
    operation: &'static str,
    session: &RuntimeSession,
    agent: &AgentInstance,
) -> Result<Vec<CharioxMcpServerConfig>, DaemonError> {
    let mcp_grants = agent.mcp_grants();
    if mcp_grants.is_empty() {
        return Ok(Vec::new());
    }
    let roots = crate::mcp::CharioxMcpRegistry::user_root()
        .map(|root| vec![root])
        .unwrap_or_default();
    let registry = crate::mcp::CharioxMcpRegistry::new(roots);
    let mut servers = Vec::new();
    for grant in mcp_grants {
        let Some(server) = registry.get(&grant)? else {
            crate::logging::warn_with_fields(
                "daemon.provider",
                "skipping missing MCP extension grant during provider launch",
                serde_json::json!({
                    "operation": operation,
                    "session_id": session.id(),
                    "agent_id": agent.id(),
                    "agent_ref": agent.agent_ref(),
                    "mcp": grant,
                }),
            );
            continue;
        };
        if server.enabled {
            servers.push(server);
        }
    }
    Ok(servers)
}

pub(crate) fn apply_metaagent_launch_policy(
    mut request: LaunchProviderRequest,
    agent: Option<&AgentInstance>,
) -> LaunchProviderRequest {
    if !agent.is_some_and(AgentInstance::is_metaagent) {
        return request;
    }
    request = request
        .with_provider_config_override("features.multi_agent", serde_json::json!(false))
        .with_provider_config_override("chariox.metaagent_tools_only", serde_json::json!(true))
        .with_mcp_servers(Vec::new())
        .with_remote_extension_manifest(crate::extension::RemoteExtensionManifest::default());
    request
}

pub(crate) fn failed_provider_resume_state_replacement(
    run: &RuntimeProviderRun,
    error: &DaemonError,
) -> Option<ProviderResumeState> {
    let DaemonError::ProviderProtocol { operation, .. } = error else {
        return None;
    };
    run.resume_state()
        .replacement_after_provider_resume_failure(run.adapter_key(), operation)
}

pub(crate) fn failed_provider_resume_state_replacement_from_message(
    run: &RuntimeProviderRun,
    message: &str,
) -> Option<ProviderResumeState> {
    let provider_message = message
        .strip_prefix("Provider prompt dispatch failed: ")
        .unwrap_or(message)
        .trim();
    let operation = if provider_message.contains("thread/resume") {
        "thread/resume"
    } else if provider_message.contains("codex_thread_resume") {
        "codex_thread_resume"
    } else if run.adapter_key().eq_ignore_ascii_case("opencode")
        && (provider_message.starts_with("Provider finish_reason: network_error")
            || provider_message.contains("Upstream request failed: Endpoint is unavailable"))
    {
        "provider_stream/network_error"
    } else if run.adapter_key().eq_ignore_ascii_case("opencode")
        && provider_message.starts_with("OpenCode became idle without producing assistant output")
    {
        "provider_stream/empty_idle_assistant"
    } else {
        return None;
    };
    run.resume_state()
        .replacement_after_provider_resume_failure(run.adapter_key(), operation)
}

pub(crate) fn generate_runtime_mcp_auth_token() -> String {
    Alphanumeric.sample_string(&mut rand::thread_rng(), 32)
}

pub(crate) fn workspace_live_sync_protected_roots(
    session: &RuntimeSession,
    working_directory: Option<&Path>,
    host_machine_id: &str,
    host_daemon_id: &str,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(root) = working_directory
        .and_then(resolve_git_root)
        .or_else(|| working_directory.map(PathBuf::from))
    {
        push_unique_root(&mut roots, root);
    }
    for link in session.workspace_links() {
        for attachment in link.attachments() {
            if attachment.machine_id() == host_machine_id
                && attachment.kernel_id() == host_daemon_id
            {
                push_unique_root(&mut roots, PathBuf::from(attachment.repo_root()));
            }
        }
    }
    roots
}

pub(crate) fn registered_workflow_runtime_worktree_root(
    session: &RuntimeSession,
    agent_id: Option<&str>,
    working_directory: Option<&Path>,
) -> Option<PathBuf> {
    let agent_id = agent_id?;
    let working_directory = working_directory?;
    let canonical_working_directory = working_directory.canonicalize().ok()?;
    session
        .workflow_runtime_instances()
        .iter()
        .filter(|instance| !instance.primary())
        .find_map(|instance| {
            let owns_agent = instance
                .node_agent_ids()
                .values()
                .any(|runtime_agent_id| runtime_agent_id == agent_id);
            if !owns_agent {
                return None;
            }
            let root = PathBuf::from(instance.worktree_id());
            let canonical_root = root.canonicalize().ok()?;
            canonical_working_directory
                .starts_with(canonical_root)
                .then_some(root)
        })
}

fn resolve_git_root(path: &Path) -> Option<PathBuf> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .arg("rev-parse")
        .arg("--show-toplevel")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let root = String::from_utf8(output.stdout).ok()?;
    let root = root.trim();
    (!root.is_empty()).then(|| PathBuf::from(root))
}

fn push_unique_root(roots: &mut Vec<PathBuf>, root: PathBuf) {
    if !roots.iter().any(|existing| existing == &root) {
        roots.push(root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opencode_run_with_resume_state() -> RuntimeProviderRun {
        let mut run = RuntimeProviderRun::from_control_capability_inference(
            "provider-run-1",
            "session-1".to_string(),
            Some("agent-1".to_string()),
            "opencode".to_string(),
        );
        run.set_resume_state(ProviderResumeState::from_opencode_session_id(
            "open-session-1",
        ));
        run
    }

    #[test]
    fn opencode_network_error_retires_only_the_failed_resume_session() {
        let run = opencode_run_with_resume_state();

        let replacement = failed_provider_resume_state_replacement_from_message(
            &run,
            "Provider prompt dispatch failed: Provider finish_reason: network_error (retries exhausted)",
        )
        .expect("retry-exhausted OpenCode stream failures should retire the session");

        assert_eq!(replacement.opencode_session_id(), None);
    }

    #[test]
    fn opencode_empty_idle_assistant_retires_only_the_failed_resume_session() {
        let run = opencode_run_with_resume_state();

        let replacement = failed_provider_resume_state_replacement_from_message(
            &run,
            "OpenCode became idle without producing assistant output. Chariox closed this turn so the agent can be retried with a fresh provider session.",
        )
        .expect("an empty idle assistant poisons the resumed OpenCode session");

        assert_eq!(replacement.opencode_session_id(), None);
    }

    #[test]
    fn opencode_unavailable_upstream_retires_only_the_failed_resume_session() {
        let run = opencode_run_with_resume_state();

        let replacement = failed_provider_resume_state_replacement_from_message(
            &run,
            "Provider prompt dispatch failed: Error from provider (Console): Upstream request failed: Endpoint is unavailable.",
        )
        .expect("an unavailable OpenCode upstream poisons the resumed provider session");

        assert_eq!(replacement.opencode_session_id(), None);
    }

    #[test]
    fn opencode_resume_recovery_ignores_unrelated_terminal_failures() {
        let run = opencode_run_with_resume_state();

        assert!(failed_provider_resume_state_replacement_from_message(
            &run,
            "provider request failed: network_error",
        )
        .is_none());
    }

    #[test]
    fn workspace_live_sync_protected_roots_include_working_directory_and_local_links() {
        let mut session = crate::session::RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "/repo/main",
            "machine-1",
            "daemon-1",
        );
        session.create_workspace_link(crate::session::WorkspaceLinkDefinition::new(
            "link-1",
            "session-1",
            "shared",
            "local",
        ));
        session
            .workspace_link_mut("link-1")
            .expect("link should exist")
            .attach(crate::session::WorkspaceLinkAttachment::new(
                "link-1",
                "local",
                "machine-1",
                "daemon-1",
                "/repo/attached",
                None,
                None,
            ));
        session
            .workspace_link_mut("link-1")
            .expect("link should exist")
            .attach(crate::session::WorkspaceLinkAttachment::new(
                "link-1",
                "peer",
                "remote-machine",
                "remote-daemon",
                "/remote/repo",
                None,
                None,
            ));

        let roots = workspace_live_sync_protected_roots(
            &session,
            Some(Path::new("/repo/main")),
            "machine-1",
            "daemon-1",
        );

        assert_eq!(
            roots,
            vec![PathBuf::from("/repo/main"), PathBuf::from("/repo/attached"),]
        );
    }

    #[test]
    fn workspace_live_sync_protected_roots_do_not_include_sibling_repos() {
        let base = std::env::temp_dir().join(format!(
            "chariox-live-sync-root-scope-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let selected = base.join("selected");
        let selected_child = selected.join("src");
        let sibling = base.join("sibling");
        std::fs::create_dir_all(&selected_child).expect("selected repo fixture should exist");
        std::fs::create_dir_all(&sibling).expect("sibling repo fixture should exist");
        run_git_init(&selected);
        run_git_init(&sibling);
        let session = crate::session::RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            selected_child.to_string_lossy().to_string(),
            "machine-1",
            "daemon-1",
        );

        let roots = workspace_live_sync_protected_roots(
            &session,
            Some(selected_child.as_path()),
            "machine-1",
            "daemon-1",
        );

        let canonical_selected = selected
            .canonicalize()
            .expect("selected repo should canonicalize");
        let _ = std::fs::remove_dir_all(&base);
        assert_eq!(roots, vec![canonical_selected]);
    }

    fn run_git_init(path: &Path) {
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
