import type {
  AgentInstance,
  ProviderAccountProfile,
  RuntimeSession,
} from "./cli-types.js"
import type { ResolvedAgentReference } from "@chariox/kernel-client/session-agent-resolver"
import {
  normalizeBackendProviderId,
  selectConfiguredModel,
  selectConfiguredVariant,
  type ProviderCatalog,
} from "./provider-catalog.js"
import {
  providerAccountDisplayLabel,
  providerAccountsForProvider,
  selectedProviderAccount,
} from "./waiting-room-provider-accounts.js"

const SESSION_AGENT_MODE_CONFIG_KEY = "agents.mode"
const SESSION_AGENT_PERMISSION_CONFIG_KEY = "agents.permissions"

type FooterTone = "info" | "error"

type AgentConfigUpdatePayload = {
  agent: AgentInstance
  session: RuntimeSession
}

export type AgentConfigCommandHandlerDeps = {
  sessionState: () => RuntimeSession
  focusedAgentId: () => string | null
  flashFooter: (message: string, tone: FooterTone) => void
  formatError: (error: unknown) => string
  updateAgentConfig?: (
    sessionId: string,
    agentId: string,
    options: {
      executionMode?: "build" | "plan" | null
      clearExecutionMode?: boolean
      permissionLevel?: "required" | "yolo" | null
      clearPermissionLevel?: boolean
    },
  ) => Promise<AgentConfigUpdatePayload>
  updateAgentProfile?: (
    sessionId: string,
    agentId: string,
    options: {
      provider?: string | null
      accountProfile?: string | null
      model?: string | null
      effort?: string | null
      clearEffort?: boolean
    },
  ) => Promise<AgentConfigUpdatePayload>
  listProviderAccountProfiles?: (provider?: string | null) => Promise<ProviderAccountProfile[]>
  getProviderCatalogForAgent?: (
    agent: AgentInstance,
    provider: string,
    accountProfile: string,
  ) => Promise<ProviderCatalog>
  aliasAgent?: (
    sessionId: string,
    agentId: string,
    alias: string,
  ) => Promise<AgentConfigUpdatePayload>
  applySessionState: (session: RuntimeSession) => void
  refreshAgentPanes: (session: RuntimeSession) => Promise<void>
  resolveSessionAgent: (reference?: string | null) => ResolvedAgentReference
  formatAgentLabel: (agent: AgentInstance | null | undefined) => string
}

export async function handleAgentAliasCommand(
  deps: AgentConfigCommandHandlerDeps,
  args: string[],
): Promise<void> {
  if (!deps.aliasAgent) {
    deps.flashFooter("agent aliases are unavailable in this build", "error")
    return
  }
  const reference = args[1]
  const explicitAliasArgs = args.length > 2 ? args.slice(2) : args.slice(1)
  const resolved = deps.resolveSessionAgent(args.length > 2 ? reference : deps.focusedAgentId() ?? undefined)
  if (!resolved.agent) {
    deps.flashFooter(resolved.error ?? "usage: /agent alias [agent-ref] <alias|clear>", "error")
    return
  }
  const rawAlias = explicitAliasArgs.join(" ").trim()
  if (!rawAlias) {
    deps.flashFooter(`${deps.formatAgentLabel(resolved.agent)} alias: ${resolved.agent.alias ?? "<none>"}`, "info")
    return
  }
  const shouldClearAgentAlias = rawAlias === "clear" || rawAlias === "none" || rawAlias === "-"
  try {
    const payload = await deps.aliasAgent(deps.sessionState().id, resolved.agent.id, shouldClearAgentAlias ? "" : rawAlias)
    deps.applySessionState(payload.session)
    await deps.refreshAgentPanes(payload.session)
    deps.flashFooter(`${deps.formatAgentLabel(payload.agent)} alias: ${payload.agent.alias ?? "<none>"}`, "info")
  } catch (error) {
    deps.flashFooter(deps.formatError(error), "error")
  }
}

