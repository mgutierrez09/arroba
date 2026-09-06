#!/usr/bin/env node
// No builds or downloads. Supply an existing slice-compatible image and Docker
// endpoint. Only this run's uniquely named containers, volume and temp archive
// are created/deleted. Source must be visible to the Docker daemon for mounting.
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { randomUUID } from "node:crypto";

const exec = promisify(execFile);
const source = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(source, "../../..");
const image = process.env.CHARIOX_BROWSER_PROFILE_IMAGE;
const legacy = process.env.CHARIOX_BROWSER_PROFILE_LEGACY_SEED === "1";
assert.ok(image, "set CHARIOX_BROWSER_PROFILE_IMAGE to an existing slice-compatible image");
const id = `chariox-browser-profile-${randomUUID().slice(0, 8)}`;
const volume = `${id}-home`;
const root = await mkdtemp(path.join(tmpdir(), "chariox-browser-profile-"));
const archive = path.join(root, "home.tar.zst");
const docker = async (...args) => {
  const result = await exec("docker", args, { timeout: 120_000, maxBuffer: 256 * 1024 });
  return result.stdout.trim();
};
// Match the production task cap: Linux counts renderer threads as well as
// processes here. Memory and CPU remain independently capped for this drill.
const limits = ["--memory", "768m", "--memory-swap", "768m", "--cpus", "1", "--pids-limit", "1024", "--ulimit", "core=0"];
try {
  await docker("image", "inspect", image);
  await create(legacy);
  console.log(await docker("exec", id, "node", "/src/apps/kernel/slice-linux-docker/slice-browser-profile.drill.mjs", "seed"));
  await docker("exec", "--user", "0", id, "tar", "--zstd", "-C", "/home/slice", "-cf", "/tmp/home.tar.zst", ".");
  await docker("cp", `${id}:/tmp/home.tar.zst`, archive);
  await docker("rm", "-f", id);
  await docker("volume", "rm", volume);
  console.log("Removed original container and home volume; restoring only the archive");
  await create();
  await docker("cp", archive, `${id}:/tmp/home.tar.zst`);
  await docker("exec", "--user", "0", id, "tar", "--zstd", "-C", "/home/slice", "-xf", "/tmp/home.tar.zst");
  await docker("exec", "--user", "0", id, "chown", "-R", "1000:1000", "/home/slice");
  console.log(await docker("exec", id, "node", "/src/apps/kernel/slice-linux-docker/slice-browser-profile.drill.mjs", "restore"));
  console.log("SLICE_BROWSER_PROFILE_ROUNDTRIP_PASS");
} catch (error) {
  const memoryEvents = await docker("exec", id, "cat", "/sys/fs/cgroup/memory.events").catch(() => "unavailable");
  const pidsEvents = await docker("exec", id, "cat", "/sys/fs/cgroup/pids.events").catch(() => "unavailable");
  const usage = await docker("stats", "--no-stream", "--format", "{{.MemUsage}}", id).catch(() => "unavailable");
  console.error(JSON.stringify({ check: "failed-drill-resources", memoryEvents, pidsEvents, usage }));
  throw error;
} finally {
  // Docker removal is idempotent here, but cleanup errors must fail the drill.
  const failures = [];
  for (const args of [["rm", "-f", id], ["rm", "-f", `${id}-init`], ["volume", "rm", volume]]) {
    try { await docker(...args); } catch (error) {
      if (!/No such container|No such volume/.test(error.stderr ?? "")) failures.push(error);
    }
  }
  await rm(root, { recursive: true, force: true });
  if (failures.length) throw new AggregateError(failures, "browser profile drill cleanup failed");
}

async function create(legacySeed = false) {
  await docker("volume", "create", volume);
  await docker("run", "--rm", "--name", `${id}-init`, ...limits, "--user", "0",
    "--mount", `type=volume,src=${volume},dst=/home/slice`, image, "chown", "1000:1000", "/home/slice");
  await docker("run", "-d", "--init", "--name", id, ...limits, "--user", "1000:1000",
    ...(legacySeed ? [] : ["--security-opt", `seccomp=${path.join(source, "chromium-seccomp.json")}`]),
    "--mount", `type=bind,src=${repo},dst=/src,readonly`,
    "--mount", `type=volume,src=${volume},dst=/home/slice`,
    "-e", "HOME=/home/slice", "-e", "CHARIOX_DISPOSABLE_BROWSER_DRILL=1",
    ...(legacySeed ? ["-e", "CHARIOX_TEST_LEGACY_BROWSER=1"] : []),
    image, "sleep", "infinity");
}
