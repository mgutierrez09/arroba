import { assertNotCancelled } from "./browser-controller-actions.mjs";

const MIN_WAIT_TIMEOUT_MS = 100;
const MAX_WAIT_TIMEOUT_MS = 5_000;
const DEFAULT_WAIT_TIMEOUT_MS = 5_000;
const WAIT_POLL_INTERVAL_MS = 50;
const MAX_SELECTOR_BYTES = 8_192;
const MAX_URL_BYTES = 8_192;

export class BrowserCompatibilityError extends Error {
  constructor(code, message) {
    super(message);
    this.name = "BrowserCompatibilityError";
    this.code = code;
  }
}

export async function navigateBrowser({
  connection,
  sessionId,
  targetId,
  documentId,
  url,
  signal,
}) {
  assertNotCancelled(signal);
  const normalizedUrl = normalizeNavigationUrl(url);
  await assertCurrentDocument(connection, sessionId, targetId, documentId);
  assertNotCancelled(signal);
  const result = await connection.send(
    "Page.navigate",
    { url: normalizedUrl },
    sessionId,
  );
  if (typeof result?.errorText === "string" && result.errorText) {
    throw new BrowserCompatibilityError(
      "browser_navigation_failed",
      `browser navigation failed: ${result.errorText}`,
    );
  }
  if (typeof result?.loaderId !== "string" || !result.loaderId) {
    throw new BrowserCompatibilityError(
      "browser_navigation_identity_missing",
      "browser navigation did not return a document identity",
    );
  }
  return {
    target_id: targetId,
    document_id: result.loaderId,
    url: normalizedUrl,
  };
}

export async function waitForBrowserState({
  connection,
  sessionId,
  targetId,
  documentId,
  kind,
  selector,
  timeoutMs,
  now = Date.now,
  sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds)),
}) {
  const wait = normalizeWait(kind, selector, timeoutMs);
  const startedAt = now();
  while (true) {
    await assertCurrentDocument(connection, sessionId, targetId, documentId);
    const response = await connection.send(
      "Runtime.evaluate",
      {
        expression: wait.expression,
        returnByValue: true,
        awaitPromise: false,
      },
      sessionId,
    );
    if (response?.exceptionDetails) {
      throw new BrowserCompatibilityError(
        "browser_wait_invalid",
        `browser ${wait.kind} wait evaluation failed`,
      );
    }
    if (response?.result?.value === true) {
      return {
        target_id: targetId,
        document_id: documentId,
        kind: wait.kind,
        ok: true,
        elapsed_ms: Math.max(0, now() - startedAt),
      };
    }
    if (now() - startedAt >= wait.timeoutMs) {
      throw new BrowserCompatibilityError(
        "browser_wait_timeout",
        `browser ${wait.kind} wait timed out after ${wait.timeoutMs}ms`,
      );
    }
    await sleep(WAIT_POLL_INTERVAL_MS);
  }
}

function normalizeNavigationUrl(rawUrl) {
  if (typeof rawUrl !== "string" || !rawUrl || utf8ByteLength(rawUrl) > MAX_URL_BYTES) {
    throw new BrowserCompatibilityError(
      "browser_navigation_invalid",
      `browser navigation URL must be a non-empty string no larger than ${MAX_URL_BYTES} UTF-8 bytes`,
    );
  }
  let url;
  try {
    url = new URL(rawUrl);
  } catch {
    throw new BrowserCompatibilityError(
      "browser_navigation_invalid",
      "browser navigation URL must be absolute",
    );
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new BrowserCompatibilityError(
      "browser_navigation_invalid",
      "browser navigation URL must use HTTP or HTTPS",
    );
  }
  return url.toString();
}

function normalizeWait(kind, selector, timeoutMs) {
  const normalizedTimeout = timeoutMs ?? DEFAULT_WAIT_TIMEOUT_MS;
  if (
    !Number.isSafeInteger(normalizedTimeout) ||
    normalizedTimeout < MIN_WAIT_TIMEOUT_MS ||
    normalizedTimeout > MAX_WAIT_TIMEOUT_MS
  ) {
    throw new BrowserCompatibilityError(
      "browser_wait_invalid",
      `browser wait timeout must be between ${MIN_WAIT_TIMEOUT_MS} and ${MAX_WAIT_TIMEOUT_MS} milliseconds`,
    );
  }
  if (kind === "idle") {
    return {
      kind,
      timeoutMs: normalizedTimeout,
      expression: "document.readyState === 'complete'",
    };
  }
  if (
    kind !== "selector" ||
    typeof selector !== "string" ||
    !selector ||
    utf8ByteLength(selector) > MAX_SELECTOR_BYTES
  ) {
    throw new BrowserCompatibilityError(
      "browser_wait_invalid",
      `browser selector wait requires a selector no larger than ${MAX_SELECTOR_BYTES} UTF-8 bytes`,
    );
  }
  const serializedSelector = JSON.stringify(selector);
  return {
    kind,
    timeoutMs: normalizedTimeout,
    expression: `(() => {
      const element = document.querySelector(${serializedSelector});
      if (!element) return false;
      const style = getComputedStyle(element);
      const rect = element.getBoundingClientRect();
      return style.visibility !== "hidden" && style.display !== "none" && rect.width > 0 && rect.height > 0;
    })()`,
  };
}

async function assertCurrentDocument(connection, sessionId, targetId, documentId) {
  const frameTree = await connection.send("Page.getFrameTree", {}, sessionId);
  if (frameTree?.frameTree?.frame?.loaderId !== documentId) {
    throw new BrowserCompatibilityError(
      "stale_document_reference",
      `browser target ${JSON.stringify(targetId)} moved away from the requested document`,
    );
  }
}

function utf8ByteLength(value) {
  return new TextEncoder().encode(value).byteLength;
}
