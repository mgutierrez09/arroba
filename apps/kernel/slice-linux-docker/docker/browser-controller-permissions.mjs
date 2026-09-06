import { assertNotCancelled } from "./browser-controller-actions.mjs";

const PERMISSION_DESCRIPTORS = new Map([
  ["camera", { name: "camera" }],
  ["clipboard-read-write", { name: "clipboard-read" }],
  ["clipboard-sanitized-write", { name: "clipboard-write", allowWithoutSanitization: false }],
  ["display-capture", { name: "display-capture" }],
  ["geolocation", { name: "geolocation" }],
  ["local-fonts", { name: "local-fonts" }],
  ["microphone", { name: "microphone" }],
  ["midi", { name: "midi" }],
  ["midi-sysex", { name: "midi", sysex: true }],
  ["notifications", { name: "notifications" }],
]);
const PERMISSION_SETTINGS = new Set(["granted", "denied", "prompt"]);

export class BrowserPermissionError extends Error {
  constructor(code, message) {
    super(message);
    this.name = "BrowserPermissionError";
    this.code = code;
  }
}

export async function setBrowserPermission({
  connection,
  sessionId,
  targetId,
  documentId,
  targetUrl,
  permission,
  setting,
  signal,
}) {
  assertNotCancelled(signal);
  const descriptor = PERMISSION_DESCRIPTORS.get(permission);
  if (!descriptor || !PERMISSION_SETTINGS.has(setting)) {
    throw new BrowserPermissionError(
      "browser_permission_invalid",
      "browser permission name or setting is not supported",
    );
  }
  const frameTree = await connection.send("Page.getFrameTree", {}, sessionId);
  if (frameTree?.frameTree?.frame?.loaderId !== documentId) {
    throw new BrowserPermissionError(
      "stale_document_reference",
      `browser target ${JSON.stringify(targetId)} moved away from the requested document`,
    );
  }
  let origin;
  try {
    const url = new URL(targetUrl);
    if (url.protocol !== "http:" && url.protocol !== "https:") throw new Error("unsafe scheme");
    origin = url.origin;
  } catch {
    throw new BrowserPermissionError(
      "browser_permission_origin_denied",
      "browser permissions require the current HTTP or HTTPS origin",
    );
  }
  assertNotCancelled(signal);
  await connection.send("Browser.setPermission", {
    permission: descriptor,
    setting,
    origin,
  });
  return {
    target_id: targetId,
    document_id: documentId,
    permission,
    setting,
  };
}
