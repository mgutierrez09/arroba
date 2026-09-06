use super::*;
use crate::local::{
    CancelProviderLoginRequest, GetProviderLoginStatusRequest, ProviderLoginProcessState,
    ProviderLoginStatus, SendProviderLoginInputRequest, StartProviderLoginRequest,
};
use crate::provider::ProviderLoginStart;
use sha2::{Digest, Sha256};

#[test]
fn local_daemon_protocol_provider_credential_policy_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);
    let response = LocalDaemonResponse::CredentialsListed {
        credentials: vec![crate::config::UserCredentialConfig {
            id: "claude-profile-token".to_string(),
            description: None,
            source: crate::config::UserCredentialSourceConfig::Vault {
                key: "provider-account-claude".to_string(),
            },
            allowed_hosts: Vec::new(),
            allowed_uses: vec![crate::config::UserCredentialUse::Provider],
            injection: crate::config::UserCredentialInjectionConfig::Provider,
            metadata: None,
        }],
    };
    let serialized = serde_json::to_string(&response).expect("credential response should encode");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&serialized)
            .expect("credential response should decode"),
        serde_json::json!({
            "CredentialsListed": {
                "credentials": [{
                    "id": "claude-profile-token",
                    "source": {"type": "vault", "key": "provider-account-claude"},
                    "allowed_uses": ["provider"],
                    "injection": {"kind": "provider"}
                }]
            }
        })
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(serialized.as_bytes())),
        "872fcb6015e458ab2cccee1292da8b25a9af94cae376a8630961aafc2d14fbad"
    );
}

#[test]
fn local_daemon_protocol_provider_catalog_selection_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);
    let request = LocalDaemonRequest::GetProviderCatalog(crate::local::GetProviderCatalogRequest {
        provider: Some("codex".to_string()),
        account_profiles: std::collections::BTreeMap::from([(
            "codex".to_string(),
            "work".to_string(),
        )]),
        execution_location: crate::local::api::ProviderCatalogExecutionLocation::Slice {
            slice_ref: "slice-a".to_string(),
        },
    });
    let snapshot = serde_json::to_value(request).expect("catalog request should encode");
    assert_eq!(
        snapshot.pointer("/GetProviderCatalog/account_profiles/codex"),
        Some(&serde_json::json!("work"))
    );
    assert_eq!(
        snapshot.pointer("/GetProviderCatalog/execution_location/kind"),
        Some(&serde_json::json!("slice"))
    );
    let serialized = serde_json::to_string(&snapshot).expect("catalog snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "f082c2dfd494f8f55dd5865e571927218b31320c42c196feae8d9f44970ef733"
    );
}

#[test]
fn local_daemon_protocol_provider_terminal_login_shape_is_versioned_and_redacted() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let requests = vec![
        LocalDaemonRequest::GetProviderLoginStatus(GetProviderLoginStatusRequest {
            login_id: "provider-login-1".to_string(),
        }),
        LocalDaemonRequest::SendProviderLoginInput(SendProviderLoginInputRequest {
            login_id: "provider-login-1".to_string(),
            data_base64: "c2VjcmV0Cg==".to_string(),
        }),
        LocalDaemonRequest::CancelProviderLogin(CancelProviderLoginRequest {
            login_id: "provider-login-1".to_string(),
        }),
    ];
    let status = ProviderLoginStatus {
        provider: "claude".to_string(),
        account_profile: "work".to_string(),
        login_id: "provider-login-1".to_string(),
        state: ProviderLoginProcessState::Running,
        interaction: None,
        terminal_output_base64: "c2VjcmV0LW91dHB1dA==".to_string(),
        started_at_ms: 1_000,
        updated_at_ms: 1_100,
    };
    let response = LocalDaemonResponse::ProviderLoginStatus {
        login: status.clone(),
    };
    let logout = ProviderLoginStart {
        provider: "opencode".to_string(),
        account_profile: "work".to_string(),
        login_kind: "terminal_logout".to_string(),
        login_id: Some("provider-login-2".to_string()),
        auth_url: Some("https://auth.example/secret".to_string()),
        verification_url: None,
        user_code: Some("SECRET-CODE".to_string()),
    };
    let logout_response = LocalDaemonResponse::ProviderLogoutStarted {
        logout: logout.clone(),
    };

    let snapshot = serde_json::json!([requests, response, logout_response]);
    assert_eq!(
        snapshot.pointer("/0/1/SendProviderLoginInput/data_base64"),
        Some(&serde_json::json!("c2VjcmV0Cg=="))
    );
    assert_eq!(
        snapshot.pointer("/1/ProviderLoginStatus/login/state"),
        Some(&serde_json::json!("running"))
    );
    let debug = format!("{status:?}");
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains("c2VjcmV0LW91dHB1dA=="));
    let request_debug = format!("{:?}", requests[1]);
    assert!(request_debug.contains("[REDACTED]"));
    assert!(!request_debug.contains("c2VjcmV0Cg=="));
    let logout_debug = format!("{logout:?}");
    assert!(logout_debug.contains("[REDACTED]"));
    assert!(!logout_debug.contains("SECRET-CODE"));
    assert!(!logout_debug.contains("auth.example"));

    let serialized =
        serde_json::to_string(&snapshot).expect("provider login snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "946209baece227091714d82a9cd15e2f01487e331e8da080be8617371afa8522"
    );
}

