const DEFAULT_ACTION_TIMEOUT_MS = 5_000;
const MAX_ACTION_TIMEOUT_MS = 5_000;
const MIN_ACTION_TIMEOUT_MS = 100;
const ACTION_POLL_INTERVAL_MS = 50;
const MAX_FILL_TEXT_BYTES = 65_536;
const DIALOG_OPEN_WAIT_MS = 5_000;

export class BrowserActionError extends Error {
  constructor(code, message, details = {}) {
    super(message);
    this.name = "BrowserActionError";
    this.code = code;
    Object.assign(this, details);
  }
}

export async function performBrowserAction({
  connection,
  sessionId,
  targetId,
  documentId,
  nodeRef,
  action,
  timeoutMs,
  signal,
  now = Date.now,
  sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds)),
  assertContext = async () => {},
}) {
  const backendNodeId = parseBackendNodeReference(nodeRef);
  const normalizedAction = normalizeAction(action);
  const boundedTimeoutMs = normalizeTimeout(timeoutMs);
  const startedAt = now();
  let attempts = 0;
  let previousGeometry = null;
  let lastReason = "not_ready";
  let releaseInBackground = false;

  while (true) {
    assertNotCancelled(signal);
    if (attempts > 0 && now() - startedAt >= boundedTimeoutMs) {
      throw new BrowserActionError(
        "browser_action_timeout",
        `browser ${normalizedAction.kind} did not become actionable within ${boundedTimeoutMs}ms`,
        { reason: lastReason, attempts, timeoutMs: boundedTimeoutMs },
      );
    }
    attempts += 1;
    await assertContext();
    await assertCurrentDocument(connection, sessionId, targetId, documentId);
    const objectId = await resolveBackendNode(connection, sessionId, backendNodeId);
    try {
      const actionability = await inspectActionability(connection, sessionId, objectId);
      if (actionability.state === "detached") {
        throw new BrowserActionError(
          "stale_element_reference",
          "browser element was detached; capture a fresh snapshot before retrying",
        );
      }
      const geometry = readyGeometry(actionability, normalizedAction);
      lastReason = actionability.state;
      if (geometry && sameGeometry(previousGeometry, geometry)) {
        assertNotCancelled(signal);
        const actionResult = await executeAction(
          connection,
          sessionId,
          objectId,
          geometry,
          normalizedAction,
          signal,
        );
        releaseInBackground = actionResult.dialogOpened;
        return {
          target_id: targetId,
          document_id: documentId,
          action_kind: normalizedAction.kind,
          dialog_opened: actionResult.dialogOpened,
          attempts,
          elapsed_ms: Math.max(0, now() - startedAt),
        };
      }
      previousGeometry = geometry;
    } finally {
      if (releaseInBackground) {
        void releaseObject(connection, sessionId, objectId);
      } else {
        await releaseObject(connection, sessionId, objectId);
      }
    }
    await sleep(ACTION_POLL_INTERVAL_MS);
  }
}

export function assertNotCancelled(signal) {
  if (signal?.aborted) {
    throw new BrowserActionError("browser_action_cancelled", "browser action was cancelled");
  }
}

function normalizeAction(action) {
  if (action?.kind === "click") {
    return { kind: "click" };
  }
  if (action?.kind === "fill") {
    if (typeof action.text !== "string") {
      throw invalidAction("fill text must be a string");
    }
    if (utf8ByteLength(action.text) > MAX_FILL_TEXT_BYTES) {
      throw invalidAction(`fill text exceeds ${MAX_FILL_TEXT_BYTES} UTF-8 bytes`);
    }
    return {
      kind: "fill",
      text: action.text,
      append: action.append === true,
      submit: action.submit === true,
      expectedDocumentUrl: normalizeExpectedDocumentUrl(action.expected_document_url),
    };
  }
  if (action?.kind === "submit") {
    return { kind: "submit" };
  }
  throw invalidAction(`unsupported browser action ${JSON.stringify(action?.kind ?? null)}`);
}

function normalizeExpectedDocumentUrl(value) {
  if (value === undefined || value === null) return null;
  if (typeof value !== "string" || value.length === 0 || utf8ByteLength(value) > 2_048) {
    throw invalidAction("secret fill expected document URL is invalid");
  }
  try {
    return new URL(value).href;
  } catch {
    throw invalidAction("secret fill expected document URL is invalid");
  }
}

