//! Shared runtime-state facade and async orchestration wiring.
//!
//! Domain modules own the concrete session/provider/prompt/workflow and workspace live sync mutations.
//! This root keeps the public `KernelRuntimeState` entry points, shared fields, and cross-domain
//! plumbing that would otherwise create cycles between those modules.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use tokio::sync::{Mutex, Notify};

use crate::agent::AgentServiceStore;
use crate::app::{
    ActiveTurnStore, AttachedProviderTranscriptCursorStore, DaemonApp,
    ExternalProviderSessionIndexStore, PromptActivityStore, PromptWorkspaceClaimStore,
    ProviderLaunchFailureRetryStore, ProviderProcessTrackingStore, WorkflowDesignEventStore,
};
use crate::attachment::AttachmentServiceStore;
use crate::durable_state::DurableKernelStateStore;
use crate::error::DaemonError;
use crate::history::{OperationalHistoryStore, SessionHistoryEntry};
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::provider::{ProviderProcessServiceStore, ProviderRunOperationLanes};
use crate::runtime::metaagent_event::MetaagentEventStore;
use crate::session::{SessionStateOwner, SessionStateStore};
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use chariox_relay::protocol::ClientTarget;

mod workspace_live_sync;
use workspace_live_sync::*;
mod workspace_live_sync_workspace_context;
use workspace_live_sync_workspace_context::*;
mod context_handoff;
use context_handoff::*;
mod computer_secret_input_runtime_state;
mod config_runtime_state;
mod provider_output_deadline_store;
mod provider_reload;
use provider_output_deadline_store::ProviderOutputDeadlineStore;
pub(crate) use provider_reload::*;
mod event_delivery_runtime_state;
mod human_browser_action_runtime_state;
mod human_environment_action_runtime_state;
mod managed_activity_runtime_state;
mod provider_launch_defaults_owned_state;
mod provider_relaunch_runtime;
mod provider_reload_pending_runtime;
mod provider_run_read_state;
mod publication_activation;
mod room_browser_controller;
mod room_computer_observation;
mod room_display;
mod room_environment_placement;
mod room_environment_state;
mod room_screenshot;

#[derive(Clone)]
pub(crate) struct KernelRuntimeState {
    app: Arc<Mutex<DaemonApp>>,
    provider_runtime_lanes: ProviderRunOperationLanes,
    detached_workflow_provider_launches: Arc<std::sync::Mutex<BTreeSet<String>>>,
    owned: KernelRuntimeOwnedState,
}

