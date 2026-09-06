use super::*;
use crate::local::{RelayStatus, WaitingRoomGitCredentialSummary, WaitingRoomInventorySnapshot};

#[test]
fn local_daemon_protocol_waiting_room_git_credentials_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let response = LocalDaemonResponse::WaitingRoomInventory {
        snapshot: WaitingRoomInventorySnapshot {
            inventory_version: "inventory-1".to_string(),
            structural_version: "structural-1".to_string(),
            activity_revision: "activity-1".to_string(),
            sessions: Vec::new(),
            projects: Vec::new(),
            external_provider_sessions: Vec::new(),
            external_provider_sessions_has_more: false,
            external_provider_sessions_next_cursor: None,
            relay_status: RelayStatus {
                configured: true,
                connected: true,
                relay_url: Some("wss://relay.example.test".to_string()),
                relay_token_configured: true,
                daemon_id: "kernel-1".to_string(),
                daemon_alias: Some("Source kernel".to_string()),
                machine_id: "machine-1".to_string(),
                machine_alias: Some("Source machine".to_string()),
            },
            remote_machines: Vec::new(),
            remote_kernels: Vec::new(),
            terminals: Vec::new(),
            launch_target: None,
            provider_accounts: Vec::new(),
            git_credentials: vec![WaitingRoomGitCredentialSummary {
                credential_id: "github".to_string(),
                hostname: "github.com".to_string(),
                label: "GitHub".to_string(),
            }],
        },
    };
    let snapshot = serde_json::to_value(response).expect("Waiting Room inventory should encode");
    assert_eq!(
        snapshot.pointer("/WaitingRoomInventory/snapshot/git_credentials/0"),
        Some(&serde_json::json!({
            "credentialId": "github",
            "hostname": "github.com",
            "label": "GitHub"
        }))
    );
}

#[test]
fn local_daemon_protocol_workflow_run_pagination_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let request = LocalDaemonRequest::ListWorkflowRuns(ListWorkflowRunsRequest {
        session_id: "session-1".to_string(),
        workflow_ref: Some("workflow-1".to_string()),
        cursor: Some("v1:20:run-2".to_string()),
        limit: Some(25),
    });
    assert_eq!(
        serde_json::to_value(request).expect("workflow run page request should encode"),
        serde_json::json!({
            "ListWorkflowRuns": {
                "session_id": "session-1",
                "workflow_ref": "workflow-1",
                "cursor": "v1:20:run-2",
                "limit": 25
            }
        })
    );

    let response = LocalDaemonResponse::WorkflowRunsListed {
        workflow_runs: Vec::new(),
        next_cursor: Some("v1:10:run-1".to_string()),
    };
    assert_eq!(
        serde_json::to_value(response).expect("workflow run page response should encode"),
        serde_json::json!({
            "WorkflowRunsListed": {
                "workflow_runs": [],
                "next_cursor": "v1:10:run-1"
            }
        })
    );
}

