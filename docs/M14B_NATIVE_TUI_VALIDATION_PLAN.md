# M14B Native TUI Validation Plan

## Status

In progress. This milestone replaces the earlier split-native TUI plan with a
validation-first plan for Chariox-managed provider-native TUIs:

```text
chariox codex [session-ref]
chariox opencode [session-ref]
chariox claude [session-ref]
```

Every native TUI launch is owned by Chariox. If no session ref is provided,
Chariox creates a session and the first native-TUI agent. If a session ref is
provided, Chariox attaches a new native-TUI agent to that Chariox session. Native
TUI launches never attach to an existing provider run.

This plan is intentionally about provider-native TUI behavior, not workspace live sync.
Prompt attachments are files/images transferred with a prompt. Workspace live sync is
the separate Chariox MCP `read_artifact`/`write_artifact` workspace-coordination
system and remains covered by the workspace live sync milestones and drills.

2026-05-14 update: Codex/OpenCode prompt attachment byte transfer is
implemented for relay-attached native TUI clients and Chariox TUI submissions.
The same-host relay drill validates native-TUI-origin and Chariox-TUI-origin
attachments with two provider-native TUIs plus one Chariox observer in the same
session. The older direct Docker slice-target drill has been removed; slice
coverage now uses the home-managed slice topology only.

2026-05-16 permissions update: native TUI permissions now use a single
kernel-owned interaction contract. Provider-native permission requests are
projected to every Chariox TUI in the session regardless of prompt origin.
Codex/OpenCode provider-native approval replies are routed back to the active
Chariox interaction when they arrive through the native proxy. Claude local
native TUI permission hooks bridge all origins into Chariox; remote-rendered
Claude detects visible PTY permission prompts and creates the same Chariox
interaction before injecting the resolved decision back into the PTY.

2026-05-14 Claude attachment update: local Claude native TUI prompt attachments
are implemented and covered by `live-native-tui-attachment-drill.mjs --provider
claude`. Native-origin `@file`/`@image` prompts are captured as kernel prompt
attachments. Chariox-origin text attachments are delivered through Claude hook
`additionalContext`; Chariox-origin non-text attachments are materialized and
submitted to Claude's TUI as native `@path` mentions.

2026-05-14 standard remote finding, now superseded: the original native TUI
launch contract could not validate standard home-worker native TUI because it
only launched provider runs on the home kernel. Standard home-worker native TUI
requires remote-backed native provider-run launch so provider execution happens
on the worker.

2026-05-15 implementation update: the first remote-backed native provider-run
path is in progress. Native launchers accept `--machine`/`--kernel-ref` and
move the native TUI agent onto a worker lease before launching the provider run.
For remote placement, Codex/OpenCode require `--server-in-kernel` and Claude
requires `--remote-rendered`, so provider execution is worker-owned rather than
handed a local provider endpoint. The home kernel forwards native provider-run
launches for remote-backed agents to the worker kernel over relay peer transport.
The live drill now has a `--standard-home-worker` mode for an isolated
same-host home/worker relay topology and a `--hetzner-worker` mode for a real
cross-host worker.

2026-05-15 standard home-worker validation update: Codex and OpenCode pass the
same-host standard home-worker drills for prompt/turns, provider permissions,
and prompt attachments. The drill uses two native TUIs plus one Chariox observer
CLI in one session, separated provider runs, no cross-agent marker
contamination, and badge transitions back to idle. Codex uses a native TUI
projection path that translates home-kernel session output into Codex app-server
notifications for the visible provider TUI while preserving the home-kernel
prompt queue and worker-owned provider execution.

2026-05-15 Claude standard home-worker update: Claude Code now has a
remote-rendered PTY path for worker-owned execution. Prompt/turn observation
passes with two Claude native TUIs plus one Chariox observer in the same Chariox
session, with no cross-agent marker contamination and badge transitions back to
idle. Image prompt attachments pass in both directions in same-host home-worker
mode: local Claude `@path` image prompts are intercepted by the remote-rendered
wrapper, transmitted as inline prompt attachments, materialized on the worker,
and injected into the worker-owned Claude TUI; Chariox-origin image attachments
follow the same worker materialization path. Permissions pass in both
native-origin and Chariox-origin directions: permission prompts surface in the
remote-rendered Claude native TUI and approval is sent through kernel-owned PTY
input to the worker provider run.

