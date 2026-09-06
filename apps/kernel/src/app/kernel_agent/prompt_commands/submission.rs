use super::super::KernelAgentService;
use crate::app::prompt_lifecycle::{KernelPromptAdmission, KernelPromptOwnerSubmission};
use crate::app::{
    KernelPreparedPromptSubmission, KernelPromptDispatch, KernelPromptSubmission,
    KernelRemotePromptDispatch,
};
use crate::error::DaemonError;
use crate::provider::ProviderRunState;
use crate::session::{PromptAttachment, PromptQueueItem, PromptStatus, PromptSubmissionOutcome};

fn remote_workspace_live_sync_mode_for_submission(
    app: &crate::app::DaemonApp,
    session_id: &str,
    agent_id: &str,
) -> Option<crate::config::WorkspaceLiveSyncMode> {
    let session = app.sessions().get_session(session_id).ok()?;
    let agent = app.agents().get_agent(agent_id).ok()?;
    Some(
        crate::provider::provider_workspace_live_sync_mode_for_session(
            agent.provider(),
            app.config(),
            Some(&session),
        ),
    )
}

impl<'a> KernelAgentService<'a> {
    pub(crate) fn submit_prepared_prompt_for_kernel(
        &mut self,
        prepared: KernelPreparedPromptSubmission,
    ) -> Result<KernelPromptSubmission, DaemonError> {
        let admission = self.prepare_prompt_admission(prepared)?;
        let submitted = self.submit_admitted_prompt_to_owner(admission)?;
        if let PromptSubmissionOutcome::Started { prompt } = &submitted.outcome {
            self.spawn_prompt_history_append(
                &submitted.admission,
                None,
                prompt.id(),
                prompt.prompt(),
                prompt.attachments(),
            )?;
        }
        let (dispatch, remote_dispatch) = self.prepare_prompt_submission_effects(&submitted)?;
        let session = crate::app::KernelSessionReadService::new(self.app)
            .session_snapshot(&submitted.admission.session_id)?;
        Ok(KernelPromptSubmission {
            outcome: submitted.outcome,
            session,
            dispatch,
            remote_dispatch,
        })
    }

    pub(crate) fn submit_prompt(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        target_agent_id: Option<&str>,
        prompt: &str,
        attachments: Vec<PromptAttachment>,
    ) -> Result<PromptSubmissionOutcome, DaemonError> {
        self.submit_prompt_with_hidden_system_context(
            session_id,
            attachment_id,
            target_agent_id,
            prompt,
            "",
            attachments,
        )
    }

    pub(crate) fn submit_prompt_with_hidden_system_context(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        target_agent_id: Option<&str>,
        prompt: &str,
        hidden_system_context: &str,
        attachments: Vec<PromptAttachment>,
    ) -> Result<PromptSubmissionOutcome, DaemonError> {
        crate::app::KernelSessionReadService::new(self.app)
            .ensure_attachment_in_session(session_id, attachment_id)?;
        let target_agent_id = match target_agent_id {
            Some(target_agent_id) => target_agent_id.to_string(),
            None => self
                .app
                .sessions()
                .get_session(session_id)?
                .focused_agent_id()
                .ok_or_else(|| DaemonError::AgentNotFound {
                    agent_id: "no focused agent".to_string(),
                })?
                .to_string(),
        };
        let prepared_prompt = PromptQueueItem::new(
            "pending-draft:compat-submit",
            attachment_id,
            &target_agent_id,
            prompt,
            PromptStatus::Queued,
        )
        .with_attachments(attachments)
        .with_hidden_system_context(hidden_system_context);
        let submitted = self.submit_prepared_prompt_for_kernel(KernelPreparedPromptSubmission {
            session_id: session_id.to_string(),
            prompt: prepared_prompt,
            force_queue: false,
            refresh_projection: true,
        })?;
        let outcome = submitted.outcome;
        self.finish_compat_prompt_dispatch(submitted.dispatch)?;
        self.finish_compat_remote_prompt_dispatch(submitted.remote_dispatch)?;
        crate::app::KernelSessionReadService::new(self.app).session_snapshot(session_id)?;
        Ok(outcome)
    }

