# Chariox v1 Protocol

## Status

Draft protocol aligned with `docs/spec-v1.md`.

## 1. Scope

This document defines message classes and protocol contracts between:

- clients
- server relay
- kernel
- provider adapters

It is intentionally transport-agnostic at the message level.

Current implementation baseline:

- local daemon-client communication now defaults to a daemon-owned WebSocket transport with pushed events
- the older Unix-socket request/response IPC path still exists for harnessing/tests and compatibility
- daemon-OpenCode communication uses native local HTTP control plus SSE events

Target direction:

- one kernel-owned bidirectional transport for terminal clients
- one transport shape for both local and remote terminal members, with relay as a forwarding layer rather than a second authority
- relay is an external member that speaks the same transport contract, not a second kernel
- generic agent-facing transport remains deferred; current agent integrations continue to use native/provider-specific adapter protocols
- WebSocket is the current and recommended transport for the kernel-client path

## 2. Design Principles

- preserve native provider interaction semantics, using PTY passthrough where required and structured local provider protocols where they are stronger and officially supported
- reserve `/...` as the Chariox command namespace
- keep structured control surface intentionally small
- isolate capability/control errors from terminal stream
- ensure all user-generated in-transit payloads are session-E2E encrypted on remote transport, including prompts, workflow inputs/outputs, and transferred/attached artifacts
- this requirement applies equally to:
  - self-hosted relay deployments
  - any later managed relay deployment
- relay must only ever see opaque encrypted payloads plus the minimum metadata required for routing and liveness
- serialized instants must be absolute UTC values: RFC 3339 strings use a `Z` suffix and numeric
  timestamps are Unix epoch values; timezone-free wall-clock strings are not protocol instants
- an IANA timezone is carried only when the timezone is part of the operation's semantics, such as
  a recurring cron schedule; fixed UTC offsets are not timezone identities

Current sequencing note:

- OpenCode is the reference provider for the current development cycle
- protocol and adapter boundaries should stay future-compatible, but they should not be generalized prematurely at the expense of finishing the OpenCode-first runtime
- web/mobile clients come before multi-provider expansion in the current rollout order
- same-kernel remote clients should fit the same kernel-owned protocol rather than a separate remote-only API
- same-kernel remote agents remain part of the architecture, but their generic transport contract is intentionally deferred until Chariox has integrated more than one concrete agent family

## 2.1 Node Roles

The protocol should distinguish at least these logical roles:

- `client`
- `agent_endpoint`
- `relay_or_server`

The kernel remains the workspace/runtime authority in all cases.

## 3. Protocol Lanes

## 3.1 Terminal Lane (Provider Output Stream)

Purpose:

- user keystrokes to provider PTY
- provider stdout/stderr/control sequences to clients

Semantics:

- byte-stream-like behavior
- no requirement for structured parse by Chariox for ordinary non-command traffic
- for providers with structured event streams, Chariox MAY render provider output into the client terminal without treating PTY bytes as the source of truth for turn lifecycle
- for same-kernel remote clients, the terminal lane should still be kernel-routed; relay changes the path, not the workspace authority

Suggested events:

- `terminal.input`
- `terminal.output`
- `terminal.resize`

`terminal.resize` carries the session id, dimensions, and an optional provider-run id. General
Chariox clients may omit the provider-run id to resize the session's active run. Provider-native
clients must target their own provider run so concurrent native terminals cannot resize each
other. When that run is projected from a leased worker, the home kernel forwards the resize to
the worker PTY and reports success only after the worker applies the requested dimensions.
Clients retry the latest dimensions after a transient transport failure.

OpenCode-specific note:

- OpenCode should graduate from PTY-polled `terminal.output` to adapter-fed output derived from its local event stream
- incremental assistant text should come from provider message-part delta events
- terminal rendering remains kernel-owned even when the source is a structured provider event stream
- the current protocol surface should be proven against OpenCode first before new provider families drive further adapter generalization

## 3.2 Capability Lane (Structured Daemon Actions)

Purpose:

- daemon-owned operations invoked from Chariox slash-command dispatch

Suggested request envelope:

- `request_id`
- `session_id`
- `capability`
- `args`
- `sent_at`

Suggested result envelope:

- `request_id`
- `status` (`ok` | `error`)
- `result` or `error`
- `completed_at`

Capabilities in v1:

- `shell.run`
- `dir.tree`
- `file.view`
- `file.edit`
- `screenshot.capture`
- `git.info`
- `file.transfer`
- `file.attach_transferred`
- `context.compact` (mapped from `/compact`)
- `schedule.*`

Slash-command routing rules:

- `/...` is parsed by Chariox before PTY forwarding
- `/<provider> ...` is resolved against the focused provider command catalog
- ordinary non-command input continues through `terminal.input`
- unsupported provider versions MAY produce warnings, but MUST NOT disable best-effort `/<provider> ...` completions by default

## 3.3 Control Lane (Structured Daemon->Provider Adapter)

Canonical operations in v1:

- `attach_file`
- `request_memory_update`
- `request_compaction_summary`

These operations are not typed by users into ordinary terminal traffic.

Chariox MAY route `/<provider> ...` invocations into the control lane after resolving the focused provider command catalog.

OpenCode-specific structured adapter contract:

- prompt submit maps to the provider session prompt operation
- `/<provider> ...` command invoke maps to the provider session command operation
- turn abort maps to the provider session abort operation
- provider lifecycle and output state are consumed from the provider event stream rather than inferred from PTY EOF or PTY idleness
- later providers such as Claude Code and Codex should fit behind the same daemon/client contract after the OpenCode-first cycle is closed

Provider hidden-context injection contract:

- Prompt submission from the kernel to a provider adapter is conceptually a `PromptEnvelope`, not one concatenated string.
- `visible_user_prompt` is the only prompt body that may be shown in Chariox prompt blobs, terminal input history, native provider prompt boxes, or user-facing prompt echoes.
- `hidden_system_context` carries Chariox runtime/system prompt material: runtime instructions, Workspace Live Sync managed/tracked instructions, native permission rules, workflow-level prompts, node-level instructions, granted capability summaries, continuation instructions, and utility-call instructions.
- `attachments` remain structured prompt attachments and are not used to smuggle hidden system instructions.
- `manifest` records prompt template ids, template hashes or versions, assembly conditions, and the provider injection channel selected for the turn; the manifest is audit/debug metadata, not prompt UI content.
- Chariox MUST NOT implement hidden context by prepending text to `visible_user_prompt` and later redacting it from UI surfaces.
- The relay MUST treat prompt envelopes as opaque encrypted payloads and MUST NOT inspect, transform, redact, or split visible versus hidden prompt fields.

Provider adapter hidden-context channels:

- Codex adapters MUST send hidden context through `thread/start.developerInstructions` or `thread/resume.developerInstructions` when a Codex thread is created or resumed. Codex does not accept this context through `turn/start`; for kernel-managed Codex runs, the kernel MUST hot-reload the Codex thread before a turn when the assembled hidden context fingerprint changes.
- OpenCode adapters MUST send turn-scoped hidden context through the provider session prompt request `system` field, currently `POST /session/{id}/prompt_async` body `system`.
- Claude Code adapters MUST send turn-scoped hidden context through the `UserPromptSubmit` hook response `hookSpecificOutput.additionalContext`.
- If a provider channel is unavailable, the adapter may run without hidden context for that turn or restart the provider process with an initialization-scoped system prompt only when the caller explicitly accepts that behavior; it must not silently fall back to visible prompt injection.
- Live provider drills validate direct provider hidden-context channels in current supported harnesses. Prompt assembly changes that touch these channels must keep or update `pnpm --filter @chariox/cli run provider-context-injection:drill`.
- End-to-end prompt assembly changes must also keep `pnpm --filter @chariox/cli run prompt-assembly:drill` passing. That drill edits a temporary `~/.chariox/prompts/runtime/base.md`, runs real Chariox provider turns for Codex/OpenCode/Claude, verifies the model sees the hidden registry token through the provider-native hidden channel on successive turns, and verifies Chariox user-prompt history does not contain the hidden token.

Prompt template storage:

- Chariox prompt templates are user-owned markdown files under `~/.chariox/prompts`.
- Source-controlled defaults may be materialized there for first run, but runtime assembly reads from the registry path rather than hardcoding prompt text in adapter code.
- Required templates include runtime base instructions, Workspace Live Sync managed instructions, Workspace Live Sync tracked instructions, native permission instructions, slice runtime instructions, MCP/skill continuation instructions, workflow turn/completion/intermediate-output templates, and utility-call templates.
- Cloud editing, if introduced later, edits this registry model and must not create a second prompt source of truth.

Provider-local visibility caveat:

- Chariox UI and protocol prompt blobs must hide `hidden_system_context`, but provider-local histories may still store it in provider-native form.
- Current provider harnesses expose hidden context in internal histories/transcripts: Codex history APIs, OpenCode message `info.system`, and Claude transcript `hook_additional_context`.
- The protocol guarantee is therefore “not visible in Chariox/native prompt input surfaces,” not “unrecoverable from provider-owned local state.”

## 3.3.1 Agent Endpoint Direction

Longer-term agent runtimes compatible with Chariox should speak a daemon-facing endpoint contract rather than requiring the daemon to launch only local child processes.

Required properties:

- bidirectional messaging
- explicit prompt or turn lifecycle
- explicit tool/runtime events
- health and capability advertisement

Existing providers like OpenCode may continue to be adapted through their native protocols.

## 3.3.2 Native TUI Agents

Native TUI agents let a user run a familiar provider CLI UI while the Chariox kernel remains the session authority.

Current commands:

- `chariox codex [session-ref] [--kernel-port PORT|--kernel-url URL]`
- `chariox opencode [session-ref] [--kernel-port PORT|--kernel-url URL]`
- `chariox claude [session-ref] [--kernel-port PORT|--kernel-url URL]`

Semantics:

- if no session ref is provided, Chariox creates a session and its first native TUI agent
- if a session ref is provided, Chariox attaches a new top-level native TUI agent to that Chariox session
- local native TUI launchers default to the web-dev kernel at `ws://127.0.0.1:43119/kernel`; `--kernel-port` selects another local kernel port
- a native TUI launch never attaches to an existing provider run; every native TUI agent owns its own provider run
- prompts from the provider TUI are intercepted and submitted through the same kernel prompt path as Chariox clients
- prompts from Chariox clients are forwarded through the kernel-managed provider run so the provider TUI observes the same turns
- native TUI provider runs are marked with `client_interface = native_tui`
- Chariox clients must treat model/variant controls for those runs as provider-controlled; provider-native changes may be recorded when observable, but Chariox-side parameter mutation is disabled for the active native TUI run
- daemon health reports `duplicate_chariox_agent_bindings` when more than one active Chariox provider run is bound to one session/agent, and `multi_interface_agent_bindings` when active Chariox and native TUI provider runs are bound to the same session/agent
- daemon health `provider_catalog` reports whether provider/model metadata is cached, expired, and how old it is; clients should surface stale catalog state near provider/session launch diagnostics
- daemon-tracked provider process listings include PID and best-effort current RSS (`resident_set_bytes`) when the host can read it; clients should surface this beside teardown safety so provider memory pressure is diagnosable without external process tools
- `ExportDebugBundle { session_id, bundle_label, limit }` is the shared session-scoped debug bundle request for TUI, web, and remote clients. The caller supplies only a session id, optional label, and optional record limit; the kernel filters current structured logs by `session_id`, sanitizes the label, writes `manifest.json` and `logs.ndjson` under its own debug-bundles root, and returns `DebugBundleExported { bundle_dir, manifest_path, logs_path, log_root, record_count, limit }`. Clients must display the returned paths as kernel-machine-local paths and must not send arbitrary output directories.
- Agent inspection and pane chrome should surface the session home kernel/machine alongside agent placement, worktree, provider run, extension grants, and remote extension manifest state so users can distinguish session authority from worker execution.

Remote native TUI composition:

- remote native TUI mode MUST compose existing protocol paths rather than create a second prompt/runtime protocol
- provider-native TUIs and Chariox TUIs attach to the home kernel session through the same client/session attachment semantics used locally
- provider-native TUI prompts MUST enter the home kernel through the same `SubmitPrompt` path as Chariox prompts
- the home kernel MUST dispatch remote execution through the existing leased-agent relay path (`SubmitLeasedPrompt`, remote prompt attachments, remote MCP/skill checks, and related completion/cancel paths)
- `ExecutionLeaseCreated` MUST include `relay_peer_protocol_version`; the home kernel must reject a worker that omits it or advertises a lower version before `SpawnLeasedAgent`, and persist the negotiated version on the binding. Restored bindings with a missing or mismatched version MUST be rejected before any leased prompt/native-provider dispatch and require a rebind, so stale remote kernels fail with an upgrade/restart action instead of breaking during provider tool calls
- the worker kernel MUST talk to the provider through the same kernel-provider adapter/server path used by ordinary worker-owned provider runs
- worker output, notices, completions, and permission interactions MUST return to the home kernel through existing leased runtime projection and native interaction relay paths
- relay peer protocol v3 extends leased runtime projection with an optional worker `provider_run` snapshot. The home kernel MUST project that worker-owned run onto the home session/agent, including any resolved `provider_session_id` and resume state, because some providers such as Codex only expose the durable thread id after the first turn rather than at launch time.
- relay peer protocol v7 correlates projected completions with `home_prompt_id`. Workers retain the latest settled completion for pull-based replay, and home kernels MUST ignore a stale replay when that prompt is no longer active. This makes a completion recoverable when a fire-and-forget worker projection is lost without allowing the replay to settle a later queued turn.
- relay peer protocol v8 carries an optional exact worker-local skill requirement set for native TUI launch and prompt submission. Standard workers validate matching local package hashes without receiving package payloads; home-managed slices may materialize packages first. Workers project the validated set onto the leased backing agent so per-prompt hidden skill context stays worker-local and grant revocations cannot leave stale skills active.
- relay peer protocol v9 adds idempotent queued-prompt steering for leased agents. The home kernel keeps queue/history authority and removes the queued prompt only after the worker acknowledges provider delivery; the worker keys delivery by the home queued prompt ID, rejects stale active-turn targets, and replays acknowledgements without injecting duplicate provider input.
- relay peer protocol v10 forwards provider-targeted terminal resizes to leased worker PTYs. The worker validates that the requested provider run belongs to the leased agent and acknowledges the applied dimensions; the home kernel does not report a remote resize as successful before that acknowledgement.
- the provider-native proxy/launcher MAY translate home-kernel session output back into provider-native UI protocol or PTY rendering, but it must not become a session authority or bypass the home kernel prompt queue
- the relay remains transport-only and must not inspect or transform provider-native prompts, outputs, attachments, permissions, or history
- slice-backed native TUI mode follows the same contract: provider TUIs and Chariox TUIs attach to the home kernel session, `slice_ref` selects a home-managed worker execution environment, and the slice worker uses the same worker-owned provider adapter/server path as remote leased agents

Native TUI MCP and skill placement:

- local native TUI provider runs use the same agent-scoped grant filtering as ordinary local provider runs, so only MCPs and skills granted to that agent are injected or rendered for that run
- standard home-worker native TUI may expose home-authorized remote extension manifests to the worker. Home-owned active extensions remain grant/revoke authoritative on the home kernel and execute on home through relay peer calls; the worker only advertises the manifest and forwards calls. Each forwarded call carries `invocation_id`, optional `provider_tool_call_id`, `attempt`, and optional `idempotency_key`; home reconstructs the current tool definition before execution and rejects stale or forged worker metadata, including calls from a worker provider run that is not the current remote binding.
- when an extension is explicitly worker-local, the home kernel may still compute grant-derived remote MCP requirements and pass those requirements to the worker launch/prompt path so the worker can fail fast on missing or mismatched local worker definitions before provider execution
- slice-backed native TUI may synchronize Chariox skill packages from the home kernel to the child worker because the slice is home-managed; this is not a general remote-machine install mechanism
- slice-backed native TUI still executes worker-local MCP commands on the worker side, so worker-local MCP commands and environment must be available in the slice image or injected slice environment; Chariox vault credentials remain home-owned and are exposed to slice workers only through home-authorized credential proxy calls and one-operation secret injection
- capability grants remain agent-scoped in all modes; native TUI launch must not expose ungranted local/user MCPs or skills just because the provider CLI can see them natively

Native TUI permissions:

