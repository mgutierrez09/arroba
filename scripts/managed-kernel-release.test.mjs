import assert from "node:assert/strict"
import { createHash, generateKeyPairSync, sign, verify } from "node:crypto"
import { chmod, lstat, mkdir, mkdtemp, readFile, readdir, readlink, rename, rm, stat, symlink, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join, relative, sep } from "node:path"
import { fileURLToPath } from "node:url"
import { spawn, spawnSync } from "node:child_process"
import { once } from "node:events"
import { test } from "node:test"

const repositoryRoot = fileURLToPath(new URL("..", import.meta.url))
const packager = join(repositoryRoot, "scripts/package-managed-kernel-release.mjs")
const builder = join(repositoryRoot, "scripts/build-managed-kernel-release.mjs")
const installer = join(repositoryRoot, "deploy/managed-kernel/install-image.sh")
const verifier = join(repositoryRoot, "deploy/managed-kernel/verify-image-release.mjs")
const service = join(repositoryRoot, "deploy/managed-kernel/chariox-managed-bootstrap.service")
const rootlessDockerService = join(repositoryRoot, "deploy/managed-kernel/chariox-rootless-docker.service")
const sliceBrokerService = join(repositoryRoot, "deploy/managed-kernel/chariox-slice-broker.service")
const sourceDateEpoch = "946684800"

test("managed prebuilt slice runtime materializes its runtime output directory", async () => {
  const dockerfile = await readFile(
    join(repositoryRoot, "apps/kernel/slice-linux-docker/docker/Dockerfile"),
    "utf8",
  )
  const start = dockerfile.indexOf("RUN mkdir -p /opt/chariox-runtime-bin")
  const end = dockerfile.indexOf("\n\nFROM ", start)
  assert.notEqual(start, -1, "prebuilt runtime branch is missing")
  assert.notEqual(end, -1, "prebuilt runtime branch has no stage boundary")

  const fixture = await mkdtemp(join(tmpdir(), "chariox-prebuilt-slice-"))
  const prebuilt = join(fixture, "prebuilt")
  const runtimeBin = join(fixture, "runtime-bin")
  try {
    await mkdir(prebuilt, { recursive: true })
    for (const name of ["chariox-kernel", "chariox-relay"]) {
      const path = join(prebuilt, name)
      await writeFile(path, `${name}\n`)
      await chmod(path, 0o755)
    }
    const command = dockerfile
      .slice(start + "RUN ".length, end)
      .replaceAll("\\\n", " ")
      .replaceAll("/opt/chariox-prebuilt", prebuilt)
      .replaceAll("/opt/chariox-runtime-bin", runtimeBin)
    const result = spawnSync("/bin/sh", ["-c", command], {
      cwd: fixture,
      encoding: "utf8",
      env: { ...process.env, CHARIOX_PREBUILT_RUNTIME: "1" },
    })
    assert.equal(result.status, 0, result.stderr)
    assert.equal(await readFile(join(runtimeBin, "chariox-kernel"), "utf8"), "chariox-kernel\n")
    assert.equal(await readFile(join(runtimeBin, "chariox-relay"), "utf8"), "chariox-relay\n")
  } finally {
    await rm(fixture, { recursive: true, force: true })
  }
})

async function waitForPath(path, timeoutMs = 5000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if (await lstat(path).then(() => true, () => false)) return
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 20))
  }
  throw new Error(`timed out waiting for ${path}`)
}

async function withTimeout(promise, message, timeoutMs = 10_000) {
  let timer
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error(message)), timeoutMs)
      }),
    ])
  } finally {
    clearTimeout(timer)
  }
}

function rawPublicKey(publicKey) {
  const der = publicKey.export({ format: "der", type: "spki" })
  return der.subarray(der.length - 32)
}

function packagerArguments({
  kernel,
  supervisor,
  relay,
  builderAttestation,
  builderAttestationSignature,
  trustedBuilderPublicKey,
  signingKey,
  sourceRepository,
  sourceCommit,
  output,
}) {
  return [
    packager,
    "--kernel", kernel,
    "--supervisor", supervisor,
    "--relay", relay,
    "--builder-attestation", builderAttestation,
    "--builder-attestation-signature", builderAttestationSignature,
    "--trusted-builder-public-key", trustedBuilderPublicKey,
    "--signing-key", signingKey,
    "--source-repository", sourceRepository,
    "--source-commit", sourceCommit,
    "--output", output,
  ]
}

function runPackager(options, umask = "022", extraEnvironment = {}) {
  return spawnSync(
    "/bin/sh",
    ["-c", 'umask "$1"; shift; exec "$@"', "chariox-release-packager", umask, process.execPath, ...packagerArguments(options)],
    { encoding: "utf8", env: { ...process.env, ...extraEnvironment, SOURCE_DATE_EPOCH: sourceDateEpoch } },
  )
}

function runVerifier(rootfs, digest, trustedPublicKey) {
  return spawnSync(process.execPath, [verifier, rootfs, digest, trustedPublicKey], { encoding: "utf8" })
}

async function snapshotTree(root, current = root) {
  const metadata = await lstat(current)
  const record = {
    path: relative(root, current) || ".",
    mode: metadata.mode & 0o777,
    mtimeMs: metadata.mtimeMs,
    type: metadata.isDirectory() ? "directory" : metadata.isFile() ? "file" : "unsupported",
  }
  if (metadata.isFile()) record.sha256 = createHash("sha256").update(await readFile(current)).digest("hex")
  const records = [record]
  if (metadata.isDirectory()) {
    for (const name of (await readdir(current)).sort()) records.push(...(await snapshotTree(root, join(current, name))))
  }
  return records
}

async function updateTreeHash(root, current, hash) {
  for (const name of (await readdir(current)).sort()) {
    const path = join(current, name)
    const metadata = await lstat(path)
    const pathFromRoot = relative(root, path).split(sep).join("/")
    const mode = metadata.mode & 0o7777
    if (metadata.isDirectory()) {
      hash.update(`directory:${Buffer.byteLength(pathFromRoot)}:${pathFromRoot}:${mode}:`)
      await updateTreeHash(root, path, hash)
      continue
    }
    hash.update(`file:${Buffer.byteLength(pathFromRoot)}:${pathFromRoot}:${mode}:${metadata.size}:`)
    hash.update(await readFile(path))
  }
}

async function treeDigest(root) {
  const hash = createHash("sha256")
  const metadata = await lstat(root)
  hash.update(`directory:1:.:${metadata.mode & 0o7777}:`)
  await updateTreeHash(root, root, hash)
  return `sha256:${hash.digest("hex")}`
}

