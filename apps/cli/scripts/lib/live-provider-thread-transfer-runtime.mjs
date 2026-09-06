import { spawn } from "node:child_process"
import { createHmac } from "node:crypto"
import { createWriteStream, existsSync } from "node:fs"
import { chmod, copyFile, mkdir, readdir, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { setTimeout as sleep } from "node:timers/promises"

import { LocalIpcClient } from "../../dist/ipc.js"
import {
  createSessionRequest,
  endSessionRequest,
  getProviderRunRequest,
  getSliceRequest,
  getSessionHistoryBlobContentRequest,
  getSessionHistoryOutlineRequest,
  getSessionStateRequest,
  listProviderProcessesRequest,
  listSessionsRequest,
} from "../../dist/ipc-requests.js"
import {
  makeAvailablePorts,
  portIsAvailable,
  resolveBuiltBinarySync,
  terminateChild,
} from "./drill-runtime-helpers.mjs"
import { sanitizeDrillMetadata } from "./drill-secrets.mjs"

export const scriptDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
export const cliRoot = path.resolve(scriptDir, "..")
export const repoRoot = path.resolve(cliRoot, "..", "..")
export const kernelBinary = resolveBinaryPath("kernel", "chariox-kernel")
export const relayBinary = resolveBinaryPath("relay", "chariox-relay")
export const defaultLocalDockerSliceImage = process.env.CHARIOX_SLICE_DOCKER_IMAGE ?? "chariox-slice-linux:0.1.0"

export const DEFAULT_PROVIDERS = ["opencode", "codex"]
export const DEFAULT_MODEL = "gpt-5.2"
export const DEFAULT_CODEX_MODEL = process.env.CHARIOX_PROVIDER_THREAD_CODEX_MODEL ?? "gpt-5.5"
export const DEFAULT_TIMEOUT_MS = 420_000
export const DEFAULT_POLL_MS = 1_000
export const DEFAULT_SLICE_BUILD_IMAGE_POLICY = process.env.CHARIOX_PROVIDER_THREAD_SLICE_BUILD_IMAGE ?? "always"
export const RELAY_ISSUER = "chariox-provider-thread-transfer-drill"
export const RELAY_SECRET = "chariox-provider-thread-transfer-drill-secret"
export const RELAY_REALM = "provider-thread-transfer-drill"

function resolveBinaryPath(crateName, binName) {
  const appLocalBinary = path.join(repoRoot, "apps", crateName, "target", "debug", binName)
  return resolveBuiltBinarySync(
    appLocalBinary,
    path.join(repoRoot, "apps", crateName, "Cargo.toml"),
    binName,
  )
}

export function parseArgs(argv) {
  const options = {
    providers: DEFAULT_PROVIDERS,
    model: DEFAULT_MODEL,
    providerModels: {},
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
    spawnDaemon: true,
    kernel: null,
    drill: "local-reload",
    keepArtifactsOnFailure: true,
    skipRecallPrompt: false,
    workerState: "shared",
    sliceBuildImage: DEFAULT_SLICE_BUILD_IMAGE_POLICY,
    keepSliceOnFailure: false,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--") continue
    else if (arg === "--provider") options.providers = [argv[++index]]
    else if (arg === "--providers") options.providers = argv[++index].split(",").map((value) => value.trim()).filter(Boolean)
    else if (arg === "--model") options.model = argv[++index]
    else if (arg === "--provider-model") {
      const [provider, model] = argv[++index].split("=", 2)
      if (!provider || !model) throw new Error("--provider-model must use provider=model")
      options.providerModels[provider] = model
    } else if (arg === "--timeout-ms") options.timeoutMs = Number(argv[++index])
    else if (arg === "--poll-ms") options.pollMs = Number(argv[++index])
    else if (arg === "--kernel") {
      options.kernel = argv[++index]
      options.spawnDaemon = false
    } else if (arg === "--no-spawn-daemon") {
      options.spawnDaemon = false
    } else if (arg === "--drill") {
      options.drill = argv[++index]
    } else if (arg === "--skip-recall-prompt") {
      options.skipRecallPrompt = true
    } else if (arg === "--worker-state") {
      options.workerState = argv[++index]
    } else if (arg === "--slice-build-image") {
      options.sliceBuildImage = argv[++index]
    } else if (arg === "--keep-slice-on-failure") {
      options.keepSliceOnFailure = true
    } else if (arg === "--cleanup-on-success") {
      options.keepArtifactsOnFailure = true
      options.cleanupOnSuccess = true
    } else if (arg === "--help" || arg === "-h") {
      options.help = true
    } else {
      throw new Error(`unknown argument: ${arg}`)
    }
  }
  if (options.providers.length === 0) throw new Error("at least one provider is required")
  if (!["local-reload", "worker-resume", "slice-restart", "slice-shutdown", "slice-save-failure", "live-migrate-to-slice", "live-migrate-roundtrip-slice"].includes(options.drill)) {
    throw new Error(`unsupported --drill ${options.drill}; implemented drills: local-reload, worker-resume, slice-restart, slice-shutdown, slice-save-failure, live-migrate-to-slice, live-migrate-roundtrip-slice`)
  }
  if (!["shared", "isolated"].includes(options.workerState)) {
    throw new Error(`unsupported --worker-state ${options.workerState}; expected shared or isolated`)
  }
  if (!["always", "auto", "never"].includes(options.sliceBuildImage)) {
    throw new Error(`unsupported --slice-build-image ${options.sliceBuildImage}; expected always, auto, or never`)
  }
  return options
}

export function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/live-provider-thread-transfer-drill.mjs [options]",
    "",
    "Runs executable drills from docs/CHARIOX_SERVER_PROVIDER_THREAD_TRANSFER_DRILLS_PLAN.md.",
    "",
    "Implemented drill:",
    "  local-reload  Drill 1: baseline local reload preserves provider thread",
    "  worker-resume  Drill 3 precursor: resume a captured provider thread on a same-host worker",
    "  slice-restart  Drill 4 precursor: save/restart a local Docker slice and relaunch the same agent",
    "  slice-shutdown  Save/shut down a local Docker slice, then explicitly start it and relaunch the same agent",
    "  slice-save-failure  Preserve the running slice, agent thread, and prior saved generation after injected capture failure",
    "  live-migrate-to-slice  Drill 4: start locally, move the same agent to a slice, and resume the same provider thread",
    "  live-migrate-roundtrip-slice  Drill 5: move local -> slice -> local and resume the same provider thread both ways",
    "",
    "Options:",
    `  --providers ${DEFAULT_PROVIDERS.join(",")}`,
    "  --provider PROVIDER",
    "  --provider-model PROVIDER=MODEL",
    "  --model MODEL",
    `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
    `  --poll-ms ${DEFAULT_POLL_MS}`,
    "  --kernel ws://127.0.0.1:PORT",
    "  --no-spawn-daemon",
    "  --skip-recall-prompt",
    "  --worker-state shared|isolated",
    `  --slice-build-image always|auto|never (default ${DEFAULT_SLICE_BUILD_IMAGE_POLICY})`,
    "  --keep-slice-on-failure",
    "  --cleanup-on-success (accepted for compatibility; disposable runtime is always cleaned)",
  ].join("\n"))
}

export function providerThreadSliceOptLevel(env = process.env) {
  const level = env.CHARIOX_PROVIDER_THREAD_SLICE_OPT_LEVEL ?? "1"
  if (!["0", "1", "2", "3", "s", "z"].includes(level)) {
    throw new Error(
      "CHARIOX_PROVIDER_THREAD_SLICE_OPT_LEVEL must be a Cargo optimization level: 0, 1, 2, 3, s, or z",
    )
  }
  return level
}

export function providerThreadSliceBuildProfile(env = process.env) {
  const profile = env.CHARIOX_PROVIDER_THREAD_SLICE_BUILD_PROFILE ?? "dev"
  if (!["dev", "release"].includes(profile)) {
    throw new Error(
      "CHARIOX_PROVIDER_THREAD_SLICE_BUILD_PROFILE must be a supported Cargo build profile: dev or release",
    )
  }
  return profile
}

export function providerThreadSliceBuildEnv(env = process.env) {
  return {
    CHARIOX_SLICE_RUNTIME_BUILD_PROFILE: providerThreadSliceBuildProfile(env),
    CHARIOX_SLICE_CARGO_PROFILE_RELEASE_OPT_LEVEL: providerThreadSliceOptLevel(env),
  }
}

export function providerThreadSliceConfigLines({ sliceRoot, image, buildImage }) {
  return [
    "[slices]",
    `root = ${JSON.stringify(sliceRoot)}`,
    "",
    "[slices.linux]",
    `docker_image = ${JSON.stringify(image)}`,
    `build_image = ${JSON.stringify(buildImage)}`,
    "memory_mb = 2048",
    `cpus = ${JSON.stringify("1.0")}`,
    "allow_unconfined_seccomp = true",
  ]
}

export function variant(response, name) {
  if (!response || !(name in response)) {
    throw new Error(`expected ${name}, got ${JSON.stringify(response)}`)
  }
  return response[name]
}

export function variantAny(response, ...names) {
  for (const name of names) {
    if (response && name in response) return response[name]
  }
  throw new Error(`expected one of ${names.join(", ")}, got ${JSON.stringify(response)}`)
}

export function providerModel(provider, options) {
  if (options.providerModels[provider]) return options.providerModels[provider]
  if (provider === "opencode") return options.model
  if (provider === "codex" && options.model === DEFAULT_MODEL) return DEFAULT_CODEX_MODEL
  if (provider === "codex" && !options.model.endsWith("-codex") && /^gpt-5\.[23]$/.test(options.model)) {
    return `${options.model}-codex`
  }
  if ((provider === "claude-p" || provider === "claude-headless") && !options.model.startsWith("claude-")) {
    return "claude-sonnet-4-6"
  }
  return options.model
}

export function providerEffort(provider) {
  if (provider === "claude-p" || provider === "claude-headless") return "low"
  return "low"
}

function base64url(input) {
  return Buffer.from(input).toString("base64url")
}

export function signRelayToken(claims) {
  const payload = base64url(JSON.stringify(claims))
  const signature = createHmac("sha256", RELAY_SECRET).update(payload).digest("base64url")
  return `chariox-scoped-v1.${payload}.${signature}`
}

export function relayClaims({ subject, subjectKind, actions, userId = "local", targets = null }) {
  return {
    issuer: RELAY_ISSUER,
    subject,
    subject_kind: subjectKind,
    realm_id: RELAY_REALM,
    allowed_actions: actions,
    allowed_targets: targets,
    issued_at_ms: Date.now(),
    expires_at_ms: Date.now() + 10 * 60_000,
    token_id: `${subject}-${Date.now()}`,
    account_id: "provider-thread-transfer-drill-account",
    organization_id: null,
    user_id: userId,
    device_id: subject,
    machine_id: subjectKind === "kernel" || subjectKind === "machine" ? subject : null,
    client_id: subjectKind === "client" ? subject : null,
    public_key_thumbprint: null,
    entitlements_version: "drill",
  }
}

export async function makeWorkerResumePorts() {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const ports = await makeAvailablePorts()
    const expanded = {
      ...ports,
      homeKernelPort: ports.kernelPort,
      homeMcpPort: ports.mcpPort,
      homeOpenCodePort: ports.openCodePort,
      homeCodexPort: ports.codexPort,
      workerOpenCodePort: ports.openCodePort + 101,
      workerCodexPort: ports.codexPort + 101,
    }
    if (
      await portIsAvailable(expanded.workerOpenCodePort)
      && await portIsAvailable(expanded.workerCodexPort)
    ) {
      return expanded
    }
  }
  throw new Error("could not find available worker-resume drill ports")
}

export function providerThreadId(run) {
  return run?.provider_session_id
    ?? run?.resume_state?.opencode_session_id
    ?? run?.resume_state?.codex_thread_id
    ?? run?.resume_state?.claude_session_id
    ?? null
}

export function providerRunSnapshot(run) {
  return {
    id: run?.id ?? null,
    provider: run?.provider ?? null,
    adapter_key: run?.adapter_key ?? null,
    account_profile: run?.account_profile ?? null,
    state: run?.state ?? null,
    provider_session_id: run?.provider_session_id ?? null,
    resume_state: run?.resume_state ?? null,
    mcp_servers: (run?.mcp_servers ?? []).map((server) => server.name ?? server),
    execution_mode: run?.execution_mode ?? null,
    permission_level: run?.permission_level ?? null,
    write_access_mode: run?.write_access_mode ?? null,
    working_directory: run?.working_directory ?? null,
    started_at_ms: run?.started_at_ms ?? null,
    last_activity_at_ms: run?.last_activity_at_ms ?? null,
  }
}

export function providerThreadKernelEventSnapshot(event, observedAtMs = Date.now()) {
  return sanitizeDrillMetadata({
    observed_at_ms: observedAtMs,
    ...event,
  })
}

export function sliceRestartContinuityChecks({
  beforeRun,
  afterRun,
  beforeBinding,
  afterBinding,
  sliceBeforeRestart,
  restartedSlice,
  savedState,
}) {
  const agentBindingRepaired = Boolean(
    beforeBinding
    && afterBinding
    && afterBinding.worker_kernel_id === restartedSlice?.worker_kernel_id
    && afterBinding.worker_machine_id === restartedSlice?.worker_machine_id
    && afterBinding.execution_lease_id !== beforeBinding.execution_lease_id
    && afterBinding.leased_agent_id !== beforeBinding.leased_agent_id,
  )
  const sliceWorkerIdentityPreserved = Boolean(
    sliceBeforeRestart?.worker_kernel_id
    && sliceBeforeRestart?.worker_machine_id
    && sliceBeforeRestart.worker_kernel_id === restartedSlice?.worker_kernel_id
    && sliceBeforeRestart.worker_machine_id === restartedSlice?.worker_machine_id,
  )
  const beforeStartedAtMs = beforeRun?.started_at_ms
  const savedAtMs = savedState?.created_at_ms
  const afterStartedAtMs = afterRun?.started_at_ms
  const sliceRestartTimelineValid = (
    Number.isFinite(beforeStartedAtMs)
    && Number.isFinite(savedAtMs)
    && Number.isFinite(afterStartedAtMs)
    && beforeStartedAtMs <= savedAtMs
    && savedAtMs <= afterStartedAtMs
  )
  const sliceRestartCompleted = Boolean(
    restartedSlice?.status === "running"
    && savedState?.image_ref
    && beforeRun?.id
    && afterRun?.id
    && beforeRun.id !== afterRun.id
    && agentBindingRepaired
    && sliceWorkerIdentityPreserved
    && sliceRestartTimelineValid,
  )

  return {
    agent_binding_repaired: agentBindingRepaired,
    slice_worker_identity_preserved: sliceWorkerIdentityPreserved,
    slice_restart_timeline_valid: sliceRestartTimelineValid,
    slice_restart_completed: sliceRestartCompleted,
  }
}

export function sliceShutdownCheckpointChecks({ savedSlice, parkedRun, stoppedSession }) {
  const sliceShutdownLeftStopped = String(savedSlice?.status ?? "").toLowerCase() === "stopped"
  const sliceShutdownParkedProviderRun = String(parkedRun?.state ?? "").toLowerCase() === "ended"
  const sliceShutdownClearedActiveProviderRun = !stoppedSession?.active_provider_run_id

  return {
    slice_shutdown_left_stopped: sliceShutdownLeftStopped,
    slice_shutdown_parked_provider_run: sliceShutdownParkedProviderRun,
    slice_shutdown_cleared_active_provider_run: sliceShutdownClearedActiveProviderRun,
    slice_shutdown_checkpoint_valid: (
      sliceShutdownLeftStopped
      && sliceShutdownParkedProviderRun
      && sliceShutdownClearedActiveProviderRun
    ),
  }
}

export function sliceRecordSnapshot(slice) {
  return {
    id: slice?.id ?? null,
    name: slice?.name ?? null,
    status: slice?.status ?? null,
    backend: slice?.backend ?? null,
    display_mode: slice?.display_mode ?? null,
    worker_kernel_ref: slice?.worker_kernel_ref ?? null,
    worker_kernel_id: slice?.worker_kernel_id ?? null,
    worker_machine_id: slice?.worker_machine_id ?? null,
    providers: slice?.providers ?? [],
    session_ids: slice?.session_ids ?? [],
    agent_ids: slice?.agent_ids ?? [],
    saved_state_id: slice?.active_saved_state_id ?? slice?.saved_state_id ?? slice?.saved_state_ref ?? null,
    saved_state_ref: slice?.saved_state_ref ?? null,
    saved_state_status: slice?.saved_state_status ?? null,
    saved_state_updated_at_ms: slice?.saved_state_updated_at_ms ?? null,
    operation: slice?.operation ?? null,
  }
}

export function sliceSavedStateSnapshot(state) {
  return {
    id: state?.id ?? null,
    slice_id: state?.slice_id ?? null,
    backend: state?.backend ?? null,
    status: state?.status ?? null,
    image_ref: state?.image_ref ?? null,
    created_at_ms: state?.created_at_ms ?? null,
    updated_at_ms: state?.updated_at_ms ?? null,
  }
}

export function logStep(result, provider, step, details = {}) {
  const entry = {
    at_ms: Date.now(),
    step,
    ...details,
  }
  result.evidence.steps ??= []
  result.evidence.steps.push(entry)
  console.log(`${provider}: ${step}${Object.keys(details).length ? ` ${JSON.stringify(details)}` : ""}`)
}

export async function withTimeout(promise, label, timeoutMs) {
  let timer = null
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = globalThis.setTimeout(() => reject(new Error(`${label} timed out after ${timeoutMs}ms`)), timeoutMs)
      }),
    ])
  } finally {
    if (timer) globalThis.clearTimeout(timer)
  }
}

export async function sendControlRequest(kernelUrl, request, label, timeoutMs) {
  const controlClient = new LocalIpcClient(kernelUrl, {
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  })
  try {
    return await withTimeout(
      controlClient.send(request),
      label,
      timeoutMs,
    )
  } finally {
    await controlClient.close().catch(() => {})
  }
}

export async function waitForLocalDaemon(kernelUrl, workspace, worktree) {
  let lastError = null
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const client = new LocalIpcClient(kernelUrl, {
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    try {
      const session = variant(await client.send(createSessionRequest(workspace, worktree)), "SessionCreated").session
      await client.send(endSessionRequest(session.id)).catch(() => {})
      await client.close()
      return
    } catch (error) {
      lastError = error
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`kernel did not become ready: ${lastError?.message ?? lastError}`)
}

export async function waitForRemoteMachine(localClient, machineRef, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    try {
      const response = await localClient.send({ ListRemoteMachineKernels: { machine_ref: machineRef } })
      const payload = variant(response, "RemoteMachineKernelsListed")
      if ((payload.kernels ?? []).length > 0) return payload.kernels
    } catch (error) {
      lastError = error
    }
    await sleep(pollMs)
  }
  throw new Error(`remote machine ${machineRef} did not become reachable: ${lastError?.message ?? lastError ?? "unknown error"}`)
}

export async function waitForRelayTarget(relayUrl, relayToken, targetDaemonAlias, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    const client = new LocalIpcClient(relayUrl, {
      relayAuthToken: relayToken,
      targetDaemonAlias,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    try {
      await withTimeout(client.send(listSessionsRequest()), `probe relay target ${targetDaemonAlias}`, 2_000)
      await client.close()
      return
    } catch (error) {
      lastError = error
      await client.close().catch(() => {})
      await sleep(pollMs)
    }
  }
  throw new Error(`relay target ${targetDaemonAlias} did not become reachable: ${lastError?.message ?? lastError ?? "unknown error"}`)
}

export function realProviderEnv() {
  const home = process.env.HOME ?? os.homedir()
  const xdgDataHome = process.env.XDG_DATA_HOME ?? path.join(home, ".local", "share")
  return {
    HOME: home,
    CODEX_HOME: process.env.CODEX_HOME ?? path.join(home, ".codex"),
    OPENCODE_CONFIG_DIR: process.env.OPENCODE_CONFIG_DIR ?? path.join(home, ".config", "opencode"),
    OPENCODE_DATA_HOME: process.env.OPENCODE_DATA_HOME ?? path.join(xdgDataHome, "opencode"),
    XDG_CONFIG_HOME: process.env.XDG_CONFIG_HOME ?? path.join(home, ".config"),
    XDG_DATA_HOME: xdgDataHome,
    XDG_STATE_HOME: process.env.XDG_STATE_HOME ?? path.join(home, ".local", "state"),
    XDG_CACHE_HOME: process.env.XDG_CACHE_HOME ?? path.join(home, ".cache"),
  }
}

export async function copySecretIfPresent(source, destination) {
  try {
    await mkdir(path.dirname(destination), { recursive: true })
    await copyFile(source, destination)
    await chmod(destination, 0o600).catch(() => {})
    return true
  } catch (error) {
    if (error?.code === "ENOENT") return false
    throw error
  }
}

export function providersNeedClaudeCredentials(providers) {
  return providers.some((provider) => (
    provider === "claude" || provider === "claude-p" || provider === "claude-headless"
  ))
}

export async function writeClaudeCredentialsPayload(destination, payload) {
  const parsed = JSON.parse(Buffer.from(payload).toString("utf8"))
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("Claude credential export did not contain a JSON object")
  }
  await mkdir(path.dirname(destination), { recursive: true })
  await writeFile(destination, payload, { mode: 0o600 })
  await chmod(destination, 0o600)
  return destination
}

export const CLAUDE_UNATTENDED_CREDENTIALS_GUIDANCE =
  "Claude credential materialization for this legacy direct-worker drill is unavailable until it uses the managed Chariox-vault setup-token path. Chariox will not read macOS Keychain or copy refreshable credentials into worker profiles."

export async function prepareSliceModeProviderEnv(root, providers = DEFAULT_PROVIDERS) {
  if (providersNeedClaudeCredentials(providers)) {
    throw new Error(CLAUDE_UNATTENDED_CREDENTIALS_GUIDANCE)
  }
  const real = realProviderEnv()
  const codexHome = path.join(root, "codex-home")
  const xdgConfigHome = path.join(root, "xdg-config")
  const xdgDataHome = path.join(root, "xdg-data")
  const xdgStateHome = path.join(root, "xdg-state")
  const xdgCacheHome = path.join(root, "xdg-cache")
  const opencodeDataHome = path.join(xdgDataHome, "opencode")

  await mkdir(codexHome, { recursive: true })
  await mkdir(opencodeDataHome, { recursive: true })
  await mkdir(xdgStateHome, { recursive: true })
  await mkdir(xdgCacheHome, { recursive: true })

  const codexAuthCopied = providers.includes("codex")
    ? await copySecretIfPresent(
        path.join(real.CODEX_HOME, "auth.json"),
        path.join(codexHome, "auth.json"),
      )
    : false
  const opencodeAuthCopied = providers.includes("opencode")
    ? await copySecretIfPresent(
        path.join(real.OPENCODE_DATA_HOME, "auth.json"),
        path.join(opencodeDataHome, "auth.json"),
      )
    : false

  return {
    HOME: real.HOME,
    CODEX_HOME: codexHome,
    OPENCODE_CONFIG_DIR: real.OPENCODE_CONFIG_DIR,
    OPENCODE_DATA_HOME: opencodeDataHome,
    XDG_CONFIG_HOME: xdgConfigHome,
    XDG_DATA_HOME: xdgDataHome,
    XDG_STATE_HOME: xdgStateHome,
    XDG_CACHE_HOME: xdgCacheHome,
    ...providerThreadSliceBuildEnv(),
    CHARIOX_PROVIDER_THREAD_CODEX_AUTH_COPIED: codexAuthCopied ? "1" : "0",
    CHARIOX_PROVIDER_THREAD_OPENCODE_AUTH_COPIED: opencodeAuthCopied ? "1" : "0",
  }
}

export async function cleanupSliceModeProviderCredentials(providerEnv) {
  if (!providerEnv) return
  const removals = []
  if (
    providerEnv.CHARIOX_PROVIDER_THREAD_CODEX_AUTH_COPIED === "1"
    && providerEnv.CODEX_HOME
  ) {
    removals.push(rm(path.join(providerEnv.CODEX_HOME, "auth.json"), { force: true }))
  }
  if (
    providerEnv.CHARIOX_PROVIDER_THREAD_OPENCODE_AUTH_COPIED === "1"
    && providerEnv.OPENCODE_DATA_HOME
  ) {
    removals.push(rm(path.join(providerEnv.OPENCODE_DATA_HOME, "auth.json"), { force: true }))
  }
  if (providerEnv.CHARIOX_PROVIDER_THREAD_CLAUDE_SECRET_ROOT) {
    removals.push(
      rm(providerEnv.CHARIOX_PROVIDER_THREAD_CLAUDE_SECRET_ROOT, {
        recursive: true,
        force: true,
      }),
    )
  }
  await Promise.all(removals)
}

export async function prepareIsolatedWorkerProviderEnv(providers = DEFAULT_PROVIDERS, role = "worker") {
  if (providersNeedClaudeCredentials(providers)) {
    throw new Error(CLAUDE_UNATTENDED_CREDENTIALS_GUIDANCE)
  }
  const real = realProviderEnv()
  const secretRoot = path.join(
    os.tmpdir(),
    `chariox-provider-transfer-secrets-${role}-${process.pid}-${Date.now()}`,
  )
  const isolatedHome = path.join(secretRoot, "home")
  const codexHome = path.join(secretRoot, "codex")
  const xdgDataHome = path.join(secretRoot, "xdg-data")
  const xdgStateHome = path.join(secretRoot, "xdg-state")
  const xdgCacheHome = path.join(secretRoot, "xdg-cache")
  const opencodeDataHome = path.join(secretRoot, "opencode-data")

  await mkdir(isolatedHome, { recursive: true })
  await mkdir(codexHome, { recursive: true })
  await mkdir(path.join(xdgDataHome, "opencode"), { recursive: true })
  await mkdir(xdgStateHome, { recursive: true })
  await mkdir(xdgCacheHome, { recursive: true })
  await mkdir(opencodeDataHome, { recursive: true })

  const codexAuthCopied = providers.includes("codex")
    ? await copySecretIfPresent(
        path.join(real.CODEX_HOME, "auth.json"),
        path.join(codexHome, "auth.json"),
      )
    : false
  const opencodeSourceDataHome = process.env.OPENCODE_DATA_HOME
    ?? path.join(real.XDG_DATA_HOME, "opencode")
  const opencodeAuthSource = path.join(opencodeSourceDataHome, "auth.json")
  const opencodeDataAuthCopied = providers.includes("opencode")
    ? await copySecretIfPresent(
        opencodeAuthSource,
        path.join(opencodeDataHome, "auth.json"),
      )
    : false
  const opencodeXdgAuthCopied = providers.includes("opencode")
    ? await copySecretIfPresent(
        opencodeAuthSource,
        path.join(xdgDataHome, "opencode", "auth.json"),
      )
    : false

  return {
    secretRoot,
    providerEnv: {
      HOME: isolatedHome,
      CODEX_HOME: codexHome,
      OPENCODE_CONFIG_DIR: real.OPENCODE_CONFIG_DIR,
      OPENCODE_DATA_HOME: opencodeDataHome,
      XDG_CONFIG_HOME: real.XDG_CONFIG_HOME,
      XDG_DATA_HOME: xdgDataHome,
      XDG_STATE_HOME: xdgStateHome,
      XDG_CACHE_HOME: xdgCacheHome,
    },
    evidence: {
      mode: "isolated",
      codex_auth_copied: codexAuthCopied,
      opencode_auth_copied: opencodeDataAuthCopied || opencodeXdgAuthCopied,
      claude_auth_copied: false,
      claude_auth_verified: false,
      claude_config_copied: false,
      claude_settings_copied: false,
      opencode_config_shared: true,
      provider_data_shared: false,
      provider_cache_shared: false,
      provider_home_shared: false,
    },
  }
}

export function workerResumeDaemonEnv({
  ports,
  root,
  relayToken,
  daemonId,
  daemonAlias,
  machineId,
  machineAlias,
  acceptRemoteLeases,
  socketName,
  kernelPort,
  mcpPort,
  openCodePort,
  codexPort,
  providerEnv = realProviderEnv(),
}) {
  const xdgConfigHome = path.join(root, `${daemonId}-xdg-config`)
  return {
    ...process.env,
    ...providerEnv,
    CHARIOX_HOME: path.join(xdgConfigHome, "chariox"),
    XDG_CONFIG_HOME: xdgConfigHome,
    XDG_STATE_HOME: path.join(root, `${daemonId}-xdg-state`),
    CHARIOX_KERNEL_PORT: String(kernelPort),
    CHARIOX_MCP_PORT: String(mcpPort),
    CHARIOX_OPENCODE_PORT: String(openCodePort),
    CHARIOX_CODEX_PORT: String(codexPort),
    CHARIOX_RELAY_URL: `ws://127.0.0.1:${ports.relayPort}`,
    CHARIOX_RELAY_TOKEN: relayToken,
    CHARIOX_DAEMON_ID: daemonId,
    CHARIOX_DAEMON_ALIAS: daemonAlias,
    CHARIOX_MACHINE_ID: machineId,
    CHARIOX_MACHINE_ALIAS: machineAlias,
    CHARIOX_ACCEPT_REMOTE_LEASES: acceptRemoteLeases ? "1" : "0",
    CHARIOX_DAEMON_SOCKET: path.join(root, socketName),
    CHARIOX_SESSION_HISTORY_DIR: path.join(root, `${daemonId}-history`),
    CHARIOX_CAPABILITY_ISOLATION_ROOT: path.join(root, `${daemonId}-capabilities`),
    CHARIOX_PROVIDER_RUNTIME_INIT_DELAY_MS: "250",
  }
}

