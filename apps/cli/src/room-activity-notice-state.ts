import type { TranscriptEntry } from "./cli-types.js"
import { reindexTranscriptEntries, transcriptRetentionSlice } from "@chariox/kernel-client/transcript-entry-state"

export function roomActivityNoticeKey(
  sessionId: string, environmentId: string, source: "state" | "resync" | "events", cursor: number, index: number,
): string {
  return `room-environment:${encodeURIComponent(sessionId)}:${encodeURIComponent(environmentId)}:${source}:${cursor}:${index}`
}

// Room events are kernel-owned, but their TUI notices are not provider history.
// Retain this bounded view projection when provider history replaces a pane.
export function retainRoomActivityNotices(
  refreshed: TranscriptEntry[], current: readonly TranscriptEntry[], sessionId: string,
): TranscriptEntry[] {
  const prefix = `room-environment:${encodeURIComponent(sessionId)}:`
  const isRoomNotice = (entry: TranscriptEntry) => entry.role === "notice"
    && entry.mergeKey?.startsWith("room-environment:")
  const notices = new Map<string, TranscriptEntry>()
  for (const entry of [...refreshed, ...current]) {
    if (!isRoomNotice(entry) || !entry.mergeKey?.startsWith(prefix) || entry.text.length > 64 * 1024) continue
    const notice = { ...entry, turnTracking: "none" as const }
    delete notice.turnId
    notices.set(entry.mergeKey, notice)
  }
  const retained = transcriptRetentionSlice([...notices.values()], { maxEntries: 128, maxChars: 64 * 1024 }).kept
  return reindexTranscriptEntries([...refreshed.filter((entry) => !isRoomNotice(entry)), ...retained], 0)
}
