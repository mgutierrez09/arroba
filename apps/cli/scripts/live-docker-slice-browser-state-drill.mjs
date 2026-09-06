#!/usr/bin/env node
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { access, mkdir, readdir, rm, writeFile } from "node:fs/promises"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { browserStateCleanupFailure } from "./lib/browser-state-drill-cleanup.mjs"
import { browserStateDrillImageConfig } from "./lib/browser-state-drill-image.mjs"
import { resolveBrowserStateDrillPaths } from "./lib/browser-state-drill-paths.mjs"
import { startBrowserComputerFixture } from "./lib/browser-computer-fixture.mjs"
import { finalizeDrillArtifacts } from "./lib/drill-artifacts.mjs"
import { resolveBuiltBinary } from "./lib/drill-runtime-helpers.mjs"

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const startedAt = new Date().toISOString()
const stamp = startedAt.replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z")
const runId = `m20-docker-state-${process.pid}-${stamp}`
const { artifactDir, tempRoot } = resolveBrowserStateDrillPaths({
  homeDir: os.homedir(),
  runId,
  stamp,
  env: process.env,
})
const kernelPort = Number.parseInt(process.env.M20_KERNEL_PORT ?? "", 10) || 55000 + Math.floor(Math.random() * 2000)
const kernelUrl = `ws://127.0.0.1:${kernelPort}/kernel`
// The slice marks this deterministic fixture origin as trustworthy so Chromium
// can exercise service workers without weakening arbitrary HTTP origins.
const fixturePort = 4321
const sliceName = `m20-${process.pid}`
const containerName = `chariox-slice-${sliceName}`
const homeVolume = `${containerName}-home`
const email = "agent@chariox.test"
const password = `m20-password-${process.pid}`
const recipient = "recipient@chariox.test"
const markers = {
  stateCookie: `cookie-${process.pid}-${Date.now()}`,
  stateLocalStorage: `local-${process.pid}-${Date.now()}`,
  stateIndexedDb: `idb-${process.pid}-${Date.now()}`,
  stateCacheStorage: `cache-${process.pid}-${Date.now()}`,
  downloadedFile: "CHARIOX_FIXTURE_DOWNLOAD",
  appConfig: `app-config-${process.pid}-${Date.now()}`,
  appUserData: `app-data-${process.pid}-${Date.now()}`,
  backupMutationOne: `backup-mutation-one-${process.pid}-${Date.now()}`,
  backupMutationTwo: `backup-mutation-two-${process.pid}-${Date.now()}`,
  firstSubject: `M20 first ${process.pid}`,
  secondSubject: `M20 second ${process.pid}`,
  reauthSubject: `M20 reauth ${process.pid}`,
}
const screenshots = {}
const children = []
let client = null
let requests = null
let slice = null
let fixture = null
let savedState = null
let namedBackup = null
let corruptBackup = null
let cleanupResult = null
let sourceIdentity = null
let stateImagesBefore = new Set()
let rollbackImagesBefore = new Set()
const sliceRuntime = {}
const persistenceIdentity = {}
const externalServiceReauthentication = {}
const resources = []

await mkdir(artifactDir, { recursive: true })
await mkdir(tempRoot, { recursive: true })

let failure = null
try {
  await run()
} catch (error) {
  failure = error
}
try {
  cleanupResult = await cleanup()
} catch (error) {
  failure ??= error
}
if (failure) {
  await writeManifest(false, failure)
  await finalizeDrillArtifacts({
    rootDir: artifactDir,
    passed: false,
    preserveOnFailure: true,
    failure,
    metadata: {
      drill: "docker-slice-browser-state",
      artifactDir,
      tempRoot,
      sliceName,
      containerName,
      homeVolume,
      fixturePort,
      markers,
      screenshots,
      cleanup: cleanupResult,
    },
    log,
  })
  console.error(failure?.stack ?? String(failure))
  process.exitCode = 1
} else {
  await writeManifest(true)
  console.log(`M20_DOCKER_SLICE_BROWSER_STATE_PASS ${JSON.stringify({ artifactDir, screenshots, markers, cleanup: cleanupResult })}`)
}

