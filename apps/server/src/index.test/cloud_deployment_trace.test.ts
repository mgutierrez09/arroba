import {
  acquireAgentAppReplica,
  appendCloudPublicationDeploymentLogs,
  assert,
  baseConfig,
  buildServer,
  clearAgentAppEffectStoresForTests,
  clearAgentAppReplicaPoolsForTests,
  collectPublicationTraceEvents,
  createPublicationTraceStreamState,
  createServer,
  findWorkflowRunByInvocationRequestId,
  firstSetCookieValue,
  invokePublicationInput,
  join,
  loadPublicationConfigFromKernel,
  loadPublicationPackageConfig,
  mkdir,
  mkdtemp,
  promptFromInvocationInput,
  providerCatalogResponse,
  publicationConfigFromKernelRecord,
  publicationConfigFromPackage,
  publicationCloudBackendIngress,
  publicationForAgentAppInvocation,
  publicationInvocationEnvelope,
  publishedHttpConfig,
  readFile,
  registerCloudPublicationDeploymentBackend,
  releaseAgentAppReplicaInvocation,
  rememberAgentAppInvocationRoute,
  rm,
  setOptionalEnv,
  test,
  tmpdir,
  visibleWorkflowRun,
  waitForCondition,
  writeFile,
  type WorkflowPublicationConfig,
} from "../index.test-support.js"
import { readConnectedPublicationOperationalStatus } from "../publication-cloud-operational-status.js"

test("connected backend refresh reads only its publication runs and never pumps the workflow", async () => {
  const publication = { ...baseConfig, workflow_ref: "workflow-1", endpoint_ref: "endpoint-1" }
  const requests: string[] = []
  const client = {
    async send<T>(request: unknown): Promise<T> {
      const kind = Object.keys(request as object)[0]!
      requests.push(kind)
      const responses: Record<string, unknown> = {
        ListWorkflowRuns: { WorkflowRunsListed: { workflow_runs: [
          { id: "ours", workflow_id: "workflow-1", endpoint_id: "endpoint-1", status: "Completed", created_at_ms: 10,
            publication_invocation: { publication_id: publication.publication_id, invocation_id: "our-invocation" }, final_output: "private" },
          { id: "other-publication", workflow_id: "workflow-1", endpoint_id: "endpoint-1", publication_invocation: { publication_id: "other" } },
          { id: "manual", workflow_id: "workflow-1", endpoint_id: "endpoint-1" },
          { id: "other-endpoint", workflow_id: "workflow-1", endpoint_id: "endpoint-2", publication_invocation: { publication_id: publication.publication_id } },
        ] } },
        ListQueuedWorkflowPrompts: { QueuedWorkflowPromptsListed: { queued_prompts: [] } },
        ListWorkflowPromptQueues: { WorkflowPromptQueuesListed: { queues: [{ id: "queue-1", alias: "default" }] } },
      }
      assert.ok(kind in responses, `Unexpected mutating or detailed-history request: ${kind}`)
      return responses[kind] as T
    },
  }
  const operationalStatus = await readConnectedPublicationOperationalStatus(client, publication)
  let payload: Record<string, unknown> | undefined
  await registerCloudPublicationDeploymentBackend({
    deploymentId: "connected",
    publication,
    operationalStatus,
    localUrl: "http://127.0.0.1:4567/",
    profile: { apiUrl: "https://cloud.test", accountId: "account" },
    now: () => 100,
    fetch: async (_url, init) => {
      payload = JSON.parse(String(init?.body))
      return new Response(null, { status: 200 })
    },
  })
  assert.deepEqual(payload?.backendTarget, {
    kind: "local_runtime", url: "http://127.0.0.1:4567/", updated_at_ms: 100,
    queueDepth: 0,
    runs: [{ id: "ours", status: "Completed", created_at_ms: 10, publication_invocation: { invocation_id: "our-invocation" } }],
  })
  assert.equal(requests.length, 3)
})

