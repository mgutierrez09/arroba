import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance, SliceBackupRecord, SliceDisplayEndpoint, SliceRecord, SliceSavedStateRecord } from "./cli-types.js"
import { handleSliceSlashCommand, type SliceCommandHandlerDeps } from "./slice-command-handlers.js"

test("slice command list renders lifecycle scope and provider auth details", async () => {
  const harness = sliceHarness({
    slices: [
      slice({
        id: "slice-1",
        name: "linux-dev",
        status: "running",
        display_mode: "headed",
        worktree_id: "/repo/feature",
        session_ids: ["session-1", "session-2"],
        agent_ids: ["agent-1"],
        relay_endpoint: { url: "wss://relay.example/slice", private: false },
        providers: ["codex", "claude"],
        provider_auth: [
          { provider: "codex", account_profile: "work", state: "configured", account_id: "acct-1", source: "test" },
          { provider: "claude", account_profile: "default", state: "authenticated", email: "user@example.com", organization_name: "Team", subscription_type: "pro", source: "test" },
        ],
      }),
    ],
  })

  await handleSliceSlashCommand(harness.deps, command("list"))

  assert.match(harness.notices.at(-1) ?? "", /linux-dev id=slice-1 status=running display=headed/)
  assert.match(harness.notices.at(-1) ?? "", /worktree=\/repo\/feature agents=1 sessions=2/)
  assert.match(harness.notices.at(-1) ?? "", /worker=kernel-slice relay=shared:wss:\/\/relay.example\/slice/)
  assert.match(harness.notices.at(-1) ?? "", /auth_status=ready codex, claude/)
  assert.match(harness.notices.at(-1) ?? "", /providers=codex,claude auth_status=ready codex, claude auth=codex:work \(acct-1\),claude:default \(user@example.com\)\/org=Team\/plan=pro/)
  assert.equal(harness.footers.at(-1)?.message, "listed 1 slice")
})

test("slice command list renders provider account recovery hints", async () => {
  const harness = sliceHarness({
    slices: [
      slice({
        id: "slice-1",
        name: "linux-dev",
        providers: ["codex"],
        provider_auth: [],
      }),
    ],
  })

  await handleSliceSlashCommand(harness.deps, command("list"))

  assert.match(harness.notices.at(-1) ?? "", /auth_status=missing codex/)
  assert.match(harness.notices.at(-1) ?? "", /providers=codex auth_status=missing codex auth=-/)
  assert.match(harness.notices.at(-1) ?? "", /next=import or login provider accounts for codex with \/slice auth import linux-dev codex <account-profile> or \/slice auth login linux-dev codex <account-profile>/)
})

test("slice command list renders concrete commands for multi-provider recovery hints", async () => {
  const harness = sliceHarness({
    slices: [
      slice({
        id: "slice-1",
        name: "linux-dev",
        providers: ["codex", "opencode:openai"],
        provider_auth: [],
      }),
    ],
  })

  await handleSliceSlashCommand(harness.deps, command("list"))

  assert.match(harness.notices.at(-1) ?? "", /auth_status=missing codex, opencode:openai/)
  assert.match(harness.notices.at(-1) ?? "", /providers=codex,opencode:openai auth_status=missing codex, opencode:openai auth=-/)
  assert.match(harness.notices.at(-1) ?? "", /next=import or login provider accounts for codex,opencode:openai with \/slice auth import linux-dev codex <account-profile> or \/slice auth login linux-dev codex <account-profile>; for opencode:openai use \/slice auth import linux-dev opencode:openai <account-profile> or \/slice auth login linux-dev opencode:openai <account-profile>/)
})

