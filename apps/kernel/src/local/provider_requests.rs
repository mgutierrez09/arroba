use crate::app::DaemonApp;
use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::provider::{LaunchProviderRequest, OpenCodeProviderCatalog, ProviderClientInterface};
use crate::transport::relay_client::send_peer_request_via_temporary_connection;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use chariox_relay::protocol::ClientTarget;

use super::api::{
    ApproveRemoteMachineRequest, ConfigureRelayRequest, ForgetRemoteMachineRequest,
    LaunchProviderRunRequest, ListRemoteMachineKernelsRequest, LocalDaemonResponse, RelayStatus,
    RenameRemoteMachineRequest,
};

mod blocking;
mod catalog;
mod remote_machines;

use blocking::block_on_relay_query;
pub(crate) use catalog::{
    load_provider_catalog, logout_provider_response, provider_auth_status_response,
    provider_command_catalogs_response, refresh_provider_account_profile_response,
    start_provider_login_response, PROVIDER_CATALOG_CACHE_TTL,
};
pub(crate) use remote_machines::{
    forgotten_machine_record, record_for_machine_id, remote_machine_records,
    resolve_machine_for_registry, resolve_machine_id_for_registry,
    resolve_registered_or_raw_machine_ref,
};

#[allow(dead_code)]
impl DaemonApp {
    pub(super) fn launch_provider_run_response(
        &mut self,
        request: LaunchProviderRunRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        if request.native_tui {
            if let Some(response) = remote_native_provider_run_response(self, &request)? {
                return Ok(response);
            }
        }
        let launch_request = launch_provider_request_from_local(self, request);
        let provider_run = self.launch_provider(launch_request)?;
        crate::logging::debug_with_fields(
            "daemon.local_api",
            "returning launched provider run to client",
            serde_json::json!({
                "provider_run_id": provider_run.id(),
                "session_id": provider_run.session_id(),
                "provider": provider_run.provider(),
                "model": provider_run.model(),
                "variant": provider_run.variant(),
                "state": provider_run.state().to_string(),
            }),
        );
        self.update_provider_run_projection(provider_run.clone());
        Ok(LocalDaemonResponse::ProviderRunLaunched { provider_run })
    }

    pub(crate) fn provider_catalog_response(&mut self) -> Result<LocalDaemonResponse, DaemonError> {
        if let Some(catalog) = self.cached_provider_catalog() {
            return Ok(LocalDaemonResponse::ProviderCatalog { catalog });
        }

        let catalog = load_provider_catalog(
            self.config().clone(),
            self.provider_account_profile_registry(),
            crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
            crate::local::GetProviderCatalogRequest::default(),
        )?;
        self.cache_provider_catalog(catalog.clone());
        Ok(LocalDaemonResponse::ProviderCatalog { catalog })
    }

    pub(crate) fn cached_provider_catalog(&self) -> Option<OpenCodeProviderCatalog> {
        self.provider_catalog_cache
            .get_fresh(PROVIDER_CATALOG_CACHE_TTL)
    }

    pub(crate) fn cache_provider_catalog(&mut self, catalog: OpenCodeProviderCatalog) {
        self.provider_catalog_cache.set(catalog.clone());
        self.update_provider_catalog_projection(catalog);
    }

    pub(crate) fn invalidate_provider_catalog_cache(&mut self) {
        self.provider_catalog_cache.clear();
        self.invalidate_provider_catalog_projection();
    }

    pub(super) fn provider_command_catalogs_response_for_app(
        &mut self,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        provider_command_catalogs_response()
    }

