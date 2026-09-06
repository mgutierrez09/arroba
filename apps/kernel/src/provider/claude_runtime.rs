use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::TryRecvError;
use std::time::{Duration, Instant};

use rand::RngCore;
use serde_json::json;

use crate::error::DaemonError;
use crate::prompt_assembly::PromptEnvelope;
use crate::terminal::TerminalOutputKind;

use super::claude::materialize_runtime_claude_mcp_config;
use super::managed_isolation::expose_runtime_directory_in_managed_namespace;
use super::{
    AgentExecutionMode, AgentPermissionLevel, ProviderPromptSignalBatch, RuntimeProviderRun,
};

const CLAUDE_EVENT_DRAIN_MAX_MESSAGES: usize = 256;
const DEFAULT_CLAUDE_TURN_STALL_TIMEOUT: Duration = Duration::from_secs(60);

mod events;
mod input;
mod process;
mod state;
mod tool_transcript;
pub(crate) mod usage;
mod watchdog;

use events::apply_claude_message;
use input::claude_user_content;
use process::{spawn_claude_child, stop_child, write_json_line, ClaudeRuntimeMessage};
pub(crate) use state::{ClaudeRunSelection, ClaudeRuntimeBinding, ClaudeRuntimeState};
use usage::apply_claude_usage_capture;
use watchdog::ClaudeTurnStallAction;

#[cfg(test)]
pub(crate) fn initialize_claude_runtime(
    run: &RuntimeProviderRun,
) -> Result<ClaudeRuntimeBinding, DaemonError> {
    initialize_claude_runtime_with_credentials(
        run,
        &crate::provider::ProviderCredentialEnvironment::default(),
    )
}

pub(crate) fn initialize_claude_runtime_with_credentials(
    run: &RuntimeProviderRun,
    credentials: &crate::provider::ProviderCredentialEnvironment,
) -> Result<ClaudeRuntimeBinding, DaemonError> {
    let program = run
        .pty_program()
        .ok_or_else(|| DaemonError::ProviderProtocol {
            provider_run_id: run.id().to_string(),
            operation: "claude_executable_missing",
            message: "Claude provider run did not include an executable".to_string(),
        })?
        .to_string();
    let mut args = run.pty_args().to_vec();
    let mcp_config_file = materialize_runtime_claude_mcp_config(run)?;
    install_claude_mcp_config_argument(
        &mut args,
        mcp_config_file.as_ref().map(|file| file.path()),
    )?;
    let session_id = run
        .resume_state()
        .claude_session_id()
        .map(str::to_string)
        .unwrap_or_else(new_claude_session_id);
    if run.resume_state().claude_session_id().is_none() {
        args.extend(["--session-id".to_string(), session_id.clone()]);
    }
    let env = run.pty_env().clone();
    let context_file = env.get("CHARIOX_CLAUDE_NATIVE_CONTEXT").map(PathBuf::from);
    let settings_file = env.get("CHARIOX_CLAUDE_SETTINGS_FILE").map(PathBuf::from);
    let usage_file = env.get("CHARIOX_CLAUDE_USAGE_FILE").map(PathBuf::from);
    let env_remove = run.pty_env_remove().to_vec();
    let working_directory = run.working_directory().cloned();
    let (child, stdin, receiver) = spawn_claude_child(
        run.id(),
        &program,
        &args,
        &env,
        credentials,
        &env_remove,
        working_directory.as_ref(),
        "initialize_claude_runtime",
    )?;

    Ok(ClaudeRuntimeBinding {
        state: ClaudeRuntimeState {
            program,
            args,
            env,
            provider_credential_env: credentials.clone(),
            env_remove,
            working_directory,
            context_file,
            settings_file,
            usage_file,
            last_usage_file_contents: None,
            mcp_config_file,
            child,
            stdin,
            receiver,
            active_model: run.model().to_string(),
            active_variant: run.variant().map(str::to_string),
            active_execution_mode: run.execution_mode(),
            active_permission_level: run.permission_level(),
            session_id: Some(session_id),
            active_stream_message_id: None,
            active_turn_id: None,
            active_prompt_message: None,
            turn_watchdog: Default::default(),
            cancelled_turn_pending_settlement: false,
            next_turn_number: 1,
            result_number: 1,
            emitted_text_by_block: BTreeMap::new(),
            tool_transcript: Default::default(),
            completed_text_blocks: Default::default(),
            exit_reported: false,
        },
        selection: ClaudeRunSelection {
            model: Some(run.model().to_string()),
            variant: run.variant().map(str::to_string),
        },
    })
}

