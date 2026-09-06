use super::*;

#[test]
fn workflow_code_limits_have_large_defaults() {
    let config = DaemonConfig::new("daemon", "machine", "tester");
    let limits = config.workflow_code_limits();

    assert_eq!(
        config.session_default_max_agents(),
        crate::session::DEFAULT_SESSION_MAX_AGENTS
    );
    assert_eq!(
        config.max_workflow_queues_per_workflow(),
        crate::session::DEFAULT_WORKFLOW_CODE_MAX_QUEUES as usize
    );
    assert_eq!(
        limits.max_concurrent,
        crate::session::DEFAULT_WORKFLOW_CODE_MAX_CONCURRENT
    );
    assert_eq!(
        limits.max_agents,
        crate::session::DEFAULT_WORKFLOW_CODE_MAX_AGENTS
    );
    assert_eq!(
        limits.max_endpoints,
        crate::session::DEFAULT_WORKFLOW_CODE_MAX_ENDPOINTS
    );
    assert_eq!(
        limits.max_generated_prompt_bytes,
        crate::session::DEFAULT_WORKFLOW_CODE_MAX_GENERATED_PROMPT_BYTES
    );
}

#[test]
fn workflow_code_limits_can_be_set_and_unset() {
    let path = std::env::temp_dir().join(format!(
        "chariox-workflow-code-config-test-{}-{}.toml",
        std::process::id(),
        generate_identity_suffix()
    ));
    let mut config = DaemonConfig::new("daemon", "machine", "tester");
    config.user_config_path = path.clone();

    config
        .set_user_config_value("workflow.session_default_max_agents", "2048")
        .expect("session agent cap should update");
    config
        .set_user_config_value("workflow.code.max_concurrent", "64")
        .expect("workflow-code concurrency should update");
    config
        .set_user_config_value("workflow.code.max_nodes", "256")
        .expect("workflow-code node cap should update");
    config
        .set_user_config_value("workflow.code.max_endpoints", "128")
        .expect("workflow-code endpoint cap should update");

    assert_eq!(config.session_default_max_agents(), 2048);
    let limits = config.workflow_code_limits();
    assert_eq!(limits.max_concurrent, 64);
    assert_eq!(limits.max_nodes, 256);
    assert_eq!(limits.max_endpoints, 128);

    config
        .unset_user_config_value("workflow.code.max_concurrent")
        .expect("workflow-code concurrency should unset");

    assert_eq!(
        config.workflow_code_limits().max_concurrent,
        crate::session::DEFAULT_WORKFLOW_CODE_MAX_CONCURRENT
    );
    assert_eq!(
        config
            .user_config
            .workflow
            .code
            .as_ref()
            .expect("remaining workflow-code config should stay")
            .max_nodes,
        Some(256)
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn workflow_code_queue_limit_is_capped_by_runtime_queue_limit() {
    let mut config = DaemonConfig::new("daemon", "machine", "tester");
    config.user_config.workflow.max_queues_per_workflow = Some(2);
    config.user_config.workflow.code = Some(UserWorkflowCodeConfig {
        max_queues: Some(8),
        ..UserWorkflowCodeConfig::default()
    });

    assert_eq!(config.max_workflow_queues_per_workflow(), 2);
    assert_eq!(config.workflow_code_limits().max_queues, 2);
}

#[test]
fn workflow_code_limits_reject_zero_values() {
    let mut config = DaemonConfig::new("daemon", "machine", "tester");

    let error = config
        .set_user_config_value("workflow.code.max_concurrent", "0")
        .expect_err("zero concurrency should be rejected");

    assert!(matches!(
        error,
        DaemonError::InvalidConfig {
            field: "workflow.code.max_concurrent",
            ..
        }
    ));
}

#[test]
fn history_and_state_config_defaults_are_available() {
    let config = DaemonConfig::new("daemon", "machine", "tester");

    assert_eq!(
        config.user_config.history.operational.backend,
        HistoryOperationalBackend::Sqlite
    );
    assert!(config.user_config.history.operational.enabled);
    assert_eq!(
        config.user_config.history.operational.retention_days,
        Some(30)
    );
    assert_eq!(
        config.user_config.history.operational.max_size_mb,
        Some(crate::history::OPERATIONAL_HISTORY_HARD_MAX_MB)
    );
    assert_eq!(
        config.operational_history_max_size_bytes(),
        crate::history::OPERATIONAL_HISTORY_HARD_MAX_BYTES
    );
    assert_eq!(
        config.user_config.history.archive.mode,
        HistoryArchiveMode::Disabled
    );
    assert_eq!(config.user_config.state.backend, StateBackend::Sqlite);
    assert_eq!(
        config.user_config.state.snapshot_interval_events,
        Some(1_000)
    );
    assert_eq!(
        config.user_config.state.snapshot_interval_bytes,
        Some(4 * 1024 * 1024)
    );
    assert_eq!(
        config.user_config.state.snapshot_interval_seconds,
        Some(300)
    );
    assert_eq!(
        config.user_config.state.snapshot_max_tail_bytes,
        Some(16 * 1024 * 1024)
    );
}

#[test]
fn history_archive_external_requires_url() {
    let mut config = DaemonConfig::new("daemon", "machine", "tester");

    let error = config
        .set_user_config_value("history.archive.mode", "external")
        .expect_err("external archive without a URL should be rejected");

    match error {
        DaemonError::InvalidConfig { field, .. } => {
            assert_eq!(field, "history.archive.url");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn history_and_state_config_can_be_changed_and_persisted() {
    let path = std::env::temp_dir().join(format!(
        "chariox-history-config-test-{}-{}.toml",
        std::process::id(),
        generate_identity_suffix()
    ));
    let mut config = DaemonConfig::new("daemon", "machine", "tester");
    config.user_config_path = path.clone();

    config
        .set_user_config_value("history.operational.enabled", "false")
        .expect("operational history capture should update");
    config
        .set_user_config_value("history.operational.path", "~/.chariox/custom/history.db")
        .expect("operational history path should update");
    config
        .set_user_config_value("history.operational.retention_days", "10")
        .expect("retention should update");
    config
        .set_user_config_value("history.archive.url", "http://127.0.0.1:49300")
        .expect("archive URL should update");
    config
        .set_user_config_value("history.archive.mode", "external")
        .expect("archive mode should update after URL is set");
    config
        .set_user_config_value("state.snapshot_interval_events", "250")
        .expect("state snapshot interval should update");
    config
        .set_user_config_value("state.snapshot_interval_bytes", "1048576")
        .expect("state snapshot byte interval should update");
    config
        .set_user_config_value("state.snapshot_interval_seconds", "30")
        .expect("state snapshot time interval should update");
    config
        .set_user_config_value("state.snapshot_max_tail_bytes", "2097152")
        .expect("state snapshot hard tail budget should update");

    let loaded = load_user_config_from_path(&path);
    assert_eq!(
        loaded.history.operational.path.as_deref(),
        Some("~/.chariox/custom/history.db")
    );
    assert!(!loaded.history.operational.enabled);
    assert_eq!(loaded.history.operational.retention_days, Some(10));
    assert_eq!(loaded.history.archive.mode, HistoryArchiveMode::External);
    assert_eq!(
        loaded.history.archive.url.as_deref(),
        Some("http://127.0.0.1:49300")
    );
    assert_eq!(loaded.state.snapshot_interval_events, Some(250));
    assert_eq!(loaded.state.snapshot_interval_bytes, Some(1_048_576));
    assert_eq!(loaded.state.snapshot_interval_seconds, Some(30));
    assert_eq!(loaded.state.snapshot_max_tail_bytes, Some(2_097_152));

    let _ = std::fs::remove_file(path);
}

#[test]
fn operational_history_size_config_is_clamped_to_hard_cap() {
    let path = std::env::temp_dir().join(format!(
        "chariox-history-size-config-test-{}-{}.toml",
        std::process::id(),
        generate_identity_suffix()
    ));
    let mut config = DaemonConfig::new("daemon", "machine", "tester");
    config.user_config_path = path.clone();

    config
        .set_user_config_value("history.operational.max_size_mb", "5000")
        .expect("oversized history cap should clamp");

    let loaded = load_user_config_from_path(&path);
    assert_eq!(
        loaded.history.operational.max_size_mb,
        Some(crate::history::OPERATIONAL_HISTORY_HARD_MAX_MB)
    );
    config.user_config = loaded;
    assert_eq!(
        config.operational_history_max_size_bytes(),
        crate::history::OPERATIONAL_HISTORY_HARD_MAX_BYTES
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn default_user_config_rejects_test_persistence_paths() {
    let mut config = CharioxUserConfig::default();
    config.history.operational.path = Some("/tmp/chariox-tests/operational-history.db".to_string());

    let error = reject_test_persistence_paths_for_persist(
        &DaemonConfig::default_user_config_path(),
        &config,
    )
    .expect_err("default config should reject leaked test paths");

    assert!(matches!(
        error,
        DaemonError::InvalidConfig {
            field: "user_config",
            ..
        }
    ));
}

#[test]
fn operational_history_path_expands_home() {
    let _environment = crate::env_lock::lock();
    let mut config = DaemonConfig::new("daemon", "machine", "tester");
    config.user_config.history.operational.path = Some("~/.chariox/custom/history.db".to_string());

    assert!(config
        .operational_history_path()
        .ends_with(".chariox/custom/history.db"));
}

#[test]
fn durable_state_path_expands_home() {
    let _environment = crate::env_lock::lock();
    let mut config = DaemonConfig::new("daemon", "machine", "tester");
    config.user_config.state.path = Some("~/.chariox/custom/state.db".to_string());

    assert!(config
        .durable_state_path()
        .ends_with(".chariox/custom/state.db"));
}

#[test]
fn event_counter_paths_expand_state_home_before_parent() {
    let _environment = crate::env_lock::lock();
    let mut config = DaemonConfig::new("daemon", "machine", "tester");
    config.user_config.state.path = Some("~/.chariox/custom/state.db".to_string());

    let kernel_counter = config.kernel_event_counter_path();
    let relay_counter = config.kernel_relay_event_counter_path();
    let prompt_counter = config.kernel_prompt_counter_path();

    assert!(!kernel_counter.starts_with("~"));
    assert!(!relay_counter.starts_with("~"));
    assert!(!prompt_counter.starts_with("~"));
    assert!(kernel_counter.ends_with(".chariox/custom/kernel-events/daemon/event-counter.json"));
    assert!(
        relay_counter.ends_with(".chariox/custom/kernel-events/daemon/relay-event-counter.json")
    );
    assert!(prompt_counter.ends_with(".chariox/custom/kernel-events/daemon/prompt-counter.json"));
}
