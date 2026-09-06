import assert from "node:assert/strict"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import type {
  AgentInstance,
  CharioxMcpServerConfig,
  CharioxSkillMetadata,
  ProviderProcessInfo,
  WorkspaceLinkDefinition,
} from "./kernel-types.js"
import { applyShellCommandResult, createDefaultShellContext, parseShellCommand } from "./shell-core.js"
import { executeShellCommand } from "./shell-executor.js"
import {
  daemonHealth,
  fakeClient,
  makeAgent,
  makeSession,
  makeWorkflow,
  makeWorkflowPublication,
  makeWorkflowRun,
  makeWorkflowWatchdog,
} from "./shell-executor.test-support.js"

test("executeShellCommand handles shell-local context mutations", async () => {
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("set model gpt-5.3"), context, { client: fakeClient(() => ({})).client })
  assert.equal(result.ok, true)
  assert.deepEqual(result.contextUpdates, { model: "gpt-5.3" })
  const next = applyShellCommandResult(context, result)
  assert.equal(next.model, "gpt-5.3")
})

test("executeShellCommand help advertises workspace live sync config values", async () => {
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("help"), context, { client: fakeClient(() => ({})).client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /session list\|status\|new\|attach\|use\|members\|invites\|invite\|join\|revoke-invite\|mode\|permissions/)
  assert.match(result.message ?? "", /kernel health\|status\|remote-runtime\|runtime\|debug-bundle \[label\]\|delete/)
  assert.match(result.message ?? "", /agent list\|spawn \[--count <n>\]\|focus\|inspect\|cycle\|mode\|permissions\|substitute/)
  assert.match(result.message ?? "", /config show\|path\|keys\|schema\|set\|unset\|workspace-live-sync off\|managed\|tracked/)
  assert.match(result.message ?? "", /extension import providers\|grant\|revoke\|grants\|sync-status\|sync-retry\|audit/)
  assert.match(result.message ?? "", /workspace sync status\|doctor\|targets\|conflicts\|ignore\|audit\|off\|managed\|tracked\|default\|link/)
  assert.match(result.message ?? "", /slice list\|create\|status\|doctor\|logs\|audit\|state\|save-state\|backup\|reset-state\|start\|stop\|delete\|auth import\|auth remove\|auth login\|screen/)
  assert.match(result.message ?? "", /slice auth import copies a selected provider account into the slice; auth login starts provider login inside the slice; auth remove purges the selected slice-local account/)
  assert.match(result.message ?? "", /provider status\|login\|setup-token\|logout\|reauth\|processes \[provider\]\|processes teardown <provider>/)
})

test("executeShellCommand exports session-scoped kernel debug bundle", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
  })
  const fake = fakeClient((request) => {
    assert.deepEqual(request, {
      ExportDebugBundle: {
        session_id: "session-1",
        bundle_label: "glitch",
        limit: null,
      },
    })
    return {
      DebugBundleExported: {
        bundle_dir: "/kernel/logs/debug-bundles/session-1-glitch",
        manifest_path: "/kernel/logs/debug-bundles/session-1-glitch/manifest.json",
        logs_path: "/kernel/logs/debug-bundles/session-1-glitch/logs.ndjson",
        log_root: "/kernel/logs",
        record_count: 12,
        limit: 1000,
      },
    }
  })

  const result = await executeShellCommand(parseShellCommand("kernel debug-bundle glitch"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /kernel debug bundle exported on kernel machine: \/kernel\/logs\/debug-bundles\/session-1-glitch \(12\/1000 records\)/)
  assert.equal(fake.requests.length, 1)
})

test("executeShellCommand requires an active session for kernel debug bundle", async () => {
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const fake = fakeClient(() => {
    throw new Error("kernel should not be called without a session")
  })

  const result = await executeShellCommand(parseShellCommand("kernel debug-bundle"), context, { client: fake.client })

  assert.equal(result.ok, false)
  assert.match(result.message ?? "", /requires an active session/)
  assert.equal(fake.requests.length, 0)
})

