use super::*;
use crate::local::{
    CaptureRoomEnvironmentScreenshotRequest, ReadRoomEnvironmentScreenshotChunkRequest,
    RoomEnvironmentScreenshotArtifact, RoomEnvironmentScreenshotChunk,
};
use crate::session::{
    CanonicalViewport, EnvironmentAction, EnvironmentActionState, EnvironmentActor,
    EnvironmentActorColor, EnvironmentActorKind, EnvironmentActorPresence, EnvironmentComponent,
    EnvironmentComponentHealth, EnvironmentComponentHealthState, EnvironmentEvent,
    EnvironmentEventKind, EnvironmentLifecycle, EnvironmentMode, EnvironmentReplay, EnvironmentTab,
    InputOwnership, InputTarget, RoomEnvironmentSnapshot,
};

#[test]
fn room_environment_screenshot_transfer_shape_is_versioned_and_bounded() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let capture = LocalDaemonRequest::CaptureRoomEnvironmentScreenshot(
        CaptureRoomEnvironmentScreenshotRequest {
            session_id: "session-1".to_string(),
            attachment_id: "attachment-1".to_string(),
        },
    );
    assert_eq!(
        serde_json::to_value(&capture).expect("screenshot capture request should encode"),
        serde_json::json!({"CaptureRoomEnvironmentScreenshot": {
            "session_id": "session-1",
            "attachment_id": "attachment-1"
        }})
    );

    let read = LocalDaemonRequest::ReadRoomEnvironmentScreenshotChunk(
        ReadRoomEnvironmentScreenshotChunkRequest {
            session_id: "session-1".to_string(),
            attachment_id: "attachment-1".to_string(),
            artifact_id: "artifact-1".to_string(),
            offset: 131_072,
            max_bytes: 131_072,
        },
    );
    assert_eq!(
        serde_json::to_value(&read).expect("screenshot chunk request should encode"),
        serde_json::json!({"ReadRoomEnvironmentScreenshotChunk": {
            "session_id": "session-1",
            "attachment_id": "attachment-1",
            "artifact_id": "artifact-1",
            "offset": 131072,
            "max_bytes": 131072
        }})
    );

    let captured = LocalDaemonResponse::RoomEnvironmentScreenshotCaptured {
        artifact: RoomEnvironmentScreenshotArtifact {
            artifact_id: "artifact-1".to_string(),
            sha256: "bef57ec7f53a6d40beb640a780a639c83bc29ac8a9816f1fc6c5c6dcd93c4721".to_string(),
            size_bytes: 6,
            media_type: "image/png".to_string(),
            display_name: "capture.png".to_string(),
        },
    };
    let captured_value =
        serde_json::to_value(&captured).expect("screenshot artifact response should encode");
    assert_eq!(
        serde_json::from_value::<LocalDaemonResponse>(captured_value)
            .expect("screenshot artifact response should decode"),
        captured
    );

    let chunk = LocalDaemonResponse::RoomEnvironmentScreenshotChunk {
        chunk: RoomEnvironmentScreenshotChunk {
            artifact_id: "artifact-1".to_string(),
            offset: 0,
            data_base64: "YWJj".to_string(),
            eof: false,
        },
    };
    let chunk_value =
        serde_json::to_value(&chunk).expect("screenshot chunk response should encode");
    assert_eq!(
        serde_json::from_value::<LocalDaemonResponse>(chunk_value)
            .expect("screenshot chunk response should decode"),
        chunk
    );
}

