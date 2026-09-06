use super::*;

#[tokio::test]
async fn codex_tool_output_text_does_not_classify_as_terminal_failure() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-tool-output",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt-5.5",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-codex",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-codex".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-codex-runtime".to_string()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());

    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "read tool output\n",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should start");
    crate::transport::flow_control::note_prompt_started(&mut app, run.id());

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let records = runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            crate::provider::ProviderPromptSignalBatch {
                chunks: vec![crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderTool,
                    merge_key: Some("tool-call-1".to_string()),
                    bytes: br#"{"tool":"bash","status":"completed","output":"Check if a CLAUDE.md file exists in the project root. If it does not exist, create it. This text mentions model context but is normal tool output."}"#.to_vec(),
                }],
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("structured tool output should be accepted");

    assert_eq!(records.len(), 1);
    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(
        session_state.active_prompt_for_agent(agent.id()).is_some(),
        "normal Codex tool output must not settle the active prompt"
    );
    let agent_activity = runtime
        .agent_activity_for_session(&session_state)
        .get(agent.id())
        .cloned()
        .expect("agent activity should be projected");
    assert!(agent_activity.busy);
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(run.id())
            .expect("provider run should exist")
            .terminal_diagnostic(),
        None
    );
}

#[tokio::test]
async fn structured_terminal_failure_settles_and_persists_single_provider_error() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-provider-error",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt-5.3-codex-spark",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-codex-error",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-codex".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-codex-runtime".to_string()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());

    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "trigger provider error\n",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should start");
    crate::transport::flow_control::note_prompt_started(&mut app, run.id());

    let raw_error = r#"{
  "type": "error",
  "error": {
    "type": "invalid_request_error",
    "code": "unsupported_parameter",
    "message": "Unsupported parameter: 'reasoning.summary' is not supported with the 'gpt-5.3-codex-spark' model.",
    "param": "reasoning.summary"
  },
  "status": 400
}"#;
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            crate::provider::ProviderPromptSignalBatch {
                notices: vec![raw_error.to_string(), raw_error.to_string()],
                terminal_failure: Some(raw_error.to_string()),
                prompt_completed: true,
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("structured terminal failure should be accepted");

    let records = runtime
        .owned
        .terminal_stream
        .drain_output_records(session.id(), attachment.id());
    let provider_errors = records
        .iter()
        .filter(|record| record.kind == crate::terminal::TerminalOutputKind::ProviderError)
        .collect::<Vec<_>>();
    assert_eq!(provider_errors.len(), 1);
    assert_eq!(
        String::from_utf8_lossy(&provider_errors[0].bytes),
        "Provider prompt dispatch failed: Unsupported parameter: 'reasoning.summary' is not supported with the 'gpt-5.3-codex-spark' model."
    );
    let durable_errors = runtime
        .owned
        .operational_history_store
        .load_session_events(session.id(), Some(agent.id()))
        .expect("canonical operational history should load")
        .into_iter()
        .filter(|event| {
            event.kind == crate::history::HistoryEventKind::ProviderError
                && event.content.as_deref()
                    == Some(
                        "Provider prompt dispatch failed: Unsupported parameter: 'reasoning.summary' is not supported with the 'gpt-5.3-codex-spark' model.",
                    )
        })
        .collect::<Vec<_>>();
    assert_eq!(durable_errors.len(), 1);
    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(session_state.active_prompt_for_agent(agent.id()).is_none());
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(run.id())
            .expect("provider run should exist")
            .terminal_diagnostic(),
        Some(
            "Provider prompt dispatch failed: Unsupported parameter: 'reasoning.summary' is not supported with the 'gpt-5.3-codex-spark' model."
        )
    );
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(run.id())
            .expect("provider run should exist")
            .state(),
        crate::provider::ProviderRunState::Ended
    );
    let activity = runtime.agent_activity_for_session(&session_state);
    let agent_activity = activity
        .get(agent.id())
        .expect("agent activity should be projected");
    assert_eq!(
        agent_activity.status,
        crate::runtime::projection::AgentRuntimeStatus::Error
    );
    assert_eq!(
        agent_activity
            .last_completed_turn
            .as_ref()
            .expect("failed turn should remain visible")
            .settlement_status,
        crate::git_observer::CompletedTurnSettlementStatus::Failed
    );
}

