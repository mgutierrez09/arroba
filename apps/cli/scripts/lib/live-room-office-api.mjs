import assert from "node:assert/strict"
import { createHash, randomUUID } from "node:crypto"
import { lstat, readFile, writeFile } from "node:fs/promises"
import path from "node:path"
import { startOfficeInventoryFixture } from "./office-inventory-fixture.mjs"
import { fetchOfficeRelease } from "./office-release-api.mjs"
import { verifyOfficeExtension } from "./office-extension-proof.mjs"
import { runOnboardingProviderTurn } from "./live-room-onboarding-turn.mjs"
import { readOnboardingTurnTools } from "./live-room-onboarding-history.mjs"
import { readRoomDrillActionHistory } from "./room-drill-action-history.mjs"
import { captureRoomProviderDiagnostic } from "./live-room-provider-diagnostic.mjs"

export async function runRoomOfficeApi(input) {
  const { client, requests, sessionId, agentId } = input
  const nonce = randomUUID()
  const scriptName = `office_release_${nonce.replaceAll("-", "")}`
  const environmentName = `office_python_${nonce.replaceAll("-", "")}`
  const sourcePath = `${scriptName}.py`
  const sourceFile = path.join(input.workspace, sourcePath)
  await assert.rejects(lstat(sourceFile), { code: "ENOENT" }, "office script must be newly authored")
  const release = await fetchOfficeRelease()
  const service = await startOfficeInventoryFixture({ expectedRelease: release, host: "0.0.0.0", publicHost: "host.docker.internal" })
  const report = { scenario: "public-api-extension", agentId, provider: input.options.provider, release,
    scriptName, environmentName, fixtureClosed: false }
  const agentState = async () => {
    const state = await client.send(requests.getSessionStateRequest(sessionId))
    const agent = state?.SessionState?.session?.agents?.find(a => a.id === agentId)
    assert.ok(agent && agent.is_processing === false, "office API requires an idle official agent")
    return agent
  }
  try {
    const before = await agentState()
    const resumeKey = { codex: "codex_thread_id", claude: "claude_session_id", opencode: "opencode_session_id" }[input.options.provider]
    const providerSession = before.provider_resume_state?.[resumeKey]
    assert.ok(typeof providerSession === "string" && providerSession, "office API provider session identity missing")
    await input.checkpoint({ phase: "office-api-prompting", officeApi: report })
    const turn = await runOnboardingProviderTurn(input, { poll: async () => {}, prompt: [
      `Complete the software inventory task at ${service.origin}/inventory using Chariox Browser tools.`,
      "Read that page and its linked official API documentation to discover the public release endpoint. Do not guess the release.",
      "Use only Chariox runtime MCP tools. Do not use shell, provider-native file/browser tools, direct HTTP tools, or another agent.",
      `Author ${sourcePath} yourself with chariox.write_artifact, content_text, using Python standard library only.`,
      "Implement run(nonce: str) -> dict with a docstring. It must GET the public API using urllib.request with a User-Agent, Accept application/vnd.github+json and X-GitHub-Api-Version 2026-03-10, no credentials, a 15-second timeout and a 1 MiB response bound. Return id, tag_name, html_url, published_at, draft, prerelease and the supplied nonce. No printing or other side effects. Define test_run() with local deterministic checks only, never another network call. No network access at module import.",
      `After writing the file, call register_environment with ${JSON.stringify({ config: { name: environmentName, runtime: { type: "python", python: "/usr/bin/python3" } } })}.`,
      `Call register_script_path with ${JSON.stringify({ path: sourcePath, environment: environmentName, name: scriptName, grant_to_current_agent: false })}.`,
      `Then call request_extension with ${JSON.stringify({ kind: "script", name: scriptName, environment: environmentName })}.`,
      `Invoke the new ${scriptName} tool with ${JSON.stringify({ nonce })} in this same provider session. Do not restart or use a different session.`,
      "Use the returned tag, release URL and publication timestamp to fill and submit the inventory form through Browser tools. Submit exactly once. Activate the completed inventory browser tab and leave the receipt visible. Report completion without reproducing source code.",
    ].join("\n") })
    const after = await agentState()
    assert.equal(after.provider_resume_state?.[resumeKey], providerSession, "office extension changed provider sessions")
    assert.ok(path.isAbsolute(input.evidenceRoot), "office source evidence requires an explicit external root")
    const tools = await readOnboardingTurnTools(input, turn.promptId)
    report.toolHistoryArtifact = path.join(input.evidenceRoot, "office-api-tool-history.json")
    await writeFile(report.toolHistoryArtifact, JSON.stringify(tools, null, 2), { mode: 0o600, flag: "wx" })
    await input.onboardingRuntime.assertNoLeaks()
    const info = await lstat(sourceFile)
    assert.ok(info.isFile() && !info.isSymbolicLink() && info.size > 0 && info.size <= 65536, "invalid authored office script")
    const source = await readFile(sourceFile)
    report.sourceArtifact = path.join(input.evidenceRoot, sourcePath)
    await writeFile(report.sourceArtifact, source, { mode: 0o600, flag: "wx" })
    await input.onboardingRuntime.assertNoLeaks()
    report.extension = verifyOfficeExtension({ tools, scriptName, environmentName, sourcePath,
      sourceHash: createHash("sha256").update(source).digest("hex"), agentRef: before.agent_ref, nonce, release })
    const receiptResponse = await fetch(`http://127.0.0.1:${new URL(service.origin).port}/api/inventory`, { signal: AbortSignal.timeout(5000) })
    assert.equal(receiptResponse.status, 200)
    assert.deepEqual(await receiptResponse.json(), { status: "updated", repository: "jqlang/jq",
      version: release.tag_name, release_url: release.html_url, published_at: release.published_at })
    const history = await readRoomDrillActionHistory(async (before, limit) => {
      const response = await client.send(requests.listRoomEnvironmentActionHistoryRequest(sessionId, before, limit))
      return response.RoomEnvironmentActionHistoryListed.page
    })
    const completed = [...new Map(tools.map(t => [t.callId, t])).values()].filter(t => t.status === "completed")
    const invocation = completed.findIndex(t => t.callId === report.extension.invocationCallId)
    const actionTools = completed.slice(invocation + 1).filter(t => ["slice_browser_fill", "slice_browser_submit", "slice_browser_tab"].includes(t.name)
      && (t.name !== "slice_browser_tab" || t.input?.action === "activate"))
    const actions = actionTools.map(t => history.find(a => a.action_id === t.output?.action_id))
    assert.deepEqual(actions.map(a => a?.kind), ["fill", "fill", "fill", "submit", "browser_tab_activate"],
      "office API result must be filled, submitted once and activated after extension invocation")
    for (let i = 0; i < actions.length; i++) {
      const action = actions[i]
      assert.ok(action.actor_id === `agent:${agentId}` && action.mode === "browser" && action.state === "completed",
        "office API action lacks official provider attribution")
      if (i) assert.ok(action.sequence > actions[i - 1].sequence, "office API action order differs")
      await input.waitForTuis(new RegExp(`^Room action #${action.sequence}: real-${input.options.provider} · browser ${action.kind} · completed$`))
    }
    const text = await input.officeRuntime.sliceScreen(["browser-text"])
    assert.ok(text.includes("CHARIOX_INVENTORY_UPDATED") && text.includes(release.tag_name), "inventory receipt is not visible")
    report.screenshot = await input.screenshot("office-api-completed")
    report.turn = turn
    report.sameProviderSession = true
    report.actions = actions.map(a => ({ id: a.action_id, sequence: a.sequence, kind: a.kind }))
    report.localTuiObserved = true
    report.remoteTuiObserved = true
    report.skipped = ["Web projection", "other providers", "managed-machine repeat", "provider save/resume", "independent source-code audit"]
    return report
  } catch (error) {
    report.diagnostic = await captureRoomProviderDiagnostic(input).catch(() => ({ codes: ["diagnostic_unavailable"] }))
    try { await input.screenshot("office-api-failed") } catch { /* Preserve the failure. */ }
    try { await input.checkpoint({ phase: "office-api-failed", officeApi: report }) } catch { /* Cleanup still runs. */ }
    throw error
  } finally {
    await service.close()
    report.fixtureClosed = true
  }
}
