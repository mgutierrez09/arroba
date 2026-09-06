use super::*;

#[test]
fn local_daemon_protocol_semantic_recall_search_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 312);

    let request = LocalDaemonRequest::SemanticSearchRecall(SemanticSearchRecallRequest {
        query: "why did the build fail".to_string(),
        mode: Some(crate::local::SemanticSearchRecallMode::Agent),
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        provider: Some("codex".to_string()),
        model: Some("gpt-5".to_string()),
        workflow_id: None,
        machine_id: None,
        repo_root: None,
        worktree_path: None,
        kind: Some("provider_output".to_string()),
        cursor: Some("cursor-0".to_string()),
        limit: Some(12),
    });
    let request_snapshot = serde_json::to_value(request).expect("request should serialize");
    assert_eq!(
        request_snapshot.pointer("/SemanticSearchRecall/query"),
        Some(&serde_json::json!("why did the build fail"))
    );
    assert_eq!(
        request_snapshot.pointer("/SemanticSearchRecall/mode"),
        Some(&serde_json::json!("agent"))
    );
    assert_eq!(
        request_snapshot.pointer("/SemanticSearchRecall/session_id"),
        Some(&serde_json::json!("session-1"))
    );
    assert_eq!(
        request_snapshot.pointer("/SemanticSearchRecall/limit"),
        Some(&serde_json::json!(12))
    );
    assert_eq!(
        request_snapshot.pointer("/SemanticSearchRecall/cursor"),
        Some(&serde_json::json!("cursor-0"))
    );

    let event = crate::history::HistoryEvent {
        event_id: "event-1".to_string(),
        sequence: 7,
        timestamp_ms: 1234,
        workspace_id: None,
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        agent_alias: None,
        provider: Some("codex".to_string()),
        model: Some("gpt-5".to_string()),
        turn_id: None,
        prompt_id: None,
        provider_run_id: None,
        provider_session_id: None,
        workflow_id: None,
        workflow_run_id: None,
        workflow_node_id: None,
        machine_id: None,
        repo_root: None,
        worktree_path: None,
        kind: crate::history::HistoryEventKind::ProviderOutput,
        role: Some(crate::history::HistoryEventRole::Assistant),
        content: Some("the build failed because tests failed".to_string()),
        content_ref: None,
        metadata: BTreeMap::new(),
        candidate_agent_ids: Vec::new(),
        candidate_prompt_ids: Vec::new(),
        candidate_turn_ids: Vec::new(),
        attribution_confidence: None,
        caused_by_event_id: None,
    };
    let response = LocalDaemonResponse::SemanticRecallEvents {
        results: vec![SemanticRecallMatch {
            event,
            score_millis: Some(914),
            chunk_index: Some(0),
            chunk_text: Some("build failed because tests failed".to_string()),
            reason: Some("high: direct match".to_string()),
        }],
        next_cursor: Some("cursor-1".to_string()),
        unavailable_reason: None,
        answer: Some("The build failed because tests failed.".to_string()),
    };
    let response_snapshot = serde_json::to_value(response).expect("response should serialize");
    assert_eq!(
        response_snapshot.pointer("/SemanticRecallEvents/results/0/score_millis"),
        Some(&serde_json::json!(914))
    );
    assert_eq!(
        response_snapshot.pointer("/SemanticRecallEvents/results/0/chunk_text"),
        Some(&serde_json::json!("build failed because tests failed"))
    );
    assert_eq!(
        response_snapshot.pointer("/SemanticRecallEvents/results/0/reason"),
        Some(&serde_json::json!("high: direct match"))
    );
    assert_eq!(
        response_snapshot.pointer("/SemanticRecallEvents/answer"),
        Some(&serde_json::json!("The build failed because tests failed."))
    );
    assert_eq!(
        response_snapshot.pointer("/SemanticRecallEvents/unavailable_reason"),
        Some(&serde_json::Value::Null)
    );

    let serialized = serde_json::to_string(&serde_json::json!({
        "request": request_snapshot,
        "response": response_snapshot,
    }))
    .expect("semantic recall snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "5c28f67e35c7840b0f754e4d635fce1670dba9064756a90893204415418f5d2a"
    );
}