export async function handleAgentModeCommand(
  deps: AgentConfigCommandHandlerDeps,
  args: string[],
): Promise<void> {
  if (!deps.updateAgentConfig) {
    deps.flashFooter("agent config updates are unavailable in this build", "error")
    return
  }
  const reference = args[1]
  const rawValue = args[2] ?? (reference && (parseExecutionMode(reference) || reference === "inherit") ? reference : undefined)
  const resolved = deps.resolveSessionAgent(rawValue ? reference : deps.focusedAgentId() ?? undefined)
  if (!resolved.agent) {
    deps.flashFooter(resolved.error ?? "usage: /agent mode [agent-ref] <build|plan|inherit>", "error")
    return
  }
  if (!rawValue) {
    const effective = effectiveAgentExecutionMode(deps.sessionState(), resolved.agent)
    const source = resolved.agent.execution_mode_override ? "agent" : "session"
    deps.flashFooter(`${deps.formatAgentLabel(resolved.agent)} mode: ${effective} (${source})`, "info")
    return
  }
  if (rawValue !== "inherit" && !parseExecutionMode(rawValue)) {
    deps.flashFooter("usage: /agent mode [agent-ref] <build|plan|inherit>", "error")
    return
  }
  const payload = await deps.updateAgentConfig(deps.sessionState().id, resolved.agent.id, {
    executionMode: rawValue === "inherit" ? null : parseExecutionMode(rawValue),
    clearExecutionMode: rawValue === "inherit",
  })
  deps.applySessionState(payload.session)
  await deps.refreshAgentPanes(payload.session)
  const effective = effectiveAgentExecutionMode(payload.session, payload.agent)
  deps.flashFooter(
    `${deps.formatAgentLabel(payload.agent)} mode: ${effective}${rawValue === "inherit" ? " (session)" : " (agent)"}`,
    "info",
  )
}

export async function handleAgentProfileCommand(
  deps: AgentConfigCommandHandlerDeps,
  args: string[],
  subcommand: "provider" | "account" | "model" | "variant",
): Promise<void> {
  if (!deps.updateAgentProfile) {
    deps.flashFooter("agent profile updates are unavailable in this build", "error")
    return
  }
  const reference = args[1]
  const rawValue = args.length > 2 ? args.slice(2).join(" ").trim() : args.slice(1).join(" ").trim()
  const resolved = deps.resolveSessionAgent(args.length > 2 ? reference : deps.focusedAgentId() ?? undefined)
  if (!resolved.agent) {
    deps.flashFooter(resolved.error ?? `usage: /agent ${subcommand} [agent-ref] <value>`, "error")
    return
  }
  if (!rawValue) {
    const value = subcommand === "provider"
      ? resolved.agent.provider
      : subcommand === "account"
        ? await agentAccountDisplayValue(deps, resolved.agent)
      : subcommand === "model"
        ? resolved.agent.model ?? "<none>"
        : resolved.agent.effort ?? "<none>"
    deps.flashFooter(`${deps.formatAgentLabel(resolved.agent)} ${subcommand}: ${value}`, "info")
    return
  }
  if (subcommand === "account") {
    await updateAgentAccountProfile(deps, resolved.agent, rawValue)
    return
  }
  const shouldClearEffort = subcommand === "variant" && ["clear", "none", "-", "default"].includes(rawValue)
  const payload = await deps.updateAgentProfile(deps.sessionState().id, resolved.agent.id, {
    ...(subcommand === "provider" ? { provider: rawValue } : {}),
    ...(subcommand === "model" ? { model: rawValue } : {}),
    ...(subcommand === "variant" && !shouldClearEffort ? { effort: rawValue } : {}),
    ...(shouldClearEffort ? { clearEffort: true } : {}),
  })
  deps.applySessionState(payload.session)
  await deps.refreshAgentPanes(payload.session)
  const value = subcommand === "provider"
    ? payload.agent.provider
    : subcommand === "model"
      ? payload.agent.model ?? "<none>"
      : payload.agent.effort ?? "<none>"
  deps.flashFooter(`${deps.formatAgentLabel(payload.agent)} ${subcommand}: ${value}`, "info")
}

async function agentAccountDisplayValue(
  deps: AgentConfigCommandHandlerDeps,
  agent: AgentInstance,
): Promise<string> {
  const profileId = agent.account_profile?.trim() || "default"
  if (!deps.listProviderAccountProfiles) return "Account unavailable"
  const profiles = await deps.listProviderAccountProfiles(agent.provider)
  const profile = selectedProviderAccount(profiles, agent.provider, profileId)
  return profile ? providerAccountDisplayLabel(profile, agent.model) : "Account unavailable"
}

