# Chariox Validation Platform

Chariox runtime features must be validated through reusable drill primitives, not one-off scripts with private conventions. The goal is to make local, remote, hosted, collab, native TUI, slice, and provider-specific behavior comparable across features.

## Shared Primitives

- Artifact lifecycle: use `apps/cli/scripts/lib/drill-artifacts.mjs` to prepare drill roots and preserve failed runs with `chariox-drill-failure.json`.
- Failure summaries: use `apps/cli/scripts/lib/drill-failure-manifest.mjs` or `apps/cli/scripts/drill-failure-summary.mjs` to validate and summarize preserved failed runs without printing credentials or large payloads.
- Failure taxonomy: use `apps/cli/scripts/lib/drill-failure-taxonomy.mjs` for classification owners and next actions shared by failure manifests and matrix reports.
- Runtime signals: use `apps/cli/scripts/lib/drill-runtime-signals.mjs` for stable distributed-runtime signal ids and owner mapping. Do not hand-write signal owner tables in feature drills. Use `runtime-projection-health` for kernel read-model freshness/invariant drift, and `client-projection-health` for web/TUI/native transcript rendering or client-visible projection state.
- Runtime authority invariants: use `apps/cli/scripts/lib/drill-runtime-authority-invariants.mjs` for the stable "clients render and request; kernel decides" contract. Validation-suite artifacts embed this manifest so client, relay, Cloud, worker, and home-kernel authority drift is caught as a contract change instead of staying as prose-only architecture guidance.
- Deterministic chaos: use `apps/cli/scripts/lib/drill-deterministic-chaos.mjs` for the seeded virtual clock, fault transport, and process generations; `drill-chaos-contract.mjs` for replay validation; and `drill-runtime-convergence-invariants.mjs` for no-loss, exactly-once, cursor, authority, convergence, queue, stale-callback, and cleanup assertions. Every failed seed must remain directly replayable.
- Aggregate actions: use `apps/cli/scripts/lib/drill-aggregate-actions.mjs` to group and validate owner/classification/next-action counts consistently across reports.
- Secret hygiene: use `apps/cli/scripts/lib/drill-secrets.mjs` for shared drill metadata redaction and token-shaped value detection.
- Time fields: use `apps/cli/scripts/lib/drill-time.mjs` to validate strict ISO timestamps and report start/end ordering.
- Matrix execution: use `apps/cli/scripts/lib/drill-matrix-runner.mjs` for scenario selection, include-gate enforcement, command rendering, expected-failure handling, failure classification, skipped-scenario accounting, summaries, and reports.
- Environment presets: use `apps/cli/scripts/lib/drill-environment-presets.mjs` for shared Hetzner pass-through parsing and non-secret preset metadata.
- Provider profiles: use `apps/cli/scripts/lib/drill-provider-profiles.mjs` for shared provider list parsing, provider-model overrides, model resolution, and non-secret profile metadata. Provider account aliases are diagnostic labels only; never record provider credentials, emails, tokens, or raw account ids.
- Runtime waits: use `apps/cli/scripts/lib/drill-runtime-helpers.mjs` wait helpers so timeouts include the last observed runtime state.
- Child failure classification: use `apps/cli/scripts/lib/drill-child-process.mjs` so provider auth/account failures are separated from runtime regressions.
- Feature fixtures: put reusable setup and assertions under `apps/cli/scripts/lib/`; entry scripts should stay thin.

Run the shared non-live validation platform checks with:

```bash
pnpm run validation:suite
node apps/cli/scripts/drill-validation-suite.mjs
node apps/cli/scripts/drill-validation-suite.mjs --check
node apps/cli/scripts/drill-validation-suite.mjs --json
node apps/cli/scripts/drill-validation-suite.mjs --json --output .artifacts/drill-validation-suite.json
node apps/cli/scripts/drill-validation-suite.mjs --run-json --output .artifacts/drill-validation-suite-run.json
node apps/cli/scripts/drill-validation-suite.mjs --list
node apps/cli/scripts/drill-validation-suite.mjs --command
```

The `--json` output uses schema `chariox.drill.validation_suite.v1` and lists the exact test paths and command covered by the suite. Use `--output PATH` with `--json` when CI or staging jobs should collect the coverage manifest as an artifact.
The `--run-json` output uses schema `chariox.drill.validation_suite_run.v1`, runs the suite, records pass/fail status, duration, exit code, command, and embeds the manifest. Use `--run-json --output PATH --output-artifact-index PATH` when a staging or release gate needs evidence that the suite actually executed.

