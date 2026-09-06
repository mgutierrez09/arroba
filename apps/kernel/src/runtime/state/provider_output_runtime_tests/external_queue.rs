use super::*;

#[tokio::test]
async fn unavailable_provider_account_defers_explicit_queue_advances_without_losing_work() {
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-unavailable-account-queue",
            "worktree-unavailable-account-queue",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "unavailable-account-queue-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("client should attach");
    let account = app
        .provider_account_profile_registry()
        .create_managed(
            crate::session::DEFAULT_LOCAL_USER_ID,
            "codex",
            "Queued work account",
        )
        .expect("account profile should register");
    crate::test_support::authenticate_provider_account(
        &app.provider_account_profile_registry(),
        crate::session::DEFAULT_LOCAL_USER_ID,
        "codex",
        &account.profile_id,
    )
    .expect("account should start authenticated");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .update_agent_profile(
            session.id(),
            agent.id(),
            crate::session::DEFAULT_LOCAL_USER_ID,
            Some("codex".to_string()),
            Some(account.profile_id.clone()),
            Some("gpt-5.4".to_string()),
            Some(Some("low".to_string())),
        )
        .await
        .expect("authenticated account should be assigned");
    let (provider_run_id, queued_prompt_id) = {
        let mut app = app.lock().await;
        let run = app
            .launch_provider(
                crate::provider::LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "codex",
                    &account.profile_id,
                    "gpt-5.4",
                )
                .with_agent_id(agent.id()),
            )
            .expect("provider fixture should launch");
        app.update_provider_run_projection(run.clone());
        let queued = crate::session::PromptQueueItem::new(
            app.sessions_mut().reserve_prompt_id(),
            attachment.id(),
            agent.id(),
            "preserve this queued prompt",
            crate::session::PromptStatus::Queued,
        );
        let crate::session::PromptSubmissionOutcome::Queued { prompt } = app
            .prompt_owner_submit_prepared_prompt(session.id(), queued, true)
            .expect("prompt should queue")
        else {
            panic!("forced queue submission should remain queued");
        };
        (run.id().to_string(), prompt.id().to_string())
    };
    app.lock()
        .await
        .provider_account_profile_registry()
        .update_observation(
            crate::session::DEFAULT_LOCAL_USER_ID,
            "codex",
            &account.profile_id,
            crate::account_profile::ProviderAccountAuthState::Expired,
            None,
            None,
            None,
            None,
        )
        .expect("account should expire before queue advancement");

    assert!(runtime
        .owned
        .activate_next_queued_prompt_for_agent(session.id(), agent.id(), None)
        .expect("unavailable account should defer direct queue activation")
        .is_none());
    assert!(runtime
        .owned
        .advance_next_queued_prompt_dispatch(session.id(), agent.id(), &provider_run_id)
        .expect("unavailable account should defer provider queue dispatch")
        .is_none());
    app.lock()
        .await
        .provider_account_profile_registry()
        .remove_registration(
            crate::session::DEFAULT_LOCAL_USER_ID,
            "codex",
            &account.profile_id,
        )
        .expect("fixture should simulate a missing bound account");
    assert!(runtime
        .owned
        .activate_next_queued_prompt_for_agent(session.id(), agent.id(), None)
        .expect("missing account should fail closed and defer queue activation")
        .is_none());
    assert!(runtime
        .owned
        .advance_next_queued_prompt_dispatch(session.id(), agent.id(), &provider_run_id)
        .expect("missing account should fail closed and defer provider dispatch")
        .is_none());

    let snapshot = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should remain available");
    assert!(snapshot.active_prompt_for_agent(agent.id()).is_none());
    let queued = snapshot
        .queued_prompts_for_agent(agent.id())
        .expect("queued work should remain durable");
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].id(), queued_prompt_id);
}

#[tokio::test]
async fn completed_metaagent_task_starts_queued_task_despite_stale_session_prompt_mirror() {
    let mut app =
        DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-metaagent-fifo",
            "worktree-metaagent-fifo",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "metaagent-fifo-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("client should attach");
    app.sessions_mut()
        .start_or_update_metaagent_task(session.id(), agent.id(), "first Meta task")
        .expect("first Meta task should start");
    app.sessions_mut()
        .complete_metaagent_task(session.id(), agent.id(), Some("done".to_string()))
        .expect("first Meta task should complete");
    let queued = app
        .sessions_mut()
        .enqueue_metaagent_task(
            session.id(),
            agent.id(),
            attachment.id(),
            "second Meta task",
            Vec::new(),
        )
        .expect("second Meta task should queue");
    app.sessions_mut()
        .submit_prompt(
            session.id(),
            attachment.id(),
            agent.id(),
            "stale completed prompt mirror",
            Vec::new(),
        )
        .expect("legacy session mirror should contain a stale active prompt");
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;

    let dispatches = runtime
        .owned
        .workflow_maybe_start_next_queued_prompt(session.id());

    assert_eq!(dispatches.starting_metaagent_tasks.len(), 1);
    assert_eq!(dispatches.starting_metaagent_tasks[0].id(), queued.id());
}

