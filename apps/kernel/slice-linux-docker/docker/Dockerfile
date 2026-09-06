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
RUN if [ "$CHARIOX_PREBUILT_RUNTIME" = "1" ]; then \
      test -x /opt/chariox-prebuilt/chariox-kernel \
      && test -x /opt/chariox-prebuilt/chariox-relay \
      && mkdir -p target/release \
      && install -m 0755 /opt/chariox-prebuilt/chariox-kernel target/release/chariox-kernel \
      && install -m 0755 /opt/chariox-prebuilt/chariox-relay target/release/chariox-relay; \
    else \
      CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 cargo build --locked --release --package chariox-kernel --bin chariox-kernel --bin chariox-managed-bootstrap \
      && CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 cargo build --locked --release --package chariox-relay --bin chariox-relay; \
    fi

FROM node:22-bookworm-slim@sha256:d649c27dae7ba0137b3cef5dd75baa422c08dc3d9e3fc0c23dfb172dc3cc6436

ENV DEBIAN_FRONTEND=noninteractive
ENV CHARIOX_SLICE_ROOT=/opt/chariox-slice

COPY --from=ca-bundle /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
RUN printf '%s\n' 'deb [check-valid-until=no] https://snapshot.debian.org/archive/debian/20260701T000000Z bookworm main' > /etc/apt/sources.list \
  && rm -f /etc/apt/sources.list.d/debian.sources \
  && apt-get -o Acquire::Check-Valid-Until=false update \
  && apt-get install -y --no-install-recommends \
    bash \
    bubblewrap \
    build-essential \
    ca-certificates \
    chromium \
    chromium-driver \
    chromium-sandbox \
    curl \
    fonts-dejavu \
    fonts-liberation \
    gh \
    git \
    jq \
    libdbus-1-dev \
    libssl-dev \
    lsof \
    novnc \
    openbox \
    pkg-config \
    procps \
    protobuf-compiler \
    screen \
    scrot \
    socat \
    tesseract-ocr \
    tesseract-ocr-eng \
    unzip \
    websockify \
    x11-utils \
    x11vnc \
    xclip \
    xdotool \
    xvfb \
    zstd \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /opt/chariox-toolchain
COPY apps/kernel/slice-linux-docker/toolchain/package.json apps/kernel/slice-linux-docker/toolchain/package-lock.json ./
RUN npm_config_cache=/tmp/chariox-npm-cache npm ci --omit=dev \
  && test "$(node_modules/.bin/codex --version)" = "codex-cli 0.144.0" \
  && test "$(node_modules/.bin/opencode --version)" = "1.18.23" \
  && test "$(node_modules/.bin/claude --version)" = "2.1.212 (Claude Code)" \
  && test "$(node_modules/.bin/pnpm --version)" = "11.22.0" \
  && rm -rf /tmp/chariox-npm-cache /root/.npm
ENV PATH=/opt/chariox-toolchain/node_modules/.bin:/usr/local/bin:/usr/local/sbin:/usr/sbin:/usr/bin:/sbin:/bin

RUN groupadd --gid 1001 slice \
  && useradd --uid 1001 --gid 1001 -m -s /bin/bash slice \
  && mkdir -p /opt/chariox-slice/bin /opt/chariox-slice/logs \
  && chown -R slice:slice /opt/chariox-slice

COPY --from=rust-builder --chown=slice:slice /opt/chariox-source/target/release/chariox-kernel /opt/chariox-slice/bin/chariox-kernel
COPY --from=rust-builder --chown=slice:slice /opt/chariox-source/target/release/chariox-relay /opt/chariox-slice/bin/chariox-relay
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/browser-cdp.mjs /opt/chariox-slice/browser-cdp.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/managed-provider-isolation-probe.mjs /opt/chariox-slice/managed-provider-isolation-probe.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/managed-provider-isolation-probe-wrapper.sh /opt/chariox-slice/managed-provider-isolation-probe-wrapper.sh
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/provider-port-bridge.mjs /opt/chariox-slice/provider-port-bridge.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/slice-screen.sh /opt/chariox-slice/slice-screen.sh
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/start-providers.sh /opt/chariox-slice/start-providers.sh
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/start-runtime.sh /opt/chariox-slice/start-runtime.sh
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/validate-screen.sh /opt/chariox-slice/validate-screen.sh

RUN chmod +x /opt/chariox-slice/browser-cdp.mjs /opt/chariox-slice/managed-provider-isolation-probe.mjs /opt/chariox-slice/managed-provider-isolation-probe-wrapper.sh /opt/chariox-slice/provider-port-bridge.mjs /opt/chariox-slice/slice-screen.sh /opt/chariox-slice/start-providers.sh /opt/chariox-slice/start-runtime.sh /opt/chariox-slice/validate-screen.sh

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
