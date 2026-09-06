import assert from 'node:assert/strict'
import test from 'node:test'
import { completedAccountTurns } from './live-multi-account-turns.mjs'

const expected = [{ agentId: 'a', marker: 'UNIQUE_A', profileId: 'profile-a' }]
const history = (entries, lifecycle = 'completed') => [{ agent_id: 'a', turns: [{
  turn_id: 'turn-a', prompt_id: 'prompt-a', lifecycle,
  user_prompt: { entry: { kind: 'user_prompt', text: 'Reply UNIQUE_A' } },
  entries: entries.map(entry => ({ entry })), blobs: [],
}] }]

test('an idle or completed turn without provider output is not success', () => {
  assert.equal(completedAccountTurns(history([]), expected), null)
})
test('the prompt echo cannot satisfy the output assertion', () => {
  assert.equal(completedAccountTurns(history([{ kind: 'user_prompt', text: 'UNIQUE_A' }]), expected), null)
})
test('provider errors and cancelled turns fail the drill', () => {
  assert.throws(() => completedAccountTurns(history([{ kind: 'provider_error', text: 'exhausted' }]), expected), /Provider error/)
  assert.throws(() => completedAccountTurns(history([], 'cancelled'), expected), /cancelled/)
})
test('completed provider output records the exact turn and run identity', () => {
  assert.deepEqual(completedAccountTurns(history([{ kind: 'provider_output', text: 'UNIQUE_A', provider_run_id: 'run-a' }]), expected), [{ agentId: 'a', profileId: 'profile-a', turnId: 'turn-a', promptId: 'prompt-a', providerRunIds: ['run-a'], marker: 'UNIQUE_A' }])
})
test('a prior turn cannot satisfy the next account-switch turn', () => {
  assert.equal(completedAccountTurns(history([{ kind: 'provider_output', text: 'UNIQUE_A', provider_run_id: 'run-a' }]), [{ ...expected[0], marker: 'UNIQUE_B' }]), null)
})
test('completed output in the history summary retains provider-run attribution', () => {
  const outline = history([])
  outline[0].turns[0].summary = { entry: { kind: 'provider_output', text: 'UNIQUE_A', provider_run_id: 'run-a' } }
  assert.deepEqual(completedAccountTurns(outline, expected)?.[0].providerRunIds, ['run-a'])
  outline[0].turns[0].summary.entry.kind = 'user_prompt'
  assert.equal(completedAccountTurns(outline, expected), null)
})
