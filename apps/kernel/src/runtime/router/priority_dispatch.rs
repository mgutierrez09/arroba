use std::sync::Arc;

use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::runtime::agent_utility_executor::execute_agent_utility_request;
use crate::runtime::capability_executor::execute_required_capability_request;
use crate::runtime::capability_registry::execute_capability_registry_request;
use crate::runtime::cloud_relay_executor::execute_cloud_relay_request;
use crate::runtime::command::{command_caller_user_id, KernelCommand};
use crate::runtime::credential_enrollment_control::{
    execute_arm_credential_enrollment, execute_credential_enrollment_interaction,
};
use crate::runtime::daemon_health_projection::execute_daemon_health_request;
use crate::runtime::debug_bundle_control::execute_export_debug_bundle_request;
use crate::runtime::external_provider_session_control::execute_external_provider_session_request;
use crate::runtime::history_executor::execute_history_request;
use crate::runtime::interactive_command_dispatcher::{
    dispatch_interactive_command, is_interactive_command,
};
use crate::runtime::kernel_lifecycle_executor::execute_kernel_lifecycle_request;
use crate::runtime::metaagent_event_control::execute_metaagent_event_request;
use crate::runtime::native_interaction_bridge::execute_native_provider_interaction_request;
use crate::runtime::pairing_invite_executor::execute_pairing_request;
use crate::runtime::prompt_settings_executor::execute_prompt_settings_request;
use crate::runtime::provider_account_control::execute_provider_account_request;
use crate::runtime::provider_auth_control::execute_provider_auth_request;
use crate::runtime::provider_catalog_control::execute_provider_catalog_request;
use crate::runtime::provider_launch_executor::{
    execute_provider_batch_launch_command, execute_provider_launch_command,
};
use crate::runtime::provider_process_control::execute_provider_process_request;
use crate::runtime::provider_run_control::execute_provider_run_request;
use crate::runtime::relay_config_control::execute_relay_config_request;
use crate::runtime::remote_machine_registry::execute_remote_machine_registry_request;
use crate::runtime::remote_relay_inventory::execute_remote_relay_inventory_request;
use crate::runtime::session_collaboration_executor::execute_session_collaboration_request;
use crate::runtime::session_read_control::execute_session_read_request;
use crate::runtime::slice_command_executor::execute_slice_request;
use crate::runtime::state::workflow_publication_endpoint_runtime::execute_register_workflow_publication_endpoint_request;
use crate::runtime::state::workflow_publication_runtime_lifecycle::{
    execute_bind_workflow_publication_deployment_request,
    execute_control_workflow_publication_runtime_request,
};
use crate::runtime::terminal_command_catalog::terminal_command_catalog_response;
use crate::runtime::terminal_output_executor::{
    execute_append_native_provider_output_batch_request,
    execute_append_native_provider_output_request,
};
use crate::runtime::user_config_executor::execute_user_config_request;
use crate::runtime::waiting_room_control::execute_waiting_room_request;
use crate::runtime::workflow_actor::is_workflow_command;
use crate::runtime::workspace_command_executor::execute_workspace_command_request;

use super::CommandRouter;

impl CommandRouter {
    pub(super) async fn dispatch_interactive(
        &self,
        command: KernelCommand,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        dispatch_interactive_command(
            &self.session_runtime,
            &self.agent_runtime,
            &self.runtime_state,
            command,
            request,
        )
        .await
    }

