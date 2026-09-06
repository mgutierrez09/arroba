# Tauri terminal parity inventory

Status: implementation baseline (2026-08-19)

This inventory defines the reference behavior, ownership boundary, and validation
surface for the shared Chariox terminal. The reference is
`chariox-cloud/apps/web`; the installed application belongs in this repository.
It is a migration checklist, not a claim that the Tauri targets are complete.

## Authority and transport invariants

- The kernel remains the authority for sessions, agents, prompts, history,
  workflows, permissions, interactions, workspaces, and runtime state.
- Cloud remains the identity, account, billing, relay-bootstrap, and hosted
  control plane. It does not proxy terminal packets.
- Runtime traffic remains `client <-> relay <-> kernel <-> agent` for remote
  sessions and `client <-> kernel <-> agent` for local sessions.
- Tauri is a client shell and operating-system integration layer. It must not
  create a second prompt, permission, attachment, history, event, or relay path.
- The existing kernel and relay protocols remain the source of truth. Any
  serialized protocol change must follow the version, snapshot, and drill rules
  in `AGENTS.md`.

## Reference implementation audit

The browser terminal is a hybrid runtime, not a single React root:

- `chariox-cloud/apps/web/src/react/main.tsx` delegates terminal routes to the
  separately built `/terminal-runtime.js` entry.
- `chariox-cloud/apps/web/src/client.ts` starts
  `terminal/app/terminal-browser-app.ts`, the current composition root.
- React owns route shells, panes, overlays, workflow views, side panels, prompt
  pieces, and waiting-room surfaces.
- Imperative controllers, projections, and the Zustand-backed portal bridge
  coordinate those React mounts with transport and browser lifecycle behavior.
- The runtime uses the terminal, React, kernel, UI, workflow, design-system, and
  style trees together. Moving only `src/react/terminal` would omit behavior and
  create a divergent client.

The combined terminal runtime and stylesheet dependency closure contains 657
non-test files (about 168,865 lines), including 370 terminal
controllers/projections, 129 React modules, 41 kernel transport/request modules,
51 UI modules, 38 style modules, 15 design-system modules, and 13 other/helper
modules.
The browser build exposes terminal routes for `/terminal`, `/test`,
`/waiting-room`, `/machines`, `/recall`, `/slices`, `/view`, `/workflows`,
`/deployed`, `/settings`, `/sessions/invites`, `/invite/*`, and conditional
`/activate?user_code` handling.

The extraction unit is therefore the complete runtime dependency closure. It
will become an OSS-owned `@chariox/web-terminal` package with injected adapters
for environment-specific identity, Cloud control-plane requests, secure storage,
deep links, files, notifications, and kernel lifecycle. The Cloud browser entry
and Tauri entry must compose that same package. A pure-React rewrite is not a
prerequisite and must not run in parallel with this migration.

Cross-repository consumption must use a pinned, reproducible package artifact;
permanent sibling-directory imports are prohibited. The exact publication and
release pin are established with the package-boundary subtask before Cloud is
switched to the package.

## Environment adapter boundary

Shared terminal code may depend on narrow interfaces for:

- current identity and Cloud control-plane requests;
- relay target discovery and scoped relay-token bootstrap;
- kernel/relay transport creation and lifecycle signals;
- credential persistence without exposing secrets to shared UI state;
- deep links and OAuth callback delivery;
- file/photo selection, sharing, export, clipboard, and notifications;
- desktop kernel discovery, launch, readiness, and shutdown.

Browser adapters retain cookie authentication and CSRF. Installed adapters use
the system browser with Authorization Code and S256 PKCE, short-lived Bearer
credentials, and OS-backed secure storage. Provider handling must remain neutral
because Cloud currently supports Auth0, WorkOS, and WorkOS-primary dual mode.
Tauri capabilities must grant only the operations required by these adapters.
OAuth must never run inside the application WebView. No installed client may
embed a client secret or place credentials in local storage, plaintext
preferences, logs, screenshots, fixtures, or error messages.

Cloud-only deployment, publication, billing, infrastructure, administration,
marketing, and browser device-approval behavior stays in `chariox-cloud` behind
optional route/sidebar contributions. Kernel-backed settings, waiting room,
runtime, workflows, slices, view, recall, workspace, provider accounts,
extensions, notifications, and vault behavior belongs in the shared package.

## Known extraction constraints

- The Cloud browser relay client already implements request correlation, event
  subscriptions, replay cursors, heartbeat watchdogs, reconnect, control replay,
  and lane handling. Its Web Crypto ECDH P-256, HKDF-SHA256, and AES-GCM path
  must be transferred unchanged before it is reconciled with OSS client code.
- The OSS `@chariox/kernel-client` root currently imports Node filesystem,
  `node:crypto`, `Buffer`, and `ws`. It cannot be shipped wholesale in a WebView.
  Browser-safe entry points must use `Uint8Array`, native `WebSocket`, and Web
  Crypto; Node polyfills are not an acceptable installed-client solution.