- provider-native permission requests MUST be represented as one agent-scoped, kernel-owned `RuntimeInteraction`
- that interaction MUST be projected to every Chariox TUI attached to the session, regardless of whether the current turn was submitted from a Chariox TUI or provider-native TUI
- answering from a Chariox TUI resolves the kernel interaction and the provider adapter/proxy forwards the resulting decision to the provider
- where a provider-native TUI can submit an approval response through a stable proxy or hook seam, the native response MUST resolve the same kernel interaction rather than bypassing it; first valid resolution wins
- if the provider only exposes the approval through a rendered PTY, Chariox may detect the rendered prompt and create the kernel interaction, then inject the resulting decision back into the PTY using the provider's native selection semantics

Provider-native credential enrollment callback bridge (local daemon protocol 241):

- an attached client first sends `ArmDeploymentCredentialEnrollment` to its home kernel. The arm is bound to the authenticated user and relay realm plus the exact enrollment, profile, target version, session, focused agent, and attachment ownership
- arms are kernel-memory-only, one-time, bounded by TTL and capacity, and shared by local and relay command routers for that kernel process. Expired, mismatched, consumed, or wrong-kernel arms fail closed
- the expected hosted helper subject is `deployment-credential-enrollment:<enrollment_id>`. `RequestCredentialEnrollmentInteraction` is accepted only from the encrypted relay client lane when relay-authenticated caller metadata identifies that exact `Service` subject and the arm's user and realm. Request-body identity is not accepted as authorization
- a relay-authenticated `Service` caller is request-scoped: the kernel rejects every local-daemon request other than `RequestCredentialEnrollmentInteraction`, even when the token's user is a session member
- the helper sends the provider authorization URL inside the encrypted kernel payload. Cloud and the transport-only relay receive no provider URL or callback content
- the kernel creates one ordinary agent-scoped `RuntimeInteraction` containing the authorization URL, a `Cancel` choice, and a custom choice whose `input_kind` is `secret`. Every attached web or TUI client receives the same interaction, and the first valid response wins
- cancel and timeout return terminal status without a callback. A submitted callback is returned only in the awaiting helper response; it is not stored in session state, projected as an event, included in command payload logging, or entered in the in-memory or persistent command-result cache
- this bridge drives the official Claude CLI callback seam only. Chariox does not implement OAuth authorization, token exchange, PKCE generation, or provider credential storage

Native TUI hidden context:

- granted skill prompt context and other Chariox-only prompt injections MUST be delivered on the provider-facing path without becoming visible provider-TUI text
- Codex native TUI hidden context MUST use the same Codex turn-scoped `developer_instructions` channel as ordinary Codex provider runs
- OpenCode native TUI hidden context MUST use the same OpenCode prompt request `system` field as ordinary OpenCode provider runs
- Claude Code native TUI MUST use the `UserPromptSubmit` hook `additionalContext` path for hidden context; the hook emits a scoped context request id, and the Chariox CLI bridge or worker kernel writes the matching context response before the hook returns
- Claude hook context responses are scoped to the session, agent, and provider run; they must not expose broad kernel authority or accept arbitrary provider-origin file paths
- if a Claude hook context response is unavailable before timeout, the provider-facing hidden context is empty and the native TUI remains coherent; Chariox MUST NOT fall back to visible PTY prompt injection for skill bodies or system prompt blocks
- local Claude native TUI can answer hook context requests through the launcher bridge and home kernel; remote/slice Claude native TUI answers them on the provider-execution side so worker-local or slice-isolated skill material is used

Provider-specific transport:

- Codex uses a native WebSocket proxy in front of a Codex app-server endpoint and binds the observed Codex thread to the Chariox provider run.
- OpenCode uses a native HTTP proxy in front of a launcher-managed `opencode serve` endpoint. The kernel binds its provider run to the proxy endpoint, while the provider TUI attaches to the same proxy/provider session.
- Claude Code has no stable provider UI/server split. Local and remote native TUI mode therefore use a kernel-owned PTY: the provider process runs where execution belongs, and the launcher streams/render-controls that PTY while the kernel projects prompts, output, attachments, status, and supported interactions back into the Chariox session.

## 3.3.3 Metaagent Event Prompts

Metaagent event notifications are Chariox runtime-origin prompts. They are not
hidden provider context, and adapters MUST NOT deliver them through hidden
system/developer channels. The visible prompt text should identify the message
as a Chariox runtime event, summarize what happened, and point the metaagent to
`chariox.meta.read_event`, `chariox.meta.turn_overview`, or
`chariox.meta.turn_blob` for detail payloads that are too large for the prompt.

Each recorded event carries prompt-delivery state so a reconnecting metaagent
can reconstruct what happened:

- `recorded`: the event exists in the kernel inbox but has not reached a provider prompt path
- `submitted`: the kernel submitted a visible event prompt to the provider path
- `steered`: the event was attached to an already-active metaagent turn
- `queued`: the event prompt is queued behind an active turn
- `delivered`: the provider accepted or completed the corresponding event prompt
- `failed`: delivery failed and the event should be visible as a liveness fault until retried or superseded

Provider-specific delivery behavior:

- Codex: event prompts use the ordinary visible user-prompt path for the bound
  Codex thread. If the Codex run is active and supports same-turn steering,
  Chariox may mark the event `steered`; otherwise the prompt remains queued and
  visible in Chariox prompt history.
- OpenCode: event prompts use the provider session prompt API as visible
  prompt content. Hidden `system` context remains reserved for runtime
  instructions and MUST NOT carry event notifications. OpenCode event-stream
  completion should update delivery status without relying on PTY idleness.
- Claude Code: event prompts are submitted through the same visible prompt path
  used for user turns. `UserPromptSubmit.additionalContext` remains reserved for
  hidden context such as skill bodies and MUST NOT carry event notifications.
  When Claude is exposed through a kernel-owned PTY, Chariox may render or steer
  the visible event prompt through that PTY only as provider-visible prompt
  input, not as a hidden hook response.

Required metaagent events, including owned-agent turn completion, owned-agent
turn failure, and owned regular-agent runtime interactions, must preserve
ordering per metaagent. Optional workflow subscriptions may share the same
visible prompt mechanism, but filtering and durable inbox state remain
kernel-owned. A missing provider run or delivery failure must be surfaced in
the event status and retry path rather than being silently dropped.

## 3.4 Workflow Coordination Semantics

Multi-agent workflow coordination is a daemon-owned structured protocol concern.

Delivery priority inside v1:

- circular topology is the earlier implementation target
- hierarchical topology remains in scope for v1, but is expected to land later in v1 after lower-level runtime and protocol foundations are stable

Required rules:

- node-to-node communication MUST use structured handoff payloads
- workflow progression MUST be driven by node completion reports, not raw provider turns
- workflow routing, barrier/fan-in handling, and termination decisions MUST NOT depend on forwarding raw terminal transcript output between agents

## 4. Common Message Envelope

All structured messages should carry a minimum common envelope. Some fields are lane-specific or message-class-specific.

Common fields:

- `version` (protocol version, currently `v2` for the shared local daemon protocol)
- `lane` when applicable (`capability` | `control`)
- `type` (event/action identifier)
- `request_id` when request/response matching is needed
- `command_id` when the message represents a kernel command or a command-caused event
- `session_id`
- `agent_id` when agent-scoped
- `provider_run_id` when provider-scoped
- `workflow_run_id` when workflow-scoped
- `node_run_id` when node-scoped
- `target_node_id` or `target_node_run_id` when routing workflow handoffs
- `payload`
- `meta` (timestamps, source attachment id, causation id, correlation id, trace id)

Future unified node-transport fields should also allow:

- `connection_id`
- `attachment_id`
- `member_role`
- `event_id`
- `resume_from_event_id`

Relay peer protocol v15 carries the home kernel's private hidden prompt context in
`SubmitLeasedPrompt`. This preserves catalog manifest markers and other kernel-owned
context when a queued prompt is dispatched to a leased worker; older workers are
rejected by the existing relay peer version check.

Relay peer protocol v25 requires `SubmitLeasedPrompt.expected_profile`, containing
the home-selected provider, stable provider-account ID, optional model, and optional
effort. It carries no credentials. The worker reconciles this profile before replay
or admission, using its lease owner's existing account registry. An unavailable
account blocks admission rather than selecting a different account. Profile changes
and prompt admission are serialized per leased agent through provider launch and
prompt ownership assignment. Confirming an unchanged profile is safe during a
replayed active turn; changing it requires an idle agent. This makes the next prompt
use the profile committed at home even if an earlier worker update acknowledgement
was lost. Incompatible peer versions must be rebound before dispatch.

