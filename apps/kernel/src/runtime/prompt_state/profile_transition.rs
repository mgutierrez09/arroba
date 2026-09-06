//! Reserve an idle agent while its execution profile is being reconciled.
//! Admission and reservation use the same prompt-owner lock; no lock is held across I/O.

use super::*;

pub(crate) struct AgentProfileTransitionClaim {
    state: Arc<StdMutex<PromptStateOwnerState>>,
    key: PromptStateKey,
    identity: Arc<()>,
}

impl std::fmt::Debug for AgentProfileTransitionClaim {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentProfileTransitionClaim")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

impl AgentProfileTransitionClaim {
    /// Release admission and reserve the oldest queued turn in one owner transaction.
    /// No newly arriving prompt can pass the queue between these two operations.
    pub(crate) fn finish_and_activate_next(
        self,
        session: &RuntimeSession,
        agent_id: &str,
        prompt_id: String,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.key != PromptStateKey::new(session.id(), agent_id)
            || !owner
                .profile_transitions
                .get(&self.key)
                .is_some_and(|identity| Arc::ptr_eq(identity, &self.identity))
        {
            return Err(DaemonError::LocalTransport {
                operation: "finish agent profile transition",
                message: "profile reservation does not belong to the target agent".to_string(),
            });
        }
        let result = PromptStateOwner::activate_owned_queued_prompt(
            owner.ensure_agent_state(session, agent_id),
            agent_id,
            None,
            prompt_id,
        );
        owner.profile_transitions.remove(&self.key);
        result
    }
}

impl Drop for AgentProfileTransitionClaim {
    fn drop(&mut self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state
            .profile_transitions
            .get(&self.key)
            .is_some_and(|identity| Arc::ptr_eq(identity, &self.identity))
        {
            state.profile_transitions.remove(&self.key);
        }
    }
}

impl PromptStateOwner {
    pub(crate) fn complete_active_prompt_and_claim_profile_transition(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        expected_prompt_id: &str,
    ) -> Result<Option<(PromptQueueItem, AgentProfileTransitionClaim)>, DaemonError> {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = PromptStateKey::new(session.id(), agent_id);
        if owner
            .ensure_agent_state(session, agent_id)
            .active_prompt
            .as_ref()
            .map(PromptQueueItem::id)
            != Some(expected_prompt_id)
        {
            return Ok(None);
        }
        if owner.profile_transitions.contains_key(&key) {
            return Err(DaemonError::LocalTransport {
                operation: "settle prompt for agent profile transition",
                message: "the agent already has a profile change in progress".to_string(),
            });
        }
        let mut completed = owner
            .ensure_agent_state(session, agent_id)
            .active_prompt
            .take()
            .expect("exact active prompt checked under owner lock");
        completed.set_status(PromptStatus::Completed);
        let identity = Arc::new(());
        owner
            .profile_transitions
            .insert(key.clone(), identity.clone());
        Ok(Some((
            completed,
            AgentProfileTransitionClaim {
                state: self.state.clone(),
                key,
                identity,
            },
        )))
    }

    pub(crate) fn claim_idle_agent_profile_transition(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> Result<AgentProfileTransitionClaim, DaemonError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = PromptStateKey::new(session.id(), agent_id);
        if state.profile_transitions.contains_key(&key) {
            return Err(DaemonError::LocalTransport {
                operation: "update agent profile",
                message: "the agent already has a profile change in progress".to_string(),
            });
        }
        if state
            .ensure_agent_state(session, agent_id)
            .active_prompt
            .is_some()
        {
            return Err(DaemonError::LocalTransport {
                operation: "update agent profile",
                message: format!(
                    "agent `{agent_id}` has an active turn; update the profile after it finishes"
                ),
            });
        }
        let identity = Arc::new(());
        state
            .profile_transitions
            .insert(key.clone(), identity.clone());
        Ok(AgentProfileTransitionClaim {
            state: self.state.clone(),
            key,
            identity,
        })
    }

