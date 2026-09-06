use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::app::{DaemonApp, TrackedProviderProcess};
use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::provider::{
    AgentEndpointMode, ProviderProcessInfo, ProviderRunState, RuntimeProviderRun,
};

use super::provider_liveness::poll_provider_run_process_running;

pub(crate) struct ProviderLaunchProcessRuntime<'a> {
    app: &'a mut DaemonApp,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ProviderProcessReapSummary {
    pub(crate) tracked_processes_reaped: usize,
    pub(crate) orphan_processes_reaped: usize,
}

impl<'a> ProviderLaunchProcessRuntime<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn spawn_for_launch(&mut self, run: &RuntimeProviderRun) -> Result<(), DaemonError> {
        self.spawn_for_launch_with_credentials(
            run,
            &crate::provider::ProviderCredentialEnvironment::default(),
        )
    }

    pub(crate) fn spawn_for_launch_with_credentials(
        &mut self,
        run: &RuntimeProviderRun,
        credentials: &crate::provider::ProviderCredentialEnvironment,
    ) -> Result<(), DaemonError> {
        if run.endpoint_mode() != AgentEndpointMode::Managed {
            return Ok(());
        }
        self.app
            .pty
            .spawn_for_run_with_credentials(run, credentials)?;
        ProviderProcessTracker::new(self.app).register_managed_run(run)
    }

    pub(crate) fn remove_run(
        &mut self,
        provider_run_id: &str,
    ) -> Result<(bool, Option<String>), DaemonError> {
        remove_provider_pty_process(self.app, provider_run_id)
    }

    pub(crate) fn poll_running(&mut self, provider_run_id: &str) -> Result<bool, DaemonError> {
        poll_provider_run_process_running(self.app, provider_run_id)
    }
}

