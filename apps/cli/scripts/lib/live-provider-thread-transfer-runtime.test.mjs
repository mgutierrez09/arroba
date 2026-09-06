import assert from "node:assert/strict"
import { chmod, mkdir, mkdtemp, readFile, realpath, rm, stat, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { DatabaseSync } from "node:sqlite"
import test from "node:test"

import {
  cleanupSliceRuntime,
  failResultOnSliceCleanupErrors,
} from "./live-provider-thread-transfer-slice-scenarios.mjs"
import {
  materializedProviderEnvironment,
  providerStateCopySpecs,
  transferProviderThreadStateToWorker,
  transferProviderStateToWorker,
} from "./live-provider-thread-transfer-provider-state.mjs"
import {
  cleanupSliceModeProviderCredentials,
  loadRawHistoryOutputText,
  normalizeProviderOutputText,
  parseArgs,
  providerThreadSliceOptLevel,
  providerThreadSliceBuildProfile,
  providerThreadSliceBuildEnv,
  providerThreadSliceConfigLines,
  providerThreadKernelEventSnapshot,
  providerRunSnapshot,
  providersNeedClaudeCredentials,
  prepareIsolatedWorkerProviderEnv,
  prepareSliceModeProviderEnv,
  repoRoot,
  relayClaims,
  terminalProviderHistoryError,
  sliceRestartContinuityChecks,
  sliceShutdownCheckpointChecks,
  workerResumeDaemonEnv,
  writeClaudeCredentialsPayload,
} from "./live-provider-thread-transfer-runtime.mjs"

test("local provider transfer tokens do not claim an unverified key binding", () => {
  const claims = relayClaims({
    subject: "kernel-1",
    subjectKind: "kernel",
    actions: ["daemon_register"],
  })

  assert.equal(claims.public_key_thumbprint, null)
})

test("provider thread drill accepts the explicit slice shutdown scenario", () => {
  assert.equal(parseArgs(["--drill", "slice-shutdown"]).drill, "slice-shutdown")
})

test("provider thread drill accepts the injected slice save failure scenario", () => {
  assert.equal(parseArgs(["--drill", "slice-save-failure"]).drill, "slice-save-failure")
})

test("slice shutdown checkpoint requires a stopped slice and parked provider", () => {
  assert.deepEqual(sliceShutdownCheckpointChecks({
    savedSlice: { status: "stopped" },
    parkedRun: { state: "ended" },
    stoppedSession: { active_provider_run_id: null },
  }), {
    slice_shutdown_left_stopped: true,
    slice_shutdown_parked_provider_run: true,
    slice_shutdown_cleared_active_provider_run: true,
    slice_shutdown_checkpoint_valid: true,
  })
})

test("provider thread evidence redacts credentials from kernel events", () => {
  assert.deepEqual(
    providerThreadKernelEventSnapshot({
      event: "session_snapshot",
      provider_run: {
        runtime_mcp_auth_token: "fixture-runtime-token",
        pty_env: {
          CHARIOX_MCP_TOKEN: "fixture-runtime-token",
          SAFE_VALUE: "visible",
        },
      },
    }, 42),
    {
      observed_at_ms: 42,
      event: "session_snapshot",
      provider_run: {
        runtime_mcp_auth_token: "<redacted>",
        pty_env: {
          CHARIOX_MCP_TOKEN: "<redacted>",
          SAFE_VALUE: "visible",
        },
      },
    },
  )
})

test("slice restart continuity requires stable worker identity and fresh execution", () => {
  assert.deepEqual(
    sliceRestartContinuityChecks({
      beforeRun: { id: "provider-run-before", started_at_ms: 100 },
      afterRun: { id: "provider-run-after", started_at_ms: 300 },
      beforeBinding: {
        worker_kernel_id: "kernel-stable",
        worker_machine_id: "slice:slice-1",
        execution_lease_id: "lease-before",
        leased_agent_id: "leased-agent-before",
      },
      afterBinding: {
        worker_kernel_id: "kernel-stable",
        worker_machine_id: "slice:slice-1",
        execution_lease_id: "lease-after",
        leased_agent_id: "leased-agent-after",
      },
      sliceBeforeRestart: {
        worker_kernel_id: "kernel-stable",
        worker_machine_id: "slice:slice-1",
      },
      restartedSlice: {
        status: "running",
        worker_kernel_id: "kernel-stable",
        worker_machine_id: "slice:slice-1",
      },
      savedState: {
        image_ref: "chariox-slice-state:fixture",
        created_at_ms: 200,
      },
    }),
    {
      agent_binding_repaired: true,
      slice_worker_identity_preserved: true,
      slice_restart_timeline_valid: true,
      slice_restart_completed: true,
    },
  )
})

test("slice restart continuity rejects an unchanged execution lease", () => {
  const checks = sliceRestartContinuityChecks({
    beforeRun: { id: "provider-run-before", started_at_ms: 100 },
    afterRun: { id: "provider-run-after", started_at_ms: 300 },
    beforeBinding: {
      worker_kernel_id: "kernel-stable",
      worker_machine_id: "slice:slice-1",
      execution_lease_id: "lease-same",
      leased_agent_id: "leased-agent-same",
    },
    afterBinding: {
      worker_kernel_id: "kernel-stable",
      worker_machine_id: "slice:slice-1",
      execution_lease_id: "lease-same",
      leased_agent_id: "leased-agent-same",
    },
    sliceBeforeRestart: {
      worker_kernel_id: "kernel-stable",
      worker_machine_id: "slice:slice-1",
    },
    restartedSlice: {
      status: "running",
      worker_kernel_id: "kernel-stable",
      worker_machine_id: "slice:slice-1",
    },
    savedState: {
      image_ref: "chariox-slice-state:fixture",
      created_at_ms: 200,
    },
  })

  assert.equal(checks.agent_binding_repaired, false)
  assert.equal(checks.slice_restart_completed, false)
})

test("provider run evidence retains account and execution authority", () => {
  assert.deepEqual(
    providerRunSnapshot({
      id: "run-1",
      provider: "codex",
      adapter_key: "codex",
      account_profile: "work",
      execution_mode: "plan",
      permission_level: "required",
    }),
    {
      id: "run-1",
      provider: "codex",
      adapter_key: "codex",
      account_profile: "work",
      state: null,
      provider_session_id: null,
      resume_state: null,
      mcp_servers: [],
      execution_mode: "plan",
      permission_level: "required",
      write_access_mode: null,
      working_directory: null,
      started_at_ms: null,
      last_activity_at_ms: null,
    },
  )
})

test("raw history fallback joins fragmented provider output from current SQLite layouts", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-provider-history-test-"))
  try {
    for (const [name, databasePath] of [
      ["local", path.join(root, "local", "history", "operational.db")],
      ["slice", path.join(root, "slice", "home-kernel-storage", "operational-history.db")],
    ]) {
      const historyDir = path.join(root, name, "history")
      await mkdir(historyDir, { recursive: true })
      await mkdir(path.dirname(databasePath), { recursive: true })
      const database = new DatabaseSync(databasePath)
      database.exec(`
        CREATE TABLE history_events (
          sequence INTEGER NOT NULL,
          kind TEXT NOT NULL,
          session_id TEXT,
          agent_id TEXT,
          content TEXT
        )
      `)
      const insert = database.prepare(
        "INSERT INTO history_events(sequence, kind, session_id, agent_id, content) VALUES (?, ?, ?, ?, ?)",
      )
      insert.run(1, "provider_output", "session-1", "agent-1", "THREAD_")
      insert.run(2, "provider_output", "session-1", "agent-1", "TRANSFER")
      insert.run(3, "provider_output", "session-1", "agent-1", "_READY")
      database.close()

      assert.equal(
        await loadRawHistoryOutputText({ historyDir, sessionId: "session-1", agentId: "agent-1" }),
        "THREAD_TRANSFER_READY",
      )
    }
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("provider output marker matching removes ANSI without accepting subsequences", () => {
  const expected = "THREAD_TRANSFER_READY"
  assert.equal(
    normalizeProviderOutputText("THREAD_\x1b[31mTRANSFER\x1b[0m_READY").includes(expected),
    true,
  )
  assert.equal(
    normalizeProviderOutputText("THREAD_TRANSFER was requested with suffix _READY").includes(expected),
    false,
  )
})

test("Claude provider aliases request isolated credentials", () => {
  assert.equal(providersNeedClaudeCredentials(["codex", "opencode"]), false)
  assert.equal(providersNeedClaudeCredentials(["claude"]), true)
  assert.equal(providersNeedClaudeCredentials(["claude-p"]), true)
  assert.equal(providersNeedClaudeCredentials(["claude-headless"]), true)
})

test("Claude provider state uses the selected provider home", () => {
  const specs = providerStateCopySpecs("claude-headless", { HOME: "/isolated/provider-home" })
  assert.deepEqual(
    specs.map((spec) => spec.source),
    ["/isolated/provider-home/.claude", "/isolated/provider-home/.claude.json"],
  )
})

test("provider state transfer copies into an isolated worker home", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-provider-state-test-"))
  try {
    const sourceHome = path.join(root, "source")
    const destinationHome = path.join(root, "destination")
    await writeClaudeCredentialsPayload(
      path.join(sourceHome, ".claude", "projects", "session.json"),
      Buffer.from('{"session":"one"}\n'),
    )
    await writeClaudeCredentialsPayload(
      path.join(sourceHome, ".claude.json"),
      Buffer.from('{"hasCompletedOnboarding":true}\n'),
    )

    const evidence = await transferProviderStateToWorker({
      provider: "claude-headless",
      sourceProviderEnv: { HOME: sourceHome },
      destinationProviderEnv: { HOME: destinationHome },
    })

    assert.equal(evidence.copied.length, 2)
    assert.equal(
      await readFile(path.join(destinationHome, ".claude", "projects", "session.json"), "utf8"),
      '{"session":"one"}\n',
    )
    assert.equal(
      await readFile(path.join(destinationHome, ".claude.json"), "utf8"),
      '{"hasCompletedOnboarding":true}\n',
    )
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("Codex thread transfer copies only the requested rollout into the materialized worker profile", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-codex-thread-state-test-"))
  try {
    const sourceCodexHome = path.join(root, "source-codex")
    const workerStorageRoot = path.join(root, "worker-storage")
    const threadId = "019ec2de-c98f-7440-b3e4-3edbdb3aa1ab"
    const relativeRollout = path.join(
      "sessions",
      "2026",
      "06",
      "13",
      `rollout-2026-06-13T12-00-00-${threadId}.jsonl`,
    )
    await mkdir(path.join(sourceCodexHome, path.dirname(relativeRollout)), { recursive: true })
    await writeFile(path.join(sourceCodexHome, relativeRollout), '{"thread":"requested"}\n')
    await writeFile(path.join(sourceCodexHome, "unrelated.json"), '{"thread":"other"}\n')

    const destinationProviderEnv = materializedProviderEnvironment({
      provider: "codex",
      storageRoot: workerStorageRoot,
      ownerUserId: "local",
      profileId: "codex-1",
    })
    const evidence = await transferProviderThreadStateToWorker({
      provider: "codex",
      providerSessionId: threadId,
      sourceProviderEnv: { CODEX_HOME: sourceCodexHome },
      destinationProviderEnv,
    })

    assert.deepEqual(evidence.copied, [{
      kind: "codex_rollout",
      relative_path: relativeRollout,
      byte_length: 23,
    }])
    assert.equal(
      await readFile(path.join(destinationProviderEnv.CODEX_HOME, relativeRollout), "utf8"),
      '{"thread":"requested"}\n',
    )
    await assert.rejects(
      readFile(path.join(destinationProviderEnv.CODEX_HOME, "unrelated.json")),
      { code: "ENOENT" },
    )
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("materialized provider paths reject profile traversal", () => {
  assert.throws(
    () => materializedProviderEnvironment({
      provider: "codex",
      storageRoot: "/worker-storage",
      ownerUserId: "local",
      profileId: "..",
    }),
    /profile id is not a safe path component/,
  )
})

test("OpenCode thread transfer imports from the resumed workspace", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-opencode-thread-state-test-"))
  try {
    const command = path.join(root, "fake-opencode.mjs")
    await writeFile(command, [
      "#!/usr/bin/env node",
      "import { copyFileSync, mkdirSync, writeFileSync } from 'node:fs'",
      "import path from 'node:path'",
      "const [operation, value] = process.argv.slice(2)",
      "if (operation === 'export') process.stdout.write(JSON.stringify({ id: value }))",
      "else if (operation === 'import') {",
      "  const destination = path.join(process.env.XDG_DATA_HOME, 'opencode', 'imported.json')",
      "  mkdirSync(path.dirname(destination), { recursive: true })",
      "  copyFileSync(value, destination)",
      "  writeFileSync(path.join(process.env.XDG_DATA_HOME, 'opencode', 'import-cwd.txt'), process.cwd())",
      "} else process.exitCode = 2",
      "",
    ].join("\n"))
    await chmod(command, 0o700)
    const threadId = "ses_13d274232ffec5B9kAwaIWSNhG"
    const sourceDataHome = path.join(root, "source-data")
    const destinationDataHome = path.join(root, "destination-data")
    const workingDirectory = path.join(root, "resumed-workspace")
    await mkdir(workingDirectory, { recursive: true })
    const evidence = await transferProviderThreadStateToWorker({
      provider: "opencode",
      providerSessionId: threadId,
      sourceProviderEnv: { XDG_DATA_HOME: sourceDataHome },
      destinationProviderEnv: { XDG_DATA_HOME: destinationDataHome },
      openCodeCommand: command,
      workingDirectory,
    })

    assert.deepEqual(evidence.copied, [{
      kind: "opencode_session_export",
      byte_length: 39,
    }])
    assert.deepEqual(
      JSON.parse(await readFile(path.join(destinationDataHome, "opencode", "imported.json"), "utf8")),
      { id: threadId },
    )
    assert.equal(
      await readFile(path.join(destinationDataHome, "opencode", "import-cwd.txt"), "utf8"),
      await realpath(workingDirectory),
    )
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("OpenCode thread transfer bounds provider command execution", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-opencode-thread-timeout-test-"))
  try {
    const command = path.join(root, "slow-opencode.mjs")
    await writeFile(command, [
      "#!/usr/bin/env node",
      "process.stdout.write('{}')",
      "setTimeout(() => {}, 200)",
      "",
    ].join("\n"))
    await chmod(command, 0o700)
    await assert.rejects(
      transferProviderThreadStateToWorker({
        provider: "opencode",
        providerSessionId: "ses_timeout",
        sourceProviderEnv: { XDG_DATA_HOME: path.join(root, "source") },
        destinationProviderEnv: { XDG_DATA_HOME: path.join(root, "destination") },
        openCodeCommand: command,
        openCodeCommandTimeoutMs: 25,
      }),
      /OpenCode export timed out/,
    )
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("Claude credential payloads are validated and written mode 600", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-claude-credentials-test-"))
  try {
    const destination = path.join(root, ".claude", ".credentials.json")
    await writeClaudeCredentialsPayload(destination, Buffer.from('{"oauth":"test"}\n'))
    assert.equal(await readFile(destination, "utf8"), '{"oauth":"test"}\n')
    assert.equal((await stat(destination)).mode & 0o777, 0o600)
    await assert.rejects(
      writeClaudeCredentialsPayload(path.join(root, "invalid.json"), Buffer.from("[]")),
      /JSON object/,
    )
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("legacy Claude worker drills fail closed before copying credentials", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-claude-fail-closed-test-"))
  try {
    await assert.rejects(
      prepareSliceModeProviderEnv(root, ["claude"]),
      /managed Chariox-vault setup-token path/,
    )
    await assert.rejects(
      prepareIsolatedWorkerProviderEnv(["claude"], "test-worker"),
      /will not read macOS Keychain or copy refreshable credentials/,
    )
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("unattended Claude drills never invoke macOS Keychain", async () => {
  const sources = await Promise.all([
    "apps/cli/scripts/lib/live-provider-thread-transfer-runtime.mjs",
    "apps/cli/scripts/live-claude-headless-slice-drill.mjs",
    "apps/cli/scripts/live-remote-native-tui-drill.mjs",
  ].map((relativePath) => readFile(path.join(repoRoot, relativePath), "utf8")))

  for (const source of sources) {
    assert.doesNotMatch(source, /find-generic-password/)
    assert.doesNotMatch(source, /(?:spawn|execFile|commandOutput)\s*\(\s*["']security["']/)
    assert.doesNotMatch(source, /exportClaudeCredentials/)
  }
})

test("slice provider credentials are removed without deleting provider state", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-slice-credentials-test-"))
  try {
    const codexHome = path.join(root, "codex")
    const opencodeHome = path.join(root, "opencode")
    const claudeSecretRoot = path.join(root, "claude-secrets")
    await writeClaudeCredentialsPayload(
      path.join(codexHome, "auth.json"),
      Buffer.from('{"token":"codex"}\n'),
    )
    await writeClaudeCredentialsPayload(
      path.join(opencodeHome, "auth.json"),
      Buffer.from('{"token":"opencode"}\n'),
    )
    await writeClaudeCredentialsPayload(
      path.join(claudeSecretRoot, "credentials.json"),
      Buffer.from('{"token":"claude"}\n'),
    )
    await writeClaudeCredentialsPayload(
      path.join(codexHome, "sessions", "state.json"),
      Buffer.from('{"session":"retained"}\n'),
    )

    await cleanupSliceModeProviderCredentials({
      CODEX_HOME: codexHome,
      OPENCODE_DATA_HOME: opencodeHome,
      CHARIOX_PROVIDER_THREAD_CODEX_AUTH_COPIED: "1",
      CHARIOX_PROVIDER_THREAD_OPENCODE_AUTH_COPIED: "1",
      CHARIOX_PROVIDER_THREAD_CLAUDE_SECRET_ROOT: claudeSecretRoot,
    })

    await assert.rejects(stat(path.join(codexHome, "auth.json")), { code: "ENOENT" })
    await assert.rejects(stat(path.join(opencodeHome, "auth.json")), { code: "ENOENT" })
    await assert.rejects(stat(claudeSecretRoot), { code: "ENOENT" })
    assert.equal(
      await readFile(path.join(codexHome, "sessions", "state.json"), "utf8"),
      '{"session":"retained"}\n',
    )
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("slice restart cleanup resets saved state before deleting the slice", async () => {
  const requests = []
  const evidence = {}
  const client = {
    async send(request) {
      requests.push(request)
      if ("ResetSliceState" in request) throw new Error("saved state cleanup failed")
      return { SliceDeleted: { slice: { id: "slice-1" } } }
    },
  }

  await cleanupSliceRuntime(client, "slice-1", evidence, { resetSavedState: true })

  assert.deepEqual(requests, [
    { ResetSliceState: { slice_ref: "slice-1" } },
    { DeleteSlice: { slice_ref: "slice-1" } },
  ])
  assert.equal(evidence.slice_state_cleanup_error, "saved state cleanup failed")
  assert.equal(evidence.slice_cleanup_error, undefined)
})

test("slice cleanup errors fail an otherwise passing drill result", () => {
  const result = {
    status: "passed",
    evidence: { slice_state_cleanup_error: "state image remained" },
    errors: [],
  }

  failResultOnSliceCleanupErrors(result, { resetSavedState: true })

  assert.equal(result.status, "failed")
  assert.deepEqual(result.errors, ["slice cleanup failed: state image remained"])
})

test("provider thread transfer fails fast on terminal provider history", () => {
  const failure = terminalProviderHistoryError([
    { kind: "notice", text: "provider is starting" },
    { kind: "provider_error", text: "account balance exhausted" },
  ])

  assert.equal(failure?.text, "account balance exhausted")
  assert.equal(
    terminalProviderHistoryError([
      {
        kind: "notice",
        text: "Provider run `provider-run-1` for `claude-headless` ended unexpectedly. No active prompt was running.",
      },
    ])?.kind,
    "notice",
  )
})

test("provider thread transfer ignores nonterminal provider history", () => {
  assert.equal(terminalProviderHistoryError([
    { kind: "notice", text: "provider is starting" },
    { kind: "provider_output", text: "done" },
  ]), null)
})

test("worker resume daemons keep Chariox state inside the drill runtime root", () => {
  const env = workerResumeDaemonEnv({
    ports: { relayPort: 4000 },
    root: "/tmp/provider-runtime",
    relayToken: "token",
    daemonId: "worker-1",
    daemonAlias: "worker",
    machineId: "machine-1",
    machineAlias: "machine",
    acceptRemoteLeases: true,
    socketName: "worker.sock",
    kernelPort: 4001,
    mcpPort: 4002,
    openCodePort: 4003,
    codexPort: 4004,
    providerEnv: {},
  })

  assert.equal(env.CHARIOX_HOME, "/tmp/provider-runtime/worker-1-xdg-config/chariox")
  assert.equal(env.CHARIOX_SESSION_HISTORY_DIR, "/tmp/provider-runtime/worker-1-history")
  assert.equal(env.CHARIOX_DAEMON_SOCKET, "/tmp/provider-runtime/worker.sock")
})

test("slice provider drills leave account publication to execution-lease materialization", async () => {
  const source = await readFile(
    new URL("./live-provider-thread-transfer-slice-scenarios.mjs", import.meta.url),
    "utf8",
  )
  assert.doesNotMatch(source, /sliceProviderAuthImportRequest|import-slice-provider-auth/)
  assert.match(source, /kernel_execution_lease_materialization/)
})

test("provider thread slice builds use a bounded optimization level", () => {
  assert.equal(providerThreadSliceOptLevel({}), "1")
  assert.equal(
    providerThreadSliceOptLevel({ CHARIOX_PROVIDER_THREAD_SLICE_OPT_LEVEL: "0" }),
    "0",
  )
  assert.throws(
    () => providerThreadSliceOptLevel({ CHARIOX_PROVIDER_THREAD_SLICE_OPT_LEVEL: "fast" }),
    /optimization level/,
  )
})

test("provider thread slice builds use a low-memory development profile", () => {
  assert.equal(providerThreadSliceBuildProfile({}), "dev")
  assert.equal(
    providerThreadSliceBuildProfile({ CHARIOX_PROVIDER_THREAD_SLICE_BUILD_PROFILE: "release" }),
    "release",
  )
  assert.throws(
    () => providerThreadSliceBuildProfile({
      CHARIOX_PROVIDER_THREAD_SLICE_BUILD_PROFILE: "benchmark",
    }),
    /build profile/,
  )
})

test("provider thread slice builds delegate bounded settings to the provisioner", () => {
  assert.deepEqual(
    providerThreadSliceBuildEnv({}),
    {
      CHARIOX_SLICE_RUNTIME_BUILD_PROFILE: "dev",
      CHARIOX_SLICE_CARGO_PROFILE_RELEASE_OPT_LEVEL: "1",
    },
  )
  assert.deepEqual(
    providerThreadSliceBuildEnv({
      CHARIOX_PROVIDER_THREAD_SLICE_BUILD_PROFILE: "release",
      CHARIOX_PROVIDER_THREAD_SLICE_OPT_LEVEL: "2",
    }),
    {
      CHARIOX_SLICE_RUNTIME_BUILD_PROFILE: "release",
      CHARIOX_SLICE_CARGO_PROFILE_RELEASE_OPT_LEVEL: "2",
    },
  )
})

test("provider thread slice drills enable the explicit nested-namespace compatibility boundary", () => {
  const lines = providerThreadSliceConfigLines({
    sliceRoot: "/tmp/provider-thread-slices",
    image: "chariox-slice-linux:test",
    buildImage: "never",
  })

  assert.deepEqual(lines, [
    "[slices]",
    'root = "/tmp/provider-thread-slices"',
    "",
    "[slices.linux]",
    'docker_image = "chariox-slice-linux:test"',
    'build_image = "never"',
    "memory_mb = 2048",
    'cpus = "1.0"',
    "allow_unconfined_seccomp = true",
  ])
})
