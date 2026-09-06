use super::*;

#[test]
fn local_daemon_managed_context_outbound_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 310);
    let plan = crate::managed_bootstrap::ManagedKernelContextPlan::source_project_for_tests(
        "context-1",
        "realm-1",
        "source-kernel",
        &"a".repeat(64),
        "project-1",
    );
    let ticket = crate::managed_context::outbound_service::ManagedContextTransferTicket {
        environment_id: "environment-1".to_string(),
        context_plan: plan,
        target: crate::managed_context::outbound_service::ManagedContextTransferTarget {
            relay_realm_id: "realm-1".to_string(),
            machine_id: "target-machine".to_string(),
            kernel_id: "target-kernel".to_string(),
            relay_public_key: "target-public-key".to_string(),
            key_thumbprint: "b".repeat(64),
        },
    };
    let status = crate::managed_context::outbound_service::ManagedContextOutboundOperationStatus {
        context_id: "context-1".to_string(),
        plan_digest: ticket.context_plan.package_binding().plan_digest,
        phase: crate::managed_context::outbound_service::ManagedContextOutboundOperationPhase::Uploading,
        accepted_bytes: 512,
        package_size_bytes: 1_024,
        receipt: None,
        failure_code: None,
        failure_message: None,
        retryable: false,
        updated_at_ms: 1_234,
    };
    let snapshot = serde_json::json!([
        LocalDaemonRequest::StartManagedContextTransfer(
            crate::local::StartManagedContextTransferRequest { ticket },
        ),
        LocalDaemonRequest::GetManagedContextTransferStatus(
            crate::local::GetManagedContextTransferStatusRequest {
                context_id: "context-1".to_string(),
            },
        ),
        LocalDaemonRequest::GetManagedContextLaunchTarget(
            crate::local::GetManagedContextLaunchTargetRequest {
                context_id: "context-1".to_string(),
                plan_digest: status.plan_digest.clone(),
            },
        ),
        LocalDaemonResponse::ManagedContextTransferStarted {
            status: status.clone(),
        },
        LocalDaemonResponse::ManagedContextTransferStatus {
            status: status.clone(),
        },
        LocalDaemonResponse::ManagedContextLaunchTarget {
            target: crate::local::ManagedContextLaunchTarget {
                environment_id: "environment-empty".to_string(),
                kernel_id: "target-kernel".to_string(),
                context_id: "context-empty".to_string(),
                plan_digest: status.plan_digest.clone(),
                development: crate::local::ManagedContextDevelopmentLaunchTarget::Empty {
                    workspace_path: "/managed/empty-workspace".to_string(),
                },
            },
        },
        LocalDaemonResponse::ManagedContextLaunchTarget {
            target: crate::local::ManagedContextLaunchTarget {
                environment_id: "environment-1".to_string(),
                kernel_id: "target-kernel".to_string(),
                context_id: "context-1".to_string(),
                plan_digest: status.plan_digest,
                development: crate::local::ManagedContextDevelopmentLaunchTarget::FromSource {
                    project_id: "project-1".to_string(),
                    destination_root: "/managed/context".to_string(),
                    primary_repository_id: "repository-1".to_string(),
                    repositories: vec![crate::local::ManagedContextRepositoryLaunchTarget {
                        repository_id: "repository-1".to_string(),
                        role:
                            crate::managed_context::development::DevelopmentRepositoryRole::Primary,
                        target_directory: "primary".to_string(),
                        workspace_path: "/managed/context/primary".to_string(),
                        head_sha: "c".repeat(40),
                    }],
                },
            },
        },
    ]);
    assert_eq!(
        snapshot.pointer("/0/StartManagedContextTransfer/ticket/environmentId"),
        Some(&serde_json::json!("environment-1"))
    );
    assert_eq!(
        snapshot.pointer("/0/StartManagedContextTransfer/ticket/contextPlan/developmentSetup/kind"),
        Some(&serde_json::json!("source_project"))
    );
    assert_eq!(
        snapshot.pointer("/3/ManagedContextTransferStarted/status/phase"),
        Some(&serde_json::json!("uploading"))
    );
    assert_eq!(
        snapshot.pointer("/5/ManagedContextLaunchTarget/target/development/workspacePath"),
        Some(&serde_json::json!("/managed/empty-workspace"))
    );
    assert_eq!(
        snapshot.pointer(
            "/6/ManagedContextLaunchTarget/target/development/repositories/0/workspacePath"
        ),
        Some(&serde_json::json!("/managed/context/primary"))
    );
    assert_eq!(
        snapshot.pointer("/6/ManagedContextLaunchTarget/target/development/projectId"),
        Some(&serde_json::json!("project-1"))
    );
    assert_eq!(
        snapshot.pointer("/6/ManagedContextLaunchTarget/target/development/destinationRoot"),
        Some(&serde_json::json!("/managed/context"))
    );
    let serialized = serde_json::to_string(&snapshot).expect("managed-context shape should encode");
    assert_eq!(
        format!("{:x}", Sha256::digest(serialized.as_bytes())),
        "177b7975c738d66d7ed3245f52a3fbf7e660ff1006bbd47e4ae7f0aee5862d89"
    );
}

#[test]
fn managed_context_launch_target_reads_schema_v4_variant_fields() {
    let development =
        serde_json::from_value::<crate::local::ManagedContextDevelopmentLaunchTarget>(
            serde_json::json!({
                "kind": "from_source",
                "project_id": "project-1",
                "destination_root": "/managed/context",
                "primary_repository_id": "repository-1",
                "repositories": [{
                    "repositoryId": "repository-1",
                    "role": "primary",
                    "targetDirectory": "primary",
                    "workspacePath": "/managed/context/primary",
                    "headSha": "c".repeat(40),
                }],
            }),
        )
        .expect("schema-v4 launch target fields remain readable");
    assert!(matches!(
        development,
        crate::local::ManagedContextDevelopmentLaunchTarget::FromSource { .. }
    ));
    let empty = serde_json::from_value::<crate::local::ManagedContextDevelopmentLaunchTarget>(
        serde_json::json!({ "kind": "empty" }),
    )
    .expect("schema-v4 empty launch target remains readable");
    assert_eq!(
        empty,
        crate::local::ManagedContextDevelopmentLaunchTarget::Empty {
            workspace_path: String::new(),
        }
    );
}
