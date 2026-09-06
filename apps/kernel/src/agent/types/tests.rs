use super::{
    calculate_agent_layout, AgentInstance, AgentState, AgentSubstituteProfile, GridPosition,
};

#[test]
fn calculate_agent_layout_expands_past_six_agents() {
    assert_eq!(
        calculate_agent_layout(7),
        vec![
            GridPosition::new(0, 0, 1, 1),
            GridPosition::new(0, 1, 1, 1),
            GridPosition::new(0, 2, 1, 1),
            GridPosition::new(0, 3, 1, 1),
            GridPosition::new(1, 0, 1, 1),
            GridPosition::new(1, 1, 1, 1),
            GridPosition::new(1, 2, 1, 1),
        ]
    );
}

#[test]
fn substitute_deactivation_restores_primary_without_default_model() {
    let mut agent = AgentInstance::new(
        "agent-1",
        "agent-1",
        "session-1",
        None,
        "opencode",
        None,
        None,
        None,
        GridPosition::new(0, 0, 1, 1),
    );
    agent.set_primary_profile("opencode", None, None);
    agent.add_substitute(AgentSubstituteProfile::new(
        "codex",
        "gpt-5.4",
        Some("medium".to_string()),
    ));

    let activated = agent.activate_substitute(0, "manual");
    assert_eq!(
        activated.as_ref().map(|profile| profile.provider.as_str()),
        Some("codex")
    );
    assert_eq!(agent.provider(), "codex");
    assert_eq!(agent.model(), Some("gpt-5.4"));
    assert_eq!(agent.effort(), Some("medium"));

    agent.deactivate_substitute();
    assert_eq!(agent.provider(), "opencode");
    assert_eq!(agent.model(), None);
    assert_eq!(agent.effort(), None);
}

