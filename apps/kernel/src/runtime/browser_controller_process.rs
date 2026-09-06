use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use wait_timeout::ChildExt;

use super::browser_controller_action::{
    validate_browser_action_timeout, BrowserControllerActionResult, BrowserControllerDialogResult,
    BrowserDialogAction, BrowserLocatorAction,
};
use super::browser_controller_compatibility::{
    normalize_browser_navigation_url, BrowserCompatibilityWait,
    BrowserControllerCompatibilityWaitResult, BrowserControllerNavigationResult,
};
use super::browser_controller_event::{BrowserControllerEventBatch, MAX_BROWSER_EVENT_POLL_LIMIT};
use super::browser_controller_file_transfer::{
    BrowserControllerDownloadCancellationResult, BrowserControllerDownloadsResult,
    BrowserControllerUploadResult, BrowserDownloadCancellation, BrowserUploadFiles,
};
use super::browser_controller_history::{BrowserControllerHistoryResult, BrowserHistoryAction};
use super::browser_controller_permission::{
    BrowserControllerPermissionResult, BrowserPermissionName, BrowserPermissionSetting,
};
use super::browser_controller_snapshot::BrowserControllerStructuredSnapshot;
use super::browser_controller_tab::{BrowserControllerTabResult, BrowserTabAction};
use crate::session::CanonicalViewport;

mod cancellation;
mod configuration_cancellation;
pub(crate) use configuration_cancellation::BrowserConfiguration;
#[cfg(test)]
mod upload_cancellation_tests;

