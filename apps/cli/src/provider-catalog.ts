export type ProviderCatalog = {
  all: ProviderInfo[]
  default: Record<string, string>
  connected: string[]
  source?: "daemon" | "local_fallback"
  unavailable_reason?: string
}

export type ProviderInfo = {
  id: string
  name: string
  remote_machine_aliases?: string[]
  models: Record<string, ProviderModel>
}

export type ProviderModel = {
  id: string
  name: string
  status: string
  limit?: {
    context: number
    input?: number
    output?: number
  }
  variants?: Record<string, unknown>
}

export type CatalogModelOption = {
  id: string
  providerId: string
  providerName: string
  label: string
  variants: string[]
}

export const BACKEND_PROVIDER_IDS = ["opencode", "codex", "claude-headless", "claude-p"] as const

export type BackendProviderId = typeof BACKEND_PROVIDER_IDS[number]

export function isBackendProviderId(value: string): value is BackendProviderId {
  return (BACKEND_PROVIDER_IDS as readonly string[]).includes(value)
}

export function normalizeBackendProviderId(value: string): BackendProviderId {
  if (value === "claude") {
    return "claude-p"
  }
  return isBackendProviderId(value) ? value : "opencode"
}

export function backendProviderLabel(providerId: BackendProviderId) {
  switch (providerId) {
    case "codex":
      return "Codex"
    case "claude-headless":
      return "Claude headless"
    case "claude-p":
      return "Claude -p"
    case "opencode":
      return "OpenCode"
  }
}

export function fallbackProviderCatalog(options: {
  source?: "local_fallback"
  unavailableReason?: string
} = {}) {
  return {
    all: [
      {
        id: "codex",
        name: "Codex",
        remote_machine_aliases: [],
        models: {
          "gpt-5.4": {
            id: "gpt-5.4",
            name: "GPT-5.4",
            status: "active",
            variants: {
              low: {},
              medium: {},
              high: {},
              xhigh: {},
            },
          },
        },
      },
      {
        id: "opencode",
        name: "OpenCode",
        remote_machine_aliases: [],
        models: {
          "gpt-5.4": {
            id: "gpt-5.4",
            name: "GPT-5.4",
            status: "active",
            variants: {
              low: {},
              medium: {},
              high: {},
            },
          },
        },
      },
      {
        id: "claude-headless",
        name: "Claude headless",
        remote_machine_aliases: [],
        models: {
          "claude-sonnet-4-6": {
            id: "claude-sonnet-4-6",
            name: "Claude Sonnet 4.6",
            status: "active",
            variants: {
              low: {},
              medium: {},
              high: {},
              xhigh: {},
              max: {},
            },
          },
        },
      },
      {
        id: "claude-p",
        name: "Claude -p",
        remote_machine_aliases: [],
        models: {
          "claude-sonnet-4-6": {
            id: "claude-sonnet-4-6",
            name: "Claude Sonnet 4.6",
            status: "active",
            variants: {
              low: {},
              medium: {},
              high: {},
              xhigh: {},
              max: {},
            },
          },
        },
      },
    ],
    default: {
      codex: "gpt-5.4",
      opencode: "gpt-5.4",
      "claude-headless": "claude-sonnet-4-6",
      "claude-p": "claude-sonnet-4-6",
    },
    connected: ["codex", "opencode", "claude-headless", "claude-p"],
    ...(options.source ? { source: options.source } : {}),
    ...(options.unavailableReason ? { unavailable_reason: options.unavailableReason } : {}),
  } satisfies ProviderCatalog
}

export function providerCatalogIsLocalFallback(catalog: ProviderCatalog) {
  return catalog.source === "local_fallback"
}

export function catalogModelOptions(catalog: ProviderCatalog, backendProviderId?: BackendProviderId) {
  const connectedProviderIds = new Set(catalog.connected)
  return catalog.all
    .filter((provider) => (
      connectedProviderIds.has(provider.id)
      && providerBelongsToBackend(provider.id, backendProviderId)
    ))
    .flatMap((provider) =>
      Object.values(provider.models)
        .filter((model) => model.status !== "deprecated")
        .map((model) => ({
          id: `${provider.id}/${model.id}`,
          providerId: provider.id,
          providerName: providerDisplayName(provider),
          label: model.name || model.id,
          variants: Object.keys(model.variants ?? {}),
        })),
    )
    .sort((left, right) => {
      if (left.providerId === right.providerId) {
        return left.label.localeCompare(right.label)
      }
      return left.providerName.localeCompare(right.providerName)
    })
}

export function providerDisplayName(provider: ProviderInfo) {
  const remoteAliases = (provider.remote_machine_aliases ?? [])
    .map((alias) => alias.trim())
    .filter(Boolean)
  if (remoteAliases.length === 0) {
    return provider.name
  }
  return `${provider.name} (${remoteAliases.join(", ")})`
}

export function selectConfiguredModel(
  catalog: ProviderCatalog,
  configured?: string | null,
  backendProviderId?: BackendProviderId,
) {
  const options = catalogModelOptions(catalog, backendProviderId)
  if (options.length === 0) {
    return null
  }
  const exact = configured ? options.find((option) => option.id === configured) : null
  if (exact) {
    return exact
  }
  const unqualified = configured
    ? options.find((option) => option.id.endsWith(`/${configured}`))
    : null
  if (unqualified) {
    return unqualified
  }
  for (const provider of catalog.all) {
    if (!providerBelongsToBackend(provider.id, backendProviderId)) {
      continue
    }
    const modelId = catalog.default[provider.id]
    if (!modelId) {
      continue
    }
    const match = options.find((option) => option.id === `${provider.id}/${modelId}`)
    if (match) {
      return match
    }
  }
  return options[0] ?? null
}

export function selectConfiguredVariant(option: CatalogModelOption | null, configured?: string | null) {
  if (!option || option.variants.length === 0) {
    return ""
  }
  if (configured && option.variants.includes(configured)) {
    return configured
  }
  return option.variants.includes("high") ? "high" : option.variants[0]!
}

function providerBelongsToBackend(
  providerId: string,
  backendProviderId?: BackendProviderId,
) {
  if (!backendProviderId) {
    return true
  }
  if (backendProviderId === "codex") {
    return providerId === "codex"
  }
  if (backendProviderId === "claude-headless") {
    return providerId === "claude-headless"
  }
  if (backendProviderId === "claude-p") {
    return providerId === "claude-p" || providerId === "claude"
  }
  return providerId !== "codex"
    && providerId !== "claude"
    && providerId !== "claude-headless"
    && providerId !== "claude-p"
    && providerId !== "dev-stub"
}