#[derive(Clone)]
struct KernelRuntimeOwnedState {
    config_projection: crate::runtime::projection::DaemonConfigProjectionStore,
    session_store: SessionStateStore,
    agent_store: AgentServiceStore,
    attachment_store: AttachmentServiceStore,
    provider_store: ProviderProcessServiceStore,
    pending_provider_launch_credentials: Arc<
        std::sync::Mutex<
            BTreeMap<String, crate::provider::ProviderCredentialEnvironment>,
        >,
    >,
    workflow_provider_launch_lock: Arc<std::sync::Mutex<()>>,
    workflow_instance_provision_lock: Arc<std::sync::Mutex<()>>,
    publication_activation: Arc<publication_activation::PublicationActivation>,
    provider_process_tracking: ProviderProcessTrackingStore,
    provider_launch_failure_retries: ProviderLaunchFailureRetryStore,
    external_provider_sessions: ExternalProviderSessionIndexStore,
    attached_provider_transcript_cursors: AttachedProviderTranscriptCursorStore,
    slice_store: crate::slice::SliceStore,
    browser_controller_processes:
        crate::runtime::browser_controller_process::BrowserControllerProcessStore,
    computer_input_executions:
        crate::runtime::computer_input_execution::ComputerInputExecutionStore,
    browser_controller_generations:
        Arc<std::sync::Mutex<BTreeMap<String, (u64, bool)>>>,
    session_projection: crate::runtime::projection::SessionStateProjectionStore,
    agent_runtime_projection: crate::runtime::projection::AgentRuntimeProjectionStore,
    provider_run_projection: crate::runtime::projection::ProviderRunProjectionStore,
    provider_process_projection: crate::runtime::projection::ProviderProcessProjectionStore,
    operational_history_store: OperationalHistoryStore,
    transcript_history_append_lock: Arc<std::sync::Mutex<()>>,
    durable_state_store: DurableKernelStateStore,
    legacy_workflow_history: crate::app::LegacyWorkflowHistoryStore,
    provider_account_profiles: crate::account_profile::ProviderAccountProfileRegistry,
    provider_login_processes: ProviderLoginProcessStore,
    event_connection_registry: crate::event_connection::EventConnectionRegistry,
    prompt_state_owner: crate::runtime::prompt_state::PromptStateOwner,
    active_turns: ActiveTurnStore,
    prompt_activity: PromptActivityStore,
    provider_output_deadlines: ProviderOutputDeadlineStore,
    prompt_workspace_claims: PromptWorkspaceClaimStore,
    structured_output_records: crate::app::provider_output::StructuredOutputRecordStore,
    pty_output_signal: crate::pty::PtyOutputSignal,
    terminal_stream: crate::terminal::TerminalStreamStore,
    runtime_projection_changes: Arc<RuntimeChangeSignal>,
    workflow_design_events: WorkflowDesignEventStore,
    workspace_coordinator: crate::runtime::workspace_coordinator::WorkspaceCoordinator,
    workspace_live_sync_coordinator: Arc<Mutex<crate::io::ArtifactEditCoordinator>>,
    workspace_live_sync_external_changes: crate::io::ArtifactExternalChangeMonitor,
    workspace_identity_monitor:
        crate::runtime::workspace_identity_monitor::WorkspaceIdentityMonitor,
    pending_agent_context_handoffs: PendingAgentContextHandoffStore,
    pending_mcp_continuations: PendingMcpContinuationStore,
    metaagent_events: crate::runtime::metaagent_event::MetaagentEventStore,
    metaagent_trace_subscriptions: crate::runtime::metaagent_trace::MetaagentTraceSubscriptionStore,
    connector_adapter_processes: crate::connector::ConnectorAdapterProcessPool,
    pending_provider_reloads: PendingProviderReloadStore,
    pending_interactions: PendingInteractionStore,
    git_turn_snapshots: crate::git_observer::GitTurnSnapshotStore,
    completed_git_turn_snapshots: crate::git_observer::CompletedGitTurnSnapshotStore,
    workspace_live_sync_journal: crate::git_observer::WorkspaceLiveSyncJournal,
    remote_workspace_live_sync_invocations:
        Arc<Mutex<BTreeMap<String, RemoteWorkspaceLiveSyncInvocationState>>>,
    remote_extension_invocations: Arc<Mutex<BTreeMap<String, RemoteExtensionInvocationState>>>,
    remote_extension_cancellations: Arc<Mutex<std::collections::BTreeSet<String>>>,
    remote_home_extension_inflight:
        Arc<Mutex<BTreeMap<String, Vec<RemoteHomeExtensionInflightInvocation>>>>,
    remote_extension_manifest_retry_counts: Arc<Mutex<BTreeMap<String, u32>>>,
    agent_message_idempotency: Arc<Mutex<AgentMessageIdempotencyStore>>,
    next_provider_process_gc_at_ms: Arc<AtomicU64>,
    relay_state: Arc<tokio::sync::RwLock<crate::transport::relay_client::RelayClientState>>,
    remote_prompt_projection_drains:
        Arc<std::sync::Mutex<BTreeMap<(String, String), u64>>>,
    remote_prompt_recoveries: Arc<std::sync::Mutex<BTreeMap<(String, String), u64>>>,
    slice_private_relay_connectors: Arc<Mutex<BTreeMap<String, SlicePrivateRelayConnector>>>,
    workflow_publication_runtimes:
        crate::runtime::state::workflow_publication_runtime_lifecycle::WorkflowPublicationRuntimeProcessStore,
}

#[derive(Debug, Clone)]
struct RemoteHomeExtensionInflightInvocation {
    context: crate::transport::relay_peer::RemoteExtensionInvocationContext,
    metadata: crate::extension::RemoteExtensionInvocationMetadata,
}