export function spawnLogged(command, args, { cwd, env, stdoutPath, stderrPath }) {
  const stdout = createWriteStream(stdoutPath, { flags: "a" })
  const stderr = createWriteStream(stderrPath, { flags: "a" })
  const child = spawn(command, args, {
    cwd,
    env,
    stdio: ["ignore", "pipe", "pipe"],
  })
  child.stdout?.pipe(stdout)
  child.stderr?.pipe(stderr)
  return child
}

export async function runLoggedCommand(command, args, { cwd, env, stdoutPath, stderrPath, timeoutMs }) {
  const child = spawnLogged(command, args, { cwd, env, stdoutPath, stderrPath })
  try {
    const status = await withTimeout(
      new Promise((resolve, reject) => {
        child.on("error", reject)
        child.on("close", (code, signal) => resolve({ code, signal }))
      }),
      `${command} ${args.join(" ")}`,
      timeoutMs,
    )
    if (status.code !== 0) {
      const stdoutTail = await readLogTail(stdoutPath)
      const stderrTail = await readLogTail(stderrPath)
      throw new Error([
        `${command} ${args.join(" ")} exited with code ${status.code}${status.signal ? ` signal ${status.signal}` : ""}`,
        stdoutTail ? `stdout tail:\n${stdoutTail}` : null,
        stderrTail ? `stderr tail:\n${stderrTail}` : null,
      ].filter(Boolean).join("\n"))
    }
  } catch (error) {
    await terminateChild(child)
    throw error
  }
}

