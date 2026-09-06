use super::*;
use crate::config::{
    CredentialVaultBackend, UserCredentialInjectionConfig, UserCredentialMetadataConfig,
    UserCredentialSourceConfig, UserCredentialUse,
};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Debug, Default)]
struct MemoryVaultStore {
    secrets: Mutex<BTreeMap<(String, String), String>>,
}

impl CredentialVaultStore for MemoryVaultStore {
    fn get_secret(&self, service: &str, key: &str) -> Result<String, DaemonError> {
        self.secrets
            .lock()
            .unwrap()
            .get(&(service.to_string(), key.to_string()))
            .cloned()
            .ok_or_else(|| {
                secret_error("credential_vault", format!("credential `{key}` not found"))
            })
    }

    fn set_secret(&self, service: &str, key: &str, value: &str) -> Result<(), DaemonError> {
        self.secrets
            .lock()
            .unwrap()
            .insert((service.to_string(), key.to_string()), value.to_string());
        Ok(())
    }

    fn delete_secret(&self, service: &str, key: &str) -> Result<(), DaemonError> {
        self.secrets
            .lock()
            .unwrap()
            .remove(&(service.to_string(), key.to_string()));
        Ok(())
    }
}

#[derive(Debug, Default)]
struct WriteOnlyVaultStore {
    secrets: Mutex<BTreeMap<(String, String), String>>,
}

impl CredentialVaultStore for WriteOnlyVaultStore {
    fn get_secret(&self, _service: &str, key: &str) -> Result<String, DaemonError> {
        Err(secret_error(
            "credential_vault",
            format!("credential `{key}` backing store read unavailable"),
        ))
    }

    fn set_secret(&self, service: &str, key: &str, value: &str) -> Result<(), DaemonError> {
        self.secrets
            .lock()
            .unwrap()
            .insert((service.to_string(), key.to_string()), value.to_string());
        Ok(())
    }

    fn delete_secret(&self, service: &str, key: &str) -> Result<(), DaemonError> {
        self.secrets
            .lock()
            .unwrap()
            .remove(&(service.to_string(), key.to_string()));
        Ok(())
    }
}

#[test]
fn credential_handles_do_not_include_sources_or_values() {
    let service = RuntimeSecretService::new(vec![UserCredentialConfig {
        id: "github".to_string(),
        description: Some("GitHub API".to_string()),
        source: UserCredentialSourceConfig::Env {
            name: "GH_TOKEN".to_string(),
        },
        allowed_hosts: vec!["api.github.com".to_string()],
        allowed_uses: vec![UserCredentialUse::Http],
        injection: UserCredentialInjectionConfig::Header {
            name: "authorization".to_string(),
            value: "Bearer ${secret}".to_string(),
        },
        metadata: None,
    }]);

    let serialized = serde_json::to_string(&service.list_handles()).unwrap();
    assert!(serialized.contains("github"));
    assert!(!serialized.contains("GH_TOKEN"));
    assert!(!serialized.contains("${secret}"));
}

#[test]
fn credential_handles_do_not_include_internal_vault_metadata() {
    let service = RuntimeSecretService::new(vec![UserCredentialConfig {
        id: "gmail-password".to_string(),
        description: Some("Gmail password".to_string()),
        source: UserCredentialSourceConfig::Vault {
            key: "gmail-password-vault-key".to_string(),
        },
        allowed_hosts: vec!["accounts.google.com".to_string()],
        allowed_uses: vec![UserCredentialUse::Browser],
        injection: UserCredentialInjectionConfig::Browser,
        metadata: Some(UserCredentialMetadataConfig {
            created_by_kind: Some("agent".to_string()),
            created_by_id: Some("agent-1".to_string()),
            session_id: Some("session-1".to_string()),
            provider: Some("codex".to_string()),
            provider_run_id: Some("provider-run-1".to_string()),
            vault_key: Some("gmail-password-vault-key".to_string()),
            created_at_ms: Some(1),
            updated_at_ms: Some(2),
        }),
    }]);

    let serialized = serde_json::to_string(&service.list_handles()).unwrap();

    assert!(serialized.contains("gmail-password"));
    assert!(serialized.contains("agent-1"));
    assert!(!serialized.contains("vault_key"));
    assert!(!serialized.contains("gmail-password-vault-key"));
    assert!(!serialized.contains("provider_run_id"));
    assert!(!serialized.contains("provider-run-1"));
}

