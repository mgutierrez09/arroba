use super::*;
use crate::local::{GetSessionHistoryOutlineRequest, UpdateQueuedPromptRequest};

fn run_async_with_large_test_stack<F, Fut>(name: &'static str, test: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap_or_else(|error| panic!("{name} tokio runtime should build: {error}"))
                .block_on(test());
        })
        .unwrap_or_else(|error| panic!("{name} test thread should spawn: {error}"))
        .join()
        .unwrap_or_else(|error| std::panic::resume_unwind(error));
}

#[tokio::test]
async fn pending_provider_launch_cleanup_does_not_wait_for_app_lock_when_projection_is_cold() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    router
        .provider_launch_pending
        .insert_for_tests("cold-session")
        .await;

    let app_guard = app.lock().await;
    let cleanup_router = router.clone();
    let cleanup_task = tokio::spawn(async move {
        cleanup_router
            .provider_launch_pending
            .clear_if_settled(
                &cleanup_router.app,
                "cold-session",
                &cleanup_router.session_projection,
                &cleanup_router.provider_run_projection,
            )
            .await;
    });

    timeout(Duration::from_millis(100), cleanup_task)
        .await
        .expect("cold pending launch cleanup should not wait for the app lock")
        .expect("cleanup task should join");
    drop(app_guard);

    assert!(
        router
            .provider_launch_pending
            .contains_for_tests("cold-session")
            .await,
        "cold cleanup should leave the guard for a later projection-backed refresh"
    );
}

#[tokio::test]
async fn routes_interactive_commands_through_bounded_lane() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let request = LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
        session_id: session_id.clone(),
        client_id: "cli-1".to_string(),
        capability_level: ClientCapabilityLevel::FullTerminal,
    });
    let command = KernelCommand::from_local_request("cmd-1", None, None, &request);

    let response = router
        .dispatch(command, request)
        .await
        .expect("command should run");

    assert!(matches!(
        response,
        crate::local::LocalDaemonResponse::SessionAttached { .. }
    ));
}

#[tokio::test]
async fn queued_prompt_cancel_routes_through_interactive_dispatch_and_removes_strip_state() {
    let fixture = queued_prompt_router_fixture("cancel");
    let request = LocalDaemonRequest::CancelQueuedPrompt(CancelQueuedPromptRequest {
        session_id: fixture.session_id.clone(),
        attachment_id: fixture.attachment_id.clone(),
        target_agent_id: fixture.agent_id.clone(),
        prompt_id: fixture.queued_prompt_id.clone(),
    });
    let command =
        KernelCommand::from_local_request("cmd-cancel-queued-prompt", None, None, &request);

    let response = fixture
        .router
        .dispatch(command, request)
        .await
        .expect("queued prompt cancel should route through interactive dispatch");

    let LocalDaemonResponse::QueuedPromptCancelled {
        prompt,
        session,
        agent_activity,
        ..
    } = response
    else {
        panic!("unexpected queued prompt cancel response");
    };
    assert_eq!(prompt.id(), fixture.queued_prompt_id);
    assert_eq!(prompt.status(), PromptStatus::Cancelled);
    assert!(agent_activity.contains_key(&fixture.agent_id));
    assert_eq!(
        session
            .active_prompt_for_agent(&fixture.agent_id)
            .map(|prompt| prompt.id()),
        Some(fixture.active_prompt_id.as_str())
    );
    assert!(
        session
            .queued_prompts_for_agent(&fixture.agent_id)
            .map(|queued| queued.is_empty())
            .unwrap_or(true),
        "cancelled queued prompt should disappear from prompt state"
    );
}

