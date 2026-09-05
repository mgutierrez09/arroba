import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

const execFile = promisify(execFileWithCallback)
const script = fileURLToPath(new URL("./live-resource-exhaustion-fault-drill.mjs", import.meta.url))

test("resource exhaustion dry-run records two isolated commands outside the repository", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-resource-exhaustion-"))
  const reportPath = path.join(root, "report.json")
  try {
    await execFile(process.execPath, [script, "--dry-run", "--report", reportPath])
    const report = JSON.parse(await readFile(reportPath, "utf8"))
    assert.equal(report.schema, "chariox.resource_exhaustion_fault_drill.v1")
    assert.equal(report.status, "dry-run")
    assert.deepEqual(report.caseIds, ["fault.resource-exhaustion", "cleanup.resources"])
    assert.deepEqual(report.commands.map((command) => command.mode), ["file-descriptor", "process"])
    assert(report.commands.every((command) => command.name === "bash"))
    const processCommand = report.commands.find((command) => command.mode === "process")
    assert.match(processCommand.args[1], /current_tasks=/)
    assert.match(processCommand.args[1], /current_tasks \+ \$1/)
    assert.equal(processCommand.args[3], "32")
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("resource exhaustion drill rejects repository-local evidence", async () => {
  const reportPath = path.join(path.dirname(script), "resource-exhaustion-report.json")
  await assert.rejects(
    execFile(process.execPath, [script, "--dry-run", "--report", reportPath]),
    /evidence must stay outside repositories/,
  )
})
