import assert from "node:assert/strict"
import test from "node:test"

import { setProviderAccountCredentialRequest, startProviderLoginRequest } from "./ipc-provider-requests.js"
import * as providerRequests from "./ipc-provider-requests.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"

test("provider login request carries the selected enrollment method", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 309)
  const request = startProviderLoginRequest("codex", "work", "device_code") as {
    StartProviderLogin: Record<string, unknown>
  }
  assert.equal(request.StartProviderLogin.provider, "codex")
  assert.equal(request.StartProviderLogin.account_profile, "work")
  assert.equal(request.StartProviderLogin.method, "device_code")
})

test("provider login request omits the method key for default enrollment", () => {
  const request = startProviderLoginRequest("claude") as {
    StartProviderLogin: Record<string, unknown>
  }
  assert.equal("method" in request.StartProviderLogin, false)
})

test("native account import requests the kernel scope without accepting a client path", () => {
  assert.equal(typeof providerRequests.importNativeProviderAccountProfileRequest, "function")
  assert.deepEqual(providerRequests.importNativeProviderAccountProfileRequest("claude"), {
    ImportNativeProviderAccountProfile: { provider: "claude" },
  })
})

test("provider setup token request carries account scope and explicit replacement", () => {
  assert.deepEqual(
    setProviderAccountCredentialRequest(
      "claude",
      "work",
      "secret-token",
      true,
      { sessionId: "session-1", agentId: "agent-1" },
    ),
    {
      SetProviderAccountCredential: {
        session_id: "session-1",
        agent_id: "agent-1",
        provider: "claude",
        account_profile: "work",
        value: "secret-token",
        overwrite: true,
      },
    },
  )
})
