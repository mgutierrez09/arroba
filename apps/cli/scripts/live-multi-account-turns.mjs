export function completedAccountTurns(agents, expected) {
  const receipts = []
  for (const { agentId, profileId, marker } of expected) {
    const turn = agents.find(agent => agent.agent_id === agentId)?.turns.find(turn => turn.user_prompt?.entry?.text.includes(marker))
    if (!turn) return null
    const entries = [...turn.entries, ...(turn.summary ? [turn.summary] : [])].map(page => page.entry)
    if (turn.lifecycle === 'cancelled') throw new Error(`Account test turn cancelled for ${agentId}`)
    if (entries.some(entry => entry.kind === 'provider_error')) throw new Error(`Provider error in account test turn for ${agentId}`)
    if (turn.lifecycle !== 'completed') return null
    const output = entries.filter(entry => entry.kind === 'provider_output')
    if (!output.map(entry => entry.text).join('').includes(marker)) return null
    const providerRunIds = [...new Set(output.map(entry => entry.provider_run_id).filter(Boolean))]
    if (!providerRunIds.length) throw new Error(`Missing provider-run attribution for ${agentId}`)
    receipts.push({ agentId, profileId, turnId: turn.turn_id, promptId: turn.prompt_id, providerRunIds, marker })
  }
  return receipts
}
