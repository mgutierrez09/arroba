use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchProviderRunRequest {
    pub session_id: String,
    pub agent_id: Option<String>,
    pub adapter_key: String,
    pub provider: String,
    pub account_profile: String,
    pub model: String,
    pub variant: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_session_id: Option<String>,
    #[serde(default)]
    pub native_tui: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchProviderRunsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrency: Option<usize>,
    pub launches: Vec<LaunchProviderRunRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRunBatchLaunchResult {
    pub index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub provider_run: RuntimeProviderRun,
    #[serde(default)]
    pub reused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateAgentConfigRequest {
    pub session_id: String,
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<crate::provider::AgentExecutionMode>,
    #[serde(default)]
    pub clear_execution_mode: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_level: Option<crate::provider::AgentPermissionLevel>,
    #[serde(default)]
    pub clear_permission_level: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub clear_workspace_id: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<String>,
    #[serde(default)]
    pub clear_worktree_id: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateAgentProfileRequest {
    pub session_id: String,
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default)]
    pub clear_effort: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentSubstituteAction {
    Add {
        provider: String,
        model: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        variant: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account_profile: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kernel_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        worktree_id: Option<String>,
    },
    Remove {
        index: usize,
    },
    Move {
        from_index: usize,
        to_index: usize,
    },
    Clear {},
    SetTimeout {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    Activate {
        index: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    Primary {},
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateAgentSubstitutesRequest {
    pub session_id: String,
    pub agent_id: String,
    pub action: AgentSubstituteAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetProviderRunRequest {
    pub provider_run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateProviderRunSelectionRequest {
    pub session_id: String,
    pub provider_run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    #[serde(default)]
    pub clear_variant: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetProviderCatalogRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Optional provider-profile overrides. Providers omitted here resolve to
    /// the owner's registered default; values are stable registry profile IDs.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub account_profiles: BTreeMap<String, String>,
    #[serde(default)]
    pub execution_location: ProviderCatalogExecutionLocation,
}

impl Default for GetProviderCatalogRequest {
    fn default() -> Self {
        Self {
            provider: None,
            account_profiles: BTreeMap::new(),
            execution_location: ProviderCatalogExecutionLocation::Local,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderCatalogExecutionLocation {
    Local,
    Worker { kernel_ref: String },
    Slice { slice_ref: String },
}

impl Default for ProviderCatalogExecutionLocation {
    fn default() -> Self {
        Self::Local
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetProviderCommandCatalogsRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetProviderAuthStatusRequest {
    pub provider: String,
    pub account_profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartProviderLoginRequest {
    pub provider: String,
    pub account_profile: String,
    /// Optional client-selected enrollment method (public names such as
    /// `device_code` or `terminal`). `None` keeps the provider's historical
    /// default. Validated against what the provider adapter supports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetProviderLoginStatusRequest {
    pub login_id: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendProviderLoginInputRequest {
    pub login_id: String,
    pub data_base64: String,
}

impl std::fmt::Debug for SendProviderLoginInputRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SendProviderLoginInputRequest")
            .field("login_id", &self.login_id)
            .field("data_base64", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelProviderLoginRequest {
    pub login_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderLoginProcessState {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderLoginStatus {
    pub provider: String,
    pub account_profile: String,
    pub login_id: String,
    pub state: ProviderLoginProcessState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interaction: Option<crate::session::RuntimeInteraction>,
    /// Ephemeral PTY output. It is never written to durable state or logs.
    pub terminal_output_base64: String,
    pub started_at_ms: u64,
    pub updated_at_ms: u64,
}

impl std::fmt::Debug for ProviderLoginStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderLoginStatus")
            .field("provider", &self.provider)
            .field("account_profile", &self.account_profile)
            .field("login_id", &self.login_id)
            .field("state", &self.state)
            .field("interaction", &self.interaction)
            .field("terminal_output_base64", &"[REDACTED]")
            .field("started_at_ms", &self.started_at_ms)
            .field("updated_at_ms", &self.updated_at_ms)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogoutProviderRequest {
    pub provider: String,
    pub account_profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListProviderAccountProfilesRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetProviderAccountProfileRequest {
    pub provider: String,
    pub account_profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateProviderAccountProfileRequest {
    pub provider: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkProviderAccountProfileRequest {
    pub provider: String,
    pub label: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportNativeProviderAccountProfileRequest {
    pub provider: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenameProviderAccountProfileRequest {
    pub provider: String,
    pub account_profile: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetDefaultProviderAccountProfileRequest {
    pub provider: String,
    pub account_profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshProviderAccountProfileRequest {
    pub provider: String,
    pub account_profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoveProviderAccountProfileRequest {
    pub provider: String,
    pub account_profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteProviderAccountProfileDataRequest {
    pub provider: String,
    pub account_profile: String,
    pub confirmation_profile_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListProviderProcessesRequest {
    pub provider: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeardownProviderProcessesRequest {
    pub provider: Option<String>,
    #[serde(default)]
    pub force: bool,
}
