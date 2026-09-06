use super::super::*;
use crate::agent::CreateAgentRequest;
use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::provider::{
    LaunchProviderRequest, ProviderClientInterface, ProviderResumeState, ProviderRunState,
};
use crate::session::{
    CreateSessionRequest, PromptQueueItem, PromptStatus, PromptSubmissionOutcome,
    SessionAgentDefaults,
};

#[test]
fn prompt_auto_launch_uses_agent_owner_and_resume_state() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(
            CreateSessionRequest::new("workspace-1", "worktree-1")
                .with_owner_user_id("cloud-user")
                .with_agent_defaults(
                    SessionAgentDefaults::new("dev-stub")
                        .with_model("sonnet")
                        .with_account_profile("profile-a"),
                ),
        )
        .expect("session create should succeed");
    assert_eq!(agent.account_profile(), Some("profile-a"));
    let resume_state = ProviderResumeState::from_codex_thread_id("codex-thread-1");
    app.agents
        .set_agent_runtime_profile(
            agent.id(),
            "dev-stub",
            Some("sonnet".to_string()),
            None,
            resume_state.clone(),
        )
        .expect("agent resume state should update");

    let run_id = app
        .ensure_prompt_provider_run_for_agent(session.id(), agent.id())
        .expect("prompt auto-launch should create a provider run");
    let run = app
        .providers()
        .get_run(&run_id)
        .expect("provider run should exist");

    assert_eq!(run.owner_user_id(), "cloud-user");
    assert_eq!(run.account_profile(), "profile-a");
    assert_eq!(run.resume_state(), &resume_state);
}

#[test]
fn prompt_auto_launch_failure_does_not_leave_running_provider_run() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(
            CreateSessionRequest::new("workspace-1", "worktree-1")
                .with_agent_defaults(SessionAgentDefaults::new("dev-stub").with_model("sonnet")),
        )
        .expect("session create should succeed");

    let result = app.launch_provider_detached(
        LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "runtime-init-fail",
            "default",
            "sonnet",
        )
        .with_agent_id(agent.id().to_string()),
    );

    assert!(result.is_err());
    let run = app
        .providers()
        .get_latest_run_for_agent(session.id(), agent.id())
        .expect("failed launch should still leave an ended run record");
    assert_eq!(run.state(), ProviderRunState::Ended);
    assert!(app
        .sessions()
        .get_session(session.id())
        .expect("session should resolve")
        .active_provider_run_id()
        .is_none());
    assert!(app
        .list_provider_processes(None)
        .expect("provider processes should list")
        .is_empty());
}

#[test]
fn detached_provider_launch_profile_persistence_failure_cleans_up_runtime() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(
            CreateSessionRequest::new("workspace-profile-failure", "worktree-profile-failure")
                .with_agent_defaults(
                    SessionAgentDefaults::new("dev-stub").with_model("native-tui-idle"),
                ),
        )
        .expect("session create should succeed");
    let agent_before_launch = app
        .agents
        .get_agent(agent.id())
        .expect("agent should exist before launch");
    let connection = rusqlite::Connection::open(app.durable_state_store().path())
        .expect("durable database should open for failure injection");
    connection
        .execute_batch(
            "CREATE TRIGGER fail_detached_launch_profile_append
             BEFORE INSERT ON durable_state_events
             WHEN NEW.kind = 'agent.runtime_profile_updated'
             BEGIN
               SELECT RAISE(FAIL, 'injected detached launch profile persistence failure');
             END;",
        )
        .expect("profile failure trigger should install");

    let error = app
        .launch_provider_detached(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "native-tui-idle",
            )
            .with_agent_id(agent.id()),
        )
        .expect_err("durable profile failure should fail detached launch");

    connection
        .execute_batch("DROP TRIGGER fail_detached_launch_profile_append;")
        .expect("profile failure trigger should be removed");
    assert!(error
        .to_string()
        .contains("injected detached launch profile persistence failure"));
    let run = app
        .providers()
        .get_latest_run_for_agent(session.id(), agent.id())
        .expect("failed launch should retain an ended run record");
    assert_eq!(run.state(), ProviderRunState::Ended);
    assert_eq!(
        app.sessions()
            .get_session(session.id())
            .expect("session should resolve")
            .active_provider_run_id(),
        None,
    );
    assert_eq!(
        app.agents
            .get_agent(agent.id())
            .expect("agent should remain available"),
        agent_before_launch,
        "the failed durable profile write must restore the prior agent state",
    );
    assert!(app
        .list_provider_processes(None)
        .expect("provider processes should list")
        .is_empty());
    let tracking = app.provider_process_tracking.snapshot();
    assert!(tracking.processes.is_empty());
    assert!(tracking.run_processes.is_empty());
}

