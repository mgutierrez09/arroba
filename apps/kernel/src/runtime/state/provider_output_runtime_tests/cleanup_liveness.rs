use super::*;

#[tokio::test]
async fn cancellation_acknowledgement_does_not_fail_an_already_stopped_workflow() {
    assert_stopped_workflow_cancellation_acknowledgement(false).await;
}

#[tokio::test]
async fn cancellation_acknowledgement_preserves_existing_workflow_failures() {
    assert_stopped_workflow_cancellation_acknowledgement(true).await;
}

async fn assert_stopped_workflow_cancellation_acknowledgement(existing_failure: bool) {
    let mut app = DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "cancel-ack",
            "cancel-ack",
        ))
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "cancel-ack-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let workflow = app
        .sessions_mut()
        .create_workflow(session.id(), None)
        .unwrap();
    let node = app
        .sessions_mut()
        .add_workflow_node(session.id(), workflow.id(), agent.id())
        .unwrap();
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(session.id(), workflow.id(), node.id(), None)
        .unwrap();
    let run = app
        .sessions_mut()
        .invoke_workflow_endpoint(session.id(), workflow.id(), endpoint.id(), None)
        .unwrap();
    let prompt = crate::session::PromptQueueItem::new(
        "cancel-ack-prompt",
        attachment.id(),
        agent.id(),
        "cancel",
        crate::session::PromptStatus::Queued,
    )
    .with_workflow_context(run.id(), run.node_runs()[0].id());
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt.clone(), false)
        .unwrap();
    app.sessions_mut()
        .cancel_workflow_run(session.id(), run.id())
        .unwrap();
    if existing_failure {
        app.sessions_mut()
            .record_workflow_failure_event(
                session.id(),
                run.id(),
                crate::session::WorkflowFailureEvent::new(
                    crate::session::WorkflowFailureKind::ProviderFailure,
                    run.node_runs()[0].id(),
                    Vec::new(),
                    "provider failed before stop",
                ),
            )
            .unwrap();
    }
    let expected_failures = app
        .sessions()
        .resolve_workflow_run_ref(session.id(), run.id())
        .unwrap()
        .failure_events()
        .to_vec();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    for _ in 0..2 {
        runtime
            .owned
            .workflow_cancel_prompt(session.id(), &prompt)
            .unwrap();
        let mut app = app.lock().await;
        crate::app::workflow_runtime::cancel_workflow_prompt_from_runtime(
            &mut app,
            session.id(),
            &prompt,
        )
        .unwrap();
        let stopped = app
            .sessions()
            .resolve_workflow_run_ref(session.id(), run.id())
            .unwrap();
        assert_eq!(stopped.status(), crate::session::WorkflowRunStatus::Stopped);
        assert_eq!(
            stopped.failure_events(),
            expected_failures,
            "acknowledging an intentional stop must preserve failure events unchanged"
        );
    }
}

#[tokio::test]
async fn unexpected_owned_provider_exit_marks_active_agent_error() {
    assert_owned_provider_exit_state(false).await;
}

#[tokio::test]
async fn cancelled_owned_provider_exit_does_not_mark_agent_error() {
    assert_owned_provider_exit_state(true).await;
}

async fn assert_owned_provider_exit_state(cancelling: bool) {
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::DaemonConfig::for_tests())
            .expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-unexpected-exit",
            "worktree-unexpected-exit",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-unexpected-exit",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "codex",
                "default",
                "gpt-5",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider should launch");
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(agent.id()),
        "do work\n",
        Vec::new(),
    )
    .expect("prompt should start");
    if cancelling {
        app.prompt_owner_begin_cancelling_active_prompt(session.id(), agent.id())
            .expect("cancellation should be recorded before provider exit");
    }
    let ended = app
        .providers_mut()
        .mark_run_ended_provider_only(session.id(), run.id())
        .expect("provider run should end")
        .into_run();
    app.update_provider_run_projection(ended);

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let outcome = runtime
        .settle_unexpected_provider_run_exit(session.id(), run.id(), agent.id())
        .await
        .expect("unexpected provider exit should settle");

    assert!(outcome.had_active_prompt);
    assert_eq!(outcome.cancelled_prompt, cancelling);
    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(session_state.active_prompt_for_agent(agent.id()).is_none());
    let agent_state = runtime
        .owned
        .agent_store
        .get_agent(agent.id())
        .expect("agent should remain available")
        .state();
    if cancelling {
        assert_ne!(
            agent_state,
            crate::agent::AgentState::Error,
            "a deliberate cancellation must not become an unexpected provider failure"
        );
    } else {
        assert_eq!(agent_state, crate::agent::AgentState::Error);
    }
}

