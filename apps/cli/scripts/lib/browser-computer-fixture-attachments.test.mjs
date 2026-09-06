import assert from "node:assert/strict"
import test from "node:test"
import { startBrowserComputerFixture } from "./browser-computer-fixture.mjs"

test("webmail receives the exact Unicode document attached through multipart form data", async () => {
  const fixture = await startBrowserComputerFixture({ password: "fixture-password" })
  try {
    const login = await fetch(`${fixture.origin}/mail/login`, {
      method: "POST", redirect: "manual",
      body: new URLSearchParams({ email: fixture.account, password: "fixture-password" }),
    })
    assert.equal(login.status, 303)
    const headers = { cookie: login.headers.get("set-cookie") }
    const form = new FormData()
    form.set("to", "recipient@chariox.test")
    form.set("subject", "Edited document")
    form.set("body", "Document attached")
    form.append("attachment", new Blob(["Chariox\nGrüße 世界\n"], { type: "text/plain" }), "office.txt")
    const sent = await fetch(`${fixture.origin}/mail/send`, {
      method: "POST", redirect: "manual", headers, body: form,
    })
    assert.equal(sent.status, 303)
    const response = await fetch(`${fixture.origin}/api/messages`, { headers })
    assert.equal(response.status, 200)
    const { messages } = await response.json()
    assert.equal(messages.length, 1)
    assert.equal(messages[0].subject, "Edited document")
    assert.equal(messages[0].to, "recipient@chariox.test")
    assert.equal(messages[0].body, "Document attached")
    assert.equal(messages[0].attachments?.length, 1)
    assert.equal(messages[0].attachments[0].name, "office.txt")
    assert.equal(messages[0].attachments[0].contentType, "text/plain")
    assert.equal(messages[0].attachments[0].sizeBytes, 23)
    assert.equal(messages[0].attachments[0].sha256,
      "6ceda9d0aa573f2f93e9ee1d9b4ee106c5fad19035ad9402bab292ec3c9af52c")
    const compose = await fetch(`${fixture.origin}/mail/compose`, { headers }).then(response => response.text())
    assert.match(compose, /enctype="multipart\/form-data"/)
    assert.match(compose, /type="file" name="attachment"/)
  } finally {
    await fixture.close()
  }
})

test("invalid or oversized mail attachments create no message", async () => {
  const fixture = await startBrowserComputerFixture({ password: "fixture-password" })
  try {
    const login = await fetch(`${fixture.origin}/mail/login`, {
      method: "POST", redirect: "manual",
      body: new URLSearchParams({ email: fixture.account, password: "fixture-password" }),
    })
    const headers = { cookie: login.headers.get("set-cookie") }
    const invalidFile = new FormData()
    invalidFile.set("attachment", "not a file")
    const fileSubject = new FormData()
    fileSubject.set("subject", new Blob(["not a text field"]), "subject.txt")
    const duplicateSubject = new FormData()
    duplicateSubject.append("subject", "first")
    duplicateSubject.append("subject", "second")
    const tooMany = new FormData()
    for (let index = 0; index < 21; index++) tooMany.append("attachment", new Blob([]), `${index}.txt`)
    const oversized = new FormData()
    oversized.append("attachment", new Blob([new Uint8Array(1_048_576)]), "large.bin")
    for (const body of [invalidFile, fileSubject, duplicateSubject, tooMany]) {
      const response = await fetch(`${fixture.origin}/mail/send`, { method: "POST", headers, body, redirect: "manual" })
      assert.equal(response.status, 400)
    }
    const large = await fetch(`${fixture.origin}/mail/send`, { method: "POST", headers, body: oversized, redirect: "manual" })
    assert.equal(large.status, 413)
    const malformed = await fetch(`${fixture.origin}/mail/send`, {
      method: "POST", headers: { ...headers, "content-type": "multipart/form-data; boundary=missing" }, body: "invalid",
    })
    assert.equal(malformed.status, 400)
    const unauthenticated = await fetch(`${fixture.origin}/mail/send`, { method: "POST", body: invalidFile })
    assert.equal(unauthenticated.status, 403)
    const { messages } = await fetch(`${fixture.origin}/api/messages`, { headers }).then(response => response.json())
    assert.deepEqual(messages, [])
  } finally {
    await fixture.close()
  }
})

test("webmail preserves binary and empty attachments and shows escaped names in confirmation", async () => {
  const fixture = await startBrowserComputerFixture({ password: "fixture-password" })
  try {
    const login = await fetch(`${fixture.origin}/mail/login`, {
      method: "POST", redirect: "manual",
      body: new URLSearchParams({ email: fixture.account, password: "fixture-password" }),
    })
    const headers = { cookie: login.headers.get("set-cookie") }
    const form = new FormData()
    form.append("attachment", new Blob([new Uint8Array([0, 255, 13, 10, 65, 128])]), "<report>.bin")
    form.append("attachment", new Blob([]), "empty.txt")
    form.append("attachment", new Blob([]), "")
    const sent = await fetch(`${fixture.origin}/mail/send`, {
      method: "POST", redirect: "manual", headers, body: form,
    })
    assert.equal(sent.status, 303)
    const { messages } = await fetch(`${fixture.origin}/api/messages`, { headers }).then(response => response.json())
    assert.equal(messages[0].attachments.length, 2)
    assert.deepEqual(messages[0].attachments[0], {
      name: "<report>.bin", contentType: "application/octet-stream", sizeBytes: 6,
      sha256: "051f62016e08834b55cfb24630c50ca69d46da1a8fe9a4ee5a53c40b76401002",
    })
    assert.deepEqual(messages[0].attachments[1], {
      name: "empty.txt", contentType: "application/octet-stream", sizeBytes: 0,
      sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    })
    const confirmation = await fetch(new URL(sent.headers.get("location"), fixture.origin), { headers }).then(response => response.text())
    assert.match(confirmation, /&lt;report&gt;\.bin/)
    assert.match(confirmation, /empty\.txt/)
    assert.doesNotMatch(confirmation, /<report>/)
  } finally {
    await fixture.close()
  }
})
