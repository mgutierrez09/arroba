use std::path::PathBuf;

use chariox_relay::protocol::ClientTarget;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::LaunchProviderRequest;
use crate::session::{PromptQueueItem, PromptSubmissionOutcome};
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

pub(super) fn submit_claimed_workflow_prompt(
    app: &mut DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
    target_agent_id: &str,
    prompt: &str,
) -> Result<PromptSubmissionOutcome, DaemonError> {
    let outcome = app.prompt_owner_submit_workflow_prompt(
        session_id,
        &super::workflow_prompt_source_attachment_id(workflow_run_id),
        target_agent_id,
        workflow_run_id,
        workflow_node_run_id,
        prompt.to_string(),
    )?;
    Ok(outcome)
}

pub(super) fn dispatch_workflow_prompt(
    app: &mut DaemonApp,
    session_id: &str,
    target_agent_id: &str,
    prompt: &PromptQueueItem,
) -> Result<(), DaemonError> {
    let target_agent = app.agents().get_agent(target_agent_id)?;
    if let Some(remote_execution) = target_agent.remote_execution().cloned() {
        app.ensure_remote_agent_binding_protocol(&remote_execution)?;
        app.mark_active_prompt_delivery(
            session_id,
            target_agent_id,
            prompt.id(),
            crate::session::DurablePromptDeliveryPhase::Dispatching,
            None,
            None,
        )?;
        let session = app.sessions().get_session(session_id)?;
        let workspace_live_sync_mode =
            crate::provider::provider_workspace_live_sync_mode_for_session(
                target_agent.provider(),
                app.config(),
                Some(&session),
            );
        let workflow_context = crate::app::RemoteWorkflowTurnContextResolver::new(app)
            .remote_workflow_turn_context_for_prompt(session_id, target_agent_id, prompt)?;
        let (required_mcps, required_skills, remote_extension_manifest) =
            app.remote_prompt_capabilities_for_agent(&target_agent)?;
        let relay_config = app.config().clone();
        let response = app.send_remote_prompt_peer_request_with_credential_retry(
            &relay_config,
            ClientTarget {
                daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                daemon_alias: None,
            },
            RelayPeerRequest::SubmitLeasedPrompt {
                leased_agent_id: remote_execution.leased_agent_id,
                prompt: prompt.prompt().to_string(),
                hidden_system_context: prompt.hidden_system_context().to_string(),
                attachments: app.serialize_remote_prompt_attachments(prompt.attachments())?,
                workflow_context: Some(workflow_context),
                git_context: Some(crate::transport::relay_peer::RemoteGitTurnContext {
                    home_session_id: session_id.to_string(),
                    home_agent_id: target_agent_id.to_string(),
                    home_prompt_id: prompt.id().to_string(),
                    home_turn_id: prompt.id().to_string(),
                    source_attachment_id: Some(prompt.source_attachment_id().to_string()),
                    workspace_live_sync_mode: Some(workspace_live_sync_mode),
                    prompt_origin: Some(prompt.prompt_origin()),
                    external_provider: prompt.external_provider().map(str::to_string),
                    external_provider_session_id: prompt
                        .external_provider_session_id()
                        .map(str::to_string),
                    external_provider_turn_id: prompt
                        .external_provider_turn_id()
                        .map(str::to_string),
                    prompt_summary: crate::prompt_transcript::render_prompt_transcript(
                        prompt.prompt(),
                        prompt.attachments(),
                    ),
                }),
                required_mcps,
                required_skills,
                remote_extension_manifest,
                provider_launch_credential: None,
            },
            &target_agent,
        );
        return match response {
            Ok(RelayPeerResponse::LeasedPromptSubmitted {
                provider_run_id, ..
            }) => {
                app.mark_active_prompt_delivery(
                    session_id,
                    target_agent_id,
                    prompt.id(),
                    crate::session::DurablePromptDeliveryPhase::Delivered,
                    Some(provider_run_id),
                    None,
                )?;
                Ok(())
            }
            Ok(other) => Err(DaemonError::LocalTransport {
                operation: "dispatch remote workflow prompt",
                message: format!("unexpected remote workflow prompt response: {other:?}"),
            }),
            Err(error) => Err(error),
        };
    }

    let dispatch = |app: &mut DaemonApp, provider_run_id: &str| {
        crate::app::ProviderPromptDispatcher::new(app).dispatch_prompt_to_provider(
            session_id,
            provider_run_id,
            prompt.id(),
            prompt.source_attachment_id(),
            prompt.prompt(),
            prompt.hidden_system_context(),
            prompt.attachments(),
        )
    };
    let mut last_retryable_error = None;
    for attempt in 0..3 {
        let provider_run_id =
            crate::app::workflow_runtime::ensure_workflow_provider_run_for_prompt_from_runtime(
                app,
                session_id,
                target_agent_id,
                prompt,
            )?;
        match dispatch(app, &provider_run_id) {
            Ok(()) => {
                crate::transport::flow_control::note_prompt_started(app, &provider_run_id);
                return Ok(());
            }
            Err(
                error @ (DaemonError::InvalidProviderRunState { .. }
                | DaemonError::NoActiveProviderRun { .. }
                | DaemonError::PtyWrite { .. }
                | DaemonError::PtyProcessNotFound { .. }),
            ) if attempt < 2 => {
                last_retryable_error = Some(error);
                continue;
            }
            Err(other) => return Err(other),
        }
    }
    Err(
        last_retryable_error.unwrap_or(DaemonError::NoActiveProviderRun {
            session_id: session_id.to_string(),
        }),
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn ensure_workflow_provider_run_for_agent(
    app: &mut DaemonApp,
    session_id: &str,
    agent_id: &str,
    event_reply_enabled: bool,
    event_context_enabled: bool,
    event_actions_enabled: bool,
    fresh_context: bool,
    workflow_node_run_id: Option<&str>,
) -> Result<String, DaemonError> {
    if fresh_context {
        app.end_provider_run_for_workflow_context_flush(session_id, agent_id)?;
    }
    if let Some(run) = app.providers().get_run_for_agent(session_id, agent_id) {
        if run.workflow_tools_enabled()
            && run.workflow_event_reply_enabled() == event_reply_enabled
            && run.workflow_event_context_enabled() == event_context_enabled
            && run.workflow_event_actions_enabled() == event_actions_enabled
        {
            let provider_run_id = app.ensure_prompt_provider_run_for_agent(session_id, agent_id)?;
            app.sessions_mut()
                .set_active_provider_run(session_id, Some(provider_run_id.clone()))?;
            return Ok(provider_run_id);
        }
        // An ordinary provider may already have cached the reduced MCP tool
        // list. Replace it before the workflow prompt is dispatched so the
        // provider discovers the workflow-only actions at startup; flipping
        // a flag on a running process is too late (tools/list is not dynamic).
        if run.state() == crate::provider::ProviderRunState::Running
            && app.provider_run_has_active_prompt(session_id, &run)?
        {
            return Ok(run.id().to_string());
        }
    }
    // Cold workflow admission must launch the workflow-capable process directly.
    // Starting an ordinary process first and replacing it below leaves two live
    // provider processes for the same workflow agent.
    let request = workflow_provider_request(
        app,
        session_id,
        agent_id,
        event_reply_enabled,
        event_context_enabled,
        event_actions_enabled,
        fresh_context,
    )?;
    let provider_run =
        app.start_workflow_provider_launch_for_node(request, workflow_node_run_id)?;
    Ok(provider_run.id().to_string())
}

fn workflow_provider_request(
    app: &DaemonApp,
    session_id: &str,
    agent_id: &str,
    event_reply_enabled: bool,
    event_context_enabled: bool,
    event_actions_enabled: bool,
    fresh_context: bool,
) -> Result<LaunchProviderRequest, DaemonError> {
    let agent = app.agents().get_agent(agent_id)?;
    let provider = crate::provider::provider_id_for_launch(agent.provider());
    let adapter_key = crate::provider::adapter_key_for_provider(provider);
    let session = app.sessions().get_session(session_id)?;
    let effective_config = crate::session::effective_agent_execution_config(&session, Some(&agent));
    let mut request = LaunchProviderRequest::new(
        session_id,
        adapter_key,
        provider,
        agent.provider_account_profile(),
        agent.model().unwrap_or("default"),
    )
    .with_workflow_event_reply(event_reply_enabled)
    .with_workflow_event_context(event_context_enabled)
    .with_workflow_event_actions(event_actions_enabled)
    .with_agent_id(agent.id().to_string())
    .with_variant(agent.effort().map(str::to_string))
    .with_execution_mode(effective_config.mode)
    .with_permission_level(effective_config.permission_level);
    if fresh_context {
        // `Some(empty)` explicitly suppresses the agent profile's durable resume state
        // during launch preparation. It remains local to this workflow launch request.
        request.resume_state = Some(crate::provider::ProviderResumeState::default());
    }
    if let Some(working_directory) = app
        .providers()
        .get_run_for_agent(session_id, agent_id)
        .and_then(|run| run.working_directory().cloned())
        .or_else(|| agent.worktree_id().map(PathBuf::from))
    {
        request = request.with_working_directory(working_directory);
    }
    Ok(request)
}
