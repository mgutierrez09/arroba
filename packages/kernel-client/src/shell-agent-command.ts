import type {
  AgentInstance,
  RelayKernelPresence,
  RuntimeProviderRun,
  RuntimeSession,
  SliceRecord,
} from "./kernel-types.js"
import {
  aliasAgentRequest,
  cycleAgentFocusRequest,
  focusAgentRequest,
  getProviderRunRequest,
  getSessionStateRequest,
  launchProviderRunRequest,
  launchProviderRunsRequest,
  listRemoteMachineKernelsRequest,
  listSlicesRequest,
  spawnAgentRequest,
  spawnAgentsRequest,
  submitPromptRequest,
  submitPromptsRequest,
  updateAgentConfigRequest,
  updateAgentProfileRequest,
  updateAgentSubstitutesRequest,
} from "./ipc-requests.js"
import type { ParsedShellCommand, ShellCommandResult, ShellContext } from "./shell-core.js"
import {
  formatAgentInspectSummary,
  formatAgentListSummary,
  formatAgentRef,
  formatAgentSubstituteSummary,
} from "./shell-agent-format.js"
import {
  parseExecutionMode,
  parsePermissionLevel,
} from "./shell-agent-policy.js"
import { resolveShellAgent } from "./shell-agent-resolver.js"
import {
  parsePlacementOptions,
  resolveShellPlacement,
  shellGitWorktreePlacement,
  type ShellPlacementDeps,
} from "./shell-placement.js"
import {
  resolveShellSliceRef,
  shellSliceCreatesPlacement,
} from "./shell-slice-placement.js"
import { remoteKernelReadinessForProvider } from "./shell-remote-format.js"
import { resolveShellAttachmentId } from "./shell-session-attachment.js"

type ShellKernelClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
}

const MAX_SHELL_BATCH_SPAWN_AGENTS = 200
const MAX_SHELL_BATCH_SPAWN_CONCURRENCY = 50
const SHELL_BATCH_SPAWN_CONFIRMATION_THRESHOLD = 50

export type ShellAgentCommandDeps = ShellPlacementDeps & {
  client: ShellKernelClient
}