#[test]
fn local_daemon_protocol_provider_account_profile_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let request = LocalDaemonRequest::CreateProviderAccountProfile(
        crate::local::api::CreateProviderAccountProfileRequest {
            provider: "codex".to_string(),
            label: "Work".to_string(),
        },
    );
    let import_native = LocalDaemonRequest::ImportNativeProviderAccountProfile(
        crate::local::api::ImportNativeProviderAccountProfileRequest {
            provider: "claude".to_string(),
        },
    );
    let response = LocalDaemonResponse::ProviderAccountProfile {
        profile: crate::account_profile::ProviderAccountProfile {
            owner_user_id: "user-1".to_string(),
            provider: "codex".to_string(),
            profile_id: "work".to_string(),
            label: "Work".to_string(),
            origin: crate::account_profile::ProviderAccountProfileOrigin::CharioxCreated,
            is_default: false,
            auth_state: crate::account_profile::ProviderAccountAuthState::Authenticated,
            credential_kind: Some(crate::account_profile::ProviderCredentialKind::Subscription),
            credential_kind_not_reported_reason: None,
            identity_summary: Some("work@example.com".to_string()),
            plan: Some("pro".to_string()),
            detected_provider_version: Some("1.2.3".to_string()),
            last_validated_at_ms: Some(1_234),
            usage: crate::account_profile::ProviderAccountUsageSnapshot {
                profile_id: "work".to_string(),
                provider: "codex".to_string(),
                availability: crate::account_profile::ProviderAccountUsageAvailability::Available,
                meters: vec![crate::account_profile::ProviderAccountUsageMeter {
                    meter_id: "primary".to_string(),
                    label: "5 hour limit".to_string(),
                    kind: crate::account_profile::ProviderAccountUsageMeterKind::RollingLimit,
                    scope: crate::account_profile::ProviderAccountUsageMeterScope::Account,
                    used_percent: Some(25.0),
                    used: None,
                    remaining: None,
                    total: None,
                    unit: None,
                    window_duration_minutes: Some(300),
                    resets_at_ms: Some(9_876),
                    state: crate::account_profile::ProviderAccountUsageMeterState::Healthy,
                    source: "codex_app_server".to_string(),
                    observed_at_ms: 1_234,
                }],
                observed_at_ms: Some(1_234),
                source: "codex_app_server".to_string(),
                management_url: Some("https://chatgpt.com/codex/settings/usage".to_string()),
            },
            materializations: vec![
                crate::account_profile::ProviderAccountMaterializationStatus {
                    target_kind:
                        crate::account_profile::ProviderAccountMaterializationTargetKind::Slice,
                    target_ref: "slice-1".to_string(),
                    state:
                        crate::account_profile::ProviderAccountMaterializationState::Materialized,
                    observed_at_ms: 1_235,
                    last_error: None,
                },
            ],
        },
    };

    let login_request = LocalDaemonRequest::StartProviderLogin(StartProviderLoginRequest {
        provider: "codex".to_string(),
        account_profile: "work".to_string(),
        method: Some("device_code".to_string()),
    });
    let login_default_method = LocalDaemonRequest::StartProviderLogin(StartProviderLoginRequest {
        provider: "codex".to_string(),
        account_profile: "work".to_string(),
        method: None,
    });

    let snapshot = serde_json::json!([
        request,
        import_native,
        response,
        login_request,
        login_default_method,
    ]);
    assert_eq!(
        snapshot.pointer("/0/CreateProviderAccountProfile/label"),
        Some(&serde_json::json!("Work"))
    );
    assert_eq!(
        snapshot.pointer("/1/ImportNativeProviderAccountProfile/provider"),
        Some(&serde_json::json!("claude"))
    );
    assert!(
        serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
            "ImportNativeProviderAccountProfile": {
                "provider": "claude",
                "path": "/client/supplied/path"
            }
        }))
        .is_err()
    );
    assert_eq!(
        snapshot.pointer("/2/ProviderAccountProfile/profile/usage/meters/0/used_percent"),
        Some(&serde_json::json!(25.0))
    );
    assert_eq!(
        snapshot.pointer("/2/ProviderAccountProfile/profile/credential_kind"),
        Some(&serde_json::json!("subscription"))
    );
    let profile_object = snapshot
        .pointer("/2/ProviderAccountProfile/profile")
        .and_then(serde_json::Value::as_object)
        .expect("profile payload should be an object");
    assert!(!profile_object.contains_key("path"));
    assert!(!profile_object.contains_key("locator"));
    assert!(!profile_object.contains_key("credential_kind_not_reported_reason"));
    // Method selection is explicit on the wire; omission keeps the historical
    // default without adding a key.
    assert_eq!(
        snapshot.pointer("/3/StartProviderLogin/method"),
        Some(&serde_json::json!("device_code"))
    );
    let omitted_method = snapshot
        .pointer("/4/StartProviderLogin")
        .and_then(serde_json::Value::as_object)
        .expect("default-method request should be an object");
    assert!(!omitted_method.contains_key("method"));

    let serialized =
        serde_json::to_string(&snapshot).expect("provider account profile snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "604a27fc9fe5141724b89dc0d0b5ac981742fe36659268647560fa23c47ab27c"
    );
}

