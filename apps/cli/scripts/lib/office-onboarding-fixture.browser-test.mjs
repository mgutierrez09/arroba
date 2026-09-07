import assert from "node:assert/strict"
import { mkdir, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { startBrowserComputerFixture } from "./browser-computer-fixture.mjs"
import { startOfficeOnboardingFixture } from "./office-onboarding-fixture.mjs"

test("Chrome completes registration only through the authenticated confirmation email", { timeout: 60000 }, async () => {
  assert.ok(process.env.PLAYWRIGHT_MODULE, "set PLAYWRIGHT_MODULE; do not download dependencies")
  const { chromium } = await import(process.env.PLAYWRIGHT_MODULE)
  const evidence = path.join(os.homedir(), ".codex/evidence/browser-computer-use/office-onboarding",
    new Date().toISOString().replaceAll(":", "-"))
  await mkdir(evidence, { recursive: true })
  const mail = await startBrowserComputerFixture({ password: "synthetic-mail-password" })
  let service
  let browser
  const errors = []
  try {
    service = await startOfficeOnboardingFixture({ mail })
    browser = await chromium.launch({ channel: "chrome", headless: true, chromiumSandbox: true })
    const context = await browser.newContext({ viewport: { width: 1000, height: 700 } })
    const page = await context.newPage()
    page.on("pageerror", error => errors.push(error.message))
    await page.goto(`${service.origin}/service/register`)
    await page.getByLabel("Email", { exact: true }).fill(mail.account)
    await page.getByLabel("Organization", { exact: true }).fill("Chariox office")
    await page.getByLabel("Password", { exact: true }).fill("synthetic-generated-password")
    await page.getByRole("button", { name: "Create account", exact: true }).click()
    await page.getByRole("heading", { name: "Check your email", exact: true }).waitFor()
    await page.screenshot({ path: path.join(evidence, "check-email.png") })

    const mailbox = await context.newPage()
    mailbox.on("pageerror", error => errors.push(error.message))
    await mailbox.goto(`${mail.origin}/mail/login`)
    await mailbox.getByLabel("Email", { exact: true }).fill(mail.account)
    await mailbox.getByLabel("Password", { exact: true }).fill("synthetic-mail-password")
    await mailbox.getByRole("button", { name: "Sign in", exact: true }).click()
    await mailbox.getByRole("link", { name: "Confirm your Office Service account", exact: true }).click()
    await mailbox.getByRole("heading", { name: "Confirm your Office Service account", exact: true }).waitFor()
    await mailbox.screenshot({ path: path.join(evidence, "confirmation-email.png") })
    await mailbox.getByRole("link", { name: "Confirm account", exact: true }).click()
    await mailbox.getByRole("button", { name: "Confirm account", exact: true }).click()
    await mailbox.getByText("CHARIOX_FIXTURE_ONBOARDING_COMPLETE", { exact: true }).waitFor()
    assert.match(await mailbox.locator("body").innerText(), /Account: service-account-1/)
    assert.match(await mailbox.locator("body").innerText(), /Organization: Chariox office/)
    assert.doesNotMatch(await mailbox.locator("body").innerText(), /synthetic-.*password/)
    await mailbox.screenshot({ path: path.join(evidence, "onboarding-complete.png") })
    assert.deepEqual(errors, [])
    await writeFile(path.join(evidence, "result.json"), JSON.stringify({ passed: true,
      scope: "installed Chrome fixture navigation only; no provider, vault, kernel or TUI acceptance",
      browserVersion: browser.version(), mail: "controlled fixture, not Gmail",
      registration: true, confirmationEmail: true, onboardingComplete: true }, null, 2), { mode: 0o600 })
    console.log(JSON.stringify({ evidence, passed: true }))
  } finally {
    await browser?.close()
    await service?.close()
    await mail.close()
  }
})
