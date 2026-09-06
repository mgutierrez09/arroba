#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

hash_stdin() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum | awk '{ print $1 }'
    return
  fi
  shasum -a 256 | awk '{ print $1 }'
}

runtime_source_revision() {
  if [[ "${CHARIOX_SLICE_BUILD_CONTEXT_DIGEST:-}" =~ ^sha256:[a-f0-9]{64}$ ]]; then
    printf '%s\n' "$CHARIOX_SLICE_BUILD_CONTEXT_DIGEST"
    return
  fi
  (
    cd "$REPO_ROOT"
    if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
      git ls-files --cached --others --exclude-standard \
        Cargo.toml Cargo.lock \
        adapters/rust \
        apps/aegs-dummy apps/kernel apps/relay \
        examples/workflow-code \
        packages/aegs-sdk packages/event-protocol
    else
      find \
        Cargo.toml Cargo.lock \
        adapters/rust \
        apps/aegs-dummy apps/kernel apps/relay \
        examples/workflow-code \
        packages/aegs-sdk packages/event-protocol \
        -type f \
        ! -path '*/target/*' \
        ! -path '*/node_modules/*' \
        | LC_ALL=C sort
    fi \
      | while IFS= read -r path; do
          [[ -f "$path" ]] || continue
          printf '%s ' "$path"
          hash_stdin < "$path"
        done
  ) | hash_stdin
}

SLICE_NAME="${CHARIOX_SLICE_NAME:-chariox-slice-linux}"
SLICE_HOSTNAME="${CHARIOX_SLICE_HOSTNAME:-$SLICE_NAME}"
SLICE_ID="${CHARIOX_SLICE_ID:-slice-linux}"
SLICE_OWNER_KERNEL_ID="${CHARIOX_SLICE_OWNER_KERNEL_ID:-}"
SLICE_OWNER_MACHINE_ID="${CHARIOX_SLICE_OWNER_MACHINE_ID:-}"
SLICE_OWNER_PUBLIC_KEY="${CHARIOX_SLICE_OWNER_PUBLIC_KEY:-}"
SLICE_IMAGE="${CHARIOX_SLICE_DOCKER_IMAGE:-chariox-slice-linux:0.1.0}"
SLICE_BASE_IMAGE="${CHARIOX_SLICE_BASE_IMAGE:-chariox-slice-linux:0.1.0}"
SLICE_BUILD_IMAGE="${CHARIOX_SLICE_BUILD_IMAGE:-auto}"
SLICE_RUNTIME_BUILD_PROFILE="${CHARIOX_SLICE_RUNTIME_BUILD_PROFILE:-release}"
SLICE_CARGO_PROFILE_RELEASE_OPT_LEVEL="${CHARIOX_SLICE_CARGO_PROFILE_RELEASE_OPT_LEVEL:-3}"
SLICE_EXTENSION_DOCKERFILE="${CHARIOX_SLICE_EXTENSION_DOCKERFILE:-}"
SLICE_DOCKER_MEMORY="${CHARIOX_SLICE_DOCKER_MEMORY:-}"
SLICE_DOCKER_CPUS="${CHARIOX_SLICE_DOCKER_CPUS:-}"
SLICE_DOCKER_PIDS_LIMIT="${CHARIOX_SLICE_DOCKER_PIDS_LIMIT:-1024}"
SLICE_DOCKER_NOFILE_LIMIT="${CHARIOX_SLICE_DOCKER_NOFILE_LIMIT:-8192}"
SLICE_HOME_VOLUME="${CHARIOX_SLICE_HOME_VOLUME:-${SLICE_NAME}-home}"
SLICE_SAVED_HOME_ARCHIVE="${CHARIOX_SLICE_SAVED_HOME_ARCHIVE:-}"
SLICE_WORKSPACE="${CHARIOX_SLICE_WORKSPACE:-$REPO_ROOT}"
SLICE_WORKSPACE_SOURCE="${CHARIOX_SLICE_WORKSPACE_SOURCE:-$SLICE_WORKSPACE}"
SLICE_DEVELOPMENT_MOUNT_COUNT="${CHARIOX_SLICE_DEVELOPMENT_MOUNT_COUNT:-0}"
SLICE_WORKSPACE_MOUNT_MODE="${CHARIOX_SLICE_WORKSPACE_MOUNT_MODE:-rw}"
SLICE_ALLOW_UNCONFINED_SECCOMP="${CHARIOX_SLICE_ALLOW_UNCONFINED_SECCOMP:-0}"
SLICE_APPARMOR_PROFILE="${CHARIOX_SLICE_APPARMOR_PROFILE:-unconfined}"
SLICE_RECREATE="${CHARIOX_SLICE_RECREATE:-0}"
SLICE_START_DESKTOP="${CHARIOX_SLICE_START_DESKTOP:-1}"
SLICE_START_PROVIDER_SERVERS="${CHARIOX_SLICE_START_PROVIDER_SERVERS:-1}"
SLICE_START_RUNTIME="${CHARIOX_SLICE_START_RUNTIME:-0}"
MANAGED_PROVIDER_ISOLATION_PROBE="${CHARIOX_MANAGED_PROVIDER_ISOLATION_PROBE:-0}"
SLICE_IMPORT_PROVIDER_AUTH="${CHARIOX_SLICE_IMPORT_PROVIDER_AUTH:-0}"
SLICE_MIN_FREE_MB="${CHARIOX_SLICE_MIN_FREE_MB:-256}"
SLICE_CODEX_PORT="${CHARIOX_SLICE_CODEX_PORT:-43252}"
SLICE_OPENCODE_PORT="${CHARIOX_SLICE_OPENCODE_PORT:-43140}"
SLICE_CODEX_PORT_RANGE="${CHARIOX_SLICE_CODEX_PORT_RANGE:-43260-43279}"
SLICE_OPENCODE_PORT_RANGE="${CHARIOX_SLICE_OPENCODE_PORT_RANGE:-43150-43169}"
SLICE_PROVIDER_BIND_HOST="${CHARIOX_SLICE_PROVIDER_BIND_HOST:-127.0.0.1}"
SLICE_KERNEL_PORT="${CHARIOX_SLICE_KERNEL_PORT:-43119}"
SLICE_MCP_PORT="${CHARIOX_SLICE_MCP_PORT:-43120}"
SLICE_RELAY_PORT="${CHARIOX_SLICE_RELAY_PORT:-43130}"
SLICE_NOVNC_PORT="${CHARIOX_SLICE_NOVNC_PORT:-6080}"
SLICE_RELAY_URL="${CHARIOX_SLICE_RELAY_URL:-}"
SLICE_RELAY_TOKEN="${CHARIOX_SLICE_RELAY_TOKEN:-slice-local}"
SLICE_CLOUD_RELAY_CONFIG_JSON="${CHARIOX_SLICE_CLOUD_RELAY_CONFIG_JSON:-}"
SLICE_CLOUD_RELAY_CONFIG_HOST_PATH="${CHARIOX_SLICE_CLOUD_RELAY_CONFIG_HOST_PATH:-}"
SLICE_DAEMON_ALIAS="${CHARIOX_SLICE_DAEMON_ALIAS:-slice:linux}"
SLICE_MACHINE_ID="${CHARIOX_SLICE_MACHINE_ID:-slice:linux}"
SLICE_MACHINE_ALIAS="${CHARIOX_SLICE_MACHINE_ALIAS:-linux}"
SLICE_CODEX_AUTH="${CHARIOX_SLICE_CODEX_AUTH:-$HOME/.codex/auth.json}"
SLICE_OPENCODE_AUTH="${CHARIOX_SLICE_OPENCODE_AUTH:-$HOME/.local/share/opencode/auth.json}"
SLICE_CLAUDE_JSON="${CHARIOX_SLICE_CLAUDE_JSON:-$HOME/.claude.json}"
SLICE_CLAUDE_SETTINGS="${CHARIOX_SLICE_CLAUDE_SETTINGS:-$HOME/.claude/settings.json}"
SLICE_CLAUDE_STATS="${CHARIOX_SLICE_CLAUDE_STATS:-$HOME/.claude/stats-cache.json}"
SLICE_GITHUB_HOST="${CHARIOX_SLICE_GITHUB_HOST:-github.com}"
SLICE_GITHUB_TOKEN_FILE="${CHARIOX_SLICE_GITHUB_TOKEN_FILE:-}"
SLICE_OPENCODE_PROVIDER="${CHARIOX_SLICE_OPENCODE_PROVIDER:-openai}"
SLICE_OPENCODE_LOGIN_METHOD="${CHARIOX_SLICE_OPENCODE_LOGIN_METHOD:-ChatGPT Pro/Plus (headless)}"
SLICE_LOGIN_PROVIDER="${CHARIOX_SLICE_LOGIN_PROVIDER:-codex}"
SLICE_AUTH_PROVIDER="${CHARIOX_SLICE_AUTH_PROVIDER:-all}"
SLICE_ACCOUNT_OWNER="${CHARIOX_SLICE_ACCOUNT_OWNER:-local-user}"
SLICE_ACCOUNT_PROFILE="${CHARIOX_SLICE_ACCOUNT_PROFILE:-default}"
SLICE_ACCOUNT_ROOT="/home/slice/.chariox/daemon/provider-accounts/$SLICE_ACCOUNT_OWNER"
SLICE_PROVIDER_HOME="/home/slice/.chariox/provider-home"
SLICE_RELAY_PEER_PROTOCOL_VERSION="$(sed -nE 's/^pub const RELAY_PEER_PROTOCOL_VERSION: u32 = ([0-9]+);$/\1/p' "$REPO_ROOT/apps/kernel/src/transport/relay_peer.rs" | head -n 1)"
SLICE_RUNTIME_SOURCE_REVISION="$(runtime_source_revision)"