async function makeFixture(root, variant = "") {
  const kernel = join(root, "chariox-kernel")
  const supervisor = join(root, "chariox-managed-bootstrap")
  const relay = join(root, "chariox-relay")
  const signingKey = join(root, "release-key.pem")
  const trustedPublicKey = join(root, "trusted-release-public-key")
  const kernelContents = variant ? `kernel fixture ${variant}\n` : "kernel fixture\n"
  const relayContents = variant ? `relay fixture ${variant}\n` : "relay fixture\n"
  await writeFile(kernel, kernelContents, { mode: 0o755 })
  await writeFile(supervisor, "supervisor fixture\n", { mode: 0o755 })
  await writeFile(relay, relayContents, { mode: 0o755 })
  const { privateKey, publicKey } = generateKeyPairSync("ed25519")
  await writeFile(signingKey, privateKey.export({ format: "pem", type: "pkcs8" }), { mode: 0o600 })
  await writeFile(trustedPublicKey, rawPublicKey(publicKey).toString("base64"), { mode: 0o600 })
  const sourceRepository = join(root, "source")
  await mkdir(sourceRepository, { recursive: true })
  const sourceFiles = new Map([
    ["Cargo.toml", "[workspace]\nmembers = []\n"],
    ["Cargo.lock", "version = 4\n"],
    ["adapters/rust/Cargo.toml", "[package]\nname = \"adapter-fixture\"\n"],
    ["apps/aegs-dummy/Cargo.toml", "[package]\nname = \"aegs-fixture\"\n"],
    ["apps/kernel/Cargo.toml", "[package]\nname = \"kernel-fixture\"\n"],
    ["apps/kernel/slice-linux-docker/docker/Dockerfile", "FROM fixture@sha256:0000000000000000000000000000000000000000000000000000000000000000\n"],
    [
      "apps/kernel/slice-linux-docker/managed-docker-broker.mjs",
      await readFile(join(repositoryRoot, "apps/kernel/slice-linux-docker/managed-docker-broker.mjs")),
    ],
    ["apps/kernel/slice-linux-docker/enter-rootless-docker-namespace.sh", "#!/bin/sh\nexec \"$@\"\n"],
    ["apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh", "#!/bin/sh\nSLICE_BUILD_IMAGE=fixture\n"],
    ["apps/kernel/slice-linux-docker/managed-publication-access.sh", "#!/bin/sh\nexit 0\n"],
    ["apps/kernel/slice-linux-docker/toolchain/package-lock.json", "{\"lockfileVersion\":3}\n"],
    ["apps/kernel/src/transport/relay_peer.rs", "pub const RELAY_PEER_PROTOCOL_VERSION: u32 = 1;\n"],
    ["apps/relay/Cargo.toml", "[package]\nname = \"relay-fixture\"\n"],
    ["deploy/managed-kernel/chariox-managed-bootstrap.service", await readFile(service)],
    ["deploy/managed-kernel/chariox-rootless-docker.service", await readFile(rootlessDockerService)],
    ["deploy/managed-kernel/chariox-slice-broker.service", await readFile(sliceBrokerService)],
    ["examples/workflow-code/example.md", "workflow fixture\n"],
    ["packages/aegs-sdk/Cargo.toml", "[package]\nname = \"sdk-fixture\"\n"],
    ["packages/event-protocol/Cargo.toml", "[package]\nname = \"event-fixture\"\n"],
  ])
  if (variant) sourceFiles.set("release-variant.txt", `${variant}\n`)
  for (const [path, contents] of sourceFiles) {
    const destination = join(sourceRepository, path)
    await mkdir(join(destination, ".."), { recursive: true })
    await writeFile(destination, contents, {
      mode: path.endsWith("enter-rootless-docker-namespace.sh") || path.endsWith("provision-linux-docker-slice.sh") || path.endsWith("managed-publication-access.sh") ? 0o755 : 0o644,
    })
  }
  const git = (args) => spawnSync("git", args, {
    cwd: sourceRepository,
    encoding: "utf8",
    env: {
      ...process.env,
      GIT_AUTHOR_NAME: "Release Fixture",
      GIT_AUTHOR_EMAIL: "release@example.invalid",
      GIT_AUTHOR_DATE: "2000-01-01T00:00:00Z",
      GIT_COMMITTER_NAME: "Release Fixture",
      GIT_COMMITTER_EMAIL: "release@example.invalid",
      GIT_COMMITTER_DATE: "2000-01-01T00:00:00Z",
    },
  })
  assert.equal(git(["init", "-q"]).status, 0)
  assert.equal(git(["add", "."]).status, 0)
  const committed = git(["commit", "-q", "-m", "fixture"])
  assert.equal(committed.status, 0, committed.stderr)
  const sourceCommit = git(["rev-parse", "HEAD"]).stdout.trim()
  const sourceTree = git(["rev-parse", "HEAD^{tree}"]).stdout.trim()
  const builderAttestation = join(root, "build-attestation.json")
  const builderAttestationSignature = join(root, "build-attestation.sig")
  const trustedBuilderPublicKey = join(root, "trusted-builder-public-key")
  const builderKeys = generateKeyPairSync("ed25519")
  const binaryDigest = (path) => `sha256:${createHash("sha256").update(path).digest("hex")}`
  const attestationBytes = Buffer.from(JSON.stringify({
    schemaVersion: 1,
    sourceCommit,
    sourceTree,
    target: "x86_64-unknown-linux-gnu",
    artifacts: [
      { name: "chariox-kernel", sha256: binaryDigest(kernelContents) },
      { name: "chariox-managed-bootstrap", sha256: binaryDigest("supervisor fixture\n") },
      { name: "chariox-relay", sha256: binaryDigest(relayContents) },
    ],
  }))
  await writeFile(builderAttestation, attestationBytes, { mode: 0o644 })
  await writeFile(
    builderAttestationSignature,
    sign(null, attestationBytes, builderKeys.privateKey).toString("base64"),
    { mode: 0o644 },
  )
  await writeFile(trustedBuilderPublicKey, rawPublicKey(builderKeys.publicKey).toString("base64"), {
    mode: 0o600,
  })
  return {
    kernel,
    kernelContents,
    supervisor,
    relay,
    relayContents,
    signingKey,
    trustedPublicKey,
    builderAttestation,
    builderAttestationSignature,
    trustedBuilderPublicKey,
    builderPublicKey: builderKeys.publicKey,
    builderPrivateKey: builderKeys.privateKey,
    publicKey,
    sourceRepository,
    sourceCommit,
    sourceTree,
    serviceBytes: sourceFiles.get("deploy/managed-kernel/chariox-managed-bootstrap.service"),
    rootlessDockerServiceBytes: sourceFiles.get("deploy/managed-kernel/chariox-rootless-docker.service"),
    sliceBrokerServiceBytes: sourceFiles.get("deploy/managed-kernel/chariox-slice-broker.service"),
  }
}

