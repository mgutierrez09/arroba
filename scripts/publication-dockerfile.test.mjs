import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import { spawnSync } from "node:child_process"
import { test } from "node:test"

const dockerfile = await readFile(new URL("../docker/publication/Dockerfile", import.meta.url), "utf8")
const egressDockerfile = await readFile(new URL("../docker/publication-egress/Dockerfile", import.meta.url), "utf8")
const kernelTypes = await readFile(new URL("../packages/kernel-client/src/kernel-types.ts", import.meta.url), "utf8")
const kernelCargo = await readFile(new URL("../apps/kernel/Cargo.toml", import.meta.url), "utf8")
const workflowCode = await readFile(new URL("../apps/kernel/src/workflow_code.rs", import.meta.url), "utf8")
const toolchainPackage = JSON.parse(await readFile(
  new URL("../docker/publication/toolchain/package.json", import.meta.url),
  "utf8",
))
const toolchainLock = JSON.parse(await readFile(
  new URL("../docker/publication/toolchain/package-lock.json", import.meta.url),
  "utf8",
))

test("publication image copies compile-time workflow examples before building the kernel", () => {
  assert.match(workflowCode, /include_str!\("\.\.\/\.\.\/\.\.\/examples\/workflow-code\//)

  const rustStageStart = dockerfile.indexOf("FROM rust:1.88-bookworm@sha256:")
  const nextStageStart = dockerfile.indexOf("\nFROM ", rustStageStart + 1)
  assert.notEqual(rustStageStart, -1)
  assert.notEqual(nextStageStart, -1)

  const rustStage = dockerfile.slice(rustStageStart, nextStageStart)
  const examplesCopy = rustStage.indexOf("COPY examples/workflow-code examples/workflow-code")
  const kernelBuild = rustStage.indexOf("RUN cargo build --locked --manifest-path apps/kernel/Cargo.toml")
  assert.ok(examplesCopy >= 0, "the Rust build stage must copy compile-time workflow examples")
  assert.ok(kernelBuild >= 0, "the Rust build stage must compile the kernel")
  assert.ok(examplesCopy < kernelBuild, "compile-time workflow examples must be copied before the kernel build")
})

test("publication Rust build consumes the workspace lock and every kernel path dependency", () => {
  const rustStageStart = dockerfile.indexOf("FROM rust:1.88-bookworm@sha256:")
  const nextStageStart = dockerfile.indexOf("\nFROM ", rustStageStart + 1)
  const rustStage = dockerfile.slice(rustStageStart, nextStageStart)
  const kernelBuild = rustStage.indexOf("RUN cargo build --locked --manifest-path apps/kernel/Cargo.toml")

  assert.ok(kernelBuild >= 0, "the kernel must build with the committed Cargo.lock")
  for (const requiredCopy of [
    "COPY Cargo.toml Cargo.lock ./",
    "COPY apps/relay apps/relay",
    "COPY packages/event-protocol packages/event-protocol",
  ]) {
    const copy = rustStage.indexOf(requiredCopy)
    assert.ok(copy >= 0, `the Rust stage must include ${requiredCopy}`)
    assert.ok(copy < kernelBuild, `${requiredCopy} must happen before the kernel build`)
  }

  const kernelPathDependencies = [...kernelCargo.matchAll(/^\s*[\w-]+\s*=\s*\{[^\n}]*path\s*=\s*"([^"]+)"/gm)]
    .map((match) => match[1])
  assert.deepEqual(kernelPathDependencies.sort(), ["../../packages/event-protocol", "../relay"])
  assert.match(
    rustStage,
    /test "\$\(target\/release\/chariox-kernel --print-local-daemon-protocol-version\)"/,
    "the build must execute the kernel from Cargo's workspace target directory",
  )
  assert.match(
    dockerfile,
    /COPY --from=rust-builder \/opt\/chariox\/target\/release\/chariox-kernel \/usr\/local\/bin\/chariox-kernel/,
    "the runtime image must copy the kernel from Cargo's workspace target directory",
  )
  assert.doesNotMatch(dockerfile, /apps\/kernel\/target\/release\/chariox-kernel/)
})

test("publication images pin every base image by immutable digest", () => {
  const publicationBases = (dockerfile.match(/^FROM\s+\S+/gm) ?? [])
    .filter((base) => base !== "FROM js-toolchain")
  assert.equal(publicationBases.length, 4)
  for (const base of publicationBases) {
    assert.match(base, /@sha256:[0-9a-f]{64}$/)
  }
  assert.match(egressDockerfile, /^FROM node:22-bookworm-slim@sha256:[0-9a-f]{64}$/m)
})

test("publication image reserves isolated credential, action, and gateway identities", () => {
  assert.match(dockerfile, /useradd --create-home --uid 1001 .* chariox/)
  assert.match(dockerfile, /useradd --create-home --uid 1002 .* chariox-action/)
  assert.match(dockerfile, /useradd --create-home --uid 1003 .* chariox-gateway/)
  assert.match(dockerfile, /chown -R root:root \/opt\/chariox/)
  assert.match(dockerfile, /chmod -R go-w \/opt\/chariox/)
  assert.doesNotMatch(dockerfile, /chown -R chariox:chariox \/opt\/chariox/)
  assert.match(dockerfile, /chmod 700 \/home\/chariox \/home\/chariox-action \/home\/chariox-gateway/)
  assert.match(dockerfile, /WORKDIR \/workspace/)
  assert.match(dockerfile, /mkdir -p \/var\/lib\/chariox/)
  assert.match(dockerfile, /chmod 755 \/var\/lib\/chariox/)
  assert.match(dockerfile, /ENTRYPOINT \["tini", "--", "chariox-publication-container"\]/)
  assert.doesNotMatch(dockerfile, /^USER\s+/m, "PID 1 must retain only the root bootstrap needed to prepare isolated role state")
})

test("publication image pins and verifies every official provider CLI", () => {
  assert.match(dockerfile, /ARG CHARIOX_CODEX_VERSION=\d+\.\d+\.\d+/)
  assert.match(dockerfile, /ARG CHARIOX_OPENCODE_VERSION=\d+\.\d+\.\d+/)
  assert.match(dockerfile, /ARG CHARIOX_CLAUDE_VERSION=\d+\.\d+\.\d+/)
  assert.equal(toolchainPackage.dependencies["@openai/codex"], "0.144.0")
  assert.equal(toolchainPackage.dependencies["opencode-ai"], "1.18.23")
  assert.equal(toolchainPackage.dependencies["@anthropic-ai/claude-code"], "2.1.212")
  assert.equal(toolchainPackage.dependencies.pnpm, "9.15.0")
  assert.match(dockerfile, /npm ci --omit=dev/)
  assert.match(dockerfile, /npm sbom --sbom-format cyclonedx/)
  assert.match(dockerfile, /test "\$\(codex --version\)" = "codex-cli \$\{CHARIOX_CODEX_VERSION\}"/)
  assert.match(dockerfile, /test "\$\(opencode --version\)" = "\$\{CHARIOX_OPENCODE_VERSION\}"/)
  assert.match(dockerfile, /test "\$\(claude --version\)" = "\$\{CHARIOX_CLAUDE_VERSION\} \(Claude Code\)"/)
  assert.doesNotMatch(dockerfile, /RUN npm install|corepack prepare/)
})

test("publication network installs are integrity-locked and snapshot-bound", () => {
  const caStage = dockerfile.match(
    /^FROM node:22\.17\.1-bookworm@sha256:[0-9a-f]{64} AS ca-bundle\nRUN echo '([0-9a-f]{64})  \/etc\/ssl\/certs\/ca-certificates\.crt' \| sha256sum --check --strict$/m,
  )
  assert.ok(caStage, "the CA donor must be digest-pinned and its bundle hash verified")
  assert.equal(caStage[1], "a3413a37a8e09cc21b2c11c9ffb23d92d2fc9d1933c9e7617f5c4fba4f72d37d")
  assert.equal(
    (dockerfile.match(/COPY --from=ca-bundle \/etc\/ssl\/certs\/ca-certificates\.crt \/etc\/ssl\/certs\/ca-certificates\.crt/g) ?? []).length,
    2,
  )

  for (const stageName of ["rust-builder", null]) {
    const stageStart = stageName
      ? dockerfile.indexOf(` AS ${stageName}`)
      : dockerfile.lastIndexOf("\nFROM node:22-bookworm-slim@sha256:")
    const nextStage = dockerfile.indexOf("\nFROM ", stageStart + 1)
    const stage = dockerfile.slice(stageStart, nextStage < 0 ? undefined : nextStage)
    const caCopy = stage.indexOf("COPY --from=ca-bundle /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt")
    const aptUpdate = stage.indexOf("apt-get -o Acquire::Check-Valid-Until=false update")
    assert.ok(caCopy >= 0, `${stageName ?? "final"} stage must import the verified CA bundle`)
    assert.ok(aptUpdate >= 0, `${stageName ?? "final"} stage must update from the Debian snapshot`)
    assert.ok(caCopy < aptUpdate, `${stageName ?? "final"} stage must import CA trust before HTTPS APT`)
  }

  assert.equal((dockerfile.match(/snapshot\.debian\.org\/archive\/debian\/20250701T000000Z/g) ?? []).length, 2)
  for (const [path, entry] of Object.entries(toolchainLock.packages)) {
    if (!path || entry.link) continue
    assert.match(entry.resolved ?? "", /^https:\/\/registry\.npmjs\.org\//, `${path} needs a registry artifact`)
    assert.match(entry.integrity ?? "", /^sha512-/, `${path} needs SHA-512 integrity`)
  }
})

test("embedded toolchain SBOM removes volatile identity and time fields", () => {
  const script = new URL("../docker/publication/toolchain/canonicalize-sbom.mjs", import.meta.url)
  const generate = (serialNumber, timestamp) => spawnSync(
    process.execPath,
    [script.pathname],
    {
      encoding: "utf8",
      input: JSON.stringify({
        serialNumber,
        metadata: { timestamp, component: { version: "1", name: "toolchain" } },
        components: [{ version: "0.144.0", name: "codex" }],
        bomFormat: "CycloneDX",
      }),
    },
  )
  const first = generate("urn:uuid:first", "2026-08-18T00:00:00.000Z")
  const second = generate("urn:uuid:second", "2027-01-01T00:00:00.000Z")
  assert.equal(first.status, 0, first.stderr)
  assert.equal(second.status, 0, second.stderr)
  assert.equal(first.stdout, second.stdout)
  assert.doesNotMatch(first.stdout, /serialNumber|timestamp|uuid/)
})

test("publication image labels the protocol version verified against its kernel", () => {
  const protocolVersion = kernelTypes.match(/LOCAL_DAEMON_PROTOCOL_VERSION\s*=\s*(\d+)/)?.[1]
  assert.ok(protocolVersion, "the shared kernel client protocol version must be readable")
  const protocolDefaults = [...dockerfile.matchAll(/^ARG CHARIOX_LOCAL_DAEMON_PROTOCOL_VERSION=(\d+)$/gm)]
    .map((match) => match[1])
  assert.deepEqual(
    protocolDefaults,
    [protocolVersion, protocolVersion],
    "both the Rust build check and runtime label must use the shared protocol version",
  )
  assert.match(
    dockerfile,
    /chariox-kernel --print-local-daemon-protocol-version\)" = "\$\{CHARIOX_LOCAL_DAEMON_PROTOCOL_VERSION\}"/,
  )
  assert.match(
    dockerfile,
    /LABEL dev\.chariox\.local-daemon-protocol-version="\$\{CHARIOX_LOCAL_DAEMON_PROTOCOL_VERSION\}"/,
  )
})

test("publication egress image runs only the dedicated unprivileged gateway", () => {
  assert.match(egressDockerfile, /USER 10001:10001/)
  assert.match(egressDockerfile, /ENTRYPOINT \["node", "\/opt\/chariox-egress\/gateway\.mjs"\]/)
  assert.match(
    egressDockerfile,
    /chmod -R a\+rX,go-w \/opt\/chariox-egress/,
    "the unprivileged gateway must remain readable when the build context was checked out under umask 077",
  )
  assert.doesNotMatch(egressDockerfile, /COPY apps|COPY packages|COPY \. \/|npm install/)
})
