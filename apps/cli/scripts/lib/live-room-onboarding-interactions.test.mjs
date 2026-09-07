import assert from "node:assert/strict"
import test from "node:test"
import { createOnboardingInteractionResponder } from "./live-room-onboarding-interactions.mjs"

test("onboarding answers only its provider's kernel-owned secret interactions once", async () => {
  const replies = []
  const responder = createOnboardingInteractionResponder({
    sessionId: "room", agentId: "agent", credentialTitle: "Office mail credential test-1",
    mailPassword: "private-mail", vaultPassphrase: "private-vault",
    requests: { respondToInteractionRequest: (...args) => args },
    client: { send: async args => {
      replies.push(args)
      return { InteractionResponded: { interaction_id: args[1] } }
    } },
  })
  const unlock = { id: "unlock", agent_id: "agent", title: "Unlock Chariox Vault",
    custom_choice: { input_kind: "secret" }, choices: [{ id: "unlock_default_ttl" }] }
  const mail = { id: "mail", agent_id: "agent", title: "Office mail credential test-1",
    custom_choice: { id: "supply-secret", input_kind: "secret" } }
  const state = { session: { id: "room", active_interactions: [
    { ...mail, id: "other-agent", agent_id: "other" }, unlock, mail,
  ] } }
  await responder.poll(state)
  await responder.poll(state)
  assert.deepEqual(replies, [
    ["room", "unlock", "unlock_default_ttl", "private-vault"],
    ["room", "mail", "supply-secret", "private-mail"],
  ])
  assert.deepEqual(responder.report(), { vaultUnlocks: 1, mailCredentialSupplied: true })
  assert.doesNotMatch(JSON.stringify(responder.report()), /private-/)
})

test("onboarding never supplies secrets to another Room or an unexpected credential request", async () => {
  for (const [room, title, choices] of [["other-room", "Mail", []], ["room", "Unrelated credential", []],
    ["room", "Unlock Chariox Vault", []]]) {
    let sent = false
    const responder = createOnboardingInteractionResponder({ sessionId: "room", agentId: "agent", credentialTitle: "Mail",
      mailPassword: "private-mail", vaultPassphrase: "private-vault", requests: { respondToInteractionRequest: () => ({}) },
      client: { send: async () => { sent = true } } })
    await assert.rejects(responder.poll({ session: { id: room, active_interactions: [{ id: "request", agent_id: "agent",
      title, choices, custom_choice: { id: "reply", input_kind: "secret" } }] } }))
    assert.equal(sent, false)
  }
})
