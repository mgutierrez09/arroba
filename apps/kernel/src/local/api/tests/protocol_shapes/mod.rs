use super::*;
use crate::local::{
    GetTerminalCommandCatalogRequest, TerminalCommandCatalog,
    TerminalCommandCatalogExecutionTarget, TerminalCommandCatalogNode,
    TerminalCommandCatalogNodeKind, TerminalCommandCatalogSurface,
};

mod core;
mod credential_enrollment;
mod event_publication;
mod managed_context;
mod managed_environment;
mod native_spawn_slice;
mod prompt_settings;
mod provider_account_credential;
mod provider_usage_activity;
mod publication;
mod recall_terminal_metaagent;
mod room_controller;
mod room_environment;
mod room_environment_placement;
mod slice_display;
mod slice_logs;
mod workflow_code;
mod workspace_history_external;

fn history_page_entry(
    sequence: usize,
    kind: crate::history::SessionHistoryEntryKind,
    agent_id: &str,
    text: &str,
) -> crate::session_history_page::SessionHistoryPageEntry {
    crate::session_history_page::SessionHistoryPageEntry {
        entry_index: sequence,
        fragment_start: 0,
        fragment_end: text.chars().count(),
        total_chars: text.chars().count(),
        entry: crate::history::SessionHistoryEntry {
            session_id: "session-1".to_string(),
            provider_run_id: Some("provider-run-1".to_string()),
            agent_id: Some(agent_id.to_string()),
            source_attachment_id: Some("attachment-1".to_string()),
            prompt_origin: match kind {
                crate::history::SessionHistoryEntryKind::UserPrompt => {
                    Some(crate::session::PromptOrigin::Chariox)
                }
                _ => None,
            },
            kind,
            merge_key: None,
            source: None,
            external_provider: None,
            external_provider_session_id: None,
            external_provider_turn_id: None,
            observed_at_ms: None,
            external_observation: None,
            attachments: Vec::new(),
            text: text.to_string(),
            timestamp_ms: 42 + sequence as u64,
        },
    }
}
