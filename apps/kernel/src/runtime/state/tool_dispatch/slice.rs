use base64::Engine;
use std::io::Read;
use wait_timeout::ChildExt;

use crate::error::DaemonError;
use crate::runtime::state::KernelRuntimeState;

mod controller_browser;
mod controller_browser_compatibility;
mod controller_browser_projection;
mod controller_browser_runtime;
mod controller_computer;
mod controller_computer_observation;
mod slice_browser;
use slice_browser::*;

const DEFAULT_SLICE_SCREEN_COMMAND_TIMEOUT_MS: u64 = 70_000;
const ROOM_COMPUTER_INPUT_TIMEOUT_MS: u64 = 5_000;
const SLICE_SCREEN_COMMAND_OUTPUT_MAX_BYTES: usize = 256 * 1024;

impl KernelRuntimeState {
    pub(super) async fn dispatch_slice_runtime_tool_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let Some(slice_id) = self
            .room_browser_slice_for_tool(provider_run.session_id(), tool_name)
            .or_else(|| self.slice_kernel_id())
        else {
            return Err(DaemonError::LocalTransport {
                operation: "dispatch_slice_runtime_tool_call",
                message: "slice runtime tools are only available inside Chariox slices".to_string(),
            });
        };
        let agent_id =
            provider_run
                .agent_instance_id()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "dispatch_slice_runtime_tool_call",
                    message: "provider run is not bound to an agent".to_string(),
                })?;
        if self.browser_controller_enabled_for_room(provider_run.session_id())
            && super::is_room_browser_controller_runtime_tool(tool_name)
        {
            return self
                .dispatch_room_browser_controller_runtime_tool_call(
                    provider_run.session_id(),
                    &slice_id,
                    agent_id,
                    tool_name,
                    arguments,
                )
                .await;
        }
        let output = match tool_name {
            crate::transport::runtime_tools::SLICE_SCREEN_STATUS_TOOL => {
                run_slice_screen_command(vec!["status".to_string()]).await?
            }
            crate::transport::runtime_tools::SLICE_SCREENSHOT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::SliceScreenshotArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_screenshot",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let image_path = args
                    .path
                    .unwrap_or_else(|| "/tmp/chariox-slice-screenshot.png".to_string());
                let output =
                    run_slice_screen_command(vec!["screenshot".to_string(), image_path.clone()])
                        .await?;
                let mut payload = slice_tool_payload(&slice_id, agent_id, &output);
                payload["image_path"] = serde_json::Value::String(image_path.clone());
                payload["mime_type"] = serde_json::Value::String("image/png".to_string());
                if output.success && args.return_image_base64 {
                    let image_path = std::path::PathBuf::from(&image_path);
                    let image_bytes = read_slice_screenshot_for_mcp(&image_path)?;
                    payload["image_base64"] = serde_json::Value::String(
                        base64::engine::general_purpose::STANDARD.encode(image_bytes),
                    );
                }
                return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: output.success,
                    payload,
                });
            }
            crate::transport::runtime_tools::SLICE_OCR_TOOL => {
                let args = serde_json::from_value::<crate::transport::runtime_tools::SliceOcrArgs>(
                    arguments,
                )
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_ocr",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                reject_room_artifact_for_local_slice(
                    args.artifact_id.as_deref(),
                    "runtime_tool_slice_ocr",
                )?;
                let mut command_args = vec!["ocr".to_string()];
                if let Some(image_path) = args.image_path {
                    command_args.push(image_path);
                }
                let output = run_slice_screen_command(command_args).await?;
                let mut payload = slice_tool_payload(&slice_id, agent_id, &output);
                payload["text"] = serde_json::Value::String(output.stdout.as_str().to_string());
                return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: output.success,
                    payload,
                });
            }
            crate::transport::runtime_tools::SLICE_FIND_TEXT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::SliceFindTextArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_find_text",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                reject_room_artifact_for_local_slice(
                    args.artifact_id.as_deref(),
                    "runtime_tool_slice_find_text",
                )?;
                let query = validated_slice_find_text_query(&args.query)?;
                let mut command_args = vec!["find-text".to_string(), query];
                if let Some(image_path) = args.image_path {
                    command_args.push(image_path);
                }
                let output = run_slice_screen_command(command_args).await?;
                return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: output.success,
                    payload: slice_find_text_payload(&slice_id, agent_id, &output),
                });
            }
            crate::transport::runtime_tools::SLICE_MOUSE_TOOL => {
                let args =
                    serde_json::from_value::<crate::transport::runtime_tools::SliceMouseArgs>(
                        arguments,
                    )
                    .map_err(|error| DaemonError::LocalTransport {
                        operation: "runtime_tool_slice_mouse",
                        message: format!("invalid tool arguments: {error}"),
                    })?;
                run_slice_screen_command(slice_mouse_command_args(args)?).await?
            }
            crate::transport::runtime_tools::SLICE_KEYBOARD_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::SliceKeyboardArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_keyboard",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                run_slice_screen_command(slice_keyboard_command_args(args)?).await?
            }
            crate::transport::runtime_tools::SLICE_CLIPBOARD_WRITE_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::SliceClipboardWriteArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_clipboard_write",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let text =
                    crate::transport::room_browser_controller::RoomComputerClipboardText::from_zeroizing(
                        args.into_zeroizing(),
                    );
                let utf8_byte_count = text.as_str().len();
                let character_count = text.as_str().chars().count();
                run_room_clipboard_write_inner(text, None).await?;
                return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "source": "slice_computer",
                        "slice_id": slice_id,
                        "agent_id": agent_id,
                        "action_kind": "clipboard_write",
                        "utf8_byte_count": utf8_byte_count,
                        "character_count": character_count,
                    }),
                });
            }
            crate::transport::runtime_tools::PASTE_SECRET_TO_SLICE_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::PasteSecretToSliceArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_paste_secret_to_slice",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let status_output =
                    run_slice_screen_command(vec!["browser-status".to_string()]).await?;
                let browser_status = slice_browser_json(&status_output)?;
                let browser_url = browser_status_url(&browser_status)?;
                ensure_browser_target_matches_expectations(&browser_status, &args)?;
                let selector = browser_selector(args.selector.as_deref(), args.field_id.as_deref());
                ensure_browser_fill_target(&browser_status, selector.as_deref())?;
                ensure_browser_secret_target_is_masked(&browser_status, selector.as_deref())?;
                let secret = match self
                    .resolve_remote_home_credential_secret(
                        provider_run,
                        &args.credential_id,
                        crate::transport::relay_peer::RemoteCredentialSecretInjection::Browser {
                            target_url: browser_url.clone(),
                        },
                    )
                    .await?
                {
                    Some(secret) => secret,
                    None => {
                        let _vault_unlock = self
                            .ensure_vault_unlocked_for_provider_run(
                                provider_run,
                                "runtime_tool_paste_secret_to_slice",
                            )
                            .await?;
                        let user_config = self.owned.config_projection.snapshot().user_config;
                        let credentials = crate::credential::load_user_credentials()?;
                        let service = crate::secret::RuntimeSecretService::with_vault_config(
                            credentials,
                            &user_config.credential_vault,
                        )?;
                        zeroize::Zeroizing::new(service.browser_secret_input_for_target_url(
                            &args.credential_id,
                            &browser_url,
                        )?)
                    }
                };
                let mut command_args = vec![if args.submit {
                    "secret-paste-submit-stdin".to_string()
                } else {
                    "secret-paste-stdin".to_string()
                }];
                if let Some(selector) = selector.clone() {
                    command_args.push(selector);
                }
                let output = run_slice_screen_command_with_stdin(command_args, secret).await?;
                return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: output.success,
                    payload: secret_paste_payload(
                        &slice_id,
                        agent_id,
                        &args.credential_id,
                        args.submit && output.success,
                        &output,
                    ),
                });
            }
            crate::transport::runtime_tools::PASTE_SECRET_TO_COMPUTER_TOOL => {
                // Keep the infrequent approval and vault flow off the shared dispatch future's
                // stack; this router is also used by latency-sensitive non-secret commands.
                return Box::pin(self.dispatch_computer_secret_input_tool(
                    provider_run,
                    agent_id,
                    arguments,
                ))
                .await;
            }
            crate::transport::runtime_tools::SLICE_OPEN_URL_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::SliceOpenUrlArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_open_url",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                run_slice_screen_command(vec!["open-url".to_string(), args.url]).await?
            }
            crate::transport::runtime_tools::SLICE_BROWSER_STATUS_TOOL => {
                let output = run_slice_screen_command(vec!["browser-status".to_string()]).await?;
                return Ok(slice_browser_tool_result(&slice_id, agent_id, output));
            }
            crate::transport::runtime_tools::SLICE_BROWSER_TAB_TOOL => {
                serde_json::from_value::<crate::transport::runtime_tools::SliceBrowserTabArgs>(
                    arguments,
                )
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_browser_tab",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                return Err(DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_browser_tab",
                    message:
                        "browser tab lifecycle requires the long-running Room browser controller"
                            .to_string(),
                });
            }
            crate::transport::runtime_tools::SLICE_BROWSER_HISTORY_TOOL => {
                serde_json::from_value::<crate::transport::runtime_tools::SliceBrowserHistoryArgs>(
                    arguments,
                )
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_browser_history",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                return Err(DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_browser_history",
                    message: "browser history requires the long-running Room browser controller"
                        .to_string(),
                });
            }
            crate::transport::runtime_tools::SLICE_BROWSER_FIND_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::SliceBrowserFindArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_browser_find",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let output = run_slice_screen_command(vec![
                    "browser-find".to_string(),
                    args.query,
                    args.kind.unwrap_or_else(|| "any".to_string()),
                ])
                .await?;
                return Ok(slice_browser_tool_result(&slice_id, agent_id, output));
            }
            crate::transport::runtime_tools::SLICE_BROWSER_FILL_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::SliceBrowserFillArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_browser_fill",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let selector = required_browser_selector(
                    args.selector.as_deref(),
                    args.field_id.as_deref(),
                    "runtime_tool_slice_browser_fill",
                )?;
                let output =
                    run_slice_screen_command(vec!["browser-fill".to_string(), selector, args.text])
                        .await?;
                return Ok(slice_browser_tool_result(&slice_id, agent_id, output));
            }
            crate::transport::runtime_tools::SLICE_BROWSER_CLICK_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::SliceBrowserClickArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_browser_click",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let selector = required_browser_selector(
                    args.selector.as_deref(),
                    args.field_id.as_deref(),
                    "runtime_tool_slice_browser_click",
                )?;
                let output =
                    run_slice_screen_command(vec!["browser-click".to_string(), selector]).await?;
                return Ok(slice_browser_tool_result(&slice_id, agent_id, output));
            }
            crate::transport::runtime_tools::SLICE_BROWSER_SUBMIT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::SliceBrowserSubmitArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_browser_submit",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let mut command_args = vec!["browser-submit".to_string()];
                if let Some(selector) =
                    browser_selector(args.selector.as_deref(), args.field_id.as_deref())
                {
                    command_args.push(selector);
                }
                let output = run_slice_screen_command(command_args).await?;
                return Ok(slice_browser_tool_result(&slice_id, agent_id, output));
            }
            crate::transport::runtime_tools::SLICE_BROWSER_DIALOG_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::SliceBrowserDialogArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_browser_dialog",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let mut command_args = vec!["browser-dialog".to_string(), args.action];
                if let Some(prompt_text) = args.prompt_text {
                    command_args.push(prompt_text);
                }
                let output = run_slice_screen_command(command_args).await?;
                return Ok(slice_browser_tool_result(&slice_id, agent_id, output));
            }
            crate::transport::runtime_tools::SLICE_BROWSER_EVENTS_TOOL => {
                serde_json::from_value::<crate::transport::runtime_tools::SliceBrowserEventsArgs>(
                    arguments,
                )
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_browser_events",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                return Err(DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_browser_events",
                    message: "browser events require the long-running Room browser controller"
                        .to_string(),
                });
            }
            crate::transport::runtime_tools::SLICE_BROWSER_DOWNLOADS_TOOL => {
                serde_json::from_value::<
                    crate::transport::runtime_tools::SliceBrowserDownloadsArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_browser_downloads",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                return Err(DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_browser_downloads",
                    message: "browser downloads require the long-running Room browser controller"
                        .to_string(),
                });
            }
            crate::transport::runtime_tools::SLICE_BROWSER_UPLOAD_TOOL => {
                serde_json::from_value::<crate::transport::runtime_tools::SliceBrowserUploadArgs>(
                    arguments,
                )
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_browser_upload",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                return Err(DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_browser_upload",
                    message: "browser uploads require the long-running Room browser controller"
                        .to_string(),
                });
            }
            crate::transport::runtime_tools::SLICE_BROWSER_PERMISSION_TOOL => {
                serde_json::from_value::<
                    crate::transport::runtime_tools::SliceBrowserPermissionArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_browser_permission",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                return Err(DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_browser_permission",
                    message: "browser permissions require the long-running Room browser controller"
                        .to_string(),
                });
            }
            crate::transport::runtime_tools::SLICE_BROWSER_TEXT_TOOL => {
                let output = run_slice_screen_command(vec!["browser-text".to_string()]).await?;
                let mut payload = slice_tool_payload(&slice_id, agent_id, &output);
                payload["text"] = serde_json::Value::String(output.stdout.as_str().to_string());
                return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: output.success,
                    payload,
                });
            }
            crate::transport::runtime_tools::SLICE_BROWSER_WAIT_FOR_TEXT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::SliceBrowserWaitForTextArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_browser_wait_for_text",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let output = run_slice_screen_command(vec![
                    "browser-wait-text".to_string(),
                    args.text,
                    browser_timeout_arg(args.timeout_ms),
                ])
                .await?;
                return Ok(slice_browser_tool_result(&slice_id, agent_id, output));
            }
            crate::transport::runtime_tools::SLICE_BROWSER_WAIT_FOR_SELECTOR_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::SliceBrowserWaitForSelectorArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_browser_wait_for_selector",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let output = run_slice_screen_command(vec![
                    "browser-wait-selector".to_string(),
                    args.selector,
                    browser_timeout_arg(args.timeout_ms),
                ])
                .await?;
                return Ok(slice_browser_tool_result(&slice_id, agent_id, output));
            }
            crate::transport::runtime_tools::SLICE_BROWSER_WAIT_FOR_IDLE_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::SliceBrowserWaitForIdleArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_browser_wait_for_idle",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let output = run_slice_screen_command(vec![
                    "browser-wait-idle".to_string(),
                    browser_timeout_arg(args.timeout_ms),
                ])
                .await?;
                return Ok(slice_browser_tool_result(&slice_id, agent_id, output));
            }
            _ => {
                return Err(DaemonError::LocalTransport {
                    operation: "dispatch_slice_runtime_tool_call",
                    message: format!("unknown slice runtime tool `{tool_name}`"),
                });
            }
        };
        Ok(crate::transport::runtime_tools::RuntimeToolResult {
            ok: output.success,
            payload: slice_tool_payload(&slice_id, agent_id, &output),
        })
    }
}

