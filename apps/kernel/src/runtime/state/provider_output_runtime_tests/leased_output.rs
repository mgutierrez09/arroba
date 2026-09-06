use super::*;
use crate::transport::relay_peer::RelayPeerEvent;
use std::os::unix::fs::PermissionsExt;
use wait_timeout::ChildExt;

const CHILD_ROOT: &str = "CHARIOX_LEASED_OUTPUT_START_TEST_ROOT";
const TEST_NAME: &str = "runtime::state::provider_output_runtime_tests::leased_output::leased_claude_failure_reaches_home_projection_without_terminal_polling";

struct FixtureCleanup {
    root: std::path::PathBuf,
    providers: Option<crate::provider::ProviderProcessServiceStore>,
}

impl Drop for FixtureCleanup {
    fn drop(&mut self) {
        if let Some(providers) = self.providers.as_mut() {
            for run in providers.list_runs() {
                providers.clear_runtime(run.id());
            }
        }
        std::fs::remove_dir_all(&self.root).expect("remove isolated leased-output fixture");
    }
}

#[tokio::test]
async fn leased_claude_failure_reaches_home_projection_without_terminal_polling() {
    // Executable/profile overrides live only in the child test process. Never
    // change the parent test runner's provider environment or native accounts.
    let Some(fixture_root) = std::env::var_os(CHILD_ROOT) else {
        run_isolated_launch_fixture();
        return;
    };
    let fixture_root = std::path::PathBuf::from(fixture_root);
    let root = fixture_root.join("runtime");
    std::fs::create_dir_all(&root).unwrap();
    let mut cleanup = FixtureCleanup {
        root: root.clone(),
        providers: None,
    };
    let mut config = crate::DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    config.provider_runtime_init_delay_ms = 100;
    config.local_socket_path = root.join("kernel.sock");
    config = config.with_session_history_root(root.join("history"));
    config.user_config.state.path = Some(root.join("state.db").display().to_string());
    config.user_config.history.operational.path =
        Some(root.join("events.db").display().to_string());
    config.user_config.artifacts.operational.root =
        Some(root.join("artifacts").display().to_string());
    config.user_config.artifacts.operational.index_path =
        Some(root.join("artifacts.db").display().to_string());
    let app = DaemonApp::bootstrap(config).expect("worker bootstrap");
    cleanup.providers = Some(app.providers().clone());
    let account = app
        .provider_account_profile_registry()
        .create_managed("owner", "claude", "fixture")
        .expect("isolated worker account profile");
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let lease = runtime
        .create_relay_execution_lease("home", "room", "agent", false, "owner")
        .await
        .expect("execution lease");
    let leased = runtime
        .create_relay_leased_agent(
            &lease.id,
            "claude",
            &account.profile_id,
            Some("sonnet".to_string()),
            None,
            None,
            None,
            None,
            Some(root.display().to_string()),
            None,
        )
        .await
        .expect("leased Claude agent");

    // Let the real leased submission launch and initialize Claude after the
    // runtime starts. No preinserted run, binding or active-session pointer.
    let received = fixture_root.join("received");
    let (run_id, _accepted) = runtime
        .submit_relay_leased_prompt(
            &leased.id,
            "inspect the Room",
            "",
            Vec::new(),
            None,
            Some(crate::transport::relay_peer::RemoteGitTurnContext {
                home_session_id: "room".to_string(),
                home_agent_id: "agent".to_string(),
                home_prompt_id: "home-prompt".to_string(),
                home_turn_id: "home-prompt".to_string(),
                source_attachment_id: None,
                workspace_live_sync_mode: None,
                prompt_origin: Some(crate::session::PromptOrigin::Chariox),
                external_provider: None,
                external_provider_session_id: None,
                external_provider_turn_id: None,
                prompt_summary: "inspect the Room".to_string(),
            }),
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
            None,
        )
        .await
        .expect("leased prompt accepted");
    // Cold launches may accept into the queue until their binding is ready.
    // The required outcome is eventual native delivery, projection and settlement.

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut projected_errors = Vec::new();
    loop {
        runtime.pump_transport_runtime().await;
        // Same drain request as the home kernel's active-prompt loop. No local
        // terminal client is attached to wake the provider-output pump.
        if let Some((_, event)) = runtime
            .drain_relay_leased_runtime_projection(&leased.id, &run_id, true, true)
            .await
            .unwrap()
        {
            let RelayPeerEvent::LeasedRuntimeProjection { output_chunks, .. } = event;
            projected_errors.extend(
                output_chunks
                    .into_iter()
                    .filter(|chunk| {
                        chunk.kind == crate::terminal::TerminalOutputKind::ProviderError
                    })
                    .map(|chunk| String::from_utf8_lossy(&chunk.bytes).into_owned()),
            );
        }
        if !projected_errors.is_empty() || tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        received.is_file(),
        "native provider must have received the prompt and emitted its error; projected errors: {projected_errors:?}"
    );
    assert!(projected_errors.iter().any(|message| message.contains("Fixture Claude login required")),
        "accepted leased provider error must reach home projection without a terminal-output request: {projected_errors:?}");
    let session = runtime
        .owned
        .session_store
        .get_session(&leased.backing_session_id)
        .unwrap();
    assert!(
        session
            .active_prompt_for_agent(&leased.backing_agent_id)
            .is_none(),
        "failed turn must settle"
    );
}

fn run_isolated_launch_fixture() {
    let root = std::env::temp_dir().join(format!(
        "chariox-leased-output-start-{}-{}",
        std::process::id(),
        rand::random::<u64>(),
    ));
    std::fs::create_dir_all(&root).unwrap();
    let _cleanup = FixtureCleanup {
        root: root.clone(),
        providers: None,
    };
    let executable = root.join("claude");
    std::fs::write(&executable, r#"#!/bin/bash
if [[ "$1" == "--version" ]]; then printf 'Claude Code 2.1.207\n'; exit 0; fi
if IFS= read -r -t 10 line; then
    : > "$CHARIOX_TEST_RECEIVED"
    printf '%s\n' '{"type":"result","subtype":"error_during_execution","is_error":true,"error":"Fixture Claude login required"}'
    IFS= read -r -t 10 ignored || true
fi
"#).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([TEST_NAME, "--exact", "--test-threads=1", "--nocapture"])
        .current_dir(&root)
        .env(CHILD_ROOT, &root)
        .env("HOME", &root)
        .env("CLAUDE_CONFIG_DIR", root.join("claude-config"))
        .env("CHARIOX_HOME", root.join("home"))
        .env("CHARIOX_LOG_DIR", root.join("logs"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_STATE_HOME", root.join("state"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("CHARIOX_CLAUDE_BIN", executable)
        .env("CHARIOX_TEST_RECEIVED", root.join("received"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    if child
        .wait_timeout(std::time::Duration::from_secs(25))
        .unwrap()
        .is_none()
    {
        let _ = child.kill();
        let _ = child.wait();
        panic!("isolated leased launch fixture timed out");
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "isolated leased launch failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