#[test]
fn local_daemon_protocol_project_management_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let create = LocalDaemonRequest::CreateSession(
        crate::session::CreateSessionRequest::new("workspace-1", "worktree-1")
            .with_project_selection(crate::session::SessionProjectSelection::Existing {
                project_id: "project-1".to_string(),
            }),
    );
    assert_eq!(
        serde_json::to_value(create)
            .expect("project-selecting session create request should encode")
            .pointer("/CreateSession/project_selection"),
        Some(&serde_json::json!({
            "kind": "existing",
            "project_id": "project-1"
        }))
    );

    let requests = [
        LocalDaemonRequest::ListProjects(ListProjectsRequest {
            include_archived: true,
        }),
        LocalDaemonRequest::RenameProject(RenameProjectRequest {
            project_id: "project-1".to_string(),
            name: "Renamed".to_string(),
        }),
        LocalDaemonRequest::UpdateProjectWorkspaces(UpdateProjectWorkspacesRequest {
            project_id: "project-1".to_string(),
            workspace_ids: vec!["workspace-1".to_string(), "workspace-2".to_string()],
        }),
        LocalDaemonRequest::ArchiveProject(ArchiveProjectRequest {
            project_id: "project-1".to_string(),
        }),
        LocalDaemonRequest::DeleteProject(crate::local::DeleteProjectRequest {
            project_id: "project-1".to_string(),
        }),
        LocalDaemonRequest::RestoreProject(RestoreProjectRequest {
            project_id: "project-1".to_string(),
        }),
    ];
    assert_eq!(
        requests
            .into_iter()
            .map(|request| serde_json::to_value(request).expect("project request should encode"))
            .collect::<Vec<_>>(),
        vec![
            serde_json::json!({ "ListProjects": { "include_archived": true } }),
            serde_json::json!({ "RenameProject": { "project_id": "project-1", "name": "Renamed" } }),
            serde_json::json!({
                "UpdateProjectWorkspaces": {
                    "project_id": "project-1",
                    "workspace_ids": ["workspace-1", "workspace-2"]
                }
            }),
            serde_json::json!({ "ArchiveProject": { "project_id": "project-1" } }),
            serde_json::json!({ "DeleteProject": { "project_id": "project-1" } }),
            serde_json::json!({ "RestoreProject": { "project_id": "project-1" } }),
        ]
    );

    let project: crate::session::RuntimeProject = serde_json::from_value(serde_json::json!({
        "id": "project-1",
        "owner_user_id": "owner-1",
        "workspace_id": "workspace-1",
        "name": "Owner/repo",
        "kind": "default",
        "status": "archived",
        "created_at_ms": 10,
        "updated_at_ms": 20,
        "archived_at_ms": 20
    }))
    .expect("runtime project should decode");
    let response = LocalDaemonResponse::ProjectArchived {
        project,
        sessions: Vec::new(),
    };
    assert_eq!(
        serde_json::to_value(response).expect("project response should encode"),
        serde_json::json!({
            "ProjectArchived": {
                "project": {
                    "id": "project-1",
                    "owner_user_id": "owner-1",
                    "workspace_id": "workspace-1",
                    "name": "Owner/repo",
                    "kind": "default",
                    "status": "archived",
                    "created_at_ms": 10,
                    "updated_at_ms": 20,
                    "archived_at_ms": 20
                },
                "sessions": []
            }
        })
    );

    let updated_project: crate::session::RuntimeProject =
        serde_json::from_value(serde_json::json!({
            "id": "project-1",
            "owner_user_id": "owner-1",
            "workspace_id": "workspace-1",
            "workspace_ids": ["workspace-1", "workspace-2"],
            "name": "Owner/repo",
            "kind": "default",
            "status": "active",
            "created_at_ms": 10,
            "updated_at_ms": 30
        }))
        .expect("multi-Workspace project should decode");
    assert_eq!(
        serde_json::to_value(LocalDaemonResponse::ProjectWorkspacesUpdated {
            project: updated_project,
        })
        .expect("project Workspace response should encode"),
        serde_json::json!({
            "ProjectWorkspacesUpdated": {
                "project": {
                    "id": "project-1",
                    "owner_user_id": "owner-1",
                    "workspace_id": "workspace-1",
                    "workspace_ids": ["workspace-1", "workspace-2"],
                    "name": "Owner/repo",
                    "kind": "default",
                    "status": "active",
                    "created_at_ms": 10,
                    "updated_at_ms": 30
                }
            }
        })
    );
}

