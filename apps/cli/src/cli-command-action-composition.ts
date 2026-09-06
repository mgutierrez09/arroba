import type { BootstrapState, RuntimeSession } from "./cli-types.js"
import type { CharioxLogger } from "./logging.js"
import { createCommandActionHandlers } from "./command-actions.js"
import { resolveConfiguredCloudRelayApiUrl } from "./cli-options.js"
import { bootstrapCloudRelayProfile } from "./cloud-relay.js"
import { importExternalProviderAgent } from "./external-provider-session-api.js"
import { openExternalUrl } from "./external-url.js"
import { formatAgentLabel } from "./agent-label.js"
import {
  aliasAgent,
  cycleAgentFocus as cycleAgentFocusApi,
  destroyAgent as destroyAgentApi,
  focusAgent as focusAgentApi,
  forkAgent as forkAgentApi,
  spawnAgent as spawnAgentApi,
  spawnAgents as spawnAgentsApi,
  undoTurn as undoTurnApi,
  updateAgentConfig,
  updateAgentProfile,
  updateAgentSubstitutes,
} from "./agent-api.js"
import {
  acceptCloudSessionInvite,
  createCloudSessionInvite,
  createSessionInvite,
  joinSessionInvite,
  listCloudCollaborators,
  listCloudSessionMembers,
} from "./cloud-session-api.js"
import {
  getUserConfig,
  getUserConfigSchema,
  getCredentialVaultStatus,
  lockCredentialVault,
  manageCredentialVault,
  setCredentialSecret,
  setWorkspaceLiveSyncMode,
  setUserConfigValue,
  unsetUserConfigValue,
} from "./config-api.js"
import {
  getMcpServer,
  getConnector,
  getConnectorAdapter,
  getCredential,
  getEnvironment,
  getScript,
  getSkill,
  grantAgentMcp,
  grantAgentConnector,
  grantAgentScript,
  grantAgentSkill,
  importMcpServers,
  importSkills,
  installMcpServer,
  installSkill,
  listEnvironments,
  listConnectors,
  listConnectorAdapters,
  listCredentials,
  listMcpServers,
  listHomeExtensionAudit,
  listScripts,
  listSkills,
  registerEnvironment,
  registerConnector,
  registerConnectorAdapter,
  registerCredential,
  registerScript,
  removeEnvironment,
  removeConnector,
  removeConnectorAdapter,
  removeCredential,
  removeScript,
  revokeAgentMcp,
  revokeAgentConnector,
  revokeAgentScript,
  revokeAgentSkill,
  syncRemoteExtensionManifest,
  uninstallMcpServer,
  uninstallSkill,
  updateMcpServer,
  updateSkill,
  validateScript,
  testConnector,
} from "./extension-api.js"
import { deleteKernel, exportDebugBundle, getDaemonHealth } from "./kernel-api.js"
import {
  mergeRelayCloudProfile,
  mergeUiPreferences,
  relayCloudProfile,
  saveRelayCloudProfile,
  saveUiPreferences,
} from "./preferences.js"
import {
  getProviderAuthStatus,
  getProviderCatalog,
  getProviderLoginStatus,
  listProviderAccountProfiles,
  createProviderAccountProfile,
  linkProviderAccountProfile,
  importNativeProviderAccountProfile,
  renameProviderAccountProfile,
  setDefaultProviderAccountProfile,
  refreshProviderAccountProfile,
  removeProviderAccountProfile,
  deleteProviderAccountProfileData,
  getProviderRun,
  launchProviderRun,
  launchProviderRuns,
  listProviderProcesses,
  logoutProvider,
  cancelProviderLogin,
  sendProviderLoginInput,
  startProviderLogin,
  teardownProviderProcesses,
  updateSessionConfig,
} from "./provider-api.js"
import {
  approveRemoteMachine,
  forgetRemoteMachine,
  listRemoteMachineKernels,
  listRemoteMachines,
  renameRemoteMachine,
} from "./remote-machine-api.js"
import {
  configureRelay,
  connectKernelCloudRelay,
  getRelayStatus,
  issueKernelCloudRelayClientToken,
  logoutCloudRelay,
  pairKernelCloudRelayClient,
  pairKernelCloudRelayMachine,
  pollCloudRelayLogin,
  startCloudRelayLogin,
} from "./relay-api.js"
import {
  aliasSession,
  abortMetaagentTask,
  createSession,
  deleteSessionByRef,
  getSessionState,
  listSessions,
  pauseMetaagentTask,
  resolveSession,
  resumeMetaagentTask,
  updateMetaagentTask,
} from "./session-api.js"
import { SESSION_CONFIG_RESPONSE_LAYOUT_KEY } from "@chariox/kernel-client/session-config-projection"
import { createAgentPromptScheduleRequest } from "@chariox/kernel-client/ipc-requests"
import { formatSessionList } from "./sessions.js"
import {
  createSlice,
  createSliceBackup,
  deleteSlice,
  getSlice,
  getSliceDisplayEndpoint,
  getSliceLogs,
  getSliceStateStatus,
  importSliceProviderAuth,
  listSliceAudit,
  listSlices,
  removeSliceProviderAuth,
  resetSliceState,
  saveSliceState,
  startSliceProviderLogin,
  startSlice,
  stopSlice,
} from "./slice-api.js"
import {
  attachWorkspaceLink,
  createWorkspaceLink,
  detachWorkspaceLink,
  getWorkspaceLiveSyncStatus,
  listWorkspaceLiveSyncAudit,
  listWorkspaceLinks,
  showWorkspaceLink,
} from "./workspace-link-api.js"