test("publication gateway registers local runtime backend with Cloud deployment", async () => {
  const calls: Array<{ url: string; init: RequestInit }> = []
  const registered = await registerCloudPublicationDeploymentBackend({
    deploymentId: "deployment-1",
    publication: baseConfig,
    localUrl: "http://127.0.0.1:4567/",
    operationalStatus: {
      queue_depth: 2,
      recent_runs: [{
        id: "run-connected",
        status: "Completed",
        created_at_ms: 1_699_999_999_000,
        completed_at_ms: 1_699_999_999_900,
        publication_invocation: { invocation_id: "invocation-connected", caller: { token: "private-caller" } },
        final_output: { message: "private-output" },
        prompt: "private-prompt",
      }],
      latest_output: { message: "private-output" },
    },
    now: () => 1_700_000_000_000,
    profile: {
      apiUrl: "https://cloud.example.test/",
      accountId: "account-1",
      cloudSessionToken: "session-token",
    },
    fetch: async (url, init) => {
      calls.push({ url: String(url), init: init ?? {} })
      return new Response(JSON.stringify({ deployment: { id: "deployment-1" } }), { status: 200 })
    },
  })

  assert.equal(registered, true)
  assert.equal(calls[0]?.url, "https://cloud.example.test/publication-deployments/deployment-1/local-backend")
  assert.equal((calls[0]?.init.headers as Record<string, string>).authorization, "Bearer session-token")
  assert.deepEqual(JSON.parse(String(calls[0]?.init.body)), {
    accountId: "account-1",
    status: "ready",
    runtimeSessionId: "session-1",
    backendTarget: {
      kind: "local_runtime",
      url: "http://127.0.0.1:4567/",
      updated_at_ms: 1_700_000_000_000,
      queueDepth: 2,
      runs: [{
        id: "run-connected",
        status: "Completed",
        created_at_ms: 1_699_999_999_000,
        completed_at_ms: 1_699_999_999_900,
        publication_invocation: { invocation_id: "invocation-connected" },
      }],
    },
  })
})

test("publication gateway includes Agent App replica status in local runtime backend", async () => {
  const calls: Array<{ url: string; init: RequestInit }> = []
  const registered = await registerCloudPublicationDeploymentBackend({
    deploymentId: "deployment-agent-app",
    publication: {
      ...baseConfig,
      publication_id: "pub-agent-app-local",
      replica_session_ids: ["replica-session-1", "replica-session-2"],
      agent_app: {
        enabled: true,
        assets: { public_dir: "app", index: "index.html" },
        replicas: { count: 2, per_caller_ordering: true },
        routes: [{
          path: "/add/*",
          hook_id: "pub-test-hook",
          prompt_source: "path_tail",
          response: "streaming_shell",
        }],
      },
    },
    localUrl: "http://127.0.0.1:4567/",
    now: () => 1_700_000_000_000,
    profile: {
      apiUrl: "https://cloud.example.test/",
      accountId: "account-1",
    },
    fetch: async (url, init) => {
      calls.push({ url: String(url), init: init ?? {} })
      return new Response(JSON.stringify({ deployment: { id: "deployment-agent-app" } }), { status: 200 })
    },
  })

  assert.equal(registered, true)
  assert.deepEqual(JSON.parse(String(calls[0]?.init.body)).backendTarget, {
    kind: "local_runtime",
    url: "http://127.0.0.1:4567/",
    updated_at_ms: 1_700_000_000_000,
    queueDepth: 0,
    activeReplicaCount: 0,
    readyReplicaCount: 2,
  })
})

