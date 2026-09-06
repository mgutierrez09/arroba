use super::*;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, VecDeque};
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Condvar;

use crate::transport::room_browser_controller::RoomBrowserControllerResult as Response;

const COMPLETED_BROWSER_ACTION_LIMIT: usize = 256;
type ExecutionKey = (String, String);
type ExecutionOutcome = Result<Response, String>;

#[derive(Default)]
pub(super) struct CancellationSignal {
    requested: AtomicBool,
    stopped: AtomicBool,
    fenced: AtomicBool,
}

impl CancellationSignal {
    pub(super) fn requested(&self) -> bool {
        self.requested.load(Ordering::Acquire)
    }
    pub(super) fn confirm_stop(&self) {
        self.stopped.store(true, Ordering::Release);
    }
    pub(super) fn confirm_fence(&self) {
        self.fenced.store(true, Ordering::Release);
        self.confirm_stop();
    }
    fn fenced(&self) -> bool {
        self.fenced.load(Ordering::Acquire)
    }
}

struct ExecutionRecord {
    fingerprint: [u8; 32],
    signal: Arc<CancellationSignal>,
    outcome: Mutex<Option<ExecutionOutcome>>,
    completed: Condvar,
}

struct CompletedExecution {
    key: ExecutionKey,
    fingerprint: [u8; 32],
    outcome: ExecutionOutcome,
}

#[derive(Default)]
struct ExecutionRegistryState {
    active: BTreeMap<ExecutionKey, Arc<ExecutionRecord>>,
    completed: VecDeque<CompletedExecution>,
}

#[derive(Clone, Default)]
pub(super) struct BrowserActionExecutions {
    state: Arc<Mutex<ExecutionRegistryState>>,
    #[cfg(test)]
    cancellation_requests: Arc<AtomicUsize>,
}

struct ActiveAction {
    registry: BrowserActionExecutions,
    key: ExecutionKey,
    record: Arc<ExecutionRecord>,
    signal: Arc<CancellationSignal>,
    finished: bool,
}

impl Drop for ActiveAction {
    fn drop(&mut self) {
        if !self.finished {
            self.registry.complete(
                &self.key,
                &self.record,
                Err("browser action execution ended without a terminal result".into()),
            );
        }
    }
}

enum ExecutionAdmission {
    Start(ActiveAction),
    Wait(Arc<ExecutionRecord>),
    Replay(ExecutionOutcome),
}

enum RecoveryAdmission {
    Wait(Arc<ExecutionRecord>),
    Replay(ExecutionOutcome),
}

impl ActiveAction {
    fn finish(mut self, outcome: ExecutionOutcome) -> ExecutionOutcome {
        self.registry
            .complete(&self.key, &self.record, outcome.clone());
        self.finished = true;
        outcome
    }
}

impl ExecutionRecord {
    fn wait(&self) -> ExecutionOutcome {
        let mut outcome = self
            .outcome
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while outcome.is_none() {
            outcome = self
                .completed
                .wait(outcome)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        outcome
            .clone()
            .expect("completed browser execution outcome")
    }
}

impl BrowserActionExecutions {
    #[cfg(test)]
    fn forget_completed(&self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .completed
            .clear();
    }

    #[cfg(test)]
    fn cancellation_request_count(&self) -> usize {
        self.cancellation_requests.load(Ordering::Acquire)
    }

    fn register(
        &self,
        session_id: &str,
        execution_id: &str,
        fingerprint: [u8; 32],
    ) -> Result<ExecutionAdmission, String> {
        if execution_id.len() != 32 || !execution_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("browser action requires a 128-bit execution identity".into());
        }
        let key = (session_id.to_string(), execution_id.to_string());
        let mut state = self
            .state
            .lock()
            .map_err(|_| "browser execution registry poisoned")?;
        if let Some(completed) = state.completed.iter().find(|entry| entry.key == key) {
            return if completed.fingerprint == fingerprint {
                Ok(ExecutionAdmission::Replay(completed.outcome.clone()))
            } else {
                Err("browser execution identity was reused for a different request".into())
            };
        }
        if let Some(active) = state.active.get(&key) {
            return if active.fingerprint == fingerprint {
                Ok(ExecutionAdmission::Wait(Arc::clone(active)))
            } else {
                Err("browser execution identity was reused for a different request".into())
            };
        }
        if state.active.len() >= 64 {
            return Err("browser execution capacity is exhausted".into());
        }
        let signal = Arc::new(CancellationSignal::default());
        let record = Arc::new(ExecutionRecord {
            fingerprint,
            signal: Arc::clone(&signal),
            outcome: Mutex::new(None),
            completed: Condvar::new(),
        });
        state.active.insert(key.clone(), Arc::clone(&record));
        Ok(ExecutionAdmission::Start(ActiveAction {
            registry: self.clone(),
            key,
            record,
            signal,
            finished: false,
        }))
    }

