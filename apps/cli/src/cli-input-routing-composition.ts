import { parseKeypress } from "@opentui/core"
import { useKeyboard } from "@opentui/solid"

import { createCliStdinKeyController } from "./cli-stdin-key-controller.js"
import { createFocusedInteractionChoiceController } from "./focused-interaction-choice-controller.js"
import { createGlobalKeyboardShortcutController } from "./global-keyboard-shortcut-controller.js"
import { createNormalPromptSubmitController } from "./normal-prompt-submit-controller.js"
import { createPromptKeyDownController } from "./prompt-keydown-controller.js"
import { createPromptSubmitCoordinator } from "./prompt-submit-coordinator.js"
import { createPromptTurnNavigationController } from "./prompt-turn-navigation-controller.js"
import { createProviderNamespaceSubmitController } from "./provider-namespace-submit-controller.js"
import {
  preparePromptAttachmentsForSubmit,
  promptAttachmentTransferIsForced,
} from "./prompt-attachment-transfer.js"
import {
  respondToInteraction,
  submitPromptWithRecovery,
} from "./prompt-runtime-api.js"
import { getSessionState } from "./session-api.js"
import { sharedShellCommandForSlashCommand } from "./commands.js"
import { createSlashCommandSubmitController } from "./slash-command-submit-controller.js"
import { renderPromptTranscript } from "./transcript-render.js"
import { createWaitingRoomKeyController } from "./waiting-room-key-controller.js"
import { createWaitingRoomPromptBootstrapController } from "./waiting-room-prompt-bootstrap-controller.js"
import { handleWaitingRoomSlashCommand } from "./waiting-room-slash-command-policy.js"
import { createWorkspaceShellSubmitController } from "./workspace-shell-controller.js"
import { createWorkflowPromptSubmitController } from "./workflow-prompt-submit-controller.js"

type AnyFn = (...args: any[]) => any

