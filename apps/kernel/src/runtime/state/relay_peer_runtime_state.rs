use crate::app::RemoteLeaseRuntime;
use crate::execution_lease::{ExecutionLease, LeasedAgent, RemoteWorkflowTurnContext};
use crate::provider::{AgentExecutionMode, AgentPermissionLevel};
use crate::runtime::projection::SessionSnapshotProjection;
use crate::runtime_transport::WatchResult;
use crate::skill::CharioxSkillPackage;
use crate::transport::relay_peer::{
    RelayPeerEvent, RelayProjectedCompletion, RelayProjectedOutputChunk, RelayProjectedPrompt,
    RelayPromptAttachment, RemoteGitObservation, RemoteGitTurnContext, RemoteMcpAvailability,
    RemoteMcpCheckContext, RemoteSkillMaterialization, RemoteSkillSyncContext, RequiredRemoteMcp,
};

use super::*;

impl KernelRuntimeState {
    pub(crate) async fn relay_registration(&self) -> chariox_relay::protocol::DaemonRegistration {
        self.with_app_side_effect(|app| app.relay_registration())
            .await
    }

    pub(crate) async fn remote_native_interaction_context(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<
        Option<(
            crate::config::DaemonConfig,
            String,
            crate::transport::relay_peer::RemoteNativeInteractionContext,
        )>,
        DaemonError,
    > {
        let session_id = session_id.to_string();
        let agent_id = agent_id.to_string();
        self.with_app_side_effect(move |app| {
            let target = RemoteLeaseRuntime::new(app).native_interaction_context_for_backing_agent(
                &session_id,
                &agent_id,
                "unknown",
            );
            Ok::<_, DaemonError>(
                target.map(|(daemon_id, context)| (app.config().clone(), daemon_id, context)),
            )
        })
        .await
    }

    pub(crate) async fn watch_relay_subscription_state(
        &self,
        session_id: &str,
        attachment_id: &str,
        should_check_snapshot: bool,
        previous_snapshot: Option<SessionSnapshotProjection>,
        last_workflow_design_sequence: u64,
    ) -> WatchResult {
        let session_id = session_id.to_string();
        let attachment_id = attachment_id.to_string();
        let previous_snapshot_for_compare = previous_snapshot.clone();
        let mut result = self
            .with_app_side_effect({
                let session_id = session_id.clone();
                let attachment_id = attachment_id.clone();
                move |app| {
                    crate::runtime_transport::watch_subscription_state(
                        app,
                        &session_id,
                        &attachment_id,
                        false,
                        None,
                        last_workflow_design_sequence,
                    )
                }
            })
            .await;
        if !should_check_snapshot {
            return result;
        }
        let projected_snapshot = match self.session_snapshot_projection_for_attachment(
            &session_id,
            &attachment_id,
            self.session_projection_change_sequence(),
        ) {
            Ok(mut snapshot) => {
                snapshot.metadata.last_event_id = self.session_projection_change_sequence();
                Box::new(
                    (previous_snapshot_for_compare.as_ref() != Some(&snapshot)).then_some(snapshot),
                )
            }
            Err(DaemonError::SessionNotFound { .. })
            | Err(DaemonError::AttachmentNotFound { .. })
            | Err(DaemonError::AttachmentNotInSession { .. }) => {
                return WatchResult::Unavailable(
                    "Current session is no longer available.".to_string(),
                );
            }
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.runtime_transport",
                    "kernel event loop failed to build owned session snapshot",
                    serde_json::json!({
                        "session_id": session_id,
                        "attachment_id": attachment_id,
                        "error": error.to_string(),
                    }),
                );
                Box::new(None)
            }
        };
        if let WatchResult::Ok { snapshot, .. } = &mut result {
            *snapshot = projected_snapshot;
        }
        result
    }

    pub(crate) async fn create_relay_execution_lease(
        &self,
        home_kernel_id: &str,
        home_session_id: &str,
        home_agent_id: &str,
        home_agent_metaagent: bool,
        owner_user_id: &str,
    ) -> Result<ExecutionLease, DaemonError> {
        let home_kernel_id = home_kernel_id.to_string();
        let home_session_id = home_session_id.to_string();
        let home_agent_id = home_agent_id.to_string();
        let owner_user_id = owner_user_id.to_string();
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).create_execution_lease(
                &home_kernel_id,
                &home_session_id,
                &home_agent_id,
                home_agent_metaagent,
                &owner_user_id,
            )
        })
        .await
    }

    pub(crate) async fn destroy_relay_execution_lease(
        &self,
        lease_id: &str,
    ) -> Result<ExecutionLease, DaemonError> {
        let lease_id = lease_id.to_string();
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).destroy_execution_lease(&lease_id)
        })
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn create_relay_leased_agent(
        &self,
        lease_id: &str,
        provider: &str,
        account_profile: &str,
        model: Option<String>,
        effort: Option<String>,
        execution_mode: Option<AgentExecutionMode>,
        permission_level: Option<AgentPermissionLevel>,
        workspace_live_sync_mode: Option<crate::config::WorkspaceLiveSyncMode>,
        worktree_id: Option<String>,
        worktree_placement: Option<crate::agent::GitWorktreePlacement>,
    ) -> Result<LeasedAgent, DaemonError> {
        let lease_id = lease_id.to_string();
        let provider = provider.to_string();
        let account_profile = account_profile.to_string();
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).create_leased_agent(
                &lease_id,
                &provider,
                &account_profile,
                model,
                effort,
                execution_mode,
                permission_level,
                workspace_live_sync_mode,
                worktree_id,
                worktree_placement,
            )
        })
        .await
    }

    pub(crate) async fn destroy_relay_leased_agent(
        &self,
        leased_agent_id: &str,
    ) -> Result<LeasedAgent, DaemonError> {
        let leased_agent_id = leased_agent_id.to_string();
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).destroy_leased_agent(&leased_agent_id)
        })
        .await
    }

    pub(crate) async fn update_relay_leased_agent_config(
        &self,
        leased_agent_id: &str,
        execution_mode: AgentExecutionMode,
        permission_level: AgentPermissionLevel,
    ) -> Result<LeasedAgent, DaemonError> {
        let leased_agent_id = leased_agent_id.to_string();
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).update_leased_agent_config(
                &leased_agent_id,
                execution_mode,
                permission_level,
            )
        })
        .await
    }

    pub(crate) async fn update_relay_leased_agent_profile(
        &self,
        leased_agent_id: &str,
        provider: String,
        account_profile: String,
        model: Option<String>,
        effort: Option<String>,
    ) -> Result<LeasedAgent, DaemonError> {
        let _operation = self.leased_agent_operations.lock(leased_agent_id).await;
        let leased_agent_id = leased_agent_id.to_string();
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).update_leased_agent_profile(
                &leased_agent_id,
                provider,
                account_profile,
                model,
                effort,
            )
        })
        .await
    }

    pub(crate) async fn update_relay_leased_agent_meta_mode(
        &self,
        leased_agent_id: &str,
        active: bool,
    ) -> Result<LeasedAgent, DaemonError> {
        let leased_agent_id = leased_agent_id.to_string();
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).update_leased_agent_meta_mode(&leased_agent_id, active)
        })
        .await
    }

    pub(crate) async fn update_relay_leased_agent_remote_extension_manifest(
        &self,
        leased_agent_id: &str,
        remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    ) -> Result<(), DaemonError> {
        let leased_agent_id = leased_agent_id.to_string();
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).update_leased_agent_remote_extension_manifest(
                &leased_agent_id,
                remote_extension_manifest,
            )
        })
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn launch_relay_leased_native_provider_run(
        &self,
        leased_agent_id: &str,
        adapter_key: &str,
        provider: &str,
        account_profile: &str,
        model: &str,
        variant: Option<String>,
        structured_endpoint: Option<String>,
        provider_session_id: Option<String>,
        required_mcps: Vec<RequiredRemoteMcp>,
        required_skills: Option<Vec<crate::transport::relay_peer::RequiredRemoteSkill>>,
        remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    ) -> Result<crate::provider::RuntimeProviderRun, DaemonError> {
        let leased_agent_id = leased_agent_id.to_string();
        let adapter_key = adapter_key.to_string();
        let provider = provider.to_string();
        let account_profile = account_profile.to_string();
        let model = model.to_string();
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).launch_leased_native_provider_run(
                &leased_agent_id,
                &adapter_key,
                &provider,
                &account_profile,
                &model,
                variant,
                structured_endpoint,
                provider_session_id,
                required_mcps,
                required_skills,
                remote_extension_manifest,
            )
        })
        .await
    }

    pub(crate) async fn send_relay_leased_native_provider_input(
        &self,
        leased_agent_id: &str,
        provider_run_id: &str,
        attachment_id: &str,
        data_base64: &str,
    ) -> Result<usize, DaemonError> {
        let leased_agent_id = leased_agent_id.to_string();
        let provider_run_id = provider_run_id.to_string();
        let attachment_id = attachment_id.to_string();
        let data_base64 = data_base64.to_string();
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).send_leased_native_provider_input(
                &leased_agent_id,
                &provider_run_id,
                &attachment_id,
                &data_base64,
            )
        })
        .await
    }

    pub(crate) async fn resize_relay_leased_provider_terminal(
        &self,
        leased_agent_id: &str,
        provider_run_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), DaemonError> {
        let leased_agent_id = leased_agent_id.to_string();
        let provider_run_id = provider_run_id.to_string();
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).resize_leased_provider_terminal(
                &leased_agent_id,
                &provider_run_id,
                cols,
                rows,
            )
        })
        .await
    }

    // Keep the relay request fields explicit, like the adjacent lease adapters.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn submit_relay_leased_prompt(
        &self,
        leased_agent_id: &str,
        expected_profile: crate::transport::relay_peer::RelayAgentExecutionProfile,
        prompt: &str,
        hidden_system_context: &str,
        attachments: Vec<RelayPromptAttachment>,
        workflow_context: Option<RemoteWorkflowTurnContext>,
        git_context: Option<RemoteGitTurnContext>,
        required_mcps: Vec<RequiredRemoteMcp>,
        required_skills: Option<Vec<crate::transport::relay_peer::RequiredRemoteSkill>>,
        remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    ) -> Result<(String, crate::session::PromptSubmissionOutcome), DaemonError> {
        let _operation = self.leased_agent_operations.lock(leased_agent_id).await;
        let leased_agent_id = leased_agent_id.to_string();
        let prompt = prompt.to_string();
        let hidden_system_context = hidden_system_context.to_string();
        let replay_leased_agent_id = leased_agent_id.clone();
        let replay_git_context = git_context.clone();
        if let Some(replayed) = self
            .with_app_side_effect(move |app| {
                // A prior profile acknowledgement may have been lost. Home remains
                // authoritative, including when this is a retry of an active prompt.
                RemoteLeaseRuntime::new(app).update_leased_agent_profile(
                    &replay_leased_agent_id,
                    expected_profile.provider,
                    expected_profile.account_profile,
                    expected_profile.model,
                    expected_profile.effort,
                )?;
                RemoteLeaseRuntime::new(app).replay_active_leased_prompt_submission(
                    &replay_leased_agent_id,
                    replay_git_context.as_ref(),
                )
            })
            .await?
        {
            return Ok(replayed);
        }
        let prepared = self
            .with_app_side_effect(move |app| {
                RemoteLeaseRuntime::new(app).prepare_leased_prompt_submission(
                    &leased_agent_id,
                    &prompt,
                    &hidden_system_context,
                    attachments,
                    workflow_context,
                    git_context,
                    required_mcps,
                    required_skills,
                    remote_extension_manifest,
                )
            })
            .await?;
        let provider_run_id = match &prepared.provider_run {
            crate::app::PreparedLeasedProviderRun::Ready(provider_run_id) => {
                provider_run_id.clone()
            }
            crate::app::PreparedLeasedProviderRun::LaunchRequired(request) => self
                .launch_provider_for_remote_lease_detached(request.clone())
                .await?
                .id()
                .to_string(),
        };
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app)
                .finish_prepared_leased_prompt_submission(prepared, provider_run_id)
        })
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn steer_relay_leased_prompt(
        &self,
        leased_agent_id: &str,
        steer_id: &str,
        target_home_prompt_id: &str,
        prompt: &str,
        hidden_system_context: &str,
        attachments: Vec<RelayPromptAttachment>,
        required_skills: Option<Vec<crate::transport::relay_peer::RequiredRemoteSkill>>,
    ) -> Result<(String, bool), DaemonError> {
        let leased_agent_id = leased_agent_id.to_string();
        let steer_id = steer_id.to_string();
        let target_home_prompt_id = target_home_prompt_id.to_string();
        let prompt = prompt.to_string();
        let hidden_system_context = hidden_system_context.to_string();
        loop {
            let provider_run_id = self
                .with_app_side_effect(|app| {
                    RemoteLeaseRuntime::new(app).leased_agent_provider_run_id(&leased_agent_id)
                })
                .await?
                .ok_or_else(|| DaemonError::NoActiveProviderRun {
                    session_id: format!("leased-agent:{leased_agent_id}"),
                })?;
            let _permit = self.provider_runtime_lanes.acquire(&provider_run_id).await;
            let (prepared_provider_run_id, dispatch) = self
                .with_app_side_effect(|app| {
                    RemoteLeaseRuntime::new(app).prepare_leased_prompt_steer(
                        &leased_agent_id,
                        &steer_id,
                        &target_home_prompt_id,
                        &prompt,
                        &hidden_system_context,
                        attachments.clone(),
                        required_skills.clone(),
                    )
                })
                .await?;
            if prepared_provider_run_id != provider_run_id {
                continue;
            }
            let Some(dispatch) = dispatch else {
                return Ok((provider_run_id, true));
            };
            let reserved = self
                .with_app_side_effect(|app| {
                    RemoteLeaseRuntime::new(app).reserve_leased_prompt_steer(
                        &leased_agent_id,
                        &steer_id,
                        &target_home_prompt_id,
                    )
                })
                .await?;
            if !reserved {
                return Ok((provider_run_id, true));
            }
            if let Err(error) = self.enqueue_prompt_dispatch(&dispatch).await {
                self.with_app_side_effect(|app| {
                    RemoteLeaseRuntime::new(app)
                        .rollback_leased_prompt_steer(&leased_agent_id, &steer_id);
                })
                .await;
                return Err(error);
            }
            return Ok((provider_run_id, false));
        }
    }

    pub(crate) async fn ensure_relay_remote_skill_packages(
        &self,
        context: RemoteSkillSyncContext,
        packages: Vec<CharioxSkillPackage>,
    ) -> Result<Vec<RemoteSkillMaterialization>, DaemonError> {
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).ensure_remote_skill_packages(context, packages)
        })
        .await
    }

    pub(crate) async fn ensure_relay_remote_provider_account(
        &self,
        context: crate::transport::relay_peer::RemoteProviderAccountSyncContext,
        materialization: crate::account_profile::ProviderAccountMaterialization,
    ) -> Result<crate::account_profile::ProviderAccountProfile, DaemonError> {
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).ensure_remote_provider_account(context, materialization)
        })
        .await
    }

    pub(crate) async fn check_relay_remote_mcp_availability(
        &self,
        context: RemoteMcpCheckContext,
        required_mcps: Vec<RequiredRemoteMcp>,
    ) -> Result<Vec<RemoteMcpAvailability>, DaemonError> {
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).check_remote_mcp_availability(context, required_mcps)
        })
        .await
    }

    pub(crate) async fn complete_relay_leased_prompt(
        &self,
        leased_agent_id: &str,
    ) -> Result<crate::session::PromptCompletion, DaemonError> {
        let leased_agent_id = leased_agent_id.to_string();
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).complete_leased_prompt(&leased_agent_id)
        })
        .await
    }

    pub(crate) async fn observe_relay_leased_git_after(
        &self,
        leased_agent_id: &str,
        provider_run_id: &str,
    ) -> Result<
        (
            Vec<RemoteGitObservation>,
            Option<crate::git_observer::WorkspaceLiveSyncChange>,
        ),
        DaemonError,
    > {
        let leased_agent_id = leased_agent_id.to_string();
        let provider_run_id = provider_run_id.to_string();
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app)
                .observe_leased_git_after(&leased_agent_id, &provider_run_id)
        })
        .await
    }

    pub(crate) async fn cancel_relay_leased_prompt(
        &self,
        leased_agent_id: &str,
    ) -> Result<crate::session::PromptCancellation, DaemonError> {
        self.cancel_remote_home_extension_invocations_for_leased_agent(leased_agent_id)
            .await;
        let leased_agent_id = leased_agent_id.to_string();
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).cancel_leased_prompt(&leased_agent_id)
        })
        .await
    }

    pub(crate) async fn relay_leased_agent_provider_run_id(
        &self,
        leased_agent_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        let leased_agent_id = leased_agent_id.to_string();
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).leased_agent_provider_run_id(&leased_agent_id)
        })
        .await
    }

    pub(crate) async fn try_pump_relay_leased_runtime_projections(
        &self,
    ) -> Result<Option<Vec<(String, RelayPeerEvent)>>, DaemonError> {
        match self.try_with_app_side_effect(|app| {
            RemoteLeaseRuntime::new(app).pump_leased_runtime_projections()
        }) {
            Some(result) => result.map(Some),
            None => Ok(None),
        }
    }

    pub(crate) async fn drain_relay_leased_runtime_projection(
        &self,
        leased_agent_id: &str,
        provider_run_id: &str,
        pump_output: bool,
        replay_settled_completion: bool,
    ) -> Result<Option<(String, RelayPeerEvent)>, DaemonError> {
        let leased_agent_id = leased_agent_id.to_string();
        let provider_run_id = provider_run_id.to_string();
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).drain_leased_runtime_projection_with_recovery(
                &leased_agent_id,
                &provider_run_id,
                pump_output,
                replay_settled_completion,
            )
        })
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn project_relay_remote_runtime_projection(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
        provider_run: Option<crate::provider::RuntimeProviderRun>,
        prompts: Vec<RelayProjectedPrompt>,
        output_chunks: Vec<RelayProjectedOutputChunk>,
        notices: Vec<String>,
        completions: Vec<RelayProjectedCompletion>,
    ) -> Result<(), DaemonError> {
        let provider_auth_observation = provider_run.as_ref().and_then(|run| {
            remote_provider_auth_observation(
                run,
                output_chunks.iter().any(|chunk| {
                    chunk.kind != crate::terminal::TerminalOutputKind::ProviderTerminal
                }) || !completions.is_empty(),
            )
        });
        let session_id = session_id.to_string();
        let agent_id = agent_id.to_string();
        let provider_run_id = provider_run_id.to_string();
        let projection_session_id = session_id.clone();
        let projection_agent_id = agent_id.clone();
        let projection_provider_run_id = provider_run_id.clone();
        let outcome = self
            .with_app_side_effect(move |app| {
                RemoteLeaseRuntime::new(app).project_remote_runtime_projection(
                    &projection_session_id,
                    &projection_agent_id,
                    &projection_provider_run_id,
                    provider_run,
                    prompts,
                    output_chunks,
                    notices,
                    completions,
                )
            })
            .await?;
        if !outcome.accepted {
            return Ok(());
        }
        if let Some(failure) = outcome.provider_failure {
            self.finish_remote_provider_failure(
                &session_id,
                &agent_id,
                &provider_run_id,
                &outcome.completions,
                failure,
            )
            .await?;
        }
        for completion in outcome.completions {
            self.inject_metaagent_turn_completion_event(&session_id, &agent_id, &completion)?;
        }
        if let Some((provider, account_profile, state)) = provider_auth_observation {
            self.apply_remote_slice_provider_auth_observation(
                &agent_id,
                &provider,
                &account_profile,
                state,
            )?;
        }
        Ok(())
    }

    fn apply_remote_slice_provider_auth_observation(
        &self,
        agent_id: &str,
        provider: &str,
        account_profile: &str,
        state: crate::slice_provider_auth::SliceProviderAuthState,
    ) -> Result<(), DaemonError> {
        let source = if state == crate::slice_provider_auth::SliceProviderAuthState::Authenticated {
            "provider_runtime_authenticated"
        } else {
            "provider_auth_failure"
        };
        for slice in self
            .list_slices()
            .into_iter()
            .filter(|slice| slice.agent_ids.iter().any(|id| id == agent_id))
        {
            let mut provider_auth = slice.provider_auth.clone();
            let mut matched = false;
            for summary in &mut provider_auth {
                if crate::provider::canonical_provider_family(&summary.provider) == Some(provider)
                    && summary.account_profile == account_profile
                {
                    summary.state = state.clone();
                    summary.source = source.to_string();
                    matched = true;
                }
            }
            if !matched {
                provider_auth.push(crate::slice_provider_auth::SliceProviderAuthSummary {
                    provider: provider.to_string(),
                    account_profile: account_profile.to_string(),
                    state: state.clone(),
                    auth_type: None,
                    account_id: None,
                    email: None,
                    organization_id: None,
                    organization_name: None,
                    subscription_type: None,
                    source: source.to_string(),
                });
            }
            self.set_slice_provider_auth(&slice.id, provider_auth)?;
        }
        Ok(())
    }
}

