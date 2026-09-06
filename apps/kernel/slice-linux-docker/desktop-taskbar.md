# Desktop applications bar

Headed slices use Openbox with a compact tint2 taskbar at the bottom of the
desktop. Each open application keeps a named button when minimized. Clicking
an inactive or minimized task restores it; clicking the active task minimizes it.
The bar reserves 34 pixels so maximized applications do not cover it.

Explicit Browser tab activation also restores a minimized Chromium window.
It uses the existing document-bound controller operation, not simulated taskbar
clicks. Navigation alone does not request desktop focus. Office workflows that
need the browser visible should explicitly activate their tab; reads and
background navigation must not take focus from a desktop application.

The desktop launcher owns the bar's startup, health check, and shutdown. Both
noVNC and Selkies display the same desktop, without browser-only restore logic.
New slice images install tint2. Support refresh installs it when absent from an
older saved image, before replacing the launcher. A failed dependency install
fails provisioning instead of reporting a healthy desktop without a taskbar.

Run `live-desktop-settings-drill.mjs` with `CHARIOX_DESKTOP_SETTINGS_IMAGE` set
to an existing slice image. It clicks the taskbar to restore minimized Chromium
and Mousepad, then checks editor contents and settings after desktop restart
and a saved-home restore into a replacement container. Its screenshots and
cleanup report remain outside the repository. This focused drill does not
replace production-image or full kernel-managed saved-state validation.
