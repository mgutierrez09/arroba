import assert from "node:assert/strict"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import type {
  AgentInstance,
  CharioxMcpServerConfig,
  CharioxSkillMetadata,
  ProviderProcessInfo,
  WorkspaceLinkDefinition,
} from "./kernel-types.js"
import { applyShellCommandResult, createDefaultShellContext, parseShellCommand } from "./shell-core.js"
import { executeShellCommand } from "./shell-executor.js"
import {
  fakeClient,
  makeAgent,
  makeSession,
  makeWorkflow,
  makeWorkflowPublication,
  makeWorkflowRun,
  makeWorkflowWatchdog,
} from "./shell-executor.test-support.js"

test("executeShellCommand manages provider auth and processes", async () => {
  const process: ProviderProcessInfo = {
    process_id: "process-1",
    provider: "codex",
    process_label: "codex-agent",
    pid: 4321,
    resident_set_bytes: 134217728,
    endpoint_mode: "managed",
    status: "idle",
    started_at_ms: 0,
    last_activity_at_ms: 0,
    provider_session_ids: [],
    owner_session_ids: ["session-1"],
    owner_provider_run_ids: [],
    attached_session_ids: [],
    active_workflow_run_ids: [],
    teardown_safe: true,
    teardown_blockers: [],
  }
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("StartProviderLogin" in request) {
          return { ProviderLoginStarted: { login: { provider: "codex", account_profile: "default", login_kind: "device", login_id: "login-1", auth_url: null, verification_url: "https://auth.example", user_code: "ABCD" } } }
        }
        if ("LogoutProvider" in request) {
          return { ProviderLoggedOut: { provider: "codex" } }
        }
        if ("TeardownProviderProcesses" in request) {
          return { ProviderProcessesTornDown: { processes: [process] } }
        }
        return { ProviderProcessesListed: { processes: [process] } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", provider: "codex" })
  const login = await executeShellCommand(parseShellCommand("provider login"), context, { client: fake.client })
  const logout = await executeShellCommand(parseShellCommand("provider logout codex"), context, { client: fake.client })
  const reauth = await executeShellCommand(parseShellCommand("provider reauth codex"), context, { client: fake.client })
  const list = await executeShellCommand(parseShellCommand("provider processes codex"), context, { client: fake.client })
  const broadTeardown = await executeShellCommand(parseShellCommand("provider processes teardown"), context, { client: fake.client })
  const teardown = await executeShellCommand(parseShellCommand("provider processes teardown codex"), context, { client: fake.client })
  const unknown = await executeShellCommand(parseShellCommand("provider unknown"), context, { client: fake.client })
  assert.equal(login.ok, true)
  assert.match(login.message ?? "", /codex login started/)
  assert.equal(logout.ok, true)
  assert.equal(reauth.ok, true)
  assert.match(reauth.message ?? "", /codex reauth started/)
  assert.equal(list.ok, true)
  assert.match(list.message ?? "", /process-1 codex codex-agent pid=4321 rss=128.0MiB/)
  assert.match(list.message ?? "", /next=run \/provider processes teardown codex to stop only safe daemon-tracked processes owned by you/)
  assert.equal(broadTeardown.ok, false)
  assert.match(broadTeardown.message ?? "", /usage: provider processes teardown <provider>/)
  assert.equal(teardown.ok, true)
  assert.match(teardown.message ?? "", /tore down 1 provider process/)
  assert.equal(unknown.ok, false)
  assert.match(unknown.message ?? "", /usage: provider status\|login\|setup-token\|login-status\|login-cancel\|logout\|reauth\|processes \[provider\]\|processes teardown <provider>/)
  assert.deepEqual(requests, [
    { StartProviderLogin: { provider: "codex", account_profile: "default" } },
    { LogoutProvider: { provider: "codex", account_profile: "default" } },
    { LogoutProvider: { provider: "codex", account_profile: "default" } },
    { StartProviderLogin: { provider: "codex", account_profile: "default" } },
    { ListProviderProcesses: { provider: "codex" } },
    { TeardownProviderProcesses: { provider: "codex", force: false } },
  ])
})

test("executeShellCommand stores Claude setup tokens through hidden input", async () => {
  const requests: Record<string, unknown>[] = []
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    agentId: "agent-1",
  })
  const result = await executeShellCommand(
    parseShellCommand("provider setup-token claude work --replace"),
    context,
    {
      client: {
        send: async (request) => {
          requests.push(request)
          return {
            ProviderAccountCredentialStored: {
              provider: "claude",
              account_profile: "work",
              credential_id: "provider-account-claude-redacted",
              replaced: true,
            },
          }
        },
      },
      readSecret: async () => "  setup-token-secret  ",
    },
  )

  assert.equal(result.ok, true)
  assert.equal(result.message, "claude/work setup token replaced in Chariox Vault")
  assert.deepEqual(result.data, {
    provider: "claude",
    account_profile: "work",
    replaced: true,
  })
  assert.deepEqual(requests, [{
    SetProviderAccountCredential: {
      session_id: "session-1",
      agent_id: "agent-1",
      provider: "claude",
      account_profile: "work",
      value: "setup-token-secret",
      overwrite: true,
    },
  }])
})

