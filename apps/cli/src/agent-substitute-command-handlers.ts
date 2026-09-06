import type {
  AgentInstance,
  ProviderAccountProfile,
  RuntimeProviderRun,
  RuntimeSession,
} from "./cli-types.js"
import {
  formatAgentSubstituteSummary as formatSharedAgentSubstituteSummary,
  type AgentInstance as SharedAgentInstance,
  type SubstituteAccountLabelResolver,
} from "@chariox/kernel-client"
import { providerAccountsForProvider } from "./waiting-room-provider-accounts.js"
import type { ResolvedAgentReference } from "@chariox/kernel-client/session-agent-resolver"

type FooterTone = "info" | "error"

type AgentConfigUpdatePayload = {
  agent: AgentInstance
  session: RuntimeSession
}

export type AgentSubstituteCommandHandlerDeps = {
  sessionState: () => RuntimeSession
  focusedAgentId: () => string | null
  currentModelId: () => string
  currentVariantId: () => string
  flashFooter: (message: string, tone: FooterTone) => void
  updateAgentSubstitutes?: (
    sessionId: string,
    agentId: string,
    action: Record<string, unknown>,
  ) => Promise<AgentConfigUpdatePayload>
  applySessionState: (session: RuntimeSession) => void
  refreshAgentPanes: (session: RuntimeSession) => Promise<void>
  launchAgentProviderRun: (
    provider: string,
    model: string,
    variant: string,
    agentId: string,
    accountProfile?: string,
  ) => Promise<RuntimeProviderRun>
  setProviderRunState: (run: RuntimeProviderRun | null) => void
  refreshSessionState: (sessionId: string) => Promise<RuntimeSession>
  resolveSessionAgent: (reference?: string | null) => ResolvedAgentReference
  formatAgentLabel: (agent: AgentInstance | null | undefined) => string
  listProviderAccountProfiles?: (
    provider?: string | null,
  ) => Promise<ProviderAccountProfile[]>
}

