use super::*;

#[test]
fn stale_worker_snapshot_cannot_restore_profile_after_remote_substitution() {
    assert_stale_worker_snapshot_preserves_selected_profile(Some("current-worker-run"));
}

#[test]
fn stale_worker_snapshot_cannot_restore_profile_before_successor_dispatch() {
    assert_stale_worker_snapshot_preserves_selected_profile(None);
}

fn assert_stale_worker_snapshot_preserves_selected_profile(active_worker_run: Option<&str>) {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "home-client",
            ClientCapabilityLevel::InteractiveStructured,
        ))
        .unwrap();
    app.agents()
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker-kernel".into(),
                worker_machine_id: "worker-machine".into(),
                execution_lease_id: "lease-1".into(),
                leased_agent_id: "leased-agent-1".into(),
                active_worker_provider_run_id: active_worker_run.map(str::to_string),
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .unwrap();
    let expected = app
        .agents()
        .set_agent_runtime_profile_with_account_profile(
            agent.id(),
            "codex",
            Some("gpt-5.6-sol".into()),
            Some("high".into()),
            Some("selected-codex-account".into()),
            ProviderResumeState::from_codex_thread_id("current-codex-thread"),
        )
        .unwrap();
    let next_prompt = crate::session::PromptQueueItem::new(
        "next-home-prompt",
        attachment.id(),
        agent.id(),
        "next review",
        PromptStatus::Queued,
    );
    let PromptSubmissionOutcome::Started {
        prompt: next_prompt,
    } = app
        .prompt_owner_submit_prepared_prompt(session.id(), next_prompt, false)
        .unwrap()
    else {
        panic!("successor prompt should be active");
    };
    let request = LaunchProviderRequest::new(
        "worker-session",
        "claude-headless",
        "claude-headless",
        "old-claude-account",
        "claude-opus-4-8",
    )
    .with_variant(Some("medium".into()));
    let stale_run = RuntimeProviderRun::new(
        "old-worker-run",
        &request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "old-worker-claude".into(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    let outcome = RemoteLeaseRuntime::new(&mut app)
        .project_remote_runtime_projection(
            session.id(),
            agent.id(),
            stale_run.id(),
            Some(stale_run.clone()),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![RelayProjectedCompletion {
                message_id: "old-completion".into(),
                completed_at_ms: crate::session::unix_epoch_ms(),
                home_prompt_id: Some("previous-home-prompt".into()),
            }],
        )
        .unwrap();
    assert!(!outcome.accepted);
    let current = app.agents().get_agent(agent.id()).unwrap();
    assert_eq!(current.provider(), expected.provider());
    assert_eq!(current.account_profile(), expected.account_profile());
    assert_eq!(current.model(), expected.model());
    assert_eq!(current.effort(), expected.effort());
    assert_eq!(
        current.provider_resume_state(),
        expected.provider_resume_state()
    );
    assert_eq!(
        current
            .remote_execution()
            .unwrap()
            .active_worker_provider_run_id,
        active_worker_run.map(str::to_string)
    );
    assert_eq!(
        app.prompt_owner_active_prompt_for_agent(session.id(), agent.id())
            .unwrap()
            .unwrap()
            .id(),
        next_prompt.id()
    );
}

#[test]
fn native_worker_snapshot_can_establish_run_binding_without_home_dispatch() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .unwrap();
    app.agents()
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker-kernel".into(),
                worker_machine_id: "worker-machine".into(),
                execution_lease_id: "lease-native".into(),
                leased_agent_id: "leased-native".into(),
                active_worker_provider_run_id: None,
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .unwrap();
    let request =
        LaunchProviderRequest::new("worker-room", "codex", "codex", "default", "gpt-5.6-sol")
            .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let run = RuntimeProviderRun::new(
        "native-worker-run",
        &request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "native-worker".into(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    let outcome = RemoteLeaseRuntime::new(&mut app)
        .project_remote_runtime_projection(
            session.id(),
            agent.id(),
            run.id(),
            Some(run.clone()),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
    assert!(outcome.accepted);
    let updated = app.agents().get_agent(agent.id()).unwrap();
    assert_eq!(
        updated
            .remote_execution()
            .unwrap()
            .active_worker_provider_run_id
            .as_deref(),
        Some("native-worker-run")
    );
    assert_eq!(updated.model(), Some("gpt-5.6-sol"));
}

#[test]
fn remote_workflow_completion_preserves_worker_provider_failure_diagnostic() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
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
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("review".into()),
        )
        .unwrap();
    let node_run_id = run.node_runs()[0].id().to_string();
    app.sessions_mut()
        .prepare_workflow_turn(
            session.id(),
            run.id(),
            &node_run_id,
            "turn-token".into(),
            "review".into(),
            None,
            None,
        )
        .unwrap();
    app.sessions_mut()
        .start_workflow_node_run(session.id(), run.id(), &node_run_id)
        .unwrap();
    let prompt = crate::session::PromptQueueItem::new(
        "home-prompt",
        crate::scheduler::runtime::workflow_prompt_source_attachment_id(run.id()),
        agent.id(),
        "review",
        PromptStatus::Queued,
    )
    .with_workflow_context(run.id(), &node_run_id);
    let PromptSubmissionOutcome::Started { prompt } = app
        .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .unwrap()
    else {
        panic!("workflow turn should start");
    };
    let request =
        LaunchProviderRequest::new("worker-session", "codex", "codex", "default", "gpt-5.6-sol");
    let mut worker_run = RuntimeProviderRun::new(
        "worker-run-1",
        &request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "worker-codex".into(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    let diagnostic = "You've hit your session limit";
    worker_run.set_terminal_diagnostic(diagnostic);
    let outcome = RemoteLeaseRuntime::new(&mut app)
        .project_remote_runtime_projection(
            session.id(),
            agent.id(),
            "worker-run-1",
            Some(worker_run),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![RelayProjectedCompletion {
                message_id: "failed-turn-1".into(),
                completed_at_ms: crate::session::unix_epoch_ms(),
                home_prompt_id: Some(prompt.id().into()),
            }],
        )
        .unwrap();
    let current = app.sessions().get_session(session.id()).unwrap();
    let failed = current.workflow_run(run.id()).unwrap();
    assert_eq!(failed.failure_events().len(), 1);
    assert_eq!(
        failed.failure_events()[0].kind(),
        crate::session::WorkflowFailureKind::ProviderFailure
    );
    assert_eq!(failed.failure_events()[0].message(), diagnostic);
    assert!(app
        .prompt_owner_active_prompt_for_agent(session.id(), agent.id())
        .unwrap()
        .is_none());
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "concurrent-client",
            ClientCapabilityLevel::InteractiveStructured,
        ))
        .unwrap();
    let followup = crate::session::PromptQueueItem::new(
        "concurrent-followup",
        attachment.id(),
        agent.id(),
        "next review",
        PromptStatus::Queued,
    );
    assert!(
        matches!(
            app.prompt_owner_submit_prepared_prompt(session.id(), followup, false)
                .unwrap(),
            PromptSubmissionOutcome::Queued { .. }
        ),
        "failure settlement must reserve admission until substitute reconciliation finishes"
    );
    drop(outcome);
}

#[test]
fn remote_runtime_projection_records_output_and_completion_on_home_session() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(
            CreateSessionRequest::new("workspace-1", "worktree-1")
                .with_agent_defaults(crate::session::SessionAgentDefaults::new("dev-stub")),
        )
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::InteractiveStructured,
        ))
        .expect("attachment should attach");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = app
        .agents_mut()
        .activate_agent_meta_mode(metaagent.id(), None)
        .expect("agent should enter meta mode");
    let trace_subscription = app.metaagent_trace_subscription_store().subscribe(
        session.id(),
        metaagent.id(),
        agent.id(),
        crate::runtime::metaagent_trace::MetaagentTraceMode::Compact,
    );
    let prompt = app
        .submit_prompt(
            session.id(),
            attachment.id(),
            Some(agent.id()),
            "remote prompt",
            Vec::new(),
        )
        .expect("prompt should start");
    let PromptSubmissionOutcome::Started { prompt } = prompt else {
        panic!("prompt should be active");
    };
    let started_at_ms = app
        .operational_history_store()
        .load_session_events(session.id(), Some(agent.id()))
        .expect("prompt history should load")
        .into_iter()
        .find(|event| event.prompt_id.as_deref() == Some(prompt.id()))
        .expect("prompt history event should exist")
        .timestamp_ms;
    let completed_at_ms = started_at_ms.saturating_add(9_000);
    let projected_completion = RelayProjectedCompletion {
        message_id: "assistant-msg-1".to_string(),
        completed_at_ms,
        home_prompt_id: Some(prompt.id().to_string()),
    };

    RemoteLeaseRuntime::new(&mut app)
        .project_remote_runtime_projection(
            session.id(),
            agent.id(),
            "remote:worker:provider-run-1",
            None,
            Vec::new(),
            vec![RelayProjectedOutputChunk {
                kind: TerminalOutputKind::ProviderOutput,
                merge_key: Some("assistant-1".to_string()),
                bytes: b"remote output".to_vec(),
            }],
            vec!["remote notice".to_string()],
            vec![projected_completion.clone(), projected_completion],
        )
        .expect("projection should succeed");

    let outputs = app
        .terminal_mut()
        .drain_output_records(session.id(), attachment.id());
    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0].agent_id.as_deref(), Some(agent.id()));
    assert_eq!(outputs[0].bytes, b"remote output".to_vec());

    let notices = app
        .terminal_mut()
        .drain_notice_records(session.id(), attachment.id());
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].agent_id.as_deref(), Some(agent.id()));
    assert_eq!(notices[0].message, "remote notice");

    let completions = app
        .terminal_mut()
        .drain_completion_records(session.id(), attachment.id());
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].agent_id.as_deref(), Some(agent.id()));
    assert_eq!(completions[0].message_id, "assistant-msg-1");

    let trace_outputs = app
        .terminal_mut()
        .drain_output_records(session.id(), &trace_subscription.recipient_attachment_id);
    assert_eq!(trace_outputs.len(), 1);
    assert_eq!(trace_outputs[0].agent_id.as_deref(), Some(agent.id()));
    assert_eq!(trace_outputs[0].bytes, b"remote output".to_vec());

    let trace_notices = app
        .terminal_mut()
        .drain_notice_records(session.id(), &trace_subscription.recipient_attachment_id);
    assert_eq!(trace_notices.len(), 1);
    assert_eq!(trace_notices[0].agent_id.as_deref(), Some(agent.id()));
    assert_eq!(trace_notices[0].message, "remote notice");

    let trace_completions = app
        .terminal_mut()
        .drain_completion_records(session.id(), &trace_subscription.recipient_attachment_id);
    assert_eq!(trace_completions.len(), 1);
    assert_eq!(trace_completions[0].agent_id.as_deref(), Some(agent.id()));
    assert_eq!(trace_completions[0].message_id, "assistant-msg-1");

    let projected = app
        .session_state_projection_store()
        .get(session.id())
        .expect("projection should refresh");
    assert!(projected
        .prompt_states()
        .get(agent.id())
        .and_then(|state| state.active_prompt())
        .is_none());

    let operational_history = app.operational_history_store();
    drop(app);
    let response = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("history runtime should build")
        .block_on(
            crate::runtime::history_requests::execute_session_history_outline_request(
                operational_history,
                crate::local::GetSessionHistoryOutlineRequest {
                    session_id: session.id().to_string(),
                    agent_ids: Some(vec![agent.id().to_string()]),
                    latest_prompt_count: Some(4),
                    cursor: None,
                },
            ),
        )
        .expect("history outline should reload");
    let crate::local::LocalDaemonResponse::SessionHistoryOutline { agents } = response else {
        panic!("history outline response should load");
    };
    let turn = agents
        .first()
        .and_then(|agent| agent.turns.first())
        .expect("remote turn should reload");
    assert_eq!(turn.prompt_id.as_deref(), Some(prompt.id()));
    assert_eq!(
        turn.lifecycle,
        crate::local::SessionHistoryOutlineTurnLifecycle::Completed
    );
    assert_eq!(turn.started_at_ms, started_at_ms);
    assert_eq!(turn.completed_at_ms, Some(completed_at_ms));
    assert!(completed_at_ms > turn.started_at_ms);
}

