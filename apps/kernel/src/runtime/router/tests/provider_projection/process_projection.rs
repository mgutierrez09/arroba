use super::*;

#[test]
fn list_provider_processes_uses_warmed_projection_without_app_lock() {
    run_provider_projection_large_stack_test(
        "list-provider-processes-uses-warmed-projection-without-app-lock",
        list_provider_processes_uses_warmed_projection_without_app_lock_inner,
    );
}

async fn list_provider_processes_uses_warmed_projection_without_app_lock_inner() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let launch_request = LocalDaemonRequest::LaunchProviderRun(LaunchProviderRunRequest {
        session_id: session_id.clone(),
        agent_id: Some(agent_id.clone()),
        adapter_key: "dev-stub".to_string(),
        provider: "claude-code".to_string(),
        account_profile: "default".to_string(),
        model: "sonnet".to_string(),
        variant: None,
        structured_endpoint: None,
        provider_session_id: None,
        native_tui: false,
    });
    let launch_command = KernelCommand::from_local_request(
        "cmd-process-provider-launch",
        None,
        None,
        &launch_request,
    );
    let provider_run_id = match router
        .dispatch(launch_command, launch_request)
        .await
        .expect("provider launch should be accepted")
    {
        LocalDaemonResponse::ProviderRunLaunchAccepted { provider_run } => {
            provider_run.id().to_string()
        }
        _ => panic!("unexpected launch response"),
    };

    let canonical_processes = {
        let app = app.lock().await;
        app.list_provider_processes(None)
            .expect("provider process list should warm projection")
    };
    router
        .provider_process_projection
        .update_list(canonical_processes);

    let app_guard = app.lock().await;
    let projected_list_request =
        LocalDaemonRequest::ListProviderProcesses(ListProviderProcessesRequest { provider: None });
    let projected_list_command = KernelCommand::from_local_request(
        "cmd-process-list-projection",
        None,
        None,
        &projected_list_request,
    );
    let list_response = tokio::time::timeout(
        Duration::from_millis(100),
        router.dispatch_pre_lane(
            &projected_list_command,
            &projected_list_request,
            crate::session::DEFAULT_LOCAL_USER_ID,
        ),
    )
    .await
    .expect("warmed ListProviderProcesses should not wait for the app lock")
    .expect("provider process projection should not fail")
    .expect("warmed ListProviderProcesses should be served from projection");
    drop(app_guard);
    match list_response {
        LocalDaemonResponse::ProviderProcessesListed { processes } => {
            assert_eq!(processes.len(), 1);
            assert_eq!(processes[0].owner_provider_run_ids, vec![provider_run_id]);
        }
        _ => panic!("unexpected provider process list response"),
    }
}

#[test]
fn list_provider_processes_blocks_teardown_for_per_agent_active_prompt_without_app_lock() {
    run_provider_projection_large_stack_test(
        "list-provider-processes-blocks-teardown-for-per-agent-active-prompt-without-app-lock",
        list_provider_processes_blocks_teardown_for_per_agent_active_prompt_without_app_lock_inner,
    );
}

async fn list_provider_processes_blocks_teardown_for_per_agent_active_prompt_without_app_lock_inner(
) {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, focused_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let focused_agent_id = focused_agent.id().to_string();
    let background_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(&session_id, "dev-stub").with_alias("background"))
        .expect("background agent should be created");
    let background_agent_id = background_agent.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "client-1",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("session should attach");
    launch_test_provider(
        &mut app,
        &session_id,
        &background_agent_id,
        "dev-stub",
        "claude-code",
        "sonnet",
    );
    app.focus_agent(&session_id, &focused_agent_id)
        .expect("focused agent should be restored");
    app.detach(attachment.id())
        .expect("detaching should remove attached-session teardown blocker");

    let background_prompt = crate::session::PromptQueueItem::new(
        "prompt-background-active",
        attachment.id(),
        &background_agent_id,
        "background active prompt",
        crate::session::PromptStatus::Running,
    );
    app.prompt_owner_activate_prompt(&session_id, background_prompt)
        .expect("background prompt state should activate");
    let projected_session = app
        .sessions()
        .get_session(&session_id)
        .expect("session should exist");
    assert!(
        projected_session.active_prompt().is_none(),
        "legacy focused prompt projection intentionally has no active prompt"
    );
    assert!(
        projected_session.has_any_active_prompt(),
        "per-agent prompt state still has active work"
    );

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let _app_guard = app.lock().await;
    let process_list = router.runtime_state.list_provider_processes(None);

    assert_eq!(process_list.filtered_processes.len(), 1);
    assert!(
        !process_list.filtered_processes[0].teardown_safe,
        "{:?}",
        process_list.filtered_processes[0]
    );
    assert!(process_list.filtered_processes[0]
        .attached_session_ids
        .is_empty());
    assert_eq!(
        process_list.filtered_processes[0].teardown_blockers,
        vec!["active prompt"]
    );
}

