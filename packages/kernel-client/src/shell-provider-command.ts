import type {
  ProviderAuthStatus,
  ProviderLoginStart,
  ProviderLoginStatus,
  ProviderProcessInfo,
} from "./kernel-types.js"
import {
  getProviderAuthStatusRequest,
  getProviderLoginStatusRequest,
  cancelProviderLoginRequest,
  listProviderProcessesRequest,
  logoutProviderRequest,
  setProviderAccountCredentialRequest,
  startProviderLoginRequest,
  teardownProviderProcessesRequest,
} from "./ipc-requests.js"
import type { ParsedShellCommand, ShellCommandResult, ShellContext } from "./shell-core.js"
import {
  formatProviderAuthStatus,
  formatProviderLoginStart,
  formatProviderProcesses,
} from "./shell-provider-format.js"

type ShellProviderCommandDeps = {
  client: {
    send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  }
  readSecret?: ((prompt: string) => Promise<string>) | undefined
}

export async function executeProviderCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellProviderCommandDeps,
): Promise<ShellCommandResult> {
  const [action, providerArg, ...rest] = parsed.args
  if (action === "setup-token") {
    const provider = providerArg?.trim()
    const accountProfile = rest.find((value) => value !== "--replace")?.trim() || "default"
    const replace = rest.includes("--replace")
    if (provider !== "claude" || rest.filter((value) => value !== "--replace").length > 1) {
      return { ok: false, message: "usage: provider setup-token claude [account-profile] [--replace]" }
    }
    if (!deps.readSecret) {
      return {
        ok: false,
        message: "provider setup-token requires hidden input support; run it from interactive chariox-shell",
      }
    }
    const value = (await deps.readSecret(`Claude setup token for ${accountProfile}: `)).trim()
    if (!value) return { ok: false, message: "Claude setup token must not be empty" }
    const response = await deps.client.send(setProviderAccountCredentialRequest(
      provider,
      accountProfile,
      value,
      replace,
      { sessionId: context.sessionId, agentId: context.agentId },
    ))
    const stored = expectVariant<{
      provider: string
      account_profile: string
      credential_id: string
      replaced: boolean
    }>(response, "ProviderAccountCredentialStored")
    return {
      ok: true,
      message: `${stored.provider}/${stored.account_profile} setup token ${stored.replaced ? "replaced" : "stored"} in Chariox Vault`,
      data: { provider: stored.provider, account_profile: stored.account_profile, replaced: stored.replaced },
    }
  }
  if (action === "status") {
    const provider = providerArg ?? context.provider
    const response = await deps.client.send(getProviderAuthStatusRequest(provider))
    const status = expectVariant<{ status: ProviderAuthStatus }>(response, "ProviderAuthStatus").status
    return { ok: true, message: formatProviderAuthStatus(status), data: { status } }
  }
  if (action === "login" || action === "logout" || action === "reauth") {
    const provider = providerArg ?? context.provider
    if (action === "logout") {
      const response = await deps.client.send(logoutProviderRequest(provider))
      if ("ProviderLogoutStarted" in response) {
        const logout = (response.ProviderLogoutStarted as { logout: ProviderLoginStart }).logout
        return { ok: true, message: formatProviderLoginStart(logout, "logout"), data: { logout } }
      }
      const result = expectVariant<{ provider: string }>(response, "ProviderLoggedOut")
      return { ok: true, message: `${result.provider} logged out`, data: result }
    }
    if (action === "reauth") {
      const logoutResponse = await deps.client.send(logoutProviderRequest(provider))
      if ("ProviderLogoutStarted" in logoutResponse) {
        const logout = (logoutResponse.ProviderLogoutStarted as { logout: ProviderLoginStart }).logout
        return { ok: true, message: `${formatProviderLoginStart(logout, "logout")}\nFinish logout before starting reauthentication.`, data: { logout } }
      }
    }
    const response = await deps.client.send(startProviderLoginRequest(provider))
    const login = expectVariant<{ login: ProviderLoginStart }>(response, "ProviderLoginStarted").login
    const verb = action === "reauth" ? "reauth" : "login"
    return { ok: true, message: formatProviderLoginStart(login, verb), data: { login } }
  }
  if (action === "login-status") {
    if (!providerArg) return { ok: false, message: "usage: provider login-status <login-id>" }
    const response = await deps.client.send(getProviderLoginStatusRequest(providerArg))
    const login = expectVariant<{ login: ProviderLoginStatus }>(response, "ProviderLoginStatus").login
    const output = Buffer.from(login.terminal_output_base64, "base64").toString("utf8").trimEnd()
    return {
      ok: login.state !== "failed",
      message: [output, `${login.provider}/${login.account_profile} login ${login.state}`].filter(Boolean).join("\n"),
      data: { login },
    }
  }
  if (action === "login-cancel") {
    if (!providerArg) return { ok: false, message: "usage: provider login-cancel <login-id>" }
    const response = await deps.client.send(cancelProviderLoginRequest(providerArg))
    const login = expectVariant<{ login: ProviderLoginStatus }>(response, "ProviderLoginCancelled").login
    return { ok: true, message: `${login.provider}/${login.account_profile} login cancelled`, data: { login } }
  }
  if (action === "processes") {
    const subcommand = providerArg
    if (subcommand === "teardown") {
      const provider = rest[0]?.trim() || null
      if (!provider) {
        return { ok: false, message: "usage: provider processes teardown <provider>" }
      }
      const response = await deps.client.send(teardownProviderProcessesRequest(provider))
      const processes = expectVariant<{ processes: ProviderProcessInfo[] }>(response, "ProviderProcessesTornDown").processes
      return { ok: true, message: processes.length === 0 ? "no safe provider processes to tear down" : `tore down ${processes.length} provider process(es)\n${formatProviderProcesses(processes)}`, data: { processes } }
    }
    const provider = providerArg ?? null
    const response = await deps.client.send(listProviderProcessesRequest(provider))
    const processes = expectVariant<{ processes: ProviderProcessInfo[] }>(response, "ProviderProcessesListed").processes
    return { ok: true, message: formatProviderProcesses(processes), data: { processes } }
  }
  return { ok: false, message: "usage: provider status|login|setup-token|login-status|login-cancel|logout|reauth|processes [provider]|processes teardown <provider>" }
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
