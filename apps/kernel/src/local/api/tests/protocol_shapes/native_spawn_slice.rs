use super::*;

#[test]
fn local_daemon_protocol_native_provider_interaction_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 287);

    let request = LocalDaemonRequest::RequestNativeProviderInteraction(
        RequestNativeProviderInteractionRequest::allow_deny(
            "session-1",
            "agent-1",
            "native-permission-1",
            Some("Approve Claude Code Bash?".to_string()),
            "Claude Code wants to run:\n\n`echo hello`",
            Some(300),
        ),
    );
    let snapshot = serde_json::to_value(request).expect("request should serialize");
    assert_eq!(
        snapshot.pointer("/RequestNativeProviderInteraction/interaction_id"),
        Some(&serde_json::json!("native-permission-1"))
    );
    assert_eq!(
        snapshot.pointer("/RequestNativeProviderInteraction/choices/0/id"),
        Some(&serde_json::json!("allow_once"))
    );
    assert_eq!(
        snapshot.pointer("/RequestNativeProviderInteraction/default_on_timeout"),
        Some(&serde_json::json!("deny"))
    );
    let response = LocalDaemonResponse::NativeProviderInteractionResolved {
        resolution: super::NativeProviderInteractionResolution {
            status: "answered".to_string(),
            choice_id: Some("allow_once".to_string()),
            reply: Some("allow".to_string()),
        },
    };
    let response_snapshot = serde_json::to_value(response).expect("response should serialize");
    assert_eq!(
        response_snapshot.pointer("/NativeProviderInteractionResolved/resolution/reply"),
        Some(&serde_json::json!("allow"))
    );
    let serialized = serde_json::to_string(&serde_json::json!({
        "request": snapshot,
        "response": response_snapshot,
    }))
    .expect("native provider interaction snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "de3b26de0ee408204a7afaf15173bf02180358db6c10ea566f0f9f22b0d32031"
    );
}

#[test]
fn local_daemon_protocol_kernel_targeted_spawn_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 287);

    let request = LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
        account_profile: None,
        session_id: "session-1".to_string(),
        alias: Some("worker".to_string()),
        provider: Some("codex".to_string()),
        model: Some("gpt-5".to_string()),
        effort: Some("medium".to_string()),
        execution_mode: Some(AgentExecutionMode::Build),
        permission_level: Some(AgentPermissionLevel::Required),
        worktree_id: None,
        kernel_ref: Some("kernel-worker".to_string()),
        slice_ref: None,
        worktree_placement: None,
        metaagent: false,
    });
    let snapshot = serde_json::to_value(request).expect("request should serialize");
    assert_eq!(
        snapshot.pointer("/SpawnAgent/kernel_ref"),
        Some(&serde_json::json!("kernel-worker"))
    );
    assert_eq!(snapshot.pointer("/SpawnAgent/machine_ref"), None);
    let serialized =
        serde_json::to_string(&snapshot).expect("kernel-targeted spawn snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "712cecc5815da7fa33661de8db724f62e1aa90cfdbe56e332a5d13fbc8f4b848"
    );
}

#[test]
fn local_daemon_protocol_slice_targeted_spawn_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 287);

    let request = LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
        account_profile: None,
        session_id: "session-1".to_string(),
        alias: Some("worker".to_string()),
        provider: Some("codex".to_string()),
        model: Some("gpt-5".to_string()),
        effort: Some("medium".to_string()),
        execution_mode: Some(AgentExecutionMode::Build),
        permission_level: Some(AgentPermissionLevel::Required),
        worktree_id: None,
        kernel_ref: None,
        slice_ref: Some("linux-dev".to_string()),
        worktree_placement: None,
        metaagent: false,
    });
    let snapshot = serde_json::to_value(request).expect("request should serialize");
    assert_eq!(
        snapshot.pointer("/SpawnAgent/slice_ref"),
        Some(&serde_json::json!("linux-dev"))
    );
    assert_eq!(
        snapshot.pointer("/SpawnAgent/kernel_ref"),
        Some(&serde_json::Value::Null)
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("slice-targeted spawn snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "a7bd0c2bb693c63aa515a2e89f98a5b6144bb750fdcd266bc87e9d4903d4a1d4"
    );
}

