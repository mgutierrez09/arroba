#!/usr/bin/env node

import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { createHash } from "node:crypto"
import { createWriteStream } from "node:fs"
import { access, chmod, mkdir, mkdtemp, readFile, readdir, rm, stat, writeFile } from "node:fs/promises"
import http from "node:http"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { fileURLToPath, pathToFileURL } from "node:url"

import { runRoomEnvironmentCompanion } from "./lib/live-room-environment-companion-verifier.mjs"
import { captureRoomStreamerDiagnostics } from "./lib/room-streamer-diagnostics.mjs"
import { captureRoomKernelDiagnostics } from "./lib/room-kernel-diagnostics.mjs"
import { startRoomSliceWithForwarding } from "./lib/room-colima-forwarding.mjs"
import { roomRealProviderOptions, runRoomRealProvider } from "./lib/live-room-real-provider.mjs"
import { roomProviderBrowserFixture } from "./lib/room-provider-browser-fixture.mjs"
import { createDrillInterruption } from "./lib/drill-interruption.mjs"
import { makeAvailablePorts, portIsAvailable } from "./lib/drill-runtime-helpers.mjs"
import {
  assertRetainedClipboardEvidenceIsRedacted,
  assertRetainedTextIsRedacted,
  clipboardCaseSummary,
  textCaseSummary,
  utf8TextFromChunks,
} from "./lib/computer-clipboard-x11-drill.mjs"
import { assertRoomClipboardAction } from "./lib/room-environment-clipboard-drill.mjs"
import {
  assertRoomKeyboardKeyAction,
  assertRoomKeyboardTextAction,
} from "./lib/room-environment-computer-input-drill.mjs"
import {
  assertHumanDesktopTakeoverCompleted,
  assertRoomComputerActionCancelled,
  assertRoomComputerActionRunning,
  roomComputerCancellationTimings,
} from "./lib/room-environment-computer-cancellation-drill.mjs"
import {
  assertRoomPointerClickAction,
  assertRoomPointerDragAction,
  assertRoomPointerMoveAction,
  assertRoomPointerScrollAction,
} from "./lib/room-environment-computer-pointer-drill.mjs"
import {
  automationNoticeEntries,
  automationNoticeIds,
  automationNoticeTexts,
  roomActionNoticePattern,
} from "./lib/room-tui-notices.mjs"
import { hasRoomReadyProjection } from "./lib/room-drill-ready-notices.mjs"
import { roomDrillRelayToken } from "./lib/room-drill-relay-token.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, "..", "..", "..")
const sliceMemoryMb = Number(process.env.CHARIOX_ROOM_DRILL_MEMORY_MB ?? 2048)
assert.ok(Number.isSafeInteger(sliceMemoryMb) && sliceMemoryMb > 0 && sliceMemoryMb <= 0xffff_ffff,
  "CHARIOX_ROOM_DRILL_MEMORY_MB must be a positive u32 number of MiB")
const companionOnly = process.env.CHARIOX_ROOM_DRILL_FOCUS === "web-companion"
const realProviderOptions = roomRealProviderOptions(process.env)
if (companionOnly && !process.env.CHARIOX_ROOM_DRILL_COORDINATION_DIR?.trim()) {
  throw new Error("web-companion focus requires CHARIOX_ROOM_DRILL_COORDINATION_DIR")
}
const kernelClientRoot = path.join(repoRoot, "packages", "kernel-client")
const startedAt = new Date().toISOString()
const stamp = startedAt.replace(/[:.]/g, "-")
const runId = `room-pointer-${process.pid}-${stamp}`
const webKeyboardText = process.env.CHARIOX_ROOM_DRILL_WEB_KEYBOARD === "1"
  ? `web-${runId}-Grüße 世界`
  : null
const webKeyboardReplacementText = webKeyboardText ? `ime-${runId}-日本語` : null
const webPointerGestures = process.env.CHARIOX_ROOM_DRILL_WEB_GESTURES === "1"
const evidenceRoot = path.join(
  os.homedir(),
  ".codex",
  "evidence",
  "browser-computer-use",
  "computer-secret-room-e2e",
  stamp,
)
const containerName = `chariox-slice-${runId}`
const homeVolume = `${containerName}-home`
const userCredentialId = `${runId}-user-computer`
const generatedCredentialId = `${runId}-generated-computer`
const drillCredentialIds = [userCredentialId, generatedCredentialId]
const userSecret = `room-computer-secret-${process.pid}-${Date.now()}`
const vaultPassphrase = `room-vault-passphrase-${process.pid}-${Date.now()}`
const agentClipboardText = `agent-clipboard-${runId}-Grüße 世界\nsecond line\n`
const blockedAgentClipboardText = `blocked-agent-clipboard-${runId}\n`
const humanClipboardText = `human-clipboard-${runId}\t\nsecond line\n`
const physicalClipboardText = `physical-clipboard-${runId}-áéíóú\nsecond line\n`
// Cross the former 5-second worker and 15-second relay limits using physical
// input, without making the acceptance dependent on clipboard insertion.
const keyboardText = `keyboard-${runId}-Grüße 世界 ${"long-input ".repeat(38)}`
const keyboardReplacementText = `focus-${runId}-ABC`
const keyboardAfterRepeat = keyboardReplacementText.slice(0, -3)
const cancellationText = `cancel-${runId}-` + "x".repeat(1_800)
const takeoverCancellationText = `takeover-${runId}-` + "y".repeat(1_800)
const cancellationRecoveryText = `recovered-${runId}`
const pointerMatrix = Object.freeze({
  move: { x: 160, y: 220 },
  singleClick: { x: 460, y: 220, button: "left", clickCount: 1 },
  rightClick: { x: 700, y: 220, button: "right", clickCount: 1 },
  doubleClick: { x: 940, y: 220, button: "left", clickCount: 2 },
  textSelection: {
    fromX: 220,
    fromY: 450,
    toX: 900,
    toY: 450,
    button: "left",
  },
  scroll: {
    x: 640,
    y: 700,
    horizontalSteps: 4,
    verticalSteps: 5,
  },
  keyboardFocus: { x: 640, y: 393, button: "left", clickCount: 1 },
})
const clipboardValues = [
  agentClipboardText,
  blockedAgentClipboardText,
  humanClipboardText,
  physicalClipboardText,
]
const sensitiveValues = [
  ...(webKeyboardText ? [webKeyboardText] : []),
  ...(webKeyboardReplacementText ? [webKeyboardReplacementText] : []),
  userSecret,
  vaultPassphrase,
  keyboardText,
  keyboardReplacementText,
  keyboardAfterRepeat,
  cancellationText,
  takeoverCancellationText,
  cancellationRecoveryText,
  ...clipboardValues,
]
const generatedSecretLength = 24
const { kernelPort, relayPort } = await makeAvailablePorts({
  candidateFactory: () => {
    const kernelPort = 20000 + Math.floor(Math.random() * 4000)
    return { kernelPort, relayPort: kernelPort + 20 }
  },
  localAvailability: async ({ kernelPort, relayPort }) => (await Promise.all(
    [kernelPort, kernelPort + 1, kernelPort + 2, kernelPort + 3, relayPort].map(portIsAvailable),
  )).every(Boolean),
})
const relayScopedIssuer = `${runId}-issuer`
const relayScopedSecret = `${runId}-scoped-secret`
const homeDaemonId = `${runId}-home`
const daemonRelayToken = scopedRelayToken({
  subject: homeDaemonId,
  subjectKind: "kernel",
  actions: ["daemon_register", "daemon_heartbeat", "packet_route", "peer_request", "peer_event"],
})
const remoteTuiRelayToken = scopedRelayToken({
  subject: `${runId}-remote-tui`,
  subjectKind: "client",
  actions: ["client_metadata_read", "client_connect", "packet_route"],
  userId: "local",
})
const directDaemonEnvironmentNames = [
  "CHARIOX_DAEMON_SOCKET",
  "CHARIOX_DAEMON_ID",
  "CHARIOX_DAEMON_ALIAS",
  "CHARIOX_KERNEL_PORT",
  "CHARIOX_MCP_PORT",
  "CHARIOX_CODEX_PORT",
  "CHARIOX_OPENCODE_PORT",
  "CHARIOX_MACHINE_ID",
  "CHARIOX_MACHINE_ALIAS",
  "CHARIOX_RELAY_URL",
  "CHARIOX_RELAY_TOKEN",
  "CHARIOX_SESSION_HISTORY_DIR",
]
const tempRootPromise = realProviderOptions
  ? mkdir(path.join(os.homedir(), ".chariox", "dev", "browser-computer-use"), { recursive: true })
    .then(() => mkdtemp(path.join(os.homedir(), ".chariox", "dev", "browser-computer-use", "room-provider-")))
  : mkdtemp(path.join(os.tmpdir(), "chariox-room-pointer-"))
const children = []
let localForwarding = null
const resources = []
let client = null
let observerClient = null
let workerClient = null
let localAutomation = null
let remoteAutomation = null
const tuiOutput = { local: "", remote: "" }
let remoteTuiHome = null
let fixture = null
let slice = null
let sessionId = null
let requests = null
let failure = null
let result = null
let companionResult = null
let secretAgent = null
let secretProviderRun = null
let sourceIdentity = null
let sliceRuntimeIdentity = null
let fixtureWorkspace = repoRoot

const interruption = createDrillInterruption()
await interruption.run(async () => {
  await mkdir(evidenceRoot, { recursive: true })
  await run()
}, cleanup, (error) => { failure = error })

if (failure) {
  console.error(failure?.stack ?? String(failure))
  process.exitCode = 1
} else {
  console.log(JSON.stringify({ status: "passed", evidenceRoot }, null, 2))
}