#[test]
fn secret_like_env_name_matches_common_tokens() {
    assert!(secret_like_env_name("GITHUB_TOKEN"));
    assert!(secret_like_env_name("OPENAI_API_KEY"));
    assert!(secret_like_env_name("DB_PASSWORD"));
    assert!(!secret_like_env_name("PATH"));
}

#[test]
fn http_request_with_credential_injects_header_without_returning_secret() {
    let _guard = crate::env_lock::lock();
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("request should arrive");
        let mut buffer = [0_u8; 4096];
        let read = stream.read(&mut buffer).expect("request should read");
        let request = String::from_utf8_lossy(&buffer[..read]).to_string();
        let saw_auth = request.contains("authorization: Bearer test-secret")
            || request.contains("Authorization: Bearer test-secret");
        let body = serde_json::json!({ "ok": saw_auth }).to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .expect("response should write");
    });

    std::env::set_var("CHARIOX_TEST_SECRET_HTTP_TOKEN", "test-secret");
    let service = RuntimeSecretService::new(vec![UserCredentialConfig {
        id: "demo".to_string(),
        description: None,
        source: UserCredentialSourceConfig::Env {
            name: "CHARIOX_TEST_SECRET_HTTP_TOKEN".to_string(),
        },
        allowed_hosts: vec![format!("127.0.0.1:{port}")],
        allowed_uses: vec![UserCredentialUse::Http],
        injection: UserCredentialInjectionConfig::Header {
            name: "authorization".to_string(),
            value: "Bearer ${secret}".to_string(),
        },
        metadata: None,
    }]);

    let response = service
        .http_request_with_credential(CredentialHttpRequest {
            credential_id: "demo".to_string(),
            method: "GET".to_string(),
            url: format!("http://127.0.0.1:{port}/demo"),
            headers: BTreeMap::new(),
            body_text: None,
            body_json: None,
            timeout_ms: 30_000,
            max_response_bytes: 1_048_576,
        })
        .expect("credential request should succeed");
    std::env::remove_var("CHARIOX_TEST_SECRET_HTTP_TOKEN");
    server.join().expect("server should finish");

    assert_eq!(response.status, 200);
    assert_eq!(response.body_json, Some(serde_json::json!({ "ok": true })));
    assert!(!serde_json::to_string(&response)
        .unwrap()
        .contains("test-secret"));
}

#[test]
fn http_request_with_credential_rejects_wrong_host_before_secret_read() {
    let _guard = crate::env_lock::lock();
    std::env::remove_var("CHARIOX_TEST_SECRET_MISSING_TOKEN");
    let service = RuntimeSecretService::new(vec![UserCredentialConfig {
        id: "demo".to_string(),
        description: None,
        source: UserCredentialSourceConfig::Env {
            name: "CHARIOX_TEST_SECRET_MISSING_TOKEN".to_string(),
        },
        allowed_hosts: vec!["api.example.com".to_string()],
        allowed_uses: vec![UserCredentialUse::Http],
        injection: UserCredentialInjectionConfig::Header {
            name: "authorization".to_string(),
            value: "Bearer ${secret}".to_string(),
        },
        metadata: None,
    }]);

    let error = service
        .http_request_with_credential(CredentialHttpRequest {
            credential_id: "demo".to_string(),
            method: "GET".to_string(),
            url: "http://127.0.0.1:1/demo".to_string(),
            headers: BTreeMap::new(),
            body_text: None,
            body_json: None,
            timeout_ms: 30_000,
            max_response_bytes: 1_048_576,
        })
        .expect_err("wrong host should be rejected");

    assert!(error.to_string().contains("not allowed for host"));
    assert!(!error
        .to_string()
        .contains("CHARIOX_TEST_SECRET_MISSING_TOKEN"));
}