async function readLogTail(filePath) {
  if (!filePath) return ""
  try {
    return (await readFile(filePath, "utf8")).split("\n").slice(-80).join("\n").trim()
  } catch {
    return ""
  }
}

export async function waitForProviderRun({ client, providerRunId, timeoutMs, pollMs, requireThreadId = true }) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    const response = await client.send(getProviderRunRequest(providerRunId)).catch((error) => {
      last = { error: error.message ?? String(error) }
      return null
    })
    const run = response ? variant(response, "ProviderRun").provider_run : null
    if (run) {
      last = providerRunSnapshot(run)
      const state = String(run.state ?? "").toLowerCase()
      const threadId = providerThreadId(run)
      if ((state === "running" || state === "parked" || state === "starting") && (!requireThreadId || threadId)) {
        return run
      }
      if (state === "ended" || state === "failed" || state === "error") {
        throw new Error(`provider run ${providerRunId} ended before becoming ready: ${JSON.stringify(last)}`)
      }
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for provider run ${providerRunId}; last=${JSON.stringify(last)}`)
}

export async function waitForProviderRunEnded({ client, providerRunId, timeoutMs, pollMs }) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    const response = await client.send(getProviderRunRequest(providerRunId)).catch((error) => {
      last = { error: error.message ?? String(error) }
      return null
    })
    const run = response ? variant(response, "ProviderRun").provider_run : null
    if (run) {
      last = providerRunSnapshot(run)
      if (String(run.state ?? "").toLowerCase() === "ended") {
        return run
      }
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for provider run ${providerRunId} to end; last=${JSON.stringify(last)}`)
}

