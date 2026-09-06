use crate::error::DaemonError;
use crate::runtime::browser_controller_event::{RoomBrowserEvent, RoomBrowserEventBatch};
use crate::runtime::browser_controller_process::BrowserControllerProcessSnapshot;
use crate::session::{
    EnvironmentComponent, EnvironmentComponentHealthState, EnvironmentError, EnvironmentLifecycle,
    RoomEnvironmentSnapshot,
};

use super::room_browser_controller::controller_route_error;
use super::KernelRuntimeState;
use crate::transport::room_browser_controller::{
    RoomBrowserControllerCommand, RoomBrowserControllerResult,
};

impl KernelRuntimeState {
    #[cfg(test)]
    pub(crate) fn set_browser_controller_process_store_for_test(
        &mut self,
        processes: crate::runtime::browser_controller_process::BrowserControllerProcessStore,
    ) {
        self.owned.browser_controller_processes = processes;
        self.owned
            .browser_controller_generations
            .lock()
            .expect("browser controller generation lock poisoned")
            .clear();
    }

    #[cfg(test)]
    pub(crate) fn test_forget_completed_browser_action_receipts(&self) {
        self.owned
            .browser_controller_processes
            .test_forget_completed_browser_actions();
    }

    #[cfg(test)]
    pub(crate) fn test_browser_action_cancellation_request_count(&self) -> usize {
        self.owned
            .browser_controller_processes
            .test_browser_action_cancellation_request_count()
    }

    pub(crate) fn browser_controller_process_enabled(&self) -> bool {
        self.owned.browser_controller_processes.is_enabled()
    }

    pub(crate) async fn ensure_browser_controller_process_started(
        &self,
        session_id: &str,
    ) -> Result<Option<BrowserControllerProcessSnapshot>, DaemonError> {
        let RoomBrowserControllerResult::Process { snapshot } = self
            .room_browser_controller_command(session_id, RoomBrowserControllerCommand::Acquire)
            .await?
        else {
            return Err(controller_route_error(
                "unexpected controller start response",
            ));
        };
        if let Some(snapshot) = snapshot.as_ref() {
            self.observe_browser_controller_generation(session_id, snapshot.runtime_generation)?;
        }
        Ok(snapshot)
    }

    pub(crate) async fn finish_room_environment_controller_start(
        &self,
        session_id: &str,
        operation: &'static str,
    ) -> Result<RoomEnvironmentSnapshot, DaemonError> {
        if !self.browser_controller_enabled_for_room(session_id) {
            return self
                .room_environment_snapshot(session_id)
                .map_err(|error| environment_runtime_error(operation, error));
        }
        self.update_room_environment_component_health(
            session_id,
            EnvironmentComponent::BrowserController,
            EnvironmentComponentHealthState::Starting,
            None,
        )
        .map_err(|error| environment_runtime_error(operation, error))?;
        if let Err(error) = self
            .ensure_browser_controller_process_started(session_id)
            .await
        {
            let _ = self.update_room_environment_component_health(
                session_id,
                EnvironmentComponent::BrowserController,
                EnvironmentComponentHealthState::Unavailable,
                Some("controller_start_failed"),
            );
            let _ = self.transition_room_environment(session_id, EnvironmentLifecycle::Failed);
            return Err(error);
        }
        self.update_room_environment_component_health(
            session_id,
            EnvironmentComponent::BrowserController,
            EnvironmentComponentHealthState::Ready,
            None,
        )
        .map_err(|error| environment_runtime_error(operation, error))?;
        self.update_room_environment_component_health(
            session_id,
            EnvironmentComponent::Browser,
            EnvironmentComponentHealthState::Starting,
            None,
        )
        .map_err(|error| environment_runtime_error(operation, error))?;
        match self
            .reconcile_browser_controller_environment(session_id)
            .await
        {
            Ok(_) => {
                let environment = self
                    .update_room_environment_component_health(
                        session_id,
                        EnvironmentComponent::Browser,
                        EnvironmentComponentHealthState::Ready,
                        None,
                    )
                    .map_err(|error| environment_runtime_error(operation, error))?;
                self.complete_bound_slice_computer_start(session_id, environment, operation)
            }
            Err(error) => {
                let _ = self.update_room_environment_component_health(
                    session_id,
                    EnvironmentComponent::Browser,
                    EnvironmentComponentHealthState::Unavailable,
                    Some("browser_reconcile_failed"),
                );
                let _ = self.transition_room_environment(session_id, EnvironmentLifecycle::Failed);
                Err(error)
            }
        }
    }

