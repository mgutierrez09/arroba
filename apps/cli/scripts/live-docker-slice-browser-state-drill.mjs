#!/usr/bin/env node
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import http from "node:http"
import { mkdir, rm, writeFile } from "node:fs/promises"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"
import {
  assertBrowserComputerEvidencePath,
  assertBrowserComputerPreflight,
  collectBrowserComputerResourceSnapshot,
  defaultBrowserComputerEvidenceDir,
  evaluateBrowserComputerCleanup,
  parseBrowserComputerByteBudget,
} from "./lib/browser-computer-drill-guard.mjs"
import { finalizeDrillArtifacts } from "./lib/drill-artifacts.mjs"
import { resolveBuiltBinary } from "./lib/drill-runtime-helpers.mjs"

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const stamp = new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z")
const runId = `m20-docker-state-${process.pid}-${stamp}`
const artifactDir = process.env.M20_ARTIFACT_DIR ?? defaultBrowserComputerEvidenceDir(runId)
assertBrowserComputerEvidencePath(artifactDir, [repoRoot])
const tempRoot = path.join(os.tmpdir(), runId)
const kernelPort = Number.parseInt(process.env.M20_KERNEL_PORT ?? "", 10) || 55000 + Math.floor(Math.random() * 2000)
const kernelUrl = `ws://127.0.0.1:${kernelPort}/kernel`
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
  firstSubject: `M20 first ${process.pid}`,
  secondSubject: `M20 second ${process.pid}`,
}
const screenshots = {}
const children = []
let client = null
let requests = null
let slice = null
let fixture = null
let fixturePort = null
let resourceBefore = null
let resourceAfter = null
let resourcePreflight = null
let cleanupResult = null
let failure = null

await mkdir(artifactDir, { recursive: true })
await mkdir(tempRoot, { recursive: true })

try {
  log("checking Docker")
  await assertDockerReady()
  log("recording local resources and checking the operation budget")
  resourceBefore = await collectBrowserComputerResourceSnapshot({
    runCommand,
    filesystemPath: artifactDir,
  })
  resourcePreflight = assertBrowserComputerPreflight(resourceBefore, {
    allowExistingHeadedSlices: process.env.M20_ALLOW_EXISTING_SLICES === "1",
    requiredMemoryBytes: parseBrowserComputerByteBudget(process.env.M20_REQUIRED_MEMORY_BYTES),
    requiredDiskBytes: parseBrowserComputerByteBudget(process.env.M20_REQUIRED_DISK_BYTES),
  })
  for (const warning of resourcePreflight.warnings) log(warning)
  await writeFile(path.join(artifactDir, "resources-before.json"), `${JSON.stringify({
    snapshot: resourceBefore,
    preflight: resourcePreflight,
  }, null, 2)}\n`)
  await run()
} catch (error) {
  failure = error
} finally {
  await cleanup().catch((error) => {
    failure ??= error
  })
  if (resourceBefore) {
    try {
      resourceAfter = await collectBrowserComputerResourceSnapshot({
        runCommand,
        filesystemPath: artifactDir,
      })
      cleanupResult = await evaluateBrowserComputerCleanup({
        before: resourceBefore,
        after: resourceAfter,
        ownedContainers: [containerName],
        ownedVolumes: [homeVolume],
        tempRoots: [tempRoot],
        childProcesses: children,
        allowRetainedResources: process.env.M20_KEEP_RESOURCES === "1",
      })
      await writeFile(path.join(artifactDir, "resources-after.json"), `${JSON.stringify({
        snapshot: resourceAfter,
        cleanup: cleanupResult,
      }, null, 2)}\n`)
      if (!cleanupResult.ok) {
        failure ??= new Error(`browser/computer drill cleanup failed:\n- ${cleanupResult.violations.join("\n- ")}`)
      }
    } catch (error) {
      failure ??= error
    }
  }
  await writeManifest(failure === null, failure)
}

if (failure) {
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
  console.log(`M20_DOCKER_SLICE_BROWSER_STATE_PASS ${JSON.stringify({ artifactDir, screenshots, markers, cleanup: cleanupResult })}`)
}