Version 25 also removes the separate `ForwardWorkflowProviderFailure` request and
its acknowledgement. Workers settle failed leased turns locally without waiting
for a home RPC. The existing runtime projection carries the terminal diagnostic
and correlated, replayable completion. The home settles only the matching active
turn, reserves agent admission, releases the failed workflow's workspace claim,
and confirms the selected substitute on the worker before advancing queued work.
Rejected profile acknowledgements preserve the queue. Delayed managed projections
cannot re-establish a cleared worker-run binding after a profile change.

## 4.1 Current Kernel Transport Baseline

For the current local baseline, the kernel exposes a request/response plus pushed-event surface over a daemon-owned WebSocket transport.

Transport scope (current definition):

- connects clients (CLI, relay, agent adapters)
- maintains live session state subscriptions
- emits output/notices/config updates to attachments
- enforces prompt flow control policies (queue advancement, idle/timeout completion, cancellation transitions)
- provides request/response dispatch for the local transport
- bridges the transport contract across local and remote transports

Current implementation notes:

- the TypeScript CLI now defaults to `ws://127.0.0.1:${CHARIOX_KERNEL_PORT:-43118}/kernel`
- the Rust daemon process hosts that WebSocket listener directly
- the older Unix-socket local IPC path still exists for daemon harnessing/tests and compatibility shims, but it is no longer the primary CLI transport
- the current wire shape now supports request/response plus pushed kernel events over one long-lived connection
- subscriptions carry optional `resume_from_event_id`
- the kernel emits monotonic in-process `event_id` values on pushed events
- heartbeat events are part of the current transport so the CLI can detect and recover stale connections
- reconnect/resubscribe is now part of the intended local CLI behavior
- current replay is bounded by the daemon's retained recent-event window; if a resume cursor falls outside that window, the M4.5 contract requires an explicit replay-gap response plus a fresh projection snapshot
- event ids should not be treated as daemon-restart durable until a persisted event log or equivalent projection checkpoint/tail-event store lands

Current pushed event contract:

- all pushed events use the `KernelOutgoingFrame::Event` envelope with monotonic `event_id` plus an `event` payload tagged by its `event` string
- `terminal_output` carries terminal records and should be used for terminal append/update rendering without forcing `session.state.get`
- `runtime_notices` carries runtime notices for the subscribed attachment/session
- `assistant_message_completed` carries `session_id`, `provider_run_id`, optional `agent_id`, `message_id`, and `completed_at_ms`
- `session_snapshot` is the full subscribed-session projection and remains the fallback after attach, replay gaps, explicit recovery, and structural changes
- `agent_activity_changed` carries `session_id` and the complete agent activity map for activity-only projection updates
- `provider_run_changed` carries `session_id` and the current provider run, or `null` when no provider run is active
- `session_metadata_changed` carries `session_id` and a `metadata` patch with alias, last-used timestamps, hidden state, focused agent, and workspace live-sync mode
- `runtime_interactions_changed` carries `session_id` and the current active runtime interactions for permission/choice prompts
- `waiting_room_inventory_changed` carries only `inventory_version` and requires clients to refetch the full waiting-room snapshot when fields outside the row patch change, including provider accounts, Git credentials, external provider sessions, relay inventory, remote kernels, and terminals
- `waiting_room_rows_changed` carries `inventory_version`, `schema_version`, `generated_at_ms`, optional `launch_target`, changed session rows, and `removed_session_ids`; clients should apply it as a row patch instead of refetching the full waiting-room snapshot
- `provider_catalog_changed` carries `generated_at_ms` and the current provider catalog
- `slices_changed` carries `generated_at_ms` and the current slice list
- `workflow_run_updated` carries `session_id` and the updated workflow run for workflow-run-only updates
- `heartbeat`, `transport_resumed`, `replay_gap`, `session_unavailable`, and `transport_closed` are transport/recovery signals; heartbeat and successful resume should not force full session, waiting-room, or prompt-history reads, while replay gaps require clients to discard optimistic deltas and request a fresh projection

Managed remote-kernel control uses local daemon protocol version 280. The kernel
client surface includes:

- Waiting Room inventory fields for provider accounts and safe Git credential
  summaries
- managed-environment create, list, detail, lifecycle, keep-running, and transfer
  preparation requests
- direct managed-context transfer start and status requests plus an explicit
  launch-target request
- multi-Workspace Project updates and exact slice repository selections

Every kernel that implements the direct source side of this contract advertises
`managed_context_source_protocol_version: 1` in its Cloud relay-presence
metadata. Cloud lists a kernel under `Kernel context from` only while that
marker, the kernel relay public key, an active owner-bound machine identity, and
a fresh authenticated heartbeat are all present. The capability marker is
independent of the local daemon protocol version so Cloud never infers transfer
support from an unrelated client protocol bump.

These messages coordinate a launch but do not move runtime authority into Cloud or
the client. A source-backed launch is bound to one source target, one target
kernel, one context id, and one plan digest. Package chunks travel through the
encrypted relay peer lane directly between those kernels. The target durably
commits each offset before acknowledgement and returns an idempotent consumed
receipt after completion. Retryable disconnects resume from the committed target
offset. Clients must surface failure and must not substitute Empty context.

Kernel-context schema v2 makes a stdio MCP portable only when its executable,
working directory, and referenced runtime files are contained by the dedicated
package directory `<user-mcp-root>/<mcp-name>/`. Export snapshots that directory,
hashes every file, preserves executable bits, and normalizes command paths before
transfer. Host-path arguments, ambient or literal credential environment values,
and paths outside the package are rejected. Import verifies the package before
materializing it below the target user's MCP root and rewriting command paths.
HTTP MCP definitions carry configuration only and never a local runtime package.

An imported managed Project reports target-owned Workspace and worktree paths.
The target kernel persists and publishes that Project before it returns the launch
target, so a retry or restart cannot expose a launch target that session creation
cannot resolve.

Minimum request set:

- `session.create`
- `session.list`
- `session.resolve`
- `session.attach`
- `session.detach`
- `session.delete`
- `agent.spawn`
- `agent.destroy`
- `agent.focus`
- `agent.cycle`
- `agent.list`
- `provider_run.launch`
- `session.state.get`
- `session.notice.poll`
- `prompt.submit`
- `prompt.complete`
- `prompt.cancel`
- `session.config.update`
- `terminal.output.poll`
- `terminal.resize`
- `session.end`

Minimum response/result shapes:

- session creation returns structured session metadata
- session resolution returns structured session metadata
- attach/detach returns structured attachment metadata
- agent lifecycle/focus operations return structured agent metadata plus updated focused-agent state where relevant
- provider launch returns structured provider-run metadata
- session state reads return canonical queue and config state
- notice polling returns structured daemon notices scoped to the requesting attachment within the session
- prompt submission returns structured prompt status (`started` or `queued`) plus canonical session state
- prompt completion returns structured completion details and the next started prompt when relevant
- prompt cancellation returns the updated prompt state; for provider-backed turns the daemon advances queued work only after the provider confirms the stop
- config update returns canonical session config state, version, and updated session state
- terminal output polling returns structured terminal-output fan-out records, including distinct provider text, reasoning, tool, error, status, and transient `provider_terminal` output kinds
- `provider_terminal` carries fullscreen native-renderer bytes only; clients must not treat it as semantic transcript, turn activity, history, or completion evidence
- end-session returns structured final session metadata

Current session-management semantics:

- user-facing clients should prefer `session.delete` over an implicit "end on exit" model
- `session.resolve` and `session.delete` accept a `session_ref` that may be:
  - full session id
  - unique session-id prefix
  - alias
  - unique alias prefix
- `session.create` accepts an optional alias
- deleting the currently attached session invalidates the attachment and the client should transition to an unattached "no session" state instead of forcing process exit
- `session.delete` is a real delete operation: after runtime teardown the session is removed from the daemon registry and can no longer be listed, resolved, or reattached
- if a session reference is ambiguous, the daemon rejects it with a structured ambiguity error

Current agent-management semantics:

- the local daemon API now includes top-level session-agent management operations (`spawn`, `destroy`, `focus`, `cycle`, `list`)
- focused-agent state is part of canonical session state and is intended to determine which top-level agent receives direct user interaction
- direct prompt submission now targets the focused top-level agent in the local runtime
- provider runs are now tracked per top-level agent and the daemon can park/resume them as focus changes or the session returns to idle
- session history and terminal-derived structured output records are now agent-scoped for the local multi-agent path
- pane-capable clients can now render per-agent transcript surfaces from daemon-owned state, although the current TypeScript CLI split-pane surface is still an initial slice rather than the final generalized layout

Local cancellation policy:

- any currently attached client in a session may request cancellation of that session's active prompt
- cancellation is session-scoped rather than attachment-owned because the active provider turn is shared session state

This local API MUST remain daemon-owned, local-first, and compatible with later workflow-mode runtime surfaces.

Architectural note:

- the WebSocket request/event transport is the primary CLI path
- the request/response IPC surface remains a bootstrap, harness, and compatibility transport
- both transports should normalize mutating requests into `KernelCommand` values during the M4.5 refactor
- future kernel-client and kernel-agent communication should converge on one long-lived event-capable connection model

Current local runtime note:

- the primary local CLI implementation is now a TypeScript OpenTUI client
- `chariox-cli` currently launches that TypeScript client through a small Rust compatibility wrapper
- the Unix-socket local transport remains useful for daemon smoke coverage and compatibility shims, but it is no longer the primary local user path

Current slice-management surface:

- `slice.list`, `slice.create`, `slice.get`, `slice.start`, `slice.stop`, `slice.delete`, `slice.display_endpoint.get`, `slice.logs.get`, `slice.state.save`, `slice.state.status`, `slice.state.reset`, and `slice.backup.create` are daemon-owned local requests.
- Local Docker slice records persist their assigned host port set in `local_docker_ports`; clients may display these diagnostics, but launch, relay, display, and log behavior must use the kernel-owned values rather than reconstructing ports from the slice id.
- Ordinary local Docker slices keep Docker's default container security profiles unless the home user explicitly sets `slices.linux.allow_unconfined_seccomp = true`; clients must present this as an advanced security compatibility option. Broker-backed managed hosts force that compatibility mode inside their dedicated rootless Docker daemon because the worker kernel's fail-closed bubblewrap boundary needs nested user, PID, and mount namespaces. The managed broker joins only that daemon's user and mount namespaces, pins each validated publication directory by device and inode, and publishes it as a stable broker-owned bind mount. Docker never receives the original publication path or a `/proc` magic link. Durable handle records let the broker reuse mounts after its own restart and recreate them after a daemon restart; slice destruction removes the records and mountpoints.
- Slice lifecycle status is `stopped`, `starting`, `stopping`, `running`, or `unhealthy`. Start must only report `running` after the worker kernel has been discovered; otherwise the slice remains `unhealthy` and diagnostics are available through `slice.logs.get`.
- Slice records also carry display-only operation diagnostics: `last_operation`, `last_operation_status`, `last_error`, and `last_operation_at_ms`. The kernel updates these on lifecycle operations and restart reconciliation; clients may render them in status/doctor views, but must continue to treat `status` as the lifecycle state and audit/log records as the detailed diagnostic source.
- Daemon health `slice_lifecycle.issues` identifies each unhealthy slice or failed slice operation by slice id/name, status, last operation/status/error, sessions, agents, and worktree so clients can point users directly to the affected slice before they open logs/audit or restart/delete it. `slice_lifecycle.provider_auth_issues` separately identifies attached-agent slices with no provider account summaries or with `unknown`/`not_configured` provider auth, including provider, alias/identity, sessions, agents, and worktree. Clients should surface this from kernel health and point users to `/slice doctor`, `/slice audit`, and slice auth login/import before they send more provider prompts.
- Local Docker slices use the kernel-configured relay when it has a token and a non-loopback `ws://` or `wss://` URL, so hosted Cloud and self-hosted relay deployments expose the slice worker on the same relay fabric as other remote workers. A hosted slice initially receives a short bootstrap token limited to registration and heartbeat. After discovering its relay key, the home kernel obtains a key-bound token that targets only that owner kernel and installs it through the encrypted peer lane with a fresh activation nonce. The worker queues the encrypted install acknowledgement before closing its bootstrap relay socket. After reconnecting, the worker must return that nonce in a worker-originated confirmation whose relay caller identity is bound to the installed key. The owner matches the slice, worker id, relay subject, full relay key, and nonce, then requires same-key live presence plus an encrypted ping before reporting the slice as running. The slice receives no Cloud session or machine credential. Loopback or incomplete relay configuration falls back to a private per-slice relay owned by the home kernel; clients should render the projected `relay_endpoint.private` flag rather than guessing from the URL.
- Kernel restart reconciliation must not leave runtime-only states active. Local Docker reconciliation inspects the host container: missing/stopped previously running slices become `stopped`, still-running or unverifiable runtime state becomes `unhealthy`, and interrupted `starting`/`stopping` transitions become `unhealthy`.
- `slice.logs.get` returns structured log entries for local Docker slice provisioner actions and recent container logs. Clients should render these as diagnostics only and must not treat log text as control data.
- Slice provider auth import/login/alias/remove requests are scoped by provider and the kernel owns displayed provider auth summaries. Removal purges the slice-side provider credential files and clears matching auth summaries from kernel state; `opencode` removes all `opencode:*` account summaries for that slice.
- Local daemon protocol v267 adds kernel-owned provider-account CRUD, profile-scoped auth/login/logout/status, provider-neutral usage meters, and account-aware catalog requests. Waiting-room projections expose safe profile metadata and materialization state, never profile paths or credential payloads.
- `GetProviderCatalog` carries provider/profile overrides plus `local`, `worker`, or `slice` execution location. Cache identity includes owner, request/profile selection, and location. Worker/slice requests require a matching kernel-projected materialization record.
- Provider account selection is immutable within one provider run. Updating an existing agent's provider/account/model uses the normal bounded context-handoff and run replacement lifecycle; it does not mutate provider auth in place.
- Relay peer protocol v18 carries the selected encrypted provider-account materialization before leased-agent spawn. The relay routes the opaque packet only. The worker validates lease ownership, materializes a distinct profile root, and launches the existing adapter/native-TUI path with that profile environment.
- Relay peer protocol v22 adds managed-slice relay credential activation and renewal. Relay tokens remain redacted from debug output and travel only inside encrypted peer payloads. The slice validates the token subject, owner target, machine, action set, expiry, and worker-key thumbprint before installing it. A token change forces a new relay socket; the relay independently closes active scoped connections at expiry. Before the refresh window closes, the slice requests a replacement from its recorded owner over the same encrypted owner-targeted lane.
- Relay peer protocol v23 adds an offline-recovery credential beside the short-lived active credential. The recovery token is bound to the same slice relay key and exactly one owner target, permits only `daemon.register`, `daemon.heartbeat`, and `peer.request`, and is capped at 30 days. It cannot route packets or receive peer events. The slice persists it atomically with the relay owner and key, uses it only when the active credential is missing or expired, then asks that owner for a replacement active/recovery pair over the encrypted peer lane. Each successful refresh rotates the recovery credential. A restart after an outage spanning active-token expiry must retain this same-key recovery path; a key or owner mismatch fails closed.
- Relay peer protocol v24 adds the nonce-bound `ConfirmManagedSliceRelayToken` request and `ManagedSliceRelayTokenActivated` response. The bootstrap credential cannot send a valid confirmation because it lacks the worker-key-bound kernel identity required by the owner. Owner expectations and worker retries are short-lived and bounded; a matching confirmation remains idempotently acknowledgeable for a short window so response loss does not strand the worker. A v24 owner rejects a worker that returns the older install response or cannot perform the confirmation handshake, so mixed-version activation fails closed and requires the owner and slice worker to run the same release.
- Slice saved state is a kernel-owned product concept, not a Docker-management UX. `slice.state.save` overwrites the active state for the slice, `slice.state.status` returns the active saved-state metadata, `slice.state.reset` removes the active state so future starts use the base slice image, and `slice.backup.create` creates a separate backup plus manual swap instructions. Saved state is composite: a Docker image tag and a `/home/slice` archive under the Chariox slice state root. Slice records expose only metadata (`saved_state_ref`, `saved_state_status`, `saved_state_updated_at_ms`); clients must not inspect archive contents or expose them to provider transcripts.
- `slice.create.from_saved_state` may reference an existing saved-state id/name. Local Docker restore uses the saved image tag instead of the configured base image and extracts the saved home archive into the fresh slice home volume before normal provisioning continues. Restore still allocates fresh ports, relay identity, and worker identity through the normal slice start path.

