import assert from "node:assert/strict";
import test from "node:test";
import { uploadBrowserFiles } from "./browser-controller-files.mjs";
import { withBrowserActionFrame } from "./browser-controller-frames.mjs";

for (const scope of ["page", "child", "parent"]) {
  for (const boundary of ["DOM.resolveNode", "Runtime.callFunctionOn"]) {
    test(`upload rejects ${scope} navigation during ${boundary} before exposing files`, async () => {
      const fixture = uploadFixture(scope, boundary);
      await assert.rejects(fixture.upload(), (error) => error.code === "stale_document_reference");
      assert.equal(fixture.calls.some(({ method }) => method === "DOM.setFileInputFiles"), false);
      assert.equal(fixture.calls.filter(({ method }) => method === "Runtime.releaseObject").length, 1);
      if (scope !== "page") {
        assert.equal(fixture.calls.filter(({ method }) => method === "Target.detachFromTarget").length, 1);
      }
    });
  }
}

for (const scope of ["page", "child"]) {
  test(`a current ${scope} file input still uploads exactly once`, async () => {
    const fixture = uploadFixture(scope, null);
    const result = await fixture.upload();
    assert.equal(result.file_count, 1);
    assert.equal(result.document_id, "page-document");
    assert.equal(fixture.calls.filter(({ method }) => method === "DOM.setFileInputFiles").length, 1);
    assert.equal(JSON.stringify(result).includes("/uploads"), false);
  });
}

function uploadFixture(scope, boundary) {
  const calls = [];
  let pageDocument = "page-document";
  let childDocument = "child-document";
  const connection = {
    async send(method, params = {}, sessionId) {
      calls.push({ method, params, sessionId });
      // The browser computed a valid node result before navigation, but that
      // result reaches the controller after the document changes.
      if (method === boundary) {
        if (scope === "child") childDocument = "replacement";
        else pageDocument = "replacement";
      }
      switch (method) {
        case "Page.getFrameTree": return { frameTree: { frame: sessionId === "child-session"
          ? { id: "child", parentId: "page", loaderId: childDocument }
          : { id: "page", loaderId: pageDocument } } };
        case "Target.getTargets": return { targetInfos: [{ type: "iframe", targetId: "child" }] };
        case "Target.attachToTarget": return { sessionId: "child-session" };
        case "Target.detachFromTarget": return {};
        case "DOM.resolveNode": return { object: { objectId: "file-input" } };
        case "Runtime.callFunctionOn": return { result: { value: "file" } };
        case "DOM.setFileInputFiles":
        case "Runtime.releaseObject": return {};
        default: throw new Error(`unexpected method ${method}`);
      }
    },
  };
  const fileSystem = {
    realpath: async (value) => value,
    stat: async (value) => ({ size: 12, isDirectory: () => value === "/uploads", isFile: () => value === "/uploads/report.txt" }),
  };
  return {
    calls,
    upload: () => withBrowserActionFrame({ connection, sessionId: "page-session", targetId: "page",
      documentId: "page-document", nodeRef: scope === "page" ? "backend:1" : "frame:child:child-document:backend:1",
      filePaths: ["/uploads/report.txt"], uploadRoots: ["/uploads"], fileSystem }, uploadBrowserFiles),
  };
}
