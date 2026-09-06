# Secure browser launch defaults

Normal headed Chromium and URL recovery launches do not opt HTTP origins out of
secure-context restrictions. Leaving `CHARIOX_SLICE_CHROME_TRUSTED_INSECURE_ORIGINS`
unset or empty omits the Chromium override. An explicit nonempty value remains
a development-only opt-in and causes Chromium's unsupported-flag warning.

Use HTTPS or loopback access for local terminal development. The M20 fixture
uses a disposable loopback TCP bridge to its host server, started after each
slice boot, instead of making `host.docker.internal` a trusted HTTP origin.
The bridge has a connection cap and timeout and disappears with its container.

The browser-profile lifecycle drill checks the actual Chromium arguments with
unset configuration during seeding and empty configuration during restoration.
It checks sandbox layers and preserved authenticated storage during normal
start, restart, URL fallback, browser closure/crash recovery, and full container
and home-volume replacement. Its fixture uses ordinary loopback secure contexts.
The obsolete source-string test requiring the unsafe production default has
been replaced by these live behavioral assertions.

This does not suppress security warnings. It removes the default exception that
caused one. It does not promise that external services never require sign-in.
