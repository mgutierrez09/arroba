use super::*;
use crate::transport::relay_client::send_peer_request_via_temporary_connection;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use base64::Engine as _;
use chariox_relay::protocol::ClientTarget;
use futures_util::FutureExt;

#[test]
fn worker_computer_tools_use_home_room_authority() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let _env_guard = crate::env_lock::lock();
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(64 * 1024 * 1024)
                .enable_all()
                .build()
                .expect("computer tools test runtime")
                .block_on(worker_computer_tools_scenario());
        })
        .expect("computer tools test thread")
        .join()
        .expect("computer tools test thread should not panic");
}

async fn worker_computer_tools_scenario() {
    let mut fixture = LiveWorker::start_configured_with_home_vault(
        false,
        true,
        Some(crate::config::CredentialVaultBackend::ProcessMemory),
    )
    .await;
    let check = std::panic::AssertUnwindSafe(check_worker_computer_tools(&mut fixture))
        .catch_unwind()
        .await;
    let controller_cleanup = fixture
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
    controller_cleanup.expect("stop worker controller after computer tools test");
    provider_cleanup.expect("stop worker provider after computer tools test");
    if let Err(panic) = check {
        std::panic::resume_unwind(panic);
    }
}

async fn check_worker_computer_tools(fixture: &mut LiveWorker) {
    const SECRET: &str = "remote-room-computer-secret-canary";
    let mut screenshot_bytes = vec![0_u8; 131_079];
    for (index, byte) in screenshot_bytes.iter_mut().enumerate() {
        *byte = (index % 251) as u8;
    }
    screenshot_bytes[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
    let screen_tool = fixture._worker_state.root.join("computer-tools-screen.sh");
    let screen_log = fixture._worker_state.root.join("computer-tools-screen.log");
    let screenshot = fixture._worker_state.root.join("computer-tools-screen.png");
    std::fs::write(&screenshot, &screenshot_bytes).expect("computer screenshot fixture");
    std::fs::write(
        &screen_tool,
        "#!/bin/sh\nset -eu\ncase \"${1:-}\" in\n  status)\n    printf 'available=true\\ndisplay=:99\\nscreen=1024x600\\nviewer=http://127.0.0.1:6080/vnc.html\\nmode=desktop\\n'\n    printf 'worker-viewer=http://127.0.0.1:6080/vnc.html\\n' >&2\n    ;;\n  screenshot)\n    cp \"$CHARIOX_REMOTE_ROOM_SCREENSHOT\" \"$2\"\n    ;;\n  ocr)\n    if [ \"$#\" -eq 2 ]; then\n      cmp \"$2\" \"$CHARIOX_REMOTE_ROOM_SCREENSHOT\"\n      printf 'worker-path=%s\\n' \"$2\" >&2\n      printf 'Artifact OCR\\n'\n    else\n      printf 'Grüße 世界\\nShared Computer\\n'\n    fi\n    ;;\n  find-text)\n    if [ \"${2:-}\" = 'Shared Computer' ]; then\n      if [ \"$#\" -eq 3 ]; then\n        cmp \"$3\" \"$CHARIOX_REMOTE_ROOM_SCREENSHOT\"\n        printf 'worker-path=%s\\n' \"$3\" >&2\n        printf '%s\\n' '{\"text\":\"Shared Computer\",\"left\":640,\"top\":400,\"width\":240,\"height\":40,\"center_x\":760,\"center_y\":420}'\n      else\n        printf '%s\\n' '{\"text\":\"Shared Computer\",\"left\":320,\"top\":200,\"width\":240,\"height\":40,\"center_x\":440,\"center_y\":220}'\n      fi\n    else\n      printf 'null\\n'\n      exit 1\n    fi\n    ;;\n  computer-secret-paste-stdin)\n    input=$(cat)\n    [ \"$input\" = \"$CHARIOX_REMOTE_ROOM_COMPUTER_SECRET\" ]\n    printf 'computer-secret-input-ok\\n' >> \"$CHARIOX_REMOTE_ROOM_COMPUTER_LOG\"\n    ;;\n  computer-type-stdin|computer-key-stdin|computer-clipboard-write-stdin)\n    input=$(cat)\n    printf '%s|%s\\n' \"$*\" \"$input\" >> \"$CHARIOX_REMOTE_ROOM_COMPUTER_LOG\"\n    ;;\n  *)\n    printf '%s\\n' \"$*\" >> \"$CHARIOX_REMOTE_ROOM_COMPUTER_LOG\"\n    ;;\nesac\n",
    )
    .expect("computer secret screen helper");
    let multi_match_tool = fixture
        ._worker_state
        .root
        .join("computer-tools-multi-match-screen.sh");
    std::fs::write(
        &multi_match_tool,
        "#!/bin/sh\nset -eu\nif [ \"${1:-}\" = 'find-text' ] && [ \"${2:-}\" = 'Shared Computer' ] && [ \"$#\" -eq 2 ]; then\n  printf '%s\\n' '{\"text\":\"Shared Computer\",\"left\":320,\"top\":200,\"width\":240,\"height\":40,\"center_x\":440,\"center_y\":220}'\n  printf '%s\\n' '{\"text\":\"Shared Computer\",\"left\":800,\"top\":400,\"width\":480,\"height\":80,\"center_x\":1040,\"center_y\":440}'\n  exit 0\nfi\nexec \"$CHARIOX_TEST_BASE_SCREEN_TOOL\" \"$@\"\n",
    )
    .expect("computer multi-match screen helper");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for tool in [&screen_tool, &multi_match_tool] {
            std::fs::set_permissions(tool, std::fs::Permissions::from_mode(0o700))
                .expect("computer screen helper permissions");
        }
    }
    let _environment = ScopedEnvironment::set([
        (
            "CHARIOX_HOME",
            fixture.home_state.root.as_os_str().to_os_string(),
        ),
        ("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT", "1".into()),
        ("CHARIOX_REMOTE_ROOM_COMPUTER_SECRET", SECRET.into()),
        (
            "CHARIOX_REMOTE_ROOM_COMPUTER_LOG",
            screen_log.as_os_str().to_os_string(),
        ),
        (
            "CHARIOX_REMOTE_ROOM_SCREENSHOT",
            screenshot.as_os_str().to_os_string(),
        ),
        (
            "CHARIOX_SLICE_SCREEN_TOOL",
            multi_match_tool.as_os_str().to_os_string(),
        ),
        (
            "CHARIOX_TEST_BASE_SCREEN_TOOL",
            screen_tool.as_os_str().to_os_string(),
        ),
    ]);
    crate::credential::CharioxCredentialRegistry::user()
        .expect("isolated credential registry")
        .upsert(crate::config::UserCredentialConfig {
            id: "remote-desktop-login".to_string(),
            description: None,
            source: crate::config::UserCredentialSourceConfig::Env {
                name: "CHARIOX_REMOTE_ROOM_COMPUTER_SECRET".to_string(),
            },
            allowed_hosts: Vec::new(),
            allowed_uses: vec![crate::config::UserCredentialUse::Computer],
            injection: crate::config::UserCredentialInjectionConfig::Computer,
            metadata: None,
        })
        .expect("register the isolated computer credential");

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
        .expect("test slice running");
    let room = fixture.rooms[0].clone();
    dispatch_json(
        &fixture.home,
        json!({"BindRoomEnvironmentSlice": {
            "session_id":room, "slice_ref":"desktop"
        }}),
    )
    .await
    .expect("bind the home Room to its worker slice");
    dispatch_json(
        &fixture.home,
        json!({"StartRoomEnvironment": {
            "session_id":room, "viewport": {
                "css_width":1280, "css_height":800, "device_scale_factor":1,
                "desktop_pixel_width":1280, "desktop_pixel_height":800
            }
        }}),
    )
    .await
    .expect("start the home-owned Room environment");

    let placement = fixture.placement();
    let spawned = dispatch_json(
        &fixture.home,
        json!({"SpawnAgent": {
            "session_id":room, "provider":"managed-dev-stub", "model":"default",
            "slice_ref":"desktop", "worktree_placement":placement
        }}),
    )
    .await
    .expect("spawn a leased Room agent on the worker");
    let home_agent_id = spawned["AgentSpawned"]["agent"]["id"]
        .as_str()
        .expect("home agent id")
        .to_string();
    let leased_agent_id = spawned["AgentSpawned"]["agent"]["remote_execution"]["leased_agent_id"]
        .as_str()
        .expect("leased agent id")
        .to_string();
    let remote_execution: crate::agent::RemoteAgentBinding =
        serde_json::from_value(spawned["AgentSpawned"]["agent"]["remote_execution"].clone())
            .expect("remote execution binding");
    let worker_relay_config = fixture
        .home
        .app
        .lock()
        .await
        .relay_config_for_remote_execution(&remote_execution);
    let response = send_peer_request_via_temporary_connection(
        &worker_relay_config,
        ClientTarget {
            daemon_id: Some("environment-worker".to_string()),
            daemon_alias: None,
        },
        RelayPeerRequest::SubmitLeasedPrompt {
            leased_agent_id,
            prompt: "launch the worker provider for the computer secret test".to_string(),
            hidden_system_context: String::new(),
            attachments: Vec::new(),
            workflow_context: None,
            git_context: None,
            required_mcps: Vec::new(),
            required_skills: None,
            remote_extension_manifest: Default::default(),
            provider_launch_credential: None,
        },
    )
    .await
    .expect("launch the worker provider through the relay");
    let RelayPeerResponse::LeasedPromptSubmitted {
        provider_run_id: worker_provider_run_id,
        ..
    } = response
    else {
        panic!("unexpected leased prompt response: {response:?}")
    };
    fixture
        .home
        .app
        .lock()
        .await
        .agents()
        .set_remote_execution_active_worker_provider_run_id(
            &home_agent_id,
            Some(worker_provider_run_id.clone()),
        )
        .expect("record the active worker provider run");
    let worker_session_id = fixture
        .worker
        .app
        .lock()
        .await
        .providers()
        .get_run(&worker_provider_run_id)
        .expect("worker provider run")
        .session_id()
        .to_string();
    let token = fixture
        .worker
        .runtime_state
        .runtime_mcp_auth_token_for_provider_run(&worker_provider_run_id)
        .expect("worker provider runtime MCP token");
    let actions_before_observations = fixture
        .home
        .runtime_state
        .room_environment_snapshot(&room)
        .expect("home Room before provider observations")
        .actions
        .len();

    let input_token = token.clone();
    assert!(fixture
        .worker
        .runtime_state
        .runtime_tool_specs_for_auth_token(&token)
        .iter()
        .any(|spec| spec.name == "slice_screenshot"));
    assert!(fixture
        .worker
        .runtime_state
        .runtime_tool_specs_for_auth_token(&token)
        .iter()
        .any(|spec| spec.name == "slice_screen_status"));
    let screen_status = fixture
        .worker
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &token,
            "slice_screen_status",
            json!({
                "session_id": fixture.rooms[1],
                "slice_id": "forged-slice"
            }),
        )
        .await
        .expect("worker provider screen status should use home Room authority");
    assert!(screen_status.ok, "{:?}", screen_status.payload);
    assert_eq!(screen_status.payload["source"], "computer_controller");
    assert_eq!(screen_status.payload["session_id"], room);
    assert_eq!(screen_status.payload["slice_id"], "slice-1");
    assert_eq!(screen_status.payload["agent_id"], home_agent_id);
    assert_eq!(screen_status.payload["available"], true);
    assert_eq!(screen_status.payload["screen"], "1280x800");
    assert_eq!(
        screen_status.payload["canonical_viewport"],
        json!({
            "css_width":1280,
            "css_height":800,
            "device_scale_factor":1,
            "desktop_pixel_width":1280,
            "desktop_pixel_height":800,
            "revision":1,
            "last_actor_id":Value::Null,
        })
    );
    assert_eq!(screen_status.payload["mode"], "desktop");
    assert_eq!(screen_status.payload["viewer"], Value::Null);
    assert_eq!(screen_status.payload["stdout"], Value::Null);
    assert_eq!(screen_status.payload["stderr"], Value::Null);
    assert!(!screen_status.payload.to_string().contains("127.0.0.1:6080"));
    assert_eq!(
        screen_status.payload["viewer_access"],
        "client_attachment_required"
    );
    assert!(fixture
        .worker
        .runtime_state
        .runtime_tool_specs_for_auth_token(&token)
        .iter()
        .any(|spec| spec.name == "slice_ocr"));
    let ocr = fixture
        .worker
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(&token, "slice_ocr", json!({}))
        .await
        .expect("worker provider OCR should use home Room authority");
    assert!(ocr.ok, "{:?}", ocr.payload);
    assert_eq!(ocr.payload["source"], "computer_controller");
    assert_eq!(ocr.payload["session_id"], room);
    assert_eq!(ocr.payload["slice_id"], "slice-1");
    assert_eq!(ocr.payload["agent_id"], home_agent_id);
    assert_eq!(ocr.payload["text"], "Grüße 世界\nShared Computer");
    assert_eq!(ocr.payload["stdout"], Value::Null);
    assert_eq!(ocr.payload["stderr"], Value::Null);
    assert!(fixture
        .worker
        .runtime_state
        .runtime_tool_specs_for_auth_token(&token)
        .iter()
        .any(|spec| spec.name == "slice_find_text"));
    let found = fixture
        .worker
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &token,
            "slice_find_text",
            json!({"query":"Shared Computer"}),
        )
        .await
        .expect("worker provider text lookup should use home Room authority");
    assert!(found.ok, "{:?}", found.payload);
    assert_eq!(found.payload["source"], "computer_controller");
    assert_eq!(found.payload["session_id"], room);
    assert_eq!(found.payload["slice_id"], "slice-1");
    assert_eq!(found.payload["agent_id"], home_agent_id);
    assert_eq!(found.payload["match"]["text"], "Shared Computer");
    assert_eq!(found.payload["match"]["left"], 320);
    assert_eq!(found.payload["match"]["top"], 200);
    assert_eq!(found.payload["match"]["width"], 240);
    assert_eq!(found.payload["match"]["height"], 40);
    assert_eq!(found.payload["match"]["center_x"], 440);
    assert_eq!(found.payload["match"]["center_y"], 220);
    assert_eq!(found.payload["match_count"], 2);
    assert_eq!(
        found.payload["matches"],
        json!([
            {
                "text":"Shared Computer",
                "left":320,
                "top":200,
                "width":240,
                "height":40,
                "center_x":440,
                "center_y":220
            },
            {
                "text":"Shared Computer",
                "left":800,
                "top":400,
                "width":480,
                "height":80,
                "center_x":1040,
                "center_y":440
            }
        ])
    );
    assert_eq!(found.payload["stdout"], Value::Null);
    assert_eq!(found.payload["stderr"], Value::Null);
    let missing = fixture
        .worker
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &token,
            "slice_find_text",
            json!({"query":"not visible"}),
        )
        .await
        .expect("a text miss is a completed observation");
    assert!(!missing.ok);
    assert_eq!(missing.payload["source"], "computer_controller");
    assert_eq!(missing.payload["match"], Value::Null);
    assert_eq!(missing.payload["match_count"], 0);
    assert_eq!(missing.payload["matches"], json!([]));
    for query in ["   ".to_string(), "x".repeat(4097)] {
        let error = fixture
            .worker
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(
                &token,
                "slice_find_text",
                json!({"query":query}),
            )
            .await
            .expect_err("invalid text lookup query must fail before worker execution");
        assert!(
            error
                .to_string()
                .contains("query must contain between 1 and 4096 UTF-8 bytes"),
            "unexpected text lookup validation error: {error}"
        );
    }
    let observed = fixture
        .worker
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &token,
            "slice_screenshot",
            json!({
                "return_image_base64": true,
                "session_id": fixture.rooms[1],
                "slice_id": "forged-slice",
                "path": "/tmp/forged-provider-path.png"
            }),
        )
        .await
        .expect("worker provider screenshot should use home Room authority");
    assert!(observed.ok, "{:?}", observed.payload);
    assert_eq!(observed.payload["source"], "computer_controller");
    assert_eq!(observed.payload["session_id"], room);
    assert_eq!(observed.payload["slice_id"], "slice-1");
    assert_eq!(observed.payload["agent_id"], home_agent_id);
    assert_eq!(observed.payload["mime_type"], "image/png");
    assert_eq!(observed.payload["image_path"], Value::Null);
    assert!(observed.payload["artifact_id"].as_str().is_some());
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(
                observed.payload["image_base64"]
                    .as_str()
                    .expect("inline provider screenshot"),
            )
            .expect("provider screenshot Base64"),
        screenshot_bytes,
    );
    let screenshot_artifact_id = observed.payload["artifact_id"]
        .as_str()
        .expect("provider screenshot artifact ID");
    let artifact_ocr = fixture
        .worker
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &token,
            "slice_ocr",
            json!({"artifact_id":screenshot_artifact_id}),
        )
        .await
        .expect("worker provider OCR should reuse an opaque Room screenshot artifact");
    assert!(artifact_ocr.ok, "{:?}", artifact_ocr.payload);
    assert_eq!(artifact_ocr.payload["source"], "computer_controller");
    assert_eq!(artifact_ocr.payload["text"], "Artifact OCR");
    let unknown_artifact = fixture
        .worker
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &token,
            "slice_ocr",
            json!({"artifact_id":"unknown-artifact"}),
        )
        .await
        .expect_err("OCR must reject an unknown Room screenshot artifact");
    assert!(
        unknown_artifact
            .to_string()
            .contains("artifact was not found"),
        "unexpected OCR artifact error: {unknown_artifact}"
    );
    let foreign_screenshot = fixture._worker_state.root.join("foreign-room-screen.png");
    let mut foreign_screenshot_bytes = screenshot_bytes.clone();
    *foreign_screenshot_bytes
        .last_mut()
        .expect("foreign screenshot fixture has bytes") ^= 1;
    std::fs::write(&foreign_screenshot, foreign_screenshot_bytes)
        .expect("write foreign Room screenshot fixture");
    let foreign_store = crate::artifacts::OperationalArtifactStore::open(
        fixture._worker_state.config.operational_artifact_root(),
        fixture
            ._worker_state
            .config
            .operational_artifact_index_path(),
    )
    .expect("open worker artifact store for foreign Room fixture");
    let foreign_artifact = foreign_store
        .store_existing_file(crate::artifacts::StoreArtifactRequest {
            source_path: foreign_screenshot,
            display_name: "foreign-room.png".to_string(),
            source_kind: "room_environment_screenshot".to_string(),
            media_type: Some("image/png".to_string()),
            enqueue_archive: false,
            session_id: Some(fixture.rooms[1].clone()),
            attachment_id: None,
            workspace_id: None,
            worktree_path: None,
            metadata: std::collections::BTreeMap::from([(
                "slice_id".to_string(),
                json!("slice-1"),
            )]),
        })
        .expect("store foreign Room screenshot fixture");
    let foreign_artifact_error = fixture
        .worker
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &token,
            "slice_ocr",
            json!({"artifact_id":foreign_artifact.artifact_id}),
        )
        .await
        .expect_err("OCR must reject an artifact from another Room");
    assert!(
        foreign_artifact_error
            .to_string()
            .contains("does not belong to this Room Environment"),
        "unexpected cross-Room OCR error: {foreign_artifact_error}"
    );
    let forged_path = fixture
        .worker
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &token,
            "slice_ocr",
            json!({"image_path":"/tmp/forged-provider-ocr.png"}),
        )
        .await
        .expect_err("Room OCR must reject a caller-supplied worker path");
    assert!(
        forged_path.to_string().contains("not image_path"),
        "unexpected Room OCR path error: {forged_path}"
    );
    let artifact_match = fixture
        .worker
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &token,
            "slice_find_text",
            json!({
                "query":"Shared Computer",
                "artifact_id":screenshot_artifact_id
            }),
        )
        .await
        .expect("worker provider text lookup should reuse an opaque screenshot artifact");
    assert!(artifact_match.ok, "{:?}", artifact_match.payload);
    assert_eq!(artifact_match.payload["source"], "computer_controller");
    assert_eq!(artifact_match.payload["match"]["left"], 640);
    assert_eq!(artifact_match.payload["match"]["center_x"], 760);
    assert_eq!(
        fixture
            .home
            .runtime_state
            .room_environment_snapshot(&room)
            .expect("home Room after provider screenshot")
            .actions
            .len(),
        actions_before_observations,
        "Computer observations do not enter the mutating Room Action ledger",
    );
    let runtime = fixture.worker.runtime_state.clone();
    let call = tokio::spawn(async move {
        runtime
            .dispatch_authenticated_runtime_tool_call(
                &token,
                "paste_secret_to_computer",
                json!({"credential_id":"remote-desktop-login"}),
            )
            .await
    });
    let interaction_id = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let session = fixture
                .home
                .runtime_state
                .session_snapshot_projection(&room, 0)
                .expect("home Room projection")
                .session;
            if let Some(interaction) = session
                .active_interactions()
                .iter()
                .find(|interaction| interaction.title() == Some("Computer credential input"))
            {
                break interaction.id().to_string();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("computer credential approval appears on home");
    fixture
        .home
        .runtime_state
        .resolve_runtime_interaction(&room, &interaction_id, "allow", None)
        .await
        .expect("approve the computer credential on home");
    let result = call
        .await
        .expect("computer secret task joins")
        .expect("worker computer secret uses home Room authority");

    assert!(result.ok, "{:?}", result.payload);
    assert_eq!(result.payload["target"], "desktop_focus");
    assert_eq!(
        result.payload["actor_id"],
        crate::session::agent_environment_actor_id(&home_agent_id)
    );
    assert!(
        !serde_json::to_string(&result.payload)
            .expect("serialize runtime tool payload")
            .contains(SECRET),
        "secret material must not cross the runtime tool result"
    );
    assert_eq!(
        std::fs::read_to_string(&screen_log).expect("worker secret input log"),
        "computer-secret-input-ok\n"
    );
    let home_environment = fixture
        .home
        .runtime_state
        .room_environment_snapshot(&room)
        .expect("home Room keeps the action ledger");
    let action = home_environment
        .actions
        .iter()
        .find(|action| action.kind == "secret_input")
        .expect("home Room records computer secret input");
    assert_eq!(
        action.state,
        crate::session::EnvironmentActionState::Completed
    );
    assert_eq!(action.arguments, None);

    let input_cases = [
        (
            "slice_mouse",
            json!({"action":"move","x":120,"y":160}),
            "pointer_move",
        ),
        (
            "slice_mouse",
            json!({"action":"click","x":220,"y":260,"button":"right"}),
            "pointer_click",
        ),
        (
            "slice_mouse",
            json!({"action":"double_click","x":320,"y":360}),
            "pointer_click",
        ),
        (
            "slice_mouse",
            json!({"action":"drag","x":120,"y":160,"to_x":720,"to_y":560,"button":"middle"}),
            "pointer_drag",
        ),
        (
            "slice_mouse",
            json!({"action":"scroll","x":640,"y":400,"amount":5,"horizontal_steps":-3}),
            "pointer_scroll",
        ),
        (
            "slice_clipboard_write",
            json!({"text":"Clipboard Grüße 世界"}),
            "clipboard_write",
        ),
        (
            "slice_keyboard",
            json!({"action":"type","text":"Grüße 世界"}),
            "keyboard_text",
        ),
        (
            "slice_keyboard",
            json!({"action":"key","key":"ctrl+shift+p","repeat":3}),
            "keyboard_key",
        ),
    ];
    let mut action_ids = Vec::new();
    for (tool, arguments, action_kind) in input_cases {
        let result = fixture
            .worker
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(&input_token, tool, arguments)
            .await
            .expect("worker Computer tool should forward through home Room authority");
        assert!(result.ok, "{action_kind}: {:?}", result.payload);
        assert_eq!(result.payload["source"], "computer_controller");
        assert_eq!(result.payload["session_id"], room);
        assert_eq!(result.payload["agent_id"], home_agent_id);
        assert_eq!(
            result.payload["actor_id"],
            crate::session::agent_environment_actor_id(&home_agent_id)
        );
        assert_eq!(result.payload["action_kind"], action_kind);
        action_ids.push(
            result.payload["action_id"]
                .as_str()
                .expect("Computer tool result should identify its Room Action")
                .to_string(),
        );
    }

    let action_count_before_invalid = fixture
        .home
        .runtime_state
        .room_environment_snapshot(&room)
        .expect("home Room before invalid provider input")
        .actions
        .len();
    let invalid_input = fixture
        .worker
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &input_token,
            "slice_keyboard",
            json!({"action":"key","key":"ctrl+p","repeat":33}),
        )
        .await
        .expect_err("home Room authority should reject an excessive key repeat");
    assert!(
        invalid_input
            .to_string()
            .contains("environment_invalid_keyboard_repeat"),
        "unexpected invalid input error: {invalid_input}"
    );

    let home_environment = fixture
        .home
        .runtime_state
        .room_environment_snapshot(&room)
        .expect("home Room keeps all Computer Actions");
    assert_eq!(
        home_environment.actions.len(),
        action_count_before_invalid,
        "rejected provider input must not enter the home Action ledger"
    );
    for (action_id, expected_kind) in action_ids.iter().zip([
        "pointer_move",
        "pointer_click",
        "pointer_click",
        "pointer_drag",
        "pointer_scroll",
        "clipboard_write",
        "keyboard_text",
        "keyboard_key",
    ]) {
        let action = home_environment
            .actions
            .iter()
            .find(|action| &action.action_id == action_id)
            .expect("home Room should record the provider Computer Action");
        assert_eq!(action.kind, expected_kind);
        assert_eq!(
            action.actor_id,
            crate::session::agent_environment_actor_id(&home_agent_id)
        );
        assert_eq!(
            action.state,
            crate::session::EnvironmentActionState::Completed
        );
    }
    let keyboard = home_environment
        .actions
        .iter()
        .find(|action| action.action_id == action_ids[6])
        .expect("keyboard text Action");
    assert_eq!(
        keyboard.arguments,
        Some(crate::session::EnvironmentActionArguments::KeyboardText {
            utf8_byte_count: 14,
            character_count: 8,
        })
    );
    let key = home_environment
        .actions
        .iter()
        .find(|action| action.action_id == action_ids[7])
        .expect("keyboard key Action");
    assert_eq!(
        key.arguments,
        Some(crate::session::EnvironmentActionArguments::KeyboardKey { repeat: 3 })
    );
    let clipboard = home_environment
        .actions
        .iter()
        .find(|action| action.action_id == action_ids[5])
        .expect("clipboard write Action");
    assert_eq!(
        clipboard.arguments,
        Some(crate::session::EnvironmentActionArguments::ClipboardWrite {
            utf8_byte_count: 24,
            character_count: 18,
        })
    );
    let environment_debug = format!("{home_environment:?}");
    assert!(
        !environment_debug.contains("Grüße 世界")
            && !environment_debug.contains("Clipboard Grüße 世界")
            && !environment_debug.contains("ctrl+shift+p"),
        "Room history must not retain keyboard or clipboard contents"
    );
    assert!(
        fixture
            .worker
            .runtime_state
            .room_environment_snapshot(&worker_session_id)
            .is_err(),
        "worker must not create parallel Room authority for any Computer tool"
    );

    assert_eq!(
        std::fs::read_to_string(&screen_log).expect("worker Computer tool log"),
        concat!(
            "computer-secret-input-ok\n",
            "move 120 160\n",
            "pointer-click 220 260 right 1\n",
            "pointer-click 320 360 left 2\n",
            "pointer-drag 120 160 720 560 middle\n",
            "pointer-scroll 640 400 -3 5\n",
            "computer-clipboard-write-stdin|Clipboard Grüße 世界\n",
            "computer-type-stdin|Grüße 世界\n",
            "computer-key-stdin 3|ctrl+shift+p\n",
        ),
        "provider Computer tools must use the shared physical input adapter",
    );

    dispatch_json(
        &fixture.home,
        json!({"DestroyAgent":{"session_id":room,"agent_id":home_agent_id}}),
    )
    .await
    .expect("destroy the leased agent after the computer tools test");
    dispatch_json(
        &fixture.home,
        json!({"StopRoomEnvironment":{"session_id":room}}),
    )
    .await
    .expect("stop the home Room environment after the computer tools test");
}

