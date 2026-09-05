import { spawn } from 'node:child_process'
import { createHmac } from 'node:crypto'
import { mkdir, rm, symlink } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { finalizeDrillArtifacts } from './lib/drill-artifacts.mjs'
import {
  childTerminationStatus,
  makeAvailablePorts,
  resolveBuiltBinary,
} from './lib/drill-runtime-helpers.mjs'
import { sanitizeDrillMetadata } from './lib/drill-secrets.mjs'
import {
  collectPumpedTerminalOutput,
  terminalText,
} from './lib/relay-runtime-terminal-output.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

async function loadCliModules(runtimeDir) {
  const [{ transformAsync }, tsPreset] = await Promise.all([
    import('@babel/core'),
    import('@babel/preset-typescript'),
  ])
  for (const rel of ['src/ipc.ts', 'src/ipc-requests.ts']) {
    const sourcePath = path.join(cliRoot, rel)
    const outPath = path.join(runtimeDir, path.basename(rel).replace(/\.tsx?$/, '.js'))
    const code = await (await import('node:fs/promises')).readFile(sourcePath, 'utf8')
    const transformed = await transformAsync(code, {
      filename: sourcePath,
      presets: [[tsPreset.default ?? tsPreset]],
      sourceMaps: false,
    })
    await (await import('node:fs/promises')).writeFile(outPath, transformed?.code ?? '', 'utf8')
  }
  await symlink(path.join(cliRoot, 'node_modules'), path.join(runtimeDir, 'node_modules'), 'dir')
  const ipcUrl = pathToFileURL(path.join(runtimeDir, 'ipc.js')).href
  const requestsUrl = pathToFileURL(path.join(runtimeDir, 'ipc-requests.js')).href
  const { LocalIpcClient } = await import(ipcUrl)
  const requests = await import(requestsUrl)
  return { LocalIpcClient, requests }
}

let LocalIpcClient
let createSessionRequest
let attachToSessionRequest
let getSessionStateRequest
let listSessionsRequest
let launchProviderRunRequest
let submitPromptRequest
let pumpTerminalOutputRequest
let endSessionRequest

const DEFAULT_MODEL = 'kimi2.5'
const DEFAULT_PROVIDER = 'opencode'
const DEFAULT_WORKSPACE = repoRoot
const DEFAULT_WORKTREE = repoRoot
const DEFAULT_TIMEOUT_MS = 180_000
const DEFAULT_POLL_MS = 1_000
const RELAY_ISSUER = 'chariox-relay-runtime-drill'
const RELAY_SECRET = 'chariox-relay-runtime-drill-secret'
const RELAY_REALM = 'relay-runtime-drill'

function base64url(input) {
  return Buffer.from(input).toString('base64url')
}

function signRelayToken(claims) {
  const header = base64url(JSON.stringify({ alg: 'HS256', typ: 'JWT' }))
  const payload = base64url(JSON.stringify({
    iss: claims.issuer,
    sub: claims.subject,
    subject_kind: claims.subject_kind,
    realm_id: claims.realm_id,
    allowed_actions: claims.allowed_actions.map(relayActionName),
    allowed_targets: claims.allowed_targets,
    iat: Math.floor(claims.issued_at_ms / 1_000),
    exp: Math.ceil(claims.expires_at_ms / 1_000),
    jti: claims.token_id,
    account_id: claims.account_id,
    organization_id: claims.organization_id,
    user_id: claims.user_id,
    device_id: claims.device_id,
    machine_id: claims.machine_id,
    client_id: claims.client_id,
    public_key_thumbprint: claims.public_key_thumbprint,
    entitlements_version: claims.entitlements_version,
  }))
  const signingInput = `${header}.${payload}`
  const signature = createHmac('sha256', RELAY_SECRET).update(signingInput).digest('base64url')
  return `${signingInput}.${signature}`
}

