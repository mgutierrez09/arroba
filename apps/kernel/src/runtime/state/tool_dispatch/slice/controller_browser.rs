use crate::error::DaemonError;
use crate::runtime::state::KernelRuntimeState;

use super::controller_browser_projection::*;
use super::slice_browser::{
    browser_status_url, ensure_browser_fill_target, ensure_browser_secret_target_is_masked,
    ensure_browser_target_matches_expectations,
};

impl KernelRuntimeState {
    pub(super) async fn controller_paste_secret_to_slice_tool_result(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        slice_id: &str,
        agent_id: &str,
        args: crate::transport::runtime_tools::PasteSecretToSliceArgs,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let status =
            capture_controller_browser_status(self, provider_run.session_id(), slice_id, agent_id)
                .await?;
        let browser =
            status
                .result
                .payload
                .get("browser")
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "runtime_tool_paste_secret_to_slice",
                    message:
                        "controller-backed browser status omitted its compatibility projection"
                            .to_string(),
                })?;
        browser_status_url(browser)?;
        ensure_browser_target_matches_expectations(browser, &args)?;
        let element_ref = match controller_browser_element_ref(
            args.selector.as_deref(),
            args.field_id.as_deref(),
            "runtime_tool_paste_secret_to_slice",
        ) {
            Ok(reference) => reference,
            Err(_) if args.selector.is_none() && args.field_id.is_none() => {
                ensure_browser_fill_target(browser, None)?;
                browser
                    .pointer("/focusedElement/field_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| DaemonError::LocalTransport {
                        operation: "runtime_tool_paste_secret_to_slice",
                        message: "the focused browser field has no opaque element reference"
                            .to_string(),
                    })?
            }
            Err(error) => return Err(error),
        };
        ensure_browser_fill_target(browser, Some(&element_ref))?;
        ensure_browser_secret_target_is_masked(browser, Some(&element_ref))?;
        let target_document_url = status
            .structured_snapshot
            .as_ref()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "runtime_tool_paste_secret_to_slice",
                message: "browser snapshot is unavailable for secret target authorization"
                    .to_string(),
            })?
            .document_url_for_element(&element_ref)
            .map(str::to_string)
            .map_err(|message| DaemonError::LocalTransport {
                operation: "runtime_tool_paste_secret_to_slice",
                message,
            })?;
        let secret = match self
            .resolve_remote_home_credential_secret(
                provider_run,
                &args.credential_id,
                crate::transport::relay_peer::RemoteCredentialSecretInjection::Browser {
                    target_url: target_document_url.clone(),
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
                    &target_document_url,
                )?)
            }
        };
        let result = self
            .perform_browser_environment_locator_action_as_agent(
                provider_run.session_id(),
                agent_id,
                &element_ref,
                crate::runtime::browser_controller_action::BrowserLocatorAction::Fill {
                    text: secret.to_string(),
                    append: false,
                    submit: args.submit,
                    expected_document_url: Some(target_document_url),
                },
                crate::runtime::browser_controller_action::MAX_BROWSER_ACTION_TIMEOUT_MS,
            )
            .await?;
        Ok(controller_secret_paste_tool_result(
            slice_id,
            agent_id,
            &args.credential_id,
            args.submit,
            result,
        ))
    }

    pub(super) async fn controller_browser_status_tool_result(
        &self,
        session_id: &str,
        slice_id: &str,
        agent_id: &str,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        run_controller_browser_status_tool(self, session_id, slice_id, agent_id).await
    }

    pub(super) async fn controller_browser_tab_tool_result(
        &self,
        session_id: &str,
        slice_id: &str,
        agent_id: &str,
        args: crate::transport::runtime_tools::SliceBrowserTabArgs,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let action = match args.action.as_str() {
            "activate" => crate::runtime::browser_controller_tab::BrowserTabAction::Activate,
            "close" => crate::runtime::browser_controller_tab::BrowserTabAction::Close,
            other => {
                return Err(DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_browser_tab",
                    message: format!("unsupported browser tab action `{other}`"),
                });
            }
        };
        let execution = self
            .manage_browser_environment_tab_as_agent(session_id, agent_id, &args.tab_id, action)
            .await?;
        Ok(controller_browser_tab_tool_result(
            slice_id, agent_id, action, execution,
        ))
    }

    pub(super) async fn controller_browser_history_tool_result(
        &self,
        session_id: &str,
        slice_id: &str,
        agent_id: &str,
        args: crate::transport::runtime_tools::SliceBrowserHistoryArgs,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let action = match args.action.as_str() {
            "back" => crate::runtime::browser_controller_history::BrowserHistoryAction::Back,
            "forward" => crate::runtime::browser_controller_history::BrowserHistoryAction::Forward,
            "reload" => crate::runtime::browser_controller_history::BrowserHistoryAction::Reload,
            other => {
                return Err(DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_browser_history",
                    message: format!("unsupported browser history action `{other}`"),
                });
            }
        };
        let tab_id = args.tab_id;
        let execution = self
            .navigate_browser_environment_history_as_agent(session_id, agent_id, &tab_id, action)
            .await?;
        controller_browser_history_tool_result(slice_id, agent_id, &tab_id, action, execution)
    }

    pub(super) async fn controller_browser_find_tool_result(
        &self,
        session_id: &str,
        slice_id: &str,
        agent_id: &str,
        query: &str,
        kind: &str,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        run_controller_browser_find_tool(self, session_id, slice_id, agent_id, query, kind).await
    }

    pub(super) async fn controller_browser_text_tool_result(
        &self,
        session_id: &str,
        slice_id: &str,
        agent_id: &str,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        run_controller_browser_text_tool(self, session_id, slice_id, agent_id).await
    }

    pub(super) async fn controller_browser_wait_for_text_tool_result(
        &self,
        session_id: &str,
        slice_id: &str,
        agent_id: &str,
        query: &str,
        timeout_ms: Option<u64>,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        run_controller_browser_wait_for_text_tool(
            self, session_id, slice_id, agent_id, query, timeout_ms,
        )
        .await
    }

    pub(super) async fn controller_browser_fill_tool_result(
        &self,
        session_id: &str,
        slice_id: &str,
        agent_id: &str,
        args: crate::transport::runtime_tools::SliceBrowserFillArgs,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let element_ref = controller_browser_element_ref(
            args.selector.as_deref(),
            args.field_id.as_deref(),
            "runtime_tool_slice_browser_fill",
        )?;
        let result = self
            .perform_browser_environment_locator_action_as_agent(
                session_id,
                agent_id,
                &element_ref,
                crate::runtime::browser_controller_action::BrowserLocatorAction::Fill {
                    text: args.text,
                    append: false,
                    submit: false,
                    expected_document_url: None,
                },
                crate::runtime::browser_controller_action::MAX_BROWSER_ACTION_TIMEOUT_MS,
            )
            .await?;
        Ok(controller_browser_action_tool_result(
            slice_id, agent_id, result,
        ))
    }

    pub(super) async fn controller_browser_click_tool_result(
        &self,
        session_id: &str,
        slice_id: &str,
        agent_id: &str,
        args: crate::transport::runtime_tools::SliceBrowserClickArgs,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let element_ref = controller_browser_element_ref(
            args.selector.as_deref(),
            args.field_id.as_deref(),
            "runtime_tool_slice_browser_click",
        )?;
        let result = self
            .perform_browser_environment_locator_action_as_agent(
                session_id,
                agent_id,
                &element_ref,
                crate::runtime::browser_controller_action::BrowserLocatorAction::Click,
                crate::runtime::browser_controller_action::MAX_BROWSER_ACTION_TIMEOUT_MS,
            )
            .await?;
        Ok(controller_browser_action_tool_result(
            slice_id, agent_id, result,
        ))
    }

    pub(super) async fn controller_browser_submit_tool_result(
        &self,
        session_id: &str,
        slice_id: &str,
        agent_id: &str,
        args: crate::transport::runtime_tools::SliceBrowserSubmitArgs,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let element_ref = match controller_browser_element_ref(
            args.selector.as_deref(),
            args.field_id.as_deref(),
            "runtime_tool_slice_browser_submit",
        ) {
            Ok(reference) => reference,
            Err(_) if args.selector.is_none() && args.field_id.is_none() => {
                let status =
                    run_controller_browser_status_tool(self, session_id, slice_id, agent_id)
                        .await?;
                status
                    .payload
                    .pointer("/browser/focusedElement/field_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| DaemonError::LocalTransport {
                        operation: "runtime_tool_slice_browser_submit",
                        message: "the Room browser has no focused element to submit".to_string(),
                    })?
            }
            Err(error) => return Err(error),
        };
        let result = self
            .perform_browser_environment_locator_action_as_agent(
                session_id,
                agent_id,
                &element_ref,
                crate::runtime::browser_controller_action::BrowserLocatorAction::Submit,
                crate::runtime::browser_controller_action::MAX_BROWSER_ACTION_TIMEOUT_MS,
            )
            .await?;
        Ok(controller_browser_action_tool_result(
            slice_id, agent_id, result,
        ))
    }

    pub(super) async fn controller_browser_dialog_tool_result(
        &self,
        session_id: &str,
        slice_id: &str,
        agent_id: &str,
        args: crate::transport::runtime_tools::SliceBrowserDialogArgs,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let action = match args.action.as_str() {
            "accept" => crate::runtime::browser_controller_action::BrowserDialogAction::Accept {
                prompt_text: args.prompt_text,
            },
            "dismiss" => crate::runtime::browser_controller_action::BrowserDialogAction::Dismiss,
            other => {
                return Err(DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_browser_dialog",
                    message: format!("unsupported browser dialog action `{other}`"),
                });
            }
        };
        let environment = ensure_controller_browser_environment(
            self,
            session_id,
            "runtime_tool_slice_browser_dialog",
        )
        .await?;
        let tab_id = environment
            .focused_tab_id
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "runtime_tool_slice_browser_dialog",
                message: "the Room browser has no focused tab".to_string(),
            })?;
        let result = self
            .handle_browser_environment_dialog_as_agent(session_id, agent_id, &tab_id, action)
            .await?;
        Ok(controller_browser_dialog_tool_result(
            slice_id, agent_id, result,
        ))
    }

    pub(super) async fn controller_browser_events_tool_result(
        &self,
        session_id: &str,
        slice_id: &str,
        agent_id: &str,
        args: crate::transport::runtime_tools::SliceBrowserEventsArgs,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        if args.browser_generation == 0
            || args.limit == 0
            || args.limit > crate::runtime::browser_controller_event::MAX_BROWSER_EVENT_POLL_LIMIT
        {
            return Err(DaemonError::LocalTransport {
                operation: "runtime_tool_slice_browser_events",
                message: "browser_generation must be positive and limit must be between 1 and 200"
                    .to_string(),
            });
        }
        ensure_controller_browser_environment(
            self,
            session_id,
            "runtime_tool_slice_browser_events",
        )
        .await?;
        let batch = self
            .poll_browser_environment_events(
                session_id,
                args.browser_generation,
                args.cursor,
                args.limit,
            )
            .await?;
        Ok(controller_browser_events_tool_result(
            slice_id, agent_id, session_id, batch,
        ))
    }

    pub(super) async fn controller_browser_downloads_tool_result(
        &self,
        session_id: &str,
        slice_id: &str,
        agent_id: &str,
        args: crate::transport::runtime_tools::SliceBrowserDownloadsArgs,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let environment = ensure_controller_browser_environment(
            self,
            session_id,
            "runtime_tool_slice_browser_downloads",
        )
        .await?;
        if let Some(cancel) = args.cancel {
            let cancellation =
                crate::runtime::browser_controller_file_transfer::BrowserDownloadCancellation::new(
                    cancel.browser_generation,
                    cancel.guid,
                )
                .map_err(|message| DaemonError::LocalTransport {
                    operation: "runtime_tool_slice_browser_downloads",
                    message,
                })?;
            let execution = self
                .cancel_browser_download_as_agent(session_id, agent_id, cancellation)
                .await?;
            return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({
                    "source": "browser_controller", "slice_id": slice_id, "agent_id": agent_id,
                    "session_id": session_id, "environment_id": execution.environment_id,
                    "runtime_generation": execution.runtime_generation,
                    "action_id": execution.action_id, "actor_id": execution.actor_id,
                    "browser_generation": execution.value.browser_generation,
                    "guid": execution.value.guid,
                    "cancellation_requested": execution.value.cancellation_requested,
                }),
            });
        }
        let tab_id = environment
            .focused_tab_id
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "runtime_tool_slice_browser_downloads",
                message: "the Room browser has no focused tab".to_string(),
            })?;
        let result = self
            .configure_browser_environment_downloads(session_id, &tab_id)
            .await?;
        Ok(controller_browser_downloads_tool_result(
            slice_id, agent_id, result,
        ))
    }

    pub(super) async fn controller_browser_upload_tool_result(
        &self,
        session_id: &str,
        slice_id: &str,
        agent_id: &str,
        args: crate::transport::runtime_tools::SliceBrowserUploadArgs,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        ensure_controller_browser_environment(
            self,
            session_id,
            "runtime_tool_slice_browser_upload",
        )
        .await?;
        let result = self
            .upload_browser_environment_files_as_agent(
                session_id,
                agent_id,
                &args.field_id,
                args.files,
            )
            .await?;
        Ok(controller_browser_upload_tool_result(
            slice_id,
            agent_id,
            result.value,
        ))
    }

    pub(super) async fn controller_browser_permission_tool_result(
        &self,
        session_id: &str,
        slice_id: &str,
        agent_id: &str,
        args: crate::transport::runtime_tools::SliceBrowserPermissionArgs,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let permission = runtime_tool_browser_permission(&args.permission).ok_or_else(|| {
            DaemonError::LocalTransport {
                operation: "runtime_tool_slice_browser_permission",
                message: "unsupported browser permission".to_string(),
            }
        })?;
        let setting = serde_json::from_value::<
            crate::runtime::browser_controller_permission::BrowserPermissionSetting,
        >(serde_json::Value::String(args.setting))
        .map_err(|_| DaemonError::LocalTransport {
            operation: "runtime_tool_slice_browser_permission",
            message: "unsupported browser permission setting".to_string(),
        })?;
        let environment = ensure_controller_browser_environment(
            self,
            session_id,
            "runtime_tool_slice_browser_permission",
        )
        .await?;
        let tab_id = environment
            .focused_tab_id
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "runtime_tool_slice_browser_permission",
                message: "the Room browser has no focused tab".to_string(),
            })?;
        let result = self
            .set_browser_environment_permission(session_id, &tab_id, permission, setting)
            .await?;
        Ok(controller_browser_permission_tool_result(
            slice_id, agent_id, result,
        ))
    }
}

