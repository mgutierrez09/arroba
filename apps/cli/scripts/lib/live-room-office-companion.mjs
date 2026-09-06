import assert from "node:assert/strict"
import path from "node:path"
import { readRoomDrillActionHistory } from "./room-drill-action-history.mjs"
import { verifyRoomCompanionTuis } from "./room-companion-tui-evidence.mjs"
import { assertRoomRealProviderAction } from "./live-room-real-provider.mjs"

// Office mode has its own physical result, not the earlier click-page marker.
// Web proves the displayed editor/mail; OSS independently verifies the ledger
// and both real TUIs before claiming end-to-end completion.
export async function verifyRoomOfficeCompanion(input, companion, evidence) {
  const office = companion.office
  const provider = companion.provider
  assert.ok(office, "Web companion omitted required office evidence")
  assert.equal(companion.client, "production-local-web-office-view")
  assert.equal(office.agentId, provider.agentId)
  assert.equal(office.provider, input.ready.realProvider.provider)
  assert.equal(office.model, input.ready.realProvider.model)
  assert.equal(office.fixtureClosed, true, "office fixture was not cleaned up")
  assert.equal(office.edit?.exactDocument, true)
  assert.equal(office.edit?.focusPreserved, true)
  assert.equal(office.mail?.visibleBrowser, true)
  assert.equal(office.mail?.submissions, 1)
  assert.equal(office.mail?.received?.sizeBytes, 86)
  assert.equal(office.mail.received.sha256,
    "1f3aee501e8366f5506b53a57e770f5f404502c36ff54b7706ce73510fea440c", "office document hash differs")
  for (const name of ["editor", "mail"]) {
    const phase = office.web?.[name]
    assert.equal(phase?.matched, true, `Web ${name} did not match the physical desktop`)
    assert.equal(phase.width, input.ready.viewport.desktop_pixel_width)
    assert.equal(phase.height, input.ready.viewport.desktop_pixel_height)
    assert.ok(typeof phase.screenshot === "string" && path.isAbsolute(phase.screenshot))
  }
  const history = await readRoomDrillActionHistory(async (before, limit) => {
    const response = await input.client.send(input.requests.listRoomEnvironmentActionHistoryRequest(input.ready.sessionId, before, limit))
    assert.ok(response?.RoomEnvironmentActionHistoryListed?.page, "missing office action history")
    return response.RoomEnvironmentActionHistoryListed.page
  })
  const wanted = [
    [provider.actionId, "computer", "pointer_click"],
    [office.edit.typedActionId, "computer", "keyboard_text"],
    [office.mail.activationActionId, "browser", "browser_tab_activate"],
    [office.mail.uploadActionId, "browser", "upload"],
    [office.mail.submitActionId, "browser", "submit"],
  ]
  const actions = wanted.map(([id, mode, kind]) => {
    const action = history.find(a => a.action_id === id)
    assert.ok(action, `office ${kind} was absent from kernel history`)
    assert.equal(action.actor_id, provider.actorId, "office actor differs")
    assert.equal(action.mode, mode)
    assert.equal(action.kind, kind)
    assert.equal(action.state, "completed")
    return action
  })
  assertRoomRealProviderAction(actions[0], "computer")
  assert.equal(actions[1].arguments?.utf8_byte_count, 86)
  for (let i = 1; i < actions.length; i++) assert.ok(actions[i].sequence > actions[i - 1].sequence, "office action order differs")
  const tab = actions[3].targets?.[0]
  assert.ok(tab?.kind === "browser_tab" && typeof tab.id === "string" && tab.id)
  assert.deepEqual(actions[2].targets, [{ kind: "desktop" }, tab], "office activation must reserve the desktop and same mail tab")
  for (const action of actions.slice(3)) assert.deepEqual(action.targets, [tab], "office actions must target the same mail tab")
  const tuiEvidence = await verifyRoomCompanionTuis(input, actions, evidence)
  const response = await input.observerClient.send(input.requests.getRoomEnvironmentStateRequest(input.ready.sessionId))
  assert.ok(Array.isArray(response?.RoomEnvironmentState?.environment?.input_ownership))
  assert.equal(response.RoomEnvironmentState.environment.input_ownership.length, 0, "office input ownership was not released")
  return { ...companion, tuiEvidence, office: { ...office,
    skipped: (office.skipped ?? []).filter(item => item !== "TUI verification deferred to OSS companion"),
    edit: { ...office.edit, localTuiObserved: true, remoteTuiObserved: true },
    mail: { ...office.mail, localTuiObserved: true, remoteTuiObserved: true } } }
}
