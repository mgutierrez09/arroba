import assert from "node:assert/strict"
import path from "node:path"
import { assertRoomRealProviderAction, assertRoomBrowserFormActions } from "./live-room-real-provider.mjs"
import { assertRoomBrowserRecoveryActions } from "./live-room-browser-recovery.mjs"
import { roomDrillCompanionTimeoutMs } from "./room-drill-companion-budget.mjs"
import { readRoomDrillActionHistory } from "./room-drill-action-history.mjs"
import { createRoomDrillTuiEvidence } from "./room-drill-tui-evidence.mjs"
import { verifyRoomCompanionTuis } from "./room-companion-tui-evidence.mjs"
import { verifyRoomOfficeCompanion } from "./live-room-office-companion.mjs"
import { runRoomWebOnboarding } from "./live-room-web-onboarding.mjs"
import { verifyWebOnboardingResult } from "./live-room-web-onboarding-proof.mjs"
import { setTimeout as sleep } from "node:timers/promises"

import {
  publishRoomDrillCompanionReady,
  waitForRoomDrillCompanionResult,
} from "./room-drill-companion.mjs"

export async function runRoomEnvironmentCompanion(input) {
  const directory = input.env.CHARIOX_ROOM_DRILL_COORDINATION_DIR?.trim()
  if (!directory) return null
  if (!path.isAbsolute(directory)) {
    throw new Error("CHARIOX_ROOM_DRILL_COORDINATION_DIR must be an absolute disposable directory")
  }
  const timeoutMs = roomDrillCompanionTimeoutMs(input.env)
  await input.prepare?.()
  await publishRoomDrillCompanionReady(directory, {
    schema: "chariox.room_environment.companion_ready.v1",
    ...input.ready,
  })
  const tuiEvidence = input.readTuiNotices
    ? createRoomDrillTuiEvidence(input.localNoticeIds, input.remoteNoticeIds) : null
  let nextSampleAt = 0
  const sampleTuis = async (force = false) => {
    if (!tuiEvidence || (!force && Date.now() < nextSampleAt)) return
    tuiEvidence.observe(await input.readTuiNotices())
    nextSampleAt = Date.now() + 2000
  }
  await sampleTuis()
  const owner = input.ready.realProvider?.officeScenario === "onboarding"
    ? await runRoomWebOnboarding({ ...input.onboardingInput,
      directory, evidenceRoot: input.ready.evidenceRoot,
      sessionId: input.ready.sessionId, environmentId: input.ready.environmentId,
      sliceId: input.ready.sliceId, sleep: input.sleep, sampleTuis,
      verifyTuiActions: async actions => {
        await sampleTuis(true)
        return verifyRoomCompanionTuis(input, actions, tuiEvidence)
      },
    }) : null
  const companion = await waitForRoomDrillCompanionResult(directory, {
    sessionId: input.ready.sessionId,
    environmentId: input.ready.environmentId,
    timeoutMs,
    pollIntervalMs: 100,
    sleep: async ms => { await sampleTuis(); await (input.sleep ?? sleep)(ms) },
  })
  await sampleTuis(true)
  validateCompanionResult(companion)
  if (input.ready.realProvider) {
    assert.ok(companion.provider, "Web companion omitted required real-provider evidence")
    assert.equal(companion.provider.provider, input.ready.realProvider.provider)
    assert.equal(companion.provider.model, input.ready.realProvider.model)
    assert.equal(companion.provider.browserLayout, input.ready.realProvider.browserLayout)
    assert.equal(companion.provider.browserMutation, input.ready.realProvider.browserMutation)
    assert.equal(companion.provider.webObserved, true, "Web must observe the provider action")
    assert.ok(typeof companion.provider.agentId === "string" && companion.provider.agentId.length > 0)
    assert.equal(companion.provider.actorId, `agent:${companion.provider.agentId}`)
    assert.ok(typeof companion.provider.screenshot === "string" && path.isAbsolute(companion.provider.screenshot))
  }
  if (input.ready.realProvider?.computerTask === "office") {
    return verifyRoomOfficeCompanion(input, companion, tuiEvidence)
  }
  if (input.ready.realProvider?.officeScenario === "onboarding") {
    return verifyWebOnboardingResult(input, companion, owner, tuiEvidence)
  }
  if (input.ready.keyboardText) {
    assert.ok(companion.keyboard, "Web companion omitted required keyboard evidence")
  }
  if (input.ready.keyboardReplacementText) {
    assert.ok(companion.keyboard?.replacement, "Web companion omitted shortcut/IME evidence")
  }
  if (input.ready.pointerGestures) {
    assert.ok(companion.gestures, "Web companion omitted drag/scroll evidence")
  }
  await input.waitForPhysicalEffect(companion.physicalEffect)
  if (input.ready.realProvider?.browserTask === "form") {
    await input.waitForPhysicalEffect("BROWSER_FORM_ACCEPTED")
  } else if (input.ready.realProvider?.mode === "browser") await input.waitForPhysicalEffect("BROWSER_CLICK_ACCEPTED")
  if (input.ready.realProvider?.browserMutation === "replace-field") await input.waitForPhysicalEffect("BROWSER_STALE_RECOVERY_ACCEPTED")
  if (companion.keyboard) {
    assert.equal(companion.keyboard.physicalEffect, "WEB_KEYBOARD_TEXT_OK")
    assert.equal(typeof companion.keyboard.actionId, "string")
    assert.ok(companion.keyboard.actionId.length > 0)
    await input.waitForPhysicalEffect(companion.keyboard.physicalEffect)
    if (companion.keyboard.replacement) {
      assert.equal(companion.keyboard.replacement.physicalEffect, "WEB_KEYBOARD_REPLACEMENT_OK")
      await input.waitForPhysicalEffect(companion.keyboard.replacement.physicalEffect)
    }
  }

  if (companion.gestures) {
    await input.waitForPhysicalEffect("WEB_DRAG_SELECTION_OK WINDOW_GEOMETRY_STABLE")
    await input.waitForPhysicalEffect("WEB_SCROLL_BOTH_AXES_OK")
  }
  const history = await readRoomDrillActionHistory(async (before, limit) => unwrap(
    await input.client.send(input.requests.listRoomEnvironmentActionHistoryRequest(
      input.ready.sessionId,
      before,
      limit,
    )),
    "RoomEnvironmentActionHistoryListed",
  ).page)
  const webAction = history.find((action) => action.action_id === companion.actionId)
  assert.ok(webAction, `Web companion action ${companion.actionId} was absent from kernel history`)
  assert.equal(webAction.actor_id, companion.actorId)
  assert.equal(webAction.kind, "pointer_click")
  assert.equal(webAction.state, "completed")
  const actions = [webAction]
  if (input.ready.realProvider) {
    const provider = companion.provider
    const action = history.find((item) => item.action_id === provider.actionId)
    assert.ok(action, "real-provider action was absent from kernel history")
    assert.equal(action.actor_id, provider.actorId)
    assertRoomRealProviderAction(action, input.ready.realProvider.mode, input.ready.realProvider.browserTask)
    assert.ok(action.sequence < webAction.sequence, "provider action must precede human takeover")
    actions.unshift(action)
    if (input.ready.realProvider.browserTask === "form") {
      const fill = assertRoomBrowserFormActions(history, action, provider.baselineSequence)
      assert.equal(fill.action_id, provider.fillActionId)
      actions.unshift(fill)
      if (input.ready.realProvider.browserMutation === "replace-field") {
        const recovery = assertRoomBrowserRecoveryActions(history, action, provider.baselineSequence)
        assert.equal(recovery.replacement.action_id, provider.replacementActionId)
        assert.equal(recovery.stale.action_id, provider.staleActionId)
        assert.equal(provider.staleErrorObserved, true)
        actions.unshift(recovery.replacement, recovery.stale)
      }
    }
  }
  if (companion.keyboard) {
    const keyboard = history.find((action) => action.action_id === companion.keyboard.actionId)
    assert.ok(keyboard, "Web keyboard action was absent from kernel history")
    assert.equal(keyboard.kind, "keyboard_text")
    assert.equal(keyboard.state, "completed")
    assert.equal(keyboard.actor_id, companion.actorId)
    assert.ok(keyboard.sequence > webAction.sequence, "typing must follow the focus click")
    if (input.ready.keyboardText) {
      assert.ok(!JSON.stringify(history).includes(input.ready.keyboardText), "history retained Web typed text")
    }
    actions.push(keyboard)
    if (companion.keyboard.replacement) {
      const replacement = companion.keyboard.replacement
      let previous = keyboard
      for (const [id, kind] of [[replacement.shortcutActionId, "keyboard_key"], [replacement.actionId, "keyboard_text"]]) {
        assert.equal(typeof id, "string")
        assert.ok(id.length > 0)
        const action = history.find((item) => item.action_id === id)
        assert.ok(action, "Web shortcut/IME action was absent from kernel history")
        assert.equal(action.kind, kind)
        assert.equal(action.state, "completed")
        assert.equal(action.actor_id, companion.actorId)
        assert.ok(action.sequence > previous.sequence, "shortcut and IME must follow initial typing in order")
        actions.push(action)
        previous = action
      }
      if (input.ready.keyboardReplacementText) {
        assert.ok(!JSON.stringify(history).includes(input.ready.keyboardReplacementText), "history retained Web IME text")
      }
    }
  }

  if (companion.gestures) {
    for (const [id, kind] of [[companion.gestures.dragActionId, "pointer_drag"],
      [companion.gestures.scrollActionId, "pointer_scroll"]]) {
      assert.equal(typeof id, "string")
      assert.ok(id.length > 0)
      const action = history.find((item) => item.action_id === id)
      assert.ok(action, "Web gesture was absent from kernel history")
      assert.equal(action.kind, kind)
      assert.equal(action.state, "completed")
      assert.equal(action.actor_id, companion.actorId)
      assert.ok(action.sequence > actions.at(-1).sequence, "gestures must follow typing in order")
      actions.push(action)
    }
    const afterTyping = actions.at(-3).sequence
    assert.deepEqual(history.filter((action) => action.actor_id === companion.actorId && action.sequence > afterTyping)
      .sort((a, b) => a.sequence - b.sequence).map((action) => action.action_id),
    [companion.gestures.dragActionId, companion.gestures.scrollActionId], "Web gestures emitted extra actions")
  }
  const observed = await verifyRoomCompanionTuis(input, actions, tuiEvidence)
  const after = unwrap(
    await input.observerClient.send(input.requests.getRoomEnvironmentStateRequest(input.ready.sessionId)),
    "RoomEnvironmentState",
  ).environment
  assert.equal(after.input_ownership.some((owner) => owner.target?.kind === "desktop"), false)
  return { ...companion, tuiEvidence: observed }
}

function validateCompanionResult(companion) {
  assert.equal(companion.status, "passed", "companion status must be passed")
  assert.ok(
    typeof companion.client === "string" && companion.client.trim().length > 0,
    "companion client must be a non-empty string",
  )
  assert.equal(typeof companion.actionId, "string")
  assert.ok(companion.actionId.length > 0)
  assert.equal(typeof companion.actorId, "string")
  assert.ok(companion.actorId.length > 0)
  assert.match(companion.physicalEffect, /^POINTER_CLICK_COUNT=\d+$/)
  assert.ok(
    typeof companion.screenshot === "string" && path.isAbsolute(companion.screenshot),
    "companion screenshot must be an absolute path",
  )
}

function unwrap(response, variant) {
  assert.ok(response && typeof response === "object" && variant in response, `expected ${variant}, got ${JSON.stringify(response)}`)
  return response[variant]
}
