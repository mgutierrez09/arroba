use serde::{Deserialize, Deserializer, Serialize};

use crate::error::DaemonError;

use super::{validate_config_key_path, validate_non_empty};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserCredentialConfig {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub source: UserCredentialSourceConfig,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_hosts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_uses: Vec<UserCredentialUse>,
    pub injection: UserCredentialInjectionConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<UserCredentialMetadataConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserCredentialMetadataConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UserCredentialSourceConfig {
    Env { name: String },
    File { path: String },
    Vault { key: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserCredentialVaultConfig {
    #[serde(default = "default_credential_vault_backend")]
    pub backend: CredentialVaultBackend,
    #[serde(default = "default_credential_vault_service")]
    pub service: String,
    #[serde(default = "default_credential_vault_path")]
    pub path: String,
    #[serde(default)]
    pub unlock_policy: CredentialVaultUnlockPolicy,
    #[serde(default = "default_credential_vault_default_ttl_minutes")]
    pub default_ttl_minutes: u64,
    #[serde(default = "default_credential_vault_max_ttl_minutes")]
    pub max_ttl_minutes: u64,
    #[serde(default)]
    pub agent_management: CredentialVaultAgentManagementPolicy,
}

impl Default for UserCredentialVaultConfig {
    fn default() -> Self {
        Self {
            backend: default_credential_vault_backend(),
            service: default_credential_vault_service(),
            path: default_credential_vault_path(),
            unlock_policy: CredentialVaultUnlockPolicy::default(),
            default_ttl_minutes: default_credential_vault_default_ttl_minutes(),
            max_ttl_minutes: default_credential_vault_max_ttl_minutes(),
            agent_management: CredentialVaultAgentManagementPolicy::default(),
        }
    }
}

impl UserCredentialVaultConfig {
    pub fn service_name(&self) -> &str {
        self.service.trim()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialVaultBackend {
    #[serde(alias = "arroba_encrypted")]
    CharioxEncrypted,
    ProcessMemory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialVaultUnlockPolicy {
    KernelInit,
    Ttl,
    Always,
}

impl Default for CredentialVaultUnlockPolicy {
    fn default() -> Self {
        Self::Ttl
    }
}

impl<'de> Deserialize<'de> for CredentialVaultUnlockPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse("credential_vault.unlock_policy", &value).map_err(|_| {
            serde::de::Error::custom(
                "unsupported credential vault unlock policy; use kernel_init, ttl, or always",
            )
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialVaultAgentManagementPolicy {
    Allow,
    Deny,
}

impl Default for CredentialVaultAgentManagementPolicy {
    fn default() -> Self {
        Self::Allow
    }
}

impl CredentialVaultAgentManagementPolicy {
    pub(crate) fn parse(field: &'static str, value: &str) -> Result<Self, DaemonError> {
        match value.trim() {
            "allow" => Ok(Self::Allow),
            "deny" => Ok(Self::Deny),
            _ => Err(DaemonError::InvalidConfig {
                field,
                message: "unsupported credential vault agent management policy",
            }),
        }
    }
}

impl CredentialVaultBackend {
    pub(crate) fn parse(field: &'static str, value: &str) -> Result<Self, DaemonError> {
        match value.trim() {
            "chariox_encrypted" => Ok(Self::CharioxEncrypted),
            "process_memory" => Ok(Self::ProcessMemory),
            _ => Err(DaemonError::InvalidConfig {
                field,
                message: "unsupported credential vault backend",
            }),
        }
    }
}

impl CredentialVaultUnlockPolicy {
    pub(crate) fn parse(field: &'static str, value: &str) -> Result<Self, DaemonError> {
        match value.trim() {
            "kernel_init" => Ok(Self::KernelInit),
            "ttl" => Ok(Self::Ttl),
            "always" => Ok(Self::Always),
            _ => Err(DaemonError::InvalidConfig {
                field,
                message:
                    "unsupported credential vault unlock policy; use kernel_init, ttl, or always",
            }),
        }
    }
}

fn default_credential_vault_backend() -> CredentialVaultBackend {
    CredentialVaultBackend::CharioxEncrypted
}

fn default_credential_vault_service() -> String {
    "chariox".to_string()
}

fn default_credential_vault_path() -> String {
    "~/.chariox/vault/vault.json".to_string()
}

fn default_credential_vault_default_ttl_minutes() -> u64 {
    30
}

fn default_credential_vault_max_ttl_minutes() -> u64 {
    240
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserCredentialUse {
    Http,
    Pty,
    Connector,
    Browser,
    Computer,
    Mcp,
    Provider,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UserCredentialInjectionConfig {
    Header {
        name: String,
        value: String,
    },
    Query {
        name: String,
    },
    Basic {
        #[serde(default)]
        username: String,
    },
    Hmac {
        #[serde(default = "default_hmac_timestamp_header")]
        timestamp_header: String,
        #[serde(default = "default_hmac_signature_header")]
        signature_header: String,
    },
    Pty,
    Browser,
    Computer,
    Provider,
}

fn default_hmac_timestamp_header() -> String {
    "x-chariox-timestamp".to_string()
}

fn default_hmac_signature_header() -> String {
    "x-chariox-signature".to_string()
}

pub fn validate_credentials(credentials: &[UserCredentialConfig]) -> Result<(), DaemonError> {
    let mut seen = std::collections::BTreeSet::new();
    for credential in credentials {
        validate_config_key_path(&credential.id)?;
        if !seen.insert(credential.id.as_str()) {
            return Err(DaemonError::InvalidConfig {
                field: "credentials",
                message: "credential ids must be unique",
            });
        }
        match &credential.source {
            UserCredentialSourceConfig::Env { name } => {
                validate_non_empty("credentials.source.name", name)?;
            }
            UserCredentialSourceConfig::File { path } => {
                validate_non_empty("credentials.source.path", path)?;
            }
            UserCredentialSourceConfig::Vault { key } => {
                validate_non_empty("credentials.source.key", key)?;
            }
        }
        for host in &credential.allowed_hosts {
            validate_non_empty("credentials.allowed_hosts", host)?;
        }
        match &credential.injection {
            UserCredentialInjectionConfig::Header { name, value } => {
                validate_non_empty("credentials.injection.name", name)?;
                validate_non_empty("credentials.injection.value", value)?;
            }
            UserCredentialInjectionConfig::Query { name } => {
                validate_non_empty("credentials.injection.name", name)?;
            }
            UserCredentialInjectionConfig::Basic { .. } => {}
            UserCredentialInjectionConfig::Hmac {
                timestamp_header,
                signature_header,
            } => {
                validate_non_empty("credentials.injection.timestamp_header", timestamp_header)?;
                validate_non_empty("credentials.injection.signature_header", signature_header)?;
            }
            UserCredentialInjectionConfig::Pty => {}
            UserCredentialInjectionConfig::Browser => {}
            UserCredentialInjectionConfig::Computer => {}
            UserCredentialInjectionConfig::Provider => {}
        }
    }
    Ok(())
}