test("publication gateway can mark Cloud local runtime backend unavailable", async () => {
  const calls: Array<{ url: string; init: RequestInit }> = []
  const registered = await registerCloudPublicationDeploymentBackend({
    deploymentId: "deployment-unavailable",
    publication: baseConfig,
    status: "unavailable",
    lastError: "relay display tunnel unavailable",
    now: () => 1_700_000_000_000,
    profile: {
      apiUrl: "https://cloud.example.test/",
      accountId: "account-1",
      cloudSessionToken: "session-token",
    },
    fetch: async (url, init) => {
      calls.push({ url: String(url), init: init ?? {} })
      return new Response(JSON.stringify({ deployment: { id: "deployment-unavailable" } }), { status: 200 })
    },
  })

  assert.equal(registered, true)
  assert.deepEqual(JSON.parse(String(calls[0]?.init.body)), {
    accountId: "account-1",
    status: "unavailable",
    runtimeSessionId: "session-1",
    lastError: "relay display tunnel unavailable",
  })
})

test("publication gateway can register Cloud backend from env profile", async () => {
  const previous = {
    apiUrl: process.env.CHARIOX_PUBLICATION_CLOUD_API_URL,
    accountId: process.env.CHARIOX_PUBLICATION_CLOUD_ACCOUNT_ID,
    token: process.env.CHARIOX_PUBLICATION_CLOUD_SESSION_TOKEN,
  }
  process.env.CHARIOX_PUBLICATION_CLOUD_API_URL = "https://cloud-env.example.test/"
  process.env.CHARIOX_PUBLICATION_CLOUD_ACCOUNT_ID = "account-env"
  process.env.CHARIOX_PUBLICATION_CLOUD_SESSION_TOKEN = "token-env"
  try {
    const calls: Array<{ url: string; init: RequestInit }> = []
    const registered = await registerCloudPublicationDeploymentBackend({
      deploymentId: "deployment-env",
      publication: baseConfig,
      localUrl: "http://127.0.0.1:4568/",
      fetch: async (url, init) => {
        calls.push({ url: String(url), init: init ?? {} })
        return new Response(JSON.stringify({ deployment: { id: "deployment-env" } }), { status: 200 })
      },
    })

    assert.equal(registered, true)
    assert.equal(calls[0]?.url, "https://cloud-env.example.test/publication-deployments/deployment-env/local-backend")
    assert.equal((calls[0]?.init.headers as Record<string, string>).authorization, "Bearer token-env")
    assert.equal(JSON.parse(String(calls[0]?.init.body)).accountId, "account-env")
  } finally {
    setOptionalEnv("CHARIOX_PUBLICATION_CLOUD_API_URL", previous.apiUrl)
    setOptionalEnv("CHARIOX_PUBLICATION_CLOUD_ACCOUNT_ID", previous.accountId)
    setOptionalEnv("CHARIOX_PUBLICATION_CLOUD_SESSION_TOKEN", previous.token)
  }
})

