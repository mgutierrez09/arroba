# Email-gated office onboarding validation

This implements the controlled local acceptance fixture for scenario 1 of
`AGENT_SLICE_OFFICE_WORK_DRILLS.md`. It is not Gmail or external SaaS acceptance.

## Fixture contract

- The existing mail fixture requires its configured account and password.
- Incoming mail has authenticated inbox links and message detail pages. It does
  not increment the agent-sent message collection or satisfy a send assertion.
- `startOfficeOnboardingFixture({ mail, host, publicHost })` starts a separate
  HTTP service. Set `publicHost` to the hostname reachable inside a slice when
  running a Docker drill. The service generates links from this configured
  origin, not an untrusted request Host header.
- `/service/register` accepts the fixture recipient, organization and a password
  of 12 to 1024 characters. It stores a salted scrypt verifier, delivers a
  confirmation message and redirects to a check-email page. It never returns
  the password or a confirmation link in the registration response.
- The agent must sign in to mail, open the message, follow its link and submit
  the confirmation form. Reading the link alone does not activate the account.
- Confirmation expires after five minutes and can succeed only once. Guessed,
  expired and replayed links cannot establish a service session. Duplicate
  registration is rejected, including concurrent requests.
- `/service/login`, `/service/dashboard` and `/api/account` require completed
  confirmation. The account API returns metadata only. Service and mail use
  different session cookies.
- Call both fixture `close()` methods in `finally`; no state should survive the
  disposable drill. Registration bodies and received mail have explicit bounds.

## Focused validation

Run without builds or dependency installation:

```sh
node --test apps/cli/scripts/lib/office-onboarding-fixture.test.mjs \
  apps/cli/scripts/lib/browser-computer-fixture-inbox.test.mjs \
  apps/cli/scripts/lib/browser-computer-fixture.test.mjs \
  apps/cli/scripts/lib/browser-computer-fixture-attachments.test.mjs
```

Set `PLAYWRIGHT_MODULE` to an existing installed Playwright module, then run
`node --test apps/cli/scripts/lib/office-onboarding-fixture.browser-test.mjs`
under the existing host memory guard. This Chrome check fills synthetic test
passwords directly to validate fixture navigation. It cannot prove vault safety,
kernel authority, provider capability, Web display or TUI visibility.

Screenshots and results go outside the repository under
`~/.codex/evidence/browser-computer-use/office-onboarding/`.

## Official-provider runner

The source runner is wired to the existing local Room drill. Select it explicitly:

```sh
CHARIOX_ROOM_DRILL_FOCUS=real-provider \
CHARIOX_ROOM_DRILL_PROVIDER=codex \
CHARIOX_ROOM_DRILL_MODEL=gpt-5.6-sol \
CHARIOX_ROOM_DRILL_PROVIDER_MODE=browser \
CHARIOX_ROOM_DRILL_OFFICE_SCENARIO=onboarding \
node apps/cli/scripts/live-room-environment-pointer-click-drill.mjs
```

Use the existing validated slice image and shared binaries, private Docker
configuration, resource guard and normal cleanup wrapper. Do not create another
unbounded build. The runner first completes the ordinary provider/browser probe,
then uses that same agent for four turns: vault-backed mail login, generated-
credential registration, confirmation-email reading, and confirmation submission.

Only the synthetic test user's private replies go through `RespondToInteraction`.
The driver never calls the agent MCP endpoint, fills the browser, logs in on the
agent's behalf or follows the confirmation link. Each prompt uses a fresh
attachment and waits for its exact completed provider turn. Screenshots and
physical browser text are captured at each phase.

Acceptance reads the complete kernel tool records for those prompts and binds
credential tool results to attributed Browser actions. It requires the expected
credential scopes, ordered paste/submission actions, explicit browser activation,
and both TUI notices. The fixture's private password observer registers the
generated test secret with the existing driver's leak scanner and error redactor;
it never exposes that value through an HTTP or model-visible response. The outer
drill owns credential, kernel, slice and scratch cleanup.

The implementation has focused regression coverage but has not yet passed a live
official-provider run. Web mode rejects this scenario explicitly until its
projection and final verifier are connected. It must not silently run the simpler
Browser click test and claim onboarding success.

## Remaining full-scenario acceptance

Run and validate the official-provider path above. Add Web projection and final
Web/OSS verification, then run the same scenario for all providers. Locked-vault
and wrong-origin rejection remain required negative cases. The existing byte
scans cover retained textual payloads, not plaintext rendered inside compressed
screenshots; screenshot OCR checks remain required before claiming the full
secret-safety gate.

Run the explicit Gmail and external-service scenario with an authorized test
account after controlled local functionality passes. Record any human approval
or verification step. A controlled fixture pass cannot close that requirement.
