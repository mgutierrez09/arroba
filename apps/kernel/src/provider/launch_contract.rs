use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::mcp::CharioxMcpServerConfig;
use crate::session::DEFAULT_LOCAL_USER_ID;

use super::types::{
    AgentEndpointMode, ControlCapability, ControlCapabilityMode, ControlOperation,
    ProviderClientInterface,
};

pub(super) fn default_provider_owner_user_id() -> String {
    DEFAULT_LOCAL_USER_ID.to_string()
}

const EXTERNAL_PROVIDER_SESSION_PROVIDERS: &[&str] = &["codex", "claude", "opencode"];

pub fn external_provider_session_providers() -> &'static [&'static str] {
    EXTERNAL_PROVIDER_SESSION_PROVIDERS
}

pub fn canonical_external_provider_session_id(
    provider: &str,
    provider_session_id: &str,
) -> Option<String> {
    let provider = provider.trim().to_ascii_lowercase();
    let provider_session_id = provider_session_id.trim();
    (!provider_session_id.is_empty()
        && EXTERNAL_PROVIDER_SESSION_PROVIDERS.contains(&provider.as_str()))
    .then(|| format!("{provider}:{provider_session_id}"))
}

pub fn canonical_profile_external_provider_session_id(
    provider: &str,
    account_profile: &str,
    provider_session_id: &str,
) -> Option<String> {
    let provider = provider.trim().to_ascii_lowercase();
    let account_profile = account_profile.trim();
    let provider_session_id = provider_session_id.trim();
    (!account_profile.is_empty()
        && !provider_session_id.is_empty()
        && EXTERNAL_PROVIDER_SESSION_PROVIDERS.contains(&provider.as_str()))
    .then(|| format!("{provider}:{account_profile}:{provider_session_id}"))
}

pub fn external_provider_import_model(provider: &str, requested_model: Option<String>) -> String {
    requested_model.unwrap_or_else(|| match provider.trim().to_ascii_lowercase().as_str() {
        "codex" => "default".to_string(),
        "claude" => "claude-sonnet-4-6".to_string(),
        _ => "default".to_string(),
    })
}

pub fn normalize_provider_resume_model(provider: &str, model: &str) -> String {
    let trimmed = model.trim();
    match provider.trim().to_ascii_lowercase().as_str() {
        "codex" => trimmed
            .strip_prefix("codex/")
            .unwrap_or(trimmed)
            .to_string(),
        _ => trimmed.to_string(),
    }
}

pub fn provider_resume_failure_notice(provider: &str, provider_session_id: &str) -> Option<String> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "codex" => Some(format!(
            "Codex resume thread `{provider_session_id}` is no longer available. Chariox cleared it from the agent profile so the next prompt can start a new durable Codex thread."
        )),
        _ => None,
    }
}

pub fn provider_uses_inferred_runtime_mcp_binding(provider: &str) -> bool {
    matches!(
        provider.trim().to_ascii_lowercase().as_str(),
        "claude" | "codex" | "opencode"
    )
}

pub fn default_provider_control_capabilities(
    provider: &str,
    has_runtime_mcp_binding: bool,
) -> Vec<ControlCapability> {
    let provider = provider.trim().to_ascii_lowercase();
    let mut capabilities = Vec::new();

    if provider_uses_inferred_runtime_mcp_binding(&provider) {
        capabilities.push(ControlCapability::new(
            ControlOperation::InterruptTurn,
            ControlCapabilityMode::Native,
        ));
        capabilities.push(ControlCapability::new(
            ControlOperation::CancelPrompt,
            ControlCapabilityMode::Native,
        ));
    }

    if provider == "dev-stub" {
        capabilities.push(ControlCapability::new(
            ControlOperation::AckWorkflowTurn,
            ControlCapabilityMode::AdapterEmulated,
        ));
        capabilities.push(ControlCapability::new(
            ControlOperation::ValidateWorkflowHandoff,
            ControlCapabilityMode::AdapterEmulated,
        ));
    } else if has_runtime_mcp_binding {
        capabilities.push(ControlCapability::new(
            ControlOperation::AckWorkflowTurn,
            ControlCapabilityMode::Mcp,
        ));
        capabilities.push(ControlCapability::new(
            ControlOperation::ValidateWorkflowHandoff,
            ControlCapabilityMode::Mcp,
        ));
    }

    capabilities
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderResumeState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    opencode_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    codex_thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    claude_session_id: Option<String>,
}