#[tokio::test]
async fn provider_launch_failure_retries_durable_resume_invalidation_automatically() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-resume-clear-failure",
            "worktree-resume-clear-failure",
        ))
        .expect("session create should succeed");
    let stale_resume = ProviderResumeState::from_codex_thread_id("stale-thread");
    app.agents
        .set_agent_runtime_profile_durably(
            &app.durable_state,
            agent.id(),
            "codex",
            Some("gpt-5.5".to_string()),
            Some("default".to_string()),
            Some("default".to_string()),
            stale_resume.clone(),
            None,
            Some("test_resume_seeded"),
        )
        .expect("stale resume should persist");
    let request = LaunchProviderRequest::new(session.id(), "codex", "codex", "default", "gpt-5.5")
        .with_agent_id(agent.id())
        .with_resume_state(stale_resume);
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-resume-clear-failure",
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
        .expect("provider run should become active");
    let started = StartedProviderLaunch {
        run: run.clone(),
        previous_active_run_id: None,
        provider_credential_env: Default::default(),
    };
    let resume_error = DaemonError::ProviderProtocol {
        provider_run_id: run.id().to_string(),
        operation: "thread/resume",
        message: "no rollout found for thread".to_string(),
    };
    let connection = rusqlite::Connection::open(app.durable_state_store().path())
        .expect("durable database should open for failure injection");
    connection
        .execute_batch(
            "CREATE TRIGGER fail_launch_resume_clear_append
             BEFORE INSERT ON durable_state_events
             WHEN NEW.kind = 'agent.runtime_profile_updated'
             BEGIN
               SELECT RAISE(FAIL, 'injected launch resume clear persistence failure');
             END;",
        )
        .expect("resume clear failure trigger should install");

    app.fail_provider_launch(&started, &resume_error);

    assert_eq!(
        app.providers()
            .get_run(run.id())
            .expect("provider run should remain available")
            .state(),
        ProviderRunState::Running,
        "the provider must remain recoverable until invalidation is durable",
    );
    assert_eq!(
        app.sessions()
            .get_session(session.id())
            .expect("session should remain available")
            .active_provider_run_id(),
        Some(run.id()),
    );
    assert_eq!(
        app.agents
            .get_agent(agent.id())
            .expect("agent should remain available")
            .provider_resume_state()
            .codex_thread_id(),
        Some("stale-thread"),
    );

    connection
        .execute_batch("DROP TRIGGER fail_launch_resume_clear_append;")
        .expect("resume clear failure trigger should be removed");
    let app = std::sync::Arc::new(tokio::sync::Mutex::new(app));
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
        std::sync::Arc::clone(&app),
        1,
    );
    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    router.pump_transport_runtime().await;
    let app = app.lock().await;

    assert_eq!(
        app.providers()
            .get_run(run.id())
            .expect("provider run should remain addressable")
            .state(),
        ProviderRunState::Ended,
    );
    assert_eq!(
        app.sessions()
            .get_session(session.id())
            .expect("session should remain available")
            .active_provider_run_id(),
        None,
    );
    assert_eq!(
        app.agents
            .get_agent(agent.id())
            .expect("agent should remain available")
            .provider_resume_state()
            .codex_thread_id(),
        None,
    );
}

