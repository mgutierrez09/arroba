use crate::error::DaemonError;
use crate::local::{
    RoomEnvironmentBrowserHistoryAction, RoomEnvironmentBrowserTabAction,
    RoomEnvironmentHumanBrowserAction, SubmitRoomEnvironmentBrowserActionRequest,
};
use crate::session::{
    ActionAdmission, EnvironmentActionRequest, EnvironmentActor, EnvironmentError, InputTarget,
    RoomEnvironmentSnapshot,
};

use super::KernelRuntimeState;

const HUMAN_BROWSER_ACTION_IDEMPOTENCY_KEY_MAX_BYTES: usize = 128;

#[derive(Clone, Copy)]
enum HumanBrowserRuntimeAction {
    History(crate::runtime::browser_controller_history::BrowserHistoryAction),
    Tab(crate::runtime::browser_controller_tab::BrowserTabAction),
}

impl KernelRuntimeState {
    pub(crate) async fn execute_human_room_environment_browser_action(
        &self,
        request: SubmitRoomEnvironmentBrowserActionRequest,
        actor: EnvironmentActor,
    ) -> Result<(String, RoomEnvironmentSnapshot), DaemonError> {
        let environment = self
            .reconcile_browser_controller_environment(&request.session_id)
            .await?;
        validate_idempotency_key(&request).map_err(human_browser_environment_error)?;

        let (tab_id, action_kind, runtime_action) = match &request.action {
            RoomEnvironmentHumanBrowserAction::History { tab_id, action } => match action {
                RoomEnvironmentBrowserHistoryAction::Back => (
                    tab_id.as_str(),
                    "browser_history_back",
                    HumanBrowserRuntimeAction::History(
                        crate::runtime::browser_controller_history::BrowserHistoryAction::Back,
                    ),
                ),
                RoomEnvironmentBrowserHistoryAction::Forward => (
                    tab_id.as_str(),
                    "browser_history_forward",
                    HumanBrowserRuntimeAction::History(
                        crate::runtime::browser_controller_history::BrowserHistoryAction::Forward,
                    ),
                ),
                RoomEnvironmentBrowserHistoryAction::Reload => (
                    tab_id.as_str(),
                    "browser_history_reload",
                    HumanBrowserRuntimeAction::History(
                        crate::runtime::browser_controller_history::BrowserHistoryAction::Reload,
                    ),
                ),
            },
            RoomEnvironmentHumanBrowserAction::Tab { tab_id, action } => match action {
                RoomEnvironmentBrowserTabAction::Activate => (
                    tab_id.as_str(),
                    "browser_tab_activate",
                    HumanBrowserRuntimeAction::Tab(
                        crate::runtime::browser_controller_tab::BrowserTabAction::Activate,
                    ),
                ),
                RoomEnvironmentBrowserTabAction::Close => (
                    tab_id.as_str(),
                    "browser_tab_close",
                    HumanBrowserRuntimeAction::Tab(
                        crate::runtime::browser_controller_tab::BrowserTabAction::Close,
                    ),
                ),
            },
        };
        let document_revision = environment
            .tabs
            .iter()
            .find(|tab| tab.tab_id == tab_id)
            .map(|tab| tab.document_revision)
            .unwrap_or_default();
        let action_request = match runtime_action {
            HumanBrowserRuntimeAction::History(_) => EnvironmentActionRequest::browser_mutation(
                &actor.actor_id,
                request.runtime_generation,
                action_kind,
                tab_id,
                document_revision,
            ),
            HumanBrowserRuntimeAction::Tab(_) => EnvironmentActionRequest::browser_tab_mutation(
                &actor.actor_id,
                request.runtime_generation,
                action_kind,
                tab_id,
                document_revision,
            ),
        }
        .with_idempotency_key(request.idempotency_key.trim());
        if let Some(ActionAdmission::Existing { action_id, .. }) = self
            .existing_room_environment_action(&request.session_id, &action_request)
            .map_err(human_browser_environment_error)?
        {
            return Ok((action_id, environment));
        }
        validate_runtime_generation(&environment, request.runtime_generation)
            .map_err(human_browser_environment_error)?;
        if !environment.tabs.iter().any(|tab| tab.tab_id == tab_id) {
            return Err(human_browser_environment_error(
                EnvironmentError::UnknownTab {
                    tab_id: tab_id.to_string(),
                },
            ));
        }
        validate_browser_input_authority(&environment, &actor.actor_id, tab_id)
            .map_err(human_browser_environment_error)?;

        let execution = self
            .execute_browser_mutation(&request.session_id, action_request, None, async {
                match runtime_action {
                    HumanBrowserRuntimeAction::History(action) => {
                        self.navigate_browser_environment_history(
                            &request.session_id,
                            tab_id,
                            action,
                        )
                        .await
                    }
                    HumanBrowserRuntimeAction::Tab(action) => {
                        self.manage_browser_environment_tab(&request.session_id, tab_id, action)
                            .await
                    }
                }
            })
            .await?;
        let environment = self
            .room_environment_snapshot(&request.session_id)
            .map_err(human_browser_environment_error)?;
        Ok((execution.action_id, environment))
    }
}

fn validate_idempotency_key(
    request: &SubmitRoomEnvironmentBrowserActionRequest,
) -> Result<(), EnvironmentError> {
    let idempotency_key = request.idempotency_key.trim();
    if idempotency_key.is_empty()
        || idempotency_key.len() > HUMAN_BROWSER_ACTION_IDEMPOTENCY_KEY_MAX_BYTES
    {
        return Err(EnvironmentError::InvalidIdempotencyKey);
    }
    Ok(())
}

fn validate_runtime_generation(
    environment: &RoomEnvironmentSnapshot,
    runtime_generation: u64,
) -> Result<(), EnvironmentError> {
    if runtime_generation != environment.runtime_generation {
        return Err(EnvironmentError::StaleRuntimeGeneration {
            expected: environment.runtime_generation,
            actual: runtime_generation,
        });
    }
    Ok(())
}

fn validate_browser_input_authority(
    environment: &RoomEnvironmentSnapshot,
    actor_id: &str,
    tab_id: &str,
) -> Result<(), EnvironmentError> {
    let target = InputTarget::BrowserTab(tab_id.to_string());
    if !environment
        .input_ownership
        .iter()
        .any(|ownership| ownership.target == target && ownership.actor_id == actor_id)
    {
        return Err(EnvironmentError::InputTakeoverRequired {
            actor_id: actor_id.to_string(),
        });
    }
    Ok(())
}

fn human_browser_environment_error(error: EnvironmentError) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "environment.browser_action.submit",
        message: format!("{}: {error:?}", error.code()),
    }
}