export async function executeAgentCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellAgentCommandDeps,
): Promise<ShellCommandResult> {
  const sessionId = context.sessionId
  if (!sessionId) {
    return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
  }

  const [action, ...args] = parsed.args
  switch (action) {
    case "list":
    case "ls": {
      const session = await getShellSessionState(deps, sessionId)
      const agents = session.agents
      const { slices } = agents.some((agent) => agent.remote_execution)
        ? await listAgentInspectSlices(deps)
        : { slices: [] }
      const providerRunContext = await activeProviderRunContext(deps, session)
      return {
        ok: true,
        message: formatAgentListSummary(agents, slices, providerRunContext, {
          session,
          homeKernelId: session.host_daemon_id ?? null,
          homeMachineId: session.host_machine_id ?? null,
          ownerUserId: session.owner_user_id ?? null,
          workspaceLiveSyncMode: session.workspace_live_sync_mode ?? null,
          workspaceLiveSyncWorktree: session.worktree_id ?? null,
        }),
        data: { agents, slices, session, providerRunContext },
      }
    }
    case "inspect":
    case "info":
    case "show": {
      const resolved = await resolveShellAgent(context, deps, args[0] ?? context.agentId)
      if (!resolved.ok) {
        return { ok: false, message: args[0] ? resolved.message : "usage: agent inspect [agent-ref]" }
      }
      const session = await getShellSessionState(deps, sessionId)
      const agent = session.agents.find((entry) => entry.id === resolved.agent.id) ?? resolved.agent
      const { slices, error } = resolved.agent.remote_execution
        ? await listAgentInspectSlices(deps)
        : { slices: [], error: null }
      const providerRunContext = await activeProviderRunContext(deps, session)
      return {
        ok: true,
        message: formatAgentInspectSummary(agent, slices, error, providerRunContext, {
          session,
          homeKernelId: session.host_daemon_id ?? null,
          homeMachineId: session.host_machine_id ?? null,
          ownerUserId: session.owner_user_id ?? null,
          workspaceLiveSyncMode: session.workspace_live_sync_mode ?? null,
          workspaceLiveSyncWorktree: session.worktree_id ?? null,
        }),
        data: { agent, slices, session, providerRunContext },
      }
    }
    case "spawn": {
      const metaagent = args.includes("--meta") || args.includes("--metaagent")
      if (metaagent) {
        return { ok: false, message: "creating separate metaagents is deprecated; send /meta <task> to a regular agent to enter meta mode" }
      }
      const controlParse = parseAgentSpawnControlOptions(args.filter((arg) => arg !== "--meta" && arg !== "--metaagent"), parsed.command)
      if (!controlParse.ok) {
        return { ok: false, message: controlParse.message }
      }
      const spawnArgs = controlParse.args
      const spawnCount = controlParse.count
      const parsedSpawn = parsePlacementOptions(spawnArgs, true)
      if (parsedSpawn.error) {
        return { ok: false, message: parsedSpawn.error }
      }
      const [alias, model] = parsedSpawn.options.positional
      if (parsedSpawn.options.positional.length > 2) {
        return { ok: false, message: agentSpawnUsage() }
      }
      if (spawnCount >= SHELL_BATCH_SPAWN_CONFIRMATION_THRESHOLD && !controlParse.confirmLarge) {
        return {
          ok: false,
          message: `spawning ${spawnCount} agents requires confirmation; rerun with --confirm-large`,
        }
      }
      if (spawnCount > 1 && (parsedSpawn.options.gitWorktree || parsedSpawn.options.branch || parsedSpawn.options.fromRef)) {
        return { ok: false, message: "agent spawn --count does not accept --worktree/--branch; use --dir or create worktrees before spawning" }
      }
      if (controlParse.prompt && spawnCount > 1 && parsed.assignment) {
        return { ok: false, message: "agent spawn --prompt with --count cannot bind one agent id assignment; omit `as <name>`" }
      }
      const resolvedMachineKernel = await resolveMachineSpawnKernelRef(parsedSpawn.options.machineRef, context.provider, deps)
      if (!resolvedMachineKernel.ok) {
        return { ok: false, message: resolvedMachineKernel.message }
      }
      const remoteKernelRef = parsedSpawn.options.kernelRef ?? resolvedMachineKernel.kernelRef
      if (
        parsedSpawn.options.sliceRef
        && !shellSliceCreatesPlacement(parsedSpawn.options.sliceRef)
        && parsedSpawn.options.sliceRef !== "off"
        && (parsedSpawn.options.directory || parsedSpawn.options.gitWorktree || parsedSpawn.options.branch || parsedSpawn.options.fromRef)
      ) {
        return { ok: false, message: "usage: agent spawn [alias] [model] --slice <slice-ref> does not accept --dir or --worktree" }
      }
      const worktree = await resolveShellPlacement(parsedSpawn.options, context.worktree, "agent working directory", deps, false)
      const worktreePlacement = shellGitWorktreePlacement(parsedSpawn.options)
      const effectiveWorktree = worktree ?? context.worktree
      const sliceRef = await resolveShellSliceRef(
        parsedSpawn.options.sliceRef,
        context,
        effectiveWorktree,
        deps,
        parsedSpawn.options.sliceDisplayMode,
        remoteKernelRef,
      )
      if (spawnCount > 1) {
        const provider = controlParse.provider ?? context.provider
        const effectiveModel = controlParse.model ?? model ?? context.model
        const effort = controlParse.effort ?? context.effort
        const response = await deps.client.send(spawnAgentsRequest(
          sessionId,
          Array.from({ length: spawnCount }, (_, index) => ({
            provider,
            alias: batchAgentAlias(alias, index),
            model: effectiveModel,
            worktreeId: worktree ?? null,
            effort,
            kernelRef: sliceRef ? null : remoteKernelRef ?? null,
            sliceRef: sliceRef ?? null,
          })),
        ))
        const agents = expectVariant<{ agents: AgentInstance[] }>(response, "AgentsSpawned").agents
        const promptSummary = controlParse.prompt
          ? await launchAndPromptAgents({
            sessionId,
            agents,
            provider,
            model: effectiveModel,
            effort,
            prompt: controlParse.prompt,
            concurrency: controlParse.concurrency,
          }, context, deps)
          : null
        const first = agents[0]
        const last = agents.at(-1) ?? first
        const promptMessage = promptSummary ? formatBatchPromptSummary(promptSummary) : null
        const result = resourceResult(
          [
            `spawned ${agents.length} agents${first?.agent_ref && last?.agent_ref ? ` (${first.agent_ref}..${last.agent_ref})` : ""}`,
            promptMessage,
          ].filter(Boolean).join("; "),
          parsed.assignment,
          last?.id ?? "",
          last ? { agentId: last.id } : {},
          { agents, promptSummary },
        )
        return promptSummary?.failed ? { ...result, ok: false } : result
      }
      const response = await deps.client.send(spawnAgentRequest(
        sessionId,
        controlParse.provider ?? context.provider,
        alias,
        controlParse.model ?? model ?? context.model,
        worktree,
        controlParse.effort ?? context.effort,
        undefined,
        undefined,
        sliceRef ? undefined : remoteKernelRef,
        worktreePlacement,
        sliceRef,
      ))
      const agent = expectVariant<{ agent: AgentInstance }>(response, "AgentSpawned").agent
      const promptSummary = controlParse.prompt
        ? await launchAndPromptAgents({
          sessionId,
          agents: [agent],
          provider: controlParse.provider ?? agent.provider ?? context.provider,
          model: controlParse.model ?? agent.model ?? model ?? context.model,
          effort: controlParse.effort ?? agent.effort ?? context.effort,
          prompt: controlParse.prompt,
          concurrency: 1,
        }, context, deps)
        : null
      const placement = agent.remote_execution
        ? sliceRef
          ? ` in slice ${sliceRef}`
          : ` on ${remoteKernelRef ?? agent.remote_execution.worker_machine_id}`
        : agent.worktree_id ? ` in ${agent.worktree_id}` : ""
      return resourceResult(
        `spawned agent ${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}${placement}${promptSummary ? "; prompted agent" : ""}`,
        parsed.assignment,
        agent.id,
        { agentId: agent.id },
        { agent, promptSummary },
      )
    }
    case "focus": {
      const agentRef = args[0]
      if (!agentRef) {
        return { ok: false, message: "usage: agent focus <agent-id>" }
      }
      const response = await deps.client.send(focusAgentRequest(sessionId, agentRef))
      const agent = expectVariant<{ agent: AgentInstance }>(response, "AgentFocused").agent
      return resourceResult(
        `current agent = ${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}`,
        parsed.assignment,
        agent.id,
        { agentId: agent.id },
        { agent },
      )
    }
    case "cycle": {
      const response = await deps.client.send(cycleAgentFocusRequest(sessionId))
      const agent = expectVariant<{ agent: AgentInstance | null }>(response, "AgentFocusCycled").agent
      if (!agent) {
        return { ok: true, message: "no agents to cycle" }
      }
      return resourceResult(
        `current agent = ${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}`,
        parsed.assignment,
        agent.id,
        { agentId: agent.id },
        { agent },
      )
    }
    case "alias":
    case "name": {
      const reference = args.length > 1 ? args[0] : context.agentId
      const rawAlias = (args.length > 1 ? args.slice(1) : args).join(" ").trim()
      const resolved = await resolveShellAgent(context, deps, reference)
      if (!resolved.ok) {
        return { ok: false, message: args[0] ? resolved.message : "usage: agent alias [agent-ref] <alias|clear>" }
      }
      if (!rawAlias) {
        return { ok: true, message: `${formatAgentRef(resolved.agent)} alias = ${resolved.agent.alias ?? "<none>"}`, data: { agent: resolved.agent } }
      }
      const shouldClearAgentAlias = rawAlias === "clear" || rawAlias === "none" || rawAlias === "-"
      const response = await deps.client.send(aliasAgentRequest(
        sessionId,
        resolved.agent.id,
        shouldClearAgentAlias ? "" : rawAlias,
      ))
      const payload = expectVariant<{ agent: AgentInstance; session: RuntimeSession }>(response, "AgentAliased")
      return { ok: true, message: `${formatAgentRef(payload.agent)} alias = ${payload.agent.alias ?? "<none>"}`, data: payload }
    }
    case "provider":
    case "model":
    case "variant": {
      const resolved = await resolveShellAgent(context, deps, args.length > 1 ? args[0] : context.agentId)
      const rawValue = args.length > 1 ? args.slice(1).join(" ").trim() : args.join(" ").trim()
      if (!resolved.ok) {
        return { ok: false, message: args[0] ? resolved.message : `usage: agent ${action} [agent-ref] <value>` }
      }
      if (!rawValue) {
        const response = await deps.client.send(getSessionStateRequest(sessionId))
        const session = expectVariant<{ session: RuntimeSession }>(response, "SessionState").session
        const agent = session.agents.find((entry) => entry.id === resolved.agent.id) ?? resolved.agent
        const value = action === "provider"
          ? agent.provider
          : action === "model"
            ? agent.model ?? "<none>"
            : agent.effort ?? "<none>"
        return { ok: true, message: `${formatAgentRef(agent)} ${action} = ${value}`, data: { session, agent } }
      }
      const shouldClearEffort = action === "variant" && ["clear", "none", "-", "default"].includes(rawValue)
      const response = await deps.client.send(updateAgentProfileRequest({
        sessionId,
        agentId: resolved.agent.id,
        ...(action === "provider" ? { provider: rawValue } : {}),
        ...(action === "model" ? { model: rawValue } : {}),
        ...(action === "variant" && !shouldClearEffort ? { effort: rawValue } : {}),
        ...(shouldClearEffort ? { clearEffort: true } : {}),
      }))
      const payload = expectVariant<{ agent: AgentInstance; session: RuntimeSession }>(response, "AgentProfileUpdated")
      const value = action === "provider"
        ? payload.agent.provider
        : action === "model"
          ? payload.agent.model ?? "<none>"
          : payload.agent.effort ?? "<none>"
      return { ok: true, message: `${formatAgentRef(payload.agent)} ${action} = ${value}`, data: payload }
    }
    case "mode": {
      const firstArgIsMode = args[0] === "inherit" || parseExecutionMode(args[0]) != null
      const resolved = await resolveShellAgent(context, deps, firstArgIsMode ? context.agentId : args[0])
      if (!resolved.ok) {
        return { ok: false, message: args[0] ? resolved.message : "usage: agent mode [agent-ref] <build|plan|inherit>" }
      }
      const rawValue = args.length > 1 ? args[1] : firstArgIsMode ? args[0] : undefined
      if (!rawValue) {
        const response = await deps.client.send(getSessionStateRequest(sessionId))
        const session = expectVariant<{ session: RuntimeSession }>(response, "SessionState").session
        const agent = session.agents.find((entry) => entry.id === resolved.agent.id) ?? resolved.agent
        const sessionMode = parseExecutionMode(session.config_state?.values?.["agents.mode"]) ?? "build"
        const effectiveMode = agent.execution_mode_override ?? sessionMode
        const source = agent.execution_mode_override ? "agent" : "session"
        return { ok: true, message: `${formatAgentRef(agent)} mode = ${effectiveMode} (${source})`, data: { session, agent } }
      }
      if (rawValue !== "inherit" && !parseExecutionMode(rawValue)) {
        return { ok: false, message: "usage: agent mode [agent-ref] <build|plan|inherit>" }
      }
      const response = await deps.client.send(updateAgentConfigRequest({
        sessionId,
        agentId: resolved.agent.id,
        executionMode: rawValue === "inherit" ? null : parseExecutionMode(rawValue),
        clearExecutionMode: rawValue === "inherit",
      }))
      const payload = expectVariant<{ agent: AgentInstance; session: RuntimeSession }>(response, "AgentConfigUpdated")
      const sessionMode = parseExecutionMode(payload.session.config_state?.values?.["agents.mode"]) ?? "build"
      const effectiveMode = payload.agent.execution_mode_override ?? sessionMode
      return { ok: true, message: `${formatAgentRef(payload.agent)} mode = ${effectiveMode}${rawValue === "inherit" ? " (session)" : " (agent)"}`, data: payload }
    }
    case "permissions": {
      const firstArgIsPermission = args[0] === "inherit" || parsePermissionLevel(args[0]) != null
      const resolved = await resolveShellAgent(context, deps, firstArgIsPermission ? context.agentId : args[0])
      if (!resolved.ok) {
        return { ok: false, message: args[0] ? resolved.message : "usage: agent permissions [agent-ref] <required|yolo|inherit>" }
      }
      const rawValue = args.length > 1 ? args[1] : firstArgIsPermission ? args[0] : undefined
      if (!rawValue) {
        const response = await deps.client.send(getSessionStateRequest(sessionId))
        const session = expectVariant<{ session: RuntimeSession }>(response, "SessionState").session
        const agent = session.agents.find((entry) => entry.id === resolved.agent.id) ?? resolved.agent
        const sessionLevel = parsePermissionLevel(session.config_state?.values?.["agents.permissions"]) ?? "yolo"
        const effectiveLevel = agent.permission_level_override ?? sessionLevel
        const source = agent.permission_level_override ? "agent" : "session"
        return { ok: true, message: `${formatAgentRef(agent)} permissions = ${effectiveLevel} (${source})`, data: { session, agent } }
      }
      if (rawValue !== "inherit" && !parsePermissionLevel(rawValue)) {
        return { ok: false, message: "usage: agent permissions [agent-ref] <required|yolo|inherit>" }
      }
      const response = await deps.client.send(updateAgentConfigRequest({
        sessionId,
        agentId: resolved.agent.id,
        permissionLevel: rawValue === "inherit" ? null : parsePermissionLevel(rawValue),
        clearPermissionLevel: rawValue === "inherit",
      }))
      const payload = expectVariant<{ agent: AgentInstance; session: RuntimeSession }>(response, "AgentConfigUpdated")
      const sessionLevel = parsePermissionLevel(payload.session.config_state?.values?.["agents.permissions"]) ?? "yolo"
      const effectiveLevel = payload.agent.permission_level_override ?? sessionLevel
      return { ok: true, message: `${formatAgentRef(payload.agent)} permissions = ${effectiveLevel}${rawValue === "inherit" ? " (session)" : " (agent)"}`, data: payload }
    }
    case "substitute":
    case "subs":
      return executeAgentSubstituteCommand(args, context, deps, sessionId)
    default:
      return { ok: false, message: "usage: agent list|inspect|spawn|focus|cycle|alias|provider|model|variant|mode|permissions|substitute" }
  }
}

