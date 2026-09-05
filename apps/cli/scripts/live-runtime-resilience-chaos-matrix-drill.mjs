#!/usr/bin/env node
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

import {
  defaultDrillMatrixArtifactIndexPath,
  parseDrillScenarioIds,
  runDrillMatrix,
  selectDrillMatrixScenarios,
} from "./lib/drill-matrix-runner.mjs"
import {
  appendHetznerPassthrough,
  drillDeploymentPresetMetadata,
  parseHetznerPassthroughArg,
} from "./lib/drill-environment-presets.mjs"
import {
  applyProviderAccountAlias,
  applyProviderModelOverride,
  providerProfileMetadata,
  resolveProviderModel,
} from "./lib/drill-provider-profiles.mjs"
import {
  DRILL_CHAOS_FAULT_KINDS,
  DRILL_CHAOS_INVARIANT_IDS,
  DRILL_CHAOS_REPLAY_SCHEMA,
} from "./lib/drill-chaos-contract.mjs"
import { DEFAULT_DETERMINISTIC_RUNTIME_CHAOS_SEED } from "./lib/drill-deterministic-runtime-model.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, "..", "..", "..")

const kernelReconnectDrill = path.join(scriptDir, "live-kernel-reconnect-drill.mjs")
const roomTakeoverReconnectFaultDrill = path.join(scriptDir, "live-room-takeover-reconnect-fault-drill.mjs")
const localRestartDrill = path.join(scriptDir, "live-local-restart-persistence-drill.mjs")
const browserControllerFaultDrill = path.join(scriptDir, "live-browser-controller-fault-drill.mjs")
const queueSaturationFaultDrill = path.join(scriptDir, "live-queue-saturation-fault-drill.mjs")
const relayIdentitySecurityDrill = path.join(scriptDir, "live-relay-identity-security-drill.mjs")
const reconnectStormDrill = path.join(scriptDir, "live-reconnect-storm-drill.mjs")
const memoryPressureAdmissionFaultDrill = path.join(scriptDir, "live-memory-pressure-admission-fault-drill.mjs")
const diskPressureAdmissionFaultDrill = path.join(scriptDir, "live-disk-pressure-admission-fault-drill.mjs")
const sliceSaveAckLossFaultDrill = path.join(scriptDir, "live-slice-save-ack-loss-fault-drill.mjs")
const sliceSaveInterruptionFaultDrill = path.join(scriptDir, "live-slice-save-interruption-fault-drill.mjs")
const savedStateCorruptionFaultDrill = path.join(scriptDir, "live-saved-state-corruption-fault-drill.mjs")
const sliceRestoreInterruptionFaultDrill = path.join(scriptDir, "live-slice-restore-interruption-fault-drill.mjs")
const browserDownloadDiskFaultDrill = path.join(scriptDir, "live-browser-download-disk-fault-drill.mjs")
const resourceExhaustionFaultDrill = path.join(scriptDir, "live-resource-exhaustion-fault-drill.mjs")
const relayRuntimeDrill = path.join(scriptDir, "live-relay-runtime-drill.mjs")
const remoteRestartDrill = path.join(scriptDir, "live-remote-restart-drill.mjs")
const remoteHomeExtensionDrill = path.join(scriptDir, "live-remote-home-extension-drill.mjs")
const hostedCloudRelayDrill = path.join(scriptDir, "live-hosted-cloud-relay-drill.mjs")
const tuiWebParityDrill = path.join(scriptDir, "tui-web-terminal-parity-drill.mjs")
const providerThreadTransferDrill = path.join(scriptDir, "live-provider-thread-transfer-drill.mjs")
const deterministicRuntimeChaosDrill = path.join(scriptDir, "deterministic-runtime-chaos-drill.mjs")
const sliceDisplayFaultDrill = path.join(scriptDir, "live-slice-display-fault-drill.mjs")

const DEFAULT_CODEX_MODEL = process.env.CHARIOX_RUNTIME_RESILIENCE_CODEX_MODEL
  ?? process.env.CHARIOX_CODEX_MODEL
  ?? "gpt-5.4-mini"
