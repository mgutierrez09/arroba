use std::sync::{Arc, Mutex, MutexGuard};

use crate::error::DaemonError;
use crate::prompt_assembly::PromptAssemblyMode;
use crate::provider::{
    FinishedProviderOutputPollJob, FinishedProviderPromptAbortJob, FinishedProviderPromptSubmitJob,
    LaunchProviderRequest, ProviderNativeInteractionBridge, ProviderPromptSignalBatch,
    ProviderRegistry, ProviderRunOperationLanes, ProviderRunTokenUsage, RuntimeProviderRun,
};
use crate::session::PromptAttachment;

use super::{
    ProviderProcessService, ProviderRunEndedOutcome, ProviderRunLivenessReconciliation,
    ProviderRunParkedOutcome, ProviderRunResumedOutcome, ProviderRunStartedOutcome,
    ProviderRuntimeBinding, ProviderSessionRunsTerminatedOutcome,
};

#[derive(Clone)]
pub struct ProviderProcessServiceStore {
    inner: Arc<Mutex<ProviderProcessService>>,
}

impl std::fmt::Debug for ProviderProcessServiceStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderProcessServiceStore")
            .finish_non_exhaustive()
    }
}

impl ProviderProcessServiceStore {
    pub fn new(service: ProviderProcessService) -> Self {
        Self {
            inner: Arc::new(Mutex::new(service)),
        }
    }

