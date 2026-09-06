use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

pub(crate) mod attachment_artifacts;
mod config_runtime;
mod daemon_lifecycle;
mod durable_runtime_state;
mod external_provider_session_discovery;
pub(crate) use external_provider_session_discovery::find_external_provider_prompt_recovery_match;
mod external_provider_sessions;
mod history_access;
mod history_event_context;
mod kernel_agent;
mod kernel_api_facade;
mod kernel_session;
mod legacy_workflow_history;
mod prompt_activity;
mod prompt_lifecycle;
mod prompt_state_owner;
mod provider_activation;
mod provider_first_output_watchdog;
mod provider_focus;
mod provider_launch_failure_retry;
mod provider_launch_policy;
mod provider_launch_request;
mod provider_liveness;
pub(crate) mod provider_output;
mod provider_output_claude_native;
mod provider_output_fanout;
mod provider_output_prompt_settlement;
mod provider_output_trace;
mod provider_processes;
mod provider_prompt_launch;
mod provider_run_read;
mod provider_runtime;
mod provider_tracking;
mod relay_runtime;
mod remote_agent_binding;
mod remote_kernel_selection;
mod remote_lease;
mod remote_workspace_live_sync_fanout;
mod session_runtime;
mod terminal_fanout;
pub(crate) mod terminal_input;
mod workflow_design_events;
pub(crate) mod workflow_runtime;
mod workflow_workspace_claims;

pub(crate) use attachment_artifacts::{attachment_artifact_root, attachment_artifact_roots};
pub(crate) use external_provider_session_discovery::{
    discover_external_provider_sessions, discover_external_provider_sessions_for_profiles,
    external_provider_session_candidate_paths_for_profiles,
    external_provider_session_discovery_candidate_paths,
    external_provider_session_discovery_signature_for_candidates,
    external_provider_session_transcript_needs_refresh,
    external_provider_session_transcript_needs_refresh_for_profile,
    read_external_provider_observed_turns, read_external_provider_observed_turns_for_profile,
    ExternalProviderSessionDiscoverySignature, ExternalProviderSessionProfileRoot,
};
pub(crate) use external_provider_sessions::{
    AttachedProviderTranscriptCursorKey, AttachedProviderTranscriptCursorStore,
    ExternalProviderSessionAttachmentRef, ExternalProviderSessionIndexStore,
};
pub(crate) use history_event_context::{HistoryEventContextOverrides, HistoryEventContextResolver};
pub(crate) use prompt_activity::{
    ActivePromptState, ActiveTurnPhase, ActiveTurnState, ActiveTurnStore, PromptActivityStore,
    PromptWorkspaceClaimStore,
};
pub(crate) use prompt_lifecycle::{
    serialize_remote_prompt_attachments, KernelPreparedPromptSubmission, KernelPromptAbortDispatch,
    KernelPromptCancellation, KernelPromptDispatch, KernelPromptSubmission,
    KernelQueuedPromptCancellation, KernelQueuedPromptSteer, KernelQueuedPromptUpdate,
    KernelRemotePromptDispatch,
};
pub(crate) use provider_output_claude_native::{
    claude_native_recent_terminal_failure, ClaudeNativeDispatchAttempt, ClaudeNativeProcessOutcome,
};
pub(crate) use provider_tracking::{
    ProviderCatalogCacheStore, ProviderProcessTrackingStore, TrackedProviderProcess,
};
pub(crate) use workflow_design_events::WorkflowDesignEventStore;