2026-05-15 historical Hetzner validation update: Codex and OpenCode pass against a
real Hetzner worker for prompt/turns, provider permissions, and prompt
attachments in both native-origin and Chariox-origin directions. The drill uses
SSH local forwarding for the relay and an SSH provider endpoint bridge for
worker-local Codex/OpenCode provider endpoints. Claude Code initially failed
extended Hetzner validation because macOS stores Claude Code credentials in the
Keychain while Linux expects `~/.claude/.credentials.json`; copying only
`.claude.json` transfers account metadata but not the login credential. Exporting
the local `Claude Code-credentials` Keychain payload into the worker
`~/.claude/.credentials.json` makes `claude auth status` green on the worker.
After that transfer, Claude Code passes the actual Hetzner prompt/turn,
permission, and image prompt-attachment drill through the remote-rendered PTY
path. The Keychain export used by this historical drill is retired. Managed
validation must use a provider-supported `claude setup-token` credential from
the Chariox encrypted vault. The legacy direct-worker drill fails closed until
it uses that managed materialization path. It must not access macOS Keychain or
copy refreshable credentials into a worker profile.

2026-05-15 home-managed slice validation update: Codex, OpenCode, and Claude
Code pass the local Docker home-managed slice drill for prompt/turns, provider
permissions, and prompt attachments. The native provider TUIs and Chariox
observer attach to the home kernel session; the home kernel places provider
execution on the slice worker through `slice_ref` and reuses the existing
leased-runtime projection path. Codex/OpenCode run in server-in-kernel mode with
worker-owned provider endpoints. Claude uses the remote-rendered PTY path.
Local Docker slice auth import historically copied Claude Code credentials.
Managed validation must inject a vaulted `CLAUDE_CODE_OAUTH_TOKEN` into the
official Claude CLI process and mark `/workspace` trusted in the slice. The
legacy direct-worker drill fails closed until it adopts that managed path.
Neither path may read or write macOS Keychain or copy a refreshable credential
into a worker profile.

2026-05-16 MCP/skills update: Native TUI MCP/skill drills now validate
agent-scoped grants for Codex and OpenCode in local, same-host standard
home-worker, and home-managed local Docker slice modes. Home-managed slices use
`CHARIOX_CAPABILITY_ISOLATION_ROOT` so worker MCP/skill registries are isolated
from host workspace and persisted-home registries. Claude native TUI hidden
skill injection now uses Claude Code's `UserPromptSubmit` hook
`additionalContext` path: the hook emits a scoped request id, and the local
launcher bridge or provider-execution worker kernel writes the matching hidden
context response before the hook returns. The hidden skill context does not
appear in the native TUI transcript.

## Goal

Validate and complete native TUI parity across the three providers and three
execution scenarios.

Providers:

- Codex
- OpenCode
- Claude Code

Functional areas:

- prompt and turn observation with two provider TUIs plus one Chariox TUI in one
  Chariox session
- provider permissions
- prompt attachments, meaning files/images sent with a prompt
- MCPs and skills

Scenarios:

- local: provider TUI and provider execution are on the same host/kernel
- standard remote: home kernel owns the session, worker kernel owns provider
  execution
- slice: home kernel owns the session and manages a slice/worker execution
  environment

## Current Coverage

Prompt/turns:

- Local Codex/OpenCode: covered by native TUI drills.
- Local Claude: covered by the Claude native TUI drill.
- Same-host relay Codex/OpenCode/Claude: covered by
  `live-remote-native-tui-drill.mjs`.
- Standard remote home-worker: Codex/OpenCode prompt/turn coverage passes in
  same-host relay mode and against the Hetzner worker. Claude prompt/turn
  coverage passes through the remote-rendered PTY path in same-host relay mode
  and against the Hetzner worker.
- Home-managed slice Codex/OpenCode/Claude: covered by
  `live-remote-native-tui-drill.mjs --home-managed-slice-local-docker`.

Permissions:

- Local Codex/OpenCode: covered by `live-native-tui-permission-drill.mjs` in
  both native-TUI-origin and Chariox-TUI-origin directions.