#[tokio::test]
async fn opencode_network_terminal_failure_retires_the_failed_resume_session() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-opencode-network-error",
            "worktree-opencode-network-error",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-opencode-network-error",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let resume_state =
        crate::provider::ProviderResumeState::from_opencode_session_id("failed-opencode-session");
    app.agents
        .set_agent_runtime_profile(
            agent.id(),
            "opencode",
            Some("opencode/x-preview-f-free".to_string()),
            Some("high".to_string()),
            resume_state.clone(),
        )
        .expect("agent should retain the provider session");
    let mut run = crate::provider::RuntimeProviderRun::from_control_capability_inference(
        "provider-run-opencode-network-error",
        session.id().to_string(),
        Some(agent.id().to_string()),
        "opencode".to_string(),
    );
    run.set_resume_state(resume_state);
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());
    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "continue on the configured model\n",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should start");
    crate::transport::flow_control::note_prompt_started(&mut app, run.id());

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            crate::provider::ProviderPromptSignalBatch {
                terminal_failure: Some("Provider finish_reason: network_error".to_string()),
                prompt_completed: true,
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("terminal network failure should settle");

    assert_eq!(
        runtime
            .owned
            .agent_store
            .get_agent(agent.id())
            .expect("agent should exist")
            .provider_resume_state()
            .opencode_session_id(),
        None,
        "the next user prompt must start a fresh OpenCode provider session",
    );
    assert!(runtime
        .owned
        .durable_state_store
        .load_events_by_kind("agent.runtime_profile_updated")
        .expect("durable agent events should load")
        .into_iter()
        .any(|event| event.subject_id.as_deref() == Some(agent.id())));
    assert!(runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist")
        .active_prompt_for_agent(agent.id())
        .is_none());
}