test("managed kernel release packages one reproducible signed rootfs", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-release-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  const firstOutput = join(root, "release-one")
  const secondOutput = join(root, "release-two")
  const first = runPackager({ ...fixture, output: firstOutput }, "022")
  await writeFile(
    join(fixture.sourceRepository, "apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh"),
    "working tree drift must not be packaged\n",
  )
  const second = runPackager({ ...fixture, output: secondOutput }, "077")
  assert.equal(first.status, 0, first.stderr)
  assert.equal(second.status, 0, second.stderr)
  assert.match(first.stdout, /^sha256:[a-f0-9]{64}\n$/)
  assert.equal(second.stdout, first.stdout)
  assert.equal(first.stderr, "")

  const releaseRoot = join(firstOutput, "rootfs")
  const secondReleaseRoot = join(secondOutput, "rootfs")
  assert.deepEqual(await snapshotTree(releaseRoot), await snapshotTree(secondReleaseRoot))
  const snapshot = await snapshotTree(releaseRoot)
  assert.equal(snapshot.every((entry) => entry.mtimeMs === Number(sourceDateEpoch) * 1000), true)
  assert.equal(snapshot.every((entry) => entry.type !== "unsupported"), true)
  const packagedPaths = snapshot.filter((entry) => entry.type === "file").map((entry) => entry.path)
  for (const requiredPath of [
    "etc/systemd/system/chariox-managed-bootstrap.service",
    "etc/systemd/system/chariox-rootless-docker.service",
    "etc/systemd/system/chariox-slice-broker.service",
    "usr/lib/chariox/release-manifest.json",
    "usr/lib/chariox/release-manifest.sig",
    "usr/lib/chariox/release-public-key",
    "usr/lib/chariox/build-attestation.json",
    "usr/lib/chariox/build-attestation.sig",
    "usr/lib/chariox/builder-public-key",
    "usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/enter-rootless-docker-namespace.sh",
    "usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh",
    "usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/managed-publication-access.sh",
    "usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/managed-docker-broker.mjs",
    "usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/prebuilt/.managed-release",
    "usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/prebuilt/chariox-kernel",
    "usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/prebuilt/chariox-relay",
    "usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/toolchain/package-lock.json",
    "usr/lib/chariox/slice-build-context/Cargo.lock",
    "usr/lib/chariox/slice-build-context/apps/kernel/src/transport/relay_peer.rs",
    "usr/lib/chariox/slice-build-context/apps/relay/Cargo.toml",
    "usr/lib/chariox/slice-build-context/packages/event-protocol/Cargo.toml",
    "usr/local/bin/chariox-kernel",
    "usr/local/bin/chariox-managed-bootstrap",
  ]) {
    assert.ok(packagedPaths.includes(requiredPath), `missing packaged path ${requiredPath}`)
  }

  const manifestBytes = await readFile(join(releaseRoot, "usr/lib/chariox/release-manifest.json"))
  const manifest = JSON.parse(manifestBytes)
  const digest = (contents) => `sha256:${createHash("sha256").update(contents).digest("hex")}`
  assert.deepEqual(manifest, {
    schemaVersion: 2,
    sourceCommit: fixture.sourceCommit,
    sourceTree: fixture.sourceTree,
    artifacts: [
      { name: "chariox-kernel", path: "/usr/local/bin/chariox-kernel", sha256: digest("kernel fixture\n") },
      { name: "chariox-managed-bootstrap", path: "/usr/local/bin/chariox-managed-bootstrap", sha256: digest("supervisor fixture\n") },
      {
        name: "chariox-managed-bootstrap.service",
        path: "/etc/systemd/system/chariox-managed-bootstrap.service",
        sha256: digest(fixture.serviceBytes),
      },
      {
        name: "chariox-rootless-docker.service",
        path: "/etc/systemd/system/chariox-rootless-docker.service",
        sha256: digest(fixture.rootlessDockerServiceBytes),
      },
      {
        name: "chariox-slice-broker.service",
        path: "/etc/systemd/system/chariox-slice-broker.service",
        sha256: digest(fixture.sliceBrokerServiceBytes),
      },
      {
        name: "chariox-slice-build-context",
        path: "/usr/lib/chariox/slice-build-context",
        sha256: await treeDigest(join(releaseRoot, "usr/lib/chariox/slice-build-context")),
      },
      {
        name: "chariox-build-attestation",
        path: "/usr/lib/chariox/build-attestation.json",
        sha256: digest(await readFile(fixture.builderAttestation)),
      },
      {
        name: "chariox-build-attestation-signature",
        path: "/usr/lib/chariox/build-attestation.sig",
        sha256: digest((await readFile(fixture.builderAttestationSignature, "utf8")).trim()),
      },
      {
        name: "chariox-builder-public-key",
        path: "/usr/lib/chariox/builder-public-key",
        sha256: digest((await readFile(fixture.trustedBuilderPublicKey, "utf8")).trim()),
      },
    ],
  })
  assert.equal(first.stdout.trim(), digest(manifestBytes))
  const signature = Buffer.from((await readFile(join(releaseRoot, "usr/lib/chariox/release-manifest.sig"), "utf8")).trim(), "base64")
  assert.equal(signature.length, 64)
  assert.equal(verify(null, manifestBytes, fixture.publicKey, signature), true)
  assert.deepEqual(
    Buffer.from((await readFile(join(releaseRoot, "usr/lib/chariox/release-public-key"), "utf8")).trim(), "base64"),
    rawPublicKey(fixture.publicKey),
  )
  assert.deepEqual(
    await readFile(join(releaseRoot, "etc/systemd/system/chariox-managed-bootstrap.service")),
    fixture.serviceBytes,
  )
  assert.match(
    await readFile(
      join(releaseRoot, "usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh"),
      "utf8",
    ),
    /SLICE_BUILD_IMAGE=fixture/,
  )
  assert.equal(
    await readFile(
      join(releaseRoot, "usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/prebuilt/chariox-kernel"),
      "utf8",
    ),
    fixture.kernelContents,
  )
  assert.equal(
    await readFile(
      join(releaseRoot, "usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/prebuilt/chariox-relay"),
      "utf8",
    ),
    fixture.relayContents,
  )
  assert.equal(snapshot.find((entry) => entry.path === "usr/local/bin/chariox-kernel").mode, 0o755)
  assert.equal(snapshot.find((entry) => entry.path === "usr/local/bin/chariox-managed-bootstrap").mode, 0o755)
  assert.equal(
    snapshot.find((entry) => entry.path.endsWith("/enter-rootless-docker-namespace.sh")).mode,
    0o755,
  )
  assert.equal(
    snapshot.find((entry) => entry.path.endsWith("/provision-linux-docker-slice.sh")).mode,
    0o755,
  )
  assert.equal(
    snapshot.find((entry) => entry.path.endsWith("/managed-publication-access.sh")).mode,
    0o755,
  )
  assert.equal(snapshot.find((entry) => entry.path.endsWith("/prebuilt/chariox-kernel")).mode, 0o755)
  assert.equal(snapshot.find((entry) => entry.path.endsWith("/prebuilt/chariox-relay")).mode, 0o755)
  assert.equal(
    snapshot
      .filter((entry) =>
        entry.type === "file" &&
        !entry.path.startsWith("usr/local/bin/") &&
        !entry.path.endsWith("/enter-rootless-docker-namespace.sh") &&
        !entry.path.endsWith("/provision-linux-docker-slice.sh") &&
        !entry.path.endsWith("/managed-publication-access.sh") &&
        !entry.path.endsWith("/prebuilt/chariox-kernel") &&
        !entry.path.endsWith("/prebuilt/chariox-relay"),
      )
      .every((entry) => entry.mode === 0o644),
    true,
  )
  assert.equal(snapshot.some((entry) => entry.path.includes("release-key")), false)
  assert.equal(runVerifier(releaseRoot, first.stdout.trim(), fixture.trustedPublicKey).status, 0)

  const rerun = runPackager({ ...fixture, output: firstOutput })
  assert.equal(rerun.status, 1)
  assert.match(rerun.stderr, /output directory must be empty/)
})

