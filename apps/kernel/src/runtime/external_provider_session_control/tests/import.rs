use super::*;

#[test]
fn import_external_provider_session_creates_session_agent_and_run() {
    let _environment = crate::env_lock::lock();
    let runtime = tokio::runtime::Runtime::new().expect("runtime should create");
    runtime.block_on(async {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot"),
        ));
        let store = {
            let app = crate::runtime::app_lock::lock_app_instrumented(
                &app,
                "external_provider_session_control",
            )
            .await;
            app.external_provider_session_index_store()
        };
        store.upsert(record("dev-stub", "external-1", "/tmp/external-one"));

        let response = execute_external_provider_session_request(
            &app,
            None,
            LocalDaemonRequest::ImportExternalProviderSession(
                ImportExternalProviderSessionRequest {
                    external_session_id: "dev-stub:external-1".to_string(),
                    alias: Some("Imported external one".to_string()),
                    provider: Some("dev-stub".to_string()),
                    model: Some("default".to_string()),
                    effort: None,
                    worktree_id: None,
                },
            ),
            "external-import-user",
        )
        .await
        .expect("import should succeed");

        let LocalDaemonResponse::ExternalProviderSessionImported {
            session,
            agent,
            provider_run,
        } = response
        else {
            panic!("unexpected response")
        };
        assert_eq!(session.alias(), Some("imported-external-one-external-1"));
        assert_eq!(session.worktree_id(), "/tmp/external-one");
        assert_eq!(session.owner_user_id(), "external-import-user");
        assert_eq!(agent.provider(), "dev-stub");
        assert_eq!(agent.alias(), Some("Imported external one"));
        assert_eq!(agent.owner_user_id(), "external-import-user");
        let provider_run = provider_run.expect("provider run should launch");
        assert_eq!(provider_run.session_id(), session.id());
        assert_eq!(provider_run.agent_instance_id(), Some(agent.id()));
        assert_eq!(provider_run.adapter_key(), "dev-stub");
        assert_eq!(
            session.external_provider_imports()[0].external_provider_session_id,
            "dev-stub:external-1"
        );
        assert_eq!(
            agent
                .external_provider_import()
                .expect("agent import metadata should persist")
                .external_provider_session_provider_id,
            "external-1"
        );
        assert_eq!(
            provider_run
                .external_provider_import()
                .expect("provider run import metadata should persist")
                .external_provider,
            "dev-stub"
        );
        assert!(store
            .get("dev-stub:external-1")
            .expect("record should remain indexed")
            .is_attached_to_chariox());
    });
}

#[test]
fn persist_external_import_metadata_refreshes_runtime_session_projection() {
    let _environment = crate::env_lock::lock();
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-import-projection",
            std::env::temp_dir().display().to_string(),
        ))
        .expect("session should create");
    let run = app
        .launch_provider(
            LaunchProviderRequest::new(session.id(), "dev-stub", "dev-stub", "default", "default")
                .with_agent_id(agent.id()),
        )
        .expect("provider run should launch");
    app.sessions
        .set_active_provider_run(session.id(), None)
        .expect("test should clear stale stored active run");
    app.update_session_projection(
        app.sessions()
            .get_session(session.id())
            .expect("session should still exist"),
    );

    persist_external_import_metadata(
        &mut app,
        session.id(),
        agent.id(),
        ExternalProviderImportMetadata::observed_history(
            "dev-stub:external-import-projection".to_string(),
            "dev-stub".to_string(),
            "external-import-projection".to_string(),
        ),
    )
    .expect("external import metadata should persist");

    let projected = app
        .session_state_projection_store()
        .get(session.id())
        .expect("session projection should refresh");
    assert_eq!(projected.active_provider_run_id(), Some(run.id()));
    assert_eq!(
        projected.external_provider_imports()[0].external_provider_session_id,
        "dev-stub:external-import-projection"
    );
    let projected_agent = projected
        .agents()
        .iter()
        .find(|projected_agent| projected_agent.id() == agent.id())
        .expect("projected session should include imported agent");
    assert_eq!(
        projected_agent
            .external_provider_import()
            .expect("projected agent import metadata should refresh")
            .external_provider_session_provider_id,
        "external-import-projection"
    );
}