#[test]
fn local_daemon_protocol_batch_spawn_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 287);

    let request = LocalDaemonRequest::SpawnAgents(SpawnAgentsRequest {
        session_id: "session-1".to_string(),
        agents: vec![SpawnAgentsRequestItem {
            account_profile: None,
            alias: Some("worker-1".to_string()),
            provider: Some("codex".to_string()),
            model: Some("gpt-5".to_string()),
            effort: Some("medium".to_string()),
            execution_mode: Some(AgentExecutionMode::Build),
            permission_level: Some(AgentPermissionLevel::Required),
            worktree_id: None,
            kernel_ref: Some("kernel-worker".to_string()),
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        }],
    });
    let snapshot = serde_json::to_value(request).expect("request should serialize");
    assert_eq!(
        snapshot.pointer("/SpawnAgents/agents/0/kernel_ref"),
        Some(&serde_json::json!("kernel-worker"))
    );
    assert_eq!(snapshot.pointer("/SpawnAgents/agents/0/machine_ref"), None);
    let serialized = serde_json::to_string(&snapshot).expect("batch spawn snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "3febaf301396d6312a7a0d28248f5d6d13851c2fab5cb8d06ca8017df095bfd2"
    );
}

#[test]
fn local_daemon_protocol_turn_undo_and_agent_fork_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 287);

    let undo_request = LocalDaemonRequest::UndoTurn(crate::local::UndoTurnRequest {
        session_id: "session-1".to_string(),
        agent_ref: Some("worker".to_string()),
        turn_ref: Some("turn-1".to_string()),
    });
    let fork_request = LocalDaemonRequest::ForkAgent(crate::local::ForkAgentRequest {
        session_id: "session-1".to_string(),
        source_agent_ref: None,
        alias: Some("fork".to_string()),
    });
    let undo_response =
        LocalDaemonResponse::TurnUndone {
            result: crate::local::TurnUndoResult {
                session_id: "session-1".to_string(),
                agent_id: "agent-1".to_string(),
                turn_id: "turn-1".to_string(),
                prompt_id: "prompt-1".to_string(),
                provider_run_id: "provider-run-1".to_string(),
                reverted_paths: vec!["src/lib.rs".to_string()],
                path_results:
                    vec![crate::workspace_live_sync_journal::WorkspaceLiveSyncPathApplyResult {
                path: "src/lib.rs".to_string(),
                status: crate::workspace_live_sync_journal::WorkspaceLiveSyncApplyStatus::Applied,
                message: "restored".to_string(),
            }],
            },
        };
    let mut agent_value = serde_json::to_value(crate::agent::AgentInstance::new(
        "agent-2",
        "agent-ref-2",
        "session-1",
        Some("fork".to_string()),
        "codex",
        Some("gpt-5".to_string()),
        Some("medium".to_string()),
        Some("worktree-1".to_string()),
        crate::agent::GridPosition::new(0, 0, 1, 1),
    ))
    .expect("agent snapshot should encode");
    agent_value["created_at_ms"] = serde_json::json!(1_000);
    agent_value["last_activity_at_ms"] = serde_json::json!(1_000);
    let agent: crate::agent::AgentInstance =
        serde_json::from_value(agent_value).expect("agent snapshot should decode");
    let run_request = crate::provider::LaunchProviderRequest::new(
        "session-1",
        "codex",
        "codex",
        "default",
        "gpt-5",
    )
    .with_agent_id("agent-2")
    .with_variant(Some("medium".to_string()));
    let run = crate::provider::RuntimeProviderRun::new(
        "provider-run-2",
        &run_request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "codex".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    let mut run_value = serde_json::to_value(run).expect("provider run should encode");
    run_value["started_at_ms"] = serde_json::json!(1_000);
    run_value["last_activity_at_ms"] = serde_json::json!(1_000);
    let mut session_value = serde_json::to_value(crate::session::RuntimeSession::new(
        "session-1",
        None,
        "workspace-1",
        "worktree-1",
        "machine-1",
        "daemon-1",
    ))
    .expect("session snapshot should encode");
    session_value["created_at_ms"] = serde_json::json!(1_000);
    session_value["last_used_at_ms"] = serde_json::json!(1_000);
    let fork_response = LocalDaemonResponse::AgentForked {
        source_agent_id: "agent-1".to_string(),
        agent,
        provider_run: serde_json::from_value(run_value).expect("provider run should decode"),
        session: serde_json::from_value(session_value).expect("session snapshot should decode"),
    };

    let snapshot = serde_json::json!([undo_request, fork_request, undo_response, fork_response]);
    assert_eq!(
        snapshot.pointer("/0/UndoTurn/agent_ref"),
        Some(&serde_json::json!("worker"))
    );
    assert_eq!(snapshot.pointer("/1/ForkAgent/source_agent_ref"), None);
    assert_eq!(
        snapshot.pointer("/2/TurnUndone/result/path_results/0/status"),
        Some(&serde_json::json!("applied"))
    );
    assert_eq!(
        snapshot.pointer("/3/AgentForked/provider_run/agent_instance_id"),
        Some(&serde_json::json!("agent-2"))
    );
    let serialized = serde_json::to_string(&snapshot).expect("turn action snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "dd8a65c9a2897467813564b15e3e437948826e71196a23d96e254be55552c933"
    );
}