    pub fn read(&self) -> MutexGuard<'_, ProviderProcessService> {
        self.inner.lock().expect("provider service mutex poisoned")
    }

    pub fn write(&self) -> MutexGuard<'_, ProviderProcessService> {
        self.inner.lock().expect("provider service mutex poisoned")
    }

    pub fn registry(&self) -> ProviderRegistry {
        *self.read().registry()
    }

    pub(crate) fn run_operation_lanes(&self) -> ProviderRunOperationLanes {
        self.read().run_operation_lanes()
    }

    pub(crate) fn set_native_interaction_bridge(
        &self,
        bridge: Arc<dyn ProviderNativeInteractionBridge>,
    ) {
        self.read().set_native_interaction_bridge(bridge);
    }

    pub(crate) fn native_interaction_bridge(
        &self,
    ) -> Option<Arc<dyn ProviderNativeInteractionBridge>> {
        self.read().native_interaction_bridge()
    }

    pub(crate) fn run_actor_completion_signal(
        &self,
    ) -> crate::provider::ProviderRunActorCompletionSignal {
        self.read().run_actor_completion_signal()
    }

    pub fn get_run(&self, run_id: &str) -> Result<RuntimeProviderRun, DaemonError> {
        self.read().get_run(run_id)
    }

    #[doc(hidden)]
    pub fn structured_runtime_state_bound_for_tests(&self, provider_run_id: &str) -> bool {
        self.read()
            .structured_runtime_state_bound_for_tests(provider_run_id)
    }

    pub(crate) fn start_run_provider_only(
        &self,
        request: LaunchProviderRequest,
    ) -> Result<ProviderRunStartedOutcome, DaemonError> {
        self.write().start_run_provider_only(request)
    }

    pub(crate) fn launch_run_detached(
        &self,
        request: LaunchProviderRequest,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        self.write().launch_run_detached(request)
    }

    pub(crate) fn park_run_provider_only(
        &self,
        session_id: &str,
        run_id: &str,
    ) -> Result<ProviderRunParkedOutcome, DaemonError> {
        self.write().park_run_provider_only(session_id, run_id)
    }

    pub(crate) fn resume_run_provider_only(
        &self,
        session_id: &str,
        run_id: &str,
    ) -> Result<ProviderRunResumedOutcome, DaemonError> {
        self.write().resume_run_provider_only(session_id, run_id)
    }

    pub fn resume_run_detached(&self, run_id: &str) -> Result<RuntimeProviderRun, DaemonError> {
        self.write().resume_run_detached(run_id)
    }

    pub(crate) fn terminate_run_provider_only(
        &self,
        session_id: &str,
        run_id: &str,
    ) -> Result<ProviderRunEndedOutcome, DaemonError> {
        self.write().terminate_run_provider_only(session_id, run_id)
    }

    pub fn list_runs(&self) -> Vec<RuntimeProviderRun> {
        self.read().list_runs()
    }

    pub fn get_run_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<RuntimeProviderRun> {
        self.read().get_run_for_agent(session_id, agent_id)
    }

    pub fn get_latest_run_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<RuntimeProviderRun> {
        self.read().get_latest_run_for_agent(session_id, agent_id)
    }

    pub fn get_session_run_for_provider(
        &self,
        session_id: &str,
        provider: &str,
    ) -> Option<RuntimeProviderRun> {
        self.read()
            .get_session_run_for_provider(session_id, provider)
    }

    pub fn get_run_by_runtime_mcp_auth_token(
        &self,
        auth_token: &str,
    ) -> Option<RuntimeProviderRun> {
        self.read().get_run_by_runtime_mcp_auth_token(auth_token)
    }

    pub fn get_runs_by_runtime_mcp_auth_token(&self, auth_token: &str) -> Vec<RuntimeProviderRun> {
        self.read().get_runs_by_runtime_mcp_auth_token(auth_token)
    }

    pub(crate) fn structured_prompt_io_in_flight(&self, provider_run_id: &str) -> bool {
        self.read().structured_prompt_io_in_flight(provider_run_id)
    }

    pub fn record_run_activity(&self, run_id: &str) -> Result<(), DaemonError> {
        self.write().record_run_activity(run_id)
    }

    pub(crate) fn mark_run_running(&self, run_id: &str) -> Result<RuntimeProviderRun, DaemonError> {
        self.write().mark_run_running(run_id)
    }

    pub(crate) fn adapter_supports_turn_scoped_execution_config(&self, adapter_key: &str) -> bool {
        self.read()
            .adapter_supports_turn_scoped_execution_config(adapter_key)
    }

    pub(crate) fn update_run_execution_config(
        &self,
        run_id: &str,
        execution_mode: crate::provider::AgentExecutionMode,
        permission_level: crate::provider::AgentPermissionLevel,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        self.write()
            .update_run_execution_config(run_id, execution_mode, permission_level)
    }

    pub(crate) fn update_run_remote_extension_manifest(
        &self,
        run_id: &str,
        manifest: crate::extension::RemoteExtensionManifest,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        self.write()
            .update_run_remote_extension_manifest(run_id, manifest)
    }

    pub(crate) fn enable_workflow_tools(
        &self,
        run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        self.write().enable_workflow_tools(run_id)
    }

    pub(crate) fn mark_workflow_fresh_context(
        &self,
        run_id: &str,
        workflow_node_run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        self.write()
            .mark_workflow_fresh_context(run_id, workflow_node_run_id)
    }

    pub(crate) fn reconcile_run_liveness_provider_only(
        &self,
        session_id: &str,
        run_id: &str,
        process_running: Option<bool>,
    ) -> Result<ProviderRunLivenessReconciliation, DaemonError> {
        self.write()
            .reconcile_run_liveness_provider_only(session_id, run_id, process_running)
    }

    pub(crate) fn terminate_session_runs_provider_only(
        &self,
        session_id: &str,
    ) -> Result<ProviderSessionRunsTerminatedOutcome, DaemonError> {
        self.write()
            .terminate_session_runs_provider_only(session_id)
    }

    pub fn initialize_runtime(&self, run: &RuntimeProviderRun) -> Result<(), DaemonError> {
        let binding = ProviderProcessService::initialize_runtime_binding(run)?;
        if let Some(binding) = binding {
            self.write().apply_runtime_binding(run.id(), binding)?;
        }
        Ok(())
    }

    pub(crate) fn initialize_runtime_with_credentials(
        &self,
        run: &RuntimeProviderRun,
        credentials: &crate::provider::ProviderCredentialEnvironment,
    ) -> Result<(), DaemonError> {
        let binding =
            ProviderProcessService::initialize_runtime_binding_with_credentials(run, credentials)?;
        if let Some(binding) = binding {
            self.write().apply_runtime_binding(run.id(), binding)?;
        }
        Ok(())
    }

    pub(crate) fn apply_runtime_binding(
        &self,
        run_id: &str,
        binding: ProviderRuntimeBinding,
    ) -> Result<(), DaemonError> {
        self.write().apply_runtime_binding(run_id, binding)
    }

    pub(crate) fn run_uses_structured_prompt_io(&self, run: &RuntimeProviderRun) -> bool {
        self.read().run_uses_structured_prompt_io(run)
    }

    pub fn enqueue_run_selection_sync(&self, provider_run_id: &str) -> Result<(), DaemonError> {
        self.write().enqueue_run_selection_sync(provider_run_id)
    }

    pub(crate) fn update_run_selection(
        &self,
        provider_run_id: &str,
        model: Option<String>,
        variant: Option<String>,
        clear_variant: bool,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        self.write()
            .update_run_selection(provider_run_id, model, variant, clear_variant)
    }

    pub(crate) fn record_observed_usage(
        &self,
        provider_run_id: &str,
        usage: ProviderRunTokenUsage,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        self.write().record_observed_usage(provider_run_id, usage)
    }

    pub(crate) fn apply_finished_provider_run_selection_sync_jobs(&self) {
        self.write()
            .apply_finished_provider_run_selection_sync_jobs()
    }

    pub fn clear_runtime(&self, provider_run_id: &str) {
        self.write().clear_runtime(provider_run_id)
    }

    pub(crate) fn enqueue_structured_prompt_submit(
        &self,
        session_id: String,
        provider_run_id: String,
        agent_id: String,
        prompt_id: String,
        run: &RuntimeProviderRun,
        prompt: &str,
        hidden_system_context: &str,
        attachments: &[PromptAttachment],
        mode: PromptAssemblyMode,
        steering: bool,
    ) -> Result<(), DaemonError> {
        self.write().enqueue_structured_prompt_submit(
            session_id,
            provider_run_id,
            agent_id,
            prompt_id,
            run,
            prompt,
            hidden_system_context,
            attachments,
            mode,
            steering,
        )
    }

    pub(crate) fn run_structured_utility_prompt(
        &self,
        run: &RuntimeProviderRun,
        visible_user_prompt: &str,
        hidden_system_context: &str,
        timeout: std::time::Duration,
    ) -> Result<String, DaemonError> {
        self.write().run_structured_utility_prompt(
            run,
            visible_user_prompt,
            hidden_system_context,
            timeout,
        )
    }

    pub(crate) fn enqueue_structured_prompt_abort(
        &self,
        session_id: String,
        provider_run_id: String,
    ) -> Result<(), DaemonError> {
        self.write()
            .enqueue_structured_prompt_abort(session_id, provider_run_id)
    }

    pub(crate) fn drain_finished_structured_prompt_submit_jobs(
        &self,
    ) -> Vec<FinishedProviderPromptSubmitJob> {
        self.write().drain_finished_structured_prompt_submit_jobs()
    }

    pub(crate) fn schedule_finished_structured_prompt_submit_retry(
        &self,
        finished: FinishedProviderPromptSubmitJob,
    ) {
        self.write()
            .schedule_finished_structured_prompt_submit_retry(finished);
    }

    pub(crate) fn schedule_finished_structured_output_poll_retry(
        &self,
        finished: FinishedProviderOutputPollJob,
    ) {
        self.write()
            .schedule_finished_structured_output_poll_retry(finished);
    }

    pub(crate) fn preview_structured_output_metadata(
        &self,
        provider_run_id: &str,
        batch: &ProviderPromptSignalBatch,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        self.read()
            .preview_structured_output_metadata(provider_run_id, batch)
    }

    #[cfg(test)]
    pub(crate) fn push_finished_structured_prompt_submit_for_test(
        &self,
        session_id: String,
        provider_run_id: String,
        agent_id: String,
        prompt_id: String,
        result: Result<crate::provider::ProviderPromptSubmitAcknowledgement, DaemonError>,
    ) {
        self.write()
            .push_finished_structured_prompt_submit_for_test(
                session_id,
                provider_run_id,
                agent_id,
                prompt_id,
                result,
            );
    }

    pub(crate) fn apply_prompt_submit_acknowledgement(
        &self,
        provider_run_id: &str,
        acknowledgement: &crate::provider::ProviderPromptSubmitAcknowledgement,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        self.write()
            .apply_prompt_submit_acknowledgement(provider_run_id, acknowledgement)
    }

    pub(crate) fn drain_finished_structured_prompt_abort_jobs(
        &self,
    ) -> Vec<FinishedProviderPromptAbortJob> {
        self.write().drain_finished_structured_prompt_abort_jobs()
    }

    pub fn enqueue_structured_output_poll(
        &self,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        self.write().enqueue_structured_output_poll(provider_run_id)
    }

    pub fn set_output_poll_delay_for_tests(
        &self,
        provider_run_id: &str,
        delay: std::time::Duration,
    ) {
        self.read()
            .set_output_poll_delay_for_tests(provider_run_id, delay);
    }

    pub(crate) fn drain_finished_structured_output_poll_jobs(
        &self,
    ) -> Vec<FinishedProviderOutputPollJob> {
        self.write().drain_finished_structured_output_poll_jobs()
    }

    pub(crate) fn apply_structured_output_metadata(
        &self,
        provider_run_id: &str,
        batch: &ProviderPromptSignalBatch,
    ) -> Result<(), DaemonError> {
        self.write()
            .apply_structured_output_metadata(provider_run_id, batch)
    }

    pub(crate) fn record_terminal_diagnostic(
        &self,
        provider_run_id: &str,
        diagnostic: impl Into<String>,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        self.write()
            .record_terminal_diagnostic(provider_run_id, diagnostic)
    }
}
