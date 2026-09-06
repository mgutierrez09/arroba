import type {
  ProviderAuthStatus,
  ProviderAccountProfile,
  ProviderLoginStart,
  ProviderLoginStatus,
  ProviderLogoutOutcome,
  ProviderProcessInfo,
} from "./cli-types.js"
import type { ParsedSlashCommand } from "./commands.js"
import { isBackendProviderId } from "./provider-catalog.js"
import {
  providerAccountDisplayLabel,
  providerAccountsForProvider,
  selectedProviderAccount,
} from "./waiting-room-provider-accounts.js"

type FooterTone = "info" | "error"

export type ProviderCommandHandlerDeps = {
  currentProviderId: () => string
  flashFooter: (message: string, tone: FooterTone) => void
  appendNotice: (message: string) => void
  applyProviderSelection?: (value: string) => Promise<void>
  getProviderAuthStatus?: (provider: string, accountProfile?: string) => Promise<ProviderAuthStatus>
  startProviderLogin?: (
    provider: string,
    accountProfile?: string,
    method?: string,
  ) => Promise<ProviderLoginStart>
  getProviderLoginStatus?: (loginId: string) => Promise<ProviderLoginStatus>
  sendProviderLoginInput?: (loginId: string, dataBase64: string) => Promise<{ login_id: string; byte_count: number }>
  cancelProviderLogin?: (loginId: string) => Promise<ProviderLoginStatus>
  readSecret?: (prompt: string) => Promise<string>
  logoutProvider?: (provider: string, accountProfile?: string) => Promise<ProviderLogoutOutcome>
  listProviderAccountProfiles?: (provider?: string | null) => Promise<ProviderAccountProfile[]>
  createProviderAccountProfile?: (provider: string, label: string) => Promise<ProviderAccountProfile>
  linkProviderAccountProfile?: (provider: string, label: string, path: string) => Promise<ProviderAccountProfile>
  importNativeProviderAccountProfile?: (provider: string) => Promise<ProviderAccountProfile>
  renameProviderAccountProfile?: (provider: string, profile: string, label: string) => Promise<ProviderAccountProfile>
  setDefaultProviderAccountProfile?: (provider: string, profile: string) => Promise<ProviderAccountProfile>
  refreshProviderAccountProfile?: (provider: string, profile: string) => Promise<ProviderAccountProfile>
  removeProviderAccountProfile?: (provider: string, profile: string) => Promise<ProviderAccountProfile>
  deleteProviderAccountProfileData?: (provider: string, profile: string) => Promise<ProviderAccountProfile>
  listProviderProcesses?: (provider?: string | null) => Promise<ProviderProcessInfo[]>
  teardownProviderProcesses?: (provider?: string | null) => Promise<ProviderProcessInfo[]>
}

export async function handleProviderSlashCommand(
  deps: ProviderCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "provider" }>,
): Promise<void> {
  const { value } = command
  if (!value) {
    deps.flashFooter("usage: /provider <opencode|codex|claude-headless|claude-p|status|login|login-status|login-input|login-cancel|logout|reauth|processes>", "error")
    return
  }

  const parts = value.split(/\s+/).filter(Boolean)
  const [action] = parts
  const method = extractEnrollmentMethod(parts)
  if (method.error) {
    deps.flashFooter(method.error, "error")
    return
  }
  const rest = method.rest
  const [maybeProvider, maybeProfile] = rest.slice(1)
  if (action === "accounts") {
    await handleProviderAccountsCommand(deps, parts.slice(1))
    return
  }
  if (action === "status") {
    await showProviderStatus(deps, maybeProvider ?? deps.currentProviderId(), maybeProfile)
    return
  }
  if (action === "login") {
    await startProviderLogin(deps, maybeProvider ?? deps.currentProviderId(), maybeProfile, method.value)
    return
  }
  if (action === "login-status") {
    await showProviderLoginStatus(deps, maybeProvider)
    return
  }
  if (action === "login-input") {
    await sendProviderLoginInput(deps, maybeProvider)
    return
  }
  if (action === "login-cancel") {
    await cancelProviderLogin(deps, maybeProvider)
    return
  }
  if (action === "logout") {
    await logoutProvider(deps, maybeProvider ?? deps.currentProviderId(), maybeProfile)
    return
  }
  if (action === "reauth") {
    await reauthProvider(deps, maybeProvider ?? deps.currentProviderId(), maybeProfile, method.value)
    return
  }
  if (action === "processes") {
    await handleProviderProcessesCommand(deps, parts)
    return
  }
  if (!isBackendProviderId(value)) {
    deps.flashFooter(`unknown provider: ${value}`, "error")
    return
  }
  if (deps.applyProviderSelection) {
    await deps.applyProviderSelection(value)
  } else {
    deps.flashFooter(`${value} selected`, "info")
  }
}