test("release identity rejects unattested binaries and verifier rejects tampering", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-release-binding-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  const originalOutput = join(root, "original")
  const changedOutput = join(root, "changed")
  const original = runPackager({ ...fixture, output: originalOutput })
  assert.equal(original.status, 0, original.stderr)
  await writeFile(fixture.supervisor, "different supervisor\n", { mode: 0o755 })
  const changed = runPackager({ ...fixture, output: changedOutput })
  assert.equal(changed.status, 1)
  assert.match(changed.stderr, /builder attestation artifacts do not match/)
  assert.equal(await lstat(join(changedOutput, "rootfs/usr/lib/chariox/release-manifest.json")).then(() => true, () => false), false)
  await writeFile(fixture.supervisor, "supervisor fixture\n", { mode: 0o755 })

  const originalRoot = join(originalOutput, "rootfs")
  await writeFile(join(originalRoot, "usr/local/bin/chariox-managed-bootstrap"), "tampered\n")
  const supervisorTamper = runVerifier(originalRoot, original.stdout.trim(), fixture.trustedPublicKey)
  assert.equal(supervisorTamper.status, 1)
  assert.match(supervisorTamper.stderr, /chariox-managed-bootstrap is corrupted/)

  const serviceOutput = join(root, "service")
  const serviceRelease = runPackager({ ...fixture, output: serviceOutput })
  assert.equal(serviceRelease.status, 0, serviceRelease.stderr)
  const changedRoot = join(serviceOutput, "rootfs")
  await writeFile(join(changedRoot, "etc/systemd/system/chariox-managed-bootstrap.service"), "tampered service\n")
  const serviceTamper = runVerifier(changedRoot, serviceRelease.stdout.trim(), fixture.trustedPublicKey)
  assert.equal(serviceTamper.status, 1)
  assert.match(serviceTamper.stderr, /chariox-managed-bootstrap\.service is corrupted/)

  const contextOutput = join(root, "context")
  const contextRelease = runPackager({ ...fixture, output: contextOutput })
  assert.equal(contextRelease.status, 0, contextRelease.stderr)
  const contextRoot = join(contextOutput, "rootfs")
  await writeFile(
    join(contextRoot, "usr/lib/chariox/slice-build-context/apps/relay/Cargo.toml"),
    "tampered context\n",
  )
  const contextTamper = runVerifier(
    contextRoot,
    contextRelease.stdout.trim(),
    fixture.trustedPublicKey,
  )
  assert.equal(contextTamper.status, 1)
  assert.match(contextTamper.stderr, /chariox-slice-build-context is corrupted/)

  const emptyDirectoryOutput = join(root, "empty-directory")
  const emptyDirectoryRelease = runPackager({ ...fixture, output: emptyDirectoryOutput })
  assert.equal(emptyDirectoryRelease.status, 0, emptyDirectoryRelease.stderr)
  const emptyDirectoryRoot = join(emptyDirectoryOutput, "rootfs")
  await mkdir(join(emptyDirectoryRoot, "usr/lib/chariox/slice-build-context/unsigned-empty"))
  const emptyDirectoryTamper = runVerifier(
    emptyDirectoryRoot,
    emptyDirectoryRelease.stdout.trim(),
    fixture.trustedPublicKey,
  )
  assert.equal(emptyDirectoryTamper.status, 1)
  assert.match(emptyDirectoryTamper.stderr, /chariox-slice-build-context is corrupted/)

  const rootModeOutput = join(root, "root-mode")
  const rootModeRelease = runPackager({ ...fixture, output: rootModeOutput })
  assert.equal(rootModeRelease.status, 0, rootModeRelease.stderr)
  const rootModeRoot = join(rootModeOutput, "rootfs")
  await chmod(join(rootModeRoot, "usr/lib/chariox/slice-build-context"), 0o700)
  const rootModeTamper = runVerifier(rootModeRoot, rootModeRelease.stdout.trim(), fixture.trustedPublicKey)
  assert.equal(rootModeTamper.status, 1)
  assert.match(rootModeTamper.stderr, /chariox-slice-build-context is corrupted/)

  const nestedModeOutput = join(root, "nested-mode")
  const nestedModeRelease = runPackager({ ...fixture, output: nestedModeOutput })
  assert.equal(nestedModeRelease.status, 0, nestedModeRelease.stderr)
  const nestedModeRoot = join(nestedModeOutput, "rootfs")
  await chmod(join(nestedModeRoot, "usr/lib/chariox/slice-build-context/apps"), 0o750)
  const nestedModeTamper = runVerifier(
    nestedModeRoot,
    nestedModeRelease.stdout.trim(),
    fixture.trustedPublicKey,
  )
  assert.equal(nestedModeTamper.status, 1)
  assert.match(nestedModeTamper.stderr, /chariox-slice-build-context is corrupted/)
})

test("release verifier rejects root and key aliases into the packaged image", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-release-alias-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  const output = join(root, "release")
  const packaged = runPackager({ ...fixture, output })
  assert.equal(packaged.status, 0, packaged.stderr)
  const rootfs = join(output, "rootfs")
  const rootfsAlias = join(root, "rootfs-alias")
  await symlink(rootfs, rootfsAlias)
  const rootAliasResult = runVerifier(rootfsAlias, packaged.stdout.trim(), fixture.trustedPublicKey)
  assert.equal(rootAliasResult.status, 1)
  assert.match(rootAliasResult.stderr, /image root must be a directory, not a symlink/)

  const releaseDirectoryAlias = join(root, "release-directory-alias")
  await symlink(join(rootfs, "usr/lib/chariox"), releaseDirectoryAlias)
  const keyAliasResult = runVerifier(
    rootfs,
    packaged.stdout.trim(),
    join(releaseDirectoryAlias, "release-public-key"),
  )
  assert.equal(keyAliasResult.status, 1)
  assert.match(keyAliasResult.stderr, /trusted release public key must be supplied outside/)

  const externalLocal = join(root, "external-local")
  await rename(join(rootfs, "usr/local"), externalLocal)
  await symlink(externalLocal, join(rootfs, "usr/local"))
  const ancestorAliasResult = runVerifier(rootfs, packaged.stdout.trim(), fixture.trustedPublicKey)
  assert.equal(ancestorAliasResult.status, 1)
  assert.match(ancestorAliasResult.stderr, /image root contains a symbolic link/)
})

test("managed kernel release packaging rejects symlink inputs", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-release-symlink-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  const kernelLink = join(root, "kernel-link")
  await symlink(fixture.kernel, kernelLink)
  const result = runPackager({ ...fixture, kernel: kernelLink, output: join(root, "output") })
  assert.equal(result.status, 1)
  assert.match(result.stderr, /kernel binary must be a bounded regular file/)
})

test("managed kernel release packaging rejects a broadly readable signing key", async (context) => {
  if (process.platform === "win32") return
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-release-key-mode-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  await chmod(fixture.signingKey, 0o644)
  const result = runPackager({ ...fixture, output: join(root, "output") })
  assert.equal(result.status, 1)
  assert.match(result.stderr, /must not be readable by group or other users/)
})