impl ProviderResumeState {
    pub fn is_empty(&self) -> bool {
        self.opencode_session_id.is_none()
            && self.codex_thread_id.is_none()
            && self.claude_session_id.is_none()
    }

    pub fn from_opencode_session_id(session_id: impl Into<String>) -> Self {
        let mut state = Self::default();
        state.set_opencode_session_id(session_id);
        state
    }

    pub fn from_codex_thread_id(thread_id: impl Into<String>) -> Self {
        let mut state = Self::default();
        state.set_codex_thread_id(thread_id);
        state
    }

    pub fn from_claude_session_id(session_id: impl Into<String>) -> Self {
        let mut state = Self::default();
        state.set_claude_session_id(session_id);
        state
    }

    pub fn opencode_session_id(&self) -> Option<&str> {
        self.opencode_session_id.as_deref()
    }

    pub fn codex_thread_id(&self) -> Option<&str> {
        self.codex_thread_id.as_deref()
    }

    pub fn claude_session_id(&self) -> Option<&str> {
        self.claude_session_id.as_deref()
    }

    pub fn provider_session_id(&self, provider: &str) -> Option<&str> {
        match provider.trim().to_ascii_lowercase().as_str() {
            "codex" => self.codex_thread_id(),
            "opencode" => self.opencode_session_id(),
            "claude" => self.claude_session_id(),
            _ => None,
        }
    }

    pub fn set_opencode_session_id(&mut self, session_id: impl Into<String>) {
        self.opencode_session_id = Some(session_id.into());
    }

    pub fn set_codex_thread_id(&mut self, thread_id: impl Into<String>) {
        self.codex_thread_id = Some(thread_id.into());
    }

    pub fn set_claude_session_id(&mut self, session_id: impl Into<String>) {
        self.claude_session_id = Some(session_id.into());
    }

    pub(crate) fn set_provider_session_id(
        &mut self,
        provider: &str,
        session_id: impl Into<String>,
    ) -> bool {
        let session_id = session_id.into();
        match provider.trim().to_ascii_lowercase().as_str() {
            "codex" => self.set_codex_thread_id(session_id),
            "opencode" => self.set_opencode_session_id(session_id),
            "claude" => self.set_claude_session_id(session_id),
            _ => return false,
        }
        true
    }

    pub fn without_opencode_session_id(&self) -> Self {
        Self {
            opencode_session_id: None,
            codex_thread_id: self.codex_thread_id.clone(),
            claude_session_id: self.claude_session_id.clone(),
        }
    }

    pub fn without_codex_thread_id(&self) -> Self {
        Self {
            opencode_session_id: self.opencode_session_id.clone(),
            codex_thread_id: None,
            claude_session_id: self.claude_session_id.clone(),
        }
    }

    pub fn without_claude_session_id(&self) -> Self {
        Self {
            opencode_session_id: self.opencode_session_id.clone(),
            codex_thread_id: self.codex_thread_id.clone(),
            claude_session_id: None,
        }
    }

    pub fn without_provider_session_id(&self, provider: &str) -> Self {
        match provider.trim().to_ascii_lowercase().as_str() {
            "codex" => self.without_codex_thread_id(),
            "opencode" => self.without_opencode_session_id(),
            "claude" => self.without_claude_session_id(),
            _ => self.clone(),
        }
    }

    pub fn replacement_after_provider_resume_failure(
        &self,
        provider: &str,
        operation: &str,
    ) -> Option<Self> {
        if self.provider_session_id(provider).is_none() {
            return None;
        }
        match (
            provider.trim().to_ascii_lowercase().as_str(),
            operation.trim(),
        ) {
            ("codex", "codex_thread_resume" | "thread/resume") => {
                Some(self.without_provider_session_id(provider))
            }
            ("opencode", "provider_stream/network_error") => {
                Some(self.without_provider_session_id(provider))
            }
            ("opencode", "provider_stream/empty_idle_assistant") => {
                Some(self.without_provider_session_id(provider))
            }
            _ => None,
        }
    }

    pub fn with_opencode_resume_state(&self, opencode_resume_state: &Self) -> Self {
        Self {
            opencode_session_id: opencode_resume_state.opencode_session_id.clone(),
            codex_thread_id: self.codex_thread_id.clone(),
            claude_session_id: self.claude_session_id.clone(),
        }
    }