function formatProviderProcessNotice(process: ProviderProcessInfo): string {
  const lines = [
    `${process.process_id} provider=${process.provider} pid=${process.pid ?? "-"} rss=${formatBytes(process.resident_set_bytes ?? null)} status=${process.status} mode=${process.endpoint_mode} safe=${String(process.teardown_safe)}`,
    `  provider sessions: ${process.provider_session_ids.join(",") || "-"}`,
    `  owner runs: ${process.owner_provider_run_ids.join(",") || "-"}`,
    `  owner sessions: ${process.owner_session_ids.join(",") || "-"}`,
    `  attached sessions: ${process.attached_session_ids.join(",") || "-"}`,
    `  active workflow runs: ${process.active_workflow_run_ids.join(",") || "-"}`,
  ]
  if (process.teardown_blockers.length > 0) {
    lines.push(`  blockers: ${process.teardown_blockers.join("; ")}`)
  }
  lines.push(`  next: ${providerProcessNextAction(process)}`)
  return lines.join("\n")
}

function providerProcessNextAction(process: ProviderProcessInfo): string {
  const teardown = `run /provider processes teardown ${process.provider}`
  if (process.teardown_safe) {
    return `${teardown} to stop only safe daemon-tracked processes owned by you`
  }
  if (process.attached_session_ids.length > 0) {
    return `detach or finish attached sessions ${process.attached_session_ids.join(",")}; then ${teardown}`
  }
  if (process.active_workflow_run_ids.length > 0) {
    return `stop or finish workflow runs ${process.active_workflow_run_ids.join(",")}; then ${teardown}`
  }
  if (process.teardown_blockers.length > 0) {
    return `resolve blockers: ${process.teardown_blockers.join("; ")}; then ${teardown}`
  }
  return `inspect the owning session before teardown; then ${teardown}`
}

function formatBytes(bytes: number | null): string {
  if (typeof bytes !== "number" || !Number.isFinite(bytes) || bytes <= 0) {
    return "unknown"
  }
  const units = ["B", "KiB", "MiB", "GiB"]
  let value = bytes
  let unitIndex = 0
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024
    unitIndex += 1
  }
  const formatted = unitIndex === 0 ? `${Math.round(value)}` : value >= 10 ? value.toFixed(1) : value.toFixed(2)
  return `${formatted}${units[unitIndex]}`
}

async function showProviderStatus(
  deps: ProviderCommandHandlerDeps,
  provider: string,
  accountProfile?: string,
): Promise<void> {
  if (!deps.getProviderAuthStatus) {
    deps.flashFooter("provider status is not available in this daemon", "error")
    return
  }
  const resolvedAccount = await resolveProviderAccountReference(deps, provider, accountProfile)
  if (resolvedAccount === null) return
  const status = await deps.getProviderAuthStatus(provider, resolvedAccount)
  const accountLabel = await providerAccountPublicLabel(deps, status.provider, status.account_profile)
  const details = `${providerAccountSubject(status.provider, accountLabel)}: ${status.auth_state}${status.identity_summary ? ` as ${status.identity_summary}` : ""}`
  deps.appendNotice(
    [
      details,
      status.detected_version ? `version ${status.detected_version}` : null,
      status.plan ? `plan ${status.plan}` : null,
      status.login_hint ?? null,
    ].filter(Boolean).join(" • "),
  )
  deps.flashFooter(details, "info")
}

async function startProviderLogin(
  deps: ProviderCommandHandlerDeps,
  provider: string,
  accountProfile?: string,
  method?: string,
): Promise<void> {
  if (!deps.startProviderLogin) {
    deps.flashFooter("provider login is not available in this daemon", "error")
    return
  }
  const resolvedAccount = await resolveProviderAccountReference(deps, provider, accountProfile)
  if (resolvedAccount === null) return
  const login = await deps.startProviderLogin(provider, resolvedAccount, method)
  const message = formatProviderLoginNotice(login, "login started", await providerAccountPublicLabel(deps, login.provider, login.account_profile))
  deps.appendNotice(message)
  deps.flashFooter(message, "info")
}

