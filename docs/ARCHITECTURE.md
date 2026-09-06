# Chariox v1 Architecture

## Status

Draft architecture aligned with `docs/spec-v1.md`.

## 1. Purpose

This document translates the v1 specification into an implementation-oriented architecture view:

- component boundaries
- runtime ownership
- trust and security boundaries
- state ownership and storage boundaries
- critical runtime flows

## 1.1 Terminology

Target architectural terms:

- `Chariox Kernel`
  - the authoritative orchestration/runtime kernel
- `chariox-kernel`
  - the process that hosts the kernel on one machine/user context
- `workspace`
  - the project files and source-control context bonded to a Room or agent
- `Room`
  - the durable collaboration runtime that owns users, agents, workflows, history, and the default shared Environment
- `Environment`
  - the Room-owned browser and graphical computer shared by users and agents
- `workflow`
  - a directed execution graph inside one Room

Current implementation note:

- the current Rust code still uses `daemon` and `session` heavily
- the product term `Room` maps to the current code's `session` and `session_id`
- `docs/spec-v1.md` and older planning documents retain the legacy use of `workspace` for this runtime domain
- older sections of this document use `workspace` for that runtime domain; read those legacy uses as `Room`
- `workspace` now means project files and source-control state, matching the product distinction between a Room and its bonded Workspace
- the canonical glossary is [CONTEXT.md](../CONTEXT.md)

## 2. System Topology

Chariox v1 is composed of five runtime components:

- Client
- Machine
- Chariox Kernel
- Relay Server
- Directory Service

High-level topology:

`Client <-> Chariox Kernel <-> Agent Endpoint`

Remote topology:

`Remote Client <-> Relay Server <-> Chariox Kernel <-> Agent Endpoint`

Discovery topology:

`Client | Kernel -> Directory Service`

Current implementation mapping:

- the kernel currently runs inside the Rust runtime in [apps/kernel](/Users/miguel/chariox/apps/kernel)
- the primary client is the TypeScript CLI in [apps/cli](/Users/miguel/chariox/apps/cli)
- the current OpenCode adapter talks to a local OpenCode HTTP + SSE endpoint
- the primary daemon-client transport is now a kernel WebSocket with pushed events; the local Unix-socket IPC surface remains for harnesses, compatibility, and local management paths
- relay, directory, and unified node transport are later implementation work, not current code
- the relay is planned as an independent app, separate from both daemon and CLI

## 2.1 Architectural Rules

- CLI is one client implementation, not the owner of business logic.
- The kernel owns Room state, routing, workflow state, and coordination.
- Transport and discovery are separate concerns.
- Relay forwards traffic; it does not own rendezvous/discovery.
- Directory provides identity/discovery/reachability metadata; it does not own Room state.
- Managed identity/discovery service, if introduced later, should remain outside this repository and consume the same relay/directory boundaries rather than becoming a dependency of core runtime code.
- New features should land in kernel/protocol layers first, not UI-specific code.
- Runtime and control-plane persistence store instants in UTC, and transport serializes them as
  UTC RFC 3339 or Unix epoch values. Presentation converts only at the client boundary.
- Recurring wall-clock schedules are the deliberate exception: they retain an IANA timezone so
  daylight-saving transitions are resolved by timezone rules rather than a hard-coded offset;
  each computed occurrence is still stored and transported as a UTC instant.

## 2.2 Connectivity Model

Chariox should model a kernel-hosted runtime domain that may contain both local and remote members.

Members of the same Room may include:

- local terminals
- remote terminals attached through relay
- local agent endpoints
- remote agent endpoints attached through relay

Normative rules:

- locality is a transport property, not an authority property
- local and remote members attached to the same kernel belong to the same runtime domain
- the kernel remains the authority for Rooms, attachments, prompt queues, provider runs, workflow routing, and coordination regardless of member locality
- relay must not become the Room or workflow authority

## 2.3 Workflow Rule

The purpose of a multi-agent Room is to host workflows.

Normative rules:

- a Room may contain multiple workflow definitions
- a workflow is a directed graph inside one Room
- graph nodes are top-level Chariox-managed agents
- graph edges define allowed message flow
- each workflow definition may expose multiple endpoints
- each workflow endpoint targets exactly one entry node in that workflow
- disconnected subgraphs are allowed inside one workflow; a subgraph is only reachable if some endpoint targets an entry node within it
- the kernel owns workflow runs, routing, and turn activation

## 3. Component Responsibilities

### 3.1 Client

Responsibilities:

- render Room state, transcript/output, and focused-agent state
- capture terminal input or structured actions and route them to the kernel
- render slash-command completion, help, warnings, and command results
- upload or reference artifacts
- remain attached or return to a no-Room state without becoming a runtime authority

Current implementation note:

- the primary local client is the TypeScript OpenTUI app in [apps/cli](/Users/miguel/chariox/apps/cli)
- `chariox-cli` is currently a Rust launcher for that client
- the primary local transport is the kernel WebSocket event stream; local IPC remains as a lower-level compatibility and harness surface
- M4.5 session, agent, and workflow commands are routed through the kernel command router into bounded runtime lanes; local IPC, relay-proxied workflow requests, and actor workers delegate to explicit runtime request handlers while the legacy `DaemonApp` remains only as the current mutation mirror. Session runtime execution now reaches session lifecycle/config/alias mutations through `KernelSessionService` directly instead of broad app-level request helpers.

### 3.2 Machine

A machine hosts one kernel process per OS user context.

Responsibilities:

- provide execution environment for kernel, providers, artifacts, and worktrees
- host Room runtime files and bonded Workspace files
- later participate in registration and reachability metadata

Remote-machine note:

- a machine is a user-facing placement target
- users choose a machine when spawning a remote agent; they do not need to choose a kernel in the common path
- once spawned, a remote agent is bound to one selected worker kernel for its lifetime
- provider availability is advertised from the worker kernel back through relay metadata
- provider login remains local to the worker kernel; the home kernel consumes provider availability but does not proxy provider auth flows

Managed-machine note:

- a Chariox-managed environment is a Cloud-owned lifecycle record linked to a
  runtime Machine only after its independent kernel registers
- the managed kernel is not a worker lease and does not import the source Machine
  identity, sessions, agents, provider runs, prompt history, or grants
- stop and start preserve the independent kernel identity and runtime-owned state
- Cloud owns desired lifecycle, durable operations, provider resource mappings,
  bootstrap grants, coarse activity, and auto-stop policy; the private
  infrastructure manager owns provider credentials and provider reconciliation
- the relay remains transport-only, and direct source-to-target context transfer is
  encrypted between kernels
- the TUI and web Waiting Room project managed environments through the existing
  Machine field and suppress the linked runtime Machine as a duplicate row

See [M28_ENVIRONMENT_CONTEXT_MATERIALIZATION_PLAN.md](M28_ENVIRONMENT_CONTEXT_MATERIALIZATION_PLAN.md)
for the implemented context and launch contract.