#[test]
fn local_daemon_protocol_query_recall_context_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 312);

    let request = LocalDaemonRequest::QueryRecall(QueryRecallRequest {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        provider: None,
        model: None,
        workflow_id: None,
        machine_id: None,
        repo_root: None,
        worktree_path: None,
        kind: Some("provider_output".to_string()),
        text: None,
        after_sequence: None,
        before_sequence: Some(42),
        limit: Some(10),
    });
    let snapshot = serde_json::to_value(request).expect("request should serialize");
    assert_eq!(
        snapshot.pointer("/QueryRecall/before_sequence"),
        Some(&serde_json::json!(42))
    );

    let serialized = serde_json::to_string(&snapshot).expect("query recall snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "4e8f0791f65d7a0d7df9d983cafe6584d2e155bd59d0f0a51f176d6f50ba7485"
    );
}

#[test]
fn local_daemon_protocol_agent_config_workspace_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 312);

    let request = LocalDaemonRequest::UpdateAgentConfig(UpdateAgentConfigRequest {
        session_id: "session-1".to_string(),
        agent_id: "agent-1".to_string(),
        execution_mode: Some(AgentExecutionMode::Build),
        clear_execution_mode: false,
        permission_level: Some(AgentPermissionLevel::Required),
        clear_permission_level: false,
        workspace_id: Some("/repo".to_string()),
        clear_workspace_id: false,
        worktree_id: Some("/repo-feature".to_string()),
        clear_worktree_id: false,
    });
    let snapshot = serde_json::to_value(request).expect("request should serialize");
    assert_eq!(
        snapshot.pointer("/UpdateAgentConfig/workspace_id"),
        Some(&serde_json::json!("/repo"))
    );
    assert_eq!(
        snapshot.pointer("/UpdateAgentConfig/worktree_id"),
        Some(&serde_json::json!("/repo-feature"))
    );
    let serialized = serde_json::to_string(&snapshot).expect("agent config snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "826ea52fcd9a136d573384f51126a8b59e5829b8fd6160d6601e5d5759d5f6a2"
    );
}

