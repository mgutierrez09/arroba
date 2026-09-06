#!/usr/bin/env sh
set -eu
: >/tmp/chariox-desktop-settings-error
# Use the actual desktop's environment and the default settings backend.
# A same-process cache is insufficient: a second process must read the value.
if [ ! -e /tmp/chariox-desktop-read-only ]; then
  gsettings set org.xfce.mousepad.preferences.view show-line-numbers true 2>>/tmp/chariox-desktop-settings-error
fi
gsettings get org.xfce.mousepad.preferences.view show-line-numbers >/tmp/chariox-desktop-settings-result 2>>/tmp/chariox-desktop-settings-error
mkdir -p "$HOME/Documents"
mousepad --disable-server "$HOME/Documents/desktop-settings.txt" >/tmp/chariox-desktop-editor-log 2>&1 &