const DEFAULT_OPENCODE_MODEL = process.env.CHARIOX_RUNTIME_RESILIENCE_OPENCODE_MODEL
  ?? process.env.CHARIOX_OPENCODE_MODEL
  ?? "opencode/gpt-5.2"
const DEFAULT_CHAOS_SEED = process.env.CHARIOX_RUNTIME_RESILIENCE_CHAOS_SEED
  ?? DEFAULT_DETERMINISTIC_RUNTIME_CHAOS_SEED

const MATRIX = [
  scenario({
    id: "deterministic-runtime-convergence",
    description: "seeded virtual-clock fault injection proves idempotent execution and eventual TUI/web convergence",
    script: deterministicRuntimeChaosDrill,
    args: [],
    classification: "ui-client-projection",
    runtimeSignals: ["client-projection-health", "runtime-projection-health", "runtime-transition-audit", "session-authority"],
    deployment: "local",
    provider: "dev-stub",
    exitCriteria: [
      "every accepted action executes exactly once despite drop, delay, duplication, and reorder faults",
      "TUI and web projections converge through cursor replay or snapshot fallback with monotonic cursors",
      "process death suppresses stale callbacks and leaves bounded empty queues with no leaked resources",
    ],
  }),
  scenario({
    id: "local-kernel-websocket-drop",
    description: "local kernel websocket close, reconnect, resubscribe, and request replay",
    script: kernelReconnectDrill,
    args: ["--keep-artifacts-on-failure"],
    classification: "relay-runtime",
    runtimeSignals: ["client-projection-health", "relay-target-freshness"],
    deployment: "local",
    provider: "dev-stub",
    exitCriteria: [
      "client observes transport_closed and transport_resumed without losing the control request",
      "second subscription resumes from the last retained event id instead of resetting transcript state",
    ],
  }),
  scenario({
    id: "local-room-takeover-reconnect",
    description: "lose a committed human-takeover response, reconnect, and retain one authoritative input owner",
    script: roomTakeoverReconnectFaultDrill,
    args: [],
    classification: "kernel-authority",
    runtimeSignals: ["client-projection-health", "runtime-transition-audit", "session-authority"],
    deployment: "local",
    provider: "dev-stub",
    exitCriteria: [
      "a dropped takeover response does not roll back the committed human desktop owner",
      "retrying the same command id from a fresh connection replays the exact response without a second takeover event",
      "agent mutation remains rejected until the human explicitly releases input",
      "the focused drill records resources externally and removes its private fixture",
    ],
  }),
  scenario({
    id: "local-kernel-restart-durable-state",
    description: "local kernel crash restores durable session, grants, history, and active workflow execution",
    script: localRestartDrill,
    args: ["--keep-artifacts-on-failure"],
    classification: "kernel-authority",
    runtimeSignals: ["provider-run-lifecycle", "runtime-transition-audit", "session-authority"],
    deployment: "local",
    provider: "dev-stub",
    exitCriteria: [
      "session, agent, MCP grant, skill grant, and history outline survive a kernel restart",
      "the active prompt keeps its identity while the provider run is relaunched and the workflow remains running",
    ],
  }),
  scenario({
    id: "local-browser-controller-crash",
    description: "SIGKILL the Room Browser Controller, fence stale work, and reconcile one authoritative browser",
    script: browserControllerFaultDrill,
    args: [],
    classification: "kernel-authority",
    runtimeSignals: ["runtime-transition-audit", "session-authority", "slice-runtime-state"],
    deployment: "local",
    provider: "dev-stub",
    exitCriteria: [
      "controller process death is attributed as process_lost and stale element references are rejected",
      "recovery replaces the process while preserving tabs and input authority without duplication",
      "running and queued mutations settle without replay before one fresh action executes exactly once",
      "one fresh post-recovery action completes exactly once and fixture processes are removed",
    ],
  }),
  scenario({
    id: "local-relay-queue-saturation",
    description: "fill relay target and subscriber queues and prove bounded isolation without closing healthy readers",
    script: queueSaturationFaultDrill,
    args: [],
    classification: "relay-runtime",
    runtimeSignals: ["client-projection-health", "relay-target-freshness"],
    deployment: "local",
    provider: "dev-stub",
    exitCriteria: [
      "full client and peer target queues return retryable backpressure and clear their pending request",
      "one slow subscription is removed while another subscription and the daemon reader lane remain live",
      "backpressure metrics record the bounded fault and no owned process remains",
    ],
  }),
  scenario({
    id: "local-relay-token-expiry-isolation",
    description: "expire one accepted relay token under clock skew while healthy peers and realm isolation remain intact",
    script: relayIdentitySecurityDrill,
    args: [],
    classification: "relay-runtime",
    runtimeSignals: ["client-projection-health", "relay-target-freshness"],
    deployment: "local",
    provider: "dev-stub",
    exitCriteria: [
      "an accepted short-lived client is closed at expiry while a healthy client completes routed request round trips before and after the close",
      "an already-expired token and a token issued beyond clock-skew tolerance are rejected",
      "production JWT tokens honor clock-skew tolerance while invalid identity bindings and cross-realm routes are rejected",
      "the exact external relay process is stopped and resource evidence is retained outside the repository",
    ],
  }),
  scenario({
    id: "local-reconnect-storm-slow-viewer",
    description: "repeat concurrent viewer reconnects and isolate one slow display consumer",
    script: reconnectStormDrill,
    args: ["--clients", "8", "--cycles", "3", "--slow-events", "4096"],
    classification: "relay-runtime",
    runtimeSignals: ["client-projection-health", "relay-target-freshness", "session-authority"],
    deployment: "local",
    provider: "dev-stub",
    exitCriteria: [
      "every viewer resumes its own monotonic event cursor through three reconnect cycles",
      "one stalled viewer is closed without delaying healthy viewers, agents, or kernel control traffic",
      "the kernel stays within the bounded memory and CPU envelope and every owned process is removed",
    ],
  }),
  scenario({
    id: "local-slice-memory-pressure-admission",
    description: "reject unsafe concurrent local slice starts before Docker reaches memory exhaustion",
    script: memoryPressureAdmissionFaultDrill,
    args: [],
    classification: "slice-runtime",
    runtimeSignals: ["runtime-transition-audit", "slice-runtime-state"],
    deployment: "local",
    provider: "dev-stub",
    exitCriteria: [
      "new Linux slices receive a 2048 MiB memory limit when no override is configured",
      "host-wide Docker admission serializes kernel processes and retains 512 MiB for the engine",
      "existing targets reserve their actual limit and legacy unbounded slices fail closed",
      "rejection leaves active state unchanged and admission reopens after capacity recovers",
    ],
  }),
  scenario({
    id: "local-slice-disk-pressure-admission",
    description: "reject unsafe slice snapshots before Docker or Chariox state storage reaches ENOSPC",
    script: diskPressureAdmissionFaultDrill,
    args: [],
    classification: "slice-runtime",
    runtimeSignals: ["runtime-transition-audit", "slice-runtime-state"],
    deployment: "local",
    provider: "dev-stub",
    exitCriteria: [
      "snapshot demand includes the home archive and writable container layer",
      "host-wide admission retains 2048 MiB in Docker and Chariox state storage",
      "rejection leaves the active slice and last known-good generation unchanged",
      "admission reopens after disk capacity recovers and measurement helpers are removed",
    ],
  }),
  scenario({
    id: "local-slice-save-acknowledgement-loss",
    description: "lose a completed slice-save response and replay its original generation without a second dispatch",
    script: sliceSaveAckLossFaultDrill,
    args: [],
    classification: "kernel-authority",
    runtimeSignals: ["runtime-transition-audit", "slice-runtime-state"],
    deployment: "local",
    provider: "dev-stub",
    exitCriteria: [
      "same-process retry returns the first slice-save result without another dispatch",
      "kernel-cache reload returns the same saved-state generation without another dispatch",
      "reusing the command id for a different save request fails closed",
      "the focused drill records resources externally and removes its temporary cache",
    ],
  }),
  scenario({
    id: "local-slice-save-interruption",
    description: "interrupt saved-state publication before commit and after manifest rename without losing a restorable generation",
    script: sliceSaveInterruptionFaultDrill,
    args: [],
    classification: "kernel-authority",
    runtimeSignals: ["runtime-transition-audit", "slice-runtime-state"],
    deployment: "local",
    provider: "dev-stub",
    exitCriteria: [
      "a pre-commit publication failure preserves the prior manifest and archive while removing the unpublished generation",
      "uncertain durability after manifest rename retains both the prior and next archives",
      "the manifest-selected generation and retained prior generation both remain restorable",
      "the focused drill records resources externally and removes every fixture",
    ],
  }),
  scenario({
    id: "local-saved-state-corruption",
    description: "quarantine a corrupt saved-state archive without disturbing the prior known-good backup",
    script: savedStateCorruptionFaultDrill,
    args: [],
    classification: "kernel-authority",
    runtimeSignals: ["runtime-transition-audit", "slice-runtime-state"],
    deployment: "local",
    provider: "dev-stub",
    exitCriteria: [
      "a corrupt archive is rejected before image inspection or runtime replacement",
      "the corrupt archive leaves its restore path and remains quarantined for inspection",
      "an independent known-good backup remains byte-identical and passes full integrity validation",
      "the focused drill records resources externally and removes every fixture",
    ],
  }),
  scenario({
    id: "local-slice-restore-interruption",
    description: "interrupt backup restore after replacement creation and recover the rollback generation on kernel restart",
    script: sliceRestoreInterruptionFaultDrill,
    args: [],
    classification: "kernel-authority",
    runtimeSignals: ["runtime-transition-audit", "slice-runtime-state"],
    deployment: "local",
    provider: "dev-stub",
    exitCriteria: [
      "restore intent is durable before the target replacement starts",
      "SIGKILL after replacement creation leaves no committed restore resolution",
      "kernel startup restores the rollback generation and removes the partial runtime",
      "the focused drill records resources externally and removes every private fixture",
    ],
  }),
  scenario({
    id: "local-browser-download-disk-pressure",
    description: "fail closed and cancel active browser downloads when slice storage crosses its reserve",
    script: browserDownloadDiskFaultDrill,
    args: [],
    classification: "slice-runtime",
    runtimeSignals: ["runtime-transition-audit", "slice-runtime-state"],
    deployment: "local",
    provider: "dev-stub",
    exitCriteria: [
      "download policy is not enabled when capacity is low or unavailable",
      "active downloads are canceled with a terminal disk-pressure reason",
      "a download arriving during cancellation receives a follow-up capacity check",
      "the focused drill records resources externally and leaves no owned process",
    ],
  }),
  scenario({
    id: "local-process-file-descriptor-exhaustion",
    description: "exhaust isolated process and file-descriptor budgets while an established terminal lane remains live",
    script: resourceExhaustionFaultDrill,
    args: [],
    classification: "slice-runtime",
    runtimeSignals: ["client-projection-health", "runtime-transition-audit", "slice-runtime-state"],
    deployment: "local",
    provider: "dev-stub",
    exitCriteria: [
      "file-descriptor exhaustion returns EMFILE or ENFILE without starving an established terminal lane",
      "process exhaustion returns EAGAIN without starving an established terminal lane",
      "new and reused slices enforce finite process and file-descriptor ceilings before startup",
      "every probe removes its owned descriptors, sockets, and child processes",
    ],
  }),
  scenario({
    id: "local-relay-restart-reconnect",
    description: "self-hosted relay restart reconnects kernel/client transport and accepts a post-reconnect prompt",
    script: relayRuntimeDrill,
    args: ["--provider", "dev-stub", "--model", "runtime-resilience-dev-stub"],
    classification: "relay-target-freshness",
    runtimeSignals: ["provider-run-lifecycle", "relay-target-freshness", "session-authority"],
    deployment: "self-hosted-relay",
    provider: "dev-stub",
    exitCriteria: [
      "relay restart produces transport_closed then transport_resumed for the active session",
      "post-reconnect prompt completes exactly once through the kernel-owned session path",
    ],
  }),
  scenario({
    id: "local-tui-web-terminal-parity",
    description: "non-interactive TUI and web terminal projection parity checks",
    script: tuiWebParityDrill,
    args: [],
    classification: "ui-client-projection",
    runtimeSignals: ["client-projection-health", "runtime-projection-health", "session-authority"],
    deployment: "local",
    provider: "dev-stub",
    exitCriteria: [
      "TUI transcript, queue, footer, prompt, and waiting-room projections match web terminal contracts",
      "visual-session and visual-control scripts remain syntactically valid for screenshot-backed evidence runs",
    ],
  }),
  scenario({
    id: "same-host-remote-worker-restart",
    description: "same-host home and worker kernel restart repairs leased-agent binding",
    script: remoteRestartDrill,
    args: ["--keep-artifacts-on-failure"],
    classification: "relay-target-freshness",
    runtimeSignals: ["lease-health", "relay-target-freshness", "session-authority"],
    deployment: "same-host-remote",
    provider: "dev-stub",
    exitCriteria: [
      "home restart preserves the remote binding",
      "worker restart refreshes stale leased-agent state before the next prompt",
      "both-restart path leaves exactly one repaired remote execution binding",
    ],
  }),
  providerThreadScenario({
    id: "worker-provider-resume-codex",
    provider: "codex",
    description: "Codex provider thread resumes on a same-host worker after worker transfer",
    drill: "worker-resume",
    classification: "provider-error",
    runtimeSignals: ["lease-health", "provider-run-lifecycle", "session-authority"],
    deployment: "same-host-remote",
    extraArgs: ["--cleanup-on-success"],
    exitCriteria: [
      "provider resume state is captured before transfer",
      "worker-side provider run resumes without duplicating the prompt",
    ],
  }),
  providerThreadScenario({
    id: "worker-provider-resume-opencode",
    provider: "opencode",
    description: "OpenCode provider thread resumes on a same-host worker after worker transfer",
    drill: "worker-resume",
    classification: "provider-error",
    runtimeSignals: ["lease-health", "provider-run-lifecycle", "session-authority"],
    deployment: "same-host-remote",
    extraArgs: ["--cleanup-on-success"],
    exitCriteria: [
      "provider resume state is captured before transfer",
      "worker-side provider run resumes without duplicating the prompt",
    ],
  }),
  scenario({
    id: "local-slice-display-process-faults",
    description: "bounded local slice injects Selkies and Chromium process death and proves recovery plus cleanup",
    script: sliceDisplayFaultDrill,
    args: [],
    classification: "slice-runtime",
    runtimeSignals: ["client-projection-health", "slice-runtime-state"],
    deployment: "local",
    provider: "dev-stub",
    requires: ["slice"],
    exitCriteria: [
      "Selkies process death degrades display while Browser remains usable and one retry restores the streamer",
      "Chromium process death leaves Selkies live and desktop lifecycle recovery preserves browser profile state",
      "the capped network-disabled container, display socket, and listeners are removed",
    ],
  }),
  providerThreadScenario({
    id: "slice-restart-codex",
    provider: "codex",
    description: "Codex slice worker restart preserves recoverable provider thread state",
    drill: "slice-restart",
    classification: "slice-runtime",
    runtimeSignals: ["provider-run-lifecycle", "session-authority", "slice-runtime-state"],
    deployment: "local",
    requires: ["slice"],
    extraArgs: ["--slice-build-image", "auto", "--cleanup-on-success"],
    exitCriteria: [
      "slice restart leaves kernel-owned session authority intact",
      "provider state is either resumed once or surfaced as a structured retry state",
    ],
  }),
  providerThreadScenario({
    id: "slice-restart-opencode",
    provider: "opencode",
    description: "OpenCode slice worker restart preserves recoverable provider thread state",
    drill: "slice-restart",
    classification: "slice-runtime",
    runtimeSignals: ["provider-run-lifecycle", "session-authority", "slice-runtime-state"],
    deployment: "local",
    requires: ["slice"],
    extraArgs: ["--slice-build-image", "auto", "--cleanup-on-success"],
    exitCriteria: [
      "slice restart leaves kernel-owned session authority intact",
      "provider state is either resumed once or surfaced as a structured retry state",
    ],
  }),
  scenario({
    id: "hetzner-collaborator-reconnect-authority",
    description: "Hetzner collaborator remote agent survives relay loss and repairs after worker loss while home authority remains enforced",
    script: remoteHomeExtensionDrill,
    args: ["--hetzner-worker", "--collab", "--restart-relay", "--restart-worker"],
    requires: ["hetzner"],
    classification: "kernel-authority",
    runtimeSignals: ["home-extension-manifest-sync", "lease-health", "session-authority"],
    deployment: "hetzner",
    provider: "dev-stub",
    exitCriteria: [
      "the active collaborator runtime resumes through a restarted relay without losing its home extension grants",
      "worker restart repairs the stale lease and grant/revoke plus stale invocation checks remain home-kernel-owned",
    ],
  }),
  scenario({
    id: "hosted-cloud-relay-second-kernel-reconnect",
    description: "hosted Cloud relay second-kernel runtime reconnect smoke",
    script: hostedCloudRelayDrill,
    args: [],
    env: {
      CHARIOX_CLOUD_HOSTED_SECOND_KERNEL: "1",
      CHARIOX_CLOUD_HOSTED_MULTI_USER: "0",
    },
    requires: ["hosted-cloud"],
    classification: "relay-runtime",
    runtimeSignals: ["agent-lifecycle", "lease-health", "provider-run-lifecycle", "relay-target-freshness"],
    deployment: "hosted-cloud",
    provider: "dev-stub",
    exitCriteria: [
      "Cloud-issued relay credentials reconnect home and worker kernels",
      "hosted second-kernel provider turn completes through relay transport without Cloud proxying runtime traffic",
    ],
  }),
]