#[test]
fn local_daemon_protocol_slice_targeted_create_session_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 287);

    let request = LocalDaemonRequest::CreateSession(
        CreateSessionRequest::new("workspace-1", "worktree-1")
            .with_alias("slice-session")
            .with_slice_ref("linux-dev"),
    );
    let snapshot = serde_json::to_value(request).expect("request should serialize");
    assert_eq!(
        snapshot.pointer("/CreateSession/slice_ref"),
        Some(&serde_json::json!("linux-dev"))
    );
    let serialized = serde_json::to_string(&snapshot)
        .expect("slice-targeted create session snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "eae0e4f3d41fcebc91db7c38d98b03818730a08f3c213d8068f38d8634e6e236"
    );
}

#[test]
fn local_daemon_protocol_create_session_worktree_placement_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 287);

    let request = LocalDaemonRequest::CreateSession(
        CreateSessionRequest::new("workspace-1", "worktree-1").with_worktree_placement(
            crate::agent::GitWorktreePlacement {
                target_directory: Some("../feature".to_string()),
                branch: Some("feature/session".to_string()),
                from_ref: Some("main".to_string()),
            },
        ),
    );
    let snapshot = serde_json::to_value(request).expect("request should serialize");
    assert_eq!(
        snapshot.pointer("/CreateSession/worktree_placement"),
        Some(&serde_json::json!({
            "target_directory": "../feature",
            "branch": "feature/session",
            "from_ref": "main",
        }))
    );
    let serialized = serde_json::to_string(&snapshot)
        .expect("worktree-placement create session snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "260da39fcac4bf240f45e1627bc729089c7b2f60bab77e577a9948d652923c35"
    );
}

#[test]
fn local_daemon_protocol_kernel_targeted_create_session_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 287);

    let request = LocalDaemonRequest::CreateSession(
        CreateSessionRequest::new("workspace-1", "worktree-1")
            .with_alias("remote-session")
            .with_kernel_ref("kernel-worker")
            .with_metaagent(true),
    );
    let snapshot = serde_json::to_value(request).expect("request should serialize");
    assert_eq!(
        snapshot.pointer("/CreateSession/kernel_ref"),
        Some(&serde_json::json!("kernel-worker"))
    );
    assert_eq!(snapshot.pointer("/CreateSession/machine_ref"), None);
    assert_eq!(
        snapshot.pointer("/CreateSession/metaagent"),
        Some(&serde_json::json!(true))
    );
    let serialized = serde_json::to_string(&snapshot)
        .expect("kernel-targeted create session snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "1f7e945b15c33327efa044e75bb7b92eb0e4a5aefa9f26106019e0cabf113a72"
    );
}

