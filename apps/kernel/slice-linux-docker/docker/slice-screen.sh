#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="${CHARIOX_SLICE_ROOT:-/opt/chariox-slice}"
LOGS="$ROOT/logs"
DISPLAY_ID="${CHARIOX_SLICE_DISPLAY:-:99}"
DISPLAY_MODE="${CHARIOX_SLICE_DISPLAY_MODE:-unknown}"
SCREEN_GEOMETRY="${CHARIOX_SLICE_SCREEN_GEOMETRY:-1280x800x24}"
SCREEN_SIZE="${SCREEN_GEOMETRY%x*}"
VNC_PORT="${CHARIOX_SLICE_VNC_PORT:-5900}"
NOVNC_PORT="${CHARIOX_SLICE_NOVNC_PORT:-6080}"
VIEWER_BACKEND="${CHARIOX_SLICE_VIEWER_BACKEND:-novnc}"
CHROME_URL="${CHARIOX_SLICE_CHROME_URL:-about:blank}"
CHROME_PROFILE="${CHARIOX_SLICE_CHROME_PROFILE:-$HOME/.chariox/browser/chromium}"
# Development-only opt-in. Production and explicitly empty configuration must
# retain Chromium's normal secure-context restrictions.
CHROME_TRUSTED_INSECURE_ORIGINS="${CHARIOX_SLICE_CHROME_TRUSTED_INSECURE_ORIGINS:-}"

export DISPLAY="$DISPLAY_ID"

case "$VIEWER_BACKEND" in
  novnc|selkies) ;;
  *) printf 'Unsupported slice viewer backend: %s\n' "$VIEWER_BACKEND" >&2; exit 2 ;;
esac

slice_selkies() {
  /opt/chariox-selkies/bin/python "$ROOT/slice-selkies.py" "$@"
}

mkdir -p "$LOGS" "$CHROME_PROFILE"

log() {
  printf '[slice-screen] %s\n' "$*" >&2
}

run_xdotool() {
  timeout --foreground 10s xdotool "$@"
}

run_xdotool_utf8() {
  LC_ALL=C.UTF-8 run_xdotool "$@"
}

start_process() {
  local name="$1"
  shift
  if pgrep -af "$name" >/dev/null; then
    return
  fi
  nohup "$@" >"$LOGS/$name.log" 2>&1 &
}

wait_for_display() {
  local attempt
  for attempt in $(seq 1 50); do
    if xdpyinfo -display "$DISPLAY_ID" >/dev/null 2>&1; then
      return
    fi
    sleep 0.1
  done
  log "X display $DISPLAY_ID did not become ready"
  tail -n 40 "$LOGS/xvfb.log" >&2 || true
  return 1
}

require_process() {
  local pattern="$1"
  local label="$2"
  local log_path="$3"
  if pgrep -af "$pattern" | grep -v defunct >/dev/null; then
    return 0
  fi
  log "$label did not stay running"
  tail -n 40 "$log_path" >&2 || true
  return 1
}

process_running() {
  pgrep -af "$1" | grep -v defunct >/dev/null
}

stop_process_pattern() {
  local pattern="$1"
  local attempt
  pkill -TERM -f "$pattern" >/dev/null 2>&1 || true
  for attempt in $(seq 1 30); do
    if ! process_running "$pattern"; then
      return 0
    fi
    sleep 0.1
  done
  pkill -KILL -f "$pattern" >/dev/null 2>&1 || true
  for attempt in $(seq 1 20); do
    if ! process_running "$pattern"; then
      return 0
    fi
    sleep 0.1
  done
}

running_novnc_port() {
  pgrep -af "websockify.*127\\.0\\.0\\.1:$VNC_PORT" \
    | grep -v defunct \
    | awk '{
        for (i = 1; i <= NF; i++) {
          if ($i ~ /^0\.0\.0\.0:[0-9]+$/) {
            sub(/^0\.0\.0\.0:/, "", $i)
            print $i
            exit
          }
        }
      }' \
    | head -n 1
}