use crate::agent::{AgentService, AgentServiceStore};
use crate::attachment::{AttachmentService, AttachmentServiceStore};
use crate::config::DaemonConfig;
use crate::durable_state::DurableKernelStateStore;
use crate::error::DaemonError;
use crate::execution_lease::{ExecutionLease, LeasedAgent, LeasedWorkflowTurnBinding};
use crate::history::{OperationalHistoryStore, SessionHistoryStore};
use crate::provider::{
    OpenCodeProviderCatalog, ProviderProcessInfo, ProviderProcessService,
    ProviderProcessServiceStore, ProviderRunOperationLanes, RuntimeProviderRun,
};
use crate::pty::{PtyManager, PtyOutputSignal};
use crate::runtime::projection::{
    AgentRuntimeProjectionStore, DaemonConfigProjectionStore, ProviderCatalogProjectionStore,
    ProviderProcessProjectionStore, ProviderRunProjectionStore,
    RemoteRelayInventoryProjectionStore, SessionStateProjectionStore, TransportHealthStore,
};
use crate::runtime::prompt_state::PromptStateOwner;
use crate::runtime::workspace_coordinator::WorkspaceCoordinator;
use crate::session::{RuntimeSession, SessionService, SessionStateStore};
use crate::terminal::{TerminalStreamHealthStore, TerminalStreamStore};
use crate::transport::relay_client::RelayClientState;
pub(crate) use kernel_agent::KernelAgentService;
pub(crate) use kernel_session::{KernelSessionReadService, KernelSessionService};
pub(crate) use legacy_workflow_history::LegacyWorkflowHistoryStore;
pub(crate) use prompt_lifecycle::{ProviderPromptDispatcher, RemoteWorkflowTurnContextResolver};
pub(crate) use provider_activation::StartedProviderLaunch;
pub(crate) use provider_first_output_watchdog::{
    provider_first_output_timeout_candidates, provider_first_output_timeout_diagnostic,
    provider_inactivity_timeout_candidates, provider_inactivity_timeout_diagnostic,
    ProviderFirstOutputTimeoutCandidate, ProviderInactivityTimeoutCandidate,
    PROVIDER_OUTPUT_TIMEOUT_MS,
};
pub(crate) use provider_launch_failure_retry::{
    ProviderLaunchFailureRetry, ProviderLaunchFailureRetryScheduleOutcome,
    ProviderLaunchFailureRetryStore,
};
pub(crate) use provider_launch_policy::{
    apply_metaagent_launch_policy, default_provider_env_remove,
    failed_provider_resume_state_replacement,
    failed_provider_resume_state_replacement_from_message, generate_runtime_mcp_auth_token,
    granted_mcp_servers_for_agent_launch, registered_workflow_runtime_worktree_root,
    resolve_mcp_credentials_for_launch, sanitize_resume_state_for_launch,
    workspace_live_sync_protected_roots,
};
pub(crate) use provider_liveness::ProviderRunExitSessionSummary;
pub(crate) use provider_processes::{ProviderLaunchProcessRuntime, ProviderProcessReapSummary};
pub(crate) use provider_run_read::ProviderRunReadService;
pub(crate) use remote_lease::{
    PreparedLeasedProviderRun, RemoteLeaseRuntime, RemoteProviderFailure,
};

pub struct DaemonApp {
    config: DaemonConfig,
    started_at_ms: u64,
    relay_client_state: Arc<tokio::sync::RwLock<RelayClientState>>,
    pub(crate) agents: AgentServiceStore,
    pub(crate) attachments: AttachmentServiceStore,
    pty: PtyManager,
    pub(crate) providers: ProviderProcessServiceStore,
    pub(crate) provider_catalog_cache: ProviderCatalogCacheStore,
    pub(crate) provider_process_tracking: ProviderProcessTrackingStore,
    provider_launch_failure_retries: ProviderLaunchFailureRetryStore,
    external_provider_sessions: ExternalProviderSessionIndexStore,
    attached_provider_transcript_cursors: AttachedProviderTranscriptCursorStore,
    pub(crate) active_turns: ActiveTurnStore,
    pub(crate) prompt_activity: PromptActivityStore,
    prompt_workspace_claims: PromptWorkspaceClaimStore,
    prompt_state_owner: PromptStateOwner,
    pub(crate) sessions: SessionStateStore,
    history: SessionHistoryStore,
    operational_history: OperationalHistoryStore,
    durable_state: DurableKernelStateStore,
    managed_context_transfers: crate::managed_context::transfer::ManagedContextTransferStore,
    managed_context_outbound:
        crate::managed_context::outbound_service::ManagedContextOutboundOperationStore,
    managed_kernel_registration:
        Option<crate::managed_bootstrap::ConfirmedManagedKernelRegistration>,
    legacy_workflow_history: LegacyWorkflowHistoryStore,
    provider_account_profiles: crate::account_profile::ProviderAccountProfileRegistry,
    metaagent_events: crate::runtime::metaagent_event::MetaagentEventStore,
    metaagent_trace_subscriptions: crate::runtime::metaagent_trace::MetaagentTraceSubscriptionStore,
    config_projection: DaemonConfigProjectionStore,
    session_projection: SessionStateProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
    provider_catalog_projection: ProviderCatalogProjectionStore,
    provider_run_projection: ProviderRunProjectionStore,
    provider_process_projection: ProviderProcessProjectionStore,
    remote_relay_inventory_projection: RemoteRelayInventoryProjectionStore,
    credential_enrollment_control:
        crate::runtime::credential_enrollment_control::CredentialEnrollmentControl,
    transport_health: TransportHealthStore,
    workspace_coordinator: WorkspaceCoordinator,
    terminal: TerminalStreamStore,
    workflow_design_events: WorkflowDesignEventStore,
    pending_structured_output_records: provider_output::StructuredOutputRecordStore,
    execution_leases: BTreeMap<String, ExecutionLease>,
    leased_agents: BTreeMap<String, LeasedAgent>,
    /// Workflow bindings are keyed by backing/home prompt, not provider run.
    /// A provider run can have one active turn plus queued turns, each with a
    /// different workflow context and capability snapshot.
    leased_workflow_turns: BTreeMap<String, LeasedWorkflowTurnBinding>,
    remote_git_turn_snapshots: crate::git_observer::GitTurnSnapshotStore,
    completed_git_turn_snapshots: crate::git_observer::CompletedGitTurnSnapshotStore,
    slices: crate::slice::SliceStore,
    next_execution_lease_number: u64,
    next_leased_agent_number: u64,
}

