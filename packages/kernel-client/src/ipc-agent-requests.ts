export function aliasAgentRequest(sessionId: string, agentId: string, alias: string) {
  return {
    AliasAgent: {
      session_id: sessionId,
      agent_id: agentId,
      alias,
    },
  }
}

export function spawnAgentRequest(
  sessionId: string,
  provider?: string | null,
  alias?: string,
  model?: string | null,
  worktreeId?: string,
  effort?: string | null,
  executionMode?: "build" | "plan",
  permissionLevel?: "required" | "yolo",
  kernelRef?: string,
  worktreePlacement?: Record<string, unknown>,
  sliceRef?: string,
  accountProfile?: string | null,
) {
  return {
    SpawnAgent: {
      session_id: sessionId,
      provider: provider ?? null,
      ...(accountProfile ? { account_profile: accountProfile } : {}),
      alias: alias ?? null,
      model: model ?? null,
      effort: effort ?? null,
      execution_mode: executionMode ?? null,
      permission_level: permissionLevel ?? null,
      worktree_id: worktreeId ?? null,
      kernel_ref: kernelRef ?? null,
      slice_ref: sliceRef ?? null,
      worktree_placement: worktreePlacement ?? null,
    },
  }
}

export type SpawnAgentBatchItem = {
  provider?: string | null
  accountProfile?: string | null
  alias?: string | null
  model?: string | null
  worktreeId?: string | null
  effort?: string | null
  executionMode?: "build" | "plan" | null
  permissionLevel?: "required" | "yolo" | null
  kernelRef?: string | null
  worktreePlacement?: Record<string, unknown> | null
  sliceRef?: string | null
}

export function spawnAgentsRequest(sessionId: string, agents: SpawnAgentBatchItem[]) {
  return {
    SpawnAgents: {
      session_id: sessionId,
      agents: agents.map((agent) => ({
        provider: agent.provider ?? null,
        ...(agent.accountProfile ? { account_profile: agent.accountProfile } : {}),
        alias: agent.alias ?? null,
        model: agent.model ?? null,
        effort: agent.effort ?? null,
        execution_mode: agent.executionMode ?? null,
        permission_level: agent.permissionLevel ?? null,
        worktree_id: agent.worktreeId ?? null,
        kernel_ref: agent.kernelRef ?? null,
        slice_ref: agent.sliceRef ?? null,
        worktree_placement: agent.worktreePlacement ?? null,
      })),
    },
  }
}

export function undoTurnRequest(sessionId: string, agentRef?: string | null, turnRef?: string | null) {
  return {
    UndoTurn: {
      session_id: sessionId,
      agent_ref: agentRef ?? null,
      turn_ref: turnRef ?? null,
    },
  }
}

export function forkAgentRequest(sessionId: string, sourceAgentRef?: string | null, alias?: string | null) {
  return {
    ForkAgent: {
      session_id: sessionId,
      source_agent_ref: sourceAgentRef ?? null,
      alias: alias ?? null,
    },
  }
}

export function updateAgentConfigRequest(options: {
  sessionId: string
  agentId: string
  executionMode?: "build" | "plan" | null
  clearExecutionMode?: boolean
  permissionLevel?: "required" | "yolo" | null
  clearPermissionLevel?: boolean
  workspaceId?: string | null
  clearWorkspaceId?: boolean
  worktreeId?: string | null
  clearWorktreeId?: boolean
}) {
  return {
    UpdateAgentConfig: {
      session_id: options.sessionId,
      agent_id: options.agentId,
      execution_mode: options.executionMode ?? null,
      clear_execution_mode: options.clearExecutionMode ?? false,
      permission_level: options.permissionLevel ?? null,
      clear_permission_level: options.clearPermissionLevel ?? false,
      workspace_id: options.workspaceId ?? null,
      clear_workspace_id: options.clearWorkspaceId ?? false,
      worktree_id: options.worktreeId ?? null,
      clear_worktree_id: options.clearWorktreeId ?? false,
    },
  }
}

export function updateAgentProfileRequest(options: {
  sessionId: string
  agentId: string
  provider?: string | null
  accountProfile?: string | null
  model?: string | null
  effort?: string | null
  clearEffort?: boolean
}) {
  return {
    UpdateAgentProfile: {
      session_id: options.sessionId,
      agent_id: options.agentId,
      provider: options.provider ?? null,
      ...(options.accountProfile ? { account_profile: options.accountProfile } : {}),
      model: options.model ?? null,
      effort: options.effort ?? null,
      clear_effort: options.clearEffort ?? false,
    },
  }
}

export type AgentSubstituteAction =
  | { Add: { provider: string; model: string; variant?: string | null; account_profile?: string | null; kernel_id?: string | null; worktree_id?: string | null } }
  | { Remove: { index: number } }
  | { Move: { from_index: number; to_index: number } }
  | { Clear: Record<string, never> }
  | { SetTimeout: { timeout_ms?: number | null } }
  | { Activate: { index: number; reason?: string | null } }
  | { Primary: Record<string, never> }

export function updateAgentSubstitutesRequest(options: {
  sessionId: string
  agentId: string
  action: AgentSubstituteAction
}) {
  return {
    UpdateAgentSubstitutes: {
      session_id: options.sessionId,
      agent_id: options.agentId,
      action: options.action,
    },
  }
}

export function moveAgentToRemoteRequest(sessionId: string, agentRef: string, machineRef: string) {
  return {
    MoveAgentToRemote: {
      session_id: sessionId,
      agent_ref: agentRef,
      machine_ref: machineRef,
    },
  }
}

export function moveAgentToLocalRequest(sessionId: string, agentRef: string) {
  return {
    MoveAgentToLocal: {
      session_id: sessionId,
      agent_ref: agentRef,
    },
  }
}

export function destroyAgentRequest(sessionId: string, agentId: string) {
  return {
    DestroyAgent: {
      session_id: sessionId,
      agent_id: agentId,
    },
  }
}

export function focusAgentRequest(sessionId: string, agentId: string) {
  return {
    FocusAgent: {
      session_id: sessionId,
      agent_id: agentId,
    },
  }
}

export function cycleAgentFocusRequest(sessionId: string) {
  return {
    CycleAgentFocus: {
      session_id: sessionId,
    },
  }
}

export function listAgentsRequest(sessionId: string) {
  return {
    ListAgents: {
      session_id: sessionId,
    },
  }
}