export type CliInputRoutingCompositionDeps = {
  client: any
  options: any
  appLogger: any
  formatError: AnyFn
  isAttached: AnyFn
  sessionState: AnyFn
  recordPromptAreaHistoryEntry: AnyFn
  promptTextController: {
    clear: AnyFn
    currentText: AnyFn
    cursorOffset: AnyFn
    setText: AnyFn
  }
  setPromptHistoryIndex: AnyFn
  setPromptHistoryDraft: AnyFn
  clearCommandCenter: AnyFn
  flashFooter: AnyFn
  appendNotice: AnyFn
  requestExit: AnyFn
  requestWaitingRoom: AnyFn
  promptStopController: {
    request: AnyFn
  }
  handleAttachmentCommand: AnyFn
  handleSessionCommand: AnyFn
  handleProviderCommand: AnyFn
  handleAccountCommand: AnyFn
  handleModelCommand: AnyFn
  handleVariantCommand: AnyFn
  handleModeCommand: AnyFn
  handlePermissionsCommand: AnyFn
  handleViewCommand: AnyFn
  handleUndoCommand: AnyFn
  handleForkCommand: AnyFn
  handleAgentCommand: AnyFn
  handleKernelCommand: AnyFn
  handleMachineCommand: AnyFn
  handleSliceCommand: AnyFn
  handleRelayCommand: AnyFn
  handleCloudCommand: AnyFn
  handleCollabCommand: AnyFn
  handleConfigCommand: AnyFn
  handleWorkspaceCommand: AnyFn
  handleWorktreeCommand: AnyFn
  handleWorkflowCommand: AnyFn
  handleNotificationsCommand: AnyFn
  handleSettingsCommand: AnyFn
  handleLoopCommand: AnyFn
  handleGoalCommand: AnyFn
  handleWaitCommand: AnyFn
  handleMcpCommand: AnyFn
  handleSkillCommand: AnyFn
  handleEnvCommand: AnyFn
  handleScriptCommand: AnyFn
  handleCredentialCommand: AnyFn
  handleConnectorCommand: AnyFn
  handleExtensionCommand: AnyFn
  workspaceShellContext: AnyFn
  setWorkspaceShellContext: AnyFn
  workspaceShellEntryCounter: AnyFn
  setWorkspaceShellEntryCounter: AnyFn
  setWorkspaceShellEntries: AnyFn
  applySessionState: AnyFn
  selectedWorkflowId: AnyFn
  setSelectedWorkflowId: AnyFn
  setSelectedWorkflowNodeId: AnyFn
  rebuildTranscript: AnyFn
  workflowPromptState: AnyFn
  workflowInspectorMode: AnyFn
  setWorkflowInspectorMode: AnyFn
  selectedWorkflowNodeId: AnyFn
  selectedWorkflowComponent: AnyFn
  pendingAttachments: AnyFn
  beginSubmittedPromptUi: AnyFn
  restoreFailedPromptUi: AnyFn
  invokeWorkflowEndpoint: AnyFn
  focusedBackendProvider: AnyFn
  workflowScreenShowing: AnyFn
  waitForPendingAgentFocusTransition: AnyFn
  focusedAgentId: AnyFn
  primaryTranscriptRuntimeStore: {
    clearActiveToolLabels: AnyFn
    entryWrapperY: AnyFn
    setLastScrollTop: AnyFn
  }
  setProviderActivityLabel: AnyFn
  setActiveStatusLabel: AnyFn
  attachmentState: AnyFn
  appendUserPrompt: AnyFn
  setStreamingAgentId: AnyFn
  setWorking: AnyFn
  updateSessionChrome: AnyFn
  promptSubmissionAgentStateController: {
    getSubmittingAgentId: AnyFn
    setSubmittingAgentId: AnyFn
  }
  clearAgentBusy: AnyFn
  setSubmitting: AnyFn
  setFatalError: AnyFn
  setStatusLine: AnyFn
  promptInputRefController: {
    plainText: AnyFn
    isFocused: AnyFn
    hasInput: AnyFn
    focus: AnyFn
  }
  ensureBackgroundPollersStarted: AnyFn
  workflowNodeInstructionsEditor: AnyFn
  openWorkflowNodeInstructionsEditor: AnyFn
  closeWorkflowNodeInstructionsEditor: AnyFn
  focusedAgentInteraction: AnyFn
  interactionChoiceStore: {
    getSelectedIndex: AnyFn
    setSelectedIndex: AnyFn
    customReply: AnyFn
    setCustomReply: AnyFn
    clearCustomReply: AnyFn
    isCustomEditing: AnyFn
    setCustomEditing: AnyFn
  }
  renderAgentInteractions: AnyFn
  applyResponseLayout: AnyFn
  handleHotkeysToggleShortcut: AnyFn
  dialogOverlayOpen: AnyFn
  closeActiveDialogOverlay: AnyFn
  hasActiveTurnWork: AnyFn
  handleCommandCenterKey: AnyFn
  handleQueuedPromptKey: AnyFn
  commandCenterOpen: AnyFn
  promptHistoryIndex: AnyFn
  promptHistoryDraft: AnyFn
  navigatePromptHistoryInput: AnyFn
  visibleTranscriptEntries: AnyFn
  transcriptScrollboxRefController: {
    scrollState: AnyFn
    scrollTo: AnyFn
    requestRender: AnyFn
  }
  commandCenterController: {
    query: AnyFn
  }
  waitingRoomState: AnyFn
  availableSessions: AnyFn
  waitingRoomProjects: AnyFn
  waitingRoomTargets: AnyFn
  providerCatalogState: AnyFn
  relayStatusState: AnyFn
  remoteMachinesState: AnyFn
  remoteKernelsState: AnyFn
  providerAccountsState: AnyFn
  terminalsState: AnyFn
  slicesState: AnyFn
  themeRegistryState: AnyFn
  reconcileWaitingRoom: AnyFn
  setWaitingRoomState: AnyFn
  applyWaitingRoomSessionLifecycleAction: AnyFn
  restoreWaitingRoomProject: AnyFn
  renameWaitingRoomProject: AnyFn
  activateWaitingRoom: AnyFn
  startSessionFromWaitingRoomDefaults: AnyFn
  handleSessionBrowserKey: AnyFn
  handleManagedMachineDialogKey: AnyFn
  openManagedMachineDialog: AnyFn
  toggleWorkspaceScreen: AnyFn
  workflowScreenActive: AnyFn
  cycleWorkflowCanvasNode: AnyFn
  handleCycleAgentFocus: AnyFn
  copyPromptSelection: AnyFn
  removePromptAttachmentsForEdit: AnyFn
  removeLastPendingPromptAttachment: AnyFn
}