    pub(crate) fn record_native_prompt_started(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        history_source_attachment_id: &str,
        target_agent_id: &str,
        prompt: &str,
        attachments: Vec<PromptAttachment>,
    ) -> Result<PromptSubmissionOutcome, DaemonError> {
        crate::app::KernelSessionReadService::new(self.app)
            .ensure_attachment_in_session(session_id, attachment_id)?;
        let prepared_prompt = PromptQueueItem::new(
            "pending-draft:native-record",
            attachment_id,
            target_agent_id,
            prompt,
            PromptStatus::Queued,
        )
        .with_attachments(attachments);
        let admission = self.prepare_prompt_admission(KernelPreparedPromptSubmission {
            session_id: session_id.to_string(),
            prompt: prepared_prompt,
            force_queue: false,
            refresh_projection: true,
        })?;
        if admission.remote_execution.is_some() {
            return Err(DaemonError::LocalTransport {
                operation: "record native prompt",
                message: "native provider prompt recording requires a local provider run"
                    .to_string(),
            });
        }
        let submitted = self.submit_admitted_prompt_to_owner(admission)?;
        if let PromptSubmissionOutcome::Started { prompt } = &submitted.outcome {
            self.spawn_prompt_history_append(
                &submitted.admission,
                Some(history_source_attachment_id),
                prompt.id(),
                prompt.prompt(),
                prompt.attachments(),
            )?;
        }
        let provider_run_id = submitted.admission.provider_run_id.clone();
        let (dispatch, _) = self.prepare_local_prompt_submission_effects(&submitted)?;
        if matches!(submitted.outcome, PromptSubmissionOutcome::Started { .. }) {
            if let Some(provider_run_id) = provider_run_id.or_else(|| {
                dispatch
                    .as_ref()
                    .map(|dispatch| dispatch.provider_run_id.clone())
            }) {
                crate::transport::flow_control::note_prompt_started(self.app, &provider_run_id);
            }
        }
        let _ = crate::app::KernelSessionReadService::new(self.app)
            .session_snapshot(&submitted.admission.session_id)?;
        Ok(submitted.outcome)
    }

    pub(super) fn finish_compat_prompt_dispatch(
        &mut self,
        dispatch: Option<KernelPromptDispatch>,
    ) -> Result<(), DaemonError> {
        let Some(dispatch) = dispatch else {
            return Ok(());
        };
        if let Err(error) = crate::app::ProviderPromptDispatcher::new(self.app)
            .dispatch_prompt_to_provider(
                &dispatch.session_id,
                &dispatch.provider_run_id,
                &dispatch.prompt_id,
                &dispatch.source_attachment_id,
                &dispatch.prompt,
                &dispatch.hidden_system_context,
                &dispatch.attachments,
            )
        {
            crate::app::KernelAgentService::new(self.app).cancel_active_after_prompt_start_failure(
                &dispatch.session_id,
                &dispatch.agent_id,
                &dispatch.provider_run_id,
            );
            let _ = crate::app::KernelSessionReadService::new(self.app)
                .session_snapshot(&dispatch.session_id);
            self.app.record_notice(
                &dispatch.session_id,
                Some(&dispatch.provider_run_id),
                self.app
                    .attachments
                    .list_session_attachment_ids(&dispatch.session_id),
                format!("Prompt dispatch failed after acknowledgement: {error}"),
            );
            return Err(error);
        }
        crate::transport::flow_control::note_prompt_started(self.app, &dispatch.provider_run_id);
        Ok(())
    }

