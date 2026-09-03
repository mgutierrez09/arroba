import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

test("CI workflow declares every trigger once", async () => {
  const workflow = await readFile(new URL("../.github/workflows/ci.yml", import.meta.url), "utf8")
  const eventBlock = workflow.match(/^on:\n(?<events>[\s\S]*?)^jobs:/m)?.groups?.events
  assert.ok(eventBlock, "CI workflow must contain an on block before jobs")

  const events = [...eventBlock.matchAll(/^ {2}([a-z_]+):/gm)].map((match) => match[1])
  assert.deepEqual(events, ["workflow_dispatch", "push", "pull_request"])
})
