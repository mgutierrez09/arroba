# Deployed Workflows Threat Model

Status: normative baseline for managed Agent App activation, updated 2026-07-16

This threat model applies to Chariox-managed workflow and Agent App deployments.
It complements `DEPLOYED_WORKFLOWS_AGENT_APP_PLATFORM_PLAN.html` and the runtime
boundaries in `ARCHITECTURE.md`. A later design may add controls, but it must not
weaken these invariants without an explicit security review and migration plan.

Implementation status is intentionally narrower than the normative model. The
committed OSS and Cloud branches implement the v3 package contract, runtime
compatibility admission, provider CLI pinning, process isolation/supervision,
resource ceilings, caller claims, protocol-241 callback authority and worker,
relay service-key proof of possession, Cloud callback claim binding, web/TUI
callback arming, and fail-closed hosted egress policy/orchestration. The final
local two-surface matrix passes on OSS `4d503ba98` and Cloud `6ca6fffe` with
stable provenance, resource evidence, and clean teardown. That local result is
not production activation evidence; the live-provider, DNS/TLS, and guarded
Hetzner residual gates below remain open until exercised end to end.

## Scope

In scope:

- Cloud deployment control APIs and web UI
- OSS CLI/TUI Cloud deployment commands
- publication package upload, storage, download, and materialization
- hosted runners, publication containers, and local-runtime ingress registration
- per-revision hosted networks, egress policy compilation, CONNECT gateways, and cleanup
- public HTTP, SSE, WebSocket, and MCP ingress
- account handoff, audience access, credential binding, domains, logs, and usage
- relay/bootstrap traffic used to reach the home kernel

Out of scope for Phase 0 approval:

- claiming that current public slug URLs provide audience authentication
- transferring provider credential bytes through Cloud
- arbitrary persistent workflow patches in managed deployments
- managed application databases or durable customer application storage
- compliance certification or an SLA not backed by implemented controls

## Security Objectives

1. An actor can control only deployments in an account where their authenticated
   identity has the required role.
2. A public hostname or canonical deployment route resolves to exactly one
   deployment. Ambiguity fails closed.
3. Cloud, relay, ingress, logs, and browser clients do not receive provider
   credential payloads.
4. A managed hosted release is an immutable package v3 plus a digest-bound
   deployment contract. Configuration, credential bindings, provider bundles,
   audience policy, and egress policy cannot broaden its declared ceilings.
5. Runtime callers cannot forge Chariox identity, runner, session, or internal
   transport headers.
6. One caller cannot read another caller's state, prompts, output, attachments,
   traces, overlays, credentials, or quota state.
7. Managed deployments cannot apply persistent patches until reviewable diffs,
   authorization, audit, rollback, expiry, and concurrency controls exist.
8. Runtime and control-plane failures fail closed without silently switching
   account, deployment, credential, release, or audience identity.
9. Hosted package, kernel, gateway, action, and provider processes remain inside
   explicit CPU, memory, PID, file, storage, timeout, and lifecycle supervision.
10. A hosted revision has no direct public egress path. It can reach only an exact
    declared TLS/443 destination through its dedicated policy gateway, with no
    legacy or availability fallback.

## Actors And Trust

| Actor | Trusted for | Not trusted for |
| --- | --- | --- |
| Platform operator | Operating Chariox infrastructure under audited access | Customer ownership decisions or reading provider secrets by default |
| Account owner/admin | Account policy, deployment lifecycle, handoff, billing, and credential bindings | Other accounts or platform-wide policy |
| Deployer/operator | Explicit deployment and runtime operations granted by role | Ownership, billing, or capability expansion |
| Builder | Supplying source and an immutable package for review | Customer credentials, destination ownership, or undeclared capabilities |
| Customer reviewer | Accepting a claim and selecting destination policy | Builder source account or unrelated customers |
| End user | Invoking routes granted by audience policy | Control APIs, other callers, secrets, logs, or provider state |
| Machine caller | Invoking explicit routes with scoped credentials | Interactive sessions or unrelated routes |
| Home kernel | Session, workflow, agent, interaction, queue, and execution authority | Cloud account or billing authority |
| Hosted runner | Materializing approved packages and reporting observed runtime state | Changing desired state or account policy |
| Provider CLI | Provider-native execution and provider-local credentials | Chariox account authorization or ingress identity |
| Relay | Admitting scoped connections and routing encrypted packets | Runtime authority or plaintext inspection |
| Public ingress | TLS, host routing, audience auth, quotas, and trusted claim injection | Workflow or provider execution authority |

