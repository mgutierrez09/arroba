use std::fs;

use serde_json::Value;

use crate::account_profile::{
    ProviderAccountUsageAvailability, ProviderAccountUsageMeter, ProviderAccountUsageMeterKind,
    ProviderAccountUsageMeterScope, ProviderAccountUsageMeterState, ProviderAccountUsageSnapshot,
};
use crate::session::unix_epoch_ms;

use super::state::ClaudeRuntimeState;
use super::ProviderPromptSignalBatch;

pub(super) fn apply_claude_usage_capture(
    state: &mut ClaudeRuntimeState,
    batch: &mut ProviderPromptSignalBatch,
) {
    let Some(path) = state.usage_file.as_ref() else {
        return;
    };
    let Ok(contents) = fs::read_to_string(path) else {
        return;
    };
    let contents = contents.trim();
    if contents.is_empty()
        || state
            .last_usage_file_contents
            .as_deref()
            .is_some_and(|previous| previous == contents)
    {
        return;
    }
    let Ok(value) = serde_json::from_str::<Value>(contents) else {
        return;
    };
    let Some(snapshot) = claude_status_line_usage_snapshot(&value) else {
        return;
    };
    state.last_usage_file_contents = Some(contents.to_string());
    merge_claude_account_usage(&mut batch.account_usage, snapshot);
}

pub(super) fn merge_claude_account_usage(
    current: &mut Option<ProviderAccountUsageSnapshot>,
    mut incoming: ProviderAccountUsageSnapshot,
) {
    let Some(existing) = current.as_mut() else {
        *current = Some(incoming);
        return;
    };
    for meter in incoming.meters.drain(..) {
        if let Some(index) = existing
            .meters
            .iter()
            .position(|candidate| candidate.meter_id == meter.meter_id)
        {
            existing.meters[index] = meter;
        } else {
            existing.meters.push(meter);
        }
    }
    existing
        .meters
        .sort_by_key(|meter| meter.window_duration_minutes.unwrap_or(u64::MAX));
    existing.availability = ProviderAccountUsageAvailability::Available;
    existing.observed_at_ms = existing.observed_at_ms.max(incoming.observed_at_ms);
    existing.source = if existing.source == incoming.source {
        existing.source.clone()
    } else {
        "claude.native_usage".to_string()
    };
    if incoming.management_url.is_some() {
        existing.management_url = incoming.management_url;
    }
}

pub(crate) fn claude_status_line_usage_snapshot(
    value: &Value,
) -> Option<ProviderAccountUsageSnapshot> {
    let rate_limits = value
        .get("rate_limits")
        .or_else(|| value.get("rateLimits"))?;
    let observed_at_ms = unix_epoch_ms();
    let mut meters = Vec::new();
    if let Some(limit) = rate_limits
        .get("five_hour")
        .or_else(|| rate_limits.get("fiveHour"))
    {
        if let Some(meter) = status_line_meter("five_hour", "5-hour", 5 * 60, limit, observed_at_ms)
        {
            meters.push(meter);
        }
    }
    if let Some(limit) = rate_limits
        .get("seven_day")
        .or_else(|| rate_limits.get("sevenDay"))
    {
        if let Some(meter) =
            status_line_meter("seven_day", "Weekly", 7 * 24 * 60, limit, observed_at_ms)
        {
            meters.push(meter);
        }
    }
    if meters.is_empty() {
        return None;
    }
    Some(ProviderAccountUsageSnapshot {
        profile_id: String::new(),
        provider: "claude".to_string(),
        availability: ProviderAccountUsageAvailability::Available,
        meters,
        observed_at_ms: Some(observed_at_ms),
        source: "claude.status_line".to_string(),
        management_url: Some("https://claude.ai/settings/usage".to_string()),
    })
}

fn status_line_meter(
    meter_id: &str,
    label: &str,
    window_duration_minutes: u64,
    value: &Value,
    observed_at_ms: u64,
) -> Option<ProviderAccountUsageMeter> {
    let used_percent = value
        .get("used_percentage")
        .or_else(|| value.get("usedPercentage"))
        .and_then(Value::as_f64)?;
    let resets_at_ms = value
        .get("resets_at")
        .or_else(|| value.get("resetsAt"))
        .and_then(timestamp_ms);
    Some(ProviderAccountUsageMeter {
        meter_id: format!("rate_limit/{meter_id}"),
        label: label.to_string(),
        service_id: None,
        kind: ProviderAccountUsageMeterKind::RollingLimit,
        scope: ProviderAccountUsageMeterScope::Account,
        used_percent: Some(used_percent),
        used: None,
        remaining: None,
        total: None,
        unit: None,
        window_duration_minutes: Some(window_duration_minutes),
        resets_at_ms,
        state: meter_state(used_percent),
        source: "claude.status_line".to_string(),
        observed_at_ms,
    })
}

