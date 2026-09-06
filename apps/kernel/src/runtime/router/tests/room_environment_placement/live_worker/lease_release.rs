use super::*;
use crate::transport::relay_client::send_peer_request_via_temporary_connection;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use chariox_relay::protocol::ClientTarget;
use futures_util::FutureExt;

#[test]
fn room_environment_worker_lease_release_cleans_its_agents_and_preserves_other_leases() {
    run_test(lease_release_cleans_its_agents_and_preserves_other_leases);
}

async fn peer(fixture: &LiveWorker, request: RelayPeerRequest) -> RelayPeerResponse {
    send_peer_request_via_temporary_connection(
        &fixture.home_state.config,
        ClientTarget {
            daemon_id: Some("environment-worker".to_string()),
            daemon_alias: None,
        },
        request,
    )
    .await
    .expect("worker peer request")
}

async fn lease_release_cleans_its_agents_and_preserves_other_leases() {
    let mut fixture = LiveWorker::start().await;
    let result = std::panic::AssertUnwindSafe(check_lease_release(&fixture))
        .catch_unwind()
        .await;
    // The fixture owns this entire worker. Always remove its synthetic provider
    // processes, including when a failed lease release has forgotten its agents.
    let cleanup = fixture
        .worker
        .app
        .lock()
        .await
        .teardown_provider_processes(Some("managed-dev-stub"), true);
    fixture.stop().await;
    cleanup.expect("remove fixture-owned provider processes");
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

async fn check_lease_release(fixture: &LiveWorker) {
    let workspace = fixture.home_state.root.join("worker-workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut leases = Vec::new();
    for home_agent_id in ["first", "second"] {
        let RelayPeerResponse::ExecutionLeaseCreated { lease, .. } = peer(
            fixture,
            RelayPeerRequest::CreateExecutionLease {
                home_kernel_id: "environment-home".to_string(),
                home_session_id: fixture.rooms[0].clone(),
                home_agent_id: home_agent_id.to_string(),
                home_agent_metaagent: false,
                owner_user_id: crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
            },
        )
        .await
        else {
            panic!("expected created lease")
        };
        leases.push(lease);
    }
    let mut agents = Vec::new();
    for lease in [&leases[0], &leases[0], &leases[1]] {
        let RelayPeerResponse::LeasedAgentSpawned { leased_agent } = peer(
            fixture,
            RelayPeerRequest::SpawnLeasedAgent {
                lease_id: lease.id.clone(),
                provider: "managed-dev-stub".to_string(),
                account_profile: "default".to_string(),
                model: Some("default".to_string()),
                effort: None,
                execution_mode: None,
                permission_level: None,
                workspace_live_sync_mode: None,
                worktree_id: Some(workspace.display().to_string()),
                worktree_placement: None,
            },
        )
        .await
        else {
            panic!("expected created agent")
        };
        agents.push(leased_agent);
    }
    let worker_room = &agents[0].backing_session_id;
    assert!(agents
        .iter()
        .all(|agent| &agent.backing_session_id == worker_room));
    let mut runs = Vec::new();
    for agent in &agents {
        let RelayPeerResponse::LeasedNativeProviderRunLaunched { provider_run } = peer(
            fixture,
            RelayPeerRequest::LaunchLeasedNativeProviderRun {
                leased_agent_id: agent.id.clone(),
                adapter_key: "managed-dev-stub".to_string(),
                provider: "managed-dev-stub".to_string(),
                account_profile: "default".to_string(),
                model: "default".to_string(),
                variant: None,
                structured_endpoint: None,
                provider_session_id: None,
                required_mcps: Vec::new(),
                required_skills: None,
                remote_extension_manifest: Default::default(),
                provider_launch_credential: None,
            },
        )
        .await
        else {
            panic!("expected launched provider")
        };
        runs.push(provider_run);
    }
    // Hidden worker Rooms are deliberately absent from the client Room list.
    // The public process inventory identifies the real managed PTYs instead.
    let inventory = dispatch_json(
        &fixture.worker,
        json!({"ListProviderProcesses":{"provider":"managed-dev-stub"}}),
    )
    .await
    .unwrap();
    let processes = inventory["ProviderProcessesListed"]["processes"]
        .as_array()
        .unwrap();
    assert_eq!(processes.len(), 3, "three independent managed processes");
    let pids = runs
        .iter()
        .map(|run| {
            let process = processes
                .iter()
                .find(|process| {
                    process["owner_provider_run_ids"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|id| id == run.id())
                })
                .expect("launched run appears in public inventory");
            u32::try_from(process["pid"].as_u64().expect("managed process PID")).unwrap()
        })
        .collect::<Vec<_>>();
    eprintln!("lease release fixture provider PIDs: {pids:?}");
    assert!(pids
        .iter()
        .all(|pid| crate::runtime::process_health::process_running(*pid)));

    let released = peer(
        fixture,
        RelayPeerRequest::DestroyExecutionLease {
            lease_id: leases[0].id.clone(),
        },
    )
    .await;
    assert_eq!(
        released,
        RelayPeerResponse::ExecutionLeaseDestroyed {
            lease_id: leases[0].id.clone()
        }
    );
    let stopped = timeout(Duration::from_secs(3), async {
        while pids[..2]
            .iter()
            .any(|pid| crate::runtime::process_health::process_running(*pid))
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .is_ok();
    assert!(
        stopped,
        "releasing a lease must stop all of its provider processes"
    );
    assert!(
        crate::runtime::process_health::process_running(pids[2]),
        "other lease's process must remain alive"
    );
    let input = peer(
        fixture,
        RelayPeerRequest::SendLeasedNativeProviderInput {
            leased_agent_id: agents[2].id.clone(),
            provider_run_id: runs[2].id().to_string(),
            attachment_id: "lease-release-drill".to_string(),
            data_base64: "b2sK".to_string(),
        },
    )
    .await;
    assert_eq!(
        input,
        RelayPeerResponse::LeasedNativeProviderInputSent { byte_count: 3 }
    );
    let removed = peer(
        fixture,
        RelayPeerRequest::DestroyLeasedAgent {
            leased_agent_id: agents[2].id.clone(),
        },
    )
    .await;
    assert_eq!(
        removed,
        RelayPeerResponse::LeasedAgentDestroyed {
            leased_agent_id: agents[2].id.clone()
        }
    );
    peer(
        fixture,
        RelayPeerRequest::DestroyExecutionLease {
            lease_id: leases[1].id.clone(),
        },
    )
    .await;
    assert!(
        !crate::runtime::process_health::process_running(pids[2]),
        "final agent cleanup stops the remaining process"
    );
}