#[test]
fn local_daemon_protocol_provider_capability_import_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let request = LocalDaemonRequest::ImportProviderCapabilities(
        crate::local::ImportProviderCapabilitiesRequest {
            workspace_id: Some("/repo".to_string()),
            providers: vec!["codex".to_string(), "claude".to_string()],
            kind: Some("all".to_string()),
            name: Some("docs".to_string()),
            dry_run: true,
        },
    );
    let response = LocalDaemonResponse::ProviderCapabilitiesImported {
        report: crate::local::ProviderCapabilityImportReport {
            dry_run: true,
            providers: vec!["codex".to_string(), "claude".to_string()],
            summary: crate::local::ProviderCapabilityImportSummary {
                candidates: 2,
                imported: 0,
                updated: 0,
                already_installed: 1,
                deduped: 1,
                skipped: 0,
                errors: 0,
            },
            mcps: vec![crate::local::ProviderCapabilityImportEntry {
                kind: "mcp".to_string(),
                name: "docs".to_string(),
                provider: "claude".to_string(),
                source: "/repo/.mcp.json".to_string(),
                hash: Some("hash-1".to_string()),
                action: "would_import".to_string(),
                reason: "would import newest provider definition".to_string(),
                duplicates: vec![crate::local::ProviderCapabilityImportDuplicate {
                    provider: "codex".to_string(),
                    source: "/repo/.codex/config.toml".to_string(),
                    hash: Some("hash-0".to_string()),
                    reason: "different definition hash, older source".to_string(),
                }],
            }],
            skills: vec![crate::local::ProviderCapabilityImportEntry {
                kind: "skill".to_string(),
                name: "qa".to_string(),
                provider: "codex".to_string(),
                source: "/repo/.codex/skills/qa".to_string(),
                hash: Some("hash-2".to_string()),
                action: "already_installed".to_string(),
                reason: "matching skill package already installed in Chariox".to_string(),
                duplicates: Vec::new(),
            }],
        },
    };

    let snapshot = serde_json::json!([request, response]);
    assert_eq!(
        snapshot.pointer("/0/ImportProviderCapabilities/providers/1"),
        Some(&serde_json::json!("claude"))
    );
    assert_eq!(
        snapshot.pointer("/0/ImportProviderCapabilities/dry_run"),
        Some(&serde_json::json!(true))
    );
    assert_eq!(
        snapshot.pointer("/1/ProviderCapabilitiesImported/report/summary/deduped"),
        Some(&serde_json::json!(1))
    );
    assert_eq!(
        snapshot.pointer("/1/ProviderCapabilitiesImported/report/mcps/0/duplicates/0/provider"),
        Some(&serde_json::json!("codex"))
    );
    let serialized = serde_json::to_string(&snapshot)
        .expect("provider capability import snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "431b8a8d30850209aad51fab65d31ffb9e8d9ff6ced4533fbc92f96c12ba6060"
    );
}

