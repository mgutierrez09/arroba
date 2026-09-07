import assert from "node:assert/strict"
import test from "node:test"
import { verifyWebOnboardingResult } from "./live-room-web-onboarding-proof.mjs"

async function fixture(change = () => {}, missingTui) {
  const names = ["mail-login", "registration", "confirmation-email", "confirmation"]
  const kinds = ["click", "fill", "submit", "fill", "submit", "click", "click", "submit", "browser_tab_activate"]
  const actions = kinds.map((kind, index) => ({ action_id: `a${index}`, sequence: index + 1, kind,
    mode: "browser", state: "completed", actor_id: "agent:agent", targets: [{ kind: "browser_tab", id: "tab" }] }))
  const phases = names.map(name => ({ name, screenshot: `/evidence/onboarding-${name}.png`, physicalResult: true }))
  const owner = { runId: "run", agentId: "agent", onboarding: { scenario: "email-gated-onboarding", agentId: "agent",
    provider: "codex", model: "fixture", fixturesClosed: true, localTuiObserved: true, remoteTuiObserved: true,
    phases, actions: actions.slice(1).map(a => ({ id: a.action_id, sequence: a.sequence, kind: a.kind })) } }
  const companion = { client: "production-local-web-onboarding-view", provider: { agentId: "agent", actorId: "agent:agent", actionId: "a0" },
    onboarding: { runId: "run", agentId: "agent", phases: names.map(name => ({ name, matched: true, width: 1280, height: 800,
      physicalScreenshot: `/evidence/onboarding-${name}.png`, screenshot: `/evidence/web-onboarding-${name}.png` })) } }
  const noticed = { local: [], remote: [] }
  const input = { ready: { sessionId: "room", environmentId: "environment", runtimeGeneration: 1,
    evidenceRoot: "/evidence", realProvider: { provider: "codex", model: "fixture" }, viewport: { desktop_pixel_width: 1280, desktop_pixel_height: 800 } },
    requests: { listRoomEnvironmentActionHistoryRequest: () => ({}), getRoomEnvironmentStateRequest: () => ({}) },
    client: { send: async () => ({ RoomEnvironmentActionHistoryListed: { page: { actions } } }) },
    observerClient: { send: async () => ({ RoomEnvironmentState: { environment: { environment_id: "environment",
      runtime_generation: 1, lifecycle: "ready", input_ownership: [] } } }) },
    activityController: { synchronize: async () => {} },
    ...Object.fromEntries(["local", "remote"].map(side => [`waitFor${side === "local" ? "Local" : "Remote"}ActionNotice`, async (_, action) => {
      assert.notEqual(`${side}:${action.action_id}`, missingTui, "missing TUI observation")
      noticed[side].push(action.action_id)
    }])),
  }
  change({ companion, owner, actions })
  const result = await verifyWebOnboardingResult(input, companion, owner)
  return { result, noticed }
}

test("Web onboarding acceptance requires all physical phases and both TUI action histories", async () => {
  const { result, noticed } = await fixture()
  assert.equal(result.onboarding.fixturesClosed, true)
  assert.deepEqual(noticed.local, ["a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7", "a8"])
  assert.deepEqual(noticed.remote, noticed.local)
  await assert.rejects(fixture(() => {}, "remote:a3"), /missing TUI/)
})

for (const [name, change] of [
  ["stale run", ({ companion }) => { companion.onboarding.runId = "old" }],
  ["missing phase", ({ companion }) => { companion.onboarding.phases.pop() }],
  ["wrong physical image", ({ companion }) => { companion.onboarding.phases[0].physicalScreenshot = "/evidence/other.png" }],
  ["mismatched canvas", ({ companion }) => { companion.onboarding.phases[0].matched = false }],
  ["wrong dimensions", ({ companion }) => { companion.onboarding.phases[0].width = 640 }],
  ["other actor", ({ actions }) => { actions[3].actor_id = "agent:other" }],
  ["failed action", ({ actions }) => { actions[3].state = "failed" }],
  ["omitted owner action", ({ owner }) => { owner.onboarding.actions.pop() }],
]) test(`Web onboarding cannot pass ${name}`, () => assert.rejects(fixture(change)))