#[test]
fn substitute_profile_preserves_account_profile_binding_and_default_semantics() {
    let default_profile = AgentSubstituteProfile::new("codex", "gpt-5.4", None);
    assert_eq!(default_profile.account_profile, None);

    let bound = AgentSubstituteProfile::new("codex", "gpt-5.4", None)
        .with_account_profile(Some("  work  ".to_string()));
    assert_eq!(bound.account_profile.as_deref(), Some("work"));

    let cleared = AgentSubstituteProfile::new("codex", "gpt-5.4", None)
        .with_account_profile(Some("   ".to_string()));
    assert_eq!(cleared.account_profile, None);

    // Persistence round-trip: the bound profile survives serialization and a
    // legacy profile without account_profile deserializes back to `None`.
    let serialized = serde_json::to_string(&bound).expect("substitute profile should serialize");
    let deserialized: AgentSubstituteProfile =
        serde_json::from_str(&serialized).expect("substitute profile should deserialize");
    assert_eq!(deserialized, bound);
    let legacy: AgentSubstituteProfile =
        serde_json::from_str(r#"{"provider":"codex","model":"gpt-5.4"}"#)
            .expect("legacy substitute profile should deserialize");
    assert_eq!(legacy.account_profile, None);

    let mut agent = AgentInstance::new(
        "agent-1",
        "agent-1",
        "session-1",
        None,
        "opencode",
        None,
        None,
        None,
        GridPosition::new(0, 0, 1, 1),
    );
    agent.add_substitute(bound.clone());
    let activated = agent.activate_substitute(0, "manual");
    assert_eq!(
        activated.map(|profile| profile.account_profile),
        Some(Some("work".to_string()))
    );
}

#[test]
fn substitute_activation_switches_account_and_deactivation_restores_primary_account() {
    let mut agent = AgentInstance::new(
        "agent-1",
        "agent-1",
        "session-1",
        None,
        "opencode",
        None,
        None,
        None,
        GridPosition::new(0, 0, 1, 1),
    );
    agent.set_account_profile(Some("primary-work".to_string()));
    agent.set_primary_profile("opencode", Some("gpt-5.4".to_string()), None);
    agent.add_substitute(
        AgentSubstituteProfile::new("codex", "gpt-5.4", None)
            .with_account_profile(Some("substitute-personal".to_string())),
    );

    agent.activate_substitute(0, "manual");
    assert_eq!(agent.provider(), "codex");
    assert_eq!(agent.account_profile(), Some("substitute-personal"));

    // A chained activation keeps the original primary account for restore.
    agent.add_substitute(
        AgentSubstituteProfile::new("claude", "sonnet", None)
            .with_account_profile(Some("claude-second".to_string())),
    );
    agent.activate_substitute(1, "manual");
    assert_eq!(agent.account_profile(), Some("claude-second"));

    agent.deactivate_substitute();
    assert_eq!(agent.provider(), "opencode");
    assert_eq!(agent.account_profile(), Some("primary-work"));
}

#[test]
fn legacy_persisted_unbound_substitute_keeps_default_fallback_until_rebound() {
    // New substitutes can no longer be created without a bound stable account
    // (the kernel add seam resolves or rejects). This test constrains the
    // dynamic default fallback to LEGACY persisted records only: an existing
    // substitute deserialized without `account_profile` keeps launching via
    // the default sentinel, and returning to primary still restores the exact
    // primary profile.
    let mut agent = AgentInstance::new(
        "agent-1",
        "agent-1",
        "session-1",
        None,
        "opencode",
        Some("gpt-5.4".to_string()),
        None,
        None,
        GridPosition::new(0, 0, 1, 1),
    );
    agent.set_account_profile(Some("primary-work".to_string()));
    agent.set_primary_profile("opencode", Some("gpt-5.4".to_string()), None);
    let mut substitute = AgentSubstituteProfile::new("codex", "gpt-5.4", None);
    substitute.account_profile = None;
    agent.add_substitute(substitute);
    // Simulate the legacy persisted record (no account binding on disk).
    let mut restored: AgentInstance =
        serde_json::from_str(&serde_json::to_string(&agent).expect("agent should serialize"))
            .expect("agent should deserialize");
    assert_eq!(restored.substitutes()[0].account_profile, None);

    restored.activate_substitute(0, "manual");
    assert_eq!(restored.account_profile(), None);
    assert_eq!(restored.provider_account_profile(), "default");

    restored.deactivate_substitute();
    assert_eq!(restored.provider(), "opencode");
    assert_eq!(restored.account_profile(), Some("primary-work"));
}

#[test]
fn default_primary_round_trips_persistence_and_restores_default_account() {
    let mut agent = AgentInstance::new(
        "agent-1",
        "agent-1",
        "session-1",
        None,
        "opencode",
        Some("gpt-5.4".to_string()),
        None,
        None,
        GridPosition::new(0, 0, 1, 1),
    );
    agent.set_primary_profile("opencode", Some("gpt-5.4".to_string()), None);
    agent.add_substitute(
        AgentSubstituteProfile::new("codex", "gpt-5.4", None)
            .with_account_profile(Some("substitute-personal".to_string())),
    );
    agent.activate_substitute(0, "manual");

    // Persistence round-trip: the stored primary snapshot (including its
    // default account) survives serialize/deserialize.
    let serialized = serde_json::to_string(&agent).expect("agent should serialize");
    let mut restored: AgentInstance =
        serde_json::from_str(&serialized).expect("agent should deserialize");
    restored.deactivate_substitute();
    assert_eq!(restored.provider(), "opencode");
    assert_eq!(restored.model(), Some("gpt-5.4"));
    assert_eq!(restored.account_profile(), None);
    assert_eq!(restored.provider_account_profile(), "default");
}

#[test]
fn named_primary_round_trips_persistence_and_restores_named_account() {
    let mut agent = AgentInstance::new(
        "agent-1",
        "agent-1",
        "session-1",
        None,
        "opencode",
        Some("gpt-5.4".to_string()),
        None,
        None,
        GridPosition::new(0, 0, 1, 1),
    );
    agent.set_account_profile(Some("primary-work".to_string()));
    agent.set_primary_profile("opencode", Some("gpt-5.4".to_string()), None);
    agent.add_substitute(
        AgentSubstituteProfile::new("codex", "gpt-5.4", None)
            .with_account_profile(Some("substitute-personal".to_string())),
    );
    agent.activate_substitute(0, "manual");
    assert_eq!(agent.provider(), "codex");
    assert_eq!(agent.account_profile(), Some("substitute-personal"));

    let serialized = serde_json::to_string(&agent).expect("agent should serialize");
    let mut restored: AgentInstance =
        serde_json::from_str(&serialized).expect("agent should deserialize");
    restored.deactivate_substitute();
    assert_eq!(restored.provider(), "opencode");
    assert_eq!(restored.model(), Some("gpt-5.4"));
    assert_eq!(restored.effort(), None);
    assert_eq!(restored.account_profile(), Some("primary-work"));
}

#[test]
fn switching_substitutes_does_not_overwrite_stored_primary_and_removing_active_restores() {
    let mut agent = AgentInstance::new(
        "agent-1",
        "agent-1",
        "session-1",
        None,
        "opencode",
        Some("gpt-5.4".to_string()),
        Some("high".to_string()),
        None,
        GridPosition::new(0, 0, 1, 1),
    );
    agent.set_account_profile(Some("primary-work".to_string()));
    agent.set_primary_profile(
        "opencode",
        Some("gpt-5.4".to_string()),
        Some("high".to_string()),
    );
    agent.add_substitute(
        AgentSubstituteProfile::new("codex", "gpt-5.4", None)
            .with_account_profile(Some("substitute-a".to_string())),
    );
    agent.add_substitute(
        AgentSubstituteProfile::new("claude", "sonnet", None)
            .with_account_profile(Some("substitute-b".to_string())),
    );

    agent.activate_substitute(0, "manual");
    agent.activate_substitute(1, "manual");
    assert_eq!(agent.provider(), "claude");
    assert_eq!(agent.account_profile(), Some("substitute-b"));

    // Removing the ACTIVE substitute returns to the exact primary profile.
    agent.remove_substitute(1);
    assert_eq!(agent.active_substitute_index(), None);
    assert_eq!(agent.provider(), "opencode");
    assert_eq!(agent.model(), Some("gpt-5.4"));
    assert_eq!(agent.effort(), Some("high"));
    assert_eq!(agent.account_profile(), Some("primary-work"));

    // Clearing with an active substitute also restores atomically.
    agent.activate_substitute(0, "manual");
    agent.clear_substitutes();
    assert_eq!(agent.provider(), "opencode");
    assert_eq!(agent.model(), Some("gpt-5.4"));
    assert_eq!(agent.effort(), Some("high"));
    assert_eq!(agent.account_profile(), Some("primary-work"));
}

#[test]
fn moving_substitutes_preserves_order_and_tracks_the_active_profile_identity() {
    let mut agent = AgentInstance::new(
        "agent-1",
        "agent-1",
        "session-1",
        None,
        "claude",
        Some("claude-opus-4-8".to_string()),
        Some("high".to_string()),
        None,
        GridPosition::new(0, 0, 1, 1),
    );
    for (provider, model) in [
        ("opencode", "opencode-go/deepseek-v4-pro"),
        ("opencode", "deepseek-v4-pro"),
        ("codex", "gpt-5.6-sol"),
    ] {
        agent.add_substitute(AgentSubstituteProfile::new(provider, model, None));
    }
    agent.activate_substitute(1, "resource exhausted");

    assert!(agent.move_substitute(1, 0));
    assert_eq!(
        agent
            .substitutes()
            .iter()
            .map(|profile| profile.model.as_str())
            .collect::<Vec<_>>(),
        vec![
            "deepseek-v4-pro",
            "opencode-go/deepseek-v4-pro",
            "gpt-5.6-sol"
        ]
    );
    assert_eq!(agent.active_substitute_index(), Some(0));
    assert_eq!(
        agent
            .last_substitution()
            .map(|record| record.substitute_index),
        Some(0)
    );
    assert_eq!(agent.model(), Some("deepseek-v4-pro"));

    assert!(agent.move_substitute(2, 0));
    assert_eq!(agent.active_substitute_index(), Some(1));
    assert_eq!(
        agent
            .last_substitution()
            .map(|record| record.substitute_index),
        Some(1)
    );
    assert_eq!(agent.model(), Some("deepseek-v4-pro"));

    agent.remove_substitute(0);
    assert_eq!(agent.active_substitute_index(), Some(0));
    assert_eq!(
        agent
            .last_substitution()
            .map(|record| record.substitute_index),
        Some(0)
    );
    assert_eq!(agent.model(), Some("deepseek-v4-pro"));
    assert!(!agent.move_substitute(3, 0));
    assert!(!agent.move_substitute(0, 3));
}

#[test]
fn workflow_runtime_materialization_preserves_config_without_live_state() {
    let mut source = AgentInstance::new(
        "agent-1",
        "source-ref",
        "session-1",
        Some("reviewer".to_string()),
        "opencode",
        Some("x-preview-f-free".to_string()),
        Some("high".to_string()),
        Some("/source".to_string()),
        GridPosition::new(0, 0, 1, 1),
    );
    source.set_account_profile(Some("zen".to_string()));
    source.set_execution_mode_override(Some(crate::provider::AgentExecutionMode::Build));
    source.set_permission_level_override(Some(crate::provider::AgentPermissionLevel::Yolo));
    source.grant_mcp("github");
    source.add_substitute(
        AgentSubstituteProfile::new("codex", "gpt-5.6-sol", Some("high".to_string()))
            .with_account_profile(Some("codex-work".to_string())),
    );
    source.activate_substitute(0, "resource exhausted");
    source.set_provider_resume_state(
        crate::provider::ProviderResumeState::from_opencode_session_id("provider-session-secret"),
    );
    source.set_state(AgentState::Working);
    source.set_processing(true);

    let runtime = source.materialized_for_workflow_runtime(
        "agent-runtime",
        "runtime-ref",
        "session-1",
        "/isolated",
    );

    assert_eq!(runtime.provider(), "codex");
    assert_eq!(runtime.model(), Some("gpt-5.6-sol"));
    assert_eq!(runtime.effort(), Some("high"));
    assert_eq!(runtime.account_profile(), Some("codex-work"));
    assert_eq!(runtime.active_substitute_index(), Some(0));
    assert_eq!(
        runtime
            .last_substitution()
            .map(|record| record.reason.as_str()),
        Some("resource exhausted")
    );
    assert_eq!(
        runtime.execution_mode_override(),
        Some(crate::provider::AgentExecutionMode::Build)
    );
    assert_eq!(
        runtime.permission_level_override(),
        Some(crate::provider::AgentPermissionLevel::Yolo)
    );
    assert_eq!(runtime.mcp_grants(), vec!["github".to_string()]);
    assert_eq!(runtime.worktree_id(), Some("/isolated"));
    assert_eq!(runtime.state(), AgentState::Idle);
    assert!(!runtime.is_processing());
    assert!(runtime.provider_resume_state().is_empty());
    assert!(!runtime.visible_in_freeform());
}
