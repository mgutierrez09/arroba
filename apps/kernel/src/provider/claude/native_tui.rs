//! Claude native TUI hook files and launch arguments.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::DaemonError;
use crate::provider::{AgentExecutionMode, AgentPermissionLevel, LaunchProviderRequest};

use super::launch_args::{normalized_claude_model, request_uses_metaagent_tools_only};
use super::mcp_config::{
    create_claude_runtime_files_root, materialize_request_claude_mcp_config, ClaudeRuntimeFilesRoot,
};
use super::usage_capture::materialize_claude_usage_capture;

pub(crate) const CLAUDE_NATIVE_CONTEXT_HOOK_CHUNKS: usize = 8;
pub(crate) const CLAUDE_NATIVE_CONTEXT_CHUNK_BYTES: usize = 6_000;
pub(crate) const CLAUDE_NATIVE_MAX_HIDDEN_CONTEXT_BYTES: usize =
    CLAUDE_NATIVE_CONTEXT_HOOK_CHUNKS * CLAUDE_NATIVE_CONTEXT_CHUNK_BYTES;

pub(crate) fn ensure_claude_native_hidden_context_fits(
    provider_run_id: &str,
    hidden_context: &str,
) -> Result<(), DaemonError> {
    let mut chunk_count = usize::from(!hidden_context.is_empty());
    let mut chunk_bytes = 0usize;
    for scalar in hidden_context.chars() {
        let scalar_bytes = scalar.len_utf8();
        if chunk_bytes > 0 && chunk_bytes + scalar_bytes > CLAUDE_NATIVE_CONTEXT_CHUNK_BYTES {
            chunk_count += 1;
            chunk_bytes = 0;
        }
        chunk_bytes += scalar_bytes;
    }
    if chunk_count <= CLAUDE_NATIVE_CONTEXT_HOOK_CHUNKS {
        return Ok(());
    }
    Err(DaemonError::ProviderProtocol {
        provider_run_id: provider_run_id.to_string(),
        operation: "claude_hidden_context_size",
        message: format!(
            "hidden context is {} bytes and requires {} UTF-8-safe chunks; Claude native delivery supports {} chunks of at most {} bytes ({} bytes theoretical maximum)",
            hidden_context.len(),
            chunk_count,
            CLAUDE_NATIVE_CONTEXT_HOOK_CHUNKS,
            CLAUDE_NATIVE_CONTEXT_CHUNK_BYTES,
            CLAUDE_NATIVE_MAX_HIDDEN_CONTEXT_BYTES
        ),
    })
}

pub(super) struct ClaudeNativeTuiFiles {
    root: ClaudeRuntimeFilesRoot,
    pub(super) events_file: PathBuf,
    pub(super) context_file: PathBuf,
    pub(super) context_response_dir: PathBuf,
    pub(super) permission_response_dir: PathBuf,
    pub(super) settings_file: PathBuf,
    pub(super) usage_file: PathBuf,
    mcp_config_file: Option<PathBuf>,
}

impl ClaudeNativeTuiFiles {
    pub(super) fn materialize_mcp_config(
        &mut self,
        request: &LaunchProviderRequest,
    ) -> Result<(), DaemonError> {
        self.mcp_config_file = materialize_request_claude_mcp_config(request, &self.root)?;
        Ok(())
    }

    pub(super) fn mcp_config_file(&self) -> Option<&Path> {
        self.mcp_config_file.as_deref()
    }

    pub(super) fn persist_for_launch(&mut self) {
        self.root.persist_for_launch();
    }
}

