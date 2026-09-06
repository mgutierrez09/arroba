use crate::agent::GitWorktreePlacement;
use crate::error::DaemonError;
use crate::execution_lease::{ExecutionLease, LeasedAgent, RemoteWorkflowTurnContext};
use crate::provider::{AgentExecutionMode, AgentPermissionLevel};
use crate::runtime::projection::{
    ProviderRunProjectionStore, SessionSnapshotProjection, SessionStateProjectionStore,
};
use crate::runtime::state::KernelRuntimeState;
use crate::runtime_transport::WatchResult;
use crate::session::{PromptCancellation, PromptCompletion, PromptSubmissionOutcome};
use crate::skill::CharioxSkillPackage;
use crate::transport::relay_peer::{
    RelayPeerEvent, RelayProjectedCompletion, RelayProjectedOutputChunk, RelayProjectedPrompt,
    RelayPromptAttachment, RemoteGitObservation, RemoteGitTurnContext, RemoteMcpAvailability,
    RemoteMcpCheckContext, RemoteSkillMaterialization, RemoteSkillSyncContext, RequiredRemoteMcp,
};

pub(crate) async fn ensure_relay_subscription_attachment(
    runtime_state: &KernelRuntimeState,
    session_projection: &SessionStateProjectionStore,
    session_id: &str,
    attachment_id: &str,
) -> Result<(), DaemonError> {
    if let Some(session) = session_projection.get(session_id) {
        if session.has_attachment(attachment_id) {
            return Ok(());
        }
        return Err(DaemonError::AttachmentNotInSession {
            session_id: session_id.to_string(),
            attachment_id: attachment_id.to_string(),
        });
    }
    runtime_state
        .ensure_attachment_in_session(session_id, attachment_id)
        .await
        .map(|_| ())
}

pub(crate) async fn watch_relay_subscription_state(
    runtime_state: &KernelRuntimeState,
    session_id: &str,
    attachment_id: &str,
    should_check_snapshot: bool,
    previous_snapshot: Option<SessionSnapshotProjection>,
    last_workflow_design_sequence: u64,
) -> WatchResult {
    runtime_state
        .watch_relay_subscription_state(
            session_id,
            attachment_id,
            should_check_snapshot,
            previous_snapshot,
            last_workflow_design_sequence,
        )
        .await
}

pub(crate) async fn create_relay_execution_lease(
    runtime_state: &KernelRuntimeState,
    home_kernel_id: &str,
    home_session_id: &str,
    home_agent_id: &str,
    home_agent_metaagent: bool,
    owner_user_id: &str,
) -> Result<ExecutionLease, DaemonError> {
    runtime_state
        .create_relay_execution_lease(
            home_kernel_id,
            home_session_id,
            home_agent_id,
            home_agent_metaagent,
            owner_user_id,
        )
        .await
}

pub(crate) async fn destroy_relay_execution_lease(
    runtime_state: &KernelRuntimeState,
    lease_id: &str,
) -> Result<(), DaemonError> {
    runtime_state.destroy_relay_execution_lease(lease_id).await
}

