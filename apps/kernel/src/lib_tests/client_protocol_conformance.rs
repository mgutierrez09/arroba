use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const REQUIRED_SURFACES: &[&str] = &[
    "local_tui",
    "remote_tui",
    "browser_terminal",
    "native_provider_tui",
    "future_native_clients",
];

const REQUIRED_AXES: &[&str] = &[
    "protocol_snapshots",
    "version_rules",
    "reconnect_replay",
    "permissions",
    "attachments",
    "workflow_events",
];

#[derive(Clone, Copy)]
struct Evidence {
    path: &'static str,
    markers: &'static [&'static str],
}

#[test]
fn client_protocol_conformance_gate_covers_required_surfaces_and_axes() {
    let evidence = conformance_evidence();
    let mut failures = Vec::new();

    let expected_surfaces = REQUIRED_SURFACES.iter().copied().collect::<BTreeSet<_>>();
    let actual_surfaces = evidence.keys().copied().collect::<BTreeSet<_>>();
    if actual_surfaces != expected_surfaces {
        failures.push(format!(
            "conformance surfaces mismatch: expected {expected_surfaces:?}, got {actual_surfaces:?}"
        ));
    }

    for surface in REQUIRED_SURFACES {
        let Some(axes) = evidence.get(surface) else {
            failures.push(format!("missing conformance surface `{surface}`"));
            continue;
        };
        for axis in REQUIRED_AXES {
            let Some(items) = axes.get(axis) else {
                failures.push(format!("surface `{surface}` is missing axis `{axis}`"));
                continue;
            };
            if items.is_empty() {
                failures.push(format!("surface `{surface}` axis `{axis}` has no evidence"));
            }
            for item in items {
                assert_evidence_exists(surface, axis, *item, &mut failures);
            }
        }
    }

    assert_file_contains(
        ".github/workflows/ci.yml",
        &[
            "Client protocol conformance gate",
            "@anthropic-ai/claude-code@2.1.212",
            "test \"$(claude --version)\" = \"2.1.212 (Claude Code)\"",
            "cargo test -p chariox-kernel client_protocol_conformance",
            "cargo test --workspace",
            "pnpm test",
        ],
        &mut failures,
    );
    assert_file_contains("package.json", &["\"protocol:conformance\""], &mut failures);

    assert!(
        failures.is_empty(),
        "client protocol conformance gate is incomplete:\n{}",
        failures.join("\n")
    );
}

#[test]
fn local_daemon_protocol_version_matches_typescript_kernel_client() {
    let source = read_repo_file("packages/kernel-client/src/kernel-types.ts");
    let typescript_version = parse_typescript_protocol_version(&source);

    assert_eq!(
        crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
        typescript_version,
        "Rust and TypeScript local daemon protocol versions must stay in lockstep",
    );
}

#[test]
fn kernel_runtime_events_match_typescript_client_contract() {
    let rust_events = rust_kernel_event_names();
    let ts_events = ts_kernel_event_names();

    let missing_in_ts = rust_events
        .difference(&ts_events)
        .cloned()
        .collect::<BTreeSet<_>>();
    assert!(
        missing_in_ts.is_empty(),
        "TypeScript KernelEvent is missing Rust runtime events: {missing_in_ts:?}"
    );

    let unexpected_ts_events = ts_events
        .difference(&rust_events)
        .filter(|event| event.as_str() != "transport_closed")
        .cloned()
        .collect::<BTreeSet<_>>();
    assert!(
        unexpected_ts_events.is_empty(),
        "TypeScript KernelEvent contains unexpected non-synthetic events: {unexpected_ts_events:?}"
    );
}

#[test]
fn cli_dispatch_handles_every_runtime_kernel_event() {
    let rust_events = rust_kernel_event_names();
    let dispatch_cases = ts_dispatch_cases();

    let missing_cases = rust_events
        .difference(&dispatch_cases)
        .cloned()
        .collect::<BTreeSet<_>>();
    assert!(
        missing_cases.is_empty(),
        "CLI kernel event dispatcher is missing runtime event cases: {missing_cases:?}"
    );
    assert!(
        dispatch_cases.contains("transport_closed"),
        "CLI kernel event dispatcher must handle the synthetic transport_closed event",
    );
}

