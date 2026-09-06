use super::*;

fn materialize_empty_codex_profile_for_test(app: &DaemonApp, owner_user_id: &str) {
    app.provider_account_profile_registry()
        .materialize_replica(
            owner_user_id,
            &crate::account_profile::ProviderAccountMaterialization {
                profile: crate::account_profile::ProviderAccountReplicaMetadata {
                    owner_user_id: owner_user_id.to_string(),
                    provider: "codex".to_string(),
                    profile_id: "default".to_string(),
                    label: "Default".to_string(),
                    origin: crate::account_profile::ProviderAccountProfileOrigin::Default,
                    is_default: true,
                },
                files: Vec::new(),
                generated_at_ms: crate::session::unix_epoch_ms(),
            },
        )
        .expect("test worker should materialize the selected Codex profile");
    crate::test_support::authenticate_provider_account(
        &app.provider_account_profile_registry(),
        owner_user_id,
        "codex",
        "default",
    )
    .expect("synthetic worker account should be authenticated");
}

#[test]
fn leased_agents_can_submit_and_complete_prompts_through_backing_session() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    config.user_config.providers.workspace_live_sync =
        crate::config::WorkspaceLiveSyncConfig::from_mode(
            crate::config::WorkspaceLiveSyncMode::Managed,
        );
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "session-1",
            "agent-home-1",
            false,
            "user-home",
        )
        .expect("execution lease should be created");
    let leased_agent = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
            Some("sonnet".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("leased agent should be created");

    let hidden_backing_session = app
        .sessions()
        .get_session(&leased_agent.backing_session_id)
        .expect("backing session should exist");
    assert!(hidden_backing_session.is_hidden());
    let backing_attachment = app
        .attachments()
        .get_attachment(&leased_agent.backing_attachment_id)
        .expect("leased backing attachment should exist");
    assert_eq!(backing_attachment.owner_user_id(), "user-home");
    assert!(app
        .sessions()
        .list_sessions()
        .into_iter()
        .all(|session| session.id() != leased_agent.backing_session_id));

    let scheduled_context = "<!-- chariox-prompt-manifest-entry:{\"template_id\":\"runtime/scheduled-prompt\",\"sha256\":\"scheduled-hash\"} -->\n<chariox-scheduled-prompt schedule_id=\"schedule-remote\">";
    let (provider_run_id, outcome) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt_with_hidden_context(
            &leased_agent.id,
            "remote leased prompt\n",
            scheduled_context,
        )
        .expect("leased prompt should submit");
    match outcome {
        PromptSubmissionOutcome::Started { .. } => {}
        other => panic!("unexpected prompt submission outcome: {other:?}"),
    }

    let provider_run = app
        .providers()
        .get_run(&provider_run_id)
        .expect("provider run should exist");
    assert!(provider_run.requires_workspace_live_sync());
    assert_eq!(provider_run.session_id(), leased_agent.backing_session_id);
    assert_eq!(
        provider_run.agent_instance_id(),
        Some(leased_agent.backing_agent_id.as_str())
    );
    let backing_active = app
        .prompt_owner_active_prompt_for_agent(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        )
        .expect("backing prompt should be active")
        .expect("backing prompt should exist");
    assert!(backing_active
        .hidden_system_context()
        .contains("<chariox-scheduled-prompt schedule_id=\"schedule-remote\">"));

    let completion = RemoteLeaseRuntime::new(&mut app)
        .complete_leased_prompt(&leased_agent.id)
        .expect("leased prompt should complete");
    assert_eq!(
        completion.completed.target_agent_id(),
        leased_agent.backing_agent_id
    );
}