impl DaemonApp {
    pub fn bootstrap(config: DaemonConfig) -> Result<Self, DaemonError> {
        let bootstrap_started = Instant::now();
        let validate_started = Instant::now();
        config.validate()?;
        if config.user_config.credential_vault.backend
            == crate::config::CredentialVaultBackend::CharioxEncrypted
        {
            crate::secret::restore_transferred_vault_unlock(
                &config.user_config.credential_vault.path,
                &config.daemon_id,
                &config.relay_private_key,
            )?;
        }
        crate::logging::info_with_fields(
            "daemon.startup",
            "daemon config validated",
            serde_json::json!({
                "validate_ms": validate_started.elapsed().as_millis(),
                "bootstrap_elapsed_ms": bootstrap_started.elapsed().as_millis(),
            }),
        );

        let history_started = Instant::now();
        let history = SessionHistoryStore::new_with_read_delay(
            config.session_history_root(),
            config.session_history_read_delay_ms,
        )?;
        crate::logging::info_with_fields(
            "daemon.startup",
            "session history store opened",
            serde_json::json!({
                "open_ms": history_started.elapsed().as_millis(),
                "bootstrap_elapsed_ms": bootstrap_started.elapsed().as_millis(),
                "session_history_root": config.session_history_root().display().to_string(),
            }),
        );

        let operational_history_started = Instant::now();
        let operational_history = OperationalHistoryStore::open_with_read_delay_and_max_size(
            config.operational_history_path(),
            config.operational_history_read_delay_ms,
            config.operational_history_max_size_bytes(),
        )?;
        operational_history.set_capture_enabled(config.user_config.history.operational.enabled);
        crate::logging::info_with_fields(
            "daemon.startup",
            "operational history store opened",
            serde_json::json!({
                "open_ms": operational_history_started.elapsed().as_millis(),
                "bootstrap_elapsed_ms": bootstrap_started.elapsed().as_millis(),
            }),
        );

        let durable_state_started = Instant::now();
        let durable_state = DurableKernelStateStore::open_owned(config.durable_state_path())?;
        let managed_context_root = config.private_runtime_state_root();
        let managed_kernel_registration =
            crate::managed_bootstrap::confirmed_managed_kernel_registration_from_env()?;
        let managed_context_launch_recovery =
            managed_kernel_registration
                .as_ref()
                .and_then(|registration| {
                    registration.context_plan.as_ref().map(|plan| {
                        crate::managed_context::transfer::ManagedContextLaunchRecoveryBinding {
                            environment_id: registration.environment_id.clone(),
                            kernel_id: registration.kernel_id.clone(),
                            plan: plan.package_binding(),
                        }
                    })
                });
        let managed_context_transfers =
            crate::managed_context::transfer::ManagedContextTransferStore::open_with_launch_recovery(
                managed_context_root.join("managed-context-transfers"),
                managed_context_launch_recovery.as_ref(),
            )?;
        let managed_context_outbound =
            crate::managed_context::outbound_service::ManagedContextOutboundOperationStore::open(
                managed_context_root.join("managed-context-outbound"),
            )?;
        crate::logging::info_with_fields(
            "daemon.startup",
            "durable state store opened",
            serde_json::json!({
                "open_ms": durable_state_started.elapsed().as_millis(),
                "bootstrap_elapsed_ms": bootstrap_started.elapsed().as_millis(),
            }),
        );

        let provider_account_profiles =
            crate::account_profile::ProviderAccountProfileRegistry::open(
                config.account_profile_registry_path(),
            )?;
        crate::publication_provider_accounts::materialize_publication_provider_accounts(
            &provider_account_profiles,
            crate::session::DEFAULT_LOCAL_USER_ID,
        )?;
        let provider_home = std::env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        provider_account_profiles
            .migrate_effective_defaults(crate::session::DEFAULT_LOCAL_USER_ID, &provider_home)?;

        let mut app = Self {
            agents: AgentServiceStore::new(AgentService::new()),
            attachments: AttachmentServiceStore::new(AttachmentService::new()),
            pty: PtyManager::new(),
            providers: ProviderProcessServiceStore::new(ProviderProcessService::new()),
            provider_catalog_cache: ProviderCatalogCacheStore::default(),
            provider_process_tracking: ProviderProcessTrackingStore::default(),
            provider_launch_failure_retries: ProviderLaunchFailureRetryStore::default(),
            external_provider_sessions: ExternalProviderSessionIndexStore::default(),
            attached_provider_transcript_cursors: AttachedProviderTranscriptCursorStore::default(),
            active_turns: ActiveTurnStore::default(),
            prompt_activity: PromptActivityStore::default(),
            prompt_workspace_claims: PromptWorkspaceClaimStore::default(),
            prompt_state_owner: PromptStateOwner::default(),
            sessions: SessionStateStore::new(SessionService::new(&config)),
            history,
            operational_history,
            durable_state,
            managed_context_transfers,
            managed_context_outbound,
            managed_kernel_registration,
            legacy_workflow_history: LegacyWorkflowHistoryStore::default(),
            provider_account_profiles,
            metaagent_events: crate::runtime::metaagent_event::MetaagentEventStore::default(),
            metaagent_trace_subscriptions:
                crate::runtime::metaagent_trace::MetaagentTraceSubscriptionStore::default(),
            config_projection: DaemonConfigProjectionStore::new(config.clone()),
            session_projection: SessionStateProjectionStore::default(),
            agent_runtime_projection: AgentRuntimeProjectionStore::default(),
            provider_catalog_projection: ProviderCatalogProjectionStore::default(),
            provider_run_projection: ProviderRunProjectionStore::default(),
            provider_process_projection: ProviderProcessProjectionStore::default(),
            remote_relay_inventory_projection: RemoteRelayInventoryProjectionStore::default(),
            credential_enrollment_control:
                crate::runtime::credential_enrollment_control::CredentialEnrollmentControl::default(
                ),
            transport_health: TransportHealthStore::default(),
            workspace_coordinator: WorkspaceCoordinator::default(),
            terminal: TerminalStreamStore::new(),
            workflow_design_events: WorkflowDesignEventStore::default(),
            pending_structured_output_records:
                provider_output::StructuredOutputRecordStore::default(),
            execution_leases: BTreeMap::new(),
            leased_agents: BTreeMap::new(),
            leased_workflow_turns: BTreeMap::new(),
            remote_git_turn_snapshots: crate::git_observer::GitTurnSnapshotStore::default(),
            completed_git_turn_snapshots:
                crate::git_observer::CompletedGitTurnSnapshotStore::default(),
            slices: crate::slice::SliceStore::default(),
            next_execution_lease_number: 0,
            next_leased_agent_number: 0,
            started_at_ms: crate::session::unix_epoch_ms(),
            relay_client_state: Arc::new(tokio::sync::RwLock::new(RelayClientState::default())),
            config,
        };
        let restore_started = Instant::now();
        app.restore_durable_state()?;
        let restored_publication_tunnel_count = {
            let sessions = app.sessions();
            let mut relay_state = app.relay_client_state.try_write().map_err(|error| {
                DaemonError::LocalTransport {
                    operation: "restore workflow publication tunnels",
                    message: error.to_string(),
                }
            })?;
            crate::runtime::state::workflow_publication_endpoint_runtime::restore_durable_workflow_publication_tunnels(
                &mut relay_state,
                &sessions,
                crate::session::unix_epoch_ms(),
            )
        };
        crate::logging::info_with_fields(
            "daemon.startup",
            "durable state restored",
            serde_json::json!({
                "restore_ms": restore_started.elapsed().as_millis(),
                "bootstrap_elapsed_ms": bootstrap_started.elapsed().as_millis(),
                "restored_publication_tunnel_count": restored_publication_tunnel_count,
            }),
        );
        crate::logging::info_with_fields(
            "daemon.startup",
            "kernel runtime initialized",
            serde_json::json!({
                "bootstrap_ms": bootstrap_started.elapsed().as_millis(),
                "daemon_id": app.config.daemon_id.as_str(),
                "machine_id": app.config.host_machine_id.as_str(),
            }),
        );
        Ok(app)
    }

