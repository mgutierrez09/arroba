import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

import {
  PROCESS_LIMIT_HEADROOM,
  PROCESS_LIMIT_SHELL,
  parseResourceExhaustionProbes,
} from "./resource-exhaustion-fault-drill.mjs"

const execFile = promisify(execFileWithCallback)
const probe = fileURLToPath(new URL("./resource-exhaustion-probe.mjs", import.meta.url))

test("lowered operating-system limits fail boundedly without starving an established terminal lane", async () => {
  const marker = `chariox-resource-probe-test-${process.pid}`
  const commands = [
    ["ulimit -n \"$1\"; exec \"$2\" \"$3\" --mode file-descriptor --marker \"$4\"", "64"],
    [`${PROCESS_LIMIT_SHELL}; exec "$2" "$3" --mode process --marker "$4"`, PROCESS_LIMIT_HEADROOM],
  ]
  const outputs = []
  for (const [script, limit] of commands) {
    const result = await execFile("bash", ["-c", script, "chariox-resource-probe", limit, process.execPath, probe, marker], {
      cwd: path.dirname(probe),
      timeout: 10_000,
    })
    outputs.push(result.stdout)
  }
  const parsed = parseResourceExhaustionProbes(outputs)
  assert.equal(parsed.fileDescriptorLimitEnforced, true)
  assert.equal(parsed.processLimitEnforced, true)
  assert.equal(parsed.terminalLaneLive, true)
  assert.equal(parsed.cleanupComplete, true)
})