fn install_claude_mcp_config_argument(
    args: &mut Vec<String>,
    config_file: Option<&std::path::Path>,
) -> Result<(), DaemonError> {
    let config_index = args.iter().position(|arg| arg == "--mcp-config");
    match (config_index, config_file) {
        (Some(index), Some(config_file)) => {
            let value = args
                .get_mut(index + 1)
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "materialize claude mcp config argument",
                    message: "Claude launch has --mcp-config without a value".to_string(),
                })?;
            *value = config_file.display().to_string();
        }
        (None, Some(config_file)) => {
            args.extend([
                "--mcp-config".to_string(),
                config_file.display().to_string(),
                "--strict-mcp-config".to_string(),
            ]);
        }
        (Some(_), None) => {
            return Err(DaemonError::LocalTransport {
                operation: "materialize claude mcp config argument",
                message: "Claude launch has --mcp-config without a materialized config file"
                    .to_string(),
            });
        }
        _ => {}
    }
    if let Some(directory) = config_file.and_then(std::path::Path::parent) {
        expose_runtime_directory_in_managed_namespace(args, directory)?;
    }
    Ok(())
}

pub(crate) fn submit_claude_prompt(
    run: &RuntimeProviderRun,
    state: &mut ClaudeRuntimeState,
    envelope: &PromptEnvelope,
) -> Result<(), DaemonError> {
    if claude_runtime_selection_changed(run, state) || claude_runtime_child_exited(state) {
        restart_claude_runtime(run, state, "claude_restart_for_selection_change")?;
    }
    write_claude_hidden_context(run.id(), state, &envelope.hidden_system_context)?;
    let turn_id = format!("turn-{}", state.next_turn_number);
    state.next_turn_number += 1;
    let message = json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": claude_user_content(&envelope.visible_user_prompt, &envelope.attachments)
        }
    });
    write_json_line(&mut state.stdin, &message)?;
    state.active_turn_id = Some(turn_id);
    state.active_prompt_message = Some(message);
    state.turn_watchdog.begin(Instant::now());
    state.active_stream_message_id = None;
    state.emitted_text_by_block.clear();
    state.tool_transcript.clear();
    state.completed_text_blocks.clear();
    Ok(())
}

pub(crate) fn abort_claude_turn(
    run: &RuntimeProviderRun,
    state: &mut ClaudeRuntimeState,
) -> Result<(), DaemonError> {
    let message = json!({
        "type": "control_request",
        "request_id": format!("chariox-claude-interrupt-{}", run.id()),
        "request": { "subtype": "interrupt" }
    });
    let _ = write_json_line(&mut state.stdin, &message);
    clear_active_claude_turn(state);
    state.cancelled_turn_pending_settlement = true;
    restart_claude_runtime(run, state, "claude_restart_after_abort")
}