test("executeShellCommand renders kernel health diagnostics", async () => {
  const baseHealth = daemonHealth()
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { GetDaemonHealth: null })
    return {
      DaemonHealth: {
        projection: daemonHealth({
          provider_runs: {
            ...baseHealth.provider_runs,
            projected_runs: 2,
            active_runs: 2,
            chariox_active_runs: 2,
            duplicate_chariox_agent_bindings: [{
              session_id: "session-1",
              agent_id: "agent-1",
              provider_run_ids: ["run-1", "run-2"],
            }],
            duplicate_native_tui_agent_bindings: [],
          },
          remote_execution: {
            ...baseHealth.remote_execution,
            remote_agents: 1,
            active_remote_agents: 1,
            missing_active_worker_runs: 1,
            issues: [{
              kind: "missing_active_worker_provider_run",
              session_id: "session-1",
              agent_id: "agent-remote",
              agent_ref: "agent-remote",
              worker_kernel_id: "worker-kernel",
              worker_machine_id: "worker-machine",
              execution_lease_id: "lease-1",
              leased_agent_id: "leased-agent-1",
              state: "working",
              is_processing: true,
              details: "active remote agent has no worker run",
            }],
          },
          workspace_live_sync: {
            ...baseHealth.workspace_live_sync,
            managed_mode: {
              write_fence_supported: false,
              write_fence_backend: null,
              unavailable_reason: "managed mode needs selective write fencing",
            },
          },
        }),
      },
    }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("kernel health"), context, { client: fake.client })

  assert.equal(result.ok, false)
  assert.match(result.message ?? "", /^kernel health/)
  assert.match(result.message ?? "", /command lanes: session=0\/0 agent=0\/0 workflow=0\/0 provider=0\/0 saturated=0/)
  assert.match(result.message ?? "", /provider runs: projected=2 active=2 chariox=2 native_tui=0/)
  assert.match(result.message ?? "", /remote execution: remote_agents=1 active=1 missing_worker_runs=1 malformed=0/)
  assert.match(result.message ?? "", /duplicate Chariox provider run bindings:/)
  assert.match(result.message ?? "", /session=session-1 agent=agent-1 runs=run-1,run-2/)
  assert.match(result.message ?? "", /remote execution issues: missing_worker_runs=1 malformed=0/)
  assert.match(result.message ?? "", /agent=agent-remote \(agent-remote\) session=session-1 worker=worker-kernel\/worker-machine lease=lease-1 leased_agent=leased-agent-1 state=working processing=yes kind=missing_active_worker_provider_run: active remote agent has no worker run/)
  assert.match(result.message ?? "", /next: run \/kernel remote-runtime; run \/agent inspect agent-remote; run \/machine kernels worker-machine; reconnect or relaunch the remote\/slice worker/)
  assert.match(result.message ?? "", /remote runtime affected: agents=agent-remote/)
  assert.match(result.message ?? "", /workspace live sync scope: selected workspace\/worktree only; other repositories unrestricted/)
  assert.match(result.message ?? "", /workspace live sync managed capability: unavailable \(managed mode needs selective write fencing\); tracked\/off modes unaffected/)
  assert.doesNotMatch(result.message ?? "", /next: select tracked mode on this worker or run the managed provider on a supported host/)
})

test("executeShellCommand renders generic slice provider auth recovery without placeholders", async () => {
  const baseHealth = daemonHealth()
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { GetDaemonHealth: null })
    return {
      DaemonHealth: {
        projection: daemonHealth({
          slice_lifecycle: {
            ...baseHealth.slice_lifecycle,
            total_slices: 1,
            running_slices: 1,
            provider_auth_missing_slices: 1,
            provider_auth_issues: [{
              slice_id: "slice-1",
              name: "dev",
              status: "running",
              session_ids: ["session-1"],
              agent_ids: ["agent-1"],
              worktree_id: "/repo",
              provider: "",
              provider_auth_state: "",
              alias: null,
              identity: null,
              details: "slice provider account needs login or import",
            }],
          },
        }),
      },
    }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("kernel health"), context, { client: fake.client })

  assert.equal(result.ok, false)
  assert.match(result.message ?? "", /slice provider auth issues: missing=1 unconfigured=0/)
  assert.match(result.message ?? "", /slice=dev \(slice-1\) status=running worktree=\/repo agents=agent-1: slice provider account needs login or import/)
  assert.match(result.message ?? "", /next: run \/slice doctor slice-1; inspect \/slice audit slice-1; after provider discovery, use the matching \/slice auth login or \/slice auth import command before sending prompts to agents in that slice/)
  assert.doesNotMatch(result.message ?? "", /open Slices and choose/)
  assert.doesNotMatch(result.message ?? "", /<provider>|provider-specific/)
})