#[tokio::test]
async fn unexpected_owned_provider_exit_without_active_prompt_preserves_agent_state() {
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::DaemonConfig::for_tests())
            .expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-idle-exit",
            "worktree-idle-exit",
        ))
        .expect("session should be created");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "codex",
                "default",
                "gpt-5",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider should launch");
    let state_before = agent.state();
    let ended = app
        .providers_mut()
        .mark_run_ended_provider_only(session.id(), run.id())
        .expect("provider run should end")
        .into_run();
    app.update_provider_run_projection(ended);

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let outcome = runtime
        .settle_unexpected_provider_run_exit(session.id(), run.id(), agent.id())
        .await
        .expect("idle provider exit should settle");

    assert!(!outcome.had_active_prompt);
    assert_eq!(
        runtime
            .owned
            .agent_store
            .get_agent(agent.id())
            .expect("agent should remain available")
            .state(),
        state_before,
    );
}

#[tokio::test]
async fn owned_end_session_clears_stale_prompt_runtime_state_for_already_ended_session() {
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::DaemonConfig::for_tests())
            .expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let run = app
        .launch_provider(crate::provider::LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "codex",
            "default",
            "gpt-5",
        ))
        .expect("provider should launch");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .end_session(session.id())
        .await
        .expect("session should end once");
    {
        let app = app.lock().await;
        app.prompt_activity_store().write().insert(
            run.id().to_string(),
            crate::app::ActivePromptState {
                last_output_at: Some(Instant::now()),
                saw_response_content: true,
                completion_recorded: true,
                settlement_requested: true,
                active_tool_ids: std::collections::BTreeSet::new(),
            },
        );
        app.active_turn_store().start(
            crate::app::ActiveTurnState::new(
                session.id().to_string(),
                agent.id().to_string(),
                "prompt-stale".to_string(),
                run.id().to_string(),
            )
            .with_phase(crate::app::ActiveTurnPhase::Settling),
        );
    }

    runtime
        .end_session(session.id())
        .await
        .expect("already ended session should clean stale runtime state");

    let app = app.lock().await;
    assert!(
        !app.prompt_activity_store().read().contains_key(run.id()),
        "prompt activity should not survive already-ended session cleanup"
    );
    assert!(
        !app.active_turn_store().snapshot().contains_key(run.id()),
        "active turn should not survive already-ended session cleanup"
    );
}

#[tokio::test]
async fn owned_liveness_reconciliation_settles_already_ended_active_prompt() {
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::DaemonConfig::for_tests())
            .expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-1",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "codex",
                "default",
                "gpt-5",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider should launch");
    app.update_provider_run_projection(run.clone());
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(agent.id()),
        "do work\n",
        Vec::new(),
    )
    .expect("prompt should start");
    crate::transport::flow_control::note_prompt_started(&mut app, run.id());
    let ended = app
        .providers_mut()
        .mark_run_ended_provider_only(session.id(), run.id())
        .expect("provider run should be marked ended")
        .into_run();
    app.update_provider_run_projection(ended);

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let already_ended = runtime
        .reconcile_provider_run_exit(session.id(), run.id())
        .await
        .expect("already-ended liveness reconciliation should succeed");

    assert!(already_ended);
    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(
        session_state.active_prompt_for_agent(agent.id()).is_none(),
        "already-ended provider reconciliation should close the active prompt"
    );
    let app = app.lock().await;
    assert!(
        !app.prompt_activity_store().read().contains_key(run.id()),
        "already-ended provider reconciliation should clear prompt activity"
    );
    assert!(
        !app.active_turn_store().snapshot().contains_key(run.id()),
        "already-ended provider reconciliation should clear active turn state"
    );
    assert_ne!(
        app.agents()
            .get_agent(agent.id())
            .expect("agent should remain available")
            .state(),
        crate::agent::AgentState::Error,
        "already-ended reconciliation must not classify the agent as a new failure",
    );
}

#[tokio::test]
async fn stale_provider_exit_does_not_settle_prompt_on_replacement_run() {
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::DaemonConfig::for_tests())
            .expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-1",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let stale_run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "codex",
                "default",
                "gpt-5",
            )
            .with_agent_id(agent.id()),
        )
        .expect("initial provider should launch");
    let ended = app
        .providers_mut()
        .mark_run_ended_provider_only(session.id(), stale_run.id())
        .expect("initial provider should end")
        .into_run();
    app.update_provider_run_projection(ended);
    let replacement_run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "codex",
                "default",
                "gpt-5",
            )
            .with_agent_id(agent.id()),
        )
        .expect("replacement provider should launch");
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(agent.id()),
        "continue on the replacement\n",
        Vec::new(),
    )
    .expect("replacement prompt should start");
    let prompt_id = app
        .sessions()
        .get_session(session.id())
        .expect("session should exist")
        .active_prompt_for_agent(agent.id())
        .expect("replacement prompt should remain active")
        .id()
        .to_string();
    app.active_turn_store()
        .start(crate::app::ActiveTurnState::new(
            session.id().to_string(),
            agent.id().to_string(),
            prompt_id,
            replacement_run.id().to_string(),
        ));

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let already_ended = runtime
        .reconcile_provider_run_exit(session.id(), stale_run.id())
        .await
        .expect("stale provider reconciliation should succeed");

    assert!(already_ended);
    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(
        session_state.active_prompt_for_agent(agent.id()).is_some(),
        "stale provider reconciliation must not settle the replacement prompt"
    );
    assert!(
        runtime
            .owned
            .active_turns
            .snapshot()
            .contains_key(replacement_run.id()),
        "replacement active turn must remain tracked"
    );
}