test("managed kernel release requires an exact immutable source commit", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-release-source-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  const abbreviated = runPackager({
    ...fixture,
    sourceCommit: fixture.sourceCommit.slice(0, 12),
    output: join(root, "abbreviated"),
  })
  assert.equal(abbreviated.status, 1)
  assert.match(abbreviated.stderr, /source commit must be a full lowercase Git commit ID/)

  const missing = runPackager({
    ...fixture,
    sourceCommit: "0".repeat(40),
    output: join(root, "missing"),
  })
  assert.equal(missing.status, 1)
  assert.match(missing.stderr, /source commit cannot be resolved/)
})

test("managed release materialization ignores ambient Git attributes and tar options", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-release-attributes-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  await writeFile(join(fixture.sourceRepository, ".git/info/attributes"), "* export-ignore\n")
  const output = join(root, "output")
  const result = runPackager(
    { ...fixture, output },
    "022",
    { TAR_OPTIONS: "--files-from=/etc/passwd", GIT_CONFIG_GLOBAL: join(root, "hostile-gitconfig") },
  )
  assert.equal(result.status, 0, result.stderr)
  assert.equal(
    await readFile(join(output, "rootfs/usr/lib/chariox/slice-build-context/Cargo.lock"), "utf8"),
    "version = 4\n",
  )
})

test("managed kernel builder archives the exact commit and emits a signed binary attestation", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-builder-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  const bin = join(root, "builder-bin")
  const trace = join(root, "builder-trace")
  const extractionState = join(root, "builder-extraction-state")
  const output = join(root, "builder-output")
  await mkdir(bin)
  await writeHarnessCommand(join(bin, "docker"), `#!/bin/sh
set -eu
[ -z "\${RUSTC_WRAPPER:-}" ]
[ -z "\${CARGO:-}" ]
[ -z "\${RUSTFLAGS:-}" ]
case "$1" in
  build)
    case " $* " in *" --pull --platform linux/amd64 --target rust-builder "*) ;; *) exit 31 ;; esac
    case " $* " in *" --tag chariox-managed-builder:"*"-"*"-"*) ;; *) exit 31 ;; esac
    for source do :; done
    dockerfile=
    previous=
    for argument do
      if [ "$previous" = "--file" ]; then dockerfile=$argument; fi
      previous=$argument
    done
    [ -f "$dockerfile" ]
    [ "$dockerfile" = "$source/apps/kernel/slice-linux-docker/docker/Dockerfile" ]
    [ ! -e "$source/.git" ]
    [ ! -e "$source/working-tree-only" ]
    printf '%s\n' "$*" > '${trace}'
    ;;
  run)
    case " $* " in *" --rm --pull=never --platform linux/amd64 --entrypoint cat sha256:"*) ;; *) exit 32 ;; esac
    previous=
    image=
    for argument do
      if [ "$previous" = "cat" ]; then image=$argument; fi
      previous=$argument
      source_path=$argument
    done
    [ "$image" = "sha256:1111111111111111111111111111111111111111111111111111111111111111" ]
    case "$source_path" in
      *chariox-kernel)
        printf 'tag replaced after first immutable read\n' > '${extractionState}'
        printf 'kernel from archived commit\n'
        ;;
      *chariox-managed-bootstrap)
        [ -f '${extractionState}' ]
        printf 'supervisor from archived commit\n'
        ;;
      *chariox-relay)
        [ -f '${extractionState}' ]
        printf 'relay from archived commit\n'
        ;;
      *) exit 32 ;;
    esac
    ;;
  image)
    case "$2" in
      inspect) printf '%s\n' 'sha256:1111111111111111111111111111111111111111111111111111111111111111' ;;
      rm) ;;
      *) exit 33 ;;
    esac
    ;;
  rm) ;;
  *) exit 33 ;;
esac
`)
  await writeFile(join(fixture.sourceRepository, "working-tree-only"), "must not enter the build\n")
  await writeFile(join(fixture.sourceRepository, ".git/info/attributes"), "* export-ignore\n")
  const result = spawnSync(
    process.execPath,
    [
      builder,
      "--source-repository", fixture.sourceRepository,
      "--source-commit", fixture.sourceCommit,
      "--builder-signing-key", fixture.signingKey,
      "--output", output,
    ],
    {
      encoding: "utf8",
      env: {
        ...process.env,
        PATH: `${bin}:${process.env.PATH}`,
        RUSTC_WRAPPER: "/tmp/hostile-rustc-wrapper",
        CARGO: "/tmp/hostile-cargo",
        RUSTFLAGS: "-C link-arg=/tmp/hostile",
        TAR_OPTIONS: "--files-from=/etc/passwd",
      },
    },
  )
  assert.equal(result.status, 0, result.stderr)
  assert.match(await readFile(trace, "utf8"), /--target rust-builder/)
  assert.match(
    await readFile(trace, "utf8"),
    /--file .*\/apps\/kernel\/slice-linux-docker\/docker\/Dockerfile/,
  )
  const attestationBytes = await readFile(join(output, "build-attestation.json"))
  const attestation = JSON.parse(attestationBytes)
  assert.equal(attestation.sourceCommit, fixture.sourceCommit)
  assert.equal(attestation.sourceTree, fixture.sourceTree)
  assert.equal(attestation.target, "x86_64-unknown-linux-gnu")
  assert.deepEqual(
    attestation.artifacts.map((artifact) => artifact.name),
    ["chariox-kernel", "chariox-managed-bootstrap", "chariox-relay"],
  )
  const signature = Buffer.from(await readFile(join(output, "build-attestation.sig"), "utf8"), "base64")
  assert.equal(verify(null, attestationBytes, fixture.publicKey, signature), true)
  assert.equal(
    await readFile(join(output, "builder-public-key"), "utf8"),
    rawPublicKey(fixture.publicKey).toString("base64"),
  )
})

test("managed kernel release requires a matching trusted builder attestation", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-release-attestation-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  const original = JSON.parse(await readFile(fixture.builderAttestation, "utf8"))
  const writeAttestation = async (value) => {
    const bytes = Buffer.from(JSON.stringify(value))
    await writeFile(fixture.builderAttestation, bytes)
    await writeFile(
      fixture.builderAttestationSignature,
      sign(null, bytes, fixture.builderPrivateKey).toString("base64"),
    )
  }

  await writeAttestation({ ...original, sourceCommit: "0".repeat(40) })
  const stale = runPackager({ ...fixture, output: join(root, "stale") })
  assert.equal(stale.status, 1)
  assert.match(stale.stderr, /identity or target does not match/)

  await writeAttestation({ ...original, target: "aarch64-unknown-linux-gnu" })
  const wrongTarget = runPackager({ ...fixture, output: join(root, "wrong-target") })
  assert.equal(wrongTarget.status, 1)
  assert.match(wrongTarget.stderr, /identity or target does not match/)

  await writeAttestation(original)
  await writeFile(fixture.builderAttestationSignature, Buffer.alloc(64).toString("base64"))
  const badSignature = runPackager({ ...fixture, output: join(root, "bad-signature") })
  assert.equal(badSignature.status, 1)
  assert.match(badSignature.stderr, /builder attestation signature is invalid/)

  await writeAttestation({
    ...original,
    artifacts: [original.artifacts[1], original.artifacts[0], original.artifacts[2]],
  })
  const wrongArtifacts = runPackager({ ...fixture, output: join(root, "wrong-artifacts") })
  assert.equal(wrongArtifacts.status, 1)
  assert.match(wrongArtifacts.stderr, /artifacts do not match/)
})

