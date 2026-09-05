import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { createRequire } from "node:module"
import { mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"

const scriptRoot = path.dirname(fileURLToPath(import.meta.url))
const repositoryRoot = path.resolve(scriptRoot, "..")
const require = createRequire(path.join(repositoryRoot, "apps/cli/package.json"))
const { WebSocketServer } = require("ws")
const probe = path.join(
  repositoryRoot,
  "apps/kernel/slice-linux-docker/docker/managed-provider-isolation-probe.mjs",
)

test("provider isolation probe authenticates and launches Codex through the kernel", async (t) => {
  const workspace = await mkdtemp(path.join(os.tmpdir(), "chariox-managed-isolation-probe-"))
  const token = "local-kernel-auth-token"
  const requests = []
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise((resolve) => server.once("listening", resolve))
  t.after(async () => {
    await new Promise((resolve) => server.close(resolve))
    await rm(workspace, { recursive: true, force: true })
  })

  server.on("connection", (socket, request) => {
    assert.equal(request.headers.authorization, `Bearer ${token}`)
    socket.on("message", async (payload) => {
      const frame = JSON.parse(String(payload))
      assert.equal(frame.command_id, frame.request_id)
      requests.push(frame.request)
      if (frame.request.CreateSession) {
        socket.send(response(frame.request_id, {
          SessionCreated: {
            session: { id: "session-1" },
            agent: { id: "agent-1" },
          },
        }))
        return
      }
      if (frame.request.LaunchProviderRun) {
        await writeFile(
          path.join(workspace, ".chariox-managed-isolation-probe.result"),
          `managed_provider_isolation=ok\nreal_provider=/opt/chariox-toolchain/bin/codex\nworkspace=${workspace}\n`,
        )
        socket.send(response(frame.request_id, {
          ProviderRunLaunched: { provider_run: { id: "provider-run-1" } },
        }))
        return
      }
      if (frame.request.GetProviderRun) {
        socket.send(response(frame.request_id, {
          ProviderRun: {
            provider_run: { id: "provider-run-1", state: "running" },
          },
        }))
        return
      }
      socket.send(response(frame.request_id, { SessionEnded: { session_id: "session-1" } }))
    })
  })

  const address = server.address()
  assert.equal(typeof address, "object")
  const result = await runProbe({
    CHARIOX_KERNEL_URL: `ws://127.0.0.1:${address.port}`,
    CHARIOX_KERNEL_LOCAL_AUTH_TOKEN: token,
    CHARIOX_MANAGED_ISOLATION_PROBE_WORKSPACE: workspace,
    CHARIOX_PROBE_PACKAGE_JSON: path.join(repositoryRoot, "apps/cli/package.json"),
  })

  assert.equal(result.code, 0, result.stderr)
  assert.deepEqual(JSON.parse(result.stdout), {
    authenticated: true,
    provider: "codex",
    accountProfile: "default",
    workspace,
    denied: [
      "kernel state",
      "Vault and other provider accounts",
      "slice publication root",
      "Docker broker",
      "unselected repository",
      "host process roots",
    ],
  })
  assert.equal(requests.length, 4)
  assert.deepEqual(requests[0], {
    CreateSession: { workspace_id: workspace, worktree_id: workspace },
  })
  assert.equal(requests[1].LaunchProviderRun.provider, "codex")
  assert.equal(requests[1].LaunchProviderRun.adapter_key, "codex")
  assert.deepEqual(requests[2], {
    GetProviderRun: { provider_run_id: "provider-run-1" },
  })
  assert.deepEqual(requests[3], { EndSession: { session_id: "session-1" } })
})

function response(requestId, payload) {
  return JSON.stringify({
    type: "response",
    request_id: requestId,
    response: payload,
    error: null,
  })
}

async function runProbe(environment) {
  return await new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [probe], {
      cwd: repositoryRoot,
      env: {
        PATH: process.env.PATH,
        ...environment,
      },
      stdio: ["ignore", "pipe", "pipe"],
    })
    let stdout = ""
    let stderr = ""
    child.stdout.on("data", (chunk) => { stdout += chunk })
    child.stderr.on("data", (chunk) => { stderr += chunk })
    child.once("error", reject)
    child.once("close", (code) => resolve({ code, stdout, stderr }))
  })
}
