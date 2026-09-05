use super::*;

impl KernelRuntimeState {
    pub(crate) async fn grant_agent_extension(
        &self,
        agent_ref: &str,
        grant: crate::extension::ExtensionGrant,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        match grant.kind {
            crate::extension::ExtensionKind::Mcp => {
                self.grant_agent_mcp(agent_ref, grant.name, caller_user_id)
                    .await
            }
            crate::extension::ExtensionKind::Skill => {
                self.grant_agent_skill(agent_ref, grant.name, caller_user_id)
                    .await
            }
            crate::extension::ExtensionKind::Script => {
                self.ensure_agent_extension_tool_names_available(
                    agent_ref,
                    &grant,
                    caller_user_id,
                )?;
                let agent =
                    self.owned
                        .grant_agent_extension(agent_ref, grant.clone(), caller_user_id)?;
                self.append_agent_durable_event(
                    "agent.extension_granted",
                    &agent,
                    Some(&format!("script:{}", grant.name)),
                )
                .await?;
                self.append_home_extension_grant_audit_event(
                    "home_extension.grant.created",
                    &agent,
                    caller_user_id,
                    &grant,
                )?;
                self.sync_remote_extension_manifest_for_agent(
                    &agent,
                    Some(caller_user_id),
                    Some(false),
                )
                .await?;
                self.invalidate_workflow_copies_after_source_agent_change(
                    agent.session_id(),
                    agent.id(),
                )?;
                Ok(agent)
            }
            crate::extension::ExtensionKind::Connector => {
                self.ensure_agent_extension_tool_names_available(
                    agent_ref,
                    &grant,
                    caller_user_id,
                )?;
                let agent =
                    self.owned
                        .grant_agent_extension(agent_ref, grant.clone(), caller_user_id)?;
                self.append_agent_durable_event(
                    "agent.extension_granted",
                    &agent,
                    Some(&format!("connector:{}", grant.name)),
                )
                .await?;
                self.append_home_extension_grant_audit_event(
                    "home_extension.grant.created",
                    &agent,
                    caller_user_id,
                    &grant,
                )?;
                self.sync_remote_extension_manifest_for_agent(
                    &agent,
                    Some(caller_user_id),
                    Some(false),
                )
                .await?;
                self.invalidate_workflow_copies_after_source_agent_change(
                    agent.session_id(),
                    agent.id(),
                )?;
                Ok(agent)
            }
        }
    }

    fn ensure_agent_extension_tool_names_available(
        &self,
        agent_ref: &str,
        proposed: &crate::extension::ExtensionGrant,
        caller_user_id: &str,
    ) -> Result<(), DaemonError> {
        let agent = self
            .owned
            .agent_store
            .get_agent(agent_ref)
            .or_else(|_| self.owned.agent_store.get_agent_by_ref(agent_ref))?;
        self.owned.ensure_agent_extension_authority(
            agent.id(),
            caller_user_id,
            "grant agent extension",
        )?;
        let session = self.owned.session_store.get_session(agent.session_id())?;
        let mut reserved = static_runtime_tool_names();
        let script_registry = crate::script::CharioxScriptRegistry::new(
            crate::runtime::capability_registry::script_registry_roots(Some(
                session.workspace_id(),
            ))?,
        );
        let connector_registry = crate::connector::CharioxConnectorRegistry::user()?;
        for grant in agent.extension_grants() {
            if grant.kind == proposed.kind && grant.name == proposed.name {
                continue;
            }
            match grant.kind {
                crate::extension::ExtensionKind::Script => {
                    if let Some(script) = script_registry.get(&grant.name)? {
                        reserved.insert(script.name);
                    }
                }
                crate::extension::ExtensionKind::Connector => {
                    if let Some(connector) = connector_registry.get(&grant.name)? {
                        let max_safety =
                            crate::connector::ConnectorSafety::parse(grant.max_safety.as_deref())?;
                        reserved.extend(connector.allowed_operation_tool_names(max_safety));
                    }
                }
                crate::extension::ExtensionKind::Mcp | crate::extension::ExtensionKind::Skill => {}
            }
        }
        let proposed_names = match proposed.kind {
            crate::extension::ExtensionKind::Script => script_registry
                .get(&proposed.name)?
                .map(|script| vec![script.name])
                .unwrap_or_default(),
            crate::extension::ExtensionKind::Connector => {
                let Some(connector) = connector_registry.get(&proposed.name)? else {
                    return Ok(());
                };
                let max_safety =
                    crate::connector::ConnectorSafety::parse(proposed.max_safety.as_deref())?;
                connector.allowed_operation_tool_names(max_safety)
            }
            crate::extension::ExtensionKind::Mcp | crate::extension::ExtensionKind::Skill => {
                Vec::new()
            }
        };
        for name in proposed_names {
            if reserved.contains(&name) {
                return Err(DaemonError::LocalTransport {
                    operation: "agent.extension.grant",
                    message: format!("extension tool name `{name}` is already in use"),
                });
            }
            reserved.insert(name);
        }
        Ok(())
    }

