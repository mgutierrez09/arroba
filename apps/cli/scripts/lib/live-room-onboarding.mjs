import assert from "node:assert/strict"
import { randomUUID } from "node:crypto"
import { startBrowserComputerFixture } from "./browser-computer-fixture.mjs"
import { startOfficeOnboardingFixture } from "./office-onboarding-fixture.mjs"
import { createOnboardingInteractionResponder } from "./live-room-onboarding-interactions.mjs"
import { runOnboardingProviderTurn } from "./live-room-onboarding-turn.mjs"
import { readOnboardingTurnTools } from "./live-room-onboarding-history.mjs"
import { verifyOnboardingCredentialActions, verifyOnboardingEmailPath } from "./live-room-onboarding-proof.mjs"
import { readRoomDrillActionHistory } from "./room-drill-action-history.mjs"
import { captureRoomProviderDiagnostic } from "./live-room-provider-diagnostic.mjs"

export async function runRoomOnboarding(input) {
  const { client, requests, sessionId, agentId, onboardingRuntime: runtime } = input
  assert.ok(runtime && typeof runtime.rememberSecret === "function" && typeof runtime.trackCredential === "function"
    && typeof runtime.assertNoLeaks === "function", "onboarding requires the owning drill's private vault and cleanup context")
  assert.ok(typeof input.waitForTuis === "function", "onboarding requires both real TUI observers")
  const nonce = randomUUID()
  const mailId = `office-mail-${nonce}`
  const serviceId = `office-service-${nonce}`
  const credentialTitle = `Office mail credential ${nonce}`
  const organization = `Chariox office ${nonce}`
  runtime.trackCredential(mailId)
  runtime.trackCredential(serviceId)
  runtime.rememberSecret(runtime.mailPassword)
  runtime.rememberSecret(runtime.vaultPassphrase)
  const mail = await startBrowserComputerFixture({ host: "0.0.0.0", password: runtime.mailPassword })
  const mailOrigin = `http://host.docker.internal:${new URL(mail.origin).port}`
  const responder = createOnboardingInteractionResponder({ ...input, credentialTitle,
    mailPassword: runtime.mailPassword, vaultPassphrase: runtime.vaultPassphrase })
  const report = { scenario: "email-gated-onboarding", agentId, provider: input.options.provider,
    model: input.options.model, phases: [], fixturesClosed: false }
  const tools = []
  let service
  const rules = "Use only Chariox runtime MCP tools for all work. Do not use shell, scripts, direct HTTP, provider-native browser tools, or another browser. Never read, print, copy or reveal a password. Use the opaque field_id from slice_browser_find."

  async function phase(name, prompt, expectedText) {
    await input.checkpoint({ phase: `onboarding-${name}`, onboarding: report })
    const turn = await runOnboardingProviderTurn(input, { prompt: `${rules}\n${prompt}`, poll: responder.poll })
    tools.push(...(await readOnboardingTurnTools({ ...input, onToolError: error => {
      if (typeof runtime.redactError !== "function") return
      report.toolErrors ??= []
      if (report.toolErrors.length < 8) report.toolErrors.push(runtime.redactError(error).slice(0, 2048))
    } }, turn.promptId)).map(tool => ({ ...tool, phase: name })))
    const text = await input.officeRuntime.sliceScreen(["browser-text"])
    report.browserMarkers = Object.fromEntries(["CHARIOX_FIXTURE_INBOX", "Fixture mail login", "Invalid credentials",
      "Check your email", "CHARIOX_FIXTURE_ONBOARDING_COMPLETE"].map(marker => [marker, text.includes(marker)]))
    for (const expected of expectedText) assert.ok(text.includes(expected), "onboarding browser did not show its required result")
    const screenshot = await input.screenshot(`onboarding-${name}`)
    report.phases.push({ name, turn, physicalResult: true, screenshot })
    await runtime.assertNoLeaks()
  }

  try {
    service = await startOfficeOnboardingFixture({ mail, host: "0.0.0.0", publicHost: "host.docker.internal",
      onPasswordReceived: runtime.rememberSecret })
    const credentials = [
      { id: mailId, description: "Disposable office mail", allowed_hosts: [new URL(mailOrigin).host],
        allowed_uses: ["browser"], injection: { kind: "browser" } },
      { id: serviceId, description: "Disposable office service", allowed_hosts: [new URL(service.origin).host],
        allowed_uses: ["browser"], injection: { kind: "browser" } },
    ]
    await phase("mail-login", [
      "First call request_credential_secret with exactly this argument:",
      JSON.stringify({ credential: credentials[0], overwrite: false, prompt: { title: credentialTitle,
        message: "Enter the disposable office mail password.", min_length: 12, max_length: 128, timeout_sec: 120 } }),
      "The test user will answer through the private Chariox interaction. Never ask for the value in chat.",
      `Open ${mailOrigin}/mail/login with slice_open_url. Find and fill Email with ${mail.account}.`,
      `Find the Password field and call paste_secret_to_slice with credential_id=${mailId}, expected_host=${new URL(mailOrigin).host}, field_id and submit=false.`,
      "Find Sign in and submit its form once with slice_browser_submit. Inspect the inbox with slice_browser_text, then stop.",
    ].join("\n"), ["CHARIOX_FIXTURE_INBOX", mail.account])
    assert.equal(responder.report().mailCredentialSupplied, true, "mail credential did not go through the private interaction")

    await phase("registration", [
      "Call create_generated_credential with exactly this argument:",
      JSON.stringify({ credential: credentials[1], generator: { kind: "password", length: 32,
        symbols: false, avoid_ambiguous: true }, overwrite: false }),
      `Open ${service.origin}/service/register. Find and fill Email with ${mail.account} and Organization with ${organization}.`,
      `Find the Password field and call paste_secret_to_slice with credential_id=${serviceId}, expected_host=${new URL(service.origin).host}, field_id and submit=false.`,
      "Find Create account and submit its form once with slice_browser_submit. Inspect the check-email page and stop. Do not complete confirmation in this turn.",
    ].join("\n"), ["Check your email"])

    await phase("confirmation-email", [
      `Open ${mailOrigin}/mail/inbox. Find and open the message 'Confirm your Office Service account'.`,
      "Read the message with slice_browser_text. Leave the email open and stop. Do not click its link yet.",
    ].join("\n"), ["Confirm your Office Service account", "Confirm your email to finish onboarding."])

    await phase("confirmation", [
      "The confirmation email is open. Find its Confirm account link and click it through slice_browser_click. Do not guess a confirmation URL or token.",
      "On the service confirmation page, find the Confirm account button and submit its form once with slice_browser_submit.",
      "Inspect the completed account. Use slice_browser_status and slice_browser_tab action=activate with its tab_id to leave Chromium visible on the shared desktop.",
      "Report only the account id, email, organization and the credential handle. Do not return passwords or the confirmation token. Then stop.",
    ].join("\n"), ["CHARIOX_FIXTURE_ONBOARDING_COMPLETE", "service-account-1", mail.account, organization])

    const history = await readRoomDrillActionHistory(async (before, limit) => {
      const response = await client.send(requests.listRoomEnvironmentActionHistoryRequest(sessionId, before, limit))
      return response.RoomEnvironmentActionHistoryListed.page
    })
    const secretActions = verifyOnboardingCredentialActions({ tools, history, actorId: `agent:${agentId}`, credentials })
    const emailActions = verifyOnboardingEmailPath({ tools, history, actorId: `agent:${agentId}` })
    const activationTool = tools.findLast(t => t.name === "slice_browser_tab" && t.status === "completed" && t.input?.action === "activate")
    const activation = history.find(a => a.action_id === activationTool?.output?.action_id)
    assert.ok(activation?.kind === "browser_tab_activate", "onboarding did not activate its completed account tab")
    const submits = tools.filter(t => t.name === "slice_browser_submit" && t.status === "completed")
      .map(t => history.find(a => a.action_id === t.output?.action_id))
    assert.equal(submits.length, 3, "onboarding needs one mail login, registration and confirmation submission")
    assert.ok(submits.every(a => a?.kind === "submit"), "onboarding submission tool lacks its submit action")
    const observed = [secretActions[0], submits[0], secretActions[1], submits[1], ...emailActions, submits[2], activation]
    for (let i = 1; i < observed.length; i++) {
      assert.ok(observed[i]?.sequence > observed[i - 1]?.sequence, "onboarding submission order differs")
    }
    for (let i = 0; i < 2; i++) {
      assert.ok(secretActions[i].targets?.length === 1 && secretActions[i].targets[0].kind === "browser_tab")
      assert.deepEqual(submits[i].targets, secretActions[i].targets, "onboarding submission targeted a different tab")
    }
    for (const action of observed) {
      assert.ok(action && action.actor_id === `agent:${agentId}` && action.mode === "browser" && action.state === "completed",
        "onboarding action was not completed by the official provider")
      await input.waitForTuis(new RegExp(`^Room action #${action.sequence}: real-${input.options.provider} · browser ${action.kind} · completed$`))
    }
    const listed = await client.send(requests.listCredentialsRequest())
    for (const expected of credentials) {
      const actual = listed?.CredentialsListed?.credentials?.find(c => c.id === expected.id)
      assert.ok(actual?.source?.type === "vault", "onboarding credential was not stored in the Chariox vault")
      assert.deepEqual(actual.allowed_hosts, expected.allowed_hosts)
      assert.deepEqual(actual.allowed_uses, ["browser"])
    }
    await runtime.assertNoLeaks()
    report.credentialHandles = credentials.map(c => c.id)
    report.interactions = responder.report()
    report.actions = observed.map(a => ({ id: a.action_id, sequence: a.sequence, kind: a.kind }))
    report.localTuiObserved = true
    report.remoteTuiObserved = true
    report.skipped = ["Web projection", "real Gmail and external SaaS", "other providers", "locked-vault rejection",
      "wrong-origin rejection", "screenshot OCR secret scan", "provider save/resume"]
    return report
  } catch (error) {
    report.interactions = responder.report()
    const known = new Set(["request_credential_secret", "create_generated_credential", "paste_secret_to_slice",
      "slice_open_url", "slice_browser_find", "slice_browser_fill", "slice_browser_submit", "slice_browser_text"])
    report.observedTools = tools.slice(-64).map(tool => ({ name: known.has(tool.name) ? tool.name : "other",
      completed: tool.status === "completed", succeeded: tool.output?.ok === true,
      failed: tool.status === "error" || tool.status === "failed" || tool.output?.ok === false || tool.output?.isError === true,
      errorCodes: tool.errorCodes }))
    report.diagnostic = await captureRoomProviderDiagnostic(input).catch(() => ({ codes: ["diagnostic_unavailable"] }))
    try { await input.screenshot("onboarding-failed") } catch { /* Preserve the original failure. */ }
    try { await input.checkpoint({ phase: "onboarding-failed", onboarding: report }) } catch { /* Cleanup still owns teardown. */ }
    throw error
  } finally {
    const cleanup = await Promise.allSettled([service?.close(), mail.close()])
    report.fixturesClosed = cleanup.every(result => result.status === "fulfilled")
    assert.ok(report.fixturesClosed, "onboarding fixture cleanup failed")
  }
}