#[test]
fn provider_process_projection_invalidates_when_prompt_state_changes() {
    run_provider_projection_large_stack_test(
        "provider-process-projection-invalidates-when-prompt-state-changes",
        provider_process_projection_invalidates_when_prompt_state_changes_inner,
    );
}

async fn provider_process_projection_invalidates_when_prompt_state_changes_inner() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "client-1",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("session should attach");
    launch_test_provider(
        &mut app,
        &session_id,
        &agent_id,
        "dev-stub",
        "claude-code",
        "sonnet",
    );
    let canonical_processes = app
        .list_provider_processes(None)
        .expect("provider process list should warm projection");

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    router
        .provider_process_projection
        .update_list(canonical_processes);
    assert!(
        router.provider_process_projection.list(None).is_some(),
        "test setup should start with a warmed provider-process projection"
    );

    let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session_id.clone(),
        attachment_id: attachment.id().to_string(),
        target_agent_id: Some(agent_id.clone()),
        prompt: "invalidate provider process projection".to_string(),
        attachments: Vec::new(),
    });
    let prompt_command = KernelCommand::from_local_request(
        "cmd-process-projection-invalidating-prompt",
        None,
        None,
        &prompt_request,
    );
    router
        .dispatch(prompt_command, prompt_request)
        .await
        .expect("prompt submit should succeed");
    assert!(
        router.provider_process_projection.list(None).is_none(),
        "prompt-state mutation should invalidate stale provider-process projection"
    );

    let list_request =
        LocalDaemonRequest::ListProviderProcesses(ListProviderProcessesRequest { provider: None });
    let list_command = KernelCommand::from_local_request(
        "cmd-process-list-after-prompt-invalidation",
        None,
        None,
        &list_request,
    );
    let list_response = router
        .dispatch(list_command, list_request)
        .await
        .expect("provider process list should recompute after invalidation");
    match list_response {
        LocalDaemonResponse::ProviderProcessesListed { processes } => {
            assert_eq!(processes.len(), 1);
            assert!(!processes[0].teardown_safe);
            assert!(processes[0]
                .teardown_blockers
                .iter()
                .any(|blocker| blocker == "active prompt"));
        }
        _ => panic!("unexpected provider process list response"),
    }
}

#[test]
fn provider_process_projection_stores_canonical_unfiltered_snapshot() {
    run_provider_projection_large_stack_test(
        "provider-process-projection-stores-canonical-unfiltered-snapshot",
        provider_process_projection_stores_canonical_unfiltered_snapshot_inner,
    );
}

async fn provider_process_projection_stores_canonical_unfiltered_snapshot_inner() {
    let mut app = crate::test_support::bootstrap_authenticated_app(DaemonConfig::for_tests())
        .expect("daemon should boot");
    for (idx, provider, model) in [(1, "claude-code", "sonnet"), (2, "codex", "gpt-5.4")] {
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                format!("workspace-{idx}"),
                format!("worktree-{idx}"),
            ))
            .expect("session should be created");
        launch_test_provider(
            &mut app,
            session.id(),
            agent.id(),
            "dev-stub",
            provider,
            model,
        );
    }

    let filtered = app
        .list_provider_processes(Some("claude-code"))
        .expect("filtered process list should warm projection");
    assert_eq!(filtered.len(), 1);

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let app_guard = app.lock().await;
    let list_request =
        LocalDaemonRequest::ListProviderProcesses(ListProviderProcessesRequest { provider: None });
    let list_command = KernelCommand::from_local_request(
        "cmd-process-canonical-projection",
        None,
        None,
        &list_request,
    );
    let list_router = router.clone();
    let list_task =
        tokio::spawn(async move { list_router.dispatch(list_command, list_request).await });

    let list_response = tokio::time::timeout(Duration::from_millis(100), list_task)
        .await
        .expect("projected provider process list should not wait for the app lock")
        .expect("list task should join")
        .expect("list should resolve");
    drop(app_guard);

    match list_response {
        LocalDaemonResponse::ProviderProcessesListed { processes } => {
            assert_eq!(processes.len(), 2);
        }
        _ => panic!("unexpected provider process list response"),
    }
}

