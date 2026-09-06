#![allow(unused_imports)]
use super::support::*;

#[test]
fn provider_account_materialization_peer_shape_is_versioned_and_debug_redacted() {
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        43
    );
    let mut materialization = crate::account_profile::ProviderAccountMaterialization {
        profile: crate::account_profile::ProviderAccountReplicaMetadata {
            owner_user_id: "user-1".to_string(),
            provider: "codex".to_string(),
            profile_id: "work".to_string(),
            label: "Work".to_string(),
            origin: crate::account_profile::ProviderAccountProfileOrigin::CharioxCreated,
            is_default: false,
        },
        files: Vec::new(),
        generated_at_ms: 1_234,
    };
    let request = RelayPeerRequest::EnsureRemoteProviderAccount {
        context: crate::transport::relay_peer::RemoteProviderAccountSyncContext {
            home_kernel_id: "home".to_string(),
            home_session_id: "session-1".to_string(),
            home_agent_id: "agent-1".to_string(),
            execution_lease_id: "lease-1".to_string(),
        },
        materialization: materialization.clone(),
    };
    let value = serde_json::to_value(request).expect("request should serialize");
    assert_eq!(
        value.pointer("/kind"),
        Some(&serde_json::json!("ensure_remote_provider_account"))
    );
    assert_eq!(
        value.pointer("/materialization/profile/profile_id"),
        Some(&serde_json::json!("work"))
    );
    assert!(value.pointer("/materialization/profile/locator").is_none());

    materialization
        .files
        .push(crate::account_profile::ProviderAccountMaterializationFile {
            relative_path: "auth.json".to_string(),
            contents_base64: "bmV2ZXItbG9nLXRoaXM=".to_string(),
        });
    assert!(!format!("{materialization:?}").contains("bmV2ZXItbG9nLXRoaXM"));
}

#[test]
fn remote_provider_launch_credential_peer_shape_is_versioned_and_debug_redacted() {
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        43
    );
    let request = RelayPeerRequest::SubmitLeasedPrompt {
        leased_agent_id: "leased-agent-1".to_string(),
        prompt: "continue".to_string(),
        hidden_system_context: String::new(),
        attachments: Vec::new(),
        workflow_context: None,
        git_context: None,
        required_mcps: Vec::new(),
        required_skills: None,
        remote_extension_manifest: Default::default(),
        provider_launch_credential: Some(
            crate::transport::relay_peer::RemoteProviderLaunchCredential {
                provider: "claude".to_string(),
                account_profile: "work".to_string(),
                secret_input: crate::transport::relay_peer::RemoteCredentialSecretInput::new(
                    "setup-token-secret".to_string(),
                ),
            },
        ),
    };
    let debug = format!("{request:?}");
    assert!(!debug.contains("setup-token-secret"));
    let mut value = serde_json::to_value(&request).expect("request should serialize");
    assert_eq!(
        value.pointer("/provider_launch_credential/provider"),
        Some(&serde_json::json!("claude"))
    );
    assert_eq!(
        value.pointer("/provider_launch_credential/account_profile"),
        Some(&serde_json::json!("work"))
    );
    assert_eq!(
        value.pointer("/provider_launch_credential/secret_input"),
        Some(&serde_json::json!("setup-token-secret"))
    );

    value
        .as_object_mut()
        .expect("request object")
        .remove("provider_launch_credential");
    let legacy_shape: RelayPeerRequest =
        serde_json::from_value(value).expect("missing optional credential stays compatible");
    assert!(matches!(
        legacy_shape,
        RelayPeerRequest::SubmitLeasedPrompt {
            provider_launch_credential: None,
            ..
        }
    ));
}

