#![allow(unused_imports)]
use super::support::*;

#[test]
fn agents_can_be_spawned_on_a_remote_machine_and_cleaned_up() {
    run_async_with_large_test_stack(
        "remote-agents-spawn-resize-cleanup",
        agents_can_be_spawned_on_a_remote_machine_and_cleaned_up_async,
    );
}

async fn agents_can_be_spawned_on_a_remote_machine_and_cleaned_up_async() {
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
    let state_home = {
        let app = app_home.lock().await;
        app.relay_client_state()
    };
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
    config_worker.host_machine_alias = Some("builder-west".to_string());
    config_worker.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_worker.relay_token = Some("secret".to_string());
    config_worker.relay_heartbeat_ms = 50;
    config_worker.accept_remote_leases = true;
    let app_worker = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_worker.clone()).expect("worker daemon should bootstrap"),
    ));
    let state_worker = {
        let app = app_worker.lock().await;
        app.relay_client_state()
    };
    let (shutdown_worker_tx, shutdown_worker_rx) = watch::channel(false);
    let connector_worker = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_worker),
        Arc::clone(&state_worker),
        shutdown_worker_rx,
    ));

    wait_for_daemon_registration(registry.clone(), &config_home.daemon_id).await;
    wait_for_daemon_registration(registry.clone(), &config_worker.daemon_id).await;

    let worker_kernels =
        relay_discovery::list_live_kernels_for_machine(&config_home, "builder-west")
            .await
            .expect("worker kernels should be discoverable");
    let provider = worker_kernels
        .first()
        .and_then(|kernel| {
            kernel
                .available_providers
                .iter()
                .find(|provider| provider.as_str() == "managed-dev-stub")
        })
        .cloned()
        .expect("worker should advertise managed-dev-stub");
    refresh_remote_inventory_projection_for_app_with_relay_state(&app_home)
        .await
        .expect("home remote inventory should refresh");

    let session_id = {
        let mut app = app_home.lock().await;
        let (session, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-home", "worktree-home"))
            .expect("home session should be created");
        session.id().to_string()
    };

    let remote_agent = {
        let mut app = app_home.lock().await;
        crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(&session_id, &provider)
                    .with_alias("remote-reviewer")
                    .with_model("default")
                    .with_effort("medium")
                    .with_kernel(&config_worker.daemon_id),
            )
            .expect("remote agent should spawn")
    };

    let remote_execution = remote_agent
        .remote_execution()
        .cloned()
        .expect("remote binding should be present");
    assert_eq!(remote_execution.worker_kernel_id, config_worker.daemon_id);
    assert_eq!(
        remote_execution.worker_machine_id,
        config_worker.host_machine_id
    );

    {
        let mut app = app_worker.lock().await;
        assert_eq!(RemoteLeaseRuntime::new(&mut app).execution_lease_count(), 1);
        assert_eq!(RemoteLeaseRuntime::new(&mut app).leased_agent_count(), 1);
        let worker_agents = app.agents().list_agents();
        assert_eq!(
            worker_agents
                .iter()
                .filter(|agent| agent.is_metaagent())
                .count(),
            0,
            "remote agents start regular until /meta activates temporary meta mode"
        );
    }

    let response = crate::transport::relay_client::send_peer_request_via_temporary_connection(
        &config_home,
        ClientTarget {
            daemon_id: Some(config_worker.daemon_id.clone()),
            daemon_alias: None,
        },
        RelayPeerRequest::UpdateLeasedAgentMetaMode {
            leased_agent_id: remote_execution.leased_agent_id.clone(),
            active: true,
        },
    )
    .await
    .expect("remote meta mode activation should be sent");
    assert!(matches!(
        response,
        RelayPeerResponse::LeasedAgentMetaModeUpdated { .. }
    ));

    {
        let app = app_worker.lock().await;
        let worker_agents = app.agents().list_agents();
        assert_eq!(
            worker_agents
                .iter()
                .filter(|agent| agent.is_metaagent())
                .count(),
            1,
            "remote meta mode update should activate the backing agent"
        );
    }

    let response = crate::transport::relay_client::send_peer_request_via_temporary_connection(
        &config_home,
        ClientTarget {
            daemon_id: Some(config_worker.daemon_id.clone()),
            daemon_alias: None,
        },
        RelayPeerRequest::UpdateLeasedAgentMetaMode {
            leased_agent_id: remote_execution.leased_agent_id.clone(),
            active: false,
        },
    )
    .await
    .expect("remote meta mode deactivation should be sent");
    assert!(matches!(
        response,
        RelayPeerResponse::LeasedAgentMetaModeUpdated { .. }
    ));

    {
        let app = app_worker.lock().await;
        let worker_agents = app.agents().list_agents();
        assert_eq!(
            worker_agents
                .iter()
                .filter(|agent| agent.is_metaagent())
                .count(),
            0,
            "remote meta mode deactivation should restore the backing agent"
        );
    }

    Box::pin(assert_remote_native_terminal_resize(
        &app_home,
        &app_worker,
        &session_id,
        &provider,
        &remote_agent,
    ))
    .await;

    {
        let mut app = app_home.lock().await;
        let destroyed = crate::app::KernelSessionService::new(&mut app)
            .destroy_agent(remote_agent.id())
            .expect("remote agent should destroy");
        assert_eq!(destroyed.id(), remote_agent.id());
    }

    {
        let mut app = app_worker.lock().await;
        assert_eq!(RemoteLeaseRuntime::new(&mut app).execution_lease_count(), 0);
        assert_eq!(RemoteLeaseRuntime::new(&mut app).leased_agent_count(), 0);
    }

    let _ = shutdown_home_tx.send(true);
    let _ = shutdown_worker_tx.send(true);
    connector_home.await.expect("home connector should join");
    connector_worker
        .await
        .expect("worker connector should join");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
}