#[test]
fn stale_remote_completion_replay_does_not_complete_the_next_prompt() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(
            CreateSessionRequest::new("workspace-1", "worktree-1")
                .with_agent_defaults(crate::session::SessionAgentDefaults::new("dev-stub")),
        )
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::InteractiveStructured,
        ))
        .expect("attachment should attach");
    let first = app
        .submit_prompt(
            session.id(),
            attachment.id(),
            Some(agent.id()),
            "first remote prompt",
            Vec::new(),
        )
        .expect("first prompt should start");
    let PromptSubmissionOutcome::Started { prompt: first } = first else {
        panic!("first prompt should be active");
    };
    let stale_completion = RelayProjectedCompletion {
        message_id: "assistant-msg-1".to_string(),
        completed_at_ms: 1234,
        home_prompt_id: Some(first.id().to_string()),
    };
    RemoteLeaseRuntime::new(&mut app)
        .project_remote_runtime_projection(
            session.id(),
            agent.id(),
            "remote:worker:provider-run-1",
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![stale_completion.clone()],
        )
        .expect("first completion should project");
    let _ = app
        .terminal_mut()
        .drain_completion_records(session.id(), attachment.id());

    let second = app
        .submit_prompt(
            session.id(),
            attachment.id(),
            Some(agent.id()),
            "second remote prompt",
            Vec::new(),
        )
        .expect("second prompt should start");
    let PromptSubmissionOutcome::Started { prompt: second } = second else {
        panic!("second prompt should be active");
    };

    RemoteLeaseRuntime::new(&mut app)
        .project_remote_runtime_projection(
            session.id(),
            agent.id(),
            "remote:worker:provider-run-1",
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![stale_completion],
        )
        .expect("stale replay should be ignored");

    let active = app
        .prompt_owner_active_prompt_for_agent(session.id(), agent.id())
        .expect("active prompt should load")
        .expect("second prompt must remain active");
    assert_eq!(active.id(), second.id());
    assert!(app
        .terminal_mut()
        .drain_completion_records(session.id(), attachment.id())
        .is_empty());
}