    pub(super) async fn dispatch_normal_or_background(
        &self,
        command: KernelCommand,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        match request {
            LocalDaemonRequest::GetTerminalCommandCatalog(_) => terminal_command_catalog_response(),
            LocalDaemonRequest::RespondToInteraction(_) => Err(DaemonError::LocalTransport {
                operation: "dispatch normal or background",
                message: "interaction responses must be dispatched through the session runtime"
                    .to_string(),
            }),
            LocalDaemonRequest::ArmDeploymentCredentialEnrollment(request) => {
                execute_arm_credential_enrollment(
                    &self.credential_enrollment_control,
                    &self.runtime_state,
                    &command,
                    request,
                )
                .await
            }
            LocalDaemonRequest::RequestCredentialEnrollmentInteraction(request) => {
                execute_credential_enrollment_interaction(
                    &self.credential_enrollment_control,
                    &self.runtime_state,
                    &command,
                    request,
                )
                .await
            }
            LocalDaemonRequest::RequestNativeProviderInteraction(request) => {
                execute_native_provider_interaction_request(&self.runtime_state, request).await
            }
            LocalDaemonRequest::LaunchProviderRun(request) => {
                execute_provider_launch_command(&self.runtime_state, &command, request).await
            }
            LocalDaemonRequest::LaunchProviderRuns(request) => {
                execute_provider_batch_launch_command(&self.runtime_state, &command, request).await
            }
            LocalDaemonRequest::CaptureRoomEnvironmentScreenshot(request) => {
                let artifact = self
                    .runtime_state
                    .capture_room_environment_screenshot(&command.caller, request)
                    .await?;
                Ok(LocalDaemonResponse::RoomEnvironmentScreenshotCaptured { artifact })
            }
            LocalDaemonRequest::ReadRoomEnvironmentScreenshotChunk(request) => {
                let chunk = self
                    .runtime_state
                    .read_room_environment_screenshot_chunk(&command.caller, request)
                    .await?;
                Ok(LocalDaemonResponse::RoomEnvironmentScreenshotChunk { chunk })
            }
            request @ (LocalDaemonRequest::ListSessions(_)
            | LocalDaemonRequest::ResolveSession(_)
            | LocalDaemonRequest::GetSessionState(_)
            | LocalDaemonRequest::GetRoomEnvironmentState(_)
            | LocalDaemonRequest::GetRoomEnvironmentSlice(_)
            | LocalDaemonRequest::GetRoomEnvironmentEvents(_)
            | LocalDaemonRequest::ListRoomEnvironmentActionHistory(_)
            | LocalDaemonRequest::ListAgents(_)) => {
                execute_session_read_request(&self.runtime_state, request).await
            }
            request @ (LocalDaemonRequest::SearchMetaagentCommands(_)
            | LocalDaemonRequest::GetMetaagentTurnOverview(_)
            | LocalDaemonRequest::GetMetaagentTurnBlob(_)
            | LocalDaemonRequest::ListMetaagentEvents(_)
            | LocalDaemonRequest::ReadMetaagentEvent(_)
            | LocalDaemonRequest::AckMetaagentEvents(_)) => {
                let caller_user_id = command_caller_user_id(&command);
                execute_metaagent_event_request(&self.runtime_state, request, &caller_user_id).await
            }
            request @ (LocalDaemonRequest::UpdateMetaagentTask(_)
            | LocalDaemonRequest::PauseMetaagentTask(_)
            | LocalDaemonRequest::ResumeMetaagentTask(_)
            | LocalDaemonRequest::AbortMetaagentTask(_)) => {
                self.runtime_state
                    .execute_metaagent_task_request(request)
                    .await
            }
            request @ LocalDaemonRequest::GetDaemonHealth(_) => {
                execute_daemon_health_request(self.daemon_health_projection_input(0), request).await
            }
            LocalDaemonRequest::ExportDebugBundle(request) => {
                execute_export_debug_bundle_request(request)
            }
            request @ (LocalDaemonRequest::GetProviderRun(_)
            | LocalDaemonRequest::UpdateProviderRunSelection(_)) => {
                let caller_user_id = command_caller_user_id(&command);
                execute_provider_run_request(
                    &self.runtime_state,
                    &self.provider_catalog_projection,
                    &caller_user_id,
                    request,
                )
                .await
            }
            request @ (LocalDaemonRequest::GetPromptInputHistory(_)
            | LocalDaemonRequest::RecordPromptInputHistory(_)) => {
                execute_history_request(
                    self.history_store.clone(),
                    self.operational_history_store.clone(),
                    &self.runtime_state,
                    &self.config_projection,
                    request,
                )
                .await
            }
            request @ (LocalDaemonRequest::ListSlices(_)
            | LocalDaemonRequest::CreateSlice(_)
            | LocalDaemonRequest::GetSlice(_)
            | LocalDaemonRequest::StartSlice(_)
            | LocalDaemonRequest::StopSlice(_)
            | LocalDaemonRequest::DeleteSlice(_)
            | LocalDaemonRequest::ImportSliceProviderAuth(_)
            | LocalDaemonRequest::RemoveSliceProviderAuth(_)
            | LocalDaemonRequest::StartSliceProviderLogin(_)
            | LocalDaemonRequest::GetSliceDisplayEndpoint(_)
            | LocalDaemonRequest::GetSliceLogs(_)
            | LocalDaemonRequest::ListSliceAudit(_)
            | LocalDaemonRequest::SaveSliceState(_)
            | LocalDaemonRequest::GetSliceStateStatus(_)
            | LocalDaemonRequest::ResetSliceState(_)
            | LocalDaemonRequest::CreateSliceBackup(_)
            | LocalDaemonRequest::RestoreSliceBackup(_)) => {
                execute_slice_request(
                    &self.runtime_state,
                    &self.config_projection,
                    Some(Arc::clone(&self.relay_state)),
                    &command.caller,
                    request,
                )
                .await
            }
            request @ (LocalDaemonRequest::GetProviderCatalog(_)
            | LocalDaemonRequest::GetProviderCommandCatalogs(_)) => {
                execute_provider_catalog_request(
                    &self.provider_catalog_projection,
                    &self.config_projection,
                    self.runtime_state.provider_account_profile_registry(),
                    &command_caller_user_id(&command),
                    request,
                )
                .await
            }
            request @ (LocalDaemonRequest::InstallMcpServer(_)
            | LocalDaemonRequest::UpdateMcpServer(_)
            | LocalDaemonRequest::UninstallMcpServer(_)
            | LocalDaemonRequest::ImportMcpServers(_)
            | LocalDaemonRequest::ImportProviderCapabilities(_)
            | LocalDaemonRequest::GetMcpServer(_)
            | LocalDaemonRequest::ListMcpServers(_)
            | LocalDaemonRequest::RegisterEnvironment(_)
            | LocalDaemonRequest::RemoveEnvironment(_)
            | LocalDaemonRequest::GetEnvironment(_)
            | LocalDaemonRequest::ListEnvironments(_)
            | LocalDaemonRequest::ValidateScript(_)
            | LocalDaemonRequest::RegisterScript(_)
            | LocalDaemonRequest::RemoveScript(_)
            | LocalDaemonRequest::GetScript(_)
            | LocalDaemonRequest::ListScripts(_)
            | LocalDaemonRequest::RegisterCredential(_)
            | LocalDaemonRequest::UpsertCredential(_)
            | LocalDaemonRequest::RemoveCredential(_)
            | LocalDaemonRequest::GetCredential(_)
            | LocalDaemonRequest::ListCredentials(_)
            | LocalDaemonRequest::RegisterConnector(_)
            | LocalDaemonRequest::UpsertConnector(_)
            | LocalDaemonRequest::RegisterConnectorAdapter(_)
            | LocalDaemonRequest::RemoveConnectorAdapter(_)
            | LocalDaemonRequest::GetConnectorAdapter(_)
            | LocalDaemonRequest::ListConnectorAdapters(_)
            | LocalDaemonRequest::RemoveConnector(_)
            | LocalDaemonRequest::GetConnector(_)
            | LocalDaemonRequest::ListConnectors(_)
            | LocalDaemonRequest::TestConnector(_)
            | LocalDaemonRequest::UpsertSkill(_)
            | LocalDaemonRequest::InstallSkill(_)
            | LocalDaemonRequest::UpdateSkill(_)
            | LocalDaemonRequest::UninstallSkill(_)
            | LocalDaemonRequest::ImportSkills(_)
            | LocalDaemonRequest::GetSkill(_)
            | LocalDaemonRequest::ListSkills(_)) => {
                let credential_vault = self
                    .config_projection
                    .snapshot()
                    .user_config
                    .credential_vault;
                execute_capability_registry_request(request, credential_vault)
            }
            request @ LocalDaemonRequest::RelayStatus(_) => {
                execute_relay_config_request(
                    &self.runtime_state,
                    Arc::clone(&self.relay_state),
                    &self.config_projection,
                    &self.provider_catalog_projection,
                    request,
                )
                .await
            }
            request @ LocalDaemonRequest::ConfigureRelay(_) => {
                execute_relay_config_request(
                    &self.runtime_state,
                    Arc::clone(&self.relay_state),
                    &self.config_projection,
                    &self.provider_catalog_projection,
                    request,
                )
                .await
            }
            request @ (LocalDaemonRequest::CloudRelayStatus(_)
            | LocalDaemonRequest::StartCloudRelayLogin(_)
            | LocalDaemonRequest::PollCloudRelayLogin(_)
            | LocalDaemonRequest::LogoutCloudRelay(_)
            | LocalDaemonRequest::PairCloudRelayClient(_)
            | LocalDaemonRequest::PairCloudRelayMachine(_)
            | LocalDaemonRequest::ConnectCloudRelay(_)
            | LocalDaemonRequest::IssueCloudRelayClientToken(_)
            | LocalDaemonRequest::ResolveKernelClientConnection(_)
            | LocalDaemonRequest::CreateCloudSessionInvite(_)
            | LocalDaemonRequest::ShowCloudSessionInvite(_)
            | LocalDaemonRequest::AcceptCloudSessionInvite(_)
            | LocalDaemonRequest::RevokeCloudSessionInvite(_)
            | LocalDaemonRequest::ListCloudSessionMembers(_)
            | LocalDaemonRequest::ListCloudCollaborators(_)) => {
                execute_cloud_relay_request(
                    &self.runtime_state,
                    &self.config_projection,
                    &self.provider_catalog_projection,
                    &self.remote_relay_inventory_projection,
                    Arc::clone(&self.relay_state),
                    request,
                )
                .await
            }
            request @ (LocalDaemonRequest::GetUserConfig(_)
            | LocalDaemonRequest::GetUserConfigSchema(_)
            | LocalDaemonRequest::SetUserConfigValue(_)
            | LocalDaemonRequest::SetWorkspaceLiveSyncMode(_)
            | LocalDaemonRequest::UnsetUserConfigValue(_)
            | LocalDaemonRequest::SetCredentialSecret(_)
            | LocalDaemonRequest::DeleteCredentialSecret(_)
            | LocalDaemonRequest::SetProviderAccountCredential(_)
            | LocalDaemonRequest::GetCredentialVaultStatus(_)
            | LocalDaemonRequest::LockCredentialVault(_)
            | LocalDaemonRequest::ManageCredentialVault(_)) => {
                execute_user_config_request(
                    &self.config_projection,
                    &self.runtime_state,
                    &command,
                    request,
                )
                .await
            }
            request @ (LocalDaemonRequest::ListPromptSettings(_)
            | LocalDaemonRequest::GetPromptSetting(_)
            | LocalDaemonRequest::UpdatePromptSetting(_)
            | LocalDaemonRequest::PreviewPromptSetting(_)
            | LocalDaemonRequest::ResetPromptSetting(_)
            | LocalDaemonRequest::ResetAllPromptSettings(_)) => {
                execute_prompt_settings_request(&command, request).await
            }
            request @ LocalDaemonRequest::DeleteKernel(_) => {
                execute_kernel_lifecycle_request(
                    &self.config_projection,
                    &self.runtime_state,
                    request,
                )
                .await
            }
            request @ (LocalDaemonRequest::ListRemoteMachines(_)
            | LocalDaemonRequest::ListRemoteMachineKernels(_)) => {
                execute_remote_relay_inventory_request(
                    Arc::clone(&self.relay_state),
                    self.config_projection.clone(),
                    self.remote_relay_inventory_projection.clone(),
                    request,
                )
                .await
            }
            request @ (LocalDaemonRequest::GetWaitingRoomInventory(_)
            | LocalDaemonRequest::GetWaitingRoomPublicSnapshot(_)) => {
                let caller_user_id = command_caller_user_id(&command);
                execute_waiting_room_request(
                    &self.runtime_state,
                    &self.session_projection,
                    &self.waiting_room_session_summaries,
                    Arc::clone(&self.relay_state),
                    self.config_projection.clone(),
                    self.remote_relay_inventory_projection.clone(),
                    request,
                    &caller_user_id,
                )
                .await
            }
            request @ (LocalDaemonRequest::ListExternalProviderSessions(_)
            | LocalDaemonRequest::RefreshExternalProviderSessions(_)
            | LocalDaemonRequest::ImportExternalProviderSession(_)
            | LocalDaemonRequest::ImportExternalProviderAgent(_)) => {
                let caller_user_id = command_caller_user_id(&command);
                execute_external_provider_session_request(
                    &self.app,
                    Some(&self.runtime_state),
                    request,
                    &caller_user_id,
                )
                .await
            }
            LocalDaemonRequest::RegisterWorkflowPublicationEndpoint(request) => {
                let caller_user_id = command_caller_user_id(&command);
                execute_register_workflow_publication_endpoint_request(
                    &self.runtime_state,
                    &self.config_projection,
                    Arc::clone(&self.relay_state),
                    request,
                    &caller_user_id,
                )
                .await
            }
            LocalDaemonRequest::ControlWorkflowPublicationRuntime(request) => {
                let caller_user_id = command_caller_user_id(&command);
                execute_control_workflow_publication_runtime_request(
                    &self.runtime_state,
                    request,
                    &caller_user_id,
                )
                .await
            }
            LocalDaemonRequest::BindWorkflowPublicationDeployment(request) => {
                let caller_user_id = command_caller_user_id(&command);
                execute_bind_workflow_publication_deployment_request(
                    &self.runtime_state,
                    &self.config_projection,
                    Arc::clone(&self.relay_state),
                    request,
                    &caller_user_id,
                )
                .await
            }
            request @ (LocalDaemonRequest::SearchWorkspaceDirectories(_)
            | LocalDaemonRequest::CreateWorkspaceDirectory(_)
            | LocalDaemonRequest::ListWorkspaceWorktrees(_)
            | LocalDaemonRequest::CreateWorkspaceWorktree(_)
            | LocalDaemonRequest::DeleteWorkspaceWorktree(_)
            | LocalDaemonRequest::CreateWorkspacePullRequest(_)
            | LocalDaemonRequest::GetWorkspaceGitOverview(_)
            | LocalDaemonRequest::ListWorkspaceFiles(_)
            | LocalDaemonRequest::GetWorkspaceFileContent(_)
            | LocalDaemonRequest::CommitWorkspaceChanges(_)
            | LocalDaemonRequest::PushWorkspaceBranch(_)
            | LocalDaemonRequest::CommitAndPushWorkspaceChanges(_)) => {
                execute_workspace_command_request(
                    &self.runtime_state,
                    &self.session_projection,
                    request,
                )
                .await
            }
            request @ (LocalDaemonRequest::RunAgentUtility(_)
            | LocalDaemonRequest::GenerateWorkspaceCommitMessage(_)) => {
                execute_agent_utility_request(&self.runtime_state, &self.config_projection, request)
                    .await
            }
            request @ (LocalDaemonRequest::ApproveRemoteMachine(_)
            | LocalDaemonRequest::ForgetRemoteMachine(_)
            | LocalDaemonRequest::RenameRemoteMachine(_)) => {
                execute_remote_machine_registry_request(
                    &self.app,
                    &self.config_projection,
                    &self.provider_catalog_projection,
                    &self.remote_relay_inventory_projection,
                    request,
                )
                .await
            }
            request @ (LocalDaemonRequest::ListSessionMembers(_)
            | LocalDaemonRequest::CreateSessionInvite(_)
            | LocalDaemonRequest::JoinSessionInvite(_)
            | LocalDaemonRequest::RevokeSessionInvite(_)
            | LocalDaemonRequest::CreateWorkspaceLink(_)
            | LocalDaemonRequest::ListWorkspaceLinks(_)
            | LocalDaemonRequest::ShowWorkspaceLink(_)
            | LocalDaemonRequest::AttachWorkspaceLink(_)
            | LocalDaemonRequest::DetachWorkspaceLink(_)
            | LocalDaemonRequest::GetWorkspaceLiveSyncStatus(_)) => {
                execute_session_collaboration_request(
                    &self.runtime_state,
                    &self.config_projection,
                    &command,
                    request,
                )
                .await
            }
            request @ (LocalDaemonRequest::CreatePairingInvite(_)
            | LocalDaemonRequest::JoinPairingInvite(_)
            | LocalDaemonRequest::CreateTerminalPairingLink(_)
            | LocalDaemonRequest::JoinTerminalPairingLink(_)
            | LocalDaemonRequest::ListTerminals(_)
            | LocalDaemonRequest::ListPairedClients(_)
            | LocalDaemonRequest::RecordPairedClient(_)
            | LocalDaemonRequest::RevokePairedClient(_)) => {
                execute_pairing_request(
                    &self.runtime_state,
                    &self.config_projection,
                    &self.provider_catalog_projection,
                    request,
                )
                .await
            }
            request @ (LocalDaemonRequest::GetProviderAuthStatus(_)
            | LocalDaemonRequest::StartProviderLogin(_)
            | LocalDaemonRequest::GetProviderLoginStatus(_)
            | LocalDaemonRequest::SendProviderLoginInput(_)
            | LocalDaemonRequest::CancelProviderLogin(_)) => {
                let caller_user_id = command_caller_user_id(&command);
                execute_provider_auth_request(&self.runtime_state, &caller_user_id, request).await
            }
            request @ (LocalDaemonRequest::ListProviderAccountProfiles(_)
            | LocalDaemonRequest::GetProviderAccountProfile(_)
            | LocalDaemonRequest::CreateProviderAccountProfile(_)
            | LocalDaemonRequest::LinkProviderAccountProfile(_)
            | LocalDaemonRequest::ImportNativeProviderAccountProfile(_)
            | LocalDaemonRequest::RenameProviderAccountProfile(_)
            | LocalDaemonRequest::SetDefaultProviderAccountProfile(_)
            | LocalDaemonRequest::RefreshProviderAccountProfile(_)
            | LocalDaemonRequest::RemoveProviderAccountProfile(_)
            | LocalDaemonRequest::DeleteProviderAccountProfileData(_)) => {
                let caller_user_id = command_caller_user_id(&command);
                execute_provider_account_request(&self.runtime_state, &caller_user_id, request)
                    .await
            }
            request @ LocalDaemonRequest::LogoutProvider(_) => {
                let caller_user_id = command_caller_user_id(&command);
                execute_provider_run_request(
                    &self.runtime_state,
                    &self.provider_catalog_projection,
                    &caller_user_id,
                    request,
                )
                .await
            }
            request @ (LocalDaemonRequest::ListProviderProcesses(_)
            | LocalDaemonRequest::TeardownProviderProcesses(_)) => {
                let caller_user_id = command_caller_user_id(&command);
                execute_provider_process_request(
                    &self.runtime_state,
                    &self.session_projection,
                    &self.agent_runtime_projection,
                    &self.provider_process_projection,
                    &self.provider_run_projection,
                    &caller_user_id,
                    request,
                )
                .await
            }
            request @ (LocalDaemonRequest::GetSessionHistoryOutline(_)
            | LocalDaemonRequest::GetSessionHistoryBlobContent(_)
            | LocalDaemonRequest::QueryRecall(_)
            | LocalDaemonRequest::SearchRecall(_)
            | LocalDaemonRequest::SemanticSearchRecall(_)) => {
                execute_history_request(
                    self.history_store.clone(),
                    self.operational_history_store.clone(),
                    &self.runtime_state,
                    &self.config_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::PumpTerminalOutput(request) => {
                self.terminal_output_executor.execute(request).await
            }
            LocalDaemonRequest::AppendNativeProviderOutput(request) => {
                execute_append_native_provider_output_request(
                    &self.runtime_state,
                    &self.session_projection,
                    &self.agent_runtime_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::AppendNativeProviderOutputBatch(request) => {
                execute_append_native_provider_output_batch_request(
                    &self.runtime_state,
                    &self.session_projection,
                    &self.agent_runtime_projection,
                    request,
                )
                .await
            }
            request @ (LocalDaemonRequest::RunShellCommand(_)
            | LocalDaemonRequest::ReadDirectoryTree(_)
            | LocalDaemonRequest::ReadFile(_)
            | LocalDaemonRequest::EditFile(_)
            | LocalDaemonRequest::InspectGit(_)
            | LocalDaemonRequest::CaptureScreenshot(_)
            | LocalDaemonRequest::StoreTransferredFile(_)) => {
                execute_required_capability_request(
                    &self.capability_runtime,
                    self.capability_health.clone(),
                    request,
                )
                .await
            }
            LocalDaemonRequest::SubmitPrompt(request) => {
                self.agent_runtime
                    .dispatch_prompt_submit(&command, request)
                    .await
            }
            LocalDaemonRequest::SubmitPrompts(request) => {
                self.agent_runtime
                    .dispatch_prompt_submit_batch(&command, request)
                    .await
            }
            LocalDaemonRequest::CompletePrompt(request) => {
                self.agent_runtime
                    .dispatch_prompt_complete(&command, request)
                    .await
            }
            LocalDaemonRequest::CancelActivePrompt(request) => {
                self.agent_runtime
                    .dispatch_prompt_cancel(&command, request)
                    .await
            }
            LocalDaemonRequest::SteerQueuedPrompt(request) => {
                self.agent_runtime
                    .dispatch_prompt_steer_queued(&command, request)
                    .await
            }
            LocalDaemonRequest::CancelQueuedPrompt(request) => {
                self.agent_runtime
                    .dispatch_prompt_cancel_queued(&command, request)
                    .await
            }
            LocalDaemonRequest::UpdateQueuedPrompt(request) => {
                self.agent_runtime
                    .dispatch_prompt_update_queued(&command, request)
                    .await
            }
            request => {
                if is_workflow_command(&request) {
                    self.workflow_runtime
                        .dispatch_workflow_command(command, request)
                        .await
                } else if is_interactive_command(&request) {
                    self.dispatch_interactive(command, request).await
                } else {
                    unreachable!("normal/background request should be handled before fallback")
                }
            }
        }
    }
}