function batchAgentAlias(alias: string | undefined, index: number): string | null {
  if (!alias) {
    return null
  }
  return index === 0 ? alias : `${alias}-${index + 1}`
}

async function resolveMachineSpawnKernelRef(
  machineRef: string | undefined,
  provider: string,
  deps: ShellAgentCommandDeps,
): Promise<{ readonly ok: true; readonly kernelRef?: string } | { readonly ok: false; readonly message: string }> {
  if (!machineRef) {
    return { ok: true }
  }
  const response = await deps.client.send(listRemoteMachineKernelsRequest(machineRef))
  const kernels = expectVariant<{ kernels: RelayKernelPresence[] }>(response, "RemoteMachineKernelsListed").kernels
  if (kernels.length === 0) {
    return { ok: false, message: `remote machine ${machineRef} has no live worker kernels; next: run /machine kernels ${machineRef}; reconnect that machine or choose another worker` }
  }
  const leaseReady = kernels.filter((kernel) => remoteKernelAcceptsRemoteLeases(kernel))
  if (leaseReady.length === 0) {
    return { ok: false, message: `remote machine ${machineRef} has no ready worker kernel; next: run /machine kernels ${machineRef}; fix the listed readiness/account issue or choose another worker` }
  }
  const providerCandidates = leaseReady.filter((kernel) => (kernel.available_providers ?? []).includes(provider))
  if (providerCandidates.length === 0) {
    return { ok: false, message: `remote machine ${machineRef} has no accepting kernel with provider ${provider}; next: run /machine kernels ${machineRef}; choose a ready worker with ${provider}, configure/import its provider account, or change the agent provider` }
  }
  const providerReady = providerCandidates.find((kernel) => (
    remoteKernelReadinessForProvider(kernel, provider) === "ready"
  ))
  if (!providerReady) {
    return { ok: false, message: `remote machine ${machineRef} has no ready worker kernel with a usable ${provider} account; next: run /machine kernels ${machineRef}; configure/import or refresh the ${provider} account, or choose another worker` }
  }
  return { ok: true, kernelRef: providerReady.kernel_id }
}