function relayActionName(action) {
  const names = {
    daemon_register: 'daemon.register',
    daemon_heartbeat: 'daemon.heartbeat',
    client_metadata_read: 'client.metadata.read',
    client_connect: 'client.connect',
    packet_route: 'packet.route',
    peer_request: 'peer.request',
    peer_event: 'peer.event',
  }
  const name = names[action]
  if (!name) throw new Error(`unsupported relay action ${action}`)
  return name
}

function relayClaims({ subject, subjectKind, actions, userId = null, targets = null }) {
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
    account_id: 'relay-runtime-drill-account',
    organization_id: null,
    user_id: userId,
    device_id: subject,
    machine_id: subjectKind === 'kernel' || subjectKind === 'machine' ? subject : null,
    client_id: subjectKind === 'client' ? subject : null,
    // Local drill tokens are intentionally unbound. Production-issued tokens
    // bind this claim to the kernel's real relay key thumbprint; a fabricated
    // value makes the relay correctly reject the daemon registration.
    public_key_thumbprint: null,
    entitlements_version: 'drill',
  }
}

function parseArgs(argv) {
  const options = {
    model: DEFAULT_MODEL,
    provider: DEFAULT_PROVIDER,
    workspace: DEFAULT_WORKSPACE,
    worktree: DEFAULT_WORKTREE,
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
    dryRun: false,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--model') options.model = argv[++i]
    else if (arg === '--provider') options.provider = argv[++i]
    else if (arg === '--workspace') options.workspace = argv[++i]
    else if (arg === '--worktree') options.worktree = argv[++i]
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++i])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++i])
    else if (arg === '--dry-run') options.dryRun = true
    else if (arg === '--help') options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  return options
}