pub(crate) fn drain_claude_events(
    run: &RuntimeProviderRun,
    state: &mut ClaudeRuntimeState,
) -> Result<ProviderPromptSignalBatch, DaemonError> {
    let mut batch = ProviderPromptSignalBatch::default();
    for _ in 0..CLAUDE_EVENT_DRAIN_MAX_MESSAGES {
        match state.receiver.try_recv() {
            Ok(ClaudeRuntimeMessage::Stdout(value)) => {
                state.turn_watchdog.record_runtime_message(Instant::now());
                handle_claude_tool_uses(run.id(), state, &value, &mut batch)?;
                apply_claude_message(run.id(), state, value, &mut batch);
            }
            Ok(ClaudeRuntimeMessage::StdoutParseError(error)) => {
                state.turn_watchdog.record_runtime_message(Instant::now());
                batch
                    .notices
                    .push(format!("Claude stdout parse warning: {error}"));
            }
            Ok(ClaudeRuntimeMessage::Stderr(line)) => {
                apply_claude_stderr(run, state, &line, &mut batch);
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => break,
        }
    }
    apply_claude_usage_capture(state, &mut batch);

    if !batch.prompt_completed && !state.exit_reported {
        match state.child.try_wait() {
            Ok(Some(status)) => {
                state.exit_reported = true;
                if !status.success() || state.active_turn_id.is_some() {
                    batch.terminal_failure = Some(format!(
                        "Claude Code exited before completing the active turn: {status}"
                    ));
                    batch.prompt_completed = state.active_turn_id.is_some();
                    clear_active_claude_turn(state);
                }
            }
            Ok(None) => {}
            Err(error) => {
                state.exit_reported = true;
                batch.terminal_failure =
                    Some(format!("failed to poll Claude Code process: {error}"));
            }
        }
    }
    if !batch.prompt_completed && state.cancelled_turn_pending_settlement {
        state.cancelled_turn_pending_settlement = false;
        batch.prompt_completed = true;
    }
    if batch.prompt_completed {
        clear_active_claude_turn(state);
    } else {
        apply_claude_turn_stall_policy(run, state, &mut batch)?;
    }

    Ok(batch)
}

fn apply_claude_stderr(
    run: &RuntimeProviderRun,
    state: &mut ClaudeRuntimeState,
    line: &str,
    batch: &mut ProviderPromptSignalBatch,
) {
    if batch.terminal_failure.is_none() {
        if let Some(failure) =
            crate::provider::classify_provider_terminal_failure_text(run.adapter_key(), line)
        {
            batch.terminal_failure = Some(failure);
            batch.prompt_completed = state.active_turn_id.is_some();
            clear_active_claude_turn(state);
            return;
        }
    }
    batch.notices.push(format!("Claude stderr: {line}"));
}

fn apply_claude_turn_stall_policy(
    run: &RuntimeProviderRun,
    state: &mut ClaudeRuntimeState,
    batch: &mut ProviderPromptSignalBatch,
) -> Result<(), DaemonError> {
    match state
        .turn_watchdog
        .action(Instant::now(), claude_turn_stall_timeout())
    {
        ClaudeTurnStallAction::Wait => Ok(()),
        ClaudeTurnStallAction::Restart => retry_stalled_claude_turn(run, state, batch),
        ClaudeTurnStallAction::Fail => {
            stop_child(&mut state.child);
            clear_active_claude_turn(state);
            batch.terminal_failure = Some(
                "Claude Code stopped emitting runtime events; the active turn was ended after its bounded recovery attempt"
                    .to_string(),
            );
            batch.prompt_completed = true;
            Ok(())
        }
    }
}

fn retry_stalled_claude_turn(
    run: &RuntimeProviderRun,
    state: &mut ClaudeRuntimeState,
    batch: &mut ProviderPromptSignalBatch,
) -> Result<(), DaemonError> {
    let message =
        state
            .active_prompt_message
            .clone()
            .ok_or_else(|| DaemonError::ProviderProtocol {
                provider_run_id: run.id().to_string(),
                operation: "claude_stalled_turn_retry",
                message: "active Claude turn did not retain its prompt message".to_string(),
            })?;
    restart_claude_runtime(run, state, "claude_restart_after_unacknowledged_turn_stall")?;
    write_json_line(&mut state.stdin, &message)?;
    let turn_id = format!("turn-{}", state.next_turn_number);
    state.next_turn_number += 1;
    state.active_turn_id = Some(turn_id);
    state.active_prompt_message = Some(message);
    state.turn_watchdog.record_restart(Instant::now());
    batch.chunks.push(crate::provider::ProviderPromptChunk {
        kind: TerminalOutputKind::ProviderStatus,
        merge_key: Some(crate::provider::PROVIDER_CONNECTION_RETRY_MERGE_KEY.to_string()),
        bytes: crate::provider::provider_retry_status("Claude", Some("runtime unresponsive"))
            .into_bytes(),
    });
    Ok(())
}

fn clear_active_claude_turn(state: &mut ClaudeRuntimeState) {
    state.tool_transcript.clear();
    state.active_turn_id = None;
    state.active_prompt_message = None;
    state.turn_watchdog.settle();
}

fn claude_turn_stall_timeout() -> Duration {
    std::env::var("CHARIOX_CLAUDE_TURN_STALL_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_CLAUDE_TURN_STALL_TIMEOUT)
}

fn handle_claude_tool_uses(
    provider_run_id: &str,
    state: &mut ClaudeRuntimeState,
    value: &serde_json::Value,
    batch: &mut ProviderPromptSignalBatch,
) -> Result<(), DaemonError> {
    for payload in state.tool_transcript.observe(value) {
        let bytes =
            serde_json::to_vec(&payload).map_err(|error| DaemonError::ProviderProtocol {
                provider_run_id: provider_run_id.to_string(),
                operation: "claude_tool_transcript_serialize",
                message: error.to_string(),
            })?;
        batch.chunks.push(super::ProviderPromptChunk {
            kind: TerminalOutputKind::ProviderTool,
            merge_key: payload
                .get("id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
            bytes,
        });
    }
    if state.tool_transcript.take_truncation_notice() {
        batch.notices.push(
            "Claude tool transcript truncated to resource limits; provider execution is unchanged"
                .to_string(),
        );
    }
    let tool_results = state.tool_transcript.take_unsupported_results();
    if !tool_results.is_empty() {
        let response = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": tool_results,
            }
        });
        write_json_line(&mut state.stdin, &response)?;
        batch.notices.push(format!(
            "Rejected unsupported Claude stream-json tool use for `{provider_run_id}`"
        ));
    }
    Ok(())
}

fn claude_runtime_selection_changed(run: &RuntimeProviderRun, state: &ClaudeRuntimeState) -> bool {
    state.active_model != run.model()
        || state.active_variant.as_deref() != run.variant()
        || state.active_execution_mode != run.execution_mode()
        || state.active_permission_level != run.permission_level()
}

