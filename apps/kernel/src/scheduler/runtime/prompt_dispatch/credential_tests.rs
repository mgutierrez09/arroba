use super::*;
use crate::agent::{CreateAgentRequest, RemoteAgentBinding};
use crate::app::{KernelAgentService, KernelSessionService};
use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::config::{CredentialVaultBackend, DaemonConfig};
use crate::session::{CreateSessionRequest, PromptStatus, DEFAULT_LOCAL_USER_ID};

#[derive(Clone, Copy, Debug)]
enum Caller {
    Workflow,
    Compatibility,
    Queued,
}

struct Fixture {
    app: DaemonApp,
    root: PathBuf,
    previous_home: Option<std::ffi::OsString>,
    session_id: String,
    agent_id: String,
    attachment_id: String,
    profile_id: String,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "chariox-remote-dispatch-credentials-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let previous_home = std::env::var_os("CHARIOX_HOME");
        std::env::set_var("CHARIOX_HOME", &root);
        let mut config = DaemonConfig::for_tests().with_session_history_root(root.join("history"));
        config.user_config.state.path = Some(root.join("state.db").display().to_string());
        config.user_config.history.operational.path =
            Some(root.join("operations.db").display().to_string());
        config.user_config.artifacts.operational.root =
            Some(root.join("artifacts").display().to_string());
        config.user_config.artifacts.operational.index_path =
            Some(root.join("artifacts.db").display().to_string());
        config.user_config.credential_vault.backend = CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.path =
            root.join("credentials.vault").display().to_string();
        // Intentionally no relay: credential admission must happen before transport.
        config.relay_url = None;
        let mut app = DaemonApp::bootstrap(config).unwrap();
        let (session, _) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                root.to_string_lossy(),
                root.to_string_lossy(),
            ))
            .unwrap();
        let profile = app
            .provider_account_profile_registry()
            .create_managed(DEFAULT_LOCAL_USER_ID, "claude", "Dispatch fixture")
            .unwrap();
        let agent = KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(session.id(), "claude")
                    .with_account_profile(profile.profile_id.clone()),
            )
            .unwrap();
        let attachment = KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "credential-test",
                ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        app.agents()
            .bind_remote_execution(
                agent.id(),
                RemoteAgentBinding {
                    worker_kernel_id: "worker".into(),
                    worker_machine_id: "worker-machine".into(),
                    execution_lease_id: "lease".into(),
                    leased_agent_id: "leased-agent".into(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .unwrap();
        Self {
            app,
            root,
            previous_home,
            session_id: session.id().into(),
            agent_id: agent.id().into(),
            attachment_id: attachment.id().into(),
            profile_id: profile.profile_id,
        }
    }

    fn store_token(&mut self) {
        crate::secret::unlock_chariox_encrypted_vault(
            &self.root.join("credentials.vault"),
            "fixture passphrase",
            crate::secret::VaultUnlockLease::KernelShutdown,
        )
        .unwrap();
        crate::provider::store_provider_account_credential(
            self.app.config(),
            DEFAULT_LOCAL_USER_ID,
            "claude",
            &self.profile_id,
            "dispatch-token-canary",
            false,
        )
        .unwrap();
    }

    fn dispatch(&mut self, caller: Caller) -> DaemonError {
        match caller {
            Caller::Compatibility => KernelAgentService::new(&mut self.app)
                .submit_prompt(
                    &self.session_id,
                    &self.attachment_id,
                    Some(&self.agent_id),
                    "dispatch fixture",
                    Vec::new(),
                )
                .unwrap_err(),
            Caller::Queued => {
                let prompt = PromptQueueItem::new(
                    "queued",
                    &self.attachment_id,
                    &self.agent_id,
                    "queued fixture",
                    PromptStatus::Queued,
                );
                KernelAgentService::new(&mut self.app)
                    .advance_next_queued_prompt_remote(
                        &self.session_id,
                        &self.agent_id,
                        "worker",
                        "leased-agent",
                        None,
                        None,
                        Some(&prompt),
                    )
                    .unwrap_err()
            }
            Caller::Workflow => {
                let workflow = self
                    .app
                    .sessions_mut()
                    .create_workflow(&self.session_id, None)
                    .unwrap();
                let node = self
                    .app
                    .sessions_mut()
                    .add_workflow_node(&self.session_id, workflow.id(), &self.agent_id)
                    .unwrap();
                let endpoint = self
                    .app
                    .sessions_mut()
                    .create_workflow_endpoint(&self.session_id, workflow.id(), node.id(), None)
                    .unwrap();
                let run = self
                    .app
                    .sessions_mut()
                    .invoke_workflow_endpoint(
                        &self.session_id,
                        workflow.id(),
                        endpoint.id(),
                        Some("workflow fixture".into()),
                    )
                    .unwrap();
                let node_run_id = run.node_runs()[0].id();
                self.app
                    .sessions_mut()
                    .prepare_workflow_turn(
                        &self.session_id,
                        run.id(),
                        node_run_id,
                        "fixture-delivery".into(),
                        "workflow fixture".into(),
                        None,
                        None,
                    )
                    .unwrap();
                let prompt = match submit_claimed_workflow_prompt(
                    &mut self.app,
                    &self.session_id,
                    run.id(),
                    node_run_id,
                    &self.agent_id,
                    "workflow fixture",
                )
                .unwrap()
                {
                    PromptSubmissionOutcome::Started { prompt } => prompt,
                    other => panic!("workflow should start: {other:?}"),
                };
                dispatch_workflow_prompt(&mut self.app, &self.session_id, &self.agent_id, &prompt)
                    .unwrap_err()
            }
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = crate::secret::lock_chariox_encrypted_vault(&self.root.join("credentials.vault"));
        let _ = crate::secret::clear_vault_secret_process_cache();
        match self.previous_home.take() {
            Some(value) => std::env::set_var("CHARIOX_HOME", value),
            None => std::env::remove_var("CHARIOX_HOME"),
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn remote_dispatch_callers_require_vaulted_claude_token_before_transport() {
    let _env = crate::env_lock::lock();
    for caller in [Caller::Workflow, Caller::Compatibility, Caller::Queued] {
        let mut fixture = Fixture::new();
        let error = fixture.dispatch(caller);
        assert!(
            error.to_string().contains("remote Claude launch requires"),
            "{caller:?}: {error}"
        );
    }
}

#[test]
fn remote_dispatch_callers_surface_locked_vault_before_transport() {
    let _env = crate::env_lock::lock();
    for caller in [Caller::Workflow, Caller::Compatibility, Caller::Queued] {
        let mut fixture = Fixture::new();
        fixture.store_token();
        crate::secret::lock_chariox_encrypted_vault(&fixture.root.join("credentials.vault"))
            .unwrap();
        crate::secret::clear_vault_secret_process_cache().unwrap();
        let error = fixture.dispatch(caller);
        assert!(
            crate::secret::is_chariox_vault_locked_error(&error),
            "{caller:?}: {error}"
        );
        assert!(!error.to_string().contains("dispatch-token-canary"));
    }
}

#[test]
fn remote_dispatch_callers_admit_vaulted_launch_and_reuse_active_run_without_token() {
    let _env = crate::env_lock::lock();
    for caller in [Caller::Workflow, Caller::Compatibility, Caller::Queued] {
        for active_run in [false, true] {
            let mut fixture = Fixture::new();
            if active_run {
                fixture
                    .app
                    .agents()
                    .set_remote_execution_active_worker_provider_run_id(
                        &fixture.agent_id,
                        Some("active-worker-run".into()),
                    )
                    .unwrap();
            } else {
                fixture.store_token();
            }
            let error = fixture.dispatch(caller);
            assert!(error.to_string().contains("relay_url is not configured"),
                "{caller:?}, active={active_run}: should reach transport after credential admission: {error}");
            assert!(!error.to_string().contains("dispatch-token-canary"));
        }
    }
}
