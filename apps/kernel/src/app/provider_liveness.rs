use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::{ProviderRunLivenessReconciliation, RuntimeProviderRun};
use crate::pty::PtyProcessState;
use crate::session::PromptStatus;

use super::provider_processes::ProviderProcessTracker;
use super::provider_run_read::ProviderRunReadService;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum ProviderRunExitPromptSettlement {
    FinalizeCancellation,
    CompleteActivePrompt,
    SyncIdleProvider,
}

impl ProviderRunExitPromptSettlement {
    fn from_active_prompt_status(active_prompt_status: Option<PromptStatus>) -> Self {
        match active_prompt_status {
            Some(PromptStatus::Cancelling) => Self::FinalizeCancellation,
            Some(_) => Self::CompleteActivePrompt,
            None => Self::SyncIdleProvider,
        }
    }
}

pub(crate) struct ProviderRunLivenessRuntime<'a> {
    app: &'a mut DaemonApp,
}

#[derive(Debug, Clone)]
struct ProviderRunLivenessOutcome {
    ended_run: RuntimeProviderRun,
    session_id: String,
    provider_run_id: String,
    agent_id: String,
    transition: ProviderRunLivenessTransition,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum ProviderRunLivenessTransition {
    AlreadyEnded,
    UnexpectedExit,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
struct ProviderRunExitSessionOutcome {
    had_active_prompt: bool,
    cancelled_prompt: bool,
    started_next_prompt: bool,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) struct ProviderRunExitSessionSummary {
    pub(crate) had_active_prompt: bool,
    pub(crate) cancelled_prompt: bool,
    pub(crate) started_next_prompt: bool,
}

struct ProviderRunLivenessProcesses;

impl ProviderRunLivenessProcesses {
    fn poll_process_running(
        app: &mut DaemonApp,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        match app.pty.poll_process_state(provider_run_id) {
            Ok(PtyProcessState::Running) => Ok(true),
            Ok(PtyProcessState::Exited) => Ok(false),
            Err(DaemonError::PtyProcessNotFound { .. }) => Ok(false),
            Err(error) => Err(error),
        }
    }

    fn remove_tracked_process(
        app: &mut DaemonApp,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        ProviderProcessTracker::new(app).remove_run(provider_run_id)
    }
}

struct ProviderRunLivenessState;

impl ProviderRunLivenessState {
    fn reconcile_run_liveness(
        app: &mut DaemonApp,
        session_id: &str,
        provider_run_id: &str,
        process_running: Option<bool>,
    ) -> Result<ProviderRunLivenessReconciliation, DaemonError> {
        let reconciliation = app.providers.reconcile_run_liveness_provider_only(
            session_id,
            provider_run_id,
            process_running,
        )?;
        Self::sync_ended_provider_run_session_state(
            app,
            session_id,
            provider_run_id,
            &reconciliation,
        )?;
        Ok(reconciliation)
    }

    fn sync_ended_provider_run_session_state(
        app: &mut DaemonApp,
        session_id: &str,
        provider_run_id: &str,
        reconciliation: &ProviderRunLivenessReconciliation,
    ) -> Result<(), DaemonError> {
        if !matches!(
            reconciliation,
            ProviderRunLivenessReconciliation::AlreadyEnded(_)
                | ProviderRunLivenessReconciliation::NewlyEnded(_)
        ) {
            return Ok(());
        }
        clear_active_provider_run_session_pointer(app, session_id, provider_run_id)
    }
}

pub(super) fn poll_provider_run_process_running(
    app: &mut DaemonApp,
    provider_run_id: &str,
) -> Result<bool, DaemonError> {
    ProviderRunLivenessProcesses::poll_process_running(app, provider_run_id)
}

pub(super) fn clear_active_provider_run_session_pointer(
    app: &mut DaemonApp,
    session_id: &str,
    provider_run_id: &str,
) -> Result<(), DaemonError> {
    if app
        .sessions
        .get_session(session_id)?
        .active_provider_run_id()
        == Some(provider_run_id)
    {
        app.sessions.set_active_provider_run(session_id, None)?;
    }
    Ok(())
}

struct ProviderRunLivenessNotices;

impl ProviderRunLivenessNotices {
    fn record_provider_exit(
        app: &mut DaemonApp,
        session_id: &str,
        provider_run_id: &str,
        message: String,
    ) {
        let recipients = app.attachments.list_session_attachment_ids(session_id);
        app.record_notice(session_id, Some(provider_run_id), recipients, message);
    }
}

struct ProviderRunLivenessSessionEffects;

impl ProviderRunLivenessSessionEffects {
    fn apply_provider_exit(
        app: &mut DaemonApp,
        outcome: &ProviderRunLivenessOutcome,
    ) -> Result<ProviderRunExitSessionOutcome, DaemonError> {
        let active_prompt_status = app
            .prompt_owner_active_prompt_for_agent(&outcome.session_id, &outcome.agent_id)?
            .and_then(|prompt| prompt.is_chariox_owned().then(|| prompt.status()));
        let had_active_prompt = active_prompt_status.is_some();
        let cancelled_prompt = active_prompt_status == Some(PromptStatus::Cancelling);
        let started_next_prompt = match ProviderRunExitPromptSettlement::from_active_prompt_status(
            active_prompt_status,
        ) {
            ProviderRunExitPromptSettlement::FinalizeCancellation => app
                .finalize_active_prompt_cancellation(
                    &outcome.session_id,
                    &outcome.agent_id,
                    Some(&outcome.provider_run_id),
                )?
                .started_next
                .is_some(),
            ProviderRunExitPromptSettlement::CompleteActivePrompt => app
                .complete_active_prompt(
                    &outcome.session_id,
                    &outcome.agent_id,
                    Some(&outcome.provider_run_id),
                )?
                .started_next
                .is_some(),
            ProviderRunExitPromptSettlement::SyncIdleProvider => {
                app.sync_focused_provider_run_if_idle(&outcome.session_id)?;
                false
            }
        };
        if !cancelled_prompt
            && app
                .agents
                .mark_unexpected_provider_exit_error(&outcome.agent_id, had_active_prompt)?
        {
            let _ = crate::app::KernelSessionReadService::new(app)
                .session_snapshot(&outcome.session_id)?;
        }

        Ok(ProviderRunExitSessionOutcome {
            had_active_prompt,
            cancelled_prompt,
            started_next_prompt,
        })
    }
}

impl<'a> ProviderRunLivenessRuntime<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn reconcile_provider_run_exit(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        let Some(outcome) =
            self.reconcile_provider_run_exit_provider_phase(session_id, provider_run_id)?
        else {
            return Ok(false);
        };
        if outcome.transition == ProviderRunLivenessTransition::AlreadyEnded {
            return Ok(true);
        }

        let session_outcome =
            ProviderRunLivenessSessionEffects::apply_provider_exit(self.app, &outcome)?;
        if session_outcome.cancelled_prompt {
            return Ok(true);
        }
        ProviderRunLivenessNotices::record_provider_exit(
            self.app,
            &outcome.session_id,
            &outcome.provider_run_id,
            format!(
                "Provider run `{}` for `{}` ended unexpectedly. {}",
                outcome.provider_run_id,
                outcome.ended_run.provider(),
                if session_outcome.had_active_prompt {
                    if session_outcome.started_next_prompt {
                        "The active prompt was closed and Chariox advanced the queued backlog onto the next available provider run."
                    } else {
                        "The active prompt was closed without starting the queued backlog."
                    }
                } else {
                    "No active prompt was running."
                }
            ),
        );

        Ok(true)
    }

