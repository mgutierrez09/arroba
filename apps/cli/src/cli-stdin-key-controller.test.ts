import assert from "node:assert/strict"
import test from "node:test"

import {
  createCliStdinKeyController,
  type CliStdinKeyControllerDeps,
  type CliStdinKeyEvent,
} from "./cli-stdin-key-controller.js"

test("cli stdin key controller ignores unparsable input", () => {
  const harness = createHarness({ parsedEvent: null })

  assert.equal(harness.controller.handleData("x"), false)
  assert.deepEqual(harness.calls(), ["parse:x:true"])
})

test("cli stdin key controller closes dialog overlays on escape before other handlers", () => {
  const harness = createHarness({
    dialogOpen: true,
    sessionBrowserHandled: true,
    parsedEvent: keyEvent("escape"),
  })

  assert.equal(harness.controller.handleData("x"), true)
  assert.deepEqual(harness.calls(), ["parse:x:true", "close-dialog"])
})

test("cli stdin key controller delegates session browser keys early", () => {
  const harness = createHarness({
    sessionBrowserHandled: true,
    parsedEvent: keyEvent("down"),
  })

  assert.equal(harness.controller.handleData("x"), true)
  assert.deepEqual(harness.calls(), ["parse:x:true", "session-browser:down"])
})

test("cli stdin key controller routes exit before prompt state", () => {
  const exitHarness = createHarness({ parsedEvent: keyEvent("e", { ctrl: true }) })
  assert.equal(exitHarness.controller.handleData("x"), true)
  assert.deepEqual(exitHarness.calls(), ["parse:x:true", "session-browser:e", "exit"])
})

test("cli stdin key controller yields focused interactions to the focused prompt input", () => {
  const promptHarness = createHarness({
    focusedInteractionActive: true,
    focusedInteractionHandled: true,
    promptFocused: true,
    parsedEvent: keyEvent("p"),
  })
  assert.equal(promptHarness.controller.handleData("x"), true)
  assert.deepEqual(promptHarness.calls(), [
    "parse:x:true",
    "session-browser:p",
  ])

  const focusedHarness = createHarness({
    focusedInteractionHandled: true,
    parsedEvent: keyEvent("return"),
  })
  assert.equal(focusedHarness.controller.handleData("x"), true)
  assert.deepEqual(focusedHarness.calls(), ["parse:x:true", "session-browser:return", "focused:return"])
})

test("cli stdin key controller delegates queued prompt shortcuts before command center prompt input", () => {
  const queuedEvents: CliStdinKeyEvent[] = []
  const harness = createHarness({
    queuedPromptHandled: true,
    promptFocused: true,
    commandCenterOpen: true,
    parsedEvent: keyEvent("s", { meta: true }),
    onQueuedPromptKey: (event) => {
      queuedEvents.push(event)
      return true
    },
  })

  assert.equal(harness.controller.handleData("x"), true)
  const queuedEvent = queuedEvents[0]
  assert.ok(queuedEvent)
  assert.equal(queuedEvent?.alt, true)
  assert.equal(queuedEvent?.meta, false)
  assert.deepEqual(harness.calls(), [
    "parse:x:true",
    "session-browser:s",
    "focused:s",
    "queued-prompt:s",
  ])
})

test("cli stdin key controller lets command center own prompt input", () => {
  const harness = createHarness({
    promptFocused: true,
    commandCenterOpen: true,
    parsedEvent: keyEvent("escape"),
  })

  assert.equal(harness.controller.handleData("x"), true)
  assert.deepEqual(harness.calls(), [
    "parse:x:true",
    "session-browser:escape",
    "focused:escape",
    "clear-command-center",
  ])
})

test("cli stdin key controller routes workspace and focus cycling shortcuts", () => {
  const workspaceHarness = createHarness({ parsedEvent: keyEvent("p", { ctrl: true }) })
  assert.equal(workspaceHarness.controller.handleData("x"), true)
  assert.deepEqual(workspaceHarness.calls(), [
    "parse:x:true",
    "session-browser:p",
    "focused:p",
    "toggle-workspace",
  ])

  const focusHarness = createHarness({ attached: true, parsedEvent: keyEvent("tab") })
  assert.equal(focusHarness.controller.handleData("x"), true)
  assert.deepEqual(focusHarness.calls(), [
    "parse:x:true",
    "session-browser:tab",
    "focused:tab",
    "cycle-agent-focus",
  ])

  const workflowHarness = createHarness({
    attached: true,
    workflowScreenActive: true,
    parsedEvent: keyEvent("tab"),
  })
  assert.equal(workflowHarness.controller.handleData("x"), true)
  assert.deepEqual(workflowHarness.calls(), [
    "parse:x:true",
    "session-browser:tab",
    "focused:tab",
    "cycle-workflow-node",
  ])
})