async function run() {
  log("checking Docker")
  await assertDockerReady()
  stateImagesBefore = new Set(await listDockerImageRefs(`chariox-slice-state:${sliceName}-*`))
  rollbackImagesBefore = new Set(await listDockerImageRefs("chariox-slice-backup:restore-rollback-*"))
  resources.push(await resourceSnapshot("before"))
  log("writing disposable config")
  await seedConfig()
  log("starting local webmail fixture")
  fixture = await startFixture()
  assert.equal(Number(new URL(fixture.origin).port), fixturePort)
  await assertFixtureAlive()
  log(`fixture listening on ${fixturePort}`)

  log("building kernel")
  const kernel = await buildKernel()
  sourceIdentity = await captureSourceIdentity(kernel)
  log("building kernel client")
  await buildKernelClient()
  log("starting disposable kernel")
  start("kernel", kernel, [], {
    env: {
      ...process.env,
      CHARIOX_HOME: path.join(tempRoot, "home"),
      XDG_CONFIG_HOME: path.join(tempRoot, "config"),
      CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT: "1",
      CHARIOX_KERNEL_PORT: String(kernelPort),
      CHARIOX_MCP_PORT: String(kernelPort + 1),
      CHARIOX_CODEX_PORT: String(kernelPort + 2),
      CHARIOX_OPENCODE_PORT: String(kernelPort + 3),
      CHARIOX_DAEMON_SOCKET: path.join(tempRoot, "daemon.sock"),
      CHARIOX_DAEMON_ID: `m20-daemon-${process.pid}`,
      CHARIOX_DAEMON_ALIAS: `m20-${process.pid}`,
      CHARIOX_SESSION_HISTORY_DIR: path.join(tempRoot, "session-history"),
      // Exact-head dev binaries keep the Docker-side link within laptop memory
      // while exercising the same slice lifecycle and runtime protocol.
      CHARIOX_SLICE_RUNTIME_BUILD_PROFILE: "dev",
    },
  })

  const imported = await Promise.all([
    import("../../../packages/kernel-client/dist/ipc.js"),
    import("../../../packages/kernel-client/dist/ipc-requests.js"),
  ])
  const [{ LocalIpcClient }, importedRequests] = imported
  requests = importedRequests
  log(`waiting for kernel ${kernelUrl}`)
  client = await waitFor(async () => {
    const candidate = new LocalIpcClient(kernelUrl)
    try {
      await candidate.send(requests.listSlicesRequest())
      return candidate
    } catch (error) {
      candidate.close?.()
      throw error
    }
  }, 60_000, "kernel did not accept local connections")

  log(`creating slice ${sliceName}`)
  slice = unwrap(await client.send(requests.createSliceRequest({
    name: sliceName,
    backend: "local_docker",
    displayMode: "headed",
    displayBackend: "selkies",
    workspaceMount: repoRoot,
    workerKernelRef: `m20-worker-${process.pid}`,
  })), "SliceCreated").slice
  log("starting slice")
  await client.send(requests.startSliceRequest(slice.id))
  slice = await waitForSliceRunning(slice.id)
  const initialSlicePorts = structuredClone(slice.local_docker_ports)
  const initialDisplayUrl = slice.display_endpoint?.url
  assert.ok(initialSlicePorts?.novnc, "slice must retain its allocated display port")
  assert.equal(
    initialDisplayUrl,
    `http://127.0.0.1:${initialSlicePorts.novnc}/`,
    "Selkies display endpoint must use the slice's durable port assignment",
  )
  sliceRuntime.initial = await inspectSliceRuntime()
  assert.match(
    sliceRuntime.initial.installedRuntimeSourceRevision,
    /^[a-f0-9]{64}$/,
    "initial slice must report an installed runtime source revision",
  )
  assert.equal(
    sliceRuntime.initial.runtimeSourceRevision,
    sliceRuntime.initial.installedRuntimeSourceRevision,
    "initial slice image and installed runtime revisions must match",
  )
  assertSliceResourceLimits(sliceRuntime.initial.containerLimits, "initial slice")
  log("slice is running")

  await writeFile(path.join(artifactDir, "container-before-save.inspect.json"), await dockerText(["inspect", containerName]))
  await inspectState("initial")
  persistenceIdentity.initial = await inspectPersistenceIdentity()
  assertInitialPersistenceIdentity(persistenceIdentity.initial)
  log("installing program marker")
  await installProgramMarker()
  log("running local browser state phase")
  await runLocalBrowserStatePhase("before")
  log("running first webmail phase")
  await runWebmailPhase("first", markers.firstSubject)
  log("creating browser download and application state")
  await seedUserPersistenceMarkers()
  await screenshot("01-before-save")

  log("saving slice state")
  const saved = unwrap(await client.send(requests.saveSliceStateRequest(slice.id, "shutdown")), "SliceStateSaved")
  assert.ok(saved.state?.id, "save-state should create a saved state record")
  savedState = saved.state
  await writeFile(path.join(artifactDir, "save-state-response.json"), JSON.stringify(saved, null, 2))
  log("removing container and home volume to force saved-state restore")
  await removeContainerAndHomeVolume()

  log("starting restored slice")
  await client.send(requests.startSliceRequest(slice.id))
  slice = await waitForSliceRunning(slice.id)
  assert.deepEqual(
    slice.local_docker_ports,
    initialSlicePorts,
    "restoring the same slice must retain every durable port assignment",
  )
  assert.equal(
    slice.display_endpoint?.url,
    initialDisplayUrl,
    "restoring the same slice must retain its display endpoint",
  )
  sliceRuntime.restored = await inspectSliceRuntime()
  assert.equal(
    sliceRuntime.restored.runtimeSourceRevision,
    sliceRuntime.restored.installedRuntimeSourceRevision,
    "restored slice image and installed runtime revisions must match",
  )
  assert.equal(
    sliceRuntime.restored.installedRuntimeSourceRevision,
    sliceRuntime.initial.installedRuntimeSourceRevision,
    "restored slice must retain the same installed runtime source revision",
  )
  assertSliceResourceLimits(sliceRuntime.restored.containerLimits, "restored slice")
  log("restored slice is running")
  await writeFile(path.join(artifactDir, "container-after-restore.inspect.json"), await dockerText(["inspect", containerName]))
  await inspectState("restored")
  persistenceIdentity.restored = await inspectPersistenceIdentity()
  assert.deepEqual(
    persistenceIdentity.restored,
    persistenceIdentity.initial,
    "slice identity, display, browser profile, and password-store policy should survive restore",
  )
  log("verifying program marker")
  await verifyProgramMarker()
  log("verifying downloaded file and application state after restore")
  await verifyUserPersistenceMarkers()
  log("verifying local browser state after restore")
  await verifyLocalBrowserStateAfterRestore()
  log("running second webmail phase after restore")
  await runWebmailPhase("second", markers.secondSubject, { expectAuthenticated: true })
  await screenshot("02-after-restore-second-send")

  assert.equal(fixture.messages.filter((message) => message.subject === markers.firstSubject).length, 1)
  assert.equal(fixture.messages.filter((message) => message.subject === markers.secondSubject).length, 1)
  await writeFile(path.join(artifactDir, "fixture-messages.json"), JSON.stringify(fixture.messages, null, 2))

  log("creating immutable named browser-state backup")
  const namedBackupResult = unwrap(
    await client.send(requests.createSliceBackupRequest(slice.id, "browser-baseline")),
    "SliceBackupCreated",
  )
  namedBackup = namedBackupResult.backup
  assert.match(namedBackup.home_archive_sha256 ?? "", /^[a-f0-9]{64}$/)
  assert.match(namedBackup.image_id ?? "", /^sha256:/)
  assert.equal(namedBackupResult.slice.status, "running")
  await writeFile(
    path.join(artifactDir, "named-backup-response.json"),
    JSON.stringify(namedBackupResult, null, 2),
  )
  await waitForBrowserText(
    "CHARIOX_FIXTURE_MESSAGE_SENT",
    30_000,
    "browser did not resume after live backup capture",
  )
  assert.equal(
    fixture.messages.filter((message) => message.subject === markers.secondSubject).length,
    1,
    "browser resume must not repeat the mail POST",
  )

  log("creating a corrupt candidate without mutating the known-good backup")
  await writeSliceFile("/home/slice/.config/m20-state-app/config.txt", `${markers.backupMutationOne}\n`)
  const corruptBackupResult = unwrap(
    await client.send(requests.createSliceBackupRequest(slice.id, "corrupt-candidate")),
    "SliceBackupCreated",
  )
  corruptBackup = corruptBackupResult.backup
  assert.equal(corruptBackupResult.slice.status, "running")
  await client.send(requests.stopSliceRequest(slice.id))
  slice = await waitForSliceStatus(slice.id, "stopped")
  const containerBeforeRejectedRestore = await inspectContainerId()
  await writeFile(corruptBackup.home_archive_path, "deliberately corrupted backup archive")
  await assert.rejects(
    client.send(requests.restoreSliceBackupRequest(slice.id, corruptBackup.id)),
    /archive integrity check failed.*quarantined/,
  )
  slice = await waitForSliceStatus(slice.id, "stopped")
  assert.equal(
    await inspectContainerId(),
    containerBeforeRejectedRestore,
    "corrupt backup rejection must happen before container replacement",
  )
  assert.equal(
    await access(corruptBackup.home_archive_path).then(() => true, () => false),
    false,
    "the corrupt archive must leave its restore path",
  )
  assert.equal(
    (await readdir(path.dirname(corruptBackup.home_archive_path)))
      .filter((entry) => entry.startsWith(`${path.basename(corruptBackup.home_archive_path)}.corrupt-`))
      .length,
    1,
    "the corrupt archive must remain in one owned quarantine file",
  )

  log("restoring the named backup by its human-readable name")
  const firstBackupRestore = unwrap(
    await client.send(requests.restoreSliceBackupRequest(slice.id, "browser-baseline")),
    "SliceBackupRestored",
  )
  assert.equal(firstBackupRestore.backup.id, namedBackup.id)
  assert.equal(firstBackupRestore.slice.status, "stopped")
  await client.send(requests.startSliceRequest(slice.id))
  slice = await waitForSliceRunning(slice.id)
  await verifyUserPersistenceMarkers()
  await verifyLocalBrowserStateAfterRestore()
  await screenshot("04-after-named-backup-restore")

  log("restoring the same immutable backup again by id")
  await writeSliceFile("/home/slice/.config/m20-state-app/config.txt", `${markers.backupMutationTwo}\n`)
  await client.send(requests.stopSliceRequest(slice.id))
  slice = await waitForSliceStatus(slice.id, "stopped")
  const secondBackupRestore = unwrap(
    await client.send(requests.restoreSliceBackupRequest(slice.id, namedBackup.id)),
    "SliceBackupRestored",
  )
  assert.equal(secondBackupRestore.backup.id, namedBackup.id)
  assert.equal(secondBackupRestore.slice.status, "stopped")
  await client.send(requests.startSliceRequest(slice.id))
  slice = await waitForSliceRunning(slice.id)
  await verifyUserPersistenceMarkers()
  await verifyLocalBrowserStateAfterRestore()
  await screenshot("05-after-repeated-backup-restore")

  log("invalidating the external service session without changing browser state")
  await verifyExternalServiceReauthentication()

  log("verifying restored service worker serves cached content while the fixture is offline")
  await fixture.close()
  await sliceScreen(["open-url", fixtureUrl("/offline-marker")])
  await waitForBrowserText(
    "CHARIOX_FIXTURE_OFFLINE_MARKER",
    30_000,
    "restored service worker did not serve the cached offline marker",
  )
  await screenshot("03-service-worker-offline-after-restore")
  resources.push(await resourceSnapshot("restored-active"))
}