fn remote_provider_auth_observation(
    run: &crate::provider::RuntimeProviderRun,
    saw_provider_activity: bool,
) -> Option<(
    String,
    String,
    crate::slice_provider_auth::SliceProviderAuthState,
)> {
    let provider = crate::provider::canonical_provider_family(run.provider())?.to_string();
    let account_profile = run.account_profile().to_string();
    if run
        .terminal_diagnostic()
        .is_some_and(provider_diagnostic_is_auth_failure)
    {
        return Some((
            provider,
            account_profile,
            crate::slice_provider_auth::SliceProviderAuthState::NotConfigured,
        ));
    }
    saw_provider_activity.then_some((
        provider,
        account_profile,
        crate::slice_provider_auth::SliceProviderAuthState::Authenticated,
    ))
}

fn provider_diagnostic_is_auth_failure(diagnostic: &str) -> bool {
    let normalized = diagnostic.to_ascii_lowercase();
    [
        "401 unauthorized",
        "access token could not be refreshed",
        "authentication token has been invalidated",
        "refresh token was revoked",
        "please log out and sign in again",
        "please try signing in again",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

#[cfg(test)]
mod provider_auth_observation_tests {
    use super::provider_diagnostic_is_auth_failure;

    #[test]
    fn classifies_revoked_provider_credentials_without_matching_unrelated_failures() {
        assert!(provider_diagnostic_is_auth_failure(
            "Provider prompt dispatch failed: Your access token could not be refreshed because your refresh token was revoked. Please log out and sign in again."
        ));
        assert!(!provider_diagnostic_is_auth_failure(
            "Provider prompt dispatch failed: Unsupported parameter reasoning.summary"
        ));
    }
}
