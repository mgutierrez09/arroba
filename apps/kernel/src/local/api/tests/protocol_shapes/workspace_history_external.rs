use super::*;

#[test]
fn local_daemon_protocol_debug_bundle_export_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let request = LocalDaemonRequest::ExportDebugBundle(ExportDebugBundleRequest {
        session_id: "session-1".to_string(),
        bundle_label: Some("support".to_string()),
        limit: Some(500),
    });
    let response = LocalDaemonResponse::DebugBundleExported {
        bundle_dir: "/state/chariox/debug-bundles/session-1-support".to_string(),
        manifest_path: "/state/chariox/debug-bundles/session-1-support/manifest.json".to_string(),
        logs_path: "/state/chariox/debug-bundles/session-1-support/logs.ndjson".to_string(),
        log_root: "/state/chariox/logs".to_string(),
        record_count: 12,
        limit: 500,
    };
    let snapshot = serde_json::json!([request, response]);

    assert_eq!(
        snapshot.pointer("/0/ExportDebugBundle/session_id"),
        Some(&serde_json::json!("session-1"))
    );
    assert_eq!(
        snapshot.pointer("/0/ExportDebugBundle/bundle_label"),
        Some(&serde_json::json!("support"))
    );
    assert_eq!(
        snapshot.pointer("/1/DebugBundleExported/record_count"),
        Some(&serde_json::json!(12))
    );
    let serialized = serde_json::to_string(&snapshot).expect("debug-bundle snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "84d81368a5adf1e65c8753f837da5eda87a9fff0f3675fdd7d9790078fe13fe8"
    );
}

