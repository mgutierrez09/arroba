use std::time::Duration;

mod account_credential;
mod claude;
mod claude_runtime;
pub(crate) use claude_runtime::usage::claude_status_line_usage_snapshot;
mod codex;
mod codex_client;
mod codex_runtime;
mod command_catalog;
mod credential_environment;
#[cfg(test)]
pub(crate) use credential_environment::{
    record_provider_credential_delivery_for_test, ProviderCredentialDeliveryProbe,
};
mod executable_resolution;
mod external_observation;
mod launch_contract;
mod managed_isolation;
mod mcp_proxy;
mod opencode;
mod opencode_binding;
mod opencode_client;
mod opencode_runtime;
mod process_info;
mod prompt_signals;
mod registry;
mod run_actor;
mod runtime_run;
mod service;
mod types;
mod workspace_live_sync_policy;
mod workspace_write_fence;

#[cfg(test)]
pub(crate) use account_credential::provider_account_credential_id;
pub(crate) use account_credential::{
    resolve_provider_account_credentials_for_launch, store_provider_account_credential,
    validate_provider_account_credential_input,
};
pub(crate) use claude::ensure_claude_native_hidden_context_fits;
pub(crate) use claude::probe_claude_account_usage;
pub use claude::{claude_provider_catalog, plan_claude_launch, resolve_claude_executable};
pub(crate) use claude_runtime::ClaudeRuntimeState;
pub use codex::{
    codex_catalog_endpoint, logout_codex, plan_codex_launch, resolve_codex_executable,
};
pub(crate) use codex::{
    ensure_codex_account_endpoint, invalidate_codex_account_endpoint,
    shutdown_codex_account_endpoints,
};
pub use codex_client::{
    CodexClient, CodexNotification, CodexRunSelection, CodexSocket, CodexThread,
    CodexThreadStartResponse, ProviderAuthStatus, ProviderLoginStart,
};
pub use codex_runtime::CodexRuntimeState;
pub use command_catalog::{
    default_provider_command_catalogs, ProviderCommandCatalog, ProviderCommandCatalogDiscovery,
    ProviderCommandCatalogSource, ProviderCommandDescriptor,
};
pub(crate) use credential_environment::ProviderCredentialEnvironment;
pub(crate) use external_observation::{
    clean_observed_turn_text, clean_provider_prompt, normalized_observed_prompt_text,
    observed_role, text_from_content, ExternalProviderObservationPolicy,
    ObservedExternalProviderTurn, ObservedExternalProviderTurnRole,
};
pub use launch_contract::{
    canonical_external_provider_session_id, canonical_profile_external_provider_session_id,
    default_provider_control_capabilities, external_provider_import_model,
    external_provider_session_providers, normalize_provider_resume_model,
    provider_resume_failure_notice, provider_uses_inferred_runtime_mcp_binding, AgentExecutionMode,
    AgentPermissionLevel, ExternalProviderImportMetadata, ExternalProviderObservedCursor,
    LaunchProviderRequest, ProviderLaunchResult, ProviderResumeState, ProviderWriteAccessMode,
    RuntimeMcpBinding,
};
#[cfg(test)]
pub(crate) use managed_isolation::MANAGED_PROVIDER_ISOLATION_ENV;
pub(crate) use managed_isolation::{
    apply_managed_provider_isolation, command_from_provider_launch,
    managed_isolated_utility_command, managed_provider_control_env_remove,
    managed_provider_isolation_required,
};
pub(crate) use mcp_proxy::{
    dispatch_provider_mcp_proxy_request, shutdown_provider_mcp_proxy_session,
};
pub(crate) use opencode::{
    ensure_opencode_account_endpoint, invalidate_opencode_account_endpoint,
    shutdown_opencode_account_endpoints,
};
pub use opencode::{opencode_catalog_endpoint, plan_opencode_launch, resolve_opencode_executable};
pub use opencode_client::{
    OpenCodeClient, OpenCodeEvent, OpenCodeEventSubscription, OpenCodeMessage,
    OpenCodeMessageCacheTokens, OpenCodeMessageInfo, OpenCodeMessageTime, OpenCodeMessageTokens,
    OpenCodePart, OpenCodePartTime, OpenCodeProviderCatalog, OpenCodeProviderInfo,
    OpenCodeProviderModel, OpenCodeProviderModelLimit, OpenCodeSelectedModel,
    OpenCodeSessionSnapshot, OpenCodeToolState,
};
pub use process_info::{ProviderProcessInfo, ProviderProcessStatus};
pub(crate) use prompt_signals::{
    classify_provider_substitutable_failure_text, classify_provider_terminal_failure_output_text,
    classify_provider_terminal_failure_text, provider_retry_status,
    PROVIDER_CONNECTION_RETRY_MERGE_KEY,
};
pub use prompt_signals::{
    ProviderAssistantCompletion, ProviderPromptChunk, ProviderPromptSignalBatch,
};
pub use registry::{AgentEndpointAdapter, ProviderRegistry};
pub(crate) use run_actor::{
    FinishedProviderOutputPollJob, FinishedProviderPromptAbortJob, FinishedProviderPromptSubmitJob,
    ProviderNativeInteractionBridge, ProviderNativeInteractionResolution,
    ProviderPromptSubmitAcknowledgement, ProviderRunActorCompletionSignal, ProviderRunActorMailbox,
    ProviderRunOperationLanes,
};
pub(crate) use runtime_run::{
    projected_leased_provider_run_id, worker_provider_run_id_from_projected_leased_id,
};
pub use runtime_run::{ProviderRunTokenUsage, RuntimeProviderRun};
pub use service::{ProviderProcessService, ProviderProcessServiceStore};
pub(crate) use service::{ProviderRunLivenessReconciliation, ProviderRuntimeBinding};
pub(crate) use types::provider_workspace_live_sync_mode_for_session;
pub use types::{
    AgentEndpointMode, ControlCapability, ControlCapabilityMode, ControlOperation,
    ProviderClientInterface, ProviderRunState,
};
pub(crate) use workspace_live_sync_policy::{
    native_tui_hidden_instructions_block, NATIVE_TUI_HIDDEN_INSTRUCTIONS_END,
    NATIVE_TUI_HIDDEN_INSTRUCTIONS_START, WORKSPACE_LIVE_SYNC_INSTRUCTIONS_SOURCE_PATH,
};

