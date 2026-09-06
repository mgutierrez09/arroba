import assert from "node:assert/strict"
import test from "node:test"

import {
  armDeploymentCredentialEnrollmentRequest,
  deploymentCredentialEnrollmentServiceSubject,
  requestCredentialEnrollmentInteractionRequest,
} from "./ipc-terminal-runtime-requests.js"
import {
  LOCAL_DAEMON_PROTOCOL_VERSION,
  type CredentialEnrollmentInteractionResolvedResponse,
  type DeploymentCredentialEnrollmentArmedResponse,
} from "./kernel-types.js"

test("credential enrollment requests match current protocol", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 287)
  assert.equal(
    deploymentCredentialEnrollmentServiceSubject("enrollment-1"),
    "deployment-credential-enrollment:enrollment-1",
  )
  assert.deepEqual(
    armDeploymentCredentialEnrollmentRequest(
      "session-1",
      "attachment-1",
      "agent-1",
      "enrollment-1",
      "profile-1",
      7,
    ),
    {
      ArmDeploymentCredentialEnrollment: {
        session_id: "session-1",
        attachment_id: "attachment-1",
        agent_id: "agent-1",
        enrollment_id: "enrollment-1",
        profile_id: "profile-1",
        target_version: 7,
      },
    },
  )
  assert.deepEqual(
    requestCredentialEnrollmentInteractionRequest(
      "session-1",
      "agent-1",
      "enrollment-1",
      "profile-1",
      7,
      "https://claude.com/oauth/authorize?state=opaque",
      120,
    ),
    {
      RequestCredentialEnrollmentInteraction: {
        session_id: "session-1",
        agent_id: "agent-1",
        enrollment_id: "enrollment-1",
        profile_id: "profile-1",
        target_version: 7,
        provider_authorization_url: "https://claude.com/oauth/authorize?state=opaque",
        timeout_sec: 120,
      },
    },
  )
})

test("credential enrollment response types preserve callback optionality", () => {
  const armed = {
    DeploymentCredentialEnrollmentArmed: {
      enrollment_id: "enrollment-1",
      profile_id: "profile-1",
      target_version: 7,
      session_id: "session-1",
      agent_id: "agent-1",
      expires_at_ms: 1_234,
    },
  } satisfies DeploymentCredentialEnrollmentArmedResponse
  const submitted = {
    CredentialEnrollmentInteractionResolved: {
      status: "submitted",
      callback: "https://localhost/callback?code=fixture",
    },
  } satisfies CredentialEnrollmentInteractionResolvedResponse
  const canceled = {
    CredentialEnrollmentInteractionResolved: {
      status: "canceled",
    },
  } satisfies CredentialEnrollmentInteractionResolvedResponse

  assert.equal(armed.DeploymentCredentialEnrollmentArmed.target_version, 7)
  assert.equal(submitted.CredentialEnrollmentInteractionResolved.status, "submitted")
  assert.equal(canceled.CredentialEnrollmentInteractionResolved.status, "canceled")
})
