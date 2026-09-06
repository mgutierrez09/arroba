use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::error::DaemonError;
use crate::provider::{AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult};

use self::mcp_config::runtime_mcp_config;
use self::ports::resolve_codex_launch_port;
use super::executable_resolution::ExecutableResolutionState;

mod catalog_endpoint;
mod mcp_config;
mod ports;

pub use catalog_endpoint::codex_catalog_endpoint;
pub(crate) use catalog_endpoint::{
    ensure_codex_account_endpoint, invalidate_codex_account_endpoint,
    shutdown_codex_account_endpoints,
};

const CODEX_ENV_OVERRIDE: &str = "CHARIOX_CODEX_BIN";
static CODEX_EXECUTABLE_RESOLUTION: ExecutableResolutionState =
    ExecutableResolutionState::new("codex");
const CODEX_BIND_HOST_OVERRIDE: &str = "CHARIOX_CODEX_BIND_HOST";
pub(crate) const CODEX_MCP_TOKEN_ENV: &str = "CHARIOX_MCP_TOKEN";
const CODEX_SESSION_ENV_VARS: &[&str] = &[
    "CODEX_THREAD_ID",
    "CODEX_TURN_METADATA_HEADER",
    "CODEX_TURN_STATE_HEADER",
    "CODEX_STARTING_DIFF",
    "CODEX_ESCALATE_SOCKET",
    "RUST_LOG",
    // This is macOS process bookkeeping, not provider configuration. Carrying
    // a parent's 0x2 value into a fresh Codex app-server breaks its auth HTTP
    // requests, even though the same request succeeds without the inherited flag.
    #[cfg(target_os = "macos")]
    "XPC_FLAGS",
];

pub fn resolve_codex_executable() -> Result<PathBuf, DaemonError> {
    let _guard = crate::env_lock::lock();
    resolve_codex_executable_unlocked()
}

fn resolve_codex_executable_unlocked() -> Result<PathBuf, DaemonError> {
    if let Some(path) = env::var_os(CODEX_ENV_OVERRIDE).map(PathBuf::from) {
        return CODEX_EXECUTABLE_RESOLUTION
            .resolve(|| resolve_candidate(path.clone(), true))
            .ok_or_else(|| DaemonError::ProviderExecutableNotFound {
                adapter_key: "codex".to_string(),
                executable: env::var(CODEX_ENV_OVERRIDE).unwrap_or_else(|_| "codex".to_string()),
            });
    }

    CODEX_EXECUTABLE_RESOLUTION
        .resolve(|| resolve_candidate(PathBuf::from("codex"), false))
        .ok_or_else(|| DaemonError::ProviderExecutableNotFound {
            adapter_key: "codex".to_string(),
            executable: "codex".to_string(),
        })
}

pub fn plan_codex_launch(
    request: Option<&LaunchProviderRequest>,
) -> Result<ProviderLaunchResult, DaemonError> {
    let _guard = crate::env_lock::lock();
    plan_codex_launch_unlocked(request)
}

fn plan_codex_launch_unlocked(
    request: Option<&LaunchProviderRequest>,
) -> Result<ProviderLaunchResult, DaemonError> {
    if let Some(endpoint) = request.and_then(|request| request.structured_endpoint.clone()) {
        let working_directory = request.and_then(|request| request.working_directory.clone());
        return Ok(ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "codex:native-app-server".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: BTreeMap::new(),
            pty_env_remove: codex_provider_env_remove(request),
            working_directory,
            structured_endpoint: Some(endpoint),
        });
    }

    let port = resolve_codex_launch_port(request.is_some())?;
    let endpoint = format!("ws://127.0.0.1:{port}");
    let listen_endpoint = format!("ws://{}:{port}", resolve_codex_bind_host());

    let executable = resolve_codex_executable_unlocked()?;
    let (config_args, mut env) = runtime_mcp_config(request)?;
    if let Some(request) = request {
        env.extend(request.provider_account_env.clone());
    }
    let working_directory = request.and_then(|request| request.working_directory.clone());
    Ok(ProviderLaunchResult {
        endpoint_mode: AgentEndpointMode::Managed,
        process_label: "codex:app-server".to_string(),
        pty_target: None,
        pty_program: Some(executable.display().to_string()),
        pty_args: {
            let mut args = vec!["app-server".to_string()];
            args.extend(config_args);
            args.extend(["--listen".to_string(), listen_endpoint]);
            args
        },
        pty_env: env,
        pty_env_remove: codex_provider_env_remove(request),
        working_directory,
        structured_endpoint: Some(endpoint),
    })
}

