use std::collections::BTreeMap;
use std::sync::{Arc, Mutex as StdMutex, MutexGuard as StdMutexGuard};

use crate::provider::RuntimeProviderRun;

mod builder;
use builder::build_agent_context_handoff_from_history;

#[derive(Debug, Clone)]
pub(super) struct PendingAgentContextHandoff {
    pub(super) source_provider_run_id: String,
    pub(super) source_provider: String,
    pub(super) source_account_profile: String,
    pub(super) source_model: String,
    pub(super) target_provider_run_id: Option<String>,
    pub(super) target_provider: String,
    pub(super) target_account_profile: String,
    pub(super) target_model: Option<String>,
    pub(super) context: String,
}

#[derive(Debug, Clone, Default)]
pub(super) struct PendingAgentContextHandoffStore {
    inner: Arc<StdMutex<BTreeMap<String, PendingAgentContextHandoff>>>,
}

impl PendingAgentContextHandoffStore {
    fn write(&self) -> StdMutexGuard<'_, BTreeMap<String, PendingAgentContextHandoff>> {
        self.inner
            .lock()
            .expect("pending agent context handoff mutex poisoned")
    }

    pub(super) fn set(
        &self,
        session_id: &str,
        agent_id: &str,
        handoff: PendingAgentContextHandoff,
    ) {
        self.write()
            .insert(pending_handoff_key(session_id, agent_id), handoff);
    }

    pub(super) fn clear(&self, session_id: &str, agent_id: &str) {
        self.write()
            .remove(&pending_handoff_key(session_id, agent_id));
    }

    pub(super) fn peek_matching(
        &self,
        session_id: &str,
        agent_id: &str,
        target_run: &RuntimeProviderRun,
    ) -> Option<PendingAgentContextHandoff> {
        self.write()
            .get(&pending_handoff_key(session_id, agent_id))
            .filter(|handoff| handoff.matches_target(target_run))
            .cloned()
    }

    pub(super) fn consume_matching(
        &self,
        session_id: &str,
        agent_id: &str,
        target_run: &RuntimeProviderRun,
    ) -> Option<PendingAgentContextHandoff> {
        let key = pending_handoff_key(session_id, agent_id);
        let mut handoffs = self.write();
        if handoffs
            .get(&key)
            .is_some_and(|handoff| handoff.matches_target(target_run))
        {
            handoffs.remove(&key)
        } else {
            None
        }
    }
}

impl PendingAgentContextHandoff {
    fn matches_target(&self, target_run: &RuntimeProviderRun) -> bool {
        self.target_provider == target_run.provider()
            && self.target_account_profile == target_run.account_profile()
            && self
                .target_provider_run_id
                .as_deref()
                .is_none_or(|run_id| run_id == target_run.id())
            && self
                .target_model
                .as_deref()
                .is_none_or(|model| model == target_run.model())
    }
}

pub(super) fn inject_context_handoff(prompt: &str, handoff: &PendingAgentContextHandoff) -> String {
    crate::provider::encode_account_handoff(&context_handoff_for_provider(handoff), prompt)
}

impl super::KernelRuntimeOwnedState {
    pub(super) fn prepare_provider_switch_context_handoff(
        &self,
        source_run: &RuntimeProviderRun,
        target_run: &RuntimeProviderRun,
    ) {
        let Some(agent_id) = target_run.agent_instance_id() else {
            return;
        };
        if source_run.agent_instance_id() != Some(agent_id) {
            return;
        }
        if !requires_provider_context_handoff(source_run, target_run) {
            return;
        }
        self.prepare_provider_switch_context_handoff_for_target(
            source_run,
            target_run.session_id(),
            agent_id,
            agent_id,
            Some(target_run.id()),
            target_run.provider(),
            target_run.account_profile(),
            Some(target_run.model()),
        );
    }

