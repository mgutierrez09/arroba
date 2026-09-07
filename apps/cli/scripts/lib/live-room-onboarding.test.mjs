import assert from "node:assert/strict"
import test from "node:test"
import { runRoomOnboarding } from "./live-room-onboarding.mjs"

test("onboarding uses the provider prompt path and closes its mail fixture if submission fails", async () => {
  const failure = new Error("provider submission failed")
  const replies = []
  const credentials = []
  const secrets = []
  let mailOrigin
  let report
  const requests = Object.fromEntries(["attachToSession", "submitPrompt", "detachFromSession"]
    .map(name => [`${name}Request`, (...args) => ({ name, args })]))
  await assert.rejects(runRoomOnboarding({ requests, sessionId: "room", agentId: "agent",
    options: { provider: "codex", model: "fixture" },
    client: { send: async ({ name, args }) => {
      replies.push(name)
      if (name === "attachToSession") return { SessionAttached: { attachment: { id: "attachment" } } }
      if (name === "detachFromSession") return { SessionDetached: {} }
      assert.equal(name, "submitPrompt")
      const prompt = args[3]
      assert.ok(prompt.includes("request_credential_secret") && prompt.includes("paste_secret_to_slice"))
      assert.ok(!prompt.includes("private-mail-password") && !prompt.includes("private-vault-passphrase"))
      const port = prompt.match(/http:\/\/host\.docker\.internal:(\d+)\/mail\/login/)?.[1]
      assert.ok(port)
      mailOrigin = `http://127.0.0.1:${port}`
      // The login page exists, but the driver must not have established a session.
      const response = await fetch(`${mailOrigin}/mail/inbox`, { redirect: "manual" })
      assert.equal(response.headers.get("location"), "/mail/login")
      throw failure
    } },
    checkpoint: async value => { report = value.onboarding },
    waitForTuis: async () => { throw new Error("no provider action occurred") },
    officeRuntime: { sliceScreen: async () => { throw new Error("driver must not perform provider work") } },
    onboardingRuntime: { mailPassword: "private-mail-password", vaultPassphrase: "private-vault-passphrase",
      rememberSecret: secret => secrets.push(secret), trackCredential: id => credentials.push(id), assertNoLeaks: async () => {} },
  }), error => error === failure)
  assert.deepEqual(replies, ["attachToSession", "submitPrompt", "detachFromSession"])
  assert.equal(credentials.length, 2, "both generated handles must be registered with the cleanup owner")
  assert.deepEqual(secrets, ["private-mail-password", "private-vault-passphrase"])
  assert.equal(report.fixturesClosed, true)
  assert.equal(report.localTuiObserved, undefined)
  await assert.rejects(fetch(`${mailOrigin}/mail/login`))
})

test("onboarding cannot start without private leak-scan and cleanup ownership", async () => {
  await assert.rejects(runRoomOnboarding({}), /private vault and cleanup context/)
})
