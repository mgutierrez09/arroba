use crate::local::LocalDaemonRequest;

use super::KernelCommandPriority;

#[derive(Debug)]
pub(super) struct LocalRequestMetadata {
    pub(super) command_type: &'static str,
    pub(super) priority: KernelCommandPriority,
    pub(super) session_id: Option<String>,
    pub(super) attachment_id: Option<String>,
    pub(super) agent_id: Option<String>,
    pub(super) provider_run_id: Option<String>,
    pub(super) workflow_run_id: Option<String>,
    pub(super) node_run_id: Option<String>,
}

impl LocalRequestMetadata {
    fn new(command_type: &'static str, priority: KernelCommandPriority) -> Self {
        Self {
            command_type,
            priority,
            session_id: None,
            attachment_id: None,
            agent_id: None,
            provider_run_id: None,
            workflow_run_id: None,
            node_run_id: None,
        }
    }

    fn session(mut self, session_id: &str) -> Self {
        self.session_id = Some(session_id.to_string());
        self
    }

    fn attachment(mut self, attachment_id: &str) -> Self {
        self.attachment_id = Some(attachment_id.to_string());
        self
    }

    fn agent(mut self, agent_id: &str) -> Self {
        self.agent_id = Some(agent_id.to_string());
        self
    }

    fn optional_session(mut self, session_id: Option<&str>) -> Self {
        if let Some(session_id) = session_id {
            self.session_id = Some(session_id.to_string());
        }
        self
    }

    fn optional_agent(mut self, agent_id: Option<&str>) -> Self {
        if let Some(agent_id) = agent_id {
            self.agent_id = Some(agent_id.to_string());
        }
        self
    }

    fn provider_run(mut self, provider_run_id: &str) -> Self {
        self.provider_run_id = Some(provider_run_id.to_string());
        self
    }

    fn workflow_run(mut self, workflow_run_id: &str) -> Self {
        self.workflow_run_id = Some(workflow_run_id.to_string());
        self
    }
}