async function showProviderLoginStatus(
  deps: ProviderCommandHandlerDeps,
  loginId?: string,
): Promise<void> {
  if (!loginId || !deps.getProviderLoginStatus) {
    deps.flashFooter("usage: /provider login-status <login-id>", "error")
    return
  }
  const login = await deps.getProviderLoginStatus(loginId)
  const output = Buffer.from(login.terminal_output_base64, "base64").toString("utf8").trimEnd()
  if (output) deps.appendNotice(output)
  const accountLabel = await providerAccountPublicLabel(deps, login.provider, login.account_profile)
  deps.flashFooter(`${providerAccountSubject(login.provider, accountLabel)} login ${login.state}`, login.state === "failed" ? "error" : "info")
}

async function sendProviderLoginInput(
  deps: ProviderCommandHandlerDeps,
  loginId?: string,
): Promise<void> {
  if (!loginId || !deps.sendProviderLoginInput) {
    deps.flashFooter("usage: /provider login-input <login-id>", "error")
    return
  }
  if (!deps.readSecret) {
    deps.flashFooter("provider login input requires hidden input support", "error")
    return
  }
  const value = await deps.readSecret("provider login input: ")
  await deps.sendProviderLoginInput(loginId, Buffer.from(`${value}\r`, "utf8").toString("base64"))
  deps.flashFooter(`input sent; run /provider login-status ${loginId}`, "info")
}

async function cancelProviderLogin(
  deps: ProviderCommandHandlerDeps,
  loginId?: string,
): Promise<void> {
  if (!loginId || !deps.cancelProviderLogin) {
    deps.flashFooter("usage: /provider login-cancel <login-id>", "error")
    return
  }
  const login = await deps.cancelProviderLogin(loginId)
  const accountLabel = await providerAccountPublicLabel(deps, login.provider, login.account_profile)
  deps.flashFooter(`${providerAccountSubject(login.provider, accountLabel)} login cancelled`, "info")
}

async function logoutProvider(
  deps: ProviderCommandHandlerDeps,
  provider: string,
  accountProfile?: string,
): Promise<void> {
  if (!deps.logoutProvider) {
    deps.flashFooter("provider logout is not available in this daemon", "error")
    return
  }
  const resolvedAccount = await resolveProviderAccountReference(deps, provider, accountProfile)
  if (resolvedAccount === null) return
  const outcome = await deps.logoutProvider(provider, resolvedAccount)
  const message = outcome.kind === "logged_out"
    ? `${providerAccountSubject(outcome.result.provider, await providerAccountPublicLabel(deps, outcome.result.provider, outcome.result.account_profile))} logged out`
    : formatProviderLoginNotice(outcome.workflow, "logout started", await providerAccountPublicLabel(deps, outcome.workflow.provider, outcome.workflow.account_profile))
  deps.appendNotice(message)
  deps.flashFooter(message, "info")
}

async function reauthProvider(
  deps: ProviderCommandHandlerDeps,
  provider: string,
  accountProfile?: string,
  method?: string,
): Promise<void> {
  if (!deps.logoutProvider || !deps.startProviderLogin) {
    deps.flashFooter("provider reauth is not available in this daemon", "error")
    return
  }
  const resolvedAccount = await resolveProviderAccountReference(deps, provider, accountProfile)
  if (resolvedAccount === null) return
  const logout = await deps.logoutProvider(provider, resolvedAccount)
  if (logout.kind === "interaction_required") {
    const message = formatProviderLoginNotice(logout.workflow, "logout started; finish it before reauth", await providerAccountPublicLabel(deps, logout.workflow.provider, logout.workflow.account_profile))
    deps.appendNotice(message)
    deps.flashFooter(message, "info")
    return
  }
  const login = await deps.startProviderLogin(provider, resolvedAccount, method)
  const message = formatProviderLoginNotice(login, "reauth started", await providerAccountPublicLabel(deps, login.provider, login.account_profile))
  deps.appendNotice(message)
  deps.flashFooter(message, "info")
}

