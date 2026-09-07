import assert from "node:assert/strict"
import test from "node:test"
import { readOnboardingTurnTools } from "./live-room-onboarding-history.mjs"

test("onboarding classifies provider tool errors without retaining private error text", async () => {
  const tools = await readOnboardingTurnTools({ sessionId: "room", agentId: "agent", withTimeout: async p => p,
    onToolError: () => { throw new Error("private error text escaped the history boundary") },
    requests: { getSessionHistoryOutlineRequest: () => ({}) },
    client: { send: async () => ({ SessionHistoryOutline: { agents: [{ agent_id: "agent", turns: [{
      prompt_id: "current", lifecycle: "completed", entries: [{ entry_index: 1, entry: {
        kind: "provider_tool", text: JSON.stringify({ tool: "paste_secret_to_slice", status: "error",
          error: "slice browser field `private-value` was not found or is not fillable" }),
      } }],
    }] }] } }) } }, "current")
  assert.deepEqual(tools[0].errorCodes, ["field_unavailable"])
  assert.equal(JSON.stringify(tools).includes("private-value"), false)
})

test("onboarding reads complete tool records from the exact prompt, not a truncated preview", async () => {
  const requests = { getSessionHistoryOutlineRequest: (...args) => ({ name: "outline", args }),
    getSessionHistoryBlobContentRequest: (...args) => ({ name: "blob", args }) }
  const tools = await readOnboardingTurnTools({ sessionId: "room", agentId: "agent", requests,
    withTimeout: async p => p,
    client: { send: async ({ name }) => name === "outline" ? { SessionHistoryOutline: { agents: [{ agent_id: "agent", turns: [
      { prompt_id: "old", entries: [{ entry: { kind: "provider_tool", text: '{"tool":"old"}' } }] },
      { prompt_id: "current", lifecycle: "completed", entries: [{ entry_index: 2, entry: { kind: "provider_tool", text: "truncated" } }],
        blobs: [{ blob_id: "blob", total_chars: 200 }] },
    ] }] } } : { SessionHistoryBlobContent: { blob_id: "blob", entries: [{ entry_index: 2, entry: {
      kind: "provider_tool", text: JSON.stringify({ tool: "mcp__chariox__paste_secret_to_slice", status: "completed",
        input: { credential_id: "mail" }, output: { action_id: "action-3" } }),
    } }] } } },
  }, "current")
  assert.equal(tools.length, 1)
  assert.equal(tools[0].name, "paste_secret_to_slice")
  assert.equal(tools[0].output.action_id, "action-3")
})

test("onboarding refuses incomplete tool history rather than accepting summaries", async () => {
  for (const blob of [null, { blob_id: "large", total_chars: 1048577 }]) {
    const input = { sessionId: "room", agentId: "agent", withTimeout: async p => p,
      requests: { getSessionHistoryOutlineRequest: () => ({}) },
      client: { send: async () => ({ SessionHistoryOutline: { agents: [{ agent_id: "agent", turns: [{
        prompt_id: "current", lifecycle: "completed", blobs: blob ? [blob] : [],
        entries: [{ entry_index: 1, entry: { kind: "provider_tool", text: "truncated credential history" } }],
      }] }] } }) } }
    await assert.rejects(readOnboardingTurnTools(input, "current"), /onboarding tool/)
  }
})