### 3.3 Chariox Kernel

The Chariox Kernel is the runtime authority for live Room state on one machine/user context.

Responsibilities:

- Room lifecycle and attachment lifecycle
- Room routing between all attached local and remote members
- workflow scheduling and workflow-run state ownership
- inter-agent message routing and structured handoff processing
- worktree allocation and isolation enforcement
- PTY lifecycle for provider runs
- provider switching and parked-run management
- capability execution
- extension/MCP/runtime ownership
- logging/root correlation metadata
- workspace coordination to reduce edit/integration conflicts across top-level agents in the same workspace

Remote-agent note:

- a home kernel remains the only session authority even when some agents execute on remote machines
- worker kernels host leased execution for those remote agents but do not become session authorities
- from the user point of view, a remote agent should behave the same way as a local agent after placement, with machine placement shown as metadata rather than as a separate runtime mode
- home-owned active extensions and vault credentials for remote agents preserve that authority split: the home kernel owns grant/revoke and credential policy, reconstructs the current tool definition before every forwarded extension invocation, executes scripts/connectors/MCP proxy calls and vault operations on the home machine, and records durable audit events where applicable. The worker kernel only projects approved manifests to the provider runtime, forwards calls with invocation metadata, requests scoped credential injection material for worker-local targets, and sends best-effort cancellation for in-flight calls when a leased prompt is cancelled. Computer credential calls forward only the credential handle to home; home admits the redacted Room Action and sends the one-operation secret input through the existing encrypted Room controller route. An unattended remote Claude launch is the narrow provider-launch exception: home resolves the selected profile's vaulted setup token only when the worker needs a run, sends it in the encrypted lease request, and the worker moves it directly into a zeroizing `CLAUDE_CODE_OAUTH_TOKEN` launch environment. Neither kernel persists it, and no `.credentials.json` replica is created.

Runtime authority invariants:

- clients render state and submit typed requests; they must not synthesize session, agent, provider-run, permission, history, or health state
- the home kernel owns sessions, prompts, attachments, transcript history, runtime interactions, Workspace Live Sync policy, extension grants, and remote-agent leases
- worker kernels own only the execution they host: provider process lifecycle, worker-local tool execution, slice container/runtime state, and leased-agent transport to home
- relay and Cloud remain bootstrap/transport/control-plane surfaces; neither may inspect or mutate runtime prompts, provider payloads, workspace files, extension credentials, or session history
- provider-native TUIs, web terminals, local TUIs, remote TUIs, and slice-backed agents must enter through the same kernel-owned prompt, permission, provider-run, and projection primitives
- every projected remote state with authority implications, including lease health, active worker provider run, remote extension manifest sync, slice auth, and Workspace Live Sync mode, must have a kernel-owned health/audit projection and a validation-platform runtime signal

### 3.3.1 Internal Kernel Subsystems

The kernel should be understood as containing several internal subsystems even when they are not yet split into separate processes.

Required subsystem roles:

- `WorkspaceRouter`
  - authoritative routing/fanout of prompt lifecycle, notices, provider output, and workflow handoffs
- `TransportGateway`
  - accepts local and remote terminal connections and normalizes them into kernel-owned attachments
  - current agent integrations remain adapter-owned and do not yet share this transport
- `AgentEndpointManager`
  - connects to managed or external agent endpoints and normalizes their native/provider-specific protocols into kernel events
- `WorkspaceCoordinator`
  - manages worktree/branch allocation, current coarse workspace claims, and later integration/merge safety checks inside one workspace

Current implementation note:

- the current codebase has pieces of these responsibilities inside the daemon crate, but not yet as a unified bidirectional node transport
- the current OpenCode adapter is already a provider-endpoint integration example
- the current local CLI transport is now a long-lived WebSocket subscription with pushed kernel events
- a generic WebSocket transport for agent endpoints is still intentionally deferred
- the current `WorkspaceCoordinator` remains a coarse safety/scheduling boundary for worktree-level claims, while Workspace Live Sync managed mode owns fine-grained text coordination and opaque whole-file coordination for Chariox-managed provider sessions

### 3.3.1.1 Kernel Runtime and Workflow Details

Detailed target-kernel implementation model, workflow model, workflow messaging, endpoint/run-output behavior, publication runtime behavior, agent binding rules, and observability baseline now live in [ARCHITECTURE_KERNEL_RUNTIME_WORKFLOW.md](ARCHITECTURE_KERNEL_RUNTIME_WORKFLOW.md). Keep this main architecture file focused on top-level component boundaries and authority rules.

### 3.4 Relay Server

The relay server is a lightweight transport-forwarding layer.

Responsibilities:

- websocket relay
- daemon connection registry and liveness
- client-to-daemon request/response/event forwarding
- minimal routing metadata needed to target a connected daemon

Current architectural interpretation:

- the relay should forward transport, not own discovery/rendezvous
- local and remote connections should ideally share one daemon-owned application protocol even if they arrive through different physical paths
- the relay must not become the Room or workflow authority
- the relay should be implemented as an independent Rust app
- daemon connections should be outbound from daemon to relay so the model works cleanly through NAT/firewall boundaries
- one daemon should use one active relay connection at a time in v1, even if multiple relay endpoints can be configured
- self-hosted relay mode must work without any external managed identity/discovery service
- all user-generated payloads that cross relay boundaries must be session-scoped end-to-end encrypted, including prompts, workflow payloads, and transferred artifacts
- this encryption requirement applies equally to self-hosted relay deployments; self-hosting does not relax the transport privacy model
- the same CLI should support local direct daemon operation and relay-mediated remote operation without becoming two apps
- the CLI should always open the waiting room first; local sessions remain available even when relay is not configured or disconnected
- relay connection is configured from slash commands or the waiting-room relay section, then auto-connects in the background
- `/relay use <ws-url>` may read the token from `CHARIOX_RELAY_TOKEN`; passing the token as a visible slash-command argument remains supported for self-hosted/manual testing but should not be the preferred documented path for shared terminals or screenshots
- worker processes may receive `CHARIOX_CLOUD_RELAY_CONFIG_JSON` only when the launcher intentionally delegates Cloud relay refresh authority to that worker; the normal env relay URL/token path remains scoped runtime relay configuration and must not imply access to home-owned Cloud credentials
- the waiting room groups relay status and relay actions together under `Relay`; it also groups machines and pending machine counts together under `Machines`
- once relay connects, machine/provider availability updates automatically; if the user is already in a session, remote capability can become available silently with at most a small informational footer
- waiting-room remote machine/kernel discovery must be projection-backed, not request-backed: the kernel refreshes relay presence in the background and publishes a daemon-owned remote-inventory projection; CLI/web waiting-room reads consume that projection and must not synchronously open new relay metadata sockets on the user-facing read path
- the home kernel maintains local machine trust state: live unknown machines are pending, `/machine approve` makes them spawn targets, `/machine rename` stores a user alias, and `/machine forget` hides them from normal machine/provider availability
- relay kernel display names are relay-scoped live labels, not durable user aliases: each registered kernel reports its OS name and kernel start time, and the relay exposes addressable names such as `machine 1 (macOS)` for discovery and routing
- relay machine lists remain grouped by stable machine identity; when several kernels are online from the same machine, `/machine kernels <machine-ref>` shows each addressable relay kernel alias separately
- user-facing stable names still come from local home-kernel rename/approval state; relay aliases are plain live metadata and do not become relay-owned user preferences
- relay-visible machine metadata remains plain routing/liveness metadata; trust decisions and aliases are local home-kernel state, not relay authority
- relay health endpoints expose transport-level backpressure counters such as target queue saturation and slow subscription closures; these metrics are for operations and scale drills only and do not require inspecting encrypted runtime payloads


