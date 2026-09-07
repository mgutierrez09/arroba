import assert from "node:assert/strict"
import test from "node:test"
import { verifyOnboardingCredentialActions, verifyOnboardingEmailPath } from "./live-room-onboarding-proof.mjs"

test("onboarding requires attributed email and confirmation clicks and rejects bypass tools", () => {
  const sample = () => ({ actorId: "agent:agent", tools: [
    { name: "slice_browser_find", phase: "confirmation-email", status: "completed", output: { browser: { matches: [
      { kind: "link", field_id: "element-mail", label: "Confirm your Office Service account" },
    ] } } },
    { name: "slice_browser_click", phase: "confirmation-email", status: "completed", input: { field_id: "element-mail" },
      output: { action_id: "mail", browser: { field_id: "element-mail" } } },
    { name: "slice_browser_find", phase: "confirmation", status: "completed", output: { browser: { matches: [
      { kind: "link", field_id: "element-confirm", label: "Confirm account" },
    ] } } },
    { name: "slice_browser_click", phase: "confirmation", status: "completed", input: { field_id: "element-confirm" },
      output: { action_id: "link", browser: { field_id: "element-confirm" } } },
  ], history: ["mail", "link"].map((id, i) => ({ action_id: id, sequence: i + 10, actor_id: "agent:agent",
    mode: "browser", kind: "click", state: "completed", targets: [{ kind: "browser_tab", tab_id: "mail-tab" }] })) })
  assert.equal(verifyOnboardingEmailPath(sample()).length, 2)
  for (const mutate of [
    x => { x.tools.pop() },
    x => { x.tools.push({ name: "bash", status: "completed" }) },
    x => { x.tools.push({ name: "browser_click", status: "running" }) },
    x => { x.history[1].actor_id = "user:local" },
    x => { x.history[1].targets[0].tab_id = "unrelated-tab" },
    x => { x.history[1].sequence = 2 },
    x => { x.tools.push({ name: "slice_open_url", phase: "confirmation", status: "completed" }) },
    x => { x.tools[2].output.browser.matches[0].label = "Unrelated link" },
    x => { x.tools[3].input.field_id = "unrelated-element" },
    x => { x.tools[3].output.browser.field_id = "unrelated-element" },
    x => { x.tools[2].output.browser.matches[0].label = "Unrelated link";
      x.tools.push({ name: "slice_open_url", phase: "confirmation", status: "completed" }) },
  ]) { const invalid = sample(); mutate(invalid); assert.throws(() => verifyOnboardingEmailPath(invalid)) }
})

function fixture() {
  const credentials = ["mail", "service"].map(id => ({ id, allowed_hosts: [`${id}.test:4321`],
    allowed_uses: ["browser"], injection: { kind: "browser" } }))
  return { actorId: "agent:agent", credentials, history: credentials.map((c, i) => ({
    action_id: `action-${i}`, sequence: i + 1, actor_id: "agent:agent", kind: "fill", mode: "browser", state: "completed",
  })), tools: credentials.flatMap((c, i) => [
    { name: i === 0 ? "request_credential_secret" : "create_generated_credential", status: "completed",
      input: { credential: c }, output: { credential: c } },
    { name: "paste_secret_to_slice", status: "completed", input: { credential_id: c.id, expected_host: c.allowed_hosts[0] },
      output: { credential_id: c.id, action_id: `action-${i}`, actor_id: "agent:agent" } },
  ]) }
}

test("onboarding binds vault tools to the attributed completed Browser actions", () => {
  const input = fixture()
  assert.equal(verifyOnboardingCredentialActions(input).length, 2)
  for (const mutate of [
    f => { f.tools = f.tools.filter(t => t.name !== "create_generated_credential") },
    f => { f.tools[1].input.expected_host = "wrong.test" },
    f => { f.tools[1].output.credential_id = "other" },
    f => { f.history[0].actor_id = "user:local" },
    f => { f.history[0].state = "failed" },
  ]) {
    const invalid = fixture(); mutate(invalid)
    assert.throws(() => verifyOnboardingCredentialActions(invalid))
  }
})
