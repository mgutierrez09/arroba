use super::*;
use chariox_relay::{RelayConfig, RelayServer};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration};

mod batch;
mod cleanup;
mod computer_input;
mod controller;
mod controller_cancellation;
mod controller_compatibility;
mod controller_configuration;
mod controller_configuration_queue;
mod controller_configuration_recovery;
mod controller_events;
mod controller_integrations;
mod controller_mutations;
mod controller_observations;
mod controller_recovery;
mod controller_response_loss;
mod controller_upload_cancellation;
mod controller_upload_recovery;
mod controller_worker_mcp;
mod display;
mod lease_release;
mod screenshot;
mod session;

pub(super) async fn controller_placement_lifecycle() {
    controller::controller_scenario(false).await;
}

struct LiveWorker {
    home: Arc<CommandRouter>,
    worker: Arc<CommandRouter>,
    rooms: Vec<String>,
    shutdown: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
    address: std::net::SocketAddr,
    relay_addresses: Vec<std::net::SocketAddr>,
    private_relay: bool,
    worktrees: Vec<std::path::PathBuf>,
    home_state: TestState,
    _worker_state: TestState,
}

impl LiveWorker {
    async fn start() -> Self {
        Self::start_with_private_relay(false).await
    }

    async fn start_with_private_relay(private_relay: bool) -> Self {
        Self::start_configured(private_relay, false).await
    }

    async fn start_configured(private_relay: bool, browser_controller: bool) -> Self {
        Self::start_configured_with_home_vault(private_relay, browser_controller, None).await
    }