async function seedConfig() {
  const configDir = path.join(tempRoot, "home")
  await mkdir(configDir, { recursive: true })
  await writeFile(path.join(configDir, "config.toml"), [
    "version = 1",
    "",
    "[credential_vault]",
    "backend = \"process_memory\"",
    `service = "chariox-${runId}"`,
    "agent_management = \"allow\"",
    "",
    "[state]",
    `path = "${path.join(tempRoot, "state.db").replaceAll("\\", "\\\\")}"`,
    "",
    "[slices]",
    `root = "${path.join(tempRoot, "slices").replaceAll("\\", "\\\\")}"`,
    "",
    "[slices.linux]",
    ...browserStateDrillImageConfig(process.env),
    "screen_width = 1280",
    "screen_height = 800",
    "memory_mb = 2048",
    "cpus = \"1.0\"",
    "",
  ].join("\n"))
}

async function startFixture() {
  return await startBrowserComputerFixture({
    host: "0.0.0.0",
    port: fixturePort,
    account: email,
    password,
  })
}

async function runLocalBrowserStatePhase(label) {
  await assertFixtureAlive()
  const query = new URLSearchParams({
    cookie: markers.stateCookie,
    local: markers.stateLocalStorage,
    idb: markers.stateIndexedDb,
    cache: markers.stateCacheStorage,
  })
  await sliceScreen(["open-url", fixtureUrl(`/state/seed?${query}`)])
  await waitForBrowserText(
    "CHARIOX_FIXTURE_STATE_SEEDED",
    30_000,
    `${label} state seed did not complete`,
  )
  await screenshot(`state-seeded-${label}`)
}