function printHelp() {
  console.log([
    'Usage: node apps/cli/scripts/live-relay-runtime-drill.mjs [options]',
    '',
    'Options:',
    `  --model ${DEFAULT_MODEL}`,
    `  --provider ${DEFAULT_PROVIDER}`,
    `  --workspace ${DEFAULT_WORKSPACE}`,
    `  --worktree ${DEFAULT_WORKTREE}`,
    `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
    `  --poll-ms ${DEFAULT_POLL_MS}`,
    '  --dry-run',
  ].join('\n'))
}

function makeChildrenEnv(ports, rootDir) {
  const daemonId = `relay-drill-daemon-${process.pid}-${Date.now()}`
  const daemonAlias = `relay-drill-${process.pid}`
  const daemonRelayToken = signRelayToken(relayClaims({
    subject: daemonId,
    subjectKind: 'kernel',
    actions: ['daemon_register', 'daemon_heartbeat', 'packet_route', 'peer_request', 'peer_event'],
    userId: 'local',
  }))
  const clientRelayToken = signRelayToken(relayClaims({
    subject: `relay-drill-client-${process.pid}`,
    subjectKind: 'client',
    actions: ['client_connect', 'client_metadata_read', 'packet_route'],
    userId: 'local',
  }))
  return {
    relayToken: clientRelayToken,
    daemonId,
    daemonAlias,
    relayEnv: {
      ...process.env,
      CHARIOX_RELAY_HOST: '127.0.0.1',
      CHARIOX_RELAY_PORT: String(ports.relayPort),
      CHARIOX_RELAY_SCOPED_ISSUER: RELAY_ISSUER,
      CHARIOX_RELAY_SCOPED_HMAC_SECRET: RELAY_SECRET,
    },
    daemonEnv: {
      ...process.env,
      CHARIOX_HOME: path.join(rootDir, 'home'),
      CHARIOX_LOG_DIR: path.join(rootDir, 'logs'),
      CHARIOX_KERNEL_PORT: String(ports.kernelPort),
      CHARIOX_MCP_PORT: String(ports.mcpPort),
      CHARIOX_OPENCODE_PORT: String(ports.openCodePort),
      CHARIOX_CODEX_PORT: String(ports.codexPort),
      CHARIOX_RELAY_URL: `ws://127.0.0.1:${ports.relayPort}`,
      CHARIOX_RELAY_TOKEN: daemonRelayToken,
      CHARIOX_DAEMON_ID: daemonId,
      CHARIOX_DAEMON_ALIAS: daemonAlias,
      CHARIOX_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
      CHARIOX_SESSION_HISTORY_DIR: path.join(rootDir, 'session-history'),
    },
  }
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const unwrap = (resp, key) => resp?.[key] ?? resp

function unwrapVariant(resp, ...keys) {
  for (const key of keys) {
    if (resp?.[key] != null) return resp[key]
  }
  return resp
}

function logStep(name, details = null) {
  if (details == null) console.log(`[relay-drill] ${name}`)
  else console.log(`[relay-drill] ${name}`, JSON.stringify(sanitizeDrillMetadata(details)))
}

function nowStamp() {
  return new Date().toISOString().replace(/[-:]/g, '').replace(/\.\d{3}Z$/, 'Z')
}

async function waitForLocalDaemon(kernelUrl, workspace, worktree, daemonChild) {
  for (let attempt = 0; attempt < 60; attempt += 1) {
    const probe = new LocalIpcClient(kernelUrl)
    try {
      const session = unwrap(await probe.send(createSessionRequest(workspace, worktree, undefined, undefined, null, 'off')), 'SessionCreated').session
      await probe.send(endSessionRequest(session.id)).catch(() => {})
      await probe.close()
      return
    } catch {
      await probe.close().catch(() => {})
      const termination = childTerminationStatus(daemonChild)
      if (termination) {
        const stderr = sanitizeDrillMetadata({ stderr: daemonChild.logs?.stderr?.slice(-2000) ?? '' }).stderr
        throw new Error(`local daemon exited before readiness with ${termination}: ${stderr}`)
      }
      await sleep(250)
    }
  }
  throw new Error('local daemon did not become ready')
}

async function waitForRelayTarget(relayUrl, relayToken, targetDaemonAlias) {
  let lastError = null
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const client = new LocalIpcClient(relayUrl, { relayAuthToken: relayToken, targetDaemonAlias })
    try {
      await Promise.race([
        client.send(listSessionsRequest()),
        sleep(2_000).then(() => { throw new Error('probe timeout') }),
      ])
      await client.close()
      return
    } catch (error) {
      lastError = error instanceof Error ? error.message : String(error)
      if ((attempt + 1) % 10 === 0) {
        logStep('relay_target_wait_retry', { attempt: attempt + 1, error: lastError })
      }
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`relay target daemon did not become reachable: ${lastError ?? 'unknown error'}`)
}

async function waitForEvent(predicate, bucket, timeoutMs, description) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    const match = bucket.find(predicate)
    if (match) return match
    await sleep(100)
  }
  throw new Error(`timed out waiting for ${description}`)
}

async function waitForCompletion(remoteClient, sessionId, attachmentId, eventBucket, timeoutMs, pollMs, baselineCount = 0) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    const completions = eventBucket.filter((event) => event.event === 'assistant_message_completed')
    if (completions.length > baselineCount) {
      return completions[completions.length - 1]
    }
    const response = await remoteClient.send(pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => null)
    collectPumpedTerminalOutput(response, eventBucket)
    await sleep(pollMs)
  }
  throw new Error('timed out waiting for assistant message completion')
}

async function waitForPromptEvidence(remoteClient, sessionId, attachmentId, eventBucket, options) {
  if (options.provider !== 'dev-stub') {
    return await waitForCompletion(
      remoteClient,
      sessionId,
      attachmentId,
      eventBucket,
      options.timeoutMs,
      options.pollMs,
      options.completionBaseline,
    )
  }
  const started = Date.now()
  while (Date.now() - started < options.timeoutMs) {
    if (terminalText(eventBucket).includes(options.expectedText)) {
      return { completed_at_ms: Date.now(), evidence: 'terminal_output' }
    }
    const response = await remoteClient.send(pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => null)
    collectPumpedTerminalOutput(response, eventBucket)
    await sleep(options.pollMs)
  }
  throw new Error(`timed out waiting for dev-stub terminal output ${options.expectedText}`)
}

