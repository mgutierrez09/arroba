import assert from "node:assert/strict"
import test from "node:test"
import { runOnboardingProviderTurn } from "./live-room-onboarding-turn.mjs"

test("onboarding waits for the submitted provider turn and releases its prompt attachment", async () => {
  let polls = 0
  const attachments = new Set()
  const requests = Object.fromEntries(["attachToSession", "submitPrompt", "detachFromSession",
    "getSessionState", "getSessionHistoryOutline"].map(name => [`${name}Request`, (...args) => ({ name, args })]))
  const result = await runOnboardingProviderTurn({
    requests, sessionId: "room", agentId: "agent", withTimeout: async promise => promise,
    waitFor: async check => {
      for (let i = 0; i < 5; i++) { const value = await check(); if (value) return value }
      throw new Error("fixture provider never settled")
    },
    client: { send: async ({ name }) => {
      if (name === "attachToSession") { attachments.add("attached"); return { SessionAttached: { attachment: { id: "attached" } } } }
      if (name === "submitPrompt") return { PromptSubmitted: { outcome: { Started: { prompt: { id: "new" } } } } }
      if (name === "detachFromSession") { attachments.clear(); return { SessionDetached: {} } }
      if (name === "getSessionState") return { SessionState: { session: { id: "room", agents: [{ id: "agent", is_processing: polls < 3 }] } } }
      if (name === "getSessionHistoryOutline") return { SessionHistoryOutline: { agents: [{ agent_id: "agent", turns: [
        { prompt_id: "old", turn_id: "old-turn", lifecycle: "completed" },
        { prompt_id: "new", turn_id: "new-turn", lifecycle: polls >= 3 ? "completed" : "open" },
      ] }] } }
      throw new Error("unexpected protocol request")
    } },
  }, { prompt: "Use the vault handle, never print its value.", poll: async () => { polls++ } })
  assert.equal(result.promptId, "new")
  assert.equal(result.turnId, "new-turn")
  assert.ok(polls >= 3)
  assert.equal(attachments.size, 0)
})

test("a failed secret interaction ends onboarding polling without retrying or exposing the reply", async () => {
  const requests = Object.fromEntries(["attachToSession", "submitPrompt", "detachFromSession", "getSessionState"]
    .map(name => [`${name}Request`, () => name]))
  let polls = 0
  await assert.rejects(runOnboardingProviderTurn({ requests, sessionId: "room", agentId: "agent",
    withTimeout: async p => p, client: { send: async name => ({
      attachToSession: { SessionAttached: { attachment: { id: "a" } } },
      submitPrompt: { PromptSubmitted: { outcome: { Started: { prompt: { id: "p" } } } } },
      detachFromSession: {}, getSessionState: { SessionState: { session: { id: "room" } } },
    })[name] },
    waitFor: async check => {
      const result = await check().catch(() => false)
      assert.ok(result, "terminal interaction failure must not be swallowed by the retrying waiter")
      return result
    },
  }, { prompt: "Use the credential handle", poll: async () => { polls++; throw new Error("private reply must not leak") } }),
  /onboarding secret interaction could not be resolved/)
  assert.equal(polls, 1)
})