- A WebView WebSocket cannot set the Bearer header required by the local kernel.
  Desktop local mode therefore needs a narrow Rust transport bridge that opens
  the authenticated loopback socket and forwards existing serialized frames
  without interpreting runtime state. Remote relay transport remains direct in
  shared TypeScript so relay encryption stays end to end.
- The kernel can be built as the `chariox-kernel` sidecar. Desktop lifecycle must
  detect an existing kernel, use a one-shot auth file, wait for readiness, bound
  diagnostic output, and shut down only a process owned by the app. Mobile must
  not bundle or launch the kernel.
- Kernel cleanup currently follows its Ctrl-C path. The sidecar manager must
  prove a graceful signal or Windows console-control path on all three desktop
  systems before using forced termination. It must also handle the sparse `PATH`
  of GUI-launched apps: provider executable discovery and missing-provider
  diagnostics belong in the kernel/shared configuration, not Tauri-only policy.
- The old bundle identifier is `dev.chariox.ios`; it is retained as a migration
  input until signing and provisioning configuration can confirm the final ID.
- Production non-loopback kernel and hosted-relay connections require `wss://`
  with normal platform certificate validation. iOS App Transport Security and
  Android network-security configuration must reject cleartext remote endpoints
  by default; explicit development exceptions must be narrow and excluded from
  release builds.
- Current CI has no installed-platform jobs. Platform support requires native
  macOS, Windows, and Linux runners, Xcode for iOS, and Gradle/Android tooling for
  Android; a build for one operating system is not evidence for another.

## Old iOS replacement scope

The tracked `apps/ios` tree is the obsolete SwiftUI implementation: its Xcode
project/workspace, Swift package, application and UI-test targets, assets,
configuration, and app-specific instructions. The first iOS migration commit
must remove that implementation and atomically add the Tauri replacement entry
point. It must not leave two iOS clients or an unexplained missing target.

The Swift protocol tests are useful as behavioral evidence, but their Swift
implementation must not survive as a second client. Equivalent protocol framing,
replay, session, prompt, cancellation, and command behavior belongs in shared
TypeScript tests around the existing kernel client and web terminal package.
Historical drill records remain historical; current SwiftUI plans and task
descriptions must be replaced.

## Feature parity matrix

Legend: `R` reference behavior inventoried, `P` implementation or verification
pending, `G` required behavior missing from the browser reference. A row may be
marked verified only with the test or evidence named in the final validation
record. Every `G` must be implemented in the shared package and verified in the
browser before the migration can complete.

| Capability | Browser | iOS | Android | macOS | Windows | Linux |
| --- | --- | --- | --- | --- | --- | --- |
| App shell, sidebar, resize, mobile rail, route persistence | R | P | P | P | P | P |
| Waiting-room kernels, freshness, projects, sessions, workflows | R | P | P | P | P | P |
| Waiting-room agents, collaborators, workspaces, and worktrees | R | P | P | P | P | P |
| Cloud sign-in, activation, and invitation acceptance | R | P | P | P | P | P |
| Direct hosted-relay terminal transport and encryption | R | P | P | P | P | P |
| Existing reachable-kernel connection | R | P | P | P | P | P |
| Local-kernel start/readiness/shutdown | N/A | N/A | N/A | P | P | P |
| Session create, join, restore, reload, close, attach, detach | R | P | P | P | P | P |
| One-to-six agent desks, focus, aliases, status, and footers | R | P | P | P | P | P |
| Agent spawn, destroy, fork, configuration, and placement | R | P | P | P | P | P |
| Transcript history and durable paging | R | P | P | P | P | P |
| Transcript virtualization | G | P | P | P | P | P |
| Streaming output, reasoning, tools, code, files, and failures | R | P | P | P | P | P |
| Prompt submit, cancellation, queued prompts, and draft retention | R | P | P | P | P | P |
| Commands, slash commands, mentions, and keyboard discovery | R | P | P | P | P | P |
| Attachments, previews, progress, isolation, and history | R | P | P | P | P | P |
| Kernel-owned permissions and runtime interactions | R | P | P | P | P | P |
| Runtime drawer, diagnostics, and side panels | R | P | P | P | P | P |
| Workflow canvas, inspectors, triggers, schedules, and queues | R | P | P | P | P | P |
| Workflow run, pause, resume, stop, console, trace, steering | R | P | P | P | P | P |
| Slices, headed view, display endpoint, and trace history | R | P | P | P | P | P |
| Workspace files, compare, Git, worktrees, and live sync | R | P | P | P | P | P |
| Provider accounts, profiles, login, logout, and refresh | R | P | P | P | P | P |
| Extensions, grants, worker/home capabilities, and sync | R | P | P | P | P | P |
| Vault metadata and locked/unlocked state without secret values | R | P | P | P | P | P |
| Recall query, results, history, loading, empty, and error | R | P | P | P | P | P |
| Prompt settings, save/reset, dirty guard, disconnected mode | R | P | P | P | P | P |
| Empty, loading, busy, cancelled, unavailable, and failed states | R | P | P | P | P | P |
| Disconnect, reconnect, heartbeat, replay, and replay gap | R | P | P | P | P | P |
| Scroll restoration, anchoring, deduplication, and shortcuts | R | P | P | P | P | P |
| Theme tokens, dark/light persistence, and system preference | R | P | P | P | P | P |
| Typography, font fallback, text scale, and information density | R | P | P | P | P | P |
| Colors, backgrounds, borders, shadows, and spacing | R | P | P | P | P | P |
| Icons, agent-pane footers, and global footer | R | P | P | P | P | P |
| Responsive phone, tablet, rotation, and desktop layouts | R | P | P | P | P | P |
| Safe areas, software keyboard, touch targets, and sheets | R | P | P | N/A | N/A | N/A |
| Background/foreground reconnect | N/A | P | P | P | P | P |
| Native files, photos, artifacts, share, export, and clipboard | N/A | P | P | P | P | P |
| Deep links for login, activation, sessions, and invitations | R | P | P | P | P | P |
| In-app kernel notification center | R | P | P | P | P | P |
| Native permission/completion notifications | N/A | P | P | P | P | P |
| Accessible labels, focus, listbox semantics, and dynamic text | R | P | P | P | P | P |
| Secure credential persistence | Cookie | P | P | P | P | P |
| Signed packages and update path | N/A | P | P | P | P | P |

