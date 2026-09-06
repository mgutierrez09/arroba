# Provider accounts

Chariox supports multiple named account profiles for Codex, Claude, and OpenCode. The kernel owns profile metadata, selection, status/usage projection, and orchestration. Provider CLIs remain the credential-format and token-refresh authority.

## User workflow

Open **Provider Accounts** in either waiting room to list, create, link, rename, set a default, refresh, log in, log out, remove, or explicitly delete a managed profile. The launch form selects Provider, Account, Model, and Variant. New agents inherit the selected account unless another profile is chosen.

Changing an existing agent's account uses the same bounded context handoff used for provider/model changes. The active provider run ends, incompatible provider resume state is cleared, and a fresh run starts under the selected profile. Credentials are never hot-mutated in a running provider process.

The configured Cloud owner and local TUI share the home kernel's account registry. Collaborators retain separate namespaces and cannot list, use, or receive the host owner's profiles.

New managed Machines default to every authenticated, transferable profile discovered by the selected source kernel. The user may exclude profiles or disable account transfer. Before asking Cloud to create the Machine, the source kernel exports each selected profile into its provider-native portable credential shape. A missing or non-portable credential stops the launch before compute is rented. The managed-context plan still records an explicit canonical profile list; Cloud never receives credential contents.

## Provider roots

- Codex: every profile has a distinct `CODEX_HOME`. Managed profiles force `cli_auth_credentials_store = "file"`; `auth.json`, app-server processes, catalogs, usage, login, and logout are profile-scoped.
- Claude: managed and directory-linked profiles use an explicit `CLAUDE_CONFIG_DIR`. An effective native default registered without that variable preserves its absence. These are different credential scopes on macOS, even when the explicit directory is `$HOME/.claude`. Chariox invokes the official `claude auth login`, `logout`, and `status` commands in the selected scope.
- OpenCode: every profile has distinct data, config, state, cache, and `OPENCODE_CONFIG_DIR` roots. A profile may contain multiple upstream connections. Upstream usage is a capability matrix: local stats and supported native seams are projected, while unknown billing providers report unavailable rather than guessing or reading secrets for third-party APIs.

Existing effective default roots migrate once into the durable registry with stable profile IDs and public labels. `default` resolves the currently selected default; it is not a replacement for a stored profile ID. Static provider-profile configuration is not a second source of truth.

### Claude native login and legacy profiles

A successful native `claude auth status` does not prove that a directory-linked Chariox profile is signed in. First compare the selected profile's credential scope with the native invocation. Preserve the real macOS HOME. Do not copy credentials, log out, or start another login merely because the two status results differ.

Legacy registries did not record whether `$HOME/.claude` was an ambient default or an explicit `CLAUDE_CONFIG_DIR`. Migration preserves explicit scope for those ambiguous records rather than silently switching accounts. Refreshing status or choosing that profile as the default does not change its scope. Linking `$HOME/.claude` also creates an explicit directory scope, not an ambient-native account.

Use `/provider accounts import-native <provider>` to register the kernel host's current native scope explicitly. Import is idempotent for a scope already registered. When the native scope differs from an ambiguous legacy profile, it creates a separate stable profile and does not replace the old registration, change the default, start login, or copy provider files. Select or mark the imported profile as default only after its native status is refreshed. Do not edit a running kernel's registry file or remove/recreate profiles that existing agents depend on. Registration removal preserves provider files but does not preserve references to the removed profile ID.

## Workers and slices

The home kernel remains authoritative. When an agent is assigned to a trusted home-worker or home-managed slice, only its selected profile is materialized through the existing encrypted kernel-to-worker channel. Separate profiles use separate roots. Cloud and the relay receive only opaque encrypted packets and safe materialization status.

Materialization is denied before launch when the existing trust/ownership policy does not authorize credential transfer. A credential replica is refreshed by rematerializing from the home authority; it does not become an independent credential source. Chariox never reads or copies Claude credentials from macOS Keychain. Foreground local `/login` remains provider-owned. Unattended Claude profiles use a provider-supported `claude setup-token` credential stored in the Chariox encrypted vault and injected only into the official Claude CLI process as `CLAUDE_CODE_OAUTH_TOKEN`. Missing or locked credentials fail before launch without an OS dialog.

Model catalogs are cached by owner, selected profile, and execution location. Remote/slice selections must have a kernel-projected materialization record; clients never infer availability from labels.

OpenCode account transfer exports `data/opencode/auth.json` and the portable
configuration files `config`, `config.json`, `opencode.json`, `opencode.jsonc`,
`tui.json`, and `tui.jsonc` from the profile's global and custom configuration
directories. It does not traverse the data, state, or configuration trees.
Session databases, prompt history, snapshots, caches, locks, installed
`node_modules`, and capability packages are not account credentials and do not
belong in this transfer. Provider-native configuration continues to refer to
capabilities installed at the execution location; managed-slice capability
transfer uses its separate existing kernel path. Missing optional files are
allowed, but exported roots and files must be regular, non-symlink entries and
the existing 64 MiB total limit still applies. Managed-machine context export
remains the separate credential-only, 16 MiB path.

Refreshing an existing regular OpenCode replica updates only those portable
account files. It preserves worker-created history, databases, and directories
held open by a running provider. Portable files omitted by the home authority
are removed, so revoked credentials do not survive a refresh. If the registry
commit fails, the previous portable files are restored. On Unix, refresh and
rollback use held directory descriptors and no-follow reads so a concurrent
symlink replacement cannot redirect credential writes. Each file publication
is atomic; this is not an atomic multi-file provider-state snapshot.

This account transfer is not a history backup or provider-session migration.
Environment and slice saved-state acceptance must validate their own durable
provider state independently.

## Usage semantics

Usage meters identify their source, kind, unit, limits/balance where exposed, reset time, freshness, and availability. Missing numbers mean the provider did not expose them; they are not treated as zero. Codex subscription windows and credits use app-server methods. Claude uses provider-native rate-limit observations and an explicit official-CLI `/usage` refresh with tools disabled and session persistence disabled. The refresh accepts only structured results with all required model-activity fields present and zero; missing fields are not assumed to be zero. Chariox does not rewrite Claude's onboarding or trust settings for this probe. OpenCode billing remains best-effort and extensible per upstream provider.

## Security and deletion

Profile paths and credential values are private kernel state and never appear in waiting-room, Cloud, relay, or protocol projections. Ambient provider API-key variables are scrubbed from managed launches so a named profile cannot silently execute under unrelated environment credentials.

Logout, deregistration, and deletion reject profiles with active runs. Removing a profile keeps provider data. Deleting managed data is a separate operation requiring the exact profile ID and is unavailable for linked/default roots.

## Live validation

`apps/cli/scripts/live-multi-account-drill.mjs` is opt-in and accepts existing profile IDs. It never copies or prints credentials. OpenCode remaining-balance validation is intentionally pending until suitable accounts/upstreams are available; unsupported sources remain visible as unavailable capability states.