novnc_running() {
  if process_running "websockify.*$NOVNC_PORT"; then
    return 0
  fi
  [[ -n "$(running_novnc_port)" ]]
}

clear_chromium_profile_locks() {
  if [[ -d "$CHROME_PROFILE" ]]; then
    find "$CHROME_PROFILE" -maxdepth 1 \
      \( -name 'Singleton*' -o -name 'LOCK' -o -name 'lockfile' \) \
      -exec rm -rf {} + >/dev/null 2>&1 || true
  fi
}

chromium_has_restorable_session() {
  local default_profile="$CHROME_PROFILE/Default"
  if [[ -d "$default_profile/Sessions" ]] \
    && find "$default_profile/Sessions" -maxdepth 1 -type f -name 'Session_*' -size +0c -print -quit \
      | grep -q .; then
    return 0
  fi
  [[ -s "$default_profile/Last Session" || -s "$default_profile/Current Session" ]]
}

configure_chromium_profile_preferences() {
  python3 - "$CHROME_PROFILE" <<'PY' >/dev/null 2>&1 || true
import json
import os
import sys

profile = sys.argv[1]
default_dir = os.path.join(profile, "Default")
os.makedirs(default_dir, exist_ok=True)
path = os.path.join(default_dir, "Preferences")
try:
    with open(path, "r", encoding="utf-8") as handle:
        prefs = json.load(handle)
except Exception:
    prefs = {}

signin = prefs.setdefault("signin", {})
signin["allowed"] = False
prefs.setdefault("sync", {})["requested"] = False
prefs["credentials_enable_service"] = False
profile_prefs = prefs.setdefault("profile", {})
profile_prefs["password_manager_enabled"] = False
profile_prefs["password_manager_leak_detection"] = False

tmp = f"{path}.tmp"
with open(tmp, "w", encoding="utf-8") as handle:
    json.dump(prefs, handle, separators=(",", ":"))
os.replace(tmp, path)
PY
}

screen_missing_components() {
  local missing=()
  if ! xdpyinfo -display "$DISPLAY_ID" >/dev/null 2>&1; then
    missing+=("display")
  fi
  if ! process_running "Xvfb $DISPLAY_ID"; then
    missing+=("xvfb")
  fi
  if ! process_running '(^|/)tint2([[:space:]]|$)'; then
    missing+=("taskbar")
  fi
  if [[ "$VIEWER_BACKEND" == "selkies" ]]; then
    if ! slice_selkies status >/dev/null; then
      missing+=("selkies")
    fi
  else
    if ! process_running "x11vnc.*$DISPLAY_ID"; then
      missing+=("x11vnc")
    fi
    if ! novnc_running; then
      missing+=("novnc")
    fi
  fi
  if ! process_running "chromium.*$CHROME_PROFILE"; then
    missing+=("chromium")
  fi
  if [[ "${#missing[@]}" -eq 0 ]]; then
    return 0
  fi
  printf '%s\n' "${missing[@]}"
}

tool_blocking_missing_components() {
  local missing=()
  if ! xdpyinfo -display "$DISPLAY_ID" >/dev/null 2>&1; then
    missing+=("display")
  fi
  if ! process_running "Xvfb $DISPLAY_ID"; then
    missing+=("xvfb")
  fi
  if ! process_running "chromium.*$CHROME_PROFILE"; then
    missing+=("chromium")
  fi
  if [[ "${#missing[@]}" -eq 0 ]]; then
    return 0
  fi
  printf '%s\n' "${missing[@]}"
}

join_by_comma() {
  local IFS=,
  printf '%s' "$*"
}

require_screen_available() {
  local missing
  missing="$(tool_blocking_missing_components)"
  if [[ -z "$missing" ]]; then
    return 0
  fi
  status
  return 1
}

