use super::*;

#[test]
fn slice_creation_preserves_explicit_display_backend_on_the_wire() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 310);
    let request: LocalDaemonRequest = serde_json::from_value(serde_json::json!({
        "CreateSlice": {
            "name": "headed", "display_mode": "headed", "display_backend": "selkies"
        }
    }))
    .expect("Selkies slice request should decode");
    let serialized = serde_json::to_value(request).expect("request should encode");
    assert_eq!(
        serialized.pointer("/CreateSlice/display_backend"),
        Some(&serde_json::json!("selkies"))
    );
}

#[test]
fn legacy_slice_requests_keep_novnc_and_unknown_backends_fail_closed() {
    let request: crate::local::CreateSliceRequest =
        serde_json::from_value(serde_json::json!({"name": "legacy", "display_mode": "headed"}))
            .expect("legacy request should decode");
    assert_eq!(
        request.display_backend,
        crate::slice::SliceDisplayBackend::Novnc
    );
    assert!(serde_json::to_value(request)
        .unwrap()
        .get("display_backend")
        .is_none());
    assert!(serde_json::from_value::<crate::local::CreateSliceRequest>(
        serde_json::json!({"name": "invalid", "display_backend": "unknown"}),
    )
    .is_err());
}

#[test]
fn local_daemon_protocol_selkies_endpoint_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 310);
    let response = LocalDaemonResponse::SliceDisplayEndpoint {
        endpoint: crate::slice::SliceDisplayEndpoint {
            slice_id: "slice-1".to_string(),
            kind: crate::slice::SliceDisplayEndpointKind::Selkies,
            url: "http://127.0.0.1:45500/".to_string(),
            access: crate::slice::SliceDisplayEndpointAccess::Local,
            expires_at_ms: None,
            capabilities: vec!["view", "websocket", "h264", "software_encoding"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            stream_protocol: None,
            stream_id: None,
            peer_public_key: None,
        },
    };
    let value = serde_json::to_value(response).expect("endpoint should encode");
    let roundtrip: LocalDaemonResponse =
        serde_json::from_value(value.clone()).expect("endpoint should decode");
    assert!(matches!(
        roundtrip,
        LocalDaemonResponse::SliceDisplayEndpoint { .. }
    ));
    let serialized = serde_json::to_string(&value).unwrap();
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "63d4a1fab36398178f7b3c4d984409f4f1ba3dedb7a5da5512693914e2d1de02"
    );
}

#[test]
fn encrypted_display_fragment_contract_is_versioned() {
    use crate::transport::relay_crypto;
    use crate::transport::secure_display::{DisplayMessageKind, DisplayPeer, SecureDisplayChannel};

    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 310);
    let kernel_key = relay_crypto::generate_private_key_base64();
    let viewer_key = relay_crypto::generate_private_key_base64();
    let viewer_public = relay_crypto::public_key_from_private_key_base64(&viewer_key).unwrap();
    let mut channel =
        SecureDisplayChannel::new(kernel_key, viewer_public, "stream-1", DisplayPeer::Kernel)
            .unwrap();
    let packets = channel
        .encode(DisplayMessageKind::Text, b"START_VIDEO")
        .unwrap();
    let plaintext =
        relay_crypto::decrypt_payload_for_private_key(&viewer_key, &packets[0]).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&plaintext.plaintext).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "protocol": "chariox-display-v1", "stream_id": "stream-1", "sender": "kernel",
            "sequence": 0, "kind": "text", "final_fragment": true, "data_base64": "U1RBUlRfVklERU8=",
        })
    );
    let hash = Sha256::digest(serde_json::to_vec(&value).unwrap());
    assert_eq!(
        format!("{hash:x}"),
        "9a6f015a6a9bdcf89322c15e1ccba68f37b88e72d832868f22b9d2684c4123c8"
    );
}

