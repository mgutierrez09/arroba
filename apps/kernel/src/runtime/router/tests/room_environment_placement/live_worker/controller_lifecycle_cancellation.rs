use super::*;
use crate::runtime::browser_controller_action::BrowserDialogAction;
use crate::runtime::browser_controller_history::BrowserHistoryAction;
use crate::runtime::browser_controller_tab::BrowserTabAction;
use crate::session::EnvironmentActionState;
use futures_util::FutureExt;

pub(super) async fn check(fixture: &LiveWorker, token: &str) {
    let runtime = &fixture.home.runtime_state;
    let room = &fixture.rooms[0];
    let status = runtime
        .dispatch_authenticated_runtime_tool_call(token, "slice_browser_status", json!({}))
        .await
        .unwrap();
    let tab = status.payload["tab_id"].as_str().unwrap();
    let agent = status.payload["agent_id"].as_str().unwrap();
    let takeover_target = json!({"kind":"browser_tab","id":tab});
    for (tool, args, kind, counter) in [
        (
            "slice_browser_tab",
            json!({"tab_id":tab,"action":"activate"}),
            "browser_tab_activate",
            "activateCount",
        ),
        (
            "slice_browser_history",
            json!({"tab_id":tab,"action":"reload"}),
            "browser_history_reload",
            "reloadCount",
        ),
        (
            "slice_open_url",
            json!({"url":"https://worker.test/lifecycle"}),
            "navigate",
            "navigateCount",
        ),
        (
            "slice_browser_dialog",
            json!({"tab_id":tab,"action":"dismiss"}),
            "dialog",
            "dialogCount",
        ),
    ] {
        let root = &fixture._worker_state.root;
        let hold = root.join("hold-lifecycle");
        let pending = root.join("lifecycle-pending");
        let observed = root.join("lifecycle-cancel-observed");
        let before = physical_count(fixture, counter);
        std::fs::write(&hold, "hold before physical dispatch").unwrap();
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
                    "session_id":room,"target":takeover_target
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
        let error = result.expect_err("takeover cancels the pending browser operation");
        assert!(
            error.to_string().to_lowercase().contains("cancel"),
            "{error}"
        );
        assert_eq!(physical_count(fixture, counter), before);
        assert_terminal(fixture, agent, kind, EnvironmentActionState::Cancelled);
        let state = serde_json::to_value(runtime.room_environment_snapshot(room).unwrap()).unwrap();
        assert!(state["input_ownership"]
            .as_array()
            .unwrap()
            .iter()
            .any(|owner| owner["target"] == takeover_target
                && owner["actor_id"].as_str().unwrap().starts_with("user:")));
        dispatch_json(
            &fixture.home,
            json!({"ReleaseRoomEnvironmentInput":{
                "session_id":room,"target":takeover_target
            }}),
        )
        .await
        .unwrap();
        assert!(
            runtime
                .dispatch_authenticated_runtime_tool_call(token, tool, args)
                .await
                .unwrap()
                .ok
        );
        assert_eq!(physical_count(fixture, counter), before + 1);
        std::fs::remove_file(pending).unwrap();
        std::fs::remove_file(observed).unwrap();

        let relay = fixture.worker.app.lock().await.relay_client_state();
        for forget in [false, true] {
            let before = physical_count(fixture, counter);
            if forget {
                relay
                    .write()
                    .await
                    .test_lose_next_peer_response_payload_and_forget_action_receipts();
            } else {
                relay.write().await.test_lose_next_peer_response_payload();
            }
            let result = execute_admitted(fixture, agent, tab, kind).await;
            if forget {
                let error = result.expect_err("missing proof cannot repeat a physical action");
                assert!(
                    error.to_string().contains("receipt") || error.to_string().contains("proof"),
                    "{error}"
                );
            } else {
                result.expect("lost reply recovers the original receipt");
            }
            assert_eq!(physical_count(fixture, counter), before + 1);
            assert_terminal(
                fixture,
                agent,
                kind,
                if forget {
                    EnvironmentActionState::Failed
                } else {
                    EnvironmentActionState::Completed
                },
            );
            // Refresh observation after an unproven navigation, never replay it.
            runtime
                .reconcile_browser_controller_environment(room)
                .await
                .unwrap();
            execute_admitted(fixture, agent, tab, kind).await.unwrap();
            assert_eq!(physical_count(fixture, counter), before + 2);
        }
    }
    check_human_cancellation(fixture, tab).await;
}