#[test]
fn import_codex_session_without_model_uses_persisted_thread_model() {
    let _environment = crate::env_lock::lock();
    let runtime = tokio::runtime::Runtime::new().expect("runtime should create");
    runtime.block_on(async {
        let app = Arc::new(Mutex::new(
            crate::test_support::bootstrap_authenticated_app(DaemonConfig::for_tests())
                .expect("app should boot"),
        ));
        let store = {
            let app = crate::runtime::app_lock::lock_app_instrumented(
                &app,
                "external_provider_session_control",
            )
            .await;
            app.external_provider_session_index_store()
        };
        store.upsert(record("codex", "thread-1", "/tmp/codex-thread"));

        let response = execute_external_provider_session_request(
            &app,
            None,
            LocalDaemonRequest::ImportExternalProviderSession(
                ImportExternalProviderSessionRequest {
                    external_session_id: "codex:thread-1".to_string(),
                    alias: None,
                    provider: None,
                    model: None,
                    effort: None,
                    worktree_id: None,
                },
            ),
            crate::session::DEFAULT_LOCAL_USER_ID,
        )
        .await
        .expect("import should succeed");

        let LocalDaemonResponse::ExternalProviderSessionImported {
            agent,
            provider_run,
            ..
        } = response
        else {
            panic!("unexpected response")
        };
        let provider_run = provider_run.expect("provider run should launch");
        assert_eq!(agent.provider(), "codex");
        assert_eq!(agent.model(), Some("default"));
        assert_eq!(provider_run.model(), "default");
        assert_eq!(
            provider_run.resume_state().codex_thread_id(),
            Some("thread-1")
        );
    });
}

#[test]
fn import_external_provider_session_rejects_already_attached_thread() {
    let _environment = crate::env_lock::lock();
    let runtime = tokio::runtime::Runtime::new().expect("runtime should create");
    runtime.block_on(async {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot"),
        ));
        let store = {
            let app = crate::runtime::app_lock::lock_app_instrumented(
                &app,
                "external_provider_session_control",
            )
            .await;
            app.external_provider_session_index_store()
        };
        store.upsert(record(
            "dev-stub",
            "external-attached",
            "/tmp/external-attached",
        ));
        store.mark_attached(
            "dev-stub:external-attached",
            "session-existing",
            "agent-existing",
        );

        let error = execute_external_provider_session_request(
            &app,
            None,
            LocalDaemonRequest::ImportExternalProviderSession(
                ImportExternalProviderSessionRequest {
                    external_session_id: "dev-stub:external-attached".to_string(),
                    alias: None,
                    provider: Some("dev-stub".to_string()),
                    model: Some("default".to_string()),
                    effort: None,
                    worktree_id: None,
                },
            ),
            "external-import-user",
        )
        .await
        .expect_err("already attached external session should be rejected");

        assert!(error.to_string().contains(
            "already attached to Chariox session `session-existing` agent `agent-existing`"
        ));
    });
}

#[test]
fn import_external_provider_session_rejects_thread_owned_by_agent_resume_state() {
    let _environment = crate::env_lock::lock();
    let runtime = tokio::runtime::Runtime::new().expect("runtime should create");
    runtime.block_on(async {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot"),
        ));
        let (session_id, agent_id, store) = {
            let mut app = crate::runtime::app_lock::lock_app_instrumented(
                &app,
                "external_provider_session_control",
            )
            .await;
            let (session, agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should create");
            app.agents()
                .set_agent_runtime_profile(
                    agent.id(),
                    "codex",
                    Some("gpt-test".to_string()),
                    None,
                    ProviderResumeState::from_codex_thread_id("thread-owned-by-resume"),
                )
                .expect("agent runtime profile should update");
            attach_test_session(&app, session.id());
            let store = app.external_provider_session_index_store();
            (session.id().to_string(), agent.id().to_string(), store)
        };
        store.upsert(record(
            "codex",
            "thread-owned-by-resume",
            "/tmp/thread-owned-by-resume",
        ));
        assert!(
            store
                .get("codex:thread-owned-by-resume")
                .expect("record should start indexed")
                .is_attachable_to_chariox(),
            "test starts from a stale attachable store record",
        );

        let error = execute_external_provider_session_request(
            &app,
            None,
            LocalDaemonRequest::ImportExternalProviderSession(
                ImportExternalProviderSessionRequest {
                    external_session_id: "codex:thread-owned-by-resume".to_string(),
                    alias: None,
                    provider: None,
                    model: None,
                    effort: None,
                    worktree_id: None,
                },
            ),
            "external-import-user",
        )
        .await
        .expect_err("Chariox-owned Codex thread should not import as a second session");

        let message = error.to_string();
        assert!(
            message.contains(&format!(
                "already attached to Chariox session `{session_id}` agent `{agent_id}`"
            )),
            "unexpected import error: {message}"
        );
        assert!(store
            .get("codex:thread-owned-by-resume")
            .expect("record should remain indexed")
            .is_attached_to_chariox());
    });
}

