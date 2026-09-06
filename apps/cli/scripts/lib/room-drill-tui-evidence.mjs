import assert from "node:assert/strict"
import { roomActionNoticePattern } from "./room-tui-notices.mjs"

// Keep only observed Room action notices, not provider output or snapshots.
// Product transcript retention stays bounded independently of drill evidence.
export function createRoomDrillTuiEvidence(localBaseline = [], remoteBaseline = []) {
  const baseline = { local: new Set(localBaseline), remote: new Set(remoteBaseline) }
  const seen = { local: new Map(), remote: new Map() }
  let samples = 0
  let bytes = 0
  return {
    observe(snapshot, at = new Date().toISOString()) {
      samples++
      for (const side of ["local", "remote"]) {
        assert.ok(Array.isArray(snapshot[side]), `missing ${side} TUI evidence sample`)
        for (const entry of snapshot[side]) {
          if (baseline[side].has(entry.id) || typeof entry.text !== "string"
            || !/^Room action #\d+: /.test(entry.text) || seen[side].has(entry.text)) continue
          assert.ok(entry.text.length <= 1024, "Room action notice exceeds the evidence bound")
          bytes += Buffer.byteLength(entry.text)
          assert.ok(bytes <= 1024 * 1024 && seen[side].size < 4096, "TUI evidence exceeds the drill bound")
          seen[side].set(entry.text, { id: entry.id, text: entry.text, observedAt: at })
        }
      }
    },
    find(side, action) {
      const pattern = roomActionNoticePattern(action)
      for (const entry of seen[side].values()) if (pattern.test(entry.text)) return entry
      return null
    },
    summary() {
      return { samples, bytes, localNotices: seen.local.size, remoteNotices: seen.remote.size }
    },
  }
}
