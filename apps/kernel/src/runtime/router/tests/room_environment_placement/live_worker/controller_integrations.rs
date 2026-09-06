use super::*;

pub(super) async fn check(fixture: &LiveWorker, token: &str, status: &Value) {
    let runtime = &fixture.home.runtime_state;
    let room = &fixture.rooms[0];
    let tab_id = status["tab_id"].as_str().expect("focused Room tab");

    let snapshot = runtime
        .capture_browser_environment_snapshot(room, tab_id)
        .await
        .expect("nested browser structure crosses the bound worker relay");
    let frame = snapshot
        .dom_documents
        .iter()
        .find(|document| document.owner_element_ref.is_some())
        .expect("nested frame document");
    let frame_owner = frame
        .owner_element_ref
        .as_deref()
        .expect("nested frame owner reference");
    assert!(snapshot
        .dom_nodes
        .iter()
        .any(|node| node.element_ref == frame_owner));
    let frame_button = snapshot
        .dom_nodes
        .iter()
        .find(|node| {
            node.node_name == "BUTTON"
                && node
                    .bounds
                    .is_some_and(|bounds| bounds.x == 20.0 && bounds.y == 110.0)
        })
        .expect("button inside nested frame");
    let shadow = snapshot.shadow_roots.first().expect("open shadow root");
    let shadow_button = snapshot
        .dom_nodes
        .iter()
        .find(|node| {
            node.parent_ref.as_deref() == Some(shadow.element_ref.as_str())
                && node.node_name == "BUTTON"
        })
        .expect("button inside shadow root");
    let upload_field = snapshot
        .dom_nodes
        .iter()
        .find(|node| {
            node.node_name == "INPUT"
                && node
                    .attributes
                    .get("type")
                    .is_some_and(|kind| kind == "file")
        })
        .expect("worker upload field")
        .element_ref
        .clone();

    let agent_id = status["agent_id"].as_str().expect("Room agent id");
    assert!(runtime
        .runtime_tool_specs_for_auth_token(token)
        .iter()
        .any(|spec| spec.name == "slice_browser_dialog"));
    for name in [
        "slice_browser_tab",
        "slice_browser_history",
        "slice_browser_events",
        "slice_browser_downloads",
        "slice_browser_upload",
        "slice_browser_permission",
    ] {
        assert!(
            runtime
                .runtime_tool_specs_for_auth_token(token)
                .iter()
                .any(|spec| spec.name == name),
            "bound Room runtime MCP omitted {name}"
        );
    }
    let clicked = runtime
        .perform_browser_environment_locator_action_as_agent(
            room,
            agent_id,
            &shadow_button.element_ref,
            crate::runtime::browser_controller_action::BrowserLocatorAction::Click,
            crate::runtime::browser_controller_action::MAX_BROWSER_ACTION_TIMEOUT_MS,
        )
        .await
        .expect("shadow-root action crosses the bound worker relay");
    assert_eq!(clicked.value.element_ref, shadow_button.element_ref);
    let frame_clicked = runtime
        .perform_browser_environment_locator_action_as_agent(
            room,
            agent_id,
            &frame_button.element_ref,
            crate::runtime::browser_controller_action::BrowserLocatorAction::Click,
            crate::runtime::browser_controller_action::MAX_BROWSER_ACTION_TIMEOUT_MS,
        )
        .await
        .expect("nested-frame action crosses the bound worker relay");
    assert_eq!(frame_clicked.value.element_ref, frame_button.element_ref);

    let popup_status = runtime
        .dispatch_authenticated_runtime_tool_call(token, "slice_browser_status", json!({}))
        .await
        .expect("popup is reconciled into the stable Room tab registry");
    let popup_tab_id = popup_status.payload["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tab| tab["title"] == "Worker popup")
        .and_then(|tab| tab["tab_id"].as_str())
        .expect("stable popup tab id")
        .to_string();
    super::controller_configuration::check(fixture, token, agent_id, tab_id, &popup_tab_id).await;
    let activated = runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_tab",
            json!({"tab_id":popup_tab_id.as_str(),"action":"activate"}),
        )
        .await
        .expect("public tab tool activates the popup through the bound worker");
    assert!(activated.ok, "{:?}", activated.payload);
    assert_eq!(activated.payload["focused_tab_id"], popup_tab_id);
    let closed = runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_tab",
            json!({"tab_id":popup_tab_id.as_str(),"action":"close"}),
        )
        .await
        .expect("public tab tool closes the popup through the bound worker");
    assert!(closed.ok, "{:?}", closed.payload);
    assert!(closed.payload["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .all(|tab| tab["tab_id"] != popup_tab_id));

    runtime
        .perform_browser_environment_locator_action_as_agent(
            room,
            agent_id,
            &shadow_button.element_ref,
            crate::runtime::browser_controller_action::BrowserLocatorAction::Click,
            crate::runtime::browser_controller_action::MAX_BROWSER_ACTION_TIMEOUT_MS,
        )
        .await
        .expect("agent reopens the popup for authenticated human control");
    let human_popup_status = runtime
        .dispatch_authenticated_runtime_tool_call(token, "slice_browser_status", json!({}))
        .await
        .expect("reopened popup is reconciled before human control");
    let human_popup_tab_id = human_popup_status.payload["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tab| tab["title"] == "Worker popup")
        .and_then(|tab| tab["tab_id"].as_str())
        .expect("stable reopened popup tab id")
        .to_string();
    let popup_target = json!({"kind":"browser_tab","id":human_popup_tab_id});
    let activate_popup = json!({
        "SubmitRoomEnvironmentBrowserAction": {
            "session_id": room,
            "runtime_generation": human_popup_status.payload["runtime_generation"],
            "idempotency_key": "human-popup-activate-1",
            "action": {
                "kind": "tab",
                "tab_id": human_popup_tab_id,
                "action": "activate"
            }
        }
    });
    let denied = dispatch_json(&fixture.home, activate_popup.clone())
        .await
        .expect_err("human tab activation requires explicit tab takeover");
    assert!(
        denied
            .to_string()
            .contains("environment_input_takeover_required"),
        "{denied}"
    );
    dispatch_json(
        &fixture.home,
        json!({"RequestRoomEnvironmentInputTakeover":{
            "session_id":room,"target":popup_target
        }}),
    )
    .await
    .expect("human takes over the reopened popup");
    let activated = dispatch_json(&fixture.home, activate_popup)
        .await
        .expect("authenticated human activates the popup through the bound worker");
    let activated = &activated["RoomEnvironmentActionSubmitted"];
    assert_eq!(
        activated["environment"]["focused_tab_id"],
        human_popup_tab_id
    );
    let activated_action_id = activated["action_id"]
        .as_str()
        .expect("human activation action ID");
    let activated_action = activated["environment"]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["action_id"] == activated_action_id)
        .expect("attributed human activation action");
    assert_eq!(activated_action["kind"], "browser_tab_activate");
    assert_eq!(
        activated_action["targets"],
        json!([
            {"kind":"desktop"},
            {"kind":"browser_tab","id":human_popup_tab_id}
        ])
    );

    let close_popup = json!({
        "SubmitRoomEnvironmentBrowserAction": {
            "session_id": room,
            "runtime_generation": activated["environment"]["runtime_generation"],
            "idempotency_key": "human-popup-close-1",
            "action": {
                "kind": "tab",
                "tab_id": human_popup_tab_id,
                "action": "close"
            }
        }
    });
    let closed = dispatch_json(&fixture.home, close_popup.clone())
        .await
        .expect("authenticated human closes the popup through the bound worker");
    let closed = &closed["RoomEnvironmentActionSubmitted"];
    let close_action_id = closed["action_id"].clone();
    assert!(closed["environment"]["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .all(|tab| tab["tab_id"] != human_popup_tab_id));
    assert!(closed["environment"]["input_ownership"]
        .as_array()
        .unwrap()
        .iter()
        .all(|ownership| ownership["target"] != popup_target));
    let replayed_close = dispatch_json(&fixture.home, close_popup)
        .await
        .expect("exact close replay succeeds after tab removal and ownership cleanup");
    assert_eq!(
        replayed_close["RoomEnvironmentActionSubmitted"]["action_id"],
        close_action_id
    );

    let dialog = runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_dialog",
            json!({"action":"accept","prompt_text":"approved by home"}),
        )
        .await
        .expect("public dialog tool reaches the bound worker controller");
    assert!(dialog.ok, "{:?}", dialog.payload);
    assert_eq!(dialog.payload["browser"]["action"], "accept");

    let downloads = runtime
        .dispatch_authenticated_runtime_tool_call(token, "slice_browser_downloads", json!({}))
        .await
        .expect("runtime MCP download configuration reaches the bound worker controller");
    assert!(downloads.ok, "{:?}", downloads.payload);
    assert_eq!(downloads.payload["enabled"], true);
    assert_eq!(downloads.payload["tab_id"], tab_id);

    let cancel_args = json!({"cancel": {
        "browser_generation": status["browser_generation"], "guid": "worker-active-download"
    }});
    for (args, expected) in [
        (
            json!({"cancel":{"browser_generation":0,"guid":"worker-active-download"}}),
            "generation",
        ),
        (
            json!({"cancel":{"browser_generation":status["browser_generation"],"guid":"../download"}}),
            "GUID",
        ),
        (
            json!({"cancel":{"browser_generation":status["browser_generation"].as_u64().unwrap() + 1,"guid":"worker-active-download"}}),
            "stale_browser_generation",
        ),
        (
            json!({"cancel":{"browser_generation":status["browser_generation"],"guid":"unobserved-download"}}),
            "browser_download_not_active",
        ),
    ] {
        let denied = runtime
            .dispatch_authenticated_runtime_tool_call(token, "slice_browser_downloads", args)
            .await
            .expect_err("invalid or unobserved cancellation must fail closed");
        assert!(denied.to_string().contains(expected), "{denied}");
    }
    let desktop = json!({"kind":"desktop"});
    dispatch_json(
        &fixture.home,
        json!({"RequestRoomEnvironmentInputTakeover":{
            "session_id":room,"target":desktop
        }}),
    )
    .await
    .expect("human owns the desktop before cancellation");
    let denied = runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_downloads",
            cancel_args.clone(),
        )
        .await
        .expect_err("agent download cancellation must respect human ownership");
    assert!(denied.to_string().contains("belongs to"), "{denied}");
    dispatch_json(
        &fixture.home,
        json!({"ReleaseRoomEnvironmentInput":{
            "session_id":room,"target":desktop
        }}),
    )
    .await
    .expect("release human ownership before retrying cancellation");
    let canceled = runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_downloads",
            cancel_args.clone(),
        )
        .await
        .expect("download cancellation crosses the authenticated worker relay");
    assert!(canceled.ok, "{:?}", canceled.payload);
    assert_eq!(canceled.payload["cancellation_requested"], true);
    assert_eq!(canceled.payload["guid"], "worker-active-download");
    assert_eq!(
        canceled.payload["actor_id"],
        crate::session::agent_environment_actor_id(agent_id)
    );
    assert!(canceled.payload["action_id"]
        .as_str()
        .is_some_and(|id| !id.is_empty()));
    let state = dispatch_json(
        &fixture.home,
        json!({"GetRoomEnvironmentState":{"session_id":room}}),
    )
    .await
    .expect("read the shared action history after cancellation");
    let environment = &state["RoomEnvironmentState"]["environment"];
    let action = environment["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["action_id"] == canceled.payload["action_id"])
        .expect("cancellation must be attributed in the Room action history");
    assert_eq!(action["kind"], "download_cancel");
    assert_eq!(action["state"], "completed");
    assert_eq!(action["actor_id"], canceled.payload["actor_id"]);
    assert_eq!(action["mode"], "browser");
    assert_eq!(action["targets"], json!([{"kind":"desktop"}]));
    assert_eq!(
        canceled.payload["environment_id"],
        environment["environment_id"]
    );
    assert_eq!(
        canceled.payload["runtime_generation"],
        action["runtime_generation"]
    );
    let repeated = runtime
        .dispatch_authenticated_runtime_tool_call(token, "slice_browser_downloads", cancel_args)
        .await
        .expect_err("a terminal download cannot be canceled again");
    assert!(
        repeated.to_string().contains("browser_download_not_active"),
        "{repeated}"
    );

    let upload_path = fixture._worker_state.root.join("relay-upload.txt");
    std::fs::write(&upload_path, b"relay upload").expect("write bounded upload fixture");
    let upload_target =
        serde_json::to_value(crate::session::InputTarget::BrowserTab(tab_id.into()))
            .expect("serialize upload target");
    dispatch_json(
        &fixture.home,
        json!({"RequestRoomEnvironmentInputTakeover":{
            "session_id":room,"target":upload_target
        }}),
    )
    .await
    .expect("human reserves the upload tab");
    let denied_upload = runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_upload",
            json!({"field_id": upload_field, "files": [upload_path.clone()]}),
        )
        .await
        .expect_err("agent upload must respect human tab ownership");
    assert!(
        denied_upload.to_string().contains("belongs to"),
        "{denied_upload}"
    );
    dispatch_json(
        &fixture.home,
        json!({"ReleaseRoomEnvironmentInput":{
            "session_id":room,"target":upload_target
        }}),
    )
    .await
    .expect("release upload tab before retry");
    let upload = runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_upload",
            json!({"field_id": upload_field, "files": [upload_path.clone()]}),
        )
        .await
        .expect("runtime MCP file upload reaches the bound worker controller");
    assert!(upload.ok, "{:?}", upload.payload);
    assert_eq!(upload.payload["file_count"], 1);
    assert_eq!(upload.payload["total_bytes"], 12);
    let uploaded_state = dispatch_json(
        &fixture.home,
        json!({"GetRoomEnvironmentState":{"session_id":room}}),
    )
    .await
    .expect("upload action history");
    let uploads = uploaded_state["RoomEnvironmentState"]["environment"]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|action| action["kind"] == "upload" && action["state"] == "completed")
        .collect::<Vec<_>>();
    assert_eq!(
        uploads.len(),
        1,
        "exactly one upload may complete after release"
    );
    assert_eq!(
        uploads[0]["actor_id"],
        crate::session::agent_environment_actor_id(agent_id)
    );
    assert_eq!(uploads[0]["targets"], json!([upload_target]));
    assert!(!uploads[0].to_string().contains("relay-upload.txt"));
    assert!(
        !upload.payload.to_string().contains("relay-upload.txt"),
        "upload paths must not return through runtime MCP"
    );

    super::controller_upload_cancellation::check(
        fixture,
        token,
        &upload_field,
        &upload_path,
        &upload_target,
    )
    .await;
    super::controller_upload_recovery::check_response_loss(
        fixture,
        agent_id,
        &upload_field,
        &upload_path,
    )
    .await;

    let permission = runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_permission",
            json!({"permission": "geolocation", "setting": "denied"}),
        )
        .await
        .expect("runtime MCP permission decision reaches the bound worker controller");
    assert!(permission.ok, "{:?}", permission.payload);
    assert_eq!(permission.payload["permission"], "geolocation");
    assert_eq!(permission.payload["setting"], "denied");

    let physical = std::fs::read_to_string(fixture._worker_state.root.join("chromium-state.json"))
        .expect("worker browser state");
    let physical: Value = serde_json::from_str(&physical).expect("worker browser state JSON");
    assert_eq!(physical["shadowClicked"], true);
    assert_eq!(physical["popup"], false);
    assert_eq!(physical["focusedTarget"], "worker-tab");
    assert_eq!(physical["frameClicked"], true);
    assert_eq!(physical["dialog"]["accept"], true);
    assert_eq!(physical["dialog"]["promptText"], "approved by home");
    assert_eq!(physical["downloads"]["behavior"], "allowAndName");
    assert_eq!(physical["canceledDownload"], "worker-active-download");
    assert_eq!(physical["upload"]["backendNodeId"], 104);
    assert_eq!(physical["upload"]["fileCount"], 1);
    assert_eq!(physical["permission"]["setting"], "denied");
    assert_eq!(physical["permission"]["origin"], "https://worker.test");

    let navigated = runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_open_url",
            json!({"url":"https://history.worker.test/current"}),
        )
        .await
        .expect("navigate before public history operation");
    assert!(navigated.ok, "{:?}", navigated.payload);
    let history_tab_id = tab_id;
    let back = runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_history",
            json!({"tab_id":history_tab_id,"action":"back"}),
        )
        .await
        .expect("public history tool crosses the bound worker relay");
    assert!(back.ok, "{:?}", back.payload);
    assert_eq!(back.payload["action"], "back");
    assert_eq!(back.payload["tab_id"], history_tab_id);
    assert_eq!(back.payload["url"], "https://worker.test/");
    let unavailable = runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_history",
            json!({"tab_id":history_tab_id,"action":"back"}),
        )
        .await
        .expect_err("history before the first entry must fail closed");
    assert!(
        unavailable
            .to_string()
            .contains("browser_history_unavailable"),
        "{unavailable}"
    );
    let forward = runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_history",
            json!({"tab_id":history_tab_id,"action":"forward"}),
        )
        .await
        .expect("public history tool moves forward through the bound worker relay");
    assert_eq!(
        forward.payload["url"],
        "https://history.worker.test/current"
    );
    let before_reload = forward.payload["document_revision"]
        .as_u64()
        .expect("document revision before reload");
    let reloaded = runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_history",
            json!({"tab_id":history_tab_id,"action":"reload"}),
        )
        .await
        .expect("public history tool reloads through the bound worker relay");
    assert_eq!(
        reloaded.payload["url"],
        "https://history.worker.test/current"
    );
    assert!(
        reloaded.payload["document_revision"]
            .as_u64()
            .is_some_and(|revision| revision > before_reload),
        "reload must advance the Room document revision: {:?}",
        reloaded.payload
    );
    let environment = runtime
        .room_environment_snapshot(room)
        .expect("Room environment after history operations");
    for (action_id, kind) in [
        (&back.payload["action_id"], "browser_history_back"),
        (&forward.payload["action_id"], "browser_history_forward"),
        (&reloaded.payload["action_id"], "browser_history_reload"),
    ] {
        assert!(environment.actions.iter().any(|action| {
            action.action_id == action_id.as_str().expect("history action id")
                && action.kind == kind
                && action.state == crate::session::EnvironmentActionState::Completed
        }));
    }

    let browser_target = json!({"kind":"browser_tab","id":history_tab_id});
    let human_history_request = json!({
        "SubmitRoomEnvironmentBrowserAction": {
            "session_id": room,
            "runtime_generation": environment.runtime_generation,
            "idempotency_key": "human-history-back-1",
            "action": {
                "kind": "history",
                "tab_id": history_tab_id,
                "action": "back"
            }
        }
    });
    let denied = dispatch_json(&fixture.home, human_history_request.clone())
        .await
        .expect_err("human Browser history requires explicit tab takeover");
    assert!(
        denied
            .to_string()
            .contains("environment_input_takeover_required"),
        "{denied}"
    );
    dispatch_json(
        &fixture.home,
        json!({"RequestRoomEnvironmentInputTakeover":{
            "session_id":room,"target":browser_target.clone()
        }}),
    )
    .await
    .expect("human takes over the stable Browser tab");
    std::fs::write(
        fixture
            ._worker_state
            .root
            .join("external-browser-navigation"),
        "external browser changed the document",
    )
    .expect("arm external browser navigation");
    let submitted = dispatch_json(&fixture.home, human_history_request.clone())
        .await
        .expect("authenticated human history refreshes an externally navigated tab");
    let submitted = &submitted["RoomEnvironmentActionSubmitted"];
    assert_eq!(
        submitted["environment"]["tabs"][0]["url"],
        "https://worker.test/"
    );
    let human_action_id = submitted["action_id"]
        .as_str()
        .expect("human Browser action ID");
    let action = submitted["environment"]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["action_id"] == human_action_id)
        .expect("attributed human Browser action");
    assert_eq!(action["mode"], "browser");
    assert_eq!(action["kind"], "browser_history_back");
    assert!(action["actor_id"]
        .as_str()
        .is_some_and(|actor| actor.starts_with("user:")));
    assert_eq!(action["state"], "completed");
    dispatch_json(
        &fixture.home,
        json!({"ReleaseRoomEnvironmentInput":{
            "session_id":room,"target":browser_target
        }}),
    )
    .await
    .expect("release human Browser tab ownership");
    let replayed = dispatch_json(&fixture.home, human_history_request)
        .await
        .expect("exact history replay does not require a second takeover");
    assert_eq!(
        replayed["RoomEnvironmentActionSubmitted"]["action_id"],
        human_action_id
    );
    assert_eq!(
        replayed["RoomEnvironmentActionSubmitted"]["environment"]["tabs"][0]["url"],
        "https://worker.test/",
        "idempotency replay must not move through history twice"
    );

    std::fs::remove_file(upload_path).expect("remove upload fixture");
}

