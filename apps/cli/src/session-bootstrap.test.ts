import assert from "node:assert/strict"
import test from "node:test"

import { fallbackProviderCatalog } from "./provider-catalog.js"
import { fallbackProviderCommandCatalogs } from "./provider-command-catalog.js"
import { bootstrapSession } from "./session-bootstrap.js"
import { hydrateSessionHistoryOutlineAgentEntries } from "@chariox/kernel-client/session-history-transcript"
import type { CliOptions, RuntimeSession } from "./cli-types.js"

function terminalCatalog() {
  return {
    revision: "test",
    nodes: [],
  }
}

test("bootstrapSession returns waiting-room bootstrap when no session should attach", async () => {
  const catalog = fallbackProviderCatalog()
  const bootstrap = await bootstrapSession(
    {} as never,
    {
      clientId: "cli-1",
      model: "default",
      accountProfile: "default",
      effort: "",
    },
    "/workspace",
    "/workspace",
    {},
    {
      listSessions: async () => [],
      getProviderCatalog: async () => catalog,
      getProviderCommandCatalogs: async () => fallbackProviderCommandCatalogs(),
      getTerminalCommandCatalog: async () => terminalCatalog(),
      createSession: async () => { throw new Error("should not create") },
      resolveSession: async () => { throw new Error("should not resolve") },
      attachToSession: async () => { throw new Error("should not attach") },
      getSessionState: async () => { throw new Error("should not fetch") },
      launchProviderRun: async () => { throw new Error("should not launch") },
      tryGetProviderRun: async () => { throw new Error("should not lookup") },
      catchUpAttachedSession: async () => undefined,
      getSessionHistoryOutline: async () => ({ agents: [] }),
      resolveVisibleAgentId: () => null,
      prepareHistoryOutlineAgent: () => [],
    },
  )

  assert.equal(bootstrap.binding, null)
  assert.deepEqual(bootstrap.sessions, [])
  assert.equal(bootstrap.providerCatalog, catalog)
  assert.deepEqual(bootstrap.providerCommandCatalogs, fallbackProviderCommandCatalogs())
})

test("bootstrapSession seeds provider/model/effort from the kernel config.toml default when unset", async () => {
  const catalog = fallbackProviderCatalog()
  const options: CliOptions = {
    clientId: "cli-1",
    model: "default",
    accountProfile: "default",
    effort: "",
  }

  await bootstrapSession(
    {} as never,
    options,
    "/workspace",
    "/workspace",
    {},
    {
      getConfiguredProviderLaunchDefaults: async () => ({
        provider: "codex",
        model: "gpt-5.1",
        effort: "high",
      }),
      listSessions: async () => [],
      getProviderCatalog: async () => catalog,
      getProviderCommandCatalogs: async () => fallbackProviderCommandCatalogs(),
      getTerminalCommandCatalog: async () => terminalCatalog(),
      createSession: async () => { throw new Error("should not create") },
      resolveSession: async () => { throw new Error("should not resolve") },
      attachToSession: async () => { throw new Error("should not attach") },
      getSessionState: async () => { throw new Error("should not fetch") },
      launchProviderRun: async () => { throw new Error("should not launch") },
      tryGetProviderRun: async () => { throw new Error("should not lookup") },
      catchUpAttachedSession: async () => undefined,
      getSessionHistoryOutline: async () => ({ agents: [] }),
      resolveVisibleAgentId: () => null,
      prepareHistoryOutlineAgent: () => [],
    },
  )

  assert.equal(options.provider, "codex")
  assert.equal(options.model, "gpt-5.1")
  assert.equal(options.effort, "high")
})