launch_chromium() {
  local -a chrome_secure_context_args=()
  local -a chrome_startup_target_args=()
  if [[ -n "$CHROME_TRUSTED_INSECURE_ORIGINS" ]]; then
    chrome_secure_context_args+=(
      "--unsafely-treat-insecure-origin-as-secure=$CHROME_TRUSTED_INSECURE_ORIGINS"
    )
  fi
  if ! process_running "chromium.*$CHROME_PROFILE"; then
    clear_chromium_profile_locks
    configure_chromium_profile_preferences
    if chromium_has_restorable_session; then
      chrome_startup_target_args+=(--restore-last-session)
    fi
  fi
  if [[ "$#" -gt 0 ]]; then
    chrome_startup_target_args+=(--new-window -- "$@")
  elif [[ "${#chrome_startup_target_args[@]}" -eq 0 ]]; then
    chrome_startup_target_args=(-- "$CHROME_URL")
  fi

  nohup chromium \
    --user-data-dir="$CHROME_PROFILE" \
    --password-store=basic \
    --no-first-run \
    --no-default-browser-check \
    --disable-sync \
    --disable-dev-shm-usage \
    --disable-gpu \
    --remote-debugging-address=127.0.0.1 \
    --remote-debugging-port=9222 \
    "${chrome_secure_context_args[@]}" \
    "${chrome_startup_target_args[@]}" >>"$LOGS/chromium-gui.log" 2>&1 &
}

start_desktop() {
  if process_running "chromium.*$CHROME_PROFILE" || process_running "Xvfb $DISPLAY_ID" || process_running "x11vnc.*$DISPLAY_ID" || novnc_running; then
    stop_desktop || true
  fi
  # Stop an owned previous Selkies process even when switching to noVNC.
  if [[ -x /opt/chariox-selkies/bin/python ]]; then
    slice_selkies stop --allow-forced >/dev/null
  fi
  stop_process_pattern "websockify.*127\\.0\\.0\\.1:$VNC_PORT"
  stop_process_pattern "websockify.*$NOVNC_PORT"
  stop_process_pattern "x11vnc.*$DISPLAY_ID"
  stop_process_pattern "x11vnc.*$VNC_PORT"
  stop_process_pattern '(^|/)openbox([[:space:]]|$)'
  stop_process_pattern '(^|/)tint2([[:space:]]|$)'
  stop_process_pattern "chromium.*$CHROME_PROFILE"
  stop_process_pattern "/usr/lib/chromium/chromium"
  stop_process_pattern "Xvfb $DISPLAY_ID"
  rm -f "/tmp/.X${DISPLAY_ID#:}-lock" "/tmp/.X11-unix/X${DISPLAY_ID#:}"

  nohup Xvfb "$DISPLAY_ID" -screen 0 "$SCREEN_GEOMETRY" -ac +extension RANDR +extension XTEST >"$LOGS/xvfb.log" 2>&1 &
  wait_for_display

  # Desktop-launched programs inherit one session bus. Without it, ordinary
  # GTK applications cannot persist dconf settings. The supervisor stops the
  # bus when Openbox exits, so stop only Openbox rather than killing both.
  nohup dbus-run-session -- openbox >"$LOGS/openbox.log" 2>&1 &
  nohup tint2 -c "$ROOT/tint2rc" >"$LOGS/taskbar.log" 2>&1 &
  if [[ "$VIEWER_BACKEND" == "selkies" ]]; then
    if ! slice_selkies start >/dev/null; then
      stop_desktop
      return 1
    fi
  else
    nohup x11vnc -display "$DISPLAY_ID" -localhost -nopw -forever -shared -rfbport "$VNC_PORT" >"$LOGS/x11vnc.log" 2>&1 &
    nohup websockify --web=/usr/share/novnc/ "0.0.0.0:$NOVNC_PORT" "127.0.0.1:$VNC_PORT" >"$LOGS/novnc.log" 2>&1 &
  fi

  launch_chromium

  sleep 2
  require_process "Xvfb $DISPLAY_ID" "Xvfb" "$LOGS/xvfb.log"
  require_process '(^|/)openbox([[:space:]]|$)' "Openbox" "$LOGS/openbox.log"
  require_process '(^|/)tint2([[:space:]]|$)' "Applications taskbar" "$LOGS/taskbar.log"
  if [[ "$VIEWER_BACKEND" == "selkies" ]]; then
    slice_selkies status >/dev/null
  else
    require_process "x11vnc.*$DISPLAY_ID" "x11vnc" "$LOGS/x11vnc.log"
    require_process "websockify.*$NOVNC_PORT" "noVNC websockify" "$LOGS/novnc.log"
  fi
  require_process "chromium.*$CHROME_PROFILE" "Chromium" "$LOGS/chromium-gui.log"
  status
}

