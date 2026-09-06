use crate::runtime::browser_controller_process::{
    BrowserControllerProcessStore, CONTROLLER_RESTARTED_BEFORE_OPERATION,
};
use crate::transport::room_browser_controller::{
    RoomBrowserControllerCommand as Command, RoomBrowserControllerResult as Response,
};

use super::*;

impl KernelRuntimeState {
    pub(crate) fn browser_controller_enabled_for_room(&self, session_id: &str) -> bool {
        self.owned
            .slice_store
            .environment_slice(session_id)
            .is_some()
            || self.browser_controller_process_enabled()
            || self
                .owned
                .config_projection
                .snapshot()
                .room_environment_worker_binding
                .is_some()
    }

    pub(super) async fn room_browser_controller_command(
        &self,
        session_id: &str,
        command: Command,
    ) -> Result<Response, DaemonError> {
        let admitted_mutation_command = matches!(
            &command,
            Command::Action { .. }
                | Command::Upload { .. }
                | Command::Tab { .. }
                | Command::History { .. }
                | Command::Dialog { .. }
                | Command::Navigate { .. }
                | Command::ComputerInput { .. }
                | Command::CancelDownload { .. }
        );
        let response = if let Some(slice) = self.owned.slice_store.environment_slice(session_id) {
            // Keep the relay client's large future off callers' async stacks. Local
            // controller operations stay allocation-free; only the remote boundary
            // owns this boxed transport future.
            Box::pin(self.route_room_browser_controller_command(session_id, slice, command)).await?
        } else {
            if self
                .owned
                .config_projection
                .snapshot()
                .room_environment_worker_binding
                .is_some()
            {
                return Err(controller_route_error(
                    "browser_controller_scope_denied: provisioned slice controller requires the home Room relay path",
                ));
            }
            execute_local(
                self.owned.browser_controller_processes.clone(),
                self.owned.computer_input_executions.clone(),
                session_id,
                command,
            )
            .await?
        };
        match response {
            Response::RecoveryRequired { process } if admitted_mutation_command => {
                Err(DaemonError::BrowserControllerRecoveryRequired {
                    runtime_generation: process.runtime_generation,
                })
            }
            Response::RecoveryRequired { process } => {
                Box::pin(self.recover_browser_controller_after_restart(
                    session_id,
                    process.runtime_generation,
                ))
                .await?;
                Err(controller_route_error(
                    CONTROLLER_RESTARTED_BEFORE_OPERATION,
                ))
            }
            response => Ok(response),
        }
    }