test("publication gateway registers Cloud backend from canonical daemon profile", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-publication-cloud-profile-"))
  const previous = {
    charioxHome: process.env.CHARIOX_HOME,
    apiUrl: process.env.CHARIOX_PUBLICATION_CLOUD_API_URL,
    accountId: process.env.CHARIOX_PUBLICATION_CLOUD_ACCOUNT_ID,
    token: process.env.CHARIOX_PUBLICATION_CLOUD_SESSION_TOKEN,
  }
  await mkdir(join(root, "daemon"), { recursive: true })
  await writeFile(join(root, "daemon", "config.json"), JSON.stringify({
    cloud_relay: {
      api_url: "https://cloud-daemon.example.test/",
      account_id: "account-daemon",
      cloud_session_token: "token-daemon",
    },
  }))
  process.env.CHARIOX_HOME = root
  delete process.env.CHARIOX_PUBLICATION_CLOUD_API_URL
  delete process.env.CHARIOX_PUBLICATION_CLOUD_ACCOUNT_ID
  delete process.env.CHARIOX_PUBLICATION_CLOUD_SESSION_TOKEN
  try {
    const calls: Array<{ url: string; init: RequestInit }> = []
    const registered = await registerCloudPublicationDeploymentBackend({
      deploymentId: "deployment-daemon",
      publication: baseConfig,
      localUrl: "http://127.0.0.1:4569/",
      fetch: async (url, init) => {
        calls.push({ url: String(url), init: init ?? {} })
        return new Response(JSON.stringify({ deployment: { id: "deployment-daemon" } }), { status: 200 })
      },
    })

    assert.equal(registered, true)
    assert.equal(calls[0]?.url, "https://cloud-daemon.example.test/publication-deployments/deployment-daemon/local-backend")
    assert.equal((calls[0]?.init.headers as Record<string, string>).authorization, "Bearer token-daemon")
    assert.equal(JSON.parse(String(calls[0]?.init.body)).accountId, "account-daemon")
  } finally {
    setOptionalEnv("CHARIOX_HOME", previous.charioxHome)
    setOptionalEnv("CHARIOX_PUBLICATION_CLOUD_API_URL", previous.apiUrl)
    setOptionalEnv("CHARIOX_PUBLICATION_CLOUD_ACCOUNT_ID", previous.accountId)
    setOptionalEnv("CHARIOX_PUBLICATION_CLOUD_SESSION_TOKEN", previous.token)
    await rm(root, { recursive: true, force: true })
  }
})

test("hosted publication container skips local runtime Cloud backend registration", () => {
  assert.deepEqual(publicationCloudBackendIngress({
    cloudDeploymentId: "deployment-hosted",
    cloudRunnerKey: "runner-secret",
    access: "local",
  }), { kind: "hosted_container" })
})

test("connected publication without a relay tunnel marks the Cloud backend unavailable", () => {
  const decision = publicationCloudBackendIngress({
    cloudDeploymentId: "deployment-connected",
    cloudRunnerKey: null,
    access: "local",
  })
  assert.equal(decision.kind, "unavailable")
  assert.equal(
    decision.kind === "unavailable" ? decision.lastError : "",
    "Cloud local-runtime publication requires a relay display tunnel; endpoint registered with access local",
  )
})

test("connected publication with a relay tunnel registers the Cloud backend ready", () => {
  assert.deepEqual(publicationCloudBackendIngress({
    cloudDeploymentId: "deployment-tunnel",
    access: "tunnel",
  }), { kind: "ready" })
  assert.deepEqual(publicationCloudBackendIngress({
    cloudDeploymentId: null,
    cloudRunnerKey: null,
    access: "local",
  }), { kind: "no_cloud_deployment" })
})

test("publication gateway appends account-scoped deployment logs", async () => {
  const calls: Array<{ url: string; init: RequestInit }> = []
  const appended = await appendCloudPublicationDeploymentLogs({
    deploymentId: "deployment-log",
    profile: {
      apiUrl: "https://cloud.example.test/",
      accountId: "account-1",
      cloudSessionToken: "session-token",
    },
    entries: [{
      level: "info",
      message: "agent app action `cart.add` completed",
      metadata: { kind: "agent_app_action", action_id: "cart.add" },
    }],
    fetch: async (url, init) => {
      calls.push({ url: String(url), init: init ?? {} })
      return new Response(JSON.stringify({ logs: [] }), { status: 201 })
    },
  })

  assert.equal(appended, true)
  assert.equal(calls[0]?.url, "https://cloud.example.test/publication-deployments/deployment-log/logs")
  assert.equal((calls[0]?.init.headers as Record<string, string>).authorization, "Bearer session-token")
  assert.deepEqual(JSON.parse(String(calls[0]?.init.body)), {
    accountId: "account-1",
    entries: [{
      level: "info",
      message: "agent app action `cart.add` completed",
      metadata: { kind: "agent_app_action", action_id: "cart.add" },
    }],
  })
})