pub(super) fn prepare_claude_native_tui_files(
    request: &LaunchProviderRequest,
) -> Result<ClaudeNativeTuiFiles, DaemonError> {
    let root = create_claude_runtime_files_root()?;
    let events_file = root.path().join("events.jsonl");
    let context_file = root.path().join("hidden-context.txt");
    let context_response_dir = root.path().join("hook-context-responses");
    let permission_response_dir = root.path().join("permission-responses");
    let settings_file = root.path().join("settings.json");
    let hook_handler_file = root.path().join("hook-handler.mjs");
    let usage_capture = materialize_claude_usage_capture(&root)?;
    let usage_file = usage_capture.usage_file().to_path_buf();
    fs::create_dir_all(&context_response_dir).map_err(|error| DaemonError::LocalTransport {
        operation: "prepare claude native context response dir",
        message: error.to_string(),
    })?;
    fs::create_dir_all(&permission_response_dir).map_err(|error| DaemonError::LocalTransport {
        operation: "prepare claude native permission response dir",
        message: error.to_string(),
    })?;
    fs::write(&events_file, "").map_err(|error| DaemonError::LocalTransport {
        operation: "prepare claude native events file",
        message: error.to_string(),
    })?;
    fs::write(&context_file, "").map_err(|error| DaemonError::LocalTransport {
        operation: "prepare claude native context file",
        message: error.to_string(),
    })?;
    fs::write(&hook_handler_file, claude_native_hook_handler()).map_err(|error| {
        DaemonError::LocalTransport {
            operation: "prepare claude native hook handler",
            message: error.to_string(),
        }
    })?;
    let hook_command = claude_native_hook_command(
        &hook_handler_file,
        &events_file,
        &context_file,
        &context_response_dir,
        &permission_response_dir,
        None,
    );
    let prompt_hooks = (0..CLAUDE_NATIVE_CONTEXT_HOOK_CHUNKS)
        .map(|chunk_index| {
            serde_json::json!({
                "type": "command",
                "command": claude_native_hook_command(
                    &hook_handler_file,
                    &events_file,
                    &context_file,
                    &context_response_dir,
                    &permission_response_dir,
                    Some(chunk_index),
                )
            })
        })
        .collect::<Vec<_>>();
    let settings = serde_json::json!({
        "skipDangerousModePermissionPrompt": request.permission_level.unwrap_or_default()
            == AgentPermissionLevel::Yolo,
        "hooks": {
            "SessionStart": [{ "hooks": [{ "type": "command", "command": hook_command }] }],
            "UserPromptSubmit": [{ "hooks": prompt_hooks }],
            "Stop": [{ "hooks": [{ "type": "command", "command": hook_command }] }],
            "StopFailure": [{ "hooks": [{ "type": "command", "command": hook_command }] }],
            "SessionEnd": [{ "hooks": [{ "type": "command", "command": hook_command }] }],
            "PermissionRequest": [{ "matcher": "*", "hooks": [{ "type": "command", "command": hook_command }] }]
        },
        "statusLine": {
            "type": "command",
            "command": usage_capture.command()
        }
    });
    let settings =
        serde_json::to_string_pretty(&settings).map_err(|error| DaemonError::LocalTransport {
            operation: "prepare claude native settings",
            message: error.to_string(),
        })?;
    fs::write(&settings_file, settings).map_err(|error| DaemonError::LocalTransport {
        operation: "prepare claude native settings file",
        message: error.to_string(),
    })?;
    Ok(ClaudeNativeTuiFiles {
        root,
        events_file,
        context_file,
        context_response_dir,
        permission_response_dir,
        settings_file,
        usage_file,
        mcp_config_file: None,
    })
}

fn claude_native_hook_command(
    hook_handler_file: &Path,
    events_file: &Path,
    context_file: &Path,
    context_response_dir: &Path,
    permission_response_dir: &Path,
    context_chunk_index: Option<usize>,
) -> String {
    let quoted = |path: &Path| {
        serde_json::to_string(&path.display().to_string())
            .expect("serializing a filesystem path should not fail")
    };
    let context_chunk = context_chunk_index
        .map(|index| format!(" CHARIOX_CLAUDE_NATIVE_CONTEXT_CHUNK={index}"))
        .unwrap_or_default();
    format!(
        "CHARIOX_CLAUDE_NATIVE_EVENTS={} CHARIOX_CLAUDE_NATIVE_CONTEXT={} CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES={} CHARIOX_CLAUDE_NATIVE_PERMISSION_RESPONSES={}{} node {}",
        quoted(events_file),
        quoted(context_file),
        quoted(context_response_dir),
        quoted(permission_response_dir),
        context_chunk,
        quoted(hook_handler_file),
    )
}

fn claude_native_hook_handler() -> &'static str {
    r#"#!/usr/bin/env node
import { appendFileSync, existsSync, readFileSync, unlinkSync, writeFileSync } from "node:fs"
import { dirname, join } from "node:path"
import { setTimeout as setCallbackTimeout } from "node:timers"
import { setTimeout as sleep } from "node:timers/promises"

const hookWatchdog = setCallbackTimeout(() => {
  try {
    appendFileSync(`${process.env.CHARIOX_CLAUDE_NATIVE_EVENTS}.watchdog`, JSON.stringify({
      at: new Date().toISOString(),
      reason: "hook_event_not_resolved"
    }) + "\n")
  } catch {}
  process.exit(0)
}, 7000)

async function readHookInput() {
  const chunks = []
  let settled = false
  return await new Promise((resolve) => {
    const finish = () => {
      if (settled) return
      settled = true
      resolve(Buffer.concat(chunks).toString("utf8"))
    }
    process.stdin.on("data", (chunk) => chunks.push(chunk))
    process.stdin.once("end", finish)
    process.stdin.once("error", finish)
    process.stdin.resume()
    setCallbackTimeout(finish, 1000)
  })
}

