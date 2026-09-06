//! Local provider prompt dispatch and abort execution.
//!
//! This module owns writing admitted prompts or cancellation signals to local provider runtimes,
//! including structured prompt I/O and provider-runtime lane spawning.

use super::*;

const CLAUDE_HEADLESS_PROMPT_ACK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const CLAUDE_HEADLESS_PROMPT_ACK_OPERATION: &str = "acknowledge Claude headless prompt";

struct DetachedWorkflowProviderLaunchClaim {
    provider_run_id: String,
    claims: Arc<std::sync::Mutex<BTreeSet<String>>>,
}

impl Drop for DetachedWorkflowProviderLaunchClaim {
    fn drop(&mut self) {
        self.claims
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.provider_run_id);
    }
}

fn claude_native_dispatch_terminal_failure(
    provider_run: &crate::provider::RuntimeProviderRun,
) -> Option<String> {
    crate::app::claude_native_recent_terminal_failure(provider_run)
}

fn claude_headless_dispatch_failure_requires_provider_retirement(
    provider_run: &crate::provider::RuntimeProviderRun,
    error: &DaemonError,
) -> bool {
    if !crate::provider::provider_run_is_claude_headless(provider_run) {
        return false;
    }
    matches!(
        error,
        DaemonError::LocalTransport {
            operation: "submit Claude headless prompt",
            ..
        } | DaemonError::LocalTransport {
            operation: CLAUDE_HEADLESS_PROMPT_ACK_OPERATION,
            ..
        } | DaemonError::ProviderProtocol {
            operation: "submit Claude headless prompt",
            ..
        }
    )
}

fn claude_headless_dispatch_failure_invalidates_resume(error: &DaemonError) -> bool {
    matches!(
        error,
        DaemonError::LocalTransport {
            operation: CLAUDE_HEADLESS_PROMPT_ACK_OPERATION,
            ..
        }
    )
}

impl KernelRuntimeOwnedState {
    fn prompt_dispatch_matches_active_prompt(
        &self,
        dispatch: &crate::app::KernelPromptDispatch,
    ) -> Result<bool, DaemonError> {
        let session = self.session_store.get_session(&dispatch.session_id)?;
        let prompt_is_dispatch_prompt = |prompt: &crate::session::PromptQueueItem| {
            if !prompt.is_chariox_owned() {
                return false;
            }
            if dispatch.steering {
                return dispatch
                    .target_active_prompt_id
                    .as_deref()
                    .is_some_and(|target_prompt_id| target_prompt_id == prompt.id());
            }
            prompt.id() == dispatch.prompt_id
        };
        Ok(self
            .prompt_state_owner
            .active_prompt_for_agent(&session, &dispatch.agent_id)
            .is_some_and(|prompt| prompt_is_dispatch_prompt(&prompt)))
    }