#[tokio::test]
async fn queued_user_prompt_update_routes_through_interactive_dispatch() {
    let fixture = queued_prompt_router_fixture("update");
    let request = LocalDaemonRequest::UpdateQueuedPrompt(UpdateQueuedPromptRequest {
        session_id: fixture.session_id.clone(),
        attachment_id: fixture.attachment_id.clone(),
        target_agent_id: fixture.agent_id.clone(),
        prompt_id: fixture.queued_prompt_id.clone(),
        prompt: "updated queued user prompt".to_string(),
    });
    let command =
        KernelCommand::from_local_request("cmd-update-queued-prompt", None, None, &request);

    let response = fixture
        .router
        .dispatch(command, request)
        .await
        .expect("queued user prompt update should route through interactive dispatch");

    let LocalDaemonResponse::QueuedPromptUpdated {
        prompt, session, ..
    } = response
    else {
        panic!("unexpected queued prompt update response");
    };
    assert_eq!(prompt.id(), fixture.queued_prompt_id);
    assert_eq!(prompt.prompt(), "updated queued user prompt");
    assert_eq!(
        session
            .queued_prompts_for_agent(&fixture.agent_id)
            .and_then(|queued| queued.front())
            .map(|queued| queued.prompt()),
        Some("updated queued user prompt")
    );
}

#[tokio::test]
async fn queued_prompt_steer_routes_through_interactive_dispatch_and_removes_strip_state() {
    let fixture = queued_prompt_router_fixture("steer");
    let request = LocalDaemonRequest::SteerQueuedPrompt(SteerQueuedPromptRequest {
        session_id: fixture.session_id.clone(),
        attachment_id: fixture.attachment_id.clone(),
        target_agent_id: fixture.agent_id.clone(),
        prompt_id: fixture.queued_prompt_id.clone(),
    });
    let command =
        KernelCommand::from_local_request("cmd-steer-queued-prompt", None, None, &request);

    let response = fixture
        .router
        .dispatch(command, request)
        .await
        .expect("queued prompt steer should route through interactive dispatch");

    let LocalDaemonResponse::QueuedPromptSteered {
        prompt, session, ..
    } = response
    else {
        panic!("unexpected queued prompt steer response");
    };
    assert_eq!(prompt.id(), fixture.queued_prompt_id);
    assert_eq!(
        session
            .active_prompt_for_agent(&fixture.agent_id)
            .map(|prompt| prompt.id()),
        Some(fixture.active_prompt_id.as_str())
    );
    assert!(
        session
            .queued_prompts_for_agent(&fixture.agent_id)
            .map(|queued| queued.is_empty())
            .unwrap_or(true),
        "steered queued prompt should disappear from prompt state"
    );
}