    fn complete_bound_slice_computer_start(
        &self,
        session_id: &str,
        environment: RoomEnvironmentSnapshot,
        operation: &'static str,
    ) -> Result<RoomEnvironmentSnapshot, DaemonError> {
        let Some(slice) = self.owned.slice_store.environment_slice(session_id) else {
            return Ok(environment);
        };
        if slice.status != crate::slice::SliceStatus::Running
            || slice.display_mode != crate::slice::SliceDisplayMode::Headed
        {
            return Ok(environment);
        }
        // A headed slice is not published as running until its provisioner has
        // started and health-checked both the desktop and selected streamer,
        // then discovered the worker kernel. Controller reconciliation above
        // proves that this is still the bound worker for the Room.
        self.update_room_environment_component_health(
            session_id,
            EnvironmentComponent::Desktop,
            EnvironmentComponentHealthState::Ready,
            None,
        )
        .map_err(|error| environment_runtime_error(operation, error))?;
        let environment = self
            .update_room_environment_component_health(
                session_id,
                EnvironmentComponent::Streamer,
                EnvironmentComponentHealthState::Ready,
                None,
            )
            .map_err(|error| environment_runtime_error(operation, error))?;
        if environment.lifecycle == EnvironmentLifecycle::Ready {
            return Ok(environment);
        }
        self.transition_room_environment(session_id, EnvironmentLifecycle::Ready)
            .map_err(|error| environment_runtime_error(operation, error))
    }