pub(crate) fn adapter_key_for_provider(provider: &str) -> &str {
    match provider {
        "default" => "opencode",
        "claude-headless" | "claude-p" => "claude",
        value => value,
    }
}

pub(crate) fn provider_id_for_launch(provider: &str) -> &str {
    match provider {
        "default" => "opencode",
        value => value,
    }
}

pub(crate) fn canonical_provider_family(provider: &str) -> Option<&'static str> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "codex" => Some("codex"),
        "opencode" => Some("opencode"),
        "claude" | "claude-headless" | "claude-p" => Some("claude"),
        _ => None,
    }
}

pub(crate) fn provider_run_is_claude_headless(run: &RuntimeProviderRun) -> bool {
    run.adapter_key() == "claude" && run.provider() == "claude-headless"
}

pub(crate) fn provider_run_uses_claude_native_bridge(run: &RuntimeProviderRun) -> bool {
    run.adapter_key() == "claude"
        && (!run.client_interface().is_chariox() || provider_run_is_claude_headless(run))
}

pub(crate) fn provider_run_uses_structured_prompt_io(run: &RuntimeProviderRun) -> bool {
    run.adapter_key() == "codex"
        || (run.adapter_key() == "claude"
            && run.client_interface().is_chariox()
            && !provider_run_uses_claude_native_bridge(run))
        || run.adapter_key() == "opencode"
        || (run.adapter_key() == "dev-stub" && run.provider() == "slow-structured")
}

pub(crate) fn provider_run_supports_selection_sync(run: &RuntimeProviderRun) -> bool {
    run.adapter_key() == "opencode"
}

