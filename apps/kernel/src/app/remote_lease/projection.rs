use crate::app::provider_output;
use crate::error::DaemonError;
use crate::execution_lease::LeasedAgent;
use crate::history::SessionHistoryEntryKind;
use crate::session::{PromptCompletion, PromptQueueItem, PromptStatus, PromptSubmissionOutcome};
use crate::terminal::TerminalOutputKind;
use crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout;
use crate::transport::relay_peer::{
    RelayPeerEvent, RelayPeerRequest, RelayPeerResponse, RelayProjectedCompletion,
    RelayProjectedOutputChunk, RelayProjectedPrompt,
};
use chariox_relay::protocol::ClientTarget;

use super::RemoteLeaseRuntime;

const REMOTE_COMPLETION_HARVEST_RESPONSE_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(60);

#[derive(Debug, Default)]
pub(crate) struct RemoteRuntimeProjectionOutcome {
    pub(crate) accepted: bool,
    pub(crate) completions: Vec<PromptCompletion>,
    pub(crate) provider_failure: Option<RemoteProviderFailure>,
}

#[derive(Debug)]
pub(crate) struct RemoteProviderFailure {
    pub(crate) adapter_key: String,
    pub(crate) message: String,
    pub(crate) profile_transition: crate::runtime::prompt_state::AgentProfileTransitionClaim,
}

impl<'a> RemoteLeaseRuntime<'a> {
    pub(crate) fn drain_leased_runtime_projection(
        &mut self,
        leased_agent_id: &str,
        provider_run_id: &str,
        pump_output: bool,
    ) -> Result<Option<(String, RelayPeerEvent)>, DaemonError> {
        self.drain_leased_runtime_projection_with_recovery(
            leased_agent_id,
            provider_run_id,
            pump_output,
            false,
        )
    }

