use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::config::DaemonConfig;
use crate::error::DaemonError;
use jsonschema::JSONSchema;
use serde_json::Value;

use super::types::{
    WorkflowIntermediateOutput, WorkflowRunOutputSubmission, WorkflowTurnSubmissionKind,
};
use super::{
    unix_epoch_ms, AgentPromptSchedule, AgentPromptScheduleDispatch, AgentPromptScheduleKind,
    CanonicalViewport, CollaborationLevel, CreateSessionRequest, DurableWorkflowHotState,
    EnvironmentError, PromptDetachEffect, PromptQueueItem, RoomEnvironmentRegistry,
    RoomEnvironmentSnapshot, RuntimeProject, RuntimeProjectKind, RuntimeProjectStatus,
    RuntimeSession, SessionConfigState, SessionInvite, SessionMember, SessionProjectSelection,
    SessionStatus, SessionStore, WorkflowCompletionSnapshot, WorkflowConsole, WorkflowConsoleEntry,
    WorkflowDefinition, WorkflowEdgeDefinition, WorkflowEndpointDefinition, WorkflowFailureEvent,
    WorkflowFailureKind, WorkflowHandoffPayload, WorkflowHandoffValidationPolicy, WorkflowMessage,
    WorkflowNodeDefinition, WorkflowNodeRun, WorkflowNodeRunStatus, WorkflowOutputPayload,
    WorkflowPromptQueueDefinition, WorkflowPublicationDefinition, WorkflowQueuedPrompt,
    WorkflowQueuedPromptSource, WorkflowQueuedPromptStatus, WorkflowRun, WorkflowRunStatus,
    WorkflowRuntimeToolCallEvent, WorkflowScheduleDefinition, WorkflowScheduleOverlapPolicy,
    WorkflowScheduleTrigger, WorkflowSchemaDefinition, WorkflowTurnEnvelope,
    WorkflowTurnRuntimeState, WorkflowWatchdogDefinition, WorkflowWatchdogPolicy,
    WorkspaceLinkAttachment, WorkspaceLinkDefinition, DEFAULT_LOCAL_USER_ID,
};
#[cfg(test)]
use super::{PromptAttachment, PromptSubmissionOutcome};

const PROMPT_ID_RESERVATION_BLOCK: u64 = 4_096;

#[derive(Debug, Clone)]
pub struct PromptIdAllocator {
    inner: Arc<PromptIdAllocatorInner>,
}

#[derive(Debug)]
struct PromptIdAllocatorInner {
    state: Mutex<PromptIdAllocatorState>,
    counter_path: Option<PathBuf>,
}

#[derive(Debug)]
struct PromptIdAllocatorState {
    last_issued: u64,
    reserved_until: u64,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct DurablePromptIdCounter {
    high_water_prompt_id: u64,
}

impl Default for PromptIdAllocator {
    fn default() -> Self {
        Self {
            inner: Arc::new(PromptIdAllocatorInner {
                state: Mutex::new(PromptIdAllocatorState {
                    last_issued: 0,
                    reserved_until: u64::MAX,
                }),
                counter_path: None,
            }),
        }
    }
}

impl PromptIdAllocator {
    pub(crate) fn persistent(counter_path: PathBuf) -> Self {
        Self {
            inner: Arc::new(PromptIdAllocatorInner {
                state: Mutex::new(PromptIdAllocatorState {
                    last_issued: 0,
                    reserved_until: 0,
                }),
                counter_path: Some(counter_path),
            }),
        }
    }

    pub(crate) fn next_prompt_id(&self) -> String {
        let mut state = self
            .inner
            .state
            .lock()
            .expect("prompt id allocator lock poisoned");
        if state.last_issued >= state.reserved_until {
            if let Some(counter_path) = self.inner.counter_path.as_deref() {
                match reserve_prompt_id_block(counter_path, state.last_issued) {
                    Ok((first, reserved_until)) => {
                        state.last_issued = first.saturating_sub(1);
                        state.reserved_until = reserved_until;
                    }
                    Err(error) => {
                        let fallback = super::unix_epoch_ms()
                            .saturating_mul(1_000_000)
                            .max(state.last_issued.saturating_add(1));
                        crate::logging::warn_with_fields(
                            "durable_state.prompt_id",
                            "failed to reserve durable prompt id block; using process-local high-water fallback",
                            serde_json::json!({
                                "counter_path": counter_path.display().to_string(),
                                "error": error.to_string(),
                                "fallback_prompt_number": fallback,
                            }),
                        );
                        state.last_issued = fallback.saturating_sub(1);
                        state.reserved_until = u64::MAX;
                    }
                }
            }
        }
        state.last_issued = state.last_issued.saturating_add(1);
        let next = state.last_issued;
        format!("prompt-{next}")
    }

    #[cfg(test)]
    pub(crate) fn observe_prompt_id(&self, prompt_id: &str) {
        if let Some(number) = prompt_id_number(prompt_id) {
            self.advance_to_at_least(number);
        }
    }