#[test]
fn native_completion_correlation_distinguishes_durable_and_native_prompts() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::InteractiveStructured,
        ))
        .expect("attachment should attach");
    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "durable remote prompt",
        crate::session::PromptStatus::Queued,
    )
    .with_durable_operation("operation-1", "fingerprint-1");
    let outcome = app
        .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should start");
    let PromptSubmissionOutcome::Started { prompt } = outcome else {
        panic!("prompt should be active");
    };
    assert_eq!(prompt.durable_operation_id(), Some("operation-1"));

    RemoteLeaseRuntime::new(&mut app)
        .project_remote_runtime_projection(
            session.id(),
            agent.id(),
            "remote:worker:provider-run-1",
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![RelayProjectedCompletion {
                message_id: "prior-native-completion".to_string(),
                completed_at_ms: 1234,
                home_prompt_id: None,
            }],
        )
        .expect("unscoped native completion should be ignored");

    let active = app
        .prompt_owner_active_prompt_for_agent(session.id(), agent.id())
        .expect("active prompt should load")
        .expect("durable prompt must remain active");
    assert_eq!(active.id(), prompt.id());
    assert!(app
        .terminal_mut()
        .drain_completion_records(session.id(), attachment.id())
        .is_empty());

    RemoteLeaseRuntime::new(&mut app)
        .project_remote_runtime_projection(
            session.id(),
            agent.id(),
            "remote:worker:provider-run-1",
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![RelayProjectedCompletion {
                message_id: "current-home-completion".to_string(),
                completed_at_ms: 5678,
                home_prompt_id: Some(prompt.id().to_string()),
            }],
        )
        .expect("scoped home completion should settle the prompt");

    assert!(app
        .prompt_owner_active_prompt_for_agent(session.id(), agent.id())
        .expect("prompt state should load")
        .is_none());
    let completions = app
        .terminal_mut()
        .drain_completion_records(session.id(), attachment.id());
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].message_id, "current-home-completion");

    RemoteLeaseRuntime::new(&mut app)
        .project_remote_runtime_projection(
            session.id(),
            agent.id(),
            "remote:worker:provider-run-1",
            None,
            vec![crate::transport::relay_peer::RelayProjectedPrompt {
                prompt_id: "native-prompt".to_string(),
                text: "native-origin prompt".to_string(),
            }],
            Vec::new(),
            Vec::new(),
            vec![RelayProjectedCompletion {
                message_id: "native-completion".to_string(),
                completed_at_ms: 6789,
                home_prompt_id: None,
            }],
        )
        .expect("unscoped completion should settle a native-origin prompt");

    assert!(app
        .prompt_owner_active_prompt_for_agent(session.id(), agent.id())
        .expect("native prompt state should load")
        .is_none());
    let completions = app
        .terminal_mut()
        .drain_completion_records(session.id(), attachment.id());
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].message_id, "native-completion");
}