Builders and account owners can deploy reviewed code into their own destination
account. Package code is still untrusted with respect to provider credentials,
the kernel control transport, the platform runner credential, other deployments,
and other callers. Accepting a release does not authorize package code to cross
those boundaries.

## Protected Assets

- account, organization, deployment project, environment, and billing ownership
- immutable package bytes, digest, provenance, release signature, and active pointer
- deployment contract, provider bundle references, egress policy digest, revision
  networks, gateway identity, and pinned upstream addresses
- provider-native credentials and external integration credentials
- deployment configuration and credential-binding versions
- audience identities, allowlists, API keys, sessions, and invocation claims
- prompts, outputs, attachments, workflow state, traces, and overlays
- custom-domain verification and TLS state
- runtime volumes, container identity, routes, queues, and replica affinity
- audit events, logs, usage records, budgets, and incident artifacts

## Trust Boundaries

### Browser Or CLI To Cloud Control Plane

- Browser mutations require an authenticated Cloud session and CSRF token.
- Bearer clients require a valid Cloud session token; CSRF does not replace bearer
  authentication.
- Account and creator identity are derived and verified server-side. Body or query
  identifiers are lookup hints, not authorization evidence.
- Read access requires membership. Managed lifecycle mutations require owner or
  admin access until narrower deployment roles are implemented.
- Cross-account reads and mutations return no target details.

### Cloud To Runner

- Cloud expresses desired state and issues account-scoped runner work.
- Runner identity is an opaque hashed credential and is scoped to one account.
- The runner materializes the selected package and reports observed state. It does
  not decide ownership, audience, billing, or desired release.
- Managed runners inspect the materialized manifest before Docker and reject
  persistent patch capability. Reconciliation removes legacy unsafe containers.
- A successful queued job is not proof of a healthy deployment. Ingress activates
  only from ready observed state with a backend target.

### Package Boundary

- Package archives are untrusted input until size, structure, digest, contract,
  and capability verification complete.
- Extraction must prevent path traversal, symlink escapes, device files, archive
  bombs, and writes outside the deployment package directory.
- Package v3 is the native managed-hosting format. Its deployment contract binds
  package/source identity, the SHA-256 digest of all other package files,
  compatibility floors, routes, credential slots, capabilities, resources,
  network ceilings, provider bundle references, and presentation metadata.
- Cloud admission must verify the v3 archive and contract before release
  creation, preserve one immutable release pointer, and allow a job only on a
  runner whose immutable image reports a compatible local daemon protocol.
- Package v1/v2 and the old `egress_policy: deployment_tightens` marker are
  explicit legacy adapters. They may remain usable locally, but hosted start,
  restart, promotion, and recovery must reject them as obsolete unrestricted packages
  and reject them before a runner can claim the job.
- Package identity and capability declarations are immutable release data.
- Cloud stores package bytes or object references, not provider credentials.
- Reupload cannot silently activate a release; readiness and activation remain
  explicit lifecycle operations.
- Digest verification is implemented. The present empty contract signature list
  is not a production provenance signature; signing and key policy remain an
  activation decision if the release trust model requires them.

### Public Ingress To Runtime

- Canonical routing authority is a globally unique deployment/environment identity
  or verified hostname. Human-readable slugs are presentation only.
- Legacy slug-only routing is allowed only when exactly one globally ready match
  exists; zero or multiple matches fail closed.
- Caller-supplied internal Chariox and identity headers are removed.
- Audience authentication must complete before opening HTTP streams, SSE,
  WebSockets, or MCP sessions.
- A later signed invocation envelope must bind deployment, environment, subject,
  organization, roles, audience, expiry, nonce, and invocation ID and be verified
  again at the runtime boundary.
- Dynamic/authenticated responses are never CDN cached.

### Kernel And Provider Boundary

- The home kernel remains runtime authority. Cloud is bootstrap and control plane,
  not a second runtime implementation or long-stream proxy.
- Providers run through official provider harnesses. Provider credentials remain
  in provider-native stores on the execution machine.
- Structured identity claims are not automatically inserted into model-visible
  prompt text.
- Runtime interactions, including permissions and user requests, remain
  kernel-owned and are projected consistently to web and TUI clients.

