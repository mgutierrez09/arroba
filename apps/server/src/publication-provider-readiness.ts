import { access, readFile } from "node:fs/promises"
import { join } from "node:path"
import { execFile } from "node:child_process"
import { promisify } from "node:util"
import process from "node:process"

import { LocalIpcClient } from "@chariox/kernel-client/ipc"
import { getProviderAuthStatusRequest } from "@chariox/kernel-client/ipc-requests"

import { defaultKernelEndpoint } from "./kernel-publication-client.js"
import type {
  GatewayDeps,
  PublicationPackageMaterializationStatus,
  PublicationProviderReadiness,
  WorkflowPublicationConfig,
  WorkflowPublicationSnapshot,
  KernelLookupClient,
} from "./publication-types.js"

const execFileAsync = promisify(execFile)

export async function publicationHealthDetails(
  publication: WorkflowPublicationConfig,
  deps: GatewayDeps,
): Promise<{
  package: PublicationPackageMaterializationStatus
  provider_readiness: readonly PublicationProviderReadiness[]
}> {
  return {
    package: await packageMaterializationStatus(publication),
    provider_readiness: await publicationProviderReadiness(publication, deps),
  }
}

export async function packageMaterializationStatus(
  publication: WorkflowPublicationConfig,
): Promise<PublicationPackageMaterializationStatus> {
  const packageRoot = publication.package_root ?? null
  if (!packageRoot) {
    return { materialized: true, package_root: null, missing_files: [] }
  }
  const required = ["publication.json", "workflow.snapshot.json", "requirements.json"]
  const missing: string[] = []
  for (const file of required) {
    if (!await fileExists(join(packageRoot, file))) {
      missing.push(file)
    }
  }
  return { materialized: missing.length === 0, package_root: packageRoot, missing_files: missing }
}

export async function publicationProviderReadiness(
  publication: WorkflowPublicationConfig,
  deps: GatewayDeps,
): Promise<readonly PublicationProviderReadiness[]> {
  if (deps.getProviderReadiness) {
    return deps.getProviderReadiness(publication)
  }
  const accounts = await requiredPublicationProviderAccounts(publication)
  if (accounts.length === 0) {
    return []
  }
  const endpoint = publication.kernel_endpoint ?? defaultKernelEndpoint()
  const client = deps.createProviderReadinessClient?.(endpoint) ?? new LocalIpcClient(endpoint)
  try {
    const readiness: PublicationProviderReadiness[] = []
    for (const account of accounts) {
      readiness.push(await providerReadiness(account, client, deps.getProviderCliStatus))
    }
    return readiness
  } finally {
    await client.close?.().catch(() => {})
  }
}

type PublicationProviderAccount = {
  provider: string
  accountProfile: string
}

async function requiredPublicationProviderAccounts(
  publication: WorkflowPublicationConfig,
): Promise<PublicationProviderAccount[]> {
  if (!publication.package_root) {
    return []
  }
  const snapshot = JSON.parse(await readFile(join(publication.package_root, "workflow.snapshot.json"), "utf8")) as WorkflowPublicationSnapshot
  const accounts = new Map<string, PublicationProviderAccount>()
  for (const agent of snapshot.agents ?? []) {
    const provider = normalizeProvider(agent.provider)
    if (!provider) continue
    const accountProfile = normalizedAccountProfile(agent.account_profile)
    const account = { provider, accountProfile }
    accounts.set(JSON.stringify([provider, accountProfile]), account)
  }
  return [...accounts.values()].sort((left, right) => (
    left.provider.localeCompare(right.provider)
    || left.accountProfile.localeCompare(right.accountProfile)
  ))
}