#[test]
fn local_daemon_protocol_workspace_live_sync_status_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let request = LocalDaemonRequest::GetWorkspaceLiveSyncStatus(
        crate::local::GetWorkspaceLiveSyncStatusRequest {
            session_id: "session-1".to_string(),
        },
    );
    let mode_request = LocalDaemonRequest::SetWorkspaceLiveSyncMode(
        crate::local::SetWorkspaceLiveSyncModeRequest {
            session_id: "session-1".to_string(),
            mode: crate::config::WorkspaceLiveSyncMode::Tracked,
        },
    );
    let response = LocalDaemonResponse::WorkspaceLiveSyncStatus {
        status: crate::local::WorkspaceLiveSyncStatus {
            session_id: "session-1".to_string(),
            mode: crate::config::WorkspaceLiveSyncMode::Tracked,
            footer_state: crate::local::WorkspaceLiveSyncFooterState::Tracked,
            sync_groups: vec![crate::local::WorkspaceLiveSyncGroupStatus {
                group_id: "workspace-link-1".to_string(),
                group_name: "shared".to_string(),
                target_count: 1,
                ready_targets: 1,
                degraded_targets: 0,
                conflicted_targets: 0,
            }],
            targets: vec![crate::local::WorkspaceLiveSyncTargetStatus {
                link_id: "workspace-link-1".to_string(),
                link_name: "shared".to_string(),
                user_id: "user-1".to_string(),
                machine_id: "machine-1".to_string(),
                kernel_id: "kernel-1".to_string(),
                repo_root: "/repo".to_string(),
                branch: Some("main".to_string()),
                repo_fingerprint: Some("fingerprint-1".to_string()),
                status: crate::local::WorkspaceLiveSyncTargetState::Ready,
                attached_at_ms: 42,
            }],
            conflicts: vec![crate::local::WorkspaceLiveSyncConflictSummary {
                conflict_id: "conflict-1".to_string(),
                link_id: "workspace-link-1".to_string(),
                source_agent_id: "agent-1".to_string(),
                target_user_id: "user-2".to_string(),
                target_repo_root: "/repo-2".to_string(),
                path: "src/lib.rs".to_string(),
                next_action: "Assign a resolver agent.".to_string(),
            }],
            ignore: crate::local::WorkspaceLiveSyncIgnoreStatus {
                ignore_file: Some(".charioxignore".to_string()),
                rules: vec!["ignored/**".to_string(), "*.secret".to_string()],
                force_excludes: vec![".git/**".to_string(), ".chariox/**".to_string()],
            },
        },
    };
    let mut mode_session = crate::session::RuntimeSession::new(
        "session-1",
        None,
        "/repo",
        "/repo",
        "machine-1",
        "daemon-1",
    );
    mode_session.set_workspace_live_sync_mode(Some(crate::config::WorkspaceLiveSyncMode::Tracked));
    let mode_response = LocalDaemonResponse::WorkspaceLiveSyncModeUpdated {
        session: mode_session,
        effects: vec![crate::local::UserConfigMutationEffect {
            kind: "provider_reload".to_string(),
            path: "session.workspace_live_sync_mode".to_string(),
            message:
                "session workspace live sync mode updated; provider reloads: 1 reloaded, 1 deferred, 0 unaffected"
                    .to_string(),
            provider_reload: Some(crate::local::UserConfigProviderReloadSummary {
                reloaded: 1,
                deferred: 1,
                unaffected: 0,
            }),
        }],
    };

    let mut snapshot = serde_json::json!([request, mode_request, response, mode_response]);
    *snapshot
        .pointer_mut("/3/WorkspaceLiveSyncModeUpdated/session/created_at_ms")
        .expect("session created_at_ms should encode") = serde_json::json!(42);
    *snapshot
        .pointer_mut("/3/WorkspaceLiveSyncModeUpdated/session/last_used_at_ms")
        .expect("session last_used_at_ms should encode") = serde_json::json!(42);
    assert_eq!(
        snapshot.pointer("/0/GetWorkspaceLiveSyncStatus/session_id"),
        Some(&serde_json::json!("session-1"))
    );
    assert_eq!(
        snapshot.pointer("/1/SetWorkspaceLiveSyncMode/session_id"),
        Some(&serde_json::json!("session-1"))
    );
    assert_eq!(
        snapshot.pointer("/1/SetWorkspaceLiveSyncMode/mode"),
        Some(&serde_json::json!("tracked"))
    );
    assert_eq!(
        snapshot.pointer("/2/WorkspaceLiveSyncStatus/status/footer_state"),
        Some(&serde_json::json!("tracked"))
    );
    assert_eq!(
        snapshot.pointer("/2/WorkspaceLiveSyncStatus/status/sync_groups/0/group_id"),
        Some(&serde_json::json!("workspace-link-1"))
    );
    assert_eq!(
        snapshot.pointer("/2/WorkspaceLiveSyncStatus/status/targets/0/status"),
        Some(&serde_json::json!("ready"))
    );
    assert_eq!(
        snapshot.pointer("/3/WorkspaceLiveSyncModeUpdated/session/id"),
        Some(&serde_json::json!("session-1"))
    );
    assert_eq!(
        snapshot.pointer("/3/WorkspaceLiveSyncModeUpdated/session/workspace_live_sync_mode"),
        Some(&serde_json::json!("tracked"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("workspace live sync status should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "9532c94456dbfaec1a8ea4ec4c1678d9d84f5187c0fcf845b0a10921c7dd3342"
    );
}

#[test]
fn local_daemon_protocol_session_history_outline_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let request = LocalDaemonRequest::GetSessionHistoryOutline(
        crate::local::GetSessionHistoryOutlineRequest {
            session_id: "session-1".to_string(),
            agent_ids: Some(vec!["agent-1".to_string(), "agent-2".to_string()]),
            latest_prompt_count: Some(4),
            cursor: Some(crate::local::SessionHistoryOutlineCursor {
                before_sequence: 10,
            }),
        },
    );
    let blob_request = LocalDaemonRequest::GetSessionHistoryBlobContent(
        crate::local::GetSessionHistoryBlobContentRequest {
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
            blob_id: "history:11:11".to_string(),
        },
    );
    let mut user_prompt = history_page_entry(
        10,
        crate::history::SessionHistoryEntryKind::UserPrompt,
        "agent-1",
        "prompt",
    );
    user_prompt.entry.attachments = vec![crate::history::SessionHistoryPromptAttachment {
        url: "chariox-terminal://prompt-attachment/attachment-1/Screenshot.png".to_string(),
        mime: "image/png".to_string(),
        filename: Some("Screenshot.png".to_string()),
        preview_url: Some("data:image/png;base64,aW1hZ2U=".to_string()),
    }];
    user_prompt.entry.prompt_origin = Some(crate::session::PromptOrigin::External);
    let summary = history_page_entry(
        12,
        crate::history::SessionHistoryEntryKind::ProviderOutput,
        "agent-1",
        "summary",
    );
    let mut assistant_entry = history_page_entry(
        13,
        crate::history::SessionHistoryEntryKind::ProviderOutput,
        "agent-1",
        "assistant detail",
    );
    assistant_entry.entry.source =
        Some(crate::history::SessionHistoryEntrySource::ExternalProviderObserved);
    assistant_entry.entry.external_provider = Some("codex".to_string());
    assistant_entry.entry.external_provider_session_id = Some("thread-1".to_string());
    assistant_entry.entry.external_provider_turn_id = Some("turn-1".to_string());
    assistant_entry.entry.observed_at_ms = Some(42);
    assistant_entry.entry.external_observation =
        Some(crate::history::SessionHistoryExternalObservation {
            settles_active_prompt: true,
            passive_telemetry: false,
        });
    assistant_entry.entry.prompt_origin = Some(crate::session::PromptOrigin::External);
    let mut blob_entry = history_page_entry(
        11,
        crate::history::SessionHistoryEntryKind::ProviderTool,
        "agent-1",
        "{\"tool\":\"bash\",\"status\":\"completed\"}",
    );
    blob_entry.entry.prompt_origin = Some(crate::session::PromptOrigin::External);
    let outline = LocalDaemonResponse::SessionHistoryOutline {
        agents: vec![crate::local::SessionHistoryOutlineAgent {
            agent_id: "agent-1".to_string(),
            turns: vec![crate::local::SessionHistoryOutlineTurn {
                turn_id: "turn-1".to_string(),
                prompt_id: Some("prompt-1".to_string()),
                prompt_origin: crate::session::PromptOrigin::External,
                external_provider: Some("codex".to_string()),
                external_provider_session_id: Some("thread-1".to_string()),
                external_provider_turn_id: Some("turn-1".to_string()),
                started_at_ms: 42,
                lifecycle: crate::local::SessionHistoryOutlineTurnLifecycle::Open,
                completed_at_ms: None,
                user_prompt,
                entries: vec![assistant_entry],
                summary: Some(summary),
                blobs: vec![crate::local::SessionHistoryOutlineBlob {
                    blob_id: "history:11:11".to_string(),
                    kind: crate::history::SessionHistoryEntryKind::ProviderTool,
                    title: "bash · COMPLETED".to_string(),
                    summary: "$ cargo test".to_string(),
                    sequence_start: 11,
                    sequence_end: 11,
                    entry_count: 1,
                    total_chars: 38,
                    timestamp_ms: 43,
                }],
            }],
            next_cursor: Some(crate::local::SessionHistoryOutlineCursor {
                before_sequence: 10,
            }),
        }],
    };
    let blob_response = LocalDaemonResponse::SessionHistoryBlobContent {
        blob_id: "history:11:11".to_string(),
        entries: vec![blob_entry],
    };

    let snapshot = serde_json::json!([request, blob_request, outline, blob_response]);
    assert_eq!(
        snapshot.pointer("/0/GetSessionHistoryOutline/latest_prompt_count"),
        Some(&serde_json::json!(4))
    );
    assert_eq!(
        snapshot.pointer("/0/GetSessionHistoryOutline/cursor/before_sequence"),
        Some(&serde_json::json!(10))
    );
    assert_eq!(
        snapshot.pointer("/2/SessionHistoryOutline/agents/0/turns/0/entries/0/entry/text"),
        Some(&serde_json::json!("assistant detail"))
    );
    assert_eq!(
        snapshot.pointer("/2/SessionHistoryOutline/agents/0/turns/0/prompt_origin"),
        Some(&serde_json::json!("external"))
    );
    assert_eq!(
        snapshot
            .pointer("/2/SessionHistoryOutline/agents/0/turns/0/user_prompt/entry/prompt_origin"),
        Some(&serde_json::json!("external"))
    );
    assert_eq!(
        snapshot.pointer("/2/SessionHistoryOutline/agents/0/turns/0/external_provider"),
        Some(&serde_json::json!("codex"))
    );
    assert_eq!(
        snapshot.pointer("/2/SessionHistoryOutline/agents/0/turns/0/external_provider_session_id"),
        Some(&serde_json::json!("thread-1"))
    );
    assert_eq!(
        snapshot.pointer("/2/SessionHistoryOutline/agents/0/turns/0/external_provider_turn_id"),
        Some(&serde_json::json!("turn-1"))
    );
    assert_eq!(
        snapshot.pointer("/2/SessionHistoryOutline/agents/0/turns/0/completed_at_ms"),
        Some(&serde_json::Value::Null)
    );
    assert_eq!(
        snapshot.pointer("/2/SessionHistoryOutline/agents/0/turns/0/lifecycle"),
        Some(&serde_json::json!("open"))
    );
    assert_eq!(
        snapshot.pointer(
            "/2/SessionHistoryOutline/agents/0/turns/0/user_prompt/entry/attachments/0/filename"
        ),
        Some(&serde_json::json!("Screenshot.png"))
    );
    assert_eq!(
        snapshot.pointer(
            "/2/SessionHistoryOutline/agents/0/turns/0/user_prompt/entry/attachments/0/preview_url"
        ),
        Some(&serde_json::json!("data:image/png;base64,aW1hZ2U="))
    );
    assert_eq!(
        snapshot.pointer("/2/SessionHistoryOutline/agents/0/turns/0/entries/0/entry/source"),
        Some(&serde_json::json!("external_provider_observed"))
    );
    assert_eq!(
        snapshot.pointer("/2/SessionHistoryOutline/agents/0/turns/0/entries/0/entry/prompt_origin"),
        Some(&serde_json::json!("external"))
    );
    assert_eq!(
        snapshot.pointer(
            "/2/SessionHistoryOutline/agents/0/turns/0/entries/0/entry/external_observation/settles_active_prompt"
        ),
        Some(&serde_json::json!(true))
    );
    assert_eq!(
        snapshot.pointer("/2/SessionHistoryOutline/agents/0/turns/0/blobs/0/blob_id"),
        Some(&serde_json::json!("history:11:11"))
    );
    assert_eq!(
        snapshot.pointer("/3/SessionHistoryBlobContent/entries/0/entry/kind"),
        Some(&serde_json::json!("provider_tool"))
    );
    assert_eq!(
        snapshot.pointer("/3/SessionHistoryBlobContent/entries/0/entry/prompt_origin"),
        Some(&serde_json::json!("external"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("session history outline shape should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "94157ad720fb4c9c3f38baa8a2e19546f112a8605152dd6a5d5f9b71be193afd"
    );
}

#[test]
fn local_daemon_protocol_provider_process_memory_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let request =
        LocalDaemonRequest::ListProviderProcesses(crate::local::ListProviderProcessesRequest {
            provider: Some("codex".to_string()),
        });
    let response = LocalDaemonResponse::ProviderProcessesListed {
        processes: vec![crate::provider::ProviderProcessInfo {
            process_id: "managed:codex:process-1".to_string(),
            provider: "codex".to_string(),
            process_label: "codex:default".to_string(),
            pid: Some(4321),
            resident_set_bytes: Some(134_217_728),
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            status: crate::provider::ProviderProcessStatus::Active,
            started_at_ms: 1,
            last_activity_at_ms: 2,
            provider_session_ids: vec!["thread-1".to_string()],
            owner_session_ids: vec!["session-1".to_string()],
            owner_provider_run_ids: vec!["provider-run-1".to_string()],
            attached_session_ids: vec!["session-1".to_string()],
            active_workflow_run_ids: Vec::new(),
            teardown_safe: false,
            teardown_blockers: vec!["attached sessions: session-1".to_string()],
        }],
    };

    let snapshot = serde_json::json!([request, response]);
    assert_eq!(
        snapshot.pointer("/0/ListProviderProcesses/provider"),
        Some(&serde_json::json!("codex"))
    );
    assert_eq!(
        snapshot.pointer("/1/ProviderProcessesListed/processes/0/resident_set_bytes"),
        Some(&serde_json::json!(134_217_728))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("provider process memory shape should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "300ae3afacdf79ffa6507654b76db48818a563cff5cab6e715d8f8b887f6509a"
    );
}

#[test]
fn local_daemon_protocol_external_provider_session_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let request = LocalDaemonRequest::ListExternalProviderSessions(
        crate::local::ListExternalProviderSessionsRequest {
            provider: Some("codex".to_string()),
            cursor: Some("cursor-1".to_string()),
            limit: Some(25),
        },
    );
    let record = crate::local::ExternalProviderSessionRecord {
        owner_user_id: crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
        external_session_id: "codex:thread-1".to_string(),
        provider: "codex".to_string(),
        provider_session_id: "thread-1".to_string(),
        title: Some("Investigate failing tests".to_string()),
        title_source: Some("provider_title".to_string()),
        first_prompt_preview: Some("Investigate failing tests in the CLI".to_string()),
        created_at_ms: Some(10),
        last_modified_at_ms: 20,
        worktree_path: Some("/repo".to_string()),
        account_profile: "default".to_string(),
        capabilities: crate::local::ExternalProviderSessionCapabilities {
            can_read_history: true,
        },
        attached_to_chariox: true,
        attached_session_ids: vec!["session-1".to_string()],
        attached_agent_ids: vec!["agent-1".to_string()],
    };
    let response = LocalDaemonResponse::ExternalProviderSessionsListed {
        page: crate::local::ExternalProviderSessionPage {
            sessions: vec![record],
            next_cursor: Some("cursor-2".to_string()),
            has_more: true,
            generated_at_ms: 30,
        },
    };

    let snapshot = serde_json::json!([request, response]);
    assert_eq!(
        snapshot.pointer("/0/ListExternalProviderSessions/provider"),
        Some(&serde_json::json!("codex"))
    );
    assert_eq!(
        snapshot.pointer(
            "/1/ExternalProviderSessionsListed/page/sessions/0/capabilities/can_read_history"
        ),
        Some(&serde_json::json!(true))
    );
    assert_eq!(
        snapshot.pointer("/1/ExternalProviderSessionsListed/page/sessions/0/attached_to_chariox"),
        None
    );
    assert_eq!(
        snapshot.pointer("/1/ExternalProviderSessionsListed/page/sessions/0/attached_agent_ids"),
        None
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("external provider session shape should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "6b01adcc578bdf90bbe162035d023da1b5e2b2805f81e9774148a48cd0ebab6a"
    );
}

#[test]
fn relay_workspace_live_sync_apply_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let context = crate::transport::relay_peer::RemoteWorkspaceLiveSyncApplyContext {
        home_session_id: "session-1".to_string(),
        link_id: "workspace-link-1".to_string(),
        link_name: "team-sync".to_string(),
        source_agent_id: "agent-1".to_string(),
        source_worktree_path: "/home/user/project".to_string(),
        target_user_id: "user-2".to_string(),
        target_machine_id: "machine-2".to_string(),
        target_kernel_id: "kernel-2".to_string(),
        target_repo_root: "/remote/user/project".to_string(),
    };
    let change = crate::git_observer::WorkspaceLiveSyncChange {
        session_id: "session-1".to_string(),
        agent_id: "agent-1".to_string(),
        provider_run_id: "provider-run-1".to_string(),
        prompt_id: "prompt-1".to_string(),
        repo_root: "/home/user/project".to_string(),
        worktree_path: "/home/user/project".to_string(),
        branch: Some("main".to_string()),
        changed_paths: vec!["src/lib.rs".to_string()],
        file_changes: vec![crate::git_observer::WorkspaceLiveSyncFileChange {
            path: "src/lib.rs".to_string(),
            previous_path: None,
            kind: crate::git_observer::WorkspaceLiveSyncFileChangeKind::Modified,
            before_content_base64: Some("b2xkCg==".to_string()),
            after_content_base64: Some("bmV3Cg==".to_string()),
            binary: false,
        }],
        status_fingerprint: " M src/lib.rs".to_string(),
    };
    let target_result = crate::git_observer::WorkspaceLiveSyncTargetResult {
        session_id: "session-1".to_string(),
        link_id: "workspace-link-1".to_string(),
        link_name: "team-sync".to_string(),
        source_agent_id: "agent-1".to_string(),
        source_worktree_path: "/home/user/project".to_string(),
        target_user_id: "user-2".to_string(),
        target_machine_id: "machine-2".to_string(),
        target_kernel_id: "kernel-2".to_string(),
        target_repo_root: "/remote/user/project".to_string(),
        path_results: vec![crate::git_observer::WorkspaceLiveSyncPathApplyResult {
            path: "src/lib.rs".to_string(),
            status: crate::git_observer::WorkspaceLiveSyncApplyStatus::Rebased,
            message: "applied after non-overlap rebase".to_string(),
        }],
    };
    let request = crate::transport::relay_peer::RelayPeerRequest::ApplyWorkspaceLiveSyncChange {
        context,
        change,
    };
    let response =
        crate::transport::relay_peer::RelayPeerResponse::WorkspaceLiveSyncChangeApplied {
            target_result,
        };

    let snapshot = serde_json::json!([request, response]);
    assert_eq!(
        snapshot.pointer("/0/kind"),
        Some(&serde_json::json!("apply_workspace_live_sync_change"))
    );
    assert_eq!(
        snapshot.pointer("/0/context/link_id"),
        Some(&serde_json::json!("workspace-link-1"))
    );
    assert_eq!(
        snapshot.pointer("/0/change/file_changes/0/kind"),
        Some(&serde_json::json!("modified"))
    );
    assert_eq!(
        snapshot.pointer("/1/kind"),
        Some(&serde_json::json!("workspace_live_sync_change_applied"))
    );
    assert_eq!(
        snapshot.pointer("/1/target_result/path_results/0/status"),
        Some(&serde_json::json!("rebased"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("relay workspace live sync shape should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "4b55d5a1dd6ef7e5132a20004156f84c70f88cd11385dc8cb93fb68ddc258107"
    );
}

#[test]
fn relay_home_extension_invocation_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let context = crate::transport::relay_peer::RemoteExtensionInvocationContext {
        home_kernel_id: "home-kernel".to_string(),
        home_session_id: "session-1".to_string(),
        home_agent_id: "agent-1".to_string(),
        leased_agent_id: "leased-agent-1".to_string(),
        worker_provider_run_id: "provider-run-1".to_string(),
        worker_kernel_id: Some("worker-kernel".to_string()),
        worker_machine_id: Some("worker-machine".to_string()),
    };
    let metadata = crate::extension::RemoteExtensionInvocationMetadata {
        invocation_id: "invoke-1".to_string(),
        provider_tool_call_id: Some("tool-call-1".to_string()),
        attempt: 1,
        idempotency_key: Some("idem-1".to_string()),
        started_at_ms: 42,
    };
    let tool = crate::extension::RemoteExtensionTool {
        kind: crate::extension::ExtensionKind::Script,
        name: "home_lookup".to_string(),
        tool_name: "home_lookup".to_string(),
        description: "Home lookup".to_string(),
        input_schema: serde_json::json!({"type": "object"}),
        authority: crate::extension::ExtensionAuthority::Home,
        definition_origin: crate::extension::ExtensionDefinitionOrigin::Home,
        execution_location: crate::extension::ExtensionExecutionLocation::Home,
        safety: Some("read".to_string()),
        timeout_sec: Some(10),
        version_hash: Some("hash-1".to_string()),
    };
    let request = crate::transport::relay_peer::RelayPeerRequest::InvokeHomeExtensionTool {
        context: context.clone(),
        metadata: metadata.clone(),
        tool,
        arguments: serde_json::json!({"query": "status"}),
    };
    let mcp_request = crate::transport::relay_peer::RelayPeerRequest::InvokeHomeMcpProxy {
        context: context.clone(),
        metadata: metadata.clone(),
        name: "home_browser".to_string(),
        tool: crate::extension::RemoteExtensionTool {
            kind: crate::extension::ExtensionKind::Mcp,
            name: "home_browser".to_string(),
            tool_name: "home_browser".to_string(),
            description: "Home MCP".to_string(),
            input_schema: serde_json::json!({}),
            authority: crate::extension::ExtensionAuthority::Home,
            definition_origin: crate::extension::ExtensionDefinitionOrigin::Home,
            execution_location: crate::extension::ExtensionExecutionLocation::Home,
            safety: None,
            timeout_sec: Some(15),
            version_hash: Some("mcp-hash-1".to_string()),
        },
        payload: serde_json::json!({
            "jsonrpc": "2.0",
            "id": "rpc-1",
            "method": "tools/list"
        }),
    };
    let cancel_request =
        crate::transport::relay_peer::RelayPeerRequest::CancelHomeExtensionInvocation {
            context,
            metadata: metadata.clone(),
        };
    let response = crate::transport::relay_peer::RelayPeerResponse::HomeExtensionToolHandled {
        result: crate::transport::runtime_tools::RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({"status": "ok"}),
        },
    };
    let mcp_response = crate::transport::relay_peer::RelayPeerResponse::HomeMcpProxyHandled {
        response: serde_json::json!({
            "jsonrpc": "2.0",
            "id": "rpc-1",
            "result": {"tools": []}
        }),
    };
    let cancel_response =
        crate::transport::relay_peer::RelayPeerResponse::HomeExtensionInvocationCancelled {
            invocation_id: metadata.invocation_id,
            cancelled: true,
        };
    let snapshot = serde_json::json!([
        request,
        mcp_request,
        cancel_request,
        response,
        mcp_response,
        cancel_response
    ]);
    assert_eq!(
        snapshot.pointer("/0/kind"),
        Some(&serde_json::json!("invoke_home_extension_tool"))
    );
    assert_eq!(
        snapshot.pointer("/0/context/worker_kernel_id"),
        Some(&serde_json::json!("worker-kernel"))
    );
    assert_eq!(
        snapshot.pointer("/0/metadata/idempotency_key"),
        Some(&serde_json::json!("idem-1"))
    );
    assert_eq!(
        snapshot.pointer("/0/tool/execution_location"),
        Some(&serde_json::json!("home"))
    );
    assert_eq!(
        snapshot.pointer("/1/kind"),
        Some(&serde_json::json!("invoke_home_mcp_proxy"))
    );
    assert_eq!(
        snapshot.pointer("/1/tool/version_hash"),
        Some(&serde_json::json!("mcp-hash-1"))
    );
    assert_eq!(
        snapshot.pointer("/2/kind"),
        Some(&serde_json::json!("cancel_home_extension_invocation"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("relay home extension shape should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "f95aa81795cca480486b387383662637b3d7e7bdad60077df3cc31141ad6e5d1"
    );
}

#[test]
fn relay_home_credential_proxy_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let context = crate::transport::relay_peer::RemoteExtensionInvocationContext {
        home_kernel_id: "home-kernel".to_string(),
        home_session_id: "session-1".to_string(),
        home_agent_id: "agent-1".to_string(),
        leased_agent_id: "leased-agent-1".to_string(),
        worker_provider_run_id: "provider-run-1".to_string(),
        worker_kernel_id: Some("worker-kernel".to_string()),
        worker_machine_id: Some("worker-machine".to_string()),
    };
    let list_request = crate::transport::relay_peer::RelayPeerRequest::InvokeHomeCredentialTool {
        context: context.clone(),
        tool_name: crate::transport::runtime_tools::LIST_CREDENTIAL_HANDLES_TOOL.to_string(),
        arguments: serde_json::json!({}),
    };
    let browser_secret_request =
        crate::transport::relay_peer::RelayPeerRequest::ResolveHomeCredentialSecret {
            context: context.clone(),
            credential_id: "gmail-password".to_string(),
            injection: crate::transport::relay_peer::RemoteCredentialSecretInjection::Browser {
                target_url: "https://accounts.google.com/signin".to_string(),
            },
        };
    let pty_secret_request =
        crate::transport::relay_peer::RelayPeerRequest::ResolveHomeCredentialSecret {
            context: context.clone(),
            credential_id: "ssh-password".to_string(),
            injection: crate::transport::relay_peer::RemoteCredentialSecretInjection::Pty,
        };
    let computer_secret_request =
        crate::transport::relay_peer::RelayPeerRequest::ResolveHomeCredentialSecret {
            context: context.clone(),
            credential_id: "desktop-password".to_string(),
            injection: crate::transport::relay_peer::RemoteCredentialSecretInjection::Computer,
        };
    let list_response =
        crate::transport::relay_peer::RelayPeerResponse::HomeCredentialToolHandled {
            result: crate::transport::runtime_tools::RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({
                    "credentials": [{
                        "id": "gmail-password",
                        "allowed_uses": ["browser"]
                    }]
                }),
            },
        };
    let secret_response =
        crate::transport::relay_peer::RelayPeerResponse::HomeCredentialSecretResolved {
            credential_id: "gmail-password".to_string(),
            secret_input: crate::transport::relay_peer::RemoteCredentialSecretInput::new(
                "redacted-by-test-fixture".to_string(),
            ),
        };
    let snapshot = serde_json::json!([
        list_request,
        browser_secret_request,
        computer_secret_request,
        pty_secret_request,
        list_response,
        secret_response
    ]);
    assert_eq!(
        snapshot.pointer("/0/kind"),
        Some(&serde_json::json!("invoke_home_credential_tool"))
    );
    assert_eq!(
        snapshot.pointer("/1/injection/kind"),
        Some(&serde_json::json!("browser"))
    );
    assert_eq!(
        snapshot.pointer("/2/injection/kind"),
        Some(&serde_json::json!("computer"))
    );
    assert_eq!(
        snapshot.pointer("/3/injection/kind"),
        Some(&serde_json::json!("pty"))
    );
    assert_eq!(
        snapshot.pointer("/4/kind"),
        Some(&serde_json::json!("home_credential_tool_handled"))
    );
    assert_eq!(
        snapshot.pointer("/5/kind"),
        Some(&serde_json::json!("home_credential_secret_resolved"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("relay home credential shape should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "1db26e45df57ec3bc21aeba0aed6ca232352a75163d331398ab4bac2b98e51d5"
    );
}

#[test]
fn relay_home_credential_secret_debug_output_is_redacted() {
    let response = crate::transport::relay_peer::RelayPeerResponse::HomeCredentialSecretResolved {
        credential_id: "desktop-password".to_string(),
        secret_input: crate::transport::relay_peer::RemoteCredentialSecretInput::new(
            "must-not-appear-in-relay-debug".to_string(),
        ),
    };

    let debug = format!("{response:?}");
    assert!(debug.contains("[redacted remote credential secret input]"));
    assert!(!debug.contains("must-not-appear-in-relay-debug"));
}

#[test]
fn local_daemon_protocol_extension_install_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let mcp = LocalDaemonRequest::InstallMcpServer(crate::local::InstallMcpServerRequest {
        workspace_id: Some("/repo".to_string()),
        config: crate::mcp::CharioxMcpServerConfig {
            name: "github".to_string(),
            transport: crate::mcp::CharioxMcpTransportConfig::Stdio {
                command: "npx".to_string(),
                args: vec![
                    "-y".to_string(),
                    "@modelcontextprotocol/server-github".to_string(),
                ],
                env: Default::default(),
                credential_env: std::collections::BTreeMap::from([(
                    "GITHUB_TOKEN".to_string(),
                    crate::mcp::CharioxMcpCredentialBinding {
                        credential: "github-token".to_string(),
                    },
                )]),
                env_vars: Vec::new(),
                cwd: None,
            },
            enabled: true,
            required: false,
            startup_timeout_sec: None,
            tool_timeout_sec: Some(30),
            enabled_tools: None,
            disabled_tools: None,
            tools: Default::default(),
        },
    });
    let skill = LocalDaemonRequest::UpsertSkill(crate::local::UpsertSkillRequest {
        workspace_id: Some("/repo".to_string()),
        source: crate::local::SkillInstallSource::Url {
            url: "https://github.com/example/skills/tree/main/review".to_string(),
        },
    });
    let connector = LocalDaemonRequest::UpsertConnector(crate::local::UpsertConnectorRequest {
        connector: crate::connector::CharioxConnectorDefinition {
            kind: "connector".to_string(),
            name: "status-api".to_string(),
            description: "Read status".to_string(),
            adapter: "http".to_string(),
            credential: Some(crate::connector::ConnectorCredentialPolicy { required: true }),
            timeout_ms: 30000,
            max_response_bytes: 1048576,
            operations: vec![crate::connector::ConnectorOperation {
                name: "get".to_string(),
                description: "Read status".to_string(),
                safety: crate::connector::ConnectorSafety::Read,
                input_schema: serde_json::json!({"type":"object"}),
                config: serde_json::json!({"method":"GET","base_url":"https://example.test","path":"/status"}),
            }],
        },
    });
    let sync = LocalDaemonRequest::SyncRemoteExtensionManifest(
        crate::local::SyncRemoteExtensionManifestRequest {
            agent_ref: "agent-1".to_string(),
        },
    );
    let audit =
        LocalDaemonRequest::ListHomeExtensionAudit(crate::local::ListHomeExtensionAuditRequest {
            agent_ref: "agent-1".to_string(),
            limit: Some(10),
        });

    let snapshot = serde_json::json!([mcp, skill, connector, sync, audit]);
    assert_eq!(
        snapshot
            .pointer("/0/InstallMcpServer/config/transport/credential_env/GITHUB_TOKEN/credential"),
        Some(&serde_json::json!("github-token"))
    );
    assert_eq!(
        snapshot.pointer("/1/UpsertSkill/source/type"),
        Some(&serde_json::json!("url"))
    );
    assert_eq!(
        snapshot.pointer("/2/UpsertConnector/connector/operations/0/config/path"),
        Some(&serde_json::json!("/status"))
    );
    assert_eq!(
        snapshot.pointer("/3/SyncRemoteExtensionManifest/agent_ref"),
        Some(&serde_json::json!("agent-1"))
    );
    assert_eq!(
        snapshot.pointer("/4/ListHomeExtensionAudit/limit"),
        Some(&serde_json::json!(10))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("extension install snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "2409b4d10bab0b296bd880e060b8931116f30f2452e5c9205e7701f9ccbe0108"
    );
}
