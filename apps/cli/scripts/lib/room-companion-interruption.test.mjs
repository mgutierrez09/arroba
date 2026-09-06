import assert from "node:assert/strict"
import { EventEmitter } from "node:events"
import { spawn } from "node:child_process"
import { access, mkdtemp, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import { createDrillInterruption } from "./drill-interruption.mjs"
import { runRoomEnvironmentCompanion } from "./live-room-environment-companion-verifier.mjs"

for (const signal of ["SIGINT", "SIGTERM"]) {
test(`${signal} during a waiting Web companion promptly enters protected cleanup`, async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-companion-interruption-"))
  const signals = new EventEmitter()
  const interruption = createDrillInterruption(signals)
  let failure
  let cleaned = false
  let interruptedAt
  const work = interruption.run(() => runRoomEnvironmentCompanion({
    env: { CHARIOX_ROOM_DRILL_COORDINATION_DIR: root, CHARIOX_ROOM_DRILL_COMPANION_TIMEOUT_MS: "1000" },
    ready: { sessionId: "session-1", environmentId: "environment-1" },
    sleep: (ms) => interruption.sleep(ms),
  }), async () => {
    signals.emit("SIGTERM")
    await interruption.sleep(1)
    await rm(root, { recursive: true, force: true })
    cleaned = true
  }, error => { failure = error })
  try {
    const deadline = Date.now() + 1000
    while (true) {
      try { await access(path.join(root, "ready.json")); break } catch {
        assert.ok(Date.now() < deadline, "companion should publish readiness")
        await new Promise(resolve => setTimeout(resolve, 5))
      }
    }
    interruptedAt = Date.now()
    signals.emit(signal)
    await work
    assert.equal(failure?.message, `drill interrupted by ${signal}`)
    assert.ok(Date.now() - interruptedAt < 500, "cleanup must not wait for the companion timeout")
    assert.equal(cleaned, true)
    assert.equal(signals.listenerCount("SIGINT"), 0)
    assert.equal(signals.listenerCount("SIGTERM"), 0)
    await assert.rejects(access(root))
  } finally {
    await work
    await rm(root, { recursive: true, force: true })
  }
})

test(`real waiting companion cleans up on ${signal} without waiting for its deadline`, async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-companion-process-"))
  const child = spawn(process.execPath, ["--input-type=module", "-e", `
    import { rm } from "node:fs/promises";
    import { createDrillInterruption } from ${JSON.stringify(new URL("./drill-interruption.mjs", import.meta.url).href)};
    import { runRoomEnvironmentCompanion } from ${JSON.stringify(new URL("./live-room-environment-companion-verifier.mjs", import.meta.url).href)};
    const guard = createDrillInterruption();
    let announced = false;
    await guard.run(() => runRoomEnvironmentCompanion({
      env: { CHARIOX_ROOM_DRILL_COORDINATION_DIR: ${JSON.stringify(root)}, CHARIOX_ROOM_DRILL_COMPANION_TIMEOUT_MS: "29400000" },
      ready: { sessionId: "session-1", environmentId: "environment-1" },
      sleep: ms => { if (!announced) { announced = true; process.send("waiting"); } return guard.sleep(ms); },
    }), async () => {
      process.send("cleanup");
      await guard.sleep(30);
      await rm(${JSON.stringify(root)}, { recursive: true, force: true });
      process.send("cleaned");
    }, error => { process.send(error.message); process.exitCode = 1; });
    process.disconnect();
  `], { stdio: ["ignore", "ignore", "pipe", "ipc"] })
  const messages = []
  let stderr = ""
  child.stderr.on("data", data => { stderr = (stderr + data).slice(-2000) })
  child.on("message", message => {
    messages.push(message)
    if (message === "waiting") child.kill(signal)
    if (message === "cleanup") { child.kill("SIGINT"); child.kill("SIGTERM") }
  })
  const exit = new Promise((resolve, reject) => {
    child.once("error", reject)
    child.once("exit", (code, signal) => resolve({ code, signal }))
  })
  const watchdog = setTimeout(() => child.kill("SIGKILL"), 3000)
  try {
    assert.deepEqual(await exit, { code: 1, signal: null })
    assert.equal(stderr, "")
    assert.deepEqual(messages, ["waiting", `drill interrupted by ${signal}`, "cleanup", "cleaned"])
    await assert.rejects(access(root))
  } finally {
    clearTimeout(watchdog)
    if (child.exitCode === null && child.signalCode === null) { child.kill("SIGKILL"); await exit }
    await rm(root, { recursive: true, force: true })
  }
})
}
