import {
  backendProviderLabel,
  catalogModelOptions,
  providerCatalogIsLocalFallback,
  type BackendProviderId,
} from "./provider-catalog.js"
import {
  type ProviderCommandCatalogs,
  providerCommandCatalogIsLocalFallback,
  providerNamespace,
  providerNamespaceDescription,
} from "./provider-command-catalog.js"
import { filterCommandCenterItems } from "./command-center-search.js"
import { mapNodeToItem, type CommandNode } from "./command-center-tree-projection.js"
import type { CommandCenterDynamicContext } from "./command-center-context.js"
import type { CommandCenterItem } from "./command-center-types.js"
import {
  providerAccountCapacity,
  providerAccountDisplayLabel,
  providerAccountsForProvider,
} from "./waiting-room-provider-accounts.js"

export function providerNamespaceRootItem(
  provider: BackendProviderId,
  catalogs: ProviderCommandCatalogs,
): CommandCenterItem {
  const catalog = catalogs[provider] ?? emptyProviderCommandCatalog(provider)
  const localFallback = providerCommandCatalogIsLocalFallback(catalog)
  return {
    id: `provider-namespace-${provider}`,
    label: providerNamespace(provider),
    description: providerNamespaceDescription(provider, catalog.commands.length, { localFallback }),
    kind: "group",
    value: `${providerNamespace(provider)} `,
    ...(catalog.commands.length > 0
      ? {
        searchAliases: catalog.commands.flatMap((command) => [command.name, command.description, command.value]),
      }
      : {}),
  }
}

export function providerNamespaceScopeNode(
  provider: BackendProviderId,
  catalogs: ProviderCommandCatalogs,
): CommandNode {
  const catalog = catalogs[provider] ?? emptyProviderCommandCatalog(provider)
  const localFallback = providerCommandCatalogIsLocalFallback(catalog)
  return {
    id: `provider-namespace-${provider}`,
    label: providerNamespace(provider),
    description: providerNamespaceDescription(provider, catalog.commands.length, { localFallback }),
    value: `${providerNamespace(provider)} `,
  }
}

export function buildProviderNamespaceItems(
  input: string,
  provider: BackendProviderId,
  catalogs: ProviderCommandCatalogs,
) {
  const catalog = catalogs[provider] ?? emptyProviderCommandCatalog(provider)
  const localFallback = providerCommandCatalogIsLocalFallback(catalog)
  const namespace = providerNamespace(provider)
  const rootItem: CommandCenterItem = {
    id: `provider-namespace-${provider}`,
    label: namespace,
    description: catalog.commands.length > 0
      ? providerNamespaceDescription(provider, catalog.commands.length, { localFallback })
      : `${providerNamespaceDescription(provider, 0, { localFallback })}; no registered completions for this provider version yet`,
    kind: "group",
    value: `${namespace} `,
  }
  if (input === namespace || input === `${namespace} `) {
    return catalog.commands.length > 0
      ? catalog.commands.map((command) => ({
        id: `${provider}-${command.id}`,
        label: command.name,
        description: command.description,
        kind: "command" as const,
        value: `${namespace} ${command.value}`,
      }))
      : [rootItem]
  }
  const query = input.slice(`${namespace} `.length).trim().toLowerCase()
  if (catalog.commands.length === 0) {
    return filterCommandCenterItems([rootItem], query)
  }
  return filterCommandCenterItems(
    catalog.commands.map((command) => ({
      id: `${provider}-${command.id}`,
      label: command.name,
      description: command.description,
      kind: "command" as const,
      value: `${namespace} ${command.value}`,
    })),
    query,
  )
}