function scenario(definition) {
  return {
    args: [],
    env: undefined,
    requires: [],
    ...definition,
  }
}

function providerThreadScenario({
  id,
  provider,
  description,
  drill,
  classification,
  runtimeSignals,
  deployment,
  requires = [],
  extraArgs = [],
  exitCriteria,
}) {
  return scenario({
    id,
    provider,
    providerFamily: provider,
    description,
    script: providerThreadTransferDrill,
    args: ["--drill", drill, "--provider", provider, ...extraArgs],
    classification,
    runtimeSignals,
    deployment,
    requires,
    exitCriteria,
  })
}

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/live-runtime-resilience-chaos-matrix-drill.mjs [options]",
    "",
    "Runs runtime resilience chaos coverage by composing existing reconnect, restart, relay, remote, TUI/web, provider-resume, display-fault, slice, Hetzner, and hosted Cloud drills.",
    "Local and same-host scenarios are selected by default. Slice, Hetzner, and hosted Cloud scenarios are opt-in.",
    "",
    "Options:",
    "  --include-slices         Include local Docker slice restart provider scenarios",
    "  --include-hetzner        Include Hetzner worker/collaborator scenarios",
    "  --include-hosted-cloud   Include hosted Cloud relay scenarios",
    "  --only IDS               Comma-separated scenario ids",
    "  --dry-run                Print selected commands without running drills",
    "  --continue-on-failure    Run every selected scenario before exiting non-zero",
    "  --report PATH            Write a machine-readable matrix report; defaults under ~/.codex/evidence/browser-computer-use",
    "  --artifact-index PATH     Write a verifiable artifact index for the matrix report",
    "  --chaos-seed VALUE        Replay seed for deterministic fault injection",
    "  --chaos-replay PATH       Deterministic replay artifact path",
    "  --provider-model P=M      Override model for provider-resume scenarios",
    "  --provider-account P=A    Label provider account/profile metadata without exposing credentials",
    "  --hetzner-host HOST       Forwarded to Hetzner drill scenarios",
    "  --hetzner-key PATH        Forwarded to Hetzner drill scenarios",
    "  --hetzner-repo PATH       Forwarded to Hetzner drill scenarios",
    "",
    "Environment defaults:",
    `  CHARIOX_RUNTIME_RESILIENCE_CODEX_MODEL=${DEFAULT_CODEX_MODEL}`,
    `  CHARIOX_RUNTIME_RESILIENCE_OPENCODE_MODEL=${DEFAULT_OPENCODE_MODEL}`,
    `  CHARIOX_RUNTIME_RESILIENCE_CHAOS_SEED=${DEFAULT_CHAOS_SEED}`,
    "",
    "Scenario ids:",
    ...MATRIX.map((scenarioItem) => `  ${scenarioItem.id.padEnd(42)} ${scenarioItem.description}`),
  ].join("\n"))
}