export async function handleAgentSubstituteCommand(
  deps: AgentSubstituteCommandHandlerDeps,
  args: string[],
): Promise<void> {
  if (!deps.updateAgentSubstitutes) {
    deps.flashFooter("agent substitute updates are unavailable in this build", "error")
    return
  }
  const subcommand = args[1] ?? "list"
  const subArgs = args.slice(2)
  const agentFlagIndex = subArgs.indexOf("--agent")
  const agentRefFromFlag = agentFlagIndex >= 0 ? subArgs[agentFlagIndex + 1] : undefined
  const filteredArgs = agentFlagIndex >= 0
    ? subArgs.filter((_, index) => index !== agentFlagIndex && index !== agentFlagIndex + 1)
    : subArgs
  const resolved = deps.resolveSessionAgent(agentRefFromFlag ?? deps.focusedAgentId() ?? undefined)
  if (!resolved.agent) {
    deps.flashFooter(resolved.error ?? "no focused agent", "error")
    return
  }
  const agent = resolved.agent
  if (subcommand === "list" || subcommand === "ls") {
    deps.flashFooter(await formatSubstituteSummaryWithAccountLabels(deps, agent), "info")
    return
  }
  const applyUpdate = async (action: Record<string, unknown>) => {
    const payload = await deps.updateAgentSubstitutes!(
      deps.sessionState().id,
      agent.id,
      action,
    )
    deps.applySessionState(payload.session)
    await deps.refreshAgentPanes(payload.session)
    return payload
  }
  if (subcommand === "add") {
    const provider = filteredArgs[0]
    const model = filteredArgs[1]
    const variantIndex = filteredArgs.indexOf("--variant")
    const variant = variantIndex >= 0 ? filteredArgs[variantIndex + 1] : undefined
    const accountIndex = filteredArgs.indexOf("--account")
    if (accountIndex >= 0) {
      const accountAliasValue = filteredArgs[accountIndex + 1]
      if (accountAliasValue == null || accountAliasValue.startsWith("--")) {
        deps.flashFooter("usage: /agent substitute add <provider> <model> [--variant v] [--account alias] [--kernel k] [--worktree dir] [--agent a]", "error")
        return
      }
    }
    const accountAlias = accountIndex >= 0 ? filteredArgs[accountIndex + 1] : undefined
    const kernelIndex = filteredArgs.indexOf("--kernel")
    const kernelId = kernelIndex >= 0 ? filteredArgs[kernelIndex + 1] : undefined
    const worktreeIndex = filteredArgs.indexOf("--worktree")
    const worktreeId = worktreeIndex >= 0 ? filteredArgs[worktreeIndex + 1] : undefined
    if (!provider || !model) {
      deps.flashFooter("usage: /agent substitute add <provider> <model> [--variant v] [--account alias] [--kernel k] [--worktree dir] [--agent a]", "error")
      return
    }
    let accountProfileId: string | null = null
    let accountLabel: string | null = null
    if (accountAlias != null) {
      const resolved = await resolveProviderAccountAlias(deps, provider, accountAlias)
      if (!resolved) {
        return
      }
      accountProfileId = resolved.profile_id
      accountLabel = resolved.label
    }
    const payload = await applyUpdate({
      Add: {
        provider,
        model,
        variant: variant ?? null,
        account_profile: accountProfileId,
        kernel_id: kernelId ?? null,
        worktree_id: worktreeId ?? null,
      },
    })
    const accountSuffix = accountLabel ? ` · account ${accountLabel}` : ""
    deps.flashFooter(`${deps.formatAgentLabel(payload.agent)} substitute added: ${provider}/${model}${variant ? `/${variant}` : ""}${accountSuffix}`, "info")
    return
  }
  if (subcommand === "remove" || subcommand === "rm") {
    const index = Number.parseInt(filteredArgs[0] ?? "", 10)
    if (!Number.isFinite(index)) {
      deps.flashFooter("usage: /agent substitute remove <index> [--agent a]", "error")
      return
    }
    const payload = await applyUpdate({ Remove: { index } })
    deps.flashFooter(`${deps.formatAgentLabel(payload.agent)} substitute ${index} removed`, "info")
    return
  }
  if (subcommand === "move") {
    const fromIndex = Number.parseInt(filteredArgs[0] ?? "", 10)
    const toIndex = Number.parseInt(filteredArgs[1] ?? "", 10)
    if (!Number.isInteger(fromIndex) || !Number.isInteger(toIndex)) {
      deps.flashFooter("usage: /agent substitute move <from-index> <to-index> [--agent a]", "error")
      return
    }
    const payload = await applyUpdate({ Move: { from_index: fromIndex, to_index: toIndex } })
    deps.flashFooter(`${deps.formatAgentLabel(payload.agent)} substitute moved from ${fromIndex} to ${toIndex}`, "info")
    return
  }
  if (subcommand === "clear") {
    const payload = await applyUpdate({ Clear: {} })
    deps.flashFooter(`${deps.formatAgentLabel(payload.agent)} substitutes cleared`, "info")
    return
  }
  if (subcommand === "timeout") {
    const timeoutMs = parseSubstitutionTimeoutMs(filteredArgs[0])
    if (timeoutMs === undefined && filteredArgs[0] !== "inherit" && filteredArgs[0] !== "default") {
      deps.flashFooter("usage: /agent substitute timeout <ms|Ns|inherit> [--agent a]", "error")
      return
    }
    const payload = await applyUpdate({ SetTimeout: { timeout_ms: timeoutMs ?? null } })
    deps.flashFooter(`${deps.formatAgentLabel(payload.agent)} substitute timeout: ${timeoutMs == null ? "default" : `${timeoutMs}ms`}`, "info")
    return
  }
  if (subcommand === "activate") {
    const index = Number.parseInt(filteredArgs[0] ?? "", 10)
    if (!Number.isFinite(index)) {
      deps.flashFooter("usage: /agent substitute activate <index> [--agent a]", "error")
      return
    }
    const payload = await applyUpdate({ Activate: { index, reason: "manual" } })
    const profile = payload.agent.substitutes?.[index]
    if (!profile) {
      deps.flashFooter(`${deps.formatAgentLabel(payload.agent)} substitute ${index} is not available`, "error")
      return
    }
    const run = await deps.launchAgentProviderRun(
      profile.provider,
      profile.model,
      profile.variant ?? "",
      payload.agent.id,
      profile.account_profile ?? undefined,
    )
    deps.setProviderRunState(run)
    const refreshedSession = await deps.refreshSessionState(payload.session.id)
    deps.applySessionState(refreshedSession)
    await deps.refreshAgentPanes(refreshedSession)
    const accountSuffix = profile.account_profile
      ? await formatAccountSuffixForProfileId(deps, profile.provider, profile.account_profile)
      : ""
    deps.flashFooter(`${deps.formatAgentLabel(payload.agent)} activated substitute ${index}: ${profile.provider}/${profile.model}${accountSuffix}`, "info")
    return
  }
  if (subcommand === "primary" || subcommand === "reset") {
    const payload = await applyUpdate({ Primary: {} })
    const run = await deps.launchAgentProviderRun(
      payload.agent.provider,
      payload.agent.model ?? deps.currentModelId(),
      payload.agent.effort ?? deps.currentVariantId(),
      payload.agent.id,
      payload.agent.account_profile ?? undefined,
    )
    deps.setProviderRunState(run)
    const refreshedSession = await deps.refreshSessionState(payload.session.id)
    deps.applySessionState(refreshedSession)
    await deps.refreshAgentPanes(refreshedSession)
    deps.flashFooter(`${deps.formatAgentLabel(payload.agent)} reset to starter profile`, "info")
    return
  }
  deps.flashFooter("usage: /agent substitute list|add|remove|move|clear|timeout|activate|reset", "error")
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

export function formatAgentSubstituteSummary(agent: AgentInstance): string {
  return formatSharedAgentSubstituteSummary(agent as SharedAgentInstance)
}

async function resolveProviderAccountAlias(
  deps: AgentSubstituteCommandHandlerDeps,
  provider: string,
  alias: string,
): Promise<Pick<ProviderAccountProfile, "profile_id" | "label"> | null> {
  if (!deps.listProviderAccountProfiles) {
    deps.flashFooter("provider account inventory is unavailable", "error")
    return null
  }
  const profiles = providerAccountsForProvider(
    await deps.listProviderAccountProfiles(provider),
    provider,
  )
  const profile = profiles.find((entry) =>
    entry.label.localeCompare(alias, undefined, { sensitivity: "accent" }) === 0
  )
  if (profile) {
    return { profile_id: profile.profile_id, label: profile.label }
  }
  deps.flashFooter(`provider account alias ${alias} was not found for ${provider}`, "error")
  return null
}

async function substituteAccountLabelResolver(
  deps: AgentSubstituteCommandHandlerDeps,
): Promise<SubstituteAccountLabelResolver> {
  if (!deps.listProviderAccountProfiles) return () => null
  const profiles = await deps.listProviderAccountProfiles(null).catch(() => [])
  const byId = new Map(profiles.map((profile) => [profile.profile_id, profile]))
  return (provider, accountProfile) =>
    providerAccountsForProvider([...byId.values()], provider)
      .find((profile) => profile.profile_id === accountProfile)?.label ?? null
}

async function formatSubstituteSummaryWithAccountLabels(
  deps: AgentSubstituteCommandHandlerDeps,
  agent: AgentInstance,
): Promise<string> {
  return formatSharedAgentSubstituteSummary(
    agent as SharedAgentInstance,
    await substituteAccountLabelResolver(deps),
  )
}

async function formatAccountSuffixForProfileId(
  deps: AgentSubstituteCommandHandlerDeps,
  provider: string,
  accountProfileId: string,
): Promise<string> {
  const resolveLabel = await substituteAccountLabelResolver(deps)
  const label = resolveLabel(provider, accountProfileId)
  return label ? ` · account ${label}` : " · custom account"
}
