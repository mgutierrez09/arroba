import assert from "node:assert/strict"
import test from "node:test"

import { buildCliAutomationSnapshot } from "./cli-automation-snapshot.js"
import type { ShellContext } from "@chariox/kernel-client/shell-core"
import {
  EXTERNAL_PROVIDER_OBSERVED_SOURCE,
} from "@chariox/kernel-client/external-provider-observation"
import type { AgentInstance, RuntimeSession, ProviderAccountProfile } from "./cli-types.js"
import { fallbackProviderCatalog } from "./provider-catalog.js"
import { DEFAULT_THEME_REGISTRY } from "./theme-registry.js"
import { createWaitingRoomState } from "./waiting-room-state.js"
import type { RelayStatusView } from "./relay-api.js"

test("buildCliAutomationSnapshot projects session and interaction state for automation", () => {
  const catalog = fallbackProviderCatalog()
  const agent = {
    id: "agent-1",
    agent_ref: "A",
    alias: "frontend",
    provider: "opencode",
    model: "sonnet",
    effort: "high",
    account_profile: "work",
    execution_mode_override: "plan",
    permission_level_override: "required",
    primary_provider: "opencode",
    primary_model: "sonnet",
    primary_effort: "medium",
    worktree_id: "/worker/repo",
    remote_execution: {
      worker_machine_id: "machine-1",
      worker_kernel_id: "kernel-1",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
      active_worker_provider_run_id: "provider-run-1",
    },
    visible_in_freeform: false,
    state: "Idle",
    is_processing: false,
  } as AgentInstance
  const session = {
    id: "session-1",
    workspace_id: "/repo",
    worktree_id: "/repo",
    focused_agent_id: "agent-1",
    agents: [agent],
    active_interactions: [{
      id: "interaction-1",
      agent_id: "agent-1",
      kind: "choice",
      level: "info",
      title: "Approve",
      message: "Continue?",
      timeout_sec: null,
      default_on_timeout: null,
      requested_at_ms: 1,
      choices: [{ id: "yes", label: "Yes", reply: "yes", style: "primary" }],
    }],
    agent_activity_revision: 7,
    workflows: [{
      id: "workflow-1",
      alias: "release",
      flush_agent_context_before_run: true,
      run_output_schema_ref: "schema-1",
      nodes: [{
        id: "node-1",
        agent_id: "agent-1",
        public_label: "review",
        instructions: "Review the release.",
        can_complete_workflow_run: true,
        can_emit_intermediate_run_output: true,
        wait_for_all_inputs: true,
        intermediate_output_schema_ref: "schema-2",
        max_turns: 3,
      }],
      edges: [{
        id: "edge-1",
        from_node_id: "node-1",
        to_node_id: "node-1",
        source_side: "right",
        target_side: "left",
        handoff_schema_ref: "schema-3",
        validation_policy: "halt",
      }],
      endpoints: [{ id: "endpoint-1", alias: "entry", entry_node_id: "node-1" }],
    }],
    workflow_runs: [],
    workflow_prompt_queues: [{
      id: "queue-1",
      workflow_id: "workflow-1",
      alias: "urgent",
      priority: 10,
      enabled: true,
      created_at_ms: 1,
      updated_at_ms: 1,
    }],
    workflow_queued_prompts: [{
      id: "queued-1",
      queue_id: "queue-1",
      workflow_id: "workflow-1",
      endpoint_id: "endpoint-1",
      prompt: "Ship it",
      source: "manual",
      status: "queued",
      created_at_ms: 1,
      updated_at_ms: 1,
    }],
  } as unknown as RuntimeSession

  const snapshot = buildCliAutomationSnapshot({
    attachmentId: () => "attachment-1",
    workspaceScreenMode: () => "workflow",
    workflowScreenActive: () => true,
    daemonDisconnected: () => false,
    statusLine: () => "ready",
    sessionState: () => session,
    focusedAgentId: () => "agent-1",
    agentActivityLabels: () => ({ "agent-1": "reviewing" }),
    streamingAgentId: () => null,
    agentBusyLatch: () => false,
    isAttached: () => true,
    waitingRoomState: () => createWaitingRoomState([], catalog, "opencode", "default", "", "opencode", DEFAULT_THEME_REGISTRY),
    availableSessions: () => [],
    providerCatalogState: () => catalog,
    waitingRoomCloudNotice: () => null,
    waitingRoomInventoryStatus: () => "ready",
    relayStatusState: () => null,
    remoteMachinesState: () => [],
    remoteKernelsState: () => [],
    terminalsState: () => [],
    externalProviderSessionsState: () => [],
    externalProviderSessionsPageState: () => ({ hasMore: false, nextCursor: null }),
    slicesState: () => [],
    providerAccountsState: () => [],
    waitingRoomTargets: () => ({ workspacePath: "/repo", worktreePath: "/repo" }),
    themeRegistryState: () => DEFAULT_THEME_REGISTRY,
    selectedWorkflowId: () => "workflow-1",
    selectedWorkflowNodeId: () => null,
    workspaceShellContext: () => ({ cwd: "/repo", env: {} }) as unknown as ShellContext,
    workspaceShellEntries: () => [],
    transcriptEntries: () => [],
    visibleTranscriptAgentId: () => "agent-1",
    agentPaneEntries: () => ({}),
    footerFlash: () => null,
    interactionChoiceSelection: () => 1,
    interactionCustomReply: () => "ship it",
    interactionCustomEditing: () => true,
  })

  assert.equal(snapshot.screen, "workflow")
  assert.equal(snapshot.attachmentId, "attachment-1")
  assert.equal((snapshot.session as { id: string }).id, "session-1")
  assert.deepEqual((snapshot.session as { agents: unknown[] }).agents[0], {
    id: "agent-1",
    agentRef: "A",
    alias: "frontend",
    provider: "opencode",
    model: "sonnet",
    effort: "high",
    accountProfile: "work",
    executionMode: "plan",
    permissionLevel: "required",
    primaryProvider: "opencode",
    primaryModel: "sonnet",
    primaryEffort: "medium",
    worktreeId: "/worker/repo",
    remoteExecution: {
      workerMachineId: "machine-1",
      workerKernelId: "kernel-1",
      executionLeaseId: "lease-1",
      leasedAgentId: "leased-agent-1",
      activeWorkerProviderRunId: "provider-run-1",
    },
    visibleInFreeform: false,
    state: "Idle",
    isProcessing: false,
    badge: { label: "REVIEWING", tone: "working" },
  })
  assert.equal((snapshot.session as { agentActivityRevision: number }).agentActivityRevision, 7)
  assert.equal(snapshot.transcript?.visibleAgentId, "agent-1")
  assert.equal((snapshot.selectedWorkflow as { alias: string }).alias, "release")
  assert.deepEqual(snapshot.selectedWorkflow, {
    id: "workflow-1",
    alias: "release",
    flushAgentContextBeforeRun: true,
    runOutputSchemaRef: "schema-1",
    nodeCount: 1,
    edgeCount: 1,
    endpointCount: 1,
    nodes: [{
      id: "node-1",
      agentId: "agent-1",
      publicLabel: "review",
      instructions: "Review the release.",
      canCompleteWorkflowRun: true,
      canEmitIntermediateRunOutput: true,
      waitForAllInputs: true,
      intermediateOutputSchemaRef: "schema-2",
      maxTurns: 3,
    }],
    edges: [{
      id: "edge-1",
      fromNodeId: "node-1",
      toNodeId: "node-1",
      sourceSide: "right",
      targetSide: "left",
      handoffSchemaRef: "schema-3",
      validationPolicy: "halt",
    }],
    endpoints: [{ id: "endpoint-1", alias: "entry", entryNodeId: "node-1" }],
    promptQueues: [{ id: "queue-1", alias: "urgent", priority: 10, enabled: true }],
    queuedPrompts: [{
      id: "queued-1",
      queueId: "queue-1",
      endpointId: "endpoint-1",
      prompt: "Ship it",
      source: "manual",
      status: "queued",
      workflowRunId: null,
    }],
  })
  assert.deepEqual((snapshot.interactions as Array<Record<string, unknown>>)[0], {
    id: "interaction-1",
    agentId: "agent-1",
    kind: "choice",
    level: "info",
    title: "Approve",
    message: "Continue?",
    timeoutSec: null,
    defaultOnTimeout: null,
    focused: true,
    selectedChoiceIndex: 1,
    customChoice: null,
    customReply: "ship it",
    customEditing: true,
    choices: [{ id: "yes", label: "Yes", style: "primary" }],
  })
})