type AnyFn = (...args: any[]) => any

export type CliCommandActionCompositionDeps = {
  client: BootstrapState["client"]
  options: BootstrapState["options"]
  preferencesState: AnyFn
  setPreferencesState: AnyFn
  initialWorkspaceTarget: string
  initialWorktreeTarget: string
  pendingWorkspaceTarget: AnyFn
  pendingWorktreeTarget: AnyFn
  setPendingWorkspaceTarget: AnyFn
  setPendingWorktreeTarget: AnyFn
  isAttached: AnyFn
  sessionState: AnyFn
  attachmentState: AnyFn
  providerRunState: AnyFn
  currentModelId: AnyFn
  currentVariantId: AnyFn
  currentAccountProfileId?: AnyFn
  focusedAgentId: AnyFn
  multiAgentResponseLayout: AnyFn
  maxAgentsPerScreen: AnyFn
  flashFooter: AnyFn
  appendNotice: AnyFn
  readSecret?: AnyFn
  appendCloudNotice: AnyFn
  formatError: AnyFn
  attachBinding: AnyFn
  transitionToNoSession: AnyFn
  applyProviderSelection: AnyFn
  applyAccountSelection: AnyFn
  applyModelSelection: AnyFn
  applyVariantSelection: AnyFn
  applyModeSelection: AnyFn
  applyPermissionSelection: AnyFn
  currentExecutionMode: AnyFn
  currentPermissionLevel: AnyFn
  refreshWaitingRoomData: AnyFn
  remoteMachinesState: AnyFn
  setRemoteMachinesState: AnyFn
  reconcileWaitingRoom: AnyFn
  setSlicesState: AnyFn
  appLogger: CharioxLogger | null | undefined
  setMultiAgentResponseLayout: AnyFn
  applyResponseLayout: AnyFn
  applySessionState: AnyFn
  refreshAgentPanes: AnyFn
  setWorkspaceLiveSyncStatus?: AnyFn
  openWorkflowNodeInstructionsEditor: AnyFn
  closeWorkflowNodeInstructionsEditor: AnyFn
  getWorkflowNodeInstructionsDraft: AnyFn
  getWorkflowNodeInstructionsContext: AnyFn
  openWorkflowTerminalPanel: AnyFn
  rebuildTranscript: AnyFn
  requestRootRender: AnyFn
  scheduleTimer: (callback: () => void, delayMs: number) => unknown
  logViewDebug: AnyFn
  describeRenderableDebug: AnyFn
  currentFocusedRenderable: AnyFn
  trackAgentFocusTransition: AnyFn
  setProviderRunState: AnyFn
  resolveSessionAgent: AnyFn
  workflowScreenActive: AnyFn
  showWorkflowScreen: AnyFn
  selectedWorkflowId: AnyFn
  selectWorkflowCanvas: AnyFn
  replaceWorkflowDefinitions: AnyFn
  upsertWorkflowDefinition: AnyFn
  createWorkflow: AnyFn
  listWorkflows: AnyFn
  resolveWorkflow: AnyFn
  assignWorkflowAlias: AnyFn
  deleteWorkflow: AnyFn
  createWorkflowEndpoint: AnyFn
  assignWorkflowEndpointAlias: AnyFn
  bindWorkflowEndpoint: AnyFn
  setWorkflowEndpointMaxInstances: AnyFn
  removeWorkflowEndpoint: AnyFn
  addWorkflowNode: AnyFn
  removeWorkflowNode: AnyFn
  addWorkflowEdge: AnyFn
  removeWorkflowEdge: AnyFn
  updateWorkflowNodeInstructions: AnyFn
  setWorkflowNodeCanCompleteRun: AnyFn
  setWorkflowNodeCanEmitIntermediateOutput: AnyFn
  setWorkflowNodeWaitForAllInputs: AnyFn
  setWorkflowNodeIntermediateOutputSchema: AnyFn
  setWorkflowNodeMaxTurns: AnyFn
  invokeWorkflowEndpoint: AnyFn
  runWorkflowRegistryEntry: AnyFn
  listWorkflowPromptQueues: AnyFn
  createWorkflowPromptQueue: AnyFn
  updateWorkflowPromptQueue: AnyFn
  removeWorkflowPromptQueue: AnyFn
  listQueuedWorkflowPrompts: AnyFn
  updateQueuedWorkflowPrompt: AnyFn
  removeQueuedWorkflowPrompt: AnyFn
  clearWorkflowPromptQueue: AnyFn
  createWorkflowWatchdog: AnyFn
  listWorkflowWatchdogs: AnyFn
  setWorkflowWatchdogEnabled: AnyFn
  removeWorkflowWatchdog: AnyFn
  createWorkflowSchedule: AnyFn
  listWorkflowSchedules: AnyFn
  setWorkflowScheduleEnabled: AnyFn
  removeWorkflowSchedule: AnyFn
  setWorkflowFlushContext: AnyFn
  setWorkflowRunOutputSchema: AnyFn
  listWorkflowRuns: AnyFn
  getWorkflowRun: AnyFn
  cancelWorkflowRun: AnyFn
  pauseWorkflowRun: AnyFn
  resumeWorkflowRun: AnyFn
  refreshSplitPaneFocusRepaint: AnyFn
}