Run or replay the deterministic runtime convergence scenario with:

```bash
pnpm --filter @chariox/cli run runtime-resilience:deterministic-chaos-drill -- --seed local-replay
node apps/cli/scripts/live-runtime-resilience-chaos-matrix-drill.mjs \
  --only deterministic-runtime-convergence \
  --chaos-seed local-replay
```

The replay schema is `chariox.drill.chaos_replay.v1`. It records the seed, fault plan, monotonic virtual-time trace, invariant evidence, queue/resource summaries, and stale-callback suppression without recording credentials or payload secrets. The resilience matrix captures replay paths from successful as well as failed children so a passing baseline and a regression can both be audited.

Export the shared failure taxonomy with:

```bash
node apps/cli/scripts/drill-failure-taxonomy.mjs
node apps/cli/scripts/drill-failure-taxonomy.mjs --target drill --output .artifacts/drill-failure-taxonomy.json
```

Collect the shared platform contracts as one artifact bundle with:

```bash
node apps/cli/scripts/drill-platform-bundle.mjs --output-dir .artifacts/drill-platform
node apps/cli/scripts/drill-platform-bundle.mjs --verify-dir .artifacts/drill-platform
```

The bundle writes `index.json`, `validation-suite.json`, `failure-taxonomy-scenario.json`, and `failure-taxonomy-drill.json`.
The verifier checks the bundle index, required artifact set, relative artifact paths, and artifact schema consistency.

Gate collected artifacts before treating a drill run as release/staging evidence:

```bash
node apps/cli/scripts/drill-validation-gate.mjs \
  --platform-bundle .artifacts/drill-platform \
  --artifact-root .artifacts \
  --require-artifact-coverage-area distributed-observability \
  --require-artifact-schema chariox.drill.validation_suite_run.v1 \
  --require-artifact-exit-criterion-status satisfied \
  --require-runtime-signal-owner kernel-authority \
  --require-artifact-provider-account-alias codex=work \
  --require-artifact-planned-owner runtime-state \
  --require-artifact-planned-classification workspace-live-sync-conflict \
  --require-artifact-max-age-ms 3600000 \
  --matrix-root .artifacts/drill-matrices \
  --require-matrix-max-age-ms 3600000 \
  --failure-root .artifacts \
  --require-failure-max-age-ms 3600000 \
  --require-complete \
  --json --output .artifacts/drill-validation-gate.json
```

The gate output schema is `chariox.drill.validation_gate.v1`. It fails when no checks are configured, verifies the platform bundle, verifies indexed artifacts, can require platform runtime-signal owner coverage with `--require-runtime-signal-owner`, can require artifact metadata coverage with `--require-artifact-coverage-area`, can require artifact schema coverage with `--require-artifact-schema`, can require artifact exit-criterion status evidence with `--require-artifact-exit-criterion-status` and `--require-artifact-incomplete-exit-criterion-status`, can require non-secret provider account labels with `--require-artifact-provider-account-alias provider=alias`, can require dry-run matrix planned diagnostic coverage with `--require-artifact-planned-owner` and `--require-artifact-planned-classification`, can reject stale artifact indexes with `--require-artifact-max-age-ms`, can reject stale matrix reports with `--require-matrix-max-age-ms`, can reject stale preserved failure bundles with `--require-failure-max-age-ms`, fails when selected matrix reports failed or are incomplete under `--require-complete`, and fails when preserved failure manifests are present. Validation-gate report and aggregate `presets` fields must use the closed preset registry; typoed labels are rejected instead of counted as new coverage. Failed gate reports include `nextActions` grouped by owner/classification so CI and staging operators can route the next fix without opening raw logs first; missing planned owners and classifications route to the owning subsystem instead of only producing a generic artifact-index failure. Text summaries include discovered artifact coverage areas, artifact schema counts, runtime signal counts, runtime signal owner counts, provider account alias labels, planned owner/classification labels, stale artifact indexes, stale matrix reports, stale failure manifests, exit-criterion status labels, and incomplete exit-criterion status labels for artifact, matrix, failure, and validation-gate aggregate evidence so operators can see which evidence types, provider profiles, dry-run limitations, and runtime subsystems were present without opening raw JSON. `drill-artifact-index-summary.mjs` accepts the same `--require-artifact-max-age-ms` and `--require-matrix-max-age-ms` freshness flags when CI needs to reject stale evidence before it reaches a validation gate.
Use `--matrix-report PATH` and `--failure-manifest PATH` when CI already knows the exact artifact paths and should avoid broad discovery.
The distributed-runtime preset requires artifact coverage area `distributed-observability`, schema `chariox.drill.validation_suite_run.v1`, and exit-criterion status `satisfied`, so release/staging evidence must include a passing executed validation-suite report whose artifact index metadata proves the distributed-observability checks ran rather than only publishing a coverage manifest or a generic suite run. Pass `--include-default-artifacts` or explicit artifact indexes so the gate can verify that schema, coverage, and status metadata. When the executable suite evidence is missing, the gate's next action points operators to rerun the suite with `--run-json --output PATH --output-artifact-index PATH`; when the coverage area is missing, the next action points operators to run validation-suite artifacts covering `distributed-observability`.