test("slice command list renders concrete stale-auth recovery hints", async () => {
  const harness = sliceHarness({
    slices: [
      slice({
        id: "slice-1",
        name: "linux-a",
        providers: ["codex"],
        provider_auth: [{ provider: "codex", account_profile: "default", state: "not_configured", source: "slice" }],
      }),
      slice({
        id: "slice-2",
        name: "linux-b",
        providers: ["codex", "opencode:openai"],
        provider_auth: [
          { provider: "codex", account_profile: "default", state: "not_configured", source: "slice" },
          { provider: "opencode:openai", account_profile: "default", state: "unknown", source: "slice" },
        ],
      }),
    ],
  })

  await handleSliceSlashCommand(harness.deps, command("list"))

  const notice = harness.notices.at(-1) ?? ""
  assert.match(notice, /linux-a[\s\S]*auth_status=refresh codex/)
  assert.match(notice, /linux-b[\s\S]*auth_status=refresh codex, opencode:openai/)
  assert.match(notice, /linux-a[\s\S]*next=refresh provider login for codex with \/slice auth login linux-a codex <account-profile>/)
  assert.match(notice, /linux-b[\s\S]*next=refresh provider login for codex,opencode:openai with \/slice auth login linux-b codex <account-profile>; for opencode:openai use \/slice auth login linux-b opencode:openai <account-profile>/)
})

test("slice command create passes display mode and current worktree mount", async () => {
  const harness = sliceHarness()

  await handleSliceSlashCommand(harness.deps, command("create", "qa", "--headed"))

  assert.deepEqual(harness.createdSlices, [{
    name: "qa",
    displayMode: "headed",
    workspaceId: "/repo",
    worktreeId: "/repo/wt",
    workspaceMount: "/repo/wt",
    workerKernelRef: null,
    displayUrl: null,
    fromSavedState: null,
    base: null,
  }])
  assert.equal(harness.footers.at(-1)?.message, "created slice qa")
})

test("slice command create can request a clean base", async () => {
  const harness = sliceHarness()

  await handleSliceSlashCommand(harness.deps, command("create", "qa-clean", "--clean"))

  assert.equal(harness.createdSlices[0]?.base, "clean")
  assert.equal(harness.footers.at(-1)?.message, "created slice qa-clean")
})

test("slice command create can restore from saved state", async () => {
  const harness = sliceHarness()

  await handleSliceSlashCommand(harness.deps, command("create", "qa-restored", "--from-state", "qa"))

  assert.equal(harness.createdSlices[0]?.fromSavedState, "qa")
  assert.equal(harness.footers.at(-1)?.message, "created slice qa-restored")
})

test("slice command screen resolves focused agent slice and opens endpoint", async () => {
  const harness = sliceHarness({
    slices: [
      slice({
        id: "slice-1",
        name: "linux-dev",
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
      }),
    ],
    focusedAgent: {
      remote_execution: {
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
        execution_lease_id: "lease-1",
        leased_agent_id: "worker-agent",
      },
    },
    endpoint: { slice_id: "slice-1", kind: "novnc", url: "http://127.0.0.1:6080", access: "local" },
  })

  await handleSliceSlashCommand(harness.deps, command("screen"))

  assert.deepEqual(harness.openedUrls, ["http://127.0.0.1:6080"])
  assert.deepEqual(harness.displayEndpointRefs, ["linux-dev"])
  assert.equal(harness.footers.at(-1)?.message, "opened http://127.0.0.1:6080")
})

test("slice command focused lookup prefers explicit agent bindings", async () => {
  const harness = sliceHarness({
    slices: [
      slice({
        id: "slice-1",
        name: "wrong-by-worker",
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
        agent_ids: ["agent-other"],
      }),
      slice({
        id: "slice-2",
        name: "right-by-agent",
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
        agent_ids: ["agent-1"],
      }),
    ],
    focusedAgent: {
      remote_execution: {
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
        execution_lease_id: "lease-1",
        leased_agent_id: "worker-agent",
      },
    },
    endpoint: { slice_id: "slice-2", kind: "novnc", url: "http://127.0.0.1:6081", access: "local" },
  })

  await handleSliceSlashCommand(harness.deps, command("screen"))

  assert.deepEqual(harness.displayEndpointRefs, ["right-by-agent"])
  assert.deepEqual(harness.openedUrls, ["http://127.0.0.1:6081"])
})

