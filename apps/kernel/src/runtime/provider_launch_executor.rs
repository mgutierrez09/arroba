use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use futures_util::StreamExt;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

use crate::app::DaemonApp;
use crate::app::StartedProviderLaunch;
use crate::error::DaemonError;
use crate::local::{
    BatchOperationFailure, LaunchProviderRunRequest, LaunchProviderRunsRequest,
    LocalDaemonResponse, ProviderRunBatchLaunchResult,
};
use crate::provider::{ProviderProcessService, ProviderRunState};
use crate::runtime::command::{command_caller_user_id, KernelCommand};
use crate::runtime::command_latency::{
    log_provider_launch_accepted, log_provider_runtime_binding_failed,
    log_provider_runtime_binding_started, log_provider_runtime_binding_succeeded, CommandTrace,
};
use crate::runtime::projection::{ProviderRunProjectionStore, SessionStateProjectionStore};
use crate::runtime::state::{KernelRuntimeState, ProviderLaunchStartOutcome};

const DEFAULT_PROVIDER_BATCH_LAUNCH_CONCURRENCY: usize = 8;
const DEFAULT_PROVIDER_BATCH_LAUNCH_CONCURRENCY_PER_SESSION: usize = 8;
const DEFAULT_PROVIDER_BATCH_LAUNCH_PROVIDER_LIMIT: usize = 16;

#[derive(Clone)]
pub(crate) struct ProviderLaunchCommandExecutor {
    store: ProviderLaunchStore,
}

#[derive(Clone)]
pub(crate) struct ProviderLaunchStore {
    state: KernelRuntimeState,
}

#[derive(Clone, Default)]
pub(crate) struct ProviderLaunchPendingTracker {
    sessions: Arc<Mutex<HashSet<String>>>,
}

pub(crate) async fn execute_provider_launch_command(
    runtime_state: &KernelRuntimeState,
    command: &KernelCommand,
    request: LaunchProviderRunRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let command_trace = CommandTrace::from_command(command);
    ProviderLaunchCommandExecutor::new(runtime_state.clone())
        .execute(request, command_caller_user_id(command), command_trace)
        .await
}

pub(crate) async fn execute_provider_batch_launch_command(
    runtime_state: &KernelRuntimeState,
    command: &KernelCommand,
    request: LaunchProviderRunsRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    ProviderLaunchCommandExecutor::new(runtime_state.clone())
        .execute_batch(
            request,
            command_caller_user_id(command),
            CommandTrace::from_command(command),
        )
        .await
}

impl ProviderLaunchCommandExecutor {
    pub(crate) fn new(state: KernelRuntimeState) -> Self {
        Self {
            store: ProviderLaunchStore::new(state),
        }
    }