    pub fn from_external_provider_session(
        provider: &str,
        provider_session_id: impl Into<String>,
    ) -> Self {
        match provider.trim().to_ascii_lowercase().as_str() {
            "codex" => Self::from_codex_thread_id(provider_session_id),
            "opencode" => Self::from_opencode_session_id(provider_session_id),
            "claude" => Self::from_claude_session_id(provider_session_id),
            _ => Self::default(),
        }
    }

    pub fn external_provider_sessions(&self) -> Vec<(&'static str, &str)> {
        let mut sessions = Vec::new();
        if let Some(provider_session_id) = self.codex_thread_id() {
            sessions.push(("codex", provider_session_id));
        }
        if let Some(provider_session_id) = self.claude_session_id() {
            sessions.push(("claude", provider_session_id));
        }
        if let Some(provider_session_id) = self.opencode_session_id() {
            sessions.push(("opencode", provider_session_id));
        }
        sessions
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalProviderObservedCursor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_observed_turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_observed_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_observed_merge_key: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub chariox_owned_observed_prompt_turn_ids: BTreeSet<String>,
}

impl ExternalProviderObservedCursor {
    pub fn new(
        last_observed_turn_id: Option<String>,
        last_observed_at_ms: Option<u64>,
        last_observed_merge_key: Option<String>,
    ) -> Self {
        Self {
            last_observed_turn_id,
            last_observed_at_ms,
            last_observed_merge_key,
            chariox_owned_observed_prompt_turn_ids: BTreeSet::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.last_observed_turn_id.is_none()
            && self.last_observed_at_ms.is_none()
            && self.last_observed_merge_key.is_none()
            && self.chariox_owned_observed_prompt_turn_ids.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalProviderImportMetadata {
    pub external_provider_session_id: String,
    pub external_provider: String,
    pub external_provider_session_provider_id: String,
    #[serde(default = "default_external_provider_account_profile")]
    pub account_profile: String,
    #[serde(
        default,
        skip_serializing_if = "ExternalProviderObservedCursor::is_empty"
    )]
    pub observed_cursor: ExternalProviderObservedCursor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_observed_turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_observed_at_ms: Option<u64>,
    pub imported_at_ms: u64,
}

impl ExternalProviderImportMetadata {
    pub fn observed_history(
        external_provider_session_id: impl Into<String>,
        external_provider: impl Into<String>,
        external_provider_session_provider_id: impl Into<String>,
    ) -> Self {
        let external_provider = external_provider.into().trim().to_ascii_lowercase();
        let external_provider_session_provider_id = external_provider_session_provider_id
            .into()
            .trim()
            .to_string();
        let external_provider_session_id = external_provider_session_id.into().trim().to_string();
        let external_provider_session_id = canonical_profile_external_provider_session_id(
            &external_provider,
            "default",
            &external_provider_session_provider_id,
        )
        .unwrap_or(external_provider_session_id);
        Self {
            external_provider_session_id,
            external_provider,
            external_provider_session_provider_id,
            account_profile: "default".to_string(),
            observed_cursor: ExternalProviderObservedCursor::default(),
            last_observed_turn_id: None,
            last_observed_at_ms: None,
            imported_at_ms: crate::session::unix_epoch_ms(),
        }
    }

    pub fn observed_history_for_profile(
        external_provider: impl Into<String>,
        account_profile: &str,
        external_provider_session_provider_id: impl Into<String>,
    ) -> Self {
        let external_provider = external_provider.into().trim().to_ascii_lowercase();
        let provider_session_id = external_provider_session_provider_id
            .into()
            .trim()
            .to_string();
        let external_provider_session_id = canonical_profile_external_provider_session_id(
            &external_provider,
            account_profile,
            &provider_session_id,
        )
        .or_else(|| {
            canonical_external_provider_session_id(&external_provider, &provider_session_id)
        })
        .unwrap_or_else(|| format!("{external_provider}:{provider_session_id}"));
        Self {
            external_provider_session_id,
            external_provider,
            external_provider_session_provider_id: provider_session_id,
            account_profile: account_profile.to_string(),
            observed_cursor: ExternalProviderObservedCursor::default(),
            last_observed_turn_id: None,
            last_observed_at_ms: None,
            imported_at_ms: crate::session::unix_epoch_ms(),
        }
    }

