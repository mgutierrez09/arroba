# Desktop settings lifecycle drill

Run `live-desktop-settings-drill.mjs` with `CHARIOX_DESKTOP_SETTINGS_IMAGE` pointing
to an existing slice image. Set the normal Docker endpoint/configuration for the
test host. The script mounts this checkout read-only, runs the production
`slice-screen.sh`, and installs Mousepad 0.5.10-2 and `libglib2.0-bin` in its disposable container.
It needs package-network access but does not build an image or Rust binary.

The test launches a GSettings client and Mousepad from the real Openbox menu. It
uses Mousepad's installed line-number preference and the default settings backend,
inheriting the desktop environment. A second process must read the saved value,
with no write warning. Chariox Computer keyboard helpers enter and save a Unicode
document in the real editor. Subsequent launches must display the exact document,
verified through clipboard contents and fresh screenshot OCR before capturing
evidence. A mapped/focused window alone cannot pass visual readiness. Screenshots
remain under `~/.codex/evidence/browser-computer-use/desktop-settings/`.
After desktop shutdown, no live
Openbox, session supervisor, D-Bus daemon or dconf service may remain. A new
desktop launch must read the saved value without writing it again.

Next the test archives the home directory after desktop shutdown, deletes the
container and creates a fresh one. It checks the fresh default value before
extracting the archive, then requires a desktop-launched reader to retrieve the
saved setting without rewriting it. The temporary archive lives under a unique
directory in `~/.chariox/dev/browser-computer-use/`, which cleanup removes even
if container cleanup fails. Fixture tools are installed again in the fresh
container; this is settings restoration, not installed-package persistence.

The temporary fixture menu exists only in this test container. No host
browser profile, provider account, cookie or Keychain access is involved. Docker
limits the test to 768 MiB, one CPU and 1024 tasks, with no additional swap or
published ports. SIGINT/SIGTERM use the shared drill interruption lifecycle and
remove the exact owned container after in-flight work returns.

This test failed before the fix with `false !== true` after a desktop-launched
settings write. Running Openbox through `dbus-run-session` makes the same test
pass. The [D-Bus supervisor](https://dbus.freedesktop.org/doc/dbus-run-session.1.html)
gives child programs a session address and terminates the bus when Openbox exits.
Shutdown targets Openbox itself so the supervisor can perform that cleanup.

This is desktop startup, settings-write, restart and home-archive restoration
evidence. It is not a full kernel save/archive/restore run, a graphical application's complete settings
suite, managed-image acceptance or the provider editor-to-email drill. Those
acceptance cases remain in the end-to-end plan. In particular, the older editor
persistence drill's explicit keyfile backend is not evidence of default dconf
behavior.