    pub(crate) fn provider_run_operation_lanes(&self) -> ProviderRunOperationLanes {
        self.providers.run_operation_lanes()
    }

    pub fn config(&self) -> &DaemonConfig {
        &self.config
    }

    pub(crate) fn slices(&self) -> crate::slice::SliceStore {
        self.slices.clone()
    }

    pub(crate) fn relay_client_state(&self) -> Arc<tokio::sync::RwLock<RelayClientState>> {
        Arc::clone(&self.relay_client_state)
    }

    pub(crate) fn credential_enrollment_control(
        &self,
    ) -> crate::runtime::credential_enrollment_control::CredentialEnrollmentControl {
        self.credential_enrollment_control.clone()
    }

    pub(crate) fn config_projection_store(&self) -> DaemonConfigProjectionStore {
        self.config_projection.clone()
    }

    pub(crate) fn session_state_store(&self) -> SessionStateStore {
        self.sessions.clone()
    }

    pub fn sessions(&self) -> SessionService {
        self.sessions.snapshot()
    }

    pub(crate) fn history_store(&self) -> SessionHistoryStore {
        self.history.clone()
    }

    pub(crate) fn operational_history_store(&self) -> OperationalHistoryStore {
        self.operational_history.clone()
    }

    pub(crate) fn durable_state_store(&self) -> DurableKernelStateStore {
        self.durable_state.clone()
    }