#[test]
fn room_environment_state_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let request = LocalDaemonRequest::GetRoomEnvironmentState(GetRoomEnvironmentStateRequest {
        session_id: "session-1".to_string(),
    });
    assert_eq!(
        serde_json::to_value(&request).expect("Room Environment state request should encode"),
        serde_json::json!({
            "GetRoomEnvironmentState": {
                "session_id": "session-1"
            }
        })
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
            "GetRoomEnvironmentState": {
                "session_id": "session-1"
            }
        }))
        .expect("Room Environment state request should decode"),
        request
    );

    let response = LocalDaemonResponse::RoomEnvironmentState {
        environment: RoomEnvironmentSnapshot {
            session_id: "session-1".to_string(),
            environment_id: "environment-1".to_string(),
            runtime_generation: 1,
            lifecycle: EnvironmentLifecycle::Stopped,
            health: vec![EnvironmentComponentHealth {
                component: EnvironmentComponent::BrowserController,
                state: EnvironmentComponentHealthState::Unavailable,
                diagnostic_code: None,
            }],
            viewport: CanonicalViewport::new(1280, 800, 2, 2560, 1600)
                .expect("viewport should be valid"),
            actors: vec![EnvironmentActor {
                actor_id: "agent-1".to_string(),
                kind: EnvironmentActorKind::Agent,
                display_label: "Browser agent".to_string(),
                presence: EnvironmentActorPresence::Present,
                presentation_color: EnvironmentActorColor::Blue,
            }],
            pointers: Vec::new(),
            tabs: vec![EnvironmentTab {
                tab_id: "tab-1".to_string(),
                url: "https://example.test/".to_string(),
                title: "Example".to_string(),
                document_revision: 3,
                focused: true,
            }],
            focused_tab_id: Some("tab-1".to_string()),
            actions: vec![
                EnvironmentAction {
                    action_id: "action-1".to_string(),
                    sequence: 1,
                    idempotency_key: Some("idempotency-1".to_string()),
                    actor_id: "agent-1".to_string(),
                    runtime_generation: 1,
                    mode: EnvironmentMode::Browser,
                    kind: "click".to_string(),
                    arguments: None,
                    targets: vec![
                        InputTarget::Desktop,
                        InputTarget::BrowserTab("tab-1".to_string()),
                    ],
                    state: EnvironmentActionState::Running,
                    cancellation_requested: true,
                    submitted_at_ms: 40,
                    started_at_ms: Some(40),
                    finished_at_ms: None,
                    outcome: None,
                },
                EnvironmentAction {
                    action_id: "action-2".to_string(),
                    sequence: 2,
                    idempotency_key: None,
                    actor_id: "agent-1".to_string(),
                    runtime_generation: 1,
                    mode: EnvironmentMode::Browser,
                    kind: "second-click".to_string(),
                    arguments: None,
                    targets: vec![InputTarget::BrowserTab("tab-1".to_string())],
                    state: EnvironmentActionState::Queued,
                    cancellation_requested: false,
                    submitted_at_ms: 41,
                    started_at_ms: None,
                    finished_at_ms: None,
                    outcome: None,
                },
            ],
            input_ownership: vec![InputOwnership {
                target: InputTarget::Desktop,
                actor_id: "agent-1".to_string(),
            }],
            pending_input_takeovers: Vec::new(),
            event_cursor: 0,
        },
    };
    assert_eq!(
        serde_json::to_value(&response).expect("Room Environment state response should encode"),
        serde_json::json!({
            "RoomEnvironmentState": {
                "environment": {
                    "session_id": "session-1",
                    "environment_id": "environment-1",
                    "runtime_generation": 1,
                    "lifecycle": "stopped",
                    "health": [{
                        "component": "browser_controller",
                        "state": "unavailable",
                        "diagnostic_code": null
                    }],
                    "viewport": {
                        "css_width": 1280,
                        "css_height": 800,
                        "device_scale_factor": 2,
                        "desktop_pixel_width": 2560,
                        "desktop_pixel_height": 1600,
                        "revision": 1,
                        "last_actor_id": null
                    },
                    "actors": [{
                        "actor_id": "agent-1",
                        "kind": "agent",
                        "display_label": "Browser agent",
                        "presence": "present",
                        "presentation_color": "blue"
                    }],
                    "pointers": [],
                    "tabs": [{
                        "tab_id": "tab-1",
                        "url": "https://example.test/",
                        "title": "Example",
                        "document_revision": 3,
                        "focused": true
                    }],
                    "focused_tab_id": "tab-1",
                    "actions": [
                        {
                            "action_id": "action-1",
                            "sequence": 1,
                            "idempotency_key": "idempotency-1",
                            "actor_id": "agent-1",
                            "runtime_generation": 1,
                            "mode": "browser",
                            "kind": "click",
                            "targets": [
                                {
                                    "kind": "desktop"
                                },
                                {
                                    "kind": "browser_tab",
                                    "id": "tab-1"
                                }
                            ],
                            "state": "running",
                            "cancellation_requested": true,
                            "submitted_at_ms": 40,
                            "started_at_ms": 40,
                            "finished_at_ms": null,
                            "outcome": null
                        },
                        {
                            "action_id": "action-2",
                            "sequence": 2,
                            "idempotency_key": null,
                            "actor_id": "agent-1",
                            "runtime_generation": 1,
                            "mode": "browser",
                            "kind": "second-click",
                            "targets": [{
                                "kind": "browser_tab",
                                "id": "tab-1"
                            }],
                            "state": "queued",
                            "cancellation_requested": false,
                            "submitted_at_ms": 41,
                            "started_at_ms": null,
                            "finished_at_ms": null,
                            "outcome": null
                        }
                    ],
                    "input_ownership": [{
                        "target": {
                            "kind": "desktop"
                        },
                        "actor_id": "agent-1"
                    }],
                    "pending_input_takeovers": [],
                    "event_cursor": 0
                }
            }
        })
    );

    let mut previous_protocol_value =
        serde_json::to_value(&response).expect("Room Environment response should encode");
    previous_protocol_value
        .pointer_mut("/RoomEnvironmentState/environment")
        .and_then(serde_json::Value::as_object_mut)
        .expect("Room Environment snapshot should be an object")
        .remove("pending_input_takeovers");
    previous_protocol_value
        .pointer_mut("/RoomEnvironmentState/environment")
        .and_then(serde_json::Value::as_object_mut)
        .expect("Room Environment snapshot should be an object")
        .remove("pointers");
    previous_protocol_value
        .pointer_mut("/RoomEnvironmentState/environment/actors/0")
        .and_then(serde_json::Value::as_object_mut)
        .expect("Room Environment Actor should be an object")
        .remove("presentation_color");
    previous_protocol_value
        .pointer_mut("/RoomEnvironmentState/environment/actions/0")
        .and_then(serde_json::Value::as_object_mut)
        .expect("Room Environment Action should be an object")
        .remove("sequence");
    previous_protocol_value
        .pointer_mut("/RoomEnvironmentState/environment/actions/0")
        .and_then(serde_json::Value::as_object_mut)
        .expect("Room Environment Action should be an object")
        .remove("cancellation_requested");
    for field in [
        "submitted_at_ms",
        "started_at_ms",
        "finished_at_ms",
        "outcome",
    ] {
        previous_protocol_value
            .pointer_mut("/RoomEnvironmentState/environment/actions/0")
            .and_then(serde_json::Value::as_object_mut)
            .expect("Room Environment Action should be an object")
            .remove(field);
    }
    let LocalDaemonResponse::RoomEnvironmentState { environment } =
        serde_json::from_value(previous_protocol_value)
            .expect("pre-v272 snapshots should decode with no pending takeovers")
    else {
        panic!("expected Room Environment state response");
    };
    assert!(environment.pending_input_takeovers.is_empty());
    assert!(environment.pointers.is_empty());
    assert_eq!(
        environment.actors[0].presentation_color,
        EnvironmentActorColor::Slate
    );
    assert_eq!(environment.actions[0].sequence, 0);
    assert!(!environment.actions[0].cancellation_requested);
    assert_eq!(environment.actions[0].submitted_at_ms, 0);
    assert_eq!(environment.actions[0].started_at_ms, None);
    assert_eq!(environment.actions[0].finished_at_ms, None);
    assert_eq!(environment.actions[0].outcome, None);

    assert_eq!(
        serde_json::to_value(crate::session::EnvironmentActionOutcome::Failed {
            code: crate::session::EnvironmentActionFailureCode::ProcessLost,
        })
        .expect("redacted Action failure should encode"),
        serde_json::json!({
            "status": "failed",
            "code": "process_lost",
        })
    );
    assert_eq!(
        serde_json::to_value(crate::session::EnvironmentActionOutcome::Cancelled {
            reason: crate::session::EnvironmentActionCancellationReason::HumanTakeover,
        })
        .expect("redacted Action cancellation should encode"),
        serde_json::json!({
            "status": "cancelled",
            "reason": "human_takeover",
        })
    );
}

