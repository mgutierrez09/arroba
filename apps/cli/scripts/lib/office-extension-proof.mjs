import assert from "node:assert/strict"
import { createHash } from "node:crypto"

// Input comes from complete, exact-prompt kernel history, never provider prose.
// This proves provenance and ordering, not the script's network implementation.
export function verifyOfficeExtension({ tools, scriptName, environmentName, sourcePath, sourceHash, agentRef, nonce, release }) {
  const latest = new Map()
  for (const tool of tools) {
    assert.ok(typeof tool.callId === "string" && tool.callId, "extension history lacks a tool-call identity")
    latest.set(tool.callId, tool)
  }
  const completed = [...latest.values()].filter(t => t.status === "completed"
    && !t.errorCodes?.length && t.output?.ok !== false && t.output?.isError !== true)
  let index = -1
  const next = (name, check) => {
    const found = completed.findIndex((t, i) => i > index && t.name === name && check(t))
    assert.ok(found >= 0, `missing or out-of-order office extension evidence: ${name}`)
    index = found
    return completed[found]
  }
  next("write_artifact", t => t.input?.path === sourcePath && typeof t.input.content_text === "string"
    && createHash("sha256").update(t.input.content_text).digest("hex") === sourceHash)
  next("register_environment", t => t.input?.config?.name === environmentName
    && t.output?.registered === true && t.output.kind === "environment" && t.output.name === environmentName)
  const registered = next("register_script_path", t => t.input?.path === sourcePath
    && t.input.name === scriptName && t.input.environment === environmentName
    && t.output?.registered === true && t.output.kind === "script" && t.output.name === scriptName
    && t.output.granted === false)
  const granted = next("request_extension", t => t.input?.kind === "script" && t.input.name === scriptName
    && t.input.environment === environmentName && t.output?.granted === true && t.output.kind === "script"
    && t.output.name === scriptName && t.output.agent_ref === agentRef && t.output.effective === "now"
    && t.output.requires_provider_restart === false)
  const invoked = next(scriptName, t => {
    const output = t.output?.result ?? t.output
    return t.input?.nonce === nonce && output?.nonce === nonce
      && Object.entries(release).every(([key, value]) => output[key] === value)
  })
  return { sourceHash, registrationCallId: registered.callId, grantCallId: granted.callId,
    invocationCallId: invoked.callId }
}
