import assert from "node:assert/strict"
import test from "node:test"
import { createCredentialRevocationResponder, verifyCredentialRevocation } from "./live-room-credential-revocation.mjs"

const credential = { id: "mail", source: { type: "vault", key: "private-key" }, allowed_hosts: ["mail.test:1234"] }
const interaction = { id: "unlock", agent_id: "agent", title: "Unlock Chariox Vault",
  custom_choice: { input_kind: "secret" }, choices: [{ id: "unlock_default_ttl" }] }
const state = { session: { id: "room", active_interactions: [interaction] } }

test("human revokes browser scope through kernel IPC before answering the waiting unlock", async () => {
  let current = credential
  const messages = []
  const responder = createCredentialRevocationResponder({ sessionId: "room", agentId: "agent",
    credential, vaultPassphrase: "private-passphrase", requests: {
      upsertCredentialRequest: value => ({ UpsertCredential: { credential: value } }),
      getCredentialRequest: id => ({ GetCredential: { id } }),
      respondToInteractionRequest: (...args) => ({ Respond: args }),
    }, client: { send: async request => {
      messages.push(request)
      if (request.UpsertCredential) { current = request.UpsertCredential.credential; return { CredentialUpserted: { credential: current } } }
      if (request.GetCredential) return { Credential: { credential: current } }
      assert.deepEqual(current.allowed_hosts, ["revoked.invalid"])
      return { InteractionResponded: { interaction_id: "unlock" } }
    } } })
  await responder.poll({ session: { id: "room", active_interactions: [{ ...interaction, agent_id: "other" }] } })
  assert.equal(messages.length, 0)
  await responder.poll(state)
  await responder.poll(state)
  assert.deepEqual(messages.map(m => Object.keys(m)[0]), ["UpsertCredential", "GetCredential", "Respond"])
  assert.deepEqual(responder.report(), { scopeRevoked: true, vaultUnlocks: 1 })
  assert.deepEqual(current.source, credential.source)
  assert.doesNotMatch(JSON.stringify(responder.report()), /private-/)
})

test("failed scope mutation never unlocks the vault", async () => {
  const responder = createCredentialRevocationResponder({ sessionId: "room", agentId: "agent", credential,
    vaultPassphrase: "private-passphrase", requests: { upsertCredentialRequest: () => ({}) },
    client: { send: async () => ({ Error: {} }) } })
  await assert.rejects(responder.poll(state), /revocation was not acknowledged/)
  assert.equal(responder.report().vaultUnlocks, 0)
  await assert.rejects(responder.poll({ session: { id: "wrong", active_interactions: [] } }), /Room mismatch/)
})

const evidence = () => ({ credentialId: "mail", expectedHost: "mail.test:1234", actorId: "agent:agent", baseline: 20,
  interaction: { scopeRevoked: true, vaultUnlocks: 1 },
  tools: [{ name: "slice_open_url", status: "completed", input: { url: "http://mail.test:1234/mail/login" } },
    { name: "slice_browser_find", status: "completed", output: { browser: { matches: [
      { field_id: "element-password", kind: "field", label: "Password" },
    ] } } }, { name: "paste_secret_to_slice", status: "failed", input: { credential_id: "mail",
    expected_host: "mail.test:1234", field_id: "element-password", submit: false }, errorCodes: ["credential_scope"] }],
  history: [{ sequence: 19, actor_id: "agent:agent", kind: "fill", state: "completed" }] })

test("revocation proof requires the scoped denial and no post-revocation fill", () => {
  assert.equal(verifyCredentialRevocation(evidence()).denied, true)
  const filled = evidence()
  filled.history.push({ sequence: 21, actor_id: "agent:agent", kind: "fill", state: "completed" })
  assert.throws(() => verifyCredentialRevocation(filled), /filled after revocation/)
  for (const mutate of [
    e => { e.tools[2].status = "completed"; e.tools[2].errorCodes = [] },
    e => { e.tools[2].errorCodes = ["field_unavailable"] },
    e => { e.tools[2].input.credential_id = "unrelated" },
    e => { e.interaction.vaultUnlocks = 0 },
    e => { e.tools.push({ name: "bash", status: "failed" }) },
    e => { e.tools.push({ ...e.tools[2] }) },
  ]) {
    const value = evidence(); mutate(value)
    assert.throws(() => verifyCredentialRevocation(value))
  }
})

test("revocation denial must target the Password field freshly discovered after opening the login page", () => {
  for (const mutate of [
    e => { e.tools.splice(0, 2) },
    e => { e.tools[0].input.url = "http://other.test/mail/login" },
    e => { e.tools[0].status = "failed" },
    e => { e.tools[1].status = "failed" },
    e => { e.tools[1].output.browser.matches[0].field_id = "element-stale" },
    e => { e.tools[1].output.browser.matches[0].label = "Email" },
    e => { e.tools[1].output.browser.matches[0].kind = "link" },
    e => { e.tools.push(e.tools.shift()) },
    e => { e.tools.splice(2, 0, { ...e.tools[0] }) },
  ]) {
    const value = evidence(); mutate(value)
    assert.throws(() => verifyCredentialRevocation(value))
  }
})

test("one running and terminal record is one call, but retries and unsettled calls cannot pass", () => {
  const value = evidence()
  value.tools[2].callId = "paste-1"
  value.tools.splice(2, 0, { ...value.tools[2], status: "running", errorCodes: [] })
  assert.equal(verifyCredentialRevocation(value).denied, true)
  for (const mutate of [
    e => { e.tools[2].callId = "another-paste" },
    e => { e.tools[2].name = "bash" },
    e => { e.tools[2].input = { ...e.tools[2].input, credential_id: "other" } },
    e => { e.tools[2].status = "error" },
    e => { e.tools.pop() },
  ]) {
    const candidate = structuredClone(value); mutate(candidate)
    assert.throws(() => verifyCredentialRevocation(candidate))
  }
})
