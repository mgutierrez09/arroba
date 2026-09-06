import assert from "node:assert/strict"
import test from "node:test"

import { startProviderLoginRequest } from "./ipc-provider-requests.js"
import * as providerRequests from "./ipc-provider-requests.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"

test("provider login request carries the selected enrollment method", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 307)
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