export async function waitForActiveProviderRunChange({ client, sessionId, previousRunId, timeoutMs, pollMs }) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    const state = variantAny(await client.send(getSessionStateRequest(sessionId)), "SessionState", "SessionStateLoaded")
    const session = state.session ?? state
    const activeRunId = session.active_provider_run_id ?? null
    last = { activeRunId }
    if (activeRunId && activeRunId !== previousRunId) {
      const run = await waitForProviderRun({
        client,
        providerRunId: activeRunId,
        timeoutMs: Math.min(timeoutMs, 180_000),
        pollMs,
        requireThreadId: true,
      })
      return run
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for provider run change from ${previousRunId}; last=${JSON.stringify(last)}`)
}

export async function waitForSessionActiveProviderRun({ client, sessionId, timeoutMs, pollMs }) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    const state = variantAny(await client.send(getSessionStateRequest(sessionId)), "SessionState", "SessionStateLoaded")
    const session = state.session ?? state
    const activeRunId = session.active_provider_run_id ?? null
    last = { activeRunId }
    if (activeRunId) {
      const run = await waitForProviderRun({
        client,
        providerRunId: activeRunId,
        timeoutMs: Math.min(timeoutMs, 180_000),
        pollMs,
        requireThreadId: true,
      })
      return run
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for active provider run in session ${sessionId}; last=${JSON.stringify(last)}`)
}