struct ScopedEnvironment(Vec<(&'static str, Option<std::ffi::OsString>)>);

impl ScopedEnvironment {
    fn set<const N: usize>(values: [(&'static str, std::ffi::OsString); N]) -> Self {
        let mut previous = Vec::with_capacity(N);
        for (name, value) in values {
            previous.push((name, std::env::var_os(name)));
            std::env::set_var(name, value);
        }
        Self(previous)
    }
}

impl Drop for ScopedEnvironment {
    fn drop(&mut self) {
        for (name, value) in self.0.drain(..).rev() {
            if let Some(value) = value {
                std::env::set_var(name, value);
            } else {
                std::env::remove_var(name);
            }
        }
    }
}

pub(super) async fn check(fixture: &LiveWorker, placement: Value) {
    let room = fixture.rooms[0].clone();
    let spawned = dispatch_json(
        &fixture.home,
        json!({"SpawnAgent": {
            "session_id":room,
            "provider":"managed-dev-stub",
            "model":"default",
            "slice_ref":"desktop",
            "worktree_placement":placement
        }}),
    )
    .await
    .expect("spawn a leased Room agent on the browser worker");
    let home_agent_id = spawned["AgentSpawned"]["agent"]["id"]
        .as_str()
        .expect("home agent id")
        .to_string();
    let leased_agent_id = spawned["AgentSpawned"]["agent"]["remote_execution"]["leased_agent_id"]
        .as_str()
        .expect("leased agent id")
        .to_string();
    let remote_execution: crate::agent::RemoteAgentBinding =
        serde_json::from_value(spawned["AgentSpawned"]["agent"]["remote_execution"].clone())
            .expect("remote execution binding");
    let worker_relay_config = {
        let app = fixture.home.app.lock().await;
        app.relay_config_for_remote_execution(&remote_execution)
    };

    let check_result = std::panic::AssertUnwindSafe(async {
        let response = send_peer_request_via_temporary_connection(
            &worker_relay_config,
            ClientTarget {
                daemon_id: Some("environment-worker".to_string()),
                daemon_alias: None,
            },
            RelayPeerRequest::SubmitLeasedPrompt {
                leased_agent_id: leased_agent_id.clone(),
                prompt: "launch the worker provider for the runtime MCP drill".to_string(),
                hidden_system_context: String::new(),
                attachments: Vec::new(),
                workflow_context: None,
                git_context: None,
                required_mcps: Vec::new(),
                required_skills: None,
                remote_extension_manifest: Default::default(),
                provider_launch_credential: None,
            },
        )
        .await
        .expect("submit the leased prompt through the authenticated relay");
        let RelayPeerResponse::LeasedPromptSubmitted {
            provider_run_id: worker_provider_run_id,
            ..
        } = response
        else {
            panic!("unexpected leased prompt response: {response:?}")
        };
        fixture
            .home
            .app
            .lock()
            .await
            .agents()
            .set_remote_execution_active_worker_provider_run_id(
                &home_agent_id,
                Some(worker_provider_run_id.clone()),
            )
            .expect("record the acknowledged worker provider on the home binding");
        let worker_session_id = {
            let app = fixture.worker.app.lock().await;
            let run = app
                .providers()
                .get_run(&worker_provider_run_id)
                .expect("worker provider run");
            run.session_id().to_string()
        };
        let token = fixture
            .worker
            .runtime_state
            .runtime_mcp_auth_token_for_provider_run(&worker_provider_run_id)
            .expect("worker provider runtime MCP token");
        let advertised = fixture
            .worker
            .runtime_state
            .runtime_tool_specs_for_auth_token(&token)
            .into_iter()
            .map(|spec| spec.name)
            .collect::<std::collections::BTreeSet<_>>();
        for expected in [
            "slice_browser_status",
            "slice_browser_tab",
            "slice_browser_history",
            "slice_open_url",
            "slice_browser_click",
            "slice_browser_fill",
            "slice_browser_submit",
            "slice_browser_dialog",
            "slice_browser_events",
            "slice_browser_downloads",
            "slice_browser_upload",
            "slice_browser_permission",
            "slice_browser_find",
            "slice_browser_text",
            "slice_browser_wait_for_text",
            "slice_browser_wait_for_selector",
            "slice_browser_wait_for_idle",
        ] {
            assert!(
                advertised.contains(expected),
                "missing worker tool {expected}"
            );
        }
        let denied = send_peer_request_via_temporary_connection(
            &fixture.home_state.config,
            ClientTarget {
                daemon_id: Some(fixture.home_state.config.daemon_id.clone()),
                daemon_alias: None,
            },
            RelayPeerRequest::ForwardRoomBrowserRuntimeTool {
                context: crate::transport::relay_peer::RemoteExtensionInvocationContext {
                    home_kernel_id: fixture.home_state.config.daemon_id.clone(),
                    home_session_id: room.clone(),
                    home_agent_id: home_agent_id.clone(),
                    leased_agent_id: leased_agent_id.clone(),
                    worker_provider_run_id: worker_provider_run_id.clone(),
                    worker_kernel_id: Some("environment-worker".to_string()),
                    worker_machine_id: Some("slice:slice-1".to_string()),
                },
                call: crate::transport::relay_peer::RemoteRoomBrowserRuntimeToolCall {
                    tool_name: "slice_browser_status".to_string(),
                    arguments: json!({}),
                },
            },
        )
        .await
        .expect_err("a non-worker relay sender must not exercise the Room browser");
        assert!(
            denied
                .to_string()
                .contains("relay sender does not match the bound worker kernel"),
            "{denied}"
        );
        let url = "https://worker-agent.worker.test/path?runtime=mcp";
        let result = fixture
            .worker
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(&token, "slice_open_url", json!({"url":url}))
            .await
            .expect("worker provider browser MCP call forwards to the home Room");
        assert!(result.ok, "{:?}", result.payload);
        assert_eq!(result.payload["session_id"], room);
        assert_eq!(result.payload["agent_id"], home_agent_id);
        assert_eq!(result.payload["actor_id"], format!("agent:{home_agent_id}"));
        assert!(result.payload["action_id"]
            .as_str()
            .is_some_and(|action_id| !action_id.is_empty()));
        assert_eq!(result.payload["browser"]["url"], url);

        let status = fixture
            .worker
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(&token, "slice_browser_status", json!({}))
            .await
            .expect("worker provider reads the stable Room tab after navigation");
        let tab_id = status.payload["tab_id"]
            .as_str()
            .expect("worker provider stable Room tab id");
        let history = fixture
            .worker
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(
                &token,
                "slice_browser_history",
                json!({"tab_id":tab_id,"action":"back"}),
            )
            .await
            .expect("worker provider history call forwards to the home Room");
        assert!(history.ok, "{:?}", history.payload);
        assert_eq!(history.payload["session_id"], room);
        assert_eq!(history.payload["agent_id"], home_agent_id);
        assert_eq!(
            history.payload["actor_id"],
            format!("agent:{home_agent_id}")
        );
        assert_eq!(history.payload["tab_id"], tab_id);
        assert_ne!(history.payload["url"], url);

        let environment = fixture
            .home
            .runtime_state
            .room_environment_snapshot(&room)
            .expect("home Room after worker-provider navigation");
        let focused = environment
            .focused_tab_id
            .as_deref()
            .and_then(|focused| environment.tabs.iter().find(|tab| tab.tab_id == focused))
            .expect("focused home Room tab");
        assert_eq!(focused.url, history.payload["url"]);
        assert!(
            fixture
                .worker
                .runtime_state
                .room_environment_snapshot(&worker_session_id)
                .is_err(),
            "worker MCP forwarding must not create parallel Room authority"
        );
    })
    .catch_unwind()
    .await;

    let cleanup = dispatch_json(
        &fixture.home,
        json!({"DestroyAgent":{"session_id":room,"agent_id":home_agent_id}}),
    )
    .await;
    if let Err(panic) = check_result {
        let _ = cleanup;
        std::panic::resume_unwind(panic);
    }
    cleanup.expect("destroy the leased worker agent after the MCP drill");
}
