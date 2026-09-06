import assert from "node:assert/strict"
import test from "node:test"

import { createSliceRequest } from "./ipc-slice-requests.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"

test("slice create serializes exact multi-repository development selection", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 287)
  assert.deepEqual(createSliceRequest({
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
  }), {
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
  })
})

test("legacy slice create omits the optional development selection", () => {
  const request = createSliceRequest({ name: "legacy-slice" })
  assert.equal("development" in request.CreateSlice, false)
})