#[test]
fn local_daemon_protocol_agent_prompt_schedule_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let create = LocalDaemonRequest::CreateAgentPromptSchedule(
        crate::local::CreateAgentPromptScheduleRequest {
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
            kind: crate::session::AgentPromptScheduleKind::Recurring,
            interval_seconds: 300,
            prompt: Some("Continue the audit.".to_string()),
        },
    );
    assert_eq!(
        serde_json::to_value(create).expect("agent prompt schedule request should encode"),
        serde_json::json!({
            "CreateAgentPromptSchedule": {
                "session_id": "session-1",
                "agent_id": "agent-1",
                "kind": "recurring",
                "interval_seconds": 300,
                "prompt": "Continue the audit."
            }
        })
    );

    let cancel = LocalDaemonRequest::CancelAgentPromptSchedule(
        crate::local::CancelAgentPromptScheduleRequest {
            session_id: "session-1".to_string(),
            schedule_id: "schedule-1".to_string(),
        },
    );
    assert_eq!(
        serde_json::to_value(cancel).expect("agent prompt schedule cancellation should encode"),
        serde_json::json!({
            "CancelAgentPromptSchedule": {
                "session_id": "session-1",
                "schedule_id": "schedule-1"
            }
        })
    );

    let mut session = crate::session::RuntimeSession::new(
        "session-1",
        None,
        "workspace-1",
        "worktree-1",
        "machine-1",
        "daemon-1",
    );
    let schedule = crate::session::AgentPromptSchedule::new(
        "schedule-1",
        "agent-1",
        crate::session::AgentPromptScheduleKind::Once,
        60,
        "Continue from where you left off.",
        1_000,
    );
    session.add_agent_prompt_schedule(schedule.clone());
    let response = LocalDaemonResponse::AgentPromptScheduleCreated { schedule, session };
    let snapshot =
        serde_json::to_value(response).expect("agent prompt schedule response should encode");
    assert_eq!(
        snapshot.pointer("/AgentPromptScheduleCreated/schedule/next_run_at_ms"),
        Some(&serde_json::json!(61_000))
    );
    assert_eq!(
        snapshot.pointer("/AgentPromptScheduleCreated/session/agent_prompt_schedules/0/id"),
        Some(&serde_json::json!("schedule-1"))
    );
}

#[test]
fn local_daemon_protocol_queued_metaagent_task_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);
    let mut session = crate::session::RuntimeSession::new(
        "session-1",
        None,
        "workspace-1",
        "worktree-1",
        "machine-1",
        "daemon-1",
    );
    session.enqueue_metaagent_task(crate::session::QueuedMetaagentTask::new(
        "session-task:prompt-2",
        "agent-1",
        "attachment-1",
        "coordinate the queued work",
        Vec::new(),
    ));
    let snapshot = serde_json::to_value(session).expect("queued Meta task should encode");
    assert_eq!(
        snapshot.pointer("/queued_metaagent_tasks/0/id"),
        Some(&serde_json::json!("session-task:prompt-2")),
    );
    assert_eq!(
        snapshot.pointer("/queued_metaagent_tasks/0/task_markdown"),
        Some(&serde_json::json!("coordinate the queued work")),
    );
}

#[test]
fn local_daemon_protocol_pause_workflow_run_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let request = LocalDaemonRequest::PauseWorkflowRun(PauseWorkflowRunRequest {
        session_id: "session-1".to_string(),
        workflow_run_ref: "run-1".to_string(),
    });
    assert_eq!(
        serde_json::to_value(request).expect("pause workflow run request should encode"),
        serde_json::json!({
            "PauseWorkflowRun": {
                "session_id": "session-1",
                "workflow_run_ref": "run-1"
            }
        })
    );

    let response = LocalDaemonResponse::WorkflowRunPaused {
        workflow_run: crate::session::WorkflowRun::new(
            "run-1",
            "workflow-1",
            "endpoint-1",
            "node-1",
            Some("pause this run".to_string()),
            None,
            Vec::new(),
            Vec::new(),
        ),
        session: crate::session::RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        ),
    };
    let snapshot =
        serde_json::to_value(response).expect("pause workflow run response should encode");
    assert_eq!(
        snapshot.pointer("/WorkflowRunPaused/workflow_run/id"),
        Some(&serde_json::json!("run-1"))
    );
    assert_eq!(
        snapshot.pointer("/WorkflowRunPaused/session/id"),
        Some(&serde_json::json!("session-1"))
    );
}