    async fn route_room_browser_controller_command(
        &self,
        session_id: &str,
        slice: crate::slice::SliceRecord,
        command: Command,
    ) -> Result<Response, DaemonError> {
        // The original action retains its operation guard until terminal proof.
        // Cancellation must not wait for that very action to release the guard.
        let _guard = if matches!(&command, Command::CancelAction { .. }) {
            None
        } else {
            Some(self.owned.slice_store.guard_environment_use(
                &slice.id,
                Some(session_id),
                "browser_controller.route",
            )?)
        };
        let config = self.owned.config_projection.snapshot();
        let config = config.slice_relay_override(&slice).unwrap_or(config);
        let target = ClientTarget {
            daemon_id: slice.worker_kernel_id.clone(),
            daemon_alias: slice
                .worker_kernel_id
                .is_none()
                .then(|| slice.worker_kernel_ref.clone()),
        };
        let recovery = receipt_recovery_command(&command);
        let request = |command| RelayPeerRequest::RoomBrowserController {
            session_id: session_id.to_string(),
            slice_id: slice.id.clone(),
            command,
        };
        let send = |target, command| async {
            let timeout = match &command {
                Command::ComputerInput {
                    action: crate::transport::room_browser_controller::RoomComputerInputAction::KeyboardText { input },
                    ..
                } => Duration::from_millis(
                    crate::runtime::computer_input_action::keyboard_text_timeout_ms(input.as_str()) + 10_000,
                ),
                _ => Duration::from_secs(15),
            };
            match self.connected_relay_state_for_config(&config).await {
                Some(relay_state) => {
                    crate::transport::relay_client::send_peer_request_via_connected_relay_with_timeout(
                        &config,
                        &relay_state,
                        target,
                        request(command),
                        timeout,
                    )
                    .await
                }
                None => {
                    crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
                        &config,
                        target,
                        request(command),
                        timeout,
                    )
                    .await
                }
            }
        };
        let first = send(target.clone(), command.clone()).await;
        let response = match first {
            Ok(response) => response,
            Err(first_error) if recovery.is_some() => {
                send(target, recovery.expect("action recovery command"))
                .await.map_err(|retry_error| controller_route_error(&format!(
                    "browser action result remained unavailable after non-mutating receipt recovery: {retry_error}; initial delivery error: {first_error}"
                )))?
            }
            Err(error) => return Err(error),
        };
        match response {
            RelayPeerResponse::RoomBrowserController {
                session_id: returned_room,
                slice_id,
                result,
            } if returned_room == session_id && slice_id == slice.id => Ok(result),
            _ => Err(controller_route_error(
                "worker returned a mismatched controller response",
            )),
        }
    }

    pub(crate) async fn execute_bound_room_browser_controller(
        &self,
        authenticated_kernel_id: &str,
        authenticated_public_key: &str,
        session_id: &str,
        slice_id: &str,
        command: Command,
    ) -> Result<Response, DaemonError> {
        let config = self.owned.config_projection.snapshot();
        let permitted = config
            .room_environment_worker_binding
            .as_ref()
            .is_some_and(|binding| {
                binding.permits(
                    authenticated_kernel_id,
                    authenticated_public_key,
                    session_id,
                    slice_id,
                )
            });
        if !permitted {
            return Err(controller_route_error("browser_controller_scope_denied: peer or Room does not match the provisioned slice binding"));
        }
        if !matches!(
            &command,
            Command::ComputerInput { .. } | Command::ComputerClipboardRead { .. }
        ) && !self.browser_controller_process_enabled()
        {
            return Err(controller_route_error(
                "browser_controller_unavailable: slice has no configured controller",
            ));
        }
        execute_local(
            self.owned.browser_controller_processes.clone(),
            self.owned.computer_input_executions.clone(),
            session_id,
            command,
        )
        .await
    }
}

fn receipt_recovery_command(command: &Command) -> Option<Command> {
    match command {
        Command::Upload {
            execution_id,
            target_id,
            document_id,
            node_ref,
            files,
        } => Some(Command::RecoverUpload {
            execution_id: execution_id.clone(),
            target_id: target_id.clone(),
            document_id: document_id.clone(),
            node_ref: node_ref.clone(),
            files: files.clone(),
        }),
        Command::Action {
            execution_id,
            target_id,
            document_id,
            node_ref,
            action,
            timeout_ms,
        } => Some(Command::RecoverAction {
            execution_id: execution_id.clone(),
            target_id: target_id.clone(),
            document_id: document_id.clone(),
            node_ref: node_ref.clone(),
            action: action.clone(),
            timeout_ms: *timeout_ms,
        }),
        _ => None,
    }
}