status() {
  local missing
  missing="$(screen_missing_components)"
  printf 'display=%s\n' "$DISPLAY_ID"
  printf 'screen=%s\n' "$SCREEN_SIZE"
  printf 'mode=%s\n' "$DISPLAY_MODE"
  if [[ -z "$missing" ]]; then
    printf 'available=true\n'
    if [[ "$VIEWER_BACKEND" == "selkies" ]]; then
      printf 'viewer=http://127.0.0.1:%s/\n' "$NOVNC_PORT"
      return 0
    fi
    local viewer_port="$NOVNC_PORT"
    local discovered_port
    discovered_port="$(running_novnc_port)"
    if [[ -n "$discovered_port" ]]; then
      viewer_port="$discovered_port"
    fi
    printf 'viewer=http://127.0.0.1:%s/vnc.html?host=127.0.0.1&port=%s&autoconnect=true&resize=scale\n' "$viewer_port" "$viewer_port"
    pgrep -af "Xvfb $DISPLAY_ID|openbox|x11vnc|websockify|chromium.*$CHROME_PROFILE" | grep -v defunct || true
    return 0
  fi
  local missing_csv
  missing_csv="$(join_by_comma $missing)"
  printf 'available=false\n'
  printf 'missing=%s\n' "$missing_csv"
  printf 'message=slice screen is unavailable; missing %s\n' "$missing_csv"
  return 1
}

stop_desktop() {
  local streamer_exit=0
  if [[ -x /opt/chariox-selkies/bin/python ]]; then
    slice_selkies stop >/dev/null || streamer_exit=$?
  fi
  if process_running "chromium.*$CHROME_PROFILE"; then
    node "$ROOT/browser-cdp.mjs" close-browser >/dev/null 2>&1 || true
  fi
  local attempt
  for attempt in $(seq 1 80); do
    if ! process_running "chromium.*$CHROME_PROFILE"; then
      break
    fi
    sleep 0.1
  done
  pkill -TERM -f "chromium.*$CHROME_PROFILE" >/dev/null 2>&1 || true
  for attempt in $(seq 1 30); do
    if ! process_running "chromium.*$CHROME_PROFILE"; then
      break
    fi
    sleep 0.1
  done
  stop_process_pattern "chromium.*$CHROME_PROFILE"
  stop_process_pattern "/usr/lib/chromium/chromium"
  stop_process_pattern "websockify.*127\\.0\\.0\\.1:$VNC_PORT"
  stop_process_pattern "websockify.*$NOVNC_PORT"
  stop_process_pattern "x11vnc.*$DISPLAY_ID"
  stop_process_pattern "x11vnc.*$VNC_PORT"
  stop_process_pattern '(^|/)openbox([[:space:]]|$)'
  stop_process_pattern '(^|/)tint2([[:space:]]|$)'
  stop_process_pattern "Xvfb $DISPLAY_ID"
  clear_chromium_profile_locks
  return "$streamer_exit"
}

