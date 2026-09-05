function responseVariant(response, key) {
  return response?.[key] ?? null
}

export function collectPumpedTerminalOutput(response, eventBucket) {
  const output = responseVariant(response, 'TerminalOutput')
    ?? responseVariant(response, 'TerminalOutputPumped')
  const records = output?.records ?? []
  if (records.length === 0) return 0
  eventBucket.push({
    event: 'terminal_output',
    records,
    observed_at_ms: Date.now(),
  })
  return records.length
}

export function terminalText(eventBucket) {
  return eventBucket
    .filter((event) => event.event === 'terminal_output')
    .flatMap((event) => event.records ?? [])
    .filter((record) => record.kind !== 'PromptEcho')
    .map((record) => Array.isArray(record.bytes)
      ? Buffer.from(record.bytes).toString('utf8')
      : String(record.text ?? record.data ?? record.output ?? ''))
    .join('')
}
