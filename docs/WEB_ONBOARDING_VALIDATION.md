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

## Live integration

`live-room-web-onboarding.mjs` now validates the selected agent through the
kernel before invoking the existing private onboarding runner. The runner
provides an optional phase observation callback after physical verification
and leak scanning. Focused tests exercise the kernel request and provider
submission boundaries, including fixture cleanup on failed submission.

The live companion now connects the owner adapter and samples both TUIs during
provider turns and acknowledgement waits. Final verification checks all four
Web receipts against the owner's physical screenshots and credential/action
proof, including independent kernel history and both TUI observations.

Select `CHARIOX_ROOM_DRILL_FOCUS=web-companion`,
`CHARIOX_ROOM_DRILL_WEB_REAL_PROVIDER=1`,
`CHARIOX_ROOM_DRILL_PROVIDER_MODE=browser`, and
`CHARIOX_ROOM_DRILL_OFFICE_SCENARIO=onboarding` through the paired Cloud local
Room drill. Explicitly select the official provider and model. Use the standard
initial Browser click, not an unrelated form/layout/recovery combination.

The paired Cloud observer must compare the production Web canvas with each
physical screenshot and reject changed targets, stale frames and late Web
errors. These focused tests do not prove live official-provider acceptance;
that still requires a full run and cleanup evidence at the paired commits.

Keep the Web revocation scenario disabled until its additional negative phase
has corresponding observer coverage. Controlled fixture acceptance will not
prove real Gmail, external SaaS, other providers, or managed deployment.
