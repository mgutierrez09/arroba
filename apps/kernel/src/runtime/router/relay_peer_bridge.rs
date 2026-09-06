use super::CommandRouter;
use crate::error::DaemonError;
use crate::runtime::native_interaction_bridge::forward_relay_native_interaction;
use crate::runtime::relay_peer_runtime_executor as relay_peer_runtime;

impl CommandRouter {
    pub(crate) async fn relay_room_browser_controller(
        &self,
        kernel_id: &str,
        public_key: &str,
        session_id: &str,
        slice_id: &str,
        command: crate::transport::room_browser_controller::RoomBrowserControllerCommand,
    ) -> Result<crate::transport::room_browser_controller::RoomBrowserControllerResult, DaemonError>
    {
        self.runtime_state
            .execute_bound_room_browser_controller(
                kernel_id, public_key, session_id, slice_id, command,
            )
            .await
    }

    pub(crate) async fn relay_open_room_display(
        &self,
        kernel_id: &str,
        public_key: &str,
        session_id: &str,
        slice_id: &str,
        viewer_public_key: String,
    ) -> Result<crate::slice::SliceDisplayEndpoint, DaemonError> {
        self.runtime_state
            .execute_bound_room_display_open(
                kernel_id,
                public_key,
                session_id,
                slice_id,
                viewer_public_key,
            )
            .await
    }

    pub(crate) async fn relay_capture_room_screenshot(
        &self,
        kernel_id: &str,
        public_key: &str,
        session_id: &str,
        slice_id: &str,
    ) -> Result<crate::local::RoomEnvironmentScreenshotArtifact, DaemonError> {
        self.runtime_state
            .execute_bound_room_screenshot_capture(kernel_id, public_key, session_id, slice_id)
            .await
    }

    pub(crate) fn relay_read_room_screenshot_chunk(
        &self,
        kernel_id: &str,
        public_key: &str,
        session_id: &str,
        slice_id: &str,
        artifact_id: &str,
        offset: u64,
        max_bytes: u32,
    ) -> Result<crate::local::RoomEnvironmentScreenshotChunk, DaemonError> {
        self.runtime_state.execute_bound_room_screenshot_chunk(
            kernel_id,
            public_key,
            session_id,
            slice_id,
            artifact_id,
            offset,
            max_bytes,
        )
    }

    pub(crate) async fn relay_observe_room_computer(
        &self,
        kernel_id: &str,
        public_key: &str,
        session_id: &str,
        slice_id: &str,
        call: crate::transport::relay_peer::RemoteRoomComputerObservationCall,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        self.runtime_state
            .execute_bound_room_computer_observation(
                kernel_id, public_key, session_id, slice_id, call,
            )
            .await
    }

    pub(crate) fn relay_daemon_id(&self) -> String {
        self.config_projection.snapshot().daemon_id
    }

    pub(crate) fn relay_private_key(&self) -> String {
        self.config_projection.snapshot().relay_private_key
    }

    pub(crate) async fn relay_registration(&self) -> chariox_relay::protocol::DaemonRegistration {
        self.runtime_state.relay_registration().await
    }

