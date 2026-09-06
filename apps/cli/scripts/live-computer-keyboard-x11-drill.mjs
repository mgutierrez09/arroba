#!/usr/bin/env node

import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { createHash } from "node:crypto"
import { mkdir, readFile, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..")
const screen = path.join(repo, "apps/kernel/slice-linux-docker/docker/slice-screen.sh")
const stamp = new Date().toISOString().replace(/[:.]/g, "-")
const container = `chariox-keyboard-x11-${process.pid}`
const root = "/tmp/chariox-keyboard-x11"
const profile = `${root}/profile`
const image = process.env.CHARIOX_SLICE_IMAGE ?? "chariox-slice-linux:0.1.0"
const evidence = path.join(os.homedir(), ".codex/evidence/browser-computer-use/computer-keyboard-x11", stamp)
const report = { startedAt: new Date().toISOString(), cases: [], resources: [], cleanup: null }
const children = new Set()
let interrupted = false

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.once(signal, () => {
    interrupted = true
    for (const child of children) child.kill("SIGTERM")
  })
}

function command(executable, args, input = null) {
  return new Promise((resolve, reject) => {
    const child = spawn(executable, args, { cwd: repo, stdio: ["pipe", "pipe", "pipe"] })
    children.add(child)
    const out = []
    const err = []
    const timer = setTimeout(() => child.kill("SIGKILL"), 30_000)
    child.stdout.on("data", (chunk) => out.push(chunk))
    child.stderr.on("data", (chunk) => err.push(chunk))
    child.once("error", (error) => {
      clearTimeout(timer)
      children.delete(child)
      reject(error)
    })
    child.once("close", (code, signal) => {
      clearTimeout(timer)
      children.delete(child)
      if (code !== 0) return reject(new Error(`${executable} failed: ${code ?? signal}: ${Buffer.concat(err)}`))
      resolve(Buffer.concat(out).toString("utf8"))
    })
    child.stdin.on("error", () => {})
    child.stdin.end(input ?? undefined)
  })
}

const docker = (args, input) => command("docker", args, input)
const exec = (args, input) => docker(["exec", "-i", "-u", "slice", "-e", "DISPLAY=:99", "-e", `CHARIOX_SLICE_CHROME_PROFILE=${profile}`, container, ...args], input)

async function resources(label) {
  const [memory, disk] = await Promise.all([
    command("memory_pressure", ["-Q"]).catch(() => "unavailable"),
    command("df", ["-k", repo]),
  ])
  report.resources.push({ label, at: new Date().toISOString(), memory: memory.trim(), disk: disk.trim().split("\n").at(-1) })
}

const evaluateScript = `
const chunks=[];for await(const chunk of process.stdin)chunks.push(chunk);
const expression=Buffer.concat(chunks).toString('utf8');
const pages=await(await fetch('http://127.0.0.1:9222/json/list')).json();
const page=pages.find(page=>page.type==='page');
const socket=new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve,reject)=>{socket.onopen=resolve;socket.onerror=reject});
socket.send(JSON.stringify({id:1,method:'Runtime.evaluate',params:{expression,returnByValue:true}}));
const result=await new Promise((resolve,reject)=>{socket.onmessage=event=>{const data=JSON.parse(event.data);if(data.id===1)resolve(data)};socket.onerror=reject});
socket.close();
if(result.error||result.result?.exceptionDetails)throw new Error('fixture evaluation failed');
process.stdout.write(JSON.stringify(result.result.result.value));
`
const evaluate = async (expression) => JSON.parse(await exec(["node", "--input-type=module", "-e", evaluateScript], expression))
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