screenshot() {
  require_screen_available
  local path="${1:-/tmp/chariox-slice-screenshot.png}"
  scrot -z "$path"
  printf '%s\n' "$path"
}

focus_chromium() {
  local window
  window="$(run_xdotool search --onlyvisible --class chromium 2>/dev/null | tail -n 1 || true)"
  if [[ -n "$window" ]]; then
    run_xdotool windowactivate --sync "$window" >/dev/null 2>&1 || true
    run_xdotool windowfocus --sync "$window" >/dev/null 2>&1 || true
    sleep 0.1
  fi
}

click() {
  require_screen_available
  focus_chromium
  run_xdotool mousemove "$1" "$2" click 1
}

double_click() {
  require_screen_available
  focus_chromium
  run_xdotool mousemove "$1" "$2" click --repeat 2 --delay 80 1
}

pointer_click() {
  require_screen_available
  local x="$1"
  local y="$2"
  local button_name="$3"
  local click_count="$4"
  local button
  case "$button_name" in
    left) button=1 ;;
    middle) button=2 ;;
    right) button=3 ;;
    *) printf 'pointer button must be left, middle, or right\n' >&2; return 2 ;;
  esac
  case "$click_count" in
    1|2) ;;
    *) printf 'pointer click count must be 1 or 2\n' >&2; return 2 ;;
  esac
  run_xdotool mousemove "$x" "$y" click --repeat "$click_count" --delay 80 "$button"
}

pointer_drag() {
  require_screen_available
  local from_x="$1"
  local from_y="$2"
  local to_x="$3"
  local to_y="$4"
  local button_name="$5"
  local button
  case "$button_name" in
    left) button=1 ;;
    middle) button=2 ;;
    right) button=3 ;;
    *) printf 'pointer button must be left, middle, or right\n' >&2; return 2 ;;
  esac
  run_xdotool mousemove "$from_x" "$from_y" mousedown "$button" mousemove --sync "$to_x" "$to_y" mouseup "$button"
}

pointer_scroll() {
  require_screen_available
  local x="$1"
  local y="$2"
  local horizontal_steps="$3"
  local vertical_steps="$4"
  if [[ ! "$horizontal_steps" =~ ^-?[0-9]+$ || ! "$vertical_steps" =~ ^-?[0-9]+$ ]]; then
    printf 'pointer scroll steps must be integers\n' >&2
    return 2
  fi
  if (( horizontal_steps == 0 && vertical_steps == 0 \
    || horizontal_steps < -120 || horizontal_steps > 120 \
    || vertical_steps < -120 || vertical_steps > 120 )); then
    printf 'pointer scroll steps must be nonzero and between -120 and 120\n' >&2
    return 2
  fi
  run_xdotool mousemove "$x" "$y"
  if (( horizontal_steps < 0 )); then
    run_xdotool click --repeat "$((-horizontal_steps))" --delay 20 6
  elif (( horizontal_steps > 0 )); then
    run_xdotool click --repeat "$horizontal_steps" --delay 20 7
  fi
  if (( vertical_steps < 0 )); then
    run_xdotool click --repeat "$((-vertical_steps))" --delay 20 4
  elif (( vertical_steps > 0 )); then
    run_xdotool click --repeat "$vertical_steps" --delay 20 5
  fi
}

drag() {
  require_screen_available
  focus_chromium
  run_xdotool mousemove "$1" "$2" mousedown 1 mousemove --sync "$3" "$4" mouseup 1
}

move_mouse() {
  require_screen_available
  run_xdotool mousemove "$1" "$2"
}

scroll() {
  require_screen_available
  local amount="${1:-1}"
  local button=5
  if [[ "$amount" =~ ^- ]]; then
    button=4
    amount="${amount#-}"
  fi
  local i
  for i in $(seq 1 "$amount"); do
    run_xdotool click "$button"
  done
}

