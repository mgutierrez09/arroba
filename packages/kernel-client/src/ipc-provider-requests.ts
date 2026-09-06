export function listProviderProcessesRequest(provider?: string | null) {
  return {
    ListProviderProcesses: {
      provider: provider ?? null,
    },
  }
}

export function teardownProviderProcessesRequest(provider?: string | null, force = false) {
  return {
    TeardownProviderProcesses: {
      provider: provider ?? null,
      force,
    },
  }
}

export function getProviderRunRequest(providerRunId: string) {
  return {
    GetProviderRun: {
      provider_run_id: providerRunId,
    },
  }
}

export function updateProviderRunSelectionRequest(
  sessionId: string,
  providerRunId: string,
  options: { model?: string | null; variant?: string | null; clearVariant?: boolean } = {},
) {
  return {
    UpdateProviderRunSelection: {
      session_id: sessionId,
      provider_run_id: providerRunId,
      model: options.model ?? null,
      variant: options.variant ?? null,
      clear_variant: options.clearVariant ?? false,
    },
  }
}

export type ProviderCatalogExecutionLocation =
  | { kind: "local" }
  | { kind: "worker"; kernel_ref: string }
  | { kind: "slice"; slice_ref: string }

export function getProviderCatalogRequest(options: {
  provider?: string | null
  accountProfile?: string | null
  executionLocation?: ProviderCatalogExecutionLocation
} = {}) {
  const accountProfiles = options.provider && options.accountProfile
    ? { [options.provider]: options.accountProfile }
    : {}
  return {
    GetProviderCatalog: {
      provider: options.provider ?? null,
      account_profiles: accountProfiles,
      execution_location: options.executionLocation ?? { kind: "local" },
    },
  }
}

export function getProviderCommandCatalogsRequest() {
  return { GetProviderCommandCatalogs: null }
}

export function getProviderAuthStatusRequest(provider: string, accountProfile = "default") {
  return {
    GetProviderAuthStatus: {
      provider,
      account_profile: accountProfile,
    },
  }
}

export function startProviderLoginRequest(
  provider: string,
  accountProfile = "default",
  method?: string | null,
) {
  return {
    StartProviderLogin: {
      provider,
      account_profile: accountProfile,
      ...(method ? { method } : {}),
    },
  }
}

export function getProviderLoginStatusRequest(loginId: string) {
  return { GetProviderLoginStatus: { login_id: loginId } }
}

export function sendProviderLoginInputRequest(loginId: string, dataBase64: string) {
  return { SendProviderLoginInput: { login_id: loginId, data_base64: dataBase64 } }
}

export function cancelProviderLoginRequest(loginId: string) {
  return { CancelProviderLogin: { login_id: loginId } }
}

export function logoutProviderRequest(provider: string, accountProfile = "default") {
  return {
    LogoutProvider: {
      provider,
      account_profile: accountProfile,
    },
  }
}

export function listProviderAccountProfilesRequest(provider?: string | null) {
  return { ListProviderAccountProfiles: { provider: provider ?? null } }
}

export function getProviderAccountProfileRequest(provider: string, accountProfile: string) {
  return { GetProviderAccountProfile: { provider, account_profile: accountProfile } }
}

export function createProviderAccountProfileRequest(provider: string, label: string) {
  return { CreateProviderAccountProfile: { provider, label } }
}

export function linkProviderAccountProfileRequest(provider: string, label: string, path: string) {
  return { LinkProviderAccountProfile: { provider, label, path } }
}

export function importNativeProviderAccountProfileRequest(provider: string) {
  return { ImportNativeProviderAccountProfile: { provider } }
}

export function renameProviderAccountProfileRequest(provider: string, accountProfile: string, label: string) {
  return { RenameProviderAccountProfile: { provider, account_profile: accountProfile, label } }
}

export function setDefaultProviderAccountProfileRequest(provider: string, accountProfile: string) {
  return { SetDefaultProviderAccountProfile: { provider, account_profile: accountProfile } }
}

export function refreshProviderAccountProfileRequest(provider: string, accountProfile: string) {
  return { RefreshProviderAccountProfile: { provider, account_profile: accountProfile } }
}

export function removeProviderAccountProfileRequest(provider: string, accountProfile: string) {
  return { RemoveProviderAccountProfile: { provider, account_profile: accountProfile } }
}

export function deleteProviderAccountProfileDataRequest(provider: string, accountProfile: string, confirmationProfileId: string) {
  return { DeleteProviderAccountProfileData: { provider, account_profile: accountProfile, confirmation_profile_id: confirmationProfileId } }
}

export type ProviderAccountCredentialRequestContext = {
  readonly sessionId?: string | null
  readonly agentId?: string | null
}

export function setProviderAccountCredentialRequest(
  provider: string,
  accountProfile: string,
  value: string,
  overwrite = false,
  context: ProviderAccountCredentialRequestContext = {},
) {
  return {
    SetProviderAccountCredential: {
      ...(context.sessionId ? { session_id: context.sessionId } : {}),
      ...(context.agentId ? { agent_id: context.agentId } : {}),
      provider,
      account_profile: accountProfile,
      value,
      overwrite,
    },
  }
}

export function launchProviderRunRequest(
  sessionId: string,
  provider: string,
  accountProfile: string,
  model: string,
  effort: string,
  agentId?: string | null,
  native?: {
    structuredEndpoint?: string | null
    providerSessionId?: string | null
    nativeTui?: boolean | null
  } | null,
) {
  const adapterKey = provider === "claude-headless" || provider === "claude-p"
    ? "claude"
    : provider
  const normalizedModel = normalizedProviderModel(provider, model)
  return {
    LaunchProviderRun: {
      session_id: sessionId,
      agent_id: agentId ?? null,
      adapter_key: adapterKey,
      provider,
      account_profile: accountProfile,
      model: normalizedModel,
      variant: effort.trim() || null,
      structured_endpoint: native?.structuredEndpoint ?? null,
      provider_session_id: native?.providerSessionId ?? null,
      native_tui: native?.nativeTui ?? false,
    },
  }
}

export type LaunchProviderRunBatchItem = {
  sessionId: string
  provider: string
  accountProfile: string
  model: string
  effort: string
  agentId?: string | null
  native?: {
    structuredEndpoint?: string | null
    providerSessionId?: string | null
    nativeTui?: boolean | null
  } | null
}

export function launchProviderRunsRequest(
  launches: LaunchProviderRunBatchItem[],
  maxConcurrency?: number | null,
) {
  return {
    LaunchProviderRuns: {
      max_concurrency: maxConcurrency ?? null,
      launches: launches.map((launch) => {
        const single = launchProviderRunRequest(
          launch.sessionId,
          launch.provider,
          launch.accountProfile,
          launch.model,
          launch.effort,
          launch.agentId,
          launch.native,
        )
        return single.LaunchProviderRun
      }),
    },
  }
}

function normalizedProviderModel(provider: string, model: string) {
  if (provider === "codex" && model.startsWith("codex/")) {
    return model.slice("codex/".length)
  }
  for (const claudeProvider of ["claude", "claude-headless", "claude-p"]) {
    if (provider === claudeProvider && model.startsWith(`${claudeProvider}/`)) {
      return model.slice(claudeProvider.length + 1)
    }
  }
  return model
}