### Docker Remote-Machine Lab

The Docker lab models containers as ordinary Chariox machines. The base image includes Chariox and provider harnesses but no baked credentials. For home-managed slices and trusted home-workers, the home kernel materializes only the selected named provider profile through the existing encrypted worker channel. Claude materialization contains non-secret profile state only; its setup token is delivered transiently at launch. Ordinary independently managed machines may still authenticate locally, but that is not a second account authority for a home-owned agent. See [PROVIDER_ACCOUNTS.md](PROVIDER_ACCOUNTS.md).

Required container properties:

- outbound internet for provider APIs, package installation, hosted relay access, and auth flows
- persistent runtime state so machine identity and authorized profile replicas survive restart
- separate machine identity per container, derived from persisted config rather than baked into the image
- optional URL-printing browser/`xdg-open` shims for provider login flows that request a browser
- documented provider compatibility for the small launch-provider set, including login method, callback-port behavior, and tested CLI versions

Normal provider runtime ports do not need host mapping when the provider process and worker kernel run in the same container. Login callback ports are provider-specific and must be tested/documented per provider.

### 3.5 Directory Service

The directory service is a later, intentionally simple control-plane component for:

- identity registration
- discovery
- reachability metadata
- rendezvous/bootstrap information

It is distinct from relay:

- directory answers where/how a kernel or published endpoint can be reached
- relay forwards traffic after that decision is made
- a later managed service may provide identity/discovery on top of these boundaries, but that service remains outside this repository

### 3.6 Agent Endpoints

An agent endpoint is the kernel-facing runtime interface implemented by a provider integration or Chariox-native agent runtime.

Required endpoint modes:

- `managed`
  - kernel launches the endpoint/runtime itself
- `external`
  - the endpoint already exists and the kernel discovers/configures/connects to it

Normative rules:

- the kernel should depend on an endpoint contract, not only on child-process ownership
- existing providers like OpenCode may keep native transport adapters
- Chariox-native or third-party agent runtimes should eventually target a canonical daemon-facing agent protocol directly
- transport unification should happen at the kernel protocol/event model level, not by forcing every provider to mimic the same wire transport internally

### 3.7 Workspace Coordination

If Chariox is to orchestrate multiple top-level agents without relying on a human to manually clean up merge conflicts, workspace coordination must be kernel-owned.

Baseline responsibilities:

- allocate worktrees and branches per top-level agent
- record edit intent or claim information at least at workspace/file granularity
- prevent or warn about obviously conflicting edits
- run integration and mergeability checks before changes are combined

Near-term practical rule:

- keep current worktree-level coordination as the near-term guardrail while the kernel refactor completes
- use Workspace Live Sync managed mode for Chariox-managed provider-session writes; keep port claims, integration/merge checks, and post-v1 artifact-specific region models as future coordination work
- the kernel should own integration policy rather than delegating all conflict discovery to late Git merges or human PR review

Scope rule:

- coordination is workspace-scoped, not machine-wide or repo-wide across all workspaces
- different workspaces may still collide at integration time in the same way independent PRs can conflict

Non-responsibility:

- should not require plaintext access to user-generated session content

## 4. Runtime Ownership and State Authority

### 4.1 Authority Model

- **Daemon**: source of truth for active runtime state
- **Server**: source of truth for shared operational metadata
- **Provider process**: source of truth for provider-native behavior

### 4.2 Session Ownership

A session is bound to:

- one workspace
- one primary worktree in single-agent mode
- one active provider run in single-agent mode
- an eligible set of machines with one active host at a time

A session may include:

- many top-level Chariox-managed agents
- many client attachments
- parked provider runs
- agent-scoped provider runs when multi-agent session mode or workflow mode is active
- agent-scoped history/runtime metadata and worktree assignments when multi-agent session mode or workflow mode is active
- prompt queue state and canonical session config state
- a workflow definition and zero or one active workflow run
- node-scoped provider runs when workflow mode is active
- worktree assignments for isolated workflow branches
- schedules
- artifacts
- extension bindings resolved for top-level provider runs

### 4.2.1 Shared Attachment and Queue Ownership

In single-agent mode, attachments are shared session participants rather than exclusive control roles.

Required daemon-owned responsibilities:

- serializing prompt execution per session
- maintaining explicit scheduler state boundaries (`idle`, `runnable`, `running`, `waiting`) for queued work
- maintaining canonical queued-prompt state
- exposing canonical session state and runtime notices to attachments
- applying accepted config changes to canonical session config state
- rejecting unsafe config changes while a prompt is running
- notifying all other attachments when a prompt is queued
- propagating canonical config updates to all attachments after acceptance

Current M1 runtime note:

- the daemon now keeps explicit primary worktree assignment metadata for each session even in single-agent mode so later branch/worktree isolation can extend the same runtime shape

The daemon MUST treat prompt scheduling and config state as structured runtime state, not terminal-local behavior.

### 4.2.2 Multi-Agent Session Ownership

Manual multi-agent session behavior is still daemon-owned runtime behavior, not just client chrome.

Required daemon-owned responsibilities:

- maintain the canonical top-level agent list for each session
- maintain focused-agent state for direct user interaction
- route prompt submission to the selected agent's runtime context
- keep agent-scoped provider-run, history, and worktree-assignment metadata authoritative in daemon state
- expose enough agent-scoped state for pane-based clients to render one visible sub-area per active agent

Current implementation note:

- the local runtime already has session agent records, focused-agent metadata, and Chariox-owned `/agent ...` management commands
- the current CLI footer/chrome reflects that state, but transcript routing and provider execution are still effectively single-agent
- the next implementation step is to make focused-agent changes affect both prompt routing and visible per-agent panes/history

## Workflow Console

Kernel components now include a workflow-scoped shared console service.

Responsibilities:

- one append-only console per workflow definition
- shared human-facing output stream separate from provider traces
- readable/writable/clearable by workflow nodes through Chariox MCP tools
- rendered by the CLI in the workflow right-side panel via `/workflow terminal`

Ownership split:

- transport exposes MCP tools and authenticates/scopes calls
- scheduler/runtime owns workflow-console state and semantics
- CLI renders the live console stream without rewriting content

