use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use base64::Engine;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::config::{
    UserCredentialConfig, UserCredentialInjectionConfig, UserCredentialSourceConfig,
    UserCredentialUse, UserCredentialVaultConfig,
};
use crate::credential::CharioxCredentialRegistry;
use crate::error::DaemonError;

mod vault;
pub(crate) use vault::remove_installed_transferred_vault;
use vault::vault_store_for_config;
pub use vault::{
    chariox_encrypted_vault_status, clear_all_chariox_encrypted_vault_unlocks,
    export_transferred_vault_snapshot, extend_chariox_encrypted_vault,
    install_transferred_vault_snapshot, is_chariox_vault_locked_error,
    lock_chariox_encrypted_vault, restore_transferred_vault_unlock, unlock_chariox_encrypted_vault,
    validate_installed_transferred_vault, validate_transferred_vault_snapshot_for_export,
    CharioxVaultUnlockStatus, CredentialVaultStore, TransferredVaultSnapshot,
    TransferredVaultSourceBinding, VaultUnlockLease,
};

#[cfg(test)]
pub(crate) fn set_chariox_encrypted_vault_secret_for_test(
    path: impl Into<PathBuf>,
    service: &str,
    key: &str,
    value: &str,
) -> Result<(), DaemonError> {
    vault::CharioxEncryptedCredentialVaultStore::new(path.into()).set_secret(service, key, value)
}

