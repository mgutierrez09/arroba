import { strict as assert } from "node:assert"
import test from "node:test"

import type { CharioxLogger } from "./logging.js"
import {
  createCliProcessLoggerRegistry,
  formatCliError,
  redactCliStartupArgs,
} from "./cli-process-logging.js"

test("CLI process logger registry returns child loggers after initialization", () => {
  const children: Array<{ component: string, fields: Record<string, unknown> }> = []
  const rootLogger = {
    child(component: string, fields: Record<string, unknown>) {
      children.push({ component, fields })
      return rootLogger
    },
  } as unknown as CharioxLogger
  const registry = createCliProcessLoggerRegistry({
    createLogger: () => rootLogger,
  })

  assert.equal(registry.getLogger("cli.main"), null)
  assert.equal(registry.initialize("cli"), rootLogger)
  assert.equal(registry.getLogger("cli.main", { argv: ["--help"] }), rootLogger)
  assert.deepEqual(children, [
    { component: "cli.main", fields: { argv: ["--help"] } },
  ])
})

test("formatCliError delegates CLI error presentation", () => {
  assert.equal(formatCliError(new Error("boom")), "boom")
  assert.equal(formatCliError("plain"), "plain")
})

test("CLI startup logging redacts relay credentials and terminal pairing links", () => {
  assert.deepEqual(redactCliStartupArgs([
    "--detached",
    "--relay-url",
    "wss://relay.example.test",
    "--relay-token",
    "secret-relay-token",
    "--target-daemon-id",
    "kernel-1",
    "--terminal-pairing-link",
    "chariox-terminal-pair-v1.named-secret",
    "chariox-terminal-pair-v1.bare-secret",
    "--client-id",
    "client-1",
  ]), [
    "--detached",
    "--relay-url",
    "wss://relay.example.test",
    "--relay-token",
    "[redacted]",
    "--target-daemon-id",
    "kernel-1",
    "--terminal-pairing-link",
    "[redacted]",
    "[redacted-terminal-pairing-link]",
    "--client-id",
    "client-1",
  ])
})

test("CLI startup logging redacts secret options before malformed equals syntax is rejected", () => {
  assert.deepEqual(redactCliStartupArgs([
    "--relay-token=secret-relay-token",
    "--terminal-pairing-link=chariox-terminal-pair-v1.named-secret",
    "--pairing-link=chariox-terminal-pair-v1.alias-secret",
  ]), [
    "--relay-token=[redacted]",
    "--terminal-pairing-link=[redacted]",
    "--pairing-link=[redacted]",
  ])
})
