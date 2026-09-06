use super::*;
use crate::local::RoomEnvironmentScreenshotChunk;
use crate::transport::relay_peer::{
    RelayPeerRequest, RelayPeerResponse, RemoteExtensionInvocationContext,
    RemoteRoomBrowserRuntimeToolCall, RemoteRoomBrowserRuntimeToolResult,
    RemoteRoomComputerObservationCall, RemoteRoomComputerObservationResult,
    RELAY_PEER_PROTOCOL_VERSION,
};
use crate::transport::room_browser_controller::RoomBrowserControllerCommand;
use crate::transport::room_browser_controller::{
    RoomComputerClipboardText, RoomComputerInputAction, RoomComputerKeyboardInput,
    RoomComputerPointerButton, RoomComputerSecretInput,
};

#[test]
fn browser_history_peer_contract_is_document_bound_and_versioned() {
    use crate::runtime::browser_controller_history::{
        BrowserControllerHistoryResult, BrowserHistoryAction,
    };
    use crate::transport::room_browser_controller::RoomBrowserControllerResult;

    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 310);
    assert_eq!(RELAY_PEER_PROTOCOL_VERSION, 44);
    let command = RoomBrowserControllerCommand::History {
        target_id: "target-a".into(),
        document_id: "document-a".into(),
        action: BrowserHistoryAction::Back,
    };
    assert_eq!(
        serde_json::to_value(&command).unwrap(),
        serde_json::json!({
            "kind":"history", "target_id":"target-a", "document_id":"document-a",
            "action":"back"
        })
    );
    let result = RoomBrowserControllerResult::History {
        result: Some(BrowserControllerHistoryResult {
            browser_generation: 2,
            target_id: "target-a".into(),
            document_id: "document-b".into(),
            action: BrowserHistoryAction::Back,
            url: "https://example.test/previous".into(),
        }),
    };
    assert_eq!(
        serde_json::to_value(&result).unwrap(),
        serde_json::json!({
            "kind":"history", "result": {
                "browser_generation":2, "target_id":"target-a",
                "document_id":"document-b", "action":"back",
                "url":"https://example.test/previous"
            }
        })
    );
}

#[test]
fn download_cancellation_peer_contract_is_versioned_and_does_not_require_a_live_tab() {
    use crate::runtime::browser_controller_file_transfer::{
        BrowserControllerDownloadCancellationResult, BrowserDownloadCancellation,
    };
    use crate::transport::room_browser_controller::RoomBrowserControllerResult;
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 310);
    assert_eq!(RELAY_PEER_PROTOCOL_VERSION, 44);
    let command = RoomBrowserControllerCommand::CancelDownload {
        cancellation: BrowserDownloadCancellation::new(2, "download-a".into()).unwrap(),
    };
    assert_eq!(
        serde_json::to_value(&command).unwrap(),
        serde_json::json!({
            "kind": "cancel_download", "cancellation": {"browser_generation": 2, "guid": "download-a"}
        })
    );
    let result = RoomBrowserControllerResult::DownloadCancellation {
        result: Some(BrowserControllerDownloadCancellationResult {
            browser_generation: 2,
            guid: "download-a".into(),
            cancellation_requested: true,
        }),
    };
    assert_eq!(
        serde_json::to_value(&result).unwrap(),
        serde_json::json!({
            "kind": "download_cancellation", "result": {"browser_generation": 2, "guid": "download-a", "cancellation_requested": true}
        })
    );
    let action =
        crate::session::EnvironmentActionRequest::browser_download_cancellation("agent:a", 1);
    assert_eq!(action.mode, crate::session::EnvironmentMode::Browser);
    assert_eq!(action.targets, vec![crate::session::InputTarget::Desktop]);
    assert!(action.tab_preconditions.is_empty());
}