test("buildCliAutomationSnapshot exposes external transcript and queued prompt metadata", () => {
  const catalog = fallbackProviderCatalog()
  const agent = {
    id: "agent-1",
    agent_ref: "A",
    alias: "worker",
    provider: "opencode",
    model: "default",
    state: "Idle",
    is_processing: false,
  } as AgentInstance
  const session = {
    id: "session-1",
    workspace_id: "/repo",
    worktree_id: "/repo",
    focused_agent_id: "agent-1",
    agents: [agent],
    active_interactions: [],
    workflows: [],
    workflow_runs: [],
  } as unknown as RuntimeSession

  const snapshot = buildCliAutomationSnapshot({
    workspaceScreenMode: () => "agents",
    workflowScreenActive: () => false,
    daemonDisconnected: () => false,
    statusLine: () => "ready",
    sessionState: () => session,
    focusedAgentId: () => "agent-1",
    agentActivityLabels: () => ({}),
    streamingAgentId: () => null,
    agentBusyLatch: () => false,
    isAttached: () => true,
    waitingRoomState: () => createWaitingRoomState([], catalog, "opencode", "default", "", "opencode", DEFAULT_THEME_REGISTRY),
    availableSessions: () => [],
    providerCatalogState: () => catalog,
    waitingRoomCloudNotice: () => null,
    waitingRoomInventoryStatus: () => "ready",
    relayStatusState: () => null,
    remoteMachinesState: () => [],
    remoteKernelsState: () => [],
    terminalsState: () => [],
    externalProviderSessionsState: () => [],
    externalProviderSessionsPageState: () => ({ hasMore: false, nextCursor: null }),
    slicesState: () => [],
    providerAccountsState: () => [],
    waitingRoomTargets: () => ({ workspacePath: "/repo", worktreePath: "/repo" }),
    themeRegistryState: () => DEFAULT_THEME_REGISTRY,
    selectedWorkflowId: () => null,
    selectedWorkflowNodeId: () => null,
    workspaceShellContext: () => ({ cwd: "/repo", env: {} }) as unknown as ShellContext,
    workspaceShellEntries: () => [],
    visibleTranscriptAgentId: () => "agent-1",
    transcriptEntries: () => [{
      id: 1,
      role: "assistant",
      text: "external output",
      promptId: "prompt-external",
      sourceAttachmentId: "attachment-1",
      attachments: [{
        url: "chariox-terminal://prompt-attachment/attachment-1/Screenshot.png",
        mime: "image/png",
        filename: "Screenshot.png",
        preview_url: "data:image/png;base64,aW1hZ2U=",
      }],
      source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
      externalProvider: "opencode",
      externalProviderSessionId: "thread-1",
      externalProviderTurnId: "turn-1",
      observedAtMs: 123,
      externalObservation: {
        settles_active_prompt: true,
        passive_telemetry: false,
      },
    }, {
      id: 3,
      role: "assistant",
      text: "ordinary output",
      source: "provider_output",
      externalProvider: "opencode",
      externalProviderSessionId: "thread-stale",
      externalProviderTurnId: "turn-stale",
      observedAtMs: 456,
      externalObservation: {
        settles_active_prompt: true,
        passive_telemetry: false,
      },
    }],
    agentPaneEntries: () => ({
      "agent-1": [{
        id: 2,
        role: "notice",
        text: "queued behind external turn",
        queuedPrompt: {
          promptId: "prompt-1",
          agentId: "agent-1",
          promptOrigin: "chariox",
          status: "queued",
          attachmentCount: 0,
          steerDisabled: true,
          canSteer: false,
          canCancel: true,
          steerDisabledReason: "Steering is unavailable while the active provider turn was started outside Chariox.",
          cancelDisabledReason: null,
        },
      }],
    }),
    queuedPromptStripItemsForAgent: (agentId) => agentId === "agent-1"
      ? [{
        promptId: "prompt-1",
        agentId: "agent-1",
        sourceAttachmentId: null,
        prompt: "queued behind external turn",
        promptOrigin: "chariox",
        status: "queued",
        attachmentCount: 0,
        steerDisabled: true,
        canSteer: false,
        canCancel: true,
        steerDisabledReason: "Steering is unavailable while the active provider turn was started outside Chariox.",
        cancelDisabledReason: null,
      }]
      : [],
    selectedQueuedPromptIndexForAgent: () => 0,
    footerFlash: () => null,
    interactionChoiceSelection: () => 0,
    interactionCustomReply: () => "",
    interactionCustomEditing: () => false,
  })

  assert.deepEqual((snapshot.transcript?.entries as Array<Record<string, unknown>>)[0], {
    id: 1,
    role: "assistant",
    text: "external output",
    promptId: "prompt-external",
    sourceAttachmentId: "attachment-1",
    attachments: [{
      url: "chariox-terminal://prompt-attachment/attachment-1/Screenshot.png",
      mime: "image/png",
      filename: "Screenshot.png",
      preview_url: "data:image/png;base64,aW1hZ2U=",
    }],
    queuedPrompt: null,
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "opencode",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
    observedAtMs: 123,
    externalObservation: {
      settles_active_prompt: true,
      passive_telemetry: false,
    },
    turnId: null,
    hidden: false,
    blobCollapsible: false,
    blobCollapsed: null,
    blobTitle: null,
    blobSummary: null,
    historyBlobId: null,
    historyBlobAgentId: null,
    historyBlobSourceId: null,
    historyBlobSourceAgentId: null,
    historyBlobLoaded: null,
    historyBlobLoading: null,
    historyBlobError: null,
  })
  assert.deepEqual((snapshot.agentPanes?.["agent-1"] as Array<Record<string, unknown>>)[0]?.queuedPrompt, {
    promptId: "prompt-1",
    agentId: "agent-1",
    promptOrigin: "chariox",
    status: "queued",
    attachmentCount: 0,
    steerDisabled: true,
    canSteer: false,
    canCancel: true,
    steerDisabledReason: "Steering is unavailable while the active provider turn was started outside Chariox.",
    cancelDisabledReason: null,
  })
  assert.deepEqual(snapshot.queuedPromptStrips?.["agent-1"], {
    selectedIndex: 0,
    items: [{
      promptId: "prompt-1",
      agentId: "agent-1",
      sourceAttachmentId: null,
      prompt: "queued behind external turn",
      promptOrigin: "chariox",
      status: "queued",
      attachmentCount: 0,
      steerDisabled: true,
      canSteer: false,
      canCancel: true,
      steerDisabledReason: "Steering is unavailable while the active provider turn was started outside Chariox.",
      cancelDisabledReason: null,
    }],
  })
  assert.deepEqual(pickExternalFields((snapshot.transcript?.entries as Array<Record<string, unknown>>)[1]), {
    source: "provider_output",
    externalProvider: null,
    externalProviderSessionId: null,
    externalProviderTurnId: null,
    observedAtMs: null,
    externalObservation: null,
  })
})