#[test]
fn provider_launch_failure_cleans_failed_run_after_resume_was_superseded() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-superseded-resume",
            "worktree-superseded-resume",
        ))
        .expect("session create should succeed");
    let stale_resume = ProviderResumeState::from_codex_thread_id("stale-thread");
    let request = LaunchProviderRequest::new(session.id(), "codex", "codex", "default", "gpt-5.5")
        .with_agent_id(agent.id())
        .with_resume_state(stale_resume);
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-superseded-resume",
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
        .expect("provider run should become active");
    app.agents
        .set_agent_runtime_profile_durably(
            &app.durable_state,
            agent.id(),
            "codex",
            Some("gpt-5.5".to_string()),
            Some("default".to_string()),
            Some("default".to_string()),
            ProviderResumeState::from_codex_thread_id("newer-thread"),
            Some("provider-run-newer-resume"),
            Some("test_resume_superseded"),
        )
        .expect("newer resume should persist");
    let started = StartedProviderLaunch {
        run: run.clone(),
        previous_active_run_id: None,
        provider_credential_env: Default::default(),
    };
    let resume_error = DaemonError::ProviderProtocol {
        provider_run_id: run.id().to_string(),
        operation: "thread/resume",
        message: "no rollout found for thread".to_string(),
    };

    app.fail_provider_launch(&started, &resume_error);

    assert_eq!(
        app.providers()
            .get_run(run.id())
            .expect("failed provider run should remain addressable")
            .state(),
        ProviderRunState::Ended,
    );
    assert_eq!(
        app.sessions()
            .get_session(session.id())
            .expect("session should remain available")
            .active_provider_run_id(),
        None,
    );
    assert_eq!(
        app.agents
            .get_agent(agent.id())
            .expect("agent should remain available")
            .provider_resume_state()
            .codex_thread_id(),
        Some("newer-thread"),
    );
}

#[test]
fn provider_launch_failure_preserves_durable_active_prompt_for_retry() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(
            CreateSessionRequest::new("workspace-retry", "worktree-retry")
                .with_agent_defaults(SessionAgentDefaults::new("dev-stub").with_model("sonnet")),
        )
        .expect("session create should succeed");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-retry",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let outcome = app
        .prompt_owner_submit_prepared_prompt(
            session.id(),
            PromptQueueItem::new(
                "pending-retry",
                attachment.id(),
                agent.id(),
                "retry after provider recovery",
                PromptStatus::Queued,
            )
            .with_durable_operation("command-retry", "fingerprint-retry"),
            false,
        )
        .expect("prompt should be accepted");
    let prompt_id = match outcome {
        PromptSubmissionOutcome::Started { prompt } => prompt.id().to_string(),
        PromptSubmissionOutcome::Queued { .. } => panic!("prompt should start"),
    };

    let result = app.launch_provider_detached(
        LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "runtime-init-fail",
            "default",
            "sonnet",
        )
        .with_agent_id(agent.id().to_string()),
    );

    assert!(result.is_err());
    let active = app
        .prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
        .expect("prompt state should load")
        .expect("durable prompt should remain active");
    assert_eq!(active.id(), prompt_id);
    assert_eq!(
        active.durable_delivery_phase(),
        Some(crate::session::DurablePromptDeliveryPhase::Accepted)
    );
}

#[test]
fn provider_activation_accepts_and_restores_projected_remote_predecessor() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, remote_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-remote-predecessor",
            "worktree-remote-predecessor",
        ))
        .expect("session should be created");
    let local_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("local-popup"))
        .expect("local agent should spawn");
    let projected_run_id = "leased:leased-agent-1:worker-run-1";
    let mut projected_run = crate::provider::RuntimeProviderRun::from_control_capability_inference(
        "worker-run-1",
        "worker-session-1".to_string(),
        Some("leased-agent-1".to_string()),
        "dev-stub".to_string(),
    );
    projected_run.mark_running();
    let projected_run = projected_run.projected_for_home_agent_with_id(
        projected_run_id,
        session.id(),
        remote_agent.id(),
    );
    app.update_provider_run_projection(projected_run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(projected_run_id.to_string()))
        .expect("projected remote run should be active");

    let started = crate::app::provider_activation::ProviderRunActivationState::start_provider_run_for_session(
        &mut app,
        LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "dev-stub",
            "default",
            "popup-model",
        )
        .with_agent_id(local_agent.id()),
    )
    .expect("local provider should activate beside a projected remote run");

    assert_eq!(
        started.previous_active_run_id.as_deref(),
        Some(projected_run_id)
    );
    let failed = app
        .providers
        .terminate_run_provider_only(session.id(), started.run.id())
        .expect("simulated failed launch should terminate");
    super::super::super::provider_liveness::clear_active_provider_run_session_pointer(
        &mut app,
        session.id(),
        failed.run().id(),
    )
    .expect("failed local launch should clear its pointer");
    let restored = crate::app::provider_activation::ProviderRunActivationState::resume_provider_run_for_session(
        &mut app,
        session.id(),
        projected_run_id,
    )
    .expect("rollback should restore the projected remote predecessor");

    assert_eq!(restored, projected_run);
    assert_eq!(
        app.sessions
            .get_session(session.id())
            .expect("session should remain available")
            .active_provider_run_id(),
        Some(projected_run_id)
    );
}

