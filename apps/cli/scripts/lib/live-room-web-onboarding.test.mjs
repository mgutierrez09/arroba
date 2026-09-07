import assert from "node:assert/strict"
import test from "node:test"
import { mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { runRoomWebOnboarding } from "./live-room-web-onboarding.mjs"
import { runRoomEnvironmentCompanion } from "./live-room-environment-companion-verifier.mjs"

async function fixture(work, { agent = {}, slices = [{ id: "slice", agent_ids: ["agent"] }] } = {}) {
  const directory = await mkdtemp(path.join(os.tmpdir(), "chariox-web-onboarding-owner-"))
  const calls = []
  const credentials = []
  let report
  let mailOrigin
  const requests = Object.fromEntries(["getSessionState", "listSlices", "attachToSession", "submitPrompt", "detachFromSession"]
    .map(name => [`${name}Request`, (...args) => ({ name, args })]))
  const input = { directory, evidenceRoot: directory, sessionId: "room", environmentId: "environment", sliceId: "slice",
    timeoutMs: 100, options: { provider: "codex", model: "fixture", accountProfile: "default", officeScenario: "onboarding" },
    requests, withTimeout: promise => promise, sampleTuis: async () => {},
    waitForTuis: async () => { throw new Error("provider did not execute") },
    checkpoint: async value => { report = value.onboarding },
    onboardingRuntime: { mailPassword: "private-mail-password", vaultPassphrase: "private-vault-passphrase",
      rememberSecret: () => {}, trackCredential: id => credentials.push(id), assertNoLeaks: async () => {} },
    client: { send: async ({ name, args }) => {
      calls.push(name)
      if (name === "getSessionState") return { SessionState: { session: { agents: [{ id: "agent", session_id: "room",
        provider: "codex", model: "fixture", account_profile: "default", is_processing: false, ...agent }] } } }
      if (name === "listSlices") return { SlicesListed: { slices } }
      if (name === "attachToSession") return { SessionAttached: { attachment: { id: "attachment" } } }
      if (name === "detachFromSession") return { SessionDetached: {} }
      assert.equal(name, "submitPrompt")
      assert.deepEqual(args.slice(0, 3), ["room", "attachment", "agent"])
      assert.match(args[3], /request_credential_secret/)
      assert.doesNotMatch(args[3], /private-mail-password|private-vault-passphrase/)
      const port = args[3].match(/http:\/\/host\.docker\.internal:(\d+)\/mail\/login/)?.[1]
      assert.ok(port)
      mailOrigin = `http://127.0.0.1:${port}`
      assert.equal((await fetch(`${mailOrigin}/mail/login`)).status, 200)
      throw new Error("controlled provider submission failure")
    } },
  }
  try {
    await writeFile(path.join(directory, "onboarding-request.json"), JSON.stringify({ schema: "chariox.onboarding.request.v1",
      sessionId: "room", environmentId: "environment", agentId: "agent", runId: "11111111-1111-4111-8111-111111111111" }))
    await work({ input, calls, credentials, report: () => report, mailOrigin: () => mailOrigin })
  } finally { await rm(directory, { recursive: true, force: true }) }
}

test("Web onboarding keeps private replies in the owner and uses the existing provider prompt path", () => fixture(async ({ input, calls, credentials, report, mailOrigin }) => {
  await assert.rejects(runRoomWebOnboarding(input), /controlled provider submission failure/)
  assert.deepEqual(calls.slice(0, 2), ["getSessionState", "listSlices"])
  assert.deepEqual(calls.filter(name => ["attachToSession", "submitPrompt", "detachFromSession"].includes(name)),
    ["attachToSession", "submitPrompt", "detachFromSession"])
  assert.equal(credentials.length, 2)
  assert.equal(report().fixturesClosed, true)
  await assert.rejects(fetch(`${mailOrigin()}/mail/login`))
  for (const name of await readdir(input.directory)) {
    assert.doesNotMatch(await readFile(path.join(input.directory, name), "utf8"), /private-mail-password|private-vault-passphrase/)
  }
}))

for (const [field, value] of [["id", "other"], ["session_id", "other"], ["provider", "dev-stub"],
  ["model", "other"], ["account_profile", "other"], ["is_processing", true]]) {
  test(`Web cannot start onboarding when kernel ${field} differs`, () => fixture(async ({ input, calls, credentials }) => {
    await assert.rejects(runRoomWebOnboarding(input), /authoritative|idle/)
    assert.equal(calls.includes("submitPrompt"), false)
    assert.equal(credentials.length, 0)
  }, { agent: { [field]: value } }))
}

test("Web cannot choose an agent from another slice", () => fixture(async ({ input, calls }) => {
  await assert.rejects(runRoomWebOnboarding(input), /intended slice/)
  assert.equal(calls.includes("submitPrompt"), false)
}, { slices: [{ id: "another-slice", agent_ids: ["agent"] }] }))

test("the live companion routes onboarding to the private owner without publishing its context", () => fixture(async ({ input, report }) => {
  const ready = { sessionId: input.sessionId, environmentId: input.environmentId,
    sliceId: input.sliceId, evidenceRoot: input.evidenceRoot, realProvider: input.options }
  await assert.rejects(runRoomEnvironmentCompanion({
    env: { CHARIOX_ROOM_DRILL_COORDINATION_DIR: input.directory, CHARIOX_ROOM_DRILL_COMPANION_TIMEOUT_MS: "1000" },
    ready, onboardingInput: input, readTuiNotices: async () => ({ local: [], remote: [] }),
  }), /controlled provider submission failure/)
  assert.equal(report().fixturesClosed, true)
  const handoff = await readFile(path.join(input.directory, "ready.json"), "utf8")
  assert.doesNotMatch(handoff, /private-mail-password|private-vault-passphrase|onboardingRuntime/)
}))
