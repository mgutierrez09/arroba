use super::*;

#[test]
fn publication_output_rejects_opencode_model_substitution_before_run_mutation() {
    let request = crate::provider::LaunchProviderRequest::new(
        "session-publication-model-lock",
        "opencode",
        "opencode",
        "default",
        "opencode/gpt-5.2",
    );
    let run = crate::provider::RuntimeProviderRun::new(
        "provider-run-publication-model-lock",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-opencode-publication-model-lock".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-opencode-runtime".to_string()),
        },
    );
    let mut batch = crate::provider::ProviderPromptSignalBatch {
        resolved_model: Some("opencode/big-pickle".to_string()),
        resolved_model_source: Some("message.updated"),
        ..crate::provider::ProviderPromptSignalBatch::default()
    };

    let failure = super::super::structured_provider_output_runtime::reject_workflow_publication_opencode_model_substitution(
        true,
        &run,
        &mut batch,
    )
    .expect("publication model drift should be rejected");

    assert!(failure.contains("substitution is disabled"));
    assert_eq!(run.model(), "opencode/gpt-5.2");
    assert_eq!(batch.resolved_model, None);
    assert_eq!(batch.resolved_model_source, None);
    assert_eq!(batch.terminal_failure.as_deref(), Some(failure.as_str()));
    assert!(batch.prompt_completed);
}

#[test]
fn interactive_output_keeps_opencode_selection_sync_behavior() {
    let request = crate::provider::LaunchProviderRequest::new(
        "session-interactive-model-sync",
        "opencode",
        "opencode",
        "default",
        "opencode/gpt-5.2",
    );
    let run = crate::provider::RuntimeProviderRun::new(
        "provider-run-interactive-model-sync",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-opencode-interactive-model-sync".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-opencode-runtime".to_string()),
        },
    );
    let mut batch = crate::provider::ProviderPromptSignalBatch {
        resolved_model: Some("opencode/big-pickle".to_string()),
        resolved_model_source: Some("message.updated"),
        ..crate::provider::ProviderPromptSignalBatch::default()
    };

    assert_eq!(
        super::super::structured_provider_output_runtime::reject_workflow_publication_opencode_model_substitution(
            false,
            &run,
            &mut batch,
        ),
        None,
    );
    assert_eq!(batch.resolved_model.as_deref(), Some("opencode/big-pickle"));
    assert_eq!(batch.terminal_failure, None);
}

async fn assert_owned_output_pump_drains_pending_record_after_run_state_change(
    state: crate::provider::ProviderRunState,
) {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-pending-structured-output",
            "worktree-pending-structured-output",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-pending-structured-output",
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
        "provider-run-pending-structured-output",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-opencode-pending-output".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-opencode-runtime".to_string()),
        },
    );
    match state {
        crate::provider::ProviderRunState::Parked => run.mark_parked(),
        crate::provider::ProviderRunState::Ended => run.mark_ended(),
        other => panic!("unsupported terminal test state: {other:?}"),
    }
    app.providers_mut().insert_run_for_test(run.clone());
    app.update_provider_run_projection(run.clone());
    let expected = crate::terminal::TerminalOutputRecord {
        record_id: None,
        timestamp_ms: 1_000,
        session_id: session.id().to_string(),
        provider_run_id: run.id().to_string(),
        agent_id: Some(agent.id().to_string()),
        prompt_id: None,
        prompt_origin: None,
        source_attachment_id: None,
        kind: crate::terminal::TerminalOutputKind::ProviderOutput,
        merge_key: None,
        recipient_attachment_ids: vec![attachment.id().to_string()],
        pending_recipient_attachment_ids: vec![attachment.id().to_string()],
        bytes: b"completed output".to_vec(),
        external_observation_metadata: None,
    };
    let output_store = app.structured_output_record_store();
    output_store.append(run.id().to_string(), vec![expected.clone()]);
    output_store.schedule_next_poll(run.id().to_string(), 2_000);

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let records = runtime
        .pump_owned_provider_output(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            true,
        )
        .await
        .expect("owned provider output pump should succeed");

    assert_eq!(records, vec![expected], "state was {state:?}");
    assert_eq!(output_store.poll_due_at_ms(run.id()), None);
    assert!(output_store.take(run.id()).is_empty());
}