#[test]
fn managed_context_peer_shape_is_versioned_and_debug_redacts_bearer_material() {
    use crate::transport::relay_peer::{
        RelayManagedContextCapability, RelayManagedContextChunk, RelayManagedContextImportReceipt,
        RelayManagedContextImportedRepository, RelayManagedContextTransferPhase,
        RelayManagedContextTransferStatus, RelayManagedKernelContextImportReceipt,
    };

    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        43
    );
    let request = RelayPeerRequest::UploadManagedContextChunk {
        transfer_id: "ctx_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        capability: RelayManagedContextCapability::new("capability-canary".to_string()),
        offset: 0,
        data_base64: RelayManagedContextChunk::new("chunk-canary".to_string()),
        chunk_sha256: "a".repeat(64),
    };
    let debug = format!("{request:?}");
    assert!(!debug.contains("capability-canary"));
    assert!(!debug.contains("chunk-canary"));
    let value = serde_json::to_value(&request).expect("managed context request should serialize");
    assert_eq!(
        value.pointer("/kind"),
        Some(&serde_json::json!("upload_managed_context_chunk"))
    );
    assert_eq!(
        value.pointer("/capability"),
        Some(&serde_json::json!("capability-canary"))
    );
    let arm = serde_json::to_value(RelayPeerRequest::ArmManagedContextImport {
        context_id: "context-1".to_string(),
        plan_digest: format!("sha256:{}", "f".repeat(64)),
        target_environment_id: "environment-1".to_string(),
        target_kernel_id: "target-kernel-1".to_string(),
        target_key_thumbprint: "d".repeat(64),
        capability: RelayManagedContextCapability::new("arm-capability-canary".to_string()),
        archive_sha256: "e".repeat(64),
        archive_size_bytes: 42,
    })
    .expect("managed context arm should serialize");
    assert_eq!(
        arm.pointer("/context_id"),
        Some(&serde_json::json!("context-1"))
    );

    let response = RelayPeerResponse::ManagedContextImportStatus {
        status: RelayManagedContextTransferStatus {
            transfer_id: "ctx_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            phase: RelayManagedContextTransferPhase::Consumed,
            accepted_bytes: 42,
            archive_size_bytes: 42,
            expires_at_ms: 1_000,
            receipt: Some(RelayManagedContextImportReceipt {
                transfer_id: "ctx_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                archive_sha256: "a".repeat(64),
                plan_digest: format!("sha256:{}", "f".repeat(64)),
                development: crate::transport::relay_peer::RelayManagedDevelopmentContextImportReceipt::FromSource {
                    project_id: "project-1".to_string(),
                    destination_root: "/managed/context".to_string(),
                    primary_repository_id: "repo-primary".to_string(),
                    repositories: vec![RelayManagedContextImportedRepository {
                        repository_id: "repo-primary".to_string(),
                        role: crate::managed_context::development::DevelopmentRepositoryRole::Primary,
                        target_directory: "primary".to_string(),
                        destination_path: "/managed/context/primary".to_string(),
                        head_sha: "b".repeat(40),
                    }],
                },
                kernel_context: RelayManagedKernelContextImportReceipt::Empty,
                receipt_sha256: "c".repeat(64),
            }),
        },
    };
    let value = serde_json::to_value(&response).expect("managed context response should serialize");
    assert_eq!(
        value.pointer("/status/phase"),
        Some(&serde_json::json!("consumed"))
    );
    assert_eq!(
        value.pointer("/status/receipt/development/repositories/0/role"),
        Some(&serde_json::json!("primary"))
    );
    assert_eq!(
        value.pointer("/status/receipt/kernel_context/kind"),
        Some(&serde_json::json!("empty"))
    );
    let failure = serde_json::to_value(RelayPeerResponse::ManagedContextImportFailed {
        code: "invalid_managed_context".to_string(),
        retryable: false,
    })
    .expect("managed context failure should serialize");
    assert_eq!(
        failure.pointer("/kind"),
        Some(&serde_json::json!("managed_context_import_failed"))
    );
    assert_eq!(failure.pointer("/message"), None);
}