#[test]
fn import_external_provider_session_rejects_discovered_thread_owned_by_agent_resume_state() {
    let _guard = crate::env_lock::lock();
    let codex_home = temp_root("codex-owned-discovery");
    let previous_codex_home = env::var_os("CODEX_HOME");
    env::set_var("CODEX_HOME", &codex_home);
    let session_dir = codex_home.join("archived_sessions");
    fs::create_dir_all(&session_dir).expect("codex session dir should create");
    fs::write(
            session_dir.join("owned-discovered.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-owned-discovered\",\"cwd\":\"/tmp/owned-discovered\",\"model_provider\":\"openai\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"This thread was created by Chariox and should not be attachable.\"}]}}\n",
            ),
        )
        .expect("codex session should write");

    let runtime = tokio::runtime::Runtime::new().expect("runtime should create");
    runtime.block_on(async {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot"),
        ));
        let (session_id, agent_id, store) = {
            let mut app = crate::runtime::app_lock::lock_app_instrumented(
                &app,
                "external_provider_session_control",
            )
            .await;
            let (session, agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should create");
            app.agents()
                .set_agent_runtime_profile(
                    agent.id(),
                    "codex",
                    Some("gpt-test".to_string()),
                    None,
                    ProviderResumeState::from_codex_thread_id("thread-owned-discovered"),
                )
                .expect("agent runtime profile should update");
            attach_test_session(&app, session.id());
            let store = app.external_provider_session_index_store();
            (session.id().to_string(), agent.id().to_string(), store)
        };
        assert!(
            store.get("codex:thread-owned-discovered").is_none(),
            "test must start from an empty external session cache"
        );

        let error = execute_external_provider_session_request(
            &app,
            None,
            LocalDaemonRequest::ImportExternalProviderSession(
                ImportExternalProviderSessionRequest {
                    external_session_id: "codex:thread-owned-discovered".to_string(),
                    alias: None,
                    provider: None,
                    model: None,
                    effort: None,
                    worktree_id: None,
                },
            ),
            "external-import-user",
        )
        .await
        .expect_err("discovered Chariox-owned Codex thread should not import");

        let message = error.to_string();
        assert!(
            message.contains(&format!(
                "already attached to Chariox session `{session_id}` agent `{agent_id}`"
            )),
            "unexpected import error: {message}"
        );
        assert!(store
            .get("codex:thread-owned-discovered")
            .expect("discovered record should remain indexed")
            .is_attached_to_chariox());
    });

    restore_env_var("CODEX_HOME", previous_codex_home);
    let _ = fs::remove_dir_all(codex_home);
}

#[test]
fn import_external_provider_agent_adds_agent_to_existing_session() {
    let _environment = crate::env_lock::lock();
    let runtime = tokio::runtime::Runtime::new().expect("runtime should create");
    runtime.block_on(async {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot"),
        ));
        let (session_id, store) = {
            let mut app = crate::runtime::app_lock::lock_app_instrumented(
                &app,
                "external_provider_session_control",
            )
            .await;
            let (session, _) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should create");
            let store = app.external_provider_session_index_store();
            (session.id().to_string(), store)
        };
        store.upsert(record("dev-stub", "external-2", "/tmp/external-two"));

        let response = execute_external_provider_session_request(
            &app,
            None,
            LocalDaemonRequest::ImportExternalProviderAgent(ImportExternalProviderAgentRequest {
                session_id: session_id.clone(),
                external_session_id: "dev-stub:external-2".to_string(),
                alias: Some("Imported agent".to_string()),
                provider: Some("dev-stub".to_string()),
                model: Some("default".to_string()),
                effort: None,
                focus: Some(true),
            }),
            "external-agent-user",
        )
        .await
        .expect("import should succeed");

        let LocalDaemonResponse::ExternalProviderAgentImported {
            session,
            agent,
            provider_run,
        } = response
        else {
            panic!("unexpected response")
        };
        assert_eq!(session.id(), session_id);
        assert_eq!(session.focused_agent_id(), Some(agent.id()));
        assert_eq!(agent.provider(), "dev-stub");
        assert_eq!(agent.alias(), Some("Imported agent"));
        assert_eq!(agent.owner_user_id(), "external-agent-user");
        assert_eq!(agent.worktree_id(), Some("/tmp/external-two"));
        assert_eq!(
            provider_run
                .expect("provider run should launch")
                .agent_instance_id(),
            Some(agent.id())
        );
        assert_eq!(
            session.external_provider_imports()[0].external_provider_session_id,
            "dev-stub:external-2"
        );
        assert_eq!(
            agent
                .external_provider_import()
                .expect("agent import metadata should persist")
                .external_provider_session_provider_id,
            "external-2"
        );
    });
}

