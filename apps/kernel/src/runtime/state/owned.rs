//! Owned-state result envelopes shared across runtime-state domains.
//!
//! These structs keep mutation methods explicit about which owned objects were removed or
//! completed, without exposing broader `KernelRuntimeOwnedState` internals.

pub(in crate::runtime::state) struct OwnedProviderRunExit {
    pub(in crate::runtime::state) ended_run: crate::provider::RuntimeProviderRun,
    pub(in crate::runtime::state) already_ended: bool,
}

pub(in crate::runtime::state) struct OwnedAgentConfigUpdate {
    pub(in crate::runtime::state) agent: crate::agent::AgentInstance,
    pub(in crate::runtime::state) terminated_run_ids: Vec<String>,
    pub(in crate::runtime::state) remote_update: Option<OwnedRemoteAgentConfigUpdate>,
}

pub(in crate::runtime::state) struct OwnedRemoteAgentConfigUpdate {
    pub(in crate::runtime::state) worker_kernel_id: String,
    pub(in crate::runtime::state) leased_agent_id: String,
    pub(in crate::runtime::state) relay_url: Option<String>,
    pub(in crate::runtime::state) relay_token: Option<String>,
    pub(in crate::runtime::state) execution_mode: crate::provider::AgentExecutionMode,
    pub(in crate::runtime::state) permission_level: crate::provider::AgentPermissionLevel,
}

pub(in crate::runtime::state) struct OwnedAgentProfileUpdate {
    pub(in crate::runtime::state) agent: crate::agent::AgentInstance,
    pub(in crate::runtime::state) terminated_run_ids: Vec<String>,
    pub(in crate::runtime::state) remote_update: Option<OwnedRemoteAgentProfileUpdate>,
}

pub(in crate::runtime::state) struct OwnedRemoteAgentProfileUpdate {
    pub(in crate::runtime::state) worker_kernel_id: String,
    pub(in crate::runtime::state) execution_lease_id: String,
    pub(in crate::runtime::state) leased_agent_id: String,
    pub(in crate::runtime::state) relay_url: Option<String>,
    pub(in crate::runtime::state) relay_token: Option<String>,
    pub(in crate::runtime::state) provider: String,
    pub(in crate::runtime::state) account_profile: String,
    pub(in crate::runtime::state) model: Option<String>,
    pub(in crate::runtime::state) effort: Option<String>,
}

pub(in crate::runtime::state) struct OwnedPromptCompletion {
    pub(in crate::runtime::state) completion: crate::session::PromptCompletion,
    pub(in crate::runtime::state) released_claim: bool,
    pub(in crate::runtime::state) dispatch: Option<crate::app::KernelPromptDispatch>,
}

pub(in crate::runtime::state) struct OwnedPromptCancellation {
    pub(in crate::runtime::state) cancellation: crate::session::PromptCancellation,
    pub(in crate::runtime::state) released_claim: bool,
    pub(in crate::runtime::state) dispatch: Option<crate::app::KernelPromptDispatch>,
}
