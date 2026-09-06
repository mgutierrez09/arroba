use super::*;
use crate::app::RemoteLeaseRuntime;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse, RelayProjectedCompletion};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_workflow_exhaustion_confirms_substitute_and_advances_queue_once() {
    assert_remote_workflow_substitute_confirmation(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejected_remote_substitute_preserves_workflow_queue_and_releases_admission() {
    assert_remote_workflow_substitute_confirmation(false).await;
}

async fn assert_remote_workflow_substitute_confirmation(valid_acknowledgement: bool) {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.relay_url = Some("ws://127.0.0.1:1".into());
    config.relay_token = Some("test-relay".into());
    let home_key = config.relay_public_key.clone();
    let home_id = config.daemon_id.clone();
    let (app, runtime, session_id, agent_id) = agent_config_runtime_with_config(config).await;
    let mut worker_config = crate::config::DaemonConfig::for_tests();
    worker_config.accept_remote_leases = true;
    let worker_id = worker_config.daemon_id.clone();
    let worker_private = worker_config.relay_private_key.clone();
    let worker_public = worker_config.relay_public_key.clone();
    let mut worker = DaemonApp::bootstrap(worker_config).unwrap();
    let lease = RemoteLeaseRuntime::new(&mut worker)
        .create_execution_lease(
            &home_id,
            &session_id,
            &agent_id,
            false,
            crate::session::DEFAULT_LOCAL_USER_ID,
        )
        .unwrap();
    let leased_agent = RemoteLeaseRuntime::new(&mut worker)
        .create_leased_agent(
            &lease.id,
            "dev-stub",
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
    let (run_id, prompt_id, queued_id) = {
        let mut app = app.lock().await;
        app.agents_mut()
            .set_agent_runtime_profile(
                &agent_id,
                "claude-headless",
                Some("claude-opus-4-8".into()),
                Some("high".into()),
                crate::provider::ProviderResumeState::default(),
            )
            .unwrap();
        app.agents_mut()
            .bind_remote_execution(
                &agent_id,
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: worker_id.clone(),
                    worker_machine_id: "worker-machine".into(),
                    execution_lease_id: lease.id.clone(),
                    leased_agent_id: leased_agent.id.clone(),
                    active_worker_provider_run_id: Some("failed-worker-run".into()),
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .unwrap();
        let workflow = app
            .sessions_mut()
            .create_workflow(&session_id, None)
            .unwrap();
        let node = app
            .sessions_mut()
            .add_workflow_node(&session_id, workflow.id(), &agent_id)
            .unwrap();
        let endpoint = app
            .sessions_mut()
            .create_workflow_endpoint(&session_id, workflow.id(), node.id(), None)
            .unwrap();
        let run = app
            .sessions_mut()
            .invoke_workflow_endpoint(
                &session_id,
                workflow.id(),
                endpoint.id(),
                Some("failed review".into()),
            )
            .unwrap();
        let node_id = run.node_runs()[0].id().to_string();
        app.sessions_mut()
            .prepare_workflow_turn(
                &session_id,
                run.id(),
                &node_id,
                "failure-token".into(),
                "failed review".into(),
                None,
                None,
            )
            .unwrap();
        app.sessions_mut()
            .start_workflow_node_run(&session_id, run.id(), &node_id)
            .unwrap();
        app.acquire_workflow_node_workspace_claim(
            &session_id,
            "failed-worker-run",
            &agent_id,
            run.id(),
            &node_id,
        )
        .unwrap();
        let prompt = crate::session::PromptQueueItem::new(
            "failed-prompt",
            crate::scheduler::runtime::workflow_prompt_source_attachment_id(run.id()),
            &agent_id,
            "failed review",
            crate::session::PromptStatus::Queued,
        )
        .with_workflow_context(run.id(), &node_id);
        let crate::session::PromptSubmissionOutcome::Started { prompt } = app
            .prompt_owner_submit_prepared_prompt(&session_id, prompt, false)
            .unwrap()
        else {
            panic!("failure fixture must start");
        };
        let queued = app
            .sessions_mut()
            .enqueue_workflow_prompt(
                &session_id,
                workflow.id(),
                endpoint.id(),
                Some("next review".into()),
                None,
                crate::session::WorkflowQueuedPromptSource::Manual,
                None,
            )
            .unwrap();
        (
            run.id().to_string(),
            prompt.id().to_string(),
            queued.id().to_string(),
        )
    };
    runtime
        .update_agent_substitutes(
            &session_id,
            &agent_id,
            crate::session::DEFAULT_LOCAL_USER_ID,
            crate::local::AgentSubstituteAction::Add {
                provider: "dev-stub".into(),
                model: "fallback-model".into(),
                variant: Some("low".into()),
                account_profile: None,
                kernel_id: None,
                worktree_id: None,
            },
        )
        .await
        .unwrap();
    let relay_state = app.lock().await.relay_client_state();
    let (sender, mut requests, _events) =
        crate::transport::relay_client::RelayOutgoingSender::channel(4);
    {
        let mut relay = relay_state.write().await;
        relay.test_set_connected_sender(sender, "ws://127.0.0.1:1".to_string());
        relay.remember_peer_public_key(&worker_id, worker_public);
    }
    let request = crate::provider::LaunchProviderRequest::new(
        &leased_agent.backing_session_id,
        "claude",
        "claude-headless",
        "default",
        "claude-opus-4-8",
    );
    let mut failed = crate::provider::RuntimeProviderRun::new(
        "failed-worker-run",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "failed-worker".into(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    failed.set_terminal_diagnostic("You've hit your session limit");
    let completion = tokio::spawn({
        let runtime = runtime.clone();
        let session_id = session_id.clone();
        let agent_id = agent_id.clone();
        async move {
            runtime
                .project_relay_remote_runtime_projection(
                    &session_id,
                    &agent_id,
                    "failed-worker-run",
                    Some(failed),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    vec![RelayProjectedCompletion {
                        message_id: "failed-completion".into(),
                        completed_at_ms: crate::session::unix_epoch_ms(),
                        home_prompt_id: Some(prompt_id),
                    }],
                )
                .await
        }
    });
    let received = tokio::time::timeout(std::time::Duration::from_secs(2), requests.recv()).await;
    if received.is_err() {
        completion.abort();
    }
    let envelope = received
        .expect("exhaustion completion must request the configured substitute")
        .unwrap();
    let chariox_relay::protocol::RelayEnvelope::DaemonPeerRequest {
        request_id,
        target,
        encrypted_request,
    } = envelope
    else {
        panic!("expected encrypted worker request");
    };
    assert_eq!(target.daemon_id.as_deref(), Some(worker_id.as_str()));
    let payload = crate::transport::relay_crypto::decrypt_payload_for_private_key(
        &worker_private,
        &encrypted_request,
    )
    .unwrap();
    let RelayPeerRequest::UpdateLeasedAgentProfile {
        leased_agent_id,
        provider,
        account_profile,
        model,
        effort,
    } = serde_json::from_slice(&payload.plaintext).unwrap()
    else {
        panic!("must confirm profile before new work");
    };
    assert_eq!(provider, "dev-stub");
    assert_eq!(model.as_deref(), Some("fallback-model"));
    let next_workspace_claim = runtime
        .owned
        .workspace_coordinator
        .acquire_worktree_write_claim(
            "workspace-1",
            "worktree-1",
            &session_id,
            None,
            "next_prompt_dispatch",
        )
        .expect("failed turn must release its workspace before substitute confirmation");
    drop(next_workspace_claim);
    let current = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .unwrap();
    assert_eq!(
        current.workflow_queued_prompts().len(),
        1,
        "queued workflow must wait for worker confirmation"
    );
    assert!(runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&current, &agent_id)
        .is_none());
    assert_eq!(
        current.workflow_run(&run_id).unwrap().failure_events()[0].kind(),
        crate::session::WorkflowFailureKind::ProviderFailure
    );
    assert_eq!(
        runtime
            .owned
            .agent_store
            .get_agent(&agent_id)
            .unwrap()
            .active_substitute_index(),
        None,
        "home must wait for worker confirmation"
    );
    let mut updated = RemoteLeaseRuntime::new(&mut worker)
        .update_leased_agent_profile(&leased_agent_id, provider, account_profile, model, effort)
        .unwrap();
    if !valid_acknowledgement {
        updated.account_profile = "wrong-worker-account".into();
    }
    let response = RelayPeerResponse::LeasedAgentProfileUpdated {
        leased_agent: updated,
    };
    let encrypted = crate::transport::relay_crypto::encrypt_payload_for_peer(
        &worker_private,
        &home_key,
        &serde_json::to_vec(&response).unwrap(),
    )
    .unwrap();
    crate::transport::relay_client::resolve_pending_peer_response_for_test(
        &relay_state,
        request_id,
        worker_id,
        encrypted,
    )
    .await;
    completion.await.unwrap().unwrap();
    let selected = runtime.owned.agent_store.get_agent(&agent_id).unwrap();
    if !valid_acknowledgement {
        assert_eq!(selected.active_substitute_index(), None);
        assert_eq!(selected.provider(), "claude-headless");
        assert_eq!(selected.model(), Some("claude-opus-4-8"));
        let current = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .unwrap();
        assert_eq!(current.workflow_queued_prompts().len(), 1);
        assert_eq!(current.workflow_queued_prompts()[0].id(), queued_id);
        assert!(!current
            .workflow_runs()
            .iter()
            .any(|run| run.queue_item_id() == Some(queued_id.as_str())));
        assert!(runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&current, &agent_id)
            .is_none());
        let retry_claim = runtime
            .owned
            .prompt_state_owner
            .claim_idle_agent_profile_transition(&current, &agent_id)
            .expect(
                "failed confirmation must release admission so explicit repair remains possible",
            );
        drop(retry_claim);
        return;
    }
    assert_eq!(selected.active_substitute_index(), Some(0));
    assert_eq!(selected.provider(), "dev-stub");
    assert_eq!(selected.primary_provider(), "claude-headless");
    assert_eq!(selected.model(), Some("fallback-model"));
    let current = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .unwrap();
    let successors = current
        .workflow_runs()
        .iter()
        .filter(|run| run.queue_item_id() == Some(queued_id.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        successors.len(),
        1,
        "the queued workflow must advance exactly once on the substitute"
    );
    assert_ne!(successors[0].id(), run_id);
    assert!(current.workflow_queued_prompts().is_empty());
}