#[derive(Debug, Clone)]
struct RemoteExtensionInvocationState {
    invocation_id: String,
    result: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
struct RemoteWorkspaceLiveSyncInvocationState {
    request_fingerprint: String,
    result: Option<RemoteWorkspaceLiveSyncInvocationResult>,
    completion_tx: tokio::sync::watch::Sender<Option<RemoteWorkspaceLiveSyncInvocationResult>>,
    finalized: bool,
}

#[derive(Debug, Default)]
struct AgentMessageIdempotencyStore {
    entries: BTreeMap<String, AgentMessageIdempotencyEntry>,
    order: VecDeque<String>,
}

#[derive(Debug, Clone)]
struct AgentMessageIdempotencyEntry {
    fingerprint: String,
    result: crate::transport::runtime_tools::RuntimeToolResult,
}

impl AgentMessageIdempotencyStore {
    const LIMIT: usize = 1_024;

    fn record(
        &mut self,
        operation_id: String,
        fingerprint: String,
        result: crate::transport::runtime_tools::RuntimeToolResult,
    ) {
        if !self.entries.contains_key(&operation_id) {
            self.order.push_back(operation_id.clone());
        }
        self.entries.insert(
            operation_id,
            AgentMessageIdempotencyEntry {
                fingerprint,
                result,
            },
        );
        while self.entries.len() > Self::LIMIT {
            if let Some(expired) = self.order.pop_front() {
                self.entries.remove(&expired);
            }
        }
    }
}

type RemoteWorkspaceLiveSyncInvocationResult = (
    crate::transport::runtime_tools::RuntimeToolResult,
    Vec<crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState>,
);

struct SlicePrivateRelayConnector {
    relay_url: String,
    state: Arc<tokio::sync::RwLock<crate::transport::relay_client::RelayClientState>>,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    task: std::thread::JoinHandle<()>,
}

mod agent_config_owned_state;
mod agent_config_runtime_state;
mod agent_lifecycle_owned_state;
mod agent_profile_owned_state;
mod agent_prompt_schedule_runtime_state;
mod agent_turn_actions_runtime_state;
mod agent_utility_runtime_state;
mod attachment_owned_state;
mod browser_controller_action_cancellation_runtime_state;
mod browser_controller_action_execution_runtime_state;
mod browser_controller_compatibility_runtime_state;
pub(crate) use browser_controller_action_execution_runtime_state::BrowserControllerActionExecution;
mod browser_configuration_runtime_state;
mod browser_controller_runtime_state;
mod browser_download_cancellation_runtime_state;
mod browser_upload_runtime_state;
mod capability_owned_state;
mod detached_provider_run_owned_state;
mod owned;
mod pending_runtime_state;
use pending_runtime_state::*;
mod local_prompt_dispatch_runtime;
mod local_prompt_submission_owned_state;
mod metaagent_event_owned_state;
mod metaagent_task_runtime_state;
pub(crate) use metaagent_task_runtime_state::parse_meta_slash_command;
mod project_runtime_state;
mod prompt;
mod prompt_activity_owned_state;
mod prompt_cancellation_owned_state;
mod prompt_dispatch;
mod prompt_git_observer_runtime;
mod prompt_queue_owned_state;
mod prompt_skill_context_state;
mod prompt_transcript_owned_state;
mod provider;
mod provider_focus_owned_state;
mod provider_launch_failure_runtime;
mod provider_launch_owned_state;
mod provider_launch_runtime;
pub(crate) use provider_launch_runtime::ProviderLaunchStartOutcome;
mod provider_liveness_runtime;
mod provider_login_state;
pub(in crate::runtime) use provider_login_state::{
    ProviderAuthProcessOperation, ProviderLoginProcessBackend, ProviderLoginProcessRecord,
    ProviderLoginProcessStore, PROVIDER_LOGIN_TIMEOUT_MS,
};
mod provider_mcp_continuation_runtime;
mod provider_output_runtime;
mod provider_process_runtime_state;
pub(crate) use provider_process_runtime_state::*;
#[cfg(test)]
mod provider_output_runtime_tests;
mod provider_prompt_failure_runtime;
mod provider_prompt_settlement_runtime;
mod provider_substitute_runtime;
mod relay_peer_runtime_state;
mod remote_prompt_dispatch_runtime;
mod remote_prompt_lifecycle_runtime;
mod remote_prompt_owned_state;
mod remote_prompt_worker_submission_runtime;
mod restart_recovery_runtime;
pub(crate) use restart_recovery_runtime::is_internal_recovery_prompt_attachment;
mod agent_batch_runtime_state;
mod runtime_interaction_owned_state;
mod runtime_interaction_state;
mod runtime_notice_owned_state;
mod runtime_state_views;
mod runtime_vault_unlock_state;
mod session;
mod session_collaboration_state;
mod session_lifecycle_runtime_state;
mod session_lookup_state;
mod slice_development_runtime_state;
mod slice_runtime_state;
pub(crate) use slice_runtime_state::SliceAgentRelaunchManifest;
mod structured_provider_output_runtime;
mod terminal_runtime_state;
mod tool_dispatch;
mod transport_runtime_state;
mod workflow;
mod workflow_access_owned_state;
mod workflow_admin;
mod workflow_artifact_request_runtime_state;
mod workflow_blocked_claim_retry;
mod workflow_code_request_runtime_state;
mod workflow_code_request_support;
mod workflow_completion_owned_state;
mod workflow_completion_snapshot_owned_state;
mod workflow_console_tool;
mod workflow_definition_owned_state;
mod workflow_definition_settings_owned_state;
mod workflow_dispatch;
mod workflow_endpoint_owned_state;
mod workflow_launch_owned_state;
mod workflow_node_owned_state;
mod workflow_output_tool;
mod workflow_prompt_dispatches;
mod workflow_prompt_queue_owned_state;
use workflow_prompt_dispatches::*;
mod workflow_prompt_failure_owned_state;
pub(crate) mod workflow_publication_endpoint_runtime;
mod workflow_publication_owned_state;
pub(crate) mod workflow_publication_runtime_lifecycle;
mod workflow_query_owned_state;
mod workflow_registry_request_runtime_state;
mod workflow_request_runtime_state;
mod workflow_resume_owned_state;
mod workflow_run_request_runtime_state;
mod workflow_scheduling_owned_state;
mod workflow_tool;
mod workflow_turn_admin_owned_state;
mod workflow_turn_prompt_owned_state;

impl KernelRuntimeState {
    pub(crate) fn session_store(&self) -> &SessionStateStore {
        &self.owned.session_store
    }