async fn assert_remote_native_terminal_resize(
    app_home: &Arc<Mutex<DaemonApp>>,
    app_worker: &Arc<Mutex<DaemonApp>>,
    session_id: &str,
    provider: &str,
    remote_agent: &crate::agent::AgentInstance,
) {
    let home_router =
        crate::runtime::router::CommandRouter::with_interactive_capacity(Arc::clone(app_home), 8);
    let launch_request = LocalDaemonRequest::LaunchProviderRun(LaunchProviderRunRequest {
        session_id: session_id.to_string(),
        agent_id: Some(remote_agent.id().to_string()),
        adapter_key: crate::provider::adapter_key_for_provider(provider).to_string(),
        provider: provider.to_string(),
        account_profile: "default".to_string(),
        model: "default".to_string(),
        variant: None,
        structured_endpoint: None,
        provider_session_id: None,
        native_tui: true,
    });
    let launch_command = KernelCommand::from_local_request(
        "launch-remote-native-resize",
        None,
        None,
        &launch_request,
    );
    let home_provider_run = match home_router
        .dispatch(launch_command, launch_request)
        .await
        .expect("remote native provider should launch")
    {
        LocalDaemonResponse::ProviderRunLaunched { provider_run } => provider_run,
        other => panic!("unexpected provider launch response: {other:?}"),
    };
    let worker_provider_run_id = {
        let app = app_home.lock().await;
        app.agents()
            .get_agent(remote_agent.id())
            .expect("remote agent should remain available")
            .remote_execution()
            .and_then(|binding| binding.active_worker_provider_run_id.clone())
            .expect("worker provider run should be projected")
    };
    let resize_request = LocalDaemonRequest::ResizeTerminal(ResizeTerminalRequest {
        session_id: session_id.to_string(),
        provider_run_id: Some(home_provider_run.id().to_string()),
        cols: 83,
        rows: 27,
    });
    let resize_command =
        KernelCommand::from_local_request("resize-remote-native", None, None, &resize_request);
    assert!(matches!(
        home_router
            .dispatch(resize_command, resize_request)
            .await
            .expect("remote native provider terminal should resize"),
        LocalDaemonResponse::TerminalResized {
            cols: 83,
            rows: 27,
            ..
        }
    ));
    let app = app_worker.lock().await;
    assert_eq!(app.pty().size(&worker_provider_run_id), Some((83, 27)));
}

#[test]
fn remote_machine_agents_execute_prompts_through_the_home_session() {
    run_async_with_large_test_stack(
        "remote-agents-execute-prompts",
        remote_machine_agents_execute_prompts_through_the_home_session_async,
    );
}