log() {
  printf '[slice-linux] %s\n' "$*" >&2
}

fail() {
  printf '[slice-linux] error: %s\n' "$*" >&2
  exit 1
}

if [[ ! "$SLICE_ACCOUNT_OWNER" =~ ^[A-Za-z0-9-]+$ || ! "$SLICE_ACCOUNT_PROFILE" =~ ^[A-Za-z0-9-]+$ ]]; then
  fail "slice account owner/profile contains an unsafe path component"
fi
if [[ ! "$SLICE_DEVELOPMENT_MOUNT_COUNT" =~ ^[0-9]+$ || "$SLICE_DEVELOPMENT_MOUNT_COUNT" -gt 128 ]]; then
  fail "slice development mount count is invalid"
fi
case "$SLICE_RUNTIME_BUILD_PROFILE" in
  dev|release) ;;
  *) fail "CHARIOX_SLICE_RUNTIME_BUILD_PROFILE must be dev or release" ;;
esac
case "$SLICE_CARGO_PROFILE_RELEASE_OPT_LEVEL" in
  0|1|2|3|s|z) ;;
  *) fail "CHARIOX_SLICE_CARGO_PROFILE_RELEASE_OPT_LEVEL is invalid" ;;
esac
if [[ ! "$SLICE_APPARMOR_PROFILE" =~ ^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$ ]]; then
  fail "CHARIOX_SLICE_APPARMOR_PROFILE is invalid"
fi

run_with_timeout() {
  local seconds="$1"
  shift
  local timeout_marker="${TMPDIR:-/tmp}/chariox-slice-timeout.$$.$RANDOM"
  rm -f "$timeout_marker"
  "$@" &
  local child=$!
  (
    local elapsed=0
    while (( elapsed < seconds )); do
      sleep 1
      elapsed=$((elapsed + 1))
    done
    if kill -0 "$child" >/dev/null 2>&1; then
      : >"$timeout_marker"
      kill "$child" >/dev/null 2>&1 || true
      sleep 2
      kill -9 "$child" >/dev/null 2>&1 || true
    fi
  ) &
  local watchdog=$!
  local status=0
  wait "$child" || status=$?
  kill "$watchdog" >/dev/null 2>&1 || true
  wait "$watchdog" 2>/dev/null || true
  if [[ -f "$timeout_marker" ]]; then
    rm -f "$timeout_marker"
    log "managed slice Docker operation timed out after ${seconds}s"
    return 124
  fi
  rm -f "$timeout_marker"
  return "$status"
}

run_with_file_stdin_timeout() {
  local seconds="$1"
  local input_file="$2"
  shift 2
  local timeout_marker="${TMPDIR:-/tmp}/chariox-slice-timeout.$$.$RANDOM"
  rm -f "$timeout_marker"
  "$@" <"$input_file" &
  local child=$!
  (
    local elapsed=0
    while (( elapsed < seconds )); do
      sleep 1
      elapsed=$((elapsed + 1))
    done
    if kill -0 "$child" >/dev/null 2>&1; then
      : >"$timeout_marker"
      kill "$child" >/dev/null 2>&1 || true
      sleep 2
      kill -9 "$child" >/dev/null 2>&1 || true
    fi
  ) &
  local watchdog=$!
  local status=0
  wait "$child" || status=$?
  kill "$watchdog" >/dev/null 2>&1 || true
  wait "$watchdog" 2>/dev/null || true
  if [[ -f "$timeout_marker" ]]; then
    rm -f "$timeout_marker"
    log "managed slice Docker operation timed out after ${seconds}s"
    return 124
  fi
  rm -f "$timeout_marker"
  return "$status"
}

usage() {
  cat <<EOF
Usage: $(basename "$0") [provision|recover|status|stop|destroy|import-provider-auth|remove-provider-auth|start-provider-login|start-desktop|validate-screen|start-runtime|start-providers|shell]
       $(basename "$0") [login-codex|logout-codex|login-opencode|logout-opencode]

This Docker path is a provider/runtime validation fallback for Mac hosts when
the Lume Ubuntu prebuilt image is unavailable.

CHARIOX_SLICE_DOCKER_PIDS_LIMIT caps container processes and threads (default:
1024). Set a positive integer to tune it for the workload. Reused containers
receive the current limit before startup. Stop and destroy remain available
even if the configured value is invalid.

CHARIOX_SLICE_DOCKER_NOFILE_LIMIT sets the inherited soft and hard open-file
limit (default: 8192; allowed: 1024-1048576). Docker cannot update this limit
on an existing container, so a mismatched container fails closed with recreate
guidance before any service starts.
EOF
}

require_docker() {
  command -v docker >/dev/null || fail "docker is required"
  run_with_timeout 20 docker info >/dev/null || fail "docker is not running"
}

container_exists() {
  run_with_timeout 20 docker container inspect "$SLICE_NAME" >/dev/null 2>&1
}

container_running() {
  local state
  state="$(run_with_timeout 20 docker inspect -f '{{.State.Running}}' "$SLICE_NAME" 2>/dev/null)" || return 1
  [[ "$state" == "true" ]]
}

restore_saved_home_volume() {
  [[ -n "$SLICE_SAVED_HOME_ARCHIVE" ]] || return 0
  [[ -f "$SLICE_SAVED_HOME_ARCHIVE" ]] || fail "saved slice home archive not found: $SLICE_SAVED_HOME_ARCHIVE"
  local helper
  helper="${SLICE_NAME}-home-restore-$$"
  log "restoring saved home archive $SLICE_SAVED_HOME_ARCHIVE into volume $SLICE_HOME_VOLUME"
  run_with_timeout 30 docker rm -f "$helper" >/dev/null 2>&1 || true
  run_with_timeout 60 docker create --name "$helper" --user root \
    -v "$SLICE_HOME_VOLUME:/home-dst" \
    "$SLICE_IMAGE" \
    sleep infinity >/dev/null
  run_with_timeout 60 docker start "$helper" >/dev/null
  run_with_timeout 120 docker cp -L "$SLICE_SAVED_HOME_ARCHIVE" "$helper:/tmp/home.tar.zst"
  run_with_timeout 120 docker exec -u root "$helper" \
    bash -lc "set -euo pipefail; find /home-dst -mindepth 1 -maxdepth 1 -exec rm -rf {} +; cd /home-dst; tar --zstd -xf /tmp/home.tar.zst; chown -R slice:slice /home-dst"
  run_with_timeout 30 docker rm -f "$helper" >/dev/null 2>&1 || true
}

machine_id_hex() {
  printf '%s' "$SLICE_MACHINE_ID" | sha256sum | awk '{ print substr($1, 1, 32) }'
}

configure_stable_machine_identity() {
  local machine_id
  machine_id="$(machine_id_hex)"
  run_with_timeout 30 docker exec -u root "$SLICE_NAME" bash -lc "
    set -euo pipefail
    printf '%s\n' '$machine_id' > /etc/machine-id
    mkdir -p /var/lib/dbus
    printf '%s\n' '$machine_id' > /var/lib/dbus/machine-id
    chmod 0444 /etc/machine-id /var/lib/dbus/machine-id
  " || log "stable machine-id refresh unavailable; continuing"
}