fn runtime_tool_browser_permission(
    value: &str,
) -> Option<crate::runtime::browser_controller_permission::BrowserPermissionName> {
    use crate::runtime::browser_controller_permission::BrowserPermissionName;
    match value {
        "camera" => Some(BrowserPermissionName::Camera),
        "clipboard-read-write" | "clipboard_read_write" => {
            Some(BrowserPermissionName::ClipboardReadWrite)
        }
        "clipboard-sanitized-write" | "clipboard_sanitized_write" => {
            Some(BrowserPermissionName::ClipboardSanitizedWrite)
        }
        "display-capture" | "display_capture" => Some(BrowserPermissionName::DisplayCapture),
        "geolocation" => Some(BrowserPermissionName::Geolocation),
        "local-fonts" | "local_fonts" => Some(BrowserPermissionName::LocalFonts),
        "microphone" => Some(BrowserPermissionName::Microphone),
        "midi" => Some(BrowserPermissionName::Midi),
        "midi-sysex" | "midi_sysex" => Some(BrowserPermissionName::MidiSysex),
        "notifications" => Some(BrowserPermissionName::Notifications),
        _ => None,
    }
}

pub(super) async fn run_controller_browser_status_tool(
    state: &KernelRuntimeState,
    session_id: &str,
    slice_id: &str,
    agent_id: &str,
) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
    Ok(
        capture_controller_browser_status(state, session_id, slice_id, agent_id)
            .await?
            .result,
    )
}