### Credential Enrollment Boundary

- A credential setup is a short-lived, one-time enrollment bound to account,
  profile, target version, runner, mode, and expiry. Claim and consumption are
  atomic; a wrong runner, stale version, expired enrollment, or replay fails
  closed.
- Runner jobs use expiring leases and monotonic claim attempts. Heartbeats renew
  only the current attempt; restart may reclaim an expired lease on the same
  runner, and a superseded attempt cannot report progress or completion.
- Cloud stores enrollment status, account label when provider-native
  verification returns one, and opaque runtime references. It never receives
  provider setup URLs, provider credential files, provider tokens, OAuth
  callback codes, or integration secret values.
- Provider setup URLs and replies that must travel back to a login process,
  including Claude callback values, use the encrypted relay client lane directly
  between the hosted helper and home kernel. Relay-authenticated metadata must
  prove the enrollment-specific hosted `Service` subject plus the armed user and
  realm; body fields are not identity. Relay and Cloud see no URL or callback
  payload.
- Local daemon protocol 241 provides the OSS checkpoint: an attached client arms
  an exact enrollment/profile/version/session/focused-agent tuple in bounded
  kernel memory, the verified helper atomically consumes it, and one normal
  secret `RuntimeInteraction` is projected to all attached clients. First reply
  wins; cancel and timeout return no secret; callback-bearing commands bypass the
  response cache and callback values are not persisted or reprojected.
- The direct callback channel bridges the official Claude CLI. Chariox does not
  implement provider OAuth authorization, token exchange, or PKCE.
- Web and TUI arm the same home-kernel request path. Cloud stores an immutable
  route bound to account, enrollment, profile, target version, realm, kernel,
  session, focused agent, and arming user. The helper bootstrap additionally
  requires the exact live runner/job/claim attempt and ephemeral public-key
  thumbprint, and mints only the enrollment-specific `Service` subject and exact
  kernel target. A stale heartbeat, expired lease/enrollment, runner swap,
  route mutation, wrong user, or replay fails closed.
- Before decrypting or dispatching a relay request from a `Service` identity that
  carries `public_key_thumbprint`, the kernel must compare the claim with the
  SHA-256 digest of the encrypted envelope's `sender_public_key`. A mismatch
  fails closed; relay identities without that service thumbprint contract keep
  their existing behavior.
- Provider-native verification is an explicit runner attestation after the
  official provider harness reports ready. Copying a pre-existing file is not
  provider-native verification.
- Runner-seeded enrollment is an explicit local/self-hosted migration and drill
  mechanism. Hosted Cloud disables it by default, and every surface labels its
  identity as unverified.
- Enrollment sources are minimal, permission-restricted, symlink-free, bounded,
  manifest-bound to the enrollment, consumed once, and removed after use.
- Every credential-bearing deployment and replacement job is pinned to the one
  runner that owns all of its materialized profiles. Mixed, missing, and live
  cross-runner bindings are rejected before desired state changes.

### Hosted Egress Boundary

- The immutable v3 contract is the maximum egress ceiling. An environment may
  select fewer destination and provider-slot IDs, never add or alter one. The
  compiled policy is canonical, digest-bound, deny-by-default, and contains only
  exact lowercase DNS names on TLS port 443.
- Each hosted revision uses its own Docker `--internal` network. The publication
  container attaches only to that network and has no direct route to a public or
  shared egress network.
- One dedicated unprivileged CONNECT gateway is dual-homed on the revision's
  internal network and a separate egress-capable network. It exposes no ordinary
  HTTP proxying and accepts only policy-listed CONNECT authorities on port 443.
- Before connecting, the gateway resolves all A and AAAA answers and rejects the
  entire answer set if any address is loopback, private, link-local, metadata,
  reserved, mapped, multicast, documentation, or otherwise non-public unicast.
  It connects directly to one validated numeric address from that answer set;
  later DNS changes cannot redirect the open tunnel.
- The first tunneled bytes must be a bounded TLS ClientHello whose canonical SNI
  equals the CONNECT host exactly. Missing, duplicate, malformed, mixed-case, IP,
  wildcard, alternate-port, unknown-host, or mismatched-SNI requests fail closed.
- The gateway and publication network are revision-scoped supervised resources.
  Gateway startup/health failure, DNS failure, policy mismatch, unsupported
  provider bundle, or teardown race blocks activation or invocation. There is no
  direct, legacy-unrestricted, or availability fallback.
