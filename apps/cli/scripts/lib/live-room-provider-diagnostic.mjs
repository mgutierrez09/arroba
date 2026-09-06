const entryKinds = ["user_prompt", "provider_output", "provider_reasoning", "provider_tool", "provider_error", "provider_status", "notice"]
const actionStates = ["queued", "running", "completed", "failed", "cancelled"]
const diagnosticTools = new Set([
  "list_mcp_resources", "list_mcp_resource_templates", "list_slices", "tool_search",
  "slice_screen_status", "slice_screenshot", "slice_mouse", "slice_keyboard",
  "slice_browser_find", "slice_browser_click", "slice_browser_fill", "slice_browser_submit",
])
// Presence signals narrow an otherwise unclassified failure. They are not a
// diagnosis, and are taken only from provider errors, never the user's prompt.
const errorSignals = [
  "account", "authentication", "certificate", "configuration", "credentials",
  "denied", "dns", "expired", "keychain", "launch", "login", "model", "network",
  "permission", "protocol", "provider", "quota", "refresh", "request", "response",
  "sandbox", "session", "thread", "timeout", "token", "transport", "unsupported",
  "vault", "workspace",
].map(signal => [signal, new RegExp(`\\b${signal}\\b`, "i")])
const diagnosticPatterns = [
  ["endpoint_unhealthy", /(?:codex|claude|opencode)_endpoint_unhealthy/i],
  ["provider_launch", /provider launch/i],
  ["unauthorized", /unauthorized|authentication failed|invalid api key/i],
  ["auth_refresh_failed", /token refresh failed/i],
  ["rate_limit", /rate.?limit|too many requests/i],
  ["usage_limit", /hit your (?:usage )?limit|out of (?:extra )?usage|usage limit (?:reached|exceeded)|(?:don['’]t|do not) have usage credits/i],
  ["model_unavailable", /model.{0,100}(?:not found|not available|unsupported|does not exist|not supported|do not have access)|ProviderModelNotFoundError/i],
  ["tool_unavailable", /(?:unknown tool|tool not found|no such tool)/i],
  ["permission_denied", /permission denied|not permitted/i],
  ["connection_failed", /connection refused|econnrefused|connection reset/i],
  ["missing_api_key", /api.?key.{0,40}(?:missing|required|not set)|(?:missing|required).{0,40}api.?key/i],
  ["mcp_setup", /opencode_mcp_ready|MCP server.{0,160}(?:failed|needs_auth|needs_client_registration)|timed out waiting for OpenCode MCP/i],
  ["unknown_provider_error", /OpenCode reported an unknown (?:session|assistant) error|Claude Code reported an error|Codex reported an unknown error|Codex turn failed/i],
  ["provider_request_failed", /OpenCode request failed|\bAPI Error\b|unexpected status [45]\d\d\b/i],
  ["tls_error", /certificate.{0,40}(?:failed|invalid|unknown|expired)|(?:tls|ssl).{0,40}(?:error|failed)/i],
  ["dns_error", /\bdns error\b|failed to resolve|name resolution failed/i],
  ["auth_required", /not logged in|please (?:run )?login|authentication required/i],
  ["provider_protocol_error", /JSON-RPC error|invalid params|method not found/i],
  ["invalid_tool_schema", /invalid.{0,40}schema|schema.{0,40}(?:invalid|not supported)/i],
  ["missing_module", /Cannot find (?:module|package)|ModuleNotFound/i],
  ["context_overflow", /ContextOverflowError|context.{0,30}(?:too long|exceed)/i],
  ["invalid_configuration", /config.{0,40}(?:invalid|parse error)|invalid.{0,40}config/i],
  ["resource_exhausted", /no space left|out of memory|ENOMEM|ENOSPC/i],
]
const counters = (keys) => Object.fromEntries(keys.map((key) => [key, 0]))
const known = (value, values) => values.includes(value) ? value : "unknown"

// Evidence contains only fixed enums, booleans and bounded counters. Never copy
// prompt/output text, tool arguments, endpoints, error messages or dynamic keys.
export async function captureRoomProviderDiagnostic(input) {
  const { client, requests, sessionId, agentId } = input
  const deadline = Date.now() + 8_000
  const result = {
    schema: "chariox.room_provider_diagnostic.v1",
    agentState: "unknown", activityStatus: "unknown", promptStatus: "unknown", activeTurnPhase: "unknown",
    turns: [], entryCounts: counters([...entryKinds, "unknown"]),
    blobCounts: counters([...entryKinds, "unknown"]), actionCounts: counters([...actionStates, "unknown"]),
    computerToolMentioned: false, observedTools: [], browserFindResults: [], truncated: false, codes: [], providerErrorSignals: [],
  }
  const codes = new Set()
  const observedTools = new Set()
  const observedErrorSignals = new Set()
  let inspectedChars = 0
  let inspectedEntries = 0
  let loadedBlobs = 0
  let requestedBlobChars = 0
  const loadedIds = new Set()
  const request = async (value, variant) => {
    const remaining = deadline - Date.now()
    if (remaining <= 0) throw new Error("diagnostic deadline")
    const response = await input.withTimeout(client.send(value), remaining, "provider diagnostic deadline")
    if (!response?.[variant]) throw new Error("unexpected diagnostic response")
    return response[variant]
  }
  const section = async (code, action) => {
    try { await action() } catch { codes.add(code) }
  }
  const inspectText = (value, tool = false, providerError = false) => {
    if (typeof value !== "string") return
    const text = value.slice(0, Math.min(4096, Math.max(0, 131072 - inspectedChars)))
    inspectedChars += text.length
    if (text.length < value.length) result.truncated = true
    for (const [code, pattern] of diagnosticPatterns) if (pattern.test(text)) codes.add(code)
    if (providerError) {
      for (const [signal, pattern] of errorSignals) if (pattern.test(text)) observedErrorSignals.add(signal)
    }
    if (tool && /\bslice_mouse\b/.test(text)) result.computerToolMentioned = true
    return text
  }
  const inspectEntry = (item, seen, discovered) => {
    if (!item?.entry) return
    if (inspectedEntries >= 256) { result.truncated = true; return }
    inspectedEntries += 1
    const kind = known(item.entry.kind, entryKinds)
    // Deduplicate the count, not inspection: the outline preview and hydrated
    // blob can contain different fragments of the same history entry.
    if (!Number.isSafeInteger(item.entry_index) || !seen.has(item.entry_index)) result.entryCounts[kind] += 1
    if (Number.isSafeInteger(item.entry_index)) {
      seen.add(item.entry_index)
    }
    const text = inspectText(item.entry.text, kind === "provider_tool", kind === "provider_error")
    if (kind === "provider_tool") {
      try {
        const value = JSON.parse(text)
        const tool = typeof value?.tool === "string"
          ? value.tool.replace(/^(?:mcp__chariox__|chariox\.|chariox_)/, "") : ""
        if (diagnosticTools.has(tool)) observedTools.add(tool)
        if (tool === "slice_mouse") result.computerToolMentioned = true
        if (tool === "slice_browser_find" && value.status === "completed") {
          const output = typeof value.output === "string" ? JSON.parse(value.output) : value.output
          const matches = output?.browser?.matches ?? output?.payload?.browser?.matches
          if (Array.isArray(matches) && (!Number.isSafeInteger(item.entry_index) || !discovered.has(item.entry_index))) {
            if (Number.isSafeInteger(item.entry_index)) discovered.add(item.entry_index)
            if (result.browserFindResults.length < 16) {
              const query = [
                ["Browser sample", "field"], ["Replace Browser field", "replacement"], ["Submit Browser form", "submit"],
              ].find(([text]) => text === value.input?.query)?.[1] ?? "other"
              result.browserFindResults.push({ query, matches: Math.min(matches.length, 100) })
            } else result.truncated = true
          }
        }
      } catch { /* A truncated or non-JSON preview cannot prove a tool identity. */ }
    }
  }
  await section("state_unavailable", async () => {
    const state = await request(requests.getSessionStateRequest(sessionId), "SessionState")
    const agent = state.session?.agents?.find((item) => item.id === agentId)
    const activity = state.agent_activity?.[agentId]
    result.agentState = known(agent?.state, ["Idle", "Working", "Focused", "Error"])
    result.activityStatus = known(activity?.status, ["idle", "working", "error"])
    result.promptStatus = known(activity?.prompt_status, ["none", "queued", "dispatching", "running", "cancelling", "settling"])
    result.activeTurnPhase = known(activity?.active_turn?.phase, ["accepted", "awaiting_first_output", "streaming", "settling"])
  })
  await section("actions_unavailable", async () => {
    const history = await request(requests.listRoomEnvironmentActionHistoryRequest(sessionId, null, 100), "RoomEnvironmentActionHistoryListed")
    for (const action of (history.page?.actions ?? []).slice(0, 100)) {
      if (action.actor_id === `agent:${agentId}`) result.actionCounts[known(action.state, actionStates)] += 1
    }
  })
  await section("history_unavailable", async () => {
    const outline = await request(requests.getSessionHistoryOutlineRequest(sessionId, [agentId], 2), "SessionHistoryOutline")
    const turns = outline.agents?.find((item) => item.agent_id === agentId)?.turns ?? []
    if (turns.length > 2) result.truncated = true
    for (const turn of turns.slice(0, 2)) {
      const seen = new Set()
      const discovered = new Set()
      result.turns.push({ lifecycle: known(turn.lifecycle, ["open", "completed", "cancelled"]) })
      inspectEntry(turn.user_prompt, seen, discovered)
      for (const item of (turn.entries ?? []).slice(0, 256)) inspectEntry(item, seen, discovered)
      if ((turn.entries?.length ?? 0) > 256) result.truncated = true
      inspectEntry(turn.summary, seen, discovered)
      if ((turn.blobs?.length ?? 0) > 16) result.truncated = true
      for (const blob of (turn.blobs ?? []).slice(0, 16)) {
        result.blobCounts[known(blob.kind, entryKinds)] += 1
        inspectText(blob.summary, blob.kind === "provider_tool", blob.kind === "provider_error")
        if (loadedIds.has(blob.blob_id)) continue
        if (typeof blob.blob_id !== "string" || !Number.isSafeInteger(blob.total_chars)
          || blob.total_chars < 0 || blob.total_chars > 32768 || loadedBlobs >= 8
          || requestedBlobChars + blob.total_chars > 131072 || Date.now() >= deadline) {
          result.truncated = true
          continue
        }
        loadedIds.add(blob.blob_id)
        loadedBlobs += 1
        requestedBlobChars += blob.total_chars
        await section("blob_unavailable", async () => {
          const content = await request(requests.getSessionHistoryBlobContentRequest(sessionId, agentId, blob.blob_id), "SessionHistoryBlobContent")
          for (const item of (content.entries ?? []).slice(0, 256)) inspectEntry(item, seen, discovered)
          if ((content.entries?.length ?? 0) > 256) result.truncated = true
        })
      }
    }
  })
  result.codes = [...codes].sort()
  result.observedTools = [...observedTools].sort()
  result.providerErrorSignals = [...observedErrorSignals].sort()
  return result
}
