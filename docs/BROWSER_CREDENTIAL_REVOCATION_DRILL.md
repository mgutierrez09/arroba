# Credential revocation during vault unlock

Run the existing guarded real-provider Room drill with
`CHARIOX_ROOM_DRILL_OFFICE_SCENARIO=onboarding-revocation`, Browser mode, and
an explicitly selected official provider and model. It first completes the
four-phase onboarding flow, proving the credential works before revocation.

The test then locks the home vault through kernel IPC. The same agent opens
the password form and requests one secret paste. When the agent is waiting
on its real vault-unlock interaction, the simulated human changes that exact
credential's allowed host to `revoked.invalid` through kernel IPC, reads the
change back, and only then answers the private unlock request.

Acceptance requires exactly one scope-denied paste and no completed fill after
the action-history baseline. A stale field, timeout, unrelated credential,
missing unlock, successful paste, retry or alternate tool cannot pass. The
positive onboarding assertions remain unchanged. Credential metadata retains
its vault source so the owning drill can delete the secret during cleanup.
Passwords never enter provider prompts or proof reports. The owning drill
performs its existing plaintext-secret scan and removes its slice and state.

Focused tests use the kernel request/response boundary and the retained
provider-history/action-ledger proof boundary. Live acceptance must run the
same JavaScript drill on both the previous cached-authority runtime and the
post-unlock refresh runtime. A failing old-runtime run and passing new-runtime
run are required before claiming this regression is covered end to end.

This scenario does not prove all credential-removal, vault-configuration,
concurrent policy-update, real-service, Web, provider, or managed-machine cases.
It proves host-scope revocation across the pending-unlock boundary.