configure_chromium_browser_policy() {
  run_with_timeout 30 docker exec -u root "$SLICE_NAME" bash -lc "
    set -euo pipefail
    for dir in /etc/chromium/policies/managed /etc/chromium-browser/policies/managed; do
      mkdir -p \"\$dir\"
      cat > \"\$dir/chariox-slice.json\" <<'JSON'
{\"BrowserSignin\":0}
JSON
      chmod 0644 \"\$dir/chariox-slice.json\"
    done
  " || log "Chromium browser policy refresh unavailable; continuing"
}

configure_slice_state_directory() {
  run_with_timeout 30 docker exec -u root "$SLICE_NAME" bash -lc "
    set -euo pipefail
    mkdir -p /tmp/chariox-slice-state
    chown slice:slice /tmp/chariox-slice-state
    chmod 700 /tmp/chariox-slice-state
  " || log "slice state directory ownership refresh unavailable; continuing"
}

refresh_slice_support_files() {
  # Saved images retain their original packages. Prepare the desktop services
  # before overlaying the current launcher.
  run_with_timeout 180 docker exec -u root "$SLICE_NAME" bash -lc '
    set -euo pipefail
    if ! command -v tint2 >/dev/null 2>&1 || ! command -v dbus-run-session >/dev/null 2>&1 \
      || [[ ! -r /usr/share/dbus-1/services/org.a11y.Bus.service ]]; then
      export DEBIAN_FRONTEND=noninteractive
      apt-get -qq update
      apt-get -y -qq --no-install-recommends install dbus tint2 at-spi2-core
      apt-get clean
      find /var/lib/apt/lists -mindepth 1 -delete
    fi
  ' || fail "could not prepare desktop services in the existing slice image"
  run_with_timeout 30 docker cp "$REPO_ROOT/apps/kernel/slice-linux-docker/docker/start-runtime.sh" "$SLICE_NAME:/opt/chariox-slice/start-runtime.sh" \
    || log "runtime script overlay refresh unavailable; continuing"
  run_with_timeout 30 docker cp "$REPO_ROOT/apps/kernel/slice-linux-docker/docker/start-providers.sh" "$SLICE_NAME:/opt/chariox-slice/start-providers.sh" \
    || log "provider server script overlay refresh unavailable; continuing"
  run_with_timeout 30 docker cp "$REPO_ROOT/apps/kernel/slice-linux-docker/docker/slice-screen.sh" "$SLICE_NAME:/opt/chariox-slice/slice-screen.sh" \
    || log "screen script overlay refresh unavailable; continuing"
  run_with_timeout 30 docker cp "$REPO_ROOT/apps/kernel/slice-linux-docker/docker/tint2rc" "$SLICE_NAME:/opt/chariox-slice/tint2rc" \
    || log "applications taskbar configuration refresh unavailable; continuing"
  run_with_timeout 30 docker cp "$REPO_ROOT/apps/kernel/slice-linux-docker/docker/slice-text-finder.py" "$SLICE_NAME:/opt/chariox-slice/slice-text-finder.py" \
    || log "screen text finder overlay refresh unavailable; continuing"
  run_with_timeout 30 docker cp "$REPO_ROOT/apps/kernel/slice-linux-docker/docker/slice-selkies.py" "$SLICE_NAME:/opt/chariox-slice/slice-selkies.py" \
    || log "Selkies lifecycle overlay refresh unavailable; continuing"
  run_with_timeout 30 docker cp "$REPO_ROOT/apps/kernel/slice-linux-docker/docker/slice-selkies-stream.py" "$SLICE_NAME:/opt/chariox-slice/slice-selkies-stream.py" \
    || log "Selkies stream overlay refresh unavailable; continuing"
  run_with_timeout 30 docker cp "$REPO_ROOT/apps/kernel/slice-linux-docker/docker/selkies_viewers.py" "$SLICE_NAME:/opt/chariox-slice/selkies_viewers.py" \
    || log "Selkies private viewer module overlay refresh unavailable; continuing"
  run_with_timeout 30 docker cp "$REPO_ROOT/apps/kernel/slice-linux-docker/docker/browser-cdp.mjs" "$SLICE_NAME:/opt/chariox-slice/browser-cdp.mjs" \
    || log "browser CDP helper overlay refresh unavailable; continuing"
  run_with_timeout 30 docker cp "$REPO_ROOT/apps/kernel/slice-linux-docker/docker/browser-controller-actions.mjs" "$SLICE_NAME:/opt/chariox-slice/browser-controller-actions.mjs" \
    || log "browser controller actions module overlay refresh unavailable; continuing"
  run_with_timeout 30 docker cp "$REPO_ROOT/apps/kernel/slice-linux-docker/docker/browser-controller-cdp.mjs" "$SLICE_NAME:/opt/chariox-slice/browser-controller-cdp.mjs" \
    || log "browser controller CDP module overlay refresh unavailable; continuing"
  run_with_timeout 30 docker cp "$REPO_ROOT/apps/kernel/slice-linux-docker/docker/browser-controller-dialogs.mjs" "$SLICE_NAME:/opt/chariox-slice/browser-controller-dialogs.mjs" \
    || log "browser controller dialog module overlay refresh unavailable; continuing"
  run_with_timeout 30 docker cp "$REPO_ROOT/apps/kernel/slice-linux-docker/docker/browser-controller-compatibility.mjs" "$SLICE_NAME:/opt/chariox-slice/browser-controller-compatibility.mjs" \
    || log "browser controller compatibility module overlay refresh unavailable; continuing"
  run_with_timeout 30 docker cp "$REPO_ROOT/apps/kernel/slice-linux-docker/docker/browser-controller-events.mjs" "$SLICE_NAME:/opt/chariox-slice/browser-controller-events.mjs" \
    || log "browser controller events module overlay refresh unavailable; continuing"
  run_with_timeout 30 docker cp "$REPO_ROOT/apps/kernel/slice-linux-docker/docker/browser-controller-files.mjs" "$SLICE_NAME:/opt/chariox-slice/browser-controller-files.mjs" \
    || log "browser controller file-transfer module overlay refresh unavailable; continuing"
  run_with_timeout 30 docker cp "$REPO_ROOT/apps/kernel/slice-linux-docker/docker/browser-controller-frames.mjs" "$SLICE_NAME:/opt/chariox-slice/browser-controller-frames.mjs" \
    || log "browser controller frame module overlay refresh unavailable; continuing"
  run_with_timeout 30 docker cp "$REPO_ROOT/apps/kernel/slice-linux-docker/docker/browser-controller-history.mjs" "$SLICE_NAME:/opt/chariox-slice/browser-controller-history.mjs" \
    || log "browser controller history module overlay refresh unavailable; continuing"
  run_with_timeout 30 docker cp "$REPO_ROOT/apps/kernel/slice-linux-docker/docker/browser-controller-permissions.mjs" "$SLICE_NAME:/opt/chariox-slice/browser-controller-permissions.mjs" \
    || log "browser controller permissions module overlay refresh unavailable; continuing"
  run_with_timeout 30 docker cp "$REPO_ROOT/apps/kernel/slice-linux-docker/docker/browser-controller-snapshot.mjs" "$SLICE_NAME:/opt/chariox-slice/browser-controller-snapshot.mjs" \
    || log "browser controller snapshot module overlay refresh unavailable; continuing"
  run_with_timeout 30 docker cp "$REPO_ROOT/apps/kernel/slice-linux-docker/docker/browser-controller.mjs" "$SLICE_NAME:/opt/chariox-slice/browser-controller.mjs" \
    || log "browser controller overlay refresh unavailable; continuing"
  run_with_timeout 30 docker cp "$REPO_ROOT/apps/kernel/slice-linux-docker/docker/managed-provider-isolation-probe.mjs" "$SLICE_NAME:/opt/chariox-slice/managed-provider-isolation-probe.mjs" \
    || log "provider isolation probe overlay refresh unavailable; continuing"
  run_with_timeout 30 docker cp "$REPO_ROOT/apps/kernel/slice-linux-docker/docker/managed-provider-isolation-probe-wrapper.sh" "$SLICE_NAME:/opt/chariox-slice/managed-provider-isolation-probe-wrapper.sh" \
    || log "provider isolation probe wrapper refresh unavailable; continuing"
  run_with_timeout 30 docker cp "$REPO_ROOT/apps/kernel/slice-linux-docker/docker/provider-port-bridge.mjs" "$SLICE_NAME:/opt/chariox-slice/provider-port-bridge.mjs" \
    || log "provider bridge overlay refresh unavailable; continuing"
  run_with_timeout 30 docker cp "$REPO_ROOT/apps/kernel/slice-linux-docker/docker/validate-screen.sh" "$SLICE_NAME:/opt/chariox-slice/validate-screen.sh" \
    || log "screen validator overlay refresh unavailable; continuing"
  run_with_timeout 30 docker exec -u root "$SLICE_NAME" chmod +x \
    /opt/chariox-slice/start-runtime.sh \
    /opt/chariox-slice/start-providers.sh \
    /opt/chariox-slice/slice-screen.sh \
    /opt/chariox-slice/browser-cdp.mjs \
    /opt/chariox-slice/browser-controller-actions.mjs \
    /opt/chariox-slice/browser-controller-cdp.mjs \
    /opt/chariox-slice/browser-controller-dialogs.mjs \
    /opt/chariox-slice/browser-controller-compatibility.mjs \
    /opt/chariox-slice/browser-controller-events.mjs \
    /opt/chariox-slice/browser-controller-files.mjs \
    /opt/chariox-slice/browser-controller-frames.mjs \
    /opt/chariox-slice/browser-controller-history.mjs \
    /opt/chariox-slice/browser-controller-permissions.mjs \
    /opt/chariox-slice/browser-controller-snapshot.mjs \
    /opt/chariox-slice/browser-controller.mjs \
    /opt/chariox-slice/managed-provider-isolation-probe.mjs \
    /opt/chariox-slice/managed-provider-isolation-probe-wrapper.sh \
    /opt/chariox-slice/provider-port-bridge.mjs \
    /opt/chariox-slice/validate-screen.sh \
    || log "script permission refresh unavailable; continuing"
}

wait_for_container_running() {
  local attempts="${1:-6}"
  local delay_seconds="${2:-5}"
  local attempt
  for ((attempt = 1; attempt <= attempts; attempt += 1)); do
    if container_running; then
      return 0
    fi
    sleep "$delay_seconds"
  done
  return 1
}

available_mb_for_path() {
  local path="$1"
  local output
  output="$(run_with_timeout 20 docker exec -u slice "$SLICE_NAME" df -Pm "$path" 2>/dev/null)" || return 1
  awk 'NR == 2 { print $4 }' <<<"$output"
}

require_slice_free_space() {
  local phase="$1"
  shift
  [[ "$SLICE_MIN_FREE_MB" =~ ^[0-9]+$ ]] || fail "CHARIOX_SLICE_MIN_FREE_MB must be a non-negative integer"
  local paths=("$@")
  local path
  for path in "${paths[@]}"; do
    local available_mb=""
    available_mb="$(available_mb_for_path "$path" || true)"
    if [[ ! "$available_mb" =~ ^[0-9]+$ ]]; then
      log "slice storage preflight unavailable for $path during $phase; continuing"
      continue
    fi
    if (( available_mb < SLICE_MIN_FREE_MB )); then
      log "slice storage preflight failed for $phase: $path has ${available_mb}MiB free, needs ${SLICE_MIN_FREE_MB}MiB"
      run_with_timeout 10 docker exec -u slice "$SLICE_NAME" df -h "${paths[@]}" >&2 || true
      fail "slice $phase needs more free space in the Docker/Colima slice filesystem. Free Docker disk or delete unused slice containers/volumes, then retry."
    fi
    log "slice storage preflight ok for $phase: $path has ${available_mb}MiB free"
  done
}

image_runtime_compatible() {
  local image="$1"
  local image_protocol_version
  local image_runtime_revision
  image_protocol_version="$(docker image inspect -f '{{ index .Config.Labels "io.chariox.relay-peer-protocol-version" }}' "$image" 2>/dev/null || true)"
  image_runtime_revision="$(docker image inspect -f '{{ index .Config.Labels "io.chariox.runtime-source-revision" }}' "$image" 2>/dev/null || true)"
  [[ "$image_protocol_version" == "$SLICE_RELAY_PEER_PROTOCOL_VERSION" \
    && "$image_runtime_revision" == "$SLICE_RUNTIME_SOURCE_REVISION" ]]
}

docker_target_arch() {
  local architecture
  architecture="$(docker info --format '{{.Architecture}}' 2>/dev/null || true)"
  case "$architecture" in
    amd64|x86_64) printf 'amd64\n' ;;
    arm64|aarch64) printf 'arm64\n' ;;
    *) fail "unsupported Docker server architecture: ${architecture:-unknown}" ;;
  esac
}

