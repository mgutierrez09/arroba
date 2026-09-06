import assert from "node:assert/strict";
import { PassThrough } from "node:stream";
import readline from "node:readline";
import test from "node:test";
import { BrowserControllerStdioServer, handleBrowserControllerRequest } from "./browser-controller.mjs";
import { BrowserCdpClient } from "./browser-controller-cdp.mjs";

const cases = [
  ["browser.tab", { action: "activate" }, "Target.activateTarget", "Target.getTargets"],
  ["browser.tab", { action: "close" }, "Target.closeTarget", "Target.getTargets"],
  ["browser.navigate", { url: "https://example.test/next" }, "Page.navigate", "Page.getFrameTree"],
  ["browser.history", { action: "back" }, "Page.navigateToHistoryEntry", "Page.getNavigationHistory"],
  ["browser.history", { action: "forward" }, "Page.navigateToHistoryEntry", "Page.getNavigationHistory"],
  ["browser.history", { action: "reload" }, "Page.reload", "Page.getFrameTree"],
  ["browser.dialog", { action: "accept", prompt_text: "fixture answer" }, "Page.handleJavaScriptDialog", "Target.getTargets"],
  ["browser.dialog", { action: "dismiss" }, "Page.handleJavaScriptDialog", "Target.getTargets"],
];

for (const method of new Set(cases.map(([method]) => method))) {
  test(`${method} cancelled before entry never opens Chrome`, async () => {
    const cancel = new AbortController(); cancel.abort();
    let connections = 0;
    const browser = new BrowserCdpClient({ connectionFactory: async () => { connections++; throw new Error("must not connect"); } });
    try {
      const result = await handleBrowserControllerRequest({ id: 1, method, params: {} }, { browser, signal: cancel.signal });
      assert.equal(result.ok, false);
      assert.equal(result.error.code, "browser_action_cancelled");
      assert.equal(connections, 0);
    } finally { await browser.close(); }
  });
}

for (const [method, args, mutation, preparation] of cases) {
  for (const boundary of [preparation, mutation]) {
    test(`${method} ${args.action ?? "url"} cancellation at ${boundary} preserves physical outcome`, { timeout: 5_000 }, async (t) => {
      const reached = Promise.withResolvers();
      const resume = Promise.withResolvers();
      let hold = false;
      let mutations = 0;
      let closed = false;
      let document = "document";
      let currentIndex = 1;
      const entries = [0, 1, 2].map((id) => ({ id, url: `https://example.test/${id}` }));
      const chromium = {
        isOpen: () => true, close: async () => {},
        async send(command, params) {
          if (hold && command === boundary) { reached.resolve(); await resume.promise; }
          if (command === mutation) {
            mutations++;
            if (command === "Target.closeTarget") { closed = true; return { success: true }; }
            if (command === "Page.navigateToHistoryEntry") currentIndex = params.entryId;
            if (["Page.navigate", "Page.reload", "Page.navigateToHistoryEntry"].includes(command)) document = `document-${mutations}`;
            return { loaderId: document };
          }
          switch (command) {
            case "Target.getTargets": return { targetInfos: closed ? [] : [{ type: "page", targetId: "page", url: entries[currentIndex].url, title: "Fixture" }] };
            case "Target.attachToTarget": return { sessionId: "cdp" };
            case "Page.getFrameTree": return { frameTree: { frame: { id: "main", loaderId: document, url: entries[currentIndex].url } } };
            case "Page.getNavigationHistory": return { currentIndex, entries };
            case "Runtime.evaluate": return { result: { value: true } };
            case "Target.setDiscoverTargets": case "Target.setAutoAttach": case "Target.detachFromTarget":
            case "Page.enable": case "Page.setLifecycleEventsEnabled": case "Runtime.enable":
            case "Network.enable": case "Inspector.enable": case "Emulation.setDeviceMetricsOverride": return {};
            default: throw new Error(`unexpected command ${command}`);
          }
        },
      };
      const input = new PassThrough(); const output = new PassThrough();
      const lines = readline.createInterface({ input: output });
      const waiters = new Map();
      lines.on("line", (line) => { const response = JSON.parse(line); waiters.get(response.id)?.resolve(response); });
      const browser = new BrowserCdpClient({ connectionFactory: async () => chromium });
      const server = new BrowserControllerStdioServer({ input, output, browser });
      const running = server.run();
      t.after(async () => { hold = false; resume.resolve(); input.end(); await running; await browser.close(); lines.close(); output.end(); });
      const rpc = (id, method, params) => {
        const response = Promise.withResolvers(); waiters.set(id, response);
        const timer = setTimeout(() => response.reject(new Error(`${method} timed out`)), 2000);
        input.write(`${JSON.stringify({ id, method, params })}\n`);
        return response.promise.finally(() => { clearTimeout(timer); waiters.delete(id); });
      };
      const viewport = { css_width: 800, css_height: 600, device_scale_factor: 1, desktop_pixel_width: 800, desktop_pixel_height: 600 };
      await browser.reconcile(viewport);
      hold = true;
      const pending = rpc(1, method, { target_id: "page", document_id: document, ...args });
      void pending.catch(() => {});
      await reached.promise;
      assert.equal((await rpc(2, "browser.cancel", { request_id: 1 })).result.accepted, true);
      hold = false; resume.resolve();
      const result = await pending;
      const dispatched = boundary === mutation;
      assert.equal(result.ok, dispatched);
      if (!dispatched) assert.equal(result.error.code, "browser_action_cancelled");
      assert.equal(mutations, dispatched ? 1 : 0);
      // Restore the fixture page/history after an actual close or navigation.
      // A retry has fresh observed identity, not a pre-navigation reference.
      closed = false; currentIndex = 1;
      await browser.reconcile(viewport);
      assert.equal((await rpc(3, method, { target_id: "page", document_id: document, ...args })).ok, true);
      assert.equal(mutations, dispatched ? 2 : 1);
      assert.equal((await rpc(4, "browser.cancel", { request_id: 1 })).result.accepted, false);
    });
  }
}