await mkdir(evidence, { recursive: true })
let failure = null
try {
  report.sourceCommit = (await command("git", ["rev-parse", "HEAD"])).trim()
  report.topology = { kind: "local-docker-x11", host: os.hostname(), hostOs: os.release(), network: "none" }
  report.resourceLimits = { memoryBytes: 805_306_368, memorySwapBytes: 805_306_368, nanoCpus: 1_000_000_000, pidsLimit: 256 }
  report.command = "node apps/cli/scripts/live-computer-keyboard-x11-drill.mjs"
  report.drillSha256 = createHash("sha256").update(await readFile(fileURLToPath(import.meta.url))).digest("hex")
  report.screenSha256 = createHash("sha256").update(await readFile(screen)).digest("hex")
  report.keyboardSha256 = createHash("sha256").update(await readFile(path.join(path.dirname(screen), "slice-keyboard.py"))).digest("hex")
  report.imageId = (await docker(["image", "inspect", "--format", "{{.Id}}", image])).trim()
  await resources("before")
  await docker(["run", "-d", "--rm", "--name", container, "--memory", "768m", "--memory-swap", "768m", "--cpus", "1", "--pids-limit", "256", "--network", "none", "--entrypoint", "/bin/sleep", image, "infinity"])
  await docker(["exec", "-u", "root", container, "mkdir", "-p", root])
  await docker(["cp", screen, `${container}:${root}/slice-screen.sh`])
  await docker(["cp", path.join(path.dirname(screen), "slice-keyboard.py"), `${container}:${root}/slice-keyboard.py`])
  await docker(["exec", "-u", "root", container, "chown", "-R", "slice:slice", root])
  await exec(["node", "-e", "const fs=require('node:fs');let data='';process.stdin.on('data',c=>data+=c);process.stdin.on('end',()=>fs.writeFileSync(process.argv[1],data));", `${root}/fixture.html`], '<!doctype html><input id="input" type="password" autofocus><textarea id="multiline"></textarea><script>window.busy=false;setInterval(()=>{if(!busy)return;const end=performance.now()+120;while(performance.now()<end){}},160)</script>')
  await docker(["exec", "-d", "-u", "slice", container, "Xvfb", ":99", "-screen", "0", "1280x800x24", "-ac", "+extension", "XTEST"])
  await pause(500)
  await docker(["exec", "-d", "-u", "slice", "-e", "DISPLAY=:99", container, "chromium", "--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage", "--no-first-run", "--no-default-browser-check", `--user-data-dir=${profile}`, "--remote-debugging-port=9222", `file://${root}/fixture.html`])
  let ready = false
  for (let attempt = 0; attempt < 60 && !ready; attempt++) {
    if (interrupted) throw new Error("drill interrupted")
    ready = await evaluate("Boolean(document.querySelector('#input'))").catch(() => false)
    if (!ready) await pause(250)
  }
  assert.ok(ready, "keyboard fixture ready")
  report.containerStats = (await docker(["stats", "--no-stream", "--format", "{{json .}}", container])).trim()
  report.versions = (await exec(["bash", "-lc", "chromium --version; xdotool version"])).trim()
  for (const separator of ["\n", "\r"]) {
    await evaluate("document.querySelector('#multiline').value='';document.querySelector('#multiline').focus();true")
    await exec([`${root}/slice-screen.sh`, "computer-type-stdin"], `first${separator}Grüße 世界${separator}`)
    const actual = await evaluate("document.querySelector('#multiline').value")
    report.cases.push({ kind: "multiline", separator: separator === "\n" ? "LF" : "CR",
      matches: actual === "first\nGrüße 世界\n" })
    assert.equal(actual, "first\nGrüße 世界\n", "physical typing must preserve line breaks")
  }
  await evaluate("document.querySelector('#input').focus();true")
  await exec(["xdotool", "keydown", "Shift_L"])
  const timeoutResult = await exec(["timeout", "--foreground", "--kill-after=2s", "1.5s", "/opt/chariox-selkies/bin/python", `${root}/slice-keyboard.py`], "x".repeat(400)).then(
    () => ({ failed: false }),
    (error) => ({ failed: true, error: error.message }),
  )
  report.timeoutOutcome = timeoutResult
  assert.ok(timeoutResult.failed && timeoutResult.error.includes("124"), "watchdog must terminate active typing")
  const shiftRestored = (await exec(["/opt/chariox-selkies/bin/python", "-c", "from selkies.Xlib import display,XK; d=display.Display(); code=d.keysym_to_keycode(XK.string_to_keysym('Shift_L')); print(bool(d.query_keymap()[code//8] & (1<<(code%8)))); d.close()"])).trim() === "True"
  await exec([`${root}/slice-screen.sh`, "computer-input-reset"])
  assert.ok(shiftRestored, "SIGTERM must restore the modifier lifted during typing")
  report.timeoutCleanup = { shiftRestored }
  for (const busy of [false, true]) {
    const values = [
      ...Array.from({ length: 5 }, (_, iteration) => `keyboard-${iteration}-Grüße 世界 áéíóú Ж`),
      Array.from({ length: 96 }, (_, index) => String.fromCodePoint(0x4e00 + index)).join(""),
      "Long text Grüße 世界 ".repeat(18),
    ]
    for (const [iteration, value] of values.entries()) {
      if (interrupted) throw new Error("drill interrupted")
      await evaluate(`window.busy=${busy};document.querySelector('#input').value='';document.querySelector('#input').focus();true`)
      await exec([`${root}/slice-screen.sh`, "computer-type-stdin"], value)
      const result = await evaluate(`(()=>{const actual=Array.from(document.querySelector('#input').value);const expected=Array.from(${JSON.stringify(value)});return {matches:actual.join('')===expected.join(''),actualCount:actual.length,expectedCount:expected.length,mismatchPositions:expected.flatMap((value,index)=>actual[index]===value?[]:[index])}})()`)
      report.cases.push({ busy, iteration, ...result })
      assert.ok(result.matches, `physical Unicode input mismatch: ${JSON.stringify(report.cases.at(-1))}`)
    }
  }
  await evaluate("window.busy=false;document.querySelector('#input').value='';document.querySelector('#input').focus();true")
  let typingSettlement = null
  const typing = exec(["/opt/chariox-selkies/bin/python", "-c", "import os; os.getpgrp()==os.getpid() or os.setsid(); open('/tmp/chariox-keyboard-x11/input-pgid','w').write(str(os.getpid())); os.execv('/tmp/chariox-keyboard-x11/slice-screen.sh',['slice-screen.sh','computer-type-stdin'])"], "x".repeat(1_800)).then(
    () => (typingSettlement = { failed: false }),
    (error) => (typingSettlement = { failed: true, error: error.message }),
  )
  let beforeCancel = 0
  for (let attempt = 0; attempt < 40 && beforeCancel === 0; attempt++) {
    beforeCancel = await evaluate("document.querySelector('#input').value.length")
    if (typingSettlement) break
    if (!beforeCancel) await pause(50)
  }
  assert.ok(beforeCancel > 0 && beforeCancel < 1_800, `cancellation must interrupt active physical typing: ${typingSettlement?.error ?? beforeCancel}`)
  const cancelledAt = Date.now()
  await exec(["/opt/chariox-selkies/bin/python", "-c", "import os,signal; pid=int(open('/tmp/chariox-keyboard-x11/input-pgid').read()); assert os.getpgid(pid)==pid; os.killpg(pid,signal.SIGKILL)"])
  assert.equal((await typing).failed, true, "cancelled helper must not report success")
  await exec([`${root}/slice-screen.sh`, "computer-input-reset"])
  const afterCancel = await evaluate("document.querySelector('#input').value.length")
  await pause(750)
  assert.equal(await evaluate("document.querySelector('#input').value.length"), afterCancel, "physical typing continued after cancellation and input reset")
  report.cancellation = { beforeCancel, afterCancel, observationLatencyMs: Date.now() - cancelledAt }
  await exec(["xdotool", "keydown", "a", "Shift_L"])
  await exec([`${root}/slice-screen.sh`, "computer-input-reset"])
  const heldKeys = Number(await exec(["/opt/chariox-selkies/bin/python", "-c", "from selkies.Xlib import display; d=display.Display(); print(sum(value.bit_count() for value in d.query_keymap())); d.close()"]));
  assert.equal(heldKeys, 0, "input reset must release printable keys as well as modifiers")
  report.heldKeyReset = true
  await resources("after-cases")
} catch (error) {
  failure = error
  report.failure = error.message
} finally {
  const removed = await docker(["rm", "-f", container]).then(() => true, () => false)
  const absent = !(await docker(["ps", "-aq", "--filter", `name=^/${container}$`])).trim()
  report.cleanup = { removed, absent }
  await resources("after-cleanup")
  report.finishedAt = new Date().toISOString()
  await writeFile(path.join(evidence, "report.json"), JSON.stringify(report, null, 2))
  if (!absent) failure ??= new Error("keyboard drill container cleanup failed")
}
console.log(JSON.stringify({ evidence, passed: !failure, cases: report.cases.length }))
if (failure) throw failure