#[tokio::test]
async fn owned_output_pump_drains_completed_pending_output_after_run_quiesces() {
    assert_owned_output_pump_drains_pending_record_after_run_state_change(
        crate::provider::ProviderRunState::Parked,
    )
    .await;
    assert_owned_output_pump_drains_pending_record_after_run_state_change(
        crate::provider::ProviderRunState::Ended,
    )
    .await;
}

#[tokio::test]
async fn live_structured_poll_failures_are_retried_then_surfaced() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-structured-poll-retry",
            "worktree-structured-poll-retry",
        ))
        .expect("session should be created");
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt-5.6-luna",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-structured-poll-retry",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-codex-structured-poll-retry".to_string(),
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
    let output_store = app.structured_output_record_store();
    output_store.mark_poll_enqueued(run.id(), None);
    app.providers_mut()
        .push_finished_structured_output_poll_for_test(
            run.id().to_string(),
            Err(crate::error::DaemonError::ProviderProtocol {
                provider_run_id: run.id().to_string(),
                operation: "thread/turns/list",
                message: "new rollout is temporarily empty".to_string(),
            }),
        );

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let records = runtime
        .pump_owned_structured_provider_output(session.id(), run.id(), Vec::new())
        .await
        .expect("a live structured poll failure should be deferred");

    assert!(records.is_empty());
    assert!(
        output_store.poll_due_at_ms(run.id()).is_some(),
        "the live provider run must remain scheduled after a transient poll failure",
    );
    assert!(
        !output_store.poll_due(run.id(), crate::session::unix_epoch_ms()),
        "the retry should respect the empty-poll backoff",
    );

    for attempt in 2..=crate::app::provider_output::STRUCTURED_OUTPUT_POLL_FAILURE_RETRY_LIMIT {
        output_store.mark_poll_enqueued(run.id(), None);
        app.lock()
            .await
            .providers_mut()
            .push_finished_structured_output_poll_for_test(
                run.id().to_string(),
                Err(crate::error::DaemonError::ProviderProtocol {
                    provider_run_id: run.id().to_string(),
                    operation: "thread/turns/list",
                    message: "new rollout is temporarily empty".to_string(),
                }),
            );
        let result = runtime
            .pump_owned_structured_provider_output(session.id(), run.id(), Vec::new())
            .await;
        if attempt < crate::app::provider_output::STRUCTURED_OUTPUT_POLL_FAILURE_RETRY_LIMIT {
            assert!(result
                .expect("a transient structured poll failure should be deferred")
                .is_empty());
            assert!(output_store.poll_due_at_ms(run.id()).is_some());
        } else {
            assert!(matches!(
                result,
                Err(crate::error::DaemonError::ProviderProtocol { .. })
            ));
            assert!(output_store.poll_due_at_ms(run.id()).is_none());
            assert!(!output_store.poll_due(run.id(), u64::MAX));
        }
    }
}

#[tokio::test]
async fn structured_output_batch_fans_out_chunks_with_one_terminal_notification() {
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
            "client-structured-batch",
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
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(agent.id()),
        "status\n",
        Vec::new(),
    )
    .expect("prompt should start");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let terminal = runtime.owned.terminal_stream.clone();
    let before = terminal.attachment_change_sequence(session.id(), attachment.id());
    let records = runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            crate::provider::ProviderPromptSignalBatch {
                chunks: vec![
                    crate::provider::ProviderPromptChunk {
                        kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                        merge_key: Some("structured-batch-1".to_string()),
                        bytes: b"first".to_vec(),
                    },
                    crate::provider::ProviderPromptChunk {
                        kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                        merge_key: Some("structured-batch-2".to_string()),
                        bytes: vec![0xff, b's', b'e', b'c', b'o', b'n', b'd'],
                    },
                ],
                completions: vec![crate::provider::ProviderAssistantCompletion {
                    message_id: "structured-batch-2".to_string(),
                    completed_at_ms: crate::session::unix_epoch_ms(),
                }],
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("structured output batch should be accepted");

    assert_eq!(records.len(), 2);
    assert_eq!(records[0].bytes, b"first");
    assert_eq!(
        records[1].bytes,
        vec![0xff, b's', b'e', b'c', b'o', b'n', b'd']
    );
    assert_eq!(
        terminal.attachment_change_sequence(session.id(), attachment.id()),
        before + 2,
        "the output batch and its completion should each notify the attachment"
    );
    assert_eq!(
        terminal
            .drain_output_records(session.id(), attachment.id())
            .into_iter()
            .map(|record| record.bytes)
            .collect::<Vec<_>>(),
        vec![
            b"first".to_vec(),
            vec![0xff, b's', b'e', b'c', b'o', b'n', b'd']
        ]
    );
    assert_eq!(
        terminal
            .drain_completion_records(session.id(), attachment.id())
            .into_iter()
            .map(|completion| completion.message_id)
            .collect::<Vec<_>>(),
        vec!["structured-batch-2".to_string()]
    );
}