test("buildCliAutomationSnapshot trusts projected idle runtime state over stale agent state", () => {
  const catalog = fallbackProviderCatalog()
  const agent = {
    id: "agent-1",
    agent_ref: "A",
    alias: "worker",
    provider: "opencode",
    model: "default",
    state: "Working",
    is_processing: true,
  } as AgentInstance

  for (const projection of [
    {
      agent_activity: {
        "agent-1": {
          status: "idle",
          prompt_status: "none",
          busy: false,
          unread_idle_output: false,
        },
      },
    },
    {
      prompt_states: {
        "agent-1": {
          active_prompt: null,
          queued_prompts: [],
        },
      },
    },
  ] satisfies Array<Partial<RuntimeSession>>) {
    const session = {
      id: "session-1",
      workspace_id: "/repo",
      worktree_id: "/repo",
      focused_agent_id: "agent-1",
      agents: [agent],
      ...projection,
      active_interactions: [],
      workflows: [],
      workflow_runs: [],
    } as unknown as RuntimeSession

    const snapshot = buildCliAutomationSnapshot({
      workspaceScreenMode: () => "agents",
      workflowScreenActive: () => false,
      daemonDisconnected: () => false,
      statusLine: () => "ready",
      sessionState: () => session,
      focusedAgentId: () => "agent-1",
      agentActivityLabels: () => ({}),
      streamingAgentId: () => null,
      agentBusyLatch: () => false,
      isAttached: () => true,
      waitingRoomState: () => createWaitingRoomState([], catalog, "opencode", "default", "", "opencode", DEFAULT_THEME_REGISTRY),
      availableSessions: () => [],
      providerCatalogState: () => catalog,
      waitingRoomCloudNotice: () => null,
      waitingRoomInventoryStatus: () => "ready",
      relayStatusState: () => null,
      remoteMachinesState: () => [],
      remoteKernelsState: () => [],
      terminalsState: () => [],
      externalProviderSessionsState: () => [],
      externalProviderSessionsPageState: () => ({ hasMore: false, nextCursor: null }),
      slicesState: () => [],
      providerAccountsState: () => [],
      waitingRoomTargets: () => ({ workspacePath: "/repo", worktreePath: "/repo" }),
      themeRegistryState: () => DEFAULT_THEME_REGISTRY,
      selectedWorkflowId: () => null,
      selectedWorkflowNodeId: () => null,
      workspaceShellContext: () => ({ cwd: "/repo", env: {} }) as unknown as ShellContext,
      workspaceShellEntries: () => [],
      visibleTranscriptAgentId: () => "agent-1",
      transcriptEntries: () => [],
      agentPaneEntries: () => ({}),
      footerFlash: () => null,
      interactionChoiceSelection: () => 0,
      interactionCustomReply: () => "",
      interactionCustomEditing: () => false,
    })

    const snapshotAgent = (snapshot.session as {
      agents: Array<{ state: unknown; isProcessing: unknown; badge: unknown }>
    }).agents[0]
    assert.equal(snapshotAgent?.state, "Idle")
    assert.equal(snapshotAgent?.isProcessing, false)
    assert.deepEqual(snapshotAgent?.badge, {
      label: "IDLE",
      tone: "idle",
    })
  }
})