#[tokio::test]
async fn queued_prompt_steer_rejects_external_active_prompt() {
    let fixture =
        queued_prompt_router_fixture_with_origin("external-steer", PromptOrigin::External);
    let request = LocalDaemonRequest::SteerQueuedPrompt(SteerQueuedPromptRequest {
        session_id: fixture.session_id.clone(),
        attachment_id: fixture.attachment_id.clone(),
        target_agent_id: fixture.agent_id.clone(),
        prompt_id: fixture.queued_prompt_id.clone(),
    });
    let command =
        KernelCommand::from_local_request("cmd-steer-external-active-prompt", None, None, &request);

    let error = fixture
        .router
        .dispatch(command, request)
        .await
        .expect_err("external active prompt should reject steering");

    match error {
        DaemonError::LocalTransport { operation, message } => {
            assert_eq!(operation, "steer queued prompt");
            assert!(
                message.contains("externally started provider turns"),
                "unexpected error message: {message}"
            );
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn queued_workflow_prompt_rejects_manual_update_and_cancel() {
    let update_fixture = queued_workflow_prompt_router_fixture("workflow-update");
    let update_request = LocalDaemonRequest::UpdateQueuedPrompt(UpdateQueuedPromptRequest {
        session_id: update_fixture.session_id.clone(),
        attachment_id: update_fixture.attachment_id.clone(),
        target_agent_id: update_fixture.agent_id.clone(),
        prompt_id: update_fixture.queued_prompt_id.clone(),
        prompt: "manually rewritten workflow turn".to_string(),
    });
    let update_command = KernelCommand::from_local_request(
        "cmd-update-workflow-prompt",
        None,
        None,
        &update_request,
    );
    let update_error = update_fixture
        .router
        .dispatch(update_command, update_request)
        .await
        .expect_err("workflow queued prompt should reject manual update");
    assert!(update_error
        .to_string()
        .contains("workflow queued prompts cannot be updated manually"));

    let cancel_fixture = queued_workflow_prompt_router_fixture("workflow-cancel");
    let cancel_request = LocalDaemonRequest::CancelQueuedPrompt(CancelQueuedPromptRequest {
        session_id: cancel_fixture.session_id.clone(),
        attachment_id: cancel_fixture.attachment_id.clone(),
        target_agent_id: cancel_fixture.agent_id.clone(),
        prompt_id: cancel_fixture.queued_prompt_id.clone(),
    });
    let cancel_command = KernelCommand::from_local_request(
        "cmd-cancel-workflow-prompt",
        None,
        None,
        &cancel_request,
    );
    let cancel_error = cancel_fixture
        .router
        .dispatch(cancel_command, cancel_request)
        .await
        .expect_err("workflow queued prompt should reject manual cancellation");
    assert!(cancel_error
        .to_string()
        .contains("workflow queued prompts cannot be cancelled manually"));
}

#[tokio::test]
async fn rejects_session_commands_when_bounded_lane_is_full() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_and_session_capacity(Arc::clone(&app), 1, 1);
    let app_guard = app.lock().await;

    let first_request = attach_request(&session_id, "cli-1");
    let first_result_rx = router
        .session_runtime
        .enqueue_for_test(&session_id, "cmd-1", "session.attach", first_request)
        .await
        .expect("first command should enter the session lane");

    let mut first_command_is_running = false;
    for _ in 0..50 {
        if router
            .session_runtime
            .lane_capacity(&session_id)
            .await
            .is_some_and(|capacity| capacity == 1)
        {
            first_command_is_running = true;
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        first_command_is_running,
        "first session command should be running before filling the queue"
    );

    let queued_request = attach_request(&session_id, "queued-cli");
    let queued_result_rx = router
        .session_runtime
        .enqueue_for_test(&session_id, "cmd-queued", "session.attach", queued_request)
        .await
        .expect("queued command should fill the session lane");

    let mut session_lane_is_full = false;
    for _ in 0..50 {
        if router
            .session_runtime
            .lane_capacity(&session_id)
            .await
            .is_some_and(|capacity| capacity == 0)
        {
            session_lane_is_full = true;
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        session_lane_is_full,
        "session command queue should be full before overflow dispatch"
    );

    let third_request = attach_request(&session_id, "cli-overflow");
    let third_command =
        KernelCommand::from_local_request("cmd-overflow", None, None, &third_request);
    let error = router
        .dispatch(third_command, third_request)
        .await
        .expect_err("overflow session command should be rejected while lane is full");
    assert!(error
        .to_string()
        .contains("session command lane overloaded"));

    drop(app_guard);
    let _ = first_result_rx.await.expect("first result should resolve");
    let _ = queued_result_rx
        .await
        .expect("queued result should resolve");
}

struct QueuedPromptRouterFixture {
    router: CommandRouter,
    session_id: String,
    agent_id: String,
    attachment_id: String,
    active_prompt_id: String,
    queued_prompt_id: String,
}

fn queued_prompt_router_fixture(label: &str) -> QueuedPromptRouterFixture {
    queued_prompt_router_fixture_with_options(label, PromptOrigin::Chariox, false)
}

fn queued_workflow_prompt_router_fixture(label: &str) -> QueuedPromptRouterFixture {
    queued_prompt_router_fixture_with_options(label, PromptOrigin::Chariox, true)
}

fn queued_prompt_router_fixture_with_origin(
    label: &str,
    active_prompt_origin: PromptOrigin,
) -> QueuedPromptRouterFixture {
    queued_prompt_router_fixture_with_options(label, active_prompt_origin, false)
}

fn queued_prompt_router_fixture_with_options(
    label: &str,
    active_prompt_origin: PromptOrigin,
    workflow_owned_queued_prompt: bool,
) -> QueuedPromptRouterFixture {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            &format!("workspace-queued-{label}"),
            &format!("worktree-queued-{label}"),
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            &format!("client-queued-{label}"),
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let _provider_run = launch_test_provider(
        &mut app,
        session.id(),
        agent.id(),
        "dev-stub",
        "claude-code",
        "sonnet",
    );
    let active_prompt = PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "active prompt",
        PromptStatus::Queued,
    )
    .with_prompt_origin(active_prompt_origin);
    let PromptSubmissionOutcome::Started {
        prompt: active_prompt,
    } = app
        .prompt_owner_submit_prepared_prompt(session.id(), active_prompt, false)
        .expect("active prompt should start")
    else {
        panic!("first prompt should start");
    };
    let mut queued_prompt = PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "queued prompt",
        PromptStatus::Queued,
    );
    if workflow_owned_queued_prompt {
        queued_prompt = queued_prompt.with_workflow_context("workflow-run-1", "node-run-1");
    }
    let PromptSubmissionOutcome::Queued {
        prompt: queued_prompt,
    } = app
        .prompt_owner_submit_prepared_prompt(session.id(), queued_prompt, false)
        .expect("second prompt should queue")
    else {
        panic!("second prompt should queue");
    };
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment_id = attachment.id().to_string();
    let active_prompt_id = active_prompt.id().to_string();
    let queued_prompt_id = queued_prompt.id().to_string();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);

    QueuedPromptRouterFixture {
        router,
        session_id,
        agent_id,
        attachment_id,
        active_prompt_id,
        queued_prompt_id,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn prompt_submit_does_not_wait_behind_slow_history_load() {
    let mut config = DaemonConfig::for_tests();
    config.operational_history_read_delay_ms = 120;
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-slow-history",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    app.history_store()
        .append(
            &session,
            &crate::history::SessionHistoryEntry::user_prompt(
                &session_id,
                attachment.id(),
                &agent_id,
                "slow history entry",
            ),
        )
        .expect("legacy-only history should append");
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
    let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
        session_id: session_id.clone(),
    });
    let state_command =
        KernelCommand::from_local_request("cmd-history-prompt-state", None, None, &state_request);
    router
        .dispatch(state_command, state_request)
        .await
        .expect("state read should warm session projection");

    let history_request =
        LocalDaemonRequest::GetSessionHistoryOutline(GetSessionHistoryOutlineRequest {
            session_id: session_id.clone(),
            agent_ids: Some(vec![agent_id.clone()]),
            latest_prompt_count: Some(4),
            cursor: None,
        });
    let history_command = KernelCommand::from_local_request(
        "cmd-history-slow-background",
        None,
        None,
        &history_request,
    );
    let history_router = router.clone();
    let mut history_task = tokio::spawn(async move {
        history_router
            .dispatch(history_command, history_request)
            .await
    });
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(
        !history_task.is_finished(),
        "test setup should keep history loading in the background"
    );

    let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session_id.clone(),
        attachment_id: attachment.id().to_string(),
        target_agent_id: Some(agent_id.clone()),
        prompt: "submit while history is slow".to_string(),
        attachments: Vec::new(),
    });
    let prompt_command =
        KernelCommand::from_local_request("cmd-prompt-during-history", None, None, &prompt_request);
    let prompt_dispatch = router.dispatch(prompt_command, prompt_request);
    tokio::pin!(prompt_dispatch);
    let prompt_response = timeout(Duration::from_secs(2), async {
        tokio::select! {
            prompt_response = &mut prompt_dispatch => {
                prompt_response.expect("prompt submit should succeed")
            }
            history_response = &mut history_task => {
                panic!(
                    "prompt submit waited behind slow history; history resolved first: {history_response:?}"
                );
            }
        }
    })
    .await
    .expect("prompt submit should not stall while history loads");
    assert!(matches!(
        prompt_response,
        LocalDaemonResponse::PromptSubmitted { .. }
    ));

    let _ = history_task
        .await
        .expect("history task should join")
        .expect("history should eventually resolve");
}