Boundary:

- the workflow console is not mailbox state
- the workflow console is not handoff state
- the workflow console is not audit state
- it is a shared presentation/output surface for one workflow

### 4.2.3 Persistent Session and Deletion Ownership

Chariox session lifetime should be explicit and daemon-owned.

Required rules:

- the daemon MUST treat detach and delete as distinct operations
- detaching the last client MUST NOT delete the session by default
- idle sessions SHOULD remain discoverable and reattachable until explicit deletion
- deleting a session MUST:
  - terminate or clear active provider/runtime state
  - remove the session from the daemon registry
  - invalidate further attach attempts
  - notify attached clients that the session no longer exists
- attached clients SHOULD transition to an unattached "no session" state when their current session is deleted, rather than being forced to terminate the whole client process

Planned client behavior:

- `/exit` detaches from the current session
- explicit session deletion is handled through a dedicated session-management command or external control command
- when the currently attached session is deleted, the client clears transcript/session chrome, renders a Chariox ASCII-art landing state, and returns to a reusable unattached shell state

Current local baseline:

- the TypeScript CLI now supports an unattached no-session state after explicit session deletion
- temporary session-management commands exist ahead of the general slash-command system:
  - `/session create [alias]`
  - `/session attach <ref>`
  - `/session delete [ref]`

### 4.2.4 Shared Room environment authority

Each Room owns at most one default shared Environment. The Environment belongs to the Room, not to an agent, provider run, client attachment, slice, display streamer, or browser process. Attaching another user, agent, Web client, TUI, or provider-native TUI must not create another Environment.

The kernel owns:

- Environment identity, lifecycle, runtime generation, and health
- the shared browser profile and tab registry
- the canonical viewport and resize policy
- actor presence and input ownership
- Action admission, ordering, cancellation, outcome, and history
- save, restore, reset, reconnect, and recovery reconciliation
- Browser mode and Computer mode projections

The worker that hosts a slice owns its local browser, Browser Controller, streamer, desktop, and input processes. This is execution ownership only. The home kernel remains the Room and Environment authority. Agent execution uses the existing leased-agent path; physical browser lifecycle and observations use the authenticated Room controller route over the same encrypted peer transport and do not require an agent lease.

Cloud may authenticate the user, provision a machine, issue scoped relay credentials, and render the Environment projection. It must not proxy runtime display or input traffic, assign tab identity, order Actions, or store browser and desktop history. The relay transports encrypted runtime and display packets without inspecting them.

#### Identity and lifecycle

`environment_id` identifies the logical Environment for its lifetime. Stop, start, save, restore, controller restart, browser restart, and streamer restart preserve that identity. Reset preserves the Room binding but increments `runtime_generation` and invalidates runtime-only references. Replacing or deleting the Environment ends the identity.

`runtime_generation` changes whenever the kernel can no longer prove that runtime handles still name the same browser, desktop, or controller state. An element reference therefore includes its Environment, Tab, runtime generation, and document revision. A stale generation or revision must fail with an actionable rediscovery error.

The Environment lifecycle is separate from slice lifecycle. The initial states are:

| State | Meaning |
| --- | --- |
| `stopped` | No usable Environment runtime is present. |
| `starting` | The worker is creating or reconciling browser, controller, streamer, and desktop processes. |
| `ready` | Browser and Computer actions may be admitted according to permissions and input ownership. |
| `degraded` | Part of the Environment is unavailable, but unaffected work may continue. |
| `saving` | New mutations are closed while the kernel captures a consistent generation. |
| `restoring` | The kernel is recreating runtime state from a saved generation. |
| `stopping` | New actions are closed and managed processes are shutting down. |
| `failed` | Automatic recovery ended without a usable runtime and requires an explicit retry, reset, or repair. |

Lifecycle transitions must have bounded deadlines. Failure cannot leave the Environment reported as ready or leave an input target owned by a dead Actor.

#### One browser and stable tabs

The Browser Controller owns browser-process integration below the kernel. The kernel owns the Room-visible tab registry and assigns stable `tab_id` values. Controller or browser target identifiers remain implementation details.

On Linux slices, the Browser Controller is a long-lived child process owned directly by the worker kernel. The kernel is its only caller and communicates through a private, request-correlated stdin/stdout channel with bounded health and shutdown deadlines. The controller exposes no agent-addressable socket or command surface. Each physical browser/profile has exactly one Room owner. Repeated acquisition by that Room reuses its controller; a different Room must use a different physical Environment, never another lease on the same browser. Releasing the lease stops the controller but retains the owner binding, because neither the browser nor its profile is erased by controller shutdown. Failed startup also retains that binding. Kernel shutdown terminates the complete controller process group. Controller readiness updates only the Browser Controller component health. It does not by itself make the Environment ready before the browser, desktop, and streamer have also reconciled.

The opt-in local controller store currently enforces this binding for its own lifetime. Durable Room-to-slice/worker placement and recovery after kernel replacement remain required before multi-Room product enablement. Creating another store pointed at the same CDP endpoint is not isolation. The home kernel must bind distinct Rooms to distinct physical Environments and restore those bindings before admitting browser or viewer requests.

The home kernel now records explicit physical placement in `SliceRecord.environment_session_id` through protocol v282. The slice store serializes competing claims and commits the reservation before publishing it. Reverse Room lookup is derived from those records, not a second mutable index. Bindings survive kernel replay and are not cleared by controller stop. Public single/batch agent spawn, session creation, and agent move requests preflight known slice reservations before worktree preparation, provider-run termination, or worker contact. They hold the existing slice operation guards through admission and attachment; batch requests acquire each canonical slice once. Creating a new Room cannot enter an already-reserved slice. Ordinary remote kernels and unassigned legacy slices retain their existing admission behavior.

The Room controller route validates the provisioned home key/kernel/Room/slice tuple on the worker. Lifecycle, structured observations, mutations, integrations, events, compatibility calls, and worker-agent browser tools consume the persisted placement, while the home assigns stable tabs and opaque element references and owns action admission and history. If the worker discovers an implicit controller restart before an operation, it returns the new process generation. The home fails the admitted mutation without replay, invalidates old element references, reconciles stable Tabs, and restores component health before accepting another mutation.

Provider-facing mouse, keyboard, and clipboard-write tools use that route as well. A leased provider call reaches the home kernel first, where the home derives the agent Actor, validates the current Room Environment, admits one redacted Computer Action, and sends the physical input to the bound worker. Clipboard text is bounded, transported to the physical helper over stdin, and represented in history only by byte and character counts. Agents have no clipboard-read tool. The worker executes the input but does not create a private Room, choose an Actor, or record a second Action ledger. This is the same authority path used by browser tools, human Computer input, takeover cancellation, and Computer credential input.

Provider-facing screenshots follow the same Room placement without entering the mutation ledger. Home derives the Room and agent from the provider run, asks the bound worker to capture through the authenticated screenshot peer, and reads the opaque artifact back in bounded chunks. A leased provider forwards its request to home rather than invoking a worker-local screenshot path. Worker filesystem paths never cross the boundary; inline provider images are capped before allocation and verified against the worker artifact digest.

