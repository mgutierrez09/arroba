import assert from "node:assert/strict"
import { captureRoomProviderDiagnostic } from "./live-room-provider-diagnostic.mjs"
import { assertRoomBrowserRecoveryActions, observeRoomStaleToolError } from "./live-room-browser-recovery.mjs"
import { waitForRoomProviderSettlement } from "./live-room-provider-settlement.mjs"
import { runRoomOfficeWork } from "./live-room-office-work.mjs"

// Opt-in only: this runs a paid, official provider through the kernel, not a
// driver impersonating an agent by calling its MCP endpoint.
export function roomRealProviderOptions(env) {
  const web = env.CHARIOX_ROOM_DRILL_FOCUS === "web-companion"
    && env.CHARIOX_ROOM_DRILL_WEB_REAL_PROVIDER === "1"
  if (env.CHARIOX_ROOM_DRILL_FOCUS !== "real-provider" && !web) return null
  const provider = env.CHARIOX_ROOM_DRILL_PROVIDER
  assert.ok(["codex", "claude", "opencode"].includes(provider), "select an official Room drill provider")
  const model = env.CHARIOX_ROOM_DRILL_MODEL?.trim()
  assert.ok(model, "CHARIOX_ROOM_DRILL_MODEL must explicitly select a provider model")
  const mode = env.CHARIOX_ROOM_DRILL_PROVIDER_MODE ?? "computer"
  assert.ok(["computer", "browser"].includes(mode), "select Browser or Computer provider mode")
  const computerTask = env.CHARIOX_ROOM_DRILL_COMPUTER_TASK
  assert.ok(computerTask === undefined || (computerTask === "office" && mode === "computer"), "invalid Computer task")
  assert.ok(computerTask === undefined || !web, "office work currently requires the standalone provider drill")
  const browserTask = env.CHARIOX_ROOM_DRILL_BROWSER_TASK
  assert.ok(browserTask === undefined || mode === "browser", "Browser task requires Browser mode")
  assert.ok(browserTask === undefined || ["click", "form"].includes(browserTask), "invalid Browser task")
  const browserLayout = env.CHARIOX_ROOM_DRILL_BROWSER_LAYOUT
  assert.ok(browserLayout === undefined || browserTask === "form", "Browser layout requires the form task")
  assert.ok(browserLayout === undefined || ["page", "nested-frame", "shadow-root"].includes(browserLayout), "invalid Browser layout")
  const browserMutation = env.CHARIOX_ROOM_DRILL_BROWSER_MUTATION
  assert.ok(browserMutation === undefined || browserTask === "form", "Browser mutation requires the form task")
  assert.ok(browserMutation === undefined || browserMutation === "replace-field", "invalid Browser mutation")
  return { provider, model, mode, ...(computerTask ? { computerTask } : {}), ...(browserTask ? { browserTask } : {}), ...(browserLayout ? { browserLayout } : {}),
    ...(browserMutation ? { browserMutation } : {}), accountProfile: "default", importFirst: env.CHARIOX_ROOM_DRILL_IMPORT_FIRST === "1" }
}

export async function runRoomRealProvider(input) {
  const result = await runRoomRealProviderAction(input)
  await input.waitForPhysicalEffect(result.expectedPhysicalEffect)
  if (result.browserMutation === "replace-field") {
    await input.waitForPhysicalEffect("BROWSER_STALE_RECOVERY_ACCEPTED")
    await input.waitForTuis(new RegExp(`^Room action #${result.replacementActionSequence}: real-${result.provider} · browser click · completed$`))
    await input.waitForTuis(new RegExp(`^Room action #${result.staleActionSequence}: real-${result.provider} · browser fill · failed \\(controller_failure\\)$`))
  }
  if (result.browserTask === "form") {
    await input.waitForPhysicalEffect("BROWSER_FORM_ACCEPTED")
    await input.waitForTuis(new RegExp(`^Room action #${result.fillActionSequence}: real-${result.provider} · browser fill · completed$`))
  } else if (result.mode === "browser") await input.waitForPhysicalEffect("BROWSER_CLICK_ACCEPTED")
  await input.waitForTuis(new RegExp(`^Room action #\\d+: real-${result.provider} · ${result.mode} ${result.actionKind} · completed$`))
  await input.screenshot("after-real-provider-click")
  const verified = {
    ...result, physicalEffect: result.expectedPhysicalEffect, localTuiObserved: true, remoteTuiObserved: true,
    coverage: `Official provider calls Chariox ${result.mode} input in the shared Room`,
    skipped: [result.mode === "browser" ? "remaining Browser action matrix" : "structured Browser actions",
      "Web observation of the provider action", "provider save and resume"],
  }
  if (input.options.computerTask === "office") {
    verified.office = await runRoomOfficeWork({ ...input, agentId: result.agentId })
  }
  await input.checkpoint({ phase: "passed", ...verified })
  return verified
}