#[tokio::test]
async fn structured_submit_resume_failure_clears_agent_and_session_state() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-submit-resume-failure",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt-5.5",
    )
    .with_agent_id(agent.id());
    app.agents
        .set_agent_runtime_profile(
            agent.id(),
            "codex",
            Some("gpt-5.5".to_string()),
            Some("default".to_string()),
            crate::provider::ProviderResumeState::from_codex_thread_id("stale-thread"),
        )
        .expect("agent should start with the stale provider session");
    assert_eq!(
        app.agents
            .get_agent(agent.id())
            .expect("agent should exist")
            .provider_resume_state()
            .codex_thread_id(),
        Some("stale-thread")
    );
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-codex-submit-resume-failure",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-codex".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-codex-runtime".to_string()),
        },
    );
    run.set_resume_state(crate::provider::ProviderResumeState::from_codex_thread_id(
        "stale-thread",
    ));
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());
    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "submit after stale resume\n",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should start");
    let prompt_id = app
        .prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
        .expect("prompt state should load")
        .expect("prompt should be active")
        .id()
        .to_string();
    app.mark_active_prompt_delivery(
        session.id(),
        agent.id(),
        &prompt_id,
        crate::session::DurablePromptDeliveryPhase::Dispatching,
        Some(run.id().to_string()),
        Some("stale-thread".to_string()),
    )
    .expect("structured prompt dispatch should be durable");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .owned
        .provider_store
        .push_finished_structured_prompt_submit_for_test(
            session.id().to_string(),
            run.id().to_string(),
            agent.id().to_string(),
            prompt_id,
            Err(crate::error::DaemonError::ProviderProtocol {
                provider_run_id: run.id().to_string(),
                operation: "thread/resume",
                message: "no rollout found for thread".to_string(),
            }),
        );
    let durable_path = runtime.owned.durable_state_store.path().to_path_buf();
    let connection = rusqlite::Connection::open(&durable_path)
        .expect("durable database should open for failure injection");
    connection
        .execute_batch(
            "CREATE TRIGGER fail_resume_clear_append
             BEFORE INSERT ON durable_state_events
             WHEN NEW.kind = 'agent.runtime_profile_updated'
             BEGIN
               SELECT RAISE(FAIL, 'injected provider resume clear persistence failure');
             END;",
        )
        .expect("resume clear failure trigger should install");

    let completion_sequence = runtime
        .owned
        .provider_store
        .run_actor_completion_signal()
        .sequence();
    runtime.owned.reap_structured_prompt_jobs();
    assert_eq!(
        runtime
            .owned
            .provider_store
            .run_actor_completion_signal()
            .sequence(),
        completion_sequence,
        "a failed durable settlement must not immediately wake itself for another attempt",
    );

    let failed_clear_session = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist after failed clear");
    assert_eq!(
        failed_clear_session.active_provider_run_id(),
        Some(run.id()),
        "the failed provider must remain active until resume invalidation is durable",
    );
    let failed_clear_prompt = failed_clear_session
        .active_prompt_for_agent(agent.id())
        .expect("failed delivery intent should remain active");
    assert_eq!(
        failed_clear_prompt.status(),
        crate::session::PromptStatus::Cancelling,
    );
    assert!(failed_clear_prompt.durable_delivery_failure_pending());
    assert_eq!(
        runtime
            .owned
            .agent_store
            .get_agent(agent.id())
            .expect("agent should exist")
            .provider_resume_state()
            .codex_thread_id(),
        Some("stale-thread"),
        "the failed durable write must restore the stale resume in memory",
    );
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(run.id())
            .expect("provider run should exist")
            .state(),
        crate::provider::ProviderRunState::Running,
    );

    connection
        .execute_batch("DROP TRIGGER fail_resume_clear_append;")
        .expect("resume clear failure trigger should be removed");
    tokio::time::sleep(std::time::Duration::from_millis(125)).await;
    runtime.owned.reap_structured_prompt_jobs();

    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert_eq!(session_state.active_provider_run_id(), None);
    assert!(session_state.active_prompt_for_agent(agent.id()).is_none());
    assert_eq!(
        runtime
            .owned
            .agent_store
            .get_agent(agent.id())
            .expect("agent should exist")
            .provider_resume_state()
            .codex_thread_id(),
        None
    );
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(run.id())
            .expect("provider run should exist")
            .state(),
        crate::provider::ProviderRunState::Ended
    );
}