#[test]
fn room_environment_event_replay_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let request = LocalDaemonRequest::GetRoomEnvironmentEvents(GetRoomEnvironmentEventsRequest {
        session_id: "session-1".to_string(),
        cursor: 41,
    });
    assert_eq!(
        serde_json::to_value(&request).expect("Room Environment event request should encode"),
        serde_json::json!({
            "GetRoomEnvironmentEvents": {
                "session_id": "session-1",
                "cursor": 41,
            }
        })
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
            "GetRoomEnvironmentEvents": {
                "session_id": "session-1",
                "cursor": 41,
            }
        }))
        .expect("Room Environment event request should decode"),
        request
    );

    let response = LocalDaemonResponse::RoomEnvironmentEvents {
        replay: EnvironmentReplay::Events {
            events: vec![EnvironmentEvent {
                event_id: 42,
                environment_id: "environment-1".to_string(),
                runtime_generation: 3,
                kind: EnvironmentEventKind::ViewportChanged { revision: 7 },
            }],
            next_cursor: 42,
        },
    };
    let value = serde_json::json!({
        "RoomEnvironmentEvents": {
            "replay": {
                "Events": {
                    "events": [{
                        "event_id": 42,
                        "environment_id": "environment-1",
                        "runtime_generation": 3,
                        "kind": {
                            "ViewportChanged": {
                                "revision": 7,
                            }
                        }
                    }],
                    "next_cursor": 42,
                }
            }
        }
    });
    assert_eq!(
        serde_json::to_value(&response).expect("Room Environment event replay should encode"),
        value
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonResponse>(value)
            .expect("Room Environment event replay should decode"),
        response
    );
    assert_eq!(
        serde_json::from_value::<EnvironmentEventKind>(serde_json::json!({
            "ActionChanged": {
                "action_id": "action-1",
                "state": "running",
            }
        }))
        .expect("pre-v276 Action events should decode"),
        EnvironmentEventKind::ActionChanged {
            action_id: "action-1".to_string(),
            state: EnvironmentActionState::Running,
            cancellation_requested: false,
            submitted_at_ms: 0,
            started_at_ms: None,
            finished_at_ms: None,
            outcome: None,
        }
    );
    assert_eq!(
        serde_json::to_value(EnvironmentEventKind::ActionChanged {
            action_id: "action-2".to_string(),
            state: EnvironmentActionState::Completed,
            cancellation_requested: false,
            submitted_at_ms: 40,
            started_at_ms: Some(41),
            finished_at_ms: Some(44),
            outcome: Some(crate::session::EnvironmentActionOutcome::Completed),
        })
        .expect("v278 Action event should encode"),
        serde_json::json!({
            "ActionChanged": {
                "action_id": "action-2",
                "state": "completed",
                "cancellation_requested": false,
                "submitted_at_ms": 40,
                "started_at_ms": 41,
                "finished_at_ms": 44,
                "outcome": { "status": "completed" },
            }
        })
    );
}

