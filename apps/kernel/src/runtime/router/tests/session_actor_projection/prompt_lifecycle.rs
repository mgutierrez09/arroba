use super::*;

#[tokio::test]
async fn get_session_state_projection_tracks_prompt_completion_without_app_lock() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-complete-projection",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    launch_test_provider(
        &mut app,
        &session_id,
        &agent_id,
        "dev-stub",
        "claude-code",
        "sonnet",
    );

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session_id.clone(),
        attachment_id: attachment.id().to_string(),
        target_agent_id: Some(agent_id.clone()),
        prompt: "complete projection".to_string(),
        attachments: Vec::new(),
    });
    let prompt_command =
        KernelCommand::from_local_request("cmd-prompt-complete-state", None, None, &prompt_request);
    router
        .dispatch(prompt_command, prompt_request)
        .await
        .expect("prompt submit should warm active prompt projection");
    let prompt_projection = router
        .agent_runtime_projection
        .get(&agent_id)
        .expect("agent runtime projection should track prompt state after submit");
    assert!(prompt_projection.active_prompt.is_some());
    assert_eq!(prompt_projection.queued_prompt_count, 0);

    let before_complete_projection_sequence = router.session_projection_change_sequence();
    let complete_request = LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
        session_id: session_id.clone(),
    });
    let complete_command = KernelCommand::from_local_request(
        "cmd-complete-state-projection",
        None,
        None,
        &complete_request,
    );
    router
        .dispatch(complete_command, complete_request)
        .await
        .expect("prompt completion should publish session projection through agent runtime");
    assert!(
        router.session_projection_change_sequence() > before_complete_projection_sequence,
        "prompt completion should wake session projection subscribers"
    );
    let prompt_projection = router
        .agent_runtime_projection
        .get(&agent_id)
        .expect("agent runtime projection should retain prompt state after complete");
    assert!(prompt_projection.active_prompt.is_none());
    assert_eq!(prompt_projection.queued_prompt_count, 0);

    let app_guard = app.lock().await;
    let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
        session_id: session_id.clone(),
    });
    let state_command = KernelCommand::from_local_request(
        "cmd-state-complete-projection",
        None,
        None,
        &state_request,
    );
    let state_router = router.clone();
    let state_task =
        tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });

    tokio::task::yield_now().await;
    assert!(
        state_task.is_finished(),
        "completed prompt state should be served from projection without app lock access"
    );
    drop(app_guard);

    let state_response = state_task
        .await
        .expect("state task should join")
        .expect("state should resolve");
    match state_response {
        LocalDaemonResponse::SessionState { session, .. } => {
            assert!(session.active_prompt_for_agent(&agent_id).is_none());
        }
        _ => panic!("unexpected state response"),
    }
}

#[test]
fn session_snapshot_refresh_tracks_agent_runtime_projection() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_stack_size(8 * 1024 * 1024)
        .enable_all()
        .build()
        .expect("test runtime should build");
    runtime.block_on(async {
        tokio::spawn(async move {
            session_snapshot_refresh_tracks_agent_runtime_projection_inner().await
        })
        .await
        .expect("test task should complete");
    });
}

async fn session_snapshot_refresh_tracks_agent_runtime_projection_inner() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-prompt-shadow-refresh",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    launch_test_provider(
        &mut app,
        &session_id,
        &agent_id,
        "dev-stub",
        "claude-code",
        "sonnet",
    );

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session_id.clone(),
        attachment_id: attachment.id().to_string(),
        target_agent_id: Some(agent_id.clone()),
        prompt: "shadow refresh".to_string(),
        attachments: Vec::new(),
    });
    let prompt_command =
        KernelCommand::from_local_request("cmd-shadow-submit", None, None, &prompt_request);
    router
        .dispatch(prompt_command, prompt_request)
        .await
        .expect("prompt submit should warm agent runtime projection");
    assert!(router
        .agent_runtime_projection
        .get(&agent_id)
        .and_then(|projection| projection.active_prompt)
        .is_some());

    {
        let mut app = app.lock().await;
        app.prompt_owner_complete_active_prompt_only(&session_id, &agent_id)
            .expect("prompt owner should settle");
    }
    assert!(
        router
            .agent_runtime_projection
            .get(&agent_id)
            .and_then(|projection| projection.active_prompt)
            .is_none(),
        "prompt owner completion should refresh the agent runtime projection"
    );

    let pump_request = LocalDaemonRequest::PumpTerminalOutput(PumpTerminalOutputRequest {
        session_id: session_id.clone(),
        attachment_id: attachment.id().to_string(),
    });
    let pump_command =
        KernelCommand::from_local_request("cmd-shadow-refresh", None, None, &pump_request);
    router
        .dispatch(pump_command, pump_request)
        .await
        .expect("snapshot-producing pump should refresh projections");

    let prompt_projection = router
        .agent_runtime_projection
        .get(&agent_id)
        .expect("agent prompt projection should remain registered");
    assert!(prompt_projection.active_prompt.is_none());
    assert_eq!(prompt_projection.queued_prompt_count, 0);
}

