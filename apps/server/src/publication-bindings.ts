import { readFile, writeFile } from "node:fs/promises"
import { createInterface } from "node:readline/promises"
import process from "node:process"

import { getProviderCatalogRequest } from "@chariox/kernel-client/ipc-requests"
import {
  workflowPublicationAllowedProviders,
  type WorkflowPublicationDeploymentContract,
} from "@chariox/kernel-client/workflow-publication-deployment-contract"

import type {
  KernelLookupClient,
  PublicationProviderModelProfile,
  WorkflowPublicationBindings,
  WorkflowPublicationSnapshot,
} from "./publication-types.js"

export type ProviderModelBindingPrompt = (request: {
  agent_id: string
  captured: PublicationProviderModelProfile
  available: ProviderCatalogIndex
}) => Promise<PublicationProviderModelProfile>

export async function resolvePublicationProviderModelBindings(
  snapshot: WorkflowPublicationSnapshot,
  bindingsPath: string,
  client: KernelLookupClient,
  options: {
    deploymentContract?: WorkflowPublicationDeploymentContract
    promptReplacement?: ProviderModelBindingPrompt | false
  } = {},
) {
  const catalog = await providerCatalogIndex(client)
  const bindings = await loadPublicationBindings(bindingsPath, snapshot)
  let changed = false
  for (const agent of snapshot.agents ?? []) {
    const binding = bindingForAgent(bindings, snapshot, agent)
    const selected = binding.replacement ?? binding.captured
    const allowedProviders = new Set(options.deploymentContract
      ? workflowPublicationAllowedProviders(options.deploymentContract, agent.id, binding.captured.provider)
      : [binding.captured.provider])
    if (!allowedProviders.has(selected.provider)) {
      throw new Error(`publication provider is not permitted for agent ${agent.id}: ${selected.provider}`)
    }
    const allowedCatalog = providerCatalogForAllowedProviders(catalog, allowedProviders)
    const selectedProfile = binding.replacement
      ? availableProviderProfile(allowedCatalog, selected)
      : internalDevelopmentAdapterProfile(selected) ?? availableProviderProfile(allowedCatalog, selected)
    if (selectedProfile) {
      applyAgentProfile(agent, selectedProfile)
      continue
    }
    const promptReplacement = options.promptReplacement ?? promptProviderModelReplacement
    if (promptReplacement === false) {
      throw new Error(`publication provider/model is unavailable for agent ${agent.id}: ${profileLabel(selected)}`)
    }
    const replacement = await promptReplacement({
      agent_id: agent.id,
      captured: binding.captured,
      available: allowedCatalog,
    })
    if (!allowedProviders.has(replacement.provider)) {
      throw new Error(`publication provider replacement is not permitted for agent ${agent.id}: ${replacement.provider}`)
    }
    const replacementProfile = availableProviderProfile(allowedCatalog, replacement)
    if (!replacementProfile) {
      throw new Error(`publication provider/model replacement is unavailable for agent ${agent.id}: ${profileLabel(replacement)}`)
    }
    binding.replacement = replacementProfile
    applyAgentProfile(agent, replacementProfile)
    changed = true
  }
  if (changed) {
    await writeFile(bindingsPath, `${JSON.stringify(bindings, null, 2)}\n`)
  }
  return { snapshot, bindings, changed }
}

function internalDevelopmentAdapterProfile(
  profile: PublicationProviderModelProfile,
): PublicationProviderModelProfile | null {
  return profile.provider === "dev-stub" ? profile : null
}

function providerCatalogForAllowedProviders(
  catalog: ProviderCatalogIndex,
  allowedProviders: ReadonlySet<string>,
): ProviderCatalogIndex {
  const providers = new Map<string, Set<string>>()
  for (const provider of allowedProviders) {
    const models = providerFamilyModels(catalog, provider)
    if (models) providers.set(provider, models)
    if (provider === "opencode") {
      const goModels = catalog.providers.get("opencode-go")
      if (goModels) providers.set("opencode-go", goModels)
    }
  }
  return { providers }
}

export type ProviderCatalogIndex = {
  providers: Map<string, Set<string>>
}

async function providerCatalogIndex(client: KernelLookupClient): Promise<ProviderCatalogIndex> {
  const response = await client.send(getProviderCatalogRequest())
  const catalog = (response.ProviderCatalog as { catalog?: { all?: unknown[] } } | undefined)?.catalog
  const providers = new Map<string, Set<string>>()
  for (const provider of catalog?.all ?? []) {
    if (!provider || typeof provider !== "object" || Array.isArray(provider)) continue
    const record = provider as Record<string, unknown>
    if (typeof record.id !== "string" || !record.id.trim()) continue
    const models = new Set<string>()
    if (record.models && typeof record.models === "object" && !Array.isArray(record.models)) {
      for (const modelId of Object.keys(record.models)) {
        if (modelId.trim()) models.add(modelId)
      }
    }
    providers.set(record.id, models)
  }
  return { providers }
}