#[test]
fn focus_resize_and_cancel_do_not_wait_behind_slow_provider_catalog() {
    run_async_with_large_test_stack(
        "focus-resize-cancel-slow-provider-catalog",
        focus_resize_and_cancel_do_not_wait_behind_slow_provider_catalog_inner,
    );
}

async fn focus_resize_and_cancel_do_not_wait_behind_slow_provider_catalog_inner() {
    let mut config = DaemonConfig::for_tests();
    // Assert independence from catalog discovery, not a sub-second CI disk/scheduler SLA.
    // The catalog delay exceeds the deadlock budget; each response must also arrive
    // before discovery completes, so a serialized command cannot pass this drill.
    const CATALOG_DELAY: Duration = Duration::from_secs(3);
    const INTERACTIVE_BUDGET: Duration = Duration::from_secs(2);
    config.provider_catalog_read_delay_ms = CATALOG_DELAY.as_millis() as u64;
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-slow-catalog",
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
        prompt: "prompt to cancel while catalog is slow".to_string(),
        attachments: Vec::new(),
    });
    let prompt_command =
        KernelCommand::from_local_request("cmd-catalog-prompt", None, None, &prompt_request);
    router
        .dispatch(prompt_command, prompt_request)
        .await
        .expect("prompt should start before catalog drill");

    let catalog_request =
        LocalDaemonRequest::GetProviderCatalog(GetProviderCatalogRequest::default());
    let catalog_command =
        KernelCommand::from_local_request("cmd-slow-catalog", None, None, &catalog_request);
    let catalog_router = router.clone();
    let catalog_task = tokio::spawn(async move {
        catalog_router
            .dispatch(catalog_command, catalog_request)
            .await
    });
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(
        !catalog_task.is_finished(),
        "test setup should keep provider catalog discovery in the background"
    );

    let focus_request = focus_request(&session_id, &agent_id);
    let focus_command =
        KernelCommand::from_local_request("cmd-focus-during-catalog", None, None, &focus_request);
    let focus_response = timeout(
        INTERACTIVE_BUDGET,
        router.dispatch(focus_command, focus_request),
    )
    .await
    .expect("focus should not wait behind slow catalog")
    .expect("focus should succeed");
    assert!(matches!(
        focus_response,
        LocalDaemonResponse::AgentFocused { .. }
    ));
    assert!(
        !catalog_task.is_finished(),
        "focus must finish while catalog discovery is still pending"
    );

    let resize_request = LocalDaemonRequest::ResizeTerminal(ResizeTerminalRequest {
        session_id: session_id.clone(),
        provider_run_id: None,
        cols: 120,
        rows: 40,
    });
    let resize_command =
        KernelCommand::from_local_request("cmd-resize-during-catalog", None, None, &resize_request);
    let resize_response = timeout(
        INTERACTIVE_BUDGET,
        router.dispatch(resize_command, resize_request),
    )
    .await
    .expect("resize should not wait behind slow catalog")
    .expect("resize should succeed");
    assert!(matches!(
        resize_response,
        LocalDaemonResponse::TerminalResized { .. }
    ));
    assert!(
        !catalog_task.is_finished(),
        "resize must finish while catalog discovery is still pending"
    );

    let cancel_request = LocalDaemonRequest::CancelActivePrompt(CancelActivePromptRequest {
        session_id: session_id.clone(),
        attachment_id: attachment.id().to_string(),
        target_agent_id: None,
    });
    let cancel_command =
        KernelCommand::from_local_request("cmd-cancel-during-catalog", None, None, &cancel_request);
    let cancel_response = timeout(
        INTERACTIVE_BUDGET,
        router.dispatch(cancel_command, cancel_request),
    )
    .await
    .expect("cancel should not wait behind slow catalog")
    .expect("cancel should succeed");
    assert!(matches!(
        cancel_response,
        LocalDaemonResponse::PromptCancelled { .. }
    ));
    assert!(
        !catalog_task.is_finished(),
        "cancel must finish while catalog discovery is still pending"
    );

    let _ = catalog_task.await.expect("catalog task should join");
}
