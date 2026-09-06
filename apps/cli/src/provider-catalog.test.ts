import assert from "node:assert/strict"
import test from "node:test"

import {
  backendProviderLabel,
  catalogModelOptions,
  fallbackProviderCatalog,
  normalizeBackendProviderId,
  providerCatalogIsLocalFallback,
  providerDisplayName,
  type ProviderCatalog,
} from "./provider-catalog.js"

test("providerDisplayName appends remote machine aliases", () => {
  assert.equal(
    providerDisplayName({
      id: "codex",
      name: "Codex",
      remote_machine_aliases: ["builder-west"],
      models: {},
    }),
    "Codex (builder-west)",
  )
})

test("catalogModelOptions uses remote machine qualified provider names", () => {
  const catalog: ProviderCatalog = {
    all: [
      {
        id: "codex",
        name: "Codex",
        remote_machine_aliases: ["builder-west"],
        models: {
          "gpt-5.4": {
            id: "gpt-5.4",
            name: "GPT-5.4",
            status: "active",
            variants: { high: {} },
          },
        },
      },
    ],
    default: {
      codex: "gpt-5.4",
    },
    connected: ["codex"],
  }

  const options = catalogModelOptions(catalog, "codex")
  assert.equal(options.length, 1)
  assert.equal(options[0]?.providerName, "Codex (builder-west)")
})

test("fallback catalog exposes Claude headless and Claude -p as isolated backends", () => {
  const catalog = fallbackProviderCatalog()

  assert.equal(backendProviderLabel("claude-headless"), "Claude headless")
  assert.equal(backendProviderLabel("claude-p"), "Claude -p")
  assert.equal(normalizeBackendProviderId("claude"), "claude-p")

  const claudeHeadlessOptions = catalogModelOptions(catalog, "claude-headless")
  assert.deepEqual(claudeHeadlessOptions.map((option) => option.providerId), ["claude-headless"])
  assert.deepEqual(claudeHeadlessOptions.map((option) => option.id), ["claude-headless/claude-sonnet-4-6"])

  const claudePrintOptions = catalogModelOptions(catalog, "claude-p")
  assert.deepEqual(claudePrintOptions.map((option) => option.providerId), ["claude-p"])
  assert.deepEqual(claudePrintOptions.map((option) => option.id), ["claude-p/claude-sonnet-4-6"])

  const opencodeOptions = catalogModelOptions(catalog, "opencode")
  assert.equal(opencodeOptions.some((option) => option.providerId.startsWith("claude")), false)
})

test("OpenCode model selection includes Go, Zen, and arbitrary upstream providers", () => {
  const catalog: ProviderCatalog = {
    all: [
      provider("opencode-go", "OpenCode Go", "deepseek-v4-pro"),
      provider("opencode", "OpenCode Zen", "gpt-5.2"),
      provider("openai", "OpenAI", "gpt-5.2"),
      provider("codex", "Codex", "gpt-5.6"),
    ],
    default: {},
    connected: ["opencode-go", "opencode", "openai", "codex"],
  }

  assert.deepEqual(
    catalogModelOptions(catalog, "opencode").map((option) => option.providerId).sort(),
    ["openai", "opencode", "opencode-go"],
  )
})

function provider(id: string, name: string, modelId: string) {
  return {
    id,
    name,
    remote_machine_aliases: [],
    models: {
      [modelId]: { id: modelId, name: modelId, status: "active", variants: {} },
    },
  }
}

test("fallback catalog can be marked as local fallback metadata", () => {
  const catalog = fallbackProviderCatalog({
    source: "local_fallback",
    unavailableReason: "provider catalog unavailable",
  })

  assert.equal(providerCatalogIsLocalFallback(catalog), true)
  assert.equal(catalog.source, "local_fallback")
  assert.equal(catalog.unavailable_reason, "provider catalog unavailable")
  assert.equal(providerCatalogIsLocalFallback(fallbackProviderCatalog()), false)
})
