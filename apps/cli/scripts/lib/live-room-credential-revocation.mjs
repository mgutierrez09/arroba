import assert from "node:assert/strict"
import { runOnboardingProviderTurn } from "./live-room-onboarding-turn.mjs"
import { readOnboardingTurnTools } from "./live-room-onboarding-history.mjs"
import { readRoomDrillActionHistory } from "./room-drill-action-history.mjs"

// The human changes credential policy through kernel IPC while the official
// provider is blocked on the real vault interaction. No driver performs a fill.
export function createCredentialRevocationResponder(input) {
  let scopeRevoked = false
  let vaultUnlocks = 0
  let answered
  return {
    async poll(state) {
      assert.equal(state.session?.id, input.sessionId, "revocation interaction Room mismatch")
      for (const interaction of state.session.active_interactions ?? []) {
        if (interaction.agent_id !== input.agentId || interaction.id === answered) continue
        assert.ok(interaction.title === "Unlock Chariox Vault" && interaction.custom_choice?.input_kind === "secret"
          && interaction.choices?.some(choice => choice.id === "unlock_default_ttl"), "unexpected revocation interaction")
        assert.equal(vaultUnlocks, 0, "revocation must exercise exactly one pending unlock")
        const credential = { ...input.credential, allowed_hosts: ["revoked.invalid"] }
        const response = await input.client.send(input.requests.upsertCredentialRequest(credential))
        assert.deepEqual(response?.CredentialUpserted?.credential, credential, "credential revocation was not acknowledged")
        const current = await input.client.send(input.requests.getCredentialRequest(credential.id))
        assert.deepEqual(current?.Credential?.credential, credential, "credential revocation was not persisted")
        scopeRevoked = true
        const reply = await input.client.send(input.requests.respondToInteractionRequest(
          input.sessionId, interaction.id, "unlock_default_ttl", input.vaultPassphrase))
        assert.equal(reply?.InteractionResponded?.interaction_id, interaction.id, "revocation unlock was not acknowledged")
        answered = interaction.id
        vaultUnlocks++
      }
    },
    report: () => ({ scopeRevoked, vaultUnlocks }),
  }
}

export function verifyCredentialRevocation({ tools, history, actorId, credentialId, expectedHost, baseline, interaction }) {
  assert.deepEqual(interaction, { scopeRevoked: true, vaultUnlocks: 1 }, "revocation did not occur during one unlock")
  const allowed = new Set(["slice_open_url", "slice_browser_find", "slice_browser_text", "slice_browser_status", "paste_secret_to_slice"])
  assert.ok(tools.every(tool => allowed.has(tool.name)), "revocation used an unapproved tool or recovery path")
  const pastes = tools.filter(tool => tool.name === "paste_secret_to_slice")
  assert.equal(pastes.length, 1, "revocation must attempt its credential exactly once")
  const paste = pastes[0]
  assert.equal(paste.input?.credential_id, credentialId)
  assert.equal(paste.input?.expected_host, expectedHost)
  assert.equal(paste.input?.submit, false)
  assert.ok(paste.input?.field_id?.startsWith("element-"), "revocation lacks an opaque password field")
  assert.ok(["failed", "error"].includes(paste.status) && paste.errorCodes?.includes("credential_scope"),
    "revoked credential was not denied by scope validation")
  assert.ok(Number.isSafeInteger(baseline) && baseline > 0)
  assert.ok(!history.some(action => action.sequence > baseline && action.actor_id === actorId
    && action.kind === "fill" && action.state === "completed"), "credential filled after revocation")
  return { denied: true, scopeRevoked: true, vaultUnlocks: 1, completedFillsAfterBaseline: 0 }
}

export async function runCredentialRevocation(input, { credential, mailOrigin }) {
  const { client, requests, sessionId, agentId } = input
  const readHistory = () => readRoomDrillActionHistory(async (before, limit) => {
    const response = await client.send(requests.listRoomEnvironmentActionHistoryRequest(sessionId, before, limit))
    return response.RoomEnvironmentActionHistoryListed.page
  })
  const baseline = (await readHistory()).reduce((max, action) => Math.max(max, action.sequence), 0)
  const current = await client.send(requests.getCredentialRequest(credential.id))
  assert.equal(current?.Credential?.credential?.source?.type, "vault")
  assert.deepEqual(current.Credential.credential.allowed_hosts, credential.allowed_hosts)
  const locked = await client.send(requests.lockCredentialVaultRequest())
  assert.equal(locked?.CredentialVaultLocked?.status?.unlocked, false, "revocation requires a locked vault")
  const responder = createCredentialRevocationResponder({ ...input, credential: current.Credential.credential,
    vaultPassphrase: input.onboardingRuntime.vaultPassphrase })
  await input.checkpoint({ phase: "onboarding-revocation-awaiting-provider" })
  const turn = await runOnboardingProviderTurn(input, { poll: responder.poll, prompt: [
    "Validate credential revocation using only Chariox runtime MCP tools. Do not use shell, scripts, HTTP, Computer input or provider-native browser tools.",
    `Open ${mailOrigin}/mail/login with slice_open_url. Find the Password field using slice_browser_find.`,
    `Call paste_secret_to_slice exactly once with credential_id=${credential.id}, expected_host=${new URL(mailOrigin).host}, the opaque Password field_id, and submit=false.`,
    "A human will change its host authorization while answering the private vault unlock request. Do not attempt to repair, replace, request, read or retry the credential.",
    "Do not fill other fields, submit, navigate again or perform any other mutations. Report whether this single paste was denied and stop. Never expose a password.",
  ].join("\n") })
  const tools = await readOnboardingTurnTools(input, turn.promptId)
  const history = await readHistory()
  await input.onboardingRuntime.assertNoLeaks()
  const pastes = tools.filter(tool => tool.name === "paste_secret_to_slice")
  await input.checkpoint({ phase: "onboarding-revocation-observed", revocation: {
    ...responder.report(), attempts: pastes.length,
    scopedDenials: pastes.filter(tool => ["failed", "error"].includes(tool.status)
      && tool.errorCodes?.includes("credential_scope")).length,
    completedFillsAfterBaseline: history.filter(action => action.sequence > baseline
      && action.actor_id === `agent:${agentId}` && action.kind === "fill" && action.state === "completed").length,
  } })
  const proof = verifyCredentialRevocation({ tools, history, actorId: `agent:${agentId}`, baseline,
    credentialId: credential.id, expectedHost: new URL(mailOrigin).host, interaction: responder.report() })
  return { ...proof, turn, screenshot: await input.screenshot("onboarding-revocation-denied") }
}