pub(crate) struct ProviderProcessTracker<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> ProviderProcessTracker<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn register_managed_run(
        &mut self,
        run: &RuntimeProviderRun,
    ) -> Result<(), DaemonError> {
        let process_key = self.app.pty.process_key(run.id())?;
        let pid = self.app.pty.process_id(run.id())?;
        let process_id = format!("managed:{}:{}", run.provider(), process_key);
        let mut tracking = self.app.provider_process_tracking.write();
        let entry = tracking
            .processes
            .entry(process_key.clone())
            .or_insert_with(|| TrackedProviderProcess {
                process_id: process_id.clone(),
                pid,
                endpoint_mode: run.endpoint_mode(),
                process_label: run.process_label().to_string(),
                started_at_ms: run.started_at_ms(),
                owner_provider_run_ids: Vec::new(),
            });
        entry.pid = pid.or(entry.pid);
        if !entry.owner_provider_run_ids.iter().any(|id| id == run.id()) {
            entry.owner_provider_run_ids.push(run.id().to_string());
        }
        tracking
            .run_processes
            .insert(run.id().to_string(), process_key.clone());
        drop(tracking);
        crate::logging::info_with_fields(
            "daemon.provider_process_gc",
            "registered managed provider process",
            serde_json::json!({
                "pid": pid,
                "process_key": process_key,
                "provider_run_id": run.id(),
            }),
        );
        Ok(())
    }

    pub(crate) fn remove_run(&mut self, provider_run_id: &str) -> Result<bool, DaemonError> {
        let tracked_process_key = self
            .app
            .provider_process_tracking
            .read()
            .run_processes
            .get(provider_run_id)
            .cloned();
        let (removed, process_key) = remove_provider_pty_process(self.app, provider_run_id)?;
        let process_key = tracked_process_key.or(process_key);
        let Some(process_key) = process_key else {
            return Ok(removed);
        };
        let mut tracking = self.app.provider_process_tracking.write();
        tracking.run_processes.remove(provider_run_id);
        let should_remove_entry = if let Some(entry) = tracking.processes.get_mut(&process_key) {
            entry
                .owner_provider_run_ids
                .retain(|id| id != provider_run_id);
            entry.owner_provider_run_ids.is_empty()
        } else {
            false
        };
        if should_remove_entry {
            tracking.processes.remove(&process_key);
        }
        Ok(removed)
    }

    pub(crate) fn list(
        app: &DaemonApp,
        provider: Option<&str>,
    ) -> Result<Vec<ProviderProcessInfo>, DaemonError> {
        let processes = Self::snapshot(app);
        app.update_provider_process_projection(processes.clone());
        Ok(filter_provider_processes(processes, provider))
    }

    pub(crate) fn teardown_safe_processes(
        &mut self,
        provider: Option<&str>,
        force: bool,
    ) -> Result<Vec<ProviderProcessInfo>, DaemonError> {
        let safe_processes = Self::list(self.app, provider)?
            .into_iter()
            .filter(|process| {
                process.teardown_safe
                    || (force
                        && !process.teardown_blockers.iter().any(|blocker| {
                            blocker == "active prompt"
                                || blocker.starts_with("active workflow runs:")
                        }))
            })
            .collect::<Vec<_>>();
        for process in &safe_processes {
            let run_ids: Vec<String> = self
                .app
                .provider_process_tracking
                .read()
                .processes
                .values()
                .find(|tracked| tracked.process_id == process.process_id)
                .map(|tracked| tracked.owner_provider_run_ids.clone())
                .unwrap_or_else(|| process.owner_provider_run_ids.clone());
            for run_id in run_ids {
                let run = match self.app.providers.get_run(&run_id) {
                    Ok(run) => run,
                    Err(_) => continue,
                };
                if run.state() == ProviderRunState::Ended {
                    continue;
                }
                if let Ok(outcome) = self
                    .app
                    .providers
                    .terminate_run_provider_only(run.session_id(), run.id())
                {
                    clear_active_provider_run_session_pointer(
                        self.app,
                        run.session_id(),
                        outcome.run().id(),
                    )?;
                    self.app.update_provider_run_projection(outcome.into_run());
                }
                let _ = self.remove_run(run.id());
            }
        }
        self.app
            .update_provider_process_projection(Self::snapshot(self.app));
        Ok(safe_processes)
    }

    pub(crate) fn reap_idle_processes(
        &mut self,
        now_ms: u64,
        idle_ttl_ms: u64,
        orphan_ttl_ms: u64,
    ) -> Result<ProviderProcessReapSummary, DaemonError> {
        let mut summary = ProviderProcessReapSummary::default();
        let safe_processes = Self::snapshot(self.app)
            .into_iter()
            .filter(|process| process.teardown_safe)
            .filter(|process| now_ms.saturating_sub(process.last_activity_at_ms) >= idle_ttl_ms)
            .collect::<Vec<_>>();

        for process in safe_processes {
            self.reap_tracked_process(&process)?;
            summary.tracked_processes_reaped += 1;
        }

        let orphan_process_ids = owned_orphan_provider_process_ids(
            self.app.config(),
            &tracked_provider_pids(self.app),
            orphan_ttl_ms,
        );
        for pid in orphan_process_ids {
            if crate::runtime::process_health::terminate_process_tree(pid) {
                summary.orphan_processes_reaped += 1;
                crate::logging::warn_with_fields(
                    "daemon.provider_process_gc",
                    "reaped orphaned managed provider process",
                    serde_json::json!({
                        "pid": pid,
                    }),
                );
            }
        }

        self.app
            .update_provider_process_projection(Self::snapshot(self.app));
        Ok(summary)
    }

    fn reap_tracked_process(&mut self, process: &ProviderProcessInfo) -> Result<(), DaemonError> {
        let (process_key, run_ids, pid) = {
            let tracking = self.app.provider_process_tracking.read();
            let Some((process_key, tracked)) = tracking
                .processes
                .iter()
                .find(|(_, tracked)| tracked.process_id == process.process_id)
            else {
                return Ok(());
            };
            (
                process_key.clone(),
                tracked.owner_provider_run_ids.clone(),
                tracked.pid,
            )
        };

        for run_id in &run_ids {
            if let Ok(run) = self.app.providers.get_run(run_id) {
                if run.state() != ProviderRunState::Ended {
                    if let Ok(outcome) = self
                        .app
                        .providers
                        .terminate_run_provider_only(run.session_id(), run.id())
                    {
                        clear_active_provider_run_session_pointer(
                            self.app,
                            run.session_id(),
                            outcome.run().id(),
                        )?;
                        self.app.update_provider_run_projection(outcome.into_run());
                    }
                }
            }
            let _ = self.remove_run(run_id);
        }

        let removed_by_key = self.app.pty.remove_process_by_key(&process_key, None)?;
        if !removed_by_key {
            if let Some(pid) = pid {
                let _ = crate::runtime::process_health::terminate_process_tree(pid);
            }
            self.app
                .provider_process_tracking
                .write()
                .processes
                .remove(&process_key);
        }

        crate::logging::warn_with_fields(
            "daemon.provider_process_gc",
            "reaped idle managed provider process",
            serde_json::json!({
                "process_id": process.process_id,
                "pid": pid,
                "owner_provider_run_ids": run_ids,
            }),
        );

        Ok(())
    }

    fn snapshot(app: &DaemonApp) -> Vec<ProviderProcessInfo> {
        let mut processes = Vec::new();
        let tracking = app.provider_process_tracking.read();
        for tracked in tracking.processes.values() {
            let runs = tracked
                .owner_provider_run_ids
                .iter()
                .filter_map(|run_id| app.providers.get_run(run_id).ok())
                .filter(|run| run.state() != ProviderRunState::Ended)
                .collect::<Vec<_>>();
            if runs.is_empty() {
                continue;
            }
            let owner_session_ids = runs
                .iter()
                .map(|run| run.session_id().to_string())
                .collect::<BTreeSet<_>>();
            let attached_session_ids = owner_session_ids
                .iter()
                .filter(|session_id| {
                    !app.attachments
                        .list_session_attachment_ids(session_id)
                        .is_empty()
                })
                .cloned()
                .collect::<BTreeSet<_>>();
            let active_workflow_run_ids = owner_session_ids
                .iter()
                .flat_map(|session_id| {
                    app.sessions
                        .get_session(session_id)
                        .ok()
                        .map(|session| session.workflow_runs().iter().cloned().collect::<Vec<_>>())
                        .into_iter()
                        .flatten()
                        .filter(|run| {
                            !matches!(
                                run.status(),
                                crate::session::WorkflowRunStatus::Completed
                                    | crate::session::WorkflowRunStatus::Failed
                                    | crate::session::WorkflowRunStatus::Stopped
                            )
                        })
                        .map(|run| run.id().to_string())
                })
                .collect::<BTreeSet<_>>();
            let has_active_prompt = owner_session_ids.iter().any(|session_id| {
                app.sessions
                    .get_session(session_id)
                    .ok()
                    .is_some_and(|session| app.prompt_state_owner.has_any_active_prompt(&session))
            });
            let mut teardown_blockers = Vec::new();
            if !attached_session_ids.is_empty() {
                teardown_blockers.push(format!(
                    "attached sessions: {}",
                    attached_session_ids
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(",")
                ));
            }
            if has_active_prompt {
                teardown_blockers.push("active prompt".to_string());
            }
            if !active_workflow_run_ids.is_empty() {
                teardown_blockers.push(format!(
                    "active workflow runs: {}",
                    active_workflow_run_ids
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(",")
                ));
            }
            let teardown_safe = attached_session_ids.is_empty()
                && active_workflow_run_ids.is_empty()
                && !has_active_prompt;
            if let Some(mut process) = ProviderProcessInfo::from_runs(
                tracked.process_id.clone(),
                &runs,
                attached_session_ids,
                active_workflow_run_ids,
                teardown_safe,
                teardown_blockers,
            ) {
                process.pid = tracked.pid;
                process.resident_set_bytes = tracked
                    .pid
                    .and_then(crate::runtime::process_health::resident_set_bytes_for_pid);
                process.process_label = tracked.process_label.clone();
                process.endpoint_mode = tracked.endpoint_mode;
                process.started_at_ms = tracked.started_at_ms;
                processes.push(process);
            }
        }
        processes
    }
}