#[test]
fn local_daemon_protocol_provider_targeted_terminal_resize_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let request = LocalDaemonRequest::ResizeTerminal(crate::local::ResizeTerminalRequest {
        session_id: "session-1".to_string(),
        provider_run_id: Some("provider-run-2".to_string()),
        cols: 80,
        rows: 24,
    });
    assert_eq!(
        serde_json::to_value(request).expect("provider-targeted terminal resize should encode"),
        serde_json::json!({
            "ResizeTerminal": {
                "session_id": "session-1",
                "provider_run_id": "provider-run-2",
                "cols": 80,
                "rows": 24
            }
        })
    );

    let legacy: LocalDaemonRequest = serde_json::from_value(serde_json::json!({
        "ResizeTerminal": {
            "session_id": "session-1",
            "cols": 120,
            "rows": 40
        }
    }))
    .expect("legacy active-provider resize should still decode");
    assert!(matches!(
        legacy,
        LocalDaemonRequest::ResizeTerminal(crate::local::ResizeTerminalRequest {
            provider_run_id: None,
            ..
        })
    ));
}

#[test]
fn local_daemon_protocol_terminal_command_catalog_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let request = LocalDaemonRequest::GetTerminalCommandCatalog(GetTerminalCommandCatalogRequest);
    assert_eq!(
        serde_json::to_value(request).expect("terminal command catalog request should encode"),
        serde_json::json!({ "GetTerminalCommandCatalog": null })
    );

    let response = LocalDaemonResponse::TerminalCommandCatalog {
        catalog: TerminalCommandCatalog {
            revision: "sha256:catalog".to_string(),
            nodes: vec![TerminalCommandCatalogNode {
                id: "meta".to_string(),
                label: "/meta".to_string(),
                description: "Start a temporary Meta mode task".to_string(),
                value: "/meta ".to_string(),
                kind: TerminalCommandCatalogNodeKind::PromptPrefix,
                execution_target: TerminalCommandCatalogExecutionTarget::PromptPrefix,
                surfaces: vec![TerminalCommandCatalogSurface::Session],
                search_aliases: vec!["delegate".to_string()],
                intents: vec!["coordinate workers".to_string()],
                examples: vec!["/meta Build this through workers".to_string()],
                dynamic_source: None,
                children: vec![TerminalCommandCatalogNode {
                    id: "meta-child".to_string(),
                    label: "child".to_string(),
                    description: "Child command".to_string(),
                    value: "/meta child".to_string(),
                    kind: TerminalCommandCatalogNodeKind::Command,
                    execution_target: TerminalCommandCatalogExecutionTarget::Kernel,
                    surfaces: vec![TerminalCommandCatalogSurface::Session],
                    search_aliases: Vec::new(),
                    intents: Vec::new(),
                    examples: Vec::new(),
                    dynamic_source: Some("test.dynamic".to_string()),
                    children: Vec::new(),
                }],
            }],
        },
    };

    assert_eq!(
        serde_json::to_value(response).expect("terminal command catalog response should encode"),
        serde_json::json!({
            "TerminalCommandCatalog": {
                "catalog": {
                    "revision": "sha256:catalog",
                    "nodes": [{
                        "id": "meta",
                        "label": "/meta",
                        "description": "Start a temporary Meta mode task",
                        "value": "/meta ",
                        "kind": "prompt_prefix",
                        "execution_target": "prompt_prefix",
                        "surfaces": ["session"],
                        "search_aliases": ["delegate"],
                        "intents": ["coordinate workers"],
                        "examples": ["/meta Build this through workers"],
                        "children": [{
                            "id": "meta-child",
                            "label": "child",
                            "description": "Child command",
                            "value": "/meta child",
                            "kind": "command",
                            "execution_target": "kernel",
                            "surfaces": ["session"],
                            "dynamic_source": "test.dynamic"
                        }]
                    }]
                }
            }
        })
    );
}