#[test]
fn provider_launch_rejects_agent_from_another_session() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (first_session, first_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("first session create should succeed");
    let (second_session, _second_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-2", "worktree-2"))
        .expect("second session create should succeed");

    let error = app
        .launch_provider(
            LaunchProviderRequest::new(
                second_session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(first_agent.id()),
        )
        .expect_err("launch should reject an agent outside the requested session");

    assert!(matches!(
        error,
        DaemonError::AgentNotInSession {
            session_id,
            agent_id,
        } if session_id == second_session.id() && agent_id == first_agent.id()
    ));
    assert!(app
        .providers()
        .get_latest_run_for_agent(first_session.id(), first_agent.id())
        .is_none());
    assert!(app
        .providers()
        .get_latest_run_for_agent(second_session.id(), first_agent.id())
        .is_none());
}

#[test]
fn detached_provider_launch_rejects_agent_from_another_session() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (_first_session, first_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("first session create should succeed");
    let (second_session, _second_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-2", "worktree-2"))
        .expect("second session create should succeed");

    let error = app
        .launch_provider_detached(
            LaunchProviderRequest::new(
                second_session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(first_agent.id()),
        )
        .expect_err("detached launch should reject an agent outside the requested session");

    assert!(matches!(
        error,
        DaemonError::AgentNotInSession {
            session_id,
            agent_id,
        } if session_id == second_session.id() && agent_id == first_agent.id()
    ));
    assert!(app.providers().list_runs().is_empty());
}

#[test]
fn provider_launch_replaces_existing_chariox_run_for_target_agent() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, first_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session create should succeed");
    let second_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("agent-b")
                .with_worktree("worktree-1"),
        )
        .expect("second agent should spawn");

    let first_run = app
        .launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(first_agent.id()),
        )
        .expect("first agent provider run should launch");
    let second_run = app
        .launch_provider(
            LaunchProviderRequest::new(session.id(), "dev-stub", "claude-code", "default", "opus")
                .with_agent_id(second_agent.id()),
        )
        .expect("second agent provider run should launch");
    assert_eq!(
        app.providers()
            .get_run(first_run.id())
            .expect("first run should still exist")
            .state(),
        ProviderRunState::Parked,
        "launching a different agent may park the previous session-active run"
    );
    assert_eq!(
        app.sessions()
            .get_session(session.id())
            .expect("session should resolve")
            .active_provider_run_id(),
        Some(second_run.id())
    );

    let replacement = app
        .launch_provider(
            LaunchProviderRequest::new(session.id(), "dev-stub", "claude-code", "default", "haiku")
                .with_agent_id(first_agent.id()),
        )
        .expect("replacement provider run should launch");

    assert_eq!(
        app.providers()
            .get_run(first_run.id())
            .expect("first run should remain addressable")
            .state(),
        ProviderRunState::Ended,
        "normal Chariox relaunch should not leave a second non-ended run for the same agent"
    );
    assert_eq!(replacement.state(), ProviderRunState::Running);
    assert_eq!(replacement.agent_instance_id(), Some(first_agent.id()));
    let non_ended_first_agent_runs = app
        .providers()
        .list_runs()
        .into_iter()
        .filter(|run| {
            run.session_id() == session.id()
                && run.agent_instance_id() == Some(first_agent.id())
                && run.state() != ProviderRunState::Ended
        })
        .collect::<Vec<_>>();
    assert_eq!(
        non_ended_first_agent_runs
            .iter()
            .map(|run| run.id())
            .collect::<Vec<_>>(),
        vec![replacement.id()]
    );
}

#[test]
fn provider_launch_rejects_replacing_target_agent_with_active_prompt() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session create should succeed");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("client should attach");

    let active_run = app
        .launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider run should launch");
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(agent.id()),
        "in-flight prompt\n",
        Vec::new(),
    )
    .expect("prompt should start");

    let error = app
        .launch_provider(
            LaunchProviderRequest::new(session.id(), "dev-stub", "claude-code", "default", "haiku")
                .with_agent_id(agent.id()),
        )
        .expect_err("same-agent relaunch should reject while a prompt is active");

    assert!(matches!(
        error,
        DaemonError::InvalidProviderRunState {
            provider_run_id,
            state: ProviderRunState::Running,
            operation: "replace agent provider run",
        } if provider_run_id == active_run.id()
    ));
    assert_eq!(
        app.providers()
            .get_run(active_run.id())
            .expect("active run should remain addressable")
            .state(),
        ProviderRunState::Running
    );
}