#[tokio::test]
async fn paused_workflow_prompt_cannot_be_promoted_after_provider_launch() {
    let mut app =
        DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-paused-workflow-queue",
            "worktree-paused-workflow-queue",
        ))
        .expect("session should be created");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider run should launch");
    app.update_provider_run_projection(run.clone());
    let workflow = app
        .sessions_mut()
        .create_workflow(session.id(), Some("paused-queue".to_string()))
        .expect("workflow should be created");
    let node = app
        .sessions_mut()
        .add_workflow_node(session.id(), workflow.id(), agent.id())
        .expect("workflow node should be added");
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");
    let workflow_run = app
        .sessions_mut()
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("run once".to_string()),
        )
        .expect("workflow run should be created");
    let node_run_id = workflow_run.node_runs()[0].id().to_string();
    app.sessions_mut()
        .prepare_workflow_turn(
            session.id(),
            workflow_run.id(),
            &node_run_id,
            format!("workflow-ack:{node_run_id}"),
            "queued workflow turn".to_string(),
            None,
            None,
        )
        .expect("workflow turn should be prepared");
    let queued = crate::session::PromptQueueItem::new(
        "pending-paused-workflow",
        crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
        agent.id(),
        "queued workflow turn",
        crate::session::PromptStatus::Queued,
    )
    .with_workflow_context(workflow_run.id(), &node_run_id);
    let crate::session::PromptSubmissionOutcome::Queued { .. } = app
        .prompt_owner_submit_prepared_prompt(session.id(), queued, true)
        .expect("workflow prompt should remain queued while provider launch settles")
    else {
        panic!("workflow prompt should be queued");
    };
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .execute_workflow_interrupt_run(session.id(), workflow_run.id(), true)
        .await
        .expect("workflow should pause through the authoritative runtime path");
    let dispatch = runtime
        .owned
        .advance_next_queued_prompt_dispatch(session.id(), agent.id(), run.id())
        .expect("paused workflow queue cleanup should not fail");

    assert!(
        dispatch.is_none(),
        "a prompt owned by a paused workflow must never reach the provider"
    );
    let snapshot = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should remain available");
    assert!(snapshot.active_prompt_for_agent(agent.id()).is_none());
    assert!(
        snapshot
            .queued_prompts_for_agent(agent.id())
            .is_none_or(|queued| queued.is_empty()),
        "the stale paused-workflow prompt should be removed from the authoritative queue"
    );
}

#[tokio::test]
async fn external_active_prompt_blocks_queue_until_observer_settles_it() {
    let mut app =
        DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-external-queue-settlement",
            "worktree-external-queue-settlement",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-external-queue-settlement",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider run should launch");
    app.update_provider_run_projection(run.clone());
    let (external_prompt_id, queued_prompt_id) =
        sync_external_active_prompt_and_queue_chariox_prompt(
            &mut app,
            session.id(),
            attachment.id(),
            agent.id(),
        );

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    assert_external_active_prompt_and_queued_chariox_prompt(
        &runtime,
        session.id(),
        agent.id(),
        &external_prompt_id,
        &queued_prompt_id,
    );

    let blocked = runtime
        .owned
        .advance_next_queued_prompt_dispatch(session.id(), agent.id(), run.id())
        .expect("active external prompt should not error while blocking queue dispatch");
    assert!(
        blocked.is_none(),
        "queued Chariox prompt must not dispatch while external prompt is active"
    );

    {
        let mut app = app.lock().await;
        let changed = app
            .prompt_owner_sync_external_active_prompt(session.id(), agent.id(), None)
            .expect("observer settlement should clear external active prompt");
        assert!(changed);
    }

    let dispatch = runtime
        .owned
        .advance_next_queued_prompt_dispatch(session.id(), agent.id(), run.id())
        .expect("settled external prompt should release queued prompt")
        .expect("queued prompt should dispatch after external settlement");
    assert_eq!(dispatch.session_id, session.id());
    assert_eq!(dispatch.provider_run_id, run.id());
    assert_eq!(dispatch.agent_id, agent.id());
    assert_eq!(dispatch.prompt, "queued from Chariox\n");
    assert!(!dispatch.steering);

    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    let active_prompt = session_state
        .active_prompt_for_agent(agent.id())
        .expect("queued prompt should now be active");
    assert_eq!(active_prompt.id(), dispatch.prompt_id);
    assert_eq!(
        active_prompt.prompt_origin(),
        crate::session::PromptOrigin::Chariox
    );
    assert_eq!(active_prompt.prompt(), "queued from Chariox\n");
    assert!(
        session_state
            .queued_prompts_for_agent(agent.id())
            .map(|queued| queued.is_empty())
            .unwrap_or(true),
        "queue should be empty after deterministic promotion"
    );
}

#[tokio::test]
async fn external_active_prompt_rejects_queued_prompt_steering() {
    let mut app =
        DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-external-steering",
            "worktree-external-steering",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-external-steering",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider run should launch");
    app.update_provider_run_projection(run);
    let (external_prompt_id, queued_prompt_id) =
        sync_external_active_prompt_and_queue_chariox_prompt(
            &mut app,
            session.id(),
            attachment.id(),
            agent.id(),
        );

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    assert_external_active_prompt_and_queued_chariox_prompt(
        &runtime,
        session.id(),
        agent.id(),
        &external_prompt_id,
        &queued_prompt_id,
    );

    let error = match runtime.owned.steer_queued_prompt(
        session.id(),
        agent.id(),
        attachment.id(),
        &queued_prompt_id,
    ) {
        Ok(_) => panic!("external active prompt should reject steering"),
        Err(error) => error,
    };
    match error {
        DaemonError::LocalTransport { operation, message } => {
            assert_eq!(operation, "steer queued prompt");
            assert_eq!(
                message,
                "queued prompts cannot be steered into externally started provider turns"
            );
        }
        other => panic!("expected LocalTransport steering error, got {other:?}"),
    }

    assert_external_active_prompt_and_queued_chariox_prompt(
        &runtime,
        session.id(),
        agent.id(),
        &external_prompt_id,
        &queued_prompt_id,
    );
}