test("slice command doctor renders health checks", async () => {
  const harness = sliceHarness({
    slices: [
      slice({
        id: "slice-1",
        name: "linux-dev",
        status: "unhealthy",
        display_mode: "headed",
        worker_kernel_id: null,
        worktree_id: "/repo/wt",
        relay_endpoint: null,
        session_ids: ["session-1"],
        agent_ids: ["agent-1"],
      }),
    ],
  })

  await handleSliceSlashCommand(harness.deps, command("doctor", "linux-dev"))

  assert.match(harness.notices.at(-1) ?? "", /slice doctor linux-dev \(slice-1\)/)
  assert.match(harness.notices.at(-1) ?? "", /fail lifecycle: unhealthy/)
  assert.match(harness.notices.at(-1) ?? "", /fail display: headed/)
  assert.match(harness.notices.at(-1) ?? "", /ok relay: none/)
  assert.match(harness.notices.at(-1) ?? "", /ok agents: 1 attached/)
  assert.match(harness.notices.at(-1) ?? "", /next: inspect slice logs and \/slice audit/)
  assert.equal(harness.footers.at(-1)?.tone, "error")
})

test("slice command doctor flags running slices without a relay endpoint", async () => {
  const harness = sliceHarness({
    slices: [
      slice({
        id: "slice-1",
        name: "linux-dev",
        status: "running",
        worker_kernel_id: "kernel-slice",
        relay_endpoint: null,
      }),
    ],
  })

  await handleSliceSlashCommand(harness.deps, command("doctor", "linux-dev"))

  assert.match(harness.notices.at(-1) ?? "", /fail relay: none/)
  assert.match(harness.notices.at(-1) ?? "", /next: check relay connectivity/)
  assert.equal(harness.footers.at(-1)?.tone, "error")
})

test("slice command doctor flags missing provider accounts", async () => {
  const harness = sliceHarness({
    slices: [
      slice({
        id: "slice-1",
        name: "linux-dev",
        status: "running",
        worker_kernel_id: "kernel-slice",
        relay_endpoint: { url: "wss://relay.example/slice", private: false },
        worktree_id: "/repo/wt",
        providers: ["codex"],
        provider_auth: [],
      }),
    ],
  })

  await handleSliceSlashCommand(harness.deps, command("doctor", "linux-dev"))

  assert.match(harness.notices.at(-1) ?? "", /ok provider CLIs: codex/)
  assert.match(harness.notices.at(-1) ?? "", /fail provider accounts: missing codex/)
  assert.match(harness.notices.at(-1) ?? "", /next: import or login provider accounts for codex/)
  assert.equal(harness.footers.at(-1)?.tone, "error")
})

test("slice command doctor requires provider auth for every advertised provider", async () => {
  const harness = sliceHarness({
    slices: [
      slice({
        id: "slice-1",
        name: "linux-dev",
        status: "running",
        worker_kernel_id: "kernel-slice",
        relay_endpoint: { url: "wss://relay.example/slice", private: false },
        worktree_id: "/repo/wt",
        providers: ["codex", "opencode:openai"],
        provider_auth: [
          { provider: "codex", account_profile: "default", state: "authenticated", email: "codex@example.com", source: "test" },
        ],
      }),
    ],
  })

  await handleSliceSlashCommand(harness.deps, command("doctor", "linux-dev"))

  assert.match(harness.notices.at(-1) ?? "", /ok provider CLIs: codex,opencode:openai/)
  assert.match(harness.notices.at(-1) ?? "", /fail provider accounts: codex:default \(codex@example.com\); missing opencode:openai/)
  assert.match(harness.notices.at(-1) ?? "", /next: import or login provider accounts for opencode:openai with \/slice auth import linux-dev opencode:openai <account-profile> or \/slice auth login linux-dev opencode:openai <account-profile>/)
  assert.equal(harness.footers.at(-1)?.tone, "error")
})

