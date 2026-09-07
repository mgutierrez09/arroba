import assert from "node:assert/strict"

// Only fixed categories leave the private history boundary. Provider errors
// can contain selectors, URLs or other user-controlled/private values.
const errorPatterns = [
  ["field_unavailable", /not found or is not fillable|requires a focused fillable|stale.*(?:field|reference)/i],
  ["target_mismatch", /does not match expected (?:host|URL)|origin.*mismatch/i],
  ["credential_unavailable", /(?:credential|secret).*(?:not found|unknown|missing|unavailable)/i],
  ["credential_scope", /(?:host|use|credential).*(?:not allowed|denied|forbidden)/i],
  ["vault_locked", /vault.*(?:locked|unlock)/i],
  ["invalid_arguments", /invalid.*arguments|missing field/i],
  ["timeout", /timed out|timeout/i],
  ["controller_home_route_required", /browser_controller_scope_denied: provisioned slice controller requires the home Room relay path/],
  ["invalid_environment_lifecycle", /environment_invalid_lifecycle_transition/],
]

// Complete kernel-owned records are required to prove which credential tool
// caused an action. Outline summaries alone cannot provide that evidence.
export async function readOnboardingTurnTools(input, promptId) {
  const { client, requests, sessionId, agentId } = input
  const request = value => input.withTimeout(client.send(value), 5000, "onboarding tool history")
  const outline = await request(requests.getSessionHistoryOutlineRequest(sessionId, [agentId], 2))
  const turn = outline?.SessionHistoryOutline?.agents?.find(a => a.agent_id === agentId)?.turns
    ?.find(t => t.prompt_id === promptId)
  assert.equal(turn?.lifecycle, "completed", "onboarding prompt is not complete")
  const rows = new Map()
  const add = entries => {
    for (const row of entries ?? []) {
      if (row.entry?.kind !== "provider_tool") continue
      assert.ok(Number.isSafeInteger(row.entry_index), "onboarding tool lacks a history identity")
      rows.set(row.entry_index, row)
    }
  }
  add(turn.entries)
  add(turn.summary ? [turn.summary] : [])
  let chars = 0
  const blobs = turn.blobs ?? []
  assert.ok(blobs.length <= 64, "onboarding tool history exceeds blob bound")
  for (const blob of blobs) {
    if (blob.kind && blob.kind !== "provider_tool") continue
    assert.ok(Number.isSafeInteger(blob.total_chars) && blob.total_chars >= 0
      && blob.total_chars <= 1048576, "onboarding tool blob exceeds evidence bound")
    chars += blob.total_chars
    assert.ok(chars <= 4194304, "onboarding tool history exceeds evidence bound")
    const response = await request(requests.getSessionHistoryBlobContentRequest(sessionId, agentId, blob.blob_id))
    const content = response?.SessionHistoryBlobContent
    assert.equal(content?.blob_id, blob.blob_id, "onboarding tool blob identity differs")
    assert.ok(Array.isArray(content.entries) && content.entries.length <= 4096, "invalid onboarding tool history")
    add(content.entries)
  }
  assert.ok(rows.size <= 4096, "onboarding tool count exceeds evidence bound")
  return [...rows.values()].sort((a, b) => a.entry_index - b.entry_index).map(row => {
    try {
      const tool = JSON.parse(row.entry.text)
      assert.equal(typeof tool.tool, "string")
      const output = typeof tool.output === "string" ? JSON.parse(tool.output) : tool.output
      const errorCodes = typeof tool.error === "string" && tool.error
        ? errorPatterns.filter(([, pattern]) => pattern.test(tool.error)).map(([code]) => code) : []
      if (tool.error && errorCodes.length === 0) errorCodes.push("unclassified_tool_error")
      return { name: tool.tool.replace(/^(?:(?:mcp__chariox__|chariox\.|chariox_))+/, ""),
        status: tool.status, input: tool.input, output: output?.structuredContent ?? output?.payload ?? output, errorCodes }
    } catch { throw new Error("onboarding tool history is incomplete or malformed") }
  })
}
