# M20 Docker Slice Browser State Validation Plan

## Goal

Make Docker-backed slices preserve browser login state, installed programs, and user-visible slice state reliably enough for real work.

The immediate product requirement is that a user or agent can log into services such as Gmail inside a slice, save the slice, later relaunch it, and continue working without repeating login or 2FA unless the external service itself invalidates the session.

This milestone is not a substrate bakeoff. Docker is the required implementation path for slice saved state. Any failure in the drills must be treated as a Docker slice persistence bug or an external-service policy issue to root-cause, not as a reason to switch substrates.

## Local resource checks

The local drill records available memory, disk, and Docker inventory. It does
not reject work because of a fixed free-space or free-memory percentage.
Before a heavy run, estimate its additional peak resource use and recovery
reserve from previous measurements or a conservative first-run estimate. Pass
those byte budgets as `M20_REQUIRED_MEMORY_BYTES` and `M20_REQUIRED_DISK_BYTES`.
The drill rejects an operation whose supplied budget exceeds available capacity.

If a budget is omitted, the drill reports a warning and records resources for
observation; a passing check is not proof that an unestimated workload will fit.
Actual zero available memory or disk still fails. Run builds and headed slices
serially, monitor during execution, and stop only an operation that presents a
credible exhaustion risk. Keep build caches and runtime state outside the
repository. The existing single-slice default and cleanup checks remain in force.

## Working Hypothesis

Docker may be sufficient if Chariox preserves the correct browser and OS identity state. A simple container image commit is not enough when important state is stored in bind mounts, volumes, tmpfs, keyrings, or runtime-only browser profile directories.

The browser state that may matter includes:

- Chromium profile data: cookies, local storage, IndexedDB, history, cache, extension state, and browser local state.
- Linux keyring or secret service state used by Chromium to decrypt stored browser data.
- Stable machine identity such as `/etc/machine-id`.
- Stable Linux user, home path, UID/GID, hostname, and browser profile path.
- Installed packages and desktop/browser dependencies.
- Graceful shutdown of browser processes so profile SQLite databases are not captured while locked or partially flushed.

## Non-Goals

- Do not add VM-backed slices in this milestone or use VMs as a fallback.
- Do not add Chariox-managed live sync beyond copying the bonded workspace into the slice.
- Do not build generic browser automation abstractions beyond what is needed for the drill.
- Do not make Gmail-specific product behavior. Gmail is the representative hard validation target, not a special case.

## Research Baseline

The implementation and validation should account for these known constraints:

- Chromium user data directories are the canonical location for browser profile state.
- Browser automation stacks distinguish ephemeral browser contexts from persistent profile contexts.
- Docker image commits do not include mounted volumes.
- Linux Chromium may depend on a keyring or secret service for persisted encrypted data.

Useful references:

- Chromium user data directory: https://chromium.googlesource.com/chromium/src/+/HEAD/docs/user_data_dir.md
- Browserless persistent user data directory: https://docs.browserless.io/enterprise/user-data-directory
- Docker storage and volumes: https://docs.docker.com/engine/storage/
- Playwright authentication/persistent state model: https://playwright.dev/docs/auth

## Phase 1: Inspect Current Slice State Boundaries

Map the current Docker slice runtime before changing behavior.

Tasks:

- Identify the slice container image, container name, mounts, volumes, tmpfs mounts, user, UID/GID, home directory, hostname, and environment.
- Identify the browser binary and launch command used in the slice.
- Record Chromium flags, especially `--user-data-dir`, `--profile-directory`, `--password-store`, sandbox flags, and remote debugging flags.
- Locate actual browser profile directories inside the running slice.
- Locate keyring or secret-service state, including `.local/share/keyrings`, D-Bus runtime paths, and any configured password store.
- Check whether `/etc/machine-id` is stable across save/relaunch.
- Check whether browser profile or keyring paths currently live in container writable layer, bind mounts, Docker volumes, or tmpfs.