#[test]
fn leased_prompt_submit_replays_the_active_run_for_the_same_home_prompt_id() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "session-idempotent-submit",
            "agent-home-idempotent-submit",
            false,
            "user-home",
        )
        .expect("execution lease should be created");
    let leased_agent = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
            Some("sonnet".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("leased agent should be created");
    let git_context = crate::transport::relay_peer::RemoteGitTurnContext {
        home_session_id: "session-idempotent-submit".to_string(),
        home_agent_id: "agent-home-idempotent-submit".to_string(),
        home_prompt_id: "home-prompt-idempotent".to_string(),
        home_turn_id: "home-prompt-idempotent".to_string(),
        source_attachment_id: None,
        workspace_live_sync_mode: None,
        prompt_origin: Some(PromptOrigin::Chariox),
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        prompt_summary: "idempotent remote prompt".to_string(),
    };

    let (first_run_id, first_outcome) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt_with_workflow_context(
            &leased_agent.id,
            "idempotent remote prompt\n",
            Vec::new(),
            None,
            Some(git_context.clone()),
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
        .expect("first leased prompt should submit");
    let PromptSubmissionOutcome::Started {
        prompt: first_prompt,
    } = first_outcome
    else {
        panic!("first leased prompt should start");
    };

    let initial_projection = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased_agent.id, &first_run_id, false)
        .expect("initial leased prompt projection should drain");
    if let Some((_target, RelayPeerEvent::LeasedRuntimeProjection { prompts, .. })) =
        initial_projection
    {
        assert!(
            prompts.is_empty(),
            "the worker copy of a home prompt must not be projected back as a second native prompt"
        );
    }

    let (replayed_run_id, replayed_outcome) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt_with_workflow_context(
            &leased_agent.id,
            "idempotent remote prompt\n",
            Vec::new(),
            None,
            Some(git_context),
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
        .expect("duplicate leased prompt should replay its accepted result");
    let PromptSubmissionOutcome::Started {
        prompt: replayed_prompt,
    } = replayed_outcome
    else {
        panic!("duplicate leased prompt should replay the active outcome");
    };

    assert_eq!(replayed_run_id, first_run_id);
    assert_eq!(replayed_prompt.id(), first_prompt.id());
    assert_eq!(
        app.prompt_owner_queued_prompt_count_for_agent(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        )
        .expect("queue count should load"),
        0,
    );
}

#[test]
fn leased_prompt_identity_uses_the_accepted_prompt_timestamp_when_backing_activity_is_stale() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "session-prompt-watermark",
            "agent-home-prompt-watermark",
            false,
            "user-home",
        )
        .expect("execution lease should be created");
    let leased_agent = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
            Some("sonnet".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("leased agent should be created");
    let remote_context = |prompt_id: &str| crate::transport::relay_peer::RemoteGitTurnContext {
        home_session_id: "session-prompt-watermark".to_string(),
        home_agent_id: "agent-home-prompt-watermark".to_string(),
        home_prompt_id: prompt_id.to_string(),
        home_turn_id: prompt_id.to_string(),
        source_attachment_id: None,
        workspace_live_sync_mode: None,
        prompt_origin: Some(PromptOrigin::Chariox),
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        prompt_summary: prompt_id.to_string(),
    };

    let (_, first_outcome) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt_with_workflow_context(
            &leased_agent.id,
            "first remote prompt\n",
            Vec::new(),
            None,
            Some(remote_context("home-prompt-1")),
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
        .expect("first leased prompt should submit");
    let PromptSubmissionOutcome::Started {
        prompt: first_prompt,
    } = first_outcome
    else {
        panic!("first leased prompt should start");
    };
    RemoteLeaseRuntime::new(&mut app)
        .clear_active_home_prompt_projection_for_test(&leased_agent.id);
    std::thread::sleep(std::time::Duration::from_millis(2));

    let (_, second_outcome) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt_with_workflow_context(
            &leased_agent.id,
            "second remote prompt\n",
            Vec::new(),
            None,
            Some(remote_context("home-prompt-2")),
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
        .expect("second leased prompt should be accepted behind stale backing activity");
    let PromptSubmissionOutcome::Queued {
        prompt: second_prompt,
    } = second_outcome
    else {
        panic!("second leased prompt should queue behind stale backing activity");
    };
    let projected = RemoteLeaseRuntime::new(&mut app)
        .leased_agent_snapshot_for_test(&leased_agent.id)
        .expect("leased agent projection should load");

    assert!(second_prompt.created_at_ms() >= first_prompt.created_at_ms());
    assert_eq!(
        projected.active_home_prompt_id.as_deref(),
        Some("home-prompt-2")
    );
    assert_eq!(
        projected.active_home_prompt_started_at_ms,
        Some(second_prompt.created_at_ms()),
    );
}

#[test]
fn leased_projection_recovers_a_queued_prompt_left_idle_by_completion_reordering() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "session-queue-recovery",
            "agent-home-queue-recovery",
            false,
            "user-home",
        )
        .expect("execution lease should be created");
    let leased_agent = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
            Some("sonnet".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("leased agent should be created");

    let (_provider_run_id, first) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt(&leased_agent.id, "first leased prompt\n", Vec::new())
        .expect("first leased prompt should submit");
    assert!(matches!(first, PromptSubmissionOutcome::Started { .. }));
    let (_provider_run_id, second) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt(&leased_agent.id, "second leased prompt\n", Vec::new())
        .expect("second leased prompt should submit");
    assert!(matches!(second, PromptSubmissionOutcome::Queued { .. }));

    app.prompt_owner_complete_active_prompt_only(
        &leased_agent.backing_session_id,
        &leased_agent.backing_agent_id,
    )
    .expect("completion race should leave the first prompt settled");
    assert!(app
        .prompt_owner_active_prompt_for_agent(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        )
        .expect("active prompt state should load")
        .is_none());

    let recovered = RemoteLeaseRuntime::new(&mut app)
        .recover_idle_leased_prompt_queue(&leased_agent.id)
        .expect("idle leased queue recovery should succeed")
        .expect("queued prompt should be promoted");
    assert_eq!(recovered.prompt(), "second leased prompt\n");
    assert!(app
        .prompt_owner_peek_next_queued_prompt(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        )
        .expect("queued prompt state should load")
        .is_none());
    assert_eq!(
        app.prompt_owner_active_prompt_for_agent(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        )
        .expect("active prompt state should load")
        .expect("recovered prompt should be active")
        .id(),
        recovered.id()
    );
}

#[test]
fn leased_projection_keeps_queued_prompt_while_provider_run_is_starting() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "session-starting-provider",
            "agent-home-starting-provider",
            false,
            "user-home",
        )
        .expect("execution lease should be created");
    let leased_agent = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
            Some("sonnet".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("leased agent should be created");

    let prepared = RemoteLeaseRuntime::new(&mut app)
        .prepare_leased_prompt_submission(
            &leased_agent.id,
            "queued while provider starts\n",
            "",
            Vec::new(),
            None,
            None,
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
        .expect("leased prompt should prepare");
    let request = match &prepared.provider_run {
        crate::app::PreparedLeasedProviderRun::LaunchRequired(request) => request.clone(),
        crate::app::PreparedLeasedProviderRun::Ready(_) => {
            panic!("first leased prompt should require a provider launch")
        }
    };
    let started = app
        .start_provider_launch(request)
        .expect("provider launch should enter starting state");
    let (_provider_run_id, outcome) = RemoteLeaseRuntime::new(&mut app)
        .finish_prepared_leased_prompt_submission(prepared, started.run.id().to_string())
        .expect("prompt should queue behind provider startup");
    assert!(matches!(outcome, PromptSubmissionOutcome::Queued { .. }));

    let recovered = RemoteLeaseRuntime::new(&mut app)
        .recover_idle_leased_prompt_queue(&leased_agent.id)
        .expect("startup queue recovery should defer cleanly");
    assert!(recovered.is_none());
    assert_eq!(
        app.prompt_owner_peek_next_queued_prompt(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        )
        .expect("queued prompt state should load")
        .expect("startup must not drop the queued prompt")
        .prompt(),
        "queued while provider starts\n"
    );
    assert!(app
        .prompt_owner_active_prompt_for_agent(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        )
        .expect("active prompt state should load")
        .is_none());
}

#[test]
fn queued_leased_workflow_context_rotates_by_backing_prompt_after_completion() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "session-leased-workflow-queue",
            "agent-home-leased-workflow-queue",
            false,
            "user-home",
        )
        .expect("execution lease should be created");
    let leased_agent = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
            Some("sonnet".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("leased agent should be created");

    let first_home_prompt_id = "home-leased-workflow-first";
    let (first_provider_run_id, first_outcome) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt_with_workflow_context(
            &leased_agent.id,
            "first leased workflow prompt\n",
            Vec::new(),
            Some(crate::execution_lease::RemoteWorkflowTurnContext {
                home_kernel_id: "home-kernel".to_string(),
                home_session_id: "session-leased-workflow-queue".to_string(),
                home_agent_id: "agent-home-leased-workflow-queue".to_string(),
                workflow_run_id: "workflow-first".to_string(),
                workflow_node_run_id: "node-first".to_string(),
                delivery_token: "delivery-first".to_string(),
                event_reply_enabled: true,
                event_context_enabled: false,
                event_actions_enabled: false,
            }),
            Some(crate::transport::relay_peer::RemoteGitTurnContext {
                home_session_id: "session-leased-workflow-queue".to_string(),
                home_agent_id: "agent-home-leased-workflow-queue".to_string(),
                home_prompt_id: first_home_prompt_id.to_string(),
                home_turn_id: first_home_prompt_id.to_string(),
                source_attachment_id: None,
                workspace_live_sync_mode: None,
                prompt_origin: Some(PromptOrigin::Chariox),
                external_provider: None,
                external_provider_session_id: None,
                external_provider_turn_id: None,
                prompt_summary: "first leased workflow prompt".to_string(),
            }),
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
        .expect("first leased workflow prompt should submit");
    assert!(matches!(
        first_outcome,
        PromptSubmissionOutcome::Started { .. }
    ));
    assert!(app
        .providers()
        .get_run(&first_provider_run_id)
        .expect("first provider run should exist")
        .workflow_event_reply_enabled());

    let second_home_prompt_id = "home-leased-workflow-second";
    let (queued_provider_run_id, second_outcome) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt_with_workflow_context(
            &leased_agent.id,
            "second leased workflow prompt\n",
            Vec::new(),
            Some(crate::execution_lease::RemoteWorkflowTurnContext {
                home_kernel_id: "home-kernel".to_string(),
                home_session_id: "session-leased-workflow-queue".to_string(),
                home_agent_id: "agent-home-leased-workflow-queue".to_string(),
                workflow_run_id: "workflow-second".to_string(),
                workflow_node_run_id: "node-second".to_string(),
                delivery_token: "delivery-second".to_string(),
                event_reply_enabled: false,
                event_context_enabled: false,
                event_actions_enabled: false,
            }),
            Some(crate::transport::relay_peer::RemoteGitTurnContext {
                home_session_id: "session-leased-workflow-queue".to_string(),
                home_agent_id: "agent-home-leased-workflow-queue".to_string(),
                home_prompt_id: second_home_prompt_id.to_string(),
                home_turn_id: second_home_prompt_id.to_string(),
                source_attachment_id: None,
                workspace_live_sync_mode: None,
                prompt_origin: Some(PromptOrigin::Chariox),
                external_provider: None,
                external_provider_session_id: None,
                external_provider_turn_id: None,
                prompt_summary: "second leased workflow prompt".to_string(),
            }),
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
        .expect("second leased workflow prompt should submit");
    let second_backing_prompt_id = match &second_outcome {
        PromptSubmissionOutcome::Queued { prompt } => prompt.id().to_string(),
        other => panic!("second workflow prompt should queue: {other:?}"),
    };
    let second_prompt_text = match &second_outcome {
        PromptSubmissionOutcome::Queued { prompt } => prompt.prompt().to_string(),
        other => panic!("second workflow prompt should queue: {other:?}"),
    };
    assert_eq!(queued_provider_run_id, first_provider_run_id);

    RemoteLeaseRuntime::new(&mut app)
        .complete_leased_workflow_prompt_for_provider_run(&first_provider_run_id)
        .expect("first leased workflow prompt should complete")
        .expect("first completion should settle the active prompt");
    let promoted = app
        .prompt_owner_active_prompt_for_agent(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        )
        .expect("active prompt state should load")
        .or_else(|| {
            RemoteLeaseRuntime::new(&mut app)
                .recover_idle_leased_prompt_queue(&leased_agent.id)
                .expect("queued leased workflow prompt should recover")
        })
        .expect("queued leased workflow prompt should be promoted");
    assert_eq!(promoted.prompt(), second_prompt_text);

    let second_provider_run_id = app
        .providers()
        .get_run_for_agent(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        )
        .expect("replacement provider run should be active")
        .id()
        .to_string();
    assert_ne!(second_provider_run_id, first_provider_run_id);
    assert!(!app
        .providers()
        .get_run(&second_provider_run_id)
        .expect("replacement provider run should exist")
        .workflow_event_reply_enabled());
    assert_eq!(
        RemoteLeaseRuntime::new(&mut app)
            .leased_agent_snapshot_for_test(&leased_agent.id)
            .and_then(|agent| agent.active_home_prompt_id),
        Some(second_home_prompt_id.to_string())
    );
    assert_eq!(
        RemoteLeaseRuntime::new(&mut app)
            .leased_workflow_turn_binding_for_test(second_home_prompt_id),
        Some((second_backing_prompt_id, second_provider_run_id, false))
    );
    assert!(!RemoteLeaseRuntime::new(&mut app)
        .has_leased_workflow_turn_binding_for_test(first_home_prompt_id));
}

#[test]
fn leased_workflow_bindings_do_not_overwrite_equal_home_prompt_ids() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");

    let lease_one = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel-one",
            "session-leased-one",
            "agent-home-one",
            false,
            "user-home",
        )
        .expect("first execution lease should be created");
    let agent_one = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease_one.id,
            "managed-dev-stub",
            "default",
            Some("sonnet".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("first leased agent should be created");
    let lease_two = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel-two",
            "session-leased-two",
            "agent-home-two",
            false,
            "user-home",
        )
        .expect("second execution lease should be created");
    let agent_two = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease_two.id,
            "managed-dev-stub",
            "default",
            Some("sonnet".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("second leased agent should be created");

    let submit = |app: &mut DaemonApp,
                  agent: &crate::execution_lease::LeasedAgent,
                  home_kernel_id: &str,
                  home_session_id: &str,
                  home_agent_id: &str,
                  workflow_run_id: &str,
                  event_reply_enabled: bool| {
        RemoteLeaseRuntime::new(app).submit_leased_prompt_with_workflow_context(
            &agent.id,
            "same home prompt id from independent lease\n",
            Vec::new(),
            Some(crate::execution_lease::RemoteWorkflowTurnContext {
                home_kernel_id: home_kernel_id.to_string(),
                home_session_id: home_session_id.to_string(),
                home_agent_id: home_agent_id.to_string(),
                workflow_run_id: workflow_run_id.to_string(),
                workflow_node_run_id: format!("{workflow_run_id}-node"),
                delivery_token: format!("{workflow_run_id}-delivery"),
                event_reply_enabled,
                event_context_enabled: false,
                event_actions_enabled: false,
            }),
            Some(crate::transport::relay_peer::RemoteGitTurnContext {
                home_session_id: home_session_id.to_string(),
                home_agent_id: home_agent_id.to_string(),
                home_prompt_id: "same-home-prompt".to_string(),
                home_turn_id: "same-home-prompt".to_string(),
                source_attachment_id: None,
                workspace_live_sync_mode: None,
                prompt_origin: Some(PromptOrigin::Chariox),
                external_provider: None,
                external_provider_session_id: None,
                external_provider_turn_id: None,
                prompt_summary: "same home prompt id from independent lease".to_string(),
            }),
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
    };

    let (first_provider_run_id, first_outcome) = submit(
        &mut app,
        &agent_one,
        "home-kernel-one",
        "session-leased-one",
        "agent-home-one",
        "workflow-one",
        true,
    )
    .expect("first workflow prompt should submit");
    let first_backing_prompt_id = match first_outcome {
        PromptSubmissionOutcome::Started { prompt } => prompt.id().to_string(),
        other => panic!("first workflow prompt should start: {other:?}"),
    };
    let (second_provider_run_id, second_outcome) = submit(
        &mut app,
        &agent_two,
        "home-kernel-two",
        "session-leased-two",
        "agent-home-two",
        "workflow-two",
        false,
    )
    .expect("second workflow prompt should submit");
    let second_backing_prompt_id = match second_outcome {
        PromptSubmissionOutcome::Started { prompt } => prompt.id().to_string(),
        other => panic!("second workflow prompt should start: {other:?}"),
    };

    assert_ne!(first_backing_prompt_id, second_backing_prompt_id);
    assert_eq!(
        RemoteLeaseRuntime::new(&mut app)
            .leased_workflow_turn_binding_count_for_test("same-home-prompt"),
        2,
        "independent leases may reuse a home prompt id without overwriting context"
    );

    RemoteLeaseRuntime::new(&mut app)
        .complete_leased_workflow_prompt_for_provider_run(&first_provider_run_id)
        .expect("first workflow prompt should complete")
        .expect("first workflow completion should settle");
    assert_eq!(
        RemoteLeaseRuntime::new(&mut app)
            .leased_workflow_turn_binding_count_for_test("same-home-prompt"),
        1,
    );
    assert_eq!(
        RemoteLeaseRuntime::new(&mut app).leased_workflow_turn_binding_for_test("same-home-prompt"),
        Some((second_backing_prompt_id, second_provider_run_id, false)),
        "completing one lease must leave the other lease's event context intact"
    );
}

#[test]
fn leased_projection_forwards_completion_when_backing_prompt_already_settled() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "session-1",
            "agent-home-1",
            false,
            "user-home",
        )
        .expect("execution lease should be created");
    let leased_agent = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
            Some("sonnet".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("leased agent should be created");

    let (provider_run_id, outcome) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt(&leased_agent.id, "remote leased prompt\n", Vec::new())
        .expect("leased prompt should submit");
    match outcome {
        PromptSubmissionOutcome::Started { .. } => {}
        other => panic!("unexpected prompt submission outcome: {other:?}"),
    }
    let started_at_ms = RemoteLeaseRuntime::new(&mut app)
        .leased_agent_snapshot_for_test(&leased_agent.id)
        .and_then(|agent| agent.active_home_prompt_started_at_ms)
        .expect("active home prompt should remember its worker start time");
    app.terminal_mut().record_assistant_message_completion(
        &leased_agent.backing_session_id,
        &provider_run_id,
        Some(&leased_agent.backing_agent_id),
        vec![leased_agent.backing_attachment_id.clone()],
        "assistant-msg-1",
        started_at_ms.saturating_add(1),
    );
    app.complete_active_prompt(
        &leased_agent.backing_session_id,
        &leased_agent.backing_agent_id,
        Some(&provider_run_id),
    )
    .expect("backing prompt should settle first");

    let (_target_kernel_id, event) = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
        .expect("settled backing prompt should not block completion projection")
        .expect("completion projection should be emitted");
    let RelayPeerEvent::LeasedRuntimeProjection { completions, .. } = event;
    assert!(completions
        .iter()
        .any(|completion| completion.message_id == "assistant-msg-1"));
}

#[test]
fn leased_projection_drops_completion_records_older_than_the_active_home_prompt() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "session-current-prompt",
            "agent-home-current-prompt",
            false,
            "user-home",
        )
        .expect("execution lease should be created");
    let leased_agent = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
            Some("sonnet".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("leased agent should be created");
    let home_prompt_id = "home-prompt-current";
    let (provider_run_id, outcome) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt_with_workflow_context(
            &leased_agent.id,
            "current remote prompt\n",
            Vec::new(),
            None,
            Some(crate::transport::relay_peer::RemoteGitTurnContext {
                home_session_id: "session-current-prompt".to_string(),
                home_agent_id: "agent-home-current-prompt".to_string(),
                home_prompt_id: home_prompt_id.to_string(),
                home_turn_id: home_prompt_id.to_string(),
                source_attachment_id: None,
                workspace_live_sync_mode: None,
                prompt_origin: Some(PromptOrigin::Chariox),
                external_provider: None,
                external_provider_session_id: None,
                external_provider_turn_id: None,
                prompt_summary: "current remote prompt".to_string(),
            }),
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
        .expect("leased prompt should submit");
    assert!(matches!(outcome, PromptSubmissionOutcome::Started { .. }));
    let started_at_ms = RemoteLeaseRuntime::new(&mut app)
        .leased_agent_snapshot_for_test(&leased_agent.id)
        .and_then(|agent| agent.active_home_prompt_started_at_ms)
        .expect("active home prompt should remember its worker start time");

    app.terminal_mut().record_assistant_message_completion(
        &leased_agent.backing_session_id,
        &provider_run_id,
        Some(&leased_agent.backing_agent_id),
        vec![leased_agent.backing_attachment_id.clone()],
        "assistant-msg-stale",
        started_at_ms.saturating_sub(1),
    );
    let stale = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
        .expect("stale completion drain should succeed");
    assert!(stale.is_none());
    assert!(app
        .prompt_owner_active_prompt_for_agent_snapshot(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        )
        .expect("active prompt should load")
        .is_some());

    app.terminal_mut().record_assistant_message_completion(
        &leased_agent.backing_session_id,
        &provider_run_id,
        Some(&leased_agent.backing_agent_id),
        vec![leased_agent.backing_attachment_id.clone()],
        "assistant-msg-current",
        started_at_ms.saturating_add(1),
    );
    let current = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
        .expect("current completion drain should succeed")
        .expect("current completion should be projected");
    let RelayPeerEvent::LeasedRuntimeProjection { completions, .. } = current.1;
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].message_id, "assistant-msg-current");
    assert_eq!(
        completions[0].home_prompt_id.as_deref(),
        Some(home_prompt_id)
    );
}

