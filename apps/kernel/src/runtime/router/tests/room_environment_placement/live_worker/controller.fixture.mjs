// Run the production Chariox controller. Only external Chromium/CDP responses
// are synthetic; kernel routing, relay encryption and controller stdio are real.
import { appendFileSync, existsSync, readFileSync, unlinkSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";

const [directory, pidFile] = process.argv.slice(2);
const { BrowserControllerStdioServer } = await import(pathToFileURL(join(directory, "browser-controller.mjs")));
const { BrowserCdpClient } = await import(pathToFileURL(join(directory, "browser-controller-cdp.mjs")));
// The production supervisor may fence and restart the controller while the
// external browser remains alive. Keep the current generation convenient for
// assertions and retain every PID so cleanup can prove that none leaked.
writeFileSync(pidFile, String(process.pid));
appendFileSync(`${pidFile}s`, `${process.pid}\n`);
const stateFile = join(dirname(pidFile), "chromium-state.json");
let state = existsSync(stateFile)
  ? JSON.parse(readFileSync(stateFile, "utf8"))
  : { open: true, saved: false, clickCount: 0, pressed: false, note: "", submitted: null, focused: "worker-save" };
state.clickCount ??= 0;
state.url ??= "https://worker.test/";
state.documentId ??= "worker-document";
state.focusedTarget ??= "worker-tab";
state.history ??= [{ id: 1, url: state.url, title: "Worker browser" }];
state.historyIndex ??= state.history.length - 1;
state.documentSequence ??= 1;
const persist = () => writeFileSync(stateFile, JSON.stringify(state));
const subscribers = new Set();
const emit = (message) => {
  for (const subscriber of subscribers) subscriber(message);
};
persist();
const chromium = {
  isOpen: () => state.open,
  subscribe(listener) {
    subscribers.add(listener);
    return () => subscribers.delete(listener);
  },
  close: async () => { state.open = false; persist(); },
  async send(method, params = {}, sessionId) {
    switch (method) {
      case "Target.getTargets": {
        const externalNavigation = join(dirname(pidFile), "external-browser-navigation");
        if (existsSync(externalNavigation)) {
          unlinkSync(externalNavigation);
          state.documentId = `worker-document-${++state.documentSequence}`;
          persist();
          emit({ method: "Page.frameNavigated", sessionId: "worker-cdp-session", params: { frame: {
            id: "worker-frame", loaderId: state.documentId, url: state.url,
          } } });
        }
        // External browser fault: the user can close every page while a
        // browser-owned download continues in the background.
        if (existsSync(join(dirname(pidFile), "close-browser-tabs"))) {
          if (!state.tabsClosed) {
            state.tabsClosed = true;
            for (const targetId of ["worker-tab", ...(state.popup ? ["worker-popup"] : [])]) {
              emit({ method: "Target.targetDestroyed", params: { targetId } });
            }
          }
          return { targetInfos: [] };
        }
        state.tabsClosed = false;
        return { targetInfos: [
        { type: "page", targetId: "worker-tab", url: state.url, title: "Worker browser" },
        ...(state.popup ? [{ type: "page", targetId: "worker-popup", url: "https://popup.worker.test/", title: "Worker popup" }] : []),
        ] };
      }
      case "Target.attachToTarget": return { sessionId: params.targetId === "worker-popup" ? "worker-popup-session" : "worker-cdp-session" };
      case "Page.getFrameTree": return { frameTree: { frame: {
        id: sessionId === "worker-popup-session" ? "worker-popup-frame" : "worker-frame",
        loaderId: sessionId === "worker-popup-session" ? "worker-popup-document" : state.documentId,
        url: sessionId === "worker-popup-session" ? "https://popup.worker.test/" : state.url,
      } } };
      case "Page.navigate": {
        state.navigateCount = (state.navigateCount ?? 0) + 1;
        state.navigationSession = sessionId;
        state.url = params.url;
        state.documentId = `worker-document-${++state.documentSequence}`;
        state.history.splice(state.historyIndex + 1);
        state.history.push({
          id: Math.max(0, ...state.history.map((entry) => entry.id)) + 1,
          url: state.url,
          title: "Worker browser",
        });
        state.historyIndex = state.history.length - 1;
        persist();
        return { frameId: "worker-frame", loaderId: state.documentId };
      }
      case "Page.getNavigationHistory": return {
        currentIndex: state.historyIndex,
        entries: state.history,
      };
      case "Page.navigateToHistoryEntry": {
        const index = state.history.findIndex((entry) => entry.id === params.entryId);
        if (index < 0) throw new Error("unknown worker history entry");
        state.historyIndex = index;
        state.url = state.history[index].url;
        state.documentId = `worker-document-${++state.documentSequence}`;
        persist();
        return {};
      }
      case "Page.reload":
        state.reloadCount = (state.reloadCount ?? 0) + 1;
        state.documentId = `worker-document-${++state.documentSequence}`;
        persist();
        return {};
      case "Runtime.evaluate": return { result: { value:
        state.focusedTarget === (sessionId === "worker-popup-session" ? "worker-popup" : "worker-tab")
      } };
      case "Target.activateTarget":
        state.activateCount = (state.activateCount ?? 0) + 1;
        state.focusedTarget = params.targetId;
        persist();
        return {};
      case "Target.closeTarget":
        if (params.targetId === "worker-popup" && state.popup) {
          state.popup = false;
          state.focusedTarget = "worker-tab";
          persist();
          emit({ method: "Target.targetDestroyed", params: { targetId: "worker-popup" } });
          return { success: true };
        }
        return { success: false };
      case "Accessibility.getFullAXTree": return { nodes: [{
        nodeId: "ax-save", backendDOMNodeId: 103, ignored: false,
        role: { value: "button" }, name: { value: state.submitted === null ? (state.saved ? "Saved on worker" : "Save on worker") : `Submitted: ${state.submitted}` },
        properties: [{ name: "focused", value: { value: state.focused === "worker-save" } }],
      }, {
        nodeId: "ax-note", backendDOMNodeId: 104, ignored: false,
        role: { value: "textbox" }, name: { value: "Worker note" }, value: { value: state.note },
        properties: [{ name: "focused", value: { value: state.focused === "worker-note" } }],
      }] };
      case "DOMSnapshot.captureSnapshot": return {
        strings: ["#document", "BUTTON", "", "Save on worker", "https://worker.test/", "INPUT", "type", "file", "IFRAME", "DIV", "#document-fragment", "open", "https://frame.worker.test/"],
        documents: [{ documentURL: 4, nodes: {
          parentIndex: [-1, 0, 0, 0, 0, 4, 5], nodeType: [9, 1, 1, 1, 1, 11, 1], nodeName: [0, 1, 5, 8, 9, 10, 1],
          nodeValue: [2, 3, 2, 2, 2, 2, 2], backendNodeId: [100, 103, 104, 105, 106, 107, 108], attributes: [[], [], [6, 7], [], [], [], []],
          contentDocumentIndex: { index: [3], value: [1] },
          shadowRootType: { index: [5], value: [11] },
        }, layout: { nodeIndex: [1, 2, 3, 4, 5, 6], bounds: [[10, 20, 100, 30], [10, 60, 100, 30], [10, 100, 200, 80], [230, 100, 200, 80], [230, 100, 200, 80], [240, 110, 100, 30]] } }, {
          documentURL: 12, nodes: {
            parentIndex: [-1, 0], nodeType: [9, 1], nodeName: [0, 1],
            nodeValue: [2, 2], backendNodeId: [200, 201], attributes: [[], []],
          }, layout: { nodeIndex: [1], bounds: [[20, 110, 100, 30]] },
        }],
      };
      case "DOM.resolveNode": {
        if (![103, 104, 108, 201].includes(params.backendNodeId)) throw new Error("wrong worker node");
        const objectId = new Map([
          [103, "worker-save"], [104, "worker-note"],
          [108, "worker-shadow"], [201, "worker-frame"],
        ]).get(params.backendNodeId);
        return { object: { objectId } };
      }
      case "Runtime.callFunctionOn": {
        if (!["worker-save", "worker-note", "worker-shadow", "worker-frame"].includes(params.objectId)) throw new Error("wrong worker object");
        if (params.functionDeclaration.includes('this.localName === "input"')) {
          if (existsSync(join(dirname(pidFile), "hold-upload"))) {
            writeFileSync(join(dirname(pidFile), "upload-pending"), "file input inspection pending");
            while (existsSync(join(dirname(pidFile), "hold-upload"))) {
              await new Promise(resolve => setTimeout(resolve, 5));
            }
          }
          return { result: { value: params.objectId === "worker-note" ? "file" : "invalid" } };
        }
        // External page fault injection: keep the button disabled until the
        // test releases it. No Chariox state or controller behavior is mocked.
        if ((params.objectId === "worker-save" && existsSync(join(dirname(pidFile), "hold-click"))) ||
            (params.objectId === "worker-note" && existsSync(join(dirname(pidFile), "hold-fill")))) {
          return { result: { value: { state: "disabled" } } };
        }
        if (params.functionDeclaration.includes("requestSubmit")) {
          state.submitted = state.note;
          persist();
          return { result: { value: { ok: true } } };
        }
        if (params.functionDeclaration.includes("this.focus()")) {
          state.focused = params.objectId;
          if (state.focused === "worker-note" && params.arguments?.[0]?.value) state.note = "";
          persist();
          return { result: { value: { ok: true } } };
        }
        const y = new Map([
          ["worker-save", 35], ["worker-note", 75],
          ["worker-frame", 125], ["worker-shadow", 165],
        ]).get(params.objectId);
        return { result: { value: { state: "ready", x: 60, y, width: 100, height: 30, editable: params.objectId === "worker-note" } } };
      }
      case "Input.insertText": {
        if (state.focused !== "worker-note") throw new Error("worker input is not focused");
        state.note += params.text;
        persist();
        return {};
      }
      case "Runtime.releaseObject":
        if (existsSync(join(dirname(pidFile), "hold-release"))) {
          writeFileSync(join(dirname(pidFile), "release-pending"), "external browser cleanup pending");
        }
        while (existsSync(join(dirname(pidFile), "hold-release"))) {
          await new Promise(resolve => setTimeout(resolve, 10));
        }
        return {};
      case "Input.dispatchMouseEvent": {
        if (params.x !== 60 || ![35, 125, 165].includes(params.y)) throw new Error("wrong click coordinates");
        if (params.type === "mousePressed") state.pressed = true;
        if (params.type === "mouseReleased" && state.pressed) {
          if (params.y === 35) state.saved = true;
          if (params.y === 125) state.frameClicked = true;
          if (params.y === 165) {
            state.shadowClicked = true;
            if (!state.popup) {
              state.popup = true;
              emit({
                method: "Target.targetCreated",
                params: { targetInfo: {
                  type: "page", targetId: "worker-popup",
                  url: "https://popup.worker.test/?secret=must-not-cross-relay",
                } },
              });
            }
          }
          state.clickCount += 1;
          state.submitted = null;
          state.pressed = false;
        }
        persist();
        return {};
      }
      case "Page.handleJavaScriptDialog":
        state.dialogCount = (state.dialogCount ?? 0) + 1;
        emit({ method: "Page.javascriptDialogOpening", sessionId, params: {
          type: "prompt", message: "must-not-cross-relay", defaultPrompt: "must-not-cross-relay",
        } });
        state.dialog = params;
        persist();
        emit({ method: "Page.javascriptDialogClosed", sessionId, params: {
          result: params.accept === true, userInput: params.promptText,
        } });
        return {};
      case "Browser.setDownloadBehavior":
        state.downloadConfigureCount = (state.downloadConfigureCount ?? 0) + 1;
        state.downloads = params;
        persist();
        emit({ method: "Browser.downloadWillBegin", params: {
          frameId: "worker-frame", guid: "worker-download",
          url: "https://worker.test/report?secret=must-not-cross-relay",
          suggestedFilename: "report.txt",
        } });
        emit({ method: "Browser.downloadProgress", params: {
          guid: "worker-download", state: "completed", receivedBytes: 12, totalBytes: 12,
        } });
        emit({ method: "Browser.downloadWillBegin", params: {
          frameId: "worker-frame", guid: "worker-active-download", url: "https://worker.test/large", suggestedFilename: "large.txt",
        } });
        return {};
      case "Browser.cancelDownload":
        if (params.guid !== "worker-active-download") throw new Error("wrong active download");
        state.canceledDownload = params.guid;
        persist();
        emit({ method: "Browser.downloadProgress", params: { guid: params.guid, state: "canceled", receivedBytes: 4, totalBytes: 100 } });
        return {};
      case "DOM.setFileInputFiles": state.uploadCount = (state.uploadCount ?? 0) + 1; state.upload = { backendNodeId: params.objectId === "worker-note" ? 104 : params.backendNodeId, fileCount: params.files.length }; persist(); return {};
      case "Browser.setPermission":
        state.permissionCount = (state.permissionCount ?? 0) + 1;
        state.permission = params;
        persist();
        if (existsSync(join(dirname(pidFile), "quiet-permission-events"))) return {};
        emit({ method: "Runtime.consoleAPICalled", sessionId: "worker-cdp-session", params: {
          type: "warning", args: [{ value: "must-not-cross-relay" }],
        } });
        emit({ method: "Network.requestWillBeSent", sessionId: "worker-cdp-session", params: {
          requestId: "worker-request", type: "Fetch",
          request: {
            method: "POST",
            url: "https://worker.test/api?secret=must-not-cross-relay",
            headers: { authorization: "Bearer must-not-cross-relay" },
            postData: "must-not-cross-relay",
          },
        } });
        emit({ method: "Network.responseReceived", sessionId: "worker-cdp-session", params: {
          requestId: "worker-request", type: "Fetch",
          response: {
            status: 204,
            url: "https://worker.test/api?secret=must-not-cross-relay",
            mimeType: "application/json",
          },
        } });
        emit({ method: "Page.frameNavigated", sessionId: "worker-cdp-session", params: { frame: {
          id: "worker-frame", loaderId: "worker-navigation-document",
          url: "https://worker.test/next?secret=must-not-cross-relay",
        } } });
        emit({ method: "Page.domContentEventFired", sessionId: "worker-cdp-session", params: {} });
        emit({ method: "Page.loadEventFired", sessionId: "worker-cdp-session", params: {} });
        return {};
      case "Target.setDiscoverTargets":
      case "Target.setAutoAttach":
      case "Target.detachFromTarget":
      case "Page.enable":
      case "Page.setLifecycleEventsEnabled":
      case "Runtime.enable":
      case "Network.enable":
      case "Inspector.enable":
      case "Emulation.setDeviceMetricsOverride": return {};
      default: throw new Error(`Unexpected CDP command: ${method}`);
    }
  },
};
const browser = new BrowserCdpClient({
    connectionFactory: async () => chromium,
    downloadDirectory: join(dirname(pidFile), "downloads"),
    uploadRoots: [dirname(pidFile)],
});
const uploadFiles = browser.uploadFiles.bind(browser);
browser.uploadFiles = async (request, options = {}) => {
  const observedAbort = () => writeFileSync(join(dirname(pidFile), "upload-cancel-observed"), "cancel observed");
  options.signal?.addEventListener("abort", observedAbort, { once: true });
  try { return await uploadFiles(request, options); }
  finally { options.signal?.removeEventListener("abort", observedAbort); }
};
for (const [scope, methods] of [
  ["configuration", ["configureDownloads", "setPermission"]],
  ["lifecycle", ["manageTab", "navigate", "manageHistory", "handleDialog"]],
]) for (const method of methods) {
  const configure = browser[method].bind(browser);
  browser[method] = async (request, options = {}) => {
    const root = dirname(pidFile);
    const observedAbort = () => writeFileSync(join(root, `${scope}-cancel-observed`), "cancel observed");
    options.signal?.addEventListener("abort", observedAbort, { once: true });
    try {
      if (existsSync(join(root, `hold-${scope}`))) {
        writeFileSync(join(root, `${scope}-pending`), "pending");
        while (existsSync(join(root, `hold-${scope}`))) await new Promise((resolve) => setTimeout(resolve, 5));
      }
      return await configure(request, options);
    } finally { options.signal?.removeEventListener("abort", observedAbort); }
  };
}
await new BrowserControllerStdioServer({ browser }).run();