    fn recover(
        &self,
        session_id: &str,
        execution_id: &str,
        fingerprint: [u8; 32],
    ) -> Result<RecoveryAdmission, String> {
        if execution_id.len() != 32 || !execution_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("browser action recovery requires a 128-bit execution identity".into());
        }
        let key = (session_id.to_string(), execution_id.to_string());
        let state = self
            .state
            .lock()
            .map_err(|_| "browser execution registry poisoned")?;
        if let Some(completed) = state.completed.iter().find(|entry| entry.key == key) {
            return if completed.fingerprint == fingerprint {
                Ok(RecoveryAdmission::Replay(completed.outcome.clone()))
            } else {
                Err("browser execution identity was reused for a different recovery request".into())
            };
        }
        if let Some(active) = state.active.get(&key) {
            return if active.fingerprint == fingerprint {
                Ok(RecoveryAdmission::Wait(Arc::clone(active)))
            } else {
                Err("browser execution identity was reused for a different recovery request".into())
            };
        }
        Err("browser action receipt is unknown; physical completion proof is unavailable".into())
    }

    fn complete(
        &self,
        key: &ExecutionKey,
        record: &Arc<ExecutionRecord>,
        outcome: ExecutionOutcome,
    ) {
        {
            let mut stored = record
                .outcome
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *stored = Some(outcome.clone());
            record.completed.notify_all();
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.active.remove(key);
        if state.completed.len() >= COMPLETED_BROWSER_ACTION_LIMIT {
            state.completed.pop_front();
        }
        state.completed.push_back(CompletedExecution {
            key: key.clone(),
            fingerprint: record.fingerprint,
            outcome,
        });
    }
}

impl BrowserControllerProcessStore {
    #[cfg(test)]
    pub(crate) fn test_forget_completed_browser_actions(&self) {
        self.executions.forget_completed();
    }

    #[cfg(test)]
    pub(crate) fn test_browser_action_cancellation_request_count(&self) -> usize {
        self.executions.cancellation_request_count()
    }

    pub(crate) fn cancel_browser_action(&self, session_id: &str, execution_id: &str) -> bool {
        #[cfg(test)]
        self.executions
            .cancellation_requests
            .fetch_add(1, Ordering::AcqRel);
        let state = self
            .executions
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(execution) = state
            .active
            .get(&(session_id.to_string(), execution_id.to_string()))
        else {
            return false;
        };
        execution.signal.requested.store(true, Ordering::Release);
        true
    }

    pub(crate) fn perform_cancellable_browser_action(
        &self,
        session_id: &str,
        execution_id: &str,
        target_id: &str,
        document_id: &str,
        node_ref: &str,
        action: &BrowserLocatorAction,
        timeout_ms: u64,
    ) -> Result<crate::transport::room_browser_controller::RoomBrowserControllerResult, String>
    {
        let fingerprint = action_fingerprint(target_id, document_id, node_ref, action, timeout_ms)?;
        self.perform_cancellable_operation(
            session_id,
            execution_id,
            fingerprint,
            Response::Action { result: None },
            |ownership| {
                ownership
                    .perform_browser_action(
                        session_id,
                        target_id,
                        document_id,
                        node_ref,
                        action,
                        timeout_ms,
                    )
                    .map(|result| Response::Action {
                        result: Some(result),
                    })
            },
        )
    }

    pub(crate) fn perform_cancellable_browser_upload(
        &self,
        session_id: &str,
        execution_id: &str,
        target_id: &str,
        document_id: &str,
        node_ref: &str,
        files: &BrowserUploadFiles,
    ) -> ExecutionOutcome {
        let fingerprint = upload_fingerprint(target_id, document_id, node_ref, files)?;
        self.perform_cancellable_operation(
            session_id,
            execution_id,
            fingerprint,
            Response::Upload { result: None },
            |ownership| {
                ownership
                    .upload_browser_files(session_id, target_id, document_id, node_ref, files)
                    .map(|result| Response::Upload {
                        result: Some(result),
                    })
            },
        )
    }

    fn perform_cancellable_operation(
        &self,
        session_id: &str,
        execution_id: &str,
        fingerprint: [u8; 32],
        unavailable: Response,
        operation: impl FnOnce(&mut StdioOwnership) -> ExecutionOutcome,
    ) -> ExecutionOutcome {
        let Some(ownership) = &self.ownership else {
            return Ok(unavailable);
        };
        let active = match self
            .executions
            .register(session_id, execution_id, fingerprint)?
        {
            ExecutionAdmission::Replay(outcome) => return outcome,
            ExecutionAdmission::Wait(record) => return record.wait(),
            ExecutionAdmission::Start(active) => active,
        };
        let mut ownership = ownership
            .lock()
            .map_err(|_| "browser controller supervisor lock poisoned")?;
        ownership.supervisor.backend.action_cancellation = Some(Arc::clone(&active.signal));
        let result = operation(&mut ownership);
        ownership.supervisor.backend.action_cancellation = None;
        let outcome = if active.signal.stopped.load(Ordering::Acquire) {
            let controller_fenced = active.signal.fenced();
            Ok(Response::ActionCancelled { controller_fenced })
        } else {
            result
        };
        active.finish(outcome)
    }

