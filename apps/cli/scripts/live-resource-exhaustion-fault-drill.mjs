#!/usr/bin/env node

import { execFile as execFileWithCallback } from "node:child_process"
import { randomUUID } from "node:crypto"
import { chmod, mkdir, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

import {
  PROCESS_LIMIT_HEADROOM,
  PROCESS_LIMIT_SHELL,
  RESOURCE_EXHAUSTION_CASE_IDS,
  boundedEvidenceText,
  parseResourceExhaustionProbes,
} from "./lib/resource-exhaustion-fault-drill.mjs"

const execFile = promisify(execFileWithCallback)
const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, "..", "..", "..")
const probe = path.join(scriptDir, "lib", "resource-exhaustion-probe.mjs")
const marker = `chariox-resource-probe-${randomUUID()}`
const commands = [
  probeCommand("file-descriptor", "ulimit -n \"$1\"", "64"),
  probeCommand("process", PROCESS_LIMIT_SHELL, PROCESS_LIMIT_HEADROOM),
]

const options = parseArgs(process.argv.slice(2))
if (!options.help) await run(options)

function probeCommand(mode, limitCommand, limit) {
  return {
    mode,
    name: "bash",
    args: [
      "-c",
      `${limitCommand}; exec \"$2\" \"$3\" --mode \"$4\" --marker \"$5\"`,
      "chariox-resource-probe",
      limit,
      process.execPath,
      probe,
      mode,
      marker,
    ],
  }
}

async function run({ dryRun, reportPath: requestedReport }) {
  const reportPath = externalReportPath(requestedReport ?? defaultReportPath(), repoRoot)
  const report = {
    schema: "chariox.resource_exhaustion_fault_drill.v1",
    startedAt: new Date().toISOString(),
    status: dryRun ? "dry-run" : "running",
    caseIds: RESOURCE_EXHAUSTION_CASE_IDS,
    source: { commit: (await execFile("git", ["rev-parse", "HEAD"], { cwd: repoRoot })).stdout.trim() },
    commands,
    evidenceRoot: path.dirname(reportPath),
    resources: [],
    cleanup: null,
  }
  let failure = null
  try {
    if (!dryRun) {
      report.resources.push(await resourceSnapshot("before"))
      const outputs = []
      for (const command of commands) {
        const result = await execFile(command.name, command.args, {
          cwd: repoRoot,
          maxBuffer: 256 * 1024,
          timeout: 10_000,
        })
        outputs.push(result.stdout)
      }
      report.probe = parseResourceExhaustionProbes(outputs)
      report.status = "passed"
    }
  } catch (error) {
    failure = error
    report.status = "failed"
    report.failure = boundedEvidenceText(error instanceof Error ? error.message : error)
  } finally {
    const remaining = dryRun ? [] : await matchingProcesses(marker)
    report.cleanup = { ownedProcessesAbsent: remaining.length === 0, remaining }
    if (!dryRun) report.resources.push(await resourceSnapshot("after-cleanup"))
    if (remaining.length > 0 && !failure) {
      failure = new Error("resource exhaustion drill left owned processes running")
      report.status = "failed"
      report.failure = failure.message
    }
    report.completedAt = new Date().toISOString()
    await writeReport(reportPath, report)
  }
  console.log(JSON.stringify({ status: report.status, reportPath }))
  if (failure) throw failure
}

function parseArgs(argv) {
  const options = { dryRun: false, help: false, reportPath: null }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--dry-run") options.dryRun = true
    else if (arg === "--help" || arg === "-h") options.help = true
    else if (arg === "--report") options.reportPath = readValue(argv, ++index, arg)
    else if (arg.startsWith("--report=")) options.reportPath = arg.slice("--report=".length)
    else throw new Error(`unknown argument: ${arg}`)
  }
  if (options.help) {
    console.log([
      "Usage: node live-resource-exhaustion-fault-drill.mjs [options]",
      "",
      "Runs isolated process and file-descriptor exhaustion probes.",
      "",
      "  --report PATH  Absolute external JSON report path",
      "  --dry-run      Record the exact commands without running them",
      "  --help         Show this help",
    ].join("\n"))
  }
  return options
}

function readValue(argv, index, flag) {
  const value = argv[index]
  if (!value || value.startsWith("--")) throw new Error(`${flag} requires a value`)
  return value
}

function externalReportPath(value, root) {
  if (!path.isAbsolute(value)) throw new Error("evidence report must be absolute")
  const normalized = path.normalize(value)
  const relative = path.relative(root, normalized)
  const withinRepo = relative === "" || (
    relative !== ".." && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative)
  )
  if (withinRepo) throw new Error("evidence must stay outside repositories")
  return normalized
}

function defaultReportPath(now = new Date()) {
  const stamp = now.toISOString().replace(/[:.]/g, "-")
  return path.join(os.homedir(), ".codex", "evidence", "browser-computer-use", "resource-exhaustion", stamp, "report.json")
}

async function matchingProcesses(processMarker) {
  try {
    const result = await execFile("pgrep", ["-f", processMarker], { timeout: 5_000 })
    return result.stdout.trim().split("\n").filter(Boolean)
  } catch (error) {
    if (error?.code === 1) return []
    throw error
  }
}

async function resourceSnapshot(label) {
  const [memory, swap, disk] = await Promise.all([
    execFile("memory_pressure", ["-Q"], { timeout: 10_000 }).catch(() => null),
    execFile("sysctl", ["vm.swapusage"], { timeout: 10_000 }).catch(() => null),
    execFile("df", ["-k", "/System/Volumes/Data"], { timeout: 10_000 }).catch(() => null),
  ])
  return {
    label,
    at: new Date().toISOString(),
    freeMemoryBytes: os.freemem(),
    loadAverage: os.loadavg(),
    memoryPressure: memory ? boundedEvidenceText(memory.stdout, 1_000).trim() : null,
    swap: swap ? boundedEvidenceText(swap.stdout, 1_000).trim() : null,
    disk: disk ? disk.stdout.trim().split("\n").at(-1) : null,
  }
}

async function writeReport(reportPath, report) {
  await mkdir(path.dirname(reportPath), { recursive: true, mode: 0o700 })
  await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`, { mode: 0o600 })
  await chmod(reportPath, 0o600)
}