async fn remote_machine_agents_execute_prompts_through_the_home_session_async() {
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

    let mut config_home = DaemonConfig::for_tests();
    config_home.daemon_id = "daemon-home".to_string();
    config_home.daemon_alias = Some("home".to_string());
    config_home.host_machine_id = "machine-home".to_string();
    config_home.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_home.relay_token = Some("secret".to_string());
    config_home.relay_heartbeat_ms = 50;
    let mut config_worker = DaemonConfig::for_tests();
    config_worker.daemon_id = "daemon-worker".to_string();
    config_worker.daemon_alias = Some("worker".to_string());
    config_worker.host_machine_id = "machine-worker".to_string();
    config_worker.host_machine_alias = Some("builder-west".to_string());
    config_worker.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_worker.relay_token = Some("secret".to_string());
    config_worker.relay_heartbeat_ms = 50;
    config_worker.accept_remote_leases = true;
    let app_worker = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_worker.clone()).expect("worker daemon should bootstrap"),
    ));
    let state_worker = {
        let app = app_worker.lock().await;
        app.relay_client_state()
    };
    let (shutdown_worker_tx, shutdown_worker_rx) = watch::channel(false);
    let connector_worker = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_worker),
        Arc::clone(&state_worker),
        shutdown_worker_rx,
    ));

    wait_for_daemon_registration(registry.clone(), &config_worker.daemon_id).await;

    let provider = relay_discovery::list_live_kernels_for_machine(&config_home, "builder-west")
        .await
        .expect("worker kernels should be discoverable")
        .first()
        .and_then(|kernel| {
            kernel
                .available_providers
                .iter()
                .find(|provider| provider.as_str() == "managed-dev-stub")
        })
        .cloned()
        .expect("worker should advertise managed-dev-stub");

    let app_home = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_home.clone()).expect("home daemon should bootstrap"),
    ));
    let state_home = {
        let app = app_home.lock().await;
        app.relay_client_state()
    };
    let (shutdown_home_tx, shutdown_home_rx) = watch::channel(false);
    let connector_home = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_home),
        Arc::clone(&state_home),
        shutdown_home_rx,
    ));
    wait_for_daemon_registration(registry.clone(), &config_home.daemon_id).await;
    refresh_remote_inventory_projection_for_app_with_relay_state(&app_home)
        .await
        .expect("home remote inventory should refresh");

    let (session_id, attachment_id, steering_attachment_id, local_provider_run_id) = {
        let mut app_home = app_home.lock().await;
        let (session, local_agent) = crate::app::KernelSessionService::new(&mut app_home)
            .create_session(CreateSessionRequest::new("workspace-home", "worktree-home"))
            .expect("home session should be created");
        let local_provider_run = app_home
            .launch_provider(
                crate::provider::LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "claude-code",
                    "default",
                    "native-tui-idle",
                )
                .with_agent_id(local_agent.id().to_string()),
            )
            .expect("home provider should launch to force a cross-kernel run-id collision");
        let attachment = crate::app::KernelSessionService::new(&mut app_home)
            .attach(AttachRequest::new(
                session.id(),
                "home-client",
                ClientCapabilityLevel::InteractiveStructured,
            ))
            .expect("home attachment should attach");
        let steering_attachment = crate::app::KernelSessionService::new(&mut app_home)
            .attach(AttachRequest::new(
                session.id(),
                "home-steering-client",
                ClientCapabilityLevel::InteractiveStructured,
            ))
            .expect("home steering attachment should attach");
        (
            session.id().to_string(),
            attachment.id().to_string(),
            steering_attachment.id().to_string(),
            local_provider_run.id().to_string(),
        )
    };

    let remote_agent_id = {
        let mut app_home = app_home.lock().await;
        crate::app::KernelSessionService::new(&mut app_home)
            .spawn_agent(
                CreateAgentRequest::new(&session_id, &provider)
                    .with_alias("remote-reviewer")
                    .with_model("native-tui-idle")
                    .with_effort("medium")
                    .with_kernel(&config_worker.daemon_id),
            )
            .expect("remote agent should spawn")
            .id()
            .to_string()
    };

    let leased_agent_id = app_home
        .lock()
        .await
        .agents()
        .get_agent(&remote_agent_id)
        .expect("remote agent should still exist")
        .remote_execution()
        .expect("remote binding should still exist")
        .leased_agent_id
        .clone();
    assert_eq!(
        state_worker
            .read()
            .await
            .peer_public_key(&config_home.daemon_id)
            .as_deref(),
        Some(config_home.relay_public_key.as_str()),
        "worker should retain the authenticated home key for projection events"
    );

    let _ = server_shutdown_tx.send(());
    server_task.await.expect("relay accept loop should stop");

    let router = Arc::new(
        crate::runtime::router::CommandRouter::with_interactive_capacity(Arc::clone(&app_home), 1),
    );
    let prompt_request = LocalDaemonRequest::SubmitPrompt(crate::local::SubmitPromptRequest {
        session_id: session_id.clone(),
        attachment_id: attachment_id.clone(),
        target_agent_id: Some(remote_agent_id.clone()),
        prompt: "remote prompt over home session\n".to_string(),
        attachments: Vec::new(),
    });
    let prompt_command = KernelCommand::from_local_request(
        "command-remote-persistent-prompt",
        None,
        None,
        &prompt_request,
    );
    let prompt_response = router
        .dispatch(prompt_command, prompt_request)
        .await
        .expect("remote prompt should submit while new relay connections are unavailable");
    assert!(matches!(
        prompt_response,
        LocalDaemonResponse::PromptSubmitted {
            outcome: crate::session::PromptSubmissionOutcome::Started { .. },
            ..
        }
    ));

    let mut worker_received_prompt = false;
    for _ in 0..1200 {
        worker_received_prompt = {
            let mut app = app_worker.lock().await;
            let leased_agent = RemoteLeaseRuntime::new(&mut app)
                .leased_agent_snapshot_for_test(&leased_agent_id)
                .expect("worker leased agent should remain available");
            leased_agent.active_home_prompt_id.as_deref() == Some("prompt-1")
                && app
                    .prompt_owner_queued_prompt_count_for_agent(
                        &leased_agent.backing_session_id,
                        &leased_agent.backing_agent_id,
                    )
                    .expect("worker queue count should load")
                    == 0
        };
        if worker_received_prompt {
            break;
        }
        sleep(Duration::from_millis(25)).await;
    }

    let listener = server
        .bind_listener()
        .await
        .expect("relay listener should restart");
    let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
    let server_task = {
        let server = Arc::clone(&server);
        tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = server_shutdown_rx.await;
                })
                .await
                .expect("relay server should resume accepting connections");
        })
    };
    assert!(
        worker_received_prompt,
        "remote prompt must reuse the persistent relay lane when new relay connections are unavailable"
    );

    let mut home_provider_running = false;
    for _ in 0..400 {
        home_provider_running = {
            let app = app_home.lock().await;
            let worker_acknowledged = app
                .agents()
                .get_agent(&remote_agent_id)
                .expect("remote agent should remain available")
                .remote_execution()
                .and_then(|remote| remote.active_worker_provider_run_id.as_deref())
                .is_some();
            let projected_running = app
                .provider_run_projection_store()
                .get_for_agent(&session_id, &remote_agent_id)
                .is_some_and(|run| run.state() == crate::provider::ProviderRunState::Running);
            worker_acknowledged && projected_running
        };
        if home_provider_running {
            break;
        }
        sleep(Duration::from_millis(25)).await;
    }
    let home_projection_debug = {
        let app = app_home.lock().await;
        (
            app.agents()
                .get_agent(&remote_agent_id)
                .ok()
                .and_then(|agent| {
                    agent
                        .remote_execution()
                        .and_then(|remote| remote.active_worker_provider_run_id.clone())
                }),
            app.provider_run_projection_store()
                .get_for_agent(&session_id, &remote_agent_id)
                .map(|run| (run.id().to_string(), run.state())),
        )
    };
    assert!(
        home_provider_running,
        "home agent must project the running worker provider run before follow-up steering: {home_projection_debug:?}"
    );
    let (worker_provider_run_id, projected_provider_run_id) = {
        let app = app_home.lock().await;
        let worker_provider_run_id = app
            .agents()
            .get_agent(&remote_agent_id)
            .expect("remote agent should remain available")
            .remote_execution()
            .and_then(|remote| remote.active_worker_provider_run_id.clone())
            .expect("worker provider run should remain projected");
        let projected_provider_run_id = app
            .provider_run_projection_store()
            .get_for_agent(&session_id, &remote_agent_id)
            .expect("remote projected provider run should exist")
            .id()
            .to_string();
        (worker_provider_run_id, projected_provider_run_id)
    };
    assert_eq!(
        worker_provider_run_id, local_provider_run_id,
        "the regression requires raw provider-run IDs to collide across kernels"
    );
    assert_ne!(projected_provider_run_id, local_provider_run_id);

    {
        let mut app = app_worker.lock().await;
        let leased_agent = RemoteLeaseRuntime::new(&mut app)
            .leased_agent_snapshot_for_test(&leased_agent_id)
            .expect("worker leased agent should exist");
        let backing_active = app
            .prompt_owner_active_prompt_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )
            .expect("worker active prompt should load");
        assert_eq!(
            leased_agent.active_home_prompt_id.as_deref(),
            Some("prompt-1"),
            "worker backing active prompt: {backing_active:?}"
        );
        assert_eq!(
            app.prompt_owner_queued_prompt_count_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )
            .expect("worker queue count should load"),
            0,
            "provider launch promotion must not leave a duplicate queued prompt"
        );
    }

    let queued_prompt_id = match app_home
        .lock()
        .await
        .submit_prompt(
            &session_id,
            &attachment_id,
            Some(&remote_agent_id),
            "REMOTE_QUEUE_STEER_DELIVERY\n",
            Vec::new(),
        )
        .expect("second remote prompt should queue")
    {
        crate::session::PromptSubmissionOutcome::Queued { prompt, .. } => prompt.id().to_string(),
        other => panic!("unexpected queued prompt outcome: {other:?}"),
    };
    let request = LocalDaemonRequest::SteerQueuedPrompt(crate::local::SteerQueuedPromptRequest {
        session_id: session_id.clone(),
        attachment_id: steering_attachment_id,
        target_agent_id: remote_agent_id.clone(),
        prompt_id: queued_prompt_id.clone(),
    });
    let command =
        KernelCommand::from_local_request("command-remote-queued-steer", None, None, &request);
    let response = router
        .dispatch(command, request)
        .await
        .expect("remote queued prompt should steer through the worker");
    let LocalDaemonResponse::QueuedPromptSteered {
        prompt, session, ..
    } = response
    else {
        panic!("unexpected remote queued prompt steer response");
    };
    assert_eq!(prompt.id(), queued_prompt_id);
    assert_eq!(
        session
            .active_prompt_for_agent(&remote_agent_id)
            .map(|prompt| prompt.prompt()),
        Some("remote prompt over home session\n")
    );
    assert!(session
        .queued_prompts_for_agent(&remote_agent_id)
        .is_none_or(|prompts| prompts.is_empty()));
    let worker_steer_deliveries = app_worker
        .lock()
        .await
        .terminal()
        .input_records()
        .into_iter()
        .filter(|record| {
            String::from_utf8_lossy(&record.bytes).contains("REMOTE_QUEUE_STEER_DELIVERY")
        })
        .count();
    assert_eq!(worker_steer_deliveries, 1);
    let browser_steering_echo = app_home
        .lock()
        .await
        .terminal_mut()
        .drain_output_records(&session_id, &attachment_id)
        .into_iter()
        .find(|record| {
            String::from_utf8_lossy(&record.bytes).contains("REMOTE_QUEUE_STEER_DELIVERY")
        })
        .expect("queued steering should echo to the prompt source attachment");
    assert_eq!(
        browser_steering_echo.agent_id.as_deref(),
        Some(remote_agent_id.as_str()),
        "remote steering echoes must stay scoped to the target agent"
    );
    let steering_merge_key = crate::history::steering_prompt_merge_key(&queued_prompt_id);
    let steering_history = app_home
        .lock()
        .await
        .operational_history_store()
        .load_session_events(&session_id, Some(&remote_agent_id))
        .expect("remote steering history should load")
        .into_iter()
        .filter(|event| {
            event
                .metadata
                .get("merge_key")
                .and_then(serde_json::Value::as_str)
                == Some(steering_merge_key.as_str())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        steering_history.len(),
        1,
        "successful remote steering must persist one canonical history event"
    );
    assert_eq!(
        steering_history[0].provider_run_id.as_deref(),
        Some(projected_provider_run_id.as_str()),
        "home history must reference the namespaced remote projection, not a colliding local run"
    );
    assert!(steering_history[0]
        .content
        .as_deref()
        .is_some_and(|content| content.contains("REMOTE_QUEUE_STEER_DELIVERY")));

    let queued_request = LocalDaemonRequest::SubmitPrompt(crate::local::SubmitPromptRequest {
        session_id: session_id.clone(),
        attachment_id: attachment_id.clone(),
        target_agent_id: Some(remote_agent_id.clone()),
        prompt: "REMOTE_QUEUE_COMPLETION_DELIVERY\n".to_string(),
        attachments: Vec::new(),
    });
    let queued_command = KernelCommand::from_local_request(
        "command-remote-completion-queue",
        None,
        None,
        &queued_request,
    );
    assert!(matches!(
        router
            .dispatch(queued_command, queued_request)
            .await
            .unwrap(),
        LocalDaemonResponse::PromptSubmitted {
            outcome: crate::session::PromptSubmissionOutcome::Queued { .. },
            ..
        }
    ));
    let complete_request =
        LocalDaemonRequest::CompletePrompt(crate::local::CompletePromptRequest {
            session_id: session_id.clone(),
        });
    let complete_command = KernelCommand::from_local_request(
        "command-remote-completion-promote",
        None,
        None,
        &complete_request,
    );
    let response = router
        .dispatch(complete_command, complete_request)
        .await
        .expect("remote completion must promote the queued prompt through the runtime");
    let LocalDaemonResponse::PromptCompleted { completion, .. } = response else {
        panic!("unexpected remote completion response");
    };
    assert_eq!(completion.completed.target_agent_id(), remote_agent_id);
    assert_eq!(
        completion.completed.prompt(),
        "remote prompt over home session\n"
    );
    let promoted = completion
        .started_next
        .expect("queued prompt must be promoted");
    assert_eq!(promoted.prompt(), "REMOTE_QUEUE_COMPLETION_DELIVERY\n");
    let mut home = app_home.lock().await;
    let confirmed_run = home
        .agents()
        .get_agent(&remote_agent_id)
        .unwrap()
        .remote_execution()
        .unwrap()
        .active_worker_provider_run_id
        .clone();
    assert_eq!(
        confirmed_run.as_deref(),
        Some(worker_provider_run_id.as_str()),
        "queue promotion must retain the acknowledged worker run so managed output is accepted"
    );
    let active = home
        .prompt_owner_active_prompt_for_agent(&session_id, &remote_agent_id)
        .unwrap()
        .expect("promoted prompt must remain active");
    assert_eq!(active.id(), promoted.id());
    assert_eq!(
        active.durable_delivery_phase(),
        Some(crate::session::DurablePromptDeliveryPhase::Delivered),
        "acknowledged queue promotion must be durable as delivered"
    );
    drop(home);

    let worker_run = app_worker
        .lock()
        .await
        .providers()
        .get_run(&worker_provider_run_id)
        .expect("acknowledged worker run must exist");
    let event = RelayPeerEvent::LeasedRuntimeProjection {
        home_session_id: session_id.clone(),
        home_agent_id: remote_agent_id.clone(),
        provider_run_id: worker_provider_run_id.clone(),
        provider_run: Some(worker_run),
        prompts: Vec::new(),
        output_chunks: vec![crate::transport::relay_peer::RelayProjectedOutputChunk {
            kind: crate::terminal::TerminalOutputKind::ProviderOutput,
            merge_key: Some("promoted-worker-result".into()),
            bytes: b"PROMOTED_WORKER_RESULT".to_vec(),
        }],
        notices: Vec::new(),
        completions: vec![crate::transport::relay_peer::RelayProjectedCompletion {
            message_id: "promoted-worker-completion".into(),
            completed_at_ms: crate::session::unix_epoch_ms(),
            home_prompt_id: Some(promoted.id().to_string()),
        }],
    };
    let encrypted = relay_crypto::encrypt_payload_for_peer(
        &config_worker.relay_private_key,
        &config_home.relay_public_key,
        &serde_json::to_vec(&event).unwrap(),
    )
    .unwrap();
    handle_daemon_peer_event(&router, encrypted)
        .await
        .expect("acknowledged managed worker output must project to the home");
    let mut home = app_home.lock().await;
    let output = home
        .terminal_mut()
        .drain_output_records(&session_id, &attachment_id);
    assert!(
        output
            .iter()
            .any(|record| record.bytes == b"PROMOTED_WORKER_RESULT"
                && record.agent_id.as_deref() == Some(remote_agent_id.as_str())),
        "promoted managed output must reach the home attachment"
    );
    assert!(
        home.prompt_owner_active_prompt_for_agent(&session_id, &remote_agent_id)
            .unwrap()
            .is_none(),
        "promoted prompt must settle from its worker completion"
    );
    drop(home);

    let _ = shutdown_home_tx.send(true);
    let _ = shutdown_worker_tx.send(true);
    connector_home.await.expect("home connector should join");
    connector_worker
        .await
        .expect("worker connector should join");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
}
#[tokio::test(flavor = "multi_thread")]
async fn remote_machine_agents_materialize_file_attachments_on_the_worker() {
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

    let mut config_home = DaemonConfig::for_tests();
    config_home.daemon_id = "daemon-home".to_string();
    config_home.daemon_alias = Some("home".to_string());
    config_home.host_machine_id = "machine-home".to_string();
    config_home.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_home.relay_token = Some("secret".to_string());
    config_home.relay_heartbeat_ms = 50;
    let mut config_worker = DaemonConfig::for_tests();
    config_worker.daemon_id = "daemon-worker".to_string();
    config_worker.daemon_alias = Some("worker".to_string());
    config_worker.host_machine_id = "machine-worker".to_string();
    config_worker.host_machine_alias = Some("builder-west".to_string());
    config_worker.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_worker.relay_token = Some("secret".to_string());
    config_worker.relay_heartbeat_ms = 50;
    config_worker.accept_remote_leases = true;
    let app_worker = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_worker.clone()).expect("worker daemon should bootstrap"),
    ));
    let state_worker = {
        let app = app_worker.lock().await;
        app.relay_client_state()
    };
    let (shutdown_worker_tx, shutdown_worker_rx) = watch::channel(false);
    let connector_worker = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_worker),
        Arc::clone(&state_worker),
        shutdown_worker_rx,
    ));

    wait_for_daemon_registration(registry.clone(), &config_worker.daemon_id).await;

    let provider = relay_discovery::list_live_kernels_for_machine(&config_home, "builder-west")
        .await
        .expect("worker kernels should be discoverable")
        .first()
        .and_then(|kernel| {
            kernel
                .available_providers
                .iter()
                .find(|provider| provider.as_str() == "managed-dev-stub")
        })
        .cloned()
        .expect("worker should advertise managed-dev-stub");

    let app_home = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_home.clone()).expect("home daemon should bootstrap"),
    ));
    let state_home = {
        let app = app_home.lock().await;
        app.relay_client_state()
    };
    let (shutdown_home_tx, shutdown_home_rx) = watch::channel(false);
    let connector_home = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_home),
        Arc::clone(&state_home),
        shutdown_home_rx,
    ));
    wait_for_daemon_registration(registry.clone(), &config_home.daemon_id).await;
    refresh_remote_inventory_projection_for_app_with_relay_state(&app_home)
        .await
        .expect("home remote inventory should refresh");
    let (session_id, attachment_id, remote_agent_id, remote_leased_agent_id) = {
        let mut app_home = app_home.lock().await;
        let (session, _) = crate::app::KernelSessionService::new(&mut app_home)
            .create_session(CreateSessionRequest::new("workspace-home", "worktree-home"))
            .expect("home session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app_home)
            .attach(AttachRequest::new(
                session.id(),
                "home-client",
                ClientCapabilityLevel::InteractiveStructured,
            ))
            .expect("home attachment should attach");
        let remote_agent = crate::app::KernelSessionService::new(&mut app_home)
            .spawn_agent(
                CreateAgentRequest::new(session.id(), &provider)
                    .with_alias("remote-reviewer")
                    .with_kernel(&config_worker.daemon_id),
            )
            .expect("remote agent should spawn");
        let leased_agent_id = remote_agent
            .remote_execution()
            .expect("remote binding should exist")
            .leased_agent_id
            .clone();
        (
            session.id().to_string(),
            attachment.id().to_string(),
            remote_agent.id().to_string(),
            leased_agent_id,
        )
    };

    let source_path = std::env::temp_dir().join(format!(
        "chariox-remote-attachment-{}.txt",
        crate::session::unix_epoch_ms()
    ));
    std::fs::write(&source_path, b"remote attachment body")
        .expect("source attachment should be written");

    let outcome = app_home
        .lock()
        .await
        .submit_prompt(
            &session_id,
            &attachment_id,
            Some(&remote_agent_id),
            "prompt with attachment\n",
            vec![crate::session::PromptAttachment::new(
                format!("file://{}", source_path.display()),
                "text/plain",
                Some("note.txt".to_string()),
            )],
        )
        .expect("remote prompt should submit");
    assert!(matches!(
        outcome,
        crate::session::PromptSubmissionOutcome::Started { .. }
    ));

    let worker_attachments = wait_for_leased_agent_active_prompt_attachments(
        app_worker.clone(),
        &remote_leased_agent_id,
    )
    .await;
    assert_eq!(worker_attachments.len(), 1);
    let materialized = &worker_attachments[0];
    assert_eq!(materialized.filename(), Some("note.txt"));
    assert_eq!(materialized.mime(), "text/plain");
    assert!(materialized.url().starts_with("file://"));
    assert_ne!(
        materialized.url(),
        format!("file://{}", source_path.display())
    );
    let worker_path = materialized.url().trim_start_matches("file://");
    let worker_bytes = std::fs::read(worker_path).expect("worker attachment should exist");
    assert_eq!(worker_bytes, b"remote attachment body");

    let _ = std::fs::remove_file(&source_path);
    let _ = shutdown_home_tx.send(true);
    let _ = shutdown_worker_tx.send(true);
    connector_home.await.expect("home connector should join");
    connector_worker
        .await
        .expect("worker connector should join");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
}