    fn prepare_prompt_admission(
        &mut self,
        prepared: KernelPreparedPromptSubmission,
    ) -> Result<KernelPromptAdmission, DaemonError> {
        let session_id = prepared.session_id;
        let attachment_id = prepared.prompt.source_attachment_id().to_string();
        let target_agent_id = prepared.prompt.target_agent_id().to_string();
        let source_attachment = crate::app::KernelSessionReadService::new(self.app)
            .ensure_attachment_in_session(&session_id, &attachment_id)?;
        let prompt = prepared.prompt.with_source_attribution(
            source_attachment.client_id(),
            source_attachment.owner_user_id(),
        );

        let target_agent = self.app.agents.get_agent(&target_agent_id)?;
        if target_agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.clone(),
                agent_id: target_agent_id,
            });
        }
        let remote_execution = target_agent.remote_execution().cloned();
        if remote_execution.is_none() {
            self.app
                .provider_account_profile_registry()
                .require_agent_authenticated(&self.app.config, &target_agent, "submit prompt")?;
        }
        let (provider_run_id, provider_run_is_starting) = if remote_execution.is_some() {
            (None, false)
        } else {
            let queued_while_active = self
                .app
                .prompt_owner_active_prompt_for_agent(&session_id, &target_agent_id)?
                .is_some();
            let provider_run_id = if queued_while_active {
                self.app
                    .providers
                    .get_run_for_agent(&session_id, &target_agent_id)
                    .map(|run| run.id().to_string())
            } else {
                Some(
                    self.app
                        .ensure_prompt_provider_run_for_agent(&session_id, &target_agent_id)?,
                )
            };
            let provider_run_is_starting = provider_run_id
                .as_deref()
                .and_then(|provider_run_id| self.app.providers.get_run(provider_run_id).ok())
                .is_some_and(|run| run.state() == ProviderRunState::Starting);
            (provider_run_id, provider_run_is_starting)
        };

        Ok(KernelPromptAdmission {
            session_id,
            attachment_id,
            target_agent_id,
            prompt,
            force_queue: prepared.force_queue,
            provider_run_id,
            remote_execution,
            provider_run_is_starting,
        })
    }

    fn spawn_prompt_history_append(
        &self,
        admission: &KernelPromptAdmission,
        history_source_attachment_id: Option<&str>,
        prompt_id: &str,
        prompt: &str,
        attachments: &[PromptAttachment],
    ) -> Result<(), DaemonError> {
        self.app.spawn_user_prompt_history_append_with_prompt_id(
            &admission.session_id,
            history_source_attachment_id.unwrap_or(&admission.attachment_id),
            &admission.target_agent_id,
            prompt,
            attachments,
            admission.prompt.prompt_origin(),
            prompt_id,
            admission.prompt.created_at_ms(),
            admission.prompt.workflow_run_id(),
            admission.prompt.workflow_node_run_id(),
        )
    }

    fn submit_admitted_prompt_to_owner(
        &mut self,
        admission: KernelPromptAdmission,
    ) -> Result<KernelPromptOwnerSubmission, DaemonError> {
        let outcome = self.app.prompt_owner_submit_prepared_prompt(
            &admission.session_id,
            admission.prompt.clone(),
            admission.force_queue || admission.provider_run_is_starting,
        )?;
        if admission.remote_execution.is_none() {
            self.app
                .agents
                .clear_local_prompt_error(&admission.target_agent_id)?;
        }
        Ok(KernelPromptOwnerSubmission { admission, outcome })
    }

    fn prepare_prompt_submission_effects(
        &mut self,
        submitted: &KernelPromptOwnerSubmission,
    ) -> Result<
        (
            Option<KernelPromptDispatch>,
            Option<KernelRemotePromptDispatch>,
        ),
        DaemonError,
    > {
        if submitted.admission.remote_execution.is_some() {
            return self.prepare_remote_prompt_submission_effects(submitted);
        }
        self.prepare_local_prompt_submission_effects(submitted)
    }

    fn prepare_remote_prompt_submission_effects(
        &mut self,
        submitted: &KernelPromptOwnerSubmission,
    ) -> Result<
        (
            Option<KernelPromptDispatch>,
            Option<KernelRemotePromptDispatch>,
        ),
        DaemonError,
    > {
        let mut remote_dispatch = None;
        match &submitted.outcome {
            PromptSubmissionOutcome::Started { prompt } => {
                let Some(remote_execution) = submitted.admission.remote_execution.as_ref() else {
                    return Ok((None, None));
                };
                remote_dispatch = Some(KernelRemotePromptDispatch {
                    session_id: submitted.admission.session_id.clone(),
                    agent_id: submitted.admission.target_agent_id.clone(),
                    prompt_id: prompt.id().to_string(),
                    worker_kernel_id: remote_execution.worker_kernel_id.clone(),
                    leased_agent_id: remote_execution.leased_agent_id.clone(),
                    relay_url: remote_execution.relay_url.clone(),
                    relay_token: remote_execution.relay_token.clone(),
                    source_attachment_id: prompt.source_attachment_id().to_string(),
                    prompt: prompt.prompt().to_string(),
                    hidden_system_context: prompt.hidden_system_context().to_string(),
                    attachments: prompt.attachments().to_vec(),
                    workspace_live_sync_mode: remote_workspace_live_sync_mode_for_submission(
                        self.app,
                        &submitted.admission.session_id,
                        &submitted.admission.target_agent_id,
                    ),
                    prompt_origin: prompt.prompt_origin(),
                    external_provider: prompt.external_provider().map(str::to_string),
                    external_provider_session_id: prompt
                        .external_provider_session_id()
                        .map(str::to_string),
                    external_provider_turn_id: prompt
                        .external_provider_turn_id()
                        .map(str::to_string),
                    workflow_context: None,
                });
            }
            PromptSubmissionOutcome::Queued { prompt } => {
                self.record_queued_prompt_notice(submitted, prompt, None);
            }
        }
        Ok((None, remote_dispatch))
    }

    fn prepare_local_prompt_submission_effects(
        &mut self,
        submitted: &KernelPromptOwnerSubmission,
    ) -> Result<
        (
            Option<KernelPromptDispatch>,
            Option<KernelRemotePromptDispatch>,
        ),
        DaemonError,
    > {
        let mut dispatch = None;
        match &submitted.outcome {
            PromptSubmissionOutcome::Started { prompt } => {
                let provider_run_id =
                    submitted
                        .admission
                        .provider_run_id
                        .as_deref()
                        .ok_or_else(|| DaemonError::NoActiveProviderRun {
                            session_id: submitted.admission.session_id.clone(),
                        })?;
                self.app.echo_prompt_to_other_attachments(
                    &submitted.admission.session_id,
                    provider_run_id,
                    prompt.id(),
                    prompt.source_attachment_id(),
                    prompt.prompt(),
                    prompt.attachments(),
                );
                dispatch = Some(KernelPromptDispatch {
                    session_id: submitted.admission.session_id.clone(),
                    provider_run_id: provider_run_id.to_string(),
                    agent_id: submitted.admission.target_agent_id.clone(),
                    prompt_id: prompt.id().to_string(),
                    target_active_prompt_id: None,
                    source_attachment_id: prompt.source_attachment_id().to_string(),
                    prompt: prompt.prompt().to_string(),
                    hidden_system_context: prompt.hidden_system_context().to_string(),
                    attachments: prompt.attachments().to_vec(),
                    prompt_origin: prompt.prompt_origin(),
                    external_provider: prompt.external_provider().map(str::to_string),
                    external_provider_session_id: prompt
                        .external_provider_session_id()
                        .map(str::to_string),
                    external_provider_turn_id: prompt
                        .external_provider_turn_id()
                        .map(str::to_string),
                    steering: false,
                });
            }
            PromptSubmissionOutcome::Queued { prompt } => {
                self.record_queued_prompt_notice(
                    submitted,
                    prompt,
                    submitted.admission.provider_run_id.as_deref(),
                );
            }
        }
        Ok((dispatch, None))
    }

    fn record_queued_prompt_notice(
        &mut self,
        submitted: &KernelPromptOwnerSubmission,
        prompt: &PromptQueueItem,
        provider_run_id: Option<&str>,
    ) {
        let recipient_attachment_ids = self.app.other_attachment_ids(
            &submitted.admission.session_id,
            prompt.source_attachment_id(),
        );
        self.app.record_notice_for_agent(
            &submitted.admission.session_id,
            provider_run_id,
            Some(&submitted.admission.target_agent_id),
            recipient_attachment_ids,
            format!(
                "Attachment `{}` queued prompt `{}` for agent `{}`.",
                prompt.source_attachment_id(),
                prompt.id(),
                submitted.admission.target_agent_id
            ),
        );
    }
}