#[test]
fn room_selkies_viewer_admission_contract_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 310);
    let request: LocalDaemonRequest = serde_json::from_value(serde_json::json!({
        "GetSliceDisplayEndpoint": {
            "slice_ref": "slice-1",
            "session_id": "room-1",
            "attachment_id": "attachment-1",
            "viewer_public_key": "viewer-public-key"
        }
    }))
    .expect("Room Selkies viewer request should decode");
    let value = serde_json::to_value(request).expect("viewer request should encode");
    assert_eq!(
        value,
        serde_json::json!({
            "GetSliceDisplayEndpoint": {
                "slice_ref": "slice-1",
                "session_id": "room-1",
                "attachment_id": "attachment-1",
                "viewer_public_key": "viewer-public-key"
            }
        })
    );

    let endpoint = crate::slice::SliceDisplayEndpoint {
        slice_id: "slice-1".to_string(),
        kind: crate::slice::SliceDisplayEndpointKind::Selkies,
        url: "wss://relay.example.test/display/display-1/stream".to_string(),
        access: crate::slice::SliceDisplayEndpointAccess::Tunnel,
        expires_at_ms: Some(60_000),
        capabilities: vec![
            "view".to_string(),
            "websocket".to_string(),
            "h264".to_string(),
            "software_encoding".to_string(),
            "encrypted".to_string(),
            "single_use".to_string(),
        ],
        stream_protocol: Some("chariox-display-v1".to_string()),
        stream_id: Some("display-1".to_string()),
        peer_public_key: Some("worker-public-key".to_string()),
    };
    let value = serde_json::to_value(LocalDaemonResponse::SliceDisplayEndpoint { endpoint })
        .expect("viewer endpoint should encode");
    assert_eq!(
        value.pointer("/SliceDisplayEndpoint/endpoint/stream_protocol"),
        Some(&serde_json::json!("chariox-display-v1"))
    );
    assert_eq!(
        value.pointer("/SliceDisplayEndpoint/endpoint/stream_id"),
        Some(&serde_json::json!("display-1"))
    );
    assert_eq!(
        value.pointer("/SliceDisplayEndpoint/endpoint/peer_public_key"),
        Some(&serde_json::json!("worker-public-key"))
    );
}

#[test]
fn room_selkies_worker_admission_contract_is_versioned() {
    use crate::transport::relay_peer::{
        RelayPeerRequest, RelayPeerResponse, RELAY_PEER_PROTOCOL_VERSION,
    };

    assert_eq!(RELAY_PEER_PROTOCOL_VERSION, 44);
    let endpoint = crate::slice::SliceDisplayEndpoint {
        slice_id: "slice-1".to_string(),
        kind: crate::slice::SliceDisplayEndpointKind::Selkies,
        url: "wss://relay.example.test/display/display-1/stream".to_string(),
        access: crate::slice::SliceDisplayEndpointAccess::Tunnel,
        expires_at_ms: Some(60_000),
        capabilities: vec!["encrypted".to_string(), "single_use".to_string()],
        stream_protocol: Some("chariox-display-v1".to_string()),
        stream_id: Some("display-1".to_string()),
        peer_public_key: Some("worker-public-key".to_string()),
    };
    assert_eq!(
        serde_json::to_value((
            RelayPeerRequest::OpenRoomDisplay {
                session_id: "room-1".to_string(),
                slice_id: "slice-1".to_string(),
                viewer_public_key: "viewer-public-key".to_string(),
            },
            RelayPeerResponse::RoomDisplayOpened {
                session_id: "room-1".to_string(),
                slice_id: "slice-1".to_string(),
                endpoint,
            },
        ))
        .expect("Room display relay contract should encode"),
        serde_json::json!([
            {
                "kind": "open_room_display",
                "session_id": "room-1",
                "slice_id": "slice-1",
                "viewer_public_key": "viewer-public-key"
            },
            {
                "kind": "room_display_opened",
                "session_id": "room-1",
                "slice_id": "slice-1",
                "endpoint": {
                    "slice_id": "slice-1",
                    "kind": "selkies",
                    "url": "wss://relay.example.test/display/display-1/stream",
                    "access": "tunnel",
                    "expires_at_ms": 60_000,
                    "capabilities": ["encrypted", "single_use"],
                    "stream_protocol": "chariox-display-v1",
                    "stream_id": "display-1",
                    "peer_public_key": "worker-public-key"
                }
            }
        ])
    );
}