#[test]
fn local_daemon_protocol_native_tui_provider_selection_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 312);

    let request =
        LocalDaemonRequest::UpdateProviderRunSelection(UpdateProviderRunSelectionRequest {
            session_id: "session-1".to_string(),
            provider_run_id: "provider-run-1".to_string(),
            model: Some("openai/gpt-5.4".to_string()),
            variant: Some("high".to_string()),
            clear_variant: false,
        });
    let snapshot = serde_json::to_value(request).expect("request should serialize");
    assert_eq!(
        snapshot.pointer("/UpdateProviderRunSelection/model"),
        Some(&serde_json::json!("openai/gpt-5.4"))
    );
    assert_eq!(
        snapshot.pointer("/UpdateProviderRunSelection/variant"),
        Some(&serde_json::json!("high"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("provider selection snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "bce42e6fc169c8747199a98a5dc059e5b40ac7f8aafb0f9a7a67f4b336ef57e5"
    );
}

#[test]
fn local_daemon_protocol_terminal_input_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 312);

    let request = LocalDaemonRequest::SendTerminalInput(SendTerminalInputRequest {
        session_id: "session-1".to_string(),
        attachment_id: "attachment-1".to_string(),
        provider_run_id: Some("provider-run-1".to_string()),
        data_base64: "aGVsbG8N".to_string(),
    });
    let snapshot = serde_json::to_value(request).expect("request should serialize");
    assert_eq!(
        snapshot.pointer("/SendTerminalInput/session_id"),
        Some(&serde_json::json!("session-1"))
    );
    assert_eq!(
        snapshot.pointer("/SendTerminalInput/attachment_id"),
        Some(&serde_json::json!("attachment-1"))
    );
    assert_eq!(
        snapshot.pointer("/SendTerminalInput/provider_run_id"),
        Some(&serde_json::json!("provider-run-1"))
    );
    assert_eq!(
        snapshot.pointer("/SendTerminalInput/data_base64"),
        Some(&serde_json::json!("aGVsbG8N"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("terminal input snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "95c089680846665e95d16114c5c6245b2f4e49c0f0b1dfdc390c62a9f1ff836a"
    );
}

#[test]
fn local_daemon_protocol_terminal_output_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 312);

    let batch_request = LocalDaemonRequest::AppendNativeProviderOutputBatch(
        AppendNativeProviderOutputBatchRequest {
            session_id: "session-1".to_string(),
            attachment_id: "attachment-1".to_string(),
            outputs: vec![
                AppendNativeProviderOutputBatchItem {
                    provider_run_id: "provider-run-1".to_string(),
                    kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                    merge_key: Some("batch-1".to_string()),
                    text: "hello\n".to_string(),
                },
                AppendNativeProviderOutputBatchItem {
                    provider_run_id: "provider-run-2".to_string(),
                    kind: crate::terminal::TerminalOutputKind::ProviderReasoning,
                    merge_key: None,
                    text: "thinking\n".to_string(),
                },
            ],
        },
    );
    let batch_snapshot =
        serde_json::to_value(batch_request).expect("batch output request should serialize");
    assert_eq!(
        batch_snapshot.pointer("/AppendNativeProviderOutputBatch/outputs/0/kind"),
        Some(&serde_json::json!("provider_output"))
    );
    assert_eq!(
        batch_snapshot.pointer("/AppendNativeProviderOutputBatch/outputs/1/kind"),
        Some(&serde_json::json!("provider_reasoning"))
    );

    let response = LocalDaemonResponse::TerminalOutput {
        records: vec![
            crate::terminal::TerminalOutputRecord {
                record_id: Some(7),
                timestamp_ms: 1_700_000_000_000,
                session_id: "session-1".to_string(),
                provider_run_id: "provider-run-1".to_string(),
                agent_id: Some("agent-1".to_string()),
                prompt_id: Some("prompt-42".to_string()),
                prompt_origin: Some(crate::session::PromptOrigin::External),
                source_attachment_id: Some("attachment-1".to_string()),
                kind: crate::terminal::TerminalOutputKind::PromptEcho,
                merge_key: None,
                recipient_attachment_ids: vec!["attachment-2".to_string()],
                pending_recipient_attachment_ids: vec!["attachment-2".to_string()],
                bytes: b"hello\n".to_vec(),
                external_observation_metadata: Some(
                    crate::terminal::TerminalOutputExternalObservationMetadata {
                        source: crate::history::SessionHistoryEntrySource::ExternalProviderObserved,
                        external_provider: Some("codex".to_string()),
                        external_provider_session_id: Some("thread-1".to_string()),
                        external_provider_turn_id: Some("turn-1".to_string()),
                        observed_at_ms: Some(1_234),
                        external_observation: Some(
                            crate::history::SessionHistoryExternalObservation {
                                settles_active_prompt: true,
                                passive_telemetry: false,
                            },
                        ),
                    },
                ),
            },
            crate::terminal::TerminalOutputRecord {
                record_id: Some(8),
                timestamp_ms: 1_700_000_000_001,
                session_id: "session-1".to_string(),
                provider_run_id: "provider-run-1".to_string(),
                agent_id: Some("agent-1".to_string()),
                prompt_id: Some("prompt-42".to_string()),
                prompt_origin: Some(crate::session::PromptOrigin::Chariox),
                source_attachment_id: Some("attachment-1".to_string()),
                kind: crate::terminal::TerminalOutputKind::ProviderTerminal,
                merge_key: None,
                recipient_attachment_ids: vec!["attachment-2".to_string()],
                pending_recipient_attachment_ids: vec!["attachment-2".to_string()],
                bytes: b"\x1b[2Jfullscreen".to_vec(),
                external_observation_metadata: None,
            },
        ],
    };
    let snapshot = serde_json::to_value(response).expect("response should serialize");
    assert_eq!(
        snapshot.pointer("/TerminalOutput/records/0/record_id"),
        Some(&serde_json::json!(7))
    );
    assert_eq!(
        snapshot.pointer("/TerminalOutput/records/0/timestamp_ms"),
        Some(&serde_json::json!(1_700_000_000_000u64))
    );
    assert_eq!(
        snapshot.pointer("/TerminalOutput/records/0/kind"),
        Some(&serde_json::json!("prompt_echo"))
    );
    assert_eq!(
        snapshot.pointer("/TerminalOutput/records/0/prompt_id"),
        Some(&serde_json::json!("prompt-42"))
    );
    assert_eq!(
        snapshot.pointer("/TerminalOutput/records/0/prompt_origin"),
        Some(&serde_json::json!("external"))
    );
    assert_eq!(
        snapshot.pointer("/TerminalOutput/records/0/source_attachment_id"),
        Some(&serde_json::json!("attachment-1"))
    );
    assert_eq!(
        snapshot.pointer("/TerminalOutput/records/0/source"),
        Some(&serde_json::json!("external_provider_observed"))
    );
    assert_eq!(
        snapshot.pointer("/TerminalOutput/records/0/external_provider"),
        Some(&serde_json::json!("codex"))
    );
    assert_eq!(
        snapshot.pointer("/TerminalOutput/records/0/external_provider_session_id"),
        Some(&serde_json::json!("thread-1"))
    );
    assert_eq!(
        snapshot.pointer("/TerminalOutput/records/0/external_provider_turn_id"),
        Some(&serde_json::json!("turn-1"))
    );
    assert_eq!(
        snapshot.pointer("/TerminalOutput/records/0/external_observation/settles_active_prompt"),
        Some(&serde_json::json!(true))
    );
    assert_eq!(
        snapshot.pointer("/TerminalOutput/records/1/kind"),
        Some(&serde_json::json!("provider_terminal"))
    );
    let snapshots = serde_json::json!({
        "batch_request": batch_snapshot,
        "response": snapshot,
    });
    let serialized =
        serde_json::to_string(&snapshots).expect("terminal output snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "e482165e9cbadadaa8daff7cac05690baf98eb18441fc7a78786c090b339ef6d"
    );
}

#[test]
fn local_daemon_protocol_metaagent_event_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 312);

    let search =
        LocalDaemonRequest::SearchMetaagentCommands(crate::local::SearchMetaagentCommandsRequest {
            session_id: "session-1".to_string(),
            metaagent_id: "meta-1".to_string(),
            query: Some("agent".to_string()),
            tag: Some("agent".to_string()),
            scope: Some("session".to_string()),
            mutates: Some(true),
            policy: Some("allow".to_string()),
            limit: Some(5),
        });
    let turn_overview = LocalDaemonRequest::GetMetaagentTurnOverview(
        crate::local::GetMetaagentTurnOverviewRequest {
            session_id: "session-1".to_string(),
            metaagent_id: "meta-1".to_string(),
            agent_ref: Some("worker".to_string()),
            turn_ref: Some("turn-1".to_string()),
            turns_back: Some(1),
            limit: Some(50),
        },
    );
    let turn_blob =
        LocalDaemonRequest::GetMetaagentTurnBlob(crate::local::GetMetaagentTurnBlobRequest {
            session_id: "session-1".to_string(),
            metaagent_id: "meta-1".to_string(),
            blob_id: "blob-1".to_string(),
        });
    let list = LocalDaemonRequest::ListMetaagentEvents(crate::local::ListMetaagentEventsRequest {
        session_id: "session-1".to_string(),
        metaagent_id: "meta-1".to_string(),
        limit: Some(20),
        status: Some("unread".to_string()),
        kind: Some("agent.turn.completed".to_string()),
    });
    let read = LocalDaemonRequest::ReadMetaagentEvent(crate::local::ReadMetaagentEventRequest {
        session_id: "session-1".to_string(),
        metaagent_id: "meta-1".to_string(),
        event_id: "event-1".to_string(),
    });
    let ack = LocalDaemonRequest::AckMetaagentEvents(crate::local::AckMetaagentEventsRequest {
        session_id: "session-1".to_string(),
        metaagent_id: "meta-1".to_string(),
        event_id: Some("event-1".to_string()),
        event_ids: Some(vec!["event-2".to_string()]),
        up_to_sequence: Some(7),
    });
    let update_task =
        LocalDaemonRequest::UpdateMetaagentTask(crate::local::UpdateMetaagentTaskRequest {
            session_id: "session-1".to_string(),
            metaagent_id: "meta-1".to_string(),
            task_markdown: Some("# Task".to_string()),
            plan_markdown: Some("1. Delegate.".to_string()),
        });
    let pause_task =
        LocalDaemonRequest::PauseMetaagentTask(crate::local::PauseMetaagentTaskRequest {
            session_id: "session-1".to_string(),
            metaagent_id: "meta-1".to_string(),
        });
    let resume_task =
        LocalDaemonRequest::ResumeMetaagentTask(crate::local::ResumeMetaagentTaskRequest {
            session_id: "session-1".to_string(),
            metaagent_id: "meta-1".to_string(),
        });
    let abort_task =
        LocalDaemonRequest::AbortMetaagentTask(crate::local::AbortMetaagentTaskRequest {
            session_id: "session-1".to_string(),
            metaagent_id: "meta-1".to_string(),
            reason: Some("user aborted".to_string()),
        });
    let event = serde_json::json!({
        "event_id": "event-1",
        "metaagent_id": "meta-1",
        "kind": "agent.turn.completed",
        "sequence": 7,
    });
    let listed = LocalDaemonResponse::MetaagentEventsListed {
        events: vec![event.clone()],
    };
    let searched = LocalDaemonResponse::MetaagentCommandsSearched {
        commands: vec![serde_json::json!({
            "name": "agent list",
            "usage": "agent list",
            "scope": "session",
            "policy": "allow",
        })],
    };
    let overview_response = LocalDaemonResponse::MetaagentTurnOverview {
        overview: serde_json::json!({
            "agent": { "id": "agent-1", "alias": "worker" },
            "turns": [{ "turn_id": "turn-1", "items": [] }],
        }),
    };
    let blob_response = LocalDaemonResponse::MetaagentTurnBlob {
        blob: serde_json::json!({
            "agent": { "id": "agent-1", "alias": "worker" },
            "blob_id": "blob-1",
            "entries": [],
        }),
    };
    let read_response = LocalDaemonResponse::MetaagentEventRead {
        event: event.clone(),
    };
    let acked = LocalDaemonResponse::MetaagentEventsAcked { acked: vec![event] };
    let mut task_session = crate::session::RuntimeSession::new(
        "session-1",
        None,
        "/repo",
        "/repo",
        "machine-1",
        "daemon-1",
    );
    task_session.update_metaagent_task_markdown("meta-1", "# Task");
    task_session.update_metaagent_plan_markdown("meta-1", "1. Delegate.");
    let task = task_session.metaagent_task("meta-1").cloned();
    let task_response = LocalDaemonResponse::MetaagentTaskUpdated {
        session: task_session,
        task,
    };
    let mut snapshot = serde_json::json!([
        search,
        turn_overview,
        turn_blob,
        list,
        read,
        ack,
        update_task,
        pause_task,
        resume_task,
        abort_task,
        listed,
        searched,
        overview_response,
        blob_response,
        read_response,
        acked,
        task_response
    ]);
    *snapshot
        .pointer_mut("/16/MetaagentTaskUpdated/session/created_at_ms")
        .expect("session created_at_ms should encode") = serde_json::json!(42);
    *snapshot
        .pointer_mut("/16/MetaagentTaskUpdated/session/last_used_at_ms")
        .expect("session last_used_at_ms should encode") = serde_json::json!(42);
    *snapshot
        .pointer_mut("/16/MetaagentTaskUpdated/session/metaagent_tasks/0/task_id")
        .expect("session task_id should encode") = serde_json::json!("metaagent-task-1");
    *snapshot
        .pointer_mut("/16/MetaagentTaskUpdated/session/metaagent_tasks/0/created_at_ms")
        .expect("session task created_at_ms should encode") = serde_json::json!(42);
    *snapshot
        .pointer_mut("/16/MetaagentTaskUpdated/session/metaagent_tasks/0/updated_at_ms")
        .expect("session task updated_at_ms should encode") = serde_json::json!(42);
    *snapshot
        .pointer_mut("/16/MetaagentTaskUpdated/task/task_id")
        .expect("response task_id should encode") = serde_json::json!("metaagent-task-1");
    *snapshot
        .pointer_mut("/16/MetaagentTaskUpdated/task/created_at_ms")
        .expect("response task created_at_ms should encode") = serde_json::json!(42);
    *snapshot
        .pointer_mut("/16/MetaagentTaskUpdated/task/updated_at_ms")
        .expect("response task updated_at_ms should encode") = serde_json::json!(42);

    assert_eq!(
        snapshot.pointer("/0/SearchMetaagentCommands/query"),
        Some(&serde_json::json!("agent"))
    );
    assert_eq!(
        snapshot.pointer("/1/GetMetaagentTurnOverview/agent_ref"),
        Some(&serde_json::json!("worker"))
    );
    assert_eq!(
        snapshot.pointer("/2/GetMetaagentTurnBlob/blob_id"),
        Some(&serde_json::json!("blob-1"))
    );
    assert_eq!(
        snapshot.pointer("/3/ListMetaagentEvents/metaagent_id"),
        Some(&serde_json::json!("meta-1"))
    );
    assert_eq!(
        snapshot.pointer("/4/ReadMetaagentEvent/event_id"),
        Some(&serde_json::json!("event-1"))
    );
    assert_eq!(
        snapshot.pointer("/5/AckMetaagentEvents/up_to_sequence"),
        Some(&serde_json::json!(7))
    );
    assert_eq!(
        snapshot.pointer("/6/UpdateMetaagentTask/task_markdown"),
        Some(&serde_json::json!("# Task"))
    );
    assert_eq!(
        snapshot.pointer("/7/PauseMetaagentTask/metaagent_id"),
        Some(&serde_json::json!("meta-1"))
    );
    assert_eq!(
        snapshot.pointer("/8/ResumeMetaagentTask/session_id"),
        Some(&serde_json::json!("session-1"))
    );
    assert_eq!(
        snapshot.pointer("/9/AbortMetaagentTask/reason"),
        Some(&serde_json::json!("user aborted"))
    );
    assert_eq!(
        snapshot.pointer("/10/MetaagentEventsListed/events/0/kind"),
        Some(&serde_json::json!("agent.turn.completed"))
    );
    assert_eq!(
        snapshot.pointer("/11/MetaagentCommandsSearched/commands/0/name"),
        Some(&serde_json::json!("agent list"))
    );
    assert_eq!(
        snapshot.pointer("/12/MetaagentTurnOverview/overview/turns/0/turn_id"),
        Some(&serde_json::json!("turn-1"))
    );
    assert_eq!(
        snapshot.pointer("/13/MetaagentTurnBlob/blob/blob_id"),
        Some(&serde_json::json!("blob-1"))
    );
    assert_eq!(
        snapshot.pointer("/14/MetaagentEventRead/event/event_id"),
        Some(&serde_json::json!("event-1"))
    );
    assert_eq!(
        snapshot.pointer("/15/MetaagentEventsAcked/acked/0/sequence"),
        Some(&serde_json::json!(7))
    );
    assert_eq!(
        snapshot.pointer("/16/MetaagentTaskUpdated/task/plan_markdown"),
        Some(&serde_json::json!("1. Delegate."))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("metaagent event protocol shape should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "a806fee4ae9d00f189581a767bc55296a3c98e380f5b8a0d925b3479ca7e6cd4"
    );
}

