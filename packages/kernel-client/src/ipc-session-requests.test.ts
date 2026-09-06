import assert from "node:assert/strict"
import test from "node:test"

import {
  archiveProjectRequest,
  createSessionRequest,
  deleteProjectRequest,
  listProjectsRequest,
  renameProjectRequest,
  restoreProjectRequest,
  updateProjectWorkspacesRequest,
} from "./ipc-session-requests.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"

test("project session selection and lifecycle requests match the current protocol", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 309)
  assert.deepEqual(
    createSessionRequest(
      "workspace-1",
      "worktree-1",
      undefined,
      undefined,
      undefined,
      undefined,
      undefined,
      undefined,
      { kind: "existing", project_id: "project-1" },
    ),
    {
      CreateSession: {
        workspace_id: "workspace-1",
        worktree_id: "worktree-1",
        alias: null,
        slice_ref: null,
        project_selection: { kind: "existing", project_id: "project-1" },
      },
    },
  )
  assert.deepEqual(listProjectsRequest(true), { ListProjects: { include_archived: true } })
  assert.deepEqual(renameProjectRequest("project-1", "Renamed"), {
    RenameProject: { project_id: "project-1", name: "Renamed" },
  })
  assert.deepEqual(updateProjectWorkspacesRequest("project-1", ["workspace-1", "workspace-2"]), {
    UpdateProjectWorkspaces: {
      project_id: "project-1",
      workspace_ids: ["workspace-1", "workspace-2"],
    },
  })
  assert.deepEqual(archiveProjectRequest("project-1"), {
    ArchiveProject: { project_id: "project-1" },
  })
  assert.deepEqual(deleteProjectRequest("project-1"), {
    DeleteProject: { project_id: "project-1" },
  })
  assert.deepEqual(restoreProjectRequest("project-1"), {
    RestoreProject: { project_id: "project-1" },
  })
})
