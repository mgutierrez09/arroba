import assert from "node:assert/strict"
import test from "node:test"
import { mkdir, mkdtemp, readFile, readdir, rm, symlink, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { serveWebOnboarding, observeWebOnboarding } from "./web-onboarding-channel.mjs"

const phases = ["mail-login", "registration", "confirmation-email", "confirmation"]
async function fixture(work) {
  const directory = await mkdtemp(path.join(os.tmpdir(), "chariox-onboarding-channel-"))
  try { await work({ directory, evidenceRoot: directory, sessionId: "room", environmentId: "environment", timeoutMs: 1500 }) }
  finally { await rm(directory, { recursive: true, force: true }) }
}

test("owner waits for each Web observation before starting the next onboarding phase", () => fixture(async input => {
  const events = []
  const owner = serveWebOnboarding({ ...input, validateAgent: async id => assert.equal(id, "agent"),
    run: async ({ agentId, observePhase }) => {
      assert.equal(agentId, "agent")
      for (const phase of phases) {
        events.push(`ready:${phase}`)
        const screenshot = path.join(input.evidenceRoot, `onboarding-${phase}.png`)
        await writeFile(screenshot, "physical fixture")
        await observePhase(phase, screenshot)
        events.push(`ack:${phase}`)
      }
      return { fixturesClosed: true }
    } })
  const web = observeWebOnboarding({ ...input, agentId: "agent", observe: async (phase, screenshot) => {
    assert.equal(await readFile(screenshot, "utf8"), "physical fixture")
    events.push(`web:${phase}`)
    return { name: phase, matched: true, width: 1280, height: 800 }
  } })
  const [result, observed] = await Promise.all([owner, web])
  assert.equal(result.onboarding.fixturesClosed, true)
  assert.equal(result.runId, observed.runId)
  assert.deepEqual(observed.phases.map(p => p.name), phases)
  assert.deepEqual(events, phases.flatMap(p => [`ready:${p}`, `web:${p}`, `ack:${p}`]))
  for (const name of (await readdir(input.directory)).filter(n => n.endsWith(".json"))) {
    assert.doesNotMatch(await readFile(path.join(input.directory, name), "utf8"), /password|passphrase|secret|token/i)
  }
}))

test("a mismatched Web frame stops the owner before the next phase", () => fixture(async input => {
  let advanced = false
  const owner = serveWebOnboarding({ ...input, validateAgent: async () => {}, run: async ({ observePhase }) => {
    const screenshot = path.join(input.evidenceRoot, "onboarding-mail-login.png")
    await writeFile(screenshot, "physical fixture")
    await observePhase("mail-login", screenshot)
    advanced = true
  } })
  const web = observeWebOnboarding({ ...input, agentId: "agent", observe: async () => ({ matched: false }) })
  const results = await Promise.allSettled([owner, web])
  assert.ok(results.every(r => r.status === "rejected"))
  assert.equal(advanced, false)
}))

test("missing Web requests and aborted observers terminate within their bound", () => fixture(async input => {
  await assert.rejects(serveWebOnboarding({ ...input, timeoutMs: 30 }), /timed out/)
  const abort = new AbortController(); abort.abort(new Error("cancelled"))
  await assert.rejects(observeWebOnboarding({ ...input, agentId: "agent", signal: abort.signal }), /cancelled/)
}))

test("a failed coordination publish leaves no temporary files", () => fixture(async input => {
  await mkdir(path.join(input.directory, "onboarding-request.json"))
  await assert.rejects(observeWebOnboarding({ ...input, agentId: "agent" }))
  assert.deepEqual(await readdir(input.directory), ["onboarding-request.json"])
}))

const request = { schema: "chariox.onboarding.request.v1", sessionId: "room", environmentId: "environment",
  agentId: "agent", runId: "11111111-1111-4111-8111-111111111111" }

for (const [name, change, expected] of [
  ["another Room", { sessionId: "other" }, /Room mismatch/],
  ["another Environment", { environmentId: "other" }, /Environment mismatch/],
  ["additional credential fields", { password: "never-accepted" }, /unexpected onboarding message fields/],
]) test(`owner rejects ${name} before provider execution`, () => fixture(async input => {
  await writeFile(path.join(input.directory, "onboarding-request.json"), JSON.stringify({ ...request, ...change }))
  let started = false
  await assert.rejects(serveWebOnboarding({ ...input, validateAgent: async () => { started = true },
    run: async () => { started = true } }), expected)
  assert.equal(started, false)
}))

test("an acknowledgement from another run cannot advance the provider", () => fixture(async input => {
  await writeFile(path.join(input.directory, "onboarding-request.json"), JSON.stringify(request))
  await writeFile(path.join(input.directory, "onboarding-ack-1.json"), JSON.stringify({ ...request,
    schema: "chariox.onboarding.ack.v1", runId: "22222222-2222-4222-8222-222222222222",
    phase: "mail-login", sequence: 1, matched: true }))
  let advanced = false
  await assert.rejects(serveWebOnboarding({ ...input, validateAgent: async () => {}, run: async ({ observePhase }) => {
    const screenshot = path.join(input.evidenceRoot, "onboarding-mail-login.png")
    await writeFile(screenshot, "physical fixture")
    await observePhase("mail-login", screenshot)
    advanced = true
  } }), /message identity differs/)
  assert.equal(advanced, false)
}))

test("coordination refuses symbolic links and oversized records", () => fixture(async input => {
  const target = path.join(input.directory, "foreign.json")
  const message = path.join(input.directory, "onboarding-request.json")
  await writeFile(target, JSON.stringify(request))
  await symlink(target, message)
  await assert.rejects(serveWebOnboarding(input), error => error.code === "ELOOP")
  await rm(message)
  await writeFile(message, "x".repeat(16385))
  await assert.rejects(serveWebOnboarding(input), /exceeds size bound/)
}))

test("cancellation interrupts an observer already waiting for the owner", () => fixture(async input => {
  const abort = new AbortController()
  const observer = observeWebOnboarding({ ...input, agentId: "agent", signal: abort.signal,
    onPoll: () => abort.abort(new Error("cancelled while waiting")) })
  await assert.rejects(observer, /cancelled|aborted/)
  assert.deepEqual(await readdir(input.directory), ["onboarding-request.json"])
}))

test("owner refuses an agent rejected by the authoritative validator", () => fixture(async input => {
  await writeFile(path.join(input.directory, "onboarding-request.json"), JSON.stringify(request))
  let started = false
  await assert.rejects(serveWebOnboarding({ ...input,
    validateAgent: async () => { throw new Error("agent is not in the expected slice") },
    run: async () => { started = true } }), /expected slice/)
  assert.equal(started, false)
}))

test("owner cannot omit or reorder required onboarding phases", () => fixture(async input => {
  await writeFile(path.join(input.directory, "onboarding-request.json"), JSON.stringify(request))
  await assert.rejects(serveWebOnboarding({ ...input, validateAgent: async () => {}, run: async () => ({}) }), /omitted required phases/)
  await assert.rejects(serveWebOnboarding({ ...input, validateAgent: async () => {},
    run: ({ observePhase }) => observePhase("registration", path.join(input.evidenceRoot, "onboarding-registration.png")),
  }), /out of order/)
}))

test("only the exact phase screenshot can cross the observation boundary", () => fixture(async input => {
  await writeFile(path.join(input.directory, "onboarding-request.json"), JSON.stringify(request))
  const foreign = path.join(input.directory, "unrelated.png")
  await writeFile(foreign, "not the phase screenshot")
  await assert.rejects(serveWebOnboarding({ ...input, validateAgent: async () => {},
    run: ({ observePhase }) => observePhase("mail-login", foreign),
  }), /screenshot path differs/)
  const screenshot = path.join(input.evidenceRoot, "onboarding-mail-login.png")
  await symlink(foreign, screenshot)
  await assert.rejects(serveWebOnboarding({ ...input, validateAgent: async () => {},
    run: ({ observePhase }) => observePhase("mail-login", screenshot),
  }), /invalid onboarding screenshot/)
  assert.equal((await readdir(input.directory)).some(name => name.startsWith("onboarding-phase-")), false)
}))
