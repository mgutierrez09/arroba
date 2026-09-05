# syntax=docker/dockerfile:1

FROM node:22.17.1-bookworm@sha256:37ff334612f77d8f999c10af8797727b731629c26f2e83caa6af390998bdc49c AS ca-bundle
RUN echo 'a3413a37a8e09cc21b2c11c9ffb23d92d2fc9d1933c9e7617f5c4fba4f72d37d  /etc/ssl/certs/ca-certificates.crt' | sha256sum --check --strict

FROM node:22-bookworm-slim@sha256:d649c27dae7ba0137b3cef5dd75baa422c08dc3d9e3fc0c23dfb172dc3cc6436 AS js-toolchain
WORKDIR /opt/chariox-toolchain
COPY docker/publication/toolchain/package.json docker/publication/toolchain/package-lock.json ./
COPY docker/publication/toolchain/canonicalize-sbom.mjs ./
RUN npm ci --omit=dev \
  && npm sbom --sbom-format cyclonedx | node canonicalize-sbom.mjs > toolchain-sbom.cdx.json \
  && test "$(node_modules/.bin/pnpm --version)" = "9.15.0" \
  && test "$(node_modules/.bin/codex --version)" = "codex-cli 0.144.5" \
  && test "$(node_modules/.bin/opencode --version)" = "1.18.23" \
  && test "$(node_modules/.bin/claude --version)" = "2.1.207 (Claude Code)"

FROM js-toolchain AS js-builder
ENV PATH=/opt/chariox-toolchain/node_modules/.bin:/usr/local/bin:/usr/local/sbin:/usr/sbin:/usr/bin:/sbin:/bin
WORKDIR /opt/chariox
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml tsconfig.base.json ./
COPY packages packages
COPY apps/server/package.json apps/server/package.json
RUN pnpm install --frozen-lockfile
COPY apps/server apps/server
RUN pnpm --filter @chariox/tool-display run build \
  && pnpm --filter @chariox/server run build

FROM rust:1.88-bookworm@sha256:af306cfa71d987911a781c37b59d7d67d934f49684058f96cf72079c3626bfe0 AS rust-builder
ARG CHARIOX_LOCAL_DAEMON_PROTOCOL_VERSION=306
WORKDIR /opt/chariox
COPY --from=ca-bundle /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
RUN printf '%s\n' 'deb [check-valid-until=no] https://snapshot.debian.org/archive/debian/20250701T000000Z bookworm main' > /etc/apt/sources.list \
  && rm -f /etc/apt/sources.list.d/debian.sources \
  && apt-get -o Acquire::Check-Valid-Until=false update \
  && apt-get install -y --no-install-recommends \
    libdbus-1-dev \
    pkg-config \
  && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY adapters/rust adapters/rust
COPY apps/aegs-dummy apps/aegs-dummy
COPY apps/kernel apps/kernel
COPY apps/relay apps/relay
COPY examples/workflow-code examples/workflow-code
COPY packages/aegs-sdk packages/aegs-sdk
COPY packages/event-protocol packages/event-protocol
RUN cargo build --locked --manifest-path apps/kernel/Cargo.toml --release --bin chariox-kernel \
  && test "$(target/release/chariox-kernel --print-local-daemon-protocol-version)" = "${CHARIOX_LOCAL_DAEMON_PROTOCOL_VERSION}"

FROM node:22-bookworm-slim@sha256:d649c27dae7ba0137b3cef5dd75baa422c08dc3d9e3fc0c23dfb172dc3cc6436
LABEL org.opencontainers.image.title="Chariox workflow publication"
LABEL org.opencontainers.image.description="Standalone Chariox kernel plus workflow publication gateway runtime."
LABEL dev.chariox.debian-snapshot="20250701T000000Z"