// Shared by the headless physical/TUI drill and the Web companion. This only
// proves the attributed kernel action; each caller must verify its own viewers.
export async function runRoomRealProviderAction(input) {
  const { client, requests, sessionId, sliceId, options } = input
  const mode = options.mode ?? "computer"
  assert.ok(["computer", "browser"].includes(mode), "select Browser or Computer provider mode")
  const form = mode === "browser" && options.browserTask === "form"
  const recovery = form && options.browserMutation === "replace-field"
  const actionKind = mode === "browser" ? (form ? "submit" : "click") : "pointer_click"
  assert.ok(!(input.agent && options.importFirst), "import-first must precede agent creation")
  if (options.importFirst) {
    await input.checkpoint({ phase: "importing-account", provider: options.provider })
    unwrap(await client.send(requests.importSliceProviderAuthRequest(
      sliceId, options.provider, options.accountProfile,
    )), "SliceProviderAuthImported")
  }
  await input.checkpoint({ phase: "spawning", provider: options.provider, importFirst: options.importFirst })
  const alias = `real-${options.provider}`
  const agent = input.agent ?? unwrap(await client.send(requests.spawnAgentRequest(
    sessionId, options.provider, alias, options.model, input.workspace,
    "low", "build", "yolo", undefined, undefined, sliceId, options.accountProfile,
  )), "AgentSpawned").agent
  const attachment = unwrap(await client.send(requests.attachToSessionRequest(
    sessionId, "real-provider-drill",
  )), "SessionAttached").attachment
  await input.checkpoint({ phase: "prompting", provider: options.provider, agentId: agent.id })
  const actorId = `agent:${agent.id}`
  let action
  let fillAction
  let recoveryActions
  let settlement
  let verifiedAgent = agent
  let baselineSequence = 0
  const priorTurnIds = new Set()
  let lastFailureProbe = 0
  try {
    await input.beforePrompt?.(agent)
    if (input.agent) {
      const state = unwrap(await input.withTimeout(client.send(requests.getSessionStateRequest(sessionId)),
        5_000, "provider configuration lookup"), "SessionState")
      verifiedAgent = state.session?.agents?.find((item) => item.id === agent.id)
      const slices = unwrap(await input.withTimeout(client.send(requests.listSlicesRequest()),
        5_000, "provider slice lookup"), "SlicesListed").slices
      assert.ok(slices.some((slice) => slice.id === sliceId && slice.agent_ids?.includes(agent.id)),
        "real provider must belong to the intended slice")
    }
    assert.ok(verifiedAgent && verifiedAgent.session_id === sessionId
      && verifiedAgent.provider === options.provider && verifiedAgent.model === options.model
      && (verifiedAgent.account_profile ?? "default") === options.accountProfile,
    "authoritative provider configuration does not match the requested provider/model/profile/Room")
    if (input.agent) {
      assert.equal(verifiedAgent.is_processing, false, "reused provider must be idle before this drill")
      const outline = unwrap(await input.withTimeout(client.send(requests.getSessionHistoryOutlineRequest(
        sessionId, [agent.id], 2)), 5_000, "provider history baseline"), "SessionHistoryOutline")
      const turns = outline.agents?.find((item) => item.agent_id === agent.id)?.turns ?? []
      for (const turn of turns.slice(0, 2)) {
        assert.ok(typeof turn.turn_id === "string" && turn.turn_id.length > 0, "provider history baseline lacks a turn identity")
        priorTurnIds.add(turn.turn_id)
      }
    }
    const baseline = unwrap(await input.withTimeout(client.send(requests.listRoomEnvironmentActionHistoryRequest(
      sessionId, null, 100)), 5_000, "provider action baseline"), "RoomEnvironmentActionHistoryListed").page.actions
    baselineSequence = baseline.reduce((latest, item) => Number.isSafeInteger(item.sequence)
      ? Math.max(latest, item.sequence) : latest, 0)
    const prompt = form ? [
      "You are validating the Chariox Room browser. Use only the Chariox runtime MCP tools.",
      "Use slice_browser_find with query='Browser sample' and kind=field.",
      ...(recovery ? [
        "Keep that original field_id. Find the button 'Replace Browser field' and click it exactly once with slice_browser_click.",
        "Now try slice_browser_fill exactly once with the ORIGINAL field_id and text='STALE ATTEMPT MUST NOT LAND'. This attempt must fail because the field was replaced.",
        "If it fails, use slice_browser_find again with query='Browser sample' and kind=field to rediscover the replacement. Do not reuse the original reference.",
      ] : []),
      "Call slice_browser_fill with that returned opaque field_id and text='Chariox form sample'.",
      "Then use slice_browser_find with query='Submit Browser form' and kind=button.",
      "Call slice_browser_submit once with the returned button field_id to submit its form.",
      "The shared browser is already open. Do not use coordinates, Computer input, shell commands, scripts, or provider-native browser tools.",
      "Do not navigate independently, open another browser, edit files, or contact external services.",
      "After form submission, stop and report whether the tool succeeded.",
    ].join(" ") : mode === "browser" ? [
      "You are validating the Chariox Room browser. Use only the Chariox runtime MCP tools.",
      "Use slice_browser_find with query='Browser action target' and kind=button.",
      "Then call slice_browser_click exactly once with the returned opaque field_id for that button.",
      "Do not use coordinates, Computer input, shell commands, provider-native browser tools, or scripts.",
      "The shared browser is already open. Do not navigate, open another browser, edit files, or contact external services.",
      "After the single button click, stop and report whether the tool succeeded.",
    ].join(" ") : [
    "You are validating the Chariox Room computer. Use only the Chariox runtime MCP tools.",
    "Call slice_mouse exactly once with action=click, x=640, y=400, button=left.",
    "The Room desktop is already running. Do not launch a browser, navigate, use shell commands,",
    "edit any files, or call any external service. Do not use a provider-native browser tool.",
    "After that single click, stop and report whether the tool succeeded.",
    ].join(" ")
    const submitted = unwrap(await client.send(requests.submitPromptRequest(sessionId, attachment.id, agent.id, prompt, [])), "PromptSubmitted")
    const promptId = (submitted.outcome?.Started ?? submitted.outcome?.Queued)?.prompt?.id
    const findCompletedAction = (actions) => {
      const completed = actions.find((item) => item.actor_id === actorId
        && Number.isSafeInteger(item.sequence) && item.sequence > baselineSequence
        && item.kind === actionKind && item.state === "completed")
      if (completed) {
        if (form) fillAction = assertRoomBrowserFormActions(actions, completed, baselineSequence)
        if (recovery) recoveryActions = assertRoomBrowserRecoveryActions(actions, completed, baselineSequence)
      }
      return completed
    }
    action = await input.waitFor(async () => {
      const actions = unwrap(await client.send(requests.listRoomEnvironmentActionHistoryRequest(
        sessionId, null, 100,
      )), "RoomEnvironmentActionHistoryListed").page.actions
      const completed = findCompletedAction(actions)
      if (completed) return completed
      // Ignore turns predating this prompt when Web reuses an idle agent.
      // A warning/error on a still-open turn must not abort the action wait.
      if (Date.now() - lastFailureProbe >= 2_000) {
        lastFailureProbe = Date.now()
        const outline = await input.withTimeout(client.send(requests.getSessionHistoryOutlineRequest(
          sessionId, [agent.id], 2,
        )), 2_000, "provider failure probe").catch(() => null)
        const turns = outline?.SessionHistoryOutline?.agents?.find((item) => item.agent_id === agent.id)?.turns ?? []
        const completedTurns = turns.slice(0, 2).filter((turn) => !priorTurnIds.has(turn.turn_id)
          && (!input.agent || typeof turn.turn_id === "string") && turn.lifecycle === "completed")
        if (completedTurns.length) {
          // The action may have committed between the first history read and
          // observing the completed turn. Re-read before declaring it missing.
          const latest = unwrap(await input.withTimeout(client.send(requests.listRoomEnvironmentActionHistoryRequest(
            sessionId, null, 100,
          )), 2_000, "completed provider action lookup"), "RoomEnvironmentActionHistoryListed").page.actions
          const finalAction = findCompletedAction(latest)
          if (finalAction) return finalAction
          const providerFailed = completedTurns.some((turn) => turn.summary?.entry?.kind === "provider_error"
            || (turn.entries ?? []).slice(0, 256).some((item) => item.entry?.kind === "provider_error")
            || (turn.blobs ?? []).slice(0, 16).some((item) => item.kind === "provider_error"))
          return providerFailed ? { providerFailed: true } : { providerCompletedWithoutAction: true }
        }
      }
      return false
    }, 180_000, `official provider did not complete a Room ${mode} ${actionKind}`)
    if (action.providerFailed) throw new Error("official provider turn failed before completing the Room action")
    if (action.providerCompletedWithoutAction) throw new Error("official provider turn completed without the required Room action")
    if (recovery) await input.waitFor(() => observeRoomStaleToolError(input, agent.id, priorTurnIds),
      15_000, "provider tool history did not confirm stale_element_reference")
    settlement = await waitForRoomProviderSettlement(input, agent.id, promptId)
  } catch (error) {
    const diagnostic = await captureRoomProviderDiagnostic({ ...input, agentId: agent.id })
      .catch(() => ({ codes: ["diagnostic_unavailable"] }))
    await input.checkpoint({ phase: "action-failed", provider: options.provider, agentId: agent.id, diagnostic })
    throw error
  }
  assertRoomRealProviderAction(action, mode, options.browserTask)
  const result = {
    provider: verifiedAgent.provider, model: verifiedAgent.model, accountProfile: verifiedAgent.account_profile ?? "default", importFirst: options.importFirst,
    agentId: agent.id, actorId, actionId: action.action_id, mode, actionKind,
    baselineSequence, actionSequence: action.sequence,
    settlement,
    ...(form ? { browserTask: "form", fillActionId: fillAction.action_id, fillActionSequence: fillAction.sequence } : {}),
    ...(options.browserLayout ? { browserLayout: options.browserLayout } : {}),
    ...(recovery ? { browserMutation: options.browserMutation, staleErrorObserved: true,
      replacementActionId: recoveryActions.replacement.action_id, replacementActionSequence: recoveryActions.replacement.sequence,
      staleActionId: recoveryActions.stale.action_id, staleActionSequence: recoveryActions.stale.sequence } : {}),
    expectedPhysicalEffect: input.expectedPhysicalEffect ?? (form ? "POINTER_CLICK_COUNT=1" : "POINTER_CLICK_COUNT=2"),
  }
  await input.checkpoint({ phase: "action-completed", ...result })
  return result
}