    pub(super) fn prepare_agent_profile_context_handoff(
        &self,
        source_run: &RuntimeProviderRun,
        target_provider: &str,
        target_account_profile: &str,
        target_model: Option<&str>,
    ) {
        let Some(agent_id) = source_run.agent_instance_id() else {
            return;
        };
        if source_run.provider() == target_provider
            && target_model.is_some_and(|model| model == source_run.model())
            && source_run.account_profile() == target_account_profile
        {
            return;
        }
        self.prepare_provider_switch_context_handoff_for_target(
            source_run,
            source_run.session_id(),
            agent_id,
            agent_id,
            None,
            target_provider,
            target_account_profile,
            target_model,
        );
    }

    pub(super) fn prepare_agent_fork_context_handoff(
        &self,
        source_run: &RuntimeProviderRun,
        target_agent_id: &str,
        target_run: &RuntimeProviderRun,
    ) {
        if source_run.session_id() != target_run.session_id() {
            return;
        }
        let source_agent_id = source_run.agent_instance_id().unwrap_or(target_agent_id);
        self.prepare_provider_switch_context_handoff_for_target(
            source_run,
            target_run.session_id(),
            source_agent_id,
            target_agent_id,
            Some(target_run.id()),
            target_run.provider(),
            target_run.account_profile(),
            Some(target_run.model()),
        );
    }

    fn prepare_provider_switch_context_handoff_for_target(
        &self,
        source_run: &RuntimeProviderRun,
        session_id: &str,
        source_history_agent_id: &str,
        agent_id: &str,
        target_provider_run_id: Option<&str>,
        target_provider: &str,
        target_account_profile: &str,
        target_model: Option<&str>,
    ) {
        self.pending_agent_context_handoffs
            .clear(session_id, agent_id);
        match build_agent_context_handoff_from_history(
            &self.operational_history_store,
            session_id,
            source_history_agent_id,
        ) {
            Ok(Some(context)) => {
                self.pending_agent_context_handoffs.set(
                    session_id,
                    agent_id,
                    PendingAgentContextHandoff {
                        source_provider_run_id: source_run.id().to_string(),
                        source_provider: source_run.provider().to_string(),
                        source_account_profile: source_run.account_profile().to_string(),
                        source_model: source_run.model().to_string(),
                        target_provider_run_id: target_provider_run_id.map(str::to_string),
                        target_provider: target_provider.to_string(),
                        target_account_profile: target_account_profile.to_string(),
                        target_model: target_model.map(str::to_string),
                        context,
                    },
                );
            }
            Ok(None) => {}
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.provider_context_handoff",
                    "failed to prepare provider switch context handoff",
                    serde_json::json!({
                        "session_id": session_id,
                        "agent_id": agent_id,
                        "source_history_agent_id": source_history_agent_id,
                        "source_provider_run_id": source_run.id(),
                        "target_provider_run_id": target_provider_run_id,
                        "target_provider": target_provider,
                        "target_account_profile": target_account_profile,
                        "target_model": target_model,
                        "error": error.to_string(),
                    }),
                );
            }
        }
    }

    pub(super) fn prompt_with_pending_context_handoff(
        &self,
        session_id: &str,
        agent_id: &str,
        _source_attachment_id: &str,
        target_run: &RuntimeProviderRun,
        prompt: &str,
    ) -> String {
        self.pending_agent_context_handoffs
            .peek_matching(session_id, agent_id, target_run)
            .map(|handoff| inject_context_handoff(prompt, &handoff))
            .unwrap_or_else(|| prompt.to_string())
    }

    pub(super) fn hidden_context_with_pending_context_handoff(
        &self,
        session_id: &str,
        agent_id: &str,
        target_run: &RuntimeProviderRun,
        hidden_system_context: &str,
    ) -> String {
        let Some(handoff) = self
            .pending_agent_context_handoffs
            .peek_matching(session_id, agent_id, target_run)
        else {
            return hidden_system_context.to_string();
        };
        join_context_sections(
            hidden_system_context,
            &format!(
                "{}\n\nThe active user request is supplied separately.",
                context_handoff_for_provider(&handoff)
            ),
        )
    }

    pub(super) fn consume_pending_context_handoff(
        &self,
        session_id: &str,
        agent_id: &str,
        target_run: &RuntimeProviderRun,
    ) {
        let _ = self
            .pending_agent_context_handoffs
            .consume_matching(session_id, agent_id, target_run);
    }
}