test("slice command logs renders focused slice diagnostics", async () => {
  const harness = sliceHarness({
    slices: [
      slice({
        id: "slice-1",
        name: "linux-dev",
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
      }),
    ],
    focusedAgent: {
      remote_execution: {
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
        execution_lease_id: "lease-1",
        leased_agent_id: "worker-agent",
      },
    },
  })

  await handleSliceSlashCommand(harness.deps, command("logs", "--tail", "25"))

  assert.deepEqual(harness.logRequests, [{ sliceRef: "linux-dev", tailLines: 25 }])
  assert.match(harness.notices.at(-1) ?? "", /slice logs linux-dev \(slice-1\)/)
  assert.match(harness.notices.at(-1) ?? "", /== provision path=\/tmp\/slice.log truncated ==/)
  assert.match(harness.notices.at(-1) ?? "", /slice booted/)
  assert.equal(harness.footers.at(-1)?.message, "slice logs linux-dev")
})

test("slice command audit resolves focused slice and passes limit", async () => {
  const harness = sliceHarness({
    slices: [
      slice({
        id: "slice-1",
        name: "linux-dev",
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
      }),
    ],
    focusedAgent: {
      remote_execution: {
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
        execution_lease_id: "lease-1",
        leased_agent_id: "worker-agent",
      },
    },
  })

  await handleSliceSlashCommand(harness.deps, command("audit", "--limit", "5"))

  assert.deepEqual(harness.auditRequests, [{ sliceRef: "linux-dev", limit: 5 }])
  assert.match(harness.notices.at(-1) ?? "", /2026-01-02T03:04:05.000Z auth\.import completed slice=linux-dev provider=codex/)
  assert.match(harness.notices.at(-1) ?? "", /status=running backend=local_docker display=headless worktree=\/repo\/wt sessions=2 agents=1 worker=kernel-slice machine=machine-slice/)
  assert.match(harness.notices.at(-1) ?? "", /2026-01-02T03:04:06.000Z auth\.login failed slice=linux-dev provider=opencode message=login failed/)
  assert.match(harness.notices.at(-1) ?? "", /next: run \/slice doctor linux-dev; retry with \/slice auth login linux-dev opencode <account-profile> or \/slice auth import linux-dev opencode <account-profile>/)
  assert.equal(harness.footers.at(-1)?.message, "slice audit linux-dev")
})

test("slice saved-state commands call kernel APIs and render metadata", async () => {
  const harness = sliceHarness()

  await handleSliceSlashCommand(harness.deps, command("save-state", "linux-dev"))
  await handleSliceSlashCommand(harness.deps, command("save-state", "linux-dev", "--future-slices"))
  await handleSliceSlashCommand(harness.deps, command("state", "linux-dev"))
  await handleSliceSlashCommand(harness.deps, command("reset-state", "linux-dev"))

  assert.deepEqual(harness.savedStates, [
    { sliceRef: "linux-dev", mode: undefined, scope: undefined },
    { sliceRef: "linux-dev", mode: undefined, scope: "future_slices" },
  ])
  assert.deepEqual(harness.stateStatusRequests, ["linux-dev"])
  assert.deepEqual(harness.resetStates, ["linux-dev"])
  assert.match(harness.notices.join("\n"), /saved slice state linux-dev/)
  assert.match(harness.notices.join("\n"), /slice state linux-dev/)
  assert.match(harness.notices.join("\n"), /removed_state=state-1/)
})

test("slice backup command supports explicit create action and backup name", async () => {
  const harness = sliceHarness()

  await handleSliceSlashCommand(harness.deps, command("backup", "create", "linux-dev", "--name", "before-upgrade"))

  assert.deepEqual(harness.backups, [{ sliceRef: "linux-dev", name: "before-upgrade" }])
  assert.match(harness.notices.at(-1) ?? "", /created slice backup linux-dev/)
  assert.match(harness.notices.at(-1) ?? "", /Use this backup by swapping slice state directories/)
})