function remoteKernelAcceptsRemoteLeases(kernel: RelayKernelPresence): boolean {
  return kernel.accepting_remote_leases === true
}

async function listAgentInspectSlices(deps: ShellAgentCommandDeps): Promise<{
  slices: SliceRecord[]
  error: string | null
}> {
  try {
    const response = await deps.client.send(listSlicesRequest())
    return {
      slices: expectVariant<{ slices: SliceRecord[] }>(response, "SlicesListed").slices,
      error: null,
    }
  } catch (error) {
    return {
      slices: [],
      error: error instanceof Error ? error.message : "slice inventory unavailable",
    }
  }
}

async function getShellSessionState(
  deps: ShellAgentCommandDeps,
  sessionId: string,
): Promise<RuntimeSession> {
  const response = await deps.client.send(getSessionStateRequest(sessionId))
  return expectVariant<{ session: RuntimeSession }>(response, "SessionState").session
}

async function activeProviderRunContext(
  deps: ShellAgentCommandDeps,
  session: RuntimeSession,
): Promise<{
  activeProviderRunId?: string | null
  activeProviderRunAgentId?: string | null
  activeProviderRunLookupError?: string | null
}> {
  if (!session.active_provider_run_id) {
    return {}
  }
  try {
    const response = await deps.client.send(getProviderRunRequest(session.active_provider_run_id))
    const providerRun = expectVariant<{ provider_run: RuntimeProviderRun }>(response, "ProviderRun").provider_run
    return {
      activeProviderRunId: providerRun.id,
      activeProviderRunAgentId: providerRun.agent_instance_id ?? null,
    }
  } catch (error) {
    return {
      activeProviderRunId: session.active_provider_run_id,
      activeProviderRunAgentId: null,
      activeProviderRunLookupError: error instanceof Error ? error.message : "provider run lookup failed",
    }
  }
}