function normalizeTimeout(timeoutMs) {
  if (timeoutMs === undefined) {
    return DEFAULT_ACTION_TIMEOUT_MS;
  }
  if (!Number.isSafeInteger(timeoutMs) || timeoutMs <= 0) {
    throw invalidAction("browser action timeout must be a positive integer");
  }
  return Math.max(MIN_ACTION_TIMEOUT_MS, Math.min(timeoutMs, MAX_ACTION_TIMEOUT_MS));
}

function parseBackendNodeReference(nodeRef) {
  if (typeof nodeRef !== "string" || !/^backend:[1-9][0-9]*$/.test(nodeRef)) {
    throw invalidAction("browser action requires a valid controller node reference");
  }
  const backendNodeId = Number(nodeRef.slice("backend:".length));
  if (!Number.isSafeInteger(backendNodeId)) {
    throw invalidAction("browser action node reference exceeds the safe integer range");
  }
  return backendNodeId;
}

function invalidAction(message) {
  return new BrowserActionError("browser_action_invalid", message);
}

async function assertCurrentDocument(connection, sessionId, targetId, documentId) {
  const frameTree = await connection.send("Page.getFrameTree", {}, sessionId);
  const currentDocumentId = frameTree?.frameTree?.frame?.loaderId;
  if (currentDocumentId !== documentId) {
    throw new BrowserActionError(
      "stale_document_reference",
      `browser target ${JSON.stringify(targetId)} moved away from the requested document`,
    );
  }
}

async function resolveBackendNode(connection, sessionId, backendNodeId) {
  let resolved;
  try {
    resolved = await connection.send(
      "DOM.resolveNode",
      { backendNodeId },
      sessionId,
    );
  } catch (error) {
    if (error?.code !== "browser_cdp_command_failed") {
      throw error;
    }
    throw new BrowserActionError(
      "stale_element_reference",
      "browser element is no longer attached to the current document",
    );
  }
  const objectId = resolved?.object?.objectId;
  if (typeof objectId !== "string" || !objectId) {
    throw new BrowserActionError(
      "stale_element_reference",
      "browser element is no longer attached to the current document",
    );
  }
  return objectId;
}

async function inspectActionability(connection, sessionId, objectId) {
  const response = await connection.send(
    "Runtime.callFunctionOn",
    {
      objectId,
      functionDeclaration: actionabilityFunction.toString(),
      returnByValue: true,
      awaitPromise: false,
    },
    sessionId,
  );
  if (response?.exceptionDetails) {
    throw new BrowserActionError(
      "browser_action_failed",
      "browser actionability inspection failed",
    );
  }
  const result = response?.result?.value;
  if (!result || typeof result.state !== "string") {
    throw new BrowserActionError(
      "browser_action_failed",
      "browser actionability inspection returned an invalid result",
    );
  }
  if (result.state === "ready" && result.crossOriginFrame) {
    const { quads } = await connection.send("DOM.getContentQuads", { objectId }, sessionId);
    const quad = quads?.[0];
    if (!Array.isArray(quad) || quad.length !== 8 || !quad.every(Number.isFinite)) {
      return { state: "frame_unavailable" };
    }
    result.x = (quad[0] + quad[2] + quad[4] + quad[6]) / 4;
    result.y = (quad[1] + quad[3] + quad[5] + quad[7]) / 4;
  }
  return result;
}

