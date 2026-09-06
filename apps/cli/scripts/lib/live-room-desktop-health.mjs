import assert from "node:assert/strict"

// Run inside the disposable slice. Return booleans only, never command lines
// or environment values, which may contain profile data or credentials.
const probe = String.raw`
const fs = require("node:fs");
const read = p => { try { return fs.readFileSync(p, "utf8") } catch { return "" } };
const processes = fs.readdirSync("/proc").filter(p => /^\d+$/.test(p)).map(pid => ({
  pid, args: read("/proc/" + pid + "/cmdline").split("\0").filter(Boolean),
  status: read("/proc/" + pid + "/status"),
})).filter(p => p.args.length && !/^State:\s+Z/m.test(p.status));
const named = name => processes.filter(p => p.args[0].split("/").at(-1) === name);
const chromium = named("chromium");
const browser = chromium.find(p => !p.args.some(a => a.startsWith("--type=")));
const bus = p => read("/proc/" + p.pid + "/environ").split("\0").find(v => v.startsWith("DBUS_SESSION_BUS_ADDRESS="));
const openbox = named("openbox")[0];
const editor = named("mousepad")[0];
(async () => {
// Sandboxed renderer metadata may be unreadable from an ordinary peer process.
// Use Chromium's own sandbox report, in a temporary background target.
const { BrowserCdpClient } = await import("/opt/chariox-slice/browser-controller-cdp.mjs");
const client = new BrowserCdpClient();
let connection, target, sandbox = {};
try {
  connection = await client.ensureConnection();
  target = await connection.send("Target.createTarget", { url: "chrome://sandbox", background: true });
  const session = await client.ensureTargetSession(connection, target.targetId);
  for (let attempt = 0; attempt < 50; attempt++) {
    const result = await connection.send("Runtime.evaluate", { expression: "document.body?.innerText ?? ''", returnByValue: true }, session);
    const text = result.result?.value ?? "";
    if (text.includes("Seccomp")) {
      sandbox = { namespace: /Layer 1 Sandbox\s+Namespace/.test(text),
        suid: /Layer 1 Sandbox\s+SUID/.test(text),
        pid: /PID namespaces\s+Yes/.test(text), network: /Network namespaces\s+Yes/.test(text),
        seccomp: /Seccomp-BPF sandbox\s+Yes/.test(text) };
      break;
    }
    await new Promise(resolve => setTimeout(resolve, 100));
  }
} finally {
  try { if (target) await connection.send("Target.closeTarget", { targetId: target.targetId }); }
  finally { await client.close(); }
}
console.log(JSON.stringify({
  browserRunning: !!browser,
  insecureOriginException: chromium.some(p => p.args.some(a => a.startsWith("--unsafely-treat-insecure-origin-as-secure"))),
  sandboxDisabled: chromium.some(p => p.args.some(a => /^--(?:no-sandbox|disable-(?:namespace|seccomp-filter|setuid)-sandbox)(?:=|$)/.test(a))),
  sandboxedRenderers: (sandbox.namespace || sandbox.suid) && sandbox.pid && sandbox.network && sandbox.seccomp,
  sandbox,
  taskbarRunning: named("tint2").length === 1,
  desktopSessionBus: !!openbox && !!bus(openbox),
  editorSessionBus: !!editor && !!openbox && !!bus(editor) && bus(editor) === bus(openbox),
  editorDefaultSettings: !!editor && !read("/proc/" + editor.pid + "/environ").split("\0").some(v => v.startsWith("GSETTINGS_BACKEND=")),
}));
})().catch(() => { console.error("desktop health probe failed"); process.exitCode = 1; });
`

export async function verifyRoomDesktopHealth(command, { editor = false } = {}) {
  const health = JSON.parse(await command(["node", "-e", probe]))
  assert.equal(health.browserRunning, true, "desktop Chromium is missing")
  assert.equal(health.insecureOriginException, false, "desktop uses an insecure-origin exception")
  assert.equal(health.sandboxDisabled, false, "desktop disables Chromium sandboxing")
  assert.equal(health.sandboxedRenderers, true, "Chromium renderer isolation: " + JSON.stringify(health.sandbox))
  assert.equal(health.taskbarRunning, true, "desktop applications taskbar is missing")
  assert.equal(health.desktopSessionBus, true, "desktop applications lack a session bus")
  if (editor) {
    assert.equal(health.editorDefaultSettings, true, "graphical editor uses a test-only settings backend")
    assert.equal(health.editorSessionBus, true, "graphical editor did not inherit the desktop session bus")
  }
  return health
}