test("cli stdin key controller routes copy and ctrl-c shortcuts", () => {
  const copyHarness = createHarness({
    copyHandled: true,
    parsedEvent: keyEvent("c", { meta: true }),
  })
  assert.equal(copyHarness.controller.handleData("x"), true)
  assert.deepEqual(copyHarness.calls(), [
    "parse:x:true",
    "session-browser:c",
    "focused:c",
    "queued-prompt:c",
    "copy",
  ])

  const stopHarness = createHarness({
    activeTurnWork: true,
    parsedEvent: keyEvent("c", { ctrl: true }),
  })
  assert.equal(stopHarness.controller.handleData("x"), true)
  assert.deepEqual(stopHarness.calls(), [
    "parse:x:true",
    "session-browser:c",
    "focused:c",
    "stop",
  ])

  const exitHarness = createHarness({ parsedEvent: keyEvent("c", { ctrl: true }) })
  assert.equal(exitHarness.controller.handleData("x"), true)
  assert.deepEqual(exitHarness.calls(), [
    "parse:x:true",
    "session-browser:c",
    "focused:c",
    "exit",
  ])
})

test("cli stdin key controller routes prompt attachment edits", () => {
  const editHarness = createHarness({
    promptFocused: true,
    removeEditHandled: true,
    parsedEvent: keyEvent("backspace"),
  })
  assert.equal(editHarness.controller.handleData("x"), true)
  assert.deepEqual(editHarness.calls(), [
    "parse:x:true",
    "session-browser:backspace",
    "focused:backspace",
    "remove-edit:backspace",
  ])

  const trailingHarness = createHarness({
    attached: true,
    currentPromptText: "",
    pendingAttachmentCount: 1,
    parsedEvent: keyEvent("backspace"),
  })
  assert.equal(trailingHarness.controller.handleData("x"), true)
  assert.deepEqual(trailingHarness.calls(), [
    "parse:x:true",
    "session-browser:backspace",
    "focused:backspace",
    "remove-last-attachment",
  ])
})

test("cli stdin key controller routes workflow detail pane keys before prompt navigation", () => {
  const harness = createHarness({
    workflowScreenActive: true,
    workflowDetailHandled: true,
    parsedEvent: keyEvent("l"),
  })

  assert.equal(harness.controller.handleData("x"), true)
  assert.deepEqual(harness.calls(), [
    "parse:x:true",
    "session-browser:l",
    "focused:l",
    "workflow-detail:l",
  ])
})

test("cli stdin key controller falls through to prompt-turn and waiting-room handlers", () => {
  const promptTurnHarness = createHarness({
    promptTurnHandled: true,
    parsedEvent: keyEvent("up", { shift: true }),
  })
  assert.equal(promptTurnHarness.controller.handleData("x"), true)
  assert.deepEqual(promptTurnHarness.calls(), [
    "parse:x:true",
    "session-browser:up",
    "focused:up",
    "prompt-turn:up",
  ])

  const waitingRoomHarness = createHarness({
    waitingRoomHandled: true,
    parsedEvent: keyEvent("down"),
  })
  assert.equal(waitingRoomHarness.controller.handleData("x"), true)
  assert.deepEqual(waitingRoomHarness.calls(), [
    "parse:x:true",
    "session-browser:down",
    "focused:down",
    "prompt-turn:down",
    "waiting-room:down",
  ])
})