Provider-facing Computer status, OCR, and text lookup use the same authority route. Direct-home and leased providers both reach the home kernel, which derives the Room, agent, bound physical worker, and canonical viewport. Home sends a typed observation over the authenticated encrypted peer; only the bound worker runs the physical screen helper. A Room agent may pass an opaque screenshot artifact ID to OCR or text lookup so the worker can inspect the exact stored frame. The worker resolves that ID only after checking its Room and slice scope. Text lookup returns every non-overlapping occurrence in visual reading order using native screenshot-pixel coordinates; `match` retains the first occurrence for compatibility while `matches` and `match_count` carry the complete result. Artifact paths, private display identifiers, viewer URLs, and raw helper output never cross the worker interface. Home returns canonical Room dimensions and requires clients to obtain viewer access through their own attachment. These reads do not enter the mutating Action ledger.

Room display admission consumes the same persisted placement. The home validates the requesting attachment and key, then asks the bound worker over the authenticated peer route. The worker revalidates its provisioned home key/kernel/Room/slice tuple and registers one expiring, single-use encrypted stream with the relay. The opening grant and the active-view lease are distinct: the grant cannot be replayed, while the worker renews the private adapter only for the lifetime of the admitted WebSocket. The relay routes outer tunnel metadata and ciphertext but never becomes Room authority or sees video. Display controls are read-only. Human mouse, keyboard, and clipboard-write input enters through authenticated local protocol requests, explicit desktop takeover, and the kernel Action ledger before the same bound-worker route executes it. Clipboard reads also require the authenticated human to own desktop input and cross the same bound-worker route, but they are observations and do not create an Action. Clipboard contents are held in zeroizing, redacted values and never enter Room state, history, traces, or helper arguments. Physical input is at-most-once and has no transport replay command. The worker tracks each live input helper below that authority boundary. Cancellation kills its process group and resets held modifier keys and mouse buttons before the worker confirms cancellation, so takeover cannot transfer ownership while physical input remains active. Complete recovery, saved-state acceptance, and managed-machine acceptance remain unfinished.

Tab rules:

- a recoverable controller reconnect must not duplicate a Tab
- navigation preserves `tab_id` and increments `document_revision`
- closing a Tab retires its identity and invalidates its element references
- restore may retain `tab_id` only when the saved tab can be matched without ambiguity
- reconciliation creates a new `tab_id` when identity cannot be proved
- an old target ID, URL, title, or tab index is never sufficient authority to mutate a Tab

Browser mode exposes structured tab, accessibility, document, lifecycle, console, and network observations. Computer mode exposes the same Environment through its graphical display and desktop input. Switching modes does not create a browser, move state, or change authority.

Vault-backed Computer credential input uses that same desktop-input authority. The agent supplies only a credential handle. For a leased agent, its worker forwards that handle-only call to home instead of resolving a secret or creating a parallel Action locally. The home kernel validates the Computer-specific credential policy, requires user confirmation, resolves the secret, and admits the redacted `secret_input` Action against the authoritative Room. The secret reaches the physical worker only inside the existing encrypted Room controller command, where stdin types it into the current desktop focus without using the clipboard. Browser credential input remains DOM-bound and automatically rejects unmasked targets; Computer input relies on explicit user verification because arbitrary X11 controls do not expose a universal password-field contract.

#### Canonical viewport

One canonical viewport defines browser layout, desktop resolution, display encoding, screenshot pixels, OCR coordinates, pointer coordinates, and every viewer's aspect ratio. Clients scale presentation locally but must not resize the browser, desktop, or streamer directly.

The kernel accepts or rejects viewport requests. While a user owns desktop input, that user owns viewport changes. Without a user input owner, the Environment keeps its configured viewport unless kernel policy accepts another Actor's request. Every accepted change increments `viewport_revision` and reaches the browser, desktop, streamer, screenshots, coordinates, and viewers as one reconciled transition.

#### Actors, Actions, and input ordering

Actor presence is kernel-owned ephemeral Room state. Protocol v295 assigns each Actor a stable semantic presentation color derived from the Actor ID and projects at most one desktop-pixel pointer per Actor. An authenticated pointer update carries the runtime generation, viewport revision, and optional coordinates. The session lane supplies the Actor identity. Presence does not create an Action or grant input ownership. The kernel removes the pointer when the Actor disconnects, the canonical viewport changes, the runtime is invalidated, or the Environment stops or fails. Consecutive pointer events coalesce in replay storage so mouse motion cannot crowd lifecycle, health, ownership, or Action events out of the bounded log. The overlay belongs above the display stream in Chariox clients, never in webpage DOM.

Every Browser or Computer Action records:

- Action, Actor, Room, and Environment identity
- Tab identity when the Action targets a Tab
- input target, Action kind, and bounded deadline
- queued, started, completed, failed, or cancelled lifecycle
- pointer coordinates when applicable
- outcome and failure classification without secret payloads

Observations may run concurrently. Mutating Browser Actions serialize per Tab. Mutations on different Tabs may run concurrently. Computer mutations serialize on the desktop input target. A Computer Action that can affect the focused browser Tab reserves both the desktop and that Tab for its duration. A Browser Action that opens, closes, or focuses Tabs also reserves the desktop and every affected Tab. These rules prevent graphical input and tab lifecycle changes from racing a structured mutation on the visible page.

Reservation ordering is always desktop before Tab, then Tab IDs in lexical order when an operation needs more than one Tab. Acquisition has a deadline and cancellation releases every reservation. No caller may hold a reservation while waiting for a permission response or network reconnect.

#### Human takeover

Takeover is a kernel-owned transition, not browser focus or client hover. A user requests ownership of an input target. The kernel then:

1. closes admission for new agent mutations on that target
2. cancels or pauses the active agent Action and waits for a terminal Action state
3. records the ownership change and reason
4. grants the user input only after the prior Action cannot continue

The user retains ownership until explicit release, disconnect-policy expiry, or Environment stop. Reconnect must not silently restore ownership to an agent. Every client sees the same ownership and cancellation state.

#### History, reconnect, and recovery

The kernel keeps an append-only Action ledger with bounded observations and redacted outcomes. It records enough information to answer who acted, on which target, in what order, and whether the Action completed. It must not store vault values, authentication headers, unredacted clipboard secrets, or provider-private payloads.

Clients reconnect by event cursor. A retained cursor replays missing Environment events in order. A replay gap requires a fresh Environment snapshot plus the next event cursor. Clients discard optimistic state after a gap and never infer completion from a reappearing browser or display.

Controller, browser, streamer, worker, kernel, and relay recovery must reconcile Environment identity, generation, tabs, viewport, input ownership, in-flight Actions, and saved-state generation before reporting ready. Completed Actions are never repeated. An Action without durable completion evidence fails or resumes under an explicit idempotency rule.