async function handleProviderAccountsCommand(
  deps: ProviderCommandHandlerDeps,
  args: string[],
): Promise<void> {
  const [action = "list", provider, profile, ...rest] = args
  if (action === "list") {
    if (!deps.listProviderAccountProfiles) {
      deps.flashFooter("provider account management is unavailable", "error")
      return
    }
    const profiles = await deps.listProviderAccountProfiles(provider ?? null)
    const lines = profiles.map((entry) => {
      const services = formatProviderAccountServices(entry)
      return `${entry.provider} ${entry.label}${entry.is_default ? " [default]" : ""} · ${credentialKindLabel(entry)} · ${entry.auth_state}${entry.plan ? ` · ${entry.plan}` : ""}${services ? ` · ${services}` : ""} · ${formatProviderAccountUsage(entry)}`
    })
    deps.appendNotice(lines.length > 0 ? lines.join("\n") : "No provider accounts registered")
    deps.flashFooter(`${profiles.length} provider account profile${profiles.length === 1 ? "" : "s"}`, "info")
    return
  }
  if (!provider) {
    deps.flashFooter("usage: /provider accounts <add|link|import-native|refresh|rename|default|remove|delete> <provider> ...", "error")
    return
  }
  let result: ProviderAccountProfile | null = null
  if (action === "add" && deps.createProviderAccountProfile) {
    const { rest: addOperands, value: addMethod } = extractEnrollmentMethod(
      [profile, ...rest].filter((operand): operand is string => operand !== undefined),
    )
    result = await deps.createProviderAccountProfile(
      provider,
      addOperands.filter(Boolean).join(" "),
    )
    if (result && addMethod) {
      if (!deps.startProviderLogin) {
        deps.flashFooter("provider login is not available in this daemon", "error")
        return
      }
      const login = await deps.startProviderLogin(provider, result.profile_id, addMethod)
      const message = formatProviderLoginNotice(
        login,
        "enrollment started",
        await providerAccountPublicLabel(deps, login.provider, login.account_profile),
      )
      deps.appendNotice(message)
      deps.flashFooter(message, "info")
      return
    }
  } else if (action === "import-native" && !profile && deps.importNativeProviderAccountProfile) {
    result = await deps.importNativeProviderAccountProfile(provider)
  } else if (action === "link" && profile && deps.linkProviderAccountProfile) {
    const operands = [profile, ...rest]
    const path = operands.at(-1)!
    result = await deps.linkProviderAccountProfile(provider, operands.slice(0, -1).join(" "), path)
  } else if (["refresh", "rename", "default", "remove", "delete"].includes(action) && profile) {
    const profileId = await resolveProviderAccountAlias(deps, provider, profile)
    if (!profileId) return
    if (action === "refresh" && deps.refreshProviderAccountProfile) {
      result = await deps.refreshProviderAccountProfile(provider, profileId)
    } else if (action === "rename" && rest.length > 0 && deps.renameProviderAccountProfile) {
      result = await deps.renameProviderAccountProfile(provider, profileId, rest.join(" "))
    } else if (action === "default" && deps.setDefaultProviderAccountProfile) {
      result = await deps.setDefaultProviderAccountProfile(provider, profileId)
    } else if (action === "remove" && deps.removeProviderAccountProfile) {
      result = await deps.removeProviderAccountProfile(provider, profileId)
    } else if (action === "delete" && rest[0] === profile && deps.deleteProviderAccountProfileData) {
      result = await deps.deleteProviderAccountProfileData(provider, profileId)
    }
  }
  if (!result) {
    deps.flashFooter("invalid or unavailable Provider Accounts action; destructive delete requires repeating the account alias", "error")
    return
  }
  deps.appendNotice(`${action}: ${result.provider} ${result.label}`)
  deps.flashFooter(`provider account ${action} complete`, "info")
}

function formatProviderAccountServices(profile: ProviderAccountProfile): string {
  return (profile.services ?? []).map((service) => {
    const billing = service.billing_kind ? `/${service.billing_kind.replaceAll("_", " ")}` : ""
    return `${service.label} ${service.auth_state} (${service.credential_type.replaceAll("_", " ")}${billing})`
  }).join(" | ")
}

export async function resolveProviderAccountAlias(
  deps: ProviderCommandHandlerDeps,
  provider: string,
  alias: string,
): Promise<string | null> {
  if (!deps.listProviderAccountProfiles) {
    deps.flashFooter("provider account management is unavailable", "error")
    return null
  }
  const profile = findProviderAccountByAlias(
    await deps.listProviderAccountProfiles(provider),
    provider,
    alias,
  )
  if (profile) return profile.profile_id
  deps.flashFooter(`provider account alias ${alias.trim()} was not found for ${provider}`, "error")
  return null
}