function readValue(argv, index, flag) {
  const value = argv[index + 1]
  if (!value || value.startsWith("--")) throw new Error(`${flag} requires a value`)
  return value
}

function parseArgs(argv) {
  const options = {
    includeSlices: false,
    includeHetzner: false,
    includeHostedCloud: false,
    only: null,
    dryRun: false,
    continueOnFailure: false,
    reportPath: null,
    artifactIndexPath: null,
    chaosSeed: DEFAULT_CHAOS_SEED,
    chaosReplayPath: null,
    providerAccounts: {},
    providerModels: {},
    passthrough: [],
    help: false,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--") continue
    else if (arg === "--include-slices") options.includeSlices = true
    else if (arg === "--include-hetzner") options.includeHetzner = true
    else if (arg === "--include-hosted-cloud") options.includeHostedCloud = true
    else if (arg === "--dry-run") options.dryRun = true
    else if (arg === "--continue-on-failure") options.continueOnFailure = true
    else if (arg === "--report") options.reportPath = readValue(argv, index++, arg)
    else if (arg.startsWith("--report=")) options.reportPath = arg.slice("--report=".length)
    else if (arg === "--artifact-index") options.artifactIndexPath = readValue(argv, index++, arg)
    else if (arg.startsWith("--artifact-index=")) options.artifactIndexPath = arg.slice("--artifact-index=".length)
    else if (arg === "--chaos-seed") options.chaosSeed = readValue(argv, index++, arg)
    else if (arg.startsWith("--chaos-seed=")) options.chaosSeed = arg.slice("--chaos-seed=".length)
    else if (arg === "--chaos-replay") options.chaosReplayPath = readValue(argv, index++, arg)
    else if (arg.startsWith("--chaos-replay=")) options.chaosReplayPath = arg.slice("--chaos-replay=".length)
    else if (arg === "--provider-account") applyProviderAccountAlias(options.providerAccounts, readValue(argv, index++, arg))
    else if (arg.startsWith("--provider-account=")) applyProviderAccountAlias(options.providerAccounts, arg.slice("--provider-account=".length))
    else if (arg === "--provider-model") applyProviderModelOverride(options.providerModels, readValue(argv, index++, arg))
    else if (arg.startsWith("--provider-model=")) applyProviderModelOverride(options.providerModels, arg.slice("--provider-model=".length))
    else if (arg === "--help" || arg === "-h") options.help = true
    else if (arg === "--only") options.only = parseDrillScenarioIds(readValue(argv, index++, arg))
    else if (arg.startsWith("--only=")) options.only = parseDrillScenarioIds(arg.slice("--only=".length))
    else {
      const hetznerArg = parseHetznerPassthroughArg(argv, index)
      if (hetznerArg) {
        options.passthrough.push(...hetznerArg.args)
        index = hetznerArg.nextIndex
        continue
      }
      throw new Error(`unknown argument: ${arg}`)
    }
  }
  return options
}

