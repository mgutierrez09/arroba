use std::time::Duration;

use chariox_relay::protocol::ClientTarget;

use super::*;

impl KernelRuntimeState {
    pub(super) async fn try_dispatch_remote_room_browser_runtime_tool_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<Option<crate::transport::runtime_tools::RuntimeToolResult>, DaemonError> {
        if !is_room_browser_controller_runtime_tool(tool_name) {
            return Ok(None);
        }
        let Some(context) = self
            .with_app_side_effect(|app| {
                crate::app::RemoteLeaseRuntime::new(app)
                    .leased_extension_invocation_context_for_runtime_provider_run(provider_run)
            })
            .await
        else {
            return Ok(None);
        };
        let relay_config = self.with_app_side_effect(|app| app.config().clone()).await;
        let response_timeout =
            room_browser_runtime_tool_response_timeout(&relay_config, tool_name, &arguments);
        let response = crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
            &relay_config,
            ClientTarget {
                daemon_id: Some(context.home_kernel_id.clone()),
                daemon_alias: None,
            },
            RelayPeerRequest::ForwardRoomBrowserRuntimeTool {
                context,
                call: crate::transport::relay_peer::RemoteRoomBrowserRuntimeToolCall {
                    tool_name: tool_name.to_string(),
                    arguments,
                },
            },
            response_timeout,
        )
        .await?;
        match response {
            RelayPeerResponse::RoomBrowserRuntimeToolHandled { result } => Ok(Some(result.0)),
            other => Err(DaemonError::LocalTransport {
                operation: "remote Room browser runtime tool",
                message: format!("unexpected home Room browser response: {other:?}"),
            }),
        }
    }
}

fn room_browser_runtime_tool_response_timeout(
    config: &crate::config::DaemonConfig,
    tool_name: &str,
    arguments: &serde_json::Value,
) -> Duration {
    let default_timeout = Duration::from_millis(config.relay_request_timeout_ms);
    if canonical_room_browser_runtime_tool(tool_name)
        == Some(crate::transport::runtime_tools::PASTE_SECRET_TO_SLICE_TOOL)
    {
        // 300-second home vault unlock, browser work and relay response buffer.
        return std::cmp::max(default_timeout, Duration::from_secs(345));
    }
    if !matches!(
        tool_name,
        crate::transport::runtime_tools::SLICE_BROWSER_WAIT_FOR_TEXT_TOOL
            | crate::transport::runtime_tools::SLICE_BROWSER_WAIT_FOR_SELECTOR_TOOL
            | crate::transport::runtime_tools::SLICE_BROWSER_WAIT_FOR_IDLE_TOOL
    ) {
        return default_timeout;
    }
    let requested_timeout = arguments
        .get("timeout_ms")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(10_000)
        .clamp(100, 60_000);
    std::cmp::max(
        default_timeout,
        Duration::from_millis(requested_timeout).saturating_add(Duration::from_secs(15)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_secret_forwarding_waits_for_home_vault_unlock() {
        let mut config = crate::config::DaemonConfig::for_tests();
        for name in [
            crate::transport::runtime_tools::PASTE_SECRET_TO_SLICE_TOOL,
            crate::transport::runtime_tools::PASTE_SECRET_TO_SLICE_TOOL_ALIAS,
        ] {
            config.relay_request_timeout_ms = 1_000;
            assert_eq!(
                room_browser_runtime_tool_response_timeout(&config, name, &serde_json::json!({})),
                Duration::from_secs(345)
            );
            config.relay_request_timeout_ms = 400_000;
            assert_eq!(
                room_browser_runtime_tool_response_timeout(&config, name, &serde_json::json!({})),
                Duration::from_secs(400)
            );
        }
    }

    #[test]
    fn browser_wait_forwarding_keeps_a_relay_response_buffer() {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.relay_request_timeout_ms = 1_000;
        assert_eq!(
            room_browser_runtime_tool_response_timeout(
                &config,
                crate::transport::runtime_tools::SLICE_BROWSER_WAIT_FOR_TEXT_TOOL,
                &serde_json::json!({"timeout_ms": 60_000}),
            ),
            Duration::from_secs(75),
        );
    }
}