#[test]
fn local_daemon_protocol_provider_run_usage_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let mut provider_run = RuntimeProviderRun::from_control_capability_inference(
        "provider-run-1",
        "session-1".to_string(),
        Some("agent-1".to_string()),
        "codex".to_string(),
    );
    provider_run.set_usage(ProviderRunTokenUsage {
        total_tokens: Some(42_100),
        last_tokens: Some(8_900),
        context_tokens: Some(8_900),
        context_window: Some(128_000),
    });

    let response = LocalDaemonResponse::ProviderRun { provider_run };
    let snapshot = serde_json::to_value(response).expect("response should serialize");

    assert_eq!(
        snapshot.pointer("/ProviderRun/provider_run/usage/total_tokens"),
        Some(&serde_json::json!(42_100))
    );
    assert_eq!(
        snapshot.pointer("/ProviderRun/provider_run/usage/last_tokens"),
        Some(&serde_json::json!(8_900))
    );
    assert_eq!(
        snapshot.pointer("/ProviderRun/provider_run/usage/context_tokens"),
        Some(&serde_json::json!(8_900))
    );
    assert_eq!(
        snapshot.pointer("/ProviderRun/provider_run/usage/context_window"),
        Some(&serde_json::json!(128_000))
    );

    let usage_snapshot = snapshot
        .pointer("/ProviderRun/provider_run/usage")
        .expect("usage should serialize");
    let serialized = serde_json::to_string(usage_snapshot).expect("usage snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "bb7a57b01ed4658729be85e00a5e5ae23f877b8a19973ac9f007c01d45ca1335"
    );

    let listing = WorkspaceRepoFileListing {
        workspace_id: "workspace-1".to_string(),
        worktree_id: "worktree-1".to_string(),
        path_prefix: "src".to_string(),
        compare_ref: "origin/main".to_string(),
        total_entries: 2,
        truncated: true,
        entries: vec![WorkspaceRepoFileEntry {
            path: "src/app.rs".to_string(),
            name: "app.rs".to_string(),
            kind: "file".to_string(),
            changed: true,
            status: Some("modified".to_string()),
            additions: 3,
            deletions: 1,
        }],
        generated_at_ms: 1234,
    };
    let listing_snapshot =
        serde_json::to_value(LocalDaemonResponse::WorkspaceFilesListed { listing })
            .expect("workspace listing should serialize");
    let listing_payload = listing_snapshot
        .pointer("/WorkspaceFilesListed/listing")
        .expect("workspace listing payload should serialize");
    assert_eq!(
        listing_payload.pointer("/compare_ref"),
        Some(&serde_json::json!("origin/main"))
    );
    assert_eq!(
        listing_payload.pointer("/total_entries"),
        Some(&serde_json::json!(2))
    );
    assert_eq!(
        listing_payload.pointer("/truncated"),
        Some(&serde_json::json!(true))
    );
    let serialized =
        serde_json::to_string(listing_payload).expect("workspace listing snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "d53bd6870d6a9236c231fcfaafe4c99d893029c6fed44efd31642cdc57adc918"
    );

    let substitute_request =
        LocalDaemonRequest::UpdateAgentSubstitutes(UpdateAgentSubstitutesRequest {
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
            action: AgentSubstituteAction::Add {
                provider: "codex".to_string(),
                model: "gpt-5.4".to_string(),
                variant: Some("medium".to_string()),
                account_profile: Some("work".to_string()),
                kernel_id: Some("kernel-1".to_string()),
                worktree_id: Some("/repo/sub".to_string()),
            },
        });
    let substitute_snapshot =
        serde_json::to_value(substitute_request).expect("substitute request should serialize");
    assert_eq!(
        substitute_snapshot.pointer("/UpdateAgentSubstitutes/action/Add/kernel_id"),
        Some(&serde_json::json!("kernel-1"))
    );
    assert_eq!(
        substitute_snapshot.pointer("/UpdateAgentSubstitutes/action/Add/worktree_id"),
        Some(&serde_json::json!("/repo/sub"))
    );
    assert_eq!(
        substitute_snapshot.pointer("/UpdateAgentSubstitutes/action/Add/account_profile"),
        Some(&serde_json::json!("work"))
    );
    // Null/default semantics: omitting account_profile must not change the wire
    // shape so older clients keep binding to the provider default account.
    let default_account_request =
        LocalDaemonRequest::UpdateAgentSubstitutes(UpdateAgentSubstitutesRequest {
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
            action: AgentSubstituteAction::Add {
                provider: "codex".to_string(),
                model: "gpt-5.4".to_string(),
                variant: None,
                account_profile: None,
                kernel_id: None,
                worktree_id: None,
            },
        });
    let default_account_snapshot = serde_json::to_value(default_account_request)
        .expect("default-account substitute request should serialize");
    assert_eq!(
        default_account_snapshot
            .pointer("/UpdateAgentSubstitutes/action/Add")
            .and_then(|add| add.as_object())
            .map(|add| !add.contains_key("account_profile")),
        Some(true)
    );
    let substitute_add_snapshot = substitute_snapshot
        .pointer("/UpdateAgentSubstitutes/action/Add")
        .expect("substitute add payload should serialize");
    let serialized =
        serde_json::to_string(substitute_add_snapshot).expect("substitute payload should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "c194be05129cb5a452f1e1c7ee49d1a5fe469a6dff62d6b632f03d53163fe2f2"
    );

    let layout_request = serde_json::to_value(LocalDaemonRequest::UpdateWorkflowCanvasLayout(
        super::UpdateWorkflowCanvasLayoutRequest {
            session_id: "session-1".to_string(),
            workflow_ref: "workflow-1".to_string(),
            base_layout_revision: Some(7),
            patches: vec![
                crate::session::WorkflowCanvasLayoutPatch::NodePosition {
                    node_id: "node-1".to_string(),
                    x: 120,
                    y: 80,
                },
                crate::session::WorkflowCanvasLayoutPatch::EndpointPosition {
                    endpoint_id: "endpoint-1".to_string(),
                    x: 140,
                    y: 42,
                },
                crate::session::WorkflowCanvasLayoutPatch::ExitPosition {
                    node_id: "node-1".to_string(),
                    x: 360,
                    y: 90,
                },
                crate::session::WorkflowCanvasLayoutPatch::EdgeWaypoints {
                    edge_id: "edge-1".to_string(),
                    waypoints: vec![crate::session::WorkflowCanvasPoint { x: 220, y: 80 }],
                },
            ],
        },
    ))
    .expect("layout request should serialize");
    let layout_payload = layout_request
        .pointer("/UpdateWorkflowCanvasLayout")
        .expect("layout request payload should serialize");
    assert_eq!(
        layout_payload.pointer("/patches/0/kind"),
        Some(&serde_json::json!("node_position"))
    );
    let serialized =
        serde_json::to_string(layout_payload).expect("layout request snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "2e36f06886355e83a62ab91ba79243e60f4498afd7ace6694260c383daec34ad"
    );

    let design_op_request = serde_json::to_value(LocalDaemonRequest::ApplyWorkflowDesignOp(
        super::ApplyWorkflowDesignOpRequest {
            session_id: "session-1".to_string(),
            origin_client_id: "web-client-1".to_string(),
            op_id: "op-1".to_string(),
            op: super::WorkflowDesignOp::NodeAdd {
                workflow_id: "workflow-1".to_string(),
                node: super::WorkflowDesignNode {
                    id: "node-1".to_string(),
                    agent_id: "agent-1".to_string(),
                    label: None,
                    instructions: Some("Review the change".to_string()),
                    can_complete_workflow_run: None,
                    can_emit_intermediate_run_output: None,
                    wait_for_all_inputs: None,
                    intermediate_output_schema_ref: None,
                    max_turns: Some(3),
                },
                position: Some(super::WorkflowDesignPoint { x: 120, y: 80 }),
            },
        },
    ))
    .expect("design op request should serialize");
    let design_op_payload = design_op_request
        .pointer("/ApplyWorkflowDesignOp")
        .expect("design op payload should serialize");
    assert_eq!(
        design_op_payload.pointer("/op/workflow_id"),
        Some(&serde_json::json!("workflow-1"))
    );
    assert_eq!(
        design_op_payload.pointer("/op/position/x"),
        Some(&serde_json::json!(120))
    );
    let serialized =
        serde_json::to_string(design_op_payload).expect("design op snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "6beed0034e0a7717008a1e7269e1d01f096ce14af5a65c94efe7d111cdc52e94"
    );

    let workflow_prompt_design_op_request = serde_json::to_value(
        LocalDaemonRequest::ApplyWorkflowDesignOp(super::ApplyWorkflowDesignOpRequest {
            session_id: "session-1".to_string(),
            origin_client_id: "web-client-1".to_string(),
            op_id: "op-3".to_string(),
            op: super::WorkflowDesignOp::WorkflowUpdate {
                workflow_id: "workflow-1".to_string(),
                patch: crate::local::WorkflowDesignWorkflowPatch {
                    alias: None,
                    prompt: Some(Some("Shared workflow context".to_string())),
                    flush_agent_context_before_run: None,
                    max_concurrent: None,
                    run_output_schema_ref: None,
                },
            },
        }),
    )
    .expect("workflow prompt design op request should serialize");
    let workflow_prompt_design_op_payload = workflow_prompt_design_op_request
        .pointer("/ApplyWorkflowDesignOp")
        .expect("workflow prompt design op payload should serialize");
    assert_eq!(
        workflow_prompt_design_op_payload.pointer("/op/patch/prompt"),
        Some(&serde_json::json!("Shared workflow context"))
    );
    let serialized = serde_json::to_string(workflow_prompt_design_op_payload)
        .expect("workflow prompt design op snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "0ed8482687fbefc4645e72d6aad988bccd906c94b7e5ada2a745dbb2380a028f"
    );

    let schema_design_op_request = serde_json::to_value(LocalDaemonRequest::ApplyWorkflowDesignOp(
        super::ApplyWorkflowDesignOpRequest {
            session_id: "session-1".to_string(),
            origin_client_id: "web-client-1".to_string(),
            op_id: "op-2".to_string(),
            op: super::WorkflowDesignOp::SchemaAdd {
                workflow_id: "workflow-1".to_string(),
                schema: crate::session::WorkflowSchemaDefinition::new(
                    "schema-1",
                    Some("Review payload".to_string()),
                    Some("Structured review handoff".to_string()),
                    serde_json::json!({
                        "type": "object",
                        "required": ["summary"],
                        "properties": {
                            "summary": { "type": "string" }
                        }
                    }),
                ),
            },
        },
    ))
    .expect("schema design op request should serialize");
    let schema_design_op_payload = schema_design_op_request
        .pointer("/ApplyWorkflowDesignOp")
        .expect("schema design op payload should serialize");
    assert_eq!(
        schema_design_op_payload.pointer("/op/kind"),
        Some(&serde_json::json!("schema_add"))
    );
    assert_eq!(
        schema_design_op_payload.pointer("/op/schema/id"),
        Some(&serde_json::json!("schema-1"))
    );
    let serialized = serde_json::to_string(schema_design_op_payload)
        .expect("schema design op snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "72370a11b682cd890bd22f88a536f0394bfc1188dfa124ca85432de5daa6e0c5"
    );

    let wait_for_all_inputs_request =
        serde_json::to_value(LocalDaemonRequest::SetWorkflowNodeWaitForAllInputs(
            super::SetWorkflowNodeWaitForAllInputsRequest {
                session_id: "session-1".to_string(),
                workflow_ref: "workflow-1".to_string(),
                node_id: "node-1".to_string(),
                wait_for_all_inputs: true,
                expected_workflow_revision: Some(7),
            },
        ))
        .expect("wait-for-all-inputs request should serialize");
    let wait_for_all_inputs_payload = wait_for_all_inputs_request
        .pointer("/SetWorkflowNodeWaitForAllInputs")
        .expect("wait-for-all-inputs payload should serialize");
    assert_eq!(
        wait_for_all_inputs_payload.pointer("/wait_for_all_inputs"),
        Some(&serde_json::json!(true))
    );
    let serialized = serde_json::to_string(wait_for_all_inputs_payload)
        .expect("wait-for-all-inputs request snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "f264411172709de929512f443cbafd5341808a4fc761b8541ae35557ab334ff7"
    );

    let edge_request = serde_json::to_value(LocalDaemonRequest::AddWorkflowEdge(
        super::AddWorkflowEdgeRequest {
            session_id: "session-1".to_string(),
            workflow_ref: "workflow-1".to_string(),
            from_node_id: "node-1".to_string(),
            to_node_id: "node-2".to_string(),
            handoff_schema_ref: None,
            validation_policy: None,
            source_side: Some(crate::session::WorkflowEdgeEndpointSide::Right),
            target_side: Some(crate::session::WorkflowEdgeEndpointSide::Left),
            expected_workflow_revision: Some(7),
        },
    ))
    .expect("edge request should serialize");
    let edge_payload = edge_request
        .pointer("/AddWorkflowEdge")
        .expect("edge request payload should serialize");
    assert_eq!(
        edge_payload.pointer("/source_side"),
        Some(&serde_json::json!("right"))
    );
    assert_eq!(
        edge_payload.pointer("/target_side"),
        Some(&serde_json::json!("left"))
    );
    let serialized =
        serde_json::to_string(edge_payload).expect("edge request snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "10955426ce27ba4a006a9c3ec20ebb73964eaf275f93c490e79fd109fdb6b123"
    );

    let set_secret_request = serde_json::to_value(LocalDaemonRequest::SetCredentialSecret(
        crate::local::SetCredentialSecretRequest {
            session_id: Some("session-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            key: "gmail-password".to_string(),
            value: "secret-value".to_string(),
        },
    ))
    .expect("set credential secret request should serialize");
    let set_secret_payload = set_secret_request
        .pointer("/SetCredentialSecret")
        .expect("set credential secret payload should serialize");
    assert_eq!(
        set_secret_payload.pointer("/session_id"),
        Some(&serde_json::json!("session-1"))
    );
    assert_eq!(
        set_secret_payload.pointer("/agent_id"),
        Some(&serde_json::json!("agent-1"))
    );
    let manage_vault_request = serde_json::to_value(LocalDaemonRequest::ManageCredentialVault(
        crate::local::ManageCredentialVaultRequest {
            session_id: "session-1".to_string(),
            agent_id: Some("agent-1".to_string()),
        },
    ))
    .expect("manage credential vault request should serialize");
    let manage_vault_payload = manage_vault_request
        .pointer("/ManageCredentialVault")
        .expect("manage credential vault payload should serialize");
    assert_eq!(
        manage_vault_payload.pointer("/session_id"),
        Some(&serde_json::json!("session-1"))
    );
    let serialized = serde_json::to_string(&serde_json::json!({
        "set_secret": set_secret_payload,
        "manage_vault": manage_vault_payload,
    }))
    .expect("credential vault request snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "8780d0be92d8154fd1ed4d5edc1831063bedf76e291e9f7519ae624198782ed6"
    );

    let custom_interaction = crate::session::RuntimeInteraction::new(
        "interaction-1",
        "agent-1",
        crate::session::RuntimeInteractionKind::Choice,
        crate::session::RuntimeInteractionLevel::Info,
        Some("Pick a color".to_string()),
        "Choose a color or type another one.",
        vec![
            crate::session::RuntimeInteractionChoice::new("green", "Green", "Green", None),
            crate::session::RuntimeInteractionChoice::new("red", "Red", "Red", None),
        ],
        Some(crate::session::RuntimeInteractionCustomChoice::new(
            "custom",
            "Other",
            Some("Type a color".to_string()),
            Some(1),
            Some(120),
        )),
        None,
        None,
    );
    let custom_interaction_snapshot =
        serde_json::to_value(custom_interaction).expect("custom interaction should serialize");
    assert_eq!(
        custom_interaction_snapshot.pointer("/custom_choice/id"),
        Some(&serde_json::json!("custom"))
    );
    assert_eq!(
        custom_interaction_snapshot.pointer("/custom_choice/placeholder"),
        Some(&serde_json::json!("Type a color"))
    );
    let custom_response_request = serde_json::to_value(LocalDaemonRequest::RespondToInteraction(
        super::RespondToInteractionRequest {
            session_id: "session-1".to_string(),
            interaction_id: "interaction-1".to_string(),
            choice_id: "custom".to_string(),
            custom_reply: Some("Blue".to_string()),
        },
    ))
    .expect("custom interaction response should serialize");
    let custom_response_payload = custom_response_request
        .pointer("/RespondToInteraction")
        .expect("custom interaction response payload should serialize");
    assert_eq!(
        custom_response_payload.pointer("/custom_reply"),
        Some(&serde_json::json!("Blue"))
    );
    let serialized = serde_json::to_string(&serde_json::json!({
        "custom_choice": custom_interaction_snapshot
            .pointer("/custom_choice")
            .expect("custom choice should serialize"),
        "response": custom_response_payload,
    }))
    .expect("custom interaction snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "f1ded2949999d324de8a29805cbe0f0841625106e63ab556c3c6076fcf3f640d"
    );

    let secret_interaction = crate::session::RuntimeInteraction::new(
        "credential-secret-1",
        "agent-1",
        crate::session::RuntimeInteractionKind::Choice,
        crate::session::RuntimeInteractionLevel::Critical,
        Some("Add secret".to_string()),
        "Enter a password to store in Chariox Vault.",
        vec![crate::session::RuntimeInteractionChoice::new(
            "cancel",
            "Cancel",
            "cancel",
            Some(crate::session::RuntimeInteractionChoiceStyle::Danger),
        )],
        Some(crate::session::RuntimeInteractionCustomChoice::secret(
            "secret",
            "Secret",
            Some("Password".to_string()),
            Some(8),
            Some(256),
        )),
        Some(300),
        Some("cancel".to_string()),
    );
    let mut secret_interaction_snapshot =
        serde_json::to_value(secret_interaction).expect("secret interaction should serialize");
    secret_interaction_snapshot["requested_at_ms"] = serde_json::json!(1234);
    assert_eq!(
        secret_interaction_snapshot.pointer("/custom_choice/input_kind"),
        Some(&serde_json::json!("secret"))
    );
    let serialized =
        serde_json::to_string(&secret_interaction_snapshot).expect("secret snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "75d5e3e796965d9d5fbf104bfd875c37d9ba01016f3147c4a79740f5ee30ff43"
    );

    let content = WorkspaceFileContent {
        workspace_id: "workspace-1".to_string(),
        worktree_id: "worktree-1".to_string(),
        path: "src/app.rs".to_string(),
        name: "app.rs".to_string(),
        language: "rust".to_string(),
        mime: "text/x-rust".to_string(),
        encoding: "utf-8".to_string(),
        content_text: Some("fn main() {}\n".to_string()),
        content_base64: None,
        size_bytes: 13,
        mtime_ms: 1235,
        fingerprint: "fingerprint-1".to_string(),
        sha256: Some("sha256-1".to_string()),
        truncated: false,
        status: Some("modified".to_string()),
        additions: 3,
        deletions: 1,
        compare_ref: "origin/main".to_string(),
        generated_at_ms: 1236,
    };
    let content_snapshot =
        serde_json::to_value(LocalDaemonResponse::WorkspaceFileContent { content })
            .expect("workspace file content should serialize");
    let content_payload = content_snapshot
        .pointer("/WorkspaceFileContent/content")
        .expect("workspace file content payload should serialize");
    assert_eq!(
        content_payload.pointer("/language"),
        Some(&serde_json::json!("rust"))
    );
    assert_eq!(
        content_payload.pointer("/encoding"),
        Some(&serde_json::json!("utf-8"))
    );
    assert_eq!(
        content_payload.pointer("/content_text"),
        Some(&serde_json::json!("fn main() {}\n"))
    );
    let serialized = serde_json::to_string(content_payload)
        .expect("workspace file content snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "a2bff9ada5aa65ea753652ae69c8b574759bfcd50962dd07015c5958908dfdd4"
    );

    let delete_worktree_request = serde_json::to_value(
        LocalDaemonRequest::DeleteWorkspaceWorktree(DeleteWorkspaceWorktreeRequest {
            workspace_id: "workspace-1".to_string(),
            worktree_id: "worktree-1".to_string(),
            force: true,
        }),
    )
    .expect("delete worktree request should serialize");
    let delete_worktree_payload = delete_worktree_request
        .pointer("/DeleteWorkspaceWorktree")
        .expect("delete worktree payload should serialize");
    assert_eq!(
        delete_worktree_payload.pointer("/worktree_id"),
        Some(&serde_json::json!("worktree-1"))
    );
    let serialized = serde_json::to_string(delete_worktree_payload)
        .expect("delete worktree request snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "d3df9ce72d0e27572f5ce66e5e29ba809d6e00d73c5508deda2aa810969f40e8"
    );

    let create_pr_request = serde_json::to_value(LocalDaemonRequest::CreateWorkspacePullRequest(
        CreateWorkspacePullRequestRequest {
            workspace_id: "workspace-1".to_string(),
            worktree_id: "worktree-1".to_string(),
            title: Some("Ship feature".to_string()),
            body: Some("Body".to_string()),
            base_ref: Some("main".to_string()),
            draft: true,
        },
    ))
    .expect("create pull request request should serialize");
    let create_pr_payload = create_pr_request
        .pointer("/CreateWorkspacePullRequest")
        .expect("create pull request payload should serialize");
    assert_eq!(
        create_pr_payload.pointer("/base_ref"),
        Some(&serde_json::json!("main"))
    );
    let serialized = serde_json::to_string(create_pr_payload)
        .expect("create pull request request snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "318e776a7f78bd7b0a8543028eea8d1acd44865c3f49a7bd22a7596c77b22471"
    );

    let pull_request = WorkspacePullRequestRecord {
        workspace_id: "workspace-1".to_string(),
        worktree_id: "worktree-1".to_string(),
        branch: "feature".to_string(),
        base_ref: "main".to_string(),
        url: "https://github.com/example/repo/pull/1".to_string(),
        title: Some("Ship feature".to_string()),
        draft: true,
        generated_at_ms: 1237,
    };
    let pr_response =
        serde_json::to_value(LocalDaemonResponse::WorkspacePullRequestCreated { pull_request })
            .expect("pull request response should serialize");
    let pr_response_payload = pr_response
        .pointer("/WorkspacePullRequestCreated/pull_request")
        .expect("pull request response payload should serialize");
    assert_eq!(
        pr_response_payload.pointer("/url"),
        Some(&serde_json::json!("https://github.com/example/repo/pull/1"))
    );
    let serialized = serde_json::to_string(pr_response_payload)
        .expect("pull request response snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "4354e2b9a67c08033d5306739f223f09af4ca44c747a01c30499526766bca00f"
    );
}

