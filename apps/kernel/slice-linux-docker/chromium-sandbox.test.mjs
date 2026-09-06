import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import test from "node:test";

test("Chromium container profile changes only the three namespace syscall rules", async () => {
  const profile = JSON.parse(await readFile(new URL("./chromium-seccomp.json", import.meta.url), "utf8"));
  const namespaces = profile.syscalls.pop();
  assert.deepEqual(namespaces.names, ["clone", "setns", "unshare"]);
  assert.equal(namespaces.action, "SCMP_ACT_ALLOW");
  assert.equal(profile.defaultAction, "SCMP_ACT_ERRNO");
  // Canonical JSON of upstream moby/profiles at the documented revision. This
  // guards every other rule, including blocked syscalls and capability checks.
  assert.equal(createHash("sha256").update(JSON.stringify(profile)).digest("hex"),
    "afb4934b023cfceaaec1a9d752ca3f801aaa96eb2e59abe6e7ea16976948e080");
});

test("headed, URL fallback and smoke launch paths never disable the renderer sandbox", async () => {
  for (const file of ["./docker/slice-screen.sh", "./provision-linux-docker-slice.sh"]) {
    const source = await readFile(new URL(file, import.meta.url), "utf8");
    assert.doesNotMatch(source, /--(?:no-sandbox|disable-setuid-sandbox|disable-seccomp-filter-sandbox)/);
  }
});

test("desktop startup and URL fallback share one Chromium launch configuration", async () => {
  const source = await readFile(new URL("./docker/slice-screen.sh", import.meta.url), "utf8");
  assert.equal((source.match(/nohup chromium/g) ?? []).length, 1);
  assert.match(source, /launch_chromium\n/);
  assert.match(source, /launch_chromium "\$1"/);
  assert.match(source, /chrome_startup_target_args\+=\(--new-window -- "\$@"\)/);
  assert.match(source, /chrome_startup_target_args=\(-- "\$CHROME_URL"\)/);
});

test("browser recovery drill uses the production task cap without extra swap", async () => {
  const provisioner = await readFile(new URL("./provision-linux-docker-slice.sh", import.meta.url), "utf8");
  const drill = await readFile(new URL("./live-browser-profile-drill.mjs", import.meta.url), "utf8");
  assert.equal(drill.match(/"--pids-limit", "(\d+)"/)[1], provisioner.match(/CHARIOX_SLICE_DOCKER_PIDS_LIMIT:-(\d+)/)[1]);
  assert.equal(drill.match(/"--memory", "([^"]+)"/)[1], drill.match(/"--memory-swap", "([^"]+)"/)[1]);
});