Current session-lifecycle note:

- the local implementation still exposes `session.end` as an internal/runtime operation, but the intended user-facing local client contract is persistent detached sessions plus explicit `session.delete`
- `session.end` and `session.delete` are intentionally distinct:
  - `session.end` is an internal/runtime operation and may still be reused for resumable daemon-owned transitions
  - `session.delete` is the user-facing destructive operation and removes the session from the daemon registry after teardown
- the current local implementation now uses 16-character lowercase hexadecimal session ids with optional aliases and unique-prefix resolution
- detaching the last terminal does not cancel an active provider turn or queued
  prompt backlog. The kernel keeps source attribution in private durable prompt
  state, advances queued prompts without a live attachment, writes output and
  completion to durable agent-scoped history, and retains unresolved runtime
  interactions for later attachments
- bounded terminal fanout records are recipient-scoped and are not the recovery
  source for a long disconnection. Reattachment uses the session snapshot and
  durable history before it accepts live tail output

OpenCode current runtime note:

- the daemon already routes OpenCode prompt submit through the provider-native local HTTP session APIs
- the daemon already consumes OpenCode output and completion through the provider event stream
- provider-native TUI mode can supply an external OpenCode structured endpoint so the native launcher can proxy both kernel and provider-TUI traffic before forwarding to `opencode serve`
- active-turn cancellation is routed through the OpenCode abort API and reconciled from provider events before queued prompts advance
- PTY remains a liveness/process-management surface for the OpenCode server process, not the primary prompt/output transport
- the same daemon-owned local request/response surface remains the client contract while the adapter becomes more provider-specific internally

## 4.1.1 Unified Node-Transport Direction

The intended node architecture now assumes that the kernel should eventually act as a general router for:

- local clients
- remote clients connected through relay
- local agent endpoints
- remote agent endpoints connected through relay

Recommended direction:

- one long-lived kernel-owned bidirectional protocol
- request/response messages for control
- pushed daemon events for prompt/session/provider updates
- relay forwarding without changing daemon authority

This does not require all provider adapters to use the same wire transport internally.

## 4.1.2 Shared Room environment protocol direction

This section defines the logical contract for the Room-owned browser and graphical Environment. It is a design contract, not part of local daemon protocol v268. Adding any request, response, event, or serialized field below requires the normal protocol version bump, snapshot update, minimum-client decision, and focused cross-boundary drill.

The current `session_id` is the wire identity for the product Room until a deliberate migration introduces `room_id`. New code must not create both identities for the same runtime domain. `environment_id` identifies the default shared Environment within that Room.

### Environment snapshot

A full Environment snapshot carries at least:

- `session_id`
- `environment_id`
- `runtime_generation`
- lifecycle and health state
- current saved-state generation when present
- Browser Controller, browser, desktop, and streamer health
- canonical viewport dimensions, scale, revision, and current owner
- ordered Room-visible tabs and the focused Tab
- present Actors and current input ownership
- active and recently terminal Actions
- snapshot event cursor

Health details may name a failed managed process and a safe diagnostic code. They must not contain environment variables, command lines with credentials, browser data, page content, clipboard content, or provider payloads.

### Tab and document identity

A Tab projection carries:

- `tab_id`
- optional controller-local target metadata restricted to diagnostics
- URL, title, lifecycle, and focus state
- `document_revision`
- last activity Actor and timestamp
- structured observation availability

An element reference is opaque to clients. Its validation scope includes `environment_id`, `runtime_generation`, `tab_id`, and `document_revision`. An action using a stale reference fails with `stale_element_reference` and returns enough metadata to request a fresh observation. It must not retarget by text, selector, index, or coordinates without a new explicit Action.

### Actor and presence projection

An Actor projection carries:

- `actor_id`
- kind, either `human` or `agent`
- safe display label and stable presentation color
- presence state
- current observed mode when useful
- owned input targets
- active Action IDs

Attachment identity and Actor identity are distinct. A human may reconnect through another Attachment and retain the same Actor identity. An agent may change provider runs without becoming another Actor. Presence never grants permission or input ownership.

### Action envelope

Every Browser and Computer Action uses one kernel-owned envelope:

- `action_id`
- optional idempotency key
- `session_id`, `environment_id`, and `runtime_generation`
- `actor_id`
- mode, either `browser` or `computer`
- Action kind and redacted arguments
- target kind and target identity
- optional `tab_id`, `document_revision`, or viewport revision precondition
- queued, started, and terminal timestamps
- state, one of `queued`, `running`, `completed`, `failed`, or `cancelled`
- redacted outcome or structured failure

Action acceptance validates Room membership, Environment generation, capability grant, target existence, preconditions, ownership, and queue capacity before execution. The kernel assigns order. Provider tool completion, a browser event, or a returned screenshot does not by itself prove Action completion; the Action ledger does.

Vault-backed input carries a credential reference and expected-target policy in the Action envelope. The resolved value travels only through the existing scoped secret-delivery path to the approved local input target. Keyboard text, clipboard content, and resolved secret values are never copied into Action history.

### Input targets and concurrency

Input target kinds are:

- `browser_tab`, identified by `tab_id`
- `desktop`, identified by `environment_id`

Observations do not reserve a mutation target. Their results carry the generation and revision observed so callers can detect staleness.

A structured browser mutation reserves its Tab. A Computer mutation reserves the desktop. If that mutation can affect the focused browser Tab, the kernel also reserves that Tab. A browser mutation that opens, closes, or focuses Tabs reserves the desktop and every affected Tab. Other mutations on separate Tabs may proceed concurrently. The kernel rejects or queues an Action when its required target is reserved; clients never implement their own lock queue.

The initial queue outcomes are:

- `accepted`, with the Action already running
- `queued`, with stable queue position or ordering metadata
- `rejected_busy`, when policy does not queue the Action
- `rejected_saturated`, when bounded queue capacity is reached
- `rejected_takeover`, when a human owns the target

Queue and reservation waits have bounded deadlines. Cancellation and process loss must release every target reservation.

### Human takeover

Takeover requests identify Actor, Environment, and input target. A successful response is not sent until the active agent Action has reached `cancelled`, `failed`, or `completed` and the target belongs to the human Actor.

Takeover emits ordered Action and ownership events. Every attached client projects the same transition. A takeover request is idempotent for the same Actor and target. A conflicting human request follows Room permission policy rather than last-writer-wins behavior.

Release is explicit. Disconnect may start a bounded expiry policy, but reconnect during that interval retains ownership. Expiry emits an ownership event and leaves the target unowned. It never assigns an agent automatically.

### Viewport contract

The canonical viewport carries:

- CSS width and height
- device scale factor
- desktop pixel width and height
- revision
- owner Actor when a user input owner controls resize

Clients submit viewport requests with the revision they observed. The kernel accepts one transition or rejects it as stale, unauthorized, unsupported, or unsafe. An accepted response is complete only when browser layout, desktop resolution, streamer dimensions, screenshot coordinates, and input coordinates agree on the new revision.

Viewer-only scaling is local presentation state and does not change the canonical viewport.

### Planned requests

The smallest planned request set is:

- `environment.state.get`
- `environment.start`
- `environment.stop`
- `environment.retry`
- `environment.viewport.update`
- `environment.input.takeover`
- `environment.input.release`
- `environment.action.submit`
- `environment.action.cancel`
- `environment.history.list`

Browser and Computer tools submit through `environment.action.submit`; they do not add provider-specific action request types. Existing public runtime MCP tool names may remain as adapters over this request.

Slice save, restore, reset, and backup remain the existing slice lifecycle requests. Their Environment effects appear through Environment lifecycle and generation events instead of a parallel save authority.

### Planned events

The smallest planned pushed-event set is:

- `environment_snapshot`
- `environment_lifecycle_changed`
- `environment_health_changed`
- `environment_tabs_changed`
- `environment_viewport_changed`
- `environment_presence_changed`
- `environment_input_ownership_changed`
- `environment_action_changed`

Each event carries `session_id`, `environment_id`, `runtime_generation`, and the normal monotonic kernel `event_id`. Structural deltas carry a base revision. A mismatched base revision or replay gap forces a fresh Environment snapshot.