async function executeAgentSubstituteCommand(
  args: string[],
  context: ShellContext,
  deps: ShellAgentCommandDeps,
  sessionId: string,
): Promise<ShellCommandResult> {
  const [subcommand = "list", ...rawArgs] = args
  const agentFlagIndex = rawArgs.indexOf("--agent")
  const agentRefFromFlag = agentFlagIndex >= 0 ? rawArgs[agentFlagIndex + 1] : undefined
  const filteredArgs = agentFlagIndex >= 0
    ? rawArgs.filter((_, index) => index !== agentFlagIndex && index !== agentFlagIndex + 1)
    : rawArgs
  const resolved = await resolveShellAgent(context, deps, agentRefFromFlag)
  if (!resolved.ok) {
    return { ok: false, message: resolved.message }
  }
  const agent = resolved.agent
  if (subcommand === "list" || subcommand === "ls") {
    return { ok: true, message: formatAgentSubstituteSummary(agent), data: { agent } }
  }
  const update = async (action: Record<string, unknown>) => {
    const response = await deps.client.send(updateAgentSubstitutesRequest({
      sessionId,
      agentId: agent.id,
      action: action as never,
    }))
    return expectVariant<{ agent: AgentInstance; session: RuntimeSession }>(response, "AgentConfigUpdated")
  }
  if (subcommand === "add") {
    const provider = filteredArgs[0]
    const model = filteredArgs[1]
    const variantIndex = filteredArgs.indexOf("--variant")
    const variant = variantIndex >= 0 ? filteredArgs[variantIndex + 1] : undefined
    const kernelIndex = filteredArgs.indexOf("--kernel")
    const kernelId = kernelIndex >= 0 ? filteredArgs[kernelIndex + 1] : undefined
    const worktreeIndex = filteredArgs.indexOf("--worktree")
    const worktreeId = worktreeIndex >= 0 ? filteredArgs[worktreeIndex + 1] : undefined
    if (!provider || !model) {
      return { ok: false, message: "usage: agent substitute add <provider> <model> [--variant v] [--kernel k] [--worktree dir] [--agent a]" }
    }
    const payload = await update({
      Add: {
        provider,
        model,
        variant: variant ?? null,
        kernel_id: kernelId ?? null,
        worktree_id: worktreeId ?? null,
      },
    })
    return { ok: true, message: `${formatAgentRef(payload.agent)} substitute added: ${provider}/${model}${variant ? `/${variant}` : ""}`, data: payload }
  }
  if (subcommand === "remove" || subcommand === "rm") {
    const index = Number.parseInt(filteredArgs[0] ?? "", 10)
    if (!Number.isFinite(index)) {
      return { ok: false, message: "usage: agent substitute remove <index> [--agent a]" }
    }
    const payload = await update({ Remove: { index } })
    return { ok: true, message: `${formatAgentRef(payload.agent)} substitute ${index} removed`, data: payload }
  }
  if (subcommand === "move") {
    const fromIndex = Number.parseInt(filteredArgs[0] ?? "", 10)
    const toIndex = Number.parseInt(filteredArgs[1] ?? "", 10)
    if (!Number.isInteger(fromIndex) || !Number.isInteger(toIndex)) {
      return { ok: false, message: "usage: agent substitute move <from-index> <to-index> [--agent a]" }
    }
    const payload = await update({ Move: { from_index: fromIndex, to_index: toIndex } })
    return { ok: true, message: `${formatAgentRef(payload.agent)} substitute moved from ${fromIndex} to ${toIndex}`, data: payload }
  }
  if (subcommand === "clear") {
    const payload = await update({ Clear: {} })
    return { ok: true, message: `${formatAgentRef(payload.agent)} substitutes cleared`, data: payload }
  }
  if (subcommand === "timeout") {
    const timeoutMs = parseSubstitutionTimeoutMs(filteredArgs[0])
    if (timeoutMs === undefined && filteredArgs[0] !== "inherit" && filteredArgs[0] !== "default") {
      return { ok: false, message: "usage: agent substitute timeout <ms|Ns|inherit> [--agent a]" }
    }
    const payload = await update({ SetTimeout: { timeout_ms: timeoutMs ?? null } })
    return { ok: true, message: `${formatAgentRef(payload.agent)} substitute timeout: ${timeoutMs == null ? "default" : `${timeoutMs}ms`}`, data: payload }
  }
  if (subcommand === "activate") {
    const index = Number.parseInt(filteredArgs[0] ?? "", 10)
    if (!Number.isFinite(index)) {
      return { ok: false, message: "usage: agent substitute activate <index> [--agent a]" }
    }
    const payload = await update({ Activate: { index, reason: "manual" } })
    const profile = payload.agent.substitutes?.[index]
    if (!profile) {
      return { ok: false, message: `${formatAgentRef(payload.agent)} substitute ${index} is not available`, data: payload }
    }
    const response = await deps.client.send(launchProviderRunRequest(
      sessionId,
      profile.provider,
      "default",
      profile.model,
      profile.variant ?? "",
      payload.agent.id,
    ))
    return { ok: true, message: `${formatAgentRef(payload.agent)} activated substitute ${index}: ${profile.provider}/${profile.model}`, data: { ...payload, launch: response }, contextUpdates: { agentId: payload.agent.id } }
  }
  if (subcommand === "primary" || subcommand === "reset") {
    const payload = await update({ Primary: {} })
    const response = await deps.client.send(launchProviderRunRequest(
      sessionId,
      payload.agent.provider,
      "default",
      payload.agent.model ?? context.model,
      payload.agent.effort ?? context.effort,
      payload.agent.id,
    ))
    return { ok: true, message: `${formatAgentRef(payload.agent)} reset to starter profile`, data: { ...payload, launch: response }, contextUpdates: { agentId: payload.agent.id } }
  }
  return { ok: false, message: "usage: agent substitute list|add|remove|move|clear|timeout|activate|reset" }
}

