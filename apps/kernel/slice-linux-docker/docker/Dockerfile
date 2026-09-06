# syntax=docker/dockerfile:1@sha256:ecfaec9ed6d810b56388c508f4121597bfbba70d41a6dfeee4d8cad5f295fc32
FROM node:22.17.1-bookworm@sha256:37ff334612f77d8f999c10af8797727b731629c26f2e83caa6af390998bdc49c AS ca-bundle
RUN echo 'a3413a37a8e09cc21b2c11c9ffb23d92d2fc9d1933c9e7617f5c4fba4f72d37d  /etc/ssl/certs/ca-certificates.crt' | sha256sum --check --strict

FROM rust:1.88-bookworm@sha256:af306cfa71d987911a781c37b59d7d67d934f49684058f96cf72079c3626bfe0 AS rust-builder
WORKDIR /opt/chariox-source
COPY --from=ca-bundle /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
RUN printf '%s\n' 'deb [check-valid-until=no] https://snapshot.debian.org/archive/debian/20260701T000000Z bookworm main' > /etc/apt/sources.list \
  && rm -f /etc/apt/sources.list.d/debian.sources \
  && apt-get -o Acquire::Check-Valid-Until=false update \
  && apt-get install -y --no-install-recommends \
    libdbus-1-dev \
    libssl-dev \
    pkg-config \
    protobuf-compiler \
  && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY adapters/rust adapters/rust
COPY apps/aegs-dummy apps/aegs-dummy
COPY apps/kernel apps/kernel
COPY apps/relay apps/relay
COPY examples/workflow-code examples/workflow-code
COPY packages/aegs-sdk packages/aegs-sdk
COPY packages/event-protocol packages/event-protocol
COPY apps/kernel/slice-linux-docker/prebuilt /opt/chariox-prebuilt
ARG CHARIOX_PREBUILT_RUNTIME=0
ARG CARGO_PROFILE_RELEASE_OPT_LEVEL=3
ARG CHARIOX_RUNTIME_BUILD_PROFILE=release
RUN mkdir -p /opt/chariox-runtime-bin \
  && if [ "$CHARIOX_PREBUILT_RUNTIME" = "1" ]; then \
      test -x /opt/chariox-prebuilt/chariox-kernel \
      && test -x /opt/chariox-prebuilt/chariox-relay \
      && install -m 0755 /opt/chariox-prebuilt/chariox-kernel /opt/chariox-runtime-bin/chariox-kernel \
      && install -m 0755 /opt/chariox-prebuilt/chariox-relay /opt/chariox-runtime-bin/chariox-relay; \
    else \
      case "$CHARIOX_RUNTIME_BUILD_PROFILE" in \
        release) \
          CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 cargo build --locked --release --package chariox-kernel --bin chariox-kernel --bin chariox-managed-bootstrap \
          && CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 cargo build --locked --release --package chariox-relay --bin chariox-relay \
          && profile_dir="release" \
          ;; \
        dev) \
          CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 cargo build --locked --package chariox-kernel --bin chariox-kernel --bin chariox-managed-bootstrap \
          && CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 cargo build --locked --package chariox-relay --bin chariox-relay \
          && profile_dir="debug" \
          ;; \
        *) echo "unsupported runtime build profile: $CHARIOX_RUNTIME_BUILD_PROFILE" >&2; exit 2 ;; \
      esac \
      && install -m 0755 "target/$profile_dir/chariox-kernel" /opt/chariox-runtime-bin/chariox-kernel \
      && install -m 0755 "target/$profile_dir/chariox-relay" /opt/chariox-runtime-bin/chariox-relay; \
    fi

FROM node:26-bookworm-slim@sha256:367679cf9792759492a486e4aa4b421764d71a9546a6dae8aab81a99eb797b3e AS selkies-build
RUN apt-get update && apt-get install -y --no-install-recommends python3 python3-venv \
  && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY apps/kernel/slice-linux-docker/fetch-selkies.mjs apps/kernel/slice-linux-docker/selkies.lock.json ./
COPY apps/kernel/slice-linux-docker/selkies-build/ ./locks/
ARG TARGETARCH
RUN bash /build/locks/build.sh "$TARGETARCH"

FROM node:22-bookworm-slim@sha256:d649c27dae7ba0137b3cef5dd75baa422c08dc3d9e3fc0c23dfb172dc3cc6436