fn reject_room_artifact_for_local_slice(
    artifact_id: Option<&str>,
    operation: &'static str,
) -> Result<(), DaemonError> {
    if artifact_id.is_some() {
        return Err(DaemonError::LocalTransport {
            operation,
            message:
                "artifact_id is only valid for an opaque Room screenshot; local slices use image_path"
                    .to_string(),
        });
    }
    Ok(())
}

fn validated_slice_find_text_query(query: &str) -> Result<String, DaemonError> {
    let query = query.trim();
    if query.is_empty() || query.len() > 4096 {
        return Err(DaemonError::LocalTransport {
            operation: "runtime_tool_slice_find_text",
            message: "query must contain between 1 and 4096 UTF-8 bytes".to_string(),
        });
    }
    Ok(query.to_string())
}

fn read_slice_screenshot_for_mcp(path: &std::path::Path) -> Result<Vec<u8>, DaemonError> {
    let file = std::fs::File::open(path).map_err(|error| DaemonError::LocalTransport {
        operation: "runtime_tool_slice_screenshot",
        message: format!("failed to open screenshot `{}`: {error}", path.display()),
    })?;
    read_bounded_png(
        file,
        super::super::room_screenshot::ROOM_SCREENSHOT_INLINE_MAX_BYTES as usize,
    )
    .map_err(|message| DaemonError::LocalTransport {
        operation: "runtime_tool_slice_screenshot",
        message: format!("screenshot `{}` {message}", path.display()),
    })
}