Deliverable:

- A short findings note in the implementation report identifying what state is currently preserved and what is lost.

## Phase 2: Define Docker Slice Persistence Contract

Make the slice persistence model explicit and portable.

Required persisted state:

- Container filesystem or derived image for installed programs and OS-level mutations.
- A slice-owned state directory or volume for browser profile, keyring, desktop state, and other user-level state that should survive restart.
- Stable machine identity for the saved slice.
- Stable user identity and home path.

Policy:

- Bonded workspace files are copied into the slice according to the existing slice behavior.
- Chariox does not add live sync by default.
- Browser and keyring state are slice state, not workspace state.
- Save must either stop relevant processes gracefully or refuse while managed agents are actively running.

## Phase 3: Implement Minimal Docker Hardening

Make only the changes needed to validate the hypothesis.

Expected changes:

- Ensure Chromium launches with an explicit persistent profile path inside slice state.
- Ensure the profile path is included in save/restore, whether through image commit, volume snapshot, or a slice state directory copy.
- Ensure keyring or password-store behavior is deterministic across restart.
- Preserve or restore `/etc/machine-id` for the saved slice.
- Gracefully stop browser/desktop processes before snapshotting when possible.
- Restore the saved slice with the same state paths, user identity, and browser profile path.
- Keep this logic in kernel slice services, not only in TUI or web UI.

Avoid:

- Gmail-specific hacks.
- Runtime MCP save actions for agents.
- VM abstractions or fallback paths.
- Broad refactors unrelated to slice persistence.

## Phase 4: Local Deterministic Drill

Before using Gmail, validate with a local web page that exercises browser persistence.

Drill:

1. Launch a Docker slice.
2. Open Chromium to a local test page.
3. Store data in cookies, localStorage, IndexedDB, and a service worker cache.
4. Install a small package or program inside the slice.
5. Save the slice.
6. Fully remove the running container.
7. Relaunch from the saved slice.
8. Verify the installed program still exists.
9. Verify browser cookies, localStorage, IndexedDB, and cache survived.
10. Capture screenshots before save and after restore.

Pass criteria:

- Browser data survives a full container destroy/recreate.
- Installed program survives.
- No manual file repair is needed after restore.

## Phase 5: Gmail Live Drill

Validate the real product requirement.

Drill:

1. Launch a Docker slice from the web terminal view page.
2. Start an agent in the slice.
3. Ask the agent to use runtime MCP tools to open Gmail in Chromium.
4. Use Chariox vault injection for the Gmail password.
5. Relay any Google confirmation code or number to the user and wait for confirmation.
6. Once logged in, ask the agent to send a test email.
7. Save the slice as the user from the web terminal view page.
8. Stop and fully remove the running slice container.
9. Relaunch the saved slice.
10. Start or reattach an agent in the restored slice.
11. Ask the agent to open Gmail and send a second test email without re-entering the password.
12. Capture screenshots throughout the web terminal view page, including before save, after restore, and the sent email result.

Pass criteria:

- Gmail opens after restore without password entry.
- The agent can send the second email after restore.
- The preserved browser state is produced by Docker slice save/restore, not by manual host browser interaction.
- Screenshots prove the user-facing behavior from the web terminal view page.

Known acceptable outcome:

- Google may still force reauth based on its own risk model. If that happens, capture the exact prompt and compare against the deterministic local drill. One external-service reauth does not prove Docker persistence failed, but repeated Gmail reauth with preserved local browser state means the product should describe the behavior as persistent slice state, not guaranteed third-party session continuity.

## Phase 5A: No-Intervention Webmail Drill

Run this drill before the Gmail live drill. It validates the same Chariox-controlled behavior without depending on Google 2FA, CAPTCHA, phone approval, abuse checks, or external account risk scoring.

This drill does not prove Google will preserve a Gmail session. It proves that Docker slice save/restore preserves the browser-authenticated web-app state needed for Gmail-like work when the service itself does not invalidate the session.