function selectScenarios(options) {
  const enabledRequirements = new Set()
  if (options.includeSlices) enabledRequirements.add("slice")
  if (options.includeHetzner) enabledRequirements.add("hetzner")
  if (options.includeHostedCloud) enabledRequirements.add("hosted-cloud")
  return selectDrillMatrixScenarios({
    scenarios: MATRIX,
    requestedIds: options.only,
    enabledRequirements,
    requirementLabels: {
      slice: "--include-slices",
      hetzner: "--include-hetzner",
      "hosted-cloud": "--include-hosted-cloud",
    },
  })
}

function modelForProvider(provider, options) {
  const defaultModel = provider === "codex"
    ? DEFAULT_CODEX_MODEL
    : DEFAULT_OPENCODE_MODEL
  return resolveProviderModel(provider, {
    defaultModel,
    providerModels: options.providerModels,
  })
}

function commandForScenario(scenarioItem, options) {
  let args = [scenarioItem.script, ...scenarioItem.args]
  if (scenarioItem.script === deterministicRuntimeChaosDrill) {
    args = [...args, "--seed", options.chaosSeed]
    if (options.chaosReplayPath) args = [...args, "--output", path.resolve(options.chaosReplayPath)]
  }
  if (scenarioItem.script === providerThreadTransferDrill && scenarioItem.provider && scenarioItem.provider !== "dev-stub") {
    args = [...args, "--provider-model", `${scenarioItem.provider}=${modelForProvider(scenarioItem.provider, options)}`]
  }
  return {
    command: process.execPath,
    args: appendHetznerPassthrough(args, scenarioItem, options.passthrough),
    env: scenarioItem.env,
  }
}

