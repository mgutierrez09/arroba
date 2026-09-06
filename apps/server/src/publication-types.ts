import type {
  WorkflowPublicationDeploymentExtensionRequirement,
  WorkflowPublicationDeploymentNetworkDestination,
} from "@chariox/kernel-client/workflow-publication-deployment-contract"

export type ParserKind =
  | "json"
  | "query_params"
  | "headers"
  | "regex"
  | "path_template"
  | "webhook"
  | "custom_command"

export type ParserConfig = {
  kind: ParserKind
  source?: "body" | "path" | "query" | "headers" | "request"
  pattern?: string
  template?: string
  command?: string
  args?: string[]
}

export type InputSchema = {
  type?: "object"
  required?: string[]
  properties?: Record<string, { type?: "string" | "number" | "boolean" | "object" | "array" }>
}

export type TlsConfig = {
  enabled?: boolean
  key_file?: string
  cert_file?: string
}

export type WorkflowPublicationConfig = {
  publication_id: string
  session_id: string
  source_session_id?: string
  workflow_ref: string
  endpoint_ref: string
  hook_id?: string
  queue_ref?: string
  kernel_endpoint?: string
  transport?: string
  route?: string
  methods?: Array<"GET" | "POST">
  parser?: ParserConfig
  input_schema?: InputSchema
  trace_exposure?: PublicationTraceExposurePolicy
  trace_context?: PublicationTraceContext
  tls?: TlsConfig
  mode?: "sync" | "async"
  sync_timeout_ms?: number
  poll_ms?: number
  package_root?: string
  agent_app?: AgentAppConfig
  replica_session_ids?: string[]
}

export type PublicationHookConfig = {
  id: string
  publication_id?: string
  transport: string
  endpoint_id: string
  queue_ref?: string
  route?: string
  methods?: string[]
  parser?: ParserConfig
  input_schema?: InputSchema | null
  trace_exposure?: PublicationTraceExposurePolicy | null
  mode?: "sync" | "async"
  sync_timeout_ms?: number | null
  poll_ms?: number | null
  response_mode?: string
}

export type WorkflowPublicationPackage = {
  schema_version: number
  package_version?: number
  publication_id: string
  alias?: string | null
  source_session_id?: string
  workflow_id: string
  default_bindings_path?: string
  event_bindings_path?: string
  deployment_contract?: {
    path?: string
    schema_version?: number
  }
  hooks: PublicationHookConfig[]
  agent_app?: AgentAppConfig
}

export type AgentAppConfig = {
  enabled?: boolean
  assets?: AgentAppAssetsConfig
  routes?: AgentAppRouteConfig[]
  actions?: Record<string, AgentAppActionConfig>
  replicas?: AgentAppReplicaConfig
  network?: AgentAppNetworkConfig
  persistent_patch?: { enabled?: boolean }
}

export type AgentAppNetworkConfig = {
  destinations?: AgentAppNetworkDestinationConfig[]
}

export type AgentAppNetworkDestinationConfig = {
  id: string
  host: string
  credential_slot_ids?: string[]
}

export type AgentAppAssetsConfig = {
  public_dir?: string
  index?: string
}

export type AgentAppRouteConfig = {
  path: string
  hook_id?: string
  prompt_source?: "path_tail"
  response?: "streaming_shell"
  required_role?: string
  manipulation?: {
    level?: "none" | "state" | "overlay" | "state_and_overlay" | "full_ephemeral" | "persistent_patch"
    scope?: "invocation" | "session" | "persistent"
    allowed_paths?: string[]
    protected_paths?: string[]
    allowed_actions?: string[]
  }
}

export type AgentAppActionConfig = {
  input_schema?: InputSchema
  transport?: {
    kind?: "http"
    method?: "GET" | "POST"
    url?: string
  }
}

export type AgentAppReplicaConfig = {
  count?: number
  per_caller_ordering?: boolean
  max_queue_depth?: number
  timeout_ms?: number
}

export type PublicationTraceLevel =
  | "user_prompt"
  | "output_summary"
  | "assistant_messages"
  | "thinking"
  | "tool_use"

export type PublicationTraceExposurePolicy = {
  nodes?: Record<string, PublicationTraceLevel[]>
}

export type PublicationTraceNodeContext = {
  node_id: string
  node_label?: string | null
  agent_id?: string | null
  agent_alias?: string | null
}

export type PublicationTraceContext = {
  nodes: Record<string, PublicationTraceNodeContext>
}

export type PublicationTraceEvent = {
  workflow_run_id: string
  workflow_node_run_id: string
  node_id: string
  node_label: string | null
  agent_id: string
  agent_alias: string | null
  level: PublicationTraceLevel
  sequence: number
  timestamp_ms: number
  message: string
  data?: unknown
}

export type WorkflowPublicationSnapshot = KernelWorkflowPublicationSnapshot

export type PublicationProviderModelProfile = {
  provider: string
  model?: string | null
  effort?: string | null
  account_profile?: string | null
}

