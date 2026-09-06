import { readFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"

import { agentAppReplicaStatus } from "./publication-agent-app-replicas.js"
import { publicationCloudOperationalFields } from "./publication-cloud-operational-status.js"
import type { WorkflowPublicationConfig } from "./publication-types.js"

export interface PublicationCloudProfile {
  readonly apiUrl: string
  readonly accountId: string
  readonly cloudSessionToken?: string
}

export interface RegisterCloudPublicationBackendInput {
  readonly deploymentId: string
  readonly publication: WorkflowPublicationConfig
  readonly localUrl?: string
  readonly status?: "ready" | "unavailable" | "failed"
  readonly lastError?: string | null
  readonly operationalStatus?: unknown
  readonly profile?: PublicationCloudProfile | null
  readonly fetch?: typeof fetch
  readonly now?: () => number
}

export interface PublicationDeploymentLogEntry {
  readonly level?: "debug" | "info" | "warn" | "error" | string
  readonly message: string
  readonly metadata?: unknown
  readonly occurredAt?: Date | string
}

export interface AppendCloudPublicationDeploymentLogsInput {
  readonly deploymentId?: string
  readonly entries: readonly PublicationDeploymentLogEntry[]
  readonly profile?: PublicationCloudProfile | null
  readonly runnerKey?: string | null
  readonly fetch?: typeof fetch
}

export async function registerCloudPublicationDeploymentBackend(
  input: RegisterCloudPublicationBackendInput,
): Promise<boolean> {
  const profile = input.profile === undefined ? await loadCloudPublicationProfile() : input.profile
  if (!profile) return false
  const fetchImpl = input.fetch ?? fetch
  const status = input.status ?? "ready"
  if (status === "ready" && !input.localUrl) {
    throw new Error("Cloud publication backend registration requires localUrl when status is ready")
  }
  const response = await fetchImpl(
    `${normalizeApiUrl(profile.apiUrl)}/publication-deployments/${encodeURIComponent(input.deploymentId)}/local-backend`,
    {
      method: "POST",
      signal: AbortSignal.timeout(5_000),
      headers: {
        accept: "application/json",
        "content-type": "application/json",
        ...(profile.cloudSessionToken ? { authorization: `Bearer ${profile.cloudSessionToken}` } : {}),
      },
      body: JSON.stringify({
        accountId: profile.accountId,
        status,
        runtimeSessionId: input.publication.session_id,
        ...(input.lastError ? { lastError: input.lastError } : {}),
        ...(status === "ready" ? {
          backendTarget: localRuntimeBackendTarget(input.publication, input.localUrl!, input.now?.() ?? Date.now(), input.operationalStatus),
        } : {}),
      }),
    },
  )
  if (!response.ok) {
    throw new Error(`Cloud publication backend registration failed with HTTP ${response.status}: ${await response.text()}`)
  }
  return true
}

function localRuntimeBackendTarget(publication: WorkflowPublicationConfig, localUrl: string, updatedAtMs: number, operationalStatus?: unknown): Record<string, unknown> {
  const base = {
    kind: "local_runtime",
    url: localUrl,
    updated_at_ms: updatedAtMs,
    ...publicationCloudOperationalFields(operationalStatus),
  }
  if (publication.agent_app?.enabled !== true) return base
  const status = agentAppReplicaStatus(publication)
  return {
    ...base,
    queueDepth: status.queueDepth,
    activeReplicaCount: status.activeReplicaCount,
    readyReplicaCount: status.readyReplicaCount,
  }
}

export type PublicationCloudBackendIngress =
  | { readonly kind: "no_cloud_deployment" }
  | { readonly kind: "hosted_container" }
  | { readonly kind: "unavailable"; readonly lastError: string }
  | { readonly kind: "ready" }

export function publicationCloudBackendIngress(input: {
  readonly cloudDeploymentId?: string | null | undefined
  readonly cloudRunnerKey?: string | null | undefined
  readonly access: string
}): PublicationCloudBackendIngress {
  const deploymentId = input.cloudDeploymentId?.trim()
  if (!deploymentId) return { kind: "no_cloud_deployment" }
  if (input.cloudRunnerKey?.trim()) return { kind: "hosted_container" }
  if (input.access !== "tunnel") {
    return {
      kind: "unavailable",
      lastError: `Cloud local-runtime publication requires a relay display tunnel; endpoint registered with access ${input.access}`,
    }
  }
  return { kind: "ready" }
}

export async function appendCloudPublicationDeploymentLogs(
  input: AppendCloudPublicationDeploymentLogsInput,
): Promise<boolean> {
  const deploymentId = input.deploymentId?.trim() || process.env.CHARIOX_PUBLICATION_CLOUD_DEPLOYMENT_ID?.trim()
  if (!deploymentId || input.entries.length === 0) return false
  const runnerKey = input.runnerKey ?? process.env.CHARIOX_PUBLICATION_CLOUD_RUNNER_KEY?.trim() ?? null
  const fetchImpl = input.fetch ?? fetch
  if (runnerKey) {
    const apiUrl = input.profile?.apiUrl || process.env.CHARIOX_PUBLICATION_CLOUD_API_URL?.trim()
    if (!apiUrl) return false
    const response = await fetchImpl(
      `${normalizeApiUrl(apiUrl)}/runner/publication-deployments/${encodeURIComponent(deploymentId)}/logs`,
      {
        method: "POST",
        headers: {
          accept: "application/json",
          "content-type": "application/json",
        },
        body: JSON.stringify({
          runnerKey,
          entries: input.entries,
        }),
      },
    )
    if (!response.ok) {
      throw new Error(`Cloud publication deployment log append failed with HTTP ${response.status}: ${await response.text()}`)
    }
    return true
  }
  const profile = input.profile === undefined ? await loadCloudPublicationProfile() : input.profile
  if (!profile) return false
  const response = await fetchImpl(
    `${normalizeApiUrl(profile.apiUrl)}/publication-deployments/${encodeURIComponent(deploymentId)}/logs`,
    {
      method: "POST",
      headers: {
        accept: "application/json",
        "content-type": "application/json",
        ...(profile.cloudSessionToken ? { authorization: `Bearer ${profile.cloudSessionToken}` } : {}),
      },
      body: JSON.stringify({
        accountId: profile.accountId,
        entries: input.entries,
      }),
    },
  )
  if (!response.ok) {
    throw new Error(`Cloud publication deployment log append failed with HTTP ${response.status}: ${await response.text()}`)
  }
  return true
}

async function loadCloudPublicationProfile(): Promise<PublicationCloudProfile | null> {
  const envProfile = loadCloudPublicationProfileFromEnv()
  if (envProfile) return envProfile
  const config = JSON.parse(await readFile(cloudProfilePath(), "utf8").catch(() => "{}")) as {
    cloud_relay?: {
      api_url?: string
      account_id?: string
      cloud_session_token?: string
    } | null
  }
  const cloud = config.cloud_relay
  if (!cloud?.api_url || !cloud.account_id) return null
  return {
    apiUrl: cloud.api_url,
    accountId: cloud.account_id,
    ...(cloud.cloud_session_token ? { cloudSessionToken: cloud.cloud_session_token } : {}),
  }
}

function loadCloudPublicationProfileFromEnv(): PublicationCloudProfile | null {
  const apiUrl = process.env.CHARIOX_PUBLICATION_CLOUD_API_URL?.trim()
  const accountId = process.env.CHARIOX_PUBLICATION_CLOUD_ACCOUNT_ID?.trim()
  const cloudSessionToken = process.env.CHARIOX_PUBLICATION_CLOUD_SESSION_TOKEN?.trim()
  if (!apiUrl || !accountId) return null
  return {
    apiUrl,
    accountId,
    ...(cloudSessionToken ? { cloudSessionToken } : {}),
  }
}

function cloudProfilePath(): string {
  const charioxHome = process.env.CHARIOX_HOME?.trim()
  if (charioxHome) return path.join(charioxHome, "daemon", "config.json")
  const xdg = process.env.XDG_CONFIG_HOME?.trim()
  if (xdg) return path.join(xdg, "chariox", "daemon", "config.json")
  return path.join(os.homedir(), ".chariox", "daemon", "config.json")
}

function normalizeApiUrl(apiUrl: string): string {
  return apiUrl.trim().replace(/\/+$/, "")
}
