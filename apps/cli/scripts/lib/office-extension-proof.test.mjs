import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import test from "node:test"
import { verifyOfficeExtension } from "./office-extension-proof.mjs"

function evidence() {
  const source = 'def run(nonce: str) -> dict:\n    return {}\n'
  const release = { id: 1234, tag_name: "jq-1.8.1", html_url: "https://github.com/jqlang/jq/releases/tag/jq-1.8.1",
    published_at: "2025-07-01T12:00:00Z", draft: false, prerelease: false }
  const call = (name, input, output) => ({ name, input, output, status: "completed", callId: name })
  return { scriptName: "office_release", environmentName: "office_python", sourcePath: "release.py",
    sourceHash: createHash("sha256").update(source).digest("hex"), agentRef: "agent-2", nonce: "unique-run", release,
    tools: [
      call("write_artifact", { path: "release.py", content_text: source }, {}),
      call("register_environment", { config: { name: "office_python" } }, { registered: true, kind: "environment", name: "office_python" }),
      call("register_script_path", { path: "release.py", name: "office_release", environment: "office_python" },
        { registered: true, kind: "script", name: "office_release", granted: false }),
      call("request_extension", { kind: "script", name: "office_release", environment: "office_python" },
        { granted: true, kind: "script", name: "office_release", agent_ref: "agent-2", effective: "now", requires_provider_restart: false }),
      call("office_release", { nonce: "unique-run" }, { ...release, nonce: "unique-run" }),
    ] }
}

test("extension evidence links authored bytes, registration, own grant and current invocation in order", () => {
  assert.deepEqual(verifyOfficeExtension(evidence()), { sourceHash: evidence().sourceHash,
    registrationCallId: "register_script_path", grantCallId: "request_extension", invocationCallId: "office_release" })
})

test("a correct API result cannot hide a missing grant, wrong actor, stale invocation or different source", () => {
  for (const mutate of [
    e => e.tools.splice(3, 1), e => { e.tools[3].output.agent_ref = "agent-other" },
    e => { e.tools[4].input.nonce = "previous-run" }, e => { e.tools[4].output.tag_name = "wrong" },
    e => { e.tools[0].input.content_text += "# changed" },
    e => { e.tools[3].output.requires_provider_restart = true },
    e => { e.tools[3].status = "failed" }, e => { e.tools.reverse() },
  ]) {
    const input = evidence(); mutate(input)
    assert.throws(() => verifyOfficeExtension(input))
  }
})