#[test]
fn local_daemon_protocol_waiting_room_activity_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let summary = crate::local::WaitingRoomSessionActivitySummary {
        agent_count: 4,
        working_agent_count: 1,
        active_prompt_count: 1,
        queued_prompt_count: 2,
        error_agent_count: 1,
        remote_agent_count: 3,
        missing_worker_provider_run_count: 1,
        home_proxy_agent_count: 2,
        remote_extension_sync_issue_count: 1,
        remote_extension_pending_revoke_count: 1,
        unread_idle_agent_count: 1,
    };

    assert_eq!(
        serde_json::to_value(summary).expect("waiting-room activity summary should encode"),
        serde_json::json!({
            "agent_count": 4,
            "working_agent_count": 1,
            "active_prompt_count": 1,
            "queued_prompt_count": 2,
            "error_agent_count": 1,
            "remote_agent_count": 3,
            "missing_worker_provider_run_count": 1,
            "home_proxy_agent_count": 2,
            "remote_extension_sync_issue_count": 1,
            "remote_extension_pending_revoke_count": 1,
            "unread_idle_agent_count": 1
        })
    );
}

#[test]
fn local_daemon_protocol_transport_health_relay_reconnect_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let snapshot = crate::runtime::projection::TransportHealthSnapshot {
        active_connections: 1,
        active_subscriptions: 2,
        retained_event_limit: 256,
        command_result_cache_limit: 512,
        inbound_request_limit: 8,
        incoming_requests: 3,
        emitted_events: 4,
        replay_gaps: 5,
        inbound_overload_rejections: 6,
        duplicate_command_conflicts: 7,
        outgoing_queue_overflows: 8,
        slow_consumer_closes: 9,
        relay_reconnect_attempts: 10,
        relay_last_reconnect_reason: Some("relay heartbeat send failed".to_string()),
        relay_last_reconnect_delay_ms: Some(750),
        relay_last_reconnect_url: Some("wss://relay-b.example.test".to_string()),
        relay_last_connected_url: Some("wss://relay-a.example.test".to_string()),
    };

    assert_eq!(
        serde_json::to_value(snapshot).expect("transport health snapshot should encode"),
        serde_json::json!({
            "active_connections": 1,
            "active_subscriptions": 2,
            "retained_event_limit": 256,
            "command_result_cache_limit": 512,
            "inbound_request_limit": 8,
            "incoming_requests": 3,
            "emitted_events": 4,
            "replay_gaps": 5,
            "inbound_overload_rejections": 6,
            "duplicate_command_conflicts": 7,
            "outgoing_queue_overflows": 8,
            "slow_consumer_closes": 9,
            "relay_reconnect_attempts": 10,
            "relay_last_reconnect_reason": "relay heartbeat send failed",
            "relay_last_reconnect_delay_ms": 750,
            "relay_last_reconnect_url": "wss://relay-b.example.test",
            "relay_last_connected_url": "wss://relay-a.example.test"
        })
    );
}

