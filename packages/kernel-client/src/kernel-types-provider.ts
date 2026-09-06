import type { ExternalProviderImportMetadata, RuntimeSession } from "./kernel-types-session.js"
import type { AgentRuntimeActivity, PromptQueueItem } from "./kernel-types-runtime.js"
import type { SessionHistoryExternalObservation } from "./kernel-types-history.js"

export type RuntimeProviderRun = {
  id: string
  session_id: string
  agent_instance_id: string | null
  adapter_key: string
  provider: string
  account_profile: string
  model: string
  variant: string | null
  usage_tokens_total: number | null
  usage?: {
    total_tokens?: number | null
    last_tokens?: number | null
    context_tokens?: number | null
    context_window?: number | null
  }
  state: string
  endpoint_mode?: string
  client_interface?: "chariox" | "native_tui" | string
  process_label?: string
  structured_endpoint?: string | null
  provider_session_id?: string | null
  working_directory?: string | null
  started_at_ms?: number
  last_activity_at_ms?: number
  control_capabilities?: {
    operation: string
    mode: string
  }[]
  external_provider_import?: ExternalProviderImportMetadata | null
}


export type TerminalCommandCatalogNodeKind =
  | "group"
  | "command"
  | "prompt_prefix"
  | "dynamic"

export type TerminalCommandCatalogExecutionTarget =
  | "kernel"
  | "terminal_local"
  | "prompt_prefix"

export type TerminalCommandCatalogSurface =
  | "session"
  | "waiting_room"
  | "workflow_screen"

export type TerminalCommandCatalogNode = {
  id: string
  label: string
  description: string
  value: string
  kind: TerminalCommandCatalogNodeKind
  execution_target: TerminalCommandCatalogExecutionTarget
  surfaces: TerminalCommandCatalogSurface[]
  search_aliases?: string[]
  intents?: string[]
  examples?: string[]
  dynamic_source?: string | null
  children?: TerminalCommandCatalogNode[]
}

export type TerminalCommandCatalog = {
  revision: string
  nodes: TerminalCommandCatalogNode[]
}

export type DebugBundleExportedResponse = {
  DebugBundleExported: {
    bundle_dir: string
    manifest_path: string
    logs_path: string
    log_root: string
    record_count: number
    limit: number
  }
}

export type AgentUtilityKind = "WorkspaceCommitMessage"

export type WorkspaceCommitMessageUtilityInput = {
  workspace_id: string
  worktree_id: string
  compare_ref?: string | null
}

export type AgentUtilityInput = {
  WorkspaceCommitMessage: WorkspaceCommitMessageUtilityInput
}

export type RunAgentUtilityRequest = {
  session_id: string
  agent_id: string
  kind: AgentUtilityKind
  input: AgentUtilityInput
}

export type AgentUtilityOutput = {
  WorkspaceCommitMessage: {
    message: string
  }
}

export type AgentUtilityResult = {
  utility_run_id: string
  session_id: string
  agent_id: string
  kind: AgentUtilityKind
  output: AgentUtilityOutput
  generated_at_ms: number
}

export type ProviderProcessInfo = {
  process_id: string
  provider: string
  process_label: string
  pid?: number | null
  resident_set_bytes?: number | null
  endpoint_mode: string
  status: "active" | "idle"
  started_at_ms: number
  last_activity_at_ms: number
  provider_session_ids: string[]
  owner_session_ids: string[]
  owner_provider_run_ids: string[]
  attached_session_ids: string[]
  active_workflow_run_ids: string[]
  teardown_safe: boolean
  teardown_blockers: string[]
}

export type ProviderAuthStatus = {
  provider: string
  auth_state: string
  account_profile: string | null
  identity_summary?: string | null
  plan?: string | null
  login_hint: string | null
  detected_version: string | null
}

export type ProviderLoginStart = {
  provider: string
  account_profile: string
  login_kind: string
  login_id: string | null
  auth_url: string | null
  verification_url: string | null
  user_code: string | null
}

export type ProviderLoginProcessState = "running" | "succeeded" | "failed" | "cancelled"

export type ProviderLoginStatus = {
  provider: string
  account_profile: string
  login_id: string
  state: ProviderLoginProcessState
  interaction?: Record<string, unknown> | null
  terminal_output_base64: string
  started_at_ms: number
  updated_at_ms: number
}

export type SliceProviderLoginStart = {
  provider: string
  login_kind: string
  auth_url?: string | null
  verification_url?: string | null
  user_code?: string | null
  status: string
  message: string
}

export type ProviderLogoutResult = {
  provider: string
  account_profile: string
}

export type ProviderLogoutOutcome =
  | { kind: "logged_out"; result: ProviderLogoutResult }
  | { kind: "interaction_required"; workflow: ProviderLoginStart }