#[tokio::test]
async fn structured_prompt_acknowledgement_retries_until_profile_and_delivery_are_durable() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-ack-retry",
            "worktree-ack-retry",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-ack-retry",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt-5.5",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-structured-ack-retry",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-codex".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-codex-runtime".to_string()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());
    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "persist this acknowledgement\n",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should start");
    let prompt_id = app
        .prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
        .expect("prompt state should load")
        .expect("prompt should be active")
        .id()
        .to_string();
    app.mark_active_prompt_delivery(
        session.id(),
        agent.id(),
        &prompt_id,
        crate::session::DurablePromptDeliveryPhase::Dispatching,
        Some(run.id().to_string()),
        None,
    )
    .expect("structured prompt dispatch should be durable");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .owned
        .provider_store
        .push_finished_structured_prompt_submit_for_test(
            session.id().to_string(),
            run.id().to_string(),
            agent.id().to_string(),
            prompt_id.clone(),
            Ok(crate::provider::ProviderPromptSubmitAcknowledgement {
                resume_state: crate::provider::ProviderResumeState::from_codex_thread_id(
                    "codex-thread-acknowledged",
                ),
            }),
        );
    let durable_path = runtime.owned.durable_state_store.path().to_path_buf();
    let connection = rusqlite::Connection::open(&durable_path)
        .expect("durable database should open for failure injection");
    connection
        .execute_batch(
            "CREATE TRIGGER fail_ack_profile_append
             BEFORE INSERT ON durable_state_events
             WHEN NEW.kind = 'agent.runtime_profile_updated'
             BEGIN
               SELECT RAISE(FAIL, 'injected acknowledgement profile persistence failure');
             END;",
        )
        .expect("profile failure trigger should install");

    runtime.owned.reap_structured_prompt_jobs();

    connection
        .execute_batch("DROP TRIGGER fail_ack_profile_append;")
        .expect("profile failure trigger should be removed");
    assert_eq!(
        runtime
            .owned
            .agent_store
            .get_agent(agent.id())
            .expect("agent should exist")
            .provider_resume_state()
            .codex_thread_id(),
        None,
    );
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(run.id())
            .expect("provider run should exist")
            .resume_state()
            .codex_thread_id(),
        None,
        "the in-memory run acknowledgement must wait for durable profile persistence",
    );
    assert_eq!(
        runtime
            .owned
            .session_snapshot(session.id())
            .expect("session should remain available")
            .active_prompt_for_agent(agent.id())
            .expect("prompt should remain active")
            .durable_delivery_phase(),
        Some(crate::session::DurablePromptDeliveryPhase::Dispatching),
    );

    connection
        .execute_batch(
            "CREATE TRIGGER fail_ack_delivery_append
             BEFORE INSERT ON durable_state_events
             WHEN NEW.kind = 'session.prompt_state.updated'
             BEGIN
               SELECT RAISE(FAIL, 'injected acknowledgement delivery persistence failure');
             END;",
        )
        .expect("delivery failure trigger should install");
    tokio::time::sleep(std::time::Duration::from_millis(125)).await;
    runtime.owned.reap_structured_prompt_jobs();
    connection
        .execute_batch("DROP TRIGGER fail_ack_delivery_append;")
        .expect("delivery failure trigger should be removed");
    assert_eq!(
        runtime
            .owned
            .session_snapshot(session.id())
            .expect("session should remain available")
            .active_prompt_for_agent(agent.id())
            .expect("prompt should remain active")
            .durable_delivery_phase(),
        Some(crate::session::DurablePromptDeliveryPhase::Dispatching),
        "a failed delivery append must roll the in-memory and session mirrors back",
    );
    assert_eq!(
        runtime
            .owned
            .agent_store
            .get_agent(agent.id())
            .expect("agent should exist")
            .provider_resume_state()
            .codex_thread_id(),
        Some("codex-thread-acknowledged"),
    );

    tokio::time::sleep(std::time::Duration::from_millis(225)).await;
    runtime.owned.reap_structured_prompt_jobs();

    let active_prompt = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session should remain available")
        .active_prompt_for_agent(agent.id())
        .expect("acknowledged prompt should remain active")
        .clone();
    assert_eq!(
        active_prompt.durable_delivery_phase(),
        Some(crate::session::DurablePromptDeliveryPhase::Delivered),
    );
    assert_eq!(
        active_prompt.status(),
        crate::session::PromptStatus::Running
    );
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(run.id())
            .expect("provider run should exist")
            .resume_state()
            .codex_thread_id(),
        Some("codex-thread-acknowledged"),
    );
}

