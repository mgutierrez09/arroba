import assert from "node:assert/strict"
import { mkdtemp, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import {
  assertBrowserComputerEvidencePath,
  assertBrowserComputerPreflight,
  collectBrowserComputerResourceSnapshot,
  defaultBrowserComputerEvidenceDir,
  evaluateBrowserComputerCleanup,
  evaluateBrowserComputerPreflight,
  parseBrowserComputerByteBudget,
} from "./browser-computer-drill-guard.mjs"

test("browser/computer evidence defaults outside repositories", () => {
  assert.equal(
    defaultBrowserComputerEvidenceDir("run-1", "/Users/tester"),
    "/Users/tester/.codex/evidence/browser-computer-use/m0/run-1",
  )
  assert.equal(
    assertBrowserComputerEvidencePath("/Users/tester/.codex/evidence/run-1", ["/work/chariox"]),
    "/Users/tester/.codex/evidence/run-1",
  )
  assert.throws(
    () => assertBrowserComputerEvidencePath("/work/chariox/.artifacts/run-1", ["/work/chariox"]),
    /must stay outside repositories/,
  )
  assert.throws(() => defaultBrowserComputerEvidenceDir(""), /run id is required/)
})

test("resource collection uses Docker inventory and macOS available pages", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-browser-resource-test-"))
  const commands = []
  try {
    const result = await collectBrowserComputerResourceSnapshot({
      filesystemPath: rootDir,
      platform: "darwin",
      runCommand: async (command, args) => {
        commands.push([command, ...args])
        if (command === "vm_stat") {
          return {
            code: 0,
            stdout: [
              "Mach Virtual Memory Statistics: (page size of 16384 bytes)",
              "Pages free: 10.",
              "Pages inactive: 20.",
              "Pages speculative: 5.",
              "Pages purgeable: 2.",
            ].join("\n"),
            stderr: "",
          }
        }
        if (args[0] === "ps") return { code: 0, stdout: "chariox-slice-existing\nunrelated\n", stderr: "" }
        if (args[0] === "volume") return { code: 0, stdout: "chariox-slice-existing-home\n", stderr: "" }
        throw new Error(`unexpected command: ${command} ${args.join(" ")}`)
      },
    })

    assert.equal(result.memory.availableBytes, 37 * 16_384)
    assert.deepEqual(result.docker.containers, ["chariox-slice-existing", "unrelated"])
    assert.deepEqual(result.docker.volumes, ["chariox-slice-existing-home"])
    assert.deepEqual(commands.map((command) => command.slice(0, 2)), [
      ["docker", "ps"],
      ["docker", "volume"],
      ["vm_stat"],
    ])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("resource preflight compares the next operation's byte budget and limits concurrency", () => {
  const result = evaluateBrowserComputerPreflight(snapshot({
    memoryAvailableBytes: 19,
    diskAvailableBytes: 14,
    containers: ["chariox-slice-existing"],
  }), { requiredMemoryBytes: 20, requiredDiskBytes: 15 })

  assert.equal(result.ok, false)
  assert.deepEqual(result.existingSliceContainers, ["chariox-slice-existing"])
  assert.match(result.violations.join("\n"), /available memory 19 bytes.*20 bytes/)
  assert.match(result.violations.join("\n"), /available disk 14 bytes.*15 bytes/)
  assert.match(result.violations.join("\n"), /single-slice developer run unsafe/)
  assert.throws(
    () => assertBrowserComputerPreflight(snapshot({ memoryAvailableBytes: 19 }), { requiredMemoryBytes: 20 }),
    /resource preflight failed/,
  )
})

test("resource preflight accepts an exact-fit byte budget and explicit concurrency", () => {
  const result = evaluateBrowserComputerPreflight(snapshot({
    memoryAvailableBytes: 20,
    diskAvailableBytes: 15,
    containers: ["chariox-slice-explicit-concurrency"],
  }), { allowExistingHeadedSlices: true, requiredMemoryBytes: 20, requiredDiskBytes: 15 })

  assert.equal(result.ok, true)
  assert.equal(result.memoryHeadroom, 0.2)
  assert.equal(result.diskHeadroom, 0.15)
})

test("low free percentages alone do not prevent safe work", () => {
  const gib = 1024 ** 3
  const resources = {
    memory: { totalBytes: 16 * gib, availableBytes: 2 * gib },
    disk: { totalBytes: 1024 * gib, availableBytes: 52 * gib },
    docker: { containers: [], volumes: [] },
  }
  assert.equal(evaluateBrowserComputerPreflight(resources, {
    requiredMemoryBytes: gib, requiredDiskBytes: 8 * gib,
  }).ok, true)
  assert.equal(evaluateBrowserComputerPreflight(resources).ok, true)
  assert.equal(evaluateBrowserComputerPreflight(snapshot({ memoryAvailableBytes: 90 }), {
    requiredMemoryBytes: 91,
  }).ok, false, "a high free percentage cannot excuse an operation that does not fit")
})

test("resource byte budgets must be valid and actual exhaustion still fails", () => {
  for (const invalid of [-1, NaN, Infinity, 1.5]) {
    assert.throws(() => evaluateBrowserComputerPreflight(snapshot(), {
      requiredMemoryBytes: invalid,
    }), /non-negative.*integer/)
    assert.throws(() => evaluateBrowserComputerPreflight(snapshot(), {
      requiredDiskBytes: invalid,
    }), /non-negative.*integer/)
  }
  assert.equal(evaluateBrowserComputerPreflight(snapshot({ diskAvailableBytes: 0 })).ok, false)
  assert.equal(evaluateBrowserComputerPreflight(snapshot({ memoryAvailableBytes: 0 })).ok, false)
})

test("blank environment budgets preserve the missing-budget warning", () => {
  for (const value of [undefined, "", " ", "\t\n"]) {
    const budget = parseBrowserComputerByteBudget(value)
    assert.equal(budget, undefined)
    const result = evaluateBrowserComputerPreflight(snapshot(), {
      requiredMemoryBytes: budget,
      requiredDiskBytes: budget,
    })
    assert.equal(result.ok, true)
    assert.match(result.warnings.join("\n"), /budget not fully specified/)
  }
  for (const [value, expected] of [["0", 0], [" 20 ", 20]]) {
    const budget = parseBrowserComputerByteBudget(value)
    assert.equal(budget, expected)
    assert.deepEqual(evaluateBrowserComputerPreflight(snapshot(), {
      requiredMemoryBytes: budget,
      requiredDiskBytes: budget,
    }).warnings, [])
  }
  for (const value of ["-1", "1.5", "NaN", "Infinity", "not-a-budget"]) {
    assert.throws(() => evaluateBrowserComputerPreflight(snapshot(), {
      requiredMemoryBytes: parseBrowserComputerByteBudget(value),
    }), /non-negative.*integer/)
  }
})

test("cleanup rejects owned and newly-created slice resources", async () => {
  const tempRoot = await mkdtemp(path.join(os.tmpdir(), "chariox-browser-cleanup-test-"))
  try {
    const result = await evaluateBrowserComputerCleanup({
      before: snapshot({ containers: ["chariox-slice-unrelated"], volumes: ["chariox-slice-unrelated-home"] }),
      after: snapshot({
        memoryAvailableBytes: 90,
        diskAvailableBytes: 80,
        containers: ["chariox-slice-unrelated", "chariox-slice-owned", "chariox-slice-leaked"],
        volumes: ["chariox-slice-unrelated-home", "chariox-slice-owned-home"],
      }),
      ownedContainers: ["chariox-slice-owned"],
      ownedVolumes: ["chariox-slice-owned-home"],
      tempRoots: [tempRoot],
      childProcesses: [
        { drillLabel: "kernel", exitCode: null, signalCode: null },
        { drillLabel: "fixture", exitCode: 0, signalCode: null },
      ],
    })

    assert.equal(result.ok, false)
    assert.match(result.violations.join("\n"), /owned container remains: chariox-slice-owned/)
    assert.match(result.violations.join("\n"), /new slice container remains: chariox-slice-leaked/)
    assert.match(result.violations.join("\n"), /owned volume remains: chariox-slice-owned-home/)
    assert.match(result.violations.join("\n"), /temporary root remains/)
    assert.match(result.violations.join("\n"), /child process remains alive: kernel/)
    assert.doesNotMatch(result.violations.join("\n"), /new slice container remains: chariox-slice-owned/)
    assert.doesNotMatch(result.violations.join("\n"), /new slice volume remains: chariox-slice-owned-home/)
    assert.doesNotMatch(result.violations.join("\n"), /fixture/)
  } finally {
    await rm(tempRoot, { recursive: true, force: true })
  }
})

test("cleanup permits deliberately retained resources but still rejects live children", async () => {
  const tempRoot = await mkdtemp(path.join(os.tmpdir(), "chariox-browser-cleanup-retained-"))
  try {
    const result = await evaluateBrowserComputerCleanup({
      before: snapshot(),
      after: snapshot({
        containers: ["chariox-slice-owned", "chariox-slice-debug"],
        volumes: ["chariox-slice-owned-home", "chariox-slice-debug-home"],
      }),
      ownedContainers: ["chariox-slice-owned"],
      ownedVolumes: ["chariox-slice-owned-home"],
      tempRoots: [tempRoot],
      childProcesses: [{ drillLabel: "kernel", exitCode: null, signalCode: null }],
      allowRetainedResources: true,
    })

    assert.deepEqual(result.violations, ["child process remains alive: kernel"])
  } finally {
    await rm(tempRoot, { recursive: true, force: true })
  }
})

test("cleanup accepts restored inventory and records resource deltas", async () => {
  const removedRoot = await mkdtemp(path.join(os.tmpdir(), "chariox-browser-cleanup-pass-"))
  await rm(removedRoot, { recursive: true, force: true })
  const before = snapshot({ memoryAvailableBytes: 70, diskAvailableBytes: 60 })
  const after = snapshot({ memoryAvailableBytes: 75, diskAvailableBytes: 58 })
  const result = await evaluateBrowserComputerCleanup({
    before,
    after,
    ownedContainers: ["chariox-slice-owned"],
    ownedVolumes: ["chariox-slice-owned-home"],
    tempRoots: [removedRoot],
  })

  assert.equal(result.ok, true)
  assert.deepEqual(result.violations, [])
  assert.equal(result.memoryAvailableDeltaBytes, 5)
  assert.equal(result.diskAvailableDeltaBytes, -2)
})

function snapshot({
  memoryAvailableBytes = 100,
  diskAvailableBytes = 100,
  containers = [],
  volumes = [],
} = {}) {
  return {
    memory: { totalBytes: 100, availableBytes: memoryAvailableBytes },
    disk: { totalBytes: 100, availableBytes: diskAvailableBytes },
    docker: { containers, volumes },
  }
}
