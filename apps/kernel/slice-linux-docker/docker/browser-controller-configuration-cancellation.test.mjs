import assert from "node:assert/strict";
import { PassThrough } from "node:stream";
import readline from "node:readline";
import test from "node:test";
import { BrowserControllerStdioServer, handleBrowserControllerRequest } from "./browser-controller.mjs";
import { BrowserCdpClient } from "./browser-controller-cdp.mjs";

for (const method of ["browser.downloads.configure", "browser.permission", "browser.upload"]) {
  test(`${method} cancelled before CDP entry returns typed cancellation without opening Chrome`, async () => {
    const cancellation = new AbortController();
    cancellation.abort();
    let connections = 0;
    const browser = new BrowserCdpClient({ connectionFactory: async () => { connections++; throw new Error("must not connect"); } });
    try {
      const result = await handleBrowserControllerRequest({ id: 1, method, params: {} }, { browser, signal: cancellation.signal });
      assert.equal(result.ok, false);
      assert.equal(result.error.code, "browser_action_cancelled");
      assert.equal(connections, 0);
    } finally { await browser.close(); }
  });
}

for (const method of ["browser.downloads.configure", "browser.permission"]) {
  const physicalMethod = method === "browser.permission" ? "Browser.setPermission" : "Browser.setDownloadBehavior";
  for (const boundary of ["Page.getFrameTree", physicalMethod]) {
    test(`${method} cancellation during ${boundary} preserves physical outcome and permits retry`, { timeout: 5_000 }, async (t) => {
      const reached = Promise.withResolvers();
      const resume = Promise.withResolvers();
      let hold = true;
      let mutations = 0;
      const chromium = {
        isOpen: () => true,
        close: async () => {},
        async send(command) {
          if (hold && command === boundary) { reached.resolve(); await resume.promise; }
          if (command === physicalMethod) { mutations++; return {}; }
          switch (command) {
            case "Target.getTargets": return { targetInfos: [{ type: "page", targetId: "page", url: "https://example.test/" }] };
            case "Target.attachToTarget": return { sessionId: "cdp" };
            case "Page.getFrameTree": return { frameTree: { frame: { loaderId: "document" } } };
            case "Target.setDiscoverTargets": case "Target.setAutoAttach": case "Page.enable":
            case "Page.setLifecycleEventsEnabled": case "Runtime.enable": case "Network.enable":
            case "Inspector.enable": return {};
            default: throw new Error(`unexpected command ${command}`);
          }
        },
      };
      const input = new PassThrough();
      const output = new PassThrough();
      const lines = readline.createInterface({ input: output });
      const waiters = new Map();
      lines.on("line", (line) => { const reply = JSON.parse(line); waiters.get(reply.id)?.resolve(reply); });
      const browser = new BrowserCdpClient({ connectionFactory: async () => chromium,
        downloadDirectory: "/downloads", fileSystem: {
          mkdir: async () => {}, realpath: async (value) => value,
          stat: async () => ({ isDirectory: () => true }),
          statfs: async () => ({ bsize: 4096, bavail: 1_000_000 }),
        } });
      const server = new BrowserControllerStdioServer({ input, output, browser });
      const running = server.run();
      t.after(async () => { hold = false; resume.resolve(); input.end(); await running; await browser.close(); lines.close(); output.end(); });
      const rpc = (id, command, params) => {
        const result = Promise.withResolvers(); waiters.set(id, result);
        const deadline = setTimeout(() => result.reject(new Error(`${command} timed out`)), 2000);
        input.write(`${JSON.stringify({ id, method: command, params })}\n`);
        return result.promise.finally(() => { clearTimeout(deadline); waiters.delete(id); });
      };
      const params = { target_id: "page", document_id: "document", permission: "geolocation", setting: "denied" };
      const pending = rpc(1, method, params);
      void pending.catch(() => {});
      await reached.promise;
      assert.equal((await rpc(2, "browser.cancel", { request_id: 1 })).result.accepted, true);
      hold = false; resume.resolve();
      const result = await pending;
      const dispatched = boundary === physicalMethod;
      assert.equal(result.ok, dispatched);
      if (!dispatched) assert.equal(result.error.code, "browser_action_cancelled");
      assert.equal(mutations, dispatched ? 1 : 0);
      assert.equal((await rpc(3, method, params)).ok, true);
      assert.equal(mutations, dispatched ? 2 : 1);
      assert.equal((await rpc(4, "browser.cancel", { request_id: 1 })).result.accepted, false);
    });
  }
}