async function updateAgentAccountProfile(
  deps: AgentConfigCommandHandlerDeps,
  agent: AgentInstance,
  accountAlias: string,
): Promise<void> {
  if (!deps.listProviderAccountProfiles || !deps.getProviderCatalogForAgent) {
    deps.flashFooter("agent account updates are unavailable in this build", "error")
    return
  }
  try {
    const profiles = await deps.listProviderAccountProfiles(agent.provider)
    const accounts = providerAccountsForProvider(profiles, agent.provider)
    const profile = accountAlias === "default"
      ? selectedProviderAccount(accounts, agent.provider, "default")
      : accounts.find((entry) => entry.label.localeCompare(accountAlias, undefined, { sensitivity: "accent" }) === 0)
    if (!profile) {
      deps.flashFooter(`account is unavailable for ${agent.provider}`, "error")
      return
    }
    const catalog = await deps.getProviderCatalogForAgent(agent, agent.provider, profile.profile_id)
    const provider = normalizeBackendProviderId(agent.provider)
    const model = selectConfiguredModel(catalog, agent.model, provider)
    if (!model) {
      deps.flashFooter(`no models are available for ${providerAccountDisplayLabel(profile)}`, "error")
      return
    }
    const effort = selectConfiguredVariant(model, agent.effort)
    const payload = await deps.updateAgentProfile!(deps.sessionState().id, agent.id, {
      provider: agent.provider,
      accountProfile: profile.profile_id,
      model: model.id,
      ...(effort ? { effort } : { clearEffort: true }),
    })
    deps.applySessionState(payload.session)
    await deps.refreshAgentPanes(payload.session)
    deps.flashFooter(`${deps.formatAgentLabel(payload.agent)} account: ${providerAccountDisplayLabel(profile, payload.agent.model)}`, "info")
  } catch (error) {
    deps.flashFooter(deps.formatError(error), "error")
  }
}

export async function handleAgentPermissionsCommand(
  deps: AgentConfigCommandHandlerDeps,
  args: string[],
): Promise<void> {
  if (!deps.updateAgentConfig) {
    deps.flashFooter("agent config updates are unavailable in this build", "error")
    return
  }
  const reference = args[1]
  const rawValue = args[2] ?? (reference && (parsePermissionLevel(reference) || reference === "inherit") ? reference : undefined)
  const resolved = deps.resolveSessionAgent(rawValue ? reference : deps.focusedAgentId() ?? undefined)
  if (!resolved.agent) {
    deps.flashFooter(resolved.error ?? "usage: /agent permissions [agent-ref] <required|yolo|inherit>", "error")
    return
  }
  if (!rawValue) {
    const effective = effectiveAgentPermissionLevel(deps.sessionState(), resolved.agent)
    const source = resolved.agent.permission_level_override ? "agent" : "session"
    deps.flashFooter(`${deps.formatAgentLabel(resolved.agent)} permissions: ${effective} (${source})`, "info")
    return
  }
  if (rawValue !== "inherit" && !parsePermissionLevel(rawValue)) {
    deps.flashFooter("usage: /agent permissions [agent-ref] <required|yolo|inherit>", "error")
    return
  }
  const payload = await deps.updateAgentConfig(deps.sessionState().id, resolved.agent.id, {
    permissionLevel: rawValue === "inherit" ? null : parsePermissionLevel(rawValue),
    clearPermissionLevel: rawValue === "inherit",
  })
  deps.applySessionState(payload.session)
  await deps.refreshAgentPanes(payload.session)
  const effective = effectiveAgentPermissionLevel(payload.session, payload.agent)
  deps.flashFooter(
    `${deps.formatAgentLabel(payload.agent)} permissions: ${effective}${rawValue === "inherit" ? " (session)" : " (agent)"}`,
    "info",
  )
}

function parseExecutionMode(value: string | null | undefined): "build" | "plan" | null {
  if (!value) return null
  const normalized = value.trim().toLowerCase()
  return normalized === "build" || normalized === "plan" ? normalized : null
}

function parsePermissionLevel(value: string | null | undefined): "required" | "yolo" | null {
  if (!value) return null
  const normalized = value.trim().toLowerCase()
  return normalized === "required" || normalized === "yolo" ? normalized : null
}

function effectiveAgentExecutionMode(session: RuntimeSession, agent: AgentInstance | null | undefined): "build" | "plan" {
  return agent?.execution_mode_override
    ?? parseExecutionMode(session.config_state?.values?.[SESSION_AGENT_MODE_CONFIG_KEY])
    ?? parseExecutionMode(session.agent_defaults?.execution_mode)
    ?? "build"
}

function effectiveAgentPermissionLevel(session: RuntimeSession, agent: AgentInstance | null | undefined): "required" | "yolo" {
  return agent?.permission_level_override
    ?? parsePermissionLevel(session.config_state?.values?.[SESSION_AGENT_PERMISSION_CONFIG_KEY])
    ?? parsePermissionLevel(session.agent_defaults?.permission_level)
    ?? "yolo"
}