test("bootstrapSession attaches, launches, and hydrates history for the visible agent", async () => {
  const catalog = fallbackProviderCatalog()
  const launched: Array<{ provider: string; model: string; effort: string }> = []
  const session = {
    id: "session-1",
    project_id: "project-default",
    workspace_id: "/workspace",
    worktree_id: "/workspace",
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-a",
    max_agents: 8,
    agents: [{
      id: "agent-a",
      agent_ref: "agent-a",
      session_id: "session-1",
      alias: null,
      provider: "codex",
      model: "codex/gpt-5.4-mini",
      effort: "low",
      worktree_id: null,
      state: "Idle" as const,
      is_processing: false,
      grid_row: 0,
      grid_col: 0,
      grid_row_span: 1,
      grid_col_span: 1,
      created_at_ms: 1,
      last_activity_at_ms: 1,
    }],
    config_state: { version: 1, values: {} },
  }
  const calls: string[] = []
  const bootstrap = await bootstrapSession(
    {} as never,
    {
      clientId: "cli-1",
      sessionId: "session-1",
      model: "gpt-5.4",
      accountProfile: "default",
      effort: "high",
    },
    "/workspace",
    "/workspace",
    {},
    {
      listSessions: async () => [session],
      getProviderCatalog: async () => catalog,
      getProviderCommandCatalogs: async () => fallbackProviderCommandCatalogs(),
      getTerminalCommandCatalog: async () => terminalCatalog(),
      createSession: async () => { throw new Error("should not create") },
      resolveSession: async () => {
        calls.push("resolve")
        return session
      },
      attachToSession: async () => {
        calls.push("attach")
        return { id: "attachment-1", session_id: "session-1" }
      },
      getSessionState: async () => {
        calls.push("session")
        return session
      },
      launchProviderRun: async (_client, _sessionId, provider, _accountProfile, model, effort) => {
        calls.push("launch")
        launched.push({ provider, model, effort })
        return {
          id: "run-1",
          session_id: "session-1",
          agent_instance_id: "agent-a",
          adapter_key: provider,
          provider,
          account_profile: "default",
          model,
          variant: effort,
          usage_tokens_total: null,
          state: "Running",
        }
      },
      tryGetProviderRun: async () => null,
      catchUpAttachedSession: async () => {
        calls.push("catchup")
      },
      getSessionHistoryOutline: async (_client, _sessionId, agentIds) => {
        calls.push(`outline:${agentIds.join(",")}`)
        return {
          agents: [outlineAgent("agent-a", "hi", "done", { before_sequence: 9 })],
        }
      },
      getPromptInputHistory: async () => {
        calls.push("prompt-history")
        return {
          entries: [{
            sequence: 1,
            timestamp_ms: 1,
            session_id: "session-1",
            kind: "prompt",
            text: "hi",
          }],
        }
      },
      resolveVisibleAgentId: () => "agent-a",
      prepareHistoryOutlineAgent: (agent) => agent.turns.map((_turn, index) => ({ id: index + 1, role: "user", text: "hi" })),
    },
  )

  assert.deepEqual(calls, ["resolve", "attach", "session", "launch", "catchup", "session", "outline:agent-a", "prompt-history"])
  assert.deepEqual(launched, [{ provider: "codex", model: "codex/gpt-5.4-mini", effort: "low" }])
  assert.equal(bootstrap.binding?.attachment.id, "attachment-1")
  assert.equal(bootstrap.binding?.providerRun?.id, "run-1")
  assert.deepEqual(bootstrap.binding?.historyEntries, [])
  assert.deepEqual(bootstrap.binding?.promptHistoryEntries, [])
  const deferredHistory = await bootstrap.deferred?.attachedHistory
  assert.deepEqual(deferredHistory?.historyEntries, [{ id: 1, role: "user", text: "hi" }])
  assert.deepEqual(deferredHistory?.agentEntries["agent-a"], [{ id: 1, role: "user", text: "hi" }])
  assert.deepEqual(deferredHistory?.promptHistoryEntries, ["hi"])
  assert.deepEqual(deferredHistory?.nextHistoryCursor, {
    agentId: "agent-a",
    cursor: { before_sequence: 9 },
  })
})