struct ControllerBrowserStatusCapture {
    result: crate::transport::runtime_tools::RuntimeToolResult,
    structured_snapshot:
        Option<crate::runtime::browser_controller_snapshot::RoomBrowserStructuredSnapshot>,
}

async fn capture_controller_browser_status(
    state: &KernelRuntimeState,
    session_id: &str,
    slice_id: &str,
    agent_id: &str,
) -> Result<ControllerBrowserStatusCapture, DaemonError> {
    let environment = ensure_controller_browser_environment(
        state,
        session_id,
        "runtime_tool_slice_browser_status",
    )
    .await?;
    let structured_snapshot = match environment.focused_tab_id.as_deref() {
        Some(tab_id) => Some(
            state
                .capture_browser_environment_snapshot(session_id, tab_id)
                .await?,
        ),
        None => None,
    };
    let focused = environment
        .focused_tab_id
        .as_deref()
        .and_then(|focused_id| environment.tabs.iter().find(|tab| tab.tab_id == focused_id));
    let url = focused.map(|tab| tab.url.as_str()).unwrap_or_default();
    let title = focused.map(|tab| tab.title.as_str()).unwrap_or_default();
    let host = url::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_default();
    let surfaces = controller_browser_status_surfaces(structured_snapshot.as_ref());
    let browser_generation = structured_snapshot
        .as_ref()
        .map(|snapshot| snapshot.browser_generation);
    let browser = controller_browser_status_compatibility(url, &host, title, &surfaces);
    let payload = serde_json::json!({
        "source": "browser_controller",
        "slice_id": slice_id,
        "agent_id": agent_id,
        "session_id": session_id,
        "environment_id": environment.environment_id,
        "runtime_generation": environment.runtime_generation,
        "browser_generation": browser_generation,
        "viewport": environment.viewport,
        "tab_id": focused.map(|tab| tab.tab_id.as_str()),
        "url": url,
        "host": host,
        "title": title,
        "document_revision": focused.map(|tab| tab.document_revision),
        "tabs": environment.tabs,
        "browser": browser,
    });
    Ok(ControllerBrowserStatusCapture {
        result: crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload },
        structured_snapshot,
    })
}