pub(super) fn local_request_metadata(request: &LocalDaemonRequest) -> LocalRequestMetadata {
    use KernelCommandPriority::{Background, Interactive, Normal};

    match request {
        LocalDaemonRequest::BindRoomEnvironmentSlice(request) => {
            LocalRequestMetadata::new("environment.slice.bind", Interactive)
                .session(&request.session_id)
        }
        LocalDaemonRequest::GetRoomEnvironmentSlice(request) => {
            LocalRequestMetadata::new("environment.slice.get", Normal).session(&request.session_id)
        }
        LocalDaemonRequest::CaptureRoomEnvironmentScreenshot(request) => {
            LocalRequestMetadata::new("environment.screenshot.capture", Normal)
                .session(&request.session_id)
        }
        LocalDaemonRequest::ReadRoomEnvironmentScreenshotChunk(request) => {
            LocalRequestMetadata::new("environment.screenshot.read", Normal)
                .session(&request.session_id)
        }
        LocalDaemonRequest::CreateSession(_) => {
            LocalRequestMetadata::new("session.create", Interactive)
        }
        LocalDaemonRequest::AttachToSession(request) => {
            LocalRequestMetadata::new("session.attach", Interactive).session(&request.session_id)
        }
        LocalDaemonRequest::DetachFromSession(request) => {
            LocalRequestMetadata::new("session.detach", Interactive)
                .attachment(&request.attachment_id)
        }
        LocalDaemonRequest::CreateWorkspaceLink(request) => {
            LocalRequestMetadata::new("workspace_link.create", Normal).session(&request.session_id)
        }
        LocalDaemonRequest::ListWorkspaceLinks(request) => {
            LocalRequestMetadata::new("workspace_link.list", Normal).session(&request.session_id)
        }
        LocalDaemonRequest::ShowWorkspaceLink(request) => {
            LocalRequestMetadata::new("workspace_link.show", Normal).session(&request.session_id)
        }
        LocalDaemonRequest::AttachWorkspaceLink(request) => {
            LocalRequestMetadata::new("workspace_link.attach", Normal).session(&request.session_id)
        }
        LocalDaemonRequest::DetachWorkspaceLink(request) => {
            LocalRequestMetadata::new("workspace_link.detach", Normal).session(&request.session_id)
        }
        LocalDaemonRequest::GetWorkspaceLiveSyncStatus(request) => {
            LocalRequestMetadata::new("workspace_live_sync.status", Normal)
                .session(&request.session_id)
        }
        LocalDaemonRequest::SetWorkspaceLiveSyncMode(request) => {
            LocalRequestMetadata::new("workspace_live_sync.mode", Normal)
                .session(&request.session_id)
        }
        LocalDaemonRequest::SetCredentialSecret(request) => {
            LocalRequestMetadata::new("credential.secret.set", Normal)
                .optional_session(request.session_id.as_deref())
                .optional_agent(request.agent_id.as_deref())
        }
        LocalDaemonRequest::DeleteCredentialSecret(request) => {
            LocalRequestMetadata::new("credential.secret.delete", Normal)
                .optional_session(request.session_id.as_deref())
                .optional_agent(request.agent_id.as_deref())
        }
        LocalDaemonRequest::SetProviderAccountCredential(request) => {
            LocalRequestMetadata::new("provider_account.credential.set", Interactive)
                .optional_session(request.session_id.as_deref())
                .optional_agent(request.agent_id.as_deref())
        }
        LocalDaemonRequest::GetCredentialVaultStatus(_) => {
            LocalRequestMetadata::new("credential_vault.status", Normal)
        }
        LocalDaemonRequest::LockCredentialVault(_) => {
            LocalRequestMetadata::new("credential_vault.lock", Normal)
        }
        LocalDaemonRequest::ManageCredentialVault(request) => {
            LocalRequestMetadata::new("credential_vault.manage", Interactive)
                .session(&request.session_id)
                .optional_agent(request.agent_id.as_deref())
        }
        LocalDaemonRequest::ListManagedEnvironmentCatalog(_) => {
            LocalRequestMetadata::new("managed_environment.catalog", Normal)
        }
        LocalDaemonRequest::GetManagedEnvironment(_) => {
            LocalRequestMetadata::new("managed_environment.get", Normal)
        }
        LocalDaemonRequest::PrepareManagedEnvironmentContextTransfer(_) => {
            LocalRequestMetadata::new("managed_environment.context_transfer.prepare", Interactive)
        }
        LocalDaemonRequest::CreateManagedEnvironment(_) => {
            LocalRequestMetadata::new("managed_environment.create", Interactive)
        }
        LocalDaemonRequest::RequestManagedEnvironmentLifecycle(_) => {
            LocalRequestMetadata::new("managed_environment.lifecycle", Interactive)
        }
        LocalDaemonRequest::StartManagedContextTransfer(_) => {
            LocalRequestMetadata::new("managed_context.transfer.start", Background)
        }
        LocalDaemonRequest::GetManagedContextTransferStatus(_) => {
            LocalRequestMetadata::new("managed_context.transfer.status", Normal)
        }
        LocalDaemonRequest::GetManagedContextLaunchTarget(_) => {
            LocalRequestMetadata::new("managed_context.launch_target.get", Normal)
        }
        LocalDaemonRequest::SubmitPrompt(request) => {
            let mut metadata = LocalRequestMetadata::new("prompt.submit", Interactive)
                .session(&request.session_id)
                .attachment(&request.attachment_id);
            if let Some(agent_id) = request.target_agent_id.as_deref() {
                metadata = metadata.agent(agent_id);
            }
            metadata
        }
        LocalDaemonRequest::SubmitPrompts(request) => {
            LocalRequestMetadata::new("prompts.submit", Interactive)
                .session(&request.session_id)
                .attachment(&request.attachment_id)
        }
        LocalDaemonRequest::CreateAgentPromptSchedule(request) => {
            LocalRequestMetadata::new("agent_prompt_schedule.create", Interactive)
                .session(&request.session_id)
                .agent(&request.agent_id)
        }
        LocalDaemonRequest::CancelAgentPromptSchedule(request) => {
            LocalRequestMetadata::new("agent_prompt_schedule.cancel", Interactive)
                .session(&request.session_id)
        }
        LocalDaemonRequest::CancelActivePrompt(request) => {
            let mut metadata = LocalRequestMetadata::new("prompt.cancel", Interactive)
                .session(&request.session_id)
                .attachment(&request.attachment_id);
            if let Some(agent_id) = request.target_agent_id.as_deref() {
                metadata = metadata.agent(agent_id);
            }
            metadata
        }
        LocalDaemonRequest::SteerQueuedPrompt(request) => {
            LocalRequestMetadata::new("prompt.queued.steer", Interactive)
                .session(&request.session_id)
                .attachment(&request.attachment_id)
                .agent(&request.target_agent_id)
        }
        LocalDaemonRequest::CancelQueuedPrompt(request) => {
            LocalRequestMetadata::new("prompt.queued.cancel", Interactive)
                .session(&request.session_id)
                .attachment(&request.attachment_id)
                .agent(&request.target_agent_id)
        }
        LocalDaemonRequest::UpdateQueuedPrompt(request) => {
            LocalRequestMetadata::new("prompt.queued.update", Interactive)
                .session(&request.session_id)
                .attachment(&request.attachment_id)
                .agent(&request.target_agent_id)
        }
        LocalDaemonRequest::ResizeTerminal(request) => {
            LocalRequestMetadata::new("terminal.resize", Interactive).session(&request.session_id)
        }
        LocalDaemonRequest::SendTerminalInput(request) => {
            LocalRequestMetadata::new("terminal.input.send", Interactive)
                .session(&request.session_id)
                .attachment(&request.attachment_id)
        }
        LocalDaemonRequest::PollRuntimeNotices(request) => {
            LocalRequestMetadata::new("runtime_notice.poll", Interactive)
                .session(&request.session_id)
                .attachment(&request.attachment_id)
        }
        LocalDaemonRequest::RespondToInteraction(request) => {
            LocalRequestMetadata::new("interaction.respond", Interactive)
                .session(&request.session_id)
        }
        LocalDaemonRequest::ArmDeploymentCredentialEnrollment(request) => {
            LocalRequestMetadata::new("credential_enrollment.arm", Normal)
                .session(&request.session_id)
                .attachment(&request.attachment_id)
                .agent(&request.agent_id)
        }
        LocalDaemonRequest::RequestCredentialEnrollmentInteraction(request) => {
            LocalRequestMetadata::new("credential_enrollment.interaction.request", Normal)
                .session(&request.session_id)
                .agent(&request.agent_id)
        }
        LocalDaemonRequest::RequestNativeProviderInteraction(request) => {
            LocalRequestMetadata::new("native_provider.interaction.request", Normal)
                .session(&request.session_id)
                .agent(&request.agent_id)
        }
        LocalDaemonRequest::UpdateSessionConfig(request) => {
            LocalRequestMetadata::new("session.config.update", Interactive)
                .session(&request.session_id)
                .attachment(&request.attachment_id)
        }
        LocalDaemonRequest::UpdateAgentConfig(request) => {
            LocalRequestMetadata::new("agent.config.update", Interactive)
                .session(&request.session_id)
                .agent(&request.agent_id)
        }
        LocalDaemonRequest::UpdateAgentProfile(request) => {
            LocalRequestMetadata::new("agent.profile.update", Interactive)
                .session(&request.session_id)
                .agent(&request.agent_id)
        }
        LocalDaemonRequest::AliasAgent(request) => {
            LocalRequestMetadata::new("agent.alias", Interactive)
                .session(&request.session_id)
                .agent(&request.agent_id)
        }
        LocalDaemonRequest::UpdateAgentSubstitutes(request) => {
            LocalRequestMetadata::new("agent.substitutes.update", Interactive)
                .session(&request.session_id)
                .agent(&request.agent_id)
        }
        LocalDaemonRequest::AliasSession(request) => {
            LocalRequestMetadata::new("session.alias", Interactive).session(&request.session_id)
        }
        LocalDaemonRequest::FocusAgent(request) => {
            LocalRequestMetadata::new("agent.focus", Interactive)
                .session(&request.session_id)
                .agent(&request.agent_id)
        }
        LocalDaemonRequest::AcknowledgeAgentOutputSeen(request) => {
            LocalRequestMetadata::new("agent.output.seen", Interactive)
                .session(&request.session_id)
                .agent(&request.agent_id)
        }
        LocalDaemonRequest::CycleAgentFocus(request) => {
            LocalRequestMetadata::new("agent.cycle_focus", Interactive).session(&request.session_id)
        }
        LocalDaemonRequest::GrantAgentExtension(request) => {
            LocalRequestMetadata::new("agent.extension.grant", Interactive)
                .agent(&request.agent_ref)
        }
        LocalDaemonRequest::RevokeAgentExtension(request) => {
            LocalRequestMetadata::new("agent.extension.revoke", Interactive)
                .agent(&request.agent_ref)
        }
        LocalDaemonRequest::SyncRemoteExtensionManifest(request) => {
            LocalRequestMetadata::new("agent.extension.manifest_sync", Interactive)
                .agent(&request.agent_ref)
        }
        LocalDaemonRequest::ListHomeExtensionAudit(request) => {
            LocalRequestMetadata::new("agent.extension.audit", Interactive)
                .agent(&request.agent_ref)
        }
        LocalDaemonRequest::EndSession(request) => {
            LocalRequestMetadata::new("session.end", Interactive).session(&request.session_id)
        }
        LocalDaemonRequest::DeleteSession(request) => {
            LocalRequestMetadata::new("session.delete", Interactive).session(&request.session_ref)
        }
        LocalDaemonRequest::DeleteKernel(_) => LocalRequestMetadata::new("kernel.delete", Normal),
        LocalDaemonRequest::SpawnAgent(request) => {
            LocalRequestMetadata::new("agent.spawn", Interactive).session(&request.session_id)
        }
        LocalDaemonRequest::SpawnAgents(request) => {
            LocalRequestMetadata::new("agents.spawn", Interactive).session(&request.session_id)
        }
        LocalDaemonRequest::UndoTurn(request) => {
            LocalRequestMetadata::new("turn.undo", Interactive).session(&request.session_id)
        }
        LocalDaemonRequest::ForkAgent(request) => {
            LocalRequestMetadata::new("agent.fork", Interactive).session(&request.session_id)
        }
        LocalDaemonRequest::DestroyAgent(request) => {
            LocalRequestMetadata::new("agent.destroy", Interactive)
                .session(&request.session_id)
                .agent(&request.agent_id)
        }
        LocalDaemonRequest::GetSessionHistoryOutline(request) => {
            LocalRequestMetadata::new("session.history.outline", Background)
                .session(&request.session_id)
        }
        LocalDaemonRequest::GetSessionHistoryBlobContent(request) => {
            LocalRequestMetadata::new("session.history.blob", Background)
                .session(&request.session_id)
                .agent(&request.agent_id)
        }
        LocalDaemonRequest::GetPromptInputHistory(request) => {
            LocalRequestMetadata::new("prompt_input_history.get", Background)
                .session(&request.session_id)
        }
        LocalDaemonRequest::RecordPromptInputHistory(request) => {
            let mut metadata = LocalRequestMetadata::new("prompt_input_history.record", Normal)
                .session(&request.session_id);
            if let Some(attachment_id) = request.attachment_id.as_deref() {
                metadata = metadata.attachment(attachment_id);
            }
            metadata
        }
        LocalDaemonRequest::SearchMetaagentCommands(request) => {
            LocalRequestMetadata::new("metaagent.command.search", Normal)
                .session(&request.session_id)
                .agent(&request.metaagent_id)
        }
        LocalDaemonRequest::GetMetaagentTurnOverview(request) => {
            LocalRequestMetadata::new("metaagent.turn.overview", Normal)
                .session(&request.session_id)
                .agent(&request.metaagent_id)
        }
        LocalDaemonRequest::GetMetaagentTurnBlob(request) => {
            LocalRequestMetadata::new("metaagent.turn.blob", Normal)
                .session(&request.session_id)
                .agent(&request.metaagent_id)
        }
        LocalDaemonRequest::ListMetaagentEvents(request) => {
            LocalRequestMetadata::new("metaagent.event.list", Normal)
                .session(&request.session_id)
                .agent(&request.metaagent_id)
        }
        LocalDaemonRequest::ReadMetaagentEvent(request) => {
            LocalRequestMetadata::new("metaagent.event.read", Normal)
                .session(&request.session_id)
                .agent(&request.metaagent_id)
        }
        LocalDaemonRequest::AckMetaagentEvents(request) => {
            LocalRequestMetadata::new("metaagent.event.ack", Normal)
                .session(&request.session_id)
                .agent(&request.metaagent_id)
        }
        LocalDaemonRequest::UpdateMetaagentTask(request) => {
            LocalRequestMetadata::new("metaagent.task.update", Normal)
                .session(&request.session_id)
                .agent(&request.metaagent_id)
        }
        LocalDaemonRequest::PauseMetaagentTask(request) => {
            LocalRequestMetadata::new("metaagent.task.pause", Normal)
                .session(&request.session_id)
                .agent(&request.metaagent_id)
        }
        LocalDaemonRequest::ResumeMetaagentTask(request) => {
            LocalRequestMetadata::new("metaagent.task.resume", Normal)
                .session(&request.session_id)
                .agent(&request.metaagent_id)
        }
        LocalDaemonRequest::AbortMetaagentTask(request) => {
            LocalRequestMetadata::new("metaagent.task.abort", Normal)
                .session(&request.session_id)
                .agent(&request.metaagent_id)
        }
        LocalDaemonRequest::GetDaemonHealth(_) => {
            LocalRequestMetadata::new("daemon.health.get", Normal)
        }
        LocalDaemonRequest::ExportDebugBundle(request) => {
            LocalRequestMetadata::new("daemon.debug_bundle.export", Normal)
                .session(&request.session_id)
        }
        LocalDaemonRequest::GetProviderRun(request) => {
            LocalRequestMetadata::new("provider_run.get", Normal)
                .provider_run(&request.provider_run_id)
        }
        LocalDaemonRequest::UpdateProviderRunSelection(request) => {
            LocalRequestMetadata::new("provider_run.selection.update", Normal)
                .session(&request.session_id)
                .provider_run(&request.provider_run_id)
        }
        LocalDaemonRequest::CancelWorkflowRun(request) => {
            LocalRequestMetadata::new("workflow_run.cancel", Normal)
                .session(&request.session_id)
                .workflow_run(&request.workflow_run_ref)
        }
        LocalDaemonRequest::PauseWorkflowRun(request) => {
            LocalRequestMetadata::new("workflow_run.pause", Normal)
                .session(&request.session_id)
                .workflow_run(&request.workflow_run_ref)
        }
        LocalDaemonRequest::ResumeWorkflowRun(request) => {
            LocalRequestMetadata::new("workflow_run.resume", Normal)
                .session(&request.session_id)
                .workflow_run(&request.workflow_run_ref)
        }
        _ => LocalRequestMetadata::new(local_request_command_type(request), Normal),
    }
}