#[tokio::test]
async fn structured_output_usage_resolves_the_cloud_owners_local_account_authority() {
    let cloud_owner_user_id = "cloud-owner";
    let mut config = crate::config::DaemonConfig::for_tests();
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        user_id: cloud_owner_user_id.to_string(),
        ..Default::default()
    });
    let mut app = crate::test_support::bootstrap_authenticated_app(config)
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(
            crate::session::CreateSessionRequest::new(
                "workspace-cloud-owner-usage",
                "worktree-cloud-owner-usage",
            )
            .with_owner_user_id(cloud_owner_user_id),
        )
        .expect("cloud-owned session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::for_user(
            session.id(),
            "client-cloud-owner-usage",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
            cloud_owner_user_id,
        ))
        .expect("cloud owner should attach");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider run should launch through the local account authority");
    app.update_provider_run_projection(run.clone());
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(agent.id()),
        "status\n",
        Vec::new(),
    )
    .expect("prompt should start");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            crate::provider::ProviderPromptSignalBatch {
                chunks: vec![crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                    merge_key: Some("cloud-owner-usage-output".to_string()),
                    bytes: b"continued output".to_vec(),
                }],
                account_usage: Some(crate::account_profile::ProviderAccountUsageSnapshot {
                    profile_id: "default".to_string(),
                    provider: "claude".to_string(),
                    availability:
                        crate::account_profile::ProviderAccountUsageAvailability::Available,
                    meters: Vec::new(),
                    observed_at_ms: Some(1_000),
                    source: "cloud-owner-usage-test".to_string(),
                    management_url: None,
                }),
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("usage should update without aborting the output batch");

    let profile = runtime
        .owned
        .provider_account_profiles
        .get(crate::session::DEFAULT_LOCAL_USER_ID, "claude", "default")
        .expect("the local account authority profile should remain resolvable");
    assert_eq!(profile.usage.source, "cloud-owner-usage-test");
    assert_eq!(
        runtime
            .owned
            .terminal_stream
            .drain_output_records(session.id(), attachment.id())
            .into_iter()
            .map(|record| record.bytes)
            .collect::<Vec<_>>(),
        vec![b"continued output".to_vec()]
    );
}