fn remove_provider_pty_process(
    app: &mut DaemonApp,
    provider_run_id: &str,
) -> Result<(bool, Option<String>), DaemonError> {
    let process_key = app.pty.process_key(provider_run_id).ok();
    let cleanup_root = app
        .providers
        .get_run(provider_run_id)
        .ok()
        .and_then(|run| claude_native_tui_cleanup_root(&run));
    let removed = app.pty.remove_process(provider_run_id)?;
    let process_still_running = process_key
        .as_deref()
        .is_some_and(|key| app.pty.has_process_key(key));
    if !process_still_running {
        if let Some(root) = cleanup_root {
            let _ = std::fs::remove_dir_all(root);
        }
    }
    Ok((removed, process_key))
}

fn claude_native_tui_cleanup_root(run: &RuntimeProviderRun) -> Option<PathBuf> {
    if run.endpoint_mode() != AgentEndpointMode::Managed
        || !run.process_label().starts_with("claude:")
    {
        return None;
    }
    let events_file = PathBuf::from(run.pty_env().get("CHARIOX_CLAUDE_NATIVE_EVENTS")?);
    let root = events_file.parent()?;
    if root.parent() != Some(std::env::temp_dir().as_path())
        || !root
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.starts_with("chariox-claude-remote-native-"))
    {
        return None;
    }
    for key in [
        "CHARIOX_CLAUDE_NATIVE_CONTEXT",
        "CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES",
        "CHARIOX_CLAUDE_NATIVE_PERMISSION_RESPONSES",
        "CHARIOX_CLAUDE_USAGE_FILE",
    ] {
        if Path::new(run.pty_env().get(key)?).parent() != Some(root) {
            return None;
        }
    }
    Some(root.to_path_buf())
}