ENV DEBIAN_FRONTEND=noninteractive
ENV CHARIOX_SLICE_ROOT=/opt/chariox-slice
COPY --from=ca-bundle /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
RUN printf '%s\n' 'deb [check-valid-until=no] https://snapshot.debian.org/archive/debian/20260701T000000Z bookworm main' > /etc/apt/sources.list \
  && rm -f /etc/apt/sources.list.d/debian.sources \
  && apt-get -o Acquire::Check-Valid-Until=false update \
  && apt-get install -y --no-install-recommends --allow-downgrades \
    bash \
    bubblewrap \
    build-essential \
    ca-certificates \
    chromium \
    chromium-driver \
    chromium-sandbox \
    curl \
    dbus \
    fonts-dejavu \
    fonts-liberation \
    gh \
    git \
    jq \
    libdbus-1-dev \
    libdrm2 \
    libegl1 \
    libgbm1 \
    libpixman-1-0 \
    libpulse0 \
    libseccomp-dev \
    libssl3=3.0.20-1~deb12u1 \
    libssl-dev \
    libva2 \
    libva-drm2 \
    libva-x11-2 \
    libwayland-server0 \
    libxkbcommon0 \
    lsof \
    novnc \
    openbox \
    pkg-config \
    procps \
    protobuf-compiler \
    pulseaudio \
    python3 \
    python3-venv \
    screen \
    scrot \
    socat \
    tesseract-ocr \
    tesseract-ocr-eng \
    unzip \
    websockify \
    x11vnc \
    x11-utils \
    x11-xserver-utils \
    xclip \
    xdotool \
    xvfb \
    zstd \
  && rm -rf /var/lib/apt/lists/*

# Nested Docker cannot write an unprivileged uid_map on every host. The
# setuid helper only sets up Bubblewrap's namespace and drops its caps
# before the provider starts.
RUN chmod 4755 /usr/bin/bwrap

RUN --mount=type=bind,source=apps/kernel/slice-linux-docker/docker/managed-provider-seccomp.c,target=/tmp/managed-provider-seccomp.c \
    mkdir -p /usr/local/libexec/chariox \
  && cc -O2 -Wall -Wextra -Werror /tmp/managed-provider-seccomp.c -lseccomp -o /tmp/managed-provider-seccomp \
  && /tmp/managed-provider-seccomp /usr/local/libexec/chariox/managed-provider-seccomp.bpf \
  && chmod 0444 /usr/local/libexec/chariox/managed-provider-seccomp.bpf
COPY --chmod=0555 apps/kernel/slice-linux-docker/docker/managed-provider-bwrap.sh /usr/local/libexec/chariox/managed-provider-bwrap

RUN --mount=type=bind,from=selkies-build,source=/out,target=/tmp/selkies-package \
    --mount=type=bind,source=apps/kernel/slice-linux-docker/selkies-runtime.requirements.txt,target=/tmp/selkies-requirements.txt \
    python3 -m venv /opt/chariox-selkies \
  && /opt/chariox-selkies/bin/pip install --no-cache-dir --only-binary=:all: \
    --require-hashes --requirement /tmp/selkies-requirements.txt \
  && /opt/chariox-selkies/bin/pip install --no-cache-dir --no-index --no-deps \
    /tmp/selkies-package/selkies-*.whl /tmp/selkies-package/wheels/*.whl \
  && /opt/chariox-selkies/bin/pip check \
  && mkdir -p /usr/share/doc/chariox-selkies \
  && cp -a /tmp/selkies-package/licenses/. /usr/share/doc/chariox-selkies/ \
  && cp /tmp/selkies-requirements.txt /usr/share/doc/chariox-selkies/python-requirements.txt
ENV CHARIOX_SLICE_SELKIES_BIN=/opt/chariox-selkies/bin/selkies
RUN --mount=type=bind,source=apps/kernel/slice-linux-docker/docker,target=/tmp/chariox-slice-runtime-tests \
    PYTHONDONTWRITEBYTECODE=1 /opt/chariox-selkies/bin/python -m unittest discover \
      -s /tmp/chariox-slice-runtime-tests -p 'test_*.py'
COPY apps/kernel/slice-linux-docker/docker/slice-selkies.py \
     apps/kernel/slice-linux-docker/docker/slice-selkies-stream.py \
     apps/kernel/slice-linux-docker/docker/selkies_viewers.py /opt/chariox-slice/

WORKDIR /opt/chariox-toolchain
COPY apps/kernel/slice-linux-docker/toolchain/package.json apps/kernel/slice-linux-docker/toolchain/package-lock.json ./
RUN npm_config_cache=/tmp/chariox-npm-cache npm ci --omit=dev \
  && test "$(node_modules/.bin/codex --version)" = "codex-cli 0.144.5" \
  && test "$(node_modules/.bin/opencode --version)" = "1.18.23" \
  && test "$(node_modules/.bin/claude --version)" = "2.1.207 (Claude Code)" \
  && test "$(node_modules/.bin/pnpm --version)" = "11.22.0" \
  && rm -rf /tmp/chariox-npm-cache /root/.npm
ENV PATH=/opt/chariox-toolchain/node_modules/.bin:/usr/local/bin:/usr/local/sbin:/usr/sbin:/usr/bin:/sbin:/bin

RUN groupadd --gid 1001 slice \
  && useradd --uid 1001 --gid 1001 -m -s /bin/bash slice \
  && mkdir -p /opt/chariox-slice/bin /opt/chariox-slice/logs \
  && chown -R slice:slice /opt/chariox-slice

COPY --from=rust-builder --chown=slice:slice /opt/chariox-runtime-bin/chariox-kernel /opt/chariox-slice/bin/chariox-kernel
COPY --from=rust-builder --chown=slice:slice /opt/chariox-runtime-bin/chariox-relay /opt/chariox-slice/bin/chariox-relay
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/browser-cdp.mjs /opt/chariox-slice/browser-cdp.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/browser-controller-actions.mjs /opt/chariox-slice/browser-controller-actions.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/browser-controller-cdp.mjs /opt/chariox-slice/browser-controller-cdp.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/browser-controller-dialogs.mjs /opt/chariox-slice/browser-controller-dialogs.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/browser-controller-compatibility.mjs /opt/chariox-slice/browser-controller-compatibility.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/browser-controller-events.mjs /opt/chariox-slice/browser-controller-events.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/browser-controller-files.mjs /opt/chariox-slice/browser-controller-files.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/browser-controller-frames.mjs /opt/chariox-slice/browser-controller-frames.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/browser-controller-history.mjs /opt/chariox-slice/browser-controller-history.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/browser-controller-permissions.mjs /opt/chariox-slice/browser-controller-permissions.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/browser-controller-snapshot.mjs /opt/chariox-slice/browser-controller-snapshot.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/browser-controller.mjs /opt/chariox-slice/browser-controller.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/managed-provider-isolation-probe.mjs /opt/chariox-slice/managed-provider-isolation-probe.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/managed-provider-isolation-probe-wrapper.sh /opt/chariox-slice/managed-provider-isolation-probe-wrapper.sh
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/provider-port-bridge.mjs /opt/chariox-slice/provider-port-bridge.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/slice-screen.sh /opt/chariox-slice/slice-screen.sh
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/slice-keyboard.py /opt/chariox-slice/slice-keyboard.py
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/slice-text-finder.py /opt/chariox-slice/slice-text-finder.py
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/start-providers.sh /opt/chariox-slice/start-providers.sh
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/start-runtime.sh /opt/chariox-slice/start-runtime.sh
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/validate-screen.sh /opt/chariox-slice/validate-screen.sh

RUN chmod +x /opt/chariox-slice/browser-cdp.mjs \
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
    /opt/chariox-slice/slice-screen.sh \
    /opt/chariox-slice/start-providers.sh \
    /opt/chariox-slice/start-runtime.sh \
    /opt/chariox-slice/validate-screen.sh

ARG CHARIOX_RELAY_PEER_PROTOCOL_VERSION=unknown
ARG CHARIOX_RUNTIME_SOURCE_REVISION=unknown
LABEL io.chariox.relay-peer-protocol-version="${CHARIOX_RELAY_PEER_PROTOCOL_VERSION}"
LABEL io.chariox.runtime-source-revision="${CHARIOX_RUNTIME_SOURCE_REVISION}"
LABEL io.chariox.debian-snapshot="20260701T000000Z"

RUN printf '%s\n' "${CHARIOX_RUNTIME_SOURCE_REVISION}" > /opt/chariox-slice/runtime-source-revision

USER slice
EXPOSE 43119 43130 43140 43252 6080
WORKDIR /workspace
CMD ["sleep", "infinity"]
