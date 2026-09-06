use super::*;
use futures_util::FutureExt;

pub(super) async fn check(fixture: &LiveWorker, token: &str, agent: &str, tab: &str) {
    for (tool, args, kind, counter) in [
        (
            "slice_browser_downloads",
            json!({}),
            "download_configure",
            "downloadConfigureCount",
        ),
        (
            "slice_browser_permission",
            json!({"permission":"geolocation","setting":"denied"}),
            "permission_set",
            "permissionCount",
        ),
    ] {
        let root = &fixture._worker_state.root;
        let hold = root.join("hold-configuration");
        let pending = root.join("configuration-pending");
        let observed = root.join("configuration-cancel-observed");
        let before = physical_count(fixture, counter);
        std::fs::write(&hold, "hold configuration before dispatch").unwrap();
        let home = Arc::clone(&fixture.home);
        let owned_token = token.to_string();
        let request_args = args.clone();
        let operation = tokio::spawn(async move {
            home.runtime_state
                .dispatch_authenticated_runtime_tool_call(&owned_token, tool, request_args)
                .await
        });
        let assertions = std::panic::AssertUnwindSafe(async {
            wait_file(&pending).await;
            dispatch_json(
                &fixture.home,
                json!({"RequestRoomEnvironmentInputTakeover":{
                    "session_id":fixture.rooms[0],"target":{"kind":"desktop"}
                }}),
            )
            .await
            .unwrap();
            wait_file(&observed).await;
            assert_eq!(physical_count(fixture, counter), before);
        })
        .catch_unwind()
        .await;
        std::fs::remove_file(&hold).unwrap();
        let result = timeout(Duration::from_secs(5), operation)
            .await
            .unwrap()
            .unwrap();
        if let Err(panic) = assertions {
            std::panic::resume_unwind(panic);
        }
        assert!(result
            .expect_err("human takeover cancels pending configuration")
            .to_string()
            .to_lowercase()
            .contains("cancel"));
        assert_eq!(physical_count(fixture, counter), before);
        let environment = fixture
            .home
            .runtime_state
            .room_environment_snapshot(&fixture.rooms[0])
            .unwrap();
        let action = environment
            .actions
            .iter()
            .filter(|action| action.kind == kind)
            .max_by_key(|action| action.sequence)
            .unwrap();
        assert_eq!(
            action.state,
            crate::session::EnvironmentActionState::Cancelled
        );
        assert_eq!(
            action.actor_id,
            crate::session::agent_environment_actor_id(agent)
        );
        let state = serde_json::to_value(environment).unwrap();
        assert!(state["input_ownership"]
            .as_array()
            .unwrap()
            .iter()
            .any(|owner| owner["target"] == json!({"kind":"desktop"})
                && owner["actor_id"].as_str().unwrap().starts_with("user:")));
        dispatch_json(
            &fixture.home,
            json!({"ReleaseRoomEnvironmentInput":{
                "session_id":fixture.rooms[0],"target":{"kind":"desktop"}
            }}),
        )
        .await
        .unwrap();
        assert!(
            fixture
                .home
                .runtime_state
                .dispatch_authenticated_runtime_tool_call(token, tool, args)
                .await
                .unwrap()
                .ok
        );
        assert_eq!(physical_count(fixture, counter), before + 1);
        std::fs::remove_file(pending).unwrap();
        std::fs::remove_file(observed).unwrap();

        let relay = fixture.worker.app.lock().await.relay_client_state();
        for forget_receipt in [false, true] {
            let before = physical_count(fixture, counter);
            if forget_receipt {
                relay
                    .write()
                    .await
                    .test_lose_next_peer_response_payload_and_forget_action_receipts();
            } else {
                relay.write().await.test_lose_next_peer_response_payload();
            }
            // Inject after MCP preflight, at the admitted mutation interface.
            let result = configure(fixture, agent, tab, kind).await;
            if forget_receipt {
                let error = result.expect_err("missing receipt cannot replay a configuration");
                assert!(
                    error.to_string().contains("receipt") || error.to_string().contains("proof"),
                    "{error}"
                );
            } else {
                result.expect("completed configuration recovers its receipt");
            }
            assert_eq!(physical_count(fixture, counter), before + 1);
            let environment = fixture
                .home
                .runtime_state
                .room_environment_snapshot(&fixture.rooms[0])
                .unwrap();
            let action = environment
                .actions
                .iter()
                .filter(|action| action.kind == kind)
                .max_by_key(|action| action.sequence)
                .unwrap();
            assert_eq!(
                action.state,
                if forget_receipt {
                    crate::session::EnvironmentActionState::Failed
                } else {
                    crate::session::EnvironmentActionState::Completed
                }
            );
            configure(fixture, agent, tab, kind).await.unwrap();
            assert_eq!(physical_count(fixture, counter), before + 2);
        }
    }
}

async fn configure(
    fixture: &LiveWorker,
    agent: &str,
    tab: &str,
    kind: &str,
) -> Result<(), crate::error::DaemonError> {
    let runtime = &fixture.home.runtime_state;
    let room = &fixture.rooms[0];
    if kind == "permission_set" {
        runtime
            .set_browser_permission_as_agent(
                room,
                agent,
                tab,
                crate::runtime::browser_controller_permission::BrowserPermissionName::Geolocation,
                crate::runtime::browser_controller_permission::BrowserPermissionSetting::Denied,
            )
            .await
            .map(|_| ())
    } else {
        runtime
            .configure_browser_downloads_as_agent(room, agent, tab)
            .await
            .map(|_| ())
    }
}

fn physical_count(fixture: &LiveWorker, counter: &str) -> u64 {
    let state: Value = serde_json::from_slice(
        &std::fs::read(fixture._worker_state.root.join("chromium-state.json")).unwrap(),
    )
    .unwrap();
    state[counter].as_u64().unwrap_or(0)
}

async fn wait_file(path: &std::path::Path) {
    timeout(Duration::from_secs(2), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("controller reaches configuration fault-injection point");
}
