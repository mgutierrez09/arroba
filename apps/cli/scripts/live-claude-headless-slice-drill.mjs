#!/usr/bin/env node
import { spawn } from "node:child_process"
import { access, mkdir, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { setTimeout as sleep } from "node:timers/promises"

import { finalizeDrillArtifacts, prepareDrillArtifacts } from "./lib/drill-artifacts.mjs"
import { LocalIpcClient } from "../dist/ipc.js"
import {
  attachToSessionRequest,
  createSessionRequest,
  createSliceRequest,
  deleteSliceRequest,
  endSessionRequest,
  getSessionHistoryBlobContentRequest,
  getSessionHistoryOutlineRequest,
  importSliceProviderAuthRequest,
  listRemoteMachinesRequest,
  pumpTerminalOutputRequest,
  startSliceRequest,
  submitPromptRequest,
} from "../dist/ipc-requests.js"
import {
  assertBinary,
  makeAvailablePorts,
  resolveBuiltBinarySync,
  runLogged,
  terminateChild,
  waitForTcpPort,
} from "./lib/drill-runtime-helpers.mjs"
import { CLAUDE_UNATTENDED_CREDENTIALS_GUIDANCE } from "./lib/live-provider-thread-transfer-runtime.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const kernelBinary = resolveBuiltBinarySync(
  path.join(repoRoot, "apps/kernel/target/debug/chariox-kernel"),
  path.join(repoRoot, "apps/kernel/Cargo.toml"),
  "chariox-kernel",
)
const relayBinary = resolveBuiltBinarySync(
  path.join(repoRoot, "apps/relay/target/debug/chariox-relay"),
  path.join(repoRoot, "apps/relay/Cargo.toml"),
  "chariox-relay",
)
const realHomeDir = os.homedir()
const defaultLocalDockerSliceImage = process.env.CHARIOX_SLICE_DOCKER_IMAGE ?? "chariox-slice-linux:0.1.0"

async function dockerHostEnv() {
  if (process.env.DOCKER_HOST?.trim()) return process.env.DOCKER_HOST
  const colimaSocket = path.join(realHomeDir, ".colima", "default", "docker.sock")
  if (await access(colimaSocket).then(() => true, () => false)) {
    return `unix://${colimaSocket}`
  }
  return null
}

function parseArgs(argv) {
  const options = {
    provider: "claude-headless",
    model: "sonnet",
    timeoutMs: 300_000,
    keepArtifactsOnFailure: false,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--provider") options.provider = argv[++index]
    else if (arg === "--model") options.model = argv[++index]
    else if (arg === "--timeout-ms") options.timeoutMs = Number(argv[++index])
    else if (arg === "--keep-artifacts-on-failure") options.keepArtifactsOnFailure = true
    else if (arg === "--help" || arg === "-h") {
      console.log("Usage: node apps/cli/scripts/live-claude-headless-slice-drill.mjs [--provider claude-headless] [--model sonnet] [--timeout-ms 300000] [--keep-artifacts-on-failure]")
      process.exit(0)
    } else {
      throw new Error(`unknown argument: ${arg}`)
    }
  }
  if (options.provider !== "claude-headless") {
    throw new Error(`unsupported provider ${options.provider}; this drill validates claude-headless`)
  }
  return options
}

function unwrap(response, variant) {
  if (!response || !(variant in response)) {
    throw new Error(`expected ${variant}, got ${JSON.stringify(response)}`)
  }
  return response[variant]
}

function relayClient(relayUrl, relayToken, targetDaemonAlias, targetDaemonId = null) {
  return new LocalIpcClient(relayUrl, {
    relayAuthToken: relayToken,
    targetDaemonAlias: targetDaemonId ? undefined : targetDaemonAlias,
    targetDaemonId: targetDaemonId ?? undefined,
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  })
}