fn local_request_command_type(request: &LocalDaemonRequest) -> &'static str {
    match request {
        LocalDaemonRequest::CreateSession(_) => "session.create",
        LocalDaemonRequest::ListProjects(_) => "project.list",
        LocalDaemonRequest::RenameProject(_) => "project.rename",
        LocalDaemonRequest::UpdateProjectWorkspaces(_) => "project.workspaces.update",
        LocalDaemonRequest::ArchiveProject(_) => "project.archive",
        LocalDaemonRequest::DeleteProject(_) => "project.delete",
        LocalDaemonRequest::RestoreProject(_) => "project.restore",
        LocalDaemonRequest::LaunchProviderRun(_) => "provider_run.launch",
        LocalDaemonRequest::LaunchProviderRuns(_) => "provider_runs.launch",
        LocalDaemonRequest::UpdateProviderRunSelection(_) => "provider_run.selection.update",
        LocalDaemonRequest::ListSessionMembers(_) => "session.members.list",
        LocalDaemonRequest::CreateSessionInvite(_) => "session.invite.create",
        LocalDaemonRequest::JoinSessionInvite(_) => "session.invite.join",
        LocalDaemonRequest::RevokeSessionInvite(_) => "session.invite.revoke",
        LocalDaemonRequest::CreateWorkspaceLink(_) => "workspace_link.create",
        LocalDaemonRequest::ListWorkspaceLinks(_) => "workspace_link.list",
        LocalDaemonRequest::ShowWorkspaceLink(_) => "workspace_link.show",
        LocalDaemonRequest::AttachWorkspaceLink(_) => "workspace_link.attach",
        LocalDaemonRequest::DetachWorkspaceLink(_) => "workspace_link.detach",
        LocalDaemonRequest::GetWorkspaceLiveSyncStatus(_) => "workspace_live_sync.status",
        LocalDaemonRequest::SetWorkspaceLiveSyncMode(_) => "workspace_live_sync.mode",
        LocalDaemonRequest::ListSessions(_) => "session.list",
        LocalDaemonRequest::ResolveSession(_) => "session.resolve",
        LocalDaemonRequest::GetSessionState(_) => "session.state.get",
        LocalDaemonRequest::GetRoomEnvironmentState(_) => "environment.state.get",
        LocalDaemonRequest::GetRoomEnvironmentSlice(_) => "environment.slice.get",
        LocalDaemonRequest::BindRoomEnvironmentSlice(_) => "environment.slice.bind",
        LocalDaemonRequest::CaptureRoomEnvironmentScreenshot(_) => "environment.screenshot.capture",
        LocalDaemonRequest::ReadRoomEnvironmentScreenshotChunk(_) => "environment.screenshot.read",
        LocalDaemonRequest::GetRoomEnvironmentEvents(_) => "environment.events.get",
        LocalDaemonRequest::ListRoomEnvironmentActionHistory(_) => "environment.history.list",
        LocalDaemonRequest::StartRoomEnvironment(_) => "environment.start",
        LocalDaemonRequest::StopRoomEnvironment(_) => "environment.stop",
        LocalDaemonRequest::RetryRoomEnvironment(_) => "environment.retry",
        LocalDaemonRequest::UpdateRoomEnvironmentViewport(_) => "environment.viewport.update",
        LocalDaemonRequest::UpdateRoomEnvironmentPointer(_) => "environment.pointer.update",
        LocalDaemonRequest::RequestRoomEnvironmentInputTakeover(_) => "environment.input.takeover",
        LocalDaemonRequest::ReleaseRoomEnvironmentInput(_) => "environment.input.release",
        LocalDaemonRequest::SubmitRoomEnvironmentAction(_) => "environment.action.submit",
        LocalDaemonRequest::SubmitRoomEnvironmentBrowserAction(_) => {
            "environment.browser_action.submit"
        }
        LocalDaemonRequest::ReadRoomEnvironmentClipboard(_) => "environment.clipboard.read",
        LocalDaemonRequest::CancelRoomEnvironmentAction(_) => "environment.action.cancel",
        LocalDaemonRequest::SearchMetaagentCommands(_) => "metaagent.command.search",
        LocalDaemonRequest::GetMetaagentTurnOverview(_) => "metaagent.turn.overview",
        LocalDaemonRequest::GetMetaagentTurnBlob(_) => "metaagent.turn.blob",
        LocalDaemonRequest::ListMetaagentEvents(_) => "metaagent.event.list",
        LocalDaemonRequest::ReadMetaagentEvent(_) => "metaagent.event.read",
        LocalDaemonRequest::AckMetaagentEvents(_) => "metaagent.event.ack",
        LocalDaemonRequest::UpdateMetaagentTask(_) => "metaagent.task.update",
        LocalDaemonRequest::PauseMetaagentTask(_) => "metaagent.task.pause",
        LocalDaemonRequest::ResumeMetaagentTask(_) => "metaagent.task.resume",
        LocalDaemonRequest::AbortMetaagentTask(_) => "metaagent.task.abort",
        LocalDaemonRequest::GetTerminalCommandCatalog(_) => "terminal.command_catalog.get",
        LocalDaemonRequest::GetProviderCatalog(_) => "provider.catalog.get",
        LocalDaemonRequest::GetProviderCommandCatalogs(_) => "provider.command_catalogs.get",
        LocalDaemonRequest::ListProviderAccountProfiles(_) => "provider_account.list",
        LocalDaemonRequest::GetProviderAccountProfile(_) => "provider_account.get",
        LocalDaemonRequest::CreateProviderAccountProfile(_) => "provider_account.create",
        LocalDaemonRequest::LinkProviderAccountProfile(_) => "provider_account.link",
        LocalDaemonRequest::ImportNativeProviderAccountProfile(_) => {
            "provider_account.native.import"
        }
        LocalDaemonRequest::RenameProviderAccountProfile(_) => "provider_account.rename",
        LocalDaemonRequest::SetDefaultProviderAccountProfile(_) => "provider_account.default.set",
        LocalDaemonRequest::RefreshProviderAccountProfile(_) => "provider_account.refresh",
        LocalDaemonRequest::RemoveProviderAccountProfile(_) => "provider_account.remove",
        LocalDaemonRequest::DeleteProviderAccountProfileData(_) => "provider_account.data.delete",
        LocalDaemonRequest::InstallMcpServer(_) => "mcp.install",
        LocalDaemonRequest::UpdateMcpServer(_) => "mcp.update",
        LocalDaemonRequest::UninstallMcpServer(_) => "mcp.uninstall",
        LocalDaemonRequest::ImportMcpServers(_) => "mcp.import",
        LocalDaemonRequest::ImportProviderCapabilities(_) => "extension.import.providers",
        LocalDaemonRequest::GetMcpServer(_) => "mcp.get",
        LocalDaemonRequest::ListMcpServers(_) => "mcp.list",
        LocalDaemonRequest::RegisterEnvironment(_) => "env.register",
        LocalDaemonRequest::RemoveEnvironment(_) => "env.remove",
        LocalDaemonRequest::GetEnvironment(_) => "env.get",
        LocalDaemonRequest::ListEnvironments(_) => "env.list",
        LocalDaemonRequest::ValidateScript(_) => "script.validate",
        LocalDaemonRequest::RegisterScript(_) => "script.register",
        LocalDaemonRequest::RemoveScript(_) => "script.remove",
        LocalDaemonRequest::GetScript(_) => "script.get",
        LocalDaemonRequest::ListScripts(_) => "script.list",
        LocalDaemonRequest::InstallSkill(_) => "skill.install",
        LocalDaemonRequest::UpsertSkill(_) => "skill.upsert",
        LocalDaemonRequest::UpdateSkill(_) => "skill.update",
        LocalDaemonRequest::UninstallSkill(_) => "skill.uninstall",
        LocalDaemonRequest::ImportSkills(_) => "skill.import",
        LocalDaemonRequest::GetSkill(_) => "skill.get",
        LocalDaemonRequest::ListSkills(_) => "skill.list",
        LocalDaemonRequest::RelayStatus(_) => "relay.status",
        LocalDaemonRequest::ConfigureRelay(_) => "relay.configure",
        LocalDaemonRequest::CloudRelayStatus(_) => "cloud_relay.status",
        LocalDaemonRequest::StartCloudRelayLogin(_) => "cloud_relay.login.start",
        LocalDaemonRequest::PollCloudRelayLogin(_) => "cloud_relay.login.poll",
        LocalDaemonRequest::LogoutCloudRelay(_) => "cloud_relay.logout",
        LocalDaemonRequest::PairCloudRelayClient(_) => "cloud_relay.client.pair",
        LocalDaemonRequest::PairCloudRelayMachine(_) => "cloud_relay.machine.pair",
        LocalDaemonRequest::ConnectCloudRelay(_) => "cloud_relay.connect",
        LocalDaemonRequest::IssueCloudRelayClientToken(_) => "cloud_relay.client_token.issue",
        LocalDaemonRequest::ResolveKernelClientConnection(_) => "kernel.client_connection.resolve",
        LocalDaemonRequest::CreateCloudSessionInvite(_) => "cloud_session.invite.create",
        LocalDaemonRequest::ShowCloudSessionInvite(_) => "cloud_session.invite.show",
        LocalDaemonRequest::AcceptCloudSessionInvite(_) => "cloud_session.invite.accept",
        LocalDaemonRequest::RevokeCloudSessionInvite(_) => "cloud_session.invite.revoke",
        LocalDaemonRequest::ListCloudSessionMembers(_) => "cloud_session.members.list",
        LocalDaemonRequest::ListCloudCollaborators(_) => "cloud_session.collaborators.list",
        LocalDaemonRequest::GetUserConfig(_) => "config.get",
        LocalDaemonRequest::GetUserConfigSchema(_) => "config.schema",
        LocalDaemonRequest::ListPromptSettings(_) => "prompt_settings.list",
        LocalDaemonRequest::GetPromptSetting(_) => "prompt_settings.get",
        LocalDaemonRequest::UpdatePromptSetting(_) => "prompt_settings.update",
        LocalDaemonRequest::PreviewPromptSetting(_) => "prompt_settings.preview",
        LocalDaemonRequest::ResetPromptSetting(_) => "prompt_settings.reset",
        LocalDaemonRequest::ResetAllPromptSettings(_) => "prompt_settings.reset_all",
        LocalDaemonRequest::SetUserConfigValue(_) => "config.set",
        LocalDaemonRequest::UnsetUserConfigValue(_) => "config.unset",
        LocalDaemonRequest::SetCredentialSecret(_) => "credential.secret.set",
        LocalDaemonRequest::DeleteCredentialSecret(_) => "credential.secret.delete",
        LocalDaemonRequest::SetProviderAccountCredential(_) => "provider_account.credential.set",
        LocalDaemonRequest::GetCredentialVaultStatus(_) => "credential_vault.status",
        LocalDaemonRequest::LockCredentialVault(_) => "credential_vault.lock",
        LocalDaemonRequest::ManageCredentialVault(_) => "credential_vault.manage",
        LocalDaemonRequest::RegisterCredential(_) => "credential.register",
        LocalDaemonRequest::UpsertCredential(_) => "credential.upsert",
        LocalDaemonRequest::RemoveCredential(_) => "credential.remove",
        LocalDaemonRequest::GetCredential(_) => "credential.get",
        LocalDaemonRequest::ListCredentials(_) => "credential.list",
        LocalDaemonRequest::RegisterConnector(_) => "connector.register",
        LocalDaemonRequest::UpsertConnector(_) => "connector.upsert",
        LocalDaemonRequest::RegisterConnectorAdapter(_) => "connector.adapter.register",
        LocalDaemonRequest::RemoveConnectorAdapter(_) => "connector.adapter.remove",
        LocalDaemonRequest::GetConnectorAdapter(_) => "connector.adapter.get",
        LocalDaemonRequest::ListConnectorAdapters(_) => "connector.adapter.list",
        LocalDaemonRequest::RemoveConnector(_) => "connector.remove",
        LocalDaemonRequest::GetConnector(_) => "connector.get",
        LocalDaemonRequest::ListConnectors(_) => "connector.list",
        LocalDaemonRequest::TestConnector(_) => "connector.test",
        LocalDaemonRequest::ListSlices(_) => "slice.list",
        LocalDaemonRequest::CreateSlice(_) => "slice.create",
        LocalDaemonRequest::GetSlice(_) => "slice.get",
        LocalDaemonRequest::StartSlice(_) => "slice.start",
        LocalDaemonRequest::StopSlice(_) => "slice.stop",
        LocalDaemonRequest::DeleteSlice(_) => "slice.delete",
        LocalDaemonRequest::ImportSliceProviderAuth(_) => "slice.auth.import",
        LocalDaemonRequest::RemoveSliceProviderAuth(_) => "slice.auth.remove",
        LocalDaemonRequest::StartSliceProviderLogin(_) => "slice.auth.login",
        LocalDaemonRequest::GetSliceDisplayEndpoint(_) => "slice.display_endpoint.get",
        LocalDaemonRequest::GetSliceLogs(_) => "slice.logs.get",
        LocalDaemonRequest::ListSliceAudit(_) => "slice.audit.list",
        LocalDaemonRequest::SaveSliceState(_) => "slice.state.save",
        LocalDaemonRequest::GetSliceStateStatus(_) => "slice.state.status",
        LocalDaemonRequest::ResetSliceState(_) => "slice.state.reset",
        LocalDaemonRequest::CreateSliceBackup(_) => "slice.backup.create",
        LocalDaemonRequest::RestoreSliceBackup(_) => "slice.backup.restore",
        LocalDaemonRequest::ListRemoteMachines(_) => "remote_machine.list",
        LocalDaemonRequest::ListRemoteMachineKernels(_) => "remote_machine.kernel.list",
        LocalDaemonRequest::GetWaitingRoomInventory(_) => "waiting_room.inventory.get",
        LocalDaemonRequest::GetWaitingRoomPublicSnapshot(_) => "waiting_room.public_snapshot.get",
        LocalDaemonRequest::ListExternalProviderSessions(_) => "external_provider_session.list",
        LocalDaemonRequest::RefreshExternalProviderSessions(_) => {
            "external_provider_session.refresh"
        }
        LocalDaemonRequest::ImportExternalProviderSession(_) => {
            "external_provider_session.import_session"
        }
        LocalDaemonRequest::ImportExternalProviderAgent(_) => {
            "external_provider_session.import_agent"
        }
        LocalDaemonRequest::SearchWorkspaceDirectories(_) => "workspace.directory.search",
        LocalDaemonRequest::CreateWorkspaceDirectory(_) => "workspace.directory.create",
        LocalDaemonRequest::ListWorkspaceWorktrees(_) => "workspace.worktree.list",
        LocalDaemonRequest::CreateWorkspaceWorktree(_) => "workspace.worktree.create",
        LocalDaemonRequest::DeleteWorkspaceWorktree(_) => "workspace.worktree.delete",
        LocalDaemonRequest::CreateWorkspacePullRequest(_) => "workspace.pull_request.create",
        LocalDaemonRequest::GetWorkspaceGitOverview(_) => "workspace.git.overview",
        LocalDaemonRequest::ListWorkspaceFiles(_) => "workspace.files.list",
        LocalDaemonRequest::GetWorkspaceFileContent(_) => "workspace.file.content",
        LocalDaemonRequest::RunAgentUtility(_) => "agent.utility.run",
        LocalDaemonRequest::GenerateWorkspaceCommitMessage(_) => {
            "workspace.commit_message.generate"
        }
        LocalDaemonRequest::CommitWorkspaceChanges(_) => "workspace.git.commit",
        LocalDaemonRequest::PushWorkspaceBranch(_) => "workspace.git.push",
        LocalDaemonRequest::CommitAndPushWorkspaceChanges(_) => "workspace.git.commit_and_push",
        LocalDaemonRequest::ApproveRemoteMachine(_) => "remote_machine.approve",
        LocalDaemonRequest::ForgetRemoteMachine(_) => "remote_machine.forget",
        LocalDaemonRequest::RenameRemoteMachine(_) => "remote_machine.rename",
        LocalDaemonRequest::CreatePairingInvite(_) => "pairing_invite.create",
        LocalDaemonRequest::JoinPairingInvite(_) => "pairing_invite.join",
        LocalDaemonRequest::CreateTerminalPairingLink(_) => "terminal_pairing_link.create",
        LocalDaemonRequest::JoinTerminalPairingLink(_) => "terminal_pairing_link.join",
        LocalDaemonRequest::ListTerminals(_) => "terminal.list",
        LocalDaemonRequest::ListPairedClients(_) => "paired_client.list",
        LocalDaemonRequest::RecordPairedClient(_) => "paired_client.record",
        LocalDaemonRequest::RevokePairedClient(_) => "paired_client.revoke",
        LocalDaemonRequest::GetProviderAuthStatus(_) => "provider.auth_status.get",
        LocalDaemonRequest::StartProviderLogin(_) => "provider.login.start",
        LocalDaemonRequest::GetProviderLoginStatus(_) => "provider.login.status",
        LocalDaemonRequest::SendProviderLoginInput(_) => "provider.login.input",
        LocalDaemonRequest::CancelProviderLogin(_) => "provider.login.cancel",
        LocalDaemonRequest::LogoutProvider(_) => "provider.logout",
        LocalDaemonRequest::ListProviderProcesses(_) => "provider_process.list",
        LocalDaemonRequest::TeardownProviderProcesses(_) => "provider_process.teardown",
        LocalDaemonRequest::QueryRecall(_) => "recall.query",
        LocalDaemonRequest::SearchRecall(_) => "recall.search",
        LocalDaemonRequest::SemanticSearchRecall(_) => "recall.semantic_search",
        LocalDaemonRequest::GetPromptInputHistory(_) => "prompt_input_history.get",
        LocalDaemonRequest::RecordPromptInputHistory(_) => "prompt_input_history.record",
        LocalDaemonRequest::PollRuntimeNotices(_) => "runtime_notice.poll",
        LocalDaemonRequest::RespondToInteraction(_) => "interaction.respond",
        LocalDaemonRequest::ArmDeploymentCredentialEnrollment(_) => "credential_enrollment.arm",
        LocalDaemonRequest::RequestCredentialEnrollmentInteraction(_) => {
            "credential_enrollment.interaction.request"
        }
        LocalDaemonRequest::RequestNativeProviderInteraction(_) => {
            "native_provider.interaction.request"
        }
        LocalDaemonRequest::CompletePrompt(_) => "prompt.complete",
        LocalDaemonRequest::CreateAgentPromptSchedule(_) => "agent_prompt_schedule.create",
        LocalDaemonRequest::CancelAgentPromptSchedule(_) => "agent_prompt_schedule.cancel",
        LocalDaemonRequest::UpdateSessionConfig(_) => "session.config.update",
        LocalDaemonRequest::UpdateAgentConfig(_) => "agent.config.update",
        LocalDaemonRequest::UpdateAgentSubstitutes(_) => "agent.substitutes.update",
        LocalDaemonRequest::PumpTerminalOutput(_) => "terminal.output.poll",
        LocalDaemonRequest::AppendNativeProviderOutput(_) => "terminal.output.append_native",
        LocalDaemonRequest::AppendNativeProviderOutputBatch(_) => {
            "terminal.output.append_native_batch"
        }
        LocalDaemonRequest::RunShellCommand(_) => "capability.shell.run",
        LocalDaemonRequest::ReadDirectoryTree(_) => "capability.dir.tree",
        LocalDaemonRequest::ReadFile(_) => "capability.file.read",
        LocalDaemonRequest::EditFile(_) => "capability.file.edit",
        LocalDaemonRequest::InspectGit(_) => "capability.git.inspect",
        LocalDaemonRequest::CaptureScreenshot(_) => "capability.screenshot.capture",
        LocalDaemonRequest::StoreTransferredFile(_) => "capability.file.store_transferred",
        LocalDaemonRequest::AliasSession(_) => "session.alias",
        LocalDaemonRequest::AliasAgent(_) => "agent.alias",
        LocalDaemonRequest::UpdateAgentProfile(_) => "agent.profile.update",
        LocalDaemonRequest::SpawnAgent(_) => "agent.spawn",
        LocalDaemonRequest::SpawnAgents(_) => "agents.spawn",
        LocalDaemonRequest::UndoTurn(_) => "turn.undo",
        LocalDaemonRequest::ForkAgent(_) => "agent.fork",
        LocalDaemonRequest::MoveAgentToRemote(_) => "agent.move_remote",
        LocalDaemonRequest::MoveAgentToLocal(_) => "agent.move_local",
        LocalDaemonRequest::DestroyAgent(_) => "agent.destroy",
        LocalDaemonRequest::GrantAgentExtension(_) => "agent.extension.grant",
        LocalDaemonRequest::RevokeAgentExtension(_) => "agent.extension.revoke",
        LocalDaemonRequest::SyncRemoteExtensionManifest(_) => "agent.extension.manifest_sync",
        LocalDaemonRequest::ListHomeExtensionAudit(_) => "agent.extension.audit",
        LocalDaemonRequest::ListAgents(_) => "agent.list",
        LocalDaemonRequest::CreateWorkflow(_) => "workflow.create",
        LocalDaemonRequest::ValidateWorkflowCode(_) => "workflow_code.validate",
        LocalDaemonRequest::ApplyWorkflowCode(_) => "workflow_code.apply",
        LocalDaemonRequest::ApplyWorkflowCodeArtifact(_) => "workflow_code_artifact.apply",
        LocalDaemonRequest::RunWorkflowCode(_) => "workflow_code.run",
        LocalDaemonRequest::RunWorkflowCodeArtifact(_) => "workflow_code_artifact.run",
        LocalDaemonRequest::ListWorkflowRegistry(_) => "workflow_registry.list",
        LocalDaemonRequest::GetWorkflowRegistryEntry(_) => "workflow_registry.get",
        LocalDaemonRequest::AddWorkflowRegistryEntry(_) => "workflow_registry.add",
        LocalDaemonRequest::AddWorkflowRegistryEntryFromWorkflow(_) => {
            "workflow_registry.add_from_workflow"
        }
        LocalDaemonRequest::DeleteWorkflowRegistryEntry(_) => "workflow_registry.delete",
        LocalDaemonRequest::LoadWorkflowRegistryEntry(_) => "workflow_registry.load",
        LocalDaemonRequest::RunWorkflowRegistryEntry(_) => "workflow_registry.run",
        LocalDaemonRequest::CreateWorkflowCodeArtifact(_) => "workflow_code_artifact.create",
        LocalDaemonRequest::UpdateWorkflowCodeArtifact(_) => "workflow_code_artifact.update",
        LocalDaemonRequest::BindWorkflowCodeSource(_) => "workflow_code_source.bind",
        LocalDaemonRequest::RebuildWorkflowCodeSource(_) => "workflow_code_source.rebuild",
        LocalDaemonRequest::UpdateWorkflowCodeSourceFromWorkflow(_) => {
            "workflow_code_source.update_from_workflow"
        }
        LocalDaemonRequest::GetWorkflowCodeArtifact(_) => "workflow_code_artifact.get",
        LocalDaemonRequest::ListWorkflowCodeArtifacts(_) => "workflow_code_artifact.list",
        LocalDaemonRequest::DeleteWorkflowCodeArtifact(_) => "workflow_code_artifact.delete",
        LocalDaemonRequest::ExportWorkflowCodeArtifact(_) => "workflow_code_artifact.export",
        LocalDaemonRequest::ImportWorkflowCodeArtifact(_) => "workflow_code_artifact.import",
        LocalDaemonRequest::ExportWorkflowCodePackage(_) => "workflow_code_package.export",
        LocalDaemonRequest::ImportWorkflowCodePackage(_) => "workflow_code_package.import",
        LocalDaemonRequest::ExportWorkflowCodeSource(_) => "workflow_code_source.export",
        LocalDaemonRequest::ApplyWorkflowDesignOp(_) => "workflow_design.apply_op",
        LocalDaemonRequest::AliasWorkflow(_) => "workflow.alias",
        LocalDaemonRequest::ListWorkflows(_) => "workflow.list",
        LocalDaemonRequest::ResolveWorkflow(_) => "workflow.resolve",
        LocalDaemonRequest::CreateWorkflowPublication(_) => "workflow_publication.create",
        LocalDaemonRequest::ListWorkflowPublications(_) => "workflow_publication.list",
        LocalDaemonRequest::GetWorkflowPublication(_) => "workflow_publication.get",
        LocalDaemonRequest::ExportWorkflowPublicationPackage(_) => {
            "workflow_publication.package.export"
        }
        LocalDaemonRequest::DisableWorkflowPublication(_) => "workflow_publication.disable",
        LocalDaemonRequest::GetEventGeneratorCatalogLanding(_) => "event_catalog.landing",
        LocalDaemonRequest::SearchEventGeneratorCatalog(_) => "event_catalog.search",
        LocalDaemonRequest::BrowseEventGeneratorCategory(_) => "event_catalog.category.browse",
        LocalDaemonRequest::GetEventGeneratorDetail(_) => "event_catalog.generator.get",
        LocalDaemonRequest::BrowseEventGeneratorEvents(_) => "event_catalog.events.browse",
        LocalDaemonRequest::StartEventGeneratorAuthorization(_) => {
            "event_generator.authorization.start"
        }
        LocalDaemonRequest::ListEventGeneratorResources(_) => "event_generator.resources.list",
        LocalDaemonRequest::ListEventConnections(_) => "event_connection.list",
        LocalDaemonRequest::GetEventConnection(_) => "event_connection.get",
        LocalDaemonRequest::InstallEventConnection(_) => "event_connection.install",
        LocalDaemonRequest::ObserveEventConnectionAuthorization(_) => {
            "event_connection.authorization.observe"
        }
        LocalDaemonRequest::RefreshEventConnection(_) => "event_connection.refresh",
        LocalDaemonRequest::TestEventConnection(_) => "event_connection.test",
        LocalDaemonRequest::ReconnectEventConnection(_) => "event_connection.reconnect",
        LocalDaemonRequest::ListEventConnectionResources(_) => "event_connection.resources.list",
        LocalDaemonRequest::ListEventConnectionDependencies(_) => {
            "event_connection.dependencies.list"
        }
        LocalDaemonRequest::RemoveEventConnection(_) => "event_connection.remove",
        LocalDaemonRequest::CreateWorkflowEventBinding(_) => "workflow_event_binding.create",
        LocalDaemonRequest::ListWorkflowEventBindings(_) => "workflow_event_binding.list",
        LocalDaemonRequest::SetWorkflowEventBindingStatus(_) => "workflow_event_binding.status.set",
        LocalDaemonRequest::TransferWorkflowEventBinding(_) => "workflow_event_binding.transfer",
        LocalDaemonRequest::TestWorkflowEventBinding(_) => "workflow_event_binding.test",
        LocalDaemonRequest::GetEventDeliveryStatus(_) => "event_delivery.status",
        LocalDaemonRequest::ControlWorkflowPublicationRuntime(_) => {
            "workflow_publication.runtime.control"
        }
        LocalDaemonRequest::BindWorkflowPublicationDeployment(_) => {
            "workflow_publication.deployment.bind"
        }
        LocalDaemonRequest::RegisterWorkflowPublicationEndpoint(_) => {
            "workflow_publication.endpoint.register"
        }
        LocalDaemonRequest::MaterializeWorkflowPublication(_) => "workflow_publication.materialize",
        LocalDaemonRequest::ActivateWorkflowPublicationRuntime(_) => {
            "workflow_publication.runtime.activate"
        }
        LocalDaemonRequest::CreateWorkflowEndpoint(_) => "workflow_endpoint.create",
        LocalDaemonRequest::AliasWorkflowEndpoint(_) => "workflow_endpoint.alias",
        LocalDaemonRequest::BindWorkflowEndpoint(_) => "workflow_endpoint.bind",
        LocalDaemonRequest::AddWorkflowNode(_) => "workflow_node.add",
        LocalDaemonRequest::RemoveWorkflowNode(_) => "workflow_node.remove",
        LocalDaemonRequest::UpdateWorkflowNodeInstructions(_) => {
            "workflow_node.instructions.update"
        }
        LocalDaemonRequest::SetWorkflowNodeCanCompleteRun(_) => {
            "workflow_node.can_complete_run.set"
        }
        LocalDaemonRequest::SetWorkflowNodeCanEmitIntermediateOutput(_) => {
            "workflow_node.can_emit_intermediate_output.set"
        }
        LocalDaemonRequest::SetWorkflowNodeWaitForAllInputs(_) => {
            "workflow_node.wait_for_all_inputs.set"
        }
        LocalDaemonRequest::SetWorkflowNodeIntermediateOutputSchema(_) => {
            "workflow_node.intermediate_output_schema.set"
        }
        LocalDaemonRequest::SetWorkflowNodeMaxTurns(_) => "workflow_node.max_turns.set",
        LocalDaemonRequest::AddWorkflowEdge(_) => "workflow_edge.add",
        LocalDaemonRequest::RemoveWorkflowEdge(_) => "workflow_edge.remove",
        LocalDaemonRequest::UpdateWorkflowCanvasLayout(_) => "workflow_canvas.layout.update",
        LocalDaemonRequest::SetWorkflowRunOutputSchema(_) => "workflow.run_output_schema.set",
        LocalDaemonRequest::SetWorkflowFlushContext(_) => "workflow.flush_context.set",
        LocalDaemonRequest::InvokeWorkflowEndpoint(_) => "workflow_prompt.enqueue",
        LocalDaemonRequest::ListWorkflowRuns(_) => "workflow_run.list",
        LocalDaemonRequest::GetWorkflowRun(_) => "workflow_run.get",
        LocalDaemonRequest::AckWorkflowTurn(_) => "workflow_turn.ack",
        LocalDaemonRequest::ValidateWorkflowHandoff(_) => "workflow_handoff.validate",
        LocalDaemonRequest::CancelWorkflowRun(_) => "workflow_run.cancel",
        LocalDaemonRequest::PauseWorkflowRun(_) => "workflow_run.pause",
        LocalDaemonRequest::ResumeWorkflowRun(_) => "workflow_run.resume",
        LocalDaemonRequest::ListWorkflowPromptQueues(_) => "workflow_prompt_queue.list",
        LocalDaemonRequest::CreateWorkflowPromptQueue(_) => "workflow_prompt_queue.create",
        LocalDaemonRequest::UpdateWorkflowPromptQueue(_) => "workflow_prompt_queue.update",
        LocalDaemonRequest::RemoveWorkflowPromptQueue(_) => "workflow_prompt_queue.remove",
        LocalDaemonRequest::ListQueuedWorkflowPrompts(_) => "workflow_queued_prompt.list",
        LocalDaemonRequest::UpdateQueuedWorkflowPrompt(_) => "workflow_queued_prompt.update",
        LocalDaemonRequest::RemoveQueuedWorkflowPrompt(_) => "workflow_queued_prompt.remove",
        LocalDaemonRequest::ClearWorkflowPromptQueue(_) => "workflow_prompt_queue.clear",
        LocalDaemonRequest::CreateWorkflowWatchdog(_) => "workflow_watchdog.create",
        LocalDaemonRequest::RemoveWorkflowWatchdog(_) => "workflow_watchdog.remove",
        LocalDaemonRequest::SetWorkflowWatchdogEnabled(_) => "workflow_watchdog.enabled.set",
        LocalDaemonRequest::ListWorkflowWatchdogs(_) => "workflow_watchdog.list",
        LocalDaemonRequest::CreateWorkflowSchedule(_) => "workflow_schedule.create",
        LocalDaemonRequest::RemoveWorkflowSchedule(_) => "workflow_schedule.remove",
        LocalDaemonRequest::SetWorkflowScheduleEnabled(_) => "workflow_schedule.enabled.set",
        LocalDaemonRequest::ListWorkflowSchedules(_) => "workflow_schedule.list",
        LocalDaemonRequest::PreviewWorkflowSchedule(_) => "workflow_schedule.preview",
        LocalDaemonRequest::AttachToSession(_)
        | LocalDaemonRequest::DetachFromSession(_)
        | LocalDaemonRequest::SubmitPrompt(_)
        | LocalDaemonRequest::SubmitPrompts(_)
        | LocalDaemonRequest::CancelActivePrompt(_)
        | LocalDaemonRequest::SteerQueuedPrompt(_)
        | LocalDaemonRequest::CancelQueuedPrompt(_)
        | LocalDaemonRequest::UpdateQueuedPrompt(_)
        | LocalDaemonRequest::ResizeTerminal(_)
        | LocalDaemonRequest::SendTerminalInput(_)
        | LocalDaemonRequest::FocusAgent(_)
        | LocalDaemonRequest::AcknowledgeAgentOutputSeen(_)
        | LocalDaemonRequest::CycleAgentFocus(_)
        | LocalDaemonRequest::EndSession(_)
        | LocalDaemonRequest::DeleteSession(_)
        | LocalDaemonRequest::DeleteKernel(_)
        | LocalDaemonRequest::GetDaemonHealth(_)
        | LocalDaemonRequest::ExportDebugBundle(_)
        | LocalDaemonRequest::ListManagedEnvironmentCatalog(_)
        | LocalDaemonRequest::GetManagedEnvironment(_)
        | LocalDaemonRequest::PrepareManagedEnvironmentContextTransfer(_)
        | LocalDaemonRequest::CreateManagedEnvironment(_)
        | LocalDaemonRequest::RequestManagedEnvironmentLifecycle(_)
        | LocalDaemonRequest::StartManagedContextTransfer(_)
        | LocalDaemonRequest::GetManagedContextTransferStatus(_)
        | LocalDaemonRequest::GetManagedContextLaunchTarget(_)
        | LocalDaemonRequest::GetSessionHistoryOutline(_)
        | LocalDaemonRequest::GetSessionHistoryBlobContent(_)
        | LocalDaemonRequest::GetProviderRun(_) => unreachable!("handled by metadata matcher"),
    }
}