#[test]
fn provider_process_projection_updates_after_teardown() {
    run_provider_projection_large_stack_test(
        "provider-process-projection-updates-after-teardown",
        provider_process_projection_updates_after_teardown_inner,
    );
}

async fn provider_process_projection_updates_after_teardown_inner() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    launch_test_provider(
        &mut app,
        &session_id,
        &agent_id,
        "dev-stub",
        "claude-code",
        "sonnet",
    );
    app.list_provider_processes(None)
        .expect("process list should warm projection");
    app.teardown_provider_processes(None, false)
        .expect("teardown should update projection");

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let app_guard = app.lock().await;
    let list_request =
        LocalDaemonRequest::ListProviderProcesses(ListProviderProcessesRequest { provider: None });
    let list_command = KernelCommand::from_local_request(
        "cmd-process-post-teardown-projection",
        None,
        None,
        &list_request,
    );
    let list_router = router.clone();
    let list_task =
        tokio::spawn(async move { list_router.dispatch(list_command, list_request).await });

    let list_response = tokio::time::timeout(Duration::from_millis(100), list_task)
        .await
        .expect("post-teardown provider process list should not wait for the app lock")
        .expect("list task should join")
        .expect("list should resolve");
    drop(app_guard);

    match list_response {
        LocalDaemonResponse::ProviderProcessesListed { processes } => {
            assert!(processes.is_empty());
        }
        _ => panic!("unexpected provider process list response"),
    }
}

#[test]
fn provider_process_teardown_only_terminates_caller_owned_processes() {
    run_provider_projection_large_stack_test(
        "provider-process-teardown-only-terminates-caller-owned-processes",
        provider_process_teardown_only_terminates_caller_owned_processes_inner,
    );
}

async fn provider_process_teardown_only_terminates_caller_owned_processes_inner() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, local_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let (_, invite) = app
        .sessions_mut()
        .create_session_invite(
            &session_id,
            "provider-process-peer".to_string(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            None,
            Some(1),
            crate::session::CollaborationLevel::Full,
        )
        .expect("invite should be created");
    app.sessions_mut()
        .join_session_invite(&session_id, invite.invite_id(), "user-2".to_string(), 1)
        .expect("user should join session");
    let peer_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(&session_id, "dev-stub")
                .with_alias("peer")
                .with_owner_user_id("user-2"),
        )
        .expect("peer agent should be created");
    let local_run = launch_test_provider(
        &mut app,
        &session_id,
        local_agent.id(),
        "dev-stub",
        "claude-code",
        "sonnet",
    );
    let peer_run = launch_test_provider(
        &mut app,
        &session_id,
        peer_agent.id(),
        "dev-stub",
        "managed-dev-stub",
        "gpt-5.4",
    );
    let process_before_teardown = app
        .list_provider_processes(None)
        .expect("processes should list");
    assert_eq!(process_before_teardown.len(), 2);
    assert!(process_before_teardown
        .iter()
        .any(|process| process.owner_provider_run_ids == vec![local_run.id().to_string()]));
    assert!(process_before_teardown
        .iter()
        .any(|process| process.owner_provider_run_ids == vec![peer_run.id().to_string()]));

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let list_request =
        LocalDaemonRequest::ListProviderProcesses(ListProviderProcessesRequest { provider: None });
    let list_command = remote_command_for_request(&list_request, Some("user-2"));
    let list_response = router
        .dispatch(list_command, list_request)
        .await
        .expect("list should complete");
    match list_response {
        LocalDaemonResponse::ProviderProcessesListed { processes } => {
            assert_eq!(processes.len(), 1);
            assert!(processes[0].teardown_safe);
            assert!(processes[0].teardown_blockers.is_empty());
            assert_eq!(
                processes[0].owner_provider_run_ids,
                vec![peer_run.id().to_string()]
            );
        }
        _ => panic!("unexpected list response"),
    }

    let teardown_request =
        LocalDaemonRequest::TeardownProviderProcesses(TeardownProviderProcessesRequest {
            provider: None,
            force: false,
        });
    let teardown_command = remote_command_for_request(&teardown_request, Some("user-2"));
    let teardown_response = router
        .dispatch(teardown_command, teardown_request)
        .await
        .expect("teardown should complete");

    match teardown_response {
        LocalDaemonResponse::ProviderProcessesTornDown { processes } => {
            assert_eq!(processes.len(), 1);
            assert_eq!(
                processes[0].owner_provider_run_ids,
                vec![peer_run.id().to_string()]
            );
        }
        _ => panic!("unexpected teardown response"),
    }
    let remaining = app
        .lock()
        .await
        .list_provider_processes(None)
        .expect("remaining processes should list");
    assert_eq!(remaining.len(), 1);
    assert_eq!(
        remaining[0].owner_provider_run_ids,
        vec![local_run.id().to_string()]
    );
}

