import assert from "node:assert/strict"
import test from "node:test"

import type { PromptAttachmentPart, RuntimeAttachment, RuntimeSession } from "./cli-types.js"
import {
  createNormalPromptSubmitController,
  type NormalPromptSubmitControllerDeps,
} from "./normal-prompt-submit-controller.js"
import type { PendingPromptAttachment } from "./prompt-attachment-state.js"
import type { PromptSubmissionResult } from "./prompt-runtime-api.js"
import type { SubmittedPromptUiSnapshot } from "./prompt-submission-ui-controller.js"

test("normal prompt submit requires an attachment", async () => {
  const harness = createHarness({ attachment: null })

  await harness.controller.submit("hello")

  assert.equal(harness.footerMessages().at(-1)?.message, "No session attached.")
  assert.equal(harness.clearPromptCount(), 1)
  assert.deepEqual(harness.submissions(), [])
})

test("normal prompt submit prepares attachments, submits, and records history", async () => {
  const harness = createHarness({
    pendingAttachments: [pendingAttachment("file-1")],
    inlineLocalFiles: true,
    submitPrompt: async () => promptSubmissionResult("session-submitted", "agent-submitted", "PromptSubmitted"),
  })

  await harness.controller.submit("hello")

  assert.deepEqual(harness.preparedAttachments(), [{
    attachments: [{ url: "/tmp/file.txt", mime: "text/plain", filename: "file.txt" }],
    inlineLocalFiles: true,
  }])
  assert.deepEqual(harness.submissions(), [{
    attachmentId: "attachment-1",
    targetAgentId: "agent-1",
    prompt: "hello\n",
    attachments: [{ url: "/tmp/file.txt", mime: "text/plain", filename: "file.txt", contents_base64: "ZmlsZQ==" }],
  }])
  assert.deepEqual(harness.appendedPrompts(), [{ text: "hello\n", agentId: "agent-submitted" }])
  assert.equal(harness.appliedSessions().at(-1)?.id, "session-submitted")
  assert.deepEqual(harness.streamingAgentIds(), ["agent-submitted"])
  assert.deepEqual(harness.statusLines(), ["Prompt submitted."])
  assert.deepEqual(harness.recordedHistory(), [{ sessionId: "session-1", rawPrompt: "hello" }])
})

test("normal prompt submit lets the kernel route an alias while rendering only the prompt body", async () => {
  const harness = createHarness({
    submitPrompt: async () => ({
      ...promptSubmissionResult("session-submitted", "agent-reviewer", "PromptSubmitted"),
      payload: {
        outcome: {},
        session: runtimeSession("session-submitted", null, {
          focused_agent_id: "agent-reviewer",
          agents: [agent("agent-1"), agent("agent-reviewer")],
        }),
        agent_activity: {},
        agent_activity_revision: 1,
      },
    }),
  })

  await harness.controller.submit("@reviewer inspect package.json")

  assert.equal(harness.submissions().at(-1)?.prompt, "@reviewer inspect package.json\n")
  assert.deepEqual(harness.appendedPrompts(), [{
    text: "inspect package.json\n",
    agentId: "agent-reviewer",
  }])
  assert.equal(harness.appliedSessions().at(-1)?.focused_agent_id, "agent-reviewer")
})

test("normal prompt submit appends prompt acknowledgement metadata", async () => {
  const harness = createHarness({
    submitPrompt: async () => ({
      payload: {
        outcome: {
          Started: {
            prompt: {
              id: "prompt-started",
              source_attachment_id: "attachment-started",
              target_agent_id: "agent-1",
              prompt: "hello\n",
              status: "running",
              prompt_origin: " External ",
            },
          },
        },
        session: runtimeSession("session-submitted", "prompt-started"),
        agent_activity: {},
        agent_activity_revision: 1,
      },
      targetAgentId: "agent-1",
      outcomeName: "PromptSubmitted",
    }),
  })

  await harness.controller.submit("hello")

  assert.deepEqual(harness.appendedPrompts(), [{
    text: "hello\n",
    agentId: "agent-1",
    promptId: "prompt-started",
    sourceAttachmentId: "attachment-started",
    promptOrigin: "external",
  }])
})

test("normal prompt submit drops stale focused agent ids", async () => {
  const harness = createHarness({
    focusedAgentId: "old-agent",
    hasAgent: (agentId) => agentId === "agent-1",
  })

  await harness.controller.submit("hello")

  assert.deepEqual(harness.submissions(), [{
    attachmentId: "attachment-1",
    targetAgentId: null,
    prompt: "hello\n",
    attachments: [],
  }])
  assert.deepEqual(harness.appendedPrompts(), [{ text: "hello\n", agentId: null }])
})

