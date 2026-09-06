use super::*;
use crate::app::RemoteLeaseRuntime;
use crate::transport::relay_peer::{RelayPeerEvent, RemoteGitTurnContext};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn leased_worker_failure_settles_and_retains_completion_while_home_is_offline() {
    assert_offline_worker_failure_settlement(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn leased_worker_launch_failure_settles_without_contacting_home() {
    assert_offline_worker_failure_settlement(true).await;
}

async fn assert_offline_worker_failure_settlement(launch_failure: bool) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::AsyncWriteExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = std::sync::Arc::new(AtomicUsize::new(0));
    let observed = requests.clone();
    let relay_probe = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            observed.fetch_add(1, Ordering::SeqCst);
            let _ = stream
                .write_all(
                    b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await;
        }
    });
    let mut config = crate::config::DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    config.relay_url = Some(format!("ws://{address}"));
    config.relay_token = Some("offline-relay".to_string());
    let (app, runtime, _, _) = agent_config_runtime_with_config(config).await;
    let diagnostic = "You've hit your session limit";
    let (leased_agent, provider_run_id) = {
        let mut app = app.lock().await;
        let lease = RemoteLeaseRuntime::new(&mut app)
            .create_execution_lease(
                "offline-home",
                "home-room",
                "home-agent",
                false,
                crate::session::DEFAULT_LOCAL_USER_ID,
            )
            .unwrap();
        let leased = RemoteLeaseRuntime::new(&mut app)
            .create_leased_agent(
                &lease.id,
                "managed-dev-stub",
                "default",
                Some("default".into()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let context = crate::execution_lease::RemoteWorkflowTurnContext {
            home_kernel_id: "offline-home".into(),
            home_session_id: "home-room".into(),
            home_agent_id: "home-agent".into(),
            workflow_run_id: "home-run".into(),
            workflow_node_run_id: "home-node-run".into(),
            delivery_token: "home-turn-token".into(),
            event_reply_enabled: false,
            event_context_enabled: false,
            event_actions_enabled: false,
        };
        let git_context = RemoteGitTurnContext {
            home_session_id: "home-room".into(),
            home_agent_id: "home-agent".into(),
            home_prompt_id: "home-prompt-1".into(),
            home_turn_id: "home-prompt-1".into(),
            source_attachment_id: None,
            workspace_live_sync_mode: None,
            prompt_origin: Some(crate::session::PromptOrigin::Chariox),
            external_provider: None,
            external_provider_session_id: None,
            external_provider_turn_id: None,
            prompt_summary: "failed review".into(),
        };
        let (run_id, outcome) = RemoteLeaseRuntime::new(&mut app)
            .submit_leased_prompt_with_workflow_context(
                &leased.id,
                "failed review",
                Vec::new(),
                Some(context),
                Some(git_context),
                Vec::new(),
                None,
                crate::extension::RemoteExtensionManifest::default(),
            )
            .unwrap();
        assert!(matches!(
            outcome,
            crate::session::PromptSubmissionOutcome::Started { .. }
        ));
        assert!(RemoteLeaseRuntime::new(&mut app)
            .leased_workflow_turn_context_for_provider_run(&run_id)
            .is_some());
        app.providers_mut()
            .record_terminal_diagnostic(&run_id, diagnostic.to_string())
            .unwrap();
        (leased, run_id)
    };
    if launch_failure {
        let run = app
            .lock()
            .await
            .providers()
            .get_run(&provider_run_id)
            .unwrap();
        runtime
            .fail_provider_launch(
                &crate::app::StartedProviderLaunch {
                    run,
                    previous_active_run_id: None,
                },
                &DaemonError::LocalTransport {
                    operation: "provider launch",
                    message: diagnostic.into(),
                },
            )
            .await;
    } else {
        runtime
            .fail_owned_provider_prompt(
                &leased_agent.backing_session_id,
                &provider_run_id,
                diagnostic,
                true,
            )
            .await
            .expect(
                "worker failure settlement must not require a home callback or relay connection",
            );
    }
    relay_probe.abort();
    assert_eq!(
        requests.load(Ordering::SeqCst),
        0,
        "worker failure settlement must not contact home"
    );
    let mut app = app.lock().await;
    assert!(app
        .prompt_owner_active_prompt_for_agent(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id
        )
        .unwrap()
        .is_none());
    let (target, event) = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection_with_recovery(
            &leased_agent.id,
            &provider_run_id,
            false,
            true,
        )
        .unwrap()
        .expect("settled failure must remain available for delivery to home");
    assert_eq!(target, "offline-home");
    let RelayPeerEvent::LeasedRuntimeProjection {
        provider_run,
        completions,
        ..
    } = event;
    assert!(provider_run
        .unwrap()
        .terminal_diagnostic()
        .unwrap()
        .contains(diagnostic));
    assert_eq!(completions.len(), 1);
    assert_eq!(
        completions[0].home_prompt_id.as_deref(),
        Some("home-prompt-1")
    );
    let (_, replay) = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection_with_recovery(
            &leased_agent.id,
            &provider_run_id,
            false,
            true,
        )
        .unwrap()
        .expect("an unacknowledged completion must be recoverable");
    let RelayPeerEvent::LeasedRuntimeProjection {
        completions: retried,
        ..
    } = replay;
    assert_eq!(retried.len(), 1);
    assert_eq!(retried[0].home_prompt_id, completions[0].home_prompt_id);
    assert_eq!(retried[0].message_id, completions[0].message_id);
}