test("buildCliAutomationSnapshot exposes waiting room unattached agent rows", () => {
  const catalog = fallbackProviderCatalog()
  const snapshot = buildCliAutomationSnapshot({
    workspaceScreenMode: () => "agents",
    workflowScreenActive: () => false,
    daemonDisconnected: () => false,
    statusLine: () => "ready",
    sessionState: () => ({
      id: "detached",
      workspace_id: "/repo",
      worktree_id: "/repo",
      focused_agent_id: null,
      agents: [],
      active_interactions: [],
      workflows: [],
      workflow_runs: [],
    }) as unknown as RuntimeSession,
    focusedAgentId: () => null,
    agentActivityLabels: () => ({}),
    streamingAgentId: () => null,
    agentBusyLatch: () => false,
    isAttached: () => false,
    waitingRoomState: () => createWaitingRoomState([], catalog, "opencode", "default", "", "opencode", DEFAULT_THEME_REGISTRY),
    availableSessions: () => [],
    waitingRoomProjects: () => [{
      id: "project-1",
      owner_user_id: "owner",
      workspace_id: "/repo",
      name: "Frontend",
      kind: "named",
      status: "active",
      created_at_ms: 1,
      updated_at_ms: 2,
      session_count: 0,
      joined_collaborator_count: 1,
      pending_collaboration_invite_count: 2,
    }],
    providerCatalogState: () => catalog,
    waitingRoomCloudNotice: () => null,
    waitingRoomInventoryStatus: () => "ready",
    relayStatusState: () => null,
    remoteMachinesState: () => [],
    remoteKernelsState: () => [],
    terminalsState: () => [],
    externalProviderSessionsState: () => [{
      external_session_id: "opencode:thread-1",
      provider: "opencode",
      provider_session_id: "thread-1",
      title: "External OpenCode thread",
      first_prompt_preview: "external prompt",
      last_modified_at_ms: 1_700_000_000_000,
    }],
    externalProviderSessionsPageState: () => ({ hasMore: false, nextCursor: null }),
    slicesState: () => [],
    providerAccountsState: () => [],
    waitingRoomTargets: () => ({ workspacePath: "/repo", worktreePath: "/repo" }),
    themeRegistryState: () => DEFAULT_THEME_REGISTRY,
    selectedWorkflowId: () => null,
    selectedWorkflowNodeId: () => null,
    workspaceShellContext: () => ({ cwd: "/repo", env: {} }) as unknown as ShellContext,
    workspaceShellEntries: () => [],
    transcriptEntries: () => [],
    visibleTranscriptAgentId: () => null,
    agentPaneEntries: () => ({}),
    footerFlash: () => null,
    interactionChoiceSelection: () => 0,
    interactionCustomReply: () => "",
    interactionCustomEditing: () => false,
  })

  const rows = (snapshot.waitingRoom as { rows: Array<Record<string, unknown>> }).rows
  const projects = (snapshot.waitingRoom as { projects: Array<Record<string, unknown>> }).projects
  assert.deepEqual(projects, [{
    id: "project-1",
    name: "Frontend",
    kind: "named",
    status: "active",
    workspaceId: "/repo",
    sessionCount: 0,
    joinedCollaboratorCount: 1,
    pendingCollaborationInviteCount: 2,
    lastSessionActivityAtMs: null,
  }])
  assert.equal(rows.some((row) => row.id === "project-entry:project-1"), true)
  assert.deepEqual(rows.find((row) => row.id === "external-session:opencode:thread-1"), {
    id: "external-session:opencode:thread-1",
    externalSessionId: "opencode:thread-1",
    title: "External OpenCode thread",
    value: "opencode",
    focused: false,
    selectable: true,
  })
})