#[test]
fn teardown_provider_processes_refreshes_session_projection_without_app_lock() {
    run_provider_projection_large_stack_test(
        "teardown-provider-processes-refreshes-session-projection-without-app-lock",
        teardown_provider_processes_refreshes_session_projection_without_app_lock_inner,
    );
}

async fn teardown_provider_processes_refreshes_session_projection_without_app_lock_inner() {
    let mut config = DaemonConfig::for_tests();
    config.provider_runtime_init_delay_ms = 25;
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let agent_store = app.agents().clone();
    let provider_store = app.providers().clone();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let launch_request = LocalDaemonRequest::LaunchProviderRun(LaunchProviderRunRequest {
        session_id: session_id.clone(),
        agent_id: Some(agent_id),
        adapter_key: "dev-stub".to_string(),
        provider: "claude-code".to_string(),
        account_profile: "default".to_string(),
        model: "sonnet".to_string(),
        variant: None,
        structured_endpoint: None,
        provider_session_id: None,
        native_tui: false,
    });
    let launch_command = KernelCommand::from_local_request(
        "cmd-teardown-refresh-launch",
        None,
        None,
        &launch_request,
    );
    let launch_response = router
        .dispatch(launch_command, launch_request)
        .await
        .expect("provider launch should be accepted");
    let provider_run_id = match launch_response {
        LocalDaemonResponse::ProviderRunLaunchAccepted { provider_run }
        | LocalDaemonResponse::ProviderRunLaunched { provider_run } => {
            provider_run.id().to_string()
        }
        _ => panic!("unexpected launch response"),
    };

    let agent_guard = agent_store.write();
    timeout(Duration::from_secs(2), async {
        loop {
            if provider_store
                .get_run(&provider_run_id)
                .is_ok_and(|run| run.state() == crate::provider::ProviderRunState::Running)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("provider launch should reach running before teardown");

    let teardown_request =
        LocalDaemonRequest::TeardownProviderProcesses(TeardownProviderProcessesRequest {
            provider: None,
            force: false,
        });
    let teardown_command =
        KernelCommand::from_local_request("cmd-teardown-refresh", None, None, &teardown_request);
    let teardown_router = router.clone();
    let mut teardown_task = tokio::spawn(async move {
        teardown_router
            .dispatch(teardown_command, teardown_request)
            .await
    });
    assert!(
        timeout(Duration::from_millis(250), &mut teardown_task)
            .await
            .is_err(),
        "teardown must wait for in-flight provider launch settlement"
    );
    drop(agent_guard);
    let teardown_response = timeout(Duration::from_secs(2), teardown_task)
        .await
        .expect("teardown should complete after launch settlement")
        .expect("teardown task should join")
        .expect("safe process teardown should succeed");
    match teardown_response {
        LocalDaemonResponse::ProviderProcessesTornDown { processes } => {
            assert_eq!(processes.len(), 1);
        }
        _ => panic!("unexpected teardown response"),
    }

    let app_guard = app.lock().await;
    let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
        session_id: session_id.clone(),
    });
    let state_command =
        KernelCommand::from_local_request("cmd-teardown-refresh-state", None, None, &state_request);
    let state_router = router.clone();
    let state_task =
        tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });

    let state_response = timeout(Duration::from_millis(100), state_task)
        .await
        .expect("post-teardown session state should not wait for the app lock")
        .expect("state task should join")
        .expect("state should resolve");
    drop(app_guard);

    match state_response {
        LocalDaemonResponse::SessionState { session, .. } => {
            assert_eq!(session.id(), session_id);
            assert_eq!(session.active_provider_run_id(), None);
        }
        _ => panic!("unexpected session state response"),
    }
}
