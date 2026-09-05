import assert from 'node:assert/strict'
import test from 'node:test'

import {
  collectPumpedTerminalOutput,
  terminalText,
} from './relay-runtime-terminal-output.mjs'

test('collects terminal output returned by an explicit pump', () => {
  const records = []

  collectPumpedTerminalOutput({
    TerminalOutput: {
      records: [
        { kind: 'PromptEcho', bytes: [...Buffer.from('ignored')] },
        { kind: 'ProviderOutput', bytes: [...Buffer.from('RELAY_OK')] },
      ],
    },
  }, records)

  assert.equal(terminalText(records), 'RELAY_OK')
})

test('accepts compatibility terminal-output responses without duplicating event records', () => {
  const existing = { event: 'terminal_output', records: [{ kind: 'ProviderOutput', text: 'first' }] }
  const records = [existing]

  collectPumpedTerminalOutput({
    TerminalOutputPumped: {
      records: [{ kind: 'ProviderOutput', text: ' second' }],
    },
  }, records)
  collectPumpedTerminalOutput(null, records)

  assert.equal(terminalText(records), 'first second')
  assert.equal(records[0], existing)
})
