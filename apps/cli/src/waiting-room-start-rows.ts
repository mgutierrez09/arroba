import {
  backendProviderLabel,
  type BackendProviderId,
  type CatalogModelOption,
} from "./provider-catalog.js"
import type { SliceRecord } from "./cli-types.js"
import { providerAccountCapacityLabel, type ProviderAccountProfile } from "@chariox/kernel-client"
import { formatWaitingRoomSliceSelection, waitingRoomSlices } from "./waiting-room-slices.js"
import {
  formatWaitingRoomLaunchKernelValue,
  formatWaitingRoomLaunchMachineValue,
  waitingRoomLaunchKernelOptions,
  waitingRoomLaunchMachineOptions,
  waitingRoomSelectedLaunchKernelRef,
  waitingRoomSelectedLaunchMachineRef,
} from "@chariox/kernel-client/waiting-room-runtime-placement"
import { describeWaitingRoomWorktreeSelection } from "./waiting-room-worktrees.js"
import type { WaitingRoomRemoteState, WaitingRoomRow, WaitingRoomState, WaitingRoomTargetState } from "./waiting-room-types.js"
import { describeWaitingRoomProjectSelection } from "./waiting-room-projects.js"
import {
  managedAutoStopLabel,
  managedDurationLabel,
  managedEnvironmentIsLaunchReady,
  managedKernelContextLabel,
  managedProviderAccountIsTransferable,
  managedProviderAccountSelections,
  selectedManagedEnvironment,
  waitingRoomConfiguresNewManagedMachine,
  waitingRoomProjectRepositoryOptions,
} from "./waiting-room-managed-environments.js"

export type WaitingRoomStartRowsChoice = {
  providerId: BackendProviderId
  model: CatalogModelOption | null
  effort: string
  accountProfile?: ProviderAccountProfile | null
  slice?: SliceRecord | null
  providerCatalogFallback?: boolean
}