test("executeShellCommand renders aggregate slice provider auth recovery without placeholders", async () => {
  const baseHealth = daemonHealth()
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { GetDaemonHealth: null })
    return {
      DaemonHealth: {
        projection: daemonHealth({
          slice_lifecycle: {
            ...baseHealth.slice_lifecycle,
            total_slices: 2,
            running_slices: 2,
            provider_auth_missing_slices: 1,
            provider_auth_unconfigured_slices: 1,
            provider_auth_issues: [],
          },
        }),
      },
    }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("kernel health"), context, { client: fake.client })

  assert.equal(result.ok, false)
  assert.match(result.message ?? "", /slice provider auth issues: missing=1 unconfigured=1/)
  assert.match(result.message ?? "", /next: run \/slice list to identify affected slices; run \/slice doctor and inspect \/slice audit before choosing a provider account to login or import/)
  assert.doesNotMatch(result.message ?? "", /<provider>|provider-specific/)
})

test("executeShellCommand accepts kernel remote runtime aliases", async () => {
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { GetDaemonHealth: null })
    return { DaemonHealth: { projection: daemonHealth() } }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const remoteRuntime = await executeShellCommand(parseShellCommand("kernel remote-runtime"), context, { client: fake.client })
  const runtime = await executeShellCommand(parseShellCommand("kernel runtime"), context, { client: fake.client })

  assert.equal(remoteRuntime.ok, true)
  assert.match(remoteRuntime.message ?? "", /^remote runtime/)
  assert.match(remoteRuntime.message ?? "", /remote runtime authority: home kernel owns sessions, prompts, grants, and live sync; workers execute leased provider runs and projected tools/)
  assert.match(remoteRuntime.message ?? "", /provider runs: projected=0 active=0 chariox=0 native_tui=0/)
  assert.match(remoteRuntime.message ?? "", /remote execution: remote_agents=0 active=0 missing_worker_runs=0 malformed=0/)
  assert.match(remoteRuntime.message ?? "", /remote extensions: remote_agents=0 home_proxy_agents=0 grants=0 synced=0 syncing=0 pending=0 failed=0 stale=0 missing=0 pending_revoke=0/)
  assert.equal(runtime.ok, true)
  assert.match(runtime.message ?? "", /workspace live sync:/)
  assert.match(runtime.message ?? "", /workspace live sync scope: selected workspace\/worktree only; other repositories unrestricted/)
})

test("executeShellCommand reports degraded remote runtime attention", async () => {
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { GetDaemonHealth: null })
    return {
      DaemonHealth: {
        projection: daemonHealth({
          remote_extension_sync: {
            remote_agents: 2,
            home_proxy_agents: 2,
            home_proxy_grants: 3,
            manifest_missing_agents: 0,
            synced_agents: 0,
            syncing_agents: 1,
            pending_agents: 1,
            failed_agents: 0,
            stale_agents: 0,
            pending_revoke_agents: 0,
            issues: [],
          },
        }),
      },
    }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("kernel remote-runtime"), context, { client: fake.client })

  assert.equal(result.ok, false)
  assert.match(result.message ?? "", /^remote runtime/)
  assert.match(result.message ?? "", /remote extension sync settling: syncing=1 pending=1/)
  assert.match(result.message ?? "", /next: home keeps stale home-proxy calls blocked until worker manifests settle; run \/kernel remote-runtime and then \/extension sync-status for the affected agent before retrying sync/)
  assert.match(result.message ?? "", /remote runtime readiness: degraded \(2 attention\)/)
  assert.match(result.message ?? "", /remote runtime readiness next: run \/extension sync-status for affected agents; use \/extension sync-retry after worker connectivity is healthy/)
  assert.doesNotMatch(result.message ?? "", /open Extensions|<agent>/)
  assert.doesNotMatch(result.message ?? "", /support bundle:/)
})

test("executeShellCommand renders aggregate remote extension recovery without web-only actions", async () => {
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { GetDaemonHealth: null })
    return {
      DaemonHealth: {
        projection: daemonHealth({
          remote_extension_sync: {
            remote_agents: 1,
            home_proxy_agents: 1,
            home_proxy_grants: 1,
            manifest_missing_agents: 0,
            synced_agents: 0,
            syncing_agents: 0,
            pending_agents: 0,
            failed_agents: 1,
            stale_agents: 0,
            pending_revoke_agents: 1,
            issues: [],
          },
        }),
      },
    }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("kernel remote-runtime"), context, { client: fake.client })

  assert.equal(result.ok, false)
  assert.match(result.message ?? "", /remote extension sync issues: failed=1 stale=0 missing=0 pending_revoke=1/)
  assert.match(result.message ?? "", /next: keep the home revoke in place; run \/kernel remote-runtime to identify affected agents, then use \/extension sync-status and \/extension sync-retry after the worker reconnects/)
  assert.doesNotMatch(result.message ?? "", /open Extensions|<agent>/)
})