#[test]
fn local_daemon_protocol_active_turn_phase_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let active_turn = crate::runtime::projection::AgentActiveTurnProjection {
        prompt_id: "external:codex:thread-1:prompt-1".to_string(),
        provider_run_id: Some("provider-run-1".to_string()),
        source_attachment_id: Some("attachment-1".to_string()),
        prompt_origin: Some(crate::session::PromptOrigin::External),
        external_provider: Some("codex".to_string()),
        external_provider_session_id: Some("thread-1".to_string()),
        external_provider_turn_id: Some("prompt-1".to_string()),
        status: crate::runtime::projection::AgentPromptRuntimeStatus::Running,
        phase: crate::runtime::projection::AgentTurnRuntimePhase::AwaitingFirstOutput,
        started_at_ms: Some(1234),
    };

    let snapshot = serde_json::to_value(active_turn).expect("active turn should serialize");
    assert_eq!(
        snapshot.pointer("/phase"),
        Some(&serde_json::json!("awaiting_first_output"))
    );
    assert_eq!(
        snapshot.pointer("/started_at_ms"),
        Some(&serde_json::json!(1234))
    );
    assert_eq!(
        snapshot.pointer("/source_attachment_id"),
        Some(&serde_json::json!("attachment-1"))
    );
    assert_eq!(
        snapshot.pointer("/prompt_origin"),
        Some(&serde_json::json!("external"))
    );
    assert_eq!(
        snapshot.pointer("/external_provider"),
        Some(&serde_json::json!("codex"))
    );
    assert_eq!(
        snapshot.pointer("/external_provider_session_id"),
        Some(&serde_json::json!("thread-1"))
    );
    assert_eq!(
        snapshot.pointer("/external_provider_turn_id"),
        Some(&serde_json::json!("prompt-1"))
    );

    let serialized = serde_json::to_string(&snapshot).expect("active turn snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "68a42b6406519439687027cd8d14be740f174979dc4cb76d058ed055962df074"
    );
}