For one-command distributed-runtime evidence generation, use:

```bash
pnpm run validation:distributed-runtime-gate

node apps/cli/scripts/drill-distributed-runtime-gate.mjs \
  --run-validation-suites \
  --run-matrix-reports \
  --include-default-failures \
  --require-generated-matrix-registry-parity \
  --require-chaos-contract-registry-parity \
  --require-complete \
  --json --output .artifacts/drill-validation-gate/distributed-runtime.json \
  --output-artifact-index .artifacts/drill-validation-gate/chariox-drill-artifacts.json
```

The wrapper runs the OSS and Cloud validation suites and the distributed matrix scripts, then feeds their generated artifact indexes and matrix roots into the distributed-runtime preset. Use `--require-generated-matrix-registry-parity` and `--require-chaos-contract-registry-parity` whenever the gate spans both repos; they fail before execution if generated matrix ownership or chaos replay schemas, fault kinds, and invariants disagree. Cloud staging smoke enables both checks for the distributed-runtime gate. JSON/file reports include `generatedEvidence`, which records whether suites/matrices were generated, the generated roots, validation-suite artifact indexes, validation-suite failure roots in `validationSuites.failureRoots`, generated matrix artifact indexes in `matrixReports.artifactIndexes`, matrix report paths, command arguments, artifact-index flags, and replayable `nodeArgs` for generated child commands. Gate report and aggregate validators reject generated command records without `nodeArgs` and reject unknown generated matrix artifact-index flags, so replay metadata cannot be silently dropped or made ambiguous. Generated child-command failures include the owning repo/matrix or validation-suite label alongside the cwd, report, artifact index, and output. Generated validation-suite commands pass `--preserve-failure-root`, so a failed generated OSS or Cloud suite leaves `chariox-drill-failure.json` below its generated validation-suite output root.

Cross-repo registry parity checks are strict contracts, not label counters. Generated-matrix parity validates the OSS schema `chariox.drill.generated_matrix_names.v1`, the Cloud schema `chariox.cloud.drill.generated_matrix_names.v1`, unique matrix names, and exact matrix repo ownership before comparing the two registries. Chaos-contract parity validates schema `chariox.drill.chaos_contract.v1`, replay and invariant report schemas, unique fault kinds, and unique invariant ids. Runtime-signal parity validates schema `chariox.drill.runtime_signals.v1`, unique signal ids, owner, and description parity. Failure-taxonomy parity validates schema `chariox.drill.failure_taxonomy.v1`, target `scenario`, unique classification kinds, Cloud classifications known to OSS, and owner parity except for documented Cloud-context owner overrides. Keep these checks enabled in staging gates so a typo, stale generated matrix name, duplicate contract value, or malformed Cloud registry fails before live drills spend provider or remote-machine time.

Validation-gate aggregates copy that provenance into each report summary and count `coverage.generatedEvidenceKinds`, `coverage.generatedMatrixArtifactIndexes`, `coverage.generatedMatrixNames`, `coverage.generatedMatrixRepos`, `coverage.generatedValidationSuiteArtifactIndexes`, and `coverage.generatedValidationSuiteFailureRoots`, so a higher-level gate can prove whether it consumed generated validation-suite runs, generated matrix reports, generated matrix identity metadata, generated validation-suite artifact indexes, generated matrix artifact indexes, discovered evidence, explicit evidence, and preserved generated-suite failure bundles. Aggregate summaries also preserve stale matrix report sources as `coverage.matrixStaleReports` and stale failure manifest sources as `coverage.failureStaleManifests`, so bundled evidence still points to the exact report or preserved failure bundle that must be regenerated. Use `drill-validation-gate-summary.mjs --require-generated-evidence-kind validation-suite-run --require-generated-evidence-kind matrix-report --require-generated-validation-suite-artifact-index PATH --require-generated-matrix-artifact-index PATH --require-generated-validation-suite-failure-root PATH` when CI must reject a stale/discovered-only gate bundle or one that omits generated validation-suite artifact-index, generated matrix artifact-index, or failure-root provenance.