    pub(super) fn relay_status_response_for_app(
        &mut self,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::RelayStatus {
            status: self.relay_status_snapshot()?,
        })
    }

    pub(super) fn configure_relay_response(
        &mut self,
        request: ConfigureRelayRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.configure_relay(request.relay_url, request.relay_token)?;
        self.invalidate_provider_catalog_cache();
        Ok(LocalDaemonResponse::RelayConfigured {
            status: self.relay_status_snapshot()?,
        })
    }

    fn relay_status_snapshot(&self) -> Result<RelayStatus, DaemonError> {
        let relay_state = self.relay_client_state();
        let connected = block_on_relay_query(async move {
            Ok::<bool, DaemonError>(relay_state.read().await.connected())
        })?;
        Ok(RelayStatus {
            configured: self.config().relay_url.is_some() && self.config().relay_token.is_some(),
            connected,
            relay_url: self.config().relay_url.clone(),
            relay_token_configured: self.config().relay_token.is_some(),
            daemon_id: self.config().daemon_id.clone(),
            daemon_alias: self.config().daemon_alias.clone(),
            machine_id: self.config().host_machine_id.clone(),
            machine_alias: self.config().host_machine_alias.clone(),
        })
    }

    pub(super) fn list_remote_machines_response(
        &mut self,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let (machines, _) = self.remote_relay_inventory_projection_store().snapshot();
        Ok(LocalDaemonResponse::RemoteMachinesListed { machines })
    }

    pub(super) fn list_remote_machine_kernels_response(
        &mut self,
        request: ListRemoteMachineKernelsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let machine_ref = resolve_registered_or_raw_machine_ref(&request.machine_ref);
        let (_, kernels) = self.remote_relay_inventory_projection_store().snapshot();
        let kernels = kernels
            .into_iter()
            .filter(|kernel| {
                kernel.machine_id == machine_ref
                    || kernel
                        .machine_alias
                        .as_deref()
                        .is_some_and(|alias| alias == machine_ref)
            })
            .collect();
        Ok(LocalDaemonResponse::RemoteMachineKernelsListed {
            machine_ref,
            kernels,
        })
    }

    pub(super) fn approve_remote_machine_response(
        &mut self,
        request: ApproveRemoteMachineRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let config = self.config().clone();
        let live = block_on_relay_query(crate::transport::relay_discovery::list_live_machines(
            &config,
        ))
        .unwrap_or_default();
        let machine = resolve_machine_for_registry(&request.machine_ref, &live)?;
        DaemonConfig::approve_remote_machine(
            machine.machine_id.clone(),
            machine.machine_alias.clone(),
        )?;
        self.provider_catalog_cache.clear();
        let machine = record_for_machine_id(machine.machine_id, live, &config.host_machine_id)?;
        Ok(LocalDaemonResponse::RemoteMachineApproved { machine })
    }

    pub(super) fn forget_remote_machine_response(
        &mut self,
        request: ForgetRemoteMachineRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let config = self.config().clone();
        let live = block_on_relay_query(crate::transport::relay_discovery::list_live_machines(
            &config,
        ))
        .unwrap_or_default();
        let machine = resolve_machine_id_for_registry(&request.machine_ref, &live)?;
        let saved = DaemonConfig::forget_remote_machine(machine.clone())?;
        self.provider_catalog_cache.clear();
        let machine = forgotten_machine_record(machine, saved.alias, live, &config.host_machine_id);
        Ok(LocalDaemonResponse::RemoteMachineForgotten { machine })
    }

    pub(super) fn rename_remote_machine_response(
        &mut self,
        request: RenameRemoteMachineRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let config = self.config().clone();
        let live = block_on_relay_query(crate::transport::relay_discovery::list_live_machines(
            &config,
        ))
        .unwrap_or_default();
        let machine = resolve_machine_id_for_registry(&request.machine_ref, &live)?;
        DaemonConfig::rename_remote_machine(machine.clone(), request.alias)?;
        self.provider_catalog_cache.clear();
        let machine = record_for_machine_id(machine, live, &config.host_machine_id)?;
        Ok(LocalDaemonResponse::RemoteMachineRenamed { machine })
    }
}

fn remote_native_provider_run_response(
    app: &mut DaemonApp,
    request: &LaunchProviderRunRequest,
) -> Result<Option<LocalDaemonResponse>, DaemonError> {
    let session = app.sessions().get_session(&request.session_id)?;
    let agent_id = request
        .agent_id
        .clone()
        .or_else(|| session.focused_agent_id().map(str::to_string));
    let Some(agent_id) = agent_id else {
        return Ok(None);
    };
    let agent = app.agents().get_agent(&agent_id)?;
    let Some(remote_execution) = agent.remote_execution().cloned() else {
        return Ok(None);
    };
    let required_mcps = app.required_remote_mcps_for_native_provider_launch(&agent)?;
    let required_skills = app.required_remote_skills_for_native_provider_launch(&agent)?;
    let remote_extension_manifest = app
        .remote_extension_manifest_for_agent(&agent)?
        .without_mcp_tools();
    let relay_config = app.relay_config_for_remote_execution(&remote_execution);
    let response = app.block_on_relay_future(send_peer_request_via_temporary_connection(
        &relay_config,
        ClientTarget {
            daemon_id: Some(remote_execution.worker_kernel_id.clone()),
            daemon_alias: None,
        },
        RelayPeerRequest::LaunchLeasedNativeProviderRun {
            leased_agent_id: remote_execution.leased_agent_id.clone(),
            adapter_key: crate::provider::adapter_key_for_provider(&request.adapter_key)
                .to_string(),
            provider: request.provider.clone(),
            account_profile: request.account_profile.clone(),
            model: request.model.clone(),
            variant: request.variant.clone(),
            structured_endpoint: request.structured_endpoint.clone(),
            provider_session_id: request.provider_session_id.clone(),
            required_mcps,
            required_skills: Some(required_skills),
            remote_extension_manifest,
            provider_launch_credential: None,
        },
    ))?;
    match response {
        RelayPeerResponse::LeasedNativeProviderRunLaunched { provider_run } => {
            let agent_id = request
                .agent_id
                .clone()
                .or_else(|| {
                    app.sessions()
                        .get_session(&request.session_id)
                        .ok()
                        .and_then(|session| session.focused_agent_id().map(str::to_string))
                })
                .or_else(|| {
                    app.agents()
                        .get_focused_agent(&request.session_id)
                        .map(|agent| agent.id().to_string())
                })
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "launch remote native provider run",
                    message: format!(
                        "no focused agent available for remote native provider run in session `{}`",
                        request.session_id
                    ),
                })?;
            let home_agent_id = agent_id.clone();
            let (worker_provider_run_id, provider_run) = provider_run
                .project_leased_for_home_agent(
                    &remote_execution.leased_agent_id,
                    request.session_id.clone(),
                    agent_id,
                );
            let _ = app
                .agents()
                .set_remote_execution_active_worker_provider_run_id(
                    &home_agent_id,
                    Some(worker_provider_run_id),
                )?;
            app.update_provider_run_projection(provider_run.clone());
            app.sessions_mut().set_active_provider_run(
                provider_run.session_id(),
                Some(provider_run.id().to_string()),
            )?;
            Ok(Some(LocalDaemonResponse::ProviderRunLaunched {
                provider_run,
            }))
        }
        other => Err(DaemonError::LocalTransport {
            operation: "launch remote native provider run",
            message: format!("unexpected remote native provider launch response: {other:?}"),
        }),
    }
}