- Local Claude: product behavior is implemented with the same kernel-owned
  interaction contract for native-origin and Chariox-origin prompts. Dedicated
  automated coverage is provided by `live-native-tui-permission-drill.mjs
  --provider claude`.
- Standard remote home-worker Codex/OpenCode: covered by
  `live-remote-native-tui-drill.mjs --standard-home-worker --providers
  codex,opencode --include-permissions`. Native-origin and Chariox-origin
  prompts both surface permission interactions to the Chariox observer and can be
  approved there. The same coverage also passes with `--hetzner-worker`.
- Standard remote home-worker Claude: covered by
  `live-remote-native-tui-drill.mjs --standard-home-worker --providers claude
  --include-permissions`. Native-origin and Chariox-origin prompts both surface
  permission interactions through Chariox. The remote-rendered Claude TUI remains
  coherent while the launcher detects the PTY prompt and injects the resolved
  decision back into the provider run. This historically passed in same-host
  relay mode and with the actual Hetzner worker. Current direct-worker Claude
  runs fail closed until the managed vault setup-token path reaches the worker.
- Home-managed slice Codex/OpenCode/Claude: covered by
  `live-remote-native-tui-drill.mjs --home-managed-slice-local-docker
  --include-permissions`.

Prompt attachments:

- Local Codex/OpenCode: covered by `live-native-tui-attachment-drill.mjs`.
- Local Claude: covered by `live-native-tui-attachment-drill.mjs --provider
  claude` for native-origin and Chariox-origin image attachments. Text/file
  attachment delivery is also implemented through the same native capture and
  hook context paths.
- Same-host relay Codex/OpenCode: covered by
  `live-remote-native-tui-drill.mjs --providers opencode,codex
  --include-attachments`.
- Standard remote home-worker Codex/OpenCode: covered by
  `live-remote-native-tui-drill.mjs --standard-home-worker --providers
  codex,opencode --include-attachments`. The same coverage also passes with
  `--hetzner-worker`.
- Standard remote home-worker Claude: covered for image prompt attachments in
  both native-origin and Chariox-origin directions by
  `live-remote-native-tui-drill.mjs --standard-home-worker --providers claude
  --include-attachments` in same-host relay mode and with the actual Hetzner
  worker once credentials are transferred.
- Home-managed slice Codex/OpenCode/Claude: covered by
  `live-remote-native-tui-drill.mjs --home-managed-slice-local-docker
  --include-attachments`. Remote/slice placement forces byte transfer and
  provider-side materialization instead of passing host-local paths.

MCPs and skills:

- Covered for normal Chariox provider runs by existing local and remote
  MCP/skill drills.
- Local native TUI provider launch now follows the same agent-scoped grant
  rendering path as ordinary local provider launch.
- Standard home-worker native TUI intentionally does not install/copy MCPs or
  skills across home/worker machines. The worker must already have the matching
  MCP definitions, commands, environment, provider credentials, and any
  provider/Chariox skill material required for the run. Home may send
  grant-derived MCP requirements for fail-fast validation and provider-run
  rendering, but it must not become a remote package installer in this mode.
- Home-managed slice native TUI transfers granted Chariox skill packages from
  home to the child worker before provider execution because the slice is
  managed by the home kernel. MCP definitions are installed into an isolated
  managed-slice registry before worker launch and MCP commands/env execute on
  the worker side.
- Focused unit coverage has landed for native remote provider-run MCP
  propagation. Home-managed slice kernels set `CHARIOX_CAPABILITY_ISOLATION_ROOT`
  so project/user MCP and skill registries are isolated from any `.chariox`
  registries mounted from the host workspace or persisted in the slice home.
- Live MCP/skill drills now pass for Codex and OpenCode in local, standard
  same-host home-worker, and home-managed local Docker slice modes. Claude
  native TUI validates provider-run MCP config and hidden skill injection
  through hook `additionalContext` in local, same-host standard home-worker,
  and home-managed local Docker slice modes.

## Attachment Transfer Contract

Local fast path:

- When the provider execution process can read the same filesystem path as the
  TUI/client, Chariox may pass a local path or `file://` URL directly to the
  provider.
- The local path fast path is acceptable for local native TUI drills and avoids
  unnecessary copying.

Transmission path:

- When the provider execution process may not see the TUI/client filesystem,
  Chariox must transmit attachment bytes.
- Native TUI proxies and Chariox TUI prompt submission should convert local
  attachments into `PromptAttachment.contents_base64` when the run is remote,
  slice-backed, or explicitly exercising attachment transmission.
- The kernel materializes inline attachment bytes on the provider-execution
  side before dispatch, then rewrites the provider-facing attachment reference
  to a machine-local file path or provider-supported inline payload.
- MIME and filename metadata must be preserved.

Provider notes:

- Codex supports image prompt parts as `localImage`/`image`; non-image files are
  currently described as prompt text with a path. Remote/slice transmission must
  materialize images on the provider side before the Codex turn starts.
- OpenCode supports file prompt parts. Remote/slice transmission must
  materialize files on the provider side before forwarding the OpenCode prompt.
- Claude structured runs support inline base64 image/text handling, but Claude
  native TUI hook/PTY mode needs separate validation for both Chariox-origin and
  provider-native attachments.

## Implementation Order

1. Clean native TUI drill naming.
   - Remove native-TUI workspace live sync artifact checks from native TUI drills.
   - Use `attachments` only for prompt files/images.
   - Keep workspace live sync in its dedicated drills.

2. Implement and validate remote/slice prompt attachments for Codex and
   OpenCode.
   - Add a shared attachment-preparation helper in the CLI/native-TUI layer.
   - Preserve the local path fast path for local same-filesystem runs.
   - Encode local attachment bytes into `contents_base64` for remote/slice
     native TUI runs.
   - Confirm the kernel materializes those bytes on the provider-execution
     machine and provider-facing paths are local to that machine.
   - Add live checks for native-TUI-origin and Chariox-TUI-origin attachments.
     Same-host relay is validated for Codex/OpenCode; standard remote and
     home-managed slice remain to be added.

3. Revisit local permissions for all providers.
   - Codex/OpenCode local native TUI permissions pass in both directions.
   - Claude local permissions bridge native-origin and Chariox-origin prompts
     into the same Chariox interaction path.
   - Provider-native approval replies for Codex/OpenCode resolve the active
     Chariox interaction instead of bypassing the kernel.

4. Implement and validate local Claude prompt attachments.
   - Completed for local text/file and image attachments.
   - Native-origin `@file`/`@image` references are captured and submitted to the
     kernel as prompt attachments.
   - Chariox-origin text/file attachments are delivered through Claude hook
     `additionalContext`.
   - Chariox-origin images are materialized and injected into Claude Code as
     native `@path` mentions so the provider TUI handles them normally.

5. Validate standard remote home-worker native TUI.
   - In progress: native TUI launches can create remote-backed Chariox agents
     and request worker-owned native provider runs.
   - Current provider status:
     - Codex/OpenCode: prompt/turn, permission, and prompt-attachment drills pass
       in same-host home-worker relay mode and against the Hetzner worker.
     - Claude: prompt/turn, image prompt-attachment, and permission drills pass
       in same-host home-worker relay mode and against the Hetzner worker
       through the remote-rendered PTY path. The historical pass transferred a
       local Keychain credential payload to the worker; that fallback is now
       retired and current runs must use the managed vault path. The legacy
       direct-worker drill fails closed until that integration lands.
   - Required product work:
     - validate native TUI `--machine`/`--kernel-ref` placement arguments for
       Codex, OpenCode, and Claude;
     - validate the kernel-owned remote native provider-run launch path that
       asks the selected worker kernel to launch/bind the provider-native
       runtime for the leased agent;
     - mirror native prompt/turn output, permission interactions, status, and
       prompt attachments back to the home session without making the relay a
       runtime authority;
      - keep the Hetzner worker drill in the standard regression set for all
        providers, with an explicit Claude credential-transfer preflight.
   - Run the prompt/turn, permission, and prompt-attachment matrix for all three
     providers.
   - Home kernel owns the session; worker kernel owns provider execution.

6. Validate slice native TUI.
   - Completed for local Docker home-managed slices.
   - Codex/OpenCode/Claude pass the same prompt/turn, permission, and
     prompt-attachment matrix as standard home-worker mode.
   - Home kernel manages the slice/worker execution environment; native TUIs do
     not attach directly to the slice kernel.
   - Local Docker slice startup reuses the home relay when available and only
     falls back to a slice-private relay for standalone slice workflows.