test("normal prompt submit reports queued status with active prompt id", async () => {
  const harness = createHarness({
    submitPrompt: async () => ({
      ...promptSubmissionResult("session-submitted", null, "Queued", "prompt-active"),
      payload: {
        outcome: {},
        session: runtimeSession("session-submitted", "prompt-active", {
          agents: [agent("agent-1")],
        }),
        agent_activity: {},
        agent_activity_revision: 1,
      },
    }),
  })

  await harness.controller.submit("hello\n")

  assert.deepEqual(harness.statusLines(), ["Prompt queued behind prompt-active."])
  assert.equal(harness.submissions().at(-1)?.prompt, "hello\n")
  assert.deepEqual(harness.appendedPrompts(), [])
})

test("normal prompt submit projects queued runtime state from active session work", async () => {
  const harness = createHarness({
    focusedAgentId: "agent-queued",
    hasAgent: (agentId) => agentId === "agent-active" || agentId === "agent-queued",
    submitPrompt: async () => ({
      ...promptSubmissionResult("session-submitted", "agent-queued", "Queued"),
      payload: {
        outcome: {},
        session: runtimeSession("session-submitted", null, {
          agents: [agent("agent-active"), agent("agent-queued")],
          prompt_states: {
            "agent-active": {
              active_prompt: {
                id: "prompt-active",
                source_attachment_id: "attachment-1",
                target_agent_id: "agent-active",
                prompt: "running",
                status: "running",
              },
              queued_prompts: [],
            },
            "agent-queued": {
              active_prompt: null,
              queued_prompts: [{
                id: "prompt-queued",
                source_attachment_id: "attachment-1",
                target_agent_id: "agent-queued",
                prompt: "hello",
                status: "queued",
              }],
            },
          },
        }),
        agent_activity: {},
        agent_activity_revision: 1,
      },
    }),
  })

  await harness.controller.submit("hello")

  assert.deepEqual(harness.appendedPrompts(), [])
  assert.deepEqual(harness.streamingAgentIds(), ["agent-active"])
  assert.equal(harness.workingValues().at(-1), true)
})

test("normal prompt submit reports queued status from per-agent active prompt state", async () => {
  const harness = createHarness({
    submitPrompt: async () => ({
      ...promptSubmissionResult("session-submitted", "agent-1", "Queued"),
      payload: {
        outcome: {},
        session: runtimeSession("session-submitted", null, {
          agents: [agent("agent-1")],
          prompt_states: {
            "agent-1": {
              active_prompt: {
                id: "prompt-active-agent",
                source_attachment_id: "attachment-1",
                target_agent_id: "agent-1",
                prompt: "running",
                status: "running",
              },
              queued_prompts: [{
                id: "prompt-queued-agent",
                source_attachment_id: "attachment-1",
                target_agent_id: "agent-1",
                prompt: "hello",
                status: "queued",
              }],
            },
          },
        }),
        agent_activity: {},
        agent_activity_revision: 1,
      },
    }),
  })

  await harness.controller.submit("hello")

  assert.deepEqual(harness.statusLines(), ["Prompt queued behind prompt-active-agent."])
  assert.deepEqual(harness.appendedPrompts(), [])
})

test("normal prompt submit logs projected queued prompt counts", async () => {
  const harness = createHarness({
    submitPrompt: async () => ({
      ...promptSubmissionResult("session-submitted", "agent-1", "Queued"),
      payload: {
        outcome: {},
        session: runtimeSession("session-submitted", null, {
          agents: [agent("agent-1")],
          prompt_states: {
            "agent-1": {
              active_prompt: null,
              queued_prompts: [{
                id: "stale-queued",
                source_attachment_id: "attachment-1",
                target_agent_id: "agent-1",
                prompt: "stale",
                status: "queued",
              }],
            },
          },
          agent_activity: {
            "agent-1": {
              status: "working",
              prompt_status: "queued",
              busy: true,
              queued_prompt_count: 2,
              unread_idle_output: false,
            },
          },
        }),
        agent_activity: {},
        agent_activity_revision: 1,
      },
    }),
  })

  await harness.controller.submit("hello")

  assert.equal(harness.logInfos().find((entry) => entry.message === "prompt submitted")?.fields.queued_prompts, 2)
})