#[tokio::test]
async fn structured_output_and_completion_fanout_respect_collaborator_trace_visibility() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-collaborator-trace-output",
            "worktree-collaborator-trace-output",
        ))
        .expect("session should be created");
    {
        let mut sessions = app.sessions_mut();
        for (invite_id, user_id, level, now_ms) in [
            (
                "invite-transparent-trace-output",
                "user-transparent",
                crate::session::CollaborationLevel::Transparent,
                1,
            ),
            (
                "invite-full-trace-output",
                "user-full",
                crate::session::CollaborationLevel::Full,
                2,
            ),
            (
                "invite-private-trace-output",
                "user-private",
                crate::session::CollaborationLevel::Private,
                3,
            ),
        ] {
            let (_, invite) = sessions
                .create_session_invite(
                    session.id(),
                    invite_id.to_string(),
                    "local".to_string(),
                    None,
                    Some(1),
                    level,
                )
                .expect("collaborator invite should be created");
            sessions
                .join_session_invite(
                    session.id(),
                    invite.invite_id(),
                    user_id.to_string(),
                    now_ms,
                )
                .expect("collaborator should join");
        }
    }
    let owner_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::for_user(
            session.id(),
            "client-trace-owner",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
            "local",
        ))
        .expect("owner attachment should attach");
    let transparent_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::for_user(
            session.id(),
            "client-trace-transparent",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
            "user-transparent",
        ))
        .expect("transparent collaborator attachment should attach");
    let full_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::for_user(
            session.id(),
            "client-trace-full",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
            "user-full",
        ))
        .expect("full collaborator attachment should attach");
    let private_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::for_user(
            session.id(),
            "client-trace-private",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
            "user-private",
        ))
        .expect("private collaborator attachment should attach");
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
    app.submit_prompt(
        session.id(),
        owner_attachment.id(),
        Some(agent.id()),
        "status\n",
        Vec::new(),
    )
    .expect("prompt should start");

    let recipient_attachment_ids = vec![
        owner_attachment.id().to_string(),
        transparent_attachment.id().to_string(),
        full_attachment.id().to_string(),
        private_attachment.id().to_string(),
    ];
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let terminal = runtime.owned.terminal_stream.clone();
    for attachment_id in &recipient_attachment_ids {
        terminal.drain_output_records(session.id(), attachment_id);
    }
    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            recipient_attachment_ids.clone(),
            crate::provider::ProviderPromptSignalBatch {
                chunks: vec![crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                    merge_key: Some("collaborator-semantic-output".to_string()),
                    bytes: b"semantic output".to_vec(),
                }],
                completions: vec![crate::provider::ProviderAssistantCompletion {
                    message_id: "collaborator-assistant-completion".to_string(),
                    completed_at_ms: 1_000,
                }],
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("structured output batch should be accepted");
    runtime.owned.fan_out_terminal_outputs_to_recipients(
        session.id(),
        recipient_attachment_ids,
        vec![
            super::super::prompt_transcript_owned_state::TerminalOutputBatchAppend {
                provider_run_id: run.id().to_string(),
                agent_id: Some(agent.id().to_string()),
                kind: crate::terminal::TerminalOutputKind::ProviderTerminal,
                merge_key: Some("collaborator-raw-terminal".to_string()),
                bytes: b"raw terminal paint".to_vec(),
            },
        ],
    );
    runtime.owned.echo_promoted_queued_prompt_to_attachments(
        session.id(),
        run.id(),
        "prompt-collaborator-trace-output",
        owner_attachment.id(),
        "shared owner prompt",
        &[],
    );

    let owner_records = terminal.drain_output_records(session.id(), owner_attachment.id());
    let transparent_records =
        terminal.drain_output_records(session.id(), transparent_attachment.id());
    let full_records = terminal.drain_output_records(session.id(), full_attachment.id());
    let private_records = terminal.drain_output_records(session.id(), private_attachment.id());

    assert_eq!(
        owner_records
            .iter()
            .map(|record| (record.kind.clone(), record.bytes.clone()))
            .collect::<Vec<_>>(),
        vec![
            (
                crate::terminal::TerminalOutputKind::ProviderOutput,
                b"semantic output".to_vec(),
            ),
            (
                crate::terminal::TerminalOutputKind::ProviderTerminal,
                b"raw terminal paint".to_vec(),
            ),
            (
                crate::terminal::TerminalOutputKind::PromptEcho,
                b"shared owner prompt\n".to_vec(),
            ),
        ]
    );
    for (label, records) in [("transparent", transparent_records), ("full", full_records)] {
        assert_eq!(records.len(), 2, "{label} collaborator output");
        assert_eq!(
            records[0].kind,
            crate::terminal::TerminalOutputKind::ProviderOutput,
            "{label} collaborator must receive semantic output"
        );
        assert_eq!(records[0].bytes, b"semantic output", "{label} output");
        assert_eq!(
            records[1].kind,
            crate::terminal::TerminalOutputKind::PromptEcho,
            "{label} collaborator must receive prompt echoes"
        );
        assert_eq!(records[1].bytes, b"shared owner prompt\n", "{label} echo");
    }
    assert!(
        private_records.is_empty(),
        "private collaborator must not receive another user's agent trace"
    );

    for (label, attachment_id, should_receive) in [
        ("owner", owner_attachment.id(), true),
        ("transparent", transparent_attachment.id(), true),
        ("full", full_attachment.id(), true),
        ("private", private_attachment.id(), false),
    ] {
        let completion_ids = terminal
            .drain_completion_records(session.id(), attachment_id)
            .into_iter()
            .map(|completion| completion.message_id)
            .collect::<Vec<_>>();
        let expected = if should_receive {
            vec!["collaborator-assistant-completion".to_string()]
        } else {
            Vec::new()
        };
        assert_eq!(
            completion_ids, expected,
            "{label} collaborator completion visibility"
        );
    }
}

