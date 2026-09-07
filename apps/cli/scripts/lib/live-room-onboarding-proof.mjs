import assert from "node:assert/strict"

export function verifyOnboardingEmailPath({ tools, history, actorId }) {
  const allowed = new Set(["request_credential_secret", "create_generated_credential", "paste_secret_to_slice",
    "slice_open_url", "slice_browser_find", "slice_browser_fill", "slice_browser_submit", "slice_browser_text",
    "slice_browser_click", "slice_browser_status", "slice_browser_tab", "slice_screenshot"])
  assert.ok(tools.every(t => allowed.has(t.name)), "onboarding used a tool outside the approved Chariox browser path")
  const actions = ["confirmation-email", "confirmation"].map(phase => {
    const clicks = tools.filter(t => t.phase === phase && t.name === "slice_browser_click" && t.status === "completed")
    assert.equal(clicks.length, 1, "onboarding must open its email and follow its confirmation link")
    const action = history.find(a => a.action_id === clicks[0].output?.action_id)
    assert.ok(action?.kind === "click" && action.actor_id === actorId && action.mode === "browser"
      && action.state === "completed", "onboarding email click lacks its attributed completed action")
    assert.ok(action.targets?.length === 1 && action.targets[0].kind === "browser_tab")
    return action
  })
  assert.ok(actions[1].sequence > actions[0].sequence, "confirmation link was not clicked after opening the email")
  assert.deepEqual(actions[1].targets, actions[0].targets, "confirmation link was not followed from the email tab")
  return actions
}

export function verifyOnboardingCredentialActions({ tools, history, actorId, credentials }) {
  const completed = tools.filter(t => t.status === "completed")
  const actions = []
  for (const [index, credential] of credentials.entries()) {
    const creation = completed.find(t => t.name === (index === 0 ? "request_credential_secret" : "create_generated_credential")
      && t.input?.credential?.id === credential.id)
    assert.ok(creation, "official provider did not create the required vault credential")
    assert.equal(creation.output?.credential?.id ?? creation.output?.credential_id, credential.id,
      "vault credential creation was not acknowledged")
    assert.deepEqual(creation.input.credential.allowed_hosts, credential.allowed_hosts)
    assert.deepEqual(creation.input.credential.allowed_uses, ["browser"])
    assert.deepEqual(creation.input.credential.injection, { kind: "browser" })
    const paste = completed.find(t => t.name === "paste_secret_to_slice" && t.input?.credential_id === credential.id)
    assert.ok(paste, "official provider did not use the required browser credential tool")
    assert.equal(paste.input.expected_host, credential.allowed_hosts[0])
    assert.equal(paste.output?.credential_id, credential.id)
    assert.equal(paste.output?.actor_id, actorId)
    const action = history.find(a => a.action_id === paste.output?.action_id)
    assert.ok(action, "credential paste has no kernel action")
    assert.equal(action.actor_id, actorId)
    assert.equal(action.mode, "browser")
    assert.equal(action.kind, "fill")
    assert.equal(action.state, "completed")
    actions.push(action)
  }
  assert.ok(actions[0].sequence < actions[1].sequence, "onboarding credential action order differs")
  return actions
}