test("publication gateway appends runner-scoped deployment logs", async () => {
  const calls: Array<{ url: string; init: RequestInit }> = []
  const appended = await appendCloudPublicationDeploymentLogs({
    deploymentId: "deployment-log",
    profile: { apiUrl: "https://cloud.example.test/", accountId: "account-1" },
    runnerKey: "runner-secret",
    entries: [{ level: "warn", message: "agent app action `cart.add` failed" }],
    fetch: async (url, init) => {
      calls.push({ url: String(url), init: init ?? {} })
      return new Response(JSON.stringify({ logs: [] }), { status: 201 })
    },
  })

  assert.equal(appended, true)
  assert.equal(calls[0]?.url, "https://cloud.example.test/runner/publication-deployments/deployment-log/logs")
  assert.deepEqual(JSON.parse(String(calls[0]?.init.body)), {
    runnerKey: "runner-secret",
    entries: [{ level: "warn", message: "agent app action `cart.add` failed" }],
  })
})

test("publication trace events honor per-node level policy", () => {
  const publication: WorkflowPublicationConfig = {
    ...baseConfig,
    trace_exposure: {
      nodes: {
        "node-a": ["output_summary"],
        "node-b": ["output_summary", "assistant_messages"],
        "node-c": ["output_summary", "assistant_messages", "thinking"],
        "node-d": ["output_summary", "assistant_messages", "thinking", "tool_use"],
      },
    },
    trace_context: {
      nodes: {
        "node-a": { node_id: "node-a", node_label: "Summarizer", agent_id: "agent-a", agent_alias: "summary" },
        "node-b": { node_id: "node-b", node_label: "Research", agent_id: "agent-b", agent_alias: "researcher" },
        "node-c": { node_id: "node-c", node_label: "Planner", agent_id: "agent-c", agent_alias: "planner" },
        "node-d": { node_id: "node-d", node_label: "Builder", agent_id: "agent-d", agent_alias: "builder" },
        "node-e": { node_id: "node-e", node_label: "Hidden", agent_id: "agent-e", agent_alias: "hidden" },
      },
    },
  }
  const workflowRun = {
    id: "run-1",
    status: "Completed",
    publication_invocation: {
      publication_id: "publication-1",
      invocation_id: "invocation-1",
      transport: "human_http",
      endpoint_id: "endpoint-1",
      input: { prompt: "Build the requested dashboard" },
      artifacts: [],
      mode: "async" as const,
      caller: {},
    },
    node_runs: [{
      id: "run-node-a",
      node_id: "node-a",
      agent_id: "agent-a-runtime",
      status: "Completed",
      summary: "A summary",
      completion: { summary: "A completion", output: { message: "A assistant output" } },
      thinking_traces: [{ id: "thinking-a", message: "A private reasoning", timestamp_ms: 11 }],
      completed_at_ms: 20,
    }, {
      id: "run-node-b",
      node_id: "node-b",
      agent_id: "agent-b-runtime",
      status: "Completed",
      summary: "B summary",
      completion: { summary: "B completion", output: { message: "B assistant output" } },
      thinking_traces: [{ id: "thinking-b", message: "B private reasoning", timestamp_ms: 21 }],
      completed_at_ms: 30,
    }, {
      id: "run-node-c",
      node_id: "node-c",
      agent_id: "agent-c-runtime",
      status: "Completed",
      completion: { summary: "C summary" },
      thinking_traces: [{ id: "thinking-c", message: "C thinking", timestamp_ms: 31 }],
      completed_at_ms: 40,
    }, {
      id: "run-node-d",
      node_id: "node-d",
      agent_id: "agent-d-runtime",
      status: "Completed",
      completion: { summary: "D summary", output: { message: "D assistant output" } },
      thinking_traces: [{ id: "thinking-d", message: "D thinking", timestamp_ms: 41 }],
      turn_envelope: {
        runtime_tool_calls: [{
          tool_name: "lookup",
          arguments_json: "{\"q\":\"d\"}",
          result_json: "{\"ok\":true}",
          ok: true,
          timestamp_ms: 52,
        }],
      },
      completed_at_ms: 50,
    }, {
      id: "run-node-e",
      node_id: "node-e",
      agent_id: "agent-e-runtime",
      status: "Completed",
      completion: { summary: "E summary", output: { message: "E assistant output" } },
      thinking_traces: [{ id: "thinking-e", message: "E thinking", timestamp_ms: 51 }],
      completed_at_ms: 60,
    }],
    messages: [
      {
        id: "publication-prompt:run-1:node-b",
        source_node_run_id: "run-node-b",
        target_node_id: "node-b",
        message_type: "user_prompt",
        summary: "Build the requested dashboard",
        handoff_payload: "",
        created_at_ms: 10,
      },
      {
        id: "message-b",
        source_node_run_id: "run-node-b",
        target_node_id: "node-c",
        message_type: "handoff",
        summary: "B handoff",
        handoff_payload: "{\"completion\":{\"summary\":\"TRACE_SUMMARY B hidden\",\"output\":{\"message\":\"B assistant output\"}}}",
        created_at_ms: 25,
      },
      {
        id: "message-c",
        source_node_run_id: "run-node-c",
        target_node_id: "node-d",
        message_type: "handoff",
        summary: "C handoff",
        handoff_payload: "{\"summary\":\"C handoff\"}",
        created_at_ms: 35,
      },
      {
        id: "message-d",
        source_node_run_id: "run-node-d",
        target_node_id: "node-e",
        message_type: "handoff",
        summary: "D handoff",
        handoff_payload: "{\"summary\":\"D handoff\"}",
        created_at_ms: 45,
      },
    ],
    final_output: { message: { kind: "html", html: "<main>C assistant output</main>" } },
    completed_by_node_run_id: "run-node-c",
  }

  const state = createPublicationTraceStreamState()
  const firstPass = collectPublicationTraceEvents(publication, workflowRun, state)
  const secondPass = collectPublicationTraceEvents(publication, workflowRun, state)

  assert.deepEqual(firstPass.map((event) => [event.node_id, event.agent_alias, event.level, event.message]), [
    ["node-a", "summary", "user_prompt", "Build the requested dashboard"],
    ["node-b", "researcher", "user_prompt", "Build the requested dashboard"],
    ["node-c", "planner", "user_prompt", "Build the requested dashboard"],
    ["node-d", "builder", "user_prompt", "Build the requested dashboard"],
    ["node-b", "researcher", "assistant_messages", "B handoff"],
    ["node-b", "researcher", "assistant_messages", "B assistant output"],
    ["node-c", "planner", "thinking", "C thinking"],
    ["node-c", "planner", "assistant_messages", "C handoff"],
    ["node-c", "planner", "assistant_messages", "{\"message\":{\"kind\":\"html\",\"html\":\"<main>C assistant output</main>\"}}"],
    ["node-d", "builder", "thinking", "D thinking"],
    ["node-d", "builder", "assistant_messages", "D handoff"],
    ["node-d", "builder", "assistant_messages", "D assistant output"],
    ["node-d", "builder", "tool_use", "lookup ok"],
    ["node-a", "summary", "output_summary", "A completion"],
    ["node-b", "researcher", "output_summary", "B completion"],
    ["node-c", "planner", "output_summary", "C summary"],
    ["node-d", "builder", "output_summary", "D summary"],
  ])
  assert.equal(firstPass.some((event) => event.node_id === "node-e"), false)
  assert.equal(JSON.stringify(firstPass).includes("TRACE_SUMMARY B hidden"), false)
  assert.match(JSON.stringify(firstPass), /B assistant output/)
  assert.doesNotMatch(JSON.stringify(firstPass), /arguments_json|result_json/)
  assert.deepEqual(firstPass.map((event) => event.sequence), [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17])
  assert.deepEqual(secondPass, [])
})