async function verifyLocalBrowserStateAfterRestore() {
  await assertFixtureAlive()
  await sliceScreen(["open-url", fixtureUrl("/state/check")])
  const text = await waitForBrowserText(markers.stateIndexedDb, 30_000, "browser persisted state not visible after restore")
  assert.match(text, new RegExp(escapeRegExp(markers.stateCookie)), "cookie marker should persist")
  assert.match(text, new RegExp(escapeRegExp(markers.stateLocalStorage)), "localStorage marker should persist")
  assert.match(text, new RegExp(escapeRegExp(markers.stateIndexedDb)), "IndexedDB marker should persist")
  assert.match(text, new RegExp(escapeRegExp(markers.stateCacheStorage)), "Cache Storage marker should persist")
  assert.match(text, /"serviceWorker":true/, "service-worker registration should persist")
  await screenshot("state-check-after-restore")
}

async function runWebmailPhase(label, subject, options = {}) {
  await assertFixtureAlive()
  await sliceScreen(["open-url", fixtureUrl(options.expectAuthenticated ? "/mail/inbox" : "/mail/login")])
  const pageText = await waitForBrowserText(
    options.expectAuthenticated ? "CHARIOX_FIXTURE_INBOX" : "Fixture mail login",
    30_000,
    `${label} webmail did not open`,
  )
  if (options.expectAuthenticated) {
    assert.ok(!pageText.includes("Fixture mail login"), "restored browser should not return to login page")
  } else {
    await sliceScreen(["browser-fill", "#email", email])
    await sliceScreenWithStdin(["secret-paste-stdin", "#password"], password)
    await sliceScreen(["browser-submit", "#password"])
    await waitForBrowserText("CHARIOX_FIXTURE_INBOX", 30_000, "webmail login did not reach inbox")
  }
  await sliceScreen(["browser-click", "#compose"])
  await waitForBrowserText("Fixture compose", 30_000, `${label} compose did not open`)
  await sliceScreen(["browser-fill", "#to", recipient])
  await sliceScreen(["browser-fill", "#subject", subject])
  await sliceScreen(["browser-fill", "#body", `${label} message sent from restored Docker slice drill`])
  await sliceScreen(["browser-click", "#send"])
  await waitForBrowserText("CHARIOX_FIXTURE_MESSAGE_SENT", 30_000, `${label} message was not sent`)
  await screenshot(`webmail-${label}-sent`)
}

async function verifyExternalServiceReauthentication() {
  externalServiceReauthentication.invalidatedSessionCount = fixture.invalidateSessions()
  assert.equal(
    externalServiceReauthentication.invalidatedSessionCount,
    1,
    "the fixture should invalidate the one browser-authenticated service session",
  )

  await sliceScreen(["open-url", fixtureUrl("/mail/inbox")])
  await waitForBrowserText(
    "Fixture mail login",
    30_000,
    "external service invalidation did not return the browser to login",
  )
  externalServiceReauthentication.classification = "external_service_reauth"
  externalServiceReauthentication.exactPrompt = "Fixture mail login"
  await screenshot("06-external-service-reauth")

  await verifyLocalBrowserStateAfterRestore()
  externalServiceReauthentication.browserStatePreserved = true

  await runWebmailPhase("external-reauth", markers.reauthSubject)
  assert.equal(
    fixture.messages.filter((message) => message.subject === markers.reauthSubject).length,
    1,
    "the product should remain usable after external service reauthentication",
  )
  externalServiceReauthentication.recovered = true
  await writeFile(path.join(artifactDir, "fixture-messages.json"), JSON.stringify(fixture.messages, null, 2))
}

async function installProgramMarker() {
  await docker(["exec", "-u", "root", containerName, "bash", "-lc", "printf '#!/usr/bin/env bash\\necho M20_PROGRAM_SURVIVED\\n' >/usr/local/bin/m20-state-tool && chmod +x /usr/local/bin/m20-state-tool"])
}

async function verifyProgramMarker() {
  const output = await dockerText(["exec", containerName, "m20-state-tool"])
  assert.match(output, /M20_PROGRAM_SURVIVED/)
}

