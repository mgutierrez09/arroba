use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::{Child, ChildStdin};
use std::sync::mpsc::Receiver;

use serde_json::Value;

use super::process::{stop_child, ClaudeRuntimeMessage};
use super::watchdog::ClaudeTurnWatchdog;
use super::{AgentExecutionMode, AgentPermissionLevel};
use crate::provider::claude::ClaudeMcpConfigFile;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClaudeRunSelection {
    pub model: Option<String>,
    pub variant: Option<String>,
}

pub(crate) struct ClaudeRuntimeBinding {
    pub state: ClaudeRuntimeState,
    pub selection: ClaudeRunSelection,
}

pub struct ClaudeRuntimeState {
    pub(super) program: String,
    pub(super) args: Vec<String>,
    pub(super) env: BTreeMap<String, String>,
    pub(super) provider_credential_env: crate::provider::ProviderCredentialEnvironment,
    pub(super) env_remove: Vec<String>,
    pub(super) working_directory: Option<PathBuf>,
    pub(super) context_file: Option<PathBuf>,
    pub(super) settings_file: Option<PathBuf>,
    pub(super) usage_file: Option<PathBuf>,
    pub(super) last_usage_file_contents: Option<String>,
    pub(super) mcp_config_file: Option<ClaudeMcpConfigFile>,
    pub(super) child: Child,
    pub(super) stdin: ChildStdin,
    pub(super) receiver: Receiver<ClaudeRuntimeMessage>,
    pub(super) active_model: String,
    pub(super) active_variant: Option<String>,
    pub(super) active_execution_mode: AgentExecutionMode,
    pub(super) active_permission_level: AgentPermissionLevel,
    pub(super) session_id: Option<String>,
    pub(super) active_stream_message_id: Option<String>,
    pub(super) active_turn_id: Option<String>,
    pub(super) active_prompt_message: Option<Value>,
    pub(super) turn_watchdog: ClaudeTurnWatchdog,
    pub(super) cancelled_turn_pending_settlement: bool,
    pub(super) next_turn_number: u64,
    pub(super) result_number: u64,
    pub(super) emitted_text_by_block: BTreeMap<String, String>,
    pub(super) tool_transcript: super::tool_transcript::ClaudeToolTranscript,
    pub(super) completed_text_blocks: BTreeSet<String>,
    pub(super) exit_reported: bool,
}

impl std::fmt::Debug for ClaudeRuntimeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeRuntimeState")
            .field("program", &self.program)
            .field("args", &self.args)
            .field("working_directory", &self.working_directory)
            .field("context_file", &self.context_file)
            .field("settings_file", &self.settings_file)
            .field("usage_file", &self.usage_file)
            .field(
                "mcp_config_file",
                &self.mcp_config_file.as_ref().map(ClaudeMcpConfigFile::path),
            )
            .field("active_model", &self.active_model)
            .field("active_variant", &self.active_variant)
            .field("active_execution_mode", &self.active_execution_mode)
            .field("active_permission_level", &self.active_permission_level)
            .field("session_id", &self.session_id)
            .field("active_stream_message_id", &self.active_stream_message_id)
            .field("active_turn_id", &self.active_turn_id)
            .field("turn_watchdog", &self.turn_watchdog)
            .field(
                "cancelled_turn_pending_settlement",
                &self.cancelled_turn_pending_settlement,
            )
            .field("next_turn_number", &self.next_turn_number)
            .field("result_number", &self.result_number)
            .field(
                "emitted_text_lengths_by_block",
                &self
                    .emitted_text_by_block
                    .iter()
                    .map(|(key, text)| (key, text.len()))
                    .collect::<BTreeMap<_, _>>(),
            )
            .field("completed_text_blocks", &self.completed_text_blocks)
            .field("exit_reported", &self.exit_reported)
            .finish()
    }
}

impl ClaudeRuntimeState {
    pub(crate) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }
}

impl Drop for ClaudeRuntimeState {
    fn drop(&mut self) {
        stop_child(&mut self.child);
    }
}