test("visible workflow run hides unexposed trace levels", () => {
  const workflowRun = {
    id: "run-visibility",
    status: "Completed",
    created_at_ms: 9,
    publication_invocation: {
      publication_id: "publication-1",
      invocation_id: "request-1",
      transport: "human_http",
      endpoint_id: "endpoint-1",
      input: { prompt: "TRACE_PROMPT visible prompt" },
      artifacts: [],
      mode: "async" as const,
      caller: {},
    },
    final_output: { message: "TRACE_FINAL visible" },
    intermediate_outputs: [{ id: "partial-1", output: { message: "partial visible" }, valid: true }],
    node_runs: [{
      id: "run-node-1",
      node_id: "node-1",
      agent_id: "agent-1",
      status: "Completed",
      summary: "TRACE_SUMMARY hidden summary",
      completion: { summary: "TRACE_SUMMARY hidden completion", output: { message: "TRACE_ASSISTANT hidden output" } },
      thinking_traces: [{ id: "thinking-1", message: "hidden thinking", timestamp_ms: 10 }],
      turn_envelope: {
        runtime_tool_calls: [{
          tool_name: "lookup",
          arguments_json: "{\"secret\":true}",
          result_json: "{\"TRACE_TOOL\":\"hidden\"}",
          ok: true,
          timestamp_ms: 11,
        }],
      },
      completed_at_ms: 12,
    }],
    messages: [{
      id: "message-1",
      source_node_run_id: "run-node-1",
      target_node_id: "node-2",
      message_type: "handoff",
      summary: "TRACE_ASSISTANT hidden message",
      handoff_payload: "{\"completion\":{\"summary\":\"TRACE_SUMMARY hidden handoff\",\"output\":{\"message\":\"TRACE_ASSISTANT hidden\"}}}",
      created_at_ms: 13,
    }],
  }

  const hidden = visibleWorkflowRun(baseConfig, workflowRun)
  const hiddenText = JSON.stringify(hidden)
  assert.match(hiddenText, /TRACE_FINAL visible/)
  assert.match(hiddenText, /partial visible/)
  assert.doesNotMatch(hiddenText, /TRACE_SUMMARY|TRACE_ASSISTANT|TRACE_TOOL|thinking_traces|runtime_tool_calls/)

  const exposed = visibleWorkflowRun({
    ...baseConfig,
    trace_exposure: { nodes: { "node-1": ["output_summary", "assistant_messages", "thinking", "tool_use"] } },
  }, workflowRun)
  const exposedText = JSON.stringify(exposed)
  assert.match(exposedText, /TRACE_SUMMARY hidden summary/)
  assert.match(exposedText, /TRACE_ASSISTANT hidden output/)
  assert.match(exposedText, /hidden thinking/)
  assert.match(exposedText, /runtime_tool_calls/)
  assert.match(exposedText, /lookup/)
  assert.doesNotMatch(exposedText, /arguments_json|result_json|TRACE_TOOL/)
  assert.doesNotMatch(exposedText, /TRACE_SUMMARY hidden handoff/)
  assert.match(exposedText, /TRACE_ASSISTANT hidden/)
  assert.match(exposedText, /"message_type":"user_prompt"/)
  assert.match(exposedText, /TRACE_PROMPT visible prompt/)
})

test("publication trace events omit empty output summaries", () => {
  const events = collectPublicationTraceEvents({
    ...baseConfig,
    trace_exposure: { nodes: { "node-empty": ["output_summary"] } },
  }, {
    id: "run-empty-summary",
    status: "Stopped",
    node_runs: [{
      id: "run-node-empty",
      node_id: "node-empty",
      agent_id: "agent-empty",
      status: "Stopped",
      summary: "   ",
      completion: { summary: "" },
    }],
  }, createPublicationTraceStreamState())

  assert.deepEqual(events, [])
})