    /// Serialize a list-only profile edit without requiring the current turn to stop.
    /// New prompt admission waits briefly behind the edit, while an already active turn
    /// remains authoritative and is not disturbed.
    pub(crate) fn claim_agent_profile_list_edit(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> Result<AgentProfileTransitionClaim, DaemonError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = PromptStateKey::new(session.id(), agent_id);
        if state.profile_transitions.contains_key(&key) {
            return Err(DaemonError::LocalTransport {
                operation: "update agent profile",
                message: "the agent already has a profile change in progress".to_string(),
            });
        }
        let identity = Arc::new(());
        state
            .profile_transitions
            .insert(key.clone(), identity.clone());
        Ok(AgentProfileTransitionClaim {
            state: self.state.clone(),
            key,
            identity,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_prompt_completion_reserves_profile_before_admitting_followup() {
        let owner = PromptStateOwner::default();
        let session =
            RuntimeSession::new("room", None, "workspace", "worktree", "machine", "kernel");
        let first = PromptQueueItem::new(
            "failed-turn",
            "client",
            "agent-1",
            "first",
            PromptStatus::Queued,
        );
        assert!(matches!(
            owner
                .submit_prepared_prompt(&session, first, false)
                .unwrap(),
            PromptSubmissionOutcome::Started { .. }
        ));
        assert!(owner
            .complete_active_prompt_and_claim_profile_transition(&session, "agent-1", "stale-turn")
            .unwrap()
            .is_none());
        assert_eq!(
            owner
                .active_prompt_for_agent(&session, "agent-1")
                .unwrap()
                .id(),
            "failed-turn"
        );

        let (completed, claim) = owner
            .complete_active_prompt_and_claim_profile_transition(&session, "agent-1", "failed-turn")
            .unwrap()
            .unwrap();
        assert_eq!(completed.id(), "failed-turn");
        assert_eq!(completed.status(), PromptStatus::Completed);
        assert!(owner.active_prompt_for_agent(&session, "agent-1").is_none());
        assert!(matches!(
            owner
                .submit_prepared_prompt(
                    &session,
                    PromptQueueItem::new(
                        "followup",
                        "client",
                        "agent-1",
                        "second",
                        PromptStatus::Queued
                    ),
                    false,
                )
                .unwrap(),
            PromptSubmissionOutcome::Queued { .. }
        ));
        assert!(owner
            .activate_next_queued_prompt_with_prompt_id(
                &session,
                "agent-1",
                None,
                "must-not-start".into(),
            )
            .unwrap()
            .is_none());
        let next = claim
            .finish_and_activate_next(&session, "agent-1", "next-turn".into())
            .unwrap()
            .unwrap();
        assert_eq!(next.id(), "next-turn");
        assert_eq!(next.prompt(), "second");
    }

    #[test]
    fn profile_completion_reserves_oldest_prompt_before_new_admission() {
        let owner = PromptStateOwner::default();
        let session =
            RuntimeSession::new("room", None, "workspace", "worktree", "machine", "kernel");
        let claim = owner
            .claim_idle_agent_profile_transition(&session, "agent-1")
            .unwrap();
        for text in ["first", "second"] {
            assert!(matches!(
                owner
                    .submit_prepared_prompt(
                        &session,
                        PromptQueueItem::new(text, "client", "agent-1", text, PromptStatus::Queued)
                            .with_workflow_context(format!("run-{text}"), format!("node-{text}"))
                            .with_source_attribution("caller-client", "caller-user")
                            .with_hidden_system_context("private-context")
                            .with_durable_operation(
                                format!("operation-{text}"),
                                format!("fingerprint-{text}")
                            ),
                        false,
                    )
                    .unwrap(),
                PromptSubmissionOutcome::Queued { .. }
            ));
        }
        let started = claim
            .finish_and_activate_next(&session, "agent-1", "started".into())
            .unwrap()
            .unwrap();
        assert_eq!(started.prompt(), "first");
        assert_eq!(started.id(), "started");
        assert_eq!(started.workflow_run_id(), Some("run-first"));
        assert_eq!(started.workflow_node_run_id(), Some("node-first"));
        assert_eq!(started.source_client_id(), Some("caller-client"));
        assert_eq!(started.source_user_id(), Some("caller-user"));
        assert_eq!(started.hidden_system_context(), "private-context");
        assert_eq!(started.durable_operation_id(), Some("operation-first"));
        assert_eq!(
            started.durable_operation_fingerprint(),
            Some("fingerprint-first")
        );
        assert!(matches!(
            owner
                .submit_prepared_prompt(
                    &session,
                    PromptQueueItem::new(
                        "third",
                        "client",
                        "agent-1",
                        "third",
                        PromptStatus::Queued
                    )
                    .with_workflow_context("run-third", "node-third"),
                    false,
                )
                .unwrap(),
            PromptSubmissionOutcome::Queued { .. }
        ));
        let (active, queued) = owner.state_parts(&session, "agent-1");
        assert_eq!(active.unwrap().prompt(), "first");
        assert_eq!(
            queued
                .iter()
                .map(PromptQueueItem::prompt)
                .collect::<Vec<_>>(),
            vec!["second", "third"]
        );
        assert!(!owner
            .state
            .lock()
            .unwrap()
            .profile_transitions
            .contains_key(&PromptStateKey::new(session.id(), "agent-1")));
    }

    #[test]
    fn empty_profile_completion_reopens_admission() {
        let owner = PromptStateOwner::default();
        let session =
            RuntimeSession::new("room", None, "workspace", "worktree", "machine", "kernel");
        let claim = owner
            .claim_idle_agent_profile_transition(&session, "agent-1")
            .unwrap();
        assert!(claim
            .finish_and_activate_next(&session, "agent-1", "unused".into())
            .unwrap()
            .is_none());
        assert!(matches!(
            owner
                .submit_prepared_prompt(
                    &session,
                    PromptQueueItem::new(
                        "first",
                        "client",
                        "agent-1",
                        "first",
                        PromptStatus::Queued
                    ),
                    false,
                )
                .unwrap(),
            PromptSubmissionOutcome::Started { .. }
        ));
    }

    #[test]
    fn profile_completion_cannot_release_another_agents_reservation() {
        let owner = PromptStateOwner::default();
        let session =
            RuntimeSession::new("room", None, "workspace", "worktree", "machine", "kernel");
        let first = owner
            .claim_idle_agent_profile_transition(&session, "agent-1")
            .unwrap();
        let second = owner
            .claim_idle_agent_profile_transition(&session, "agent-2")
            .unwrap();
        assert!(first
            .finish_and_activate_next(&session, "agent-2", "wrong".into())
            .is_err());
        assert!(owner
            .claim_idle_agent_profile_transition(&session, "agent-2")
            .is_err());
        assert!(second
            .finish_and_activate_next(&session, "agent-2", "empty".into())
            .unwrap()
            .is_none());
    }

    #[test]
    fn profile_reservation_blocks_only_its_agent_and_preserves_queued_work() {
        let owner = PromptStateOwner::default();
        let session =
            RuntimeSession::new("room", None, "workspace", "worktree", "machine", "kernel");
        let claim = owner
            .claim_idle_agent_profile_transition(&session, "agent-1")
            .unwrap();
        assert!(owner
            .claim_idle_agent_profile_transition(&session, "agent-1")
            .is_err());
        let queued = owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new("first", "client", "agent-1", "first", PromptStatus::Queued),
                false,
            )
            .unwrap();
        let PromptSubmissionOutcome::Queued { prompt } = queued else {
            panic!("reserved agent must queue");
        };
        assert!(owner
            .activate_next_queued_prompt_with_prompt_id(
                &session,
                "agent-1",
                Some(prompt.id()),
                "started".into()
            )
            .unwrap()
            .is_none());
        assert!(matches!(
            owner
                .submit_prepared_prompt(
                    &session,
                    PromptQueueItem::new(
                        "other",
                        "client",
                        "agent-2",
                        "other",
                        PromptStatus::Queued
                    ),
                    false
                )
                .unwrap(),
            PromptSubmissionOutcome::Started { .. }
        ));
        assert!(owner
            .claim_idle_agent_profile_transition(&session, "agent-2")
            .is_err());
        drop(claim);
        let started = owner
            .activate_next_queued_prompt_with_prompt_id(
                &session,
                "agent-1",
                Some(prompt.id()),
                "started".into(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(started.prompt(), "first");
        assert_eq!(started.target_agent_id(), "agent-1");
    }
}