export type PublicationProviderModelOverride = {
  agent_id: string
  node_ids?: string[]
  captured: PublicationProviderModelProfile
  replacement?: PublicationProviderModelProfile | null
}

export type WorkflowPublicationBindings = {
  schema_version: number
  provider_model_overrides?: PublicationProviderModelOverride[]
}

export type PublicationNamedRequirement = {
  name: string
}

export type PublicationCredentialRequirement = {
  name: string
  used_by?: string
}

export type WorkflowPublicationRequirementsV1 = {
  schema_version: 1
  mcps?: PublicationNamedRequirement[]
  skills?: PublicationNamedRequirement[]
  scripts?: PublicationNamedRequirement[]
  connectors?: PublicationNamedRequirement[]
  credentials?: PublicationCredentialRequirement[]
}

export type WorkflowPublicationRequirementsV2 = {
  schema_version: 2
  extensions: readonly WorkflowPublicationDeploymentExtensionRequirement[]
  credential_slots: readonly WorkflowPublicationDeploymentExtensionRequirement["credential_slots"][number][]
  network_destinations: readonly WorkflowPublicationDeploymentNetworkDestination[]
}

export type WorkflowPublicationRequirements =
  | WorkflowPublicationRequirementsV1
  | WorkflowPublicationRequirementsV2

export type PublicationPackageMaterializationStatus = {
  materialized: boolean
  package_root: string | null
  missing_files: string[]
}

export type PublicationProviderReadinessStatus =
  | "provider_ready"
  | "provider_cli_missing"
  | "provider_auth_expired"
  | "provider_auth_unknown"

export type PublicationProviderReadiness = {
  provider: string
  status: PublicationProviderReadinessStatus
  ready: boolean
  cli: {
    available: boolean
    command: string
    version?: string | null
  }
  auth: {
    status: "provider_ready" | "provider_auth_expired" | "provider_auth_unknown"
    account_profile?: string | null
  }
  error?: string | null
}

export type NormalizedInvocation = {
  publication_id: string
  request_id: string
  caller: Record<string, unknown>
  input: unknown
  mode: "sync" | "async"
}

export type WorkflowPublicationInvocationEnvelope = {
  publication_id: string
  hook_id?: string | null
  invocation_id: string
  transport: string
  endpoint_id: string
  queue_ref?: string | null
  input: unknown
  artifacts: unknown[]
  mode: "sync" | "async"
  caller: Record<string, unknown>
}

export type WorkflowRun = {
  id: string
  status: string
  workflow_id?: string
  endpoint_id?: string
  invocation_prompt?: string | null
  publication_invocation?: WorkflowPublicationInvocationEnvelope | null
  completed_by_node_run_id?: string | null
  node_runs?: Array<{
    id: string
    node_id: string
    agent_id: string
    status: string
    summary?: string | null
    completion?: {
      summary?: string | null
      output?: unknown
    } | null
    turn_envelope?: {
      runtime_tool_calls?: {
        tool_name: string
        arguments_json: string
        result_json?: string | null
        ok: boolean
        timestamp_ms: number
      }[]
    } | null
    thinking_traces?: {
      id: string
      message: string
      timestamp_ms: number
    }[]
    completed_at_ms?: number | null
  }>
  messages?: Array<{
    id: string
    source_node_run_id?: string | null
    target_node_id: string
    message_type: string
    summary: string
    handoff_payload: string
    created_at_ms: number
  }>
  intermediate_outputs?: Array<{
    id: string
    source_node_run_id?: string
    output: { message: string }
    valid: boolean
    warning?: string | null
    timestamp_ms?: number
  }>
  final_output?: unknown
  created_at_ms?: number
  completed_at_ms?: number | null
}

export type WorkflowInvocationResult = {
  accepted: boolean
  queued?: boolean
  workflow_run?: WorkflowRun
  response?: unknown
}

export type GatewayDeps = {
  invokeWorkflow?: (invocation: NormalizedInvocation) => Promise<WorkflowInvocationResult>
  getPublicationStatusDetails?: (publication: WorkflowPublicationConfig) => Promise<Record<string, unknown>>
  getProviderReadiness?: (publication: WorkflowPublicationConfig) => Promise<readonly PublicationProviderReadiness[]>
  getProviderCliStatus?: (command: string) => Promise<PublicationProviderReadiness["cli"]>
  createProviderReadinessClient?: (endpoint: string) => KernelLookupClient
}

export type PublicationInvocationOptions = {
  input: unknown
  caller?: Record<string, unknown>
  mode?: "sync" | "async"
  requestIdPrefix?: string
  deps?: Pick<GatewayDeps, "invokeWorkflow">
}

export type KernelLookupClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  close?: () => Promise<void>
}

export type GatewayRequest = {
  method: string
  url: string
  headers: Record<string, string | string[] | undefined>
  body?: unknown
  query?: unknown
  raw: unknown
}
import type { WorkflowPublicationSnapshot as KernelWorkflowPublicationSnapshot } from "@chariox/kernel-client/kernel-types"
