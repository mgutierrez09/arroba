use super::*;
use sha2::{Digest, Sha256};

#[test]
fn local_daemon_protocol_provider_account_credential_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 310);

    let request = LocalDaemonRequest::SetProviderAccountCredential(
        crate::local::SetProviderAccountCredentialRequest {
            session_id: Some("session-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            provider: "claude".to_string(),
            account_profile: "work".to_string(),
            value: "setup-token-secret".to_string(),
            overwrite: true,
        },
    );
    let response = LocalDaemonResponse::ProviderAccountCredentialStored {
        provider: "claude".to_string(),
        account_profile: "work".to_string(),
        credential_id: "provider-account-claude-handle".to_string(),
        replaced: true,
    };
    let snapshot = serde_json::json!([request, response]);
    assert_eq!(
        snapshot.pointer("/0/SetProviderAccountCredential/account_profile"),
        Some(&serde_json::json!("work"))
    );
    assert_eq!(
        snapshot.pointer("/0/SetProviderAccountCredential/overwrite"),
        Some(&serde_json::json!(true))
    );
    assert_eq!(
        snapshot.pointer("/1/ProviderAccountCredentialStored/credential_id"),
        Some(&serde_json::json!("provider-account-claude-handle"))
    );

    let encoded = serde_json::to_string(&snapshot).expect("snapshot should encode");
    let hash = Sha256::digest(encoded.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "d018a75374d241bfe0555258b4c90fdc5b85e04c6c859181b6f7c807eacd9d81"
    );
}