    pub fn with_cursor(mut self, cursor: ExternalProviderObservedCursor) -> Self {
        self.last_observed_turn_id = cursor.last_observed_turn_id.clone();
        self.last_observed_at_ms = cursor.last_observed_at_ms;
        self.observed_cursor = cursor;
        self
    }

    pub fn same_observed_provider_session(&self, other: &Self) -> bool {
        self.external_provider_session_id == other.external_provider_session_id
            && self.external_provider == other.external_provider
            && self.account_profile == other.account_profile
            && self.external_provider_session_provider_id
                == other.external_provider_session_provider_id
    }

    pub fn import_order_key(&self) -> (u64, &str) {
        (
            self.imported_at_ms,
            self.external_provider_session_id.as_str(),
        )
    }
}

fn default_external_provider_account_profile() -> String {
    "default".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchProviderRequest {
    pub session_id: String,
    pub agent_id: Option<String>,
    #[serde(default = "default_provider_owner_user_id")]
    pub owner_user_id: String,
    pub adapter_key: String,
    pub provider: String,
    pub account_profile: String,
    pub model: String,
    pub variant: Option<String>,
    pub working_directory: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspace_live_sync_roots: Vec<PathBuf>,
    pub runtime_mcp_binding: Option<RuntimeMcpBinding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_servers: Vec<CharioxMcpServerConfig>,
    #[serde(
        default,
        skip_serializing_if = "crate::extension::RemoteExtensionManifest::is_empty"
    )]
    pub remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provider_config_overrides: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_env_remove: Vec<String>,
    /// Host-local account-root environment resolved from `account_profile`.
    /// It is never serialized across client or relay boundaries; each
    /// execution kernel resolves the stable profile id against its registry.
    #[serde(skip)]
    pub(crate) provider_account_env: BTreeMap<String, String>,
    /// Vault-resolved values for this in-flight launch. This is excluded from
    /// every serialized request shape and uses a redacted debug projection.
    #[serde(skip)]
    pub(crate) provider_credential_env: super::ProviderCredentialEnvironment,
    #[serde(
        default,
        skip_serializing_if = "ProviderWriteAccessMode::is_unrestricted"
    )]
    pub write_access_mode: ProviderWriteAccessMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<AgentExecutionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_level: Option<AgentPermissionLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_state: Option<ProviderResumeState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "ProviderClientInterface::is_chariox")]
    pub client_interface: ProviderClientInterface,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_provider_import: Option<ExternalProviderImportMetadata>,
    /// Workflow-only capability snapshot. This is intentionally omitted from
    /// the wire shape unless enabled; it controls whether the provider may
    /// discover the event reply action for this run.
    #[serde(default, skip_serializing_if = "is_false")]
    pub workflow_event_reply_enabled: bool,
    /// Workflow-only capability snapshot for bounded provider event context.
    /// This is independent from reply mode: an event may permit context reads
    /// while replies remain disabled.
    #[serde(default, skip_serializing_if = "is_false")]
    pub workflow_event_context_enabled: bool,
    /// Workflow-only capability snapshot for explicitly enabled provider actions.
    #[serde(default, skip_serializing_if = "is_false")]
    pub workflow_event_actions_enabled: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentExecutionMode {
    #[default]
    Build,
    Plan,
}

impl AgentExecutionMode {
    pub fn is_build(&self) -> bool {
        matches!(self, Self::Build)
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "build" => Some(Self::Build),
            "plan" => Some(Self::Plan),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Plan => "plan",
        }
    }
}

impl fmt::Display for AgentExecutionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentPermissionLevel {
    Required,
    #[default]
    Yolo,
}

impl AgentPermissionLevel {
    pub fn is_yolo(&self) -> bool {
        matches!(self, Self::Yolo)
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "required" => Some(Self::Required),
            "yolo" => Some(Self::Yolo),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::Yolo => "yolo",
        }
    }
}

impl fmt::Display for AgentPermissionLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderWriteAccessMode {
    #[default]
    Unrestricted,
    WorkspaceLiveSyncManaged,
    WorkspaceLiveSyncTracked,
}

impl ProviderWriteAccessMode {
    pub fn is_unrestricted(&self) -> bool {
        matches!(self, Self::Unrestricted)
    }