pub(crate) fn provider_run_refreshes_selection_on_read(run: &RuntimeProviderRun) -> bool {
    provider_run_supports_selection_sync(run) && run.client_interface().is_chariox()
}

pub(crate) fn provider_run_waits_for_workflow_publication_completion(
    run: &RuntimeProviderRun,
) -> bool {
    matches!(run.adapter_key(), "codex" | "claude")
}

pub(crate) fn provider_run_reuses_run_for_mcp_continuation_reload(
    run: &RuntimeProviderRun,
) -> bool {
    run.adapter_key() == "opencode"
}

pub(crate) fn provider_adapter_supports_policy_reload(adapter_key: &str) -> bool {
    matches!(adapter_key, "claude" | "codex" | "opencode")
}

pub(crate) fn provider_batch_launch_concurrency_limit(
    adapter_key: &str,
    provider: &str,
    default_limit: usize,
) -> usize {
    if adapter_key == "dev-stub" || provider == "dev-stub" {
        return 64;
    }
    if matches!(adapter_key, "codex" | "opencode" | "claude" | "claude-code")
        || matches!(provider, "codex" | "opencode" | "claude" | "claude-code")
    {
        return 16;
    }
    default_limit
}

pub(crate) fn retain_public_inventory_providers(providers: &mut Vec<String>) {
    retain_public_inventory_providers_with_dev_stub_policy(
        providers,
        dev_stub_public_inventory_enabled(),
    );
}

pub(crate) fn dev_stub_public_inventory_enabled() -> bool {
    std::env::var("CHARIOX_PROVIDER_DEV_STUB")
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
}

fn retain_public_inventory_providers_with_dev_stub_policy(
    providers: &mut Vec<String>,
    include_dev_stub: bool,
) {
    if !include_dev_stub {
        providers.retain(|provider| provider != "dev-stub");
    }
}

pub(crate) fn provider_run_supports_policy_reload(run: &RuntimeProviderRun) -> bool {
    provider_adapter_supports_policy_reload(run.adapter_key())
}

pub(crate) fn provider_run_uses_runtime_structured_utility_prompt(
    run: &RuntimeProviderRun,
) -> bool {
    run.adapter_key() == "claude" && run.client_interface().is_chariox()
}

pub(crate) fn run_blocking_provider_utility_prompt(
    run: &RuntimeProviderRun,
    visible_user_prompt: &str,
    hidden_system_context: &str,
    timeout: Duration,
    operation: &'static str,
) -> Result<String, crate::error::DaemonError> {
    match run.adapter_key() {
        "codex" => codex_runtime::run_codex_utility_prompt(
            run,
            visible_user_prompt,
            hidden_system_context,
            timeout,
        ),
        "opencode" => opencode_binding::run_opencode_utility_prompt(
            run,
            visible_user_prompt,
            hidden_system_context,
            timeout,
        ),
        adapter_key => Err(crate::error::DaemonError::LocalTransport {
            operation,
            message: format!(
                "agent utility prompts are not supported for provider adapter `{adapter_key}`"
            ),
        }),
    }
}
pub(crate) use workspace_write_fence::{
    apply_workspace_write_fence, workspace_write_fence_active, workspace_write_fence_backend,
    workspace_write_fence_supported, workspace_write_fence_unavailable_reason,
};

#[cfg(test)]
mod tests {
    use super::{
        adapter_key_for_provider, canonical_provider_family,
        provider_adapter_supports_policy_reload, provider_batch_launch_concurrency_limit,
        provider_id_for_launch, provider_run_is_claude_headless,
        provider_run_refreshes_selection_on_read,
        provider_run_reuses_run_for_mcp_continuation_reload, provider_run_supports_policy_reload,
        provider_run_supports_selection_sync, provider_run_uses_claude_native_bridge,
        provider_run_uses_runtime_structured_utility_prompt,
        provider_run_waits_for_workflow_publication_completion, retain_public_inventory_providers,
        retain_public_inventory_providers_with_dev_stub_policy,
        run_blocking_provider_utility_prompt, AgentEndpointMode, LaunchProviderRequest,
        ProviderClientInterface, ProviderLaunchResult, RuntimeProviderRun,
    };