export function waitingRoomStartRows(
  state: Pick<WaitingRoomState,
    | "focus"
    | "worktreeSelectionId"
    | "workspaceLiveSyncMode"
    | "selectedMachineRef"
    | "selectedKernelRef"
    | "projectSelectionId"
    | "sliceSelectionId"
    | "sliceDisplayMode"
    | "managedComputeClass"
    | "managedRegion"
    | "managedKernelContext"
    | "managedContextSourceTargetId"
    | "managedDevelopmentMode"
    | "managedRepositorySelection"
    | "managedRepositoryIndex"
    | "managedProviderAccountSource"
    | "managedProviderAccountSelection"
    | "managedProviderAccountIndex"
    | "managedGitCredentialSource"
    | "managedAutoStopPreset"
    | "managedCustomMinimumRuntimeSeconds"
    | "managedCustomIdleDelaySeconds"
    | "providerId"
    | "accountProfileId"
  >,
  choice: WaitingRoomStartRowsChoice,
  options: {
    modelOptions: CatalogModelOption[]
    remote?: WaitingRoomRemoteState
    targets?: WaitingRoomTargetState
    inventoryLoading: boolean
    loadingText: string
    visibleSessionCount: number
    titleWidth: number
  },
): WaitingRoomRow[] {
  const remote = options.remote ?? {}
  const selectedWorktreeLabel = describeWaitingRoomWorktreeSelection(
    state.worktreeSelectionId,
    options.targets?.worktreePath,
  )
  const selectedSliceLabel = formatWaitingRoomSliceSelection(
    state.sliceSelectionId,
    waitingRoomSlices(remote, {
      workspacePath: options.targets?.workspacePath,
      worktreeSelectionId: state.worktreeSelectionId,
      worktreePath: options.targets?.worktreePath,
      projectSelectionId: state.projectSelectionId,
      developmentMode: state.managedDevelopmentMode,
      repositorySelection: state.managedRepositorySelection,
      selectedMachineRef: state.selectedMachineRef,
      selectedKernelRef: state.selectedKernelRef,
    }),
    state.sliceDisplayMode,
  )
  const collaborationBackend = remote.collaborationBackend ?? "local"
  const configuresManaged = waitingRoomConfiguresNewManagedMachine(state.selectedMachineRef)
  const configuresSliceDevelopment = !configuresManaged
    && Boolean(state.sliceSelectionId && state.sliceSelectionId !== "none")
  const selectedEnvironment = selectedManagedEnvironment(state, remote)
  const managedRepositoryRows = waitingRoomManagedRepositoryRows(state, remote, options.titleWidth)
  const sliceDevelopmentRows: WaitingRoomRow[] = configuresSliceDevelopment
    ? [
        startRow("managed-development", "Development setup", state.managedDevelopmentMode === "current_project" ? "Current Project" : "Empty", state, options.titleWidth),
        ...managedRepositoryRows,
      ]
    : []
  return [
    {
      id: "new",
      title: configuresManaged
        ? "Create machine and start session"
        : selectedEnvironment && !managedEnvironmentIsLaunchReady(selectedEnvironment)
          ? "Start machine and session"
          : "Start New Session",
      value: "Press Enter",
      titleWidth: options.titleWidth,
      indent: 0,
      focused: state.focus === "new",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "launch-machine",
      title: "Machine",
      value: formatWaitingRoomLaunchMachineValue(state, remote),
      titleWidth: options.titleWidth,
      indent: 1,
      focused: state.focus === "launch-machine",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "launch-kernel",
      title: "Kernel",
      value: formatWaitingRoomLaunchKernelValue(state, remote),
      titleWidth: options.titleWidth,
      indent: 1,
      focused: state.focus === "launch-kernel",
      selectable: true,
      scrollbar: "",
    },
    ...((state.projectSelectionId !== undefined || remote.projects !== undefined)
      ? [{
          id: "project",
          title: "Project",
          value: describeWaitingRoomProjectSelection(
            state.projectSelectionId ?? "default",
            remote.projects,
            options.targets?.workspacePath,
          ),
          titleWidth: options.titleWidth,
          indent: 1,
          focused: state.focus === "project",
          selectable: true,
          scrollbar: "",
        }]
      : []),
    {
      id: "provider",
      title: "Provider",
      value: formatProviderValue(
        choice.providerId,
        choice.providerCatalogFallback,
      ),
      titleWidth: options.titleWidth,
      indent: 1,
      focused: state.focus === "provider",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "account",
      title: "Account",
      value: formatAccountValue(choice.accountProfile ?? null, choice.model?.id),
      titleWidth: options.titleWidth,
      indent: 1,
      focused: state.focus === "account",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "model",
      title: "Model",
      value: choice.model ? formatWaitingRoomModelValue(choice.model, options.modelOptions, choice.providerCatalogFallback) : "No models available",
      titleWidth: options.titleWidth,
      indent: 1,
      focused: state.focus === "model",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "effort",
      title: "Variant",
      value: formatVariantValue(choice.effort, choice.providerCatalogFallback),
      titleWidth: options.titleWidth,
      indent: 1,
      focused: state.focus === "effort",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "workspace",
      title: "Workspace",
      value: options.targets?.workspacePath ?? (options.inventoryLoading ? options.loadingText : "Set workspace path"),
      titleWidth: options.titleWidth,
      indent: 1,
      focused: state.focus === "workspace",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "worktree",
      title: "Worktree",
      value: options.targets?.worktreePath
        ? selectedWorktreeLabel
        : options.inventoryLoading
          ? options.loadingText
          : selectedWorktreeLabel,
      titleWidth: options.titleWidth,
      indent: 1,
      focused: state.focus === "worktree",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "live-sync",
      title: "Live Sync",
      value: formatWorkspaceLiveSyncMode(state.workspaceLiveSyncMode),
      titleWidth: options.titleWidth,
      indent: 1,
      focused: state.focus === "live-sync",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "collaborators",
      title: "Collaborators",
      value: collaborationBackend === "cloud" ? "use Cloud" : "after session start",
      titleWidth: options.titleWidth,
      indent: 1,
      focused: state.focus === "collaborators",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "slice",
      title: "Slice",
      value: options.inventoryLoading ? options.loadingText : selectedSliceLabel,
      titleWidth: options.titleWidth,
      indent: 1,
      focused: state.focus === "slice",
      selectable: true,
      scrollbar: "",
    },
    ...sliceDevelopmentRows,
    {
      id: "join-header",
      title: "Join Existing Session",
      value: options.inventoryLoading && options.visibleSessionCount === 0
        ? options.loadingText
        : options.visibleSessionCount > 0
          ? "Press Enter"
          : "",
      titleWidth: options.titleWidth,
      indent: 0,
      focused: state.focus === "join-sessions",
      selectable: true,
      scrollbar: "",
    },
  ]
}