export function actionabilityFunction() {
  if (!this.isConnected) return { state: "detached" };
  this.scrollIntoView({ block: "center", inline: "center", behavior: "instant" });
  const ownerDocument = this.ownerDocument;
  const ownerWindow = ownerDocument.defaultView;
  const style = ownerWindow.getComputedStyle(this);
  const rect = this.getBoundingClientRect();
  if (
    style.display === "none" ||
    style.visibility === "hidden" ||
    Number(style.opacity) === 0 ||
    rect.width <= 0 ||
    rect.height <= 0
  ) {
    return { state: "not_visible" };
  }
  if (
    this.disabled ||
    this.matches?.(":disabled") ||
    this.closest?.("[inert]") ||
    this.getAttribute?.("aria-disabled") === "true"
  ) {
    return { state: "disabled" };
  }
  const localX = rect.left + rect.width / 2;
  const localY = rect.top + rect.height / 2;
  let hitTarget = ownerDocument.elementFromPoint(localX, localY);
  while (hitTarget?.shadowRoot?.elementFromPoint) {
    const nestedTarget = hitTarget.shadowRoot.elementFromPoint(localX, localY);
    if (!nestedTarget || nestedTarget === hitTarget) break;
    hitTarget = nestedTarget;
  }
  let hitInsideTarget = hitTarget === this || this.contains?.(hitTarget);
  let hitRoot = hitTarget?.getRootNode?.();
  while (!hitInsideTarget && hitRoot?.host) {
    hitInsideTarget = hitRoot.host === this || this.contains?.(hitRoot.host);
    hitRoot = hitRoot.host.getRootNode?.();
  }
  if (!hitTarget || !hitInsideTarget) {
    return { state: "obscured" };
  }
  let x = localX;
  let y = localY;
  let currentWindow = ownerWindow;
  let crossOriginFrame = false;
  while (currentWindow && currentWindow !== currentWindow.top) {
    const frameElement = currentWindow.frameElement;
    if (!frameElement) {
      crossOriginFrame = true;
      break;
    }
    const frameRect = frameElement.getBoundingClientRect();
    const frameStyle = frameElement.ownerDocument.defaultView.getComputedStyle(frameElement);
    const paddingLeft = Number.parseFloat(frameStyle.paddingLeft) || 0;
    const paddingTop = Number.parseFloat(frameStyle.paddingTop) || 0;
    x += frameRect.left + frameElement.clientLeft + paddingLeft;
    y += frameRect.top + frameElement.clientTop + paddingTop;
    currentWindow = frameElement.ownerDocument.defaultView;
  }
  const inputType = this.matches?.("input")
    ? String(this.type || "text").toLowerCase()
    : null;
  const editableInput = inputType !== null && ![
    "button",
    "checkbox",
    "color",
    "file",
    "hidden",
    "image",
    "radio",
    "range",
    "reset",
    "submit",
  ].includes(inputType);
  const editable =
    this.isContentEditable ||
    ((editableInput || (this.matches?.("textarea") ?? false)) && !this.readOnly);
  return {
    state: "ready",
    x,
    y,
    width: rect.width,
    height: rect.height,
    editable,
    ...(crossOriginFrame ? { crossOriginFrame: true } : {}),
  };
}

function readyGeometry(actionability, action) {
  if (actionability.state !== "ready") {
    return null;
  }
  if (action.kind === "fill" && actionability.editable !== true) {
    actionability.state = "not_editable";
    return null;
  }
  const geometry = {
    x: actionability.x,
    y: actionability.y,
    width: actionability.width,
    height: actionability.height,
  };
  return Object.values(geometry).every(Number.isFinite) ? geometry : null;
}

function sameGeometry(left, right) {
  return left !== null &&
    left.x === right.x &&
    left.y === right.y &&
    left.width === right.width &&
    left.height === right.height;
}

async function executeAction(
  connection,
  sessionId,
  objectId,
  geometry,
  action,
  signal,
) {
  assertNotCancelled(signal);
  if (action.kind === "click") {
    return {
      dialogOpened: await dispatchClick(connection, sessionId, geometry.x, geometry.y, signal),
    };
  }
  if (action.kind === "submit") {
    await submitNearestForm(connection, sessionId, objectId);
    return { dialogOpened: false };
  }
  if (action.expectedDocumentUrl !== null) {
    await secureFillElement(connection, sessionId, objectId, action);
    return { dialogOpened: false };
  }
  await focusElement(connection, sessionId, objectId, !action.append);
  assertNotCancelled(signal);
  if (action.text) {
    await connection.send("Input.insertText", { text: action.text }, sessionId);
  } else if (!action.append) {
    await dispatchBackspace(connection, sessionId);
  }
  if (action.submit) {
    assertNotCancelled(signal);
    await submitNearestForm(connection, sessionId, objectId);
  }
  return { dialogOpened: false };
}

