use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::{
    LaunchProviderRequest, ProviderClientInterface, ProviderRunState, RuntimeProviderRun,
};

use super::provider_liveness::clear_active_provider_run_session_pointer;

#[derive(Debug, Clone)]
pub(crate) struct StartedProviderLaunch {
    pub(crate) run: RuntimeProviderRun,
    pub(crate) previous_active_run_id: Option<String>,
    pub(crate) provider_credential_env: crate::provider::ProviderCredentialEnvironment,
}

pub(super) struct ProviderRunActivationState;

impl ProviderRunActivationState {
    pub(super) fn reusable_native_tui_run_for_launch(
        app: &mut DaemonApp,
        request: &LaunchProviderRequest,
    ) -> Result<Option<RuntimeProviderRun>, DaemonError> {
        if request.client_interface != ProviderClientInterface::NativeTui {
            return Ok(None);
        }
        let Some(agent_id) = request.agent_id.as_deref() else {
            return Ok(None);
        };
        let Some(run) = app
            .providers()
            .list_runs()
            .into_iter()
            .filter(|run| {
                run.session_id() == request.session_id
                    && run.agent_instance_id() == Some(agent_id)
                    && run.client_interface() == ProviderClientInterface::NativeTui
                    && matches!(
                        run.state(),
                        ProviderRunState::Starting | ProviderRunState::Running
                    )
            })
            .max_by(|left, right| left.active_selection_cmp(right))
        else {
            return Ok(None);
        };

        if !request.matches_existing_run_selection(&run) {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: run.id().to_string(),
                state: run.state(),
                operation: "launch native TUI provider run with different parameters",
            });
        }

        app.sessions
            .set_active_provider_run(&request.session_id, Some(run.id().to_string()))?;
        app.update_provider_run_projection(run.clone());
        Ok(Some(run))
    }

    pub(super) fn start_provider_run_for_session(
        app: &mut DaemonApp,
        mut request: LaunchProviderRequest,
    ) -> Result<StartedProviderLaunch, DaemonError> {
        let session_id = request.session_id.clone();
        let mut previous_active_run_id = app
            .sessions
            .get_session(&session_id)?
            .active_provider_run_id()
            .map(str::to_owned);

        if let Some(active_run_id) = previous_active_run_id.as_deref() {
            let (active_run, locally_owned) =
                Self::provider_run_for_activation(app, &session_id, active_run_id)?;
            if locally_owned {
                match active_run.state() {
                    ProviderRunState::Ended => {
                        app.sessions.set_active_provider_run(&session_id, None)?;
                        app.providers.clear_runtime(active_run_id);
                    }
                    ProviderRunState::Starting => {
                        if active_run.client_interface().is_chariox() {
                            let outcome = app
                                .providers
                                .terminate_run_provider_only(&session_id, active_run_id)?;
                            clear_active_provider_run_session_pointer(
                                app,
                                &session_id,
                                outcome.run().id(),
                            )?;
                            app.update_provider_run_projection(outcome.into_run());
                        }
                    }
                    ProviderRunState::Running => {
                        if active_run.client_interface().is_chariox()
                            && !app.provider_run_has_active_prompt(&session_id, &active_run)?
                        {
                            let outcome = app
                                .providers
                                .park_run_provider_only(&session_id, active_run_id)?;
                            clear_active_provider_run_session_pointer(
                                app,
                                &session_id,
                                outcome.run().id(),
                            )?;
                            app.update_provider_run_projection(outcome.into_run());
                        }
                    }
                    ProviderRunState::Parked => {
                        app.sessions.set_active_provider_run(&session_id, None)?;
                    }
                }
            }
        }

        for run in Self::active_chariox_runs_for_target_agent(app, &request) {
            if run.state() == ProviderRunState::Running
                && app.provider_run_has_active_prompt(&session_id, &run)?
            {
                return Err(DaemonError::InvalidProviderRunState {
                    provider_run_id: run.id().to_string(),
                    state: run.state(),
                    operation: "replace agent provider run",
                });
            }
        }

        for run in Self::active_chariox_runs_for_target_agent(app, &request) {
            if previous_active_run_id.as_deref() == Some(run.id()) {
                continue;
            }
            let outcome = app
                .providers
                .terminate_run_provider_only(&session_id, run.id())?;
            clear_active_provider_run_session_pointer(app, &session_id, outcome.run().id())?;
            if previous_active_run_id.as_deref() == Some(outcome.run().id()) {
                previous_active_run_id = None;
            }
            app.update_provider_run_projection(outcome.into_run());
        }

        let provider_credential_env = std::mem::take(&mut request.provider_credential_env);
        let outcome = app.providers.start_run_provider_only(request)?;
        app.sessions
            .set_active_provider_run(&session_id, Some(outcome.run().id().to_string()))?;
        Ok(StartedProviderLaunch {
            run: outcome.into_run(),
            previous_active_run_id,
            provider_credential_env,
        })
    }

    fn active_chariox_runs_for_target_agent(
        app: &DaemonApp,
        request: &LaunchProviderRequest,
    ) -> Vec<RuntimeProviderRun> {
        let Some(agent_id) = request.agent_id.as_deref() else {
            return Vec::new();
        };
        if request.client_interface != ProviderClientInterface::Chariox {
            return Vec::new();
        }
        app.providers()
            .list_runs()
            .into_iter()
            .filter(|run| {
                run.session_id() == request.session_id
                    && run.agent_instance_id() == Some(agent_id)
                    && run.client_interface() == ProviderClientInterface::Chariox
                    && run.state() != ProviderRunState::Ended
            })
            .collect()
    }

    pub(super) fn resume_provider_run_for_session(
        app: &mut DaemonApp,
        session_id: &str,
        run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let active_run_id = app
            .sessions
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_owned);

        if let Some(active_run_id) = active_run_id.as_deref() {
            if active_run_id != run_id {
                let (active_run, locally_owned) =
                    Self::provider_run_for_activation(app, session_id, active_run_id)?;
                if locally_owned {
                    match active_run.state() {
                        ProviderRunState::Running => {
                            if !app.provider_run_has_active_prompt(session_id, &active_run)? {
                                let outcome = app
                                    .providers
                                    .park_run_provider_only(session_id, active_run_id)?;
                                clear_active_provider_run_session_pointer(
                                    app,
                                    session_id,
                                    outcome.run().id(),
                                )?;
                                app.update_provider_run_projection(outcome.into_run());
                            }
                        }
                        ProviderRunState::Starting => {
                            let outcome = app
                                .providers
                                .terminate_run_provider_only(session_id, active_run_id)?;
                            clear_active_provider_run_session_pointer(
                                app,
                                session_id,
                                outcome.run().id(),
                            )?;
                            app.update_provider_run_projection(outcome.into_run());
                        }
                        ProviderRunState::Parked | ProviderRunState::Ended => {
                            app.sessions.set_active_provider_run(session_id, None)?;
                        }
                    }
                }
            }
        }

        let (target_run, locally_owned) =
            Self::provider_run_for_activation(app, session_id, run_id)?;
        if !locally_owned {
            if !matches!(
                target_run.state(),
                ProviderRunState::Starting | ProviderRunState::Running
            ) {
                return Err(DaemonError::InvalidProviderRunState {
                    provider_run_id: run_id.to_string(),
                    state: target_run.state(),
                    operation: "restore projected provider run",
                });
            }
            app.sessions
                .set_active_provider_run(session_id, Some(run_id.to_string()))?;
            let _ = crate::app::KernelSessionReadService::new(app).session_snapshot(session_id)?;
            return Ok(target_run);
        }

        let outcome = app.providers.resume_run_provider_only(session_id, run_id)?;
        app.sessions
            .set_active_provider_run(session_id, Some(outcome.run().id().to_string()))?;
        let run = outcome.into_run();
        app.update_provider_run_projection(run.clone());
        Ok(run)
    }

    fn provider_run_for_activation(
        app: &DaemonApp,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<(RuntimeProviderRun, bool), DaemonError> {
        let result = match app.providers.get_run(provider_run_id) {
            Ok(run) => Ok((run, true)),
            Err(DaemonError::ProviderRunNotFound { .. }) => app
                .provider_run_projection
                .get(provider_run_id)
                .map(|run| (run, false))
                .ok_or_else(|| DaemonError::ProviderRunNotFound {
                    provider_run_id: provider_run_id.to_string(),
                }),
            Err(error) => Err(error),
        };
        result.and_then(|(run, locally_owned)| {
            if run.session_id() == session_id {
                Ok((run, locally_owned))
            } else {
                Err(DaemonError::ProviderRunNotInSession {
                    session_id: session_id.to_string(),
                    provider_run_id: provider_run_id.to_string(),
                })
            }
        })
    }
}