async function providerReadiness(
  account: PublicationProviderAccount,
  client: KernelLookupClient,
  getCliStatus: ((command: string) => Promise<PublicationProviderReadiness["cli"]>) = providerCliStatus,
): Promise<PublicationProviderReadiness> {
  const { provider, accountProfile } = account
  if (provider === "dev-stub" && developmentProviderStubEnabled()) {
    return {
      provider,
      status: "provider_ready",
      ready: true,
      cli: { available: true, command: "internal:dev-stub", version: null },
      auth: { status: "provider_ready", account_profile: "development-stub" },
    }
  }
  const command = providerCommand(provider)
  const cli = await getCliStatus(command)
  if (!cli.available) {
    return {
      provider,
      status: "provider_cli_missing",
      ready: false,
      cli,
      auth: { status: "provider_auth_unknown" },
      error: `${provider} CLI was not found`,
    }
  }
  const auth = await providerAuthStatus(provider, accountProfile, client)
  const status = auth.status === "provider_auth_expired"
    ? "provider_auth_expired"
    : "provider_ready"
  return {
    provider,
    status,
    ready: status === "provider_ready",
    cli,
    auth,
    ...(auth.status === "provider_auth_expired" ? { error: `${provider} authentication is expired or missing` } : {}),
  }
}

function developmentProviderStubEnabled(): boolean {
  return ["1", "true", "yes", "on"].includes(
    process.env.CHARIOX_PROVIDER_DEV_STUB?.trim().toLowerCase() ?? "",
  )
}

async function providerCliStatus(command: string): Promise<PublicationProviderReadiness["cli"]> {
  try {
    const result = await execFileAsync(command, ["--version"], {
      env: providerVersionEnvironment(),
      timeout: 5_000,
      maxBuffer: 64 * 1024,
    })
    const version = `${result.stdout}${result.stderr}`.trim().split(/\r?\n/)[0] ?? null
    return { available: true, command, version: version || null }
  } catch {
    return { available: false, command, version: null }
  }
}

function providerVersionEnvironment(): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = {}
  for (const name of [
    "PATH",
    "LANG",
    "LANGUAGE",
    "LC_ALL",
    "LC_CTYPE",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "NODE_EXTRA_CA_CERTS",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "no_proxy",
    "HOME",
    "TMPDIR",
    "XDG_CACHE_HOME",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
  ] as const) {
    const value = process.env[name]
    if (value !== undefined) env[name] = value
  }
  env.PATH ??= "/usr/local/bin:/usr/bin:/bin"
  return env
}

async function providerAuthStatus(
  provider: string,
  accountProfile: string,
  client: KernelLookupClient,
): Promise<PublicationProviderReadiness["auth"]> {
  try {
    const response = await client.send(getProviderAuthStatusRequest(provider, accountProfile))
    const status = (response.ProviderAuthStatus as { status?: Record<string, unknown> } | undefined)?.status
    const authState = typeof status?.auth_state === "string" ? status.auth_state : "unknown"
    if (authState === "authenticated") {
      return {
        status: "provider_ready",
        account_profile: typeof status?.account_profile === "string" ? status.account_profile : null,
      }
    }
    if (authState === "not_logged_in" || authState === "expired") {
      return { status: "provider_auth_expired" }
    }
    return { status: "provider_auth_unknown" }
  } catch {
    return { status: "provider_auth_unknown" }
  }
}

function normalizedAccountProfile(accountProfile: unknown): string {
  return typeof accountProfile === "string" && accountProfile.trim()
    ? accountProfile.trim()
    : "default"
}

function providerCommand(provider: string): string {
  if (provider === "codex") return process.env.CHARIOX_CODEX_BIN || "codex"
  if (provider === "claude") return process.env.CHARIOX_CLAUDE_BIN || "claude"
  if (provider === "opencode") return process.env.CHARIOX_OPENCODE_BIN || "opencode"
  return provider
}

function normalizeProvider(provider: unknown): string | null {
  if (typeof provider !== "string") return null
  const trimmed = provider.trim().toLowerCase()
  if (!trimmed) return null
  const base = trimmed.split(":")[0] ?? trimmed
  if (base === "default") return "opencode"
  if (base === "claude-headless" || base === "claude-p") return "claude"
  return base
}

async function fileExists(path: string): Promise<boolean> {
  try {
    await access(path)
    return true
  } catch {
    return false
  }
}
