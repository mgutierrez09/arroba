use std::env;
use std::fs;
use std::thread;
use std::time::Duration;

use chariox_kernel::attachment::{AttachRequest, ClientCapabilityLevel};
use chariox_kernel::provider::{LaunchProviderRequest, ProviderRunState};
use chariox_kernel::session::{CreateSessionRequest, PromptStatus, PromptSubmissionOutcome};
use chariox_kernel::{DaemonApp, DaemonConfig};

mod support;
use support::runtime_integration::{
    collect_provider_records_until, collect_terminal_output_until, create_opencode_fixture_script,
    opencode_env_guard, render_terminal_output, MockOpenCodeServer,
};

#[test]
fn mock_opencode_without_fixture_credentials_cannot_launch() {
    let _guard = opencode_env_guard();
    let fixture_path = create_opencode_fixture_script();
    env::set_var("CHARIOX_OPENCODE_BIN", &fixture_path);
    let fixture_data = std::path::PathBuf::from(env::var_os("XDG_DATA_HOME").unwrap());
    fs::remove_file(fixture_data.join("opencode/auth.json"))
        .expect("remove only the isolated fake credential");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, agent) = app
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .unwrap();
    let result = app.launch_provider(
        LaunchProviderRequest::new(session.id(), "opencode", "opencode", "default", "default")
            .with_agent_id(agent.id()),
    );
    fs::remove_file(&fixture_path).expect("remove fixture executable");
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("is not authenticated"),
        "a successful mock auth command alone must not bypass credential validation"
    );
}