async function run() {
  const tempRoot = await tempRootPromise
  if (realProviderOptions) {
    assert.equal((await stat(tempRoot)).mode & 0o777, 0o700, "provider workspace parent must remain private")
    fixtureWorkspace = path.join(tempRoot, "provider-workspace")
    await mkdir(fixtureWorkspace, { recursive: true })
    // Colima bind mounts preserve host ownership inside the provider user
    // namespace. Only this empty workspace is shared; its mkdtemp parent stays
    // private, and no repository permissions or host account data are changed.
    await chmod(fixtureWorkspace, 0o777)
  }
  await assertDockerReady()
  resources.push(await resourceSnapshot("before"))
  fixture = await startFixture()
  await seedConfig(tempRoot)

  const kernelBinary = await resolveRuntimeBinary("chariox-kernel")
  const relayBinary = await resolveRuntimeBinary("chariox-relay")
  sourceIdentity = await captureSourceIdentity(kernelBinary, relayBinary)
  const relayLog = createWriteStream(path.join(evidenceRoot, "relay.log"), { flags: "a" })
  const relay = spawn(relayBinary, [], {
    cwd: repoRoot,
    env: {
      ...process.env,
      CHARIOX_RELAY_HOST: "127.0.0.1",
      CHARIOX_RELAY_PORT: String(relayPort),
      CHARIOX_RELAY_SCOPED_ISSUER: relayScopedIssuer,
      CHARIOX_RELAY_SCOPED_HMAC_SECRET: relayScopedSecret,
    },
    stdio: ["ignore", "pipe", "pipe"],
  })
  relay.stdout.pipe(relayLog)
  relay.stderr.pipe(relayLog)
  relay.once("exit", () => relayLog.end())
  children.push(relay)
  await waitForTcpPort("127.0.0.1", relayPort, 20_000, "relay did not accept connections")

  const log = createWriteStream(path.join(evidenceRoot, "kernel.log"), { flags: "a" })
  const kernelEnv = {
    ...process.env,
    CHARIOX_HOME: path.join(tempRoot, "home"),
    CHARIOX_LOG_DIR: path.join(tempRoot, "kernel-logs"),
    CHARIOX_KERNEL_PORT: String(kernelPort),
    CHARIOX_MCP_PORT: String(kernelPort + 1),
    CHARIOX_CODEX_PORT: String(kernelPort + 2),
    CHARIOX_OPENCODE_PORT: String(kernelPort + 3),
    CHARIOX_DAEMON_SOCKET: path.join(tempRoot, "daemon.sock"),
    CHARIOX_DAEMON_ID: homeDaemonId,
    CHARIOX_DAEMON_ALIAS: homeDaemonId,
    CHARIOX_MACHINE_ID: `${runId}-machine`,
    CHARIOX_MACHINE_ALIAS: `${runId}-machine`,
    CHARIOX_RELAY_URL: `ws://127.0.0.1:${relayPort}`,
    CHARIOX_RELAY_TOKEN: daemonRelayToken,
    CHARIOX_SESSION_HISTORY_DIR: path.join(tempRoot, "history"),
    // Keep the live drill within laptop memory limits. The provisioned worker
    // still runs exact-head code, but a stripped dev link uses materially less
    // memory than an optimized release link inside Docker Desktop.
    CHARIOX_SLICE_RUNTIME_BUILD_PROFILE: "dev",
    CHARIOX_SLICE_DOCKER_PIDS_LIMIT: "1024",
    XDG_CONFIG_HOME: path.join(tempRoot, "xdg-config"),
    XDG_STATE_HOME: path.join(tempRoot, "xdg-state"),
    XDG_CACHE_HOME: path.join(tempRoot, "xdg-cache"),
  }
  const kernel = spawn(kernelBinary, [], {
    cwd: repoRoot,
    env: kernelEnv,
    stdio: ["ignore", "pipe", "pipe"],
  })
  kernel.stdout.pipe(log)
  kernel.stderr.pipe(log)
  kernel.once("exit", () => log.end())
  children.push(kernel)

  const [{ LocalIpcClient }, importedRequests, { createRoomEnvironmentActivityController }] = await Promise.all([
    import(pathToFileURL(path.join(kernelClientRoot, "dist", "ipc.js")).href),
    import(pathToFileURL(path.join(kernelClientRoot, "dist", "ipc-requests.js")).href),
    import(pathToFileURL(path.join(repoRoot, "apps", "cli", "dist", "room-environment-activity-controller.js")).href),
  ])
  requests = importedRequests
  client = await waitFor(async () => {
    const candidate = interruption.guardClient(new LocalIpcClient(`ws://127.0.0.1:${kernelPort}/kernel`))
    try {
      await candidate.send(requests.listSlicesRequest())
      return candidate
    } catch (error) {
      candidate.close?.()
      throw error
    }
  }, 60_000, "kernel did not accept local connections")
  observerClient = interruption.guardClient(new LocalIpcClient(`ws://127.0.0.1:${kernelPort}/kernel`))

  const session = unwrap(
    await client.send(requests.createSessionRequest(fixtureWorkspace, fixtureWorkspace, runId)),
    "SessionCreated",
  ).session
  sessionId = session.id
  remoteAutomation = await startRemoteTui({ tempRoot })
  const attachedRemoteTui = await waitForAutomationSnapshot(
    remoteAutomation,
    (snapshot) => snapshot.session?.id === sessionId,
    "relay-attached remote TUI session",
    30_000,
  )
  assert.equal(attachedRemoteTui.session?.id, sessionId)
  localAutomation = await startLocalTui({ tempRoot, kernelUrl: `ws://127.0.0.1:${kernelPort}/kernel` })
  const attachedLocalTui = await waitForAutomationSnapshot(
    localAutomation,
    (snapshot) => snapshot.session?.id === sessionId,
    "direct local TUI session",
    30_000,
  )
  assert.equal(attachedLocalTui.session?.id, sessionId)

  const createSliceResponse = await withTimeout(client.send(requests.createSliceRequest({
    name: runId,
    backend: "local_docker",
    displayMode: "headed",
    displayBackend: "selkies",
    workspaceMount: fixtureWorkspace,
    workerKernelRef: `${runId}-worker`,
    base: "clean",
  })), 15_000, "CreateSlice response")
  slice = unwrap(createSliceResponse, "SliceCreated").slice
  const binding = unwrap(
    await client.send(requests.bindRoomEnvironmentSliceRequest(sessionId, slice.id)),
    "RoomEnvironmentSlice",
  ).binding
  assert.equal(binding.session_id, sessionId)
  assert.equal(binding.slice_id, slice.id)

  localForwarding = await startRoomSliceWithForwarding({
    sshConfig: process.env.CHARIOX_ROOM_DRILL_COLIMA_SSH_CONFIG,
    slice,
    startSlice: () => client.send(requests.startSliceRequest(slice.id)),
    containerExists: async () => (await runCommand("docker", ["container", "inspect",
      "--format", "{{.Id}}", containerName], 2000)).code === 0,
  })
  if (localForwarding) sourceIdentity.localForwarding = {
    kind: "drill-owned-colima-ssh",
    ports: localForwarding.ports,
    loopbackOnly: true,
  }
  slice = await waitForSliceRunning(slice.id)
  sliceRuntimeIdentity = await inspectSliceRuntimeIdentity()
  assert.equal(
    sliceRuntimeIdentity.runtimeSourceRevision,
    sourceIdentity.runtimeSourceRevision,
    "slice image must contain the exact current runtime source",
  )
  assert.equal(
    sliceRuntimeIdentity.installedRuntimeSourceRevision,
    sourceIdentity.runtimeSourceRevision,
    "running worker must install the exact current runtime source",
  )
  assert.equal(
    sliceRuntimeIdentity.relayPeerProtocolVersion,
    String(sourceIdentity.protocolVersions.relayPeer),
    "slice image relay protocol must match current source",
  )
  const limits = await dockerLimits()
  assert.equal(limits.memoryBytes, sliceMemoryMb * 1024 * 1024)
  assert.equal(limits.memorySwapBytes, limits.memoryBytes)
  assert.equal(limits.nanoCpus, 1_000_000_000)
  assert.equal(limits.pidsLimit, 1024)
  await waitForBrowserReady(60_000)
  await sliceScreen(["open-url", `http://host.docker.internal:${fixture.port}/click`])
  await waitForBrowserText("POINTER_CLICK_READY", 30_000, "click fixture did not load")
  await screenshot("before-click")

  const environment = unwrap(await client.send(requests.startRoomEnvironmentRequest(sessionId, {
    css_width: 1280,
    css_height: 800,
    device_scale_factor: 1,
    desktop_pixel_width: 1280,
    desktop_pixel_height: 800,
  })), "RoomEnvironmentUpdated").environment
  assert.equal(environment.lifecycle, "ready")
  const [readyLocalTui, readyRemoteTui] = await Promise.all([
    waitForRoomReadyProjection(localAutomation, "local"),
    waitForRoomReadyProjection(remoteAutomation, "remote"),
  ])
  const localStatusSnapshot = await localAutomation.send(
    "submit_prompt",
    { prompt: "/room status" },
    20_000,
  )
  const localStatusNotice = automationNoticeTexts(localStatusSnapshot)
    .findLast((notice) => notice.startsWith(`Room environment ${environment.environment_id}\n`))
  assert.ok(localStatusNotice, "local TUI did not render the authoritative Room status")
  assert.match(localStatusNotice, /lifecycle=ready /)
  assert.match(localStatusNotice, /tab=.+ Room pointer drill — http:\/\/host\.docker\.internal:/)
  const remoteScreenshot = await captureScreenshotFromRemoteTui(tempRoot)
  const activityNotices = []
  const daemonActivities = []
  const activityController = createRoomEnvironmentActivityController({
    isAttached: () => true,
    sessionId: () => sessionId,
    nowMs: () => Date.now(),
    send: (request) => client.send(request),
    appendNotice: (message) => activityNotices.push(message),
    recordDaemonActivity: (kind) => daemonActivities.push(kind),
  })
  assert.equal(await activityController.synchronize(), true)
  assert.match(activityNotices.at(-1), /^Room screen: ready · tab Room pointer drill — /)

  const takeover = unwrap(
    await client.send(requests.requestRoomEnvironmentInputTakeoverRequest(sessionId, { kind: "desktop" })),
    "RoomEnvironmentTakeoverUpdated",
  )
  assert.equal(takeover.outcome.state, "granted")
  const desktopOwner = takeover.environment.input_ownership.find(
    (owner) => owner.target.kind === "desktop",
  )
  assert.equal(desktopOwner?.actor_id, "user:local")
  assert.equal(await activityController.synchronize(), true)
  assert.ok(activityNotices.includes("Room input: Local user controls desktop"))
  await Promise.all([
    waitForLocalNotice(/^Room input: Local user controls desktop$/),
    waitForRemoteNotice(/^Room input: Local user controls desktop$/),
  ])

  const noticesBeforePointers = activityNotices.length
  const localNoticesBeforePointers = automationNoticeTexts(
    await localAutomation.send("snapshot"),
  ).length
  const remoteNoticesBeforePointers = automationNoticeTexts(
    await remoteAutomation.send("snapshot"),
  ).length
  for (const pointer of [{ x: 200, y: 100 }, { x: 400, y: 200 }, { x: 640, y: 400 }]) {
    await client.send(requests.updateRoomEnvironmentPointerRequest(
      sessionId,
      takeover.environment.runtime_generation,
      takeover.environment.viewport.revision,
      pointer,
    ))
  }
  await activityController.synchronize()
  const noticesAfterPointers = activityNotices.slice(noticesBeforePointers)
  assert.equal(
    noticesAfterPointers.some((notice) => /pointer/i.test(notice)),
    false,
    `pointer movement leaked into TUI notices: ${noticesAfterPointers.join(" | ")}`,
  )
  await sleep(600)
  const localNoticesAfterPointers = automationNoticeTexts(
    await localAutomation.send("snapshot"),
  ).slice(localNoticesBeforePointers)
  assert.equal(
    localNoticesAfterPointers.some((notice) => /pointer/i.test(notice)),
    false,
    `pointer movement leaked into local TUI notices: ${localNoticesAfterPointers.join(" | ")}`,
  )
  const remoteNoticesAfterPointers = automationNoticeTexts(
    await remoteAutomation.send("snapshot"),
  ).slice(remoteNoticesBeforePointers)
  assert.equal(
    remoteNoticesAfterPointers.some((notice) => /pointer/i.test(notice)),
    false,
    `pointer movement leaked into remote TUI notices: ${remoteNoticesAfterPointers.join(" | ")}`,
  )

  const idempotencyKey = `${runId}-click`
  const click = unwrap(await client.send(requests.submitRoomEnvironmentActionRequest(
    sessionId,
    takeover.environment.runtime_generation,
    takeover.environment.viewport.revision,
    idempotencyKey,
    { kind: "pointer_click", x: 640, y: 400, button: "left", click_count: 1 },
  )), "RoomEnvironmentActionSubmitted")
  assert.equal(actionState(click.environment, click.action_id), "completed")
  assert.equal(await activityController.synchronize(), true)
  assert.match(activityNotices.at(-1), /^Room action #\d+: Local user · computer pointer_click · completed$/)
  await Promise.all([
    waitForLocalNotice(/^Room action #\d+: Local user · computer pointer_click · completed$/),
    waitForRemoteNotice(/^Room action #\d+: Local user · computer pointer_click · completed$/),
  ])
  await waitForBrowserText("POINTER_CLICK_COUNT=1", 20_000, "physical click did not reach the fixture")
  await screenshot("after-click")

  const retry = unwrap(await client.send(requests.submitRoomEnvironmentActionRequest(
    sessionId,
    takeover.environment.runtime_generation,
    takeover.environment.viewport.revision,
    idempotencyKey,
    { kind: "pointer_click", x: 640, y: 400, button: "left", click_count: 1 },
  )), "RoomEnvironmentActionSubmitted")
  assert.equal(retry.action_id, click.action_id)
  assert.equal(actionState(retry.environment, retry.action_id), "completed")
  await new Promise((resolve) => setTimeout(resolve, 500))
  assert.match(await sliceScreen(["browser-text"]), /POINTER_CLICK_COUNT=1/)

  const history = unwrap(
    await client.send(requests.listRoomEnvironmentActionHistoryRequest(sessionId, null, 25)),
    "RoomEnvironmentActionHistoryListed",
  ).page.actions
  assert.equal(history.filter((action) => action.action_id === click.action_id).length, 1)
  assert.equal(history.find((action) => action.action_id === click.action_id)?.actor_id, desktopOwner.actor_id)
  const released = unwrap(
    await client.send(requests.releaseRoomEnvironmentInputRequest(sessionId, { kind: "desktop" })),
    "RoomEnvironmentInputReleased",
  ).environment
  assert.equal(await activityController.synchronize(), true)
  assert.equal(activityNotices.at(-1), "Room input: available")
  const [releasedLocalTui, releasedRemoteTui] = await Promise.all([
    waitForLocalNotice(/^Room input: available$/),
    waitForRemoteNotice(/^Room input: available$/),
  ])
  if (realProviderOptions && !companionOnly) {
    const provider = await runRoomRealProvider({
      client, requests, sessionId, sliceId: slice.id, workspace: fixtureWorkspace,
      options: realProviderOptions, waitFor, withTimeout, screenshot,
      officeRuntime: { containerName, docker, sliceScreen, runCommandWithStdin },
      onboardingRuntime: {
        redactError: redactDrillSecrets,
        mailPassword: userSecret, vaultPassphrase,
        rememberSecret: value => { if (!sensitiveValues.includes(value)) sensitiveValues.push(value) },
        trackCredential: id => { if (!drillCredentialIds.includes(id)) drillCredentialIds.push(id) },
        assertNoLeaks: async () => {
          await assertNoPlaintextSecretInTree(await tempRootPromise, sensitiveValues)
          await assertNoPlaintextSecretInTree(evidenceRoot, sensitiveValues)
        },
      },
      checkpoint: (value) => writeFile(path.join(evidenceRoot, "real-provider.json"), `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 }),
      waitForPhysicalEffect: (marker) => waitForBrowserText(marker, 20_000, "provider click did not reach the shared browser"),
      waitForTuis: (pattern) => Promise.all([waitForLocalNotice(pattern), waitForRemoteNotice(pattern)]),
    })
    result = {
      schema: "chariox.room_environment.real_provider.v1", status: "passed", startedAt,
      source: sourceIdentity, sliceRuntime: sliceRuntimeIdentity,
      sessionId, sliceId: slice.id, environmentId: released.environment_id,
      provider, containerLimits: limits,
    }
    return
  }
  if (companionOnly) {
    companionResult = await runCompanionIfConfigured({
      environment: released,
      localNoticeIds: automationNoticeIds(releasedLocalTui),
      remoteNoticeIds: automationNoticeIds(releasedRemoteTui),
      activityController,
    })
    result = {
      schema: "chariox.room_environment.web_companion_focus.v1",
      status: "passed",
      startedAt,
      source: sourceIdentity,
      sliceRuntime: sliceRuntimeIdentity,
      sessionId,
      sliceId: slice.id,
      environmentId: released.environment_id,
      coverage: companionResult.office
        ? "Official provider edits and saves a graphical document, activates the mail tab, uploads and submits once; actual desktop matched in Web and actions observed in local and remote TUIs"
        : `Web display and pointer input${companionResult.keyboard ? " and Unicode typing" : ""}${companionResult.keyboard?.replacement ? ", select-all and native IME replacement" : ""}${companionResult.gestures ? ", physical text-selection drag and two-axis scroll" : ""} with local and remote TUI observation`,
      skipped: ["computer secret", "pointer matrix", "agent keyboard matrix", "cancellation", "clipboard",
        companionResult.keyboard?.replacement ? "remaining Web shortcuts and keyboard layouts" : "Web keyboard shortcuts and IME"],
      companion: companionResult,
      containerLimits: limits,
    }
    return
  }
  const computerSecretResult = await exerciseComputerSecretInput()
  const computerPointer = await exerciseRoomPointer(activityController, activityNotices)
  const computerCancellation = await exerciseRoomComputerCancellation(
    activityController,
    activityNotices,
  )
  await writeFile(path.join(evidenceRoot, "cancellation-checkpoint.json"), JSON.stringify({
    schema: "chariox.room_environment.computer_cancellation_checkpoint.v1",
    completedAt: new Date().toISOString(),
    source: sourceIdentity,
    sliceRuntime: sliceRuntimeIdentity,
    computerCancellation,
    note: "Phase checkpoint only; full drill acceptance also requires result.json and successful cleanup.",
  }, null, 2))
  const computerKeyboard = await exerciseRoomKeyboard(activityController, activityNotices)
  const computerClipboard = await exerciseRoomClipboard(activityController, activityNotices)
  companionResult = await runCompanionIfConfigured({
    environment: computerClipboard.environment,
    localNoticeIds: automationNoticeIds(releasedLocalTui),
    remoteNoticeIds: automationNoticeIds(releasedRemoteTui),
    activityController,
  })
  const observed = unwrap(
    await observerClient.send(requests.getRoomEnvironmentStateRequest(sessionId)),
    "RoomEnvironmentState",
  ).environment
  assert.equal(observed.environment_id, computerClipboard.environment.environment_id)
  assert.equal(observed.session_id, computerClipboard.environment.session_id)
  assert.equal(observed.runtime_generation, computerClipboard.environment.runtime_generation)
  assert.deepEqual(observed.viewport, computerClipboard.environment.viewport)
  assert.deepEqual(observed.input_ownership, computerClipboard.environment.input_ownership)
  assert.ok(observed.event_cursor >= computerClipboard.environment.event_cursor)
  const [finalLocalTui, finalRemoteTui] = await Promise.all([
    localAutomation.send("snapshot"),
    remoteAutomation.send("snapshot"),
  ])
  resources.push(await resourceSnapshot("active"))
  result = {
    schema: "chariox.room_environment.pointer_click_drill.v9",
    status: "passed",
    startedAt,
    source: sourceIdentity,
    sliceRuntime: sliceRuntimeIdentity,
    topology: companionResult
      ? "local kernel and headed Docker worker, production Web client, direct local TUI, and relay-attached remote TUI"
      : "local kernel and headed Docker worker, direct local TUI and relay-attached remote TUI",
    sessionId,
    sliceId: slice.id,
    environmentId: retry.environment.environment_id,
    actionId: click.action_id,
    actorId: desktopOwner.actor_id,
    idempotencyKey,
    physicalEffect: companionResult?.physicalEffect ?? "POINTER_CLICK_COUNT=1",
    computerSecret: computerSecretResult,
    computerPointer,
    computerKeyboard,
    computerCancellation,
    computerClipboard: computerClipboard.summary,
    containerLimits: limits,
    assertions: [
      "public Room request completed one attributed Computer Action",
      "provisioned worker applied the click to the headed desktop",
      "idempotent retry returned the original Action without a second click",
      "TUI activity projected lifecycle, focused tab, takeover, and terminal Action outcome",
      "pointer movement produced no pointer-derived TUI notices",
      "direct local and relay-attached remote TUIs simultaneously projected one authoritative Room",
      "direct local TUI rendered the current lifecycle, tab title, and URL from kernel state",
      "relay-attached remote TUI projected the same Room lifecycle, takeover, Action, and release",
      "relay-attached remote TUI captured the real headed display and verified its PNG digest locally",
      "TUI projected input release and a second protocol client observed the same or newer authoritative state",
      "slice-bound agent created user-entered and generated Computer credentials in the encrypted home vault",
      "home-kernel approvals released each credential only after the password field had focus",
      "worker typed both credentials through the Room Computer action path into the shared headed desktop",
      "Computer secret actions were attributed, argument-free, visible in both TUIs, and absent from clipboard, screenshots, logs, history, and relay output",
      "slice-bound agent moved, single-clicked, right-clicked, double-clicked, dragged, and scrolled the physical X11 desktop through Room authority",
      "pointer drag selected text without moving the Chromium window",
      "horizontal and vertical pointer scrolling both changed the physical nested scroller",
      "Room history retained pointer coordinates, button, count, and scroll steps while both TUIs retained sequence, actor, kind, and outcome",
      "slice-bound agent typed a non-US sample into the physical X11 desktop through Room authority",
      "keyboard focus survived a select-all chord, replacement text, and repeated BackSpace on the physical X11 desktop",
      "Room history and both TUIs retained keyboard attribution, counts, and repeat values without text or key names",
      "local TUI cancellation stopped an in-flight physical keyboard Action exactly once and reset input before later use",
      "remote TUI human takeover cancelled an in-flight agent keyboard Action before granting desktop ownership",
      "cancelled input stopped changing the physical field, remained redacted, and recovered through the same Room authority",
      "slice-bound agent wrote the physical X11 clipboard through the home kernel's Room Action authority",
      "human desktop takeover rejected an agent clipboard mutation without changing the clipboard or Action ledger",
      "human clipboard write crossed the public Room request and encrypted worker path with count-only history",
      "human clipboard read required takeover, observed an exact physical X11 value, and created no Action",
      "local and relay-attached remote TUIs projected both agent and human clipboard Actions without content",
      "clipboard text remained absent from retained Room state, logs, helper output, and evidence",
      ...(companionResult ? [
        "production Web client joined the same authoritative Room as both real TUIs",
        "Web input produced one attributed Computer Action and the physical headed desktop effect",
        "direct local and relay-attached remote TUIs projected the Web-originated Action",
        "Web released desktop ownership after the Action",
      ] : []),
    ],
    activityNotices,
    daemonActivities,
    localTui: {
      sessionId: finalLocalTui.session?.id,
      notices: automationNoticeTexts(finalLocalTui),
      readyNoticeCount: automationNoticeTexts(readyLocalTui).length,
      status: localStatusNotice,
    },
    remoteTui: {
      sessionId: finalRemoteTui.session?.id,
      notices: automationNoticeTexts(finalRemoteTui),
      readyNoticeCount: automationNoticeTexts(readyRemoteTui).length,
      screenshot: remoteScreenshot,
    },
    ...(companionResult ? {
      companion: {
        status: companionResult.status,
        client: companionResult.client,
        actionId: companionResult.actionId,
        actorId: companionResult.actorId,
        screenshot: companionResult.screenshot,
        ...(companionResult.keyboard ? { keyboard: companionResult.keyboard } : {}),
        ...(companionResult.gestures ? { gestures: companionResult.gestures } : {}),
      },
    } : {}),
  }
}

async function exerciseRoomPointer(activityController, activityNotices) {
  await sliceScreen(["open-url", `http://host.docker.internal:${fixture.port}/pointer-matrix`])
  await waitForBrowserText(
    "ROOM_COMPUTER_POINTER_READY",
    30_000,
    "pointer matrix fixture did not load",
  )
  await screenshot("before-pointer-matrix")
  const before = unwrap(
    await client.send(requests.getRoomEnvironmentStateRequest(sessionId)),
    "RoomEnvironmentState",
  ).environment
  const viewportRevision = before.viewport.revision

  const moved = await executeAgentPointerAction({
    args: { action: "move", ...pointerMatrix.move },
    expectedKind: "pointer_move",
    expectedMarker: "POINTER_MOVE_OK",
    markerFailure: "physical pointer move did not enter its target",
    validate: (action, actorId) => assertRoomPointerMoveAction(action, {
      actorId,
      ...pointerMatrix.move,
      viewportRevision,
    }),
    activityController,
    activityNotices,
  })
  const singleClick = await executeAgentPointerAction({
    args: {
      action: "click",
      x: pointerMatrix.singleClick.x,
      y: pointerMatrix.singleClick.y,
      button: pointerMatrix.singleClick.button,
    },
    expectedKind: "pointer_click",
    expectedMarker: "POINTER_SINGLE_CLICK_OK",
    markerFailure: "physical single click did not reach its target exactly once",
    validate: (action, actorId) => assertRoomPointerClickAction(action, {
      actorId,
      ...pointerMatrix.singleClick,
      viewportRevision,
    }),
    activityController,
    activityNotices,
  })
  const rightClick = await executeAgentPointerAction({
    args: {
      action: "click",
      x: pointerMatrix.rightClick.x,
      y: pointerMatrix.rightClick.y,
      button: pointerMatrix.rightClick.button,
    },
    expectedKind: "pointer_click",
    expectedMarker: "POINTER_RIGHT_CLICK_OK",
    markerFailure: "physical right click did not reach its target",
    validate: (action, actorId) => assertRoomPointerClickAction(action, {
      actorId,
      ...pointerMatrix.rightClick,
      viewportRevision,
    }),
    activityController,
    activityNotices,
  })
  const doubleClick = await executeAgentPointerAction({
    args: {
      action: "double_click",
      x: pointerMatrix.doubleClick.x,
      y: pointerMatrix.doubleClick.y,
      button: pointerMatrix.doubleClick.button,
    },
    expectedKind: "pointer_click",
    expectedMarker: "POINTER_DOUBLE_CLICK_OK",
    markerFailure: "physical double click did not reach its target",
    validate: (action, actorId) => assertRoomPointerClickAction(action, {
      actorId,
      ...pointerMatrix.doubleClick,
      viewportRevision,
    }),
    activityController,
    activityNotices,
  })
  const textSelection = await executeAgentPointerAction({
    args: {
      action: "drag",
      x: pointerMatrix.textSelection.fromX,
      y: pointerMatrix.textSelection.fromY,
      to_x: pointerMatrix.textSelection.toX,
      to_y: pointerMatrix.textSelection.toY,
      button: pointerMatrix.textSelection.button,
    },
    expectedKind: "pointer_drag",
    expectedMarker: "TEXT_SELECTION_OK WINDOW_GEOMETRY_STABLE",
    markerFailure: "physical drag did not select text without moving the browser window",
    validate: (action, actorId) => assertRoomPointerDragAction(action, {
      actorId,
      ...pointerMatrix.textSelection,
      viewportRevision,
    }),
    activityController,
    activityNotices,
  })
  const scrolled = await executeAgentPointerAction({
    args: {
      action: "scroll",
      x: pointerMatrix.scroll.x,
      y: pointerMatrix.scroll.y,
      amount: pointerMatrix.scroll.verticalSteps,
      horizontal_steps: pointerMatrix.scroll.horizontalSteps,
    },
    expectedKind: "pointer_scroll",
    expectedMarker: "POINTER_SCROLL_BOTH_AXES_OK",
    markerFailure: "physical pointer scroll did not move both nested-scroller axes",
    validate: (action, actorId) => assertRoomPointerScrollAction(action, {
      actorId,
      ...pointerMatrix.scroll,
      viewportRevision,
    }),
    activityController,
    activityNotices,
  })
  await screenshot("after-pointer-matrix")

  const actions = [moved, singleClick, rightClick, doubleClick, textSelection, scrolled]
  assert.equal(new Set(actions.map((entry) => entry.actionId)).size, actions.length)
  const history = unwrap(
    await client.send(requests.listRoomEnvironmentActionHistoryRequest(sessionId, null, 25)),
    "RoomEnvironmentActionHistoryListed",
  ).page.actions
  for (const pointerAction of actions) {
    pointerAction.validate(
      history.find((candidate) => candidate.action_id === pointerAction.actionId),
      pointerAction.actorId,
    )
  }

  return {
    agentId: secretAgent.id,
    actorId: moved.actorId,
    actionIds: actions.map((entry) => entry.actionId),
    cases: [
      "move",
      "single_click",
      "right_click",
      "double_click",
      "text_selection_drag",
      "horizontal_and_vertical_scroll",
    ],
    physicalEffectsExact: true,
    textSelectionExact: true,
    windowGeometryStable: true,
    localTuiObserved: true,
    remoteTuiObserved: true,
  }
}

async function executeAgentPointerAction({
  args,
  expectedKind,
  expectedMarker,
  markerFailure,
  validate,
  activityController,
  activityNotices,
}) {
  const localBaseline = new Set(automationNoticeIds(await localAutomation.send("snapshot")))
  const remoteBaseline = new Set(automationNoticeIds(await remoteAutomation.send("snapshot")))
  const response = await mcpToolCall(secretProviderRun, "slice_mouse", args)
  assert.equal(response.ok, true, redactDrillSecrets(JSON.stringify(response.raw)))
  assert.equal(response.content?.action_kind, expectedKind)
  assert.equal(response.content?.session_id, sessionId)
  assert.equal(response.content?.agent_id, secretAgent.id)
  try {
    await waitForBrowserText(expectedMarker, 20_000, markerFailure)
  } catch (error) {
    const failureLabel = expectedMarker.toLowerCase().replace(/[^a-z0-9]+/g, "-")
    await screenshot(`pointer-failure-${failureLabel}`).catch(() => {})
    throw error
  }

  const environment = unwrap(
    await client.send(requests.getRoomEnvironmentStateRequest(sessionId)),
    "RoomEnvironmentState",
  ).environment
  const action = environment.actions.find(
    (candidate) => candidate.action_id === response.content?.action_id,
  )
  validate(action, response.content?.actor_id)
  const noticePattern = new RegExp(`^Room action #\\d+: .+ · computer ${expectedKind} · completed$`)
  assert.equal(await activityController.synchronize(), true)
  assert.match(activityNotices.at(-1), noticePattern)
  await Promise.all([
    waitForTuiNoticeAfter(localAutomation, "local", noticePattern, localBaseline, 20_000),
    waitForTuiNoticeAfter(remoteAutomation, "remote", noticePattern, remoteBaseline, 20_000),
  ])
  return {
    actionId: response.content?.action_id,
    actorId: response.content?.actor_id,
    validate,
  }
}

async function exerciseRoomKeyboard(activityController, activityNotices) {
  await sliceScreen(["open-url", `http://host.docker.internal:${fixture.port}/keyboard`])
  await waitForBrowserText("ROOM_COMPUTER_KEYBOARD_READY", 30_000, "keyboard fixture did not load")
  const environment = unwrap(
    await client.send(requests.getRoomEnvironmentStateRequest(sessionId)),
    "RoomEnvironmentState",
  ).environment
  const focused = await executeAgentPointerAction({
    args: {
      action: "click",
      x: pointerMatrix.keyboardFocus.x,
      y: pointerMatrix.keyboardFocus.y,
      button: pointerMatrix.keyboardFocus.button,
    },
    expectedKind: "pointer_click",
    expectedMarker: "ROOM_COMPUTER_KEYBOARD_FOCUS_OK",
    markerFailure: "Room pointer click did not establish the physical keyboard focus",
    validate: (action, actorId) => assertRoomPointerClickAction(action, {
      actorId,
      ...pointerMatrix.keyboardFocus,
      viewportRevision: environment.viewport.revision,
    }),
    activityController,
    activityNotices,
  })
  const typed = await executeAgentKeyboardAction({
    args: { action: "type", text: keyboardText },
    retainedInput: keyboardText,
    expectedKind: "keyboard_text",
    expectedMarker: "ROOM_COMPUTER_KEYBOARD_TEXT_OK",
    markerFailure: "non-US keyboard text did not reach X11",
    validate: (action, actorId) => assertRoomKeyboardTextAction(action, {
      actorId,
      input: keyboardText,
    }),
    activityController,
    activityNotices,
  })
  const selectAll = await executeAgentKeyboardAction({
    args: { action: "key", key: "ctrl+a", repeat: 1 },
    retainedInput: "ctrl+a",
    expectedKind: "keyboard_key",
    expectedMarker: "ROOM_COMPUTER_KEYBOARD_SELECT_ALL_OK",
    markerFailure: "keyboard chord did not select the entire focused input",
    validate: (action, actorId) => assertRoomKeyboardKeyAction(action, {
      actorId,
      key: "ctrl+a",
      repeat: 1,
    }),
    activityController,
    activityNotices,
  })
  const replaced = await executeAgentKeyboardAction({
    args: { action: "type", text: keyboardReplacementText },
    retainedInput: keyboardReplacementText,
    expectedKind: "keyboard_text",
    expectedMarker: "ROOM_COMPUTER_KEYBOARD_SHORTCUT_OK",
    markerFailure: "typing after ctrl+a did not replace the selected text",
    validate: (action, actorId) => assertRoomKeyboardTextAction(action, {
      actorId,
      input: keyboardReplacementText,
    }),
    activityController,
    activityNotices,
  })
  const repeated = await executeAgentKeyboardAction({
    args: { action: "key", key: "BackSpace", repeat: 3 },
    retainedInput: "BackSpace",
    expectedKind: "keyboard_key",
    expectedMarker: "ROOM_COMPUTER_KEYBOARD_REPEAT_OK",
    markerFailure: "repeated BackSpace did not preserve focus or execute exactly three times",
    validate: (action, actorId) => assertRoomKeyboardKeyAction(action, {
      actorId,
      key: "BackSpace",
      repeat: 3,
    }),
    activityController,
    activityNotices,
  })
  const history = unwrap(
    await client.send(requests.listRoomEnvironmentActionHistoryRequest(sessionId, null, 25)),
    "RoomEnvironmentActionHistoryListed",
  ).page.actions
  for (const keyboardAction of [typed, selectAll, replaced, repeated]) {
    keyboardAction.validate(
      history.find((candidate) => candidate.action_id === keyboardAction.actionId),
      keyboardAction.actorId,
    )
  }
  const tempRoot = await tempRootPromise
  const keyboardValues = [keyboardText, keyboardReplacementText, keyboardAfterRepeat]
  await assertNoPlaintextSecretInTree(tempRoot, keyboardValues)
  await assertNoPlaintextSecretInTree(evidenceRoot, keyboardValues)

  return {
    agentId: secretAgent.id,
    actorId: typed.actorId,
    focusActionId: focused.actionId,
    actionIds: [typed, selectAll, replaced, repeated].map((entry) => entry.actionId),
    cases: [
      textCaseSummary("non-us-text", keyboardText),
      textCaseSummary("shortcut-replacement", keyboardReplacementText),
    ],
    physicalInputExact: true,
    shortcutSelectedAll: true,
    repeatExact: true,
    focusPreserved: true,
    localTuiObserved: true,
    remoteTuiObserved: true,
    retainedContentRedacted: true,
  }
}

async function exerciseRoomComputerCancellation(activityController, activityNotices) {
  await sliceScreen(["open-url", `http://host.docker.internal:${fixture.port}/cancellation`])
  await waitForBrowserText(
    "ROOM_COMPUTER_CANCELLATION_READY",
    30_000,
    "Computer cancellation fixture did not load",
  )
  const environment = unwrap(
    await client.send(requests.getRoomEnvironmentStateRequest(sessionId)),
    "RoomEnvironmentState",
  ).environment
  const focused = await executeAgentPointerAction({
    args: {
      action: "click",
      x: pointerMatrix.keyboardFocus.x,
      y: pointerMatrix.keyboardFocus.y,
      button: pointerMatrix.keyboardFocus.button,
    },
    expectedKind: "pointer_click",
    expectedMarker: "ROOM_COMPUTER_CANCELLATION_FOCUS_OK",
    markerFailure: "Room pointer click did not establish cancellation fixture focus",
    validate: (action, actorId) => assertRoomPointerClickAction(action, {
      actorId,
      ...pointerMatrix.keyboardFocus,
      viewportRevision: environment.viewport.revision,
    }),
    activityController,
    activityNotices,
  })

  const explicitTakeover = unwrap(
    await client.send(requests.requestRoomEnvironmentInputTakeoverRequest(
      sessionId,
      { kind: "desktop" },
    )),
    "RoomEnvironmentTakeoverUpdated",
  )
  assert.equal(explicitTakeover.outcome.state, "granted")
  assert.ok(explicitTakeover.environment.input_ownership.some((entry) => (
    entry.target.kind === "desktop" && entry.actor_id === "user:local"
  )))
  const explicitLocal = await exerciseCancellableKeyboardInput({
    input: cancellationText,
    actorId: "user:local",
    start: (baseline) => client.send(requests.submitRoomEnvironmentActionRequest(
      sessionId,
      baseline.runtime_generation,
      baseline.viewport.revision,
      `${runId}-human-keyboard-cancellation`,
      { kind: "keyboard_text", text: cancellationText },
    )),
    cancel: async (actionId, { localNoticeBaseline }) => {
      await localAutomation.send(
        "submit_prompt",
        { prompt: `/room cancel ${actionId}` },
        20_000,
      )
      await waitForTuiNoticeAfter(
        localAutomation,
        "local",
        new RegExp(`^Room action ${actionId} cancellation requested\\n`),
        localNoticeBaseline,
        20_000,
      )
    },
    assertSettlement: assertCancelledHumanActionSettlement,
    activityController,
    activityNotices,
    label: "local TUI explicit human cancellation",
  })
  const explicitRelease = unwrap(
    await client.send(requests.releaseRoomEnvironmentInputRequest(
      sessionId,
      { kind: "desktop" },
    )),
    "RoomEnvironmentInputReleased",
  ).environment
  assert.equal(
    explicitRelease.input_ownership.some((entry) => entry.target.kind === "desktop"),
    false,
    "explicit cancellation owner must release desktop before agent recovery",
  )
  await recoverCancellationFixture(activityController, activityNotices)
  await selectCancellationFixtureText(activityController, activityNotices)

  const takeoverLocalBaseline = new Set(automationNoticeIds(await localAutomation.send("snapshot")))
  const takeoverRemoteBaseline = new Set(automationNoticeIds(await remoteAutomation.send("snapshot")))
  const takeoverRemote = await exerciseCancellableKeyboardInput({
    input: takeoverCancellationText,
    actorId: `agent:${secretAgent.id}`,
    start: () => mcpToolCall(secretProviderRun, "slice_keyboard", {
      action: "type",
      text: takeoverCancellationText,
    }),
    cancel: async (actionId, { remoteNoticeBaseline }) => {
      await remoteAutomation.send(
        "submit_prompt",
        { prompt: "/room takeover desktop" },
        20_000,
      )
      await waitForTuiNoticeAfter(
        remoteAutomation,
        "remote",
        new RegExp(`^Room takeover requires cancellation: ${actionId}\\n`),
        remoteNoticeBaseline,
        20_000,
      )
    },
    assertSettlement: assertCancelledAgentToolSettlement,
    activityController,
    activityNotices,
    label: "remote TUI human takeover cancellation",
  })
  const takenOver = await waitFor(async () => {
    const current = unwrap(
      await client.send(requests.getRoomEnvironmentStateRequest(sessionId)),
      "RoomEnvironmentState",
    ).environment
    return current.input_ownership.some((entry) => (
      entry.target.kind === "desktop" && entry.actor_id === "user:local"
    )) ? current : false
  }, 20_000, "human takeover was not granted after physical input stopped")
  assertHumanDesktopTakeoverCompleted(takenOver, {
    actionId: takeoverRemote.actionId,
    humanActorId: "user:local",
  })
  await activityController.synchronize()
  await Promise.all([
    waitForTuiNoticeAfter(
      localAutomation,
      "local",
      /^Room input: Local user controls desktop$/,
      takeoverLocalBaseline,
      20_000,
    ),
    waitForTuiNoticeAfter(
      remoteAutomation,
      "remote",
      /^Room input: Local user controls desktop$/,
      takeoverRemoteBaseline,
      20_000,
    ),
  ])
  const releaseLocalBaseline = new Set(automationNoticeIds(await localAutomation.send("snapshot")))
  const releaseRemoteBaseline = new Set(automationNoticeIds(await remoteAutomation.send("snapshot")))
  const released = await remoteAutomation.send(
    "submit_prompt",
    { prompt: "/room release desktop" },
    20_000,
  )
  assert.ok(
    automationNoticeTexts(released).some((notice) => notice.startsWith("Room input released\n")),
    "remote TUI did not render desktop release",
  )
  await Promise.all([
    waitForTuiNoticeAfter(
      localAutomation,
      "local",
      /^Room input: available$/,
      releaseLocalBaseline,
      20_000,
    ),
    waitForTuiNoticeAfter(
      remoteAutomation,
      "remote",
      /^Room input: available$/,
      releaseRemoteBaseline,
      20_000,
    ),
  ])
  await recoverCancellationFixture(activityController, activityNotices)

  const history = unwrap(
    await client.send(requests.listRoomEnvironmentActionHistoryRequest(sessionId, null, 50)),
    "RoomEnvironmentActionHistoryListed",
  ).page.actions
  for (const cancellation of [explicitLocal, takeoverRemote]) {
    assert.equal(
      history.filter((candidate) => candidate.action_id === cancellation.actionId).length,
      1,
      `Room history must retain exactly one terminal ${cancellation.actionId}`,
    )
    assertRoomComputerActionCancelled(
      history.find((candidate) => candidate.action_id === cancellation.actionId),
      {
        actionId: cancellation.actionId,
        actorId: cancellation.actorId,
        kind: "keyboard_text",
        focusedTabId: cancellation.focusedTabId,
      },
    )
  }
  const tempRoot = await tempRootPromise
  await assertNoPlaintextSecretInTree(tempRoot, [cancellationText, takeoverCancellationText])
  await assertNoPlaintextSecretInTree(evidenceRoot, [cancellationText, takeoverCancellationText])

  return {
    agentId: secretAgent.id,
    actorId: explicitLocal.actorId,
    focusActionId: focused.actionId,
    explicitCancellationActionId: explicitLocal.actionId,
    takeoverCancellationActionId: takeoverRemote.actionId,
    cases: ["local_tui_explicit_cancel", "remote_tui_human_takeover"],
    physicalInputStartedBeforeCancellation: true,
    physicalInputStoppedAfterCancellation: true,
    terminalExactlyOnce: true,
    inputResetAndRecovered: true,
    takeoverWaitedForCancellation: true,
    localTuiObserved: true,
    remoteTuiObserved: true,
    retainedContentRedacted: true,
    cancellationLatencyMs: {
      explicit: explicitLocal.cancellationLatencyMs,
      takeover: takeoverRemote.cancellationLatencyMs,
    },
    cancellationTimings: {
      explicit: explicitLocal.cancellationTimings,
      takeover: takeoverRemote.cancellationTimings,
    },
    tuiObservationLatencyMs: {
      explicit: explicitLocal.tuiObservationLatencyMs,
      takeover: takeoverRemote.tuiObservationLatencyMs,
    },
    characterCounts: {
      explicitBeforeCancellation: explicitLocal.countBeforeCancellation,
      explicitAfterCancellation: explicitLocal.countAfterCancellation,
      takeoverBeforeCancellation: takeoverRemote.countBeforeCancellation,
      takeoverAfterCancellation: takeoverRemote.countAfterCancellation,
    },
  }
}

async function exerciseCancellableKeyboardInput({
  input,
  actorId,
  start,
  cancel,
  assertSettlement,
  activityController,
  activityNotices,
  label,
}) {
  const baseline = unwrap(
    await client.send(requests.getRoomEnvironmentStateRequest(sessionId)),
    "RoomEnvironmentState",
  ).environment
  const baselineSequence = Math.max(0, ...baseline.actions.map((action) => action.sequence))
  const baselineEventCursor = baseline.event_cursor
  assert.ok(baseline.focused_tab_id, `${label} requires a focused browser tab`)
  const localNoticeBaseline = new Set(automationNoticeIds(await localAutomation.send("snapshot")))
  const remoteNoticeBaseline = new Set(automationNoticeIds(await remoteAutomation.send("snapshot")))
  const pending = Promise.resolve().then(() => start(baseline)).then(
    (result) => ({ result, error: null }),
    (error) => ({ result: null, error }),
  )
  const started = await waitFor(async () => {
    const current = unwrap(
      await client.send(requests.getRoomEnvironmentStateRequest(sessionId)),
      "RoomEnvironmentState",
    ).environment
    const action = current.actions.find((candidate) => (
      candidate.sequence > baselineSequence
        && candidate.actor_id === actorId
        && candidate.kind === "keyboard_text"
    ))
    if (!action) return false
    const count = await cancellationFixtureCharacterCount()
    return count > 0 || action.state !== "running" ? { action, count } : false
  }, 4_000, `${label} did not begin physical input`)
  assertRoomComputerActionRunning(started.action, {
    actionId: started.action.action_id,
    actorId,
    kind: "keyboard_text",
    focusedTabId: baseline.focused_tab_id,
  })
  assert.ok(started.count > 0, `${label} did not type before cancellation`)
  assert.ok(started.count < input.length, `${label} completed before cancellation`)

  const cancelStartedAt = Date.now()
  const cancellationObservation = Promise.resolve()
    .then(() => cancel(started.action.action_id, { localNoticeBaseline, remoteNoticeBaseline }))
    .then(
      (result) => ({ result, error: null, observedAtMs: Date.now() }),
      (error) => ({ result: null, error, observedAtMs: Date.now() }),
    )
  const requestObserved = await waitFor(async () => {
    const replay = unwrap(
      await client.send(requests.getRoomEnvironmentEventsRequest(sessionId, baselineEventCursor)),
      "RoomEnvironmentEvents",
    ).replay
    const events = replay.Events?.events ?? []
    return events.some((event) => (
      event.kind?.ActionChanged?.action_id === started.action.action_id
        && event.kind.ActionChanged.cancellation_requested === true
    )) ? { observedAtMs: Date.now() } : false
  }, 20_000, `${label} cancellation request did not reach the Room ledger`)

  const terminal = await waitFor(async () => {
    const current = unwrap(
      await client.send(requests.getRoomEnvironmentStateRequest(sessionId)),
      "RoomEnvironmentState",
    ).environment
    const action = current.actions.find(
      (candidate) => candidate.action_id === started.action.action_id,
    )
    return action?.state === "cancelled" ? action : false
  }, 20_000, `${label} did not reach one terminal cancelled state`)
  assertRoomComputerActionCancelled(terminal, {
    actionId: started.action.action_id,
    actorId,
    kind: "keyboard_text",
    focusedTabId: baseline.focused_tab_id,
  })
  const cancellationTimings = roomComputerCancellationTimings(terminal, {
    initiatedAtMs: cancelStartedAt,
    requestObservedAtMs: requestObserved.observedAtMs,
  })
  assert.ok(
    cancellationTimings.endToEndLatencyMs < 2_000,
    `${label} end-to-end cancellation took ${cancellationTimings.endToEndLatencyMs}ms (request event observed after ${cancellationTimings.requestObservationLatencyMs}ms)`,
  )
  const [cancellationResult, settlement] = await Promise.all([cancellationObservation, pending])
  if (cancellationResult.error) throw cancellationResult.error
  const tuiObservationLatencyMs = cancellationResult.observedAtMs - cancelStartedAt
  assertSettlement(settlement, input, label)
  const stoppedCount = await cancellationFixtureCharacterCount()
  await sleep(750)
  assert.equal(
    await cancellationFixtureCharacterCount(),
    stoppedCount,
    `${label} continued physical typing after terminal cancellation`,
  )
  assert.equal(await activityController.synchronize(), true)
  const noticePattern = /^Room action #\d+: .+ · computer keyboard_text · cancelled \(requested\)$/
  assert.ok(activityNotices.some((notice) => noticePattern.test(notice)))
  await Promise.all([
    waitForTuiNoticeAfter(localAutomation, "local", noticePattern, localNoticeBaseline, 20_000),
    waitForTuiNoticeAfter(remoteAutomation, "remote", noticePattern, remoteNoticeBaseline, 20_000),
  ])
  return {
    actionId: started.action.action_id,
    actorId,
    focusedTabId: baseline.focused_tab_id,
    countBeforeCancellation: started.count,
    countAfterCancellation: stoppedCount,
    cancellationLatencyMs: cancellationTimings.endToEndLatencyMs,
    cancellationTimings,
    tuiObservationLatencyMs,
  }
}

function assertCancelledHumanActionSettlement(settlement, input, label) {
  assert.equal(settlement.result, null, `${label} must not return a successful submission`)
  assert.ok(settlement.error, `${label} must return an actionable cancellation error`)
  const errorText = String(settlement.error?.stack ?? settlement.error)
  assert.match(errorText, /cancel/i, `${label} cancellation error`)
  assertRetainedTextIsRedacted(
    { error: errorText },
    input,
    `${label} local response retained keyboard input`,
  )
}

function assertCancelledAgentToolSettlement(settlement, input, label) {
  if (settlement.error) throw settlement.error
  const toolResult = settlement.result
  assert.equal(toolResult.ok, false, `${label} must not report tool success`)
  assert.match(
    JSON.stringify(toolResult.raw),
    /cancel/i,
    `${label} must return an actionable cancellation result`,
  )
  assertRetainedTextIsRedacted(
    toolResult.raw,
    input,
    `${label} runtime MCP result retained keyboard input`,
  )
}

async function selectCancellationFixtureText(activityController, activityNotices) {
  return await executeAgentKeyboardAction({
    args: { action: "key", key: "ctrl+a", repeat: 1 },
    retainedInput: "ctrl+a",
    expectedKind: "keyboard_key",
    expectedMarker: "ROOM_COMPUTER_CANCELLATION_SELECTED",
    markerFailure: "cancellation fixture text was not selected after input reset",
    validate: (action, actorId) => assertRoomKeyboardKeyAction(action, {
      actorId,
      key: "ctrl+a",
      repeat: 1,
    }),
    activityController,
    activityNotices,
  })
}

async function recoverCancellationFixture(activityController, activityNotices) {
  await selectCancellationFixtureText(activityController, activityNotices)
  return await executeAgentKeyboardAction({
    args: { action: "type", text: cancellationRecoveryText },
    retainedInput: cancellationRecoveryText,
    expectedKind: "keyboard_text",
    expectedMarker: "ROOM_COMPUTER_CANCELLATION_RECOVERED",
    markerFailure: "physical keyboard input did not recover after cancellation reset",
    validate: (action, actorId) => assertRoomKeyboardTextAction(action, {
      actorId,
      input: cancellationRecoveryText,
    }),
    activityController,
    activityNotices,
  })
}

async function cancellationFixtureCharacterCount() {
  const text = await sliceScreen(["browser-text"])
  const count = Number(text.match(/ROOM_COMPUTER_CANCELLATION_COUNT=(\d+)/)?.[1])
  return Number.isInteger(count) ? count : 0
}

async function executeAgentKeyboardAction({
  args,
  retainedInput,
  expectedKind,
  expectedMarker,
  markerFailure,
  validate,
  activityController,
  activityNotices,
}) {
  const localBaseline = new Set(automationNoticeIds(await localAutomation.send("snapshot")))
  const remoteBaseline = new Set(automationNoticeIds(await remoteAutomation.send("snapshot")))
  const response = await mcpToolCall(secretProviderRun, "slice_keyboard", args)
  assert.equal(response.ok, true, redactDrillSecrets(JSON.stringify(response.raw)))
  assert.equal(response.content?.action_kind, expectedKind)
  assert.equal(response.content?.session_id, sessionId)
  assert.equal(response.content?.agent_id, secretAgent.id)
  assertRetainedTextIsRedacted(
    response.raw,
    retainedInput,
    "runtime MCP response retained keyboard input",
  )
  try {
    await waitForBrowserText(expectedMarker, 20_000, markerFailure)
  } catch (error) {
    const failureLabel = expectedMarker.toLowerCase().replace(/[^a-z0-9]+/g, "-")
    await screenshot(`keyboard-failure-${failureLabel}`).catch(() => {})
    throw error
  }

  const environment = unwrap(
    await client.send(requests.getRoomEnvironmentStateRequest(sessionId)),
    "RoomEnvironmentState",
  ).environment
  const action = environment.actions.find(
    (candidate) => candidate.action_id === response.content?.action_id,
  )
  validate(action, response.content?.actor_id)
  const noticePattern = new RegExp(`^Room action #\\d+: .+ · computer ${expectedKind} · completed$`)
  assert.equal(await activityController.synchronize(), true)
  assert.match(activityNotices.at(-1), noticePattern)
  await Promise.all([
    waitForTuiNoticeAfter(localAutomation, "local", noticePattern, localBaseline, 20_000),
    waitForTuiNoticeAfter(remoteAutomation, "remote", noticePattern, remoteBaseline, 20_000),
  ])
  return {
    actionId: response.content?.action_id,
    actorId: response.content?.actor_id,
    validate,
  }
}

async function exerciseRoomClipboard(activityController, activityNotices) {
  const before = unwrap(
    await client.send(requests.getRoomEnvironmentStateRequest(sessionId)),
    "RoomEnvironmentState",
  ).environment
  await assert.rejects(
    client.send(requests.readRoomEnvironmentClipboardRequest(
      sessionId,
      before.runtime_generation,
    )),
    /environment_input_takeover_required/,
    "human clipboard read must require desktop takeover",
  )

  const agentLocalBaseline = new Set(automationNoticeIds(await localAutomation.send("snapshot")))
  const agentRemoteBaseline = new Set(automationNoticeIds(await remoteAutomation.send("snapshot")))
  const agentWrite = await mcpToolCall(secretProviderRun, "slice_clipboard_write", {
    text: agentClipboardText,
  })
  assert.equal(agentWrite.ok, true, redactDrillSecrets(JSON.stringify(agentWrite.raw)))
  assert.equal(agentWrite.content?.action_kind, "clipboard_write")
  assert.equal(agentWrite.content?.session_id, sessionId)
  assert.equal(agentWrite.content?.agent_id, secretAgent.id)
  assertRetainedClipboardEvidenceIsRedacted(agentWrite.raw, agentClipboardText)
  assert.equal(await readPhysicalClipboard(), agentClipboardText)

  let environment = unwrap(
    await client.send(requests.getRoomEnvironmentStateRequest(sessionId)),
    "RoomEnvironmentState",
  ).environment
  const agentAction = environment.actions.find(
    (action) => action.action_id === agentWrite.content?.action_id,
  )
  assertRoomClipboardAction(agentAction, {
    actorId: agentWrite.content?.actor_id,
    clipboardText: agentClipboardText,
  })
  const noticePattern = /^Room action #\d+: .+ · computer clipboard_write · completed$/
  assert.equal(await activityController.synchronize(), true)
  assert.match(activityNotices.at(-1), noticePattern)
  await Promise.all([
    waitForTuiNoticeAfter(
      localAutomation,
      "local",
      noticePattern,
      agentLocalBaseline,
      20_000,
    ),
    waitForTuiNoticeAfter(
      remoteAutomation,
      "remote",
      noticePattern,
      agentRemoteBaseline,
      20_000,
    ),
  ])

  const takeover = unwrap(
    await client.send(requests.requestRoomEnvironmentInputTakeoverRequest(
      sessionId,
      { kind: "desktop" },
    )),
    "RoomEnvironmentTakeoverUpdated",
  )
  assert.equal(takeover.outcome.state, "granted")
  const actionCountBeforeBlockedWrite = takeover.environment.actions.length
  const blockedWrite = await mcpToolCall(secretProviderRun, "slice_clipboard_write", {
    text: blockedAgentClipboardText,
  })
  assert.equal(blockedWrite.ok, false, "agent clipboard write must fail during human takeover")
  assert.match(
    JSON.stringify(blockedWrite.raw),
    /belongs to .?user:local|input takeover|takeover required/i,
  )
  assertRetainedClipboardEvidenceIsRedacted(blockedWrite.raw, blockedAgentClipboardText)
  assert.equal(await readPhysicalClipboard(), agentClipboardText)
  environment = unwrap(
    await client.send(requests.getRoomEnvironmentStateRequest(sessionId)),
    "RoomEnvironmentState",
  ).environment
  assert.equal(
    environment.actions.length,
    actionCountBeforeBlockedWrite,
    "rejected agent clipboard write must not enter the Action ledger",
  )

  const humanLocalBaseline = new Set(automationNoticeIds(await localAutomation.send("snapshot")))
  const humanRemoteBaseline = new Set(automationNoticeIds(await remoteAutomation.send("snapshot")))
  const humanWrite = unwrap(
    await client.send(requests.submitRoomEnvironmentActionRequest(
      sessionId,
      takeover.environment.runtime_generation,
      takeover.environment.viewport.revision,
      `${runId}-human-clipboard`,
      { kind: "clipboard_write", text: humanClipboardText },
    )),
    "RoomEnvironmentActionSubmitted",
  )
  const humanAction = humanWrite.environment.actions.find(
    (action) => action.action_id === humanWrite.action_id,
  )
  assertRoomClipboardAction(humanAction, {
    actorId: "user:local",
    clipboardText: humanClipboardText,
  })
  assert.equal(await readPhysicalClipboard(), humanClipboardText)
  assert.equal(await activityController.synchronize(), true)
  assert.match(activityNotices.at(-1), noticePattern)
  await Promise.all([
    waitForTuiNoticeAfter(
      localAutomation,
      "local",
      noticePattern,
      humanLocalBaseline,
      20_000,
    ),
    waitForTuiNoticeAfter(
      remoteAutomation,
      "remote",
      noticePattern,
      humanRemoteBaseline,
      20_000,
    ),
  ])

  const actionCountBeforeRead = humanWrite.environment.actions.length
  await writePhysicalClipboard(physicalClipboardText)
  const clipboardRead = unwrap(
    await client.send(requests.readRoomEnvironmentClipboardRequest(
      sessionId,
      humanWrite.environment.runtime_generation,
    )),
    "RoomEnvironmentClipboardRead",
  )
  assert.equal(clipboardRead.content, physicalClipboardText)
  assertRetainedClipboardEvidenceIsRedacted(
    { content: "[consumed without retention]" },
    physicalClipboardText,
  )
  environment = unwrap(
    await client.send(requests.getRoomEnvironmentStateRequest(sessionId)),
    "RoomEnvironmentState",
  ).environment
  assert.equal(
    environment.actions.length,
    actionCountBeforeRead,
    "human clipboard read must not create an Action",
  )

  const released = unwrap(
    await client.send(requests.releaseRoomEnvironmentInputRequest(
      sessionId,
      { kind: "desktop" },
    )),
    "RoomEnvironmentInputReleased",
  ).environment
  const history = unwrap(
    await client.send(requests.listRoomEnvironmentActionHistoryRequest(sessionId, null, 25)),
    "RoomEnvironmentActionHistoryListed",
  ).page.actions
  assertRoomClipboardAction(
    history.find((action) => action.action_id === agentWrite.content?.action_id),
    { actorId: agentWrite.content?.actor_id, clipboardText: agentClipboardText },
  )
  assertRoomClipboardAction(
    history.find((action) => action.action_id === humanWrite.action_id),
    { actorId: "user:local", clipboardText: humanClipboardText },
  )
  for (const value of clipboardValues) {
    assertRetainedClipboardEvidenceIsRedacted(released, value)
  }
  const tempRoot = await tempRootPromise
  await assertNoPlaintextSecretInTree(tempRoot, clipboardValues)
  await assertNoPlaintextSecretInTree(evidenceRoot, clipboardValues)

  return {
    environment: released,
    summary: {
      agentId: secretAgent.id,
      agentActorId: agentWrite.content?.actor_id,
      agentActionId: agentWrite.content?.action_id,
      humanActorId: "user:local",
      humanActionId: humanWrite.action_id,
      cases: [
        clipboardCaseSummary("agent-write", agentClipboardText),
        clipboardCaseSummary("blocked-agent-write", blockedAgentClipboardText),
        clipboardCaseSummary("human-write", humanClipboardText),
        clipboardCaseSummary("physical-write-human-read", physicalClipboardText),
      ],
      takeoverRejectedAgentMutation: true,
      physicalClipboardExact: true,
      humanReadRequiredTakeover: true,
      humanReadCreatedAction: false,
      localTuiObserved: true,
      remoteTuiObserved: true,
      retainedContentRedacted: true,
    },
  }
}

async function exerciseComputerSecretInput() {
  const { agent, providerRun } = await launchComputerSecretAgent()
  secretAgent = agent
  secretProviderRun = providerRun

  const requestedCredential = mcpToolCall(providerRun, "request_credential_secret", {
    credential: {
      id: userCredentialId,
      description: "Room Computer user-entered credential drill",
      allowed_hosts: [],
      allowed_uses: ["computer"],
      injection: { kind: "computer" },
    },
    prompt: {
      title: "Room Computer credential drill",
      message: "Enter the credential for the masked Computer input drill.",
      placeholder: "Password",
      min_length: 8,
      max_length: 128,
      timeout_sec: 60,
    },
    overwrite: false,
  })
  const unlockInteraction = await waitForRuntimeInteraction(
    (interaction) => String(interaction.title ?? "").includes("Unlock Chariox Vault"),
    "vault unlock interaction",
  )
  await client.send(requests.respondToInteractionRequest(
    sessionId,
    unlockInteraction.id,
    "unlock_default_ttl",
    vaultPassphrase,
  ))
  const secretInteraction = await waitForRuntimeInteraction(
    (interaction) => interaction.title === "Room Computer credential drill",
    "user credential interaction",
  )
  assert.equal(secretInteraction.custom_choice?.input_kind, "secret")
  await client.send(requests.respondToInteractionRequest(
    sessionId,
    secretInteraction.id,
    secretInteraction.custom_choice.id,
    userSecret,
  ))
  const requested = await requestedCredential
  assert.equal(requested.ok, true, JSON.stringify(requested))
  assert.equal(requested.content?.credential?.id ?? requested.content?.credential_id, userCredentialId)
  assertNoSecretProperties(requested.raw, "user credential creation")

  const generated = await mcpToolCall(providerRun, "create_generated_credential", {
    credential: {
      id: generatedCredentialId,
      description: "Room Computer generated credential drill",
      allowed_hosts: [],
      allowed_uses: ["computer"],
      injection: { kind: "computer" },
    },
    generator: {
      kind: "password",
      length: generatedSecretLength,
      symbols: false,
      avoid_ambiguous: true,
    },
    overwrite: false,
  })
  assert.equal(generated.ok, true, JSON.stringify(generated))
  assert.equal(generated.content?.credential?.id ?? generated.content?.credential_id, generatedCredentialId)
  assertNoSecretProperties(generated.raw, "generated credential creation")

  await sliceScreen(["open-url", `http://host.docker.internal:${fixture.port}/secret`])
  await waitForBrowserText("ROOM_COMPUTER_SECRET_READY", 30_000, "secret fixture did not load")
  const clipboardSentinel = `room-clipboard-${runId}`
  await sliceScreen(["clipboard-set", clipboardSentinel])

  const localNoticeBaseline = new Set(automationNoticeIds(await localAutomation.send("snapshot")))
  const remoteNoticeBaseline = new Set(automationNoticeIds(await remoteAutomation.send("snapshot")))
  await sliceScreen(["browser-click", "#user-secret"])
  const userPaste = await pasteComputerCredential(providerRun, userCredentialId)
  await waitForBrowserText("USER_SECRET_OK", 20_000, "user-entered Computer secret did not reach the field")

  await sliceScreen(["browser-click", "#generated-secret"])
  const generatedPaste = await pasteComputerCredential(providerRun, generatedCredentialId)
  await waitForBrowserText("GENERATED_SECRET_OK", 20_000, "generated Computer secret did not reach the field")

  const browserText = await sliceScreen(["browser-text"])
  assert.equal(browserText.includes(userSecret), false, "browser projection leaked the user secret")
  assert.equal(browserText.includes(vaultPassphrase), false, "browser projection leaked the vault passphrase")
  assert.equal(await sliceScreen(["clipboard-get"]), clipboardSentinel)
  await screenshot("after-computer-secret")

  const environment = unwrap(
    await client.send(requests.getRoomEnvironmentStateRequest(sessionId)),
    "RoomEnvironmentState",
  ).environment
  const secretActions = environment.actions.filter((action) => action.kind === "secret_input")
  assert.equal(secretActions.length, 2)
  assert.deepEqual(
    secretActions.map((action) => action.action_id),
    [userPaste.content.action_id, generatedPaste.content.action_id],
  )
  assert.ok(secretActions.every((action) => action.arguments == null))
  assert.ok(secretActions.every((action) => action.state === "completed"))
  assert.ok(secretActions.every((action) => action.actor_id === userPaste.content.actor_id))

  const noticePattern = /^Room action #\d+: .+ · computer secret_input · completed$/
  const [localNotice, remoteNotice] = await Promise.all([
    waitForTuiNoticeAfter(localAutomation, "local", noticePattern, localNoticeBaseline, 20_000),
    waitForTuiNoticeAfter(remoteAutomation, "remote", noticePattern, remoteNoticeBaseline, 20_000),
  ])
  assert.ok(automationNoticeTexts(localNotice).some((notice) => noticePattern.test(notice)))
  assert.ok(automationNoticeTexts(remoteNotice).some((notice) => noticePattern.test(notice)))

  const tempRoot = await tempRootPromise
  await assertNoPlaintextSecretInTree(tempRoot, [userSecret, vaultPassphrase])
  await assertNoPlaintextSecretInTree(evidenceRoot, [userSecret, vaultPassphrase])

  await client.send(requests.deleteCredentialSecretRequest(userCredentialId))
  await client.send(requests.deleteCredentialSecretRequest(generatedCredentialId))

  return {
    agentId: agent.id,
    providerRunId: providerRun.id,
    credentialKinds: ["user-entered", "generated"],
    userActionId: userPaste.content.action_id,
    generatedActionId: generatedPaste.content.action_id,
    actorId: userPaste.content.actor_id,
    clipboardPreserved: true,
    browserProjectionRedacted: true,
    historyArgumentsRedacted: true,
    localTuiObserved: true,
    remoteTuiObserved: true,
    screenshot: path.join(evidenceRoot, "after-computer-secret.png"),
  }
}

async function launchComputerSecretAgent() {
  const spawned = unwrap(
    await client.send(requests.spawnAgentRequest(
      sessionId,
      "dev-stub",
      "computer-secret-agent",
      "native-tui-idle",
      repoRoot,
      "low",
      "build",
      "yolo",
      undefined,
      undefined,
      slice.id,
    )),
    "AgentSpawned",
  ).agent
  const attachment = unwrap(
    await client.send(requests.attachToSessionRequest(sessionId, `${runId}-secret-driver`)),
    "SessionAttached",
  ).attachment
  await client.send(requests.submitPromptRequest(
    sessionId,
    attachment.id,
    spawned.id,
    "Initialize the slice-backed runtime for the Computer secret drill.",
    [],
  ))
  const workerProviderRunId = await waitFor(async () => {
    const state = unwrapOneOf(
      await client.send(requests.getSessionStateRequest(sessionId)),
      "SessionStateLoaded",
      "SessionState",
    )
    const agent = state.session.agents.find((candidate) => candidate.id === spawned.id)
    return agent?.remote_execution?.active_worker_provider_run_id ?? false
  }, 120_000, "slice agent did not acquire a worker provider run")
  const workerRelayUrl = slice.relay_endpoint?.url
  assert.ok(workerRelayUrl, "slice did not publish its private relay endpoint")
  const workerRelayToken = `slice-local-${slice.owner_kernel_id}-${slice.id}`
  const { LocalIpcClient } = await import(
    pathToFileURL(path.join(kernelClientRoot, "dist", "ipc.js")).href
  )
  const ready = await waitFor(async () => {
    const candidate = interruption.guardClient(new LocalIpcClient(workerRelayUrl, {
      relayAuthToken: workerRelayToken,
      targetDaemonAlias: slice.worker_kernel_ref,
    }))
    try {
      const current = unwrap(
        await candidate.send(requests.getProviderRunRequest(workerProviderRunId)),
        "ProviderRun",
      ).provider_run
      if (["ended", "failed", "error"].includes(String(current.state ?? "").toLowerCase())) {
        throw new Error(`slice provider run ended before MCP became ready: ${JSON.stringify(current)}`)
      }
      if (!current.runtime_mcp_server_url || !current.runtime_mcp_auth_token) {
        await candidate.close().catch(() => undefined)
        return false
      }
      return { client: candidate, providerRun: current }
    } catch (error) {
      await candidate.close().catch(() => undefined)
      throw error
    }
  }, 120_000, "slice provider runtime MCP did not become ready")
  workerClient = ready.client
  return { agent: spawned, providerRun: ready.providerRun }
}

async function mcpToolCall(providerRun, name, argumentsValue) {
  const payload = JSON.stringify({
    url: providerRun.runtime_mcp_server_url,
    token: providerRun.runtime_mcp_auth_token,
    body: {
      jsonrpc: "2.0",
      id: `${name}-${Date.now()}`,
      method: "tools/call",
      params: { name, arguments: argumentsValue },
    },
  })
  const helper = [
    "let input='';",
    "process.stdin.setEncoding('utf8');",
    "process.stdin.on('data',chunk=>{input+=chunk});",
    "process.stdin.on('end',async()=>{",
    "const request=JSON.parse(input);",
    "const response=await fetch(request.url,{method:'POST',headers:{authorization:'Bearer '+request.token,'content-type':'application/json'},body:JSON.stringify(request.body)});",
    "const text=await response.text();",
    "process.stdout.write(JSON.stringify({status:response.status,ok:response.ok,body:text}));",
    "});",
  ].join("")
  const response = await runCommandWithStdin(
    "docker",
    ["exec", "-i", "-u", "slice", containerName, "node", "-e", helper],
    payload,
    90_000,
  )
  assert.equal(response.code, 0, `runtime MCP ${name} transport failed: ${response.stderr}`)
  const envelope = JSON.parse(response.stdout)
  assert.equal(envelope.ok, true, `runtime MCP ${name} returned HTTP ${envelope.status}`)
  const result = JSON.parse(envelope.body)
  if (result.error) return { ok: false, error: result.error, raw: result }
  return {
    ok: result.result?.isError !== true,
    content: result.result?.structuredContent,
    raw: result,
  }
}

async function pasteComputerCredential(providerRun, credentialId) {
  const paste = mcpToolCall(providerRun, "paste_secret_to_computer", {
    credential_id: credentialId,
  })
  const interaction = await waitForRuntimeInteraction(
    (candidate) => candidate.title === "Computer credential input",
    `Computer approval for ${credentialId}`,
  )
  assert.equal(interaction.level, "critical")
  assert.ok(String(interaction.message ?? "").includes("currently focused desktop control"))
  await client.send(requests.respondToInteractionRequest(sessionId, interaction.id, "allow", null))
  const result = await paste
  assert.equal(result.ok, true, JSON.stringify(result))
  assert.equal(result.content?.target, "desktop_focus")
  assertNoSecretProperties(result.raw, `Computer paste ${credentialId}`)
  return result
}

async function waitForRuntimeInteraction(predicate, label) {
  return await waitFor(async () => {
    const state = unwrapOneOf(
      await client.send(requests.getSessionStateRequest(sessionId)),
      "SessionStateLoaded",
      "SessionState",
    )
    return (state.session.active_interactions ?? []).find(predicate) ?? false
  }, 60_000, `${label} did not appear`)
}

function assertNoSecretProperties(value, label) {
  const forbidden = []
  const visit = (candidate, pathParts) => {
    if (Array.isArray(candidate)) {
      candidate.forEach((entry, index) => visit(entry, [...pathParts, String(index)]))
      return
    }
    if (!candidate || typeof candidate !== "object") return
    for (const [key, entry] of Object.entries(candidate)) {
      if (/secret|passphrase|vault_key/i.test(key)) forbidden.push([...pathParts, key].join("."))
      visit(entry, [...pathParts, key])
    }
  }
  visit(value, [])
  assert.deepEqual(forbidden, [], `${label} exposed secret-bearing properties: ${forbidden.join(", ")}`)
}

async function assertNoPlaintextSecretInTree(root, secrets) {
  const entries = await readdir(root, { withFileTypes: true }).catch(() => [])
  for (const entry of entries) {
    const target = path.join(root, entry.name)
    if (entry.isDirectory()) {
      await assertNoPlaintextSecretInTree(target, secrets)
      continue
    }
    if (!entry.isFile()) continue
    const metadata = await stat(target)
    assert.ok(metadata.size <= 64 * 1024 * 1024, `refusing unbounded leak scan for ${target}`)
    const bytes = await readFile(target)
    for (const secret of secrets) {
      assert.equal(bytes.includes(Buffer.from(secret)), false, `plaintext secret leaked into ${target}`)
    }
  }
}

function redactDrillSecrets(value) {
  return sensitiveValues.reduce(
    (current, secret) => current.replaceAll(secret, "[redacted]"),
    String(value),
  )
}

function scopedRelayToken({ subject, subjectKind, actions, userId = null }) {
  return roomDrillRelayToken({
    issuer: relayScopedIssuer,
    secret: relayScopedSecret,
    machineId: `${runId}-machine`,
    subject,
    subjectKind,
    actions,
    userId,
  })
}

async function runCompanionIfConfigured({ environment, localNoticeIds, remoteNoticeIds, activityController }) {
  const noticePattern = roomActionNoticePattern
  return await runRoomEnvironmentCompanion({
    env: process.env,
    sleep,
    prepare: async () => {
      // The keyboard/clipboard drills navigate away from the original click page.
      // Give the Web companion a fresh physical page, not the last drill's form.
      await sliceScreen(["open-url", `http://host.docker.internal:${fixture.port}/click`])
      await waitForBrowserText("POINTER_CLICK_READY", 30_000, "Web companion fixture did not reset")
      resources.push(await resourceSnapshot("before-web-companion"))
    },
    ready: {
      ...(webKeyboardText ? { keyboardText: webKeyboardText } : {}),
      ...(webKeyboardReplacementText ? { keyboardReplacementText: webKeyboardReplacementText } : {}),
      ...(webPointerGestures ? { pointerGestures: true } : {}),
      pointerClickExpectedCount: 1,
      ...(realProviderOptions ? { realProvider: realProviderOptions, providerWorkspace: fixtureWorkspace } : {}),
      kernelUrl: `ws://127.0.0.1:${kernelPort}/kernel`,
      relayUrl: `ws://127.0.0.1:${relayPort}`,
      relayToken: remoteTuiRelayToken,
      relayScopedIssuer,
      relayScopedSecret,
      daemonId: homeDaemonId,
      machineId: `${runId}-machine`,
      sessionId,
      sliceId: slice.id,
      containerName,
      environmentId: environment.environment_id,
      runtimeGeneration: environment.runtime_generation,
      viewportRevision: environment.viewport.revision,
      viewport: environment.viewport,
      evidenceRoot,
    },
    client,
    observerClient,
    requests,
    activityController,
    localNoticeIds,
    remoteNoticeIds,
    readTuiNotices: async () => {
      const [local, remote] = await Promise.all([localAutomation.send("snapshot"), remoteAutomation.send("snapshot")])
      return { local: automationNoticeEntries(local), remote: automationNoticeEntries(remote) }
    },
    waitForPhysicalEffect: (physicalEffect) => waitForBrowserText(
      physicalEffect,
      20_000,
      "Web companion click did not reach the physical browser",
    ),
    waitForLocalActionNotice: (baselineIds, action) => waitForTuiNoticeAfter(
      localAutomation,
      "local",
      noticePattern(action),
      baselineIds,
      20_000,
    ),
    waitForRemoteActionNotice: (baselineIds, action) => waitForTuiNoticeAfter(
      remoteAutomation,
      "remote",
      noticePattern(action),
      baselineIds,
      20_000,
    ),
  })
}

async function seedConfig(tempRoot) {
  const configDir = path.join(tempRoot, "home")
  const vaultPath = path.join(tempRoot, "vault", "vault.db")
  await mkdir(configDir, { recursive: true })
  await writeFile(path.join(configDir, "config.toml"), [
    "version = 1",
    "",
    "[state]",
    `path = ${JSON.stringify(path.join(tempRoot, "state.db"))}`,
    "",
    "[slices]",
    `root = ${JSON.stringify(path.join(tempRoot, "slices"))}`,
    "",
    "[slices.linux]",
    // This opt-in case runs real providers inside the production Bubblewrap
    // boundary. Docker's outer default profile prevents that boundary starting.
    ...(realProviderOptions ? ["allow_unconfined_seccomp = true"] : []),
    ...(process.env.CHARIOX_ROOM_DRILL_IMAGE?.trim()
      ? [`docker_image = ${JSON.stringify(process.env.CHARIOX_ROOM_DRILL_IMAGE.trim())}`, "build_image = \"never\""]
      : ["build_image = \"auto\""]),
    `memory_mb = ${sliceMemoryMb}`,
    "cpus = \"1\"",
    "screen_width = 1280",
    "screen_height = 800",
    "",
    "[credential_vault]",
    `service = ${JSON.stringify(`${runId}-vault`)}`,
    `path = ${JSON.stringify(vaultPath)}`,
    "backend = \"chariox_encrypted\"",
    "unlock_policy = \"ttl\"",
    "default_ttl_minutes = 30",
    "max_ttl_minutes = 240",
    "agent_management = \"allow\"",
    "",
  ].join("\n"))
}

async function resolveRuntimeBinary(name) {
  const cargoTargetDir = process.env.CARGO_TARGET_DIR
    ? path.resolve(process.env.CARGO_TARGET_DIR)
    : path.join(repoRoot, "target")
  const binary = path.join(cargoTargetDir, "debug", name)
  await access(binary).catch(() => {
    throw new Error(`missing ${binary}; build the current ${name} first`)
  })
  return binary
}

async function captureSourceIdentity(kernelBinary, relayBinary) {
  const commit = await runCommand("git", ["rev-parse", "HEAD"], 10_000)
  assert.equal(commit.code, 0, `git rev-parse failed: ${commit.stderr}`)
  const trackedStatus = await runCommand(
    "git",
    ["status", "--porcelain", "--untracked-files=no"],
    10_000,
  )
  assert.equal(trackedStatus.code, 0, `git status failed: ${trackedStatus.stderr}`)
  const [runtimeSourceRevision, kernelSha256, relaySha256, protocolVersions] = await Promise.all([
    currentRuntimeSourceRevision(),
    fileSha256(kernelBinary),
    fileSha256(relayBinary),
    currentProtocolVersions(),
  ])
  return {
    gitCommit: commit.stdout.trim(),
    trackedWorktreeClean: trackedStatus.stdout.trim().length === 0,
    runtimeSourceRevision,
    protocolVersions,
    hostBinaries: {
      kernel: { path: kernelBinary, sha256: kernelSha256 },
      relay: { path: relayBinary, sha256: relaySha256 },
    },
  }
}

async function currentRuntimeSourceRevision() {
  const roots = [
    "Cargo.toml",
    "Cargo.lock",
    "adapters/rust",
    "apps/aegs-dummy",
    "apps/kernel",
    "apps/relay",
    "examples/workflow-code",
    "packages/aegs-sdk",
    "packages/event-protocol",
  ]
  const listed = await runCommand(
    "git",
    ["ls-files", "--cached", "--others", "--exclude-standard", ...roots],
    20_000,
  )
  assert.equal(listed.code, 0, `git ls-files failed: ${listed.stderr}`)
  const aggregate = createHash("sha256")
  for (const relativePath of listed.stdout.split("\n").filter(Boolean)) {
    const bytes = await readFile(path.join(repoRoot, relativePath))
    aggregate.update(`${relativePath} ${createHash("sha256").update(bytes).digest("hex")}\n`)
  }
  return aggregate.digest("hex")
}

async function currentProtocolVersions() {
  const [localTypes, relayPeer] = await Promise.all([
    readFile(path.join(repoRoot, "apps", "kernel", "src", "local", "api", "types.rs"), "utf8"),
    readFile(path.join(repoRoot, "apps", "kernel", "src", "transport", "relay_peer.rs"), "utf8"),
  ])
  const local = Number(localTypes.match(/LOCAL_DAEMON_PROTOCOL_VERSION:\s*u32\s*=\s*(\d+)/)?.[1])
  const relay = Number(relayPeer.match(/RELAY_PEER_PROTOCOL_VERSION:\s*u32\s*=\s*(\d+)/)?.[1])
  assert.ok(Number.isInteger(local), "local daemon protocol version must be readable")
  assert.ok(Number.isInteger(relay), "relay peer protocol version must be readable")
  return { localDaemon: local, relayPeer: relay }
}

async function inspectSliceRuntimeIdentity() {
  const containerInspect = JSON.parse((await docker(["container", "inspect", containerName])).stdout)[0]
  assert.ok(containerInspect?.Image, "running slice image identity")
  const imageInspect = JSON.parse((await docker(["image", "inspect", containerInspect.Image])).stdout)[0]
  const labels = imageInspect?.Config?.Labels ?? {}
  const installed = await docker([
    "exec",
    "-u",
    "slice",
    containerName,
    "cat",
    "/opt/chariox-slice/runtime-source-revision",
  ])
  const workerKernelHash = await docker([
    "exec",
    "-u",
    "slice",
    containerName,
    "sha256sum",
    "/opt/chariox-slice/bin/chariox-kernel",
  ])
  return {
    imageId: containerInspect.Image,
    imageTag: containerInspect.Config?.Image ?? null,
    runtimeSourceRevision: labels["io.chariox.runtime-source-revision"] ?? null,
    installedRuntimeSourceRevision: installed.stdout.trim(),
    relayPeerProtocolVersion: labels["io.chariox.relay-peer-protocol-version"] ?? null,
    workerKernelSha256: workerKernelHash.stdout.trim().split(/\s+/)[0] ?? null,
  }
}

async function fileSha256(filePath) {
  return createHash("sha256").update(await readFile(filePath)).digest("hex")
}

async function startRemoteTui({ tempRoot }) {
  const env = remoteTuiEnvironment(tempRoot)
  for (const name of directDaemonEnvironmentNames) assert.equal(name in env, false)
  return await startTui({
    kind: "remote",
    tempRoot,
    env,
    connectionArgs: [
      "--relay-url", `ws://127.0.0.1:${relayPort}`,
      "--relay-token", remoteTuiRelayToken,
      "--target-daemon-id", homeDaemonId,
    ],
  })
}

async function startLocalTui({ tempRoot, kernelUrl }) {
  return await startTui({
    kind: "local",
    tempRoot,
    env: isolatedTuiEnvironment(tempRoot, "local"),
    connectionArgs: ["--kernel-url", kernelUrl],
  })
}

async function startTui({ kind, tempRoot, env, connectionArgs }) {
  const automationSocket = path.join(tempRoot, `${kind}-tui.sock`)
  const args = [
    "-q",
    "/dev/null",
    "bun",
    path.join(repoRoot, "apps", "cli", "dist", "index.js"),
    ...connectionArgs,
    "--automation-socket", automationSocket,
    "--session", sessionId,
    "--workspace", fixtureWorkspace,
    "--worktree", fixtureWorkspace,
    "--provider", "dev-stub",
    "--model", `room-activity-${kind}-tui-drill`,
    "--client-id", `${runId}-${kind}-tui`,
  ]
  const tui = spawn("script", args, {
    cwd: repoRoot,
    env,
    detached: true,
    stdio: ["ignore", "pipe", "pipe"],
  })
  tui.killProcessGroup = true
  children.push(tui)
  tui.stdout.on("data", (chunk) => {
    tuiOutput[kind] = `${tuiOutput[kind]}${chunk}`.slice(-16_000)
  })
  tui.stderr.on("data", (chunk) => {
    tuiOutput[kind] = `${tuiOutput[kind]}${chunk}`.slice(-16_000)
  })
  const startupFailure = new Promise((resolve) => {
    tui.once("error", resolve)
    tui.once("exit", (code, signal) => {
      resolve(new Error(`${kind} TUI exited during startup: code=${code ?? "none"} signal=${signal ?? "none"}`))
    })
  })
  const startup = await Promise.race([
    waitForSocket(automationSocket).then(() => null),
    startupFailure,
  ])
  if (startup) {
    throw new Error(`${startup.message}\n${kind} TUI output:\n${tuiOutput[kind].slice(-4_000)}`)
  }
  const automation = await createAutomationClient(automationSocket)
  await automation.send("ping")
  return automation
}

function remoteTuiEnvironment(tempRoot) {
  const env = isolatedTuiEnvironment(tempRoot, "remote")
  for (const name of directDaemonEnvironmentNames) {
    delete env[name]
  }
  remoteTuiHome = path.join(tempRoot, "remote-tui-os-home")
  return {
    ...env,
    HOME: remoteTuiHome,
  }
}

function isolatedTuiEnvironment(tempRoot, kind) {
  return {
    ...process.env,
    HOME: path.join(tempRoot, `${kind}-tui-os-home`),
    CHARIOX_HOME: path.join(tempRoot, `${kind}-tui-home`),
    XDG_CONFIG_HOME: path.join(tempRoot, `${kind}-tui-xdg-config`),
    XDG_STATE_HOME: path.join(tempRoot, `${kind}-tui-xdg-state`),
    XDG_CACHE_HOME: path.join(tempRoot, `${kind}-tui-xdg-cache`),
  }
}

async function captureScreenshotFromRemoteTui(tempRoot) {
  const snapshot = await remoteAutomation.send(
    "submit_prompt",
    { prompt: "/room screenshot" },
    60_000,
  )
  const notice = automationNoticeTexts(snapshot)
    .findLast((message) => message.startsWith("Room Environment screenshot saved.\n"))
  assert.ok(notice, "remote TUI did not report the saved Room screenshot")
  const fields = Object.fromEntries(notice.split("\n").slice(1).map((line) => {
    const separator = line.indexOf("=")
    assert.ok(separator > 0, `malformed screenshot notice line: ${line}`)
    return [line.slice(0, separator), line.slice(separator + 1)]
  }))
  const expectedRoot = path.join(remoteTuiHome, "Downloads", "Chariox")
  assert.equal(path.dirname(fields.path), expectedRoot)
  assert.match(fields.artifact, /^art_[0-9]+_[a-f0-9]{16}$/)
  assert.match(fields.sha256, /^[a-f0-9]{64}$/)

  const bytes = await readFile(fields.path)
  assert.deepEqual([...bytes.subarray(0, 8)], [137, 80, 78, 71, 13, 10, 26, 10])
  assert.equal(bytes.subarray(12, 16).toString("ascii"), "IHDR")
  const width = bytes.readUInt32BE(16)
  const height = bytes.readUInt32BE(20)
  assert.equal(width, 1280)
  assert.equal(height, 800)
  const sha256 = createHash("sha256").update(bytes).digest("hex")
  assert.equal(sha256, fields.sha256)
  const evidencePath = path.join(evidenceRoot, "remote-tui-room-screenshot.png")
  await writeFile(evidencePath, bytes)
  const relativeSourcePath = path.relative(tempRoot, fields.path)
  assert.equal(relativeSourcePath.startsWith(".."), false)
  assert.equal(path.isAbsolute(relativeSourcePath), false)
  return {
    artifactId: fields.artifact,
    sha256,
    sizeBytes: bytes.length,
    width,
    height,
    evidencePath,
  }
}

async function startFixture() {
  const expectedUserDigest = fnv1a64(userSecret)
  const expectedKeyboardDigest = fnv1a64(keyboardText)
  const expectedKeyboardReplacementDigest = fnv1a64(keyboardReplacementText)
  const expectedKeyboardAfterRepeatDigest = fnv1a64(keyboardAfterRepeat)
  const expectedCancellationRecoveryDigest = fnv1a64(cancellationRecoveryText)
  const server = http.createServer((request, response) => {
    if (request.url === "/secret") {
      response.writeHead(200, { "content-type": "text/html; charset=utf-8" })
      response.end(`<!doctype html><html><head><title>Room Computer secret drill</title><style>
        html,body{width:100%;height:100%;margin:0}body{background:#f4f4f3;color:#202124;font:24px sans-serif}
        main{box-sizing:border-box;margin:120px auto 0;width:680px}label{display:block;font-weight:700;margin:18px 0 8px}
        input{box-sizing:border-box;font:32px sans-serif;padding:12px 16px;width:100%}.status{font-weight:700;margin-top:8px}
      </style></head><body><main><h1>ROOM_COMPUTER_SECRET_READY</h1>
        <label for="user-secret">User-entered credential</label><input id="user-secret" type="password" autocomplete="off">
        <div class="status" id="user-status">USER_SECRET_WAITING</div>
        <label for="generated-secret">Generated credential</label><input id="generated-secret" type="password" autocomplete="off">
        <div class="status" id="generated-status">GENERATED_SECRET_WAITING</div>
      </main><script>
        const expectedUserDigest=${JSON.stringify(expectedUserDigest)};
        const generatedLength=${generatedSecretLength};
        function fnv1a64(value){let hash=14695981039346656037n;for(const byte of new TextEncoder().encode(value)){hash^=BigInt(byte);hash=BigInt.asUintN(64,hash*1099511628211n)}return hash.toString(16).padStart(16,"0")}
        document.querySelector("#user-secret").addEventListener("input",(event)=>{document.querySelector("#user-status").textContent=fnv1a64(event.target.value)===expectedUserDigest?"USER_SECRET_OK":"USER_SECRET_WAITING"});
        document.querySelector("#generated-secret").addEventListener("input",(event)=>{document.querySelector("#generated-status").textContent=event.target.value.length===generatedLength?"GENERATED_SECRET_OK":"GENERATED_SECRET_WAITING"});
      </script></body></html>`)
      return
    }
    if (request.url === "/keyboard") {
      response.writeHead(200, { "content-type": "text/html; charset=utf-8" })
      response.end(`<!doctype html><html><head><title>Room Computer keyboard drill</title><style>
        html,body{width:100%;height:100%;margin:0}body{background:#f4f4f3;color:#202124;font:24px sans-serif}
        main{box-sizing:border-box;margin:120px auto 0;width:680px}label{display:block;font-weight:700;margin:18px 0 8px}
        input{box-sizing:border-box;font:32px sans-serif;padding:12px 16px;width:100%}.status{font-weight:700;margin-top:18px}
      </style></head><body><main><h1>ROOM_COMPUTER_KEYBOARD_READY</h1>
        <label for="keyboard-input">Keyboard input</label><input id="keyboard-input" type="password" autocomplete="off" autofocus>
        <div class="status" id="keyboard-status">ROOM_COMPUTER_KEYBOARD_WAITING</div>
      </main><script>
        const expectedDigest=${JSON.stringify(expectedKeyboardDigest)};
        const expectedReplacementDigest=${JSON.stringify(expectedKeyboardReplacementDigest)};
        const expectedAfterRepeatDigest=${JSON.stringify(expectedKeyboardAfterRepeatDigest)};
        function fnv1a64(value){let hash=14695981039346656037n;for(const byte of new TextEncoder().encode(value)){hash^=BigInt(byte);hash=BigInt.asUintN(64,hash*1099511628211n)}return hash.toString(16).padStart(16,"0")}
        const input=document.querySelector("#keyboard-input");
        const status=document.querySelector("#keyboard-status");
        const confirmFocus=()=>{if(document.activeElement===input)status.textContent="ROOM_COMPUTER_KEYBOARD_FOCUS_OK"};
        input.addEventListener("input",()=>{const digest=fnv1a64(input.value);status.textContent=digest===expectedDigest?"ROOM_COMPUTER_KEYBOARD_TEXT_OK":digest===expectedReplacementDigest?"ROOM_COMPUTER_KEYBOARD_SHORTCUT_OK":digest===expectedAfterRepeatDigest?"ROOM_COMPUTER_KEYBOARD_REPEAT_OK":"ROOM_COMPUTER_KEYBOARD_WAITING"});
        input.addEventListener("focus",confirmFocus);
        input.addEventListener("select",()=>{if(input.selectionStart===0&&input.selectionEnd===input.value.length)status.textContent="ROOM_COMPUTER_KEYBOARD_SELECT_ALL_OK"});
        input.addEventListener("blur",()=>{status.textContent="ROOM_COMPUTER_KEYBOARD_FOCUS_LOST"});
        queueMicrotask(confirmFocus);
      </script></body></html>`)
      return
    }
    if (request.url === "/cancellation") {
      response.writeHead(200, { "content-type": "text/html; charset=utf-8" })
      response.end(`<!doctype html><html><head><title>Room Computer cancellation drill</title><style>
        html,body{width:100%;height:100%;margin:0}body{background:#f4f4f3;color:#202124;font:24px sans-serif}
        main{box-sizing:border-box;margin:120px auto 0;width:680px}label{display:block;font-weight:700;margin:18px 0 8px}
        input{box-sizing:border-box;font:32px sans-serif;padding:12px 16px;width:100%}.status{font-weight:700;margin-top:18px}
      </style></head><body><main><h1>ROOM_COMPUTER_CANCELLATION_READY</h1>
        <label for="cancellation-input">Cancellable keyboard input</label><input id="cancellation-input" type="password" autocomplete="off" autofocus>
        <div class="status" id="cancellation-state">ROOM_COMPUTER_CANCELLATION_WAITING</div>
        <div class="status" id="cancellation-count">ROOM_COMPUTER_CANCELLATION_COUNT=0</div>
      </main><script>
        const expectedRecoveryDigest=${JSON.stringify(expectedCancellationRecoveryDigest)};
        function fnv1a64(value){let hash=14695981039346656037n;for(const byte of new TextEncoder().encode(value)){hash^=BigInt(byte);hash=BigInt.asUintN(64,hash*1099511628211n)}return hash.toString(16).padStart(16,"0")}
        const input=document.querySelector("#cancellation-input");
        const state=document.querySelector("#cancellation-state");
        const count=document.querySelector("#cancellation-count");
        const confirmFocus=()=>{if(document.activeElement===input)state.textContent="ROOM_COMPUTER_CANCELLATION_FOCUS_OK"};
        input.addEventListener("input",()=>{count.textContent="ROOM_COMPUTER_CANCELLATION_COUNT="+input.value.length;state.textContent=fnv1a64(input.value)===expectedRecoveryDigest?"ROOM_COMPUTER_CANCELLATION_RECOVERED":"ROOM_COMPUTER_CANCELLATION_RUNNING"});
        input.addEventListener("focus",confirmFocus);
        input.addEventListener("select",()=>{if(input.selectionStart===0&&input.selectionEnd===input.value.length)state.textContent="ROOM_COMPUTER_CANCELLATION_SELECTED"});
        input.addEventListener("blur",()=>{state.textContent="ROOM_COMPUTER_CANCELLATION_FOCUS_LOST"});
        queueMicrotask(confirmFocus);
      </script></body></html>`)
      return
    }
    if (request.url === "/pointer-matrix") {
      response.writeHead(200, { "content-type": "text/html; charset=utf-8" })
      response.end(`<!doctype html><html><head><title>Room Computer pointer matrix</title><style>
        *{box-sizing:border-box}html,body{width:100%;height:100%;margin:0;overflow:hidden}body{background:#f4f4f3;color:#202124;font:20px sans-serif}
        h1{font-size:22px;left:16px;margin:0;position:fixed;top:4px}.target{align-items:center;border:3px solid #555;display:flex;font-weight:700;height:120px;justify-content:center;position:fixed;top:20px;width:200px}
        #move-target{left:60px;width:200px}#single-target{left:360px}#right-target{left:600px}#double-target{left:840px}
        #selection-input{font:32px monospace;height:100px;left:160px;padding:20px;position:fixed;top:250px;width:800px}
        #selection-status{font-weight:700;left:160px;position:fixed;top:355px}
        #scroller{border:3px solid #555;height:160px;left:160px;overflow:scroll;position:fixed;top:480px;width:960px}
        #scroll-content{background:linear-gradient(135deg,#f7b267,#70c1b3);height:640px;padding:20px;width:2200px}
        #scroll-status{font-weight:700;left:180px;position:sticky;top:20px}
      </style></head><body><h1>ROOM_COMPUTER_POINTER_READY</h1>
        <div class="target" id="move-target">POINTER_MOVE_WAITING</div>
        <button class="target" id="single-target">POINTER_SINGLE_WAITING</button>
        <button class="target" id="right-target">POINTER_RIGHT_WAITING</button>
        <button class="target" id="double-target">POINTER_DOUBLE_WAITING</button>
        <input id="selection-input" value="Select this entire physical pointer sample without moving the browser window">
        <div id="selection-status">TEXT_SELECTION_WAITING</div>
        <div id="scroller"><div id="scroll-content"><span id="scroll-status">POINTER_SCROLL_WAITING</span></div></div>
      <script>
        let stableGeometry=null;
        const moveTarget=document.querySelector("#move-target");
        moveTarget.addEventListener("mousemove",()=>{stableGeometry=[window.screenX,window.screenY,window.outerWidth,window.outerHeight].join(":");moveTarget.textContent="POINTER_MOVE_OK"});
        const singleTarget=document.querySelector("#single-target");let singleClicks=0;
        singleTarget.addEventListener("click",()=>{singleClicks+=1;singleTarget.textContent=singleClicks===1?"POINTER_SINGLE_CLICK_OK":"POINTER_SINGLE_CLICK_COUNT="+singleClicks});
        const rightTarget=document.querySelector("#right-target");
        rightTarget.addEventListener("contextmenu",(event)=>{event.preventDefault();rightTarget.textContent="POINTER_RIGHT_CLICK_OK"});
        const doubleTarget=document.querySelector("#double-target");
        doubleTarget.addEventListener("dblclick",()=>{doubleTarget.textContent="POINTER_DOUBLE_CLICK_OK"});
        const selectionInput=document.querySelector("#selection-input");
        const updateSelectionStatus=()=>{
          const selected=Math.abs(selectionInput.selectionEnd-selectionInput.selectionStart);
          const geometry=[window.screenX,window.screenY,window.outerWidth,window.outerHeight].join(":");
          document.querySelector("#selection-status").textContent=selected>=8?"TEXT_SELECTION_OK "+(stableGeometry&&geometry===stableGeometry?"WINDOW_GEOMETRY_STABLE":"WINDOW_GEOMETRY_CHANGED"):"TEXT_SELECTION_WAITING";
        };
        selectionInput.addEventListener("select",updateSelectionStatus);
        document.addEventListener("selectionchange",updateSelectionStatus);
        const scroller=document.querySelector("#scroller");
        scroller.addEventListener("scroll",()=>{
          if(scroller.scrollLeft>0)scroller.dataset.horizontal="ok";
          if(scroller.scrollTop>0)scroller.dataset.vertical="ok";
          document.querySelector("#scroll-status").textContent=scroller.dataset.horizontal==="ok"&&scroller.dataset.vertical==="ok"?"POINTER_SCROLL_BOTH_AXES_OK":"POINTER_SCROLL_WAITING";
        });
      </script></body></html>`)
      return
    }
    if (new URL(request.url, "http://fixture.invalid").pathname !== "/click") {
      response.writeHead(404).end("not found")
      return
    }
    const browserFixture = roomProviderBrowserFixture(realProviderOptions, request.url)
    response.writeHead(200, { "content-type": "text/html; charset=utf-8" })
    response.end(`<!doctype html><html><head><title>Room pointer drill</title><style>
      html,body{width:100%;height:100%;margin:0}body{display:grid;place-items:center;background:#ddd;font:${webPointerGestures ? 20 : 32}px sans-serif}
      #web-keyboard{position:fixed;inset:0;width:100%;height:100%;box-sizing:border-box;border:0;background:transparent;color:transparent;caret-color:transparent;outline:none}
      #web-selection{position:fixed;left:160px;top:40px;width:800px;height:100px;box-sizing:border-box;padding:20px;font:32px monospace;z-index:2}
      #web-scroller{position:fixed;left:160px;top:420px;width:960px;height:160px;overflow:scroll;z-index:2;border:3px solid #333}
      #web-scroll-content{width:2200px;height:640px;background:linear-gradient(135deg,#f7b267,#70c1b3)}
      main{pointer-events:none;z-index:1}
      #browser-action-target{position:fixed;left:16px;top:16px;z-index:3}
    </style></head><body>${webKeyboardText ? '<input id="web-keyboard" type="password" autocomplete="off" aria-label="Web keyboard fixture">' : ''}${webPointerGestures ? '<input data-web-gesture id="web-selection" readonly value="Select this physical text without moving the Room browser window"><div data-web-gesture id="web-scroller"><div id="web-scroll-content"></div></div>' : ''}<main><div id="state">POINTER_CLICK_READY</div>${webKeyboardText ? '<div id="web-keyboard-status">WEB_KEYBOARD_WAITING</div><div id="web-keyboard-replacement-status">WEB_KEYBOARD_REPLACEMENT_WAITING</div>' : ''}${webPointerGestures ? '<div id="web-drag-status">WEB_DRAG_WAITING</div><div id="web-scroll-status">WEB_SCROLL_WAITING</div>' : ''}</main><script>
      ${browserFixture.script}
      let clicks=${browserFixture.initialClicks};document.addEventListener("click",(event)=>{if(event.target.closest("[data-web-gesture]"))return;clicks+=1;document.body.style.background="#69d391";document.querySelector("#state").textContent="POINTER_CLICK_COUNT="+clicks})
      ${webPointerGestures ? `
      const selection=document.querySelector("#web-selection");
      // Native input gives the physical content origin even when CDP emulates
      // innerWidth/innerHeight. The Web drill reads this after its focus click.
      document.addEventListener("mousedown",event=>{
        if(event.isTrusted)window.charioxGestureOrigin={x:event.screenX-event.clientX,y:event.screenY-event.clientY};
      },true);
      let geometry=null;
      const windowGeometry=()=>[window.screenX,window.screenY,window.outerWidth,window.outerHeight].join(":");
      selection.addEventListener("mousedown",()=>{geometry=windowGeometry()});
      selection.addEventListener("select",()=>{
        if(Math.abs(selection.selectionEnd-selection.selectionStart)>=8){
          document.querySelector("#web-drag-status").textContent="WEB_DRAG_SELECTION_OK "+(geometry===windowGeometry()?"WINDOW_GEOMETRY_STABLE":"WINDOW_GEOMETRY_CHANGED");
        }
      });
      const scroller=document.querySelector("#web-scroller");
      scroller.addEventListener("scroll",()=>{
        if(scroller.scrollLeft>0&&scroller.scrollTop>0)document.querySelector("#web-scroll-status").textContent="WEB_SCROLL_BOTH_AXES_OK";
      });` : ''}
      ${webKeyboardText ? `
      document.querySelector("#web-keyboard").addEventListener("input",(event)=>{
        let hash=14695981039346656037n;
        for(const byte of new TextEncoder().encode(event.target.value)){hash^=BigInt(byte);hash=BigInt.asUintN(64,hash*1099511628211n)}
        const digest=hash.toString(16).padStart(16,"0");
        if(digest===${JSON.stringify(fnv1a64(webKeyboardText))})document.querySelector("#web-keyboard-status").textContent="WEB_KEYBOARD_TEXT_OK";
        document.querySelector("#web-keyboard-replacement-status").textContent=digest===${JSON.stringify(fnv1a64(webKeyboardReplacementText))}?"WEB_KEYBOARD_REPLACEMENT_OK":"WEB_KEYBOARD_REPLACEMENT_WAITING";
      });` : ''}
    </script></body></html>`)
  })
  await new Promise((resolve, reject) => {
    server.once("error", reject)
    server.listen(0, "0.0.0.0", resolve)
  })
  return { server, port: server.address().port }
}

function fnv1a64(value) {
  let hash = 14_695_981_039_346_656_037n
  for (const byte of Buffer.from(value)) {
    hash ^= BigInt(byte)
    hash = BigInt.asUintN(64, hash * 1_099_511_628_211n)
  }
  return hash.toString(16).padStart(16, "0")
}

async function waitForSliceRunning(sliceRef) {
  return await waitFor(async () => {
    const current = unwrap(await client.send(requests.getSliceRequest(sliceRef)), "Slice").slice
    return current.status === "running" ? current : false
  }, 600_000, `slice ${sliceRef} did not become running`)
}

async function waitForBrowserText(needle, timeoutMs, message) {
  return await waitFor(async () => {
    const text = await sliceScreen(["browser-text"]).catch(() => "")
    return text.includes(needle) ? text : false
  }, timeoutMs, message)
}

async function waitForBrowserReady(timeoutMs) {
  return await waitFor(async () => {
    const status = await sliceScreen(["browser-status"]).catch(() => "")
    try {
      const browser = JSON.parse(status)
      return browser.readyState === "complete" && typeof browser.url === "string"
        ? browser
        : false
    } catch {
      return false
    }
  }, timeoutMs, "headed Chromium did not expose a ready browser target")
}

async function screenshot(name) {
  const inside = `/tmp/${name}.png`
  await sliceScreen(["screenshot", inside])
  await docker(["cp", `${containerName}:${inside}`, path.join(evidenceRoot, `${name}.png`)])
  return path.join(evidenceRoot, `${name}.png`)
}

async function waitForRemoteNotice(pattern, timeoutMs = 20_000) {
  return await waitForTuiNotice(remoteAutomation, "remote", pattern, timeoutMs)
}

async function waitForLocalNotice(pattern, timeoutMs = 20_000) {
  return await waitForTuiNotice(localAutomation, "local", pattern, timeoutMs)
}

async function waitForRoomReadyProjection(automation, kind) {
  return await waitForAutomationSnapshot(
    automation,
    (snapshot) => hasRoomReadyProjection(automationNoticeTexts(snapshot)),
    `${kind} TUI Room ready projection`,
    60_000,
  )
}

async function waitForTuiNotice(automation, kind, pattern, timeoutMs) {
  return await waitForAutomationSnapshot(
    automation,
    (snapshot) => automationNoticeTexts(snapshot).some((notice) => pattern.test(notice)),
    `${kind} TUI notice ${pattern}`,
    timeoutMs,
  )
}

async function waitForTuiNoticeAfter(automation, kind, pattern, baselineIds, timeoutMs) {
  const baseline = new Set(baselineIds)
  return await waitForAutomationSnapshot(
    automation,
    (snapshot) => automationNoticeEntries(snapshot).some((notice) => (
      !baseline.has(notice.id) && pattern.test(notice.text)
    )),
    `${kind} TUI notice after transcript baseline ${pattern}`,
    timeoutMs,
  )
}

async function waitForSocket(socketPath, timeoutMs = 20_000) {
  return await waitFor(async () => {
    const socket = net.createConnection(socketPath)
    try {
      await new Promise((resolve, reject) => {
        socket.once("connect", resolve)
        socket.once("error", reject)
      })
      return true
    } finally {
      socket.destroy()
    }
  }, timeoutMs, `automation socket ${socketPath} did not become ready`)
}

async function waitForTcpPort(host, port, timeoutMs, message) {
  return await waitFor(async () => {
    const socket = net.createConnection({ host, port })
    try {
      await new Promise((resolve, reject) => {
        socket.once("connect", resolve)
        socket.once("error", reject)
      })
      return true
    } finally {
      socket.destroy()
    }
  }, timeoutMs, message)
}

async function createAutomationClient(socketPath) {
  const socket = net.createConnection(socketPath)
  socket.setEncoding("utf8")
  await new Promise((resolve, reject) => {
    socket.once("connect", resolve)
    socket.once("error", reject)
  })
  let buffer = ""
  let nextId = 1
  const pending = new Map()
  socket.on("data", (chunk) => {
    buffer += chunk
    while (buffer.includes("\n")) {
      const newline = buffer.indexOf("\n")
      const line = buffer.slice(0, newline).trim()
      buffer = buffer.slice(newline + 1)
      if (!line) continue
      const response = JSON.parse(line)
      const deferred = pending.get(response.id)
      if (!deferred) continue
      pending.delete(response.id)
      clearTimeout(deferred.timeout)
      if (response.ok) deferred.resolve(response.data)
      else deferred.reject(new Error(response.error ?? "automation command failed"))
    }
  })
  const rejectPending = (error) => {
    for (const deferred of pending.values()) {
      clearTimeout(deferred.timeout)
      deferred.reject(error)
    }
    pending.clear()
  }
  socket.on("error", rejectPending)
  socket.on("close", () => rejectPending(new Error("automation socket closed")))
  return {
    send(action, fields = {}, timeoutMs = 10_000) {
      const id = nextId++
      return new Promise((resolve, reject) => {
        const timeout = setTimeout(() => {
          pending.delete(id)
          reject(new Error(`automation action ${action} timed out after ${timeoutMs}ms`))
        }, timeoutMs)
        pending.set(id, { resolve, reject, timeout })
        socket.write(`${JSON.stringify({ id, action, ...fields })}\n`)
      })
    },
    close() {
      rejectPending(new Error("automation client closed"))
      socket.destroy()
    },
  }
}

async function waitForAutomationSnapshot(automation, predicate, label, timeoutMs = 20_000) {
  let lastSnapshot = null
  try {
    return await waitFor(async () => {
      lastSnapshot = await automation.send("snapshot")
      return predicate(lastSnapshot) ? lastSnapshot : false
    }, timeoutMs, `${label} did not appear`)
  } catch (error) {
    throw new Error(`${error.message}; last snapshot ${JSON.stringify(lastSnapshot)}`)
  }
}

async function sliceScreen(args) {
  const result = await docker(["exec", "-u", "slice", containerName, "/opt/chariox-slice/slice-screen.sh", ...args])
  return `${result.stdout}${result.stderr}`
}

async function readPhysicalClipboard() {
  const result = await docker([
    "exec",
    "-u",
    "slice",
    containerName,
    "/opt/chariox-slice/slice-screen.sh",
    "computer-clipboard-read",
  ])
  assert.equal(result.stderr, "", "physical clipboard read emitted stderr")
  return result.stdout
}

async function writePhysicalClipboard(text) {
  const result = await runCommandWithStdin(
    "docker",
    [
      "exec",
      "-i",
      "-u",
      "slice",
      containerName,
      "/opt/chariox-slice/slice-screen.sh",
      "computer-clipboard-write-stdin",
    ],
    text,
    20_000,
  )
  assert.equal(result.code, 0, `physical clipboard write failed: ${result.stderr}`)
  assert.equal(result.stdout, "", "physical clipboard write emitted stdout")
  assert.equal(result.stderr, "", "physical clipboard write emitted stderr")
}

async function assertDockerReady() {
  await docker(["info", "--format", "{{json .ServerVersion}}"], 20_000)
}

async function docker(args, timeoutMs = 120_000) {
  const result = await runCommand("docker", args, timeoutMs)
  if (result.code !== 0) throw new Error(`docker ${args.join(" ")} failed\n${result.stdout}${result.stderr}`)
  return result
}

async function resourceSnapshot(label) {
  const disk = await runCommand("df", ["-k", repoRoot], 10_000)
  const memoryPressure = await runCommand("memory_pressure", ["-Q"], 10_000)
    .then((result) => result.code === 0 ? result.stdout.trim() : null)
    .catch(() => null)
  const swapUsage = await runCommand("sysctl", ["-n", "vm.swapusage"], 10_000)
    .then((result) => result.code === 0 ? result.stdout.trim() : null)
    .catch(() => null)
  const dockerStats = slice
    ? await runCommand("docker", ["stats", "--no-stream", "--format", "{{json .}}", containerName], 20_000)
        .then((result) => result.code === 0 ? result.stdout.trim() : null)
    : null
  const limits = slice ? await dockerLimits().catch(() => null) : null
  return {
    label,
    at: new Date().toISOString(),
    freeMemoryBytes: os.freemem(),
    memoryPressure,
    swapUsage,
    loadAverage: os.loadavg(),
    disk: disk.stdout.trim().split("\n").at(-1),
    dockerStats,
    containerLimits: limits,
  }
}

async function dockerLimits() {
  const inspected = await docker(["container", "inspect", containerName])
  const hostConfig = JSON.parse(inspected.stdout)[0]?.HostConfig
  assert.ok(hostConfig, `missing Docker HostConfig for ${containerName}`)
  return {
    memoryBytes: hostConfig.Memory,
    memorySwapBytes: hostConfig.MemorySwap,
    nanoCpus: hostConfig.NanoCpus,
    pidsLimit: hostConfig.PidsLimit,
  }
}

async function cleanup() {
  const tempRoot = await tempRootPromise
  try { localForwarding?.assertHealthy() } catch (error) { failure ??= error }
  if (failure) {
    // Capture the failure before teardown adds disconnect/retry noise.
    const privateRelayPort = slice?.local_docker_ports?.relay
    const diagnostic = await captureRoomKernelDiagnostics(path.join(tempRoot, "kernel-logs"), {
      primary: `ws://127.0.0.1:${relayPort}`,
      private: slice?.relay_endpoint?.private ? slice.relay_endpoint.url
        : Number.isInteger(privateRelayPort) ? `ws://127.0.0.1:${privateRelayPort}` : undefined,
    }).catch(() => ({ status: "unavailable" }))
    await writeFile(path.join(evidenceRoot, "kernel-connection-diagnostic.json"),
      `${JSON.stringify(diagnostic, null, 2)}\n`, { mode: 0o600 })
      .catch(() => undefined)
  }
  if (failure && slice) {
    const diagnostic = await captureRoomStreamerDiagnostics(containerName, runCommand)
      .catch(() => ({ status: "unavailable" }))
    await writeFile(path.join(evidenceRoot, "streamer-failure-diagnostic.json"),
      `${JSON.stringify(diagnostic, null, 2)}\n`, { mode: 0o600 }).catch(() => undefined)
  }
  if (client && requests) {
    for (const id of drillCredentialIds) {
      await withTimeout(client.send(requests.deleteCredentialSecretRequest(id)), 2_000, "cleanup credential").catch(() => undefined)
    }
  }
  if (client && requests && sessionId) {
    await withTimeout(client.send(requests.stopRoomEnvironmentRequest(sessionId)), 2_000, "cleanup StopRoomEnvironment").catch(() => undefined)
    await withTimeout(client.send(requests.endSessionRequest(sessionId)), 2_000, "cleanup EndSession").catch(() => undefined)
  }
  if (failure && client && requests && slice) {
    const logs = await withTimeout(
      client.send(requests.getSliceLogsRequest(slice.id, 250)),
      5_000,
      "cleanup GetSliceLogs",
    ).catch(() => null)
    if (logs) {
      await writeFile(
        path.join(evidenceRoot, "slice-logs.json"),
        `${redactDrillSecrets(JSON.stringify(logs, null, 2))}\n`,
      ).catch(() => undefined)
    }
  }
  if (client && requests && slice) {
    await withTimeout(client.send(requests.deleteSliceRequest(slice.id)), 2_000, "cleanup DeleteSlice").catch(() => undefined)
  }
  await client?.close?.()
  await observerClient?.close?.()
  await workerClient?.close?.()
  localAutomation?.close()
  remoteAutomation?.close()
  await closeFixtureServer()
  if (localForwarding) await localForwarding.close().catch((error) => { failure ??= error })
  for (const child of children.toReversed()) await terminateChild(child)
  // Stop resource producers before the final removal, including a kernel that
  // was still provisioning when interrupted. Otherwise a late container can
  // appear after cleanup has already removed its predecessor.
  await docker(["rm", "-f", containerName]).catch(() => undefined)
  await docker(["volume", "rm", "-f", homeVolume]).catch(() => undefined)
  let leakedEvidence = false
  try {
    await assertNoPlaintextSecretInTree(tempRoot, sensitiveValues)
    await assertNoPlaintextSecretInTree(evidenceRoot, sensitiveValues)
  } catch (error) {
    leakedEvidence = true
    failure ??= error
    await rm(evidenceRoot, { recursive: true, force: true })
    await mkdir(evidenceRoot, { recursive: true })
  }
  await rm(tempRoot, { recursive: true, force: true })
  const after = await resourceSnapshot("after").catch(() => ({ label: "after", at: new Date().toISOString() }))
  resources.push(after)
  const containerGone = (await runCommand("docker", ["container", "inspect", containerName], 20_000)).code !== 0
  const volumeGone = (await runCommand("docker", ["volume", "inspect", homeVolume], 20_000)).code !== 0
  const ports = [...new Set([relayPort, kernelPort, kernelPort + 1, kernelPort + 2, kernelPort + 3,
    fixture?.port, ...(localForwarding?.ports ?? [])].filter(Number.isInteger))]
  const occupiedPorts = []
  for (const port of ports) {
    if (!(await portIsAvailable(port))) occupiedPorts.push(port)
  }
  const tempRootRemoved = await access(tempRoot).then(() => false).catch(() => true)
  const cleanupResult = {
    containerGone,
    volumeGone,
    tempRootRemoved,
    listenersReleased: occupiedPorts.length === 0,
    plaintextSecretLeak: leakedEvidence,
    occupiedPorts,
    resource: after,
  }
  await writeFile(path.join(evidenceRoot, "cleanup.json"), `${JSON.stringify(cleanupResult, null, 2)}\n`)
  if ((!containerGone || !volumeGone || !tempRootRemoved || occupiedPorts.length > 0) && failure == null) {
    failure = new Error(`drill cleanup failed: ${JSON.stringify(cleanupResult)}`)
  }
  if (result && failure == null) {
    result.finishedAt = new Date().toISOString()
    result.resources = resources
    result.cleanup = cleanupResult
    result.artifacts = await evidenceArtifacts()
    await writeFile(path.join(evidenceRoot, "result.json"), `${JSON.stringify(result, null, 2)}\n`)
  }
  if (failure) {
    await writeFile(
      path.join(evidenceRoot, "failure.txt"),
      redactDrillSecrets(`${failure?.stack ?? String(failure)}\n\nlocal TUI output:\n${tuiOutput.local}\n\nremote TUI output:\n${tuiOutput.remote}\n`),
    )
  }
}

async function evidenceArtifacts() {
  const entries = await readdir(evidenceRoot, { withFileTypes: true })
  const artifacts = []
  for (const entry of entries) {
    if (!entry.isFile() || ["result.json", "failure.txt"].includes(entry.name)) continue
    const artifactPath = path.join(evidenceRoot, entry.name)
    const metadata = await stat(artifactPath)
    artifacts.push({
      name: entry.name,
      path: artifactPath,
      sizeBytes: metadata.size,
      sha256: await fileSha256(artifactPath),
    })
  }
  return artifacts.sort((left, right) => left.name.localeCompare(right.name))
}

async function closeFixtureServer() {
  if (!fixture?.server) return
  fixture.server.closeAllConnections?.()
  fixture.server.closeIdleConnections?.()
  await new Promise((resolve) => fixture.server.close(resolve))
}

async function terminateChild(child) {
  if (!child) return
  if (child.killProcessGroup) {
    signalProcessGroup(child.pid, "SIGTERM")
    await waitForChildExit(child, 5_000)
    if (await waitForProcessGroupExit(child.pid, 500)) return
    signalProcessGroup(child.pid, "SIGKILL")
    await waitForProcessGroupExit(child.pid, 1_000)
    return
  }
  if (child.exitCode != null) return
  child.kill("SIGTERM")
  if (await waitForChildExit(child, 5_000)) return
  child.kill("SIGKILL")
  await waitForChildExit(child, 1_000)
}

function waitForChildExit(child, timeoutMs) {
  if (child.exitCode != null) return Promise.resolve(true)
  return new Promise((resolve) => {
    let settled = false
    const finish = (exited) => {
      if (settled) return
      settled = true
      clearTimeout(timeout)
      child.off("exit", onExit)
      resolve(exited)
    }
    const onExit = () => finish(true)
    const timeout = setTimeout(() => finish(false), timeoutMs)
    child.once("exit", onExit)
  })
}

async function waitForProcessGroupExit(processGroupId, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if (!processGroupExists(processGroupId)) return true
    await sleep(50)
  }
  return !processGroupExists(processGroupId)
}

function processGroupExists(processGroupId) {
  try {
    process.kill(-processGroupId, 0)
    return true
  } catch (error) {
    if (error?.code === "ESRCH") return false
    throw error
  }
}

function signalProcessGroup(processGroupId, signal) {
  try {
    process.kill(-processGroupId, signal)
  } catch (error) {
    if (error?.code !== "ESRCH") throw error
  }
}

async function waitFor(operation, timeoutMs, message) {
  const deadline = Date.now() + timeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    interruption.check()
    try {
      const value = await operation()
      if (value) return value
    } catch (error) {
      interruption.check()
      lastError = error
    }
    await sleep(250)
  }
  throw new Error(`${message}${lastError ? `: ${lastError.message}` : ""}`)
}

function runCommand(command, args, timeoutMs) {
  interruption.check()
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { cwd: repoRoot, env: process.env, stdio: ["ignore", "pipe", "pipe"] })
    const stdout = []
    const stderr = []
    const timeout = setTimeout(() => child.kill("SIGTERM"), timeoutMs)
    child.stdout.on("data", (chunk) => { stdout.push(chunk) })
    child.stderr.on("data", (chunk) => { stderr.push(chunk) })
    child.once("error", reject)
    child.once("close", (code, signal) => {
      clearTimeout(timeout)
      resolve({
        code,
        signal,
        stdout: utf8TextFromChunks(stdout),
        stderr: utf8TextFromChunks(stderr),
      })
    })
  })
}

function runCommandWithStdin(command, args, stdin, timeoutMs) {
  interruption.check()
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { cwd: repoRoot, env: process.env, stdio: ["pipe", "pipe", "pipe"] })
    const stdout = []
    const stderr = []
    const timeout = setTimeout(() => child.kill("SIGTERM"), timeoutMs)
    child.stdout.on("data", (chunk) => { stdout.push(chunk) })
    child.stderr.on("data", (chunk) => { stderr.push(chunk) })
    child.once("error", reject)
    child.once("close", (code, signal) => {
      clearTimeout(timeout)
      resolve({
        code,
        signal,
        stdout: utf8TextFromChunks(stdout),
        stderr: utf8TextFromChunks(stderr),
      })
    })
    child.stdin.end(stdin)
  })
}

function actionState(environment, actionId) {
  return environment.actions.find((action) => action.action_id === actionId)?.state
}

function unwrap(response, variant) {
  assert.ok(response && typeof response === "object" && variant in response, `expected ${variant}, got ${JSON.stringify(response)}`)
  return response[variant]
}

function unwrapOneOf(response, ...variants) {
  for (const variant of variants) {
    if (response && typeof response === "object" && variant in response) return response[variant]
  }
  assert.fail(`expected ${variants.join(" or ")}, got ${JSON.stringify(response)}`)
}

function sleep(ms) {
  return interruption.sleep(ms)
}

async function withTimeout(promise, timeoutMs, label) {
  let timeout = null
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timeout = setTimeout(() => reject(new Error(`${label} timed out after ${timeoutMs}ms`)), timeoutMs)
      }),
    ])
  } finally {
    if (timeout) clearTimeout(timeout)
  }
}