#[tokio::test]
async fn structured_output_resume_retries_without_losing_the_finished_batch() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-output-resume-retry",
            "worktree-output-resume-retry",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-output-resume-retry",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let initial_resume =
        crate::provider::ProviderResumeState::from_codex_thread_id("codex-thread-output-s1");
    let updated_agent = app
        .agents_mut()
        .set_agent_runtime_profile_with_account_profile(
            agent.id(),
            "codex",
            Some("gpt-5.5".to_string()),
            None,
            Some("default".to_string()),
            initial_resume.clone(),
        )
        .expect("initial agent resume should seed");
    app.durable_state_store()
        .append_event(
            "agent.runtime_profile_updated",
            Some(updated_agent.id().to_string()),
            serde_json::json!({
                "agent": &updated_agent,
                "reason": "test_structured_output_resume_seeded",
            }),
        )
        .expect("initial agent resume should persist");
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt-5.5",
    )
    .with_agent_id(agent.id())
    .with_resume_state(initial_resume);
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-output-resume-retry",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-codex".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: Default::default(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-codex-runtime".to_string()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());
    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "persist the output resume\n",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should start");
    let prompt_id = app
        .prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
        .expect("prompt state should load")
        .expect("prompt should be active")
        .id()
        .to_string();
    app.mark_active_prompt_delivery(
        session.id(),
        agent.id(),
        &prompt_id,
        crate::session::DurablePromptDeliveryPhase::Delivered,
        Some(run.id().to_string()),
        Some("codex-thread-output-s1".to_string()),
    )
    .expect("prompt delivery should persist");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let current_session = runtime
        .owned
        .session_store
        .get_session(session.id())
        .expect("session should remain available");
    runtime
        .owned
        .prompt_state_owner
        .mark_active_prompt_running(&current_session, agent.id())
        .expect("acknowledged prompt should be running");
    let (active_prompt, queued_prompts) = runtime
        .owned
        .prompt_state_owner
        .state_parts(&current_session, agent.id());
    runtime
        .owned
        .mirror_prompt_owner_agent_state(session.id(), agent.id(), active_prompt, queued_prompts)
        .expect("running prompt state should persist");
    runtime
        .owned
        .structured_output_records
        .mark_poll_enqueued(run.id(), Some(prompt_id));
    runtime
        .owned
        .provider_store
        .write()
        .push_finished_structured_output_poll_for_test(
            run.id().to_string(),
            Ok(Some(crate::provider::ProviderPromptSignalBatch {
                resolved_resume_state: Some(
                    crate::provider::ProviderResumeState::from_codex_thread_id(
                        "codex-thread-output-s2",
                    ),
                ),
                ..crate::provider::ProviderPromptSignalBatch::default()
            })),
        );
    let durable_path = runtime.owned.durable_state_store.path().to_path_buf();
    let connection = rusqlite::Connection::open(&durable_path)
        .expect("durable database should open for failure injection");
    connection
        .execute_batch(
            "CREATE TRIGGER fail_output_resume_append
             BEFORE INSERT ON durable_state_events
             WHEN NEW.kind = 'agent.runtime_profile_updated'
             BEGIN
               SELECT RAISE(FAIL, 'injected structured output resume persistence failure');
             END;",
        )
        .expect("output resume failure trigger should install");

    runtime
        .pump_owned_structured_provider_output(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
        )
        .await
        .expect("retryable durable failure should defer the batch");

    assert_eq!(
        runtime
            .owned
            .agent_store
            .get_agent(agent.id())
            .expect("agent should remain available")
            .provider_resume_state()
            .codex_thread_id(),
        Some("codex-thread-output-s1"),
    );
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(run.id())
            .expect("run should remain available")
            .resume_state()
            .codex_thread_id(),
        Some("codex-thread-output-s1"),
        "the process-local run must not advance beyond the durable agent profile",
    );
    let retry_sequence = runtime
        .owned
        .provider_store
        .run_actor_completion_signal()
        .sequence();

    connection
        .execute_batch("DROP TRIGGER fail_output_resume_append;")
        .expect("output resume failure trigger should be removed");
    tokio::time::sleep(std::time::Duration::from_millis(225)).await;
    assert!(
        runtime
            .owned
            .provider_store
            .run_actor_completion_signal()
            .sequence()
            > retry_sequence,
        "the deferred batch should become ready after its retry delay",
    );
    let current_session = runtime
        .owned
        .session_store
        .get_session(session.id())
        .expect("session should remain available");
    let active_prompt = current_session
        .active_prompt_for_agent(agent.id())
        .expect("prompt should remain active");
    assert_eq!(
        active_prompt.status(),
        crate::session::PromptStatus::Running
    );
    assert_eq!(
        active_prompt.durable_delivery_phase(),
        Some(crate::session::DurablePromptDeliveryPhase::Delivered),
    );
    runtime
        .pump_owned_structured_provider_output(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
        )
        .await
        .expect("deferred output batch should settle");

    assert_eq!(
        runtime
            .owned
            .agent_store
            .get_agent(agent.id())
            .expect("agent should remain available")
            .provider_resume_state()
            .codex_thread_id(),
        Some("codex-thread-output-s2"),
    );
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(run.id())
            .expect("run should remain available")
            .resume_state()
            .codex_thread_id(),
        Some("codex-thread-output-s2"),
        "the exact drained batch must be retried after storage recovers",
    );
}