## Required reference states

Visual comparison must cover the following states at the same viewport class and
with equivalent fixture data wherever possible:

1. Waiting room.
2. Connected single-agent terminal.
3. Multi-agent terminal.
4. Active streaming response.
5. Tool call.
6. Runtime permission interaction.
7. Workflow view.
8. Reconnecting state.
9. Mobile composer with the software keyboard visible.

Evidence belongs under `/Users/miguel/.codex/evidence/tauri-terminal/`, never in
Git. Target evidence sets are browser, macOS, Windows, Linux, iPhone, iPad,
Android phone, and Android tablet. A platform moves from `P` to verified only
when matching-state images are recorded side by side at the same logical
viewport dimensions and the comparison checks typography, colors, backgrounds,
borders, shadows, spacing, density, icons, pane footers, global footer, and
responsive presentation. Any accepted platform-specific difference is recorded
with its reason.

## Validation gates

The migration is not complete until automated coverage proves shared reducers
and state, request/response framing, relay encryption and routing, reconnect and
replay-gap recovery, authentication callback and token lifecycle, prompt/draft
behavior, cancellation, rendering, interactions, agent panes, workflows,
attachments, platform permissions, and desktop sidecar lifecycle.

Security validation must also prove negative properties: no WebView OAuth; no
embedded client secret; no credential in local storage, plaintext preferences,
logs, screenshots, errors, or fixtures; no broad shell, filesystem, or network
capability; no `dangerousRemoteDomainIpcAccess`; no mobile `externalBin`; and no
cleartext production remote connection. Capability manifests must be
target-scoped and checked against the commands the frontend actually invokes.
Tests must cover a successful system-browser login and secure-store round trip,
as well as rejection of mixed cookie/Bearer credentials and invalid Bearer
fallback.

Sidecar tests must exercise existing-kernel detection, readiness timeout,
bounded diagnostics, provider discovery from a GUI launch environment, and
graceful app-owned shutdown on macOS, Windows, and Linux. Mobile tests must prove
App Transport Security and Android network-security rejection of cleartext
non-loopback endpoints in release configuration.

Real runner builds are required for macOS, Windows, and Linux. iOS must build
through Xcode and Android through Gradle. Browser regression tests must be run
before extraction and again after Cloud consumes the shared package. Baseline
failures from a clean main checkout must be recorded separately and may not be
presented as migration regressions or ignored without explanation.

## Reviewable migration slices

1. Establish the OSS package boundary and deterministic Cloud consumption model.
2. Move shared protocol, relay, state, styles, and terminal runtime behavior.
3. Switch the Cloud browser composition root to the shared package with browser
   regression coverage.
4. Replace the SwiftUI tree atomically with the Tauri application structure.
5. Add desktop shell, least-privilege capabilities, and kernel sidecar lifecycle.
6. Add provider-neutral installed PKCE, Bearer admission, secure storage, renewal,
   logout, and relay bootstrap.
7. Add iOS and Android projects, lifecycle, deep links, files, sharing, and
   notifications.
8. Complete responsive phone/tablet behavior and visual parity work.
9. Add signed packaging, updates, real-platform CI, documentation, and final
   functional, security, and visual verification.

Each slice is committed only after focused validation. At least two independent,
read-only reviews inspect its exact commit SHA before it is pushed; any changed
SHA is reviewed again.