#[test]
fn leased_projection_does_not_complete_a_running_turn_without_turn_evidence() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "session-1",
            "agent-home-1",
            false,
            "user-home",
        )
        .expect("execution lease should be created");
    let leased_agent = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
            Some("sonnet".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("leased agent should be created");
    let (provider_run_id, outcome) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt(&leased_agent.id, "remote leased prompt\n", Vec::new())
        .expect("leased prompt should submit");
    assert!(matches!(outcome, PromptSubmissionOutcome::Started { .. }));

    app.prompt_owner_complete_active_prompt_only(
        &leased_agent.backing_session_id,
        &leased_agent.backing_agent_id,
    )
    .expect("the simulated startup visibility gap should settle the backing prompt");

    let projection = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
        .expect("projection drain should succeed");
    if let Some((_target, RelayPeerEvent::LeasedRuntimeProjection { completions, .. })) = projection
    {
        assert!(
            completions.is_empty(),
            "a running provider needs output or an explicit completion before home may settle"
        );
    }
}

#[test]
fn leased_projection_pull_replays_a_completion_lost_after_worker_drain() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "session-1",
            "agent-home-1",
            false,
            "user-home",
        )
        .expect("execution lease should be created");
    let leased_agent = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
            Some("sonnet".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("leased agent should be created");
    let home_prompt_id = "home-prompt-1";
    let (provider_run_id, outcome) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt_with_workflow_context(
            &leased_agent.id,
            "remote leased prompt\n",
            Vec::new(),
            None,
            Some(crate::transport::relay_peer::RemoteGitTurnContext {
                home_session_id: "session-1".to_string(),
                home_agent_id: "agent-home-1".to_string(),
                home_prompt_id: home_prompt_id.to_string(),
                home_turn_id: home_prompt_id.to_string(),
                source_attachment_id: None,
                workspace_live_sync_mode: None,
                prompt_origin: Some(PromptOrigin::Chariox),
                external_provider: None,
                external_provider_session_id: None,
                external_provider_turn_id: None,
                prompt_summary: "remote leased prompt".to_string(),
            }),
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
        .expect("leased prompt should submit");
    assert!(matches!(outcome, PromptSubmissionOutcome::Started { .. }));
    app.terminal_mut().record_assistant_message_completion(
        &leased_agent.backing_session_id,
        &provider_run_id,
        Some(&leased_agent.backing_agent_id),
        vec![leased_agent.backing_attachment_id.clone()],
        "assistant-msg-1",
        1234,
    );
    app.complete_active_prompt(
        &leased_agent.backing_session_id,
        &leased_agent.backing_agent_id,
        Some(&provider_run_id),
    )
    .expect("backing prompt should settle");

    let first = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
        .expect("first projection drain should succeed")
        .expect("first projection should carry completion");
    let RelayPeerEvent::LeasedRuntimeProjection {
        completions: first_completions,
        ..
    } = first.1;
    assert!(!first_completions.is_empty());
    assert!(first_completions
        .iter()
        .all(|completion| { completion.home_prompt_id.as_deref() == Some(home_prompt_id) }));

    let replay = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection_with_recovery(
            &leased_agent.id,
            &provider_run_id,
            false,
            true,
        )
        .expect("recovery projection drain should succeed")
        .expect("a lost completion should remain replayable");
    let RelayPeerEvent::LeasedRuntimeProjection {
        completions: replayed_completions,
        ..
    } = replay.1;
    assert_eq!(replayed_completions.len(), 1);
    assert_eq!(
        replayed_completions[0].home_prompt_id.as_deref(),
        Some(home_prompt_id)
    );

    let replay_again = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection_with_recovery(
            &leased_agent.id,
            &provider_run_id,
            false,
            true,
        )
        .expect("second recovery projection drain should succeed")
        .expect("completion replay should remain available until home settles");
    let RelayPeerEvent::LeasedRuntimeProjection {
        completions: replayed_again,
        ..
    } = replay_again.1;
    assert_eq!(replayed_again, replayed_completions);
}

#[test]
fn explicit_completion_replay_keeps_the_settled_home_prompt_output_key() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "session-explicit-replay",
            "agent-home-explicit-replay",
            false,
            "user-home",
        )
        .expect("execution lease should be created");
    let leased_agent = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
            Some("sonnet".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("leased agent should be created");
    let home_prompt_id = "home-prompt-explicit-replay";
    let (provider_run_id, outcome) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt_with_workflow_context(
            &leased_agent.id,
            "remote explicit completion prompt\n",
            Vec::new(),
            None,
            Some(crate::transport::relay_peer::RemoteGitTurnContext {
                home_session_id: "session-explicit-replay".to_string(),
                home_agent_id: "agent-home-explicit-replay".to_string(),
                home_prompt_id: home_prompt_id.to_string(),
                home_turn_id: home_prompt_id.to_string(),
                source_attachment_id: None,
                workspace_live_sync_mode: None,
                prompt_origin: Some(PromptOrigin::Chariox),
                external_provider: None,
                external_provider_session_id: None,
                external_provider_turn_id: None,
                prompt_summary: "remote explicit completion prompt".to_string(),
            }),
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
        .expect("leased prompt should submit");
    assert!(matches!(outcome, PromptSubmissionOutcome::Started { .. }));
    materialize_empty_codex_profile_for_test(&app, "user-home");
    RemoteLeaseRuntime::new(&mut app).set_leased_agent_provider_for_test(&leased_agent.id, "codex");
    app.fan_out_output_for_agent(
        &leased_agent.backing_session_id,
        &provider_run_id,
        Some(&leased_agent.backing_agent_id),
        crate::terminal::TerminalOutputKind::ProviderOutput,
        Some("assistant-output".to_string()),
        vec![leased_agent.backing_attachment_id.clone()],
        b"remote explicit completion output",
    );
    app.terminal_mut().record_assistant_message_completion(
        &leased_agent.backing_session_id,
        &provider_run_id,
        Some(&leased_agent.backing_agent_id),
        vec![leased_agent.backing_attachment_id.clone()],
        "assistant-explicit-replay",
        1234,
    );
    app.complete_active_prompt(
        &leased_agent.backing_session_id,
        &leased_agent.backing_agent_id,
        Some(&provider_run_id),
    )
    .expect("backing prompt should settle");

    let first = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
        .expect("first projection drain should succeed")
        .expect("first projection should carry completion");
    let RelayPeerEvent::LeasedRuntimeProjection { completions, .. } = first.1;
    assert_eq!(completions.len(), 1);

    let replay = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection_with_recovery(
            &leased_agent.id,
            &provider_run_id,
            false,
            true,
        )
        .expect("recovery projection drain should succeed")
        .expect("explicit completion should remain replayable");
    let RelayPeerEvent::LeasedRuntimeProjection { completions, .. } = replay.1;
    assert_eq!(completions.len(), 1);
    assert_eq!(
        completions[0].home_prompt_id.as_deref(),
        Some(home_prompt_id)
    );
}

#[test]
fn new_home_prompt_clears_prior_explicit_completion_replay() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "session-consecutive-explicit",
            "agent-home-consecutive-explicit",
            false,
            "user-home",
        )
        .expect("execution lease should be created");
    let leased_agent = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
            Some("sonnet".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("leased agent should be created");
    materialize_empty_codex_profile_for_test(&app, "user-home");
    RemoteLeaseRuntime::new(&mut app).set_leased_agent_provider_for_test(&leased_agent.id, "codex");

    let first_home_prompt_id = "home-prompt-consecutive-1";
    let (provider_run_id, first_outcome) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt_with_workflow_context(
            &leased_agent.id,
            "first remote prompt\n",
            Vec::new(),
            None,
            Some(crate::transport::relay_peer::RemoteGitTurnContext {
                home_session_id: "session-consecutive-explicit".to_string(),
                home_agent_id: "agent-home-consecutive-explicit".to_string(),
                home_prompt_id: first_home_prompt_id.to_string(),
                home_turn_id: first_home_prompt_id.to_string(),
                source_attachment_id: None,
                workspace_live_sync_mode: None,
                prompt_origin: Some(PromptOrigin::Chariox),
                external_provider: None,
                external_provider_session_id: None,
                external_provider_turn_id: None,
                prompt_summary: "first remote prompt".to_string(),
            }),
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
        .expect("first leased prompt should submit");
    assert!(matches!(
        first_outcome,
        PromptSubmissionOutcome::Started { .. }
    ));
    app.fan_out_output_for_agent(
        &leased_agent.backing_session_id,
        &provider_run_id,
        Some(&leased_agent.backing_agent_id),
        crate::terminal::TerminalOutputKind::ProviderOutput,
        Some("assistant-first".to_string()),
        vec![leased_agent.backing_attachment_id.clone()],
        b"first output",
    );
    app.terminal_mut().record_assistant_message_completion(
        &leased_agent.backing_session_id,
        &provider_run_id,
        Some(&leased_agent.backing_agent_id),
        vec![leased_agent.backing_attachment_id.clone()],
        "assistant-first",
        1234,
    );
    app.complete_active_prompt(
        &leased_agent.backing_session_id,
        &leased_agent.backing_agent_id,
        Some(&provider_run_id),
    )
    .expect("first backing prompt should settle");
    let first = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
        .expect("first projection drain should succeed")
        .expect("first completion should project");
    let RelayPeerEvent::LeasedRuntimeProjection {
        completions: first_completions,
        ..
    } = first.1;
    assert_eq!(
        first_completions[0].home_prompt_id.as_deref(),
        Some(first_home_prompt_id)
    );

    let second_home_prompt_id = "home-prompt-consecutive-2";
    let (second_provider_run_id, second_outcome) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt_with_workflow_context(
            &leased_agent.id,
            "second remote prompt\n",
            Vec::new(),
            None,
            Some(crate::transport::relay_peer::RemoteGitTurnContext {
                home_session_id: "session-consecutive-explicit".to_string(),
                home_agent_id: "agent-home-consecutive-explicit".to_string(),
                home_prompt_id: second_home_prompt_id.to_string(),
                home_turn_id: second_home_prompt_id.to_string(),
                source_attachment_id: None,
                workspace_live_sync_mode: None,
                prompt_origin: Some(PromptOrigin::Chariox),
                external_provider: None,
                external_provider_session_id: None,
                external_provider_turn_id: None,
                prompt_summary: "second remote prompt".to_string(),
            }),
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
        .expect("second leased prompt should submit");
    assert!(matches!(
        second_outcome,
        PromptSubmissionOutcome::Started { .. }
    ));
    assert_eq!(second_provider_run_id, provider_run_id);
    let second_snapshot = RemoteLeaseRuntime::new(&mut app)
        .leased_agent_snapshot_for_test(&leased_agent.id)
        .expect("leased agent should remain registered");
    assert_eq!(
        second_snapshot.active_home_prompt_id.as_deref(),
        Some(second_home_prompt_id)
    );
    assert!(
        second_snapshot.replayable_completion.is_none(),
        "the prior turn completion must not remain replayable after a new home prompt starts"
    );

    app.fan_out_output_for_agent(
        &leased_agent.backing_session_id,
        &provider_run_id,
        Some(&leased_agent.backing_agent_id),
        crate::terminal::TerminalOutputKind::ProviderOutput,
        Some("assistant-second".to_string()),
        vec![leased_agent.backing_attachment_id.clone()],
        b"second output",
    );
    app.terminal_mut().record_assistant_message_completion(
        &leased_agent.backing_session_id,
        &provider_run_id,
        Some(&leased_agent.backing_agent_id),
        vec![leased_agent.backing_attachment_id.clone()],
        "assistant-second",
        2345,
    );
    app.complete_active_prompt(
        &leased_agent.backing_session_id,
        &leased_agent.backing_agent_id,
        Some(&provider_run_id),
    )
    .expect("second backing prompt should settle");
    let second = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection_with_recovery(
            &leased_agent.id,
            &provider_run_id,
            false,
            true,
        )
        .expect("second projection drain should succeed")
        .expect("second completion should project");
    let RelayPeerEvent::LeasedRuntimeProjection {
        completions: second_completions,
        ..
    } = second.1;
    assert_eq!(second_completions.len(), 1);
    assert_eq!(
        second_completions[0].home_prompt_id.as_deref(),
        Some(second_home_prompt_id)
    );
}

#[test]
fn explicit_provider_synthesizes_completion_after_authoritative_output_only_settlement() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "session-explicit-output-only",
            "agent-home-explicit-output-only",
            false,
            "user-home",
        )
        .expect("execution lease should be created");
    let leased_agent = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
            Some("sonnet".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("leased agent should be created");
    materialize_empty_codex_profile_for_test(&app, "user-home");
    RemoteLeaseRuntime::new(&mut app).set_leased_agent_provider_for_test(&leased_agent.id, "codex");
    let home_prompt_id = "home-prompt-explicit-output-only";
    let (provider_run_id, outcome) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt_with_workflow_context(
            &leased_agent.id,
            "remote explicit output-only prompt\n",
            Vec::new(),
            None,
            Some(crate::transport::relay_peer::RemoteGitTurnContext {
                home_session_id: "session-explicit-output-only".to_string(),
                home_agent_id: "agent-home-explicit-output-only".to_string(),
                home_prompt_id: home_prompt_id.to_string(),
                home_turn_id: home_prompt_id.to_string(),
                source_attachment_id: None,
                workspace_live_sync_mode: None,
                prompt_origin: Some(PromptOrigin::Chariox),
                external_provider: None,
                external_provider_session_id: None,
                external_provider_turn_id: None,
                prompt_summary: "remote explicit output-only prompt".to_string(),
            }),
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
        .expect("leased prompt should submit");
    assert!(matches!(outcome, PromptSubmissionOutcome::Started { .. }));
    let launch_request = crate::provider::LaunchProviderRequest::new(
        &leased_agent.backing_session_id,
        "codex",
        "codex",
        "default",
        "gpt-5.4",
    )
    .with_agent_id(&leased_agent.backing_agent_id)
    .with_client_interface(crate::provider::ProviderClientInterface::Chariox);
    let mut running_provider = crate::provider::RuntimeProviderRun::new(
        &provider_run_id,
        &launch_request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "codex:codex:gpt-5.4".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    running_provider.mark_running();
    app.providers_mut().insert_run_for_test(running_provider);
    app.fan_out_output_for_agent(
        &leased_agent.backing_session_id,
        &provider_run_id,
        Some(&leased_agent.backing_agent_id),
        crate::terminal::TerminalOutputKind::ProviderOutput,
        Some("assistant-commentary".to_string()),
        vec![leased_agent.backing_attachment_id.clone()],
        b"I will run the check now.",
    );
    let output_event = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection_with_recovery(
            &leased_agent.id,
            &provider_run_id,
            false,
            true,
        )
        .expect("output projection drain should succeed")
        .expect("active output should project");
    let RelayPeerEvent::LeasedRuntimeProjection {
        output_chunks,
        completions,
        ..
    } = output_event.1;
    assert_eq!(output_chunks.len(), 1);
    assert!(
        completions.is_empty(),
        "provider output must not settle an active backing prompt"
    );
    app.complete_active_prompt(
        &leased_agent.backing_session_id,
        &leased_agent.backing_agent_id,
        Some(&provider_run_id),
    )
    .expect("authoritative provider turn should settle the backing prompt");
    let _locally_consumed_completion = app.terminal_mut().drain_completion_records(
        &leased_agent.backing_session_id,
        &leased_agent.backing_attachment_id,
    );

    let completion_event = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection_with_recovery(
            &leased_agent.id,
            &provider_run_id,
            false,
            true,
        )
        .expect("projection drain should succeed")
        .expect("authoritative settlement should project");
    let RelayPeerEvent::LeasedRuntimeProjection {
        output_chunks,
        completions,
        ..
    } = completion_event.1;
    assert!(
        output_chunks.is_empty(),
        "already projected output must not be duplicated"
    );
    assert_eq!(
        completions.len(),
        1,
        "authoritative settlement with current-turn output must synthesize a home completion"
    );
    assert_eq!(
        completions[0].home_prompt_id.as_deref(),
        Some(home_prompt_id)
    );
}

#[test]
fn native_completion_does_not_reuse_prior_home_prompt_identity() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "session-1",
            "agent-home-1",
            false,
            "user-home",
        )
        .expect("execution lease should be created");
    let leased_agent = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
            Some("sonnet".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("leased agent should be created");
    let home_prompt_id = "home-prompt-1";
    let (provider_run_id, outcome) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt_with_workflow_context(
            &leased_agent.id,
            "home-origin prompt\n",
            Vec::new(),
            None,
            Some(crate::transport::relay_peer::RemoteGitTurnContext {
                home_session_id: "session-1".to_string(),
                home_agent_id: "agent-home-1".to_string(),
                home_prompt_id: home_prompt_id.to_string(),
                home_turn_id: home_prompt_id.to_string(),
                source_attachment_id: None,
                workspace_live_sync_mode: None,
                prompt_origin: Some(PromptOrigin::Chariox),
                external_provider: None,
                external_provider_session_id: None,
                external_provider_turn_id: None,
                prompt_summary: "home-origin prompt".to_string(),
            }),
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
        .expect("home prompt should submit");
    assert!(matches!(outcome, PromptSubmissionOutcome::Started { .. }));
    app.complete_active_prompt(
        &leased_agent.backing_session_id,
        &leased_agent.backing_agent_id,
        Some(&provider_run_id),
    )
    .expect("home prompt should complete");

    let first = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
        .expect("home completion projection should succeed")
        .expect("home completion should be projected");
    let RelayPeerEvent::LeasedRuntimeProjection { completions, .. } = first.1;
    assert!(completions
        .iter()
        .all(|completion| completion.home_prompt_id.as_deref() == Some(home_prompt_id)));
    assert!(
        RemoteLeaseRuntime::new(&mut app)
            .leased_agent_snapshot_for_test(&leased_agent.id)
            .expect("leased agent should remain registered")
            .active_home_prompt_id
            .is_none(),
        "the completed home prompt identity must not leak into a later turn",
    );

    let native = app
        .record_native_prompt_started_with_attachments(
            &leased_agent.backing_session_id,
            &leased_agent.backing_attachment_id,
            "native-terminal-attachment",
            &leased_agent.backing_agent_id,
            "native-origin prompt",
            Vec::new(),
        )
        .expect("native prompt should be recorded");
    assert!(matches!(native, PromptSubmissionOutcome::Started { .. }));
    app.complete_active_prompt(
        &leased_agent.backing_session_id,
        &leased_agent.backing_agent_id,
        Some(&provider_run_id),
    )
    .expect("native prompt should complete");

    let second = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
        .expect("native completion projection should succeed")
        .expect("native prompt and completion should be projected");
    let RelayPeerEvent::LeasedRuntimeProjection {
        prompts,
        completions,
        ..
    } = second.1;
    assert_eq!(prompts.len(), 1);
    assert!(!completions.is_empty());
    assert!(
        completions
            .iter()
            .all(|completion| completion.home_prompt_id.is_none()),
        "native-origin completion must not carry a stale home prompt identity",
    );
}

#[test]
fn leased_projection_does_not_reflect_home_origin_prompt_back_to_home() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "session-1",
            "agent-home-1",
            false,
            "user-home",
        )
        .expect("execution lease should be created");
    let leased_agent = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
            Some("sonnet".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("leased agent should be created");

    let (provider_run_id, outcome) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt(&leased_agent.id, "remote leased prompt\n", Vec::new())
        .expect("leased prompt should submit");
    match outcome {
        PromptSubmissionOutcome::Started { .. } => {}
        other => panic!("unexpected prompt submission outcome: {other:?}"),
    }
    app.terminal_mut().fan_out_output(
        &leased_agent.backing_session_id,
        &provider_run_id,
        Some(&leased_agent.backing_agent_id),
        crate::terminal::TerminalOutputKind::PromptEcho,
        None,
        vec![leased_agent.backing_attachment_id.clone()],
        b"remote leased prompt\n",
    );
    app.terminal_mut().fan_out_output(
        &leased_agent.backing_session_id,
        &provider_run_id,
        Some(&leased_agent.backing_agent_id),
        crate::terminal::TerminalOutputKind::ProviderOutput,
        Some("assistant-output".to_string()),
        vec![leased_agent.backing_attachment_id.clone()],
        b"hello from worker",
    );

    let (_target_kernel_id, event) = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
        .expect("projection drain should succeed")
        .expect("output projection should be emitted");
    let RelayPeerEvent::LeasedRuntimeProjection {
        prompts,
        output_chunks,
        completions,
        ..
    } = event;
    assert!(
        prompts.is_empty(),
        "home-origin prompt must not be reflected"
    );
    assert_eq!(output_chunks.len(), 1);
    assert_eq!(
        output_chunks[0].kind,
        crate::terminal::TerminalOutputKind::ProviderOutput,
        "home-origin prompt echo must not be reflected as an output chunk"
    );
    assert_eq!(
        completions.len(),
        1,
        "current provider output should settle non-workflow leased prompts"
    );
}

#[test]
fn leased_projection_does_not_promote_passive_external_observation_to_prompt_or_output() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "session-1",
            "agent-home-1",
            false,
            "user-home",
        )
        .expect("execution lease should be created");
    let leased_agent = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
            Some("sonnet".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("leased agent should be created");
    let provider_run_id = "provider-run-1";
    for (kind, text, turn_id) in [
        (
            crate::history::SessionHistoryEntryKind::UserPrompt,
            "passive provider prompt",
            "turn-user",
        ),
        (
            crate::history::SessionHistoryEntryKind::ProviderOutput,
            "passive provider output",
            "turn-output",
        ),
    ] {
        app.append_history_entry(
            &leased_agent.backing_session_id,
            crate::history::SessionHistoryEntry::external_provider_observed(
                &leased_agent.backing_session_id,
                Some(provider_run_id),
                &leased_agent.backing_agent_id,
                kind,
                text,
                "codex",
                "thread-1",
                Some(turn_id.to_string()),
                None,
            ),
        );
    }

    let projection = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased_agent.id, provider_run_id, false)
        .expect("projection drain should succeed");
    if let Some((
        _target,
        RelayPeerEvent::LeasedRuntimeProjection {
            prompts,
            output_chunks,
            ..
        },
    )) = projection
    {
        assert!(prompts.is_empty());
        assert!(output_chunks.is_empty());
    }
}