const raw = await readHookInput()
let input = {}
try {
  input = raw.trim() ? JSON.parse(raw) : {}
} catch (error) {
  input = { hook_event_name: "parse_error", raw, error: String(error) }
}
const eventName = input.hook_event_name ?? "unknown"
const parsedContextChunkIndex = Number.parseInt(process.env.CHARIOX_CLAUDE_NATIVE_CONTEXT_CHUNK ?? "", 10)
const contextChunkIndex = Number.isInteger(parsedContextChunkIndex) && parsedContextChunkIndex >= 0
  ? parsedContextChunkIndex
  : null
const contextOnlyHook = contextChunkIndex !== null && contextChunkIndex > 0
if (!contextOnlyHook && eventName === "SessionStart") {
  try { unlinkSync(join(dirname(process.argv[1]), "mcp-config.json")) } catch {}
}
const hookContextRequestId = eventName === "UserPromptSubmit" || eventName === "PreToolUse" || eventName === "PermissionRequest"
  ? `${Date.now()}-${process.pid}-${Math.random().toString(36).slice(2)}`
  : null
if (!contextOnlyHook) {
  appendFileSync(process.env.CHARIOX_CLAUDE_NATIVE_EVENTS, JSON.stringify({
    at: new Date().toISOString(),
    hook_event_name: eventName,
    hook_context_request_id: hookContextRequestId,
    prompt: input.prompt ?? null,
    transcript_path: input.transcript_path ?? null,
    permission_mode: input.permission_mode ?? null,
    tool_name: input.tool_name ?? null,
    tool_input: input.tool_input ?? null,
    tool_response: input.tool_response ?? null,
    error: input.error ?? null,
    error_details: eventName === "StopFailure" ? input.error_details ?? null : null,
    last_assistant_message: eventName === "StopFailure" ? input.last_assistant_message ?? null : null,
  }) + "\n")
}

function utf8ContextChunk(value, chunkIndex, maximumBytes = 6000) {
  const chunks = []
  let chunk = ""
  let chunkBytes = 0
  for (const scalar of value) {
    const scalarBytes = Buffer.byteLength(scalar, "utf8")
    if (chunk && chunkBytes + scalarBytes > maximumBytes) {
      chunks.push(chunk)
      chunk = ""
      chunkBytes = 0
    }
    chunk += scalar
    chunkBytes += scalarBytes
  }
  if (chunk) chunks.push(chunk)
  return chunks[chunkIndex] ?? ""
}

if (eventName === "UserPromptSubmit") {
  let additionalContext = ""
  try {
    additionalContext = readFileSync(process.env.CHARIOX_CLAUDE_NATIVE_CONTEXT, "utf8")
  } catch {}
  if (!additionalContext && contextChunkIndex === 0 && hookContextRequestId && process.env.CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES) {
    const responseFile = join(process.env.CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES, `${hookContextRequestId}.txt`)
    const deadline = Date.now() + 5000
    while (Date.now() < deadline) {
      if (existsSync(responseFile)) {
        additionalContext = readFileSync(responseFile, "utf8")
        writeFileSync(process.env.CHARIOX_CLAUDE_NATIVE_CONTEXT, additionalContext)
        try { unlinkSync(responseFile) } catch {}
        break
      }
      await sleep(50)
    }
  }
  if (!additionalContext && contextOnlyHook) {
    const deadline = Date.now() + 5000
    while (Date.now() < deadline) {
      try {
        additionalContext = readFileSync(process.env.CHARIOX_CLAUDE_NATIVE_CONTEXT, "utf8")
      } catch {}
      if (additionalContext) break
      await sleep(50)
    }
  }
  additionalContext = utf8ContextChunk(additionalContext, contextChunkIndex ?? 0)
  process.stdout.write(JSON.stringify({
    hookSpecificOutput: {
      hookEventName: "UserPromptSubmit",
      additionalContext
    }
  }))
  process.exit(0)
} else if (eventName === "PreToolUse" || eventName === "PermissionRequest") {
  if (input.permission_mode === "bypassPermissions") {
    if (eventName === "PermissionRequest") {
      // PermissionRequestHookSpecificOutput nests the PermissionResult under
      // `decision`. PreToolUse uses a separate event-specific output shape.
      process.stdout.write(JSON.stringify({
        hookSpecificOutput: {
          hookEventName: "PermissionRequest",
          decision: {
            behavior: "allow"
          }
        }
      }))
    }
    process.exit(0)
  }
  if (!toolName) {
    process.exit(0)
  }
  clearTimeout(hookWatchdog)
  const responseDir = process.env.CHARIOX_CLAUDE_NATIVE_PERMISSION_RESPONSES
  const responseFile = responseDir && hookContextRequestId
    ? join(responseDir, `${hookContextRequestId}.json`)
    : null
  if (responseFile) {
    const deadline = Date.now() + 300000
    while (Date.now() < deadline) {
      if (existsSync(responseFile)) {
        try {
          const decision = JSON.parse(readFileSync(responseFile, "utf8"))
          try { unlinkSync(responseFile) } catch {}
          if (decision?.behavior) {
            process.stdout.write(JSON.stringify({
              hookSpecificOutput: {
                hookEventName: eventName,
                decision: {
                  behavior: decision.behavior,
                  ...(decision.behavior === "deny" ? { message: decision.message ?? "Denied through Chariox." } : {})
                }
              }
            }))
          }
        } catch {}
        break
      }
      await sleep(50)
    }
  }
}
process.exit(0)
"#
}

