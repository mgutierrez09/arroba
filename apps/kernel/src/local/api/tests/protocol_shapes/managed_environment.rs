use super::*;
use crate::local::*;

#[test]
fn local_daemon_managed_environment_control_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 287);
    let policy = ManagedEnvironmentAutoStopPolicy {
        minimum_runtime_seconds: 0,
        idle_delay_seconds: Some(900),
    };
    let context_input = ManagedEnvironmentContextPlanInput {
        source_target_id: None,
        kernel_context: ManagedEnvironmentKernelContextSelection::Empty,
        development_setup: ManagedEnvironmentDevelopmentSetup::Empty,
        provider_accounts: ManagedEnvironmentProviderAccounts::None,
        git_credentials: ManagedEnvironmentGitCredentials::None,
    };
    let source_context_input = ManagedEnvironmentContextPlanInput {
        source_target_id: Some("source-target-1".to_string()),
        kernel_context: ManagedEnvironmentKernelContextSelection::SourceKernel,
        development_setup: ManagedEnvironmentDevelopmentSetup::SourceProject {
            project_id: "project-1".to_string(),
            repositories: vec![
                ManagedEnvironmentRepositorySelection {
                    role: ManagedEnvironmentRepositoryRole::Primary,
                    workspace_id: "workspace-primary".to_string(),
                    worktree_id: Some("worktree-primary".to_string()),
                },
                ManagedEnvironmentRepositorySelection {
                    role: ManagedEnvironmentRepositoryRole::Supporting,
                    workspace_id: "workspace-supporting".to_string(),
                    worktree_id: None,
                },
            ],
        },
        provider_accounts: ManagedEnvironmentProviderAccounts::Selected {
            accounts: vec![ManagedEnvironmentProviderAccountSelection {
                provider: "codex".to_string(),
                account_profile: "work".to_string(),
            }],
        },
        git_credentials: ManagedEnvironmentGitCredentials::Selected {
            credential_ids: vec!["github-work".to_string()],
        },
    };
    let environment = managed_environment_summary(policy.clone(), context_input.clone());
    let mut source_environment =
        managed_environment_summary(policy.clone(), source_context_input.clone());
    source_environment.context_plan.source = Some(ManagedEnvironmentContextSource {
        source_target_id: "source-target-1".to_string(),
        relay_realm_id: "realm-1".to_string(),
        machine_id: "machine-1".to_string(),
        kernel_id: "kernel-1".to_string(),
        key_thumbprint: "sha256:source-key".to_string(),
    });
    let operation = ManagedEnvironmentOperationSummary {
        operation_id: "operation-1".to_string(),
        environment_id: "environment-1".to_string(),
        requested_by_user_id: "user-1".to_string(),
        kind: ManagedEnvironmentOperationKind::Create,
        idempotency_key: "create-1".to_string(),
        request_digest: format!("sha256:{}", "c".repeat(64)),
        desired_revision: 1,
        status: ManagedEnvironmentOperationStatus::Pending,
        attempt: 0,
        retryable: false,
        failure_code: None,
        failure_message: None,
        completed_at: None,
        created_at: "2026-08-21T00:00:00.000Z".to_string(),
        updated_at: "2026-08-21T00:00:00.000Z".to_string(),
    };
    let transfer_ticket = crate::managed_context::outbound_service::ManagedContextTransferTicket {
        environment_id: "environment-1".to_string(),
        context_plan: crate::managed_bootstrap::ManagedKernelContextPlan::source_project_for_tests(
            "context-1",
            "realm-1",
            "kernel-1",
            "sha256:source-key",
            "project-1",
        ),
        target: crate::managed_context::outbound_service::ManagedContextTransferTarget {
            relay_realm_id: "realm-1".to_string(),
            machine_id: "managed-machine-1".to_string(),
            kernel_id: "managed-kernel-1".to_string(),
            relay_public_key: "target-public-key".to_string(),
            key_thumbprint: "sha256:target-key".to_string(),
        },
    };
    let snapshot = serde_json::json!([
        LocalDaemonRequest::ListManagedEnvironmentCatalog(ListManagedEnvironmentCatalogRequest,),
        LocalDaemonRequest::CreateManagedEnvironment(CreateManagedEnvironmentRequest {
            client_request_id: "create-1".to_string(),
            name: "Managed agent".to_string(),
            region: "hel1".to_string(),
            compute_class: "agent-small".to_string(),
            auto_stop_policy: policy.clone(),
            context_plan: context_input,
        }),
        LocalDaemonRequest::CreateManagedEnvironment(CreateManagedEnvironmentRequest {
            client_request_id: "create-source-1".to_string(),
            name: "Managed source agent".to_string(),
            region: "fsn1".to_string(),
            compute_class: "agent-medium".to_string(),
            auto_stop_policy: ManagedEnvironmentAutoStopPolicy {
                minimum_runtime_seconds: 300,
                idle_delay_seconds: None,
            },
            context_plan: source_context_input,
        }),
        LocalDaemonRequest::GetManagedEnvironment(GetManagedEnvironmentRequest {
            environment_id: "environment-1".to_string(),
        }),
        LocalDaemonRequest::PrepareManagedEnvironmentContextTransfer(
            PrepareManagedEnvironmentContextTransferRequest {
                environment_id: "environment-1".to_string(),
            },
        ),
        LocalDaemonRequest::RequestManagedEnvironmentLifecycle(
            RequestManagedEnvironmentLifecycleRequest {
                environment_id: "environment-1".to_string(),
                action: ManagedEnvironmentLifecycleAction::Start,
                idempotency_key: "start-1".to_string(),
            },
        ),
        LocalDaemonRequest::RequestManagedEnvironmentLifecycle(
            RequestManagedEnvironmentLifecycleRequest {
                environment_id: "environment-1".to_string(),
                action: ManagedEnvironmentLifecycleAction::Stop,
                idempotency_key: "stop-1".to_string(),
            },
        ),
        LocalDaemonRequest::RequestManagedEnvironmentLifecycle(
            RequestManagedEnvironmentLifecycleRequest {
                environment_id: "environment-1".to_string(),
                action: ManagedEnvironmentLifecycleAction::Restart,
                idempotency_key: "restart-1".to_string(),
            },
        ),
        LocalDaemonRequest::RequestManagedEnvironmentLifecycle(
            RequestManagedEnvironmentLifecycleRequest {
                environment_id: "environment-1".to_string(),
                action: ManagedEnvironmentLifecycleAction::Delete,
                idempotency_key: "delete-1".to_string(),
            },
        ),
        LocalDaemonResponse::ManagedEnvironmentCatalog {
            catalog: ManagedEnvironmentCatalog {
                compute_classes: vec![ManagedEnvironmentComputeClassOption {
                    compute_class: "agent-small".to_string(),
                    regions: vec!["hel1".to_string()],
                }],
                context_sources: vec![],
                environments: vec![environment.clone(), source_environment.clone()],
            },
        },
        LocalDaemonResponse::ManagedEnvironment {
            environment: source_environment.clone(),
        },
        LocalDaemonResponse::ManagedEnvironmentContextTransferPrepared {
            ticket: transfer_ticket,
        },
        LocalDaemonResponse::ManagedEnvironmentCreated {
            result: ManagedEnvironmentResult {
                environment: environment.clone(),
                operation: operation.clone(),
            },
        },
        LocalDaemonResponse::ManagedEnvironmentLifecycleRequested {
            result: ManagedEnvironmentResult {
                environment: source_environment,
                operation,
            },
        },
        serde_json::json!({
            "desiredStates": [
                ManagedEnvironmentDesiredState::Running,
                ManagedEnvironmentDesiredState::Stopped,
                ManagedEnvironmentDesiredState::Deleted,
            ],
            "observedStates": [
                ManagedEnvironmentObservedState::Requested,
                ManagedEnvironmentObservedState::Provisioning,
                ManagedEnvironmentObservedState::Bootstrapping,
                ManagedEnvironmentObservedState::AwaitingContext,
                ManagedEnvironmentObservedState::Ready,
                ManagedEnvironmentObservedState::Starting,
                ManagedEnvironmentObservedState::Stopping,
                ManagedEnvironmentObservedState::Stopped,
                ManagedEnvironmentObservedState::Deleting,
                ManagedEnvironmentObservedState::Deleted,
                ManagedEnvironmentObservedState::Failed,
            ],
            "operationKinds": [
                ManagedEnvironmentOperationKind::Create,
                ManagedEnvironmentOperationKind::Start,
                ManagedEnvironmentOperationKind::Stop,
                ManagedEnvironmentOperationKind::Restart,
                ManagedEnvironmentOperationKind::Delete,
            ],
            "operationStatuses": [
                ManagedEnvironmentOperationStatus::Pending,
                ManagedEnvironmentOperationStatus::Running,
                ManagedEnvironmentOperationStatus::Succeeded,
                ManagedEnvironmentOperationStatus::Failed,
            ],
        }),
    ]);

    assert_eq!(
        snapshot.pointer("/1/CreateManagedEnvironment/contextPlan/kernelContext"),
        Some(&serde_json::json!("empty"))
    );
    assert_eq!(
        snapshot.pointer(
            "/2/CreateManagedEnvironment/contextPlan/developmentSetup/repositories/1/role"
        ),
        Some(&serde_json::json!("supporting"))
    );
    assert_eq!(
        snapshot.pointer(
            "/2/CreateManagedEnvironment/contextPlan/providerAccounts/accounts/0/accountProfile"
        ),
        Some(&serde_json::json!("work"))
    );
    assert_eq!(
        snapshot.pointer("/9/ManagedEnvironmentCatalog/catalog/computeClasses/0/computeClass"),
        Some(&serde_json::json!("agent-small"))
    );
    assert_eq!(
        snapshot.pointer("/13/ManagedEnvironmentLifecycleRequested/result/operation/status"),
        Some(&serde_json::json!("pending"))
    );
    assert_eq!(
        snapshot.pointer("/9/ManagedEnvironmentCatalog/catalog/environments/0/runtimeKernelId"),
        Some(&serde_json::json!("managed-kernel-1"))
    );
    assert_eq!(
        snapshot.pointer("/11/ManagedEnvironmentContextTransferPrepared/ticket/target/kernelId"),
        Some(&serde_json::json!("managed-kernel-1"))
    );
    let serialized = serde_json::to_string(&snapshot).expect("managed environment shape");
    assert_eq!(
        format!("{:x}", Sha256::digest(serialized.as_bytes())),
        "2d5398f770de7285663e8d7fca403a5213f9a2320c8b078d0b7c2c0795417b0a"
    );
}

