import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const workflow = await readFile(new URL("../.github/workflows/ci.yml", import.meta.url), "utf8")

test("CI exposes exactly one manual dispatch trigger", () => {
  const declarations = workflow.match(/^  workflow_dispatch:\s*$/gm) ?? []
  assert.equal(declarations.length, 1)
})