#[test]
fn local_daemon_protocol_queued_prompt_control_projection_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let control = crate::runtime::projection::AgentQueuedPromptControlProjection {
        prompt_id: "prompt-queued".to_string(),
        status: "queued".to_string(),
        can_steer: false,
        can_cancel: true,
        steer_disabled_reason: Some(
            "Steering is unavailable while the active provider turn was started outside Chariox."
                .to_string(),
        ),
        cancel_disabled_reason: None,
    };

    let snapshot = serde_json::to_value(control).expect("queued prompt control should serialize");
    assert_eq!(
        snapshot.pointer("/prompt_id"),
        Some(&serde_json::json!("prompt-queued"))
    );
    assert_eq!(
        snapshot.pointer("/status"),
        Some(&serde_json::json!("queued"))
    );
    assert_eq!(
        snapshot.pointer("/can_steer"),
        Some(&serde_json::json!(false))
    );
    assert_eq!(
        snapshot.pointer("/can_cancel"),
        Some(&serde_json::json!(true))
    );
    assert_eq!(
        snapshot.pointer("/steer_disabled_reason"),
        Some(&serde_json::json!(
            "Steering is unavailable while the active provider turn was started outside Chariox."
        ))
    );
    assert_eq!(snapshot.pointer("/cancel_disabled_reason"), None);

    let serialized =
        serde_json::to_string(&snapshot).expect("queued prompt control snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "6ef6a1741e0f45d8a34f5ef8207c4fd7f2d663ed6b7da51bcc4e161e45a2f022"
    );
}

