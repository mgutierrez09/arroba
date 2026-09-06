use super::*;

#[test]
fn codex_and_opencode_require_explicit_completion() {
    assert!(ExternalProviderObservationPolicy::for_provider("codex").uses_explicit_completion());
    assert!(ExternalProviderObservationPolicy::for_provider("opencode").uses_explicit_completion());
    assert!(!ExternalProviderObservationPolicy::for_provider("claude").uses_explicit_completion());
    assert!(ExternalProviderObservationPolicy::for_provider(" Codex ").uses_explicit_completion());
}

#[test]
fn completion_and_abort_statuses_settle_turns() {
    for (provider, text) in [
        ("codex", "codex task_complete\n{}"),
        (
            "codex",
            "codex event turn_aborted {\"type\":\"turn_aborted\"}",
        ),
        ("claude", "claude message completed\n{}"),
        ("opencode", "opencode message completed\n{}"),
    ] {
        let policy = ExternalProviderObservationPolicy::for_provider(provider);
        assert!(
            policy.latest_effective_turn_settles(&[ObservedExternalProviderTurn {
                role: ObservedExternalProviderTurnRole::Status,
                text: text.to_string(),
                provider_turn_id: None,
                observed_at_ms: None,
            }]),
            "{provider} status should settle"
        );
        assert_eq!(
            policy
                .observation_for_turn(&ObservedExternalProviderTurn {
                    role: ObservedExternalProviderTurnRole::Status,
                    text: text.to_string(),
                    provider_turn_id: None,
                    observed_at_ms: None,
                })
                .map(|observation| observation.settles_active_prompt),
            Some(true),
            "{provider} status should be marked as settling"
        );
        assert_eq!(
            policy
                .observation_for_turn(&ObservedExternalProviderTurn {
                    role: ObservedExternalProviderTurnRole::Status,
                    text: text.to_string(),
                    provider_turn_id: None,
                    observed_at_ms: None,
                })
                .map(|observation| observation.passive_telemetry),
            Some(false),
            "{provider} settling status should not be passive telemetry"
        );
    }
}

#[test]
fn completion_statuses_are_scoped_to_provider_policy() {
    for (provider, foreign_text) in [
        ("codex", "claude message completed\n{}"),
        ("codex", "opencode message completed\n{}"),
        ("claude", "codex task_complete\n{}"),
        (
            "claude",
            "codex event turn_aborted {\"type\":\"turn_aborted\"}",
        ),
        ("claude", "opencode message completed\n{}"),
        ("opencode", "codex task_complete\n{}"),
        ("opencode", "claude message completed\n{}"),
    ] {
        assert!(
            !ExternalProviderObservationPolicy::for_provider(provider)
                .latest_effective_turn_settles(&[ObservedExternalProviderTurn {
                    role: ObservedExternalProviderTurnRole::Status,
                    text: foreign_text.to_string(),
                    provider_turn_id: None,
                    observed_at_ms: None,
                }]),
            "{provider} policy must not settle from foreign marker {foreign_text:?}"
        );
    }
}

#[test]
fn status_prefix_markers_require_boundaries() {
    let codex = ExternalProviderObservationPolicy::for_provider("codex");
    assert!(codex.status_settles("codex task_complete\n{}"));
    assert!(!codex.status_settles("codex task_completed\n{}"));
    assert!(codex.status_is_passive_telemetry("codex token_count\n{}"));
    assert!(!codex.status_is_passive_telemetry("codex token_count_extra\n{}"));

    let claude = ExternalProviderObservationPolicy::for_provider("claude");
    assert!(claude.status_settles("claude message completed {}"));
    assert!(!claude.status_settles("claude message completedness {}"));
    assert!(claude.status_is_passive_telemetry("claude ai-title {}"));
    assert!(!claude.status_is_passive_telemetry("claude ai-title-extra {}"));

    let opencode = ExternalProviderObservationPolicy::for_provider("opencode");
    assert!(opencode.status_settles("opencode message completed {}"));
    assert!(!opencode.status_settles("opencode message completedness {}"));
}

#[test]
fn provider_policy_tolerates_legacy_provider_casing_and_whitespace() {
    let codex = ExternalProviderObservationPolicy::for_provider(" Codex ");
    assert!(codex.status_settles(" Codex task_complete\n{}"));
    assert!(codex.status_is_passive_telemetry(" CODEX token_count\n{}"));

    let claude = ExternalProviderObservationPolicy::for_provider(" CLAUDE ");
    assert!(claude.status_settles(" Claude message completed\n{}"));
    assert!(claude.status_is_passive_telemetry(" CLAUDE last-prompt {\"lastPrompt\":\"prompt\"}"));

    let opencode = ExternalProviderObservationPolicy::for_provider(" OpenCode ");
    assert!(opencode.status_settles(" OpenCode message completed\n{}"));
}