For live slice browser mutations, the worker's bounded in-memory completion
receipts provide that idempotency rule across an encrypted relay response loss.
The receipt binds the Room and execution identity to a non-plaintext request
fingerprint and terminal result. The home may retry only the identical request;
worker restart or receipt eviction removes this proof and must not trigger a
blind physical replay.

#### Client obligations

Web, local TUI, remote TUI, iOS, and planned Android clients render kernel projections. A TUI may open the graphical viewer rather than embed it, but it must show the same Environment health, current Tab, viewport, Actor, ownership, Action, and recovery state. Provider-native TUIs remain clients of the normal Room path and do not gain another browser or input authority.

### 4.3 Workflow Ownership

When a session runs in multi-agent workflow mode, the daemon MUST treat the workflow as a generic directed graph execution problem.

Required runtime concepts:

- `WorkflowDefinition`
- `WorkflowNode`
- `WorkflowEdge`
- `WorkflowRun`
- `NodeRun`
- `NodeMessage`
- `WorktreeAssignment`
- `AggregationState` or equivalent barrier/fan-in state

Required rules:

- Execution policy MUST be derived from the graph the user created, not from a separate user-declared topology flag.
- Nodes with indegree `<= 1` are serial with respect to input gating by default.
- Nodes with indegree `> 1` run once per incoming message by default.
- Nodes that must combine parallel branch outputs require explicit barrier/fan-in handling.
- Barrier/fan-in handling synchronizes incoming handoffs by source node iteration so faster loop branches do not pair with slower outputs from a different iteration.
- Nodes with outdegree `> 1` are branching points and may release outputs to multiple children.
- Cycles are a separate graph property and MUST be handled independently from input/output synchronization policy.
- The runtime SHOULD support per-node execution policy rather than a workflow-wide sync/async switch.

Implementation priority note:

- graph-derived serial execution is the earlier implementation target
- graph-derived barrier/fan-in and bounded-cycle handling should follow on top of the same generic workflow engine

### 4.4 Inter-Agent Communication Ownership

Inter-agent communication MUST be daemon-orchestrated.

Required rules:

- Agents MUST NOT communicate directly.
- The daemon MUST own all routing and delivery between workflow nodes.
- Output from one node MUST be transformed into a standardized structured handoff payload before it is delivered to the next node.
- Inter-agent communication MUST NOT be modeled as raw terminal transcript forwarding.
- Workflow scheduling MUST advance from structured node completion reports, not arbitrary provider turns.

Each node completion artifact/report MUST include at least:

- `status`
- `summary`
- optional explicit `output`
- optional `artifacts` or changed files
- `stop_recommendation`

Rules:

- `summary` is human-facing and audit-oriented; it is not the downstream workflow payload by default
- downstream workflow delivery should use explicit output messages plus optional artifact refs
- transcript history remains audit state, not workflow output

### 4.5 Worktree Isolation

Parallel code-writing branches MUST NOT share the same active worktree.

Required rules:

- Worktree assignment MUST be explicit in runtime state and the data model.
- In hierarchical workflows, each active code-writing branch or subtree SHOULD receive an isolated worktree and git branch.
- The daemon MUST reject or prevent concurrent mutation of the same active worktree by parallel code-writing nodes.

## 5. Interaction Lanes

### 5.1 Terminal Lane

- Carries raw PTY output and user keystrokes.
- Must preserve provider-native semantics.
- Must not be transformed into structured command traffic by default.
- Must not be used as the source of truth for prompt queue ordering or session config state.
- For providers with stable structured local protocols, daemon-rendered output derived from provider events is acceptable and preferred over PTY-idle heuristics.

### 5.2 Capability Lane

- Carries daemon capability requests/results.
- Used for shell, file ops, screenshot, git/worktree, schedules, transfers, and other Chariox-owned slash commands.

### 5.3 Control Lane

Structured daemon-to-provider adapter control boundary.

Canonical control operations in v1:

- `attach_file`
- `request_memory_update`
- `request_compaction_summary`

`request_memory_update` and `request_compaction_summary` are daemon-owned and distinct from normal user prompt/response traffic.

`/<provider> ...` commands are resolved by Chariox first, then dispatched into adapter-owned behavior through the control lane or adapter-specific execution hooks.

Provider authentication is not part of the control lane in v1; adapters probe and report auth state, but login itself remains a provider-native local CLI flow on the host machine.

Provider-facing extension projection is also adapter-owned: the daemon resolves the authoritative extension bindings, and the adapter materializes the provider-specific runtime view.

### 5.3.1 Prompt Assembly Service

The kernel owns prompt assembly. Clients, workflow schedulers, provider adapters, native TUI launchers, and utility-call helpers must not concatenate Chariox runtime instructions directly into provider prompt text.

The prompt assembly boundary produces a `PromptEnvelope` with these conceptual fields:

- `visible_user_prompt`: the human or endpoint prompt that should appear in Chariox prompt history and prompt input surfaces
- `hidden_system_context`: Chariox runtime instructions, workflow-level prompt, node-level instructions, granted MCP/skill summaries, runtime continuation instructions, and utility-specific instructions
- `attachments`: structured attachments, unchanged from the prompt submission path
- `manifest`: prompt template ids, template versions or content hashes, assembly conditions, and the provider injection channel used for the turn

Prompt text is loaded from the user-owned Chariox prompt registry under `~/.chariox/prompts`. The source tree may ship default templates, but those defaults are materialized into that registry and then read from disk. New prompt text should be added as a named template file, not as hardcoded Rust or TypeScript string literals. Template loading should fail loudly when a required template is missing or unreadable unless the caller is explicitly in a first-run/default-materialization path.

The registry layout is intentionally ordinary markdown so later Chariox Cloud editing can operate on the same file model:

```text
~/.chariox/prompts/
  runtime/base.md
  runtime/workspace-live-sync.md
  runtime/native-permissions.md
  runtime/slice.md
  runtime/mcp-skill-continuation.md
  workflow/turn.md
  workflow/run-completion.md
  workflow/run-intermediate-output.md
  utility/commit-message.md
  utility/semantic-recall.md
  utility/vault.md
```

Provider adapters inject `hidden_system_context` through provider-native hidden/system surfaces on every turn:

- Codex: `thread/start.developerInstructions` or `thread/resume.developerInstructions` when the thread is created or resumed. Because Codex does not accept this context through `turn/start`, kernel-managed Codex runs hot-reload the Codex thread before a turn when the assembled hidden-context fingerprint changes.
- OpenCode: `POST /session/{id}/prompt_async` request body `system`
- Claude Code: `UserPromptSubmit` hook response `hookSpecificOutput.additionalContext`

Live drills against the installed provider harnesses confirmed OpenCode and Claude Code support turn-scoped hidden context. Current Codex app-server supports this context at thread creation/resume, so the kernel performs a managed thread hot reload before a Codex turn when mid-session hidden-context changes must be reflected.

