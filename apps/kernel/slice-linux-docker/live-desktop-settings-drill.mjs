#!/usr/bin/env node
import assert from "node:assert/strict"
import { execFile } from "node:child_process"
import { randomUUID } from "node:crypto"
import { mkdir, mkdtemp, rm } from "node:fs/promises"
import { homedir } from "node:os"
import { promisify } from "node:util"
import { fileURLToPath } from "node:url"
import path from "node:path"
import { createDrillInterruption } from "../../cli/scripts/lib/drill-interruption.mjs"

const exec = promisify(execFile)
const source = path.dirname(fileURLToPath(import.meta.url))
const repo = path.resolve(source, "../../..")
const image = process.env.CHARIOX_DESKTOP_SETTINGS_IMAGE
assert.ok(image, "set CHARIOX_DESKTOP_SETTINGS_IMAGE to an existing slice image")
const id = `chariox-desktop-settings-${randomUUID().slice(0, 8)}`
const evidence = path.join(homedir(), ".codex/evidence/browser-computer-use/desktop-settings", id)
const document = "/home/slice/Documents/desktop-settings.txt"
const contents = "Chariox desktop settings\nSaved through the graphical editor.\nGrüße after restoration.\n"
let scratch
const interruption = createDrillInterruption()
const sleep = (ms) => interruption.sleep(ms)
const dockerRaw = async (...args) => {
  interruption.check()
  return (await exec("docker", args, { timeout: 120_000, maxBuffer: 128 * 1024 })).stdout
}
const docker = async (...args) => (await dockerRaw(...args)).trim()
const screen = (...args) => docker("exec", id, "/opt/chariox-slice/slice-screen.sh", ...args)
const user = (...args) => docker("exec", id, ...args)
async function input(command, value) {
  interruption.check()
  const pending = exec("docker", ["exec", "-i", id, "/opt/chariox-slice/slice-screen.sh", command,
    ...(command === "computer-key-stdin" ? ["1"] : [])], { timeout: 30_000, maxBuffer: 128 * 1024 })
  await Promise.all([pending, new Promise((resolve, reject) => {
    pending.child.stdin.once("error", reject)
    pending.child.stdin.end(value, (error) => error ? reject(error) : resolve())
  })])
}
const key = (value) => input("computer-key-stdin", value)
const waitFor = async (check) => {
  for (let attempt = 0; attempt < 80; attempt++) {
    const value = await check().catch(() => false)
    if (value) return value
    await sleep(100)
  }
  throw new Error("desktop settings probe did not complete")
}
async function launchProbe(phase, readOnly = false) {
  await user("env", "DISPLAY=:99", "xdotool", "key", "super+d")
  await user("env", "DISPLAY=:99", "xdotool", "mousemove", "40", "40", "click", "3")
  await sleep(200)
  await user("env", "DISPLAY=:99", "xdotool", "key", "Down", "Return")
  const result = await waitFor(() => user("cat", "/tmp/chariox-desktop-settings-result"))
  assert.equal(result, "true", "a setting saved by a desktop-launched app was lost")
  assert.equal(await user("cat", "/tmp/chariox-desktop-settings-error"), "", "settings save emitted a warning")
  const window = await waitFor(() => user("env", "DISPLAY=:99", "xdotool", "search", "--onlyvisible", "--class", "Mousepad"))
  assert.match(window, /^\d+$/, "expected exactly one graphical editor window")
  await user("env", "DISPLAY=:99", "xdotool", "windowactivate", "--sync", window)
  if (readOnly) {
    await key("ctrl+a")
    await key("ctrl+c")
    assert.equal(await dockerRaw("exec", id, "/opt/chariox-slice/slice-screen.sh", "computer-clipboard-read"), contents,
      "the restored editor did not display the saved document")
    await key("Right")
  } else {
    await input("computer-type-stdin", contents)
    await key("ctrl+s")
  }
  await waitFor(async () => (await dockerRaw("exec", id, "cat", document)) === contents)
  let captureAttempts = 0
  await waitFor(async () => {
    captureAttempts++
    await screen("screenshot", "/tmp/desktop-editor.png")
    return (await screen("ocr", "/tmp/desktop-editor.png")).includes("Chariox desktop settings")
  })
  console.log(JSON.stringify({ phase, renderedDocument: true, captureAttempts }))
  assert.equal(await user("env", "DISPLAY=:99", "xdotool", "getactivewindow"), window,
    "screenshot forced focus away from the graphical editor")
  await docker("cp", `${id}:/tmp/desktop-editor.png`, path.join(evidence, `${phase}.png`))
  assert.doesNotMatch(await user("cat", "/tmp/chariox-desktop-editor-log"), /dconf.*(?:WARNING|failed)|failed to commit changes/i)
  await key("ctrl+q")
  await waitFor(() => user("env", "DISPLAY=:99", "xdotool", "search", "--onlyvisible", "--class", "Mousepad")
    .then(() => false, (error) => { if (error.code === 1) return true; throw error }))
}
async function checkStopped() {
  let remaining = []
  await waitFor(async () => {
    const rows = (await user("ps", "-eo", "stat=,comm=")).split("\n")
    remaining = rows.filter((line) => {
      const [state, command] = line.trim().split(/\s+/)
      return !state.startsWith("Z") && /^(?:Xvfb|chromium|x11vnc|websockify|openbox|dbus-daemon|dbus-run-sessio[n]?|dconf-service)$/.test(command)
    })
    return remaining.length === 0
  }).catch(() => assert.fail(`desktop processes remain after shutdown: ${remaining.join(", ")}`))
}
async function stopAndCheck() {
  await screen("stop")
  await checkStopped()
}
async function create() {
  await docker("run", "-d", "--init", "--name", id,
    "--memory", "768m", "--memory-swap", "768m", "--cpus", "1", "--pids-limit", "1024",
    "--security-opt", `seccomp=${path.join(source, "chromium-seccomp.json")}`,
    "--mount", `type=bind,src=${repo},dst=/src,readonly`,
    "--mount", `type=bind,src=${path.join(source, "docker/slice-screen.sh")},dst=/opt/chariox-slice/slice-screen.sh,readonly`,
    "-e", "HOME=/home/slice", "-e", "CHARIOX_SLICE_VIEWER_BACKEND=novnc",
    image, "sleep", "infinity")
  console.log(JSON.stringify({ phase: "started", container: id }))
  await docker("exec", "-u", "root", id, "sh", "-c",
    "apt-get -qq update && apt-get -y -qq --no-install-recommends install libglib2.0-bin mousepad=0.5.10-2 && apt-get clean")
  await user("sh", "-c", [
    "set -eu",
    "mkdir -p /home/slice/.config/openbox",
    "cp /src/apps/kernel/slice-linux-docker/fixtures/desktop-settings/menu.xml /home/slice/.config/openbox/menu.xml",
  ].join("\n"))
  assert.equal(await user("gsettings", "get", "org.xfce.mousepad.preferences.view", "show-line-numbers"), "false",
  "fresh container already contains the saved setting")
}
await interruption.run(async () => {
  scratch = await mkdtemp(path.join(homedir(), ".chariox/dev/browser-computer-use/desktop-settings-"))
  await mkdir(evidence, { recursive: true })
  await create()
  await assert.rejects(() => docker("exec", id, "env",
    "PATH=/src/apps/kernel/slice-linux-docker/fixtures/desktop-settings/fail-bus:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
    "/opt/chariox-slice/slice-screen.sh", "start"),
  /Openbox did not stay running/, "a failed desktop session bus was reported as healthy")
  // Failure must clean up before the caller retries, not only on explicit stop.
  await checkStopped()
  console.log(JSON.stringify({ phase: "failed-startup", cleanedUp: true }))
  await screen("start")
  // Exercise a real application launch from Openbox. No injected bus address
  // or GSETTINGS_BACKEND override may make this probe pass.
  await launchProbe("saved")
  await stopAndCheck()
  await user("touch", "/tmp/chariox-desktop-read-only")
  await user("rm", "/tmp/chariox-desktop-settings-result")
  await screen("start")
  await launchProbe("restarted", true)
  await stopAndCheck()
  const originalContainer = await docker("inspect", "--format", "{{.Id}}", id)
  await docker("exec", "-u", "root", id, "tar", "--zstd", "-C", "/home/slice", "-cf", "/tmp/desktop-home.tar.zst", ".")
  const archive = path.join(scratch, "home.tar.zst")
  await docker("cp", `${id}:/tmp/desktop-home.tar.zst`, archive)
  await docker("rm", "-f", id)
  assert.equal(await docker("ps", "-aq", "--filter", `name=^${id}$`), "")
  await create()
  assert.notEqual(await docker("inspect", "--format", "{{.Id}}", id), originalContainer)
  await docker("cp", archive, `${id}:/tmp/desktop-home.tar.zst`)
  await docker("exec", "-u", "root", id, "tar", "--zstd", "-C", "/home/slice", "-xf", "/tmp/desktop-home.tar.zst")
  await user("touch", "/tmp/chariox-desktop-read-only")
  await screen("start")
  await launchProbe("restored", true)
  await stopAndCheck()
  console.log("SLICE_DESKTOP_SETTINGS_ARCHIVE_RESTORE_PASS")
  console.log(JSON.stringify({ evidence }))
  console.log("SLICE_DESKTOP_SETTINGS_PASS")
}, async () => {
  const failures = []
  try {
    try { await docker("rm", "-f", id) } catch (error) {
      if (!/No such container/.test(error.stderr ?? "")) throw error
    }
    assert.equal(await docker("ps", "-aq", "--filter", `name=^${id}$`), "", "owned desktop test container leaked")
  } catch (error) { failures.push(error) }
  if (scratch) {
    try { await rm(scratch, { recursive: true, force: true }) } catch (error) { failures.push(error) }
  }
  if (failures.length) throw new AggregateError(failures, "desktop settings cleanup failed")
  console.log(JSON.stringify({ cleanup: "passed", container: id }))
}, (error) => { console.error(error); process.exitCode = 1 })
