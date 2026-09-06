use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::error::DaemonError;

use super::StartedProviderLaunch;

pub(crate) const MAX_PROVIDER_LAUNCH_FAILURE_RETRY_ATTEMPTS: u32 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderLaunchFailureRetryScheduleOutcome {
    Scheduled,
    AlreadyScheduled,
    Exhausted,
}

#[derive(Debug, Clone)]
pub(crate) struct ProviderLaunchFailureRetry {
    pub(crate) started: StartedProviderLaunch,
    provider_run_id: String,
    operation: &'static str,
    message: String,
    attempt: u32,
    due_at_ms: u64,
}

impl ProviderLaunchFailureRetry {
    pub(crate) fn error(&self) -> DaemonError {
        DaemonError::ProviderProtocol {
            provider_run_id: self.provider_run_id.clone(),
            operation: self.operation,
            message: self.message.clone(),
        }
    }

    pub(crate) fn attempt(&self) -> u32 {
        self.attempt
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ProviderLaunchFailureRetryStore {
    inner: Arc<Mutex<BTreeMap<String, ProviderLaunchFailureRetry>>>,
}

impl ProviderLaunchFailureRetryStore {
    pub(crate) fn schedule_initial(
        &self,
        started: &StartedProviderLaunch,
        error: &DaemonError,
        now_ms: u64,
    ) -> bool {
        let DaemonError::ProviderProtocol {
            provider_run_id,
            operation,
            message,
        } = error
        else {
            return false;
        };
        self.schedule_if_absent(ProviderLaunchFailureRetry {
            started: started.clone(),
            provider_run_id: provider_run_id.clone(),
            operation,
            message: message.clone(),
            attempt: 1,
            due_at_ms: retry_due_at_ms(now_ms, 1),
        })
    }

    pub(crate) fn reschedule(
        &self,
        mut retry: ProviderLaunchFailureRetry,
        now_ms: u64,
    ) -> ProviderLaunchFailureRetryScheduleOutcome {
        if retry.attempt >= MAX_PROVIDER_LAUNCH_FAILURE_RETRY_ATTEMPTS {
            return ProviderLaunchFailureRetryScheduleOutcome::Exhausted;
        }
        retry.attempt = retry.attempt.saturating_add(1);
        retry.due_at_ms = retry_due_at_ms(now_ms, retry.attempt);
        if self.schedule_if_absent(retry) {
            ProviderLaunchFailureRetryScheduleOutcome::Scheduled
        } else {
            ProviderLaunchFailureRetryScheduleOutcome::AlreadyScheduled
        }
    }

    fn schedule_if_absent(&self, retry: ProviderLaunchFailureRetry) -> bool {
        let provider_run_id = retry.started.run.id().to_string();
        let mut retries = self
            .inner
            .lock()
            .expect("provider launch failure retry store poisoned");
        if retries.contains_key(&provider_run_id) {
            return false;
        }
        retries.insert(provider_run_id, retry);
        true
    }

    pub(crate) fn due_provider_run_ids(&self, now_ms: u64) -> Vec<String> {
        self.inner
            .lock()
            .expect("provider launch failure retry store poisoned")
            .iter()
            .filter(|(_, retry)| retry.due_at_ms <= now_ms)
            .map(|(provider_run_id, _)| provider_run_id.clone())
            .collect()
    }

    pub(crate) fn take_due_for(
        &self,
        provider_run_id: &str,
        now_ms: u64,
    ) -> Option<ProviderLaunchFailureRetry> {
        let mut retries = self
            .inner
            .lock()
            .expect("provider launch failure retry store poisoned");
        retries
            .get(provider_run_id)
            .filter(|retry| retry.due_at_ms <= now_ms)?;
        retries.remove(provider_run_id)
    }

    pub(crate) fn next_due_at_ms(&self) -> Option<u64> {
        self.inner
            .lock()
            .expect("provider launch failure retry store poisoned")
            .values()
            .map(|retry| retry.due_at_ms)
            .min()
    }

    pub(crate) fn clear(&self, provider_run_id: &str) {
        self.inner
            .lock()
            .expect("provider launch failure retry store poisoned")
            .remove(provider_run_id);
    }
}

fn retry_due_at_ms(now_ms: u64, attempt: u32) -> u64 {
    now_ms.saturating_add(crate::durable_state::durable_settlement_retry_delay_ms(
        attempt,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{
        AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
    };

    fn started_provider_launch() -> StartedProviderLaunch {
        let request =
            LaunchProviderRequest::new("session-retry", "codex", "codex", "default", "gpt-5.5");
        StartedProviderLaunch {
            run: RuntimeProviderRun::new(
                "provider-run-retry",
                &request,
                ProviderLaunchResult {
                    endpoint_mode: AgentEndpointMode::External,
                    process_label: "test-codex".to_string(),
                    pty_target: None,
                    pty_program: None,
                    pty_args: Vec::new(),
                    pty_env: Default::default(),
                    pty_env_remove: Vec::new(),
                    working_directory: None,
                    structured_endpoint: Some("test-codex-runtime".to_string()),
                },
            ),
            previous_active_run_id: None,
            provider_credential_env: Default::default(),
        }
    }

    fn resume_error() -> DaemonError {
        DaemonError::ProviderProtocol {
            provider_run_id: "provider-run-retry".to_string(),
            operation: "thread/resume",
            message: "no rollout found for thread".to_string(),
        }
    }

    #[test]
    fn retry_store_deduplicates_and_uses_capped_backoff() {
        let store = ProviderLaunchFailureRetryStore::default();
        let started = started_provider_launch();
        let error = resume_error();

        assert!(store.schedule_initial(&started, &error, 1_000));
        assert!(!store.schedule_initial(&started, &error, 1_000));
        assert_eq!(store.next_due_at_ms(), Some(1_100));
        assert!(store.due_provider_run_ids(1_099).is_empty());

        let mut retry = store
            .take_due_for("provider-run-retry", 1_100)
            .expect("first retry should become due");
        assert_eq!(retry.attempt(), 1);
        for attempt in 2..=7 {
            assert_eq!(
                store.reschedule(retry, 10_000),
                ProviderLaunchFailureRetryScheduleOutcome::Scheduled
            );
            let due_at_ms =
                10_000 + crate::durable_state::durable_settlement_retry_delay_ms(attempt);
            assert_eq!(store.next_due_at_ms(), Some(due_at_ms));
            retry = store
                .take_due_for("provider-run-retry", due_at_ms)
                .expect("rescheduled retry should become due");
            assert_eq!(retry.attempt(), attempt);
        }
        assert_eq!(retry.due_at_ms, 15_000);
        assert!(matches!(
            retry.error(),
            DaemonError::ProviderProtocol {
                operation: "thread/resume",
                ..
            }
        ));
    }

    #[test]
    fn retry_store_retains_due_work_until_the_run_lane_claims_it() {
        let store = ProviderLaunchFailureRetryStore::default();
        let started = started_provider_launch();
        let error = resume_error();

        assert!(store.schedule_initial(&started, &error, 1_000));
        assert_eq!(
            store.due_provider_run_ids(1_100),
            vec!["provider-run-retry".to_string()]
        );
        assert!(
            !store.schedule_initial(&started, &error, 1_100),
            "enumerating due work must not open a duplicate-scheduling window"
        );
        assert!(store.take_due_for("provider-run-retry", 1_100).is_some());
    }

    #[test]
    fn retry_store_exhausts_after_the_bounded_attempt_budget() {
        let store = ProviderLaunchFailureRetryStore::default();
        let started = started_provider_launch();
        let error = resume_error();
        assert!(store.schedule_initial(&started, &error, 0));
        let mut retry = store
            .take_due_for("provider-run-retry", 100)
            .expect("initial retry should become due");
        for attempt in 2..=MAX_PROVIDER_LAUNCH_FAILURE_RETRY_ATTEMPTS {
            assert_eq!(
                store.reschedule(retry, 0),
                ProviderLaunchFailureRetryScheduleOutcome::Scheduled
            );
            retry = store
                .take_due_for(
                    "provider-run-retry",
                    crate::durable_state::durable_settlement_retry_delay_ms(attempt),
                )
                .expect("bounded retry should become due");
        }
        assert_eq!(retry.attempt(), MAX_PROVIDER_LAUNCH_FAILURE_RETRY_ATTEMPTS);
        assert_eq!(
            store.reschedule(retry, 0),
            ProviderLaunchFailureRetryScheduleOutcome::Exhausted
        );
        assert_eq!(store.next_due_at_ms(), None);
    }
}