7. Validate MCPs and skills for native TUI runs.
   - Codex/OpenCode: live MCP/skill validation passes locally, in same-host
     standard home-worker mode, and in home-managed local Docker slice mode.
   - Claude: live validation confirms pre-granted MCPs are rendered into the
     provider run and same-turn skill requests receive hidden Chariox skill
     context through Claude Code hook `additionalContext` rather than visible
     PTY input.
   - Standard remote home-worker: do not copy/install MCPs or skills as product
     behavior. The drill preinstalls matching worker MCP definitions as setup,
     then validates grant-derived worker provider-run rendering.
   - Home-managed slice: transfer granted Chariox skill packages to the
     home-managed child worker before provider execution, validate worker-local
     materialized skill paths in prompt context where provider-supported, and
     validate MCP rendering against MCP definitions installed into the managed
     slice isolation root.
   - Keep workspace live sync marker writes out of native-TUI MCP/skill validation
     unless the drill is explicitly a workspace live sync drill.

## Matrix

Legend:

- `pass`: validated in current code
- `gap`: not validated or not implemented
- `recheck`: previously passed, but must be rerun after the native TUI cleanup
- `partial`: implemented or manually confirmed, but missing complete automated
  live-drill coverage

| Scenario | Provider | Prompt/turns | Permissions | Attachments | MCP/skills |
| --- | --- | --- | --- | --- | --- |
| local | Codex | pass | pass | pass | pass |
| local | OpenCode | pass | pass | pass | pass |
| local | Claude | pass | pass | pass | pass |
| standard remote | Codex | pass | pass | pass | pass |
| standard remote | OpenCode | pass | pass | pass | pass |
| standard remote | Claude | pass | pass | pass | pass |
| home-managed slice | Codex | pass | pass | pass | pass |
| home-managed slice | OpenCode | pass | pass | pass | pass |
| home-managed slice | Claude | pass | pass | pass | pass |

## Drill Requirements

Prompt/turn drills must launch:

- two provider-native TUIs in one Chariox session
- one observer Chariox TUI or automation-backed Chariox CLI in the same session

They must validate:

- provider-native prompt from agent A appears in Chariox history and observer UI
- provider-native prompt from agent B appears in Chariox history and observer UI
- Chariox-origin prompt to agent A appears in the provider-native UI path and
  completes
- Chariox-origin prompt to agent B appears in the provider-native UI path and
  completes
- responses are visible in Chariox history and observer UI
- no A/B cross-contamination
- agent footer badge changes from idle to working/thinking during the turn and
  returns to idle after completion

Attachment drills must validate:

- native-TUI-origin attachment reaches the provider execution side
- Chariox-TUI-origin attachment reaches the provider execution side
- local runs may pass local paths directly
- remote and slice runs must transmit bytes and materialize provider-local
  paths

Permission drills must validate:

- provider-native permission requests create a kernel-owned interaction visible
  to all Chariox TUIs attached to the session
- answering from Chariox resumes the same provider turn
- where provider-native approval replies are supported, the native reply
  resolves the same Chariox interaction instead of bypassing kernel state
- if a provider only exposes approval through a rendered PTY, the PTY remains
  coherent and Chariox injects the resolved decision back through the provider
  selection path

MCP/skill drills must validate:

- pre-granted MCPs and skills are visible to native-TUI provider runs
- same-turn skill requests work when supported
- standard remote provider execution sees worker-local MCP definitions and
  preinstalled worker-local skill material without home-to-worker package
  transfer
- slice provider execution sees worker-local MCP definitions and home-managed,
  hash-verified materialized Chariox skill files
- provider-native MCP calls execute on the provider execution machine

## Cleanup Rules

- Do not add native-TUI workspace live sync artifact checks to this milestone.
- Do not use `artifact` in native TUI drill names unless the test is about
  generic test output files kept after failure.
- Keep behavior below clients where possible: kernel owns sessions, agents,
  provider runs, permissions, attachment materialization, history, and status.
- Any protocol shape changes must bump `LOCAL_DAEMON_PROTOCOL_VERSION`, update
  protocol snapshot/hash tests, and add a focused drill.