    pub(crate) fn event_connection_registry(
        &self,
    ) -> &crate::event_connection::EventConnectionRegistry {
        &self.owned.event_connection_registry
    }

    #[allow(dead_code)]
    pub(crate) fn new_with_owned_state(
        app: Arc<Mutex<DaemonApp>>,
        config_projection: crate::runtime::projection::DaemonConfigProjectionStore,
        session_store: SessionStateStore,
        agent_store: AgentServiceStore,
        attachment_store: AttachmentServiceStore,
        provider_store: ProviderProcessServiceStore,
        provider_process_tracking: ProviderProcessTrackingStore,
        slice_store: crate::slice::SliceStore,
        session_projection: crate::runtime::projection::SessionStateProjectionStore,
        provider_run_projection: crate::runtime::projection::ProviderRunProjectionStore,
        operational_history_store: OperationalHistoryStore,
        durable_state_store: DurableKernelStateStore,
        prompt_state_owner: crate::runtime::prompt_state::PromptStateOwner,
        active_turns: ActiveTurnStore,
        prompt_activity: PromptActivityStore,
        prompt_workspace_claims: PromptWorkspaceClaimStore,
        structured_output_records: crate::app::provider_output::StructuredOutputRecordStore,
        terminal_stream: crate::terminal::TerminalStreamStore,
        workflow_design_events: WorkflowDesignEventStore,
        metaagent_events: MetaagentEventStore,
        workspace_coordinator: crate::runtime::workspace_coordinator::WorkspaceCoordinator,
    ) -> Self {
        let (
            external_provider_sessions,
            attached_provider_transcript_cursors,
            pty_output_signal,
            provider_account_profiles,
        ) = {
            let started = Instant::now();
            loop {
                if let Ok(app) = app.try_lock() {
                    break (
                        app.external_provider_session_index_store(),
                        app.attached_provider_transcript_cursor_store(),
                        app.pty_output_signal(),
                        app.provider_account_profile_registry(),
                    );
                }
                if started.elapsed() >= Duration::from_secs(5) {
                    panic!(
                        "KernelRuntimeState could not acquire the app lock for external provider session state during bootstrap"
                    );
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        };
        Self::new_with_owned_state_and_lanes(
            app,
            ProviderRunOperationLanes::default(),
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            external_provider_sessions,
            attached_provider_transcript_cursors,
            slice_store,
            session_projection,
            provider_run_projection,
            operational_history_store,
            durable_state_store,
            provider_account_profiles,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            pty_output_signal,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            crate::runtime::metaagent_trace::MetaagentTraceSubscriptionStore::default(),
            workspace_coordinator,
        )
    }

    pub(crate) fn new_with_owned_state_and_lanes(
        app: Arc<Mutex<DaemonApp>>,
        provider_runtime_lanes: ProviderRunOperationLanes,
        config_projection: crate::runtime::projection::DaemonConfigProjectionStore,
        session_store: SessionStateStore,
        agent_store: AgentServiceStore,
        attachment_store: AttachmentServiceStore,
        provider_store: ProviderProcessServiceStore,
        provider_process_tracking: ProviderProcessTrackingStore,
        external_provider_sessions: ExternalProviderSessionIndexStore,
        attached_provider_transcript_cursors: AttachedProviderTranscriptCursorStore,
        slice_store: crate::slice::SliceStore,
        session_projection: crate::runtime::projection::SessionStateProjectionStore,
        provider_run_projection: crate::runtime::projection::ProviderRunProjectionStore,
        operational_history_store: OperationalHistoryStore,
        durable_state_store: DurableKernelStateStore,
        provider_account_profiles: crate::account_profile::ProviderAccountProfileRegistry,
        prompt_state_owner: crate::runtime::prompt_state::PromptStateOwner,
        active_turns: ActiveTurnStore,
        prompt_activity: PromptActivityStore,
        prompt_workspace_claims: PromptWorkspaceClaimStore,
        structured_output_records: crate::app::provider_output::StructuredOutputRecordStore,
        pty_output_signal: crate::pty::PtyOutputSignal,
        terminal_stream: crate::terminal::TerminalStreamStore,
        workflow_design_events: WorkflowDesignEventStore,
        metaagent_events: MetaagentEventStore,
        metaagent_trace_subscriptions:
            crate::runtime::metaagent_trace::MetaagentTraceSubscriptionStore,
        workspace_coordinator: crate::runtime::workspace_coordinator::WorkspaceCoordinator,
    ) -> Self {
        let (
            completed_git_turn_snapshots,
            provider_process_projection,
            provider_launch_failure_retries,
            relay_state,
            legacy_workflow_history,
            agent_runtime_projection,
        ) = {
            let started = Instant::now();
            loop {
                if let Ok(app) = app.try_lock() {
                    break (
                        app.completed_git_turn_snapshot_store(),
                        app.provider_process_projection_store(),
                        app.provider_launch_failure_retry_store(),
                        app.relay_client_state(),
                        app.legacy_workflow_history_store(),
                        app.agent_runtime_projection_store(),
                    );
                }
                if started.elapsed() >= Duration::from_secs(5) {
                    panic!("KernelRuntimeState could not acquire the app lock during bootstrap");
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        };
        let workspace_live_sync_journal =
            match crate::git_observer::WorkspaceLiveSyncJournal::restore_from_durable_state(
                &durable_state_store,
            ) {
                Ok(journal) => journal,
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "daemon.workspace_live_sync",
                        "failed to restore workspace live sync journal",
                        serde_json::json!({
                            "error": error.to_string(),
                        }),
                    );
                    crate::git_observer::WorkspaceLiveSyncJournal::default()
                }
            };
        let publication_activation = Arc::new(publication_activation::PublicationActivation::new(
            config_projection
                .snapshot()
                .publication_control_state_root
                .is_some(),
        ));
        Self {
            app,
            provider_runtime_lanes,
            detached_workflow_provider_launches: Arc::new(std::sync::Mutex::new(BTreeSet::new())),
            owned: KernelRuntimeOwnedState {
                config_projection,
                session_store,
                agent_store,
                attachment_store,
                provider_store,
                pending_provider_launch_credentials: Arc::new(std::sync::Mutex::new(
                    BTreeMap::new(),
                )),
                workflow_provider_launch_lock: Arc::new(std::sync::Mutex::new(())),
                workflow_instance_provision_lock: Arc::new(std::sync::Mutex::new(())),
                publication_activation,
                provider_process_tracking,
                provider_launch_failure_retries,
                external_provider_sessions,
                attached_provider_transcript_cursors,
                slice_store,
                browser_controller_processes:
                    crate::runtime::browser_controller_process::BrowserControllerProcessStore::from_environment(),
                computer_input_executions:
                    crate::runtime::computer_input_execution::ComputerInputExecutionStore::default(),
                browser_controller_generations: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
                session_projection,
                agent_runtime_projection,
                provider_run_projection,
                provider_process_projection,
                operational_history_store,
                transcript_history_append_lock: Arc::new(std::sync::Mutex::new(())),
                event_connection_registry:
                    crate::event_connection::EventConnectionRegistry::new(
                        durable_state_store.clone(),
                    ),
                durable_state_store,
                legacy_workflow_history,
                provider_account_profiles,
                provider_login_processes: ProviderLoginProcessStore::default(),
                prompt_state_owner,
                active_turns,
                prompt_activity,
                provider_output_deadlines: ProviderOutputDeadlineStore::default(),
                prompt_workspace_claims,
                structured_output_records,
                pty_output_signal,
                terminal_stream,
                runtime_projection_changes: Arc::new(RuntimeChangeSignal::default()),
                workflow_design_events,
                workspace_coordinator,
                workspace_live_sync_coordinator: Arc::new(Mutex::new(
                    crate::io::ArtifactEditCoordinator::new(),
                )),
                workspace_live_sync_external_changes:
                    crate::io::ArtifactExternalChangeMonitor::default(),
                workspace_identity_monitor:
                    crate::runtime::workspace_identity_monitor::WorkspaceIdentityMonitor::default(),
                pending_agent_context_handoffs: PendingAgentContextHandoffStore::default(),
                pending_mcp_continuations: PendingMcpContinuationStore::shared(),
                metaagent_events,
                metaagent_trace_subscriptions,
                connector_adapter_processes: crate::connector::ConnectorAdapterProcessPool::default(
                ),
                pending_provider_reloads: PendingProviderReloadStore::default(),
                pending_interactions: PendingInteractionStore::shared(),
                git_turn_snapshots: crate::git_observer::GitTurnSnapshotStore::default(),
                completed_git_turn_snapshots,
                workspace_live_sync_journal,
                remote_workspace_live_sync_invocations: Arc::new(Mutex::new(BTreeMap::new())),
                remote_extension_invocations: Arc::new(Mutex::new(BTreeMap::new())),
                remote_extension_cancellations: Arc::new(Mutex::new(
                    std::collections::BTreeSet::new(),
                )),
                remote_home_extension_inflight: Arc::new(Mutex::new(BTreeMap::new())),
                remote_extension_manifest_retry_counts: Arc::new(Mutex::new(BTreeMap::new())),
                agent_message_idempotency: Arc::new(Mutex::new(
                    AgentMessageIdempotencyStore::default(),
                )),
                next_provider_process_gc_at_ms: Arc::new(AtomicU64::new(0)),
                relay_state,
                remote_prompt_projection_drains: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
                remote_prompt_recoveries: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
                slice_private_relay_connectors: Arc::new(Mutex::new(BTreeMap::new())),
                workflow_publication_runtimes:
                    crate::runtime::state::workflow_publication_runtime_lifecycle::WorkflowPublicationRuntimeProcessStore::default(),
            },
        }
    }

    pub(crate) async fn with_app_side_effect<R>(
        &self,
        operation: impl FnOnce(&mut DaemonApp) -> R,
    ) -> R {
        let mut app =
            crate::runtime::app_lock::lock_app_instrumented(&self.app, "kernel_runtime_state")
                .await;
        operation(&mut app)
    }

    pub(crate) fn provider_account_profile_registry(
        &self,
    ) -> &crate::account_profile::ProviderAccountProfileRegistry {
        &self.owned.provider_account_profiles
    }

    pub(crate) fn provider_account_authority_owner_user_id(
        &self,
        runtime_owner_user_id: &str,
    ) -> String {
        crate::account_profile::provider_account_authority_owner_user_id(
            &self.owned.config_projection.snapshot(),
            runtime_owner_user_id,
        )
    }

    pub(in crate::runtime) fn provider_login_process_store(&self) -> &ProviderLoginProcessStore {
        &self.owned.provider_login_processes
    }

    fn try_with_app_side_effect<R>(
        &self,
        operation: impl FnOnce(&mut DaemonApp) -> R,
    ) -> Option<R> {
        let mut app = self.app.try_lock().ok()?;
        Some(operation(&mut app))
    }

    async fn append_agent_durable_event(
        &self,
        kind: &'static str,
        agent: &crate::agent::AgentInstance,
        capability_name: Option<&str>,
    ) -> Result<(), DaemonError> {
        let agent = agent.clone();
        let capability_name = capability_name.map(str::to_string);
        self.owned.durable_state_store.append_event(
            kind,
            Some(agent.id().to_string()),
            serde_json::json!({
                "agent": &agent,
                "capability_name": capability_name,
            }),
        )?;
        Ok(())
    }

    async fn append_session_durable_event(
        &self,
        kind: &'static str,
        session: &crate::session::RuntimeSession,
        reason: &'static str,
    ) -> Result<(), DaemonError> {
        if kind == "session.deleted" {
            self.owned
                .durable_state_store
                .persist_session_deleted(session, reason)?;
            return Ok(());
        }
        if kind == "session.updated" && reason == "workflow" {
            self.owned
                .persist_workflow_runtime_session(session.id(), reason)?;
            return Ok(());
        }
        let session = session.durable_runtime_snapshot();
        self.owned.durable_state_store.append_event(
            kind,
            Some(session.id().to_string()),
            serde_json::json!({
                "session": &session,
                "reason": reason,
            }),
        )?;
        Ok(())
    }

    pub(crate) async fn capability_context(
        &self,
        session_id: &str,
        attachment_id: &str,
        capability: &'static str,
    ) -> Result<CapabilityRuntimeSnapshot, DaemonError> {
        self.owned
            .capability_context(session_id, attachment_id, capability)
    }
}

#[derive(Debug, Default)]
struct RuntimeChangeSignal {
    sequence: AtomicU64,
    notify: Notify,
}

impl RuntimeChangeSignal {
    fn sequence(&self) -> u64 {
        self.sequence.load(Ordering::Acquire)
    }

    fn record_change(&self) {
        self.sequence.fetch_add(1, Ordering::AcqRel);
        self.notify.notify_waiters();
    }

    async fn wait_for_change_after(&self, sequence: u64) {
        if self.sequence() != sequence {
            return;
        }
        let notified = self.notify.notified();
        if self.sequence() != sequence {
            return;
        }
        notified.await;
    }
}

pub(crate) struct CapabilityRuntimeSnapshot {
    pub(crate) workspace_id: String,
    pub(crate) worktree_root: std::path::PathBuf,
    pub(crate) workspace_coordinator: crate::runtime::workspace_coordinator::WorkspaceCoordinator,
    pub(crate) operational_history_store: crate::history::OperationalHistoryStore,
    pub(crate) operational_artifact_root: std::path::PathBuf,
    pub(crate) operational_artifact_index_path: std::path::PathBuf,
    pub(crate) history_archive_enabled: bool,
}

#[cfg(test)]
mod runtime_change_signal_tests {
    use super::RuntimeChangeSignal;
    use std::time::Duration;

    #[tokio::test]
    async fn runtime_change_signal_wakes_waiters() {
        let signal = std::sync::Arc::new(RuntimeChangeSignal::default());
        let sequence = signal.sequence();
        let waiter = {
            let signal = std::sync::Arc::clone(&signal);
            tokio::spawn(async move {
                signal.wait_for_change_after(sequence).await;
            })
        };

        signal.record_change();

        tokio::time::timeout(Duration::from_millis(100), waiter)
            .await
            .expect("change waiter should wake")
            .expect("wait task should complete");
    }

    #[tokio::test]
    async fn runtime_change_signal_returns_when_sequence_already_changed() {
        let signal = RuntimeChangeSignal::default();
        let sequence = signal.sequence();
        signal.record_change();

        tokio::time::timeout(
            Duration::from_millis(100),
            signal.wait_for_change_after(sequence),
        )
        .await
        .expect("changed sequence should not wait");
    }
}

#[cfg(test)]
mod workspace_live_sync_external_change_notice_tests;