test("bootstrapSession seeds newly created sessions from the requested provider account selection", async () => {
  const launched: Array<{ provider: string; model: string; effort: string; agentId: string | null | undefined }> = []
  let createdAgentDefaults: RuntimeSession["agent_defaults"] | undefined
  const createdSession = {
    id: "session-created",
    project_id: "project-default",
    workspace_id: "/workspace",
    worktree_id: "/workspace",
    created_at_ms: 1,
    status: "Active",
    agent_defaults: {
      provider: "opencode",
      model: "opencode/gpt-5.4",
      effort: "high",
      account_profile: "default",
    },
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: null,
    max_agents: 8,
    agents: [],
    config_state: { version: 1, values: {} },
  }

  await bootstrapSession(
    {} as never,
    {
      clientId: "cli-1",
      createSession: true,
      provider: "opencode",
      model: "opencode/gpt-5.4",
      accountProfile: "default",
      effort: "high",
    },
    "/workspace",
    "/workspace",
    {},
    {
      listSessions: async () => [],
      getProviderCatalog: async () => fallbackProviderCatalog(),
      getProviderCommandCatalogs: async () => fallbackProviderCommandCatalogs(),
      getTerminalCommandCatalog: async () => terminalCatalog(),
      createSession: async (_client, _workspace, _worktree, _alias, agentDefaults) => {
        assert.ok(agentDefaults)
        createdAgentDefaults = agentDefaults
        return {
          ...createdSession,
          agent_defaults: agentDefaults,
        }
      },
      resolveSession: async () => { throw new Error("should not resolve") },
      attachToSession: async () => ({ id: "attachment-created", session_id: "session-created" }),
      getSessionState: async () => createdSession,
      launchProviderRun: async (_client, _sessionId, provider, _accountProfile, model, effort, agentId) => {
        launched.push({ provider, model, effort, agentId })
        return {
          id: "run-created",
          session_id: "session-created",
          agent_instance_id: agentId ?? null,
          adapter_key: provider,
          provider,
          account_profile: "default",
          model,
          variant: effort,
          usage_tokens_total: null,
          state: "Running",
        }
      },
      tryGetProviderRun: async () => null,
      catchUpAttachedSession: async () => undefined,
      getSessionHistoryOutline: async () => ({ agents: [] }),
      resolveVisibleAgentId: () => null,
      prepareHistoryOutlineAgent: () => [],
    },
  )

  assert.deepEqual(createdAgentDefaults, {
    provider: "opencode",
    model: "opencode/gpt-5.4",
    effort: "high",
    account_profile: "default",
  })
  assert.deepEqual(launched, [{
    provider: "opencode",
    model: "opencode/gpt-5.4",
    effort: "high",
    agentId: null,
  }])
})