#[test]
fn leased_projection_pump_forwards_completion_after_provider_run_ends() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "session-1",
            "agent-home-1",
            false,
            "user-home",
        )
        .expect("execution lease should be created");
    let leased_agent = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
            Some("sonnet".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("leased agent should be created");

    let (provider_run_id, outcome) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt(&leased_agent.id, "remote leased prompt\n", Vec::new())
        .expect("leased prompt should submit");
    match outcome {
        PromptSubmissionOutcome::Started { .. } => {}
        other => panic!("unexpected prompt submission outcome: {other:?}"),
    }
    app.complete_active_prompt(
        &leased_agent.backing_session_id,
        &leased_agent.backing_agent_id,
        Some(&provider_run_id),
    )
    .expect("backing prompt should settle first");
    let ended = app
        .providers_mut()
        .terminate_run_provider_only(&leased_agent.backing_session_id, &provider_run_id)
        .expect("provider run should end")
        .into_run();
    app.update_provider_run_projection(ended);

    let events = RemoteLeaseRuntime::new(&mut app)
        .pump_leased_runtime_projections()
        .expect("leased projection pump should run");

    assert_eq!(events.len(), 1);
    let (_target_kernel_id, event) = &events[0];
    let RelayPeerEvent::LeasedRuntimeProjection { completions, .. } = event;
    assert_eq!(completions.len(), 1);
}