export async function waitForPromptIdle({ client, sessionId, attachmentId, agentId, timeoutMs, pollMs }) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    const state = variantAny(await client.send(getSessionStateRequest(sessionId)), "SessionState", "SessionStateLoaded")
    const session = state.session ?? state
    const agent = (session.agents ?? []).find((entry) => entry.id === agentId)
    const promptState = session.prompt_states?.[agentId]
    const activePrompt = promptState?.active_prompt ?? (session.active_prompt?.target_agent_id === agentId ? session.active_prompt : null)
    const queuedPrompts = promptState?.queued_prompts ?? (session.queued_prompts ?? []).filter((prompt) => prompt.target_agent_id === agentId)
    last = {
      agent_state: agent?.state ?? null,
      is_processing: agent?.is_processing ?? null,
      active_prompt: activePrompt?.id ?? null,
      queued_count: queuedPrompts?.length ?? 0,
    }
    if (!activePrompt && (queuedPrompts?.length ?? 0) === 0) return
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for agent ${agentId} to become idle; last=${JSON.stringify(last)}`)
}

export async function waitForSliceWorkerProvider({ client, sliceRef, provider, timeoutMs, pollMs }) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    const payload = variant(await client.send(getSliceRequest(sliceRef)), "Slice")
    const slice = payload.slice
    last = {
      status: slice?.status ?? null,
      worker_kernel_id: slice?.worker_kernel_id ?? null,
      worker_kernel_ref: slice?.worker_kernel_ref ?? null,
      providers: slice?.providers ?? [],
    }
    const providers = slice?.providers ?? []
    if (slice?.worker_kernel_id && providers.includes(provider)) return slice
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for slice ${sliceRef} worker provider ${provider}; last=${JSON.stringify(last)}`)
}