fn conformance_evidence() -> BTreeMap<&'static str, BTreeMap<&'static str, Vec<Evidence>>> {
    BTreeMap::from([
        (
            "local_tui",
            BTreeMap::from([
                (
                    "protocol_snapshots",
                    vec![
                        evidence(
                            "apps/kernel/src/local/api/tests/protocol_shapes/native_spawn_slice.rs",
                            &["LOCAL_DAEMON_PROTOCOL_VERSION", "RequestNativeProviderInteraction"],
                        ),
                        evidence(
                            "apps/kernel/src/local/api/tests/protocol_shapes/provider_usage_activity.rs",
                            &["LOCAL_DAEMON_PROTOCOL_VERSION", "ApplyWorkflowDesignOp"],
                        ),
                        evidence(
                            "apps/kernel/src/local/api/tests/protocol_shapes/credential_enrollment.rs",
                            &[
                                "LOCAL_DAEMON_PROTOCOL_VERSION",
                                "ArmDeploymentCredentialEnrollment",
                                "RequestCredentialEnrollmentInteraction",
                            ],
                        ),
                    ],
                ),
                (
                    "version_rules",
                    vec![
                        evidence(
                            "apps/kernel/src/local/api/types.rs",
                            &["LOCAL_DAEMON_PROTOCOL_VERSION"],
                        ),
                        evidence(
                            "packages/kernel-client/src/kernel-types.ts",
                            &["LOCAL_DAEMON_PROTOCOL_VERSION = "],
                        ),
                    ],
                ),
                (
                    "reconnect_replay",
                    vec![
                        evidence(
                            "apps/cli/scripts/live-cli-kernel-restart-drill.mjs",
                            &["CLI reconnected state", "reconnect|lost connection"],
                        ),
                        evidence(
                            "apps/cli/src/kernel-event-dispatch-controller.test.ts",
                            &["replay_gap", "transport_closed"],
                        ),
                    ],
                ),
                (
                    "permissions",
                    vec![evidence(
                        "apps/cli/scripts/live-workspace-live-sync-permission-drill.mjs",
                        &[
                            "workspace-live-sync-permission-passed",
                            "respondToInteractionRequest",
                        ],
                    )],
                ),
                (
                    "attachments",
                    vec![
                        evidence(
                            "apps/kernel/src/local/api/tests/workspace_capabilities/shell_and_files.rs",
                            &[
                                "local_request_api_rejects_file_capability_for_unauthorized_attachment",
                            ],
                        ),
                        evidence(
                            "apps/kernel/tests/runtime_integration.rs",
                            &["attachments_can_queue_prompts_and_receive_queue_notifications"],
                        ),
                    ],
                ),
                (
                    "workflow_events",
                    vec![
                        evidence(
                            "apps/cli/scripts/live-multi-user-cli-workflow-drill.mjs",
                            &["multi-user-cli-workflow-drill", "workflow run"],
                        ),
                        evidence(
                            "packages/kernel-client/src/kernel-events.ts",
                            &["workflow_design_op"],
                        ),
                    ],
                ),
            ]),
        ),
        (
            "remote_tui",
            BTreeMap::from([
                (
                    "protocol_snapshots",
                    vec![evidence(
                        "apps/kernel/src/transport/relay_client/tests/subscriptions.rs",
                        &["resume_from_event_id", "transport_resumed", "replay_gap"],
                    )],
                ),
                (
                    "version_rules",
                    vec![evidence(
                        "docs/PROTOCOL_CAPABILITY_SESSION_WORKFLOW.md",
                        &["breaking changes require a new major protocol version"],
                    )],
                ),
                (
                    "reconnect_replay",
                    vec![
                        evidence(
                            "apps/cli/scripts/live-remote-restart-drill.mjs",
                            &["remote-restart", "home-restart-ok", "worker-restart-ok"],
                        ),
                        evidence(
                            "apps/kernel/src/transport/relay_client/tests/subscriptions.rs",
                            &["relay_subscription_emits_replay_gap_and_snapshot_for_stale_cursor"],
                        ),
                    ],
                ),
                (
                    "permissions",
                    vec![evidence(
                        "apps/cli/scripts/live-remote-workspace-live-sync-permission-drill.mjs",
                        &["remote-workspace-live-sync-permission-live-drill"],
                    )],
                ),
                (
                    "attachments",
                    vec![evidence(
                        "apps/kernel/src/transport/relay_client/tests/remote_agents.rs",
                        &["remote_machine_agents_materialize_file_attachments_on_the_worker"],
                    )],
                ),
                (
                    "workflow_events",
                    vec![evidence(
                        "apps/cli/scripts/live-remote-workflow-runtime-drill.mjs",
                        &["live-workflow-runtime-drill.mjs", "workflow"],
                    )],
                ),
            ]),
        ),
        (
            "browser_terminal",
            BTreeMap::from([
                (
                    "protocol_snapshots",
                    vec![evidence(
                        "apps/kernel/tests/kernel_websocket_integration/replay_resume.rs",
                        &[
                            "AttachToSessionRequest",
                            "resume_from_event_id",
                            "replay_gap",
                        ],
                    )],
                ),
                (
                    "version_rules",
                    vec![evidence(
                        "packages/kernel-client/src/kernel-types.ts",
                        &["LOCAL_DAEMON_PROTOCOL_VERSION = "],
                    )],
                ),
                (
                    "reconnect_replay",
                    vec![evidence(
                        "apps/cli/src/ipc.test.ts",
                        &[
                            "transport_closed",
                            "transport_resumed",
                            "resume_from_event_id",
                        ],
                    )],
                ),
                (
                    "permissions",
                    vec![evidence(
                        "apps/kernel/src/local/api/tests/waiting_room_projection.rs",
                        &["permission_level", "TerminalType::Web"],
                    )],
                ),
                (
                    "attachments",
                    vec![evidence(
                        "apps/kernel/tests/kernel_websocket_runtime_integration/structured_io.rs",
                        &[
                            "AttachToSessionRequest",
                            "SubmitPromptRequest",
                            "attachments: Vec::new()",
                        ],
                    )],
                ),
                (
                    "workflow_events",
                    vec![evidence(
                        "apps/cli/scripts/live-cloud-relay-drill.mjs",
                        &[
                            "cloud-session-scoped-workflow-assertions",
                            "stale workflow revision mutation",
                        ],
                    )],
                ),
            ]),
        ),
        (
            "native_provider_tui",
            BTreeMap::from([
                (
                    "protocol_snapshots",
                    vec![
                        evidence(
                            "docs/PROTOCOL.md",
                            &["Native TUI permissions:", "Native TUI Agents"],
                        ),
                        evidence(
                            "apps/kernel/src/local/api/tests/protocol_shapes/native_spawn_slice.rs",
                            &["RequestNativeProviderInteraction"],
                        ),
                    ],
                ),
                (
                    "version_rules",
                    vec![evidence(
                        "packages/kernel-client/src/kernel-types.ts",
                        &["LOCAL_DAEMON_PROTOCOL_VERSION = "],
                    )],
                ),
                (
                    "reconnect_replay",
                    vec![evidence(
                        "apps/cli/scripts/live-remote-native-tui-drill.mjs",
                        &["Runs relay-attached native TUI drills", "provider"],
                    )],
                ),
                (
                    "permissions",
                    vec![evidence(
                        "apps/cli/scripts/live-native-tui-permission-drill.mjs",
                        &["permission interaction", "interaction_submit"],
                    )],
                ),
                (
                    "attachments",
                    vec![evidence(
                        "apps/cli/scripts/live-native-tui-attachment-drill.mjs",
                        &["attachments_forwarded", "submitPromptRequest"],
                    )],
                ),
                (
                    "workflow_events",
                    vec![evidence(
                        "apps/cli/scripts/live-native-tui-workflow-drill.mjs",
                        &["native TUI workflow drill", "invokeWorkflowEndpointRequest"],
                    )],
                ),
            ]),
        ),
        (
            "future_native_clients",
            BTreeMap::from([
                (
                    "protocol_snapshots",
                    vec![evidence(
                        "docs/PROTOCOL_CAPABILITY_SESSION_WORKFLOW.md",
                        &[
                            "Cross-Platform Terminal Conformance Profile",
                            "non-web clients should be validated against the same conformance suite and snapshot expectations",
                        ],
                    )],
                ),
                (
                    "version_rules",
                    vec![
                        evidence(
                            "docs/PROTOCOL_CAPABILITY_SESSION_WORKFLOW.md",
                            &["breaking changes require a new major protocol version"],
                        ),
                        evidence(
                            "packages/kernel-client/src/kernel-types.ts",
                            &["LOCAL_DAEMON_PROTOCOL_VERSION = "],
                        ),
                    ],
                ),
                (
                    "reconnect_replay",
                    vec![evidence(
                        "packages/kernel-client/src/kernel-transport-frames.ts",
                        &["resume_from_event_id", "KernelTransportEventFrame"],
                    )],
                ),
                (
                    "permissions",
                    vec![
                        evidence(
                            "packages/kernel-client/src/ipc-terminal-runtime-requests.ts",
                            &[
                                "RequestNativeProviderInteraction",
                                "armDeploymentCredentialEnrollmentRequest",
                                "requestCredentialEnrollmentInteractionRequest",
                            ],
                        ),
                        evidence(
                            "packages/kernel-client/src/credential-enrollment-requests.test.ts",
                            &["LOCAL_DAEMON_PROTOCOL_VERSION", "timeout_sec"],
                        ),
                    ],
                ),
                (
                    "attachments",
                    vec![evidence(
                        "packages/kernel-client/src/ipc-terminal-runtime-requests.ts",
                        &[
                            "storeTransferredFileRequest",
                            "submitPromptRequest",
                            "attachments",
                        ],
                    )],
                ),
                (
                    "workflow_events",
                    vec![evidence(
                        "packages/kernel-client/src/kernel-types-workflow.ts",
                        &["WorkflowDesignOpForwarded", "WorkflowDesignOp"],
                    )],
                ),
            ]),
        ),
    ])
}

