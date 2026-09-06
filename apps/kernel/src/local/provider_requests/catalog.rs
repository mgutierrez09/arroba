use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::provider::{
    claude_provider_catalog, default_provider_command_catalogs, resolve_claude_executable,
    resolve_opencode_executable, CodexClient, OpenCodeClient, OpenCodeProviderCatalog,
    OpenCodeProviderInfo, ProviderAuthStatus,
};
use chariox_relay::protocol::RelayMachinePresence;
use std::collections::BTreeMap;
use std::process::Command;
use std::thread;
use std::time::Duration;

use super::super::api::{
    GetProviderAuthStatusRequest, GetProviderCatalogRequest, LocalDaemonResponse,
    LogoutProviderRequest, ProviderCatalogExecutionLocation, StartProviderLoginRequest,
};
use super::blocking::block_on_relay_query;

pub(crate) const PROVIDER_CATALOG_CACHE_TTL: Duration = Duration::from_secs(5);

pub(crate) fn provider_command_catalogs_response() -> Result<LocalDaemonResponse, DaemonError> {
    Ok(LocalDaemonResponse::ProviderCommandCatalogs {
        catalogs: default_provider_command_catalogs(),
    })
}

pub(crate) fn load_provider_catalog(
    config: DaemonConfig,
    registry: crate::account_profile::ProviderAccountProfileRegistry,
    owner_user_id: String,
    request: GetProviderCatalogRequest,
) -> Result<OpenCodeProviderCatalog, DaemonError> {
    if config.provider_catalog_read_delay_ms > 0 {
        thread::sleep(Duration::from_millis(config.provider_catalog_read_delay_ms));
    }

    let mut catalogs = vec![claude_provider_catalog()];
    if crate::provider::dev_stub_public_inventory_enabled() {
        catalogs.push(dev_stub_provider_catalog());
    }
    let mut source_errors = Vec::new();

    let selected_profiles = resolve_catalog_profiles(&registry, &owner_user_id, &request)?;
    let opencode_profile = selected_profiles
        .get("opencode")
        .expect("OpenCode default profile is migrated");
    let opencode_environment =
        registry.resolve_environment(&owner_user_id, "opencode", &opencode_profile.profile_id)?;
    match crate::provider::ensure_opencode_account_endpoint(
        &owner_user_id,
        &opencode_profile.profile_id,
        opencode_environment,
    ) {
        Ok(endpoint) => match OpenCodeClient::new("catalog", endpoint.as_str()) {
            Ok(client) => match client.provider_catalog() {
                Ok(catalog) => catalogs.push(opencode_backend_catalog(catalog)),
                Err(error) => source_errors.push(format!("opencode catalog request: {error}")),
            },
            Err(error) => source_errors.push(format!("opencode client: {error}")),
        },
        Err(error) => source_errors.push(format!("opencode endpoint: {error}")),
    }
    let codex_profile = selected_profiles
        .get("codex")
        .expect("Codex default profile is migrated");
    let codex_environment =
        registry.resolve_environment(&owner_user_id, "codex", &codex_profile.profile_id)?;
    match crate::provider::ensure_codex_account_endpoint(
        &owner_user_id,
        &codex_profile.profile_id,
        codex_environment,
    ) {
        Ok(endpoint) => match CodexClient::new("catalog", endpoint.as_str()) {
            Ok(client) => match client.provider_catalog() {
                Ok(catalog) => catalogs.push(catalog),
                Err(error) => source_errors.push(format!("codex catalog request: {error}")),
            },
            Err(error) => source_errors.push(format!("codex client: {error}")),
        },
        Err(error) => source_errors.push(format!("codex endpoint: {error}")),
    }

    let remote_machines = if config.relay_url.is_some() && config.relay_token.is_some() {
        match block_on_relay_query(crate::transport::relay_discovery::list_live_machines(
            &config,
        )) {
            Ok(machines) => machines,
            Err(error) => {
                source_errors.push(format!("relay live machines: {error}"));
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    let approved_remote_machines =
        approved_live_remote_machines(&remote_machines, &config.host_machine_id);
    if !source_errors.is_empty() {
        crate::logging::warn_with_fields(
            "daemon.local",
            "Some provider catalog sources were unavailable",
            serde_json::json!({
                "source_errors": &source_errors,
            }),
        );
    }

    let mut catalog = merge_provider_catalogs(catalogs)
        .or_else(|| {
            remote_only_provider_catalog(&approved_remote_machines, &config.host_machine_id)
        })
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "get_provider_catalog",
            message: if source_errors.is_empty() {
                "no provider catalog sources were reachable".to_string()
            } else {
                format!(
                    "no provider catalog sources were reachable: {}",
                    source_errors.join("; ")
                )
            },
        })?;
    annotate_remote_machine_providers(
        &mut catalog,
        &approved_remote_machines,
        &config.host_machine_id,
    );
    crate::logging::info_with_fields(
        "daemon.local",
        "Retrieved merged provider catalog",
        serde_json::json!({
            "provider_count": catalog.all.len(),
            "model_count": catalog.all.iter().map(|provider| provider.models.len()).sum::<usize>(),
            "remote_provider_count": catalog.all.iter().filter(|provider| !provider.remote_machine_aliases.is_empty()).count(),
            "connected": &catalog.connected,
        }),
    );
    Ok(catalog)
}

fn resolve_catalog_profiles(
    registry: &crate::account_profile::ProviderAccountProfileRegistry,
    owner_user_id: &str,
    request: &GetProviderCatalogRequest,
) -> Result<BTreeMap<String, crate::account_profile::ProviderAccountProfile>, DaemonError> {
    let focus_provider = match request.provider.as_deref() {
        Some(provider) => Some(
            crate::provider::canonical_provider_family(provider).ok_or_else(|| {
                DaemonError::LocalTransport {
                    operation: "get_provider_catalog",
                    message: format!("unsupported provider `{provider}` in catalog request"),
                }
            })?,
        ),
        None => None,
    };
    let mut overrides = BTreeMap::new();
    for (provider, profile_id) in &request.account_profiles {
        let provider = crate::provider::canonical_provider_family(provider).ok_or_else(|| {
            DaemonError::LocalTransport {
                operation: "get_provider_catalog",
                message: format!("unsupported provider `{provider}` in account profile selection"),
            }
        })?;
        if overrides
            .insert(provider.to_string(), profile_id.clone())
            .is_some()
        {
            return Err(DaemonError::LocalTransport {
                operation: "get_provider_catalog",
                message: format!(
                    "provider `{provider}` has more than one account profile selection"
                ),
            });
        }
    }

    let mut selected = BTreeMap::new();
    for provider in ["codex", "claude", "opencode"] {
        let profile = if let Some(profile_id) = overrides.get(provider) {
            registry.get(owner_user_id, provider, profile_id)?
        } else {
            registry
                .list(owner_user_id, Some(provider))?
                .into_iter()
                .find(|profile| profile.is_default)
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "get_provider_catalog",
                    message: format!(
                        "provider `{provider}` does not have a registered default account profile"
                    ),
                })?
        };
        if focus_provider == Some(provider) {
            validate_catalog_materialization(&profile, &request.execution_location)?;
        }
        selected.insert(provider.to_string(), profile);
    }
    Ok(selected)
}