fn read_bounded_png(reader: impl Read, max_bytes: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take((max_bytes + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("could not be read: {error}"))?;
    if bytes.len() > max_bytes {
        return Err(format!("exceeds the {max_bytes} byte runtime MCP limit"));
    }
    if !bytes.starts_with(super::super::room_screenshot::ROOM_SCREENSHOT_PNG_SIGNATURE) {
        return Err("is empty or is not a PNG image".to_string());
    }
    Ok(bytes)
}

struct SliceScreenCommandOutput {
    success: bool,
    status_code: Option<i32>,
    stdout: zeroize::Zeroizing<String>,
    stderr: zeroize::Zeroizing<String>,
    stdout_truncated: bool,
    stderr_truncated: bool,
    sensitive_output: bool,
}

impl std::fmt::Debug for SliceScreenCommandOutput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = formatter.debug_struct("SliceScreenCommandOutput");
        debug
            .field("success", &self.success)
            .field("status_code", &self.status_code);
        if self.sensitive_output {
            debug
                .field("stdout", &"[redacted sensitive helper output]")
                .field("stderr", &"[redacted sensitive helper output]");
        } else {
            debug
                .field("stdout", &self.stdout.as_str())
                .field("stderr", &self.stderr.as_str());
        }
        debug
            .field("stdout_truncated", &self.stdout_truncated)
            .field("stderr_truncated", &self.stderr_truncated)
            .finish()
    }
}

async fn run_slice_screen_command(
    args: Vec<String>,
) -> Result<SliceScreenCommandOutput, DaemonError> {
    run_slice_screen_command_inner(args, None, None).await
}

pub(in crate::runtime::state) async fn execute_room_computer_observation(
    call: crate::transport::relay_peer::RemoteRoomComputerObservationCall,
    artifact_path: Option<std::path::PathBuf>,
) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
    let output = match &call {
        crate::transport::relay_peer::RemoteRoomComputerObservationCall::ScreenStatus => {
            run_slice_screen_command(vec!["status".to_string()]).await?
        }
        crate::transport::relay_peer::RemoteRoomComputerObservationCall::Ocr { .. } => {
            let mut args = vec!["ocr".to_string()];
            if let Some(path) = artifact_path.as_ref() {
                args.push(room_computer_artifact_path(path)?);
            }
            run_slice_screen_command(args).await?
        }
        crate::transport::relay_peer::RemoteRoomComputerObservationCall::FindText {
            query, ..
        } => {
            let query = validated_slice_find_text_query(query)?;
            let mut args = vec!["find-text".to_string(), query];
            if let Some(path) = artifact_path.as_ref() {
                args.push(room_computer_artifact_path(path)?);
            }
            run_slice_screen_command(args).await?
        }
    };
    let is_find_text = matches!(
        &call,
        crate::transport::relay_peer::RemoteRoomComputerObservationCall::FindText { .. }
    );
    let mut payload = if is_find_text {
        slice_find_text_payload("", "", &output)
    } else {
        slice_tool_payload("", "", &output)
    };
    if matches!(
        &call,
        crate::transport::relay_peer::RemoteRoomComputerObservationCall::Ocr { .. }
    ) {
        payload["text"] = serde_json::Value::String(output.stdout.as_str().to_string());
    }
    if let Some(payload) = payload.as_object_mut() {
        for field in [
            "slice_id", "agent_id", "display", "viewer", "stdout", "stderr",
        ] {
            payload.remove(field);
        }
    }
    Ok(crate::transport::runtime_tools::RuntimeToolResult {
        ok: output.success,
        payload,
    })
}

fn room_computer_artifact_path(path: &std::path::Path) -> Result<String, DaemonError> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "environment.computer.observe",
            message: "Room screenshot artifact path is not valid UTF-8".to_string(),
        })
}