test("executeShellCommand searches current session recall", async () => {
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        return {
          RecallEvents: {
            events: [{
              event_id: "event-1",
              sequence: 42,
              timestamp_ms: 1_700_000_000_000,
              session_id: "session-1",
              agent_id: "agent-1",
              provider: "codex",
              model: "gpt-5",
              kind: "provider_output",
              role: "assistant",
              content: "Fixed the failing build by updating the test.",
            }],
            next_sequence: null,
          },
        }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const result = await executeShellCommand(parseShellCommand("recall search failing build"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /Fixed the failing build/)
  assert.deepEqual(requests, [
    {
      SearchRecall: {
        query: "failing build",
        session_id: "session-1",
        agent_id: null,
        provider: null,
        model: null,
        workflow_id: null,
        machine_id: null,
        repo_root: null,
        worktree_path: null,
        kind: null,
        after_sequence: null,
        limit: 50,
      },
    },
  ])
})

test("executeShellCommand surfaces semantic recall search availability", async () => {
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        return {
          SemanticRecallEvents: {
            results: [],
            next_cursor: null,
            unavailable_reason: "semantic recall search is not configured for this kernel",
          },
        }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const result = await executeShellCommand(parseShellCommand("recall semantic-search why did tests fail"), context, { client: fake.client })
  assert.equal(result.ok, false)
  assert.match(result.message ?? "", /not configured/)
  assert.deepEqual(requests, [
    {
      SemanticSearchRecall: {
        query: "why did tests fail",
        mode: "knn",
        session_id: "session-1",
        agent_id: null,
        provider: null,
        model: null,
        workflow_id: null,
        machine_id: null,
        repo_root: null,
        worktree_path: null,
        kind: null,
        cursor: null,
        limit: 20,
      },
    },
  ])
})

test("executeShellCommand requests focused-agent semantic recall search", async () => {
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        return {
          SemanticRecallEvents: {
            answer: "Tests failed because the snapshot changed.",
            results: [],
            next_cursor: null,
            unavailable_reason: null,
          },
        }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const result = await executeShellCommand(parseShellCommand("recall semantic-search --agent why did tests fail"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /snapshot changed/)
  assert.deepEqual(requests, [
    {
      SemanticSearchRecall: {
        query: "why did tests fail",
        mode: "agent",
        session_id: "session-1",
        agent_id: null,
        provider: null,
        model: null,
        workflow_id: null,
        machine_id: null,
        repo_root: null,
        worktree_path: null,
        kind: null,
        cursor: null,
        limit: 20,
      },
    },
  ])
})

test("executeShellCommand cancels active prompt through the current session attachment", async () => {
  const session = makeSession({ attachment_ids: ["attachment-1"] })
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("GetSessionState" in request) {
          return { SessionState: { session } }
        }
        return { PromptCancelled: { cancellation: { prompt: { id: "prompt-1" }, started_next: null } } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const result = await executeShellCommand(parseShellCommand("stop"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /prompt prompt-1/)
  assert.deepEqual(requests, [
    { GetSessionState: { session_id: "session-1" } },
    { CancelActivePrompt: { session_id: "session-1", attachment_id: "attachment-1" } },
  ])
})

test("executeShellCommand cancels active prompt through shell context attachment", async () => {
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        return { PromptCancelled: { cancellation: { prompt: null, started_next: null } } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1", attachmentId: "attachment-shell" })
  const result = await executeShellCommand(parseShellCommand("stop"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.deepEqual(requests, [
    { CancelActivePrompt: { session_id: "session-1", attachment_id: "attachment-shell" } },
  ])
})
