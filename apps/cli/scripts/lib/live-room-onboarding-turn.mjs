import assert from "node:assert/strict"
import { waitForRoomProviderSettlement } from "./live-room-provider-settlement.mjs"

export async function runOnboardingProviderTurn(input, { prompt, poll }) {
  const { client, requests, sessionId, agentId } = input
  const attachment = await client.send(requests.attachToSessionRequest(sessionId, "office-onboarding"))
  const attachmentId = attachment?.SessionAttached?.attachment?.id
  assert.ok(attachmentId, "onboarding prompt attachment missing")
  let submitted
  try {
    submitted = await client.send(requests.submitPromptRequest(sessionId, attachmentId, agentId, prompt, []))
  } finally { await client.send(requests.detachFromSessionRequest(attachmentId)) }
  const outcome = submitted?.PromptSubmitted?.outcome
  const promptId = (outcome?.Started ?? outcome?.Queued)?.prompt?.id
  assert.ok(promptId, "onboarding prompt identity missing")
  const progress = await input.waitFor(async () => {
    const state = await input.withTimeout(client.send(requests.getSessionStateRequest(sessionId)), 5000,
      "onboarding provider state")
    assert.ok(state?.SessionState, "onboarding provider state missing")
    try { await poll(state.SessionState) }
    catch { return { interactionFailed: true } }
    const outline = await input.withTimeout(client.send(requests.getSessionHistoryOutlineRequest(
      sessionId, [agentId], 2)), 5000, "onboarding provider history")
    const turn = outline?.SessionHistoryOutline?.agents?.find(a => a.agent_id === agentId)?.turns
      ?.find(t => t.prompt_id === promptId)
    return turn?.lifecycle === "completed"
  }, 300000, "onboarding provider turn did not complete")
  assert.ok(!progress.interactionFailed, "onboarding secret interaction could not be resolved")
  return waitForRoomProviderSettlement(input, agentId, promptId)
}