pub(in crate::runtime::state) async fn capture_room_environment_screenshot(
    destination: &std::path::Path,
) -> Result<(), DaemonError> {
    let destination = destination
        .to_str()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "environment.screenshot.capture",
            message: "screenshot destination is not valid UTF-8".to_string(),
        })?;
    let output =
        run_slice_screen_command(vec!["screenshot".to_string(), destination.to_string()]).await?;
    if !output.success {
        return Err(DaemonError::LocalTransport {
            operation: "environment.screenshot.capture",
            message: format!(
                "slice screenshot helper exited with status {}",
                output
                    .status_code
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            ),
        });
    }
    let metadata = std::fs::metadata(destination).map_err(|error| DaemonError::LocalTransport {
        operation: "environment.screenshot.capture",
        message: format!("slice screenshot helper did not create the capture: {error}"),
    })?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(DaemonError::LocalTransport {
            operation: "environment.screenshot.capture",
            message: "slice screenshot helper produced an empty capture".to_string(),
        });
    }
    Ok(())
}

async fn run_slice_screen_command_with_stdin(
    args: Vec<String>,
    stdin: zeroize::Zeroizing<String>,
) -> Result<SliceScreenCommandOutput, DaemonError> {
    run_slice_screen_command_inner(args, Some(stdin), None).await
}

async fn run_slice_screen_command_inner(
    args: Vec<String>,
    stdin: Option<zeroize::Zeroizing<String>>,
    timeout_override_ms: Option<u64>,
) -> Result<SliceScreenCommandOutput, DaemonError> {
    run_slice_screen_command_inner_with_output_policy(args, stdin, timeout_override_ms, None, false)
        .await
}

async fn run_slice_screen_command_inner_exact_stdout(
    args: Vec<String>,
    timeout_override_ms: Option<u64>,
) -> Result<SliceScreenCommandOutput, DaemonError> {
    run_slice_screen_command_inner_with_output_policy(args, None, timeout_override_ms, None, true)
        .await
}

async fn run_slice_screen_command_inner_with_cancellation(
    args: Vec<String>,
    stdin: Option<zeroize::Zeroizing<String>>,
    timeout_override_ms: Option<u64>,
    cancellation: Option<crate::runtime::computer_input_execution::ComputerInputCancellation>,
) -> Result<SliceScreenCommandOutput, DaemonError> {
    run_slice_screen_command_inner_with_output_policy(
        args,
        stdin,
        timeout_override_ms,
        cancellation,
        false,
    )
    .await
}

async fn run_slice_screen_command_inner_with_output_policy(
    args: Vec<String>,
    stdin: Option<zeroize::Zeroizing<String>>,
    timeout_override_ms: Option<u64>,
    cancellation: Option<crate::runtime::computer_input_execution::ComputerInputCancellation>,
    preserve_stdout: bool,
) -> Result<SliceScreenCommandOutput, DaemonError> {
    let tool_path = std::env::var("CHARIOX_SLICE_SCREEN_TOOL")
        .unwrap_or_else(|_| "/opt/chariox-slice/slice-screen.sh".to_string());
    let sensitive_output = preserve_stdout || stdin.is_some();
    tokio::task::spawn_blocking(move || {
        let mut command = std::process::Command::new(&tool_path);
        command
            .args(&args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if stdin.is_some() {
            command.stdin(std::process::Stdio::piped());
        }
        #[cfg(unix)]
        if cancellation.is_some() {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command
            .spawn()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "run_slice_screen_command",
                message: format!("failed to run `{tool_path}`: {error}"),
            })?;
        let process_group = child.id();
        if let Some(cancellation) = cancellation.as_ref() {
            cancellation.register_process_group(process_group);
        }
        if let Some(stdin) = stdin {
            use std::io::Write;
            let Some(mut child_stdin) = child.stdin.take() else {
                if let Some(cancellation) = cancellation.as_ref() {
                    cancellation.terminate_process_group();
                    cancellation.clear_process_group(process_group);
                }
                let _ = child.kill();
                let _ = child.wait();
                return Err(DaemonError::LocalTransport {
                    operation: "run_slice_screen_command",
                    message: "slice screen command did not expose stdin".to_string(),
                });
            };
            if let Err(error) = child_stdin.write_all(stdin.as_bytes()) {
                if let Some(cancellation) = cancellation.as_ref() {
                    cancellation.terminate_process_group();
                    cancellation.clear_process_group(process_group);
                }
                let _ = child.kill();
                let _ = child.wait();
                if cancellation.as_ref().is_some_and(|value| value.requested()) {
                    return Err(computer_input_cancelled());
                }
                return Err(DaemonError::LocalTransport {
                    operation: "run_slice_screen_command",
                    message: format!("failed to write slice screen stdin: {error}"),
                });
            }
        }
        drop(child.stdin.take());
        let Some(stdout) = child.stdout.take() else {
            if let Some(cancellation) = cancellation.as_ref() {
                cancellation.terminate_process_group();
                cancellation.clear_process_group(process_group);
            }
            let _ = child.kill();
            let _ = child.wait();
            return Err(DaemonError::LocalTransport {
                operation: "run_slice_screen_command",
                message: "slice screen command did not expose stdout".to_string(),
            });
        };
        let Some(stderr) = child.stderr.take() else {
            if let Some(cancellation) = cancellation.as_ref() {
                cancellation.terminate_process_group();
                cancellation.clear_process_group(process_group);
            }
            let _ = child.kill();
            let _ = child.wait();
            drop(stdout);
            return Err(DaemonError::LocalTransport {
                operation: "run_slice_screen_command",
                message: "slice screen command did not expose stderr".to_string(),
            });
        };
        let stdout_reader = std::thread::spawn(move || {
            if preserve_stdout {
                read_child_output_exact_utf8(stdout)
            } else {
                read_child_output(stdout)
            }
        });
        let stderr_reader = std::thread::spawn(move || read_child_output(stderr));
        let timeout_ms = timeout_override_ms.unwrap_or_else(slice_screen_command_timeout_ms);
        let status = match child.wait_timeout(std::time::Duration::from_millis(timeout_ms)) {
            Ok(Some(status)) => {
                if let Some(cancellation) = cancellation.as_ref() {
                    cancellation.clear_process_group(process_group);
                }
                status
            }
            Ok(None) => {
                if let Some(cancellation) = cancellation.as_ref() {
                    cancellation.terminate_process_group();
                    cancellation.clear_process_group(process_group);
                }
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                if cancellation.as_ref().is_some_and(|value| value.requested()) {
                    return Err(computer_input_cancelled());
                }
                return Err(DaemonError::LocalTransport {
                    operation: "run_slice_screen_command",
                    message: format!("slice screen command timed out after {timeout_ms}ms"),
                });
            }
            Err(error) => {
                if let Some(cancellation) = cancellation.as_ref() {
                    cancellation.terminate_process_group();
                    cancellation.clear_process_group(process_group);
                }
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                if cancellation.as_ref().is_some_and(|value| value.requested()) {
                    return Err(computer_input_cancelled());
                }
                return Err(DaemonError::LocalTransport {
                    operation: "run_slice_screen_command",
                    message: format!("failed to wait for `{tool_path}`: {error}"),
                });
            }
        };
        let (stdout, stdout_truncated) =
            stdout_reader
                .join()
                .map_err(|_| DaemonError::LocalTransport {
                    operation: "run_slice_screen_command",
                    message: "slice screen stdout reader panicked".to_string(),
                })??;
        let (stderr, stderr_truncated) =
            stderr_reader
                .join()
                .map_err(|_| DaemonError::LocalTransport {
                    operation: "run_slice_screen_command",
                    message: "slice screen stderr reader panicked".to_string(),
                })??;
        if !status.success() && cancellation.as_ref().is_some_and(|value| value.requested()) {
            return Err(computer_input_cancelled());
        }
        Ok(SliceScreenCommandOutput {
            success: status.success(),
            status_code: status.code(),
            stdout,
            stderr,
            stdout_truncated,
            stderr_truncated,
            sensitive_output,
        })
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "run_slice_screen_command",
        message: error.to_string(),
    })?
}