#[tokio::test]
async fn structured_output_batch_persists_one_turn_id_for_all_chunks() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-structured-history-turn",
            "worktree-structured-history-turn",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-structured-history-turn",
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
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(agent.id()),
        "status\n",
        Vec::new(),
    )
    .expect("prompt should start");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let active_prompt = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should load")
        .active_prompt_for_agent(agent.id())
        .expect("active prompt should exist")
        .clone();
    runtime.start_active_turn_with_trace_id(
        session.id(),
        agent.id(),
        active_prompt.id(),
        run.id(),
        "trace-structured-history-turn",
    );
    let active_turn = runtime
        .owned
        .active_turns
        .get(run.id())
        .expect("active turn should be tracked");
    assert_eq!(
        active_turn.source_attachment_id.as_deref(),
        Some(active_prompt.source_attachment_id())
    );
    assert_eq!(
        active_turn.prompt_origin,
        Some(active_prompt.prompt_origin())
    );

    let records = runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            crate::provider::ProviderPromptSignalBatch {
                chunks: vec![
                    crate::provider::ProviderPromptChunk {
                        kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                        merge_key: Some("structured-history-turn-1".to_string()),
                        bytes: b"first".to_vec(),
                    },
                    crate::provider::ProviderPromptChunk {
                        kind: crate::terminal::TerminalOutputKind::ProviderReasoning,
                        merge_key: Some("structured-history-turn-2".to_string()),
                        bytes: b"second".to_vec(),
                    },
                ],
                ..crate::provider::ProviderPromptSignalBatch::default()
            },
        )
        .await
        .expect("structured output batch should be accepted");
    assert_eq!(records.len(), 2);

    let events = runtime
        .owned
        .operational_history_store
        .load_session_events(session.id(), Some(agent.id()))
        .expect("canonical operational events should load");
    let chunk_turn_ids = ["structured-history-turn-1", "structured-history-turn-2"]
        .into_iter()
        .map(|merge_key| {
            events
                .iter()
                .find(|event| {
                    event
                        .metadata
                        .get("merge_key")
                        .and_then(|value| value.as_str())
                        == Some(merge_key)
                })
                .unwrap_or_else(|| panic!("event for merge key {merge_key} should exist"))
                .turn_id
                .as_deref()
                .map(str::to_string)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        chunk_turn_ids,
        vec![
            Some("trace-structured-history-turn".to_string()),
            Some("trace-structured-history-turn".to_string())
        ]
    );
}

#[tokio::test]
async fn active_turn_trace_metadata_uses_prompt_owner_when_session_mirror_is_stale() {
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-stale-active-turn",
            "worktree-stale-active-turn",
        ))
        .expect("session should be created");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "codex",
                "default",
                "gpt-test",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider run should launch");
    let external_prompt = crate::session::PromptQueueItem::external_observed_running(
        "codex",
        "codex-thread-stale-active-turn",
        "codex-turn-stale-active-turn",
        agent.id(),
        "external prompt with owner-only metadata",
    );
    let external_prompt_id = external_prompt.id().to_string();
    app.prompt_owner_sync_external_active_prompt(session.id(), agent.id(), Some(external_prompt))
        .expect("external active prompt should sync");
    app.sessions_mut()
        .mirror_agent_prompt_state(
            session.id(),
            agent.id(),
            None,
            std::collections::VecDeque::new(),
        )
        .expect("test drift should clear stale session prompt mirror");
    assert!(
        app.sessions()
            .get_session(session.id())
            .expect("session should load")
            .active_prompt_for_agent(agent.id())
            .is_none(),
        "session mirror should not expose the active prompt"
    );

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime.start_active_turn_with_trace_id(
        session.id(),
        agent.id(),
        &external_prompt_id,
        run.id(),
        "trace-stale-active-turn",
    );

    let active_turn = runtime
        .owned
        .active_turns
        .get(run.id())
        .expect("active turn should be tracked");
    assert_eq!(
        active_turn.source_attachment_id.as_deref(),
        Some("external:codex")
    );
    assert_eq!(
        active_turn.prompt_origin,
        Some(crate::session::PromptOrigin::External)
    );
    let external_observed_id = active_turn
        .external_observed_id
        .expect("external observed id should come from prompt owner");
    assert_eq!(external_observed_id.provider, "codex");
    assert_eq!(
        external_observed_id.provider_session_id,
        "codex-thread-stale-active-turn"
    );
    assert_eq!(
        external_observed_id.provider_turn_id,
        "codex-turn-stale-active-turn"
    );
}