test("slice backup restore routes the selected backup and reports the stopped result", async () => {
  const harness = sliceHarness()

  await handleSliceSlashCommand(
    harness.deps,
    command("backup", "restore", "linux-dev", "before-upgrade"),
  )

  assert.deepEqual(harness.restoredBackups, [
    { sliceRef: "linux-dev", backupRef: "before-upgrade" },
  ])
  assert.match(harness.notices.at(-1) ?? "", /restored slice backup linux-dev/)
  assert.match(harness.notices.at(-1) ?? "", /status=stopped/)
})

test("slice backup restore blocks slices with attached agents", async () => {
  const harness = sliceHarness({
    slices: [slice({
      id: "slice-1",
      name: "linux-dev",
      agent_ids: ["agent-build"],
    })],
  })

  await handleSliceSlashCommand(
    harness.deps,
    command("backup", "restore", "linux-dev", "before-upgrade"),
  )

  assert.deepEqual(harness.restoredBackups, [])
  assert.equal(
    harness.footers.at(-1)?.message,
    "cannot restore backup for slice linux-dev; move or end attached agents first: agent-build",
  )
})

test("slice command auth import can target the focused agent slice", async () => {
  const harness = sliceHarness({
    slices: [
      slice({
        id: "slice-1",
        name: "linux-dev",
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
      }),
    ],
    focusedAgent: {
      remote_execution: {
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
        execution_lease_id: "lease-1",
        leased_agent_id: "worker-agent",
      },
    },
  })

  await handleSliceSlashCommand(harness.deps, command("auth", "import", "codex", "work"))

  assert.deepEqual(harness.importedAuth, [{ sliceRef: "linux-dev", provider: "codex", accountProfile: "work" }])
  assert.equal(harness.footers.at(-1)?.message, "slice auth import codex: imported")
})

test("slice command auth remove can target the focused agent slice", async () => {
  const harness = sliceHarness({
    slices: [
      slice({
        id: "slice-1",
        name: "linux-dev",
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
      }),
    ],
    focusedAgent: {
      remote_execution: {
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
        execution_lease_id: "lease-1",
        leased_agent_id: "worker-agent",
      },
    },
  })

  await handleSliceSlashCommand(harness.deps, command("auth", "remove", "opencode", "default"))

  assert.deepEqual(harness.removedAuth, [{ sliceRef: "linux-dev", provider: "opencode", accountProfile: "default" }])
  assert.equal(harness.footers.at(-1)?.message, "slice auth remove opencode: removed")
})

test("slice command auth import and remove explain unsupported worker operations", async () => {
  const harness = sliceHarness({
    importedAuthStatus: "not_implemented",
    removedAuthStatus: "not_implemented",
    slices: [
      slice({
        id: "slice-1",
        name: "linux-dev",
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
      }),
    ],
    focusedAgent: {
      remote_execution: {
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
        execution_lease_id: "lease-1",
        leased_agent_id: "worker-agent",
      },
    },
  })

  await handleSliceSlashCommand(harness.deps, command("auth", "import", "codex", "work"))
  await handleSliceSlashCommand(harness.deps, command("auth", "remove", "codex", "work"))

  assert.equal(harness.footers.at(-2)?.tone, "error")
  assert.match(
    harness.footers.at(-2)?.message ?? "",
    /slice auth import codex is unavailable on this kernel\. Next action: use \/slice auth login linux-dev codex <account-profile>, open \/slice screen linux-dev to configure the account inside the slice, or update\/restart the worker kernel if auth import should be available\./,
  )
  assert.equal(harness.footers.at(-1)?.tone, "error")
  assert.match(
    harness.footers.at(-1)?.message ?? "",
    /slice auth remove codex is unavailable on this kernel\. Next action: open \/slice screen linux-dev to remove the provider account inside the slice, or update\/restart the worker kernel if auth removal should be available\./,
  )
})