fn requires_provider_context_handoff(
    source_run: &RuntimeProviderRun,
    target_run: &RuntimeProviderRun,
) -> bool {
    source_run.provider() != target_run.provider()
        || source_run.model() != target_run.model()
        || source_run.account_profile() != target_run.account_profile()
}

fn context_handoff_for_provider(handoff: &PendingAgentContextHandoff) -> String {
    format!(
        "{}\n\nProvider/account switch: {} [{}] ({}) from run {} -> {} [{}] ({}) run {}.",
        handoff.context.trim(),
        handoff.source_provider,
        handoff.source_account_profile,
        model_label(Some(&handoff.source_model)),
        handoff.source_provider_run_id,
        handoff.target_provider,
        handoff.target_account_profile,
        model_label(handoff.target_model.as_deref()),
        handoff
            .target_provider_run_id
            .as_deref()
            .unwrap_or("pending")
    )
}

fn join_context_sections(first: &str, second: &str) -> String {
    match (first.trim(), second.trim()) {
        ("", "") => String::new(),
        (first, "") => first.to_string(),
        ("", second) => second.to_string(),
        (first, second) => format!("{first}\n\n{second}"),
    }
}

fn pending_handoff_key(session_id: &str, agent_id: &str) -> String {
    format!("{session_id}\n{agent_id}")
}