async function waitForLocalDaemon(kernelUrl, workspace, worktree) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const client = new LocalIpcClient(kernelUrl)
    try {
      const session = unwrap(await client.send(createSessionRequest(workspace, worktree)), "SessionCreated").session
      await client.send(endSessionRequest(session.id)).catch(() => {})
      await client.close()
      return
    } catch {
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error("home kernel did not become ready")
}

async function waitForRelayTarget(relayUrl, relayToken, targetDaemonAlias, targetDaemonId = null) {
  for (let attempt = 0; attempt < 120; attempt += 1) {
    const client = relayClient(relayUrl, relayToken, targetDaemonAlias, targetDaemonId)
    try {
      await Promise.race([
        client.send(listRemoteMachinesRequest()),
        sleep(2_000).then(() => {
          throw new Error("relay target probe timed out")
        }),
      ])
      await client.close().catch(() => {})
      return
    } catch {
      await client.close().catch(() => {})
      await sleep(500)
    }
  }
  throw new Error(`relay target ${targetDaemonId ?? targetDaemonAlias} did not become reachable`)
}

async function sendWithTimeout(client, request, timeoutMs, label) {
  return await Promise.race([
    client.send(request),
    sleep(timeoutMs).then(() => {
      throw new Error(`${label} did not complete within ${timeoutMs}ms`)
    }),
  ])
}

async function prebuildLocalDockerSliceImageIfNeeded(root, policy) {
  if (policy !== "always") return
  const logFile = path.join(root, "slice-image-build.log")
  await runLogged("docker", [
    "build",
    "-f",
    path.join(repoRoot, "apps/kernel/slice-linux-docker/docker/Dockerfile"),
    "-t",
    defaultLocalDockerSliceImage,
    repoRoot,
  ]).catch((error) => {
    throw new Error(`failed to build local Docker slice image; see ${logFile}: ${error.message}`)
  })
}