pub(super) fn claude_native_tui_args(
    request: &LaunchProviderRequest,
    settings_file: &Path,
    mcp_config_file: Option<&Path>,
) -> Result<Vec<String>, DaemonError> {
    let mut args = vec![
        "--settings".to_string(),
        settings_file.display().to_string(),
        "--permission-mode".to_string(),
        match (
            request.execution_mode.unwrap_or_default(),
            request.permission_level.unwrap_or_default(),
        ) {
            (AgentExecutionMode::Plan, _) => "plan".to_string(),
            (AgentExecutionMode::Build, AgentPermissionLevel::Required) => "default".to_string(),
            (AgentExecutionMode::Build, AgentPermissionLevel::Yolo) => {
                "bypassPermissions".to_string()
            }
        },
    ];
    let model = normalized_claude_model(&request.model);
    if !model.is_empty() && model != "default" {
        args.extend(["--model".to_string(), model]);
    }
    if let Some(variant) = request
        .variant
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        args.extend(["--effort".to_string(), variant.to_string()]);
    }
    if let Some(session_id) = request
        .resume_state
        .as_ref()
        .and_then(|state| state.claude_session_id())
    {
        args.extend(["--resume".to_string(), session_id.to_string()]);
    }
    if request.permission_level.unwrap_or_default() == AgentPermissionLevel::Yolo {
        args.push("--allow-dangerously-skip-permissions".to_string());
    }
    if let Some(config_file) = mcp_config_file {
        args.extend([
            "--mcp-config".to_string(),
            config_file.display().to_string(),
        ]);
        args.push("--strict-mcp-config".to_string());
        if request.runtime_mcp_binding.is_some() {
            args.extend(["--allowedTools".to_string(), "mcp__chariox__*".to_string()]);
        }
    }
    if request_uses_metaagent_tools_only(request) {
        args.extend(["--tools".to_string(), String::new()]);
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;
    use std::path::Path;
    use std::process::{Command, Stdio};

    use crate::provider::{
        AgentPermissionLevel, LaunchProviderRequest, ProviderResumeState, RuntimeMcpBinding,
    };

    use super::{
        claude_native_hook_handler, claude_native_tui_args,
        ensure_claude_native_hidden_context_fits, prepare_claude_native_tui_files,
        CLAUDE_NATIVE_CONTEXT_CHUNK_BYTES, CLAUDE_NATIVE_CONTEXT_HOOK_CHUNKS,
        CLAUDE_NATIVE_MAX_HIDDEN_CONTEXT_BYTES,
    };
    use crate::error::DaemonError;

    #[test]
    fn claude_stop_failure_hook_preserves_error_fields() {
        let request = LaunchProviderRequest::new(
            "session-1",
            "claude",
            "claude-headless",
            "default",
            "sonnet",
        );
        let native = prepare_claude_native_tui_files(&request).unwrap();
        let input = serde_json::json!({
            "hook_event_name": "StopFailure",
            "error": "rate_limit",
            "error_details": "429 Too Many Requests",
            "last_assistant_message": "You've hit your session limit · resets 4am (Europe/Madrid)"
        });
        let mut child = Command::new("node")
            .arg(
                native
                    .events_file
                    .parent()
                    .unwrap()
                    .join("hook-handler.mjs"),
            )
            .env("CHARIOX_CLAUDE_NATIVE_EVENTS", &native.events_file)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.to_string().as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let recorded: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&native.events_file).unwrap()).unwrap();
        for key in [
            "hook_event_name",
            "error",
            "error_details",
            "last_assistant_message",
        ] {
            assert_eq!(
                recorded[key], input[key],
                "{key} must reach the native failure bridge"
            );
        }
    }

    #[test]
    fn hook_auto_allows_only_bypass_permissions() {
        let handler = claude_native_hook_handler();

        assert!(handler.contains("if (input.permission_mode === \"bypassPermissions\")"));
        assert!(handler.contains("behavior: \"allow\""));
        assert!(!handler.contains("permissionDecision"));
        assert!(!handler.contains("toolName.startsWith"));
        assert!(handler.contains("process.exit(0)"));
    }

    #[test]
    fn status_line_captures_official_subscription_windows_atomically() {
        if Command::new("node").arg("--version").output().is_err() {
            return;
        }
        let request = LaunchProviderRequest::new(
            "session-usage",
            "claude",
            "claude-headless",
            "default",
            "sonnet",
        );
        let native =
            prepare_claude_native_tui_files(&request).expect("native files should be prepared");
        let handler = native
            .usage_file
            .parent()
            .expect("usage file should have a root")
            .join("usage-handler.mjs");
        let mut child = Command::new("node")
            .arg(handler)
            .env("CHARIOX_CLAUDE_USAGE_FILE", &native.usage_file)
            .stdin(Stdio::piped())
            .spawn()
            .expect("usage handler should start");
        child
            .stdin
            .take()
            .expect("usage stdin should be piped")
            .write_all(
                br#"{"rate_limits":{"five_hour":{"used_percentage":21},"seven_day":{"used_percentage":34}}}"#,
            )
            .expect("usage input should write");
        assert!(child.wait().expect("usage handler should finish").success());

        let captured: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(&native.usage_file).expect("usage capture should exist"),
        )
        .expect("usage capture should be valid JSON");
        assert_eq!(captured["rate_limits"]["five_hour"]["used_percentage"], 21);
        assert!(!native
            .usage_file
            .parent()
            .expect("usage root")
            .read_dir()
            .expect("usage root should list")
            .flatten()
            .any(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("tmp")));

        let settings: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(&native.settings_file).expect("settings should exist"),
        )
        .expect("settings should be valid JSON");
        assert_eq!(settings["statusLine"]["type"], "command");
        assert!(settings["statusLine"]["command"]
            .as_str()
            .is_some_and(|command| command.contains("usage-handler.mjs")));
    }

    #[test]
    fn status_line_preserves_usage_when_a_later_tick_has_no_rate_limits() {
        if Command::new("node").arg("--version").output().is_err() {
            return;
        }
        let request = LaunchProviderRequest::new(
            "session-usage-sequence",
            "claude",
            "claude-headless",
            "default",
            "sonnet",
        );
        let native =
            prepare_claude_native_tui_files(&request).expect("native files should be prepared");
        let settings: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(&native.settings_file).expect("settings should exist"),
        )
        .expect("settings should be valid JSON");
        let usage_command = settings["statusLine"]["command"]
            .as_str()
            .expect("usage command");
        for payload in [
            br#"{"rate_limits":{"five_hour":{"used_percentage":21}}}"#.as_slice(),
            br#"{"model":{"display_name":"Claude"}}"#.as_slice(),
        ] {
            let mut child = Command::new("/bin/sh")
                .args(["-c", usage_command])
                // The generated native command must override an inherited
                // internal probe flag instead of capturing ordinary ticks.
                .env("CHARIOX_CLAUDE_CAPTURE_ALL", "1")
                .stdin(Stdio::piped())
                .spawn()
                .expect("usage handler should start");
            child
                .stdin
                .take()
                .expect("usage stdin should be piped")
                .write_all(payload)
                .expect("usage input should write");
            assert!(child.wait().expect("usage handler should finish").success());
        }

        let captured: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(&native.usage_file).expect("usage capture should exist"),
        )
        .expect("usage capture should be valid JSON");
        assert_eq!(captured["rate_limits"]["five_hour"]["used_percentage"], 21);
    }

    #[test]
    fn permission_request_hook_uses_permission_request_specific_decision_contract() {
        if Command::new("node").arg("--version").output().is_err() {
            return;
        }
        let request = LaunchProviderRequest::new(
            "session-hook-contract",
            "claude",
            "claude-headless",
            "default",
            "opus",
        );
        let native =
            prepare_claude_native_tui_files(&request).expect("native files should be prepared");
        let hook_handler = native
            .events_file
            .parent()
            .expect("events file should have a root")
            .join("hook-handler.mjs");
        let contract: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../../../fixtures/claude-permission-hook-contract.json"
        ))
        .expect("shared Claude permission contract should be valid JSON");

        for contract_case in contract {
            let mut child = Command::new("node")
                .arg(&hook_handler)
                .env("CHARIOX_CLAUDE_NATIVE_EVENTS", &native.events_file)
                .env("CHARIOX_CLAUDE_NATIVE_CONTEXT", &native.context_file)
                .env(
                    "CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES",
                    &native.context_response_dir,
                )
                .env(
                    "CHARIOX_CLAUDE_NATIVE_PERMISSION_RESPONSES",
                    &native.permission_response_dir,
                )
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("hook handler should start");
            child
                .stdin
                .take()
                .expect("hook stdin should be piped")
                .write_all(
                    serde_json::to_string(&contract_case["input"])
                        .expect("contract input should serialize")
                        .as_bytes(),
                )
                .expect("hook input should write");

            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while child
                .try_wait()
                .expect("hook process status should be readable")
                .is_none()
            {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!(
                        "PermissionRequest hook blocked instead of resolving contract case {}",
                        contract_case["name"]
                    );
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let output = child
                .wait_with_output()
                .expect("hook handler should finish");
            assert!(
                output.status.success(),
                "hook handler failed for {}: {}",
                contract_case["name"],
                String::from_utf8_lossy(&output.stderr)
            );
            let response: serde_json::Value =
                serde_json::from_slice(&output.stdout).expect("hook response should be JSON");
            assert_eq!(
                response["hookSpecificOutput"]["hookEventName"], "PermissionRequest",
                "{}",
                contract_case["name"]
            );
            assert_eq!(
                response["hookSpecificOutput"]["decision"]["behavior"], contract_case["behavior"],
                "{}",
                contract_case["name"]
            );
        }
    }

    #[test]
    fn user_prompt_hooks_deliver_large_hidden_context_without_persisted_output_fallback() {
        if Command::new("node").arg("--version").output().is_err() {
            return;
        }
        let request = LaunchProviderRequest::new(
            "session-large-context",
            "claude",
            "claude-headless",
            "default",
            "opus",
        );
        let native =
            prepare_claude_native_tui_files(&request).expect("native files should be prepared");
        let context = format!("{}🦀END", "reviewer-instruction\n".repeat(750));
        fs::write(&native.context_file, &context).expect("hidden context should write");
        let hook_handler = native
            .events_file
            .parent()
            .expect("events file should have a root")
            .join("hook-handler.mjs");
        let mut reconstructed = String::new();
        let mut context_chunks = Vec::new();

        for chunk_index in 0..CLAUDE_NATIVE_CONTEXT_HOOK_CHUNKS {
            let mut child = Command::new("node")
                .arg(&hook_handler)
                .env("CHARIOX_CLAUDE_NATIVE_EVENTS", &native.events_file)
                .env("CHARIOX_CLAUDE_NATIVE_CONTEXT", &native.context_file)
                .env(
                    "CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES",
                    &native.context_response_dir,
                )
                .env(
                    "CHARIOX_CLAUDE_NATIVE_PERMISSION_RESPONSES",
                    &native.permission_response_dir,
                )
                .env(
                    "CHARIOX_CLAUDE_NATIVE_CONTEXT_CHUNK",
                    chunk_index.to_string(),
                )
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("hook handler should start");
            child
                .stdin
                .take()
                .expect("hook stdin should be piped")
                .write_all(
                    br#"{"hook_event_name":"UserPromptSubmit","prompt":"review exact head"}"#,
                )
                .expect("hook input should write");
            let output = child.wait_with_output().expect("hook should finish");
            assert!(
                output.status.success(),
                "hook failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let response: serde_json::Value =
                serde_json::from_slice(&output.stdout).expect("hook response should be JSON");
            let chunk = response["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .expect("context chunk should be present");
            assert!(chunk.len() <= CLAUDE_NATIVE_CONTEXT_CHUNK_BYTES);
            reconstructed.push_str(chunk);
            context_chunks.push(chunk.to_string());
        }

        assert_eq!(reconstructed, context);
        assert_eq!(
            fs::read_to_string(&native.events_file)
                .expect("hook events should read")
                .lines()
                .count(),
            1,
            "context-only hooks must not duplicate prompt acknowledgements"
        );
        let settings: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(native.settings_file).unwrap()).unwrap();
        let hooks = settings["hooks"]["UserPromptSubmit"][0]["hooks"]
            .as_array()
            .expect("prompt hooks should be an array");
        assert_eq!(hooks.len(), CLAUDE_NATIVE_CONTEXT_HOOK_CHUNKS);
        for (index, hook) in hooks.iter().enumerate() {
            assert!(hook["command"]
                .as_str()
                .is_some_and(|command| command
                    .contains(&format!("CHARIOX_CLAUDE_NATIVE_CONTEXT_CHUNK={index}"))));
        }

        fs::write(&native.context_file, "").expect("context should clear");
        let mut waiting_child = Command::new("node")
            .arg(&hook_handler)
            .env("CHARIOX_CLAUDE_NATIVE_EVENTS", &native.events_file)
            .env("CHARIOX_CLAUDE_NATIVE_CONTEXT", &native.context_file)
            .env(
                "CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES",
                &native.context_response_dir,
            )
            .env(
                "CHARIOX_CLAUDE_NATIVE_PERMISSION_RESPONSES",
                &native.permission_response_dir,
            )
            .env("CHARIOX_CLAUDE_NATIVE_CONTEXT_CHUNK", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("context-only hook should start");
        waiting_child
            .stdin
            .take()
            .expect("hook stdin should be piped")
            .write_all(br#"{"hook_event_name":"UserPromptSubmit","prompt":"review exact head"}"#)
            .expect("hook input should write");
        std::thread::sleep(std::time::Duration::from_millis(100));
        fs::write(&native.context_file, &context).expect("dynamic context should publish");
        let waiting_output = waiting_child
            .wait_with_output()
            .expect("context-only hook should finish");
        assert!(waiting_output.status.success());
        let waiting_response: serde_json::Value = serde_json::from_slice(&waiting_output.stdout)
            .expect("context-only hook response should be JSON");
        assert_eq!(
            waiting_response["hookSpecificOutput"]["additionalContext"].as_str(),
            Some(context_chunks[1].as_str())
        );

        fs::write(&native.context_file, "").expect("context should clear");
        fs::write(&native.events_file, "").expect("events should clear");
        let spawn_context_hook = |chunk_index: usize| {
            let mut child = Command::new("node")
                .arg(&hook_handler)
                .env("CHARIOX_CLAUDE_NATIVE_EVENTS", &native.events_file)
                .env("CHARIOX_CLAUDE_NATIVE_CONTEXT", &native.context_file)
                .env(
                    "CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES",
                    &native.context_response_dir,
                )
                .env(
                    "CHARIOX_CLAUDE_NATIVE_PERMISSION_RESPONSES",
                    &native.permission_response_dir,
                )
                .env(
                    "CHARIOX_CLAUDE_NATIVE_CONTEXT_CHUNK",
                    chunk_index.to_string(),
                )
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("context hook should start");
            child
                .stdin
                .take()
                .expect("hook stdin should be piped")
                .write_all(br#"{"hook_event_name":"UserPromptSubmit","prompt":"dynamic context"}"#)
                .expect("hook input should write");
            child
        };
        let response_hook = spawn_context_hook(0);
        let response_deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let request_id = loop {
            if let Some(request_id) =
                fs::read_to_string(&native.events_file)
                    .ok()
                    .and_then(|events| {
                        events.lines().find_map(|line| {
                            serde_json::from_str::<serde_json::Value>(line)
                                .ok()?
                                .get("hook_context_request_id")?
                                .as_str()
                                .map(str::to_string)
                        })
                    })
            {
                break request_id;
            }
            assert!(
                std::time::Instant::now() < response_deadline,
                "primary hook should publish its response request id"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        let sibling_hook = spawn_context_hook(1);
        fs::write(
            native
                .context_response_dir
                .join(format!("{request_id}.txt")),
            &context,
        )
        .expect("dynamic context response should publish");
        let response_output = response_hook
            .wait_with_output()
            .expect("primary response hook should finish");
        let sibling_output = sibling_hook
            .wait_with_output()
            .expect("sibling response hook should finish");
        assert!(response_output.status.success());
        assert!(sibling_output.status.success());
        let response: serde_json::Value = serde_json::from_slice(&response_output.stdout)
            .expect("primary response should be JSON");
        let sibling: serde_json::Value = serde_json::from_slice(&sibling_output.stdout)
            .expect("sibling response should be JSON");
        assert_eq!(
            response["hookSpecificOutput"]["additionalContext"].as_str(),
            Some(context_chunks[0].as_str())
        );
        assert_eq!(
            sibling["hookSpecificOutput"]["additionalContext"].as_str(),
            Some(context_chunks[1].as_str())
        );
        assert_eq!(
            fs::read_to_string(&native.context_file).expect("context file should read"),
            context,
            "the primary response hook must publish the full context for sibling chunks"
        );
    }

    #[test]
    fn oversized_hidden_context_fails_before_claude_receives_a_partial_instruction_set() {
        let five_chunk_reviewer_context = "x".repeat(CLAUDE_NATIVE_CONTEXT_CHUNK_BYTES * 5);
        ensure_claude_native_hidden_context_fits(
            "run-five-chunk-reviewer-context",
            &five_chunk_reviewer_context,
        )
        .expect("the native bridge should carry a bounded five-chunk reviewer context");

        let exact = "x".repeat(CLAUDE_NATIVE_MAX_HIDDEN_CONTEXT_BYTES);
        ensure_claude_native_hidden_context_fits("run-exact", &exact)
            .expect("the exact transport ceiling should be accepted");

        let mut multibyte_boundary_spill = "x".repeat(5_999);
        for _ in 0..CLAUDE_NATIVE_CONTEXT_HOOK_CHUNKS - 1 {
            multibyte_boundary_spill.push('🦀');
            multibyte_boundary_spill.push_str(&"x".repeat(5_995));
        }
        multibyte_boundary_spill.push('🦀');
        assert!(multibyte_boundary_spill.len() <= CLAUDE_NATIVE_MAX_HIDDEN_CONTEXT_BYTES);
        let error = ensure_claude_native_hidden_context_fits(
            "run-multibyte-boundary-spill",
            &multibyte_boundary_spill,
        )
        .expect_err("UTF-8 boundary spill requiring a fifth chunk must fail closed");
        assert!(matches!(
            error,
            DaemonError::ProviderProtocol {
                provider_run_id,
                operation: "claude_hidden_context_size",
                ..
            } if provider_run_id == "run-multibyte-boundary-spill"
        ));

        let oversized = format!("{exact}x");
        let error = ensure_claude_native_hidden_context_fits("run-oversized", &oversized)
            .expect_err("oversized hidden context must fail closed");
        assert!(matches!(
            error,
            DaemonError::ProviderProtocol {
                provider_run_id,
                operation: "claude_hidden_context_size",
                ..
            } if provider_run_id == "run-oversized"
        ));
    }

    #[test]
    fn session_start_hook_removes_materialized_mcp_credentials() {
        if Command::new("node").arg("--version").output().is_err() {
            return;
        }
        let request = LaunchProviderRequest::new(
            "session-1",
            "claude",
            "claude",
            "default",
            "claude-sonnet-4-6",
        )
        .with_runtime_mcp_binding(RuntimeMcpBinding::new(
            "http://127.0.0.1:43120/mcp",
            "private-token",
        ));
        let mut native =
            prepare_claude_native_tui_files(&request).expect("native files should be prepared");
        native
            .materialize_mcp_config(&request)
            .expect("MCP config should materialize");
        let config_path = native
            .mcp_config_file()
            .expect("MCP config path should exist")
            .to_path_buf();
        let hook_handler = native
            .events_file
            .parent()
            .expect("events file should have a root")
            .join("hook-handler.mjs");
        let mut child = Command::new("node")
            .arg(hook_handler)
            .env("CHARIOX_CLAUDE_NATIVE_EVENTS", &native.events_file)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("hook handler should start");
        child
            .stdin
            .take()
            .expect("hook stdin should be piped")
            .write_all(br#"{"hook_event_name":"SessionStart"}"#)
            .expect("hook input should write");

        let output = child
            .wait_with_output()
            .expect("hook handler should finish");

        assert!(
            output.status.success(),
            "hook handler failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!config_path.exists());
        assert!(std::fs::read_to_string(&native.events_file)
            .expect("hook event should be recorded")
            .contains("SessionStart"));
    }

    #[test]
    fn native_tui_resumes_requested_claude_session() {
        let request = LaunchProviderRequest::new(
            "session-1",
            "claude",
            "claude-headless",
            "default",
            "sonnet",
        )
        .with_resume_state(ProviderResumeState::from_claude_session_id(
            "claude-session-1",
        ));

        let args = claude_native_tui_args(&request, Path::new("settings.json"), None)
            .expect("Claude native TUI args should resolve");

        assert!(args
            .windows(2)
            .any(|pair| pair == ["--resume", "claude-session-1"]));
    }

    #[test]
    fn native_settings_accept_dangerous_mode_only_for_yolo_agents() {
        let yolo = LaunchProviderRequest::new(
            "session-1",
            "claude",
            "claude-headless",
            "default",
            "sonnet",
        )
        .with_permission_level(AgentPermissionLevel::Yolo);
        let required = LaunchProviderRequest::new(
            "session-2",
            "claude",
            "claude-headless",
            "default",
            "sonnet",
        )
        .with_permission_level(AgentPermissionLevel::Required);

        let yolo_files = prepare_claude_native_tui_files(&yolo).unwrap();
        let required_files = prepare_claude_native_tui_files(&required).unwrap();
        let yolo_settings: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(yolo_files.settings_file).unwrap()).unwrap();
        let required_settings: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(required_files.settings_file).unwrap())
                .unwrap();

        assert_eq!(yolo_settings["skipDangerousModePermissionPrompt"], true);
        assert_eq!(
            required_settings["skipDangerousModePermissionPrompt"],
            false
        );
        let yolo_hook = yolo_settings["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(yolo_hook.contains("CHARIOX_CLAUDE_NATIVE_EVENTS="));
        assert!(yolo_hook.contains("CHARIOX_CLAUDE_NATIVE_CONTEXT="));
        assert!(yolo_hook.contains("CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES="));
        assert!(yolo_hook.contains("CHARIOX_CLAUDE_NATIVE_PERMISSION_RESPONSES="));
        assert_eq!(
            yolo_settings["hooks"]["PermissionRequest"][0]["matcher"],
            "*"
        );
    }
}