export function assertRoomRealProviderAction(action, mode = "computer", browserTask = "click") {
  assert.ok(["computer", "browser"].includes(mode), "invalid provider mode")
  assert.equal(action.mode, mode)
  assert.equal(action.state, "completed")
  assert.equal(action.kind, mode === "browser" ? (browserTask === "form" ? "submit" : "click") : "pointer_click")
  if (mode === "browser") {
    assert.equal(action.targets?.length, 1, "Browser click must target exactly one tab")
    assert.equal(action.targets[0].kind, "browser_tab")
    assert.ok(typeof action.targets[0].id === "string" && action.targets[0].id.length > 0)
  } else {
    assert.equal(action.arguments.x, 640)
    assert.equal(action.arguments.y, 400)
    assert.equal(action.arguments.button, "left")
    assert.equal(action.arguments.click_count, 1)
  }
}

export function assertRoomBrowserFormActions(actions, submission, baselineSequence) {
  assertRoomRealProviderAction(submission, "browser", "form")
  assert.ok(Number.isSafeInteger(baselineSequence) && baselineSequence >= 0, "invalid form baseline")
  assert.ok(Number.isSafeInteger(submission.sequence) && submission.sequence > baselineSequence, "submit must be fresh")
  const fill = actions.find((item) => item.kind === "fill" && item.mode === "browser"
    && item.state === "completed" && item.actor_id === submission.actor_id
    && Number.isSafeInteger(item.sequence) && item.sequence > baselineSequence && item.sequence < submission.sequence
    && item.targets?.length === 1 && item.targets[0].kind === "browser_tab"
    && item.targets[0].id === submission.targets[0].id)
  assert.ok(fill, "form submission requires a fresh completed fill by the same actor in the same tab")
  return fill
}

function unwrap(response, variant) {
  assert.ok(response && variant in response, `kernel did not return ${variant}`)
  return response[variant]
}
