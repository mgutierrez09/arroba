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