    pub(crate) async fn revoke_agent_extension(
        &self,
        agent_ref: &str,
        kind: crate::extension::ExtensionKind,
        name: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        match kind {
            crate::extension::ExtensionKind::Mcp => {
                self.revoke_agent_mcp(agent_ref, name, caller_user_id).await
            }
            crate::extension::ExtensionKind::Skill => {
                self.revoke_agent_skill(agent_ref, name, caller_user_id)
                    .await
            }
            crate::extension::ExtensionKind::Script => {
                let agent = self.owned.revoke_agent_extension(
                    agent_ref,
                    crate::extension::ExtensionKind::Script,
                    name,
                    caller_user_id,
                )?;
                self.append_agent_durable_event(
                    "agent.extension_revoked",
                    &agent,
                    Some(&format!("script:{name}")),
                )
                .await?;
                self.append_home_extension_named_grant_audit_event(
                    "home_extension.grant.revoked",
                    &agent,
                    caller_user_id,
                    crate::extension::ExtensionKind::Script,
                    name,
                )?;
                self.sync_remote_extension_manifest_for_agent(
                    &agent,
                    Some(caller_user_id),
                    Some(true),
                )
                .await?;
                self.invalidate_workflow_copies_after_source_agent_change(
                    agent.session_id(),
                    agent.id(),
                )?;
                Ok(agent)
            }
            crate::extension::ExtensionKind::Connector => {
                let agent = self.owned.revoke_agent_extension(
                    agent_ref,
                    crate::extension::ExtensionKind::Connector,
                    name,
                    caller_user_id,
                )?;
                self.append_agent_durable_event(
                    "agent.extension_revoked",
                    &agent,
                    Some(&format!("connector:{name}")),
                )
                .await?;
                self.append_home_extension_named_grant_audit_event(
                    "home_extension.grant.revoked",
                    &agent,
                    caller_user_id,
                    crate::extension::ExtensionKind::Connector,
                    name,
                )?;
                self.sync_remote_extension_manifest_for_agent(
                    &agent,
                    Some(caller_user_id),
                    Some(true),
                )
                .await?;
                self.invalidate_workflow_copies_after_source_agent_change(
                    agent.session_id(),
                    agent.id(),
                )?;
                Ok(agent)
            }
        }
    }

    pub(crate) async fn grant_agent_mcp(
        &self,
        agent_ref: &str,
        name: String,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let existing = self
            .owned
            .agent_store
            .get_agent(agent_ref)
            .or_else(|_| self.owned.agent_store.get_agent_by_ref(agent_ref))?;
        self.owned.ensure_agent_extension_authority(
            existing.id(),
            caller_user_id,
            "grant agent capability",
        )?;
        if existing.remote_execution().is_some() && !existing.mcp_grants().contains(&name) {
            let mut checked = existing.clone();
            checked.grant_mcp(name.clone());
            self.ensure_remote_mcp_availability_for_agent(&checked)
                .await?;
        }
        let agent = self
            .owned
            .grant_agent_mcp(agent_ref, name.clone(), caller_user_id)?;
        self.append_agent_durable_event("agent.mcp_granted", &agent, Some(&name))
            .await?;
        self.append_home_extension_named_grant_audit_event(
            "home_extension.grant.created",
            &agent,
            caller_user_id,
            crate::extension::ExtensionKind::Mcp,
            &name,
        )?;
        self.sync_remote_extension_manifest_for_agent(&agent, Some(caller_user_id), Some(false))
            .await?;
        let _ = self
            .apply_provider_reload_policy(ProviderReloadTrigger::AgentMcpChanged {
                session_id: agent.session_id().to_string(),
                agent_id: agent.id().to_string(),
                name,
            })
            .await?;
        self.invalidate_workflow_copies_after_source_agent_change(agent.session_id(), agent.id())?;
        Ok(agent)
    }