function resourceResult(
  message: string,
  assignment: string | undefined,
  value: string,
  contextUpdates: ShellCommandResult["contextUpdates"],
  data: unknown,
): ShellCommandResult {
  return {
    ok: true,
    message,
    data,
    bindings: assignment ? { [assignment]: value } : undefined,
    contextUpdates,
  }
}

function agentSpawnUsage(): string {
  return "usage: agents spawn <count> [alias] [model] [--provider <provider>] [--prompt <text>] [--concurrency <n>] [--confirm-large] [--dir <directory>] [--machine <machine-ref>|--kernel <kernel-ref>] [--slice off|new:headless|new:headed|<slice-ref>]"
}

type AgentSpawnControlOptions = {
  readonly args: string[]
  readonly count: number
  readonly provider: string | undefined
  readonly model: string | undefined
  readonly effort: string | undefined
  readonly prompt: string | undefined
  readonly concurrency: number
  readonly confirmLarge: boolean
}

function parseAgentSpawnControlOptions(
  args: string[],
  command: string | undefined,
): { ok: true } & AgentSpawnControlOptions | { ok: false; message: string } {
  const stripped: string[] = []
  let count = 1
  let provider: string | undefined
  let model: string | undefined
  let effort: string | undefined
  let prompt: string | undefined
  let concurrency = 10
  let confirmLarge = false
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]
    if (!arg) {
      continue
    }
    const next = args[index + 1]
    if (arg === "--count" || arg === "-n") {
      const parsed = parseSpawnCount(next)
      if (!parsed.ok) return parsed
      count = parsed.count
      index += 1
    } else if (arg === "--provider" && next) {
      provider = next
      index += 1
    } else if (arg === "--model" && next) {
      model = next
      index += 1
    } else if ((arg === "--effort" || arg === "--variant") && next) {
      effort = next
      index += 1
    } else if (arg === "--prompt" && next) {
      prompt = next
      index += 1
    } else if (arg === "--concurrency" && next) {
      const parsed = Number.parseInt(next, 10)
      if (!Number.isFinite(parsed) || parsed < 1 || parsed > MAX_SHELL_BATCH_SPAWN_CONCURRENCY) {
        return { ok: false, message: `--concurrency must be between 1 and ${MAX_SHELL_BATCH_SPAWN_CONCURRENCY}` }
      }
      concurrency = parsed
      index += 1
    } else if (arg === "--confirm-large" || arg === "--yes" || arg === "-y") {
      confirmLarge = true
    } else {
      stripped.push(arg)
    }
  }
  const first = stripped[0]
  if ((command === "agents" || count === 1) && first && /^\d+$/.test(first)) {
    const parsed = parseSpawnCount(first)
    if (!parsed.ok) return parsed
    count = parsed.count
    stripped.shift()
  }
  if (concurrency > count) {
    concurrency = count
  }
  return { ok: true, args: stripped, count, provider, model, effort, prompt, concurrency, confirmLarge }
}

