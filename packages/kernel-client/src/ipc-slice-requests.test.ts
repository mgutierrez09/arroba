import assert from "node:assert/strict"
import test from "node:test"
import {
  createSliceRequest,
  getSliceDisplayEndpointRequest,
  restoreSliceBackupRequest,
} from "./ipc-slice-requests.js"
import type { SliceDisplayEndpoint } from "./kernel-types-cloud.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"

test("slice creation forwards an explicit display backend on the shared client path", () => {
  const request = createSliceRequest({
    name: "desktop",
    displayMode: "headed",
    displayBackend: "selkies",
  })
  assert.equal(request.CreateSlice.display_backend, "selkies")
  assert.equal(request.CreateSlice.display_mode, "headed")
})

test("legacy slice creation does not add a backend field", () => {
  assert.equal(Object.hasOwn(createSliceRequest({ name: "legacy" }).CreateSlice, "display_backend"), false)
})

test("slice backup restore uses the shared kernel lifecycle contract", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 307)
  assert.deepEqual(
    restoreSliceBackupRequest("linux-dev", "gmail-ready-20260609"),
    {
      RestoreSliceBackup: {
        slice_ref: "linux-dev",
        backup_ref: "gmail-ready-20260609",
      },
    },
  )
})

test("Room display admission sends the attachment and viewer identity in protocol 293", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 307)
  assert.deepEqual(
    getSliceDisplayEndpointRequest("slice-1", {
      sessionId: "room-1",
      attachmentId: "attachment-1",
      viewerPublicKey: "viewer-public-key",
    }),
    {
      GetSliceDisplayEndpoint: {
        slice_ref: "slice-1",
        session_id: "room-1",
        attachment_id: "attachment-1",
        viewer_public_key: "viewer-public-key",
      },
    },
  )
})

test("Room display endpoint exposes the encrypted stream metadata", () => {
  const endpoint: SliceDisplayEndpoint = {
    slice_id: "slice-1",
    kind: "selkies",
    url: "wss://relay.example.test/display/display-1/stream",
    access: "tunnel",
    capabilities: ["encrypted", "single_use"],
    stream_protocol: "chariox-display-v1",
    stream_id: "display-1",
    peer_public_key: "worker-public-key",
  }
  assert.equal(endpoint.stream_protocol, "chariox-display-v1")
  assert.equal(endpoint.stream_id, "display-1")
  assert.equal(endpoint.peer_public_key, "worker-public-key")
})

test("slice create serializes exact multi-repository development selection", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 307)
  assert.deepEqual(
    createSliceRequest({
      name: "project-slice",
      workspaceId: "/primary",
      worktreeId: "/primary-worktree",
      workspaceMount: "/primary-worktree",
      developmentSetup: {
        kind: "source_project",
        projectId: "project-1",
        repositories: [
          { role: "primary", workspaceId: "/primary", worktreeId: "/primary-worktree" },
          { role: "supporting", workspaceId: "/supporting", worktreeId: null },
        ],
      },
    }),
    {
      CreateSlice: {
        name: "project-slice",
        backend: "local_docker",
        os: "linux",
        display_mode: "headless",
        workspace_id: "/primary",
        worktree_id: "/primary-worktree",
        workspace_mount: "/primary-worktree",
        development: {
          kind: "source_project",
          project_id: "project-1",
          repositories: [
            { role: "primary", workspaceId: "/primary", worktreeId: "/primary-worktree" },
            { role: "supporting", workspaceId: "/supporting", worktreeId: null },
          ],
        },
        worker_kernel_ref: null,
        display_url: null,
        provider_auth: [],
        from_saved_state: null,
        base: null,
      },
    },
  )
})

test("legacy slice create omits the optional development selection", () => {
  const request = createSliceRequest({ name: "legacy-slice" })
  assert.equal("development" in request.CreateSlice, false)
})
