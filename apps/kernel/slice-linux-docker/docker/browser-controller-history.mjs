import { assertNotCancelled } from "./browser-controller-actions.mjs";

const HISTORY_ACTIONS = new Set(["back", "forward", "reload"]);
const DEFAULT_HISTORY_TIMEOUT_MS = 5_000;
const HISTORY_POLL_INTERVAL_MS = 25;

export class BrowserHistoryError extends Error {
  constructor(code, message) {
    super(message);
    this.name = "BrowserHistoryError";
    this.code = code;
  }
}

export async function navigateBrowserHistory({
  connection,
  sessionId,
  targetId,
  documentId,
  action,
  signal,
  timeoutMs = DEFAULT_HISTORY_TIMEOUT_MS,
  now = Date.now,
  sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds)),
}) {
  assertNotCancelled(signal);
  if (!HISTORY_ACTIONS.has(action)) {
    throw new BrowserHistoryError(
      "browser_history_action_invalid",
      "browser history action must be back, forward, or reload",
    );
  }
  const initialFrame = await assertCurrentDocument(
    connection,
    sessionId,
    targetId,
    documentId,
  );
  let expectedEntry = null;

  if (action === "reload") {
    assertNotCancelled(signal);
    await connection.send("Page.reload", {}, sessionId);
  } else {
    const history = await connection.send("Page.getNavigationHistory", {}, sessionId);
    const offset = action === "back" ? -1 : 1;
    const entry = history?.entries?.[history.currentIndex + offset];
    if (!entry || !Number.isSafeInteger(entry.id)) {
      throw new BrowserHistoryError(
        "browser_history_unavailable",
        `browser history has no ${action} entry`,
      );
    }
    expectedEntry = entry;
    assertNotCancelled(signal);
    await connection.send("Page.navigateToHistoryEntry", { entryId: entry.id }, sessionId);
  }

  const startedAt = now();
  while (now() - startedAt <= timeoutMs) {
    const frameTree = await connection.send("Page.getFrameTree", {}, sessionId);
    const frame = frameTree?.frameTree?.frame;
    if (typeof frame?.loaderId === "string" && frame.loaderId && frame.loaderId !== documentId) {
      return {
        target_id: targetId,
        document_id: frame.loaderId,
        action,
        url: frame.url,
      };
    }
    if (expectedEntry && frame?.loaderId === documentId) {
      const history = await connection.send("Page.getNavigationHistory", {}, sessionId);
      const currentEntry = history?.entries?.[history.currentIndex];
      const reachedEntry = currentEntry?.id === expectedEntry.id;
      const reachedUrl =
        frame?.url === expectedEntry.url || expectedEntry.url === initialFrame.url;
      if (reachedEntry && reachedUrl) {
        return {
          target_id: targetId,
          document_id: documentId,
          action,
          url: frame.url,
        };
      }
    }
    await sleep(HISTORY_POLL_INTERVAL_MS);
  }
  throw new BrowserHistoryError(
    "browser_history_timeout",
    `browser ${action} did not produce a new document within ${timeoutMs}ms`,
  );
}

async function assertCurrentDocument(connection, sessionId, targetId, documentId) {
  const frameTree = await connection.send("Page.getFrameTree", {}, sessionId);
  const frame = frameTree?.frameTree?.frame;
  if (frame?.loaderId !== documentId) {
    throw new BrowserHistoryError(
      "stale_document_reference",
      `browser target ${JSON.stringify(targetId)} moved away from the requested document`,
    );
  }
  return frame;
}
