import assert from "node:assert/strict"
import { mkdir, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { startBrowserComputerFixture } from "./browser-computer-fixture.mjs"

assert.ok(process.env.PLAYWRIGHT_MODULE, "set PLAYWRIGHT_MODULE to an installed module; no dependency downloads")
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE)

test("Chrome submits UTF-8 and binary attachment bytes through the webmail file control", async () => {
  const evidence = path.join(os.homedir(), ".codex/evidence/browser-computer-use/mail-attachments",
    new Date().toISOString().replaceAll(":", "-"))
  await mkdir(evidence, { recursive: true })
  const fixture = await startBrowserComputerFixture({ password: "fixture-password" })
  let browser
  try {
    browser = await chromium.launch({ channel: "chrome", headless: true, chromiumSandbox: true })
    const page = await browser.newPage()
    await page.goto(`${fixture.origin}/mail/login`)
    await page.getByLabel("Email").fill(fixture.account)
    await page.getByLabel("Password").fill("fixture-password")
    await page.getByRole("button", { name: "Sign in" }).click()
    await page.getByRole("link", { name: "Compose", exact: true }).click()
    await page.getByLabel("To", { exact: true }).fill("recipient@chariox.test")
    await page.getByLabel("Subject", { exact: true }).fill("Edited document")
    await page.getByLabel("Body", { exact: true }).fill("Saved in the editor")
    await page.getByLabel("Attachment", { exact: true }).setInputFiles([
      { name: "office.txt", mimeType: "text/plain", buffer: Buffer.from("Chariox\nGrüße 世界\n") },
      { name: "binary.bin", mimeType: "application/octet-stream", buffer: Buffer.from([0, 255, 13, 10, 65, 128]) },
    ])
    await page.getByRole("button", { name: "Send", exact: true }).click()
    await page.getByText("CHARIOX_FIXTURE_MESSAGE_SENT", { exact: true }).waitFor()
    const result = await page.request.get(`${fixture.origin}/api/messages`)
    assert.equal(result.status(), 200)
    const { messages } = await result.json()
    assert.equal(messages.length, 1)
    assert.equal(messages[0].subject, "Edited document")
    assert.deepEqual(messages[0].attachments.map(file => [file.name, file.sizeBytes, file.sha256]), [
      ["office.txt", 23, "6ceda9d0aa573f2f93e9ee1d9b4ee106c5fad19035ad9402bab292ec3c9af52c"],
      ["binary.bin", 6, "051f62016e08834b55cfb24630c50ca69d46da1a8fe9a4ee5a53c40b76401002"],
    ])
    await page.screenshot({ path: path.join(evidence, "sent-attachments.png") })
    // The default browser form sends an unnamed empty part when no file was
    // selected. Preserve the existing no-attachment mail flow as well.
    await page.goto(`${fixture.origin}/mail/compose`)
    await page.getByLabel("Subject", { exact: true }).fill("No attachment")
    await page.getByRole("button", { name: "Send", exact: true }).click()
    await page.getByText("CHARIOX_FIXTURE_MESSAGE_SENT", { exact: true }).waitFor()
    const after = await page.request.get(`${fixture.origin}/api/messages`).then(response => response.json())
    assert.equal(after.messages.length, 2)
    assert.equal(after.messages[1].attachments, undefined)
    await writeFile(path.join(evidence, "result.json"), JSON.stringify({ passed: true,
      scope: "installed Chrome fixture form; not provider or kernel acceptance",
      attachments: messages[0].attachments, noAttachmentFlow: true }, null, 2), { mode: 0o600 })
    console.log(JSON.stringify({ evidence, passed: true }))
  } finally {
    await browser?.close()
    await fixture.close()
  }
})