#[test]
fn local_daemon_protocol_queued_prompt_controls_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let active_cancel_request =
        LocalDaemonRequest::CancelActivePrompt(crate::local::CancelActivePromptRequest {
            session_id: "session-1".to_string(),
            attachment_id: "attachment-1".to_string(),
            target_agent_id: Some("agent-1".to_string()),
        });
    let active_cancel_snapshot =
        serde_json::to_value(active_cancel_request).expect("active cancel request should encode");
    assert_eq!(
        active_cancel_snapshot.pointer("/CancelActivePrompt/target_agent_id"),
        Some(&serde_json::json!("agent-1"))
    );

    let steer_request =
        LocalDaemonRequest::SteerQueuedPrompt(crate::local::SteerQueuedPromptRequest {
            session_id: "session-1".to_string(),
            attachment_id: "attachment-1".to_string(),
            target_agent_id: "agent-1".to_string(),
            prompt_id: "prompt-queued".to_string(),
        });
    let steer_snapshot = serde_json::to_value(steer_request).expect("steer request should encode");
    assert_eq!(
        steer_snapshot.pointer("/SteerQueuedPrompt/prompt_id"),
        Some(&serde_json::json!("prompt-queued"))
    );

    let cancel_request =
        LocalDaemonRequest::CancelQueuedPrompt(crate::local::CancelQueuedPromptRequest {
            session_id: "session-1".to_string(),
            attachment_id: "attachment-1".to_string(),
            target_agent_id: "agent-1".to_string(),
            prompt_id: "prompt-queued".to_string(),
        });
    let cancel_snapshot =
        serde_json::to_value(cancel_request).expect("cancel request should encode");
    assert_eq!(
        cancel_snapshot.pointer("/CancelQueuedPrompt/target_agent_id"),
        Some(&serde_json::json!("agent-1"))
    );

    let update_request =
        LocalDaemonRequest::UpdateQueuedPrompt(crate::local::UpdateQueuedPromptRequest {
            session_id: "session-1".to_string(),
            attachment_id: "attachment-1".to_string(),
            target_agent_id: "agent-1".to_string(),
            prompt_id: "prompt-queued".to_string(),
            prompt: "updated queued text".to_string(),
        });
    let update_snapshot =
        serde_json::to_value(update_request).expect("update request should encode");
    assert_eq!(
        update_snapshot.pointer("/UpdateQueuedPrompt/prompt"),
        Some(&serde_json::json!("updated queued text"))
    );

    let external_prompt = crate::session::PromptQueueItem::external_observed_running(
        "codex",
        "thread-1",
        "user-1",
        "agent-1",
        "external text",
    );
    let external_prompt_snapshot =
        serde_json::to_value(external_prompt).expect("external prompt should encode");
    assert_eq!(
        external_prompt_snapshot.pointer("/prompt_origin"),
        Some(&serde_json::json!("external"))
    );
    assert_eq!(
        external_prompt_snapshot.pointer("/external_provider"),
        Some(&serde_json::json!("codex"))
    );
    assert_eq!(
        external_prompt_snapshot.pointer("/external_provider_session_id"),
        Some(&serde_json::json!("thread-1"))
    );
    assert_eq!(
        external_prompt_snapshot.pointer("/external_provider_turn_id"),
        Some(&serde_json::json!("user-1"))
    );

    let prompt = crate::session::PromptQueueItem::new(
        "prompt-queued",
        "attachment-1",
        "agent-1",
        "queued text",
        crate::session::PromptStatus::Queued,
    );
    let session = crate::session::RuntimeSession::new(
        "session-1",
        None,
        "workspace-1",
        "worktree-1",
        "machine-1",
        "daemon-1",
    );
    let steer_response = LocalDaemonResponse::QueuedPromptSteered {
        prompt: prompt.clone(),
        session: session.clone(),
        agent_activity: std::collections::BTreeMap::new(),
        agent_activity_revision: 7,
    };
    let steer_response_snapshot =
        serde_json::to_value(steer_response).expect("steer response should encode");
    assert_eq!(
        steer_response_snapshot.pointer("/QueuedPromptSteered/prompt/id"),
        Some(&serde_json::json!("prompt-queued"))
    );
    assert_eq!(
        steer_response_snapshot.pointer("/QueuedPromptSteered/prompt/prompt_origin"),
        Some(&serde_json::json!("chariox"))
    );
    assert_eq!(
        steer_response_snapshot.pointer("/QueuedPromptSteered/agent_activity_revision"),
        Some(&serde_json::json!(7))
    );

    let cancel_response = LocalDaemonResponse::QueuedPromptCancelled {
        prompt: prompt.clone(),
        session: session.clone(),
        agent_activity: std::collections::BTreeMap::new(),
        agent_activity_revision: 8,
    };
    let cancel_response_snapshot =
        serde_json::to_value(cancel_response).expect("cancel response should encode");
    assert_eq!(
        cancel_response_snapshot.pointer("/QueuedPromptCancelled/session/id"),
        Some(&serde_json::json!("session-1"))
    );
    assert_eq!(
        cancel_response_snapshot.pointer("/QueuedPromptCancelled/agent_activity_revision"),
        Some(&serde_json::json!(8))
    );

    let update_response = LocalDaemonResponse::QueuedPromptUpdated {
        prompt,
        session,
        agent_activity: std::collections::BTreeMap::new(),
        agent_activity_revision: 9,
    };
    let update_response_snapshot =
        serde_json::to_value(update_response).expect("update response should encode");
    assert_eq!(
        update_response_snapshot.pointer("/QueuedPromptUpdated/prompt/prompt"),
        Some(&serde_json::json!("queued text"))
    );
    assert_eq!(
        update_response_snapshot.pointer("/QueuedPromptUpdated/agent_activity_revision"),
        Some(&serde_json::json!(9))
    );
}

