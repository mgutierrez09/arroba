import assert from "node:assert/strict"
import test from "node:test"

import {
  buildAccountItems,
  buildModelItems,
  buildProviderItems,
  buildProviderNamespaceItems,
  buildVariantItems,
  buildViewItems,
  providerNamespaceRootItem,
} from "./command-center-dynamic-items.js"
import { loadCommandCenterTestCatalog } from "./command-center-test-catalog.js"
import { fallbackProviderCatalog } from "./provider-catalog.js"
import { fallbackProviderCommandCatalogs, type ProviderCommandCatalogs } from "./provider-command-catalog.js"

const commandTree = loadCommandCenterTestCatalog()

test("command center dynamic items project provider namespaces and completions", () => {
  const catalogs = withCodexCommand()

  assert.equal(providerNamespaceRootItem("codex", catalogs).searchAliases?.includes("resume"), true)
  assert.deepEqual(
    buildProviderNamespaceItems("/codex ", "codex", catalogs).map((item) => item.value),
    ["/codex resume "],
  )
})

test("command center marks provider namespace command lists from local fallback", () => {
  const catalogs = fallbackProviderCommandCatalogs({ catalogSource: "local_fallback" })

  assert.match(providerNamespaceRootItem("codex", catalogs).description, /local command list/)
  assert.match(buildProviderNamespaceItems("/codex ", "codex", catalogs)[0]?.description ?? "", /local command list/)
})

test("command center dynamic items project provider, model, variant, and view choices", () => {
  const providerNode = commandTree.find((node) => node.id === "provider")!
  const context = {
    providerCatalog: fallbackProviderCatalog(),
    currentProvider: "opencode" as const,
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
  }

  assert.deepEqual(
    buildProviderItems("/provider cla", providerNode, context)
      .filter((item) => item.kind === "provider")
      .map((item) => item.value)
      .sort(),
    ["claude-headless", "claude-p"],
  )
  assert.equal(buildProviderItems("/provider proc", providerNode, context).some((item) => item.value === "/provider processes "), true)
  const teardownItem = buildProviderItems("/provider teardown", providerNode, context).find((item) => item.value === "/provider processes teardown ")
  assert.equal(teardownItem?.description, "Tear down safe daemon-tracked provider processes for one provider")
  assert.equal(buildModelItems("/model gpt", context)[0]?.kind, "model")
  assert.equal(buildVariantItems("/variant med", context)[0]?.value, "medium")
  assert.equal(buildViewItems("/view spl")[0]?.value, "/view split")
})

test("command center account choices display labels but execute stable profile ids", () => {
  const items = buildAccountItems("/account val", {
    providerCatalog: fallbackProviderCatalog(),
    providerAccounts: [{
      provider: "codex",
      profile_id: "secondary",
      label: "Validation",
      identity_summary: "validation@example.com",
      auth_state: "authenticated",
      is_default: false,
    } as never],
    currentProvider: "codex",
    currentAccount: "default",
    currentModel: "codex/gpt-5.6-luna",
    currentVariant: "low",
  })

  assert.deepEqual(items.map((item) => ({ label: item.label, value: item.value })), [{
    label: "Validation",
    value: "secondary",
  }])
})

test("command center marks hard account exhaustion red and allowance-only Claude exhaustion amber", () => {
  const nowMs = Date.now()
  const context = {
    providerCatalog: fallbackProviderCatalog(),
    currentProvider: "codex" as const,
    currentAccount: "default",
    currentModel: "codex/gpt-5.6-luna",
    currentVariant: "low",
  }
  const allowance = {
    meter_id: "allowance",
    label: "Allowance",
    kind: "rolling_limit",
    scope: "plan",
    state: "exhausted",
    source: "test",
    observed_at_ms: nowMs,
    resets_at_ms: nowMs + 60_000,
  }
  const account = {
    provider: "codex",
    profile_id: "codex-account",
    label: "Codex account",
    auth_state: "authenticated",
    is_default: true,
    usage: {
      profile_id: "codex-account",
      provider: "codex",
      availability: "available",
      meters: [allowance],
      observed_at_ms: nowMs,
      source: "test",
    },
  }

  const allowanceOnly = buildAccountItems("/account ", {
    ...context,
    providerAccounts: [account] as never,
  })[0]
  assert.equal(allowanceOnly?.label, "Codex account")
  assert.equal(allowanceOnly?.tone, "warning")
  assert.match(allowanceOnly?.description ?? "", /credits not reported/)

  const fullyExhausted = buildAccountItems("/account ", {
    ...context,
    providerAccounts: [{
      ...account,
      usage: {
        ...account.usage,
        meters: [allowance, {
          ...allowance,
          meter_id: "credits",
          label: "Credits",
          kind: "credit_balance",
          scope: "account",
          resets_at_ms: undefined,
        }],
      },
    }] as never,
  })[0]
  assert.equal(fullyExhausted?.label, "Codex account (exhausted)")
  assert.equal(fullyExhausted?.tone, "danger")
  assert.match(fullyExhausted?.description ?? "", /allowance and credits exhausted/)
})

test("command center marks provider and model choices from local fallback provider catalog", () => {
  const providerNode = commandTree.find((node) => node.id === "provider")!
  const context = {
    providerCatalog: fallbackProviderCatalog({ source: "local_fallback" }),
    currentProvider: "opencode" as const,
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
  }

  assert.match(buildProviderItems("/provider codex", providerNode, context)[0]?.description ?? "", /local provider list/)
  assert.match(buildModelItems("/model gpt", context)[0]?.description ?? "", /local provider list/)
  assert.match(buildVariantItems("/variant high", context)[0]?.description ?? "", /local provider list/)
})

function withCodexCommand(): ProviderCommandCatalogs {
  const catalogs = fallbackProviderCommandCatalogs()
  catalogs.codex = {
    ...catalogs.codex,
    commands: [{
      id: "resume",
      name: "resume",
      description: "Resume the focused provider turn",
      value: "resume ",
    }],
  }
  return catalogs
}