export function createCliInputRoutingComposition(deps: CliInputRoutingCompositionDeps) {
  const waitingRoomRemoteTargets = () => {
    const targets = deps.waitingRoomTargets()
    const managedEnvironmentCatalog = targets.managedEnvironmentCatalog
    return {
      ...(targets.workspaceId ? { workspaceId: targets.workspaceId } : {}),
      ...(targets.worktreeId ? { worktreeId: targets.worktreeId } : {}),
      ...(managedEnvironmentCatalog
        ? {
            managedComputeClasses: managedEnvironmentCatalog.computeClasses,
            managedContextSources: managedEnvironmentCatalog.contextSources,
            managedEnvironments: managedEnvironmentCatalog.environments,
          }
        : {}),
    }
  }
  let handleSharedShellCommand = async (_rawCommand: string): Promise<boolean> => false
  let pendingProjectRenameId: string | null = null
  const slashCommandSubmitController = createSlashCommandSubmitController({
    isAttached: deps.isAttached,
    getSessionId: () => deps.sessionState().id,
    recordPromptAreaHistoryEntry: deps.recordPromptAreaHistoryEntry,
    clearPromptText: () => deps.promptTextController.clear(),
    setPromptHistoryIndex: deps.setPromptHistoryIndex,
    setPromptHistoryDraft: deps.setPromptHistoryDraft,
    clearCommandCenter: deps.clearCommandCenter,
    flashFooter: deps.flashFooter,
    logError: (message, fields) => deps.appLogger?.error(message, fields),
    formatError: deps.formatError,
    onExit: deps.requestExit,
    onWaiting: deps.requestWaitingRoom,
    onStop: () => requestPromptStop(),
    handleAttachmentCommand: deps.handleAttachmentCommand,
    handleSessionCommand: deps.handleSessionCommand,
    handleProviderCommand: deps.handleProviderCommand,
    handleAccountCommand: deps.handleAccountCommand,
    handleModelCommand: deps.handleModelCommand,
    handleVariantCommand: deps.handleVariantCommand,
    handleModeCommand: deps.handleModeCommand,
    handlePermissionsCommand: deps.handlePermissionsCommand,
    handleViewCommand: deps.handleViewCommand,
    handleUndoCommand: deps.handleUndoCommand,
    handleForkCommand: deps.handleForkCommand,
    handleAgentCommand: deps.handleAgentCommand,
    handleKernelCommand: deps.handleKernelCommand,
    handleMachineCommand: deps.handleMachineCommand,
    handleSliceCommand: deps.handleSliceCommand,
    handleRelayCommand: deps.handleRelayCommand,
    handleCloudCommand: deps.handleCloudCommand,
    handleCollabCommand: deps.handleCollabCommand,
    handleConfigCommand: deps.handleConfigCommand,
    handleWorkspaceCommand: deps.handleWorkspaceCommand,
    handleWorktreeCommand: deps.handleWorktreeCommand,
    handleWorkflowCommand: deps.handleWorkflowCommand,
    handleNotificationsCommand: deps.handleNotificationsCommand,
    handleSettingsCommand: deps.handleSettingsCommand,
    handleLoopCommand: deps.handleLoopCommand,
    handleGoalCommand: deps.handleGoalCommand,
    handleWaitCommand: deps.handleWaitCommand,
    handleMcpCommand: deps.handleMcpCommand,
    handleSkillCommand: deps.handleSkillCommand,
    handleEnvCommand: deps.handleEnvCommand,
    handleScriptCommand: deps.handleScriptCommand,
    handleCredentialCommand: deps.handleCredentialCommand,
    handleConnectorCommand: deps.handleConnectorCommand,
    handleExtensionCommand: deps.handleExtensionCommand,
    handleSharedShellCommand: (rawCommand) => handleSharedShellCommand(rawCommand),
  })

  const workspaceShellSubmitController = createWorkspaceShellSubmitController({
    client: deps.client,
    clientId: deps.options.clientId,
    workspaceShellContext: deps.workspaceShellContext,
    setWorkspaceShellContext: (context) => {
      deps.setWorkspaceShellContext(context)
    },
    nextEntryId: () => {
      const id = deps.workspaceShellEntryCounter() + 1
      deps.setWorkspaceShellEntryCounter((counter: number) => counter + 1)
      return id
    },
    setWorkspaceShellEntries: (updater) => {
      deps.setWorkspaceShellEntries(updater)
    },
    sessionState: deps.sessionState,
    refreshSessionState: (sessionId) => getSessionState(deps.client, sessionId),
    applySessionState: deps.applySessionState,
    selectedWorkflowId: deps.selectedWorkflowId,
    setSelectedWorkflowId: deps.setSelectedWorkflowId,
    setSelectedWorkflowNodeId: deps.setSelectedWorkflowNodeId,
    rebuildTranscript: deps.rebuildTranscript,
    flashFooter: deps.flashFooter,
    onSessionRefreshError: (sessionId, error) => {
      deps.appLogger?.warn("workspace shell session refresh failed", {
        session_id: sessionId,
        error: deps.formatError(error),
      })
    },
  })
  const submitWorkspaceShellCommand = workspaceShellSubmitController.submit
  handleSharedShellCommand = async (rawCommand) => {
    const shellCommand = sharedShellCommandForSlashCommand(rawCommand)
    if (!shellCommand) {
      return false
    }
    if (!deps.isAttached()) {
      deps.flashFooter("start or join a session first", "error")
      return true
    }
    const result = await submitWorkspaceShellCommand(shellCommand)
    if (result.output && !deps.workflowScreenShowing()) {
      deps.appendNotice(result.output)
    }
    return true
  }

  const workflowPromptSubmitController = createWorkflowPromptSubmitController({
    getWorkflowPromptState: deps.workflowPromptState,
    getPendingAttachmentCount: () => deps.pendingAttachments().length,
    submitAgentPrompt: (rawPrompt, targetAgentId) => normalPromptSubmitController.submit(rawPrompt, targetAgentId),
    flashFooter: deps.flashFooter,
    formatError: deps.formatError,
  })

  const providerNamespaceSubmitController = createProviderNamespaceSubmitController({
    getFocusedProvider: deps.focusedBackendProvider,
    workflowScreenShowing: deps.workflowScreenShowing,
    getPendingAttachmentCount: () => deps.pendingAttachments().length,
    waitForPendingAgentFocusTransition: deps.waitForPendingAgentFocusTransition,
    getFocusedAgentId: deps.focusedAgentId,
    getSession: deps.sessionState,
    hasAgent: (agentId) => deps.sessionState().agents.some((agent: { id?: string }) => agent.id === agentId),
    clearActiveToolLabels: deps.primaryTranscriptRuntimeStore.clearActiveToolLabels,
    setProviderActivityLabel: deps.setProviderActivityLabel,
    setActiveStatusLabel: deps.setActiveStatusLabel,
    getAttachment: deps.attachmentState,
    getSessionId: () => deps.sessionState().id,
    clearPromptText: () => deps.promptTextController.clear(),
    beginSubmittedPromptUi: deps.beginSubmittedPromptUi,
    renderPromptTranscript,
    appendUserPrompt: deps.appendUserPrompt,
    submitProviderNamespacePrompt: (attachmentId, targetAgentId, forwardedPrompt) =>
      submitPromptWithRecovery(
        deps.client,
        deps.sessionState().id,
        attachmentId,
        targetAgentId,
        forwardedPrompt,
        [],
        deps.sessionState,
        deps.options,
        deps.appLogger,
      ),
    applySessionState: deps.applySessionState,
    setStreamingAgentId: deps.setStreamingAgentId,
    setWorking: deps.setWorking,
    updateSessionChrome: deps.updateSessionChrome,
    recordPromptAreaHistoryEntry: deps.recordPromptAreaHistoryEntry,
    clearCommandCenter: deps.clearCommandCenter,
    restoreFailedPromptUi: deps.restoreFailedPromptUi,
    getSubmittingAgentId: deps.promptSubmissionAgentStateController.getSubmittingAgentId,
    clearAgentBusy: deps.clearAgentBusy,
    setSubmittingAgentId: deps.promptSubmissionAgentStateController.setSubmittingAgentId,
    setSubmitting: deps.setSubmitting,
    setFatalError: deps.setFatalError,
    flashFooter: deps.flashFooter,
    logError: (message, fields) => deps.appLogger?.error(message, fields),
    formatError: deps.formatError,
  })

  const normalPromptSubmitController = createNormalPromptSubmitController({
    getPendingAttachments: deps.pendingAttachments,
    waitForPendingAgentFocusTransition: deps.waitForPendingAgentFocusTransition,
    getFocusedAgentId: deps.focusedAgentId,
    getSession: deps.sessionState,
    hasAgent: (agentId) => deps.sessionState().agents.some((agent: { id?: string }) => agent.id === agentId),
    clearActiveToolLabels: deps.primaryTranscriptRuntimeStore.clearActiveToolLabels,
    setProviderActivityLabel: deps.setProviderActivityLabel,
    setActiveStatusLabel: deps.setActiveStatusLabel,
    getAttachment: deps.attachmentState,
    getSessionId: () => deps.sessionState().id,
    clearPromptText: () => deps.promptTextController.clear(),
    shouldInlineLocalFiles: () => Boolean(deps.options.relayUrl) || promptAttachmentTransferIsForced(),
    preparePromptAttachmentsForSubmit,
    beginSubmittedPromptUi: deps.beginSubmittedPromptUi,
    renderPromptTranscript,
    appendUserPrompt: deps.appendUserPrompt,
    submitPrompt: (attachmentId, targetAgentId, prompt, attachments) =>
      submitPromptWithRecovery(
        deps.client,
        deps.sessionState().id,
        attachmentId,
        targetAgentId,
        prompt,
        attachments,
        deps.sessionState,
        deps.options,
        deps.appLogger,
      ),
    applySessionState: deps.applySessionState,
    setStreamingAgentId: deps.setStreamingAgentId,
    setWorking: deps.setWorking,
    updateSessionChrome: deps.updateSessionChrome,
    setStatusLine: deps.setStatusLine,
    recordPromptAreaHistoryEntry: deps.recordPromptAreaHistoryEntry,
    restoreFailedPromptUi: deps.restoreFailedPromptUi,
    getSubmittingAgentId: deps.promptSubmissionAgentStateController.getSubmittingAgentId,
    clearAgentBusy: deps.clearAgentBusy,
    setSubmittingAgentId: deps.promptSubmissionAgentStateController.setSubmittingAgentId,
    setSubmitting: deps.setSubmitting,
    setFatalError: deps.setFatalError,
    flashFooter: deps.flashFooter,
    logInfo: (message, fields) => deps.appLogger?.info(message, fields),
    logError: (message, fields) => deps.appLogger?.error(message, fields),
    formatError: deps.formatError,
  })

  const waitingRoomPromptBootstrapController = createWaitingRoomPromptBootstrapController({
    isAttached: deps.isAttached,
    startSessionFromWaitingRoomDefaults: async () => {
      if (pendingProjectRenameId) {
        const projectId = pendingProjectRenameId
        pendingProjectRenameId = null
        const name = deps.promptTextController.currentText().trim()
        await deps.renameWaitingRoomProject(projectId, name)
        deps.promptTextController.setText("")
        return
      }
      return deps.startSessionFromWaitingRoomDefaults()
    },
    flashFooter: deps.flashFooter,
    formatError: deps.formatError,
    warn: (message, fields) => deps.appLogger?.warn(message, fields),
  })

  const promptSubmitCoordinator = createPromptSubmitCoordinator({
    getPromptText: deps.promptInputRefController.plainText,
    ensureBackgroundPollersStarted: () => deps.ensureBackgroundPollersStarted(),
    getPendingAttachmentCount: () => deps.pendingAttachments().length,
    clearPromptText: () => deps.promptTextController.clear(),
    workflowScreenShowing: deps.workflowScreenShowing,
    submitWorkspaceShellCommand: async (rawPrompt) => {
      await submitWorkspaceShellCommand(rawPrompt)
    },
    workflowNodeInstructionsEditorOpen: () => Boolean(deps.workflowNodeInstructionsEditor()),
    submitDetachedSlashCommand: (rawPrompt) =>
      handleWaitingRoomSlashCommand(rawPrompt, {
        clearCommandCenter: deps.clearCommandCenter,
        clearPromptText: () => deps.promptTextController.clear(),
        flashFooter: deps.flashFooter,
      }),
    submitSlashCommand: async (rawPrompt, submitOptions) =>
      Boolean(await slashCommandSubmitController.submit(rawPrompt, submitOptions)),
    submitProviderNamespacePrompt: (rawPrompt) => providerNamespaceSubmitController.submit(rawPrompt),
    bootstrapDetachedPrompt: () => waitingRoomPromptBootstrapController.bootstrap(),
    isAttached: deps.isAttached,
    submitWorkflowPrompt: (rawPrompt) => workflowPromptSubmitController.submit(rawPrompt),
    submitNormalPrompt: (rawPrompt) => normalPromptSubmitController.submit(rawPrompt),
    flashFooter: deps.flashFooter,
    formatError: deps.formatError,
  })
  const submitPrompt = promptSubmitCoordinator.submit

  const requestPromptStop = async () => {
    await deps.promptStopController.request()
  }

  const focusedInteractionChoiceController = createFocusedInteractionChoiceController({
    getFocusedInteraction: deps.focusedAgentInteraction,
    isAttached: deps.isAttached,
    getSessionId: () => deps.sessionState().id,
    getSelectedIndex: deps.interactionChoiceStore.getSelectedIndex,
    setSelectedIndex: deps.interactionChoiceStore.setSelectedIndex,
    getCustomReply: deps.interactionChoiceStore.customReply,
    setCustomReply: deps.interactionChoiceStore.setCustomReply,
    clearCustomReply: deps.interactionChoiceStore.clearCustomReply,
    isCustomEditing: deps.interactionChoiceStore.isCustomEditing,
    setCustomEditing: deps.interactionChoiceStore.setCustomEditing,
    renderAgentInteractions: deps.renderAgentInteractions,
    applyResponseLayout: deps.applyResponseLayout,
    respondToInteraction: (sessionId, interactionId, choiceId, customReply) =>
      respondToInteraction(deps.client, sessionId, interactionId, choiceId, customReply),
    applySessionState: deps.applySessionState,
    flashFooter: deps.flashFooter,
    formatError: deps.formatError,
  })
  const submitFocusedInteractionChoice = focusedInteractionChoiceController.submitChoice
  const cycleFocusedInteractionChoice = focusedInteractionChoiceController.cycleChoice
  const handleFocusedInteractionKey = focusedInteractionChoiceController.handleKey

  const globalKeyboardShortcutController = createGlobalKeyboardShortcutController({
    handleHotkeysToggleShortcut: deps.handleHotkeysToggleShortcut,
    dialogOverlayOpen: deps.dialogOverlayOpen,
    closeActiveDialogOverlay: deps.closeActiveDialogOverlay,
    requestExit: () => {
      void deps.requestExit()
    },
    requestPromptStop: () => {
      void requestPromptStop()
    },
    hasActiveTurnWork: deps.hasActiveTurnWork,
  })
  useKeyboard(globalKeyboardShortcutController.handleKey)
  const handleSigint = globalKeyboardShortcutController.handleSigint

  const promptKeyDownController = createPromptKeyDownController({
    handleFocusedInteractionKey,
    handleCommandCenterKey: deps.handleCommandCenterKey,
    handleQueuedPromptKey: deps.handleQueuedPromptKey,
    isAttached: deps.isAttached,
    promptFocused: deps.promptInputRefController.isFocused,
    commandCenterOpen: deps.commandCenterOpen,
    currentPromptText: () => deps.promptTextController.currentText(),
    promptCursorOffset: () => deps.promptTextController.cursorOffset(),
    promptHistoryIndex: deps.promptHistoryIndex,
    promptHistoryDraft: deps.promptHistoryDraft,
    navigatePromptHistoryInput: deps.navigatePromptHistoryInput,
    handleHotkeysToggleShortcut: deps.handleHotkeysToggleShortcut,
  })
  const handlePromptKeyDown = (
    event: Parameters<typeof promptKeyDownController.handleKeyDown>[0],
  ) => {
    if (
      pendingProjectRenameId
      && event.eventType !== "release"
      && event.name === "escape"
    ) {
      pendingProjectRenameId = null
      deps.promptTextController.clear()
      deps.flashFooter("project rename canceled", "info")
      event.preventDefault?.()
      event.stopPropagation?.()
      return true
    }
    return promptKeyDownController.handleKeyDown(event)
  }

  const promptTurnNavigationController = createPromptTurnNavigationController({
    isAttached: deps.isAttached,
    getPromptText: () => deps.promptInputRefController.hasInput() ? deps.promptTextController.currentText() : undefined,
    getPromptOffsets: () => deps.visibleTranscriptEntries()
      .filter((entry: { role: string }) => entry.role === "user")
      .map((entry: { id: string }) => deps.primaryTranscriptRuntimeStore.entryWrapperY(entry.id))
      .filter((offset: number | null): offset is number => offset !== null),
    getScrollState: deps.transcriptScrollboxRefController.scrollState,
    scrollTo: deps.transcriptScrollboxRefController.scrollTo,
    requestRender: deps.transcriptScrollboxRefController.requestRender,
    setLastTranscriptScrollTop: deps.primaryTranscriptRuntimeStore.setLastScrollTop,
  })

  const waitingRoomKeyController = createWaitingRoomKeyController({
    isAttached: deps.isAttached,
    hotkeysOpen: deps.dialogOverlayOpen,
    promptFocused: deps.promptInputRefController.isFocused,
    commandCenterOpen: deps.commandCenterOpen,
    commandCenterQuery: () => deps.commandCenterController.query(),
    getWaitingRoomState: deps.waitingRoomState,
    getSessions: deps.availableSessions,
    getProviderCatalog: deps.providerCatalogState,
    getRemoteState: () => ({
      ...waitingRoomRemoteTargets(),
      relay: deps.relayStatusState(),
      machines: deps.remoteMachinesState(),
      kernels: deps.remoteKernelsState(),
      providerAccounts: deps.providerAccountsState(),
      terminals: deps.terminalsState(),
      slices: deps.slicesState(),
      projects: deps.waitingRoomProjects(),
    }),
    getThemeRegistry: deps.themeRegistryState,
    reconcileWaitingRoom: deps.reconcileWaitingRoom,
    setWaitingRoomState: deps.setWaitingRoomState,
    rebuildTranscript: deps.rebuildTranscript,
    applyLifecycleAction: (action) => {
      void deps.applyWaitingRoomSessionLifecycleAction(action)
    },
    beginProjectRename: (projectId, currentName) => {
      pendingProjectRenameId = projectId
      deps.promptTextController.setText(currentName)
      deps.promptInputRefController.focus()
      deps.flashFooter("edit the project name and press Enter", "info")
    },
    restoreProject: (projectId) => {
      void deps.restoreWaitingRoomProject(projectId)
    },
    activateWaitingRoom: () => {
      void deps.activateWaitingRoom()
    },
    openManagedMachineDialog: deps.openManagedMachineDialog,
  })

  const handleWorkflowDetailPaneKey = (event: { eventType?: string; name?: string; ctrl?: boolean; meta?: boolean; alt?: boolean }) => {
    if (
      !deps.workflowScreenActive()
      || deps.promptInputRefController.isFocused()
      || deps.commandCenterOpen()
      || event.eventType === "release"
      || event.ctrl
      || event.meta
      || event.alt
    ) {
      return false
    }
    const setMode = (mode: "logs" | "trace" | "edit") => {
      deps.setWorkflowInspectorMode(mode)
      deps.rebuildTranscript()
      return true
    }
    if (event.name === "l") {
      return setMode("logs")
    }
    if (event.name === "t") {
      return setMode("trace")
    }
    if (event.name === "e") {
      return setMode("edit")
    }
    if (event.name === "escape") {
      if (deps.workflowNodeInstructionsEditor()) {
        deps.closeWorkflowNodeInstructionsEditor()
        return true
      }
      if (deps.workflowInspectorMode() === "edit") {
        return setMode(deps.selectedWorkflowNodeId() ? "trace" : "logs")
      }
      return false
    }
    if (event.name === "return" || event.name === "enter") {
      const component = deps.selectedWorkflowComponent()
      if (component?.kind && component.kind !== "node") {
        return setMode("edit")
      }
      const workflow = deps.sessionState().workflows?.find((entry: { id: string }) => entry.id === deps.selectedWorkflowId()) ?? null
      const node = workflow?.nodes?.find((entry: { id: string }) => entry.id === deps.selectedWorkflowNodeId()) ?? null
      if (!workflow || !node) {
        deps.flashFooter("select a workflow node to edit", "info")
        return true
      }
      deps.openWorkflowNodeInstructionsEditor(workflow.id, node.id, node.instructions ?? "")
      return true
    }
    return false
  }

  const stdinKeyController = createCliStdinKeyController({
    parseKeypress: (chunk, options) => parseKeypress(chunk, options),
    dialogOverlayOpen: deps.dialogOverlayOpen,
    closeActiveDialogOverlay: deps.closeActiveDialogOverlay,
    handleManagedMachineDialogKey: deps.handleManagedMachineDialogKey,
    handleSessionBrowserKey: deps.handleSessionBrowserKey,
    requestExit: () => {
      void deps.requestExit()
    },
    focusedInteractionActive: () => Boolean(deps.focusedAgentInteraction()),
    handleFocusedInteractionKey,
    handleQueuedPromptKey: deps.handleQueuedPromptKey,
    promptFocused: deps.promptInputRefController.isFocused,
    commandCenterOpen: deps.commandCenterOpen,
    commandCenterQuery: () => deps.commandCenterController.query(),
    clearCommandCenter: deps.clearCommandCenter,
    toggleWorkspaceScreen: deps.toggleWorkspaceScreen,
    isAttached: deps.isAttached,
    workflowScreenActive: deps.workflowScreenActive,
    cycleWorkflowCanvasNode: deps.cycleWorkflowCanvasNode,
    handleWorkflowDetailPaneKey,
    cycleAgentFocus: () => {
      void deps.handleCycleAgentFocus()
    },
    copyPromptSelection: deps.copyPromptSelection,
    hasActiveTurnWork: deps.hasActiveTurnWork,
    requestPromptStop: () => {
      void requestPromptStop()
    },
    removePromptAttachmentsForEdit: deps.removePromptAttachmentsForEdit,
    currentPromptText: () => deps.promptTextController.currentText(),
    pendingAttachmentCount: () => deps.pendingAttachments().length,
    removeLastPendingPromptAttachment: deps.removeLastPendingPromptAttachment,
    handlePromptTurnNavigationKey: (event) => promptTurnNavigationController.handleKey({
      ...event,
      eventType: event.eventType ?? "",
    }),
    handleWaitingRoomKey: waitingRoomKeyController.handleKey,
  })

  return {
    cycleFocusedInteractionChoice,
    handlePromptKeyDown,
    handleSigint,
    handleStdinData: stdinKeyController.handleData,
    requestPromptStop,
    submitFocusedInteractionChoice,
    submitPrompt,
    handleSharedShellCommand,
    submitWorkspaceShellCommand,
  }
}
