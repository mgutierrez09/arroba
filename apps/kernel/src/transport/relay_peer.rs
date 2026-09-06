use serde::{Deserialize, Serialize};

use crate::agent::GitWorktreePlacement;
use crate::execution_lease::{ExecutionLease, LeasedAgent, RemoteWorkflowTurnContext};
use crate::history::{HistoryAttributionConfidence, HistoryEventKind, HistoryEventTurnContext};
use crate::io::WorkspaceIdentity;
use crate::mcp::CharioxMcpServerConfig;
use crate::session::{PromptCancellation, PromptCompletion, PromptOrigin, PromptSubmissionOutcome};
use crate::skill::CharioxSkillPackage;
use crate::terminal::TerminalOutputKind;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RelayManagedContextCapability(String);

impl RelayManagedContextCapability {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl std::fmt::Debug for RelayManagedContextCapability {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[redacted managed-context capability]")
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RelayManagedContextChunk(String);

impl RelayManagedContextChunk {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl std::fmt::Debug for RelayManagedContextChunk {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[redacted managed-context chunk]")
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RelayManagedSliceToken(String);

impl RelayManagedSliceToken {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl std::fmt::Debug for RelayManagedSliceToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[redacted managed-slice relay token]")
    }
}

/// Version 25 requires the home-selected execution profile on every leased prompt
/// and retires the separate workflow-provider failure RPC.
pub const RELAY_PEER_PROTOCOL_VERSION: u32 = 25;

/// Home-selected execution identity for a leased prompt. Contains no credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayAgentExecutionProfile {
    pub provider: String,
    pub account_profile: String,
    pub model: Option<String>,
    pub effort: Option<String>,
}

impl From<&crate::agent::AgentInstance> for RelayAgentExecutionProfile {
    fn from(agent: &crate::agent::AgentInstance) -> Self {
        Self {
            provider: agent.provider().to_string(),
            account_profile: agent.provider_account_profile().to_string(),
            model: agent.model().map(str::to_string),
            effort: agent.effort().map(str::to_string),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayManagedContextTransferPhase {
    Armed,
    Receiving,
    ReadyToImport,
    Importing,
    Consumed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayManagedContextImportedRepository {
    pub repository_id: String,
    pub role: crate::managed_context::development::DevelopmentRepositoryRole,
    pub target_directory: String,
    pub destination_path: String,
    pub head_sha: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayManagedContextImportReceipt {
    pub transfer_id: String,
    pub archive_sha256: String,
    pub plan_digest: String,
    pub development: RelayManagedDevelopmentContextImportReceipt,
    pub kernel_context: RelayManagedKernelContextImportReceipt,
    pub receipt_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RelayManagedDevelopmentContextImportReceipt {
    Empty,
    FromSource {
        project_id: String,
        destination_root: String,
        primary_repository_id: String,
        repositories: Vec<RelayManagedContextImportedRepository>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RelayManagedKernelContextImportReceipt {
    Empty,
    FromKernel {
        context_id: String,
        source_kernel_id: String,
        source_key_thumbprint: String,
        snapshot_sha256: String,
        extension_count: usize,
        dependency_count: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayManagedContextTransferStatus {
    pub transfer_id: String,
    pub phase: RelayManagedContextTransferPhase,
    pub accepted_bytes: u64,
    pub archive_size_bytes: u64,
    pub expires_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<RelayManagedContextImportReceipt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayPromptAttachment {
    pub url: String,
    pub mime: String,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contents_base64: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteWorkspaceLiveSyncContext {
    pub home_kernel_id: String,
    pub home_session_id: String,
    pub home_agent_id: String,
    pub leased_agent_id: String,
    pub worker_kernel_id: String,
    pub worker_machine_id: String,
    pub worker_provider_run_id: String,
    pub worker_worktree_path: String,
    pub worker_workspace_identity: WorkspaceIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteSkillSyncContext {
    pub home_kernel_id: String,
    pub home_session_id: String,
    pub home_agent_id: String,
    pub leased_agent_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteProviderAccountSyncContext {
    pub home_kernel_id: String,
    pub home_session_id: String,
    pub home_agent_id: String,
    pub execution_lease_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteSkillMaterialization {
    pub name: String,
    pub version_hash: String,
    pub materialized_root: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteMcpCheckContext {
    pub home_kernel_id: String,
    pub home_session_id: String,
    pub home_agent_id: String,
    pub leased_agent_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteExtensionInvocationContext {
    pub home_kernel_id: String,
    pub home_session_id: String,
    pub home_agent_id: String,
    pub leased_agent_id: String,
    pub worker_provider_run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_kernel_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_machine_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RemoteCredentialSecretInjection {
    Browser { target_url: String },
    Pty,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteNativeInteractionContext {
    pub home_session_id: String,
    pub home_agent_id: String,
    pub leased_agent_id: String,
    pub worker_provider_run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequiredRemoteMcp {
    pub config: CharioxMcpServerConfig,
    pub definition_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequiredRemoteSkill {
    pub name: String,
    pub version_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteMcpAvailability {
    pub name: String,
    pub expected_hash: String,
    pub status: RemoteMcpAvailabilityStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteGitTurnContext {
    pub home_session_id: String,
    pub home_agent_id: String,
    pub home_prompt_id: String,
    pub home_turn_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_attachment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_live_sync_mode: Option<crate::config::WorkspaceLiveSyncMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_origin: Option<PromptOrigin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_provider_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_provider_turn_id: Option<String>,
    pub prompt_summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteGitObservation {
    pub kind: HistoryEventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default)]
    pub metadata: std::collections::BTreeMap<String, serde_json::Value>,
    pub context: HistoryEventTurnContext,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_agent_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_prompt_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_turn_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution_confidence: Option<HistoryAttributionConfidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteWorkspaceLiveSyncApplyContext {
    pub home_session_id: String,
    pub link_id: String,
    pub link_name: String,
    pub source_agent_id: String,
    pub source_worktree_path: String,
    pub target_user_id: String,
    pub target_machine_id: String,
    pub target_kernel_id: String,
    pub target_repo_root: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RemoteMcpAvailabilityStatus {
    Available,
    Missing,
    DefinitionMismatch { worker_hash: String },
    MissingCommand { command: String },
    MissingEnv { names: Vec<String> },
    Invalid { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteWorkspaceLiveSyncArtifactState {
    pub path: String,
    pub exists: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_base64: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteWorkspaceLiveSyncInvocationMetadata {
    pub invocation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_tool_call_id: Option<String>,
    #[serde(default = "default_workspace_live_sync_invocation_attempt")]
    pub attempt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

impl RemoteWorkspaceLiveSyncInvocationMetadata {
    pub fn new(
        provider_run_id: &str,
        tool_name: &str,
        provider_tool_call_id: Option<String>,
    ) -> Self {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let started_at_ms = crate::session::unix_epoch_ms();
        let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self {
            invocation_id: format!("{provider_run_id}:{tool_name}:{started_at_ms}:{sequence}"),
            provider_tool_call_id,
            attempt: 1,
            idempotency_key: None,
        }
    }
}

fn default_workspace_live_sync_invocation_attempt() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RelayPeerRequest {
    Ping {
        value: String,
    },
    InstallManagedSliceRelayToken {
        slice_id: String,
        owner_kernel_id: String,
        owner_machine_id: String,
        activation_nonce: String,
        relay_token: RelayManagedSliceToken,
        expires_at_ms: u64,
        relay_recovery_token: RelayManagedSliceToken,
        recovery_expires_at_ms: u64,
    },
    ConfirmManagedSliceRelayToken {
        slice_id: String,
        owner_kernel_id: String,
        worker_kernel_id: String,
        activation_nonce: String,
    },
    RefreshManagedSliceRelayToken {
        slice_id: String,
        owner_kernel_id: String,
        worker_kernel_id: String,
    },
    CreateExecutionLease {
        home_kernel_id: String,
        home_session_id: String,
        home_agent_id: String,
        #[serde(default)]
        home_agent_metaagent: bool,
        owner_user_id: String,
    },
    DestroyExecutionLease {
        lease_id: String,
    },
    SpawnLeasedAgent {
        lease_id: String,
        provider: String,
        account_profile: String,
        model: Option<String>,
        effort: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        execution_mode: Option<crate::provider::AgentExecutionMode>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        permission_level: Option<crate::provider::AgentPermissionLevel>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace_live_sync_mode: Option<crate::config::WorkspaceLiveSyncMode>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        worktree_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        worktree_placement: Option<GitWorktreePlacement>,
    },
    DestroyLeasedAgent {
        leased_agent_id: String,
    },
    UpdateLeasedAgentConfig {
        leased_agent_id: String,
        execution_mode: crate::provider::AgentExecutionMode,
        permission_level: crate::provider::AgentPermissionLevel,
    },
    UpdateLeasedAgentProfile {
        leased_agent_id: String,
        provider: String,
        account_profile: String,
        model: Option<String>,
        effort: Option<String>,
    },
    UpdateLeasedAgentMetaMode {
        leased_agent_id: String,
        active: bool,
    },
    UpdateLeasedAgentRemoteExtensionManifest {
        leased_agent_id: String,
        remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    },
    LaunchLeasedNativeProviderRun {
        leased_agent_id: String,
        adapter_key: String,
        provider: String,
        account_profile: String,
        model: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        variant: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        structured_endpoint: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_session_id: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        required_mcps: Vec<RequiredRemoteMcp>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        required_skills: Option<Vec<RequiredRemoteSkill>>,
        #[serde(
            default,
            skip_serializing_if = "crate::extension::RemoteExtensionManifest::is_empty"
        )]
        remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    },
    SendLeasedNativeProviderInput {
        leased_agent_id: String,
        provider_run_id: String,
        attachment_id: String,
        data_base64: String,
    },
    ResizeLeasedProviderTerminal {
        leased_agent_id: String,
        provider_run_id: String,
        cols: u16,
        rows: u16,
    },
    SubmitLeasedPrompt {
        leased_agent_id: String,
        expected_profile: RelayAgentExecutionProfile,
        prompt: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        hidden_system_context: String,
        #[serde(default)]
        attachments: Vec<RelayPromptAttachment>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workflow_context: Option<RemoteWorkflowTurnContext>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        git_context: Option<RemoteGitTurnContext>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        required_mcps: Vec<RequiredRemoteMcp>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        required_skills: Option<Vec<RequiredRemoteSkill>>,
        #[serde(
            default,
            skip_serializing_if = "crate::extension::RemoteExtensionManifest::is_empty"
        )]
        remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    },
    SteerLeasedPrompt {
        leased_agent_id: String,
        steer_id: String,
        target_home_prompt_id: String,
        prompt: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        hidden_system_context: String,
        #[serde(default)]
        attachments: Vec<RelayPromptAttachment>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        required_skills: Option<Vec<RequiredRemoteSkill>>,
    },
    DrainLeasedRuntimeProjection {
        leased_agent_id: String,
        provider_run_id: String,
        #[serde(default)]
        pump_output: bool,
    },
    CompleteLeasedPrompt {
        leased_agent_id: String,
    },
    ObserveLeasedGitAfter {
        leased_agent_id: String,
        provider_run_id: String,
    },
    CancelLeasedPrompt {
        leased_agent_id: String,
    },
    ForwardWorkflowRuntimeTool {
        context: RemoteWorkflowTurnContext,
        tool_name: String,
        arguments: serde_json::Value,
    },
    ForwardWorkspaceLiveSyncRuntimeTool {
        context: RemoteWorkspaceLiveSyncContext,
        metadata: RemoteWorkspaceLiveSyncInvocationMetadata,
        tool_name: String,
        arguments: serde_json::Value,
        artifact_states: Vec<RemoteWorkspaceLiveSyncArtifactState>,
    },
    FinalizeWorkspaceLiveSyncRuntimeTool {
        context: RemoteWorkspaceLiveSyncContext,
        metadata: RemoteWorkspaceLiveSyncInvocationMetadata,
        tool_name: String,
        arguments: serde_json::Value,
        initial_artifact_states: Vec<RemoteWorkspaceLiveSyncArtifactState>,
        final_artifact_states: Vec<RemoteWorkspaceLiveSyncArtifactState>,
    },
    ForwardCapabilityRuntimeTool {
        context: RemoteWorkspaceLiveSyncContext,
        tool_name: String,
        arguments: serde_json::Value,
    },
    ForwardMetaRuntimeTool {
        context: RemoteWorkspaceLiveSyncContext,
        tool_name: String,
        arguments: serde_json::Value,
    },
    InvokeHomeExtensionTool {
        context: RemoteExtensionInvocationContext,
        #[serde(default)]
        metadata: crate::extension::RemoteExtensionInvocationMetadata,
        tool: crate::extension::RemoteExtensionTool,
        arguments: serde_json::Value,
    },
    InvokeHomeMcpProxy {
        context: RemoteExtensionInvocationContext,
        #[serde(default)]
        metadata: crate::extension::RemoteExtensionInvocationMetadata,
        name: String,
        tool: crate::extension::RemoteExtensionTool,
        payload: serde_json::Value,
    },
    CancelHomeExtensionInvocation {
        context: RemoteExtensionInvocationContext,
        #[serde(default)]
        metadata: crate::extension::RemoteExtensionInvocationMetadata,
    },
    InvokeHomeCredentialTool {
        context: RemoteExtensionInvocationContext,
        tool_name: String,
        arguments: serde_json::Value,
    },
    ResolveHomeCredentialSecret {
        context: RemoteExtensionInvocationContext,
        credential_id: String,
        injection: RemoteCredentialSecretInjection,
    },
    ApplyWorkspaceLiveSyncChange {
        context: RemoteWorkspaceLiveSyncApplyContext,
        change: crate::git_observer::WorkspaceLiveSyncChange,
    },
    ForwardNativeInteraction {
        context: RemoteNativeInteractionContext,
        interaction: crate::session::RuntimeInteraction,
    },
    EnsureRemoteSkillPackages {
        context: RemoteSkillSyncContext,
        packages: Vec<CharioxSkillPackage>,
    },
    EnsureRemoteProviderAccount {
        context: RemoteProviderAccountSyncContext,
        materialization: crate::account_profile::ProviderAccountMaterialization,
    },
    CheckRemoteMcpAvailability {
        context: RemoteMcpCheckContext,
        required_mcps: Vec<RequiredRemoteMcp>,
    },
    ArmManagedContextImport {
        context_id: String,
        plan_digest: String,
        target_environment_id: String,
        target_kernel_id: String,
        target_key_thumbprint: String,
        capability: RelayManagedContextCapability,
        archive_sha256: String,
        archive_size_bytes: u64,
    },
    BeginManagedContextImport {
        transfer_id: String,
        capability: RelayManagedContextCapability,
    },
    UploadManagedContextChunk {
        transfer_id: String,
        capability: RelayManagedContextCapability,
        offset: u64,
        data_base64: RelayManagedContextChunk,
        chunk_sha256: String,
    },
    FinalizeManagedContextImport {
        transfer_id: String,
        capability: RelayManagedContextCapability,
    },
    GetManagedContextImportStatus {
        transfer_id: String,
        capability: RelayManagedContextCapability,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RelayPeerResponse {
    Pong {
        value: String,
        daemon_id: String,
    },
    ManagedSliceRelayTokenInstalled {
        slice_id: String,
        activation_nonce: String,
        relay_peer_protocol_version: u32,
    },
    ManagedSliceRelayTokenActivated {
        slice_id: String,
        activation_nonce: String,
        relay_peer_protocol_version: u32,
    },
    ManagedSliceRelayTokenRefreshed {
        slice_id: String,
        relay_token: RelayManagedSliceToken,
        expires_at_ms: u64,
        relay_recovery_token: RelayManagedSliceToken,
        recovery_expires_at_ms: u64,
        relay_peer_protocol_version: u32,
    },
    ManagedSliceRelayTokenFailed {
        code: String,
        retryable: bool,
    },
    ExecutionLeaseCreated {
        lease: ExecutionLease,
        #[serde(default)]
        relay_peer_protocol_version: u32,
    },
    ExecutionLeaseDestroyed {
        lease_id: String,
    },
    LeasedAgentSpawned {
        leased_agent: LeasedAgent,
    },
    LeasedAgentDestroyed {
        leased_agent_id: String,
    },
    LeasedAgentConfigUpdated {
        leased_agent: LeasedAgent,
    },
    LeasedAgentProfileUpdated {
        leased_agent: LeasedAgent,
    },
    LeasedAgentMetaModeUpdated {
        leased_agent: LeasedAgent,
    },
    LeasedAgentRemoteExtensionManifestUpdated {
        leased_agent_id: String,
    },
    LeasedNativeProviderRunLaunched {
        provider_run: crate::provider::RuntimeProviderRun,
    },
    LeasedNativeProviderInputSent {
        byte_count: usize,
    },
    LeasedProviderTerminalResized {
        provider_run_id: String,
        cols: u16,
        rows: u16,
    },
    LeasedPromptSubmitted {
        provider_run_id: String,
        outcome: PromptSubmissionOutcome,
    },
    LeasedPromptSteered {
        provider_run_id: String,
        steer_id: String,
        replayed: bool,
    },
    LeasedRuntimeProjectionDrained {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        event: Option<RelayPeerEvent>,
    },
    LeasedPromptCompleted {
        provider_run_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_diagnostic: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        git_observations: Vec<RemoteGitObservation>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace_live_sync_change: Option<crate::git_observer::WorkspaceLiveSyncChange>,
        completion: PromptCompletion,
    },
    LeasedGitObserved {
        provider_run_id: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        git_observations: Vec<RemoteGitObservation>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace_live_sync_change: Option<crate::git_observer::WorkspaceLiveSyncChange>,
    },
    LeasedPromptCancelled {
        cancellation: PromptCancellation,
    },
    WorkflowRuntimeToolHandled {
        result: crate::transport::runtime_tools::RuntimeToolResult,
    },
    WorkspaceLiveSyncRuntimeToolHandled {
        result: crate::transport::runtime_tools::RuntimeToolResult,
        final_artifact_states: Vec<RemoteWorkspaceLiveSyncArtifactState>,
    },
    WorkspaceLiveSyncRuntimeToolFinalized,
    CapabilityRuntimeToolHandled {
        result: crate::transport::runtime_tools::RuntimeToolResult,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        skill_package: Option<CharioxSkillPackage>,
        #[serde(
            default,
            skip_serializing_if = "crate::extension::RemoteExtensionManifest::is_empty"
        )]
        remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    },
    MetaRuntimeToolHandled {
        result: crate::transport::runtime_tools::RuntimeToolResult,
    },
    HomeExtensionToolHandled {
        result: crate::transport::runtime_tools::RuntimeToolResult,
    },
    HomeMcpProxyHandled {
        response: serde_json::Value,
    },
    HomeExtensionInvocationCancelled {
        invocation_id: String,
        cancelled: bool,
    },
    HomeCredentialToolHandled {
        result: crate::transport::runtime_tools::RuntimeToolResult,
    },
    HomeCredentialSecretResolved {
        credential_id: String,
        secret_input: String,
    },
    WorkspaceLiveSyncChangeApplied {
        target_result: crate::git_observer::WorkspaceLiveSyncTargetResult,
    },
    NativeInteractionResolved {
        resolution: crate::provider::ProviderNativeInteractionResolution,
    },
    RemoteSkillPackagesEnsured {
        materialized: Vec<RemoteSkillMaterialization>,
    },
    RemoteProviderAccountEnsured {
        provider: String,
        account_profile: String,
    },
    RemoteMcpAvailabilityChecked {
        results: Vec<RemoteMcpAvailability>,
    },
    ManagedContextImportArmed {
        transfer_id: String,
        capability: RelayManagedContextCapability,
        expires_at_ms: u64,
        max_chunk_bytes: usize,
        relay_peer_protocol_version: u32,
    },
    ManagedContextImportStatus {
        status: RelayManagedContextTransferStatus,
    },
    ManagedContextImportFailed {
        code: String,
        retryable: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayProjectedOutputChunk {
    pub kind: TerminalOutputKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_key: Option<String>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayProjectedCompletion {
    pub message_id: String,
    pub completed_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_prompt_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayProjectedPrompt {
    pub prompt_id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RelayPeerEvent {
    LeasedRuntimeProjection {
        home_session_id: String,
        home_agent_id: String,
        provider_run_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_run: Option<crate::provider::RuntimeProviderRun>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        prompts: Vec<RelayProjectedPrompt>,
        output_chunks: Vec<RelayProjectedOutputChunk>,
        notices: Vec<String>,
        completions: Vec<RelayProjectedCompletion>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retired_workflow_failure_forwarding_is_not_an_accepted_peer_protocol_path() {
        let request = serde_json::json!({
            "kind": "forward_workflow_provider_failure",
            "context": {
                "home_kernel_id": "home",
                "home_session_id": "room",
                "home_agent_id": "agent",
                "workflow_run_id": "run",
                "workflow_node_run_id": "node",
                "delivery_token": "test-turn",
                "event_reply_enabled": false,
                "event_context_enabled": false,
                "event_actions_enabled": false
            },
            "message": "provider capacity exhausted"
        });
        assert!(
            serde_json::from_value::<RelayPeerRequest>(request).is_err(),
            "provider failures must use correlated runtime projection, not a second RPC"
        );
        assert!(
            serde_json::from_value::<RelayPeerResponse>(serde_json::json!({
                "kind": "workflow_provider_failure_handled"
            }))
            .is_err()
        );
    }

    #[test]
    fn managed_slice_relay_token_keeps_wire_shape_and_redacts_debug_output() {
        let request = RelayPeerRequest::InstallManagedSliceRelayToken {
            slice_id: "slice-1".to_string(),
            owner_kernel_id: "kernel-owner".to_string(),
            owner_machine_id: "machine-owner".to_string(),
            activation_nonce: "activation-1".to_string(),
            relay_token: RelayManagedSliceToken::new("secret-relay-token".to_string()),
            expires_at_ms: 300_000,
            relay_recovery_token: RelayManagedSliceToken::new("secret-recovery-token".to_string()),
            recovery_expires_at_ms: 2_592_000_000,
        };
        let encoded = serde_json::to_value(&request).expect("request should serialize");
        assert_eq!(encoded["kind"], "install_managed_slice_relay_token");
        assert_eq!(encoded["relay_token"], "secret-relay-token");
        assert_eq!(encoded["relay_recovery_token"], "secret-recovery-token");

        let debug = format!("{request:?}");
        assert!(debug.contains("[redacted managed-slice relay token]"));
        assert!(!debug.contains("secret-relay-token"));
        assert!(!debug.contains("secret-recovery-token"));

        let decoded: RelayPeerRequest =
            serde_json::from_value(encoded).expect("request should deserialize");
        assert_eq!(decoded, request);
    }

    #[test]
    fn managed_slice_relay_refresh_response_redacts_token() {
        let response = RelayPeerResponse::ManagedSliceRelayTokenRefreshed {
            slice_id: "slice-1".to_string(),
            relay_token: RelayManagedSliceToken::new("refreshed-secret".to_string()),
            expires_at_ms: 600_000,
            relay_recovery_token: RelayManagedSliceToken::new(
                "refreshed-recovery-secret".to_string(),
            ),
            recovery_expires_at_ms: 2_592_600_000,
            relay_peer_protocol_version: RELAY_PEER_PROTOCOL_VERSION,
        };
        let debug = format!("{response:?}");
        assert!(debug.contains("[redacted managed-slice relay token]"));
        assert!(!debug.contains("refreshed-secret"));
        assert!(!debug.contains("refreshed-recovery-secret"));
    }

    #[test]
    fn managed_slice_activation_confirmation_has_versioned_nonce_wire_shape() {
        let request = RelayPeerRequest::ConfirmManagedSliceRelayToken {
            slice_id: "slice-1".to_string(),
            owner_kernel_id: "kernel-owner".to_string(),
            worker_kernel_id: "kernel-worker".to_string(),
            activation_nonce: "activation-1".to_string(),
        };
        let request_value = serde_json::to_value(&request).expect("request should serialize");
        assert_eq!(request_value["kind"], "confirm_managed_slice_relay_token");
        assert_eq!(request_value["activation_nonce"], "activation-1");
        assert_eq!(
            serde_json::from_value::<RelayPeerRequest>(request_value)
                .expect("request should deserialize"),
            request
        );

        let response = RelayPeerResponse::ManagedSliceRelayTokenActivated {
            slice_id: "slice-1".to_string(),
            activation_nonce: "activation-1".to_string(),
            relay_peer_protocol_version: RELAY_PEER_PROTOCOL_VERSION,
        };
        let response_value = serde_json::to_value(&response).expect("response should serialize");
        assert_eq!(
            response_value["kind"],
            "managed_slice_relay_token_activated"
        );
        assert_eq!(
            response_value["relay_peer_protocol_version"],
            RELAY_PEER_PROTOCOL_VERSION
        );
        assert_eq!(
            serde_json::from_value::<RelayPeerResponse>(response_value)
                .expect("response should deserialize"),
            response
        );
    }
}