#[test]
fn local_daemon_protocol_slice_record_relay_endpoint_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 287);

    let response = LocalDaemonResponse::Slice {
        slice: crate::slice::SliceRecord {
            id: "slice-1".to_string(),
            name: "linux-dev".to_string(),
            owner_kernel_id: "home-kernel".to_string(),
            owner_machine_id: "home-machine".to_string(),
            session_id: Some("session-1".to_string()),
            session_ids: vec!["session-1".to_string(), "session-2".to_string()],
            agent_ids: vec!["agent-1".to_string(), "agent-2".to_string()],
            backend: crate::slice::SliceBackendKind::LocalDocker,
            os: "linux".to_string(),
            display_mode: crate::slice::SliceDisplayMode::Headed,
            status: crate::slice::SliceStatus::Running,
            last_operation: Some("start".to_string()),
            last_operation_status: Some(crate::slice::SliceOperationStatus::Completed),
            last_error: None,
            last_operation_at_ms: Some(1900),
            workspace_id: Some("workspace-1".to_string()),
            worktree_id: Some("worktree-1".to_string()),
            workspace_mount: Some("/repo".to_string()),
            development: None,
            development_storage_root: Some("/state/slices/development/slice-1".to_string()),
            development_publication: None,
            worker_kernel_ref: "slice:linux-dev".to_string(),
            worker_kernel_id: Some("slice-worker".to_string()),
            worker_machine_id: Some("slice:slice-1".to_string()),
            relay_endpoint: Some(crate::slice::SliceRelayEndpoint {
                url: "ws://127.0.0.1:43130".to_string(),
                private: true,
            }),
            local_docker_ports: Some(crate::slice::SliceLocalDockerPorts {
                codex: 44000,
                opencode: 44300,
                kernel: 44600,
                mcp: 44900,
                relay: 45200,
                novnc: 45500,
                codex_range_start: 46000,
                opencode_range_start: 51200,
            }),
            providers: vec!["codex".to_string(), "opencode".to_string()],
            provider_auth: vec![crate::slice_provider_auth::SliceProviderAuthSummary {
                provider: "codex".to_string(),
                account_profile: "default".to_string(),
                state: crate::slice_provider_auth::SliceProviderAuthState::Configured,
                auth_type: Some("chatgpt".to_string()),
                account_id: Some("acct-1".to_string()),
                email: None,
                organization_id: None,
                organization_name: None,
                subscription_type: None,
                source: "home_codex_auth_json".to_string(),
            }],
            saved_state_ref: None,
            saved_state_status: None,
            saved_state_updated_at_ms: None,
            display_endpoint: None,
            created_at_ms: 1000,
            updated_at_ms: 2000,
        },
    };
    let snapshot = serde_json::to_value(response).expect("response should serialize");
    assert_eq!(
        snapshot.pointer("/Slice/slice/relay_endpoint/url"),
        Some(&serde_json::json!("ws://127.0.0.1:43130"))
    );
    assert_eq!(
        snapshot.pointer("/Slice/slice/relay_endpoint/private"),
        Some(&serde_json::json!(true))
    );
    assert_eq!(
        snapshot.pointer("/Slice/slice/session_id"),
        Some(&serde_json::json!("session-1"))
    );
    assert_eq!(
        snapshot.pointer("/Slice/slice/local_docker_ports/novnc"),
        Some(&serde_json::json!(45500))
    );
    assert_eq!(
        snapshot.pointer("/Slice/slice/development_storage_root"),
        Some(&serde_json::json!("/state/slices/development/slice-1"))
    );
    let serialized = serde_json::to_string(&snapshot).expect("slice record snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "b1dd5c4a49ec8243c410f8d5e063842e113505e0121ff58b4a1859ec7bb8f24d"
    );
}