#[test]
fn leased_projection_pump_leaves_home_prompt_records_for_authoritative_drain() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "session-home-drain",
            "agent-home-drain",
            false,
            "user-home",
        )
        .expect("execution lease should be created");
    let leased_agent = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
            Some("sonnet".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("leased agent should be created");
    let git_context = crate::transport::relay_peer::RemoteGitTurnContext {
        home_session_id: "session-home-drain".to_string(),
        home_agent_id: "agent-home-drain".to_string(),
        home_prompt_id: "home-prompt-drain".to_string(),
        home_turn_id: "home-prompt-drain".to_string(),
        source_attachment_id: None,
        workspace_live_sync_mode: None,
        prompt_origin: Some(PromptOrigin::Chariox),
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        prompt_summary: "home prompt drain".to_string(),
    };
    let (provider_run_id, outcome) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt_with_workflow_context(
            &leased_agent.id,
            "home prompt drain\n",
            Vec::new(),
            None,
            Some(git_context),
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
        .expect("leased prompt should submit");
    assert!(matches!(outcome, PromptSubmissionOutcome::Started { .. }));
    app.fan_out_output_for_agent(
        &leased_agent.backing_session_id,
        &provider_run_id,
        Some(&leased_agent.backing_agent_id),
        crate::terminal::TerminalOutputKind::ProviderOutput,
        Some("assistant-output".to_string()),
        vec![leased_agent.backing_attachment_id.clone()],
        b"remote assistant output",
    );
    app.complete_active_prompt(
        &leased_agent.backing_session_id,
        &leased_agent.backing_agent_id,
        Some(&provider_run_id),
    )
    .expect("backing prompt should settle");

    let events = RemoteLeaseRuntime::new(&mut app)
        .pump_leased_runtime_projections()
        .expect("best-effort projection pump should run");
    assert!(events.is_empty());

    let (_target_kernel_id, event) = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
        .expect("authoritative projection drain should succeed")
        .expect("authoritative projection drain should receive the settled turn");
    let RelayPeerEvent::LeasedRuntimeProjection {
        output_chunks,
        completions,
        ..
    } = event;
    assert_eq!(output_chunks.len(), 1);
    assert_eq!(output_chunks[0].bytes, b"remote assistant output");
    assert_eq!(completions.len(), 1);
}

#[test]
fn leased_projection_pump_settles_quiet_non_workflow_prompt() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "session-1",
            "agent-home-1",
            false,
            "user-home",
        )
        .expect("execution lease should be created");
    let leased_agent = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
            Some("sonnet".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("leased agent should be created");

    let (provider_run_id, outcome) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt(&leased_agent.id, "remote leased prompt\n", Vec::new())
        .expect("leased prompt should submit");
    match outcome {
        PromptSubmissionOutcome::Started { .. } => {}
        other => panic!("unexpected prompt submission outcome: {other:?}"),
    }
    {
        let mut prompt_activity = app.prompt_activity.write();
        let state = prompt_activity
            .get_mut(&provider_run_id)
            .expect("active leased turn should be tracked");
        state.saw_response_content = true;
        state.last_output_at = Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
    }

    let (_target_kernel_id, event) = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, true)
        .expect("projection drain should succeed")
        .expect("quiet prompt completion should be projected");
    let RelayPeerEvent::LeasedRuntimeProjection { completions, .. } = event;
    assert_eq!(completions.len(), 1);
    assert!(app
        .prompt_owner_active_prompt_for_agent_snapshot(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        )
        .expect("active prompt should load")
        .is_none());
}