    pub fn requires_workspace_live_sync(&self) -> bool {
        matches!(self, Self::WorkspaceLiveSyncManaged)
    }

    pub fn tracks_workspace_live_sync(&self) -> bool {
        matches!(self, Self::WorkspaceLiveSyncTracked)
    }

    pub fn uses_workspace_live_sync(&self) -> bool {
        self.requires_workspace_live_sync() || self.tracks_workspace_live_sync()
    }

    pub fn from_config_mode(mode: crate::config::WorkspaceLiveSyncMode) -> Self {
        match mode {
            crate::config::WorkspaceLiveSyncMode::Managed => Self::WorkspaceLiveSyncManaged,
            crate::config::WorkspaceLiveSyncMode::Tracked => Self::WorkspaceLiveSyncTracked,
            crate::config::WorkspaceLiveSyncMode::Unrestricted => Self::Unrestricted,
        }
    }
}

impl LaunchProviderRequest {
    pub fn new(
        session_id: impl Into<String>,
        adapter_key: impl Into<String>,
        provider: impl Into<String>,
        account_profile: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            agent_id: None,
            owner_user_id: default_provider_owner_user_id(),
            adapter_key: adapter_key.into(),
            provider: provider.into(),
            account_profile: account_profile.into(),
            model: model.into(),
            variant: None,
            working_directory: None,
            workspace_live_sync_roots: Vec::new(),
            runtime_mcp_binding: None,
            mcp_servers: Vec::new(),
            remote_extension_manifest: crate::extension::RemoteExtensionManifest::default(),
            provider_config_overrides: BTreeMap::new(),
            provider_env_remove: Vec::new(),
            provider_account_env: BTreeMap::new(),
            provider_credential_env: super::ProviderCredentialEnvironment::default(),
            write_access_mode: ProviderWriteAccessMode::Unrestricted,
            execution_mode: None,
            permission_level: None,
            resume_state: None,
            structured_endpoint: None,
            client_interface: ProviderClientInterface::Chariox,
            external_provider_import: None,
            workflow_event_reply_enabled: false,
            workflow_event_context_enabled: false,
            workflow_event_actions_enabled: false,
        }
    }

    pub fn with_variant(mut self, variant: Option<String>) -> Self {
        self.variant = variant.and_then(|value| {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_string())
        });
        self
    }

    pub fn with_agent_id(mut self, agent_id: impl Into<String>) -> Self {
        self.agent_id = Some(agent_id.into());
        self
    }

    pub fn with_owner_user_id(mut self, owner_user_id: impl Into<String>) -> Self {
        self.owner_user_id = owner_user_id.into();
        self
    }

    pub fn with_working_directory(mut self, working_directory: PathBuf) -> Self {
        self.working_directory = Some(working_directory);
        self
    }

    pub fn with_workspace_live_sync_roots(mut self, roots: Vec<PathBuf>) -> Self {
        self.workspace_live_sync_roots = roots;
        self
    }

    pub fn with_runtime_mcp_binding(mut self, binding: RuntimeMcpBinding) -> Self {
        self.runtime_mcp_binding = Some(binding);
        self
    }

    pub fn with_mcp_servers(mut self, mcp_servers: Vec<CharioxMcpServerConfig>) -> Self {
        self.mcp_servers = mcp_servers;
        self
    }

    pub fn with_remote_extension_manifest(
        mut self,
        manifest: crate::extension::RemoteExtensionManifest,
    ) -> Self {
        self.remote_extension_manifest = manifest;
        self
    }

    pub fn with_provider_config_override(mut self, key: impl Into<String>, value: Value) -> Self {
        self.provider_config_overrides.insert(key.into(), value);
        self
    }

    pub fn with_provider_config_overrides(mut self, overrides: BTreeMap<String, Value>) -> Self {
        self.provider_config_overrides = overrides;
        self
    }

    pub fn with_provider_env_remove(mut self, env_remove: Vec<String>) -> Self {
        self.provider_env_remove = env_remove;
        self
    }

    pub(crate) fn with_provider_account_env(
        mut self,
        environment: BTreeMap<String, String>,
    ) -> Self {
        self.provider_account_env = environment;
        self
    }

    pub(crate) fn with_provider_credential_env(
        mut self,
        environment: super::ProviderCredentialEnvironment,
    ) -> Self {
        self.provider_credential_env = environment;
        self
    }

    pub fn with_workspace_live_sync_managed(mut self) -> Self {
        self.write_access_mode = ProviderWriteAccessMode::WorkspaceLiveSyncManaged;
        self
    }

    pub fn with_workspace_live_sync_mode(
        mut self,
        mode: crate::config::WorkspaceLiveSyncMode,
    ) -> Self {
        self.write_access_mode = ProviderWriteAccessMode::from_config_mode(mode);
        self
    }

    pub fn with_execution_mode(mut self, execution_mode: AgentExecutionMode) -> Self {
        self.execution_mode = Some(execution_mode);
        self
    }

    pub fn with_permission_level(mut self, permission_level: AgentPermissionLevel) -> Self {
        self.permission_level = Some(permission_level);
        self
    }

    pub fn requires_workspace_live_sync(&self) -> bool {
        self.write_access_mode.requires_workspace_live_sync()
    }

    pub fn tracks_workspace_live_sync(&self) -> bool {
        self.write_access_mode.tracks_workspace_live_sync()
    }

    pub fn uses_workspace_live_sync(&self) -> bool {
        self.write_access_mode.uses_workspace_live_sync()
    }

    pub fn with_resume_state(mut self, resume_state: ProviderResumeState) -> Self {
        self.resume_state = (!resume_state.is_empty()).then_some(resume_state);
        self
    }

    pub fn with_structured_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.structured_endpoint = Some(endpoint.into());
        self
    }

    pub fn with_client_interface(mut self, client_interface: ProviderClientInterface) -> Self {
        self.client_interface = client_interface;
        self
    }

    pub fn with_external_provider_import(mut self, import: ExternalProviderImportMetadata) -> Self {
        self.external_provider_import = Some(import);
        self
    }

    pub fn with_workflow_event_reply(mut self, enabled: bool) -> Self {
        self.workflow_event_reply_enabled = enabled;
        self
    }

    pub fn with_workflow_event_context(mut self, enabled: bool) -> Self {
        self.workflow_event_context_enabled = enabled;
        self
    }

    pub fn with_workflow_event_actions(mut self, enabled: bool) -> Self {
        self.workflow_event_actions_enabled = enabled;
        self
    }

    pub(crate) fn matches_existing_run_selection(&self, run: &super::RuntimeProviderRun) -> bool {
        run.session_id() == self.session_id
            && run.agent_instance_id() == self.agent_id.as_deref()
            && run.owner_user_id() == self.owner_user_id
            && run.adapter_key() == self.adapter_key
            && run.provider() == self.provider
            && run.account_profile() == self.account_profile
            && run.model() == self.model
            && run.variant() == self.variant.as_deref()
            && run.client_interface() == self.client_interface
            && run.working_directory() == self.working_directory.as_ref()
            && run.write_access_mode() == self.write_access_mode
            && run.execution_mode() == self.execution_mode.unwrap_or_default()
            && run.permission_level() == self.permission_level.unwrap_or_default()
            && run.provider_config_overrides() == &self.provider_config_overrides
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeMcpBinding {
    pub server_url: String,
    pub auth_token: String,
}

impl RuntimeMcpBinding {
    pub fn new(server_url: impl Into<String>, auth_token: impl Into<String>) -> Self {
        Self {
            server_url: server_url.into(),
            auth_token: auth_token.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderLaunchResult {
    pub endpoint_mode: AgentEndpointMode,
    pub process_label: String,
    pub pty_target: Option<String>,
    pub pty_program: Option<String>,
    pub pty_args: Vec<String>,
    pub pty_env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pty_env_remove: Vec<String>,
    pub working_directory: Option<PathBuf>,
    pub structured_endpoint: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{
        default_provider_control_capabilities, external_provider_import_model,
        normalize_provider_resume_model, provider_resume_failure_notice,
        provider_uses_inferred_runtime_mcp_binding, ExternalProviderImportMetadata,
        LaunchProviderRequest, ProviderResumeState,
    };
    use crate::provider::{ControlCapabilityMode, ControlOperation};
    use zeroize::Zeroizing;

    #[test]
    fn provider_credentials_are_absent_from_wire_and_debug_shapes() {
        let mut credentials = super::super::ProviderCredentialEnvironment::default();
        credentials.insert(
            "CLAUDE_CODE_OAUTH_TOKEN",
            Zeroizing::new("setup-token-secret".to_string()),
        );
        let request = LaunchProviderRequest::new("session-1", "claude", "claude", "work", "sonnet")
            .with_provider_credential_env(credentials);

        let encoded = serde_json::to_string(&request).expect("launch request should serialize");
        let debug = format!("{request:?}");
        assert!(!encoded.contains("CLAUDE_CODE_OAUTH_TOKEN"));
        assert!(!encoded.contains("setup-token-secret"));
        assert!(!debug.contains("CLAUDE_CODE_OAUTH_TOKEN"));
        assert!(!debug.contains("setup-token-secret"));
    }

    #[test]
    fn launch_request_tracks_managed_workspace_live_sync_mode() {
        let request =
            LaunchProviderRequest::new("session-1", "codex", "codex", "default", "default")
                .with_workspace_live_sync_managed();

        assert!(request.requires_workspace_live_sync());
        let json = serde_json::to_value(&request).expect("request should serialize");
        assert_eq!(json["write_access_mode"], "workspace_live_sync_managed");
    }

    #[test]
    fn launch_request_tracks_tracked_workspace_live_sync_mode() {
        let request =
            LaunchProviderRequest::new("session-1", "codex", "codex", "default", "default")
                .with_workspace_live_sync_mode(crate::config::WorkspaceLiveSyncMode::Tracked);

        assert!(!request.requires_workspace_live_sync());
        assert!(request.tracks_workspace_live_sync());
        let json = serde_json::to_value(&request).expect("request should serialize");
        assert_eq!(json["write_access_mode"], "workspace_live_sync_tracked");
    }

    #[test]
    fn external_provider_import_identity_ignores_cursor_and_import_time() {
        let mut first =
            ExternalProviderImportMetadata::observed_history("codex:thread-1", "codex", "thread-1");
        first.imported_at_ms = 1;
        let mut second = first
            .clone()
            .with_cursor(super::ExternalProviderObservedCursor {
                last_observed_turn_id: Some("turn-2".to_string()),
                last_observed_at_ms: Some(2),
                last_observed_merge_key: Some("merge-2".to_string()),
                chariox_owned_observed_prompt_turn_ids: std::collections::BTreeSet::new(),
            });
        second.imported_at_ms = 2;
        let different =
            ExternalProviderImportMetadata::observed_history("codex:thread-2", "codex", "thread-2");

        assert!(first.same_observed_provider_session(&second));
        assert!(!first.same_observed_provider_session(&different));
        assert_eq!(first.import_order_key(), (1, "codex:default:thread-1"));
    }

    #[test]
    fn external_provider_import_identity_is_canonicalized() {
        let import = ExternalProviderImportMetadata::observed_history(
            " Codex : stale-thread ",
            " Codex ",
            " thread-1 ",
        );

        assert_eq!(
            import.external_provider_session_id,
            "codex:default:thread-1"
        );
        assert_eq!(import.external_provider, "codex");
        assert_eq!(import.external_provider_session_provider_id, "thread-1");
    }

    #[test]
    fn external_provider_import_model_uses_provider_contract_defaults() {
        assert_eq!(
            external_provider_import_model("codex", None),
            "default".to_string()
        );
        assert_eq!(
            external_provider_import_model(" Codex ", None),
            "default".to_string()
        );
        assert_eq!(
            external_provider_import_model("claude", None),
            "claude-sonnet-4-6".to_string()
        );
        assert_eq!(
            external_provider_import_model("opencode", None),
            "default".to_string()
        );
        assert_eq!(
            external_provider_import_model("unknown", None),
            "default".to_string()
        );
        assert_eq!(
            external_provider_import_model("claude", Some("custom-model".to_string())),
            "custom-model".to_string()
        );
    }

    #[test]
    fn provider_resume_state_maps_provider_keys_to_session_ids() {
        let mut state = ProviderResumeState::from_codex_thread_id("codex-thread");
        state.set_claude_session_id("claude-session");
        state.set_opencode_session_id("opencode-session");

        assert_eq!(state.provider_session_id(" Codex "), Some("codex-thread"));
        assert_eq!(state.provider_session_id("CLAUDE"), Some("claude-session"));
        assert_eq!(
            state.provider_session_id("opencode"),
            Some("opencode-session")
        );
        assert_eq!(state.provider_session_id("unknown"), None);
    }

    #[test]
    fn provider_resume_state_removes_only_requested_provider_session() {
        let mut state = ProviderResumeState::from_codex_thread_id("codex-thread");
        state.set_claude_session_id("claude-session");
        state.set_opencode_session_id("opencode-session");

        let without_codex = state.without_provider_session_id("codex");
        assert_eq!(without_codex.codex_thread_id(), None);
        assert_eq!(without_codex.claude_session_id(), Some("claude-session"));
        assert_eq!(
            without_codex.opencode_session_id(),
            Some("opencode-session")
        );

        assert_eq!(
            state
                .without_provider_session_id("unknown")
                .provider_session_id("codex"),
            Some("codex-thread")
        );
    }

    #[test]
    fn provider_resume_state_from_external_provider_session_normalizes_provider_key() {
        assert_eq!(
            ProviderResumeState::from_external_provider_session(" Codex ", "thread-1")
                .codex_thread_id(),
            Some("thread-1")
        );
        assert_eq!(
            ProviderResumeState::from_external_provider_session("CLAUDE", "session-1")
                .claude_session_id(),
            Some("session-1")
        );
        assert!(
            ProviderResumeState::from_external_provider_session("unknown", "session-1").is_empty()
        );
    }

    #[test]
    fn provider_resume_state_replacement_after_resume_failure_is_provider_policy() {
        let mut state = ProviderResumeState::from_codex_thread_id("codex-thread");
        state.set_opencode_session_id("opencode-session");

        let replacement = state
            .replacement_after_provider_resume_failure("codex", "codex_thread_resume")
            .expect("Codex resume failures should clear the stale Codex thread id");

        assert_eq!(replacement.codex_thread_id(), None);
        assert_eq!(replacement.opencode_session_id(), Some("opencode-session"));
        assert_eq!(
            state.replacement_after_provider_resume_failure("codex", "other_operation"),
            None
        );
        assert!(state
            .replacement_after_provider_resume_failure("codex", "thread/resume")
            .is_some_and(|replacement| replacement.codex_thread_id().is_none()));
        assert_eq!(
            state.replacement_after_provider_resume_failure("opencode", "codex_thread_resume"),
            None
        );
        let replacement = state
            .replacement_after_provider_resume_failure("opencode", "provider_stream/network_error")
            .expect("OpenCode stream failures should retire the failed provider session");
        assert_eq!(replacement.opencode_session_id(), None);
        assert_eq!(replacement.codex_thread_id(), Some("codex-thread"));
        assert!(provider_resume_failure_notice("codex", "codex-thread")
            .is_some_and(|message| message.contains("codex-thread")));
        assert_eq!(
            provider_resume_failure_notice("opencode", "session-1"),
            None
        );
    }

    #[test]
    fn provider_resume_model_normalization_is_provider_contract_policy() {
        assert_eq!(
            normalize_provider_resume_model("codex", " codex/gpt-test "),
            "gpt-test"
        );
        assert_eq!(
            normalize_provider_resume_model("claude", " claude/sonnet "),
            "claude/sonnet"
        );
    }

    #[test]
    fn provider_control_capability_defaults_are_provider_contract_policy() {
        assert!(provider_uses_inferred_runtime_mcp_binding("codex"));
        assert!(provider_uses_inferred_runtime_mcp_binding("CLAUDE"));
        assert!(provider_uses_inferred_runtime_mcp_binding("opencode"));
        assert!(!provider_uses_inferred_runtime_mcp_binding("dev-stub"));

        let codex = default_provider_control_capabilities("codex", true);
        assert!(codex.iter().any(|capability| {
            capability.operation() == ControlOperation::InterruptTurn
                && capability.mode() == ControlCapabilityMode::Native
        }));
        assert!(codex.iter().any(|capability| {
            capability.operation() == ControlOperation::CancelPrompt
                && capability.mode() == ControlCapabilityMode::Native
        }));
        assert!(codex.iter().any(|capability| {
            capability.operation() == ControlOperation::AckWorkflowTurn
                && capability.mode() == ControlCapabilityMode::Mcp
        }));

        let dev_stub = default_provider_control_capabilities("dev-stub", true);
        assert_eq!(
            dev_stub
                .iter()
                .filter(|capability| capability.mode() == ControlCapabilityMode::AdapterEmulated)
                .count(),
            2
        );

        assert!(default_provider_control_capabilities("unknown", false).is_empty());
    }
}
