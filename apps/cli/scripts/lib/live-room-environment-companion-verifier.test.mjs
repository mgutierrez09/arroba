import assert from "node:assert/strict"
import { access, mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import { runRoomEnvironmentCompanion } from "./live-room-environment-companion-verifier.mjs"

for (const hours of [8, 24]) {
test(`Room companion accepts a ${hours}-hour soak budget before preparation`, async () => {
  const prepared = new Error("prepared")
  await assert.rejects(runRoomEnvironmentCompanion({
    env: {
      CHARIOX_ROOM_DRILL_COORDINATION_DIR: path.join(os.tmpdir(), "unused-chariox-soak-probe"),
      CHARIOX_ROOM_DRILL_COMPANION_TIMEOUT_MS: String((hours * 3600 + 600) * 1000),
    },
    prepare: () => { throw prepared },
  }), error => error === prepared)
})
}

for (const scenario of [null, "computer", "browser", "form", "nested-frame", "shadow-root", "replace-field", "history-rollover"]) {
const recovery = scenario === "replace-field"
const form = ["form", "nested-frame", "shadow-root", "replace-field", "history-rollover"].includes(scenario)
const browserMutation = recovery ? "replace-field" : undefined
const browserLayout = ["nested-frame", "shadow-root"].includes(scenario) ? scenario : undefined
const providerMode = form ? "browser" : scenario
const includeProvider = providerMode !== null
test(`Room companion verifier uses stable TUI baselines, provider scenario=${scenario}`, async () => {
  let prepared = false
  let preparedAtReady = false
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-room-companion-verifier-"))
  const localNoticeIds = [1]
  const remoteNoticeIds = [2]
  const action = {
    sequence: 7,
    action_id: "action-web",
    actor_id: "user:local",
    kind: "pointer_click",
    state: "completed",
  }
  const keyboardAction = { ...action, action_id: "action-keyboard", kind: "keyboard_text", sequence: 8 }
  const providerAction = { ...action, action_id: "action-provider", actor_id: "agent:agent-real", sequence: 6,
    mode: providerMode, kind: form ? "submit" : providerMode === "browser" ? "click" : "pointer_click",
    targets: [{ kind: "browser_tab", id: "tab-1" }],
    arguments: { x: 640, y: 400, button: "left", click_count: 1 } }
  const fillAction = { ...providerAction, action_id: "action-fill", kind: "fill", sequence: 5 }
  const replaceAction = { ...providerAction, action_id: "action-replace", kind: "click", sequence: 3 }
  const staleAction = { ...fillAction, action_id: "action-stale", sequence: 4, state: "failed", outcome: { status: "failed", code: "controller_failure" } }
  const shortcutAction = { ...action, action_id: "action-shortcut", kind: "keyboard_key", sequence: 9 }
  const replacementAction = { ...keyboardAction, action_id: "action-ime", sequence: 10 }
  const dragAction = { ...action, action_id: "action-drag", kind: "pointer_drag", sequence: 11 }
  const scrollAction = { ...action, action_id: "action-scroll", kind: "pointer_scroll", sequence: 12 }
  const noticed = { local: [], remote: [] }
  const physical = []
  const resultWriter = (async () => {
    const readyPath = path.join(root, "ready.json")
    while (true) {
      try {
        await access(readyPath)
        break
      } catch {
        await new Promise((resolve) => setTimeout(resolve, 5))
      }
    }
    preparedAtReady = prepared
    await writeFile(path.join(root, "result.json"), JSON.stringify({
      schema: "chariox.room_environment.companion_result.v1",
      status: "passed",
      sessionId: "session-1",
      environmentId: "environment-1",
      actionId: action.action_id,
      actorId: action.actor_id,
      ...(includeProvider ? { provider: { provider: "codex", model: "gpt-5.4", agentId: "agent-real",
        actorId: "agent:agent-real", actionId: providerAction.action_id, webObserved: true,
        ...(form ? { browserTask: "form", browserLayout, fillActionId: fillAction.action_id, baselineSequence: 1 } : {}),
        ...(recovery ? { browserMutation, replacementActionId: replaceAction.action_id, staleActionId: staleAction.action_id, staleErrorObserved: true } : {}),
        screenshot: path.join(root, "provider.png") } } : {}),
      gestures: { dragActionId: dragAction.action_id, scrollActionId: scrollAction.action_id },
      keyboard: {
        actionId: keyboardAction.action_id, physicalEffect: "WEB_KEYBOARD_TEXT_OK",
        replacement: {
          shortcutActionId: shortcutAction.action_id,
          actionId: replacementAction.action_id,
          physicalEffect: "WEB_KEYBOARD_REPLACEMENT_OK",
        },
      },
      physicalEffect: "POINTER_CLICK_COUNT=2",
      client: "production-local-web-view",
      screenshot: path.join(root, "web-room-tui-shared.png"),
    }))
  })()

  try {
    const verified = await runRoomEnvironmentCompanion({
      prepare: async () => { prepared = true },
      env: {
        CHARIOX_ROOM_DRILL_COORDINATION_DIR: root,
        CHARIOX_ROOM_DRILL_COMPANION_TIMEOUT_MS: "1000",
      },
      ready: {
        schema: "chariox.room_environment.companion_ready.v1",
        sessionId: "session-1",
        environmentId: "environment-1",
        keyboardText: "fixture typing",
        keyboardReplacementText: "fixture replacement",
        pointerGestures: true,
        ...(includeProvider ? { realProvider: { provider: "codex", model: "gpt-5.4", mode: providerMode, ...(form ? { browserTask: "form", browserLayout, browserMutation } : {}) } } : {}),
      },
      client: {
        send: async ({ before }) => {
          const early = [...(includeProvider ? [providerAction] : []), ...(form ? [fillAction] : []), ...(recovery ? [replaceAction, staleAction] : [])]
          const recent = [action, keyboardAction, shortcutAction, replacementAction, dragAction, scrollAction]
          if (scenario === "history-rollover") {
            return { RoomEnvironmentActionHistoryListed: { page: before == null ? {
              actions: [...Array.from({ length: 94 }, (_, index) => ({ action_id: `soak-${index}`, sequence: 1000 - index,
                actor_id: "user:soak", kind: "browser_history_reload", state: "completed" })), ...recent],
              next_before_sequence: 7,
            } : { actions: early, next_before_sequence: null } } }
          }
          return { RoomEnvironmentActionHistoryListed: { page: { actions: [...recent, ...early] } } }
        },
      },
      observerClient: {
        send: async () => ({ RoomEnvironmentState: { environment: { input_ownership: [] } } }),
      },
      requests: {
        listRoomEnvironmentActionHistoryRequest: (_session, before) => ({ before }),
        getRoomEnvironmentStateRequest: () => ({}),
      },
      activityController: { synchronize: async () => true },
      localNoticeIds,
      remoteNoticeIds,
      waitForPhysicalEffect: async (value) => { physical.push(value) },
      waitForLocalActionNotice: async (baseline, target) => {
        assert.equal(baseline, localNoticeIds)
        noticed.local.push(target?.sequence)
      },
      waitForRemoteActionNotice: async (baseline, target) => {
        assert.equal(baseline, remoteNoticeIds)
        noticed.remote.push(target?.sequence)
      },
    })

    assert.equal(verified.actionId, action.action_id)
    assert.equal(verified.status, "passed")
    assert.deepEqual(physical, ["POINTER_CLICK_COUNT=2", ...(form ? ["BROWSER_FORM_ACCEPTED"] : providerMode === "browser" ? ["BROWSER_CLICK_ACCEPTED"] : []), ...(recovery ? ["BROWSER_STALE_RECOVERY_ACCEPTED"] : []), "WEB_KEYBOARD_TEXT_OK", "WEB_KEYBOARD_REPLACEMENT_OK",
      "WEB_DRAG_SELECTION_OK WINDOW_GEOMETRY_STABLE", "WEB_SCROLL_BOTH_AXES_OK"])
    const expectedNotices = [...(recovery ? [3, 4] : []), ...(form ? [5] : []), ...(includeProvider ? [6] : []), 7, 8, 9, 10, 11, 12]
    assert.deepEqual(noticed, { local: expectedNotices, remote: expectedNotices })
    assert.equal(preparedAtReady, true, "physical fixture must be reset before Web receives its handoff")
    assert.equal(verified.client, "production-local-web-view")
    assert.equal(verified.screenshot, path.join(root, "web-room-tui-shared.png"))
    await resultWriter
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})
}

test("real-provider opt-in rejects a stub-only Web result", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-room-provider-required-"))
  const writer = (async () => {
    for (let attempt = 0; attempt < 100; attempt += 1) {
      try { await access(path.join(root, "ready.json")); break } catch { await new Promise((resolve) => setTimeout(resolve, 5)) }
    }
    await writeFile(path.join(root, "result.json"), JSON.stringify({
      schema: "chariox.room_environment.companion_result.v1", status: "passed",
      sessionId: "session-1", environmentId: "environment-1", actionId: "web", actorId: "user:local",
      client: "production-local-web-view", physicalEffect: "POINTER_CLICK_COUNT=1", screenshot: path.join(root, "web.png"),
    }))
  })()
  try {
    await assert.rejects(runRoomEnvironmentCompanion({
      env: { CHARIOX_ROOM_DRILL_COORDINATION_DIR: root, CHARIOX_ROOM_DRILL_COMPANION_TIMEOUT_MS: "1000" },
      ready: { sessionId: "session-1", environmentId: "environment-1", realProvider: { provider: "codex", model: "gpt-5.4" } },
    }), /omitted required real-provider evidence/)
    await writer
  } finally { await rm(root, { recursive: true, force: true }) }
})

test("Room companion verifier rejects incomplete evidence metadata", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-room-companion-verifier-"))
  const resultWriter = (async () => {
    const readyPath = path.join(root, "ready.json")
    while (true) {
      try {
        await access(readyPath)
        break
      } catch {
        await new Promise((resolve) => setTimeout(resolve, 5))
      }
    }
    await writeFile(path.join(root, "result.json"), JSON.stringify({
      schema: "chariox.room_environment.companion_result.v1",
      status: "passed",
      sessionId: "session-1",
      environmentId: "environment-1",
      actionId: "action-web",
      actorId: "user:local",
      physicalEffect: "POINTER_CLICK_COUNT=2",
    }))
  })()

  try {
    await assert.rejects(runRoomEnvironmentCompanion({
      env: {
        CHARIOX_ROOM_DRILL_COORDINATION_DIR: root,
        CHARIOX_ROOM_DRILL_COMPANION_TIMEOUT_MS: "1000",
      },
      ready: {
        schema: "chariox.room_environment.companion_ready.v1",
        sessionId: "session-1",
        environmentId: "environment-1",
      },
      waitForPhysicalEffect: async () => undefined,
    }), /companion client/i)
    await resultWriter
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})