    fn reconcile_provider_run_exit_provider_phase(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<Option<ProviderRunLivenessOutcome>, DaemonError> {
        let provider_run = ProviderRunReadService::new(self.app)
            .ensure_provider_run_in_session(session_id, provider_run_id)?;
        let agent_id = provider_run
            .agent_instance_id()
            .map(str::to_string)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: "provider run has no agent".to_string(),
            })?;
        match ProviderRunLivenessState::reconcile_run_liveness(
            self.app,
            session_id,
            provider_run_id,
            None,
        )? {
            ProviderRunLivenessReconciliation::AlreadyEnded(run) => {
                self.app.update_provider_run_projection(run.clone());
                let _ = ProviderRunLivenessProcesses::remove_tracked_process(
                    self.app,
                    provider_run_id,
                )?;
                return Ok(Some(ProviderRunLivenessOutcome {
                    ended_run: run,
                    session_id: session_id.to_string(),
                    provider_run_id: provider_run_id.to_string(),
                    agent_id,
                    transition: ProviderRunLivenessTransition::AlreadyEnded,
                }));
            }
            ProviderRunLivenessReconciliation::ExternalEndpoint(_)
            | ProviderRunLivenessReconciliation::NewlyEnded(_) => return Ok(None),
            ProviderRunLivenessReconciliation::StillRunning(_) => {}
        }