test("executeShellCommand summarizes affected remote runtime targets", async () => {
  const baseHealth = daemonHealth()
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { GetDaemonHealth: null })
    return {
      DaemonHealth: {
        projection: daemonHealth({
          remote_extension_sync: {
            ...baseHealth.remote_extension_sync,
            remote_agents: 1,
            home_proxy_agents: 1,
            home_proxy_grants: 1,
            failed_agents: 1,
            issues: [{
              session_id: "session-1",
              agent_id: "agent-remote-id",
              agent_ref: "agent-remote",
              worker_kernel_id: "worker-kernel",
              worker_machine_id: "worker-machine",
              execution_lease_id: "lease-1",
              leased_agent_id: "leased-agent-1",
              active_worker_provider_run_id: "worker-run-1",
              state: "failed",
              manifest_hash: "abcdef1234567890",
              last_error: "worker offline",
              pending_revoke: false,
              home_proxy_grants: ["script:home-tool"],
              worktree_id: "/repo/worktree-a",
            }],
          },
          slice_lifecycle: {
            ...baseHealth.slice_lifecycle,
            provider_auth_missing_slices: 1,
            provider_auth_issues: [{
              slice_id: "slice-1",
              name: "slice-dev",
              status: "running",
              session_ids: ["session-1"],
              agent_ids: ["agent-slice"],
              worktree_id: "/repo/worktree-b",
              provider: "codex",
              provider_auth_state: "not_configured",
              alias: null,
              identity: null,
              details: "codex auth missing",
            }],
          },
          workspace_live_sync: {
            ...baseHealth.workspace_live_sync,
            workspace_identity: {
              ...baseHealth.workspace_live_sync.workspace_identity,
              identity_changed_provider_runs: 1,
              issues: [{
                provider_run_id: "provider-run-1",
                root: "/repo/synced",
                generation: 2,
                valid: false,
                baseline_fingerprint: "base",
                current_fingerprint: "current",
              }],
            },
          },
        }),
      },
    }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("kernel remote-runtime"), context, { client: fake.client })

  assert.equal(result.ok, false)
  assert.match(result.message ?? "", /remote runtime readiness: blocked/)
  assert.match(
    result.message ?? "",
    /remote runtime affected: agents=agent-remote,agent-slice worktrees=\/repo\/worktree-a,\/repo\/worktree-b roots=\/repo\/synced/,
  )
  assert.match(result.message ?? "", /remote runtime readiness next: run \/workspace sync status, \/workspace sync targets, and \/workspace sync conflicts/)
})

test("executeShellCommand renders attached session runtime context before kernel health", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    agentId: "agent-1",
  })
  const fake = fakeClient((request) => {
    if ("GetDaemonHealth" in request) {
      return { DaemonHealth: { projection: daemonHealth() } }
    }
    assert.deepEqual(request, { GetSessionState: { session_id: "session-1" } })
    return {
      SessionState: {
        session: makeSession({
          id: "session-1",
          host_daemon_id: "home-kernel-1",
          host_machine_id: "home-machine-1",
          owner_user_id: "user-1",
        }),
        agent_activity: {},
      },
    }
  })

  const result = await executeShellCommand(parseShellCommand("kernel health"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /^session runtime:\n  session: session-1\n  home kernel: home-kernel-1@home-machine-1\n  owner: user-1\n  authority: home owns sessions, prompts, grants, and live sync; workers execute leases and projected tools\n  agent: agent-1\nkernel health/)
  assert.deepEqual(fake.requests, [
    { GetDaemonHealth: null },
    { GetSessionState: { session_id: "session-1" } },
  ])
})

test("executeShellCommand keeps kernel health available when session runtime lookup fails", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-missing",
  })
  const fake = fakeClient((request) => {
    if ("GetDaemonHealth" in request) {
      return { DaemonHealth: { projection: daemonHealth() } }
    }
    throw new Error("session not found")
  })

  const result = await executeShellCommand(parseShellCommand("kernel health"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /^session runtime:\n  session: session-missing\n  home kernel: unknown\n  authority: unknown until session state is available\n  lookup: session not found\nkernel health/)
})

