use crate::error::DaemonError;
use crate::runtime::browser_controller_file_transfer::RoomBrowserUploadResult;

use super::room_browser_controller::controller_route_error;
use super::{BrowserControllerActionExecution, KernelRuntimeState};

impl KernelRuntimeState {
    pub(crate) async fn upload_browser_environment_files_as_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        element_ref: &str,
        paths: Vec<std::path::PathBuf>,
    ) -> Result<BrowserControllerActionExecution<RoomBrowserUploadResult>, DaemonError> {
        let element = self
            .resolve_room_environment_element_reference(session_id, element_ref)
            .map_err(|error| controller_route_error(&format!("{}: {error:?}", error.code())))?;
        let execution_id = format!("{:032x}", rand::random::<u128>());
        self.execute_browser_mutation_as_agent(
            session_id,
            agent_id,
            &element.tab_id,
            element.document_revision,
            "upload",
            Some(&execution_id),
            self.upload_browser_environment_files(session_id, &execution_id, element_ref, paths),
        )
        .await
    }
}