export async function loadAgentHistoryEntries(client, sessionId, agentId, latestPromptCount = 20) {
  const outline = variant(
    await client.send(getSessionHistoryOutlineRequest(sessionId, [agentId], latestPromptCount)),
    "SessionHistoryOutline",
  )
  const entries = []
  const agent = outline.agents?.find((entry) => entry.agent_id === agentId)
  for (const turn of agent?.turns ?? []) {
    for (const row of turn.entries ?? []) {
      if (row?.entry) entries.push(row.entry)
    }
    for (const blob of turn.blobs ?? []) {
      const content = variant(
        await client.send(getSessionHistoryBlobContentRequest(sessionId, agentId, blob.blob_id)),
        "SessionHistoryBlobContent",
      )
      for (const row of content.entries ?? []) {
        if (row?.entry) entries.push(row.entry)
      }
    }
  }
  return entries
}

export async function waitForHistoryOutputMarker({ client, sessionId, attachmentId, agentId, marker, timeoutMs, pollMs, historyDir = null }) {
  const deadline = Date.now() + timeoutMs
  let lastText = ""
  let lastCompactText = ""
  let lastFallbackCompactText = ""
  let lastRawCompactText = ""
  let lastNormalizedText = ""
  while (Date.now() < deadline) {
    const entries = await loadAgentHistoryEntries(client, sessionId, agentId)
    const outputEntries = entries.filter((entry) => entry?.kind !== "user_prompt")
    const terminalError = terminalProviderHistoryError(outputEntries)
    if (terminalError) {
      throw new Error(
        `provider failed while waiting for marker ${marker}: ${historyEntryText(terminalError).slice(-4000)}`,
      )
    }
    const textFragments = outputEntries
      .filter((entry) => entry.agent_id == null || entry.agent_id === agentId)
      .map(historyEntryText)
      .filter(Boolean)
    const fallbackTextFragments = outputEntries.map(historyEntryText).filter(Boolean)
    lastText = textFragments.join("\n")
    lastCompactText = textFragments.join("")
    lastFallbackCompactText = fallbackTextFragments.join("")
    lastRawCompactText = historyDir
      ? await loadRawHistoryOutputText({ historyDir, sessionId, agentId }).catch(() => "")
      : ""
    lastNormalizedText = normalizeProviderOutputText([
      lastCompactText,
      lastFallbackCompactText,
      lastRawCompactText,
    ].join("\n"))
    if (
      lastText.includes(marker) ||
      lastCompactText.includes(marker) ||
      lastFallbackCompactText.includes(marker) ||
      lastRawCompactText.includes(marker) ||
      lastNormalizedText.includes(marker)
    ) {
      return {
        entries,
        text: lastText,
        compactText: lastCompactText,
        fallbackCompactText: lastFallbackCompactText,
        rawCompactText: lastRawCompactText,
        normalizedText: lastNormalizedText,
      }
    }
    await sleep(pollMs)
  }
  throw new Error(
    `timed out waiting for marker ${marker}\n${lastText.slice(-4000)}\ncompact:\n${lastCompactText.slice(-4000)}\nfallback_compact:\n${lastFallbackCompactText.slice(-4000)}\nraw_compact:\n${lastRawCompactText.slice(-4000)}\nnormalized:\n${lastNormalizedText.slice(-4000)}`,
  )
}