Artifact-index gates can enforce the same provenance with `--require-artifact-generated-evidence-kind validation-suite-run --require-artifact-generated-evidence-kind matrix-report --require-artifact-generated-validation-suite-artifact-index PATH --require-artifact-generated-matrix-artifact-index PATH`. `drill-artifact-index-summary.mjs` can also require generated matrix identity with `--require-generated-matrix-name NAME` and `--require-generated-matrix-repo REPO`, generated validation-suite artifact-index provenance with `--require-generated-validation-suite-artifact-index PATH`, a preserved generated-suite failure bundle with `--require-generated-validation-suite-failure-root PATH`, non-secret provider account profile labels with `--require-provider-account-alias PROVIDER=ALIAS`, dry-run planned diagnostics with `--require-planned-owner OWNER` and `--require-planned-classification CLASSIFICATION`, and runtime owner coverage with `--require-runtime-signal-owner OWNER`. Gate presets derive required runtime-signal owners from their required runtime signals, and the explicit owner flag remains available when a staging gate needs proof from a subsystem such as `kernel-authority`, `runtime-network`, or `worker-kernel` without hard-coding every signal owned by that subsystem. Artifact-index aggregates preserve `generatedValidationSuiteArtifactIndexes`, `generatedMatrixArtifactIndexes`, `generatedMatrixNames`, `generatedMatrixRepos`, `generatedValidationSuiteFailureRoots`, `providerAccountAliases`, `plannedOwners`, and `plannedClassifications`, and validation-gate aggregate coverage exposes the same roots, names, repos, and aliases as `artifactGeneratedValidationSuiteArtifactIndexes`, `artifactGeneratedMatrixArtifactIndexes`, `artifactGeneratedMatrixNames`, `artifactGeneratedMatrixRepos`, `artifactGeneratedValidationSuiteFailureRoots`, `artifactProviderAccountAliases`, `artifactPlannedOwners`, and `artifactPlannedClassifications`, so downstream staging jobs can route generated validation-suite artifact indexes, generated matrix artifact indexes, generated matrix identity, preserved generated suite failures, missing dry-run planned diagnostics, and missing provider profile evidence without opening every source index. Artifact-index summary JSON also includes validated `nextActions` grouped by owner/classification/action for stale artifacts, missing generated evidence, missing generated matrix identity, missing provider account labels, missing planned diagnostics, and missing runtime-signal coverage. Output artifact metadata also records generated evidence kinds, generated roots, generated validation-suite artifact indexes, required or missing generated validation-suite artifact indexes, generated matrix artifact indexes, required or missing generated matrix artifact indexes, generated matrix names/repos, required or missing generated matrix names/repos, generated validation-suite failure roots, missing required failure roots, provider account aliases, missing required provider account aliases, runtime signal requirements, runtime owner requirements, planned owner requirements, and planned classification requirements, so downstream staging jobs can distinguish gate-generated evidence from discovered or explicitly supplied evidence. Cloud staging smoke reports add artifact hints for the generated validation-suite and matrix roots when `--generate-distributed-runtime-gate-evidence` is enabled.

When the distributed-runtime wrapper generates matrix reports with `--matrix-dry-run`, it validates matrix names, scenarios, deployment presets, providers, runtime signals, validation-suite run artifacts, generated-evidence provenance, and planned diagnostic ownership, but it does not treat dry-run scenarios as release-grade failure-classification evidence. In that mode the gate records generated matrix limitation `dry-run-classification-coverage` and suppresses only the preset-derived matrix classification requirement. Follow it with `drill-validation-gate-summary.mjs --require-generated-matrix-limitation dry-run-classification-coverage --require-artifact-generated-matrix-limitation dry-run-classification-coverage --require-artifact-planned-owner OWNER --require-artifact-planned-classification CLASSIFICATION --require-artifact-max-age-ms 3600000` to prove the limitation was explicit, the planned owner/classification metadata is present, and the artifact metadata inputs are fresh. Rerun without `--matrix-dry-run` before treating the distributed-runtime gate as release evidence.

