use super::*;

#[test]
fn outline_turn_joins_trailing_assistant_fragments_into_complete_summary() {
    let context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        prompt_id: Some("prompt-1".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    let prompt = HistoryEvent::transcript(
        10,
        &SessionHistoryEntry::user_prompt("session-1", "attachment-1", "agent-1", "hello"),
        context.clone(),
    );
    let fragments = ["The complete ", "assistant reply", "."]
        .into_iter()
        .enumerate()
        .map(|(index, text)| {
            HistoryEvent::transcript(
                11 + index as u64,
                &SessionHistoryEntry::provider_output(
                    "session-1",
                    "run-1",
                    Some("agent-1"),
                    TerminalOutputKind::ProviderOutput,
                    None,
                    text,
                ),
                context.clone(),
            )
        })
        .collect::<Vec<_>>();
    let mut events = vec![prompt.clone()];
    events.extend(fragments);

    let turn = outline_turn_from_events(&prompt, events, false).expect("turn should be outlined");

    assert!(turn.entries.is_empty());
    assert_eq!(
        turn.summary.as_ref().map(|entry| entry.entry.text.as_str()),
        Some("The complete assistant reply.")
    );
    assert_eq!(turn.lifecycle, SessionHistoryOutlineTurnLifecycle::Open);
    assert_eq!(turn.completed_at_ms, None);
}

#[test]
fn outline_turn_restores_provider_fragment_order_when_concurrent_history_writes_cross() {
    let context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        prompt_id: Some("prompt-1".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    let prompt = HistoryEvent::transcript(
        10,
        &SessionHistoryEntry::user_prompt("session-1", "attachment-1", "agent-1", "hello"),
        context.clone(),
    );
    let fragments = [
        (11, 100, "new "),
        (12, 300, "both "),
        (13, 200, "turn "),
        (14, 400, "placeholder"),
    ]
    .into_iter()
    .map(|(sequence, timestamp_ms, text)| {
        let mut event = HistoryEvent::transcript(
            sequence,
            &SessionHistoryEntry::provider_output(
                "session-1",
                "run-1",
                Some("agent-1"),
                TerminalOutputKind::ProviderOutput,
                Some("msg-provider-final-1".to_string()),
                text,
            ),
            context.clone(),
        );
        event.timestamp_ms = timestamp_ms;
        event
    })
    .collect::<Vec<_>>();
    let mut events = vec![prompt.clone()];
    events.extend(fragments);

    let turn = outline_turn_from_events(&prompt, events, false).expect("turn should be outlined");

    assert_eq!(
        turn.summary.as_ref().map(|entry| entry.entry.text.as_str()),
        Some("new turn both placeholder")
    );
}

#[test]
fn outline_latest_chariox_turn_does_not_infer_completion_from_output() {
    let context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("prompt-1".to_string()),
        prompt_id: Some("prompt-1".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    let prompt = HistoryEvent::transcript(
        10,
        &SessionHistoryEntry::user_prompt("session-1", "attachment-1", "agent-1", "hello"),
        context.clone(),
    );
    let output = HistoryEvent::transcript(
        11,
        &SessionHistoryEntry::provider_output(
            "session-1",
            "run-1",
            Some("agent-1"),
            TerminalOutputKind::ProviderOutput,
            None,
            "still streaming",
        ),
        context,
    );

    let turn = outline_turn_from_events(&prompt, vec![prompt.clone(), output], false)
        .expect("active Chariox turn should be outlined");

    assert_eq!(turn.prompt_origin, PromptOrigin::Chariox);
    assert_eq!(turn.lifecycle, SessionHistoryOutlineTurnLifecycle::Open);
    assert_eq!(turn.completed_at_ms, None);
    assert_eq!(
        turn.summary.as_ref().map(|entry| entry.entry.text.as_str()),
        Some("still streaming")
    );
}

#[test]
fn outline_latest_chariox_turn_uses_hidden_prompt_settlement_timestamp() {
    let context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("prompt-1".to_string()),
        prompt_id: Some("prompt-1".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    let mut prompt = HistoryEvent::transcript(
        10,
        &SessionHistoryEntry::user_prompt("session-1", "attachment-1", "agent-1", "hello"),
        context.clone(),
    );
    prompt.timestamp_ms = 9_000;
    let mut output = HistoryEvent::transcript(
        11,
        &SessionHistoryEntry::provider_output(
            "session-1",
            "run-1",
            Some("agent-1"),
            TerminalOutputKind::ProviderOutput,
            None,
            "finished response",
        ),
        context.clone(),
    );
    output.timestamp_ms = 9_500;
    let settlement = HistoryEvent::operational(
        12,
        HistoryEventKind::ProviderStatus,
        Some(crate::history::HistoryEventRole::System),
        None,
        BTreeMap::from([(
            crate::history::PROMPT_SETTLED_AT_MS_METADATA_KEY.to_string(),
            serde_json::json!(9_876),
        )]),
        context,
    );

    let turn = outline_turn_from_events(&prompt, vec![prompt.clone(), output, settlement], false)
        .expect("settled Chariox turn should be outlined");

    assert_eq!(
        turn.lifecycle,
        SessionHistoryOutlineTurnLifecycle::Completed
    );
    assert_eq!(turn.completed_at_ms, Some(9_876));
    assert!(turn.entries.is_empty(), "hidden settlement must not render");
    assert_eq!(
        turn.summary.as_ref().map(|entry| entry.entry.text.as_str()),
        Some("finished response")
    );
}

#[test]
fn outline_cancelled_chariox_turn_preserves_cancelled_lifecycle() {
    let context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("prompt-1".to_string()),
        prompt_id: Some("prompt-1".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    let mut prompt = HistoryEvent::transcript(
        10,
        &SessionHistoryEntry::user_prompt("session-1", "attachment-1", "agent-1", "hello"),
        context.clone(),
    );
    prompt.timestamp_ms = 9_000;
    let settlement = HistoryEvent::operational(
        11,
        HistoryEventKind::ProviderStatus,
        Some(crate::history::HistoryEventRole::System),
        None,
        BTreeMap::from([
            (
                crate::history::PROMPT_SETTLED_AT_MS_METADATA_KEY.to_string(),
                serde_json::json!(9_876),
            ),
            (
                crate::history::PROMPT_SETTLEMENT_STATUS_METADATA_KEY.to_string(),
                serde_json::json!("cancelled"),
            ),
        ]),
        context,
    );

    let turn = outline_turn_from_events(&prompt, vec![prompt.clone(), settlement], false)
        .expect("cancelled turn should be outlined");

    assert_eq!(
        turn.lifecycle,
        SessionHistoryOutlineTurnLifecycle::Cancelled
    );
    assert_eq!(turn.completed_at_ms, Some(9_876));
    assert!(turn.summary.is_none());
}

#[test]
fn outline_completed_turn_never_settles_before_its_final_output() {
    let context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("prompt-1".to_string()),
        prompt_id: Some("prompt-1".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    let prompt = HistoryEvent::transcript(
        10,
        &SessionHistoryEntry::user_prompt("session-1", "attachment-1", "agent-1", "hello"),
        context.clone(),
    );
    let settlement = HistoryEvent::operational(
        11,
        HistoryEventKind::ProviderStatus,
        Some(crate::history::HistoryEventRole::System),
        None,
        BTreeMap::from([(
            crate::history::PROMPT_SETTLED_AT_MS_METADATA_KEY.to_string(),
            serde_json::json!(1_500),
        )]),
        context.clone(),
    );
    let final_output = HistoryEvent::transcript(
        12,
        &SessionHistoryEntry::provider_output(
            "session-1",
            "run-1",
            Some("agent-1"),
            TerminalOutputKind::ProviderOutput,
            Some("assistant".to_string()),
            "finished response",
        ),
        context,
    );
    let final_output_at_ms = final_output.timestamp_ms;

    let turn = outline_turn_from_events(
        &prompt,
        vec![prompt.clone(), settlement, final_output],
        false,
    )
    .expect("settled turn should be outlined");

    assert_eq!(
        turn.lifecycle,
        SessionHistoryOutlineTurnLifecycle::Completed
    );
    assert_eq!(turn.completed_at_ms, Some(final_output_at_ms));
}

#[test]
fn outline_completed_turn_ignores_late_notice_without_a_settlement_marker() {
    let context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    let mut prompt = HistoryEvent::transcript(
        10,
        &SessionHistoryEntry::user_prompt("session-1", "attachment-1", "agent-1", "first prompt"),
        context.clone(),
    );
    prompt.timestamp_ms = 1_000;
    let mut output = HistoryEvent::transcript(
        11,
        &SessionHistoryEntry::provider_output(
            "session-1",
            "run-1",
            Some("agent-1"),
            TerminalOutputKind::ProviderOutput,
            None,
            "finished response",
        ),
        context.clone(),
    );
    output.timestamp_ms = 2_000;
    let mut queued = HistoryEvent::transcript(
        12,
        &SessionHistoryEntry::notice(
            "session-1",
            Some("run-1"),
            Some("agent-1"),
            "Attachment `attachment-2` queued prompt `pending-2` for agent `agent-1`.",
        ),
        context,
    );
    queued.timestamp_ms = 100_000;

    let turn = outline_turn_from_events(&prompt, vec![prompt.clone(), output, queued], true)
        .expect("completed turn should be outlined");

    assert_eq!(
        turn.lifecycle,
        SessionHistoryOutlineTurnLifecycle::Completed
    );
    assert_eq!(turn.completed_at_ms, Some(2_000));
}

#[test]
fn agent_outline_uses_suppressed_observer_settlements_for_chariox_owned_turns() {
    let path = std::env::temp_dir().join(format!(
        "chariox-observer-settlement-outline-{}-{}.db",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history store should open");
    let context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    let mut events = Vec::new();
    let mut sequence = 10;

    for (index, base_ms) in [(1, 1_000_u64), (2, 100_000_u64)] {
        let prompt_text = format!("workflow prompt {index}");
        let attachment_id = format!("workflow-run:{index}");
        let mut prompt = HistoryEvent::transcript(
            sequence,
            &SessionHistoryEntry::user_prompt(
                "session-1",
                &attachment_id,
                "agent-1",
                prompt_text.clone(),
            ),
            context.clone(),
        );
        prompt.timestamp_ms = base_ms;
        events.push(prompt);
        sequence += 1;

        let mut output = HistoryEvent::transcript(
            sequence,
            &SessionHistoryEntry::provider_output(
                "session-1",
                "run-1",
                Some("agent-1"),
                TerminalOutputKind::ProviderOutput,
                Some(format!("owned-output-{index}")),
                format!("owned response {index}"),
            )
            .with_source_attachment_id(Some(attachment_id.clone())),
            context.clone(),
        );
        output.timestamp_ms = base_ms + 1_000;
        events.push(output);
        sequence += 1;

        let observed_prompt = SessionHistoryEntry::external_provider_observed(
            "session-1",
            Some("run-1"),
            "agent-1",
            SessionHistoryEntryKind::UserPrompt,
            prompt_text,
            "codex",
            "thread-1",
            Some(format!("observed-user-{index}")),
            Some(base_ms + 100),
        );
        let mut observed_prompt_event =
            HistoryEvent::transcript(sequence, &observed_prompt, context.clone());
        observed_prompt_event.timestamp_ms = base_ms + 100;
        events.push(observed_prompt_event);
        sequence += 1;

        let observed_output = SessionHistoryEntry::external_provider_observed(
            "session-1",
            Some("run-1"),
            "agent-1",
            SessionHistoryEntryKind::ProviderOutput,
            format!("observed response {index}"),
            "codex",
            "thread-1",
            Some(format!("observed-user-{index}")),
            Some(base_ms + 900),
        );
        let mut observed_output_event =
            HistoryEvent::transcript(sequence, &observed_output, context.clone());
        observed_output_event.timestamp_ms = base_ms + 900;
        events.push(observed_output_event);
        sequence += 1;

        let mut settlement = SessionHistoryEntry::external_provider_observed(
            "session-1",
            Some("run-1"),
            "agent-1",
            SessionHistoryEntryKind::ProviderStatus,
            "codex task_complete",
            "codex",
            "thread-1",
            Some(format!("observed-user-{index}")),
            Some(base_ms + 1_100),
        );
        settlement.external_observation = Some(crate::history::SessionHistoryExternalObservation {
            settles_active_prompt: true,
            passive_telemetry: false,
        });
        let mut settlement_event = HistoryEvent::transcript(sequence, &settlement, context.clone());
        settlement_event.timestamp_ms = base_ms + 1_100;
        events.push(settlement_event);
        sequence += 1;

        if index == 1 {
            let mut queued = HistoryEvent::transcript(
                sequence,
                &SessionHistoryEntry::notice(
                    "session-1",
                    Some("run-1"),
                    Some("agent-1"),
                    "Attachment `workflow-run:2` queued prompt `pending-2` for agent `agent-1`.",
                ),
                context.clone(),
            );
            queued.timestamp_ms = 99_900;
            events.push(queued);
            sequence += 1;
        }
    }

    store
        .append_many(&events)
        .expect("workflow transcript should append");

    let outline = load_agent_outline(&store, "session-1", "agent-1", 2, None)
        .expect("agent outline should load");

    assert_eq!(outline.turns.len(), 2);
    assert_eq!(outline.turns[0].completed_at_ms, Some(2_100));
    assert_eq!(outline.turns[1].completed_at_ms, Some(101_100));
    assert!(outline
        .turns
        .iter()
        .all(|turn| turn.lifecycle == SessionHistoryOutlineTurnLifecycle::Completed));
    assert!(outline
        .turns
        .iter()
        .all(|turn| turn.external_provider.is_none()));
    assert_eq!(
        outline.turns[0]
            .summary
            .as_ref()
            .map(|entry| entry.entry.text.as_str()),
        Some("owned response 1")
    );
    assert_eq!(
        outline.turns[1]
            .summary
            .as_ref()
            .map(|entry| entry.entry.text.as_str()),
        Some("owned response 2")
    );

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn agent_outline_joins_prompt_settlement_that_persisted_before_prompt_history() {
    let path = std::env::temp_dir().join(format!(
        "chariox-out-of-order-settlement-outline-{}-{}.db",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history store should open");
    let context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("prompt-1".to_string()),
        prompt_id: Some("prompt-1".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    let settlement = HistoryEvent::operational(
        10,
        HistoryEventKind::ProviderStatus,
        Some(crate::history::HistoryEventRole::System),
        None,
        BTreeMap::from([(
            crate::history::PROMPT_SETTLED_AT_MS_METADATA_KEY.to_string(),
            serde_json::json!(9_876),
        )]),
        context.clone(),
    );
    let mut prompt = HistoryEvent::transcript(
        11,
        &SessionHistoryEntry::user_prompt("session-1", "attachment-1", "agent-1", "hello"),
        context.clone(),
    );
    prompt.timestamp_ms = 9_000;
    let mut output = HistoryEvent::transcript(
        12,
        &SessionHistoryEntry::provider_output(
            "session-1",
            "run-1",
            Some("agent-1"),
            TerminalOutputKind::ProviderOutput,
            None,
            "finished response",
        ),
        context,
    );
    output.timestamp_ms = 9_500;
    store
        .append_many(&[settlement, prompt, output])
        .expect("out-of-order prompt lifecycle should append");

    let outline = load_agent_outline(&store, "session-1", "agent-1", 1, None)
        .expect("agent outline should load");

    assert_eq!(outline.turns.len(), 1);
    assert_eq!(outline.turns[0].completed_at_ms, Some(9_876));
    assert_eq!(
        outline.turns[0].lifecycle,
        SessionHistoryOutlineTurnLifecycle::Completed
    );

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn outline_turn_uses_transcript_admission_for_provider_status() {
    let context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        prompt_id: Some("prompt-1".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    let mut prompt = HistoryEvent::transcript(
        10,
        &SessionHistoryEntry::user_prompt("session-1", "attachment-1", "agent-1", "hello"),
        context.clone(),
    );
    prompt.timestamp_ms = 10;
    let mut assistant = HistoryEvent::transcript(
        11,
        &SessionHistoryEntry::provider_output(
            "session-1",
            "run-1",
            Some("agent-1"),
            TerminalOutputKind::ProviderOutput,
            None,
            "assistant body before tool",
        ),
        context.clone(),
    );
    assistant.timestamp_ms = 11;
    let mut tool = HistoryEvent::transcript(
        12,
        &SessionHistoryEntry::provider_output(
            "session-1",
            "run-1",
            Some("agent-1"),
            TerminalOutputKind::ProviderTool,
            Some("tool-1".to_string()),
            r#"{"tool":"bash","status":"completed","input":{"command":"echo ok"},"output":"detail"}"#,
        ),
        context.clone(),
    );
    tool.timestamp_ms = 12;
    let mut status = HistoryEvent::transcript(
        13,
        &SessionHistoryEntry::provider_output(
            "session-1",
            "run-1",
            Some("agent-1"),
            TerminalOutputKind::ProviderStatus,
            Some("status-1".to_string()),
            "codex token_count\n{\"info\":{\"total_token_usage\":{\"total_tokens\":42}}}",
        ),
        context.clone(),
    );
    status.timestamp_ms = 13;
    let mut external_status_entry = SessionHistoryEntry::external_provider_observed(
        "session-1",
        None,
        "agent-1",
        SessionHistoryEntryKind::ProviderStatus,
        "codex task_complete",
        "codex",
        "thread-1",
        Some("done-1".to_string()),
        Some(15),
    );
    external_status_entry.external_observation =
        Some(crate::history::SessionHistoryExternalObservation {
            settles_active_prompt: true,
            passive_telemetry: false,
        });
    let mut external_status = HistoryEvent::transcript(15, &external_status_entry, context.clone());
    external_status.timestamp_ms = 15;
    let mut passive_status_entry = SessionHistoryEntry::external_provider_observed(
        "session-1",
        None,
        "agent-1",
        SessionHistoryEntryKind::ProviderStatus,
        "codex token_count\n{\"info\":{\"total_token_usage\":{\"total_tokens\":42}}}",
        "codex",
        "thread-1",
        Some("token-count-1".to_string()),
        Some(16),
    );
    passive_status_entry.external_observation =
        Some(crate::history::SessionHistoryExternalObservation {
            settles_active_prompt: false,
            passive_telemetry: true,
        });
    let mut passive_status = HistoryEvent::transcript(16, &passive_status_entry, context.clone());
    passive_status.timestamp_ms = 16;
    let mut summary = HistoryEvent::transcript(
        14,
        &SessionHistoryEntry::provider_output(
            "session-1",
            "run-1",
            Some("agent-1"),
            TerminalOutputKind::ProviderOutput,
            None,
            "final assistant body",
        ),
        context,
    );
    summary.timestamp_ms = 14;

    let turn = outline_turn_from_events(
        &prompt,
        vec![
            prompt.clone(),
            assistant,
            tool,
            status,
            external_status,
            passive_status,
            summary,
        ],
        false,
    )
    .expect("turn should be outlined");

    assert_eq!(turn.prompt_origin, PromptOrigin::Chariox);
    assert_eq!(turn.external_provider.as_deref(), Some("codex"));
    assert_eq!(
        turn.external_provider_session_id.as_deref(),
        Some("thread-1")
    );
    assert_eq!(turn.external_provider_turn_id.as_deref(), Some("done-1"));
    assert_eq!(
        turn.lifecycle,
        SessionHistoryOutlineTurnLifecycle::Completed
    );
    assert_eq!(turn.completed_at_ms, Some(15));
    assert_eq!(turn.entries.len(), 2);
    assert_eq!(
        turn.entries[0].entry.kind,
        SessionHistoryEntryKind::ProviderOutput
    );
    assert_eq!(turn.entries[0].entry.text, "assistant body before tool");
    assert_eq!(
        turn.entries[1].entry.kind,
        SessionHistoryEntryKind::ProviderStatus
    );
    assert!(turn.entries[1].entry.is_external_provider_observed());
    assert_eq!(
        turn.entries[1]
            .entry
            .external_provider_session_id
            .as_deref(),
        Some("thread-1")
    );
    assert_eq!(
        turn.entries[1]
            .entry
            .external_observation
            .as_ref()
            .map(|observation| observation.settles_active_prompt),
        Some(true)
    );
    assert_eq!(turn.blobs.len(), 1);
    assert_eq!(turn.blobs[0].kind, SessionHistoryEntryKind::ProviderTool);
    assert_eq!(
        turn.summary.as_ref().map(|entry| entry.entry.text.as_str()),
        Some("final assistant body")
    );
}

#[test]
fn outline_turn_suppresses_sparse_legacy_reimport_of_keyed_delta() {
    let context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        prompt_id: Some("prompt-1".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    let prompt = HistoryEvent::transcript(
        1,
        &SessionHistoryEntry::user_prompt("session-1", "attachment-1", "agent-1", "reply"),
        context.clone(),
    );
    let chunks = [
        "LOCAL",
        "_DEPLOYED_WORKFLOW_SOURCE",
        "_",
        "39669-178431858622",
        "1",
        "_COMPLE",
        "TED",
    ];
    let mut events = vec![prompt.clone()];
    let mut final_entry = None;
    for (index, text) in chunks.iter().enumerate() {
        let mut entry = SessionHistoryEntry::provider_output(
            "session-1",
            "run-1",
            Some("agent-1"),
            TerminalOutputKind::ProviderOutput,
            Some("message-1".to_string()),
            *text,
        );
        entry.timestamp_ms = 42;
        if index == chunks.len() - 1 {
            final_entry = Some(entry.clone());
        }
        events.push(HistoryEvent::transcript(
            index as u64 + 2,
            &entry,
            context.clone(),
        ));
    }
    events.push(HistoryEvent::transcript(
        chunks.len() as u64 + 2,
        &final_entry.expect("final chunk should exist"),
        HistoryEventTurnContext::default(),
    ));

    let turn = outline_turn_from_events(&prompt, events, false).expect("turn should be outlined");
    let output = turn
        .entries
        .iter()
        .chain(turn.summary.iter())
        .filter(|entry| entry.entry.kind == SessionHistoryEntryKind::ProviderOutput)
        .map(|entry| entry.entry.text.as_str())
        .collect::<String>();

    assert_eq!(
        output,
        "LOCAL_DEPLOYED_WORKFLOW_SOURCE_39669-1784318586221_COMPLETED"
    );
}

#[test]
fn outline_external_turn_without_settlement_stays_incomplete() {
    let observed_at_ms = crate::session::unix_epoch_ms();
    let context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        prompt_id: Some("prompt-1".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    let external_prompt = SessionHistoryEntry::external_provider_observed(
        "session-1",
        Some("run-1"),
        "agent-1",
        SessionHistoryEntryKind::UserPrompt,
        "external prompt",
        "codex",
        "thread-1",
        Some("turn-1".to_string()),
        Some(observed_at_ms),
    );
    let external_assistant = SessionHistoryEntry::external_provider_observed(
        "session-1",
        Some("run-1"),
        "agent-1",
        SessionHistoryEntryKind::ProviderOutput,
        "partial output",
        "codex",
        "thread-1",
        Some("turn-1".to_string()),
        Some(observed_at_ms),
    );
    let prompt = HistoryEvent::transcript(10, &external_prompt, context.clone());
    let assistant = HistoryEvent::transcript(11, &external_assistant, context);

    let turn = outline_turn_from_events(&prompt, vec![prompt.clone(), assistant], false)
        .expect("external active turn should be outlined");

    assert_eq!(turn.prompt_origin, PromptOrigin::External);
    assert_eq!(turn.lifecycle, SessionHistoryOutlineTurnLifecycle::Open);
    assert_eq!(turn.completed_at_ms, None);
    assert_eq!(
        turn.summary.as_ref().map(|entry| entry.entry.text.as_str()),
        Some("partial output")
    );
}

#[test]
fn outline_open_external_turn_keeps_notice_inline_and_reasoning_as_blob() {
    let observed_at_ms = crate::session::unix_epoch_ms();
    let context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        prompt_id: Some("external:opencode:thread-1:user-1".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    let external_prompt = SessionHistoryEntry::external_provider_observed(
        "session-1",
        Some("run-1"),
        "agent-1",
        SessionHistoryEntryKind::UserPrompt,
        "external prompt",
        "opencode",
        "thread-1",
        Some("user-1".to_string()),
        Some(observed_at_ms),
    );
    let notice = SessionHistoryEntry::notice(
        "session-1",
        Some("run-1"),
        Some("agent-1"),
        "Attachment `attachment-1` queued prompt `pending-1` for agent `agent-1`.",
    );
    let reasoning = SessionHistoryEntry::external_provider_observed(
        "session-1",
        Some("run-1"),
        "agent-1",
        SessionHistoryEntryKind::ProviderReasoning,
        "Planning file create, edit, inspect, and delete actions.",
        "opencode",
        "thread-1",
        Some("user-1".to_string()),
        Some(observed_at_ms + 1),
    );
    let status = SessionHistoryEntry::external_provider_observed(
        "session-1",
        Some("run-1"),
        "agent-1",
        SessionHistoryEntryKind::ProviderStatus,
        "opencode message metadata",
        "opencode",
        "thread-1",
        Some("user-1".to_string()),
        Some(observed_at_ms + 2),
    );
    let prompt = HistoryEvent::transcript(10, &external_prompt, context.clone());
    let notice = HistoryEvent::transcript(11, &notice, context.clone());
    let reasoning = HistoryEvent::transcript(12, &reasoning, context.clone());
    let status = HistoryEvent::transcript(13, &status, context);

    let turn = outline_turn_from_events(
        &prompt,
        vec![prompt.clone(), notice, reasoning, status],
        false,
    )
    .expect("open external turn should be outlined");

    assert_eq!(turn.lifecycle, SessionHistoryOutlineTurnLifecycle::Open);
    assert_eq!(turn.completed_at_ms, None);
    assert_eq!(turn.entries.len(), 2);
    assert_eq!(turn.entries[0].entry.kind, SessionHistoryEntryKind::Notice);
    assert_eq!(
        turn.entries[0].entry.text,
        "Attachment `attachment-1` queued prompt `pending-1` for agent `agent-1`."
    );
    assert_eq!(
        turn.entries[1].entry.kind,
        SessionHistoryEntryKind::ProviderStatus
    );
    assert_eq!(turn.blobs.len(), 1);
    assert_eq!(
        turn.blobs[0].kind,
        SessionHistoryEntryKind::ProviderReasoning
    );
    assert_eq!(
        turn.blobs[0].summary,
        "Planning file create, edit, inspect, and delete actions."
    );
}

#[test]
fn outline_stale_external_turn_without_settlement_completes_at_latest_content() {
    let context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        prompt_id: Some("prompt-1".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    let external_prompt = SessionHistoryEntry::external_provider_observed(
        "session-1",
        Some("run-1"),
        "agent-1",
        SessionHistoryEntryKind::UserPrompt,
        "external prompt",
        "codex",
        "thread-1",
        Some("turn-1".to_string()),
        Some(2_000),
    );
    let external_assistant = SessionHistoryEntry::external_provider_observed(
        "session-1",
        Some("run-1"),
        "agent-1",
        SessionHistoryEntryKind::ProviderOutput,
        "final output",
        "codex",
        "thread-1",
        Some("turn-1".to_string()),
        Some(2_100),
    );
    let prompt = HistoryEvent::transcript(10, &external_prompt, context.clone());
    let assistant = HistoryEvent::transcript(11, &external_assistant, context);

    let turn = outline_turn_from_events(&prompt, vec![prompt.clone(), assistant], false)
        .expect("stale external turn should be outlined");

    assert_eq!(turn.prompt_origin, PromptOrigin::External);
    assert_eq!(
        turn.lifecycle,
        SessionHistoryOutlineTurnLifecycle::Completed
    );
    assert_eq!(turn.completed_at_ms, Some(2_100));
}

#[test]
fn outline_turn_uses_persisted_prompt_origin_without_observed_source() {
    let observed_at_ms = crate::session::unix_epoch_ms();
    let context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        prompt_id: Some("prompt-1".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    let external_prompt = SessionHistoryEntry::user_prompt(
        "session-1",
        "attachment-1",
        "agent-1",
        "external prompt without observed source",
    )
    .with_prompt_origin(PromptOrigin::External);
    let external_assistant = SessionHistoryEntry::provider_output(
        "session-1",
        "run-1",
        Some("agent-1"),
        TerminalOutputKind::ProviderOutput,
        Some("assistant-1".to_string()),
        "partial output",
    )
    .with_prompt_origin(PromptOrigin::External);
    let prompt = HistoryEvent::transcript(10, &external_prompt, context.clone());
    let mut prompt = prompt;
    prompt.timestamp_ms = observed_at_ms;
    let mut assistant = HistoryEvent::transcript(11, &external_assistant, context);
    assistant.timestamp_ms = observed_at_ms;

    let turn = outline_turn_from_events(&prompt, vec![prompt.clone(), assistant], false)
        .expect("external-origin turn should be outlined");

    assert_eq!(turn.prompt_origin, PromptOrigin::External);
    assert_eq!(turn.lifecycle, SessionHistoryOutlineTurnLifecycle::Open);
    assert_eq!(turn.completed_at_ms, None);
    assert_eq!(turn.external_provider, None);
    assert_eq!(turn.external_provider_session_id, None);
    assert_eq!(turn.external_provider_turn_id, None);
    assert_eq!(
        turn.summary.as_ref().map(|entry| entry.entry.text.as_str()),
        Some("partial output")
    );
}

#[test]
fn outline_external_turn_without_settlement_completes_when_newer_prompt_exists() {
    let context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        prompt_id: Some("prompt-1".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    let external_prompt = SessionHistoryEntry::external_provider_observed(
        "session-1",
        Some("run-1"),
        "agent-1",
        SessionHistoryEntryKind::UserPrompt,
        "external prompt",
        "codex",
        "thread-1",
        Some("turn-1".to_string()),
        Some(2_000),
    );
    let external_assistant = SessionHistoryEntry::external_provider_observed(
        "session-1",
        Some("run-1"),
        "agent-1",
        SessionHistoryEntryKind::ProviderOutput,
        "final output",
        "codex",
        "thread-1",
        Some("turn-1".to_string()),
        Some(2_100),
    );
    let prompt = HistoryEvent::transcript(10, &external_prompt, context.clone());
    let assistant = HistoryEvent::transcript(11, &external_assistant, context);

    let turn = outline_turn_from_events(&prompt, vec![prompt.clone(), assistant], true)
        .expect("external bounded turn should be outlined");

    assert_eq!(turn.prompt_origin, PromptOrigin::External);
    assert_eq!(
        turn.lifecycle,
        SessionHistoryOutlineTurnLifecycle::Completed
    );
    assert_eq!(turn.completed_at_ms, Some(2_100));
}

#[test]
fn agent_outline_completes_bounded_external_turns_without_client_repair() {
    let observed_now_ms = crate::session::unix_epoch_ms();
    let path = std::env::temp_dir().join(format!(
        "chariox-external-bounded-outline-{}-{}.db",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history store should open");
    for index in 1..=2 {
        let context = HistoryEventTurnContext {
            session_id: Some("session-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            turn_id: Some(format!("turn-{index}")),
            prompt_id: Some(format!("prompt-{index}")),
            provider_run_id: Some(format!("run-{index}")),
            ..HistoryEventTurnContext::default()
        };
        let prompt = SessionHistoryEntry::external_provider_observed(
            "session-1",
            Some(&format!("run-{index}")),
            "agent-1",
            SessionHistoryEntryKind::UserPrompt,
            &format!("external prompt {index}"),
            "codex",
            "thread-1",
            Some(format!("turn-{index}")),
            Some(if index == 2 {
                observed_now_ms
            } else {
                index * 1_000
            }),
        );
        let assistant = SessionHistoryEntry::external_provider_observed(
            "session-1",
            Some(&format!("run-{index}")),
            "agent-1",
            SessionHistoryEntryKind::ProviderOutput,
            &format!("external output {index}"),
            "codex",
            "thread-1",
            Some(format!("turn-{index}")),
            Some(if index == 2 {
                observed_now_ms
            } else {
                index * 1_000 + 100
            }),
        );
        store
            .append(&HistoryEvent::transcript(
                index * 10,
                &prompt,
                context.clone(),
            ))
            .expect("external prompt should append");
        store
            .append(&HistoryEvent::transcript(
                index * 10 + 1,
                &assistant,
                context,
            ))
            .expect("external assistant output should append");
    }

    let outline =
        load_agent_outline(&store, "session-1", "agent-1", 2, None).expect("outline should load");

    assert_eq!(outline.turns.len(), 2);
    assert_eq!(outline.turns[0].prompt_origin, PromptOrigin::External);
    assert_eq!(outline.turns[0].completed_at_ms, Some(1_100));
    assert_eq!(outline.turns[1].prompt_origin, PromptOrigin::External);
    assert_eq!(outline.turns[1].completed_at_ms, None);

    let older = load_agent_outline(&store, "session-1", "agent-1", 1, Some(20))
        .expect("older outline page should load");
    assert_eq!(older.turns.len(), 1);
    assert_eq!(older.turns[0].completed_at_ms, Some(1_100));

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn agent_outline_suppresses_chariox_owned_external_prompt_echoes() {
    let path = std::env::temp_dir().join(format!(
        "chariox-owned-external-echo-outline-{}-{}.db",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history store should open");
    let context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("chariox-turn".to_string()),
        prompt_id: Some("prompt-1".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    store
        .append(&HistoryEvent::transcript(
            10,
            &SessionHistoryEntry::user_prompt(
                "session-1",
                "attachment-1",
                "agent-1",
                "  same   prompt\nfrom Chariox ",
            ),
            context.clone(),
        ))
        .expect("Chariox prompt should append");
    let external_prompt = SessionHistoryEntry::external_provider_observed(
        "session-1",
        Some("run-1"),
        "agent-1",
        SessionHistoryEntryKind::UserPrompt,
        "same prompt from Chariox",
        "codex",
        "thread-1",
        Some("user-echo".to_string()),
        Some(2_000),
    );
    let external_prompt_with_attachment_markup = SessionHistoryEntry::external_provider_observed(
        "session-1",
        Some("run-1"),
        "agent-1",
        SessionHistoryEntryKind::UserPrompt,
        "same prompt from Chariox <image name=[Image #1] path=\"/tmp/screenshot.png\"> </image>",
        "codex",
        "thread-1",
        Some("user-echo-2".to_string()),
        Some(2_001),
    );
    let external_prompt_with_provider_attachment_suffix =
        SessionHistoryEntry::external_provider_observed(
            "session-1",
            Some("run-1"),
            "agent-1",
            SessionHistoryEntryKind::UserPrompt,
            "same prompt from CharioxAttachment: note.txt (text/plain) at data:text/plain;base64,SGVsbG8=",
            "codex",
            "thread-1",
            Some("user-echo-3".to_string()),
            Some(2_002),
        );
    let external_tool_call_echo = SessionHistoryEntry::external_provider_observed(
        "session-1",
        Some("run-1"),
        "agent-1",
        SessionHistoryEntryKind::UserPrompt,
        "Called the Read tool with the following input: {\"filePath\":\"/tmp/screenshot.png\"}",
        "opencode",
        "opencode-session-1",
        Some("tool-call-echo".to_string()),
        Some(2_003),
    );
    let external_assistant = SessionHistoryEntry::external_provider_observed(
        "session-1",
        Some("run-1"),
        "agent-1",
        SessionHistoryEntryKind::ProviderOutput,
        "duplicated provider reply",
        "codex",
        "thread-1",
        Some("assistant-echo".to_string()),
        Some(2_100),
    );
    store
        .append(&HistoryEvent::transcript(
            11,
            &external_prompt,
            context.clone(),
        ))
        .expect("external prompt should append");
    store
        .append(&HistoryEvent::transcript(
            12,
            &external_prompt_with_attachment_markup,
            context.clone(),
        ))
        .expect("external prompt should append");
    store
        .append(&HistoryEvent::transcript(
            13,
            &external_prompt_with_provider_attachment_suffix,
            context.clone(),
        ))
        .expect("external provider attachment prompt should append");
    store
        .append(&HistoryEvent::transcript(
            14,
            &external_tool_call_echo,
            context.clone(),
        ))
        .expect("external tool call echo should append");
    store
        .append(&HistoryEvent::transcript(15, &external_assistant, context))
        .expect("external assistant should append");

    let import =
        ExternalProviderImportMetadata::observed_history("codex:thread-1", "codex", "thread-1");
    let outline = load_scoped_agent_outline(&store, "session-1", "agent-1", 4, None, Some(&import))
        .expect("outline should load");

    assert_eq!(outline.turns.len(), 1);
    assert_eq!(outline.turns[0].prompt_origin, PromptOrigin::Chariox);
    assert_eq!(
        outline.turns[0].user_prompt.entry.text,
        "  same   prompt\nfrom Chariox "
    );
    assert!(outline.turns[0].external_provider.is_none());
    assert!(outline.turns[0].entries.is_empty());

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn agent_outline_suppresses_reformatted_workflow_prompt_echoes_by_delivery_token() {
    let path = std::env::temp_dir().join(format!(
        "chariox-workflow-external-echo-outline-{}-{}.db",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history store should open");
    let context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("prompt-77".to_string()),
        prompt_id: Some("prompt-77".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    let owned_prompt = r#"<workflow-handoff-payloads>
[
  { "workflow_run_id": "run-1", "output": { "message": "20" } }
]
</workflow-handoff-payloads>
Call ack_workflow_turn with {"delivery_token":"workflow-ack:workflow-node-run-2"}."#;
    store
        .append(&HistoryEvent::transcript(
            10,
            &SessionHistoryEntry::user_prompt(
                "session-1",
                "workflow-run:run-1",
                "agent-1",
                owned_prompt,
            ),
            context.clone(),
        ))
        .expect("Chariox workflow prompt should append");
    let observed_prompt = SessionHistoryEntry::external_provider_observed(
        "session-1",
        Some("run-1"),
        "agent-1",
        SessionHistoryEntryKind::UserPrompt,
        r#"<workflow-handoff-payloads> [{"workflow_run_id":"run-1","output":{"message":"20"}}] </workflow-handoff-payloads> Call ack_workflow_turn with `workflow-ack:workflow-node-run-2`. <runtime-instructions>extra provider context</runtime-instructions>"#,
        "codex",
        "thread-1",
        Some("observed-user".to_string()),
        Some(2_000),
    );
    store
        .append(&HistoryEvent::transcript(
            11,
            &observed_prompt,
            context.clone(),
        ))
        .expect("observed workflow prompt should append");
    let assistant = SessionHistoryEntry::provider_output(
        "session-1",
        "run-1",
        Some("agent-1"),
        TerminalOutputKind::ProviderOutput,
        None,
        "The number is 11.",
    )
    .with_source_attachment_id(Some("workflow-run:run-1".to_string()));
    store
        .append(&HistoryEvent::transcript(12, &assistant, context))
        .expect("assistant output should append");

    let import =
        ExternalProviderImportMetadata::observed_history("codex:thread-1", "codex", "thread-1");
    let outline = load_scoped_agent_outline(&store, "session-1", "agent-1", 4, None, Some(&import))
        .expect("outline should load");

    assert_eq!(outline.turns.len(), 1);
    assert_eq!(outline.turns[0].turn_id, "prompt-77");
    assert_eq!(outline.turns[0].prompt_origin, PromptOrigin::Chariox);
    assert_eq!(outline.turns[0].user_prompt.entry.text, owned_prompt);
    assert_eq!(
        outline.turns[0]
            .summary
            .as_ref()
            .map(|entry| entry.entry.text.as_str()),
        Some("The number is 11.")
    );

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn agent_outline_suppresses_endpoint_only_workflow_prompt_echoes() {
    let path = std::env::temp_dir().join(format!(
        "chariox-workflow-endpoint-echo-outline-{}-{}.db",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history store should open");
    let context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("prompt-77".to_string()),
        prompt_id: Some("prompt-77".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    let owned_prompt = r#"<endpoint-prompt>
Inspect the lifecycle and report one concise result.
</endpoint-prompt>

<workflow-runtime-instructions>
Call ack_workflow_turn with {"delivery_token":"workflow-ack:node-run-1"}.
</workflow-runtime-instructions>"#;
    store
        .append(&HistoryEvent::transcript(
            10,
            &SessionHistoryEntry::user_prompt(
                "session-1",
                "workflow-run:run-1",
                "agent-1",
                owned_prompt,
            ),
            context.clone(),
        ))
        .expect("Chariox workflow prompt should append");
    let observed_prompt = SessionHistoryEntry::external_provider_observed(
        "session-1",
        Some("run-1"),
        "agent-1",
        SessionHistoryEntryKind::UserPrompt,
        "<endpoint-prompt> Inspect the lifecycle and report one concise result. </endpoint-prompt>",
        "codex",
        "thread-1",
        Some("observed-user".to_string()),
        Some(2_000),
    );
    store
        .append(&HistoryEvent::transcript(
            11,
            &observed_prompt,
            context.clone(),
        ))
        .expect("observed workflow prompt should append");
    let assistant = SessionHistoryEntry::provider_output(
        "session-1",
        "run-1",
        Some("agent-1"),
        TerminalOutputKind::ProviderOutput,
        None,
        "Lifecycle is stable.",
    )
    .with_source_attachment_id(Some("workflow-run:run-1".to_string()));
    store
        .append(&HistoryEvent::transcript(12, &assistant, context))
        .expect("assistant output should append");

    let import =
        ExternalProviderImportMetadata::observed_history("codex:thread-1", "codex", "thread-1");
    let outline = load_scoped_agent_outline(&store, "session-1", "agent-1", 4, None, Some(&import))
        .expect("outline should load");

    assert_eq!(outline.turns.len(), 1);
    assert_eq!(outline.turns[0].turn_id, "prompt-77");
    assert_eq!(outline.turns[0].prompt_origin, PromptOrigin::Chariox);
    assert_eq!(
        outline.turns[0].user_prompt.entry.text,
        "Inspect the lifecycle and report one concise result."
    );
    assert_eq!(outline.turns[0].user_prompt.fragment_start, 0);
    assert_eq!(
        outline.turns[0].user_prompt.fragment_end,
        "Inspect the lifecycle and report one concise result."
            .chars()
            .count()
    );
    assert_eq!(
        outline.turns[0].user_prompt.total_chars,
        outline.turns[0].user_prompt.fragment_end
    );

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn agent_outline_suppresses_legacy_workflow_echo_by_structured_handoff_payload() {
    let path = std::env::temp_dir().join(format!(
        "chariox-workflow-handoff-external-echo-outline-{}-{}.db",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history store should open");
    let owned_context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("prompt-77".to_string()),
        prompt_id: Some("prompt-77".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    let owned_prompt = r#"<workflow-handoff-payloads>
[
  {
    "source_node_id": "node-1",
    "workflow_run_id": "run-1",
    "output": { "message": "18", "kind": "final" }
  }
]
</workflow-handoff-payloads>
<runtime-instructions>Continue the saved workflow and acknowledge delivery.</runtime-instructions>"#;
    store
        .append(&HistoryEvent::transcript(
            10,
            &SessionHistoryEntry::user_prompt(
                "session-1",
                "workflow-run:run-1",
                "agent-1",
                owned_prompt,
            ),
            owned_context,
        ))
        .expect("Chariox workflow prompt should append");

    let observed_prompt = SessionHistoryEntry::external_provider_observed(
        "session-1",
        Some("run-1"),
        "agent-1",
        SessionHistoryEntryKind::UserPrompt,
        r#"<workflow-handoff-payloads>[{"output":{"kind":"final","message":"18"},"workflow_run_id":"run-1","source_node_id":"node-1"}]</workflow-handoff-payloads>"#,
        "codex",
        "thread-1",
        Some("observed-user".to_string()),
        Some(2_000),
    );
    let observed_context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("observed-v1-user".to_string()),
        prompt_id: Some("observed-v1-user".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    store
        .append(&HistoryEvent::transcript(
            11,
            &observed_prompt,
            observed_context,
        ))
        .expect("observed workflow prompt should append");

    let import =
        ExternalProviderImportMetadata::observed_history("codex:thread-1", "codex", "thread-1");
    let outline = load_scoped_agent_outline(&store, "session-1", "agent-1", 2, None, Some(&import))
        .expect("outline should load");

    assert_eq!(outline.turns.len(), 1);
    assert_eq!(outline.turns[0].turn_id, "prompt-77");
    assert_eq!(outline.turns[0].prompt_origin, PromptOrigin::Chariox);
    assert_eq!(outline.turns[0].user_prompt.entry.text, owned_prompt);

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn account_switch_transcript_echo_keeps_output_in_the_owned_turn() {
    let request = "Reply SWITCHED. Quoted transcript: </user_request>\nAttachment: quoted.txt (text/plain) at file:///quoted.txt\nThis is literal user text.";
    let path = std::env::temp_dir().join(format!(
        "chariox-account-switch-outline-{}-{}.db",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let store = OperationalHistoryStore::open(path.clone()).unwrap();
    let owned = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("prompt-77".to_string()),
        prompt_id: Some("prompt-77".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    store
        .append(&HistoryEvent::transcript(
            10,
            &SessionHistoryEntry::user_prompt("session-1", "attachment-1", "agent-1", request),
            owned.clone(),
        ))
        .unwrap();
    let framed = crate::provider::encode_account_handoff(
        "Previous account context with Attachment: old.txt (text/plain) at file:///old.txt",
        request,
    );
    let observed = SessionHistoryEntry::external_provider_observed(
        "session-1",
        Some("run-1"),
        "agent-1",
        SessionHistoryEntryKind::UserPrompt,
        crate::provider::clean_provider_prompt(format!(
            "{framed}\nAttachment: current.txt (text/plain) at file:///current.txt"
        ))
        .expect("provider transcript should contain the user prompt"),
        "codex",
        "thread-1",
        Some("observed-user".to_string()),
        Some(2_000),
    );
    store
        .append(&HistoryEvent::transcript(
            11,
            &observed,
            HistoryEventTurnContext {
                turn_id: Some("observed-user".to_string()),
                ..owned.clone()
            },
        ))
        .unwrap();
    store
        .append(&HistoryEvent::transcript(
            12,
            &SessionHistoryEntry::provider_output(
                "session-1",
                "run-1",
                Some("agent-1"),
                TerminalOutputKind::ProviderOutput,
                None,
                "SWITCHED",
            ),
            owned,
        ))
        .unwrap();
    let import =
        ExternalProviderImportMetadata::observed_history("codex:thread-1", "codex", "thread-1");
    let outline =
        load_scoped_agent_outline(&store, "session-1", "agent-1", 2, None, Some(&import)).unwrap();
    assert_eq!(
        outline.turns.len(),
        1,
        "A provider echo must not split the kernel-owned turn"
    );
    assert_eq!(outline.turns[0].turn_id, "prompt-77");
    assert_eq!(
        outline.turns[0].summary.as_ref().unwrap().entry.text,
        "SWITCHED"
    );
    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn scoped_agent_outline_excludes_external_turns_outside_import() {
    let path = std::env::temp_dir().join(format!(
        "chariox-scoped-external-outline-{}-{}.db",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history store should open");
    let import =
        ExternalProviderImportMetadata::observed_history("codex:thread-1", "codex", "thread-1");
    for (sequence, provider, provider_session_id, prompt_text, output_text) in [
        (
            10,
            "codex",
            "thread-1",
            "visible old prompt",
            "visible old output",
        ),
        (
            20,
            "claude",
            "claude-session-1",
            "wrong provider prompt",
            "wrong provider output",
        ),
        (
            30,
            "codex",
            "thread-2",
            "wrong codex prompt",
            "wrong codex output",
        ),
        (
            40,
            "codex",
            "thread-1",
            "visible new prompt",
            "visible new output",
        ),
    ] {
        let context = HistoryEventTurnContext {
            session_id: Some("session-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            turn_id: Some(format!("{provider_session_id}:{sequence}")),
            prompt_id: Some(format!("prompt-{sequence}")),
            provider_run_id: Some(format!("run-{sequence}")),
            ..HistoryEventTurnContext::default()
        };
        let prompt = SessionHistoryEntry::external_provider_observed(
            "session-1",
            Some(&format!("run-{sequence}")),
            "agent-1",
            SessionHistoryEntryKind::UserPrompt,
            prompt_text,
            provider,
            provider_session_id,
            Some(format!("turn-{sequence}")),
            Some(sequence * 100),
        );
        let output = SessionHistoryEntry::external_provider_observed(
            "session-1",
            Some(&format!("run-{sequence}")),
            "agent-1",
            SessionHistoryEntryKind::ProviderOutput,
            output_text,
            provider,
            provider_session_id,
            Some(format!("turn-{sequence}")),
            Some(sequence * 100 + 1),
        );
        store
            .append(&HistoryEvent::transcript(
                sequence,
                &prompt,
                context.clone(),
            ))
            .expect("external prompt should append");
        store
            .append(&HistoryEvent::transcript(sequence + 1, &output, context))
            .expect("external output should append");
    }
    let missing_identity_context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("missing-identity".to_string()),
        prompt_id: Some("prompt-missing-identity".to_string()),
        provider_run_id: Some("run-missing-identity".to_string()),
        ..HistoryEventTurnContext::default()
    };
    let mut missing_identity_prompt = SessionHistoryEntry::external_provider_observed(
        "session-1",
        Some("run-missing-identity"),
        "agent-1",
        SessionHistoryEntryKind::UserPrompt,
        "missing identity prompt",
        "codex",
        "thread-missing",
        Some("turn-missing".to_string()),
        Some(2_500),
    );
    missing_identity_prompt.external_provider = None;
    missing_identity_prompt.external_provider_session_id = None;
    let mut missing_identity_output = SessionHistoryEntry::external_provider_observed(
        "session-1",
        Some("run-missing-identity"),
        "agent-1",
        SessionHistoryEntryKind::ProviderOutput,
        "missing identity output",
        "codex",
        "thread-missing",
        Some("turn-missing".to_string()),
        Some(2_501),
    );
    missing_identity_output.external_provider = None;
    missing_identity_output.external_provider_session_id = None;
    let mut missing_identity_tool = SessionHistoryEntry::external_provider_observed(
        "session-1",
        Some("run-missing-identity"),
        "agent-1",
        SessionHistoryEntryKind::ProviderTool,
        r#"{"tool":"bash","status":"completed","input":{"command":"echo hidden"}}"#,
        "codex",
        "thread-missing",
        Some("tool-missing".to_string()),
        Some(2_502),
    );
    missing_identity_tool.external_provider = None;
    missing_identity_tool.external_provider_session_id = None;
    for (sequence, entry) in [
        (25, missing_identity_prompt),
        (26, missing_identity_tool),
        (27, missing_identity_output),
    ] {
        store
            .append(&HistoryEvent::transcript(
                sequence,
                &entry,
                missing_identity_context.clone(),
            ))
            .expect("missing-identity external entry should append");
    }

    let outline = load_scoped_agent_outline(&store, "session-1", "agent-1", 2, None, Some(&import))
        .expect("scoped outline should load");

    assert_eq!(outline.turns.len(), 2);
    assert_eq!(outline.next_cursor, None);
    assert_eq!(
        outline.turns[0].user_prompt.entry.text,
        "visible old prompt"
    );
    assert_eq!(
        outline.turns[0]
            .summary
            .as_ref()
            .map(|entry| entry.entry.text.as_str()),
        Some("visible old output")
    );
    assert_eq!(outline.turns[0].entries.len(), 0);
    assert_eq!(
        outline.turns[0].external_provider_session_id.as_deref(),
        Some("thread-1")
    );
    assert_eq!(
        outline.turns[1].user_prompt.entry.text,
        "visible new prompt"
    );
    assert_eq!(
        outline.turns[1]
            .summary
            .as_ref()
            .map(|entry| entry.entry.text.as_str()),
        Some("visible new output")
    );
    assert_eq!(
        outline.turns[1].external_provider_session_id.as_deref(),
        Some("thread-1")
    );

    let wrong_blob_response = tokio::runtime::Runtime::new()
        .expect("runtime should create")
        .block_on(execute_scoped_session_history_blob_content_request(
            store.clone(),
            GetSessionHistoryBlobContentRequest {
                session_id: "session-1".to_string(),
                agent_id: "agent-1".to_string(),
                blob_id: blob_id(21, 21),
            },
            Some(import.clone()),
        ))
        .expect("scoped blob content should load");
    let LocalDaemonResponse::SessionHistoryBlobContent { entries, .. } = wrong_blob_response else {
        panic!("unexpected response")
    };
    assert!(entries.is_empty());

    let missing_identity_blob_response = tokio::runtime::Runtime::new()
        .expect("runtime should create")
        .block_on(execute_scoped_session_history_blob_content_request(
            store.clone(),
            GetSessionHistoryBlobContentRequest {
                session_id: "session-1".to_string(),
                agent_id: "agent-1".to_string(),
                blob_id: blob_id(26, 26),
            },
            Some(import),
        ))
        .expect("scoped blob content should load");
    let LocalDaemonResponse::SessionHistoryBlobContent { entries, .. } =
        missing_identity_blob_response
    else {
        panic!("unexpected response")
    };
    assert!(entries.is_empty());

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn outline_external_turn_uses_settlement_observed_time_as_completion() {
    let context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        prompt_id: Some("prompt-1".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    let external_prompt = SessionHistoryEntry::external_provider_observed(
        "session-1",
        Some("run-1"),
        "agent-1",
        SessionHistoryEntryKind::UserPrompt,
        "external prompt",
        "codex",
        "thread-1",
        Some("turn-1".to_string()),
        Some(2_000),
    );
    let mut external_status = SessionHistoryEntry::external_provider_observed(
        "session-1",
        Some("run-1"),
        "agent-1",
        SessionHistoryEntryKind::ProviderStatus,
        "codex task_complete",
        "codex",
        "thread-1",
        Some("turn-1".to_string()),
        Some(2_200),
    );
    external_status.external_observation =
        Some(crate::history::SessionHistoryExternalObservation {
            settles_active_prompt: true,
            passive_telemetry: false,
        });
    let prompt = HistoryEvent::transcript(10, &external_prompt, context.clone());
    let status = HistoryEvent::transcript(11, &external_status, context);

    let turn = outline_turn_from_events(&prompt, vec![prompt.clone(), status], false)
        .expect("external completed turn should be outlined");

    assert_eq!(turn.prompt_origin, PromptOrigin::External);
    assert_eq!(turn.completed_at_ms, Some(2_200));
}

#[test]
fn outline_external_turn_uses_hidden_state_settlement_without_rendering_it() {
    let context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        prompt_id: Some("prompt-1".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    let external_prompt = SessionHistoryEntry::external_provider_observed(
        "session-1",
        Some("run-1"),
        "agent-1",
        SessionHistoryEntryKind::UserPrompt,
        "external prompt",
        "claude",
        "thread-1",
        Some("turn-1".to_string()),
        Some(2_000),
    );
    let external_assistant = SessionHistoryEntry::external_provider_observed(
        "session-1",
        Some("run-1"),
        "agent-1",
        SessionHistoryEntryKind::ProviderOutput,
        "final output",
        "claude",
        "thread-1",
        Some("turn-1".to_string()),
        Some(2_100),
    );
    let hidden_settlement = SessionHistoryEntry::external_provider_observed_state_signal(
        "session-1",
        Some("run-1"),
        "agent-1",
        "claude",
        "thread-1",
        crate::history::EXTERNAL_PROVIDER_ACTIVE_PROMPT_SETTLED_REASON,
        "external:claude:thread-1:assistant-1",
        "turn-1".to_string(),
        Some(2_200),
    );
    let prompt = HistoryEvent::transcript(10, &external_prompt, context.clone());
    let assistant = HistoryEvent::transcript(11, &external_assistant, context.clone());
    let settlement = HistoryEvent::transcript(12, &hidden_settlement, context);

    let turn =
        outline_turn_from_events(&prompt, vec![prompt.clone(), assistant, settlement], false)
            .expect("external completed turn should be outlined");

    assert_eq!(turn.prompt_origin, PromptOrigin::External);
    assert_eq!(turn.completed_at_ms, Some(2_200));
    assert!(
        turn.entries.is_empty(),
        "hidden state rows should not render"
    );
    assert!(turn.blobs.is_empty(), "hidden state rows should not render");
    assert_eq!(
        turn.summary.as_ref().map(|entry| entry.entry.text.as_str()),
        Some("final output")
    );
}

#[test]
fn blob_content_hides_external_observer_state_signals() {
    let path = std::env::temp_dir().join(format!(
        "chariox-hidden-state-blob-content-{}-{}.db",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history store should open");
    let context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        prompt_id: Some("prompt-1".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    let hidden_settlement = SessionHistoryEntry::external_provider_observed_state_signal(
        "session-1",
        Some("run-1"),
        "agent-1",
        "codex",
        "thread-1",
        crate::history::EXTERNAL_PROVIDER_ACTIVE_PROMPT_SETTLED_REASON,
        "external:codex:thread-1:assistant-1",
        "turn-1".to_string(),
        Some(2_200),
    );
    store
        .append(&HistoryEvent::transcript(12, &hidden_settlement, context))
        .expect("hidden settlement should append");

    let response = tokio::runtime::Runtime::new()
        .expect("runtime should create")
        .block_on(execute_session_history_blob_content_request(
            store.clone(),
            GetSessionHistoryBlobContentRequest {
                session_id: "session-1".to_string(),
                agent_id: "agent-1".to_string(),
                blob_id: blob_id(12, 12),
            },
        ))
        .expect("blob content should load");
    let LocalDaemonResponse::SessionHistoryBlobContent { entries, .. } = response else {
        panic!("unexpected response")
    };

    assert!(
        entries.is_empty(),
        "internal observer state must not project through blob content"
    );

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}