#[test]
fn local_daemon_protocol_remote_inventory_provider_accounts_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 312);

    let account = RelayProviderAccountSummary {
        provider: "codex".to_string(),
        state: "configured".to_string(),
        auth_type: Some("chatgpt".to_string()),
        account_id: Some("acct-remote-1".to_string()),
        email: None,
        organization_id: None,
        organization_name: None,
        subscription_type: None,
        alias: Some("remote-codex".to_string()),
    };
    let machine = crate::local::RemoteMachineRecord {
        machine_id: "machine-1".to_string(),
        machine_alias: Some("workstation".to_string()),
        registry_alias: Some("builder".to_string()),
        display_name: "builder".to_string(),
        trust_status: RemoteMachineTrustStatus::Approved,
        online: true,
        pending: false,
        kernel_count: 1,
        available_providers: vec!["codex".to_string()],
        provider_accounts: vec![account.clone()],
    };
    let kernel = RelayKernelPresence {
        kernel_id: "kernel-1".to_string(),
        machine_id: "machine-1".to_string(),
        machine_alias: Some("workstation".to_string()),
        relay_alias: Some("builder".to_string()),
        kernel_alias: Some("default".to_string()),
        available_providers: vec!["codex".to_string()],
        provider_accounts: vec![account],
        capabilities: vec!["kernel_ws".to_string()],
        accepting_remote_leases: true,
        leased_agent_count: 1,
        local_session_count: 2,
        public_key: "public-key".to_string(),
    };
    let snapshot = serde_json::json!([
        LocalDaemonResponse::RemoteMachinesListed {
            machines: vec![machine],
        },
        LocalDaemonResponse::RemoteMachineKernelsListed {
            machine_ref: "builder".to_string(),
            kernels: vec![kernel],
        },
    ]);
    assert_eq!(
        snapshot.pointer("/0/RemoteMachinesListed/machines/0/provider_accounts/0/alias"),
        Some(&serde_json::json!("remote-codex"))
    );
    assert_eq!(
        snapshot.pointer("/1/RemoteMachineKernelsListed/kernels/0/provider_accounts/0/account_id"),
        Some(&serde_json::json!("acct-remote-1"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("remote inventory account snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "aab66ad2af2311174b8fa36f3143c7760b078509028d29b17f6a89adf3d0c03d"
    );
}

#[test]
fn local_daemon_protocol_kernel_client_connection_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 312);

    let snapshot = serde_json::json!([
        LocalDaemonRequest::ResolveKernelClientConnection(ResolveKernelClientConnectionRequest {
            kernel_ref: "kernel-1".to_string(),
            machine_ref: Some("machine-1".to_string()),
            client_id: Some("cli-1".to_string()),
            session_id: None,
        }),
        LocalDaemonResponse::KernelClientConnectionResolved {
            connection: KernelClientConnection {
                relay_url: "wss://relay.example".to_string(),
                relay_token: "scoped-token".to_string(),
                target_daemon_id: Some("kernel-1".to_string()),
                target_daemon_alias: Some("builder-kernel".to_string()),
                token_expires_at: Some("2026-06-26T12:00:00Z".to_string()),
                machine_id: Some("machine-1".to_string()),
                kernel_id: Some("kernel-1".to_string()),
            },
        },
    ]);
    assert_eq!(
        snapshot.pointer("/0/ResolveKernelClientConnection/kernel_ref"),
        Some(&serde_json::json!("kernel-1"))
    );
    assert_eq!(
        snapshot.pointer("/1/KernelClientConnectionResolved/connection/target_daemon_alias"),
        Some(&serde_json::json!("builder-kernel"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("kernel client connection snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "623235862705f7aa8f85c6b29f98188caa2b75ac7acf0f8f72f63f822b888e9a"
    );
}