test("normal prompt submit restores UI after submit failure", async () => {
  const harness = createHarness({
    submitPrompt: async () => {
      throw new Error("submit failed")
    },
  })

  await harness.controller.submit("hello")

  assert.equal(harness.logErrors().at(-1)?.message, "prompt submission failed")
  assert.equal(harness.restoredSnapshots().at(-1)?.rawPrompt, "hello")
  assert.deepEqual(harness.clearedBusyAgents(), ["agent-busy"])
  assert.deepEqual(harness.submittingAgentIds(), [null])
  assert.deepEqual(harness.submittingValues(), [false])
  assert.equal(harness.workingValues().at(-1), false)
  assert.equal(harness.fatalErrors().at(-1), "submit failed")
  assert.deepEqual(harness.footerMessages().at(-1), {
    message: "submit failed",
    tone: "error",
  })
})

test("normal prompt submit failure preserves active session runtime state", async () => {
  const harness = createHarness({
    session: runtimeSession("session-1", null, {
      agents: [agent("agent-active")],
      prompt_states: {
        "agent-active": {
          active_prompt: {
            id: "prompt-active",
            source_attachment_id: "attachment-1",
            target_agent_id: "agent-active",
            prompt: "running",
            status: "running",
          },
          queued_prompts: [],
        },
      },
    }),
    submitPrompt: async () => {
      throw new Error("submit failed")
    },
  })

  await harness.controller.submit("hello")

  assert.deepEqual(harness.streamingAgentIds(), ["agent-active"])
  assert.equal(harness.workingValues().at(-1), true)
})

function createHarness(options: {
  attachment?: RuntimeAttachment | null
  pendingAttachments?: PendingPromptAttachment[]
  inlineLocalFiles?: boolean
  focusedAgentId?: string | null
  session?: RuntimeSession
  hasAgent?: (agentId: string) => boolean
  submitPrompt?: NormalPromptSubmitControllerDeps["submitPrompt"]
} = {}) {
  const preparedAttachments: Array<{ attachments: PromptAttachmentPart[]; inlineLocalFiles: boolean }> = []
  const submissions: Array<{
    attachmentId: string
    targetAgentId: string | null
    prompt: string
    attachments: PromptAttachmentPart[]
  }> = []
  const appendedPrompts: Array<{
    text: string
    agentId: string | null | undefined
    promptId?: string | null
    sourceAttachmentId?: string | null
    promptOrigin?: string | null
  }> = []
  const appliedSessions: RuntimeSession[] = []
  const streamingAgentIds: Array<string | null> = []
  const statusLines: string[] = []
  const recordedHistory: Array<{ sessionId: string; rawPrompt: string }> = []
  const restoredSnapshots: SubmittedPromptUiSnapshot[] = []
  const clearedBusyAgents: Array<string | null | undefined> = []
  const submittingAgentIds: Array<string | null> = []
  const submittingValues: boolean[] = []
  const workingValues: boolean[] = []
  const footerMessages: Array<{ message: string; tone: "info" | "error" }> = []
  const logErrors: Array<{ message: string; fields: Record<string, unknown> }> = []
  const logInfos: Array<{ message: string; fields: Record<string, unknown> }> = []
  const fatalErrors: string[] = []
  let clearPromptCount = 0

  const controller = createNormalPromptSubmitController({
    getPendingAttachments: () => options.pendingAttachments ?? [],
    waitForPendingAgentFocusTransition: async () => {},
    getFocusedAgentId: () => options.focusedAgentId ?? "agent-1",
    getSession: () => options.session ?? runtimeSession("session-1", null, { agents: [agent("agent-1")] }),
    hasAgent: options.hasAgent ?? ((agentId) => agentId === "agent-1"),
    clearActiveToolLabels: () => {},
    setProviderActivityLabel: () => {},
    setActiveStatusLabel: () => {},
    getAttachment: () => options.attachment === undefined ? { id: "attachment-1", session_id: "session-1" } : options.attachment,
    getSessionId: () => "session-1",
    clearPromptText: () => {
      clearPromptCount += 1
    },
    shouldInlineLocalFiles: () => options.inlineLocalFiles ?? false,
    preparePromptAttachmentsForSubmit: async (attachments, transferOptions) => {
      preparedAttachments.push({ attachments, inlineLocalFiles: transferOptions.inlineLocalFiles })
      return attachments.map((attachment) => ({ ...attachment, contents_base64: "ZmlsZQ==" }))
    },
    beginSubmittedPromptUi: (rawPrompt) => ({ rawPrompt, attachments: [], sessionId: "session-1" }),
    renderPromptTranscript: (prompt) => prompt,
    appendUserPrompt: (text, agentId, metadata) => {
      appendedPrompts.push({
        text,
        agentId,
        ...(metadata?.promptId !== undefined ? { promptId: metadata.promptId } : {}),
        ...(metadata?.sourceAttachmentId !== undefined ? { sourceAttachmentId: metadata.sourceAttachmentId } : {}),
        ...(metadata?.promptOrigin !== undefined ? { promptOrigin: metadata.promptOrigin } : {}),
      })
    },
    submitPrompt: async (attachmentId, targetAgentId, prompt, attachments) => {
      submissions.push({ attachmentId, targetAgentId, prompt, attachments })
      return options.submitPrompt
        ? options.submitPrompt(attachmentId, targetAgentId, prompt, attachments)
        : promptSubmissionResult("session-submitted", targetAgentId, "PromptSubmitted")
    },
    applySessionState: (session) => {
      appliedSessions.push(session)
    },
    setStreamingAgentId: (agentId) => {
      streamingAgentIds.push(agentId)
    },
    setWorking: (working) => {
      workingValues.push(working)
    },
    updateSessionChrome: () => {},
    setStatusLine: (line) => {
      statusLines.push(line)
    },
    recordPromptAreaHistoryEntry: (sessionId, rawPrompt) => {
      recordedHistory.push({ sessionId, rawPrompt })
    },
    restoreFailedPromptUi: (snapshot) => {
      if (snapshot) {
        restoredSnapshots.push(snapshot)
      }
      return Boolean(snapshot)
    },
    getSubmittingAgentId: () => "agent-busy",
    clearAgentBusy: (agentId) => {
      clearedBusyAgents.push(agentId)
    },
    setSubmittingAgentId: (agentId) => {
      submittingAgentIds.push(agentId)
    },
    setSubmitting: (submitting) => {
      submittingValues.push(submitting)
    },
    setFatalError: (message) => {
      fatalErrors.push(message)
    },
    flashFooter: (message, tone) => {
      footerMessages.push({ message, tone })
    },
    logError: (message, fields) => {
      logErrors.push({ message, fields })
    },
    logInfo: (message, fields) => {
      logInfos.push({ message, fields })
    },
    formatError: (error) => error instanceof Error ? error.message : String(error),
  })

  return {
    controller,
    preparedAttachments: () => preparedAttachments,
    submissions: () => submissions,
    appendedPrompts: () => appendedPrompts,
    appliedSessions: () => appliedSessions,
    streamingAgentIds: () => streamingAgentIds,
    statusLines: () => statusLines,
    recordedHistory: () => recordedHistory,
    restoredSnapshots: () => restoredSnapshots,
    clearedBusyAgents: () => clearedBusyAgents,
    submittingAgentIds: () => submittingAgentIds,
    submittingValues: () => submittingValues,
    workingValues: () => workingValues,
    footerMessages: () => footerMessages,
    logInfos: () => logInfos,
    logErrors: () => logErrors,
    fatalErrors: () => fatalErrors,
    clearPromptCount: () => clearPromptCount,
  }
}