    pub(crate) async fn revoke_agent_mcp(
        &self,
        agent_ref: &str,
        name: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self
            .owned
            .revoke_agent_mcp(agent_ref, name, caller_user_id)?;
        self.append_agent_durable_event("agent.mcp_revoked", &agent, Some(name))
            .await?;
        self.append_home_extension_named_grant_audit_event(
            "home_extension.grant.revoked",
            &agent,
            caller_user_id,
            crate::extension::ExtensionKind::Mcp,
            name,
        )?;
        self.sync_remote_extension_manifest_for_agent(&agent, Some(caller_user_id), Some(true))
            .await?;
        let _ = self
            .apply_provider_reload_policy(ProviderReloadTrigger::AgentMcpChanged {
                session_id: agent.session_id().to_string(),
                agent_id: agent.id().to_string(),
                name: name.to_string(),
            })
            .await?;
        self.invalidate_workflow_copies_after_source_agent_change(agent.session_id(), agent.id())?;
        Ok(agent)
    }

    pub(crate) async fn grant_agent_skill(
        &self,
        agent_ref: &str,
        name: String,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self
            .owned
            .grant_agent_skill(agent_ref, name.clone(), caller_user_id)?;
        self.append_agent_durable_event("agent.skill_granted", &agent, Some(&name))
            .await?;
        self.ensure_remote_skill_packages_for_agent(&agent).await?;
        self.append_home_extension_named_grant_audit_event(
            "home_extension.grant.created",
            &agent,
            caller_user_id,
            crate::extension::ExtensionKind::Skill,
            &name,
        )?;
        self.sync_remote_extension_manifest_for_agent(&agent, Some(caller_user_id), Some(false))
            .await?;
        self.invalidate_workflow_copies_after_source_agent_change(agent.session_id(), agent.id())?;
        Ok(agent)
    }

    pub(crate) async fn revoke_agent_skill(
        &self,
        agent_ref: &str,
        name: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self
            .owned
            .revoke_agent_skill(agent_ref, name, caller_user_id)?;
        self.append_agent_durable_event("agent.skill_revoked", &agent, Some(name))
            .await?;
        self.append_home_extension_named_grant_audit_event(
            "home_extension.grant.revoked",
            &agent,
            caller_user_id,
            crate::extension::ExtensionKind::Skill,
            name,
        )?;
        self.sync_remote_extension_manifest_for_agent(&agent, Some(caller_user_id), Some(false))
            .await?;
        self.invalidate_workflow_copies_after_source_agent_change(agent.session_id(), agent.id())?;
        Ok(agent)
    }

    async fn sync_remote_extension_manifest_for_agent(
        &self,
        agent: &crate::agent::AgentInstance,
        caller_user_id: Option<&str>,
        pending_revoke_intent: Option<bool>,
    ) -> Result<(), DaemonError> {
        self.sync_remote_extension_manifest_for_agent_inner::<true>(
            agent,
            caller_user_id,
            pending_revoke_intent,
        )
        .await
    }

