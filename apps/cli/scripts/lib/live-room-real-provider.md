# Real provider Room drill

Run the existing physical Room fixture with `CHARIOX_ROOM_DRILL_FOCUS=real-provider`,
`CHARIOX_ROOM_DRILL_PROVIDER=codex|claude|opencode`, and an explicit
`CHARIOX_ROOM_DRILL_MODEL`. Run one provider at a time. This makes a real provider
request and can consume provider usage.

Use `CARGO_TARGET_DIR` for the existing matching binaries and
`CHARIOX_ROOM_DRILL_IMAGE` for an existing exact-source image. The latter disables
automatic image builds. The fixture enforces one 2-GiB, one-CPU headed slice and
always cleans up its container, volume, temporary state, and listeners.

The real-provider mode explicitly enables the existing
`slices.linux.allow_unconfined_seccomp` option for this disposable local slice.
This permits the production Bubblewrap launcher to create its inner provider
namespace. The provider still runs with the inner seccomp filter, filesystem
isolation and dropped capabilities. Ordinary fixture modes and user settings
are unchanged. Do not use this local drill setting as managed-host acceptance;
managed hosts additionally require the dedicated rootless Docker boundary.

The empty provider workspace is under a private `mkdtemp` directory in
`~/.chariox/dev/browser-computer-use/`, not macOS `/var/folders`, which Colima
does not share by default. Only that empty child workspace is made writable
across provider user namespaces. The parent remains private and is deleted
with the rest of the drill state.

The kernel resolves the default linked provider profile and transfers it through
the normal slice-backed agent launch. The drill does not copy credentials or call
the provider's SDK, nor does it issue MCP calls on behalf of the agent. Local and
remote TUI observers still use stub agents; the separately spawned driver uses
the selected official provider.

Acceptance requires an agent-attributed completed Computer click in the Room
ledger, its physical click counter change in the shared browser, and matching
notices in both local and remote TUI. A textual provider success claim is not
acceptance. `real-provider.json` records the last completed phase; overall success
also requires `result.json` and successful cleanup.

On prompt rejection or action timeout, the checkpoint records bounded diagnostic
evidence before cleanup. It contains allowlisted agent/activity/turn states,
entry and action counts, and fixed error-pattern codes, never raw provider
text, tool arguments, endpoints or exception messages. A tool-name mention is
only a diagnostic hint, not proof that a tool ran. The Room ledger and physical
effect remain the acceptance criteria. Diagnostics inspect at most two turns,
256 entries, eight blobs advertised at no more than 32 KiB characters each and
128 KiB characters total, with an eight-second request deadline. Oversized or
unavailable content is reported as partial coverage. The server response size
is still governed by the existing transport bounds; these are inspection and
request budgets, not a new transport limit. An empty code set means no recognized
error pattern, not a healthy provider. Existing partial evidence survives a
later diagnostic failure.

The action wait checks this fresh agent's recent turn at most once every two
seconds. A completed turn containing a provider error ends the wait early;
errors on open turns do not. The physical click and TUI checks are unchanged.

SIGINT and SIGTERM request cooperative interruption. The next poll, command or
kernel request stops new work; already-started requests return before cleanup
runs, so their returned resource identities are not discarded. Cleanup keeps
signal handlers installed to ignore repeated interruption requests, stops owned
resource-producing processes before final Docker removal, and still checks
container, volume, temporary-state and listener removal. SIGKILL cannot run
JavaScript cleanup and is not a graceful interruption path.

The pending Web companion result wait uses the same interruptible sleep as the
rest of the OSS drill. SIGINT or SIGTERM must enter protected cleanup without
waiting for the companion deadline, even when Web has not returned a result.
The normal `test:room-provider` command includes real child-process checks for
both signals, repeated signals during cleanup, and temporary-state removal.

`CHARIOX_ROOM_DRILL_IMPORT_FIRST=1` additionally runs the public slice account
import operation before spawning. Keep this separate from the normal automatic
transfer path so import-and-launch regressions can be reproduced.

## Graphical office work