Use focused gates before the release-level distributed-runtime gate when the
change is isolated to one quality axis:

```bash
pnpm run validation:focused-runtime-gate
pnpm --filter @chariox/cli run validation:focused-runtime-gate

node apps/cli/scripts/drill-focused-runtime-gate.mjs \
  --matrix-root .artifacts/drill-matrices \
  --require-complete

node apps/cli/scripts/drill-validation-gate.mjs \
  --preset runtime-authority \
  --platform-bundle .artifacts/drill-platform \
  --matrix-root .artifacts/drill-matrices \
  --require-complete

node apps/cli/scripts/drill-validation-gate.mjs \
  --preset distributed-state-health \
  --platform-bundle .artifacts/drill-platform \
  --matrix-root .artifacts/drill-matrices \
  --require-complete
```

`runtime-authority` proves the shared kernel-owned session, agent, provider-run,
permission, and projection path across native TUI, remote agent, and slice
matrices. `distributed-state-health` proves owner-routed diagnostics and
recovery evidence for leases, provider-run lifecycle, relay freshness, slice
state, home-extension manifest sync, and Workspace Live Sync state. Run both
focused presets after refactors in those areas, then run `distributed-runtime`
once remote, hosted, collab, provider, and UI evidence has been collected.
Dry-run matrix evidence is acceptable only for scope review; it must carry
planned owner/classification metadata and cannot satisfy release evidence until
the matching live matrix has completed.

## Room drill memory

`live-room-environment-pointer-click-drill.mjs` requests 2,048 MiB for its slice
by default. Set `CHARIOX_ROOM_DRILL_MEMORY_MB` to a positive integer number of
MiB to test a different limit, for example `1280` when running a short drill
alongside an existing soak. Invalid values fail before creating drill state.
The drill checks the actual Docker memory limit against the requested value
and still requires no additional swap, one CPU, and a 1,024-process limit.
This override does not bypass the kernel's memory admission check. Choose a
limit that fits the VM's existing reservations and the workload being tested.

## Matrix Reports

Matrix scripts write JSON with schema `chariox.drill.matrix.v1`. Use `defaultDrillMatrixReportPath(...)` so reports are written under `.artifacts/drill-matrices/<matrix>/<timestamp>.json` when the caller does not pass `--report PATH`. `--report PATH` remains the override for CI jobs or custom collection directories.

Summarize one or more reports with:

```bash
node apps/cli/scripts/drill-matrix-report-summary.mjs path/to/matrix-report.json
node apps/cli/scripts/drill-matrix-report-summary.mjs --find .artifacts/drill-matrices
node apps/cli/scripts/drill-matrix-report-summary.mjs --find .artifacts --max-depth 4
node apps/cli/scripts/drill-matrix-report-summary.mjs --require-complete --find .artifacts/drill-matrices
node apps/cli/scripts/drill-matrix-report-summary.mjs --json --output path/to/aggregate.json path/to/*.json
```

The summary command exits non-zero when any input report has `status=failed`. `--find ROOT` discovers valid matrix reports below an artifact root and ignores unrelated JSON files.
Matrix report discovery is bounded by default and prunes heavy irrelevant directories such as `.git`, `node_modules`, `.pnpm-store`, `debug`, and `release`, so broad artifact roots are safe to scan in CI. Use `--max-depth N` when a CI artifact layout needs a tighter or wider traversal bound.
For dry-run reports, it prints selected scenario exit criteria so reviewers can confirm matrix scope before running live drills.
Use `--require-complete` for release/staging gates that must reject skipped or dry-run scenarios even when no scenario failed.
When more than one report is selected, the human summary prints an aggregate section with total coverage, failure owners, next actions, and incomplete scenarios.
Failed-scenario summaries include an owner and next action so humans and CI can route the fix without opening raw logs first.
The shared taxonomy can also be exported as schema `chariox.drill.failure_taxonomy.v1` by calling `drillFailureTaxonomyManifest(...)` from `apps/cli/scripts/lib/drill-failure-taxonomy.mjs`; use this instead of duplicating classification tables in feature drills or UI diagnostics.
The `--json`/`--output` aggregate schema is `chariox.drill.matrix.aggregate.v1`; its `reports`, `failedScenarios`, and `incompleteScenarios` entries include the originating report `source` path when available. Its `reports` entries must have internally consistent status/count/duration fields, and aggregate totals must equal the sum of those entries. Its `owners` map counts failed-scenario owners, `nextActions` groups repeated owner/classification/action pairs, its `failedScenarios` entries include `classification`, `owner`, `reason`, `artifactHints`, and `nextAction`, and its `incompleteScenarios` entries list skipped and dry-run coverage gaps.