#[tokio::test]
async fn first_output_timeout_projects_error_and_closes_prompt() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-first-output-timeout",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt-5.5",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-first-output-timeout",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-codex-timeout".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-codex-timeout-runtime".to_string()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());

    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "start but never answer\n",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should start");
    crate::transport::flow_control::note_prompt_started(&mut app, run.id());
    let prompt_id = app
        .sessions()
        .get_session(session.id())
        .expect("session should exist")
        .active_prompt_for_agent(agent.id())
        .expect("active prompt should exist")
        .id()
        .to_string();
    let mut timed_out_turn = crate::app::ActiveTurnState::new(
        session.id().to_string(),
        agent.id().to_string(),
        prompt_id,
        run.id().to_string(),
    )
    .with_phase(crate::app::ActiveTurnPhase::AwaitingFirstOutput);
    timed_out_turn.started_at_ms = crate::session::unix_epoch_ms().saturating_sub(11 * 60 * 1000);
    app.active_turn_store().start(timed_out_turn);

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .pump_owned_provider_output(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            true,
        )
        .await
        .expect("provider output pump should reap silent timeout");

    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(
        session_state.active_prompt_for_agent(agent.id()).is_none(),
        "silent provider timeout must close the active prompt"
    );
    let run = runtime
        .owned
        .provider_store
        .get_run(run.id())
        .expect("provider run should still exist");
    assert!(run
        .terminal_diagnostic()
        .expect("timeout diagnostic should be recorded")
        .contains("Provider prompt produced no output"));
    let provider_errors = runtime
        .owned
        .terminal_stream
        .drain_output_records(session.id(), attachment.id())
        .into_iter()
        .filter(|record| record.kind == crate::terminal::TerminalOutputKind::ProviderError)
        .collect::<Vec<_>>();
    assert_eq!(
        provider_errors.len(),
        1,
        "an unprojected terminal failure should surface exactly one provider error"
    );
    assert!(
        String::from_utf8_lossy(&provider_errors[0].bytes)
            .contains("Provider prompt produced no output"),
        "the visible provider error should preserve the terminal diagnostic"
    );
    let durable_errors = runtime
        .owned
        .operational_history_store
        .load_session_events(session.id(), Some(agent.id()))
        .expect("canonical operational history should load")
        .into_iter()
        .filter(|event| event.kind == crate::history::HistoryEventKind::ProviderError)
        .collect::<Vec<_>>();
    assert_eq!(
        durable_errors.len(),
        1,
        "the provider error should be durable across reload"
    );
    let notices = runtime
        .owned
        .terminal_stream
        .drain_notice_records(session.id(), attachment.id());
    assert!(
        notices.iter().any(|record| record
            .message
            .contains("Provider prompt produced no output")),
        "timeout diagnostic should be visible to attached clients"
    );
}

#[tokio::test]
async fn provider_inactivity_timeout_records_diagnostic_and_closes_prompt() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-inactivity-timeout",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "opencode",
        "opencode",
        "default",
        "zen",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-inactivity-timeout",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-opencode-timeout".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-opencode-runtime".to_string()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());

    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "start, emit a tool, then stall\n",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should start");
    crate::transport::flow_control::note_prompt_started(&mut app, run.id());
    crate::transport::flow_control::note_prompt_response_content(&mut app, run.id());
    app.active_turns.mark_streaming(run.id());
    if let Some(state) = app.prompt_activity.write().get_mut(run.id()) {
        state.last_output_at =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(11 * 60));
        state.saw_response_content = true;
    } else {
        panic!("prompt activity should exist for the active run");
    }

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .pump_owned_provider_output(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            true,
        )
        .await
        .expect("provider output pump should reap inactive provider turn");

    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(
        session_state.active_prompt_for_agent(agent.id()).is_none(),
        "inactive provider timeout must close the active prompt"
    );
    let run = runtime
        .owned
        .provider_store
        .get_run(run.id())
        .expect("provider run should still exist");
    assert!(run
        .terminal_diagnostic()
        .expect("timeout diagnostic should be recorded")
        .contains("Provider prompt produced no output"));
    let notices = runtime
        .owned
        .terminal_stream
        .drain_notice_records(session.id(), attachment.id());
    assert!(
        notices
            .iter()
            .any(|record| record.message.contains("after its last activity")),
        "inactivity timeout diagnostic should be visible to attached clients"
    );
}