pub(super) async fn ensure_controller_browser_environment(
    state: &KernelRuntimeState,
    session_id: &str,
    operation: &'static str,
) -> Result<crate::session::RoomEnvironmentSnapshot, DaemonError> {
    let viewport = match state.room_environment_snapshot(session_id) {
        Ok(environment) => environment.viewport,
        Err(crate::session::EnvironmentError::EnvironmentNotFound { .. }) => {
            slice_environment_viewport(operation)?
        }
        Err(error) => {
            return Err(DaemonError::LocalTransport {
                operation,
                message: format!("{}: {error:?}", error.code()),
            });
        }
    };
    let environment = state
        .start_room_environment(session_id, viewport)
        .map_err(|error| DaemonError::LocalTransport {
            operation,
            message: format!("{}: {error:?}", error.code()),
        })?;
    if environment.lifecycle == crate::session::EnvironmentLifecycle::Starting {
        state
            .finish_room_environment_controller_start(session_id, operation)
            .await?;
    } else {
        state
            .ensure_browser_controller_process_started(session_id)
            .await?;
        state
            .reconcile_browser_controller_environment(session_id)
            .await?;
    }
    state
        .reconcile_room_environment_actors(session_id, None)
        .map_err(|error| DaemonError::LocalTransport {
            operation,
            message: format!("{}: {error:?}", error.code()),
        })
}