async function seedUserPersistenceMarkers() {
  await writeSliceFile("/home/slice/.config/m20-state-app/config.txt", `${markers.appConfig}\n`)
  await writeSliceFile("/home/slice/.local/share/m20-state-app/user-data.txt", `${markers.appUserData}\n`)

  await sliceScreen(["open-url", fixtureUrl("/interactions")])
  await waitForBrowserText("Fixture interactions", 30_000, "download fixture did not open")
  await sliceScreen(["browser-click", "#download"])
  const downloaded = await waitFor(
    async () => await readSliceFile("/home/slice/Downloads/chariox-fixture.txt").catch(() => false),
    30_000,
    "browser download did not complete",
  )
  assert.equal(downloaded.trim(), markers.downloadedFile)
}

async function verifyUserPersistenceMarkers() {
  assert.equal(
    (await readSliceFile("/home/slice/Downloads/chariox-fixture.txt")).trim(),
    markers.downloadedFile,
    "browser download should survive saved-state restore",
  )
  assert.equal(
    (await readSliceFile("/home/slice/.config/m20-state-app/config.txt")).trim(),
    markers.appConfig,
    "application configuration should survive saved-state restore",
  )
  assert.equal(
    (await readSliceFile("/home/slice/.local/share/m20-state-app/user-data.txt")).trim(),
    markers.appUserData,
    "application user data should survive saved-state restore",
  )
}

async function readSliceFile(filePath) {
  return await dockerText(["exec", "-u", "slice", containerName, "cat", filePath])
}

async function writeSliceFile(filePath, contents) {
  await docker(["exec", "-u", "slice", containerName, "mkdir", "-p", path.posix.dirname(filePath)])
  await dockerText(["exec", "-i", "-u", "slice", containerName, "tee", filePath], { stdin: contents })
}

async function inspectPersistenceIdentity() {
  // A running headed slice is not ready until slice-screen.sh has started and
  // health-checked Chromium, so absence here is a lifecycle regression rather
  // than a lazy-browser state the persistence drill should tolerate.
  const [user, uid, gid, home, hostname, machineId, dbusMachineId, displayBackend, screenStatus, chromiumCommand] = await Promise.all([
    dockerText(["exec", "-u", "slice", containerName, "id", "-un"]),
    dockerText(["exec", "-u", "slice", containerName, "id", "-u"]),
    dockerText(["exec", "-u", "slice", containerName, "id", "-g"]),
    dockerText(["exec", "-u", "slice", containerName, "sh", "-lc", "printf %s \"$HOME\""]),
    dockerText(["exec", containerName, "hostname"]),
    dockerText(["exec", containerName, "cat", "/etc/machine-id"]),
    dockerText(["exec", containerName, "cat", "/var/lib/dbus/machine-id"]),
    dockerText(["exec", containerName, "printenv", "CHARIOX_SLICE_VIEWER_BACKEND"]),
    sliceScreen(["status"]),
    dockerText([
      "exec",
      containerName,
      "bash",
      "-lc",
      "pid=$(pgrep -o -f '^/usr/lib/chromium/chromium .*--user-data-dir=/home/slice/.chariox/browser/chromium' || true); test -n \"$pid\"; tr '\\0' ' ' </proc/$pid/cmdline",
    ]),
  ])
  const display = Object.fromEntries(
    screenStatus.trim().split("\n").map((line) => {
      const separator = line.indexOf("=")
      return separator < 0 ? [line, ""] : [line.slice(0, separator), line.slice(separator + 1)]
    }),
  )
  return {
    user: user.trim(),
    uid: Number(uid.trim()),
    gid: Number(gid.trim()),
    home: home.trim(),
    hostname: hostname.trim(),
    machineId: machineId.trim(),
    dbusMachineId: dbusMachineId.trim(),
    displayMode: display.mode,
    screen: display.screen,
    displayBackend: displayBackend.trim(),
    viewerUrl: display.viewer,
    browserProfile: chromiumCommand.includes("--user-data-dir=/home/slice/.chariox/browser/chromium")
      ? "/home/slice/.chariox/browser/chromium"
      : null,
    passwordStore: chromiumCommand.includes("--password-store=basic") ? "basic" : null,
  }
}

function assertInitialPersistenceIdentity(identity) {
  assert.equal(identity.user, "slice")
  assert.equal(identity.uid, 1001)
  assert.equal(identity.gid, 1001)
  assert.equal(identity.home, "/home/slice")
  assert.match(identity.hostname, /^chariox-slice-m20-/)
  assert.match(identity.machineId, /^[a-f0-9]{32}$/)
  assert.equal(identity.dbusMachineId, identity.machineId)
  assert.equal(identity.displayMode, "headed")
  assert.equal(identity.screen, "1280x800")
  assert.equal(identity.displayBackend, "selkies")
  assert.match(identity.viewerUrl, /^http:\/\/127\.0\.0\.1:\d+\/$/)
  assert.equal(identity.browserProfile, "/home/slice/.chariox/browser/chromium")
  assert.equal(identity.passwordStore, "basic")
}

async function inspectState(label) {
  const script = `
    set -euo pipefail
    echo '--- container'
    id
    hostname
    echo '--- machine-id'
    cat /etc/machine-id || true
    cat /var/lib/dbus/machine-id || true
    echo '--- chrome-profile'
    find /home/slice/.chariox/browser/chromium -maxdepth 2 -type f 2>/dev/null | sed 's#^#/##' | head -80 || true
    echo '--- mounts'
    mount | grep -E '/home/slice|/workspace' || true
  `
  await writeFile(path.join(artifactDir, `inspection-${label}.txt`), await dockerText(["exec", containerName, "bash", "-lc", script]))
}