function parseSpawnCount(value: string | undefined): { ok: true; count: number } | { ok: false; message: string } {
  const parsed = Number.parseInt(value ?? "", 10)
  if (!Number.isFinite(parsed) || parsed < 1 || parsed > MAX_SHELL_BATCH_SPAWN_AGENTS) {
    return { ok: false, message: `--count must be between 1 and ${MAX_SHELL_BATCH_SPAWN_AGENTS}` }
  }
  return { ok: true, count: parsed }
}

async function launchAndPromptAgents(
  input: {
    readonly sessionId: string
    readonly agents: readonly AgentInstance[]
    readonly provider: string
    readonly model: string
    readonly effort: string
    readonly prompt: string
    readonly concurrency: number
  },
  context: ShellContext,
  deps: ShellAgentCommandDeps,
): Promise<BatchPromptSummary> {
  const attachment = await resolveShellAttachmentId(context, deps)
  if (!attachment.ok) {
    throw new Error(attachment.message)
  }
  const promptText = input.prompt.endsWith("\n") ? input.prompt : `${input.prompt}\n`
  let prompted = 0
  const failures: BatchPromptFailure[] = []
  const agentById = new Map(input.agents.map((agent) => [agent.id, agent]))
  const launchBatch = parseBatchLaunchResponse(await deps.client.send(launchProviderRunsRequest(
    input.agents.map((agent) => ({
      sessionId: input.sessionId,
      provider: input.provider,
      accountProfile: "default",
      model: input.model,
      effort: input.effort,
      agentId: agent.id,
    })),
    input.concurrency,
  )))
  const launchFailedIndexes = new Set<number>()
  for (const failure of launchBatch.failures) {
    launchFailedIndexes.add(failure.index)
    const agent = failure.agent_id ? agentById.get(failure.agent_id) : input.agents[failure.index]
    failures.push({
      agentRef: agent?.agent_ref ?? agent?.id ?? failure.agent_id ?? `#${failure.index + 1}`,
      message: failure.message,
    })
  }
  const promptAgents = input.agents.filter((_, index) => !launchFailedIndexes.has(index))
  if (promptAgents.length > 0) {
    const promptBatch = parseBatchPromptResponse(await deps.client.send(submitPromptsRequest(
      input.sessionId,
      attachment.attachmentId,
      promptAgents.map((agent) => ({
        targetAgentId: agent.id,
        prompt: promptText,
        attachments: [],
      })),
      input.concurrency,
    )))
    prompted += promptBatch.results.length
    for (const failure of promptBatch.failures) {
      const agent = failure.agent_id ? agentById.get(failure.agent_id) : promptAgents[failure.index]
      failures.push({
        agentRef: agent?.agent_ref ?? agent?.id ?? failure.agent_id ?? `#${failure.index + 1}`,
        message: failure.message,
      })
    }
  }
  return { prompted, failed: failures.length, concurrency: input.concurrency, failures: failures.slice(0, 3) }
}