export function createCliCommandActionComposition(deps: CliCommandActionCompositionDeps) {
  const {
    client,
    options,
    preferencesState,
    setPreferencesState,
    initialWorkspaceTarget,
    initialWorktreeTarget,
    pendingWorkspaceTarget,
    pendingWorktreeTarget,
    setPendingWorkspaceTarget,
    setPendingWorktreeTarget,
    isAttached,
    sessionState,
    attachmentState,
    providerRunState,
    currentModelId,
    currentVariantId,
    currentAccountProfileId,
    focusedAgentId,
    multiAgentResponseLayout,
    maxAgentsPerScreen,
    flashFooter,
    appendNotice,
    readSecret,
    appendCloudNotice,
    formatError,
    attachBinding,
    transitionToNoSession,
    applyProviderSelection,
    applyAccountSelection,
    applyModelSelection,
    applyVariantSelection,
    applyModeSelection,
    applyPermissionSelection,
    currentExecutionMode,
    currentPermissionLevel,
    refreshWaitingRoomData,
    remoteMachinesState,
    setRemoteMachinesState,
    reconcileWaitingRoom,
    setSlicesState,
    appLogger,
    setMultiAgentResponseLayout,
    applyResponseLayout,
    applySessionState,
    refreshAgentPanes,
    setWorkspaceLiveSyncStatus,
    openWorkflowNodeInstructionsEditor,
    closeWorkflowNodeInstructionsEditor,
    getWorkflowNodeInstructionsDraft,
    getWorkflowNodeInstructionsContext,
    openWorkflowTerminalPanel,
    rebuildTranscript,
    requestRootRender,
    scheduleTimer,
    logViewDebug,
    describeRenderableDebug,
    currentFocusedRenderable,
    trackAgentFocusTransition,
    setProviderRunState,
    resolveSessionAgent,
    workflowScreenActive,
    showWorkflowScreen,
    selectedWorkflowId,
    selectWorkflowCanvas,
    replaceWorkflowDefinitions,
    upsertWorkflowDefinition,
    createWorkflow,
    listWorkflows,
    resolveWorkflow,
    assignWorkflowAlias,
    deleteWorkflow,
    createWorkflowEndpoint,
    assignWorkflowEndpointAlias,
    bindWorkflowEndpoint,
    setWorkflowEndpointMaxInstances,
    removeWorkflowEndpoint,
    addWorkflowNode,
    removeWorkflowNode,
    addWorkflowEdge,
    removeWorkflowEdge,
    updateWorkflowNodeInstructions,
    setWorkflowNodeCanCompleteRun,
    setWorkflowNodeCanEmitIntermediateOutput,
    setWorkflowNodeWaitForAllInputs,
    setWorkflowNodeIntermediateOutputSchema,
    setWorkflowNodeMaxTurns,
    invokeWorkflowEndpoint,
    runWorkflowRegistryEntry,
    listWorkflowPromptQueues,
    createWorkflowPromptQueue,
    updateWorkflowPromptQueue,
    removeWorkflowPromptQueue,
    listQueuedWorkflowPrompts,
    updateQueuedWorkflowPrompt,
    removeQueuedWorkflowPrompt,
    clearWorkflowPromptQueue,
    createWorkflowWatchdog,
    listWorkflowWatchdogs,
    setWorkflowWatchdogEnabled,
    removeWorkflowWatchdog,
    createWorkflowSchedule,
    listWorkflowSchedules,
    setWorkflowScheduleEnabled,
    removeWorkflowSchedule,
    setWorkflowFlushContext,
    setWorkflowRunOutputSchema,
    listWorkflowRuns,
    getWorkflowRun,
    cancelWorkflowRun,
    pauseWorkflowRun,
    resumeWorkflowRun,
    refreshSplitPaneFocusRepaint,
  } = deps

  return createCommandActionHandlers({
    ...(resolveConfiguredCloudRelayApiUrl(preferencesState())
      ? { cloudRelayApiUrl: resolveConfiguredCloudRelayApiUrl(preferencesState()) }
      : {}),
    workspace: initialWorkspaceTarget,
    worktree: initialWorktreeTarget,
    getWorkspaceTarget: pendingWorkspaceTarget,
    getWorktreeTarget: pendingWorktreeTarget,
    setWorkspaceTarget: setPendingWorkspaceTarget,
    setWorktreeTarget: setPendingWorktreeTarget,
    accountProfile: options.accountProfile,
    clientId: options.clientId,
    isAttached,
    sessionState,
    attachmentState,
    providerRunState,
    currentModelId,
    currentVariantId,
    currentProviderId: () => options.provider ?? "opencode",
    currentAccountProfileId: () => currentAccountProfileId?.() || options.accountProfile || "default",
    currentExecutionMode,
    currentPermissionLevel,
    focusedAgentId,
    multiAgentResponseLayout,
    maxAgentsPerScreen,
    isRelayConnection: () => Boolean(options.relayUrl),
    flashFooter,
    appendNotice,
    sendWorkflowEventPublicationRequest: (request) => client.send(request),
    appendCloudNotice,
    formatError,
    createSession: (workspace, worktree, alias, agentDefaults, worktreePlacement) =>
      createSession(client, workspace, worktree, alias, agentDefaults, undefined, undefined, undefined, worktreePlacement),
    createSessionInvite: (sessionId, expiresInMs, maxUses, collaborationLevel) =>
      createSessionInvite(client, sessionId, expiresInMs, maxUses, collaborationLevel),
    joinSessionInvite: (inviteToken, userId) => joinSessionInvite(client, inviteToken, userId),
    attachBinding: (session, createdSession) => attachBinding(session, createdSession),
    resolveSession: (reference, workspace) => resolveSession(client, reference, workspace),
    listSessions: () => listSessions(client),
    deleteSessionByRef: (reference, workspace) => deleteSessionByRef(client, reference, workspace),
    deleteKernel: () => deleteKernel(client),
    getDaemonHealth: () => getDaemonHealth(client),
    exportDebugBundle: (sessionId, label) => exportDebugBundle(client, sessionId, label),
    assignSessionAlias: (sessionId, alias) => aliasSession(client, sessionId, alias),
    aliasAgent: (sessionId, agentId, alias) => aliasAgent(client, sessionId, agentId, alias),
    updateAgentProfile: (sessionId, agentId, options) =>
      updateAgentProfile(client, sessionId, agentId, options),
    getProviderCatalogForAgent: async (agent, provider, accountProfile) => {
      const slices = await listSlices(client).catch(() => [])
      const slice = slices.find((entry) =>
        entry.agent_ids?.includes(agent.id)
        || (agent.remote_execution?.worker_kernel_id
          && entry.worker_kernel_id === agent.remote_execution.worker_kernel_id),
      )
      const workerRef = agent.remote_execution?.worker_kernel_id?.trim()
      return getProviderCatalog(client, appLogger, {
        provider,
        accountProfile,
        executionLocation: slice
          ? { kind: "slice", slice_ref: slice.id }
          : workerRef
            ? { kind: "worker", kernel_ref: workerRef }
            : { kind: "local" },
      }, false)
    },
    transitionToNoSession,
    applyProviderSelection,
    applyAccountSelection,
    applyModelSelection,
    applyVariantSelection,
    applyModeSelection,
    applyPermissionSelection,
    getProviderAuthStatus: (provider, accountProfile) => getProviderAuthStatus(client, provider, accountProfile),
    startProviderLogin: (provider, accountProfile, method) =>
      startProviderLogin(client, provider, accountProfile, method),
    getProviderLoginStatus: (loginId) => getProviderLoginStatus(client, loginId),
    sendProviderLoginInput: (loginId, dataBase64) => sendProviderLoginInput(client, loginId, dataBase64),
    cancelProviderLogin: (loginId) => cancelProviderLogin(client, loginId),
    ...(readSecret ? { readSecret } : {}),
    logoutProvider: (provider, accountProfile) => logoutProvider(client, provider, accountProfile),
    listProviderAccountProfiles: (provider) => listProviderAccountProfiles(client, provider),
    createProviderAccountProfile: (provider, label) => createProviderAccountProfile(client, provider, label),
    linkProviderAccountProfile: (provider, label, path) => linkProviderAccountProfile(client, provider, label, path),
    importNativeProviderAccountProfile: (provider) => importNativeProviderAccountProfile(client, provider),
    renameProviderAccountProfile: (provider, profile, label) => renameProviderAccountProfile(client, provider, profile, label),
    setDefaultProviderAccountProfile: (provider, profile) => setDefaultProviderAccountProfile(client, provider, profile),
    refreshProviderAccountProfile: (provider, profile) => refreshProviderAccountProfile(client, provider, profile),
    removeProviderAccountProfile: (provider, profile) => removeProviderAccountProfile(client, provider, profile),
    deleteProviderAccountProfileData: (provider, profile) => deleteProviderAccountProfileData(client, provider, profile),
    getRelayStatus: () => getRelayStatus(client),
    sendCredentialEnrollmentKernelRequest: (request) => client.send(request),
    sendDeploymentSetupKernelRequest: (request) => client.send(request),
    configureRelay: (relayUrl, relayToken) => configureRelay(client, relayUrl, relayToken),
    getCloudRelayProfile: () => relayCloudProfile(preferencesState()),
    saveCloudRelayProfile: async (profile) => {
      await saveRelayCloudProfile(profile)
      setPreferencesState((current: any) => mergeRelayCloudProfile(current, profile))
    },
    bootstrapCloudRelay: (apiUrl, email, accountSlug) =>
      bootstrapCloudRelayProfile({
        apiUrl,
        email,
        ...(accountSlug ? { accountSlug } : {}),
      }),
    startCloudDeviceLogin: (apiUrl, input) => startCloudRelayLogin(client, apiUrl, input),
    pollCloudDeviceLogin: (apiUrl, deviceCode) => pollCloudRelayLogin(client, apiUrl, deviceCode),
    openExternalUrl,
    logoutCloudRelay: (_profile, options) => logoutCloudRelay(client, options),
    pairCloudRelayClient: (_profile, clientId, alias) =>
      pairKernelCloudRelayClient(client, clientId, alias),
    pairCloudRelayMachine: (_profile, machineId, alias) =>
      pairKernelCloudRelayMachine(client, machineId, alias),
    issueCloudKernelRelayToken: async () => connectKernelCloudRelay(client),
    issueCloudMachineRelayToken: async () => connectKernelCloudRelay(client),
    issueCloudClientRelayToken: async (_profile, targetDaemonAlias, tokenOptions) =>
      issueKernelCloudRelayClientToken(
        client,
        targetDaemonAlias,
        options.clientId ?? "chariox-cli",
        tokenOptions?.sessionId ?? null,
      ),
    createCloudSessionInvite: (sessionId, inviteOptions) =>
      createCloudSessionInvite(client, sessionId, inviteOptions),
    acceptCloudSessionInvite: (inviteToken) => acceptCloudSessionInvite(client, inviteToken),
    listCloudSessionMembers: (sessionId) => listCloudSessionMembers(client, sessionId),
    listCloudCollaborators: () => listCloudCollaborators(client),
    getUserConfig: () => getUserConfig(client),
    getUserConfigSchema: () => getUserConfigSchema(client),
    setUserConfigValue: (path, value) => setUserConfigValue(client, path, value),
    setWorkspaceLiveSyncMode: (sessionId, mode) => setWorkspaceLiveSyncMode(client, sessionId, mode),
    unsetUserConfigValue: (path) => unsetUserConfigValue(client, path),
    refreshWaitingRoomData,
    getRemoteMachines: remoteMachinesState,
    setRemoteMachines: setRemoteMachinesState,
    reconcileWaitingRoom: () => reconcileWaitingRoom(),
    listRemoteMachines: () => listRemoteMachines(client),
    listRemoteMachineKernels: (machineRef) => listRemoteMachineKernels(client, machineRef),
    approveRemoteMachine: (machineRef) => approveRemoteMachine(client, machineRef),
    forgetRemoteMachine: (machineRef) => forgetRemoteMachine(client, machineRef),
    renameRemoteMachine: (machineRef, alias) => renameRemoteMachine(client, machineRef, alias),
    listSlices: async () => {
      const slices = await listSlices(client)
      setSlicesState(slices)
      return slices
    },
    createSlice: async (sliceOptions) => {
      const slice = await createSlice(client, sliceOptions)
      setSlicesState(await listSlices(client))
      return slice
    },
    getSlice: async (sliceRef) => getSlice(client, sliceRef),
    startSlice: async (sliceRef) => {
      const slice = await startSlice(client, sliceRef)
      setSlicesState(await listSlices(client))
      return slice
    },
    stopSlice: async (sliceRef) => {
      const slice = await stopSlice(client, sliceRef)
      setSlicesState(await listSlices(client))
      return slice
    },
    deleteSlice: async (sliceRef) => {
      const slice = await deleteSlice(client, sliceRef)
      setSlicesState(await listSlices(client))
      return slice
    },
    importSliceProviderAuth: async (sliceRef, provider, accountProfile) => {
      const result = await importSliceProviderAuth(client, sliceRef, provider, accountProfile)
      setSlicesState(await listSlices(client))
      return result
    },
    removeSliceProviderAuth: async (sliceRef, provider, accountProfile) => {
      const result = await removeSliceProviderAuth(client, sliceRef, provider, accountProfile)
      setSlicesState(await listSlices(client))
      return result
    },
    startSliceProviderLogin: async (sliceRef, provider, accountProfile) => {
      const result = await startSliceProviderLogin(client, sliceRef, provider, accountProfile)
      setSlicesState(await listSlices(client))
      return result
    },
    getSliceDisplayEndpoint: async (sliceRef) => getSliceDisplayEndpoint(client, sliceRef),
    getSliceLogs: async (sliceRef, tailLines) => getSliceLogs(client, sliceRef, tailLines),
    listSliceAudit: async (sliceRef, limit) => listSliceAudit(client, sliceRef, limit),
    saveSliceState: async (sliceRef, mode, scope) => {
      const result = await saveSliceState(client, sliceRef, mode, scope)
      setSlicesState(await listSlices(client))
      return result
    },
    getSliceStateStatus: async (sliceRef) => getSliceStateStatus(client, sliceRef),
    resetSliceState: async (sliceRef) => {
      const result = await resetSliceState(client, sliceRef)
      setSlicesState(await listSlices(client))
      return result
    },
    createSliceBackup: async (sliceRef, name) => {
      const result = await createSliceBackup(client, sliceRef, name)
      setSlicesState(await listSlices(client))
      return result
    },
    listProviderProcesses: (provider) => listProviderProcesses(client, provider),
    teardownProviderProcesses: (provider) => teardownProviderProcesses(client, provider),
    listMcpServers: () => listMcpServers(client, pendingWorkspaceTarget()),
    installMcpServer: (config) => installMcpServer(client, pendingWorkspaceTarget(), config),
    updateMcpServer: (config) => updateMcpServer(client, pendingWorkspaceTarget(), config),
    uninstallMcpServer: (name) => uninstallMcpServer(client, pendingWorkspaceTarget(), name),
    importMcpServers: (provider, name) => importMcpServers(client, pendingWorkspaceTarget(), provider, name),
    getMcpServer: (name) => getMcpServer(client, pendingWorkspaceTarget(), name),
    grantAgentMcp: (agentRef, name) => grantAgentMcp(client, pendingWorkspaceTarget(), agentRef, name),
    revokeAgentMcp: (agentRef, name) => revokeAgentMcp(client, agentRef, name),
    listSkills: () => listSkills(client, pendingWorkspaceTarget()),
    installSkill: (sourcePath) => installSkill(client, pendingWorkspaceTarget(), sourcePath),
    updateSkill: (sourcePath) => updateSkill(client, pendingWorkspaceTarget(), sourcePath),
    uninstallSkill: (name) => uninstallSkill(client, pendingWorkspaceTarget(), name),
    importSkills: (provider, name) => importSkills(client, pendingWorkspaceTarget(), provider, name),
    getSkill: (name) => getSkill(client, pendingWorkspaceTarget(), name),
    grantAgentSkill: (agentRef, name) => grantAgentSkill(client, pendingWorkspaceTarget(), agentRef, name),
    revokeAgentSkill: (agentRef, name) => revokeAgentSkill(client, agentRef, name),
    listEnvironments: () => listEnvironments(client, pendingWorkspaceTarget()),
    getEnvironment: (name) => getEnvironment(client, pendingWorkspaceTarget(), name),
    registerEnvironment: (config) => registerEnvironment(client, pendingWorkspaceTarget(), config),
    removeEnvironment: (name) => removeEnvironment(client, pendingWorkspaceTarget(), name),
    listScripts: () => listScripts(client, pendingWorkspaceTarget()),
    getScript: (name) => getScript(client, pendingWorkspaceTarget(), name),
    validateScript: (sourcePath, environment, name) => validateScript(client, pendingWorkspaceTarget(), sourcePath, environment, name),
    registerScript: (sourcePath, environment, name) => registerScript(client, pendingWorkspaceTarget(), sourcePath, environment, name),
    removeScript: (name) => removeScript(client, pendingWorkspaceTarget(), name),
    grantAgentScript: (agentRef, name, environment) => grantAgentScript(client, pendingWorkspaceTarget(), agentRef, name, environment),
    revokeAgentScript: (agentRef, name) => revokeAgentScript(client, agentRef, name),
    listCredentials: () => listCredentials(client),
    getCredential: (id) => getCredential(client, id),
    setCredentialSecret: (key, value) => setCredentialSecret(client, key, value, {
      sessionId: sessionState().id,
      agentId: focusedAgentId(),
    }),
    getCredentialVaultStatus: () => getCredentialVaultStatus(client),
    lockCredentialVault: () => lockCredentialVault(client),
    manageCredentialVault: () => manageCredentialVault(client, sessionState().id, focusedAgentId()),
    registerCredential: (sourcePath) => registerCredential(client, sourcePath),
    removeCredential: (id) => removeCredential(client, id),
    listConnectors: () => listConnectors(client),
    getConnector: (name) => getConnector(client, name),
    registerConnector: (sourcePath) => registerConnector(client, sourcePath),
    removeConnector: (name) => removeConnector(client, name),
    listConnectorAdapters: () => listConnectorAdapters(client),
    getConnectorAdapter: (name) => getConnectorAdapter(client, name),
    registerConnectorAdapter: (sourcePath) => registerConnectorAdapter(client, sourcePath),
    removeConnectorAdapter: (name) => removeConnectorAdapter(client, name),
    testConnector: (name, operation, input, credential, allow) => testConnector(client, name, operation, input, credential, allow),
    grantAgentConnector: (agentRef, name, credential, maxSafety) => grantAgentConnector(client, pendingWorkspaceTarget(), agentRef, name, credential, maxSafety),
    revokeAgentConnector: (agentRef, name) => revokeAgentConnector(client, agentRef, name),
    syncRemoteExtensionManifest: (agentRef) => syncRemoteExtensionManifest(client, agentRef),
    listHomeExtensionAudit: (agentRef, limit) => listHomeExtensionAudit(client, agentRef, limit),
    logViewCommand: (fields) => {
      appLogger?.info("handling view command", fields)
      logViewDebug("view command:after set layout", fields)
    },
    setMultiAgentResponseLayout,
    applyResponseLayout,
    updateSessionResponseLayout: (sessionId, attachmentId, layout) =>
      updateSessionConfig(
        client,
        sessionId,
        attachmentId,
        { [SESSION_CONFIG_RESPONSE_LAYOUT_KEY]: layout },
        false,
      ),
    updateSessionConfig: (sessionId, attachmentId, values, requiresIdle) =>
      updateSessionConfig(client, sessionId, attachmentId, values, requiresIdle),
    updateAgentConfig: (sessionId, agentId, options) =>
      updateAgentConfig(client, sessionId, agentId, options),
    updateAgentSubstitutes: (sessionId, agentId, action) =>
      updateAgentSubstitutes(client, sessionId, agentId, action),
    updateMetaagentTask: (sessionId, metaagentId, updates) =>
      updateMetaagentTask(client, sessionId, metaagentId, updates),
    pauseMetaagentTask: (sessionId, metaagentId) =>
      pauseMetaagentTask(client, sessionId, metaagentId),
    resumeMetaagentTask: (sessionId, metaagentId) =>
      resumeMetaagentTask(client, sessionId, metaagentId),
    abortMetaagentTask: (sessionId, metaagentId, reason) =>
      abortMetaagentTask(client, sessionId, metaagentId, reason),
    createAgentPromptSchedule: async (sessionId, agentId, kind, intervalSeconds, prompt) => {
      const response = await client.send<Record<string, unknown>>(
        createAgentPromptScheduleRequest({
          sessionId,
          agentId,
          kind,
          intervalSeconds,
          prompt,
        }),
      )
      const payload = "AgentPromptScheduleCreated" in response
        ? response.AgentPromptScheduleCreated as { session?: RuntimeSession }
        : null
      if (!payload?.session) {
        throw new Error("kernel did not return the scheduled session")
      }
      return { session: payload.session }
    },
    applySessionState,
    refreshAgentPanes,
    createWorkspaceLink: (name) => createWorkspaceLink(client, sessionState().id, name),
    listWorkspaceLinks: () => listWorkspaceLinks(client, sessionState().id),
    showWorkspaceLink: (linkRef) => showWorkspaceLink(client, sessionState().id, linkRef),
    attachWorkspaceLink: (linkRef, repoRoot) => attachWorkspaceLink(client, sessionState().id, linkRef, repoRoot),
    detachWorkspaceLink: (linkRef, repoRoot) => detachWorkspaceLink(client, sessionState().id, linkRef, repoRoot),
    getWorkspaceLiveSyncStatus: () => getWorkspaceLiveSyncStatus(client, sessionState().id),
    listWorkspaceLiveSyncAudit: (sessionId, limit) => listWorkspaceLiveSyncAudit(client, sessionId, limit),
    ...(setWorkspaceLiveSyncStatus ? { setWorkspaceLiveSyncStatus } : {}),
    openWorkflowNodeInstructionsEditor,
    closeWorkflowNodeInstructionsEditor,
    getWorkflowNodeInstructionsDraft,
    getWorkflowNodeInstructionsContext,
    openWorkflowTerminalPanel,
    saveUiPreferences: async (prefs) => {
      await saveUiPreferences(prefs)
      setPreferencesState((current: any) => mergeUiPreferences(current, prefs))
    },
    rebuildTranscript,
    requestRender: requestRootRender,
    afterViewRender: (layout) => {
      scheduleTimer(() => {
        logViewDebug("view command:post render tick", {
          requested_layout: layout,
          current_focus: describeRenderableDebug(currentFocusedRenderable()),
        })
      }, 0)
    },
    cycleAgentFocus: async () => {
      return trackAgentFocusTransition(async () => {
        const agent = await cycleAgentFocusApi(client, sessionState().id)
        const session = await getSessionState(client, sessionState().id)
        if (session.active_provider_run_id) {
          setProviderRunState(await getProviderRun(client, session.active_provider_run_id))
        } else {
          setProviderRunState(null)
        }
        return {
          agent,
          session,
        }
      })
    },
    launchAgentProviderRun: (provider, model, variant, agentId, accountProfile) =>
      launchProviderRun(
        client,
        sessionState().id,
        provider,
        accountProfile || options.accountProfile || "default",
        model,
        variant,
        agentId,
      ),
    setProviderRunState,
    refreshSessionState: (sessionId) => getSessionState(client, sessionId),
    undoTurn: (agentRef, turnRef) => undoTurnApi(client, sessionState().id, agentRef, turnRef),
    forkAgent: (sourceAgentRef, alias) => forkAgentApi(client, sessionState().id, sourceAgentRef, alias),
    spawnAgent: async (provider, alias, model, effort, worktreeId, machineRef, worktreePlacement, sliceRef, accountProfile) => {
      const agent = await spawnAgentApi(
        client,
        sessionState().id,
        {
          provider,
          accountProfile,
          alias,
          model,
          effort,
          worktreeId,
          kernelRef: machineRef,
          worktreePlacement,
          sliceRef,
        },
      )
      return {
        agent,
        session: await getSessionState(client, sessionState().id),
      }
    },
    spawnAgents: async (agents) => {
      const spawned = await spawnAgentsApi(
        client,
        sessionState().id,
        {
          agents: agents.map((agent) => ({
            provider: agent.provider,
            accountProfile: agent.accountProfile,
            alias: agent.alias,
            model: agent.model,
            effort: agent.effort,
            worktreeId: agent.worktreeId,
            kernelRef: agent.machineRef,
            worktreePlacement: agent.worktreePlacement,
            sliceRef: agent.sliceRef,
          })),
        },
      )
      return {
        agents: spawned,
        session: await getSessionState(client, sessionState().id),
      }
    },
    launchAgentProviderRuns: async (provider, model, variant, agentIds, accountProfile) => {
      const result = await launchProviderRuns(
        client,
        agentIds.map((agentId) => ({
          sessionId: sessionState().id,
          provider,
          accountProfile: accountProfile || options.accountProfile || "default",
          model,
          effort: variant,
          agentId,
        })),
        Math.min(8, Math.max(1, agentIds.length)),
      )
      return {
        runs: result.providerRuns,
        failures: result.failures,
      }
    },
    importExternalProviderAgent: async (externalSessionId) => {
      const payload = await importExternalProviderAgent(client, sessionState().id, externalSessionId)
      if (payload.providerRun) {
        setProviderRunState(payload.providerRun)
      }
      return {
        agent: payload.agent,
        session: payload.session,
        providerRun: payload.providerRun,
      }
    },
    destroyAgent: async (agentId) => {
      await destroyAgentApi(client, sessionState().id, agentId)
      return getSessionState(client, sessionState().id)
    },
    focusAgent: async (agentId) => {
      return trackAgentFocusTransition(async () => {
        const agent = await focusAgentApi(client, sessionState().id, agentId)
        const session = await getSessionState(client, sessionState().id)
        if (session.active_provider_run_id) {
          setProviderRunState(await getProviderRun(client, session.active_provider_run_id))
        } else {
          setProviderRunState(null)
        }
        return {
          agent,
          session,
        }
      })
    },
    resolveSessionAgent: (reference) => {
      const resolved = resolveSessionAgent(reference)
      return resolved.error
        ? { agent: resolved.agent ?? null, error: resolved.error }
        : { agent: resolved.agent ?? null }
    },
    workflowScreenActive,
    showWorkflowScreen,
    selectedWorkflowId,
    selectWorkflowCanvas,
    replaceWorkflowDefinitions,
    upsertWorkflowDefinition,
    createWorkflow,
    listWorkflows,
    resolveWorkflow,
    assignWorkflowAlias,
    deleteWorkflow,
    createWorkflowEndpoint,
    assignWorkflowEndpointAlias,
    bindWorkflowEndpoint,
    setWorkflowEndpointMaxInstances,
    removeWorkflowEndpoint,
    addWorkflowNode,
    removeWorkflowNode,
    addWorkflowEdge,
    removeWorkflowEdge,
    updateWorkflowNodeInstructions,
    setWorkflowNodeCanCompleteRun,
    setWorkflowNodeCanEmitIntermediateOutput,
    setWorkflowNodeWaitForAllInputs,
    setWorkflowNodeIntermediateOutputSchema,
    setWorkflowNodeMaxTurns,
    invokeWorkflowEndpoint,
    runWorkflowRegistryEntry,
    listWorkflowPromptQueues,
    createWorkflowPromptQueue,
    updateWorkflowPromptQueue,
    removeWorkflowPromptQueue,
    listQueuedWorkflowPrompts,
    updateQueuedWorkflowPrompt,
    removeQueuedWorkflowPrompt,
    clearWorkflowPromptQueue,
    createWorkflowWatchdog,
    listWorkflowWatchdogs,
    setWorkflowWatchdogEnabled,
    removeWorkflowWatchdog,
    createWorkflowSchedule,
    listWorkflowSchedules,
    setWorkflowScheduleEnabled,
    removeWorkflowSchedule,
    setWorkflowFlushContext,
    setWorkflowRunOutputSchema,
    listWorkflowRuns,
    getWorkflowRun,
    cancelWorkflowRun,
    pauseWorkflowRun,
    resumeWorkflowRun,
    formatAgentLabel,
    refreshSplitPaneFocusRepaint,
    formatSessionList: (sessions, currentSessionId) => formatSessionList(sessions, currentSessionId ?? undefined),
  })
}
