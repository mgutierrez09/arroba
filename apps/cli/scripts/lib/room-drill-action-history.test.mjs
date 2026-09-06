import assert from "node:assert/strict"
import test from "node:test"
import { readRoomDrillActionHistory } from "./room-drill-action-history.mjs"

test("history pagination rejects nonadvancing cursors and duplicated actions", async () => {
  await assert.rejects(readRoomDrillActionHistory(async () => ({
    actions: [{ action_id: "same", sequence: 2 }], next_before_sequence: 2,
  })), /cursor/)
  await assert.rejects(readRoomDrillActionHistory(async before => ({
    actions: [{ action_id: "same", sequence: before === null ? 3 : 1 }], next_before_sequence: before === null ? 2 : null,
  })), /duplicate/)
})

test("history pagination has a finite total evidence bound", async () => {
  let page = 0
  await assert.rejects(readRoomDrillActionHistory(async () => {
    const start = 10_000 - page++ * 100
    return { actions: Array.from({ length: 100 }, (_, i) => ({ action_id: String(start - i), sequence: start - i })),
      next_before_sequence: start - 99 }
  }), /evidence bound/)
  assert.equal(page, 41)
})