#[test]
fn execution_lease_created_response_advertises_relay_peer_protocol_version() {
    let lease = crate::execution_lease::ExecutionLease {
        id: "lease-1".to_string(),
        home_kernel_id: "home".to_string(),
        home_session_id: "session-1".to_string(),
        home_agent_id: "agent-1".to_string(),
        home_agent_metaagent: false,
        owner_user_id: "user-1".to_string(),
        worker_kernel_id: "worker".to_string(),
        machine_id: "machine-worker".to_string(),
        created_at_ms: 1,
        last_heartbeat_at_ms: 1,
    };
    let response = RelayPeerResponse::ExecutionLeaseCreated {
        lease: lease.clone(),
        relay_peer_protocol_version: crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
    };

    let value = serde_json::to_value(&response).expect("response should serialize");
    assert_eq!(
        value.pointer("/relay_peer_protocol_version"),
        Some(&serde_json::json!(
            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
        ))
    );

    let legacy = serde_json::json!({
        "kind": "execution_lease_created",
        "lease": lease,
    });
    let decoded: RelayPeerResponse =
        serde_json::from_value(legacy).expect("legacy response should decode with version 0");
    assert!(matches!(
        decoded,
        RelayPeerResponse::ExecutionLeaseCreated {
            relay_peer_protocol_version: 0,
            ..
        }
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn proxied_peer_requests_are_handled_through_relay() {
    let _relay_test_guard = relay_client_test_guard().await;
    let server = RelayServer::new(RelayConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        shared_token: Some("secret".to_string()),
    });
    let listener = server
        .bind_listener()
        .await
        .expect("relay listener should bind");
    let addr = listener.local_addr().expect("listener should have addr");
    let server = Arc::new(RelayServer::new(RelayConfig {
        host: addr.ip().to_string(),
        port: addr.port(),
        shared_token: Some("secret".to_string()),
    }));
    let registry = server.registry();
    let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
    let server_task = {
        let server = Arc::clone(&server);
        tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = server_shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        })
    };

    let mut config_a = DaemonConfig::for_tests();
    config_a.daemon_id = "daemon-a".to_string();
    config_a.daemon_alias = Some("alpha".to_string());
    config_a.host_machine_id = "machine-a".to_string();
    config_a.host_machine_alias = Some("machine-alpha".to_string());
    config_a.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_a.relay_token = Some("secret".to_string());
    config_a.relay_heartbeat_ms = 50;
    let app_a = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_a.clone()).expect("daemon A should bootstrap"),
    ));
    let state_a = Arc::new(RwLock::new(RelayClientState::default()));
    let (shutdown_a_tx, shutdown_a_rx) = watch::channel(false);
    let connector_a = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_a),
        Arc::clone(&state_a),
        shutdown_a_rx,
    ));

    let mut config_b = DaemonConfig::for_tests();
    config_b.daemon_id = "daemon-b".to_string();
    config_b.daemon_alias = Some("beta".to_string());
    config_b.host_machine_id = "machine-b".to_string();
    config_b.host_machine_alias = Some("machine-beta".to_string());
    config_b.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_b.relay_token = Some("secret".to_string());
    config_b.relay_heartbeat_ms = 50;
    let app_b = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_b.clone()).expect("daemon B should bootstrap"),
    ));
    let state_b = Arc::new(RwLock::new(RelayClientState::default()));
    let (shutdown_b_tx, shutdown_b_rx) = watch::channel(false);
    let connector_b = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_b),
        Arc::clone(&state_b),
        shutdown_b_rx,
    ));

    wait_for_daemon_registration(registry.clone(), &config_a.daemon_id).await;
    wait_for_daemon_registration(registry.clone(), &config_b.daemon_id).await;

    let kernel = relay_discovery::get_live_kernel(&config_a, "beta")
        .await
        .expect("live kernel lookup should succeed");
    assert_eq!(kernel.kernel_id, config_b.daemon_id);
    assert_eq!(kernel.public_key, config_b.relay_public_key);

    let response = send_peer_request_via_relay(
        &app_a,
        &state_a,
        ClientTarget {
            daemon_id: None,
            daemon_alias: Some("beta".to_string()),
        },
        RelayPeerRequest::Ping {
            value: "hello-remote-kernel".to_string(),
        },
    )
    .await
    .expect("peer request should succeed");
    assert_eq!(
        response,
        RelayPeerResponse::Pong {
            value: "hello-remote-kernel".to_string(),
            daemon_id: config_b.daemon_id.clone(),
        }
    );

    let missing_lease = send_peer_request_via_relay(
        &app_a,
        &state_a,
        ClientTarget {
            daemon_id: None,
            daemon_alias: Some("beta".to_string()),
        },
        RelayPeerRequest::SubmitLeasedPrompt {
            leased_agent_id: "missing-leased-agent".to_string(),
            prompt: "continue".to_string(),
            hidden_system_context: String::new(),
            attachments: Vec::new(),
            workflow_context: None,
            git_context: None,
            required_mcps: Vec::new(),
            required_skills: None,
            remote_extension_manifest: Default::default(),
            provider_launch_credential: Some(
                crate::transport::relay_peer::RemoteProviderLaunchCredential {
                    provider: "claude".to_string(),
                    account_profile: "work".to_string(),
                    secret_input: crate::transport::relay_peer::RemoteCredentialSecretInput::new(
                        "relay-secret-canary".to_string(),
                    ),
                },
            ),
        },
    )
    .await
    .expect_err("the worker should reject an unknown leased agent");
    let diagnostic = missing_lease.to_string();
    assert!(diagnostic.contains("leased agent"));
    assert!(!diagnostic.contains("relay-secret-canary"));

    let _ = shutdown_a_tx.send(true);
    let _ = shutdown_b_tx.send(true);
    connector_a.await.expect("connector A should join");
    connector_b.await.expect("connector B should join");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
}