export function terminalProviderHistoryError(entries) {
  return entries.find((entry) => (
    entry?.kind === "provider_error"
    || entry?.kind === "error"
    || (
      entry?.kind === "notice"
      && /provider run .* ended unexpectedly/i.test(historyEntryText(entry))
    )
  )) ?? null
}

export function historyEntryText(entry) {
  if (!entry) return ""
  if (typeof entry.text === "string") return entry.text
  if (typeof entry.message === "string") return entry.message
  if (typeof entry.display_text === "string") return entry.display_text
  if (typeof entry.content === "string") return entry.content
  return ""
}

export async function loadRawHistoryOutputText({ historyDir, sessionId, agentId }) {
  const names = await readdir(historyDir)
  const fragments = []
  for (const name of names) {
    if (!name.startsWith(`${sessionId}-`) || !name.endsWith(".jsonl")) continue
    const file = await readFile(path.join(historyDir, name), "utf8")
    for (const line of file.split("\n")) {
      if (!line.trim()) continue
      let entry
      try {
        entry = JSON.parse(line)
      } catch {
        continue
      }
      if (entry?.kind === "user_prompt") continue
      if (entry?.agent_id != null && entry.agent_id !== agentId) continue
      const text = historyEntryText(entry)
      if (text) fragments.push(text)
    }
  }
  const operationalDatabases = [
    path.join(historyDir, "operational.db"),
    path.join(path.dirname(historyDir), "home-kernel-storage", "operational-history.db"),
  ]
  for (const operationalDatabase of operationalDatabases) {
    if (!existsSync(operationalDatabase)) continue
    let database
    try {
      const { DatabaseSync } = await import("node:sqlite")
      database = new DatabaseSync(operationalDatabase, { readOnly: true })
      const rows = database.prepare(`
        SELECT content
        FROM history_events
        WHERE session_id = ?
          AND kind <> 'user_prompt'
          AND (agent_id IS NULL OR agent_id = ?)
          AND content IS NOT NULL
        ORDER BY sequence
      `).all(sessionId, agentId)
      fragments.push(...rows.map((row) => row.content).filter(Boolean))
    } catch {
      // The history API and legacy JSONL remain authoritative when SQLite is
      // unavailable or an older operational schema is present.
    } finally {
      database?.close()
    }
  }
  return fragments.join("")
}