fn validate_catalog_materialization(
    profile: &crate::account_profile::ProviderAccountProfile,
    location: &ProviderCatalogExecutionLocation,
) -> Result<(), DaemonError> {
    let target = match location {
        ProviderCatalogExecutionLocation::Local => return Ok(()),
        ProviderCatalogExecutionLocation::Worker { kernel_ref } => (
            crate::account_profile::ProviderAccountMaterializationTargetKind::Worker,
            kernel_ref.as_str(),
        ),
        ProviderCatalogExecutionLocation::Slice { slice_ref } => (
            crate::account_profile::ProviderAccountMaterializationTargetKind::Slice,
            slice_ref.as_str(),
        ),
    };
    let materialized = profile.materializations.iter().any(|status| {
        status.target_kind == target.0
            && status.target_ref == target.1
            && status.state
                == crate::account_profile::ProviderAccountMaterializationState::Materialized
    });
    if materialized {
        return Ok(());
    }
    Err(DaemonError::LocalTransport {
        operation: "get_provider_catalog",
        message: format!(
            "account profile `{}` is not materialized at the selected execution location",
            profile.profile_id
        ),
    })
}

pub(crate) fn provider_auth_status_response(
    registry: &crate::account_profile::ProviderAccountProfileRegistry,
    owner_user_id: &str,
    request: GetProviderAuthStatusRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let profile = registry.get(owner_user_id, &request.provider, &request.account_profile)?;
    let environment =
        registry.resolve_environment(owner_user_id, &request.provider, &profile.profile_id)?;
    match crate::provider::canonical_provider_family(&request.provider) {
        Some("codex") => {
            let endpoint = crate::provider::ensure_codex_account_endpoint(
                owner_user_id,
                &profile.profile_id,
                environment,
            )?;
            let client = CodexClient::new("provider-auth", &endpoint)?;
            let status = client.auth_status(&profile.profile_id)?;
            update_profile_auth_observation(registry, owner_user_id, &status)?;
            Ok(LocalDaemonResponse::ProviderAuthStatus { status })
        }
        Some("claude") => Ok(LocalDaemonResponse::ProviderAuthStatus {
            status: {
                let status =
                    claude_auth_status(&request.provider, &profile.profile_id, &environment)?;
                update_profile_auth_observation(registry, owner_user_id, &status)?;
                status
            },
        }),
        Some("opencode") => Ok(LocalDaemonResponse::ProviderAuthStatus {
            status: {
                let status = opencode_auth_status(&profile.profile_id, &environment)?;
                update_profile_auth_observation(registry, owner_user_id, &status)?;
                registry.update_services(
                    owner_user_id,
                    "opencode",
                    &profile.profile_id,
                    inspect_opencode_services(&environment),
                )?;
                status
            },
        }),
        _ => Err(unsupported_auth_provider(
            "get_provider_auth_status",
            &request.provider,
        )),
    }
}

pub(crate) fn refresh_provider_account_profile_response(
    registry: &crate::account_profile::ProviderAccountProfileRegistry,
    owner_user_id: &str,
    provider: &str,
    account_profile: &str,
) -> Result<crate::account_profile::ProviderAccountProfile, DaemonError> {
    let profile = registry.get(owner_user_id, provider, account_profile)?;
    let environment = registry.resolve_environment(owner_user_id, provider, &profile.profile_id)?;
    let provider_family = crate::provider::canonical_provider_family(provider);
    let opencode_services =
        (provider_family == Some("opencode")).then(|| inspect_opencode_services(&environment));
    let (status, usage) = match provider_family {
        Some("codex") => {
            let endpoint = crate::provider::ensure_codex_account_endpoint(
                owner_user_id,
                &profile.profile_id,
                environment,
            )?;
            let client = CodexClient::new("provider-account-refresh", endpoint)?;
            (
                client.auth_status(&profile.profile_id)?,
                client.usage_snapshot(&profile.profile_id)?,
            )
        }
        Some("claude") => {
            let status = claude_auth_status(provider, &profile.profile_id, &environment)?;
            let usage = if status.auth_state == "authenticated" {
                let executable = resolve_claude_executable()?;
                crate::provider::probe_claude_account_usage(
                    &executable,
                    &profile.profile_id,
                    &environment,
                )?
            } else {
                profile
                    .usage
                    .clone()
                    .reconciled_freshness(crate::session::unix_epoch_ms())
            };
            (status, usage)
        }
        Some("opencode") => (
            opencode_auth_status(&profile.profile_id, &environment)?,
            opencode_usage_snapshot(&profile.profile_id, &environment),
        ),
        _ => {
            return Err(unsupported_auth_provider(
                "refresh provider account",
                provider,
            ));
        }
    };
    let updated = registry.update_observation(
        owner_user_id,
        &status.provider,
        &status.account_profile,
        auth_state_from_status(&status.auth_state),
        status.identity_summary,
        status.plan,
        status.detected_version,
        Some(usage),
    )?;
    if let Some(services) = opencode_services {
        registry.update_services(owner_user_id, "opencode", &updated.profile_id, services)
    } else {
        Ok(updated)
    }
}

fn claude_auth_status(
    provider: &str,
    account_profile: &str,
    environment: &BTreeMap<String, String>,
) -> Result<ProviderAuthStatus, DaemonError> {
    let executable = resolve_claude_executable()?;
    let mut command = crate::provider::managed_isolated_utility_command(
        executable.display().to_string(),
        vec![
            "auth".to_string(),
            "status".to_string(),
            "--json".to_string(),
        ],
        environment.clone(),
        None,
        "claude:auth-status",
    )?;
    for name in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_BASE_URL",
        "ANTHROPIC_CUSTOM_HEADERS",
    ] {
        command.env_remove(name);
    }
    let output = command
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "get_provider_auth_status",
            message: format!("failed to run Claude auth status: {error}"),
        })?;
    if !output.status.success() {
        return Ok(ProviderAuthStatus {
            provider: provider.to_string(),
            auth_state: "not_logged_in".to_string(),
            account_profile: account_profile.to_string(),
            identity_summary: None,
            plan: None,
            login_hint: Some("Run `claude auth login` to authenticate Claude Code.".to_string()),
            detected_version: claude_version().ok(),
        });
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|error| DaemonError::LocalTransport {
            operation: "get_provider_auth_status",
            message: format!("Claude auth status returned invalid JSON: {error}"),
        })?;
    Ok(claude_auth_status_from_value(
        provider,
        account_profile,
        &value,
        claude_version().ok(),
    ))
}

fn opencode_auth_status(
    account_profile: &str,
    environment: &BTreeMap<String, String>,
) -> Result<ProviderAuthStatus, DaemonError> {
    let executable = resolve_opencode_executable()?;
    let mut command = crate::provider::managed_isolated_utility_command(
        executable.display().to_string(),
        vec!["auth".to_string(), "list".to_string()],
        environment.clone(),
        None,
        "opencode:auth-status",
    )?;
    remove_account_auth_environment(&mut command, "opencode");
    let output = command
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "get_provider_auth_status",
            message: format!("failed to run OpenCode auth list: {error}"),
        })?;
    let credential_inspection = inspect_opencode_credentials(environment);
    let has_credentials =
        output.status.success() && credential_inspection == OpenCodeCredentialInspection::Valid;
    Ok(ProviderAuthStatus {
        provider: "opencode".to_string(),
        auth_state: if has_credentials {
            "authenticated"
        } else {
            "not_logged_in"
        }
        .to_string(),
        account_profile: account_profile.to_string(),
        // A generic phrase is not an account identity. Treating it as one
        // would collapse distinct OpenCode profiles during duplicate checks.
        identity_summary: None,
        plan: None,
        login_hint: Some(
            if credential_inspection == OpenCodeCredentialInspection::Malformed {
                "Stored OpenCode credentials are malformed; reauthenticate this account."
                    .to_string()
            } else {
                "Use Provider Accounts to run `opencode auth login` for this account.".to_string()
            },
        ),
        detected_version: command_version(&executable).ok(),
    })
}

