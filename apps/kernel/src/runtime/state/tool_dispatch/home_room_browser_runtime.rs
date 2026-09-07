use super::*;

impl KernelRuntimeState {
    pub(crate) async fn dispatch_forwarded_room_browser_runtime_tool_call(
        &self,
        from_worker_kernel_id: &str,
        context: crate::transport::relay_peer::RemoteExtensionInvocationContext,
        call: crate::transport::relay_peer::RemoteRoomBrowserRuntimeToolCall,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let operation = "forwarded Room browser runtime tool";
        let canonical_tool_name =
            canonical_room_browser_runtime_tool(&call.tool_name).ok_or_else(|| {
                DaemonError::LocalTransport {
                    operation,
                    message: format!("unsupported Room browser runtime tool `{}`", call.tool_name),
                }
            })?;
        let expected_worker_kernel_id =
            context
                .worker_kernel_id
                .as_deref()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation,
                    message: "forwarded Room browser context omitted the worker kernel".to_string(),
                })?;
        if from_worker_kernel_id != expected_worker_kernel_id {
            return Err(DaemonError::LocalTransport {
                operation,
                message: "relay sender does not match the bound worker kernel".to_string(),
            });
        }
        let agent = super::home_extension_authorizer::authorize_remote_home_context(
            self, &context, operation,
        )?;
        let slice = self
            .owned
            .slice_store
            .environment_slice(&context.home_session_id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation,
                message: "the home Room has no reserved browser slice".to_string(),
            })?;
        if slice.worker_kernel_id.as_deref() != Some(expected_worker_kernel_id)
            || slice.worker_machine_id.as_deref() != context.worker_machine_id.as_deref()
        {
            return Err(DaemonError::LocalTransport {
                operation,
                message: "the bound worker does not own the home Room browser slice".to_string(),
            });
        }
        if !slice
            .agent_ids
            .iter()
            .any(|agent_id| agent_id == agent.id())
        {
            return Err(DaemonError::LocalTransport {
                operation,
                message: "the home agent is not attached to the Room browser slice".to_string(),
            });
        }
        if !self.browser_controller_enabled_for_room(&context.home_session_id) {
            return Err(DaemonError::LocalTransport {
                operation,
                message: "the home Room does not use the long-running browser controller"
                    .to_string(),
            });
        }
        self.dispatch_room_browser_controller_runtime_tool_call(
            &context.home_session_id,
            &slice.id,
            &context.home_agent_id,
            canonical_tool_name,
            call.arguments,
        )
        .await
    }
}