#[tokio::test(flavor = "multi_thread")]
async fn workspace_live_sync_changes_apply_to_linked_peer_worktree_through_relay() {
    let _relay_test_guard = relay_client_test_guard().await;
    let test_root = std::env::temp_dir().join(format!(
        "chariox-relay-workspace-live-sync-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after Unix epoch")
            .as_millis()
    ));
    let source_root = test_root.join("source");
    let target_root = test_root.join("target");
    let target_src = target_root.join("src");
    std::fs::create_dir_all(&source_root).expect("source root should be created");
    std::fs::create_dir_all(&target_src).expect("target src should be created");
    std::fs::write(target_src.join("lib.rs"), "old\n").expect("target file should be seeded");

    let server = RelayServer::new(RelayConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        shared_token: Some("secret".to_string()),
    });
    let listener = server
        .bind_listener()
        .await
        .expect("relay listener should bind");
    let addr = listener.local_addr().expect("listener should have addr");
    let server = Arc::new(RelayServer::new(RelayConfig {
        host: addr.ip().to_string(),
        port: addr.port(),
        shared_token: Some("secret".to_string()),
    }));
    let registry = server.registry();
    let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
    let server_task = {
        let server = Arc::clone(&server);
        tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = server_shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        })
    };

    let mut config_home = DaemonConfig::for_tests();
    config_home.daemon_id = "daemon-home".to_string();
    config_home.daemon_alias = Some("home".to_string());
    config_home.host_machine_id = "machine-home".to_string();
    config_home.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_home.relay_token = Some("secret".to_string());
    config_home.relay_heartbeat_ms = 50;
    let app_home = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_home.clone()).expect("home daemon should bootstrap"),
    ));
    let state_home = Arc::new(RwLock::new(RelayClientState::default()));
    let (shutdown_home_tx, shutdown_home_rx) = watch::channel(false);
    let connector_home = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_home),
        Arc::clone(&state_home),
        shutdown_home_rx,
    ));

    let mut config_worker = DaemonConfig::for_tests();
    config_worker.daemon_id = "daemon-worker".to_string();
    config_worker.daemon_alias = Some("worker".to_string());
    config_worker.host_machine_id = "machine-worker".to_string();
    config_worker.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_worker.relay_token = Some("secret".to_string());
    config_worker.relay_heartbeat_ms = 50;
    let app_worker = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_worker.clone()).expect("worker daemon should bootstrap"),
    ));
    let state_worker = Arc::new(RwLock::new(RelayClientState::default()));
    let (shutdown_worker_tx, shutdown_worker_rx) = watch::channel(false);
    let connector_worker = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_worker),
        Arc::clone(&state_worker),
        shutdown_worker_rx,
    ));

    wait_for_daemon_registration(registry.clone(), &config_home.daemon_id).await;
    wait_for_daemon_registration(registry.clone(), &config_worker.daemon_id).await;

    {
        let mut app = app_worker.lock().await;
        let session_id = create_test_session(
            &mut app,
            target_root.to_string_lossy().as_ref(),
            target_root.to_string_lossy().as_ref(),
        );
        let (_, link) = app
            .sessions_mut()
            .create_workspace_link(&session_id, "team-sync".to_string(), "user-2".to_string())
            .expect("worker link should be created");
        app.sessions_mut()
            .attach_workspace_link(
                &session_id,
                link.link_id(),
                "user-2".to_string(),
                config_worker.host_machine_id.clone(),
                config_worker.daemon_id.clone(),
                target_root.to_string_lossy().to_string(),
                None,
                None,
            )
            .expect("worker target should attach to workspace link");
    }

    let change = crate::git_observer::WorkspaceLiveSyncChange {
        session_id: "home-session-1".to_string(),
        agent_id: "agent-1".to_string(),
        provider_run_id: "provider-run-1".to_string(),
        prompt_id: "prompt-1".to_string(),
        repo_root: source_root.to_string_lossy().to_string(),
        worktree_path: source_root.to_string_lossy().to_string(),
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
        status_fingerprint: "tracked_workspace_live_sync".to_string(),
    };
    let context = crate::transport::relay_peer::RemoteWorkspaceLiveSyncApplyContext {
        home_session_id: "home-session-1".to_string(),
        link_id: "workspace-link-1".to_string(),
        link_name: "team-sync".to_string(),
        source_agent_id: "agent-1".to_string(),
        source_worktree_path: source_root.to_string_lossy().to_string(),
        target_user_id: "user-2".to_string(),
        target_machine_id: config_worker.host_machine_id.clone(),
        target_kernel_id: config_worker.daemon_id.clone(),
        target_repo_root: target_root.to_string_lossy().to_string(),
    };

    let response = send_peer_request_via_relay(
        &app_home,
        &state_home,
        ClientTarget {
            daemon_id: Some(config_worker.daemon_id.clone()),
            daemon_alias: None,
        },
        RelayPeerRequest::ApplyWorkspaceLiveSyncChange { context, change },
    )
    .await
    .expect("workspace live sync apply should cross relay");
    let target_result = match response {
        RelayPeerResponse::WorkspaceLiveSyncChangeApplied { target_result } => target_result,
        other => panic!("unexpected peer response: {other:?}"),
    };
    assert_eq!(target_result.target_user_id, "user-2");
    assert_eq!(target_result.target_kernel_id, config_worker.daemon_id);
    assert_eq!(target_result.path_results.len(), 1);
    assert_eq!(
        target_result.path_results[0].status,
        crate::git_observer::WorkspaceLiveSyncApplyStatus::Applied
    );
    assert_eq!(
        std::fs::read_to_string(target_src.join("lib.rs")).expect("target file should be readable"),
        "new\n"
    );

    let _ = shutdown_home_tx.send(true);
    let _ = shutdown_worker_tx.send(true);
    connector_home.await.expect("home connector should join");
    connector_worker
        .await
        .expect("worker connector should join");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
    std::fs::remove_dir_all(&test_root).expect("test root should be removed");
}