test("bootstrapSession reattaches and hydrates missed output from history catch-up", async () => {
  const catalog = fallbackProviderCatalog()
  const session = {
    id: "session-1",
    project_id: "project-default",
    workspace_id: "/workspace",
    worktree_id: "/workspace",
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: "run-1",
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-a",
    max_agents: 8,
    agents: [{
      id: "agent-a",
      agent_ref: "agent-a",
      session_id: "session-1",
      alias: null,
      provider: "opencode",
      model: "gpt-5.4",
      worktree_id: null,
      state: "Idle" as const,
      is_processing: false,
      grid_row: 0,
      grid_col: 0,
      grid_row_span: 1,
      grid_col_span: 1,
      created_at_ms: 1,
      last_activity_at_ms: 1,
    }],
    config_state: { version: 1, values: {} },
  }
  const calls: string[] = []

  const bootstrap = await bootstrapSession(
    {} as never,
    {
      clientId: "cli-1",
      sessionId: "session-1",
      model: "gpt-5.4",
      accountProfile: "default",
      effort: "high",
    },
    "/workspace",
    "/workspace",
    {},
    {
      listSessions: async () => [session],
      getProviderCatalog: async () => catalog,
      getProviderCommandCatalogs: async () => fallbackProviderCommandCatalogs(),
      getTerminalCommandCatalog: async () => terminalCatalog(),
      createSession: async () => { throw new Error("should not create") },
      resolveSession: async () => session,
      attachToSession: async () => {
        calls.push("attach")
        return { id: "attachment-2", session_id: "session-1" }
      },
      getSessionState: async () => {
        calls.push("session")
        return session
      },
      launchProviderRun: async () => { throw new Error("should not relaunch") },
      tryGetProviderRun: async () => {
        calls.push("load-run")
        return {
          id: "run-1",
          session_id: "session-1",
          agent_instance_id: "agent-a",
          adapter_key: "opencode",
          provider: "opencode",
          account_profile: "default",
          model: "gpt-5.4",
          variant: "high",
          usage_tokens_total: null,
          state: "Running",
        }
      },
      catchUpAttachedSession: async () => {
        calls.push("catchup")
      },
      getSessionHistoryOutline: async () => ({
        agents: [outlineAgent("agent-a", "hello\n", "while you were away")],
      }),
      getPromptInputHistory: async () => ({
        entries: [{
          sequence: 1,
          timestamp_ms: 1,
          session_id: "session-1",
          kind: "prompt",
          text: "hello\n",
        }],
      }),
      resolveVisibleAgentId: () => "agent-a",
      prepareHistoryOutlineAgent: hydrateSessionHistoryOutlineAgentEntries,
    },
  )

  assert.deepEqual(calls, ["attach", "session", "load-run", "catchup", "session"])
  assert.equal(bootstrap.binding?.attachment.id, "attachment-2")
  assert.deepEqual(bootstrap.binding?.promptHistoryEntries, [])
  const deferredHistory = await bootstrap.deferred?.attachedHistory
  assert.deepEqual(deferredHistory?.promptHistoryEntries, ["hello"])
  const assistantEntry = deferredHistory?.historyEntries.find((entry) => entry.role === "assistant")
  assert.equal(assistantEntry?.text, "while you were away")
})

test("bootstrapSession skips attach-time launch when focused agent is stale", async () => {
  const warnings: Array<Record<string, unknown> | undefined> = []
  const session = {
    id: "session-1",
    project_id: "project-default",
    workspace_id: "/workspace",
    worktree_id: "/workspace",
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "missing-agent",
    max_agents: 8,
    agents: [{
      id: "agent-a",
      agent_ref: "agent-a",
      session_id: "session-1",
      alias: null,
      provider: "codex",
      model: "codex/gpt-5.4-mini",
      effort: "low",
      worktree_id: null,
      state: "Idle" as const,
      is_processing: false,
      grid_row: 0,
      grid_col: 0,
      grid_row_span: 1,
      grid_col_span: 1,
      created_at_ms: 1,
      last_activity_at_ms: 1,
    }],
    config_state: { version: 1, values: {} },
  }

  const bootstrap = await bootstrapSession(
    {} as never,
    {
      clientId: "cli-1",
      sessionId: "session-1",
      model: "gpt-5.4",
      accountProfile: "default",
      effort: "high",
    },
    "/workspace",
    "/workspace",
    {},
    {
      logger: {
        warn: (_message: string, fields?: Record<string, unknown>) => warnings.push(fields),
      } as never,
      listSessions: async () => [session],
      getProviderCatalog: async () => fallbackProviderCatalog(),
      getProviderCommandCatalogs: async () => fallbackProviderCommandCatalogs(),
      getTerminalCommandCatalog: async () => terminalCatalog(),
      createSession: async () => { throw new Error("should not create") },
      resolveSession: async () => session,
      attachToSession: async () => ({ id: "attachment-1", session_id: "session-1" }),
      getSessionState: async () => session,
      launchProviderRun: async () => { throw new Error("should not launch with stale focus") },
      tryGetProviderRun: async () => null,
      catchUpAttachedSession: async () => undefined,
      getSessionHistoryOutline: async () => ({ agents: [] }),
      resolveVisibleAgentId: () => null,
      prepareHistoryOutlineAgent: () => [],
    },
  )

  assert.equal(bootstrap.binding?.providerRun, null)
  assert.deepEqual(warnings, [{
    session_id: "session-1",
    focused_agent_id: "missing-agent",
  }])
})

