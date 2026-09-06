import assert from "node:assert/strict"
import test from "node:test"

import type {
  ProviderAccountProfile,
  ProviderAccountUsageMeter,
} from "@chariox/kernel-client"
import {
  defaultProviderAccountProfileId,
  providerAccountFamily,
  providerAccountDisplayLabel,
  providerAccountsForProvider,
  resolveProviderAccountSelection,
  selectedProviderAccount,
} from "./waiting-room-provider-accounts.js"

const profiles: ProviderAccountProfile[] = [
  account("claude", "claude-primary", true),
  account("claude", "claude-secondary"),
  account("codex", "codex-primary", true),
]

test("Claude adapters share the Claude provider-account family", () => {
  assert.equal(providerAccountFamily("claude-headless"), "claude")
  assert.equal(providerAccountFamily("claude-p"), "claude")
  assert.deepEqual(
    providerAccountsForProvider(profiles, "claude-headless").map((profile) => profile.profile_id),
    ["claude-primary", "claude-secondary"],
  )
})

test("provider account selection stays isolated to the selected provider family", () => {
  assert.equal(
    selectedProviderAccount(profiles, "claude-p", "claude-secondary")?.profile_id,
    "claude-secondary",
  )
  assert.equal(selectedProviderAccount(profiles, "claude-p", "codex-primary"), null)
  assert.deepEqual(
    providerAccountsForProvider(profiles, "codex").map((profile) => profile.profile_id),
    ["codex-primary"],
  )
  assert.equal(selectedProviderAccount(profiles, "claude-headless", "default")?.profile_id, "claude-primary")
  assert.equal(selectedProviderAccount(profiles, "codex", undefined)?.profile_id, "codex-primary")
})

test("a missing default stays unavailable instead of silently selecting the first account", () => {
  const accounts = [account("codex", "codex-secondary")]
  assert.equal(defaultProviderAccountProfileId(accounts, "codex"), "default")
  assert.equal(selectedProviderAccount(accounts, "codex", "default"), null)
})

test("provider account display uses only the public alias", () => {
  const profile = account("codex", "internal-profile", true)
  profile.label = "codex-1"
  profile.identity_summary = "owner@example.com"

  assert.equal(providerAccountDisplayLabel(profile), "codex-1")
})

test("TUI marks hard exhaustion without hiding Claude and Codex accounts backed by credits", () => {
  const resetsAtMs = Date.now() + 60_000
  const allowance = usageMeter("allowance", "rolling_limit", "exhausted", resetsAtMs)
  const credits = usageMeter("credits", "credit_balance", "exhausted")

  assert.equal(
    providerAccountDisplayLabel(accountWithUsage("opencode", [allowance])),
    "Account",
    "unscoped exhaustion must not mark every OpenCode service exhausted",
  )
  assert.equal(
    providerAccountDisplayLabel(accountWithUsage("opencode", [
      { ...allowance, service_id: "opencode-go" },
    ]), "opencode-go/test-model"),
    "Account · OpenCode Go (exhausted)",
  )
  for (const provider of ["claude", "claude-headless", "claude-p", "codex"]) {
    assert.equal(
      providerAccountDisplayLabel(accountWithUsage(provider, [allowance])),
      "Account",
      `${provider} allowance exhaustion alone must remain selectable in the TUI`,
    )
    assert.equal(
      providerAccountDisplayLabel(accountWithUsage(provider, [allowance, credits])),
      "Account (exhausted)",
      `${provider} must be marked exhausted when allowance and credits are exhausted`,
    )
    assert.equal(
      providerAccountDisplayLabel(accountWithUsage(provider, [allowance, credits, {
        ...credits,
        meter_id: "extra-spend",
        kind: "spend_limit",
        state: "healthy",
        observed_at_ms: Date.now() - 86_400_001,
      }])),
      "Account",
      `${provider} must not discard a stale credit meter to claim total exhaustion`,
    )
  }
})

test("provider account selection accepts public aliases while preserving internal id support", () => {
  const primary = account("codex", "internal-primary", true)
  primary.label = "codex-1"
  const secondary = account("codex", "internal-secondary")
  secondary.label = "Validation"

  assert.equal(resolveProviderAccountSelection([primary, secondary], "codex", "Validation").kind, "resolved")
  assert.equal(resolvedProfileId(resolveProviderAccountSelection([primary, secondary], "codex", " validation ")), "internal-secondary")
  assert.equal(resolvedProfileId(resolveProviderAccountSelection([primary, secondary], "codex", "internal-secondary")), "internal-secondary")
  assert.equal(resolveProviderAccountSelection([primary, secondary], "opencode", "Validation").kind, "missing")
})

test("provider account selection rejects ambiguous case-folded aliases", () => {
  const first = account("codex", "internal-first")
  first.label = "Work"
  const second = account("codex", "internal-second")
  second.label = "work"

  assert.deepEqual(resolveProviderAccountSelection([first, second], "codex", "WORK"), {
    kind: "ambiguous",
    aliases: ["Work", "work"],
  })
})

function resolvedProfileId(selection: ReturnType<typeof resolveProviderAccountSelection>): string | null {
  return selection.kind === "resolved" ? selection.profile.profile_id : null
}

function account(
  provider: string,
  profileId: string,
  isDefault = false,
): ProviderAccountProfile {
  return {
    owner_user_id: "local",
    provider,
    profile_id: profileId,
    label: profileId,
    origin: "linked",
    is_default: isDefault,
    auth_state: "authenticated",
    usage: {
      profile_id: profileId,
      provider,
      availability: "unavailable",
      source: "test",
    },
  }
}

function accountWithUsage(
  provider: string,
  meters: ProviderAccountUsageMeter[],
): ProviderAccountProfile {
  const profile = account(provider, `${provider}-account`, true)
  profile.label = "Account"
  profile.usage = {
    profile_id: profile.profile_id,
    provider,
    availability: "available",
    meters,
    observed_at_ms: Date.now(),
    source: "test",
  }
  return profile
}

function usageMeter(
  meterId: string,
  kind: ProviderAccountUsageMeter["kind"],
  state: ProviderAccountUsageMeter["state"],
  resetsAtMs?: number,
): ProviderAccountUsageMeter {
  return {
    meter_id: meterId,
    label: meterId,
    kind,
    scope: "account",
    state,
    source: "test",
    observed_at_ms: Date.now(),
    ...(resetsAtMs === undefined ? {} : { resets_at_ms: resetsAtMs }),
  }
}