docker_build() {
  if docker buildx version >/dev/null 2>&1; then
    docker buildx build --load "$@"
    return
  fi
  if command -v docker-buildx >/dev/null 2>&1; then
    docker-buildx build --load "$@"
    return
  fi
  fail "Docker Buildx is required to build the slice runtime image"
}

build_standard_runtime_image() {
  local image="$1"
  local target_arch
  target_arch="$(docker_target_arch)"
  log "building $image"
  local prebuilt_marker="$REPO_ROOT/apps/kernel/slice-linux-docker/prebuilt/.managed-release"
  if [[ -f "$prebuilt_marker" ]]; then
    docker_build \
      --build-arg "TARGETARCH=$target_arch" \
      --build-arg "CHARIOX_PREBUILT_RUNTIME=1" \
      --build-arg "CHARIOX_RUNTIME_BUILD_PROFILE=$SLICE_RUNTIME_BUILD_PROFILE" \
      --build-arg "CARGO_PROFILE_RELEASE_OPT_LEVEL=$SLICE_CARGO_PROFILE_RELEASE_OPT_LEVEL" \
      --build-arg "CHARIOX_RELAY_PEER_PROTOCOL_VERSION=$SLICE_RELAY_PEER_PROTOCOL_VERSION" \
      --build-arg "CHARIOX_RUNTIME_SOURCE_REVISION=$SLICE_RUNTIME_SOURCE_REVISION" \
      -f "$REPO_ROOT/apps/kernel/slice-linux-docker/docker/Dockerfile" \
      -t "$image" \
      "$REPO_ROOT"
    return
  fi
  docker_build \
    --build-arg "TARGETARCH=$target_arch" \
    --build-arg "CHARIOX_RUNTIME_BUILD_PROFILE=$SLICE_RUNTIME_BUILD_PROFILE" \
    --build-arg "CARGO_PROFILE_RELEASE_OPT_LEVEL=$SLICE_CARGO_PROFILE_RELEASE_OPT_LEVEL" \
    --build-arg "CHARIOX_RELAY_PEER_PROTOCOL_VERSION=$SLICE_RELAY_PEER_PROTOCOL_VERSION" \
    --build-arg "CHARIOX_RUNTIME_SOURCE_REVISION=$SLICE_RUNTIME_SOURCE_REVISION" \
    -f "$REPO_ROOT/apps/kernel/slice-linux-docker/docker/Dockerfile" \
    -t "$image" \
    "$REPO_ROOT"
}

ensure_runtime_base_image() {
  if [[ "$SLICE_BUILD_IMAGE" != "always" ]] && image_runtime_compatible "$SLICE_BASE_IMAGE"; then
    return 0
  fi
  if [[ "$SLICE_BUILD_IMAGE" == "never" ]]; then
    fail "runtime image $SLICE_BASE_IMAGE is stale and build policy is never"
  fi
  build_standard_runtime_image "$SLICE_BASE_IMAGE"
}

build_image() {
  [[ -n "$SLICE_RELAY_PEER_PROTOCOL_VERSION" ]] \
    || fail "could not read relay peer protocol version from the kernel source"
  [[ -n "$SLICE_RUNTIME_SOURCE_REVISION" ]] \
    || fail "could not fingerprint the kernel and relay runtime source"
  case "$SLICE_BUILD_IMAGE" in
    auto|always|never) ;;
    *) fail "CHARIOX_SLICE_BUILD_IMAGE must be auto, always, or never" ;;
  esac

  if [[ "$SLICE_BUILD_IMAGE" == "never" ]]; then
    if image_runtime_compatible "$SLICE_IMAGE"; then
      log "using compatible existing $SLICE_IMAGE"
      return 0
    fi
    if docker image inspect "$SLICE_IMAGE" >/dev/null 2>&1; then
      fail "runtime image $SLICE_IMAGE is stale and build policy is never"
    fi
    fail "Docker image $SLICE_IMAGE does not exist and build policy is never"
  fi

  if [[ "$SLICE_BUILD_IMAGE" == "auto" ]] && image_runtime_compatible "$SLICE_IMAGE"; then
    log "using compatible cached $SLICE_IMAGE (protocol $SLICE_RELAY_PEER_PROTOCOL_VERSION, runtime $SLICE_RUNTIME_SOURCE_REVISION)"
    return 0
  fi

  if [[ -n "$SLICE_SAVED_HOME_ARCHIVE" ]]; then
    ensure_runtime_base_image
    if ! docker image inspect "$SLICE_IMAGE" >/dev/null 2>&1; then
      log "saved state image $SLICE_IMAGE is missing; restoring the saved home archive on $SLICE_BASE_IMAGE"
      SLICE_IMAGE="$SLICE_BASE_IMAGE"
    fi
    log "preserving saved state image $SLICE_IMAGE; its worker runtime will be refreshed after startup"
    return 0
  fi

  if [[ -n "$SLICE_EXTENSION_DOCKERFILE" ]]; then
    ensure_runtime_base_image
    local target_arch
    target_arch="$(docker_target_arch)"
    log "building $SLICE_IMAGE"
    [[ -f "$SLICE_EXTENSION_DOCKERFILE" ]] || fail "extension Dockerfile not found: $SLICE_EXTENSION_DOCKERFILE"
    docker_build \
      --build-arg "TARGETARCH=$target_arch" \
      --build-arg "CHARIOX_SLICE_BASE_IMAGE=$SLICE_BASE_IMAGE" \
      --build-arg "CHARIOX_RELAY_PEER_PROTOCOL_VERSION=$SLICE_RELAY_PEER_PROTOCOL_VERSION" \
      --build-arg "CHARIOX_RUNTIME_SOURCE_REVISION=$SLICE_RUNTIME_SOURCE_REVISION" \
      -f "$SLICE_EXTENSION_DOCKERFILE" \
      -t "$SLICE_IMAGE" \
      "$(dirname "$SLICE_EXTENSION_DOCKERFILE")"
    return 0
  fi
  build_standard_runtime_image "$SLICE_IMAGE"
}

refresh_saved_state_runtime() {
  [[ -n "$SLICE_SAVED_HOME_ARCHIVE" ]] || return 0
  local installed_revision=""
  installed_revision="$(run_with_timeout 20 docker exec -u slice "$SLICE_NAME" cat /opt/chariox-slice/runtime-source-revision 2>/dev/null || true)"
  if [[ "$installed_revision" == "$SLICE_RUNTIME_SOURCE_REVISION" ]]; then
    return 0
  fi

  ensure_runtime_base_image
  local helper="${SLICE_NAME}-runtime-refresh-$$"
  local runtime_dir
  runtime_dir="$(mktemp -d "${TMPDIR:-/tmp}/chariox-slice-runtime.XXXXXX")"
  run_with_timeout 30 docker rm -f "$helper" >/dev/null 2>&1 || true
  if ! run_with_timeout 60 docker create --name "$helper" "$SLICE_BASE_IMAGE" sleep infinity >/dev/null \
    || ! run_with_timeout 60 docker cp "$helper:/opt/chariox-slice/bin/." "$runtime_dir/" \
    || ! run_with_timeout 60 docker cp "$runtime_dir/." "$SLICE_NAME:/opt/chariox-slice/bin/"; then
    run_with_timeout 30 docker rm -f "$helper" >/dev/null 2>&1 || true
    rm -rf "$runtime_dir"
    fail "failed to refresh the saved slice worker runtime from $SLICE_BASE_IMAGE"
  fi
  if ! run_with_timeout 30 docker exec -u root "$SLICE_NAME" \
    sh -lc "chown -R slice:slice /opt/chariox-slice/bin && chmod 0755 /opt/chariox-slice/bin/chariox-kernel /opt/chariox-slice/bin/chariox-relay && printf '%s\n' '$SLICE_RUNTIME_SOURCE_REVISION' > /opt/chariox-slice/runtime-source-revision && chown slice:slice /opt/chariox-slice/runtime-source-revision"; then
    run_with_timeout 30 docker rm -f "$helper" >/dev/null 2>&1 || true
    rm -rf "$runtime_dir"
    fail "failed to activate the refreshed saved slice worker runtime"
  fi
  run_with_timeout 30 docker rm -f "$helper" >/dev/null 2>&1 || true
  rm -rf "$runtime_dir"
  log "refreshed saved slice worker runtime to $SLICE_RUNTIME_SOURCE_REVISION"
}

apply_container_process_limit() {
  run_with_timeout 30 docker update --pids-limit "$SLICE_DOCKER_PIDS_LIMIT" "$SLICE_NAME" >/dev/null \
    || fail "failed to apply slice process limit; refusing to start services"
}

