import assert from "node:assert/strict"
import test from "node:test"
import { createRoomDrillTuiEvidence } from "./room-drill-tui-evidence.mjs"

const action = { sequence: 4, mode: "browser", kind: "submit", state: "completed" }
const text = "Room action #4: real-codex · browser submit · completed"

test("TUI receipts require a post-baseline observation on each terminal", () => {
  const evidence = createRoomDrillTuiEvidence(["old"], [])
  evidence.observe({ local: [{ id: "old", text }], remote: [] }, "first")
  assert.equal(evidence.find("local", action), null)
  assert.equal(evidence.find("remote", action), null)
  evidence.observe({ local: [{ id: "new", text }], remote: [] }, "second")
  evidence.observe({ local: [], remote: [] }, "after-eviction")
  assert.equal(evidence.find("local", action).observedAt, "second")
  assert.equal(evidence.find("remote", action), null)
  assert.equal(evidence.find("local", { ...action, sequence: 5 }), null)
  assert.equal(evidence.find("local", { ...action, kind: "fill" }), null)
})

test("running notices cannot satisfy completion; provider output is not retained", () => {
  const evidence = createRoomDrillTuiEvidence()
  evidence.observe({ local: [{ id: 1, text: "synthetic private provider output" },
    { id: 2, text: text.replace("completed", "running") }], remote: [] })
  assert.equal(evidence.find("local", action), null)
  evidence.observe({ local: [{ id: 2, text }], remote: [] })
  assert.equal(evidence.find("local", action).text, text)
  assert.equal(evidence.summary().localNotices, 2)
  assert.equal(JSON.stringify(evidence.summary()).includes("private"), false)
})

test("repeated samples deduplicate notices and fail rather than exceed the evidence bound", () => {
  const evidence = createRoomDrillTuiEvidence()
  for (let i = 0; i < 100; i++) evidence.observe({ local: [{ id: i, text }], remote: [] })
  assert.equal(evidence.summary().localNotices, 1)
  assert.equal(evidence.summary().bytes, Buffer.byteLength(text))
  assert.throws(() => evidence.observe({ local: [{ id: 100, text: text + "x".repeat(1024) }], remote: [] }), /bound/)
})
