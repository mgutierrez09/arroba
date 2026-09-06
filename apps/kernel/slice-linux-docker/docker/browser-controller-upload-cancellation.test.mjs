import assert from "node:assert/strict";
import { PassThrough } from "node:stream";
import readline from "node:readline";
import test from "node:test";
import { BrowserControllerStdioServer } from "./browser-controller.mjs";
import { BrowserCdpClient } from "./browser-controller-cdp.mjs";

for (const boundary of ["filesystem", "DOM.resolveNode", "Runtime.callFunctionOn", "DOM.setFileInputFiles"]) {
test(`stdio upload cancellation during ${boundary} reports the physical outcome and permits retry`, { timeout: 5_000 }, async (t) => {
  const reached = Promise.withResolvers();
  const resume = Promise.withResolvers();
  let hold = true;
  let uploads = 0;
  let releases = 0;
  async function pause(at) {
    if (hold && at === boundary) { reached.resolve(); await resume.promise; }
  }
  const chromium = {
    isOpen: () => true,
    close: async () => {},
    async send(method) {
      await pause(method);
      switch (method) {
        case "Target.getTargets": return { targetInfos: [{ type: "page", targetId: "page" }] };
        case "Target.attachToTarget": return { sessionId: "cdp" };
        case "Page.getFrameTree": return { frameTree: { frame: { loaderId: "document" } } };
        case "DOM.resolveNode": return { object: { objectId: "input" } };
        case "Runtime.callFunctionOn": return { result: { value: "file" } };
        case "DOM.setFileInputFiles": uploads++; return {};
        case "Runtime.releaseObject": releases++; return {};
        case "Target.setDiscoverTargets": case "Target.setAutoAttach": case "Page.enable":
        case "Page.setLifecycleEventsEnabled": case "Runtime.enable": case "Network.enable":
        case "Inspector.enable": return {};
        default: throw new Error(`unexpected command ${method}`);
      }
    },
  };
  const input = new PassThrough();
  const output = new PassThrough();
  const lines = readline.createInterface({ input: output });
  const waiters = new Map();
  lines.on("line", (line) => { const response = JSON.parse(line); waiters.get(response.id)?.resolve(response); });
  const browser = new BrowserCdpClient({ connectionFactory: async () => chromium,
    uploadRoots: ["/uploads"], fileSystem: {
      realpath: async (value) => value,
      stat: async (value) => { if (value.endsWith("report.txt")) await pause("filesystem");
        return { size: 12, isDirectory: () => value === "/uploads", isFile: () => value.endsWith("report.txt") }; },
    } });
  const server = new BrowserControllerStdioServer({ input, output, browser });
  const running = server.run();
  t.after(async () => { hold = false; resume.resolve(); input.end(); await running; await browser.close(); lines.close(); output.end(); });
  const rpc = (id, method, params) => {
    const result = Promise.withResolvers(); waiters.set(id, result);
    const deadline = setTimeout(() => result.reject(new Error(`${method} timed out`)), 2000);
    input.write(`${JSON.stringify({ id, method, params })}\n`);
    return result.promise.finally(() => { clearTimeout(deadline); waiters.delete(id); });
  };
  const params = { target_id: "page", document_id: "document", node_ref: "backend:1", file_paths: ["/uploads/report.txt"] };
  const pending = rpc(1, "browser.upload", params);
  void pending.catch(() => {});
  await reached.promise;
  assert.equal((await rpc(2, "browser.cancel", { request_id: 1 })).result.accepted, true);
  hold = false; resume.resolve();
  const cancelled = await pending;
  const alreadyDispatched = boundary === "DOM.setFileInputFiles";
  // A cancel acknowledgement is not proof of rollback after dispatch.
  assert.equal(cancelled.ok, alreadyDispatched);
  if (!alreadyDispatched) assert.equal(cancelled.error.code, "browser_action_cancelled");
  assert.equal(uploads, alreadyDispatched ? 1 : 0);
  assert.equal(releases, boundary === "filesystem" ? 0 : 1);
  assert.equal((await rpc(3, "browser.upload", params)).ok, true);
  assert.equal(uploads, alreadyDispatched ? 2 : 1);
  assert.equal((await rpc(4, "browser.cancel", { request_id: 1 })).result.accepted, false);
});
}
