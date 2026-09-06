import assert from "node:assert/strict"
import test from "node:test"

import { updateAgentSubstitutesRequest } from "./ipc-agent-requests.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"

test("agent substitute add request carries the chosen account profile", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 307)
  assert.deepEqual(
    updateAgentSubstitutesRequest({
      sessionId: "session-1",
      agentId: "agent-1",
      action: {
        Add: {
          provider: "codex",
          model: "gpt-5.4",
          variant: "medium",
          account_profile: "work",
        },
      },
    }),
    {
      UpdateAgentSubstitutes: {
        session_id: "session-1",
        agent_id: "agent-1",
        action: {
          Add: {
            provider: "codex",
            model: "gpt-5.4",
            variant: "medium",
            account_profile: "work",
          },
        },
      },
    },
  )
})

test("agent substitute add request omits account profile for default semantics", () => {
  const request = updateAgentSubstitutesRequest({
    sessionId: "session-1",
    agentId: "agent-1",
    action: { Add: { provider: "codex", model: "gpt-5.4" } },
  })
  const add = (request.UpdateAgentSubstitutes as { action: { Add: Record<string, unknown> } })
    .action.Add
  assert.equal("account_profile" in add, false)
})