function waitingRoomManagedRepositoryRows(
  state: Pick<WaitingRoomState,
    | "focus"
    | "projectSelectionId"
    | "managedDevelopmentMode"
    | "managedRepositorySelection"
    | "managedRepositoryIndex"
  >,
  remote: WaitingRoomRemoteState,
  titleWidth: number,
): WaitingRoomRow[] {
  const repositoryOptions = waitingRoomProjectRepositoryOptions(state, remote)
  const selectedSupportingRepositories = new Set(
    state.managedRepositorySelection?.supportingWorkspaceIds ?? repositoryOptions
      .slice(1)
      .map((option) => option.workspaceId),
  )
  const selectedRepositoryCount = repositoryOptions.filter((option) => (
    option.primary || selectedSupportingRepositories.has(option.workspaceId)
  )).length
  if (state.managedDevelopmentMode !== "current_project") {
    return [{
      id: "managed-repositories",
      title: "Selected repositories",
      value: "None",
      titleWidth,
      indent: 1,
      focused: false,
      selectable: false,
      scrollbar: "",
    }]
  }
  return [
    {
      id: "managed-repositories",
      title: "Selected repositories",
      value: `${selectedRepositoryCount} of ${repositoryOptions.length} included`,
      titleWidth,
      indent: 1,
      focused: false,
      selectable: false,
      scrollbar: "",
    },
    ...repositoryOptions.map((option, index): WaitingRoomRow => ({
      id: `managed-repository:${option.workspaceId}`,
      title: option.workspaceId,
      value: option.primary
        ? "Primary (included)"
        : selectedSupportingRepositories.has(option.workspaceId) ? "Included" : "Excluded",
      titleWidth,
      indent: 2,
      focused: !option.primary
        && state.focus === "managed-repositories"
        && (state.managedRepositoryIndex ?? 0) === index - 1,
      selectable: !option.primary,
      scrollbar: "",
    })),
  ]
}

