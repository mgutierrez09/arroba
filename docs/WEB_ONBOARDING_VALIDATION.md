# Web onboarding validation

This drill extends the email-gated onboarding scenario to a production Web
observer while the OSS drill retains private vault interaction replies,
fixture passwords, leak scanning, and credential cleanup. It does not add a
provider execution path or change the kernel/relay protocol.

## Coordination boundary

`apps/cli/scripts/lib/web-onboarding-channel.mjs` implements the disposable
file coordination between the owning runner and the Web observer. The owner
validates the requested agent against the kernel before starting provider work.
The injected validator must check Room, slice, provider, model and account
profile. The channel itself checks the Room and Environment identities.

The owner publishes four ordered phases: mail login, registration,
confirmation email, and confirmation. Each phase uses a fixed screenshot
filename in the evidence root. The observer must verify the actual Web canvas,
then acknowledge that phase and run identity before the owner can continue.
Messages carry only identities, phase sequence and a match boolean. Credential
values and private callbacks never belong in this channel.

The coordination directory must be a fresh private disposable directory owned
by the drill lifecycle. Both participants are trusted local drill processes;
this is not an authentication mechanism for arbitrary local processes.
The channel rejects unknown fields, stale identities, wrong ordering,
symlinked messages/screenshots and oversized records. It bounds waits, supports
cancellation, and removes temporary publish files after failure.

Run its focused tests with:

```sh
node --test apps/cli/scripts/lib/web-onboarding-channel.test.mjs
node --test apps/cli/scripts/lib/live-room-web-onboarding.test.mjs
```

## Integration still required

`live-room-web-onboarding.mjs` now validates the selected agent through the
kernel before invoking the existing private onboarding runner. The runner
provides an optional phase observation callback after physical verification
and leak scanning. Focused tests exercise the kernel request and provider
submission boundaries, including fixture cleanup on failed submission.

These tests do not prove Web onboarding. The existing live entry point still
rejects Web onboarding. Remaining wiring must:

- connect the owner adapter to the existing live companion entry point;
- sample both TUIs during all provider turns and acknowledgement waits;
- compare the production Web canvas with each physical phase screenshot;
- reject changed targets, disconnected/stale frames and late Web errors;
- validate all four Web receipts against the owner's credential/action proof;
- run the official-provider workflow and verify complete cleanup.

Keep the Web revocation scenario disabled until its additional negative phase
has corresponding observer coverage. Controlled fixture acceptance will not
prove real Gmail, external SaaS, other providers, or managed deployment.