Visibility rules:

- Chariox prompt blobs, terminal prompt history, and native TUI prompt input must show only `visible_user_prompt`.
- Chariox must not implement hidden context by visibly prepending instructions and redacting them later.
- The manifest may be stored for audit/debug, but hidden prompt body text should not be rendered in prompt UI.
- Provider-local histories may still expose provider-native hidden context. Codex history APIs, OpenCode message records, and Claude transcripts can all persist their hidden/system context internally; Chariox can keep its own UI clean but cannot make provider-local storage opaque.

Workflow prompt assembly keeps three semantic layers distinct:

- endpoint prompt: visible input for the workflow entry turn
- workflow-level prompt: hidden shared system context for every agent in that workflow run
- node-level instructions: hidden per-agent/per-node context

Those layers should only be deduplicated when the assembled text is literally identical after template expansion. Endpoint prompt and workflow/node prompts are not duplicate channels.

Utility prompts such as commit-message generation, semantic recall/history, and vault-related calls use the same assembly service and focused provider run. Chariox should not start a separate provider conversation just to run a utility prompt unless the user explicitly requests that execution model.

The live validation path for this boundary is `pnpm --filter @chariox/cli run prompt-assembly:drill`. It runs real Codex, OpenCode, and Claude turns through the kernel with an edited temporary prompt registry, verifies the provider can use the hidden registry token, and verifies Chariox user-prompt history remains visible-prompt only.

### 5.3.2 Native TUI Client Interface

Some agents can be launched through a provider-native TUI client interface, for example `chariox codex [session-ref]`, `chariox opencode [session-ref]`, or `chariox claude [session-ref]`.

Boundary rules:

- the kernel still owns the Chariox session, top-level agent, provider run, prompt history, output fanout, and active/idle state
- the provider TUI owns provider-native parameter controls and display semantics
- the native TUI launcher is responsible for starting the provider UI process and local provider-facing proxy needed by that provider
- the relay remains transport-only; it must not inspect or own provider-native traffic
- Chariox clients must not mutate model/variant for a provider run marked `native_tui`

Remote native TUI mode is a composition of the existing local native TUI and
remote leased-agent architectures. The home kernel remains the only session
authority. Provider TUIs and Chariox TUIs attach to the home session, submit
prompts through the home prompt path, and observe home session output. The home
kernel dispatches execution to the worker through the existing leased-agent
relay protocol. The worker kernel launches and drives the provider through the
same provider adapter/server path it uses for normal worker-owned runs. Any
provider-native proxy code is only an edge translator between home-kernel
session events and provider-native UI protocol or PTY rendering; it must not
own prompt state, history, permissions, attachments, or remote execution.
Provider-native permission prompts follow the same rule: a provider request
creates one kernel-owned `RuntimeInteraction`, projected to all Chariox clients,
and provider-native approval replies are routed back to that interaction where
the provider seam allows it.
Provider-native Claude credential enrollment uses the same authority boundary.
An attached client arms a bounded, one-time enrollment on the home kernel. A
later hosted helper request arrives over the encrypted relay client lane and is
accepted only when signed relay metadata proves the enrollment-specific
`Service` subject plus the arm's user and realm. The kernel projects one normal
secret `RuntimeInteraction` to every attached client and returns the first
callback only to the waiting helper. Callback-bearing requests and responses
bypass command-result caching, and callback values never enter session state or
event projection. Hosted `Service` identities are denied every other kernel
request. Cloud and relay remain outside the encrypted provider URL and
callback payload. This is an official Claude CLI callback bridge, not a Chariox
OAuth or PKCE implementation.
Codex/OpenCode use provider protocol proxies where available. Claude Code uses
a kernel-owned remote-rendered PTY because Claude Code's public integration
surface is terminal-first rather than a separable app-server protocol.
Claude hidden prompt context for native TUI runs is delivered through Claude
Code's `UserPromptSubmit` hook `additionalContext` response. Local launchers and
worker kernels answer a scoped hook context request with the same granted-skill
prompt context used by normal Chariox provider runs, while keeping that context
out of visible PTY input and native TUI transcript rendering.

Managed Claude native interfaces keep fullscreen PTY redraw bytes separate from
semantic transcript output. The PTY bytes travel as transient
`provider_terminal` records for the native renderer and are never persisted or
projected into agent panes. Semantic assistant/reasoning/tool output comes from
Claude's hook-provided transcript path, and the Claude `Stop` hook remains the
authoritative turn-settlement signal after a final deferred transcript drain.

Slice-backed native TUI mode is the same composition with a home-managed slice
as the worker execution environment. Provider TUIs still attach to the home
kernel session; `slice_ref` only selects where provider execution runs. Native
TUI clients must not attach directly to a slice kernel for the product path.

### 5.3.3 OpenCode Structured Adapter

OpenCode is the first provider where Chariox intentionally prefers a structured local provider protocol over PTY-only inference.

Target runtime flow:

- daemon launches `opencode serve` in the assigned worktree or workspace context
- daemon waits for the local OpenCode server health endpoint
- daemon creates or binds an OpenCode session for the Chariox provider run
- daemon submits prompts through the OpenCode session API
- daemon subscribes to the OpenCode SSE event stream
- daemon maps OpenCode session/message events into Chariox prompt lifecycle, notices, and client-facing output

Target signal mapping:

- prompt submit: OpenCode session prompt API
- `/<provider> ...`: OpenCode command list plus session command API
- turn abort: OpenCode session abort API
- turn busy/idle: OpenCode session status events
- incremental text: OpenCode message-part delta and part-update events
- assistant completion: OpenCode assistant message updates with completion timestamps
- provider errors: OpenCode session error events plus adapter protocol errors when the event/session transport itself fails

Implication:

- PTY process exit remains a provider-run liveness signal
- PTY idleness is not the completion signal for OpenCode once the structured adapter path exists

### 5.4 Workflow Lane Semantics

Workflow scheduling and node-to-node handoffs belong to the daemon's structured state/control surfaces, not the terminal lane.

Implications:

- node handoffs MUST use structured daemon-owned payloads
- node completion reports MUST be machine-parseable
- workflow barriers, fan-in, aggregation, and termination decisions MUST operate on structured runtime state
- PTY traffic MAY be observed by the daemon for runtime coordination when needed, but MUST NOT be reused as the inter-agent contract

### 5.5 Durable Runtime State

Durable kernel state separates bounded execution state from paginated historical
data. Kernel readiness restores workflow definitions, active runs, queues,
bindings and pending delivery safety state; completed runs, transcripts and recall
indexes remain lazy read models. Ordinary workflow transitions persist keyed
entity mutations rather than full session aggregates. See
[`DURABLE_RUNTIME_PERSISTENCE.md`](DURABLE_RUNTIME_PERSISTENCE.md) for the storage,
migration, readiness and disk-maintenance contract.

## 6. Security and Trust Boundaries

### 6.1 E2E Encryption Scope