verify_container_nofile_limit() {
  local actual
  actual="$(run_with_timeout 20 docker inspect --format '{{range .HostConfig.Ulimits}}{{if eq .Name "nofile"}}{{.Soft}}:{{.Hard}}{{end}}{{end}}' "$SLICE_NAME" 2>/dev/null)" \
    || fail "failed to inspect the slice file-descriptor limit; refusing to start services"
  if [[ "$actual" != "$SLICE_DOCKER_NOFILE_LIMIT:$SLICE_DOCKER_NOFILE_LIMIT" ]]; then
    fail "slice file-descriptor limit is ${actual:-unset}, expected $SLICE_DOCKER_NOFILE_LIMIT:$SLICE_DOCKER_NOFILE_LIMIT; recreate the container before startup"
  fi
}

ensure_container() {
  local created_container=0
  if [[ "$SLICE_RECREATE" == "1" ]] && container_exists; then
    log "recreating container $SLICE_NAME"
    run_with_timeout 60 docker rm -f "$SLICE_NAME" >/dev/null
  fi
  if container_exists; then
    local container_image_id desired_image_id
    container_image_id="$(docker container inspect -f '{{.Image}}' "$SLICE_NAME" 2>/dev/null || true)"
    desired_image_id="$(docker image inspect -f '{{.Id}}' "$SLICE_IMAGE" 2>/dev/null || true)"
    if [[ -n "$desired_image_id" && "$container_image_id" != "$desired_image_id" ]]; then
      log "recreating $SLICE_NAME because its worker image is stale"
      run_with_timeout 60 docker rm -f "$SLICE_NAME" >/dev/null
    fi
  fi
  case "$SLICE_WORKSPACE_MOUNT_MODE" in
    rw|ro) ;;
    *) fail "CHARIOX_SLICE_WORKSPACE_MOUNT_MODE must be rw or ro" ;;
  esac
  case "$SLICE_ALLOW_UNCONFINED_SECCOMP" in
    0|1) ;;
    *) fail "CHARIOX_SLICE_ALLOW_UNCONFINED_SECCOMP must be 0 or 1" ;;
  esac

  if container_exists; then
    log "container $SLICE_NAME already exists"
    apply_container_process_limit
    verify_container_nofile_limit
  else
    log "creating container $SLICE_NAME"
    run_with_timeout 30 docker volume create "$SLICE_HOME_VOLUME" >/dev/null
    restore_saved_home_volume
    local docker_create_args=(
      --name "$SLICE_NAME"
      --hostname "$SLICE_HOSTNAME"
      --ulimit core=0:0
      --ulimit "nofile=$SLICE_DOCKER_NOFILE_LIMIT:$SLICE_DOCKER_NOFILE_LIMIT"
      --pids-limit "$SLICE_DOCKER_PIDS_LIMIT"
      --sysctl "net.ipv4.ip_local_reserved_ports=$SLICE_CODEX_PORT_RANGE,$SLICE_OPENCODE_PORT_RANGE"
      -e "CHARIOX_SLICE_VIEWER_BACKEND=${CHARIOX_SLICE_VIEWER_BACKEND:-novnc}"
      -e "CHARIOX_SLICE_DISPLAY_MODE=${CHARIOX_SLICE_DISPLAY_MODE:-unknown}"
      -e "CHARIOX_SLICE_NOVNC_PORT=$SLICE_NOVNC_PORT"
      -e "CHARIOX_SLICE_SCREEN_GEOMETRY=${CHARIOX_SLICE_SCREEN_GEOMETRY:-1280x800x24}"
      -e "CHARIOX_SLICE_MIN_FREE_MB=$SLICE_MIN_FREE_MB"
      -p "127.0.0.1:$SLICE_CODEX_PORT:$SLICE_CODEX_PORT"
      -p "127.0.0.1:$SLICE_OPENCODE_PORT:$SLICE_OPENCODE_PORT"
      -p "127.0.0.1:$SLICE_CODEX_PORT_RANGE:$SLICE_CODEX_PORT_RANGE"
      -p "127.0.0.1:$SLICE_OPENCODE_PORT_RANGE:$SLICE_OPENCODE_PORT_RANGE"
      -p "127.0.0.1:$SLICE_KERNEL_PORT:$SLICE_KERNEL_PORT"
      -p "127.0.0.1:$SLICE_RELAY_PORT:$SLICE_RELAY_PORT"
      -p "127.0.0.1:$SLICE_NOVNC_PORT:$SLICE_NOVNC_PORT"
      -v "$SLICE_HOME_VOLUME:/home/slice"
      -v "$SLICE_WORKSPACE_SOURCE:/workspace:$SLICE_WORKSPACE_MOUNT_MODE"
      --add-host "host.docker.internal:host-gateway"
    )
    if [[ "$SLICE_ALLOW_UNCONFINED_SECCOMP" == "1" ]]; then
      # The worker kernel launches providers through an inner bubblewrap user,
      # PID, and mount namespace. Docker's default seccomp, AppArmor, and
      # system-path masks block that setup before bubblewrap can install the
      # narrower provider boundary. Managed hosts run this container in the
      # dedicated rootless daemon; ordinary local slices must opt in. These
      # are Bubblewrap's documented setup capabilities; the provider receives
      # none of them because the inner sandbox uses --cap-drop ALL.
      docker_create_args+=(
        --cap-add SYS_ADMIN
        --cap-add NET_ADMIN
        --cap-add SYS_PTRACE
        --security-opt seccomp=unconfined
        --security-opt apparmor="$SLICE_APPARMOR_PROFILE"
        --security-opt systempaths=unconfined
      )
    else
      # Chromium still installs its own renderer namespace and seccomp sandbox.
      # Preserve Docker's default restrictions except the namespace syscalls
      # needed by that sandbox. No extra container capabilities are required.
      docker_create_args+=(--security-opt "seccomp=$SCRIPT_DIR/chromium-seccomp.json")
    fi
    if [[ "$SLICE_DEVELOPMENT_MOUNT_COUNT" -gt 0 ]]; then
      docker_create_args+=(-e "CHARIOX_MANAGED_WORKSPACE_ROOT_COUNT=$SLICE_DEVELOPMENT_MOUNT_COUNT")
      for ((mount_index = 0; mount_index < SLICE_DEVELOPMENT_MOUNT_COUNT; mount_index++)); do
        mount_variable="CHARIOX_SLICE_DEVELOPMENT_MOUNT_${mount_index}"
        mount_source_variable="${mount_variable}_SOURCE"
        development_mount="${!mount_variable:-}"
        development_mount_source="${!mount_source_variable:-$development_mount}"
        [[ -n "$development_mount" ]] || fail "slice development mount $mount_index is missing"
        docker_create_args+=(-e "CHARIOX_MANAGED_WORKSPACE_ROOT_${mount_index}=$development_mount")
        docker_create_args+=(-v "$development_mount_source:$development_mount:$SLICE_WORKSPACE_MOUNT_MODE")
      done
    elif [[ "$SLICE_WORKSPACE" != "/workspace" ]]; then
      docker_create_args+=(-v "$SLICE_WORKSPACE_SOURCE:$SLICE_WORKSPACE:$SLICE_WORKSPACE_MOUNT_MODE")
    fi
    if [[ -n "$SLICE_DOCKER_MEMORY" ]]; then
      docker_create_args+=(
        --memory "$SLICE_DOCKER_MEMORY"
        --memory-swap "$SLICE_DOCKER_MEMORY"
      )
    fi
    if [[ -n "$SLICE_DOCKER_CPUS" ]]; then
      docker_create_args+=(--cpus "$SLICE_DOCKER_CPUS")
    fi
    local create_status=0
    if run_with_timeout 60 docker create "${docker_create_args[@]}" "$SLICE_IMAGE" >/dev/null; then
      create_status=0
    else
      create_status=$?
    fi
    if [[ "$create_status" -ne 0 ]]; then
      if container_exists; then
        log "docker create returned $create_status but container exists; continuing"
      else
        return "$create_status"
      fi
    fi
    created_container=1
  fi

  if ! container_running; then
    log "starting container $SLICE_NAME"
    local start_status=0
    if run_with_timeout 60 docker start "$SLICE_NAME" >/dev/null; then
      start_status=0
    else
      start_status=$?
    fi
    if [[ "$start_status" -ne 0 ]]; then
      if wait_for_container_running 24 5; then
        log "docker start returned $start_status but container is running; continuing"
      else
        log "docker start returned $start_status and container did not report running yet; continuing to verify with setup commands"
      fi
    fi
  fi

  run_with_timeout 30 docker exec -u root "$SLICE_NAME" rm -f \
    /home/slice/.chariox/daemon/config.json \
    /tmp/chariox-slice-state/cloud-relay-config.json \
    || fail "failed to scrub legacy Cloud relay credentials from the slice"

  if [[ "$created_container" == "1" ]]; then
    run_with_timeout 30 docker exec -u root "$SLICE_NAME" bash -lc "mkdir -p /home/slice/.local/share /home/slice/.config /home/slice/.cache && chown -R slice:slice /home/slice" \
      || log "home directory ownership refresh unavailable; continuing"
  fi
  configure_stable_machine_identity
  configure_chromium_browser_policy
  configure_slice_state_directory
  refresh_slice_support_files
  refresh_saved_state_runtime
}

recover_existing_container() {
  container_exists || fail "slice container $SLICE_NAME does not exist; cannot recover failed state save"
  apply_container_process_limit
  verify_container_nofile_limit
  if ! container_running; then
    log "restarting existing container $SLICE_NAME after failed state save"
    if ! run_with_timeout 60 docker start "$SLICE_NAME" >/dev/null \
      && ! wait_for_container_running 24 5; then
      fail "failed to restart existing container $SLICE_NAME after failed state save"
    fi
  fi
  run_with_timeout 30 docker exec -u root "$SLICE_NAME" rm -f \
    /home/slice/.chariox/daemon/config.json \
    /tmp/chariox-slice-state/cloud-relay-config.json \
    || fail "failed to scrub legacy Cloud relay credentials from the slice"
  configure_stable_machine_identity
  configure_chromium_browser_policy
  configure_slice_state_directory
  refresh_slice_support_files
}