#[test]
fn native_tui_provider_launch_reuses_compatible_starting_run() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session create should succeed");

    let request =
        LaunchProviderRequest::new(session.id(), "dev-stub", "dev-stub", "default", "sonnet")
            .with_agent_id(agent.id())
            .with_client_interface(ProviderClientInterface::NativeTui);
    let started = app
        .start_provider_launch(request.clone())
        .expect("native launch should start");

    let reused = app
        .launch_provider(request)
        .expect("compatible native launch should reuse");

    assert_eq!(reused.id(), started.run.id());
    assert_eq!(
        app.providers()
            .list_runs()
            .into_iter()
            .filter(|run| {
                run.session_id() == session.id()
                    && run.agent_instance_id() == Some(agent.id())
                    && run.client_interface() == ProviderClientInterface::NativeTui
                    && run.state() != ProviderRunState::Ended
            })
            .count(),
        1
    );
}

#[test]
fn workflow_provider_launch_replaces_ordinary_run_before_tool_discovery() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session create should succeed");

    let ordinary = app
        .start_provider_launch(
            LaunchProviderRequest::new(session.id(), "dev-stub", "dev-stub", "default", "sonnet")
                .with_agent_id(agent.id()),
        )
        .expect("ordinary provider should start");
    assert!(!ordinary.run.workflow_tools_enabled());

    let workflow = app
        .start_workflow_provider_launch(
            LaunchProviderRequest::new(session.id(), "dev-stub", "dev-stub", "default", "sonnet")
                .with_agent_id(agent.id()),
        )
        .expect("workflow provider should replace ordinary run");

    assert_ne!(workflow.id(), ordinary.run.id());
    assert!(workflow.workflow_tools_enabled());
    assert!(matches!(
        app.providers()
            .get_run(ordinary.run.id())
            .expect("ordinary run should remain addressable")
            .state(),
        ProviderRunState::Parked | ProviderRunState::Ended
    ));
}

#[test]
fn native_tui_provider_launch_rejects_parameter_mismatch_without_duplicate() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session create should succeed");

    let request =
        LaunchProviderRequest::new(session.id(), "dev-stub", "dev-stub", "default", "sonnet")
            .with_agent_id(agent.id())
            .with_client_interface(ProviderClientInterface::NativeTui);
    let started = app
        .start_provider_launch(request)
        .expect("native launch should start");

    let error = app
        .launch_provider(
            LaunchProviderRequest::new(session.id(), "dev-stub", "dev-stub", "default", "haiku")
                .with_agent_id(agent.id())
                .with_client_interface(ProviderClientInterface::NativeTui),
        )
        .expect_err("mismatched native launch should reject");

    assert!(matches!(
        error,
        DaemonError::InvalidProviderRunState {
            provider_run_id,
            operation: "launch native TUI provider run with different parameters",
            ..
        } if provider_run_id == started.run.id()
    ));
    assert_eq!(
        app.providers()
            .list_runs()
            .into_iter()
            .filter(|run| {
                run.session_id() == session.id()
                    && run.agent_instance_id() == Some(agent.id())
                    && run.client_interface() == ProviderClientInterface::NativeTui
                    && run.state() != ProviderRunState::Ended
            })
            .count(),
        1
    );
}

#[test]
fn prompt_submission_while_native_launch_is_starting_does_not_duplicate_run() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session create should succeed");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("client should attach");

    let started = app
        .start_provider_launch(
            LaunchProviderRequest::new(session.id(), "dev-stub", "dev-stub", "default", "sonnet")
                .with_agent_id(agent.id())
                .with_client_interface(ProviderClientInterface::NativeTui),
        )
        .expect("native launch should start");

    let outcome = app
        .submit_prompt(
            session.id(),
            attachment.id(),
            Some(agent.id()),
            "queued while starting\n",
            Vec::new(),
        )
        .expect("prompt submission should queue");

    assert!(matches!(outcome, PromptSubmissionOutcome::Queued { .. }));
    assert_eq!(
        app.providers()
            .get_run(started.run.id())
            .expect("started run should remain")
            .state(),
        ProviderRunState::Starting
    );
    assert_eq!(
        app.providers()
            .list_runs()
            .into_iter()
            .filter(|run| {
                run.session_id() == session.id()
                    && run.agent_instance_id() == Some(agent.id())
                    && run.state() != ProviderRunState::Ended
            })
            .count(),
        1
    );
}
