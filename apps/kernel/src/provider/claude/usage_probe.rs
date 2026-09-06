use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use wait_timeout::ChildExt;

use crate::account_profile::ProviderAccountUsageSnapshot;
use crate::error::DaemonError;

use super::mcp_config::create_claude_runtime_files_root;

const CLAUDE_USAGE_PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const CLAUDE_USAGE_PROBE_DIAGNOSTIC_BYTES: usize = 4 * 1024;

pub(crate) fn probe_claude_account_usage(
    executable: &Path,
    account_profile: &str,
    environment: &BTreeMap<String, String>,
) -> Result<ProviderAccountUsageSnapshot, DaemonError> {
    probe_claude_account_usage_with_timeout(
        executable,
        account_profile,
        environment,
        CLAUDE_USAGE_PROBE_TIMEOUT,
    )
}

fn probe_claude_account_usage_with_timeout(
    executable: &Path,
    account_profile: &str,
    environment: &BTreeMap<String, String>,
    timeout: Duration,
) -> Result<ProviderAccountUsageSnapshot, DaemonError> {
    validate_claude_probe_environment(environment, linux_profile_home_is_supported())?;
    let root = create_claude_runtime_files_root()?;
    let stdout_path = root.path().join("usage-result.json");
    let stderr_path = root.path().join("usage-stderr.log");
    let stdout = fs::File::create(&stdout_path)
        .map_err(|error| probe_error(format!("failed to prepare Claude stdout: {error}")))?;
    let stderr = fs::File::create(&stderr_path)
        .map_err(|error| probe_error(format!("failed to prepare Claude stderr: {error}")))?;

    let mut command = Command::new(executable);
    command.args([
        "-p",
        "/usage",
        "--output-format",
        "json",
        "--no-session-persistence",
        "--no-chrome",
        "--tools",
        "",
    ]);
    command.current_dir(root.path());
    for (name, value) in environment {
        command.env(name, value);
    }
    for name in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_BASE_URL",
        "ANTHROPIC_CUSTOM_HEADERS",
    ] {
        command.env_remove(name);
    }
    command.env("DISABLE_AUTOUPDATER", "1");
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));

    let mut child = command
        .spawn()
        .map_err(|error| probe_error(format!("failed to start Claude: {error}")))?;
    let status = match child.wait_timeout(timeout) {
        Ok(Some(status)) => status,
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(probe_error(format!(
                "Claude did not report usage within {} seconds{}",
                timeout.as_secs(),
                diagnostic_suffix(&stderr_path)
            )));
        }
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(probe_error(format!(
                "failed to wait for Claude usage: {error}"
            )));
        }
    };
    if !status.success() {
        return Err(probe_error(format!(
            "Claude exited before reporting usage ({status}){}",
            diagnostic_suffix(&stderr_path)
        )));
    }

    let output = fs::read(&stdout_path)
        .map_err(|error| probe_error(format!("failed to read Claude usage: {error}")))?;
    let result: serde_json::Value = serde_json::from_slice(&output).map_err(|error| {
        probe_error(format!(
            "Claude returned invalid usage JSON: {error}{}",
            diagnostic_suffix(&stderr_path)
        ))
    })?;
    let text = validated_claude_usage_result(&result)?;
    let mut usage = claude_usage_snapshot_from_text(text).ok_or_else(|| {
        probe_error("Claude did not return subscription usage windows".to_string())
    })?;
    usage.profile_id = account_profile.to_string();
    usage.source = "claude.native_usage".to_string();
    for meter in &mut usage.meters {
        meter.source = "claude.native_usage".to_string();
    }
    Ok(usage)
}

fn validated_claude_usage_result(value: &serde_json::Value) -> Result<&str, DaemonError> {
    let success = value.get("type").and_then(serde_json::Value::as_str) == Some("result")
        && value.get("subtype").and_then(serde_json::Value::as_str) == Some("success")
        && value.get("is_error").and_then(serde_json::Value::as_bool) == Some(false);
    if !success {
        return Err(probe_error(
            "Claude returned an unsuccessful usage result".to_string(),
        ));
    }
    let no_model_activity = value
        .get("duration_api_ms")
        .and_then(serde_json::Value::as_u64)
        == Some(0)
        && value.get("num_turns").and_then(serde_json::Value::as_u64) == Some(0)
        && value
            .get("total_cost_usd")
            .and_then(serde_json::Value::as_f64)
            == Some(0.0)
        && [
            "input_tokens",
            "cache_creation_input_tokens",
            "cache_read_input_tokens",
            "output_tokens",
        ]
        .into_iter()
        .all(|field| {
            value
                .get("usage")
                .and_then(|usage| usage.get(field))
                .and_then(serde_json::Value::as_u64)
                == Some(0)
        });
    if !no_model_activity {
        return Err(probe_error(
            "Claude /usage unexpectedly performed model activity".to_string(),
        ));
    }
    value
        .get("result")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| probe_error("Claude usage result text is unavailable".to_string()))
}

