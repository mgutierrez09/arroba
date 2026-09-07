import assert from "node:assert/strict"
import test from "node:test"
import { startBrowserComputerFixture } from "./browser-computer-fixture.mjs"
import { startOfficeOnboardingFixture } from "./office-onboarding-fixture.mjs"

const mailPassword = "private-mail-fixture-password"
const servicePassword = "private-generated-service-password"
const post = (url, fields, headers = {}) => fetch(url, { method: "POST", redirect: "manual",
  headers, body: new URLSearchParams(fields) })

async function setup(t, options = {}) {
  const mail = await startBrowserComputerFixture({ password: mailPassword })
  t.after(() => mail.close())
  const service = await startOfficeOnboardingFixture({ mail, ...options })
  t.after(() => service.close())
  return { mail, service, registration: { email: mail.account, organization: "Chariox office",
    password: servicePassword } }
}

async function confirmationFromInbox(mail) {
  const login = await post(`${mail.origin}/mail/login`, { email: mail.account, password: mailPassword })
  const headers = { cookie: login.headers.get("set-cookie") }
  const inbox = await fetch(`${mail.origin}/mail/inbox`, { headers }).then(r => r.text())
  const messagePath = inbox.match(/href="(\/mail\/received\/[^\"]+)"/)?.[1]
  assert.ok(messagePath, "service must deliver a confirmation email")
  const message = await fetch(`${mail.origin}${messagePath}`, { headers }).then(r => r.text())
  assert.ok(!message.includes(servicePassword) && !message.includes(mailPassword))
  const url = message.match(/href="([^"]+\/service\/confirm\?token=[^"]+)"/)?.[1]
  assert.ok(url, "confirmation must be discoverable from message content")
  return url
}

test("service onboarding requires authenticated mail access and confirmation before account access", async t => {
  const { mail, service, registration } = await setup(t)
  assert.equal((await post(`${service.origin}/service/register`, registration)).status, 303)
  assert.equal((await post(`${service.origin}/service/login`, registration)).status, 403)
  assert.equal((await fetch(`${service.origin}/api/account`)).status, 401)
  const confirmation = await confirmationFromInbox(mail)
  const preview = await fetch(confirmation).then(r => r.text())
  assert.match(preview, /Confirm account/)
  assert.equal((await post(`${service.origin}/service/login`, registration)).status, 403,
    "reading a confirmation link must not activate the account")
  const activated = await post(confirmation, {})
  assert.equal(activated.status, 303)
  assert.equal(activated.headers.get("location"), "/service/dashboard")
  const headers = { cookie: activated.headers.get("set-cookie") }
  const result = await fetch(`${service.origin}/api/account`, { headers }).then(r => r.json())
  assert.deepEqual(result, { id: "service-account-1", email: "agent@chariox.test",
    organization: "Chariox office", status: "active" })
  assert.equal(mail.messages.length, 0)
  assert.equal((await post(`${service.origin}/service/login`, { ...registration, password: "wrong-password" })).status, 401)
  assert.equal((await post(`${service.origin}/service/login`, registration)).status, 303)
})

test("confirmation links expire without activating the account", async t => {
  let now = 0
  const { mail, service, registration } = await setup(t, { now: () => now })
  await post(`${service.origin}/service/register`, registration)
  const confirmation = await confirmationFromInbox(mail)
  now = 5 * 60 * 1000
  assert.equal((await post(confirmation, {})).status, 410)
  assert.equal((await post(`${service.origin}/service/login`, registration)).status, 403)
})

test("confirmation is one-use and cannot be bypassed with a guessed token or repeated registration", async t => {
  const { mail, service, registration } = await setup(t)
  const registrationUrl = `${service.origin}/service/register`
  const responses = await Promise.all([post(registrationUrl, registration), post(registrationUrl, registration)])
  assert.deepEqual(responses.map(r => r.status).sort(), [303, 409])
  assert.equal((await post(`${service.origin}/service/confirm?token=guessed`, {})).status, 400)
  const confirmation = await confirmationFromInbox(mail)
  assert.equal((await post(confirmation, {})).status, 303)
  const replay = await post(confirmation, {})
  assert.equal(replay.status, 400)
  assert.equal(replay.headers.get("set-cookie"), null)
  assert.equal((await fetch(confirmation)).status, 400)
})

test("onboarding rejects undeliverable and oversized input without reflecting passwords", async t => {
  const { service, registration } = await setup(t)
  for (const fields of [
    { ...registration, email: "someone-else@chariox.test" },
    { ...registration, password: "short" },
    { ...registration, organization: " " },
  ]) {
    const result = await post(`${service.origin}/service/register`, fields)
    assert.equal(result.status, 400)
    assert.ok(!(await result.text()).includes(fields.password))
  }
  const oversized = await post(`${service.origin}/service/register`, { ...registration, password: "x".repeat(9000) })
  assert.equal(oversized.status, 413)
  assert.equal((await post(`${service.origin}/service/register`, registration)).status, 303)
})

test("onboarding escapes account metadata and closes its listener", async t => {
  const { mail, service, registration } = await setup(t)
  await post(`${service.origin}/service/register`, { ...registration, organization: '<script>alert("x")</script>' })
  const confirmed = await post(await confirmationFromInbox(mail), {})
  const html = await fetch(`${service.origin}/service/dashboard`, {
    headers: { cookie: confirmed.headers.get("set-cookie") },
  }).then(r => r.text())
  assert.match(html, /&lt;script&gt;/)
  assert.doesNotMatch(html, /<script>/)
  assert.ok(!html.includes(servicePassword) && !html.includes(mailPassword))
  await service.close()
  await assert.rejects(fetch(`${service.origin}/service/login`))
})