#[cfg(test)]
pub(crate) fn get_chariox_encrypted_vault_secret_for_test(
    path: impl Into<PathBuf>,
    service: &str,
    key: &str,
) -> Result<String, DaemonError> {
    vault::CharioxEncryptedCredentialVaultStore::new(path.into()).get_secret(service, key)
}

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialHandleView {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_hosts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_uses: Vec<UserCredentialUse>,
    pub injection_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<CredentialHandleMetadataView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialHandleMetadataView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialHttpRequest {
    pub credential_id: String,
    #[serde(default = "default_http_method")]
    pub method: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_json: Option<serde_json::Value>,
    #[serde(default = "default_http_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_http_max_response_bytes")]
    pub max_response_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialHttpResponse {
    pub status: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_json: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct RuntimeSecretService {
    credentials: Vec<UserCredentialConfig>,
    vault_service: String,
    vault_backend: crate::config::CredentialVaultBackend,
    vault_path: String,
    vault_store: Arc<dyn CredentialVaultStore>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultCredentialUpsertResult {
    pub credential_id: String,
    pub vault_key: String,
    pub stored: bool,
    pub metadata_path: PathBuf,
}

impl RuntimeSecretService {
    pub fn new(credentials: Vec<UserCredentialConfig>) -> Self {
        Self::with_vault_config(credentials, &UserCredentialVaultConfig::default())
            .expect("default credential vault config should be valid")
    }

    pub fn with_vault_service(
        credentials: Vec<UserCredentialConfig>,
        vault_service: impl Into<String>,
    ) -> Self {
        let vault_service = vault_service.into();
        let vault_config = UserCredentialVaultConfig {
            service: vault_service,
            ..UserCredentialVaultConfig::default()
        };
        Self::with_vault_config(credentials, &vault_config)
            .expect("default credential vault config should be valid")
    }

    pub fn with_vault_store(
        credentials: Vec<UserCredentialConfig>,
        vault_service: impl Into<String>,
        vault_store: Arc<dyn CredentialVaultStore>,
    ) -> Self {
        Self {
            credentials,
            vault_service: vault_service.into(),
            vault_backend: crate::config::CredentialVaultBackend::ProcessMemory,
            vault_path: UserCredentialVaultConfig::default().path,
            vault_store,
        }
    }

    pub fn with_vault_config(
        credentials: Vec<UserCredentialConfig>,
        vault_config: &UserCredentialVaultConfig,
    ) -> Result<Self, DaemonError> {
        Ok(Self {
            credentials,
            vault_service: vault_config.service.clone(),
            vault_backend: vault_config.backend,
            vault_path: vault_config.path.clone(),
            vault_store: vault_store_for_config(vault_config)?,
        })
    }

    pub fn credential_env_names(&self) -> BTreeSet<String> {
        self.credentials
            .iter()
            .filter_map(|credential| match &credential.source {
                UserCredentialSourceConfig::Env { name } => Some(name.clone()),
                UserCredentialSourceConfig::File { .. }
                | UserCredentialSourceConfig::Vault { .. } => None,
            })
            .collect()
    }

    pub fn credential_env_names_from(credentials: &[UserCredentialConfig]) -> BTreeSet<String> {
        Self::new(credentials.to_vec()).credential_env_names()
    }

    pub fn list_handles(&self) -> Vec<CredentialHandleView> {
        self.credentials
            .iter()
            .map(|credential| CredentialHandleView {
                id: credential.id.clone(),
                description: credential.description.clone(),
                allowed_hosts: credential.allowed_hosts.clone(),
                allowed_uses: credential.allowed_uses.clone(),
                injection_kind: injection_kind(&credential.injection).to_string(),
                metadata: credential
                    .metadata
                    .as_ref()
                    .map(CredentialHandleMetadataView::from),
            })
            .collect()
    }

    pub fn http_request_with_credential(
        &self,
        request: CredentialHttpRequest,
    ) -> Result<CredentialHttpResponse, DaemonError> {
        let credential = self.credential(&request.credential_id)?;
        self.ensure_use_allowed(credential, UserCredentialUse::Http)?;
        let target = url::Url::parse(&request.url).map_err(|error| {
            secret_error(
                "http_request_with_credential",
                format!("invalid url: {error}"),
            )
        })?;
        self.ensure_host_allowed(credential, &target)?;
        let secret = self.resolve_secret(credential)?;

        let method = request.method.trim().to_ascii_uppercase();
        let mut headers = request.headers;
        let body = request_body(request.body_text, request.body_json)?;
        let mut target = target;

        match &credential.injection {
            UserCredentialInjectionConfig::Header { name, value } => {
                headers.insert(name.clone(), value.replace("${secret}", &secret));
            }
            UserCredentialInjectionConfig::Query { name } => {
                target.query_pairs_mut().append_pair(name, &secret);
            }
            UserCredentialInjectionConfig::Basic { username } => {
                let value = base64::engine::general_purpose::STANDARD
                    .encode(format!("{username}:{secret}"));
                headers.insert("authorization".to_string(), format!("Basic {value}"));
            }
            UserCredentialInjectionConfig::Hmac {
                timestamp_header,
                signature_header,
            } => {
                let timestamp = crate::session::unix_epoch_ms() / 1000;
                let body_hash = sha256_hex(body.as_deref().unwrap_or(""));
                let path_and_query = target[url::Position::BeforePath..].to_string();
                let canonical = format!("{method}\n{path_and_query}\n{body_hash}\n{timestamp}");
                let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).map_err(|error| {
                    secret_error(
                        "http_request_with_credential",
                        format!("failed to initialize hmac: {error}"),
                    )
                })?;
                mac.update(canonical.as_bytes());
                headers.insert(timestamp_header.clone(), timestamp.to_string());
                headers.insert(
                    signature_header.clone(),
                    hex_bytes(&mac.finalize().into_bytes()),
                );
            }
            UserCredentialInjectionConfig::Pty => {
                return Err(secret_error(
                    "http_request_with_credential",
                    format!(
                        "credential `{}` is configured for terminal input",
                        credential.id
                    ),
                ));
            }
            UserCredentialInjectionConfig::Browser => {
                return Err(secret_error(
                    "http_request_with_credential",
                    format!(
                        "credential `{}` is configured for browser input",
                        credential.id
                    ),
                ));
            }
            UserCredentialInjectionConfig::Computer => {
                return Err(secret_error(
                    "http_request_with_credential",
                    format!(
                        "credential `{}` is configured for computer input",
                        credential.id
                    ),
                ));
            }
            UserCredentialInjectionConfig::Provider => {
                return Err(secret_error(
                    "http_request_with_credential",
                    format!(
                        "credential `{}` is configured for provider launch",
                        credential.id
                    ),
                ));
            }
        }

        if request.timeout_ms == 0 {
            return Err(secret_error(
                "http_request_with_credential",
                "timeout_ms must be greater than zero".to_string(),
            ));
        }
        if request.max_response_bytes == 0 {
            return Err(secret_error(
                "http_request_with_credential",
                "max_response_bytes must be greater than zero".to_string(),
            ));
        }
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_millis(request.timeout_ms))
            .build();
        let mut http_request = match method.as_str() {
            "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" => {
                agent.request(&method, target.as_str())
            }
            _ => {
                return Err(secret_error(
                    "http_request_with_credential",
                    format!("unsupported HTTP method `{method}`"),
                ));
            }
        };
        for (name, value) in headers {
            http_request = http_request.set(&name, &value);
        }
        let response = if let Some(body) = body {
            http_request.send_string(&body)
        } else {
            http_request.call()
        }
        .map_err(|error| http_error("http_request_with_credential", error))?;

        decode_http_response(response, request.max_response_bytes)
    }

    pub fn terminal_secret_input(&self, credential_id: &str) -> Result<String, DaemonError> {
        let credential = self.validate_terminal_secret_input(credential_id)?;
        self.resolve_secret(credential)
    }

    pub fn browser_secret_input(&self, credential_id: &str) -> Result<String, DaemonError> {
        let credential = self.credential(credential_id)?;
        self.ensure_use_allowed(credential, UserCredentialUse::Browser)?;
        if !matches!(credential.injection, UserCredentialInjectionConfig::Browser) {
            return Err(secret_error(
                "credential_policy",
                format!("credential `{credential_id}` is not configured for browser input"),
            ));
        }
        self.resolve_secret(credential)
    }

    pub fn computer_secret_input(&self, credential_id: &str) -> Result<String, DaemonError> {
        let credential = self.validate_computer_secret_input(credential_id)?;
        self.resolve_secret(credential)
    }

    pub fn validate_computer_secret_input(
        &self,
        credential_id: &str,
    ) -> Result<&UserCredentialConfig, DaemonError> {
        let credential = self.credential(credential_id)?;
        self.ensure_use_allowed(credential, UserCredentialUse::Computer)?;
        if !matches!(
            credential.injection,
            UserCredentialInjectionConfig::Computer
        ) {
            return Err(secret_error(
                "credential_policy",
                format!("credential `{credential_id}` is not configured for computer input"),
            ));
        }
        Ok(credential)
    }

    pub fn browser_secret_input_for_target_url(
        &self,
        credential_id: &str,
        target_url: &str,
    ) -> Result<String, DaemonError> {
        let credential =
            self.validate_browser_secret_input_for_target_url(credential_id, target_url)?;
        self.resolve_secret(credential)
    }

    pub fn validate_terminal_secret_input(
        &self,
        credential_id: &str,
    ) -> Result<&UserCredentialConfig, DaemonError> {
        let credential = self.credential(credential_id)?;
        self.ensure_use_allowed(credential, UserCredentialUse::Pty)?;
        if !matches!(credential.injection, UserCredentialInjectionConfig::Pty) {
            return Err(secret_error(
                "credential_policy",
                format!("credential `{credential_id}` is not configured for terminal input"),
            ));
        }
        Ok(credential)
    }

    pub fn validate_browser_secret_input_for_target_url(
        &self,
        credential_id: &str,
        target_url: &str,
    ) -> Result<&UserCredentialConfig, DaemonError> {
        let credential = self.credential(credential_id)?;
        self.ensure_use_allowed(credential, UserCredentialUse::Browser)?;
        if !matches!(credential.injection, UserCredentialInjectionConfig::Browser) {
            return Err(secret_error(
                "credential_policy",
                format!("credential `{credential_id}` is not configured for browser input"),
            ));
        }
        let target = url::Url::parse(target_url).map_err(|error| {
            secret_error(
                "browser_secret_input",
                format!("invalid browser target url: {error}"),
            )
        })?;
        self.ensure_host_allowed(credential, &target)?;
        Ok(credential)
    }

    pub fn resolve_connector_secret(
        &self,
        credential_id: &str,
    ) -> Result<(UserCredentialConfig, String), DaemonError> {
        let credential = self.credential(credential_id)?;
        self.ensure_use_allowed(credential, UserCredentialUse::Connector)?;
        let secret = self.resolve_secret(credential)?;
        Ok((credential.clone(), secret))
    }

    pub fn resolve_mcp_secret(&self, credential_id: &str) -> Result<String, DaemonError> {
        let credential = self.credential(credential_id)?;
        self.ensure_use_allowed(credential, UserCredentialUse::Mcp)?;
        self.resolve_secret(credential)
    }

    pub fn provider_secret_input(
        &self,
        credential_id: &str,
    ) -> Result<Zeroizing<String>, DaemonError> {
        let credential = self.credential(credential_id)?;
        self.ensure_use_allowed(credential, UserCredentialUse::Provider)?;
        if !matches!(
            credential.injection,
            UserCredentialInjectionConfig::Provider
        ) {
            return Err(secret_error(
                "credential_policy",
                format!("credential `{credential_id}` is not configured for provider launch"),
            ));
        }
        self.resolve_secret(credential).map(Zeroizing::new)
    }

    pub fn set_vault_secret(&self, key: &str, value: &str) -> Result<(), DaemonError> {
        validate_vault_key(key)?;
        if value.is_empty() {
            return Err(secret_error(
                "credential_vault",
                "credential value must not be empty".to_string(),
            ));
        }
        let service = self.vault_service_name()?;
        let key = key.trim();
        self.vault_store.set_secret(service, key, value)?;
        cache_vault_secret(self.vault_cache_key(key)?, value)?;
        Ok(())
    }

    pub fn delete_vault_secret(&self, key: &str) -> Result<(), DaemonError> {
        validate_vault_key(key)?;
        let service = self.vault_service_name()?;
        let key = key.trim();
        self.vault_store.delete_secret(service, key)?;
        forget_cached_vault_secret(self.vault_cache_key(key)?)?;
        Ok(())
    }

    pub fn upsert_vault_backed_credential_with_secret(
        &self,
        registry: &CharioxCredentialRegistry,
        credential: UserCredentialConfig,
        secret: &str,
        overwrite: bool,
    ) -> Result<VaultCredentialUpsertResult, DaemonError> {
        let vault_key = match &credential.source {
            UserCredentialSourceConfig::Vault { key } => key.trim().to_string(),
            UserCredentialSourceConfig::Env { .. } | UserCredentialSourceConfig::File { .. } => {
                return Err(secret_error(
                    "credential_vault_upsert",
                    "runtime-created credentials must use a vault source".to_string(),
                ));
            }
        };
        validate_vault_key(&vault_key)?;
        if !overwrite && registry.get(&credential.id)?.is_some() {
            return Err(secret_error(
                "credential_vault_upsert",
                format!(
                    "credential `{}` already exists; pass overwrite=true to replace it",
                    credential.id
                ),
            ));
        }
        let service = self.vault_service_name()?;
        let cache_key = self.vault_cache_key(&vault_key)?;
        let previous_credential = registry.get(&credential.id)?;
        let previous_vault_key =
            previous_credential
                .as_ref()
                .and_then(|credential| match &credential.source {
                    UserCredentialSourceConfig::Vault { key } => Some(key.trim().to_string()),
                    UserCredentialSourceConfig::Env { .. }
                    | UserCredentialSourceConfig::File { .. } => None,
                });
        let previous_secret = previous_vault_key
            .as_deref()
            .filter(|previous_key| *previous_key == vault_key)
            .and_then(|previous_key| self.vault_store.get_secret(service, previous_key).ok())
            .map(Zeroizing::new);

        crate::config::validate_credentials(std::slice::from_ref(&credential))?;
        let _ = registry.path_for(&credential.id)?;
        serde_yaml::to_string(&credential).map_err(|error| DaemonError::LocalTransport {
            operation: "credential_vault_upsert",
            message: format!(
                "failed to serialize credential `{}` before vault write: {error}",
                credential.id
            ),
        })?;

        self.set_vault_secret(&vault_key, secret)?;
        match registry.upsert(credential.clone()) {
            Ok((_credential, path)) => Ok(VaultCredentialUpsertResult {
                credential_id: credential.id,
                vault_key,
                stored: true,
                metadata_path: path,
            }),
            Err(error) => {
                if let Some(previous_secret) = previous_secret {
                    let _ =
                        self.vault_store
                            .set_secret(service, &vault_key, previous_secret.as_str());
                    let _ = cache_vault_secret(cache_key, previous_secret.as_str());
                } else if previous_credential.is_none()
                    || previous_vault_key.as_deref() != Some(vault_key.as_str())
                {
                    let _ = self.delete_vault_secret(&vault_key);
                } else {
                    let _ = forget_cached_vault_secret(cache_key);
                }
                Err(error)
            }
        }
    }

    fn credential(&self, id: &str) -> Result<&UserCredentialConfig, DaemonError> {
        self.credentials
            .iter()
            .find(|credential| credential.id == id)
            .ok_or_else(|| secret_error("credential_lookup", format!("unknown credential `{id}`")))
    }

    fn resolve_secret(&self, credential: &UserCredentialConfig) -> Result<String, DaemonError> {
        match &credential.source {
            UserCredentialSourceConfig::Env { name } => std::env::var(name).map_err(|_| {
                secret_error(
                    "credential_resolve",
                    format!("credential `{}` env `{name}` is not set", credential.id),
                )
            }),
            UserCredentialSourceConfig::File { path } => {
                let path = expand_user_path(path);
                fs::read_to_string(&path)
                    .map(|value| value.trim_end().to_string())
                    .map_err(|error| {
                        secret_error(
                            "credential_resolve",
                            format!(
                                "failed to read credential `{}` file `{}`: {error}",
                                credential.id,
                                path.display()
                            ),
                        )
                    })
            }
            UserCredentialSourceConfig::Vault { key } => {
                let service = self.vault_service_name()?;
                let key = key.trim();
                self.ensure_vault_cache_available()?;
                if let Some(secret) = cached_vault_secret(self.vault_cache_key(key)?)? {
                    return Ok(secret);
                }
                let secret = self.vault_store.get_secret(service, key).map_err(|error| {
                    secret_error(
                        "credential_resolve",
                        format!(
                            "failed to resolve credential `{}` from vault key `{}`: {error}",
                            credential.id, key
                        ),
                    )
                })?;
                cache_vault_secret(self.vault_cache_key(key)?, &secret)?;
                Ok(secret)
            }
        }
    }

    fn vault_service_name(&self) -> Result<&str, DaemonError> {
        let service = self.vault_service.trim();
        if service.is_empty() {
            return Err(secret_error(
                "credential_vault",
                "credential vault service must not be empty".to_string(),
            ));
        }
        Ok(service)
    }

    fn ensure_vault_cache_available(&self) -> Result<(), DaemonError> {
        if self.vault_backend != crate::config::CredentialVaultBackend::CharioxEncrypted {
            return Ok(());
        }
        let status = chariox_encrypted_vault_status(&self.vault_path)?;
        if status.unlocked {
            return Ok(());
        }
        clear_vault_secret_process_cache()?;
        Err(DaemonError::LocalTransport {
            operation: "credential_vault_locked",
            message: "Chariox vault is locked".to_string(),
        })
    }

    fn vault_cache_key(&self, key: &str) -> Result<VaultSecretCacheKey, DaemonError> {
        Ok(VaultSecretCacheKey {
            backend: match self.vault_backend {
                crate::config::CredentialVaultBackend::CharioxEncrypted => "chariox_encrypted",
                crate::config::CredentialVaultBackend::ProcessMemory => "process_memory",
            }
            .to_string(),
            path: expand_user_path(&self.vault_path)
                .to_string_lossy()
                .to_string(),
            service: self.vault_service_name()?.to_string(),
            key: key.trim().to_string(),
        })
    }

    fn ensure_use_allowed(
        &self,
        credential: &UserCredentialConfig,
        requested: UserCredentialUse,
    ) -> Result<(), DaemonError> {
        if credential.allowed_uses.is_empty() || credential.allowed_uses.contains(&requested) {
            return Ok(());
        }
        Err(secret_error(
            "credential_policy",
            format!(
                "credential `{}` is not allowed for {:?}",
                credential.id, requested
            ),
        ))
    }

    fn ensure_host_allowed(
        &self,
        credential: &UserCredentialConfig,
        target: &url::Url,
    ) -> Result<(), DaemonError> {
        if credential.allowed_hosts.is_empty() {
            return Ok(());
        }
        let Some(host) = target.host_str() else {
            return Err(secret_error(
                "credential_policy",
                format!("credential `{}` target has no host", credential.id),
            ));
        };
        let host_with_port = match target.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        };
        if credential
            .allowed_hosts
            .iter()
            .any(|allowed| allowed == host || allowed == &host_with_port)
        {
            return Ok(());
        }
        Err(secret_error(
            "credential_policy",
            format!(
                "credential `{}` is not allowed for host `{host_with_port}`",
                credential.id
            ),
        ))
    }
}

impl From<&crate::config::UserCredentialMetadataConfig> for CredentialHandleMetadataView {
    fn from(metadata: &crate::config::UserCredentialMetadataConfig) -> Self {
        Self {
            created_by_kind: metadata.created_by_kind.clone(),
            created_by_id: metadata.created_by_id.clone(),
            session_id: metadata.session_id.clone(),
            provider: metadata.provider.clone(),
            created_at_ms: metadata.created_at_ms,
            updated_at_ms: metadata.updated_at_ms,
        }
    }
}

pub fn secret_like_env_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.ends_with("_TOKEN")
        || upper.ends_with("_SECRET")
        || upper.ends_with("_PASSWORD")
        || upper.ends_with("_PASS")
        || upper.ends_with("_API_KEY")
        || upper.ends_with("_PRIVATE_KEY")
        || upper.ends_with("_ACCESS_KEY")
        || matches!(
            upper.as_str(),
            "TOKEN"
                | "SECRET"
                | "PASSWORD"
                | "API_KEY"
                | "PRIVATE_KEY"
                | "ACCESS_KEY"
                | "GITHUB_TOKEN"
                | "GH_TOKEN"
                | "OPENAI_API_KEY"
                | "ANTHROPIC_API_KEY"
        )
}