#[tokio::test]
async fn stale_provider_exit_preserves_starting_cross_agent_workflow_handoff() {
    let mut app =
        DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, focused_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-cross-agent-handoff",
            "worktree-cross-agent-handoff",
        ))
        .expect("session should be created");
    let stale_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub").with_alias("stale"),
        )
        .expect("stale agent should spawn");
    let downstream_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("downstream"),
        )
        .expect("downstream agent should spawn");
    crate::app::KernelSessionService::new(&mut app)
        .focus_agent(session.id(), focused_agent.id())
        .expect("first agent should remain focused");

    let focused_run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "default",
            )
            .with_agent_id(focused_agent.id()),
        )
        .expect("focused provider should launch");
    let parked_focused_run = app
        .providers_mut()
        .park_run_provider_only(session.id(), focused_run.id())
        .expect("focused provider should park")
        .into_run();
    app.update_provider_run_projection(parked_focused_run);

    let stale_run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "default",
            )
            .with_agent_id(stale_agent.id()),
        )
        .expect("stale provider should launch");
    let ended_stale_run = app
        .providers_mut()
        .mark_run_ended_provider_only(session.id(), stale_run.id())
        .expect("stale provider should end")
        .into_run();
    app.update_provider_run_projection(ended_stale_run);
    app.sessions_mut()
        .set_active_provider_run(session.id(), None)
        .expect("ended stale provider should no longer be active");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let downstream_run = runtime
        .owned
        .start_provider_launch(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "default",
            )
            .with_agent_id(downstream_agent.id()),
        )
        .expect("downstream provider launch should start")
        .run;
    runtime
        .owned
        .provider_run_projection
        .update(downstream_run.clone());

    let already_ended = runtime
        .reconcile_provider_run_exit(session.id(), stale_run.id())
        .await
        .expect("stale provider reconciliation should succeed");

    assert!(already_ended);
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(downstream_run.id())
            .expect("downstream run should remain available")
            .state(),
        crate::provider::ProviderRunState::Starting,
        "stale settlement must not terminate a downstream provider launch",
    );
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(focused_run.id())
            .expect("focused run should remain available")
            .state(),
        crate::provider::ProviderRunState::Parked,
        "stale settlement must not resume the focused idle provider",
    );
    assert_eq!(
        runtime
            .owned
            .session_store
            .get_session(session.id())
            .expect("session should remain available")
            .active_provider_run_id(),
        Some(downstream_run.id()),
        "the downstream provider launch must remain active",
    );
}

#[tokio::test]
async fn owned_destroy_agent_clears_stale_prompt_runtime_state_for_ended_provider_runs() {
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::DaemonConfig::for_tests())
            .expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "codex",
                "default",
                "gpt-5",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider should launch");
    let ended = app
        .providers_mut()
        .terminate_run_provider_only(session.id(), run.id())
        .expect("provider run should end")
        .into_run();
    app.update_provider_run_projection(ended);
    app.prompt_activity_store().write().insert(
        run.id().to_string(),
        crate::app::ActivePromptState {
            last_output_at: Some(Instant::now()),
            saw_response_content: true,
            completion_recorded: true,
            settlement_requested: true,
            active_tool_ids: std::collections::BTreeSet::new(),
        },
    );
    app.active_turn_store().start(
        crate::app::ActiveTurnState::new(
            session.id().to_string(),
            agent.id().to_string(),
            "prompt-stale".to_string(),
            run.id().to_string(),
        )
        .with_phase(crate::app::ActiveTurnPhase::Settling),
    );

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .destroy_agent(agent.id(), crate::session::DEFAULT_LOCAL_USER_ID)
        .await
        .expect("agent should be destroyed");

    let app = app.lock().await;
    assert!(
        !app.prompt_activity_store().read().contains_key(run.id()),
        "destroying an agent should clear prompt activity for ended provider runs"
    );
    assert!(
        !app.active_turn_store().snapshot().contains_key(run.id()),
        "destroying an agent should clear active turns for ended provider runs"
    );
}