    pub(crate) async fn execute(
        &self,
        request: LaunchProviderRunRequest,
        caller_user_id: String,
        command_trace: CommandTrace,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        if let Some(response) = self
            .store
            .launch_remote_native_provider_run(&request, &caller_user_id)
            .await?
        {
            return Ok(response);
        }
        let launch_started_at_ms = crate::runtime::command_latency::now_ms();
        let start_outcome = self.store.start_launch(request, caller_user_id).await?;
        let (started, runtime_init_delay_ms) = match start_outcome {
            ProviderLaunchStartOutcome::Reused(provider_run) => {
                return Ok(LocalDaemonResponse::ProviderRunLaunched { provider_run });
            }
            ProviderLaunchStartOutcome::Started(started, runtime_init_delay_ms) => {
                (started, runtime_init_delay_ms)
            }
        };
        let accepted = started.run.clone();
        log_provider_launch_accepted(
            &command_trace,
            &accepted,
            launch_started_at_ms,
            runtime_init_delay_ms,
        );
        let store = self.store.clone();
        tokio::spawn(async move {
            if runtime_init_delay_ms > 0 {
                sleep(Duration::from_millis(runtime_init_delay_ms)).await;
            }
            let binding_started_at_ms = log_provider_runtime_binding_started(
                &command_trace,
                &started.run,
                launch_started_at_ms,
            );
            let run = started.run.clone();
            let provider_credential_env = started.provider_credential_env.clone();
            let binding = tokio::task::spawn_blocking(move || {
                ProviderProcessService::initialize_runtime_binding_with_credentials(
                    &run,
                    &provider_credential_env,
                )
            })
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "initialize provider runtime",
                message: error.to_string(),
            });

            match binding {
                Ok(Ok(binding)) => {
                    log_provider_runtime_binding_succeeded(
                        &command_trace,
                        &started.run,
                        launch_started_at_ms,
                        binding_started_at_ms,
                    );
                    store.finish_launch(&started, binding).await;
                }
                Ok(Err(error)) => {
                    log_provider_runtime_binding_failed(
                        &command_trace,
                        &started.run,
                        launch_started_at_ms,
                        binding_started_at_ms,
                        &error,
                    );
                    store.fail_launch(&started, &error).await;
                }
                Err(error) => {
                    log_provider_runtime_binding_failed(
                        &command_trace,
                        &started.run,
                        launch_started_at_ms,
                        binding_started_at_ms,
                        &error,
                    );
                    store.fail_launch(&started, &error).await;
                }
            }
        });
        Ok(LocalDaemonResponse::ProviderRunLaunchAccepted {
            provider_run: accepted,
        })
    }

    pub(crate) async fn execute_batch(
        &self,
        request: LaunchProviderRunsRequest,
        caller_user_id: String,
        command_trace: CommandTrace,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        if request.launches.is_empty() {
            return Ok(LocalDaemonResponse::ProviderRunsLaunchAccepted {
                provider_runs: Vec::new(),
                failures: Vec::new(),
            });
        }
        let max_concurrency =
            provider_batch_launch_effective_concurrency(request.max_concurrency, &request.launches);
        let mut launches = request.launches;
        if let Err(error) = self
            .store
            .normalize_batch_launch_targets(&mut launches)
            .await
        {
            return Ok(LocalDaemonResponse::ProviderRunsLaunchAccepted {
                provider_runs: Vec::new(),
                failures: provider_batch_failures(&launches, error.to_string()),
            });
        }
        if let Some(failures) = provider_batch_target_preflight_failures(&launches) {
            return Ok(LocalDaemonResponse::ProviderRunsLaunchAccepted {
                provider_runs: Vec::new(),
                failures,
            });
        }

        let mut outcomes =
            futures_util::stream::iter(interleave_batch_launches_by_session(launches))
                .map(|(index, launch_request)| {
                    let executor = self.clone();
                    let caller_user_id = caller_user_id.clone();
                    let command_trace = command_trace.clone();
                    async move {
                        let agent_id = launch_request.agent_id.clone();
                        let result = executor
                            .execute(launch_request, caller_user_id, command_trace)
                            .await;
                        (index, agent_id, result)
                    }
                })
                .buffer_unordered(max_concurrency)
                .collect::<Vec<_>>()
                .await;
        outcomes.sort_by_key(|(index, _, _)| *index);

        let mut provider_runs = Vec::new();
        let mut failures = Vec::new();
        for (index, agent_id, result) in outcomes {
            match result {
                Ok(LocalDaemonResponse::ProviderRunLaunched { provider_run }) => {
                    provider_runs.push(ProviderRunBatchLaunchResult {
                        index,
                        agent_id,
                        provider_run,
                        reused: true,
                    });
                }
                Ok(LocalDaemonResponse::ProviderRunLaunchAccepted { provider_run }) => {
                    provider_runs.push(ProviderRunBatchLaunchResult {
                        index,
                        agent_id,
                        provider_run,
                        reused: false,
                    });
                }
                Ok(other) => failures.push(BatchOperationFailure {
                    index,
                    agent_id,
                    message: format!("unexpected launch response: {other:?}"),
                }),
                Err(error) => failures.push(BatchOperationFailure {
                    index,
                    agent_id,
                    message: error.to_string(),
                }),
            }
        }

        Ok(LocalDaemonResponse::ProviderRunsLaunchAccepted {
            provider_runs,
            failures,
        })
    }
}