test("slice command auth login starts provider login in focused agent slice", async () => {
  const harness = sliceHarness({
    slices: [
      slice({
        id: "slice-1",
        name: "linux-dev",
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
      }),
    ],
    focusedAgent: {
      remote_execution: {
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
        execution_lease_id: "lease-1",
        leased_agent_id: "worker-agent",
      },
    },
  })

  await handleSliceSlashCommand(harness.deps, command("auth", "login", "codex", "work"))

  assert.deepEqual(harness.startedAuthLogins, [{ sliceRef: "linux-dev", provider: "codex", accountProfile: "work" }])
  assert.match(harness.notices.at(-1) ?? "", /url=https:\/\/auth.example/)
  assert.match(harness.notices.at(-1) ?? "", /code=ABCD-EFGH/)
  assert.equal(harness.footers.at(-1)?.message, "slice auth login codex: started")
})

test("slice command stop blocks slices with attached agents", async () => {
  const harness = sliceHarness({
    slices: [slice({
      id: "slice-1",
      name: "linux-dev",
      agent_ids: ["agent-build", "agent-review"],
    })],
  })

  await handleSliceSlashCommand(harness.deps, command("stop", "linux-dev"))

  assert.deepEqual(harness.stoppedSlices, [])
  assert.equal(harness.footers.at(-1)?.tone, "error")
  assert.equal(harness.footers.at(-1)?.message, "cannot stop slice linux-dev; move or end attached agents first: agent-build,agent-review")
})

test("slice command delete blocks slices with attached agents", async () => {
  const harness = sliceHarness({
    slices: [slice({
      id: "slice-1",
      name: "linux-dev",
      agent_ids: ["agent-build"],
    })],
  })

  await handleSliceSlashCommand(harness.deps, command("delete", "slice-1"))

  assert.deepEqual(harness.deletedSlices, [])
  assert.equal(harness.footers.at(-1)?.tone, "error")
  assert.equal(harness.footers.at(-1)?.message, "cannot delete slice linux-dev; move or end attached agents first: agent-build")
})

function command(...args: string[]) {
  return { kind: "slice" as const, args, raw: `/slice ${args.join(" ")}` }
}