async function secureFillElement(connection, sessionId, objectId, action) {
  const response = await connection.send(
    "Runtime.callFunctionOn",
    {
      objectId,
      functionDeclaration: `function(text, append, expectedDocumentUrl, submit) {
        const ownerDocument = this.ownerDocument;
        const ownerWindow = ownerDocument?.defaultView;
        if (!ownerWindow || ownerWindow.location.href !== expectedDocumentUrl) {
          return { ok: false, reason: "target_url_changed" };
        }
        const isMaskedEditableInput = () => {
          const inputPrototype = ownerWindow.HTMLInputElement?.prototype;
          const elementPrototype = ownerWindow.Element?.prototype;
          const getAttribute = elementPrototype?.getAttribute;
          const hasAttribute = elementPrototype?.hasAttribute;
          if (
            !inputPrototype ||
            !Object.prototype.isPrototypeOf.call(inputPrototype, this) ||
            typeof getAttribute !== "function" ||
            typeof hasAttribute !== "function"
          ) {
            return false;
          }
          return String(getAttribute.call(this, "type") || "text").toLowerCase() === "password" &&
            !hasAttribute.call(this, "disabled") &&
            !hasAttribute.call(this, "readonly") &&
            String(getAttribute.call(this, "aria-disabled") || "false").toLowerCase() !== "true" &&
            String(getAttribute.call(this, "aria-readonly") || "false").toLowerCase() !== "true";
        };
        if (!isMaskedEditableInput()) {
          return { ok: false, reason: "target_not_masked" };
        }
        this.focus();
        if (ownerWindow.location.href !== expectedDocumentUrl) {
          return { ok: false, reason: "target_url_changed" };
        }
        const activeElement = (this.getRootNode?.() ?? ownerDocument).activeElement;
        if (!(activeElement === this || this.contains?.(activeElement))) {
          return { ok: false, reason: "target_not_focusable" };
        }
        if (!isMaskedEditableInput()) {
          return { ok: false, reason: "target_not_masked" };
        }
        const currentValue = this.isContentEditable
          ? String(this.textContent || "")
          : String(this.value || "");
        const nextValue = append ? currentValue + text : text;
        if (this.isContentEditable) {
          this.textContent = nextValue;
        } else {
          const prototype = this.matches?.("textarea")
            ? ownerWindow.HTMLTextAreaElement?.prototype
            : ownerWindow.HTMLInputElement?.prototype;
          const setter = prototype && Object.getOwnPropertyDescriptor(prototype, "value")?.set;
          if (typeof setter !== "function") {
            return { ok: false, reason: "value_setter_unavailable" };
          }
          setter.call(this, nextValue);
        }
        this.dispatchEvent(new ownerWindow.Event("input", { bubbles: true, composed: true }));
        this.dispatchEvent(new ownerWindow.Event("change", { bubbles: true }));
        if (submit) {
          const form = this.form || this.closest?.("form");
          if (!form) return { ok: false, reason: "form_not_found" };
          if (typeof form.requestSubmit === "function") form.requestSubmit();
          else form.submit();
        }
        return { ok: true };
      }`,
      arguments: [
        { value: action.text },
        { value: action.append },
        { value: action.expectedDocumentUrl },
        { value: action.submit },
      ],
      returnByValue: true,
      awaitPromise: false,
    },
    sessionId,
  );
  const outcome = response?.result?.value;
  if (response?.exceptionDetails || outcome?.ok !== true) {
    if (outcome?.reason === "target_url_changed") {
      throw new BrowserActionError(
        "browser_secret_target_changed",
        "browser secret target changed before insertion",
      );
    }
    if (outcome?.reason === "target_not_focusable") {
      throw new BrowserActionError(
        "browser_secret_target_not_focusable",
        "browser secret target could not receive focus before insertion",
      );
    }
    if (outcome?.reason === "target_not_masked") {
      throw new BrowserActionError(
        "browser_secret_target_not_masked",
        "browser secret target must remain an editable password field during insertion",
      );
    }
    throw new BrowserActionError(
      "browser_action_failed",
      "browser secret target could not receive secure input",
      { reason: outcome?.reason ?? "secure_fill_failed" },
    );
  }
}