async fn check_human_cancellation(fixture: &LiveWorker, tab: &str) {
    let room = &fixture.rooms[0];
    let root = &fixture._worker_state.root;
    let target = json!({"kind":"browser_tab","id":tab});
    dispatch_json(
        &fixture.home,
        json!({"RequestRoomEnvironmentInputTakeover":{
            "session_id":room,"target":target
        }}),
    )
    .await
    .unwrap();
    let generation = fixture
        .home
        .runtime_state
        .room_environment_snapshot(room)
        .unwrap()
        .runtime_generation;
    let request = json!({"SubmitRoomEnvironmentBrowserAction":{
        "session_id":room,"runtime_generation":generation,"idempotency_key":"cancel-human-reload",
        "action":{"kind":"history","tab_id":tab,"action":"reload"}
    }});
    let before = physical_count(fixture, "reloadCount");
    let hold = root.join("hold-lifecycle");
    std::fs::write(&hold, "human reload before dispatch").unwrap();
    let home = Arc::clone(&fixture.home);
    let submitted = request.clone();
    let operation = tokio::spawn(async move { dispatch_json(&home, submitted).await });
    let assertions = std::panic::AssertUnwindSafe(async {
        wait_file(&root.join("lifecycle-pending")).await;
        let environment = fixture
            .home
            .runtime_state
            .room_environment_snapshot(room)
            .unwrap();
        let action = environment
            .actions
            .iter()
            .filter(|a| a.kind == "browser_history_reload")
            .max_by_key(|a| a.sequence)
            .unwrap();
        assert!(action.actor_id.starts_with("user:"));
        dispatch_json(
            &fixture.home,
            json!({"CancelRoomEnvironmentAction":{
                "session_id":room,"action_id":action.action_id
            }}),
        )
        .await
        .unwrap();
        wait_file(&root.join("lifecycle-cancel-observed")).await;
    })
    .catch_unwind()
    .await;
    std::fs::remove_file(hold).unwrap();
    let result = timeout(Duration::from_secs(5), operation)
        .await
        .unwrap()
        .unwrap();
    if let Err(panic) = assertions {
        std::panic::resume_unwind(panic);
    }
    assert!(result
        .expect_err("human may cancel their pending reload")
        .to_string()
        .to_lowercase()
        .contains("cancel"));
    assert_eq!(physical_count(fixture, "reloadCount"), before);
    let replay = dispatch_json(&fixture.home, request).await.unwrap();
    let submitted = &replay["RoomEnvironmentActionSubmitted"];
    let action = submitted["environment"]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["action_id"] == submitted["action_id"])
        .unwrap();
    assert_eq!(action["state"], "cancelled");
    assert_eq!(
        physical_count(fixture, "reloadCount"),
        before,
        "idempotent replay cannot retry cancelled input"
    );
    dispatch_json(
        &fixture.home,
        json!({"ReleaseRoomEnvironmentInput":{
            "session_id":room,"target":target
        }}),
    )
    .await
    .unwrap();
    std::fs::remove_file(root.join("lifecycle-pending")).unwrap();
    std::fs::remove_file(root.join("lifecycle-cancel-observed")).unwrap();
}

async fn execute_admitted(
    fixture: &LiveWorker,
    agent: &str,
    tab: &str,
    kind: &str,
) -> Result<(), crate::error::DaemonError> {
    let runtime = &fixture.home.runtime_state;
    let room = &fixture.rooms[0];
    let revision = runtime
        .room_environment_controller_tab_binding(room, tab)
        .unwrap()
        .document_revision;
    let execution = format!("{:032x}", rand::random::<u128>());
    // Fault injection begins after read-only preflight, at the existing shared
    // admission interface; the worker still runs the production stdio controller.
    if kind == "browser_tab_activate" {
        runtime
            .execute_browser_tab_mutation_as_agent(
                room,
                agent,
                tab,
                revision,
                kind,
                Some(&execution),
                runtime.manage_browser_environment_tab(
                    room,
                    &execution,
                    tab,
                    BrowserTabAction::Activate,
                ),
            )
            .await
            .map(|_| ())
    } else {
        runtime
            .execute_browser_mutation_as_agent(
                room,
                agent,
                tab,
                revision,
                kind,
                Some(&execution),
                async {
                    match kind {
                        "browser_history_reload" => runtime
                            .navigate_browser_environment_history(
                                room,
                                &execution,
                                tab,
                                BrowserHistoryAction::Reload,
                            )
                            .await
                            .map(|_| ()),
                        "navigate" => runtime
                            .navigate_browser_environment_compatibility(
                                room,
                                &execution,
                                tab,
                                "https://worker.test/lifecycle-receipt",
                            )
                            .await
                            .map(|_| ()),
                        "dialog" => runtime
                            .handle_browser_environment_dialog(
                                room,
                                &execution,
                                tab,
                                BrowserDialogAction::Dismiss,
                            )
                            .await
                            .map(|_| ()),
                        _ => unreachable!(),
                    }
                },
            )
            .await
            .map(|_| ())
    }
}

fn assert_terminal(
    fixture: &LiveWorker,
    agent: &str,
    kind: &str,
    expected: EnvironmentActionState,
) {
    let environment = fixture
        .home
        .runtime_state
        .room_environment_snapshot(&fixture.rooms[0])
        .unwrap();
    let action = environment
        .actions
        .iter()
        .filter(|a| a.kind == kind)
        .max_by_key(|a| a.sequence)
        .unwrap();
    assert_eq!(action.state, expected, "{kind}");
    assert_eq!(
        action.actor_id,
        crate::session::agent_environment_actor_id(agent)
    );
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
    .expect("controller reaches the lifecycle cancellation boundary");
}