#[test]
fn room_screenshot_peer_protocol_is_bounded_and_versioned() {
    assert_eq!(RELAY_PEER_PROTOCOL_VERSION, 44);

    let request = RelayPeerRequest::ReadRoomScreenshotChunk {
        session_id: "session-1".to_string(),
        slice_id: "slice-1".to_string(),
        artifact_id: "artifact-1".to_string(),
        offset: 131_072,
        max_bytes: 131_072,
    };
    assert_eq!(
        serde_json::to_value(&request).expect("Room screenshot peer request should encode"),
        serde_json::json!({
            "kind": "read_room_screenshot_chunk",
            "session_id": "session-1",
            "slice_id": "slice-1",
            "artifact_id": "artifact-1",
            "offset": 131072,
            "max_bytes": 131072
        })
    );

    let response = RelayPeerResponse::RoomScreenshotChunk {
        session_id: "session-1".to_string(),
        slice_id: "slice-1".to_string(),
        chunk: RoomEnvironmentScreenshotChunk {
            artifact_id: "artifact-1".to_string(),
            offset: 0,
            data_base64: "YWJj".to_string(),
            eof: false,
        },
    };
    let response_value =
        serde_json::to_value(&response).expect("Room screenshot peer response should encode");
    assert_eq!(
        serde_json::from_value::<RelayPeerResponse>(response_value)
            .expect("Room screenshot peer response should decode"),
        response
    );
}

#[test]
fn room_computer_observation_peer_protocol_is_typed_redacted_and_versioned() {
    assert_eq!(RELAY_PEER_PROTOCOL_VERSION, 44);
    let request = RelayPeerRequest::ObserveRoomComputer {
        session_id: "room-1".to_string(),
        slice_id: "slice-1".to_string(),
        call: RemoteRoomComputerObservationCall::ScreenStatus,
    };
    let request_wire = serde_json::json!({
        "kind":"observe_room_computer",
        "session_id":"room-1",
        "slice_id":"slice-1",
        "call":{"kind":"screen_status"}
    });
    assert_eq!(serde_json::to_value(&request).unwrap(), request_wire);
    assert_eq!(
        serde_json::from_value::<RelayPeerRequest>(request_wire).unwrap(),
        request
    );

    for (call, call_wire) in [
        (
            RemoteRoomComputerObservationCall::Ocr {
                artifact_id: Some("sensitive-artifact-id".to_string()),
            },
            serde_json::json!({
                "kind":"ocr",
                "artifact_id":"sensitive-artifact-id"
            }),
        ),
        (
            RemoteRoomComputerObservationCall::FindText {
                query: "sensitive visible query".to_string(),
                artifact_id: Some("sensitive-artifact-id".to_string()),
            },
            serde_json::json!({
                "kind":"find_text",
                "query":"sensitive visible query",
                "artifact_id":"sensitive-artifact-id"
            }),
        ),
    ] {
        let request = RelayPeerRequest::ObserveRoomComputer {
            session_id: "room-1".to_string(),
            slice_id: "slice-1".to_string(),
            call,
        };
        let wire = serde_json::json!({
            "kind":"observe_room_computer",
            "session_id":"room-1",
            "slice_id":"slice-1",
            "call":call_wire
        });
        assert_eq!(serde_json::to_value(&request).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<RelayPeerRequest>(wire).unwrap(),
            request
        );
        let debug = format!("{request:?}");
        assert!(!debug.contains("sensitive visible query"));
        assert!(!debug.contains("sensitive-artifact-id"));
    }

    let response = RelayPeerResponse::RoomComputerObserved {
        session_id: "room-1".to_string(),
        slice_id: "slice-1".to_string(),
        result: RemoteRoomComputerObservationResult(
            crate::transport::runtime_tools::RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({
                    "viewer":"http://sensitive-worker-endpoint.test/"
                }),
            },
        ),
    };
    let response_wire = serde_json::json!({
        "kind":"room_computer_observed",
        "session_id":"room-1",
        "slice_id":"slice-1",
        "result":{
            "ok":true,
            "payload":{"viewer":"http://sensitive-worker-endpoint.test/"}
        }
    });
    assert_eq!(serde_json::to_value(&response).unwrap(), response_wire);
    assert_eq!(
        serde_json::from_value::<RelayPeerResponse>(response_wire).unwrap(),
        response
    );
    assert!(!format!("{response:?}").contains("sensitive-worker-endpoint"));
}

