# Chromium renderer sandbox

Headed startup, URL-open fallback and the headless smoke command use Chromium's
renderer sandbox. None passes `--no-sandbox`. Browser profile persistence remains
independent of sandboxing: all headed launch paths retain the same profile and
password-store selection, and shutdown uses the existing browser close helper.

Normal Docker slices use `chromium-seccomp.json`, derived from
[moby/profiles default.json at 61eaf32614c7c71b60bd8927d3e6a4ffc8ff1f31](https://github.com/moby/profiles/blob/61eaf32614c7c71b60bd8927d3e6a4ffc8ff1f31/seccomp/default.json).
The original Apache 2.0 license is in `chromium-seccomp.LICENSE`.
Chariox adds one allow rule for `clone`, `setns` and `unshare`. All other rules and
the default-deny policy remain unchanged. The test pins the canonical upstream
JSON hash so an unrelated rule change cannot slip into this exception.

These namespace operations let non-root Chromium install its own isolation.
This follows [Playwright's documented Docker sandbox setup](https://playwright.dev/docs/docker).
It does not add SYS_ADMIN, disable Docker seccomp globally, or replace renderer
isolation with container isolation. The existing explicitly opted-in managed
provider/bubblewrap container mode is unchanged; its Chromium process must still
install the renderer sandbox.

Docker seccomp configuration is immutable for an existing container. New
containers, including ones restored from a saved home archive, receive this
profile. An old container created with Docker's default profile needs safe
container recreation with its saved state/home retained. Do not delete a user's
container/home to work around startup failures. Do not silently fall back to
`--no-sandbox` on hosts that restrict user namespaces through another policy.
Managed-host AppArmor/rootless configurations need their own live validation.

## Focused local validation

Run `node --test apps/kernel/slice-linux-docker/chromium-sandbox.test.mjs`
and the provisioner tests. For real headed Chromium, set
`CHARIOX_BROWSER_PROFILE_IMAGE` to an existing slice-compatible image, select the
intended Docker endpoint/config, and run:

```sh
node apps/kernel/slice-linux-docker/live-browser-profile-drill.mjs
```

The driver never builds/downloads an image. The image needs Node 22, Chromium,
Xvfb, Openbox, x11vnc, websockify/noVNC, xdotool, Python and zstd. Source must be
visible to the Docker daemon for a read-only mount. This focused fixture uses
the noVNC desktop backend only to exercise the shared Chromium lifecycle, not to
validate or select the product's streaming backend.

The drill verifies namespace/PID/network isolation and Seccomp-BPF through the
real `chrome://sandbox` page; persistent and session HttpOnly cookies; local
storage; and forced CDP navigation failure through the actual URL fallback.
It stops the actual desktop helper, archives the home, removes the entire
container and home volume, restores into a new volume and container, and checks
again. It cleans only its uniquely named containers, volume and temp archive.
It runs one browser with 768 MiB and one CPU. Output contains auth booleans, not
cookie payloads.

This is not the full kernel/Web/TUI/managed-host lifecycle drill. Passing it does
not prove Google authentication survives restoration. That reported issue stays
open until reproduced or verified using a real, user-authorized Google session.