#[test]
fn room_environment_action_history_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let request = LocalDaemonRequest::ListRoomEnvironmentActionHistory(
        ListRoomEnvironmentActionHistoryRequest {
            session_id: "session-1".to_string(),
            before_sequence: Some(42),
            limit: Some(25),
        },
    );
    let request_value = serde_json::json!({
        "ListRoomEnvironmentActionHistory": {
            "session_id": "session-1",
            "before_sequence": 42,
            "limit": 25,
        }
    });
    assert_eq!(
        serde_json::to_value(&request).expect("Room Environment history request should encode"),
        request_value
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(request_value)
            .expect("Room Environment history request should decode"),
        request
    );

    let response = LocalDaemonResponse::RoomEnvironmentActionHistoryListed {
        page: crate::session::EnvironmentActionHistoryPage {
            actions: vec![EnvironmentAction {
                action_id: "action-41".to_string(),
                sequence: 41,
                idempotency_key: None,
                actor_id: "agent-1".to_string(),
                runtime_generation: 2,
                mode: EnvironmentMode::Computer,
                kind: "key-chord".to_string(),
                arguments: None,
                targets: vec![InputTarget::Desktop],
                state: EnvironmentActionState::Completed,
                cancellation_requested: false,
                submitted_at_ms: 100,
                started_at_ms: Some(101),
                finished_at_ms: Some(102),
                outcome: Some(crate::session::EnvironmentActionOutcome::Completed),
            }],
            next_before_sequence: Some(41),
        },
    };
    let response_value = serde_json::json!({
        "RoomEnvironmentActionHistoryListed": {
            "page": {
                "actions": [{
                    "action_id": "action-41",
                    "sequence": 41,
                    "idempotency_key": null,
                    "actor_id": "agent-1",
                    "runtime_generation": 2,
                    "mode": "computer",
                    "kind": "key-chord",
                    "targets": [{ "kind": "desktop" }],
                    "state": "completed",
                    "cancellation_requested": false,
                    "submitted_at_ms": 100,
                    "started_at_ms": 101,
                    "finished_at_ms": 102,
                    "outcome": { "status": "completed" },
                }],
                "next_before_sequence": 41,
            }
        }
    });
    assert_eq!(
        serde_json::to_value(&response).expect("Room Environment history should encode"),
        response_value
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonResponse>(response_value)
            .expect("Room Environment history should decode"),
        response
    );
}

#[test]
fn room_environment_start_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let request = LocalDaemonRequest::StartRoomEnvironment(StartRoomEnvironmentRequest {
        session_id: "session-1".to_string(),
        viewport: RoomEnvironmentViewportRequest {
            css_width: 1280,
            css_height: 800,
            device_scale_factor: 2,
            desktop_pixel_width: 2560,
            desktop_pixel_height: 1600,
        },
    });
    let value = serde_json::json!({
        "StartRoomEnvironment": {
            "session_id": "session-1",
            "viewport": {
                "css_width": 1280,
                "css_height": 800,
                "device_scale_factor": 2,
                "desktop_pixel_width": 2560,
                "desktop_pixel_height": 1600
            }
        }
    });
    assert_eq!(
        serde_json::to_value(&request).expect("Room Environment start request should encode"),
        value
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(value)
            .expect("Room Environment start request should decode"),
        request
    );

    let response = LocalDaemonResponse::RoomEnvironmentUpdated {
        environment: RoomEnvironmentSnapshot {
            session_id: "session-1".to_string(),
            environment_id: "environment-session-1".to_string(),
            runtime_generation: 1,
            lifecycle: EnvironmentLifecycle::Starting,
            health: Vec::new(),
            viewport: CanonicalViewport::new(1280, 800, 2, 2560, 1600)
                .expect("viewport should be valid"),
            actors: Vec::new(),
            pointers: Vec::new(),
            tabs: Vec::new(),
            focused_tab_id: None,
            actions: Vec::new(),
            input_ownership: Vec::new(),
            pending_input_takeovers: Vec::new(),
            event_cursor: 1,
        },
    };
    assert_eq!(
        serde_json::to_value(response).expect("Room Environment start response should encode"),
        serde_json::json!({
            "RoomEnvironmentUpdated": {
                "environment": {
                    "session_id": "session-1",
                    "environment_id": "environment-session-1",
                    "runtime_generation": 1,
                    "lifecycle": "starting",
                    "health": [],
                    "viewport": {
                        "css_width": 1280,
                        "css_height": 800,
                        "device_scale_factor": 2,
                        "desktop_pixel_width": 2560,
                        "desktop_pixel_height": 1600,
                        "revision": 1,
                        "last_actor_id": null
                    },
                    "actors": [],
                    "pointers": [],
                    "tabs": [],
                    "focused_tab_id": null,
                    "actions": [],
                    "input_ownership": [],
                    "pending_input_takeovers": [],
                    "event_cursor": 1
                }
            }
        })
    );
}

#[test]
fn room_environment_stop_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let request = LocalDaemonRequest::StopRoomEnvironment(StopRoomEnvironmentRequest {
        session_id: "session-1".to_string(),
    });
    let value = serde_json::json!({
        "StopRoomEnvironment": {
            "session_id": "session-1"
        }
    });
    assert_eq!(
        serde_json::to_value(&request).expect("Room Environment stop request should encode"),
        value
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(value)
            .expect("Room Environment stop request should decode"),
        request
    );
}

