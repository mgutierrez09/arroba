import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"

const provisioner = fileURLToPath(new URL("./provision-linux-docker-slice.sh", import.meta.url))

test("new slices have a finite process limit by default", async () => {
  const { calls } = await invoke()
  const create = calls.find((args) => args[0] === "create")
  assert.ok(create, "fixture must reach container creation")
  assert.equal(create[create.indexOf("--pids-limit") + 1], "1024")
  assert.equal(create[create.indexOf("--ulimit", create.indexOf("--ulimit") + 1) + 1], "nofile=8192:8192")
  assert.equal(create[create.indexOf("--security-opt") + 1], `seccomp=${fileURLToPath(new URL("./chromium-seccomp.json", import.meta.url))}`)
  assert.equal(create.includes("--cap-add"), false, "normal Chromium slices need no extra capabilities")
})

test("invalid process limits fail before any Docker operation", async () => {
  for (const limit of ["0", "-1", "1.5", "abc", "2147483648", "9999999999999999999999999"]) {
    const { result, calls } = await invoke({ limit })
    assert.notEqual(result.status, 0)
    assert.match(result.stderr, /CHARIOX_SLICE_DOCKER_PIDS_LIMIT/)
    assert.deepEqual(calls, [], `Docker was called for invalid limit ${limit}`)
  }
})

test("invalid file-descriptor limits fail before any Docker operation", async () => {
  for (const nofileLimit of ["0", "1023", "-1", "1.5", "abc", "1048577", "9999999999999999999999999"]) {
    const { result, calls } = await invoke({ nofileLimit })
    assert.notEqual(result.status, 0)
    assert.match(result.stderr, /CHARIOX_SLICE_DOCKER_NOFILE_LIMIT/)
    assert.deepEqual(calls, [], `Docker was called for invalid nofile limit ${nofileLimit}`)
  }
})

test("reusing a slice applies the configured process cap before starting it", async () => {
  const { calls } = await invoke({ existing: true, limit: "768" })
  const update = calls.findIndex((args) => args[0] === "update")
  assert.ok(update >= 0, "existing slice must have its cap reconciled")
  assert.deepEqual(calls[update], ["update", "--pids-limit", "768", "chariox-process-limit-fixture"])
  assert.ok(calls.findIndex((args) => args[0] === "start") > update)
  assert.equal(calls.some((args) => args[0] === "create"), false)
})

test("reusing a slice verifies its immutable file-descriptor cap before startup", async () => {
  const { calls } = await invoke({ existing: true, nofileLimit: "4096", existingNofile: "4096:4096" })
  const inspect = calls.findIndex((args) => args[0] === "inspect" && args.join(" ").includes("HostConfig.Ulimits"))
  assert.ok(inspect >= 0, "existing slice must have its nofile cap inspected")
  assert.ok(calls.findIndex((args) => args[0] === "start") > inspect)
})

test("a missing or divergent file-descriptor cap cannot start or execute inside the slice", async () => {
  for (const [action, existingNofile] of [["provision", ""], ["recover", "16384:16384"], ["import-provider-auth", "4096:8192"]]) {
    const { result, calls } = await invoke({ existing: true, action, existingNofile })
    assert.notEqual(result.status, 0)
    assert.match(result.stderr, /file-descriptor limit.*recreate/i)
    assert.equal(calls.some((args) => ["start", "exec"].includes(args[0])), false)
  }
})

test("failed-save recovery cannot restart a slice without its process cap", async () => {
  const { calls } = await invoke({ existing: true, action: "recover", limit: "512" })
  const update = calls.findIndex((args) => args[0] === "update")
  assert.ok(update >= 0)
  assert.equal(calls[update][2], "512")
  assert.ok(calls.findIndex((args) => args[0] === "start") > update)
})

test("authentication setup caps an existing slice before starting it", async () => {
  const { calls } = await invoke({ existing: true, action: "import-provider-auth", limit: "512" })
  const update = calls.findIndex((args) => args[0] === "update")
  assert.ok(update >= 0)
  assert.ok(calls.findIndex((args) => args[0] === "start") > update)
})

test("failure to apply a cap cannot start or execute inside the slice", async () => {
  for (const action of ["provision", "recover", "import-provider-auth"]) {
    const { result, calls } = await invoke({ existing: true, updateFails: true, action })
    assert.notEqual(result.status, 0)
    assert.match(result.stderr, /failed to apply slice process limit/)
    assert.equal(calls.some((args) => ["start", "exec"].includes(args[0])), false)
  }
})

