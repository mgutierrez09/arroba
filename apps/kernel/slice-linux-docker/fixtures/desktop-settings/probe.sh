#!/usr/bin/env sh
set -eu
export GSETTINGS_SCHEMA_DIR=/tmp/chariox-desktop-schemas
: >/tmp/chariox-desktop-settings-error
# Use the actual desktop's environment and the default settings backend.
# A same-process cache is insufficient: a second process must read the value.
if [ ! -e /tmp/chariox-desktop-read-only ]; then
  gsettings set org.chariox.desktop-drill enabled true 2>>/tmp/chariox-desktop-settings-error
fi
gsettings get org.chariox.desktop-drill enabled >/tmp/chariox-desktop-settings-result 2>>/tmp/chariox-desktop-settings-error
