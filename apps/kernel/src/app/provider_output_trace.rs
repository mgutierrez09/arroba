use serde_json::{json, Value};

use crate::app::{ActiveTurnStore, DaemonApp, PromptActivityStore};
use crate::provider::{ProviderProcessServiceStore, ProviderPromptSignalBatch};
use crate::runtime::prompt_state::PromptStateOwner;
use crate::session::SessionStateStore;
use crate::terminal::TerminalOutputRecord;

pub(crate) struct ProviderOutputTrace {
    provider_store: ProviderProcessServiceStore,
    session_store: SessionStateStore,
    prompt_state_owner: PromptStateOwner,
    active_turns: ActiveTurnStore,
    prompt_activity: PromptActivityStore,
}

impl ProviderOutputTrace {
    pub(crate) fn new(
        app: &DaemonApp,
        provider_store: ProviderProcessServiceStore,
        active_turns: ActiveTurnStore,
        prompt_activity: PromptActivityStore,
    ) -> Self {
        Self {
            provider_store,
            session_store: app.sessions.clone(),
            prompt_state_owner: app.prompt_state_owner(),
            active_turns,
            prompt_activity,
        }
    }

    pub(crate) fn structured_poll_batch(
        &self,
        session_id: &str,
        provider_run_id: &str,
        source: &str,
        poll_result: &ProviderPromptSignalBatch,
    ) {
        if poll_result.chunks.is_empty()
            && poll_result.completions.is_empty()
            && poll_result.notices.is_empty()
            && !poll_result.prompt_completed
            && poll_result.terminal_failure.is_none()
            && poll_result.resolved_model.is_none()
            && poll_result.resolved_variant.is_none()
            && poll_result.resolved_usage_tokens_total.is_none()
            && poll_result.resolved_usage.is_none()
            && poll_result.resolved_resume_state.is_none()
        {
            return;
        }
        crate::debug_trace::record_terminal_turn(
            session_id,
            source,
            json!({
                "provider_run_id": provider_run_id,
                "prompt_completed": poll_result.prompt_completed,
                "terminal_failure": poll_result.terminal_failure.as_deref(),
                "completion_count": poll_result.completions.len(),
                "notice_count": poll_result.notices.len(),
                "chunk_count": poll_result.chunks.len(),
                "chunks": poll_result.chunks.iter().map(|chunk| {
                    json!({
                        "kind": &chunk.kind,
                        "merge_key": &chunk.merge_key,
                        "byte_len": chunk.bytes.len(),
                    })
                }).collect::<Vec<_>>(),
                "state": self.prompt_state(session_id, provider_run_id),
            }),
        );
    }

    pub(crate) fn terminal_records(
        &self,
        session_id: &str,
        provider_run_id: &str,
        source: &str,
        records: &[TerminalOutputRecord],
    ) {
        if records.is_empty() {
            return;
        }
        crate::debug_trace::record_terminal_turn(
            session_id,
            source,
            json!({
                "provider_run_id": provider_run_id,
                "record_count": records.len(),
                "records": records.iter().map(|record| {
                    json!({
                        "kind": &record.kind,
                        "agent_id": &record.agent_id,
                        "merge_key": &record.merge_key,
                        "byte_len": record.bytes.len(),
                        "pending_recipient_count": record.pending_recipient_attachment_ids.len(),
                    })
                }).collect::<Vec<_>>(),
                "state": self.prompt_state(session_id, provider_run_id),
            }),
        );
    }

    pub(crate) fn prompt_state_turn(&self, session_id: &str, provider_run_id: &str, source: &str) {
        crate::debug_trace::record_terminal_turn(
            session_id,
            source,
            json!({
                "provider_run_id": provider_run_id,
                "state": self.prompt_state(session_id, provider_run_id),
            }),
        );
    }