fn provider_batch_target_preflight_failures(
    launches: &[LaunchProviderRunRequest],
) -> Option<Vec<BatchOperationFailure>> {
    let mut seen_targets = HashSet::new();
    let duplicate_target = launches.iter().any(|launch| {
        let target = launch.agent_id.as_deref().unwrap_or("__focused_agent__");
        !seen_targets.insert((launch.session_id.as_str(), target))
    });
    if duplicate_target {
        return Some(
            launches
                .iter()
                .enumerate()
                .map(|(index, launch)| BatchOperationFailure {
                    index,
                    agent_id: launch.agent_id.clone(),
                    message: "provider batch launch contains duplicate target agents".to_string(),
                })
                .collect(),
        );
    }

    None
}

fn provider_batch_launch_effective_concurrency(
    requested: Option<usize>,
    launches: &[LaunchProviderRunRequest],
) -> usize {
    if launches.is_empty() {
        return 0;
    }
    let requested = requested.unwrap_or(DEFAULT_PROVIDER_BATCH_LAUNCH_CONCURRENCY);
    let provider_limit = launches
        .iter()
        .map(provider_batch_launch_concurrency_limit)
        .min()
        .unwrap_or(DEFAULT_PROVIDER_BATCH_LAUNCH_PROVIDER_LIMIT)
        .max(1);
    let session_limit = launches
        .iter()
        .map(|launch| launch.session_id.as_str())
        .collect::<HashSet<_>>()
        .len()
        .saturating_mul(DEFAULT_PROVIDER_BATCH_LAUNCH_CONCURRENCY_PER_SESSION)
        .max(1);
    requested
        .clamp(1, launches.len())
        .min(provider_limit)
        .min(session_limit)
}

fn provider_batch_launch_concurrency_limit(launch: &LaunchProviderRunRequest) -> usize {
    crate::provider::provider_batch_launch_concurrency_limit(
        &launch.adapter_key,
        &launch.provider,
        DEFAULT_PROVIDER_BATCH_LAUNCH_PROVIDER_LIMIT,
    )
}

fn interleave_batch_launches_by_session(
    launches: Vec<LaunchProviderRunRequest>,
) -> Vec<(usize, LaunchProviderRunRequest)> {
    let mut session_order = Vec::new();
    let mut launches_by_session: HashMap<String, VecDeque<(usize, LaunchProviderRunRequest)>> =
        HashMap::new();
    for (index, launch) in launches.into_iter().enumerate() {
        if !launches_by_session.contains_key(&launch.session_id) {
            session_order.push(launch.session_id.clone());
        }
        launches_by_session
            .entry(launch.session_id.clone())
            .or_default()
            .push_back((index, launch));
    }

    let mut interleaved = Vec::new();
    loop {
        let mut advanced = false;
        for session_id in &session_order {
            let Some(queue) = launches_by_session.get_mut(session_id) else {
                continue;
            };
            let Some(launch) = queue.pop_front() else {
                continue;
            };
            interleaved.push(launch);
            advanced = true;
        }
        if !advanced {
            break;
        }
    }
    interleaved
}

fn provider_batch_failures(
    launches: &[LaunchProviderRunRequest],
    message: String,
) -> Vec<BatchOperationFailure> {
    launches
        .iter()
        .enumerate()
        .map(|(index, launch)| BatchOperationFailure {
            index,
            agent_id: launch.agent_id.clone(),
            message: message.clone(),
        })
        .collect()
}

impl ProviderLaunchStore {
    pub(crate) fn new(state: KernelRuntimeState) -> Self {
        Self { state }
    }

    async fn normalize_batch_launch_targets(
        &self,
        launches: &mut [LaunchProviderRunRequest],
    ) -> Result<(), DaemonError> {
        if !launches.iter().any(|launch| launch.agent_id.is_none()) {
            return Ok(());
        }
        let mut focused_agents_by_session = HashMap::new();
        for launch in launches.iter().filter(|launch| launch.agent_id.is_none()) {
            if focused_agents_by_session.contains_key(&launch.session_id) {
                continue;
            }
            focused_agents_by_session.insert(
                launch.session_id.clone(),
                self.state.focused_agent_id(&launch.session_id).await?,
            );
        }
        for launch in launches {
            if launch.agent_id.is_none() {
                launch.agent_id = focused_agents_by_session
                    .get(&launch.session_id)
                    .cloned()
                    .flatten();
            }
        }
        Ok(())
    }

