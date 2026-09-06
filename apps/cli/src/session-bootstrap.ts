import type { CharioxLogger } from "./logging.js"
import type { CharioxPreferences } from "./preferences.js"
import type { TerminalCommandCatalog } from "@chariox/kernel-client/kernel-types"
import { extractPromptInputHistoryEntries } from "@chariox/kernel-client/prompt-history"
import { fallbackProviderCatalog, type ProviderCatalog } from "./provider-catalog.js"
import { fallbackProviderCommandCatalogs, type ProviderCommandCatalogs } from "./provider-command-catalog.js"
import { selectAttachableSession, decideBootstrapAction } from "./sessions.js"
import { settleAttachProviderRun } from "./attach-provider-run.js"
import { sessionHistoryCursorForVisibleAgent } from "@chariox/kernel-client/session-history-outline"

import type {
  CliOptions,
  BootstrapState,
  PromptInputHistoryEntry,
  RuntimeAttachment,
  RuntimeProviderRun,
  RuntimeSession,
  SessionHistoryOutline,
  SessionHistoryOutlineAgent,
  TranscriptEntry,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"

type BootstrapDeps = {
  logger?: CharioxLogger | null
  getConfiguredProviderLaunchDefaults?: (client: LocalIpcClient) => Promise<{
    provider?: string
    model?: string
    effort?: string
  }>
  listSessions: (client: LocalIpcClient) => Promise<RuntimeSession[]>
  getProviderCatalog: (client: LocalIpcClient, logger?: CharioxLogger | null) => Promise<ProviderCatalog>
  getProviderCommandCatalogs: (client: LocalIpcClient, logger?: CharioxLogger | null) => Promise<ProviderCommandCatalogs>
  getTerminalCommandCatalog: (client: LocalIpcClient, logger?: CharioxLogger | null) => Promise<TerminalCommandCatalog>
  createSession: (
    client: LocalIpcClient,
    workspace: string,
    worktree: string,
    alias?: string,
    agentDefaults?: RuntimeSession["agent_defaults"],
  ) => Promise<RuntimeSession>
  resolveSession: (client: LocalIpcClient, sessionRef: string, workspace: string) => Promise<RuntimeSession>
  attachToSession: (client: LocalIpcClient, sessionId: string, clientId: string) => Promise<RuntimeAttachment>
  getSessionState: (client: LocalIpcClient, sessionId: string) => Promise<RuntimeSession>
  launchProviderRun: (
    client: LocalIpcClient,
    sessionId: string,
    provider: string,
    accountProfile: string,
    model: string,
    effort: string,
    agentId?: string | null,
  ) => Promise<RuntimeProviderRun>
  tryGetProviderRun: (
    client: LocalIpcClient,
    providerRunId: string,
    logger?: CharioxLogger | null,
  ) => Promise<RuntimeProviderRun | null>
  catchUpAttachedSession: (
    client: LocalIpcClient,
    sessionId: string,
    attachmentId: string,
    session: RuntimeSession,
    logger?: CharioxLogger | null,
  ) => Promise<void>
  getSessionHistoryOutline: (
    client: LocalIpcClient,
    sessionId: string,
    agentIds: readonly string[],
  ) => Promise<SessionHistoryOutline>
  getPromptInputHistory?: (
    client: LocalIpcClient,
    sessionId: string,
  ) => Promise<{ entries: PromptInputHistoryEntry[] }>
  resolveVisibleAgentId: (session: RuntimeSession, preferences: CharioxPreferences) => string | null
  prepareHistoryOutlineAgent: (agent: SessionHistoryOutlineAgent, session: RuntimeSession) => TranscriptEntry[]
}

export async function bootstrapSession(
  client: LocalIpcClient,
  options: CliOptions,
  workspace: string,
  worktree: string,
  preferences: CharioxPreferences,
  deps: BootstrapDeps,
): Promise<BootstrapState> {
  let createdSession = false
  let session: RuntimeSession | null = null

  if (!options.provider || options.model === "default" || !options.effort.trim()) {
    const configured = await deps.getConfiguredProviderLaunchDefaults?.(client) ?? {}
    if (!options.provider && configured.provider) {
      options.provider = configured.provider
    }
    if (options.model === "default" && configured.model) {
      options.model = configured.model
    }
    if (!options.effort.trim() && configured.effort) {
      options.effort = configured.effort
    }
  }

  const sessions = await deps.listSessions(client)
  const decision = decideBootstrapAction(options, sessions, workspace, worktree)
  const requestedAgentDefaults = options.provider
    ? {
        provider: options.provider,
        model: options.model,
        effort: options.effort,
        account_profile: options.accountProfile,
      }
    : undefined
  switch (decision.action) {
    case "create":
      session = await deps.createSession(client, workspace, worktree, options.alias, requestedAgentDefaults)
      createdSession = true
      break
    case "resolve":
      session = await deps.resolveSession(client, decision.sessionRef, workspace)
      break
    case "attach_existing": {
      const existing = selectAttachableSession(sessions, workspace, worktree)
      if (!existing) {
        session = await deps.createSession(client, workspace, worktree, options.alias, requestedAgentDefaults)
        createdSession = true
        break
      }
      session = existing as RuntimeSession
      break
    }
    case "none": {
      const [providerCatalog, providerCommandCatalogs, terminalCommandCatalog] = await Promise.all([
        deps.getProviderCatalog(client, deps.logger),
        deps.getProviderCommandCatalogs(client, deps.logger),
        deps.getTerminalCommandCatalog(client, deps.logger),
      ])
      return {
        client,
        binding: null,
        sessions,
        providerCatalog,
        providerCommandCatalogs,
        terminalCommandCatalog,
        options,
        preferences,
      }
    }
  }

  if (!session) {
    const [providerCatalog, providerCommandCatalogs, terminalCommandCatalog] = await Promise.all([
      deps.getProviderCatalog(client, deps.logger),
      deps.getProviderCommandCatalogs(client, deps.logger),
      deps.getTerminalCommandCatalog(client, deps.logger),
    ])
    return {
      client,
      binding: null,
      sessions,
      providerCatalog,
      providerCommandCatalogs,
      terminalCommandCatalog,
      options,
      preferences,
    }
  }

  const attachment = await deps.attachToSession(client, session.id, options.clientId)
  let attachedSession = await deps.getSessionState(client, session.id)
  const providerSettlement = await settleAttachProviderRun(
    attachedSession,
    {
      provider: options.provider ?? "opencode",
      model: options.model,
      effort: options.effort,
    },
    options.accountProfile,
    createdSession,
    {
      launchProviderRun: (sessionId, provider, accountProfile, model, effort, targetAgentId) =>
        deps.launchProviderRun(client, sessionId, provider, accountProfile, model, effort, targetAgentId),
      getSessionState: (sessionId) => deps.getSessionState(client, sessionId),
      tryGetProviderRun: (providerRunId) => deps.tryGetProviderRun(client, providerRunId, deps.logger),
    },
  )
  attachedSession = providerSettlement.session
  const providerRun: RuntimeProviderRun | null = providerSettlement.providerRun
  if (providerSettlement.action === "skipped") {
    if (providerSettlement.recoveredRemotePlacement) {
      deps.logger?.info?.("recovered attach-time provider launch after agent moved remote", {
        session_id: session.id,
        agent_id: providerSettlement.targetAgent?.id ?? null,
        worker_kernel_id: providerSettlement.targetAgent?.remote_execution?.worker_kernel_id ?? null,
      })
    } else if (providerSettlement.reason === "no_visible_agents") {
      deps.logger?.warn("skipping provider launch because no agents are visible to this client", {
        session_id: session.id,
        focused_agent_id: attachedSession.focused_agent_id,
      })
    } else if (providerSettlement.reason === "missing_focused_agent") {
      deps.logger?.warn("skipping provider launch because focused agent is not visible to this client", {
        session_id: session.id,
        focused_agent_id: attachedSession.focused_agent_id,
      })
    } else if (providerSettlement.reason === "remote_backed_agent") {
      deps.logger?.info?.("skipping attach-time provider launch for remote-backed agent", {
        session_id: session.id,
        agent_id: providerSettlement.targetAgent?.id ?? null,
        worker_kernel_id: providerSettlement.targetAgent?.remote_execution?.worker_kernel_id ?? null,
      })
    } else if (providerSettlement.reason === "credential_vault_locked") {
      deps.logger?.warn("skipping attach-time provider launch because the credential vault is locked", {
        session_id: session.id,
        agent_id: providerSettlement.targetAgent?.id ?? null,
      })
    }
  }
  await deps.catchUpAttachedSession(client, session.id, attachment.id, attachedSession, deps.logger)
  const hydratedSession = await deps.getSessionState(client, session.id)
  const visibleAgentId = deps.resolveVisibleAgentId(hydratedSession, preferences)
  const providerCatalogPromise = deps.getProviderCatalog(client, deps.logger)
  const providerCommandCatalogsPromise = deps.getProviderCommandCatalogs(client, deps.logger)
  const terminalCommandCatalogPromise = deps.getTerminalCommandCatalog(client, deps.logger)
  const attachedHistoryPromise = hydrateAttachedHistory(
    client,
    session.id,
    visibleAgentId,
    hydratedSession,
    deps,
  )

  return {
    client,
    binding: {
      session: hydratedSession,
      attachment,
      providerRun,
      providerLaunchIssue: providerSettlement.action === "skipped"
        && providerSettlement.reason === "credential_vault_locked"
        ? "credential_vault_locked"
        : null,
      createdSession,
      historyEntries: [],
      promptHistoryEntries: [],
      nextHistoryCursor: null,
    },
    sessions,
    providerCatalog: fallbackProviderCatalog({ source: "local_fallback" }),
    providerCommandCatalogs: fallbackProviderCommandCatalogs({ catalogSource: "local_fallback" }),
    terminalCommandCatalog: null,
    options,
    preferences,
    deferred: {
      providerCatalog: providerCatalogPromise,
      providerCommandCatalogs: providerCommandCatalogsPromise,
      terminalCommandCatalog: terminalCommandCatalogPromise,
      attachedHistory: attachedHistoryPromise,
    },
  }
}

async function hydrateAttachedHistory(
  client: LocalIpcClient,
  sessionId: string,
  visibleAgentId: string | null,
  session: RuntimeSession,
  deps: Pick<BootstrapDeps, "getSessionHistoryOutline" | "getPromptInputHistory" | "prepareHistoryOutlineAgent">,
) {
  const agentIds = session.agents.map((agent) => agent.id)
  const outlinePromise = agentIds.length > 0
    ? deps.getSessionHistoryOutline(client, sessionId, agentIds)
    : Promise.resolve({ agents: [] })
  const [outline, promptHistoryEntries] = await Promise.all([
    outlinePromise,
    loadSessionPromptHistory(client, sessionId, deps),
  ])
  const agentEntries = Object.fromEntries(outline.agents.map((agent) => [
    agent.agent_id,
    deps.prepareHistoryOutlineAgent(agent, session),
  ]))
  const historyEntries = visibleAgentId ? agentEntries[visibleAgentId] ?? [] : []
  return {
    sessionId,
    visibleAgentId,
    agentEntries,
    historyEntries,
    promptHistoryEntries,
    nextHistoryCursor: sessionHistoryCursorForVisibleAgent(outline, visibleAgentId),
  }
}

async function loadSessionPromptHistory(
  client: LocalIpcClient,
  sessionId: string,
  deps: Pick<BootstrapDeps, "getPromptInputHistory">,
) {
  if (deps.getPromptInputHistory) {
    const history = await deps.getPromptInputHistory(client, sessionId)
    return extractPromptInputHistoryEntries(history.entries)
  }
  return []
}