fn claude_runtime_child_exited(state: &mut ClaudeRuntimeState) -> bool {
    if state.active_turn_id.is_some() {
        return false;
    }
    match state.child.try_wait() {
        Ok(Some(_)) => true,
        Ok(None) => false,
        Err(_) => true,
    }
}

fn restart_claude_runtime(
    run: &RuntimeProviderRun,
    state: &mut ClaudeRuntimeState,
    operation: &'static str,
) -> Result<(), DaemonError> {
    stop_child(&mut state.child);
    let resume_session_id = state
        .session_id
        .as_deref()
        .or_else(|| run.resume_state().claude_session_id());
    let base_args = claude_args_without_resume(&state.args);
    let mut args = base_args.clone();
    if let Some(session_id) = resume_session_id {
        args.extend(["--resume".to_string(), session_id.to_string()]);
    }
    let (child, stdin, receiver) = spawn_claude_child(
        run.id(),
        &state.program,
        &args,
        &state.env,
        &state.provider_credential_env,
        &state.env_remove,
        state.working_directory.as_ref(),
        operation,
    )?;
    state.child = child;
    state.stdin = stdin;
    state.receiver = receiver;
    state.args = base_args;
    state.active_model = run.model().to_string();
    state.active_variant = run.variant().map(str::to_string);
    state.active_execution_mode = run.execution_mode();
    state.active_permission_level = run.permission_level();
    state.active_stream_message_id = None;
    state.active_turn_id = None;
    state.active_prompt_message = None;
    state.turn_watchdog.settle();
    state.emitted_text_by_block.clear();
    state.completed_text_blocks.clear();
    state.exit_reported = false;
    state.tool_transcript.clear();
    Ok(())
}

fn claude_args_without_resume(args: &[String]) -> Vec<String> {
    let mut sanitized = Vec::with_capacity(args.len());
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if matches!(arg.as_str(), "--resume" | "--session-id") {
            skip_next = true;
            continue;
        }
        sanitized.push(arg.clone());
    }
    sanitized
}

pub(crate) fn new_claude_session_id() -> String {
    let mut bytes = [0_u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    )
}

fn write_claude_hidden_context(
    provider_run_id: &str,
    state: &ClaudeRuntimeState,
    hidden_system_context: &str,
) -> Result<(), DaemonError> {
    let Some(path) = &state.context_file else {
        return Ok(());
    };
    crate::provider::ensure_claude_native_hidden_context_fits(
        provider_run_id,
        hidden_system_context.trim(),
    )?;
    std::fs::write(path, hidden_system_context.trim()).map_err(|error| {
        DaemonError::ProviderProtocol {
            provider_run_id: provider_run_id.to_string(),
            operation: "claude_hidden_context_write",
            message: error.to_string(),
        }
    })
}

#[cfg(test)]
mod tests {
    mod tool_results;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use base64::Engine as _;
    use serde_json::json;

    use crate::provider::claude::CLAUDE_MCP_CONFIG_PLACEHOLDER;
    use crate::provider::{
        AgentEndpointMode, AgentExecutionMode, AgentPermissionLevel, LaunchProviderRequest,
        ProviderLaunchResult, ProviderResumeState, RuntimeMcpBinding, RuntimeProviderRun,
    };
    use crate::session::PromptAttachment;
    use crate::terminal::TerminalOutputKind;

    use super::{
        apply_claude_stderr, claude_args_without_resume, events::apply_claude_message,
        handle_claude_tool_uses, initialize_claude_runtime, input::claude_user_content,
        new_claude_session_id, restart_claude_runtime, ClaudeRuntimeState,
        ProviderPromptSignalBatch,
    };

    fn parser_state() -> (ClaudeRuntimeState, ProviderPromptSignalBatch) {
        let mut child = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg("cat >/dev/null")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("fixture child should spawn");
        let stdin = child.stdin.take().expect("fixture stdin should exist");
        let (_tx, receiver) = std::sync::mpsc::channel();
        (
            ClaudeRuntimeState {
                program: "/bin/sh".to_string(),
                args: vec!["-c".to_string(), "cat >/dev/null".to_string()],
                env: Default::default(),
                provider_credential_env: Default::default(),
                env_remove: Vec::new(),
                working_directory: None,
                context_file: None,
                settings_file: None,
                usage_file: None,
                last_usage_file_contents: None,
                mcp_config_file: None,
                child,
                stdin,
                receiver,
                active_model: "sonnet".to_string(),
                active_variant: Some("low".to_string()),
                active_execution_mode: AgentExecutionMode::Build,
                active_permission_level: AgentPermissionLevel::Yolo,
                session_id: None,
                active_stream_message_id: None,
                active_turn_id: Some("turn-1".to_string()),
                active_prompt_message: None,
                turn_watchdog: Default::default(),
                cancelled_turn_pending_settlement: false,
                next_turn_number: 1,
                result_number: 1,
                emitted_text_by_block: Default::default(),
                tool_transcript: Default::default(),
                completed_text_blocks: Default::default(),
                exit_reported: false,
            },
            ProviderPromptSignalBatch::default(),
        )
    }