Set `CHARIOX_ROOM_DRILL_COMPUTER_TASK=office` with standalone `real-provider`
focus and Computer mode. This uses the same official agent after its initial
click check, not a second provider or a driver issuing MCP calls as an agent.
The fixture installs Mousepad and xterm and establishes a synthetic webmail
session before the office prompts. It does not create the document.

The first prompt asks the agent to use the real Openbox Terminal emulator menu,
launch Mousepad through graphical terminal input, type a multiline Unicode
document with Computer tools, save it, and leave the editor focused. The driver
checks the exact saved contents, corresponding agent-attributed keyboard Action,
completed prompt identity, idle agent, and matching notices in both TUIs. A
physical screenshot must preserve Mousepad focus.

The next prompt asks the same agent to use structured Browser tools to compose
one message and attach that file. Acceptance compares the filename, byte length
and SHA-256 received by the authenticated mail fixture with the actual saved
document. It requires a fresh attributed upload followed by one submit and both
TUI notices. No real email is sent. The fixture listener closes on success or
failure; the parent drill removes the installed applications, document and
provider processes with its disposable container and home volume.

The office mode intentionally rejects Web-companion focus until that viewer's
office-work verification exists. It does not establish vault login, external
email, provider save/resume, other office scenarios, all providers, or managed
deployment. The usual runtime image, memory bounds, source identity and cleanup
requirements still apply. A successful initial click alone never passes the
office mode.

### Office validation status

On 2026-09-06 the live official-provider run proved graphical editor input,
exact saved file bytes, preserved editor focus and both TUI observations. The
mail turn then exposed an expired submitting client attachment. The office
runner now creates and releases an attachment per prompt; two focused
regressions reproduce the old failure and pass with that fix.

A full rerun was rejected before slice creation by memory admission while two
other tests held the VM's reserved capacity. End-to-end office acceptance and
Web office coverage remain unproven. Unit tests do not replace that run.

## Optional Web companion

With `CHARIOX_ROOM_DRILL_FOCUS=web-companion`, additionally set
`CHARIOX_ROOM_DRILL_WEB_REAL_PROVIDER=1` and the explicit provider/model above.
This requires the paired Cloud Web companion implementation. An older companion
that returns only stub-agent evidence is rejected, not counted as a pass.

The shared `runRoomRealProviderAction` runner can operate on the official agent
already selected by Web. Its `beforePrompt` callback must finish before prompt
submission. It proves only the agent-attributed kernel action. The caller must
separately verify physical input and fresh Web pixels; it cannot claim TUI
coverage from this helper. The OSS companion verifier then checks the provider
action against authoritative history and observes it in both TUIs before
acceptance. Human input must follow the provider action in the same Room.

For a reused agent, the runner reloads its provider/model/profile/Room from
kernel state and checks membership of the intended slice before submitting a
prompt. Reported provider identity comes from that verified configuration.
An action-sequence baseline captured after Web readiness excludes clicks from
earlier turns. A supplied agent cannot be combined with import-first.
Reused agents must be idle. Their pre-prompt turn IDs are also baselined so an
older completed error cannot abort the new prompt while its turn is still open.

The isolated provider mode still verifies physical input and both TUIs through
`runRoomRealProvider`. Structured Browser operations, persistence, permission
denial and all-provider acceptance remain in the end-to-end plan. The new Web
mode needs live acceptance and exact-head review in both repositories.

## Long companion budgets

`room-drill-companion-budget.mjs` owns timeout validation for both the OSS waiter
and the paired Web launcher. Normal defaults remain three minutes for a direct
OSS companion and four minutes for Web. An active soak automatically receives
its requested duration plus ten minutes for setup and final verification.
An explicit `CHARIOX_ROOM_DRILL_COMPANION_TIMEOUT_MS` is preserved, but Web rejects
it before provisioning if it is too short for the configured soak. The maximum
is 24 hours plus ten minutes, covering the longest planned gate without an
unbounded wait. Allowing that duration does not implement or prove the idle soak.

The paired Web launcher loads this helper from `CHARIOX_OSS_REPO`; select matching
OSS and Cloud branches. The OSS wait remains interruptible during a long budget.
Tests exercise eight-hour and 24-hour configurations without sleeping for those
durations. They are configuration/cancellation evidence, not completed soak runs.