#[tokio::test]
async fn provider_inactivity_timeout_waits_for_an_active_structured_tool() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-active-tool",
            "worktree-active-tool",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-active-tool",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "opencode",
        "opencode",
        "default",
        "zen",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-active-tool",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-opencode-active-tool".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-opencode-active-tool-runtime".to_string()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());
    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "run a long compile\n",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should start");
    crate::transport::flow_control::note_prompt_started(&mut app, run.id());

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            crate::provider::ProviderPromptSignalBatch {
                chunks: vec![crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderTool,
                    merge_key: Some("compile-1".to_string()),
                    bytes: br#"{"id":"compile-1","tool":"bash","status":"running"}"#.to_vec(),
                }],
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("running tool output should be accepted");
    runtime
        .owned
        .prompt_activity
        .write()
        .get_mut(run.id())
        .expect("prompt activity should exist")
        .last_output_at = Some(std::time::Instant::now() - std::time::Duration::from_secs(11 * 60));

    runtime
        .reap_provider_inactivity_timeouts(session.id())
        .await
        .expect("active tool should suppress the inactivity timeout");
    assert!(runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist")
        .active_prompt_for_agent(agent.id())
        .is_some());

    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            crate::provider::ProviderPromptSignalBatch {
                chunks: vec![crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderTool,
                    merge_key: Some("compile-1".to_string()),
                    bytes: br#"{"id":"compile-1","tool":"bash","status":"completed"}"#.to_vec(),
                }],
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("completed tool output should be accepted");
    runtime
        .owned
        .prompt_activity
        .write()
        .get_mut(run.id())
        .expect("prompt activity should still exist")
        .last_output_at = Some(std::time::Instant::now() - std::time::Duration::from_secs(11 * 60));

    runtime
        .reap_provider_inactivity_timeouts(session.id())
        .await
        .expect("completed tool should restore the inactivity timeout");
    assert!(runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist")
        .active_prompt_for_agent(agent.id())
        .is_none());
}

#[tokio::test]
async fn provider_inactivity_timeout_retires_managed_process() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-managed-timeout",
            "worktree-managed-timeout",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-managed-timeout",
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
        .expect("managed provider should launch");
    let pid = app
        .pty()
        .process_id(run.id())
        .expect("managed provider process should resolve")
        .expect("managed provider should have a pid");
    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "start, emit output, then stall\n",
        crate::session::PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Started { prompt } = app
        .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should start")
    else {
        panic!("prompt should start immediately");
    };
    app.mark_active_prompt_delivery(
        session.id(),
        agent.id(),
        prompt.id(),
        crate::session::DurablePromptDeliveryPhase::Delivered,
        Some(run.id().to_string()),
        None,
    )
    .expect("prompt should bind to the managed provider");
    crate::transport::flow_control::note_prompt_started(&mut app, run.id());
    crate::transport::flow_control::note_prompt_response_content(&mut app, run.id());
    app.active_turns.mark_streaming(run.id());
    app.prompt_activity
        .write()
        .get_mut(run.id())
        .expect("prompt activity should exist")
        .last_output_at = Some(std::time::Instant::now() - std::time::Duration::from_secs(11 * 60));

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .pump_owned_provider_output(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            true,
        )
        .await
        .expect("provider output pump should reap inactive provider turn");

    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(run.id())
            .expect("provider run should remain addressable")
            .state(),
        crate::provider::ProviderRunState::Ended,
    );
    assert!(
        !crate::runtime::process_health::process_running(pid),
        "terminal timeout must stop the managed provider child"
    );
    let tracking = runtime.owned.provider_process_tracking.snapshot();
    assert!(tracking.run_processes.is_empty());
    assert!(tracking.processes.is_empty());
}