async function writeHarnessCommand(path, contents) {
  await writeFile(path, contents, { mode: 0o755 })
  await chmod(path, 0o755)
}

async function createInstallerHarness(root) {
  const bin = join(root, "bin")
  const state = join(root, "command-state")
  const installRoot = join(root, "installed")
  await mkdir(bin, { recursive: true })
  await mkdir(state, { recursive: true })
  await writeHarnessCommand(join(bin, "id"), `#!/bin/sh
if [ "\${1:-}" = "-u" ]; then echo 0; exit 0; fi
if [ "\${1:-}" = "-gn" ]; then
  [ "\${2:-}" = "chariox" ] && [ -f "$HARNESS_STATE/user-chariox" ] && echo chariox && exit 0
  [ "\${2:-}" = "chariox-docker" ] && [ -f "$HARNESS_STATE/user-chariox-docker" ] && echo chariox-docker && exit 0
  exit 1
fi
if [ "\${1:-}" = "chariox" ]; then [ -f "$HARNESS_STATE/user-chariox" ]; exit $?; fi
if [ "\${1:-}" = "chariox-docker" ]; then [ -f "$HARNESS_STATE/user-chariox-docker" ]; exit $?; fi
exit 1
`)
  await writeHarnessCommand(join(bin, "getent"), `#!/bin/sh
if [ "\${1:-}" = "group" ] && [ -f "$HARNESS_STATE/group-\${2:-}" ]; then echo "\${2}:x:998:"; exit 0; fi
if [ "\${1:-}" = "passwd" ] && [ "\${2:-}" = "chariox" ] && [ -f "$HARNESS_STATE/user-chariox" ]; then echo 'chariox:x:998:998::/var/lib/chariox/home:/usr/sbin/nologin'; exit 0; fi
if [ "\${1:-}" = "passwd" ] && [ "\${2:-}" = "chariox-docker" ] && [ -f "$HARNESS_STATE/user-chariox-docker" ]; then echo 'chariox-docker:x:997:997::/var/lib/chariox-docker/home:/usr/sbin/nologin'; exit 0; fi
exit 2
`)
  await writeHarnessCommand(join(bin, "groupadd"), "#!/bin/sh\nfor value in \"$@\"; do name=$value; done\ntouch \"$HARNESS_STATE/group-$name\"\n")
  await writeHarnessCommand(join(bin, "useradd"), "#!/bin/sh\nfor value in \"$@\"; do name=$value; done\ntouch \"$HARNESS_STATE/user-$name\"\n")
  await writeHarnessCommand(join(bin, "usermod"), "#!/bin/sh\nexit 0\n")
  await writeHarnessCommand(join(bin, "setfacl"), "#!/bin/sh\nexit 0\n")
  await writeHarnessCommand(join(bin, "systemctl"), `#!/bin/sh
printf '%s\\n' "$*" >> "$HARNESS_STATE/systemctl"
if [ -n "\${HARNESS_SYSTEMCTL_FAIL:-}" ]; then
  case "$*" in *"$HARNESS_SYSTEMCTL_FAIL"*) exit 1 ;; esac
fi
`)
  await writeHarnessCommand(join(bin, "flock"), `#!/bin/sh
if [ -n "\${HARNESS_FLOCK_ID:-}" ]; then
  touch "$HARNESS_STATE/flock-entered-$HARNESS_FLOCK_ID"
  while ! mkdir "$HARNESS_STATE/flock-held" 2>/dev/null; do sleep 0.05; done
  touch "$HARNESS_STATE/flock-acquired-$HARNESS_FLOCK_ID"
  parent=$PPID
  (while kill -0 "$parent" 2>/dev/null; do sleep 0.05; done; rmdir "$HARNESS_STATE/flock-held" 2>/dev/null || true) >/dev/null 2>&1 &
  while [ ! -f "$HARNESS_STATE/flock-release-$HARNESS_FLOCK_ID" ]; do sleep 0.05; done
fi
exit 0
`)
  await writeHarnessCommand(join(bin, "find"), `#!/bin/sh
if [ "\${HARNESS_FIND_FAIL:-0}" = "1" ]; then exit 1; fi
exec /usr/bin/find "$@"
`)
  await writeHarnessCommand(join(bin, "node"), `#!/bin/sh
if [ -n "\${HARNESS_MUTATE_SOURCE:-}" ]; then
  printf '%s\n' 'mutated after staging' > "$HARNESS_MUTATE_SOURCE"
fi
exec "${process.execPath}" "$@"
`)
  await writeHarnessCommand(join(bin, "install"), `#!/usr/bin/env node
const { spawnSync } = require("node:child_process")
const args = process.argv.slice(2)
const filtered = []
for (let index = 0; index < args.length; index += 1) {
  if (args[index] === "-o" || args[index] === "-g") { index += 1; continue }
  filtered.push(args[index])
}
const result = spawnSync("/usr/bin/install", filtered, { stdio: "inherit" })
process.exit(result.status ?? 1)
`)
  await writeHarnessCommand(join(bin, "mv"), `#!/bin/sh
if [ "$1" = -Tf ]; then echo 'mv -T is not portable' >&2; exit 64; fi
exec /bin/mv "$@"
`)
  return { bin, state, installRoot }
}