    async fn start_configured_with_home_vault(
        private_relay: bool,
        browser_controller: bool,
        home_vault_backend: Option<crate::config::CredentialVaultBackend>,
    ) -> Self {
        const HOME_TOKEN: &str = "environment-worker-fixture";
        // This isolated fixture's first slice is slice-1, owned by environment-home.
        const SLICE_TOKEN: &str = "slice-local-environment-home-slice-1";
        let (shutdown, receiver) = watch::channel(false);
        let tokens = if private_relay {
            vec![SLICE_TOKEN, HOME_TOKEN]
        } else {
            vec![HOME_TOKEN]
        };
        let mut relays = Vec::new();
        let mut tasks = Vec::new();
        for token in tokens {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let relay = Arc::new(RelayServer::new(RelayConfig {
                host: address.ip().to_string(),
                port: address.port(),
                shared_token: Some(token.to_string()),
            }));
            relays.push((Arc::clone(&relay), address));
            let mut relay_shutdown = receiver.clone();
            tasks.push(tokio::spawn(async move {
                relay
                    .run_listener_until(listener, async {
                        let _ = relay_shutdown.changed().await;
                    })
                    .await
                    .unwrap();
            }));
        }
        let address = relays[0].1;
        let worker_registry = relays[0].0.registry();
        let (home_relay, home_address) = relays.last().unwrap();
        let home_registry = home_relay.registry();

        let mut home_state = TestState::new();
        let mut worker_state = TestState::new();
        if let Some(backend) = home_vault_backend {
            home_state.config.user_config.credential_vault.backend = backend;
        }
        for state in [&mut home_state, &mut worker_state] {
            state.config.relay_url = Some(format!("ws://{address}"));
            state.config.relay_token = Some("environment-worker-fixture".to_string());
            state.config.relay_heartbeat_ms = 50;
        }
        home_state.config.relay_url = Some(format!("ws://{home_address}"));
        if private_relay {
            worker_state.config.relay_token = Some(SLICE_TOKEN.to_string());
        }
        home_state.config.daemon_id = "environment-home".to_string();
        worker_state.config.daemon_id = "environment-worker".to_string();
        worker_state.config.daemon_alias = Some("desktop-worker".to_string());
        worker_state.config.host_machine_id = "slice:slice-1".to_string();
        let (home, rooms) = home_state.router();
        let home = Arc::new(home);
        if browser_controller {
            worker_state.config.room_environment_worker_binding =
                Some(crate::config::RoomEnvironmentWorkerBinding {
                    home_kernel_id: home_state.config.daemon_id.clone(),
                    home_public_key: home_state.config.relay_public_key.clone(),
                    session_id: rooms[0].clone(),
                    slice_id: "slice-1".to_string(),
                });
        }
        let mut worker = CommandRouter::with_interactive_capacity(
            Arc::new(Mutex::new(
                DaemonApp::bootstrap(worker_state.config.clone()).unwrap(),
            )),
            2,
        );
        if browser_controller {
            worker
                .runtime_state
                .set_browser_controller_process_store_for_test(controller::process_store(
                    &worker_state.root,
                ));
        }
        let worker = Arc::new(worker);
        let mut fixture = Self {
            home: Arc::clone(&home),
            worker: Arc::clone(&worker),
            rooms,
            shutdown,
            tasks,
            address,
            relay_addresses: relays.iter().map(|(_, address)| *address).collect(),
            private_relay,
            worktrees: Vec::new(),
            home_state,
            _worker_state: worker_state,
        };
        for router in [home, worker] {
            let state = router.app.lock().await.relay_client_state();
            fixture.tasks.push(tokio::spawn(
                crate::transport::relay_client::run_daemon_relay_connector_with_router(
                    router,
                    state,
                    receiver.clone(),
                ),
            ));
        }
        timeout(Duration::from_secs(10), async {
            loop {
                let home_registered = home_registry
                    .read()
                    .await
                    .daemon("environment-home")
                    .is_some();
                let registered = home_registered
                    && worker_registry
                        .read()
                        .await
                        .daemon("environment-worker")
                        .is_some();
                if registered {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("both kernels register with the real relay");
        fixture
    }

    fn placement(&mut self) -> Value {
        let path = self
            .home_state
            .root
            .join(format!("leased-worktree-{}", self.worktrees.len()));
        self.worktrees.push(path.clone());
        json!({"target_directory":path,"from_ref":"HEAD"})
    }

    async fn create_slice(&self) {
        dispatch_json(
            &self.home,
            json!({"CreateSlice": {
                "name":"desktop", "base":"clean", "display_mode":"headed",
                "worker_kernel_ref":"desktop-worker"
            }}),
        )
        .await
        .unwrap();
        // Fixture discovery metadata: use this test's relay instead of Docker.
        self.home
            .app
            .lock()
            .await
            .slices()
            .set_relay_endpoint(
                "desktop",
                Some(crate::slice::SliceRelayEndpoint {
                    url: format!("ws://{}", self.address),
                    private: self.private_relay,
                }),
                1,
            )
            .unwrap();
        self.home
            .app
            .lock()
            .await
            .slices()
            .set_worker_presence(
                "desktop",
                Some("environment-worker".to_string()),
                Some("slice:slice-1".to_string()),
                vec!["managed-dev-stub".to_string()],
                crate::session::unix_epoch_ms(),
            )
            .unwrap();
        if self.private_relay {
            let slice = self
                .home
                .runtime_state
                .resolve_slice("desktop")
                .expect("private-relay fixture slice");
            let relay_config = self
                .home_state
                .config
                .slice_relay_override(&slice)
                .expect("private slice relay config");
            self.home
                .runtime_state
                .ensure_slice_private_relay_home_connection(
                    &slice.id,
                    relay_config.relay_url.expect("private slice relay URL"),
                    relay_config.relay_token.expect("private slice relay token"),
                )
                .await
                .expect("home kernel joins the private slice relay");
        }
    }

    async fn stop(&mut self) {
        if self.private_relay {
            self.home
                .runtime_state
                .stop_slice_private_relay_home_connection("slice-1")
                .await;
        }
        let _ = self.shutdown.send(true);
        let mut failures = Vec::new();
        for mut task in self.tasks.drain(..) {
            match timeout(Duration::from_secs(5), &mut task).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => failures.push(error.to_string()),
                Err(_) => {
                    task.abort();
                    let _ = task.await;
                    failures.push("fixture task did not stop".to_string());
                }
            }
        }
        assert!(failures.is_empty(), "fixture shutdown: {failures:?}");
        for address in &self.relay_addresses {
            drop(
                tokio::net::TcpListener::bind(address)
                    .await
                    .expect("relay port released"),
            );
        }
    }
}

impl Drop for LiveWorker {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        for task in &self.tasks {
            task.abort();
        }
        for worktree in self.worktrees.iter().filter(|path| path.exists()) {
            let result = std::process::Command::new("git")
                .args(["worktree", "remove", "--force"])
                .arg(worktree)
                .output()
                .expect("remove drill worktree");
            assert!(
                result.status.success(),
                "worktree cleanup: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }
}

#[test]
fn room_environment_worker_spawn_and_destroy_use_public_commands() {
    run_test(spawn_and_destroy_use_public_commands);
}

async fn spawn_and_destroy_use_public_commands() {
    let mut fixture = LiveWorker::start().await;
    let placement = fixture.placement();
    let spawned = dispatch_json(
        &fixture.home,
        json!({"SpawnAgent": {
            "session_id":fixture.rooms[0], "provider":"managed-dev-stub", "model":"default",
            "kernel_ref":"desktop-worker", "worktree_placement":placement
        }}),
    )
    .await
    .expect("spawn a real leased agent through the home router and relay");
    let agent = &spawned["AgentSpawned"]["agent"];
    let agent_id = agent["id"].as_str().unwrap();
    let before = dispatch_json(
        &fixture.home,
        json!({"ListAgents":{"session_id":fixture.rooms[0]}}),
    )
    .await
    .unwrap();
    assert!(before["AgentsListed"]["agents"]
        .as_array()
        .unwrap()
        .iter()
        .any(|agent| agent["id"] == agent_id));
    let destroyed = dispatch_json(
        &fixture.home,
        json!({"DestroyAgent": {
            "session_id":fixture.rooms[0], "agent_id":agent_id
        }}),
    )
    .await
    .expect("destroy the leased agent through the public command");
    let after = dispatch_json(
        &fixture.home,
        json!({"ListAgents":{"session_id":fixture.rooms[0]}}),
    )
    .await
    .unwrap();
    fixture.stop().await;
    assert_eq!(
        agent["remote_execution"]["worker_kernel_id"],
        "environment-worker"
    );
    assert_eq!(destroyed["AgentDestroyed"]["agent"]["id"], agent_id);
    assert!(after["AgentsListed"]["agents"]
        .as_array()
        .unwrap()
        .iter()
        .all(|agent| agent["id"] != agent_id));
}

#[test]
fn room_environment_worker_alias_attaches_agent_to_slice() {
    run_test(worker_alias_attaches_agent_to_slice);
}

async fn worker_alias_attaches_agent_to_slice() {
    let mut fixture = LiveWorker::start().await;
    fixture.create_slice().await;
    let placement = fixture.placement();
    let spawned = dispatch_json(
        &fixture.home,
        json!({"SpawnAgent": {
            "session_id":fixture.rooms[0], "provider":"managed-dev-stub", "model":"default",
            "kernel_ref":"desktop-worker", "worktree_placement":placement
        }}),
    )
    .await
    .expect("spawn through a known slice worker alias");
    let agent_id = spawned["AgentSpawned"]["agent"]["id"].as_str().unwrap();
    let attached = dispatch_json(&fixture.home, json!({"GetSlice":{"slice_ref":"desktop"}}))
        .await
        .unwrap();
    let other_claim = dispatch_json(&fixture.home, bind(&fixture.rooms[1], "desktop")).await;
    dispatch_json(
        &fixture.home,
        json!({"DestroyAgent": {
            "session_id":fixture.rooms[0], "agent_id":agent_id
        }}),
    )
    .await
    .expect("delete and detach the worker agent");
    let detached = dispatch_json(&fixture.home, json!({"GetSlice":{"slice_ref":"desktop"}}))
        .await
        .unwrap();
    fixture.stop().await;
    assert_eq!(
        attached["Slice"]["slice"]["agent_ids"],
        json!([agent_id]),
        "worker aliases must preserve the canonical slice attachment"
    );
    assert!(
        other_claim.is_err(),
        "a Room cannot claim a slice with another Room's agent"
    );
    let detached: crate::slice::SliceRecord =
        serde_json::from_value(detached["Slice"]["slice"].clone()).unwrap();
    assert!(
        detached.agent_ids.is_empty(),
        "deletion clears the slice's durable agent attachment"
    );
}

#[test]
fn room_environment_worker_batch_preserves_mixed_target_attachments() {
    run_test(batch_preserves_mixed_target_attachments);
}

async fn batch_preserves_mixed_target_attachments() {
    let mut fixture = LiveWorker::start().await;
    fixture.create_slice().await;
    let alias_placement = fixture.placement();
    let slice_placement = fixture.placement();
    let spawned = dispatch_json(&fixture.home, json!({"SpawnAgents":{
        "session_id":fixture.rooms[0], "agents":[
            {"provider":"managed-dev-stub", "model":"default"},
            {"provider":"managed-dev-stub", "model":"default", "kernel_ref":"desktop-worker", "worktree_placement":alias_placement},
            {"provider":"managed-dev-stub", "model":"default", "slice_ref":"desktop", "worktree_placement":slice_placement}
        ]
    }})).await.expect("mixed local, worker-alias and slice batch");
    let agents = spawned["AgentsSpawned"]["agents"].as_array().unwrap();
    assert_eq!(agents.len(), 3);
    let attached = dispatch_json(&fixture.home, json!({"GetSlice":{"slice_ref":"desktop"}}))
        .await
        .unwrap();
    for agent in agents {
        dispatch_json(
            &fixture.home,
            json!({"DestroyAgent":{
                "session_id":fixture.rooms[0],"agent_id":agent["id"]
            }}),
        )
        .await
        .expect("remove each batch agent");
    }
    fixture.stop().await;
    assert!(
        agents[0]["remote_execution"].is_null(),
        "first batch target stays local"
    );
    assert_eq!(
        agents[1]["remote_execution"]["worker_kernel_id"],
        "environment-worker"
    );
    assert_eq!(
        agents[2]["remote_execution"]["worker_kernel_id"],
        "environment-worker"
    );
    assert_eq!(
        attached["Slice"]["slice"]["agent_ids"],
        json!([agents[1]["id"], agents[2]["id"]]),
        "mixed batch preserves target order and attaches both aliases of one slice"
    );
}

#[test]
fn room_environment_worker_cleanup_failure_preserves_agent_and_slice() {
    run_test(cleanup_failure_preserves_agent_and_slice);
}

async fn cleanup_failure_preserves_agent_and_slice() {
    let mut fixture = LiveWorker::start().await;
    fixture.create_slice().await;
    let placement = fixture.placement();
    let spawned = dispatch_json(
        &fixture.home,
        json!({"SpawnAgent":{
            "session_id":fixture.rooms[0], "provider":"managed-dev-stub", "model":"default",
            "kernel_ref":"desktop-worker", "worktree_placement":placement
        }}),
    )
    .await
    .unwrap();
    let agent_id = spawned["AgentSpawned"]["agent"]["id"].as_str().unwrap();
    // Cut transport before cleanup. The home cannot prove the worker stopped.
    fixture.stop().await;
    let error = dispatch_json(
        &fixture.home,
        json!({"DestroyAgent":{
            "session_id":fixture.rooms[0], "agent_id":agent_id
        }}),
    )
    .await
    .unwrap_err();
    let listed = dispatch_json(
        &fixture.home,
        json!({"ListAgents":{"session_id":fixture.rooms[0]}}),
    )
    .await
    .unwrap();
    let attached = dispatch_json(&fixture.home, json!({"GetSlice":{"slice_ref":"desktop"}}))
        .await
        .unwrap();
    assert!(
        listed["AgentsListed"]["agents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|agent| agent["id"] == agent_id),
        "failed remote cleanup must not forget the home agent"
    );
    assert_eq!(attached["Slice"]["slice"]["agent_ids"], json!([agent_id]));
    assert!(
        std::error::Error::source(&error).is_some(),
        "cleanup diagnostics must retain the underlying failure: {error}"
    );
    let websocket_error = crate::transport::kernel_protocol::map_kernel_error(&error);
    assert_eq!(websocket_error.code, "local_transport_error");
    assert!(
        websocket_error.retryable,
        "WebSocket clients must retain the transport retry classification"
    );
    assert!(websocket_error.message.contains("agent retained"));
    let error = error.to_string();
    assert!(
        error.contains("agent retained") && error.contains("retry"),
        "failed cleanup must explain that the agent remains tracked and can be retried: {error}"
    );
}
