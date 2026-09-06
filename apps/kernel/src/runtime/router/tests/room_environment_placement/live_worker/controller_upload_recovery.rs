use super::*;

pub(super) async fn check_response_loss(
    fixture: &LiveWorker,
    agent_id: &str,
    field: &str,
    file: &std::path::Path,
) {
    let relay = fixture.worker.app.lock().await.relay_client_state();
    // Inject at the admitted mutation interface so the MCP tool's preliminary
    // read-only status RPCs cannot consume the fault before upload executes.
    for forget_receipt in [false, true] {
        let before = physical_count(fixture);
        if forget_receipt {
            relay
                .write()
                .await
                .test_lose_next_peer_response_payload_and_forget_action_receipts();
        } else {
            relay.write().await.test_lose_next_peer_response_payload();
        }
        let result = fixture
            .home
            .runtime_state
            .upload_browser_environment_files_as_agent(
                &fixture.rooms[0],
                agent_id,
                field,
                vec![file.to_path_buf()],
            )
            .await;
        if forget_receipt {
            let error = result.expect_err("lost upload completion proof must fail closed");
            assert!(
                error.to_string().contains("receipt") || error.to_string().contains("proof"),
                "{error}"
            );
        } else {
            assert_eq!(
                result
                    .expect("recover completed upload receipt")
                    .value
                    .file_count,
                1
            );
        }
        assert_eq!(
            physical_count(fixture),
            before + 1,
            "lost upload reply must never resend files"
        );
        let environment = state(fixture).await;
        let action = environment["actions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|action| action["kind"] == "upload")
            .max_by_key(|action| action["sequence"].as_u64().unwrap())
            .unwrap();
        assert_eq!(
            action["state"],
            if forget_receipt {
                "failed"
            } else {
                "completed"
            }
        );
        assert!(!action
            .to_string()
            .contains(file.file_name().unwrap().to_str().unwrap()));

        let retry = fixture
            .home
            .runtime_state
            .upload_browser_environment_files_as_agent(
                &fixture.rooms[0],
                agent_id,
                field,
                vec![file.to_path_buf()],
            )
            .await
            .unwrap();
        assert_eq!(
            retry.value.file_count, 1,
            "a fresh upload remains possible after recovery"
        );
        assert_eq!(physical_count(fixture), before + 2);
    }
}

pub(super) async fn check_restart(fixture: &LiveWorker, token: &str) {
    let (field, tab_id, agent_id) = upload_reference(fixture, token).await;
    let file = fixture._worker_state.root.join("upload-restart.txt");
    std::fs::write(&file, b"restart upload").unwrap();
    let before = physical_count(fixture);
    let controller_pid = std::fs::read_to_string(fixture._worker_state.root.join("controller.pid"))
        .unwrap()
        .trim()
        .parse::<i32>()
        .unwrap();
    assert_eq!(unsafe { libc::kill(controller_pid, libc::SIGKILL) }, 0);
    // The supervisor reaps its child on the next request. kill(pid, 0) would
    // continue reporting this unreaped child as present, so do not wait on it.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let error = fixture
        .home
        .runtime_state
        .upload_browser_environment_files_as_agent(
            &fixture.rooms[0],
            &agent_id,
            &field,
            vec![file.clone()],
        )
        .await
        .expect_err("pre-restart upload must not execute");
    assert!(
        error
            .to_string()
            .contains("browser controller restarted before the operation"),
        "{error}"
    );
    assert_eq!(physical_count(fixture), before);
    let environment = state(fixture).await;
    assert!(
        !environment["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action["kind"] == "upload"
                && ["running", "queued"].contains(&action["state"].as_str().unwrap())),
        "restart must settle admitted uploads"
    );
    let (fresh, fresh_tab_id, _) = upload_reference(fixture, token).await;
    assert_eq!(
        fresh_tab_id, tab_id,
        "controller restart preserves the Room tab"
    );
    assert_ne!(fresh, field, "pre-restart reference must be replaced");
    let retry = fixture
        .home
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_upload",
            json!({"field_id":fresh,"files":[file]}),
        )
        .await
        .unwrap();
    assert!(retry.ok);
    assert_eq!(physical_count(fixture), before + 1);
    std::fs::remove_file(file).unwrap();
}

async fn upload_reference(fixture: &LiveWorker, token: &str) -> (String, String, String) {
    let status = fixture
        .home
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(token, "slice_browser_status", json!({}))
        .await
        .unwrap();
    let tab_id = status.payload["tab_id"].as_str().unwrap().to_string();
    let snapshot = fixture
        .home
        .runtime_state
        .capture_browser_environment_snapshot(&fixture.rooms[0], &tab_id)
        .await
        .unwrap();
    let field = snapshot
        .dom_nodes
        .iter()
        .find(|node| {
            node.node_name == "INPUT"
                && node
                    .attributes
                    .get("type")
                    .is_some_and(|kind| kind == "file")
        })
        .expect("observed file input")
        .element_ref
        .clone();
    (
        field,
        tab_id,
        status.payload["agent_id"].as_str().unwrap().to_string(),
    )
}

fn physical_count(fixture: &LiveWorker) -> u64 {
    let state: Value = serde_json::from_slice(
        &std::fs::read(fixture._worker_state.root.join("chromium-state.json")).unwrap(),
    )
    .unwrap();
    state["uploadCount"].as_u64().unwrap_or(0)
}

async fn state(fixture: &LiveWorker) -> Value {
    dispatch_json(
        &fixture.home,
        json!({"GetRoomEnvironmentState":{
            "session_id":fixture.rooms[0]
        }}),
    )
    .await
    .unwrap()["RoomEnvironmentState"]["environment"]
        .clone()
}