pub(super) async fn run_controller_browser_find_tool(
    state: &KernelRuntimeState,
    session_id: &str,
    slice_id: &str,
    agent_id: &str,
    query: &str,
    kind: &str,
) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
    let status = run_controller_browser_status_tool(state, session_id, slice_id, agent_id).await?;
    let browser_status =
        status
            .payload
            .get("browser")
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "runtime_tool_slice_browser_find",
                message: "controller-backed browser status omitted its compatibility projection"
                    .to_string(),
            })?;
    let browser = controller_browser_find(browser_status, query, kind).map_err(|message| {
        DaemonError::LocalTransport {
            operation: "runtime_tool_slice_browser_find",
            message,
        }
    })?;
    Ok(crate::transport::runtime_tools::RuntimeToolResult {
        ok: true,
        payload: serde_json::json!({
            "source": "browser_controller",
            "slice_id": slice_id,
            "agent_id": agent_id,
            "session_id": session_id,
            "browser": browser,
        }),
    })
}

pub(super) async fn run_controller_browser_text_tool(
    state: &KernelRuntimeState,
    session_id: &str,
    slice_id: &str,
    agent_id: &str,
) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
    let (environment, text) =
        capture_controller_browser_text(state, session_id, "runtime_tool_slice_browser_text")
            .await?;
    Ok(crate::transport::runtime_tools::RuntimeToolResult {
        ok: true,
        payload: serde_json::json!({
            "source": "browser_controller",
            "slice_id": slice_id,
            "agent_id": agent_id,
            "session_id": session_id,
            "environment_id": environment.environment_id,
            "runtime_generation": environment.runtime_generation,
            "tab_id": environment.focused_tab_id,
            "text": text,
        }),
    })
}