async function screenshot(name) {
  const inside = `/tmp/${name}.png`
  await sliceScreen(["screenshot", inside])
  const outside = path.join(artifactDir, `${name}.png`)
  await docker(["cp", `${containerName}:${inside}`, outside])
  screenshots[name] = outside
}

async function waitForBrowserText(needle, timeoutMs, message) {
  return await waitFor(async () => {
    const text = await sliceScreen(["browser-text"]).catch(() => "")
    return text.includes(needle) ? text : false
  }, timeoutMs, message)
}

async function waitForSliceRunning(sliceRef) {
  return await waitForSliceStatus(sliceRef, "running", 240_000)
}

async function waitForSliceStatus(sliceRef, status, timeoutMs = 120_000) {
  return await waitFor(async () => {
    const current = unwrap(await client.send(requests.getSliceRequest(sliceRef)), "Slice").slice
    return current.status === status ? current : false
  }, timeoutMs, `slice ${sliceRef} did not become ${status}`)
}

async function inspectContainerId() {
  const inspected = JSON.parse((await docker(["container", "inspect", containerName])).stdout)
  assert.ok(inspected[0]?.Id, "slice container should have an inspectable id")
  return inspected[0].Id
}

async function listDockerImageRefs(reference) {
  const result = await docker([
    "image",
    "ls",
    "--filter",
    `reference=${reference}`,
    "--format",
    "{{.Repository}}:{{.Tag}}",
  ])
  return result.stdout.split(/\r?\n/).map((value) => value.trim()).filter(Boolean)
}

async function assertFixtureAlive() {
  const response = await fetch(`http://127.0.0.1:${fixturePort}/mail/login`, {
    signal: AbortSignal.timeout(2_000),
  })
  assert.ok(response.ok, `fixture health returned HTTP ${response.status}`)
}

async function removeContainerAndHomeVolume() {
  await docker(["rm", "-f", containerName]).catch(() => undefined)
  await docker(["volume", "rm", "-f", homeVolume]).catch(() => undefined)
}

async function buildKernel() {
  const manifest = path.join(repoRoot, "apps/kernel/Cargo.toml")
  const binary = path.join(repoRoot, "apps/kernel/target/debug/chariox-kernel")
  const result = await runCommand("cargo", ["build", "--manifest-path", manifest, "--bin", "chariox-kernel"], { timeoutMs: 180_000 })
  if (result.code !== 0) throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  return await resolveBuiltBinary(binary, manifest, "chariox-kernel")
}

async function buildKernelClient() {
  const result = await runCommand("pnpm", ["--workspace-root", "run", "build:kernel-client"], { timeoutMs: 180_000 })
  if (result.code !== 0) throw new Error(`kernel client build failed\n${result.stdout}\n${result.stderr}`)
}

async function captureSourceIdentity(kernelBinary) {
  const [commit, trackedStatus, kernelHash] = await Promise.all([
    runCommand("git", ["rev-parse", "HEAD"], { timeoutMs: 10_000 }),
    runCommand("git", ["status", "--porcelain", "--untracked-files=no"], { timeoutMs: 10_000 }),
    runCommand("shasum", ["-a", "256", kernelBinary], { timeoutMs: 20_000 }),
  ])
  assert.equal(commit.code, 0, `git rev-parse failed: ${commit.stderr}`)
  assert.equal(trackedStatus.code, 0, `git status failed: ${trackedStatus.stderr}`)
  assert.equal(kernelHash.code, 0, `kernel hash failed: ${kernelHash.stderr}`)
  return {
    gitCommit: commit.stdout.trim(),
    trackedWorktreeClean: trackedStatus.stdout.trim().length === 0,
    kernelBinary,
    kernelSha256: kernelHash.stdout.trim().split(/\s+/)[0],
  }
}

async function inspectSliceRuntime() {
  const container = JSON.parse((await docker(["container", "inspect", containerName])).stdout)[0]
  const image = JSON.parse((await docker(["image", "inspect", container.Image])).stdout)[0]
  const labels = image?.Config?.Labels ?? {}
  const installedRevision = await dockerText([
    "exec",
    "-u",
    "slice",
    containerName,
    "cat",
    "/opt/chariox-slice/runtime-source-revision",
  ])
  const kernelHash = await dockerText([
    "exec",
    "-u",
    "slice",
    containerName,
    "sha256sum",
    "/opt/chariox-slice/bin/chariox-kernel",
  ])
  return {
    imageId: container.Image,
    imageTag: container.Config?.Image ?? null,
    runtimeSourceRevision: labels["io.chariox.runtime-source-revision"] ?? null,
    installedRuntimeSourceRevision: installedRevision.trim(),
    relayPeerProtocolVersion: labels["io.chariox.relay-peer-protocol-version"] ?? null,
    workerKernelSha256: kernelHash.trim().split(/\s+/)[0],
    containerLimits: {
      memoryBytes: container.HostConfig?.Memory ?? null,
      memorySwapBytes: container.HostConfig?.MemorySwap ?? null,
      nanoCpus: container.HostConfig?.NanoCpus ?? null,
    },
  }
}

