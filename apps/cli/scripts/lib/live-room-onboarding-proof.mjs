import assert from "node:assert/strict"

export function verifyOnboardingEmailPath({ tools, history, actorId, navigationUrls = {} }) {
  const allowed = new Set(["request_credential_secret", "create_generated_credential", "paste_secret_to_slice",
    "slice_open_url", "slice_browser_find", "slice_browser_fill", "slice_browser_submit", "slice_browser_text",
    "slice_browser_click", "slice_browser_status", "slice_browser_tab", "slice_screenshot"])
  assert.ok(tools.every(t => allowed.has(t.name)), "onboarding used a tool outside the approved Chariox browser path")
  for (const tool of tools) {
    if (tool.name === "slice_open_url") {
      assert.ok(tool.phase !== "confirmation" && navigationUrls[tool.phase]
        && tool.input?.url === navigationUrls[tool.phase], "onboarding bypassed its phase navigation path")
    }
    if (["confirmation-email", "confirmation"].includes(tool.phase)) {
      assert.ok(!["slice_browser_fill", "paste_secret_to_slice", "create_generated_credential", "request_credential_secret"]
        .includes(tool.name), "onboarding mutated the confirmation document outside its links and form")
    }
  }
  const actions = ["confirmation-email", "confirmation"].map(phase => {
    const clicks = tools.filter(t => t.phase === phase && t.name === "slice_browser_click" && t.status === "completed")
    assert.equal(clicks.length, 1, "onboarding must open its email and follow its confirmation link")
    const click = clicks[0]
    const reference = click.input?.field_id ?? click.input?.selector
    const label = phase === "confirmation-email" ? "Confirm your Office Service account" : "Confirm account"
    assert.ok(typeof reference === "string" && reference.startsWith("element-"), "onboarding click lacks an opaque link reference")
    const discovery = tools.slice(0, tools.indexOf(click)).findLast(t => t.phase === phase
      && t.name === "slice_browser_find" && t.status === "completed")
    assert.ok(discovery?.output?.browser?.matches?.some(m => m.kind === "link" && m.field_id === reference
      && (m.label === label || m.text === label)), "onboarding did not click the discovered confirmation link")
    assert.equal(click.output?.browser?.field_id, reference, "onboarding click result differs from the discovered link")
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