    pub(crate) async fn ensure_relay_subscription_attachment(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<(), DaemonError> {
        relay_peer_runtime::ensure_relay_subscription_attachment(
            &self.runtime_state,
            &self.session_projection,
            session_id,
            attachment_id,
        )
        .await
    }

    pub(crate) async fn ensure_relay_subscription_attachment_for_user(
        &self,
        session_id: &str,
        attachment_id: &str,
        user_id: &str,
    ) -> Result<(), DaemonError> {
        self.ensure_relay_subscription_attachment(session_id, attachment_id)
            .await?;
        let owner_user_id = self
            .runtime_state
            .attachment_owner_user_id(attachment_id)
            .await?;
        if owner_user_id == user_id {
            return Ok(());
        }
        Err(DaemonError::SessionAccessDenied {
            session_id: session_id.to_string(),
            user_id: user_id.to_string(),
        })
    }

    pub(crate) async fn relay_watch_subscription_state(
        &self,
        session_id: &str,
        attachment_id: &str,
        should_check_snapshot: bool,
        previous_snapshot: Option<crate::runtime::projection::SessionSnapshotProjection>,
        last_workflow_design_sequence: u64,
    ) -> crate::runtime_transport::WatchResult {
        relay_peer_runtime::watch_relay_subscription_state(
            &self.runtime_state,
            session_id,
            attachment_id,
            should_check_snapshot,
            previous_snapshot,
            last_workflow_design_sequence,
        )
        .await
    }

    pub(crate) async fn relay_create_execution_lease(
        &self,
        home_kernel_id: &str,
        home_session_id: &str,
        home_agent_id: &str,
        home_agent_metaagent: bool,
        owner_user_id: &str,
    ) -> Result<crate::execution_lease::ExecutionLease, DaemonError> {
        relay_peer_runtime::create_relay_execution_lease(
            &self.runtime_state,
            home_kernel_id,
            home_session_id,
            home_agent_id,
            home_agent_metaagent,
            owner_user_id,
        )
        .await
    }

    pub(crate) async fn relay_destroy_execution_lease(
        &self,
        lease_id: &str,
    ) -> Result<(), DaemonError> {
        relay_peer_runtime::destroy_relay_execution_lease(&self.runtime_state, lease_id).await
    }

    pub(crate) async fn relay_create_leased_agent(
        &self,
        lease_id: &str,
        provider: &str,
        account_profile: &str,
        model: Option<String>,
        effort: Option<String>,
        execution_mode: Option<crate::provider::AgentExecutionMode>,
        permission_level: Option<crate::provider::AgentPermissionLevel>,
        workspace_live_sync_mode: Option<crate::config::WorkspaceLiveSyncMode>,
        worktree_id: Option<String>,
        worktree_placement: Option<crate::agent::GitWorktreePlacement>,
    ) -> Result<crate::execution_lease::LeasedAgent, DaemonError> {
        relay_peer_runtime::create_relay_leased_agent(
            &self.runtime_state,
            lease_id,
            provider,
            account_profile,
            model,
            effort,
            execution_mode,
            permission_level,
            workspace_live_sync_mode,
            worktree_id,
            worktree_placement,
        )
        .await
    }

    pub(crate) async fn relay_destroy_leased_agent(
        &self,
        leased_agent_id: &str,
    ) -> Result<(), DaemonError> {
        relay_peer_runtime::destroy_relay_leased_agent(&self.runtime_state, leased_agent_id).await
    }

    pub(crate) async fn relay_update_leased_agent_config(
        &self,
        leased_agent_id: &str,
        execution_mode: crate::provider::AgentExecutionMode,
        permission_level: crate::provider::AgentPermissionLevel,
    ) -> Result<crate::execution_lease::LeasedAgent, DaemonError> {
        relay_peer_runtime::update_relay_leased_agent_config(
            &self.runtime_state,
            leased_agent_id,
            execution_mode,
            permission_level,
        )
        .await
    }

    pub(crate) async fn relay_update_leased_agent_profile(
        &self,
        leased_agent_id: &str,
        provider: String,
        account_profile: String,
        model: Option<String>,
        effort: Option<String>,
    ) -> Result<crate::execution_lease::LeasedAgent, DaemonError> {
        relay_peer_runtime::update_relay_leased_agent_profile(
            &self.runtime_state,
            leased_agent_id,
            provider,
            account_profile,
            model,
            effort,
        )
        .await
    }

    pub(crate) async fn relay_update_leased_agent_meta_mode(
        &self,
        leased_agent_id: &str,
        active: bool,
    ) -> Result<crate::execution_lease::LeasedAgent, DaemonError> {
        relay_peer_runtime::update_relay_leased_agent_meta_mode(
            &self.runtime_state,
            leased_agent_id,
            active,
        )
        .await
    }

    pub(crate) async fn relay_update_leased_agent_remote_extension_manifest(
        &self,
        leased_agent_id: &str,
        remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    ) -> Result<(), DaemonError> {
        relay_peer_runtime::update_relay_leased_agent_remote_extension_manifest(
            &self.runtime_state,
            leased_agent_id,
            remote_extension_manifest,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn relay_launch_leased_native_provider_run(
        &self,
        leased_agent_id: &str,
        adapter_key: &str,
        provider: &str,
        account_profile: &str,
        model: &str,
        variant: Option<String>,
        structured_endpoint: Option<String>,
        provider_session_id: Option<String>,
        required_mcps: Vec<crate::transport::relay_peer::RequiredRemoteMcp>,
        required_skills: Option<Vec<crate::transport::relay_peer::RequiredRemoteSkill>>,
        remote_extension_manifest: crate::extension::RemoteExtensionManifest,
        provider_launch_credential: Option<
            crate::transport::relay_peer::RemoteProviderLaunchCredential,
        >,
    ) -> Result<crate::provider::RuntimeProviderRun, DaemonError> {
        relay_peer_runtime::launch_relay_leased_native_provider_run(
            &self.runtime_state,
            leased_agent_id,
            adapter_key,
            provider,
            account_profile,
            model,
            variant,
            structured_endpoint,
            provider_session_id,
            required_mcps,
            required_skills,
            remote_extension_manifest,
            provider_launch_credential,
        )
        .await
    }

    pub(crate) async fn relay_send_leased_native_provider_input(
        &self,
        leased_agent_id: &str,
        provider_run_id: &str,
        attachment_id: &str,
        data_base64: &str,
    ) -> Result<usize, DaemonError> {
        relay_peer_runtime::send_relay_leased_native_provider_input(
            &self.runtime_state,
            leased_agent_id,
            provider_run_id,
            attachment_id,
            data_base64,
        )
        .await
    }

    pub(crate) async fn relay_resize_leased_provider_terminal(
        &self,
        leased_agent_id: &str,
        provider_run_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), DaemonError> {
        relay_peer_runtime::resize_relay_leased_provider_terminal(
            &self.runtime_state,
            leased_agent_id,
            provider_run_id,
            cols,
            rows,
        )
        .await
    }

    pub(crate) async fn relay_submit_leased_prompt(
        &self,
        leased_agent_id: &str,
        prompt: &str,
        hidden_system_context: &str,
        attachments: Vec<crate::transport::relay_peer::RelayPromptAttachment>,
        workflow_context: Option<crate::execution_lease::RemoteWorkflowTurnContext>,
        git_context: Option<crate::transport::relay_peer::RemoteGitTurnContext>,
        required_mcps: Vec<crate::transport::relay_peer::RequiredRemoteMcp>,
        required_skills: Option<Vec<crate::transport::relay_peer::RequiredRemoteSkill>>,
        remote_extension_manifest: crate::extension::RemoteExtensionManifest,
        provider_launch_credential: Option<
            crate::transport::relay_peer::RemoteProviderLaunchCredential,
        >,
    ) -> Result<(String, crate::session::PromptSubmissionOutcome), DaemonError> {
        relay_peer_runtime::submit_relay_leased_prompt(
            &self.runtime_state,
            leased_agent_id,
            prompt,
            hidden_system_context,
            attachments,
            workflow_context,
            git_context,
            required_mcps,
            required_skills,
            remote_extension_manifest,
            provider_launch_credential,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn relay_steer_leased_prompt(
        &self,
        leased_agent_id: &str,
        steer_id: &str,
        target_home_prompt_id: &str,
        prompt: &str,
        hidden_system_context: &str,
        attachments: Vec<crate::transport::relay_peer::RelayPromptAttachment>,
        required_skills: Option<Vec<crate::transport::relay_peer::RequiredRemoteSkill>>,
    ) -> Result<(String, bool), DaemonError> {
        relay_peer_runtime::steer_relay_leased_prompt(
            &self.runtime_state,
            leased_agent_id,
            steer_id,
            target_home_prompt_id,
            prompt,
            hidden_system_context,
            attachments,
            required_skills,
        )
        .await
    }

    pub(crate) async fn relay_ensure_remote_skill_packages(
        &self,
        context: crate::transport::relay_peer::RemoteSkillSyncContext,
        packages: Vec<crate::skill::CharioxSkillPackage>,
    ) -> Result<Vec<crate::transport::relay_peer::RemoteSkillMaterialization>, DaemonError> {
        relay_peer_runtime::ensure_relay_remote_skill_packages(
            &self.runtime_state,
            context,
            packages,
        )
        .await
    }

    pub(crate) async fn relay_ensure_remote_provider_account(
        &self,
        context: crate::transport::relay_peer::RemoteProviderAccountSyncContext,
        materialization: crate::account_profile::ProviderAccountMaterialization,
    ) -> Result<crate::account_profile::ProviderAccountProfile, DaemonError> {
        relay_peer_runtime::ensure_relay_remote_provider_account(
            &self.runtime_state,
            context,
            materialization,
        )
        .await
    }

    pub(crate) async fn relay_check_remote_mcp_availability(
        &self,
        context: crate::transport::relay_peer::RemoteMcpCheckContext,
        required_mcps: Vec<crate::transport::relay_peer::RequiredRemoteMcp>,
    ) -> Result<Vec<crate::transport::relay_peer::RemoteMcpAvailability>, DaemonError> {
        relay_peer_runtime::check_relay_remote_mcp_availability(
            &self.runtime_state,
            context,
            required_mcps,
        )
        .await
    }

    pub(crate) async fn relay_apply_workspace_live_sync_change(
        &self,
        context: crate::transport::relay_peer::RemoteWorkspaceLiveSyncApplyContext,
        change: crate::git_observer::WorkspaceLiveSyncChange,
    ) -> Result<crate::git_observer::WorkspaceLiveSyncTargetResult, DaemonError> {
        Ok(self
            .runtime_state
            .apply_forwarded_workspace_live_sync_change(context, change))
    }

    pub(crate) async fn relay_complete_leased_prompt(
        &self,
        leased_agent_id: &str,
    ) -> Result<crate::session::PromptCompletion, DaemonError> {
        relay_peer_runtime::complete_relay_leased_prompt(&self.runtime_state, leased_agent_id).await
    }

    pub(crate) async fn relay_observe_leased_git_after(
        &self,
        leased_agent_id: &str,
        provider_run_id: &str,
    ) -> Result<
        (
            Vec<crate::transport::relay_peer::RemoteGitObservation>,
            Option<crate::git_observer::WorkspaceLiveSyncChange>,
        ),
        DaemonError,
    > {
        relay_peer_runtime::observe_relay_leased_git_after(
            &self.runtime_state,
            leased_agent_id,
            provider_run_id,
        )
        .await
    }

    pub(crate) async fn relay_cancel_leased_prompt(
        &self,
        leased_agent_id: &str,
    ) -> Result<crate::session::PromptCancellation, DaemonError> {
        relay_peer_runtime::cancel_relay_leased_prompt(&self.runtime_state, leased_agent_id).await
    }

    pub(crate) async fn relay_leased_agent_provider_run_id(
        &self,
        leased_agent_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        relay_peer_runtime::relay_leased_agent_provider_run_id(&self.runtime_state, leased_agent_id)
            .await
    }

    pub(crate) async fn relay_provider_run_terminal_diagnostic(
        &self,
        provider_run_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        Ok(relay_peer_runtime::relay_provider_run_terminal_diagnostic(
            &self.provider_run_projection,
            provider_run_id,
        ))
    }

    pub(crate) async fn relay_try_pump_leased_runtime_projections(
        &self,
    ) -> Result<Option<Vec<(String, crate::transport::relay_peer::RelayPeerEvent)>>, DaemonError>
    {
        relay_peer_runtime::try_pump_relay_leased_runtime_projections(&self.runtime_state).await
    }

    pub(crate) async fn relay_drain_leased_runtime_projection(
        &self,
        leased_agent_id: &str,
        provider_run_id: &str,
        pump_output: bool,
        replay_settled_completion: bool,
    ) -> Result<Option<(String, crate::transport::relay_peer::RelayPeerEvent)>, DaemonError> {
        relay_peer_runtime::drain_relay_leased_runtime_projection(
            &self.runtime_state,
            leased_agent_id,
            provider_run_id,
            pump_output,
            replay_settled_completion,
        )
        .await
    }

    pub(crate) async fn relay_project_remote_runtime_projection(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
        provider_run: Option<crate::provider::RuntimeProviderRun>,
        prompts: Vec<crate::transport::relay_peer::RelayProjectedPrompt>,
        output_chunks: Vec<crate::transport::relay_peer::RelayProjectedOutputChunk>,
        notices: Vec<String>,
        completions: Vec<crate::transport::relay_peer::RelayProjectedCompletion>,
    ) -> Result<(), DaemonError> {
        relay_peer_runtime::project_relay_remote_runtime_projection(
            &self.runtime_state,
            session_id,
            agent_id,
            provider_run_id,
            provider_run,
            prompts,
            output_chunks,
            notices,
            completions,
        )
        .await
    }

    pub(crate) async fn relay_forward_native_interaction(
        &self,
        context: crate::transport::relay_peer::RemoteNativeInteractionContext,
        interaction: crate::session::RuntimeInteraction,
    ) -> Result<crate::provider::ProviderNativeInteractionResolution, DaemonError> {
        forward_relay_native_interaction(&self.runtime_state, context, interaction).await
    }
}