#[test]
fn local_daemon_protocol_completed_turn_action_projection_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let completed = crate::git_observer::CompletedGitTurnActionProjection {
        turn_id: "turn-1".to_string(),
        prompt_id: "external:codex:thread-1:user-1".to_string(),
        provider_run_id: "provider-run-1".to_string(),
        agent_id: "agent-1".to_string(),
        source_attachment_id: Some("attachment-1".to_string()),
        prompt_origin: Some(crate::session::PromptOrigin::External),
        external_provider: Some("codex".to_string()),
        external_provider_session_id: Some("thread-1".to_string()),
        external_provider_turn_id: Some("user-1".to_string()),
        completed_at_ms: 1_234,
        settlement_status: crate::git_observer::CompletedTurnSettlementStatus::Cancelled,
        duration_ms: Some(234),
        changed_paths: vec!["src/lib.rs".to_string()],
        undo_available: true,
        undo_unavailable_reason: None,
    };

    let snapshot = serde_json::to_value(completed).expect("completed turn action should serialize");
    assert_eq!(
        snapshot.pointer("/settlement_status"),
        Some(&serde_json::json!("cancelled"))
    );
    assert_eq!(
        snapshot.pointer("/source_attachment_id"),
        Some(&serde_json::json!("attachment-1"))
    );
    assert_eq!(
        snapshot.pointer("/prompt_origin"),
        Some(&serde_json::json!("external"))
    );
    assert_eq!(
        snapshot.pointer("/external_provider"),
        Some(&serde_json::json!("codex"))
    );
    assert_eq!(
        snapshot.pointer("/external_provider_session_id"),
        Some(&serde_json::json!("thread-1"))
    );
    assert_eq!(
        snapshot.pointer("/external_provider_turn_id"),
        Some(&serde_json::json!("user-1"))
    );

    let serialized =
        serde_json::to_string(&snapshot).expect("completed turn action snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "fe64e3f92543f035bbb5fcf1c3604fbbc28357bb1a72c31916488dde8e555828"
    );
}

#[test]
fn local_daemon_protocol_agent_runtime_activity_counts_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let activity = crate::runtime::projection::AgentRuntimeActivity {
        status: crate::runtime::projection::AgentRuntimeStatus::Working,
        prompt_status: crate::runtime::projection::AgentPromptRuntimeStatus::Running,
        busy: true,
        active_prompt_count: 1,
        queued_prompt_count: 2,
        unread_idle_output: false,
        queued_prompt_controls: std::collections::BTreeMap::new(),
        active_turn: None,
        last_completed_turn: None,
    };

    let snapshot = serde_json::to_value(activity).expect("agent runtime activity should serialize");
    assert_eq!(
        snapshot.pointer("/active_prompt_count"),
        Some(&serde_json::json!(1))
    );
    assert_eq!(
        snapshot.pointer("/queued_prompt_count"),
        Some(&serde_json::json!(2))
    );
    assert_eq!(snapshot.pointer("/queued_prompt_controls"), None);

    let serialized =
        serde_json::to_string(&snapshot).expect("agent runtime activity snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "606757082a57ec9fd0435bcf4a64aa62f4410316ac22193566288ae8e474effa"
    );
}
