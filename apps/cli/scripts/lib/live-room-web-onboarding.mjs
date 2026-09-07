import assert from "node:assert/strict"
import { serveWebOnboarding } from "./web-onboarding-channel.mjs"
import { runRoomOnboarding } from "./live-room-onboarding.mjs"

// The Web process chooses an already-visible agent. Only the owning OSS drill
// can validate it and supply the private interaction/cleanup context.
export async function runRoomWebOnboarding(input) {
  assert.equal(input.options.officeScenario, "onboarding", "Web onboarding revocation is not yet observed")
  return serveWebOnboarding({
    directory: input.directory, evidenceRoot: input.evidenceRoot,
    sessionId: input.sessionId, environmentId: input.environmentId,
    timeoutMs: input.timeoutMs, sleep: input.sleep, signal: input.signal,
    onPoll: input.sampleTuis,
    validateAgent: async agentId => {
      const state = await input.withTimeout(input.client.send(input.requests.getSessionStateRequest(input.sessionId)),
        5000, "Web onboarding provider configuration")
      const agent = state?.SessionState?.session?.agents?.find(value => value.id === agentId)
      assert.ok(agent && agent.session_id === input.sessionId && agent.provider === input.options.provider
        && agent.model === input.options.model && (agent.account_profile ?? "default") === input.options.accountProfile,
      "authoritative onboarding provider/model/profile/Room differs")
      assert.equal(agent.is_processing, false, "onboarding agent must be idle")
      const result = await input.withTimeout(input.client.send(input.requests.listSlicesRequest()),
        5000, "Web onboarding slice membership")
      assert.ok(result?.SlicesListed?.slices?.some(slice => slice.id === input.sliceId && slice.agent_ids?.includes(agentId)),
        "onboarding agent must belong to the intended slice")
    },
    run: ({ agentId, observePhase }) => runRoomOnboarding({ ...input, agentId, observePhase,
      waitFor: (check, ...args) => input.waitFor(async () => {
        await input.sampleTuis?.()
        return check()
      }, ...args),
    }),
  })
}