function createHarness(options: {
  parsedEvent?: CliStdinKeyEvent | null
  dialogOpen?: boolean
  sessionBrowserHandled?: boolean
  focusedInteractionActive?: boolean
  focusedInteractionHandled?: boolean
  queuedPromptHandled?: boolean
  onQueuedPromptKey?: (event: CliStdinKeyEvent) => boolean
  promptFocused?: boolean
  commandCenterOpen?: boolean
  commandCenterQuery?: string
  attached?: boolean
  workflowScreenActive?: boolean
  copyHandled?: boolean
  activeTurnWork?: boolean
  removeEditHandled?: boolean
  currentPromptText?: string
  pendingAttachmentCount?: number
  promptTurnHandled?: boolean
  waitingRoomHandled?: boolean
  workflowDetailHandled?: boolean
} = {}) {
  const calls: string[] = []
  const parsedEvent = options.parsedEvent === undefined
    ? keyEvent("x")
    : options.parsedEvent
  const deps: CliStdinKeyControllerDeps = {
    parseKeypress: (chunk, parseOptions) => {
      calls.push(`parse:${String(chunk)}:${parseOptions.useKittyKeyboard}`)
      return parsedEvent
    },
    dialogOverlayOpen: () => options.dialogOpen ?? false,
    closeActiveDialogOverlay: () => {
      calls.push("close-dialog")
    },
    handleSessionBrowserKey: (event) => {
      calls.push(`session-browser:${event.name}`)
      return options.sessionBrowserHandled ?? false
    },
    requestExit: () => {
      calls.push("exit")
    },
    focusedInteractionActive: () => options.focusedInteractionActive ?? false,
    handleFocusedInteractionKey: (event) => {
      calls.push(`focused:${event.name}`)
      return options.focusedInteractionHandled ?? false
    },
    handleQueuedPromptKey: (event) => {
      calls.push(`queued-prompt:${event.name}`)
      if (options.onQueuedPromptKey) {
        return options.onQueuedPromptKey(event)
      }
      return options.queuedPromptHandled ?? false
    },
    promptFocused: () => options.promptFocused ?? false,
    commandCenterOpen: () => options.commandCenterOpen ?? false,
    commandCenterQuery: () => options.commandCenterQuery ?? "",
    clearCommandCenter: () => {
      calls.push("clear-command-center")
    },
    toggleWorkspaceScreen: () => {
      calls.push("toggle-workspace")
    },
    isAttached: () => options.attached ?? false,
    workflowScreenActive: () => options.workflowScreenActive ?? false,
    cycleWorkflowCanvasNode: () => {
      calls.push("cycle-workflow-node")
    },
    handleWorkflowDetailPaneKey: (event) => {
      calls.push(`workflow-detail:${event.name}`)
      return options.workflowDetailHandled ?? false
    },
    cycleAgentFocus: () => {
      calls.push("cycle-agent-focus")
    },
    copyPromptSelection: () => {
      calls.push("copy")
      return options.copyHandled ?? false
    },
    hasActiveTurnWork: () => options.activeTurnWork ?? false,
    requestPromptStop: () => {
      calls.push("stop")
    },
    removePromptAttachmentsForEdit: (edit) => {
      calls.push(`remove-edit:${edit}`)
      return options.removeEditHandled ?? false
    },
    currentPromptText: () => options.currentPromptText ?? "prompt",
    pendingAttachmentCount: () => options.pendingAttachmentCount ?? 0,
    removeLastPendingPromptAttachment: () => {
      calls.push("remove-last-attachment")
    },
    handlePromptTurnNavigationKey: (event) => {
      calls.push(`prompt-turn:${event.name}`)
      return options.promptTurnHandled ?? false
    },
    handleWaitingRoomKey: (event) => {
      calls.push(`waiting-room:${event.name}`)
      return options.waitingRoomHandled ?? false
    },
  }
  return {
    controller: createCliStdinKeyController(deps),
    calls: () => calls,
  }
}

function keyEvent(name: string, options: {
  eventType?: string
  ctrl?: boolean
  meta?: boolean
  alt?: boolean
  shift?: boolean
} = {}): CliStdinKeyEvent {
  const event: CliStdinKeyEvent = { name }
  if (options.eventType !== undefined) {
    event.eventType = options.eventType
  }
  if (options.ctrl !== undefined) {
    event.ctrl = options.ctrl
  }
  if (options.meta !== undefined) {
    event.meta = options.meta
  }
  if (options.alt !== undefined) {
    event.alt = options.alt
  }
  if (options.shift !== undefined) {
    event.shift = options.shift
  }
  return event
}