async fn wait_for_leased_agent_active_prompt_attachments(
    app: Arc<Mutex<DaemonApp>>,
    leased_agent_id: &str,
) -> Vec<crate::session::PromptAttachment> {
    for _ in 0..400 {
        let attachments = {
            let mut app = app.lock().await;
            RemoteLeaseRuntime::new(&mut app)
                .leased_agent_active_prompt_attachments(leased_agent_id)
                .expect("worker prompt attachments should be available")
        };
        if !attachments.is_empty() {
            return attachments;
        }
        sleep(Duration::from_millis(25)).await;
    }
    panic!(
        "worker prompt attachments did not become available for leased agent `{leased_agent_id}`"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn remote_machine_agents_cancel_prompts_through_the_home_session() {
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

    let mut config_home = DaemonConfig::for_tests();
    config_home.daemon_id = "daemon-home".to_string();
    config_home.daemon_alias = Some("home".to_string());
    config_home.host_machine_id = "machine-home".to_string();
    config_home.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_home.relay_token = Some("secret".to_string());
    config_home.relay_heartbeat_ms = 50;
    let mut config_worker = DaemonConfig::for_tests();
    config_worker.daemon_id = "daemon-worker".to_string();
    config_worker.daemon_alias = Some("worker".to_string());
    config_worker.host_machine_id = "machine-worker".to_string();
    config_worker.host_machine_alias = Some("builder-west".to_string());
    config_worker.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_worker.relay_token = Some("secret".to_string());
    config_worker.relay_heartbeat_ms = 50;
    config_worker.accept_remote_leases = true;
    let app_worker = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_worker.clone()).expect("worker daemon should bootstrap"),
    ));
    let state_worker = {
        let app = app_worker.lock().await;
        app.relay_client_state()
    };
    let (shutdown_worker_tx, shutdown_worker_rx) = watch::channel(false);
    let connector_worker = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_worker),
        Arc::clone(&state_worker),
        shutdown_worker_rx,
    ));

    wait_for_daemon_registration(registry.clone(), &config_worker.daemon_id).await;

    let provider = relay_discovery::list_live_kernels_for_machine(&config_home, "builder-west")
        .await
        .expect("worker kernels should be discoverable")
        .first()
        .and_then(|kernel| {
            kernel
                .available_providers
                .iter()
                .find(|provider| provider.as_str() == "managed-dev-stub")
        })
        .cloned()
        .expect("worker should advertise managed-dev-stub");

    let app_home = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_home.clone()).expect("home daemon should bootstrap"),
    ));
    let state_home = {
        let app = app_home.lock().await;
        app.relay_client_state()
    };
    let (shutdown_home_tx, shutdown_home_rx) = watch::channel(false);
    let connector_home = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_home),
        Arc::clone(&state_home),
        shutdown_home_rx,
    ));
    wait_for_daemon_registration(registry.clone(), &config_home.daemon_id).await;
    refresh_remote_inventory_projection_for_app_with_relay_state(&app_home)
        .await
        .expect("home remote inventory should refresh");
    let (session_id, attachment_id) = {
        let mut app_home = app_home.lock().await;
        let (session, _) = crate::app::KernelSessionService::new(&mut app_home)
            .create_session(CreateSessionRequest::new("workspace-home", "worktree-home"))
            .expect("home session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app_home)
            .attach(AttachRequest::new(
                session.id(),
                "home-client",
                ClientCapabilityLevel::InteractiveStructured,
            ))
            .expect("home attachment should attach");
        (session.id().to_string(), attachment.id().to_string())
    };
    let remote_agent_id = {
        let mut app_home = app_home.lock().await;
        crate::app::KernelSessionService::new(&mut app_home)
            .spawn_agent(
                CreateAgentRequest::new(&session_id, &provider)
                    .with_alias("remote-reviewer")
                    .with_model("default")
                    .with_kernel(&config_worker.daemon_id),
            )
            .expect("remote agent should spawn")
            .id()
            .to_string()
    };

    let (outcome, cancellation, forced_cancellation) = {
        // Keep the home state locked from admission through the repeated
        // cancellation. The remote dev-stub may otherwise finish the prompt
        // between these assertions when the full suite is under load.
        let mut app_home = app_home.lock().await;
        let outcome = app_home
            .submit_prompt(
                &session_id,
                &attachment_id,
                Some(&remote_agent_id),
                "cancel this remote prompt\n",
                Vec::new(),
            )
            .expect("remote prompt should submit");
        let cancellation = app_home
            .cancel_active_prompt(&session_id, &attachment_id)
            .expect("remote prompt should cancel");
        let forced_cancellation = app_home
            .cancel_active_prompt(&session_id, &attachment_id)
            .expect("a repeated remote cancellation should force settlement");
        (outcome, cancellation, forced_cancellation)
    };
    assert!(matches!(
        outcome,
        crate::session::PromptSubmissionOutcome::Started { .. }
    ));

    assert_eq!(cancellation.prompt.target_agent_id(), remote_agent_id);
    assert_eq!(
        cancellation.prompt.status(),
        crate::session::PromptStatus::Cancelling
    );

    assert_eq!(
        forced_cancellation.prompt.status(),
        crate::session::PromptStatus::Cancelled
    );
    assert!(app_home
        .lock()
        .await
        .prompt_owner_active_prompt_for_agent(&session_id, &remote_agent_id)
        .expect("home prompt state should remain readable")
        .is_none());

    let _ = shutdown_home_tx.send(true);
    let _ = shutdown_worker_tx.send(true);
    connector_home.await.expect("home connector should join");
    connector_worker
        .await
        .expect("worker connector should join");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
}
