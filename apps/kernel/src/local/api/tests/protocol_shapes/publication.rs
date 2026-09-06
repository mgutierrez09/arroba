use super::*;

#[test]
fn local_daemon_protocol_workflow_publication_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 312);

    let create_request = LocalDaemonRequest::CreateWorkflowPublication(
        crate::local::CreateWorkflowPublicationRequest {
            session_id: "session-1".to_string(),
            workflow_ref: "workflow-1".to_string(),
            endpoint_ref: "endpoint-1".to_string(),
            expected_workflow_revision: Some(7),
            operation_key: Some("publish-operation-1".to_string()),
            queue_ref: Some("priority".to_string()),
            alias: Some("public_qa".to_string()),
            kind: Some("ingress".to_string()),
            route: Some("/qa/*".to_string()),
            methods: vec!["GET".to_string()],
            transport: Some(serde_json::json!({"kind": "human_http"})),
            parser: Some(serde_json::json!({"kind": "path_template", "template": "/qa/:task"})),
            input_schema: Some(serde_json::json!({"type": "object"})),
            trace_exposure: Some(serde_json::json!({
                "nodes": {
                    "node-1": ["output_summary", "assistant_messages", "tool_use"]
                }
            })),
            mode: Some("async".to_string()),
            sync_timeout_ms: Some(240_000),
            poll_ms: Some(250),
        },
    );
    let list_request = LocalDaemonRequest::ListWorkflowPublications(
        crate::local::ListWorkflowPublicationsRequest {
            session_id: "session-1".to_string(),
        },
    );
    let get_request =
        LocalDaemonRequest::GetWorkflowPublication(crate::local::GetWorkflowPublicationRequest {
            session_id: "session-1".to_string(),
            publication_ref: "publication-1".to_string(),
        });
    let export_request = LocalDaemonRequest::ExportWorkflowPublicationPackage(
        crate::local::ExportWorkflowPublicationPackageRequest {
            session_id: "session-1".to_string(),
            publication_ref: "publication-1".to_string(),
            kernel_url: Some("ws://127.0.0.1:43118".to_string()),
            agent_app: Some(serde_json::json!({
                "enabled": true,
                "assets": {
                    "public_dir": "app",
                    "index": "index.html"
                },
                "routes": [{
                    "path": "/add/*",
                    "hook_id": "publication-1-hook",
                    "prompt_source": "path_tail",
                    "response": "streaming_shell",
                    "required_role": "public",
                    "manipulation": {
                        "level": "state_and_overlay",
                        "scope": "session",
                        "allowed_paths": ["/generated/**"],
                        "protected_paths": ["/auth/**"],
                        "allowed_actions": ["cart.search", "cart.add"]
                    }
                }],
                "replicas": {
                    "count": 2,
                    "per_caller_ordering": true,
                    "max_queue_depth": 100
                },
                "persistent_patch": {
                    "enabled": false
                }
            })),
            agent_app_assets_dir: Some("/repo/dist".to_string()),
        },
    );
    let disable_request = LocalDaemonRequest::DisableWorkflowPublication(
        crate::local::DisableWorkflowPublicationRequest {
            session_id: "session-1".to_string(),
            publication_ref: "publication-1".to_string(),
        },
    );
    let register_endpoint_request = LocalDaemonRequest::RegisterWorkflowPublicationEndpoint(
        crate::local::RegisterWorkflowPublicationEndpointRequest {
            session_id: "session-1".to_string(),
            publication_ref: "publication-1".to_string(),
            local_url: "http://127.0.0.1:3000/".to_string(),
            runtime_session_id: Some("runtime-session-1".to_string()),
            ttl_ms: Some(600_000),
        },
    );
    let control_runtime_request = LocalDaemonRequest::ControlWorkflowPublicationRuntime(
        crate::local::ControlWorkflowPublicationRuntimeRequest {
            session_id: "session-1".to_string(),
            publication_ref: "publication-1".to_string(),
            action: crate::local::WorkflowPublicationRuntimeAction::Start,
            host: Some("127.0.0.1".to_string()),
            port: Some(3000),
            kernel_url: Some("ws://127.0.0.1:43118".to_string()),
        },
    );
    let inspect_runtime_request = LocalDaemonRequest::ControlWorkflowPublicationRuntime(
        crate::local::ControlWorkflowPublicationRuntimeRequest {
            session_id: "session-1".to_string(),
            publication_ref: "publication-1".to_string(),
            action: crate::local::WorkflowPublicationRuntimeAction::Inspect,
            host: None,
            port: None,
            kernel_url: None,
        },
    );
    let bind_deployment_request = LocalDaemonRequest::BindWorkflowPublicationDeployment(
        crate::local::BindWorkflowPublicationDeploymentRequest {
            session_id: "session-1".to_string(),
            publication_ref: "publication-1".to_string(),
            setup_id: "setup-1".to_string(),
            operation_key: "deployment-setup:setup-1:runtime".to_string(),
            deployment_id: "deployment-1".to_string(),
            environment_id: "environment-1".to_string(),
            release_id: "release-1".to_string(),
            package_digest: "sha256:abc123".to_string(),
            desired_revision: 7,
            caller_claims_public_key_pem: "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEA/pMgE2dD4Y9eL57S6f9+lve+T2A4M0ueD5GmOZfHjkI=\n-----END PUBLIC KEY-----\n".to_string(),
        },
    );
    let mut workflow =
        crate::session::WorkflowDefinition::new("workflow-1", Some("qa".to_string()));
    workflow.add_schema(crate::session::WorkflowSchemaDefinition::new(
        "schema-1",
        Some("Answer".to_string()),
        Some("Final answer schema".to_string()),
        serde_json::json!({
            "type": "object",
            "required": ["answer"],
            "properties": {
                "answer": { "type": "string" }
            },
            "additionalProperties": false
        }),
    ));
    workflow.add_node(crate::session::WorkflowNodeDefinition::new(
        "node-1", "agent-1",
    ));
    let endpoint = workflow.add_endpoint(crate::session::WorkflowEndpointDefinition::new(
        "endpoint-1",
        Some("ask".to_string()),
        "node-1",
    ));
    let snapshot = crate::local::WorkflowPublicationSnapshot {
        schema_version: 1,
        captured_at_ms: Some(42),
        source_session: Some(crate::local::WorkflowPublicationSourceSessionSnapshot {
            id: Some("session-1".to_string()),
            alias: Some("authoring".to_string()),
            workspace_id: "/repo".to_string(),
            worktree_id: "/repo".to_string(),
        }),
        workflow: workflow.clone(),
        endpoint: Some(endpoint.clone()),
        queues: vec![crate::session::WorkflowPromptQueueDefinition::new(
            "workflow-1:default",
            "workflow-1",
            "default",
            0,
        )],
        schedules: vec![crate::session::WorkflowScheduleDefinition::new(
            "watchdog-1",
            "workflow-1",
            "endpoint-1",
            60,
            "scheduled prompt",
            crate::session::WorkflowWatchdogPolicy::Queue,
            Some(3),
        )],
        agents: vec![crate::agent::AgentInstance::new(
            "agent-1",
            "agent-ref-1",
            "session-1",
            Some("worker".to_string()),
            "codex",
            Some("gpt-5".to_string()),
            None,
            Some("/repo".to_string()),
            crate::agent::GridPosition::new(0, 0, 1, 1),
        )],
    };
    let materialize_request = LocalDaemonRequest::MaterializeWorkflowPublication(Box::new(
        crate::local::MaterializeWorkflowPublicationRequest {
            publication_id: "publication-1".to_string(),
            snapshot: snapshot.clone(),
            runtime_key: Some("deployment-1:replica-0".to_string()),
        },
    ));
    let mut publication = crate::session::WorkflowPublicationDefinition::new_immutable(
        "publication-1",
        "session-1",
        "workflow-1",
        "endpoint-1",
        Some("priority".to_string()),
        Some("public_qa".to_string()),
        "ingress",
        Some("/qa/*".to_string()),
        vec!["GET".to_string()],
        Some(serde_json::json!({"kind": "human_http"})),
        Some(serde_json::json!({"kind": "path_template", "template": "/qa/:task"})),
        Some(serde_json::json!({"type": "object"})),
        Some(serde_json::json!({
            "nodes": {
                "node-1": ["output_summary", "assistant_messages", "tool_use"]
            }
        })),
        Some("async".to_string()),
        Some(240_000),
        Some(250),
        7,
        "sha256:publication-source-1".to_string(),
        Some("publish-operation-1".to_string()),
        Some("sha256:publication-request-1".to_string()),
        "local",
    );
    publication.set_runtime_materialization(
        crate::session::WorkflowPublicationRuntimeMaterialization {
            key: "deployment-1:replica-0".to_string(),
            agent_id_map: BTreeMap::from([("agent-1".to_string(), "runtime-agent-1".to_string())]),
        },
    );
    let session = crate::session::RuntimeSession::new(
        "session-1",
        None,
        "/repo",
        "/repo",
        "machine-1",
        "daemon-1",
    );
    let mut served_publication = publication.clone();
    served_publication.mark_served(
        "running",
        "https://relay.example.test/display/publication-1/",
        serde_json::json!({
            "kind": "tunnel",
            "url": "https://relay.example.test/display/publication-1/",
            "local_url": "http://127.0.0.1:3000/",
            "runtime_session_id": "runtime-session-1",
            "expires_at_ms": 600_042
        }),
    );
    let mut runtime_publication = publication.clone();
    runtime_publication.mark_runtime_status(
        "starting",
        Some(Some("http://127.0.0.1:3000/".to_string())),
        Some(serde_json::json!({
            "kind": "local_runtime",
            "status": "starting",
            "host": "127.0.0.1",
            "port": 3000,
            "local_url": "http://127.0.0.1:3000/",
            "process_id": 4242,
            "package_root": "/tmp/chariox-publication-runtimes/session-1/publication-1-sha256_abc123"
        })),
    );
    runtime_publication.set_runtime_observability(
        Some(serde_json::json!({ "reachable": true })),
        vec![serde_json::json!({
            "id": "watchdog-1",
            "workflow_id": "workflow-1",
            "endpoint_id": "endpoint-1",
            "queue_id": "priority",
            "next_run_at_ms": 600_000,
        })],
        Some(serde_json::json!({
            "id": "run-1",
            "status": "Completed",
            "workflow_id": "workflow-1",
            "endpoint_id": "endpoint-1",
        })),
        vec![serde_json::json!({
            "id": "run-1",
            "status": "Completed",
            "workflow_id": "workflow-1",
            "endpoint_id": "endpoint-1",
        })],
        Some(serde_json::json!({
            "kind": "final",
            "message": { "value": 1842 },
            "artifacts": [],
        })),
    );
    let mut materialized_session = crate::session::RuntimeSession::new(
        "runtime-session-1",
        None,
        "/repo",
        "/repo",
        "machine-1",
        "daemon-1",
    );
    materialized_session.set_hidden(true);
    let snapshot = serde_json::json!([
        create_request,
        list_request,
        get_request,
        export_request,
        disable_request,
        materialize_request,
        LocalDaemonResponse::WorkflowPublicationCreated {
            publication: publication.clone(),
            session: session.clone(),
        },
        LocalDaemonResponse::WorkflowPublicationsListed {
            publications: vec![publication.clone()],
        },
        LocalDaemonResponse::WorkflowPublication {
            publication: publication.clone(),
        },
        LocalDaemonResponse::WorkflowPublicationPackageExported {
            publication: publication.clone(),
            package_version: 2,
            package_digest: "sha256:abc123".to_string(),
            package_archive_base64: "YXJjaGl2ZQ==".to_string(),
            package_files: vec![crate::local::WorkflowPublicationPackageFile {
                path: "publication.json".to_string(),
                content_base64: "e30K".to_string(),
                executable: false,
            }],
        },
        LocalDaemonResponse::WorkflowPublicationDisabled {
            publication: publication.clone(),
            session: session.clone(),
        },
        LocalDaemonResponse::WorkflowPublicationMaterialized {
            publication_id: "publication-1".to_string(),
            session: materialized_session,
            agent_id_map: BTreeMap::from([("agent-1".to_string(), "agent-2".to_string(),)]),
        },
        register_endpoint_request,
        LocalDaemonResponse::WorkflowPublicationEndpointRegistered {
            publication: served_publication.clone(),
            open_url: "https://relay.example.test/display/publication-1/".to_string(),
            viewer_url: "https://relay.example.test/display/publication-1/".to_string(),
            access: "tunnel".to_string(),
            expires_at_ms: Some(600_042),
        },
        control_runtime_request,
        LocalDaemonResponse::WorkflowPublicationRuntimeControlled {
            publication: runtime_publication.clone(),
            action: crate::local::WorkflowPublicationRuntimeAction::Start,
            status: "starting".to_string(),
            local_url: Some("http://127.0.0.1:3000/".to_string()),
            open_url: Some("http://127.0.0.1:3000/".to_string()),
            viewer_url: Some("http://127.0.0.1:3000/".to_string()),
            process_id: Some(4242),
            message: Some("publication runtime starting; endpoint registration will publish a relay display URL when available".to_string()),
        },
        inspect_runtime_request,
        LocalDaemonResponse::WorkflowPublicationRuntimeControlled {
            publication: runtime_publication,
            action: crate::local::WorkflowPublicationRuntimeAction::Inspect,
            status: "running".to_string(),
            local_url: Some("http://127.0.0.1:3000/".to_string()),
            open_url: Some("http://127.0.0.1:3000/".to_string()),
            viewer_url: Some("http://127.0.0.1:3000/".to_string()),
            process_id: Some(4242),
            message: Some("publication runtime is running".to_string()),
        },
        bind_deployment_request,
        LocalDaemonResponse::WorkflowPublicationDeploymentBound {
            publication: Box::new(served_publication),
            operation_key: "deployment-setup:setup-1:runtime".to_string(),
            deployment_id: "deployment-1".to_string(),
            release_id: "release-1".to_string(),
            package_digest: "sha256:abc123".to_string(),
            desired_revision: 7,
            state: "running".to_string(),
            runtime_session_id: Some("runtime-session-1".to_string()),
            local_url: Some("http://127.0.0.1:3000/".to_string()),
            tunnel_url: Some(
                "https://relay.example.test/display/publication-1/".to_string(),
            ),
            process_id: Some(4242),
            replayed: false,
        },
        LocalDaemonRequest::ActivateWorkflowPublicationRuntime(crate::local::ActivateWorkflowPublicationRuntimeRequest {
            publication_id: "publication-1".to_string(), runtime_keys: vec!["deployment-1:replica-0".to_string()],
        }),
        LocalDaemonResponse::WorkflowPublicationRuntimeActivated {
            publication_id: "publication-1".to_string(), runtime_keys: vec!["deployment-1:replica-0".to_string()],
        },
    ]);
    let mut snapshot = snapshot;
    for path in [
        "/5/MaterializeWorkflowPublication/snapshot/workflow/created_at_ms",
        "/5/MaterializeWorkflowPublication/snapshot/queues/0/created_at_ms",
        "/5/MaterializeWorkflowPublication/snapshot/queues/0/updated_at_ms",
        "/5/MaterializeWorkflowPublication/snapshot/schedules/0/created_at_ms",
        "/5/MaterializeWorkflowPublication/snapshot/schedules/0/next_run_at_ms",
        "/5/MaterializeWorkflowPublication/snapshot/schedules/0/updated_at_ms",
        "/5/MaterializeWorkflowPublication/snapshot/agents/0/created_at_ms",
        "/5/MaterializeWorkflowPublication/snapshot/agents/0/last_activity_at_ms",
        "/6/WorkflowPublicationCreated/publication/created_at_ms",
        "/6/WorkflowPublicationCreated/publication/updated_at_ms",
        "/6/WorkflowPublicationCreated/session/created_at_ms",
        "/6/WorkflowPublicationCreated/session/last_used_at_ms",
        "/7/WorkflowPublicationsListed/publications/0/created_at_ms",
        "/7/WorkflowPublicationsListed/publications/0/updated_at_ms",
        "/8/WorkflowPublication/publication/created_at_ms",
        "/8/WorkflowPublication/publication/updated_at_ms",
        "/9/WorkflowPublicationPackageExported/publication/created_at_ms",
        "/9/WorkflowPublicationPackageExported/publication/updated_at_ms",
        "/10/WorkflowPublicationDisabled/publication/created_at_ms",
        "/10/WorkflowPublicationDisabled/publication/updated_at_ms",
        "/10/WorkflowPublicationDisabled/session/created_at_ms",
        "/10/WorkflowPublicationDisabled/session/last_used_at_ms",
        "/11/WorkflowPublicationMaterialized/session/created_at_ms",
        "/11/WorkflowPublicationMaterialized/session/last_used_at_ms",
        "/13/WorkflowPublicationEndpointRegistered/publication/created_at_ms",
        "/13/WorkflowPublicationEndpointRegistered/publication/runtime_last_heartbeat_at_ms",
        "/13/WorkflowPublicationEndpointRegistered/publication/runtime_logs/0/at_ms",
        "/13/WorkflowPublicationEndpointRegistered/publication/updated_at_ms",
        "/15/WorkflowPublicationRuntimeControlled/publication/created_at_ms",
        "/15/WorkflowPublicationRuntimeControlled/publication/runtime_last_heartbeat_at_ms",
        "/15/WorkflowPublicationRuntimeControlled/publication/runtime_logs/0/at_ms",
        "/15/WorkflowPublicationRuntimeControlled/publication/updated_at_ms",
        "/17/WorkflowPublicationRuntimeControlled/publication/created_at_ms",
        "/17/WorkflowPublicationRuntimeControlled/publication/runtime_last_heartbeat_at_ms",
        "/17/WorkflowPublicationRuntimeControlled/publication/runtime_logs/0/at_ms",
        "/17/WorkflowPublicationRuntimeControlled/publication/updated_at_ms",
        "/19/WorkflowPublicationDeploymentBound/publication/created_at_ms",
        "/19/WorkflowPublicationDeploymentBound/publication/runtime_last_heartbeat_at_ms",
        "/19/WorkflowPublicationDeploymentBound/publication/runtime_logs/0/at_ms",
        "/19/WorkflowPublicationDeploymentBound/publication/updated_at_ms",
    ] {
        *snapshot
            .pointer_mut(path)
            .expect("timestamp path should encode") = serde_json::json!(42);
    }

    assert_eq!(
        snapshot.pointer("/0/CreateWorkflowPublication/transport/kind"),
        Some(&serde_json::json!("human_http"))
    );
    assert_eq!(
        snapshot.pointer("/5/MaterializeWorkflowPublication/runtime_key"),
        Some(&serde_json::json!("deployment-1:replica-0"))
    );
    assert_eq!(
        snapshot.pointer("/5/MaterializeWorkflowPublication/snapshot/workflow/schemas/0/id"),
        Some(&serde_json::json!("schema-1"))
    );
    assert_eq!(
        snapshot.pointer("/5/MaterializeWorkflowPublication/snapshot/workflow/schemas/0/schema/properties/answer/type"),
        Some(&serde_json::json!("string"))
    );
    assert_eq!(
        snapshot.pointer("/0/CreateWorkflowPublication/queue_ref"),
        Some(&serde_json::json!("priority"))
    );
    assert_eq!(
        snapshot.pointer("/0/CreateWorkflowPublication/expected_workflow_revision"),
        Some(&serde_json::json!(7))
    );
    assert_eq!(
        snapshot.pointer("/0/CreateWorkflowPublication/operation_key"),
        Some(&serde_json::json!("publish-operation-1"))
    );
    assert_eq!(
        snapshot.pointer("/0/CreateWorkflowPublication/kind"),
        Some(&serde_json::json!("ingress"))
    );
    assert_eq!(
        snapshot.pointer("/6/WorkflowPublicationCreated/publication/queue_ref"),
        Some(&serde_json::json!("priority"))
    );
    assert_eq!(
        snapshot.pointer("/6/WorkflowPublicationCreated/publication/kind"),
        Some(&serde_json::json!("ingress"))
    );
    assert_eq!(
        snapshot.pointer("/6/WorkflowPublicationCreated/publication/source_workflow_revision"),
        Some(&serde_json::json!(7))
    );
    assert_eq!(
        snapshot.pointer("/6/WorkflowPublicationCreated/publication/source_snapshot_digest"),
        Some(&serde_json::json!("sha256:publication-source-1"))
    );
    assert_eq!(
        snapshot.pointer("/6/WorkflowPublicationCreated/publication/creation_operation_key"),
        Some(&serde_json::json!("publish-operation-1"))
    );
    assert_eq!(
        snapshot.pointer("/0/CreateWorkflowPublication/trace_exposure/nodes/node-1/1"),
        Some(&serde_json::json!("assistant_messages"))
    );
    assert_eq!(
        snapshot.pointer("/6/WorkflowPublicationCreated/publication/trace_exposure/nodes/node-1/2"),
        Some(&serde_json::json!("tool_use"))
    );
    assert_eq!(
        snapshot.pointer("/0/CreateWorkflowPublication/sync_timeout_ms"),
        Some(&serde_json::json!(240_000))
    );
    assert_eq!(
        snapshot.pointer("/6/WorkflowPublicationCreated/publication/poll_ms"),
        Some(&serde_json::json!(250))
    );
    assert_eq!(
        snapshot.pointer("/12/RegisterWorkflowPublicationEndpoint/local_url"),
        Some(&serde_json::json!("http://127.0.0.1:3000/"))
    );
    assert_eq!(
        snapshot.pointer("/3/ExportWorkflowPublicationPackage/agent_app/routes/0/path"),
        Some(&serde_json::json!("/add/*"))
    );
    assert_eq!(
        snapshot.pointer("/9/WorkflowPublicationPackageExported/package_version"),
        Some(&serde_json::json!(2))
    );
    assert_eq!(
        snapshot.pointer("/13/WorkflowPublicationEndpointRegistered/publication/status"),
        Some(&serde_json::json!("running"))
    );
    assert_eq!(
        snapshot.pointer("/13/WorkflowPublicationEndpointRegistered/publication/open_url"),
        Some(&serde_json::json!(
            "https://relay.example.test/display/publication-1/"
        ))
    );
    assert_eq!(
        snapshot.pointer("/13/WorkflowPublicationEndpointRegistered/publication/viewer_url"),
        Some(&serde_json::json!(
            "https://relay.example.test/display/publication-1/"
        ))
    );
    assert_eq!(
        snapshot.pointer("/13/WorkflowPublicationEndpointRegistered/viewer_url"),
        Some(&serde_json::json!(
            "https://relay.example.test/display/publication-1/"
        ))
    );
    assert_eq!(
        snapshot.pointer("/14/ControlWorkflowPublicationRuntime/action"),
        Some(&serde_json::json!("start"))
    );
    assert_eq!(
        snapshot.pointer("/14/ControlWorkflowPublicationRuntime/port"),
        Some(&serde_json::json!(3000))
    );
    assert_eq!(
        snapshot.pointer("/15/WorkflowPublicationRuntimeControlled/publication/deployment/status"),
        Some(&serde_json::json!("starting"))
    );
    assert_eq!(
        snapshot.pointer("/15/WorkflowPublicationRuntimeControlled/open_url"),
        Some(&serde_json::json!("http://127.0.0.1:3000/"))
    );
    assert_eq!(
        snapshot.pointer("/15/WorkflowPublicationRuntimeControlled/viewer_url"),
        Some(&serde_json::json!("http://127.0.0.1:3000/"))
    );
    assert_eq!(
        snapshot.pointer("/16/ControlWorkflowPublicationRuntime/action"),
        Some(&serde_json::json!("inspect"))
    );
    assert_eq!(
        snapshot.pointer("/17/WorkflowPublicationRuntimeControlled/action"),
        Some(&serde_json::json!("inspect"))
    );
    assert_eq!(
        snapshot.pointer("/17/WorkflowPublicationRuntimeControlled/status"),
        Some(&serde_json::json!("running"))
    );
    assert_eq!(
        snapshot.pointer("/17/WorkflowPublicationRuntimeControlled/publication/runtime/reachable"),
        Some(&serde_json::json!(true))
    );
    assert_eq!(
        snapshot.pointer("/17/WorkflowPublicationRuntimeControlled/publication/watchdog_count"),
        Some(&serde_json::json!(1))
    );
    assert_eq!(
        snapshot.pointer("/17/WorkflowPublicationRuntimeControlled/publication/watchdogs/0/id"),
        Some(&serde_json::json!("watchdog-1"))
    );
    assert_eq!(
        snapshot.pointer("/17/WorkflowPublicationRuntimeControlled/publication/latest_run/id"),
        Some(&serde_json::json!("run-1"))
    );
    assert_eq!(
        snapshot.pointer(
            "/17/WorkflowPublicationRuntimeControlled/publication/latest_output/message/value"
        ),
        Some(&serde_json::json!(1842))
    );
    assert_eq!(
        snapshot.pointer("/18/BindWorkflowPublicationDeployment/operation_key"),
        Some(&serde_json::json!("deployment-setup:setup-1:runtime"))
    );
    assert_eq!(
        snapshot.pointer("/18/BindWorkflowPublicationDeployment/desired_revision"),
        Some(&serde_json::json!(7))
    );
    assert_eq!(
        snapshot.pointer("/18/BindWorkflowPublicationDeployment/environment_id"),
        Some(&serde_json::json!("environment-1"))
    );
    assert_eq!(
        snapshot.pointer("/18/BindWorkflowPublicationDeployment/caller_claims_public_key_pem"),
        Some(&serde_json::json!("-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEA/pMgE2dD4Y9eL57S6f9+lve+T2A4M0ueD5GmOZfHjkI=\n-----END PUBLIC KEY-----\n"))
    );
    assert_eq!(
        snapshot.pointer("/19/WorkflowPublicationDeploymentBound/tunnel_url"),
        Some(&serde_json::json!(
            "https://relay.example.test/display/publication-1/"
        ))
    );
    assert_eq!(
        snapshot.pointer("/19/WorkflowPublicationDeploymentBound/replayed"),
        Some(&serde_json::json!(false))
    );
    assert_eq!(snapshot.pointer("/0/CreateWorkflowPublication/auth"), None);
    assert_eq!(
        snapshot.pointer("/6/WorkflowPublicationCreated/publication/auth"),
        None
    );
    assert_eq!(
        snapshot.pointer("/6/WorkflowPublicationCreated/publication/pairing_codes"),
        None
    );
    assert_eq!(
        snapshot.pointer("/6/WorkflowPublicationCreated/publication/trusted_senders"),
        None
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("workflow publication shape should encode");
    assert_eq!(
        snapshot.pointer("/20/ActivateWorkflowPublicationRuntime/runtime_keys"),
        Some(&serde_json::json!(["deployment-1:replica-0"]))
    );
    assert_eq!(
        snapshot.pointer("/21/WorkflowPublicationRuntimeActivated/publication_id"),
        Some(&serde_json::json!("publication-1"))
    );
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "017fbcd592319e79f1167104b7d39dec8562e3e86ed006b252ef783081c63c96"
    );
}

