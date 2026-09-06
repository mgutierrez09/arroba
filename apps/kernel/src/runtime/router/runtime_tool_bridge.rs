use super::CommandRouter;
use crate::error::DaemonError;

impl CommandRouter {
    pub(crate) fn runtime_mcp_bind_address(&self) -> (String, u16) {
        let config = self.config_projection.snapshot();
        (config.runtime_mcp_host, config.runtime_mcp_port)
    }

    pub(crate) async fn dispatch_authenticated_mcp_proxy_call(
        &self,
        auth_token: &str,
        name: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, DaemonError> {
        self.runtime_state
            .dispatch_authenticated_mcp_proxy_call(
                &self.provider_run_projection,
                auth_token,
                name,
                payload,
            )
            .await
    }

    pub(crate) async fn dispatch_authenticated_runtime_tool_call(
        &self,
        auth_token: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        if crate::transport::runtime_tools::canonical_meta_tool_name(tool_name)
            == Some(crate::transport::runtime_tools::META_RUN_COMMAND_TOOL)
        {
            return self.dispatch_meta_run_command(auth_token, arguments).await;
        }
        self.runtime_state
            .dispatch_authenticated_runtime_tool_call(auth_token, tool_name, arguments)
            .await
    }

    pub(crate) fn runtime_tool_specs_for_auth_token(
        &self,
        auth_token: &str,
    ) -> Vec<crate::transport::runtime_tools::RuntimeToolSpec> {
        self.runtime_state
            .runtime_tool_specs_for_auth_token(auth_token)
    }

    pub(crate) async fn dispatch_forwarded_workflow_runtime_tool_call(
        &self,
        context: crate::execution_lease::RemoteWorkflowTurnContext,
        tool_name: String,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        self.runtime_state
            .dispatch_forwarded_workflow_runtime_tool_call(context, tool_name, arguments)
            .await
    }

    pub(crate) async fn dispatch_forwarded_workspace_live_sync_runtime_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
        metadata: crate::transport::relay_peer::RemoteWorkspaceLiveSyncInvocationMetadata,
        tool_name: String,
        arguments: serde_json::Value,
        artifact_states: Vec<crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState>,
    ) -> Result<
        (
            crate::transport::runtime_tools::RuntimeToolResult,
            Vec<crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState>,
        ),
        DaemonError,
    > {
        self.runtime_state
            .dispatch_forwarded_workspace_live_sync_runtime_tool_call(
                context,
                metadata,
                tool_name,
                arguments,
                artifact_states,
            )
            .await
    }

    pub(crate) async fn finalize_forwarded_workspace_live_sync_runtime_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
        metadata: crate::transport::relay_peer::RemoteWorkspaceLiveSyncInvocationMetadata,
        tool_name: String,
        arguments: serde_json::Value,
        initial_artifact_states: Vec<
            crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState,
        >,
        final_artifact_states: Vec<
            crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState,
        >,
    ) -> Result<(), DaemonError> {
        self.runtime_state
            .finalize_forwarded_workspace_live_sync_runtime_tool_call(
                context,
                metadata,
                tool_name,
                arguments,
                initial_artifact_states,
                final_artifact_states,
            )
            .await
    }

    pub(crate) async fn dispatch_forwarded_capability_runtime_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
        tool_name: String,
        arguments: serde_json::Value,
    ) -> Result<
        (
            crate::transport::runtime_tools::RuntimeToolResult,
            Option<crate::skill::CharioxSkillPackage>,
            crate::extension::RemoteExtensionManifest,
        ),
        DaemonError,
    > {
        self.runtime_state
            .dispatch_forwarded_capability_runtime_tool_call(context, tool_name, arguments)
            .await
    }

    pub(crate) async fn dispatch_forwarded_meta_runtime_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
        tool_name: String,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        if crate::transport::runtime_tools::canonical_meta_tool_name(&tool_name)
            == Some(crate::transport::runtime_tools::META_RUN_COMMAND_TOOL)
        {
            return self
                .dispatch_forwarded_meta_run_command(context, arguments)
                .await;
        }
        self.runtime_state
            .dispatch_forwarded_meta_runtime_tool_call(context, tool_name, arguments)
            .await
    }

    pub(crate) async fn dispatch_forwarded_home_extension_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteExtensionInvocationContext,
        metadata: crate::extension::RemoteExtensionInvocationMetadata,
        tool: crate::extension::RemoteExtensionTool,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        self.runtime_state
            .dispatch_forwarded_home_extension_tool_call(context, metadata, tool, arguments)
            .await
    }

    pub(crate) async fn dispatch_forwarded_home_mcp_proxy_call(
        &self,
        context: crate::transport::relay_peer::RemoteExtensionInvocationContext,
        metadata: crate::extension::RemoteExtensionInvocationMetadata,
        name: String,
        tool: crate::extension::RemoteExtensionTool,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, DaemonError> {
        self.runtime_state
            .dispatch_forwarded_home_mcp_proxy_call(context, metadata, name, tool, payload)
            .await
    }

    pub(crate) async fn cancel_forwarded_home_extension_invocation(
        &self,
        context: crate::transport::relay_peer::RemoteExtensionInvocationContext,
        metadata: crate::extension::RemoteExtensionInvocationMetadata,
    ) -> Result<bool, DaemonError> {
        self.runtime_state
            .cancel_forwarded_home_extension_invocation(context, metadata)
            .await
    }

    pub(crate) async fn dispatch_forwarded_home_credential_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteExtensionInvocationContext,
        tool_name: String,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        self.runtime_state
            .dispatch_forwarded_home_credential_tool_call(context, tool_name, arguments)
            .await
    }

    pub(crate) async fn resolve_forwarded_home_credential_secret(
        &self,
        context: crate::transport::relay_peer::RemoteExtensionInvocationContext,
        credential_id: String,
        injection: crate::transport::relay_peer::RemoteCredentialSecretInjection,
    ) -> Result<(String, String), DaemonError> {
        self.runtime_state
            .resolve_forwarded_home_credential_secret(context, credential_id, injection)
            .await
    }
}
