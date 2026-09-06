use super::*;
use futures_util::FutureExt;

pub(super) fn process_store(
    root: &std::path::Path,
) -> crate::runtime::browser_controller_process::BrowserControllerProcessStore {
    let kernel = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    crate::runtime::browser_controller_process::BrowserControllerProcessStore::new(
        "node",
        vec![
            kernel.join("src/runtime/router/tests/room_environment_placement/live_worker/controller.fixture.mjs").display().to_string(),
            kernel.join("slice-linux-docker/docker").display().to_string(),
            root.join("controller.pid").display().to_string(),
        ],
        Duration::from_secs(3),
    )
}

#[test]
fn room_environment_controller_uses_its_slice_without_worker_agents() {
    run_test(uses_its_slice_without_worker_agents);
}

async fn uses_its_slice_without_worker_agents() {
    controller_scenario(false).await;
}

#[test]
fn room_environment_controller_uses_its_private_slice_relay() {
    run_test(uses_private_slice_relay);
}

async fn uses_private_slice_relay() {
    controller_scenario(true).await;
}

#[test]
fn room_environment_controller_rejects_unprovisioned_worker() {
    run_test(rejects_unprovisioned_worker);
}

async fn rejects_unprovisioned_worker() {
    let mut fixture = LiveWorker::start().await;
    let result = std::panic::AssertUnwindSafe(async {
        fixture.create_slice().await;
        let room = &fixture.rooms[0];
        dispatch_json(
            &fixture.home,
            json!({"BindRoomEnvironmentSlice": {
                "session_id":room,"slice_ref":"desktop"
            }}),
        )
        .await
        .unwrap();
        let error = dispatch_json(
            &fixture.home,
            json!({"StartRoomEnvironment": {
                "session_id":room,"viewport":{
                    "css_width":1280,"css_height":800,"device_scale_factor":1,
                    "desktop_pixel_width":1280,"desktop_pixel_height":800
                }
            }}),
        )
        .await
        .expect_err("unprovisioned worker must not yield a fake healthy Environment");
        assert!(
            error
                .to_string()
                .contains("browser_controller_scope_denied"),
            "{error}"
        );
    })
    .catch_unwind()
    .await;
    fixture.stop().await;
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

#[test]
fn room_environment_controller_boot_rejects_invalid_binding() {
    let state = TestState::new();
    for mismatch in ["key", "room", "kernel", "slice", "machine"] {
        let mut config = state.config.clone();
        config.host_machine_id = "slice:slice-1".into();
        let mut binding = crate::config::RoomEnvironmentWorkerBinding {
            home_kernel_id: "home".into(),
            home_public_key: config.relay_public_key.clone(),
            session_id: "room-1".into(),
            slice_id: "slice-1".into(),
        };
        match mismatch {
            "key" => binding.home_public_key = "not-a-relay-key".into(),
            "room" => binding.session_id.clear(),
            "kernel" => binding.home_kernel_id.clear(),
            "slice" => binding.slice_id.clear(),
            "machine" => config.host_machine_id = "another-machine".into(),
            _ => unreachable!(),
        }
        config.room_environment_worker_binding = Some(binding);
        assert!(
            matches!(
                DaemonApp::bootstrap(config),
                Err(DaemonError::InvalidConfig {
                    field: "room_environment_worker_binding",
                    ..
                })
            ),
            "{mismatch} must fail at worker boot"
        );
    }
}

pub(super) async fn controller_scenario(private_relay: bool) {
    let mut fixture = LiveWorker::start_configured(private_relay, true).await;
    let result = std::panic::AssertUnwindSafe(check_slice_controller(&mut fixture))
        .catch_unwind()
        .await;
    let pids = std::fs::read_to_string(fixture._worker_state.root.join("controller.pids"))
        .unwrap_or_default()
        .lines()
        .map(|pid| pid.parse::<u32>().expect("controller PID"))
        .collect::<Vec<_>>();
    let cleanup = fixture
        .worker
        .runtime_state
        .shutdown_browser_controller_process()
        .await;
    let provider_cleanup = fixture
        .worker
        .app
        .lock()
        .await
        .teardown_provider_processes(Some("managed-dev-stub"), true);
    fixture.stop().await;
    cleanup.expect("stop fixture controller on success and failure");
    provider_cleanup.expect("stop fixture worker provider on success and failure");
    for pid in pids {
        eprintln!("relay controller fixture PID: {pid}");
        assert!(
            !crate::runtime::process_health::process_running(pid),
            "controller must be reaped"
        );
    }
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

async fn check_slice_controller(fixture: &mut LiveWorker) {
    let worker_mcp_placement = fixture.placement();
    fixture.create_slice().await;
    fixture
        .home
        .app
        .lock()
        .await
        .slices()
        .set_status(
            "desktop",
            crate::slice::SliceStatus::Running,
            crate::session::unix_epoch_ms(),
        )
        .expect("live worker fixture slice should be running");
    let room = &fixture.rooms[0];
    let original = dispatch_json(
        &fixture.home,
        json!({"BindRoomEnvironmentSlice": {
            "session_id":room, "slice_ref":"desktop"
        }}),
    )
    .await
    .unwrap();
    let request = json!({"StartRoomEnvironment": {
        "session_id":room, "viewport": {
            "css_width":1280, "css_height":800, "device_scale_factor":1,
            "desktop_pixel_width":1280, "desktop_pixel_height":800
        }
    }});
    let first = dispatch_json(&fixture.home, request.clone())
        .await
        .expect("start bound Room browser without an execution lease");
    let environment = &first["RoomEnvironmentUpdated"]["environment"];
    assert_eq!(
        environment["lifecycle"], "ready",
        "a running headed Room slice must make Browser and Computer modes ready"
    );
    for component in ["browser_controller", "browser", "desktop", "streamer"] {
        assert!(
            environment["health"].as_array().is_some_and(|health| health
                .iter()
                .any(|entry| entry["component"] == component && entry["state"] == "ready")),
            "{component} must be ready after the bound headed slice starts: {environment}"
        );
    }
    assert_eq!(
        environment["tabs"][0]["url"], "https://worker.test/",
        "Room tabs must come from its bound worker controller, not an empty home store"
    );
    let token = {
        let mut app = fixture.home.app.lock().await;
        let agent = spawn_test_agent(&mut app, room, "browser-reader", "dev-stub");
        launch_test_provider(&mut app, room, agent.id(), "dev-stub", "dev-stub", "test")
            .runtime_mcp_auth_token()
            .unwrap()
            .to_string()
    };
    let status = fixture
        .home
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(&token, "slice_browser_status", json!({}))
        .await
        .expect("home Room agent reads its bound worker browser through runtime MCP");
    assert!(status.ok, "{:?}", status.payload);
    assert_eq!(status.payload["session_id"], *room);
    assert_eq!(status.payload["tab_id"], environment["tabs"][0]["tab_id"]);
    assert_eq!(
        status.payload["browser"]["buttons"][0]["label"],
        "Save on worker"
    );
    assert!(status.payload["browser"]["buttons"][0]["field_id"]
        .as_str()
        .is_some_and(|reference| reference.starts_with("element-")));
    assert!(fixture
        .home
        .runtime_state
        .runtime_tool_specs_for_auth_token(&token)
        .iter()
        .any(|spec| spec.name == "slice_browser_status"));
    super::controller_observations::check(fixture, &token, &status.payload).await;
    super::controller_mutations::check(fixture, &token, &status.payload).await;
    super::controller_cancellation::check(fixture, &token, &status.payload).await;
    super::controller_cancellation::check_running(fixture, &token, &status.payload).await;
    super::controller_response_loss::check(fixture, &token).await;
    super::controller_integrations::check(fixture, &token, &status.payload).await;
    super::controller_events::check(fixture, &token, &status.payload).await;
    super::controller_recovery::check(fixture, &token).await;
    // A worker-local Room can even have the same textual session ID as the
    // home Room. It must not claim a provisioned browser via the local API.
    let local_room = {
        let mut app = fixture.worker.app.lock().await;
        crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .unwrap()
            .0
            .id()
            .to_string()
    };
    let local_error = dispatch_json(
        &fixture.worker,
        json!({"StartRoomEnvironment":{
            "session_id":local_room,"viewport":{
                "css_width":1280,"css_height":800,"device_scale_factor":1,
                "desktop_pixel_width":1280,"desktop_pixel_height":800
            }
        }}),
    )
    .await
    .expect_err("worker-local Room must not bypass home authorization");
    assert!(
        local_error
            .to_string()
            .contains("browser_controller_scope_denied"),
        "{local_error}"
    );
    let slice = fixture
        .home
        .app
        .lock()
        .await
        .slices()
        .environment_slice(room)
        .unwrap();
    let owner = fixture
        .home_state
        .config
        .slice_relay_override(&slice)
        .unwrap_or_else(|| fixture.home_state.config.clone());
    // Neither a claimed kernel ID nor possession of the relay token grants
    // control of the browser. Every denied release must leave the owner intact.
    for mismatch in ["room", "slice", "kernel", "key"] {
        let mut sender = owner.clone();
        if mismatch == "kernel" {
            sender.daemon_id = "different-home".into();
        }
        if mismatch == "key" {
            sender.relay_private_key =
                crate::transport::relay_crypto::generate_private_key_base64();
            sender.relay_public_key =
                crate::transport::relay_crypto::public_key_from_private_key_base64(
                    &sender.relay_private_key,
                )
                .unwrap();
        }
        for command in [
            crate::transport::room_browser_controller::RoomBrowserControllerCommand::Release,
            crate::transport::room_browser_controller::RoomBrowserControllerCommand::CancelAction {
                execution_id: "00000000000000000000000000000000".into(),
            },
            crate::transport::room_browser_controller::RoomBrowserControllerCommand::Dialog {
                execution_id: "00000000000000000000000000000010".into(),
                target_id: "worker-tab".into(),
                document_id: "worker-document".into(),
                action: crate::runtime::browser_controller_action::BrowserDialogAction::Dismiss,
            },
            crate::transport::room_browser_controller::RoomBrowserControllerCommand::Tab {
                execution_id: "00000000000000000000000000000011".into(),
                target_id: "worker-tab".into(),
                document_id: "worker-document".into(),
                action: crate::runtime::browser_controller_tab::BrowserTabAction::Activate,
            },
            crate::transport::room_browser_controller::RoomBrowserControllerCommand::History {
                execution_id: "00000000000000000000000000000012".into(),
                target_id: "worker-tab".into(),
                document_id: "worker-document".into(),
                action: crate::runtime::browser_controller_history::BrowserHistoryAction::Back,
            },
            crate::transport::room_browser_controller::RoomBrowserControllerCommand::ConfigureDownloads {
                execution_id: "00000000000000000000000000000003".into(),
                target_id: "worker-tab".into(),
                document_id: "worker-document".into(),
            },
            crate::transport::room_browser_controller::RoomBrowserControllerCommand::CancelDownload {
                cancellation: crate::runtime::browser_controller_file_transfer::BrowserDownloadCancellation::new(
                    1, "worker-active-download".into(),
                ).unwrap(),
            },
            crate::transport::room_browser_controller::RoomBrowserControllerCommand::Upload {
                execution_id: "00000000000000000000000000000002".into(),
                target_id: "worker-tab".into(),
                document_id: "worker-document".into(),
                node_ref: "backend:104".into(),
                files: crate::runtime::browser_controller_file_transfer::BrowserUploadFiles::new(
                    vec![fixture._worker_state.root.join("denied-upload.txt")],
                )
                .unwrap(),
            },
            crate::transport::room_browser_controller::RoomBrowserControllerCommand::Permission {
                execution_id: "00000000000000000000000000000004".into(),
                target_id: "worker-tab".into(),
                document_id: "worker-document".into(),
                permission: crate::runtime::browser_controller_permission::BrowserPermissionName::Geolocation,
                setting: crate::runtime::browser_controller_permission::BrowserPermissionSetting::Denied,
            },
            crate::transport::room_browser_controller::RoomBrowserControllerCommand::PollEvents {
                browser_generation: 1,
                cursor: 0,
                limit: 10,
            },
            crate::transport::room_browser_controller::RoomBrowserControllerCommand::Navigate {
                execution_id: "00000000000000000000000000000013".into(),
                target_id: "worker-tab".into(),
                document_id: "worker-document".into(),
                url: crate::runtime::browser_controller_compatibility::BrowserNavigationUrl::new(
                    "https://denied.worker.test/",
                )
                .unwrap(),
            },
            crate::transport::room_browser_controller::RoomBrowserControllerCommand::Wait {
                target_id: "worker-tab".into(),
                document_id: "worker-document".into(),
                wait: crate::runtime::browser_controller_compatibility::BrowserCompatibilityWait::Idle,
                timeout_ms: 500,
            },
        ] {
            let denied = crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
            &sender,
            chariox_relay::protocol::ClientTarget { daemon_id: Some("environment-worker".into()), daemon_alias: None },
            crate::transport::relay_peer::RelayPeerRequest::RoomBrowserController {
                session_id: if mismatch == "room" { fixture.rooms[1].clone() } else { room.clone() },
                slice_id: if mismatch == "slice" { "other-slice".into() } else { slice.id.clone() },
                command,
            },
            Duration::from_secs(3),
        ).await.expect_err("mismatched caller must not stop the owner's browser");
            assert!(
                denied
                    .to_string()
                    .contains("browser_controller_scope_denied"),
                "{mismatch}: {denied}"
            );
        }
    }
    let again = dispatch_json(&fixture.home, request).await.unwrap();
    assert_eq!(
        again["RoomEnvironmentUpdated"]["environment"]["environment_id"],
        environment["environment_id"]
    );
    let again_tabs = again["RoomEnvironmentUpdated"]["environment"]["tabs"]
        .as_array()
        .expect("reconciled Room tabs");
    let again_tab = again_tabs.first().expect("reconciled Room tab");
    let initial_tab = environment["tabs"]
        .as_array()
        .and_then(|tabs| tabs.first())
        .expect("initial Room tab");
    assert_eq!(again_tab["tab_id"], initial_tab["tab_id"]);
    assert_eq!(again_tab["title"], initial_tab["title"]);
    assert_eq!(again_tab["focused"], initial_tab["focused"]);
    assert!(
        again_tab["document_revision"].as_u64()
            >= initial_tab["document_revision"].as_u64(),
        "Room tab revisions must not move backwards: initial={initial_tab:?}, reconciled={again_tab:?}"
    );
    assert!(
        !again_tabs.iter().any(|tab| {
            tab["url"] == "https://popup.worker.test/" && tab["title"] == "Worker popup"
        }),
        "a closed popup must not be resurrected by a stale target-created event"
    );
    super::controller_compatibility::check(fixture, &token).await;
    super::controller_worker_mcp::check(fixture, worker_mcp_placement).await;
    super::controller_integrations::check_cancellation_without_tabs(fixture, &token).await;
    dispatch_json(
        &fixture.home,
        json!({"StopRoomEnvironment":{"session_id":room}}),
    )
    .await
    .expect("stop controller through the home Room");
    assert_eq!(
        dispatch_json(&fixture.home, get(room)).await.unwrap(),
        original
    );
    dispatch_json(&fixture.home, json!({"DeleteSession":{"session_ref":room}}))
        .await
        .unwrap();
    assert!(
        dispatch_json(&fixture.home, bind(&fixture.rooms[1], "desktop"))
            .await
            .is_err(),
        "deleting a Room must not release its physical browser profile to another Room"
    );
}