function metadataFor(selected, options) {
  const includesDeterministicChaos = selected.some((scenarioItem) => scenarioItem.script === deterministicRuntimeChaosDrill)
  const providers = [...new Set([
    ...selected
      .map((scenarioItem) => scenarioItem.providerFamily ?? scenarioItem.provider)
      .filter((provider) => provider && provider !== "dev-stub"),
    ...Object.keys(options.providerAccounts),
  ])].sort()
  return {
    includeSlices: options.includeSlices,
    includeHetzner: options.includeHetzner,
    includeHostedCloud: options.includeHostedCloud,
    generatedMatrixNames: "runtime-resilience-chaos-matrix",
    generatedMatrixRepos: "oss",
    ...(includesDeterministicChaos
      ? {
        deterministicChaosSeed: options.chaosSeed,
        deterministicChaosReplaySchema: DRILL_CHAOS_REPLAY_SCHEMA,
        deterministicChaosFaultKinds: DRILL_CHAOS_FAULT_KINDS.join(","),
        deterministicChaosInvariantIds: DRILL_CHAOS_INVARIANT_IDS.join(","),
      }
      : {}),
    resourceEvidence: "child drills use isolated ports/roots and preserve cleanup artifacts on failure",
    ...drillDeploymentPresetMetadata([
      "local",
      "same-host-remote",
      "self-hosted-relay",
      ...(options.includeHetzner ? ["hetzner"] : []),
      ...(options.includeHostedCloud ? ["hosted-cloud"] : []),
    ], { hetznerPassthrough: options.passthrough }),
    ...providerProfileMetadata({
      providers,
      defaultModel: providers.length > 0 ? "per-provider" : "dev-stub",
      providerAccounts: options.providerAccounts,
      providerModels: options.providerModels,
    }),
  }
}