Fixture:

- Start a Chariox-owned test webmail fixture outside the slice.
- The fixture exposes a browser UI with login, inbox, compose, sent mail, and logout.
- The fixture sets realistic secure session cookies and uses localStorage or IndexedDB for client-side UI state.
- The fixture stores sent messages server-side and exposes a test-only verification endpoint outside the agent's browser.
- The fixture password is placed in Chariox vault and must be injected into the browser by runtime MCP secret insertion. The agent must not receive the password in its context.
- Optionally back the fixture with Mailpit for captured SMTP delivery evidence. Mailpit provides an SMTP server, web interface, and API suitable for email testing.

References:

- Mailpit: https://mailpit.axllent.org/
- Mailpit Docker image: https://hub.docker.com/r/axllent/mailpit

Drill:

1. Launch the webmail fixture and record its base URL.
2. Create a deterministic test account such as `agent@chariox.test`.
3. Store the account password in Chariox vault.
4. Launch a Docker slice from the web terminal view page.
5. Start an agent in the slice.
6. Ask the agent to open the webmail fixture in Chromium using runtime MCP browser/slice tools.
7. Ask the agent to log in using runtime MCP secret insertion from the vault.
8. Ask the agent to compose and send a first message to `recipient@chariox.test`.
9. Verify outside the agent context that the message was received by the fixture or Mailpit.
10. Save the slice as the user from the web terminal view page.
11. Stop and fully remove the running slice container.
12. Relaunch the saved slice.
13. Start or reattach an agent in the restored slice.
14. Ask the agent to open the webmail fixture again.
15. Ask the agent to send a second message without re-entering the password.
16. Verify outside the agent context that the second message was received.
17. Capture screenshots from the web terminal view page before save, after restore, and after the second send.

Pass criteria:

- The first login uses vault secret insertion and does not leak the password to the agent transcript.
- The first message is sent from inside the slice by the agent driving the browser UI.
- After save, container removal, and restore, the agent can open the same browser session without password entry.
- The second message is sent after restore.
- Browser cookies, localStorage or IndexedDB, and any relevant profile state survive restore.
- Screenshots and fixture verification prove the behavior without user intervention.

Failure interpretation:

- If this drill fails, the Docker slice persistence implementation is broken independently of Gmail.
- If this drill passes but Gmail fails, the remaining issue is likely Google-specific session risk, identity, or automation detection rather than generic Docker browser-state persistence.

## Phase 6: Cross-Platform Assessment

After the macOS Docker drill, assess portability of the Docker implementation across supported host operating systems.

Questions to answer:

- Does the same Docker persistence contract work on Linux Docker Engine?
- Does it work on Docker Desktop on macOS?
- Does it work on Docker Desktop on Windows or WSL-backed Docker?
- Are keyring and machine-id behaviors portable, or do we need a slice-internal secret-store strategy?
- Are there unavoidable host-specific dependencies?

## Success Gate

The milestone is complete only when Docker-backed slice saved state satisfies:

- Local deterministic browser state survives.
- Installed programs survive.
- Gmail or another comparable real service remains usable after restore in repeated drills, or failures are clearly external-service risk checks rather than lost slice state.
- The approach is portable enough for macOS, Linux, and Windows with small host-specific adapters.

If any drill fails:

- Preserve all evidence.
- Identify the exact missing or corrupted state.
- Fix the Docker slice persistence contract.
- Rerun the failing drill.
- Continue this loop until Docker satisfies the requirement or the remaining blocker is proven to be external-service policy outside Chariox's control.

## Evidence Requirements

Store drill artifacts under `./.artifacts/m20-docker-slice-browser-state/`.

Required artifacts:

- Current-state inspection log.
- Local deterministic drill screenshots.
- Gmail drill screenshots.
- Container inspect output before save and after restore.
- Browser profile path and keyring path evidence.
- Save/restore command or protocol transcript.
- Final pass/fail report with exact blockers if any.