type_text() {
  require_screen_available
  focus_chromium
  run_xdotool_utf8 type --clearmodifiers --delay 5 "$*"
}

computer_type_stdin() {
  require_screen_available
  # Kernel enforces a length-derived deadline and immediate cancellation.
  # Standalone safety bound accommodates the full 64 KiB input contract.
  timeout --foreground --kill-after=1s 2h /opt/chariox-selkies/bin/python \
    "${BASH_SOURCE[0]%/*}/slice-keyboard.py"
}

computer_key_stdin() {
  require_screen_available
  local repeat="$1"
  local key=""
  IFS= read -r key || [[ -n "$key" ]]
  if [[ ! "$repeat" =~ ^[0-9]+$ ]] || (( repeat < 1 || repeat > 32 )); then
    printf 'computer key repeat must be between 1 and 32\n' >&2
    return 2
  fi
  if [[ -z "$key" || ${#key} -gt 128 || "$key" == -* || "$key" =~ [[:space:]] ]]; then
    printf 'computer key must be a non-whitespace xdotool key name of at most 128 bytes\n' >&2
    return 2
  fi
  run_xdotool key --clearmodifiers --repeat "$repeat" --delay 40 "$key"
}

computer_input_reset() {
  require_screen_available
  timeout --foreground 10s /opt/chariox-selkies/bin/python \
    "${BASH_SOURCE[0]%/*}/slice-keyboard.py" reset
}

key() {
  require_screen_available
  focus_chromium
  run_xdotool key --clearmodifiers "$1"
}

clipboard_get() {
  require_screen_available
  xclip -selection clipboard -out 2>/dev/null || true
}

CLIPBOARD_OWNER_PID=""

put_clipboard_once() {
  local value="$1"
  printf '%s' "$value" | xclip -selection clipboard -in -loops 1 >/dev/null 2>&1 &
  CLIPBOARD_OWNER_PID="$!"
}

clipboard_set() {
  require_screen_available
  put_clipboard_once "$*"
}

clipboard_clear() {
  require_screen_available
  put_clipboard_once ''
}

require_clipboard_available() {
  require_screen_available
  if command -v xclip >/dev/null 2>&1; then
    return 0
  fi
  log "xclip is not available"
  return 1
}

computer_clipboard_write_stdin() (
  require_clipboard_available
  local input_path
  input_path="$(mktemp "${TMPDIR:-/tmp}/chariox-computer-clipboard.XXXXXX")"
  trap 'rm -f "$input_path"' EXIT
  cat >"$input_path"
  xclip -selection clipboard -in <"$input_path" >/dev/null 2>&1
)

computer_clipboard_read() {
  require_clipboard_available
  xclip -selection clipboard -out 2>/dev/null || true
}

paste_stdin() {
  require_screen_available
  local input
  input="$(cat)"
  if printf '%s' "$input" | node "$ROOT/browser-cdp.mjs" type-stdin >/dev/null 2>&1; then
    return 0
  fi
  focus_chromium
  printf '%s' "$input" | run_xdotool_utf8 type --clearmodifiers --delay 5 --file -
}

secret_paste_stdin() {
  require_screen_available
  local selector="${1:-}"
  if [[ -n "$selector" ]]; then
    node "$ROOT/browser-cdp.mjs" secret-paste-stdin "$selector"
    return
  fi
  node "$ROOT/browser-cdp.mjs" secret-paste-stdin
}

secret_paste_submit_stdin() {
  require_screen_available
  local selector="${1:-}"
  if [[ -n "$selector" ]]; then
    node "$ROOT/browser-cdp.mjs" secret-paste-submit-stdin "$selector"
    return
  fi
  node "$ROOT/browser-cdp.mjs" secret-paste-submit-stdin
}

computer_secret_paste_stdin() {
  require_screen_available
  run_xdotool_utf8 type --clearmodifiers --delay 5 --file -
}

browser_status() {
  require_screen_available
  run_browser_cdp status
}

run_browser_cdp() {
  timeout --kill-after=1s 30s node "$ROOT/browser-cdp.mjs" "$@"
}

run_browser_cdp_wait() {
  local requested_ms="$1"
  shift
  if [[ ! "$requested_ms" =~ ^[0-9]+$ ]]; then
    requested_ms=10000
  fi
  local deadline_seconds=$(( (requested_ms + 999) / 1000 + 2 ))
  timeout --kill-after=1s "${deadline_seconds}s" node "$ROOT/browser-cdp.mjs" "$@"
}

browser_find() {
  require_screen_available
  local query="$1"
  local kind="${2:-any}"
  run_browser_cdp find "$query" "$kind"
}

browser_fill() {
  require_screen_available
  local selector="$1"
  shift
  run_browser_cdp fill "$selector" "$*"
}

browser_click() {
  require_screen_available
  run_browser_cdp click-selector "$1"
}

browser_submit() {
  require_screen_available
  run_browser_cdp submit "${1:-}"
}

browser_dialog() {
  require_screen_available
  local action="$1"
  local status
  case "$action" in
    accept|dismiss) ;;
    *)
      printf 'dialog action must be accept or dismiss\n' >&2
      return 2
      ;;
  esac
  set +e
  timeout --kill-after=1s 6s node "$ROOT/browser-cdp.mjs" dialog "$action" "${2:-}"
  status=$?
  set -e
  if [[ "$status" -eq 0 ]]; then
    return 0
  fi
  focus_chromium
  case "$action" in
    accept) run_xdotool key --clearmodifiers Return ;;
    dismiss) run_xdotool key --clearmodifiers Escape ;;
  esac
  sleep 0.3
  printf '{"ok":true,"action":"%s","fallback":"xdotool","cdp_status":%s}' "$action" "$status"
}