#[test]
fn codex_token_count_status_projects_provider_run_usage() {
    assert_eq!(
        ExternalProviderObservationPolicy::for_provider("codex").status_usage(
            " Codex token_count\n{\"info\":{\"total_token_usage\":{\"total_tokens\":42000},\"model_context_window\":128000}}"
        ),
        Some(ProviderRunTokenUsage {
            total_tokens: Some(42_000),
            last_tokens: Some(42_000),
            context_tokens: Some(42_000),
            context_window: Some(128_000),
        })
    );
    assert_eq!(
        ExternalProviderObservationPolicy::for_provider("codex").status_usage(
            "codex token_count\n{\"last\":{\"totalTokens\":160000},\"modelContextWindow\":128000}"
        ),
        Some(ProviderRunTokenUsage {
            total_tokens: Some(160_000),
            last_tokens: Some(160_000),
            context_tokens: None,
            context_window: Some(128_000),
        })
    );
    assert_eq!(
        ExternalProviderObservationPolicy::for_provider("claude")
            .status_usage("codex token_count\n{\"last\":{\"totalTokens\":42}}"),
        None
    );
}

#[test]
fn claude_passive_telemetry_does_not_hide_prior_completion() {
    let policy = ExternalProviderObservationPolicy::for_provider("claude");
    assert!(
        policy.turn_is_passive_telemetry(&ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::Status,
            text: "claude last-prompt {\"lastPrompt\":\"prompt\"}".to_string(),
            provider_turn_id: None,
            observed_at_ms: None,
        })
    );
    assert_eq!(
        policy
            .observation_for_turn(&ObservedExternalProviderTurn {
                role: ObservedExternalProviderTurnRole::Status,
                text: "claude last-prompt {\"lastPrompt\":\"prompt\"}".to_string(),
                provider_turn_id: None,
                observed_at_ms: None,
            })
            .map(|observation| observation.passive_telemetry),
        Some(true)
    );
    assert!(policy.latest_effective_turn_settles(&[
        ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::Status,
            text: "claude message completed\n{}".to_string(),
            provider_turn_id: None,
            observed_at_ms: None,
        },
        ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::Status,
            text: "claude ai-title {\"title\":\"Title\"}".to_string(),
            provider_turn_id: None,
            observed_at_ms: None,
        },
    ]));
}

#[test]
fn codex_token_count_is_passive_telemetry_and_does_not_settle() {
    let policy = ExternalProviderObservationPolicy::for_provider("codex");
    let token_count = ObservedExternalProviderTurn {
        role: ObservedExternalProviderTurnRole::Status,
        text: "codex token_count\n{\"info\":{\"total_token_usage\":{\"total_tokens\":42}}}"
            .to_string(),
        provider_turn_id: None,
        observed_at_ms: None,
    };

    assert!(policy.turn_is_passive_telemetry(&token_count));
    assert!(!policy.latest_effective_turn_settles(std::slice::from_ref(&token_count)));
    assert_eq!(
        policy
            .observation_for_turn(&token_count)
            .map(|observation| observation.passive_telemetry),
        Some(true)
    );
}

#[test]
fn normalized_observed_prompt_text_collapses_whitespace_and_ignores_empty() {
    assert_eq!(
        normalized_observed_prompt_text("  run   this\nnow\t"),
        Some("run this now".to_string())
    );
    assert_eq!(normalized_observed_prompt_text(" \n\t "), None);
}

#[test]
fn normalized_observed_prompt_text_ignores_generated_attachment_markup() {
    assert_eq!(
        normalized_observed_prompt_text(
            "inspect this\n<image name=[Image #1] path=\"/tmp/screenshot.png\"> </image>"
        ),
        Some("inspect this".to_string())
    );
    assert_eq!(
        normalized_observed_prompt_text(
            "read this <file name=\"notes.txt\" path=\"/tmp/notes.txt\"> </file> now"
        ),
        Some("read this now".to_string())
    );
}

#[test]
fn normalized_observed_prompt_text_ignores_provider_native_attachment_suffixes() {
    let prompt = "agent agent-2 message:\n\nReview the attached evidence.";

    assert_eq!(
        normalized_observed_prompt_text(&format!(
            "{prompt}Attachment: note.txt (text/plain) at data:text/plain;base64,SGVsbG8="
        )),
        Some("agent agent-2 message: Review the attached evidence.".to_string())
    );
    assert_eq!(
        normalized_observed_prompt_text(&format!(
            "{prompt}\nAttachment: note.txt (text/plain) at file:///tmp/note.txt\n\nfile contents"
        )),
        Some("agent agent-2 message: Review the attached evidence.".to_string())
    );
    assert_eq!(
        normalized_observed_prompt_text("Explain what the Attachment: label means."),
        Some("Explain what the Attachment: label means.".to_string())
    );
}