- OSS contract validation and the standalone gateway are committed. Cloud policy
  compilation, hosted legacy rejection, runner wiring for revision-scoped
  internal networks, dual-homed gateways, host enforcement, recovery, cleanup,
  and the aligned API-to-worker lifecycle snapshot are also committed. The local
  final-revision matrix passes; the topology remains an activation gate until the
  same fail-closed behavior passes on the designated Hetzner host.

### Hosted Runtime Process Boundary

- Kernel/provider execution, the publication gateway, and package-supplied
  actions run as distinct unprivileged identities with separate homes. Package
  actions receive an allowlisted environment and cannot traverse provider homes.
- A strong kernel-local transport credential is generated inside the container,
  never passed by Docker or Cloud, removed from the kernel environment before it
  launches provider children, and unavailable to gateway-unrelated identities.
  The kernel keeps only its private in-memory copy and rejects unauthenticated
  local WebSocket handshakes when the credential is configured.
- The platform runner key never enters the deployment container. Runtime audit
  delivery uses a deployment- and revision-scoped capability to a runner-owned
  bridge, which verifies that the runtime is active, candidate, or draining
  before forwarding bounded entries.
- The publication image installs exact versioned official provider packages and
  verifies each CLI version at image build. The current pins are Codex 0.144.0,
  OpenCode 1.18.23, and Claude Code 2.1.212. Runners launch the resolved immutable
  image ID, and the v3 contract carries stable provider bundle references.
- The runner validates limits before Docker. Current defaults are 2 GiB memory
  with swap disabled, 2 CPUs, 256 PIDs, and 4,096 open files, plus bounded
  concurrency, queue, body, duration, usage, ephemeral storage, and tmpfs.
  Unknown, fractional, or out-of-range values fail before launch.
- `tini` owns PID 1. The standalone entrypoint supervises kernel, publication
  gateway, and optional action server. Unexpected exit of any required child
  terminates and waits for siblings, then exits nonzero for runner reconciliation.
- Read-only root filesystems, no-new-privileges, bounded tmpfs mounts, provider
  cache separation, rolling candidate/drain cleanup, and explicit process cleanup
  remain defense in depth; none substitutes for the identity, transport, or
  network boundaries above.

### Relay Boundary

- The relay routes encrypted packets and enforces scoped admission.
- It does not inspect or persist prompts, outputs, attachments, workspace data,
  provider payloads, or session history.
- Relay availability never changes kernel authority or account identity.

## Primary Threats And Required Controls

| Threat | Required prevention or detection |
| --- | --- |
| Forged account or creator fields | Session-derived actor plus repository membership checks |
| Browser cross-site mutation | CSRF on browser mutations, secure cookie policy, Origin/CORS tests |
| Cross-account object reference | Account-scoped service/repository lookup and denial regression tests |
| Duplicate slug hijack | Canonical deployment ID or a verified deployment hostname is required |
| Runner credential theft | Hashed opaque key, account scope, rotation/revocation, no logging |
| Package substitution | V3 archive/contract digest verification, immutable release pointer, and production authenticity policy |
| Obsolete package activates hosted | Managed admission rejects v1/v2 and packages without an enforced deny-by-default policy before job claim |
| Provider CLI or bundle substitution | Exact package versions verified at image build, immutable image ID launch, and contract-bound bundle refs |
| Malicious archive | Bounded structured extraction and traversal/bomb fixtures |
| Credential exfiltration | Provider-native stores, opaque bindings, redaction, egress policy |
| Enrollment replay or runner swap | One-time account/profile/version/runner binding, expiry, atomic claim/consume, source manifest |
| Worker crash or duplicate completion | Expiring same-runner lease, monotonic claim attempt, heartbeat, stale-attempt rejection |
| Setup URL/code disclosure | Encrypted helper-to-kernel relay lane, exact signed service binding, strict URL validation, no Cloud projection |
| OAuth callback or integration secret retained by Cloud | One-time direct runtime/relay input channel; no ordinary control-plane fields or logs |
| Callback claimed by another user/helper | Exact arming user, realm, service subject, job/attempt, runner, target, tuple, TTL, and ephemeral key binding |
| Thumbprint-bound service token replayed with another helper key | Kernel compares `SHA-256(sender_public_key)` with the service identity's `public_key_thumbprint` before decrypt/dispatch |
| Package action reads provider credentials | Separate UID/home, permission-restricted mounts, sanitized environment, denial probe |
| Package action controls kernel over loopback | In-container random kernel auth, authenticated handshake, token removed before provider launch |
| Runtime steals platform runner authority | Runner key absent from container; revision-scoped runner audit bridge capability only |
| Rotation deletes the serving credential | Preserve prior profile through candidate activation/drain; durable post-convergence GC |
| Revocation reports success while runtime serves | Process STOP reconciliation first and reject revoke/purge while any active/candidate/drain uses the ref |
| Caller identity spoofing | Strip internal headers and inject/verify signed invocation claims |
| Persistent model mutation | Managed API, clients, runner, reconciliation, and ingress reject it |
| WebSocket/SSE auth bypass | Authenticate before upgrade/stream and test disconnect/reconnect paths |
| Cross-caller state leak | Caller-scoped sessions, overlays, queues, affinity, logs, and quotas |
| Replay of claim/API key | Hashed token, audience binding, expiry, nonce, single use, revocation |
| DNS/domain takeover | Proof before bind, global host uniqueness, revoke route before release |
| Log or trace leakage | Metadata-only default, redaction, scoped access, retention and deletion |
| Resource exhaustion | Body, timeout, queue, concurrency, replica, storage, egress, and budget limits |
| SSRF, DNS rebinding, or egress bypass | Internal revision network, dedicated dual-homed gateway, whole-answer validation, IP pinning, exact SNI, no fallback |
| Partial runtime after child failure | PID 1 supervision terminates siblings and exits nonzero for runner reconciliation |
| Stale runtime after control failure | Desired/observed reconciliation, heartbeat freshness, explicit degraded state |