fn evidence(path: &'static str, markers: &'static [&'static str]) -> Evidence {
    Evidence { path, markers }
}

fn assert_evidence_exists(surface: &str, axis: &str, item: Evidence, failures: &mut Vec<String>) {
    let path = repo_root().join(item.path);
    if !path.exists() {
        failures.push(format!(
            "surface `{surface}` axis `{axis}` evidence path missing: {}",
            item.path
        ));
        return;
    }

    assert_file_contains(item.path, item.markers, failures);
}

fn assert_file_contains(path: &str, markers: &[&str], failures: &mut Vec<String>) {
    let source = match std::fs::read_to_string(repo_root().join(path)) {
        Ok(source) => source,
        Err(error) => {
            failures.push(format!("could not read `{path}`: {error}"));
            return;
        }
    };

    for marker in markers {
        if !source.contains(marker) {
            failures.push(format!("`{path}` is missing marker `{marker}`"));
        }
    }
}

fn rust_kernel_event_names() -> BTreeSet<String> {
    let source = read_repo_file("apps/kernel/src/transport/kernel_protocol.rs");
    let function_source = source
        .split("pub(crate) fn kernel_event_name")
        .nth(1)
        .and_then(|after_start| after_start.split("pub(crate) fn event_session_id").next())
        .expect("kernel_event_name function should exist");
    extract_quoted_values_after(function_source, "=> ")
}

fn ts_kernel_event_names() -> BTreeSet<String> {
    let source = read_repo_file("packages/kernel-client/src/kernel-events.ts");
    extract_quoted_values_after(&source, "event: ")
}

fn ts_dispatch_cases() -> BTreeSet<String> {
    let source = read_repo_file("apps/cli/src/kernel-event-dispatch-controller.ts");
    extract_quoted_values_after(&source, "case ")
}

fn extract_quoted_values_after(source: &str, prefix: &str) -> BTreeSet<String> {
    let mut values = BTreeSet::new();
    let mut rest = source;
    while let Some(prefix_offset) = rest.find(prefix) {
        rest = &rest[prefix_offset + prefix.len()..];
        let Some(open_quote) = rest.find('"') else {
            break;
        };
        let after_open = &rest[open_quote + 1..];
        let Some(close_quote) = after_open.find('"') else {
            break;
        };
        values.insert(after_open[..close_quote].to_string());
        rest = &after_open[close_quote + 1..];
    }
    values
}

fn parse_typescript_protocol_version(source: &str) -> u32 {
    let marker = "export const LOCAL_DAEMON_PROTOCOL_VERSION = ";
    let version_source = source
        .split(marker)
        .nth(1)
        .expect("TypeScript protocol version export should exist");
    version_source
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>()
        .parse()
        .expect("TypeScript protocol version should be a number")
}

fn read_repo_file(path: &str) -> String {
    std::fs::read_to_string(repo_root().join(path)).unwrap_or_else(|error| {
        panic!("repo file `{path}` should be readable: {error}");
    })
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("kernel crate should live at apps/kernel")
        .to_path_buf()
}