    pub(crate) async fn reconcile_browser_controller_environment(
        &self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, DaemonError> {
        let viewport = self
            .room_environment_snapshot(session_id)
            .map_err(|error| environment_runtime_error("browser_controller.reconcile", error))?
            .viewport;
        let RoomBrowserControllerResult::Reconciled { reconciliation } = self
            .room_browser_controller_command(
                session_id,
                RoomBrowserControllerCommand::Reconcile { viewport },
            )
            .await?
        else {
            return Err(controller_route_error(
                "unexpected controller reconcile response",
            ));
        };
        let Some(reconciliation) = reconciliation else {
            return self
                .room_environment_snapshot(session_id)
                .map_err(|error| environment_runtime_error("browser_controller.reconcile", error));
        };
        let controller_recovery_pending = self.observe_browser_controller_generation(
            session_id,
            reconciliation.process.runtime_generation,
        )?;
        let focused_target_id = reconciliation.browser.focused_target_id.clone();
        let tabs = reconciliation
            .browser
            .tabs
            .into_iter()
            .map(|tab| crate::session::EnvironmentTabObservation {
                runtime_target_id: tab.target_id,
                document_id: tab.document_id,
                url: tab.url,
                title: tab.title,
            })
            .collect();
        let mut environment = self
            .reconcile_room_environment_controller_tabs(
                session_id,
                tabs,
                focused_target_id.as_deref(),
            )
            .map_err(|error| environment_runtime_error("browser_controller.reconcile", error))?;
        if controller_recovery_pending {
            self.update_room_environment_component_health(
                session_id,
                EnvironmentComponent::BrowserController,
                EnvironmentComponentHealthState::Ready,
                None,
            )
            .map_err(|error| environment_runtime_error("browser_controller.recover", error))?;
            self.update_room_environment_component_health(
                session_id,
                EnvironmentComponent::Browser,
                EnvironmentComponentHealthState::Ready,
                None,
            )
            .map_err(|error| environment_runtime_error("browser_controller.recover", error))?;
            environment = self
                .complete_room_environment_browser_controller_recovery(session_id)
                .map_err(|error| environment_runtime_error("browser_controller.recover", error))?;
            self.complete_browser_controller_generation_recovery(session_id)?;
        }
        Ok(environment)
    }

    pub(crate) async fn capture_browser_environment_snapshot(
        &self,
        session_id: &str,
        tab_id: &str,
    ) -> Result<
        crate::runtime::browser_controller_snapshot::RoomBrowserStructuredSnapshot,
        DaemonError,
    > {
        let environment = self
            .room_environment_snapshot(session_id)
            .map_err(|error| environment_runtime_error("browser_controller.snapshot", error))?;
        let binding = self
            .room_environment_controller_tab_binding(session_id, tab_id)
            .map_err(|error| environment_runtime_error("browser_controller.snapshot", error))?;
        let RoomBrowserControllerResult::Snapshot {
            snapshot: Some(controller_snapshot),
        } = self
            .room_browser_controller_command(
                session_id,
                RoomBrowserControllerCommand::Snapshot {
                    target_id: binding.runtime_target_id.clone(),
                    document_id: binding.document_id.clone(),
                },
            )
            .await?
        else {
            return Err(controller_route_error(
                "browser controller did not return a snapshot",
            ));
        };
        controller_snapshot
            .validate(&binding.runtime_target_id, &binding.document_id)
            .map_err(|message| controller_route_error(&message))?;
        let references = self
            .register_room_environment_element_references(
                session_id,
                tab_id,
                environment.runtime_generation,
                binding.document_revision,
                controller_snapshot.controller_node_refs(),
            )
            .map_err(|error| environment_runtime_error("browser_controller.snapshot", error))?;
        controller_snapshot
            .into_room_snapshot(
                session_id.to_string(),
                environment.environment_id,
                environment.runtime_generation,
                tab_id.to_string(),
                binding.document_revision,
                &references,
            )
            .map_err(|message| DaemonError::LocalTransport {
                operation: "browser_controller.snapshot",
                message,
            })
    }

    pub(crate) async fn manage_browser_environment_tab(
        &self,
        session_id: &str,
        tab_id: &str,
        action: crate::runtime::browser_controller_tab::BrowserTabAction,
    ) -> Result<RoomEnvironmentSnapshot, DaemonError> {
        let binding = self
            .room_environment_controller_tab_binding(session_id, tab_id)
            .map_err(|error| environment_runtime_error("browser_controller.tab", error))?;
        let RoomBrowserControllerResult::Tab {
            result: Some(result),
        } = self
            .room_browser_controller_command(
                session_id,
                RoomBrowserControllerCommand::Tab {
                    target_id: binding.runtime_target_id.clone(),
                    document_id: binding.document_id.clone(),
                    action,
                },
            )
            .await?
        else {
            return Err(controller_route_error(
                "browser controller did not return a tab operation result",
            ));
        };
        result
            .validate(&binding.runtime_target_id, &binding.document_id, action)
            .map_err(|message| controller_route_error(&message))?;
        self.reconcile_browser_controller_environment(session_id)
            .await
    }

    pub(crate) async fn manage_browser_environment_tab_as_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        tab_id: &str,
        action: crate::runtime::browser_controller_tab::BrowserTabAction,
    ) -> Result<super::BrowserControllerActionExecution<RoomEnvironmentSnapshot>, DaemonError> {
        self.reconcile_browser_controller_environment(session_id)
            .await?;
        let binding = self
            .room_environment_controller_tab_binding(session_id, tab_id)
            .map_err(|error| environment_runtime_error("browser_controller.tab", error))?;
        self.execute_browser_tab_mutation_as_agent(
            session_id,
            agent_id,
            tab_id,
            binding.document_revision,
            match action {
                crate::runtime::browser_controller_tab::BrowserTabAction::Activate => {
                    "browser_tab_activate"
                }
                crate::runtime::browser_controller_tab::BrowserTabAction::Close => {
                    "browser_tab_close"
                }
            },
            None,
            self.manage_browser_environment_tab(session_id, tab_id, action),
        )
        .await
    }

    pub(crate) async fn navigate_browser_environment_history(
        &self,
        session_id: &str,
        tab_id: &str,
        action: crate::runtime::browser_controller_history::BrowserHistoryAction,
    ) -> Result<RoomEnvironmentSnapshot, DaemonError> {
        let binding = self
            .room_environment_controller_tab_binding(session_id, tab_id)
            .map_err(|error| environment_runtime_error("browser_controller.history", error))?;
        let RoomBrowserControllerResult::History {
            result: Some(result),
        } = self
            .room_browser_controller_command(
                session_id,
                RoomBrowserControllerCommand::History {
                    target_id: binding.runtime_target_id.clone(),
                    document_id: binding.document_id,
                    action,
                },
            )
            .await?
        else {
            return Err(controller_route_error(
                "browser controller did not return a history operation result",
            ));
        };
        result
            .validate(&binding.runtime_target_id, action)
            .map_err(|message| controller_route_error(&message))?;
        self.reconcile_browser_controller_environment(session_id)
            .await
    }