test("invalid caps never prevent stopping or destroying a running slice", async () => {
  for (const action of ["stop", "destroy"]) {
    const { result, calls } = await invoke({ limit: "0", nofileLimit: "0", existing: true, running: true, action })
    assert.equal(result.status, 0)
    assert.ok(calls.some((args) => args[0] === "stop" && args[1] === "chariox-process-limit-fixture"))
    if (action === "destroy") assert.ok(calls.some((args) => args[0] === "rm" && args[1] === "chariox-process-limit-fixture"))
    assert.equal(calls.some((args) => args[0] === "update"), false)
  }
})

async function invoke({
  limit,
  nofileLimit,
  existingNofile = "8192:8192",
  existing = false,
  running = false,
  action = "provision",
  updateFails = false,
} = {}) {
  const root = await mkdtemp(join(tmpdir(), "chariox-process-limit-"))
  try {
    const log = join(root, "docker.jsonl")
    await writeFile(join(root, "docker"), `#!/usr/bin/env node
const fs = require("node:fs");
const args = process.argv.slice(2);
fs.appendFileSync(process.env.CHARIOX_TEST_DOCKER_LOG, JSON.stringify(args) + "\\n");
if (args[0] === "info") process.exit(0);
if (args[0] === "image" && args[1] === "inspect") {
  const format = args[args.indexOf("-f") + 1] || "";
  console.log(format.includes("relay-peer") ? process.env.CHARIOX_TEST_PROTOCOL : format.includes("runtime-source") ? process.env.CHARIOX_SLICE_BUILD_CONTEXT_DIGEST : "fixture-image");
  process.exit(0);
}
if (args[0] === "container" && args[1] === "inspect") {
  if (process.env.CHARIOX_TEST_EXISTING !== "1") process.exit(1);
  console.log("fixture-image"); process.exit(0);
}
if (args[0] === "inspect") {
  const format = args[args.indexOf("--format") + 1] || args[args.indexOf("-f") + 1] || "";
  if (format.includes("HostConfig.Ulimits")) console.log(process.env.CHARIOX_TEST_NOFILE);
  else console.log(process.env.CHARIOX_TEST_RUNNING === "1" ? "true" : "false");
  process.exit(0);
}
if (args[0] === "ps") {
  if (process.env.CHARIOX_TEST_EXISTING === "1" && (args.includes("-a") || process.env.CHARIOX_TEST_RUNNING === "1")) console.log("chariox-process-limit-fixture");
  process.exit(0);
}
if (["volume", "create", "start", "stop", "rm"].includes(args[0])) process.exit(0);
if (args[0] === "update") process.exit(process.env.CHARIOX_TEST_UPDATE_FAILS === "1" ? 73 : 0);
// Stop before running setup inside the fixture container.
if (args[0] === "exec") process.exit(71);
throw new Error("unexpected Docker call: " + args[0]);
`, { mode: 0o700 })
    const env = Object.fromEntries(Object.entries(process.env).filter(([name]) => !name.startsWith("CHARIOX_SLICE_")))
    const protocol = (await readFile(new URL("../src/transport/relay_peer.rs", import.meta.url), "utf8")).match(/RELAY_PEER_PROTOCOL_VERSION: u32 = (\d+)/)[1]
    const result = spawnSync("bash", [provisioner, action], {
      encoding: "utf8", timeout: 15_000,
      env: { ...env, PATH: `${root}:${env.PATH}`, TMPDIR: root,
        CHARIOX_TEST_DOCKER_LOG: log, CHARIOX_TEST_PROTOCOL: protocol,
        CHARIOX_TEST_EXISTING: existing ? "1" : "0", CHARIOX_TEST_UPDATE_FAILS: updateFails ? "1" : "0",
        CHARIOX_TEST_RUNNING: running ? "1" : "0", CHARIOX_TEST_NOFILE: existingNofile,
        CHARIOX_SLICE_BUILD_CONTEXT_DIGEST: `sha256:${"a".repeat(64)}`,
        CHARIOX_SLICE_BUILD_IMAGE: "never", CHARIOX_SLICE_NAME: "chariox-process-limit-fixture",
        ...(limit === undefined ? {} : { CHARIOX_SLICE_DOCKER_PIDS_LIMIT: limit }),
        ...(nofileLimit === undefined ? {} : { CHARIOX_SLICE_DOCKER_NOFILE_LIMIT: nofileLimit }),
      },
    })
    assert.equal(result.error, undefined)
    const contents = await readFile(log, "utf8").catch((error) => { if (error.code === "ENOENT") return ""; throw error })
    return { result, calls: contents.trim() ? contents.trim().split("\n").map(JSON.parse) : [] }
  } finally {
    await rm(root, { recursive: true, force: true })
  }
}