#[test]
fn room_environment_retry_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let request = LocalDaemonRequest::RetryRoomEnvironment(RetryRoomEnvironmentRequest {
        session_id: "session-1".to_string(),
    });
    let value = serde_json::json!({
        "RetryRoomEnvironment": {
            "session_id": "session-1"
        }
    });
    assert_eq!(
        serde_json::to_value(&request).expect("Room Environment retry request should encode"),
        value
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(value)
            .expect("Room Environment retry request should decode"),
        request
    );
}

#[test]
fn room_environment_viewport_update_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let request =
        LocalDaemonRequest::UpdateRoomEnvironmentViewport(UpdateRoomEnvironmentViewportRequest {
            session_id: "session-1".to_string(),
            expected_revision: 4,
            viewport: RoomEnvironmentViewportRequest {
                css_width: 1440,
                css_height: 900,
                device_scale_factor: 2,
                desktop_pixel_width: 2880,
                desktop_pixel_height: 1800,
            },
        });
    let value = serde_json::json!({
        "UpdateRoomEnvironmentViewport": {
            "session_id": "session-1",
            "expected_revision": 4,
            "viewport": {
                "css_width": 1440,
                "css_height": 900,
                "device_scale_factor": 2,
                "desktop_pixel_width": 2880,
                "desktop_pixel_height": 1800
            }
        }
    });
    assert_eq!(
        serde_json::to_value(&request).expect("viewport request should encode"),
        value
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(value)
            .expect("viewport request should decode"),
        request
    );
}

#[test]
fn room_environment_pointer_update_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let request =
        LocalDaemonRequest::UpdateRoomEnvironmentPointer(UpdateRoomEnvironmentPointerRequest {
            session_id: "session-1".to_string(),
            runtime_generation: 3,
            viewport_revision: 7,
            pointer: Some(RoomEnvironmentPointerPositionRequest { x: 320, y: 180 }),
        });
    let value = serde_json::json!({
        "UpdateRoomEnvironmentPointer": {
            "session_id": "session-1",
            "runtime_generation": 3,
            "viewport_revision": 7,
            "pointer": {
                "x": 320,
                "y": 180
            }
        }
    });
    assert_eq!(
        serde_json::to_value(&request).expect("pointer presence request should encode"),
        value
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(value)
            .expect("pointer presence request should decode"),
        request
    );
    assert_eq!(
        serde_json::to_value(EnvironmentEventKind::PointersChanged)
            .expect("pointer presence event should encode"),
        serde_json::json!("PointersChanged")
    );
}

#[test]
fn room_environment_takeover_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let request = LocalDaemonRequest::RequestRoomEnvironmentInputTakeover(
        RequestRoomEnvironmentInputTakeoverRequest {
            session_id: "session-1".to_string(),
            target: InputTarget::Desktop,
        },
    );
    let request_value = serde_json::json!({
        "RequestRoomEnvironmentInputTakeover": {
            "session_id": "session-1",
            "target": {
                "kind": "desktop"
            }
        }
    });
    assert_eq!(
        serde_json::to_value(&request).expect("takeover request should encode"),
        request_value
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(request_value)
            .expect("takeover request should decode"),
        request
    );

    let response = LocalDaemonResponse::RoomEnvironmentTakeoverUpdated {
        outcome: crate::session::TakeoverOutcome::Granted,
        environment: RoomEnvironmentSnapshot {
            session_id: "session-1".to_string(),
            environment_id: "environment-session-1".to_string(),
            runtime_generation: 1,
            lifecycle: EnvironmentLifecycle::Ready,
            health: Vec::new(),
            viewport: CanonicalViewport::new(1280, 800, 1, 1280, 800)
                .expect("viewport should be valid"),
            actors: Vec::new(),
            pointers: Vec::new(),
            tabs: Vec::new(),
            focused_tab_id: None,
            actions: Vec::new(),
            input_ownership: Vec::new(),
            pending_input_takeovers: Vec::new(),
            event_cursor: 2,
        },
    };
    assert_eq!(
        serde_json::to_value(response).expect("takeover response should encode"),
        serde_json::json!({
            "RoomEnvironmentTakeoverUpdated": {
                "outcome": {
                    "state": "granted"
                },
                "environment": {
                    "session_id": "session-1",
                    "environment_id": "environment-session-1",
                    "runtime_generation": 1,
                    "lifecycle": "ready",
                    "health": [],
                    "viewport": {
                        "css_width": 1280,
                        "css_height": 800,
                        "device_scale_factor": 1,
                        "desktop_pixel_width": 1280,
                        "desktop_pixel_height": 800,
                        "revision": 1,
                        "last_actor_id": null
                    },
                    "actors": [],
                    "pointers": [],
                    "tabs": [],
                    "focused_tab_id": null,
                    "actions": [],
                    "input_ownership": [],
                    "pending_input_takeovers": [],
                    "event_cursor": 2
                }
            }
        })
    );
}