#[test]
fn browser_secret_input_rejects_wrong_host_before_secret_read() {
    let _guard = crate::env_lock::lock();
    std::env::remove_var("CHARIOX_TEST_SECRET_MISSING_BROWSER_TOKEN");
    let service = RuntimeSecretService::new(vec![UserCredentialConfig {
        id: "browser-demo".to_string(),
        description: None,
        source: UserCredentialSourceConfig::Env {
            name: "CHARIOX_TEST_SECRET_MISSING_BROWSER_TOKEN".to_string(),
        },
        allowed_hosts: vec!["accounts.google.com".to_string()],
        allowed_uses: vec![UserCredentialUse::Browser],
        injection: UserCredentialInjectionConfig::Browser,
        metadata: None,
    }]);

    let error = service
        .browser_secret_input_for_target_url("browser-demo", "https://example.com/signup")
        .expect_err("wrong browser host should be rejected before env secret read");

    assert!(error.to_string().contains("not allowed for host"));
    assert!(!error
        .to_string()
        .contains("CHARIOX_TEST_SECRET_MISSING_BROWSER_TOKEN"));
}

#[test]
fn terminal_secret_input_requires_pty_injection() {
    let _guard = crate::env_lock::lock();
    std::env::set_var("CHARIOX_TEST_TERMINAL_PASSWORD", "terminal-secret");
    let service = RuntimeSecretService::new(vec![UserCredentialConfig {
        id: "ssh_password".to_string(),
        description: None,
        source: UserCredentialSourceConfig::Env {
            name: "CHARIOX_TEST_TERMINAL_PASSWORD".to_string(),
        },
        allowed_hosts: Vec::new(),
        allowed_uses: vec![UserCredentialUse::Pty],
        injection: UserCredentialInjectionConfig::Pty,
        metadata: None,
    }]);

    assert_eq!(
        service
            .terminal_secret_input("ssh_password")
            .expect("terminal secret should resolve"),
        "terminal-secret"
    );
    std::env::remove_var("CHARIOX_TEST_TERMINAL_PASSWORD");
}

#[test]
fn provider_secret_input_requires_provider_policy() {
    let _guard = crate::env_lock::lock();
    std::env::set_var("CHARIOX_TEST_PROVIDER_TOKEN", "provider-secret");
    let service = RuntimeSecretService::new(vec![UserCredentialConfig {
        id: "claude_setup_token".to_string(),
        description: None,
        source: UserCredentialSourceConfig::Env {
            name: "CHARIOX_TEST_PROVIDER_TOKEN".to_string(),
        },
        allowed_hosts: Vec::new(),
        allowed_uses: vec![UserCredentialUse::Provider],
        injection: UserCredentialInjectionConfig::Provider,
        metadata: None,
    }]);

    assert_eq!(
        service
            .provider_secret_input("claude_setup_token")
            .expect("provider secret should resolve")
            .as_str(),
        "provider-secret"
    );
    std::env::remove_var("CHARIOX_TEST_PROVIDER_TOKEN");
}

#[test]
fn provider_secret_input_rejects_terminal_policy_before_secret_read() {
    let _guard = crate::env_lock::lock();
    std::env::remove_var("CHARIOX_TEST_PROVIDER_TOKEN_MISSING");
    let service = RuntimeSecretService::new(vec![UserCredentialConfig {
        id: "terminal_only".to_string(),
        description: None,
        source: UserCredentialSourceConfig::Env {
            name: "CHARIOX_TEST_PROVIDER_TOKEN_MISSING".to_string(),
        },
        allowed_hosts: Vec::new(),
        allowed_uses: vec![UserCredentialUse::Pty],
        injection: UserCredentialInjectionConfig::Pty,
        metadata: None,
    }]);

    let error = service
        .provider_secret_input("terminal_only")
        .expect_err("provider launch should require provider policy");
    assert!(error.to_string().contains("not allowed for Provider"));
    assert!(!error
        .to_string()
        .contains("CHARIOX_TEST_PROVIDER_TOKEN_MISSING"));
}