fn default_http_method() -> String {
    "GET".to_string()
}

fn default_http_timeout_ms() -> u64 {
    30_000
}

fn default_http_max_response_bytes() -> u64 {
    1_048_576
}

fn request_body(
    body_text: Option<String>,
    body_json: Option<serde_json::Value>,
) -> Result<Option<String>, DaemonError> {
    match (body_text, body_json) {
        (Some(text), None) => Ok(Some(text)),
        (None, Some(json)) => serde_json::to_string(&json).map(Some).map_err(|error| {
            secret_error(
                "http_request_with_credential",
                format!("failed to encode JSON request body: {error}"),
            )
        }),
        (None, None) => Ok(None),
        (Some(_), Some(_)) => Err(secret_error(
            "http_request_with_credential",
            "body_text and body_json are mutually exclusive".to_string(),
        )),
    }
}

fn decode_http_response(
    response: ureq::Response,
    max_response_bytes: u64,
) -> Result<CredentialHttpResponse, DaemonError> {
    let status = response.status();
    let mut body_text = String::new();
    let mut reader = response
        .into_reader()
        .take(max_response_bytes.saturating_add(1));
    reader.read_to_string(&mut body_text).map_err(|error| {
        secret_error(
            "http_request_with_credential",
            format!("failed to read response body: {error}"),
        )
    })?;
    if body_text.len() as u64 > max_response_bytes {
        return Err(secret_error(
            "http_request_with_credential",
            format!("response exceeded max_response_bytes ({max_response_bytes})"),
        ));
    }
    let body_json = serde_json::from_str::<serde_json::Value>(&body_text).ok();
    Ok(CredentialHttpResponse {
        status,
        body_text: body_json.is_none().then_some(body_text),
        body_json,
    })
}