#[test]
fn room_environment_input_release_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let request =
        LocalDaemonRequest::ReleaseRoomEnvironmentInput(ReleaseRoomEnvironmentInputRequest {
            session_id: "session-1".to_string(),
            target: InputTarget::Desktop,
        });
    let request_value = serde_json::json!({
        "ReleaseRoomEnvironmentInput": {
            "session_id": "session-1",
            "target": {
                "kind": "desktop"
            }
        }
    });
    assert_eq!(
        serde_json::to_value(&request).expect("input release request should encode"),
        request_value
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(request_value)
            .expect("input release request should decode"),
        request
    );

    let response = LocalDaemonResponse::RoomEnvironmentInputReleased {
        environment: RoomEnvironmentSnapshot {
            session_id: "session-1".to_string(),
            environment_id: "environment-session-1".to_string(),
            runtime_generation: 1,
            lifecycle: EnvironmentLifecycle::Ready,
            health: Vec::new(),
            viewport: CanonicalViewport::new(1280, 800, 1, 1280, 800)
                .expect("viewport should be valid"),
            actors: Vec::new(),
            pointers: Vec::new(),
            tabs: Vec::new(),
            focused_tab_id: None,
            actions: Vec::new(),
            input_ownership: Vec::new(),
            pending_input_takeovers: Vec::new(),
            event_cursor: 3,
        },
    };
    let response_value =
        serde_json::to_value(&response).expect("input release response should encode");
    assert_eq!(
        response_value.pointer("/RoomEnvironmentInputReleased/environment/event_cursor"),
        Some(&serde_json::json!(3))
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonResponse>(response_value)
            .expect("input release response should decode"),
        response
    );
}

#[test]
fn room_environment_action_cancellation_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let request =
        LocalDaemonRequest::CancelRoomEnvironmentAction(CancelRoomEnvironmentActionRequest {
            session_id: "session-1".to_string(),
            action_id: "action-7".to_string(),
        });
    let request_value = serde_json::json!({
        "CancelRoomEnvironmentAction": {
            "session_id": "session-1",
            "action_id": "action-7"
        }
    });
    assert_eq!(
        serde_json::to_value(&request).expect("Action cancellation request should encode"),
        request_value
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(request_value)
            .expect("Action cancellation request should decode"),
        request
    );

    let response = LocalDaemonResponse::RoomEnvironmentActionCancellationUpdated {
        outcome: crate::session::ActionCancellationOutcome::CancellationRequested,
        environment: RoomEnvironmentSnapshot {
            session_id: "session-1".to_string(),
            environment_id: "environment-session-1".to_string(),
            runtime_generation: 1,
            lifecycle: EnvironmentLifecycle::Ready,
            health: Vec::new(),
            viewport: CanonicalViewport::new(1280, 800, 1, 1280, 800)
                .expect("viewport should be valid"),
            actors: Vec::new(),
            pointers: Vec::new(),
            tabs: Vec::new(),
            focused_tab_id: None,
            actions: Vec::new(),
            input_ownership: Vec::new(),
            pending_input_takeovers: Vec::new(),
            event_cursor: 4,
        },
    };
    let response_value =
        serde_json::to_value(&response).expect("Action cancellation response should encode");
    assert_eq!(
        response_value.pointer("/RoomEnvironmentActionCancellationUpdated/outcome/state"),
        Some(&serde_json::json!("cancellation_requested"))
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonResponse>(response_value)
            .expect("Action cancellation response should decode"),
        response
    );
}

#[test]
fn room_environment_action_submission_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let request =
        LocalDaemonRequest::SubmitRoomEnvironmentAction(SubmitRoomEnvironmentActionRequest {
            session_id: "session-1".to_string(),
            runtime_generation: 4,
            viewport_revision: 9,
            idempotency_key: "input-1".to_string(),
            action: RoomEnvironmentHumanAction::PointerClick {
                x: 320,
                y: 180,
                button: RoomEnvironmentPointerButton::Left,
                click_count: 1,
            },
        });
    let request_value = serde_json::json!({
        "SubmitRoomEnvironmentAction": {
            "session_id": "session-1",
            "runtime_generation": 4,
            "viewport_revision": 9,
            "idempotency_key": "input-1",
            "action": {
                "kind": "pointer_click",
                "x": 320,
                "y": 180,
                "button": "left",
                "click_count": 1
            }
        }
    });
    assert_eq!(
        serde_json::to_value(&request).expect("Action submission request should encode"),
        request_value
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(request_value)
            .expect("Action submission request should decode"),
        request
    );

    let response = LocalDaemonResponse::RoomEnvironmentActionSubmitted {
        action_id: "action-7".to_string(),
        environment: RoomEnvironmentSnapshot {
            session_id: "session-1".to_string(),
            environment_id: "environment-session-1".to_string(),
            runtime_generation: 4,
            lifecycle: EnvironmentLifecycle::Ready,
            health: Vec::new(),
            viewport: CanonicalViewport::new(1280, 800, 1, 1280, 800)
                .expect("viewport should be valid"),
            actors: Vec::new(),
            pointers: Vec::new(),
            tabs: Vec::new(),
            focused_tab_id: None,
            actions: vec![EnvironmentAction {
                action_id: "action-7".to_string(),
                sequence: 7,
                idempotency_key: Some("input-1".to_string()),
                actor_id: "user:owner-1".to_string(),
                runtime_generation: 4,
                mode: EnvironmentMode::Computer,
                kind: "pointer_click".to_string(),
                arguments: Some(crate::session::EnvironmentActionArguments::PointerClick {
                    x: 320,
                    y: 180,
                    button: crate::session::EnvironmentPointerButton::Left,
                    click_count: 1,
                    viewport_revision: 9,
                }),
                targets: vec![InputTarget::Desktop],
                state: EnvironmentActionState::Completed,
                cancellation_requested: false,
                submitted_at_ms: 100,
                started_at_ms: Some(101),
                finished_at_ms: Some(102),
                outcome: Some(crate::session::EnvironmentActionOutcome::Completed),
            }],
            input_ownership: Vec::new(),
            pending_input_takeovers: Vec::new(),
            event_cursor: 5,
        },
    };
    let response_value =
        serde_json::to_value(&response).expect("Action submission response should encode");
    assert_eq!(
        response_value.pointer("/RoomEnvironmentActionSubmitted/action_id"),
        Some(&serde_json::json!("action-7"))
    );
    assert_eq!(
        response_value.pointer("/RoomEnvironmentActionSubmitted/environment/actions/0/arguments"),
        Some(&serde_json::json!({
            "kind": "pointer_click",
            "x": 320,
            "y": 180,
            "button": "left",
            "click_count": 1,
            "viewport_revision": 9,
        }))
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonResponse>(response_value)
            .expect("Action submission response should decode"),
        response
    );
}