#[test]
fn focused_agent_prompts_route_to_distinct_opencode_runs_and_history() {
    let _guard = opencode_env_guard();
    let fixture_path = create_opencode_fixture_script();
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(50));
    let previous_bin = env::var_os("CHARIOX_OPENCODE_BIN");
    let previous_port = env::var_os("CHARIOX_OPENCODE_PORT");
    env::set_var("CHARIOX_OPENCODE_BIN", &fixture_path);
    env::set_var("CHARIOX_OPENCODE_PORT", mock_server.port().to_string());

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, default_agent) = app
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let default_run = app
        .launch_provider(
            LaunchProviderRequest::new(session.id(), "opencode", "opencode", "default", "default")
                .with_agent_id(default_agent.id()),
        )
        .expect("default provider run should launch");
    let reviewer = app
        .spawn_agent(
            chariox_kernel::agent::CreateAgentRequest::new(session.id(), "opencode")
                .with_alias("reviewer"),
        )
        .expect("reviewer agent should spawn");
    let reviewer_run = app
        .launch_provider(
            LaunchProviderRequest::new(session.id(), "opencode", "opencode", "default", "default")
                .with_agent_id(reviewer.id()),
        )
        .expect("reviewer provider run should launch");

    app.focus_agent(session.id(), default_agent.id())
        .expect("default agent should focus");
    let first_submission = chariox_kernel::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "default agent prompt\n",
        Vec::new(),
    )
    .expect("default prompt should start");
    match first_submission {
        PromptSubmissionOutcome::Started { prompt } => {
            assert_eq!(prompt.target_agent_id(), default_agent.id());
        }
        _ => panic!("expected default prompt to start immediately"),
    }

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let default_records = collect_provider_records_until(
        &mut app,
        session.id(),
        default_run.id(),
        recipients.clone(),
        |records, app| {
            let text = render_terminal_output(records);
            text.contains("fixture response: default agent prompt")
                && app
                    .sessions()
                    .get_session(session.id())
                    .expect("session should still exist")
                    .active_prompt()
                    .is_none()
        },
    );
    assert!(default_records
        .iter()
        .all(|record| record.agent_id.as_deref() == Some(default_agent.id())));

    app.focus_agent(session.id(), reviewer.id())
        .expect("reviewer agent should focus");
    let second_submission = chariox_kernel::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "review agent prompt\n",
        Vec::new(),
    )
    .expect("review prompt should start");
    match second_submission {
        PromptSubmissionOutcome::Started { prompt } => {
            assert_eq!(prompt.target_agent_id(), reviewer.id());
        }
        _ => panic!("expected review prompt to start immediately"),
    }

    let reviewer_records = collect_provider_records_until(
        &mut app,
        session.id(),
        reviewer_run.id(),
        recipients,
        |records, app| {
            let text = render_terminal_output(records);
            text.contains("fixture response: review agent prompt")
                && app
                    .sessions()
                    .get_session(session.id())
                    .expect("session should still exist")
                    .active_prompt()
                    .is_none()
        },
    );
    assert!(reviewer_records
        .iter()
        .all(|record| record.agent_id.as_deref() == Some(reviewer.id())));

    let default_history = app
        .load_session_history_entries(&session, Some(default_agent.id()))
        .expect("default history should load");
    let reviewer_history = app
        .load_session_history_entries(&session, Some(reviewer.id()))
        .expect("reviewer history should load");

    let default_text = default_history
        .iter()
        .map(|entry| entry.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let reviewer_text = reviewer_history
        .iter()
        .map(|entry| entry.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");

    assert!(default_text.contains("default agent prompt"));
    assert!(default_text.contains("fixture response: default agent prompt"));
    assert!(!default_text.contains("review agent prompt"));
    assert!(reviewer_text.contains("review agent prompt"));
    assert!(reviewer_text.contains("fixture response: review agent prompt"));
    assert!(!reviewer_text.contains("default agent prompt"));

    let session_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert_eq!(session_state.focused_agent_id(), Some(reviewer.id()));
    assert_eq!(
        session_state.active_provider_run_id(),
        Some(reviewer_run.id())
    );

    if let Some(previous_bin) = previous_bin {
        env::set_var("CHARIOX_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("CHARIOX_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("CHARIOX_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("CHARIOX_OPENCODE_PORT");
    }
    mock_server.stop();
    let _ = fs::remove_file(&fixture_path);
}

#[test]
fn focusing_another_agent_during_an_opencode_prompt_keeps_the_working_run_active() {
    let _guard = opencode_env_guard();
    let fixture_path = create_opencode_fixture_script();
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(150));
    let previous_bin = env::var_os("CHARIOX_OPENCODE_BIN");
    let previous_port = env::var_os("CHARIOX_OPENCODE_PORT");
    env::set_var("CHARIOX_OPENCODE_BIN", &fixture_path);
    env::set_var("CHARIOX_OPENCODE_PORT", mock_server.port().to_string());

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, default_agent) = app
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let default_run = app
        .launch_provider(
            LaunchProviderRequest::new(session.id(), "opencode", "opencode", "default", "default")
                .with_agent_id(default_agent.id()),
        )
        .expect("default provider run should launch");
    let reviewer = app
        .spawn_agent(
            chariox_kernel::agent::CreateAgentRequest::new(session.id(), "opencode")
                .with_alias("reviewer"),
        )
        .expect("reviewer agent should spawn");
    let reviewer_run = app
        .launch_provider(
            LaunchProviderRequest::new(session.id(), "opencode", "opencode", "default", "default")
                .with_agent_id(reviewer.id()),
        )
        .expect("reviewer provider run should launch");

    app.focus_agent(session.id(), default_agent.id())
        .expect("default agent should focus");
    let started = chariox_kernel::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "keep streaming while focus changes\n",
        Vec::new(),
    )
    .expect("prompt should start");
    match started {
        PromptSubmissionOutcome::Started { prompt } => {
            assert_eq!(prompt.target_agent_id(), default_agent.id());
        }
        _ => panic!("expected prompt to start immediately"),
    }

    app.focus_agent(session.id(), reviewer.id())
        .expect("reviewer agent should focus");

    let session_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert_eq!(session_state.focused_agent_id(), Some(reviewer.id()));
    assert_eq!(
        session_state.active_provider_run_id(),
        Some(default_run.id())
    );

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let default_records = collect_provider_records_until(
        &mut app,
        session.id(),
        default_run.id(),
        recipients,
        |records, app| {
            let text = render_terminal_output(records);
            text.contains("fixture response: keep streaming while focus changes")
                && app
                    .sessions()
                    .get_session(session.id())
                    .expect("session should still exist")
                    .active_prompt()
                    .is_none()
                && app
                    .sessions()
                    .get_session(session.id())
                    .expect("session should still exist")
                    .active_provider_run_id()
                    == Some(reviewer_run.id())
        },
    );

    assert!(default_records
        .iter()
        .any(|record| record.agent_id.as_deref() == Some(default_agent.id())));

    let settled_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert_eq!(settled_state.focused_agent_id(), Some(reviewer.id()));
    assert_eq!(
        settled_state.active_provider_run_id(),
        Some(reviewer_run.id())
    );

    if let Some(previous_bin) = previous_bin {
        env::set_var("CHARIOX_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("CHARIOX_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("CHARIOX_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("CHARIOX_OPENCODE_PORT");
    }
    mock_server.stop();
    let _ = fs::remove_file(&fixture_path);
}

#[test]
fn prompt_for_another_agent_starts_on_its_own_run_without_switching_focus_selection() {
    let _guard = opencode_env_guard();
    let fixture_path = create_opencode_fixture_script();
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(150));
    let previous_bin = env::var_os("CHARIOX_OPENCODE_BIN");
    let previous_port = env::var_os("CHARIOX_OPENCODE_PORT");
    env::set_var("CHARIOX_OPENCODE_BIN", &fixture_path);
    env::set_var("CHARIOX_OPENCODE_PORT", mock_server.port().to_string());

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, default_agent) = app
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let default_run = app
        .launch_provider(
            LaunchProviderRequest::new(session.id(), "opencode", "opencode", "default", "default")
                .with_agent_id(default_agent.id()),
        )
        .expect("default provider run should launch");
    let reviewer = app
        .spawn_agent(
            chariox_kernel::agent::CreateAgentRequest::new(session.id(), "opencode")
                .with_alias("reviewer"),
        )
        .expect("reviewer agent should spawn");
    let reviewer_run = app
        .launch_provider(
            LaunchProviderRequest::new(session.id(), "opencode", "opencode", "default", "default")
                .with_agent_id(reviewer.id()),
        )
        .expect("reviewer provider run should launch");

    app.focus_agent(session.id(), default_agent.id())
        .expect("default agent should focus");
    let first_submission = chariox_kernel::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "default agent prompt stays active\n",
        Vec::new(),
    )
    .expect("default prompt should start");
    match first_submission {
        PromptSubmissionOutcome::Started { prompt } => {
            assert_eq!(prompt.target_agent_id(), default_agent.id());
        }
        _ => panic!("expected default prompt to start immediately"),
    }

    app.focus_agent(session.id(), reviewer.id())
        .expect("reviewer agent should focus while default agent is running");
    let second_submission = chariox_kernel::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "reviewer prompt should queue\n",
        Vec::new(),
    )
    .expect("reviewer prompt should start");
    match second_submission {
        PromptSubmissionOutcome::Started { prompt } => {
            assert_eq!(prompt.target_agent_id(), reviewer.id());
        }
        _ => panic!("expected reviewer prompt to start immediately"),
    }

    let queued_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert_eq!(queued_state.focused_agent_id(), Some(reviewer.id()));
    assert_eq!(
        queued_state.active_provider_run_id(),
        Some(default_run.id())
    );
    assert_eq!(
        queued_state
            .active_prompt_for_agent(default_agent.id())
            .expect("default prompt should still be active")
            .target_agent_id(),
        default_agent.id()
    );
    assert_eq!(
        queued_state
            .active_prompt_for_agent(reviewer.id())
            .expect("reviewer prompt should also be active")
            .target_agent_id(),
        reviewer.id()
    );
    assert!(queued_state
        .queued_prompts_for_agent(reviewer.id())
        .is_none_or(|queue| queue.is_empty()));

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let default_records = collect_provider_records_until(
        &mut app,
        session.id(),
        default_run.id(),
        recipients.clone(),
        |records, app| {
            let text = render_terminal_output(records);
            text.contains("fixture response: default agent prompt stays active")
                && app
                    .sessions()
                    .get_session(session.id())
                    .expect("session should still exist")
                    .active_prompt_for_agent(default_agent.id())
                    .is_none()
        },
    );
    assert!(default_records
        .iter()
        .all(|record| record.agent_id.as_deref() == Some(default_agent.id())));

    let reviewer_records = collect_provider_records_until(
        &mut app,
        session.id(),
        reviewer_run.id(),
        recipients,
        |records, app| {
            let text = render_terminal_output(records);
            text.contains("fixture response: reviewer prompt should queue")
                && app
                    .sessions()
                    .get_session(session.id())
                    .expect("session should still exist")
                    .active_prompt_for_agent(reviewer.id())
                    .is_none()
        },
    );
    assert!(reviewer_records
        .iter()
        .all(|record| record.agent_id.as_deref() == Some(reviewer.id())));

    let settled_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert_eq!(settled_state.focused_agent_id(), Some(reviewer.id()));
    assert_eq!(
        settled_state.active_provider_run_id(),
        Some(reviewer_run.id())
    );
    assert!(settled_state
        .queued_prompts_for_agent(default_agent.id())
        .is_none_or(|queue| queue.is_empty()));
    assert!(settled_state
        .queued_prompts_for_agent(reviewer.id())
        .is_none_or(|queue| queue.is_empty()));

    if let Some(previous_bin) = previous_bin {
        env::set_var("CHARIOX_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("CHARIOX_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("CHARIOX_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("CHARIOX_OPENCODE_PORT");
    }
    mock_server.stop();
    let _ = fs::remove_file(&fixture_path);
}

#[test]
fn detaching_the_last_attachment_keeps_an_active_turn_available_on_rejoin() {
    let _guard = opencode_env_guard();
    let fixture_path = create_opencode_fixture_script();
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(150));
    let previous_bin = env::var_os("CHARIOX_OPENCODE_BIN");
    let previous_port = env::var_os("CHARIOX_OPENCODE_PORT");
    env::set_var("CHARIOX_OPENCODE_BIN", &fixture_path);
    env::set_var("CHARIOX_OPENCODE_PORT", mock_server.port().to_string());

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let first = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "opencode",
            "opencode",
            "default",
            "default",
        ))
        .expect("provider run should launch");

    let _ = chariox_kernel::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        first.id(),
        "prompt survives detach\n",
        Vec::new(),
    )
    .expect("prompt should start");

    app.detach(first.id())
        .expect("detaching the only attachment should succeed");

    let detached_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert!(detached_state.attachment_ids().is_empty());
    assert_eq!(
        detached_state.active_prompt().map(|prompt| prompt.status()),
        Some(PromptStatus::Running)
    );
    assert_eq!(
        app.providers()
            .get_run(run.id())
            .expect("provider run should remain queryable")
            .state(),
        ProviderRunState::Running
    );

    thread::sleep(Duration::from_millis(75));

    let second = app
        .attach(AttachRequest::new(
            session.id(),
            "client-b",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("reattach should succeed");

    let output =
        collect_terminal_output_until(&mut app, session.id(), second.id(), |output, session| {
            output.contains("fixture response: prompt survives detach")
                && session.active_prompt().is_none()
        });

    assert!(output.contains("fixture response: prompt survives detach"));

    if let Some(previous_bin) = previous_bin {
        env::set_var("CHARIOX_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("CHARIOX_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("CHARIOX_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("CHARIOX_OPENCODE_PORT");
    }
    mock_server.stop();
    let _ = fs::remove_file(&fixture_path);
}