    pub(crate) fn managed_context_transfer_store(
        &self,
    ) -> crate::managed_context::transfer::ManagedContextTransferStore {
        self.managed_context_transfers.clone()
    }

    pub(crate) fn managed_context_outbound_operation_store(
        &self,
    ) -> crate::managed_context::outbound_service::ManagedContextOutboundOperationStore {
        self.managed_context_outbound.clone()
    }

    pub(crate) fn managed_kernel_registration(
        &self,
    ) -> Option<crate::managed_bootstrap::ConfirmedManagedKernelRegistration> {
        self.managed_kernel_registration.clone()
    }

    pub(crate) fn legacy_workflow_history_store(&self) -> LegacyWorkflowHistoryStore {
        self.legacy_workflow_history.clone()
    }

    pub(crate) fn provider_account_profile_registry(
        &self,
    ) -> crate::account_profile::ProviderAccountProfileRegistry {
        self.provider_account_profiles.clone()
    }

    pub(crate) fn metaagent_event_store(
        &self,
    ) -> crate::runtime::metaagent_event::MetaagentEventStore {
        self.metaagent_events.clone()
    }

    pub(crate) fn metaagent_trace_subscription_store(
        &self,
    ) -> crate::runtime::metaagent_trace::MetaagentTraceSubscriptionStore {
        self.metaagent_trace_subscriptions.clone()
    }

    pub(crate) fn session_state_projection_store(&self) -> SessionStateProjectionStore {
        self.session_projection.clone()
    }

    pub(crate) fn agent_runtime_projection_store(&self) -> AgentRuntimeProjectionStore {
        self.agent_runtime_projection.clone()
    }

    pub(crate) fn prompt_state_owner(&self) -> PromptStateOwner {
        self.prompt_state_owner.clone()
    }

    pub(crate) fn prompt_id_allocator(&self) -> crate::session::PromptIdAllocator {
        self.sessions.prompt_id_allocator()
    }

    pub(crate) fn update_session_projection(&self, mut session: RuntimeSession) {
        self.prompt_state_owner.project_into_session(&mut session);
        self.agent_runtime_projection.update_session(&session);
        self.session_projection.update(session);
    }

    pub(crate) fn provider_process_tracking_store(&self) -> ProviderProcessTrackingStore {
        self.provider_process_tracking.clone()
    }

    pub(crate) fn provider_launch_failure_retry_store(&self) -> ProviderLaunchFailureRetryStore {
        self.provider_launch_failure_retries.clone()
    }

    pub(crate) fn external_provider_session_index_store(
        &self,
    ) -> ExternalProviderSessionIndexStore {
        self.external_provider_sessions.clone()
    }

    pub(crate) fn attached_provider_transcript_cursor_store(
        &self,
    ) -> AttachedProviderTranscriptCursorStore {
        self.attached_provider_transcript_cursors.clone()
    }

    pub(crate) fn provider_catalog_projection_store(&self) -> ProviderCatalogProjectionStore {
        self.provider_catalog_projection.clone()
    }

    pub(crate) fn remote_relay_inventory_projection_store(
        &self,
    ) -> RemoteRelayInventoryProjectionStore {
        self.remote_relay_inventory_projection.clone()
    }

    pub(crate) fn update_provider_catalog_projection(&self, catalog: OpenCodeProviderCatalog) {
        self.provider_catalog_projection.update(catalog);
    }

    pub(crate) fn invalidate_provider_catalog_projection(&self) {
        self.provider_catalog_projection.invalidate();
    }

    pub(crate) fn provider_run_projection_store(&self) -> ProviderRunProjectionStore {
        self.provider_run_projection.clone()
    }

    pub(crate) fn provider_process_projection_store(&self) -> ProviderProcessProjectionStore {
        self.provider_process_projection.clone()
    }

    pub(crate) fn transport_health_store(&self) -> TransportHealthStore {
        self.transport_health.clone()
    }

    pub(crate) fn prompt_activity_store(&self) -> PromptActivityStore {
        self.prompt_activity.clone()
    }

    pub(crate) fn active_turn_store(&self) -> ActiveTurnStore {
        self.active_turns.clone()
    }

    pub(crate) fn completed_git_turn_snapshot_store(
        &self,
    ) -> crate::git_observer::CompletedGitTurnSnapshotStore {
        self.completed_git_turn_snapshots.clone()
    }

    pub(crate) fn prompt_workspace_claim_store(&self) -> PromptWorkspaceClaimStore {
        self.prompt_workspace_claims.clone()
    }

    pub(crate) fn structured_output_record_store(
        &self,
    ) -> provider_output::StructuredOutputRecordStore {
        self.pending_structured_output_records.clone()
    }

    pub(crate) fn workspace_coordinator(&self) -> WorkspaceCoordinator {
        self.workspace_coordinator.clone()
    }