        let process_running =
            ProviderRunLivenessProcesses::poll_process_running(self.app, provider_run_id)?;
        let ended_run = match ProviderRunLivenessState::reconcile_run_liveness(
            self.app,
            session_id,
            provider_run_id,
            Some(process_running),
        )? {
            ProviderRunLivenessReconciliation::AlreadyEnded(run)
            | ProviderRunLivenessReconciliation::NewlyEnded(run) => run,
            ProviderRunLivenessReconciliation::ExternalEndpoint(_)
            | ProviderRunLivenessReconciliation::StillRunning(_) => return Ok(None),
        };
        self.app.update_provider_run_projection(ended_run.clone());
        let _ = ProviderRunLivenessProcesses::remove_tracked_process(self.app, provider_run_id)?;

        Ok(Some(ProviderRunLivenessOutcome {
            ended_run,
            session_id: session_id.to_string(),
            provider_run_id: provider_run_id.to_string(),
            agent_id,
            transition: ProviderRunLivenessTransition::UnexpectedExit,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::KernelSessionService;
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::provider::LaunchProviderRequest;
    use crate::session::CreateSessionRequest;

    #[test]
    fn app_unexpected_provider_exit_marks_active_agent_error() {
        assert_app_provider_exit_state(false);
    }

    #[test]
    fn app_cancelled_provider_exit_does_not_mark_agent_error() {
        assert_app_provider_exit_state(true);
    }

    fn assert_app_provider_exit_state(cancelling: bool) {
        let mut app =
            crate::test_support::bootstrap_authenticated_app(crate::DaemonConfig::for_tests())
                .expect("daemon should boot");
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "workspace-app-unexpected-exit",
                "worktree-app-unexpected-exit",
            ))
            .expect("session should create");
        let attachment = KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "client-app-unexpected-exit",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let run = app
            .launch_provider(
                LaunchProviderRequest::new(session.id(), "dev-stub", "codex", "default", "gpt-5")
                    .with_agent_id(agent.id()),
            )
            .expect("provider should launch");
        app.submit_prompt(
            session.id(),
            attachment.id(),
            Some(agent.id()),
            "do work\n",
            Vec::new(),
        )
        .expect("prompt should start");
        if cancelling {
            app.prompt_owner_begin_cancelling_active_prompt(session.id(), agent.id())
                .expect("cancellation should precede provider exit");
        }
        let ended_run = app
            .providers_mut()
            .mark_run_ended_provider_only(session.id(), run.id())
            .expect("provider should end")
            .into_run();
        let outcome = ProviderRunLivenessOutcome {
            ended_run,
            session_id: session.id().to_string(),
            provider_run_id: run.id().to_string(),
            agent_id: agent.id().to_string(),
            transition: ProviderRunLivenessTransition::UnexpectedExit,
        };

        let session_outcome =
            ProviderRunLivenessSessionEffects::apply_provider_exit(&mut app, &outcome)
                .expect("unexpected exit effects should apply");

        assert!(session_outcome.had_active_prompt);
        assert_eq!(session_outcome.cancelled_prompt, cancelling);
        assert!(app
            .sessions()
            .get_session(session.id())
            .expect("session should remain available")
            .active_prompt_for_agent(agent.id())
            .is_none());
        let state = app
            .agents
            .get_agent(agent.id())
            .expect("agent should remain available")
            .state();
        if cancelling {
            assert_ne!(state, crate::agent::AgentState::Error);
            let turn = app
                .completed_git_turn_snapshot_store()
                .latest_projection_for_agent(session.id(), agent.id())
                .expect("cancelled turn outcome should be projected even without Git changes");
            assert_eq!(
                turn.settlement_status,
                crate::git_observer::CompletedTurnSettlementStatus::Cancelled
            );
        } else {
            assert_eq!(state, crate::agent::AgentState::Error);
        }
    }
}
