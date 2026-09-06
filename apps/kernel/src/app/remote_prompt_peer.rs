//! Legacy synchronous remote-prompt transport.
//!
//! New runtime callers use `KernelRuntimeState`, which can surface an async vault
//! unlock interaction. Compatibility callers still owned by `DaemonApp` route
//! through this module so every cold remote Claude launch gets the same
//! profile-bound credential and stale-run retry behavior. Their vault must
//! already be unlocked; they never fall back to provider credential files.

use chariox_relay::protocol::ClientTarget;

use crate::agent::AgentInstance;
use crate::error::DaemonError;
use crate::transport::relay_client::{
    send_peer_request_via_temporary_connection_with_timeout, LEASED_PROMPT_SUBMIT_RESPONSE_TIMEOUT,
};
use crate::transport::relay_peer::{
    RelayPeerRequest, RelayPeerResponse, RemoteCredentialSecretInput,
    RemoteProviderLaunchCredential, REMOTE_PROVIDER_LAUNCH_CREDENTIAL_REQUIRED_CODE,
};

use super::DaemonApp;

impl DaemonApp {
    pub(crate) fn send_remote_prompt_peer_request_with_credential_retry(
        &mut self,
        relay_config: &crate::config::DaemonConfig,
        target: ClientTarget,
        mut request: RelayPeerRequest,
        agent: &AgentInstance,
    ) -> Result<RelayPeerResponse, DaemonError> {
        let mut credential = self.remote_provider_launch_credential(agent, false)?;
        set_remote_prompt_launch_credential(&mut request, credential.clone())?;
        let mut response =
            self.block_on_relay_future(send_peer_request_via_temporary_connection_with_timeout(
                relay_config,
                target.clone(),
                request.clone(),
                LEASED_PROMPT_SUBMIT_RESPONSE_TIMEOUT,
            ));
        if credential.is_none() && response_requires_provider_launch_credential(&response) {
            credential = self.remote_provider_launch_credential(agent, true)?;
            set_remote_prompt_launch_credential(&mut request, credential)?;
            response = self.block_on_relay_future(
                send_peer_request_via_temporary_connection_with_timeout(
                    relay_config,
                    target,
                    request,
                    LEASED_PROMPT_SUBMIT_RESPONSE_TIMEOUT,
                ),
            );
        }
        response
    }

    fn remote_provider_launch_credential(
        &self,
        agent: &AgentInstance,
        force: bool,
    ) -> Result<Option<RemoteProviderLaunchCredential>, DaemonError> {
        if crate::provider::canonical_provider_family(agent.provider()) != Some("claude") {
            return Ok(None);
        }
        if !force
            && agent
                .remote_execution()
                .and_then(|binding| binding.active_worker_provider_run_id.as_deref())
                .is_some()
        {
            return Ok(None);
        }
        let owner_user_id = crate::account_profile::provider_account_authority_owner_user_id(
            &self.config,
            agent.owner_user_id(),
        );
        let profile = self.provider_account_profiles.get(
            &owner_user_id,
            agent.provider(),
            agent.provider_account_profile(),
        )?;
        let mut environment = crate::provider::resolve_provider_account_credentials(
            &self.config,
            &owner_user_id,
            agent.provider(),
            &profile.profile_id,
        )?;
        let token = environment
            .remove(crate::provider::CLAUDE_OAUTH_TOKEN_ENV)
            .filter(|value| !value.trim().is_empty())
            .ok_or(DaemonError::InvalidConfig {
                field: "provider account credential",
                message: "remote Claude launch requires an unlocked Chariox-vault setup token; use `provider setup-token claude <account-profile>` and unlock the vault before retrying",
            })?;
        Ok(Some(RemoteProviderLaunchCredential {
            provider: "claude".to_string(),
            account_profile: profile.profile_id,
            secret_input: RemoteCredentialSecretInput::from_zeroizing(token),
        }))
    }
}

fn set_remote_prompt_launch_credential(
    request: &mut RelayPeerRequest,
    credential: Option<RemoteProviderLaunchCredential>,
) -> Result<(), DaemonError> {
    let RelayPeerRequest::SubmitLeasedPrompt {
        provider_launch_credential,
        ..
    } = request
    else {
        return Err(DaemonError::LocalTransport {
            operation: "send remote prompt with launch credential",
            message: "remote prompt sender received a non-prompt relay request".to_string(),
        });
    };
    *provider_launch_credential = credential;
    Ok(())
}

fn response_requires_provider_launch_credential(
    response: &Result<RelayPeerResponse, DaemonError>,
) -> bool {
    let Err(DaemonError::LocalTransport { message, .. }) = response else {
        return false;
    };
    message.contains(REMOTE_PROVIDER_LAUNCH_CREDENTIAL_REQUIRED_CODE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_credential_retry_requires_the_typed_worker_diagnostic() {
        let required = Err(DaemonError::LocalTransport {
            operation: "read relay peer response",
            message: format!("{REMOTE_PROVIDER_LAUNCH_CREDENTIAL_REQUIRED_CODE}: relaunch"),
        });
        assert!(response_requires_provider_launch_credential(&required));

        let unrelated = Err(DaemonError::LocalTransport {
            operation: "read relay peer response",
            message: "worker unavailable".to_string(),
        });
        assert!(!response_requires_provider_launch_credential(&unrelated));
    }
}