#[test]
fn import_external_provider_agent_rejects_thread_owned_by_provider_run() {
    let _environment = crate::env_lock::lock();
    let runtime = tokio::runtime::Runtime::new().expect("runtime should create");
    runtime.block_on(async {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot"),
        ));
        let (target_session_id, owner_session_id, owner_agent_id, store) = {
            let mut app = crate::runtime::app_lock::lock_app_instrumented(
                &app,
                "external_provider_session_control",
            )
            .await;
            let (target_session, _) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new(
                    "workspace-target",
                    "worktree-target",
                ))
                .expect("target session should create");
            let (owner_session, owner_agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new(
                    "workspace-owner",
                    "worktree-owner",
                ))
                .expect("owner session should create");
            let run = test_codex_run(
                owner_session.id(),
                owner_agent.id(),
                "run-owned-thread",
                "thread-owned-by-run",
            );
            app.providers_mut().insert_run_for_test(run);
            let store = app.external_provider_session_index_store();
            (
                target_session.id().to_string(),
                owner_session.id().to_string(),
                owner_agent.id().to_string(),
                store,
            )
        };
        store.upsert(record(
            "codex",
            "thread-owned-by-run",
            "/tmp/thread-owned-by-run",
        ));
        assert!(
            store
                .get("codex:thread-owned-by-run")
                .expect("record should start indexed")
                .is_attachable_to_chariox(),
            "test starts from a stale attachable store record",
        );

        let error = execute_external_provider_session_request(
            &app,
            None,
            LocalDaemonRequest::ImportExternalProviderAgent(ImportExternalProviderAgentRequest {
                session_id: target_session_id,
                external_session_id: "codex:thread-owned-by-run".to_string(),
                alias: None,
                provider: None,
                model: None,
                effort: None,
                focus: Some(true),
            }),
            "external-agent-user",
        )
        .await
        .expect_err("Chariox-owned Codex thread should not import as a second agent");

        let message = error.to_string();
        assert!(message.contains(&format!(
            "already attached to Chariox session `{owner_session_id}` agent `{owner_agent_id}`"
        )));
        assert!(store
            .get("codex:thread-owned-by-run")
            .expect("record should remain indexed")
            .is_attached_to_chariox());
    });
}

#[test]
fn attached_chariox_agent_resume_state_removes_external_session_from_attachable_list() {
    let _environment = crate::env_lock::lock();
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    app.agents()
        .set_agent_runtime_profile(
            agent.id(),
            "codex",
            Some("gpt-test".to_string()),
            None,
            ProviderResumeState::from_codex_thread_id("thread-owned-by-chariox"),
        )
        .expect("agent runtime profile should update");
    attach_test_session(&app, session.id());
    let store = app.external_provider_session_index_store();
    store.upsert(record(
        "codex",
        "thread-owned-by-chariox",
        "/tmp/owned-by-chariox",
    ));

    mark_attached_external_provider_sessions(&app, None, &store);

    let page = store.list(&ListExternalProviderSessionsRequest {
        provider: Some("codex".to_string()),
        cursor: None,
        limit: None,
    });
    assert!(page.sessions.is_empty());
    let attached = store
        .get("codex:thread-owned-by-chariox")
        .expect("record should remain indexed");
    assert!(attached.is_attached_to_chariox());
    assert_eq!(attached.first_attached_session_id(), Some(session.id()));
    assert_eq!(attached.first_attached_agent_id(), Some(agent.id()));
}