pub(crate) fn launch_provider_request_from_local(
    app: &DaemonApp,
    request: LaunchProviderRunRequest,
) -> LaunchProviderRequest {
    let adapter_key = crate::provider::adapter_key_for_provider(&request.adapter_key).to_string();
    let mut launch_request = LaunchProviderRequest::new(
        request.session_id.clone(),
        adapter_key,
        request.provider,
        request.account_profile,
        request.model,
    )
    .with_variant(request.variant);
    if let Some(endpoint) = request.structured_endpoint {
        launch_request = launch_request.with_structured_endpoint(endpoint);
    }
    if request.native_tui {
        launch_request = launch_request.with_client_interface(ProviderClientInterface::NativeTui);
    }
    if let Some(provider_session_id) = request.provider_session_id {
        let resume_state = crate::provider::ProviderResumeState::from_external_provider_session(
            &launch_request.adapter_key,
            provider_session_id,
        );
        if !resume_state.is_empty() {
            launch_request = launch_request.with_resume_state(resume_state);
        }
    }
    let session = app.sessions().get_session(&request.session_id).ok();
    let focused_agent_id = session
        .as_ref()
        .and_then(|session| session.focused_agent_id().map(str::to_string));
    if let Some(agent_id) = request.agent_id.clone().or(focused_agent_id) {
        launch_request = if let Ok(agent) = app.agents().get_agent(&agent_id) {
            let effective_config = session.as_ref().map(|session| {
                crate::session::effective_agent_execution_config(session, Some(&agent))
            });
            let launch_request = launch_request
                .with_agent_id(agent_id)
                .with_owner_user_id(agent.owner_user_id().to_string());
            if let Some(effective_config) = effective_config {
                launch_request
                    .with_execution_mode(effective_config.mode)
                    .with_permission_level(effective_config.permission_level)
            } else {
                launch_request
            }
        } else {
            launch_request.with_agent_id(agent_id)
        };
    } else {
        if let Some(session) = session.as_ref() {
            let effective_config = crate::session::effective_agent_execution_config(session, None);
            launch_request = launch_request
                .with_execution_mode(effective_config.mode)
                .with_permission_level(effective_config.permission_level);
        }
    }
    launch_request
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::KernelSessionService;
    use crate::provider::{AgentExecutionMode, AgentPermissionLevel};
    use crate::session::{CreateSessionRequest, SessionAgentDefaults};

    #[test]
    fn launch_provider_request_inherits_session_agent_defaults() {
        let mut app =
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon app should boot");
        let defaults = SessionAgentDefaults::new("dev-stub")
            .with_model("model-a")
            .with_effort("low")
            .with_execution_mode(AgentExecutionMode::Plan)
            .with_permission_level(AgentPermissionLevel::Required);
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(
                CreateSessionRequest::new("workspace", "worktree").with_agent_defaults(defaults),
            )
            .expect("session should be created");

        let request = launch_provider_request_from_local(
            &app,
            LaunchProviderRunRequest {
                session_id: session.id().to_string(),
                agent_id: Some(agent.id().to_string()),
                adapter_key: "dev-stub".to_string(),
                provider: "dev-stub".to_string(),
                account_profile: "default".to_string(),
                model: "model-a".to_string(),
                variant: Some("low".to_string()),
                structured_endpoint: None,
                provider_session_id: None,
                native_tui: false,
            },
        );

        assert_eq!(request.execution_mode, Some(AgentExecutionMode::Plan));
        assert_eq!(
            request.permission_level,
            Some(AgentPermissionLevel::Required)
        );
    }
}