function assertSliceResourceLimits(limits, label) {
  assert.equal(limits.memoryBytes, 2048 * 1024 * 1024, `${label} memory limit`)
  assert.equal(limits.memorySwapBytes, limits.memoryBytes, `${label} swap must not exceed memory`)
  assert.equal(limits.nanoCpus, 1_000_000_000, `${label} CPU limit`)
}

async function resourceSnapshot(label) {
  const [pressure, swap, disk, container] = await Promise.all([
    runCommand("memory_pressure", [], { timeoutMs: 10_000 }).catch(() => null),
    runCommand("sysctl", ["vm.swapusage"], { timeoutMs: 10_000 }).catch(() => null),
    runCommand("df", ["-k", os.homedir()], { timeoutMs: 10_000 }).catch(() => null),
    runCommand(
      "docker",
      ["stats", "--no-stream", "--format", "{{json .}}", containerName],
      { timeoutMs: 20_000 },
    ).catch(() => null),
  ])
  return {
    label,
    at: new Date().toISOString(),
    freeMemoryBytes: os.freemem(),
    memoryPressure: pressure?.code === 0 ? pressure.stdout.trim() : null,
    swapUsage: swap?.code === 0 ? swap.stdout.trim() : null,
    disk: disk?.code === 0 ? disk.stdout.trim().split("\n").at(-1) : null,
    dockerStats: container?.code === 0 ? container.stdout.trim() : null,
  }
}

async function assertDockerReady() {
  const result = await runCommand("docker", ["info", "--format", "{{json .ServerVersion}}"], { timeoutMs: 20_000 })
  if (result.code !== 0) throw new Error(`Docker is required for M20 drill.\n${result.stdout}${result.stderr}`)
}

function start(label, command, args, options = {}) {
  const child = spawn(command, args, {
    cwd: options.cwd ?? repoRoot,
    env: options.env ?? process.env,
    stdio: ["ignore", "pipe", "pipe"],
  })
  children.push(child)
  child.stdout.on("data", (chunk) => process.stdout.write(`[${label}] ${chunk}`))
  child.stderr.on("data", (chunk) => process.stderr.write(`[${label}] ${chunk}`))
  child.on("exit", (code, signal) => console.log(`[${label}] exit code=${code} signal=${signal ?? "none"}`))
}

async function sliceScreen(args) {
  return await dockerText(["exec", "-u", "slice", containerName, "/opt/chariox-slice/slice-screen.sh", ...args])
}

async function sliceScreenWithStdin(args, stdin) {
  return await dockerText(["exec", "-i", "-u", "slice", containerName, "/opt/chariox-slice/slice-screen.sh", ...args], { stdin })
}

async function docker(args) {
  const result = await runCommand("docker", args, { timeoutMs: 120_000 })
  if (result.code !== 0) throw new Error(`docker ${args.join(" ")} failed\n${result.stdout}\n${result.stderr}`)
  return result
}

async function dockerText(args, options = {}) {
  const result = await runCommand("docker", args, { ...options, timeoutMs: options.timeoutMs ?? 120_000 })
  if (result.code !== 0) throw new Error(`docker ${args.join(" ")} failed\n${result.stdout}\n${result.stderr}`)
  return `${result.stdout}${result.stderr}`
}

async function runCommand(command, args, options = {}) {
  return await new Promise((resolve, reject) => {
    let settled = false
    const child = spawn(command, args, {
      cwd: options.cwd ?? repoRoot,
      env: options.env ?? process.env,
      stdio: ["pipe", "pipe", "pipe"],
    })
    let stdout = ""
    let stderr = ""
    let timeout = null
    if (options.timeoutMs) {
      timeout = setTimeout(() => {
        if (settled) return
        stderr += `\n[timed out after ${options.timeoutMs}ms: ${command} ${args.join(" ")}]\n`
        child.kill("SIGTERM")
        setTimeout(() => {
          if (!settled) child.kill("SIGKILL")
        }, 2_000).unref()
      }, options.timeoutMs)
      timeout.unref()
    }
    child.stdout.on("data", (chunk) => { stdout += chunk.toString() })
    child.stderr.on("data", (chunk) => { stderr += chunk.toString() })
    child.on("error", reject)
    child.on("close", (code, signal) => {
      settled = true
      if (timeout) clearTimeout(timeout)
      resolve({ code, signal, stdout, stderr })
    })
    if (options.stdin != null) {
      child.stdin.end(options.stdin)
    } else {
      child.stdin.end()
    }
  })
}