    fn ensure_prompt_dispatch_matches_active_prompt(
        &self,
        dispatch: &crate::app::KernelPromptDispatch,
    ) -> Result<bool, DaemonError> {
        let matches = self.prompt_dispatch_matches_active_prompt(dispatch)?;
        if matches || !dispatch.steering {
            return Ok(matches);
        }
        Err(DaemonError::LocalTransport {
            operation: "steer queued prompt",
            message: "queued prompt steer dispatch no longer matches the active prompt".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::CreateAgentRequest;
    use crate::app::KernelSessionService;
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::config::DaemonConfig;
    use crate::provider::LaunchProviderRequest;
    use crate::session::{
        CreateSessionRequest, PromptQueueItem, PromptStatus, PromptSubmissionOutcome,
    };
    use std::sync::Arc;
    use tokio::sync::Mutex;

    fn claude_native_run_with_context(
        context_file: &std::path::Path,
    ) -> crate::provider::RuntimeProviderRun {
        let request = LaunchProviderRequest::new(
            "session-claude-native",
            "claude",
            "claude",
            "default",
            "haiku",
        );
        crate::provider::RuntimeProviderRun::new(
            "provider-run-claude-native",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "claude-native-test".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: std::collections::BTreeMap::from([(
                    "CHARIOX_CLAUDE_NATIVE_CONTEXT".to_string(),
                    context_file.display().to_string(),
                )]),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        )
    }

    #[test]
    fn claude_native_dispatch_prefers_classified_terminal_failure() {
        let fixture_root = std::env::temp_dir().join(format!(
            "chariox-claude-native-dispatch-failure-{}",
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&fixture_root).expect("fixture root should create");
        std::fs::write(
            fixture_root.join("permission-recent.txt"),
            "--dangerously-skip-permissions cannot be used with root/sudo privileges for security reasons",
        )
        .expect("terminal failure fixture should write");
        let run = claude_native_run_with_context(&fixture_root.join("context.json"));

        let failure = claude_native_dispatch_terminal_failure(&run)
            .expect("terminal failure should be classified");

        assert!(failure.contains("terminal permission error"), "{failure}");
        assert!(
            failure.contains("cannot be used with root/sudo privileges"),
            "{failure}"
        );
        let _ = std::fs::remove_dir_all(fixture_root);
    }

    #[test]
    fn claude_native_dispatch_ignores_interactive_permission_request() {
        let fixture_root = std::env::temp_dir().join(format!(
            "chariox-claude-native-dispatch-permission-{}",
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&fixture_root).expect("fixture root should create");
        std::fs::write(
            fixture_root.join("permission-recent.txt"),
            "Claude wants permission to use Bash",
        )
        .expect("permission fixture should write");
        let run = claude_native_run_with_context(&fixture_root.join("context.json"));

        assert_eq!(claude_native_dispatch_terminal_failure(&run), None);
        let _ = std::fs::remove_dir_all(fixture_root);
    }

    #[test]
    fn claude_native_dispatch_detects_compact_model_credit_dialog() {
        let fixture_root = std::env::temp_dir().join(format!(
            "chariox-claude-native-dispatch-credit-{}",
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&fixture_root).expect("fixture root should create");
        std::fs::write(
            fixture_root.join("permission-recent.txt"),
            "Fable5nowusesusagecredits Youdon'thaveusagecreditsyet",
        )
        .expect("credit dialog fixture should write");
        let run = claude_native_run_with_context(&fixture_root.join("context.json"));

        let failure = claude_native_dispatch_terminal_failure(&run)
            .expect("compact model credit dialog should be classified");

        assert!(failure.contains("resource limit"), "{failure}");
        let _ = std::fs::remove_dir_all(fixture_root);
    }

    #[test]
    fn claude_headless_dispatch_allows_slow_cold_start_acknowledgement() {
        assert!(CLAUDE_HEADLESS_PROMPT_ACK_TIMEOUT >= std::time::Duration::from_secs(30));
    }

    async fn owned_runtime_state(app: &Arc<Mutex<DaemonApp>>) -> KernelRuntimeState {
        let (
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        ) = {
            let app_locked = app.lock().await;
            (
                app_locked.config_projection_store(),
                app_locked.session_state_store(),
                app_locked.agents().clone(),
                app_locked.attachments().clone(),
                app_locked.providers().clone(),
                app_locked.provider_process_tracking_store(),
                app_locked.slices(),
                app_locked.session_state_projection_store(),
                app_locked.provider_run_projection_store(),
                app_locked.operational_history_store(),
                app_locked.durable_state_store(),
                app_locked.prompt_state_owner(),
                app_locked.active_turn_store(),
                app_locked.prompt_activity_store(),
                app_locked.prompt_workspace_claim_store(),
                app_locked.structured_output_record_store(),
                app_locked.terminal_stream_store(),
                app_locked.workflow_design_event_store(),
                app_locked.metaagent_event_store(),
                app_locked.workspace_coordinator(),
            )
        };
        KernelRuntimeState::new_with_owned_state(
            Arc::clone(app),
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        )
    }

    #[tokio::test]
    async fn concurrent_workflow_provider_admission_creates_one_starting_run() {
        const INVOCATION_COUNT: usize = 32;

        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "workspace-concurrent-workflow-provider",
                "worktree-concurrent-workflow-provider",
            ))
            .expect("session should create");
        let workflow_agent = KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_alias("concurrent-workflow-provider"),
            )
            .expect("workflow agent should create");
        let session_id = session.id().to_string();
        let agent_id = workflow_agent.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let barrier = Arc::new(std::sync::Barrier::new(INVOCATION_COUNT));
        let mut handles = Vec::with_capacity(INVOCATION_COUNT);

        for index in 0..INVOCATION_COUNT {
            let owned = runtime.owned.clone();
            let barrier = Arc::clone(&barrier);
            let session_id = session_id.clone();
            let agent_id = agent_id.clone();
            handles.push(
                std::thread::Builder::new()
                    .name(format!("workflow-provider-admission-{index}"))
                    .spawn(move || {
                        barrier.wait();
                        owned
                            .workflow_ensure_provider_run(
                                &session_id,
                                &agent_id,
                                false,
                                false,
                                false,
                                false,
                                None,
                            )
                            .map(|(provider_run_id, _)| provider_run_id)
                    })
                    .expect("workflow provider admission thread should spawn"),
            );
        }

        let run_ids = handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|payload| std::panic::resume_unwind(payload))
                    .expect("concurrent workflow provider admission should succeed")
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(run_ids.len(), 1, "one workflow agent must own one run");
        let run_id = run_ids.iter().next().expect("run id should resolve");
        let run = runtime
            .owned
            .provider_store
            .get_run(run_id)
            .expect("admitted workflow provider should resolve");
        assert_eq!(run.state(), crate::provider::ProviderRunState::Starting);
        assert!(run.workflow_tools_enabled());
        assert_eq!(
            runtime
                .owned
                .provider_store
                .list_runs()
                .into_iter()
                .filter(|candidate| {
                    candidate.session_id() == session_id
                        && candidate.agent_instance_id() == Some(agent_id.as_str())
                        && candidate.state() != crate::provider::ProviderRunState::Ended
                })
                .count(),
            1
        );

        runtime.spawn_detached_workflow_provider_launch(run_id.clone());
        runtime.spawn_detached_workflow_provider_launch(run_id.clone());
        assert_eq!(
            runtime
                .detached_workflow_provider_launches
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            vec![run_id.clone()],
            "duplicate detached launch must not acquire a second lifecycle claim"
        );
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if runtime
                    .owned
                    .provider_store
                    .get_run(run_id)
                    .is_ok_and(|run| run.state() == crate::provider::ProviderRunState::Running)
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the single detached provider launch should finish");
        assert!(runtime
            .detached_workflow_provider_launches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty());
        let tracking = runtime.owned.provider_process_tracking.snapshot();
        assert_eq!(tracking.run_processes.len(), 1);
        assert_eq!(tracking.processes.len(), 1);

        let cleanup_run_id = run_id.clone();
        runtime
            .with_app_side_effect(move |app| {
                crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(&cleanup_run_id)
            })
            .await
            .expect("managed provider process should clean up");
    }

    #[tokio::test]
    async fn workflow_launch_retains_vaulted_environment_until_detached_spawn() {
        let _env = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-workflow-provider-credentials-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("test root should exist");
        std::env::set_var("CHARIOX_HOME", &root);
        std::env::set_var("CHARIOX_TEST_CLAUDE_SETUP_TOKEN", "workflow-setup-token");

        let mut app = DaemonApp::bootstrap(
            DaemonConfig::for_tests().with_session_history_root(root.join("session-history")),
        )
        .expect("daemon should boot");
        let (session, _default_agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                root.to_string_lossy(),
                root.to_string_lossy(),
            ))
            .expect("session should create");
        let profile = app
            .provider_account_profile_registry()
            .create_managed(
                crate::session::DEFAULT_LOCAL_USER_ID,
                "claude",
                "Workflow Claude",
            )
            .expect("managed Claude profile should create");
        let credential_id = crate::provider::provider_account_credential_id(
            crate::session::DEFAULT_LOCAL_USER_ID,
            "claude",
            &profile.profile_id,
        );
        crate::credential::CharioxCredentialRegistry::user()
            .expect("credential registry should resolve")
            .upsert(crate::config::UserCredentialConfig {
                id: credential_id,
                description: None,
                source: crate::config::UserCredentialSourceConfig::Env {
                    name: "CHARIOX_TEST_CLAUDE_SETUP_TOKEN".to_string(),
                },
                allowed_hosts: Vec::new(),
                allowed_uses: vec![crate::config::UserCredentialUse::Provider],
                injection: crate::config::UserCredentialInjectionConfig::Provider,
                metadata: None,
            })
            .expect("provider credential should register");
        let workflow_agent = KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(session.id(), "claude")
                    .with_model("claude-sonnet")
                    .with_account_profile(profile.profile_id),
            )
            .expect("workflow agent should create");

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let (provider_run_id, retired_provider_run_id) = runtime
            .owned
            .workflow_ensure_provider_run(
                session.id(),
                workflow_agent.id(),
                false,
                false,
                false,
                false,
                None,
            )
            .expect("workflow provider should start");
        assert!(retired_provider_run_id.is_none());
        let credential_probe = crate::provider::ProviderCredentialDeliveryProbe::install(
            &provider_run_id,
            &[("CLAUDE_CODE_OAUTH_TOKEN", "workflow-setup-token")],
        );
        let mut dispatches = WorkflowPromptDispatches::default();
        dispatches
            .starting_provider_runs
            .push(provider_run_id.clone());
        runtime.spawn_workflow_prompt_dispatches(dispatches);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if runtime
                    .owned
                    .provider_store
                    .get_run(&provider_run_id)
                    .is_ok_and(|run| run.state() == crate::provider::ProviderRunState::Running)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("detached workflow launch should finish");
        assert!(credential_probe.observed_exactly("pty_spawn"));
        assert!(credential_probe.observed_exactly("runtime_binding"));
        assert!(runtime
            .owned
            .take_pending_provider_launch_credentials(&provider_run_id)
            .is_empty());

        std::env::remove_var("CHARIOX_TEST_CLAUDE_SETUP_TOKEN");
        std::env::remove_var("CHARIOX_HOME");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn workflow_prompt_stays_bound_to_the_replacement_provider_run() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "workspace-workflow-provider-binding",
                "worktree-workflow-provider-binding",
            ))
            .expect("session should create");
        let workflow_agent = KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_alias("workflow-provider-binding"),
            )
            .expect("workflow agent should create");
        let other_agent = KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_alias("other-provider-binding"),
            )
            .expect("other agent should create");

        let old_running_run = app
            .providers()
            .launch_run_detached(
                LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "claude-code",
                    "default",
                    "sonnet",
                )
                .with_agent_id(workflow_agent.id()),
            )
            .expect("old target run should launch");
        app.update_provider_run_projection(old_running_run.clone());
        let other_running_run = app
            .providers()
            .launch_run_detached(
                LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "claude-code",
                    "default",
                    "sonnet",
                )
                .with_agent_id(other_agent.id()),
            )
            .expect("other agent run should own the session pointer");
        app.sessions_mut()
            .set_active_provider_run(session.id(), Some(other_running_run.id().to_string()))
            .expect("other agent run should become the session pointer");
        app.update_provider_run_projection(other_running_run);

        let workflow = app
            .sessions_mut()
            .create_workflow(session.id(), Some("provider-binding".to_string()))
            .expect("workflow should create");
        let node = app
            .sessions_mut()
            .add_workflow_node(session.id(), workflow.id(), workflow_agent.id())
            .expect("workflow node should create");
        let endpoint = app
            .sessions_mut()
            .create_workflow_endpoint(
                session.id(),
                workflow.id(),
                node.id(),
                Some("entry".to_string()),
            )
            .expect("workflow endpoint should create");
        let workflow_run = app
            .sessions_mut()
            .invoke_workflow_endpoint(
                session.id(),
                workflow.id(),
                endpoint.id(),
                Some("review exact head".to_string()),
            )
            .expect("workflow run should create");
        let node_run_id = workflow_run.node_runs()[0].id().to_string();
        app.sessions_mut()
            .prepare_workflow_turn(
                session.id(),
                workflow_run.id(),
                &node_run_id,
                format!("workflow-ack:{node_run_id}"),
                "review exact head".to_string(),
                None,
                None,
            )
            .expect("workflow turn should prepare");

        let session_id = session.id().to_string();
        let agent_id = workflow_agent.id().to_string();
        let run_id = workflow_run.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let dispatches = runtime
            .owned
            .workflow_submit_prepared_prompt(
                crate::app::KernelPreparedPromptSubmission {
                    session_id: session_id.clone(),
                    prompt: PromptQueueItem::new(
                        "pending-workflow-provider-binding",
                        crate::scheduler::runtime::workflow_prompt_source_attachment_id(&run_id),
                        &agent_id,
                        "review exact head",
                        PromptStatus::Queued,
                    )
                    .with_workflow_context(&run_id, &node_run_id),
                    force_queue: false,
                    refresh_projection: true,
                },
                &run_id,
                &node_run_id,
            )
            .expect("workflow prompt should bind to its replacement run");

        assert!(dispatches.local.is_empty());
        assert_eq!(dispatches.starting_provider_runs.len(), 1);
        assert_ne!(dispatches.starting_provider_runs[0], old_running_run.id());
        assert_eq!(
            runtime
                .owned
                .provider_store
                .get_run(old_running_run.id())
                .expect("old target run should remain represented")
                .state(),
            crate::provider::ProviderRunState::Ended
        );
        let replacement = runtime
            .owned
            .provider_store
            .get_run(&dispatches.starting_provider_runs[0])
            .expect("replacement run should resolve");
        assert_eq!(replacement.agent_instance_id(), Some(agent_id.as_str()));
        assert_eq!(
            replacement.state(),
            crate::provider::ProviderRunState::Starting
        );
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should resolve");
        let (active, queued) = runtime
            .owned
            .prompt_state_owner
            .state_parts(&session, &agent_id);
        assert!(active.is_none());
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].workflow_run_id(), Some(run_id.as_str()));
    }

    #[tokio::test]
    async fn failed_launch_settles_all_queued_workflow_nodes_and_advances_once() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "workspace-workflow-launch-failure",
                "worktree-workflow-launch-failure",
            ))
            .expect("session should create");
        let source = KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "attachment-workflow-launch-failure",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("source attachment should attach");
        let workflow_agent = KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_alias("workflow-launch-failure"),
            )
            .expect("workflow agent should create");
        let followup_agent = KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_alias("workflow-launch-followup"),
            )
            .expect("followup agent should create");
        let workflow = app
            .sessions_mut()
            .create_workflow(session.id(), Some("launch-failure".to_string()))
            .expect("workflow should create");
        let node = app
            .sessions_mut()
            .add_workflow_node(session.id(), workflow.id(), workflow_agent.id())
            .expect("workflow node should create");
        let endpoint = app
            .sessions_mut()
            .create_workflow_endpoint(
                session.id(),
                workflow.id(),
                node.id(),
                Some("entry".to_string()),
            )
            .expect("workflow endpoint should create");
        let first_workflow_run = app
            .sessions_mut()
            .invoke_workflow_endpoint(
                session.id(),
                workflow.id(),
                endpoint.id(),
                Some("run first queued workflow node".to_string()),
            )
            .expect("first workflow run should create");
        let first_workflow_node_run_id = first_workflow_run.node_runs()[0].id().to_string();
        app.sessions_mut()
            .prepare_workflow_turn(
                session.id(),
                first_workflow_run.id(),
                &first_workflow_node_run_id,
                format!("workflow-ack:{first_workflow_node_run_id}"),
                "run first queued workflow node".to_string(),
                None,
                None,
            )
            .expect("first workflow turn should prepare");
        let second_workflow_run = app
            .sessions_mut()
            .invoke_workflow_endpoint(
                session.id(),
                workflow.id(),
                endpoint.id(),
                Some("run second queued workflow node".to_string()),
            )
            .expect("second workflow run should create");
        let second_workflow_node_run_id = second_workflow_run.node_runs()[0].id().to_string();
        app.sessions_mut()
            .prepare_workflow_turn(
                session.id(),
                second_workflow_run.id(),
                &second_workflow_node_run_id,
                format!("workflow-ack:{second_workflow_node_run_id}"),
                "run second queued workflow node".to_string(),
                None,
                None,
            )
            .expect("second workflow turn should prepare");

        let followup_workflow = app
            .sessions_mut()
            .create_workflow(session.id(), Some("launch-followup".to_string()))
            .expect("followup workflow should create");
        let followup_node = app
            .sessions_mut()
            .add_workflow_node(session.id(), followup_workflow.id(), followup_agent.id())
            .expect("followup node should create");
        let followup_endpoint = app
            .sessions_mut()
            .create_workflow_endpoint(
                session.id(),
                followup_workflow.id(),
                followup_node.id(),
                Some("entry".to_string()),
            )
            .expect("followup endpoint should create");
        let first_queued_followup = app
            .sessions_mut()
            .enqueue_workflow_prompt(
                session.id(),
                followup_workflow.id(),
                followup_endpoint.id(),
                Some("queued followup one".to_string()),
                None,
                crate::session::WorkflowQueuedPromptSource::Manual,
                None,
            )
            .expect("first followup should queue");
        let second_queued_followup = app
            .sessions_mut()
            .enqueue_workflow_prompt(
                session.id(),
                followup_workflow.id(),
                followup_endpoint.id(),
                Some("queued followup two".to_string()),
                None,
                crate::session::WorkflowQueuedPromptSource::Manual,
                None,
            )
            .expect("second followup should queue");

        let followup_provider_run = app
            .providers()
            .launch_run_detached(
                LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "claude-code",
                    "default",
                    "sonnet",
                )
                .with_agent_id(followup_agent.id()),
            )
            .expect("followup provider should be available without a background launch");
        app.update_provider_run_projection(followup_provider_run);
        let followup_active_prompt_id = app.sessions_mut().reserve_prompt_id();
        let PromptSubmissionOutcome::Started {
            prompt: followup_active_prompt,
        } = app
            .prompt_owner_submit_prepared_prompt(
                session.id(),
                PromptQueueItem::new(
                    followup_active_prompt_id,
                    source.id(),
                    followup_agent.id(),
                    "keep followup agent busy",
                    PromptStatus::Queued,
                ),
                false,
            )
            .expect("followup active prompt should submit")
        else {
            panic!("followup prompt should start");
        };

        let session_id = session.id().to_string();
        let first_workflow_run_id = first_workflow_run.id().to_string();
        let second_workflow_run_id = second_workflow_run.id().to_string();
        let workflow_agent_id = workflow_agent.id().to_string();
        let followup_agent_id = followup_agent.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let first_workflow_prompt = PromptQueueItem::new(
            "pending-workflow-launch-failure-first",
            crate::scheduler::runtime::workflow_prompt_source_attachment_id(&first_workflow_run_id),
            &workflow_agent_id,
            "run first queued workflow node",
            PromptStatus::Queued,
        )
        .with_workflow_context(&first_workflow_run_id, &first_workflow_node_run_id);
        let dispatches = runtime
            .owned
            .workflow_submit_prepared_prompt(
                crate::app::KernelPreparedPromptSubmission {
                    session_id: session_id.clone(),
                    prompt: first_workflow_prompt,
                    force_queue: false,
                    refresh_projection: true,
                },
                &first_workflow_run_id,
                &first_workflow_node_run_id,
            )
            .expect("first workflow prompt should queue while its provider starts");
        assert_eq!(dispatches.starting_provider_runs.len(), 1);
        let provider_run = runtime
            .owned
            .provider_store
            .get_run(&dispatches.starting_provider_runs[0])
            .expect("starting provider run should resolve");
        assert_eq!(
            provider_run.state(),
            crate::provider::ProviderRunState::Starting
        );

        let normal_submission = runtime
            .owned
            .submit_local_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
                session_id: session_id.clone(),
                prompt: PromptQueueItem::new(
                    "pending-normal-launch-failure",
                    source.id(),
                    &workflow_agent_id,
                    "preserve normal queued prompt",
                    PromptStatus::Queued,
                ),
                force_queue: true,
                refresh_projection: true,
            })
            .expect("normal prompt should submit locally")
            .expect("normal prompt should use the local provider");
        let PromptSubmissionOutcome::Queued {
            prompt: normal_prompt,
        } = normal_submission.outcome
        else {
            panic!("normal prompt should remain queued while the provider starts");
        };
        let second_workflow_prompt = PromptQueueItem::new(
            "pending-workflow-launch-failure-second",
            crate::scheduler::runtime::workflow_prompt_source_attachment_id(
                &second_workflow_run_id,
            ),
            &workflow_agent_id,
            "run second queued workflow node",
            PromptStatus::Queued,
        )
        .with_workflow_context(&second_workflow_run_id, &second_workflow_node_run_id);
        let second_dispatches = runtime
            .owned
            .workflow_submit_prepared_prompt(
                crate::app::KernelPreparedPromptSubmission {
                    session_id: session_id.clone(),
                    prompt: second_workflow_prompt,
                    force_queue: false,
                    refresh_projection: true,
                },
                &second_workflow_run_id,
                &second_workflow_node_run_id,
            )
            .expect("second workflow prompt should queue while its provider starts");
        assert_eq!(
            second_dispatches.starting_provider_runs,
            vec![provider_run.id().to_string()]
        );

        let queued_before_failure = runtime
            .owned
            .prompt_state_owner
            .state_parts(
                &runtime
                    .owned
                    .session_store
                    .get_session(&session_id)
                    .expect("session should resolve before launch failure"),
                &workflow_agent_id,
            )
            .1;
        assert_eq!(queued_before_failure.len(), 3);
        let mut queued_workflow_run_ids = queued_before_failure
            .iter()
            .filter_map(|prompt| prompt.workflow_run_id())
            .collect::<Vec<_>>();
        queued_workflow_run_ids.sort_unstable();
        let mut expected_workflow_run_ids = vec![
            first_workflow_run_id.as_str(),
            second_workflow_run_id.as_str(),
        ];
        expected_workflow_run_ids.sort_unstable();
        assert_eq!(queued_workflow_run_ids, expected_workflow_run_ids);
        assert_eq!(
            queued_before_failure
                .iter()
                .filter(|prompt| prompt.id() == normal_prompt.id())
                .count(),
            1
        );

        runtime
            .fail_provider_launch(
                &crate::app::StartedProviderLaunch {
                    run: provider_run,
                    previous_active_run_id: None,
                    provider_credential_env: Default::default(),
                },
                &DaemonError::LocalTransport {
                    operation: "initialize workflow provider runtime",
                    message: "provider health check failed".to_string(),
                },
            )
            .await;

        let failed_session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("failed workflow session should resolve");
        for workflow_run_id in [&first_workflow_run_id, &second_workflow_run_id] {
            let failed_run = runtime
                .owned
                .durable_state_store
                .resolve_workflow_run(
                    failed_session.host_daemon_id(),
                    failed_session.id(),
                    workflow_run_id,
                )
                .expect("failed workflow run lookup should succeed")
                .expect("failed workflow run should resolve from durable history");
            assert_eq!(
                failed_run.status(),
                crate::session::WorkflowRunStatus::Failed
            );
            assert_eq!(
                failed_run.node_runs()[0].status(),
                crate::session::WorkflowNodeRunStatus::Failed
            );
            assert!(failed_run.failure_events().iter().any(|event| {
                event.kind() == crate::session::WorkflowFailureKind::ProviderFailure
            }));
        }

        let (active_prompt, queued_after_failure) = runtime
            .owned
            .prompt_state_owner
            .state_parts(&failed_session, &workflow_agent_id);
        assert!(active_prompt.is_none());
        assert_eq!(queued_after_failure.len(), 1);
        assert_eq!(queued_after_failure[0].id(), normal_prompt.id());
        assert!(queued_after_failure[0].workflow_run_id().is_none());

        let advanced_runs = failed_session
            .workflow_runs()
            .iter()
            .filter(|run| run.invocation_prompt() == Some("queued followup one"))
            .collect::<Vec<_>>();
        assert_eq!(
            advanced_runs.len(),
            1,
            "launch failure should advance exactly one queued workflow invocation"
        );
        assert_eq!(failed_session.workflow_queued_prompts().len(), 1);
        assert_eq!(
            failed_session.workflow_queued_prompts()[0].id(),
            second_queued_followup.id()
        );
        assert_ne!(
            failed_session.workflow_queued_prompts()[0].id(),
            first_queued_followup.id()
        );
        let (followup_active, followup_queued) = runtime
            .owned
            .prompt_state_owner
            .state_parts(&failed_session, &followup_agent_id);
        assert_eq!(
            followup_active.as_ref().map(|prompt| prompt.id()),
            Some(followup_active_prompt.id())
        );
        assert_eq!(
            followup_queued
                .iter()
                .filter_map(|prompt| prompt.workflow_run_id())
                .collect::<Vec<_>>(),
            vec![advanced_runs[0].id()]
        );
    }

    async fn runtime_with_active_prompt(
    ) -> (KernelRuntimeState, String, String, String, String, String) {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "workspace-steering-dispatch",
                "worktree-steering-dispatch",
            ))
            .expect("session should create");
        let attachment = KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "attachment-steering-dispatch",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let provider_run = app
            .launch_provider(
                LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "claude-code",
                    "default",
                    "sonnet",
                )
                .with_agent_id(agent.id()),
            )
            .expect("provider launch should succeed");
        app.update_provider_run_projection(provider_run.clone());
        let prompt = PromptQueueItem::new(
            app.sessions_mut().reserve_prompt_id(),
            attachment.id(),
            agent.id(),
            "active prompt",
            PromptStatus::Queued,
        );
        let PromptSubmissionOutcome::Started { prompt } = app
            .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
            .expect("active prompt should submit")
        else {
            panic!("prompt should start");
        };
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment_id = attachment.id().to_string();
        let active_prompt_id = prompt.id().to_string();
        let provider_run_id = provider_run.id().to_string();
        let app = Arc::new(Mutex::new(app));
        (
            owned_runtime_state(&app).await,
            session_id,
            agent_id,
            attachment_id,
            active_prompt_id,
            provider_run_id,
        )
    }

    async fn runtime_with_admitted_prompt() -> (
        KernelRuntimeState,
        String,
        String,
        String,
        String,
        crate::app::KernelPromptDispatch,
    ) {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "workspace-prompt-dispatch",
                "worktree-prompt-dispatch",
            ))
            .expect("session should create");
        let source = KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "attachment-prompt-source",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("source attachment should attach");
        let observer = KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "attachment-prompt-observer",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("observer attachment should attach");
        let provider_run = app
            .launch_provider(
                LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "claude-code",
                    "default",
                    "sonnet",
                )
                .with_agent_id(agent.id()),
            )
            .expect("provider launch should succeed");
        app.update_provider_run_projection(provider_run.clone());
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let source_id = source.id().to_string();
        let observer_id = observer.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let submission = runtime
            .submit_prepared_prompt(crate::app::KernelPreparedPromptSubmission {
                session_id: session_id.clone(),
                prompt: PromptQueueItem::new(
                    "pending-prompt-dispatch",
                    &source_id,
                    &agent_id,
                    "active prompt",
                    PromptStatus::Queued,
                ),
                force_queue: false,
                refresh_projection: true,
            })
            .await
            .expect("prompt should be admitted");
        let dispatch = submission.dispatch.expect("prompt should require dispatch");
        (
            runtime,
            session_id,
            agent_id,
            observer_id,
            provider_run.id().to_string(),
            dispatch,
        )
    }

    async fn runtime_with_claude_headless_active_prompt() -> (
        KernelRuntimeState,
        String,
        String,
        String,
        crate::provider::RuntimeProviderRun,
        crate::app::KernelPromptDispatch,
    ) {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "workspace-claude-ack-failure",
                "worktree-claude-ack-failure",
            ))
            .expect("session should create");
        let source = KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "attachment-claude-ack-failure",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("source attachment should attach");
        let resume_state =
            crate::provider::ProviderResumeState::from_claude_session_id("claude-session-poisoned");
        let configured_agent = app
            .agents_mut()
            .set_agent_runtime_profile(
                agent.id(),
                "claude-headless",
                Some("claude-opus-5".to_string()),
                None,
                resume_state.clone(),
            )
            .expect("Claude resume state should be configured");
        app.durable_state_store()
            .append_event(
                "agent.runtime_profile_updated",
                Some(configured_agent.id().to_string()),
                serde_json::json!({
                    "agent": &configured_agent,
                    "reason": "test_claude_resume_seeded",
                }),
            )
            .expect("Claude resume state should persist");
        let provider_run = app
            .providers()
            .launch_run_detached(
                LaunchProviderRequest::new(
                    session.id(),
                    "claude",
                    "claude-headless",
                    "default",
                    "claude-opus-5",
                )
                .with_agent_id(agent.id())
                .with_resume_state(resume_state),
            )
            .expect("Claude headless run should launch");
        app.sessions_mut()
            .set_active_provider_run(session.id(), Some(provider_run.id().to_string()))
            .expect("Claude headless run should become active");
        app.update_provider_run_projection(provider_run.clone());
        let process_key = format!("test-process-{}", provider_run.id());
        {
            let tracking_store = app.provider_process_tracking_store();
            let mut tracking = tracking_store.write();
            tracking
                .run_processes
                .insert(provider_run.id().to_string(), process_key.clone());
            tracking.processes.insert(
                process_key,
                crate::app::TrackedProviderProcess {
                    process_id: "managed:claude:test-process".to_string(),
                    pid: None,
                    endpoint_mode: provider_run.endpoint_mode(),
                    process_label: provider_run.process_label().to_string(),
                    started_at_ms: provider_run.started_at_ms(),
                    owner_provider_run_ids: vec![provider_run.id().to_string()],
                },
            );
        }
        let prompt = PromptQueueItem::new(
            app.sessions_mut().reserve_prompt_id(),
            source.id(),
            agent.id(),
            "review exact head",
            PromptStatus::Queued,
        );
        let PromptSubmissionOutcome::Started { prompt } = app
            .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
            .expect("prompt should start")
        else {
            panic!("prompt should be active");
        };
        let prompt_dispatch = dispatch(
            session.id(),
            agent.id(),
            source.id(),
            provider_run.id(),
            prompt.id(),
            prompt.prompt(),
            None,
            false,
        );
        app.mark_active_prompt_delivery(
            session.id(),
            agent.id(),
            prompt.id(),
            crate::session::DurablePromptDeliveryPhase::Dispatching,
            Some(provider_run.id().to_string()),
            Some("claude-session-poisoned".to_string()),
        )
        .expect("dispatch-time Claude resume identity should persist");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let source_id = source.id().to_string();
        let app = Arc::new(Mutex::new(app));
        (
            owned_runtime_state(&app).await,
            session_id,
            agent_id,
            source_id,
            provider_run,
            prompt_dispatch,
        )
    }

    fn dispatch(
        session_id: &str,
        agent_id: &str,
        attachment_id: &str,
        provider_run_id: &str,
        prompt_id: &str,
        prompt: &str,
        target_active_prompt_id: Option<String>,
        steering: bool,
    ) -> crate::app::KernelPromptDispatch {
        crate::app::KernelPromptDispatch {
            session_id: session_id.to_string(),
            provider_run_id: provider_run_id.to_string(),
            agent_id: agent_id.to_string(),
            prompt_id: prompt_id.to_string(),
            target_active_prompt_id,
            source_attachment_id: attachment_id.to_string(),
            prompt: prompt.to_string(),
            hidden_system_context: String::new(),
            attachments: Vec::new(),
            prompt_origin: crate::session::PromptOrigin::Chariox,
            external_provider: None,
            external_provider_session_id: None,
            external_provider_turn_id: None,
            steering,
        }
    }

    #[tokio::test]
    async fn steering_dispatch_matches_target_active_prompt() {
        let (runtime, session_id, agent_id, attachment_id, active_prompt_id, provider_run_id) =
            runtime_with_active_prompt().await;
        let steering_dispatch = dispatch(
            &session_id,
            &agent_id,
            &attachment_id,
            &provider_run_id,
            "queued-steering-prompt",
            "steer now",
            Some(active_prompt_id),
            true,
        );

        assert!(runtime
            .owned
            .prompt_dispatch_matches_active_prompt(&steering_dispatch)
            .expect("dispatch match should evaluate"));
    }

    #[tokio::test]
    async fn local_dispatch_persists_delivered_phase_after_provider_write() {
        let (runtime, session_id, agent_id, attachment_id, active_prompt_id, provider_run_id) =
            runtime_with_active_prompt().await;
        let prompt_dispatch = dispatch(
            &session_id,
            &agent_id,
            &attachment_id,
            &provider_run_id,
            &active_prompt_id,
            "active prompt",
            None,
            false,
        );

        runtime
            .enqueue_prompt_dispatch(&prompt_dispatch)
            .await
            .expect("prompt should reach the provider");
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should load");
        let prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .expect("active prompt should remain");

        assert_eq!(
            prompt.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Delivered)
        );
        assert_eq!(
            prompt.durable_delivery_provider_run_id(),
            Some(provider_run_id.as_str())
        );
    }

    #[tokio::test]
    async fn dispatch_match_uses_prompt_owner_when_session_mirror_is_stale() {
        let (runtime, session_id, agent_id, attachment_id, active_prompt_id, provider_run_id) =
            runtime_with_active_prompt().await;
        runtime
            .owned
            .session_store
            .mirror_agent_prompt_state(
                &session_id,
                &agent_id,
                None,
                std::collections::VecDeque::new(),
            )
            .expect("test drift should clear stale session prompt mirror");
        assert!(
            runtime
                .owned
                .session_store
                .get_session(&session_id)
                .expect("session should load")
                .active_prompt_for_agent(&agent_id)
                .is_none(),
            "session mirror should not expose the active prompt"
        );
        let steering_dispatch = dispatch(
            &session_id,
            &agent_id,
            &attachment_id,
            &provider_run_id,
            "queued-steering-prompt",
            "steer now",
            Some(active_prompt_id),
            true,
        );

        assert!(runtime
            .owned
            .prompt_dispatch_matches_active_prompt(&steering_dispatch)
            .expect("dispatch match should use prompt owner"));
    }

    #[tokio::test]
    async fn stale_steering_dispatch_is_rejected() {
        let (runtime, session_id, agent_id, attachment_id, _, provider_run_id) =
            runtime_with_active_prompt().await;
        let steering_dispatch = dispatch(
            &session_id,
            &agent_id,
            &attachment_id,
            &provider_run_id,
            "queued-steering-prompt",
            "steer now",
            Some("stale-active-prompt".to_string()),
            true,
        );

        let error = runtime
            .owned
            .ensure_prompt_dispatch_matches_active_prompt(&steering_dispatch)
            .expect_err("stale steering dispatch should fail");
        assert!(
            error
                .to_string()
                .contains("no longer matches the active prompt"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn steering_dispatch_records_provider_input() {
        let (runtime, session_id, agent_id, attachment_id, active_prompt_id, provider_run_id) =
            runtime_with_active_prompt().await;
        let steering_text = "STEERING_DELIVERY_PROOF";
        let steering_dispatch = dispatch(
            &session_id,
            &agent_id,
            &attachment_id,
            &provider_run_id,
            "queued-steering-prompt",
            steering_text,
            Some(active_prompt_id),
            true,
        );

        runtime
            .enqueue_prompt_dispatch(&steering_dispatch)
            .await
            .expect("steering dispatch should deliver");

        let input_records = runtime.owned.terminal_stream.input_records();
        assert!(
            input_records.iter().any(|record| {
                record.provider_run_id == provider_run_id
                    && String::from_utf8_lossy(&record.bytes).contains(steering_text)
            }),
            "steering prompt should be recorded as provider input: {input_records:?}"
        );
    }

    #[tokio::test]
    async fn local_prompt_is_echoed_once_across_admission_and_dispatch() {
        let (runtime, session_id, _, observer_id, _, dispatch) =
            runtime_with_admitted_prompt().await;
        let prompt_id = dispatch.prompt_id.clone();

        runtime
            .enqueue_prompt_dispatch(&dispatch)
            .await
            .expect("prompt should reach the provider");

        let prompt_echoes = runtime
            .owned
            .terminal_stream
            .drain_output_records(&session_id, &observer_id)
            .into_iter()
            .filter(|record| {
                record.kind == crate::terminal::TerminalOutputKind::PromptEcho
                    && record.prompt_id.as_deref() == Some(prompt_id.as_str())
            })
            .count();
        assert_eq!(
            prompt_echoes, 1,
            "one admitted prompt must have one prompt echo"
        );
    }

    #[tokio::test]
    async fn local_prompt_admission_clears_prior_agent_error() {
        let (runtime, session_id, agent_id, _, provider_run_id, first_dispatch) =
            runtime_with_admitted_prompt().await;
        runtime
            .owned
            .complete_local_prompt_without_advance(&session_id, &agent_id, Some(&provider_run_id))
            .expect("first prompt should settle");
        runtime
            .owned
            .agent_store
            .set_agent_state(&agent_id, crate::agent::AgentState::Error)
            .expect("error state should seed");

        let submission = runtime
            .submit_prepared_prompt(crate::app::KernelPreparedPromptSubmission {
                session_id: session_id.clone(),
                prompt: PromptQueueItem::new(
                    "pending-recovery-prompt",
                    &first_dispatch.source_attachment_id,
                    &agent_id,
                    "recover after provider failure",
                    PromptStatus::Queued,
                ),
                force_queue: false,
                refresh_projection: true,
            })
            .await
            .expect("recovery prompt should be admitted");

        assert!(matches!(
            submission.outcome,
            PromptSubmissionOutcome::Started { .. }
        ));
        assert_ne!(
            runtime
                .owned
                .agent_store
                .get_agent(&agent_id)
                .expect("agent should remain available")
                .state(),
            crate::agent::AgentState::Error,
        );
    }

    #[tokio::test]
    async fn failed_local_dispatch_emits_completion_and_marks_agent_error() {
        let (runtime, session_id, agent_id, observer_id, provider_run_id, dispatch) =
            runtime_with_admitted_prompt().await;

        runtime
            .fail_prompt_dispatch(
                dispatch,
                DaemonError::LocalTransport {
                    operation: "test prompt dispatch",
                    message: "rejected".to_string(),
                },
            )
            .await
            .expect_err("dispatch failure should be returned");

        let session = runtime
            .owned
            .session_snapshot(&session_id)
            .expect("settled session should project");
        assert!(runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .is_none());
        let agent = session
            .agents()
            .iter()
            .find(|agent| agent.id() == agent_id)
            .expect("agent should remain in session");
        assert!(!agent.is_processing());
        assert_eq!(agent.state(), crate::agent::AgentState::Error);

        let provider_errors = runtime
            .owned
            .terminal_stream
            .drain_output_records(&session_id, &observer_id)
            .into_iter()
            .filter(|record| record.kind == crate::terminal::TerminalOutputKind::ProviderError)
            .collect::<Vec<_>>();
        assert_eq!(provider_errors.len(), 1);
        assert!(String::from_utf8_lossy(&provider_errors[0].bytes)
            .contains("Provider prompt dispatch failed"));

        let completions = runtime
            .owned
            .terminal_stream
            .drain_completion_records(&session_id, &observer_id);
        assert_eq!(
            completions.len(),
            1,
            "failed dispatch must emit one completion"
        );
        assert_eq!(completions[0].provider_run_id, provider_run_id);
        assert_eq!(completions[0].agent_id.as_deref(), Some(agent_id.as_str()));
        assert_eq!(
            runtime
                .owned
                .provider_store
                .get_run(&provider_run_id)
                .expect("unrelated dispatch failure should retain provider run")
                .state(),
            crate::provider::ProviderRunState::Running,
        );
    }

    #[tokio::test]
    async fn claude_headless_ack_failure_retires_poisoned_provider_run() {
        let (runtime, session_id, agent_id, source_id, provider_run, dispatch) =
            runtime_with_claude_headless_active_prompt().await;
        let PromptSubmissionOutcome::Queued {
            prompt: queued_prompt,
        } = runtime
            .submit_prepared_prompt(crate::app::KernelPreparedPromptSubmission {
                session_id: session_id.clone(),
                prompt: PromptQueueItem::new(
                    "queued-after-poisoned-provider",
                    &source_id,
                    &agent_id,
                    "continue review on a fresh provider",
                    PromptStatus::Queued,
                ),
                force_queue: false,
                refresh_projection: true,
            })
            .await
            .expect("replacement prompt should queue")
            .outcome
        else {
            panic!("replacement prompt should remain queued until failure settlement");
        };

        runtime
            .fail_prompt_dispatch(
                dispatch,
                DaemonError::LocalTransport {
                    operation: CLAUDE_HEADLESS_PROMPT_ACK_OPERATION,
                    message: "provider did not acknowledge prompt after PTY injection".to_string(),
                },
            )
            .await
            .expect_err("acknowledgement failure should remain observable");

        assert_eq!(
            runtime
                .owned
                .provider_store
                .get_run(provider_run.id())
                .expect("poisoned provider run should remain represented")
                .state(),
            crate::provider::ProviderRunState::Ended,
        );
        let replacement_run = runtime
            .owned
            .provider_store
            .get_run_for_agent(&session_id, &agent_id)
            .expect("queued prompt should launch a replacement provider");
        assert_ne!(replacement_run.id(), provider_run.id());
        assert_eq!(
            replacement_run.state(),
            crate::provider::ProviderRunState::Running
        );
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should resolve");
        assert_eq!(session.active_provider_run_id(), Some(replacement_run.id()));
        let active_prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .expect("queued prompt should become active");
        assert_eq!(active_prompt.prompt(), queued_prompt.prompt());
        let agent = runtime
            .owned
            .agent_store
            .get_agent(&agent_id)
            .expect("agent should remain available");
        assert_eq!(
            agent.provider_resume_state().claude_session_id(),
            None,
            "the replacement must not reuse an unresponsive Claude resume session",
        );
        assert_eq!(
            replacement_run.resume_state().claude_session_id(),
            None,
            "the queued replacement must launch without the poisoned Claude session",
        );
        let tracking = runtime.owned.provider_process_tracking.snapshot();
        assert!(!tracking.run_processes.contains_key(provider_run.id()));
        assert!(tracking.processes.values().all(|process| !process
            .owner_provider_run_ids
            .iter()
            .any(|id| id == provider_run.id())));
    }

    #[tokio::test]
    async fn claude_headless_ack_failure_intent_finishes_resume_clear_after_restart() {
        let (runtime, session_id, agent_id, _, provider_run, dispatch) =
            runtime_with_claude_headless_active_prompt().await;
        let durable_path = runtime.owned.durable_state_store.path().to_path_buf();
        let connection = rusqlite::Connection::open(&durable_path)
            .expect("durable database should open for failure injection");
        connection
            .execute_batch(
                "CREATE TRIGGER fail_claude_resume_clear
                 BEFORE INSERT ON durable_state_events
                 WHEN NEW.kind = 'agent.runtime_profile_updated'
                 BEGIN
                   SELECT RAISE(FAIL, 'injected Claude resume clear persistence failure');
                 END;",
            )
            .expect("failure trigger should install");

        let result = runtime
            .fail_prompt_dispatch(
                dispatch,
                DaemonError::LocalTransport {
                    operation: CLAUDE_HEADLESS_PROMPT_ACK_OPERATION,
                    message: "provider did not acknowledge prompt after PTY injection".to_string(),
                },
            )
            .await;

        connection
            .execute_batch("DROP TRIGGER fail_claude_resume_clear;")
            .expect("failure trigger should be removed");
        let error = result.expect_err("durable failure must stop replacement admission");
        assert!(
            error
                .to_string()
                .contains("injected Claude resume clear persistence failure"),
            "{error}"
        );
        assert_eq!(
            runtime
                .owned
                .agent_store
                .get_agent(&agent_id)
                .expect("agent should remain available")
                .provider_resume_state()
                .claude_session_id(),
            Some("claude-session-poisoned"),
            "failed persistence must roll the in-memory resume clear back",
        );
        let durable_events = runtime
            .owned
            .durable_state_store
            .load_subject_events(&agent_id, 20)
            .expect("durable agent state should remain readable");
        assert_eq!(
            durable_events
                .last()
                .and_then(|event| event
                    .payload
                    .pointer("/agent/provider_resume_state/claude_session_id"))
                .and_then(serde_json::Value::as_str),
            Some("claude-session-poisoned"),
            "failed persistence must leave the prior durable resume state authoritative",
        );
        assert_eq!(
            runtime
                .owned
                .provider_store
                .get_run(provider_run.id())
                .expect("provider run should remain available")
                .state(),
            crate::provider::ProviderRunState::Running,
            "replacement admission must not retire the provider after a non-durable clear",
        );
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should remain available");
        let prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .expect("the durable failure intent should retain the active prompt");
        assert_eq!(prompt.status(), crate::session::PromptStatus::Cancelling);
        assert!(
            prompt.durable_delivery_failure_pending(),
            "restart must be able to identify the exact failed delivery",
        );

        runtime
            .finalize_cancelled_local_prompt_after_restart(&session_id, &agent_id, &prompt)
            .await
            .expect("restart recovery should finish the durable failure intent");
        assert_eq!(
            runtime
                .owned
                .agent_store
                .get_agent(&agent_id)
                .expect("agent should remain available")
                .provider_resume_state()
                .claude_session_id(),
            None,
            "restart recovery must clear the poisoned resume before advancing",
        );
        assert_eq!(
            runtime
                .owned
                .provider_store
                .get_run(provider_run.id())
                .expect("failed provider run should remain auditable")
                .state(),
            crate::provider::ProviderRunState::Ended,
            "restart recovery must retire the poisoned provider run",
        );
    }

    #[tokio::test]
    async fn claude_headless_ack_failure_does_not_clear_resume_without_durable_intent() {
        let (runtime, session_id, agent_id, _, provider_run, dispatch) =
            runtime_with_claude_headless_active_prompt().await;
        let durable_path = runtime.owned.durable_state_store.path().to_path_buf();
        let connection = rusqlite::Connection::open(&durable_path)
            .expect("durable database should open for failure injection");
        connection
            .execute_batch(
                "CREATE TRIGGER fail_claude_prompt_cancellation
                 BEFORE INSERT ON durable_state_events
                 WHEN NEW.kind = 'session.prompt_state.updated'
                 BEGIN
                   SELECT RAISE(FAIL, 'injected prompt cancellation persistence failure');
                 END;",
            )
            .expect("failure trigger should install");

        let result = runtime
            .fail_prompt_dispatch(
                dispatch,
                DaemonError::LocalTransport {
                    operation: CLAUDE_HEADLESS_PROMPT_ACK_OPERATION,
                    message: "provider did not acknowledge prompt after PTY injection".to_string(),
                },
            )
            .await;

        connection
            .execute_batch("DROP TRIGGER fail_claude_prompt_cancellation;")
            .expect("failure trigger should be removed");
        let error = result.expect_err("non-durable cancellation must stop provider retirement");
        assert!(
            error
                .to_string()
                .contains("injected prompt cancellation persistence failure"),
            "{error}"
        );
        assert_eq!(
            runtime
                .owned
                .agent_store
                .get_agent(&agent_id)
                .expect("agent should remain available")
                .provider_resume_state()
                .claude_session_id(),
            Some("claude-session-poisoned"),
            "resume invalidation must not begin before the failure intent is durable",
        );
        let durable_agent_events = runtime
            .owned
            .durable_state_store
            .load_subject_events(&agent_id, 20)
            .expect("durable agent state should remain readable");
        assert_eq!(
            durable_agent_events
                .last()
                .and_then(|event| {
                    event
                        .payload
                        .pointer("/agent/provider_resume_state/claude_session_id")
                })
                .and_then(serde_json::Value::as_str),
            Some("claude-session-poisoned"),
            "the failed intent write must leave the prior durable resume authoritative",
        );
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should remain available");
        let prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .expect("failed cancellation should leave the prompt retryable");
        assert_ne!(prompt.status(), crate::session::PromptStatus::Cancelling);
        assert_eq!(
            prompt.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Dispatching),
        );
        assert_eq!(
            runtime
                .owned
                .provider_store
                .get_run(provider_run.id())
                .expect("provider should not retire before cancellation is durable")
                .state(),
            crate::provider::ProviderRunState::Running,
        );
        let durable_prompt_events = runtime
            .owned
            .durable_state_store
            .load_subject_events(&session_id, 20)
            .expect("durable prompt state should remain readable");
        assert_eq!(
            durable_prompt_events
                .iter()
                .rev()
                .find(|event| {
                    event.kind == crate::durable_prompt_state::DURABLE_PROMPT_STATE_EVENT_KIND
                })
                .and_then(|event| event.payload.pointer("/private_states/0/delivery_phase"))
                .and_then(serde_json::Value::as_str),
            Some("dispatching"),
            "restart must recover a dispatching prompt with the poisoned resume already cleared",
        );
    }

    #[tokio::test]
    async fn claude_headless_late_resume_update_wins_before_delivery_phase_commit() {
        let (runtime, session_id, agent_id, _, provider_run, dispatch) =
            runtime_with_claude_headless_active_prompt().await;
        let current_resume_state =
            crate::provider::ProviderResumeState::from_claude_session_id("claude-session-current");
        runtime
            .owned
            .provider_store
            .apply_prompt_submit_acknowledgement(
                provider_run.id(),
                &crate::provider::ProviderPromptSubmitAcknowledgement {
                    resume_state: current_resume_state.clone(),
                },
            )
            .expect("late provider acknowledgement should update the run");
        runtime
            .owned
            .agent_store
            .set_agent_runtime_profile_durably(
                &runtime.owned.durable_state_store,
                &agent_id,
                "claude-headless",
                Some("claude-opus-5".to_string()),
                None,
                None,
                current_resume_state,
                Some(provider_run.id()),
                Some("prompt_delivery_acknowledged"),
            )
            .expect("late provider acknowledgement should persist atomically");

        runtime
            .fail_prompt_dispatch(
                dispatch,
                DaemonError::LocalTransport {
                    operation: CLAUDE_HEADLESS_PROMPT_ACK_OPERATION,
                    message: "provider did not acknowledge prompt after PTY injection".to_string(),
                },
            )
            .await
            .expect_err("the stale timeout should remain observable");

        assert_eq!(
            runtime
                .owned
                .provider_store
                .get_run(provider_run.id())
                .expect("current provider should remain available")
                .state(),
            crate::provider::ProviderRunState::Running,
        );
        assert_eq!(
            runtime
                .owned
                .agent_store
                .get_agent(&agent_id)
                .expect("current agent should remain available")
                .provider_resume_state()
                .claude_session_id(),
            Some("claude-session-current"),
        );
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should remain available");
        let prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .expect("the same active prompt must survive a superseded timeout");
        assert_eq!(prompt.status(), crate::session::PromptStatus::Dispatching);
        assert_eq!(
            prompt.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Dispatching),
        );
        assert_eq!(
            prompt.durable_delivery_provider_session_id(),
            Some("claude-session-current"),
        );
        assert!(!prompt.durable_delivery_failure_pending());
        let durable_events = runtime
            .owned
            .durable_state_store
            .load_subject_events(&agent_id, 20)
            .expect("durable agent state should load");
        assert_eq!(
            durable_events
                .last()
                .and_then(|event| event
                    .payload
                    .pointer("/agent/provider_resume_state/claude_session_id"))
                .and_then(serde_json::Value::as_str),
            Some("claude-session-current"),
            "memory and the last durable event must agree after the late acknowledgement",
        );
    }

    #[tokio::test]
    async fn claude_headless_delivered_phase_wins_over_late_timeout() {
        let (runtime, session_id, agent_id, _, provider_run, dispatch) =
            runtime_with_claude_headless_active_prompt().await;
        runtime
            .owned
            .mark_active_prompt_delivery(
                &session_id,
                &agent_id,
                &dispatch.prompt_id,
                crate::session::DurablePromptDeliveryPhase::Delivered,
                Some(provider_run.id().to_string()),
                Some("claude-session-poisoned".to_string()),
            )
            .expect("late delivery acknowledgement should commit");

        runtime
            .fail_prompt_dispatch(
                dispatch,
                DaemonError::LocalTransport {
                    operation: CLAUDE_HEADLESS_PROMPT_ACK_OPERATION,
                    message: "provider did not acknowledge prompt after PTY injection".to_string(),
                },
            )
            .await
            .expect_err("the stale timeout should remain observable");

        assert_eq!(
            runtime
                .owned
                .provider_store
                .get_run(provider_run.id())
                .expect("delivered provider should remain available")
                .state(),
            crate::provider::ProviderRunState::Running,
        );
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should remain available");
        assert_eq!(
            runtime
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, &agent_id)
                .expect("delivered prompt should remain active")
                .durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Delivered),
        );
    }

    #[tokio::test]
    async fn claude_headless_delivery_settlement_claim_blocks_timeout_retirement() {
        let (runtime, session_id, agent_id, _, provider_run, dispatch) =
            runtime_with_claude_headless_active_prompt().await;
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should remain available");
        let acknowledgement_claim = runtime
            .owned
            .prompt_state_owner
            .try_claim_active_prompt_delivery_settlement(
                &session,
                &agent_id,
                &dispatch.prompt_id,
                provider_run.id(),
            )
            .expect("acknowledgement should claim the dispatch settlement");
        let prompt_id = dispatch.prompt_id.clone();

        runtime
            .fail_prompt_dispatch(
                dispatch,
                DaemonError::LocalTransport {
                    operation: CLAUDE_HEADLESS_PROMPT_ACK_OPERATION,
                    message: "provider did not acknowledge prompt after PTY injection".to_string(),
                },
            )
            .await
            .expect_err("the losing timeout should remain observable");

        runtime
            .owned
            .mark_active_prompt_delivery(
                &session_id,
                &agent_id,
                &prompt_id,
                crate::session::DurablePromptDeliveryPhase::Delivered,
                Some(provider_run.id().to_string()),
                Some("claude-session-poisoned".to_string()),
            )
            .expect("the acknowledgement claim owner should commit delivery");
        drop(acknowledgement_claim);

        assert_eq!(
            runtime
                .owned
                .provider_store
                .get_run(provider_run.id())
                .expect("acknowledged provider should remain available")
                .state(),
            crate::provider::ProviderRunState::Running,
        );
        assert_eq!(
            runtime
                .owned
                .agent_store
                .get_agent(&agent_id)
                .expect("acknowledged agent should remain available")
                .provider_resume_state()
                .claude_session_id(),
            Some("claude-session-poisoned"),
        );
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should remain available");
        assert_eq!(
            runtime
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, &agent_id)
                .expect("acknowledged prompt should remain active")
                .durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Delivered),
        );
    }

    #[tokio::test]
    async fn stale_claude_headless_ack_failure_preserves_replacement_prompt_and_provider() {
        let (runtime, session_id, agent_id, source_id, provider_run, stale_dispatch) =
            runtime_with_claude_headless_active_prompt().await;
        runtime
            .owned
            .complete_local_prompt_without_advance(&session_id, &agent_id, Some(provider_run.id()))
            .expect("original prompt should settle");
        let replacement = runtime
            .submit_prepared_prompt(crate::app::KernelPreparedPromptSubmission {
                session_id: session_id.clone(),
                prompt: PromptQueueItem::new(
                    "replacement-after-stale-claude-dispatch",
                    &source_id,
                    &agent_id,
                    "replacement prompt",
                    PromptStatus::Queued,
                ),
                force_queue: false,
                refresh_projection: true,
            })
            .await
            .expect("replacement prompt should be admitted");
        let PromptSubmissionOutcome::Started { prompt } = replacement.outcome else {
            panic!("replacement prompt should start");
        };
        let replacement_resume_state =
            crate::provider::ProviderResumeState::from_claude_session_id("claude-session-current");
        runtime
            .owned
            .provider_store
            .apply_prompt_submit_acknowledgement(
                provider_run.id(),
                &crate::provider::ProviderPromptSubmitAcknowledgement {
                    resume_state: replacement_resume_state.clone(),
                },
            )
            .expect("replacement provider resume state should update");
        runtime
            .owned
            .agent_store
            .set_agent_runtime_profile_durably(
                &runtime.owned.durable_state_store,
                &agent_id,
                "claude-headless",
                Some("claude-opus-5".to_string()),
                None,
                None,
                replacement_resume_state,
                Some(provider_run.id()),
                Some("prompt_delivery_acknowledged"),
            )
            .expect("replacement resume state should persist atomically");

        runtime
            .fail_prompt_dispatch(
                stale_dispatch,
                DaemonError::LocalTransport {
                    operation: CLAUDE_HEADLESS_PROMPT_ACK_OPERATION,
                    message: "provider did not acknowledge prompt after PTY injection".to_string(),
                },
            )
            .await
            .expect_err("stale failure should remain observable");

        assert_eq!(
            runtime
                .owned
                .provider_store
                .get_run(provider_run.id())
                .expect("replacement prompt provider should remain available")
                .state(),
            crate::provider::ProviderRunState::Running,
        );
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should remain available");
        let active = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .expect("replacement prompt should remain active");
        assert_eq!(active.id(), prompt.id());
        let current_run = runtime
            .owned
            .provider_store
            .get_run(provider_run.id())
            .expect("replacement prompt provider should remain available");
        assert_eq!(
            current_run.resume_state().claude_session_id(),
            Some("claude-session-current"),
        );
        let current_agent = runtime
            .owned
            .agent_store
            .get_agent(&agent_id)
            .expect("replacement agent should remain available");
        assert_eq!(
            current_agent.provider_resume_state().claude_session_id(),
            Some("claude-session-current"),
        );
        let durable_events = runtime
            .owned
            .durable_state_store
            .load_subject_events(&agent_id, 20)
            .expect("replacement durable state should load");
        assert_eq!(
            durable_events
                .last()
                .and_then(|event| event
                    .payload
                    .pointer("/agent/provider_resume_state/claude_session_id"))
                .and_then(serde_json::Value::as_str),
            Some("claude-session-current"),
            "a stale acknowledgement failure must not replace the durable current session",
        );
        assert!(runtime
            .owned
            .provider_process_tracking
            .snapshot()
            .run_processes
            .contains_key(provider_run.id()));
    }

    #[tokio::test]
    async fn stale_dispatch_failure_does_not_settle_the_current_prompt() {
        let (runtime, session_id, agent_id, observer_id, provider_run_id, dispatch) =
            runtime_with_admitted_prompt().await;

        let settlement = runtime
            .owned
            .settle_failed_local_prompt_without_advance(
                &session_id,
                &agent_id,
                "stale-prompt-id",
                &provider_run_id,
                "late dispatch failure",
            )
            .expect("stale failure should be ignored");
        assert!(settlement.is_none());

        let session = runtime
            .owned
            .session_snapshot(&session_id)
            .expect("current session should project");
        let active_prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .expect("current prompt must remain active");
        assert_eq!(active_prompt.id(), dispatch.prompt_id);
        assert!(runtime
            .owned
            .terminal_stream
            .drain_completion_records(&session_id, &observer_id)
            .is_empty());
    }

    #[tokio::test]
    async fn stale_dispatch_failure_does_not_cancel_replacement_prompt() {
        let (runtime, session_id, agent_id, _observer_id, provider_run_id, stale_dispatch) =
            runtime_with_admitted_prompt().await;

        runtime
            .owned
            .complete_local_prompt_without_advance(&session_id, &agent_id, Some(&provider_run_id))
            .expect("the original prompt should settle");
        let replacement = runtime
            .submit_prepared_prompt(crate::app::KernelPreparedPromptSubmission {
                session_id: session_id.clone(),
                prompt: PromptQueueItem::new(
                    "replacement-prompt",
                    &stale_dispatch.source_attachment_id,
                    &agent_id,
                    "replacement prompt",
                    PromptStatus::Queued,
                ),
                force_queue: false,
                refresh_projection: true,
            })
            .await
            .expect("replacement prompt should be admitted");
        let PromptSubmissionOutcome::Started { prompt } = replacement.outcome else {
            panic!("replacement prompt should start");
        };

        runtime
            .fail_prompt_dispatch(
                stale_dispatch,
                DaemonError::LocalTransport {
                    operation: "stale dispatch",
                    message: "late provider failure".to_string(),
                },
            )
            .await
            .expect_err("the original dispatch failure remains observable");

        let session = runtime
            .owned
            .session_snapshot(&session_id)
            .expect("session should remain available");
        let active = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .expect("replacement prompt should remain active");
        assert_eq!(active.id(), prompt.id());
    }
}

