use super::*;
use crate::local::{BindRoomEnvironmentSliceRequest, RoomEnvironmentSliceBinding};

#[test]
fn room_environment_placement_shapes_are_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 309);
    let request = LocalDaemonRequest::BindRoomEnvironmentSlice(BindRoomEnvironmentSliceRequest {
        session_id: "session-1".into(),
        slice_ref: "desktop".into(),
    });
    let wire = serde_json::json!({"BindRoomEnvironmentSlice": {
        "session_id": "session-1", "slice_ref": "desktop"
    }});
    assert_eq!(serde_json::to_value(&request).unwrap(), wire);
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(wire).unwrap(),
        request
    );
    let response = LocalDaemonResponse::RoomEnvironmentSlice {
        binding: Some(RoomEnvironmentSliceBinding {
            session_id: "session-1".into(),
            slice_id: "slice-1".into(),
            owner_kernel_id: "home".into(),
            worker_kernel_ref: "slice:desktop".into(),
        }),
    };
    let wire = serde_json::json!({"RoomEnvironmentSlice":{"binding":{
        "session_id":"session-1", "slice_id":"slice-1",
        "owner_kernel_id":"home", "worker_kernel_ref":"slice:desktop"
    }}});
    assert_eq!(serde_json::to_value(&response).unwrap(), wire);
    assert_eq!(
        serde_json::from_value::<LocalDaemonResponse>(wire).unwrap(),
        response
    );
    assert_eq!(
        serde_json::to_value(LocalDaemonResponse::RoomEnvironmentSlice { binding: None }).unwrap(),
        serde_json::json!({"RoomEnvironmentSlice":{"binding":null}})
    );
}