    pub(crate) fn recover_cancellable_browser_action(
        &self,
        session_id: &str,
        execution_id: &str,
        target_id: &str,
        document_id: &str,
        node_ref: &str,
        action: &BrowserLocatorAction,
        timeout_ms: u64,
    ) -> Result<Response, String> {
        let fingerprint = action_fingerprint(target_id, document_id, node_ref, action, timeout_ms)?;
        self.recover_cancellable_operation(session_id, execution_id, fingerprint)
    }

    pub(crate) fn recover_cancellable_browser_upload(
        &self,
        session_id: &str,
        execution_id: &str,
        target_id: &str,
        document_id: &str,
        node_ref: &str,
        files: &BrowserUploadFiles,
    ) -> ExecutionOutcome {
        let fingerprint = upload_fingerprint(target_id, document_id, node_ref, files)?;
        self.recover_cancellable_operation(session_id, execution_id, fingerprint)
    }

    fn recover_cancellable_operation(
        &self,
        session_id: &str,
        execution_id: &str,
        fingerprint: [u8; 32],
    ) -> ExecutionOutcome {
        match self
            .executions
            .recover(session_id, execution_id, fingerprint)?
        {
            RecoveryAdmission::Replay(outcome) => outcome,
            RecoveryAdmission::Wait(record) => record.wait(),
        }
    }
}

fn upload_fingerprint(
    target_id: &str,
    document_id: &str,
    node_ref: &str,
    files: &BrowserUploadFiles,
) -> Result<[u8; 32], String> {
    let request = serde_json::to_vec(&("upload", target_id, document_id, node_ref, files))
        .map_err(|error| format!("failed to fingerprint browser upload: {error}"))?;
    Ok(Sha256::digest(request).into())
}

fn action_fingerprint(
    target_id: &str,
    document_id: &str,
    node_ref: &str,
    action: &BrowserLocatorAction,
    timeout_ms: u64,
) -> Result<[u8; 32], String> {
    let request = serde_json::to_vec(&(target_id, document_id, node_ref, action, timeout_ms))
        .map_err(|error| format!("failed to fingerprint browser action: {error}"))?;
    Ok(Sha256::digest(request).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn completed() -> ExecutionOutcome {
        Ok(Response::ActionCancelled {
            controller_fenced: false,
        })
    }

    #[test]
    fn completed_execution_replays_only_the_identical_request() {
        let executions = BrowserActionExecutions::default();
        let fingerprint = [7; 32];
        let ExecutionAdmission::Start(active) = executions
            .register("room", "00000000000000000000000000000001", fingerprint)
            .unwrap()
        else {
            panic!("first execution must start");
        };
        assert_eq!(active.finish(completed()), completed());
        assert!(matches!(
            executions
                .register("room", "00000000000000000000000000000001", fingerprint)
                .unwrap(),
            ExecutionAdmission::Replay(outcome) if outcome == completed()
        ));
        let error = match executions.register("room", "00000000000000000000000000000001", [8; 32]) {
            Ok(_) => panic!("changed request must not reuse an execution identity"),
            Err(error) => error,
        };
        assert!(error.contains("different request"));
        assert!(matches!(
            executions
                .recover("room", "00000000000000000000000000000001", fingerprint)
                .unwrap(),
            RecoveryAdmission::Replay(outcome) if outcome == completed()
        ));
    }

    #[test]
    fn concurrent_identical_execution_waits_for_one_terminal_outcome() {
        let executions = BrowserActionExecutions::default();
        let execution_id = "00000000000000000000000000000002";
        let ExecutionAdmission::Start(active) =
            executions.register("room", execution_id, [9; 32]).unwrap()
        else {
            panic!("first execution must start");
        };
        let RecoveryAdmission::Wait(waiting) =
            executions.recover("room", execution_id, [9; 32]).unwrap()
        else {
            panic!("recovery of an in-flight execution must wait");
        };
        let waiter = std::thread::spawn(move || waiting.wait());
        assert_eq!(active.finish(completed()), completed());
        assert_eq!(waiter.join().unwrap(), completed());
    }

    #[test]
    fn completed_execution_receipts_are_bounded() {
        let executions = BrowserActionExecutions::default();
        for sequence in 0..=COMPLETED_BROWSER_ACTION_LIMIT {
            let execution_id = format!("{sequence:032x}");
            let ExecutionAdmission::Start(active) = executions
                .register("room", &execution_id, [sequence as u8; 32])
                .unwrap()
            else {
                panic!("new execution must start");
            };
            active.finish(completed()).unwrap();
        }
        let state = executions.state.lock().unwrap();
        assert_eq!(state.completed.len(), COMPLETED_BROWSER_ACTION_LIMIT);
        assert!(state
            .completed
            .iter()
            .all(|receipt| receipt.key.1 != "00000000000000000000000000000000"));
    }

    #[test]
    fn missing_receipt_cannot_admit_a_new_physical_execution() {
        let executions = BrowserActionExecutions::default();
        let error = match executions.recover("room", "00000000000000000000000000000003", [3; 32]) {
            Ok(_) => panic!("unknown recovery must not start an execution"),
            Err(error) => error,
        };
        assert!(error.contains("proof is unavailable"));
        assert!(executions.state.lock().unwrap().active.is_empty());
    }
}