#[test]
fn local_daemon_protocol_publication_invocation_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 312);

    let request =
        LocalDaemonRequest::InvokeWorkflowEndpoint(crate::local::InvokeWorkflowEndpointRequest {
            session_id: "runtime-session-1".to_string(),
            workflow_ref: "workflow-1".to_string(),
            endpoint_ref: "endpoint-1".to_string(),
            queue_ref: Some("default".to_string()),
            prompt: Some("make tea".to_string()),
            publication_invocation: Some(crate::session::WorkflowPublicationInvocationEnvelope {
                publication_id: "publication-1".to_string(),
                hook_id: Some("hook-1".to_string()),
                invocation_id: "req-1".to_string(),
                transport: "human_http".to_string(),
                endpoint_id: "endpoint-1".to_string(),
                queue_ref: Some("default".to_string()),
                input: serde_json::json!({ "prompt": "make tea" }),
                artifacts: vec![serde_json::json!({
                    "id": "artifact-1",
                    "name": "image.png",
                    "media_type": "image/png"
                })],
                mode: Some("sync".to_string()),
                caller: serde_json::json!({
                    "type": "anonymous",
                    "proof": { "transport": "human_http" }
                }),
            }),
        });
    let snapshot = serde_json::json!([request]);

    assert_eq!(
        snapshot.pointer("/0/InvokeWorkflowEndpoint/prompt"),
        Some(&serde_json::json!("make tea"))
    );
    assert_eq!(
        snapshot.pointer("/0/InvokeWorkflowEndpoint/publication_invocation/invocation_id"),
        Some(&serde_json::json!("req-1"))
    );
    assert_eq!(
        snapshot.pointer("/0/InvokeWorkflowEndpoint/publication_invocation/input/prompt"),
        Some(&serde_json::json!("make tea"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("publication invocation shape should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "b82f8e8a01b0c282bf05883bd81ae8aa21320eab0ab95a3f2dfa4059471daa67"
    );
}