pub(crate) async fn create_relay_leased_agent(
    runtime_state: &KernelRuntimeState,
    lease_id: &str,
    provider: &str,
    account_profile: &str,
    model: Option<String>,
    effort: Option<String>,
    execution_mode: Option<AgentExecutionMode>,
    permission_level: Option<AgentPermissionLevel>,
    workspace_live_sync_mode: Option<crate::config::WorkspaceLiveSyncMode>,
    worktree_id: Option<String>,
    worktree_placement: Option<GitWorktreePlacement>,
) -> Result<LeasedAgent, DaemonError> {
    runtime_state
        .create_relay_leased_agent(
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

pub(crate) async fn destroy_relay_leased_agent(
    runtime_state: &KernelRuntimeState,
    leased_agent_id: &str,
) -> Result<(), DaemonError> {
    runtime_state
        .destroy_relay_leased_agent(leased_agent_id)
        .await
}

pub(crate) async fn update_relay_leased_agent_config(
    runtime_state: &KernelRuntimeState,
    leased_agent_id: &str,
    execution_mode: AgentExecutionMode,
    permission_level: AgentPermissionLevel,
) -> Result<LeasedAgent, DaemonError> {
    runtime_state
        .update_relay_leased_agent_config(leased_agent_id, execution_mode, permission_level)
        .await
}

pub(crate) async fn update_relay_leased_agent_profile(
    runtime_state: &KernelRuntimeState,
    leased_agent_id: &str,
    provider: String,
    account_profile: String,
    model: Option<String>,
    effort: Option<String>,
) -> Result<LeasedAgent, DaemonError> {
    runtime_state
        .update_relay_leased_agent_profile(
            leased_agent_id,
            provider,
            account_profile,
            model,
            effort,
        )
        .await
}

pub(crate) async fn update_relay_leased_agent_meta_mode(
    runtime_state: &KernelRuntimeState,
    leased_agent_id: &str,
    active: bool,
) -> Result<LeasedAgent, DaemonError> {
    runtime_state
        .update_relay_leased_agent_meta_mode(leased_agent_id, active)
        .await
}

pub(crate) async fn update_relay_leased_agent_remote_extension_manifest(
    runtime_state: &KernelRuntimeState,
    leased_agent_id: &str,
    remote_extension_manifest: crate::extension::RemoteExtensionManifest,
) -> Result<(), DaemonError> {
    runtime_state
        .update_relay_leased_agent_remote_extension_manifest(
            leased_agent_id,
            remote_extension_manifest,
        )
        .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn launch_relay_leased_native_provider_run(
    runtime_state: &KernelRuntimeState,
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
    runtime_state
        .launch_relay_leased_native_provider_run(
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

pub(crate) async fn send_relay_leased_native_provider_input(
    runtime_state: &KernelRuntimeState,
    leased_agent_id: &str,
    provider_run_id: &str,
    attachment_id: &str,
    data_base64: &str,
) -> Result<usize, DaemonError> {
    runtime_state
        .send_relay_leased_native_provider_input(
            leased_agent_id,
            provider_run_id,
            attachment_id,
            data_base64,
        )
        .await
}

pub(crate) async fn resize_relay_leased_provider_terminal(
    runtime_state: &KernelRuntimeState,
    leased_agent_id: &str,
    provider_run_id: &str,
    cols: u16,
    rows: u16,
) -> Result<(), DaemonError> {
    runtime_state
        .resize_relay_leased_provider_terminal(leased_agent_id, provider_run_id, cols, rows)
        .await
}

pub(crate) async fn submit_relay_leased_prompt(
    runtime_state: &KernelRuntimeState,
    leased_agent_id: &str,
    prompt: &str,
    hidden_system_context: &str,
    attachments: Vec<RelayPromptAttachment>,
    workflow_context: Option<RemoteWorkflowTurnContext>,
    git_context: Option<RemoteGitTurnContext>,
    required_mcps: Vec<RequiredRemoteMcp>,
    required_skills: Option<Vec<crate::transport::relay_peer::RequiredRemoteSkill>>,
    remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    provider_launch_credential: Option<
        crate::transport::relay_peer::RemoteProviderLaunchCredential,
    >,
) -> Result<(String, PromptSubmissionOutcome), DaemonError> {
    runtime_state
        .submit_relay_leased_prompt(
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
pub(crate) async fn steer_relay_leased_prompt(
    runtime_state: &KernelRuntimeState,
    leased_agent_id: &str,
    steer_id: &str,
    target_home_prompt_id: &str,
    prompt: &str,
    hidden_system_context: &str,
    attachments: Vec<RelayPromptAttachment>,
    required_skills: Option<Vec<crate::transport::relay_peer::RequiredRemoteSkill>>,
) -> Result<(String, bool), DaemonError> {
    runtime_state
        .steer_relay_leased_prompt(
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

pub(crate) async fn ensure_relay_remote_skill_packages(
    runtime_state: &KernelRuntimeState,
    context: RemoteSkillSyncContext,
    packages: Vec<CharioxSkillPackage>,
) -> Result<Vec<RemoteSkillMaterialization>, DaemonError> {
    runtime_state
        .ensure_relay_remote_skill_packages(context, packages)
        .await
}

pub(crate) async fn ensure_relay_remote_provider_account(
    runtime_state: &KernelRuntimeState,
    context: crate::transport::relay_peer::RemoteProviderAccountSyncContext,
    materialization: crate::account_profile::ProviderAccountMaterialization,
) -> Result<crate::account_profile::ProviderAccountProfile, DaemonError> {
    runtime_state
        .ensure_relay_remote_provider_account(context, materialization)
        .await
}

pub(crate) async fn check_relay_remote_mcp_availability(
    runtime_state: &KernelRuntimeState,
    context: RemoteMcpCheckContext,
    required_mcps: Vec<RequiredRemoteMcp>,
) -> Result<Vec<RemoteMcpAvailability>, DaemonError> {
    runtime_state
        .check_relay_remote_mcp_availability(context, required_mcps)
        .await
}

pub(crate) async fn complete_relay_leased_prompt(
    runtime_state: &KernelRuntimeState,
    leased_agent_id: &str,
) -> Result<PromptCompletion, DaemonError> {
    runtime_state
        .complete_relay_leased_prompt(leased_agent_id)
        .await
}

pub(crate) async fn observe_relay_leased_git_after(
    runtime_state: &KernelRuntimeState,
    leased_agent_id: &str,
    provider_run_id: &str,
) -> Result<
    (
        Vec<RemoteGitObservation>,
        Option<crate::git_observer::WorkspaceLiveSyncChange>,
    ),
    DaemonError,
> {
    runtime_state
        .observe_relay_leased_git_after(leased_agent_id, provider_run_id)
        .await
}

pub(crate) async fn cancel_relay_leased_prompt(
    runtime_state: &KernelRuntimeState,
    leased_agent_id: &str,
) -> Result<PromptCancellation, DaemonError> {
    runtime_state
        .cancel_relay_leased_prompt(leased_agent_id)
        .await
}

pub(crate) async fn relay_leased_agent_provider_run_id(
    runtime_state: &KernelRuntimeState,
    leased_agent_id: &str,
) -> Result<Option<String>, DaemonError> {
    runtime_state
        .relay_leased_agent_provider_run_id(leased_agent_id)
        .await
}

pub(crate) fn relay_provider_run_terminal_diagnostic(
    provider_run_projection: &ProviderRunProjectionStore,
    provider_run_id: &str,
) -> Option<String> {
    provider_run_projection
        .get(provider_run_id)
        .and_then(|run| run.terminal_diagnostic().map(str::to_string))
        .filter(|message| !message.trim().is_empty())
}

pub(crate) async fn try_pump_relay_leased_runtime_projections(
    runtime_state: &KernelRuntimeState,
) -> Result<Option<Vec<(String, RelayPeerEvent)>>, DaemonError> {
    runtime_state
        .try_pump_relay_leased_runtime_projections()
        .await
}

pub(crate) async fn drain_relay_leased_runtime_projection(
    runtime_state: &KernelRuntimeState,
    leased_agent_id: &str,
    provider_run_id: &str,
    pump_output: bool,
    replay_settled_completion: bool,
) -> Result<Option<(String, RelayPeerEvent)>, DaemonError> {
    runtime_state
        .drain_relay_leased_runtime_projection(
            leased_agent_id,
            provider_run_id,
            pump_output,
            replay_settled_completion,
        )
        .await
}

pub(crate) async fn project_relay_remote_runtime_projection(
    runtime_state: &KernelRuntimeState,
    session_id: &str,
    agent_id: &str,
    provider_run_id: &str,
    provider_run: Option<crate::provider::RuntimeProviderRun>,
    prompts: Vec<RelayProjectedPrompt>,
    output_chunks: Vec<RelayProjectedOutputChunk>,
    notices: Vec<String>,
    completions: Vec<RelayProjectedCompletion>,
) -> Result<(), DaemonError> {
    runtime_state
        .project_relay_remote_runtime_projection(
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