    #[test]
    fn claude_usage_limit_on_stderr_ends_active_turn_with_authoritative_failure() {
        let (mut state, mut batch) = parser_state();
        let run = RuntimeProviderRun::new(
            "run-1",
            &LaunchProviderRequest::new("session-1", "claude", "claude", "default", "sonnet"),
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: "test-claude".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: Default::default(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: Some("test-claude-runtime".to_string()),
            },
        );

        apply_claude_stderr(
            &run,
            &mut state,
            "You've hit your usage limit. Your limit will reset later.",
            &mut batch,
        );

        assert!(batch.prompt_completed);
        assert_eq!(state.active_turn_id, None);
        assert!(batch.notices.is_empty());
        assert_eq!(
            batch.terminal_failure.as_deref(),
            Some(
                "Provider reported a substitutable resource limit: You've hit your usage limit. Your limit will reset later."
            )
        );
    }

    #[test]
    fn claude_args_without_resume_removes_stale_session_argument() {
        let args = vec![
            "--model".to_string(),
            "sonnet".to_string(),
            "--resume".to_string(),
            "stale-session".to_string(),
            "--session-id".to_string(),
            "new-session".to_string(),
            "--mcp-config".to_string(),
            "/tmp/mcp.json".to_string(),
        ];

        assert_eq!(
            claude_args_without_resume(&args),
            vec![
                "--model".to_string(),
                "sonnet".to_string(),
                "--mcp-config".to_string(),
                "/tmp/mcp.json".to_string(),
            ]
        );
    }

