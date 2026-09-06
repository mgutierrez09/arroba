import assert from "node:assert/strict"
import test from "node:test"
import { startBrowserComputerFixture } from "./browser-computer-fixture.mjs"

test("confirmation mail is readable only from the authenticated recipient inbox", async () => {
  const fixture = await startBrowserComputerFixture({ password: "mail-test-password" })
  try {
    fixture.receiveMail({ to: fixture.account, from: "signup@service.chariox.test",
      subject: "Confirm your account", body: "Complete onboarding.",
      link: "http://127.0.0.1:1234/confirm?token=test-token", linkLabel: "Confirm account" })
    const publicMessage = await fetch(`${fixture.origin}/mail/received/incoming-1`, { redirect: "manual" })
    assert.equal(publicMessage.status, 303)
    assert.equal(publicMessage.headers.get("location"), "/mail/login")
    const login = await fetch(`${fixture.origin}/mail/login`, { method: "POST", redirect: "manual",
      body: new URLSearchParams({ email: fixture.account, password: "mail-test-password" }) })
    const headers = { cookie: login.headers.get("set-cookie") }
    const inbox = await fetch(`${fixture.origin}/mail/inbox`, { headers }).then(r => r.text())
    assert.match(inbox, /href="\/mail\/received\/incoming-1"/)
    assert.doesNotMatch(inbox, /test-token/)
    const mail = await fetch(`${fixture.origin}/mail/received/incoming-1`, { headers }).then(r => r.text())
    assert.match(mail, /Complete onboarding\./)
    assert.match(mail, /href="http:\/\/127\.0\.0\.1:1234\/confirm\?token=test-token"/)
    assert.equal(fixture.messages.length, 0, "received mail must not count as an agent submission")
  } finally { await fixture.close() }
})