    pub(crate) fn update_provider_run_projection(&self, run: RuntimeProviderRun) {
        self.provider_run_projection.update(run);
        self.provider_process_projection.invalidate();
    }

    pub(crate) fn mark_leased_provider_run(&self, provider_run_id: &str) {
        self.provider_run_projection
            .mark_leased_provider_run(provider_run_id);
    }

    pub(crate) fn update_remote_provider_run_projection(
        &self,
        run: RuntimeProviderRun,
    ) -> RuntimeProviderRun {
        let run = self.provider_run_projection.update_remote_snapshot(run);
        self.provider_process_projection.invalidate();
        run
    }

    pub(crate) fn update_provider_process_projection(&self, processes: Vec<ProviderProcessInfo>) {
        self.provider_process_projection.update_list(processes);
    }

    pub fn sessions_mut(&self) -> std::sync::RwLockWriteGuard<'_, SessionService> {
        self.sessions.write()
    }

    pub fn agents(&self) -> &AgentServiceStore {
        &self.agents
    }

    pub fn agents_mut(&self) -> std::sync::MutexGuard<'_, AgentService> {
        self.agents.write()
    }

    pub fn attachments(&self) -> &AttachmentServiceStore {
        &self.attachments
    }

    pub fn attachments_mut(&self) -> std::sync::MutexGuard<'_, AttachmentService> {
        self.attachments.write()
    }

    pub fn providers(&self) -> &ProviderProcessServiceStore {
        &self.providers
    }

    pub fn providers_mut(&self) -> std::sync::MutexGuard<'_, ProviderProcessService> {
        self.providers.write()
    }

    pub fn terminal(&self) -> &TerminalStreamStore {
        &self.terminal
    }

    pub(crate) fn terminal_health_store(&self) -> TerminalStreamHealthStore {
        self.terminal.health_store()
    }

    pub(crate) fn terminal_stream_store(&self) -> TerminalStreamStore {
        self.terminal.clone()
    }

    pub(crate) fn workflow_design_event_store(&self) -> WorkflowDesignEventStore {
        self.workflow_design_events.clone()
    }

    pub(crate) fn terminal_mut(&mut self) -> &TerminalStreamStore {
        &self.terminal
    }

    pub fn pty(&self) -> &PtyManager {
        &self.pty
    }

    pub(crate) fn pty_output_signal(&self) -> PtyOutputSignal {
        self.pty.output_signal()
    }

    pub(crate) fn pty_mut(&mut self) -> &mut PtyManager {
        &mut self.pty
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::CreateAgentRequest;
    use crate::provider::LaunchProviderRequest;
    use crate::session::CreateSessionRequest;

    #[test]
    fn durable_restore_keeps_sessions_bound_to_their_kernel_id() {
        let state_path = std::env::temp_dir().join("chariox-tests").join(format!(
            "shared-kernel-state-{}.db",
            crate::session::unix_epoch_ms()
        ));
        let mut config_a = DaemonConfig::for_tests();
        config_a.daemon_id = "kernel-a".to_string();
        config_a.user_config.state.path = Some(state_path.display().to_string());
        let session_id = {
            let mut app = DaemonApp::bootstrap(config_a.clone()).expect("kernel a should boot");
            let (session, _) = app
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should create");
            session.id().to_string()
        };

        let mut config_b = DaemonConfig::for_tests();
        config_b.daemon_id = "kernel-b".to_string();
        config_b.user_config.state.path = Some(state_path.display().to_string());
        let app_b = DaemonApp::bootstrap(config_b).expect("kernel b should boot");
        assert!(app_b.sessions().list_sessions().is_empty());
        drop(app_b);

        let app_a = DaemonApp::bootstrap(config_a).expect("kernel a should reboot");
        assert!(app_a.sessions().get_session(&session_id).is_ok());

        let _ = std::fs::remove_file(state_path);
    }

    #[test]
    fn durable_restore_ignores_orphan_ephemeral_prompt_state() {
        let state_path = std::env::temp_dir().join("chariox-tests").join(format!(
            "orphan-ephemeral-prompt-state-{}.db",
            crate::session::unix_epoch_ms()
        ));
        let mut config = DaemonConfig::for_tests();
        config.user_config.state.path = Some(state_path.display().to_string());
        {
            let app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
            app.durable_state_store()
                .append_event(
                    crate::durable_prompt_state::DURABLE_PROMPT_STATE_EVENT_KIND,
                    Some("missing-worker-session".to_string()),
                    serde_json::json!({
                        "session_id": "missing-worker-session",
                        "agent_id": "missing-worker-agent",
                        "active_prompt": null,
                        "queued_prompts": [],
                    }),
                )
                .expect("orphan prompt event should persist");
        }

        let restored = DaemonApp::bootstrap(config)
            .expect("orphan ephemeral prompt state must not prevent restart");
        assert!(restored.sessions().list_sessions().is_empty());

        let _ = std::fs::remove_file(state_path);
    }

    #[test]
    fn durable_restore_keeps_slices_bound_to_their_owner_kernel_id() {
        let state_path = std::env::temp_dir().join("chariox-tests").join(format!(
            "shared-slice-state-{}.db",
            crate::session::unix_epoch_ms()
        ));
        let mut config_a = DaemonConfig::for_tests();
        config_a.daemon_id = "kernel-a".to_string();
        config_a.user_config.state.path = Some(state_path.display().to_string());
        let slice_id = {
            let app = DaemonApp::bootstrap(config_a.clone()).expect("kernel a should boot");
            let slice = app
                .slices()
                .create(
                    &app.config().daemon_id,
                    &app.config().host_machine_id,
                    crate::slice::CreateSliceInput {
                        name: "linux-dev".to_string(),
                        backend: crate::slice::SliceBackendKind::LocalDocker,
                        os: "linux".to_string(),
                        display_mode: crate::slice::SliceDisplayMode::Headed,
                        workspace_id: None,
                        worktree_id: None,
                        workspace_mount: Some("/repo".to_string()),
                        development: None,
                        worker_kernel_ref: None,
                        display_url: Some("http://127.0.0.1:6080".to_string()),
                        provider_auth: Vec::new(),
                        from_saved_state: None,
                        now_ms: 42,
                    },
                )
                .expect("slice should create");
            app.durable_state_store()
                .append_event(
                    "slice.created",
                    Some(slice.id.clone()),
                    serde_json::json!({ "slice": &slice }),
                )
                .expect("slice event should persist");
            slice.id
        };

        let mut config_b = DaemonConfig::for_tests();
        config_b.daemon_id = "kernel-b".to_string();
        config_b.user_config.state.path = Some(state_path.display().to_string());
        let app_b = DaemonApp::bootstrap(config_b).expect("kernel b should boot");
        assert!(app_b.slices().list().is_empty());
        drop(app_b);

        let app_a = DaemonApp::bootstrap(config_a).expect("kernel a should reboot");
        assert_eq!(
            app_a
                .slices()
                .resolve("linux-dev")
                .expect("slice should restore")
                .id,
            slice_id
        );

        let _ = std::fs::remove_file(state_path);
    }

    #[test]
    fn daemon_restart_restores_sessions_after_shutdown_cleanup() {
        let state_path = std::env::temp_dir().join("chariox-tests").join(format!(
            "restart-preserves-sessions-{}.db",
            crate::session::unix_epoch_ms()
        ));
        let mut config = DaemonConfig::for_tests();
        config.user_config.state.path = Some(state_path.display().to_string());

        let session_id = {
            let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
            let (session, _) = app
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should create");
            app.shutdown_cleanup()
                .expect("shutdown should clean runtime without ending session");
            session.id().to_string()
        };

        let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
        let restored = app
            .sessions()
            .get_session(&session_id)
            .expect("session should restore after daemon restart");
        assert_ne!(restored.status(), crate::session::SessionStatus::Ended);
        assert!(
            app.agents().get_session_agents(&session_id).len() == 1,
            "default agent should restore for preserved session"
        );

        let _ = std::fs::remove_file(state_path);
    }

    #[test]
    fn durable_restore_ignores_newer_snapshot_from_other_kernel_owner() {
        let state_path = std::env::temp_dir().join("chariox-tests").join(format!(
            "shared-kernel-snapshot-owner-{}.db",
            crate::session::unix_epoch_ms()
        ));
        let mut config_a = DaemonConfig::for_tests();
        config_a.daemon_id = "kernel-a".to_string();
        config_a.user_config.state.path = Some(state_path.display().to_string());
        let session_id = {
            let mut app = DaemonApp::bootstrap(config_a.clone()).expect("kernel a should boot");
            let (session, _) = app
                .create_session(CreateSessionRequest::new("workspace-a", "worktree-a"))
                .expect("session should create");
            session.id().to_string()
        };

        let mut config_b = DaemonConfig::for_tests();
        config_b.daemon_id = "kernel-b".to_string();
        config_b.user_config.state.path = Some(state_path.display().to_string());
        {
            let mut app = DaemonApp::bootstrap(config_b).expect("kernel b should boot");
            app.create_session(CreateSessionRequest::new("workspace-b", "worktree-b"))
                .expect("kernel b session should create");
            app.save_durable_state_snapshot()
                .expect("kernel b should write latest snapshot");
        }

        let app = DaemonApp::bootstrap(config_a).expect("kernel a should reboot");
        let restored = app
            .sessions()
            .get_session(&session_id)
            .expect("kernel a session should restore from event log");
        assert_eq!(restored.host_daemon_id(), "kernel-a");

        let _ = std::fs::remove_file(state_path);
    }

    #[test]
    fn durable_restore_republishes_agent_runtime_profile_to_session_projection() {
        let state_path = std::env::temp_dir().join("chariox-tests").join(format!(
            "restart-agent-projection-{}.db",
            crate::session::unix_epoch_ms()
        ));
        let mut config = DaemonConfig::for_tests();
        config.user_config.state.path = Some(state_path.display().to_string());

        let session_id = {
            let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
            let (session, agent) = app
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should create");
            app.launch_provider(
                LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "claude-code",
                    "default",
                    "sonnet",
                )
                .with_agent_id(agent.id()),
            )
            .expect("provider should launch and persist runtime profile");
            session.id().to_string()
        };

        let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
        let projected = app
            .session_projection
            .get(&session_id)
            .expect("session projection should restore");
        let projected_agent = projected
            .agents()
            .first()
            .expect("projected session should include restored agent");

        assert_eq!(projected_agent.provider(), "claude-code");
        assert_eq!(projected_agent.model(), Some("sonnet"));

        let _ = std::fs::remove_file(state_path);
    }

    #[test]
    fn durable_restore_preserves_metaagent_event_inbox_state() {
        let state_path = std::env::temp_dir().join("chariox-tests").join(format!(
            "restart-metaagent-events-{}.db",
            crate::session::unix_epoch_ms()
        ));
        let mut config = DaemonConfig::for_tests();
        config.user_config.state.path = Some(state_path.display().to_string());

        let (metaagent_id, event_id, subscription_id) = {
            let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
            let (session, _default_agent) = app
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should create");
            let worker = crate::app::KernelSessionService::new(&mut app)
                .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("worker"))
                .expect("worker should spawn");
            let metaagent = crate::app::KernelSessionService::new(&mut app)
                .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
                .expect("metaagent should spawn");
            let metaagent = app
                .agents_mut()
                .activate_agent_meta_mode(metaagent.id(), None)
                .expect("agent should enter meta mode");
            let event = app.metaagent_event_store().record(
                crate::runtime::metaagent_event::NewMetaagentEvent {
                    session_id: session.id().to_string(),
                    metaagent_id: metaagent.id().to_string(),
                    owner_user_id: metaagent.owner_user_id().to_string(),
                    kind: "agent.turn.completed".to_string(),
                    source_agent_id: Some(worker.id().to_string()),
                    title: "Worker completed".to_string(),
                    summary: "Worker completed a turn".to_string(),
                    detail: serde_json::json!({ "turn_id": "turn-1" }),
                    injected_prompt_id: Some("prompt-1".to_string()),
                },
            );
            app.durable_state_store()
                .append_event(
                    "metaagent.event.recorded",
                    Some(event.event_id.clone()),
                    serde_json::json!({ "record": &event }),
                )
                .expect("recorded event should persist");
            let read_event = app
                .metaagent_event_store()
                .read(metaagent.id(), &event.event_id)
                .expect("event should read");
            app.durable_state_store()
                .append_event(
                    "metaagent.event.read",
                    Some(read_event.event_id.clone()),
                    serde_json::json!({ "record": &read_event }),
                )
                .expect("read event should persist");
            let acked_event = app
                .metaagent_event_store()
                .ack(metaagent.id(), &[event.event_id.clone()], None)
                .into_iter()
                .next()
                .expect("event should ack");
            app.durable_state_store()
                .append_event(
                    "metaagent.event.acked",
                    Some(acked_event.event_id.clone()),
                    serde_json::json!({ "record": &acked_event }),
                )
                .expect("acked event should persist");
            let subscription = app.metaagent_event_store().subscribe(
                metaagent.id(),
                "workflow.output.final".to_string(),
                None,
            );
            app.durable_state_store()
                .append_event(
                    "metaagent.subscription.created",
                    Some(subscription.subscription_id.clone()),
                    serde_json::json!({ "subscription": &subscription }),
                )
                .expect("subscription should persist");
            (
                metaagent.id().to_string(),
                event.event_id,
                subscription.subscription_id,
            )
        };

        let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
        let restored_events =
            app.metaagent_event_store()
                .list(&metaagent_id, Some("agent.turn.completed"), None, 10);
        assert_eq!(restored_events.len(), 1);
        let restored_event = &restored_events[0];
        assert_eq!(restored_event.event_id, event_id);
        assert!(
            restored_event.read_at_ms.is_some(),
            "read state should survive restart: {restored_event:?}"
        );
        assert!(
            restored_event.ack_at_ms.is_some(),
            "ack state should survive restart: {restored_event:?}"
        );
        assert_eq!(
            restored_event.injected_prompt_id.as_deref(),
            Some("prompt-1")
        );
        assert_eq!(
            app.metaagent_event_store()
                .list(&metaagent_id, None, Some("unacked"), 10)
                .len(),
            0
        );
        let subscriptions = app
            .metaagent_event_store()
            .list_subscriptions(&metaagent_id);
        assert!(subscriptions.iter().any(|subscription| {
            subscription.subscription_id == subscription_id
                && subscription.kind == "workflow.output.final"
        }));

        let _ = std::fs::remove_file(state_path);
    }
}
