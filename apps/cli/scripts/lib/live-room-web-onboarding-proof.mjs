import assert from "node:assert/strict"
import path from "node:path"
import { readRoomDrillActionHistory } from "./room-drill-action-history.mjs"
import { verifyRoomCompanionTuis } from "./room-companion-tui-evidence.mjs"
import { assertRoomRealProviderAction } from "./live-room-real-provider.mjs"

export async function verifyWebOnboardingResult(input, companion, owner, evidence) {
  const web = companion.onboarding
  const report = owner?.onboarding
  const provider = companion.provider
  assert.equal(companion.client, "production-local-web-onboarding-view")
  assert.ok(web && report, "missing owner/Web onboarding evidence")
  assert.equal(web.runId, owner.runId, "onboarding run differs")
  for (const value of [web, owner, report]) assert.equal(value.agentId, provider.agentId, "onboarding agent differs")
  assert.equal(report.scenario, "email-gated-onboarding")
  assert.equal(report.provider, input.ready.realProvider.provider)
  assert.equal(report.model, input.ready.realProvider.model)
  assert.equal(report.fixturesClosed, true)
  assert.equal(report.localTuiObserved, true)
  assert.equal(report.remoteTuiObserved, true)
  const names = ["mail-login", "registration", "confirmation-email", "confirmation"]
  assert.deepEqual(web.phases?.map(p => p.name), names, "Web onboarding phases differ")
  assert.deepEqual(report.phases?.map(p => p.name), names, "owner onboarding phases differ")
  for (const [index, name] of names.entries()) {
    const phase = web.phases[index]
    assert.equal(report.phases[index].physicalResult, true)
    assert.equal(phase.matched, true, "Web onboarding frame differs")
    assert.equal(phase.width, input.ready.viewport.desktop_pixel_width)
    assert.equal(phase.height, input.ready.viewport.desktop_pixel_height)
    assert.equal(phase.physicalScreenshot, report.phases[index].screenshot)
    assert.equal(phase.physicalScreenshot, path.join(input.ready.evidenceRoot, `onboarding-${name}.png`))
    assert.equal(phase.screenshot, path.join(input.ready.evidenceRoot, `web-onboarding-${name}.png`))
  }
  const kinds = ["fill", "submit", "fill", "submit", "click", "click", "submit", "browser_tab_activate"]
  assert.deepEqual(report.actions?.map(a => a.kind), kinds, "owner onboarding actions differ")
  const history = await readRoomDrillActionHistory(async (before, limit) => {
    const result = await input.client.send(input.requests.listRoomEnvironmentActionHistoryRequest(input.ready.sessionId, before, limit))
    assert.ok(result?.RoomEnvironmentActionHistoryListed?.page, "missing onboarding history")
    return result.RoomEnvironmentActionHistoryListed.page
  })
  const wanted = [{ id: provider.actionId, kind: "click" }, ...report.actions]
  const actions = wanted.map(want => {
    const action = history.find(a => a.action_id === want.id)
    assert.ok(action, "onboarding action missing from kernel history")
    assert.equal(action.actor_id, provider.actorId)
    assert.equal(action.actor_id, `agent:${owner.agentId}`)
    assert.equal(action.mode, "browser")
    assert.equal(action.kind, want.kind)
    assert.equal(action.state, "completed")
    if (want.sequence !== undefined) assert.equal(action.sequence, want.sequence)
    return action
  })
  assertRoomRealProviderAction(actions[0], "browser")
  for (let i = 1; i < actions.length; i++) assert.ok(actions[i].sequence > actions[i - 1].sequence, "onboarding action order differs")
  const tuiEvidence = await verifyRoomCompanionTuis(input, actions, evidence)
  const result = await input.observerClient.send(input.requests.getRoomEnvironmentStateRequest(input.ready.sessionId))
  const environment = result?.RoomEnvironmentState?.environment
  assert.equal(environment?.environment_id, input.ready.environmentId)
  assert.equal(environment.runtime_generation, input.ready.runtimeGeneration)
  assert.equal(environment.lifecycle, "ready")
  assert.deepEqual(environment.input_ownership, [], "onboarding retained input ownership")
  return { ...companion, onboarding: { ...report, runId: owner.runId, web: web.phases }, tuiEvidence }
}