fn opencode_usage_snapshot(
    account_profile: &str,
    environment: &BTreeMap<String, String>,
) -> crate::account_profile::ProviderAccountUsageSnapshot {
    use crate::account_profile::{
        ProviderAccountUsageAvailability, ProviderAccountUsageMeter, ProviderAccountUsageMeterKind,
        ProviderAccountUsageMeterScope, ProviderAccountUsageMeterState,
        ProviderAccountUsageSnapshot,
    };
    let observed_at_ms = crate::session::unix_epoch_ms();
    let mut meters = Vec::new();
    let local_stats = resolve_opencode_executable().ok().and_then(|executable| {
        let mut command = crate::provider::managed_isolated_utility_command(
            executable.display().to_string(),
            vec![
                "stats".to_string(),
                "--format".to_string(),
                "json".to_string(),
            ],
            environment.clone(),
            None,
            "opencode:usage",
        )
        .ok()?;
        remove_account_auth_environment(&mut command, "opencode");
        let output = command.output().ok()?;
        serde_json::from_slice::<serde_json::Value>(&output.stdout).ok()
    });
    if let Some(used) = local_stats
        .as_ref()
        .and_then(|value| find_numeric_field(value, &["tokens", "totalTokens", "total_tokens"]))
    {
        meters.push(ProviderAccountUsageMeter {
            meter_id: "local/tokens".to_string(),
            label: "Local token usage".to_string(),
            service_id: None,
            kind: ProviderAccountUsageMeterKind::TokenUsage,
            scope: ProviderAccountUsageMeterScope::Account,
            used_percent: None,
            used: Some(used),
            remaining: None,
            total: None,
            unit: Some("tokens".to_string()),
            window_duration_minutes: None,
            resets_at_ms: None,
            state: ProviderAccountUsageMeterState::Unknown,
            source: "opencode.local_stats".to_string(),
            observed_at_ms,
        });
    }
    if let Some(used) = local_stats
        .as_ref()
        .and_then(|value| find_numeric_field(value, &["cost", "totalCost", "total_cost"]))
    {
        meters.push(ProviderAccountUsageMeter {
            meter_id: "local/cost".to_string(),
            label: "Local recorded cost".to_string(),
            service_id: None,
            kind: ProviderAccountUsageMeterKind::LocalCost,
            scope: ProviderAccountUsageMeterScope::Account,
            used_percent: None,
            used: Some(used),
            remaining: None,
            total: None,
            unit: Some("USD".to_string()),
            window_duration_minutes: None,
            resets_at_ms: None,
            state: ProviderAccountUsageMeterState::Unknown,
            source: "opencode.local_stats".to_string(),
            observed_at_ms,
        });
    }
    let go_usage = opencode_go_usage(environment, observed_at_ms);
    if let OpenCodeGoUsage::Available(go_meters) = &go_usage {
        let mut combined = go_meters.clone();
        combined.extend(meters);
        meters = combined;
    }
    let has_provider_usage = matches!(go_usage, OpenCodeGoUsage::Available(_));
    ProviderAccountUsageSnapshot {
        profile_id: account_profile.to_string(),
        provider: "opencode".to_string(),
        availability: if has_provider_usage {
            ProviderAccountUsageAvailability::Available
        } else if meters.is_empty() {
            ProviderAccountUsageAvailability::Unavailable
        } else {
            // OpenCode local stats cannot represent Zen or arbitrary upstream
            // provider balances, so it is intentionally never "available".
            ProviderAccountUsageAvailability::Partial
        },
        meters,
        observed_at_ms: Some(observed_at_ms),
        source: match go_usage {
            OpenCodeGoUsage::Available(_) => "opencode.go_usage".to_string(),
            OpenCodeGoUsage::NotEntitled => "opencode.go_not_entitled".to_string(),
            OpenCodeGoUsage::Unavailable => "opencode.local_stats".to_string(),
        },
        management_url: Some("https://opencode.ai/zen".to_string()),
    }
}

#[derive(Debug, Clone, PartialEq)]
enum OpenCodeGoUsage {
    Available(Vec<crate::account_profile::ProviderAccountUsageMeter>),
    NotEntitled,
    Unavailable,
}

fn opencode_go_usage(
    environment: &BTreeMap<String, String>,
    observed_at_ms: u64,
) -> OpenCodeGoUsage {
    let Some(key) = opencode_provider_api_key(environment, "opencode-go") else {
        return OpenCodeGoUsage::Unavailable;
    };
    let response = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .get("https://opencode.ai/zen/go/v1/usage")
        .set("Authorization", &format!("Bearer {key}"))
        .set("User-Agent", "chariox-kernel/provider-usage")
        .call();
    match response {
        Ok(response) => match response
            .into_string()
            .ok()
            .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok())
        {
            Some(value) => opencode_go_usage_from_value(&value, observed_at_ms)
                .map(OpenCodeGoUsage::Available)
                .unwrap_or(OpenCodeGoUsage::Unavailable),
            None => OpenCodeGoUsage::Unavailable,
        },
        Err(ureq::Error::Status(403, response)) => {
            let body = response
                .into_string()
                .unwrap_or_default()
                .to_ascii_lowercase();
            if body.contains("entitlement") || body.contains("subscription required") {
                OpenCodeGoUsage::NotEntitled
            } else {
                OpenCodeGoUsage::Unavailable
            }
        }
        Err(_) => OpenCodeGoUsage::Unavailable,
    }
}