#[tokio::test(flavor = "multi_thread")]
async fn execution_leases_are_managed_through_peer_transport() {
    let _relay_test_guard = relay_client_test_guard().await;
    let server = RelayServer::new(RelayConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        shared_token: Some("secret".to_string()),
    });
    let listener = server
        .bind_listener()
        .await
        .expect("relay listener should bind");
    let addr = listener.local_addr().expect("listener should have addr");
    let server = Arc::new(RelayServer::new(RelayConfig {
        host: addr.ip().to_string(),
        port: addr.port(),
        shared_token: Some("secret".to_string()),
    }));
    let registry = server.registry();
    let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
    let server_task = {
        let server = Arc::clone(&server);
        tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = server_shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        })
    };

    let mut config_a = DaemonConfig::for_tests();
    config_a.daemon_id = "daemon-home".to_string();
    config_a.daemon_alias = Some("home".to_string());
    config_a.host_machine_id = "machine-home".to_string();
    config_a.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_a.relay_token = Some("secret".to_string());
    config_a.relay_heartbeat_ms = 50;
    let app_a = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_a.clone()).expect("home daemon should bootstrap"),
    ));
    let state_a = Arc::new(RwLock::new(RelayClientState::default()));
    let (shutdown_a_tx, shutdown_a_rx) = watch::channel(false);
    let connector_a = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_a),
        Arc::clone(&state_a),
        shutdown_a_rx,
    ));

    let mut config_b = DaemonConfig::for_tests();
    config_b.daemon_id = "daemon-worker".to_string();
    config_b.daemon_alias = Some("worker".to_string());
    config_b.host_machine_id = "machine-worker".to_string();
    config_b.host_machine_alias = Some("remote-builder".to_string());
    config_b.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_b.relay_token = Some("secret".to_string());
    config_b.relay_heartbeat_ms = 50;
    config_b.accept_remote_leases = true;
    let app_b = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_b.clone()).expect("worker daemon should bootstrap"),
    ));
    let state_b = Arc::new(RwLock::new(RelayClientState::default()));
    let (shutdown_b_tx, shutdown_b_rx) = watch::channel(false);
    let connector_b = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_b),
        Arc::clone(&state_b),
        shutdown_b_rx,
    ));

    wait_for_daemon_registration(registry.clone(), &config_a.daemon_id).await;
    wait_for_daemon_registration(registry.clone(), &config_b.daemon_id).await;

    let created = send_peer_request_via_relay(
        &app_a,
        &state_a,
        ClientTarget {
            daemon_id: None,
            daemon_alias: Some("worker".to_string()),
        },
        RelayPeerRequest::CreateExecutionLease {
            home_kernel_id: config_a.daemon_id.clone(),
            home_session_id: "session-remote-1".to_string(),
            home_agent_id: "agent-remote-1".to_string(),
            home_agent_metaagent: false,
            owner_user_id: "user-home".to_string(),
        },
    )
    .await
    .expect("execution lease should be created remotely");
    let lease = match created {
        RelayPeerResponse::ExecutionLeaseCreated {
            lease,
            relay_peer_protocol_version,
        } => {
            assert_eq!(
                relay_peer_protocol_version,
                crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
            );
            lease
        }
        other => panic!("unexpected peer response: {other:?}"),
    };
    assert_eq!(lease.home_kernel_id, config_a.daemon_id);
    assert_eq!(lease.worker_kernel_id, config_b.daemon_id);
    assert_eq!(lease.machine_id, config_b.host_machine_id);
    {
        let mut app = app_b.lock().await;
        assert_eq!(RemoteLeaseRuntime::new(&mut app).execution_lease_count(), 1);
    }

    let destroyed = send_peer_request_via_relay(
        &app_a,
        &state_a,
        ClientTarget {
            daemon_id: Some(config_b.daemon_id.clone()),
            daemon_alias: None,
        },
        RelayPeerRequest::DestroyExecutionLease {
            lease_id: lease.id.clone(),
        },
    )
    .await
    .expect("execution lease should be destroyed remotely");
    assert_eq!(
        destroyed,
        RelayPeerResponse::ExecutionLeaseDestroyed {
            lease_id: lease.id.clone(),
        }
    );
    {
        let mut app = app_b.lock().await;
        assert_eq!(RemoteLeaseRuntime::new(&mut app).execution_lease_count(), 0);
    }

    let _ = shutdown_a_tx.send(true);
    let _ = shutdown_b_tx.send(true);
    connector_a.await.expect("connector A should join");
    connector_b.await.expect("connector B should join");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
}
#[tokio::test(flavor = "multi_thread")]
async fn leased_agents_are_spawned_and_destroyed_through_peer_transport() {
    let _relay_test_guard = relay_client_test_guard().await;
    let server = RelayServer::new(RelayConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        shared_token: Some("secret".to_string()),
    });
    let listener = server
        .bind_listener()
        .await
        .expect("relay listener should bind");
    let addr = listener.local_addr().expect("listener should have addr");
    let server = Arc::new(RelayServer::new(RelayConfig {
        host: addr.ip().to_string(),
        port: addr.port(),
        shared_token: Some("secret".to_string()),
    }));
    let registry = server.registry();
    let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
    let server_task = {
        let server = Arc::clone(&server);
        tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = server_shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        })
    };

    let mut config_a = DaemonConfig::for_tests();
    config_a.daemon_id = "daemon-home".to_string();
    config_a.daemon_alias = Some("home".to_string());
    config_a.host_machine_id = "machine-home".to_string();
    config_a.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_a.relay_token = Some("secret".to_string());
    config_a.relay_heartbeat_ms = 50;
    let app_a = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_a.clone()).expect("home daemon should bootstrap"),
    ));
    let (home_session_id, home_agent_id) = {
        let mut app = app_a.lock().await;
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-home", "worktree-home"))
            .expect("home session should be created");
        (session.id().to_string(), agent.id().to_string())
    };
    let state_a = Arc::new(RwLock::new(RelayClientState::default()));
    let (shutdown_a_tx, shutdown_a_rx) = watch::channel(false);
    let connector_a = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_a),
        Arc::clone(&state_a),
        shutdown_a_rx,
    ));

    let mut config_b = DaemonConfig::for_tests();
    config_b.daemon_id = "daemon-worker".to_string();
    config_b.daemon_alias = Some("worker".to_string());
    config_b.host_machine_id = "machine-worker".to_string();
    config_b.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_b.relay_token = Some("secret".to_string());
    config_b.relay_heartbeat_ms = 50;
    config_b.accept_remote_leases = true;
    let app_b = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_b.clone()).expect("worker daemon should bootstrap"),
    ));
    let state_b = Arc::new(RwLock::new(RelayClientState::default()));
    let (shutdown_b_tx, shutdown_b_rx) = watch::channel(false);
    let connector_b = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_b),
        Arc::clone(&state_b),
        shutdown_b_rx,
    ));

    wait_for_daemon_registration(registry.clone(), &config_a.daemon_id).await;
    wait_for_daemon_registration(registry.clone(), &config_b.daemon_id).await;

    let lease = match send_peer_request_via_relay(
        &app_a,
        &state_a,
        ClientTarget {
            daemon_id: Some(config_b.daemon_id.clone()),
            daemon_alias: None,
        },
        RelayPeerRequest::CreateExecutionLease {
            home_kernel_id: config_a.daemon_id.clone(),
            home_session_id: home_session_id.clone(),
            home_agent_id: home_agent_id.clone(),
            home_agent_metaagent: false,
            owner_user_id: "user-home".to_string(),
        },
    )
    .await
    .expect("execution lease should be created remotely")
    {
        RelayPeerResponse::ExecutionLeaseCreated {
            lease,
            relay_peer_protocol_version,
        } => {
            assert_eq!(
                relay_peer_protocol_version,
                crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
            );
            lease
        }
        other => panic!("unexpected peer response: {other:?}"),
    };

    let leased_agent = match send_peer_request_via_relay(
        &app_a,
        &state_a,
        ClientTarget {
            daemon_id: None,
            daemon_alias: Some("worker".to_string()),
        },
        RelayPeerRequest::SpawnLeasedAgent {
            lease_id: lease.id.clone(),
            provider: "opencode".to_string(),
            account_profile: "work".to_string(),
            model: Some("kimi2.5".to_string()),
            effort: Some("medium".to_string()),
            execution_mode: None,
            permission_level: None,
            workspace_live_sync_mode: Some(crate::config::WorkspaceLiveSyncMode::Tracked),
            worktree_id: None,
            worktree_placement: None,
        },
    )
    .await
    .expect("leased agent should be spawned remotely")
    {
        RelayPeerResponse::LeasedAgentSpawned { leased_agent } => leased_agent,
        other => panic!("unexpected peer response: {other:?}"),
    };
    assert_eq!(leased_agent.lease_id, lease.id);
    assert_eq!(leased_agent.provider, "opencode");
    {
        let mut app = app_b.lock().await;
        assert_eq!(RemoteLeaseRuntime::new(&mut app).leased_agent_count(), 1);
    }

    let destroyed = send_peer_request_via_relay(
        &app_a,
        &state_a,
        ClientTarget {
            daemon_id: Some(config_b.daemon_id.clone()),
            daemon_alias: None,
        },
        RelayPeerRequest::DestroyLeasedAgent {
            leased_agent_id: leased_agent.id.clone(),
        },
    )
    .await
    .expect("leased agent should be destroyed remotely");
    assert_eq!(
        destroyed,
        RelayPeerResponse::LeasedAgentDestroyed {
            leased_agent_id: leased_agent.id.clone(),
        }
    );
    {
        let mut app = app_b.lock().await;
        assert_eq!(RemoteLeaseRuntime::new(&mut app).leased_agent_count(), 0);
    }

    let _ = shutdown_a_tx.send(true);
    let _ = shutdown_b_tx.send(true);
    connector_a.await.expect("connector A should join");
    connector_b.await.expect("connector B should join");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
}
