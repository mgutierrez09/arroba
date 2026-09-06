#!/usr/bin/env node
// Run inside a disposable slice image. The driver must replace the container
// and home volume between seed and restore, keeping only an archived home.
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { copyFile, mkdir, mkdtemp, readFile, readdir, rename, rm, writeFile } from "node:fs/promises";
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
for (const name of ["browser-cdp.mjs", "slice-selkies.py", "selkies_viewers.py", "tint2rc"]) {
  await copyFile(path.join(source, "docker", name), path.join(runtime, name));
}
const env = { ...process.env, CHARIOX_SLICE_ROOT: runtime, CHARIOX_SLICE_VIEWER_BACKEND: "novnc" };
// Exercise both ordinary configuration and an explicitly empty override.
if (phase === "seed") delete env.CHARIOX_SLICE_CHROME_TRUSTED_INSECURE_ORIGINS;
else env.CHARIOX_SLICE_CHROME_TRUSTED_INSECURE_ORIGINS = "";
const legacy = phase === "seed" && process.env.CHARIOX_TEST_LEGACY_BROWSER === "1";
if (legacy) {
  // Reproduce the old launch configuration only in this disposable fixture.
  const bin = path.join(runtime, "bin");
  await mkdir(bin);
  await writeFile(path.join(bin, "chromium"), '#!/bin/sh\nexec /usr/bin/chromium --no-sandbox "$@"\n', { mode: 0o700 });
  env.PATH = `${bin}:${process.env.PATH}`;
}
const screen = async (...args) => exec("bash", [path.join(source, "docker/slice-screen.sh"), ...args], {
  env, timeout: 30_000, maxBuffer: 128 * 1024,
});
const origin = "http://127.0.0.1:4321";
let sessionValid = true;
const server = createServer((request, response) => {
  if (request.url === "/login") {
    response.writeHead(200, { "content-type": "text/html", "set-cookie": [
      "session_auth=fixture-session; Path=/; HttpOnly; SameSite=Lax",
      "persistent_auth=fixture-persistent; Path=/; HttpOnly; SameSite=Lax; Max-Age=86400",
    ] });
    response.end(`<script>(${seedBrowserStorage.toString()})().then(() => location.replace('/'))</script>`);
    return;
  }
  if (request.url === "/worker.js") {
    response.writeHead(200, { "content-type": "application/javascript", "cache-control": "no-store" });
    response.end("self.addEventListener('install', event => event.waitUntil(self.skipWaiting()));");
    return;
  }
  if (request.url === "/auth") {
    const cookies = new Set((request.headers.cookie ?? "").split(/;\s*/));
    response.writeHead(200, { "content-type": "application/json", "cache-control": "no-store" });
    response.end(JSON.stringify({
      session: sessionValid && cookies.has("session_auth=fixture-session"),
      persistent: sessionValid && cookies.has("persistent_auth=fixture-persistent"),
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
  await assertSandbox(connection);
  const session = await openFixture(connection, phase === "seed" ? "/login" : "/");
  await assertStorage(connection, session, phase);
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
  await assertStorage(connection, fallbackSession, `${phase}-fallback`);
  await client.close();
  await screen("stop");
  await screen("start");
  let restarted = await client.ensureConnection();
  await assertSandbox(restarted);
  let restartedSession = await openFixture(restarted, "/");
  await assertStorage(restarted, restartedSession, `${phase}-restart`);
  const displayPid = (await exec("pgrep", ["-x", "Xvfb"])).stdout;
  for (const fault of ["closed", "killed"]) {
    if (fault === "closed") await restarted.send("Browser.close");
    else await exec("pkill", ["-KILL", "-f", "^/usr/lib/chromium/chromium( |$)"]);
    await client.close();
    await waitFor(async () => {
      try { await fetch("http://127.0.0.1:9222/json/version"); return false; }
      catch { return true; }
    });
    // Reopen through the product URL action while keeping the desktop alive.
    const url = `${origin}/cold-fallback-${fault}`;
    await screen("open-url", url);
    restarted = await client.ensureConnection();
    await assertSandbox(restarted);
    const target = await waitFor(async () => {
      const { targetInfos } = await restarted.send("Target.getTargets");
      return targetInfos.find((target) => target.url === url);
    });
    restartedSession = await client.ensureTargetSession(restarted, target.targetId);
    await waitFor(() => evaluate(restarted, restartedSession, "document.readyState === 'complete'"));
    await assertStorage(restarted, restartedSession, `${phase}-cold-fallback-${fault}`);
    assert.equal((await exec("pgrep", ["-x", "Xvfb"])).stdout, displayPid, "browser recovery must not restart the shared desktop");
  }
  if (phase === "restore") {
    // Distinguish an external service revocation from lost browser data.
    sessionValid = false;
    const { cookies } = await restarted.send("Storage.getCookies");
    assert.ok(["session_auth", "persistent_auth"].every((name) => cookies.some((cookie) => cookie.name === name)), "revocation control must keep both stored cookies");
    await assertStorage(restarted, restartedSession, "server-revoked", false);
    // Red-capable control: the same state assertion must reject genuine loss.
    await restarted.send("Storage.clearDataForOrigin", {
      origin, storageTypes: "local_storage,indexeddb,cache_storage,service_workers",
    }, restartedSession);
    await assert.rejects(() => assertStorage(restarted, restartedSession, "cleared-storage-control", false), /authenticated browser state must survive lifecycle changes/);
    console.log("PROFILE_LOSS_NEGATIVE_CONTROL_PASS");
  }
  await client.close();
  await screen("stop");
  await assert.rejects(() => screen("open-url", `${origin}/must-not-start-desktop`),
    (error) => /missing=.*(?:display|xvfb)/.test(error.stdout ?? ""));
  console.log("STOPPED_DESKTOP_REMAINS_STOPPED_PASS");
  console.log(`SLICE_BROWSER_PROFILE_${phase.toUpperCase()}_PASS`);
} finally {
  await client.close().catch(() => {});
  await screen("stop").catch(() => {});
  await new Promise((resolve) => server.close(resolve));
  await rm(runtime, { recursive: true, force: true });
}

async function assertSandbox(connection) {
  const mainProcesses = [];
  for (const pid of (await readdir("/proc")).filter(value => /^\d+$/.test(value))) {
    const args = await readFile(`/proc/${pid}/cmdline`, "utf8").then(value => value.split("\0"), error => {
      if (error.code === "ENOENT" || error.code === "ESRCH") return [];
      throw error;
    });
    if (args[0] === "/usr/lib/chromium/chromium" && !args.some(arg => arg.startsWith("--type="))) mainProcesses.push(args);
  }
  assert.equal(mainProcesses.length, 1, "expected one shared Chromium main process");
  assert.equal(mainProcesses[0].some(arg => arg.startsWith("--unsafely-treat-insecure-origin-as-secure")), false,
    "ordinary browser launch must not weaken insecure-origin restrictions");
  const { targetInfos } = await connection.send("Target.getTargets");
  console.log(JSON.stringify({ phase, check: "sandbox", pageCount: targetInfos.filter((target) => target.type === "page").length }));
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
  assert.deepEqual(isolation, { namespace: !legacy, seccomp: !legacy }, `unexpected renderer sandbox state: ${sandboxText}`);
  await connection.send("Target.closeTarget", { targetId: sandbox.targetId });
}

async function openFixture(connection, suffix) {
  const { targetInfos } = await connection.send("Target.getTargets");
  const page = targetInfos.find((target) => target.type === "page" && target.url.startsWith(origin))
    ?? targetInfos.find((target) => target.type === "page");
  assert.ok(page, "desktop startup must open or restore a page");
  const session = await client.ensureTargetSession(connection, page.targetId);
  await connection.send("Page.navigate", { url: `${origin}${suffix}` }, session);
  await waitFor(async () => evaluate(connection, session, `location.href === ${JSON.stringify(origin + "/")} && document.readyState === 'complete'`));
  return session;
}

async function assertStorage(connection, session, step, authenticated = true) {
  const state = await evaluate(connection, session, `(${readBrowserStorage.toString()})()`);
  // Report booleans only; this pattern must remain safe if extended to real auth.
  console.log(JSON.stringify({ step, state }));
  assert.deepEqual(state, {
    session: authenticated, persistent: authenticated, localStorage: true,
    indexedDb: true, cacheStorage: true, serviceWorker: true,
  }, "authenticated browser state must survive lifecycle changes");
}

async function seedBrowserStorage() {
  localStorage.setItem("profile-marker", "saved");
  await new Promise((resolve, reject) => {
    const request = indexedDB.open("chariox-profile-drill", 1);
    request.onupgradeneeded = () => request.result.createObjectStore("markers");
    request.onerror = () => reject(request.error);
    request.onsuccess = () => {
      const db = request.result;
      const tx = db.transaction("markers", "readwrite");
      tx.objectStore("markers").put("saved", "profile");
      tx.oncomplete = () => { db.close(); resolve(); };
      tx.onabort = () => { db.close(); reject(tx.error); };
    };
  });
  const cache = await caches.open("chariox-profile-drill");
  await cache.put("/cache-marker", new Response("saved"));
  await navigator.serviceWorker.register("/worker.js");
  await navigator.serviceWorker.ready;
}

async function readBrowserStorage() {
  const indexedDb = await new Promise((resolve, reject) => {
    const request = indexedDB.open("chariox-profile-drill");
    request.onerror = () => reject(request.error);
    request.onsuccess = () => {
      const db = request.result;
      if (!db.objectStoreNames.contains("markers")) { db.close(); resolve(false); return; }
      const value = db.transaction("markers").objectStore("markers").get("profile");
      value.onsuccess = () => { db.close(); resolve(value.result === "saved"); };
      value.onerror = () => { db.close(); reject(value.error); };
    };
  });
  const cached = await caches.match("/cache-marker");
  const worker = await navigator.serviceWorker.getRegistration();
  return {
    ...await (await fetch("/auth")).json(),
    localStorage: localStorage.getItem("profile-marker") === "saved",
    indexedDb,
    cacheStorage: Boolean(cached && await cached.text() === "saved"),
    serviceWorker: worker?.active?.state === "activated",
  };
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