fn opencode_provider_api_key(
    environment: &BTreeMap<String, String>,
    provider_id: &str,
) -> Option<String> {
    let value = read_opencode_auth_document(environment)?;
    value
        .get(provider_id)?
        .get("key")?
        .as_str()
        .map(str::trim)
        .filter(|key| valid_opencode_secret(key))
        .map(str::to_string)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenCodeCredentialInspection {
    NotObserved,
    Valid,
    Malformed,
}

fn inspect_opencode_credentials(
    environment: &BTreeMap<String, String>,
) -> OpenCodeCredentialInspection {
    let Some(document) = read_opencode_auth_document(environment) else {
        return OpenCodeCredentialInspection::NotObserved;
    };
    let Some(entries) = document.as_object() else {
        return OpenCodeCredentialInspection::Malformed;
    };
    let mut observed_malformed = false;
    for credential in entries.values() {
        let credential_type = credential.get("type").and_then(serde_json::Value::as_str);
        let secret_field = match credential_type {
            Some("api") => Some("key"),
            Some("oauth") => Some("access"),
            _ => None,
        };
        let Some(secret_field) = secret_field else {
            continue;
        };
        let secret = credential
            .get(secret_field)
            .and_then(serde_json::Value::as_str);
        if secret.is_some_and(valid_opencode_secret) {
            return OpenCodeCredentialInspection::Valid;
        }
        observed_malformed = true;
    }
    if observed_malformed {
        OpenCodeCredentialInspection::Malformed
    } else {
        OpenCodeCredentialInspection::NotObserved
    }
}

fn inspect_opencode_services(
    environment: &BTreeMap<String, String>,
) -> Vec<crate::account_profile::ProviderAccountService> {
    use crate::account_profile::{
        ProviderAccountAuthState, ProviderAccountService, ProviderAccountServiceCredentialType,
        ProviderCredentialKind,
    };
    let Some(document) = read_opencode_auth_document(environment) else {
        return Vec::new();
    };
    let Some(entries) = document.as_object() else {
        return Vec::new();
    };
    let mut services = entries
        .iter()
        .map(|(service_id, credential)| {
            let credential_type = match credential.get("type").and_then(serde_json::Value::as_str) {
                Some("api") => ProviderAccountServiceCredentialType::ApiKey,
                Some("oauth") => ProviderAccountServiceCredentialType::Oauth,
                _ => ProviderAccountServiceCredentialType::Unknown,
            };
            let secret = match credential_type {
                ProviderAccountServiceCredentialType::ApiKey => credential.get("key"),
                ProviderAccountServiceCredentialType::Oauth => credential.get("access"),
                ProviderAccountServiceCredentialType::Unknown => None,
            }
            .and_then(serde_json::Value::as_str);
            ProviderAccountService {
                service_id: service_id.clone(),
                label: opencode_service_label(service_id),
                auth_state: if secret.is_some_and(valid_opencode_secret) {
                    ProviderAccountAuthState::Authenticated
                } else {
                    ProviderAccountAuthState::Error
                },
                credential_type,
                billing_kind: match service_id.as_str() {
                    "opencode-go" => Some(ProviderCredentialKind::Subscription),
                    "opencode" => Some(ProviderCredentialKind::Prepaid),
                    _ => None,
                },
            }
        })
        .collect::<Vec<_>>();
    services.sort_by(|left, right| left.service_id.cmp(&right.service_id));
    services
}

fn opencode_service_label(service_id: &str) -> String {
    match service_id {
        "opencode-go" => "OpenCode Go".to_string(),
        "opencode" => "OpenCode Zen".to_string(),
        "openai" => "OpenAI".to_string(),
        "anthropic" => "Anthropic".to_string(),
        other => other.to_string(),
    }
}

fn read_opencode_auth_document(
    environment: &BTreeMap<String, String>,
) -> Option<serde_json::Value> {
    let data_home = environment.get("XDG_DATA_HOME")?;
    let auth_path = std::path::Path::new(data_home).join("opencode/auth.json");
    serde_json::from_slice(&std::fs::read(auth_path).ok()?).ok()
}

fn valid_opencode_secret(secret: &str) -> bool {
    let secret = secret.trim();
    !secret.is_empty()
        && !secret
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
}

fn opencode_go_usage_from_value(
    value: &serde_json::Value,
    observed_at_ms: u64,
) -> Option<Vec<crate::account_profile::ProviderAccountUsageMeter>> {
    use crate::account_profile::{
        ProviderAccountUsageMeter, ProviderAccountUsageMeterKind, ProviderAccountUsageMeterScope,
    };
    let usage = value.get("usage").unwrap_or(value);
    let windows = [
        ("rolling", "OpenCode Go 5-hour", Some(5 * 60)),
        ("weekly", "OpenCode Go weekly", Some(7 * 24 * 60)),
        ("monthly", "OpenCode Go monthly", None),
    ];
    let meters = windows
        .into_iter()
        .filter_map(|(id, label, duration)| {
            let window = usage.get(id)?;
            let used_percent = window.get("percent")?.as_f64()?;
            Some(ProviderAccountUsageMeter {
                meter_id: format!("go/{id}"),
                label: label.to_string(),
                service_id: Some("opencode-go".to_string()),
                kind: ProviderAccountUsageMeterKind::RollingLimit,
                scope: ProviderAccountUsageMeterScope::Plan,
                used_percent: Some(used_percent),
                used: None,
                remaining: None,
                total: None,
                unit: None,
                window_duration_minutes: duration,
                resets_at_ms: window.get("resetsAt").and_then(provider_timestamp_ms),
                state: usage_meter_state(used_percent, window.get("status")),
                source: "opencode.go_usage".to_string(),
                observed_at_ms,
            })
        })
        .collect::<Vec<_>>();
    (!meters.is_empty()).then_some(meters)
}

fn provider_timestamp_ms(value: &serde_json::Value) -> Option<u64> {
    if let Some(value) = value.as_u64() {
        return Some(if value < 10_000_000_000 {
            value * 1_000
        } else {
            value
        });
    }
    value
        .as_str()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .and_then(|value| u64::try_from(value.timestamp_millis()).ok())
}

fn usage_meter_state(
    used_percent: f64,
    status: Option<&serde_json::Value>,
) -> crate::account_profile::ProviderAccountUsageMeterState {
    use crate::account_profile::ProviderAccountUsageMeterState;
    let status = status
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if used_percent >= 100.0 || matches!(status, "exhausted" | "rejected" | "rate-limited") {
        ProviderAccountUsageMeterState::Exhausted
    } else if used_percent >= 80.0 || status == "warning" {
        ProviderAccountUsageMeterState::Warning
    } else {
        ProviderAccountUsageMeterState::Healthy
    }
}

fn remove_account_auth_environment(command: &mut Command, provider: &str) {
    for name in crate::account_profile::provider_auth_env_vars(provider) {
        command.env_remove(name);
    }
}

fn command_version(executable: &std::path::Path) -> Result<String, DaemonError> {
    let output = crate::provider::managed_isolated_utility_command(
        executable.display().to_string(),
        vec!["--version".to_string()],
        BTreeMap::new(),
        None,
        "provider:version",
    )?
    .output()
    .map_err(|error| DaemonError::LocalTransport {
        operation: "provider_version",
        message: error.to_string(),
    })?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty())
        .then_some(text)
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "provider_version",
            message: "provider returned no version text".to_string(),
        })
}

fn find_numeric_field(value: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    match value {
        serde_json::Value::Object(object) => keys
            .iter()
            .find_map(|key| object.get(*key).and_then(serde_json::Value::as_f64))
            .or_else(|| {
                object
                    .values()
                    .find_map(|value| find_numeric_field(value, keys))
            }),
        serde_json::Value::Array(values) => values
            .iter()
            .find_map(|value| find_numeric_field(value, keys)),
        _ => None,
    }
}