async function run() {
  log("writing disposable config")
  await seedConfig()
  log("starting local webmail fixture")
  fixture = await startFixture()
  fixturePort = fixture.port
  await assertFixtureAlive()
  log(`fixture listening on ${fixturePort}`)

  log("building kernel")
  const kernel = await buildKernel()
  log("building kernel client")
  await buildKernelClient()
  log("starting disposable kernel")
  start("kernel", kernel, [], {
    env: {
      ...process.env,
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
    workspaceMount: repoRoot,
    workerKernelRef: `m20-worker-${process.pid}`,
  })), "SliceCreated").slice
  log("starting slice")
  await client.send(requests.startSliceRequest(slice.id))
  slice = await waitForSliceRunning(slice.id)
  log("slice is running")

  await writeFile(path.join(artifactDir, "container-before-save.inspect.json"), await dockerText(["inspect", containerName]))
  await inspectState("initial")
  log("installing program marker")
  await installProgramMarker()
  log("running local browser state phase")
  await runLocalBrowserStatePhase("before")
  log("running first webmail phase")
  await runWebmailPhase("first", markers.firstSubject)
  await screenshot("01-before-save")

  log("saving slice state")
  const saved = unwrap(await client.send(requests.saveSliceStateRequest(slice.id, "shutdown")), "SliceStateSaved")
  assert.ok(saved.state?.id, "save-state should create a saved state record")
  await writeFile(path.join(artifactDir, "save-state-response.json"), JSON.stringify(saved, null, 2))
  log("removing container and home volume to force saved-state restore")
  await removeContainerAndHomeVolume()

  log("starting restored slice")
  await client.send(requests.startSliceRequest(slice.id))
  slice = await waitForSliceRunning(slice.id)
  log("restored slice is running")
  await writeFile(path.join(artifactDir, "container-after-restore.inspect.json"), await dockerText(["inspect", containerName]))
  await inspectState("restored")
  log("verifying program marker")
  await verifyProgramMarker()
  log("verifying local browser state after restore")
  await verifyLocalBrowserStateAfterRestore()
  log("running second webmail phase after restore")
  await runWebmailPhase("second", markers.secondSubject, { expectAuthenticated: true })
  await screenshot("02-after-restore-second-send")

  assert.equal(fixture.messages.filter((message) => message.subject === markers.firstSubject).length, 1)
  assert.equal(fixture.messages.filter((message) => message.subject === markers.secondSubject).length, 1)
  await writeFile(path.join(artifactDir, "fixture-messages.json"), JSON.stringify(fixture.messages, null, 2))
}

async function seedConfig() {
  const configDir = path.join(tempRoot, "config", "chariox")
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
    "build_image = \"auto\"",
    "screen_width = 1280",
    "screen_height = 800",
    "",
  ].join("\n"))
}

async function startFixture() {
  const sessions = new Map()
  const messages = []
  const server = http.createServer(async (request, response) => {
    const url = new URL(request.url ?? "/", `http://${request.headers.host}`)
    const cookies = parseCookies(request.headers.cookie ?? "")
    const send = (status, body, headers = {}) => {
      response.writeHead(status, { "content-type": "text/html; charset=utf-8", ...headers })
      response.end(body)
    }
    if (url.pathname === "/state") {
      send(200, statePage())
      return
    }
    if (url.pathname === "/state-check") {
      send(200, stateCheckPage())
      return
    }
    if (url.pathname === "/mail/login" && request.method === "GET") {
      send(200, loginPage())
      return
    }
    if (url.pathname === "/mail/login" && request.method === "POST") {
      const form = new URLSearchParams(await readRequestBody(request))
      if (form.get("email") !== email || form.get("password") !== password) {
        send(401, loginPage("Invalid credentials"))
        return
      }
      const sid = `sid-${process.pid}-${Date.now()}`
      sessions.set(sid, email)
      send(302, "", {
        "set-cookie": `m20_session=${sid}; Path=/; Max-Age=86400; HttpOnly; SameSite=Lax`,
        location: "/mail/inbox",
      })
      return
    }
    if (url.pathname === "/mail/inbox") {
      if (!sessions.has(cookies.m20_session)) {
        send(302, "", { location: "/mail/login" })
        return
      }
      send(200, inboxPage())
      return
    }
    if (url.pathname === "/mail/compose") {
      if (!sessions.has(cookies.m20_session)) {
        send(302, "", { location: "/mail/login" })
        return
      }
      send(200, composePage())
      return
    }
    if (url.pathname === "/mail/send" && request.method === "POST") {
      if (!sessions.has(cookies.m20_session)) {
        send(403, "not authenticated")
        return
      }
      const form = new URLSearchParams(await readRequestBody(request))
      messages.push({
        from: email,
        to: form.get("to"),
        subject: form.get("subject"),
        body: form.get("body"),
        sentAt: new Date().toISOString(),
      })
      send(200, sentPage(form.get("subject") ?? ""))
      return
    }
    if (url.pathname === "/api/messages") {
      response.writeHead(200, { "content-type": "application/json" })
      response.end(JSON.stringify({ messages }))
      return
    }
    send(404, "not found")
  })
  const port = await listen(server)
  return { server, port, messages }
}