#[test]
fn changed_attached_resume_state_returns_previous_provider_session_to_attachable_list() {
    let _environment = crate::env_lock::lock();
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    attach_test_session(&app, session.id());
    let store = app.external_provider_session_index_store();
    store.upsert(record("codex", "thread-old", "/tmp/thread-old"));
    store.upsert(record("codex", "thread-new", "/tmp/thread-new"));
    app.agents()
        .set_agent_runtime_profile(
            agent.id(),
            "codex",
            Some("gpt-test".to_string()),
            None,
            ProviderResumeState::from_codex_thread_id("thread-old"),
        )
        .expect("agent runtime profile should update");

    mark_attached_external_provider_sessions(&app, None, &store);

    assert!(store
        .get("codex:thread-old")
        .expect("old provider session should be indexed")
        .is_attached_to_chariox());

    app.agents()
        .set_agent_runtime_profile(
            agent.id(),
            "codex",
            Some("gpt-test".to_string()),
            None,
            ProviderResumeState::from_codex_thread_id("thread-new"),
        )
        .expect("agent runtime profile should update");

    mark_attached_external_provider_sessions(&app, None, &store);

    let page = store.list(&ListExternalProviderSessionsRequest {
        provider: Some("codex".to_string()),
        cursor: None,
        limit: None,
    });
    assert_eq!(
        page.sessions
            .iter()
            .map(|session| session.external_session_id.as_str())
            .collect::<Vec<_>>(),
        vec!["codex:default:thread-old"],
        "the previous provider session should be attachable once the agent points elsewhere"
    );
    let current = store
        .get("codex:thread-new")
        .expect("new provider session should be indexed");
    assert!(current.is_attached_to_chariox());
    assert_eq!(current.first_attached_session_id(), Some(session.id()));
    assert_eq!(current.first_attached_agent_id(), Some(agent.id()));
}

#[test]
fn live_provider_run_provider_session_id_counts_as_attached_to_chariox() {
    let request = LaunchProviderRequest::new("session-1", "codex", "codex", "default", "gpt-test")
        .with_agent_id("agent-1");
    let launch = crate::provider::ProviderLaunchResult {
        process_label: "codex:test".to_string(),
        endpoint_mode: crate::provider::AgentEndpointMode::Managed,
        pty_target: None,
        pty_program: None,
        pty_args: Vec::new(),
        pty_env: BTreeMap::new(),
        pty_env_remove: Vec::new(),
        working_directory: None,
        structured_endpoint: None,
    };
    let mut run = RuntimeProviderRun::new("run-1", &request, launch);
    run.set_provider_session_id(Some("thread-live-run".to_string()));
    let mut attached = BTreeSet::new();

    push_provider_run_attachment(&mut attached, &run);

    assert!(attached.contains(&AttachedExternalProviderSessionRef {
        external_session_id: "codex:default:thread-live-run".to_string(),
        session_id: "session-1".to_string(),
        agent_id: "agent-1".to_string(),
    }));
}

#[test]
fn chariox_owned_provider_run_provider_session_id_becomes_observer_target() {
    let _environment = crate::env_lock::lock();
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let run = test_codex_run(
        session.id(),
        agent.id(),
        "run-chariox-owned",
        "thread-chariox",
    );
    app.providers_mut().insert_run_for_test(run.clone());

    let target = single_attached_target(&app);

    assert_eq!(target.session_id, session.id());
    assert_eq!(target.agent_id, agent.id());
    assert_eq!(target.provider_run_id.as_deref(), Some(run.id()));
    assert_eq!(target.external_session_id, "codex:default:thread-chariox");
    assert_eq!(target.provider, "codex");
    assert_eq!(target.provider_session_id, "thread-chariox");
    assert!(matches!(
        target.cursor_source,
        AttachedExternalObserverCursorSource::CharioxOwned(_)
    ));
    assert!(app
        .agents()
        .get_agent(agent.id())
        .expect("agent should load")
        .external_provider_import()
        .is_none());
}

#[test]
fn imported_observer_target_keeps_import_cursor_source_when_provider_run_matches() {
    let _environment = crate::env_lock::lock();
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let import = ExternalProviderImportMetadata::observed_history(
        "codex:thread-imported".to_string(),
        "codex".to_string(),
        "thread-imported".to_string(),
    );
    persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
        .expect("import metadata should persist");
    let run = test_codex_run(session.id(), agent.id(), "run-imported", "thread-imported");
    app.providers_mut().insert_run_for_test(run.clone());

    let target = single_attached_target(&app);

    assert_eq!(target.provider_run_id.as_deref(), Some(run.id()));
    assert!(matches!(
        target.cursor_source,
        AttachedExternalObserverCursorSource::Imported(_)
    ));
}