export function buildProviderItems(input: string, providerNode: CommandNode, context: CommandCenterDynamicContext) {
  const query = input.slice("/provider ".length).trim().toLowerCase()
  const localFallback = providerCatalogIsLocalFallback(context.providerCatalog)
  return filterCommandCenterItems([
    mapNodeToItem(providerNode),
    {
      id: "provider-opencode",
      label: "OpenCode",
      description: providerSelectionDescription("OpenCode", localFallback),
      kind: "provider",
      value: "opencode",
    },
    {
      id: "provider-codex",
      label: "Codex",
      description: providerSelectionDescription("Codex", localFallback),
      kind: "provider",
      value: "codex",
    },
    {
      id: "provider-claude-headless",
      label: backendProviderLabel("claude-headless"),
      description: providerSelectionDescription("Claude headless", localFallback),
      kind: "provider",
      value: "claude-headless",
    },
    {
      id: "provider-claude-p",
      label: backendProviderLabel("claude-p"),
      description: providerSelectionDescription("Claude -p", localFallback),
      kind: "provider",
      value: "claude-p",
    },
    {
      id: "provider-status",
      label: "status",
      description: "Show auth status for the current or named provider",
      kind: "command",
      value: "/provider status ",
    },
    {
      id: "provider-login",
      label: "login",
      description: "Start login for the current or named provider",
      kind: "command",
      value: "/provider login ",
    },
    {
      id: "provider-logout",
      label: "logout",
      description: "Log out the current or named provider",
      kind: "command",
      value: "/provider logout ",
    },
    {
      id: "provider-reauth",
      label: "reauth",
      description: "Log out and start a fresh login for the current or named provider",
      kind: "command",
      value: "/provider reauth ",
    },
    {
      id: "provider-processes",
      label: "processes",
      description: "List daemon-tracked provider processes",
      kind: "command",
      value: "/provider processes ",
    },
    {
      id: "provider-processes-teardown",
      label: "teardown",
      description: "Tear down safe daemon-tracked provider processes for one provider",
      kind: "command",
      value: "/provider processes teardown ",
    },
  ], query)
}

export function buildModelItems(input: string, context: CommandCenterDynamicContext) {
  const query = input.slice("/model ".length).trim().toLowerCase()
  const localFallback = providerCatalogIsLocalFallback(context.providerCatalog)
  return filterCommandCenterItems(
    catalogModelOptions(context.providerCatalog, context.currentProvider).map((option) => ({
      id: `model-${option.id}`,
      label: `${option.providerName} ${option.label}`,
      description: modelSelectionDescription(option.id, option.id === context.currentModel, localFallback),
      kind: "model" as const,
      value: option.id,
    })),
    query,
  )
}

export function buildAccountItems(input: string, context: CommandCenterDynamicContext) {
  const query = input.slice("/account ".length).trim().toLowerCase()
  return filterCommandCenterItems(
    providerAccountsForProvider(context.providerAccounts ?? [], context.currentProvider).map((profile) => {
      const capacity = providerAccountCapacity(profile, Date.now(), context.currentModel)
      return {
        id: `account-${profile.provider}-${profile.profile_id}`,
        label: providerAccountDisplayLabel(profile, context.currentModel),
        description: profile.profile_id === (context.currentAccount ?? "default")
          ? `current account · ${capacity.detail}`
          : capacity.detail,
        kind: "account" as const,
        value: profile.profile_id,
        ...(capacity.state === "exhausted"
          ? { tone: "danger" as const }
          : capacity.state === "warning"
            ? { tone: "warning" as const }
            : {}),
      }
    }),
    query,
  )
}

export function buildVariantItems(input: string, context: CommandCenterDynamicContext) {
  const query = input.slice("/variant ".length).trim().toLowerCase()
  const localFallback = providerCatalogIsLocalFallback(context.providerCatalog)
  const current = catalogModelOptions(context.providerCatalog, context.currentProvider).find((option) => option.id === context.currentModel)
  const variants = current?.variants ?? []
  return filterCommandCenterItems(
    variants.map((variant) => ({
      id: `variant-${variant}`,
      label: variant,
      description: variantSelectionDescription(
        current?.label ?? context.currentModel,
        variant === context.currentVariant,
        localFallback,
      ),
      kind: "variant" as const,
      value: variant,
    })),
    query,
  )
}

export function buildViewItems(input: string) {
  const query = input.slice("/view ".length).trim().toLowerCase()
  return filterCommandCenterItems([
    {
      id: "view-individual",
      label: "individual",
      description: "Show one focused agent transcript at a time",
      kind: "command",
      value: "/view individual",
    },
    {
      id: "view-split",
      label: "split",
      description: "Split the response area across active session agents",
      kind: "command",
      value: "/view split",
    },
  ], query)
}

function emptyProviderCommandCatalog(provider: BackendProviderId) {
  return {
    provider,
    source: "shipped" as const,
    discovery: "none" as const,
    commands: [],
  }
}

function providerSelectionDescription(providerName: string, localFallback: boolean) {
  return localFallback ? `Use the ${providerName} backend; local provider list` : `Use the ${providerName} backend`
}

function modelSelectionDescription(modelId: string, current: boolean, localFallback: boolean) {
  const base = current ? "current model" : modelId
  return localFallback ? `${base}; local provider list` : base
}

function variantSelectionDescription(modelLabel: string, current: boolean, localFallback: boolean) {
  const base = `${modelLabel}${current ? " • current" : ""}`
  return localFallback ? `${base}; local provider list` : base
}
