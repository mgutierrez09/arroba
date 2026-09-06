use super::*;

#[test]
fn local_daemon_protocol_credential_enrollment_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let arm_request = LocalDaemonRequest::ArmDeploymentCredentialEnrollment(
        crate::local::ArmDeploymentCredentialEnrollmentRequest {
            session_id: "session-1".to_string(),
            attachment_id: "attachment-1".to_string(),
            agent_id: "agent-1".to_string(),
            enrollment_id: "enrollment-1".to_string(),
            profile_id: "profile-1".to_string(),
            target_version: 7,
        },
    );
    let interaction_request = LocalDaemonRequest::RequestCredentialEnrollmentInteraction(
        crate::local::RequestCredentialEnrollmentInteractionRequest {
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
            enrollment_id: "enrollment-1".to_string(),
            profile_id: "profile-1".to_string(),
            target_version: 7,
            provider_authorization_url: "https://claude.com/oauth/authorize?state=opaque"
                .to_string(),
            timeout_sec: Some(300),
        },
    );
    let armed_response = LocalDaemonResponse::DeploymentCredentialEnrollmentArmed {
        enrollment_id: "enrollment-1".to_string(),
        profile_id: "profile-1".to_string(),
        target_version: 7,
        session_id: "session-1".to_string(),
        agent_id: "agent-1".to_string(),
        expires_at_ms: 1_234,
    };
    let submitted_response = LocalDaemonResponse::CredentialEnrollmentInteractionResolved {
        status: crate::local::CredentialEnrollmentInteractionStatus::Submitted,
        callback: Some(crate::local::CredentialEnrollmentCallback::new(
            "https://localhost/callback?code=fixture".to_string(),
        )),
    };
    let canceled_response = LocalDaemonResponse::CredentialEnrollmentInteractionResolved {
        status: crate::local::CredentialEnrollmentInteractionStatus::Canceled,
        callback: None,
    };

    let snapshot = serde_json::json!([
        arm_request,
        interaction_request,
        armed_response,
        submitted_response,
        canceled_response,
    ]);
    assert_eq!(
        snapshot.pointer("/0/ArmDeploymentCredentialEnrollment/target_version"),
        Some(&serde_json::json!(7))
    );
    assert_eq!(
        snapshot.pointer("/1/RequestCredentialEnrollmentInteraction/provider_authorization_url"),
        Some(&serde_json::json!(
            "https://claude.com/oauth/authorize?state=opaque"
        ))
    );
    assert_eq!(
        snapshot.pointer("/3/CredentialEnrollmentInteractionResolved/status"),
        Some(&serde_json::json!("submitted"))
    );
    assert_eq!(
        snapshot.pointer("/3/CredentialEnrollmentInteractionResolved/callback"),
        Some(&serde_json::json!(
            "https://localhost/callback?code=fixture"
        ))
    );
    assert!(snapshot
        .pointer("/4/CredentialEnrollmentInteractionResolved/callback")
        .is_none());

    let serialized =
        serde_json::to_string(&snapshot).expect("credential enrollment snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "4ea4fbc3b13616f47f974889eff01ac0107fd9f9ba7b95851442a659ee30abb5"
    );
}
