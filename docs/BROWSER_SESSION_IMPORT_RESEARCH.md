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

## OSS implementation review

Reviewed on 2026-09-06. Browser import is established functionality and we should
reuse its interaction patterns and tested implementations where they fit. The
distinction below is between saved passwords and an already signed-in session,
not an objection to supporting import.

| Source and pinned revision | What the inspected code does | Reuse decision |
| --- | --- | --- |
| [Firefox ChromeProfileMigrator](https://github.com/mozilla-firefox/firefox/blob/ae0bb53a873d228e4e61b796e184dc3c687ff82e/browser/components/migration/ChromeProfileMigrator.sys.mjs) | Enumerates source profiles and available resources. Imports bookmarks, history, form data and extensions, plus passwords and payment methods when supported. Its `getResources` does not register cookie import. Individual resource discovery failures do not prevent other resources from being offered. | Reuse the profile/category selection and per-resource failure pattern. Do not describe its password importer as a live-session importer. |
| [Firefox macOS login crypto](https://github.com/mozilla-firefox/firefox/blob/ae0bb53a873d228e4e61b796e184dc3c687ff82e/browser/components/migration/ChromeMacOSLoginCrypto.sys.mjs) | Retrieves the source browser's password-encryption secret through macOS Keychain. The migrator handles cancellation of that access. | Do not port this path into unattended Chariox. It would reintroduce the OS authorization dependency the user rejected. |
| [Brave import coordinator](https://github.com/brave/brave-core/blob/a8ccd4875e747fb959ce733765b302a23c7ec479/browser/importer/brave_external_process_importer_host.cc) and [password importer](https://github.com/brave/brave-core/blob/a8ccd4875e747fb959ce733765b302a23c7ec479/browser/importer/brave_password_importer.cc) | Copies `Login Data`, reads it through Chromium's password database and OS decryptor, then submits credentials to the destination password store. The coordinator allows this password path only for Brave-to-Brave on macOS and Linux. It distinguishes an empty database from a read failure. | Reuse the failure distinctions and source-preserving approach, not the Chromium-internal dependency graph or OS decryptor. These are browser-integrated C++ components, not a standalone Chariox library. |
| [Cookie-Editor handler](https://github.com/Moustachauve/cookie-editor/blob/9f3f8fb6f7d94985009d612a9cbfd0e7f439d77d/interface/lib/genericCookieHandler.js) and [MV3 manifest](https://github.com/Moustachauve/cookie-editor/blob/9f3f8fb6f7d94985009d612a9cbfd0e7f439d77d/manifest.chrome.json) | Uses the browser cookies API, optional host permissions and browser-specific field normalization. `prepareCookie` preserves several common fields but does not copy `partitionKey` at this revision. | Useful working reference for cookie transfer, not a complete fidelity implementation. Do not copy its normalization unchanged. Its repository declares GPL-3.0; no code has been copied or license-compatibility decision made. |
| [Google MV3 cookie sample](https://github.com/GoogleChrome/chrome-extensions-samples/tree/6c3c302b349160c754138f0fd940f0f5e96ef614/api-samples/cookies/cookie-clearer) | Demonstrates a popup calling the browser cookies API. This particular example deletes cookies and requests all hosts at installation. | Reuse small MV3/API wiring examples where helpful. Do not carry over deletion or broad host access. The repository declares Apache-2.0, but file notices and any dependencies still need checking when taking code. |

Firefox and the inspected Brave files declare MPL-2.0 in their source headers.
Keep upstream revision, license and notices with any future copied or adapted
code. This investigation adds no third-party implementation or dependency.

Brave's current [user documentation](https://support.brave.app/hc/en-us/articles/360019782291-How-do-I-import-or-export-browsing-data)
also separates the standard import dialog from Chrome password import, which it
directs through explicit password export/import. That supports offering familiar
browser/profile/category choices, without implying every category has the same
transfer mechanism. No claim is made about the proprietary Codex browser's
import implementation.

### Recommended implementation order

Use the normal first-run interaction: choose a browser/profile, choose what to
import, choose the destination, then see the result. Keep "Browser sign-ins" and
"Saved passwords" separate. For the user's requested sign-ins, implement the
browser-mediated path first. Saved passwords, bookmarks and history can be
separate categories; they must not delay session transfer or appear as supported
before they are implemented.

1. Add a small MV3 connector using the documented cookies API and optional site
   permissions. The source profile is the profile running that connector, not
   arbitrary profiles on disk. Explain that boundary in the picker. An extension
   cannot silently read another Chrome profile just because it is installed in
   one profile.
2. Keep cookie values inside the connector, encrypted transfer and destination
   controller. Reuse Chariox's authenticated encrypted transport and kernel-owned
   authorization. Add the import request and progress/result projection to the
   shared protocol, not an independent web endpoint with its own authority.
3. Share one field validator and normalization contract across the transfer and
   destination adapter. Test host-only versus domain scope, session expiry,
   SameSite, Secure, HttpOnly, path and partition keys. Source browser store IDs
   identify the source store; they must not be blindly reused as destination IDs.
4. Use an explicit bounded transaction with a destination snapshot and rollback
   or an equivalent atomic publication mechanism. Cancellation, expiry or failure
   must not leave an import reported as complete. Do not clear the entire browser
   cookie store or import unrelated domains.
5. Run the two-profile fixture and security matrix below before offering the
   connector for real sign-ins. Then validate supported services with the user.

This is an implementation recommendation, not proof that the connector or its
authorization exists. The browser API supplies cookie access, not Chariox's
pairing, consent, encrypted routing, replay prevention or destination lifecycle.
Those product responsibilities remain to be implemented.

The [Chrome cookie API](https://developer.chrome.com/docs/extensions/reference/api/cookies)
documents permission, store and partition semantics. Browser-mediated access
avoids implementing Chrome's on-disk decryption inside Chariox. It does not turn
device-bound credentials into transferable credentials. Follow the site's normal
reauthentication when [DBSC](https://developer.chrome.com/docs/web-platform/device-bound-session-credentials)
or another service check rejects the copied session.

## Current evidence and remaining work

The repository saves browser home state and has a live Docker browser state
drill. Local validation now includes persistent and session cookies plus browser
storage through restart and full container/home-volume removal, recorded outside
Git in the browser-computer-use evidence directory. This proves fixture
persistence, not importing a Mac profile. The user-attended Google restore test
has restored a Gmail tab with Chromium namespace and seccomp sandbox checks
passing; confirmation that Google still accepts the login is pending. No cookie
values or inbox content were captured for that check.

No Chrome extension importer was found in the inspected runtime paths. No real
host Chrome profile, cookies or Keychain item was read for this research.

First fix and validate saved browser state and renderer sandboxing. Then build
the opt-in import with a deterministic two-profile fixture before touching real
accounts. Production import remains unimplemented until the matrix above and
the shared authorization/transport review pass.