    #[test]
    fn claude_headless_provider_mode_is_provider_policy() {
        let headless = provider_run("claude", "claude-headless");
        let regular = provider_run("claude", "claude");

        assert!(provider_run_is_claude_headless(&headless));
        assert!(provider_run_uses_claude_native_bridge(&headless));
        assert!(!provider_run_is_claude_headless(&regular));
        assert!(!provider_run_uses_claude_native_bridge(&regular));
    }

    #[test]
    fn provider_launch_identity_normalization_is_provider_policy() {
        assert_eq!(adapter_key_for_provider("default"), "opencode");
        assert_eq!(provider_id_for_launch("default"), "opencode");
        assert_eq!(adapter_key_for_provider("claude-headless"), "claude");
        assert_eq!(provider_id_for_launch("claude-headless"), "claude-headless");
        assert_eq!(adapter_key_for_provider("codex"), "codex");
        assert_eq!(provider_id_for_launch("codex"), "codex");
    }

    #[test]
    fn canonical_provider_family_normalizes_provider_modes() {
        assert_eq!(canonical_provider_family(" CODEX "), Some("codex"));
        assert_eq!(canonical_provider_family("opencode"), Some("opencode"));
        assert_eq!(canonical_provider_family("claude-headless"), Some("claude"));
        assert_eq!(canonical_provider_family("claude-p"), Some("claude"));
        assert_eq!(canonical_provider_family("unknown"), None);
    }

    #[test]
    fn opencode_selection_refresh_is_provider_policy() {
        let chariox_opencode = provider_run("opencode", "opencode");
        let native_opencode = provider_run_with_client_interface(
            "opencode",
            "opencode",
            ProviderClientInterface::NativeTui,
        );
        let codex = provider_run("codex", "codex");

        assert!(provider_run_supports_selection_sync(&chariox_opencode));
        assert!(provider_run_supports_selection_sync(&native_opencode));
        assert!(!provider_run_supports_selection_sync(&codex));

        assert!(provider_run_refreshes_selection_on_read(&chariox_opencode));
        assert!(!provider_run_refreshes_selection_on_read(&native_opencode));
        assert!(!provider_run_refreshes_selection_on_read(&codex));
    }

    #[test]
    fn workflow_publication_completion_wait_is_provider_policy() {
        let structured_claude = provider_run("claude", "claude");
        let headless_claude = provider_run("claude", "claude-headless");
        let native_claude = provider_run_with_client_interface(
            "claude",
            "claude",
            ProviderClientInterface::NativeTui,
        );
        let codex = provider_run("codex", "codex");
        let opencode = provider_run("opencode", "opencode");

        assert!(provider_run_waits_for_workflow_publication_completion(
            &structured_claude
        ));
        assert!(provider_run_waits_for_workflow_publication_completion(
            &headless_claude
        ));
        assert!(provider_run_waits_for_workflow_publication_completion(
            &native_claude
        ));
        assert!(provider_run_waits_for_workflow_publication_completion(
            &codex
        ));
        assert!(!provider_run_waits_for_workflow_publication_completion(
            &opencode
        ));
    }

    #[test]
    fn mcp_continuation_reload_run_reuse_is_provider_policy() {
        let opencode = provider_run("opencode", "opencode");
        let codex = provider_run("codex", "codex");
        let claude = provider_run("claude", "claude");

        assert!(provider_run_reuses_run_for_mcp_continuation_reload(
            &opencode
        ));
        assert!(!provider_run_reuses_run_for_mcp_continuation_reload(&codex));
        assert!(!provider_run_reuses_run_for_mcp_continuation_reload(
            &claude
        ));
    }