#[test]
fn room_environment_browser_history_submission_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    for (action, wire_action) in [
        (RoomEnvironmentBrowserHistoryAction::Back, "back"),
        (RoomEnvironmentBrowserHistoryAction::Forward, "forward"),
        (RoomEnvironmentBrowserHistoryAction::Reload, "reload"),
    ] {
        let request = LocalDaemonRequest::SubmitRoomEnvironmentBrowserAction(
            SubmitRoomEnvironmentBrowserActionRequest {
                session_id: "session-1".to_string(),
                runtime_generation: 4,
                idempotency_key: format!("history-{wire_action}"),
                action: RoomEnvironmentHumanBrowserAction::History {
                    tab_id: "tab-7".to_string(),
                    action,
                },
            },
        );
        let request_value = serde_json::json!({
            "SubmitRoomEnvironmentBrowserAction": {
                "session_id": "session-1",
                "runtime_generation": 4,
                "idempotency_key": format!("history-{wire_action}"),
                "action": {
                    "kind": "history",
                    "tab_id": "tab-7",
                    "action": wire_action
                }
            }
        });
        assert_eq!(serde_json::to_value(&request).unwrap(), request_value);
        assert_eq!(
            serde_json::from_value::<LocalDaemonRequest>(request_value).unwrap(),
            request
        );
    }
}

#[test]
fn room_environment_browser_tab_submission_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    for (action, wire_action) in [
        (RoomEnvironmentBrowserTabAction::Activate, "activate"),
        (RoomEnvironmentBrowserTabAction::Close, "close"),
    ] {
        let request = LocalDaemonRequest::SubmitRoomEnvironmentBrowserAction(
            SubmitRoomEnvironmentBrowserActionRequest {
                session_id: "session-1".to_string(),
                runtime_generation: 4,
                idempotency_key: format!("tab-{wire_action}"),
                action: RoomEnvironmentHumanBrowserAction::Tab {
                    tab_id: "tab-7".to_string(),
                    action,
                },
            },
        );
        let request_value = serde_json::json!({
            "SubmitRoomEnvironmentBrowserAction": {
                "session_id": "session-1",
                "runtime_generation": 4,
                "idempotency_key": format!("tab-{wire_action}"),
                "action": {
                    "kind": "tab",
                    "tab_id": "tab-7",
                    "action": wire_action
                }
            }
        });
        assert_eq!(serde_json::to_value(&request).unwrap(), request_value);
        assert_eq!(
            serde_json::from_value::<LocalDaemonRequest>(request_value).unwrap(),
            request
        );
    }
}

#[test]
fn room_environment_clipboard_shapes_are_redacted_and_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let content = "sensitive clipboard 世界";
    let request =
        LocalDaemonRequest::SubmitRoomEnvironmentAction(SubmitRoomEnvironmentActionRequest {
            session_id: "session-1".to_string(),
            runtime_generation: 4,
            viewport_revision: 9,
            idempotency_key: "clipboard-1".to_string(),
            action: RoomEnvironmentHumanAction::ClipboardWrite {
                text: RoomEnvironmentClipboardText::new(content.to_string()),
            },
        });
    let request_value = serde_json::json!({
        "SubmitRoomEnvironmentAction": {
            "session_id": "session-1",
            "runtime_generation": 4,
            "viewport_revision": 9,
            "idempotency_key": "clipboard-1",
            "action": {"kind": "clipboard_write", "text": content}
        }
    });
    assert_eq!(serde_json::to_value(&request).unwrap(), request_value);
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(request_value).unwrap(),
        request
    );
    assert!(!format!("{request:?}").contains(content));

    let read =
        LocalDaemonRequest::ReadRoomEnvironmentClipboard(ReadRoomEnvironmentClipboardRequest {
            session_id: "session-1".to_string(),
            runtime_generation: 4,
        });
    let read_value = serde_json::json!({
        "ReadRoomEnvironmentClipboard": {
            "session_id": "session-1",
            "runtime_generation": 4
        }
    });
    assert_eq!(serde_json::to_value(&read).unwrap(), read_value);
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(read_value).unwrap(),
        read
    );

    let response = LocalDaemonResponse::RoomEnvironmentClipboardRead {
        content: RoomEnvironmentClipboardText::new(content.to_string()),
    };
    let response_value = serde_json::json!({
        "RoomEnvironmentClipboardRead": {"content": content}
    });
    assert_eq!(serde_json::to_value(&response).unwrap(), response_value);
    assert_eq!(
        serde_json::from_value::<LocalDaemonResponse>(response_value).unwrap(),
        response
    );
    assert!(!format!("{response:?}").contains(content));
}

