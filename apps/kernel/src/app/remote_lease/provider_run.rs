use crate::error::DaemonError;
use crate::execution_lease::LeasedAgent;
use crate::provider::LaunchProviderRequest;
use crate::transport::relay_peer::RequiredRemoteMcp;

use super::mcp_availability::provider_run_mcp_set_matches;
use super::RemoteLeaseRuntime;

pub(crate) enum LeasedProviderRunMatch {
    Ready(String),
    LaunchRequired(LaunchProviderRequest),
}

impl<'a> RemoteLeaseRuntime<'a> {
    pub(super) fn ensure_home_proxy_manifest_has_no_worker_collisions(
        &self,
        leased_agent: &LeasedAgent,
        required_mcps: &[RequiredRemoteMcp],
        remote_extension_manifest: &crate::extension::RemoteExtensionManifest,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        remote_extension_manifest.validate_unique_tool_names(operation)?;
        if remote_extension_manifest.is_empty() {
            return Ok(());
        }

        let _session = self
            .app
            .sessions
            .get_session(&leased_agent.backing_session_id)?;
        let backing_agent = self.app.agents.get_agent(&leased_agent.backing_agent_id)?;
        let mut worker_tool_names = std::collections::BTreeMap::<String, String>::new();
        for spec in crate::transport::runtime_tools::workspace_live_sync_runtime_tool_specs()
            .into_iter()
            .chain(crate::transport::runtime_tools::extension_runtime_tool_specs())
            .chain(crate::transport::runtime_tools::recall_runtime_tool_specs())
            .chain(crate::transport::runtime_tools::credential_runtime_tool_specs())
            .chain(crate::transport::runtime_tools::workflow_runtime_tool_specs())
        {
            worker_tool_names.insert(spec.name, "worker runtime tool".to_string());
        }

        let mcp_roots = crate::mcp::CharioxMcpRegistry::user_root()
            .map(|root| vec![root])
            .unwrap_or_default();
        let mcp_registry = crate::mcp::CharioxMcpRegistry::new(mcp_roots);
        for name in remote_extension_manifest.home_proxy_mcp_server_names() {
            if required_mcps
                .iter()
                .any(|required| required.config.name == name)
                || mcp_registry.get(name)?.is_some()
            {
                return Err(DaemonError::LocalTransport {
                    operation,
                    message: format!(
                        "home-proxy MCP `{name}` collides with a worker-local MCP; rename one before launching the remote agent"
                    ),
                });
            }
        }

        let script_roots = crate::script::CharioxScriptRegistry::user_root()
            .map(|root| vec![root])
            .unwrap_or_default();
        let script_registry = crate::script::CharioxScriptRegistry::new(script_roots);
        for grant in backing_agent.script_grants() {
            if let Some(script) = script_registry.get(&grant.name)? {
                worker_tool_names
                    .insert(script.name, format!("worker-local script `{}`", grant.name));
            }
        }

        let connector_registry = crate::connector::CharioxConnectorRegistry::user()?;
        for grant in backing_agent.connector_grants() {
            let Some(connector) = connector_registry.get(&grant.name)? else {
                continue;
            };
            let max_safety = crate::connector::ConnectorSafety::parse(grant.max_safety.as_deref())?;
            for operation_config in connector.operations {
                if operation_config.safety > max_safety {
                    continue;
                }
                worker_tool_names.insert(
                    crate::connector::connector_tool_name(&connector.name, &operation_config.name),
                    format!("worker-local connector `{}`", connector.name),
                );
            }
        }

        for tool in &remote_extension_manifest.tools {
            if tool.execution_location != crate::extension::ExtensionExecutionLocation::Home {
                continue;
            }
            if let Some(worker_source) = worker_tool_names.get(&tool.tool_name) {
                return Err(DaemonError::LocalTransport {
                    operation,
                    message: format!(
                        "home-proxy extension tool `{}` (`{}:{}`) collides with {worker_source}; rename one before launching the remote agent",
                        tool.tool_name,
                        tool.kind.as_str(),
                        tool.name
                    ),
                });
            }
        }

        Ok(())
    }