function sliceHarness(options: {
  readonly slices?: SliceRecord[]
  readonly focusedAgent?: Partial<AgentInstance>
  readonly endpoint?: SliceDisplayEndpoint
  readonly importedAuthStatus?: string
  readonly removedAuthStatus?: string
} = {}) {
  const notices: string[] = []
  const footers: Array<{ message: string; tone: "info" | "error" }> = []
  const createdSlices: Array<Parameters<NonNullable<SliceCommandHandlerDeps["createSlice"]>>[0]> = []
  const displayEndpointRefs: string[] = []
  const openedUrls: string[] = []
  const importedAuth: Array<{ sliceRef: string; provider: string; accountProfile: string }> = []
  const removedAuth: Array<{ sliceRef: string; provider: string; accountProfile: string }> = []
  const startedAuthLogins: Array<{ sliceRef: string; provider: string; accountProfile: string }> = []
  const stoppedSlices: string[] = []
  const deletedSlices: string[] = []
  const logRequests: Array<{ sliceRef: string; tailLines: number | null | undefined }> = []
  const auditRequests: Array<{ sliceRef: string; limit: number | null | undefined }> = []
  const savedStates: Array<{ sliceRef: string; mode: string | null | undefined; scope: string | null | undefined }> = []
  const stateStatusRequests: string[] = []
  const resetStates: string[] = []
  const backups: Array<{ sliceRef: string; name: string | null | undefined }> = []
  const restoredBackups: Array<{ sliceRef: string; backupRef: string }> = []
  const slices = options.slices ?? []
  const endpoint = options.endpoint ?? { slice_id: "slice-1", kind: "novnc", url: "http://slice.local", access: "local" }
  const focusedAgent = agent(options.focusedAgent)
  const deps: SliceCommandHandlerDeps = {
    currentWorkspaceTarget: () => "/repo",
    currentWorktreeTarget: () => "/repo/wt",
    focusedAgentId: () => focusedAgent.id,
    resolveSessionAgent: () => ({ agent: focusedAgent, error: null }),
    flashFooter: (message, tone) => { footers.push({ message, tone }) },
    appendNotice: (message) => { notices.push(message) },
    openExternalUrl: async (url) => {
      openedUrls.push(url)
      return true
    },
    listSlices: async () => slices,
    createSlice: async (createOptions) => {
      createdSlices.push(createOptions)
      return slice({
        id: "slice-created",
        name: createOptions.name,
        ...(createOptions.displayMode ? { display_mode: createOptions.displayMode } : {}),
      })
    },
    getSlice: async (sliceRef) => slices.find((entry) => entry.id === sliceRef || entry.name === sliceRef) ?? slice({ id: sliceRef, name: sliceRef }),
    startSlice: async (sliceRef) => slice({ id: sliceRef, name: sliceRef, status: "running" }),
    stopSlice: async (sliceRef) => {
      stoppedSlices.push(sliceRef)
      return slice({ id: sliceRef, name: sliceRef, status: "stopped" })
    },
    deleteSlice: async (sliceRef) => {
      deletedSlices.push(sliceRef)
      return slice({ id: sliceRef, name: sliceRef })
    },
    importSliceProviderAuth: async (sliceRef, provider, accountProfile) => {
      importedAuth.push({ sliceRef, provider, accountProfile })
      return { slice: slice({ id: sliceRef, name: sliceRef }), provider, status: options.importedAuthStatus ?? "imported" }
    },
    removeSliceProviderAuth: async (sliceRef, provider, accountProfile) => {
      removedAuth.push({ sliceRef, provider, accountProfile })
      return { slice: slice({ id: sliceRef, name: sliceRef }), provider, status: options.removedAuthStatus ?? "removed" }
    },
    startSliceProviderLogin: async (sliceRef, provider, accountProfile) => {
      startedAuthLogins.push({ sliceRef, provider, accountProfile })
      return {
        slice: slice({ id: sliceRef, name: sliceRef }),
        login: {
          provider,
          login_kind: "device",
          verification_url: "https://auth.example",
          user_code: "ABCD-EFGH",
          status: "started",
          message: "Open https://auth.example and enter ABCD-EFGH",
        },
      }
    },
    getSliceDisplayEndpoint: async (sliceRef) => {
      displayEndpointRefs.push(sliceRef)
      return endpoint
    },
    getSliceLogs: async (sliceRef, tailLines) => {
      logRequests.push({ sliceRef, tailLines })
      return {
        slice: slice({ id: "slice-1", name: sliceRef }),
        entries: [{
          source: "provision",
          path: "/tmp/slice.log",
          text: "slice booted",
          truncated: true,
        }],
      }
    },
    listSliceAudit: async (sliceRef, limit) => {
      auditRequests.push({ sliceRef, limit })
      return [
        {
          sequence: 1,
          event_id: "state_evt_1",
          kind: "slice.audit",
          subject_id: "slice-1",
          timestamp_ms: Date.parse("2026-01-02T03:04:05.000Z"),
          payload: {
            slice_id: "slice-1",
            slice_name: sliceRef,
            action: "auth.import",
            outcome: "completed",
            provider: "codex",
            status: "running",
            backend: "local_docker",
            display_mode: "headless",
            worktree_id: "/repo/wt",
            session_ids: ["session-1", "session-2"],
            agent_ids: ["agent-1"],
            worker_kernel_id: "kernel-slice",
            worker_machine_id: "machine-slice",
          },
        },
        {
          sequence: 2,
          event_id: "state_evt_2",
          kind: "slice.audit",
          subject_id: "slice-1",
          timestamp_ms: Date.parse("2026-01-02T03:04:06.000Z"),
          payload: {
            slice_id: "slice-1",
            slice_name: sliceRef,
            action: "auth.login",
            outcome: "failed",
            provider: "opencode",
            message: "login failed",
            status: "running",
            backend: "local_docker",
            display_mode: "headless",
            worktree_id: "/repo/wt",
            session_ids: ["session-1", "session-2"],
            agent_ids: ["agent-1"],
            worker_kernel_id: "kernel-slice",
            worker_machine_id: "machine-slice",
          },
        },
      ]
    },
    saveSliceState: async (sliceRef, mode, scope) => {
      savedStates.push({ sliceRef, mode, scope })
      return { slice: slice({ id: sliceRef, name: sliceRef, saved_state_status: "saved" }), state: savedState({ source_slice_id: sliceRef }) }
    },
    getSliceStateStatus: async (sliceRef) => {
      stateStatusRequests.push(sliceRef)
      return { slice: slice({ id: sliceRef, name: sliceRef, saved_state_status: "saved" }), state: savedState({ source_slice_id: sliceRef }) }
    },
    resetSliceState: async (sliceRef) => {
      resetStates.push(sliceRef)
      return { slice: slice({ id: sliceRef, name: sliceRef, saved_state_status: null }), removed_state: savedState({ source_slice_id: sliceRef }) }
    },
    createSliceBackup: async (sliceRef, name) => {
      backups.push({ sliceRef, name })
      return {
        slice: slice({ id: sliceRef, name: sliceRef }),
        backup: backup({ source_slice_id: sliceRef, name: name ?? "backup-1" }),
        instructions: "Use this backup by swapping slice state directories.",
      }
    },
    restoreSliceBackup: async (sliceRef, backupRef) => {
      restoredBackups.push({ sliceRef, backupRef })
      return {
        slice: slice({ id: sliceRef, name: sliceRef, status: "stopped" }),
        backup: backup({ source_slice_id: sliceRef, id: backupRef }),
      }
    },
  }
  return { deps, notices, footers, createdSlices, displayEndpointRefs, openedUrls, importedAuth, removedAuth, startedAuthLogins, stoppedSlices, deletedSlices, logRequests, auditRequests, savedStates, stateStatusRequests, resetStates, backups, restoredBackups }
}