const DEFAULT_CONTROLLER_COMMAND_TIMEOUT_MS: u64 = 10_000;
const CONTROLLER_SCRIPT_ENV: &str = "CHARIOX_BROWSER_CONTROLLER_SCRIPT";
const CONTROLLER_NODE_ENV: &str = "CHARIOX_BROWSER_CONTROLLER_NODE";
const CONTROLLER_COMMAND_TIMEOUT_ENV: &str = "CHARIOX_BROWSER_CONTROLLER_COMMAND_TIMEOUT_MS";
pub(crate) const CONTROLLER_RESTARTED_BEFORE_OPERATION: &str =
    "browser controller restarted before the operation; reconcile and retry with fresh references";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BrowserControllerProcessState {
    Stopped,
    Starting,
    Ready,
    Unhealthy,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserControllerProcessHealth {
    pub(crate) state: BrowserControllerProcessState,
    pub(crate) process_id: Option<u32>,
    pub(crate) diagnostic_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BrowserControllerProcessSnapshot {
    pub(crate) state: BrowserControllerProcessState,
    pub(crate) process_id: Option<u32>,
    pub(crate) diagnostic_code: Option<String>,
    pub(crate) runtime_generation: u64,
    pub(crate) restart_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BrowserControllerReconciliation {
    pub(crate) process: BrowserControllerProcessSnapshot,
    pub(crate) browser: BrowserControllerBrowserSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BrowserControllerBrowserSnapshot {
    pub(crate) browser_generation: u64,
    #[serde(default)]
    pub(crate) event_cursor: u64,
    pub(crate) tabs: Vec<BrowserControllerTabSnapshot>,
    pub(crate) focused_target_id: Option<String>,
    viewport: BrowserControllerViewport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BrowserControllerTabSnapshot {
    pub(crate) target_id: String,
    pub(crate) document_id: String,
    pub(crate) url: String,
    pub(crate) title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct BrowserControllerViewport {
    css_width: u32,
    css_height: u32,
    device_scale_factor: u32,
    desktop_pixel_width: u32,
    desktop_pixel_height: u32,
}

pub(crate) trait BrowserControllerProcessBackend {
    fn health(&mut self) -> Result<BrowserControllerProcessHealth, String>;
    fn start(&mut self) -> Result<BrowserControllerProcessHealth, String>;
    fn stop(&mut self) -> Result<(), String>;
    fn reconcile_browser(
        &mut self,
        _viewport: &CanonicalViewport,
    ) -> Result<BrowserControllerBrowserSnapshot, String> {
        Err("browser controller backend does not support browser reconciliation".to_string())
    }
    fn capture_browser_snapshot(
        &mut self,
        _target_id: &str,
        _document_id: &str,
    ) -> Result<BrowserControllerStructuredSnapshot, String> {
        Err("browser controller backend does not support structured snapshots".to_string())
    }
    fn manage_browser_tab(
        &mut self,
        _target_id: &str,
        _document_id: &str,
        _action: BrowserTabAction,
    ) -> Result<BrowserControllerTabResult, String> {
        Err("browser controller backend does not support tab lifecycle operations".to_string())
    }
    fn navigate_browser_history(
        &mut self,
        _target_id: &str,
        _document_id: &str,
        _action: BrowserHistoryAction,
    ) -> Result<BrowserControllerHistoryResult, String> {
        Err("browser controller backend does not support history navigation".to_string())
    }
    fn perform_browser_action(
        &mut self,
        _target_id: &str,
        _document_id: &str,
        _node_ref: &str,
        _action: &BrowserLocatorAction,
        _timeout_ms: u64,
    ) -> Result<BrowserControllerActionResult, String> {
        Err("browser controller backend does not support locator actions".to_string())
    }
    fn navigate_browser(
        &mut self,
        _target_id: &str,
        _document_id: &str,
        _url: &str,
    ) -> Result<BrowserControllerNavigationResult, String> {
        Err("browser controller backend does not support navigation".to_string())
    }
    fn wait_for_browser(
        &mut self,
        _target_id: &str,
        _document_id: &str,
        _wait: &BrowserCompatibilityWait,
        _timeout_ms: u64,
    ) -> Result<BrowserControllerCompatibilityWaitResult, String> {
        Err("browser controller backend does not support compatibility waits".to_string())
    }
    fn handle_browser_dialog(
        &mut self,
        _target_id: &str,
        _document_id: &str,
        _action: &BrowserDialogAction,
    ) -> Result<BrowserControllerDialogResult, String> {
        Err("browser controller backend does not support dialogs".to_string())
    }
    fn configure_browser_downloads(
        &mut self,
        _target_id: &str,
        _document_id: &str,
    ) -> Result<BrowserControllerDownloadsResult, String> {
        Err("browser controller backend does not support downloads".to_string())
    }
    fn cancel_browser_download(
        &mut self,
        _cancellation: &BrowserDownloadCancellation,
    ) -> Result<BrowserControllerDownloadCancellationResult, String> {
        Err("browser controller backend does not support download cancellation".to_string())
    }
    fn upload_browser_files(
        &mut self,
        _target_id: &str,
        _document_id: &str,
        _node_ref: &str,
        _files: &BrowserUploadFiles,
    ) -> Result<BrowserControllerUploadResult, String> {
        Err("browser controller backend does not support uploads".to_string())
    }
    fn set_browser_permission(
        &mut self,
        _target_id: &str,
        _document_id: &str,
        _permission: BrowserPermissionName,
        _setting: BrowserPermissionSetting,
    ) -> Result<BrowserControllerPermissionResult, String> {
        Err("browser controller backend does not support permissions".to_string())
    }
    fn poll_browser_events(
        &mut self,
        _browser_generation: u64,
        _cursor: u64,
        _limit: u16,
    ) -> Result<BrowserControllerEventBatch, String> {
        Err("browser controller backend does not support event polling".to_string())
    }
}

pub(crate) struct BrowserControllerProcessStdioBackend {
    command: PathBuf,
    args: Vec<String>,
    timeout: Duration,
    process: Option<BrowserControllerChild>,
    next_request_id: u64,
    action_cancellation: Option<Arc<cancellation::CancellationSignal>>,
}

impl BrowserControllerProcessStdioBackend {
    pub(crate) fn new(command: impl Into<PathBuf>, args: Vec<String>, timeout: Duration) -> Self {
        Self {
            command: command.into(),
            args,
            timeout,
            process: None,
            next_request_id: 1,
            action_cancellation: None,
        }
    }

    fn from_script(script_path: impl Into<PathBuf>, timeout: Duration) -> Self {
        let command = std::env::var_os(CONTROLLER_NODE_ENV)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "node".into());
        Self::new(
            PathBuf::from(command),
            vec![
                script_path.into().display().to_string(),
                "stdio".to_string(),
            ],
            timeout,
        )
    }

    fn spawn(&mut self) -> Result<(), String> {
        let mut command = Command::new(&self.command);
        command
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn().map_err(|error| {
            format!(
                "failed to spawn browser controller `{}`: {error}",
                self.command.display()
            )
        })?;
        let stdin = child.stdin.take().ok_or_else(|| {
            kill_child(&mut child);
            "browser controller did not expose stdin".to_string()
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            kill_child(&mut child);
            "browser controller did not expose stdout".to_string()
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            kill_child(&mut child);
            "browser controller did not expose stderr".to_string()
        })?;
        let (responses_tx, responses) = mpsc::channel();
        if let Err(error) = std::thread::Builder::new()
            .name("chariox-browser-controller-reader".to_string())
            .spawn(move || read_controller_responses(stdout, responses_tx))
        {
            kill_child(&mut child);
            return Err(format!(
                "failed to start browser controller response reader: {error}"
            ));
        }
        let _ = std::thread::Builder::new()
            .name("chariox-browser-controller-stderr".to_string())
            .spawn(move || {
                for line in BufReader::new(stderr).lines() {
                    if line.is_err() {
                        break;
                    }
                }
            });
        self.process = Some(BrowserControllerChild {
            child,
            stdin,
            responses,
        });
        Ok(())
    }

    fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<BrowserControllerRpcResponse, String> {
        // Locator actions have their own bounded actionability deadline inside
        // the controller. The stdio deadline starts outside that operation, so
        // add the declared action budget instead of timing the transport out
        // while the controller is still performing or cleaning up the action.
        let timeout = if method == "browser.action" {
            self.timeout.saturating_add(Duration::from_millis(
                params
                    .get("timeout_ms")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
            ))
        } else {
            self.timeout
        };
        let cancellation = matches!(
            method,
            "browser.action"
                | "browser.upload"
                | "browser.downloads.configure"
                | "browser.permission"
        )
        .then(|| self.action_cancellation.clone())
        .flatten();
        if cancellation
            .as_ref()
            .is_some_and(|signal| signal.requested())
        {
            cancellation.as_ref().unwrap().confirm_stop();
            return Err("browser action cancelled before dispatch".into());
        }
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        let process = self
            .process
            .as_mut()
            .ok_or_else(|| "browser controller is not running".to_string())?;
        serde_json::to_writer(
            &mut process.stdin,
            &serde_json::json!({ "id": request_id, "method": method, "params": params }),
        )
        .map_err(|error| format!("failed to encode browser controller request: {error}"))?;
        process
            .stdin
            .write_all(b"\n")
            .and_then(|()| process.stdin.flush())
            .map_err(|error| format!("failed to send browser controller `{method}`: {error}"))?;
        let started = Instant::now();
        let mut cancellation_sent = false;
        loop {
            if !cancellation_sent
                && cancellation
                    .as_ref()
                    .is_some_and(|signal| signal.requested())
            {
                let cancel_id = self.next_request_id;
                self.next_request_id = self.next_request_id.saturating_add(1);
                serde_json::to_writer(
                    &mut process.stdin,
                    &serde_json::json!({
                        "id":cancel_id,"method":"browser.cancel","params":{"request_id":request_id}
                    }),
                )
                .map_err(|error| error.to_string())?;
                process
                    .stdin
                    .write_all(b"\n")
                    .and_then(|()| process.stdin.flush())
                    .map_err(|error| error.to_string())?;
                cancellation_sent = true;
            }
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                if let Some(signal) = cancellation.as_ref().filter(|signal| signal.requested()) {
                    // A timeout is not proof that physical input stopped. Kill
                    // and reap the only process capable of sending more input
                    // before confirming cancellation to the home kernel.
                    kill_child(&mut process.child);
                    signal.confirm_fence();
                    return Ok(BrowserControllerRpcResponse {
                        id: Some(request_id),
                        ok: false,
                        result: None,
                        error: Some(BrowserControllerRpcError {
                            code: "browser_action_cancelled".to_string(),
                            message: "browser controller was fenced after cancellation timed out"
                                .to_string(),
                        }),
                    });
                }
                return Err(format!(
                    "browser controller `{method}` timed out after {}ms",
                    timeout.as_millis()
                ));
            }
            let poll = if cancellation.is_some() {
                remaining.min(Duration::from_millis(20))
            } else {
                remaining
            };
            let response = match process.responses.recv_timeout(poll) {
                Ok(response) => response?,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(format!("browser controller exited during `{method}`"))
                }
            };
            if response.id == Some(request_id) {
                if !response.ok
                    && response
                        .error
                        .as_ref()
                        .is_some_and(|error| error.code == "browser_action_cancelled")
                {
                    if let Some(signal) = &cancellation {
                        signal.confirm_stop();
                    }
                }
                return Ok(response);
            }
        }
    }

    fn health_request(&mut self) -> Result<BrowserControllerProcessHealth, String> {
        let process_id = self
            .process
            .as_ref()
            .map(|process| process.child.id())
            .ok_or_else(|| "browser controller is not running".to_string())?;
        let response = self.request("health", serde_json::json!({}))?;
        let health = response.into_result::<BrowserControllerCommandHealth>("health")?;
        let health = health.into_health("health")?;
        if health.process_id != Some(process_id) {
            return Err(format!(
                "browser controller health reported process {:?}, expected {process_id}",
                health.process_id
            ));
        }
        Ok(health)
    }

    fn take_exited_process(&mut self) -> Result<Option<u32>, String> {
        let Some(process) = self.process.as_mut() else {
            return Ok(None);
        };
        let process_id = process.child.id();
        let status = process
            .child
            .try_wait()
            .map_err(|error| format!("failed to inspect browser controller: {error}"))?;
        if status.is_some() {
            self.process.take();
            return Ok(Some(process_id));
        }
        Ok(None)
    }
}

impl BrowserControllerRpcResponse {
    fn into_result<T: DeserializeOwned>(self, method: &str) -> Result<T, String> {
        if !self.ok {
            let error = self.error.unwrap_or(BrowserControllerRpcError {
                code: "controller_error".to_string(),
                message: "browser controller returned an unspecified error".to_string(),
            });
            return Err(format!(
                "browser controller `{method}` failed with {}: {}",
                error.code, error.message
            ));
        }
        let result = self
            .result
            .ok_or_else(|| format!("browser controller `{method}` omitted its result"))?;
        serde_json::from_value(result).map_err(|error| {
            format!("browser controller `{method}` returned invalid result: {error}")
        })
    }
}

impl BrowserControllerCommandHealth {
    fn into_health(self, method: &str) -> Result<BrowserControllerProcessHealth, String> {
        let state = match self.state.as_str() {
            "stopped" => BrowserControllerProcessState::Stopped,
            "starting" => BrowserControllerProcessState::Starting,
            "ready" => BrowserControllerProcessState::Ready,
            "unhealthy" => BrowserControllerProcessState::Unhealthy,
            "failed" => BrowserControllerProcessState::Failed,
            state => {
                return Err(format!(
                    "browser controller `{method}` returned unknown state `{state}`"
                ));
            }
        };
        Ok(BrowserControllerProcessHealth {
            state,
            process_id: self.process_id,
            diagnostic_code: self.diagnostic_code,
        })
    }
}

impl BrowserControllerProcessBackend for BrowserControllerProcessStdioBackend {
    fn health(&mut self) -> Result<BrowserControllerProcessHealth, String> {
        if let Some(process_id) = self.take_exited_process()? {
            return Ok(BrowserControllerProcessHealth {
                state: BrowserControllerProcessState::Unhealthy,
                process_id: Some(process_id),
                diagnostic_code: Some("process_exited".to_string()),
            });
        }
        if self.process.is_none() {
            return Ok(BrowserControllerProcessHealth {
                state: BrowserControllerProcessState::Stopped,
                process_id: None,
                diagnostic_code: None,
            });
        }
        self.health_request()
    }

    fn start(&mut self) -> Result<BrowserControllerProcessHealth, String> {
        if self.process.is_some() {
            self.stop()?;
        }
        self.spawn()?;
        match self.health_request() {
            Ok(health) => Ok(health),
            Err(error) => {
                if let Some(mut process) = self.process.take() {
                    kill_child(&mut process.child);
                }
                Err(error)
            }
        }
    }

    fn stop(&mut self) -> Result<(), String> {
        if self.process.is_none() {
            return Ok(());
        }
        let shutdown_requested = self.request("shutdown", serde_json::json!({})).is_ok();
        if let Some(mut process) = self.process.take() {
            if shutdown_requested {
                terminate_child(&mut process.child, self.timeout);
            } else {
                kill_child(&mut process.child);
            }
        }
        Ok(())
    }

    fn reconcile_browser(
        &mut self,
        viewport: &CanonicalViewport,
    ) -> Result<BrowserControllerBrowserSnapshot, String> {
        let response = self.request(
            "browser.reconcile",
            serde_json::json!({
                "viewport": {
                    "css_width": viewport.css_width,
                    "css_height": viewport.css_height,
                    "device_scale_factor": viewport.device_scale_factor,
                    "desktop_pixel_width": viewport.desktop_pixel_width,
                    "desktop_pixel_height": viewport.desktop_pixel_height,
                }
            }),
        )?;
        let snapshot =
            response.into_result::<BrowserControllerBrowserSnapshot>("browser.reconcile")?;
        snapshot.validate(viewport)?;
        Ok(snapshot)
    }

    fn capture_browser_snapshot(
        &mut self,
        target_id: &str,
        document_id: &str,
    ) -> Result<BrowserControllerStructuredSnapshot, String> {
        let response = self.request(
            "browser.snapshot",
            serde_json::json!({
                "target_id": target_id,
                "document_id": document_id,
            }),
        )?;
        let snapshot =
            response.into_result::<BrowserControllerStructuredSnapshot>("browser.snapshot")?;
        snapshot.validate(target_id, document_id)?;
        Ok(snapshot)
    }

    fn manage_browser_tab(
        &mut self,
        target_id: &str,
        document_id: &str,
        action: BrowserTabAction,
    ) -> Result<BrowserControllerTabResult, String> {
        let response = self.request(
            "browser.tab",
            serde_json::json!({
                "target_id": target_id,
                "document_id": document_id,
                "action": action.as_str(),
            }),
        )?;
        let result = response.into_result::<BrowserControllerTabResult>("browser.tab")?;
        result.validate(target_id, document_id, action)?;
        Ok(result)
    }

    fn navigate_browser_history(
        &mut self,
        target_id: &str,
        document_id: &str,
        action: BrowserHistoryAction,
    ) -> Result<BrowserControllerHistoryResult, String> {
        let response = self.request(
            "browser.history",
            serde_json::json!({
                "target_id": target_id,
                "document_id": document_id,
                "action": action.as_str(),
            }),
        )?;
        let result = response.into_result::<BrowserControllerHistoryResult>("browser.history")?;
        result.validate(target_id, action)?;
        Ok(result)
    }

    fn perform_browser_action(
        &mut self,
        target_id: &str,
        document_id: &str,
        node_ref: &str,
        action: &BrowserLocatorAction,
        timeout_ms: u64,
    ) -> Result<BrowserControllerActionResult, String> {
        action.validate()?;
        validate_browser_action_timeout(timeout_ms)?;
        let response = self.request(
            "browser.action",
            serde_json::json!({
                "target_id": target_id,
                "document_id": document_id,
                "node_ref": node_ref,
                "action": action.controller_value(),
                "timeout_ms": timeout_ms,
            }),
        )?;
        let result = response.into_result::<BrowserControllerActionResult>("browser.action")?;
        result.validate(target_id, document_id, action.kind())?;
        Ok(result)
    }

    fn navigate_browser(
        &mut self,
        target_id: &str,
        document_id: &str,
        url: &str,
    ) -> Result<BrowserControllerNavigationResult, String> {
        let url = normalize_browser_navigation_url(url)?;
        let response = self.request(
            "browser.navigate",
            serde_json::json!({
                "target_id": target_id,
                "document_id": document_id,
                "url": url,
            }),
        )?;
        let result =
            response.into_result::<BrowserControllerNavigationResult>("browser.navigate")?;
        result.validate(target_id, &url)?;
        Ok(result)
    }

    fn wait_for_browser(
        &mut self,
        target_id: &str,
        document_id: &str,
        wait: &BrowserCompatibilityWait,
        timeout_ms: u64,
    ) -> Result<BrowserControllerCompatibilityWaitResult, String> {
        wait.validate(timeout_ms)?;
        let response = self.request(
            "browser.wait",
            serde_json::json!({
                "target_id": target_id,
                "document_id": document_id,
                "kind": wait.kind(),
                "selector": wait.selector(),
                "timeout_ms": timeout_ms,
            }),
        )?;
        let result =
            response.into_result::<BrowserControllerCompatibilityWaitResult>("browser.wait")?;
        result.validate(target_id, document_id, wait)?;
        Ok(result)
    }

    fn handle_browser_dialog(
        &mut self,
        target_id: &str,
        document_id: &str,
        action: &BrowserDialogAction,
    ) -> Result<BrowserControllerDialogResult, String> {
        action.validate()?;
        let response = self.request(
            "browser.dialog",
            serde_json::json!({
                "target_id": target_id,
                "document_id": document_id,
                "action": action.kind(),
                "prompt_text": action.prompt_text(),
            }),
        )?;
        let result = response.into_result::<BrowserControllerDialogResult>("browser.dialog")?;
        result.validate(target_id, document_id, action)?;
        Ok(result)
    }

    fn configure_browser_downloads(
        &mut self,
        target_id: &str,
        document_id: &str,
    ) -> Result<BrowserControllerDownloadsResult, String> {
        let response = self.request(
            "browser.downloads.configure",
            serde_json::json!({
                "target_id": target_id,
                "document_id": document_id,
            }),
        )?;
        let result = response
            .into_result::<BrowserControllerDownloadsResult>("browser.downloads.configure")?;
        result.validate(target_id, document_id)?;
        Ok(result)
    }

    fn cancel_browser_download(
        &mut self,
        cancellation: &BrowserDownloadCancellation,
    ) -> Result<BrowserControllerDownloadCancellationResult, String> {
        let response = self.request(
            "browser.downloads.cancel",
            serde_json::to_value(cancellation).map_err(|error| error.to_string())?,
        )?;
        let result = response.into_result::<BrowserControllerDownloadCancellationResult>(
            "browser.downloads.cancel",
        )?;
        result.validate(cancellation)?;
        Ok(result)
    }

    fn upload_browser_files(
        &mut self,
        target_id: &str,
        document_id: &str,
        node_ref: &str,
        files: &BrowserUploadFiles,
    ) -> Result<BrowserControllerUploadResult, String> {
        let controller_paths = files.controller_paths();
        let response = self.request(
            "browser.upload",
            serde_json::json!({
                "target_id": target_id,
                "document_id": document_id,
                "node_ref": node_ref,
                "file_paths": controller_paths,
            }),
        )?;
        let result = response.into_result::<BrowserControllerUploadResult>("browser.upload")?;
        result.validate(target_id, document_id, controller_paths.len())?;
        Ok(result)
    }

    fn set_browser_permission(
        &mut self,
        target_id: &str,
        document_id: &str,
        permission: BrowserPermissionName,
        setting: BrowserPermissionSetting,
    ) -> Result<BrowserControllerPermissionResult, String> {
        let response = self.request(
            "browser.permission",
            serde_json::json!({
                "target_id": target_id,
                "document_id": document_id,
                "permission": permission.as_str(),
                "setting": setting.as_str(),
            }),
        )?;
        let result =
            response.into_result::<BrowserControllerPermissionResult>("browser.permission")?;
        result.validate(target_id, document_id, permission, setting)?;
        Ok(result)
    }

    fn poll_browser_events(
        &mut self,
        browser_generation: u64,
        cursor: u64,
        limit: u16,
    ) -> Result<BrowserControllerEventBatch, String> {
        if browser_generation == 0 {
            return Err("browser event generation must be positive".to_string());
        }
        if limit == 0 || limit > MAX_BROWSER_EVENT_POLL_LIMIT {
            return Err(format!(
                "browser event limit must be between 1 and {MAX_BROWSER_EVENT_POLL_LIMIT}"
            ));
        }
        let response = self.request(
            "browser.events.poll",
            serde_json::json!({
                "browser_generation": browser_generation,
                "cursor": cursor,
                "limit": limit,
            }),
        )?;
        let result = response.into_result::<BrowserControllerEventBatch>("browser.events.poll")?;
        result.validate(browser_generation, cursor, limit)?;
        Ok(result)
    }
}

impl Drop for BrowserControllerProcessStdioBackend {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

struct BrowserControllerChild {
    child: Child,
    stdin: ChildStdin,
    responses: mpsc::Receiver<Result<BrowserControllerRpcResponse, String>>,
}

#[derive(Deserialize)]
struct BrowserControllerCommandHealth {
    state: String,
    process_id: Option<u32>,
    diagnostic_code: Option<String>,
}

#[derive(Deserialize)]
struct BrowserControllerRpcResponse {
    id: Option<u64>,
    ok: bool,
    result: Option<serde_json::Value>,
    error: Option<BrowserControllerRpcError>,
}

impl BrowserControllerBrowserSnapshot {
    fn validate(&self, expected_viewport: &CanonicalViewport) -> Result<(), String> {
        if self.browser_generation == 0 {
            return Err("browser controller returned zero browser generation".to_string());
        }
        if self.viewport.css_width != expected_viewport.css_width
            || self.viewport.css_height != expected_viewport.css_height
            || self.viewport.device_scale_factor != expected_viewport.device_scale_factor
            || self.viewport.desktop_pixel_width != expected_viewport.desktop_pixel_width
            || self.viewport.desktop_pixel_height != expected_viewport.desktop_pixel_height
        {
            return Err("browser controller did not apply the canonical viewport".to_string());
        }
        let mut target_ids = BTreeSet::new();
        for tab in &self.tabs {
            if tab.target_id.is_empty() || tab.document_id.is_empty() {
                return Err(
                    "browser controller returned an empty target or document identity".to_string(),
                );
            }
            if !target_ids.insert(tab.target_id.as_str()) {
                return Err(format!(
                    "browser controller returned duplicate target `{}`",
                    tab.target_id
                ));
            }
        }
        if let Some(focused_target_id) = &self.focused_target_id {
            if !target_ids.contains(focused_target_id.as_str()) {
                return Err(format!(
                    "browser controller focused unknown target `{focused_target_id}`"
                ));
            }
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct BrowserControllerRpcError {
    code: String,
    message: String,
}

fn read_controller_responses(
    stdout: ChildStdout,
    responses: mpsc::Sender<Result<BrowserControllerRpcResponse, String>>,
) {
    for line in BufReader::new(stdout).lines() {
        let response = line
            .map_err(|error| format!("failed to read browser controller response: {error}"))
            .and_then(|line| {
                serde_json::from_str::<BrowserControllerRpcResponse>(&line)
                    .map_err(|error| format!("browser controller returned invalid JSON: {error}"))
            });
        if responses.send(response).is_err() {
            return;
        }
    }
}

fn terminate_child(child: &mut Child, timeout: Duration) {
    if child.wait_timeout(timeout).ok().flatten().is_some() {
        return;
    }
    kill_child(child);
}

fn kill_child(child: &mut Child) {
    #[cfg(unix)]
    {
        if let Ok(pid) = i32::try_from(child.id()) {
            let _ = unsafe { libc::kill(-pid, libc::SIGKILL) };
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

pub(crate) struct BrowserControllerProcessSupervisor<B> {
    backend: B,
    snapshot: BrowserControllerProcessSnapshot,
    recovery_pending: bool,
}

type StdioOwnership = BrowserControllerProcessOwnership<BrowserControllerProcessStdioBackend>;

pub(crate) struct BrowserControllerProcessOwnership<B> {
    supervisor: BrowserControllerProcessSupervisor<B>,
    // One backend addresses one physical browser/profile. Stopping the
    // controller does not reset that profile or make it safe for another Room.
    owner_session_id: Option<String>,
    leased: bool,
}

impl<B: BrowserControllerProcessBackend> BrowserControllerProcessOwnership<B> {
    pub(crate) fn new(backend: B) -> Self {
        Self {
            supervisor: BrowserControllerProcessSupervisor::new(backend),
            owner_session_id: None,
            leased: false,
        }
    }

    pub(crate) fn acquire(
        &mut self,
        session_id: &str,
    ) -> Result<BrowserControllerProcessSnapshot, String> {
        if session_id.trim().is_empty() {
            return Err("browser controller requires a Room identity".to_string());
        }
        if let Some(owner) = &self.owner_session_id {
            if owner != session_id {
                return Err("browser controller is bound to another Room".to_string());
            }
        } else {
            // Reserve before startup: a failed health check can still leave a
            // browser/profile behind, so it must not permit reassignment.
            self.owner_session_id = Some(session_id.to_string());
        }
        let snapshot = self.supervisor.ensure_started()?.clone();
        self.leased = true;
        Ok(snapshot)
    }

    pub(crate) fn release(
        &mut self,
        session_id: &str,
    ) -> Result<BrowserControllerProcessSnapshot, String> {
        if self.owner_session_id.as_deref() == Some(session_id) {
            self.supervisor.stop()?;
            self.leased = false;
        }
        Ok(self.supervisor.snapshot().clone())
    }

    pub(crate) fn reconcile_browser(
        &mut self,
        session_id: &str,
        viewport: &CanonicalViewport,
    ) -> Result<BrowserControllerReconciliation, String> {
        self.require_lease(session_id)?;
        self.supervisor.reconcile_browser(viewport)
    }

    pub(crate) fn capture_browser_snapshot(
        &mut self,
        session_id: &str,
        target_id: &str,
        document_id: &str,
    ) -> Result<BrowserControllerStructuredSnapshot, String> {
        self.require_lease(session_id)?;
        self.supervisor
            .capture_browser_snapshot(target_id, document_id)
    }

    pub(crate) fn manage_browser_tab(
        &mut self,
        session_id: &str,
        target_id: &str,
        document_id: &str,
        action: BrowserTabAction,
    ) -> Result<BrowserControllerTabResult, String> {
        self.require_lease(session_id)?;
        self.supervisor
            .manage_browser_tab(target_id, document_id, action)
    }

    pub(crate) fn navigate_browser_history(
        &mut self,
        session_id: &str,
        target_id: &str,
        document_id: &str,
        action: BrowserHistoryAction,
    ) -> Result<BrowserControllerHistoryResult, String> {
        self.require_lease(session_id)?;
        self.supervisor
            .navigate_browser_history(target_id, document_id, action)
    }

    pub(crate) fn perform_browser_action(
        &mut self,
        session_id: &str,
        target_id: &str,
        document_id: &str,
        node_ref: &str,
        action: &BrowserLocatorAction,
        timeout_ms: u64,
    ) -> Result<BrowserControllerActionResult, String> {
        self.require_lease(session_id)?;
        self.supervisor
            .perform_browser_action(target_id, document_id, node_ref, action, timeout_ms)
    }

    pub(crate) fn navigate_browser(
        &mut self,
        session_id: &str,
        target_id: &str,
        document_id: &str,
        url: &str,
    ) -> Result<BrowserControllerNavigationResult, String> {
        self.require_lease(session_id)?;
        self.supervisor
            .navigate_browser(target_id, document_id, url)
    }

    pub(crate) fn wait_for_browser(
        &mut self,
        session_id: &str,
        target_id: &str,
        document_id: &str,
        wait: &BrowserCompatibilityWait,
        timeout_ms: u64,
    ) -> Result<BrowserControllerCompatibilityWaitResult, String> {
        self.require_lease(session_id)?;
        self.supervisor
            .wait_for_browser(target_id, document_id, wait, timeout_ms)
    }

    pub(crate) fn handle_browser_dialog(
        &mut self,
        session_id: &str,
        target_id: &str,
        document_id: &str,
        action: &BrowserDialogAction,
    ) -> Result<BrowserControllerDialogResult, String> {
        self.require_lease(session_id)?;
        self.supervisor
            .handle_browser_dialog(target_id, document_id, action)
    }

    pub(crate) fn configure_browser_downloads(
        &mut self,
        session_id: &str,
        target_id: &str,
        document_id: &str,
    ) -> Result<BrowserControllerDownloadsResult, String> {
        self.require_lease(session_id)?;
        self.supervisor
            .configure_browser_downloads(target_id, document_id)
    }

    pub(crate) fn cancel_browser_download(
        &mut self,
        session_id: &str,
        cancellation: &BrowserDownloadCancellation,
    ) -> Result<BrowserControllerDownloadCancellationResult, String> {
        self.require_lease(session_id)?;
        self.supervisor.cancel_browser_download(cancellation)
    }

    pub(crate) fn upload_browser_files(
        &mut self,
        session_id: &str,
        target_id: &str,
        document_id: &str,
        node_ref: &str,
        files: &BrowserUploadFiles,
    ) -> Result<BrowserControllerUploadResult, String> {
        self.require_lease(session_id)?;
        self.supervisor
            .upload_browser_files(target_id, document_id, node_ref, files)
    }

    pub(crate) fn set_browser_permission(
        &mut self,
        session_id: &str,
        target_id: &str,
        document_id: &str,
        permission: BrowserPermissionName,
        setting: BrowserPermissionSetting,
    ) -> Result<BrowserControllerPermissionResult, String> {
        self.require_lease(session_id)?;
        self.supervisor
            .set_browser_permission(target_id, document_id, permission, setting)
    }

    pub(crate) fn poll_browser_events(
        &mut self,
        session_id: &str,
        browser_generation: u64,
        cursor: u64,
        limit: u16,
    ) -> Result<BrowserControllerEventBatch, String> {
        self.require_lease(session_id)?;
        self.supervisor
            .poll_browser_events(browser_generation, cursor, limit)
    }

    fn require_lease(&self, session_id: &str) -> Result<(), String> {
        if !self.leased || self.owner_session_id.as_deref() != Some(session_id) {
            return Err(format!(
                "browser controller is not leased by Room {session_id}"
            ));
        }
        Ok(())
    }

    pub(crate) fn shutdown(&mut self) -> Result<BrowserControllerProcessSnapshot, String> {
        let snapshot = self.supervisor.stop()?.clone();
        self.leased = false;
        Ok(snapshot)
    }
}

#[derive(Clone, Default)]
pub(crate) struct BrowserControllerProcessStore {
    ownership: Option<Arc<Mutex<StdioOwnership>>>,
    executions: cancellation::BrowserActionExecutions,
}

impl BrowserControllerProcessStore {
    pub(crate) fn new(command: impl Into<PathBuf>, args: Vec<String>, timeout: Duration) -> Self {
        Self {
            ownership: Some(Arc::new(Mutex::new(
                BrowserControllerProcessOwnership::new(BrowserControllerProcessStdioBackend::new(
                    command, args, timeout,
                )),
            ))),
            ..Self::default()
        }
    }

    pub(crate) fn from_script(script_path: impl Into<PathBuf>, timeout: Duration) -> Self {
        Self {
            ownership: Some(Arc::new(Mutex::new(
                BrowserControllerProcessOwnership::new(
                    BrowserControllerProcessStdioBackend::from_script(script_path, timeout),
                ),
            ))),
            ..Self::default()
        }
    }

    pub(crate) fn from_environment() -> Self {
        let Some(script_path) =
            std::env::var_os(CONTROLLER_SCRIPT_ENV).filter(|path| !path.is_empty())
        else {
            return Self::default();
        };
        let timeout_ms = std::env::var(CONTROLLER_COMMAND_TIMEOUT_ENV)
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_CONTROLLER_COMMAND_TIMEOUT_MS);
        Self::from_script(
            PathBuf::from(script_path),
            Duration::from_millis(timeout_ms),
        )
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.ownership.is_some()
    }

    pub(crate) fn acquire(
        &self,
        session_id: &str,
    ) -> Result<Option<BrowserControllerProcessSnapshot>, String> {
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        let mut ownership = ownership
            .lock()
            .map_err(|_| "browser controller supervisor lock poisoned".to_string())?;
        ownership.acquire(session_id).map(Some)
    }

    pub(crate) fn release(
        &self,
        session_id: &str,
    ) -> Result<Option<BrowserControllerProcessSnapshot>, String> {
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        let mut ownership = ownership
            .lock()
            .map_err(|_| "browser controller supervisor lock poisoned".to_string())?;
        ownership.release(session_id).map(Some)
    }

    pub(crate) fn reconcile_browser(
        &self,
        session_id: &str,
        viewport: &CanonicalViewport,
    ) -> Result<Option<BrowserControllerReconciliation>, String> {
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        let mut ownership = ownership
            .lock()
            .map_err(|_| "browser controller supervisor lock poisoned".to_string())?;
        ownership.reconcile_browser(session_id, viewport).map(Some)
    }

    pub(crate) fn capture_browser_snapshot(
        &self,
        session_id: &str,
        target_id: &str,
        document_id: &str,
    ) -> Result<Option<BrowserControllerStructuredSnapshot>, String> {
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        let mut ownership = ownership
            .lock()
            .map_err(|_| "browser controller supervisor lock poisoned".to_string())?;
        ownership
            .capture_browser_snapshot(session_id, target_id, document_id)
            .map(Some)
    }

    pub(crate) fn manage_browser_tab(
        &self,
        session_id: &str,
        target_id: &str,
        document_id: &str,
        action: BrowserTabAction,
    ) -> Result<Option<BrowserControllerTabResult>, String> {
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        let mut ownership = ownership
            .lock()
            .map_err(|_| "browser controller supervisor lock poisoned".to_string())?;
        ownership
            .manage_browser_tab(session_id, target_id, document_id, action)
            .map(Some)
    }

    pub(crate) fn navigate_browser_history(
        &self,
        session_id: &str,
        target_id: &str,
        document_id: &str,
        action: BrowserHistoryAction,
    ) -> Result<Option<BrowserControllerHistoryResult>, String> {
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        let mut ownership = ownership
            .lock()
            .map_err(|_| "browser controller supervisor lock poisoned".to_string())?;
        ownership
            .navigate_browser_history(session_id, target_id, document_id, action)
            .map(Some)
    }

    pub(crate) fn perform_browser_action(
        &self,
        session_id: &str,
        target_id: &str,
        document_id: &str,
        node_ref: &str,
        action: &BrowserLocatorAction,
        timeout_ms: u64,
    ) -> Result<Option<BrowserControllerActionResult>, String> {
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        let mut ownership = ownership
            .lock()
            .map_err(|_| "browser controller supervisor lock poisoned".to_string())?;
        ownership
            .perform_browser_action(
                session_id,
                target_id,
                document_id,
                node_ref,
                action,
                timeout_ms,
            )
            .map(Some)
    }

    pub(crate) fn navigate_browser(
        &self,
        session_id: &str,
        target_id: &str,
        document_id: &str,
        url: &str,
    ) -> Result<Option<BrowserControllerNavigationResult>, String> {
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        let mut ownership = ownership
            .lock()
            .map_err(|_| "browser controller supervisor lock poisoned".to_string())?;
        ownership
            .navigate_browser(session_id, target_id, document_id, url)
            .map(Some)
    }

    pub(crate) fn wait_for_browser(
        &self,
        session_id: &str,
        target_id: &str,
        document_id: &str,
        wait: &BrowserCompatibilityWait,
        timeout_ms: u64,
    ) -> Result<Option<BrowserControllerCompatibilityWaitResult>, String> {
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        let mut ownership = ownership
            .lock()
            .map_err(|_| "browser controller supervisor lock poisoned".to_string())?;
        ownership
            .wait_for_browser(session_id, target_id, document_id, wait, timeout_ms)
            .map(Some)
    }

    pub(crate) fn handle_browser_dialog(
        &self,
        session_id: &str,
        target_id: &str,
        document_id: &str,
        action: &BrowserDialogAction,
    ) -> Result<Option<BrowserControllerDialogResult>, String> {
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        let mut ownership = ownership
            .lock()
            .map_err(|_| "browser controller supervisor lock poisoned".to_string())?;
        ownership
            .handle_browser_dialog(session_id, target_id, document_id, action)
            .map(Some)
    }

    pub(crate) fn cancel_browser_download(
        &self,
        session_id: &str,
        cancellation: &BrowserDownloadCancellation,
    ) -> Result<Option<BrowserControllerDownloadCancellationResult>, String> {
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        let mut ownership = ownership
            .lock()
            .map_err(|_| "browser controller supervisor lock poisoned".to_string())?;
        ownership
            .cancel_browser_download(session_id, cancellation)
            .map(Some)
    }

    pub(crate) fn poll_browser_events(
        &self,
        session_id: &str,
        browser_generation: u64,
        cursor: u64,
        limit: u16,
    ) -> Result<Option<BrowserControllerEventBatch>, String> {
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        let mut ownership = ownership
            .lock()
            .map_err(|_| "browser controller supervisor lock poisoned".to_string())?;
        ownership
            .poll_browser_events(session_id, browser_generation, cursor, limit)
            .map(Some)
    }

    pub(crate) fn shutdown(&self) -> Result<Option<BrowserControllerProcessSnapshot>, String> {
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        let mut ownership = ownership
            .lock()
            .map_err(|_| "browser controller supervisor lock poisoned".to_string())?;
        ownership.shutdown().map(Some)
    }

    pub(crate) fn snapshot(&self) -> Result<Option<BrowserControllerProcessSnapshot>, String> {
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        let ownership = ownership
            .lock()
            .map_err(|_| "browser controller supervisor lock poisoned".to_string())?;
        Ok(Some(ownership.supervisor.snapshot().clone()))
    }
}

impl<B: BrowserControllerProcessBackend> BrowserControllerProcessSupervisor<B> {
    pub(crate) fn new(backend: B) -> Self {
        Self {
            backend,
            snapshot: BrowserControllerProcessSnapshot {
                state: BrowserControllerProcessState::Stopped,
                process_id: None,
                diagnostic_code: None,
                runtime_generation: 1,
                restart_count: 0,
            },
            recovery_pending: false,
        }
    }

    pub(crate) fn snapshot(&self) -> &BrowserControllerProcessSnapshot {
        &self.snapshot
    }

    pub(crate) fn ensure_started(&mut self) -> Result<&BrowserControllerProcessSnapshot, String> {
        let health = self.backend.health();
        match health {
            Ok(health) if health.state == BrowserControllerProcessState::Ready => {
                self.apply_health(health);
                return Ok(&self.snapshot);
            }
            Ok(health)
                if matches!(
                    health.state,
                    BrowserControllerProcessState::Unhealthy
                        | BrowserControllerProcessState::Failed
                ) =>
            {
                self.apply_health(health);
                self.restart()?;
                return Ok(&self.snapshot);
            }
            Ok(health)
                if health.state == BrowserControllerProcessState::Stopped
                    && self.snapshot.state != BrowserControllerProcessState::Stopped =>
            {
                self.apply_health(health);
                self.restart()?;
                return Ok(&self.snapshot);
            }
            Err(_) => {
                self.restart()?;
                return Ok(&self.snapshot);
            }
            Ok(health) => self.apply_health(health),
        }

        self.start()?;
        Ok(&self.snapshot)
    }

    pub(crate) fn stop(&mut self) -> Result<&BrowserControllerProcessSnapshot, String> {
        if self.snapshot.state == BrowserControllerProcessState::Stopped {
            return Ok(&self.snapshot);
        }
        self.backend.stop()?;
        self.snapshot.state = BrowserControllerProcessState::Stopped;
        self.snapshot.process_id = None;
        self.snapshot.diagnostic_code = None;
        Ok(&self.snapshot)
    }

    fn reconcile_browser(
        &mut self,
        viewport: &CanonicalViewport,
    ) -> Result<BrowserControllerReconciliation, String> {
        let process = self.ensure_started()?.clone();
        let browser = self.backend.reconcile_browser(viewport)?;
        self.recovery_pending = false;
        Ok(BrowserControllerReconciliation { process, browser })
    }

    fn capture_browser_snapshot(
        &mut self,
        target_id: &str,
        document_id: &str,
    ) -> Result<BrowserControllerStructuredSnapshot, String> {
        self.ensure_started_without_transparent_restart()?;
        self.backend
            .capture_browser_snapshot(target_id, document_id)
    }

    fn manage_browser_tab(
        &mut self,
        target_id: &str,
        document_id: &str,
        action: BrowserTabAction,
    ) -> Result<BrowserControllerTabResult, String> {
        self.ensure_started_without_transparent_restart()?;
        self.backend
            .manage_browser_tab(target_id, document_id, action)
    }

    fn navigate_browser_history(
        &mut self,
        target_id: &str,
        document_id: &str,
        action: BrowserHistoryAction,
    ) -> Result<BrowserControllerHistoryResult, String> {
        self.ensure_started_without_transparent_restart()?;
        self.backend
            .navigate_browser_history(target_id, document_id, action)
    }

    fn perform_browser_action(
        &mut self,
        target_id: &str,
        document_id: &str,
        node_ref: &str,
        action: &BrowserLocatorAction,
        timeout_ms: u64,
    ) -> Result<BrowserControllerActionResult, String> {
        self.ensure_started_without_transparent_restart()?;
        self.backend
            .perform_browser_action(target_id, document_id, node_ref, action, timeout_ms)
    }

    fn navigate_browser(
        &mut self,
        target_id: &str,
        document_id: &str,
        url: &str,
    ) -> Result<BrowserControllerNavigationResult, String> {
        self.ensure_started_without_transparent_restart()?;
        self.backend.navigate_browser(target_id, document_id, url)
    }

    fn wait_for_browser(
        &mut self,
        target_id: &str,
        document_id: &str,
        wait: &BrowserCompatibilityWait,
        timeout_ms: u64,
    ) -> Result<BrowserControllerCompatibilityWaitResult, String> {
        self.ensure_started_without_transparent_restart()?;
        self.backend
            .wait_for_browser(target_id, document_id, wait, timeout_ms)
    }

    fn handle_browser_dialog(
        &mut self,
        target_id: &str,
        document_id: &str,
        action: &BrowserDialogAction,
    ) -> Result<BrowserControllerDialogResult, String> {
        self.ensure_started_without_transparent_restart()?;
        self.backend
            .handle_browser_dialog(target_id, document_id, action)
    }

    fn configure_browser_downloads(
        &mut self,
        target_id: &str,
        document_id: &str,
    ) -> Result<BrowserControllerDownloadsResult, String> {
        self.ensure_started_without_transparent_restart()?;
        self.backend
            .configure_browser_downloads(target_id, document_id)
    }

    fn cancel_browser_download(
        &mut self,
        cancellation: &BrowserDownloadCancellation,
    ) -> Result<BrowserControllerDownloadCancellationResult, String> {
        self.ensure_started_without_transparent_restart()?;
        self.backend.cancel_browser_download(cancellation)
    }

    fn upload_browser_files(
        &mut self,
        target_id: &str,
        document_id: &str,
        node_ref: &str,
        files: &BrowserUploadFiles,
    ) -> Result<BrowserControllerUploadResult, String> {
        self.ensure_started_without_transparent_restart()?;
        self.backend
            .upload_browser_files(target_id, document_id, node_ref, files)
    }

    fn set_browser_permission(
        &mut self,
        target_id: &str,
        document_id: &str,
        permission: BrowserPermissionName,
        setting: BrowserPermissionSetting,
    ) -> Result<BrowserControllerPermissionResult, String> {
        self.ensure_started_without_transparent_restart()?;
        self.backend
            .set_browser_permission(target_id, document_id, permission, setting)
    }

    fn poll_browser_events(
        &mut self,
        browser_generation: u64,
        cursor: u64,
        limit: u16,
    ) -> Result<BrowserControllerEventBatch, String> {
        self.ensure_started_without_transparent_restart()?;
        self.backend
            .poll_browser_events(browser_generation, cursor, limit)
    }

    fn ensure_started_without_transparent_restart(&mut self) -> Result<(), String> {
        if self.recovery_pending {
            return Err(CONTROLLER_RESTARTED_BEFORE_OPERATION.to_string());
        }
        let generation = self.snapshot.runtime_generation;
        self.ensure_started()?;
        if self.snapshot.runtime_generation != generation || self.recovery_pending {
            return Err(CONTROLLER_RESTARTED_BEFORE_OPERATION.to_string());
        }
        Ok(())
    }

    fn start(&mut self) -> Result<(), String> {
        self.snapshot.state = BrowserControllerProcessState::Starting;
        match self.backend.start() {
            Ok(health) if health.state == BrowserControllerProcessState::Ready => {
                self.apply_health(health);
                Ok(())
            }
            Ok(health) => {
                let state = health.state;
                self.apply_health(health);
                self.snapshot.state = BrowserControllerProcessState::Failed;
                Err(format!(
                    "browser controller startup ended in {state:?} instead of Ready"
                ))
            }
            Err(error) => {
                self.snapshot.state = BrowserControllerProcessState::Failed;
                self.snapshot.process_id = None;
                self.snapshot.diagnostic_code = Some("start_failed".to_string());
                Err(error)
            }
        }
    }

    fn restart(&mut self) -> Result<(), String> {
        self.backend.stop()?;
        self.snapshot.runtime_generation = self.snapshot.runtime_generation.saturating_add(1);
        self.snapshot.restart_count = self.snapshot.restart_count.saturating_add(1);
        self.recovery_pending = true;
        self.snapshot.process_id = None;
        self.start()
    }

    fn apply_health(&mut self, health: BrowserControllerProcessHealth) {
        self.snapshot.state = health.state;
        self.snapshot.process_id = health.process_id;
        self.snapshot.diagnostic_code = health.diagnostic_code;
    }

    #[cfg(test)]
    fn backend(&self) -> &B {
        &self.backend
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::{
        BrowserControllerProcessBackend, BrowserControllerProcessHealth,
        BrowserControllerProcessState, BrowserControllerProcessStdioBackend,
        BrowserControllerProcessStore, BrowserControllerProcessSupervisor,
        CONTROLLER_RESTARTED_BEFORE_OPERATION,
    };
    use crate::runtime::browser_controller_action::{BrowserDialogAction, BrowserLocatorAction};
    use crate::session::CanonicalViewport;

    const HEALTHY_TEST_CONTROLLER_TIMEOUT: Duration = Duration::from_secs(5);

    #[derive(Default)]
    struct FakeBackend {
        health: VecDeque<Result<BrowserControllerProcessHealth, String>>,
        starts: VecDeque<Result<BrowserControllerProcessHealth, String>>,
        start_count: usize,
        stop_count: usize,
    }

    impl BrowserControllerProcessBackend for FakeBackend {
        fn health(&mut self) -> Result<BrowserControllerProcessHealth, String> {
            self.health
                .pop_front()
                .expect("test must provide a health result")
        }

        fn start(&mut self) -> Result<BrowserControllerProcessHealth, String> {
            self.start_count += 1;
            self.starts
                .pop_front()
                .expect("test must provide a start result")
        }

        fn stop(&mut self) -> Result<(), String> {
            self.stop_count += 1;
            Ok(())
        }
    }

    fn health(
        state: BrowserControllerProcessState,
        process_id: Option<u32>,
    ) -> BrowserControllerProcessHealth {
        BrowserControllerProcessHealth {
            state,
            process_id,
            diagnostic_code: None,
        }
    }

    #[test]
    fn ensure_started_launches_a_stopped_controller() {
        let mut backend = FakeBackend::default();
        backend
            .health
            .push_back(Ok(health(BrowserControllerProcessState::Stopped, None)));
        backend
            .starts
            .push_back(Ok(health(BrowserControllerProcessState::Ready, Some(41))));
        let mut supervisor = BrowserControllerProcessSupervisor::new(backend);

        let snapshot = supervisor.ensure_started().expect("controller starts");

        assert_eq!(snapshot.state, BrowserControllerProcessState::Ready);
        assert_eq!(snapshot.process_id, Some(41));
        assert_eq!(snapshot.runtime_generation, 1);
        assert_eq!(snapshot.restart_count, 0);
        assert_eq!(supervisor.backend().start_count, 1);
        assert_eq!(supervisor.backend().stop_count, 0);
    }

    #[test]
    fn ensure_started_reuses_a_healthy_controller() {
        let mut backend = FakeBackend::default();
        backend
            .health
            .push_back(Ok(health(BrowserControllerProcessState::Ready, Some(42))));
        let mut supervisor = BrowserControllerProcessSupervisor::new(backend);

        let snapshot = supervisor.ensure_started().expect("controller is reused");

        assert_eq!(snapshot.process_id, Some(42));
        assert_eq!(snapshot.runtime_generation, 1);
        assert_eq!(supervisor.backend().start_count, 0);
        assert_eq!(supervisor.backend().stop_count, 0);
    }

    #[test]
    fn ensure_started_restarts_an_unhealthy_controller() {
        let mut backend = FakeBackend::default();
        backend.health.push_back(Ok(BrowserControllerProcessHealth {
            state: BrowserControllerProcessState::Unhealthy,
            process_id: Some(43),
            diagnostic_code: Some("health_timeout".to_string()),
        }));
        backend
            .starts
            .push_back(Ok(health(BrowserControllerProcessState::Ready, Some(44))));
        let mut supervisor = BrowserControllerProcessSupervisor::new(backend);

        let snapshot = supervisor.ensure_started().expect("controller restarts");

        assert_eq!(snapshot.process_id, Some(44));
        assert_eq!(snapshot.runtime_generation, 2);
        assert_eq!(snapshot.restart_count, 1);
        assert_eq!(supervisor.backend().start_count, 1);
        assert_eq!(supervisor.backend().stop_count, 1);
    }

    #[test]
    fn mutation_does_not_run_after_an_implicit_controller_restart() {
        let mut backend = FakeBackend::default();
        backend.health.push_back(Ok(BrowserControllerProcessHealth {
            state: BrowserControllerProcessState::Unhealthy,
            process_id: Some(43),
            diagnostic_code: Some("health_timeout".to_string()),
        }));
        backend
            .starts
            .push_back(Ok(health(BrowserControllerProcessState::Ready, Some(44))));
        let mut supervisor = BrowserControllerProcessSupervisor::new(backend);

        let error = supervisor
            .perform_browser_action(
                "target-a",
                "loader-a",
                "backend:103",
                &BrowserLocatorAction::Click,
                1_000,
            )
            .expect_err("mutation must wait for recovery reconciliation");

        assert_eq!(
            error,
            "browser controller restarted before the operation; reconcile and retry with fresh references"
        );
        assert_eq!(supervisor.snapshot().runtime_generation, 2);
        assert_eq!(supervisor.snapshot().restart_count, 1);

        let repeated_error = supervisor
            .perform_browser_action(
                "target-a",
                "loader-a",
                "backend:103",
                &BrowserLocatorAction::Click,
                1_000,
            )
            .expect_err("the restarted controller stays fenced until reconciliation");

        assert_eq!(repeated_error, error);
        assert_eq!(supervisor.backend().start_count, 1);
        assert_eq!(supervisor.backend().stop_count, 1);
    }

    #[test]
    fn mutation_does_not_run_when_a_ready_controller_disappears() {
        let mut backend = FakeBackend::default();
        backend
            .health
            .push_back(Ok(health(BrowserControllerProcessState::Stopped, None)));
        backend
            .starts
            .push_back(Ok(health(BrowserControllerProcessState::Ready, Some(43))));
        backend
            .health
            .push_back(Ok(health(BrowserControllerProcessState::Stopped, None)));
        backend
            .starts
            .push_back(Ok(health(BrowserControllerProcessState::Ready, Some(44))));
        let mut supervisor = BrowserControllerProcessSupervisor::new(backend);
        supervisor.ensure_started().expect("controller starts");

        let error = supervisor
            .perform_browser_action(
                "target-a",
                "loader-a",
                "backend:103",
                &BrowserLocatorAction::Click,
                1_000,
            )
            .expect_err("a disappeared ready controller requires reconciliation");

        assert_eq!(error, CONTROLLER_RESTARTED_BEFORE_OPERATION);
        assert_eq!(supervisor.snapshot().runtime_generation, 2);
        assert_eq!(supervisor.snapshot().restart_count, 1);
        assert_eq!(supervisor.backend().start_count, 2);
        assert_eq!(supervisor.backend().stop_count, 1);
    }

    #[test]
    fn failed_start_is_reported_without_claiming_ready() {
        let mut backend = FakeBackend::default();
        backend
            .health
            .push_back(Ok(health(BrowserControllerProcessState::Stopped, None)));
        backend
            .starts
            .push_back(Err("controller did not become healthy".to_string()));
        let mut supervisor = BrowserControllerProcessSupervisor::new(backend);

        let error = supervisor.ensure_started().expect_err("startup must fail");

        assert_eq!(error, "controller did not become healthy");
        assert_eq!(
            supervisor.snapshot().state,
            BrowserControllerProcessState::Failed
        );
        assert_eq!(supervisor.snapshot().runtime_generation, 1);
    }

    #[test]
    fn stop_is_idempotent_and_does_not_advance_generation() {
        let mut backend = FakeBackend::default();
        backend
            .health
            .push_back(Ok(health(BrowserControllerProcessState::Ready, Some(45))));
        let mut supervisor = BrowserControllerProcessSupervisor::new(backend);
        supervisor.ensure_started().expect("controller is running");

        supervisor.stop().expect("first stop succeeds");
        supervisor.stop().expect("second stop succeeds");

        assert_eq!(
            supervisor.snapshot().state,
            BrowserControllerProcessState::Stopped
        );
        assert_eq!(supervisor.snapshot().runtime_generation, 1);
        assert_eq!(supervisor.backend().stop_count, 1);
    }

    #[test]
    fn physical_browser_binding_survives_release_and_shutdown() {
        let tool = TestTool::new(responsive_controller_script());
        let store = BrowserControllerProcessStore::new(
            tool.path(),
            Vec::new(),
            HEALTHY_TEST_CONTROLLER_TIMEOUT,
        );
        assert!(store.acquire(" ").is_err());
        let first = store.acquire("room-1").unwrap().unwrap();
        assert_eq!(
            store.acquire("room-1").unwrap().unwrap().process_id,
            first.process_id
        );
        assert!(store.acquire("room-2").is_err());
        assert_eq!(
            store.release("room-2").unwrap().unwrap().process_id,
            first.process_id
        );
        assert_eq!(
            store.release("room-1").unwrap().unwrap().state,
            BrowserControllerProcessState::Stopped
        );
        assert!(store.acquire("room-2").is_err());
        store.acquire("room-1").expect("owner restarts");
        store.shutdown().expect("controller shuts down");
        assert!(store.acquire("room-2").is_err());
        store
            .acquire("room-1")
            .expect("shutdown retains owner binding");
        store.shutdown().expect("clean up controller");
    }

    #[test]
    fn concurrent_rooms_admit_only_one_physical_browser_owner() {
        let tool = TestTool::new(responsive_controller_script());
        let store = BrowserControllerProcessStore::new(
            tool.path(),
            Vec::new(),
            HEALTHY_TEST_CONTROLLER_TIMEOUT,
        );
        let barrier = std::sync::Barrier::new(2);
        let (first, second) = std::thread::scope(|scope| {
            let first = scope.spawn(|| {
                barrier.wait();
                store.acquire("room-1")
            });
            let second = scope.spawn(|| {
                barrier.wait();
                store.acquire("room-2")
            });
            (first.join().unwrap(), second.join().unwrap())
        });
        assert_ne!(
            first.is_ok(),
            second.is_ok(),
            "exactly one Room must be admitted"
        );
        let (owner, rejected, snapshot) = match (first, second) {
            (Ok(Some(snapshot)), Err(_)) => ("room-1", "room-2", snapshot),
            (Err(_), Ok(Some(snapshot))) => ("room-2", "room-1", snapshot),
            results => panic!("unexpected admission results: {results:?}"),
        };
        store
            .release(rejected)
            .expect("rejected Room release is harmless");
        assert_eq!(
            store.acquire(owner).unwrap().unwrap().process_id,
            snapshot.process_id
        );
        store.shutdown().expect("clean up controller");
    }

    #[test]
    fn failed_controller_start_keeps_the_physical_browser_owner() {
        let tool = TestTool::new("#!/bin/sh\nexit 1\n");
        let store = BrowserControllerProcessStore::new(
            tool.path(),
            Vec::new(),
            HEALTHY_TEST_CONTROLLER_TIMEOUT,
        );
        assert!(store.acquire("room-1").is_err());
        assert_eq!(
            store.acquire("room-2").unwrap_err(),
            "browser controller is bound to another Room"
        );
        assert_eq!(
            store.release("room-1").unwrap().unwrap().state,
            BrowserControllerProcessState::Stopped
        );
        assert_eq!(
            store.acquire("room-2").unwrap_err(),
            "browser controller is bound to another Room"
        );
        store.shutdown().expect("clean up failed controller");
    }

    struct TestTool {
        root: PathBuf,
        path: PathBuf,
    }

    impl TestTool {
        fn new(script: &str) -> Self {
            static SEQUENCE: AtomicU64 = AtomicU64::new(0);
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock")
                .as_nanos();
            let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "chariox-browser-controller-process-{}-{nonce}-{sequence}",
                std::process::id()
            ));
            fs::create_dir_all(&root).expect("create test tool root");
            let path = root.join("controller-tool.sh");
            fs::write(&path, script).expect("write test tool");
            let mut permissions = fs::metadata(&path).expect("tool metadata").permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(&path, permissions).expect("make test tool executable");
            Self { root, path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestTool {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn responsive_controller_script() -> &'static str {
        "#!/bin/sh\nset -eu\nwhile IFS= read -r request; do\n  id=${request#*:}\n  id=${id%%,*}\n  case \"$request\" in\n    *'\"method\":\"health\"'*) printf '{\"id\":%s,\"ok\":true,\"result\":{\"state\":\"ready\",\"process_id\":%s,\"diagnostic_code\":null}}\\n' \"$id\" \"$$\" ;;\n    *'\"method\":\"shutdown\"'*) printf '{\"id\":%s,\"ok\":true,\"result\":{\"state\":\"stopped\",\"process_id\":null,\"diagnostic_code\":null}}\\n' \"$id\"; exit 0 ;;\n  esac\ndone\n"
    }

    #[test]
    fn stdio_backend_starts_reports_health_and_stops() {
        let tool = TestTool::new(responsive_controller_script());
        let mut backend = BrowserControllerProcessStdioBackend::new(
            tool.path(),
            Vec::new(),
            HEALTHY_TEST_CONTROLLER_TIMEOUT,
        );

        assert_eq!(
            backend.health().expect("stopped health").state,
            BrowserControllerProcessState::Stopped
        );
        let started = backend.start().expect("controller starts");
        let health = backend.health().expect("health response parses");

        assert_eq!(started.state, BrowserControllerProcessState::Ready);
        assert_eq!(health.state, BrowserControllerProcessState::Ready);
        assert_eq!(health.process_id, started.process_id);
        assert_eq!(health.diagnostic_code, None);
        backend.stop().expect("controller stops");
        assert_eq!(
            backend.health().expect("stopped health").state,
            BrowserControllerProcessState::Stopped
        );
    }

    #[test]
    fn stdio_backend_skips_a_stale_response_before_the_matching_reply() {
        let tool = TestTool::new(
            "#!/bin/sh\nset -eu\nwhile IFS= read -r request; do\n  id=${request#*:}\n  id=${id%%,*}\n  case \"$request\" in\n    *'\"method\":\"health\"'*) printf '{\"id\":0,\"ok\":true,\"result\":{\"state\":\"ready\",\"process_id\":%s,\"diagnostic_code\":null}}\\n' \"$$\"; printf '{\"id\":%s,\"ok\":true,\"result\":{\"state\":\"ready\",\"process_id\":%s,\"diagnostic_code\":null}}\\n' \"$id\" \"$$\" ;;\n    *'\"method\":\"shutdown\"'*) printf '{\"id\":%s,\"ok\":true,\"result\":{\"state\":\"stopped\",\"process_id\":null,\"diagnostic_code\":null}}\\n' \"$id\"; exit 0 ;;\n  esac\ndone\n",
        );
        let mut backend = BrowserControllerProcessStdioBackend::new(
            tool.path(),
            Vec::new(),
            HEALTHY_TEST_CONTROLLER_TIMEOUT,
        );

        let started = backend
            .start()
            .expect("a stale response should not mask the matching health reply");

        assert_eq!(started.state, BrowserControllerProcessState::Ready);
        backend.stop().expect("controller stops");
    }

    #[test]
    fn room_lease_reconciles_browser_tabs_and_canonical_viewport_over_stdio() {
        let tool = TestTool::new(
            "#!/bin/sh\nset -eu\nwhile IFS= read -r request; do\n  id=${request#*:}\n  id=${id%%,*}\n  case \"$request\" in\n    *'\"method\":\"health\"'*) printf '{\"id\":%s,\"ok\":true,\"result\":{\"state\":\"ready\",\"process_id\":%s,\"diagnostic_code\":null}}\\n' \"$id\" \"$$\" ;;\n    *'\"method\":\"browser.reconcile\"'*) printf '{\"id\":%s,\"ok\":true,\"result\":{\"browser_generation\":1,\"tabs\":[{\"target_id\":\"target-a\",\"document_id\":\"loader-a\",\"url\":\"https://a.test\",\"title\":\"A\"}],\"focused_target_id\":\"target-a\",\"viewport\":{\"css_width\":1280,\"css_height\":720,\"device_scale_factor\":1,\"desktop_pixel_width\":1280,\"desktop_pixel_height\":720}}}\\n' \"$id\" ;;\n    *'\"method\":\"shutdown\"'*) printf '{\"id\":%s,\"ok\":true,\"result\":{\"state\":\"stopped\",\"process_id\":null,\"diagnostic_code\":null}}\\n' \"$id\"; exit 0 ;;\n  esac\ndone\n",
        );
        let store = BrowserControllerProcessStore::new(
            tool.path(),
            Vec::new(),
            HEALTHY_TEST_CONTROLLER_TIMEOUT,
        );
        let viewport = CanonicalViewport::new(1280, 720, 1, 1280, 720).unwrap();

        assert!(store.reconcile_browser("room-1", &viewport).is_err());
        store.acquire("room-1").expect("Room acquires controller");
        let reconciliation = store
            .reconcile_browser("room-1", &viewport)
            .expect("browser reconciles")
            .expect("controller is enabled");

        assert_eq!(
            reconciliation.process.state,
            BrowserControllerProcessState::Ready
        );
        assert_eq!(reconciliation.browser.browser_generation, 1);
        assert_eq!(reconciliation.browser.tabs[0].target_id, "target-a");
        assert_eq!(
            reconciliation.browser.focused_target_id.as_deref(),
            Some("target-a")
        );
        store.release("room-1").expect("Room releases controller");
    }

    #[test]
    fn room_lease_captures_a_document_bound_structured_snapshot_over_stdio() {
        let tool = TestTool::new(
            "#!/bin/sh\nset -eu\nwhile IFS= read -r request; do\n  id=${request#*:}\n  id=${id%%,*}\n  case \"$request\" in\n    *'\"method\":\"health\"'*) printf '{\"id\":%s,\"ok\":true,\"result\":{\"state\":\"ready\",\"process_id\":%s,\"diagnostic_code\":null}}\\n' \"$id\" \"$$\" ;;\n    *'\"method\":\"browser.snapshot\"'*) printf '{\"id\":%s,\"ok\":true,\"result\":{\"browser_generation\":1,\"target_id\":\"target-a\",\"document_id\":\"loader-a\",\"snapshot_revision\":1,\"accessibility_nodes\":[{\"node_ref\":\"backend:103\",\"parent_ref\":null,\"child_refs\":[],\"role\":\"button\",\"name\":\"Save\",\"description\":\"\",\"value\":\"\",\"ignored\":false,\"disabled\":false,\"focused\":true}],\"dom_documents\":[{\"document_index\":0,\"url\":\"https://a.test\",\"owner_node_ref\":null}],\"dom_nodes\":[{\"node_ref\":\"backend:103\",\"parent_ref\":\"backend:102\",\"document_index\":0,\"node_type\":1,\"node_name\":\"BUTTON\",\"text\":\"\",\"attributes\":{\"id\":\"save\"},\"bounds\":{\"x\":10,\"y\":20,\"width\":100,\"height\":30}}]}}\\n' \"$id\" ;;\n    *'\"method\":\"shutdown\"'*) printf '{\"id\":%s,\"ok\":true,\"result\":{\"state\":\"stopped\",\"process_id\":null,\"diagnostic_code\":null}}\\n' \"$id\"; exit 0 ;;\n  esac\ndone\n",
        );
        let store = BrowserControllerProcessStore::new(
            tool.path(),
            Vec::new(),
            HEALTHY_TEST_CONTROLLER_TIMEOUT,
        );
        store.acquire("room-1").expect("Room acquires controller");

        let snapshot = store
            .capture_browser_snapshot("room-1", "target-a", "loader-a")
            .expect("structured snapshot succeeds")
            .expect("controller is enabled");

        assert_eq!(snapshot.browser_generation, 1);
        assert_eq!(snapshot.snapshot_revision, 1);
        assert_eq!(snapshot.accessibility_nodes[0].node_ref, "backend:103");
        assert_eq!(snapshot.dom_documents[0].document_index, 0);
        assert_eq!(snapshot.dom_nodes[0].attributes["id"], "save");
        store.release("room-1").expect("Room releases controller");
    }

    #[test]
    fn room_lease_performs_a_bounded_locator_action_over_stdio() {
        let tool = TestTool::new(
            "#!/bin/sh\nset -eu\nwhile IFS= read -r request; do\n  id=${request#*:}\n  id=${id%%,*}\n  case \"$request\" in\n    *'\"method\":\"health\"'*) printf '{\"id\":%s,\"ok\":true,\"result\":{\"state\":\"ready\",\"process_id\":%s,\"diagnostic_code\":null}}\\n' \"$id\" \"$$\" ;;\n    *'\"method\":\"browser.action\"'*) printf '{\"id\":%s,\"ok\":true,\"result\":{\"browser_generation\":1,\"target_id\":\"target-a\",\"document_id\":\"loader-a\",\"action_kind\":\"click\",\"dialog_opened\":true,\"attempts\":2,\"elapsed_ms\":50}}\\n' \"$id\" ;;\n    *'\"method\":\"browser.dialog\"'*) printf '{\"id\":%s,\"ok\":true,\"result\":{\"browser_generation\":1,\"target_id\":\"target-a\",\"document_id\":\"loader-a\",\"action\":\"dismiss\"}}\\n' \"$id\" ;;\n    *'\"method\":\"shutdown\"'*) printf '{\"id\":%s,\"ok\":true,\"result\":{\"state\":\"stopped\",\"process_id\":null,\"diagnostic_code\":null}}\\n' \"$id\"; exit 0 ;;\n  esac\ndone\n",
        );
        let store = BrowserControllerProcessStore::new(
            tool.path(),
            Vec::new(),
            HEALTHY_TEST_CONTROLLER_TIMEOUT,
        );
        store.acquire("room-1").expect("Room acquires controller");

        let result = store
            .perform_browser_action(
                "room-1",
                "target-a",
                "loader-a",
                "backend:103",
                &BrowserLocatorAction::Click,
                500,
            )
            .expect("locator action succeeds")
            .expect("controller is enabled");

        assert_eq!(result.action_kind, "click");
        assert!(result.dialog_opened);
        assert_eq!(result.attempts, 2);
        assert_eq!(result.elapsed_ms, 50);
        let dialog = store
            .handle_browser_dialog(
                "room-1",
                "target-a",
                "loader-a",
                &BrowserDialogAction::Dismiss,
            )
            .expect("dialog action succeeds")
            .expect("controller is enabled");
        assert_eq!(dialog.action, "dismiss");
        store.release("room-1").expect("Room releases controller");
    }

    #[test]
    fn stdio_supervisor_restarts_a_crashed_controller_with_a_new_process() {
        let tool = TestTool::new(responsive_controller_script());
        let backend = BrowserControllerProcessStdioBackend::new(
            tool.path(),
            Vec::new(),
            HEALTHY_TEST_CONTROLLER_TIMEOUT,
        );
        let mut supervisor = BrowserControllerProcessSupervisor::new(backend);
        let first = supervisor
            .ensure_started()
            .expect("first controller starts")
            .clone();
        let first_process_id = first.process_id.expect("first process id");

        let kill_result = unsafe { libc::kill(first_process_id as i32, libc::SIGKILL) };
        assert_eq!(kill_result, 0, "test controller should be killable");
        let restarted = supervisor
            .ensure_started()
            .expect("crashed controller restarts")
            .clone();

        assert_eq!(restarted.state, BrowserControllerProcessState::Ready);
        assert_ne!(restarted.process_id, Some(first_process_id));
        assert_eq!(restarted.runtime_generation, 2);
        assert_eq!(restarted.restart_count, 1);
        supervisor.stop().expect("restarted controller stops");
    }

    #[test]
    fn stdio_backend_reports_early_exit_without_claiming_ready() {
        let tool = TestTool::new("#!/bin/sh\nexit 9\n");
        let mut backend = BrowserControllerProcessStdioBackend::new(
            tool.path(),
            Vec::new(),
            HEALTHY_TEST_CONTROLLER_TIMEOUT,
        );

        let error = backend.start().expect_err("early exit must fail");

        assert!(error.contains("exited during `health`"));
    }

    #[test]
    fn stdio_backend_rejects_a_process_identity_mismatch_and_reaps_the_child() {
        let tool = TestTool::new(
            "#!/bin/sh\nset -eu\nIFS= read -r request\nid=${request#*:}\nid=${id%%,*}\nprintf '{\"id\":%s,\"ok\":true,\"result\":{\"state\":\"ready\",\"process_id\":1,\"diagnostic_code\":null}}\\n' \"$id\"\nexec sleep 30\n",
        );
        let mut backend = BrowserControllerProcessStdioBackend::new(
            tool.path(),
            Vec::new(),
            HEALTHY_TEST_CONTROLLER_TIMEOUT,
        );

        let error = backend.start().expect_err("foreign process id must fail");

        assert!(error.contains("expected"));
        assert_eq!(
            backend
                .health()
                .expect("mismatched controller was reaped")
                .state,
            BrowserControllerProcessState::Stopped
        );
    }

    #[test]
    fn stdio_backend_kills_a_controller_that_does_not_answer_health() {
        let tool = TestTool::new("#!/bin/sh\nexec sleep 30\n");
        let mut backend = BrowserControllerProcessStdioBackend::new(
            tool.path(),
            Vec::new(),
            Duration::from_millis(25),
        );

        let started = std::time::Instant::now();
        let error = backend.start().expect_err("health request must time out");

        assert!(error.contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(
            backend
                .health()
                .expect("timed out process was reaped")
                .state,
            BrowserControllerProcessState::Stopped
        );
    }

    #[test]
    fn stdio_backend_bounds_shutdown_and_kills_the_process_group() {
        let tool = TestTool::new(
            "#!/bin/sh\nset -eu\nIFS= read -r request\nid=${request#*:}\nid=${id%%,*}\nprintf '{\"id\":%s,\"ok\":true,\"result\":{\"state\":\"ready\",\"process_id\":%s,\"diagnostic_code\":null}}\\n' \"$id\" \"$$\"\nIFS= read -r request\nsleep 30 &\nwait\n",
        );
        let mut backend = BrowserControllerProcessStdioBackend::new(
            tool.path(),
            Vec::new(),
            Duration::from_secs(2),
        );
        backend.start().expect("controller starts");

        let started = std::time::Instant::now();
        backend.stop().expect("bounded forced stop succeeds");

        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(
            backend.health().expect("forced process was reaped").state,
            BrowserControllerProcessState::Stopped
        );
    }
}