    async fn start_launch(
        &self,
        request: LaunchProviderRunRequest,
        caller_user_id: String,
    ) -> Result<ProviderLaunchStartOutcome, DaemonError> {
        self.state
            .start_provider_launch(request, caller_user_id)
            .await
    }

    async fn launch_remote_native_provider_run(
        &self,
        request: &LaunchProviderRunRequest,
        caller_user_id: &str,
    ) -> Result<Option<LocalDaemonResponse>, DaemonError> {
        self.state
            .launch_remote_native_provider_run(request, caller_user_id)
            .await
    }

    async fn finish_launch(
        &self,
        started: &StartedProviderLaunch,
        binding: Option<crate::provider::ProviderRuntimeBinding>,
    ) {
        self.state.finish_provider_launch(started, binding).await;
    }

    async fn fail_launch(&self, started: &StartedProviderLaunch, error: &DaemonError) {
        self.state.fail_provider_launch(started, error).await;
    }
}

impl ProviderLaunchPendingTracker {
    pub(crate) async fn track_response(&self, result: &Result<LocalDaemonResponse, DaemonError>) {
        match result {
            Ok(LocalDaemonResponse::ProviderRunLaunchAccepted { provider_run }) => {
                self.sessions
                    .lock()
                    .await
                    .insert(provider_run.session_id().to_string());
            }
            Ok(LocalDaemonResponse::ProviderRunsLaunchAccepted { provider_runs, .. }) => {
                let mut sessions = self.sessions.lock().await;
                for launched in provider_runs {
                    sessions.insert(launched.provider_run.session_id().to_string());
                }
            }
            _ => {}
        }
    }

    pub(crate) async fn has_unsettled_launch(
        &self,
        session_id: &str,
        session_projection: &SessionStateProjectionStore,
        provider_run_projection: &ProviderRunProjectionStore,
    ) -> bool {
        if !self.sessions.lock().await.contains(session_id) {
            return false;
        }
        if let Some(is_starting) = provider_launch_is_still_starting_from_projection(
            session_id,
            session_projection,
            provider_run_projection,
        ) {
            if !is_starting {
                self.sessions.lock().await.remove(session_id);
            }
            return is_starting;
        }
        true
    }

    pub(crate) async fn clear_if_settled(
        &self,
        app: &Arc<Mutex<DaemonApp>>,
        session_id: &str,
        session_projection: &SessionStateProjectionStore,
        provider_run_projection: &ProviderRunProjectionStore,
    ) {
        if !self.sessions.lock().await.contains(session_id) {
            return;
        }
        if let Some(is_starting) = provider_launch_is_still_starting_from_projection(
            session_id,
            session_projection,
            provider_run_projection,
        ) {
            if !is_starting {
                self.sessions.lock().await.remove(session_id);
            }
            return;
        }
        let Ok(app) = app.try_lock() else {
            return;
        };
        let is_still_starting = app
            .sessions()
            .get_session(session_id)
            .ok()
            .and_then(|session| session.active_provider_run_id().map(str::to_string))
            .and_then(|provider_run_id| app.providers().get_run(&provider_run_id).ok())
            .is_some_and(|run| run.state() == ProviderRunState::Starting);
        if !is_still_starting {
            self.sessions.lock().await.remove(session_id);
        }
    }

    #[cfg(test)]
    pub(crate) async fn insert_for_tests(&self, session_id: impl Into<String>) {
        self.sessions.lock().await.insert(session_id.into());
    }