test("bootstrapSession recovers when the launch target moves remote during attach", async () => {
  const agent = {
    id: "agent-a",
    agent_ref: "agent-a",
    session_id: "session-1",
    alias: null,
    provider: "claude",
    model: "sonnet",
    effort: "medium",
    worktree_id: null,
    state: "Idle" as const,
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 1,
    last_activity_at_ms: 1,
  }
  const localSession: RuntimeSession = {
    id: "session-1",
    project_id: "project-default",
    workspace_id: "/workspace",
    worktree_id: "/workspace",
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: agent.id,
    max_agents: 8,
    agents: [agent],
    config_state: { version: 1, values: {} },
  }
  const remoteSession: RuntimeSession = {
    ...localSession,
    agents: [{
      ...agent,
      remote_execution: {
        worker_kernel_id: "worker-1",
        worker_machine_id: "machine-1",
        execution_lease_id: "lease-1",
        leased_agent_id: "leased-agent-1",
      },
    }],
  }
  const launchError = new Error("agent became remote-backed")
  const info: Array<{ message: string; fields: Record<string, unknown> | undefined }> = []
  let stateReads = 0
  let catchUpSession: RuntimeSession = localSession

  const bootstrap = await bootstrapSession(
    {} as never,
    {
      clientId: "cli-1",
      sessionId: "session-1",
      provider: "claude",
      model: "sonnet",
      accountProfile: "default",
      effort: "medium",
    },
    "/workspace",
    "/workspace",
    {},
    {
      logger: {
        info: (message: string, fields?: Record<string, unknown>) => info.push({ message, fields }),
      } as never,
      listSessions: async () => [localSession],
      getProviderCatalog: async () => fallbackProviderCatalog(),
      getProviderCommandCatalogs: async () => fallbackProviderCommandCatalogs(),
      getTerminalCommandCatalog: async () => terminalCatalog(),
      createSession: async () => { throw new Error("should not create") },
      resolveSession: async () => localSession,
      attachToSession: async () => ({ id: "attachment-1", session_id: "session-1" }),
      getSessionState: async () => {
        stateReads += 1
        return stateReads === 1 ? localSession : remoteSession
      },
      launchProviderRun: async (_client, _sessionId, _provider, _accountProfile, _model, _effort, agentId) => {
        assert.equal(agentId, agent.id)
        throw launchError
      },
      tryGetProviderRun: async () => null,
      catchUpAttachedSession: async (_client, _sessionId, _attachmentId, session) => {
        catchUpSession = session
      },
      getSessionHistoryOutline: async () => ({ agents: [] }),
      resolveVisibleAgentId: () => agent.id,
      prepareHistoryOutlineAgent: () => [],
    },
  )

  assert.equal(bootstrap.binding?.providerRun, null)
  assert.equal(bootstrap.binding?.session.agents[0]?.remote_execution?.worker_kernel_id, "worker-1")
  assert.equal(catchUpSession.agents[0]?.remote_execution?.worker_kernel_id, "worker-1")
  assert.equal(stateReads, 3)
  assert.deepEqual(info, [{
    message: "recovered attach-time provider launch after agent moved remote",
    fields: {
      session_id: "session-1",
      agent_id: "agent-a",
      worker_kernel_id: "worker-1",
    },
  }])
})

function outlineAgent(
  agentId: string,
  prompt: string,
  summary: string,
  nextCursor: { before_sequence: number } | null = null,
) {
  return {
    agent_id: agentId,
    turns: [{
      turn_id: "turn-1",
      prompt_id: "prompt-1",
      started_at_ms: 1,
      lifecycle: "completed",
      completed_at_ms: 2,
      user_prompt: {
        entry_index: 1,
        fragment_start: 0,
        fragment_end: prompt.length,
        total_chars: prompt.length,
        entry: { kind: "user_prompt" as const, text: prompt, agent_id: agentId },
      },
      entries: [],
      summary: {
        entry_index: 2,
        fragment_start: 0,
        fragment_end: summary.length,
        total_chars: summary.length,
        entry: { kind: "provider_output" as const, text: summary, agent_id: agentId },
      },
      blobs: [],
    }],
    next_cursor: nextCursor,
  }
}