function slice(overrides: Partial<SliceRecord> = {}): SliceRecord {
  return {
    id: "slice-1",
    name: "slice-1",
    owner_kernel_id: "kernel-local",
    owner_machine_id: "machine-local",
    backend: "local_docker",
    os: "linux",
    status: "running",
    workspace_mount: null,
    workspace_id: null,
    worktree_id: null,
    session_ids: [],
    agent_ids: [],
    display_mode: "headless",
    worker_kernel_ref: "slice:slice-1",
    worker_kernel_id: "kernel-slice",
    worker_machine_id: "machine-slice",
    relay_endpoint: null,
    providers: [],
    provider_auth: [],
    display_endpoint: null,
    created_at_ms: 0,
    updated_at_ms: 0,
    ...overrides,
  }
}

function savedState(overrides: Partial<SliceSavedStateRecord> = {}): SliceSavedStateRecord {
  return {
    id: "state-1",
    slice_name: "slice-1",
    source_slice_id: "slice-1",
    backend: "local_docker",
    os: "linux",
    image_ref: "chariox-slice-state:slice-1",
    home_archive_path: "/tmp/slice-home.tar.zst",
    created_at_ms: 0,
    updated_at_ms: 0,
    ...overrides,
  }
}

function backup(overrides: Partial<SliceBackupRecord> = {}): SliceBackupRecord {
  return {
    id: "backup-1",
    name: "backup-1",
    source_slice_id: "slice-1",
    source_state_id: "state-1",
    image_ref: "chariox-slice-backup:backup-1",
    home_archive_path: "/tmp/slice-home-backup.tar.zst",
    created_at_ms: 0,
    ...overrides,
  }
}

function agent(overrides: Partial<AgentInstance> = {}): AgentInstance {
  return {
    id: "agent-1",
    agent_ref: "agent-1",
    session_id: "session-1",
    alias: null,
    provider: "codex",
    model: "codex/gpt-5",
    effort: "high",
    worktree_id: "/repo/wt",
    state: "Idle",
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 0,
    last_activity_at_ms: 0,
    ...overrides,
  }
}
