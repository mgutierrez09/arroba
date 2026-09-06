import { mkdir, realpath, stat, statfs } from "node:fs/promises";
import path from "node:path";

const MAX_UPLOAD_FILES = 20;
const MAX_UPLOAD_PATH_BYTES = 4_096;
const MAX_UPLOAD_TOTAL_BYTES = 512 * 1024 * 1024;
export const DEFAULT_MINIMUM_DOWNLOAD_FREE_BYTES = 256 * 1024 * 1024;

const defaultFileSystem = { mkdir, realpath, stat, statfs };

export class BrowserFileTransferError extends Error {
  constructor(code, message) {
    super(message);
    this.name = "BrowserFileTransferError";
    this.code = code;
  }
}

export async function cancelBrowserDownload({
  connection, browserGeneration, requestedBrowserGeneration, guid, targetsByDownload,
}) {
  if (typeof guid !== "string" || !/^[A-Za-z0-9_-]{1,128}$/.test(guid)) {
    throw new BrowserFileTransferError("browser_download_invalid", "download cancellation requires a bounded download identifier");
  }
  if (!Number.isSafeInteger(requestedBrowserGeneration) || requestedBrowserGeneration <= 0 || requestedBrowserGeneration !== browserGeneration) {
    throw new BrowserFileTransferError("stale_browser_generation", "download cancellation requires the current browser generation");
  }
  if (!targetsByDownload.has(guid)) {
    throw new BrowserFileTransferError("browser_download_not_active", "download is not active in this browser generation");
  }
  await connection.send("Browser.cancelDownload", { guid });
  return { browser_generation: browserGeneration, guid, cancellation_requested: true };
}

export async function configureBrowserDownloads({
  connection,
  sessionId,
  targetId,
  documentId,
  downloadDirectory,
  minimumFreeBytes = DEFAULT_MINIMUM_DOWNLOAD_FREE_BYTES,
  fileSystem = defaultFileSystem,
}) {
  await assertCurrentDocument(connection, sessionId, targetId, documentId);
  if (typeof downloadDirectory !== "string" || !path.isAbsolute(downloadDirectory)) {
    throw new BrowserFileTransferError(
      "browser_download_unconfigured",
      "browser downloads require a configured absolute directory",
    );
  }
  let resolvedDirectory;
  try {
    await fileSystem.mkdir(downloadDirectory, { recursive: true, mode: 0o700 });
    resolvedDirectory = await fileSystem.realpath(downloadDirectory);
    const metadata = await fileSystem.stat(resolvedDirectory);
    if (!metadata.isDirectory()) {
      throw new Error("configured download path is not a directory");
    }
  } catch (error) {
    throw new BrowserFileTransferError(
      "browser_download_unavailable",
      `browser download directory is unavailable: ${String(error?.code ?? "filesystem_error")}`,
    );
  }
  await assertBrowserDownloadHeadroom({
    downloadDirectory: resolvedDirectory,
    minimumFreeBytes,
    fileSystem,
  });
  await assertCurrentDocument(connection, sessionId, targetId, documentId);
  await connection.send("Browser.setDownloadBehavior", {
    behavior: "allowAndName",
    downloadPath: resolvedDirectory,
    eventsEnabled: true,
  });
  return {
    target_id: targetId,
    document_id: documentId,
    enabled: true,
  };
}

