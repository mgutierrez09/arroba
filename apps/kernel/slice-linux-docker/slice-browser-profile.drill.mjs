#!/usr/bin/env node
// Run inside a disposable slice image. The driver must replace the container
// and home volume between seed and restore, keeping only an archived home.
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { copyFile, mkdir, mkdtemp, rename, rm } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import path from "node:path";
import { promisify } from "node:util";
import { fileURLToPath } from "node:url";
import { BrowserCdpClient } from "./docker/browser-controller-cdp.mjs";

const exec = promisify(execFile);
const source = path.dirname(fileURLToPath(import.meta.url));
const phase = process.argv[2];
assert.ok(["seed", "restore"].includes(phase), "expected seed or restore");
assert.equal(process.env.CHARIOX_DISPOSABLE_BROWSER_DRILL, "1", "disposable container required");
const runtime = await mkdtemp(path.join(tmpdir(), "chariox-profile-drill-"));
await mkdir(path.join(runtime, "logs"));
await copyFile(path.join(source, "docker/browser-cdp.mjs"), path.join(runtime, "browser-cdp.mjs"));
const env = { ...process.env, CHARIOX_SLICE_ROOT: runtime, CHARIOX_SLICE_VIEWER_BACKEND: "novnc" };
const screen = async (...args) => exec("bash", [path.join(source, "docker/slice-screen.sh"), ...args], {
  env, timeout: 30_000, maxBuffer: 128 * 1024,
});
const origin = "http://127.0.0.1:4321";
const server = createServer((request, response) => {
  if (request.url === "/login") {
    response.writeHead(200, { "content-type": "text/html", "set-cookie": [
      "session_auth=fixture-session; Path=/; HttpOnly; SameSite=Lax",
      "persistent_auth=fixture-persistent; Path=/; HttpOnly; SameSite=Lax; Max-Age=86400",
    ] });
    response.end('<script>localStorage.setItem("profile-marker", "saved");location.replace("/")</script>');
    return;
  }
  if (request.url === "/auth") {
    const cookies = new Set((request.headers.cookie ?? "").split(/;\s*/));
    response.writeHead(200, { "content-type": "application/json", "cache-control": "no-store" });
    response.end(JSON.stringify({
      session: cookies.has("session_auth=fixture-session"),
      persistent: cookies.has("persistent_auth=fixture-persistent"),
    }));
    return;
  }
  response.writeHead(200, { "content-type": "text/html", "cache-control": "no-store" });
  response.end("<title>Chariox browser profile drill</title><main>Profile fixture</main>");
});
await new Promise((resolve, reject) => {
  server.once("error", reject);
  server.listen(4321, "127.0.0.1", resolve);
});
const client = new BrowserCdpClient();
try {
  await screen("start");
  const connection = await client.ensureConnection();
  // A real headed internal page checks Chromium's own reported sandbox layers.
  const sandbox = await connection.send("Target.createTarget", { url: "chrome://sandbox" });
  const sandboxSession = await client.ensureTargetSession(connection, sandbox.targetId);
  const sandboxText = await waitFor(async () => {
    const result = await evaluate(connection, sandboxSession, "document.body?.innerText ?? ''");
    return result.includes("Seccomp") ? result : null;
  });
  const isolation = {
    namespace: /Layer 1 Sandbox\s+Namespace/.test(sandboxText)
      && /PID namespaces\s+Yes/.test(sandboxText)
      && /Network namespaces\s+Yes/.test(sandboxText),
    seccomp: /Seccomp-BPF sandbox\s+Yes/.test(sandboxText),
  };
  console.log(JSON.stringify({ phase, isolation }));
  if (process.env.CHARIOX_TEST_REQUIRE_SANDBOX !== "0") {
    assert.deepEqual(isolation, { namespace: true, seccomp: true }, `renderer sandbox must remain enabled: ${sandboxText}`);
  }
  await connection.send("Target.closeTarget", { targetId: sandbox.targetId });
  const { targetInfos } = await connection.send("Target.getTargets");
  const page = targetInfos.find((target) => target.type === "page" && target.url.startsWith(origin))
    ?? targetInfos.find((target) => target.type === "page");
  assert.ok(page, "desktop startup must open or restore a page");
  const session = await client.ensureTargetSession(connection, page.targetId);
  await connection.send("Page.navigate", { url: `${origin}${phase === "seed" ? "/login" : "/"}` }, session);
  await waitFor(async () => evaluate(connection, session, `location.href === ${JSON.stringify(origin + "/")} && document.readyState === 'complete'`));
  const state = await evaluate(connection, session, `(async () => ({
    ...await (await fetch('/auth')).json(),
    localStorage: localStorage.getItem('profile-marker') === 'saved'
  }))()`);
  // Report booleans only; this pattern must remain safe if extended to real auth.
  console.log(JSON.stringify({ phase, state }));
  assert.deepEqual(state, { session: true, persistent: true, localStorage: true }, "authenticated browser state must survive home archive restoration");
  // Fail only the CDP navigation helper, forcing the production URL-open
  // fallback into this same authenticated browser instead of a fresh profile.
  const cdp = path.join(runtime, "browser-cdp.mjs");
  await rename(cdp, `${cdp}.disabled`);
  try { await screen("open-url", `${origin}/fallback`); }
  finally { await rename(`${cdp}.disabled`, cdp); }
  const fallback = await waitFor(async () => {
    const { targetInfos } = await connection.send("Target.getTargets");
    return targetInfos.find((target) => target.url === `${origin}/fallback`);
  });
  const fallbackSession = await client.ensureTargetSession(connection, fallback.targetId);
  const fallbackAuth = await evaluate(connection, fallbackSession, "fetch('/auth').then(response => response.json())");
  assert.deepEqual(fallbackAuth, { session: true, persistent: true }, "URL-open fallback must retain both authenticated cookies");
  console.log(JSON.stringify({ phase, fallbackAuth }));
  await client.close();
  await screen("stop");
  console.log(`SLICE_BROWSER_PROFILE_${phase.toUpperCase()}_PASS`);
} finally {
  await client.close().catch(() => {});
  await screen("stop").catch(() => {});
  await new Promise((resolve) => server.close(resolve));
  await rm(runtime, { recursive: true, force: true });
}

async function evaluate(connection, session, expression) {
  const response = await connection.send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true }, session);
  assert.equal(response.exceptionDetails, undefined, "fixture script must not throw");
  return response.result?.value;
}

async function waitFor(check) {
  const deadline = Date.now() + 10_000;
  let lastError;
  while (Date.now() < deadline) {
    try {
      const value = await check();
      if (value) return value;
    } catch (error) { lastError = error; }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw lastError ?? new Error("browser fixture condition timed out");
}