    fn prompt_state(&self, session_id: &str, provider_run_id: &str) -> Value {
        let provider_run = self.provider_store.get_run(provider_run_id).ok();
        let agent_id = provider_run
            .as_ref()
            .and_then(|run| run.agent_instance_id())
            .map(str::to_string);
        let session = self.session_store.get_session(session_id).ok();
        let active_prompt = match (session.as_ref(), agent_id.as_deref()) {
            (Some(session), Some(agent_id)) => self
                .prompt_state_owner
                .active_prompt_for_agent_snapshot(session, agent_id),
            _ => None,
        };
        let active_turn = self.active_turns.get(provider_run_id);
        let prompt_activity = self.prompt_activity.read().get(provider_run_id).cloned();
        json!({
            "agent_id": agent_id,
            "provider_run_state": provider_run.as_ref().map(|run| format!("{:?}", run.state())),
            "session_active_provider_run_id": session.as_ref().and_then(|session| session.active_provider_run_id()).map(str::to_string),
            "active_prompt": active_prompt.as_ref().map(|prompt| {
                json!({
                    "id": prompt.id().to_string(),
                    "status": prompt.status(),
                    "target_agent_id": prompt.target_agent_id().to_string(),
                    "workflow_run_id": prompt.workflow_run_id().map(str::to_string),
                    "workflow_node_run_id": prompt.workflow_node_run_id().map(str::to_string),
                })
            }),
            "active_turn": active_turn.map(|turn| {
                json!({
                    "agent_id": turn.agent_id,
                    "prompt_id": turn.prompt_id,
                    "provider_run_id": turn.provider_run_id,
                    "trace_id": turn.trace_id,
                    "started_at_ms": turn.started_at_ms,
                    "phase": turn.phase.as_str(),
                    "settlement_requested": turn.settlement_requested,
                })
            }),
            "prompt_activity": prompt_activity.map(|activity| {
                json!({
                    "last_output_seen": activity.last_output_at.is_some(),
                    "saw_response_content": activity.saw_response_content,
                    "completion_recorded": activity.completion_recorded,
                    "settlement_requested": activity.settlement_requested,
                })
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::KernelSessionService;
    use crate::session::CreateSessionRequest;

    #[test]
    fn prompt_state_trace_reads_prompt_owner_when_session_mirror_is_stale() {
        let mut app = crate::test_support::bootstrap_authenticated_app(
            crate::config::DaemonConfig::for_tests(),
        )
        .expect("daemon bootstrap should succeed");
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "workspace-trace-owner",
                "worktree-trace-owner",
            ))
            .expect("session should create");
        let run = app
            .launch_provider(
                crate::provider::LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "codex",
                    "default",
                    "gpt-test",
                )
                .with_agent_id(agent.id()),
            )
            .expect("provider run should launch");
        let external_prompt = crate::session::PromptQueueItem::external_observed_running(
            "codex",
            "thread-trace-owner",
            "turn-trace-owner",
            agent.id(),
            "external prompt visible in trace",
        );
        let external_prompt_id = external_prompt.id().to_string();
        app.prompt_owner_sync_external_active_prompt(
            session.id(),
            agent.id(),
            Some(external_prompt),
        )
        .expect("external active prompt should sync");
        app.sessions_mut()
            .mirror_agent_prompt_state(
                session.id(),
                agent.id(),
                None,
                std::collections::VecDeque::new(),
            )
            .expect("test drift should clear stale session prompt mirror");
        assert!(
            app.sessions()
                .get_session(session.id())
                .expect("session should load")
                .active_prompt_for_agent(agent.id())
                .is_none(),
            "session mirror should not expose the active prompt"
        );

        let trace = ProviderOutputTrace::new(
            &app,
            app.providers().clone(),
            app.active_turn_store(),
            app.prompt_activity_store(),
        );
        let state = trace.prompt_state(session.id(), run.id());

        assert_eq!(
            state
                .get("active_prompt")
                .and_then(|value| value.get("id"))
                .and_then(|value| value.as_str()),
            Some(external_prompt_id.as_str())
        );
        assert_eq!(
            state
                .get("active_prompt")
                .and_then(|value| value.get("target_agent_id"))
                .and_then(|value| value.as_str()),
            Some(agent.id())
        );
    }
}
