import assert from "node:assert/strict"

// Simulates the attended test user's replies through the existing interaction
// protocol. Never gives a password to a prompt, tool argument or evidence file.
export function createOnboardingInteractionResponder(input) {
  const answered = new Set()
  let vaultUnlocks = 0
  let mailCredentialSupplied = false
  return {
    async poll(state) {
      assert.equal(state.session?.id, input.sessionId, "onboarding interaction Room mismatch")
      for (const interaction of state.session.active_interactions ?? []) {
        if (interaction.agent_id !== input.agentId || answered.has(interaction.id)
          || interaction.custom_choice?.input_kind !== "secret") continue
        let choice
        let secret
        if (interaction.title === "Unlock Chariox Vault"
          && interaction.choices?.some(choice => choice.id === "unlock_default_ttl")) {
          choice = "unlock_default_ttl"
          secret = input.vaultPassphrase
        } else if (interaction.title === input.credentialTitle && !mailCredentialSupplied) {
          choice = interaction.custom_choice.id
          secret = input.mailPassword
        } else throw new Error("unexpected onboarding secret interaction")
        assert.ok(typeof choice === "string" && choice && typeof secret === "string" && secret,
          "onboarding interaction lacks its private reply")
        const response = await input.client.send(input.requests.respondToInteractionRequest(
          input.sessionId, interaction.id, choice, secret))
        assert.equal(response?.InteractionResponded?.interaction_id, interaction.id,
          "onboarding secret reply was not acknowledged")
        answered.add(interaction.id)
        if (choice === "unlock_default_ttl") vaultUnlocks++
        else mailCredentialSupplied = true
      }
    },
    report: () => ({ vaultUnlocks, mailCredentialSupplied }),
  }
}
