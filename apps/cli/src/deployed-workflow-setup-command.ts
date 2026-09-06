import { randomUUID } from "node:crypto"

import type {
  WorkflowDefinition,
} from "@chariox/kernel-client"

import {
  createDeploymentSetup,
  getDeploymentSetup,
  listDeploymentSetups,
  type DeploymentSetup,
} from "./deployed-workflow-setup-api.js"
import type { DeploymentSetupExecutionOutcome } from "./deployed-workflow-setup-executor.js"
import {
  deploymentSetupUsage,
  draftConfiguration,
  parseSetupOptions,
  publishedConfiguration,
} from "./deployed-workflow-setup-options.js"
import {
  runDeploymentSetupRuntime,
  type AttachedDeploymentSetupRuntime,
} from "./deployed-workflow-setup-runtime.js"
import type { RuntimeSession } from "./cli-types.js"
import type { RelayCloudProfile } from "./preferences.js"

export { deploymentSetupUsage } from "./deployed-workflow-setup-options.js"

export interface DeploymentSetupCommandRuntime {
  readonly isAttached?: () => boolean
  readonly sessionState?: () => RuntimeSession
  readonly sendDeploymentSetupKernelRequest?: (
    request: Record<string, unknown>,
  ) => Promise<Record<string, unknown>>
}

export interface DeploymentSetupCommandOutput {
  readonly notice: string
  readonly footer: string
}

export async function executeDeploymentSetupCommand(
  profile: RelayCloudProfile,
  argv: readonly string[],
  runtime?: DeploymentSetupCommandRuntime,
): Promise<DeploymentSetupCommandOutput> {
  const action = argv[0] ?? "list"
  if (action === "list" || action === "ls") {
    const result = await listDeploymentSetups(profile)
    return {
      notice: result.setups.length > 0
        ? result.setups.map(formatDeploymentSetupListItem).join("\n")
        : "No deployment setups.",
      footer: `${result.setups.length} deployment setup${result.setups.length === 1 ? "" : "s"}`,
    }
  }
  if (action === "show" || action === "get") {
    const setupId = requiredArg(argv[1])
    const result = await getDeploymentSetup(profile, setupId)
    return { notice: formatDeploymentSetup(result.setup), footer: setupFooter(result.setup) }
  }

  const attached = requireAttachedRuntime(runtime)
  if (action === "resume") {
    const setupId = requiredArg(argv[1])
    const options = parseSetupOptions(argv.slice(2), { requireDeployment: false, allowTransport: false })
    return runSetup(profile, setupId, attached, options.agentAppAssets)
  }
  if (action === "draft") {
    const workflowRef = requiredArg(argv[1])
    const endpointRef = requiredArg(argv[2])
    const options = parseSetupOptions(argv.slice(3), { requireDeployment: true, allowTransport: true })
    const session = attached.sessionState()
    const workflow = resolveByIdOrAlias(session.workflows ?? [], workflowRef, "workflow")
    const endpoint = resolveByIdOrAlias(workflow.endpoints ?? [], endpointRef, "workflow endpoint")
    const revision = requiredWorkflowRevision(workflow)
    const created = await createDeploymentSetup(profile, {
      clientRequestId: options.clientRequestId ?? randomUUID(),
      origin: "draft",
      sourceSessionId: session.id,
      sourceWorkflowId: workflow.id,
      sourceWorkflowRevision: String(revision),
      configuration: draftConfiguration(endpoint.id, revision, options),
    })
    return runSetup(profile, created.setup.id, attached, options.agentAppAssets)
  }
  if (action === "publication" || action === "published") {
    const publicationRef = requiredArg(argv[1])
    const options = parseSetupOptions(argv.slice(2), { requireDeployment: true, allowTransport: false })
    const session = attached.sessionState()
    const publication = resolveByIdOrAlias(
      session.workflow_publications ?? [],
      publicationRef,
      "workflow trigger",
    )
    if (!publication.enabled) throw new Error(`workflow trigger ${publication.id} is disabled`)
    const publicationDigest = requiredSha256(
      publication.source_snapshot_digest,
      "immutable publication snapshot digest",
    )
    const created = await createDeploymentSetup(profile, {
      clientRequestId: options.clientRequestId ?? randomUUID(),
      origin: "publication",
      sourceSessionId: session.id,
      sourceWorkflowId: publication.workflow_id,
      sourceWorkflowRevision: publication.source_workflow_revision == null
        ? null
        : String(publication.source_workflow_revision),
      sourcePublicationId: publication.id,
      sourcePublicationDigest: publicationDigest,
      configuration: publishedConfiguration(publication, options),
    })
    return runSetup(profile, created.setup.id, attached, options.agentAppAssets)
  }
  throw new Error(deploymentSetupUsage)
}

async function runSetup(
  profile: RelayCloudProfile,
  setupId: string,
  runtime: AttachedDeploymentSetupRuntime,
  agentAppAssets?: string,
): Promise<DeploymentSetupCommandOutput> {
  return formatSetupOutcome(await runDeploymentSetupRuntime(profile, setupId, runtime, agentAppAssets))
}

