import assert from "node:assert/strict"
import { createHash, randomUUID } from "node:crypto"
import { startBrowserComputerFixture } from "./browser-computer-fixture.mjs"
import { waitForRoomProviderSettlement } from "./live-room-provider-settlement.mjs"
import { captureRoomProviderDiagnostic } from "./live-room-provider-diagnostic.mjs"

// The driver installs applications and establishes a synthetic mail session.
// Only the official provider may create/edit/save/upload the document.
export async function runRoomOfficeWork(input) {
  const { client, requests, sessionId, agentId, options, officeRuntime } = input
  const { containerName, docker, sliceScreen, runCommandWithStdin } = officeRuntime
  const document = `/home/slice/Documents/office-${randomUUID()}.txt`
  const contents = "Chariox office document\nPrepared through the graphical editor.\nGrüße from the Room.\n"
  const subject = `Office document ${randomUUID()}`
  const password = randomUUID()
  const account = "agent@chariox.test"
  const recipient = "recipient@chariox.test"
  const actorId = `agent:${agentId}`
  const fixture = await startBrowserComputerFixture({ host: "0.0.0.0", account, password })
  const origin = `http://host.docker.internal:${new URL(fixture.origin).port}`
  const report = { document, subject, agentId, provider: options.provider, model: options.model }
  const command = async (args) => (await docker(["exec", "-u", "slice", containerName, ...args])).stdout
  const actions = async () => unwrap(await client.send(requests.listRoomEnvironmentActionHistoryRequest(
    sessionId, null, 100)), "RoomEnvironmentActionHistoryListed").page.actions
  let baseline = 0
  const fresh = (list) => list.filter((a) => a.actor_id === actorId && a.sequence > baseline)
  const phase = async (name) => input.checkpoint({ phase: name, office: report })

  async function prompt(text, finished) {
    const history = await actions()
    baseline = Math.max(0, ...history.map((a) => a.sequence))
    // The agent turn outlives its submitting client. Do not retain an idle
    // terminal attachment across installation or the preceding provider turn.
    const attachment = unwrap(await client.send(requests.attachToSessionRequest(
      sessionId, "office-drill")), "SessionAttached").attachment
    let submitted
    try {
      submitted = unwrap(await client.send(requests.submitPromptRequest(
        sessionId, attachment.id, agentId, text, [])), "PromptSubmitted")
    } finally {
      await client.send(requests.detachFromSessionRequest(attachment.id))
    }
    const promptId = (submitted.outcome?.Started ?? submitted.outcome?.Queued)?.prompt?.id
    assert.ok(promptId, "office prompt lacks an identity")
    const completed = await input.waitFor(async () => {
      if (await finished(fresh(await actions()))) return true
      const outline = unwrap(await input.withTimeout(client.send(requests.getSessionHistoryOutlineRequest(
        sessionId, [agentId], 2)), 5_000, "office provider progress"), "SessionHistoryOutline")
      const turn = outline.agents?.find((a) => a.agent_id === agentId)?.turns?.find((t) => t.prompt_id === promptId)
      if (turn?.lifecycle === "completed") {
        return await finished(fresh(await actions())) ? true : { missingResult: true }
      }
      return false
    }, 300_000, "office provider did not complete its task")
    assert.equal(completed, true, "office provider completed without the required physical result")
    return waitForRoomProviderSettlement(input, agentId, promptId)
  }

  try {
    await phase("office-installing")
    await docker(["exec", "-u", "root", containerName, "bash", "-c", [
      "set -euo pipefail", "apt-get -qq update",
      "apt-get -y -qq --no-install-recommends install mousepad=0.5.10-2 xterm",
      "apt-get clean", "find /var/lib/apt/lists -mindepth 1 -delete",
    ].join("\n")])
    await command(["mkdir", "-p", "/home/slice/Documents"])
    report.installed = (await command(["dpkg-query", "-W", "-f=${Package} ${Version}\n", "mousepad", "xterm"])).trim()
    const menu = await command(["cat", "/etc/xdg/openbox/menu.xml"])
    assert.match(menu, /item label="Terminal emulator">\s*<action name="Execute"><execute>x-terminal-emulator<\/execute>/,
      "the desktop must provide its standard Terminal emulator menu item")
    const display = (await command(["sh", "-c", 'printf %s "${CHARIOX_SLICE_DISPLAY:-:99}"'])).trim()
    assert.match(display, /^:\d+(?:\.\d+)?$/)
    const activeClass = async () => command(["env", `DISPLAY=${display}`, "sh", "-c",
      'xprop -id "$(xdotool getactivewindow)" WM_CLASS'])

    // Establish authentication before the provider starts. No fixture password
    // appears in model context, a command argument, or a screenshot.
    await sliceScreen(["open-url", `${origin}/mail/login`])
    await input.waitFor(async () => (await sliceScreen(["browser-text"])).includes("Fixture mail login"),
      20_000, "office fixture did not open")
    await sliceScreen(["browser-fill", "#email", account])
    const pasted = await runCommandWithStdin("docker", ["exec", "-i", "-u", "slice", containerName,
      "/opt/chariox-slice/slice-screen.sh", "secret-paste-stdin", "#password"], password, 20_000)
    assert.equal(pasted.code, 0, "fixture login setup failed")
    await sliceScreen(["browser-submit", "#password"])
    await input.waitFor(async () => (await sliceScreen(["browser-text"])).includes("CHARIOX_FIXTURE_INBOX"),
      20_000, "office fixture session was not established")
    await phase("office-editing")
    report.editTurn = await prompt([
      "Validate real graphical office work in this Room. Use only Chariox runtime MCP tools, no provider shell/file tools or scripts.",
      "Use slice_keyboard to press super+d to show the desktop, then slice_mouse to right-click its background. Inspect using slice_screenshot with return_image_base64=true.",
      "Use slice_find_text to locate the 'Terminal emulator' menu item and slice_mouse to click it. Inspect the terminal before typing.",
      `Through the terminal's graphical keyboard input, type mousepad --disable-server ${document} and press Return. This command may only launch the editor, never write the document.`,
      "Inspect the desktop and wait for Mousepad. All document editing must use slice_keyboard and the graphical editor.",
      `Type exactly the text represented by this JSON string, interpreting its newline escapes as real newlines: ${JSON.stringify(contents)}.`,
      "Press ctrl+s to save. Use slice_screenshot to verify the editor remains focused and the text is visible.",
      "Leave Mousepad open and focused. Do not interact with Chromium, email, the clipboard, or any other service during this turn. Then stop.",
    ].join(" "), async (list) => {
      const typed = list.some((a) => a.kind === "keyboard_text" && a.mode === "computer" && a.state === "completed"
        && a.arguments?.utf8_byte_count === Buffer.byteLength(contents))
      if (!typed) return false
      return (await command(["cat", document]).catch(() => null)) === contents
    })
    assert.match(await activeClass(), /Mousepad/i, "provider did not leave the graphical editor focused")
    await input.screenshot("office-editor-saved")
    assert.match(await activeClass(), /Mousepad/i, "screenshot forced browser focus during office work")
    const editActions = fresh(await actions())
    const typed = editActions.find((a) => a.kind === "keyboard_text" && a.state === "completed"
      && a.arguments?.utf8_byte_count === Buffer.byteLength(contents))
    assert.ok(typed)
    await input.waitForTuis(new RegExp(`^Room action #${typed.sequence}: real-${options.provider} · computer keyboard_text · completed$`))
    report.edit = { exactDocument: true, focusPreserved: true, localTuiObserved: true, remoteTuiObserved: true,
      typedActionId: typed.action_id, typedSequence: typed.sequence }
    await phase("office-mailing")
    report.mailTurn = await prompt([
      "Continue in the same Room. The graphical editor document is saved. Use only Chariox runtime MCP Browser tools for this turn.",
      `Open ${origin}/mail/compose with slice_open_url. It is already signed in.`,
      `Discover the To, Subject and Body fields, and fill To with ${recipient}, Subject with ${JSON.stringify(subject)}, Body with 'Attached is the graphical editor document.'.`,
      `Find the Attachment file field and use slice_browser_upload with its returned field_id and files=[${JSON.stringify(document)}].`,
      "Then find Send and submit its form exactly once. Inspect the confirmation with slice_browser_text and stop.",
      "Do not create or edit any file, use shell commands, scripts, direct HTTP, another browser, or another service.",
    ].join(" "), async (list) => list.some((a) => a.mode === "browser" && a.kind === "upload" && a.state === "completed")
      && fixture.messages.some((m) => m.subject === subject))
    const delivered = fixture.messages.filter((m) => m.subject === subject)
    assert.equal(delivered.length, 1, "office mail must be submitted exactly once")
    const fileBytes = Buffer.from(await command(["cat", document]), "utf8")
    assert.equal(fileBytes.toString("utf8"), contents, "the uploaded document changed after GUI verification")
    assert.equal(delivered[0].to, recipient)
    assert.equal(delivered[0].attachments?.length, 1)
    const received = delivered[0].attachments[0]
    assert.equal(received.name, document.split("/").at(-1))
    assert.equal(received.sizeBytes, Buffer.byteLength(contents))
    assert.equal(received.sha256, createHash("sha256").update(fileBytes).digest("hex"))
    const mailActions = fresh(await actions())
    const upload = mailActions.find((a) => a.kind === "upload" && a.state === "completed")
    const submits = mailActions.filter((a) => a.kind === "submit" && a.state === "completed" && a.sequence > upload.sequence)
    assert.equal(submits.length, 1, "office mail needs one attributed form submission after upload")
    for (const action of [upload, submits[0]]) {
      await input.waitForTuis(new RegExp(`^Room action #${action.sequence}: real-${options.provider} · browser ${action.kind} · completed$`))
    }
    await input.screenshot("office-mail-sent")
    report.mail = { received, uploadActionId: upload.action_id, submitActionId: submits[0].action_id,
      localTuiObserved: true, remoteTuiObserved: true, submissions: 1 }
    report.skipped = ["Web viewer projection", "real external email service", "provider save/resume", "other providers and office scenarios"]
    await phase("office-passed")
    return report
  } catch (error) {
    await input.screenshot("office-failed").catch(() => undefined)
    report.diagnostic = await captureRoomProviderDiagnostic(input).catch(() => ({ codes: ["diagnostic_unavailable"] }))
    await phase("office-failed")
    throw error
  } finally {
    await fixture.close()
    report.fixtureClosed = true
  }
}

function unwrap(response, key) {
  assert.ok(response?.[key], `expected ${key}`)
  return response[key]
}