    async fn sync_remote_extension_manifest_for_agent_inner<const SCHEDULE_RETRIES: bool>(
        &self,
        agent: &crate::agent::AgentInstance,
        caller_user_id: Option<&str>,
        pending_revoke_intent: Option<bool>,
    ) -> Result<(), DaemonError> {
        let Some(remote_execution) = agent.remote_execution().cloned() else {
            return Ok(());
        };
        let manifest = self.remote_extension_manifest_for_agent(agent)?;
        let manifest_hash = manifest.manifest_hash();
        let tool_count = manifest.tools.len();
        let pending_revoke = remote_extension_manifest_pending_revoke(
            agent.remote_extension_manifest_sync(),
            pending_revoke_intent,
        );
        let syncing_status = crate::extension::RemoteExtensionManifestSyncStatus::pending(
            manifest_hash.clone(),
            pending_revoke,
        )
        .syncing();
        let _ = self
            .owned
            .agent_store
            .set_remote_extension_manifest_sync(agent.id(), Some(syncing_status.clone()));
        let mut config = self.config_snapshot().await;
        if let (Some(relay_url), Some(relay_token)) = (
            remote_execution.relay_url.clone(),
            remote_execution.relay_token.clone(),
        ) {
            config.apply_missing_remote_relay_override(relay_url, relay_token);
        }
        let response = crate::transport::relay_client::send_peer_request_via_temporary_connection(
            &config,
            ClientTarget {
                daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                daemon_alias: None,
            },
            RelayPeerRequest::UpdateLeasedAgentRemoteExtensionManifest {
                leased_agent_id: remote_execution.leased_agent_id,
                remote_extension_manifest: manifest,
            },
        )
        .await;
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                let error_message = error.to_string();
                let _ = self.owned.agent_store.set_remote_extension_manifest_sync(
                    agent.id(),
                    Some(syncing_status.failed(error_message.clone())),
                );
                let _ = self.append_home_extension_manifest_audit_event(
                    "home_extension.manifest.failed",
                    agent,
                    caller_user_id,
                    &manifest_hash,
                    tool_count,
                    pending_revoke,
                    Some("failed"),
                    Some(&error_message),
                    None,
                    None,
                );
                if SCHEDULE_RETRIES {
                    self.schedule_remote_extension_manifest_retry(
                        agent,
                        caller_user_id,
                        manifest_hash.clone(),
                        tool_count,
                        pending_revoke,
                        error_message.clone(),
                    )
                    .await;
                }
                crate::logging::warn_with_fields(
                    "daemon.remote_extension",
                    "remote extension manifest sync failed; home validation remains authoritative",
                    serde_json::json!({
                        "agent_id": agent.id(),
                        "worker_kernel_id": remote_execution.worker_kernel_id,
                        "error": error_message,
                    }),
                );
                return Ok(());
            }
        };
        if !matches!(
            response,
            RelayPeerResponse::LeasedAgentRemoteExtensionManifestUpdated { .. }
        ) {
            let error = "unexpected worker manifest sync response".to_string();
            let _ = self.owned.agent_store.set_remote_extension_manifest_sync(
                agent.id(),
                Some(syncing_status.failed(error.clone())),
            );
            let _ = self.append_home_extension_manifest_audit_event(
                "home_extension.manifest.failed",
                agent,
                caller_user_id,
                &manifest_hash,
                tool_count,
                pending_revoke,
                Some("failed"),
                Some(&error),
                None,
                None,
            );
            if SCHEDULE_RETRIES {
                self.schedule_remote_extension_manifest_retry(
                    agent,
                    caller_user_id,
                    manifest_hash.clone(),
                    tool_count,
                    pending_revoke,
                    error,
                )
                .await;
            }
            crate::logging::warn_with_fields(
                "daemon.remote_extension",
                "remote extension manifest sync returned an unexpected response",
                serde_json::json!({
                    "agent_id": agent.id(),
                    "worker_kernel_id": remote_execution.worker_kernel_id,
                    "response": format!("{response:?}"),
                }),
            );
        } else {
            self.owned
                .remote_extension_manifest_retry_counts
                .lock()
                .await
                .remove(&remote_extension_manifest_retry_key(
                    agent.id(),
                    &manifest_hash,
                ));
            let _ = self.owned.agent_store.set_remote_extension_manifest_sync(
                agent.id(),
                Some(crate::extension::RemoteExtensionManifestSyncStatus::synced(
                    manifest_hash.clone(),
                )),
            );
            self.append_home_extension_manifest_audit_event(
                "home_extension.manifest.synced",
                agent,
                caller_user_id,
                &manifest_hash,
                tool_count,
                pending_revoke,
                Some("synced"),
                None,
                None,
                None,
            )?;
        }
        Ok(())
    }

    async fn schedule_remote_extension_manifest_retry(
        &self,
        agent: &crate::agent::AgentInstance,
        caller_user_id: Option<&str>,
        manifest_hash: String,
        tool_count: usize,
        pending_revoke: bool,
        error: String,
    ) {
        const RETRY_DELAYS_SECONDS: [u64; 3] = [2, 10, 30];
        let agent_id = agent.id().to_string();
        let retry_key = remote_extension_manifest_retry_key(&agent_id, &manifest_hash);
        let attempt = {
            let mut counts = self
                .owned
                .remote_extension_manifest_retry_counts
                .lock()
                .await;
            let count = counts.entry(retry_key.clone()).or_insert(0);
            if *count >= RETRY_DELAYS_SECONDS.len() as u32 {
                return;
            }
            *count += 1;
            *count
        };
        let delay = RETRY_DELAYS_SECONDS[(attempt - 1) as usize];
        let mut payload = self.home_extension_agent_audit_payload(agent, caller_user_id);
        payload.insert(
            "manifest_hash".to_string(),
            serde_json::json!(manifest_hash),
        );
        payload.insert("tool_count".to_string(), serde_json::json!(tool_count));
        payload.insert(
            "pending_revoke".to_string(),
            serde_json::json!(pending_revoke),
        );
        payload.insert("attempt".to_string(), serde_json::json!(attempt));
        payload.insert("delay_sec".to_string(), serde_json::json!(delay));
        payload.insert("error".to_string(), serde_json::json!(error));
        payload.insert("status".to_string(), serde_json::json!("retry_scheduled"));
        let _ = self.owned.durable_state_store.append_event(
            "home_extension.manifest.retry_scheduled",
            Some(agent_id.clone()),
            serde_json::Value::Object(payload),
        );
        let state = self.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(delay));
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            runtime.block_on(async move {
                let Ok(agent) = state.owned.agent_store.get_agent(&agent_id) else {
                    return;
                };
                let status_matches = agent
                    .remote_extension_manifest_sync()
                    .and_then(|status| status.manifest_hash.as_deref())
                    == Some(manifest_hash.as_str());
                if !status_matches {
                    return;
                }
                let _ = state
                    .sync_remote_extension_manifest_for_agent_inner::<false>(&agent, None, None)
                    .await;
            });
        });
    }

    pub(crate) async fn retry_remote_extension_manifest_sync(
        &self,
        agent_ref: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self
            .owned
            .agent_store
            .get_agent(agent_ref)
            .or_else(|_| self.owned.agent_store.get_agent_by_ref(agent_ref))?;
        self.owned.ensure_agent_extension_authority(
            agent.id(),
            caller_user_id,
            "remote extension manifest sync retry",
        )?;
        self.sync_remote_extension_manifest_for_agent(&agent, Some(caller_user_id), None)
            .await?;
        self.owned.agent_store.get_agent(agent.id())
    }

    fn append_home_extension_grant_audit_event(
        &self,
        kind: &'static str,
        agent: &crate::agent::AgentInstance,
        caller_user_id: &str,
        grant: &crate::extension::ExtensionGrant,
    ) -> Result<(), DaemonError> {
        let mut payload = self.home_extension_agent_audit_payload(agent, Some(caller_user_id));
        payload.insert(
            "grant".to_string(),
            serde_json::json!({
                "kind": grant.kind.as_str(),
                "name": grant.name,
                "environment": grant.environment,
                "credential_present": grant.credential.is_some(),
                "max_safety": grant.max_safety,
            }),
        );
        self.owned.durable_state_store.append_event(
            kind,
            Some(agent.id().to_string()),
            serde_json::Value::Object(payload),
        )?;
        Ok(())
    }

    fn append_home_extension_manifest_audit_event(
        &self,
        kind: &'static str,
        agent: &crate::agent::AgentInstance,
        caller_user_id: Option<&str>,
        manifest_hash: &str,
        tool_count: usize,
        pending_revoke: bool,
        status: Option<&str>,
        error: Option<&str>,
        attempt: Option<u32>,
        delay_sec: Option<u64>,
    ) -> Result<(), DaemonError> {
        let mut payload = self.home_extension_agent_audit_payload(agent, caller_user_id);
        payload.insert(
            "manifest_hash".to_string(),
            serde_json::json!(manifest_hash),
        );
        payload.insert("tool_count".to_string(), serde_json::json!(tool_count));
        payload.insert(
            "pending_revoke".to_string(),
            serde_json::json!(pending_revoke),
        );
        if pending_revoke && kind == "home_extension.manifest.synced" {
            payload.insert("revoke_acknowledged".to_string(), serde_json::json!(true));
        }
        if let Some(status) = status {
            payload.insert("status".to_string(), serde_json::json!(status));
        }
        if let Some(error) = error {
            payload.insert("error".to_string(), serde_json::json!(error));
        }
        if let Some(attempt) = attempt {
            payload.insert("attempt".to_string(), serde_json::json!(attempt));
        }
        if let Some(delay_sec) = delay_sec {
            payload.insert("delay_sec".to_string(), serde_json::json!(delay_sec));
        }
        self.owned.durable_state_store.append_event(
            kind,
            Some(agent.id().to_string()),
            serde_json::Value::Object(payload),
        )?;
        Ok(())
    }

    fn append_home_extension_named_grant_audit_event(
        &self,
        kind: &'static str,
        agent: &crate::agent::AgentInstance,
        caller_user_id: &str,
        extension_kind: crate::extension::ExtensionKind,
        name: &str,
    ) -> Result<(), DaemonError> {
        let mut payload = self.home_extension_agent_audit_payload(agent, Some(caller_user_id));
        payload.insert(
            "grant".to_string(),
            serde_json::json!({
                "kind": extension_kind.as_str(),
                "name": name,
            }),
        );
        self.owned.durable_state_store.append_event(
            kind,
            Some(agent.id().to_string()),
            serde_json::Value::Object(payload),
        )?;
        Ok(())
    }

    pub(in crate::runtime::state) fn home_extension_agent_audit_payload(
        &self,
        agent: &crate::agent::AgentInstance,
        caller_user_id: Option<&str>,
    ) -> serde_json::Map<String, serde_json::Value> {
        let session = self
            .owned
            .session_store
            .get_session(agent.session_id())
            .ok();
        let remote_execution = agent.remote_execution();
        let mut payload = serde_json::Map::new();
        payload.insert(
            "home_session_id".to_string(),
            serde_json::json!(agent.session_id()),
        );
        payload.insert(
            "home_user_id".to_string(),
            serde_json::json!(session.as_ref().map(|session| session.owner_user_id())),
        );
        payload.insert(
            "caller_user_id".to_string(),
            serde_json::json!(caller_user_id),
        );
        payload.insert("agent_id".to_string(), serde_json::json!(agent.id()));
        payload.insert(
            "agent_ref".to_string(),
            serde_json::json!(agent.agent_ref()),
        );
        payload.insert(
            "agent_owner_user_id".to_string(),
            serde_json::json!(agent.owner_user_id()),
        );
        payload.insert(
            "lease_id".to_string(),
            serde_json::json!(remote_execution.map(|remote| remote.execution_lease_id.as_str())),
        );
        payload.insert(
            "leased_agent_id".to_string(),
            serde_json::json!(remote_execution.map(|remote| remote.leased_agent_id.as_str())),
        );
        payload.insert(
            "worker_kernel_id".to_string(),
            serde_json::json!(remote_execution.map(|remote| remote.worker_kernel_id.as_str())),
        );
        payload.insert(
            "worker_machine_id".to_string(),
            serde_json::json!(remote_execution.map(|remote| remote.worker_machine_id.as_str())),
        );
        payload.insert(
            "active_worker_provider_run_id".to_string(),
            serde_json::json!(
                remote_execution.and_then(|remote| remote.active_worker_provider_run_id.as_deref())
            ),
        );
        payload
    }

    pub(crate) fn list_home_extension_audit_events(
        &self,
        agent_ref: &str,
        caller_user_id: &str,
        limit: usize,
    ) -> Result<Vec<crate::durable_state::DurableStateEvent>, DaemonError> {
        let agent = self
            .owned
            .agent_store
            .get_agent(agent_ref)
            .or_else(|_| self.owned.agent_store.get_agent_by_ref(agent_ref))?;
        self.owned.ensure_agent_extension_authority(
            agent.id(),
            caller_user_id,
            "home extension audit",
        )?;
        let events = self
            .owned
            .durable_state_store
            .load_subject_events(agent.id(), limit)?;
        Ok(events
            .into_iter()
            .filter(|event| {
                event.kind.starts_with("home_extension.")
                    || event.kind.starts_with("agent.extension")
                    || event.kind.starts_with("agent.mcp_")
                    || event.kind.starts_with("agent.skill_")
                    || event.kind.starts_with("extension.registration.")
            })
            .collect())
    }

    pub(crate) async fn update_session_config(
        &self,
        session_id: &str,
        attachment_id: &str,
        values: std::collections::BTreeMap<String, String>,
        requires_idle: bool,
    ) -> Result<crate::session::SessionConfigState, DaemonError> {
        self.owned
            .update_session_config(session_id, attachment_id, values, requires_idle)
    }

    pub(crate) async fn update_agent_config(
        &self,
        session_id: &str,
        agent_id: &str,
        caller_user_id: &str,
        execution_mode_override: Option<Option<crate::provider::AgentExecutionMode>>,
        permission_level_override: Option<Option<crate::provider::AgentPermissionLevel>>,
        workspace_id: Option<Option<String>>,
        worktree_id: Option<Option<String>>,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let update = self.owned.update_agent_config(
            session_id,
            agent_id,
            caller_user_id,
            execution_mode_override.clone(),
            permission_level_override.clone(),
            workspace_id,
            worktree_id,
        )?;
        for provider_run_id in update.terminated_run_ids {
            let (_, process_key) = self
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(&provider_run_id)
                })
                .await
                .unwrap_or((false, None));
            self.owned
                .remove_provider_process_tracking_for_run(&provider_run_id, process_key);
        }
        let mut agent = update.agent;
        if let Some(remote_update) = update.remote_update {
            let mut config = self.config_snapshot().await;
            if let (Some(relay_url), Some(relay_token)) = (
                remote_update.relay_url.clone(),
                remote_update.relay_token.clone(),
            ) {
                config.apply_remote_relay_override(relay_url, relay_token);
            }
            let target = ClientTarget {
                daemon_id: Some(remote_update.worker_kernel_id.clone()),
                daemon_alias: None,
            };
            let request = RelayPeerRequest::UpdateLeasedAgentConfig {
                leased_agent_id: remote_update.leased_agent_id,
                execution_mode: remote_update.execution_mode,
                permission_level: remote_update.permission_level,
            };
            let response = match self.connected_relay_state_for_config(&config).await {
                Some(relay_state) => {
                    crate::transport::relay_client::send_peer_request_via_connected_relay(
                        &config,
                        &relay_state,
                        target,
                        request,
                    )
                    .await
                }
                None => {
                    crate::transport::relay_client::send_peer_request_via_temporary_connection(
                        &config, target, request,
                    )
                    .await
                }
            };
            match response {
                Ok(RelayPeerResponse::LeasedAgentConfigUpdated { .. }) => {}
                Ok(other) => {
                    return Err(DaemonError::LocalTransport {
                        operation: "update remote leased agent config",
                        message: format!("unexpected remote config response: {other:?}"),
                    });
                }
                Err(error) => return Err(error),
            }
            agent = self.owned.commit_remote_agent_config_update(
                session_id,
                agent_id,
                execution_mode_override,
                permission_level_override,
            )?;
        }
        self.invalidate_workflow_copies_after_source_agent_change(session_id, agent_id)?;
        Ok(agent)
    }

    pub(crate) async fn update_agent_profile(
        &self,
        session_id: &str,
        agent_id: &str,
        caller_user_id: &str,
        provider: Option<String>,
        account_profile: Option<String>,
        model: Option<String>,
        effort: Option<Option<String>>,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let original =
            self.owned
                .ensure_agent_owner(agent_id, caller_user_id, "update agent profile")?;
        let profile_transition = if original.remote_execution().is_some() {
            Some(
                self.owned
                    .prompt_state_owner
                    .claim_idle_agent_profile_transition(
                        &self.owned.session_store.get_session(session_id)?,
                        agent_id,
                    )?,
            )
        } else {
            None
        };
        let result = async {
            let update = self.owned.update_agent_profile(
                session_id,
                agent_id,
                caller_user_id,
                provider,
                account_profile,
                model,
                effort,
            )?;
            for provider_run_id in update.terminated_run_ids {
                let (_, process_key) = self
                    .with_app_side_effect(|app| {
                        crate::app::ProviderLaunchProcessRuntime::new(app)
                            .remove_run(&provider_run_id)
                    })
                    .await
                    .unwrap_or((false, None));
                self.owned
                    .remove_provider_process_tracking_for_run(&provider_run_id, process_key);
            }
            let mut agent = update.agent;
            if let Some(remote_update) = update.remote_update {
                self.confirm_remote_agent_profile(agent_id, &remote_update)
                    .await?;
                agent = self.owned.commit_remote_agent_profile_update(
                    session_id,
                    agent_id,
                    remote_update.provider,
                    remote_update.account_profile,
                    remote_update.model,
                    remote_update.effort,
                )?;
            }
            self.append_agent_durable_event("agent.updated", &agent, None)
                .await?;
            self.invalidate_workflow_copies_after_source_agent_change(session_id, agent_id)?;
            Ok(agent)
        }
        .await;
        if let Some(claim) = profile_transition {
            let finish = self
                .finish_remote_agent_profile_transition(session_id, agent_id, claim)
                .await;
            if result.is_ok() {
                finish?;
            }
        }
        result
    }

    pub(super) fn invalidate_workflow_copies_after_source_agent_change(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let affected_workflows = self
            .owned
            .session_store
            .write()
            .invalidate_workflow_runtime_instances_for_agent_change(session_id, agent_id)?;
        if affected_workflows.is_empty() {
            return Ok(());
        }
        self.owned
            .persist_workflow_runtime_session(session_id, "workflow_source_agent_changed")?;
        self.owned
            .workflow_cleanup_runtime_instances_exclusive(session_id)
    }

    pub(crate) async fn alias_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        caller_user_id: &str,
        alias: Option<String>,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self
            .owned
            .alias_agent(session_id, agent_id, caller_user_id, alias)?;
        self.invalidate_workflow_copies_after_source_agent_change(session_id, agent_id)?;
        Ok(agent)
    }

    pub(crate) async fn update_agent_substitutes(
        &self,
        session_id: &str,
        agent_id: &str,
        caller_user_id: &str,
        action: crate::local::AgentSubstituteAction,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let original =
            self.owned
                .ensure_agent_owner(agent_id, caller_user_id, "update agent substitutes")?;
        if original.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        if original.remote_execution().is_some() {
            if let Some(target) =
                super::remote_agent_profile_runtime::substitute_target(&original, &action)?
            {
                if !super::remote_agent_profile_runtime::same_execution_profile(&original, &target)
                {
                    return self
                        .update_remote_agent_substitute(original, action, target)
                        .await;
                }
            }
        }
        let profile_transition = if original.remote_execution().is_some() {
            Some(
                self.owned
                    .prompt_state_owner
                    .claim_agent_profile_list_edit(
                        &self.owned.session_store.get_session(session_id)?,
                        agent_id,
                    )?,
            )
        } else {
            None
        };
        let result = async {
            let (agent, retired_run) = self.owned.update_agent_substitutes(
                session_id,
                agent_id,
                caller_user_id,
                action,
            )?;
            if let Some(provider_run_id) = retired_run {
                let (_, process_key) = self
                    .with_app_side_effect(|app| {
                        crate::app::ProviderLaunchProcessRuntime::new(app)
                            .remove_run(&provider_run_id)
                    })
                    .await
                    .unwrap_or((false, None));
                self.owned
                    .remove_provider_process_tracking_for_run(&provider_run_id, process_key);
            }
            self.append_agent_durable_event("agent.updated", &agent, None)
                .await?;
            self.invalidate_workflow_copies_after_source_agent_change(session_id, agent_id)?;
            Ok(agent)
        }
        .await;
        if let Some(claim) = profile_transition {
            let finish = self
                .finish_remote_agent_profile_transition(session_id, agent_id, claim)
                .await;
            if result.is_ok() {
                finish?;
            }
        }
        result
    }

    pub(crate) async fn ensure_agent_owner(
        &self,
        agent_id: &str,
        caller_user_id: &str,
        operation: &'static str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.owned
            .ensure_agent_owner(agent_id, caller_user_id, operation)
    }

    pub(crate) async fn ensure_agent_prompt_access(
        &self,
        agent_id: &str,
        caller_user_id: &str,
        operation: &'static str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.owned
            .ensure_agent_prompt_access(agent_id, caller_user_id, operation)
    }
}

fn remote_extension_manifest_retry_key(agent_id: &str, manifest_hash: &str) -> String {
    format!("{agent_id}:{manifest_hash}")
}

fn remote_extension_manifest_pending_revoke(
    current_status: Option<&crate::extension::RemoteExtensionManifestSyncStatus>,
    intent: Option<bool>,
) -> bool {
    match intent {
        Some(pending_revoke) => pending_revoke,
        None => current_status
            .and_then(|status| status.pending_revoke)
            .unwrap_or(false),
    }
}

fn static_runtime_tool_names() -> std::collections::BTreeSet<String> {
    crate::transport::runtime_tools::workspace_live_sync_runtime_tool_specs()
        .into_iter()
        .chain(crate::transport::runtime_tools::extension_runtime_tool_specs())
        .chain(crate::transport::runtime_tools::credential_runtime_tool_specs())
        .chain(crate::transport::runtime_tools::workflow_runtime_tool_specs())
        .chain(crate::transport::runtime_tools::slice_runtime_tool_specs())
        .map(|spec| spec.name)
        .collect()
}

#[cfg(test)]
mod tests;