fn claude_version() -> Result<String, DaemonError> {
    let executable = resolve_claude_executable()?;
    let output = crate::provider::managed_isolated_utility_command(
        executable.display().to_string(),
        vec!["--version".to_string()],
        BTreeMap::new(),
        None,
        "claude:version",
    )?
    .output()
    .map_err(|error| DaemonError::LocalTransport {
        operation: "get_provider_auth_status",
        message: format!("failed to read Claude version: {error}"),
    })?;
    if !output.status.success() {
        return Err(DaemonError::LocalTransport {
            operation: "get_provider_auth_status",
            message: "Claude version command failed".to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn claude_auth_status_from_value(
    provider: &str,
    account_profile: &str,
    value: &serde_json::Value,
    detected_version: Option<String>,
) -> ProviderAuthStatus {
    let logged_in = value
        .get("loggedIn")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let identity_summary = if logged_in {
        value
            .get("email")
            .and_then(serde_json::Value::as_str)
            .filter(|email| !email.is_empty())
            .map(str::to_string)
    } else {
        None
    };
    ProviderAuthStatus {
        provider: provider.to_string(),
        auth_state: if logged_in {
            "authenticated".to_string()
        } else {
            "not_logged_in".to_string()
        },
        account_profile: account_profile.to_string(),
        identity_summary,
        plan: value
            .get("subscriptionType")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        login_hint: Some("Run `claude auth login` to authenticate Claude Code.".to_string()),
        detected_version,
    }
}

pub(crate) fn start_provider_login_response(
    registry: &crate::account_profile::ProviderAccountProfileRegistry,
    owner_user_id: &str,
    request: StartProviderLoginRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let profile = registry.get(owner_user_id, &request.provider, &request.account_profile)?;
    let environment =
        registry.resolve_environment(owner_user_id, &request.provider, &profile.profile_id)?;
    match crate::provider::canonical_provider_family(&request.provider) {
        Some("codex") => {
            let endpoint = crate::provider::ensure_codex_account_endpoint(
                owner_user_id,
                &profile.profile_id,
                environment,
            )?;
            let client = CodexClient::new("provider-login", endpoint)?;
            Ok(LocalDaemonResponse::ProviderLoginStarted {
                login: client.start_login(&profile.profile_id)?,
            })
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "start_provider_login",
            message: format!(
                "provider `{}` does not expose a structured login API",
                request.provider
            ),
        }),
    }
}

pub(crate) fn logout_provider_response(
    registry: &crate::account_profile::ProviderAccountProfileRegistry,
    owner_user_id: &str,
    request: LogoutProviderRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let profile = registry.get(owner_user_id, &request.provider, &request.account_profile)?;
    let environment =
        registry.resolve_environment(owner_user_id, &request.provider, &profile.profile_id)?;
    match crate::provider::canonical_provider_family(&request.provider) {
        Some("codex") => {
            crate::provider::logout_codex(&environment)?;
            crate::provider::invalidate_codex_account_endpoint(owner_user_id, &profile.profile_id);
            Ok(LocalDaemonResponse::ProviderLoggedOut {
                provider: "codex".to_string(),
                account_profile: profile.profile_id,
            })
        }
        Some("claude") => {
            let executable = resolve_claude_executable()?;
            let mut command = crate::provider::managed_isolated_utility_command(
                executable.display().to_string(),
                vec!["auth".to_string(), "logout".to_string()],
                environment,
                None,
                "claude:logout",
            )?;
            for name in [
                "ANTHROPIC_API_KEY",
                "ANTHROPIC_AUTH_TOKEN",
                "ANTHROPIC_BASE_URL",
                "ANTHROPIC_CUSTOM_HEADERS",
            ] {
                command.env_remove(name);
            }
            let status = command
                .status()
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "logout_provider",
                    message: format!("failed to run Claude logout: {error}"),
                })?;
            if !status.success() {
                return Err(DaemonError::LocalTransport {
                    operation: "logout_provider",
                    message: format!("Claude logout failed: {status}"),
                });
            }
            Ok(LocalDaemonResponse::ProviderLoggedOut {
                provider: "claude".to_string(),
                account_profile: profile.profile_id,
            })
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "logout_provider",
            message: format!(
                "provider `{}` does not expose a logout API",
                request.provider
            ),
        }),
    }
}

fn update_profile_auth_observation(
    registry: &crate::account_profile::ProviderAccountProfileRegistry,
    owner_user_id: &str,
    status: &ProviderAuthStatus,
) -> Result<(), DaemonError> {
    let auth_state = auth_state_from_status(&status.auth_state);
    registry.update_observation(
        owner_user_id,
        &status.provider,
        &status.account_profile,
        auth_state,
        status.identity_summary.clone(),
        status.plan.clone(),
        status.detected_version.clone(),
        None,
    )?;
    Ok(())
}

fn auth_state_from_status(status: &str) -> crate::account_profile::ProviderAccountAuthState {
    match status {
        "authenticated" => crate::account_profile::ProviderAccountAuthState::Authenticated,
        "not_logged_in" => crate::account_profile::ProviderAccountAuthState::NotConfigured,
        "expired" => crate::account_profile::ProviderAccountAuthState::Expired,
        "error" => crate::account_profile::ProviderAccountAuthState::Error,
        _ => crate::account_profile::ProviderAccountAuthState::Unknown,
    }
}

fn unsupported_auth_provider(operation: &'static str, provider: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation,
        message: format!("provider `{provider}` does not expose an account API"),
    }
}

fn merge_provider_catalogs(
    catalogs: Vec<OpenCodeProviderCatalog>,
) -> Option<OpenCodeProviderCatalog> {
    let mut iter = catalogs.into_iter();
    let mut merged = iter.next()?;
    for catalog in iter {
        merged.connected.extend(catalog.connected);
        merged.connected.sort();
        merged.connected.dedup();
        for (provider_id, model_id) in catalog.default {
            merged.default.insert(provider_id, model_id);
        }
        for provider in catalog.all {
            if let Some(existing) = merged.all.iter_mut().find(|item| item.id == provider.id) {
                for (model_id, model) in provider.models {
                    existing.models.insert(model_id, model);
                }
            } else {
                merged.all.push(provider);
            }
        }
    }
    merged
        .all
        .sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
    Some(merged)
}

fn dev_stub_provider_catalog() -> OpenCodeProviderCatalog {
    OpenCodeProviderCatalog {
        all: vec![OpenCodeProviderInfo {
            id: "dev-stub".to_string(),
            name: "Dev Stub".to_string(),
            remote_machine_aliases: Vec::new(),
            models: Default::default(),
        }],
        default: Default::default(),
        connected: vec!["dev-stub".to_string()],
    }
}

fn opencode_backend_catalog(catalog: OpenCodeProviderCatalog) -> OpenCodeProviderCatalog {
    let source_connected = catalog.connected;
    let source_default = catalog.default;
    let mut all = catalog
        .all
        .into_iter()
        .filter(|provider| !provider.models.is_empty())
        .map(|mut provider| {
            provider.remote_machine_aliases.clear();
            provider
        })
        .collect::<Vec<_>>();
    all.sort_by(|left, right| left.id.cmp(&right.id));

    let connected = all
        .iter()
        .filter(|provider| source_connected.iter().any(|id| id == &provider.id))
        .map(|provider| provider.id.clone())
        .collect();
    let default = all
        .iter()
        .filter_map(|provider| {
            let model_id = source_default
                .get(&provider.id)
                .filter(|model_id| provider.models.contains_key(*model_id))
                .cloned()
                .or_else(|| provider.models.keys().next().cloned())?;
            Some((provider.id.clone(), model_id))
        })
        .collect();

    OpenCodeProviderCatalog {
        all,
        default,
        connected,
    }
}

fn remote_only_provider_catalog(
    live_machines: &[RelayMachinePresence],
    local_machine_id: &str,
) -> Option<OpenCodeProviderCatalog> {
    let mut provider_ids = live_machines
        .iter()
        .filter(|machine| machine.machine_id != local_machine_id)
        .flat_map(|machine| machine.available_providers.iter().cloned())
        .collect::<Vec<_>>();
    crate::provider::retain_public_inventory_providers(&mut provider_ids);
    provider_ids.sort();
    provider_ids.dedup();
    if provider_ids.is_empty() {
        return None;
    }

    let all = provider_ids
        .into_iter()
        .map(|provider_id| OpenCodeProviderInfo {
            name: display_name_for_provider(&provider_id),
            id: provider_id,
            remote_machine_aliases: Vec::new(),
            models: Default::default(),
        })
        .collect::<Vec<_>>();

    Some(OpenCodeProviderCatalog {
        connected: all.iter().map(|provider| provider.id.clone()).collect(),
        all,
        default: Default::default(),
    })
}