#[tokio::test]
async fn prompt_complete_uses_agent_runtime_projection_when_session_projection_is_stale() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let default_agent_id = default_agent.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-complete-owner-projection",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let spawned_agent = spawn_test_agent(&mut app, &session_id, "worker", "claude-code");
    let spawned_agent_id = spawned_agent.id().to_string();
    launch_test_provider(
        &mut app,
        &session_id,
        &spawned_agent_id,
        "dev-stub",
        "claude-code",
        "sonnet",
    );
    focus_test_agent(&mut app, &session_id, &default_agent_id);
    let idle_session_snapshot = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(&session_id)
        .expect("idle session snapshot should be available");

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session_id.clone(),
        attachment_id: attachment.id().to_string(),
        target_agent_id: Some(spawned_agent_id.clone()),
        prompt: "complete owner projection".to_string(),
        attachments: Vec::new(),
    });
    let prompt_command =
        KernelCommand::from_local_request("cmd-prompt-complete-owner", None, None, &prompt_request);
    router
        .dispatch(prompt_command, prompt_request)
        .await
        .expect("prompt submit should warm active prompt projection");
    router.session_projection.update(idle_session_snapshot);

    let app_guard = app.lock().await;
    let complete_request = LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
        session_id: session_id.clone(),
    });
    let complete_command = KernelCommand::from_local_request(
        "cmd-complete-owner-projection",
        None,
        None,
        &complete_request,
    );
    let complete_router = router.clone();
    let complete_task = tokio::spawn(async move {
        complete_router
            .dispatch(complete_command, complete_request)
            .await
    });

    let mut spawned_agent_lane_created = false;
    for _ in 0..50 {
        let projection = router.daemon_health_projection(0).await;
        if projection
            .agent_command_lanes
            .iter()
            .any(|lane| lane.lane_id == spawned_agent_id)
        {
            spawned_agent_lane_created = true;
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        spawned_agent_lane_created,
        "prompt complete should resolve the active prompt owner from the agent-runtime projection before touching the app lock"
    );
    assert!(
        !complete_task.is_finished(),
        "agent worker should still wait on the deliberately held app lock"
    );

    drop(app_guard);
    let complete_response = complete_task
        .await
        .expect("complete task should join")
        .expect("prompt should complete");
    match complete_response {
        LocalDaemonResponse::PromptCompleted { .. } => {}
        _ => panic!("unexpected complete response"),
    }
}

