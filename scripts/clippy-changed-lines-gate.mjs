#!/usr/bin/env node

import { spawn, spawnSync } from "node:child_process"
import { mkdtemp, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { pathToFileURL } from "node:url"
import { createInterface } from "node:readline"

function primarySpan(message) {
  return (message.spans ?? []).find((span) => span.is_primary) ?? null
}

export function warningFromCompilerEvent(event) {
  if (event?.reason !== "compiler-message" || event.message?.level !== "warning") return null
  const span = primarySpan(event.message)
  return {
    code: event.message.code?.code ?? "warning",
    column: span?.column_start ?? 0,
    file: span?.file_name ?? "<unknown>",
    line: span?.line_start ?? 0,
    message: event.message.message,
  }
}

export function warningIdentity(warning) {
  return JSON.stringify([warning.file, warning.code, warning.message])
}

export function findWarningRegressions(baseWarnings, headWarnings) {
  const remainingBaseline = new Map()
  for (const warning of baseWarnings) {
    const key = warningIdentity(warning)
    remainingBaseline.set(key, (remainingBaseline.get(key) ?? 0) + 1)
  }

  const regressions = []
  for (const warning of headWarnings) {
    const key = warningIdentity(warning)
    const remaining = remainingBaseline.get(key) ?? 0
    if (remaining > 0) {
      remainingBaseline.set(key, remaining - 1)
    } else {
      regressions.push(warning)
    }
  }
  return regressions
}

async function runClippy(cwd, targetDir) {
  const child = spawn(
    "cargo",
    [
      "clippy",
      "--workspace",
      "--all-targets",
      "--all-features",
      "--message-format=json",
    ],
    {
      cwd,
      env: { ...process.env, CARGO_TARGET_DIR: targetDir },
      stdio: ["ignore", "pipe", "inherit"],
    },
  )

  const warnings = []
  let compilerError = false
  const lines = createInterface({ input: child.stdout })
  for await (const line of lines) {
    let event
    try {
      event = JSON.parse(line)
    } catch {
      continue
    }
    if (event?.reason === "compiler-message" && event.message?.level === "error") {
      compilerError = true
    }
    const warning = warningFromCompilerEvent(event)
    if (warning) warnings.push(warning)
  }

  const exitCode = await new Promise((resolve) => child.on("close", resolve))
  if (exitCode !== 0 || compilerError) {
    throw new Error(`Clippy could not analyze ${cwd} (exit ${exitCode ?? "unknown"}).`)
  }
  return warnings
}

function runGit(args, options = {}) {
  const result = spawnSync("git", args, { encoding: "utf8", ...options })
  if (result.status !== 0) {
    throw new Error(result.stderr || `git ${args.join(" ")} failed with exit ${result.status}`)
  }
  return result.stdout.trim()
}

export function clippyComparisonBase(explicitBase, env) {
  if (explicitBase?.trim()) return explicitBase.trim()
  if (env.GITHUB_BASE_REF) return `origin/${env.GITHUB_BASE_REF}`
  return env.GITHUB_EVENT_NAME === "workflow_dispatch" ? "origin/main" : "HEAD^"
}

async function main() {
  const explicitBase = process.argv[2]?.trim()
  const base = clippyComparisonBase(explicitBase, process.env)
  const repositoryRoot = runGit(["rev-parse", "--show-toplevel"])
  const targetDir = process.env.CARGO_TARGET_DIR || join(repositoryRoot, "target")
  const temporaryRoot = await mkdtemp(join(tmpdir(), "chariox-clippy-base-"))
  const baseWorktree = join(temporaryRoot, "checkout")

  let baseAdded = false
  try {
    const headWarnings = await runClippy(repositoryRoot, targetDir)
    runGit(["worktree", "add", "--detach", "--quiet", baseWorktree, base], {
      cwd: repositoryRoot,
    })
    baseAdded = true
    const baseWarnings = await runClippy(baseWorktree, targetDir)
    const regressions = findWarningRegressions(baseWarnings, headWarnings)

    if (regressions.length > 0) {
      console.error(`Clippy found ${regressions.length} warning regression(s) relative to ${base}:`)
      for (const diagnostic of regressions) {
        console.error(
          `${diagnostic.file}:${diagnostic.line}:${diagnostic.column}: ${diagnostic.code}: ${diagnostic.message}`,
        )
      }
      process.exitCode = 1
      return
    }

    console.log(
      `Clippy compared all workspace diagnostics with ${base}; no warning regressions found.`,
    )
  } finally {
    if (baseAdded) {
      const removal = spawnSync("git", ["worktree", "remove", "--force", baseWorktree], {
        cwd: repositoryRoot,
        encoding: "utf8",
      })
      if (removal.status !== 0) process.stderr.write(removal.stderr)
    }
    await rm(temporaryRoot, { force: true, recursive: true })
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    await main()
  } catch (error) {
    console.error(error instanceof Error ? error.message : error)
    process.exitCode = 1
  }
}