test("managed image installer verifies, installs twice, and rejects seeded runtime state", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-install-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  const output = join(root, "release")
  const packaged = runPackager({ ...fixture, output })
  assert.equal(packaged.status, 0, packaged.stderr)
  const harness = await createInstallerHarness(root)
  const env = {
    ...process.env,
    PATH: `${harness.bin}:${process.env.PATH}`,
    HARNESS_STATE: harness.state,
    CHARIOX_IMAGE_INSTALL_ROOT: harness.installRoot,
    CHARIOX_IMAGE_INSTALL_LOCK: join(harness.state, "install.lock"),
  }
  await writeFile(join(output, "rootfs/usr/local/bin/unsigned-extra"), "must not publish\n")
  const args = [join(output, "rootfs"), packaged.stdout.trim(), fixture.trustedPublicKey]
  const badDigest = spawnSync(
    installer,
    [args[0], `sha256:${"0".repeat(64)}`, args[2]],
    { encoding: "utf8", env },
  )
  assert.equal(badDigest.status, 1)
  assert.match(badDigest.stderr, /release manifest digest does not match/)
  assert.equal(await lstat(harness.installRoot).then(() => true, () => false), false)
  const brokerWantsLink = join(
    harness.installRoot,
    "etc/systemd/system/multi-user.target.wants/chariox-slice-broker.service",
  )
  await mkdir(join(brokerWantsLink, ".."), { recursive: true })
  await symlink("../chariox-slice-broker.service", brokerWantsLink)
  const sourceKernel = join(args[0], "usr/local/bin/chariox-kernel")
  const first = spawnSync("/bin/sh", ["-c", 'umask 077; exec "$@"', "managed-installer", installer, ...args], {
    encoding: "utf8",
    env: { ...env, HARNESS_MUTATE_SOURCE: sourceKernel },
  })
  assert.equal(first.status, 0, first.stderr)
  const deterministicRelease = join(
    harness.installRoot,
    "usr/lib/chariox/releases",
    packaged.stdout.trim().slice("sha256:".length),
  )
  for (const relativePath of ["usr", "usr/local", "usr/lib", "etc", "etc/systemd"]) {
    assert.equal(
      (await stat(join(deterministicRelease, relativePath))).mode & 0o777,
      0o755,
      `${relativePath} must be traversable by managed runtime users`,
    )
  }
  const firstReleaseInode = (await stat(deterministicRelease)).ino
  const currentLink = join(harness.installRoot, "usr/lib/chariox/current")
  const firstCurrentInode = (await lstat(currentLink)).ino
  const stalePending = join(
    harness.installRoot,
    "usr/lib/chariox/releases",
    `.new-${packaged.stdout.trim().slice("sha256:".length)}`,
  )
  await mkdir(stalePending)
  await writeFile(join(stalePending, "stale"), "interrupted install\n")
  await symlink("stale", `${currentLink}.new`)
  const unrelatedRelease = join(harness.installRoot, "usr/lib/chariox/releases/unrelated")
  await mkdir(unrelatedRelease)
  await writeFile(sourceKernel, "kernel fixture\n", { mode: 0o755 })
  const second = spawnSync(installer, args, { encoding: "utf8", env })
  assert.equal(second.status, 0, second.stderr)
  assert.equal((await stat(deterministicRelease)).ino, firstReleaseInode)
  assert.equal((await lstat(currentLink)).ino, firstCurrentInode)
  assert.equal(await lstat(stalePending).then(() => true, () => false), false)
  assert.equal(await lstat(`${currentLink}.new`).then(() => true, () => false), false)
  assert.equal((await stat(unrelatedRelease)).isDirectory(), true)

  await writeFile(`${currentLink}.new`, "must not be replaced\n")
  const obstructedLink = spawnSync(installer, args, { encoding: "utf8", env })
  assert.equal(obstructedLink.status, 1)
  assert.match(obstructedLink.stderr, /temporary link is obstructed/)
  assert.equal(await readFile(`${currentLink}.new`, "utf8"), "must not be replaced\n")
  await rm(`${currentLink}.new`)

  await writeFile(join(deterministicRelease, "usr/local/bin/chariox-kernel"), "corrupt release\n")
  const repairedRelease = spawnSync(installer, args, { encoding: "utf8", env })
  assert.equal(repairedRelease.status, 0, repairedRelease.stderr)
  assert.equal(
    await readFile(join(deterministicRelease, "usr/local/bin/chariox-kernel"), "utf8"),
    "kernel fixture\n",
  )
  assert.equal((await lstat(currentLink)).ino, firstCurrentInode)

  await rm(deterministicRelease, { recursive: true, force: true })
  await writeFile(deterministicRelease, "interrupted regular-file publication\n")
  const repairedObstruction = spawnSync(installer, args, { encoding: "utf8", env })
  assert.equal(repairedObstruction.status, 0, repairedObstruction.stderr)
  assert.equal((await stat(deterministicRelease)).isDirectory(), true)
  assert.equal((await lstat(currentLink)).ino, firstCurrentInode)
  assert.equal(await readFile(join(harness.installRoot, "usr/local/bin/chariox-kernel"), "utf8"), "kernel fixture\n")
  assert.equal((await stat(join(harness.installRoot, "usr/local/bin/chariox-kernel"))).mode & 0o777, 0o755)
  assert.equal((await stat(join(harness.installRoot, "usr/lib/chariox/release-manifest.json"))).mode & 0o777, 0o644)
  const installedProvisioner = join(
    harness.installRoot,
    "usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh",
  )
  assert.equal((await stat(installedProvisioner)).mode & 0o777, 0o755)
  assert.match(await readFile(installedProvisioner, "utf8"), /SLICE_BUILD_IMAGE/)
  assert.equal(
    await readFile(join(harness.installRoot, "etc/systemd/system/chariox-managed-bootstrap.service"), "utf8"),
    fixture.serviceBytes.toString("utf8"),
  )
  const installedBrokerService = join(harness.installRoot, "etc/systemd/system/chariox-slice-broker.service")
  assert.equal((await lstat(installedBrokerService)).isSymbolicLink(), true)
  assert.equal(await readFile(installedBrokerService, "utf8"), fixture.sliceBrokerServiceBytes.toString("utf8"))
  assert.equal(await lstat(brokerWantsLink).then(() => true, () => false), false)
  assert.equal(await readlink(join(harness.installRoot, "usr/lib/chariox/slice-build-context")), "current/usr/lib/chariox/slice-build-context")
  assert.match(await readlink(join(harness.installRoot, "usr/lib/chariox/current")), /^releases\/[a-f0-9]{64}$/)
  assert.equal(
    await lstat(join(harness.installRoot, "usr/lib/chariox/current/usr/local/bin/unsigned-extra"))
      .then(() => true, () => false),
    false,
  )
  const systemctl = (await readFile(join(harness.state, "systemctl"), "utf8")).trim().split("\n")
  assert.deepEqual(systemctl, [
    "daemon-reload",
    "enable chariox-rootless-docker.service",
    "enable chariox-managed-bootstrap.service",
    "daemon-reload",
    "enable chariox-rootless-docker.service",
    "enable chariox-managed-bootstrap.service",
    "daemon-reload",
    "enable chariox-rootless-docker.service",
    "enable chariox-managed-bootstrap.service",
    "daemon-reload",
    "enable chariox-rootless-docker.service",
    "enable chariox-managed-bootstrap.service",
  ])

  const failedTraversal = spawnSync(installer, args, {
    encoding: "utf8",
    env: { ...env, HARNESS_FIND_FAIL: "1" },
  })
  assert.equal(failedTraversal.status, 1)
  assert.match(failedTraversal.stderr, /managed kernel state root could not be inspected/)

  const seededIdentity = join(harness.installRoot, "var/lib/chariox/home/daemon-machine-identity.json")
  await writeFile(seededIdentity, "should never enter an image")
  const rejected = spawnSync(installer, args, { encoding: "utf8", env })
  assert.equal(rejected.status, 1)
  assert.match(rejected.stderr, /managed kernel state root is not pristine/)
  assert.equal(await readFile(seededIdentity, "utf8"), "should never enter an image")
})