fn display_name_for_provider(provider_id: &str) -> String {
    match provider_id {
        "codex" => "Codex".to_string(),
        "opencode" => "OpenCode".to_string(),
        other => other.to_string(),
    }
}

fn annotate_remote_machine_providers(
    catalog: &mut OpenCodeProviderCatalog,
    live_machines: &[RelayMachinePresence],
    local_machine_id: &str,
) {
    for provider in &mut catalog.all {
        provider.remote_machine_aliases =
            remote_machine_aliases_for_provider(&provider.id, live_machines, local_machine_id);
    }
}

fn remote_machine_aliases_for_provider(
    provider_id: &str,
    live_machines: &[RelayMachinePresence],
    local_machine_id: &str,
) -> Vec<String> {
    let mut aliases = live_machines
        .iter()
        .filter(|machine| machine.machine_id != local_machine_id)
        .filter(|machine| {
            machine
                .available_providers
                .iter()
                .any(|provider| provider == provider_id)
        })
        .map(|machine| {
            machine
                .machine_alias
                .as_deref()
                .map(str::trim)
                .filter(|alias| !alias.is_empty())
                .unwrap_or(machine.machine_id.as_str())
                .to_string()
        })
        .collect::<Vec<_>>();
    aliases.sort();
    aliases.dedup();
    aliases
}

