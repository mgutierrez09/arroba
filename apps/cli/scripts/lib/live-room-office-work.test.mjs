import assert from "node:assert/strict"
import test from "node:test"
import { runRoomOfficeWork } from "./live-room-office-work.mjs"

// Model terminal attachment expiry at the two real idle boundaries without
// Docker, a provider, credentials, or a slow wall-clock sleep.
for (const boundary of ["office-installing", "office-mailing"]) {
  test(`office prompts survive attachment expiry during ${boundary}`, async () => {
    const live = new Set()
    let next = 0
    let submitted = 0
    let loggedIn = false
    let typed = false
    const contents = "Chariox office document\nPrepared through the graphical editor.\nGrüße from the Room.\n"
    const stop = new Error("second prompt reached kernel")
    const requests = Object.fromEntries([
      "attachToSession", "detachFromSession", "submitPrompt", "listRoomEnvironmentActionHistory",
      "getSessionHistoryOutline", "getSessionState",
    ].map((name) => [`${name}Request`, (...args) => ({ name, args })]))
    const input = {
      requests, sessionId: "room", agentId: "agent", options: { provider: "codex", model: "fixture" },
      checkpoint: async ({ phase, office }) => {
        // The standard slice controller permits /workspace and Downloads,
        // not arbitrary files under the user's home directory.
        assert.ok(office.document.startsWith("/workspace/"), "office document must stay in the authorized upload workspace")
        if (phase === boundary) live.clear()
      },
      screenshot: async () => {}, waitForTuis: async () => {},
      withTimeout: async (promise) => promise,
      waitFor: async (check) => {
        const result = await check()
        assert.ok(result, "fixture did not reach the expected result")
        return result
      },
      client: { send: async ({ name, args }) => {
        if (name === "attachToSession") {
          const id = `attachment-${++next}`
          live.add(id)
          return { SessionAttached: { attachment: { id } } }
        }
        if (name === "detachFromSession") {
          live.delete(args[0])
          return { SessionDetached: {} }
        }
        if (name === "submitPrompt") {
          assert.ok(live.has(args[1]), "attachment was not found")
          submitted++
          if (submitted === 2) throw stop
          typed = true
          return { PromptSubmitted: { outcome: { Started: { prompt: { id: "prompt" } } } } }
        }
        if (name === "listRoomEnvironmentActionHistory") return { RoomEnvironmentActionHistoryListed: {
          page: { actions: typed ? [{ actor_id: "agent:agent", sequence: 1, action_id: "typed",
            kind: "keyboard_text", mode: "computer", state: "completed",
            arguments: { utf8_byte_count: Buffer.byteLength(contents) } }] : [] },
        } }
        if (name === "getSessionHistoryOutline") return { SessionHistoryOutline: { agents: [{ agent_id: "agent",
          turns: [{ prompt_id: "prompt", turn_id: "turn", lifecycle: "completed" }] }] } }
        if (name === "getSessionState") return { SessionState: { session: { agents: [{ id: "agent", is_processing: false }] } } }
        throw new Error(`unexpected fixture request ${name}`)
      } },
      officeRuntime: {
        containerName: "fixture",
        docker: async (args) => {
          let stdout = ""
          if (args.includes("node")) stdout = JSON.stringify({ browserRunning: true, insecureOriginException: false,
            sandboxDisabled: false, sandboxedRenderers: true, taskbarRunning: true, desktopSessionBus: true, editorSessionBus: true, editorDefaultSettings: true })
          else if (args.includes("/etc/xdg/openbox/menu.xml")) stdout = '<item label="Terminal emulator"><action name="Execute"><execute>x-terminal-emulator</execute>'
          else if (args.some((arg) => arg.includes("CHARIOX_SLICE_DISPLAY"))) stdout = ":99"
          else if (args.some((arg) => arg.includes("WM_CLASS"))) stdout = "Mousepad"
          else if (args.includes("cat")) stdout = contents
          return { stdout }
        },
        sliceScreen: async ([action]) => {
          if (action === "browser-submit") loggedIn = true
          return loggedIn ? "CHARIOX_FIXTURE_INBOX" : "Fixture mail login"
        },
        runCommandWithStdin: async () => ({ code: 0 }),
      },
    }
    await assert.rejects(runRoomOfficeWork(input), (error) => error === stop)
    assert.equal(submitted, 2)
    assert.equal(live.size, 0, "prompt attachments must be released even when submission fails")
  })
}