    #[cfg(test)]
    pub(crate) async fn contains_for_tests(&self, session_id: &str) -> bool {
        self.sessions.lock().await.contains(session_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_launches_are_interleaved_by_session_without_losing_original_indexes() {
        let launches = vec![
            test_launch("session-a", "agent-a-1"),
            test_launch("session-a", "agent-a-2"),
            test_launch("session-a", "agent-a-3"),
            test_launch("session-b", "agent-b-1"),
            test_launch("session-b", "agent-b-2"),
        ];

        let interleaved = interleave_batch_launches_by_session(launches);

        let order = interleaved
            .iter()
            .map(|(index, launch)| {
                (
                    *index,
                    launch.session_id.as_str(),
                    launch.agent_id.as_deref(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            order,
            vec![
                (0, "session-a", Some("agent-a-1")),
                (3, "session-b", Some("agent-b-1")),
                (1, "session-a", Some("agent-a-2")),
                (4, "session-b", Some("agent-b-2")),
                (2, "session-a", Some("agent-a-3")),
            ]
        );
    }

    #[test]
    fn batch_launch_concurrency_respects_provider_caps() {
        let codex_launches = (0..64)
            .map(|index| {
                let mut launch =
                    test_launch(&format!("session-{index}"), &format!("agent-{index}"));
                launch.adapter_key = "codex".to_string();
                launch.provider = "codex".to_string();
                launch
            })
            .collect::<Vec<_>>();
        assert_eq!(
            provider_batch_launch_effective_concurrency(Some(50), &codex_launches),
            crate::provider::provider_batch_launch_concurrency_limit("codex", "codex", 50)
        );

        let dev_stub_launches = (0..64)
            .map(|index| {
                let mut launch =
                    test_launch(&format!("session-{index}"), &format!("stub-agent-{index}"));
                launch.adapter_key = "dev-stub".to_string();
                launch.provider = "dev-stub".to_string();
                launch
            })
            .collect::<Vec<_>>();
        assert_eq!(
            provider_batch_launch_effective_concurrency(Some(50), &dev_stub_launches),
            50
        );
    }

    #[test]
    fn batch_launch_concurrency_uses_smallest_provider_cap_for_mixed_batches() {
        let mut launches = (0..40)
            .map(|index| {
                let mut launch =
                    test_launch(&format!("session-{index}"), &format!("agent-{index}"));
                launch.adapter_key = "codex".to_string();
                launch.provider = "codex".to_string();
                launch
            })
            .collect::<Vec<_>>();
        launches[0].adapter_key = "dev-stub".to_string();
        launches[0].provider = "dev-stub".to_string();

        assert_eq!(
            provider_batch_launch_effective_concurrency(Some(40), &launches),
            crate::provider::provider_batch_launch_concurrency_limit("codex", "codex", 40)
        );
    }

    #[test]
    fn batch_launch_concurrency_caps_to_per_session_budget() {
        let single_session_launches = (0..64)
            .map(|index| test_launch("session-a", &format!("agent-{index}")))
            .collect::<Vec<_>>();
        assert_eq!(
            provider_batch_launch_effective_concurrency(Some(50), &single_session_launches),
            DEFAULT_PROVIDER_BATCH_LAUNCH_CONCURRENCY_PER_SESSION
        );

        let mixed_session_launches = vec![
            test_launch("session-a", "agent-a-1"),
            test_launch("session-a", "agent-a-2"),
            test_launch("session-b", "agent-b-1"),
            test_launch("session-b", "agent-b-2"),
            test_launch("session-c", "agent-c-1"),
        ];
        assert_eq!(
            provider_batch_launch_effective_concurrency(Some(50), &mixed_session_launches),
            5
        );

        let large_mixed_session_launches = (0..64)
            .map(|index| {
                let session_index = index % 3;
                test_launch(
                    &format!("session-{session_index}"),
                    &format!("agent-{index}"),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            provider_batch_launch_effective_concurrency(Some(50), &large_mixed_session_launches),
            3 * DEFAULT_PROVIDER_BATCH_LAUNCH_CONCURRENCY_PER_SESSION
        );
    }

    fn test_launch(session_id: &str, agent_id: &str) -> LaunchProviderRunRequest {
        LaunchProviderRunRequest {
            session_id: session_id.to_string(),
            agent_id: Some(agent_id.to_string()),
            adapter_key: "dev-stub".to_string(),
            provider: "claude-code".to_string(),
            account_profile: "default".to_string(),
            model: "sonnet".to_string(),
            variant: None,
            structured_endpoint: None,
            provider_session_id: None,
            native_tui: false,
        }
    }
}

fn provider_launch_is_still_starting_from_projection(
    session_id: &str,
    session_projection: &SessionStateProjectionStore,
    provider_run_projection: &ProviderRunProjectionStore,
) -> Option<bool> {
    let session = session_projection.get(session_id)?;
    let Some(provider_run_id) = session.active_provider_run_id() else {
        return Some(false);
    };
    let run = provider_run_projection.get(provider_run_id)?;
    Some(run.state() == ProviderRunState::Starting)
}