#[tokio::test]
async fn pty_output_pump_batches_chunks_with_one_terminal_notification() {
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
            "client-pty-batch",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "dev-stub",
        "claude-code",
        "default",
        "sonnet",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-pty-batch",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "dev-stub:pty-batch".to_string(),
            pty_target: Some("stub-pty:pty-batch".to_string()),
            pty_program: Some("/bin/sh".to_string()),
            pty_args: vec![
                "-lc".to_string(),
                "sleep 0.1; printf pty-one; sleep 0.05; printf pty-two; sleep 5".to_string(),
            ],
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.update_provider_run_projection(run.clone());
    app.pty_mut()
        .spawn_for_run(&run)
        .expect("pty-backed provider run should spawn");
    let pty_output_signal = app.pty_output_signal();
    let initial_pty_sequence = pty_output_signal.sequence();
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(agent.id()),
        "status\n",
        Vec::new(),
    )
    .expect("prompt should start");

    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while pty_output_signal.sequence() < initial_pty_sequence + 2 {
            let sequence = pty_output_signal.sequence();
            pty_output_signal.wait_for_change_after(sequence).await;
        }
    })
    .await
    .expect("both delayed PTY writes should be available before the batch pump");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let terminal = runtime.owned.terminal_stream.clone();
    let before = terminal.attachment_change_sequence(session.id(), attachment.id());
    let records = runtime
        .pump_owned_provider_output(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            true,
        )
        .await
        .expect("pty output pump should accept batched chunks");

    assert!(!records.is_empty(), "PTY output pump should return records");
    let output = records
        .iter()
        .flat_map(|record| record.bytes.clone())
        .collect::<Vec<u8>>();
    let output = String::from_utf8_lossy(&output);
    assert!(output.contains("pty-one"));
    assert!(output.contains("pty-two"));
    assert_eq!(
        terminal.attachment_change_sequence(session.id(), attachment.id()),
        before + 1,
        "PTY output chunks should use one terminal batch notification"
    );

    let mut app = app.lock().await;
    let _ = app.pty_mut().remove_process(run.id());
}

#[tokio::test]
async fn idle_claude_native_tui_projects_startup_terminal_without_history() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-idle-claude-native-runtime",
            "worktree-idle-claude-native-runtime",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-idle-claude-native-runtime",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        "claude",
        "default",
        "sonnet",
    )
    .with_agent_id(agent.id())
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-idle-claude-native-runtime",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "claude:native-idle-runtime".to_string(),
            pty_target: Some("claude:native-idle-runtime".to_string()),
            pty_program: Some("/bin/sh".to_string()),
            pty_args: vec![
                "-lc".to_string(),
                "printf '\\033[?2004hClaude Code\\n'; sleep 5".to_string(),
            ],
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.update_provider_run_projection(run.clone());
    app.pty_mut()
        .spawn_for_run(&run)
        .expect("Claude native PTY should spawn");
    let history_count = app
        .load_session_history_entries(&session, Some(agent.id()))
        .expect("history should load")
        .len();

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let records = runtime
        .pump_owned_provider_output(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            true,
        )
        .await
        .expect("idle Claude native startup output should project");

    assert!(
        !records.is_empty(),
        "Claude startup frame should reach the client"
    );
    assert!(records
        .iter()
        .all(|record| record.kind == crate::terminal::TerminalOutputKind::ProviderTerminal));
    let output = records
        .iter()
        .flat_map(|record| record.bytes.clone())
        .collect::<Vec<_>>();
    let output = String::from_utf8_lossy(&output);
    assert!(output.contains("Claude Code"));
    assert!(output.contains("\u{1b}[?2004h"));

    let mut app = app.lock().await;
    assert_eq!(
        app.load_session_history_entries(&session, Some(agent.id()))
            .expect("history should still load")
            .len(),
        history_count,
        "transient terminal paint must stay out of semantic history"
    );
    let _ = app.pty_mut().remove_process(run.id());
}