fn claude_usage_snapshot_from_text(text: &str) -> Option<ProviderAccountUsageSnapshot> {
    let mut rate_limits = serde_json::Map::new();
    for line in text.lines().map(str::trim) {
        if let Some(percent) = usage_percent(line, "Current session:") {
            rate_limits.insert(
                "five_hour".to_string(),
                serde_json::json!({ "used_percentage": percent }),
            );
        } else if let Some(percent) = usage_percent(line, "Current week (all models):")
            .or_else(|| usage_percent(line, "Current week:"))
        {
            rate_limits.insert(
                "seven_day".to_string(),
                serde_json::json!({ "used_percentage": percent }),
            );
        }
    }
    crate::provider::claude_status_line_usage_snapshot(&serde_json::json!({
        "rate_limits": rate_limits,
    }))
}

fn usage_percent(line: &str, prefix: &str) -> Option<f64> {
    let percent = line
        .strip_prefix(prefix)?
        .trim_start()
        .split_once('%')?
        .0
        .trim()
        .parse::<f64>()
        .ok()?;
    (percent.is_finite() && (0.0..=100.0).contains(&percent)).then_some(percent)
}

fn diagnostic_suffix(path: &Path) -> String {
    let Ok(mut bytes) = fs::read(path) else {
        return String::new();
    };
    if bytes.len() > CLAUDE_USAGE_PROBE_DIAGNOSTIC_BYTES {
        bytes.drain(..bytes.len() - CLAUDE_USAGE_PROBE_DIAGNOSTIC_BYTES);
    }
    let diagnostic = terminal_diagnostic(&bytes);
    if diagnostic.is_empty() {
        String::new()
    } else {
        format!("; Claude stderr: {diagnostic}")
    }
}

fn validate_claude_probe_environment(
    environment: &BTreeMap<String, String>,
    profile_home_is_supported: bool,
) -> Result<(), DaemonError> {
    if environment.contains_key("HOME") && !profile_home_is_supported {
        return Err(probe_error(
            "HOME-based Claude account profiles are supported only on Linux; refusing to override HOME on this platform"
                .to_string(),
        ));
    }
    Ok(())
}

fn linux_profile_home_is_supported() -> bool {
    cfg!(target_os = "linux")
}