#[test]
fn clipboard_protocol_addition_keeps_v302_room_state_requests_compatible() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let prior_wire = serde_json::json!({
        "GetRoomEnvironmentState": {
            "session_id": "session-1"
        }
    });
    let request = serde_json::from_value::<LocalDaemonRequest>(prior_wire.clone())
        .expect("a v302 Room state request should remain valid after the clipboard addition");
    assert_eq!(
        request,
        LocalDaemonRequest::GetRoomEnvironmentState(GetRoomEnvironmentStateRequest {
            session_id: "session-1".to_string(),
        })
    );
    assert_eq!(serde_json::to_value(request).unwrap(), prior_wire);
}

#[test]
fn room_environment_complete_human_input_shapes_are_versioned_and_keyboard_history_is_redacted() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    for (action, wire) in [
        (
            RoomEnvironmentHumanAction::PointerMove { x: 640, y: 400 },
            serde_json::json!({"kind":"pointer_move","x":640,"y":400}),
        ),
        (
            RoomEnvironmentHumanAction::PointerDrag {
                from_x: 120,
                from_y: 160,
                to_x: 720,
                to_y: 560,
                button: RoomEnvironmentPointerButton::Left,
            },
            serde_json::json!({"kind":"pointer_drag","from_x":120,"from_y":160,
                "to_x":720,"to_y":560,"button":"left"}),
        ),
        (
            RoomEnvironmentHumanAction::PointerScroll {
                x: 640,
                y: 400,
                horizontal_steps: -3,
                vertical_steps: 5,
            },
            serde_json::json!({"kind":"pointer_scroll","x":640,"y":400,
                "horizontal_steps":-3,"vertical_steps":5}),
        ),
        (
            RoomEnvironmentHumanAction::KeyboardText {
                text: RoomEnvironmentKeyboardInput::new("sensitive-keyboard-text-世界".to_string()),
            },
            serde_json::json!({"kind":"keyboard_text","text":"sensitive-keyboard-text-世界"}),
        ),
        (
            RoomEnvironmentHumanAction::KeyboardKey {
                key: RoomEnvironmentKeyboardInput::new(
                    "ctrl+shift+sensitive-keyboard-key".to_string(),
                ),
                repeat: 3,
            },
            serde_json::json!({"kind":"keyboard_key",
                "key":"ctrl+shift+sensitive-keyboard-key","repeat":3}),
        ),
    ] {
        assert_eq!(
            serde_json::to_value(&action).expect("human input should encode"),
            wire
        );
        assert_eq!(
            serde_json::from_value::<RoomEnvironmentHumanAction>(wire)
                .expect("human input should decode"),
            action
        );
        assert!(
            !format!("{action:?}").contains("sensitive-keyboard"),
            "local diagnostics must not print keyboard input"
        );
    }

    for (arguments, wire) in [
        (
            crate::session::EnvironmentActionArguments::PointerMove {
                x: 640,
                y: 400,
                viewport_revision: 9,
            },
            serde_json::json!({"kind":"pointer_move","x":640,"y":400,
                "viewport_revision":9}),
        ),
        (
            crate::session::EnvironmentActionArguments::PointerDrag {
                from_x: 120,
                from_y: 160,
                to_x: 720,
                to_y: 560,
                button: crate::session::EnvironmentPointerButton::Left,
                viewport_revision: 9,
            },
            serde_json::json!({"kind":"pointer_drag","from_x":120,"from_y":160,
                "to_x":720,"to_y":560,"button":"left","viewport_revision":9}),
        ),
        (
            crate::session::EnvironmentActionArguments::PointerScroll {
                x: 640,
                y: 400,
                horizontal_steps: -3,
                vertical_steps: 5,
                viewport_revision: 9,
            },
            serde_json::json!({"kind":"pointer_scroll","x":640,"y":400,
                "horizontal_steps":-3,"vertical_steps":5,"viewport_revision":9}),
        ),
        (
            crate::session::EnvironmentActionArguments::KeyboardText {
                utf8_byte_count: 14,
                character_count: 8,
            },
            serde_json::json!({"kind":"keyboard_text","utf8_byte_count":14,
                "character_count":8}),
        ),
        (
            crate::session::EnvironmentActionArguments::KeyboardKey { repeat: 3 },
            serde_json::json!({"kind":"keyboard_key","repeat":3}),
        ),
    ] {
        assert_eq!(
            serde_json::to_value(&arguments).expect("Action arguments should encode"),
            wire
        );
        assert_eq!(
            serde_json::from_value::<crate::session::EnvironmentActionArguments>(wire)
                .expect("Action arguments should decode"),
            arguments
        );
    }
}
