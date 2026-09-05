import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const provisioner = fileURLToPath(new URL("./provision-linux-docker-slice.sh", import.meta.url));
const prefix = "CHARIOX_ROOM_ENVIRONMENT_";

test("runtime startup passes the supplied Room binding across docker exec without inventing one", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-room-binding-"));
  try {
    // Docker is the external boundary. The production provisioner runs intact.
    await writeFile(join(root, "docker"), `#!/usr/bin/env node
const { appendFileSync } = require("node:fs");
const args = process.argv.slice(2);
if (args[0] === "info" || args[0] === "container") process.exit(0);
if (args[0] === "inspect") {
  const format = args[args.indexOf("--format") + 1] ?? "";
  console.log(format.includes("HostConfig.Ulimits") ? "8192:8192" : "true");
  process.exit(0);
}
if (args[0] === "exec" && args.includes("df")) {
  console.log("Filesystem 1M-blocks Used Available Use% Mounted\\nfixture 10000 1 9999 1% /home/slice");
  process.exit(0);
}
if (args[0] === "exec" && args.includes("rm") && args.includes("/tmp/chariox-slice-state/cloud-relay-config.json")) {
  process.exit(0);
}
if (args[0] === "exec" && args.at(-1) === "/opt/chariox-slice/start-runtime.sh") {
  appendFileSync(process.env.CHARIOX_TEST_DOCKER_LOG, JSON.stringify(args) + "\\n");
  process.exit(0);
}
process.exit(0);
`, { mode: 0o700 });
    const log = join(root, "docker.jsonl");
    const environment = Object.fromEntries(Object.entries(process.env).filter(([name]) =>
      !name.startsWith(prefix) && !name.startsWith("CHARIOX_SLICE_")));
    const bindings = [
      { HOME_KERNEL_ID: "home", HOME_PUBLIC_KEY: "public-key-fixture", SESSION_ID: "room-1", SLICE_ID: "slice-1" },
      {},
      { SESSION_ID: "" },
    ];
    for (const binding of bindings) {
      const result = spawnSync("bash", [provisioner, "start-runtime"], {
        encoding: "utf8", timeout: 60_000,
        env: { ...environment, PATH: `${root}:${environment.PATH}`,
          TMPDIR: root, CHARIOX_TEST_DOCKER_LOG: log,
          CHARIOX_SLICE_NAME: "chariox-binding-fixture",
          ...Object.fromEntries(Object.entries(binding).map(([key, value]) => [prefix + key, value])),
        },
      });
      assert.equal(result.status, 0, result.stderr);
    }
    const calls = (await readFile(log, "utf8")).trim().split("\n").map(JSON.parse);
    assert.equal(calls.length, bindings.length);
    for (const [index, args] of calls.entries()) {
      const received = args.filter(value => value.startsWith(prefix));
      assert.deepEqual(received.sort(), Object.entries(bindings[index])
        .map(([key, value]) => `${prefix}${key}=${value}`).sort());
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