fn tracked_provider_pids(app: &DaemonApp) -> BTreeSet<u32> {
    app.provider_process_tracking
        .read()
        .processes
        .values()
        .filter_map(|process| process.pid)
        .collect()
}

fn owned_orphan_provider_process_ids(
    config: &DaemonConfig,
    tracked_pids: &BTreeSet<u32>,
    orphan_ttl_ms: u64,
) -> Vec<u32> {
    let output = match std::process::Command::new("ps")
        .args(["-axo", "pid=,ppid=,pgid=,etimes=,command="])
        .output()
    {
        Ok(output) => output,
        Err(_) => return Vec::new(),
    };
    let mcp_url = format!(
        "http://{}:{}/mcp",
        config.runtime_mcp_host, config.runtime_mcp_port
    );
    owned_orphan_provider_process_ids_from_ps_output(
        &String::from_utf8_lossy(&output.stdout),
        std::process::id(),
        tracked_pids,
        orphan_ttl_ms,
        &mcp_url,
    )
}

#[derive(Debug)]
struct ProviderProcessSnapshot {
    pid: u32,
    parent_pid: u32,
    process_group_id: u32,
    age_secs: u64,
    command: String,
}

fn owned_orphan_provider_process_ids_from_ps_output(
    ps_output: &str,
    current_pid: u32,
    tracked_pids: &BTreeSet<u32>,
    orphan_ttl_ms: u64,
    mcp_url: &str,
) -> Vec<u32> {
    let min_age_secs = orphan_ttl_ms.div_ceil(1_000);
    let processes = ps_output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let pid = parts.next()?.parse::<u32>().ok()?;
            let parent_pid = parts.next()?.parse::<u32>().ok()?;
            let process_group_id = parts.next()?.parse::<u32>().ok()?;
            let age_secs = parts.next()?.parse::<u64>().ok()?;
            let command = parts.collect::<Vec<_>>().join(" ");
            Some(ProviderProcessSnapshot {
                pid,
                parent_pid,
                process_group_id,
                age_secs,
                command,
            })
        })
        .map(|process| (process.pid, process))
        .collect::<BTreeMap<_, _>>();
    let tracked_process_group_ids = tracked_pids
        .iter()
        .filter_map(|pid| processes.get(pid))
        .map(|process| process.process_group_id)
        .filter(|process_group_id| *process_group_id > 0)
        .collect::<BTreeSet<_>>();

    processes
        .values()
        .filter_map(|process| {
            if process.pid == current_pid
                || tracked_pids.contains(&process.pid)
                || tracked_process_group_ids.contains(&process.process_group_id)
                || has_protected_process_ancestor(process, &processes, tracked_pids, current_pid)
                || process.age_secs < min_age_secs
                || !process.command.contains("codex app-server")
                || !process.command.contains("mcp_servers.chariox.url")
                || !process.command.contains(mcp_url)
            {
                return None;
            }
            Some(process.pid)
        })
        .collect()
}

fn has_protected_process_ancestor(
    process: &ProviderProcessSnapshot,
    processes: &BTreeMap<u32, ProviderProcessSnapshot>,
    tracked_pids: &BTreeSet<u32>,
    current_pid: u32,
) -> bool {
    let mut parent_pid = process.parent_pid;
    let mut visited = BTreeSet::new();
    while parent_pid > 0 && visited.insert(parent_pid) {
        if parent_pid == current_pid || tracked_pids.contains(&parent_pid) {
            return true;
        }
        let Some(parent) = processes.get(&parent_pid) else {
            break;
        };
        parent_pid = parent.parent_pid;
    }
    false
}

