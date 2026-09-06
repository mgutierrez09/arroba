use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex as StdMutex};

use crate::error::DaemonError;
use crate::session::{PromptQueueItem, PromptStatus, PromptSubmissionOutcome, RuntimeSession};

mod profile_transition;
pub(crate) use profile_transition::AgentProfileTransitionClaim;

pub(crate) const PROMPT_QUEUE_LIMIT: usize = 128;

#[derive(Debug, Clone, Default)]
struct OwnedAgentPromptState {
    active_prompt: Option<PromptQueueItem>,
    queued_prompts: VecDeque<PromptQueueItem>,
}

impl OwnedAgentPromptState {
    fn from_session(session: &RuntimeSession, agent_id: &str) -> Self {
        session
            .prompt_states()
            .get(agent_id)
            .map(|state| Self {
                active_prompt: state.active_prompt().cloned(),
                queued_prompts: state.queued_prompts().clone(),
            })
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PromptStateKey {
    session_id: String,
    agent_id: String,
}

impl PromptStateKey {
    fn new(session_id: &str, agent_id: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            agent_id: agent_id.to_string(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PromptStateOwner {
    state: Arc<StdMutex<PromptStateOwnerState>>,
    delivery_settlement_claims: Arc<StdMutex<BTreeSet<PromptStateKey>>>,
}

pub(crate) struct PromptDeliverySettlementClaim {
    key: PromptStateKey,
    claims: Arc<StdMutex<BTreeSet<PromptStateKey>>>,
}

impl Drop for PromptDeliverySettlementClaim {
    fn drop(&mut self) {
        self.claims
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.key);
    }
}

#[derive(Debug, Default)]
struct PromptStateOwnerState {
    states: BTreeMap<PromptStateKey, OwnedAgentPromptState>,
    profile_transitions: BTreeMap<PromptStateKey, Arc<()>>,
    next_pending_prompt_number: u64,
}

impl PromptStateOwner {
    pub(crate) fn try_claim_active_prompt_delivery_settlement(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        prompt_id: &str,
        provider_run_id: &str,
    ) -> Option<PromptDeliverySettlementClaim> {
        let key = PromptStateKey::new(session.id(), agent_id);
        {
            let mut claims = self
                .delivery_settlement_claims
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !claims.insert(key.clone()) {
                return None;
            }
        }
        let matches = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .ensure_agent_state(session, agent_id)
            .active_prompt
            .as_ref()
            .is_some_and(|prompt| {
                prompt.id() == prompt_id
                    && prompt.durable_delivery_phase()
                        == Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
                    && prompt.durable_delivery_provider_run_id() == Some(provider_run_id)
            });
        if !matches {
            self.delivery_settlement_claims
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&key);
            return None;
        }
        Some(PromptDeliverySettlementClaim {
            key,
            claims: Arc::clone(&self.delivery_settlement_claims),
        })
    }

    pub(crate) fn compare_and_mark_active_prompt_delivery_failure(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        prompt_id: &str,
        provider_run_id: &str,
        provider_session_id: &str,
        status_transition: (PromptStatus, PromptStatus),
    ) -> Option<PromptQueueItem> {
        let (expected_status, next_status) = status_transition;
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let active = owner
            .ensure_agent_state(session, agent_id)
            .active_prompt
            .as_mut()?;
        if active.id() != prompt_id
            || active.status() != expected_status
            || active.durable_delivery_phase()
                != Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
            || active.durable_delivery_provider_run_id() != Some(provider_run_id)
            || active.durable_delivery_provider_session_id() != Some(provider_session_id)
        {
            return None;
        }
        active.set_status(next_status);
        active.set_durable_delivery_failure_pending(next_status == PromptStatus::Cancelling);
        Some(active.clone())
    }

    pub(crate) fn compare_and_restore_active_prompt_after_resume_superseded(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        prompt_id: &str,
        provider_run_id: &str,
        failed_provider_session_id: &str,
        current_provider_session_id: &str,
    ) -> Option<(PromptQueueItem, PromptQueueItem)> {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let active = owner
            .ensure_agent_state(session, agent_id)
            .active_prompt
            .as_mut()?;
        if active.id() != prompt_id
            || active.status() != PromptStatus::Cancelling
            || !active.durable_delivery_failure_pending()
            || active.durable_delivery_phase()
                != Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
            || active.durable_delivery_provider_run_id() != Some(provider_run_id)
            || active.durable_delivery_provider_session_id() != Some(failed_provider_session_id)
        {
            return None;
        }
        let previous = active.clone();
        active.set_status(PromptStatus::Dispatching);
        active.set_durable_delivery(
            crate::session::DurablePromptDeliveryPhase::Dispatching,
            Some(provider_run_id.to_string()),
            Some(current_provider_session_id.to_string()),
        );
        active.set_durable_delivery_failure_pending(false);
        Some((previous, active.clone()))
    }

    pub(crate) fn replay_durable_submission(
        &self,
        session: &RuntimeSession,
        prompt: &PromptQueueItem,
    ) -> Result<Option<PromptSubmissionOutcome>, DaemonError> {
        let Some(operation_id) = prompt.durable_operation_id() else {
            return Ok(None);
        };
        let fingerprint =
            prompt
                .durable_operation_fingerprint()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "replay durable prompt submission",
                    message: format!(
                        "prompt operation `{operation_id}` is missing its request fingerprint"
                    ),
                })?;
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .submission_for_durable_operation(session.id(), operation_id, fingerprint)
    }

    pub(crate) fn active_prompt_for_agent(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> Option<PromptQueueItem> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .ensure_agent_state(session, agent_id)
            .active_prompt
            .clone()
    }

    pub(crate) fn active_prompt_for_agent_snapshot(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> Option<PromptQueueItem> {
        let key = PromptStateKey::new(session.id(), agent_id);
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .states
            .get(&key)
            .and_then(|state| state.active_prompt.clone())
    }

    pub(crate) fn active_prompt_agent_id(&self, session: &RuntimeSession) -> Option<String> {
        let owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(focused_agent_id) = session.focused_agent_id() {
            if owner
                .states
                .get(&PromptStateKey::new(session.id(), focused_agent_id))
                .and_then(|state| state.active_prompt.as_ref())
                .is_some()
            {
                return Some(focused_agent_id.to_string());
            }
        }

        let active_agents = owner
            .agent_ids_for_session(session)
            .into_iter()
            .filter(|agent_id| {
                owner
                    .states
                    .get(&PromptStateKey::new(session.id(), agent_id))
                    .and_then(|state| state.active_prompt.as_ref())
                    .is_some()
            })
            .collect::<Vec<_>>();
        if active_agents.len() == 1 {
            active_agents.into_iter().next()
        } else {
            None
        }
    }

    pub(crate) fn has_any_active_prompt(&self, session: &RuntimeSession) -> bool {
        let owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        owner
            .agent_ids_for_session(session)
            .into_iter()
            .any(|agent_id| {
                owner
                    .states
                    .get(&PromptStateKey::new(session.id(), &agent_id))
                    .and_then(|state| state.active_prompt.as_ref())
                    .is_some()
            })
    }

    pub(crate) fn active_prompt_agent_ids(&self, session: &RuntimeSession) -> Vec<String> {
        let owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        owner
            .agent_ids_for_session(session)
            .into_iter()
            .filter(|agent_id| {
                owner
                    .states
                    .get(&PromptStateKey::new(session.id(), agent_id))
                    .and_then(|state| state.active_prompt.as_ref())
                    .is_some()
            })
            .collect()
    }

    pub(crate) fn queued_prompt_count_for_agent(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> usize {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .ensure_agent_state(session, agent_id)
            .queued_prompts
            .len()
    }

    pub(crate) fn submit_prepared_prompt(
        &self,
        session: &RuntimeSession,
        mut prompt: PromptQueueItem,
        force_queue: bool,
    ) -> Result<PromptSubmissionOutcome, DaemonError> {
        let agent_id = prompt.target_agent_id().to_string();
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let (Some(operation_id), Some(fingerprint)) = (
            prompt.durable_operation_id(),
            prompt.durable_operation_fingerprint(),
        ) {
            if let Some(outcome) =
                owner.submission_for_durable_operation(session.id(), operation_id, fingerprint)?
            {
                return Ok(outcome);
            }
        }
        let profile_transition_pending = owner
            .profile_transitions
            .contains_key(&PromptStateKey::new(session.id(), &agent_id));
        let should_start = {
            let state = owner.ensure_agent_state(session, &agent_id);
            !force_queue && !profile_transition_pending && state.active_prompt.is_none()
        };
        if should_start {
            let state = owner.ensure_agent_state(session, &agent_id);
            prompt.set_durable_initially_queued(false);
            prompt.set_durable_delivery(
                crate::session::DurablePromptDeliveryPhase::Accepted,
                None,
                None,
            );
            prompt.set_status(PromptStatus::Running);
            state.active_prompt = Some(prompt.clone());
            Ok(PromptSubmissionOutcome::Started { prompt })
        } else {
            let pending_prompt_id = owner.next_pending_prompt_id();
            let state = owner.ensure_agent_state(session, &agent_id);
            if state.queued_prompts.len() >= PROMPT_QUEUE_LIMIT {
                crate::logging::warn_with_fields(
                    "daemon.prompt_queue",
                    "agent prompt queue overloaded",
                    serde_json::json!({
                        "session_id": session.id(),
                        "agent_id": agent_id,
                        "prompt_id": prompt.id(),
                        "queued_prompts": state.queued_prompts.len(),
                        "queue_limit": PROMPT_QUEUE_LIMIT,
                    }),
                );
                return Err(DaemonError::LocalTransport {
                    operation: "queue prompt",
                    message: format!(
                        "agent prompt queue overloaded: queued prompt limit {PROMPT_QUEUE_LIMIT} reached"
                    ),
                });
            }
            prompt.set_durable_initially_queued(true);
            prompt.set_durable_delivery(
                crate::session::DurablePromptDeliveryPhase::Accepted,
                None,
                None,
            );
            prompt = prompt.into_pending_queue_item(pending_prompt_id);
            if prompt.workflow_run_id().is_some() {
                state.queued_prompts.push_back(prompt.clone());
            } else {
                let insert_at = state
                    .queued_prompts
                    .iter()
                    .position(|queued| queued.workflow_run_id().is_some())
                    .unwrap_or(state.queued_prompts.len());
                state.queued_prompts.insert(insert_at, prompt.clone());
            }
            Ok(PromptSubmissionOutcome::Queued { prompt })
        }
    }

    pub(crate) fn complete_active_prompt_only(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> Option<PromptQueueItem> {
        self.complete_active_prompt_if_matches(session, agent_id, None)
    }

    pub(crate) fn complete_active_prompt_if_matches(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        expected_prompt_id: Option<&str>,
    ) -> Option<PromptQueueItem> {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = owner.ensure_agent_state(session, agent_id);
        if expected_prompt_id.is_some_and(|expected_prompt_id| {
            state.active_prompt.as_ref().map(PromptQueueItem::id) != Some(expected_prompt_id)
        }) {
            return None;
        }
        let mut completed = state.active_prompt.take()?;
        completed.set_status(PromptStatus::Completed);
        Some(completed)
    }

    pub(crate) fn cancel_active_prompt_only(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> Option<PromptQueueItem> {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = owner.ensure_agent_state(session, agent_id);
        let mut cancelled = state.active_prompt.take()?;
        cancelled.set_status(PromptStatus::Cancelled);
        Some(cancelled)
    }

    pub(crate) fn begin_cancelling_active_prompt(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> Option<PromptQueueItem> {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let active = owner
            .ensure_agent_state(session, agent_id)
            .active_prompt
            .as_mut()?;
        active.set_status(PromptStatus::Cancelling);
        Some(active.clone())
    }

    pub(crate) fn mark_active_prompt_running(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> Option<PromptQueueItem> {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let active = owner
            .ensure_agent_state(session, agent_id)
            .active_prompt
            .as_mut()?;
        active.set_status(PromptStatus::Running);
        Some(active.clone())
    }

    pub(crate) fn mark_active_prompt_delivery(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        prompt_id: &str,
        phase: crate::session::DurablePromptDeliveryPhase,
        provider_run_id: Option<String>,
        provider_session_id: Option<String>,
    ) -> Result<PromptQueueItem, DaemonError> {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let active = owner
            .ensure_agent_state(session, agent_id)
            .active_prompt
            .as_mut()
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session.id().to_string(),
            })?;
        if active.id() != prompt_id {
            return Err(DaemonError::LocalTransport {
                operation: "mark prompt delivery",
                message: format!(
                    "active prompt `{}` does not match delivery prompt `{prompt_id}`",
                    active.id()
                ),
            });
        }
        active.set_durable_delivery(phase, provider_run_id, provider_session_id);
        if phase == crate::session::DurablePromptDeliveryPhase::Delivered
            && active.status() == PromptStatus::Dispatching
        {
            active.set_status(PromptStatus::Running);
        }
        Ok(active.clone())
    }

    pub(crate) fn replace_active_prompt_if_matches(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        expected: &PromptQueueItem,
        replacement: PromptQueueItem,
    ) -> bool {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let active = &mut owner.ensure_agent_state(session, agent_id).active_prompt;
        if active.as_ref() != Some(expected) {
            return false;
        }
        *active = Some(replacement);
        true
    }

    pub(crate) fn begin_active_prompt_recovery(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        prompt_id: &str,
    ) -> Result<PromptQueueItem, DaemonError> {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let active = owner
            .ensure_agent_state(session, agent_id)
            .active_prompt
            .as_mut()
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session.id().to_string(),
            })?;
        if active.id() != prompt_id {
            return Err(DaemonError::LocalTransport {
                operation: "begin prompt recovery",
                message: format!(
                    "active prompt `{}` does not match recovery prompt `{prompt_id}`",
                    active.id()
                ),
            });
        }
        active.begin_durable_recovery_operation();
        Ok(active.clone())
    }

    pub(crate) fn mark_active_prompt_recovery_phase(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        prompt_id: &str,
        operation_id: &str,
        phase: crate::session::DurablePromptDeliveryPhase,
    ) -> Result<PromptQueueItem, DaemonError> {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let active = owner
            .ensure_agent_state(session, agent_id)
            .active_prompt
            .as_mut()
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session.id().to_string(),
            })?;
        if active.id() != prompt_id || !active.mark_durable_recovery_phase(operation_id, phase) {
            return Err(DaemonError::LocalTransport {
                operation: "mark prompt recovery",
                message: format!(
                    "active prompt `{}` does not match recovery operation `{operation_id}` for prompt `{prompt_id}`",
                    active.id()
                ),
            });
        }
        Ok(active.clone())
    }

    pub(crate) fn compare_and_mark_active_prompt_recovery_phase(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        prompt_id: &str,
        operation_id: &str,
        expected_phase: crate::session::DurablePromptDeliveryPhase,
        next_phase: crate::session::DurablePromptDeliveryPhase,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let active = owner
            .ensure_agent_state(session, agent_id)
            .active_prompt
            .as_mut()
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session.id().to_string(),
            })?;
        if active.id() != prompt_id
            || active.durable_recovery_operation_id() != Some(operation_id)
            || active.durable_recovery_phase() != Some(expected_phase)
        {
            return Ok(None);
        }
        if !active.mark_durable_recovery_phase(operation_id, next_phase) {
            return Ok(None);
        }
        Ok(Some(active.clone()))
    }

    pub(crate) fn finalize_active_prompt_cancellation(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> Option<PromptQueueItem> {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = owner.ensure_agent_state(session, agent_id);
        let active_status = state.active_prompt.as_ref()?.status();
        if active_status != PromptStatus::Cancelling {
            return None;
        }
        let mut cancelled = state.active_prompt.take()?;
        cancelled.set_status(PromptStatus::Cancelled);
        Some(cancelled)
    }

    pub(crate) fn peek_next_queued_prompt(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> Option<PromptQueueItem> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .ensure_agent_state(session, agent_id)
            .queued_prompts
            .front()
            .cloned()
    }

    #[cfg(test)]
    pub(crate) fn activate_next_queued_prompt(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        expected_prompt_id: Option<&str>,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let state = owner.ensure_agent_state(session, agent_id);
        if let Some(active_prompt) = state.active_prompt.as_ref() {
            return Err(DaemonError::LocalTransport {
                operation: "activate queued prompt",
                message: format!(
                    "cannot activate queued prompt for agent `{agent_id}` while active prompt `{}` is still running",
                    active_prompt.id()
                ),
            });
        }
        let Some(front) = state.queued_prompts.front() else {
            return Ok(None);
        };
        if let Some(expected_prompt_id) = expected_prompt_id {
            if front.id() != expected_prompt_id {
                return Err(DaemonError::LocalTransport {
                    operation: "activate expected queued prompt",
                    message: format!(
                        "expected queued prompt `{}` but prompt owner queue front was `{}`",
                        expected_prompt_id,
                        front.id()
                    ),
                });
            }
        }
        validate_prompt_target_agent("activate queued prompt", agent_id, front)?;
        let mut active = state
            .queued_prompts
            .pop_front()
            .expect("queue front checked above");
        active.set_status(PromptStatus::Running);
        state.active_prompt = Some(active.clone());
        Ok(Some(active))
    }

    pub(crate) fn activate_next_queued_prompt_with_prompt_id(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        expected_prompt_id: Option<&str>,
        prompt_id: String,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        if owner
            .profile_transitions
            .contains_key(&PromptStateKey::new(session.id(), agent_id))
        {
            return Ok(None);
        }
        let state = owner.ensure_agent_state(session, agent_id);
        Self::activate_owned_queued_prompt(state, agent_id, expected_prompt_id, prompt_id)
    }

    fn activate_owned_queued_prompt(
        state: &mut OwnedAgentPromptState,
        agent_id: &str,
        expected_prompt_id: Option<&str>,
        prompt_id: String,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        if let Some(active_prompt) = state.active_prompt.as_ref() {
            return Err(DaemonError::LocalTransport {
                operation: "activate queued prompt",
                message: format!(
                    "cannot activate queued prompt for agent `{agent_id}` while active prompt `{}` is still running",
                    active_prompt.id()
                ),
            });
        }
        let Some(front) = state.queued_prompts.front() else {
            return Ok(None);
        };
        if let Some(expected_prompt_id) = expected_prompt_id {
            if front.id() != expected_prompt_id {
                return Err(DaemonError::LocalTransport {
                    operation: "activate expected queued prompt",
                    message: format!(
                        "expected queued prompt `{}` but prompt owner queue front was `{}`",
                        expected_prompt_id,
                        front.id()
                    ),
                });
            }
        }
        validate_prompt_target_agent("activate queued prompt", agent_id, front)?;
        let mut active = state
            .queued_prompts
            .pop_front()
            .expect("queue front checked above")
            .with_id(prompt_id);
        active.set_status(PromptStatus::Dispatching);
        state.active_prompt = Some(active.clone());
        Ok(Some(active))
    }

    #[cfg(test)]
    pub(crate) fn activate_prompt(
        &self,
        session: &RuntimeSession,
        mut prompt: PromptQueueItem,
    ) -> Result<PromptQueueItem, DaemonError> {
        let agent_id = prompt.target_agent_id().to_string();
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let state = owner.ensure_agent_state(session, &agent_id);
        if let Some(active_prompt) = state.active_prompt.as_ref() {
            if active_prompt.id() != prompt.id() {
                return Err(DaemonError::LocalTransport {
                    operation: "activate prompt",
                    message: format!(
                        "cannot activate prompt `{}` for agent `{agent_id}` while active prompt `{}` is still running",
                        prompt.id(),
                        active_prompt.id()
                    ),
                });
            }
        }
        state
            .queued_prompts
            .retain(|queued| queued.id() != prompt.id());
        prompt.set_status(PromptStatus::Running);
        state.active_prompt = Some(prompt.clone());
        Ok(prompt)
    }

    pub(crate) fn sync_external_active_prompt(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        active_prompt: Option<PromptQueueItem>,
    ) -> bool {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let state = owner.ensure_agent_state(session, agent_id);
        match active_prompt {
            Some(mut prompt) => {
                if prompt.target_agent_id() != agent_id {
                    crate::logging::warn_with_fields(
                        "daemon.prompt_state",
                        "ignored external active prompt with mismatched target agent",
                        serde_json::json!({
                            "session_id": session.id(),
                            "agent_id": agent_id,
                            "prompt_id": prompt.id(),
                            "prompt_target_agent_id": prompt.target_agent_id(),
                        }),
                    );
                    return false;
                }
                if state
                    .active_prompt
                    .as_ref()
                    .is_some_and(|active| active.is_chariox_owned())
                {
                    return false;
                }
                prompt.set_status(PromptStatus::Running);
                if state.active_prompt.as_ref() == Some(&prompt) {
                    return false;
                }
                state.active_prompt = Some(prompt);
                true
            }
            None => {
                if state
                    .active_prompt
                    .as_ref()
                    .is_some_and(|active| active.is_external())
                {
                    state.active_prompt = None;
                    return true;
                }
                false
            }
        }
    }

    pub(crate) fn remove_queued_prompts_by_workflow_run(
        &self,
        session: &RuntimeSession,
        workflow_run_id: &str,
    ) -> usize {
        self.remove_queued_prompts_matching(session, |prompt| {
            prompt.workflow_run_id() == Some(workflow_run_id)
        })
    }

    pub(crate) fn remove_queued_prompts_for_agent(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> usize {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let state = owner.ensure_agent_state(session, agent_id);
        let removed = state.queued_prompts.len();
        state.queued_prompts.clear();
        removed
    }

    pub(crate) fn remove_queued_metaagent_event_prompts_for_agent(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        source_attachment_id: &str,
    ) -> usize {
        self.remove_queued_prompts_matching(session, |prompt| {
            prompt.target_agent_id() == agent_id
                && prompt.source_attachment_id() == source_attachment_id
                && prompt.prompt().trim()
                    == crate::scheduler::prompt_injection::METAAGENT_EVENT_VISIBLE_PROMPT
        })
    }

    pub(crate) fn remove_queued_prompt(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        prompt_id: &str,
    ) -> Option<PromptQueueItem> {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let state = owner.ensure_agent_state(session, agent_id);
        let index = state
            .queued_prompts
            .iter()
            .position(|prompt| prompt.id() == prompt_id)?;
        state.queued_prompts.remove(index)
    }

    pub(crate) fn update_queued_prompt(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        prompt_id: &str,
        prompt: impl Into<String>,
    ) -> Option<PromptQueueItem> {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let state = owner.ensure_agent_state(session, agent_id);
        let queued = state
            .queued_prompts
            .iter_mut()
            .find(|queued| queued.id() == prompt_id)?;
        queued.set_prompt(prompt);
        Some(queued.clone())
    }

    pub(crate) fn state_parts(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> (Option<PromptQueueItem>, VecDeque<PromptQueueItem>) {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let state = owner.ensure_agent_state(session, agent_id);
        (state.active_prompt.clone(), state.queued_prompts.clone())
    }

    pub(crate) fn project_into_session(&self, session: &mut RuntimeSession) {
        let projected_states = {
            let owner = self
                .state
                .lock()
                .expect("prompt state owner lock should not be poisoned");
            owner
                .agent_ids_for_session(session)
                .into_iter()
                .map(|agent_id| {
                    let state = owner
                        .states
                        .get(&PromptStateKey::new(session.id(), &agent_id))
                        .cloned()
                        .unwrap_or_default();
                    (agent_id, state.active_prompt, state.queued_prompts)
                })
                .collect::<Vec<_>>()
        };
        for (agent_id, active_prompt, queued_prompts) in projected_states {
            session.mirror_agent_prompt_state(&agent_id, active_prompt, queued_prompts);
        }
    }

    pub(crate) fn restore_session_state(&self, session: &RuntimeSession) {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let restored_agent_ids = session
            .prompt_states()
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        owner.states.retain(|key, _| {
            key.session_id != session.id() || restored_agent_ids.contains(&key.agent_id)
        });
        for agent_id in session.prompt_states().keys() {
            let restored = OwnedAgentPromptState::from_session(session, agent_id);
            if restored.active_prompt.is_none() && restored.queued_prompts.is_empty() {
                owner
                    .states
                    .remove(&PromptStateKey::new(session.id(), agent_id));
            } else {
                owner
                    .states
                    .insert(PromptStateKey::new(session.id(), agent_id), restored);
            }
        }
    }

    pub(crate) fn remove_session(&self, session_id: &str) {
        self.state
            .lock()
            .expect("prompt state owner lock should not be poisoned")
            .states
            .retain(|key, _| key.session_id.as_str() != session_id);
    }

    pub(crate) fn remove_agent(&self, session_id: &str, agent_id: &str) {
        self.state
            .lock()
            .expect("prompt state owner lock should not be poisoned")
            .states
            .remove(&PromptStateKey::new(session_id, agent_id));
    }

    fn remove_queued_prompts_matching(
        &self,
        session: &RuntimeSession,
        mut should_remove: impl FnMut(&PromptQueueItem) -> bool,
    ) -> usize {
        let mut owner = self
            .state
            .lock()
            .expect("prompt state owner lock should not be poisoned");
        let mut agent_ids = session
            .agents()
            .iter()
            .map(|agent| agent.id().to_string())
            .collect::<Vec<_>>();
        agent_ids.extend(session.prompt_states().keys().cloned());
        agent_ids.sort();
        agent_ids.dedup();

        let mut removed = 0;
        for agent_id in agent_ids {
            let state = owner.ensure_agent_state(session, &agent_id);
            let original_len = state.queued_prompts.len();
            state.queued_prompts.retain(|prompt| !should_remove(prompt));
            removed += original_len - state.queued_prompts.len();
        }
        removed
    }
}

fn validate_prompt_target_agent(
    operation: &'static str,
    agent_id: &str,
    prompt: &PromptQueueItem,
) -> Result<(), DaemonError> {
    if prompt.target_agent_id() == agent_id {
        return Ok(());
    }
    Err(DaemonError::LocalTransport {
        operation,
        message: format!(
            "prompt `{}` targets agent `{}` but is stored under agent `{agent_id}`",
            prompt.id(),
            prompt.target_agent_id()
        ),
    })
}

impl PromptStateOwnerState {
    fn submission_for_durable_operation(
        &self,
        session_id: &str,
        operation_id: &str,
        fingerprint: &str,
    ) -> Result<Option<PromptSubmissionOutcome>, DaemonError> {
        let prompt = self
            .states
            .iter()
            .filter(|(key, _)| key.session_id == session_id)
            .flat_map(|(_, state)| {
                state
                    .active_prompt
                    .iter()
                    .chain(state.queued_prompts.iter())
            })
            .find(|prompt| prompt.durable_operation_id() == Some(operation_id));
        let Some(prompt) = prompt else {
            return Ok(None);
        };
        if prompt.durable_operation_fingerprint() != Some(fingerprint) {
            return Err(DaemonError::LocalTransport {
                operation: "replay durable prompt submission",
                message: format!(
                    "operation id `{operation_id}` was already used for a different prompt request"
                ),
            });
        }
        let initially_queued = prompt.durable_initially_queued().unwrap_or_else(|| {
            prompt.status() == PromptStatus::Queued || prompt.pending_prompt_id().is_some()
        });
        Ok(Some(if initially_queued {
            PromptSubmissionOutcome::Queued {
                prompt: prompt.clone(),
            }
        } else {
            PromptSubmissionOutcome::Started {
                prompt: prompt.clone(),
            }
        }))
    }

    fn agent_ids_for_session(&self, session: &RuntimeSession) -> Vec<String> {
        let mut agent_ids = session
            .agents()
            .iter()
            .map(|agent| agent.id().to_string())
            .collect::<Vec<_>>();
        agent_ids.extend(session.prompt_states().keys().cloned());
        agent_ids.extend(
            self.states
                .keys()
                .filter(|key| key.session_id == session.id())
                .map(|key| key.agent_id.clone()),
        );
        agent_ids.sort();
        agent_ids.dedup();
        agent_ids
    }

    fn next_pending_prompt_id(&mut self) -> String {
        self.next_pending_prompt_number = self.next_pending_prompt_number.wrapping_add(1);
        format!(
            "pending-prompt-{:016x}",
            crate::session::unix_epoch_ms() ^ self.next_pending_prompt_number.rotate_left(17)
        )
    }

    fn ensure_agent_state(
        &mut self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> &mut OwnedAgentPromptState {
        let key = PromptStateKey::new(session.id(), agent_id);
        self.states.entry(key).or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submit_prepared_prompt_rejects_queue_overflow() {
        let owner = PromptStateOwner::default();
        let session = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );

        for index in 0..PROMPT_QUEUE_LIMIT {
            let outcome = owner
                .submit_prepared_prompt(
                    &session,
                    PromptQueueItem::new(
                        format!("prompt-{index}"),
                        "attachment-1",
                        "agent-1",
                        "queued",
                        PromptStatus::Queued,
                    ),
                    true,
                )
                .expect("prompt should fit while under queue limit");
            assert!(matches!(outcome, PromptSubmissionOutcome::Queued { .. }));
        }

        let error = owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "prompt-overflow",
                    "attachment-1",
                    "agent-1",
                    "overflow",
                    PromptStatus::Queued,
                ),
                true,
            )
            .expect_err("queue limit should reject overflow prompt");

        assert!(error.to_string().contains("agent prompt queue overloaded"));
        assert_eq!(
            owner.queued_prompt_count_for_agent(&session, "agent-1"),
            PROMPT_QUEUE_LIMIT
        );
    }

    #[test]
    fn durable_submission_replay_does_not_duplicate_active_or_queued_prompts() {
        let owner = PromptStateOwner::default();
        let session = RuntimeSession::new(
            "session-durable",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        let first = PromptQueueItem::new(
            "prompt-active",
            "attachment-1",
            "agent-1",
            "active",
            PromptStatus::Queued,
        );
        owner
            .submit_prepared_prompt(&session, first, false)
            .expect("active prompt should submit");
        let durable = PromptQueueItem::new(
            "prompt-durable-draft",
            "attachment-1",
            "agent-1",
            "queued once",
            PromptStatus::Queued,
        )
        .with_durable_operation("command-1", "fingerprint-1");
        let queued = owner
            .submit_prepared_prompt(&session, durable.clone(), false)
            .expect("durable prompt should queue");
        let replayed = owner
            .submit_prepared_prompt(&session, durable, false)
            .expect("durable prompt should replay");

        let queued_id = match queued {
            PromptSubmissionOutcome::Queued { prompt } => prompt.id().to_string(),
            other => panic!("expected queued prompt, got {other:?}"),
        };
        match replayed {
            PromptSubmissionOutcome::Queued { prompt } => assert_eq!(prompt.id(), queued_id),
            other => panic!("expected replayed queued prompt, got {other:?}"),
        }
        assert_eq!(owner.queued_prompt_count_for_agent(&session, "agent-1"), 1);

        let conflict = PromptQueueItem::new(
            "prompt-conflict",
            "attachment-1",
            "agent-1",
            "different",
            PromptStatus::Queued,
        )
        .with_durable_operation("command-1", "fingerprint-2");
        assert!(owner
            .submit_prepared_prompt(&session, conflict, false)
            .expect_err("fingerprint drift should conflict")
            .to_string()
            .contains("already used for a different prompt request"));
    }

    #[test]
    fn active_prompt_delivery_phase_tracks_matching_provider_acknowledgement() {
        let owner = PromptStateOwner::default();
        let session = RuntimeSession::new(
            "session-delivery",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        let prompt = PromptQueueItem::new(
            "prompt-delivery",
            "attachment-1",
            "agent-1",
            "deliver once",
            PromptStatus::Queued,
        )
        .with_durable_operation("command-delivery", "fingerprint-delivery");
        let PromptSubmissionOutcome::Started { prompt } = owner
            .submit_prepared_prompt(&session, prompt, false)
            .expect("prompt should start")
        else {
            panic!("prompt should be active");
        };
        assert_eq!(
            prompt.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Accepted)
        );

        let dispatching = owner
            .mark_active_prompt_delivery(
                &session,
                "agent-1",
                prompt.id(),
                crate::session::DurablePromptDeliveryPhase::Dispatching,
                Some("provider-run-1".to_string()),
                None,
            )
            .expect("dispatching phase should persist");
        assert_eq!(
            dispatching.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
        );
        assert_eq!(
            dispatching.durable_delivery_provider_run_id(),
            Some("provider-run-1")
        );

        let delivered = owner
            .mark_active_prompt_delivery(
                &session,
                "agent-1",
                prompt.id(),
                crate::session::DurablePromptDeliveryPhase::Delivered,
                Some("provider-run-1".to_string()),
                Some("provider-session-1".to_string()),
            )
            .expect("delivery acknowledgement should persist");
        assert_eq!(
            delivered.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Delivered)
        );
        assert_eq!(delivered.status(), PromptStatus::Running);
        assert_eq!(
            delivered.durable_delivery_provider_session_id(),
            Some("provider-session-1")
        );
        assert!(owner
            .mark_active_prompt_delivery(
                &session,
                "agent-1",
                "another-prompt",
                crate::session::DurablePromptDeliveryPhase::Delivered,
                None,
                None,
            )
            .is_err());
    }

    #[test]
    fn activate_next_queued_prompt_rejects_when_active_prompt_exists() {
        let owner = PromptStateOwner::default();
        let session = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        let started = owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "prompt-active",
                    "attachment-1",
                    "agent-1",
                    "active",
                    PromptStatus::Queued,
                ),
                false,
            )
            .expect("first prompt should start");
        assert!(matches!(started, PromptSubmissionOutcome::Started { .. }));
        let queued = owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "prompt-queued",
                    "attachment-1",
                    "agent-1",
                    "queued",
                    PromptStatus::Queued,
                ),
                false,
            )
            .expect("second prompt should queue");
        let queued_prompt_id = match queued {
            PromptSubmissionOutcome::Queued { prompt } => {
                assert!(prompt.id().starts_with("pending-prompt-"));
                assert_eq!(prompt.pending_prompt_id(), Some(prompt.id()));
                prompt.id().to_string()
            }
            PromptSubmissionOutcome::Started { .. } => panic!("second prompt should queue"),
        };

        let error = owner
            .activate_next_queued_prompt(&session, "agent-1", Some(&queued_prompt_id))
            .expect_err("queued prompt must not activate while active prompt is running");

        assert!(error.to_string().contains("cannot activate queued prompt"));
        assert_eq!(
            owner
                .active_prompt_for_agent_snapshot(&session, "agent-1")
                .as_ref()
                .map(|prompt| prompt.id()),
            Some("prompt-active")
        );
        assert_eq!(
            owner
                .peek_next_queued_prompt(&session, "agent-1")
                .as_ref()
                .map(|prompt| prompt.id()),
            Some(queued_prompt_id.as_str())
        );
    }

    #[test]
    fn queued_prompt_promotes_with_new_real_prompt_id() {
        let owner = PromptStateOwner::default();
        let session = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "draft-active",
                    "attachment-1",
                    "agent-1",
                    "active",
                    PromptStatus::Queued,
                ),
                false,
            )
            .expect("first prompt should start");
        let pending_prompt_id = match owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "draft-queued",
                    "attachment-1",
                    "agent-1",
                    "queued",
                    PromptStatus::Queued,
                ),
                false,
            )
            .expect("second prompt should queue")
        {
            PromptSubmissionOutcome::Queued { prompt } => {
                assert!(prompt.id().starts_with("pending-prompt-"));
                assert_eq!(prompt.pending_prompt_id(), Some(prompt.id()));
                prompt.id().to_string()
            }
            PromptSubmissionOutcome::Started { .. } => panic!("second prompt should queue"),
        };

        let completed = owner
            .complete_active_prompt_only(&session, "agent-1")
            .expect("active prompt should complete");
        assert_eq!(completed.id(), "draft-active");

        let started = owner
            .activate_next_queued_prompt_with_prompt_id(
                &session,
                "agent-1",
                Some(&pending_prompt_id),
                "prompt-real-2".to_string(),
            )
            .expect("queued prompt should activate")
            .expect("queued prompt should exist");

        assert_eq!(started.id(), "prompt-real-2");
        assert_eq!(started.pending_prompt_id(), None);
        assert_eq!(started.prompt(), "queued");
        assert_eq!(started.status(), PromptStatus::Dispatching);
        assert!(owner.peek_next_queued_prompt(&session, "agent-1").is_none());

        let running = owner
            .mark_active_prompt_running(&session, "agent-1")
            .expect("dispatching prompt should become running");
        assert_eq!(running.id(), "prompt-real-2");
        assert_eq!(running.status(), PromptStatus::Running);
    }

    #[test]
    fn stale_completion_cannot_consume_a_promoted_prompt() {
        let owner = PromptStateOwner::default();
        let session = RuntimeSession::new(
            "session-stale-completion",
            None,
            "workspace-stale-completion",
            "worktree-stale-completion",
            "machine-1",
            "daemon-1",
        );
        owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "prompt-first",
                    "attachment-1",
                    "agent-1",
                    "first",
                    PromptStatus::Queued,
                ),
                false,
            )
            .expect("first prompt should start");
        let pending_prompt_id = match owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "prompt-queued",
                    "attachment-2",
                    "agent-1",
                    "second",
                    PromptStatus::Queued,
                ),
                false,
            )
            .expect("second prompt should queue")
        {
            PromptSubmissionOutcome::Queued { prompt } => prompt.id().to_string(),
            PromptSubmissionOutcome::Started { .. } => panic!("second prompt should queue"),
        };

        owner
            .complete_active_prompt_if_matches(&session, "agent-1", Some("prompt-first"))
            .expect("the matching first prompt should complete");
        owner
            .activate_next_queued_prompt_with_prompt_id(
                &session,
                "agent-1",
                Some(&pending_prompt_id),
                "prompt-second".to_string(),
            )
            .expect("queued prompt activation should succeed")
            .expect("queued prompt should promote");

        assert!(owner
            .complete_active_prompt_if_matches(&session, "agent-1", Some("prompt-first"))
            .is_none());
        assert_eq!(
            owner
                .active_prompt_for_agent(&session, "agent-1")
                .map(|prompt| prompt.id().to_string()),
            Some("prompt-second".to_string())
        );
    }

    #[test]
    fn project_into_session_uses_owned_prompt_state() {
        let owner = PromptStateOwner::default();
        let mut session = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        session.set_agents(vec![crate::agent::AgentInstance::new(
            "agent-1",
            "agent-1",
            session.id(),
            None,
            "codex",
            None,
            None,
            None,
            crate::agent::GridPosition::new(0, 0, 1, 1),
        )]);
        let active = owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "prompt-active",
                    "attachment-1",
                    "agent-1",
                    "active",
                    PromptStatus::Queued,
                ),
                false,
            )
            .expect("active prompt should submit");
        assert!(matches!(active, PromptSubmissionOutcome::Started { .. }));
        let queued = owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "prompt-queued",
                    "attachment-1",
                    "agent-1",
                    "queued",
                    PromptStatus::Queued,
                ),
                false,
            )
            .expect("queued prompt should submit");
        assert!(matches!(queued, PromptSubmissionOutcome::Queued { .. }));
        assert!(session.active_prompt_for_agent("agent-1").is_none());
        assert!(session.queued_prompts_for_agent("agent-1").is_none());

        owner.project_into_session(&mut session);

        assert_eq!(
            session
                .active_prompt_for_agent("agent-1")
                .map(|prompt| prompt.prompt()),
            Some("active")
        );
        assert_eq!(
            session
                .queued_prompts_for_agent("agent-1")
                .and_then(|queued| queued.front())
                .map(|prompt| prompt.prompt()),
            Some("queued")
        );
    }

    #[test]
    fn projection_does_not_rehydrate_unrestored_session_mirror() {
        let owner = PromptStateOwner::default();
        let mut session = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        session.mirror_agent_prompt_state(
            "agent-1",
            Some(PromptQueueItem::new(
                "stale-prompt",
                "attachment-1",
                "agent-1",
                "stale",
                PromptStatus::Running,
            )),
            VecDeque::new(),
        );

        assert!(session.active_prompt_for_agent("agent-1").is_some());
        assert!(owner
            .active_prompt_for_agent_snapshot(&session, "agent-1")
            .is_none());

        owner.project_into_session(&mut session);

        assert!(session.active_prompt_for_agent("agent-1").is_none());
    }

    #[test]
    fn active_prompt_lookup_does_not_rehydrate_unrestored_session_mirror() {
        let owner = PromptStateOwner::default();
        let mut session = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        session.mirror_agent_prompt_state(
            "agent-1",
            Some(PromptQueueItem::new(
                "stale-prompt",
                "attachment-1",
                "agent-1",
                "stale",
                PromptStatus::Running,
            )),
            VecDeque::new(),
        );

        assert!(owner.active_prompt_for_agent(&session, "agent-1").is_none());
        assert!(owner
            .active_prompt_for_agent_snapshot(&session, "agent-1")
            .is_none());
    }

    #[test]
    fn submit_prepared_prompt_ignores_unrestored_session_mirror() {
        let owner = PromptStateOwner::default();
        let mut session = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        session.mirror_agent_prompt_state(
            "agent-1",
            Some(PromptQueueItem::new(
                "stale-prompt",
                "attachment-1",
                "agent-1",
                "stale",
                PromptStatus::Running,
            )),
            VecDeque::new(),
        );

        let outcome = owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "fresh-prompt",
                    "attachment-1",
                    "agent-1",
                    "fresh",
                    PromptStatus::Queued,
                ),
                false,
            )
            .expect("fresh prompt should submit");

        let started = match outcome {
            PromptSubmissionOutcome::Started { prompt } => prompt,
            PromptSubmissionOutcome::Queued { prompt } => {
                panic!("stale mirror queued fresh prompt as `{}`", prompt.id())
            }
        };
        assert_eq!(started.id(), "fresh-prompt");
        assert_eq!(
            owner
                .active_prompt_for_agent_snapshot(&session, "agent-1")
                .as_ref()
                .map(|prompt| prompt.id()),
            Some("fresh-prompt")
        );
    }

    #[test]
    fn restore_session_state_hydrates_owner_before_projection() {
        let owner = PromptStateOwner::default();
        let mut session = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        session.mirror_agent_prompt_state(
            "agent-1",
            Some(PromptQueueItem::new(
                "restored-prompt",
                "attachment-1",
                "agent-1",
                "restored",
                PromptStatus::Running,
            )),
            VecDeque::from([PromptQueueItem::new(
                "restored-queued",
                "attachment-1",
                "agent-1",
                "queued",
                PromptStatus::Queued,
            )]),
        );

        owner.restore_session_state(&session);
        session.mirror_agent_prompt_state("agent-1", None, VecDeque::new());

        assert!(session.active_prompt_for_agent("agent-1").is_none());
        owner.project_into_session(&mut session);

        assert_eq!(
            session
                .active_prompt_for_agent("agent-1")
                .map(|prompt| prompt.id()),
            Some("restored-prompt")
        );
        assert_eq!(
            session
                .queued_prompts_for_agent("agent-1")
                .and_then(|queued| queued.front())
                .map(|prompt| prompt.id()),
            Some("restored-queued")
        );
    }

    #[test]
    fn restore_session_state_replaces_removed_prompt_states() {
        let owner = PromptStateOwner::default();
        let mut restored_with_prompt = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        restored_with_prompt.mirror_agent_prompt_state(
            "agent-1",
            Some(PromptQueueItem::new(
                "restored-prompt",
                "attachment-1",
                "agent-1",
                "restored",
                PromptStatus::Running,
            )),
            VecDeque::new(),
        );
        owner.restore_session_state(&restored_with_prompt);
        assert!(owner
            .active_prompt_for_agent_snapshot(&restored_with_prompt, "agent-1")
            .is_some());

        let mut restored_without_prompt = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        restored_without_prompt.set_agents(vec![crate::agent::AgentInstance::new(
            "agent-1",
            "agent-1",
            restored_without_prompt.id(),
            None,
            "codex",
            None,
            None,
            None,
            crate::agent::GridPosition::new(0, 0, 1, 1),
        )]);

        owner.restore_session_state(&restored_without_prompt);
        owner.project_into_session(&mut restored_without_prompt);

        assert!(restored_without_prompt
            .active_prompt_for_agent("agent-1")
            .is_none());
        assert!(owner
            .active_prompt_for_agent_snapshot(&restored_without_prompt, "agent-1")
            .is_none());
    }

    #[test]
    fn queued_prompt_activation_rejects_prompt_stored_under_wrong_agent() {
        let owner = PromptStateOwner::default();
        let session = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        let queued_prompt = PromptQueueItem::new(
            "prompt-wrong-agent",
            "attachment-1",
            "agent-2",
            "queued",
            PromptStatus::Queued,
        );
        owner
            .state
            .lock()
            .expect("prompt state lock should not be poisoned")
            .states
            .insert(
                PromptStateKey::new(session.id(), "agent-1"),
                OwnedAgentPromptState {
                    active_prompt: None,
                    queued_prompts: VecDeque::from([queued_prompt]),
                },
            );

        let error = owner
            .activate_next_queued_prompt_with_prompt_id(
                &session,
                "agent-1",
                Some("prompt-wrong-agent"),
                "prompt-real-1".to_string(),
            )
            .expect_err("mismatched queued prompt target must be rejected");

        assert!(error.to_string().contains("prompt `prompt-wrong-agent`"));
        assert!(error.to_string().contains("targets agent `agent-2`"));
        assert!(error.to_string().contains("stored under agent `agent-1`"));
        assert!(owner
            .active_prompt_for_agent_snapshot(&session, "agent-1")
            .is_none());
        assert_eq!(
            owner
                .peek_next_queued_prompt(&session, "agent-1")
                .as_ref()
                .map(|prompt| prompt.id()),
            Some("prompt-wrong-agent")
        );
    }

    #[test]
    fn external_active_prompt_sync_ignores_prompt_targeting_different_agent() {
        let owner = PromptStateOwner::default();
        let session = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        let external_prompt = PromptQueueItem::external_observed_running(
            "codex",
            "thread-1",
            "user-1",
            "agent-2",
            "external prompt",
        );

        let changed = owner.sync_external_active_prompt(&session, "agent-1", Some(external_prompt));

        assert!(!changed);
        assert!(owner
            .active_prompt_for_agent_snapshot(&session, "agent-1")
            .is_none());
    }

    #[test]
    fn activate_prompt_rejects_replacing_different_active_prompt() {
        let owner = PromptStateOwner::default();
        let session = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        let active = owner
            .activate_prompt(
                &session,
                PromptQueueItem::new(
                    "prompt-active",
                    "attachment-1",
                    "agent-1",
                    "active",
                    PromptStatus::Queued,
                ),
            )
            .expect("first prompt should activate");
        assert_eq!(active.status(), PromptStatus::Running);

        let error = owner
            .activate_prompt(
                &session,
                PromptQueueItem::new(
                    "prompt-replacement",
                    "attachment-1",
                    "agent-1",
                    "replacement",
                    PromptStatus::Queued,
                ),
            )
            .expect_err("different active prompt must not be replaced");

        assert!(error.to_string().contains("cannot activate prompt"));
        assert_eq!(
            owner
                .active_prompt_for_agent_snapshot(&session, "agent-1")
                .as_ref()
                .map(|prompt| prompt.id()),
            Some("prompt-active")
        );
    }
}
