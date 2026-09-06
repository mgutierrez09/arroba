use super::*;
use crate::agent::RemoteAgentBinding;
use crate::agent::{AgentInstance, GridPosition};
use crate::runtime::projection::ProjectionMetadata;
use crate::session::{
    PromptQueueItem, PromptStatus, RuntimeInteraction, RuntimeInteractionChoice,
    RuntimeInteractionChoiceStyle, RuntimeInteractionKind, RuntimeInteractionLevel, RuntimeSession,
    WorkflowRunStatus,
};
use crate::terminal::TerminalOutputKind;

#[test]
fn credential_vault_locked_uses_a_stable_transport_error_code() {
    let error = DaemonError::LocalTransport {
        operation: "credential_vault_locked",
        message: "Chariox vault is locked".to_string(),
    };

    let mapped = map_kernel_error(&error);

    assert_eq!(mapped.code, "credential_vault_locked");
    assert!(!mapped.retryable);
}

#[test]
fn terminal_output_event_batches_stay_under_json_byte_cap() {
    let records = (0..20)
        .map(|index| TerminalOutputRecord {
            record_id: Some(index as u64),
            timestamp_ms: 1_000 + index as u64,
            session_id: "session-1".to_string(),
            provider_run_id: "provider-run-1".to_string(),
            agent_id: Some("agent-1".to_string()),
            prompt_id: None,
            prompt_origin: None,
            source_attachment_id: None,
            kind: TerminalOutputKind::ProviderOutput,
            merge_key: Some(format!("chunk-{index}")),
            recipient_attachment_ids: vec!["attachment-1".to_string()],
            pending_recipient_attachment_ids: vec!["attachment-1".to_string()],
            bytes: vec![b'x'; 16 * 1024],
            external_observation_metadata: None,
        })
        .collect::<Vec<_>>();

    let batches = terminal_output_event_batches(records);

    assert!(batches.len() > 1);
    for batch in batches {
        assert!(terminal_output_event_json_bytes(&batch) <= MAX_TERMINAL_OUTPUT_EVENT_JSON_BYTES);
    }
}

#[test]
fn session_snapshot_frame_redacts_remote_relay_credentials() {
    let mut projection = session_snapshot_with_agent();
    let mut agent = projection.session.agents()[0].clone();
    agent.set_remote_execution(Some(RemoteAgentBinding {
        worker_kernel_id: "worker-1".to_string(),
        worker_machine_id: "machine-1".to_string(),
        execution_lease_id: "lease-1".to_string(),
        leased_agent_id: "agent-a".to_string(),
        active_worker_provider_run_id: None,
        relay_url: Some("wss://relay.example".to_string()),
        relay_token: Some("snapshot-secret".to_string()),
        relay_peer_protocol_version: Some(
            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        ),
    }));
    projection.session.set_agents(vec![agent]);
    let frame = KernelOutgoingFrame::Event {
        event_id: 1,
        event: Box::new(KernelEvent::SessionSnapshot {
            session: Box::new(projection.session),
            provider_run: Box::new(None),
            agent_activity: Box::new(projection.agent_activity),
            agent_activity_revision: 0,
        }),
    };

    let encoded = serialize_frame(&frame).expect("session snapshot should serialize");
    let value: serde_json::Value = serde_json::from_str(&encoded).expect("valid JSON frame");
    assert!(value
        .pointer("/event/session/agents/0/remote_execution/relay_token")
        .is_none());
    assert_eq!(
        value
            .pointer("/event/session/agents/0/remote_execution/relay_url")
            .and_then(serde_json::Value::as_str),
        Some("wss://relay.example")
    );
}