fn clear_active_provider_run_session_pointer(
    app: &mut DaemonApp,
    session_id: &str,
    provider_run_id: &str,
) -> Result<(), DaemonError> {
    if app
        .sessions
        .get_session(session_id)?
        .active_provider_run_id()
        == Some(provider_run_id)
    {
        app.sessions.set_active_provider_run(session_id, None)?;
    }
    Ok(())
}

fn filter_provider_processes(
    processes: Vec<ProviderProcessInfo>,
    provider: Option<&str>,
) -> Vec<ProviderProcessInfo> {
    let Some(provider) = provider else {
        return processes;
    };
    processes
        .into_iter()
        .filter(|process| process.provider == provider)
        .collect()
}

impl DaemonApp {
    pub fn list_provider_processes(
        &self,
        provider: Option<&str>,
    ) -> Result<Vec<ProviderProcessInfo>, DaemonError> {
        ProviderProcessTracker::list(self, provider)
    }

    pub fn teardown_provider_processes(
        &mut self,
        provider: Option<&str>,
        force: bool,
    ) -> Result<Vec<ProviderProcessInfo>, DaemonError> {
        ProviderProcessTracker::new(self).teardown_safe_processes(provider, force)
    }

    pub(crate) fn reap_idle_provider_processes(
        &mut self,
        now_ms: u64,
        idle_ttl_ms: u64,
        orphan_ttl_ms: u64,
    ) -> Result<ProviderProcessReapSummary, DaemonError> {
        ProviderProcessTracker::new(self).reap_idle_processes(now_ms, idle_ttl_ms, orphan_ttl_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::owned_orphan_provider_process_ids_from_ps_output;
    use std::collections::BTreeSet;

    const MCP_URL: &str = "http://127.0.0.1:49999/mcp";

    fn codex_command(port: u16) -> String {
        format!(
            "codex app-server -c mcp_servers.chariox.url=\"{MCP_URL}\" --listen ws://127.0.0.1:{port}"
        )
    }

    #[test]
    fn orphan_scan_preserves_children_and_process_group_members_of_tracked_launchers() {
        let ps_output = format!(
            "100 1 100 45 node {}\n\
             101 100 100 44 /opt/codex {}\n\
             102 101 102 43 /opt/codex {}\n\
             103 1 100 42 /opt/codex {}\n\
             200 1 200 41 /opt/codex {}\n",
            codex_command(50001),
            codex_command(50001),
            codex_command(50001),
            codex_command(50001),
            codex_command(50002),
        );

        let orphan_ids = owned_orphan_provider_process_ids_from_ps_output(
            &ps_output,
            999,
            &BTreeSet::from([100]),
            30_000,
            MCP_URL,
        );

        assert_eq!(orphan_ids, vec![200]);
    }

    #[test]
    fn orphan_scan_preserves_processes_parented_by_the_current_kernel() {
        let ps_output = format!(
            "300 999 300 45 node {}\n\
             301 300 300 44 /opt/codex {}\n\
             400 1 400 43 /opt/codex {}\n",
            codex_command(50001),
            codex_command(50001),
            codex_command(50002),
        );

        let orphan_ids = owned_orphan_provider_process_ids_from_ps_output(
            &ps_output,
            999,
            &BTreeSet::new(),
            30_000,
            MCP_URL,
        );

        assert_eq!(orphan_ids, vec![400]);
    }

    #[test]
    fn orphan_scan_requires_age_managed_command_and_runtime_mcp_url() {
        let ps_output = format!(
            "200 1 200 29 /opt/codex {}\n\
             201 1 201 30 /opt/codex codex app-server --listen ws://127.0.0.1:50002\n\
             202 1 202 30 /opt/codex codex app-server -c mcp_servers.chariox.url=\"http://127.0.0.1:49998/mcp\"\n\
             203 1 203 30 /opt/codex {}\n",
            codex_command(50001),
            codex_command(50003),
        );

        let orphan_ids = owned_orphan_provider_process_ids_from_ps_output(
            &ps_output,
            999,
            &BTreeSet::new(),
            30_000,
            MCP_URL,
        );

        assert_eq!(orphan_ids, vec![203]);
    }
}