#[test]
fn browser_secret_input_requires_browser_use() {
    let _guard = crate::env_lock::lock();
    std::env::set_var("CHARIOX_TEST_BROWSER_PASSWORD", "browser-secret");
    let service = RuntimeSecretService::new(vec![UserCredentialConfig {
        id: "browser_password".to_string(),
        description: None,
        source: UserCredentialSourceConfig::Env {
            name: "CHARIOX_TEST_BROWSER_PASSWORD".to_string(),
        },
        allowed_hosts: Vec::new(),
        allowed_uses: vec![UserCredentialUse::Browser],
        injection: UserCredentialInjectionConfig::Browser,
        metadata: None,
    }]);

    assert_eq!(
        service
            .browser_secret_input("browser_password")
            .expect("browser secret should resolve"),
        "browser-secret"
    );
    std::env::remove_var("CHARIOX_TEST_BROWSER_PASSWORD");
}

#[test]
fn browser_secret_input_requires_browser_injection() {
    let _guard = crate::env_lock::lock();
    std::env::set_var("CHARIOX_TEST_BROWSER_PASSWORD", "browser-secret");
    let service = RuntimeSecretService::new(vec![UserCredentialConfig {
        id: "browser_password".to_string(),
        description: None,
        source: UserCredentialSourceConfig::Env {
            name: "CHARIOX_TEST_BROWSER_PASSWORD".to_string(),
        },
        allowed_hosts: Vec::new(),
        allowed_uses: vec![UserCredentialUse::Browser],
        injection: UserCredentialInjectionConfig::Header {
            name: "authorization".to_string(),
            value: "Bearer ${secret}".to_string(),
        },
        metadata: None,
    }]);

    let error = service
        .browser_secret_input("browser_password")
        .expect_err("browser input should require browser injection");
    assert!(error
        .to_string()
        .contains("not configured for browser input"));
    std::env::remove_var("CHARIOX_TEST_BROWSER_PASSWORD");
}

#[test]
fn computer_secret_input_requires_explicit_computer_policy() {
    let _guard = crate::env_lock::lock();
    std::env::set_var("CHARIOX_TEST_COMPUTER_PASSWORD", "computer-secret");
    let service = RuntimeSecretService::new(vec![UserCredentialConfig {
        id: "desktop_password".to_string(),
        description: None,
        source: UserCredentialSourceConfig::Env {
            name: "CHARIOX_TEST_COMPUTER_PASSWORD".to_string(),
        },
        allowed_hosts: Vec::new(),
        allowed_uses: vec![UserCredentialUse::Computer],
        injection: UserCredentialInjectionConfig::Computer,
        metadata: None,
    }]);

    assert_eq!(
        service
            .computer_secret_input("desktop_password")
            .expect("computer secret should resolve"),
        "computer-secret"
    );
    std::env::remove_var("CHARIOX_TEST_COMPUTER_PASSWORD");
}

#[test]
fn computer_secret_input_rejects_browser_policy_before_secret_read() {
    let _guard = crate::env_lock::lock();
    std::env::remove_var("CHARIOX_TEST_COMPUTER_SECRET_MISSING");
    let service = RuntimeSecretService::new(vec![UserCredentialConfig {
        id: "browser_only".to_string(),
        description: None,
        source: UserCredentialSourceConfig::Env {
            name: "CHARIOX_TEST_COMPUTER_SECRET_MISSING".to_string(),
        },
        allowed_hosts: Vec::new(),
        allowed_uses: vec![UserCredentialUse::Browser],
        injection: UserCredentialInjectionConfig::Browser,
        metadata: None,
    }]);

    let error = service
        .computer_secret_input("browser_only")
        .expect_err("computer input should require computer policy");
    assert!(error.to_string().contains("not allowed for Computer"));
    assert!(!error
        .to_string()
        .contains("CHARIOX_TEST_COMPUTER_SECRET_MISSING"));
}

