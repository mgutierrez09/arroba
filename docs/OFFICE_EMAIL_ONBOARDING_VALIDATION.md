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

## Remaining full-scenario acceptance

The official-provider runner must enroll a fresh mail credential in the Chariox
vault, allow the agent to use only its handle through `paste_secret_to_slice`,
and require the agent to generate and use the service credential without reading
it. It must observe registration, mailbox access and confirmation through the
normal kernel action ledger and both TUIs, capture the same physical result in
Web, and scan transcripts, history, logs and screenshots for secret leakage.
The driver must not log in, read the email, confirm the service or fabricate
provider results on the agent's behalf. Locked-vault and wrong-origin rejection
remain required negative cases.

Run the explicit Gmail and external-service scenario with an authorized test
account after controlled local functionality passes. Record any human approval
or verification step. A controlled fixture pass cannot close that requirement.