fn http_error(operation: &'static str, error: ureq::Error) -> DaemonError {
    match error {
        ureq::Error::Status(code, response) => {
            let body = response
                .into_string()
                .unwrap_or_else(|error| format!("failed to read error response: {error}"));
            secret_error(operation, format!("HTTP {code}: {body}"))
        }
        ureq::Error::Transport(error) => secret_error(operation, error.to_string()),
    }
}

fn sha256_hex(value: &str) -> String {
    hex_bytes(&Sha256::digest(value.as_bytes()))
}

fn hex_bytes(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

fn injection_kind(injection: &UserCredentialInjectionConfig) -> &'static str {
    match injection {
        UserCredentialInjectionConfig::Header { .. } => "header",
        UserCredentialInjectionConfig::Query { .. } => "query",
        UserCredentialInjectionConfig::Basic { .. } => "basic",
        UserCredentialInjectionConfig::Hmac { .. } => "hmac",
        UserCredentialInjectionConfig::Pty => "pty",
        UserCredentialInjectionConfig::Browser => "browser",
        UserCredentialInjectionConfig::Computer => "computer",
        UserCredentialInjectionConfig::Provider => "provider",
    }
}

fn expand_user_path(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

fn validate_vault_key(key: &str) -> Result<(), DaemonError> {
    if key.trim().is_empty() {
        return Err(secret_error(
            "credential_vault",
            "credential key must not be empty".to_string(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct VaultSecretCacheKey {
    backend: String,
    path: String,
    service: String,
    key: String,
}

fn cached_vault_secret(cache_key: VaultSecretCacheKey) -> Result<Option<String>, DaemonError> {
    Ok(vault_secret_process_cache()
        .lock()
        .map_err(|error| {
            secret_error("credential_vault", format!("vault cache poisoned: {error}"))
        })?
        .get(&cache_key)
        .map(|value| value.to_string()))
}

fn cache_vault_secret(cache_key: VaultSecretCacheKey, value: &str) -> Result<(), DaemonError> {
    vault_secret_process_cache()
        .lock()
        .map_err(|error| {
            secret_error("credential_vault", format!("vault cache poisoned: {error}"))
        })?
        .insert(cache_key, Zeroizing::new(value.to_string()));
    Ok(())
}

fn forget_cached_vault_secret(cache_key: VaultSecretCacheKey) -> Result<(), DaemonError> {
    vault_secret_process_cache()
        .lock()
        .map_err(|error| {
            secret_error("credential_vault", format!("vault cache poisoned: {error}"))
        })?
        .remove(&cache_key);
    Ok(())
}

pub fn clear_vault_secret_process_cache() -> Result<(), DaemonError> {
    vault_secret_process_cache()
        .lock()
        .map_err(|error| {
            secret_error("credential_vault", format!("vault cache poisoned: {error}"))
        })?
        .clear();
    Ok(())
}

fn vault_secret_process_cache() -> &'static Mutex<BTreeMap<VaultSecretCacheKey, Zeroizing<String>>>
{
    static CACHE: OnceLock<Mutex<BTreeMap<VaultSecretCacheKey, Zeroizing<String>>>> =
        OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn secret_error(operation: &'static str, message: String) -> DaemonError {
    DaemonError::LocalTransport { operation, message }
}

#[cfg(test)]
mod tests;