function defaultRuntimeResilienceReportPath(now = new Date()) {
  const configuredRoot = process.env.CHARIOX_RUNTIME_RESILIENCE_EVIDENCE_ROOT
  if (configuredRoot && !path.isAbsolute(configuredRoot)) {
    throw new Error("CHARIOX_RUNTIME_RESILIENCE_EVIDENCE_ROOT must be an absolute path")
  }
  const evidenceRoot = configuredRoot
    ?? path.join(
      os.homedir(),
      ".codex",
      "evidence",
      "browser-computer-use",
      "runtime-resilience-chaos-matrix",
    )
  const stamp = now.toISOString().replace(/[:.]/g, "-")
  return path.join(evidenceRoot, `${stamp}.json`)
}

function defaultChaosReplayPath(reportPath) {
  const extension = path.extname(reportPath)
  const basename = path.basename(reportPath, extension)
  return path.join(path.dirname(reportPath), `${basename}-chaos-replay.json`)
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  const selected = selectScenarios(options)
  const reportPath = options.reportPath ?? defaultRuntimeResilienceReportPath()
  const artifactIndexPath = options.artifactIndexPath ?? defaultDrillMatrixArtifactIndexPath(reportPath)
  const executionOptions = {
    ...options,
    chaosReplayPath: options.chaosReplayPath ?? defaultChaosReplayPath(reportPath),
  }
  const results = await runDrillMatrix({
    matrixName: "runtime-resilience-chaos-matrix",
    scenarios: selected,
    commandForScenario: (scenarioItem) => commandForScenario(scenarioItem, executionOptions),
    cwd: repoRoot,
    continueOnFailure: options.continueOnFailure,
    dryRun: options.dryRun,
    reportPath,
    artifactIndexPath,
    metadata: metadataFor(selected, options),
  })
  if (results.some((result) => !result.ok)) process.exitCode = 1
}

main().catch((error) => {
  console.error(`[runtime-resilience-chaos-matrix] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
