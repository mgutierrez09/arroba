# Optional Chrome session import

Status: feasibility and implementation requirements, not implemented.
Updated: 2026-09-06.

## User request

Offer to transfer selected Chrome sign-ins into a slice on first creation. This is
an addition to the saved-browser-state fix, not a replacement for persistence.
Import must not reintroduce unattended macOS Keychain prompts or require disabling
the Chromium renderer sandbox.

## What is feasible

A user-installed Chrome extension can read cookies through the documented
`chrome.cookies` API with cookie permission and host permission for selected
sites. The API exposes HttpOnly, Secure, SameSite, expiry, host scope and partition
metadata. It does not require Chariox to decrypt Chrome's cookie database itself.
See [Chrome cookies API](https://developer.chrome.com/docs/extensions/reference/api/cookies).

Request site access when the user chooses an import, not access to every site at
installation. Chrome supports optional host permissions and permission requests
at runtime. See [Chrome permission guidance](https://developer.chrome.com/docs/extensions/develop/concepts/declare-permissions).

This supports a proposed extension-mediated import. It is not proof that a
particular site's imported login will work. Cookie copying alone cannot carry a
device-bound session's private key. DBSC refresh requires proof from the original
device. Do not try to bypass this protection. See
[Chrome DBSC documentation](https://developer.chrome.com/docs/web-platform/device-bound-session-credentials).

Do not promise universal Google session portability. A site may require a new
login after import. Cookies also do not cover all origin storage or saved
passwords. The initial feature should import supported web sessions, not password
manager entries, passkeys, provider CLI credentials or whole Chrome profiles.

## Proposed first-creation flow

1. Offer "Import browser sign-ins" or "Start with a clean browser". Skipping it
   must never prevent Environment creation.
2. Pair the user-installed extension to the user's kernel through an authenticated
   channel. A web page must not be able to request arbitrary cookie exports.
3. Let the user choose the source Chrome profile, sites and target Room/Environment.
   Explain who can use that shared Environment. All authorized actors in that
   Environment may inherit the imported access, not only the initiating agent.
4. Show and approve the exact domains needed, including any identity-provider
   domains. Domain permission is not reliable per-account isolation: several
   accounts may share one browser session. Never claim a single account was
   imported when cookies grant access to more accounts.
5. Transfer the approved session once, end-to-end encrypted to the destination
   kernel, and install it into the Environment's browser using its controller.
   Cloud and relay remain transport/control-plane components, not credential
   readers. Keep secret values out of agent messages, logs, screenshots and
   ordinary action responses. Validate destination identity, authorization,
   request expiry, payload bounds and replay protection before applying anything.
6. Verify the site is usable without exposing cookie values. Report per-site
   success, unsupported state or "Sign in required" honestly. Preserve the source
   browser unchanged. Never bypass MFA, device binding or site security checks.
7. Discard transfer buffers and temporary artifacts. Future restarts restore the
   slice's own saved browser state. Never silently re-import stale host cookies
   over a newer slice session. Another import requires an explicit user action.

The kernel owns import admission and the target Environment operation. Web and
TUI clients must use that shared path. Any new serialized request needs the
protocol version/snapshot updates and focused drill required by AGENTS.md.
Reuse the existing authenticated kernel transport where suitable; do not expose
Chrome remote debugging publicly or launch the user's normal Chrome with weaker
security flags. A browser extension is an extra installation step, not something
the Chariox web app can assume is present.

## Validation before shipping

| Case | Required result |
| --- | --- |
| Extension absent, access denied, user skips | Clean Environment still starts; no prompt loop |
| Wrong Room/user/kernel, expired request, replay | Rejected before cookie access or destination mutation |
| Selected domain and unselected control domain | Only approved scope transfers; no suffix matching leaks |
| Multiple profiles, accounts, incognito | Explicit source; no hidden cross-profile or incognito import |
| HttpOnly, Secure, SameSite, host-only, path, expiry, partitioned cookies | Preserve supported semantics; explicitly reject unsupported combinations |
| Expired, server-revoked, device-bound or incomplete session | Honest reauthentication result; no security bypass |
| Transfer cancellation, disconnection, destination crash | Bounded cleanup and no partially published successful import |
| Concurrent actions, repeated import, existing destination login | Serialized operation; no silent overwrite or stale replay |
| Logs, errors, audit records, artifacts | No credential values; audit only approved non-secret metadata |
| Save, shutdown, full container/home-volume removal, restore | Imported supported login survives just like a native slice login |
| Browser crash, restart and URL-open fallback | Same profile and sandbox guarantees |
| Web, local TUI and remote TUI | Same kernel authorization and result, no client-specific authority |
| Real Google session | Explicit user-consented acceptance test; fixture success is not Google proof |

## Current evidence and remaining work

The repository already saves browser home state and has a live Docker browser
state drill. That fixture currently authenticates using a persistent HttpOnly
cookie. Session-cookie coverage and the reported Google shutdown loss need
separate verification. No Chrome extension importer was found in the inspected
runtime paths, and no real Chrome profile, cookies or Keychain item was read for
this research.

First fix and validate saved browser state and renderer sandboxing. Then build
the opt-in import with a deterministic two-profile fixture before touching real
accounts. Production import remains unimplemented until the matrix above and
the shared authorization/transport review pass.