pub(super) async fn run_controller_browser_wait_for_text_tool(
    state: &KernelRuntimeState,
    session_id: &str,
    slice_id: &str,
    agent_id: &str,
    query: &str,
    timeout_ms: Option<u64>,
) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
    validate_controller_browser_text_query(query).map_err(|message| {
        DaemonError::LocalTransport {
            operation: "runtime_tool_slice_browser_wait_for_text",
            message,
        }
    })?;
    let timeout_ms = timeout_ms.unwrap_or(10_000).clamp(100, 60_000);
    let timeout = std::time::Duration::from_millis(timeout_ms);
    let started = std::time::Instant::now();
    let environment = ensure_controller_browser_environment(
        state,
        session_id,
        "runtime_tool_slice_browser_wait_for_text",
    )
    .await?;
    let tab_id = environment
        .focused_tab_id
        .as_deref()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "runtime_tool_slice_browser_wait_for_text",
            message: "the Room browser has no focused tab".to_string(),
        })?
        .to_string();
    loop {
        let text = capture_controller_browser_text_from_tab(state, session_id, &tab_id).await?;
        let waited_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        if text.contains(query) {
            return Ok(controller_browser_wait_for_text_result(
                slice_id,
                agent_id,
                environment.clone(),
                query,
                waited_ms,
                true,
                timeout_ms,
            ));
        }
        let elapsed = started.elapsed();
        if elapsed >= timeout {
            return Ok(controller_browser_wait_for_text_result(
                slice_id,
                agent_id,
                environment.clone(),
                query,
                waited_ms,
                false,
                timeout_ms,
            ));
        }
        tokio::time::sleep(
            std::time::Duration::from_millis(250).min(timeout.saturating_sub(elapsed)),
        )
        .await;
    }
}

pub(super) async fn capture_controller_browser_text(
    state: &KernelRuntimeState,
    session_id: &str,
    operation: &'static str,
) -> Result<(crate::session::RoomEnvironmentSnapshot, String), DaemonError> {
    let environment = ensure_controller_browser_environment(state, session_id, operation).await?;
    let tab_id =
        environment
            .focused_tab_id
            .as_deref()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation,
                message: "the Room browser has no focused tab".to_string(),
            })?;
    let text = capture_controller_browser_text_from_tab(state, session_id, tab_id).await?;
    Ok((environment, text))
}

async fn capture_controller_browser_text_from_tab(
    state: &KernelRuntimeState,
    session_id: &str,
    tab_id: &str,
) -> Result<String, DaemonError> {
    let snapshot = state
        .capture_browser_environment_snapshot(session_id, tab_id)
        .await?;
    Ok(controller_browser_document_text(&snapshot))
}

fn slice_environment_viewport(
    operation: &'static str,
) -> Result<crate::session::CanonicalViewport, DaemonError> {
    let geometry = std::env::var("CHARIOX_SLICE_SCREEN_GEOMETRY")
        .unwrap_or_else(|_| "1280x800x24".to_string());
    let mut parts = geometry.split('x');
    let width = parts.next().and_then(|value| value.parse::<u32>().ok());
    let height = parts.next().and_then(|value| value.parse::<u32>().ok());
    if width.is_none() || height.is_none() || parts.next().is_none() || parts.next().is_some() {
        return Err(DaemonError::LocalTransport {
            operation,
            message: "CHARIOX_SLICE_SCREEN_GEOMETRY must use WIDTHxHEIGHTxDEPTH".to_string(),
        });
    }
    let (width, height) = (
        width.expect("validated width"),
        height.expect("validated height"),
    );
    crate::session::CanonicalViewport::new(width, height, 1, width, height).map_err(|_| {
        DaemonError::LocalTransport {
            operation,
            message: "CHARIOX_SLICE_SCREEN_GEOMETRY must contain positive dimensions".to_string(),
        }
    })
}