#[test]
fn terminal_output_event_batch_size_accounting_matches_serialized_json() {
    let records = (0..5)
        .map(|index| TerminalOutputRecord {
            record_id: Some(index as u64),
            timestamp_ms: 2_000 + index as u64,
            session_id: format!("session-{index}"),
            provider_run_id: format!("provider-run-{index}"),
            agent_id: Some(format!("agent-{index}")),
            prompt_id: None,
            prompt_origin: None,
            source_attachment_id: None,
            kind: TerminalOutputKind::ProviderOutput,
            merge_key: Some(format!("chunk-{index}")),
            recipient_attachment_ids: vec!["attachment-1".to_string()],
            pending_recipient_attachment_ids: vec!["attachment-1".to_string()],
            bytes: vec![b'x'; 256 + index],
            external_observation_metadata: None,
        })
        .collect::<Vec<_>>();
    let accounted_bytes =
        records
            .iter()
            .fold(empty_terminal_output_event_json_bytes(), |bytes, record| {
                let comma = usize::from(bytes != empty_terminal_output_event_json_bytes());
                bytes + terminal_output_record_json_bytes(record) + comma
            });

    assert_eq!(accounted_bytes, terminal_output_event_json_bytes(&records));
}

#[test]
fn waiting_room_rows_changed_event_sends_only_changed_and_removed_rows() {
    let previous = waiting_room_snapshot(
        "inventory-a",
        vec![
            session_summary("session-a", 1),
            session_summary("session-b", 1),
        ],
    );
    let current = waiting_room_snapshot(
        "inventory-b",
        vec![
            session_summary("session-a", 2),
            session_summary("session-c", 1),
        ],
    );

    let event = waiting_room_rows_changed_event(current, Some(&previous))
        .expect("inventory version change should produce event");

    match event {
        KernelEvent::WaitingRoomRowsChanged {
            inventory_version,
            schema_version,
            sessions,
            removed_session_ids,
            ..
        } => {
            assert_eq!(inventory_version, "inventory-b");
            assert_eq!(schema_version, 11);
            assert_eq!(
                sessions
                    .iter()
                    .map(|session| session.id.as_str())
                    .collect::<Vec<_>>(),
                vec!["session-a", "session-c"]
            );
            assert_eq!(removed_session_ids, vec!["session-b"]);
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn waiting_room_inventory_changed_event_refreshes_for_provider_accounts() {
    let previous = waiting_room_snapshot("inventory-a", Vec::new());
    let mut current = previous.clone();
    current.inventory_version = "inventory-b".to_string();
    current.provider_accounts.push(provider_account_profile());

    assert!(matches!(
        waiting_room_inventory_changed_event(&current, Some(&previous)),
        Some(KernelEvent::WaitingRoomInventoryChanged { inventory_version })
            if inventory_version == "inventory-b"
    ));
}

#[test]
fn waiting_room_inventory_event_projection_tracks_full_and_row_refreshes_once() {
    let mut projection = WaitingRoomInventoryEventProjection::default();
    let initial = waiting_room_snapshot("inventory-a", Vec::new());

    let initial_events = projection.project(initial.clone());
    assert_eq!(initial_events.len(), 2);
    assert!(matches!(
        initial_events.as_slice(),
        [
            KernelEvent::WaitingRoomRowsChanged { .. },
            KernelEvent::WaitingRoomInventoryChanged { .. }
        ]
    ));

    let mut account_changed = initial;
    account_changed.inventory_version = "inventory-b".to_string();
    account_changed
        .provider_accounts
        .push(provider_account_profile());
    let account_events = projection.project(account_changed.clone());
    assert!(matches!(
        account_events.as_slice(),
        [KernelEvent::WaitingRoomInventoryChanged { inventory_version }]
            if inventory_version == "inventory-b"
    ));
    assert!(projection.project(account_changed).is_empty());

    let mut row_changed =
        waiting_room_snapshot("inventory-c", vec![session_summary("session-a", 1)]);
    row_changed
        .provider_accounts
        .push(provider_account_profile());
    assert!(matches!(
        projection.project(row_changed).as_slice(),
        [KernelEvent::WaitingRoomRowsChanged { inventory_version, .. }]
            if inventory_version == "inventory-c"
    ));
}

#[test]
fn waiting_room_inventory_changed_event_skips_row_only_changes() {
    let previous = waiting_room_snapshot("inventory-a", vec![session_summary("session-a", 1)]);
    let current = waiting_room_snapshot("inventory-b", vec![session_summary("session-a", 2)]);

    assert!(waiting_room_inventory_changed_event(&current, Some(&previous)).is_none());
}

#[test]
fn waiting_room_rows_changed_event_sends_project_upserts_and_removals() {
    let mut previous = waiting_room_snapshot("inventory-a", Vec::new());
    previous.projects = vec![
        project_summary("project-a", "A"),
        project_summary("project-b", "B"),
    ];
    let mut current = waiting_room_snapshot("inventory-b", Vec::new());
    current.projects = vec![
        project_summary("project-a", "A renamed"),
        project_summary("project-c", "C"),
    ];

    let event = waiting_room_rows_changed_event(current, Some(&previous))
        .expect("project changes should produce a row event");
    match event {
        KernelEvent::WaitingRoomRowsChanged {
            projects,
            removed_project_ids,
            ..
        } => {
            assert_eq!(
                projects
                    .iter()
                    .map(|project| project.id.as_str())
                    .collect::<Vec<_>>(),
                vec!["project-a", "project-c"]
            );
            assert_eq!(removed_project_ids, vec!["project-b"]);
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn waiting_room_rows_changed_event_keeps_archived_ended_session_and_restores_it() {
    let mut active =
        waiting_room_snapshot("inventory-active", vec![session_summary("session-a", 1)]);
    active.projects = vec![project_summary("project-a", "A")];

    let mut ended = session_summary("session-a", 1);
    ended.status = crate::session::SessionStatus::Ended;
    let mut archived_project = project_summary("project-a", "A");
    archived_project.status = crate::session::RuntimeProjectStatus::Archived;
    archived_project.archived_at_ms = Some(2);
    let mut archived = waiting_room_snapshot("inventory-archived", vec![ended]);
    archived.projects = vec![archived_project];

    let archived_event = waiting_room_rows_changed_event(archived.clone(), Some(&active))
        .expect("archiving should upsert the ended drill-down row");
    match archived_event {
        KernelEvent::WaitingRoomRowsChanged {
            sessions,
            removed_session_ids,
            projects,
            ..
        } => {
            assert_eq!(sessions.len(), 1);
            assert_eq!(sessions[0].id, "session-a");
            assert_eq!(sessions[0].status, crate::session::SessionStatus::Ended);
            assert!(removed_session_ids.is_empty());
            assert_eq!(
                projects[0].status,
                crate::session::RuntimeProjectStatus::Archived
            );
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let mut parked = session_summary("session-a", 1);
    parked.status = crate::session::SessionStatus::Parked;
    let mut restored = waiting_room_snapshot("inventory-restored", vec![parked]);
    restored.projects = vec![project_summary("project-a", "A")];
    let restored_event = waiting_room_rows_changed_event(restored, Some(&archived))
        .expect("restoring should upsert the parked drill-down row");
    match restored_event {
        KernelEvent::WaitingRoomRowsChanged {
            sessions,
            removed_session_ids,
            projects,
            ..
        } => {
            assert_eq!(sessions.len(), 1);
            assert_eq!(sessions[0].status, crate::session::SessionStatus::Parked);
            assert!(removed_session_ids.is_empty());
            assert_eq!(
                projects[0].status,
                crate::session::RuntimeProjectStatus::Active
            );
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn waiting_room_rows_changed_event_skips_unchanged_rows() {
    let previous = waiting_room_snapshot("inventory-a", vec![session_summary("session-a", 1)]);
    let current = waiting_room_snapshot("inventory-b", vec![session_summary("session-a", 1)]);

    assert!(waiting_room_rows_changed_event(current, Some(&previous)).is_none());
}

#[test]
fn workflow_run_only_changes_can_skip_session_snapshot() {
    let previous = session_snapshot_with_workflow_status(WorkflowRunStatus::Created);
    let mut current = previous.clone();
    current
        .session
        .workflow_run_mut("workflow-run-a")
        .expect("workflow run should exist")
        .set_status(WorkflowRunStatus::Running);

    assert!(workflow_run_only_changed(&current, Some(&previous)));
    let events = workflow_run_updated_events(&current, Some(&previous));

    assert_eq!(events.len(), 1);
    match &events[0] {
        KernelEvent::WorkflowRunUpdated {
            session_id,
            workflow_run,
        } => {
            assert_eq!(session_id, "session-a");
            assert_eq!(workflow_run.id(), "workflow-run-a");
            assert_eq!(workflow_run.status(), WorkflowRunStatus::Running);
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn provider_run_only_changes_can_skip_session_snapshot() {
    let mut previous = session_snapshot_with_workflow_status(WorkflowRunStatus::Created);
    previous.provider_run = Some(RuntimeProviderRun::from_control_capability_inference(
        "provider-run-a",
        "session-a".to_string(),
        Some("agent-a".to_string()),
        "codex".to_string(),
    ));
    let mut current = previous.clone();
    current
        .provider_run
        .as_mut()
        .expect("provider run should exist")
        .mark_running();

    let event = provider_run_changed_event(&current, Some(&previous))
        .expect("provider run only change should produce event");

    match event {
        KernelEvent::ProviderRunChanged {
            session_id,
            provider_run,
        } => {
            assert_eq!(session_id, "session-a");
            let provider_run = provider_run.expect("provider run should be included");
            assert_eq!(provider_run.id(), "provider-run-a");
            assert_eq!(
                provider_run.state(),
                crate::provider::ProviderRunState::Running
            );
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn prompt_runtime_activity_changes_can_skip_session_snapshot() {
    let previous = session_snapshot_with_agent();
    let mut current = previous.clone();
    let active_prompt = PromptQueueItem::new(
        "prompt-a",
        "attachment-a",
        "agent-a",
        "run it",
        PromptStatus::Running,
    );
    let active_prompt_value =
        serde_json::to_value(&active_prompt).expect("serialize active prompt");
    let mut session_value = serde_json::to_value(&current.session).expect("serialize session");
    session_value["prompt_runtime"] = serde_json::json!({
        "prompt_states": {
            "agent-a": {
                "active_prompt": active_prompt_value,
                "queued_prompts": [],
            },
        },
        "active_prompt": active_prompt_value,
        "queued_prompts": [],
        "scheduler_state": "Running",
    });
    current.session = serde_json::from_value(session_value).expect("deserialize session");
    current.agent_activity.insert(
        "agent-a".to_string(),
        AgentRuntimeActivity {
            status: crate::runtime::projection::AgentRuntimeStatus::Working,
            prompt_status: crate::runtime::projection::AgentPromptRuntimeStatus::Running,
            busy: true,
            active_prompt_count: 1,
            queued_prompt_count: 0,
            unread_idle_output: false,
            queued_prompt_controls: BTreeMap::new(),
            active_turn: Some(crate::runtime::projection::AgentActiveTurnProjection {
                prompt_id: "prompt-a".to_string(),
                provider_run_id: None,
                source_attachment_id: Some("attachment-a".to_string()),
                status: crate::runtime::projection::AgentPromptRuntimeStatus::Running,
                phase: crate::runtime::projection::AgentTurnRuntimePhase::Accepted,
                prompt_origin: Some(crate::session::PromptOrigin::Chariox),
                external_provider: None,
                external_provider_session_id: None,
                external_provider_turn_id: None,
                started_at_ms: None,
            }),
            last_completed_turn: None,
        },
    );

    let event = agent_activity_changed_event(&current, Some(&previous))
        .expect("prompt runtime activity change should produce agent activity delta");

    match event {
        KernelEvent::AgentActivityChanged {
            session_id,
            agent_activity,
            ..
        } => {
            assert_eq!(session_id, "session-a");
            assert_eq!(
                agent_activity
                    .get("agent-a")
                    .expect("agent activity")
                    .prompt_status,
                crate::runtime::projection::AgentPromptRuntimeStatus::Running
            );
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn session_metadata_only_changes_can_skip_session_snapshot() {
    let previous = session_snapshot_with_workflow_status(WorkflowRunStatus::Created);
    let mut current = previous.clone();
    current.session.set_alias(Some("daily".to_string()));
    current
        .session
        .set_workspace_live_sync_mode(Some(WorkspaceLiveSyncMode::Tracked));

    let event = session_metadata_changed_event(&current, Some(&previous))
        .expect("metadata only change should produce event");

    match event {
        KernelEvent::SessionMetadataChanged {
            session_id,
            metadata,
        } => {
            assert_eq!(session_id, "session-a");
            assert_eq!(metadata.alias.as_deref(), Some("daily"));
            assert_eq!(
                metadata.workspace_live_sync_mode,
                Some(WorkspaceLiveSyncMode::Tracked)
            );
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn runtime_interactions_only_changes_can_skip_session_snapshot() {
    let previous = session_snapshot_with_workflow_status(WorkflowRunStatus::Created);
    let mut current = previous.clone();
    current
        .session
        .add_active_interaction(RuntimeInteraction::new(
            "interaction-a",
            "agent-a",
            RuntimeInteractionKind::Permission,
            RuntimeInteractionLevel::Warning,
            Some("Approve command".to_string()),
            "Allow shell command?",
            vec![
                RuntimeInteractionChoice::new(
                    "approve",
                    "Approve",
                    "approve",
                    Some(RuntimeInteractionChoiceStyle::Primary),
                ),
                RuntimeInteractionChoice::new(
                    "deny",
                    "Deny",
                    "deny",
                    Some(RuntimeInteractionChoiceStyle::Danger),
                ),
            ],
            None,
            None,
            None,
        ));

    let event = runtime_interactions_changed_event(&current, Some(&previous))
        .expect("runtime interaction only change should produce event");

    match event {
        KernelEvent::RuntimeInteractionsChanged {
            session_id,
            active_interactions,
        } => {
            assert_eq!(session_id, "session-a");
            assert_eq!(active_interactions.len(), 1);
            let interaction = active_interactions.first().expect("interaction");
            assert_eq!(interaction.id(), "interaction-a");
            assert_eq!(interaction.agent_id(), "agent-a");
            assert_eq!(interaction.kind(), RuntimeInteractionKind::Permission);
            assert_eq!(interaction.choices().len(), 2);
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

fn session_snapshot_with_agent() -> SessionSnapshotProjection {
    let mut session = RuntimeSession::new(
        "session-a",
        Some("Test".to_string()),
        "workspace-a",
        "worktree-a",
        "machine-a",
        "daemon-a",
    );
    session.set_agents(vec![AgentInstance::new(
        "agent-a",
        "agent-a",
        "session-a",
        None,
        "codex",
        Some("model".to_string()),
        None,
        Some("worktree-a".to_string()),
        GridPosition::new(0, 0, 1, 1),
    )]);
    let mut agent_activity = BTreeMap::new();
    agent_activity.insert(
        "agent-a".to_string(),
        AgentRuntimeActivity {
            status: crate::runtime::projection::AgentRuntimeStatus::Idle,
            prompt_status: crate::runtime::projection::AgentPromptRuntimeStatus::None,
            busy: false,
            active_prompt_count: 0,
            queued_prompt_count: 0,
            unread_idle_output: false,
            queued_prompt_controls: BTreeMap::new(),
            active_turn: None,
            last_completed_turn: None,
        },
    );
    SessionSnapshotProjection {
        metadata: ProjectionMetadata::new(2, 0),
        session,
        provider_run: None,
        agent_activity,
    }
}

fn session_snapshot_with_workflow_status(status: WorkflowRunStatus) -> SessionSnapshotProjection {
    let mut session = RuntimeSession::new(
        "session-a",
        Some("Test".to_string()),
        "workspace-a",
        "worktree-a",
        "machine-a",
        "daemon-a",
    );
    let mut workflow_run = WorkflowRun::new(
        "workflow-run-a",
        "workflow-a",
        "endpoint-a",
        "node-a",
        Some("run it".to_string()),
        None,
        Vec::new(),
        Vec::new(),
    );
    workflow_run.set_status(status);
    session.create_workflow_run(workflow_run);
    SessionSnapshotProjection {
        metadata: ProjectionMetadata::new(2, 0),
        session,
        provider_run: None,
        agent_activity: BTreeMap::new(),
    }
}

fn waiting_room_snapshot(
    inventory_version: &str,
    sessions: Vec<WaitingRoomPublicSessionSummary>,
) -> WaitingRoomPublicSnapshot {
    WaitingRoomPublicSnapshot {
        provider_accounts: Vec::new(),
        git_credentials: Vec::new(),
        schema_version: 11,
        inventory_version: inventory_version.to_string(),
        structural_version: format!("structural-{inventory_version}"),
        activity_revision: format!("activity-{inventory_version}"),
        generated_at_ms: 100,
        sessions,
        projects: Vec::new(),
        external_provider_sessions: Vec::new(),
        external_provider_sessions_has_more: false,
        external_provider_sessions_next_cursor: None,
        relay_status: RelayStatus {
            configured: false,
            connected: false,
            relay_url: None,
            relay_token_configured: false,
            daemon_id: "daemon-1".to_string(),
            daemon_alias: None,
            machine_id: "machine-1".to_string(),
            machine_alias: None,
        },
        remote_machines: Vec::new(),
        remote_kernels: Vec::new(),
        terminals: Vec::new(),
        launch_target: None,
    }
}

fn provider_account_profile() -> crate::account_profile::ProviderAccountProfile {
    crate::account_profile::ProviderAccountProfile {
        owner_user_id: crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
        provider: "claude".to_string(),
        profile_id: "managed-claude".to_string(),
        label: "Managed Claude".to_string(),
        origin: crate::account_profile::ProviderAccountProfileOrigin::Linked,
        is_default: false,
        auth_state: crate::account_profile::ProviderAccountAuthState::Authenticated,
        identity_summary: None,
        plan: Some("pro".to_string()),
        detected_provider_version: None,
        last_validated_at_ms: Some(100),
        services: Vec::new(),
        usage: crate::account_profile::ProviderAccountUsageSnapshot::unavailable(
            "managed-claude",
            "claude",
        ),
        credential_kind: None,
        credential_kind_not_reported_reason: Some(
            "provider did not report credential type".to_string(),
        ),
        materializations: Vec::new(),
    }
}

fn session_summary(id: &str, last_used_at_ms: u64) -> WaitingRoomPublicSessionSummary {
    WaitingRoomPublicSessionSummary {
        id: id.to_string(),
        project_id: "project-a".to_string(),
        alias: None,
        workspace_id: "workspace-1".to_string(),
        worktree_id: "worktree-1".to_string(),
        workspace_label: None,
        directory: None,
        worktree_label: None,
        workspace_live_sync_mode: None,
        created_at_ms: 1,
        last_used_at_ms: Some(last_used_at_ms),
        last_prompt_sent_at_ms: None,
        status: crate::session::SessionStatus::Active,
        connected_cli_count: 0,
        joined_collaborator_count: 0,
        pending_collaboration_invite_count: 0,
        activity: crate::local::WaitingRoomSessionActivitySummary::default(),
        agents: Vec::new(),
        workflows: Vec::new(),
    }
}

fn project_summary(id: &str, name: &str) -> crate::local::WaitingRoomPublicProjectSummary {
    crate::local::WaitingRoomPublicProjectSummary {
        id: id.to_string(),
        owner_user_id: crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
        workspace_id: "workspace-1".to_string(),
        workspace_ids: vec!["workspace-1".to_string()],
        name: name.to_string(),
        kind: crate::session::RuntimeProjectKind::Named,
        status: crate::session::RuntimeProjectStatus::Active,
        created_at_ms: 1,
        updated_at_ms: 1,
        archived_at_ms: None,
        session_count: 0,
        last_session_activity_at_ms: None,
        joined_collaborator_count: 0,
        pending_collaboration_invite_count: 0,
    }
}