start_slice_services() {
  if [[ "$SLICE_IMPORT_PROVIDER_AUTH" == "1" ]]; then
    import_provider_auth
  fi
  if [[ "$SLICE_START_DESKTOP" == "1" ]]; then
    require_slice_free_space "desktop" /home/slice /tmp
    run_required_phase desktop exec_slice_with_timeout 60 bash -lc "/opt/chariox-slice/slice-screen.sh start"
  fi
  if [[ "$SLICE_START_RUNTIME" == "1" ]]; then
    require_slice_free_space "runtime" /home/slice /tmp
    run_required_phase runtime exec_slice /opt/chariox-slice/start-runtime.sh
  fi
  if [[ "$SLICE_START_PROVIDER_SERVERS" == "1" ]]; then
    run_required_phase provider-servers exec_slice /opt/chariox-slice/start-providers.sh
  fi
}

ensure_auth_target_container() {
  require_docker
  if ! container_exists; then
    fail "container $SLICE_NAME does not exist"
  fi
  apply_container_process_limit
  verify_container_nofile_limit
  if ! container_running; then
    log "starting container $SLICE_NAME"
    run_with_timeout 60 docker start "$SLICE_NAME" >/dev/null || fail "failed to start container $SLICE_NAME"
  fi
  run_with_timeout 30 docker exec -u root "$SLICE_NAME" rm -f \
    /home/slice/.chariox/daemon/config.json \
    /tmp/chariox-slice-state/cloud-relay-config.json \
    || fail "failed to scrub legacy Cloud relay credentials from the slice"
}

exec_slice_with_timeout() {
  local seconds="$1"
  shift
  local relay_env_args=()
  # Forward a provisioner-supplied binding, including partial values so kernel
  # boot validation rejects incomplete identities rather than running unbound.
  local binding_name
  for binding_name in \
    CHARIOX_ROOM_ENVIRONMENT_HOME_KERNEL_ID \
    CHARIOX_ROOM_ENVIRONMENT_HOME_PUBLIC_KEY \
    CHARIOX_ROOM_ENVIRONMENT_SESSION_ID \
    CHARIOX_ROOM_ENVIRONMENT_SLICE_ID; do
    if [[ -n "${!binding_name+x}" ]]; then
      relay_env_args+=(-e "$binding_name=${!binding_name}")
    fi
  done
  if [[ "$SLICE_DEVELOPMENT_MOUNT_COUNT" -gt 0 ]]; then
    relay_env_args+=(-e "CHARIOX_MANAGED_WORKSPACE_ROOT_COUNT=$SLICE_DEVELOPMENT_MOUNT_COUNT")
    local mount_index mount_variable development_mount
    for ((mount_index = 0; mount_index < SLICE_DEVELOPMENT_MOUNT_COUNT; mount_index++)); do
      mount_variable="CHARIOX_SLICE_DEVELOPMENT_MOUNT_${mount_index}"
      development_mount="${!mount_variable:-}"
      [[ -n "$development_mount" ]] || fail "slice development mount $mount_index is missing"
      relay_env_args+=(-e "CHARIOX_MANAGED_WORKSPACE_ROOT_${mount_index}=$development_mount")
    done
  fi
  local relay_token_path="/tmp/chariox-slice-state/relay-token"
  local relay_token_input
  relay_token_input="$(mktemp "${TMPDIR:-/tmp}/chariox-slice-relay-token.XXXXXX")"
  trap 'rm -f "$relay_token_input"' RETURN
  chmod 600 "$relay_token_input"
  printf '%s' "$SLICE_RELAY_TOKEN" >"$relay_token_input"
  run_with_timeout 30 docker exec -u slice "$SLICE_NAME" mkdir -p /tmp/chariox-slice-state
  if ! run_with_file_stdin_timeout 30 "$relay_token_input" docker exec -i -u slice "$SLICE_NAME" \
    sh -c 'umask 077; cat > /tmp/chariox-slice-state/relay-token'; then
    rm -f "$relay_token_input"
    fail "failed to transfer the slice relay token"
  fi
  rm -f "$relay_token_input"
  trap - RETURN
  if [[ -n "$SLICE_CLOUD_RELAY_CONFIG_HOST_PATH" || -n "$SLICE_CLOUD_RELAY_CONFIG_JSON" ]]; then
    local cloud_relay_config_path="/tmp/chariox-slice-state/cloud-relay-config.json"
    if [[ -n "$SLICE_CLOUD_RELAY_CONFIG_HOST_PATH" && -f "$SLICE_CLOUD_RELAY_CONFIG_HOST_PATH" ]]; then
      run_with_timeout 30 docker exec -u root "$SLICE_NAME" mkdir -p /tmp/chariox-slice-state
      run_with_timeout 30 docker cp -L "$SLICE_CLOUD_RELAY_CONFIG_HOST_PATH" "$SLICE_NAME:$cloud_relay_config_path"
      run_with_timeout 30 docker exec -u root "$SLICE_NAME" chown slice:slice "$cloud_relay_config_path"
      run_with_timeout 30 docker exec -u root "$SLICE_NAME" chmod 600 "$cloud_relay_config_path"
    else
      run_with_timeout 30 docker exec -i -u slice "$SLICE_NAME" bash -lc "set -euo pipefail; umask 077; mkdir -p /tmp/chariox-slice-state; cat > '$cloud_relay_config_path'" <<<"$SLICE_CLOUD_RELAY_CONFIG_JSON"
    fi
    relay_env_args+=(-e CHARIOX_SLICE_CLOUD_RELAY_CONFIG_PATH="$cloud_relay_config_path")
  fi
  relay_env_args+=(-e CHARIOX_SLICE_RELAY_TOKEN_FILE="$relay_token_path")
  if [[ -n "$SLICE_RELAY_URL" ]]; then
    relay_env_args+=(-e CHARIOX_SLICE_RELAY_URL="$SLICE_RELAY_URL")
  fi
  run_with_timeout "$seconds" docker exec \
    -e CHARIOX_SLICE_MIN_FREE_MB="$SLICE_MIN_FREE_MB" \
    -e CHARIOX_SLICE_CODEX_PORT="$SLICE_CODEX_PORT" \
    -e CHARIOX_SLICE_OPENCODE_PORT="$SLICE_OPENCODE_PORT" \
    -e CHARIOX_SLICE_CODEX_PORT_RANGE="$SLICE_CODEX_PORT_RANGE" \
    -e CHARIOX_SLICE_OPENCODE_PORT_RANGE="$SLICE_OPENCODE_PORT_RANGE" \
    -e CHARIOX_SLICE_PROVIDER_BIND_HOST="$SLICE_PROVIDER_BIND_HOST" \
    -e CHARIOX_SLICE_KERNEL_PORT="$SLICE_KERNEL_PORT" \
    -e CHARIOX_SLICE_MCP_PORT="$SLICE_MCP_PORT" \
    -e CHARIOX_SLICE_RELAY_PORT="$SLICE_RELAY_PORT" \
    -e CHARIOX_SLICE_NOVNC_PORT="$SLICE_NOVNC_PORT" \
    -e CHARIOX_SLICE_VIEWER_BACKEND="${CHARIOX_SLICE_VIEWER_BACKEND:-novnc}" \
    "${relay_env_args[@]}" \
    -e CHARIOX_SLICE_DAEMON_ALIAS="$SLICE_DAEMON_ALIAS" \
    -e CHARIOX_SLICE_MACHINE_ID="$SLICE_MACHINE_ID" \
    -e CHARIOX_SLICE_MACHINE_ALIAS="$SLICE_MACHINE_ALIAS" \
    -e CHARIOX_SLICE_ID="$SLICE_ID" \
    -e CHARIOX_SLICE_OWNER_KERNEL_ID="$SLICE_OWNER_KERNEL_ID" \
    -e CHARIOX_SLICE_OWNER_MACHINE_ID="$SLICE_OWNER_MACHINE_ID" \
    -e CHARIOX_SLICE_OWNER_PUBLIC_KEY="$SLICE_OWNER_PUBLIC_KEY" \
    -e CHARIOX_SLICE_SCREEN_GEOMETRY="${CHARIOX_SLICE_SCREEN_GEOMETRY:-1280x800x24}" \
    -e CHARIOX_MANAGED_PROVIDER_ISOLATION_PROBE="$MANAGED_PROVIDER_ISOLATION_PROBE" \
    -u slice \
    "$SLICE_NAME" \
    "$@"
}

exec_slice() {
  exec_slice_with_timeout 90 "$@"
}

slice_screen_diagnostics() {
  log "slice screen diagnostics"
  run_with_timeout 30 docker exec -u slice "$SLICE_NAME" bash -lc "
    set +e
    /opt/chariox-slice/slice-screen.sh status
    echo '--- processes'
    pgrep -af 'Xvfb|openbox|x11vnc|websockify|chromium' || true
    echo '--- logs'
    for log_file in /opt/chariox-slice/logs/xvfb.log /opt/chariox-slice/logs/openbox.log /opt/chariox-slice/logs/x11vnc.log /opt/chariox-slice/logs/novnc.log /opt/chariox-slice/logs/chromium-gui.log; do
      echo \"==== \${log_file}\"
      tail -n 40 \"\${log_file}\" 2>/dev/null || true
    done
  " >&2 || log "slice screen diagnostics unavailable"
}