type BatchFailurePayload = {
  readonly index: number
  readonly agent_id?: string | null
  readonly message: string
}

function parseBatchLaunchResponse(response: Record<string, unknown>): {
  readonly failures: readonly BatchFailurePayload[]
} {
  if (!("ProviderRunsLaunchAccepted" in response)) {
    throw new Error(`unexpected batch launch response ${JSON.stringify(response)}`)
  }
  const payload = response.ProviderRunsLaunchAccepted as {
    failures?: readonly BatchFailurePayload[]
  }
  return { failures: payload.failures ?? [] }
}

function parseBatchPromptResponse(response: Record<string, unknown>): {
  readonly results: readonly unknown[]
  readonly failures: readonly BatchFailurePayload[]
} {
  if (!("PromptsSubmitted" in response)) {
    throw new Error(`unexpected batch prompt response ${JSON.stringify(response)}`)
  }
  const payload = response.PromptsSubmitted as {
    results?: readonly unknown[]
    failures?: readonly BatchFailurePayload[]
  }
  return { results: payload.results ?? [], failures: payload.failures ?? [] }
}

type BatchPromptFailure = {
  readonly agentRef: string
  readonly message: string
}

type BatchPromptSummary = {
  readonly prompted: number
  readonly failed: number
  readonly concurrency: number
  readonly failures: readonly BatchPromptFailure[]
}

function formatBatchPromptSummary(summary: BatchPromptSummary): string {
  if (summary.failed === 0) {
    return `prompted ${summary.prompted} agents with concurrency ${summary.concurrency}`
  }
  const examples = summary.failures
    .map((failure) => `${failure.agentRef}: ${failure.message}`)
    .join("; ")
  const overflow = summary.failed > summary.failures.length ? `; +${summary.failed - summary.failures.length} more` : ""
  return `prompted ${summary.prompted} agents with concurrency ${summary.concurrency}; failed to prompt ${summary.failed} agents (${examples}${overflow})`
}

function parseSubstitutionTimeoutMs(value: string | null | undefined): number | undefined {
  if (!value || value === "inherit" || value === "default") return undefined
  const normalized = value.trim().toLowerCase()
  const match = normalized.match(/^(\d+)(ms|s|m)?$/)
  if (!match) return undefined
  const amount = Number.parseInt(match[1] ?? "", 10)
  const unit = match[2] ?? "ms"
  if (!Number.isFinite(amount)) return undefined
  if (unit === "m") return amount * 60_000
  if (unit === "s") return amount * 1_000
  return amount
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
