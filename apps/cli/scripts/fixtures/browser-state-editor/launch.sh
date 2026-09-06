#!/usr/bin/env sh
set -eu
exec 2>/tmp/chariox-browser-state-launch.log
if [ ! -e "$HOME/.config/chariox-browser-state-editor/verify" ]; then
  gsettings set org.xfce.mousepad.preferences.view show-line-numbers true
fi
gsettings get org.xfce.mousepad.preferences.view show-line-numbers >/tmp/chariox-browser-state-preference
gdbus call --session --dest org.a11y.Bus --object-path /org/a11y/bus \
  --method org.a11y.Bus.GetAddress >/tmp/chariox-browser-state-accessibility
exec mousepad --disable-server "$HOME/Documents/chariox-browser-state-editor.txt"