pub(super) fn timestamp_ms(value: &Value) -> Option<u64> {
    if let Some(value) = value.as_u64() {
        return Some(if value < 10_000_000_000 {
            value * 1_000
        } else {
            value
        });
    }
    if let Some(value) = value.as_f64() {
        return (value > 0.0).then(|| {
            let value = value as u64;
            if value < 10_000_000_000 {
                value * 1_000
            } else {
                value
            }
        });
    }
    value
        .as_str()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .and_then(|value| u64::try_from(value.timestamp_millis()).ok())
}

fn meter_state(used_percent: f64) -> ProviderAccountUsageMeterState {
    if used_percent >= 100.0 {
        ProviderAccountUsageMeterState::Exhausted
    } else if used_percent >= 80.0 {
        ProviderAccountUsageMeterState::Warning
    } else {
        ProviderAccountUsageMeterState::Healthy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_official_claude_subscription_windows() {
        let usage = claude_status_line_usage_snapshot(&serde_json::json!({
            "rate_limits": {
                "five_hour": {
                    "used_percentage": 84.0,
                    "resets_at": "2027-01-15T12:00:00Z"
                },
                "seven_day": {
                    "used_percentage": 31.0,
                    "resets_at": 1_800_000_000
                }
            }
        }))
        .expect("usage snapshot");

        assert_eq!(usage.meters.len(), 2);
        assert_eq!(usage.meters[0].label, "5-hour");
        assert_eq!(
            usage.meters[0].state,
            ProviderAccountUsageMeterState::Warning
        );
        assert_eq!(usage.meters[1].label, "Weekly");
        assert_eq!(usage.meters[1].resets_at_ms, Some(1_800_000_000_000));
    }

    #[test]
    fn accepts_each_claude_window_independently() {
        let usage = claude_status_line_usage_snapshot(&serde_json::json!({
            "rate_limits": {
                "seven_day": { "used_percentage": 100.0 }
            }
        }))
        .expect("weekly-only usage snapshot");

        assert_eq!(usage.meters.len(), 1);
        assert_eq!(usage.meters[0].meter_id, "rate_limit/seven_day");
        assert_eq!(
            usage.meters[0].state,
            ProviderAccountUsageMeterState::Exhausted
        );
    }

    #[test]
    fn parses_camel_case_status_line_windows() {
        let usage = claude_status_line_usage_snapshot(&serde_json::json!({
            "rateLimits": {
                "fiveHour": {
                    "usedPercentage": 12.0,
                    "resetsAt": 1_800_000_000_000u64
                }
            }
        }))
        .expect("camel-case usage snapshot");

        assert_eq!(usage.meters.len(), 1);
        assert_eq!(usage.meters[0].meter_id, "rate_limit/five_hour");
        assert_eq!(usage.meters[0].used_percent, Some(12.0));
        assert_eq!(usage.meters[0].resets_at_ms, Some(1_800_000_000_000));
        assert_eq!(usage.meters[0].window_duration_minutes, Some(5 * 60));
    }

    #[test]
    fn merges_windows_without_discarding_the_other_period() {
        let mut usage = claude_status_line_usage_snapshot(&serde_json::json!({
            "rate_limits": {
                "five_hour": { "used_percentage": 20.0 }
            }
        }));
        let weekly = claude_status_line_usage_snapshot(&serde_json::json!({
            "rate_limits": {
                "seven_day": { "used_percentage": 45.0 }
            }
        }))
        .expect("weekly usage");

        merge_claude_account_usage(&mut usage, weekly);

        let usage = usage.expect("merged usage");
        assert_eq!(usage.meters.len(), 2);
        assert_eq!(usage.meters[0].meter_id, "rate_limit/five_hour");
        assert_eq!(usage.meters[1].meter_id, "rate_limit/seven_day");
    }
}
