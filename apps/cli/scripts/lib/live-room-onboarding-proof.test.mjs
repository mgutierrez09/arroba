import assert from "node:assert/strict"
import test from "node:test"
import { verifyOnboardingCredentialActions } from "./live-room-onboarding-proof.mjs"

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