## Committed Implementation Baseline

- Deployment control routes authenticate Cloud sessions.
- Browser mutations require CSRF; bearer clients require valid bearer sessions.
- Account membership is verified server-side and creator identity is session-derived.
- Canonical ingress URLs include a stable deployment ID.
- Legacy slug-only lookup returns a route only for one globally unique ready match.
- Managed persistent patch controls are absent from the web UI.
- Cloud creation/start/restart and public ingress reject persistent patch metadata.
- OSS CLI/TUI Cloud deploy and reupload reject persistent patch packages before
  network access.
- Hosted runners reject unsafe packages before Docker and remove legacy unsafe
  containers during reconciliation.
- OSS exports immutable package v3 contracts. Cloud verifies the archive/contract,
  persists immutable release compatibility, probes the runtime image protocol,
  launches by immutable image ID, and does not offer incompatible jobs to a runner.
- The publication image pins and verifies all three provider CLIs. Hosted runner
  limits, read-only/isolation controls, scoped audit capability, and standalone
  child supervision are committed with focused tests.
- Public ingress strips forged internal headers, injects signed caller claims,
  and the runtime verifies caller affinity across HTTP, SSE, WebSocket, and MCP.
- Protocol 241, the bounded one-time kernel arm, the shared secret interaction,
  Cloud callback route/claim binding, callback worker, and both web and TUI
  arming are committed.
- OSS `b6c58cec2` enforces service-key proof of possession before
  decrypt/dispatch, with focused match, mismatch, and ordinary-client-unaffected
  tests. Those focused tests do not establish live callback E2E completion.
- The v3 egress ceiling and dedicated CONNECT gateway implementation are committed
  in OSS, including whole-answer address validation, IP pinning, and SNI equality.

These are implementation facts, not a production-hosting sign-off. The final
local web/TUI matrix verifies callback, gateway, legacy-hosted denial, transport,
identity, disruption, bounded-load, resource, visual, and cleanup behavior. Live
provider credentials, real DNS/TLS, and the hosted/remote/slice/collaborator
matrix remain open; the Hetzner managed-slice preflight stopped before mutation
because available disk was 203,464,704 bytes below its effective requirement.

## Residual Risks And Activation Gates

The following still block a production customer-hosting claim:

- refresh live Claude credentials and pass web- and TUI-initiated connect,
  cancel, timeout, replay, expiry, rotation, revocation, proof-of-possession, and
  cleanup through the committed protocol-241 callback path
- repeat the committed v3 egress snapshot, unsupported-bundle and
  obsolete unrestricted-package denials, revision-scoped internal network, dual-homed
  gateway, host enforcement, no-direct-route invariant, and
  start/restart/promotion/recovery/teardown behavior on Hetzner
