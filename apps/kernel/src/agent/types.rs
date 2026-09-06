use serde::{Deserialize, Serialize};

use crate::extension::{ExtensionGrant, ExtensionKind, RemoteExtensionManifestSyncStatus};
use crate::provider::{
    AgentExecutionMode, AgentPermissionLevel, ExternalProviderImportMetadata, ProviderResumeState,
};
use crate::session::DEFAULT_LOCAL_USER_ID;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteAgentBinding {
    pub worker_kernel_id: String,
    pub worker_machine_id: String,
    pub execution_lease_id: String,
    pub leased_agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_worker_provider_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_peer_protocol_version: Option<u32>,
}

impl RemoteAgentBinding {
    pub(crate) fn relay_peer_protocol_compatible(&self) -> bool {
        self.relay_peer_protocol_version.is_some_and(|version| {
            version >= crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitWorktreePlacement {
    pub target_directory: Option<String>,
    pub branch: Option<String>,
    pub from_ref: Option<String>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentState {
    Idle,
    Working,
    Focused,
    Error,
}

impl AgentState {
    pub(crate) fn with_focus(self, focused: bool) -> Self {
        match self {
            Self::Working | Self::Error => self,
            Self::Idle | Self::Focused => {
                if focused {
                    Self::Focused
                } else {
                    Self::Idle
                }
            }
        }
    }
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    #[default]
    Standard,
    Meta,
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentOperatingMode {
    #[default]
    Regular,
    Meta,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaModeState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    task_id: Option<String>,
    activated_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    baseline_execution_mode_override: Option<AgentExecutionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    baseline_permission_level_override: Option<AgentPermissionLevel>,
}

impl MetaModeState {
    pub fn new(
        task_id: Option<String>,
        baseline_execution_mode_override: Option<AgentExecutionMode>,
        baseline_permission_level_override: Option<AgentPermissionLevel>,
    ) -> Self {
        Self {
            task_id,
            activated_at_ms: crate::session::unix_epoch_ms(),
            baseline_execution_mode_override,
            baseline_permission_level_override,
        }
    }

    pub fn task_id(&self) -> Option<&str> {
        self.task_id.as_deref()
    }

    pub fn activated_at_ms(&self) -> u64 {
        self.activated_at_ms
    }

    pub fn baseline_execution_mode_override(&self) -> Option<AgentExecutionMode> {
        self.baseline_execution_mode_override
    }

    pub fn baseline_permission_level_override(&self) -> Option<AgentPermissionLevel> {
        self.baseline_permission_level_override
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridPosition {
    pub row: u32,
    pub col: u32,
    pub row_span: u32,
    pub col_span: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSubstituteProfile {
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    /// Provider account profile used when launching this substitute. `None`
    /// keeps the historical default-account behavior.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kernel_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<String>,
}

impl AgentSubstituteProfile {
    pub fn new(
        provider: impl Into<String>,
        model: impl Into<String>,
        variant: Option<String>,
    ) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            variant,
            account_profile: None,
            kernel_id: None,
            worktree_id: None,
        }
    }

    pub fn with_account_profile(mut self, account_profile: Option<String>) -> Self {
        self.account_profile = normalize_optional_profile(account_profile);
        self
    }

    pub fn with_kernel_id(mut self, kernel_id: Option<String>) -> Self {
        self.kernel_id = kernel_id;
        self
    }

    pub fn with_worktree_id(mut self, worktree_id: Option<String>) -> Self {
        self.worktree_id = worktree_id;
        self
    }
}

fn normalize_optional_profile(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSubstitutionRecord {
    pub substitute_index: usize,
    pub reason: String,
    pub activated_at_ms: u64,
}

impl GridPosition {
    pub fn new(row: u32, col: u32, row_span: u32, col_span: u32) -> Self {
        Self {
            row,
            col,
            row_span,
            col_span,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentInstance {
    id: String,
    agent_ref: String,
    session_id: String,
    #[serde(default = "default_agent_owner_user_id")]
    owner_user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    controlled_by_metaagent_id: Option<String>,
    #[serde(default)]
    role: AgentRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    meta_mode: Option<MetaModeState>,
    alias: Option<String>,
    provider: String,
    model: Option<String>,
    effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    account_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    primary_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    primary_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    primary_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    execution_mode_override: Option<AgentExecutionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    permission_level_override: Option<AgentPermissionLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    worktree_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remote_execution: Option<RemoteAgentBinding>,
    #[serde(default, skip_serializing_if = "ProviderResumeState::is_empty")]
    provider_resume_state: ProviderResumeState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    external_provider_import: Option<ExternalProviderImportMetadata>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    extension_grants: Vec<ExtensionGrant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remote_extension_manifest_sync: Option<RemoteExtensionManifestSyncStatus>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    substitutes: Vec<AgentSubstituteProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active_substitute_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_substitution: Option<AgentSubstitutionRecord>,
    /// Account bound to the stored primary profile. Captured whenever the
    /// primary profile is snapshotted so returning from a substitute restores
    /// the exact primary account. Absent on legacy records means the default
    /// account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    primary_account_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    substitution_timeout_ms: Option<u64>,
    #[serde(
        default = "default_visible_in_freeform",
        skip_serializing_if = "is_default_visible_in_freeform"
    )]
    visible_in_freeform: bool,
    state: AgentState,
    is_processing: bool,
    position: GridPosition,
    created_at_ms: u64,
    last_activity_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_prompt_sent_at_ms: Option<u64>,
}

impl AgentInstance {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        agent_ref: impl Into<String>,
        session_id: impl Into<String>,
        alias: Option<String>,
        provider: impl Into<String>,
        model: Option<String>,
        effort: Option<String>,
        worktree_id: Option<String>,
        position: GridPosition,
    ) -> Self {
        let now = crate::session::unix_epoch_ms();
        Self {
            id: id.into(),
            agent_ref: agent_ref.into(),
            session_id: session_id.into(),
            owner_user_id: default_agent_owner_user_id(),
            controlled_by_metaagent_id: None,
            role: AgentRole::Standard,
            meta_mode: None,
            alias,
            provider: provider.into(),
            model,
            effort,
            account_profile: None,
            primary_provider: None,
            primary_model: None,
            primary_effort: None,
            execution_mode_override: None,
            permission_level_override: None,
            workspace_id: None,
            worktree_id,
            remote_execution: None,
            provider_resume_state: ProviderResumeState::default(),
            external_provider_import: None,
            extension_grants: Vec::new(),
            remote_extension_manifest_sync: None,
            substitutes: Vec::new(),
            active_substitute_index: None,
            last_substitution: None,
            primary_account_profile: None,
            substitution_timeout_ms: None,
            visible_in_freeform: true,
            state: AgentState::Idle,
            is_processing: false,
            position,
            created_at_ms: now,
            last_activity_at_ms: now,
            last_prompt_sent_at_ms: None,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn agent_ref(&self) -> &str {
        &self.agent_ref
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn owner_user_id(&self) -> &str {
        &self.owner_user_id
    }

    pub fn controlled_by_metaagent_id(&self) -> Option<&str> {
        self.controlled_by_metaagent_id.as_deref()
    }

    pub fn role(&self) -> AgentRole {
        self.role
    }

    pub fn operating_mode(&self) -> AgentOperatingMode {
        if self.meta_mode.is_some() {
            AgentOperatingMode::Meta
        } else {
            AgentOperatingMode::Regular
        }
    }

    pub fn meta_mode(&self) -> Option<&MetaModeState> {
        self.meta_mode.as_ref()
    }

    pub fn is_metaagent(&self) -> bool {
        self.meta_mode.is_some()
    }

    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    pub fn effort(&self) -> Option<&str> {
        self.effort.as_deref()
    }

    pub fn account_profile(&self) -> Option<&str> {
        self.account_profile.as_deref()
    }

    pub fn provider_account_profile(&self) -> &str {
        self.account_profile.as_deref().unwrap_or("default")
    }

    pub fn primary_provider(&self) -> &str {
        self.primary_provider
            .as_deref()
            .unwrap_or(self.provider.as_str())
    }

    pub fn primary_model(&self) -> Option<&str> {
        if self.primary_provider.is_some() {
            self.primary_model.as_deref()
        } else {
            self.model.as_deref()
        }
    }

    pub fn primary_effort(&self) -> Option<&str> {
        if self.primary_provider.is_some() {
            self.primary_effort.as_deref()
        } else {
            self.effort.as_deref()
        }
    }

    pub fn primary_account_profile(&self) -> Option<&str> {
        if self.primary_provider.is_some() {
            self.primary_account_profile.as_deref()
        } else {
            self.account_profile.as_deref()
        }
    }

    pub fn execution_mode_override(&self) -> Option<AgentExecutionMode> {
        self.execution_mode_override
    }

    pub fn permission_level_override(&self) -> Option<AgentPermissionLevel> {
        self.permission_level_override
    }

    pub fn workspace_id(&self) -> Option<&str> {
        self.workspace_id.as_deref()
    }

    pub fn worktree_id(&self) -> Option<&str> {
        self.worktree_id.as_deref()
    }

    pub fn remote_execution(&self) -> Option<&RemoteAgentBinding> {
        self.remote_execution.as_ref()
    }

    pub fn provider_resume_state(&self) -> &ProviderResumeState {
        &self.provider_resume_state
    }

    pub fn external_provider_import(&self) -> Option<&ExternalProviderImportMetadata> {
        self.external_provider_import.as_ref()
    }

    pub fn extension_grants(&self) -> &[ExtensionGrant] {
        &self.extension_grants
    }

    pub fn remote_extension_manifest_sync(&self) -> Option<&RemoteExtensionManifestSyncStatus> {
        self.remote_extension_manifest_sync.as_ref()
    }

    pub fn granted_extension_names(&self, kind: ExtensionKind) -> Vec<String> {
        self.extension_grants
            .iter()
            .filter(|grant| grant.kind == kind)
            .map(|grant| grant.name.clone())
            .collect()
    }

    pub fn mcp_grants(&self) -> Vec<String> {
        self.granted_extension_names(ExtensionKind::Mcp)
    }

    pub fn skill_grants(&self) -> Vec<String> {
        self.granted_extension_names(ExtensionKind::Skill)
    }

    pub fn script_grants(&self) -> Vec<ExtensionGrant> {
        self.extension_grants
            .iter()
            .filter(|grant| grant.kind == ExtensionKind::Script)
            .cloned()
            .collect()
    }

    pub fn connector_grants(&self) -> Vec<ExtensionGrant> {
        self.extension_grants
            .iter()
            .filter(|grant| grant.kind == ExtensionKind::Connector)
            .cloned()
            .collect()
    }

    pub fn has_extension_grant(&self, kind: ExtensionKind, name: &str) -> bool {
        self.extension_grants
            .iter()
            .any(|grant| grant.kind == kind && grant.name == name)
    }

    pub fn substitutes(&self) -> &[AgentSubstituteProfile] {
        &self.substitutes
    }

    pub fn active_substitute_index(&self) -> Option<usize> {
        self.active_substitute_index
    }

    pub fn last_substitution(&self) -> Option<&AgentSubstitutionRecord> {
        self.last_substitution.as_ref()
    }

    pub fn substitution_timeout_ms(&self) -> Option<u64> {
        self.substitution_timeout_ms
    }

    pub fn state(&self) -> AgentState {
        self.state
    }

    pub fn is_processing(&self) -> bool {
        self.is_processing
    }

    pub fn position(&self) -> &GridPosition {
        &self.position
    }

    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    pub fn last_activity_at_ms(&self) -> u64 {
        self.last_activity_at_ms
    }

    pub fn last_prompt_sent_at_ms(&self) -> Option<u64> {
        self.last_prompt_sent_at_ms
    }

    pub(crate) fn note_prompt_sent_at(&mut self, timestamp_ms: u64) {
        self.last_prompt_sent_at_ms = Some(timestamp_ms);
    }

    pub fn visible_in_freeform(&self) -> bool {
        self.visible_in_freeform
    }

    /// Projection-only collaborator visibility hint. Authoritative agent ownership and prompting
    /// rights remain session policy; durable state must not treat this as an access-control source.
    pub(crate) fn set_visible_in_freeform(&mut self, visible: bool) {
        self.visible_in_freeform = visible;
    }

    pub fn redacted_parameters(mut self) -> Self {
        self.provider = "redacted".to_string();
        self.model = None;
        self.effort = None;
        self.account_profile = None;
        self.primary_provider = None;
        self.primary_model = None;
        self.primary_effort = None;
        self.execution_mode_override = None;
        self.permission_level_override = None;
        self.controlled_by_metaagent_id = None;
        self.meta_mode = None;
        self.workspace_id = None;
        self.worktree_id = None;
        self.remote_execution = None;
        self.provider_resume_state = ProviderResumeState::default();
        self.external_provider_import = None;
        self.extension_grants.clear();
        self.substitutes.clear();
        self.active_substitute_index = None;
        self.last_substitution = None;
        self.substitution_timeout_ms = None;
        self
    }

    pub fn set_state(&mut self, state: AgentState) {
        self.state = state;
        self.last_activity_at_ms = crate::session::unix_epoch_ms();
    }

    pub fn set_processing(&mut self, is_processing: bool) {
        self.is_processing = is_processing;
        self.last_activity_at_ms = crate::session::unix_epoch_ms();
    }

    pub fn set_position(&mut self, position: GridPosition) {
        self.position = position;
    }

    pub fn set_alias(&mut self, alias: Option<String>) {
        self.alias = alias;
    }

    pub fn set_owner_user_id(&mut self, owner_user_id: impl Into<String>) {
        self.owner_user_id = owner_user_id.into();
    }

    pub fn set_controlled_by_metaagent_id(&mut self, metaagent_id: Option<String>) {
        self.controlled_by_metaagent_id = metaagent_id;
    }

    pub fn set_role(&mut self, role: AgentRole) {
        self.role = role;
    }

    pub fn activate_meta_mode(&mut self, task_id: Option<String>) {
        if self.meta_mode.is_none() {
            self.meta_mode = Some(MetaModeState::new(
                task_id,
                self.execution_mode_override,
                self.permission_level_override,
            ));
        } else if let Some(meta_mode) = self.meta_mode.as_mut() {
            if meta_mode.task_id.is_none() {
                meta_mode.task_id = task_id;
            }
        }
        self.role = AgentRole::Standard;
        self.last_activity_at_ms = crate::session::unix_epoch_ms();
    }

    pub fn deactivate_meta_mode(&mut self) -> Option<MetaModeState> {
        let previous = self.meta_mode.take();
        self.role = AgentRole::Standard;
        if let Some(meta_mode) = previous.as_ref() {
            self.execution_mode_override = meta_mode.baseline_execution_mode_override();
            self.permission_level_override = meta_mode.baseline_permission_level_override();
            self.last_activity_at_ms = crate::session::unix_epoch_ms();
        }
        previous
    }

    pub fn set_model(&mut self, model: Option<String>) {
        self.model = model;
    }

    pub fn set_provider(&mut self, provider: impl Into<String>) {
        self.provider = provider.into();
    }

    pub fn set_effort(&mut self, effort: Option<String>) {
        self.effort = effort;
    }

    pub fn set_account_profile(&mut self, account_profile: Option<String>) {
        self.account_profile = normalized_agent_account_profile(account_profile);
    }

    pub fn set_primary_profile(
        &mut self,
        provider: impl Into<String>,
        model: Option<String>,
        effort: Option<String>,
    ) {
        self.primary_provider = Some(provider.into());
        self.primary_model = model;
        self.primary_effort = effort;
        self.primary_account_profile = self.account_profile.clone();
    }

    /// Directly rewrites the stored primary snapshot (used for primary-profile
    /// edits while a substitute is active; the running substitute is untouched).
    /// A literal `default` sentinel is normalized away so stored snapshots keep
    /// `None` for the default account.
    pub fn set_primary_profile_snapshot(
        &mut self,
        provider: impl Into<String>,
        model: Option<String>,
        effort: Option<String>,
        account_profile: Option<String>,
    ) {
        self.primary_provider = Some(provider.into());
        self.primary_model = model;
        self.primary_effort = effort;
        self.primary_account_profile =
            normalized_agent_account_profile(account_profile.filter(|value| value != "default"));
        self.last_activity_at_ms = crate::session::unix_epoch_ms();
    }

    pub fn set_execution_mode_override(&mut self, execution_mode: Option<AgentExecutionMode>) {
        self.execution_mode_override = execution_mode;
    }

    pub fn set_permission_level_override(
        &mut self,
        permission_level: Option<AgentPermissionLevel>,
    ) {
        self.permission_level_override = permission_level;
    }

    pub fn set_workspace_id(&mut self, workspace_id: Option<String>) {
        self.workspace_id = workspace_id;
    }

    pub fn set_worktree_id(&mut self, worktree_id: Option<String>) {
        self.worktree_id = worktree_id;
    }

    pub fn canonicalized_for_publication_package(mut self, workspace_id: &str) -> Self {
        self.workspace_id = Some(workspace_id.to_string());
        self.worktree_id = Some(workspace_id.to_string());
        self.clear_publication_runtime_state();
        self.last_activity_at_ms = self.created_at_ms;
        self
    }

    pub fn materialized_for_publication_runtime(
        mut self,
        id: impl Into<String>,
        agent_ref: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Self {
        self.id = id.into();
        self.agent_ref = agent_ref.into();
        self.session_id = session_id.into();
        self.clear_publication_runtime_state();
        self.created_at_ms = crate::session::unix_epoch_ms();
        self.last_activity_at_ms = self.created_at_ms;
        self
    }

    pub fn materialized_for_workflow_runtime(
        mut self,
        id: impl Into<String>,
        agent_ref: impl Into<String>,
        session_id: impl Into<String>,
        worktree_id: impl Into<String>,
    ) -> Self {
        self.id = id.into();
        self.agent_ref = agent_ref.into();
        self.session_id = session_id.into();
        self.worktree_id = Some(worktree_id.into());
        self.clear_publication_runtime_state();
        self.controlled_by_metaagent_id = None;
        self.meta_mode = None;
        self.visible_in_freeform = false;
        self.created_at_ms = crate::session::unix_epoch_ms();
        self.last_activity_at_ms = self.created_at_ms;
        self
    }

    fn clear_publication_runtime_state(&mut self) {
        self.remote_execution = None;
        self.provider_resume_state = ProviderResumeState::default();
        self.external_provider_import = None;
        self.remote_extension_manifest_sync = None;
        self.state = AgentState::Idle;
        self.is_processing = false;
        self.last_prompt_sent_at_ms = None;
    }

    pub fn set_provider_resume_state(&mut self, resume_state: ProviderResumeState) {
        self.provider_resume_state = resume_state;
    }

    pub fn set_external_provider_import(&mut self, import: Option<ExternalProviderImportMetadata>) {
        self.external_provider_import = import;
    }

    pub fn set_remote_execution(&mut self, remote_execution: Option<RemoteAgentBinding>) {
        self.remote_execution = remote_execution;
    }

    pub fn set_remote_execution_active_worker_provider_run_id(
        &mut self,
        provider_run_id: Option<String>,
    ) {
        if let Some(remote_execution) = self.remote_execution.as_mut() {
            remote_execution.active_worker_provider_run_id = provider_run_id;
        }
    }

    pub fn set_remote_extension_manifest_sync(
        &mut self,
        status: Option<RemoteExtensionManifestSyncStatus>,
    ) {
        self.remote_extension_manifest_sync = status;
    }

    pub fn grant_extension(&mut self, grant: ExtensionGrant) {
        self.extension_grants
            .retain(|existing| !(existing.kind == grant.kind && existing.name == grant.name));
        self.extension_grants.push(grant);
        self.extension_grants.sort();
    }

    pub fn revoke_extension(&mut self, kind: ExtensionKind, name: &str) {
        self.extension_grants
            .retain(|grant| !(grant.kind == kind && grant.name == name));
    }

    pub fn grant_mcp(&mut self, name: impl Into<String>) {
        self.grant_extension(ExtensionGrant::new(ExtensionKind::Mcp, name));
    }

    pub fn revoke_mcp(&mut self, name: &str) {
        self.revoke_extension(ExtensionKind::Mcp, name);
    }

    pub fn grant_skill(&mut self, name: impl Into<String>) {
        self.grant_extension(ExtensionGrant::new(ExtensionKind::Skill, name));
    }

    pub fn revoke_skill(&mut self, name: &str) {
        self.revoke_extension(ExtensionKind::Skill, name);
    }

    pub fn grant_script(&mut self, name: impl Into<String>, environment: impl Into<String>) {
        self.grant_extension(ExtensionGrant::script(name, environment));
    }

    pub fn revoke_script(&mut self, name: &str) {
        self.revoke_extension(ExtensionKind::Script, name);
    }

    pub fn grant_connector(
        &mut self,
        name: impl Into<String>,
        credential: Option<String>,
        max_safety: impl Into<String>,
    ) {
        self.grant_extension(ExtensionGrant::connector(name, credential, max_safety));
    }

    pub fn revoke_connector(&mut self, name: &str) {
        self.revoke_extension(ExtensionKind::Connector, name);
    }

    pub fn add_substitute(&mut self, profile: AgentSubstituteProfile) {
        self.substitutes.push(profile);
        self.last_activity_at_ms = crate::session::unix_epoch_ms();
    }

    pub fn remove_substitute(&mut self, index: usize) -> Option<AgentSubstituteProfile> {
        if index >= self.substitutes.len() {
            return None;
        }
        let removed = self.substitutes.remove(index);
        match self.active_substitute_index {
            Some(active) if active == index => self.deactivate_substitute(),
            Some(active) if active > index => {
                let shifted_active = active - 1;
                self.active_substitute_index = Some(shifted_active);
                if let Some(record) = self.last_substitution.as_mut() {
                    record.substitute_index = shifted_active;
                }
                self.last_activity_at_ms = crate::session::unix_epoch_ms();
            }
            _ => self.last_activity_at_ms = crate::session::unix_epoch_ms(),
        }
        Some(removed)
    }

    pub fn move_substitute(&mut self, from_index: usize, to_index: usize) -> bool {
        if from_index >= self.substitutes.len() || to_index >= self.substitutes.len() {
            return false;
        }
        if from_index == to_index {
            return true;
        }
        let profile = self.substitutes.remove(from_index);
        self.substitutes.insert(to_index, profile);
        if let Some(active) = self.active_substitute_index {
            let moved_active = if active == from_index {
                to_index
            } else if from_index < active && active <= to_index {
                active - 1
            } else if to_index <= active && active < from_index {
                active + 1
            } else {
                active
            };
            self.active_substitute_index = Some(moved_active);
            if let Some(record) = self.last_substitution.as_mut() {
                record.substitute_index = moved_active;
            }
        }
        self.last_activity_at_ms = crate::session::unix_epoch_ms();
        true
    }

    pub fn clear_substitutes(&mut self) {
        let had_active = self.active_substitute_index.is_some();
        self.substitutes.clear();
        if had_active {
            self.deactivate_substitute();
        } else {
            self.active_substitute_index = None;
            self.last_substitution = None;
            self.last_activity_at_ms = crate::session::unix_epoch_ms();
        }
    }

    pub fn set_substitution_timeout_ms(&mut self, timeout_ms: Option<u64>) {
        self.substitution_timeout_ms = timeout_ms;
        self.last_activity_at_ms = crate::session::unix_epoch_ms();
    }

    pub fn activate_substitute(
        &mut self,
        index: usize,
        reason: impl Into<String>,
    ) -> Option<AgentSubstituteProfile> {
        let profile = self.substitutes.get(index)?.clone();
        if self.active_substitute_index.is_none() {
            // Entering substitution from the primary profile: snapshot the full
            // primary profile so returning to primary restores it exactly, even
            // across persistence restarts.
            self.primary_provider = Some(self.provider.clone());
            self.primary_model = self.model.clone();
            self.primary_effort = self.effort.clone();
            self.primary_account_profile = self.account_profile.clone();
        }
        let account_profile = normalized_agent_account_profile(profile.account_profile.clone());
        self.provider = profile.provider.clone();
        self.model = Some(profile.model.clone());
        self.effort = profile.variant.clone();
        self.account_profile = account_profile;
        self.active_substitute_index = Some(index);
        self.last_substitution = Some(AgentSubstitutionRecord {
            substitute_index: index,
            reason: reason.into(),
            activated_at_ms: crate::session::unix_epoch_ms(),
        });
        self.last_activity_at_ms = crate::session::unix_epoch_ms();
        Some(profile)
    }

    pub fn deactivate_substitute(&mut self) {
        let was_active = self.active_substitute_index.take().is_some();
        self.provider = self.primary_provider().to_string();
        self.model = self.primary_model().map(str::to_string);
        self.effort = self.primary_effort().map(str::to_string);
        if was_active || self.primary_provider.is_some() {
            self.account_profile = normalized_agent_account_profile(
                self.primary_account_profile().map(str::to_string),
            );
        }
        self.last_substitution = None;
        self.last_activity_at_ms = crate::session::unix_epoch_ms();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateAgentRequest {
    pub session_id: String,
    #[serde(default = "default_agent_owner_user_id")]
    pub owner_user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controlled_by_metaagent_id: Option<String>,
    #[serde(default)]
    pub role: AgentRole,
    pub alias: Option<String>,
    pub provider: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_profile: Option<String>,
    pub execution_mode_override: Option<AgentExecutionMode>,
    pub permission_level_override: Option<AgentPermissionLevel>,
    pub worktree_id: Option<String>,
    pub kernel_ref: Option<String>,
    pub worktree_placement: Option<GitWorktreePlacement>,
}

impl CreateAgentRequest {
    pub fn new(session_id: impl Into<String>, provider: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            owner_user_id: default_agent_owner_user_id(),
            controlled_by_metaagent_id: None,
            role: AgentRole::Standard,
            alias: None,
            provider: provider.into(),
            model: None,
            effort: None,
            account_profile: None,
            execution_mode_override: None,
            permission_level_override: None,
            worktree_id: None,
            kernel_ref: None,
            worktree_placement: None,
        }
    }

    pub fn with_alias(mut self, alias: impl Into<String>) -> Self {
        self.alias = Some(alias.into());
        self
    }

    pub fn with_owner_user_id(mut self, owner_user_id: impl Into<String>) -> Self {
        self.owner_user_id = owner_user_id.into();
        self
    }

    pub fn with_controlled_by_metaagent_id(mut self, metaagent_id: impl Into<String>) -> Self {
        self.controlled_by_metaagent_id = Some(metaagent_id.into());
        self
    }

    pub fn with_role(mut self, role: AgentRole) -> Self {
        self.role = role;
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    pub fn with_effort(mut self, effort: impl Into<String>) -> Self {
        self.effort = Some(effort.into());
        self
    }

    pub fn with_account_profile(mut self, account_profile: impl Into<String>) -> Self {
        self.account_profile = normalized_agent_account_profile(Some(account_profile.into()));
        self
    }

    pub fn with_execution_mode_override(mut self, execution_mode: AgentExecutionMode) -> Self {
        self.execution_mode_override = Some(execution_mode);
        self
    }

    pub fn with_permission_level_override(
        mut self,
        permission_level: AgentPermissionLevel,
    ) -> Self {
        self.permission_level_override = Some(permission_level);
        self
    }

    pub fn with_worktree(mut self, worktree_id: impl Into<String>) -> Self {
        self.worktree_id = Some(worktree_id.into());
        self
    }

    pub fn with_kernel(mut self, kernel_ref: impl Into<String>) -> Self {
        self.kernel_ref = Some(kernel_ref.into());
        self
    }

    pub fn with_worktree_placement(mut self, placement: GitWorktreePlacement) -> Self {
        self.worktree_placement = Some(placement);
        self
    }
}

fn default_agent_owner_user_id() -> String {
    DEFAULT_LOCAL_USER_ID.to_string()
}

fn default_visible_in_freeform() -> bool {
    true
}

fn is_default_visible_in_freeform(value: &bool) -> bool {
    *value
}

fn normalized_agent_account_profile(account_profile: Option<String>) -> Option<String> {
    account_profile
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && value != "default")
}

/// Calculates grid layout for agents based on count.
/// Layout progression:
/// - 1 agent: full screen (2x2)
/// - 2 agents: split vertically (1x2)
/// - 3 agents: split horizontally, leave 1 empty (2x2 with 1 slot)
/// - 4 agents: fill 2x2 grid
/// - 5+ agents: expand the two-row grid horizontally as needed
pub fn calculate_agent_layout(agent_count: usize) -> Vec<GridPosition> {
    match agent_count {
        1 => vec![GridPosition::new(0, 0, 2, 2)],
        2 => vec![GridPosition::new(0, 0, 2, 1), GridPosition::new(0, 1, 2, 1)],
        count => {
            let column_count = count.div_ceil(2);
            let mut positions = Vec::with_capacity(count);
            for index in 0..count {
                let row = if index < column_count { 0 } else { 1 };
                let col = if index < column_count {
                    index
                } else {
                    index - column_count
                };
                positions.push(GridPosition::new(row as u32, col as u32, 1, 1));
            }
            positions
        }
    }
}

/// Recalculate positions for all agents after adding/removing
pub fn recalculate_positions(agents: &mut [AgentInstance]) {
    let positions = calculate_agent_layout(agents.len());
    for (i, agent) in agents.iter_mut().enumerate() {
        if let Some(position) = positions.get(i) {
            agent.set_position(position.clone());
        }
    }
}

/// Generate a git-like agent reference (8-char hex)
pub fn generate_agent_ref() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let hex_chars: Vec<char> = (0..8)
        .map(|_| rng.gen_range(0..16))
        .map(|n| std::char::from_digit(n, 16).unwrap())
        .collect();
    hex_chars.into_iter().collect()
}

#[cfg(test)]
mod tests;