pub(crate) async fn run_room_pointer_click(
    x: u32,
    y: u32,
    button: crate::transport::room_browser_controller::RoomComputerPointerButton,
    click_count: u8,
    desktop_pixel_width: u32,
    desktop_pixel_height: u32,
    cancellation: crate::runtime::computer_input_execution::ComputerInputCancellation,
) -> Result<(), DaemonError> {
    if desktop_pixel_width == 0
        || desktop_pixel_height == 0
        || x >= desktop_pixel_width
        || y >= desktop_pixel_height
    {
        return Err(room_computer_input_error(
            "environment_pointer_out_of_bounds",
        ));
    }
    if !matches!(click_count, 1 | 2) {
        return Err(room_computer_input_error("environment_invalid_click_count"));
    }
    let button = room_computer_pointer_button_arg(button);
    let output = run_slice_screen_command_inner_with_cancellation(
        vec![
            "pointer-click".to_string(),
            x.to_string(),
            y.to_string(),
            button.to_string(),
            click_count.to_string(),
        ],
        None,
        Some(ROOM_COMPUTER_INPUT_TIMEOUT_MS),
        Some(cancellation),
    )
    .await?;
    if output.success {
        Ok(())
    } else {
        Err(room_computer_input_error(&format!(
            "slice pointer helper exited with status {}",
            output
                .status_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        )))
    }
}

pub(crate) async fn run_room_pointer_move(
    x: u32,
    y: u32,
    desktop_pixel_width: u32,
    desktop_pixel_height: u32,
    cancellation: crate::runtime::computer_input_execution::ComputerInputCancellation,
) -> Result<(), DaemonError> {
    if desktop_pixel_width == 0
        || desktop_pixel_height == 0
        || x >= desktop_pixel_width
        || y >= desktop_pixel_height
    {
        return Err(room_computer_input_error(
            "environment_pointer_out_of_bounds",
        ));
    }
    let output = run_slice_screen_command_inner_with_cancellation(
        vec!["move".to_string(), x.to_string(), y.to_string()],
        None,
        Some(ROOM_COMPUTER_INPUT_TIMEOUT_MS),
        Some(cancellation),
    )
    .await?;
    if output.success {
        Ok(())
    } else {
        Err(room_computer_input_error(&format!(
            "slice pointer helper exited with status {}",
            output
                .status_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        )))
    }
}

pub(crate) async fn run_room_pointer_drag(
    from_x: u32,
    from_y: u32,
    to_x: u32,
    to_y: u32,
    button: crate::transport::room_browser_controller::RoomComputerPointerButton,
    desktop_pixel_width: u32,
    desktop_pixel_height: u32,
    cancellation: crate::runtime::computer_input_execution::ComputerInputCancellation,
) -> Result<(), DaemonError> {
    if desktop_pixel_width == 0
        || desktop_pixel_height == 0
        || from_x >= desktop_pixel_width
        || from_y >= desktop_pixel_height
        || to_x >= desktop_pixel_width
        || to_y >= desktop_pixel_height
    {
        return Err(room_computer_input_error(
            "environment_pointer_out_of_bounds",
        ));
    }
    let button = room_computer_pointer_button_arg(button);
    let output = run_slice_screen_command_inner_with_cancellation(
        vec![
            "pointer-drag".to_string(),
            from_x.to_string(),
            from_y.to_string(),
            to_x.to_string(),
            to_y.to_string(),
            button.to_string(),
        ],
        None,
        Some(ROOM_COMPUTER_INPUT_TIMEOUT_MS),
        Some(cancellation),
    )
    .await?;
    if output.success {
        Ok(())
    } else {
        Err(room_computer_input_error(&format!(
            "slice pointer helper exited with status {}",
            output
                .status_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        )))
    }
}

pub(crate) async fn run_room_pointer_scroll(
    x: u32,
    y: u32,
    horizontal_steps: i16,
    vertical_steps: i16,
    desktop_pixel_width: u32,
    desktop_pixel_height: u32,
    cancellation: crate::runtime::computer_input_execution::ComputerInputCancellation,
) -> Result<(), DaemonError> {
    if desktop_pixel_width == 0
        || desktop_pixel_height == 0
        || x >= desktop_pixel_width
        || y >= desktop_pixel_height
    {
        return Err(room_computer_input_error(
            "environment_pointer_out_of_bounds",
        ));
    }
    if (horizontal_steps == 0 && vertical_steps == 0)
        || horizontal_steps.unsigned_abs()
            > crate::transport::room_browser_controller::ROOM_COMPUTER_SCROLL_MAX_STEPS
        || vertical_steps.unsigned_abs()
            > crate::transport::room_browser_controller::ROOM_COMPUTER_SCROLL_MAX_STEPS
    {
        return Err(room_computer_input_error(
            "environment_invalid_scroll_steps",
        ));
    }
    let output = run_slice_screen_command_inner_with_cancellation(
        vec![
            "pointer-scroll".to_string(),
            x.to_string(),
            y.to_string(),
            horizontal_steps.to_string(),
            vertical_steps.to_string(),
        ],
        None,
        Some(ROOM_COMPUTER_INPUT_TIMEOUT_MS),
        Some(cancellation),
    )
    .await?;
    if output.success {
        Ok(())
    } else {
        Err(room_computer_input_error(&format!(
            "slice pointer helper exited with status {}",
            output
                .status_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        )))
    }
}

pub(crate) async fn run_room_keyboard_text(
    input: crate::transport::room_browser_controller::RoomComputerKeyboardInput,
    cancellation: crate::runtime::computer_input_execution::ComputerInputCancellation,
) -> Result<(), DaemonError> {
    if input.as_str().is_empty()
        || input.as_str().len()
            > crate::transport::room_browser_controller::ROOM_COMPUTER_KEYBOARD_TEXT_MAX_UTF8_BYTES
    {
        return Err(room_computer_input_error(
            "environment_invalid_keyboard_text",
        ));
    }
    let timeout_ms =
        crate::runtime::computer_input_action::keyboard_text_timeout_ms(input.as_str());
    let output = run_slice_screen_command_inner_with_cancellation(
        vec!["computer-type-stdin".to_string()],
        Some(input.into_zeroizing()),
        Some(timeout_ms),
        Some(cancellation),
    )
    .await?;
    if output.success {
        Ok(())
    } else {
        Err(room_computer_input_error(&format!(
            "slice computer keyboard helper exited with status {}",
            output
                .status_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        )))
    }
}