export async function assertBrowserDownloadHeadroom({
  downloadDirectory,
  minimumFreeBytes = DEFAULT_MINIMUM_DOWNLOAD_FREE_BYTES,
  fileSystem = defaultFileSystem,
}) {
  if (typeof downloadDirectory !== "string" || !path.isAbsolute(downloadDirectory)) {
    throw new BrowserFileTransferError(
      "browser_download_unconfigured",
      "browser downloads require a configured absolute directory",
    );
  }
  if (!Number.isSafeInteger(minimumFreeBytes) || minimumFreeBytes < 0) {
    throw new BrowserFileTransferError(
      "browser_download_unconfigured",
      "browser download free-space reserve must be a non-negative integer",
    );
  }
  let filesystem;
  try {
    filesystem = await fileSystem.statfs(downloadDirectory);
  } catch (error) {
    throw new BrowserFileTransferError(
      "browser_download_unavailable",
      `browser download storage capacity is unavailable: ${String(error?.code ?? "filesystem_error")}`,
    );
  }
  const availableBytes = filesystemAvailableBytes(filesystem);
  if (availableBytes < BigInt(minimumFreeBytes)) {
    throw new BrowserFileTransferError(
      "browser_download_low_disk",
      `browser downloads need ${Math.ceil(minimumFreeBytes / (1024 * 1024))} MiB of free slice storage; free disk space and retry`,
    );
  }
}

function filesystemAvailableBytes(filesystem) {
  const available = filesystem?.bavail;
  const blockSize = filesystem?.bsize;
  if (
    !["bigint", "number"].includes(typeof available) ||
    !["bigint", "number"].includes(typeof blockSize)
  ) {
    throw new BrowserFileTransferError(
      "browser_download_unavailable",
      "browser download storage capacity is invalid",
    );
  }
  if (
    (typeof available === "number" && (!Number.isSafeInteger(available) || available < 0)) ||
    (typeof blockSize === "number" && (!Number.isSafeInteger(blockSize) || blockSize < 0)) ||
    (typeof available === "bigint" && available < 0n) ||
    (typeof blockSize === "bigint" && blockSize < 0n)
  ) {
    throw new BrowserFileTransferError(
      "browser_download_unavailable",
      "browser download storage capacity is invalid",
    );
  }
  const availableBytes = BigInt(available) * BigInt(blockSize);
  if (availableBytes < 0n) {
    throw new BrowserFileTransferError(
      "browser_download_unavailable",
      "browser download storage capacity is invalid",
    );
  }
  return availableBytes;
}

export async function uploadBrowserFiles({
  connection,
  sessionId,
  targetId,
  documentId,
  nodeRef,
  filePaths,
  uploadRoots,
  fileSystem = defaultFileSystem,
  assertContext = async () => {},
}) {
  await assertCurrentDocument(connection, sessionId, targetId, documentId);
  const backendNodeId = parseBackendNodeReference(nodeRef);
  if (!Array.isArray(filePaths) || filePaths.length === 0 || filePaths.length > MAX_UPLOAD_FILES) {
    throw invalidUpload(`browser upload requires 1 through ${MAX_UPLOAD_FILES} files`);
  }
  if (!Array.isArray(uploadRoots) || uploadRoots.length === 0) {
    throw new BrowserFileTransferError(
      "browser_upload_denied",
      "browser uploads require a configured file root",
    );
  }

  const roots = await resolveUploadRoots(uploadRoots, fileSystem);
  const files = [];
  let totalBytes = 0;
  for (const candidate of filePaths) {
    if (
      typeof candidate !== "string" ||
      !path.isAbsolute(candidate) ||
      !utf8ByteLengthAtMost(candidate, MAX_UPLOAD_PATH_BYTES)
    ) {
      throw invalidUpload("browser upload paths must be bounded absolute paths");
    }
    let resolved;
    let metadata;
    try {
      resolved = await fileSystem.realpath(candidate);
      metadata = await fileSystem.stat(resolved);
    } catch (error) {
      throw invalidUpload(`browser upload file is unavailable: ${String(error?.code ?? "filesystem_error")}`);
    }
    if (!roots.some((root) => isWithinRoot(root, resolved))) {
      throw new BrowserFileTransferError(
        "browser_upload_denied",
        "browser upload file is outside configured roots",
      );
    }
    if (!metadata.isFile() || !Number.isSafeInteger(metadata.size) || metadata.size < 0) {
      throw invalidUpload("browser uploads require regular files with a bounded size");
    }
    totalBytes += metadata.size;
    if (!Number.isSafeInteger(totalBytes) || totalBytes > MAX_UPLOAD_TOTAL_BYTES) {
      throw invalidUpload(`browser upload exceeds ${MAX_UPLOAD_TOTAL_BYTES} total bytes`);
    }
    files.push(resolved);
  }

  await assertContext();
  await assertCurrentDocument(connection, sessionId, targetId, documentId);
  let objectId;
  try {
    const resolved = await connection.send("DOM.resolveNode", { backendNodeId }, sessionId);
    objectId = resolved?.object?.objectId;
    if (!objectId) throw staleFileInput();
    const inspected = await connection.send("Runtime.callFunctionOn", {
      objectId,
      functionDeclaration: `function() {
        if (!this.isConnected || this.ownerDocument !== this.ownerDocument.defaultView?.document) return "detached";
        return this.localName === "input" && this.type === "file" ? "file" : "invalid";
      }`,
      returnByValue: true,
      awaitPromise: false,
    }, sessionId);
    if (inspected?.exceptionDetails || inspected?.result?.value !== "file") {
      if (inspected?.result?.value === "invalid") throw invalidUpload("browser upload requires a file input");
      throw staleFileInput();
    }
    // Resolving and inspecting the node cross asynchronous renderer calls.
    // Recheck both the owning document and its parents before exposing files.
    await assertContext();
    await assertCurrentDocument(connection, sessionId, targetId, documentId);
    await connection.send(
      "DOM.setFileInputFiles",
      { objectId, files },
      sessionId,
    );
  } catch (error) {
    if (error?.code !== "browser_cdp_command_failed") throw error;
    throw staleFileInput();
  } finally {
    if (objectId) await connection.send("Runtime.releaseObject", { objectId }, sessionId).catch(() => {});
  }
  return {
    target_id: targetId,
    document_id: documentId,
    file_count: files.length,
    total_bytes: totalBytes,
  };
}