export function waitingRoomManagedMachineDialogRows(
  state: WaitingRoomState,
  remote: WaitingRoomRemoteState = {},
  titleWidth = 28,
): WaitingRoomRow[] {
  const repositoryRows = waitingRoomManagedRepositoryRows(state, remote, titleWidth)
  const accountSelections = managedProviderAccountSelections(state, remote)
  return [
    startRow("managed-compute", "Compute class", state.managedComputeClass ?? "Unavailable", state, titleWidth),
    startRow("managed-region", "Region", state.managedRegion ?? "Unavailable", state, titleWidth),
    startRow("managed-kernel-context", "Kernel context from", managedKernelContextLabel(state, remote), state, titleWidth),
    startRow("managed-development", "Development setup", state.managedDevelopmentMode === "current_project" ? "Current Project" : "Empty", state, titleWidth),
    ...repositoryRows,
    startRow(
      "managed-provider-accounts",
      "Provider accounts source",
      state.managedProviderAccountSource === "none" ? "None" : `${accountSelections.length} selected`,
      state,
      titleWidth,
    ),
    ...(remote.providerAccounts ?? []).map((profile, managedProviderAccountIndex): WaitingRoomRow => {
      const transferable = managedProviderAccountIsTransferable(profile)
      const included = accountSelections.some((selection) => (
        selection.provider === profile.provider && selection.accountProfile === profile.profile_id
      ))
      return {
        id: `managed-provider-account:${profile.provider}:${profile.profile_id}`,
        title: profile.label,
        value: `${formatManagedProviderAccountFamily(profile.provider)} · ${included
          ? transferable ? "Included" : `Included, ${profile.auth_state}`
          : transferable ? "Excluded" : `Unavailable, ${profile.auth_state}`}`,
        titleWidth,
        indent: 2,
        focused: state.focus === "managed-provider-account"
          && (state.managedProviderAccountIndex ?? 0) === managedProviderAccountIndex,
        selectable: transferable,
        scrollbar: "",
      }
    }),
    startRow(
      "managed-git-credentials",
      "Git credentials source",
      state.managedGitCredentialSource === "none" ? "None" : "GitHub",
      state,
      titleWidth,
    ),
    startRow("managed-auto-stop", "Auto-stop policy", managedAutoStopLabel(state), state, titleWidth),
    ...(state.managedAutoStopPreset === "custom"
      ? [
          startRow(
            "managed-custom-minimum",
            "Minimum runtime",
            managedDurationLabel(state.managedCustomMinimumRuntimeSeconds),
            state,
            titleWidth,
          ),
          startRow(
            "managed-custom-idle",
            "Idle delay",
            managedDurationLabel(state.managedCustomIdleDelaySeconds),
            state,
            titleWidth,
          ),
        ]
      : []),
  ]
}

function startRow(
  id: WaitingRoomState["focus"],
  title: string,
  value: string,
  state: Pick<WaitingRoomState, "focus">,
  titleWidth: number,
): WaitingRoomRow {
  return {
    id,
    title,
    value,
    titleWidth,
    indent: 1,
    focused: state.focus === id,
    selectable: true,
    scrollbar: "",
  }
}

function formatAccountValue(profile: ProviderAccountProfile | null, model?: string | null): string {
  if (!profile) {
    return "Default (not discovered)"
  }
  return providerAccountCapacityLabel(profile, Date.now(), model)
}

function formatTitleCase(value: string) {
  return value.charAt(0).toUpperCase() + value.slice(1)
}

function formatManagedProviderAccountFamily(provider: string): string {
  if (provider === "opencode") return "OpenCode"
  if (provider === "codex") return "Codex"
  if (provider === "claude") return "Claude"
  return formatTitleCase(provider)
}

function formatBackendProviderLabel(providerId: BackendProviderId) {
  return backendProviderLabel(providerId)
}

function formatProviderValue(
  providerId: BackendProviderId,
  fallback = false,
) {
  const label = formatBackendProviderLabel(providerId)
  return fallback ? `${label} (local list)` : label
}

function formatWaitingRoomModelValue(
  model: CatalogModelOption,
  options: CatalogModelOption[],
  fallback = false,
) {
  const label = formatWaitingRoomModelLabel(model, options)
  return fallback ? `${label} (local list)` : label
}

function formatVariantValue(effort: string, fallback = false) {
  const label = effort ? formatTitleCase(effort) : "Default"
  return fallback ? `${label} (local list)` : label
}

function formatWorkspaceLiveSyncMode(mode: WaitingRoomState["workspaceLiveSyncMode"]) {
  if (mode === "managed") return "managed (selected workspace/worktree only; other repositories unrestricted)"
  if (mode === "tracked") return "tracked (turn-end; selected workspace/worktree only; other repositories unrestricted)"
  return "off (default; all repositories unrestricted)"
}

function formatWaitingRoomModelLabel(
  model: CatalogModelOption,
  options: CatalogModelOption[],
) {
  const providerCount = new Set(options.map((option) => option.providerId)).size
  return providerCount <= 1 ? model.label : `${model.providerName} ${model.label}`
}