async fn execute_local(
    processes: BrowserControllerProcessStore,
    computer_input_executions: crate::runtime::computer_input_execution::ComputerInputExecutionStore,
    session_id: &str,
    command: Command,
) -> Result<Response, DaemonError> {
    let command = match command {
        Command::ComputerInput {
            action_id,
            actor_id,
            runtime_generation,
            viewport_revision,
            desktop_pixel_width,
            desktop_pixel_height,
            action,
        } => {
            if action_id.trim().is_empty()
                || actor_id.trim().is_empty()
                || runtime_generation == 0
                || viewport_revision == 0
            {
                return Err(controller_route_error(
                    "environment_input_invalid_authority_context",
                ));
            }
            let execution = computer_input_executions
                .begin(session_id, &action_id)
                .map_err(controller_route_error)?;
            let cancellation = execution.cancellation();
            let input_result = match action {
                crate::transport::room_browser_controller::RoomComputerInputAction::PointerMove {
                    x,
                    y,
                } => super::tool_dispatch::run_room_pointer_move(
                    x,
                    y,
                    desktop_pixel_width,
                    desktop_pixel_height,
                    cancellation,
                )
                .await,
                crate::transport::room_browser_controller::RoomComputerInputAction::PointerDrag {
                    from_x,
                    from_y,
                    to_x,
                    to_y,
                    button,
                } => {
                    super::tool_dispatch::run_room_pointer_drag(
                        from_x,
                        from_y,
                        to_x,
                        to_y,
                        button,
                        desktop_pixel_width,
                        desktop_pixel_height,
                        cancellation,
                    )
                    .await
                }
                crate::transport::room_browser_controller::RoomComputerInputAction::PointerScroll {
                    x,
                    y,
                    horizontal_steps,
                    vertical_steps,
                } => {
                    super::tool_dispatch::run_room_pointer_scroll(
                        x,
                        y,
                        horizontal_steps,
                        vertical_steps,
                        desktop_pixel_width,
                        desktop_pixel_height,
                        cancellation,
                    )
                    .await
                }
                crate::transport::room_browser_controller::RoomComputerInputAction::KeyboardText {
                    input,
                } => super::tool_dispatch::run_room_keyboard_text(input, cancellation).await,
                crate::transport::room_browser_controller::RoomComputerInputAction::KeyboardKey {
                    input,
                    repeat,
                } => {
                    super::tool_dispatch::run_room_keyboard_key(input, repeat, cancellation).await
                }
                crate::transport::room_browser_controller::RoomComputerInputAction::ClipboardWrite {
                    text,
                } => super::tool_dispatch::run_room_clipboard_write(text, cancellation).await,
                crate::transport::room_browser_controller::RoomComputerInputAction::PointerClick {
                    x,
                    y,
                    button,
                    click_count,
                } => {
                    super::tool_dispatch::run_room_pointer_click(
                        x,
                        y,
                        button,
                        click_count,
                        desktop_pixel_width,
                        desktop_pixel_height,
                        cancellation,
                    )
                    .await
                }
                crate::transport::room_browser_controller::RoomComputerInputAction::SecretText {
                    input,
                } => super::tool_dispatch::run_room_secret_text_input(input, cancellation).await,
            };
            if matches!(
                input_result,
                Err(DaemonError::BrowserControllerActionCancelled { .. })
            ) {
                super::tool_dispatch::reset_room_computer_input().await?;
                return Ok(Response::ActionCancelled {
                    controller_fenced: false,
                });
            }
            input_result?;
            return Ok(Response::ComputerInputApplied { action_id });
        }
        Command::ComputerClipboardRead {
            actor_id,
            runtime_generation,
        } => {
            if actor_id.trim().is_empty() || runtime_generation == 0 {
                return Err(controller_route_error(
                    "environment_clipboard_invalid_authority_context",
                ));
            }
            let content = super::tool_dispatch::run_room_clipboard_read().await?;
            return Ok(Response::ComputerClipboard { content });
        }
        command => command,
    };
    let session_id = session_id.to_string();
    let recovery_processes = processes.clone();
    let result = tokio::task::spawn_blocking(move || match command {
        Command::CancelAction { execution_id } => {
            let accepted = computer_input_executions.cancel(&session_id, &execution_id)
                || processes.cancel_browser_action(&session_id, &execution_id);
            Ok(Response::CancellationRequested { accepted })
        }
        Command::Acquire => processes
            .acquire(&session_id)
            .map(|snapshot| Response::Process { snapshot }),
        Command::Release => processes
            .release(&session_id)
            .map(|snapshot| Response::Process { snapshot }),
        Command::Reconcile { viewport } => processes
            .reconcile_browser(&session_id, &viewport)
            .map(|reconciliation| Response::Reconciled { reconciliation }),
        Command::Snapshot {
            target_id,
            document_id,
        } => processes
            .capture_browser_snapshot(&session_id, &target_id, &document_id)
            .map(|snapshot| Response::Snapshot { snapshot }),
        Command::Tab {
            target_id,
            document_id,
            action,
        } => processes
            .manage_browser_tab(&session_id, &target_id, &document_id, action)
            .map(|result| Response::Tab { result }),
        Command::History {
            target_id,
            document_id,
            action,
        } => processes
            .navigate_browser_history(&session_id, &target_id, &document_id, action)
            .map(|result| Response::History { result }),
        Command::Navigate {
            target_id,
            document_id,
            url,
        } => processes
            .navigate_browser(&session_id, &target_id, &document_id, url.as_str())
            .map(|result| Response::Navigation { result }),
        Command::Wait {
            target_id,
            document_id,
            wait,
            timeout_ms,
        } => processes
            .wait_for_browser(&session_id, &target_id, &document_id, &wait, timeout_ms)
            .map(|result| Response::Wait { result }),
        Command::Dialog {
            target_id,
            document_id,
            action,
        } => processes
            .handle_browser_dialog(&session_id, &target_id, &document_id, &action)
            .map(|result| Response::Dialog { result }),
        Command::ConfigureDownloads {
            target_id,
            document_id,
        } => processes
            .configure_browser_downloads(&session_id, &target_id, &document_id)
            .map(|result| Response::Downloads { result }),
        Command::CancelDownload { cancellation } => processes
            .cancel_browser_download(&session_id, &cancellation)
            .map(|result| Response::DownloadCancellation { result }),
        Command::Upload {
            execution_id,
            target_id,
            document_id,
            node_ref,
            files,
        } => processes.perform_cancellable_browser_upload(
            &session_id,
            &execution_id,
            &target_id,
            &document_id,
            &node_ref,
            &files,
        ),
        Command::RecoverUpload {
            execution_id,
            target_id,
            document_id,
            node_ref,
            files,
        } => processes.recover_cancellable_browser_upload(
            &session_id,
            &execution_id,
            &target_id,
            &document_id,
            &node_ref,
            &files,
        ),
        Command::Permission {
            target_id,
            document_id,
            permission,
            setting,
        } => processes
            .set_browser_permission(&session_id, &target_id, &document_id, permission, setting)
            .map(|result| Response::Permission { result }),
        Command::PollEvents {
            browser_generation,
            cursor,
            limit,
        } => processes
            .poll_browser_events(&session_id, browser_generation, cursor, limit)
            .map(|batch| Response::Events { batch }),
        Command::Action {
            execution_id,
            target_id,
            document_id,
            node_ref,
            action,
            timeout_ms,
        } => processes.perform_cancellable_browser_action(
            &session_id,
            &execution_id,
            &target_id,
            &document_id,
            &node_ref,
            &action,
            timeout_ms,
        ),
        Command::RecoverAction {
            execution_id,
            target_id,
            document_id,
            node_ref,
            action,
            timeout_ms,
        } => processes.recover_cancellable_browser_action(
            &session_id,
            &execution_id,
            &target_id,
            &document_id,
            &node_ref,
            &action,
            timeout_ms,
        ),
        Command::ComputerInput { .. } => {
            unreachable!("Computer input executes before the blocking controller path")
        }
        Command::ComputerClipboardRead { .. } => {
            unreachable!("Computer clipboard reads execute before the blocking controller path")
        }
    })
    .await
    .map_err(|error| controller_route_error(&error.to_string()))?;
    match result {
        Err(message) if message == CONTROLLER_RESTARTED_BEFORE_OPERATION => {
            let process = recovery_processes
                .snapshot()
                .map_err(|error| controller_route_error(&error))?
                .ok_or_else(|| controller_route_error(&message))?;
            Ok(Response::RecoveryRequired { process })
        }
        result => result.map_err(|message| controller_route_error(&message)),
    }
}