#[test]
fn local_daemon_protocol_slice_saved_state_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 287);

    let create_request = LocalDaemonRequest::CreateSlice(crate::local::CreateSliceRequest {
        name: "linux-dev".to_string(),
        backend: crate::slice::SliceBackendKind::LocalDocker,
        os: "linux".to_string(),
        display_mode: crate::slice::SliceDisplayMode::Headed,
        workspace_id: Some("workspace-1".to_string()),
        worktree_id: Some("worktree-1".to_string()),
        workspace_mount: Some("/repo".to_string()),
        development: None,
        worker_kernel_ref: None,
        display_url: None,
        provider_auth: Vec::new(),
        from_saved_state: None,
        base: Some(crate::local::SliceCreateBase::Clean),
    });
    let save_request = LocalDaemonRequest::SaveSliceState(crate::local::SliceStateSaveRequest {
        slice_ref: "linux-dev".to_string(),
        mode: Some(crate::local::SliceStateSaveMode::RestartAgents),
        scope: Some(crate::local::SliceStateSaveScope::FutureSlices),
    });
    let status_request =
        LocalDaemonRequest::GetSliceStateStatus(crate::local::SliceStateStatusRequest {
            slice_ref: "linux-dev".to_string(),
        });
    let reset_request = LocalDaemonRequest::ResetSliceState(crate::local::SliceStateResetRequest {
        slice_ref: "linux-dev".to_string(),
    });
    let backup_request =
        LocalDaemonRequest::CreateSliceBackup(crate::local::CreateSliceBackupRequest {
            slice_ref: "linux-dev".to_string(),
            name: Some("gmail-ready".to_string()),
        });
    let saved_state = crate::slice::SliceSavedStateRecord {
        id: "linux-dev".to_string(),
        slice_name: "linux-dev".to_string(),
        source_slice_id: "slice-1".to_string(),
        backend: crate::slice::SliceBackendKind::LocalDocker,
        os: "linux".to_string(),
        image_ref: "chariox-slice-state:linux-dev".to_string(),
        home_archive_path: "/home/user/.chariox/slices/states/linux-dev/home.tar.zst".to_string(),
        manifest_path: "/home/user/.chariox/slices/states/linux-dev/manifest.json".to_string(),
        created_at_ms: 1000,
        updated_at_ms: 2000,
        size_bytes: Some(4096),
        last_operation: Some("state.save".to_string()),
        last_operation_status: Some(crate::slice::SliceOperationStatus::Completed),
        last_error: None,
    };
    let backup = crate::slice::SliceBackupRecord {
        id: "linux-dev-20260609-181500".to_string(),
        name: "gmail-ready".to_string(),
        source_slice_id: "slice-1".to_string(),
        source_state_id: "linux-dev".to_string(),
        image_ref: "chariox-slice-backup:linux-dev-20260609-181500".to_string(),
        home_archive_path:
            "/home/user/.chariox/slices/backups/linux-dev-20260609-181500/home.tar.zst".to_string(),
        manifest_path: "/home/user/.chariox/slices/backups/linux-dev-20260609-181500/manifest.json"
            .to_string(),
        created_at_ms: 3000,
        size_bytes: Some(4096),
    };
    let slice = crate::slice::SliceRecord {
        id: "slice-1".to_string(),
        name: "linux-dev".to_string(),
        owner_kernel_id: "home-kernel".to_string(),
        owner_machine_id: "home-machine".to_string(),
        session_id: None,
        session_ids: Vec::new(),
        agent_ids: Vec::new(),
        backend: crate::slice::SliceBackendKind::LocalDocker,
        os: "linux".to_string(),
        display_mode: crate::slice::SliceDisplayMode::Headed,
        status: crate::slice::SliceStatus::Stopped,
        last_operation: Some("state.save".to_string()),
        last_operation_status: Some(crate::slice::SliceOperationStatus::Completed),
        last_error: None,
        last_operation_at_ms: Some(2000),
        workspace_id: Some("workspace-1".to_string()),
        worktree_id: Some("worktree-1".to_string()),
        workspace_mount: Some("/repo".to_string()),
        development: None,
        development_storage_root: None,
        development_publication: None,
        worker_kernel_ref: "slice:linux-dev".to_string(),
        worker_kernel_id: None,
        worker_machine_id: None,
        relay_endpoint: None,
        local_docker_ports: None,
        providers: Vec::new(),
        provider_auth: Vec::new(),
        saved_state_ref: Some("linux-dev".to_string()),
        saved_state_status: Some(crate::slice::SliceSavedStateStatus::Saved),
        saved_state_updated_at_ms: Some(2000),
        display_endpoint: None,
        created_at_ms: 1000,
        updated_at_ms: 2000,
    };
    let snapshot = serde_json::json!([
        create_request,
        save_request,
        status_request,
        reset_request,
        backup_request,
        LocalDaemonResponse::SliceStateSaved {
            slice: slice.clone(),
            state: saved_state.clone(),
        },
        LocalDaemonResponse::SliceStateStatus {
            slice: slice.clone(),
            state: Some(saved_state.clone()),
        },
        LocalDaemonResponse::SliceStateReset {
            slice: slice.clone(),
            removed_state: Some(saved_state),
        },
        LocalDaemonResponse::SliceBackupCreated {
            slice,
            backup,
            instructions: "swap backup directory with active state directory".to_string(),
        },
    ]);
    assert_eq!(
        snapshot.pointer("/0/CreateSlice/base"),
        Some(&serde_json::json!("clean"))
    );
    assert_eq!(
        snapshot.pointer("/1/SaveSliceState/slice_ref"),
        Some(&serde_json::json!("linux-dev"))
    );
    assert_eq!(
        snapshot.pointer("/1/SaveSliceState/mode"),
        Some(&serde_json::json!("restart_agents"))
    );
    assert_eq!(
        snapshot.pointer("/1/SaveSliceState/scope"),
        Some(&serde_json::json!("future_slices"))
    );
    assert_eq!(
        snapshot.pointer("/5/SliceStateSaved/state/image_ref"),
        Some(&serde_json::json!("chariox-slice-state:linux-dev"))
    );
    assert_eq!(
        snapshot.pointer("/6/SliceStateStatus/slice/saved_state_status"),
        Some(&serde_json::json!("saved"))
    );
    assert_eq!(
        snapshot.pointer("/8/SliceBackupCreated/backup/source_state_id"),
        Some(&serde_json::json!("linux-dev"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("slice saved state snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "a17f91fc015b37f06e1bc559f5882c079c54782cfbb8d4fe33e8995992db2cb6"
    );
}

#[test]
fn local_daemon_protocol_slice_multi_repository_development_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 287);
    let request = LocalDaemonRequest::CreateSlice(crate::local::CreateSliceRequest {
        name: "project-slice".to_string(),
        backend: crate::slice::SliceBackendKind::LocalDocker,
        os: "linux".to_string(),
        display_mode: crate::slice::SliceDisplayMode::Headless,
        workspace_id: Some("/repo/primary".to_string()),
        worktree_id: Some("/repo/primary-worktree".to_string()),
        workspace_mount: Some("/repo/primary-worktree".to_string()),
        development: Some(
            crate::managed_context::package::ManagedContextDevelopmentSelection::SourceProject {
                project_id: "project-1".to_string(),
                repositories: vec![
                    crate::managed_context::development::DevelopmentSourceRepositoryBinding {
                        role: crate::managed_context::development::DevelopmentRepositoryRole::Primary,
                        workspace_id: "/repo/primary".to_string(),
                        worktree_id: Some("/repo/primary-worktree".to_string()),
                    },
                    crate::managed_context::development::DevelopmentSourceRepositoryBinding {
                        role: crate::managed_context::development::DevelopmentRepositoryRole::Supporting,
                        workspace_id: "/repo/supporting".to_string(),
                        worktree_id: None,
                    },
                ],
            },
        ),
        worker_kernel_ref: None,
        display_url: None,
        provider_auth: Vec::new(),
        from_saved_state: None,
        base: None,
    });
    let publication = crate::slice::SliceDevelopmentPublication {
        publication_id: "slice-1-0123456789abcdef".to_string(),
        destination_root: "/state/slices/development/slice-1/publication".to_string(),
        primary_repository_path: "/state/slices/development/slice-1/publication/primary"
            .to_string(),
        repository_paths: vec![
            "/state/slices/development/slice-1/publication/primary".to_string(),
            "/state/slices/development/slice-1/publication/supporting".to_string(),
        ],
    };
    let snapshot = serde_json::json!([request, publication]);
    assert_eq!(
        snapshot.pointer("/0/CreateSlice/development/kind"),
        Some(&serde_json::json!("source_project"))
    );
    assert_eq!(
        snapshot.pointer("/0/CreateSlice/development/repositories/1/workspaceId"),
        Some(&serde_json::json!("/repo/supporting"))
    );
    assert_eq!(
        snapshot.pointer("/1/primaryRepositoryPath"),
        Some(&serde_json::json!(
            "/state/slices/development/slice-1/publication/primary"
        ))
    );
    assert_eq!(
        snapshot.pointer("/1/repositoryPaths/1"),
        Some(&serde_json::json!(
            "/state/slices/development/slice-1/publication/supporting"
        ))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("slice development snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "cd00f8ea87f2e45c7159acebd8c4a1a10138eb88b24c900cee9f3c041ce07293"
    );
}

#[test]
fn local_daemon_protocol_slice_auth_remove_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 287);

    let request =
        LocalDaemonRequest::RemoveSliceProviderAuth(crate::local::RemoveSliceProviderAuthRequest {
            slice_ref: "linux-dev".to_string(),
            provider: "codex".to_string(),
            account_profile: "default".to_string(),
        });
    let response = LocalDaemonResponse::SliceProviderAuthRemoved {
        slice: crate::slice::SliceRecord {
            id: "slice-1".to_string(),
            name: "linux-dev".to_string(),
            owner_kernel_id: "home-kernel".to_string(),
            owner_machine_id: "home-machine".to_string(),
            session_id: None,
            session_ids: Vec::new(),
            agent_ids: Vec::new(),
            backend: crate::slice::SliceBackendKind::LocalDocker,
            os: "linux".to_string(),
            display_mode: crate::slice::SliceDisplayMode::Headed,
            status: crate::slice::SliceStatus::Running,
            last_operation: None,
            last_operation_status: None,
            last_error: None,
            last_operation_at_ms: None,
            workspace_id: Some("workspace-1".to_string()),
            worktree_id: Some("worktree-1".to_string()),
            workspace_mount: Some("/repo".to_string()),
            development: None,
            development_storage_root: None,
            development_publication: None,
            worker_kernel_ref: "slice:linux-dev".to_string(),
            worker_kernel_id: Some("slice-worker".to_string()),
            worker_machine_id: Some("slice:slice-1".to_string()),
            relay_endpoint: None,
            local_docker_ports: None,
            providers: vec!["codex".to_string()],
            provider_auth: Vec::new(),
            saved_state_ref: None,
            saved_state_status: None,
            saved_state_updated_at_ms: None,
            display_endpoint: None,
            created_at_ms: 1000,
            updated_at_ms: 2001,
        },
        provider: "codex".to_string(),
        status: "removed".to_string(),
    };
    let snapshot = serde_json::json!([request, response]);
    assert_eq!(
        snapshot.pointer("/0/RemoveSliceProviderAuth/slice_ref"),
        Some(&serde_json::json!("linux-dev"))
    );
    assert_eq!(
        snapshot.pointer("/1/SliceProviderAuthRemoved/status"),
        Some(&serde_json::json!("removed"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("slice auth remove snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "6cc54f6061ce318ccafa45896ebc85d8c383619d91ee1b22247fba742b7d3de5"
    );
}

#[test]
fn local_daemon_protocol_slice_provider_login_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 287);

    let request =
        LocalDaemonRequest::StartSliceProviderLogin(crate::local::StartSliceProviderLoginRequest {
            slice_ref: "linux-dev".to_string(),
            provider: "codex".to_string(),
            account_profile: "default".to_string(),
        });
    let response = LocalDaemonResponse::SliceProviderLoginStarted {
        slice: crate::slice::SliceRecord {
            id: "slice-1".to_string(),
            name: "linux-dev".to_string(),
            owner_kernel_id: "home-kernel".to_string(),
            owner_machine_id: "home-machine".to_string(),
            session_id: None,
            session_ids: Vec::new(),
            agent_ids: Vec::new(),
            backend: crate::slice::SliceBackendKind::LocalDocker,
            os: "linux".to_string(),
            display_mode: crate::slice::SliceDisplayMode::Headed,
            status: crate::slice::SliceStatus::Running,
            last_operation: None,
            last_operation_status: None,
            last_error: None,
            last_operation_at_ms: None,
            workspace_id: Some("workspace-1".to_string()),
            worktree_id: Some("worktree-1".to_string()),
            workspace_mount: Some("/repo".to_string()),
            development: None,
            development_storage_root: None,
            development_publication: None,
            worker_kernel_ref: "slice:linux-dev".to_string(),
            worker_kernel_id: Some("slice-worker".to_string()),
            worker_machine_id: Some("slice:slice-1".to_string()),
            relay_endpoint: None,
            local_docker_ports: None,
            providers: vec!["codex".to_string()],
            provider_auth: Vec::new(),
            saved_state_ref: None,
            saved_state_status: None,
            saved_state_updated_at_ms: None,
            display_endpoint: None,
            created_at_ms: 1000,
            updated_at_ms: 2000,
        },
        login: crate::slice::SliceProviderLoginStart {
            provider: "codex".to_string(),
            login_kind: "device".to_string(),
            auth_url: Some("https://auth.example".to_string()),
            verification_url: Some("https://auth.example".to_string()),
            user_code: Some("ABCD-EFGH".to_string()),
            status: "started".to_string(),
            message: "Open https://auth.example and enter ABCD-EFGH".to_string(),
        },
    };
    let snapshot = serde_json::json!([request, response]);
    assert_eq!(
        snapshot.pointer("/0/StartSliceProviderLogin/slice_ref"),
        Some(&serde_json::json!("linux-dev"))
    );
    assert_eq!(
        snapshot.pointer("/1/SliceProviderLoginStarted/login/user_code"),
        Some(&serde_json::json!("ABCD-EFGH"))
    );
    assert_eq!(
        snapshot.pointer("/1/SliceProviderLoginStarted/login/verification_url"),
        Some(&serde_json::json!("https://auth.example"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("slice provider login snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "a92b8e698af45ba2c2e9a15a169dcf6d3f159f785a3422d715078dd26f9f120a"
    );
}
