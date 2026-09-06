use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::account_profile::{
    ProviderAccountUsageAvailability, ProviderAccountUsageMeter, ProviderAccountUsageMeterKind,
    ProviderAccountUsageMeterScope, ProviderAccountUsageMeterState, ProviderAccountUsageSnapshot,
};
use crate::error::DaemonError;

use super::{resolve_codex_executable, CodexClient};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderAuthStatus {
    pub provider: String,
    pub auth_state: String,
    pub account_profile: String,
    pub identity_summary: Option<String>,
    pub plan: Option<String>,
    pub login_hint: Option<String>,
    pub detected_version: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderLoginStart {
    pub provider: String,
    pub account_profile: String,
    pub login_kind: String,
    pub login_id: Option<String>,
    pub auth_url: Option<String>,
    pub verification_url: Option<String>,
    pub user_code: Option<String>,
}

impl std::fmt::Debug for ProviderLoginStart {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderLoginStart")
            .field("provider", &self.provider)
            .field("account_profile", &self.account_profile)
            .field("login_kind", &self.login_kind)
            .field("login_id", &self.login_id)
            .field("auth_url", &self.auth_url.as_ref().map(|_| "[REDACTED]"))
            .field(
                "verification_url",
                &self.verification_url.as_ref().map(|_| "[REDACTED]"),
            )
            .field("user_code", &self.user_code.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

#[derive(Debug, Clone, Deserialize)]
struct CodexGetAccountResponse {
    account: Option<CodexAccount>,
    #[serde(rename = "requiresOpenaiAuth")]
    requires_openai_auth: bool,
    #[serde(rename = "planType", default)]
    plan_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CodexAccount {
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CodexLoginStartResponse {
    #[serde(rename = "type")]
    login_kind: String,
    #[serde(rename = "loginId", default)]
    login_id: Option<String>,
    #[serde(rename = "authUrl", default)]
    auth_url: Option<String>,
    #[serde(rename = "verificationUrl", default)]
    verification_url: Option<String>,
    #[serde(rename = "userCode", default)]
    user_code: Option<String>,
}

impl CodexClient {
    pub fn auth_status(&self, account_profile: &str) -> Result<ProviderAuthStatus, DaemonError> {
        let mut socket = self.connect_initialized()?;
        let mut next_request_id = 1;
        let response: CodexGetAccountResponse =
            self.send_request(&mut socket, &mut next_request_id, "account/read", json!({}))?;
        Ok(ProviderAuthStatus {
            provider: "codex".to_string(),
            auth_state: if response.account.is_some() {
                "authenticated".to_string()
            } else if response.requires_openai_auth {
                "not_logged_in".to_string()
            } else {
                "unknown".to_string()
            },
            account_profile: account_profile.to_string(),
            identity_summary: response.account.and_then(|account| account.email),
            plan: response.plan_type,
            login_hint: Some("Run /provider login codex to authenticate Codex.".to_string()),
            detected_version: codex_version().ok(),
        })
    }

    pub fn start_login(&self, account_profile: &str) -> Result<ProviderLoginStart, DaemonError> {
        let mut socket = self.connect_initialized()?;
        let mut next_request_id = 1;
        let response: CodexLoginStartResponse = self.send_request(
            &mut socket,
            &mut next_request_id,
            "account/login/start",
            json!({ "type": "chatgptDeviceCode" }),
        )?;
        Ok(ProviderLoginStart {
            provider: "codex".to_string(),
            account_profile: account_profile.to_string(),
            login_kind: response.login_kind,
            login_id: response.login_id,
            auth_url: response.auth_url,
            verification_url: response.verification_url,
            user_code: response.user_code,
        })
    }

    pub fn cancel_login(&self, login_id: &str) -> Result<(), DaemonError> {
        let mut socket = self.connect_initialized()?;
        let mut next_request_id = 1;
        let _: serde_json::Value = self.send_request(
            &mut socket,
            &mut next_request_id,
            "account/login/cancel",
            json!({ "loginId": login_id }),
        )?;
        Ok(())
    }

    /// Reads every account-usage surface exposed by the official Codex app
    /// server. The methods have evolved independently, so an unavailable
    /// surface degrades the snapshot to `partial` instead of discarding meters
    /// returned by the other one.
    pub fn usage_snapshot(
        &self,
        account_profile: &str,
    ) -> Result<ProviderAccountUsageSnapshot, DaemonError> {
        let mut socket = self.connect_initialized()?;
        let mut next_request_id = 1;
        let rate_limits = self
            .send_request::<serde_json::Value>(
                &mut socket,
                &mut next_request_id,
                "account/rateLimits/read",
                json!({}),
            )
            .ok();
        let usage = self
            .send_request::<serde_json::Value>(
                &mut socket,
                &mut next_request_id,
                "account/usage/read",
                json!({}),
            )
            .ok();
        Ok(normalize_codex_usage(
            account_profile,
            rate_limits.as_ref(),
            usage.as_ref(),
        ))
    }
}

fn normalize_codex_usage(
    account_profile: &str,
    rate_limits: Option<&serde_json::Value>,
    usage: Option<&serde_json::Value>,
) -> ProviderAccountUsageSnapshot {
    let observed_at_ms = crate::session::unix_epoch_ms();
    let mut meters = Vec::new();
    if let Some(value) = rate_limits {
        collect_usage_meters(value, "rate_limits", None, observed_at_ms, &mut meters);
    }
    if let Some(value) = usage {
        collect_usage_meters(value, "usage", None, observed_at_ms, &mut meters);
    }
    // Same-identity meters from a later surface are fresher, so last wins.
    let mut deduped: Vec<ProviderAccountUsageMeter> = Vec::with_capacity(meters.len());
    for meter in meters {
        match deduped
            .iter()
            .position(|existing| existing.meter_id == meter.meter_id)
        {
            Some(index) => deduped[index] = meter,
            None => deduped.push(meter),
        }
    }
    deduped.sort_by(|left, right| left.meter_id.cmp(&right.meter_id));
    let meters = deduped;
    let available_surfaces = usize::from(rate_limits.is_some()) + usize::from(usage.is_some());
    ProviderAccountUsageSnapshot {
        profile_id: account_profile.to_string(),
        provider: "codex".to_string(),
        availability: match (available_surfaces, meters.is_empty()) {
            (2, false) => ProviderAccountUsageAvailability::Available,
            (1.., false) => ProviderAccountUsageAvailability::Partial,
            (1.., true) => ProviderAccountUsageAvailability::Partial,
            _ => ProviderAccountUsageAvailability::Unavailable,
        },
        meters,
        observed_at_ms: (available_surfaces > 0).then_some(observed_at_ms),
        source: if available_surfaces > 0 {
            "codex.app_server".to_string()
        } else {
            "provider_api_unavailable".to_string()
        },
        management_url: Some("https://chatgpt.com/codex/settings/usage".to_string()),
    }
}

fn collect_usage_meters(
    value: &serde_json::Value,
    path: &str,
    scope_label: Option<&str>,
    observed_at_ms: u64,
    meters: &mut Vec<ProviderAccountUsageMeter>,
) {
    match value {
        serde_json::Value::Object(object) => {
            let scope_label = string_field(object, &["limitName", "limit_name"])
                .filter(|label| !label.trim().is_empty())
                .or_else(|| scope_label.map(str::to_string));
            let used_percent =
                number_field(object, &["usedPercent", "used_percent"]).or_else(|| {
                    number_field(object, &["utilization"]).map(|value| {
                        if value <= 1.0 {
                            value * 100.0
                        } else {
                            value
                        }
                    })
                });
            let remaining = number_field(object, &["remaining", "balance", "credits"]);
            let used = number_field(object, &["used", "amountUsed", "amount_used"]);
            let total = number_field(object, &["total", "limit", "spendLimit", "spend_limit"]);
            if used_percent.is_some() || remaining.is_some() || used.is_some() || total.is_some() {
                let lower_path = path.to_ascii_lowercase();
                let kind = if lower_path.contains("credit") || lower_path.contains("balance") {
                    ProviderAccountUsageMeterKind::CreditBalance
                } else if lower_path.contains("spend") || lower_path.contains("cost") {
                    ProviderAccountUsageMeterKind::SpendLimit
                } else {
                    ProviderAccountUsageMeterKind::RollingLimit
                };
                let state = match (kind, used_percent, remaining) {
                    (_, Some(value), _) if value >= 100.0 => {
                        ProviderAccountUsageMeterState::Exhausted
                    }
                    (_, Some(value), _) if value >= 80.0 => ProviderAccountUsageMeterState::Warning,
                    (_, Some(_), _) => ProviderAccountUsageMeterState::Healthy,
                    (ProviderAccountUsageMeterKind::CreditBalance, None, Some(value))
                        if value <= 0.0 =>
                    {
                        ProviderAccountUsageMeterState::Exhausted
                    }
                    (ProviderAccountUsageMeterKind::CreditBalance, None, Some(_)) => {
                        ProviderAccountUsageMeterState::Healthy
                    }
                    _ => ProviderAccountUsageMeterState::Unknown,
                };
                let window_duration_minutes = integer_field(
                    object,
                    &[
                        "windowDurationMins",
                        "windowDurationMinutes",
                        "window_duration_minutes",
                    ],
                );
                let period_label = match window_duration_minutes {
                    Some(300) => Some("5-hour"),
                    Some(10_080) => Some("Weekly"),
                    Some(43_200..=44_640) => Some("Monthly"),
                    _ => None,
                };
                let label = match (scope_label.as_deref(), period_label) {
                    (Some(scope), Some(period)) => format!("{scope} · {period}"),
                    (Some(scope), None) => scope.to_string(),
                    (None, Some(period)) => period.to_string(),
                    _ => scoped_meter_label(path),
                };
                // Meter identity is the limit kind plus window duration and,
                // when the payload carries one, the provider-native scoped
                // limit id — never the JSON traversal path. App-server
                // payloads expose the same windows under different shapes
                // across versions and surfaces, and stable identities keep
                // the account merge from showing duplicate or orphaned window
                // meters while keeping distinct same-duration limits
                // separate.
                let scoped_limit_id = scoped_container_identity(path);
                let meter_id = match kind {
                    ProviderAccountUsageMeterKind::CreditBalance => scoped_limit_id
                        .map(|limit_id| format!("credits/{limit_id}"))
                        .unwrap_or_else(|| "credits".to_string()),
                    ProviderAccountUsageMeterKind::SpendLimit => scoped_limit_id
                        .map(|limit_id| format!("spend/{limit_id}"))
                        .unwrap_or_else(|| "spend".to_string()),
                    _ => match (window_duration_minutes, scoped_meter_identity(path)) {
                        (Some(minutes), Some(limit_id)) => format!("rolling/{minutes}/{limit_id}"),
                        (Some(minutes), None) => format!("rolling/{minutes}"),
                        (None, _) => format!("rolling/{}", path.replace('.', "/")),
                    },
                };
                meters.push(ProviderAccountUsageMeter {
                    meter_id,
                    label,
                    service_id: None,
                    kind,
                    scope: ProviderAccountUsageMeterScope::Account,
                    used_percent,
                    used,
                    remaining,
                    total,
                    unit: string_field(object, &["unit", "currency"]),
                    window_duration_minutes,
                    resets_at_ms: timestamp_field_ms(object, &["resetsAt", "resetAt", "resets_at"]),
                    state,
                    source: "codex.app_server".to_string(),
                    observed_at_ms,
                });
            }
            // Newer app-server versions return the same convenience windows
            // under `rateLimits` and the authoritative, scoped form under
            // `rateLimitsByLimitId`. Prefer the scoped form instead of showing
            // duplicate meters to the user.
            let has_scoped_rate_limits = object.keys().any(|key| {
                matches!(
                    key.as_str(),
                    "rateLimitsByLimitId" | "rate_limits_by_limit_id"
                )
            });
            for (key, child) in object {
                if has_scoped_rate_limits && matches!(key.as_str(), "rateLimits" | "rate_limits") {
                    continue;
                }
                let child_path = format!("{path}.{key}");
                collect_usage_meters(
                    child,
                    &child_path,
                    scope_label.as_deref(),
                    observed_at_ms,
                    meters,
                );
            }
        }
        serde_json::Value::Array(values) => {
            for (index, child) in values.iter().enumerate() {
                collect_usage_meters(
                    child,
                    &format!("{path}.{index}"),
                    scope_label,
                    observed_at_ms,
                    meters,
                );
            }
        }
        _ => {}
    }
}

fn number_field(object: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| {
        let value = object.get(*key)?;
        value
            .as_f64()
            .or_else(|| value.as_str()?.trim().parse::<f64>().ok())
    })
}

fn integer_field(
    object: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<u64> {
    keys.iter().find_map(|key| {
        let value = object.get(*key)?;
        value
            .as_u64()
            .or_else(|| value.as_f64().map(|value| value as u64))
    })
}

fn string_field(
    object: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<String> {
    keys.iter()
        .find_map(|key| object.get(*key)?.as_str().map(str::to_string))
}

fn timestamp_field_ms(
    object: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<u64> {
    keys.iter().find_map(|key| {
        let value = object.get(*key)?;
        if let Some(value) = value.as_u64() {
            return Some(epoch_to_ms(value));
        }
        if let Some(value) = value.as_f64() {
            return (value > 0.0).then(|| epoch_to_ms(value as u64));
        }
        let text = value.as_str()?.trim();
        if let Ok(value) = text.parse::<u64>() {
            return Some(epoch_to_ms(value));
        }
        chrono::DateTime::parse_from_rfc3339(text)
            .ok()
            .and_then(|value| u64::try_from(value.timestamp_millis()).ok())
    })
}

/// Scoped limit identity for meter ids: the nearest provider-native limit
/// segment between the surface prefix and the window entry. Surface names,
/// container keys, positional family keys, and array indices carry no
/// provider identity, so the same scoped limit normalizes identically across
/// the `rate_limits` and `usage` surfaces.
fn scoped_meter_identity(path: &str) -> Option<&str> {
    if let Some(identity) = scoped_container_identity(path) {
        return Some(identity);
    }
    path.split('.').skip(1).find(|segment| {
        !matches!(
            *segment,
            "rateLimits"
                | "rate_limits"
                | "rateLimitsByLimitId"
                | "rate_limits_by_limit_id"
                | "primary"
                | "secondary"
                | "tertiary"
        ) && segment.parse::<u64>().is_err()
    })
}

fn scoped_container_identity(path: &str) -> Option<&str> {
    let segments = path.split('.').collect::<Vec<_>>();
    if let Some(container_index) = segments
        .iter()
        .position(|segment| matches!(*segment, "rateLimitsByLimitId" | "rate_limits_by_limit_id"))
    {
        return segments
            .get(container_index + 1)
            .copied()
            .filter(|segment| {
                !segment.is_empty() && !matches!(*segment, "primary" | "secondary" | "tertiary")
            });
    }
    None
}

/// User-facing labels must be window periods or provider-reported scoped
/// names, never positional `primary`/`secondary` family keys.
fn scoped_meter_label(path: &str) -> String {
    let readable = |segment: &str| segment.replace(['_', '-'], " ");
    let tail = path.rsplit('.').next().unwrap_or(path);
    if matches!(tail, "primary" | "secondary" | "tertiary") {
        return path
            .split('.')
            .rev()
            .nth(1)
            .map(readable)
            .unwrap_or_else(|| "rate limit".to_string());
    }
    readable(tail)
}

fn epoch_to_ms(value: u64) -> u64 {
    if value < 10_000_000_000 {
        value * 1_000
    } else {
        value
    }
}

fn codex_version() -> Result<String, DaemonError> {
    let executable = resolve_codex_executable()?;
    let output = crate::provider::managed_isolated_utility_command(
        executable.display().to_string(),
        vec!["--version".to_string()],
        BTreeMap::new(),
        None,
        "codex:version",
    )?
    .output()
    .map_err(|error| DaemonError::LocalTransport {
        operation: "codex_version",
        message: error.to_string(),
    })?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stdout.is_empty() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return Ok(stderr);
    }
    Err(DaemonError::LocalTransport {
        operation: "codex_version",
        message: "codex returned no version text".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::normalize_codex_usage;
    use crate::account_profile::{
        ProviderAccountUsageAvailability, ProviderAccountUsageMeterState,
    };

    #[test]
    fn normalizes_multiple_codex_limit_windows_and_credit_balance() {
        let snapshot = normalize_codex_usage(
            "work",
            Some(&json!({
                "rateLimits": {
                    "primary": {"usedPercent": 82.0, "windowDurationMins": 300, "resetsAt": 1_800_000_000},
                    "secondary": {"usedPercent": 100.0, "windowDurationMins": 10080}
                }
            })),
            Some(&json!({"credits": {"balance": 12.5, "unit": "USD"}})),
        );

        assert_eq!(
            snapshot.availability,
            ProviderAccountUsageAvailability::Available
        );
        assert_eq!(snapshot.meters.len(), 3);
        assert!(snapshot
            .meters
            .iter()
            .any(|meter| meter.state == ProviderAccountUsageMeterState::Exhausted));
        assert!(snapshot
            .meters
            .iter()
            .any(|meter| meter.remaining == Some(12.5)));
    }

    #[test]
    fn prefers_scoped_codex_windows_and_uses_period_labels() {
        let snapshot = normalize_codex_usage(
            "work",
            Some(&json!({
                "rateLimits": {
                    "primary": {"usedPercent": 12.0, "windowDurationMins": 300},
                    "secondary": {"usedPercent": 34.0, "windowDurationMins": 10080}
                },
                "rateLimitsByLimitId": {
                    "codex": {
                        "limitName": null,
                        "primary": {"usedPercent": 12.0, "windowDurationMins": 300},
                        "secondary": {"usedPercent": 34.0, "windowDurationMins": 10080}
                    },
                    "codex_bengalfox": {
                        "limitName": "GPT-5.3-Codex-Spark",
                        "primary": {"usedPercent": 1.0, "windowDurationMins": 300},
                        "secondary": {"usedPercent": 2.0, "windowDurationMins": 10080}
                    }
                }
            })),
            None,
        );

        assert_eq!(snapshot.meters.len(), 4);
        let five_hour = snapshot
            .meters
            .iter()
            .find(|meter| meter.window_duration_minutes == Some(300))
            .expect("5-hour window");
        let weekly = snapshot
            .meters
            .iter()
            .find(|meter| meter.window_duration_minutes == Some(10_080))
            .expect("weekly window");
        assert_eq!(five_hour.label, "5-hour");
        assert_eq!(weekly.label, "Weekly");
        assert_eq!(five_hour.meter_id, "rolling/300/codex");
        assert_eq!(weekly.meter_id, "rolling/10080/codex");
        let spark_labels = snapshot
            .meters
            .iter()
            .filter(|meter| meter.meter_id.ends_with("/codex_bengalfox"))
            .map(|meter| meter.label.as_str())
            .collect::<Vec<_>>();
        assert!(spark_labels.contains(&"GPT-5.3-Codex-Spark · 5-hour"));
        assert!(spark_labels.contains(&"GPT-5.3-Codex-Spark · Weekly"));
    }

    #[test]
    fn parses_string_credit_balances_and_marks_zero_exhausted() {
        let snapshot = normalize_codex_usage(
            "work",
            Some(&json!({
                "rateLimits": {
                    "credits": {"hasCredits": false, "balance": "0"}
                }
            })),
            None,
        );

        assert_eq!(snapshot.meters.len(), 1);
        let credits = &snapshot.meters[0];
        assert_eq!(credits.meter_id, "credits");
        assert_eq!(credits.remaining, Some(0.0));
        assert_eq!(credits.state, ProviderAccountUsageMeterState::Exhausted);
    }

    #[test]
    fn keeps_same_duration_scoped_limits_distinct() {
        let snapshot = normalize_codex_usage(
            "work",
            Some(&json!({
                "rateLimitsByLimitId": {
                    "codex": {"primary": {"usedPercent": 12.0, "windowDurationMins": 300}},
                    "gpt5": {"primary": {"usedPercent": 60.0, "windowDurationMins": 300}}
                }
            })),
            None,
        );

        assert_eq!(snapshot.meters.len(), 2);
        let ids = [
            snapshot.meters[0].meter_id.clone(),
            snapshot.meters[1].meter_id.clone(),
        ];
        assert!(ids.contains(&"rolling/300/codex".to_string()));
        assert!(ids.contains(&"rolling/300/gpt5".to_string()));
    }

    #[test]
    fn preserves_scoped_labels_without_a_known_window_duration() {
        let snapshot = normalize_codex_usage(
            "work",
            Some(&json!({
                "rateLimitsByLimitId": {
                    "codex_bengalfox": {
                        "limitName": "GPT-5.3-Codex-Spark",
                        "primary": {"usedPercent": 12.0}
                    }
                }
            })),
            None,
        );

        assert_eq!(snapshot.meters.len(), 1);
        assert_eq!(snapshot.meters[0].label, "GPT-5.3-Codex-Spark");
    }

    #[test]
    fn keeps_scoped_credit_balances_distinct() {
        let snapshot = normalize_codex_usage(
            "work",
            Some(&json!({
                "rateLimitsByLimitId": {
                    "personal": {
                        "limitName": "Personal credits",
                        "creditBalance": {"balance": "5"}
                    },
                    "team": {
                        "limitName": "Team credits",
                        "creditBalance": {"balance": "9"}
                    }
                }
            })),
            None,
        );

        assert_eq!(snapshot.meters.len(), 2);
        let ids = snapshot
            .meters
            .iter()
            .map(|meter| meter.meter_id.as_str())
            .collect::<Vec<_>>();
        assert!(ids.contains(&"credits/personal"));
        assert!(ids.contains(&"credits/team"));
        let labels = snapshot
            .meters
            .iter()
            .map(|meter| meter.label.as_str())
            .collect::<Vec<_>>();
        assert!(labels.contains(&"Personal credits"));
        assert!(labels.contains(&"Team credits"));
    }

    #[test]
    fn keeps_unnamed_scoped_credit_balances_distinct() {
        let snapshot = normalize_codex_usage(
            "work",
            Some(&json!({
                "rateLimitsByLimitId": {
                    "personal": {
                        "limitName": null,
                        "creditBalance": {"balance": "5"}
                    },
                    "team": {
                        "creditBalance": {"balance": "9"}
                    }
                }
            })),
            None,
        );

        assert_eq!(snapshot.meters.len(), 2);
        let ids = snapshot
            .meters
            .iter()
            .map(|meter| meter.meter_id.as_str())
            .collect::<Vec<_>>();
        assert!(ids.contains(&"credits/personal"));
        assert!(ids.contains(&"credits/team"));
    }

    #[test]
    fn keeps_numeric_scoped_limit_ids_distinct() {
        let snapshot = normalize_codex_usage(
            "work",
            Some(&json!({
                "rateLimitsByLimitId": {
                    "2026": {"primary": {"usedPercent": 12.0, "windowDurationMins": 300}},
                    "2027": {"primary": {"usedPercent": 60.0, "windowDurationMins": 300}}
                }
            })),
            None,
        );

        assert_eq!(snapshot.meters.len(), 2);
        let ids = snapshot
            .meters
            .iter()
            .map(|meter| meter.meter_id.as_str())
            .collect::<Vec<_>>();
        assert!(ids.contains(&"rolling/300/2026"));
        assert!(ids.contains(&"rolling/300/2027"));
    }

    #[test]
    fn dedupes_the_same_scoped_limit_across_usage_surfaces() {
        let snapshot = normalize_codex_usage(
            "work",
            Some(&json!({
                "rateLimitsByLimitId": {
                    "codex": {"primary": {"usedPercent": 12.0, "windowDurationMins": 300}}
                }
            })),
            Some(&json!({
                "rateLimitsByLimitId": {
                    "codex": {"primary": {"utilization": 0.55, "windowDurationMins": 300}}
                }
            })),
        );

        assert_eq!(snapshot.meters.len(), 1);
        assert_eq!(snapshot.meters[0].meter_id, "rolling/300/codex");
    }

    #[test]
    fn keeps_one_meter_per_window_across_usage_surfaces() {
        let snapshot = normalize_codex_usage(
            "work",
            Some(&json!({
                "rateLimits": {
                    "primary": {"usedPercent": 50.0, "windowDurationMins": 300}
                }
            })),
            Some(&json!({
                "rateLimits": {
                    "primary": {"utilization": 0.55, "windowDurationMins": 300}
                }
            })),
        );

        assert_eq!(snapshot.meters.len(), 1);
        assert_eq!(snapshot.meters[0].meter_id, "rolling/300");
        assert!(
            (snapshot.meters[0].used_percent.expect("used percentage") - 55.0).abs()
                < f64::EPSILON * 100.0
        );
    }

    #[test]
    fn keeps_distinct_durationless_windows_without_exposing_positional_labels() {
        let snapshot = normalize_codex_usage(
            "work",
            Some(&json!({
                "rateLimits": {
                    "primary": {"usedPercent": 40.0},
                    "secondary": {"usedPercent": 10.0}
                }
            })),
            None,
        );

        assert_eq!(snapshot.meters.len(), 2);
        assert_ne!(snapshot.meters[0].meter_id, snapshot.meters[1].meter_id);
        assert!(snapshot
            .meters
            .iter()
            .all(|meter| !matches!(meter.label.as_str(), "primary" | "secondary")));
    }

    #[test]
    fn parses_string_and_ratio_limit_fields() {
        let snapshot = normalize_codex_usage(
            "work",
            Some(&json!({
                "rateLimits": {
                    "primary": {
                        "utilization": 0.4,
                        "windowDurationMins": 10080,
                        "resetsAt": "2027-01-15T12:00:00Z"
                    },
                    "secondary": {
                        "usedPercent": 10.0,
                        "resetsAt": 1_800_000_000
                    }
                }
            })),
            None,
        );

        assert_eq!(snapshot.meters.len(), 2);
        let weekly = snapshot
            .meters
            .iter()
            .find(|meter| meter.window_duration_minutes == Some(10_080))
            .expect("weekly window");
        assert_eq!(weekly.used_percent, Some(40.0));
        assert_eq!(weekly.resets_at_ms, Some(1_800_014_400_000));
        let other = snapshot
            .meters
            .iter()
            .find(|meter| meter.window_duration_minutes.is_none())
            .expect("duration-less window");
        assert_ne!(other.label, "secondary");
    }

    #[test]
    fn unavailable_codex_methods_are_explicit() {
        let snapshot = normalize_codex_usage("work", None, None);
        assert_eq!(
            snapshot.availability,
            ProviderAccountUsageAvailability::Unavailable
        );
        assert!(snapshot.meters.is_empty());
    }
}
