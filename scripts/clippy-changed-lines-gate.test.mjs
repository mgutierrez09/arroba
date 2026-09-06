import assert from "node:assert/strict"
import test from "node:test"

import {
  clippyComparisonBase,
  findWarningRegressions,
  warningFromCompilerEvent,
  warningIdentity,
} from "./clippy-changed-lines-gate.mjs"

test("final-head manual CI compares the complete branch, not just its last commit", () => {
  assert.equal(clippyComparisonBase(undefined, { GITHUB_EVENT_NAME: "workflow_dispatch" }), "origin/main")
  assert.equal(clippyComparisonBase(undefined, { GITHUB_BASE_REF: "release" }), "origin/release")
  assert.equal(clippyComparisonBase("reviewed-sha", { GITHUB_EVENT_NAME: "workflow_dispatch" }), "reviewed-sha")
  assert.equal(clippyComparisonBase(undefined, {}), "HEAD^")
})

function warning(file, line, code, message) {
  return { file, line, column: 1, code, message }
}

test("normalizes a compiler warning without tying identity to its line", () => {
  const diagnostic = warningFromCompilerEvent({
    reason: "compiler-message",
    message: {
      level: "warning",
      code: { code: "dead_code" },
      message: "function `unused` is never used",
      spans: [{ file_name: "src/lib.rs", line_start: 12, column_start: 4, is_primary: true }],
    },
  })

  assert.deepEqual(diagnostic, {
    ...warning("src/lib.rs", 12, "dead_code", "function `unused` is never used"),
    column: 4,
  })
  assert.equal(
    warningIdentity(diagnostic),
    warningIdentity(warning("src/lib.rs", 99, "dead_code", "function `unused` is never used")),
  )
})

test("detects a deletion-induced warning whose span is on an unchanged line", () => {
  const regression = warning("src/lib.rs", 4, "dead_code", "function `now_unused` is never used")
  assert.deepEqual(findWarningRegressions([], [regression]), [regression])
})

test("compares duplicate diagnostics as a multiset", () => {
  const existing = warning("src/lib.rs", 4, "dead_code", "function `unused` is never used")
  const duplicate = warning("src/lib.rs", 40, "dead_code", "function `unused` is never used")

  assert.deepEqual(findWarningRegressions([existing], [existing, duplicate]), [duplicate])
  assert.deepEqual(findWarningRegressions([existing], [duplicate]), [])
})