fn approved_live_remote_machines(
    live_machines: &[RelayMachinePresence],
    local_machine_id: &str,
) -> Vec<RelayMachinePresence> {
    let registry = DaemonConfig::machine_registry_entries();
    live_machines
        .iter()
        .filter(|machine| machine.machine_id != local_machine_id)
        .filter_map(|machine| {
            registry
                .iter()
                .find(|entry| {
                    entry.machine_id == machine.machine_id && entry.approved && !entry.forgotten
                })
                .map(|entry| {
                    let mut machine = machine.clone();
                    if entry.alias.is_some() {
                        machine.machine_alias = entry.alias.clone();
                    }
                    machine
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_profile::ProviderAccountUsageMeterState;
    use crate::provider::OpenCodeProviderModel;
    use serde_json::json;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn drill_provider_catalog_exposes_dev_stub_without_restricting_fixture_models() {
        let catalog = dev_stub_provider_catalog();

        assert_eq!(catalog.connected, vec!["dev-stub"]);
        assert_eq!(catalog.all.len(), 1);
        assert_eq!(catalog.all[0].id, "dev-stub");
        assert!(catalog.all[0].models.is_empty());
    }

    #[test]
    fn catalog_profile_resolution_uses_override_and_requires_target_materialization() {
        let root = std::env::temp_dir().join(format!(
            "chariox-catalog-profile-selection-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let registry = crate::account_profile::ProviderAccountProfileRegistry::open(
            root.join("profiles.json"),
        )
        .expect("registry should open");
        registry
            .migrate_effective_defaults("owner-a", &root.join("home"))
            .expect("defaults should migrate");
        let work = registry
            .create_managed("owner-a", "codex", "Work")
            .expect("work profile should be created");
        let request = GetProviderCatalogRequest {
            provider: Some("codex".to_string()),
            account_profiles: BTreeMap::from([("codex".to_string(), work.profile_id.clone())]),
            execution_location: ProviderCatalogExecutionLocation::Slice {
                slice_ref: "slice-a".to_string(),
            },
        };

        let error = resolve_catalog_profiles(&registry, "owner-a", &request)
            .expect_err("unmaterialized profile should be rejected");
        assert!(error.to_string().contains("not materialized"));
        registry
            .update_materialization_status(
                "owner-a",
                "codex",
                &work.profile_id,
                crate::account_profile::ProviderAccountMaterializationStatus {
                    target_kind:
                        crate::account_profile::ProviderAccountMaterializationTargetKind::Slice,
                    target_ref: "slice-a".to_string(),
                    state:
                        crate::account_profile::ProviderAccountMaterializationState::Materialized,
                    observed_at_ms: 1,
                    last_error: None,
                },
            )
            .expect("materialization should be recorded");
        let selected = resolve_catalog_profiles(&registry, "owner-a", &request)
            .expect("materialized selection should resolve");
        assert_eq!(selected["codex"].profile_id, work.profile_id);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn provider_auth_status_accepts_claude_provider_modes() {
        let _guard = crate::env_lock::lock();
        let path =
            std::env::temp_dir().join(format!("chariox-claude-auth-status-{}", std::process::id()));
        fs::write(
            &path,
            r#"#!/bin/sh
set -eu
if [ "$#" -ge 3 ] && [ "$1" = "auth" ] && [ "$2" = "status" ] && [ "$3" = "--json" ]; then
  printf '%s\n' '{"loggedIn":true,"authMethod":"claude.ai","email":"dev@example.test","orgName":"Example Org","subscriptionType":"pro"}'
  exit 0
fi
if [ "$#" -ge 1 ] && [ "$1" = "--version" ]; then
  printf '%s\n' 'claude 1.2.3'
  exit 0
fi
exit 2
"#,
        )
        .expect("fixture should exist");
        let mut permissions = fs::metadata(&path)
            .expect("fixture metadata should be readable")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("fixture should be executable");
        std::env::set_var("CHARIOX_CLAUDE_BIN", &path);

        let registry_root = std::env::temp_dir().join(format!(
            "chariox-claude-auth-registry-{}",
            std::process::id()
        ));
        let registry = crate::account_profile::ProviderAccountProfileRegistry::open(
            registry_root.join("profiles.json"),
        )
        .expect("profile registry should open");
        let migrated = registry
            .migrate_effective_defaults("local", &registry_root.join("home"))
            .expect("default profiles should migrate");
        let claude_profile_id = migrated
            .iter()
            .find(|profile| profile.provider == "claude")
            .expect("claude profile should migrate")
            .profile_id
            .clone();

        let response = provider_auth_status_response(
            &registry,
            "local",
            GetProviderAuthStatusRequest {
                provider: "claude-headless".to_string(),
                account_profile: "default".to_string(),
            },
        )
        .expect("claude mode auth status should resolve");

        std::env::remove_var("CHARIOX_CLAUDE_BIN");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir_all(&registry_root);

        match response {
            LocalDaemonResponse::ProviderAuthStatus { status } => {
                assert_eq!(status.provider, "claude-headless");
                assert_eq!(status.auth_state, "authenticated");
                assert_eq!(status.detected_version.as_deref(), Some("claude 1.2.3"));
                assert_eq!(status.account_profile, claude_profile_id);
                assert!(status
                    .identity_summary
                    .as_deref()
                    .unwrap_or_default()
                    .contains("dev@example.test"));
            }
            response => panic!("unexpected response: {response:?}"),
        }
    }

    #[test]
    fn claude_auth_status_parser_reports_not_logged_in() {
        let status = claude_auth_status_from_value(
            "claude-p",
            "work",
            &json!({ "loggedIn": false }),
            Some("claude 1.2.3".to_string()),
        );

        assert_eq!(status.provider, "claude-p");
        assert_eq!(status.auth_state, "not_logged_in");
        assert_eq!(status.account_profile, "work");
        assert_eq!(status.detected_version.as_deref(), Some("claude 1.2.3"));
    }

    #[test]
    fn claude_auth_status_uses_email_as_the_account_identity() {
        let status = claude_auth_status_from_value(
            "claude-headless",
            "work",
            &json!({
                "loggedIn": true,
                "email": "dev@example.test",
                "orgName": "dev@example.test's Organization",
                "subscriptionType": "pro"
            }),
            Some("claude 1.2.3".to_string()),
        );

        assert_eq!(status.identity_summary.as_deref(), Some("dev@example.test"));
        assert_eq!(status.plan.as_deref(), Some("pro"));
    }

    #[test]
    fn annotates_remote_machine_provider_aliases_without_including_local_machine() {
        let mut catalog = OpenCodeProviderCatalog {
            all: vec![
                OpenCodeProviderInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    remote_machine_aliases: Vec::new(),
                    models: Default::default(),
                },
                OpenCodeProviderInfo {
                    id: "opencode".to_string(),
                    name: "OpenCode".to_string(),
                    remote_machine_aliases: Vec::new(),
                    models: Default::default(),
                },
            ],
            default: Default::default(),
            connected: vec!["codex".to_string(), "opencode".to_string()],
        };
        let live_machines = vec![
            RelayMachinePresence {
                machine_id: "machine-local".to_string(),
                machine_alias: Some("home".to_string()),
                kernel_count: 1,
                available_providers: vec!["codex".to_string()],
                provider_accounts: Vec::new(),
            },
            RelayMachinePresence {
                machine_id: "machine-remote-a".to_string(),
                machine_alias: Some("builder-west".to_string()),
                kernel_count: 1,
                available_providers: vec!["codex".to_string(), "opencode".to_string()],
                provider_accounts: Vec::new(),
            },
            RelayMachinePresence {
                machine_id: "machine-remote-b".to_string(),
                machine_alias: None,
                kernel_count: 1,
                available_providers: vec!["codex".to_string()],
                provider_accounts: Vec::new(),
            },
        ];

        annotate_remote_machine_providers(&mut catalog, &live_machines, "machine-local");

        let codex = catalog
            .all
            .iter()
            .find(|provider| provider.id == "codex")
            .unwrap();
        let opencode = catalog
            .all
            .iter()
            .find(|provider| provider.id == "opencode")
            .unwrap();

        assert_eq!(
            codex.remote_machine_aliases,
            vec!["builder-west".to_string(), "machine-remote-b".to_string()]
        );
        assert_eq!(
            opencode.remote_machine_aliases,
            vec!["builder-west".to_string()]
        );
    }

    #[test]
    fn builds_remote_only_catalog_when_local_provider_sources_are_unavailable() {
        let live_machines = vec![
            RelayMachinePresence {
                machine_id: "machine-local".to_string(),
                machine_alias: Some("home".to_string()),
                kernel_count: 1,
                available_providers: vec!["codex".to_string()],
                provider_accounts: Vec::new(),
            },
            RelayMachinePresence {
                machine_id: "machine-remote".to_string(),
                machine_alias: Some("builder".to_string()),
                kernel_count: 1,
                available_providers: vec![
                    "codex".to_string(),
                    "opencode".to_string(),
                    "dev-stub".to_string(),
                ],
                provider_accounts: Vec::new(),
            },
        ];

        let mut catalog = remote_only_provider_catalog(&live_machines, "machine-local")
            .expect("remote providers should create a catalog");
        annotate_remote_machine_providers(&mut catalog, &live_machines, "machine-local");

        assert_eq!(
            catalog.connected,
            vec!["codex".to_string(), "opencode".to_string()]
        );
        assert!(!catalog.all.iter().any(|provider| provider.id == "dev-stub"));
        assert_eq!(catalog.default.len(), 0);
        let codex = catalog
            .all
            .iter()
            .find(|provider| provider.id == "codex")
            .unwrap();
        let opencode = catalog
            .all
            .iter()
            .find(|provider| provider.id == "opencode")
            .unwrap();
        assert_eq!(codex.name, "Codex");
        assert_eq!(codex.remote_machine_aliases, vec!["builder".to_string()]);
        assert_eq!(opencode.name, "OpenCode");
        assert_eq!(opencode.remote_machine_aliases, vec!["builder".to_string()]);
    }

    #[test]
    fn opencode_backend_catalog_keeps_zen_go_and_configured_upstream_providers() {
        let catalog = opencode_backend_catalog(OpenCodeProviderCatalog {
            all: vec![
                OpenCodeProviderInfo {
                    id: "openai".to_string(),
                    name: "OpenAI".to_string(),
                    remote_machine_aliases: Vec::new(),
                    models: BTreeMap::from([(
                        "gpt-5.2".to_string(),
                        OpenCodeProviderModel {
                            id: "gpt-5.2".to_string(),
                            name: "GPT-5.2".to_string(),
                            status: "active".to_string(),
                            limit: None,
                            variants: Default::default(),
                        },
                    )]),
                },
                OpenCodeProviderInfo {
                    id: "opencode".to_string(),
                    name: "OpenCode Zen".to_string(),
                    remote_machine_aliases: Vec::new(),
                    models: BTreeMap::from([(
                        "gpt-5.2".to_string(),
                        OpenCodeProviderModel {
                            id: "gpt-5.2".to_string(),
                            name: "GPT-5.2".to_string(),
                            status: "active".to_string(),
                            limit: None,
                            variants: BTreeMap::from([("low".to_string(), json!({}))]),
                        },
                    )]),
                },
                OpenCodeProviderInfo {
                    id: "opencode-go".to_string(),
                    name: "OpenCode Go".to_string(),
                    remote_machine_aliases: Vec::new(),
                    models: BTreeMap::from([(
                        "deepseek-v4-pro".to_string(),
                        OpenCodeProviderModel {
                            id: "deepseek-v4-pro".to_string(),
                            name: "DeepSeek V4 Pro".to_string(),
                            status: "active".to_string(),
                            limit: None,
                            variants: BTreeMap::from([("high".to_string(), json!({}))]),
                        },
                    )]),
                },
            ],
            default: BTreeMap::from([
                ("openai".to_string(), "gpt-5.2".to_string()),
                ("opencode".to_string(), "gpt-5.2".to_string()),
                ("opencode-go".to_string(), "deepseek-v4-pro".to_string()),
            ]),
            connected: vec![
                "openai".to_string(),
                "opencode".to_string(),
                "opencode-go".to_string(),
            ],
        });

        assert_eq!(
            catalog.connected,
            vec![
                "openai".to_string(),
                "opencode".to_string(),
                "opencode-go".to_string(),
            ]
        );
        assert_eq!(catalog.default.get("openai"), Some(&"gpt-5.2".to_string()));
        assert_eq!(
            catalog.default.get("opencode"),
            Some(&"gpt-5.2".to_string())
        );
        assert_eq!(
            catalog.default.get("opencode-go"),
            Some(&"deepseek-v4-pro".to_string())
        );
        assert_eq!(catalog.all.len(), 3);
        assert_eq!(catalog.all[0].id, "openai");
        assert_eq!(catalog.all[0].name, "OpenAI");
        assert!(catalog.all[0].models.contains_key("gpt-5.2"));
        assert_eq!(catalog.all[1].id, "opencode");
        assert_eq!(catalog.all[1].name, "OpenCode Zen");
        assert!(catalog.all[1].models.contains_key("gpt-5.2"));
        assert_eq!(catalog.all[2].id, "opencode-go");
        assert_eq!(catalog.all[2].name, "OpenCode Go");
        assert!(catalog.all[2].models.contains_key("deepseek-v4-pro"));
    }

    #[test]
    fn parses_all_opencode_go_subscription_windows() {
        let meters = opencode_go_usage_from_value(
            &json!({
                "usage": {
                    "rolling": {"status": "warning", "percent": 82.0, "resetsAt": "2027-01-15T12:00:00Z"},
                    "weekly": {"status": "active", "percent": 34.0, "resetsAt": 1_800_000_000},
                    "monthly": {"status": "rate-limited", "percent": 100.0}
                }
            }),
            42,
        )
        .expect("Go usage should parse");

        assert_eq!(meters.len(), 3);
        assert_eq!(meters[0].label, "OpenCode Go 5-hour");
        assert_eq!(meters[0].state, ProviderAccountUsageMeterState::Warning);
        assert_eq!(meters[1].label, "OpenCode Go weekly");
        assert_eq!(meters[1].resets_at_ms, Some(1_800_000_000_000));
        assert_eq!(meters[2].label, "OpenCode Go monthly");
        assert_eq!(meters[2].state, ProviderAccountUsageMeterState::Exhausted);
    }

    #[test]
    fn opencode_go_usage_is_fail_closed_without_reported_windows() {
        assert!(opencode_go_usage_from_value(&json!({}), 42).is_none());
        assert!(opencode_go_usage_from_value(
            &json!({"usage": {"rolling": {"status": "active"}}}),
            42
        )
        .is_none());
    }

    #[test]
    fn opencode_timestamps_accept_seconds_milliseconds_and_rfc3339() {
        assert_eq!(
            provider_timestamp_ms(&json!(1_800_000_000)),
            Some(1_800_000_000_000)
        );
        assert_eq!(
            provider_timestamp_ms(&json!(1_800_000_000_000u64)),
            Some(1_800_000_000_000)
        );
        assert_eq!(
            provider_timestamp_ms(&json!("2027-01-15T12:00:00Z")),
            Some(1_800_014_400_000)
        );
        assert_eq!(provider_timestamp_ms(&json!("not-a-time")), None);
    }

    #[test]
    fn reads_only_valid_opencode_provider_keys() {
        let root = std::env::temp_dir().join(format!(
            "chariox-opencode-go-key-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let data_home = root.join("data");
        fs::create_dir_all(data_home.join("opencode")).expect("auth directory should create");
        fs::write(
            data_home.join("opencode/auth.json"),
            r#"{"opencode-go":{"type":"api","key":"go-selected-profile-key"},"opencode":{"type":"api","key":"zen-selected-profile-key"},"openai":{"type":"oauth","access":"ignored"}}"#,
        )
        .expect("auth file should write");
        let environment =
            BTreeMap::from([("XDG_DATA_HOME".to_string(), data_home.display().to_string())]);

        assert_eq!(
            opencode_provider_api_key(&environment, "opencode-go").as_deref(),
            Some("go-selected-profile-key")
        );
        assert_eq!(
            opencode_provider_api_key(&environment, "opencode").as_deref(),
            Some("zen-selected-profile-key")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_menu_output_as_an_opencode_api_key() {
        let root = std::env::temp_dir().join(format!(
            "chariox-opencode-invalid-key-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let data_home = root.join("data");
        fs::create_dir_all(data_home.join("opencode")).expect("auth directory should create");
        fs::write(
            data_home.join("opencode/auth.json"),
            r#"{"opencode-go":{"type":"api","key":"└ enter"}}"#,
        )
        .expect("auth file should write");
        let environment =
            BTreeMap::from([("XDG_DATA_HOME".to_string(), data_home.display().to_string())]);

        assert_eq!(opencode_provider_api_key(&environment, "opencode-go"), None);
        assert_eq!(
            inspect_opencode_credentials(&environment),
            OpenCodeCredentialInspection::Malformed
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn accepts_opencode_oauth_and_short_opaque_api_credentials() {
        let root = std::env::temp_dir().join(format!(
            "chariox-opencode-valid-credentials-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let data_home = root.join("data");
        fs::create_dir_all(data_home.join("opencode")).expect("auth directory should create");
        let environment =
            BTreeMap::from([("XDG_DATA_HOME".to_string(), data_home.display().to_string())]);

        fs::write(
            data_home.join("opencode/auth.json"),
            r#"{"openai":{"type":"oauth","access":"oauth-token","refresh":"refresh-token"}}"#,
        )
        .expect("OAuth auth file should write");
        assert_eq!(
            inspect_opencode_credentials(&environment),
            OpenCodeCredentialInspection::Valid
        );

        fs::write(
            data_home.join("opencode/auth.json"),
            r#"{"opencode-go":{"type":"api","key":"go-key"},"opencode":{"type":"api","key":"zen-key"},"openai":{"type":"oauth","access":"oauth-token"}}"#,
        )
        .expect("mixed service auth file should write");
        let services = inspect_opencode_services(&environment);
        assert_eq!(
            services
                .iter()
                .map(|service| (
                    service.service_id.as_str(),
                    service.label.as_str(),
                    service.credential_type,
                    service.billing_kind,
                ))
                .collect::<Vec<_>>(),
            vec![
                (
                    "openai",
                    "OpenAI",
                    crate::account_profile::ProviderAccountServiceCredentialType::Oauth,
                    None,
                ),
                (
                    "opencode",
                    "OpenCode Zen",
                    crate::account_profile::ProviderAccountServiceCredentialType::ApiKey,
                    Some(crate::account_profile::ProviderCredentialKind::Prepaid),
                ),
                (
                    "opencode-go",
                    "OpenCode Go",
                    crate::account_profile::ProviderAccountServiceCredentialType::ApiKey,
                    Some(crate::account_profile::ProviderCredentialKind::Subscription),
                ),
            ]
        );

        fs::write(
            data_home.join("opencode/auth.json"),
            r#"{"opencode-go":{"type":"api","key":"短鍵"}}"#,
        )
        .expect("API auth file should write");
        assert_eq!(
            opencode_provider_api_key(&environment, "opencode-go").as_deref(),
            Some("短鍵")
        );
        assert_eq!(
            inspect_opencode_credentials(&environment),
            OpenCodeCredentialInspection::Valid
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_unobserved_and_control_character_opencode_credentials() {
        let root = std::env::temp_dir().join(format!(
            "chariox-opencode-unobserved-credentials-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let data_home = root.join("data");
        let environment =
            BTreeMap::from([("XDG_DATA_HOME".to_string(), data_home.display().to_string())]);

        assert_eq!(
            inspect_opencode_credentials(&environment),
            OpenCodeCredentialInspection::NotObserved
        );

        fs::create_dir_all(data_home.join("opencode")).expect("auth directory should create");
        fs::write(
            data_home.join("opencode/auth.json"),
            r#"{"opencode-go":{"type":"api","key":"opaque\u0000key"}}"#,
        )
        .expect("auth file should write");
        assert_eq!(opencode_provider_api_key(&environment, "opencode-go"), None);
        assert_eq!(
            inspect_opencode_credentials(&environment),
            OpenCodeCredentialInspection::Malformed
        );
        let _ = fs::remove_dir_all(root);
    }
}