function statePage() {
  return html("M20 state seed", `
    <h1>M20 state seed</h1>
    <p id="status">seeding</p>
    <script>
      const values = ${JSON.stringify(markers)};
      document.cookie = "m20_state_cookie=" + encodeURIComponent(values.stateCookie) + "; Path=/; Max-Age=86400; SameSite=Lax";
      localStorage.setItem("m20_state_local", values.stateLocalStorage);
      const request = indexedDB.open("m20_state_db", 1);
      request.onupgradeneeded = () => request.result.createObjectStore("state");
      request.onsuccess = () => {
        const tx = request.result.transaction("state", "readwrite");
        tx.objectStore("state").put(values.stateIndexedDb, "marker");
        tx.oncomplete = () => {
          document.querySelector("#status").textContent = "M20_STATE_SEEDED";
        };
      };
    </script>
  `)
}

function stateCheckPage() {
  return html("M20 state check", `
    <h1>M20 state check</h1>
    <pre id="result">checking</pre>
    <script>
      const cookie = document.cookie.split("; ").find((item) => item.startsWith("m20_state_cookie="))?.split("=")[1] || "";
      const request = indexedDB.open("m20_state_db", 1);
      request.onupgradeneeded = () => request.result.createObjectStore("state");
      request.onsuccess = () => {
        const tx = request.result.transaction("state", "readonly");
        const get = tx.objectStore("state").get("marker");
        get.onsuccess = () => {
          document.querySelector("#result").textContent = JSON.stringify({
            cookie: decodeURIComponent(cookie),
            localStorage: localStorage.getItem("m20_state_local"),
            indexedDb: get.result || null,
          });
        };
      };
    </script>
  `)
}

function loginPage(error = "") {
  return html("M20 webmail login", `
    <h1>M20 webmail login</h1>
    ${error ? `<p class="error">${escapeHtml(error)}</p>` : ""}
    <form method="post" action="/mail/login">
      <label>Email <input id="email" name="email" autocomplete="username"></label>
      <label>Password <input id="password" name="password" type="password" autocomplete="current-password"></label>
      <button id="login" type="submit">Sign in</button>
    </form>
  `)
}

function inboxPage() {
  return html("M20 webmail inbox", `
    <h1>M20_WEBMAIL_INBOX</h1>
    <p>Signed in as ${email}</p>
    <a id="compose" href="/mail/compose">Compose</a>
  `)
}

function composePage() {
  return html("M20 webmail compose", `
    <h1>M20 compose</h1>
    <form method="post" action="/mail/send">
      <label>To <input id="to" name="to"></label>
      <label>Subject <input id="subject" name="subject"></label>
      <label>Body <textarea id="body" name="body"></textarea></label>
      <button id="send" type="submit">Send</button>
    </form>
  `)
}

function sentPage(subject) {
  return html("M20 sent", `<h1>M20_MESSAGE_SENT</h1><p>${escapeHtml(subject)}</p><a id="inbox" href="/mail/inbox">Inbox</a>`)
}

function html(title, body) {
  return `<!doctype html><html><head><meta charset="utf-8"><title>${escapeHtml(title)}</title><style>
    body { font-family: system-ui, sans-serif; margin: 40px; max-width: 720px; }
    label { display: block; margin: 12px 0; }
    input, textarea { display: block; width: 520px; max-width: 100%; padding: 8px; }
    button, a { display: inline-block; margin-top: 12px; padding: 8px 12px; }
  </style></head><body>${body}</body></html>`
}

async function runLocalBrowserStatePhase(label) {
  await assertFixtureAlive()
  await sliceScreen(["open-url", fixtureUrl("/state")])
  await waitForBrowserText("M20_STATE_SEEDED", 30_000, `${label} state seed did not complete`)
  await screenshot(`state-seeded-${label}`)
}

async function verifyLocalBrowserStateAfterRestore() {
  await assertFixtureAlive()
  await sliceScreen(["open-url", fixtureUrl("/state-check")])
  const text = await waitForBrowserText(markers.stateIndexedDb, 30_000, "browser persisted state not visible after restore")
  assert.match(text, new RegExp(escapeRegExp(markers.stateCookie)), "cookie marker should persist")
  assert.match(text, new RegExp(escapeRegExp(markers.stateLocalStorage)), "localStorage marker should persist")
  assert.match(text, new RegExp(escapeRegExp(markers.stateIndexedDb)), "IndexedDB marker should persist")
  await screenshot("state-check-after-restore")
}

