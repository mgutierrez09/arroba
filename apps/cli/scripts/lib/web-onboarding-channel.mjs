import assert from "node:assert/strict"
import { randomUUID } from "node:crypto"
import { constants } from "node:fs"
import { open, realpath, rename, lstat, unlink, writeFile } from "node:fs/promises"
import path from "node:path"
import { setTimeout as sleep } from "node:timers/promises"

const phases = ["mail-login", "registration", "confirmation-email", "confirmation"]
const fields = ["schema", "sessionId", "environmentId", "agentId", "runId"]
const schema = kind => `chariox.onboarding.${kind}.v1`

// Disposable drill coordination only. The owner retains credentials and
// provider execution; the observer acknowledges a physically verified phase.
export async function serveWebOnboarding(input) {
  const request = await waitMessage(input, "request", value => {
    validate(value, "request")
    assert.equal(value.sessionId, input.sessionId, "onboarding Room mismatch")
    assert.equal(value.environmentId, input.environmentId, "onboarding Environment mismatch")
  })
  await input.validateAgent(request.agentId)
  let index = 0
  const onboarding = await input.run({ agentId: request.agentId, observePhase: async (phase, screenshot) => {
    assert.equal(phase, phases[index], "onboarding phase is out of order")
    assert.equal(path.resolve(screenshot), screenshotPath(input, phase), "onboarding screenshot path differs")
    await verifyScreenshotPath(input, phase)
    const envelope = { ...request, schema: schema("phase"), phase, sequence: index + 1 }
    await send(input, `phase-${index + 1}`, envelope)
    await waitMessage(input, `ack-${index + 1}`, value => {
      validate(value, "ack")
      matchIdentity(value, request)
      assert.equal(value.phase, phase, "onboarding acknowledgement phase differs")
      assert.equal(value.sequence, index + 1, "onboarding acknowledgement sequence differs")
      assert.equal(value.matched, true, "Web did not verify the onboarding phase")
    })
    index++
  } })
  assert.equal(index, phases.length, "onboarding omitted required phases")
  return { runId: request.runId, agentId: request.agentId, onboarding }
}

export async function observeWebOnboarding(input) {
  const request = { schema: schema("request"), sessionId: input.sessionId,
    environmentId: input.environmentId, agentId: input.agentId, runId: randomUUID() }
  validate(request, "request")
  await send(input, "request", request)
  const observed = []
  for (const [index, phase] of phases.entries()) {
    await waitMessage(input, `phase-${index + 1}`, value => {
      validate(value, "phase")
      matchIdentity(value, request)
      assert.equal(value.phase, phase, "onboarding phase is out of order")
      assert.equal(value.sequence, index + 1, "onboarding phase sequence differs")
    })
    let result
    try {
      await verifyScreenshotPath(input, phase)
      result = await input.observe(phase, screenshotPath(input, phase))
      assert.equal(result?.matched, true, "Web onboarding display did not match")
      input.signal?.throwIfAborted()
    } catch (error) {
      await send(input, `ack-${index + 1}`, { ...request, schema: schema("ack"), phase,
        sequence: index + 1, matched: false }).catch(() => {})
      throw error
    }
    await send(input, `ack-${index + 1}`, { ...request, schema: schema("ack"), phase,
      sequence: index + 1, matched: true })
    observed.push({ ...result, name: phase })
  }
  return { runId: request.runId, agentId: request.agentId, phases: observed }
}

function validate(value, kind) {
  assert.ok(value && typeof value === "object" && !Array.isArray(value), "invalid onboarding message")
  const allowed = [...fields, ...(kind === "request" ? [] : ["phase", "sequence"]), ...(kind === "ack" ? ["matched"] : [])]
  assert.deepEqual(Object.keys(value).sort(), allowed.sort(), "unexpected onboarding message fields")
  assert.equal(value.schema, schema(kind), "onboarding message schema differs")
  for (const key of fields.slice(1)) assert.ok(typeof value[key] === "string" && value[key].length > 0
    && value[key].length <= 128, "invalid onboarding identity")
  assert.match(value.runId, /^[a-f0-9-]{36}$/)
  if (kind !== "request") {
    assert.ok(phases.includes(value.phase) && value.sequence === phases.indexOf(value.phase) + 1, "invalid onboarding phase")
  }
  if (kind === "ack") assert.equal(typeof value.matched, "boolean")
}

function matchIdentity(value, expected) {
  for (const key of fields.slice(1)) assert.equal(value[key], expected[key], "onboarding message identity differs")
}

function messagePath(input, name) {
  assert.ok(path.isAbsolute(input.directory), "onboarding coordination directory must be absolute")
  return path.join(input.directory, `onboarding-${name}.json`)
}

function screenshotPath(input, phase) {
  assert.ok(path.isAbsolute(input.evidenceRoot), "onboarding evidence root must be absolute")
  assert.ok(phases.includes(phase), "unknown onboarding screenshot phase")
  return path.join(input.evidenceRoot, `onboarding-${phase}.png`)
}

async function verifyScreenshotPath(input, phase) {
  const file = screenshotPath(input, phase)
  const stat = await lstat(file)
  assert.ok(stat.isFile() && !stat.isSymbolicLink() && stat.size <= 8 * 1024 * 1024, "invalid onboarding screenshot")
  assert.equal(path.dirname(await realpath(file)), await realpath(input.evidenceRoot), "onboarding screenshot escaped evidence root")
}

async function send(input, name, value) {
  input.signal?.throwIfAborted()
  const target = messagePath(input, name)
  const temporary = `${target}.${randomUUID()}.tmp`
  try {
    await writeFile(temporary, JSON.stringify(value), { mode: 0o600, flag: "wx" })
    await rename(temporary, target)
  } finally {
    await unlink(temporary).catch(error => { if (error.code !== "ENOENT") throw error })
  }
}

async function waitMessage(input, name, check) {
  const timeout = input.timeoutMs ?? 330000
  assert.ok(Number.isSafeInteger(timeout) && timeout > 0 && timeout <= 1200000, "invalid onboarding timeout")
  const deadline = Date.now() + timeout
  while (Date.now() < deadline) {
    input.signal?.throwIfAborted()
    await input.onPoll?.()
    let handle
    try {
      handle = await open(messagePath(input, name), constants.O_RDONLY | constants.O_NOFOLLOW)
      assert.ok((await handle.stat()).isFile(), "onboarding message must be a regular file")
      const buffer = Buffer.alloc(16385)
      const { bytesRead } = await handle.read(buffer, 0, buffer.length, 0)
      assert.ok(bytesRead <= 16384, "onboarding message exceeds size bound")
      const value = JSON.parse(buffer.subarray(0, bytesRead).toString("utf8"))
      check(value)
      return value
    } catch (error) { if (error.code !== "ENOENT") throw error }
    finally { await handle?.close() }
    await (input.sleep ? input.sleep(25) : sleep(25, undefined, { signal: input.signal }))
  }
  throw new Error("onboarding coordination timed out")
}