test("buildCliAutomationSnapshot shows the selected account and provider profiles after connect", () => {
  const catalog = fallbackProviderCatalog()
  const snapshot = buildCliAutomationSnapshot({
    workspaceScreenMode: () => "agents",
    workflowScreenActive: () => false,
    daemonDisconnected: () => false,
    statusLine: () => "ready",
    sessionState: () => ({
      id: "session-1",
      workspace_id: "/repo",
      worktree_id: "/repo",
      focused_agent_id: null,
      agents: [],
      active_interactions: [],
      workflows: [],
      workflow_runs: [],
    }) as unknown as RuntimeSession,
    focusedAgentId: () => null,
    agentActivityLabels: () => ({}),
    streamingAgentId: () => null,
    agentBusyLatch: () => false,
    isAttached: () => false,
    waitingRoomState: () => createWaitingRoomState([], catalog, "opencode", "default", "", "opencode", DEFAULT_THEME_REGISTRY),
    availableSessions: () => [],
    providerCatalogState: () => catalog,
    waitingRoomCloudNotice: () => null,
    waitingRoomInventoryStatus: () => "ready",
    relayStatusState: () => ({
      configured: true,
      connected: true,
      relay_url: "wss://relay",
      relay_token_configured: true,
      daemon_id: "daemon-1",
      machine_id: "machine-1",
    }) as RelayStatusView,
    remoteMachinesState: () => [],
    remoteKernelsState: () => [],
    terminalsState: () => [],
    externalProviderSessionsState: () => [],
    externalProviderSessionsPageState: () => ({ hasMore: false, nextCursor: null }),
    slicesState: () => [],
    providerAccountsState: () => [1, 2, 3, 4].map((index) => ({
      owner_user_id: "local",
      profile_id: `profile-${index}`,
      provider: "opencode",
      label: `opencode-${index}`,
      origin: "linked",
      is_default: index === 1,
      auth_state: "authenticated",
      usage: {
        profile_id: `profile-${index}`,
        provider: "opencode",
        availability: "available",
        source: "test",
      },
    }) as ProviderAccountProfile),
    waitingRoomTargets: () => ({ workspacePath: "/repo", worktreePath: "/repo" }),
    themeRegistryState: () => DEFAULT_THEME_REGISTRY,
    selectedWorkflowId: () => null,
    selectedWorkflowNodeId: () => null,
    workspaceShellContext: () => ({ cwd: "/repo", env: {} }) as unknown as ShellContext,
    workspaceShellEntries: () => [],
    transcriptEntries: () => [],
    visibleTranscriptAgentId: () => null,
    agentPaneEntries: () => ({}),
    footerFlash: () => null,
    interactionChoiceSelection: () => 0,
    interactionCustomReply: () => "",
    interactionCustomEditing: () => false,
  })

  const rows = (snapshot.waitingRoom as { rows: Array<Record<string, unknown>> }).rows
  assert.equal(rows.find((row) => row.id === "account")?.value, "opencode-1 · OpenCode Zen")
  assert.equal(rows.find((row) => row.id === "provider-accounts")?.value, "4 profiles · Press Enter")
})

function pickExternalFields(entry: Record<string, unknown> | undefined): Record<string, unknown> {
  return {
    source: entry?.source,
    externalProvider: entry?.externalProvider,
    externalProviderSessionId: entry?.externalProviderSessionId,
    externalProviderTurnId: entry?.externalProviderTurnId,
    observedAtMs: entry?.observedAtMs,
    externalObservation: entry?.externalObservation,
  }
}