pub(crate) async fn run_room_keyboard_key(
    input: crate::transport::room_browser_controller::RoomComputerKeyboardInput,
    repeat: u16,
    cancellation: crate::runtime::computer_input_execution::ComputerInputCancellation,
) -> Result<(), DaemonError> {
    let key = input.as_str();
    if key.is_empty()
        || key.len()
            > crate::transport::room_browser_controller::ROOM_COMPUTER_KEYBOARD_KEY_MAX_UTF8_BYTES
        || key.starts_with('-')
        || !key.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(room_computer_input_error(
            "environment_invalid_keyboard_key",
        ));
    }
    if repeat == 0
        || repeat > crate::transport::room_browser_controller::ROOM_COMPUTER_KEYBOARD_KEY_MAX_REPEAT
    {
        return Err(room_computer_input_error(
            "environment_invalid_keyboard_repeat",
        ));
    }
    let output = run_slice_screen_command_inner_with_cancellation(
        vec!["computer-key-stdin".to_string(), repeat.to_string()],
        Some(input.into_zeroizing()),
        Some(ROOM_COMPUTER_INPUT_TIMEOUT_MS),
        Some(cancellation),
    )
    .await?;
    if output.success {
        Ok(())
    } else {
        Err(room_computer_input_error(&format!(
            "slice computer keyboard helper exited with status {}",
            output
                .status_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        )))
    }
}

pub(crate) async fn run_room_clipboard_write(
    text: crate::transport::room_browser_controller::RoomComputerClipboardText,
    cancellation: crate::runtime::computer_input_execution::ComputerInputCancellation,
) -> Result<(), DaemonError> {
    run_room_clipboard_write_inner(text, Some(cancellation)).await
}

async fn run_room_clipboard_write_inner(
    text: crate::transport::room_browser_controller::RoomComputerClipboardText,
    cancellation: Option<crate::runtime::computer_input_execution::ComputerInputCancellation>,
) -> Result<(), DaemonError> {
    if text.as_str().len()
        > crate::transport::room_browser_controller::ROOM_COMPUTER_CLIPBOARD_MAX_UTF8_BYTES
    {
        return Err(room_computer_input_error(
            "environment_invalid_clipboard_text",
        ));
    }
    let output = run_slice_screen_command_inner_with_cancellation(
        vec!["computer-clipboard-write-stdin".to_string()],
        Some(text.into_zeroizing()),
        Some(ROOM_COMPUTER_INPUT_TIMEOUT_MS),
        cancellation,
    )
    .await?;
    if output.success {
        Ok(())
    } else {
        Err(room_computer_input_error(&format!(
            "slice computer clipboard helper exited with status {}",
            output
                .status_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        )))
    }
}

pub(crate) async fn run_room_clipboard_read(
) -> Result<crate::transport::room_browser_controller::RoomComputerClipboardText, DaemonError> {
    let mut output = run_slice_screen_command_inner_exact_stdout(
        vec!["computer-clipboard-read".to_string()],
        Some(ROOM_COMPUTER_INPUT_TIMEOUT_MS),
    )
    .await?;
    if !output.success {
        zeroize::Zeroize::zeroize(&mut output.stdout);
        return Err(room_computer_input_error(&format!(
            "slice computer clipboard helper exited with status {}",
            output
                .status_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        )));
    }
    if output.stdout_truncated
        || output.stdout.len()
            > crate::transport::room_browser_controller::ROOM_COMPUTER_CLIPBOARD_MAX_UTF8_BYTES
    {
        zeroize::Zeroize::zeroize(&mut output.stdout);
        return Err(room_computer_input_error(
            "environment_invalid_clipboard_text",
        ));
    }
    Ok(
        crate::transport::room_browser_controller::RoomComputerClipboardText::new(std::mem::take(
            &mut *output.stdout,
        )),
    )
}

pub(crate) async fn run_room_secret_text_input(
    input: crate::transport::room_browser_controller::RoomComputerSecretInput,
    cancellation: crate::runtime::computer_input_execution::ComputerInputCancellation,
) -> Result<(), DaemonError> {
    let output = run_slice_screen_command_inner_with_cancellation(
        vec!["computer-secret-paste-stdin".to_string()],
        Some(input.into_zeroizing()),
        Some(ROOM_COMPUTER_INPUT_TIMEOUT_MS),
        Some(cancellation),
    )
    .await?;
    if output.success {
        Ok(())
    } else {
        Err(room_computer_input_error(&format!(
            "slice computer secret helper exited with status {}",
            output
                .status_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        )))
    }
}

pub(crate) async fn reset_room_computer_input() -> Result<(), DaemonError> {
    let output = run_slice_screen_command_inner(
        vec!["computer-input-reset".to_string()],
        None,
        Some(ROOM_COMPUTER_INPUT_TIMEOUT_MS),
    )
    .await?;
    if output.success {
        Ok(())
    } else {
        Err(room_computer_input_error(&format!(
            "slice computer input reset exited with status {}",
            output
                .status_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        )))
    }
}

fn computer_input_cancelled() -> DaemonError {
    DaemonError::BrowserControllerActionCancelled {
        controller_fenced: false,
    }
}

fn room_computer_input_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "environment.action.execute",
        message: message.to_string(),
    }
}

fn room_computer_pointer_button_arg(
    button: crate::transport::room_browser_controller::RoomComputerPointerButton,
) -> &'static str {
    match button {
        crate::transport::room_browser_controller::RoomComputerPointerButton::Left => "left",
        crate::transport::room_browser_controller::RoomComputerPointerButton::Middle => "middle",
        crate::transport::room_browser_controller::RoomComputerPointerButton::Right => "right",
    }
}