function spawnProcess(command, args, options) {
  const child = spawn(command, args, { ...options, stdio: ['ignore', 'pipe', 'pipe'] })
  child.logs = { stdout: '', stderr: '' }
  child.stdout.on('data', (chunk) => { child.logs.stdout += chunk.toString() })
  child.stderr.on('data', (chunk) => { child.logs.stderr += chunk.toString() })
  return child
}

async function terminateChild(child, signal = 'SIGTERM') {
  if (!child || child.exitCode != null) return
  child.kill(signal)
  await Promise.race([
    new Promise((resolve) => child.once('exit', resolve)),
    sleep(5_000),
  ])
  if (child.exitCode == null) {
    child.kill('SIGKILL')
    await Promise.race([
      new Promise((resolve) => child.once('exit', resolve)),
      sleep(2_000),
    ])
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  if (options.dryRun) {
    console.log(JSON.stringify(options, null, 2))
    return
  }

  const ports = await makeAvailablePorts()
  const rootDir = path.join(
    os.homedir(),
    '.chariox',
    'dev',
    'browser-computer-use',
    'relay-runtime',
    nowStamp(),
  )
  await rm(rootDir, { recursive: true, force: true }).catch(() => {})
  await mkdir(rootDir, { recursive: true })
  const cliRuntimeDir = path.join(rootDir, 'cli-runtime')
  await rm(cliRuntimeDir, { recursive: true, force: true }).catch(() => {})
  await mkdir(cliRuntimeDir, { recursive: true })

  let relayChild = null
  let daemonChild = null
  let localClient = null
  let remoteClient = null
  let sessionId = null
  let eventLog = []
  let succeeded = false
  let failure = null
  let envs = null
  let kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  let relayUrl = `ws://127.0.0.1:${ports.relayPort}`

  try {
    ;({ LocalIpcClient, requests: {
      createSessionRequest,
      attachToSessionRequest,
      getSessionStateRequest,
      listSessionsRequest,
      launchProviderRunRequest,
      submitPromptRequest,
      pumpTerminalOutputRequest,
      endSessionRequest,
    } } = await loadCliModules(cliRuntimeDir))
    envs = makeChildrenEnv(ports, rootDir)
    const relayBinary = await resolveBuiltBinary(
      path.join(repoRoot, 'apps/relay/target/debug/chariox-relay'),
      path.join(repoRoot, 'apps/relay/Cargo.toml'),
      'chariox-relay',
    )
    const daemonBinary = await resolveBuiltBinary(
      path.join(repoRoot, 'apps/kernel/target/debug/chariox-kernel'),
      path.join(repoRoot, 'apps/kernel/Cargo.toml'),
      'chariox-kernel',
    )
    logStep('spawn_relay', { relayUrl, daemonAlias: envs.daemonAlias, relayToken: envs.relayToken })
    relayChild = spawnProcess(
      relayBinary,
      [],
      { cwd: repoRoot, env: envs.relayEnv },
    )

    daemonChild = spawnProcess(
      daemonBinary,
      [],
      { cwd: repoRoot, env: envs.daemonEnv },
    )

    await waitForLocalDaemon(kernelUrl, options.workspace, options.worktree, daemonChild)
    await waitForRelayTarget(relayUrl, envs.relayToken, envs.daemonAlias)

    localClient = new LocalIpcClient(kernelUrl)
    const created = unwrap(await localClient.send(createSessionRequest(options.workspace, options.worktree, undefined, undefined, null, 'off')), 'SessionCreated')
    sessionId = created.session.id
    const defaultAgentId = created.agent.id
    logStep('local_session_created', { sessionId, defaultAgentId })

    remoteClient = new LocalIpcClient(relayUrl, {
      relayAuthToken: envs.relayToken,
      targetDaemonAlias: envs.daemonAlias,
    })
    remoteClient.onKernelEvent((event) => {
      eventLog.push({ ...event, observed_at_ms: Date.now() })
    })

    const listed = unwrapVariant(await remoteClient.send(listSessionsRequest()), 'SessionsListed', 'Sessions')
    const sessionIds = (listed.sessions || []).map((session) => session.id)
    if (!sessionIds.includes(sessionId)) {
      throw new Error(`relay list_sessions did not include ${sessionId}`)
    }
    logStep('relay_list_sessions_ok', { count: sessionIds.length })

    const remoteStateBeforeAttach = unwrapVariant(await remoteClient.send(getSessionStateRequest(sessionId)), 'SessionStateLoaded', 'SessionState')
    const remoteSessionBeforeAttach = remoteStateBeforeAttach.session ?? remoteStateBeforeAttach.SessionStateLoaded?.session ?? null
    if (remoteSessionBeforeAttach?.id !== sessionId) {
      throw new Error(`remote get_session_state returned unexpected session: ${JSON.stringify(remoteStateBeforeAttach)}`)
    }
    logStep('relay_get_session_state_ok', { sessionId })

    const attachment = unwrap(
      await remoteClient.send(attachToSessionRequest(sessionId, `relay-drill-client-${Date.now()}`)),
      'SessionAttached',
    ).attachment
    logStep('relay_attach_ok', { attachmentId: attachment.id })

    await remoteClient.subscribeToKernelEvents(sessionId, attachment.id)
    await waitForEvent((event) => event.event === 'session_snapshot', eventLog, 30_000, 'initial session snapshot')
    logStep('relay_subscribe_ok', { events: eventLog.length })

    const launchResponse = unwrapVariant(
      await remoteClient.send(launchProviderRunRequest(sessionId, options.provider, 'default', options.model, 'medium', defaultAgentId)),
      'ProviderRunLaunchAccepted',
      'ProviderRunLaunched',
    )
    const providerRunId = launchResponse.provider_run?.id ?? launchResponse.provider_run_id ?? null
    logStep('relay_launch_provider_run_ok', { providerRunId, provider: options.provider, model: options.model })

    const firstPrompt = 'Reply with exactly RELAY_OK and nothing else.'
    await remoteClient.send(submitPromptRequest(sessionId, attachment.id, defaultAgentId, firstPrompt, []))
    logStep('relay_submit_prompt_ok', { prompt: firstPrompt })

    const firstCompletion = await waitForPromptEvidence(
      remoteClient,
      sessionId,
      attachment.id,
      eventLog,
      {
        ...options,
        completionBaseline: 0,
        expectedText: 'RELAY_OK',
      },
    )
    const firstTerminalOutputs = eventLog.filter((event) => event.event === 'terminal_output').length
    logStep('relay_prompt_completed', {
      completedAtMs: firstCompletion.completed_at_ms,
      terminalOutputEvents: firstTerminalOutputs,
    })

    logStep('restart_relay_begin')
    await terminateChild(relayChild)
    await waitForEvent((event) => event.event === 'transport_closed', eventLog, 30_000, 'transport_closed event')
    logStep('relay_down_observed')

    relayChild = spawnProcess(
      relayBinary,
      [],
      { cwd: repoRoot, env: envs.relayEnv },
    )

    await waitForRelayTarget(relayUrl, envs.relayToken, envs.daemonAlias)
    logStep('relay_target_reachable_after_restart')
    const resumed = await waitForEvent((event) => event.event === 'transport_resumed', eventLog, 45_000, 'transport_resumed event')
    logStep('relay_reconnect_ok', { resumedFromEventId: resumed.resumed_from_event_id ?? null })

    const stateAfterResume = unwrapVariant(await remoteClient.send(getSessionStateRequest(sessionId)), 'SessionStateLoaded', 'SessionState')
    const resumedSession = stateAfterResume.session ?? stateAfterResume.SessionStateLoaded?.session ?? null
    if (resumedSession?.id !== sessionId) {
      throw new Error(`relay get_session_state failed after reconnect: ${JSON.stringify(stateAfterResume)}`)
    }
    logStep('relay_request_after_reconnect_ok', { sessionId })

    const completionBaseline = eventLog.filter((event) => event.event === 'assistant_message_completed').length
    const secondPrompt = 'Reply with exactly RELAY_RECONNECT_OK and nothing else.'
    await remoteClient.send(submitPromptRequest(sessionId, attachment.id, defaultAgentId, secondPrompt, []))
    logStep('relay_submit_prompt_after_reconnect_ok', { prompt: secondPrompt })

    const secondCompletion = await waitForPromptEvidence(
      remoteClient,
      sessionId,
      attachment.id,
      eventLog,
      {
        ...options,
        completionBaseline,
        expectedText: 'RELAY_RECONNECT_OK',
      },
    )
    logStep('relay_prompt_after_reconnect_completed', {
      completedAtMs: secondCompletion.completed_at_ms,
      totalEvents: eventLog.length,
    })

    console.log(JSON.stringify({
      relayUrl,
      kernelUrl,
      daemonId: envs.daemonId,
      daemonAlias: envs.daemonAlias,
      sessionId,
      provider: options.provider,
      model: options.model,
      events: {
        total: eventLog.length,
        sessionSnapshots: eventLog.filter((event) => event.event === 'session_snapshot').length,
        terminalOutput: eventLog.filter((event) => event.event === 'terminal_output').length,
        completed: eventLog.filter((event) => event.event === 'assistant_message_completed').length,
        transportClosed: eventLog.filter((event) => event.event === 'transport_closed').length,
        transportResumed: eventLog.filter((event) => event.event === 'transport_resumed').length,
      },
      reconnect: {
        resumedFromEventId: resumed.resumed_from_event_id ?? null,
      },
      status: 'ok',
    }, null, 2))
    succeeded = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    if (remoteClient) {
      if (sessionId) await remoteClient.send(endSessionRequest(sessionId)).catch(() => {})
      await remoteClient.close().catch(() => {})
    } else if (localClient && sessionId) {
      await localClient.send(endSessionRequest(sessionId)).catch(() => {})
    }
    if (localClient) await localClient.close().catch(() => {})
    await terminateChild(daemonChild)
    await terminateChild(relayChild)
    await finalizeDrillArtifacts({
      rootDir,
      passed: succeeded,
      preserveOnFailure: true,
      failure,
      metadata: {
        drill: 'relay-runtime',
        relayUrl,
        kernelUrl,
        daemonId: envs?.daemonId ?? null,
        daemonAlias: envs?.daemonAlias ?? null,
        sessionId,
        provider: options.provider,
        model: options.model,
        eventCounts: {
          total: eventLog.length,
          sessionSnapshots: eventLog.filter((event) => event.event === 'session_snapshot').length,
          terminalOutput: eventLog.filter((event) => event.event === 'terminal_output').length,
          completed: eventLog.filter((event) => event.event === 'assistant_message_completed').length,
          transportClosed: eventLog.filter((event) => event.event === 'transport_closed').length,
          transportResumed: eventLog.filter((event) => event.event === 'transport_resumed').length,
        },
        relayStdoutTail: relayChild?.logs?.stdout?.slice(-4000) ?? '',
        relayStderrTail: relayChild?.logs?.stderr?.slice(-4000) ?? '',
        daemonStdoutTail: daemonChild?.logs?.stdout?.slice(-4000) ?? '',
        daemonStderrTail: daemonChild?.logs?.stderr?.slice(-4000) ?? '',
      },
      log: logStep,
    })
  }
}

await main()