#[test]
fn upsert_vault_backed_credential_stores_secret_and_metadata() {
    let root = std::env::temp_dir().join(format!(
        "chariox-vault-credential-upsert-test-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let registry = CharioxCredentialRegistry::new(root.clone());
    let vault = Arc::new(MemoryVaultStore::default());
    let service = RuntimeSecretService::with_vault_store(Vec::new(), "chariox-test", vault);
    let credential = UserCredentialConfig {
        id: "generated-browser-password".to_string(),
        description: Some("generated".to_string()),
        source: UserCredentialSourceConfig::Vault {
            key: "generated-browser-password".to_string(),
        },
        allowed_hosts: vec!["accounts.example.test".to_string()],
        allowed_uses: vec![UserCredentialUse::Browser],
        injection: UserCredentialInjectionConfig::Browser,
        metadata: None,
    };

    let result = service
        .upsert_vault_backed_credential_with_secret(
            &registry,
            credential.clone(),
            "secret-value",
            false,
        )
        .expect("vault-backed credential should store");

    assert_eq!(result.credential_id, "generated-browser-password");
    assert_eq!(result.vault_key, "generated-browser-password");
    assert_eq!(
        registry
            .get("generated-browser-password")
            .expect("credential should read"),
        Some(credential)
    );
    let resolving_service = RuntimeSecretService::with_vault_store(
        vec![registry
            .get("generated-browser-password")
            .expect("credential should read")
            .expect("credential should exist")],
        "chariox-test",
        service.vault_store.clone(),
    );
    assert_eq!(
        resolving_service
            .browser_secret_input("generated-browser-password")
            .expect("stored secret should resolve"),
        "secret-value"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn vault_source_resolves_without_exposing_secret_in_handles() {
    let vault = Arc::new(MemoryVaultStore::default());
    let service = RuntimeSecretService::with_vault_store(
        vec![UserCredentialConfig {
            id: "github".to_string(),
            description: None,
            source: UserCredentialSourceConfig::Vault {
                key: "github-token".to_string(),
            },
            allowed_hosts: Vec::new(),
            allowed_uses: vec![UserCredentialUse::Pty],
            injection: UserCredentialInjectionConfig::Pty,
            metadata: None,
        }],
        "chariox-test",
        vault,
    );

    service
        .set_vault_secret("github-token", "vault-secret")
        .expect("vault secret should store");

    assert_eq!(
        service
            .terminal_secret_input("github")
            .expect("vault secret should resolve"),
        "vault-secret"
    );
    let serialized = serde_json::to_string(&service.list_handles()).unwrap();
    assert!(!serialized.contains("vault-secret"));
    assert!(!serialized.contains("github-token"));
}

#[test]
fn vault_write_warms_process_cache_across_secret_service_instances() {
    let vault = Arc::new(WriteOnlyVaultStore::default());
    let service_name = format!(
        "chariox-warm-cache-test-{}",
        crate::session::unix_epoch_ms()
    );
    let writer = RuntimeSecretService::with_vault_store(Vec::new(), &service_name, vault.clone());

    writer
        .set_vault_secret("generated-password", "generated-secret")
        .expect("vault write should succeed");

    let reader = RuntimeSecretService::with_vault_store(
        vec![UserCredentialConfig {
            id: "generated-password".to_string(),
            description: None,
            source: UserCredentialSourceConfig::Vault {
                key: "generated-password".to_string(),
            },
            allowed_hosts: vec!["workspace".to_string()],
            allowed_uses: vec![UserCredentialUse::Browser],
            injection: UserCredentialInjectionConfig::Browser,
            metadata: None,
        }],
        &service_name,
        vault,
    );

    assert_eq!(
        reader
            .browser_secret_input("generated-password")
            .expect("warm cache should resolve without backing store read"),
        "generated-secret"
    );
}

#[test]
fn vault_process_cache_is_scoped_by_backend_path_service_and_key() {
    let service_name = format!(
        "chariox-cache-scope-test-{}",
        crate::session::unix_epoch_ms()
    );
    let writer = RuntimeSecretService {
        credentials: Vec::new(),
        vault_service: service_name.clone(),
        vault_backend: CredentialVaultBackend::ProcessMemory,
        vault_path: "/tmp/chariox-cache-scope-a.json".to_string(),
        vault_store: Arc::new(WriteOnlyVaultStore::default()),
    };

    writer
        .set_vault_secret("shared-key", "scoped-secret")
        .expect("writer should warm its scoped cache");

    let reader = RuntimeSecretService {
        credentials: vec![UserCredentialConfig {
            id: "shared-key".to_string(),
            description: None,
            source: UserCredentialSourceConfig::Vault {
                key: "shared-key".to_string(),
            },
            allowed_hosts: vec!["workspace".to_string()],
            allowed_uses: vec![UserCredentialUse::Browser],
            injection: UserCredentialInjectionConfig::Browser,
            metadata: None,
        }],
        vault_service: service_name,
        vault_backend: CredentialVaultBackend::ProcessMemory,
        vault_path: "/tmp/chariox-cache-scope-b.json".to_string(),
        vault_store: Arc::new(WriteOnlyVaultStore::default()),
    };

    let error = reader
        .browser_secret_input("shared-key")
        .expect_err("different vault path must not read another cache scope");

    assert!(error.to_string().contains("backing store read unavailable"));
}

#[test]
fn upsert_vault_backed_credential_restores_previous_secret_on_metadata_write_failure() {
    let root = std::env::temp_dir().join(format!(
        "chariox-vault-upsert-rollback-test-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("registry root should exist");
    let registry = CharioxCredentialRegistry::new(root.clone());
    let vault = Arc::new(MemoryVaultStore::default());
    let service = RuntimeSecretService::with_vault_store(
        vec![UserCredentialConfig {
            id: "browser-password".to_string(),
            description: None,
            source: UserCredentialSourceConfig::Vault {
                key: "browser-password".to_string(),
            },
            allowed_hosts: Vec::new(),
            allowed_uses: vec![UserCredentialUse::Browser],
            injection: UserCredentialInjectionConfig::Browser,
            metadata: None,
        }],
        "chariox-test",
        vault,
    );
    let credential = UserCredentialConfig {
        id: "browser-password".to_string(),
        description: Some("old".to_string()),
        source: UserCredentialSourceConfig::Vault {
            key: "browser-password".to_string(),
        },
        allowed_hosts: Vec::new(),
        allowed_uses: vec![UserCredentialUse::Browser],
        injection: UserCredentialInjectionConfig::Browser,
        metadata: None,
    };
    registry
        .upsert(credential.clone())
        .expect("initial metadata should write");
    service
        .set_vault_secret("browser-password", "old-secret")
        .expect("old secret should write");
    let temp_metadata_path =
        root.join(format!(".browser-password.yaml.{}.tmp", std::process::id()));
    std::fs::create_dir(&temp_metadata_path)
        .expect("registry temp metadata path should be blocked by a directory");

    let error = service
        .upsert_vault_backed_credential_with_secret(
            &registry,
            UserCredentialConfig {
                description: Some("new".to_string()),
                ..credential
            },
            "new-secret",
            true,
        )
        .expect_err("read-only registry should reject metadata write");

    assert!(error.to_string().contains("credential.upsert"));
    assert_eq!(
        service
            .browser_secret_input("browser-password")
            .expect("old secret should be restored"),
        "old-secret"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn encrypted_vault_cache_is_not_readable_after_lock() {
    let root = std::env::temp_dir().join(format!(
        "chariox-encrypted-cache-lock-test-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let path = root.join("vault.json");
    let config = UserCredentialVaultConfig {
        backend: CredentialVaultBackend::CharioxEncrypted,
        service: format!(
            "chariox-encrypted-cache-lock-test-{}",
            crate::session::unix_epoch_ms()
        ),
        path: path.to_string_lossy().to_string(),
        ..UserCredentialVaultConfig::default()
    };
    unlock_chariox_encrypted_vault(
        &path,
        "correct horse battery staple",
        VaultUnlockLease::KernelShutdown,
    )
    .expect("vault should unlock");
    let service = RuntimeSecretService::with_vault_config(
        vec![UserCredentialConfig {
            id: "generated-password".to_string(),
            description: None,
            source: UserCredentialSourceConfig::Vault {
                key: "generated-password".to_string(),
            },
            allowed_hosts: vec!["workspace".to_string()],
            allowed_uses: vec![UserCredentialUse::Browser],
            injection: UserCredentialInjectionConfig::Browser,
            metadata: None,
        }],
        &config,
    )
    .expect("encrypted service should initialize");
    service
        .set_vault_secret("generated-password", "generated-secret")
        .expect("vault write should succeed");
    assert_eq!(
        service
            .browser_secret_input("generated-password")
            .expect("warm cache should resolve while unlocked"),
        "generated-secret"
    );

    lock_chariox_encrypted_vault(&path).expect("vault should lock");
    let error = service
        .browser_secret_input("generated-password")
        .expect_err("locked encrypted vault must not read warm cache");
    assert!(is_chariox_vault_locked_error(&error));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn vault_delete_clears_process_cache() {
    let vault = Arc::new(WriteOnlyVaultStore::default());
    let service_name = format!(
        "chariox-warm-cache-delete-test-{}",
        crate::session::unix_epoch_ms()
    );
    let service = RuntimeSecretService::with_vault_store(
        vec![UserCredentialConfig {
            id: "generated-password".to_string(),
            description: None,
            source: UserCredentialSourceConfig::Vault {
                key: "generated-password".to_string(),
            },
            allowed_hosts: vec!["workspace".to_string()],
            allowed_uses: vec![UserCredentialUse::Browser],
            injection: UserCredentialInjectionConfig::Browser,
            metadata: None,
        }],
        &service_name,
        vault,
    );

    service
        .set_vault_secret("generated-password", "generated-secret")
        .expect("vault write should succeed");
    service
        .delete_vault_secret("generated-password")
        .expect("vault delete should succeed");

    let error = service
        .browser_secret_input("generated-password")
        .expect_err("cache should be cleared after delete");
    assert!(format!("{error}").contains("backing store read unavailable"));
}

#[test]
fn process_memory_backend_round_trips_across_services() {
    let _guard = crate::env_lock::lock();
    std::env::set_var("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT", "1");
    let config = UserCredentialVaultConfig {
        backend: CredentialVaultBackend::ProcessMemory,
        service: format!(
            "chariox-process-memory-test-{}",
            crate::session::unix_epoch_ms()
        ),
        ..UserCredentialVaultConfig::default()
    };
    let writer = RuntimeSecretService::with_vault_config(Vec::new(), &config)
        .expect("process memory backend should build");
    writer
        .set_vault_secret("slice-browser-password", "super-secret")
        .expect("process memory secret should store");

    let reader = RuntimeSecretService::with_vault_config(
        vec![UserCredentialConfig {
            id: "slice-browser-password".to_string(),
            description: None,
            source: UserCredentialSourceConfig::Vault {
                key: "slice-browser-password".to_string(),
            },
            allowed_hosts: vec!["workspace".to_string()],
            allowed_uses: vec![UserCredentialUse::Browser],
            injection: UserCredentialInjectionConfig::Browser,
            metadata: None,
        }],
        &config,
    )
    .expect("process memory backend should build");

    assert_eq!(
        reader
            .browser_secret_input("slice-browser-password")
            .expect("process memory secret should resolve"),
        "super-secret"
    );
    std::env::remove_var("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT");
}

#[test]
fn process_memory_backend_requires_explicit_volatile_context() {
    let _guard = crate::env_lock::lock();
    std::env::remove_var("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT");
    std::env::remove_var("CHARIOX_SLICE_MACHINE_ID");
    let config = UserCredentialVaultConfig {
        backend: CredentialVaultBackend::ProcessMemory,
        ..UserCredentialVaultConfig::default()
    };

    let error = RuntimeSecretService::with_vault_config(Vec::new(), &config)
        .expect_err("home kernels should not use volatile process memory by accident");

    assert!(error
        .to_string()
        .contains("only allowed inside Chariox slices"));
}