#[test]
fn room_controller_protocol_shapes_are_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 310);
    assert_eq!(RELAY_PEER_PROTOCOL_VERSION, 44);
    for (command, wire_command) in [
        (
            RoomBrowserControllerCommand::Action {
                execution_id: "11111111111111111111111111111111".into(),
                target_id: "target-1".into(),
                document_id: "doc-1".into(),
                node_ref: "backend:1".into(),
                action: crate::runtime::browser_controller_action::BrowserLocatorAction::Fill {
                    text: "sensitive-fill-fixture".into(),
                    append: false,
                    submit: false,
                    expected_document_url: Some("https://example.test/login".into()),
                },
                timeout_ms: 500,
            },
            serde_json::json!({"kind":"action","execution_id":"11111111111111111111111111111111","target_id":"target-1","document_id":"doc-1",
                "node_ref":"backend:1","action":{"kind":"fill","text":"sensitive-fill-fixture",
                "append":false,"submit":false,"expected_document_url":"https://example.test/login"},"timeout_ms":500}),
        ),
        (
            RoomBrowserControllerCommand::RecoverAction {
                execution_id: "11111111111111111111111111111111".into(),
                target_id: "target-1".into(),
                document_id: "doc-1".into(),
                node_ref: "backend:1".into(),
                action: crate::runtime::browser_controller_action::BrowserLocatorAction::Fill {
                    text: "sensitive-fill-fixture".into(),
                    append: false,
                    submit: false,
                    expected_document_url: None,
                },
                timeout_ms: 500,
            },
            serde_json::json!({"kind":"recover_action","execution_id":"11111111111111111111111111111111","target_id":"target-1","document_id":"doc-1",
                "node_ref":"backend:1","action":{"kind":"fill","text":"sensitive-fill-fixture",
                "append":false,"submit":false},"timeout_ms":500}),
        ),
        (
            RoomBrowserControllerCommand::CancelAction {
                execution_id: "11111111111111111111111111111111".into(),
            },
            serde_json::json!({"kind":"cancel_action","execution_id":"11111111111111111111111111111111"}),
        ),
        (
            RoomBrowserControllerCommand::Tab {
                target_id: "target-popup".into(),
                document_id: "document-popup".into(),
                action: crate::runtime::browser_controller_tab::BrowserTabAction::Close,
            },
            serde_json::json!({
                "kind":"tab","target_id":"target-popup","document_id":"document-popup",
                "action":"close"
            }),
        ),
        (
            RoomBrowserControllerCommand::ComputerInput {
                action_id: "action-7".into(),
                actor_id: "user:owner-1".into(),
                runtime_generation: 4,
                viewport_revision: 9,
                desktop_pixel_width: 1280,
                desktop_pixel_height: 800,
                action: RoomComputerInputAction::PointerClick {
                    x: 320,
                    y: 180,
                    button: RoomComputerPointerButton::Right,
                    click_count: 2,
                },
            },
            serde_json::json!({
                "kind":"computer_input",
                "action_id":"action-7",
                "actor_id":"user:owner-1",
                "runtime_generation":4,
                "viewport_revision":9,
                "desktop_pixel_width":1280,
                "desktop_pixel_height":800,
                "action":{"kind":"pointer_click","x":320,"y":180,"button":"right","click_count":2}
            }),
        ),
        (
            RoomBrowserControllerCommand::ComputerInput {
                action_id: "action-clipboard".into(),
                actor_id: "user:owner-1".into(),
                runtime_generation: 4,
                viewport_revision: 9,
                desktop_pixel_width: 1280,
                desktop_pixel_height: 800,
                action: RoomComputerInputAction::ClipboardWrite {
                    text: RoomComputerClipboardText::new(
                        "sensitive-clipboard-fixture".to_string(),
                    ),
                },
            },
            serde_json::json!({
                "kind":"computer_input","action_id":"action-clipboard",
                "actor_id":"user:owner-1","runtime_generation":4,"viewport_revision":9,
                "desktop_pixel_width":1280,"desktop_pixel_height":800,
                "action":{"kind":"clipboard_write","text":"sensitive-clipboard-fixture"}
            }),
        ),
        (
            RoomBrowserControllerCommand::ComputerClipboardRead {
                actor_id: "user:owner-1".into(),
                runtime_generation: 4,
            },
            serde_json::json!({
                "kind":"computer_clipboard_read","actor_id":"user:owner-1",
                "runtime_generation":4
            }),
        ),
        (
            RoomBrowserControllerCommand::ComputerInput {
                action_id: "action-move".into(),
                actor_id: "user:owner-1".into(),
                runtime_generation: 4,
                viewport_revision: 9,
                desktop_pixel_width: 1280,
                desktop_pixel_height: 800,
                action: RoomComputerInputAction::PointerMove { x: 640, y: 400 },
            },
            serde_json::json!({
                "kind":"computer_input","action_id":"action-move","actor_id":"user:owner-1",
                "runtime_generation":4,"viewport_revision":9,"desktop_pixel_width":1280,
                "desktop_pixel_height":800,"action":{"kind":"pointer_move","x":640,"y":400}
            }),
        ),
        (
            RoomBrowserControllerCommand::ComputerInput {
                action_id: "action-drag".into(),
                actor_id: "user:owner-1".into(),
                runtime_generation: 4,
                viewport_revision: 9,
                desktop_pixel_width: 1280,
                desktop_pixel_height: 800,
                action: RoomComputerInputAction::PointerDrag {
                    from_x: 120,
                    from_y: 160,
                    to_x: 720,
                    to_y: 560,
                    button: RoomComputerPointerButton::Left,
                },
            },
            serde_json::json!({
                "kind":"computer_input","action_id":"action-drag","actor_id":"user:owner-1",
                "runtime_generation":4,"viewport_revision":9,"desktop_pixel_width":1280,
                "desktop_pixel_height":800,"action":{"kind":"pointer_drag","from_x":120,
                "from_y":160,"to_x":720,"to_y":560,"button":"left"}
            }),
        ),
        (
            RoomBrowserControllerCommand::ComputerInput {
                action_id: "action-scroll".into(),
                actor_id: "user:owner-1".into(),
                runtime_generation: 4,
                viewport_revision: 9,
                desktop_pixel_width: 1280,
                desktop_pixel_height: 800,
                action: RoomComputerInputAction::PointerScroll {
                    x: 640,
                    y: 400,
                    horizontal_steps: -3,
                    vertical_steps: 5,
                },
            },
            serde_json::json!({
                "kind":"computer_input","action_id":"action-scroll","actor_id":"user:owner-1",
                "runtime_generation":4,"viewport_revision":9,"desktop_pixel_width":1280,
                "desktop_pixel_height":800,"action":{"kind":"pointer_scroll","x":640,"y":400,
                "horizontal_steps":-3,"vertical_steps":5}
            }),
        ),
        (
            RoomBrowserControllerCommand::ComputerInput {
                action_id: "action-text".into(),
                actor_id: "user:owner-1".into(),
                runtime_generation: 4,
                viewport_revision: 9,
                desktop_pixel_width: 1280,
                desktop_pixel_height: 800,
                action: RoomComputerInputAction::KeyboardText {
                    input: RoomComputerKeyboardInput::new(
                        "sensitive-keyboard-text-世界".to_string(),
                    ),
                },
            },
            serde_json::json!({
                "kind":"computer_input","action_id":"action-text","actor_id":"user:owner-1",
                "runtime_generation":4,"viewport_revision":9,"desktop_pixel_width":1280,
                "desktop_pixel_height":800,"action":{"kind":"keyboard_text",
                "input":"sensitive-keyboard-text-世界"}
            }),
        ),
        (
            RoomBrowserControllerCommand::ComputerInput {
                action_id: "action-key".into(),
                actor_id: "user:owner-1".into(),
                runtime_generation: 4,
                viewport_revision: 9,
                desktop_pixel_width: 1280,
                desktop_pixel_height: 800,
                action: RoomComputerInputAction::KeyboardKey {
                    input: RoomComputerKeyboardInput::new(
                        "ctrl+shift+sensitive-keyboard-key".to_string(),
                    ),
                    repeat: 3,
                },
            },
            serde_json::json!({
                "kind":"computer_input","action_id":"action-key","actor_id":"user:owner-1",
                "runtime_generation":4,"viewport_revision":9,"desktop_pixel_width":1280,
                "desktop_pixel_height":800,"action":{"kind":"keyboard_key",
                "input":"ctrl+shift+sensitive-keyboard-key","repeat":3}
            }),
        ),
        (
            RoomBrowserControllerCommand::ComputerInput {
                action_id: "action-8".into(),
                actor_id: "agent:agent-1".into(),
                runtime_generation: 4,
                viewport_revision: 9,
                desktop_pixel_width: 1280,
                desktop_pixel_height: 800,
                action: RoomComputerInputAction::SecretText {
                    input: RoomComputerSecretInput::new("computer-secret-fixture".into()),
                },
            },
            serde_json::json!({
                "kind":"computer_input",
                "action_id":"action-8",
                "actor_id":"agent:agent-1",
                "runtime_generation":4,
                "viewport_revision":9,
                "desktop_pixel_width":1280,
                "desktop_pixel_height":800,
                "action":{"kind":"secret_text","input":"computer-secret-fixture"}
            }),
        ),
        (
            RoomBrowserControllerCommand::Snapshot {
                target_id: "target-1".into(),
                document_id: "doc-1".into(),
            },
            serde_json::json!({"kind":"snapshot","target_id":"target-1","document_id":"doc-1"}),
        ),
        (
            RoomBrowserControllerCommand::Navigate {
                target_id: "target-1".into(),
                document_id: "doc-1".into(),
                url: crate::runtime::browser_controller_compatibility::BrowserNavigationUrl::new(
                    "https://example.test/path?sensitive-navigation-fixture",
                )
                .unwrap(),
            },
            serde_json::json!({"kind":"navigate","target_id":"target-1","document_id":"doc-1",
                "url":"https://example.test/path?sensitive-navigation-fixture"}),
        ),
        (
            RoomBrowserControllerCommand::Wait {
                target_id: "target-1".into(),
                document_id: "doc-1".into(),
                wait: crate::runtime::browser_controller_compatibility::BrowserCompatibilityWait::Selector(
                    "sensitive-selector-fixture".into(),
                ),
                timeout_ms: 500,
            },
            serde_json::json!({"kind":"wait","target_id":"target-1","document_id":"doc-1",
                "wait":{"kind":"selector","selector":"sensitive-selector-fixture"},"timeout_ms":500}),
        ),
        (
            RoomBrowserControllerCommand::Dialog {
                target_id: "target-1".into(),
                document_id: "doc-1".into(),
                action: crate::runtime::browser_controller_action::BrowserDialogAction::Accept {
                    prompt_text: Some("sensitive-dialog-fixture".into()),
                },
            },
            serde_json::json!({"kind":"dialog","target_id":"target-1","document_id":"doc-1",
                "action":{"kind":"accept","prompt_text":"sensitive-dialog-fixture"}}),
        ),
        (
            RoomBrowserControllerCommand::ConfigureDownloads {
                target_id: "target-1".into(),
                document_id: "doc-1".into(),
            },
            serde_json::json!({"kind":"configure_downloads","target_id":"target-1","document_id":"doc-1"}),
        ),
        (
            RoomBrowserControllerCommand::CancelDownload {
                cancellation: crate::runtime::browser_controller_file_transfer::BrowserDownloadCancellation::new(2, "download-a".into()).unwrap(),
            },
            serde_json::json!({"kind":"cancel_download","cancellation":{"browser_generation":2,"guid":"download-a"}}),
        ),
        (
            RoomBrowserControllerCommand::Upload {
                execution_id: "00000000000000000000000000000001".into(),
                target_id: "target-1".into(),
                document_id: "doc-1".into(),
                node_ref: "backend:1".into(),
                files: crate::runtime::browser_controller_file_transfer::BrowserUploadFiles::new(
                    vec!["/workspace/sensitive-upload-fixture".into()],
                )
                .unwrap(),
            },
            serde_json::json!({"kind":"upload","execution_id":"00000000000000000000000000000001","target_id":"target-1","document_id":"doc-1",
                "node_ref":"backend:1","files":["/workspace/sensitive-upload-fixture"]}),
        ),
        (
            RoomBrowserControllerCommand::RecoverUpload {
                execution_id: "00000000000000000000000000000001".into(),
                target_id: "target-1".into(),
                document_id: "doc-1".into(),
                node_ref: "backend:1".into(),
                files: crate::runtime::browser_controller_file_transfer::BrowserUploadFiles::new(
                    vec!["/workspace/sensitive-upload-fixture".into()],
                ).unwrap(),
            },
            serde_json::json!({"kind":"recover_upload","execution_id":"00000000000000000000000000000001",
                "target_id":"target-1","document_id":"doc-1","node_ref":"backend:1",
                "files":["/workspace/sensitive-upload-fixture"]}),
        ),
        (
            RoomBrowserControllerCommand::Permission {
                target_id: "target-1".into(),
                document_id: "doc-1".into(),
                permission: crate::runtime::browser_controller_permission::BrowserPermissionName::Geolocation,
                setting: crate::runtime::browser_controller_permission::BrowserPermissionSetting::Denied,
            },
            serde_json::json!({"kind":"permission","target_id":"target-1","document_id":"doc-1",
                "permission":"geolocation","setting":"denied"}),
        ),
        (
            RoomBrowserControllerCommand::PollEvents {
                browser_generation: 3,
                cursor: 4,
                limit: 20,
            },
            serde_json::json!({"kind":"poll_events","browser_generation":3,"cursor":4,"limit":20}),
        ),
        (
            RoomBrowserControllerCommand::Acquire,
            serde_json::json!({"kind":"acquire"}),
        ),
        (
            RoomBrowserControllerCommand::Release,
            serde_json::json!({"kind":"release"}),
        ),
        (
            RoomBrowserControllerCommand::Reconcile {
                viewport: crate::session::CanonicalViewport::new(1280, 800, 1, 1280, 800).unwrap(),
            },
            serde_json::json!({"kind":"reconcile","viewport":{
                "css_width":1280,"css_height":800,"device_scale_factor":1,
                "desktop_pixel_width":1280,"desktop_pixel_height":800,
                "revision":1,"last_actor_id":null
            }}),
        ),
    ] {
        let request = RelayPeerRequest::RoomBrowserController {
            session_id: "room-1".into(),
            slice_id: "slice-1".into(),
            command,
        };
        let wire = serde_json::json!({"kind":"room_browser_controller", "session_id":"room-1",
            "slice_id":"slice-1","command":wire_command});
        assert!(
            !format!("{request:?}").contains("sensitive-fill-fixture"),
            "relay diagnostics must not print fill payloads"
        );
        assert!(
            !format!("{request:?}").contains("sensitive-dialog-fixture"),
            "relay diagnostics must not print dialog prompt payloads"
        );
        assert!(
            !format!("{request:?}").contains("sensitive-upload-fixture"),
            "relay diagnostics must not print upload paths"
        );
        assert!(
            !format!("{request:?}").contains("sensitive-navigation-fixture"),
            "relay diagnostics must not print navigation URLs"
        );
        assert!(
            !format!("{request:?}").contains("sensitive-selector-fixture"),
            "relay diagnostics must not print compatibility selectors"
        );
        assert!(
            !format!("{request:?}").contains("sensitive-keyboard"),
            "relay diagnostics must not print keyboard input"
        );
        assert!(
            !format!("{request:?}").contains("sensitive-clipboard"),
            "relay diagnostics must not print clipboard input"
        );
        assert_eq!(serde_json::to_value(&request).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<RelayPeerRequest>(wire).unwrap(),
            request
        );
    }
    for result in [
        serde_json::json!({"kind":"recovery_required","process":{
            "state":"ready","process_id":124,"diagnostic_code":null,
            "runtime_generation":3,"restart_count":2
        }}),
        serde_json::json!({"kind":"cancellation_requested","accepted":true}),
        serde_json::json!({"kind":"cancellation_requested","accepted":false}),
        serde_json::json!({"kind":"action_cancelled","controller_fenced":false}),
        serde_json::json!({"kind":"action_cancelled","controller_fenced":true}),
        serde_json::json!({"kind":"action","result":{
            "browser_generation":1,"target_id":"target-1","document_id":"doc-1",
            "action_kind":"click","dialog_opened":false,"attempts":2,"elapsed_ms":50
        }}),
        serde_json::json!({"kind":"computer_input_applied","action_id":"action-7"}),
        serde_json::json!({"kind":"computer_clipboard","content":"sensitive-clipboard-result"}),
        serde_json::json!({"kind":"snapshot","snapshot":{
            "browser_generation":1,"target_id":"target-1","document_id":"doc-1",
            "snapshot_revision":2,"accessibility_nodes":[],"dom_documents":[{
                "document_index":0,"url":"https://example.test/","owner_node_ref":null
            }],"shadow_roots":[],
            "dom_nodes":[{"node_ref":"backend:1","parent_ref":null,"document_index":0,"node_type":1,"node_name":"BUTTON",
                "text":"","attributes":{},"bounds":{"x":1.5,"y":2.0,"width":3.0,"height":4.0}}]
        }}),
        serde_json::json!({"kind":"tab","result":{
            "browser_generation":1,"target_id":"target-1","document_id":"doc-1",
            "action":"activate"
        }}),
        serde_json::json!({"kind":"navigation","result":{
            "browser_generation":1,"target_id":"target-1","document_id":"doc-2",
            "url":"https://example.test/path?sensitive-navigation-result"
        }}),
        serde_json::json!({"kind":"wait","result":{
            "browser_generation":1,"target_id":"target-1","document_id":"doc-1",
            "kind":"selector","ok":true,"elapsed_ms":7
        }}),
        serde_json::json!({"kind":"dialog","result":{
            "browser_generation":1,"target_id":"target-1","document_id":"doc-1","action":"dismiss"
        }}),
        serde_json::json!({"kind":"downloads","result":{
            "browser_generation":1,"target_id":"target-1","document_id":"doc-1","enabled":true
        }}),
        serde_json::json!({"kind":"upload","result":{
            "browser_generation":1,"target_id":"target-1","document_id":"doc-1","file_count":1,"total_bytes":12
        }}),
        serde_json::json!({"kind":"permission","result":{
            "browser_generation":1,"target_id":"target-1","document_id":"doc-1",
            "permission":"geolocation","setting":"denied"
        }}),
        serde_json::json!({"kind":"events","batch":{
            "browser_generation":3,
            "events":[{"event_id":5,"browser_generation":3,"kind":"network_request",
                "target_id":"target-1","document_id":"doc-1","data":{
                    "request_id":"sensitive-event-fixture","method":"GET","url":"https://example.test/path",
                    "resource_type":"Document"
                }}],
            "next_cursor":5,"replay_gap":false
        }}),
        serde_json::json!({"kind":"process","snapshot":{
            "state":"ready","process_id":123,"diagnostic_code":null,
            "runtime_generation":2,"restart_count":1
        }}),
        serde_json::json!({"kind":"reconciled","reconciliation":{
            "process":{"state":"ready","process_id":123,"diagnostic_code":null,
                "runtime_generation":2,"restart_count":1},
            "browser":{"browser_generation":3,"event_cursor":4,
                "tabs":[{"target_id":"target-1","document_id":"doc-1",
                    "url":"https://example.test/","title":"Example"}],
                "focused_target_id":"target-1","viewport":{
                    "css_width":1280,"css_height":800,"device_scale_factor":1,
                    "desktop_pixel_width":1280,"desktop_pixel_height":800}}
        }}),
    ] {
        let wire = serde_json::json!({"kind":"room_browser_controller", "session_id":"room-1",
            "slice_id":"slice-1","result":result});
        let response: RelayPeerResponse = serde_json::from_value(wire.clone()).unwrap();
        assert!(
            !format!("{response:?}").contains("sensitive-event-fixture"),
            "relay diagnostics must not print browser event data values"
        );
        assert!(
            !format!("{response:?}").contains("sensitive-navigation-result"),
            "relay diagnostics must not print navigation result URLs"
        );
        assert!(
            !format!("{response:?}").contains("sensitive-clipboard-result"),
            "relay diagnostics must not print clipboard contents"
        );
        assert_eq!(serde_json::to_value(response).unwrap(), wire);
    }
    assert!(
        serde_json::from_value::<RelayPeerRequest>(serde_json::json!({
            "kind":"room_browser_controller", "session_id":"room-1", "slice_id":"slice-1",
            "command":{"kind":"upload","target_id":"target-1","document_id":"doc-1",
                "node_ref":"backend:1","files":["relative-path"]}
        }))
        .is_err()
    );

    let context = RemoteExtensionInvocationContext {
        home_kernel_id: "home-kernel".into(),
        home_session_id: "room-1".into(),
        home_agent_id: "agent-1".into(),
        leased_agent_id: "leased-agent-1".into(),
        worker_provider_run_id: "worker-run-1".into(),
        worker_kernel_id: Some("worker-kernel".into()),
        worker_machine_id: Some("worker-machine".into()),
    };
    let forwarded = RelayPeerRequest::ForwardRoomBrowserRuntimeTool {
        context: context.clone(),
        call: RemoteRoomBrowserRuntimeToolCall {
            tool_name: "slice_open_url".into(),
            arguments: serde_json::json!({
                "url":"https://sensitive-worker-forward.test/path?token=secret"
            }),
        },
    };
    let forwarded_wire = serde_json::json!({
        "kind":"forward_room_browser_runtime_tool",
        "context":{
            "home_kernel_id":"home-kernel",
            "home_session_id":"room-1",
            "home_agent_id":"agent-1",
            "leased_agent_id":"leased-agent-1",
            "worker_provider_run_id":"worker-run-1",
            "worker_kernel_id":"worker-kernel",
            "worker_machine_id":"worker-machine"
        },
        "call":{
            "tool_name":"slice_open_url",
            "arguments":{"url":"https://sensitive-worker-forward.test/path?token=secret"}
        }
    });
    assert_eq!(serde_json::to_value(&forwarded).unwrap(), forwarded_wire);
    assert_eq!(
        serde_json::from_value::<RelayPeerRequest>(forwarded_wire).unwrap(),
        forwarded
    );
    assert!(!format!("{forwarded:?}").contains("sensitive-worker-forward"));

    let handled = RelayPeerResponse::RoomBrowserRuntimeToolHandled {
        result: RemoteRoomBrowserRuntimeToolResult(
            crate::transport::runtime_tools::RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({
                    "url":"https://sensitive-worker-result.test/path?token=secret"
                }),
            },
        ),
    };
    let handled_wire = serde_json::json!({
        "kind":"room_browser_runtime_tool_handled",
        "result":{
            "ok":true,
            "payload":{"url":"https://sensitive-worker-result.test/path?token=secret"}
        }
    });
    assert_eq!(serde_json::to_value(&handled).unwrap(), handled_wire);
    assert_eq!(
        serde_json::from_value::<RelayPeerResponse>(handled_wire).unwrap(),
        handled
    );
    assert!(!format!("{handled:?}").contains("sensitive-worker-result"));
}

#[test]
fn computer_secret_and_keyboard_input_debug_output_is_redacted() {
    let command = RoomBrowserControllerCommand::ComputerInput {
        action_id: "action-secret".into(),
        actor_id: "agent:agent-1".into(),
        runtime_generation: 1,
        viewport_revision: 1,
        desktop_pixel_width: 1280,
        desktop_pixel_height: 800,
        action: RoomComputerInputAction::SecretText {
            input: RoomComputerSecretInput::new("must-not-appear-in-debug".into()),
        },
    };

    let debug = format!("{command:?}");
    assert!(debug.contains("[redacted computer secret input]"));
    assert!(!debug.contains("must-not-appear-in-debug"));

    for action in [
        RoomComputerInputAction::KeyboardText {
            input: RoomComputerKeyboardInput::new("must-not-appear-text".into()),
        },
        RoomComputerInputAction::KeyboardKey {
            input: RoomComputerKeyboardInput::new("must-not-appear-key".into()),
            repeat: 1,
        },
    ] {
        let debug = format!("{action:?}");
        assert!(debug.contains("[redacted computer keyboard input]"));
        assert!(!debug.contains("must-not-appear"));
    }
}
