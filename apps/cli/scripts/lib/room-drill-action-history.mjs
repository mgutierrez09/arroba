import assert from "node:assert/strict"

// Long drills exceed a single history page. Bound the complete evidence read,
// and fail on broken pagination rather than looping or silently truncating it.
export async function readRoomDrillActionHistory(readPage) {
  const actions = []
  const ids = new Set()
  let before = null
  while (true) {
    const page = await readPage(before, 100)
    assert.ok(Array.isArray(page.actions) && page.actions.length <= 100, "invalid action history page")
    for (const action of page.actions) {
      assert.ok(Number.isSafeInteger(action.sequence) && action.sequence > 0)
      assert.ok(before === null || action.sequence < before, "action history cursor did not advance")
      assert.ok(typeof action.action_id === "string" && action.action_id && !ids.has(action.action_id), "duplicate action history identity")
      ids.add(action.action_id)
      actions.push(action)
    }
    assert.ok(actions.length <= 4096, "drill action history exceeds the evidence bound")
    const next = page.next_before_sequence
    if (next == null) return actions
    assert.ok(page.actions.length && Number.isSafeInteger(next) && next > 0
      && (before === null || next < before), "action history cursor did not advance")
    before = next
  }
}