fn model_label(model: Option<&str>) -> &str {
    let Some(model) = model else {
        return "unknown model";
    };
    if model.trim().is_empty() {
        "unknown model"
    } else {
        model
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::state::KernelRuntimeState;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    async fn owned_runtime_state(app: &Arc<Mutex<crate::DaemonApp>>) -> KernelRuntimeState {
        let (
            config_projection,
            session_state,
            agents,
            attachments,
            providers,
            provider_process_tracking,
            slices,
            session_state_projection,
            provider_run_projection,
            operational_history,
            durable_state,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        ) = {
            let app_locked = app.lock().await;
            (
                app_locked.config_projection_store(),
                app_locked.session_state_store(),
                app_locked.agents().clone(),
                app_locked.attachments().clone(),
                app_locked.providers().clone(),
                app_locked.provider_process_tracking_store(),
                app_locked.slices(),
                app_locked.session_state_projection_store(),
                app_locked.provider_run_projection_store(),
                app_locked.operational_history_store(),
                app_locked.durable_state_store(),
                app_locked.prompt_state_owner(),
                app_locked.active_turn_store(),
                app_locked.prompt_activity_store(),
                app_locked.prompt_workspace_claim_store(),
                app_locked.structured_output_record_store(),
                app_locked.terminal_stream_store(),
                app_locked.workflow_design_event_store(),
                app_locked.metaagent_event_store(),
                app_locked.workspace_coordinator(),
            )
        };
        KernelRuntimeState::new_with_owned_state(
            Arc::clone(app),
            config_projection,
            session_state,
            agents,
            attachments,
            providers,
            provider_process_tracking,
            slices,
            session_state_projection,
            provider_run_projection,
            operational_history,
            durable_state,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        )
    }

    #[test]
    fn pending_handoff_is_consumed_once() {
        let store = PendingAgentContextHandoffStore::default();
        store.set(
            "session",
            "agent",
            PendingAgentContextHandoff {
                source_provider_run_id: "run-old".to_string(),
                source_provider: "opencode".to_string(),
                source_account_profile: "default".to_string(),
                source_model: "model-old".to_string(),
                target_provider_run_id: Some("run-new".to_string()),
                target_provider: "codex".to_string(),
                target_account_profile: "default".to_string(),
                target_model: Some("model-new".to_string()),
                context: "<chariox_context_handoff>prior context</chariox_context_handoff>"
                    .to_string(),
            },
        );

        let target_run = test_run("run-new", "session", "agent", "codex", "model-new");
        let first = store.peek_matching("session", "agent", &target_run);
        let consumed = store.consume_matching("session", "agent", &target_run);
        let second = store.peek_matching("session", "agent", &target_run);

        assert!(first.is_some());
        assert!(consumed.is_some());
        assert!(second.is_none());
        let injected = inject_context_handoff("next request", &first.unwrap());
        assert!(injected.contains("prior context"));
        assert!(injected.contains("<user_request>\nnext request\n</user_request>"));
        assert_eq!(
            crate::provider::normalized_observed_prompt_text(&injected),
            Some("next request".to_string())
        );
    }

    #[test]
    fn same_provider_and_model_with_different_account_requires_handoff() {
        let source = test_run_with_account(
            "run-old", "session", "agent", "codex", "gpt-5.5", "personal",
        );
        let target =
            test_run_with_account("run-new", "session", "agent", "codex", "gpt-5.5", "work");

        assert!(requires_provider_context_handoff(&source, &target));
    }

    #[tokio::test]
    async fn fork_context_handoff_uses_source_agent_history_for_target_agent() {
        let mut app = crate::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, source_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-fork-handoff",
                "worktree-fork-handoff",
            ))
            .expect("session should be created");
        let forked_agent = app
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_alias("forked"),
            )
            .expect("fork target should spawn");
        let source_run = app
            .providers
            .start_run_provider_only(
                crate::provider::LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "dev-stub",
                    "default",
                    "model-source",
                )
                .with_agent_id(source_agent.id()),
            )
            .expect("source provider should start")
            .into_run();
        let target_run = app
            .providers
            .start_run_provider_only(
                crate::provider::LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "dev-stub",
                    "default",
                    "model-source",
                )
                .with_agent_id(forked_agent.id()),
            )
            .expect("target provider should start")
            .into_run();
        let history = app.operational_history_store();
        let prompt_entry = crate::history::SessionHistoryEntry::user_prompt(
            session.id(),
            "attachment-1",
            source_agent.id(),
            "remember source context",
        );
        history
            .append(&crate::history::HistoryEvent::transcript(
                1,
                &prompt_entry,
                crate::history::HistoryEventTurnContext {
                    provider: Some(source_run.provider().to_string()),
                    model: Some(source_run.model().to_string()),
                    provider_run_id: Some(source_run.id().to_string()),
                    prompt_id: Some("prompt-1".to_string()),
                    turn_id: Some("turn-1".to_string()),
                    ..crate::history::HistoryEventTurnContext::default()
                },
            ))
            .expect("source prompt history should append");
        let output_entry = crate::history::SessionHistoryEntry::provider_output(
            session.id(),
            source_run.id(),
            Some(source_agent.id()),
            crate::terminal::TerminalOutputKind::ProviderOutput,
            None,
            "source answer for fork handoff",
        );
        history
            .append(&crate::history::HistoryEvent::transcript(
                2,
                &output_entry,
                crate::history::HistoryEventTurnContext {
                    provider: Some(source_run.provider().to_string()),
                    model: Some(source_run.model().to_string()),
                    provider_run_id: Some(source_run.id().to_string()),
                    prompt_id: Some("prompt-1".to_string()),
                    turn_id: Some("turn-1".to_string()),
                    ..crate::history::HistoryEventTurnContext::default()
                },
            ))
            .expect("source output history should append");

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        runtime.owned.prepare_agent_fork_context_handoff(
            &source_run,
            forked_agent.id(),
            &target_run,
        );

        let handoff = runtime
            .owned
            .pending_agent_context_handoffs
            .peek_matching(session.id(), forked_agent.id(), &target_run)
            .expect("fork target should receive pending source context handoff");
        assert_eq!(handoff.source_provider_run_id, source_run.id());
        assert_eq!(
            handoff.target_provider_run_id.as_deref(),
            Some(target_run.id())
        );
        assert!(handoff.context.contains("remember source context"));
        assert!(handoff.context.contains("source answer for fork handoff"));
        assert!(runtime
            .owned
            .pending_agent_context_handoffs
            .peek_matching(session.id(), source_agent.id(), &target_run)
            .is_none());
    }

    #[test]
    fn handoff_injection_is_source_agnostic() {
        let store = PendingAgentContextHandoffStore::default();
        store.set(
            "session",
            "agent",
            PendingAgentContextHandoff {
                source_provider_run_id: "run-old".to_string(),
                source_provider: "opencode".to_string(),
                source_account_profile: "default".to_string(),
                source_model: "model-old".to_string(),
                target_provider_run_id: None,
                target_provider: "codex".to_string(),
                target_account_profile: "default".to_string(),
                target_model: None,
                context: "<chariox_context_handoff>workflow context</chariox_context_handoff>"
                    .to_string(),
            },
        );

        let target_run = test_run("run-new", "session", "agent", "codex", "model-new");
        let handoff = store.consume_matching("session", "agent", &target_run);

        assert!(handoff.is_some());
        let injected = inject_context_handoff("run workflow node", &handoff.unwrap());
        assert!(injected.contains("workflow context"));
        assert!(store
            .consume_matching("session", "agent", &target_run)
            .is_none());
    }

    #[test]
    fn handoff_hidden_context_does_not_duplicate_active_user_request() {
        let handoff = PendingAgentContextHandoff {
            source_provider_run_id: "run-old".to_string(),
            source_provider: "codex".to_string(),
            source_account_profile: "default".to_string(),
            source_model: "gpt-5.5".to_string(),
            target_provider_run_id: None,
            target_provider: "claude-headless".to_string(),
            target_account_profile: "default".to_string(),
            target_model: Some("claude-opus-4-7".to_string()),
            context: "<chariox_context_handoff>prior context</chariox_context_handoff>".to_string(),
        };
        let hidden = join_context_sections(
            "existing hidden context",
            &format!(
                "{}\n\nThe active user request is supplied separately.",
                context_handoff_for_provider(&handoff)
            ),
        );

        assert!(hidden.contains("existing hidden context"));
        assert!(hidden.contains("<chariox_context_handoff>prior context</chariox_context_handoff>"));
        assert!(hidden.contains("Provider/account switch: codex [default] (gpt-5.5)"));
        assert!(hidden.contains("The active user request is supplied separately."));
        assert!(!hidden.contains("<user_request>"));
    }

    #[test]
    fn handoff_ignores_mismatched_target_provider() {
        let store = PendingAgentContextHandoffStore::default();
        store.set(
            "session",
            "agent",
            PendingAgentContextHandoff {
                source_provider_run_id: "run-old".to_string(),
                source_provider: "claude".to_string(),
                source_account_profile: "default".to_string(),
                source_model: "opus".to_string(),
                target_provider_run_id: None,
                target_provider: "codex".to_string(),
                target_account_profile: "default".to_string(),
                target_model: Some("gpt-5".to_string()),
                context: "<chariox_context_handoff>prior context</chariox_context_handoff>"
                    .to_string(),
            },
        );

        let wrong_provider = test_run("run-new", "session", "agent", "opencode", "gpt-5");
        let right_provider = test_run("run-new", "session", "agent", "codex", "gpt-5");

        assert!(store
            .consume_matching("session", "agent", &wrong_provider)
            .is_none());
        assert!(store
            .consume_matching("session", "agent", &right_provider)
            .is_some());
    }

    fn test_run(
        run_id: &str,
        session_id: &str,
        agent_id: &str,
        provider: &str,
        model: &str,
    ) -> RuntimeProviderRun {
        test_run_with_account(run_id, session_id, agent_id, provider, model, "default")
    }

    fn test_run_with_account(
        run_id: &str,
        session_id: &str,
        agent_id: &str,
        provider: &str,
        model: &str,
        account_profile: &str,
    ) -> RuntimeProviderRun {
        let request = crate::provider::LaunchProviderRequest::new(
            session_id,
            "dev-stub",
            provider,
            account_profile,
            model,
        )
        .with_agent_id(agent_id);
        RuntimeProviderRun::new(
            run_id,
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: provider.to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: std::collections::BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        )
    }
}