pub(super) fn controller_route_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "browser_controller.route",
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_recovery_preserves_identity_and_never_redispatches_upload() {
        let command = Command::Upload {
            execution_id: "00000000000000000000000000000001".into(),
            target_id: "target-1".into(),
            document_id: "doc-1".into(),
            node_ref: "backend:1".into(),
            files: crate::runtime::browser_controller_file_transfer::BrowserUploadFiles::new(vec![
                "/workspace/report.txt".into(),
            ])
            .unwrap(),
        };
        let recovery = receipt_recovery_command(&command).unwrap();
        let mut expected = serde_json::to_value(&command).unwrap();
        expected["kind"] = "recover_upload".into();
        assert_eq!(serde_json::to_value(&recovery).unwrap(), expected);
        assert_eq!(receipt_recovery_command(&recovery), None);
    }

    #[test]
    fn physical_computer_input_has_no_transport_replay_command() {
        let command = Command::ComputerInput {
            action_id: "action-1".to_string(),
            actor_id: "user:owner-1".to_string(),
            runtime_generation: 1,
            viewport_revision: 1,
            desktop_pixel_width: 1280,
            desktop_pixel_height: 800,
            action:
                crate::transport::room_browser_controller::RoomComputerInputAction::PointerClick {
                    x: 20,
                    y: 30,
                    button:
                        crate::transport::room_browser_controller::RoomComputerPointerButton::Left,
                    click_count: 1,
                },
        };

        assert_eq!(receipt_recovery_command(&command), None);
    }
}