User-generated content in transit must use session-scoped end-to-end encryption when crossing remote transport boundaries, including:

- terminal-entered prompts/content
- capability payloads (edit instructions, prompt templates)
- uploaded file payloads
- memory transfer package payloads
- compaction summary payloads

### 6.2 Relay Trust Model

Server acts as relay/registry and should not require content plaintext to perform core duties.

### 6.3 Session Key Isolation

Cryptographic context is session-scoped. A key compromise in one session must not imply compromise of other sessions.

## 7. Memory Architecture

## 7.1 Dual Memory Model

Chariox memory has two scopes:

- short-term memory: recent transcript/task continuity
- long-term memory: durable user/project guidance

## 7.2 Memory Update Mechanism

Daemon may call `request_memory_update` to refresh memory-relevant signals after provider compaction/reset or before transfer.
Daemon may call `request_compaction_summary` during user-triggered Chariox compaction before starting a fresh warmed run.

Fallback:

- if unsupported, daemon continues with Chariox-managed memory sources
- memory refresh failure must not terminate provider run

## 7.3 Context Transfer Package

Transfer package composes:

- selected short-term snapshot
- relevant long-term entries
- workspace state

Requirements:

- deterministic and auditable composition
- user control over long-term entry inclusion
- encrypted in transit

## 7.4 Extension Architecture

Chariox manages extensions in two phases:

- install: register an extension on the machine
- bind: make that extension available to a top-level Chariox-managed agent or provider run

The daemon owns:

- extension installation metadata
- compatibility and validation checks
- per-agent binding resolution
- provider-view materialization inputs
- MCP runtime lifecycle for bound MCP servers

Provider-native subagents are not separate extension targets; they inherit whatever their parent top-level provider run can access.

### 7.5 Chariox-Driven Context Compaction

Chariox provides a user-triggered compaction command: `/compact`.

Compaction sequence:

1. daemon requests compaction summary via `request_compaction_summary`
2. daemon stores summary as a session artifact/memory input
3. daemon launches fresh provider run with empty context window
4. daemon warms new run with compaction summary + selected Chariox memory/workspace context

This flow is daemon-orchestrated and separate from ordinary user prompt traffic.

## 8. Failure and Degradation

Mandatory behavior:

- adapter lacking `attach_file`, `request_memory_update`, and/or `request_compaction_summary` does not break core PTY usage
- control operation failures are isolated and user-visible
- remote client disconnect does not terminate session by default
- workflow node failure propagation and retry policy MUST remain daemon-owned and explicit
- workflow concurrency/resource limits MUST be centrally enforced by the daemon runtime
- unsupported provider versions emit compatibility warnings but retain best-effort `/<provider> ...` completions
- provider-auth failures are surfaced as structured local host warnings; Chariox owns account metadata and orchestration while official provider CLIs remain credential-format and token-refresh authorities
- relay-mediated remote attachment must not change daemon authority over sessions, provider runs, or workflow state

## 9. Deployment and Evolution Notes

v1 is local-first and single-active-host-per-session. Architecture should remain forward-compatible with:

- relay-backed remote terminal/client attachment
- daemon identity and machine identity for remote registration
- richer multi-machine scheduling/migration
- expanded provider adapter capabilities
- optional content persistence policies
- more advanced workflow topologies
- bounded loops and other cycle policies
- multi-user and team workflows
- richer aggregation policies and barrier behavior
- explicit merge or reconciliation stages
- per-node provider, model, and account selection

## 10. Implementation Choices (v1 baseline)

Contributor workflow conventions (coding style, testing, PR hygiene) are documented in `docs/CONTRIBUTING.md`.

This section captures current implementation choices for v1 so engineering work has a stable baseline. These are implementation defaults, not product invariants, and may evolve with explicit architecture updates.

### 10.1 Monorepo and Package Management

- monorepo layout
- pnpm workspaces

### 10.2 Client Stack

- React
- TypeScript
- xterm.js for terminal rendering

### 10.3 Daemon Stack

- Rust (required for v1 daemon implementation baseline)

### 10.4 Backend Stack

- Fastify

### 10.4.1 Relay Stack

- Rust for the relay implementation baseline
- independent app/process, separate from both daemon and CLI
- shared protocol/domain model should be reused where practical, while keeping relay transport-only

### 10.5 Data Layer

- Prisma
- SQLite for early/local phases
- Postgres as scale-up target

### 10.6 Transport and Local IPC

- WebSockets for kernel-facing client transport
- relay transport later forwards the same logical client/kernel protocol
- Unix socket on Unix-like systems remains as a local compatibility and harness path
- named pipe on Windows remains a later local compatibility follow-up

Current local runtime note:

- the daemon now hosts a kernel WebSocket listener directly and the TypeScript CLI uses that path by default
- the Unix-socket local transport remains implemented for local harnessing/tests and backward-compatible tooling
- Windows local compatibility transport remains a later follow-up
- kernel-client transport hardening now includes event ids, resumable subscribe, heartbeat events, and reconnect-friendly client behavior

### 10.6.1 OpenCode Integration Strategy

- M2 baseline: PTY-launched OpenCode wrapper path
- current M3 direction: daemon-launched `opencode serve` plus local HTTP/SSE adapter
- current implementation launches Chariox-owned OpenCode provider processes rather than attaching managed runtime sessions to external OpenCode endpoints
- adapter-owned OpenCode session/event handling should remain behind daemon/provider abstractions so later providers can still use PTY or their own structured surfaces without changing client contracts
- OpenCode remains the only agent-side structured transport that Chariox is currently tightening closely against; a generic agent WebSocket protocol is intentionally deferred until more agent integrations exist

### 10.7 Governance

Implementation choices should be revised when they materially change runtime architecture, protocol assumptions, security posture, or operational behavior.

### 10.8 Cross-Platform Terminal Consistency Strategy

Chariox should use a shared terminal behavior contract while allowing platform-native implementation languages.

Approach:

- define canonical terminal behavior in protocol/conformance terms (PTY byte stream handling, resize semantics, key mapping expectations, control-sequence fidelity)
- use xterm.js as the web/remote reference implementation and golden-behavior baseline
- keep slash-command parsing, completion semantics, and warning behavior consistent across clients

Platform framework options for xterm.js-consistent rendering:

- Web: browser-hosted xterm.js
- iOS: `WKWebView` hosting xterm.js plus native shell for platform integration
- Android: `android.webkit.WebView` hosting xterm.js plus native shell for platform integration
- macOS desktop: `WKWebView` (AppKit/SwiftUI host) with xterm.js
- Windows desktop: WebView2 (`Microsoft.Web.WebView2`) with xterm.js
- Linux desktop: embedded Chromium/WebKit host (for example Electron or GTK WebKit) with xterm.js
- CLI/TUI clients: native terminal stack is allowed, but must pass the same conformance profile for input/output/resize semantics

Result:

- consistent remote terminal behavior across platforms
- freedom to use standard language/tooling per target platform