impl KernelRuntimeState {
    pub(super) async fn enqueue_prompt_dispatch(
        &self,
        dispatch: &crate::app::KernelPromptDispatch,
    ) -> Result<(), DaemonError> {
        {
            let owned = &self.owned;
            if !owned.ensure_prompt_dispatch_matches_active_prompt(dispatch)? {
                return Ok(());
            }
            let has_managed_process = owned
                .provider_process_tracking
                .read()
                .run_processes
                .contains_key(&dispatch.provider_run_id);
            if has_managed_process {
                let recipients = owned
                    .attachment_store
                    .list_session_attachment_ids(&dispatch.session_id);
                let _ = self
                    .pump_owned_provider_output(
                        &dispatch.session_id,
                        &dispatch.provider_run_id,
                        recipients,
                        false,
                    )
                    .await?;
                if !owned.ensure_prompt_dispatch_matches_active_prompt(dispatch)? {
                    return Ok(());
                }
            }
            let result = self
                .enqueue_prompt_dispatch_after_liveness(dispatch, owned)
                .await;
            if result.is_ok() {
                owned.update_metaagent_event_prompt_delivery_for_prompt(
                    &dispatch.prompt_id,
                    crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Delivered,
                    None,
                );
            }
            result
        }
    }

    pub(super) async fn enqueue_prompt_dispatch_after_liveness(
        &self,
        dispatch: &crate::app::KernelPromptDispatch,
        owned: &KernelRuntimeOwnedState,
    ) -> Result<(), DaemonError> {
        if !owned.ensure_prompt_dispatch_matches_active_prompt(dispatch)? {
            return Ok(());
        }
        if !dispatch.steering {
            let provider_run = owned
                .ensure_provider_run_in_session(&dispatch.session_id, &dispatch.provider_run_id)?;
            owned.mark_active_prompt_delivery(
                &dispatch.session_id,
                &dispatch.agent_id,
                &dispatch.prompt_id,
                crate::session::DurablePromptDeliveryPhase::Dispatching,
                Some(dispatch.provider_run_id.clone()),
                provider_run.provider_session_id().map(str::to_string),
            )?;
        }
        let internal_recovery =
            is_internal_recovery_prompt_attachment(&dispatch.source_attachment_id);
        let provider_run = owned
            .ensure_provider_run_in_session(&dispatch.session_id, &dispatch.provider_run_id)?;
        if provider_run.state() != crate::provider::ProviderRunState::Running {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: dispatch.provider_run_id.clone(),
                state: provider_run.state(),
                operation: "submit prompt",
            });
        }
        if owned
            .provider_store
            .run_uses_structured_prompt_io(&provider_run)
        {
            if !internal_recovery {
                self.observe_git_before_prompt_dispatch(dispatch, &provider_run)
                    .await;
            }
            if !dispatch.steering {
                owned.note_prompt_started(&dispatch.provider_run_id);
            }
            let prompt_with_handoff = owned.prompt_with_pending_context_handoff(
                &dispatch.session_id,
                &dispatch.agent_id,
                &dispatch.source_attachment_id,
                &provider_run,
                &dispatch.prompt,
            );
            let granted_skill_context = owned.granted_skill_hidden_context(
                &dispatch.session_id,
                &dispatch.agent_id,
                &prompt_with_handoff,
            )?;
            let hidden_system_context =
                join_hidden_context(&dispatch.hidden_system_context, &granted_skill_context);
            let (source_client_id, _source_user_id) =
                owned.active_prompt_source_attribution(&dispatch.session_id, &dispatch.agent_id)?;
            let mode = crate::prompt_assembly::provider_turn_mode_for_prompt(
                &dispatch.agent_id,
                owned
                    .agent_store
                    .get_agent(&dispatch.agent_id)?
                    .is_metaagent(),
                source_client_id.as_deref(),
                &hidden_system_context,
            );
            let result = owned.provider_store.enqueue_structured_prompt_submit(
                dispatch.session_id.clone(),
                dispatch.provider_run_id.clone(),
                dispatch.agent_id.clone(),
                dispatch.prompt_id.clone(),
                &provider_run,
                &prompt_with_handoff,
                &hidden_system_context,
                &dispatch.attachments,
                mode,
                dispatch.steering,
            );
            if result.is_ok() {
                owned.consume_pending_context_handoff(
                    &dispatch.session_id,
                    &dispatch.agent_id,
                    &provider_run,
                );
            }
            return result;
        }
        if !internal_recovery
            && !crate::scheduler::runtime::is_workflow_prompt_attachment(
                &dispatch.source_attachment_id,
            )
        {
            match owned
                .attachment_store
                .get_attachment(&dispatch.source_attachment_id)
            {
                Ok(attachment) if attachment.session_id() != dispatch.session_id => {
                    return Err(DaemonError::AttachmentNotInSession {
                        session_id: dispatch.session_id.clone(),
                        attachment_id: dispatch.source_attachment_id.clone(),
                    });
                }
                Ok(_) | Err(DaemonError::AttachmentNotFound { .. }) => {}
                Err(error) => return Err(error),
            }
        }
        let prompt_with_handoff = owned.prompt_with_pending_context_handoff(
            &dispatch.session_id,
            &dispatch.agent_id,
            &dispatch.source_attachment_id,
            &provider_run,
            &dispatch.prompt,
        );
        let prompt_with_hidden_context =
            join_hidden_context(&dispatch.hidden_system_context, &prompt_with_handoff);
        let provider_prompt = owned.apply_granted_skill_summary(
            &dispatch.session_id,
            &dispatch.agent_id,
            &prompt_with_hidden_context,
        )?;
        let uses_claude_native_bridge =
            crate::provider::provider_run_uses_claude_native_bridge(&provider_run);
        let provider_pty_input =
            crate::app::terminal_input::provider_prompt_input(&provider_prompt);
        if !internal_recovery {
            self.observe_git_before_prompt_dispatch(dispatch, &provider_run)
                .await;
            owned.terminal_stream.record_input(
                &dispatch.session_id,
                &dispatch.provider_run_id,
                &dispatch.source_attachment_id,
                if uses_claude_native_bridge {
                    provider_prompt.as_bytes()
                } else {
                    &provider_pty_input
                },
            );
        }
        let mut has_managed_process = owned
            .provider_process_tracking
            .read()
            .run_processes
            .contains_key(&dispatch.provider_run_id);
        if crate::provider::provider_run_is_claude_headless(&provider_run) && !has_managed_process {
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(10_000);
            while tokio::time::Instant::now() < deadline {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                has_managed_process = owned
                    .provider_process_tracking
                    .read()
                    .run_processes
                    .contains_key(&dispatch.provider_run_id);
                if has_managed_process {
                    break;
                }
            }
            if !has_managed_process {
                return Err(DaemonError::LocalTransport {
                    operation: "submit Claude headless prompt",
                    message: format!(
                        "provider process for `{}` was not ready",
                        dispatch.provider_run_id
                    ),
                });
            }
        }
        if !has_managed_process {
            if !dispatch.steering {
                let session = owned.session_store.get_session(&dispatch.session_id)?;
                let Some(_settlement_claim) = owned
                    .prompt_state_owner
                    .try_claim_active_prompt_delivery_settlement(
                        &session,
                        &dispatch.agent_id,
                        &dispatch.prompt_id,
                        &dispatch.provider_run_id,
                    )
                else {
                    return Ok(());
                };
                owned.note_prompt_started(&dispatch.provider_run_id);
                owned.mark_active_prompt_delivery(
                    &dispatch.session_id,
                    &dispatch.agent_id,
                    &dispatch.prompt_id,
                    crate::session::DurablePromptDeliveryPhase::Delivered,
                    Some(dispatch.provider_run_id.clone()),
                    provider_run.provider_session_id().map(str::to_string),
                )?;
            }
            return Ok(());
        }
        if uses_claude_native_bridge {
            let dispatch_with_handoff = crate::app::KernelPromptDispatch {
                session_id: dispatch.session_id.clone(),
                provider_run_id: dispatch.provider_run_id.clone(),
                agent_id: dispatch.agent_id.clone(),
                prompt_id: dispatch.prompt_id.clone(),
                target_active_prompt_id: dispatch.target_active_prompt_id.clone(),
                source_attachment_id: dispatch.source_attachment_id.clone(),
                prompt: dispatch.prompt.clone(),
                hidden_system_context: owned.hidden_context_with_pending_context_handoff(
                    &dispatch.session_id,
                    &dispatch.agent_id,
                    &provider_run,
                    &dispatch.hidden_system_context,
                ),
                attachments: dispatch.attachments.clone(),
                prompt_origin: dispatch.prompt_origin,
                external_provider: dispatch.external_provider.clone(),
                external_provider_session_id: dispatch.external_provider_session_id.clone(),
                external_provider_turn_id: dispatch.external_provider_turn_id.clone(),
                steering: dispatch.steering,
            };
            let provider_run = provider_run.clone();
            // Claude-headless confirms injection asynchronously via the
            // context-file marker; retry with the app lock released between
            // attempts so a slow provider cannot stall the whole daemon.
            // Claude's first interactive composer can take longer than the
            // normal warm-start path while it restores local state. Keep the
            // dispatch pending until UserPromptSubmit proves the provider
            // accepted it; the retry loop releases the app lock between
            // attempts, so this grace period does not block other sessions.
            let deadline = tokio::time::Instant::now() + CLAUDE_HEADLESS_PROMPT_ACK_TIMEOUT;
            loop {
                let attempt = self
                    .with_app_side_effect(|app| {
                        app.process_claude_native_prompt_dispatch_attempt_for_runtime(
                            &dispatch.session_id,
                            &dispatch.provider_run_id,
                            &provider_run,
                            &dispatch_with_handoff,
                        )
                    })
                    .await?;
                match attempt {
                    crate::app::ClaudeNativeDispatchAttempt::Completed => break,
                    crate::app::ClaudeNativeDispatchAttempt::AwaitingInjection => {
                        if let Some(message) =
                            claude_native_dispatch_terminal_failure(&provider_run)
                        {
                            return Err(DaemonError::ProviderProtocol {
                                provider_run_id: dispatch.provider_run_id.clone(),
                                operation: "submit Claude headless prompt",
                                message,
                            });
                        }
                        if tokio::time::Instant::now() >= deadline {
                            return Err(DaemonError::LocalTransport {
                                operation: CLAUDE_HEADLESS_PROMPT_ACK_OPERATION,
                                message: format!(
                                    "provider `{}` did not acknowledge prompt `{}` after PTY injection",
                                    dispatch.provider_run_id, dispatch.prompt_id
                                ),
                            });
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                }
            }
            owned.consume_pending_context_handoff(
                &dispatch.session_id,
                &dispatch.agent_id,
                &provider_run,
            );
            if !dispatch.steering {
                let session = owned.session_store.get_session(&dispatch.session_id)?;
                let Some(_settlement_claim) = owned
                    .prompt_state_owner
                    .try_claim_active_prompt_delivery_settlement(
                        &session,
                        &dispatch.agent_id,
                        &dispatch.prompt_id,
                        &dispatch.provider_run_id,
                    )
                else {
                    return Ok(());
                };
                owned.note_prompt_started(&dispatch.provider_run_id);
                owned.mark_active_prompt_delivery(
                    &dispatch.session_id,
                    &dispatch.agent_id,
                    &dispatch.prompt_id,
                    crate::session::DurablePromptDeliveryPhase::Delivered,
                    Some(dispatch.provider_run_id.clone()),
                    provider_run.provider_session_id().map(str::to_string),
                )?;
            }
            return Ok(());
        }
        let provider_run_id = dispatch.provider_run_id.clone();
        let writer = self
            .with_app_side_effect(|app| app.provider_pty_input_writer_for_runtime(&provider_run_id))
            .await?;
        let provider_run_id = dispatch.provider_run_id.clone();
        let write_task =
            tokio::task::spawn_blocking(move || writer.write_input(&provider_pty_input));
        match tokio::time::timeout(std::time::Duration::from_secs(15), write_task).await {
            Ok(result) => result.map_err(|error| DaemonError::LocalTransport {
                operation: "write provider PTY input",
                message: error.to_string(),
            })??,
            Err(_) => {
                return Err(DaemonError::PtyWrite {
                    provider_run_id,
                    message: "timed out after 15 seconds of provider backpressure".to_string(),
                });
            }
        }
        owned.consume_pending_context_handoff(
            &dispatch.session_id,
            &dispatch.agent_id,
            &provider_run,
        );
        if !dispatch.steering {
            let session = owned.session_store.get_session(&dispatch.session_id)?;
            let Some(_settlement_claim) = owned
                .prompt_state_owner
                .try_claim_active_prompt_delivery_settlement(
                    &session,
                    &dispatch.agent_id,
                    &dispatch.prompt_id,
                    &dispatch.provider_run_id,
                )
            else {
                return Ok(());
            };
            owned.note_prompt_started(&dispatch.provider_run_id);
            owned.mark_active_prompt_delivery(
                &dispatch.session_id,
                &dispatch.agent_id,
                &dispatch.prompt_id,
                crate::session::DurablePromptDeliveryPhase::Delivered,
                Some(dispatch.provider_run_id.clone()),
                provider_run.provider_session_id().map(str::to_string),
            )?;
        }
        Ok(())
    }

    pub(super) async fn fail_prompt_dispatch(
        &self,
        dispatch: crate::app::KernelPromptDispatch,
        error: DaemonError,
    ) -> Result<(), DaemonError> {
        let mut next_dispatch = None;
        let dispatch_owns_active_prompt = self
            .owned
            .prompt_dispatch_matches_active_prompt(&dispatch)?;
        let failed_provider_run = dispatch_owns_active_prompt
            .then(|| {
                self.owned
                    .provider_store
                    .get_run(&dispatch.provider_run_id)
                    .ok()
            })
            .flatten()
            .filter(|run| {
                claude_headless_dispatch_failure_requires_provider_retirement(run, &error)
            });
        let retire_failed_provider = failed_provider_run.is_some();
        let mut _delivery_settlement_claim = None;
        if retire_failed_provider {
            if let Some(provider_run) = failed_provider_run
                .as_ref()
                .filter(|_| claude_headless_dispatch_failure_invalidates_resume(&error))
            {
                let session = self.owned.session_store.get_session(&dispatch.session_id)?;
                let Some(settlement_claim) = self
                    .owned
                    .prompt_state_owner
                    .try_claim_active_prompt_delivery_settlement(
                        &session,
                        &dispatch.agent_id,
                        &dispatch.prompt_id,
                        &dispatch.provider_run_id,
                    )
                else {
                    return Err(error);
                };
                let Some(active_prompt) = self
                    .owned
                    .prompt_state_owner
                    .active_prompt_for_agent(&session, &dispatch.agent_id)
                else {
                    return Err(error);
                };
                let Some(expected_provider_session_id) = active_prompt
                    .durable_delivery_provider_session_id()
                    .map(str::to_string)
                else {
                    return Err(error);
                };
                let dispatching_status = active_prompt.status();
                if self
                    .owned
                    .compare_and_mark_active_prompt_delivery_failure(
                        &dispatch.session_id,
                        &dispatch.agent_id,
                        &dispatch.prompt_id,
                        &dispatch.provider_run_id,
                        &expected_provider_session_id,
                        (dispatching_status, crate::session::PromptStatus::Cancelling),
                    )?
                    .is_none()
                {
                    return Err(error);
                }
                match self.clear_unresponsive_provider_resume_state(
                    provider_run,
                    &expected_provider_session_id,
                ) {
                    Ok(
                        crate::agent::ProviderResumeClearOutcome::Cleared
                        | crate::agent::ProviderResumeClearOutcome::AlreadyAbsent,
                    ) => {}
                    Ok(crate::agent::ProviderResumeClearOutcome::Superseded {
                        current_provider_session_id,
                    }) => {
                        self.owned.restore_active_prompt_after_resume_superseded(
                            &dispatch.session_id,
                            &dispatch.agent_id,
                            &dispatch.prompt_id,
                            &dispatch.provider_run_id,
                            &expected_provider_session_id,
                            &current_provider_session_id,
                        )?;
                        return Err(error);
                    }
                    Err(clear_error) => return Err(clear_error),
                }
                _delivery_settlement_claim = Some(settlement_claim);
            }
            if let Ok(outcome) = self
                .owned
                .provider_store
                .terminate_run_provider_only(&dispatch.session_id, &dispatch.provider_run_id)
            {
                let _ = self.owned.clear_active_provider_run_session_pointer(
                    &dispatch.session_id,
                    outcome.run().id(),
                );
                self.owned
                    .provider_run_projection
                    .update(outcome.into_run());
            }
            let provider_run_id = dispatch.provider_run_id.clone();
            let (_, process_key) = self
                .with_app_side_effect(move |app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(&provider_run_id)
                })
                .await
                .unwrap_or((false, None));
            self.owned
                .remove_provider_process_tracking_for_run(&dispatch.provider_run_id, process_key);
        }
        let mut restart_provider_for_queued_prompt = false;
        {
            let owned = &self.owned;
            owned.update_metaagent_event_prompt_delivery_for_prompt(
                &dispatch.prompt_id,
                crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Failed,
                Some(error.to_string()),
            );
            let failed_prompt = owned.prompt_state_owner.active_prompt_for_agent(
                &owned.session_store.get_session(&dispatch.session_id)?,
                &dispatch.agent_id,
            );
            if let Some(failed_prompt) = failed_prompt
                .as_ref()
                .filter(|prompt| prompt.id() == dispatch.prompt_id)
            {
                let _ = self.inject_metaagent_turn_failure_event(
                    &dispatch.session_id,
                    &dispatch.agent_id,
                    failed_prompt,
                    Some(&dispatch.provider_run_id),
                    &error.to_string(),
                );
            }
            // A dispatch completion can arrive after the prompt has already been
            // settled and the next queued prompt has become active.  Only the
            // exact dispatch prompt may be failed or cancelled; treating any
            // active prompt as a match lets a stale provider error cancel the
            // replacement prompt and strand the queue.
            let failed_prompt_matches = failed_prompt
                .as_ref()
                .is_some_and(|prompt| prompt.id() == dispatch.prompt_id);
            let dispatch_failure = format!("Provider prompt dispatch failed: {error}");
            if failed_prompt_matches {
                owned.record_provider_failure_output(
                    &dispatch.session_id,
                    &dispatch.provider_run_id,
                    &dispatch.agent_id,
                    &dispatch_failure,
                );
            }
            let (should_advance, released_claim) = match owned
                .settle_failed_local_prompt_without_advance(
                    &dispatch.session_id,
                    &dispatch.agent_id,
                    &dispatch.prompt_id,
                    &dispatch.provider_run_id,
                    &dispatch_failure,
                ) {
                Ok(Some(released_claim)) => (true, released_claim),
                Ok(None) => (false, false),
                Err(settlement_error) => {
                    crate::logging::warn_with_fields(
                        "daemon.prompt_delivery",
                        "failed to settle prompt after dispatch failure",
                        serde_json::json!({
                            "session_id": dispatch.session_id,
                            "agent_id": dispatch.agent_id,
                            "prompt_id": dispatch.prompt_id,
                            "provider_run_id": dispatch.provider_run_id,
                            "error": settlement_error.to_string(),
                        }),
                    );
                    let cancelled = failed_prompt_matches
                        && owned
                            .cancel_active_prompt_only(&dispatch.session_id, &dispatch.agent_id)
                            .is_ok();
                    let released_claim = if cancelled {
                        owned.clear_prompt_activity(&dispatch.provider_run_id)
                    } else {
                        false
                    };
                    (cancelled, released_claim)
                }
            };
            if should_advance && retire_failed_provider {
                restart_provider_for_queued_prompt = owned
                    .prompt_state_owner
                    .peek_next_queued_prompt(
                        &owned.session_store.get_session(&dispatch.session_id)?,
                        &dispatch.agent_id,
                    )
                    .is_some();
            } else if should_advance {
                match owned.advance_next_queued_prompt_dispatch(
                    &dispatch.session_id,
                    &dispatch.agent_id,
                    &dispatch.provider_run_id,
                ) {
                    Ok(dispatch) => next_dispatch = dispatch,
                    Err(advance_error) => {
                        let recipients = owned
                            .attachment_store
                            .list_session_attachment_ids(&dispatch.session_id);
                        owned.record_notice(
                            &dispatch.session_id,
                            Some(&dispatch.provider_run_id),
                            recipients,
                            format!(
                                "Queued prompt remained pending after dispatch failure: {advance_error}"
                            ),
                        );
                    }
                }
            }
            let _ = owned.session_snapshot(&dispatch.session_id);
            let recipients = owned
                .attachment_store
                .list_session_attachment_ids(&dispatch.session_id);
            owned.record_notice(
                &dispatch.session_id,
                Some(&dispatch.provider_run_id),
                recipients,
                format!("Prompt dispatch failed after acknowledgement: {error}"),
            );
            if released_claim {
                self.spawn_workflow_prompt_dispatches(owned.workflow_retry_blocked_claims());
            }
        }
        if let Some(next_dispatch) = next_dispatch {
            self.spawn_prompt_dispatch(next_dispatch, self.provider_runtime_lanes.clone());
        }
        if restart_provider_for_queued_prompt {
            let session_id = dispatch.session_id.clone();
            let agent_id = dispatch.agent_id.clone();
            let replacement_provider_run_id = self
                .with_app_side_effect(move |app| {
                    app.ensure_prompt_provider_run_for_agent(&session_id, &agent_id)
                })
                .await?;
            if let Some(replacement_dispatch) = self.owned.advance_next_queued_prompt_dispatch(
                &dispatch.session_id,
                &dispatch.agent_id,
                &replacement_provider_run_id,
            )? {
                self.spawn_prompt_dispatch(
                    replacement_dispatch,
                    self.provider_runtime_lanes.clone(),
                );
            }
        }
        Err(error)
    }

    pub(super) fn spawn_workflow_prompt_dispatches(&self, dispatches: WorkflowPromptDispatches) {
        for task in dispatches.starting_metaagent_tasks {
            let state = self.clone();
            tokio::spawn(async move {
                if let Err(error) = state.start_queued_metaagent_task(task.clone()).await {
                    let session_id = state
                        .owned
                        .agent_store
                        .get_agent(task.metaagent_id())
                        .map(|agent| agent.session_id().to_string())
                        .unwrap_or_default();
                    if !session_id.is_empty() {
                        let mut sessions = state.owned.session_store.write();
                        let _ = sessions.block_metaagent_task(
                            &session_id,
                            task.metaagent_id(),
                            format!("queued Meta task failed to start: {error}"),
                        );
                        let _ = sessions.requeue_metaagent_task_front(&session_id, task.clone());
                        drop(sessions);
                        let _ = state.owned.persist_workflow_runtime_session(
                            &session_id,
                            "metaagent_task_start_failed",
                        );
                    }
                    state.owned.record_notice(
                        &session_id,
                        None,
                        state
                            .owned
                            .attachment_store
                            .list_session_attachment_ids(&session_id),
                        format!("Queued Meta task `{}` failed to start: {error}", task.id()),
                    );
                }
            });
        }
        let mut provider_run_retirements = dispatches.provider_run_retirements;
        for provider_run_id in dispatches.starting_provider_runs {
            let retired_provider_run_ids = provider_run_retirements
                .remove(&provider_run_id)
                .unwrap_or_default();
            if retired_provider_run_ids.is_empty() {
                self.spawn_detached_workflow_provider_launch(provider_run_id);
            } else {
                self.spawn_detached_workflow_provider_launch_after_retiring(
                    provider_run_id,
                    retired_provider_run_ids,
                );
            }
        }
        for retired_provider_run_ids in provider_run_retirements.into_values() {
            self.spawn_retired_workflow_provider_cleanup(retired_provider_run_ids);
        }
        for dispatch in dispatches.local {
            self.spawn_prompt_dispatch(dispatch, self.provider_runtime_lanes.clone());
        }
        for dispatch in dispatches.remote {
            self.spawn_remote_prompt_dispatch(dispatch);
        }
    }

    async fn start_queued_metaagent_task(
        &self,
        task: crate::session::QueuedMetaagentTask,
    ) -> Result<(), DaemonError> {
        let agent = self.owned.agent_store.get_agent(task.metaagent_id())?;
        let session_id = agent.session_id().to_string();
        self.activate_meta_mode_for_prompt(&session_id, agent.id(), task.task_markdown())
            .await?;
        let task_attachment_id = self.ensure_metaagent_task_attachment(&session_id, &agent)?;
        let prompt = crate::session::PromptQueueItem::new(
            format!("pending-draft:{}", task.id()),
            task_attachment_id,
            agent.id(),
            task.task_markdown(),
            crate::session::PromptStatus::Queued,
        )
        .with_hidden_system_context(Self::meta_mode_entered_hidden_context()?)
        .with_attachments(task.attachments().to_vec());
        let mut submission = self
            .submit_prepared_prompt(crate::app::KernelPreparedPromptSubmission {
                session_id: session_id.clone(),
                prompt,
                force_queue: false,
                refresh_projection: true,
            })
            .await?;
        if let (crate::session::PromptSubmissionOutcome::Started { prompt }, Some(dispatch)) =
            (&submission.outcome, submission.dispatch.as_ref())
        {
            self.start_active_turn_with_trace_id(
                &dispatch.session_id,
                &dispatch.agent_id,
                prompt.id(),
                &dispatch.provider_run_id,
                task.id(),
            );
        }
        if let Some(dispatch) = submission.dispatch.take() {
            self.spawn_prompt_dispatch(dispatch, self.provider_runtime_lanes.clone());
        }
        if let Some(dispatch) = submission.remote_dispatch.take() {
            self.spawn_remote_prompt_dispatch(dispatch);
        }
        if let Err(error) = self
            .owned
            .persist_workflow_runtime_session(&session_id, "metaagent_task_started")
        {
            self.owned.record_notice(
                &session_id,
                None,
                self.owned
                    .attachment_store
                    .list_session_attachment_ids(&session_id),
                format!(
                    "Meta task started, but its session snapshot could not be persisted: {error}"
                ),
            );
        }
        Ok(())
    }

    fn spawn_detached_workflow_provider_launch(&self, provider_run_id: String) {
        self.spawn_detached_workflow_provider_launch_after_retiring(provider_run_id, Vec::new());
    }

    fn spawn_retired_workflow_provider_cleanup(&self, retired_provider_run_ids: Vec<String>) {
        if retired_provider_run_ids.is_empty() {
            return;
        }
        let state = self.clone();
        tokio::spawn(async move {
            for retired_provider_run_id in retired_provider_run_ids {
                let cleanup_run_id = retired_provider_run_id.clone();
                if let Err(error) = state
                    .with_app_side_effect(move |app| {
                        crate::app::ProviderLaunchProcessRuntime::new(app)
                            .remove_run(&cleanup_run_id)
                    })
                    .await
                {
                    crate::logging::warn_with_fields(
                        "daemon.provider",
                        "failed to retire workflow provider process",
                        serde_json::json!({
                            "provider_run_id": retired_provider_run_id,
                            "error": error.to_string(),
                        }),
                    );
                }
            }
        });
    }

    fn spawn_detached_workflow_provider_launch_after_retiring(
        &self,
        provider_run_id: String,
        retired_provider_run_ids: Vec<String>,
    ) {
        let claim = {
            let mut claims = self
                .detached_workflow_provider_launches
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !claims.insert(provider_run_id.clone()) {
                return;
            }
            DetachedWorkflowProviderLaunchClaim {
                provider_run_id: provider_run_id.clone(),
                claims: Arc::clone(&self.detached_workflow_provider_launches),
            }
        };
        let state = self.clone();
        tokio::spawn(async move {
            let _claim = claim;
            for retired_provider_run_id in retired_provider_run_ids {
                let cleanup_run_id = retired_provider_run_id.clone();
                if let Err(error) = state
                    .with_app_side_effect(move |app| {
                        crate::app::ProviderLaunchProcessRuntime::new(app)
                            .remove_run(&cleanup_run_id)
                    })
                    .await
                {
                    crate::logging::warn_with_fields(
                        "daemon.provider",
                        "failed to retire workflow provider process",
                        serde_json::json!({
                            "provider_run_id": retired_provider_run_id,
                            "error": error.to_string(),
                        }),
                    );
                }
            }
            let provider_credential_env = state
                .owned
                .take_pending_provider_launch_credentials(&provider_run_id);
            let run = match state.owned.provider_store.get_run(&provider_run_id) {
                Ok(run) if run.state() == crate::provider::ProviderRunState::Starting => run,
                _ => return,
            };
            let started = crate::app::StartedProviderLaunch {
                run: run.clone(),
                previous_active_run_id: None,
                provider_credential_env,
            };
            let spawn_result = state
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app)
                        .spawn_for_launch_with_credentials(&run, &started.provider_credential_env)
                })
                .await;
            if let Err(error) = spawn_result {
                state.fail_provider_launch(&started, &error).await;
                return;
            }
            state.owned.provider_run_projection.update(run.clone());
            let runtime_init_delay_ms = state
                .owned
                .config_projection
                .snapshot()
                .provider_runtime_init_delay_ms;
            if runtime_init_delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(runtime_init_delay_ms)).await;
            }
            let provider_credential_env = started.provider_credential_env.clone();
            let binding = tokio::task::spawn_blocking(move || {
                crate::provider::ProviderProcessService::initialize_runtime_binding_with_credentials(
                    &run,
                    &provider_credential_env,
                )
            })
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "initialize workflow provider runtime",
                message: error.to_string(),
            });
            match binding {
                Ok(Ok(binding)) => state.finish_provider_launch(&started, binding).await,
                Ok(Err(error)) | Err(error) => {
                    state.fail_provider_launch(&started, &error).await;
                }
            }
        });
    }

    pub(super) async fn enqueue_prompt_abort(
        &self,
        dispatch: &crate::app::KernelPromptAbortDispatch,
    ) -> Result<(), DaemonError> {
        {
            let owned = &self.owned;
            owned.reap_structured_prompt_jobs();
            self.reconcile_provider_run_exit(&dispatch.session_id, &dispatch.provider_run_id)
                .await?;
            let provider_run = owned
                .ensure_provider_run_in_session(&dispatch.session_id, &dispatch.provider_run_id)?;
            if provider_run.state() != crate::provider::ProviderRunState::Running {
                return Err(DaemonError::InvalidProviderRunState {
                    provider_run_id: dispatch.provider_run_id.clone(),
                    state: provider_run.state(),
                    operation: "submit prompt",
                });
            }
            if owned
                .provider_store
                .run_uses_structured_prompt_io(&provider_run)
            {
                return owned.provider_store.enqueue_structured_prompt_abort(
                    dispatch.session_id.clone(),
                    dispatch.provider_run_id.clone(),
                );
            }
            owned.terminal_stream.record_input(
                &dispatch.session_id,
                &dispatch.provider_run_id,
                &dispatch.source_attachment_id,
                b"\x03",
            );
            self.with_app_side_effect(|app| {
                app.write_provider_pty_input_for_runtime(&dispatch.provider_run_id, b"\x03")
            })
            .await?;
            Ok(())
        }
    }

    pub(super) async fn structured_prompt_io_in_flight(&self, provider_run_id: &str) -> bool {
        {
            let owned = &self.owned;
            owned
                .provider_store
                .structured_prompt_io_in_flight(provider_run_id)
        }
    }

    pub(super) async fn fail_prompt_abort(
        &self,
        dispatch: crate::app::KernelPromptAbortDispatch,
        error: DaemonError,
    ) -> Result<(), DaemonError> {
        {
            let owned = &self.owned;
            let recipients = owned
                .attachment_store
                .list_session_attachment_ids(&dispatch.session_id);
            owned.record_notice(
                &dispatch.session_id,
                Some(&dispatch.provider_run_id),
                recipients,
                format!("Prompt cancellation dispatch failed after acknowledgement: {error}"),
            );
            Err(error)
        }
    }

    pub(crate) fn spawn_prompt_dispatch(
        &self,
        dispatch: crate::app::KernelPromptDispatch,
        provider_runtime_lanes: ProviderRunOperationLanes,
    ) {
        let state = self.clone();
        tokio::spawn(async move {
            let _permit = provider_runtime_lanes
                .acquire(&dispatch.provider_run_id)
                .await;
            if let Err(error) = state.enqueue_prompt_dispatch(&dispatch).await {
                let _ = state.fail_prompt_dispatch(dispatch, error).await;
            }
        });
    }

    pub(crate) fn spawn_queued_prompt_steer_dispatch(
        &self,
        dispatch: crate::app::KernelPromptDispatch,
        provider_runtime_lanes: ProviderRunOperationLanes,
    ) {
        let state = self.clone();
        tokio::spawn(async move {
            let _permit = provider_runtime_lanes
                .acquire(&dispatch.provider_run_id)
                .await;
            if let Err(error) = state.enqueue_prompt_dispatch(&dispatch).await {
                let recipients = state
                    .owned
                    .attachment_store
                    .list_session_attachment_ids(&dispatch.session_id);
                state.owned.record_notice(
                    &dispatch.session_id,
                    Some(&dispatch.provider_run_id),
                    recipients,
                    format!("Queued prompt steer dispatch failed: {error}"),
                );
            }
        });
    }

    pub(crate) fn spawn_prompt_abort(
        &self,
        dispatch: crate::app::KernelPromptAbortDispatch,
        provider_runtime_lanes: ProviderRunOperationLanes,
    ) {
        let state = self.clone();
        tokio::spawn(async move {
            let _permit = provider_runtime_lanes
                .acquire(&dispatch.provider_run_id)
                .await;
            let structured = state
                .owned
                .provider_store
                .get_run(&dispatch.provider_run_id)
                .is_ok_and(|run| {
                    state
                        .owned
                        .provider_store
                        .run_uses_structured_prompt_io(&run)
                });
            loop {
                let completion_signal =
                    structured.then(|| state.owned.provider_store.run_actor_completion_signal());
                let mut completion_sequence = completion_signal
                    .as_ref()
                    .map(|signal| signal.sequence())
                    .unwrap_or_default();
                let outcome = match state.enqueue_prompt_abort(&dispatch).await {
                    Ok(()) if !structured => {
                        if let Ok(provider_run) = state
                            .owned
                            .provider_store
                            .get_run(&dispatch.provider_run_id)
                        {
                            if let Some(agent_id) = provider_run.agent_instance_id() {
                                if let Ok(session) =
                                    state.owned.session_store.get_session(&dispatch.session_id)
                                {
                                    if let Some(prompt) = state
                                        .owned
                                        .prompt_state_owner
                                        .active_prompt_for_agent(&session, agent_id)
                                    {
                                        let _ = state
                                            .owned
                                            .workflow_cancel_prompt(&dispatch.session_id, &prompt);
                                    }
                                }
                                let _ = state
                                    .owned
                                    .finalize_local_prompt_cancellation_with_queued_advance(
                                        &dispatch.session_id,
                                        agent_id,
                                        Some(&dispatch.provider_run_id),
                                    );
                            }
                        }
                        PromptAbortDispatchOutcome::Done
                    }
                    Ok(()) => loop {
                        let completion_signal = completion_signal
                            .as_ref()
                            .expect("structured abort should have a completion signal");
                        completion_signal
                            .wait_for_change_after(completion_sequence)
                            .await;
                        completion_sequence = completion_signal.sequence();
                        state.owned.reap_structured_prompt_jobs();
                        let prompt_is_still_cancelling = state
                            .owned
                            .provider_store
                            .get_run(&dispatch.provider_run_id)
                            .ok()
                            .and_then(|run| run.agent_instance_id().map(str::to_string))
                            .and_then(|agent_id| {
                                state
                                    .owned
                                    .session_store
                                    .get_session(&dispatch.session_id)
                                    .ok()
                                    .and_then(|session| {
                                        state
                                            .owned
                                            .prompt_state_owner
                                            .active_prompt_for_agent(&session, &agent_id)
                                    })
                            })
                            .is_some_and(|prompt| {
                                prompt.status() == crate::session::PromptStatus::Cancelling
                            });
                        if !prompt_is_still_cancelling {
                            state.spawn_workflow_prompt_dispatches(
                                state.owned.workflow_retry_blocked_claims(),
                            );
                            let _ = state.owned.session_snapshot(&dispatch.session_id);
                            break PromptAbortDispatchOutcome::Done;
                        }
                    },
                    Err(_)
                        if state
                            .structured_prompt_io_in_flight(&dispatch.provider_run_id)
                            .await =>
                    {
                        PromptAbortDispatchOutcome::Retry
                    }
                    Err(error) => {
                        let _ = state.fail_prompt_abort(dispatch.clone(), error).await;
                        PromptAbortDispatchOutcome::Done
                    }
                };
                match outcome {
                    PromptAbortDispatchOutcome::Done => break,
                    PromptAbortDispatchOutcome::Retry => {
                        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                    }
                }
            }
        });
    }
}

fn join_hidden_context(first: &str, second: &str) -> String {
    match (first.trim(), second.trim()) {
        ("", "") => String::new(),
        (first, "") => first.to_string(),
        ("", second) => second.to_string(),
        (first, second) => format!("{first}\n\n{second}"),
    }
}

use super::restart_recovery_runtime::is_internal_recovery_prompt_attachment;

enum PromptAbortDispatchOutcome {
    Done,
    Retry,
}