pub(super) async fn check_cancellation_without_tabs(fixture: &LiveWorker, token: &str) {
    let runtime = &fixture.home.runtime_state;
    runtime
        .dispatch_authenticated_runtime_tool_call(token, "slice_browser_downloads", json!({}))
        .await
        .expect("start another background download before the user closes the pages");
    let status = runtime
        .dispatch_authenticated_runtime_tool_call(token, "slice_browser_status", json!({}))
        .await
        .expect("observe current browser generation");
    let close = fixture._worker_state.root.join("close-browser-tabs");
    std::fs::write(&close, b"close").expect("inject external browser page closure");
    let canceled = runtime.dispatch_authenticated_runtime_tool_call(token, "slice_browser_downloads", json!({
        "cancel":{"browser_generation":status.payload["browser_generation"],"guid":"worker-active-download"}
    })).await;
    std::fs::remove_file(close).expect("remove external browser closure fixture");
    let canceled =
        canceled.expect("download cancellation must work with no remaining browser tabs");
    assert!(canceled.ok, "{:?}", canceled.payload);
    let environment = runtime
        .room_environment_snapshot(&fixture.rooms[0])
        .unwrap();
    assert!(
        environment.tabs.is_empty(),
        "no current page may be required to cancel a download"
    );
    assert!(environment.focused_tab_id.is_none());
    assert_eq!(canceled.payload["cancellation_requested"], true);
    let events = runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_events",
            json!({
                "browser_generation":status.payload["browser_generation"],"cursor":0,"limit":200
            }),
        )
        .await
        .expect("observe terminal progress after page closure");
    assert!(events.payload["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["kind"] == "download_progress"
            && event["data"]["guid"] == "worker-active-download"
            && event["data"]["state"] == "canceled"));
}