test("executeShellCommand renders shell-local context and pwd", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo/worktree",
    sessionId: "session-1",
    attachmentId: "attach-1",
    agentId: "agent-1",
    workflowId: "workflow-1",
    provider: "codex",
    model: "gpt-5.2",
    effort: "low",
    variables: { wf: "workflow-1" },
  })
  const fake = fakeClient((request) => {
    if ("GetSessionState" in request) {
      return {
        SessionState: {
          session: makeSession({
            host_daemon_id: "home-kernel-1",
            host_machine_id: "home-machine-1",
            owner_user_id: "user-1",
            workspace_live_sync_mode: "managed",
            active_provider_run_id: "session-run-1",
            agents: [makeAgent({
              id: "agent-1",
              agent_ref: "agent-1",
              remote_execution: {
                worker_kernel_id: "slice-kernel",
                worker_machine_id: "slice-machine",
                execution_lease_id: "lease-1",
                leased_agent_id: "leased-agent-1",
                active_worker_provider_run_id: "worker-run-1",
              },
              extension_grants: [
                { kind: "script", name: "deploy" },
                { kind: "skill", name: "review" },
              ],
              remote_extension_manifest_sync: {
                state: "stale",
                manifest_hash: "abcdef1234567890",
                last_error: "worker behind",
              },
            })],
            prompt_states: {
              "agent-1": {
                active_prompt: {
                  id: "prompt-1",
                  source_attachment_id: "attach-1",
                  target_agent_id: "agent-1",
                  prompt: "hi",
                  status: "Running",
                },
                queued_prompts: [],
              },
            },
          }),
          agent_activity: {
            "agent-1": {
              status: "working",
              prompt_status: "running",
              busy: true,
            },
          },
        },
      }
    }
    if ("ListSlices" in request) {
      return {
        SlicesListed: {
          slices: [{
            id: "slice-1",
            name: "devbox",
            owner_kernel_id: "home-kernel-1",
            owner_machine_id: "home-machine-1",
            backend: "local_docker",
            os: "linux",
            status: "running",
            worker_kernel_ref: "slice-kernel",
            worker_kernel_id: "slice-kernel",
            worker_machine_id: "slice-machine",
            agent_ids: ["agent-1"],
            created_at_ms: 0,
            updated_at_ms: 0,
          }],
        },
      }
    }
    if ("GetProviderRun" in request) {
      return { ProviderRun: { provider_run: { id: "session-run-1", agent_instance_id: "agent-1" } } }
    }
    return {}
  })
  const contextResult = await executeShellCommand(parseShellCommand("context"), context, { client: fake.client })
  const pwdResult = await executeShellCommand(parseShellCommand("pwd"), context, { client: fake.client })

  assert.equal(contextResult.ok, true)
  assert.match(contextResult.message ?? "", /workspace: \/repo/)
  assert.match(contextResult.message ?? "", /worktree: \/repo\/worktree/)
  assert.match(contextResult.message ?? "", /session: session-1/)
  assert.match(contextResult.message ?? "", /home kernel: home-kernel-1@home-machine-1/)
  assert.match(contextResult.message ?? "", /session owner: user-1/)
  assert.match(contextResult.message ?? "", /runtime authority: home owns sessions, prompts, grants, and live sync; workers execute leases and projected tools/)
  assert.match(contextResult.message ?? "", /workspace live sync: managed \(selected workspace\/worktree only; other repositories unrestricted\)/)
  assert.match(contextResult.message ?? "", /agent: agent-1 \(busy\)/)
  assert.match(contextResult.message ?? "", /agent placement: slice devbox \(worker=slice-machine, kernel=slice-kernel, lease=lease-1, leased_agent=leased-agent-1, active_run=worker-run-1\)/)
  assert.match(contextResult.message ?? "", /provider run: session=session-run-1, worker=worker-run-1/)
  assert.match(contextResult.message ?? "", /extensions: 2 grants \(active tools home-proxy; skills snapshot; script=1, skill=1\)/)
  assert.match(contextResult.message ?? "", /extension runtime: home-proxy tools execute on home with home-owned grants and credentials; skills are passive snapshots/)
  assert.match(contextResult.message ?? "", /extension boundary: home validates every call; credentials never leave home/)
  assert.match(contextResult.message ?? "", /remote extension sync: stale, hash=abcdef123456, error=worker behind/)
  assert.match(contextResult.message ?? "", /remote extension next: home keeps stale home-proxy calls blocked; run \/extension sync-status agent-1; run \/machine kernels slice-machine; use \/extension sync-retry agent-1/)
  assert.match(contextResult.message ?? "", /workflow: workflow-1/)
  assert.match(contextResult.message ?? "", /provider: codex/)
  assert.match(contextResult.message ?? "", /\$wf = workflow-1/)
  assert.equal(pwdResult.message, "/repo/worktree")
  assert.equal(fake.requests.length, 3)
})