test("managed image installer rejects a linked artifact ancestor before host mutation", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-install-ancestor-link-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  const output = join(root, "release")
  const packaged = runPackager({ ...fixture, output })
  assert.equal(packaged.status, 0, packaged.stderr)

  const rootfs = join(output, "rootfs")
  const externalLocal = join(root, "external-local")
  await rename(join(rootfs, "usr/local"), externalLocal)
  await symlink(externalLocal, join(rootfs, "usr/local"))

  const harness = await createInstallerHarness(root)
  const result = spawnSync(
    installer,
    [rootfs, packaged.stdout.trim(), fixture.trustedPublicKey],
    {
      encoding: "utf8",
      env: {
        ...process.env,
        PATH: `${harness.bin}:${process.env.PATH}`,
        HARNESS_STATE: harness.state,
        CHARIOX_IMAGE_INSTALL_ROOT: harness.installRoot,
        CHARIOX_IMAGE_INSTALL_LOCK: join(harness.state, "install.lock"),
      },
    },
  )
  assert.equal(result.status, 1)
  assert.match(result.stderr, /image root contains a symbolic link/)
  assert.equal(await lstat(harness.installRoot).then(() => true, () => false), false)
})

test("managed image installer atomically pivots current to a different release", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-install-pivot-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const firstRoot = join(root, "first-fixture")
  const secondRoot = join(root, "second-fixture")
  await mkdir(firstRoot)
  await mkdir(secondRoot)
  const firstFixture = await makeFixture(firstRoot, "one")
  const secondFixture = await makeFixture(secondRoot, "two")
  const firstOutput = join(root, "first-release")
  const secondOutput = join(root, "second-release")
  const firstPackage = runPackager({ ...firstFixture, output: firstOutput })
  const secondPackage = runPackager({ ...secondFixture, output: secondOutput })
  assert.equal(firstPackage.status, 0, firstPackage.stderr)
  assert.equal(secondPackage.status, 0, secondPackage.stderr)
  assert.notEqual(firstPackage.stdout, secondPackage.stdout)

  const harness = await createInstallerHarness(root)
  const env = {
    ...process.env,
    PATH: `${harness.bin}:${process.env.PATH}`,
    HARNESS_STATE: harness.state,
    CHARIOX_IMAGE_INSTALL_ROOT: harness.installRoot,
    CHARIOX_IMAGE_INSTALL_LOCK: join(harness.state, "install.lock"),
  }
  const installRelease = (output, packaged, fixture, environment = env) => spawnSync(installer, [
    join(output, "rootfs"), packaged.stdout.trim(), fixture.trustedPublicKey,
  ], { encoding: "utf8", env: environment })
  const first = installRelease(firstOutput, firstPackage, firstFixture)
  assert.equal(first.status, 0, first.stderr)
  const current = join(harness.installRoot, "usr/lib/chariox/current")
  assert.equal(await readlink(current), `releases/${firstPackage.stdout.trim().slice(7)}`)

  const failedSecond = installRelease(secondOutput, secondPackage, secondFixture, {
    ...env,
    HARNESS_SYSTEMCTL_FAIL: "enable chariox-managed-bootstrap.service",
  })
  assert.equal(failedSecond.status, 1)
  assert.match(failedSecond.stderr, /restored previous current release/)
  assert.equal(await readlink(current), `releases/${firstPackage.stdout.trim().slice(7)}`)

  const second = installRelease(secondOutput, secondPackage, secondFixture)
  assert.equal(second.status, 0, second.stderr)
  assert.equal(await readlink(current), `releases/${secondPackage.stdout.trim().slice(7)}`)
  assert.equal(
    await readFile(join(harness.installRoot, "usr/local/bin/chariox-kernel"), "utf8"),
    secondFixture.kernelContents,
  )
})

test("a terminated managed image install releases the lock for a concurrent install", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-install-lock-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  const output = join(root, "release")
  const packaged = runPackager({ ...fixture, output })
  assert.equal(packaged.status, 0, packaged.stderr)
  const harness = await createInstallerHarness(root)
  const env = {
    ...process.env,
    PATH: `${harness.bin}:${process.env.PATH}`,
    HARNESS_STATE: harness.state,
    CHARIOX_IMAGE_INSTALL_ROOT: harness.installRoot,
    CHARIOX_IMAGE_INSTALL_LOCK: join(harness.state, "install.lock"),
  }
  const args = [join(output, "rootfs"), packaged.stdout.trim(), fixture.trustedPublicKey]
  const first = spawn(installer, args, { env: { ...env, HARNESS_FLOCK_ID: "first" } })
  context.after(() => {
    first.kill("SIGKILL")
  })
  const firstExit = once(first, "exit")
  await waitForPath(join(harness.state, "flock-acquired-first"))
  const second = spawn(installer, args, { env: { ...env, HARNESS_FLOCK_ID: "second" } })
  context.after(() => {
    second.kill("SIGKILL")
  })
  const secondExit = once(second, "exit")
  await waitForPath(join(harness.state, "flock-entered-second"))
  assert.equal(await lstat(join(harness.state, "flock-acquired-second")).then(() => true, () => false), false)
  assert.equal(await lstat(harness.installRoot).then(() => true, () => false), false)
  for (const name of ["group-chariox", "group-chariox-slice", "group-chariox-docker", "user-chariox", "user-chariox-docker"]) {
    assert.equal(await lstat(join(harness.state, name)).then(() => true, () => false), false)
  }
  first.kill("SIGTERM")
  await writeFile(join(harness.state, "flock-release-first"), "release\n")
  const [firstCode, firstSignal] = await withTimeout(firstExit, "terminated installer timed out")
  assert.equal(firstCode, null)
  assert.equal(firstSignal, "SIGTERM")
  await waitForPath(join(harness.state, "flock-acquired-second"))
  await writeFile(join(harness.state, "flock-release-second"), "release\n")
  assert.equal((await withTimeout(secondExit, "second installer timed out"))[0], 0)
})

test("managed image installer has no runtime start or network path", async () => {
  const contents = await readFile(installer, "utf8")
  assert.match(contents, /if \[ "\$\(id -u\)" -ne 0 \]/)
  assert.match(contents, /node "\$script_root\/verify-image-release\.mjs"/)
  assert.match(contents, /managed kernel state root is not pristine/)
  assert.match(contents, /groupadd --system chariox/)
  assert.match(contents, /groupadd --system chariox-docker/)
  assert.match(contents, /groupadd --system chariox-slice/)
  assert.match(contents, /useradd --system --gid chariox --home-dir \/var\/lib\/chariox\/home/)
  assert.match(contents, /useradd --system --gid chariox-docker --home-dir \/var\/lib\/chariox-docker\/home/)
  assert.match(contents, /systemctl daemon-reload/)
  assert.match(contents, /systemctl enable chariox-managed-bootstrap\.service/)
  const lockIndex = contents.indexOf("flock 9")
  for (const mutation of ["state_entry=$(find", "groupadd --system", "useradd --system", "usermod --append", "\ninstall -d", "releases_root="]) {
    assert.ok(lockIndex >= 0 && lockIndex < contents.indexOf(mutation), `${mutation} must remain behind the install lock`)
  }
  assert.doesNotMatch(contents, /systemctl (?:start|restart|enable --now)/)
  assert.doesNotMatch(contents, /\b(?:curl|wget|ssh|scp)\b/)
  assert.doesNotMatch(contents, /\bmv\s+-T/)
  assert.match(contents, /renameSync\(source, destination\)/)
  assert.doesNotMatch(contents, /\.arroba/)
})