Required top-level fields:

- `schema`: always `chariox.drill.matrix.v1`.
- `matrix`: stable matrix name.
- `status`: `passed`, `failed`, or `dry-run`; it must match scenario statuses.
- `dryRun`: boolean; true only when every selected scenario is `dry-run`.
- `startedAt`, `completedAt`, `durationMs`.
- `metadata`: feature-specific non-secret context such as enabled scenario groups, provider model ids, or provider account aliases.
- `scenarios`: selected scenario results; reports with no selected scenarios are invalid.

Distributed-runtime matrices must also include `runtimeSignals`, an aggregate count object for scenario runtime-signal ids, and `runtimeSignalScenarios`, a map from signal id to the scenario rows that provide that evidence. OSS matrix runners emit those top-level summaries whenever selected scenarios declare runtime signals, and also emit top-level `exitCriteria` and `incompleteExitCriteria` summaries from scenario evidence. Validators reconcile `runtimeSignalScenarios` with signal counts, known scenario ids, and failed/skipped/dry-run scenario diagnostics so stale or hand-edited evidence cannot point at nonexistent coverage or misreport scenario status.

Top-level `durationMs` must equal `completedAt - startedAt` exactly. This keeps generated reports comparable across local, remote, hosted, and collab matrices and catches hand-authored report drift.

Required scenario fields:

- `id`, `description`, `requires`.
- `exitCriteria`: proof points the scenario is expected to establish, or an empty array.
- `status`: `passed`, `failed`, `skipped`, or `dry-run`.
- `expectedFailure`, `classification`, `durationMs`, `reason`.
- `command`, `args`.
- `artifactHints`: optional paths to preserved artifact roots or failure manifests discovered from child drill output.

Distributed-runtime scenario rows should include `runtimeSignals`: stable signal ids covered by the scenario, such as `session-authority`, `lease-health`, or `workspace-live-sync-state`.

Runtime signal owners are derived from `drill-runtime-signals.mjs`. Matrix reports should emit `metadata.runtimeSignalOwners` and summary/gate artifacts should preserve owner counts rather than requiring operators to infer ownership from raw signal ids.

Artifact indexes and aggregate summaries treat runtime-signal metadata as validated evidence, not labels. `runtimeSignals` entries must be known signal ids, and per-index `runtimeSignalOwners` counts must be derived from those signal ids so typoed or hand-edited diagnostics fail before they reach CI summaries.

Validation-suite preset contracts use the same runtime-signal registry. Required runtime-signal fields and runtime-signal owner fields in presets must be canonical values before the suite manifest can be written. Preset-level `requiredRuntimeSignalOwners` is derived from `requiredRuntimeSignals`; hand-authored owner lists are accepted only when they exactly match the derived owners. Validation-suite artifact metadata records both observed runtime-signal coverage and required runtime-signal coverage so downstream gates can distinguish "this evidence covered a signal" from "this suite contract requires a signal."

Matrix runners preflight selected scenario definitions and all selected scenario commands before spawning child drills. Scenario ids must be unique and non-empty, selected scenarios must include descriptions, `requires` and `exitCriteria` must be string arrays or valid strings where supported, and `commandForScenario` must return a non-empty command with string args for every selected scenario.

Scenario outcome fields must be internally consistent. Failed scenarios require a non-empty `reason` and `classification` so CI can route the failure. Skipped scenarios require a non-empty `reason` and zero `durationMs`. Dry-run scenarios require zero `durationMs` and must not include a `reason` or `classification`. Passed scenarios must not include a `reason`.

Reports must not include credentials, relay tokens, provider tokens, prompt bodies, file contents, or unredacted connector payloads. If a drill needs detailed failure output, preserve the artifact directory and record a pointer in the failure manifest instead of embedding sensitive logs in the matrix report.
The matrix report validator rejects secret-looking metadata keys, token-shaped metadata values, and token-shaped artifact hints. Matrix runners validate before writing reports and skip token-shaped artifact hints extracted from child output. Child failure summaries redact token-shaped values before preserving output tails.