function formatProviderAccountUsage(profile: ProviderAccountProfile): string {
  const meters = profile.usage.meters ?? []
  if (meters.some((meter) => meter.used_percent != null || meter.remaining != null || meter.used != null)) {
    return meters.map((meter) => formatUsageMeter(meter, meters.length > 1)).join(" | ")
  }
  const reason = providerUsageSourceReason(profile.usage.source)
  if (profile.usage.availability === "partial") return `partially reported (${reason})`
  if (profile.usage.availability === "stale") return `stale (${reason})`
  if (profile.usage.availability === "error") return `refresh failed (${reason})`
  return `not reported (${reason})`
}

function formatUsageMeter(
  meter: NonNullable<ProviderAccountProfile["usage"]["meters"]>[number],
  labeled: boolean,
): string {
  const labelPrefix = labeled && meter.label ? `${meter.label} ` : ""
  const meterState = meter.state === "exhausted" ? " · exhausted" : meter.state === "warning" ? " · nearing limit" : ""
  const reset = meter.resets_at_ms ? ` · resets ${relativeResetTime(meter.resets_at_ms)}` : ""
  if (meter.used_percent != null) return `${labelPrefix}${Math.round(meter.used_percent)}% used${meterState}${reset}`
  if (meter.remaining != null) return `${labelPrefix}${meter.remaining}${meter.unit ? ` ${meter.unit}` : ""} remaining${meterState}${reset}`
  if (meter.used != null) return `${labelPrefix}${meter.used}${meter.unit ? ` ${meter.unit}` : ""} used${meterState}${reset}`
  return `${labelPrefix}no reported value`.trim()
}

function providerUsageSourceReason(source: string): string {
  switch (source) {
    case "provider_api_unavailable": return "provider usage API unavailable"
    case "provider_not_observed": return "provider has not exposed usage yet"
    case "opencode.local_stats": return "OpenCode exposes local stats, not upstream balance"
    default: return source.replaceAll(/[._-]+/g, " ")
  }
}

function relativeResetTime(timestamp: number): string {
  const seconds = Math.round((timestamp - Date.now()) / 1_000)
  if (seconds <= 0) return "now"
  if (seconds < 60) return `in ${seconds}s`
  if (seconds < 3_600) return `in ${Math.round(seconds / 60)}m`
  if (seconds < 86_400) return `in ${Math.round(seconds / 3_600)}h`
  return `in ${Math.round(seconds / 86_400)}d`
}

function formatProviderLoginNotice(
  login: ProviderLoginStart,
  action: string,
  accountLabel: string,
): string {
  return [
    `${providerAccountSubject(login.provider, accountLabel)} ${action}`,
    login.login_kind.startsWith("terminal") && login.login_id
      ? `run /provider login-status ${login.login_id}; respond with /provider login-input ${login.login_id}`
      : null,
    login.user_code ? `code ${login.user_code}` : null,
    login.verification_url ?? login.auth_url ?? null,
  ].filter(Boolean).join(" • ")
}

function providerAccountSubject(provider: string, accountLabel: string): string {
  return accountLabel === "Account unavailable" ? provider : `${provider}/${accountLabel}`
}

/// Extracts a `--method <name>` enrollment selection from the token stream,
/// leaving the positional operands intact.
function extractEnrollmentMethod(parts: string[]): {
  rest: string[]
  value?: string | undefined
  error?: string | undefined
} {
  const index = parts.indexOf("--method")
  if (index < 0) {
    return { rest: parts }
  }
  const value = parts[index + 1]
  if (!value || value.startsWith("--")) {
    return {
      rest: parts,
      error: "--method requires an enrollment method",
    }
  }
  return {
    rest: parts.filter((_, position) => position !== index && position !== index + 1),
    value,
  }
}

function credentialKindLabel(profile: {
  credential_kind?: string | null
  credential_kind_not_reported_reason?: string | null
}): string {
  switch (profile.credential_kind) {
    case "subscription":
      return "subscription"
    case "api_key":
      return "api key"
    case "prepaid":
      return "prepaid"
    case "mixed":
      return "mixed"
    default:
      return profile.credential_kind_not_reported_reason
        ? `kind not reported (${profile.credential_kind_not_reported_reason})`
        : "kind not reported"
  }
}