- validate immutable package admission, caller claims, resource ceilings, child
  supervision, promotion/rollback, audience denial, custom domains, and all
  transports in the guarded hosted matrix; these cases already pass locally on
  the final committed OSS/Cloud revisions
- complete one live two-surface managed-slice transaction on Hetzner after the
  host has at least 3,489,660,928 bytes of available disk and passes the memory
  guard; do not stop or prune unrelated workloads to manufacture headroom
- pass designated Hetzner hosted-container, pinned remote-machine, and second-
  account collaborator cases from both web and TUI with inspected screenshots,
  TUI state evidence, stable source provenance, resource samples, and cleanup
- integration-secret enrollment through a direct runtime or external-vault
  adapter boundary without secret bytes entering Cloud
- decide whether production release authenticity requires a signing authority and,
  if so, implement key custody, verification, rotation, revocation, and provenance
- complete backup, retention, deletion, incident response, privacy, and recovery
  evidence, plus a final report naming only truly required external registrations

## Verification Matrix

Every security-sensitive change must include focused tests and an end-to-end drill
for each affected surface.

| Surface | Required verification |
| --- | --- |
| Cloud API | unauthenticated, CSRF, role, cross-account, stale session, and replay probes |
| Ingress | canonical route, duplicate slug, encoded path, headers, body, SSE, WebSocket, MCP |
| Package | malformed, traversal, symlink, bomb, digest mismatch, unsupported contract/capability |
| Package admission | v1/v2 hosted denial, v3 identity/digest mismatch, protocol floor, immutable image ID, unsupported provider bundle |
| Runner | start, restart, reconcile, stale backend, crash, unsafe package, resource ceilings, supervision, cleanup |
| Credential runner | wrong account/runner/version/attempt, lease expiry/reclaim, heartbeat, stale completion, enrollment expiry/replay, source traversal, rotation overlap, revoke-before-stop, restart-safe GC |
| Claude callback | wrong arming user/realm/subject/target/tuple/key, stale target, cancel, timeout, replay, callback redaction, web/TUI first reply |
| Hosted egress | legacy denial, empty/exact/tightened policy, mixed DNS answer, private/mapped IP, IP pin, SNI mismatch, port, direct route, gateway loss, no fallback |
| Runtime isolation | action UID cannot read sentinel credential, inspect protected process env, authenticate to kernel, or obtain runner key |
| Web terminal | create, configure, deploy, recover, inspect, stop, rollback, access denial |
| TUI/CLI | same lifecycle and denial semantics, including reupload and reconnect |
| Collaboration | owner, admin, member, invited customer, end user, and revoked actor |
| Remote | hosted container, pinned remote machine, live shared managed slice, and collaborator on Hetzner with both surfaces and resource samples |

Use this worktree's own relay, kernel, server/Cloud API, publication ingress,
unused probed ports, process groups, and state roots for local drills. Use the
designated Hetzner machine for hosted, remote-machine, managed-slice, and
collaboration drills. Drive every acceptance journey from both web and TUI and
inspect desktop/mobile screenshots and TUI state artifacts. Compare technical
failures with existing publication, resilience, provider, remote-agent, and slice
drills before inventing a parallel path.

Capture source revisions and before/during/after CPU, memory, disk, open files,
processes, containers, networks, volumes, and ports locally, on slices, and on
Hetzner. Enforce the fixed 1.25 GiB memory/3 GiB disk preflight and 1 GiB/2.25 GiB
runtime floors rather than relying on a stale host reading. Managed-slice
preparation currently needs an effective 3.25 GiB disk floor including transient
staging headroom. Remove only run-owned resources, execute cleanup and orphan
verification twice, and prove the baseline returns before accepting a case.

## Review Rules

- Any serialized protocol change follows the protocol version/hash/drill rule.
- Any new trust assumption must be explicit in this document and in tests.
- Any control implemented only in React is incomplete.
- Security denials must be consistent in web and TUI surfaces and must not leak
  cross-account object existence.
- Commit and push every meaningful green increment so test evidence maps to code.
- Dirty worktree behavior and artifacts without stable OSS/Cloud provenance cannot
  satisfy an activation gate.
- External vendors remain non-blocking when a local, temporary, or existing path
  can validate the boundary. List and request only truly required registrations
  in the final acceptance report.