function requireAttachedRuntime(
  runtime: DeploymentSetupCommandRuntime | undefined,
): AttachedDeploymentSetupRuntime {
  if (!runtime?.isAttached?.() || !runtime.sessionState || !runtime.sendDeploymentSetupKernelRequest) {
    throw new Error("deployment setup execution requires an attached TUI session")
  }
  return {
    sessionState: runtime.sessionState,
    sendDeploymentSetupKernelRequest: runtime.sendDeploymentSetupKernelRequest,
  }
}

function formatSetupOutcome(outcome: DeploymentSetupExecutionOutcome): DeploymentSetupCommandOutput {
  const setup = outcome.setup
  switch (outcome.kind) {
    case "completed":
      return { notice: formatDeploymentSetup(setup), footer: `deployment ${setup.configuration.deployment.slug} ready` }
    case "awaiting_credentials":
      return {
        notice: `${formatDeploymentSetup(setup)}\nnext=configure release credentials, then run deployments setup resume ${setup.id}`,
        footer: "deployment setup awaits credentials",
      }
    case "waiting_for_runtime":
      return {
        notice: `${formatDeploymentSetup(setup)}\nnext=reconnect the relay, then run deployments setup resume ${setup.id}`,
        footer: "deployment setup waits for relay runtime",
      }
    case "activation_requested":
      return {
        notice: `${formatDeploymentSetup(setup)}\nnext=wait for hosted activation, then run deployments setup resume ${setup.id}`,
        footer: "deployment setup activation requested",
      }
    case "blocked":
      return { notice: formatDeploymentSetup(setup), footer: "deployment setup blocked" }
    case "abandoned":
      return { notice: formatDeploymentSetup(setup), footer: "deployment setup abandoned" }
  }
}

export function formatDeploymentSetup(setup: DeploymentSetup): string {
  return [
    `setup ${setup.id}`,
    `request_id=${setup.clientRequestId}`,
    `origin ${setup.origin}`,
    `status ${setup.status}`,
    `stage ${setup.stage}`,
    `workflow ${setup.sourceWorkflowId}`,
    `publication ${setup.sourcePublicationId ?? "pending"}`,
    `package ${setup.packageId ?? "pending"}`,
    `project ${setup.projectId ?? "pending"}`,
    `release ${setup.releaseId ?? "pending"}`,
    `environment ${setup.environmentId ?? "pending"}`,
    `deployment ${setup.operationalDeploymentId ?? "pending"}`,
    `mode ${setup.configuration.deployment.runtimeMode}`,
    `slug ${setup.configuration.deployment.slug}`,
    `access ${formatSetupAccess(setup.configuration.access)}`,
    ...(setup.failureCode ? [`failure ${setup.failureCode}: ${setup.failureMessage ?? "unknown"}`] : []),
    `updated ${setup.updatedAt}`,
  ].join("\n")
}

function formatSetupAccess(access: DeploymentSetup["configuration"]["access"]): string {
  if (!access || access.kind === "current_account") return "current-account"
  if (access.kind === "public") return "public"
  return `${access.kind === "email_domain" ? "verified-domain" : "email"}:${access.subject}`
}

function formatDeploymentSetupListItem(setup: DeploymentSetup): string {
  return [
    setup.id,
    setup.origin,
    setup.status,
    setup.stage,
    setup.configuration.deployment.slug,
    setup.configuration.deployment.runtimeMode,
    setup.updatedAt,
  ].join("\t")
}

function setupFooter(setup: DeploymentSetup): string {
  return `deployment setup ${setup.status}/${setup.stage}`
}

function resolveByIdOrAlias<T extends { readonly id: string; readonly alias?: string | null }>(
  values: readonly T[],
  reference: string,
  label: string,
): T {
  const byId = values.find((value) => value.id === reference)
  if (byId) return byId
  const byAlias = values.filter((value) => value.alias === reference)
  if (byAlias.length === 1) return byAlias[0]!
  if (byAlias.length > 1) throw new Error(`${label} alias ${reference} is ambiguous; use its ID`)
  throw new Error(`${label} ${reference} was not found in the attached session`)
}

function requiredWorkflowRevision(workflow: WorkflowDefinition): number {
  if (!Number.isSafeInteger(workflow.revision) || Number(workflow.revision) < 0) {
    throw new Error("workflow revision is unavailable; refresh the attached session before deploying")
  }
  return Number(workflow.revision)
}

function requiredSha256(value: unknown, label: string): string {
  if (typeof value !== "string" || !/^sha256:[a-f0-9]{64}$/.test(value)) {
    throw new Error(`${label} is invalid`)
  }
  return value
}

function requiredArg(value: string | undefined): string {
  if (!value?.trim()) throw new Error(deploymentSetupUsage)
  return value.trim()
}