fn terminal_diagnostic(output: &[u8]) -> String {
    let text = String::from_utf8_lossy(output);
    let mut cleaned = String::with_capacity(text.len());
    let mut escape = false;
    for character in text.chars() {
        if escape {
            if character.is_ascii_alphabetic() || character == '\u{7}' {
                escape = false;
            }
            continue;
        }
        if character == '\u{1b}' {
            escape = true;
        } else if character.is_control() {
            cleaned.push(' ');
        } else {
            cleaned.push(character);
        }
    }
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn probe_error(message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "refresh Claude usage",
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[cfg(unix)]
    #[test]
    fn probes_both_usage_windows_without_replacing_home() {
        if Command::new("node").arg("--version").output().is_err() {
            return;
        }
        let fixture = std::env::temp_dir().join(format!(
            "chariox-claude-usage-probe-test-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        fs::create_dir(&fixture).expect("fixture root");
        let executable = fixture.join("fake-claude.mjs");
        let observed_environment = fixture.join("environment.json");
        let claude_config_dir = fixture.join("claude-account");
        let mut file = fs::File::create(&executable).expect("fake Claude executable");
        file.write_all(
            br#"#!/usr/bin/env node
import { writeFileSync } from "node:fs"
const expected = ["-p", "/usage", "--output-format", "json", "--no-session-persistence", "--no-chrome", "--tools", ""]
if (JSON.stringify(process.argv.slice(2)) !== JSON.stringify(expected)) {
  throw new Error(`unexpected Claude usage arguments: ${JSON.stringify(process.argv.slice(2))}`)
}
writeFileSync(process.env.CHARIOX_PROBE_TEST_ENV_FILE, JSON.stringify({
  home: process.env.HOME,
  claudeConfigDir: process.env.CLAUDE_CONFIG_DIR,
  anthropicApiKey: process.env.ANTHROPIC_API_KEY,
  anthropicAuthToken: process.env.ANTHROPIC_AUTH_TOKEN,
  anthropicBaseUrl: process.env.ANTHROPIC_BASE_URL,
  anthropicCustomHeaders: process.env.ANTHROPIC_CUSTOM_HEADERS
}))
process.stdout.write(JSON.stringify({
  type: "result",
  subtype: "success",
  is_error: false,
  duration_api_ms: 0,
  num_turns: 0,
  total_cost_usd: 0,
  result: "Current session: 17% used\nCurrent week (all models): 41% used \\u00b7 resets Sep 2 at 11:59pm (Europe/Helsinki)",
  usage: { input_tokens: 0, cache_creation_input_tokens: 0, cache_read_input_tokens: 0, output_tokens: 0 }
}))
"#,
        )
        .expect("fake Claude source");
        drop(file);
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))
            .expect("fake Claude permissions");
        let inherited_home = std::env::var("HOME").expect("test HOME");
        let environment = BTreeMap::from([
            (
                "CLAUDE_CONFIG_DIR".to_string(),
                claude_config_dir.display().to_string(),
            ),
            (
                "CHARIOX_PROBE_TEST_ENV_FILE".to_string(),
                observed_environment.display().to_string(),
            ),
            ("ANTHROPIC_API_KEY".to_string(), "wrong-api-key".to_string()),
            (
                "ANTHROPIC_AUTH_TOKEN".to_string(),
                "wrong-auth-token".to_string(),
            ),
            (
                "ANTHROPIC_BASE_URL".to_string(),
                "https://wrong.invalid".to_string(),
            ),
            (
                "ANTHROPIC_CUSTOM_HEADERS".to_string(),
                "x-wrong: yes".to_string(),
            ),
        ]);

        let usage = probe_claude_account_usage_with_timeout(
            &executable,
            "claude-2",
            &environment,
            Duration::from_secs(5),
        )
        .expect("usage probe");

        assert_eq!(usage.profile_id, "claude-2");
        assert_eq!(usage.meters.len(), 2);
        assert_eq!(usage.meters[0].used_percent, Some(17.0));
        assert_eq!(usage.meters[1].used_percent, Some(41.0));
        assert_eq!(usage.source, "claude.native_usage");
        let observed: serde_json::Value =
            serde_json::from_slice(&fs::read(&observed_environment).expect("observed environment"))
                .expect("environment JSON");
        assert_eq!(observed["home"], inherited_home);
        assert_eq!(
            observed["claudeConfigDir"],
            environment["CLAUDE_CONFIG_DIR"]
        );
        for key in [
            "anthropicApiKey",
            "anthropicAuthToken",
            "anthropicBaseUrl",
            "anthropicCustomHeaders",
        ] {
            assert!(observed.get(key).is_none(), "{key} must be removed");
        }
        assert!(
            !claude_config_dir.exists(),
            "the non-interactive probe must not mutate Claude profile state"
        );
        let _ = fs::remove_dir_all(fixture);
    }

    #[test]
    fn rejects_model_activity_in_usage_result() {
        let value = serde_json::json!({
            "type": "result",
            "subtype": "success",
            "is_error": false,
            "duration_api_ms": 1,
            "num_turns": 1,
            "total_cost_usd": 0.01,
            "result": "Current session: 17% used",
            "usage": {
                "input_tokens": 1,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0,
                "output_tokens": 1
            }
        });

        let error = validated_claude_usage_result(&value)
            .expect_err("model activity must fail the usage probe");

        assert!(error.to_string().contains("performed model activity"));
    }

    #[test]
    fn parses_official_claude_usage_text() {
        let usage = claude_usage_snapshot_from_text(
            "You are currently using your subscription\n\nCurrent session: 12.5% used\nCurrent week (all models): 84% used · resets Sep 2 at 11:59pm (Europe/Helsinki)",
        )
        .expect("usage snapshot");

        assert_eq!(usage.meters.len(), 2);
        assert_eq!(usage.meters[0].used_percent, Some(12.5));
        assert_eq!(usage.meters[1].used_percent, Some(84.0));
    }

    #[test]
    fn rejects_legacy_command_summary_without_subscription_windows() {
        assert!(claude_usage_snapshot_from_text(
            "Total cost: $0.0000\nTotal duration (API): 0s\nUsage: 0 input, 0 output, 0 cache read, 0 cache write",
        )
        .is_none());
    }

    #[test]
    fn rejects_account_home_override_when_profile_home_is_unsupported() {
        let environment = BTreeMap::from([("HOME".to_string(), "/tmp/account".to_string())]);

        let error = validate_claude_probe_environment(&environment, false)
            .expect_err("unsupported HOME override must fail");

        assert!(error.to_string().contains("supported only on Linux"));
    }

    #[test]
    fn terminal_diagnostics_strip_control_sequences_and_collapse_whitespace() {
        assert_eq!(
            terminal_diagnostic(b"\x1b[31mLogin\x1b[0m\r\n  required\t now"),
            "Login required now"
        );
    }
}