run_required_phase() {
  local label="$1"
  shift
  log "starting phase: $label"
  local status=0
  "$@" || status=$?
  if [[ "$status" -eq 0 ]]; then
    log "completed phase: $label"
    return 0
  fi
  log "phase failed: $label (status $status)"
  case "$label" in
    desktop)
      slice_screen_diagnostics
      ;;
  esac
  return "$status"
}

copy_provider_auth_file() {
  local source_path="$1"
  local target_path="$2"
  local label="$3"

  if [[ ! -f "$source_path" ]]; then
    log "$label auth not found at $source_path; skipping"
    return 0
  fi

  local target_dir
  target_dir="$(dirname "$target_path")"
  local backup_path="${target_path}.before-slice-auth"
  run_with_file_stdin_timeout 90 "$source_path" docker exec -i -u slice "$SLICE_NAME" bash -lc "
    set -euo pipefail
    mkdir -p '$target_dir'
    rm -f '${target_path}.before-slice-auth-'*
    if [[ -f '$target_path' ]]; then
      cp '$target_path' '$backup_path'
    fi
    umask 077
    cat > '$target_path'
    chmod 600 '$target_path'
  "
  log "imported $label auth into $target_path"
}

trust_claude_slice_workspace() {
  if ! run_with_timeout 30 docker exec -e "CHARIOX_SLICE_TRUST_WORKSPACE=$SLICE_WORKSPACE" -u slice "$SLICE_NAME" bash -lc "node <<'NODE'
const fs = require('fs')
const file = '/home/slice/.claude.json'
let data = {}
try {
  data = JSON.parse(fs.readFileSync(file, 'utf8'))
} catch {
  data = {}
}
const projects = data.projects && typeof data.projects === 'object' ? data.projects : {}
const template = Object.values(projects).find((value) =>
  value && typeof value === 'object' && Object.prototype.hasOwnProperty.call(value, 'hasTrustDialogAccepted')
) || {}
for (const workspace of new Set(['/workspace', process.env.CHARIOX_SLICE_TRUST_WORKSPACE].filter(Boolean))) {
  projects[workspace] = {
    ...template,
    allowedTools: Array.isArray(template.allowedTools) ? template.allowedTools : [],
    hasTrustDialogAccepted: true,
    projectOnboardingSeenCount: Math.max(Number(template.projectOnboardingSeenCount) || 0, 1),
  }
}
data.projects = projects
data.hasCompletedOnboarding = true
fs.writeFileSync(file, JSON.stringify(data, null, 2))
fs.chmodSync(file, 0o600)
NODE"
  then
    log "Claude workspace trust update unavailable; continuing"
    return 0
  fi
  log "marked /workspace and $SLICE_WORKSPACE as trusted for Claude Code"
}

import_provider_auth() {
  ensure_auth_target_container
  require_slice_free_space "provider-auth" /home/slice /tmp
  case "$SLICE_AUTH_PROVIDER" in
    all)
      import_codex_auth
      import_opencode_auth
      import_claude_auth
      import_github_auth
      ;;
    codex)
      import_codex_auth
      ;;
    opencode|opencode:*)
      import_opencode_auth
      ;;
    claude)
      import_claude_auth
      ;;
    github)
      import_github_auth
      ;;
    *)
      fail "unsupported provider auth import: $SLICE_AUTH_PROVIDER"
      ;;
  esac
}

remove_provider_auth() {
  ensure_auth_target_container
  case "$SLICE_AUTH_PROVIDER" in
    all)
      remove_codex_auth
      remove_opencode_auth
      remove_claude_auth
      remove_github_auth
      ;;
    codex)
      remove_codex_auth
      ;;
    opencode|opencode:*)
      remove_opencode_auth
      ;;
    claude)
      remove_claude_auth
      ;;
    github)
      remove_github_auth
      ;;
    *)
      fail "unsupported provider auth removal: $SLICE_AUTH_PROVIDER"
      ;;
  esac
}

import_codex_auth() {
  copy_provider_auth_file "$SLICE_CODEX_AUTH" "$SLICE_ACCOUNT_ROOT/codex/$SLICE_ACCOUNT_PROFILE/codex/auth.json" "Codex"
}

remove_codex_auth() {
  exec_slice bash -lc "rm -f '$SLICE_ACCOUNT_ROOT/codex/$SLICE_ACCOUNT_PROFILE/codex/auth.json'"
  log "removed Codex auth from slice"
}

import_opencode_auth() {
  copy_provider_auth_file "$SLICE_OPENCODE_AUTH" "$SLICE_ACCOUNT_ROOT/opencode/$SLICE_ACCOUNT_PROFILE/data/opencode/auth.json" "OpenCode"
}

remove_opencode_auth() {
  exec_slice bash -lc "rm -rf '$SLICE_ACCOUNT_ROOT/opencode/$SLICE_ACCOUNT_PROFILE/data/opencode' '$SLICE_ACCOUNT_ROOT/opencode/$SLICE_ACCOUNT_PROFILE/config/opencode' '$SLICE_ACCOUNT_ROOT/opencode/$SLICE_ACCOUNT_PROFILE/state/opencode'"
  log "removed OpenCode auth from slice"
}

import_claude_auth() {
  local claude_root="$SLICE_ACCOUNT_ROOT/claude/$SLICE_ACCOUNT_PROFILE/claude"
  copy_provider_auth_file "$SLICE_CLAUDE_SETTINGS" "$claude_root/settings.json" "Claude settings"
  copy_provider_auth_file "$SLICE_CLAUDE_STATS" "$claude_root/stats-cache.json" "Claude stats"
}

remove_claude_auth() {
  exec_slice bash -lc "rm -rf '$SLICE_ACCOUNT_ROOT/claude/$SLICE_ACCOUNT_PROFILE/claude'"
  log "removed Claude auth from slice"
}

import_github_auth() {
  if ! command -v gh >/dev/null 2>&1; then
    log "GitHub CLI is not installed on the kernel host; skipping GitHub auth import"
    return 0
  fi

  local token_tmp
  token_tmp="$(mktemp "${TMPDIR:-/tmp}/chariox-github-token.XXXXXX")"
  trap 'rm -f "$token_tmp"' RETURN
  chmod 600 "$token_tmp"
  if [[ -n "$SLICE_GITHUB_TOKEN_FILE" ]]; then
    if [[ -s "$SLICE_GITHUB_TOKEN_FILE" ]]; then
      cp "$SLICE_GITHUB_TOKEN_FILE" "$token_tmp"
    elif [[ "$SLICE_AUTH_PROVIDER" == "github" ]]; then
      rm -f "$token_tmp"
      fail "GitHub auth import requires an explicit managed credential input"
    else
      rm -f "$token_tmp"
      log "GitHub auth has no explicit managed credential input; skipping"
      return 0
    fi
  elif ! gh auth token --hostname "$SLICE_GITHUB_HOST" >"$token_tmp" 2>/dev/null || [[ ! -s "$token_tmp" ]]; then
      rm -f "$token_tmp"
      log "GitHub auth is not configured on the kernel host; skipping"
      return 0
  fi

  if ! run_with_timeout 30 docker exec -u slice "$SLICE_NAME" bash -lc "command -v gh >/dev/null 2>&1"; then
    rm -f "$token_tmp"
    log "GitHub CLI is not installed in the slice image; skipping GitHub auth import"
    return 0
  fi

  local import_status=0
  run_with_file_stdin_timeout 90 "$token_tmp" docker exec -i -u slice "$SLICE_NAME" bash -lc "
    set -euo pipefail
    install -d -m 0700 '$SLICE_PROVIDER_HOME'
    export HOME='$SLICE_PROVIDER_HOME'
    gh auth login --hostname '$SLICE_GITHUB_HOST' --git-protocol https --with-token >/dev/null
    gh auth setup-git --hostname '$SLICE_GITHUB_HOST' >/dev/null
  " || import_status=$?
  rm -f "$token_tmp"
  trap - RETURN
  if [[ "$import_status" != "0" ]]; then
    fail "failed to import GitHub auth into slice"
  fi
  log "imported GitHub auth from the kernel host"
}

remove_github_auth() {
  exec_slice bash -lc "
    set +e
    export HOME='$SLICE_PROVIDER_HOME'
    gh auth logout --hostname '$SLICE_GITHUB_HOST' >/dev/null 2>&1
    git config --global --remove-section credential.https://'$SLICE_GITHUB_HOST' >/dev/null 2>&1
    git config --global --remove-section credential.https://gist.'$SLICE_GITHUB_HOST' >/dev/null 2>&1
    exit 0
  "
  log "removed GitHub auth from slice"
}

print_provider_auth_status() {
  if ! exec_slice_with_timeout 30 bash -lc "
    set +e
    probe() {
      local label=\"\$1\"
      shift
      if command -v timeout >/dev/null 2>&1; then
        timeout 8s \"\$@\"
        local status=\$?
        if [[ \$status -eq 124 ]]; then
          printf '%s probe timed out\\n' \"\$label\" >&2
        fi
        return \$status
      fi
      \"\$@\"
      return \$?
    }
    echo '--- provider auth'
    probe codex codex login status || true
    probe opencode opencode providers list || true
    probe claude claude auth status --text || probe claude claude auth status || true
  "; then
    log "provider auth status diagnostics unavailable"
  fi
}