ARG CHARIOX_CODEX_VERSION=0.144.5
ARG CHARIOX_OPENCODE_VERSION=1.18.23
ARG CHARIOX_CLAUDE_VERSION=2.1.207
ARG CHARIOX_LOCAL_DAEMON_PROTOCOL_VERSION=306
LABEL dev.chariox.provider.codex.version="${CHARIOX_CODEX_VERSION}"
LABEL dev.chariox.provider.opencode.version="${CHARIOX_OPENCODE_VERSION}"
LABEL dev.chariox.provider.claude.version="${CHARIOX_CLAUDE_VERSION}"
LABEL dev.chariox.local-daemon-protocol-version="${CHARIOX_LOCAL_DAEMON_PROTOCOL_VERSION}"

ENV DEBIAN_FRONTEND=noninteractive
COPY --from=ca-bundle /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
RUN printf '%s\n' 'deb [check-valid-until=no] https://snapshot.debian.org/archive/debian/20250701T000000Z bookworm main' > /etc/apt/sources.list \
  && rm -f /etc/apt/sources.list.d/debian.sources \
  && apt-get -o Acquire::Check-Valid-Until=false update \
  && apt-get install -y --no-install-recommends \
    bash \
    ca-certificates \
    libdbus-1-3 \
    tini \
  && rm -rf /var/lib/apt/lists/*

COPY --from=js-toolchain /opt/chariox-toolchain /opt/chariox-toolchain
ENV PATH=/opt/chariox-toolchain/node_modules/.bin:/usr/local/bin:/usr/local/sbin:/usr/sbin:/usr/bin:/sbin:/bin
RUN test "$(codex --version)" = "codex-cli ${CHARIOX_CODEX_VERSION}" \
  && test "$(opencode --version)" = "${CHARIOX_OPENCODE_VERSION}" \
  && test "$(claude --version)" = "${CHARIOX_CLAUDE_VERSION} (Claude Code)"

RUN useradd --create-home --uid 1001 --shell /bin/bash chariox \
  && useradd --create-home --uid 1002 --shell /usr/sbin/nologin chariox-action \
  && useradd --create-home --uid 1003 --shell /usr/sbin/nologin chariox-gateway

WORKDIR /opt/chariox
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml tsconfig.base.json ./
COPY --from=js-builder /opt/chariox/node_modules ./node_modules
COPY --from=js-builder /opt/chariox/packages ./packages
COPY --from=js-builder /opt/chariox/apps/server ./apps/server
COPY --from=rust-builder /opt/chariox/target/release/chariox-kernel /usr/local/bin/chariox-kernel
COPY docker/publication/entrypoint.sh /usr/local/bin/chariox-publication-container
COPY docker/publication/wait-for-tcp.mjs /usr/local/bin/chariox-wait-for-tcp.mjs
RUN chmod +x /usr/local/bin/chariox-publication-container /usr/local/bin/chariox-wait-for-tcp.mjs \
  && mkdir -p /var/lib/chariox \
  && chmod 755 /var/lib/chariox \
  && chown -R root:root /opt/chariox \
  && chmod -R go-w /opt/chariox \
  && chown -R chariox:chariox /home/chariox \
  && chown -R chariox-action:chariox-action /home/chariox-action \
  && chown -R chariox-gateway:chariox-gateway /home/chariox-gateway \
  && chmod 700 /home/chariox /home/chariox-action /home/chariox-gateway

WORKDIR /workspace
ENV HOME=/home/chariox
ENV CHARIOX_CONFIG_DIR=/home/chariox/.config/chariox
ENV CHARIOX_DATA_DIR=/home/chariox/.local/share/chariox
ENV CHARIOX_RUNTIME_DIR=/home/chariox/.cache/chariox/runtime
ENV CHARIOX_SESSION_HISTORY_DIR=/home/chariox/.local/share/chariox/sessions
ENV CHARIOX_KERNEL_HOST=127.0.0.1
ENV CHARIOX_KERNEL_PORT=43118
ENV HOST=0.0.0.0
ENV PORT=3000

EXPOSE 3000 43118
ENTRYPOINT ["tini", "--", "chariox-publication-container"]
CMD ["standalone"]