browser_text() {
  require_screen_available
  run_browser_cdp text
}

browser_wait_text() {
  require_screen_available
  local requested_ms="${2:-10000}"
  run_browser_cdp_wait "$requested_ms" wait-text "$1" "$requested_ms"
}

browser_wait_selector() {
  require_screen_available
  local requested_ms="${2:-10000}"
  run_browser_cdp_wait "$requested_ms" wait-selector "$1" "$requested_ms"
}

browser_wait_idle() {
  require_screen_available
  local requested_ms="${1:-10000}"
  run_browser_cdp_wait "$requested_ms" wait-idle "$requested_ms"
}

ocr() {
  require_screen_available
  local image="${1:-/tmp/chariox-slice-screenshot.png}"
  if [[ ! -f "$image" ]]; then
    screenshot "$image" >/dev/null
  fi
  tesseract "$image" stdout -l eng 2>/dev/null
}

find_text() {
  require_screen_available
  local query="$1"
  local image="${2:-/tmp/chariox-slice-screenshot.png}"
  local tsv="/tmp/chariox-slice-ocr.tsv"
  if [[ ! -f "$image" ]]; then
    screenshot "$image" >/dev/null
  fi
  tesseract "$image" stdout -l eng tsv >"$tsv" 2>/dev/null
  python3 "$ROOT/slice-text-finder.py" "$query" "$tsv"
}

open_url() {
  # Opening a URL may recover the browser, but must not restart or create the
  # shared desktop. Other input operations still require the full screen state.
  if ! xdpyinfo -display "$DISPLAY_ID" >/dev/null 2>&1 || ! process_running "Xvfb $DISPLAY_ID"; then
    status
    return 1
  fi
  if process_running "chromium.*$CHROME_PROFILE" && run_browser_cdp navigate "$1" >/dev/null 2>&1; then
    sleep 1
    focus_chromium
    return 0
  fi
  launch_chromium "$1"
  sleep 2
  require_process "chromium.*$CHROME_PROFILE" "Chromium" "$LOGS/chromium-gui.log"
  focus_chromium
}

