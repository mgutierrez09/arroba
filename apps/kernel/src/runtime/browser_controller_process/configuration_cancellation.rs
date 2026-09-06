use super::*;
use crate::transport::room_browser_controller::RoomBrowserControllerResult as Response;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, serde::Serialize)]
pub(crate) enum BrowserConfiguration {
    Downloads,
    Permission {
        name: BrowserPermissionName,
        setting: BrowserPermissionSetting,
    },
}

impl BrowserConfiguration {
    fn unavailable(self) -> Response {
        match self {
            Self::Downloads => Response::Downloads { result: None },
            Self::Permission { .. } => Response::Permission { result: None },
        }
    }

    fn fingerprint(self, target: &str, document: &str) -> Result<[u8; 32], String> {
        let request = serde_json::to_vec(&("configuration", target, document, self))
            .map_err(|_| "failed to fingerprint browser configuration")?;
        Ok(Sha256::digest(request).into())
    }

    fn execute(
        self,
        ownership: &mut StdioOwnership,
        room: &str,
        target: &str,
        document: &str,
    ) -> Result<Response, String> {
        match self {
            Self::Downloads => ownership
                .configure_browser_downloads(room, target, document)
                .map(|result| Response::Downloads {
                    result: Some(result),
                }),
            Self::Permission { name, setting } => ownership
                .set_browser_permission(room, target, document, name, setting)
                .map(|result| Response::Permission {
                    result: Some(result),
                }),
        }
    }
}

impl BrowserControllerProcessStore {
    pub(crate) fn perform_cancellable_browser_configuration(
        &self,
        room: &str,
        execution_id: &str,
        target: &str,
        document: &str,
        configuration: BrowserConfiguration,
    ) -> Result<Response, String> {
        self.perform_cancellable_operation(
            room,
            execution_id,
            configuration.fingerprint(target, document)?,
            configuration.unavailable(),
            |ownership| configuration.execute(ownership, room, target, document),
        )
    }

    pub(crate) fn recover_cancellable_browser_configuration(
        &self,
        room: &str,
        execution_id: &str,
        target: &str,
        document: &str,
        configuration: BrowserConfiguration,
    ) -> Result<Response, String> {
        self.recover_cancellable_operation(
            room,
            execution_id,
            configuration.fingerprint(target, document)?,
        )
    }
}