#[tokio::test]
async fn get_session_state_projection_tracks_prompt_cancellation_without_app_lock() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-cancel-projection",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    launch_test_provider(
        &mut app,
        &session_id,
        &agent_id,
        "dev-stub",
        "claude-code",
        "sonnet",
    );

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session_id.clone(),
        attachment_id: attachment.id().to_string(),
        target_agent_id: Some(agent_id.clone()),
        prompt: "cancel projection".to_string(),
        attachments: Vec::new(),
    });
    let prompt_command =
        KernelCommand::from_local_request("cmd-prompt-cancel-state", None, None, &prompt_request);
    router
        .dispatch(prompt_command, prompt_request)
        .await
        .expect("prompt submit should warm active prompt projection");
    assert!(router
        .agent_runtime_projection
        .get(&agent_id)
        .and_then(|projection| projection.active_prompt)
        .is_some());

    let cancel_request = LocalDaemonRequest::CancelActivePrompt(CancelActivePromptRequest {
        session_id: session_id.clone(),
        attachment_id: attachment.id().to_string(),
        target_agent_id: None,
    });
    let cancel_command = KernelCommand::from_local_request(
        "cmd-cancel-state-projection",
        None,
        None,
        &cancel_request,
    );
    // Keep the PTY abort worker from finalizing cancellation before we inspect
    // its intermediate projection. Cancellation admission itself is owned state.
    let app_guard = app.lock().await;
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        router.dispatch(cancel_command, cancel_request),
    )
    .await
    .expect("cancellation admission must not wait for the app lock")
    .expect("prompt cancellation should publish session projection");
    let prompt_projection = router
        .agent_runtime_projection
        .get(&agent_id)
        .expect("agent runtime projection should retain prompt state after cancel");
    assert_eq!(
        prompt_projection
            .active_prompt
            .as_ref()
            .map(|prompt| prompt.status()),
        Some(PromptStatus::Cancelling)
    );

    let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
        session_id: session_id.clone(),
    });
    let state_command = KernelCommand::from_local_request(
        "cmd-state-cancel-projection",
        None,
        None,
        &state_request,
    );
    let state_router = router.clone();
    let state_task =
        tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });

    tokio::task::yield_now().await;
    assert!(
        state_task.is_finished(),
        "cancelled prompt state should be served from projection without app lock access"
    );
    drop(app_guard);

    let state_response = state_task
        .await
        .expect("state task should join")
        .expect("state should resolve");
    match state_response {
        LocalDaemonResponse::SessionState { session, .. } => {
            let active_prompt = session
                .active_prompt_for_agent(&agent_id)
                .expect("prompt should still be settling");
            assert_eq!(active_prompt.status(), PromptStatus::Cancelling);
        }
        _ => panic!("unexpected state response"),
    }
}

#[tokio::test]
async fn prompt_cancel_uses_agent_runtime_projection_when_session_projection_is_stale() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let default_agent_id = default_agent.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-cancel-owner-projection",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let spawned_agent = spawn_test_agent(&mut app, &session_id, "worker", "claude-code");
    let spawned_agent_id = spawned_agent.id().to_string();
    launch_test_provider(
        &mut app,
        &session_id,
        &spawned_agent_id,
        "dev-stub",
        "claude-code",
        "sonnet",
    );
    focus_test_agent(&mut app, &session_id, &default_agent_id);
    let idle_session_snapshot = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(&session_id)
        .expect("idle session snapshot should be available");

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session_id.clone(),
        attachment_id: attachment.id().to_string(),
        target_agent_id: Some(spawned_agent_id.clone()),
        prompt: "cancel owner projection".to_string(),
        attachments: Vec::new(),
    });
    let prompt_command =
        KernelCommand::from_local_request("cmd-prompt-cancel-owner", None, None, &prompt_request);
    router
        .dispatch(prompt_command, prompt_request)
        .await
        .expect("prompt submit should warm active prompt projection");
    router.session_projection.update(idle_session_snapshot);

    let app_guard = app.lock().await;
    let cancel_request = LocalDaemonRequest::CancelActivePrompt(CancelActivePromptRequest {
        session_id: session_id.clone(),
        attachment_id: attachment.id().to_string(),
        target_agent_id: None,
    });
    let cancel_command = KernelCommand::from_local_request(
        "cmd-cancel-owner-projection",
        None,
        None,
        &cancel_request,
    );
    let cancel_router = router.clone();
    let cancel_task =
        tokio::spawn(async move { cancel_router.dispatch(cancel_command, cancel_request).await });

    let mut spawned_agent_lane_created = false;
    for _ in 0..50 {
        let projection = router.daemon_health_projection(0).await;
        if projection
            .agent_command_lanes
            .iter()
            .any(|lane| lane.lane_id == spawned_agent_id)
        {
            spawned_agent_lane_created = true;
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        spawned_agent_lane_created,
        "prompt cancel should resolve the active prompt owner from the agent-runtime projection before touching the app lock"
    );
    assert!(
        !cancel_task.is_finished(),
        "agent worker should still wait on the deliberately held app lock"
    );

    drop(app_guard);
    let cancel_response = cancel_task
        .await
        .expect("cancel task should join")
        .expect("prompt should cancel");
    match cancel_response {
        LocalDaemonResponse::PromptCancelled { cancellation } => {
            assert_eq!(cancellation.prompt.status(), PromptStatus::Cancelling);
        }
        _ => panic!("unexpected cancel response"),
    }
}