    pub(crate) fn drain_leased_runtime_projection_with_recovery(
        &mut self,
        leased_agent_id: &str,
        provider_run_id: &str,
        pump_output: bool,
        replay_settled_completion: bool,
    ) -> Result<Option<(String, RelayPeerEvent)>, DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        let lease = self
            .app
            .execution_leases
            .get(&leased_agent.lease_id)
            .cloned()
            .ok_or_else(|| DaemonError::ExecutionLeaseNotFound {
                lease_id: leased_agent.lease_id.clone(),
            })?;
        let home_prompt_id = leased_agent.active_home_prompt_id.clone().or_else(|| {
            replay_settled_completion
                .then_some(())
                .and_then(|()| leased_agent.replayable_completion.as_ref())
                .filter(|completion| completion.provider_run_id == provider_run_id)
                .and_then(|completion| completion.home_prompt_id.clone())
        });
        let home_prompt_started_at_ms = leased_agent.active_home_prompt_started_at_ms;
        let mut pumped_output_records = Vec::new();
        let mut settled_quiet = false;
        if pump_output {
            settled_quiet =
                self.settle_quiet_leased_prompt_if_needed(&leased_agent, provider_run_id)?;
            pumped_output_records = provider_output::pump_terminal_output_for_attachment(
                self.app,
                &leased_agent.backing_session_id,
                &leased_agent.backing_attachment_id,
            )?;
            if !settled_quiet {
                settled_quiet =
                    self.settle_quiet_leased_prompt_if_needed(&leased_agent, provider_run_id)?;
            }
        }
        let mut output_chunks = pumped_output_records
            .into_iter()
            .chain(
                self.app
                    .terminal
                    .drain_output_records(
                        &leased_agent.backing_session_id,
                        &leased_agent.backing_attachment_id,
                    )
                    .into_iter(),
            )
            .filter(|record| {
                record.provider_run_id == provider_run_id
                    && record.kind != TerminalOutputKind::PromptEcho
            })
            .map(|record| RelayProjectedOutputChunk {
                kind: record.kind,
                merge_key: record.merge_key,
                bytes: record.bytes,
            })
            .collect::<Vec<_>>();
        let mut projected_output_history_keys = Vec::new();
        output_chunks.retain(|chunk| {
            if chunk.kind != TerminalOutputKind::ProviderTool {
                return true;
            }
            let snapshot_key =
                leased_provider_run_history_chunk_key(&leased_agent, provider_run_id, chunk);
            if leased_agent
                .projected_output_history_keys
                .iter()
                .any(|key| key == &snapshot_key)
            {
                return false;
            }
            projected_output_history_keys.push(snapshot_key);
            true
        });
        let mut projected_output_stream_keys = output_chunks
            .iter()
            .map(|chunk| leased_provider_run_stream_key(&leased_agent, provider_run_id, chunk))
            .collect::<std::collections::BTreeSet<_>>();
        projected_output_history_keys.extend(projected_output_stream_keys.iter().cloned());
        let mut history_chunks =
            self.leased_provider_run_output_history_chunks(&leased_agent, provider_run_id)?;
        history_chunks.retain(|history_chunk| {
            let history_key = leased_provider_run_history_chunk_key(
                &leased_agent,
                provider_run_id,
                history_chunk,
            );
            let stream_key =
                leased_provider_run_stream_key(&leased_agent, provider_run_id, history_chunk);
            let already_projected = leased_agent
                .projected_output_history_keys
                .iter()
                .any(|key| key == &history_key || key == &stream_key)
                || projected_output_stream_keys.contains(&stream_key);
            if already_projected {
                projected_output_history_keys.push(history_key);
                return false;
            }
            projected_output_stream_keys.insert(stream_key.clone());
            projected_output_history_keys.push(stream_key);
            true
        });
        let latest_output_history_completion_key = history_chunks
            .iter()
            .rev()
            .find(|chunk| chunk.kind == TerminalOutputKind::ProviderOutput)
            .map(|chunk| {
                leased_provider_run_history_chunk_key(&leased_agent, provider_run_id, chunk)
            });
        for history_chunk in &history_chunks {
            projected_output_history_keys.push(leased_provider_run_history_chunk_key(
                &leased_agent,
                provider_run_id,
                history_chunk,
            ));
        }
        if !history_chunks.is_empty() {
            output_chunks.extend(history_chunks);
        }
        let notices = self
            .app
            .terminal
            .drain_notice_records(
                &leased_agent.backing_session_id,
                &leased_agent.backing_attachment_id,
            )
            .into_iter()
            .filter(|record| {
                record
                    .provider_run_id
                    .as_deref()
                    .is_none_or(|record_provider_run_id| record_provider_run_id == provider_run_id)
            })
            .map(|record| record.message)
            .collect::<Vec<_>>();
        let mut completions = self
            .app
            .terminal
            .drain_completion_records(
                &leased_agent.backing_session_id,
                &leased_agent.backing_attachment_id,
            )
            .into_iter()
            .filter(|record| record.provider_run_id == provider_run_id)
            .filter(|record| {
                home_prompt_started_at_ms
                    .is_none_or(|started_at_ms| record.completed_at_ms >= started_at_ms)
            })
            .map(|record| RelayProjectedCompletion {
                message_id: record.message_id,
                completed_at_ms: record.completed_at_ms,
                home_prompt_id: home_prompt_id.clone(),
            })
            .collect::<Vec<_>>();
        completions.retain(|completion| {
            let completion_key = leased_provider_run_completion_key(
                &leased_agent,
                provider_run_id,
                &completion.message_id,
            );
            !leased_agent
                .projected_completion_keys
                .iter()
                .any(|key| key == &completion_key)
        });
        let backing_active_prompt = self.app.prompt_owner_active_prompt_for_agent(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        )?;
        let mut prompts = Vec::new();
        let mut latest_home_origin_prompt_key = None;
        if let Ok(backing_session) = self
            .app
            .sessions
            .get_session(&leased_agent.backing_session_id)
        {
            let history_entries = self.app.load_session_history_entries(
                &backing_session,
                Some(&leased_agent.backing_agent_id),
            )?;
            for entry in history_entries.into_iter().filter(|entry| {
                entry.kind == SessionHistoryEntryKind::UserPrompt
                    && !entry.is_external_provider_observed()
            }) {
                let prompt_history_key = format!(
                    "history:{}:{}:{}",
                    entry
                        .source_attachment_id
                        .as_deref()
                        .unwrap_or(&leased_agent.backing_attachment_id),
                    entry.timestamp_ms,
                    stable_prompt_hash(&entry.text)
                );
                if (home_prompt_id.is_some()
                    && backing_active_prompt
                        .as_ref()
                        .is_some_and(|prompt| prompt.prompt() == entry.text))
                    || entry
                        .source_attachment_id
                        .as_deref()
                        .is_none_or(|source_attachment_id| {
                            source_attachment_id == leased_agent.backing_attachment_id
                        })
                {
                    latest_home_origin_prompt_key = Some(prompt_history_key);
                    continue;
                }
                if !leased_agent
                    .projected_prompt_ids
                    .iter()
                    .any(|id| id == &prompt_history_key)
                {
                    prompts.push(RelayProjectedPrompt {
                        prompt_id: prompt_history_key,
                        text: entry.text,
                    });
                }
            }
        }
        if let Some(prompt) = backing_active_prompt.as_ref() {
            if home_prompt_id.is_none()
                && prompt.source_attachment_id() != leased_agent.backing_attachment_id
                && !leased_agent
                    .projected_prompt_ids
                    .iter()
                    .any(|id| id == prompt.id())
            {
                prompts.push(RelayProjectedPrompt {
                    prompt_id: prompt.id().to_string(),
                    text: prompt.prompt().to_string(),
                });
            }
        }
        let mut backing_prompt_active = backing_active_prompt.is_some();
        let backing_active_prompt_id = backing_active_prompt
            .as_ref()
            .map(|prompt| prompt.id().to_string());
        let current_batch_has_provider_output = output_chunks
            .iter()
            .any(|chunk| chunk.kind == TerminalOutputKind::ProviderOutput);
        let provider_run = self.app.providers.get_run(provider_run_id).ok();
        let provider_run_projection = provider_run
            .as_ref()
            .map(|run| (run.id().to_string(), run.state()));
        let provider_run_changed = provider_run_projection.is_some()
            && leased_agent.projected_provider_run != provider_run_projection;
        let requires_explicit_completion = leased_provider_requires_explicit_completion(
            &leased_agent.provider,
            provider_run.as_ref(),
        );
        // For explicit-completion providers, message/item completion is not turn
        // completion. Codex can finish a commentary message before running a tool,
        // and Claude can likewise finish intermediate native items. Only the
        // provider-owned prompt transition may release the home turn.
        let completion_waits_for_native_prompt_settlement = requires_explicit_completion;
        if completion_waits_for_native_prompt_settlement
            && !backing_prompt_active
            && leased_agent
                .replayable_completion
                .as_ref()
                .is_some_and(|replay| replay.provider_run_id == provider_run_id)
        {
            // Prompt-state settlement also emits a generic prompt completion
            // record. Prefer the provider message identity retained while the
            // explicit turn was active; it is the stable replay identity shared
            // with the home kernel.
            completions.retain(|completion| !completion.message_id.starts_with("prompt-complete:"));
        }
        let has_settleable_output_history =
            current_batch_has_provider_output && !requires_explicit_completion;
        let provider_run_ended = provider_run
            .as_ref()
            .is_some_and(|run| run.state() == crate::provider::ProviderRunState::Ended);
        let provider_run_failed = provider_run
            .as_ref()
            .and_then(|run| run.terminal_diagnostic())
            .is_some_and(|diagnostic| !diagnostic.trim().is_empty());
        let provider_run_has_projected_output = current_batch_has_provider_output
            || latest_output_history_completion_key.is_some()
            || leased_provider_run_has_projected_transcript_output(&leased_agent, provider_run_id);
        let native_prompt_has_settled =
            completion_waits_for_native_prompt_settlement && !backing_prompt_active;
        let mut deferred_explicit_completion = false;
        let completion_waits_for_output = !completions.is_empty()
            && requires_explicit_completion
            && !native_prompt_has_settled
            && !current_batch_has_provider_output
            && !provider_run_has_projected_output
            && !provider_run_failed;
        let completion_waits_for_native_stop = !completions.is_empty()
            && completion_waits_for_native_prompt_settlement
            && backing_prompt_active
            && !provider_run_failed;
        if completion_waits_for_output || completion_waits_for_native_stop {
            if let Some(completion) = completions.last() {
                if let Some(agent) = self.app.leased_agents.get_mut(leased_agent_id) {
                    agent.replayable_completion =
                        Some(crate::execution_lease::LeasedCompletionReplay {
                            provider_run_id: provider_run_id.to_string(),
                            message_id: completion.message_id.clone(),
                            completed_at_ms: completion.completed_at_ms,
                            home_prompt_id: completion.home_prompt_id.clone(),
                        });
                }
            }
            completions.clear();
            deferred_explicit_completion = true;
        }
        let explicit_completion_waiting_for_output = requires_explicit_completion
            && !provider_run_failed
            && !native_prompt_has_settled
            && !current_batch_has_provider_output
            && !provider_run_has_projected_output
            && (deferred_explicit_completion
                || leased_agent
                    .replayable_completion
                    .as_ref()
                    .is_some_and(|replay| replay.provider_run_id == provider_run_id));
        let explicit_completion_waiting_for_prompt_settlement =
            completion_waits_for_native_prompt_settlement
                && backing_prompt_active
                && !provider_run_failed
                && leased_agent
                    .replayable_completion
                    .as_ref()
                    .is_some_and(|replay| replay.provider_run_id == provider_run_id);
        let explicit_completion_waiting = explicit_completion_waiting_for_output
            || explicit_completion_waiting_for_prompt_settlement;
        let explicit_completion_already_projected = leased_agent
            .replayable_completion
            .as_ref()
            .filter(|replay| replay.provider_run_id == provider_run_id)
            .is_some_and(|replay| {
                let completion_key = leased_provider_run_completion_key(
                    &leased_agent,
                    provider_run_id,
                    &replay.message_id,
                );
                leased_agent
                    .projected_completion_keys
                    .iter()
                    .any(|key| key == &completion_key)
            });
        if completions.is_empty()
            && requires_explicit_completion
            && !explicit_completion_waiting_for_prompt_settlement
            && (native_prompt_has_settled
                || current_batch_has_provider_output
                || provider_run_has_projected_output)
        {
            if let Some(replay) = leased_agent
                .replayable_completion
                .as_ref()
                .filter(|replay| replay.provider_run_id == provider_run_id)
            {
                let completion_key = leased_provider_run_completion_key(
                    &leased_agent,
                    provider_run_id,
                    &replay.message_id,
                );
                if !leased_agent
                    .projected_completion_keys
                    .iter()
                    .any(|key| key == &completion_key)
                {
                    completions.push(RelayProjectedCompletion {
                        message_id: replay.message_id.clone(),
                        completed_at_ms: replay.completed_at_ms,
                        home_prompt_id: replay.home_prompt_id.clone(),
                    });
                }
            }
        }
        let should_complete_from_history = completions.is_empty()
            && prompts.is_empty()
            && backing_active_prompt
                .as_ref()
                .is_some_and(|prompt| prompt.workflow_run_id().is_none())
            && has_settleable_output_history;
        if should_complete_from_history {
            let _ = self.app.complete_active_prompt(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
                Some(provider_run_id),
            )?;
            let _generated_prompt_completions = self
                .app
                .terminal
                .drain_completion_records(
                    &leased_agent.backing_session_id,
                    &leased_agent.backing_attachment_id,
                )
                .into_iter()
                .filter(|record| record.provider_run_id == provider_run_id)
                .collect::<Vec<_>>();
            backing_prompt_active = false;
            settled_quiet = true;
        }
        if !prompts.is_empty() {
            if let Some(agent) = self.app.leased_agents.get_mut(leased_agent_id) {
                for prompt in &prompts {
                    if !agent
                        .projected_prompt_ids
                        .iter()
                        .any(|id| id == &prompt.prompt_id)
                    {
                        agent.projected_prompt_ids.push(prompt.prompt_id.clone());
                    }
                }
            }
        }
        if completions.is_empty()
            && !backing_prompt_active
            && !explicit_completion_waiting
            && !((requires_explicit_completion || provider_run_failed)
                && explicit_completion_already_projected)
            && (settled_quiet
                || has_settleable_output_history
                || (requires_explicit_completion && provider_run_has_projected_output)
                || provider_run_ended
                || provider_run_failed)
        {
            let message_id = leased_synthetic_completion_message_id(
                &leased_agent,
                provider_run_id,
                backing_active_prompt_id.as_deref(),
                latest_home_origin_prompt_key.as_deref(),
                latest_output_history_completion_key.as_deref(),
                &output_chunks,
            );
            let completion_key =
                leased_provider_run_completion_key(&leased_agent, provider_run_id, &message_id);
            if !leased_agent
                .projected_completion_keys
                .iter()
                .any(|key| key == &completion_key)
            {
                completions.push(RelayProjectedCompletion {
                    message_id,
                    completed_at_ms: crate::session::unix_epoch_ms(),
                    home_prompt_id: home_prompt_id.clone(),
                });
            }
        }
        if !completions.is_empty() {
            if backing_prompt_active {
                let _ = self.app.complete_active_prompt(
                    &leased_agent.backing_session_id,
                    &leased_agent.backing_agent_id,
                    Some(provider_run_id),
                )?;
            }
            let completion_keys = completions
                .iter()
                .map(|completion| {
                    leased_provider_run_completion_key(
                        &leased_agent,
                        provider_run_id,
                        &completion.message_id,
                    )
                })
                .collect::<Vec<_>>();
            if let Some(agent) = self.app.leased_agents.get_mut(leased_agent_id) {
                for completion_key in completion_keys {
                    if !agent
                        .projected_completion_keys
                        .iter()
                        .any(|key| key == &completion_key)
                    {
                        agent.projected_completion_keys.push(completion_key);
                    }
                }
                if let Some(completion) = completions.last() {
                    agent.replayable_completion =
                        Some(crate::execution_lease::LeasedCompletionReplay {
                            provider_run_id: provider_run_id.to_string(),
                            message_id: completion.message_id.clone(),
                            completed_at_ms: completion.completed_at_ms,
                            home_prompt_id: completion.home_prompt_id.clone(),
                        });
                }
                if agent.active_home_prompt_id.as_deref() == home_prompt_id.as_deref() {
                    agent.active_home_prompt_id = None;
                    agent.active_home_prompt_started_at_ms = None;
                    agent.applied_home_steer_ids.clear();
                }
            }
            // A single provider run can own an active home turn and queued
            // leased turns. Only the binding whose home prompt completed is
            // retired; removing every binding with this provider id loses the
            // queued contexts before queue promotion can reactivate them.
            self.app.leased_workflow_turns.retain(|_, binding| {
                binding.provider_run_id != provider_run_id
                    || home_prompt_id
                        .as_deref()
                        .is_some_and(|completed| binding.home_prompt_id != completed)
            });
        }
        if replay_settled_completion
            && completions.is_empty()
            && !backing_prompt_active
            && !explicit_completion_waiting
        {
            if let Some(replay) = leased_agent
                .replayable_completion
                .as_ref()
                .filter(|replay| replay.provider_run_id == provider_run_id)
            {
                completions.push(RelayProjectedCompletion {
                    message_id: replay.message_id.clone(),
                    completed_at_ms: replay.completed_at_ms,
                    home_prompt_id: replay.home_prompt_id.clone(),
                });
            }
        }
        if !projected_output_history_keys.is_empty() {
            if let Some(agent) = self.app.leased_agents.get_mut(leased_agent_id) {
                for key in projected_output_history_keys {
                    if !agent
                        .projected_output_history_keys
                        .iter()
                        .any(|id| id == &key)
                    {
                        agent.projected_output_history_keys.push(key);
                    }
                }
            }
        }
        if output_chunks.is_empty()
            && notices.is_empty()
            && completions.is_empty()
            && prompts.is_empty()
            && !provider_run_changed
        {
            return Ok(None);
        }
        if provider_run_changed {
            if let Some(agent) = self.app.leased_agents.get_mut(leased_agent_id) {
                agent.projected_provider_run = provider_run_projection;
            }
        }
        Ok(Some((
            lease.home_kernel_id,
            RelayPeerEvent::LeasedRuntimeProjection {
                home_session_id: lease.home_session_id,
                home_agent_id: lease.home_agent_id,
                provider_run_id: provider_run_id.to_string(),
                provider_run,
                prompts,
                output_chunks,
                notices,
                completions,
            },
        )))
    }

    fn settle_quiet_leased_prompt_if_needed(
        &mut self,
        leased_agent: &LeasedAgent,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        let provider_run = self.app.providers.get_run(provider_run_id).ok();
        if leased_provider_requires_explicit_completion(
            &leased_agent.provider,
            provider_run.as_ref(),
        ) {
            return Ok(false);
        }
        let Some(active_prompt) = self.app.prompt_owner_active_prompt_for_agent(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        )?
        else {
            return Ok(false);
        };
        if active_prompt.workflow_run_id().is_some() {
            return Ok(false);
        }
        if !crate::transport::flow_control::prompt_output_quiet_after_response(
            self.app,
            provider_run_id,
            std::time::Duration::from_millis(50),
        ) {
            return Ok(false);
        }
        let _ = self.app.complete_active_prompt(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
            Some(provider_run_id),
        )?;
        Ok(true)
    }

    fn leased_provider_run_output_history_chunks(
        &mut self,
        leased_agent: &LeasedAgent,
        provider_run_id: &str,
    ) -> Result<Vec<RelayProjectedOutputChunk>, DaemonError> {
        let session = self
            .app
            .sessions
            .get_session(&leased_agent.backing_session_id)?;
        let entries = self
            .app
            .load_session_history_entries(&session, Some(&leased_agent.backing_agent_id))?;
        Ok(entries
            .into_iter()
            .filter(|entry| {
                entry.provider_run_id.as_deref() == Some(provider_run_id)
                    && entry.kind == SessionHistoryEntryKind::ProviderOutput
                    && !entry.is_external_provider_observed()
            })
            .map(|entry| RelayProjectedOutputChunk {
                kind: TerminalOutputKind::ProviderOutput,
                merge_key: entry.merge_key,
                bytes: entry.text.into_bytes(),
            })
            .collect())
    }

    pub(crate) fn pump_leased_runtime_projections(
        &mut self,
    ) -> Result<Vec<(String, RelayPeerEvent)>, DaemonError> {
        let leased_agents = self.app.leased_agents.values().cloned().collect::<Vec<_>>();
        let mut events = Vec::new();
        for leased_agent in leased_agents {
            // Home-origin prompts have their own authoritative drain loop. Let that
            // request/response path consume output and completion records so the
            // best-effort relay event pump cannot race it and mark records projected
            // before the home kernel has actually received them.
            if leased_agent.active_home_prompt_id.is_some() {
                continue;
            }
            let Some(provider_run_id) = self
                .app
                .providers
                .get_run_for_agent(
                    &leased_agent.backing_session_id,
                    &leased_agent.backing_agent_id,
                )
                .or_else(|| {
                    self.app.providers.get_latest_run_for_agent(
                        &leased_agent.backing_session_id,
                        &leased_agent.backing_agent_id,
                    )
                })
                .map(|run| run.id().to_string())
            else {
                continue;
            };
            let _ = provider_output::ProviderOutputPump::new(self.app).pump_provider_output(
                provider_output::ProviderOutputPumpRequest {
                    session_id: &leased_agent.backing_session_id,
                    provider_run_id: &provider_run_id,
                    recipient_attachment_ids: vec![leased_agent.backing_attachment_id.clone()],
                    initial_liveness_already_checked: false,
                },
            )?;
            let _ = self.recover_idle_leased_prompt_queue(&leased_agent.id)?;
            if let Some(event) =
                self.drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)?
            {
                events.push(event);
            }
        }
        Ok(events)
    }

    pub(crate) fn recover_idle_leased_prompt_queue(
        &mut self,
        leased_agent_id: &str,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        if self
            .app
            .prompt_owner_active_prompt_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )?
            .is_some()
        {
            return Ok(None);
        }
        let started = self.app.advance_next_queued_prompt(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        )?;
        if let Some(prompt) = started.as_ref() {
            crate::logging::info_with_fields(
                "daemon.remote_prompt_dispatch",
                "recovered idle leased prompt queue",
                serde_json::json!({
                    "leased_agent_id": leased_agent.id,
                    "session_id": leased_agent.backing_session_id,
                    "agent_id": leased_agent.backing_agent_id,
                    "prompt_id": prompt.id(),
                }),
            );
        }
        Ok(started)
    }

    pub(crate) fn project_remote_runtime_projection(
        &mut self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
        provider_run: Option<crate::provider::RuntimeProviderRun>,
        prompts: Vec<RelayProjectedPrompt>,
        output_chunks: Vec<RelayProjectedOutputChunk>,
        notices: Vec<String>,
        completions: Vec<RelayProjectedCompletion>,
    ) -> Result<RemoteRuntimeProjectionOutcome, DaemonError> {
        let _ = self.app.sessions.get_session(session_id)?;
        let mut outcome = RemoteRuntimeProjectionOutcome::default();
        let agent = self.app.agents.get_agent(agent_id)?;
        if let Some(remote) = agent.remote_execution() {
            let admitted = match remote.active_worker_provider_run_id.as_deref() {
                Some(current) => current == provider_run_id,
                // Native TUI prompts originate on the worker. Managed prompts
                // instead acquire their run binding through dispatch ACK/recovery;
                // old snapshots cannot recreate it after a profile switch.
                None => provider_run
                    .as_ref()
                    .is_some_and(|run| !run.client_interface().is_chariox()),
            };
            if !admitted {
                return Ok(outcome);
            }
        }
        outcome.accepted = true;
        let leased_agent_id = agent
            .remote_execution()
            .map(|remote| remote.leased_agent_id.clone())
            .unwrap_or_else(|| agent_id.to_string());
        let projected_provider_run_id =
            crate::provider::projected_leased_provider_run_id(&leased_agent_id, provider_run_id);
        if let Some(provider_run) = provider_run {
            let projected_run = provider_run.projected_for_home_agent_with_id(
                projected_provider_run_id.clone(),
                session_id.to_string(),
                agent_id.to_string(),
            );
            let projected_run = self
                .app
                .update_remote_provider_run_projection(projected_run);
            let _ = self
                .app
                .sessions
                .set_active_provider_run(session_id, Some(projected_run.id().to_string()));
            if let Ok(agent) = self.app.agents.get_agent(agent_id) {
                if agent.remote_execution().is_some() {
                    let _ = self
                        .app
                        .agents
                        .set_remote_execution_active_worker_provider_run_id(
                            agent_id,
                            Some(provider_run_id.to_string()),
                        );
                    let _ = self.app.agents.set_agent_runtime_profile(
                        agent_id,
                        projected_run.provider(),
                        Some(projected_run.model().to_string()),
                        projected_run.variant().map(str::to_string),
                        projected_run.resume_state().clone(),
                    );
                }
            }
        }
        let recipient_attachment_ids = self.app.attachments.list_session_attachment_ids(session_id);
        for prompt in prompts {
            self.project_remote_native_prompt_started(
                session_id,
                agent_id,
                provider_run_id,
                prompt,
            )?;
        }
        for chunk in output_chunks {
            self.app.fan_out_output_for_agent(
                session_id,
                provider_run_id,
                Some(agent_id),
                chunk.kind.clone(),
                chunk.merge_key.clone(),
                recipient_attachment_ids.clone(),
                &chunk.bytes,
            );
        }
        for notice in notices {
            self.app.record_notice_for_agent(
                session_id,
                Some(provider_run_id),
                Some(agent_id),
                recipient_attachment_ids.clone(),
                notice.clone(),
            );
        }
        let active_prompt = self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, agent_id)?;
        let matching_completions = active_prompt
            .as_ref()
            .map(|active_prompt| {
                completions
                    .into_iter()
                    .filter(|completion| match completion.home_prompt_id.as_deref() {
                        Some(prompt_id) => prompt_id == active_prompt.id(),
                        None => unscoped_completion_matches_prompt(active_prompt),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut completion_indexes = std::collections::BTreeMap::<String, usize>::new();
        let mut deduplicated_completions: Vec<RelayProjectedCompletion> =
            Vec::with_capacity(matching_completions.len());
        for completion in matching_completions {
            if let Some(index) = completion_indexes.get(&completion.message_id).copied() {
                if completion.completed_at_ms > deduplicated_completions[index].completed_at_ms {
                    deduplicated_completions[index] = completion;
                }
                continue;
            }
            completion_indexes.insert(
                completion.message_id.clone(),
                deduplicated_completions.len(),
            );
            deduplicated_completions.push(completion);
        }
        let matching_completions = deduplicated_completions;
        let saw_completion = !matching_completions.is_empty();
        let projected_settled_at_ms = matching_completions
            .iter()
            .map(|completion| completion.completed_at_ms)
            .max();
        for completion in &matching_completions {
            self.app.record_assistant_message_completion_for_agent(
                session_id,
                provider_run_id,
                Some(agent_id),
                recipient_attachment_ids.clone(),
                &completion.message_id,
                completion.completed_at_ms,
            );
        }
        let remote_execution = self
            .app
            .agents
            .get_agent(agent_id)
            .ok()
            .and_then(|agent| agent.remote_execution().cloned());
        if saw_completion {
            if let Some(remote_execution) = remote_execution.as_ref() {
                self.harvest_remote_completion_observations(remote_execution, provider_run_id);
            }
        }
        if let Some(active_prompt) = active_prompt {
            let workflow_output_ready = active_prompt.workflow_run_id().is_some()
                && crate::app::workflow_runtime::workflow_prompt_has_completion_output_from_runtime(
                    self.app,
                    session_id,
                    &active_prompt,
                    Some(provider_run_id),
                );
            if !saw_completion && !workflow_output_ready {
                return Ok(outcome);
            }
            if active_prompt.workflow_run_id().is_some() && !workflow_output_ready {
                if let (Some(workflow_run_id), Some(workflow_node_run_id)) = (
                    active_prompt.workflow_run_id(),
                    active_prompt.workflow_node_run_id(),
                ) {
                    let message =
                        "provider completed workflow turn without a validated workflow output";
                    let failed_provider = self
                        .app
                        .provider_run_projection
                        .get(&projected_provider_run_id);
                    let provider_diagnostic = failed_provider
                        .as_ref()
                        .and_then(|run| run.terminal_diagnostic().map(str::to_string))
                        .filter(|message| !message.trim().is_empty());
                    let failure_details = failed_provider
                        .as_ref()
                        .zip(provider_diagnostic.as_ref())
                        .map(|(run, message)| (run.adapter_key().to_string(), message.clone()));
                    let (failure_kind, failure_message, notice_message) = if let Some(diagnostic) =
                        provider_diagnostic
                    {
                        (
                            crate::session::WorkflowFailureKind::ProviderFailure,
                            diagnostic.clone(),
                            format!(
                                "Workflow run `{workflow_run_id}` failed after provider turn failure: {diagnostic}"
                            ),
                        )
                    } else {
                        (
                            crate::session::WorkflowFailureKind::MissingStructuredOutput,
                            message.to_string(),
                            format!(
                                "Workflow run `{workflow_run_id}` failed after provider turn completion without workflow output."
                            ),
                        )
                    };
                    let failure = crate::session::WorkflowFailureEvent::new(
                        failure_kind,
                        workflow_node_run_id,
                        Vec::new(),
                        failure_message,
                    );
                    let _ = self.app.sessions_mut().record_workflow_failure_event(
                        session_id,
                        workflow_run_id,
                        failure,
                    );
                    self.app.sessions_mut().fail_workflow_node_run(
                        session_id,
                        workflow_run_id,
                        workflow_node_run_id,
                    )?;
                    self.app.record_notice(
                        session_id,
                        Some(provider_run_id),
                        recipient_attachment_ids.clone(),
                        notice_message,
                    );
                    let _ = crate::app::KernelSessionReadService::new(self.app)
                        .session_snapshot(session_id);
                    self.record_projected_prompt_settlement(
                        session_id,
                        agent_id,
                        active_prompt.id(),
                        provider_run_id,
                        projected_settled_at_ms.unwrap_or_else(crate::session::unix_epoch_ms),
                    );
                    let completed = if let Some((adapter_key, message)) = failure_details {
                        let session = self.app.sessions.get_session(session_id)?;
                        let Some((completed, profile_transition)) = self
                            .app
                            .prompt_state_owner()
                            .complete_active_prompt_and_claim_profile_transition(
                            &session,
                            agent_id,
                            active_prompt.id(),
                        )?
                        else {
                            return Ok(outcome);
                        };
                        self.app
                            .mirror_prompt_owner_agent_state(session_id, agent_id)?;
                        outcome.provider_failure = Some(RemoteProviderFailure {
                            adapter_key,
                            message,
                            profile_transition,
                        });
                        completed
                    } else {
                        self.app
                            .prompt_owner_complete_active_prompt_only(session_id, agent_id)?
                    };
                    outcome.completions.push(PromptCompletion {
                        completed,
                        started_next: None,
                    });
                }
                return Ok(outcome);
            }
            self.record_projected_prompt_settlement(
                session_id,
                agent_id,
                active_prompt.id(),
                provider_run_id,
                projected_settled_at_ms.unwrap_or_else(crate::session::unix_epoch_ms),
            );
            let completed = self
                .app
                .prompt_owner_complete_active_prompt_only(session_id, agent_id)?;
            if let Ok(agent) = self.app.agents.get_agent(agent_id) {
                if agent.remote_execution().is_some() {
                    let _ = self
                        .app
                        .agents
                        .set_remote_execution_active_worker_provider_run_id(agent_id, None);
                }
            }
            let _ =
                crate::app::KernelSessionReadService::new(self.app).session_snapshot(session_id);
            crate::app::workflow_runtime::complete_workflow_prompt_from_runtime(
                self.app,
                session_id,
                &completed,
                Some(provider_run_id),
            )?;
            if let Some(remote_execution) = remote_execution {
                if self
                    .app
                    .prompt_owner_active_prompt_for_agent(session_id, agent_id)?
                    .is_none()
                {
                    let started_next = self.app.advance_next_queued_prompt_remote(
                        session_id,
                        agent_id,
                        &remote_execution.worker_kernel_id,
                        &remote_execution.leased_agent_id,
                        remote_execution.relay_url.as_deref(),
                        remote_execution.relay_token.as_deref(),
                    )?;
                    if started_next.is_none() {
                        self.app.sync_focused_provider_run_if_idle(session_id)?;
                    }
                    outcome.completions.push(PromptCompletion {
                        completed,
                        started_next,
                    });
                } else {
                    outcome.completions.push(PromptCompletion {
                        completed,
                        started_next: None,
                    });
                }
            } else {
                outcome.completions.push(PromptCompletion {
                    completed,
                    started_next: None,
                });
            }
        }
        Ok(outcome)
    }

    fn record_projected_prompt_settlement(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        provider_run_id: &str,
        settled_at_ms: u64,
    ) {
        self.app
            .operational_history_store()
            .record_prompt_settlement(
                self.app.history_archive_enabled(),
                session_id,
                agent_id,
                prompt_id,
                Some(provider_run_id),
                settled_at_ms,
                "completed",
            );
    }

    fn project_remote_native_prompt_started(
        &mut self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
        projected: RelayProjectedPrompt,
    ) -> Result<(), DaemonError> {
        if self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, agent_id)?
            .is_some()
        {
            return Ok(());
        }
        let Some(attachment_id) = self
            .app
            .attachments
            .list_session_attachment_ids(session_id)
            .into_iter()
            .next()
        else {
            return Ok(());
        };
        let prompt = PromptQueueItem::new(
            self.app.sessions_mut().reserve_prompt_id(),
            &attachment_id,
            agent_id,
            &projected.text,
            PromptStatus::Queued,
        );
        let outcome =
            self.app
                .prompt_owner_submit_prepared_prompt(session_id, prompt.clone(), false)?;
        self.app.spawn_user_prompt_history_append(
            session_id,
            &attachment_id,
            agent_id,
            prompt.prompt(),
            prompt.attachments(),
        )?;
        if matches!(outcome, PromptSubmissionOutcome::Started { .. }) {
            crate::transport::flow_control::note_prompt_started(self.app, provider_run_id);
        }
        Ok(())
    }

    fn harvest_remote_completion_observations(
        &mut self,
        remote_execution: &crate::agent::RemoteAgentBinding,
        provider_run_id: &str,
    ) {
        let relay_config = self.app.relay_config_for_remote_execution(remote_execution);
        let response = self.app.block_on_relay_future(
            send_peer_request_via_temporary_connection_with_timeout(
                &relay_config,
                ClientTarget {
                    daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                    daemon_alias: None,
                },
                RelayPeerRequest::ObserveLeasedGitAfter {
                    leased_agent_id: remote_execution.leased_agent_id.clone(),
                    provider_run_id: provider_run_id.to_string(),
                },
                REMOTE_COMPLETION_HARVEST_RESPONSE_TIMEOUT,
            ),
        );
        match response {
            Ok(RelayPeerResponse::LeasedGitObserved {
                git_observations,
                workspace_live_sync_change,
                ..
            }) => {
                if let Err(error) = crate::git_observer::append_observations(
                    &self.app.operational_history_store(),
                    git_observations,
                ) {
                    crate::logging::warn_with_fields(
                        "daemon.git_observer",
                        "failed to append projected remote git observations",
                        serde_json::json!({
                            "worker_kernel_id": remote_execution.worker_kernel_id,
                            "leased_agent_id": remote_execution.leased_agent_id,
                            "error": error.to_string(),
                        }),
                    );
                }
                if let Some(change) = workspace_live_sync_change {
                    self.app.fanout_remote_workspace_live_sync_change(
                        change,
                        Some(&remote_execution.worker_kernel_id),
                    );
                }
            }
            Ok(other) => crate::logging::warn_with_fields(
                "daemon.remote_prompt_dispatch",
                "unexpected projected remote completion harvest response",
                serde_json::json!({
                    "worker_kernel_id": remote_execution.worker_kernel_id,
                    "leased_agent_id": remote_execution.leased_agent_id,
                    "response": format!("{other:?}"),
                }),
            ),
            Err(error) => crate::logging::warn_with_fields(
                "daemon.remote_prompt_dispatch",
                "failed to harvest projected remote completion observations",
                serde_json::json!({
                    "worker_kernel_id": remote_execution.worker_kernel_id,
                    "leased_agent_id": remote_execution.leased_agent_id,
                    "error": error.to_string(),
                }),
            ),
        }
    }
}

fn leased_provider_requires_explicit_completion(
    provider: &str,
    worker_provider_run: Option<&crate::provider::RuntimeProviderRun>,
) -> bool {
    if worker_provider_run.is_some_and(crate::provider::provider_run_uses_claude_native_bridge) {
        return true;
    }
    let adapter_key = crate::provider::adapter_key_for_provider(provider);
    crate::provider::ExternalProviderObservationPolicy::for_provider(adapter_key)
        .uses_explicit_completion()
}

fn leased_provider_run_completion_key(
    leased_agent: &LeasedAgent,
    provider_run_id: &str,
    message_id: &str,
) -> String {
    format!(
        "{}:{provider_run_id}:{message_id}",
        leased_agent.backing_session_id
    )
}

fn leased_synthetic_completion_message_id(
    leased_agent: &LeasedAgent,
    provider_run_id: &str,
    backing_active_prompt_id: Option<&str>,
    latest_home_origin_prompt_key: Option<&str>,
    latest_output_history_completion_key: Option<&str>,
    output_chunks: &[RelayProjectedOutputChunk],
) -> String {
    if let Some(prompt_key) = latest_home_origin_prompt_key {
        return format!("leased-{provider_run_id}-completion:{prompt_key}");
    }
    if let Some(output_key) = latest_output_history_completion_key {
        return format!("leased-{provider_run_id}-completion:{output_key}");
    }
    if let Some(prompt_id) = backing_active_prompt_id {
        return format!("leased-{provider_run_id}-completion:{prompt_id}");
    }
    if let Some(chunk) = output_chunks
        .iter()
        .rev()
        .find(|chunk| chunk.kind == TerminalOutputKind::ProviderOutput)
    {
        return format!(
            "leased-{provider_run_id}-completion:{}",
            leased_provider_run_history_chunk_key(leased_agent, provider_run_id, chunk)
        );
    }
    format!("leased-{provider_run_id}-completion:quiet")
}

fn leased_provider_run_history_chunk_key(
    leased_agent: &LeasedAgent,
    provider_run_id: &str,
    chunk: &RelayProjectedOutputChunk,
) -> String {
    format!(
        "{}:{provider_run_id}:{}:{}:{}",
        leased_agent.backing_session_id,
        format!("{:?}", chunk.kind),
        chunk.merge_key.as_deref().unwrap_or(""),
        stable_bytes_hash(&chunk.bytes)
    )
}

fn leased_provider_run_stream_key(
    leased_agent: &LeasedAgent,
    provider_run_id: &str,
    chunk: &RelayProjectedOutputChunk,
) -> String {
    format!(
        "{}:{provider_run_id}:{}:{:?}:{}",
        leased_agent.backing_session_id,
        leased_home_prompt_projection_key(leased_agent),
        chunk.kind,
        chunk.merge_key.as_deref().unwrap_or("")
    )
}

fn leased_provider_run_has_projected_transcript_output(
    leased_agent: &LeasedAgent,
    provider_run_id: &str,
) -> bool {
    let prompt_output_prefix = format!(
        "{}:{provider_run_id}:{}:{:?}:",
        leased_agent.backing_session_id,
        leased_home_prompt_projection_key(leased_agent),
        TerminalOutputKind::ProviderOutput,
    );
    leased_agent
        .projected_output_history_keys
        .iter()
        .any(|key| key.starts_with(&prompt_output_prefix))
}

fn leased_home_prompt_projection_key(leased_agent: &LeasedAgent) -> String {
    leased_agent
        .active_home_prompt_id
        .clone()
        .or_else(|| {
            leased_agent
                .replayable_completion
                .as_ref()
                .and_then(|completion| completion.home_prompt_id.clone())
        })
        .or_else(|| {
            leased_agent
                .active_home_prompt_started_at_ms
                .map(|started_at_ms| format!("started-{started_at_ms}"))
        })
        .unwrap_or_else(|| "no-prompt".to_string())
}

fn stable_bytes_hash(bytes: &[u8]) -> u64 {
    let mut hash = 14_695_981_039_346_656_037_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    hash
}

fn stable_prompt_hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

fn unscoped_completion_matches_prompt(prompt: &PromptQueueItem) -> bool {
    prompt.durable_operation_id().is_none()
        && prompt.durable_recovery_operation_id().is_none()
        && prompt.workflow_run_id().is_none()
}

#[cfg(test)]
mod explicit_completion_tests {
    use super::*;
    use crate::config::DaemonConfig;
    use crate::provider::{
        AgentEndpointMode, LaunchProviderRequest, ProviderClientInterface, ProviderLaunchResult,
        RuntimeProviderRun,
    };
    use crate::session::PromptSubmissionOutcome;

    #[test]
    fn unscoped_completion_only_matches_native_origin_prompts() {
        let native = PromptQueueItem::new(
            "native-prompt",
            "attachment-1",
            "agent-1",
            "native prompt",
            PromptStatus::Running,
        );
        assert!(unscoped_completion_matches_prompt(&native));
        assert!(!unscoped_completion_matches_prompt(
            &native
                .clone()
                .with_durable_operation("operation-1", "fingerprint-1")
        ));
        assert!(!unscoped_completion_matches_prompt(
            &native.clone().with_workflow_context("workflow-1", "node-1")
        ));
        let mut recovering = native;
        recovering.begin_durable_recovery_operation();
        assert!(!unscoped_completion_matches_prompt(&recovering));
    }

    fn provider_run(
        id: &str,
        session_id: &str,
        agent_id: &str,
        adapter_key: &str,
        provider: &str,
        client_interface: ProviderClientInterface,
    ) -> RuntimeProviderRun {
        let request = LaunchProviderRequest::new(
            session_id,
            adapter_key,
            provider,
            "default",
            "claude-sonnet-4-6",
        )
        .with_agent_id(agent_id)
        .with_client_interface(client_interface);
        RuntimeProviderRun::new(
            id,
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: format!("{adapter_key}:{provider}:claude-sonnet-4-6"),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: std::collections::BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        )
    }

    #[test]
    fn remote_provider_projection_does_not_regress_to_starting() {
        let store = crate::runtime::projection::ProviderRunProjectionStore::default();
        let mut running = provider_run(
            "provider-run-1",
            "session-1",
            "agent-1",
            "managed-dev-stub",
            "managed-dev-stub",
            ProviderClientInterface::Chariox,
        );
        running.mark_running();
        store.update_remote_snapshot(running);

        let stale_starting = provider_run(
            "provider-run-1",
            "session-1",
            "agent-1",
            "managed-dev-stub",
            "managed-dev-stub",
            ProviderClientInterface::Chariox,
        );
        let retained = store.update_remote_snapshot(stale_starting);

        assert_eq!(retained.state(), crate::provider::ProviderRunState::Running);
        assert_eq!(
            store
                .get("provider-run-1")
                .expect("provider projection should remain available")
                .state(),
            crate::provider::ProviderRunState::Running
        );
    }

    #[test]
    fn provider_state_change_projects_without_terminal_records() {
        let mut config = DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut app =
            crate::app::DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
        let lease = RemoteLeaseRuntime::new(&mut app)
            .create_execution_lease(
                "home-kernel",
                "session-1",
                "agent-home-1",
                false,
                "user-home",
            )
            .expect("execution lease should be created");
        let leased_agent = RemoteLeaseRuntime::new(&mut app)
            .create_leased_agent(
                &lease.id,
                "managed-dev-stub",
                "default",
                Some("default".to_string()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .expect("leased agent should be created");
        let mut run = provider_run(
            "provider-run-state-only",
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
            "managed-dev-stub",
            "managed-dev-stub",
            ProviderClientInterface::Chariox,
        );
        run.mark_running();
        app.providers_mut().insert_run_for_test(run.clone());

        let first = RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased_agent.id, run.id(), false)
            .expect("state-only projection should succeed")
            .expect("new provider state should be projected");
        let RelayPeerEvent::LeasedRuntimeProjection {
            provider_run,
            prompts,
            output_chunks,
            notices,
            completions,
            ..
        } = first.1;
        assert_eq!(
            provider_run.map(|provider_run| provider_run.state()),
            Some(crate::provider::ProviderRunState::Running)
        );
        assert!(prompts.is_empty());
        assert!(output_chunks.is_empty());
        assert!(notices.is_empty());
        assert!(completions.is_empty());

        assert!(RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased_agent.id, run.id(), false)
            .expect("unchanged provider state drain should succeed")
            .is_none());

        run.mark_parked();
        app.providers_mut().insert_run_for_test(run.clone());
        let parked = RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased_agent.id, run.id(), false)
            .expect("parked state projection should succeed")
            .expect("changed provider state should be projected");
        let RelayPeerEvent::LeasedRuntimeProjection { provider_run, .. } = parked.1;
        assert_eq!(
            provider_run.map(|provider_run| provider_run.state()),
            Some(crate::provider::ProviderRunState::Parked)
        );
    }

    #[test]
    fn commentary_output_does_not_complete_an_explicit_codex_turn() {
        let mut config = DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut app =
            crate::app::DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
        let lease = RemoteLeaseRuntime::new(&mut app)
            .create_execution_lease(
                "home-kernel",
                "session-1",
                "agent-home-1",
                false,
                "user-home",
            )
            .expect("execution lease should be created");
        let leased_agent = RemoteLeaseRuntime::new(&mut app)
            .create_leased_agent(
                &lease.id,
                "managed-dev-stub",
                "default",
                Some("sonnet".to_string()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .expect("leased agent should be created");
        let (provider_run_id, outcome) = RemoteLeaseRuntime::new(&mut app)
            .submit_leased_prompt(&leased_agent.id, "remote leased prompt\n", Vec::new())
            .expect("leased prompt should submit");
        assert!(matches!(outcome, PromptSubmissionOutcome::Started { .. }));
        app.leased_agents
            .get_mut(&leased_agent.id)
            .expect("leased agent should remain registered")
            .provider = "codex".to_string();
        app.terminal_mut().fan_out_output(
            &leased_agent.backing_session_id,
            &provider_run_id,
            Some(&leased_agent.backing_agent_id),
            crate::terminal::TerminalOutputKind::ProviderOutput,
            Some("commentary-output".to_string()),
            vec![leased_agent.backing_attachment_id.clone()],
            b"Running the requested command.",
        );

        let first = RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
            .expect("commentary projection should succeed")
            .expect("commentary output should be projected");
        let RelayPeerEvent::LeasedRuntimeProjection {
            output_chunks,
            completions,
            ..
        } = first.1;
        assert_eq!(output_chunks.len(), 1);
        assert!(completions.is_empty());
        assert!(app
            .prompt_owner_active_prompt_for_agent_snapshot(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )
            .expect("active prompt should load")
            .is_some());
        let started_at_ms = RemoteLeaseRuntime::new(&mut app)
            .leased_agent_snapshot_for_test(&leased_agent.id)
            .and_then(|agent| agent.active_home_prompt_started_at_ms)
            .expect("active home prompt should remember its worker start time");

        let completed_at_ms = started_at_ms.saturating_add(1);
        app.terminal_mut().record_assistant_message_completion(
            &leased_agent.backing_session_id,
            &provider_run_id,
            Some(&leased_agent.backing_agent_id),
            vec![leased_agent.backing_attachment_id.clone()],
            "assistant-msg-explicit",
            completed_at_ms,
        );
        assert!(RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
            .expect("explicit completion projection should succeed")
            .is_none());
        assert!(app
            .prompt_owner_active_prompt_for_agent_snapshot(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )
            .expect("active prompt should load")
            .is_some());

        app.complete_active_prompt(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
            Some(&provider_run_id),
        )
        .expect("provider turn completion should settle the backing prompt");
        let settled = RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection_with_recovery(
                &leased_agent.id,
                &provider_run_id,
                false,
                true,
            )
            .expect("settled completion projection should succeed")
            .expect("provider turn completion should release the deferred message");
        let RelayPeerEvent::LeasedRuntimeProjection { completions, .. } = settled.1;
        assert_eq!(completions.len(), 1);
        assert_eq!(completions[0].message_id, "assistant-msg-explicit");
        assert_eq!(completions[0].completed_at_ms, completed_at_ms);
    }

    #[test]
    fn native_terminal_frames_do_not_release_a_deferred_explicit_codex_completion() {
        let mut config = DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut app =
            crate::app::DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
        let lease = RemoteLeaseRuntime::new(&mut app)
            .create_execution_lease(
                "home-kernel",
                "session-1",
                "agent-home-1",
                false,
                "user-home",
            )
            .expect("execution lease should be created");
        let leased_agent = RemoteLeaseRuntime::new(&mut app)
            .create_leased_agent(
                &lease.id,
                "managed-dev-stub",
                "default",
                Some("sonnet".to_string()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .expect("leased agent should be created");
        let (provider_run_id, outcome) = RemoteLeaseRuntime::new(&mut app)
            .submit_leased_prompt(&leased_agent.id, "remote leased prompt\n", Vec::new())
            .expect("leased prompt should submit");
        assert!(matches!(outcome, PromptSubmissionOutcome::Started { .. }));
        app.leased_agents
            .get_mut(&leased_agent.id)
            .expect("leased agent should remain registered")
            .provider = "codex".to_string();
        app.terminal_mut().fan_out_output(
            &leased_agent.backing_session_id,
            &provider_run_id,
            Some(&leased_agent.backing_agent_id),
            crate::terminal::TerminalOutputKind::ProviderTerminal,
            Some("native-terminal".to_string()),
            vec![leased_agent.backing_attachment_id.clone()],
            b"terminal paint only",
        );

        let terminal_frame = RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
            .expect("terminal frame projection should succeed")
            .expect("terminal frame should be projected");
        let RelayPeerEvent::LeasedRuntimeProjection {
            output_chunks,
            completions,
            ..
        } = terminal_frame.1;
        assert_eq!(output_chunks.len(), 1);
        assert_eq!(output_chunks[0].kind, TerminalOutputKind::ProviderTerminal);
        assert!(completions.is_empty());

        let completed_at_ms = RemoteLeaseRuntime::new(&mut app)
            .leased_agent_snapshot_for_test(&leased_agent.id)
            .and_then(|agent| agent.active_home_prompt_started_at_ms)
            .expect("active home prompt should remember its worker start time")
            .saturating_add(1);
        app.terminal_mut().record_assistant_message_completion(
            &leased_agent.backing_session_id,
            &provider_run_id,
            Some(&leased_agent.backing_agent_id),
            vec![leased_agent.backing_attachment_id.clone()],
            "assistant-msg-after-terminal-frame",
            completed_at_ms,
        );

        assert!(RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
            .expect("completion after terminal frame should defer")
            .is_none());
        assert!(app
            .prompt_owner_active_prompt_for_agent_snapshot(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )
            .expect("active prompt should load")
            .is_some());

        app.terminal_mut().fan_out_output(
            &leased_agent.backing_session_id,
            &provider_run_id,
            Some(&leased_agent.backing_agent_id),
            crate::terminal::TerminalOutputKind::ProviderOutput,
            Some("assistant-output".to_string()),
            vec![leased_agent.backing_attachment_id.clone()],
            b"Final answer.",
        );
        let transcript_output = RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
            .expect("transcript output projection should succeed")
            .expect("transcript output should project without settling the turn");
        let RelayPeerEvent::LeasedRuntimeProjection {
            output_chunks,
            completions,
            ..
        } = transcript_output.1;
        assert_eq!(output_chunks.len(), 1);
        assert_eq!(output_chunks[0].kind, TerminalOutputKind::ProviderOutput);
        assert!(completions.is_empty());
        assert!(app
            .prompt_owner_active_prompt_for_agent_snapshot(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )
            .expect("active prompt should load")
            .is_some());

        app.complete_active_prompt(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
            Some(&provider_run_id),
        )
        .expect("provider turn completion should settle the backing prompt");
        let settled = RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection_with_recovery(
                &leased_agent.id,
                &provider_run_id,
                false,
                true,
            )
            .expect("settled completion projection should succeed")
            .expect("provider turn completion should release the deferred message");
        let RelayPeerEvent::LeasedRuntimeProjection { completions, .. } = settled.1;
        assert_eq!(completions.len(), 1);
        assert_eq!(
            completions[0].message_id,
            "assistant-msg-after-terminal-frame",
        );
    }

    #[test]
    fn explicit_completion_waits_for_first_projected_output() {
        let mut config = DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut app =
            crate::app::DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
        let lease = RemoteLeaseRuntime::new(&mut app)
            .create_execution_lease(
                "home-kernel",
                "session-1",
                "agent-home-1",
                false,
                "user-home",
            )
            .expect("execution lease should be created");
        let leased_agent = RemoteLeaseRuntime::new(&mut app)
            .create_leased_agent(
                &lease.id,
                "managed-dev-stub",
                "default",
                Some("default".to_string()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .expect("leased agent should be created");
        let (provider_run_id, outcome) = RemoteLeaseRuntime::new(&mut app)
            .submit_leased_prompt(&leased_agent.id, "remote leased prompt\n", Vec::new())
            .expect("leased prompt should submit");
        assert!(matches!(outcome, PromptSubmissionOutcome::Started { .. }));
        app.leased_agents
            .get_mut(&leased_agent.id)
            .expect("leased agent should remain registered")
            .provider = "codex".to_string();
        let completed_at_ms = RemoteLeaseRuntime::new(&mut app)
            .leased_agent_snapshot_for_test(&leased_agent.id)
            .and_then(|agent| agent.active_home_prompt_started_at_ms)
            .expect("active home prompt should remember its worker start time")
            .saturating_add(1);
        app.terminal_mut().record_assistant_message_completion(
            &leased_agent.backing_session_id,
            &provider_run_id,
            Some(&leased_agent.backing_agent_id),
            vec![leased_agent.backing_attachment_id.clone()],
            "assistant-msg-before-output",
            completed_at_ms,
        );

        let before_output = RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
            .expect("completion-only projection should succeed");
        assert!(before_output.is_none());
        assert!(app
            .prompt_owner_active_prompt_for_agent_snapshot(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )
            .expect("active prompt should load")
            .is_some());

        app.terminal_mut().fan_out_output(
            &leased_agent.backing_session_id,
            &provider_run_id,
            Some(&leased_agent.backing_agent_id),
            crate::terminal::TerminalOutputKind::ProviderOutput,
            Some("assistant-output".to_string()),
            vec![leased_agent.backing_attachment_id.clone()],
            b"CODEX_REMOTE_OUTPUT_OK",
        );
        let after_output = RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
            .expect("output projection should succeed")
            .expect("output should project without settling the turn");
        let RelayPeerEvent::LeasedRuntimeProjection {
            output_chunks,
            completions,
            ..
        } = after_output.1;
        assert_eq!(output_chunks.len(), 1);
        assert!(completions.is_empty());
        assert!(app
            .prompt_owner_active_prompt_for_agent_snapshot(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )
            .expect("active prompt should load")
            .is_some());

        app.complete_active_prompt(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
            Some(&provider_run_id),
        )
        .expect("provider turn completion should settle the backing prompt");
        let settled = RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection_with_recovery(
                &leased_agent.id,
                &provider_run_id,
                false,
                true,
            )
            .expect("settled completion projection should succeed")
            .expect("provider turn completion should release the deferred message");
        let RelayPeerEvent::LeasedRuntimeProjection { completions, .. } = settled.1;
        assert_eq!(completions.len(), 1);
        assert_eq!(completions[0].message_id, "assistant-msg-before-output");
        assert!(app
            .prompt_owner_active_prompt_for_agent_snapshot(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )
            .expect("active prompt should load")
            .is_none());
    }

    #[test]
    fn reused_provider_run_waits_for_output_from_each_home_prompt() {
        let mut config = DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut app =
            crate::app::DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
        let lease = RemoteLeaseRuntime::new(&mut app)
            .create_execution_lease(
                "home-kernel",
                "session-1",
                "agent-home-1",
                false,
                "user-home",
            )
            .expect("execution lease should be created");
        let leased_agent = RemoteLeaseRuntime::new(&mut app)
            .create_leased_agent(
                &lease.id,
                "managed-dev-stub",
                "default",
                Some("default".to_string()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .expect("leased agent should be created");
        let (provider_run_id, first_outcome) = RemoteLeaseRuntime::new(&mut app)
            .submit_leased_prompt(&leased_agent.id, "first remote prompt\n", Vec::new())
            .expect("first leased prompt should submit");
        assert!(matches!(
            first_outcome,
            PromptSubmissionOutcome::Started { .. }
        ));
        app.leased_agents
            .get_mut(&leased_agent.id)
            .expect("leased agent should remain registered")
            .provider = "codex".to_string();
        let first_completed_at_ms = RemoteLeaseRuntime::new(&mut app)
            .leased_agent_snapshot_for_test(&leased_agent.id)
            .and_then(|agent| agent.active_home_prompt_started_at_ms)
            .expect("first home prompt should remember its worker start time")
            .saturating_add(1);
        app.terminal_mut().record_assistant_message_completion(
            &leased_agent.backing_session_id,
            &provider_run_id,
            Some(&leased_agent.backing_agent_id),
            vec![leased_agent.backing_attachment_id.clone()],
            "first-assistant-complete",
            first_completed_at_ms,
        );
        app.terminal_mut().fan_out_output(
            &leased_agent.backing_session_id,
            &provider_run_id,
            Some(&leased_agent.backing_agent_id),
            TerminalOutputKind::ProviderOutput,
            Some("first-assistant-output".to_string()),
            vec![leased_agent.backing_attachment_id.clone()],
            b"FIRST_REMOTE_OUTPUT",
        );
        let first_projection = RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
            .expect("first projection should succeed")
            .expect("first output should project without settling the turn");
        let RelayPeerEvent::LeasedRuntimeProjection {
            output_chunks,
            completions,
            ..
        } = first_projection.1;
        assert_eq!(output_chunks.len(), 1);
        assert!(completions.is_empty());
        app.complete_active_prompt(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
            Some(&provider_run_id),
        )
        .expect("first provider turn should settle");
        let first_settled = RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection_with_recovery(
                &leased_agent.id,
                &provider_run_id,
                false,
                true,
            )
            .expect("first completion projection should succeed")
            .expect("first provider turn should release its completion");
        let RelayPeerEvent::LeasedRuntimeProjection { completions, .. } = first_settled.1;
        assert_eq!(completions.len(), 1);

        // The Codex label above exercises only explicit-completion projection.
        // Admission must still use the actual stub run's execution profile.
        app.leased_agents
            .get_mut(&leased_agent.id)
            .unwrap()
            .provider = leased_agent.provider.clone();
        let (reused_provider_run_id, second_outcome) = RemoteLeaseRuntime::new(&mut app)
            .submit_leased_prompt(&leased_agent.id, "second remote prompt\n", Vec::new())
            .expect("second leased prompt should submit");
        assert!(matches!(
            second_outcome,
            PromptSubmissionOutcome::Started { .. }
        ));
        assert_eq!(reused_provider_run_id, provider_run_id);
        app.leased_agents
            .get_mut(&leased_agent.id)
            .unwrap()
            .provider = "codex".to_string();
        let second_completed_at_ms = RemoteLeaseRuntime::new(&mut app)
            .leased_agent_snapshot_for_test(&leased_agent.id)
            .and_then(|agent| agent.active_home_prompt_started_at_ms)
            .expect("second home prompt should remember its worker start time")
            .saturating_add(1);
        app.terminal_mut().record_assistant_message_completion(
            &leased_agent.backing_session_id,
            &provider_run_id,
            Some(&leased_agent.backing_agent_id),
            vec![leased_agent.backing_attachment_id.clone()],
            "second-assistant-complete",
            second_completed_at_ms,
        );

        let before_second_output = RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
            .expect("second completion-only projection should succeed");
        if let Some((_, RelayPeerEvent::LeasedRuntimeProjection { completions, .. })) =
            before_second_output
        {
            assert!(completions.is_empty());
        }
        assert!(app
            .prompt_owner_active_prompt_for_agent_snapshot(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )
            .expect("second active prompt should load")
            .is_some());
    }

    #[test]
    fn provider_dispatch_failure_completes_a_leased_turn() {
        let mut config = DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut app =
            crate::app::DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
        let lease = RemoteLeaseRuntime::new(&mut app)
            .create_execution_lease(
                "home-kernel",
                "session-1",
                "agent-home-1",
                false,
                "user-home",
            )
            .expect("execution lease should be created");
        let leased_agent = RemoteLeaseRuntime::new(&mut app)
            .create_leased_agent(
                &lease.id,
                "managed-dev-stub",
                "default",
                Some("default".to_string()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .expect("leased agent should be created");
        let (provider_run_id, outcome) = RemoteLeaseRuntime::new(&mut app)
            .submit_leased_prompt(&leased_agent.id, "prompt with revoked auth\n", Vec::new())
            .expect("leased prompt should submit");
        assert!(matches!(outcome, PromptSubmissionOutcome::Started { .. }));
        app.providers_mut()
            .record_terminal_diagnostic(
                &provider_run_id,
                "Provider prompt dispatch failed: refresh token was revoked".to_string(),
            )
            .expect("provider failure should be recorded");
        app.prompt_owner_cancel_active_prompt_only(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        )
        .expect("failed prompt should be cancelled on the worker");

        let projection = RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)
            .expect("failed projection should succeed")
            .expect("failed prompt should produce a projection");
        let RelayPeerEvent::LeasedRuntimeProjection { completions, .. } = projection.1;
        assert_eq!(completions.len(), 1);
        assert!(app
            .prompt_owner_active_prompt_for_agent_snapshot(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )
            .expect("active prompt should load")
            .is_none());
    }

    #[test]
    fn native_prompt_keeps_home_attachment_provenance_in_worker_projection() {
        let mut config = DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut app =
            crate::app::DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
        let lease = RemoteLeaseRuntime::new(&mut app)
            .create_execution_lease(
                "home-kernel",
                "session-1",
                "agent-home-1",
                false,
                "user-home",
            )
            .expect("execution lease should be created");
        let leased_agent = RemoteLeaseRuntime::new(&mut app)
            .create_leased_agent(
                &lease.id,
                "managed-dev-stub",
                "default",
                Some("default".to_string()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .expect("leased agent should be created");
        let provider_run = RemoteLeaseRuntime::new(&mut app)
            .launch_leased_native_provider_run(
                &leased_agent.id,
                "managed-dev-stub",
                "managed-dev-stub",
                "default",
                "default",
                None,
                None,
                None,
                Vec::new(),
                Some(Vec::new()),
                crate::extension::RemoteExtensionManifest::default(),
            )
            .expect("native provider run should launch");
        let outcome = app
            .record_native_prompt_started_with_attachments(
                &leased_agent.backing_session_id,
                &leased_agent.backing_attachment_id,
                "home-native-attachment",
                &leased_agent.backing_agent_id,
                "native prompt from home TUI",
                Vec::new(),
            )
            .expect("native prompt should be recorded");
        assert!(matches!(outcome, PromptSubmissionOutcome::Started { .. }));

        let projection = RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased_agent.id, provider_run.id(), false)
            .expect("native prompt projection should succeed")
            .expect("native prompt should produce a projection");
        let RelayPeerEvent::LeasedRuntimeProjection { prompts, .. } = projection.1;
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].text, "native prompt from home TUI\n");
    }

    #[test]
    fn claude_headless_leased_runs_require_explicit_completion() {
        let headless = provider_run(
            "provider-run-headless-claude",
            "session-1",
            "agent-1",
            "claude",
            "claude-headless",
            ProviderClientInterface::Chariox,
        );
        let structured = provider_run(
            "provider-run-structured-claude",
            "session-1",
            "agent-1",
            "claude",
            "claude",
            ProviderClientInterface::Chariox,
        );
        assert!(leased_provider_requires_explicit_completion(
            "claude",
            Some(&headless),
        ));
        assert!(!leased_provider_requires_explicit_completion(
            "claude",
            Some(&structured),
        ));
    }

    #[test]
    fn claude_native_tui_leased_runs_require_explicit_completion() {
        let run = provider_run(
            "provider-run-native-claude",
            "session-1",
            "agent-1",
            "claude",
            "claude",
            ProviderClientInterface::NativeTui,
        );
        assert!(crate::provider::provider_run_uses_claude_native_bridge(
            &run
        ));
        assert!(leased_provider_requires_explicit_completion(
            "claude",
            Some(&run),
        ));
    }

    #[test]
    fn claude_native_composer_output_does_not_complete_a_leased_prompt() {
        let mut config = DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut app =
            crate::app::DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
        let lease = RemoteLeaseRuntime::new(&mut app)
            .create_execution_lease(
                "home-kernel",
                "session-1",
                "agent-home-1",
                false,
                "user-home",
            )
            .expect("execution lease should be created");
        let leased_agent = RemoteLeaseRuntime::new(&mut app)
            .create_leased_agent(
                &lease.id,
                "managed-dev-stub",
                "default",
                Some("default".to_string()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .expect("leased agent should be created");
        let mut provider_run = provider_run(
            "provider-run-native-claude",
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
            "claude",
            "claude",
            ProviderClientInterface::NativeTui,
        );
        provider_run.mark_running();
        app.providers_mut()
            .insert_run_for_test(provider_run.clone());
        let outcome = app
            .record_native_prompt_started_with_attachments(
                &leased_agent.backing_session_id,
                &leased_agent.backing_attachment_id,
                "home-native-attachment",
                &leased_agent.backing_agent_id,
                "prompt waiting for Enter",
                Vec::new(),
            )
            .expect("native prompt should be recorded");
        assert!(matches!(outcome, PromptSubmissionOutcome::Started { .. }));
        app.terminal_mut().fan_out_output(
            &leased_agent.backing_session_id,
            provider_run.id(),
            Some(&leased_agent.backing_agent_id),
            TerminalOutputKind::ProviderOutput,
            Some("composer-redraw".to_string()),
            vec![leased_agent.backing_attachment_id.clone()],
            b"prompt waiting for Enter",
        );

        let projection = RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased_agent.id, provider_run.id(), false)
            .expect("composer projection should succeed")
            .expect("composer output should be projected");
        let RelayPeerEvent::LeasedRuntimeProjection { completions, .. } = projection.1;
        assert!(completions.is_empty());
        assert!(app
            .prompt_owner_active_prompt_for_agent_snapshot(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )
            .expect("active prompt should load")
            .is_some());
    }

    #[test]
    fn claude_native_assistant_completion_waits_for_stop_and_replays_once() {
        let mut config = DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut app =
            crate::app::DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
        let lease = RemoteLeaseRuntime::new(&mut app)
            .create_execution_lease(
                "home-kernel",
                "session-1",
                "agent-home-1",
                false,
                "user-home",
            )
            .expect("execution lease should be created");
        let leased_agent = RemoteLeaseRuntime::new(&mut app)
            .create_leased_agent(
                &lease.id,
                "managed-dev-stub",
                "default",
                Some("default".to_string()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .expect("leased agent should be created");
        let mut provider_run = provider_run(
            "provider-run-native-claude",
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
            "claude",
            "claude-headless",
            ProviderClientInterface::NativeTui,
        );
        provider_run.mark_running();
        app.providers_mut()
            .insert_run_for_test(provider_run.clone());
        let outcome = app
            .record_native_prompt_started_with_attachments(
                &leased_agent.backing_session_id,
                &leased_agent.backing_attachment_id,
                "home-native-attachment",
                &leased_agent.backing_agent_id,
                "prompt remains active through assistant transcript updates",
                Vec::new(),
            )
            .expect("native prompt should be recorded");
        assert!(matches!(outcome, PromptSubmissionOutcome::Started { .. }));
        app.terminal_mut().fan_out_output(
            &leased_agent.backing_session_id,
            provider_run.id(),
            Some(&leased_agent.backing_agent_id),
            TerminalOutputKind::ProviderOutput,
            Some("assistant-output".to_string()),
            vec![leased_agent.backing_attachment_id.clone()],
            b"assistant output before Stop",
        );
        let completed_at_ms = crate::session::unix_epoch_ms();
        app.terminal_mut().record_assistant_message_completion(
            &leased_agent.backing_session_id,
            provider_run.id(),
            Some(&leased_agent.backing_agent_id),
            vec![leased_agent.backing_attachment_id.clone()],
            "claude-assistant-message",
            completed_at_ms,
        );
        crate::transport::flow_control::mark_prompt_completion_recorded(
            &mut app,
            provider_run.id(),
        );

        let before_stop = RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased_agent.id, provider_run.id(), false)
            .expect("pre-Stop projection should succeed")
            .expect("assistant output should be projected");
        let RelayPeerEvent::LeasedRuntimeProjection {
            output_chunks,
            completions,
            ..
        } = before_stop.1;
        assert_eq!(output_chunks.len(), 1);
        assert!(completions.is_empty());
        assert!(app
            .prompt_owner_active_prompt_for_agent_snapshot(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )
            .expect("active prompt should load")
            .is_some());

        app.complete_active_prompt(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
            Some(provider_run.id()),
        )
        .expect("authoritative Stop should complete the worker prompt");
        let after_stop = RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased_agent.id, provider_run.id(), false)
            .expect("post-Stop projection should succeed")
            .expect("deferred assistant completion should be replayed");
        let RelayPeerEvent::LeasedRuntimeProjection { completions, .. } = after_stop.1;
        assert_eq!(completions.len(), 1);
        assert_eq!(completions[0].message_id, "claude-assistant-message");
        assert_eq!(completions[0].completed_at_ms, completed_at_ms);

        let duplicate = RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased_agent.id, provider_run.id(), false)
            .expect("duplicate projection check should succeed");
        assert!(duplicate.is_none());
    }
}
