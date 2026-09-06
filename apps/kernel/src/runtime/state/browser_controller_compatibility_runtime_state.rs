use crate::error::DaemonError;
use crate::runtime::browser_controller_compatibility::{
    BrowserCompatibilityWait, BrowserControllerCompatibilityWaitResult,
    BrowserControllerNavigationResult, BrowserNavigationUrl,
};
use crate::transport::room_browser_controller::{
    RoomBrowserControllerCommand, RoomBrowserControllerResult,
};

use super::KernelRuntimeState;

impl KernelRuntimeState {
    pub(crate) async fn navigate_browser_environment_compatibility_as_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        url: &str,
    ) -> Result<
        super::BrowserControllerActionExecution<BrowserControllerNavigationResult>,
        DaemonError,
    > {
        let environment = self
            .room_environment_snapshot(session_id)
            .map_err(|error| DaemonError::LocalTransport {
                operation: "browser_controller.compatibility.navigate",
                message: format!("{}: {error:?}", error.code()),
            })?;
        let tab_id = environment
            .focused_tab_id
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "browser_controller.compatibility.navigate",
                message: "the Room browser has no focused tab for navigate".to_string(),
            })?;
        let document_revision = environment
            .tabs
            .iter()
            .find(|tab| tab.tab_id == tab_id)
            .map(|tab| tab.document_revision)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "browser_controller.compatibility.navigate",
                message: format!("Room browser tab `{tab_id}` is not available"),
            })?;
        let execution_id = format!("{:032x}", rand::random::<u128>());
        self.execute_browser_mutation_as_agent(
            session_id,
            agent_id,
            &tab_id,
            document_revision,
            "navigate",
            Some(&execution_id),
            self.navigate_browser_environment_compatibility(
                session_id,
                &execution_id,
                &tab_id,
                url,
            ),
        )
        .await
    }

    pub(crate) async fn navigate_browser_environment_compatibility(
        &self,
        session_id: &str,
        execution_id: &str,
        tab_id: &str,
        url: &str,
    ) -> Result<BrowserControllerNavigationResult, DaemonError> {
        let url =
            BrowserNavigationUrl::new(url).map_err(|message| DaemonError::LocalTransport {
                operation: "browser_controller.compatibility.navigate",
                message,
            })?;
        let expected_url = url.as_str().to_string();
        let binding = self
            .room_environment_controller_tab_binding(session_id, tab_id)
            .map_err(|error| {
                super::room_browser_controller::controller_route_error(&format!(
                    "{}: {error:?}",
                    error.code()
                ))
            })?;
        let target_id = binding.runtime_target_id;
        let document_id = binding.document_id;
        let RoomBrowserControllerResult::Navigation { result } = self
            .room_browser_controller_command(
                session_id,
                RoomBrowserControllerCommand::Navigate {
                    execution_id: execution_id.to_string(),
                    target_id: target_id.clone(),
                    document_id,
                    url,
                },
            )
            .await?
        else {
            return Err(DaemonError::LocalTransport {
                operation: "browser_controller.compatibility.navigate",
                message: "browser controller returned an unexpected navigation response"
                    .to_string(),
            });
        };
        let result = result.ok_or_else(|| DaemonError::LocalTransport {
            operation: "browser_controller.compatibility.navigate",
            message: "browser controller is not enabled".to_string(),
        })?;
        result
            .validate(&target_id, &expected_url)
            .map_err(|message| DaemonError::LocalTransport {
                operation: "browser_controller.compatibility.navigate",
                message,
            })?;
        self.reconcile_browser_controller_environment(session_id)
            .await?;
        Ok(result)
    }

    pub(crate) async fn wait_for_browser_environment_compatibility(
        &self,
        session_id: &str,
        wait: BrowserCompatibilityWait,
        timeout_ms: u64,
    ) -> Result<BrowserControllerCompatibilityWaitResult, DaemonError> {
        wait.validate(timeout_ms)
            .map_err(|message| DaemonError::LocalTransport {
                operation: "browser_controller.compatibility.wait",
                message,
            })?;
        let (target_id, document_id) =
            self.focused_browser_controller_identity(session_id, "wait")?;
        let RoomBrowserControllerResult::Wait { result } = self
            .room_browser_controller_command(
                session_id,
                RoomBrowserControllerCommand::Wait {
                    target_id: target_id.clone(),
                    document_id: document_id.clone(),
                    wait: wait.clone(),
                    timeout_ms,
                },
            )
            .await?
        else {
            return Err(DaemonError::LocalTransport {
                operation: "browser_controller.compatibility.wait",
                message: "browser controller returned an unexpected wait response".to_string(),
            });
        };
        let result = result.ok_or_else(|| DaemonError::LocalTransport {
            operation: "browser_controller.compatibility.wait",
            message: "browser controller is not enabled".to_string(),
        })?;
        result
            .validate(&target_id, &document_id, &wait)
            .map_err(|message| DaemonError::LocalTransport {
                operation: "browser_controller.compatibility.wait",
                message,
            })?;
        Ok(result)
    }

    fn focused_browser_controller_identity(
        &self,
        session_id: &str,
        action: &str,
    ) -> Result<(String, String), DaemonError> {
        let environment = self
            .room_environment_snapshot(session_id)
            .map_err(|error| DaemonError::LocalTransport {
                operation: "browser_controller.compatibility",
                message: format!("{}: {error:?}", error.code()),
            })?;
        let tab_id = environment
            .focused_tab_id
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "browser_controller.compatibility",
                message: format!("the Room browser has no focused tab for {action}"),
            })?;
        let binding = self
            .room_environment_controller_tab_binding(session_id, &tab_id)
            .map_err(|error| DaemonError::LocalTransport {
                operation: "browser_controller.compatibility",
                message: format!("{}: {error:?}", error.code()),
            })?;
        Ok((binding.runtime_target_id, binding.document_id))
    }
}