    pub(crate) fn prepare_leased_provider_run_matches_mcps(
        &mut self,
        leased_agent: &LeasedAgent,
        required_mcps: &[RequiredRemoteMcp],
        remote_extension_manifest: &crate::extension::RemoteExtensionManifest,
        event_reply_enabled: bool,
        event_context_enabled: bool,
        event_actions_enabled: bool,
    ) -> Result<LeasedProviderRunMatch, DaemonError> {
        self.ensure_home_proxy_manifest_has_no_worker_collisions(
            leased_agent,
            required_mcps,
            remote_extension_manifest,
            "remote provider launch",
        )?;
        let lease = self
            .app
            .execution_leases
            .get(&leased_agent.lease_id)
            .cloned()
            .ok_or_else(|| DaemonError::ExecutionLeaseNotFound {
                lease_id: leased_agent.lease_id.clone(),
            })?;
        let existing = self.app.providers.get_run_for_agent(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        );
        let existing_profile_matches = existing.as_ref().is_some_and(|run| {
            run.provider() == leased_agent.provider
                && run.account_profile() == leased_agent.account_profile
                && run.model() == leased_agent.model.as_deref().unwrap_or("default")
                && run.variant() == leased_agent.effort.as_deref()
        });
        if let Some(run) = existing.as_ref() {
            let mcp_matches = provider_run_mcp_set_matches(run, required_mcps)?;
            let reply_capability_matches =
                run.workflow_event_reply_enabled() == event_reply_enabled;
            let context_capability_matches =
                run.workflow_event_context_enabled() == event_context_enabled;
            let actions_capability_matches =
                run.workflow_event_actions_enabled() == event_actions_enabled;
            if existing_profile_matches
                && mcp_matches
                && reply_capability_matches
                && context_capability_matches
                && actions_capability_matches
            {
                if !remote_extension_manifest.is_empty() {
                    let updated = self.app.providers.update_run_remote_extension_manifest(
                        run.id(),
                        remote_extension_manifest.clone(),
                    )?;
                    self.app.update_provider_run_projection(updated);
                }
                return Ok(LeasedProviderRunMatch::Ready(run.id().to_string()));
            }
            let active = self
                .app
                .prompt_owner_active_prompt_for_agent(
                    &leased_agent.backing_session_id,
                    &leased_agent.backing_agent_id,
                )?
                .is_some();
            let queued = self
                .app
                .prompt_owner_peek_next_queued_prompt(
                    &leased_agent.backing_session_id,
                    &leased_agent.backing_agent_id,
                )?
                .is_some();
            if !existing_profile_matches && (active || queued) {
                return Err(DaemonError::LocalTransport {
                    operation: "remote provider profile reconciliation",
                    message: "the worker provider run differs from the selected profile and still has pending work; settle or cancel it before retrying".to_string(),
                });
            }
            if active && !mcp_matches {
                return Err(DaemonError::LocalTransport {
                    operation: "remote MCP provider reload",
                    message: format!(
                        "remote worker provider run `{}` does not have the required MCP set and is currently busy; retry after the active turn completes",
                        run.id()
                    ),
                });
            }
            // Provider tools/list is a run-level snapshot. If only the
            // event capability differs, let a busy run finish its
            // already-admitted prompt and rotate it on the next idle boundary.
            // This preserves FIFO queueing without advertising a capability
            // change halfway through an active provider turn.
            if active {
                return Ok(LeasedProviderRunMatch::Ready(run.id().to_string()));
            }
            let run_id = run.id().to_string();
            let _ = crate::app::provider_runtime::ProviderProcessTracker::new(self.app)
                .remove_run(&run_id);
            if let Ok(outcome) = self
                .app
                .providers
                .terminate_run_provider_only(run.session_id(), run.id())
            {
                let _ = self
                    .app
                    .sessions
                    .set_active_provider_run(outcome.run().session_id(), None);
                self.app.update_provider_run_projection(outcome.into_run());
            }
        }

        let mut request = LaunchProviderRequest::new(
            &leased_agent.backing_session_id,
            &leased_agent.provider,
            &leased_agent.provider,
            &leased_agent.account_profile,
            leased_agent
                .model
                .clone()
                .unwrap_or_else(|| "default".to_string()),
        )
        .with_agent_id(&leased_agent.backing_agent_id)
        .with_owner_user_id(lease.owner_user_id)
        .with_workflow_event_reply(event_reply_enabled)
        .with_workflow_event_context(event_context_enabled)
        .with_workflow_event_actions(event_actions_enabled)
        .with_working_directory(std::path::PathBuf::from(
            self.app
                .sessions
                .get_session(&leased_agent.backing_session_id)?
                .worktree_id(),
        ))
        .with_mcp_servers(
            required_mcps
                .iter()
                .map(|required| required.config.clone())
                .collect(),
        );
        let mut mcp_servers = request.mcp_servers.clone();
        for name in remote_extension_manifest.home_proxy_mcp_server_names() {
            if !mcp_servers.iter().any(|server| server.name == name) {
                mcp_servers.push(crate::mcp::CharioxMcpServerConfig::streamable_http(
                    name,
                    "http://127.0.0.1/mcp",
                ));
            }
        }
        request = request
            .with_mcp_servers(mcp_servers)
            .with_remote_extension_manifest(remote_extension_manifest.clone());
        if let Some(execution_mode) = leased_agent.execution_mode {
            request = request.with_execution_mode(execution_mode);
        }
        if let Some(permission_level) = leased_agent.permission_level {
            request = request.with_permission_level(permission_level);
        }
        if leased_agent.effort.is_some() {
            request = request.with_variant(leased_agent.effort.clone());
        }
        if let Some(run) = existing.as_ref().filter(|_| existing_profile_matches) {
            request = request.with_resume_state(run.resume_state().clone());
            if request.variant.is_none() {
                request = request.with_variant(run.variant().map(str::to_string));
            }
        }
        let session = self
            .app
            .sessions
            .get_session(&leased_agent.backing_session_id)?;
        request = request.with_workspace_live_sync_mode(
            crate::provider::provider_workspace_live_sync_mode_for_session(
                &leased_agent.provider,
                self.app.config(),
                Some(&session),
            ),
        );
        Ok(LeasedProviderRunMatch::LaunchRequired(request))
    }
}