async function submitNearestForm(connection, sessionId, objectId) {
  const response = await connection.send(
    "Runtime.callFunctionOn",
    {
      objectId,
      functionDeclaration: `function() {
        const form = this.form || this.closest?.("form");
        if (!form) return { ok: false, reason: "form_not_found" };
        if (typeof form.requestSubmit === "function") form.requestSubmit();
        else form.submit();
        return { ok: true };
      }`,
      returnByValue: true,
      awaitPromise: false,
    },
    sessionId,
  );
  if (response?.exceptionDetails || response?.result?.value?.ok !== true) {
    throw new BrowserActionError(
      "browser_submit_failed",
      "browser element does not belong to a submittable form",
    );
  }
}

async function dispatchClick(connection, sessionId, x, y, signal) {
  if (await dispatchMouseEvent(connection, sessionId, { type: "mouseMoved", x, y })) {
    return true;
  }
  assertNotCancelled(signal);
  if (await dispatchMouseEvent(connection, sessionId, {
    type: "mousePressed",
    x,
    y,
    button: "left",
    clickCount: 1,
  })) {
    return true;
  }
  const dialogOpened = await dispatchMouseEvent(connection, sessionId, {
    type: "mouseReleased",
    x,
    y,
    button: "left",
    clickCount: 1,
  });
  // Once pressed, finish the release even if cancellation arrives. Reporting
  // cancellation before this point would hand over a stuck mouse button.
  assertNotCancelled(signal);
  return dialogOpened;
}

async function dispatchMouseEvent(connection, sessionId, params) {
  const dialogWaiter = typeof connection.waitForEvent === "function"
    ? connection.waitForEvent("Page.javascriptDialogOpening", DIALOG_OPEN_WAIT_MS, sessionId)
    : null;
  const command = connection.send("Input.dispatchMouseEvent", params, sessionId);
  if (!dialogWaiter) {
    await command;
    return false;
  }
  const outcome = await Promise.race([
    command.then(() => ({ kind: "completed" })),
    dialogWaiter.promise.then((event) => ({
      kind: event ? "dialog" : "event_timeout",
    })),
  ]);
  if (outcome.kind === "completed") {
    dialogWaiter.cancel();
    return false;
  }
  if (outcome.kind === "dialog") {
    void command.catch(() => {});
    return true;
  }
  await command;
  return false;
}

async function focusElement(connection, sessionId, objectId, selectAll) {
  const response = await connection.send(
    "Runtime.callFunctionOn",
    {
      objectId,
      functionDeclaration: `function(selectAll) {
        this.focus();
        const activeElement = (this.getRootNode?.() ?? this.ownerDocument).activeElement;
        const focused = activeElement === this || this.contains?.(activeElement);
        if (focused && selectAll) {
          if (typeof this.select === "function") {
            this.select();
          } else {
            const selection = window.getSelection();
            const range = document.createRange();
            range.selectNodeContents(this);
            selection.removeAllRanges();
            selection.addRange(range);
          }
        }
        return { ok: Boolean(focused) };
      }`,
      arguments: [{ value: selectAll }],
      returnByValue: true,
    },
    sessionId,
  );
  if (response?.exceptionDetails || response?.result?.value?.ok !== true) {
    throw new BrowserActionError(
      "browser_action_failed",
      "browser fill target could not receive focus",
    );
  }
}

async function dispatchBackspace(connection, sessionId) {
  await connection.send(
    "Input.dispatchKeyEvent",
    { type: "keyDown", key: "Backspace", code: "Backspace" },
    sessionId,
  );
  await connection.send(
    "Input.dispatchKeyEvent",
    { type: "keyUp", key: "Backspace", code: "Backspace" },
    sessionId,
  );
}

async function releaseObject(connection, sessionId, objectId) {
  try {
    await connection.send("Runtime.releaseObject", { objectId }, sessionId);
  } catch {
    // The page may have navigated after a successful click.
  }
}

function utf8ByteLength(value) {
  let length = 0;
  for (const character of value) {
    const codePoint = character.codePointAt(0);
    length += codePoint <= 0x7f
      ? 1
      : codePoint <= 0x7ff
        ? 2
        : codePoint <= 0xffff
          ? 3
          : 4;
    if (length > MAX_FILL_TEXT_BYTES) {
      break;
    }
  }
  return length;
}