test("executeShellCommand context uses projected idle over stale legacy prompt state", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    attachmentId: "attach-1",
    agentId: "agent-1",
  })
  const fake = fakeClient((request) => {
    if ("GetSessionState" in request) {
      return {
        SessionState: {
          session: makeSession({
            agents: [makeAgent({
              id: "agent-1",
              agent_ref: "agent-1",
              state: "Working",
              is_processing: true,
            })],
            prompt_states: {
              "agent-1": {
                active_prompt: {
                  id: "prompt-1",
                  source_attachment_id: "attach-1",
                  target_agent_id: "agent-1",
                  prompt: "stale",
                  status: "Running",
                },
                queued_prompts: [],
              },
            },
          }),
          agent_activity: {
            "agent-1": {
              status: "idle",
              prompt_status: "none",
              busy: false,
            },
          },
        },
      }
    }
    return {}
  })

  const result = await executeShellCommand(parseShellCommand("context"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /agent: agent-1/)
  assert.doesNotMatch(result.message ?? "", /agent: agent-1 \(busy\)/)
})

test("executeShellCommand context uses projected idle over stale remote worker state", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    attachmentId: "attach-1",
    agentId: "agent-remote",
  })
  const fake = fakeClient((request) => {
    if ("GetSessionState" in request) {
      return {
        SessionState: {
          session: makeSession({
            agents: [makeAgent({
              id: "agent-remote",
              agent_ref: "agent-remote",
              state: "Working",
              is_processing: true,
              remote_execution: {
                worker_kernel_id: "worker-kernel",
                worker_machine_id: "hetzner",
                execution_lease_id: "lease-1",
                leased_agent_id: "leased-agent-1",
              },
            })],
          }),
          agent_activity: {
            "agent-remote": {
              status: "idle",
              prompt_status: "none",
              busy: false,
            },
          },
        },
      }
    }
    if ("ListSlices" in request) {
      return { SlicesListed: { slices: [] } }
    }
    return {}
  })

  const result = await executeShellCommand(parseShellCommand("context"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /agent: agent-remote/)
  assert.doesNotMatch(result.message ?? "", /agent: agent-remote \(busy\)/)
  assert.doesNotMatch(result.message ?? "", /provider run next:/)
})

test("executeShellCommand context keeps final revokes visible after grants are gone", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo/worktree",
    sessionId: "session-1",
    agentId: "agent-1",
  })
  const fake = fakeClient((request) => {
    if ("GetSessionState" in request) {
      return {
        SessionState: {
          session: makeSession({
            agents: [makeAgent({
              id: "agent-1",
              agent_ref: "agent-1",
              remote_execution: {
                worker_kernel_id: "worker-1",
                worker_machine_id: "machine-1",
                execution_lease_id: "lease-1",
                leased_agent_id: "leased-agent-1",
              },
              extension_grants: [],
              remote_extension_manifest_sync: {
                state: "failed",
                manifest_hash: "empty-hash",
                pending_revoke: true,
                last_error: "worker offline",
              },
            })],
          }),
          agent_activity: {},
        },
      }
    }
    if ("ListSlices" in request) {
      return { SlicesListed: { slices: [] } }
    }
    return {}
  })

  const result = await executeShellCommand(parseShellCommand("context"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /extensions: none \(final revoke pending\)/)
  assert.match(result.message ?? "", /remote extension sync: failed, pending revoke, hash=empty-hash, error=worker offline/)
  assert.match(result.message ?? "", /remote extension next: keep the home revoke in place; run \/extension sync-status agent-1; run \/machine kernels machine-1 if the revoke stays pending; use \/extension sync-retry agent-1 after the worker reconnects/)
})

test("executeShellCommand context keeps home machine visible without daemon id", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
  })
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { GetSessionState: { session_id: "session-1" } })
    return {
      SessionState: {
        session: makeSession({
          host_machine_id: "home-machine-1",
        }),
      },
    }
  })

  const result = await executeShellCommand(parseShellCommand("context"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /home kernel: home-machine-1/)
  assert.match(result.message ?? "", /session owner: -/)
  assert.match(result.message ?? "", /runtime authority: home owns sessions, prompts, grants, and live sync; workers execute leases and projected tools/)
})