## Failure Manifests

Failed drills that preserve artifacts write `chariox-drill-failure.json` with schema `chariox.drill.failure.v1`.

Summarize one or more preserved roots or manifest files with:

```bash
node apps/cli/scripts/drill-failure-summary.mjs path/to/preserved-root
node apps/cli/scripts/drill-failure-summary.mjs path/to/chariox-drill-failure.json
node apps/cli/scripts/drill-failure-summary.mjs --find apps/cli/target .artifacts
node apps/cli/scripts/drill-failure-summary.mjs --find --max-depth 4 .artifacts
node apps/cli/scripts/drill-failure-summary.mjs --json --output path/to/failure-aggregate.json --find apps/cli/target .artifacts
node apps/cli/scripts/drill-failure-summary.mjs --find --require-failure-max-age-ms 3600000 .artifacts
```

Required top-level fields:

- `schema`: always `chariox.drill.failure.v1`.
- `rootDir`: preserved artifact root.
- `failedAt`: ISO timestamp for the failure.
- `metadata`: non-secret drill context such as drill name, provider profile, relay mode, or scenario id.
- `error`: error name, message, and optional stack.

Failure manifests redact sensitive metadata keys, token-shaped metadata values, and token-shaped error text before writing. The validator rejects token-shaped metadata values, including nested values, so externally supplied manifests cannot pass with credentials embedded. Failure summaries also redact sensitive metadata keys, token-shaped error messages, and omit nested values. Keep raw logs, screenshots, and packet captures in the preserved artifact root, not in the manifest.
When more than one failure manifest is selected, the summary command prints an aggregate owner/classification section so preserved failure batches can be routed quickly.
Failure manifest discovery is bounded by default, prunes heavy irrelevant directories, and accepts `--max-depth N` for CI layouts that need a tighter or wider traversal bound.
The `--json`/`--output` aggregate schema is `chariox.drill.failure.aggregate.v1`.
Its `nextActions` entries group repeated owner/classification/action pairs so CI and humans can route a failure batch without scanning every manifest.
Each failure entry includes the drill name, preserved artifact root, optional manifest `source` path, owner, classification, and next action.
Use `--require-failure-max-age-ms MS` when CI must reject stale preserved failure bundles instead of routing old failures as current blockers; aggregate output then includes `requiredFailureMaxAgeMs` and `staleFailureManifests`.

## Scenario Selection

Default runs should stay fast and local. Expensive or environment-dependent groups must be behind explicit include flags such as `--include-remote`, `--include-hetzner`, or `--include-opencode`. Selecting a gated scenario by `--only` must still require its include flag, so accidental partial validation fails loudly.

When a matrix stops after the first failure, remaining selected scenarios must be reported as `skipped`. With `--continue-on-failure`, every selected scenario should run unless its own setup fails.

## Exit Criteria

Every new runtime feature should define:

- Unit tests for policy, protocol shape, and edge cases.
- At least one local drill that proves the kernel-owned runtime path.
- Matrix scenarios for remote, hosted, collab, native TUI, slice, and provider variants when the feature crosses those surfaces.
- Failure artifacts sufficient to identify the failing owner: provider account/auth, relay, kernel authority, worker execution, UI/client projection, or test harness.
- Diagnostic wait failures that include `last_observation` should classify as runtime state timeouts unless the failure is clearly relay, cloud, provider auth/account, Docker, or test harness owned.
- Distributed-runtime failures should classify to the narrowest actionable owner: `kernel-authority` for rejected session/agent/lease/provider-run bindings and duplicate provider-run binding health signals, `runtime-projection-health` for kernel read-model freshness/invariant drift tied to that runtime signal, `projection-staleness` for stale projections or failed projection reconciliation in older/general drills, `worker-execution` for leased-agent or remote worker launch/execution failures, `ui-client-projection` for web/TUI terminal or transcript projection failures, `remote-extension-sync` for home/worker manifest drift, `workspace-live-sync-conflict` for sync conflicts or external changes, `slice-auth` for missing or wrong provider accounts inside slices, and `slice-runtime` for slice launch/container lifecycle failures.
- A reportable command that can be run by humans and CI without editing the script.
- Default matrix report output under `.artifacts/drill-matrices` so dry-runs and live runs leave auditable evidence without requiring a custom `--report` flag.

Feature work is not complete until the relevant matrix reports prove the intended scope or clearly classify external blockers that prevented validation.
