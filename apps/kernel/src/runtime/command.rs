use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::local::LocalDaemonRequest;
use crate::session::unix_epoch_ms;

mod caller;
mod local_request_metadata;

pub(crate) use caller::command_caller_user_id;
pub use caller::{KernelCaller, KernelCallerKind, KernelCommandSource};

use local_request_metadata::local_request_metadata;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelCommandPriority {
    Interactive,
    Normal,
    Background,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KernelCommand {
    pub command_id: String,
    pub command_type: String,
    pub submitted_at_ms: u64,
    pub source: KernelCommandSource,
    #[serde(default)]
    pub caller: KernelCaller,
    pub session_id: Option<String>,
    pub attachment_id: Option<String>,
    pub agent_id: Option<String>,
    pub provider_run_id: Option<String>,
    pub workflow_run_id: Option<String>,
    pub node_run_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub causation_id: Option<String>,
    pub correlation_id: String,
    pub priority: KernelCommandPriority,
    pub payload: Value,
}

impl KernelCommand {
    pub(crate) fn durable_operation_id(&self, suffix: Option<&str>) -> String {
        match suffix {
            Some(suffix) => format!("{}:{suffix}", self.command_id),
            None => self.command_id.clone(),
        }
    }

    pub(crate) fn durable_request_fingerprint(&self) -> String {
        let payload = serde_json::to_vec(&serde_json::json!({
            "command_type": self.command_type,
            "source": self.source,
            "session_id": self.session_id,
            "attachment_id": self.attachment_id,
            "payload": self.payload,
        }))
        .unwrap_or_default();
        format!("sha256:{:x}", Sha256::digest(payload))
    }

    pub fn from_local_request(
        command_id: impl Into<String>,
        correlation_id: Option<String>,
        causation_id: Option<String>,
        request: &LocalDaemonRequest,
    ) -> Self {
        Self::from_local_request_with_source(
            command_id,
            KernelCommandSource::LocalCli,
            correlation_id,
            causation_id,
            request,
        )
    }

    pub fn from_local_request_with_source(
        command_id: impl Into<String>,
        source: KernelCommandSource,
        correlation_id: Option<String>,
        causation_id: Option<String>,
        request: &LocalDaemonRequest,
    ) -> Self {
        let caller = KernelCaller::for_source(&source);
        Self::from_local_request_with_caller(
            command_id,
            source,
            caller,
            correlation_id,
            causation_id,
            request,
        )
    }

    pub fn from_local_request_with_caller(
        command_id: impl Into<String>,
        source: KernelCommandSource,
        caller: KernelCaller,
        correlation_id: Option<String>,
        causation_id: Option<String>,
        request: &LocalDaemonRequest,
    ) -> Self {
        let command_id = command_id.into();
        let payload = local_request_payload(request);
        let metadata = local_request_metadata(request);
        Self {
            command_id: command_id.clone(),
            command_type: metadata.command_type.to_string(),
            submitted_at_ms: unix_epoch_ms(),
            source,
            caller,
            session_id: metadata.session_id,
            attachment_id: metadata.attachment_id,
            agent_id: metadata.agent_id,
            provider_run_id: metadata.provider_run_id,
            workflow_run_id: metadata.workflow_run_id,
            node_run_id: metadata.node_run_id,
            idempotency_key: None,
            causation_id,
            correlation_id: correlation_id.unwrap_or(command_id),
            priority: metadata.priority,
            payload,
        }
    }
}

fn local_request_payload(request: &LocalDaemonRequest) -> Value {
    match request {
        LocalDaemonRequest::SetCredentialSecret(request) => serde_json::json!({
            "SetCredentialSecret": {
                "session_id": request.session_id,
                "agent_id": request.agent_id,
                "key": request.key,
                "value": "[redacted]"
            }
        }),
        LocalDaemonRequest::SetProviderAccountCredential(request) => serde_json::json!({
            "SetProviderAccountCredential": {
                "session_id": request.session_id,
                "agent_id": request.agent_id,
                "provider": request.provider,
                "account_profile": request.account_profile,
                "value": "[redacted]",
                "overwrite": request.overwrite
            }
        }),
        LocalDaemonRequest::RespondToInteraction(request) => serde_json::json!({
            "RespondToInteraction": {
                "session_id": request.session_id,
                "interaction_id": request.interaction_id,
                "choice_id": request.choice_id,
                "custom_reply": request.custom_reply.as_ref().map(|_| "[redacted]")
            }
        }),
        LocalDaemonRequest::RequestCredentialEnrollmentInteraction(request) => {
            serde_json::json!({
                "RequestCredentialEnrollmentInteraction": {
                    "session_id": request.session_id,
                    "agent_id": request.agent_id,
                    "enrollment_id": request.enrollment_id,
                    "profile_id": request.profile_id,
                    "target_version": request.target_version,
                    "provider_authorization_url": "[redacted]",
                    "timeout_sec": request.timeout_sec
                }
            })
        }
        _ => serde_json::to_value(request).unwrap_or(Value::Null),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::attachment::ClientCapabilityLevel;
    use crate::local::{
        AliasSessionRequest, AttachToSessionRequest, DestroyAgentRequest, EndSessionRequest,
        FocusAgentRequest, GetDaemonHealthRequest, LocalDaemonRequest, PollRuntimeNoticesRequest,
        RequestCredentialEnrollmentInteractionRequest, RespondToInteractionRequest,
        SetCredentialSecretRequest, SetProviderAccountCredentialRequest, SpawnAgentRequest,
        SubmitPromptRequest, UpdateSessionConfigRequest,
    };
    use crate::runtime::command::{
        KernelCaller, KernelCallerKind, KernelCommand, KernelCommandPriority, KernelCommandSource,
    };
    use crate::session::CreateSessionRequest;
    use chariox_relay::auth::RelaySubjectKind;
    use chariox_relay::protocol::RelayCallerIdentity;

    #[test]
    fn normalizes_prompt_submit_to_interactive_kernel_command() {
        let command = KernelCommand::from_local_request(
            "cmd-1",
            Some("corr-1".to_string()),
            None,
            &LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: "session-1".to_string(),
                attachment_id: "attachment-1".to_string(),
                target_agent_id: Some("agent-1".to_string()),
                prompt: "hello".to_string(),
                attachments: Vec::new(),
            }),
        );

        assert_eq!(command.command_id, "cmd-1");
        assert_eq!(command.command_type, "prompt.submit");
        assert_eq!(command.correlation_id, "corr-1");
        assert_eq!(command.priority, KernelCommandPriority::Interactive);
        assert_eq!(command.session_id.as_deref(), Some("session-1"));
        assert_eq!(command.attachment_id.as_deref(), Some("attachment-1"));
        assert_eq!(command.agent_id.as_deref(), Some("agent-1"));
    }

    #[test]
    fn durable_prompt_fingerprint_is_stable_and_payload_sensitive() {
        let request = |prompt: &str| {
            LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: "session-1".to_string(),
                attachment_id: "attachment-1".to_string(),
                target_agent_id: Some("agent-1".to_string()),
                prompt: prompt.to_string(),
                attachments: Vec::new(),
            })
        };
        let first = KernelCommand::from_local_request("cmd-1", None, None, &request("hello"));
        let retry = KernelCommand::from_local_request("cmd-1", None, None, &request("hello"));
        let conflict = KernelCommand::from_local_request("cmd-1", None, None, &request("changed"));

        assert_eq!(
            first.durable_request_fingerprint(),
            retry.durable_request_fingerprint()
        );
        assert_ne!(
            first.durable_request_fingerprint(),
            conflict.durable_request_fingerprint()
        );
        assert!(first.durable_request_fingerprint().starts_with("sha256:"));
        assert_eq!(first.durable_operation_id(Some("2")), "cmd-1:2");
    }

    #[test]
    fn normalizes_attach_and_focus_as_interactive_commands() {
        let create = KernelCommand::from_local_request(
            "create-1",
            None,
            None,
            &LocalDaemonRequest::CreateSession(CreateSessionRequest::new("workspace", "worktree")),
        );
        let attach = KernelCommand::from_local_request(
            "attach-1",
            None,
            None,
            &LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
                session_id: "session-1".to_string(),
                client_id: "cli-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            }),
        );
        let focus = KernelCommand::from_local_request(
            "focus-1",
            None,
            None,
            &LocalDaemonRequest::FocusAgent(FocusAgentRequest {
                session_id: "session-1".to_string(),
                agent_id: "agent-2".to_string(),
            }),
        );

        assert_eq!(create.command_type, "session.create");
        assert_eq!(create.priority, KernelCommandPriority::Interactive);
        assert_eq!(create.session_id.as_deref(), None);
        assert_eq!(attach.command_type, "session.attach");
        assert_eq!(attach.priority, KernelCommandPriority::Interactive);
        assert_eq!(attach.correlation_id, "attach-1");
        assert_eq!(focus.command_type, "agent.focus");
        assert_eq!(focus.priority, KernelCommandPriority::Interactive);
        assert_eq!(focus.agent_id.as_deref(), Some("agent-2"));
    }

    #[test]
    fn normalizes_end_session_as_interactive_command() {
        let command = KernelCommand::from_local_request(
            "end-1",
            None,
            None,
            &LocalDaemonRequest::EndSession(EndSessionRequest {
                session_id: "session-1".to_string(),
            }),
        );

        assert_eq!(command.command_type, "session.end");
        assert_eq!(command.priority, KernelCommandPriority::Interactive);
        assert_eq!(command.session_id.as_deref(), Some("session-1"));
    }

    #[test]
    fn normalizes_session_runtime_commands_as_interactive_commands() {
        let notice = KernelCommand::from_local_request(
            "notice-1",
            None,
            None,
            &LocalDaemonRequest::PollRuntimeNotices(PollRuntimeNoticesRequest {
                session_id: "session-1".to_string(),
                attachment_id: "attachment-1".to_string(),
            }),
        );
        let config = KernelCommand::from_local_request(
            "config-1",
            None,
            None,
            &LocalDaemonRequest::UpdateSessionConfig(UpdateSessionConfigRequest {
                session_id: "session-1".to_string(),
                attachment_id: "attachment-1".to_string(),
                values: BTreeMap::from([("theme".to_string(), "compact".to_string())]),
                requires_idle: false,
            }),
        );
        let alias = KernelCommand::from_local_request(
            "alias-1",
            None,
            None,
            &LocalDaemonRequest::AliasSession(AliasSessionRequest {
                session_id: "session-1".to_string(),
                alias: "review".to_string(),
            }),
        );
        let spawn = KernelCommand::from_local_request(
            "spawn-1",
            None,
            None,
            &LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                account_profile: None,
                session_id: "session-1".to_string(),
                alias: Some("reviewer".to_string()),
                provider: Some("claude-code".to_string()),
                model: None,
                effort: None,
                execution_mode: None,
                permission_level: None,
                worktree_id: None,
                kernel_ref: None,
                slice_ref: None,
                worktree_placement: None,
                metaagent: false,
            }),
        );
        let destroy = KernelCommand::from_local_request(
            "destroy-1",
            None,
            None,
            &LocalDaemonRequest::DestroyAgent(DestroyAgentRequest {
                session_id: "session-1".to_string(),
                agent_id: "agent-2".to_string(),
            }),
        );

        assert_eq!(notice.command_type, "runtime_notice.poll");
        assert_eq!(notice.priority, KernelCommandPriority::Interactive);
        assert_eq!(notice.session_id.as_deref(), Some("session-1"));
        assert_eq!(notice.attachment_id.as_deref(), Some("attachment-1"));
        assert_eq!(config.command_type, "session.config.update");
        assert_eq!(config.priority, KernelCommandPriority::Interactive);
        assert_eq!(config.session_id.as_deref(), Some("session-1"));
        assert_eq!(config.attachment_id.as_deref(), Some("attachment-1"));
        assert_eq!(alias.command_type, "session.alias");
        assert_eq!(alias.priority, KernelCommandPriority::Interactive);
        assert_eq!(alias.session_id.as_deref(), Some("session-1"));
        assert_eq!(spawn.command_type, "agent.spawn");
        assert_eq!(spawn.priority, KernelCommandPriority::Interactive);
        assert_eq!(spawn.session_id.as_deref(), Some("session-1"));
        assert_eq!(destroy.command_type, "agent.destroy");
        assert_eq!(destroy.priority, KernelCommandPriority::Interactive);
        assert_eq!(destroy.session_id.as_deref(), Some("session-1"));
        assert_eq!(destroy.agent_id.as_deref(), Some("agent-2"));
    }

    #[test]
    fn normalizes_daemon_health_as_normal_command() {
        let command = KernelCommand::from_local_request(
            "health-1",
            None,
            None,
            &LocalDaemonRequest::GetDaemonHealth(GetDaemonHealthRequest),
        );

        assert_eq!(command.command_type, "daemon.health.get");
        assert_eq!(command.priority, KernelCommandPriority::Normal);
    }

    #[test]
    fn redacts_credential_secret_payloads() {
        let command = KernelCommand::from_local_request(
            "credential-1",
            None,
            None,
            &LocalDaemonRequest::SetCredentialSecret(SetCredentialSecretRequest {
                session_id: None,
                agent_id: None,
                key: "github-token".to_string(),
                value: "super-secret".to_string(),
            }),
        );

        assert_eq!(command.command_type, "credential.secret.set");
        assert_eq!(
            command.payload["SetCredentialSecret"]["key"],
            "github-token"
        );
        assert_eq!(
            command.payload["SetCredentialSecret"]["value"],
            "[redacted]"
        );
        assert!(!serde_json::to_string(&command.payload)
            .unwrap()
            .contains("super-secret"));
    }

    #[test]
    fn redacts_provider_account_credential_payloads() {
        let request =
            LocalDaemonRequest::SetProviderAccountCredential(SetProviderAccountCredentialRequest {
                session_id: Some("session-1".to_string()),
                agent_id: Some("agent-1".to_string()),
                provider: "claude".to_string(),
                account_profile: "work".to_string(),
                value: "super-secret-setup-token".to_string(),
                overwrite: true,
            });
        let command = KernelCommand::from_local_request(
            "credential-2",
            None,
            Some("attachment-1".to_string()),
            &request,
        );

        assert_eq!(command.command_type, "provider_account.credential.set");
        assert_eq!(
            command.payload["SetProviderAccountCredential"]["value"],
            "[redacted]"
        );
        assert!(!format!("{request:?}").contains("super-secret-setup-token"));
        assert!(!serde_json::to_string(&command.payload)
            .unwrap()
            .contains("super-secret-setup-token"));
    }

    #[test]
    fn redacts_credential_enrollment_interaction_secrets() {
        let callback = "https://localhost/callback?code=callback-secret";
        let response_request =
            LocalDaemonRequest::RespondToInteraction(RespondToInteractionRequest {
                session_id: "session-1".to_string(),
                interaction_id: "interaction-1".to_string(),
                choice_id: "submit_callback".to_string(),
                custom_reply: Some(callback.to_string()),
            });
        let response_command =
            KernelCommand::from_local_request("credential-response", None, None, &response_request);
        let authorization_url = "https://claude.com/oauth/authorize?state=provider-secret";
        let helper_request = LocalDaemonRequest::RequestCredentialEnrollmentInteraction(
            RequestCredentialEnrollmentInteractionRequest {
                session_id: "session-1".to_string(),
                agent_id: "agent-1".to_string(),
                enrollment_id: "enrollment-1".to_string(),
                profile_id: "profile-1".to_string(),
                target_version: 1,
                provider_authorization_url: authorization_url.to_string(),
                timeout_sec: Some(30),
            },
        );
        let helper_command =
            KernelCommand::from_local_request("credential-helper", None, None, &helper_request);

        let response_payload = serde_json::to_string(&response_command.payload).unwrap();
        let helper_payload = serde_json::to_string(&helper_command.payload).unwrap();
        assert!(!response_payload.contains(callback));
        assert!(!helper_payload.contains(authorization_url));
        assert_eq!(
            response_command.payload["RespondToInteraction"]["custom_reply"],
            "[redacted]"
        );
        assert_eq!(
            helper_command.payload["RequestCredentialEnrollmentInteraction"]
                ["provider_authorization_url"],
            "[redacted]"
        );
    }

    #[test]
    fn can_normalize_local_ipc_commands_with_ipc_source() {
        let request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: "session-1".to_string(),
            attachment_id: "attachment-1".to_string(),
            target_agent_id: Some("agent-1".to_string()),
            prompt: "hello".to_string(),
            attachments: Vec::new(),
        });
        let command = KernelCommand::from_local_request_with_source(
            "ipc-1",
            KernelCommandSource::LocalIpc,
            None,
            None,
            &request,
        );

        assert_eq!(command.source, KernelCommandSource::LocalIpc);
        assert_eq!(command.caller.caller_kind, KernelCallerKind::LocalClient);
        assert_eq!(command.caller.caller_id, "local-ipc");
        assert_eq!(command.command_type, "prompt.submit");
        assert_eq!(command.priority, KernelCommandPriority::Interactive);
        assert_eq!(command.correlation_id, "ipc-1");
    }

    #[test]
    fn relay_identity_becomes_kernel_command_caller() {
        let request = LocalDaemonRequest::GetDaemonHealth(GetDaemonHealthRequest);
        let command = KernelCommand::from_local_request_with_caller(
            "relay-1",
            KernelCommandSource::RelayClient,
            KernelCaller::from_relay_identity(RelayCallerIdentity {
                realm_id: "realm-1".to_string(),
                subject: "client-1".to_string(),
                subject_kind: RelaySubjectKind::Client,
                expires_at_ms: 20,
                token_id: Some("token-1".to_string()),
                user_id: Some("user-1".to_string()),
                public_key_thumbprint: Some("thumbprint-1".to_string()),
            }),
            None,
            None,
            &request,
        );

        assert_eq!(command.source, KernelCommandSource::RelayClient);
        assert_eq!(command.caller.caller_kind, KernelCallerKind::RemoteClient);
        assert_eq!(command.caller.caller_id, "client-1");
        assert_eq!(command.caller.user_id.as_deref(), Some("user-1"));
        assert_eq!(command.caller.client_id.as_deref(), Some("client-1"));
        assert_eq!(command.caller.realm_id.as_deref(), Some("realm-1"));
        assert_eq!(
            command.caller.public_key_thumbprint.as_deref(),
            Some("thumbprint-1")
        );
    }

    #[test]
    fn relay_service_identity_becomes_verified_hosted_service_caller() {
        let request = LocalDaemonRequest::GetDaemonHealth(GetDaemonHealthRequest);
        let command = KernelCommand::from_local_request_with_caller(
            "relay-service-1",
            KernelCommandSource::RelayClient,
            KernelCaller::from_relay_identity(RelayCallerIdentity {
                realm_id: "realm-1".to_string(),
                subject: "deployment-credential-enrollment:enrollment-1".to_string(),
                subject_kind: RelaySubjectKind::Service,
                expires_at_ms: 20,
                token_id: Some("service-token-1".to_string()),
                user_id: Some("user-1".to_string()),
                public_key_thumbprint: Some("service-thumbprint-1".to_string()),
            }),
            None,
            None,
            &request,
        );

        assert_eq!(command.source, KernelCommandSource::RelayClient);
        assert_eq!(command.caller.caller_kind, KernelCallerKind::HostedService);
        assert_eq!(
            command.caller.caller_id,
            "deployment-credential-enrollment:enrollment-1"
        );
        assert_eq!(command.caller.user_id.as_deref(), Some("user-1"));
        assert_eq!(command.caller.realm_id.as_deref(), Some("realm-1"));
        assert!(command.caller.client_id.is_none());
        assert!(command.caller.machine_id.is_none());
    }
}
