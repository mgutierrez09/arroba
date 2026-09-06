import assert from "node:assert/strict"
import { access, mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { runRoomEnvironmentCompanion } from "./live-room-environment-companion-verifier.mjs"

test("Web office work requires independent TUI receipts for editor and mail actions", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-office-companion-"))
  const actor = "agent:agent-real"
  const actions = [
    { action_id: "pointer", sequence: 1, kind: "pointer_click", mode: "computer", arguments: { x: 640, y: 400, button: "left", click_count: 1 } },
    { action_id: "typed", sequence: 2, kind: "keyboard_text", mode: "computer", arguments: { utf8_byte_count: 86 } },
    { action_id: "activate", sequence: 3, kind: "browser_tab_activate", mode: "browser" },
    { action_id: "upload", sequence: 4, kind: "upload", mode: "browser" },
    { action_id: "submit", sequence: 5, kind: "submit", mode: "browser" },
  ].map(a => ({ ...a, actor_id: actor, state: "completed", targets: [{ kind: "browser_tab", id: "mail-tab" }] }))
  const phase = { matched: true, width: 1280, height: 800, screenshot: path.join(root, "phase.png") }
  const companion = {
    schema: "chariox.room_environment.companion_result.v1", status: "passed", sessionId: "room", environmentId: "environment",
    client: "production-local-web-office-view", actionId: "pointer", actorId: actor,
    physicalEffect: "POINTER_CLICK_COUNT=1", screenshot: phase.screenshot,
    provider: { provider: "codex", model: "fixture", agentId: "agent-real", actorId: actor, actionId: "pointer",
      screenshot: phase.screenshot, webObserved: true },
    office: { agentId: "agent-real", provider: "codex", model: "fixture", fixtureClosed: true,
      edit: { exactDocument: true, focusPreserved: true, typedActionId: "typed", localTuiObserved: false, remoteTuiObserved: false },
      mail: { activationActionId: "activate", uploadActionId: "upload", submitActionId: "submit", visibleBrowser: true,
        submissions: 1, localTuiObserved: false, remoteTuiObserved: false,
        received: { sizeBytes: 86, sha256: "1f3aee501e8366f5506b53a57e770f5f404502c36ff54b7706ce73510fea440c" } },
      web: { editor: phase, mail: phase },
    },
  }
  async function run(mutate = () => {}, missing = null) {
    const result = structuredClone(companion)
    const history = structuredClone(actions)
    mutate(result, history)
    const noticed = { local: [], remote: [] }
    const writer = (async () => {
      for (let i = 0; i < 200; i++) {
        try { await access(path.join(root, "ready.json")); break } catch { await new Promise(r => setTimeout(r, 2)) }
      }
      await writeFile(path.join(root, "result.json"), JSON.stringify(result))
    })()
    try {
      const verified = await runRoomEnvironmentCompanion({
        env: { CHARIOX_ROOM_DRILL_COORDINATION_DIR: root, CHARIOX_ROOM_DRILL_COMPANION_TIMEOUT_MS: "1000" },
        ready: { sessionId: "room", environmentId: "environment", viewport: { desktop_pixel_width: 1280, desktop_pixel_height: 800 },
          realProvider: { provider: "codex", model: "fixture", mode: "computer", computerTask: "office" } },
        client: { send: async () => ({ RoomEnvironmentActionHistoryListed: { page: { actions: history } } }) },
        observerClient: { send: async () => ({ RoomEnvironmentState: { environment: { input_ownership: [] } } }) },
        requests: { listRoomEnvironmentActionHistoryRequest: () => ({}), getRoomEnvironmentStateRequest: () => ({}) },
        localNoticeIds: [], remoteNoticeIds: [], activityController: { synchronize: async () => {} },
        waitForLocalActionNotice: async (_, a) => { assert.notEqual(missing, `local:${a.action_id}`, "missing local receipt"); noticed.local.push(a.action_id) },
        waitForRemoteActionNotice: async (_, a) => { assert.notEqual(missing, `remote:${a.action_id}`, "missing remote receipt"); noticed.remote.push(a.action_id) },
      })
      return { verified, noticed }
    } finally { await writer; await rm(path.join(root, "ready.json"), { force: true }) }
  }
  try {
    const { verified, noticed } = await run()
    assert.deepEqual(noticed, { local: ["pointer", "typed", "activate", "upload", "submit"], remote: ["pointer", "typed", "activate", "upload", "submit"] })
    assert.equal(verified.office.edit.localTuiObserved, true)
    assert.equal(verified.office.mail.remoteTuiObserved, true)
    await assert.rejects(run(() => {}, "remote:typed"), /missing remote receipt/)
    await assert.rejects(run(() => {}, "local:submit"), /missing local receipt/)
    await assert.rejects(run(r => { delete r.office }), /office evidence/)
    await assert.rejects(run(r => { r.office.web.editor.matched = false }), /Web editor/)
    await assert.rejects(run(r => { r.office.mail.received.sha256 = "0".repeat(64) }), /document hash/)
    await assert.rejects(run((_, h) => { h[3].actor_id = "agent:other" }), /office actor/)
    await assert.rejects(run((_, h) => { h[4].targets[0].id = "other-tab" }), /same mail tab/)
  } finally { await rm(root, { recursive: true, force: true }) }
})