#[test]
fn local_daemon_protocol_batch_launch_and_prompt_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let launch_request = LocalDaemonRequest::LaunchProviderRuns(LaunchProviderRunsRequest {
        max_concurrency: Some(8),
        launches: vec![
            LaunchProviderRunRequest {
                session_id: "session-1".to_string(),
                agent_id: Some("agent-1".to_string()),
                adapter_key: "codex".to_string(),
                provider: "codex".to_string(),
                account_profile: "default".to_string(),
                model: "gpt-5".to_string(),
                variant: Some("medium".to_string()),
                structured_endpoint: None,
                provider_session_id: None,
                native_tui: false,
            },
            LaunchProviderRunRequest {
                session_id: "session-1".to_string(),
                agent_id: Some("agent-2".to_string()),
                adapter_key: "opencode".to_string(),
                provider: "opencode".to_string(),
                account_profile: "work".to_string(),
                model: "gpt-5.1".to_string(),
                variant: None,
                structured_endpoint: Some("http://127.0.0.1:4567".to_string()),
                provider_session_id: None,
                native_tui: false,
            },
        ],
    });
    let prompt_request = LocalDaemonRequest::SubmitPrompts(SubmitPromptsRequest {
        session_id: "session-1".to_string(),
        attachment_id: "attachment-1".to_string(),
        max_concurrency: Some(4),
        prompts: vec![
            SubmitPromptsRequestItem {
                session_id: None,
                attachment_id: None,
                target_agent_id: "agent-1".to_string(),
                prompt: "review shard 1".to_string(),
                attachments: Vec::new(),
            },
            SubmitPromptsRequestItem {
                session_id: Some("session-2".to_string()),
                attachment_id: Some("attachment-2".to_string()),
                target_agent_id: "agent-2".to_string(),
                prompt: "review shard 2".to_string(),
                attachments: Vec::new(),
            },
        ],
    });
    let snapshot = serde_json::json!([launch_request, prompt_request]);
    assert_eq!(
        snapshot.pointer("/0/LaunchProviderRuns/max_concurrency"),
        Some(&serde_json::json!(8))
    );
    assert_eq!(
        snapshot.pointer("/0/LaunchProviderRuns/launches/1/structured_endpoint"),
        Some(&serde_json::json!("http://127.0.0.1:4567"))
    );
    assert_eq!(
        snapshot.pointer("/1/SubmitPrompts/prompts/0/target_agent_id"),
        Some(&serde_json::json!("agent-1"))
    );
    assert_eq!(
        snapshot.pointer("/1/SubmitPrompts/prompts/1/session_id"),
        Some(&serde_json::json!("session-2"))
    );
    assert_eq!(
        snapshot.pointer("/1/SubmitPrompts/prompts/1/attachment_id"),
        Some(&serde_json::json!("attachment-2"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("batch launch/prompt snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "baedf3e025266833349aeecb0c03c1e2daf959b6c42adc75dfa5baea591b2fe8"
    );
}

#[test]
fn local_daemon_protocol_move_agent_to_local_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let request = LocalDaemonRequest::MoveAgentToLocal(MoveAgentToLocalRequest {
        session_id: "session-1".to_string(),
        agent_ref: "agent-1".to_string(),
    });
    let mut agent_value = serde_json::to_value(crate::agent::AgentInstance::new(
        "agent-1",
        "agent-ref-1",
        "session-1",
        None,
        "codex",
        None,
        None,
        None,
        crate::agent::GridPosition::new(0, 0, 1, 1),
    ))
    .expect("agent snapshot should encode");
    agent_value["created_at_ms"] = serde_json::json!(1_000);
    agent_value["last_activity_at_ms"] = serde_json::json!(1_000);
    let response = LocalDaemonResponse::AgentMovedToLocal {
        agent: serde_json::from_value(agent_value).expect("agent snapshot should decode"),
    };

    let snapshot = serde_json::json!([request, response]);
    assert_eq!(
        snapshot.pointer("/0/MoveAgentToLocal/session_id"),
        Some(&serde_json::json!("session-1"))
    );
    assert_eq!(
        snapshot.pointer("/1/AgentMovedToLocal/agent/id"),
        Some(&serde_json::json!("agent-1"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("move agent to local snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "f6e7e181738b182da73d2156f9f876698b70f464d03f1becd6e62d4dc21e3196"
    );
}

#[test]
fn local_daemon_protocol_remote_agent_binding_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let mut agent = crate::agent::AgentInstance::new(
        "agent-remote",
        "agent-ref-remote",
        "session-1",
        None,
        "codex",
        None,
        None,
        None,
        crate::agent::GridPosition::new(0, 0, 1, 1),
    );
    agent.set_remote_execution(Some(crate::agent::RemoteAgentBinding {
        worker_kernel_id: "worker-kernel".to_string(),
        worker_machine_id: "worker-machine".to_string(),
        execution_lease_id: "lease-1".to_string(),
        leased_agent_id: "leased-agent-1".to_string(),
        active_worker_provider_run_id: Some("worker-run-1".to_string()),
        relay_url: Some("wss://relay.example.test".to_string()),
        relay_token: Some("secret-token".to_string()),
        relay_peer_protocol_version: Some(
            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        ),
    }));
    let mut agent_value = serde_json::to_value(agent).expect("remote agent snapshot should encode");
    agent_value["created_at_ms"] = serde_json::json!(1_000);
    agent_value["last_activity_at_ms"] = serde_json::json!(1_000);
    let response = LocalDaemonResponse::AgentMovedToRemote {
        agent: serde_json::from_value(agent_value).expect("remote agent snapshot should decode"),
    };
    let mut snapshot = serde_json::json!(response);
    snapshot
        .pointer_mut("/AgentMovedToRemote/agent/remote_execution")
        .and_then(serde_json::Value::as_object_mut)
        .expect("remote binding should be present")
        .remove("relay_token");
    assert_eq!(
        snapshot.pointer("/AgentMovedToRemote/agent/remote_execution/relay_token"),
        None
    );
    assert_eq!(
        snapshot.pointer("/AgentMovedToRemote/agent/remote_execution/relay_peer_protocol_version"),
        Some(&serde_json::json!(
            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
        ))
    );
    let serialized = serde_json::to_string(&snapshot).expect("remote agent snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "637abb74695e5c45c4a510bf3c82a96e01e293016fa4a9335e0890a840226add"
    );
    // The peer advertisement changes to v45; the existing remote binding
    // remains byte-compatible when advertising the preceding peer version.
    snapshot["AgentMovedToRemote"]["agent"]["remote_execution"]["relay_peer_protocol_version"] =
        serde_json::json!(44);
    let previous = serde_json::to_string(&snapshot).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(previous.as_bytes())),
        "ecf59ae63759119b06d95b995d7abd18ef477be4fcb0bb2389193fd616a75b29"
    );
}