test("executeShellCommand does not infer provider run ownership from focused agent", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    agentId: "agent-1",
  })
  const fake = fakeClient((request) => {
    if ("GetSessionState" in request) {
      return {
        SessionState: {
          session: makeSession({
            active_provider_run_id: "session-run-2",
            focused_agent_id: "agent-1",
            agents: [
              makeAgent({ id: "agent-1", agent_ref: "agent-1" }),
              makeAgent({ id: "agent-2", agent_ref: "agent-2" }),
            ],
          }),
        },
      }
    }
    if ("GetProviderRun" in request) {
      return { ProviderRun: { provider_run: { id: "session-run-2", agent_instance_id: "agent-2" } } }
    }
    return {}
  })

  const result = await executeShellCommand(parseShellCommand("context"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /agent: agent-1/)
  assert.match(result.message ?? "", /provider run: session=session-run-2 owned_by=agent-2/)
  assert.match(result.message ?? "", /remote extension sync: not applicable \(worker-local agent; no home-proxy manifest\)/)
  assert.match(result.message ?? "", /provider run next: run \/kernel health and \/provider processes; export a debug bundle, then close or relaunch the mismatched provider run before sending more prompts to agent-1/)
})

test("executeShellCommand context reports missing active remote worker provider run", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    agentId: "agent-remote",
  })
  const agent = makeAgent({
    id: "agent-remote",
    agent_ref: "agent-remote",
    state: "Working",
    is_processing: true,
    remote_execution: {
      worker_kernel_id: "worker-kernel",
      worker_machine_id: "hetzner",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
    },
  })
  const fake = fakeClient((request) => {
    if ("GetSessionState" in request) {
      return {
        SessionState: {
          session: makeSession({
            focused_agent_id: "agent-remote",
            agents: [agent],
            agent_activity: {
              [agent.id]: {
                status: "working",
                prompt_status: "running",
                busy: true,
                unread_idle_output: false,
                active_turn: {
                  prompt_id: "prompt-remote",
                  status: "running",
                  phase: "streaming",
                },
              },
            },
            agent_activity_revision: 1,
          }),
        },
      }
    }
    if ("ListSlices" in request) {
      return { SlicesListed: { slices: [] } }
    }
    return {}
  })

  const result = await executeShellCommand(parseShellCommand("context"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /agent placement: remote \(worker=hetzner, kernel=worker-kernel, lease=lease-1, leased_agent=leased-agent-1\)/)
  assert.match(result.message ?? "", /provider run: none/)
  assert.match(result.message ?? "", /provider run next: run \/kernel remote-runtime and \/machine kernels hetzner; reconnect or relaunch the remote\/slice worker before sending prompts to that remote\/slice agent if no active worker run appears/)
})

test("executeShellCommand lists sessions with home kernel ownership", async () => {
  const sessions = [
    makeSession({
      id: "session-1",
      alias: "main",
      host_daemon_id: "home-kernel-1",
      host_machine_id: "home-machine-1",
      owner_user_id: "user-1",
      attachment_ids: ["attachment-1"],
      worktree_id: "/repo/main",
      workspace_live_sync_mode: "managed",
    }),
    makeSession({
      id: "session-2",
      alias: null,
      host_machine_id: "home-machine-2",
      owner_user_id: "user-2",
      attachment_ids: [],
      worktree_id: "/repo/feature",
      status: "Parked",
      workspace_live_sync_mode: "tracked",
      agents: [makeAgent({
        id: "agent-remote",
        agent_ref: "remote-1",
        state: "Working",
        remote_execution: {
          worker_kernel_id: "slice:slice-1",
          worker_machine_id: "worker-machine",
          execution_lease_id: "lease-1",
          leased_agent_id: "leased-agent-1",
        },
      })],
      agent_activity: {
        "agent-remote": {
          status: "working",
          prompt_status: "running",
          busy: true,
          unread_idle_output: false,
          active_turn: {
            prompt_id: "prompt-remote",
            status: "running",
            phase: "streaming",
          },
        },
      },
      agent_activity_revision: 1,
    }),
  ]
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { ListSessions: null })
    return { SessionsListed: { sessions } }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo/main", sessionId: "session-1" })
  const result = await executeShellCommand(parseShellCommand("session list"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /`main` \(`session-1`\) - running - 1 CLI - main - home home-kernel-1@home-machine-1 - owner user-1 - authority home-owned - live sync managed \(selected workspace\/worktree only; other repositories unrestricted\) current/)
  assert.match(result.message ?? "", /`session-2` - parked - 0 CLIs - feature - home home-machine-2 - owner user-2 - authority home-owned - live sync tracked \(selected workspace\/worktree only; other repositories unrestricted\) - remote 1 agent, 1 slice, 1 worker run gap - next run \/kernel remote-runtime; run \/agent inspect remote-1; run \/machine kernels worker-machine; reconnect or relaunch the remote\/slice worker/)
})