### Recovery and history

Action history is kernel-owned and append-only. History entries use the Action envelope plus safe diagnostic and artifact references. Raw display frames, screenshots, DOM snapshots, network bodies, clipboard values, and secrets are not embedded in the ledger. Their bounded artifacts follow Room permissions and retention policy.

After reconnect, a client resumes from its last kernel event cursor. Replay preserves Action order and terminal state. After a replay gap, the client discards optimistic Actions and applies one full snapshot. It must not resubmit an Action unless the kernel reports that the original idempotency key is unknown or retryable.

Process recovery follows these rules:

- completed Actions are never repeated
- queued Actions remain ordered only when their preconditions and target generation still hold
- a running Action without durable completion proof becomes failed or cancelled
- stale element references fail and require rediscovery
- controller or browser recovery must not create duplicate Tabs
- streamer recovery does not change Environment, Tab, Action, or input ownership identity
- worker or kernel recovery reconciles ownership before admitting new mutations

### Compatibility policy

Protocol v268 clients know slice display endpoints and one-shot browser/computer tools but do not know the shared Environment contract. During migration:

- the kernel keeps the old tool names behind a compatibility adapter
- compatibility calls still enter the kernel-owned Action path once it exists
- an old client may observe the display but cannot claim human takeover or canonical viewport ownership
- the kernel rejects unsafe concurrent legacy mutations instead of allowing split authority
- unknown Environment events remain ignorable only when the client's behavior stays safe
- a client that needs takeover, Action history, stable Tabs, or canonical viewport requires the new minimum protocol version

No minimum version changes until a released client depends on the serialized contract.

## 4.2 Planned Command-Dispatch Surface

The current local API baseline does not yet expose slash-command discovery/invocation, but the protocol should reserve room for it.

Planned request types:

- `command.list`
- `command.invoke`
- `agent.command.list`
- `agent.command.invoke`
- `provider.auth.status.get`
- `provider.event.subscribe`
- `extension.install`
- `extension.list`
- `extension.bind`
- `extension.unbind`
- `mcp.runtime.list`

Planned command metadata fields:

- `command_path`
- `description`
- `source` (`builtin` | `custom` | `best_effort_catalog`)
- `provider`
- `provider_version`
- `catalog_version`
- optional `warning`

OpenCode adapter metadata additions:

- optional `provider_session_id`
- optional `provider_event_capabilities`
- optional `provider_command_source` (`catalog` | `provider_api` | `custom_files` | `merged`)

Planned provider auth status fields:

- `provider`
- `account_profile`
- `auth_state` (`authenticated` | `not_logged_in` | `expired` | `unknown` | `provider_not_installed`)
- optional `login_hint`
- optional `detected_version`

Current kernel-client metadata fields:

- `member_role` (`client`)
- `connection_mode` (`local_direct` | `relayed`)
- `protocol_version`
- optional `resume_from_event_id`

Deferred agent-endpoint note:

- OpenCode remains adapter-owned and continues to use native local HTTP control plus SSE events
- managed vs external OpenCode endpoint binding is the current agent-endpoint abstraction boundary in code
- a generic WebSocket transport for agent endpoints is explicitly deferred until after Chariox has integrated more than one agent family and can derive a better common denominator from real integrations

Planned extension metadata fields:

- `extension_id`
- `type` (`skill` | `mcp_server` | `command_pack` | `instruction_pack` | `hook`)
- `source`
- `version`
- `provider_support`
- `visibility_policy`
- `install_state`

## 5. Control Operations

## 4.3 Workflow Message and Endpoint Direction

The workflow model should use a minimal, general message envelope rather than predefined domain-specific fields.

Logical workflow message fields:

- `message`
- `recipients`
- `artifacts`

Rules:

- the workflow graph defines which recipients are valid from a given sender
- artifacts are intentionally open-ended
- the kernel validates message structure and routing before delivery
- each sender may emit at most one message per recipient in a single turn

Workflow endpoint direction:

- a workspace may contain multiple workflow definitions
- each workflow definition may expose multiple logical endpoints
- each workflow endpoint maps to one entry node in that workflow
- an endpoint may be invoked by a terminal user or by an external published API
- once accepted by the kernel, the workflow should treat the resulting input message the same way regardless of source
- disconnected subgraphs are allowed; a subgraph is reachable only if some endpoint points into it

Workflow trigger and deployment direction:

- HTTP, schedule, and event-notification triggers created on the current kernel
  remain attached to the editable source workflow and its source session
- accepting a trigger invocation MUST enqueue it through the workflow endpoint's
  normal queue path; it MUST NOT create a hidden session, cloned agents, or a
  separate queue namespace
- workflows in one session run independently. Prompts and handoffs are scheduled
  per agent, so unrelated agents may execute concurrently while work targeting a
  busy shared agent queues durably with its workflow, run, node, edge, and
  occurrence identity preserved
- an endpoint may maintain a bounded pool of runtime instances. It reuses an idle
  instance before cloning another, and every clone preserves the source agents'
  execution configuration and extension grants without copying active runs,
  transcripts, or credentials
- a local HTTP gateway is an ingress process for a source workflow trigger. It
  resolves the current publication definition from the kernel for each request
  and invokes the existing source session; starting the gateway does not export
  or materialize a workflow package
- multiple triggers MAY feed one workflow and therefore share its agents and
  configured queue namespace
- exporting or deploying a workflow is the boundary that captures an immutable
  package. A publication package contains `publication.json`,
  `workflow.snapshot.json`, `requirements.json`, optional generated app assets,
  and packaged scripts
- a packaged/self-hosted or Chariox-hosted deployment materializes its own
  kernel-owned session in the destination kernel. That deployed session is
  independent from the source session because it is a separate execution
  environment, not because a trigger was created
- protocol 282 adds optional `runtime_key` to `MaterializeWorkflowPublication`.
  A destination-owned key binds one immutable publication/snapshot to one
  runtime session and agent map. Repeating it, including after kernel restart,
  returns that runtime without reinstalling its initial queues or schedules.
  A conflicting snapshot, disabled publication, ended session, or changed agent
  ownership fails closed. Omitting the key still creates an independent runtime.
  The gateway appends `:replica-N` to `CHARIOX_PUBLICATION_RUNTIME_KEY` for each
  configured replica. Keys do not authorize access or transfer credentials.
- materialization acknowledges only after atomically persisting the initial
  session and agents. Subsequent queues, schedules, and runs use the ordinary
  kernel durable-state path. Recovery requires the same kernel identity, durable
  state and workspace mapping; a key alone is not a persistence mechanism.
- `CHARIOX_PUBLICATION_CONTROL_STATE_DIR` separates a publication kernel's
  retained state from its disposable private configuration. It selects the
  durable database, workflow definitions/code/artifacts, session and operational
  history, and monotonic event counters. Provider-account registry/home paths,
  managed-context transfer stores, relay credentials, and runtime capability
  files remain outside that root and are reconstructed from current authorized
  bindings. This is process configuration, not an additional protocol field.
  Ordinary kernels retain their existing storage layout when it is unset.
  The publication image accepts only `/var/lib/chariox/publication-control`,
  owned by the kernel identity with mode 0700, and requires explicit stable
  kernel, machine, materialization-key and workspace identities. Neither app
  actions nor the HTTP gateway can access this directory. The runner must
  mount and lifecycle-manage the matching deployment-owned volume; this
  environment setting does not create a persistent volume by itself.
- a kernel holds an exclusive process-lifetime lease on its durable store.
  Deployment replacement must stop the previous state owner before starting its
  successor. The lease is released only after the last owned store reference
  and durable writer are gone; database observers do not become schedulers.
- protocol 283 adds `ActivateWorkflowPublicationRuntime` with `publication_id`
  and the complete distinct `runtime_keys` set. A kernel using retained publication
  control storage starts with autonomous work held. Restoring state or attaching
  a client does not activate it. The gateway validates this boot's
  provider/credential/extension bindings, prepares every replica, installs event bindings and
  attaches, then requests activation. The kernel requires every enabled retained
  runtime to match a successful materialization by its owner in this process.
  `WorkflowPublicationRuntimeActivated` acknowledges that exact set. Invalid or
  incomplete preparation leaves schedules and restart recovery held without
  advancing occurrence state. Activation is process-local, never restored, and
  replaces the speculative startup grace period only in these prepared kernels.
  Ordinary kernels retain automatic startup recovery. Stopping the listener
  cancels pending recovery even if publication activation never occurs.
