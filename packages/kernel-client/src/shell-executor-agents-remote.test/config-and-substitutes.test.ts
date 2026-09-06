import {
  assert,
  createDefaultShellContext,
  executeShellCommand,
  fakeClient,
  makeAgent,
  makeSession,
  parseShellCommand,
  test,
} from "../shell-executor-agents-remote.test-support.js"
import type { AgentInstance } from "../shell-executor-agents-remote.test-support.js"

test("executeShellCommand updates agent alias through dedicated alias request", async () => {
  const agent = makeAgent({ id: "agent-2", agent_ref: "agent-2", alias: "reviewer" })
  const renamed = { ...agent, alias: "ui" }
  const requests: unknown[] = []
  const fake = fakeClient((request) => {
    requests.push(request)
    if ("ListAgents" in request) {
      return { AgentsListed: { agents: [agent] } }
    }
    if ("AliasAgent" in request) {
      return { AgentAliased: { agent: renamed, session: makeSession({ agents: [renamed] }) } }
    }
    throw new Error("unexpected request")
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const result = await executeShellCommand(parseShellCommand("agent alias reviewer ui"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /agent-2 \(ui\) alias = ui/)
  assert.deepEqual(requests, [
    { ListAgents: { session_id: "session-1" } },
    {
      AliasAgent: {
        session_id: "session-1",
        agent_id: "agent-2",
        alias: "ui",
      },
    },
  ])
})

test("executeShellCommand updates agent provider profile through dedicated profile request", async () => {
  const agent = makeAgent({ id: "agent-2", agent_ref: "agent-2", alias: "reviewer" })
  const updated = makeAgent({
    id: "agent-2",
    agent_ref: "agent-2",
    alias: "reviewer",
    provider: "codex",
    model: "gpt-5.4",
    effort: "low",
  })
  const requests: unknown[] = []
  const fake = fakeClient((request) => {
    requests.push(request)
    if ("ListAgents" in request) {
      return { AgentsListed: { agents: [agent] } }
    }
    if ("UpdateAgentProfile" in request) {
      return { AgentProfileUpdated: { agent: updated, session: makeSession({ agents: [updated] }) } }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const result = await executeShellCommand(parseShellCommand("agent provider reviewer codex"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /agent-2 \(reviewer\) provider = codex/)
  assert.deepEqual(requests, [
    { ListAgents: { session_id: "session-1" } },
    {
      UpdateAgentProfile: {
        session_id: "session-1",
        agent_id: "agent-2",
        provider: "codex",
        model: null,
        effort: null,
        clear_effort: false,
      },
    },
  ])
})

test("executeShellCommand clears agent variant through dedicated profile request", async () => {
  const agent = makeAgent({ effort: "low" })
  const updated = makeAgent({ effort: null })
  const requests: unknown[] = []
  const fake = fakeClient((request) => {
    requests.push(request)
    if ("ListAgents" in request) {
      return { AgentsListed: { agents: [agent] } }
    }
    if ("UpdateAgentProfile" in request) {
      return { AgentProfileUpdated: { agent: updated, session: makeSession({ agents: [updated] }) } }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    agentId: "agent-1",
  })
  const result = await executeShellCommand(parseShellCommand("agent variant none"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /agent-1 variant = <none>/)
  assert.deepEqual(requests, [
    { ListAgents: { session_id: "session-1" } },
    {
      UpdateAgentProfile: {
        session_id: "session-1",
        agent_id: "agent-1",
        provider: null,
        model: null,
        effort: null,
        clear_effort: true,
      },
    },
  ])
})

test("executeShellCommand updates agent mode through dedicated config request", async () => {
  const agent = makeAgent({ id: "agent-2", agent_ref: "agent-2", alias: "reviewer" })
  const updated = makeAgent({
    id: "agent-2",
    agent_ref: "agent-2",
    alias: "reviewer",
    execution_mode_override: "plan",
  })
  const requests: unknown[] = []
  const fake = fakeClient((request) => {
    requests.push(request)
    if ("ListAgents" in request) {
      return { AgentsListed: { agents: [agent] } }
    }
    if ("UpdateAgentConfig" in request) {
      return { AgentConfigUpdated: { agent: updated, session: makeSession({ agents: [updated] }) } }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const result = await executeShellCommand(parseShellCommand("agent mode reviewer plan"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /agent-2 \(reviewer\) mode = plan \(agent\)/)
  assert.deepEqual(requests, [
    { ListAgents: { session_id: "session-1" } },
    {
      UpdateAgentConfig: {
        session_id: "session-1",
        agent_id: "agent-2",
        execution_mode: "plan",
        clear_execution_mode: false,
        permission_level: null,
        clear_permission_level: false,
        workspace_id: null,
        clear_workspace_id: false,
        worktree_id: null,
        clear_worktree_id: false,
      },
    },
  ])
})

test("executeShellCommand clears agent mode override through dedicated config request", async () => {
  const agent = makeAgent({ execution_mode_override: "plan" })
  const updated = makeAgent({ execution_mode_override: null })
  const requests: unknown[] = []
  const fake = fakeClient((request) => {
    requests.push(request)
    if ("ListAgents" in request) {
      return { AgentsListed: { agents: [agent] } }
    }
    if ("UpdateAgentConfig" in request) {
      return {
        AgentConfigUpdated: {
          agent: updated,
          session: makeSession({
            agents: [updated],
            config_state: { version: 1, values: { "agents.mode": "build" } },
          }),
        },
      }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    agentId: "agent-1",
  })
  const result = await executeShellCommand(parseShellCommand("agent mode inherit"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /agent-1 mode = build \(session\)/)
  assert.deepEqual(requests, [
    { ListAgents: { session_id: "session-1" } },
    {
      UpdateAgentConfig: {
        session_id: "session-1",
        agent_id: "agent-1",
        execution_mode: null,
        clear_execution_mode: true,
        permission_level: null,
        clear_permission_level: false,
        workspace_id: null,
        clear_workspace_id: false,
        worktree_id: null,
        clear_worktree_id: false,
      },
    },
  ])
})

test("executeShellCommand manages agent substitutes", async () => {
  const baseAgent = makeAgent()
  const substituteAgent = makeAgent({
    provider: "codex",
    model: "gpt-5.4",
    effort: "medium",
    substitutes: [{ provider: "codex", model: "gpt-5.4", variant: "medium" }],
    active_substitute_index: 0,
  })
  const fake = fakeClient((request) => {
    if ("ListAgents" in request) {
      return { AgentsListed: { agents: [baseAgent] } }
    }
    if ("UpdateAgentSubstitutes" in request) {
      const payload = request.UpdateAgentSubstitutes as {
        action: Record<string, unknown>
      }
      if ("Add" in payload.action) {
        return {
          AgentConfigUpdated: {
            agent: makeAgent({ substitutes: [{ provider: "codex", model: "gpt-5.4", variant: "medium" }] }),
            session: makeSession(),
          },
        }
      }
      if ("Activate" in payload.action) {
        return { AgentConfigUpdated: { agent: substituteAgent, session: makeSession({ agents: [substituteAgent] }) } }
      }
    }
    if ("LaunchProviderRun" in request) {
      return {
        ProviderRunLaunchAccepted: {
          provider_run: {
            id: "run-sub",
            session_id: "session-1",
            agent_instance_id: "agent-1",
            adapter_key: "codex",
            provider: "codex",
            account_profile: "default",
            model: "gpt-5.4",
            variant: "medium",
            usage_tokens_total: null,
            state: "Starting",
          },
        },
      }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    agentId: "agent-1",
  })
  const addResult = await executeShellCommand(
    parseShellCommand("agent substitute add codex gpt-5.4 --variant medium --kernel kernel-local --worktree /repo/sub"),
    context,
    { client: fake.client },
  )
  const activateResult = await executeShellCommand(
    parseShellCommand("agent substitute activate 0"),
    context,
    { client: fake.client },
  )
  assert.equal(addResult.ok, true)
  assert.match(addResult.message ?? "", /substitute added/)
  assert.equal(activateResult.ok, true)
  assert.match(activateResult.message ?? "", /activated substitute 0/)
  assert.deepEqual(fake.requests.map((request) => Object.keys(request)[0]), [
    "ListAgents",
    "UpdateAgentSubstitutes",
    "ListAgents",
    "UpdateAgentSubstitutes",
    "LaunchProviderRun",
  ])
  const addRequest = fake.requests[1]
  assert.ok(addRequest && "UpdateAgentSubstitutes" in addRequest)
  const addPayload = addRequest.UpdateAgentSubstitutes as { action: unknown }
  assert.deepEqual(addPayload.action, {
    Add: {
      provider: "codex",
      model: "gpt-5.4",
      variant: "medium",
      kernel_id: "kernel-local",
      worktree_id: "/repo/sub",
    },
  })
})

test("executeShellCommand reorders substitutes and resets to the starter", async () => {
  const starter = makeAgent({
    provider: "claude",
    model: "claude-opus-4-8",
    effort: "high",
    substitutes: [
      { provider: "opencode-go", model: "deepseek-v4-pro", variant: "high" },
      { provider: "opencode", model: "deepseek-v4-pro", variant: "high" },
      { provider: "codex", model: "gpt-5.6-sol", variant: "high" },
    ],
  })
  const reordered = makeAgent({
    ...starter,
    provider: "opencode",
    model: "deepseek-v4-pro",
    substitutes: [
      { provider: "opencode", model: "deepseek-v4-pro", variant: "high" },
      { provider: "opencode-go", model: "deepseek-v4-pro", variant: "high" },
      { provider: "codex", model: "gpt-5.6-sol", variant: "high" },
    ],
    active_substitute_index: 0,
  })
  const fake = fakeClient((request) => {
    if ("ListAgents" in request) {
      return { AgentsListed: { agents: [starter] } }
    }
    if ("UpdateAgentSubstitutes" in request) {
      const payload = request.UpdateAgentSubstitutes as { action: Record<string, unknown> }
      if ("Move" in payload.action) {
        return { AgentConfigUpdated: { agent: reordered, session: makeSession({ agents: [reordered] }) } }
      }
      if ("Primary" in payload.action) {
        return { AgentConfigUpdated: { agent: starter, session: makeSession({ agents: [starter] }) } }
      }
    }
    if ("LaunchProviderRun" in request) {
      return {
        ProviderRunLaunchAccepted: {
          provider_run: {
            id: "run-starter",
            session_id: "session-1",
            agent_instance_id: "agent-1",
            adapter_key: "claude",
            provider: "claude",
            account_profile: "default",
            model: "claude-opus-4-8",
            variant: "high",
            usage_tokens_total: null,
            state: "Starting",
          },
        },
      }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    agentId: "agent-1",
  })

  const moveResult = await executeShellCommand(parseShellCommand("agent substitute move 1 0"), context, { client: fake.client })
  const resetResult = await executeShellCommand(parseShellCommand("agent substitute reset"), context, { client: fake.client })

  assert.equal(moveResult.ok, true)
  assert.match(moveResult.message ?? "", /substitute moved from 1 to 0/)
  assert.equal(resetResult.ok, true)
  assert.match(resetResult.message ?? "", /reset to starter profile/)
  const updateActions = fake.requests
    .filter((request) => "UpdateAgentSubstitutes" in request)
    .map((request) => (request.UpdateAgentSubstitutes as { action: unknown }).action)
  assert.deepEqual(updateActions, [
    { Move: { from_index: 1, to_index: 0 } },
    { Primary: {} },
  ])
})