function agent(id: string): RuntimeSession["agents"][number] {
  return {
    id,
    agent_ref: id,
    session_id: "session-submitted",
    alias: id,
    provider: "codex",
    model: null,
    worktree_id: "/workspace/tree",
    state: "Idle",
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 1,
    last_activity_at_ms: 1,
  }
}

function pendingAttachment(id: string): PendingPromptAttachment {
  return {
    id,
    url: "/tmp/file.txt",
    mime: "text/plain",
    filename: "file.txt",
    kind: "text",
    token: "[file 1]",
  }
}

function promptSubmissionResult(
  sessionId: string,
  targetAgentId: string | null,
  outcomeName: string,
  activePromptId: string | null = null,
): PromptSubmissionResult {
  return {
    payload: {
      outcome: {},
      session: runtimeSession(sessionId, activePromptId),
      agent_activity: {},
      agent_activity_revision: 1,
    },
    targetAgentId,
    outcomeName,
  }
}

function runtimeSession(
  id: string,
  activePromptId: string | null,
  overrides: Partial<RuntimeSession> = {},
): RuntimeSession {
  return {
    id,
    project_id: "project-default",
    workspace_id: "/workspace",
    worktree_id: "/workspace/tree",
    created_at_ms: 1,
    status: "Created",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    ...(activePromptId
      ? { prompt_states: {
        "agent-1": {
          active_prompt: {
            id: activePromptId,
            source_attachment_id: "attachment-1",
            target_agent_id: "agent-1",
            prompt: "hello",
            status: "running",
          },
          queued_prompts: [],
        },
      } }
      : {}),
    focused_agent_id: null,
    max_agents: 1,
    agents: [],
    config_state: {
      version: 1,
      values: {},
    },
    ...overrides,
  }
}