function findProviderAccountByAlias(
  profiles: Awaited<ReturnType<NonNullable<ProviderCommandHandlerDeps["listProviderAccountProfiles"]>>>,
  provider: string,
  alias: string,
) {
  const normalizedAlias = alias.trim()
  const candidates = providerAccountsForProvider(profiles, provider)
  const exactMatch = candidates.find((candidate) => candidate.label === normalizedAlias)
  if (exactMatch) return exactMatch
  const foldedMatches = candidates.filter(
    (candidate) => candidate.label.localeCompare(normalizedAlias, undefined, { sensitivity: "accent" }) === 0,
  )
  return foldedMatches.length === 1 ? foldedMatches[0] : undefined
}

async function resolveProviderAccountReference(
  deps: ProviderCommandHandlerDeps,
  provider: string,
  accountAlias: string | undefined,
): Promise<string | undefined | null> {
  const normalizedAlias = accountAlias?.trim()
  if (!normalizedAlias || normalizedAlias === "default") return normalizedAlias
  if (!deps.listProviderAccountProfiles) {
    deps.flashFooter("provider account selection is unavailable", "error")
    return null
  }
  const profile = findProviderAccountByAlias(await deps.listProviderAccountProfiles(provider), provider, normalizedAlias)
  if (profile) return profile.profile_id
  deps.flashFooter(`provider account alias ${normalizedAlias} was not found for ${provider}`, "error")
  return null
}

async function providerAccountPublicLabel(
  deps: ProviderCommandHandlerDeps,
  provider: string,
  accountProfile: string | null | undefined,
): Promise<string> {
  if (!deps.listProviderAccountProfiles) return "Account unavailable"
  const profile = selectedProviderAccount(
    await deps.listProviderAccountProfiles(provider),
    provider,
    accountProfile ?? undefined,
  )
  return profile ? providerAccountDisplayLabel(profile) : "Account unavailable"
}

async function handleProviderProcessesCommand(
  deps: ProviderCommandHandlerDeps,
  parts: string[],
): Promise<void> {
  if (parts[1] === "teardown") {
    const provider = parts[2]?.trim() || null
    if (!provider) {
      deps.flashFooter("usage: /provider processes teardown <provider>", "error")
      return
    }
    await teardownProviderProcesses(deps, provider)
    return
  }
  await listProviderProcesses(deps, parts[1] ?? null)
}

async function teardownProviderProcesses(
  deps: ProviderCommandHandlerDeps,
  provider: string | null,
): Promise<void> {
  if (!deps.teardownProviderProcesses) {
    deps.flashFooter("provider process teardown is not available in this daemon", "error")
    return
  }
  const blocked = deps.listProviderProcesses
    ? (await deps.listProviderProcesses(provider)).filter((process) => !process.teardown_safe)
    : []
  const tornDown = await deps.teardownProviderProcesses(provider)
  if (tornDown.length === 0) {
    if (blocked.length > 0) {
      deps.appendNotice(`blocked provider processes:\n${blocked.map((process) => formatProviderProcessNotice(process)).join("\n")}`)
    }
    deps.flashFooter("no safe provider processes to tear down", "info")
    return
  }
  deps.appendNotice(
    tornDown.map((process) => formatProviderProcessNotice(process)).join("\n"),
  )
  if (blocked.length > 0) {
    deps.appendNotice(`skipped blocked provider processes:\n${blocked.map((process) => formatProviderProcessNotice(process)).join("\n")}`)
  }
  deps.flashFooter(`tore down ${tornDown.length} provider process(es)`, "info")
}

async function listProviderProcesses(
  deps: ProviderCommandHandlerDeps,
  provider: string | null,
): Promise<void> {
  if (!deps.listProviderProcesses) {
    deps.flashFooter("provider process inspection is not available in this daemon", "error")
    return
  }
  const processes = await deps.listProviderProcesses(provider)
  if (processes.length === 0) {
    deps.flashFooter("no daemon-tracked provider processes", "info")
    return
  }
  deps.appendNotice(
    processes.map((process) => formatProviderProcessNotice(process)).join("\n"),
  )
  deps.flashFooter(`listed ${processes.length} provider process(es)`, "info")
}