- protocol 284 adds `ImportNativeProviderAccountProfile`. The authority owner
  can explicitly register the kernel host's provider-native scope without a
  client-supplied path, changing an existing profile, or copying credentials.
- serving either a live source trigger or a deployed package MUST validate
  provider/model bindings, extension requirements, and credential requirements
  before it accepts traffic
- if a packaged provider/model is unavailable, a deployment binding may
  substitute another available provider/model without mutating the package
- Cloud publication deployment is a control-plane record plus a runtime backend.
  It is not a new workflow authority and does not replace the kernel-owned
  publication runtime session.
- v1 Cloud deployment supports two backend modes:
  - `local_runtime`: a public publication ingress routes to a user's local
    `chariox serve` process over an outbound connector
  - `hosted_container`: a public publication ingress routes to one Docker
    container per deployment on the publication runner
- OpenShip-managed Chariox Cloud APIs own deployment records and control commands
  only. Runtime publication traffic should terminate at the dedicated
  publication ingress and route from there to the active backend.

Workflow run history queries:

- `ListWorkflowRuns` is a bounded keyset-paginated query. Clients MAY provide
  `limit` and the opaque `cursor` returned by an earlier page.
- `WorkflowRunsListed` returns the selected `workflow_runs` plus an optional
  `next_cursor`. The absence of `next_cursor` means the history is exhausted.
- the kernel merges bounded hot, durable-history, and in-progress legacy
  migration pages. It MUST NOT replay or scan the complete lifetime run history
  to serve one request.
- legacy terminal runs remain readable until their normalized durable migration
  transaction commits; a failed or interrupted migration MUST NOT hide them.

Publication deployment record:

- `deployment_id`
- `account_id`
- `mode` (`local_runtime` | `hosted_container`)
- `slug`
- `public_base_url`
- `status`
- `publication_id`
- `publication_alias`
- `workflow_id`
- `endpoint_id`
- `hook_id`
- `transport`
- `package_digest`
- `runner_id`
- `backend_target`
- `runtime_session_id`
- `credential_profile` or credential state
- `last_health_at`
- `last_error`

Deployment records are operational metadata. They must not contain provider
auth secrets, Chariox Cloud user session credentials, workflow prompt payloads,
artifacts, outputs, or traces.

Public deployment URL contract:

- HTTP triggers are rooted at `public_base_url`
- `GET /` opens the human/browser-compatible viewer or form
- `GET /<prompt>` invokes the workflow with an address-bar prompt path
- `POST /invoke` invokes the workflow from a form or API request
- `GET /.well-known/chariox/publication/status` returns publication status

Workflow trigger V1 exposes HTTP GET/POST only. SSE remains an internal HTTP
progress mechanism and is not a selectable trigger type. Agent tool servers and
kernel/relay transport channels are independent of the workflow trigger model.

The external contract is the same for `local_runtime` and `hosted_container`.
The caller should not infer execution location from the URL.

Publication invocation envelope:

- `publication_id`
- `hook_id`
- `invocation_id`
- `transport`
- `endpoint_id`
- `queue_ref`
- `input`
- `artifacts`
- `mode`

The invocation envelope is created after hook transport parsing. It should be a
kernel-native structured value, not only a JSON string submitted through the
ordinary prompt compatibility path.

Publication event direction:

- every accepted publication invocation should have a stable `invocation_id`
- events should cover at least `queued`, `started`, `partial`, `final`, and
  `error`
- events MAY also include `trace` when the publication explicitly exposes
  workflow traces for the node and trace level that produced the record
- trace fanout is governed by a per-node publication policy, not by transport
  defaults; if no policy is present, trace events are not exposed
- trace levels are `output_summary`, `assistant_messages`, `thinking`, and
  `tool_use`
- `thinking` trace events are sourced from provider reasoning chunks persisted
  on the active `WorkflowNodeRun.thinking_traces` list while the workflow node
  prompt is running
- each `trace` event must include `invocation_id`, `workflow_run_id`,
  `node_id`, `node_label`, `agent_id`, `agent_alias`, `level`, `sequence`,
  `timestamp_ms`, and a structured `payload`
- trace filtering is part of the publication runtime contract: clients and
  publication gateways must not infer or expose hidden workflow internals
  beyond the policy
- HTTP triggers share one viewer HTML app. The viewer renders output/status on
  the left and exposed traces on the right.
- HTTP can invoke from an address-bar GET path or from the shared viewer form;
  result pages receive publication progress through the internal SSE stream.

Publication trace exposure policy:

```json
{
  "trace_exposure": {
    "nodes": {
      "node-a": ["output_summary", "assistant_messages", "thinking"],
      "node-b": ["output_summary", "tool_use"]
    }
  }
}
```

Trace exposure policy is evaluated per workflow node. Nodes without an explicit
entry expose no traces. Unknown node ids or trace levels fail publication or
serve-time validation before a server accepts traffic. Trace policy is fixed by
the publication artifact; changing exposure requires republishing or creating a
new publication.

Human HTTP renderable output:

- a final workflow output whose message parses as `{ "kind": "html", "html":
  "..." }` is renderable HTML for `human_http`
- the split viewer must render that HTML in a sandboxed `iframe srcdoc` in the
  left pane, replacing the textual output/status region
- generated HTML must not be injected directly into the publication viewer DOM
- the right trace pane remains visible and continues to show exposed traces
  after the generated HTML is rendered
- Agent Apps generalize this renderable-output model. A future generalized
  response output can represent serving an app route, returning JSON, redirecting,
  applying overlays, invoking app actions, or emitting persistent patches while
  still remaining a workflow output interpreted by the publication server. See
  `docs/AGENT_APPS_CONCEPT.md`.

Remote terminal and Cloud invocation:

- remote Chariox terminals invoke an HTTP trigger through its configured ingress,
  not by bypassing that trigger and directly calling the workflow endpoint
- when a local-only HTTP trigger is invoked remotely, the kernel/relay may tunnel
  the HTTP request and response between the remote terminal and the local server
- the relay remains transport-only and must not inspect workflow prompts,
  artifacts, outputs, or published transport payloads
- Cloud publication ingress forwards HTTP and its internal SSE progress stream
  to the active backend target and must preserve streaming semantics.
- Chariox Cloud should not proxy runtime publication streams. It may create,
  list, start, stop, and observe deployment metadata, and the web terminal may
  embed `public_base_url` in the central panel.
- If the active backend is unavailable, HTTP returns an unavailable page or a
  structured invocation error as appropriate to the request.
- Hosted containers receive scoped deployment/runtime identity only. They must
  not receive a general Chariox Cloud user account token.
- Publication images and packages must not include provider credentials. Real
  provider hosted-container validation may use a staging credential profile
  mounted by the runner; arbitrary-user provider login and credential onboarding
  are a separate product phase.

Workflow output direction:

- a workflow run may emit zero or more outputs
- outputs are a run-level concept first; strict graph-level exit points are deferred
- entry and output may be handled by the same node when the workflow design requires it

Workflow/agent binding direction:

- creating a new agent MUST NOT implicitly add it to existing workflows
- deleting an agent MUST NOT implicitly remove workflow nodes or edges
- workflows should preserve nodes whose agents are missing and mark them unavailable until repaired

Queue and turn direction:

- each workflow agent should have an inbound queue
- turn start should use a kernel-owned `consume_input_messages` tool
- output validation should use a kernel-owned `validate_output_messages` tool
- workflow turn delivery acknowledgment should use a runtime-owned `ack_workflow_turn` operation
- a running turn should not re-open its input set mid-turn; newly arrived messages remain queued for a later turn

## 5.0 Capability, Session, Workflow, Security, and Versioning Details

Detailed capability API baseline, Workspace Live Sync coordination, provider control operations, session/attachment semantics, workflow contracts, security semantics, compatibility rules, versioning strategy, and cross-platform terminal conformance now live in [PROTOCOL_CAPABILITY_SESSION_WORKFLOW.md](PROTOCOL_CAPABILITY_SESSION_WORKFLOW.md). Keep this main protocol document focused on scope, lanes, native provider behavior, envelope shape, current transport baseline, and command/workflow message direction.