fn resolve_codex_bind_host() -> String {
    env::var(CODEX_BIND_HOST_OVERRIDE).unwrap_or_else(|_| "127.0.0.1".to_string())
}

fn codex_provider_env_remove(request: Option<&LaunchProviderRequest>) -> Vec<String> {
    let mut names = request
        .map(|request| request.provider_env_remove.clone())
        .unwrap_or_default();
    for name in CODEX_SESSION_ENV_VARS {
        if !names.iter().any(|existing| existing == name) {
            names.push((*name).to_string());
        }
    }
    names
}

pub fn logout_codex(provider_account_env: &BTreeMap<String, String>) -> Result<(), DaemonError> {
    let executable = resolve_codex_executable()?;
    let mut command = crate::provider::managed_isolated_utility_command(
        executable.display().to_string(),
        vec!["logout".to_string()],
        provider_account_env.clone(),
        None,
        "codex:logout",
    )?;
    for name in crate::account_profile::provider_auth_env_vars("codex") {
        command.env_remove(name);
    }
    let status = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "logout_codex",
            message: format!("failed to start `codex logout`: {error}"),
        })?;
    if !status.success() {
        return Err(DaemonError::LocalTransport {
            operation: "logout_codex",
            message: format!("`codex logout` exited unsuccessfully: {status}"),
        });
    }
    Ok(())
}

fn resolve_candidate(candidate: PathBuf, treat_as_literal_path: bool) -> Option<PathBuf> {
    if treat_as_literal_path || candidate.components().count() > 1 {
        return candidate.exists().then_some(candidate);
    }

    if candidate.is_absolute() && candidate.exists() {
        return Some(candidate);
    }

    let path_var = env::var_os("PATH")?;
    env::split_paths(&path_var)
        .map(|directory| directory.join(&candidate))
        .find(|path| is_executable_file(path))
}

fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use crate::mcp::CharioxMcpServerConfig;
    use crate::provider::{AgentEndpointMode, LaunchProviderRequest, RuntimeMcpBinding};

    use super::{logout_codex, plan_codex_launch, resolve_codex_executable};

    fn env_guard() -> crate::env_lock::EnvGuard {
        crate::env_lock::lock()
    }

    #[test]
    fn resolves_override_path_for_tests() {
        let _guard = env_guard();
        let path =
            std::env::temp_dir().join(format!("chariox-codex-resolve-test-{}", std::process::id()));
        fs::write(&path, "#!/bin/sh\nexit 0\n").expect("fixture should exist");
        std::env::set_var("CHARIOX_CODEX_BIN", &path);

        let resolved = resolve_codex_executable().expect("override path should resolve");

        std::env::remove_var("CHARIOX_CODEX_BIN");
        let _ = fs::remove_file(&path);
        assert_eq!(resolved, path);
    }

    #[test]
    fn plans_codex_app_server_launch() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "chariox-codex-resolve-test-{}-serve",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("CHARIOX_CODEX_BIN", &path);
        std::env::set_var("CHARIOX_CODEX_PORT", "43142");

        let launch = plan_codex_launch(None).expect("launch plan should resolve");

        std::env::remove_var("CHARIOX_CODEX_BIN");
        std::env::remove_var("CHARIOX_CODEX_PORT");
        let _ = fs::remove_file(&path);

        assert_eq!(launch.endpoint_mode, AgentEndpointMode::Managed);
        assert_eq!(
            launch.pty_program.as_deref(),
            Some(path.to_string_lossy().as_ref())
        );
        assert_eq!(
            launch.pty_args,
            vec![
                "app-server".to_string(),
                "--listen".to_string(),
                "ws://127.0.0.1:43142".to_string(),
            ]
        );
        assert_eq!(
            launch.structured_endpoint.as_deref(),
            Some("ws://127.0.0.1:43142")
        );
    }

    #[test]
    fn plans_codex_launch_scrubs_inherited_session_env() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "chariox-codex-resolve-test-{}-env-remove",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("CHARIOX_CODEX_BIN", &path);
        std::env::set_var("CHARIOX_CODEX_PORT", "43142");
        let request =
            LaunchProviderRequest::new("session-1", "codex", "codex", "default", "codex-mini")
                .with_provider_env_remove(vec!["OPENAI_API_KEY".to_string()]);

        let launch = plan_codex_launch(Some(&request)).expect("launch plan should resolve");

        std::env::remove_var("CHARIOX_CODEX_BIN");
        std::env::remove_var("CHARIOX_CODEX_PORT");
        let _ = fs::remove_file(&path);

        assert!(launch
            .pty_env_remove
            .iter()
            .any(|name| name == "OPENAI_API_KEY"));
        assert!(launch
            .pty_env_remove
            .iter()
            .any(|name| name == "CODEX_THREAD_ID"));
        assert!(launch
            .pty_env_remove
            .iter()
            .any(|name| name == "CODEX_TURN_METADATA_HEADER"));
        assert!(launch
            .pty_env_remove
            .iter()
            .any(|name| name == "CODEX_TURN_STATE_HEADER"));
        assert!(launch.pty_env_remove.iter().any(|name| name == "RUST_LOG"));
        #[cfg(target_os = "macos")]
        assert!(
            launch.pty_env_remove.iter().any(|name| name == "XPC_FLAGS"),
            "Codex must not inherit macOS XPC process flags that break account-service networking"
        );
    }

    #[test]
    fn plans_codex_catalog_launch_without_explicit_port_override() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "chariox-codex-resolve-test-{}-managed-catalog-port",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("CHARIOX_CODEX_BIN", &path);
        std::env::remove_var("CHARIOX_CODEX_PORT");

        let launch = plan_codex_launch(None).expect("managed catalog port should resolve");

        std::env::remove_var("CHARIOX_CODEX_BIN");
        std::env::remove_var("CHARIOX_CODEX_PORT");
        let _ = fs::remove_file(&path);

        assert_eq!(launch.endpoint_mode, AgentEndpointMode::Managed);
        assert!(launch
            .structured_endpoint
            .as_deref()
            .is_some_and(|endpoint| endpoint.starts_with("ws://127.0.0.1:")));
    }

    #[test]
    fn injects_runtime_mcp_config_into_managed_launch() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "chariox-codex-resolve-test-{}-mcp",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("CHARIOX_CODEX_BIN", &path);
        std::env::set_var("CHARIOX_CODEX_PORT", "43143");

        let request =
            LaunchProviderRequest::new("session-1", "codex", "codex", "default", "codex-mini")
                .with_workspace_live_sync_managed()
                .with_runtime_mcp_binding(RuntimeMcpBinding::new(
                    "http://127.0.0.1:43120/mcp",
                    "token-123",
                ));
        let launch = plan_codex_launch(Some(&request)).expect("launch plan should resolve");

        std::env::remove_var("CHARIOX_CODEX_BIN");
        std::env::remove_var("CHARIOX_CODEX_PORT");
        let _ = fs::remove_file(&path);

        assert_eq!(
            launch.pty_env.get("CHARIOX_MCP_TOKEN").map(String::as_str),
            Some("token-123")
        );
        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg.contains("mcp_servers.chariox.url")));
        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg == "mcp_servers.chariox.transport=\"streamable_http\""));
        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg.contains("mcp_servers.chariox.bearer_token_env_var")));
        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg == "mcp_servers.chariox.required=true"));
        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg == "mcp_servers.chariox.startup_timeout_sec=90"));
        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg == "mcp_servers.chariox.tool_timeout_sec=300"));
        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg.contains("model_catalog_json")));
        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg == "features.apply_patch_freeform=false"));
        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg == "include_apply_patch_tool=false"));
        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg == "approval_policy=\"never\""));
    }

    #[test]
    fn runtime_mcp_config_does_not_force_workspace_live_sync_overrides_for_unrestricted_launch() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "chariox-codex-resolve-test-{}-runtime-mcp-unrestricted",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("CHARIOX_CODEX_BIN", &path);
        std::env::set_var("CHARIOX_CODEX_PORT", "43143");

        let request =
            LaunchProviderRequest::new("session-1", "codex", "codex", "default", "codex-mini")
                .with_runtime_mcp_binding(RuntimeMcpBinding::new(
                    "http://127.0.0.1:43120/mcp",
                    "token-123",
                ));
        let launch = plan_codex_launch(Some(&request)).expect("launch plan should resolve");

        std::env::remove_var("CHARIOX_CODEX_BIN");
        std::env::remove_var("CHARIOX_CODEX_PORT");
        let _ = fs::remove_file(&path);

        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg.contains("mcp_servers.chariox.url")));
        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg == "mcp_servers.chariox.transport=\"streamable_http\""));
        assert!(!launch
            .pty_args
            .iter()
            .any(|arg| arg.contains("model_catalog_json")));
        assert!(!launch
            .pty_args
            .iter()
            .any(|arg| arg == "features.apply_patch_freeform=false"));
        assert!(!launch
            .pty_args
            .iter()
            .any(|arg| arg == "include_apply_patch_tool=false"));
        assert!(!launch
            .pty_args
            .iter()
            .any(|arg| arg == "approval_policy=\"never\""));
    }

    #[test]
    fn injects_granted_mcp_config_into_managed_launch() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "chariox-codex-resolve-test-{}-granted-mcp",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("CHARIOX_CODEX_BIN", &path);
        std::env::set_var("CHARIOX_CODEX_PORT", "43144");

        let request =
            LaunchProviderRequest::new("session-1", "codex", "codex", "default", "codex-mini")
                .with_mcp_servers(vec![CharioxMcpServerConfig::stdio(
                    "browser",
                    "npx",
                    vec!["@playwright/mcp@latest".to_string()],
                )]);
        let launch = plan_codex_launch(Some(&request)).expect("launch plan should resolve");

        std::env::remove_var("CHARIOX_CODEX_BIN");
        std::env::remove_var("CHARIOX_CODEX_PORT");
        let _ = fs::remove_file(&path);

        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg == "mcp_servers.browser.command=\"npx\""));
        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg.contains("mcp_servers.browser.args")));
        assert!(!launch
            .pty_args
            .iter()
            .any(|arg| arg.contains("mcp_servers.chariox.url")));
    }

    #[test]
    fn renders_granted_mcp_as_provider_facing_proxy_when_runtime_mcp_is_bound() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "chariox-codex-resolve-test-{}-proxied-mcp",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("CHARIOX_CODEX_BIN", &path);
        std::env::set_var("CHARIOX_CODEX_PORT", "43144");

        let request =
            LaunchProviderRequest::new("session-1", "codex", "codex", "default", "codex-mini")
                .with_runtime_mcp_binding(RuntimeMcpBinding::new(
                    "http://127.0.0.1:43120/mcp",
                    "token-123",
                ))
                .with_mcp_servers(vec![CharioxMcpServerConfig::stdio(
                    "browser",
                    "npx",
                    vec!["@playwright/mcp@latest".to_string()],
                )]);
        let launch = plan_codex_launch(Some(&request)).expect("launch plan should resolve");

        std::env::remove_var("CHARIOX_CODEX_BIN");
        std::env::remove_var("CHARIOX_CODEX_PORT");
        let _ = fs::remove_file(&path);

        let browser_config = launch
            .pty_args
            .iter()
            .find(|arg| arg.starts_with("mcp_servers.chariox_mcp_browser={"))
            .expect("browser MCP should be rendered as one streamable HTTP table");
        assert!(browser_config.contains("transport=\"streamable_http\""));
        assert!(browser_config.contains("url=\"http://127.0.0.1:43120/mcp/proxy/browser\""));
        assert!(browser_config.contains("bearer_token_env_var=\"CHARIOX_MCP_TOKEN\""));
        assert!(!browser_config.contains("http_headers"));
        assert!(!launch
            .pty_args
            .iter()
            .any(|arg| arg == "mcp_servers.browser.command=\"npx\""));
    }

    #[test]
    fn plans_managed_workspace_live_sync_launch() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "chariox-codex-resolve-test-{}-workspace-live-sync",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nsleep 60\n").expect("fixture should exist");
        std::env::set_var("CHARIOX_CODEX_BIN", &path);
        std::env::set_var("CHARIOX_CODEX_PORT", "43144");
        let request =
            LaunchProviderRequest::new("session-1", "codex", "codex", "default", "codex-mini")
                .with_workspace_live_sync_managed();

        let launch =
            plan_codex_launch(Some(&request)).expect("workspace live sync launch should resolve");

        std::env::remove_var("CHARIOX_CODEX_BIN");
        std::env::remove_var("CHARIOX_CODEX_PORT");
        let _ = fs::remove_file(&path);

        assert_eq!(launch.endpoint_mode, AgentEndpointMode::Managed);
        assert!(launch
            .structured_endpoint
            .as_deref()
            .is_some_and(|endpoint| endpoint.starts_with("ws://127.0.0.1:")));
    }

    #[test]
    fn logout_codex_invokes_the_configured_executable() {
        let _guard = env_guard();
        let path =
            std::env::temp_dir().join(format!("chariox-codex-logout-test-{}", std::process::id()));
        let marker = std::env::temp_dir().join(format!(
            "chariox-codex-logout-marker-{}",
            std::process::id()
        ));
        fs::write(
            &path,
            format!(
                "#!/bin/sh\nprintf '%s' \"$1\" > \"{}\"\nexit 0\n",
                marker.display()
            ),
        )
        .expect("fixture should exist");
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
            .expect("fixture should be executable");
        std::env::set_var("CHARIOX_CODEX_BIN", &path);

        logout_codex(&BTreeMap::new()).expect("logout should succeed");

        std::env::remove_var("CHARIOX_CODEX_BIN");
        let logged = fs::read_to_string(&marker).expect("marker should be written");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&marker);

        assert_eq!(logged, "logout");
    }
}
