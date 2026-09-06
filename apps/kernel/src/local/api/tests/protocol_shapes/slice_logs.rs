use super::*;

#[test]
fn local_daemon_protocol_slice_logs_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 287);

    let request = LocalDaemonRequest::GetSliceLogs(crate::local::GetSliceLogsRequest {
        slice_ref: "linux-dev".to_string(),
        tail_lines: Some(50),
    });
    let response = LocalDaemonResponse::SliceLogs {
        slice: crate::slice::SliceRecord {
            id: "slice-1".to_string(),
            name: "linux-dev".to_string(),
            owner_kernel_id: "home-kernel".to_string(),
            owner_machine_id: "home-machine".to_string(),
            session_id: None,
            session_ids: Vec::new(),
            agent_ids: Vec::new(),
            backend: crate::slice::SliceBackendKind::LocalDocker,
            os: "linux".to_string(),
            display_mode: crate::slice::SliceDisplayMode::Headless,
            status: crate::slice::SliceStatus::Running,
            last_operation: None,
            last_operation_status: None,
            last_error: None,
            last_operation_at_ms: None,
            workspace_id: Some("workspace-1".to_string()),
            worktree_id: Some("worktree-1".to_string()),
            workspace_mount: Some("/repo".to_string()),
            development: None,
            development_storage_root: None,
            development_publication: None,
            worker_kernel_ref: "slice:linux-dev".to_string(),
            worker_kernel_id: Some("slice-worker".to_string()),
            worker_machine_id: Some("slice:slice-1".to_string()),
            relay_endpoint: None,
            local_docker_ports: None,
            providers: vec!["codex".to_string()],
            provider_auth: Vec::new(),
            saved_state_ref: None,
            saved_state_status: None,
            saved_state_updated_at_ms: None,
            display_endpoint: None,
            created_at_ms: 1000,
            updated_at_ms: 2000,
        },
        entries: vec![crate::slice::SliceLogEntry {
            source: "provision".to_string(),
            path: Some("/tmp/chariox-slice.log".to_string()),
            text: "slice booted".to_string(),
            truncated: false,
        }],
    };
    let snapshot = serde_json::json!([request, response]);
    assert_eq!(
        snapshot.pointer("/0/GetSliceLogs/tail_lines"),
        Some(&serde_json::json!(50))
    );
    assert_eq!(
        snapshot.pointer("/1/SliceLogs/entries/0/source"),
        Some(&serde_json::json!("provision"))
    );
    let serialized = serde_json::to_string(&snapshot).expect("slice logs snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "97a4ba681305ac1b2b7c020966e7ae4bce04bb0f2c24967ddd7cf03755f63859"
    );
}

#[test]
fn local_daemon_protocol_slice_audit_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 287);

    let request = LocalDaemonRequest::ListSliceAudit(crate::local::ListSliceAuditRequest {
        slice_ref: "linux-dev".to_string(),
        limit: Some(25),
    });
    let response = LocalDaemonResponse::SliceAuditListed {
        events: vec![crate::durable_state::DurableStateEvent {
            sequence: 7,
            event_id: "state_evt_1".to_string(),
            kind: "slice.audit".to_string(),
            subject_id: Some("slice-1".to_string()),
            timestamp_ms: 42,
            payload: serde_json::json!({
                "slice_id": "slice-1",
                "slice_name": "linux-dev",
                "action": "start",
                "outcome": "completed",
                "provider": null,
                "status": "running",
                "display_mode": "headless",
                "worktree_id": "/repo",
                "agent_ids": [],
                "worker_kernel_id": "worker-1"
            }),
        }],
    };
    let snapshot = serde_json::json!([request, response]);
    assert_eq!(
        snapshot.pointer("/0/ListSliceAudit/slice_ref"),
        Some(&serde_json::json!("linux-dev"))
    );
    assert_eq!(
        snapshot.pointer("/0/ListSliceAudit/limit"),
        Some(&serde_json::json!(25))
    );
    assert_eq!(
        snapshot.pointer("/1/SliceAuditListed/events/0/kind"),
        Some(&serde_json::json!("slice.audit"))
    );
    assert_eq!(
        snapshot.pointer("/1/SliceAuditListed/events/0/payload/action"),
        Some(&serde_json::json!("start"))
    );
    let serialized = serde_json::to_string(&snapshot).expect("slice audit snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "b5e42a56330b7236af80d55b41820bea7a6acb850e07607078041fe1915b3eb0"
    );
}