test("executeShellCommand shows current session runtime status", async () => {
  const remoteAgent = makeAgent({
    id: "agent-remote",
    agent_ref: "agent-remote",
    state: "Working",
    is_processing: true,
    remote_execution: {
      worker_kernel_id: "worker-kernel",
      worker_machine_id: "worker-machine",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
    },
    extension_grants: [{ kind: "script", name: "release" }],
    remote_extension_manifest_sync: {
      state: "stale",
      manifest_hash: "hash-1",
      pending_revoke: false,
      last_error: "worker offline",
    },
  })
  const session = makeSession({
    id: "session-1",
    alias: "main",
    host_daemon_id: "home-kernel",
    host_machine_id: "home-machine",
    owner_user_id: "alice",
    worktree_id: "/repo/main",
    workspace_live_sync_mode: "tracked",
    focused_agent_id: remoteAgent.id,
    agents: [remoteAgent],
    agent_activity: {
      [remoteAgent.id]: {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-remote",
          status: "running",
          phase: "streaming",
        },
      },
    },
    agent_activity_revision: 1,
  })
  const fake = fakeClient((request) => {
    if ("GetSessionState" in request) {
      assert.deepEqual(request, { GetSessionState: { session_id: "session-1" } })
      return { SessionState: { session } }
    }
    assert.deepEqual(request, { ListSlices: null })
    return { SlicesListed: { slices: [{
      id: "slice-1",
      name: "linux-dev",
      owner_kernel_id: "home-kernel",
      owner_machine_id: "home-machine",
      backend: "local_docker",
      os: "linux",
      status: "running",
      worktree_id: "/repo/main",
      worker_kernel_ref: "worker-kernel",
      worker_kernel_id: "worker-kernel",
      worker_machine_id: "worker-machine",
      agent_ids: ["agent-remote"],
      providers: ["opencode"],
      provider_auth: [{
        provider: "opencode",
        state: "authenticated",
        email: "daily@example.com",
        alias: "daily",
        source: "test",
      }],
      created_at_ms: 0,
      updated_at_ms: 0,
    }] } }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo/main", sessionId: "session-1" })

  const result = await executeShellCommand(parseShellCommand("session status"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /^session runtime/)
  assert.match(result.message ?? "", /session: main \(session-1\)/)
  assert.match(result.message ?? "", /home kernel: home-kernel@home-machine/)
  assert.match(result.message ?? "", /live sync: tracked \(selected workspace\/worktree only; other repositories unrestricted\)/)
  assert.match(result.message ?? "", /remote runtime: 1 agent, 1 worker, 1 slice, 1 worker run gap/)
  assert.match(result.message ?? "", /home-proxy extensions: 1 agent, 1 sync issue, 0 pending revokes/)
  assert.match(result.message ?? "", /agent runtime:\n  - agent-remote: Working opencode\/gpt-5\.2 worktree=\/repo placement=slice:linux-dev slice_status=running slice_worktree=\/repo\/main slice_auth=ready opencode slice_accounts=opencode=daily \(daily@example.com\) worker=worker-machine kernel=worker-kernel lease=lease-1 leased_agent=leased-agent-1 extensions=1 grant \(active tools home-proxy; script=1\) manifest=stale hash=hash-1 error=worker offline/)
  assert.match(result.message ?? "", /next: run \/kernel remote-runtime; run \/agent inspect agent-remote; run \/machine kernels worker-machine/)
  assert.match(result.message ?? "", /next: home keeps stale home-proxy calls blocked; run \/extension sync-status agent-remote; run \/machine kernels worker-machine; use \/extension sync-retry agent-remote after worker connectivity is healthy/)
})

test("executeShellCommand resolves session status refs before rendering", async () => {
  const session = makeSession({
    id: "session-2",
    alias: "release",
    host_daemon_id: "home-kernel",
    host_machine_id: "home-machine",
  })
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { ResolveSession: { session_ref: "release", workspace_id: "/repo" } })
    return { SessionResolved: { session } }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo/main", sessionId: "session-1" })

  const result = await executeShellCommand(parseShellCommand("session status release"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /session: release \(session-2\)/)
  assert.equal(fake.requests.length, 1)
})