async function runWebmailPhase(label, subject, options = {}) {
  await assertFixtureAlive()
  await sliceScreen(["open-url", fixtureUrl(options.expectAuthenticated ? "/mail/inbox" : "/mail/login")])
  const pageText = await waitForBrowserText(options.expectAuthenticated ? "M20_WEBMAIL_INBOX" : "M20 webmail login", 30_000, `${label} webmail did not open`)
  if (options.expectAuthenticated) {
    assert.ok(!pageText.includes("M20 webmail login"), "restored browser should not return to login page")
  } else {
    await sliceScreen(["browser-fill", "#email", email])
    await sliceScreenWithStdin(["secret-paste-stdin", "#password"], password)
    await sliceScreen(["browser-submit", "#password"])
    await waitForBrowserText("M20_WEBMAIL_INBOX", 30_000, "webmail login did not reach inbox")
  }
  await sliceScreen(["browser-click", "#compose"])
  await waitForBrowserText("M20 compose", 30_000, `${label} compose did not open`)
  await sliceScreen(["browser-fill", "#to", recipient])
  await sliceScreen(["browser-fill", "#subject", subject])
  await sliceScreen(["browser-fill", "#body", `${label} message sent from restored Docker slice drill`])
  await sliceScreen(["browser-click", "#send"])
  await waitForBrowserText("M20_MESSAGE_SENT", 30_000, `${label} message was not sent`)
  await screenshot(`webmail-${label}-sent`)
}

async function installProgramMarker() {
  await docker(["exec", "-u", "root", containerName, "bash", "-lc", "printf '#!/usr/bin/env bash\\necho M20_PROGRAM_SURVIVED\\n' >/usr/local/bin/m20-state-tool && chmod +x /usr/local/bin/m20-state-tool"])
}

async function verifyProgramMarker() {
  const output = await dockerText(["exec", containerName, "m20-state-tool"])
  assert.match(output, /M20_PROGRAM_SURVIVED/)
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
    find /home/slice/.config/chariox-slice-chromium -maxdepth 2 -type f 2>/dev/null | sed 's#^#/##' | head -80 || true
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
  return await waitFor(async () => {
    const current = unwrap(await client.send(requests.getSliceRequest(sliceRef)), "Slice").slice
    return current.status === "running" ? current : false
  }, 240_000, `slice ${sliceRef} did not become running`)
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
  child.drillLabel = label
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
  if (process.env.M20_KEEP_RESOURCES !== "1") {
    if (client && requests && slice) {
      await client.send(requests.deleteSliceRequest(slice.id)).catch(() => undefined)
    }
    await removeContainerAndHomeVolume().catch(() => undefined)
    await rm(tempRoot, { recursive: true, force: true }).catch(() => undefined)
  }
  if (client?.close) await client.close().catch(() => undefined)
  await closeServer(fixture?.server)
  for (const child of children.toReversed()) await stopChild(child)
}

async function writeManifest(ok, error = null) {
  await writeFile(path.join(artifactDir, "manifest.json"), JSON.stringify({
    ok,
    error: error ? String(error?.stack ?? error) : null,
    sliceName,
    containerName,
    homeVolume,
    fixturePort,
    markers,
    screenshots,
    resourceBefore,
    resourceAfter,
    resourcePreflight,
    cleanupResult,
  }, null, 2))
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

function listen(server) {
  return new Promise((resolve, reject) => {
    server.once("error", reject)
    server.listen(0, "0.0.0.0", () => {
      const address = server.address()
      resolve(address.port)
    })
  })
}

async function readRequestBody(request) {
  const chunks = []
  for await (const chunk of request) chunks.push(Buffer.from(chunk))
  return Buffer.concat(chunks).toString("utf8")
}

function parseCookies(header) {
  return Object.fromEntries(header.split(";").map((part) => {
    const index = part.indexOf("=")
    if (index === -1) return null
    return [part.slice(0, index).trim(), decodeURIComponent(part.slice(index + 1))]
  }).filter(Boolean))
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

async function closeServer(server) {
  if (!server?.listening) return
  await new Promise((resolve) => server.close(resolve))
}

async function stopChild(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return
  child.kill("SIGTERM")
  await Promise.race([
    new Promise((resolve) => child.once("exit", resolve)),
    sleep(3_000),
  ])
  if (child.exitCode === null && child.signalCode === null) {
    child.kill("SIGKILL")
    await Promise.race([
      new Promise((resolve) => child.once("exit", resolve)),
      sleep(2_000),
    ])
  }
}

function escapeHtml(value) {
  return String(value).replace(/[&<>"']/g, (ch) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    '"': "&quot;",
    "'": "&#39;",
  })[ch])
}

function escapeRegExp(value) {
  return String(value).replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
}