provider_login_command() {
  case "$SLICE_LOGIN_PROVIDER" in
    codex)
      printf '%s\n' "CODEX_HOME='$SLICE_ACCOUNT_ROOT/codex/$SLICE_ACCOUNT_PROFILE/codex' codex login --device-auth"
      ;;
    opencode|opencode:openai)
      printf '%s\n' "XDG_DATA_HOME='$SLICE_ACCOUNT_ROOT/opencode/$SLICE_ACCOUNT_PROFILE/data' XDG_CONFIG_HOME='$SLICE_ACCOUNT_ROOT/opencode/$SLICE_ACCOUNT_PROFILE/config' XDG_STATE_HOME='$SLICE_ACCOUNT_ROOT/opencode/$SLICE_ACCOUNT_PROFILE/state' XDG_CACHE_HOME='$SLICE_ACCOUNT_ROOT/opencode/$SLICE_ACCOUNT_PROFILE/cache' OPENCODE_CONFIG_DIR='$SLICE_ACCOUNT_ROOT/opencode/$SLICE_ACCOUNT_PROFILE/config/opencode' opencode auth login"
      ;;
    claude|claude:claudeai)
      printf '%s\n' "CLAUDE_CONFIG_DIR='$SLICE_ACCOUNT_ROOT/claude/$SLICE_ACCOUNT_PROFILE/claude' claude auth login"
      ;;
    github)
      printf '%s\n' "install -d -m 0700 '$SLICE_PROVIDER_HOME' && export HOME='$SLICE_PROVIDER_HOME' && gh auth login --hostname '$SLICE_GITHUB_HOST' --git-protocol https --web && gh auth setup-git --hostname '$SLICE_GITHUB_HOST'"
      ;;
    *)
      fail "unsupported slice provider login: $SLICE_LOGIN_PROVIDER"
      ;;
  esac
}

start_provider_login() {
  ensure_container
  require_slice_free_space "provider-login" /home/slice /tmp
  local safe_provider
  safe_provider="$(printf '%s' "$SLICE_LOGIN_PROVIDER" | tr -c 'A-Za-z0-9_.-' '-')"
  local session_name="chariox-slice-login-${safe_provider}"
  local log_file="/opt/chariox-slice/logs/provider-login-${safe_provider}.log"
  local command_text
  command_text="$(provider_login_command)"
  log "starting $SLICE_LOGIN_PROVIDER login in $session_name"
  run_with_timeout 30 docker exec -u slice "$SLICE_NAME" bash -lc "
    set -euo pipefail
    mkdir -p /opt/chariox-slice/logs
    rm -f '$log_file'
    screen -S '$session_name' -X quit >/dev/null 2>&1 || true
    screen -dmS '$session_name' bash -lc \"set +e; $command_text 2>&1 | tee -a '$log_file'; printf '\\n[chariox] provider login exited with status %s\\n' \\\${PIPESTATUS[0]} | tee -a '$log_file'; exec bash\"
  " || log "provider login start command did not confirm; continuing with screen fallback"
  sleep 3
  local login_output=""
  login_output="$(run_with_timeout 30 docker exec -u slice "$SLICE_NAME" bash -lc "cat '$log_file' 2>/dev/null || true")" || true
  if [[ -n "$login_output" ]]; then
    printf '%s\n' "$login_output"
  else
    printf '[chariox] provider login started in screen session %s; open the slice screen or slice logs to continue\n' "$session_name"
  fi
}

print_status() {
  log "container: $SLICE_NAME"
  if ! exec_slice_with_timeout 30 bash -lc "
    set +e
    probe() {
      local label=\"\$1\"
      shift
      if command -v timeout >/dev/null 2>&1; then
        timeout 8s \"\$@\"
        local status=\$?
        if [[ \$status -eq 124 ]]; then
          printf '%s probe timed out\\n' \"\$label\" >&2
        fi
        return \$status
      fi
      \"\$@\"
      return \$?
    }
    echo '--- versions'
    probe node node --version || true
    probe npm npm --version || true
    probe codex codex --version || true
    probe opencode opencode --version || true
    probe claude claude --version || true
    probe chromium chromium --version || true
    probe tesseract tesseract --version | head -n 1 || true
    echo '--- browser smoke'
    if probe chromium-headless chromium --headless=new --disable-gpu --dump-dom 'data:text/html,slice-browser-ok' >/tmp/chromium-smoke.out 2>/tmp/chromium-smoke.err; then
      grep -q 'slice-browser-ok' /tmp/chromium-smoke.out && echo chromium=headless-ok
    else
      cat /tmp/chromium-smoke.err 2>/dev/null || true
      echo chromium=headless-unavailable
    fi
    echo '--- desktop'
    probe slice-screen /opt/chariox-slice/slice-screen.sh status || true
    echo '--- binaries'
    probe binaries ls -l /opt/chariox-slice/bin || true
    echo '--- processes'
    probe processes pgrep -af 'chariox-kernel|chariox-relay|codex app-server|opencode serve' || true
    echo '--- logs'
    probe logs ls -1 /opt/chariox-slice/logs || true
  "; then
    log "status diagnostics unavailable"
  fi
  print_provider_auth_status
}

stop_container() {
  if ! docker ps -a --format '{{.Names}}' | grep -Fxq "$SLICE_NAME"; then
    log "container $SLICE_NAME does not exist"
    return 0
  fi
  if docker ps --format '{{.Names}}' | grep -Fxq "$SLICE_NAME"; then
    log "stopping slice processes in $SLICE_NAME"
    docker exec -u slice "$SLICE_NAME" bash -lc "
      screen -S chariox-slice-relay -X quit >/dev/null 2>&1 || true
      screen -S chariox-slice-kernel -X quit >/dev/null 2>&1 || true
      /opt/chariox-slice/slice-screen.sh stop >/dev/null 2>&1 || true
      pkill -f 'codex app-server' >/dev/null 2>&1 || true
      pkill -f 'opencode serve' >/dev/null 2>&1 || true
    " || true
    docker stop "$SLICE_NAME" >/dev/null
  else
    log "container $SLICE_NAME is already stopped"
  fi
}

destroy_container() {
  stop_container
  if docker ps -a --format '{{.Names}}' | grep -Fxq "$SLICE_NAME"; then
    log "removing container $SLICE_NAME"
    docker rm "$SLICE_NAME" >/dev/null
  fi
  if docker volume inspect "$SLICE_HOME_VOLUME" >/dev/null 2>&1; then
    log "removing volume $SLICE_HOME_VOLUME"
    docker volume rm "$SLICE_HOME_VOLUME" >/dev/null
  fi
}

main() {
  local action="${1:-provision}"
  case "$action" in
    -h|--help|help|status|stop|destroy) ;;
    *)
      if [[ ! "$SLICE_DOCKER_PIDS_LIMIT" =~ ^[1-9][0-9]{0,9}$ ]] || (( SLICE_DOCKER_PIDS_LIMIT > 2147483647 )); then
        fail "CHARIOX_SLICE_DOCKER_PIDS_LIMIT must be an integer from 1 to 2147483647"
      fi
      if [[ ! "$SLICE_DOCKER_NOFILE_LIMIT" =~ ^[1-9][0-9]{0,6}$ ]] \
        || (( SLICE_DOCKER_NOFILE_LIMIT < 1024 || SLICE_DOCKER_NOFILE_LIMIT > 1048576 )); then
        fail "CHARIOX_SLICE_DOCKER_NOFILE_LIMIT must be an integer from 1024 to 1048576"
      fi
      ;;
  esac
  case "$action" in
    -h|--help|help)
      usage
      ;;
    provision)
      require_docker
      build_image
      ensure_container
      start_slice_services
      log "provision completed; use status or logs actions for diagnostics"
      ;;
    restore-state)
      require_docker
      [[ -n "$SLICE_SAVED_HOME_ARCHIVE" ]] || fail "restore-state requires a saved home archive"
      build_image
      destroy_container
      ensure_container
      stop_container
      log "saved slice state restored; container remains stopped"
      ;;
    recover)
      require_docker
      recover_existing_container
      start_slice_services
      log "failed-save recovery completed; existing container and home state preserved"
      ;;
    status)
      require_docker
      print_status
      ;;
    stop)
      require_docker
      stop_container
      ;;
    destroy)
      require_docker
      destroy_container
      ;;
    import-provider-auth)
      require_docker
      import_provider_auth
      log "provider auth import completed; account summaries are extracted by the home kernel"
      ;;
    remove-provider-auth)
      require_docker
      remove_provider_auth
      log "provider auth removal completed; account summaries are reconciled by the home kernel"
      ;;
    start-provider-login)
      require_docker
      start_provider_login
      ;;
    login-codex)
      require_docker
      ensure_container
      docker exec -it -u slice "$SLICE_NAME" codex login --device-auth
      ;;
    logout-codex)
      require_docker
      ensure_container
      exec_slice codex logout
      ;;
    login-opencode)
      require_docker
      ensure_container
      docker exec -it -u slice "$SLICE_NAME" opencode auth login
      ;;
    logout-opencode)
      require_docker
      ensure_container
      docker exec -it -u slice "$SLICE_NAME" opencode auth logout
      ;;
    start-desktop)
      require_docker
      ensure_container
      require_slice_free_space "desktop" /home/slice /tmp
      run_required_phase desktop exec_slice_with_timeout 60 bash -lc "/opt/chariox-slice/slice-screen.sh start"
      ;;
    validate-screen)
      require_docker
      ensure_container
      exec_slice /opt/chariox-slice/validate-screen.sh prepare
      exec_slice /opt/chariox-slice/validate-screen.sh interact
      ;;
    start-runtime)
      require_docker
      ensure_container
      require_slice_free_space "runtime" /home/slice /tmp
      exec_slice /opt/chariox-slice/start-runtime.sh
      ;;
    start-providers)
      require_docker
      ensure_container
      require_slice_free_space "provider-servers" /home/slice /tmp
      exec_slice /opt/chariox-slice/start-providers.sh
      ;;
    shell)
      require_docker
      ensure_container
      docker exec -it -u slice "$SLICE_NAME" bash
      ;;
    *)
      usage
      fail "unknown action: $action"
      ;;
  esac
}

main "$@"