#[tokio::test]
async fn meta_mode_activation_registers_pending_provider_reload_when_agent_busy() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-meta-reload",
            "worktree-meta-reload",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-meta-reload",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "model",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider run should launch");
    app.update_provider_run_projection(run.clone());
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(agent.id()),
        "busy before meta mode\n",
        Vec::new(),
    )
    .expect("prompt should start");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .activate_meta_mode_for_prompt(session.id(), agent.id(), "delegate the queued task")
        .await
        .expect("meta mode should activate");

    assert!(
        runtime
            .owned
            .pending_provider_reloads
            .write()
            .contains_key(agent.id()),
        "busy meta mode activation must register a pending provider reload"
    );
    let agent = runtime
        .owned
        .agent_store
        .get_agent(agent.id())
        .expect("agent should still exist");
    assert!(agent.is_metaagent());
}

#[tokio::test]
async fn metaagent_receives_required_failed_turn_event_on_provider_timeout() {
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-meta-failure",
            "worktree-meta-failure",
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "codex").with_alias("meta"),
        )
        .expect("metaagent should spawn");
    let metaagent = app
        .agents_mut()
        .activate_agent_meta_mode(metaagent.id(), None)
        .expect("agent should enter meta mode");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-meta-failure",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let worker_request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt-5.5",
    )
    .with_agent_id(agent.id());
    let mut worker_run = crate::provider::RuntimeProviderRun::new(
        "provider-run-meta-failure-worker",
        &worker_request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-meta-failure-worker".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-meta-failure-worker-runtime".to_string()),
        },
    );
    worker_run.mark_running();
    app.providers_mut().insert_run_for_test(worker_run.clone());
    app.update_provider_run_projection(worker_run.clone());

    let meta_request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt-5.5",
    )
    .with_agent_id(metaagent.id());
    let mut meta_run = crate::provider::RuntimeProviderRun::new(
        "provider-run-meta-failure-meta",
        &meta_request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-meta-failure-meta".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-meta-failure-meta-runtime".to_string()),
        },
    );
    meta_run.mark_running();
    app.providers_mut().insert_run_for_test(meta_run.clone());
    app.update_provider_run_projection(meta_run.clone());

    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "start but never answer\n",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should start");
    crate::transport::flow_control::note_prompt_started(&mut app, worker_run.id());
    let prompt_id = app
        .sessions()
        .get_session(session.id())
        .expect("session should exist")
        .active_prompt_for_agent(agent.id())
        .expect("active prompt should exist")
        .id()
        .to_string();
    let mut timed_out_turn = crate::app::ActiveTurnState::new(
        session.id().to_string(),
        agent.id().to_string(),
        prompt_id,
        worker_run.id().to_string(),
    )
    .with_phase(crate::app::ActiveTurnPhase::AwaitingFirstOutput);
    timed_out_turn.started_at_ms = crate::session::unix_epoch_ms().saturating_sub(11 * 60 * 1000);
    app.active_turn_store().start(timed_out_turn);

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .pump_owned_provider_output(
            session.id(),
            worker_run.id(),
            vec![attachment.id().to_string()],
            true,
        )
        .await
        .expect("provider output pump should reap silent timeout");

    let events =
        runtime
            .owned
            .metaagent_events
            .list(metaagent.id(), Some("agent.turn.failed"), None, 10);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].source_agent_id.as_deref(), Some(agent.id()));
    assert!(events[0]
        .summary
        .contains("Provider prompt produced no output"));
    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(
        session_state
            .active_prompt_for_agent(metaagent.id())
            .is_some(),
        "failed-turn event should start an inline metaagent prompt when the metaagent is idle"
    );
}
