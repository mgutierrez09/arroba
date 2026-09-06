import { shouldCycleFocusOnTabEvent } from "./hotkeys.js"
import type { ParsedShortcut } from "./keybind.js"

export type CliStdinKeyEvent = ParsedShortcut & {
  alt?: boolean
}

export type CliStdinKeypressParser = (
  chunk: Buffer | string,
  options: { useKittyKeyboard: boolean },
) => CliStdinKeyEvent | null

export type CliStdinKeyControllerDeps = {
  parseKeypress: CliStdinKeypressParser
  dialogOverlayOpen: () => boolean
  closeActiveDialogOverlay: () => void
  handleManagedMachineDialogKey?: (event: CliStdinKeyEvent) => boolean
  handleSessionBrowserKey: (event: CliStdinKeyEvent) => boolean
  requestExit: () => void
  focusedInteractionActive: () => boolean
  handleFocusedInteractionKey: (event: CliStdinKeyEvent) => boolean
  handleQueuedPromptKey: (event: CliStdinKeyEvent) => boolean
  promptFocused: () => boolean
  commandCenterOpen: () => boolean
  commandCenterQuery: () => string
  clearCommandCenter: () => void
  toggleWorkspaceScreen: () => void
  isAttached: () => boolean
  workflowScreenActive: () => boolean
  cycleWorkflowCanvasNode: () => void
  handleWorkflowDetailPaneKey: (event: CliStdinKeyEvent) => boolean
  cycleAgentFocus: () => void
  copyPromptSelection: () => boolean
  hasActiveTurnWork: () => boolean
  requestPromptStop: () => void
  removePromptAttachmentsForEdit: (edit: "backspace" | "delete") => boolean
  currentPromptText: () => string
  pendingAttachmentCount: () => number
  removeLastPendingPromptAttachment: () => void
  handlePromptTurnNavigationKey: (event: CliStdinKeyEvent) => boolean
  handleWaitingRoomKey: (event: CliStdinKeyEvent) => boolean
}

export type CliStdinKeyController = {
  handleData(chunk: Buffer | string): boolean
}

export function createCliStdinKeyController(
  deps: CliStdinKeyControllerDeps,
): CliStdinKeyController {
  return {
    handleData(chunk) {
      const event = deps.parseKeypress(chunk, { useKittyKeyboard: true })
      if (!event) {
        return false
      }
      if (event.eventType !== "release" && deps.dialogOverlayOpen() && event.name === "escape") {
        deps.closeActiveDialogOverlay()
        return true
      }
      if (deps.handleManagedMachineDialogKey?.(event)) {
        return true
      }
      if (deps.handleSessionBrowserKey(event)) {
        return true
      }
      if (event.eventType !== "release" && event.ctrl && event.name === "e") {
        deps.requestExit()
        return true
      }
      // The focused textarea receives the same terminal key through its
      // onKeyDown handler. Let it exclusively own an active interaction so a
      // printable key is not appended once here and once by the textarea.
      if (deps.promptFocused() && deps.focusedInteractionActive()) {
        return true
      }
      if (deps.handleFocusedInteractionKey(event)) {
        return true
      }
      const queuedPromptKeyEvent = queuedPromptKeyEventFromStdin(event)
      if (queuedPromptKeyEvent && deps.handleQueuedPromptKey(queuedPromptKeyEvent)) {
        return true
      }
      if (deps.promptFocused() && deps.commandCenterOpen()) {
        if (event.eventType !== "release" && event.name === "escape") {
          deps.clearCommandCenter()
        }
        return true
      }
      if (event.eventType !== "release" && event.ctrl && event.name === "p") {
        if (deps.dialogOverlayOpen()) {
          return true
        }
        deps.toggleWorkspaceScreen()
        return true
      }
      if (shouldCycleFocusOnTabEvent(event, {
        attached: deps.isAttached(),
        hotkeysOpen: deps.dialogOverlayOpen(),
        promptFocused: deps.promptFocused(),
        commandCenterOpen: deps.commandCenterOpen(),
        commandCenterQuery: deps.commandCenterQuery(),
      })) {
        if (deps.workflowScreenActive()) {
          deps.cycleWorkflowCanvasNode()
        } else {
          deps.cycleAgentFocus()
        }
        return true
      }
      if (event.eventType !== "release" && event.meta && event.name === "c" && deps.copyPromptSelection()) {
        return true
      }
      if (event.ctrl && event.name === "c") {
        if (deps.hasActiveTurnWork()) {
          deps.requestPromptStop()
        } else {
          deps.requestExit()
        }
        return true
      }
      if (deps.dialogOverlayOpen()) {
        return true
      }
      if (deps.workflowScreenActive() && deps.handleWorkflowDetailPaneKey(event)) {
        return true
      }
      if (event.eventType !== "release" && deps.promptFocused()) {
        if (event.name === "backspace" && deps.removePromptAttachmentsForEdit("backspace")) {
          return true
        }
        if (event.name === "delete" && deps.removePromptAttachmentsForEdit("delete")) {
          return true
        }
      }
      if (
        event.eventType !== "release"
        && event.name === "backspace"
        && deps.isAttached()
        && !deps.currentPromptText()
        && deps.pendingAttachmentCount() > 0
      ) {
        deps.removeLastPendingPromptAttachment()
        return true
      }
      if (deps.handlePromptTurnNavigationKey(event)) {
        return true
      }
      if (deps.handleWaitingRoomKey(event)) {
        return true
      }
      return false
    },
  }
}

function queuedPromptKeyEventFromStdin(event: CliStdinKeyEvent): CliStdinKeyEvent | null {
  if (
    event.ctrl
    || event.shift
  ) {
    return null
  }
  if (
    event.name !== "s"
    && event.name !== "c"
    && event.name !== "j"
    && event.name !== "k"
    && event.name !== "up"
    && event.name !== "down"
  ) {
    return null
  }
  if (event.alt) {
    return event
  }
  if (!event.meta) {
    return null
  }
  return {
    ...event,
    alt: true,
    meta: false,
  }
}