async function loadPublicationBindings(
  bindingsPath: string,
  snapshot: WorkflowPublicationSnapshot,
): Promise<WorkflowPublicationBindings> {
  try {
    const bindings = JSON.parse(await readFile(bindingsPath, "utf8")) as WorkflowPublicationBindings
    if (bindings.schema_version !== 1) {
      throw new Error(`unsupported publication bindings schema_version ${bindings.schema_version}`)
    }
    return bindings
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error
  }
  return {
    schema_version: 1,
    provider_model_overrides: (snapshot.agents ?? []).map((agent) => ({
      agent_id: agent.id,
      node_ids: (snapshot.workflow.nodes ?? [])
        .filter((node) => node.agent_id === agent.id)
        .map((node) => node.id),
      captured: {
        provider: agent.provider,
        model: agent.model ?? null,
        effort: agent.effort ?? null,
      },
      replacement: null,
    })),
  }
}

function bindingForAgent(
  bindings: WorkflowPublicationBindings,
  snapshot: WorkflowPublicationSnapshot,
  agent: NonNullable<WorkflowPublicationSnapshot["agents"]>[number],
) {
  bindings.provider_model_overrides ??= []
  let binding = bindings.provider_model_overrides.find((candidate) => candidate.agent_id === agent.id)
  if (!binding) {
    binding = {
      agent_id: agent.id,
      node_ids: (snapshot.workflow.nodes ?? [])
        .filter((node) => node.agent_id === agent.id)
        .map((node) => node.id),
      captured: {
        provider: agent.provider,
        model: agent.model ?? null,
        effort: agent.effort ?? null,
      },
      replacement: null,
    }
    bindings.provider_model_overrides.push(binding)
  }
  return binding
}

function availableProviderProfile(catalog: ProviderCatalogIndex, profile: PublicationProviderModelProfile): PublicationProviderModelProfile | null {
  if (profile.provider === "opencode" && profile.model?.startsWith("opencode-go/")) {
    const models = catalog.providers.get("opencode-go")
    if (!models) return null
    const model = profile.model.slice("opencode-go/".length)
    return models.size === 0 || models.has(model) ? profile : null
  }
  const models = providerFamilyModels(catalog, profile.provider)
  if (!models) return null
  if (profile.model === "default" || profile.model === `${profile.provider}/default`) {
    return { ...profile, model: null }
  }
  if (!profile.model) return profile
  const canonicalProfile = canonicalProviderModelProfile(profile)
  if (models.size === 0 || models.has(profile.model)) return canonicalProfile
  const providerPrefixedModel = `${profile.provider}/`
  if (profile.model.startsWith(providerPrefixedModel)) {
    const unprefixedModel = profile.model.slice(providerPrefixedModel.length)
    if (models.has(unprefixedModel)) {
      return profile.provider === "opencode"
        ? canonicalProfile
        : { ...profile, model: unprefixedModel }
    }
  }
  return null
}

function providerFamilyModels(catalog: ProviderCatalogIndex, provider: string): Set<string> | null {
  const direct = catalog.providers.get(provider)
  if (direct) return direct
  if (provider !== "claude") return null
  const models = new Set<string>()
  for (const alias of ["claude-headless", "claude-p"]) {
    for (const model of catalog.providers.get(alias) ?? []) models.add(model)
  }
  return models.size > 0 ? models : null
}

function canonicalProviderModelProfile(profile: PublicationProviderModelProfile): PublicationProviderModelProfile {
  if (profile.provider !== "opencode" || !profile.model || profile.model.includes("/")) return profile
  return { ...profile, model: `opencode/${profile.model}` }
}

function applyAgentProfile(agent: NonNullable<WorkflowPublicationSnapshot["agents"]>[number], profile: PublicationProviderModelProfile) {
  agent.provider = profile.provider
  agent.model = profile.model ?? null
  agent.effort = profile.effort ?? null
  if (profile.account_profile) agent.account_profile = profile.account_profile
}

async function promptProviderModelReplacement({
  agent_id,
  captured,
  available,
}: {
  agent_id: string
  captured: PublicationProviderModelProfile
  available: ProviderCatalogIndex
}) {
  if (!process.stdin.isTTY) {
    throw new Error(`publication provider/model is unavailable for agent ${agent_id}: ${profileLabel(captured)}`)
  }
  const choices = [...available.providers.entries()]
    .flatMap(([provider, models]) => {
      if (models.size === 0) return [provider]
      return [...models].map((model) => `${provider}/${model}`)
    })
    .join(", ")
  const readline = createInterface({ input: process.stdin, output: process.stderr })
  try {
    process.stderr.write(`Captured provider/model for published workflow agent ${agent_id} is unavailable: ${profileLabel(captured)}\n`)
    process.stderr.write(`Available provider/model choices: ${choices || "(none)"}\n`)
    const provider = (await readline.question("Replacement provider: ")).trim()
    const model = (await readline.question("Replacement model (blank for provider default): ")).trim()
    const effort = (await readline.question("Replacement effort (blank to keep unset): ")).trim()
    return {
      provider,
      model: model || null,
      effort: effort || null,
    }
  } finally {
    readline.close()
  }
}

function profileLabel(profile: PublicationProviderModelProfile) {
  const model = profile.model ? `/${profile.model}` : ""
  const effort = profile.effort ? ` effort=${profile.effort}` : ""
  return `${profile.provider}${model}${effort}`
}