    pub(crate) async fn navigate_browser_environment_history_as_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        tab_id: &str,
        action: crate::runtime::browser_controller_history::BrowserHistoryAction,
    ) -> Result<super::BrowserControllerActionExecution<RoomEnvironmentSnapshot>, DaemonError> {
        self.reconcile_browser_controller_environment(session_id)
            .await?;
        let binding = self
            .room_environment_controller_tab_binding(session_id, tab_id)
            .map_err(|error| environment_runtime_error("browser_controller.history", error))?;
        self.execute_browser_mutation_as_agent(
            session_id,
            agent_id,
            tab_id,
            binding.document_revision,
            match action {
                crate::runtime::browser_controller_history::BrowserHistoryAction::Back => {
                    "browser_history_back"
                }
                crate::runtime::browser_controller_history::BrowserHistoryAction::Forward => {
                    "browser_history_forward"
                }
                crate::runtime::browser_controller_history::BrowserHistoryAction::Reload => {
                    "browser_history_reload"
                }
            },
            None,
            self.navigate_browser_environment_history(session_id, tab_id, action),
        )
        .await
    }

    pub(crate) async fn perform_browser_environment_locator_action(
        &self,
        session_id: &str,
        element_ref: &str,
        execution_id: &str,
        action: crate::runtime::browser_controller_action::BrowserLocatorAction,
        timeout_ms: u64,
    ) -> Result<crate::runtime::browser_controller_action::RoomBrowserActionResult, DaemonError>
    {
        action
            .validate()
            .and_then(|_| {
                crate::runtime::browser_controller_action::validate_browser_action_timeout(
                    timeout_ms,
                )
            })
            .map_err(|message| DaemonError::LocalTransport {
                operation: "browser_controller.action",
                message,
            })?;
        let environment = self
            .room_environment_snapshot(session_id)
            .map_err(|error| environment_runtime_error("browser_controller.action", error))?;
        let element = self
            .resolve_room_environment_element_reference(session_id, element_ref)
            .map_err(|error| environment_runtime_error("browser_controller.action", error))?;
        let binding = self
            .room_environment_controller_tab_binding(session_id, &element.tab_id)
            .map_err(|error| environment_runtime_error("browser_controller.action", error))?;
        if element.runtime_generation != environment.runtime_generation
            || element.document_revision != binding.document_revision
        {
            return Err(DaemonError::LocalTransport {
                operation: "browser_controller.action",
                message: "browser element reference became stale before dispatch".to_string(),
            });
        }

        let action_kind = action.kind();
        let response = self
            .room_browser_controller_command(
                session_id,
                RoomBrowserControllerCommand::Action {
                    execution_id: execution_id.to_string(),
                    target_id: binding.runtime_target_id.clone(),
                    document_id: binding.document_id.clone(),
                    node_ref: element.controller_node_ref.clone(),
                    action,
                    timeout_ms,
                },
            )
            .await?;
        let result = match response {
            RoomBrowserControllerResult::Action {
                result: Some(result),
            } => result,
            RoomBrowserControllerResult::ActionCancelled { controller_fenced } => {
                return Err(DaemonError::BrowserControllerActionCancelled { controller_fenced })
            }
            _ => {
                return Err(controller_route_error(
                    "browser controller did not return an action result",
                ))
            }
        };
        result
            .validate(
                &binding.runtime_target_id,
                &binding.document_id,
                action_kind,
            )
            .map_err(|message| controller_route_error(&message))?;

        Ok(result.into_room_result(
            session_id.to_string(),
            environment.environment_id,
            environment.runtime_generation,
            element.tab_id,
            element.document_revision,
            element_ref.to_string(),
        ))
    }

    pub(crate) async fn recover_browser_controller_after_fence(
        &self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, DaemonError> {
        self.update_room_environment_component_health(
            session_id,
            EnvironmentComponent::BrowserController,
            EnvironmentComponentHealthState::Starting,
            Some("controller_fenced"),
        )
        .map_err(|error| environment_runtime_error("browser_controller.recover", error))?;
        self.update_room_environment_component_health(
            session_id,
            EnvironmentComponent::Browser,
            EnvironmentComponentHealthState::Starting,
            None,
        )
        .map_err(|error| environment_runtime_error("browser_controller.recover", error))?;
        let recovery = match self
            .ensure_browser_controller_process_started(session_id)
            .await
        {
            Ok(_) => {
                self.reconcile_browser_controller_environment(session_id)
                    .await
            }
            Err(error) => Err(error),
        };
        match recovery {
            Ok(environment) => Ok(environment),
            Err(error) => {
                let _ = self.update_room_environment_component_health(
                    session_id,
                    EnvironmentComponent::BrowserController,
                    EnvironmentComponentHealthState::Unavailable,
                    Some("controller_recovery_failed"),
                );
                let _ = self.update_room_environment_component_health(
                    session_id,
                    EnvironmentComponent::Browser,
                    EnvironmentComponentHealthState::Unavailable,
                    Some("controller_recovery_failed"),
                );
                let _ =
                    self.transition_room_environment(session_id, EnvironmentLifecycle::Degraded);
                Err(error)
            }
        }
    }

    pub(crate) async fn recover_browser_controller_after_restart(
        &self,
        session_id: &str,
        runtime_generation: u64,
    ) -> Result<RoomEnvironmentSnapshot, DaemonError> {
        self.observe_browser_controller_generation(session_id, runtime_generation)?;
        match self
            .reconcile_browser_controller_environment(session_id)
            .await
        {
            Ok(environment) => Ok(environment),
            Err(error) => {
                let _ = self.update_room_environment_component_health(
                    session_id,
                    EnvironmentComponent::BrowserController,
                    EnvironmentComponentHealthState::Unavailable,
                    Some("controller_recovery_failed"),
                );
                let _ = self.update_room_environment_component_health(
                    session_id,
                    EnvironmentComponent::Browser,
                    EnvironmentComponentHealthState::Unavailable,
                    Some("controller_recovery_failed"),
                );
                let _ =
                    self.transition_room_environment(session_id, EnvironmentLifecycle::Degraded);
                Err(error)
            }
        }
    }

    pub(crate) async fn perform_browser_environment_locator_action_as_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        element_ref: &str,
        action: crate::runtime::browser_controller_action::BrowserLocatorAction,
        timeout_ms: u64,
    ) -> Result<
        super::BrowserControllerActionExecution<
            crate::runtime::browser_controller_action::RoomBrowserActionResult,
        >,
        DaemonError,
    > {
        let element = self
            .resolve_room_environment_element_reference(session_id, element_ref)
            .map_err(|error| environment_runtime_error("browser_controller.action", error))?;
        let action_kind = action.kind();
        let execution_id = format!("{:032x}", rand::random::<u128>());
        self.execute_browser_mutation_as_agent(
            session_id,
            agent_id,
            &element.tab_id,
            element.document_revision,
            action_kind,
            Some(&execution_id),
            self.perform_browser_environment_locator_action(
                session_id,
                element_ref,
                &execution_id,
                action,
                timeout_ms,
            ),
        )
        .await
    }

    pub(crate) async fn handle_browser_environment_dialog(
        &self,
        session_id: &str,
        tab_id: &str,
        action: crate::runtime::browser_controller_action::BrowserDialogAction,
    ) -> Result<crate::runtime::browser_controller_action::RoomBrowserDialogResult, DaemonError>
    {
        action
            .validate()
            .map_err(|message| DaemonError::LocalTransport {
                operation: "browser_controller.dialog",
                message,
            })?;
        let environment = self
            .room_environment_snapshot(session_id)
            .map_err(|error| environment_runtime_error("browser_controller.dialog", error))?;
        let binding = self
            .room_environment_controller_tab_binding(session_id, tab_id)
            .map_err(|error| environment_runtime_error("browser_controller.dialog", error))?;
        let target_id = binding.runtime_target_id.clone();
        let document_id = binding.document_id.clone();
        let RoomBrowserControllerResult::Dialog { result } = self
            .room_browser_controller_command(
                session_id,
                RoomBrowserControllerCommand::Dialog {
                    target_id,
                    document_id,
                    action,
                },
            )
            .await?
        else {
            return Err(controller_route_error(
                "unexpected controller dialog response",
            ));
        };
        let result = result.ok_or_else(|| DaemonError::LocalTransport {
            operation: "browser_controller.dialog",
            message: "browser controller is not enabled".to_string(),
        })?;
        Ok(result.into_room_result(
            session_id.to_string(),
            environment.environment_id,
            environment.runtime_generation,
            tab_id.to_string(),
            binding.document_revision,
        ))
    }

    pub(crate) async fn handle_browser_environment_dialog_as_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        tab_id: &str,
        action: crate::runtime::browser_controller_action::BrowserDialogAction,
    ) -> Result<
        super::BrowserControllerActionExecution<
            crate::runtime::browser_controller_action::RoomBrowserDialogResult,
        >,
        DaemonError,
    > {
        let environment = self
            .room_environment_snapshot(session_id)
            .map_err(|error| environment_runtime_error("browser_controller.dialog", error))?;
        let tab = environment
            .tabs
            .iter()
            .find(|candidate| candidate.tab_id == tab_id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "browser_controller.dialog",
                message: format!("Room browser tab `{tab_id}` is not available"),
            })?;
        self.execute_browser_mutation_as_agent(
            session_id,
            agent_id,
            tab_id,
            tab.document_revision,
            "dialog",
            None,
            self.handle_browser_environment_dialog(session_id, tab_id, action),
        )
        .await
    }

    pub(crate) async fn configure_browser_environment_downloads(
        &self,
        session_id: &str,
        tab_id: &str,
    ) -> Result<
        crate::runtime::browser_controller_file_transfer::RoomBrowserDownloadsResult,
        DaemonError,
    > {
        let environment = self
            .room_environment_snapshot(session_id)
            .map_err(|error| environment_runtime_error("browser_controller.downloads", error))?;
        let binding = self
            .room_environment_controller_tab_binding(session_id, tab_id)
            .map_err(|error| environment_runtime_error("browser_controller.downloads", error))?;
        let target_id = binding.runtime_target_id.clone();
        let document_id = binding.document_id.clone();
        let RoomBrowserControllerResult::Downloads { result } = self
            .room_browser_controller_command(
                session_id,
                RoomBrowserControllerCommand::ConfigureDownloads {
                    target_id,
                    document_id,
                },
            )
            .await?
        else {
            return Err(controller_route_error(
                "unexpected controller downloads response",
            ));
        };
        let result = result.ok_or_else(|| DaemonError::LocalTransport {
            operation: "browser_controller.downloads",
            message: "browser controller is not enabled".to_string(),
        })?;
        Ok(result.into_room_result(
            session_id.to_string(),
            environment.environment_id,
            environment.runtime_generation,
            tab_id.to_string(),
            binding.document_revision,
        ))
    }

    pub(crate) async fn upload_browser_environment_files(
        &self,
        session_id: &str,
        execution_id: &str,
        element_ref: &str,
        paths: Vec<std::path::PathBuf>,
    ) -> Result<
        crate::runtime::browser_controller_file_transfer::RoomBrowserUploadResult,
        DaemonError,
    > {
        let files =
            crate::runtime::browser_controller_file_transfer::BrowserUploadFiles::new(paths)
                .map_err(|message| DaemonError::LocalTransport {
                    operation: "browser_controller.upload",
                    message,
                })?;
        let environment = self
            .room_environment_snapshot(session_id)
            .map_err(|error| environment_runtime_error("browser_controller.upload", error))?;
        let element = self
            .resolve_room_environment_element_reference(session_id, element_ref)
            .map_err(|error| environment_runtime_error("browser_controller.upload", error))?;
        let binding = self
            .room_environment_controller_tab_binding(session_id, &element.tab_id)
            .map_err(|error| environment_runtime_error("browser_controller.upload", error))?;
        if element.runtime_generation != environment.runtime_generation
            || element.document_revision != binding.document_revision
        {
            return Err(DaemonError::LocalTransport {
                operation: "browser_controller.upload",
                message: "browser element reference became stale before upload".to_string(),
            });
        }
        let target_id = binding.runtime_target_id.clone();
        let document_id = binding.document_id.clone();
        let controller_node_ref = element.controller_node_ref.clone();
        let response = self
            .room_browser_controller_command(
                session_id,
                RoomBrowserControllerCommand::Upload {
                    execution_id: execution_id.to_string(),
                    target_id,
                    document_id,
                    node_ref: controller_node_ref,
                    files,
                },
            )
            .await?;
        let result = match response {
            RoomBrowserControllerResult::Upload { result } => result,
            RoomBrowserControllerResult::ActionCancelled { controller_fenced } => {
                return Err(DaemonError::BrowserControllerActionCancelled { controller_fenced });
            }
            _ => {
                return Err(controller_route_error(
                    "unexpected controller upload response",
                ))
            }
        };
        let result = result.ok_or_else(|| DaemonError::LocalTransport {
            operation: "browser_controller.upload",
            message: "browser controller is not enabled".to_string(),
        })?;
        Ok(result.into_room_result(
            session_id.to_string(),
            environment.environment_id,
            environment.runtime_generation,
            element.tab_id,
            element.document_revision,
            element_ref.to_string(),
        ))
    }

    pub(crate) async fn set_browser_environment_permission(
        &self,
        session_id: &str,
        tab_id: &str,
        permission: crate::runtime::browser_controller_permission::BrowserPermissionName,
        setting: crate::runtime::browser_controller_permission::BrowserPermissionSetting,
    ) -> Result<
        crate::runtime::browser_controller_permission::RoomBrowserPermissionResult,
        DaemonError,
    > {
        let environment = self
            .room_environment_snapshot(session_id)
            .map_err(|error| environment_runtime_error("browser_controller.permission", error))?;
        let binding = self
            .room_environment_controller_tab_binding(session_id, tab_id)
            .map_err(|error| environment_runtime_error("browser_controller.permission", error))?;
        let target_id = binding.runtime_target_id.clone();
        let document_id = binding.document_id.clone();
        let RoomBrowserControllerResult::Permission { result } = self
            .room_browser_controller_command(
                session_id,
                RoomBrowserControllerCommand::Permission {
                    target_id,
                    document_id,
                    permission,
                    setting,
                },
            )
            .await?
        else {
            return Err(controller_route_error(
                "unexpected controller permission response",
            ));
        };
        let result = result.ok_or_else(|| DaemonError::LocalTransport {
            operation: "browser_controller.permission",
            message: "browser controller is not enabled".to_string(),
        })?;
        Ok(result.into_room_result(
            session_id.to_string(),
            environment.environment_id,
            environment.runtime_generation,
            tab_id.to_string(),
            binding.document_revision,
        ))
    }

    pub(crate) async fn poll_browser_environment_events(
        &self,
        session_id: &str,
        browser_generation: u64,
        cursor: u64,
        limit: u16,
    ) -> Result<RoomBrowserEventBatch, DaemonError> {
        let RoomBrowserControllerResult::Events { batch } = self
            .room_browser_controller_command(
                session_id,
                RoomBrowserControllerCommand::PollEvents {
                    browser_generation,
                    cursor,
                    limit,
                },
            )
            .await?
        else {
            return Err(controller_route_error(
                "unexpected controller events response",
            ));
        };
        let batch = batch.ok_or_else(|| DaemonError::LocalTransport {
            operation: "browser_controller.events",
            message: "browser controller is not enabled".to_string(),
        })?;
        let browser_disconnected = batch
            .events
            .iter()
            .any(|event| event.kind == "browser_disconnected");
        let unknown_live_target = batch.events.iter().any(|event| {
            matches!(event.kind.as_str(), "target_created" | "target_changed")
                && event.target_id.as_deref().is_some_and(|target_id| {
                    self.room_environment_tab_id_for_controller_target(session_id, target_id)
                        .ok()
                        .flatten()
                        .is_none()
                })
        });
        if unknown_live_target && !browser_disconnected {
            self.reconcile_browser_controller_environment(session_id)
                .await?;
        }
        let events = batch
            .events
            .into_iter()
            .filter_map(|event| {
                let tab_id = match event.target_id.as_deref() {
                    Some(target_id) => Some(
                        self.room_environment_tab_id_for_controller_target(session_id, target_id)
                            .ok()??,
                    ),
                    None if matches!(
                        event.kind.as_str(),
                        "browser_connected" | "browser_disconnected"
                    ) =>
                    {
                        None
                    }
                    None => return None,
                };
                Some(RoomBrowserEvent {
                    event_id: event.event_id,
                    kind: event.kind,
                    tab_id,
                    document_id: event.document_id,
                    data: event.data,
                })
            })
            .collect();
        Ok(RoomBrowserEventBatch {
            browser_generation: batch.browser_generation,
            events,
            next_cursor: batch.next_cursor,
            replay_gap: batch.replay_gap,
        })
    }

    pub(crate) async fn stop_browser_controller_process(
        &self,
        session_id: &str,
    ) -> Result<Option<BrowserControllerProcessSnapshot>, DaemonError> {
        let RoomBrowserControllerResult::Process { snapshot } = self
            .room_browser_controller_command(session_id, RoomBrowserControllerCommand::Release)
            .await?
        else {
            return Err(controller_route_error(
                "unexpected controller stop response",
            ));
        };
        self.owned
            .browser_controller_generations
            .lock()
            .map_err(|_| controller_generation_error("generation lock poisoned"))?
            .remove(session_id);
        Ok(snapshot)
    }

    pub(crate) async fn stop_managed_room_environment_runtime(
        &self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, DaemonError> {
        self.begin_stop_room_environment(session_id)
            .map_err(|error| environment_runtime_error("environment.stop", error))?;
        if let Err(error) = self.stop_browser_controller_process(session_id).await {
            let _ = self.update_room_environment_component_health(
                session_id,
                EnvironmentComponent::BrowserController,
                EnvironmentComponentHealthState::Unavailable,
                Some("controller_stop_failed"),
            );
            let _ = self.transition_room_environment(session_id, EnvironmentLifecycle::Failed);
            return Err(error);
        }
        self.update_room_environment_component_health(
            session_id,
            EnvironmentComponent::BrowserController,
            EnvironmentComponentHealthState::Unavailable,
            None,
        )
        .map_err(|error| environment_runtime_error("environment.stop", error))?;
        self.complete_stop_room_environment(session_id)
            .map_err(|error| environment_runtime_error("environment.stop", error))
    }

    pub(crate) async fn shutdown_browser_controller_process(
        &self,
    ) -> Result<Option<BrowserControllerProcessSnapshot>, DaemonError> {
        let processes = self.owned.browser_controller_processes.clone();
        let snapshot = tokio::task::spawn_blocking(move || processes.shutdown())
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "browser_controller_process.shutdown",
                message: error.to_string(),
            })?
            .map_err(|message| DaemonError::LocalTransport {
                operation: "browser_controller_process.shutdown",
                message,
            })?;
        self.owned
            .browser_controller_generations
            .lock()
            .map_err(|_| controller_generation_error("generation lock poisoned"))?
            .clear();
        Ok(snapshot)
    }

    fn observe_browser_controller_generation(
        &self,
        session_id: &str,
        generation: u64,
    ) -> Result<bool, DaemonError> {
        let mut generations = self
            .owned
            .browser_controller_generations
            .lock()
            .map_err(|_| controller_generation_error("generation lock poisoned"))?;
        let mut began_recovery = false;
        let recovery_pending = match generations.get_mut(session_id) {
            Some((previous, pending)) if *previous != generation => {
                *previous = generation;
                *pending = true;
                began_recovery = true;
                true
            }
            Some((_, pending)) => *pending,
            None => {
                generations.insert(session_id.to_string(), (generation, false));
                false
            }
        };
        drop(generations);
        if began_recovery {
            self.begin_room_environment_browser_controller_recovery(session_id)
                .map_err(|error| environment_runtime_error("browser_controller.recover", error))?;
            self.update_room_environment_component_health(
                session_id,
                EnvironmentComponent::BrowserController,
                EnvironmentComponentHealthState::Starting,
                Some("controller_restarted"),
            )
            .map_err(|error| environment_runtime_error("browser_controller.recover", error))?;
            self.update_room_environment_component_health(
                session_id,
                EnvironmentComponent::Browser,
                EnvironmentComponentHealthState::Starting,
                None,
            )
            .map_err(|error| environment_runtime_error("browser_controller.recover", error))?;
        }
        Ok(recovery_pending)
    }

    fn complete_browser_controller_generation_recovery(
        &self,
        session_id: &str,
    ) -> Result<(), DaemonError> {
        let mut generations = self
            .owned
            .browser_controller_generations
            .lock()
            .map_err(|_| controller_generation_error("generation lock poisoned"))?;
        if let Some((_, pending)) = generations.get_mut(session_id) {
            *pending = false;
        }
        Ok(())
    }
}

fn controller_generation_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "browser_controller.recover",
        message: message.to_string(),
    }
}

fn environment_runtime_error(operation: &'static str, error: EnvironmentError) -> DaemonError {
    match error {
        EnvironmentError::RoomNotFound { session_id } => {
            DaemonError::SessionNotFound { session_id }
        }
        other => DaemonError::LocalTransport {
            operation,
            message: format!("{}: {other:?}", other.code()),
        },
    }
}