fn slice_screen_command_timeout_ms() -> u64 {
    std::env::var("CHARIOX_SLICE_SCREEN_TOOL_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_SLICE_SCREEN_COMMAND_TIMEOUT_MS)
}

fn read_child_output<R: std::io::Read>(
    mut reader: R,
) -> Result<(zeroize::Zeroizing<String>, bool), DaemonError> {
    let (stored, truncated) = read_child_output_bytes(&mut reader)?;
    Ok((
        zeroize::Zeroizing::new(String::from_utf8_lossy(&stored).trim().to_string()),
        truncated,
    ))
}

fn read_child_output_exact_utf8<R: std::io::Read>(
    mut reader: R,
) -> Result<(zeroize::Zeroizing<String>, bool), DaemonError> {
    let (mut stored, truncated) = read_child_output_bytes(&mut reader)?;
    let output = match String::from_utf8(std::mem::take(&mut *stored)) {
        Ok(output) => output,
        Err(error) => {
            let mut invalid = error.into_bytes();
            zeroize::Zeroize::zeroize(&mut invalid);
            return Err(DaemonError::LocalTransport {
                operation: "run_slice_screen_command",
                message: "slice screen stdout is not valid UTF-8".to_string(),
            });
        }
    };
    Ok((zeroize::Zeroizing::new(output), truncated))
}

fn read_child_output_bytes<R: std::io::Read>(
    mut reader: R,
) -> Result<(zeroize::Zeroizing<Vec<u8>>, bool), DaemonError> {
    let mut stored = zeroize::Zeroizing::new(Vec::new());
    let mut buffer = zeroize::Zeroizing::new([0_u8; 8192]);
    let mut truncated = false;
    loop {
        let read = reader
            .read(&mut buffer[..])
            .map_err(|error| DaemonError::LocalTransport {
                operation: "run_slice_screen_command",
                message: format!("failed to read slice screen output: {error}"),
            })?;
        if read == 0 {
            break;
        }
        let remaining = SLICE_SCREEN_COMMAND_OUTPUT_MAX_BYTES.saturating_sub(stored.len());
        if remaining > 0 {
            stored.extend_from_slice(&buffer[..read.min(remaining)]);
        }
        if read > remaining {
            truncated = true;
        }
    }
    Ok((stored, truncated))
}

fn slice_tool_payload(
    slice_id: &str,
    agent_id: &str,
    output: &SliceScreenCommandOutput,
) -> serde_json::Value {
    let mut payload = serde_json::Map::new();
    payload.insert(
        "slice_id".to_string(),
        serde_json::Value::String(slice_id.to_string()),
    );
    payload.insert(
        "agent_id".to_string(),
        serde_json::Value::String(agent_id.to_string()),
    );
    payload.insert(
        "status_code".to_string(),
        output
            .status_code
            .map(serde_json::Value::from)
            .unwrap_or(serde_json::Value::Null),
    );
    if !output.sensitive_output {
        payload.insert(
            "stdout".to_string(),
            serde_json::Value::String(output.stdout.as_str().to_string()),
        );
        payload.insert(
            "stderr".to_string(),
            serde_json::Value::String(output.stderr.as_str().to_string()),
        );
    }
    if output.stdout_truncated {
        payload.insert(
            "stdout_truncated".to_string(),
            serde_json::Value::Bool(true),
        );
    }
    if output.stderr_truncated {
        payload.insert(
            "stderr_truncated".to_string(),
            serde_json::Value::Bool(true),
        );
    }
    for line in output.stdout.lines() {
        if let Some((key, value)) = line.split_once('=') {
            let value = value.trim();
            match key {
                "available" => {
                    payload.insert(key.to_string(), serde_json::Value::Bool(value == "true"));
                }
                "display" | "screen" | "viewer" | "mode" | "missing" | "message" => {
                    payload.insert(
                        key.to_string(),
                        serde_json::Value::String(value.to_string()),
                    );
                }
                _ => {}
            }
        }
    }
    serde_json::Value::Object(payload)
}

fn slice_find_text_payload(
    slice_id: &str,
    agent_id: &str,
    output: &SliceScreenCommandOutput,
) -> serde_json::Value {
    let matches = output
        .stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(serde_json::Value::is_object)
        .collect::<Vec<_>>();
    let mut payload = slice_tool_payload(slice_id, agent_id, output);
    payload["match"] = matches.first().cloned().unwrap_or(serde_json::Value::Null);
    payload["match_count"] = serde_json::Value::from(matches.len());
    payload["matches"] = serde_json::Value::Array(matches);
    payload
}

fn secret_paste_payload(
    slice_id: &str,
    agent_id: &str,
    credential_id: &str,
    submitted: bool,
    output: &SliceScreenCommandOutput,
) -> serde_json::Value {
    let mut payload = serde_json::Map::new();
    payload.insert(
        "slice_id".to_string(),
        serde_json::Value::String(slice_id.to_string()),
    );
    payload.insert(
        "agent_id".to_string(),
        serde_json::Value::String(agent_id.to_string()),
    );
    payload.insert(
        "credential_id".to_string(),
        serde_json::Value::String(credential_id.to_string()),
    );
    payload.insert("submitted".to_string(), serde_json::Value::Bool(submitted));
    payload.insert(
        "status_code".to_string(),
        output
            .status_code
            .map(serde_json::Value::from)
            .unwrap_or(serde_json::Value::Null),
    );
    if output.stdout_truncated {
        payload.insert(
            "stdout_truncated".to_string(),
            serde_json::Value::Bool(true),
        );
    }
    if output.stderr_truncated {
        payload.insert(
            "stderr_truncated".to_string(),
            serde_json::Value::Bool(true),
        );
    }
    serde_json::Value::Object(payload)
}

fn slice_mouse_command_args(
    args: crate::transport::runtime_tools::SliceMouseArgs,
) -> Result<Vec<String>, DaemonError> {
    match args.action.as_str() {
        "move" => Ok(vec![
            "move".to_string(),
            required_i64(args.x, "x", "runtime_tool_slice_mouse")?.to_string(),
            required_i64(args.y, "y", "runtime_tool_slice_mouse")?.to_string(),
        ]),
        "click" => Ok(vec![
            "click".to_string(),
            required_i64(args.x, "x", "runtime_tool_slice_mouse")?.to_string(),
            required_i64(args.y, "y", "runtime_tool_slice_mouse")?.to_string(),
        ]),
        "double_click" => Ok(vec![
            "double-click".to_string(),
            required_i64(args.x, "x", "runtime_tool_slice_mouse")?.to_string(),
            required_i64(args.y, "y", "runtime_tool_slice_mouse")?.to_string(),
        ]),
        "scroll" => Ok(vec![
            "scroll".to_string(),
            args.amount.unwrap_or(1).to_string(),
        ]),
        "drag" => Ok(vec![
            "drag".to_string(),
            required_i64(args.x, "x", "runtime_tool_slice_mouse")?.to_string(),
            required_i64(args.y, "y", "runtime_tool_slice_mouse")?.to_string(),
            required_i64(args.to_x, "to_x", "runtime_tool_slice_mouse")?.to_string(),
            required_i64(args.to_y, "to_y", "runtime_tool_slice_mouse")?.to_string(),
        ]),
        other => Err(DaemonError::LocalTransport {
            operation: "runtime_tool_slice_mouse",
            message: format!("unsupported mouse action `{other}`"),
        }),
    }
}

fn slice_keyboard_command_args(
    args: crate::transport::runtime_tools::SliceKeyboardArgs,
) -> Result<Vec<String>, DaemonError> {
    match args.action.as_str() {
        "type" => Ok(vec![
            "type".to_string(),
            required_string(args.text, "text", "runtime_tool_slice_keyboard")?,
        ]),
        "key" => Ok(vec![
            "key".to_string(),
            required_string(args.key, "key", "runtime_tool_slice_keyboard")?,
        ]),
        other => Err(DaemonError::LocalTransport {
            operation: "runtime_tool_slice_keyboard",
            message: format!("unsupported keyboard action `{other}`"),
        }),
    }
}

fn required_i64(
    value: Option<i64>,
    field: &str,
    operation: &'static str,
) -> Result<i64, DaemonError> {
    value.ok_or_else(|| DaemonError::LocalTransport {
        operation,
        message: format!("missing required `{field}`"),
    })
}

fn required_string(
    value: Option<String>,
    field: &str,
    operation: &'static str,
) -> Result<String, DaemonError> {
    value
        .filter(|value| !value.is_empty())
        .ok_or_else(|| DaemonError::LocalTransport {
            operation,
            message: format!("missing required `{field}`"),
        })
}

fn browser_timeout_arg(timeout_ms: Option<u64>) -> String {
    timeout_ms.unwrap_or(10_000).clamp(100, 60_000).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slice_mouse_args_map_to_screen_script_commands() {
        let args = crate::transport::runtime_tools::SliceMouseArgs {
            action: "drag".to_string(),
            x: Some(10),
            y: Some(20),
            to_x: Some(30),
            to_y: Some(40),
            amount: None,
            horizontal_steps: None,
            button: None,
        };

        assert_eq!(
            slice_mouse_command_args(args).expect("drag args should map"),
            vec![
                "drag".to_string(),
                "10".to_string(),
                "20".to_string(),
                "30".to_string(),
                "40".to_string()
            ]
        );
    }

    #[test]
    fn slice_keyboard_args_require_text_for_type() {
        let args = crate::transport::runtime_tools::SliceKeyboardArgs {
            action: "type".to_string(),
            text: None,
            key: None,
            repeat: None,
        };

        assert!(slice_keyboard_command_args(args).is_err());
    }

    #[test]
    fn runtime_mcp_screenshot_reader_requires_a_bounded_png() {
        let mut png = super::super::super::room_screenshot::ROOM_SCREENSHOT_PNG_SIGNATURE.to_vec();
        png.extend_from_slice(b"image");
        assert_eq!(
            read_bounded_png(std::io::Cursor::new(&png), png.len()).expect("bounded PNG"),
            png
        );
        assert!(read_bounded_png(std::io::Cursor::new(&png), png.len() - 1)
            .expect_err("oversized PNG should fail")
            .contains("runtime MCP limit"));
        assert!(read_bounded_png(std::io::Cursor::new(b"not-a-png"), 32)
            .expect_err("non-PNG should fail")
            .contains("not a PNG"));
    }

    #[test]
    fn slice_tool_payload_reports_screen_availability() {
        let output = SliceScreenCommandOutput {
            success: false,
            status_code: Some(1),
            stdout: zeroize::Zeroizing::new(
                [
                    "display=:99",
                    "screen=1280x800",
                    "mode=headless",
                    "available=false",
                    "missing=xvfb,novnc",
                    "message=slice screen is unavailable; missing xvfb,novnc",
                ]
                .join("\n"),
            ),
            stderr: zeroize::Zeroizing::new(String::new()),
            stdout_truncated: false,
            stderr_truncated: false,
            sensitive_output: false,
        };

        let payload = slice_tool_payload("slice-1", "agent-1", &output);

        assert_eq!(payload["mode"], "headless");
        assert_eq!(payload["available"], false);
        assert_eq!(payload["missing"], "xvfb,novnc");
        assert_eq!(
            payload["message"],
            "slice screen is unavailable; missing xvfb,novnc"
        );
        assert_eq!(payload.get("viewer"), None);
    }

    #[test]
    fn secret_paste_payload_does_not_reflect_helper_output() {
        let output = SliceScreenCommandOutput {
            success: true,
            status_code: Some(0),
            stdout: zeroize::Zeroizing::new("typed super-secret-value".to_string()),
            stderr: zeroize::Zeroizing::new("debug super-secret-value".to_string()),
            stdout_truncated: false,
            stderr_truncated: false,
            sensitive_output: true,
        };

        let payload = secret_paste_payload("slice-1", "agent-1", "gmail-password", true, &output);
        let serialized = serde_json::to_string(&payload).expect("payload should serialize");

        assert!(serialized.contains("gmail-password"));
        assert!(serialized.contains("\"submitted\":true"));
        assert!(!serialized.contains("super-secret-value"));
        assert!(payload.get("stdout").is_none());
        assert!(!format!("{output:?}").contains("super-secret-value"));
        assert!(payload.get("stderr").is_none());
    }

    #[test]
    fn read_child_output_caps_stored_bytes_while_draining() {
        let input = vec![b'a'; SLICE_SCREEN_COMMAND_OUTPUT_MAX_BYTES + 1024];

        let (output, truncated) =
            read_child_output(std::io::Cursor::new(input)).expect("output should read");

        assert!(truncated);
        assert_eq!(output.len(), SLICE_SCREEN_COMMAND_OUTPUT_MAX_BYTES);
    }

    #[test]
    fn exact_child_output_preserves_whitespace_and_rejects_invalid_utf8() {
        let (output, truncated) = read_child_output_exact_utf8(std::io::Cursor::new(
            b"Clipboard Gr\xc3\xbc\xc3\x9fe \xe4\xb8\x96\xe7\x95\x8c\n\n".to_vec(),
        ))
        .expect("valid clipboard text should decode exactly");
        assert_eq!(
            output.as_str(),
            "Clipboard Gr\u{fc}\u{df}e \u{4e16}\u{754c}\n\n"
        );
        assert!(!truncated);

        let error = read_child_output_exact_utf8(std::io::Cursor::new(vec![0xff, 0xfe]))
            .expect_err("invalid clipboard UTF-8 should fail closed");
        assert_eq!(
            error.to_string(),
            "local transport `run_slice_screen_command` failed: slice screen stdout is not valid UTF-8"
        );
    }

    #[tokio::test]
    async fn slice_screen_command_times_out() {
        let _guard = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-slice-timeout-test-{}",
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::create_dir_all(&root);
        let script = root.join("slice-screen-timeout.sh");
        std::fs::write(&script, "#!/bin/sh\nsleep 1\n").expect("script should write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))
                .expect("script should be executable");
        }
        std::env::set_var("CHARIOX_SLICE_SCREEN_TOOL", &script);
        std::env::set_var("CHARIOX_SLICE_SCREEN_TOOL_TIMEOUT_MS", "50");

        let error = run_slice_screen_command(vec!["status".to_string()])
            .await
            .expect_err("sleeping helper should time out");

        assert!(error.to_string().contains("timed out"));
        std::env::remove_var("CHARIOX_SLICE_SCREEN_TOOL");
        std::env::remove_var("CHARIOX_SLICE_SCREEN_TOOL_TIMEOUT_MS");
        let _ = std::fs::remove_dir_all(&root);
    }
}
