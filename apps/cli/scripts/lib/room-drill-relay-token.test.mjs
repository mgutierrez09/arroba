import assert from "node:assert/strict"
import { createHmac } from "node:crypto"
import test from "node:test"
import { roomDrillRelayToken } from "./room-drill-relay-token.mjs"

const setupAllowanceMs = 15 * 60_000
function issue(subjectKind, env = {}) {
  const token = roomDrillRelayToken({ issuer: "fixture", secret: "test-only-secret",
    machineId: "machine", subject: subjectKind, subjectKind,
    actions: ["packet_route"], userId: subjectKind === "client" ? "local" : null,
    nowMs: 1_000, env })
  const [format, payload, signature] = token.split(".")
  assert.equal(format, "chariox-scoped-v1")
  assert.equal(signature, createHmac("sha256", "test-only-secret").update(payload).digest("base64url"))
  return JSON.parse(Buffer.from(payload, "base64url").toString())
}

test("ordinary Room drill credentials keep their fifteen-minute lifetime", () => {
  assert.equal(issue("kernel").expires_at_ms, 1_000 + setupAllowanceMs)
})

test("kernel and remote TUI remain authenticated through the complete companion budget", () => {
  for (const hours of [8, 24]) {
    const timeoutMs = hours * 3_600_000 + 600_000
    for (const kind of ["kernel", "client"]) {
      const claims = issue(kind, { CHARIOX_ROOM_DRILL_COORDINATION_DIR: "/tmp/test-only",
        CHARIOX_ROOM_DRILL_COMPANION_TIMEOUT_MS: String(timeoutMs) })
      assert.equal(claims.expires_at_ms, 1_000 + setupAllowanceMs + timeoutMs)
      assert.equal(claims.machine_id, kind === "kernel" ? "machine" : null)
      assert.equal(claims.user_id, kind === "client" ? "local" : null)
    }
  }
})

test("long-lived fixture credentials require a companion and a bounded budget", () => {
  assert.equal(issue("kernel", { CHARIOX_ROOM_DRILL_COMPANION_TIMEOUT_MS: "29400000" }).expires_at_ms,
    1_000 + setupAllowanceMs)
  for (const value of ["Infinity", "87000001", "-1"]) {
    assert.throws(() => issue("kernel", { CHARIOX_ROOM_DRILL_COORDINATION_DIR: "/tmp/test-only",
      CHARIOX_ROOM_DRILL_COMPANION_TIMEOUT_MS: value }), /must be an integer/)
  }
})