#[test]
fn observed_account_handoff_matches_only_the_current_request() {
    let envelope = "<chariox_context_handoff>Prior prompt and output</chariox_context_handoff> Provider/account switch: codex [a] -> codex [b]. <user_request>Reply SWITCHED</user_request>";
    assert_eq!(
        normalized_observed_prompt_text(envelope),
        Some("Reply SWITCHED".to_string())
    );
    for suffix in [
        "",
        "Attachment: note.txt (text/plain) at data:text/plain;base64,SGVsbG8=",
        "\nAttachment: note.txt (text/plain) at file:///tmp/note.txt\n\nfile contents",
    ] {
        for context in [
            "Prior prompt and output",
            "Prior prompt Attachment: old.txt (text/plain) at file:///tmp/old.txt\nold contents",
        ] {
            let observed = envelope.replace("Prior prompt and output", context);
            assert_eq!(
                normalized_observed_prompt_text(&format!("{observed}{suffix}")),
                Some("Reply SWITCHED".to_string())
            );
        }
    }
    for literal in [
        "<user_request>Keep literal markup</user_request>",
        "<chariox_context_handoff>Missing close <user_request>Do not strip</user_request>",
        "<chariox_context_handoff>context</chariox_context_handoff> Provider/account switch: codex <user_request>Missing close",
    ] {
        assert_eq!(normalized_observed_prompt_text(literal), Some(literal.to_string()));
    }
    let request = "Inspect </user_request>\nAttachment: quoted.txt (text/plain) at file:///tmp/quoted.txt\nThis is quoted transcript text.";
    let ambiguous_legacy = envelope.replace("Reply SWITCHED", request);
    assert_eq!(
        super::strip_observed_account_handoff(&ambiguous_legacy),
        ambiguous_legacy
    );
    let echoed = crate::provider::encode_account_handoff("Prior context", request);
    assert_eq!(
        normalized_observed_prompt_text(&echoed),
        normalized_observed_prompt_text(request)
    );
}

#[test]
fn account_handoff_survives_provider_transcript_cleaning() {
    let request = "Reply SWITCHED. Do not use tools.";
    let framed = crate::provider::encode_account_handoff(
        "Previous prompt\n## My request:\nEarlier text\n<runtime-instructions>old context</runtime-instructions>",
        request,
    );
    for suffix in [
        "",
        "\nAttachment: note.txt (text/plain) at file:///note.txt",
    ] {
        let cleaned = clean_provider_prompt(format!("{framed}{suffix}"))
            .expect("The real user request must survive provider cleaning");
        assert_eq!(
            normalized_observed_prompt_text(&cleaned),
            Some(request.to_string())
        );
    }
}

#[test]
fn observed_prompt_text_ignores_chariox_generated_runtime_context() {
    let observed = "run the check <runtime-instructions>generated</runtime-instructions> \
        <native-permission-instructions>generated</native-permission-instructions>";
    assert_eq!(
        clean_provider_prompt(observed.to_string()),
        Some("run the check".to_string())
    );
    assert_eq!(
        normalized_observed_prompt_text(observed),
        Some("run the check".to_string())
    );
}

#[test]
fn clean_provider_prompt_strips_system_wrappers_and_compacts_request_text() {
    assert_eq!(
        clean_provider_prompt(
            "# AGENTS.md instructions for /repo\n\n<INSTRUCTIONS>hidden</INSTRUCTIONS>".to_string()
        ),
        None
    );
    assert_eq!(
        clean_provider_prompt("<environment_context>\n  <cwd>/repo</cwd>".to_string()),
        None
    );
    assert_eq!(
        clean_provider_prompt(
            "<recommended_plugins>generated</recommended_plugins>\n# AGENTS.md instructions for /repo\n<environment_context><cwd>/repo</cwd></environment_context>"
                .to_string()
        ),
        None
    );
    assert_eq!(
        clean_provider_prompt(
            "preamble\n## My request for Codex:\n  run   the\ncheck  ".to_string()
        ),
        Some("run the check".to_string())
    );
    assert_eq!(
        clean_provider_prompt("meta\n## My request:\n  use   provider form  ".to_string()),
        Some("use provider form".to_string())
    );
}

#[test]
fn observed_turn_text_cleanup_is_role_specific() {
    assert_eq!(
        clean_observed_turn_text(Some("user"), "  ask   this\nnow ".to_string()),
        Some("ask this now".to_string())
    );
    assert_eq!(
        clean_observed_turn_text(Some("assistant"), "  final answer\n".to_string()),
        Some("final answer".to_string())
    );
    assert_eq!(
        clean_observed_turn_text(Some("status"), "  codex task_complete {}\n".to_string()),
        Some("codex task_complete {}".to_string())
    );
    assert_eq!(
        clean_observed_turn_text(Some("unknown"), "text".to_string()),
        None
    );
}

#[test]
fn text_from_content_extracts_provider_content_shapes() {
    assert_eq!(
        text_from_content(&serde_json::json!("plain text")),
        Some("plain text".to_string())
    );
    assert_eq!(
        text_from_content(&serde_json::json!([
            {"type": "text", "text": "first"},
            {"type": "image", "url": "ignored"},
            {"type": "text", "content": "second"},
            {"value": "third"}
        ])),
        Some("first\nsecond\nthird".to_string())
    );
    assert_eq!(
        text_from_content(&serde_json::json!({"content": "object content"})),
        Some("object content".to_string())
    );
}