async function cleanup() {
  if (client && requests && slice) {
    if (savedState) {
      await client.send(requests.resetSliceStateRequest(slice.id)).catch(() => undefined)
    }
    await client.send(requests.deleteSliceRequest(slice.id)).catch(() => undefined)
  }
  await removeContainerAndHomeVolume().catch(() => undefined)
  if (savedState?.image_ref) {
    await docker(["image", "rm", "-f", savedState.image_ref]).catch(() => undefined)
  }
  for (const backup of [namedBackup, corruptBackup].filter(Boolean)) {
    await docker(["image", "rm", "-f", backup.image_ref]).catch(() => undefined)
  }
  const stateImageLeaks = (await listDockerImageRefs(`chariox-slice-state:${sliceName}-*`).catch(() => []))
    .filter((imageRef) => !stateImagesBefore.has(imageRef))
  const rollbackImageLeaks = (await listDockerImageRefs("chariox-slice-backup:restore-rollback-*").catch(() => []))
    .filter((imageRef) => !rollbackImagesBefore.has(imageRef))
  for (const imageRef of [...stateImageLeaks, ...rollbackImageLeaks]) {
    await docker(["image", "rm", "-f", imageRef]).catch(() => undefined)
  }
  client?.close?.()
  await closeFixtureServer()
  for (const child of children.toReversed()) {
    await terminateChild(child)
  }
  await rm(tempRoot, { recursive: true, force: true })

  const dockerAvailable = (await runCommand("docker", ["info", "--format", "{{json .ServerVersion}}"], { timeoutMs: 20_000 })).code === 0
  const containerGone = (await runCommand("docker", ["container", "inspect", containerName], { timeoutMs: 20_000 })).code !== 0
  const volumeGone = (await runCommand("docker", ["volume", "inspect", homeVolume], { timeoutMs: 20_000 })).code !== 0
  const knownSavedImageGone = savedState?.image_ref
    ? (await runCommand("docker", ["image", "inspect", savedState.image_ref], { timeoutMs: 20_000 })).code !== 0
    : true
  const knownBackupImagesGone = await Promise.all(
    [namedBackup, corruptBackup].filter(Boolean).map(async (backup) => (
      (await runCommand("docker", ["image", "inspect", backup.image_ref], { timeoutMs: 20_000 })).code !== 0
    )),
  ).then((values) => values.every(Boolean))
  const savedImageGone = knownSavedImageGone && stateImageLeaks.length === 0
  const backupImagesGone = knownBackupImagesGone && rollbackImageLeaks.length === 0
  const occupiedPorts = []
  for (const port of [kernelPort, kernelPort + 1, kernelPort + 2, kernelPort + 3, fixturePort].filter(Number.isInteger)) {
    if (!(await portIsAvailable(port))) occupiedPorts.push(port)
  }
  const result = {
    dockerAvailable,
    containerGone,
    volumeGone,
    savedImageGone,
    backupImagesGone,
    stateImageLeaks,
    rollbackImageLeaks,
    tempRootRemoved: await access(tempRoot).then(() => false).catch(() => true),
    listenersReleased: occupiedPorts.length === 0,
    occupiedPorts,
  }
  const afterResource = await resourceSnapshot("after-cleanup")
  resources.push(afterResource)
  result.resource = afterResource
  cleanupResult = result
  await writeFile(path.join(artifactDir, "cleanup.json"), `${JSON.stringify(result, null, 2)}\n`)
  const cleanupFailure = browserStateCleanupFailure(result)
  if (cleanupFailure) throw cleanupFailure
  return result
}

async function writeManifest(ok, error = null) {
  await writeFile(path.join(artifactDir, "manifest.json"), JSON.stringify({
    schema: "chariox.browser_computer.persistence_drill.v3",
    ok,
    startedAt,
    finishedAt: new Date().toISOString(),
    error: error ? String(error?.stack ?? error) : null,
    command: "pnpm --dir apps/cli browser-computer:persistence-drill",
    topology: "local kernel with one headed local Docker slice",
    source: sourceIdentity,
    sliceRuntime,
    persistenceIdentity,
    sliceName,
    containerName,
    homeVolume,
    fixturePort,
    markers,
    screenshots,
    externalServiceReauthentication,
    resources,
    assertions: [
      "initial and restored slices retained the 2 GiB memory, no-extra-swap, and one-CPU caps",
      "installed program survived committed-image restore",
      "browser download, application configuration, and application user data survived",
      "durable slice port assignments and the projected Selkies endpoint remained stable",
      "machine id, hostname, user, UID/GID, home, display, browser profile, and password-store policy remained stable",
      "cookie, localStorage, IndexedDB, Cache Storage, and service-worker registration survived",
      "restored service worker served cached content while the fixture was offline",
      "authenticated browser session survived complete container and home-volume removal",
      "message before save and message after restore were each submitted exactly once",
      "external service invalidation showed the exact login prompt while persisted browser state remained intact",
      "reauthentication restored service use and submitted its message exactly once",
      "named backup integrity metadata was recorded, a corrupt archive was quarantined before container replacement, and the named backup remained restorable",
      "the same immutable named backup restored browser and application state twice, once by name and once by id",
    ],
    cleanup: cleanupResult,
  }, null, 2))
}

async function closeFixtureServer() {
  await fixture?.close?.()
}

async function terminateChild(child) {
  if (!child || child.exitCode != null) return
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

async function portIsAvailable(port) {
  return await new Promise((resolve) => {
    const server = net.createServer()
    server.once("error", () => resolve(false))
    server.listen(port, "127.0.0.1", () => server.close(() => resolve(true)))
  })
}

function log(message) {
  console.log(`[m20-docker-state] ${message}`)
}

function fixtureUrl(pathname) {
  return `http://host.docker.internal:${fixturePort}${pathname}`
}

function unwrap(value, variant) {
  assert.ok(value && typeof value === "object" && variant in value, `expected ${variant}, got ${JSON.stringify(value)}`)
  return value[variant]
}

async function waitFor(predicate, timeoutMs, message) {
  const startedAt = Date.now()
  let lastError = null
  while (Date.now() - startedAt < timeoutMs) {
    try {
      const value = await predicate()
      if (value) return value
    } catch (error) {
      lastError = error
    }
    await sleep(500)
  }
  throw new Error(`${message}${lastError ? `: ${lastError.message}` : ""}`)
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

function escapeRegExp(value) {
  return String(value).replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
}