    #[test]
    fn runtime_mcp_config_uses_private_file_not_argv_and_cleans_up_after_restart() {
        let request =
            LaunchProviderRequest::new("session-1", "claude", "claude", "default", "sonnet")
                .with_runtime_mcp_binding(RuntimeMcpBinding::new(
                    "http://127.0.0.1:43120/mcp",
                    "runtime-bearer-token",
                ));
        let run = RuntimeProviderRun::new(
            "provider-run-1",
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::External,
                process_label: "claude:stream-json".to_string(),
                pty_target: None,
                pty_program: Some("/bin/sh".to_string()),
                pty_args: vec![
                    "-c".to_string(),
                    "cat >/dev/null".to_string(),
                    "--mcp-config".to_string(),
                    CLAUDE_MCP_CONFIG_PLACEHOLDER.to_string(),
                ],
                pty_env: Default::default(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        );

        let mut binding = initialize_claude_runtime(&run)
            .expect("Claude runtime should materialize and launch its MCP config");
        let config_path = binding
            .state
            .args
            .windows(2)
            .find_map(|pair| {
                (pair[0] == "--mcp-config").then(|| std::path::PathBuf::from(&pair[1]))
            })
            .expect("runtime should pass an MCP config path");
        let config_root = config_path
            .parent()
            .expect("config should have a root")
            .to_path_buf();

        assert!(config_path.is_file());
        assert!(binding
            .state
            .args
            .iter()
            .all(|arg| !arg.contains("runtime-bearer-token") && !arg.contains("mcpServers")));
        #[cfg(unix)]
        assert_eq!(
            std::fs::metadata(&config_path)
                .expect("config metadata should exist")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert!(std::fs::read_to_string(&config_path)
            .expect("config should be readable by the kernel")
            .contains("Bearer runtime-bearer-token"));

        restart_claude_runtime(&run, &mut binding.state, "test_claude_restart")
            .expect("Claude runtime should restart");
        assert!(
            config_path.is_file(),
            "restart must retain the active config file"
        );

        drop(binding);
        assert!(
            !config_root.exists(),
            "dropping the runtime must remove its private MCP config root"
        );
    }

    #[test]
    fn runtime_mcp_config_cleans_up_when_child_spawn_fails() {
        let token = format!(
            "runtime-bearer-spawn-failure-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        );
        let missing_program = std::env::temp_dir().join(format!(
            "chariox-missing-claude-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let request =
            LaunchProviderRequest::new("session-1", "claude", "claude", "default", "sonnet")
                .with_runtime_mcp_binding(RuntimeMcpBinding::new(
                    "http://127.0.0.1:43120/mcp",
                    &token,
                ));
        let run = RuntimeProviderRun::new(
            "provider-run-spawn-failure",
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::External,
                process_label: "claude:stream-json".to_string(),
                pty_target: None,
                pty_program: Some(missing_program.display().to_string()),
                pty_args: vec![
                    "--mcp-config".to_string(),
                    CLAUDE_MCP_CONFIG_PLACEHOLDER.to_string(),
                ],
                pty_env: Default::default(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        );

        assert!(
            initialize_claude_runtime(&run).is_err(),
            "missing Claude executable should fail"
        );

        let leaked_config = std::fs::read_dir(std::env::temp_dir())
            .expect("kernel temp directory should be readable")
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("mcp-config.json"))
            .filter_map(|path| std::fs::read_to_string(path).ok())
            .any(|config| config.contains(&token));
        assert!(
            !leaked_config,
            "failed Claude startup must remove its private MCP config"
        );
    }

    #[test]
    fn rejects_unmaterialized_mcp_config_argument() {
        let mut args = vec![
            "--mcp-config".to_string(),
            "{\"mcpServers\":{\"chariox\":{\"token\":\"inline-secret\"}}}".to_string(),
        ];

        let error = super::install_claude_mcp_config_argument(&mut args, None)
            .expect_err("inline config must never reach Claude argv");

        assert!(error.to_string().contains("materialized config file"));
    }

    #[test]
    fn managed_runtime_mcp_config_is_visible_inside_the_private_tmp_namespace() {
        let root = std::env::temp_dir().join(format!(
            "chariox-claude-runtime-binding-test-{}",
            std::process::id()
        ));
        let config = root.join("mcp-config.json");
        let mut args = vec![
            "--tmpfs".to_string(),
            "/tmp".to_string(),
            "--setenv".to_string(),
            crate::provider::managed_isolation::MANAGED_PROVIDER_ISOLATION_MARKER_ENV.to_string(),
            "1".to_string(),
            "--".to_string(),
            "/usr/local/bin/claude".to_string(),
            "--mcp-config".to_string(),
            CLAUDE_MCP_CONFIG_PLACEHOLDER.to_string(),
        ];

        super::install_claude_mcp_config_argument(&mut args, Some(&config))
            .expect("managed MCP config should be installed");

        let separator = args
            .iter()
            .position(|arg| arg == "--")
            .expect("managed launch should retain its separator");
        assert_eq!(
            &args[separator - 5..separator],
            [
                "--dir",
                root.to_str().expect("test root should be UTF-8"),
                "--ro-bind",
                root.to_str().expect("test root should be UTF-8"),
                root.to_str().expect("test root should be UTF-8"),
            ]
        );
        assert!(args[separator + 1..].windows(2).any(|window| {
            window
                == [
                    "--mcp-config",
                    config.to_str().expect("config should be UTF-8"),
                ]
        }));
    }

    #[test]
    fn generated_claude_session_ids_are_uuid_v4_shape() {
        let session_id = new_claude_session_id();
        assert_eq!(session_id.len(), 36);
        assert_eq!(&session_id[14..15], "4");
        assert!(matches!(&session_id[19..20], "8" | "9" | "a" | "b"));
        assert_eq!(
            session_id
                .chars()
                .enumerate()
                .filter(|(_, character)| *character == '-')
                .map(|(index, _)| index)
                .collect::<Vec<_>>(),
            vec![8, 13, 18, 23]
        );
    }

    #[test]
    fn captures_system_session_and_model() {
        let (mut state, mut batch) = parser_state();

        apply_claude_message(
            "run-1",
            &mut state,
            json!({
                "type": "system",
                "subtype": "init",
                "session_id": "claude-session-1",
                "model": "claude-sonnet-4-6"
            }),
            &mut batch,
        );

        assert_eq!(state.session_id.as_deref(), Some("claude-session-1"));
        assert!(
            batch.resolved_resume_state.is_none(),
            "provider initialization alone must not make a fresh Claude session resumable"
        );
        assert_eq!(
            batch.resolved_model.as_deref(),
            Some("claude/claude-sonnet-4-6")
        );
        assert_eq!(batch.resolved_model_source, Some("claude.system"));
    }

    #[test]
    fn captures_resumable_session_only_after_prompt_submission() {
        let (mut state, mut batch) = parser_state();
        state.active_prompt_message = Some(json!({
            "type": "user",
            "message": { "role": "user", "content": "hello" }
        }));

        apply_claude_message(
            "run-1",
            &mut state,
            json!({
                "type": "system",
                "subtype": "init",
                "session_id": "claude-session-1",
                "model": "claude-haiku-4-5"
            }),
            &mut batch,
        );

        assert_eq!(
            batch
                .resolved_resume_state
                .as_ref()
                .and_then(ProviderResumeState::claude_session_id),
            Some("claude-session-1")
        );
    }

    #[test]
    fn parses_stream_text_delta() {
        let (mut state, mut batch) = parser_state();

        apply_claude_message(
            "run-1",
            &mut state,
            json!({
                "type": "stream_event",
                "event": {
                    "type": "content_block_delta",
                    "delta": { "type": "text_delta", "text": "hello" }
                }
            }),
            &mut batch,
        );

        assert_eq!(batch.chunks.len(), 1);
        assert_eq!(batch.chunks[0].kind, TerminalOutputKind::ProviderOutput);
        assert_eq!(batch.chunks[0].bytes, b"hello");
    }

    #[test]
    fn reconciles_partial_stream_with_authoritative_assistant_snapshot() {
        let (mut state, mut streamed) = parser_state();

        apply_claude_message(
            "run-1",
            &mut state,
            json!({
                "type": "stream_event",
                "event": {
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": { "type": "text_delta", "text": "CL" }
                }
            }),
            &mut streamed,
        );
        assert_eq!(streamed.chunks[0].bytes, b"CL");

        let mut completed = ProviderPromptSignalBatch::default();
        let assistant = json!({
            "type": "assistant",
            "message": {
                "id": "msg-1",
                "content": [{ "type": "text", "text": "CLAUDE_MANAGED_EMPTY_OK" }]
            }
        });
        apply_claude_message("run-1", &mut state, assistant.clone(), &mut completed);

        assert_eq!(completed.chunks.len(), 1);
        assert_eq!(completed.chunks[0].kind, TerminalOutputKind::ProviderOutput);
        assert_eq!(completed.chunks[0].bytes, b"AUDE_MANAGED_EMPTY_OK");

        let mut duplicate = ProviderPromptSignalBatch::default();
        apply_claude_message("run-1", &mut state, assistant, &mut duplicate);
        assert!(duplicate.chunks.is_empty());
    }

    #[test]
    fn ignores_stream_replay_after_authoritative_assistant_snapshot() {
        let (mut state, mut completed) = parser_state();

        apply_claude_message(
            "run-1",
            &mut state,
            json!({
                "type": "assistant",
                "message": {
                    "id": "msg-1",
                    "content": [{ "type": "text", "text": "CLAUDE_MANAGED_EMPTY_OK" }]
                }
            }),
            &mut completed,
        );
        assert_eq!(completed.chunks.len(), 1);
        assert_eq!(completed.chunks[0].bytes, b"CLAUDE_MANAGED_EMPTY_OK");

        let mut replay = ProviderPromptSignalBatch::default();
        apply_claude_message(
            "run-1",
            &mut state,
            json!({
                "type": "stream_event",
                "event": {
                    "type": "message_start",
                    "message": { "id": "msg-1" }
                }
            }),
            &mut replay,
        );
        apply_claude_message(
            "run-1",
            &mut state,
            json!({
                "type": "stream_event",
                "event": {
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": { "type": "text_delta", "text": "CLAUDE_MANAGED_EMPTY_OK" }
                }
            }),
            &mut replay,
        );

        assert!(replay.chunks.is_empty());
    }

    #[test]
    fn reused_block_index_in_later_assistant_message_remains_streamable() {
        let (mut state, mut first) = parser_state();

        apply_claude_message(
            "run-1",
            &mut state,
            json!({
                "type": "assistant",
                "message": {
                    "id": "msg-1",
                    "content": [{ "type": "text", "text": "first message" }]
                }
            }),
            &mut first,
        );
        assert_eq!(first.chunks[0].bytes, b"first message");

        let mut second_stream = ProviderPromptSignalBatch::default();
        apply_claude_message(
            "run-1",
            &mut state,
            json!({
                "type": "stream_event",
                "event": {
                    "type": "message_start",
                    "message": { "id": "msg-2" }
                }
            }),
            &mut second_stream,
        );
        apply_claude_message(
            "run-1",
            &mut state,
            json!({
                "type": "stream_event",
                "event": {
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": { "type": "text_delta", "text": "second" }
                }
            }),
            &mut second_stream,
        );
        assert_eq!(second_stream.chunks[0].bytes, b"second");

        let mut second_completed = ProviderPromptSignalBatch::default();
        apply_claude_message(
            "run-1",
            &mut state,
            json!({
                "type": "assistant",
                "message": {
                    "id": "msg-2",
                    "content": [{ "type": "text", "text": "second message" }]
                }
            }),
            &mut second_completed,
        );
        assert_eq!(second_completed.chunks[0].bytes, b" message");
    }

    #[test]
    fn marks_result_completion_and_usage() {
        let (mut state, mut batch) = parser_state();

        apply_claude_message(
            "run-1",
            &mut state,
            json!({
                "type": "result",
                "subtype": "success",
                "is_error": false,
                "session_id": "claude-session-1",
                "usage": {
                    "input_tokens": 3,
                    "output_tokens": 5,
                    "cache_creation_input_tokens": 2
                }
            }),
            &mut batch,
        );

        assert!(batch.prompt_completed);
        assert_eq!(batch.completions.len(), 1);
        assert_eq!(batch.resolved_usage_tokens_total, Some(10));
        assert_eq!(state.active_turn_id, None);
    }

    #[test]
    fn normalizes_claude_rate_limit_event_for_selected_account() {
        let (mut state, mut batch) = parser_state();

        apply_claude_message(
            "run-1",
            &mut state,
            json!({
                "type": "rate_limit_event",
                "rate_limit_info": {
                    "rate_limit_type": "five_hour",
                    "utilization": 0.84,
                    "status": "allowed",
                    "resets_at": 1_800_000_000
                }
            }),
            &mut batch,
        );

        let usage = batch.account_usage.expect("rate limit usage");
        assert_eq!(usage.provider, "claude");
        assert_eq!(usage.meters.len(), 1);
        assert_eq!(usage.meters[0].used_percent, Some(84.0));
        assert_eq!(usage.meters[0].window_duration_minutes, Some(300));
        assert_eq!(usage.meters[0].resets_at_ms, Some(1_800_000_000_000));
    }

    #[test]
    fn normalizes_claude_rate_limit_event_string_reset_timestamps() {
        let (mut state, mut batch) = parser_state();

        apply_claude_message(
            "run-1",
            &mut state,
            json!({
                "type": "rate_limit_event",
                "rate_limit_info": {
                    "rateLimitType": "seven_day",
                    "utilization": 41.0,
                    "status": "allowed",
                    "resetsAt": "2027-01-15T12:00:00Z"
                }
            }),
            &mut batch,
        );

        let usage = batch.account_usage.expect("rate limit usage");
        assert_eq!(usage.meters.len(), 1);
        assert_eq!(usage.meters[0].meter_id, "rate_limit/seven_day");
        assert_eq!(usage.meters[0].resets_at_ms, Some(1_800_014_400_000));
    }

    #[test]
    fn rejects_unsupported_claude_tool_use_without_completing_turn() {
        let (mut state, mut batch) = parser_state();

        handle_claude_tool_uses(
            "run-1",
            &mut state,
            &json!({
                "type": "assistant",
                "message": {
                    "content": [{
                        "type": "tool_use",
                        "id": "toolu_1",
                        "name": "ToolSearch",
                        "input": { "query": "chariox workflow tools" }
                    }]
                }
            }),
            &mut batch,
        )
        .expect("tool-use rejection should write a tool result");

        assert!(!batch.prompt_completed);
        assert!(batch
            .notices
            .iter()
            .any(|notice| notice.contains("Rejected unsupported Claude stream-json tool use")));
    }

    #[test]
    fn records_provider_native_claude_tool_use_without_rejecting_it() {
        let (mut state, mut batch) = parser_state();

        handle_claude_tool_uses(
            "run-1",
            &mut state,
            &json!({
                "type": "assistant",
                "message": {
                    "content": [{
                        "type": "tool_use",
                        "id": "toolu_browser",
                        "name": "browser_snapshot",
                        "input": { "random": "value" }
                    }]
                }
            }),
            &mut batch,
        )
        .expect("provider-native tool use should be recorded");

        assert!(batch.notices.is_empty());
        assert_eq!(batch.chunks.len(), 1);
        assert_eq!(batch.chunks[0].kind, TerminalOutputKind::ProviderTool);
        let payload: serde_json::Value =
            serde_json::from_slice(&batch.chunks[0].bytes).expect("tool payload should be JSON");
        assert_eq!(payload["tool"], "browser_snapshot");
        assert_eq!(payload["status"], "running");
        assert_eq!(payload["input"]["random"], "value");
    }

    #[test]
    fn user_content_includes_text_attachment_contents() {
        let attachment = PromptAttachment::new(
            "artifact://note",
            "text/plain",
            Some("note.txt".to_string()),
        )
        .with_contents_base64(base64::engine::general_purpose::STANDARD.encode("attached marker"));

        let content = claude_user_content("read this", &[attachment]);

        assert_eq!(content[0]["text"], "read this");
        assert!(content[1]["text"]
            .as_str()
            .expect("attachment should render as text")
            .contains("attached marker"));
    }

    #[test]
    fn user_content_falls_back_to_attachment_reference_for_opaque_data() {
        let attachment = PromptAttachment::new(
            "artifact://archive",
            "application/octet-stream",
            Some("archive.bin".to_string()),
        )
        .with_contents_base64(base64::engine::general_purpose::STANDARD.encode([0, 1, 2]));

        let content = claude_user_content("", &[attachment]);

        assert_eq!(content.len(), 1);
        assert!(content[0]["text"]
            .as_str()
            .expect("opaque attachment should render as reference")
            .contains("archive.bin"));
    }
}