fn managed_environment_summary(
    policy: ManagedEnvironmentAutoStopPolicy,
    context_input: ManagedEnvironmentContextPlanInput,
) -> ManagedEnvironmentSummary {
    ManagedEnvironmentSummary {
        environment_id: "environment-1".to_string(),
        account_id: "account-1".to_string(),
        created_by_user_id: "user-1".to_string(),
        name: "Managed agent".to_string(),
        region: "hel1".to_string(),
        compute_class: "agent-small".to_string(),
        desired_state: ManagedEnvironmentDesiredState::Running,
        observed_state: ManagedEnvironmentObservedState::Requested,
        desired_revision: 1,
        observed_revision: 0,
        runtime_machine_id: Some("managed-machine-1".to_string()),
        runtime_kernel_id: Some("managed-kernel-1".to_string()),
        runtime_release_digest: None,
        context_plan: ManagedEnvironmentContextPlan {
            schema_version: 1,
            context_id: "context-1".to_string(),
            plan_digest: format!("sha256:{}", "a".repeat(64)),
            source: None,
            kernel_context: context_input.kernel_context,
            development_setup: context_input.development_setup,
            provider_accounts: context_input.provider_accounts,
            git_credentials: context_input.git_credentials,
        },
        context_manifest_digest: None,
        auto_stop_policy: policy,
        last_error_code: None,
        last_error_message: None,
        created_at: "2026-08-21T00:00:00.000Z".to_string(),
        updated_at: "2026-08-21T00:00:00.000Z".to_string(),
    }
}
