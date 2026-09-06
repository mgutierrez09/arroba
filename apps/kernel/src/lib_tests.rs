use std::collections::BTreeMap;
use std::process::Command;
use std::thread;
use std::time::Duration;

use super::agent::{CreateAgentRequest, GitWorktreePlacement};
use super::app::RemoteLeaseRuntime;
use super::attachment::{AttachRequest, ClientCapabilityLevel};
use super::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, ProviderResumeState,
    RuntimeProviderRun,
};
use super::session::{
    CreateSessionRequest, PromptOrigin, PromptStatus, PromptSubmissionOutcome, SessionStatus,
};
use super::terminal::TerminalOutputKind;
use super::transport::relay_peer::{
    RelayPeerEvent, RelayPeerRequest, RelayPeerResponse, RelayProjectedCompletion,
    RelayProjectedOutputChunk, RemoteWorkspaceLiveSyncApplyContext,
    RemoteWorkspaceLiveSyncArtifactState, RemoteWorkspaceLiveSyncContext,
    RemoteWorkspaceLiveSyncInvocationMetadata,
};
use super::{DaemonApp, DaemonConfig, DaemonError};
use sha2::{Digest, Sha256};

fn run_test_git(cwd: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git should run");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn wait_for_terminal_output(
    app: &mut DaemonApp,
    session_id: &str,
    attachment_id: &str,
) -> Vec<super::terminal::TerminalOutputRecord> {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);

    loop {
        let records = crate::app::provider_output::pump_terminal_output_for_attachment(
            app,
            session_id,
            attachment_id,
        )
        .expect("terminal output should fan out");
        if !records.is_empty() {
            return records;
        }

        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for terminal output"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

mod app_lifecycle;
mod architecture_boundaries;
mod capability_boundaries;
mod client_protocol_conformance;
mod performance_drills;
mod provider_sessions;
mod remote_leases;

#[test]
fn relay_peer_workspace_live_sync_apply_shape_is_versioned() {
    assert_eq!(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, 309);

    let context = RemoteWorkspaceLiveSyncApplyContext {
        home_session_id: "session-1".to_string(),
        link_id: "workspace-link-1".to_string(),
        link_name: "shared".to_string(),
        source_agent_id: "agent-1".to_string(),
        source_worktree_path: "/source".to_string(),
        target_user_id: "user-2".to_string(),
        target_machine_id: "machine-2".to_string(),
        target_kernel_id: "kernel-2".to_string(),
        target_repo_root: "/target".to_string(),
    };
    let change = crate::git_observer::WorkspaceLiveSyncChange {
        session_id: "session-1".to_string(),
        agent_id: "agent-1".to_string(),
        provider_run_id: "run-1".to_string(),
        prompt_id: "prompt-1".to_string(),
        repo_root: "/source".to_string(),
        worktree_path: "/source".to_string(),
        branch: Some("main".to_string()),
        changed_paths: vec!["src/lib.rs".to_string()],
        file_changes: vec![crate::git_observer::WorkspaceLiveSyncFileChange {
            path: "src/lib.rs".to_string(),
            previous_path: None,
            kind: crate::git_observer::WorkspaceLiveSyncFileChangeKind::Modified,
            before_content_base64: Some("YmVmb3JlCg==".to_string()),
            after_content_base64: Some("YWZ0ZXIK".to_string()),
            binary: false,
        }],
        status_fingerprint: "fingerprint-1".to_string(),
    };
    let request = RelayPeerRequest::ApplyWorkspaceLiveSyncChange {
        context: context.clone(),
        change,
    };
    let response = RelayPeerResponse::WorkspaceLiveSyncChangeApplied {
        target_result: crate::git_observer::WorkspaceLiveSyncTargetResult {
            session_id: context.home_session_id,
            link_id: context.link_id,
            link_name: context.link_name,
            source_agent_id: context.source_agent_id,
            source_worktree_path: context.source_worktree_path,
            target_user_id: context.target_user_id,
            target_machine_id: context.target_machine_id,
            target_kernel_id: context.target_kernel_id,
            target_repo_root: context.target_repo_root,
            path_results: vec![crate::git_observer::WorkspaceLiveSyncPathApplyResult {
                path: "src/lib.rs".to_string(),
                status: crate::git_observer::WorkspaceLiveSyncApplyStatus::Rebased,
                message: "rebased over non-overlapping target change".to_string(),
            }],
        },
    };

    let snapshot = serde_json::json!([request, response]);
    assert_eq!(
        snapshot.pointer("/0/kind"),
        Some(&serde_json::json!("apply_workspace_live_sync_change"))
    );
    assert_eq!(
        snapshot.pointer("/1/kind"),
        Some(&serde_json::json!("workspace_live_sync_change_applied"))
    );
    assert_eq!(
        snapshot.pointer("/1/target_result/path_results/0/status"),
        Some(&serde_json::json!("rebased"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("workspace live sync relay apply should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "dd483fae2ed150ca874cd7594ec682e869a5dfb2aa1d73755369bb10c3ce7e8f"
    );
}

#[test]
fn relay_peer_remote_workspace_live_sync_mode_projection_shape_is_versioned() {
    assert_eq!(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, 309);

    let spawn = RelayPeerRequest::SpawnLeasedAgent {
        lease_id: "lease-1".to_string(),
        provider: "codex".to_string(),
        account_profile: "work".to_string(),
        model: Some("gpt-5.5".to_string()),
        effort: None,
        execution_mode: None,
        permission_level: None,
        workspace_live_sync_mode: Some(crate::config::WorkspaceLiveSyncMode::Tracked),
        worktree_id: Some("/worker/repo".to_string()),
        worktree_placement: None,
    };
    let submit = RelayPeerRequest::SubmitLeasedPrompt {
        leased_agent_id: "leased-agent-1".to_string(),
        prompt: "edit a file".to_string(),
        hidden_system_context: "scheduled hidden context".to_string(),
        attachments: Vec::new(),
        workflow_context: None,
        git_context: Some(crate::transport::relay_peer::RemoteGitTurnContext {
            home_session_id: "session-1".to_string(),
            home_agent_id: "agent-1".to_string(),
            home_prompt_id: "prompt-1".to_string(),
            home_turn_id: "prompt-1".to_string(),
            source_attachment_id: Some("attachment-1".to_string()),
            workspace_live_sync_mode: Some(crate::config::WorkspaceLiveSyncMode::Tracked),
            prompt_origin: Some(PromptOrigin::External),
            external_provider: Some("codex".to_string()),
            external_provider_session_id: Some("codex-thread-1".to_string()),
            external_provider_turn_id: Some("codex-turn-1".to_string()),
            prompt_summary: "edit a file".to_string(),
        }),
        required_mcps: Vec::new(),
        required_skills: Some(vec![crate::transport::relay_peer::RequiredRemoteSkill {
            name: "review".to_string(),
            version_hash: "skill-hash-1".to_string(),
        }]),
        remote_extension_manifest: crate::extension::RemoteExtensionManifest::default(),
        provider_launch_credential: None,
    };
    let snapshot = serde_json::json!([spawn, submit]);
    assert_eq!(
        snapshot.pointer("/0/kind"),
        Some(&serde_json::json!("spawn_leased_agent"))
    );
    assert_eq!(
        snapshot.pointer("/0/workspace_live_sync_mode"),
        Some(&serde_json::json!("tracked"))
    );
    assert_eq!(
        snapshot.pointer("/1/git_context/workspace_live_sync_mode"),
        Some(&serde_json::json!("tracked"))
    );
    assert_eq!(
        snapshot.pointer("/1/git_context/prompt_origin"),
        Some(&serde_json::json!("external"))
    );
    assert_eq!(
        snapshot.pointer("/1/git_context/external_provider"),
        Some(&serde_json::json!("codex"))
    );
    assert_eq!(
        snapshot.pointer("/1/git_context/external_provider_session_id"),
        Some(&serde_json::json!("codex-thread-1"))
    );
    assert_eq!(
        snapshot.pointer("/1/git_context/external_provider_turn_id"),
        Some(&serde_json::json!("codex-turn-1"))
    );
    assert_eq!(
        snapshot.pointer("/1/required_skills/0/name"),
        Some(&serde_json::json!("review"))
    );
    assert_eq!(
        snapshot.pointer("/1/hidden_system_context"),
        Some(&serde_json::json!("scheduled hidden context"))
    );
}

#[test]
fn relay_peer_leased_runtime_projection_provider_run_shape_is_versioned() {
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        39
    );

    let launch_request =
        LaunchProviderRequest::new("worker-session-1", "codex", "codex", "default", "gpt-5.5")
            .with_agent_id("worker-agent-1")
            .with_resume_state(ProviderResumeState::from_codex_thread_id("thread-1"));
    let provider_run = RuntimeProviderRun::new(
        "provider-run-1",
        &launch_request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "codex:serve".to_string(),
            pty_target: None,
            pty_program: Some("codex".to_string()),
            pty_args: vec!["serve".to_string()],
            pty_env: BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: Some("/worker/repo".into()),
            structured_endpoint: Some("http://127.0.0.1:46000".to_string()),
        },
    );
    let event = RelayPeerEvent::LeasedRuntimeProjection {
        home_session_id: "home-session-1".to_string(),
        home_agent_id: "home-agent-1".to_string(),
        provider_run_id: "provider-run-1".to_string(),
        provider_run: Some(provider_run),
        prompts: Vec::new(),
        output_chunks: vec![
            crate::transport::relay_peer::RelayProjectedOutputChunk {
                kind: crate::terminal::TerminalOutputKind::ProviderTerminal,
                merge_key: None,
                bytes: b"\x1b[2Jfullscreen".to_vec(),
            },
            crate::transport::relay_peer::RelayProjectedOutputChunk {
                kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                merge_key: Some("claude-transcript:provider-run-1:assistant".to_string()),
                bytes: b"semantic response".to_vec(),
            },
        ],
        notices: Vec::new(),
        completions: vec![RelayProjectedCompletion {
            message_id: "assistant-msg-1".to_string(),
            completed_at_ms: 1234,
            home_prompt_id: Some("home-prompt-1".to_string()),
        }],
    };
    let mut snapshot =
        serde_json::to_value(event).expect("relay runtime projection should serialize");
    snapshot["provider_run"]["started_at_ms"] = serde_json::json!(1);
    snapshot["provider_run"]["last_activity_at_ms"] = serde_json::json!(1);

    assert_eq!(
        snapshot.pointer("/kind"),
        Some(&serde_json::json!("leased_runtime_projection"))
    );
    assert_eq!(
        snapshot.pointer("/provider_run/provider_session_id"),
        Some(&serde_json::json!("thread-1"))
    );
    assert_eq!(
        snapshot.pointer("/provider_run/resume_state/codex_thread_id"),
        Some(&serde_json::json!("thread-1"))
    );
    assert_eq!(
        snapshot.pointer("/completions/0/home_prompt_id"),
        Some(&serde_json::json!("home-prompt-1"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("leased runtime projection should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "36c410d56cee3f321c11a265221841a8e4a8a10d1c439216760b6e6c42b9dd35"
    );
}

#[test]
fn relay_peer_provider_terminal_resize_shape_is_versioned() {
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        39
    );

    let request = RelayPeerRequest::ResizeLeasedProviderTerminal {
        leased_agent_id: "leased-agent-1".to_string(),
        provider_run_id: "worker-provider-run-1".to_string(),
        cols: 80,
        rows: 24,
    };
    let response = RelayPeerResponse::LeasedProviderTerminalResized {
        provider_run_id: "worker-provider-run-1".to_string(),
        cols: 80,
        rows: 24,
    };
    assert_eq!(
        serde_json::to_value((request, response))
            .expect("terminal resize relay shape should encode"),
        serde_json::json!([
            {
                "kind": "resize_leased_provider_terminal",
                "leased_agent_id": "leased-agent-1",
                "provider_run_id": "worker-provider-run-1",
                "cols": 80,
                "rows": 24
            },
            {
                "kind": "leased_provider_terminal_resized",
                "provider_run_id": "worker-provider-run-1",
                "cols": 80,
                "rows": 24
            }
        ])
    );
}

#[test]
fn relay_peer_leased_agent_profile_update_shape_is_versioned() {
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        39
    );
    let request = RelayPeerRequest::UpdateLeasedAgentProfile {
        leased_agent_id: "leased-agent-1".to_string(),
        provider: "codex".to_string(),
        account_profile: "work".to_string(),
        model: Some("gpt-5.4".to_string()),
        effort: Some("high".to_string()),
    };
    assert_eq!(
        serde_json::to_value(request).expect("leased profile update should encode"),
        serde_json::json!({
            "kind": "update_leased_agent_profile",
            "leased_agent_id": "leased-agent-1",
            "provider": "codex",
            "account_profile": "work",
            "model": "gpt-5.4",
            "effort": "high"
        })
    );
}

#[test]
fn relay_peer_queued_prompt_steer_shape_is_versioned() {
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        39
    );

    let request = RelayPeerRequest::SteerLeasedPrompt {
        leased_agent_id: "leased-agent-1".to_string(),
        steer_id: "home-queued-prompt-2".to_string(),
        target_home_prompt_id: "home-active-prompt-1".to_string(),
        prompt: "steer the active turn".to_string(),
        hidden_system_context: "hidden steering context".to_string(),
        attachments: vec![crate::transport::relay_peer::RelayPromptAttachment {
            url: "file:///tmp/steer.txt".to_string(),
            mime: "text/plain".to_string(),
            filename: Some("steer.txt".to_string()),
            contents_base64: Some("c3RlZXI=".to_string()),
        }],
        required_skills: Some(vec![crate::transport::relay_peer::RequiredRemoteSkill {
            name: "review".to_string(),
            version_hash: "skill-hash-1".to_string(),
        }]),
    };
    let response = RelayPeerResponse::LeasedPromptSteered {
        provider_run_id: "worker-provider-run-1".to_string(),
        steer_id: "home-queued-prompt-2".to_string(),
        replayed: false,
    };
    let snapshot = serde_json::json!([request, response]);
    assert_eq!(
        snapshot.pointer("/0/kind"),
        Some(&serde_json::json!("steer_leased_prompt"))
    );
    assert_eq!(
        snapshot.pointer("/0/target_home_prompt_id"),
        Some(&serde_json::json!("home-active-prompt-1"))
    );
    assert_eq!(
        snapshot.pointer("/1/kind"),
        Some(&serde_json::json!("leased_prompt_steered"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("remote queued prompt steer should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "37d71e28b26468ad2e53f85c139b50156b9c20cbe7b9d6009d7337714b4f54d6"
    );
}

#[test]
fn relay_peer_workspace_live_sync_runtime_tool_shape_is_versioned() {
    assert_eq!(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, 309);

    let context = RemoteWorkspaceLiveSyncContext {
        home_kernel_id: "kernel-home".to_string(),
        home_session_id: "session-1".to_string(),
        home_agent_id: "agent-1".to_string(),
        leased_agent_id: "leased-agent-1".to_string(),
        worker_kernel_id: "kernel-worker".to_string(),
        worker_machine_id: "machine-worker".to_string(),
        worker_provider_run_id: "provider-run-worker".to_string(),
        worker_worktree_path: "/tmp/worker-worktree".to_string(),
        worker_workspace_identity: crate::io::WorkspaceIdentity {
            vcs_provider: Some("git".to_string()),
            repo_id: None,
            repo_url: Some("https://example.test/repo.git".to_string()),
            branch: Some("main".to_string()),
            head_commit: Some("commit-1".to_string()),
            worktree_root_fingerprint: "fingerprint-1".to_string(),
        },
    };
    let arguments = serde_json::json!({
        "path": "src/lib.rs",
        "content_text": "after\n",
        "domain": "text"
    });
    let initial_artifact_states = vec![RemoteWorkspaceLiveSyncArtifactState {
        path: "src/lib.rs".to_string(),
        exists: true,
        domain: Some("text".to_string()),
        content_text: Some("before\n".to_string()),
        content_base64: None,
    }];
    let final_artifact_states = vec![RemoteWorkspaceLiveSyncArtifactState {
        path: "src/lib.rs".to_string(),
        exists: true,
        domain: Some("text".to_string()),
        content_text: Some("after\n".to_string()),
        content_base64: None,
    }];
    let metadata = RemoteWorkspaceLiveSyncInvocationMetadata {
        invocation_id: "workspace-live-sync-invoke-1".to_string(),
        provider_tool_call_id: Some("provider-tool-call-1".to_string()),
        attempt: 1,
        idempotency_key: Some("workspace-live-sync-idempotency-1".to_string()),
    };
    let request = RelayPeerRequest::ForwardWorkspaceLiveSyncRuntimeTool {
        context: context.clone(),
        metadata: metadata.clone(),
        tool_name: "chariox.write_artifact".to_string(),
        arguments: arguments.clone(),
        artifact_states: initial_artifact_states.clone(),
    };
    let response = RelayPeerResponse::WorkspaceLiveSyncRuntimeToolHandled {
        result: crate::transport::runtime_tools::RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "applied": true,
                "path": "src/lib.rs"
            }),
        },
        final_artifact_states: final_artifact_states.clone(),
    };
    let finalize_request = RelayPeerRequest::FinalizeWorkspaceLiveSyncRuntimeTool {
        context,
        metadata,
        tool_name: "chariox.write_artifact".to_string(),
        arguments,
        initial_artifact_states,
        final_artifact_states,
    };
    let finalize_response = RelayPeerResponse::WorkspaceLiveSyncRuntimeToolFinalized;

    let snapshot = serde_json::json!([request, response, finalize_request, finalize_response]);
    assert_eq!(
        snapshot.pointer("/0/kind"),
        Some(&serde_json::json!(
            "forward_workspace_live_sync_runtime_tool"
        ))
    );
    assert_eq!(
        snapshot.pointer("/0/context/home_kernel_id"),
        Some(&serde_json::json!("kernel-home"))
    );
    assert_eq!(
        snapshot.pointer("/0/metadata/invocation_id"),
        Some(&serde_json::json!("workspace-live-sync-invoke-1"))
    );
    assert_eq!(
        snapshot.pointer("/0/artifact_states/0/domain"),
        Some(&serde_json::json!("text"))
    );
    assert_eq!(
        snapshot.pointer("/1/kind"),
        Some(&serde_json::json!(
            "workspace_live_sync_runtime_tool_handled"
        ))
    );
    assert_eq!(
        snapshot.pointer("/1/final_artifact_states/0/content_text"),
        Some(&serde_json::json!("after\n"))
    );
    assert_eq!(
        snapshot.pointer("/2/kind"),
        Some(&serde_json::json!(
            "finalize_workspace_live_sync_runtime_tool"
        ))
    );
    assert_eq!(
        snapshot.pointer("/3/kind"),
        Some(&serde_json::json!(
            "workspace_live_sync_runtime_tool_finalized"
        ))
    );
    let serialized = serde_json::to_string(&snapshot)
        .expect("workspace live sync relay runtime tool should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "8b7b15d322af3d317f09bbc6992600b472f97f4420a5e2483074378a991cb9f6"
    );
}
