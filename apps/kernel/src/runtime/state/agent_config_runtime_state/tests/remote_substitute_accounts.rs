use super::*;
use crate::app::RemoteLeaseRuntime;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_substitute_materializes_selected_account_before_worker_profile_change() {
    assert_remote_substitute_account_transfer(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_substitute_rejects_wrong_account_transfer_ack_without_changing_profile() {
    assert_remote_substitute_account_transfer(false).await;
}

async fn assert_remote_substitute_account_transfer(valid_account_ack: bool) {
    let owner = crate::session::DEFAULT_LOCAL_USER_ID;
    let mut config = crate::config::DaemonConfig::for_tests();
    config.relay_url = Some("ws://127.0.0.1:1".into());
    config.relay_token = Some("fixture-relay".into());
    let home_id = config.daemon_id.clone();
    let home_key = config.relay_public_key.clone();
    let (app, runtime, session_id, agent_id) = agent_config_runtime_with_config(config).await;
    let registry = &runtime.owned.provider_account_profiles;
    let profile = registry
        .create_managed(owner, "codex", "Selected fixture")
        .unwrap();
    let source_env = registry
        .resolve_environment(owner, "codex", &profile.profile_id)
        .unwrap();
    let credential = br#"{"OPENAI_API_KEY":"fixture-only-not-a-real-key"}"#;
    std::fs::write(
        std::path::Path::new(&source_env["CODEX_HOME"]).join("auth.json"),
        credential,
    )
    .unwrap();
    let unrelated = registry
        .create_managed(owner, "codex", "Unrelated fixture")
        .unwrap();

    let mut worker_config = crate::config::DaemonConfig::for_tests();
    worker_config.accept_remote_leases = true;
    let worker_id = worker_config.daemon_id.clone();
    let worker_private = worker_config.relay_private_key.clone();
    let worker_public = worker_config.relay_public_key.clone();
    let mut worker = DaemonApp::bootstrap(worker_config).unwrap();
    let lease = RemoteLeaseRuntime::new(&mut worker)
        .create_execution_lease(&home_id, &session_id, &agent_id, false, owner)
        .unwrap();
    let leased = RemoteLeaseRuntime::new(&mut worker)
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
    assert!(worker
        .provider_account_profile_registry()
        .get(owner, "codex", &profile.profile_id)
        .is_err());
    app.lock()
        .await
        .agents_mut()
        .bind_remote_execution(
            &agent_id,
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: worker_id.clone(),
                worker_machine_id: "fixture-machine".into(),
                execution_lease_id: lease.id.clone(),
                leased_agent_id: leased.id.clone(),
                active_worker_provider_run_id: None,
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .unwrap();
    runtime
        .update_agent_substitutes(
            &session_id,
            &agent_id,
            owner,
            crate::local::AgentSubstituteAction::Add {
                provider: "codex".into(),
                model: "gpt-5.6-sol".into(),
                variant: Some("high".into()),
                account_profile: Some(profile.profile_id.clone()),
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
    let switch = tokio::spawn({
        let runtime = runtime.clone();
        let session_id = session_id.clone();
        let agent_id = agent_id.clone();
        async move {
            runtime
                .update_agent_substitutes(
                    &session_id,
                    &agent_id,
                    owner,
                    crate::local::AgentSubstituteAction::Activate {
                        index: 0,
                        reason: None,
                    },
                )
                .await
        }
    });
    for step in 0..2 {
        let envelope = tokio::time::timeout(std::time::Duration::from_secs(2), requests.recv())
            .await
            .unwrap()
            .unwrap();
        let chariox_relay::protocol::RelayEnvelope::DaemonPeerRequest {
            request_id,
            target,
            encrypted_request,
        } = envelope
        else {
            panic!("expected encrypted worker request")
        };
        assert_eq!(target.daemon_id.as_deref(), Some(worker_id.as_str()));
        let payload = crate::transport::relay_crypto::decrypt_payload_for_private_key(
            &worker_private,
            &encrypted_request,
        )
        .unwrap();
        let request: RelayPeerRequest = serde_json::from_slice(&payload.plaintext).unwrap();
        let response = match (step, request) {
            (
                0,
                RelayPeerRequest::EnsureRemoteProviderAccount {
                    context,
                    materialization,
                },
            ) => {
                assert_eq!(context.home_kernel_id, home_id);
                assert_eq!(context.home_session_id, session_id);
                assert_eq!(context.home_agent_id, agent_id);
                assert_eq!(context.execution_lease_id, lease.id);
                assert_eq!(materialization.profile.profile_id, profile.profile_id);
                let imported = RemoteLeaseRuntime::new(&mut worker)
                    .ensure_remote_provider_account(context, materialization)
                    .unwrap();
                RelayPeerResponse::RemoteProviderAccountEnsured {
                    provider: imported.provider,
                    account_profile: if valid_account_ack {
                        imported.profile_id
                    } else {
                        "wrong-account".into()
                    },
                }
            }
            (
                1,
                RelayPeerRequest::UpdateLeasedAgentProfile {
                    leased_agent_id,
                    provider,
                    account_profile,
                    model,
                    effort,
                },
            ) => {
                assert_eq!(account_profile, profile.profile_id);
                let updated = RemoteLeaseRuntime::new(&mut worker)
                    .update_leased_agent_profile(
                        &leased_agent_id,
                        provider,
                        account_profile,
                        model,
                        effort,
                    )
                    .unwrap();
                RelayPeerResponse::LeasedAgentProfileUpdated {
                    leased_agent: updated,
                }
            }
            _ => {
                switch.abort();
                panic!("selected account must be transferred before the worker profile changes")
            }
        };
        assert_eq!(
            runtime
                .owned
                .agent_store
                .get_agent(&agent_id)
                .unwrap()
                .active_substitute_index(),
            None
        );
        let encrypted = crate::transport::relay_crypto::encrypt_payload_for_peer(
            &worker_private,
            &home_key,
            &serde_json::to_vec(&response).unwrap(),
        )
        .unwrap();
        crate::transport::relay_client::resolve_pending_peer_response_for_test(
            &relay_state,
            request_id,
            worker_id.clone(),
            encrypted,
        )
        .await;
        if !valid_account_ack {
            assert!(switch.await.unwrap().is_err());
            assert!(
                requests.try_recv().is_err(),
                "rejected account transfer must not request a profile change"
            );
            assert_eq!(
                runtime
                    .owned
                    .agent_store
                    .get_agent(&agent_id)
                    .unwrap()
                    .active_substitute_index(),
                None
            );
            assert_eq!(
                RemoteLeaseRuntime::new(&mut worker).execution_lease_count(),
                1
            );
            let worker_agent = worker.agents().get_agent(&leased.backing_agent_id).unwrap();
            assert_eq!(worker_agent.provider(), "dev-stub");
            let session = runtime
                .owned
                .session_store
                .get_session(&session_id)
                .unwrap();
            let claim = runtime
                .owned
                .prompt_state_owner
                .claim_idle_agent_profile_transition(&session, &agent_id)
                .unwrap();
            drop(claim);
            return;
        }
    }
    let selected = switch.await.unwrap().unwrap();
    assert_eq!(selected.provider_account_profile(), profile.profile_id);
    assert_eq!(selected.active_substitute_index(), Some(0));
    let target_env = worker
        .provider_account_profile_registry()
        .resolve_environment(owner, "codex", &profile.profile_id)
        .unwrap();
    assert_ne!(target_env["CODEX_HOME"], source_env["CODEX_HOME"]);
    assert_eq!(
        std::fs::read(std::path::Path::new(&target_env["CODEX_HOME"]).join("auth.json")).unwrap(),
        credential
    );
    assert!(worker
        .provider_account_profile_registry()
        .get(owner, "codex", &unrelated.profile_id)
        .is_err());
}