export function normalizeProviderOutputText(text) {
  return String(text)
    .replace(/\x1b\][^\x07]*(?:\x07|\x1b\\)/g, "")
    .replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, "")
    .replace(/\r/g, "")
}

export async function createDeterministicMcp(root, name) {
  const mcpPath = path.join(root, `${name}.mjs`)
  await writeFile(mcpPath, [
    "let buffer = Buffer.alloc(0)",
    "function write(message) { process.stdout.write(`${JSON.stringify(message)}\\n`) }",
    "function handle(message) {",
    "  const { id, method, params } = message",
    "  if (method === 'notifications/initialized') return",
    "  if (method === 'initialize') {",
    "    write({ jsonrpc: '2.0', id, result: { protocolVersion: '2024-11-05', capabilities: { tools: {} }, serverInfo: { name: 'chariox-provider-thread-transfer', version: '1.0.0' } } })",
    "    return",
    "  }",
    "  if (method === 'tools/list') {",
    "    write({ jsonrpc: '2.0', id, result: { tools: [{ name: 'thread_transfer_probe', description: 'Returns a marker for Chariox provider-thread transfer drills.', inputSchema: { type: 'object', properties: { marker: { type: 'string' } }, required: ['marker'] } }] } })",
    "    return",
    "  }",
    "  if (method === 'tools/call' && params?.name === 'thread_transfer_probe') {",
    "    write({ jsonrpc: '2.0', id, result: { content: [{ type: 'text', text: `THREAD_TRANSFER_PROBE:${params?.arguments?.marker ?? ''}` }] } })",
    "    return",
    "  }",
    "  write({ jsonrpc: '2.0', id, error: { code: -32601, message: `unknown method ${method}` } })",
    "}",
    "process.stdin.on('data', (chunk) => {",
    "  buffer = Buffer.concat([buffer, chunk])",
    "  while (true) {",
    "    const newline = buffer.indexOf('\\n')",
    "    if (newline < 0) return",
    "    const line = buffer.subarray(0, newline).toString('utf8').trim()",
    "    buffer = buffer.subarray(newline + 1)",
    "    if (line) handle(JSON.parse(line))",
    "  }",
    "})",
  ].join("\n"), "utf8")
  return mcpPath
}

export function mcpConfig(name, scriptPath) {
  return {
    name,
    transport: {
      type: "stdio",
      command: process.execPath,
      args: [scriptPath],
    },
    enabled: true,
    required: true,
    startup_timeout_sec: 45,
    tool_timeout_sec: 45,
  }
}

export async function collectProviderProcesses(client, provider) {
  const response = await client.send(listProviderProcessesRequest(provider)).catch((error) => ({
    error: error.message ?? String(error),
  }))
  return response
}