cleanup_failed_start() {
  local startup_status=$?
  trap - EXIT
  if [[ "$startup_status" -ne 0 ]]; then
    stop_desktop || true
  fi
  exit "$startup_status"
}

case "${1:-status}" in
  start)
    # Keep errexit active inside start_desktop. An `if start_desktop` wrapper
    # would suppress failures inside the function and could report success.
    trap cleanup_failed_start EXIT
    start_desktop
    ;;
  stop) stop_desktop ;;
  status) status ;;
  screenshot) shift; screenshot "$@" ;;
  click) shift; click "$@" ;;
  double-click|double_click) shift; double_click "$@" ;;
  pointer-click|pointer_click) shift; pointer_click "$@" ;;
  pointer-drag|pointer_drag) shift; pointer_drag "$@" ;;
  pointer-scroll|pointer_scroll) shift; pointer_scroll "$@" ;;
  drag) shift; drag "$@" ;;
  move|move_mouse) shift; move_mouse "$@" ;;
  scroll) shift; scroll "$@" ;;
  type|type_text) shift; type_text "$@" ;;
  computer-type-stdin|computer_type_stdin) computer_type_stdin ;;
  computer-key-stdin|computer_key_stdin) shift; computer_key_stdin "$@" ;;
  computer-clipboard-write-stdin|computer_clipboard_write_stdin) computer_clipboard_write_stdin ;;
  computer-clipboard-read|computer_clipboard_read) computer_clipboard_read ;;
  computer-input-reset|computer_input_reset) computer_input_reset ;;
  key) shift; key "$@" ;;
  clipboard-get|clipboard_get) clipboard_get ;;
  clipboard-set|clipboard_set) shift; clipboard_set "$@" ;;
  clipboard-clear|clipboard_clear) clipboard_clear ;;
  paste-stdin|paste_stdin) paste_stdin ;;
  secret-paste-stdin|secret_paste_stdin) shift; secret_paste_stdin "$@" ;;
  secret-paste-submit-stdin|secret_paste_submit_stdin) shift; secret_paste_submit_stdin "$@" ;;
  computer-secret-paste-stdin|computer_secret_paste_stdin) computer_secret_paste_stdin ;;
  browser-status|browser_status) browser_status ;;
  browser-find|browser_find) shift; browser_find "$@" ;;
  browser-fill|browser_fill) shift; browser_fill "$@" ;;
  browser-click|browser_click) shift; browser_click "$@" ;;
  browser-submit|browser_submit) shift; browser_submit "$@" ;;
  browser-dialog|browser_dialog) shift; browser_dialog "$@" ;;
  browser-text|browser_text) browser_text ;;
  browser-wait-text|browser_wait_text) shift; browser_wait_text "$@" ;;
  browser-wait-selector|browser_wait_selector) shift; browser_wait_selector "$@" ;;
  browser-wait-idle|browser_wait_idle) shift; browser_wait_idle "$@" ;;
  ocr) shift; ocr "$@" ;;
  find-text|find_text) shift; find_text "$@" ;;
  open-url|open_url) shift; open_url "$@" ;;
  *)
    cat >&2 <<EOF
Usage: $(basename "$0") start|stop|status|screenshot|click|double-click|pointer-click|pointer-drag|pointer-scroll|drag|move|scroll|type|computer-type-stdin|key|computer-key-stdin|computer-clipboard-write-stdin|computer-clipboard-read|computer-input-reset|clipboard-get|clipboard-set|clipboard-clear|paste-stdin|secret-paste-stdin|secret-paste-submit-stdin|computer-secret-paste-stdin|browser-status|browser-find|browser-fill|browser-click|browser-submit|browser-dialog|browser-text|browser-wait-text|browser-wait-selector|browser-wait-idle|ocr|find-text|open-url
EOF
    exit 2
    ;;
esac