function staleFileInput() {
  return new BrowserFileTransferError(
    "stale_element_reference",
    "browser file input is no longer attached to the current document",
  );
}

async function resolveUploadRoots(uploadRoots, fileSystem) {
  const roots = [];
  for (const root of uploadRoots) {
    if (typeof root !== "string" || !path.isAbsolute(root)) {
      throw new BrowserFileTransferError(
        "browser_upload_denied",
        "browser upload roots must be absolute directories",
      );
    }
    try {
      const resolved = await fileSystem.realpath(root);
      const metadata = await fileSystem.stat(resolved);
      if (!metadata.isDirectory()) throw new Error("upload root is not a directory");
      roots.push(resolved);
    } catch (error) {
      throw new BrowserFileTransferError(
        "browser_upload_denied",
        `browser upload root is unavailable: ${String(error?.code ?? "filesystem_error")}`,
      );
    }
  }
  return roots;
}

async function assertCurrentDocument(connection, sessionId, targetId, documentId) {
  const frameTree = await connection.send("Page.getFrameTree", {}, sessionId);
  if (frameTree?.frameTree?.frame?.loaderId !== documentId) {
    throw new BrowserFileTransferError(
      "stale_document_reference",
      `browser target ${JSON.stringify(targetId)} moved away from the requested document`,
    );
  }
}

function parseBackendNodeReference(nodeRef) {
  if (typeof nodeRef !== "string" || !/^backend:[1-9][0-9]*$/.test(nodeRef)) {
    throw invalidUpload("browser upload requires a valid controller node reference");
  }
  const backendNodeId = Number(nodeRef.slice("backend:".length));
  if (!Number.isSafeInteger(backendNodeId)) {
    throw invalidUpload("browser upload node reference exceeds the safe integer range");
  }
  return backendNodeId;
}

function isWithinRoot(root, candidate) {
  const relative = path.relative(root, candidate);
  return relative === "" || (
    relative !== ".." &&
    !relative.startsWith(`..${path.sep}`) &&
    !path.isAbsolute(relative)
  );
}

function invalidUpload(message) {
  return new BrowserFileTransferError("browser_upload_invalid", message);
}

function utf8ByteLengthAtMost(value, limit) {
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
    if (length > limit) return false;
  }
  return true;
}