    #[test]
    fn provider_policy_reload_support_is_provider_policy() {
        for adapter in ["claude", "codex", "opencode"] {
            assert!(
                provider_adapter_supports_policy_reload(adapter),
                "{adapter} should relaunch when launch-time runtime config changes"
            );
            assert!(
                provider_run_supports_policy_reload(&provider_run(adapter, adapter)),
                "{adapter} runs should relaunch when launch-time runtime config changes"
            );
        }
        for adapter in ["dev-stub", "unknown"] {
            assert!(
                !provider_adapter_supports_policy_reload(adapter),
                "{adapter} should not use provider relaunch policy"
            );
            assert!(
                !provider_run_supports_policy_reload(&provider_run(adapter, adapter)),
                "{adapter} runs should not use provider relaunch policy"
            );
        }
    }

    #[test]
    fn provider_batch_launch_concurrency_limit_is_provider_policy() {
        assert_eq!(
            provider_batch_launch_concurrency_limit("codex", "codex", 99),
            16
        );
        assert_eq!(
            provider_batch_launch_concurrency_limit("default-adapter", "opencode", 99),
            16
        );
        assert_eq!(
            provider_batch_launch_concurrency_limit("dev-stub", "codex", 99),
            64
        );
        assert_eq!(
            provider_batch_launch_concurrency_limit("custom", "custom", 99),
            99
        );
    }

    #[test]
    fn public_inventory_provider_visibility_is_provider_policy() {
        let mut providers = vec![
            "codex".to_string(),
            "dev-stub".to_string(),
            "opencode".to_string(),
        ];

        retain_public_inventory_providers(&mut providers);

        assert_eq!(providers, vec!["codex", "opencode"]);
    }

    #[test]
    fn public_inventory_provider_visibility_can_include_dev_stub_for_drills() {
        let mut providers = vec![
            "codex".to_string(),
            "dev-stub".to_string(),
            "opencode".to_string(),
        ];

        retain_public_inventory_providers_with_dev_stub_policy(&mut providers, true);

        assert_eq!(providers, vec!["codex", "dev-stub", "opencode"]);
    }

    #[test]
    fn runtime_structured_utility_prompt_is_provider_policy() {
        let structured_claude = provider_run("claude", "claude");
        let native_claude = provider_run_with_client_interface(
            "claude",
            "claude",
            ProviderClientInterface::NativeTui,
        );
        let codex = provider_run("codex", "codex");
        let opencode = provider_run("opencode", "opencode");

        assert!(provider_run_uses_runtime_structured_utility_prompt(
            &structured_claude
        ));
        assert!(!provider_run_uses_runtime_structured_utility_prompt(
            &native_claude
        ));
        assert!(!provider_run_uses_runtime_structured_utility_prompt(&codex));
        assert!(!provider_run_uses_runtime_structured_utility_prompt(
            &opencode
        ));
    }

    #[test]
    fn blocking_utility_prompt_reports_unsupported_adapter() {
        let run = provider_run("dev-stub", "utility-unsupported");
        let error = run_blocking_provider_utility_prompt(
            &run,
            "visible",
            "hidden",
            std::time::Duration::from_secs(1),
            "test utility",
        )
        .expect_err("unsupported adapter should fail before provider I/O");

        match error {
            crate::error::DaemonError::LocalTransport { operation, message } => {
                assert_eq!(operation, "test utility");
                assert!(message.contains("dev-stub"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    fn provider_run(adapter_key: &str, provider: &str) -> RuntimeProviderRun {
        provider_run_with_client_interface(adapter_key, provider, ProviderClientInterface::Chariox)
    }

    fn provider_run_with_client_interface(
        adapter_key: &str,
        provider: &str,
        client_interface: ProviderClientInterface,
    ) -> RuntimeProviderRun {
        let request =
            LaunchProviderRequest::new("session-1", adapter_key, provider, "default", "model")
                .with_client_interface(client_interface);
        RuntimeProviderRun::new(
            format!("provider-run-{adapter_key}-{provider}"),
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: provider.to_string(),
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
}