export type ProviderAccountUsageAvailability = "available" | "partial" | "unavailable" | "stale" | "error"
export type ProviderAccountUsageMeterKind = "rolling_limit" | "credit_balance" | "spend_limit" | "token_usage" | "local_cost" | "other"
export type ProviderAccountUsageMeterScope = "account" | "workspace" | "model" | "upstream_provider" | "plan"
export type ProviderAccountUsageMeterState = "healthy" | "warning" | "exhausted" | "unknown"

export type ProviderAccountUsageMeter = {
  meter_id: string
  label: string
  service_id?: string | null
  kind: ProviderAccountUsageMeterKind
  scope: ProviderAccountUsageMeterScope
  used_percent?: number | null
  used?: number | null
  remaining?: number | null
  total?: number | null
  unit?: string | null
  window_duration_minutes?: number | null
  resets_at_ms?: number | null
  state: ProviderAccountUsageMeterState
  source: string
  observed_at_ms: number
}

export type ProviderAccountUsageSnapshot = {
  profile_id: string
  provider: string
  availability: ProviderAccountUsageAvailability
  meters?: ProviderAccountUsageMeter[]
  observed_at_ms?: number | null
  source: string
  management_url?: string | null
}

export const PROVIDER_CREDENTIAL_KIND_CONTRACT_VERSION = 1

/**
 * Provider-observed account/billing class for a credential, reported only by
 * the provider-native adapter; `credential_kind` is null on records written
 * before the contract existed or before the adapter reported the class (treat
 * as not reported, see `credential_kind_not_reported_reason`).
 */
export type ProviderCredentialKind = "subscription" | "api_key" | "prepaid" | "mixed"
export type ProviderAccountServiceCredentialType = "api_key" | "oauth" | "unknown"

export type ProviderAccountService = {
  service_id: string
  label: string
  auth_state: "unknown" | "not_configured" | "authenticated" | "expired" | "error"
  credential_type: ProviderAccountServiceCredentialType
  billing_kind?: ProviderCredentialKind | null
}

export type ProviderAccountProfile = {
  owner_user_id: string
  provider: string
  profile_id: string
  label: string
  origin: "default" | "chariox_created" | "linked"
  is_default: boolean
  auth_state: "unknown" | "not_configured" | "authenticated" | "expired" | "error"
  credential_kind?: ProviderCredentialKind | null
  credential_kind_not_reported_reason?: string | null
  identity_summary?: string | null
  plan?: string | null
  detected_provider_version?: string | null
  last_validated_at_ms?: number | null
  services?: ProviderAccountService[]
  usage: ProviderAccountUsageSnapshot
  materializations?: ProviderAccountMaterializationStatus[]
}

export type ProviderAccountMaterializationStatus = {
  target_kind: "worker" | "slice"
  target_ref: string
  state: "materialized" | "stale" | "error"
  observed_at_ms: number
  last_error?: string | null
}

export type PromptAttachmentPart = {
  url: string
  mime: string
  filename: string | null
  contents_base64?: string | null
}

export type StoredTransferArtifact = {
  artifact_id: string
  stored_path: string
  display_name: string
}

export type CaptureScreenshotResult = {
  status: string
  artifact_path: string | null
  message: string
}

export type RuntimeNoticeRecord = {
  message: string
}

export type TerminalOutputRecord = {
  record_id?: number | null
  timestamp_ms: number
  agent_id?: string | null
  prompt_id?: string | null
  prompt_origin?: "chariox" | "external" | string | null
  source_attachment_id?: string | null
  kind: "provider_output" | "provider_terminal" | "prompt_echo" | "provider_reasoning" | "provider_tool" | "provider_error" | "provider_status"
  merge_key?: string
  source?: "external_provider_observed" | string | null
  external_provider?: string | null
  external_provider_session_id?: string | null
  external_provider_turn_id?: string | null
  observed_at_ms?: number | null
  external_observation?: SessionHistoryExternalObservation | null
  bytes: number[]
}

export type PromptSubmittedPayload = {
  outcome: Record<string, unknown>
  session: RuntimeSession
  agent_activity: Record<string, AgentRuntimeActivity>
  agent_activity_revision: number
}

export type QueuedPromptSteeredPayload = {
  prompt: PromptQueueItem
  session: RuntimeSession
  agent_activity: Record<string, AgentRuntimeActivity>
  agent_activity_revision: number
}

export type QueuedPromptCancelledPayload = {
  prompt: PromptQueueItem
  session: RuntimeSession
  agent_activity: Record<string, AgentRuntimeActivity>
  agent_activity_revision: number
}

export type QueuedPromptUpdatedPayload = {
  prompt: PromptQueueItem
  session: RuntimeSession
  agent_activity: Record<string, AgentRuntimeActivity>
  agent_activity_revision: number
}