    #[cfg(test)]
    fn advance_to_at_least(&self, number: u64) {
        let mut state = self
            .inner
            .state
            .lock()
            .expect("prompt id allocator lock poisoned");
        state.last_issued = state.last_issued.max(number);
    }
}

fn reserve_prompt_id_block(path: &Path, minimum: u64) -> std::io::Result<(u64, u64)> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let lock_path = path.with_extension("lock");
    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(lock_path)?;
    lock_file_exclusive(&lock_file)?;
    let current = match std::fs::read(path) {
        Ok(payload) => serde_json::from_slice::<DurablePromptIdCounter>(&payload)
            .map(|counter| counter.high_water_prompt_id)
            .map_err(std::io::Error::other)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error),
    };
    let baseline = current.max(minimum).max(super::unix_epoch_ms());
    let reserved_until = baseline
        .checked_add(PROMPT_ID_RESERVATION_BLOCK)
        .ok_or_else(|| std::io::Error::other("prompt id reservation overflow"))?;
    let payload = serde_json::to_vec(&DurablePromptIdCounter {
        high_water_prompt_id: reserved_until,
    })
    .map_err(std::io::Error::other)?;
    let temporary = path.with_extension(format!("json.tmp-{}", std::process::id()));
    {
        use std::io::Write;
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(&payload)?;
        file.sync_all()?;
    }
    std::fs::rename(&temporary, path)?;
    Ok((baseline.saturating_add(1), reserved_until))
}

#[cfg(unix)]
fn lock_file_exclusive(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;

    // `std::fs::File::lock` is not stable on the Rust 1.88 toolchain used by
    // CI. The kernel already depends on libc, and flock is released when the
    // file descriptor closes at the end of the reservation transaction.
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(unix))]
fn lock_file_exclusive(_file: &std::fs::File) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "durable prompt id allocation requires an exclusive file lock",
    ))
}

#[cfg(test)]
fn prompt_id_number(prompt_id: &str) -> Option<u64> {
    prompt_id.strip_prefix("prompt-")?.parse::<u64>().ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowDispatch {
    pub node_run: WorkflowNodeRun,
    pub messages: Vec<WorkflowMessage>,
    pub endpoint_prompt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowCompletionUpdate {
    pub workflow_run: WorkflowRun,
    pub dispatches: Vec<WorkflowDispatch>,
    pub validation_warnings: Vec<WorkflowHandoffValidationWarning>,
    pub handoff_validation_failure: Option<WorkflowHandoffValidationFailure>,
    pub missing_output_failure: Option<WorkflowMissingOutputFailure>,
    pub run_output_validation_failure: Option<WorkflowRunOutputValidationFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowHandoffValidationWarning {
    pub edge_id: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowHandoffValidationFailure {
    pub edge_id: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowRunOutputValidationFailure {
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowMissingOutputFailure {
    pub message: String,
}

#[derive(Debug, Clone)]
struct WorkflowCompletionContext {
    workflow_run: WorkflowRun,
    source_node_run: WorkflowNodeRun,
    workflow: WorkflowDefinition,
}

#[derive(Debug, Clone)]
struct PendingWorkflowTurnOutputs {
    intermediate: Option<WorkflowRunOutputSubmission>,
    final_output: Option<WorkflowRunOutputSubmission>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowWatchdogTickPlan {
    pub watchdog_id: String,
    pub session_id: String,
    pub workflow_id: String,
    pub endpoint_id: String,
    pub queue_id: Option<String>,
    pub invocation_prompt: String,
    pub enqueue_prompt: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkflowWatchdogCollection {
    pub plans: Vec<WorkflowWatchdogTickPlan>,
    pub changed_session_ids: BTreeSet<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentPromptScheduleCollection {
    pub dispatches: Vec<AgentPromptScheduleDispatch>,
}

#[derive(Debug, Clone)]
pub struct SessionService {
    store: SessionStore,
    room_environments: RoomEnvironmentRegistry,
    projects: BTreeMap<String, RuntimeProject>,
    ephemeral_session_ids: BTreeSet<String>,
    host_machine_id: String,
    host_daemon_id: String,
    event_environment_id: String,
    prompt_id_allocator: PromptIdAllocator,
    next_workflow_number: u64,
    next_workflow_schema_number: u64,
    next_workflow_endpoint_number: u64,
    next_workflow_node_number: u64,
    next_workflow_edge_number: u64,
    next_workflow_node_run_number: u64,
    next_workflow_message_number: u64,
    next_workflow_watchdog_number: u64,
    next_workflow_publication_number: u64,
    next_workflow_event_binding_number: u64,
    next_workflow_prompt_queue_number: u64,
    next_workflow_queued_prompt_number: u64,
    next_agent_prompt_schedule_number: u64,
    max_workflow_queues_per_workflow: usize,
    session_default_max_agents: i32,
    workflow_default_max_concurrent: u32,
    next_workspace_link_number: u64,
}

mod core;
mod helpers;
mod launches;
mod prompt_schedules;
mod room_environments;
mod sessions;
#[cfg(test)]
mod tests;
mod turn_completion;
mod turns;
mod watchdogs;
mod workflow_code;
mod workflow_defs;

pub use helpers::classify_workflow_failure_kind;
use helpers::{
    collect_ready_workflow_dispatches, describe_session_match, normalize_session_alias,
    normalize_workflow_alias, normalize_workflow_endpoint_alias,
    normalize_workflow_publication_alias, normalize_workflow_queue_alias,
    validate_workflow_edge_handoff,
};