async function loadAgentHistoryEntries(client, sessionId, agentId, latestPromptCount = 20) {
  const outline = unwrap(
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
      const content = unwrap(
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

async function waitForHistoryMarker(client, sessionId, attachmentId, agentId, marker, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  let lastText = ""
  while (Date.now() < deadline) {
    await client.send(pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => {})
    const entries = await loadAgentHistoryEntries(client, sessionId, agentId)
    lastText = entries
      .filter((entry) => entry.agent_id === agentId && entry.kind !== "user_prompt")
      .map((entry) => entry.text ?? "")
      .join("\n")
    if (lastText.includes(marker)) return { entries, text: lastText }
    await sleep(1_000)
  }
  throw new Error(`timed out waiting for marker ${marker}\n${lastText.slice(-4000)}`)
}

async function main() {
  throw new Error(CLAUDE_UNATTENDED_CREDENTIALS_GUIDANCE)

  const options = parseArgs(process.argv.slice(2))
  await assertBinary(kernelBinary, path.join(repoRoot, "apps/kernel/Cargo.toml"), "chariox-kernel")
  await assertBinary(relayBinary, path.join(repoRoot, "apps/relay/Cargo.toml"), "chariox-relay")

  const root = path.join("/tmp", `arb-claude-headless-slice-${process.pid}-${Date.now()}`)
  const ports = await makeAvailablePorts()
  const relayToken = `claude-headless-slice-token-${process.pid}`
  const relayUrl = `ws://127.0.0.1:${ports.relayPort}`
  const homeKernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  const targetDaemonAlias = `claude-headless-slice-home-${process.pid}`
  const workspace = path.join(root, "workspace")
  const homeDir = path.join(root, "home")
  const xdgConfigHome = path.join(root, "xdg-config")
  const xdgStateHome = path.join(root, "xdg-state")
  const xdgDataHome = path.join(root, "xdg-data")
  const xdgCacheHome = path.join(root, "xdg-cache")
  const sliceBuildImagePolicy = process.env.CHARIOX_NATIVE_TUI_SLICE_BUILD_IMAGE ?? "always"
  const rustMinStack = process.env.RUST_MIN_STACK ?? "16777216"
  const dockerHost = await dockerHostEnv()
  const dockerContext = dockerHost?.includes("/.colima/") ? "colima" : process.env.DOCKER_CONTEXT
  let relay = null
  let kernel = null
  let client = null
  let sessionId = null
  let sliceId = null
  let succeeded = false
  let failure = null
  try {
    await prepareDrillArtifacts(root)
    await mkdir(workspace, { recursive: true })
    await mkdir(homeDir, { recursive: true })
    await mkdir(xdgConfigHome, { recursive: true })
    await mkdir(xdgStateHome, { recursive: true })
    await mkdir(xdgDataHome, { recursive: true })
    await mkdir(xdgCacheHome, { recursive: true })
    await mkdir(path.join(xdgConfigHome, "chariox"), { recursive: true })
    await writeFile(path.join(xdgConfigHome, "chariox", "config.toml"), [
      "version = 1",
      "",
      "[slices]",
      `root = ${JSON.stringify(path.join(root, "slices"))}`,
      "",
      "[slices.linux]",
      `docker_image = ${JSON.stringify(defaultLocalDockerSliceImage)}`,
      `build_image = ${JSON.stringify(sliceBuildImagePolicy === "always" ? "auto" : sliceBuildImagePolicy)}`,
      "",
    ].join("\n"))
    await prebuildLocalDockerSliceImageIfNeeded(root, sliceBuildImagePolicy)
    console.log(`[claude-headless-slice-drill] docker-host ${dockerHost ?? "default"} context ${dockerContext ?? "default"}`)

    relay = spawn(relayBinary, [], {
      cwd: repoRoot,
      env: {
        ...process.env,
        CHARIOX_RELAY_HOST: "127.0.0.1",
        CHARIOX_RELAY_PORT: String(ports.relayPort),
        CHARIOX_RELAY_TOKEN: relayToken,
        RUST_MIN_STACK: rustMinStack,
      },
      stdio: ["ignore", "ignore", "inherit"],
    })
    await waitForTcpPort(ports.relayPort)
    kernel = spawn(kernelBinary, [], {
      cwd: repoRoot,
      env: {
        ...process.env,
        HOME: homeDir,
        XDG_CONFIG_HOME: xdgConfigHome,
        XDG_STATE_HOME: xdgStateHome,
        XDG_DATA_HOME: xdgDataHome,
        XDG_CACHE_HOME: xdgCacheHome,
        CODEX_HOME: process.env.CODEX_HOME ?? path.join(realHomeDir, ".codex"),
        OPENCODE_CONFIG_DIR: process.env.OPENCODE_CONFIG_DIR ?? path.join(realHomeDir, ".config", "opencode"),
        CHARIOX_LOG_DIR: path.join(root, "logs"),
        CHARIOX_KERNEL_PORT: String(ports.kernelPort),
        CHARIOX_MCP_PORT: String(ports.mcpPort),
        CHARIOX_OPENCODE_PORT: String(ports.openCodePort),
        CHARIOX_CODEX_PORT: String(ports.codexPort),
        CHARIOX_RELAY_URL: relayUrl,
        CHARIOX_RELAY_TOKEN: relayToken,
        CHARIOX_DAEMON_ID: `claude-headless-slice-home-${process.pid}-${Date.now()}`,
        CHARIOX_DAEMON_ALIAS: targetDaemonAlias,
        CHARIOX_MACHINE_ID: `claude-headless-slice-machine-${process.pid}`,
        CHARIOX_MACHINE_ALIAS: targetDaemonAlias,
        CHARIOX_ACCEPT_REMOTE_LEASES: "0",
        CHARIOX_DAEMON_SOCKET: path.join(root, "home.sock"),
        CHARIOX_SESSION_HISTORY_DIR: path.join(root, "history"),
        RUST_MIN_STACK: rustMinStack,
        ...(dockerHost ? { DOCKER_HOST: dockerHost } : {}),
        ...(dockerContext ? { DOCKER_CONTEXT: dockerContext } : {}),
      },
      stdio: ["ignore", "ignore", "inherit"],
    })
    await waitForLocalDaemon(homeKernelUrl, workspace, workspace)
    await waitForRelayTarget(relayUrl, relayToken, targetDaemonAlias)

    client = new LocalIpcClient(homeKernelUrl, {
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    const createdSlice = unwrap(await client.send(createSliceRequest({
      name: `claude-headless-${process.pid}`,
      backend: "local_docker",
      os: "linux",
      workspaceMount: workspace,
    })), "SliceCreated").slice
    sliceId = createdSlice.id
    const startedSlice = unwrap(await client.send(startSliceRequest(sliceId)), "SliceStarted").slice
    await client.send(importSliceProviderAuthRequest(startedSlice.id, options.provider))

    const created = unwrap(await client.send(createSessionRequest(workspace, workspace)), "SessionCreated")
    sessionId = created.session.id
    const attachment = unwrap(await client.send(attachToSessionRequest(sessionId, `claude-headless-slice-${Date.now()}`)), "SessionAttached").attachment
    const marker = `CLAUDE_HEADLESS_SLICE_OK_${process.pid}_${Date.now()}`
    const spawned = unwrap(await client.send({
      SpawnAgent: {
        session_id: sessionId,
        alias: "slice-claude-headless",
        provider: options.provider,
        model: options.model,
        effort: "low",
        execution_mode: null,
        permission_level: "yolo",
        worktree_id: null,
        kernel_ref: null,
        slice_ref: startedSlice.id,
        worktree_placement: null,
      },
    }), "AgentSpawned")
    await sendWithTimeout(
      client,
      submitPromptRequest(
        sessionId,
        attachment.id,
        spawned.agent.id,
        `Reply with exactly ${marker}.`,
        [],
      ),
      Math.min(options.timeoutMs, 180_000),
      "claude-headless slice prompt submission",
    )
    const history = await waitForHistoryMarker(client, sessionId, attachment.id, spawned.agent.id, marker, options.timeoutMs)
    succeeded = true
    console.log(JSON.stringify({
      status: "ok",
      provider: options.provider,
      model: options.model,
      sessionId,
      sliceId: startedSlice.id,
      workerKernelRef: startedSlice.worker_kernel_ref ?? null,
      workerKernelId: startedSlice.worker_kernel_id ?? null,
      agentId: spawned.agent.id,
      marker,
      historyEntries: history.entries.length,
    }, null, 2))
  } catch (error) {
    failure = error
    throw error
  } finally {
    if (client) {
      if (sessionId) await client.send(endSessionRequest(sessionId)).catch(() => {})
      if (sliceId) await client.send(deleteSliceRequest(sliceId)).catch(() => {})
      await client.close().catch(() => {})
    }
    await terminateChild(kernel)
    await terminateChild(relay)
    await finalizeDrillArtifacts({
      rootDir: root,
      passed: succeeded,
      preserveOnFailure: options.keepArtifactsOnFailure,
      failure,
      metadata: {
        drill: "claude-headless-slice",
        provider: options.provider,
        model: options.model,
        timeoutMs: options.timeoutMs,
        relayUrl,
        homeKernelUrl,
        targetDaemonAlias,
        workspace,
        homeDir,
        xdgConfigHome,
        xdgStateHome,
        xdgDataHome,
        xdgCacheHome,
        sessionId,
        sliceId,
        sliceBuildImagePolicy,
        dockerHost: dockerHost ?? null,
        dockerContext: dockerContext ?? null,
      },
      log: (name, details) => console.log(`[claude-headless-slice-drill] ${name}`, JSON.stringify(details)),
    })
    if (!succeeded && options.keepArtifactsOnFailure) {
      console.error(`[claude-headless-slice-drill] artifacts kept at ${root}`)
    }
  }
}

try {
  await main()
  process.exit(0)
} catch (error) {
  console.error(error)
  process.exit(1)
}
