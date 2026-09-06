use super::*;

#[test]
fn local_daemon_protocol_workflow_code_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 311);

    let validate_request =
        LocalDaemonRequest::ValidateWorkflowCode(crate::local::ValidateWorkflowCodeRequest {
            session_id: "session-1".to_string(),
            node_path: "/usr/local/bin/node".to_string(),
            source: "workflow.workflow({ alias: 'toy' })".to_string(),
            language: Some(crate::workflow_code::WorkflowCodeLanguage::TypeScript),
            provider_rebindings: vec![crate::workflow_code::WorkflowCodeProviderRebinding {
                node: "planner".to_string(),
                provider: "dev-stub".to_string(),
                model: Some("default".to_string()),
                effort: None,
                account_profile: Some("profile-a".to_string()),
            }],
            agent_rebindings: vec![crate::workflow_code::WorkflowCodeAgentRebinding {
                node: "worker".to_string(),
                agent_ref: "agent-entry".to_string(),
            }],
        });
    let apply_request =
        LocalDaemonRequest::ApplyWorkflowCode(crate::local::ApplyWorkflowCodeRequest {
            session_id: "session-1".to_string(),
            node_path: "/usr/local/bin/node".to_string(),
            source: "workflow.workflow({ alias: 'toy' })".to_string(),
            language: None,
            provider_rebindings: vec![crate::workflow_code::WorkflowCodeProviderRebinding {
                node: "planner".to_string(),
                provider: "opencode".to_string(),
                model: Some("qwen3-coder".to_string()),
                effort: None,
                account_profile: None,
            }],
            agent_rebindings: Vec::new(),
        });
    let run_request = LocalDaemonRequest::RunWorkflowCode(crate::local::RunWorkflowCodeRequest {
        session_id: "session-1".to_string(),
        node_path: "/usr/local/bin/node".to_string(),
        source: "workflow.workflow({ alias: 'toy' })".to_string(),
        language: None,
        provider_rebindings: vec![crate::workflow_code::WorkflowCodeProviderRebinding {
            node: "planner".to_string(),
            provider: "codex".to_string(),
            model: Some("gpt-5".to_string()),
            effort: Some("medium".to_string()),
            account_profile: Some("work".to_string()),
        }],
        agent_rebindings: vec![crate::workflow_code::WorkflowCodeAgentRebinding {
            node: "worker".to_string(),
            agent_ref: "agent-entry".to_string(),
        }],
        endpoint: Some("entry".to_string()),
        queue_ref: Some("default".to_string()),
        prompt: "Run this scripted workflow.".to_string(),
    });
    let create_artifact_request = LocalDaemonRequest::CreateWorkflowCodeArtifact(
        crate::local::CreateWorkflowCodeArtifactRequest {
            session_id: "session-1".to_string(),
            name: "toy-flow".to_string(),
            language: crate::workflow_code::WorkflowCodeLanguage::JavaScript,
            node_path: "/usr/local/bin/node".to_string(),
            source: "workflow.workflow({ alias: 'toy' })".to_string(),
        },
    );
    let update_artifact_request = LocalDaemonRequest::UpdateWorkflowCodeArtifact(
        crate::local::UpdateWorkflowCodeArtifactRequest {
            session_id: "session-1".to_string(),
            name: "toy-flow".to_string(),
            language: crate::workflow_code::WorkflowCodeLanguage::TypeScript,
            node_path: "/usr/local/bin/node".to_string(),
            source: "workflow.workflow({ alias: 'toy2' })".to_string(),
        },
    );
    let get_artifact_request =
        LocalDaemonRequest::GetWorkflowCodeArtifact(crate::local::GetWorkflowCodeArtifactRequest {
            session_id: "session-1".to_string(),
            name: "toy-flow".to_string(),
        });
    let list_artifacts_request = LocalDaemonRequest::ListWorkflowCodeArtifacts(
        crate::local::ListWorkflowCodeArtifactsRequest {
            session_id: "session-1".to_string(),
        },
    );
    let delete_artifact_request = LocalDaemonRequest::DeleteWorkflowCodeArtifact(
        crate::local::DeleteWorkflowCodeArtifactRequest {
            session_id: "session-1".to_string(),
            name: "toy-flow".to_string(),
        },
    );
    let export_artifact_request = LocalDaemonRequest::ExportWorkflowCodeArtifact(
        crate::local::ExportWorkflowCodeArtifactRequest {
            session_id: "session-1".to_string(),
            name: "toy-flow".to_string(),
        },
    );

    let definition = crate::workflow_code::WorkflowCodeDefinition {
        schema_version: crate::workflow_code::WORKFLOW_CODE_SCHEMA_VERSION,
        parameters_schema: None,
        workflow: crate::workflow_code::WorkflowCodeWorkflow {
            alias: Some("toy".to_string()),
            prompt: Some("Run this scripted workflow.".to_string()),
            flush_agent_context_before_run: Some(true),
            max_concurrent: Some(32),
            run_output_schema: Some("final".to_string()),
        },
        schemas: vec![crate::workflow_code::WorkflowCodeSchemaDefinition {
            handle: "final".to_string(),
            alias: Some("Final".to_string()),
            description: None,
            schema: serde_json::json!({"type": "object"}),
        }],
        nodes: vec![crate::workflow_code::WorkflowCodeNodeDefinition {
            handle: "planner".to_string(),
            agent: crate::workflow_code::WorkflowCodeAgentBinding::Create(
                crate::workflow_code::WorkflowCodeAgentCreate {
                    alias: Some("planner".to_string()),
                    provider: "codex".to_string(),
                    model: Some("gpt-5".to_string()),
                    effort: None,
                    account_profile: None,
                },
            ),
            public_label: Some("Planner".to_string()),
            instructions: Some("Plan then hand off.".to_string()),
            can_complete_workflow_run: Some(false),
            can_emit_intermediate_run_output: Some(true),
            wait_for_all_inputs: Some(false),
            intermediate_output_schema: None,
            max_turns: Some(3),
            extensions: Vec::new(),
            canvas: Some(crate::workflow_code::WorkflowCodeCanvasPoint { x: 10, y: 20 }),
        }],
        edges: Vec::new(),
        endpoints: vec![crate::workflow_code::WorkflowCodeEndpointDefinition {
            handle: "entry".to_string(),
            entry_node: "planner".to_string(),
            alias: Some("entry".to_string()),
            max_instances: None,
            canvas: None,
        }],
        queues: Vec::new(),
        schedules: vec![crate::workflow_code::WorkflowCodeScheduleDefinition {
            handle: "watchdog-1".to_string(),
            endpoint: "entry".to_string(),
            queue: None,
            enabled: Some(false),
            trigger: crate::session::WorkflowScheduleTrigger::interval(60),
            invocation_prompt: "Check for scripted work.".to_string(),
            overlap_policy: crate::session::WorkflowScheduleOverlapPolicy::Queue,
            max_runs: Some(3),
        }],
    };
    let compile = crate::workflow_code::WorkflowCodeCompileResult {
        definition: definition.clone(),
        validation: crate::workflow_code::WorkflowCodeValidationReport {
            ok: false,
            diagnostics: vec![crate::workflow_code::WorkflowCodeValidationDiagnostic {
                severity: crate::workflow_code::WorkflowCodeValidationSeverity::Error,
                code: "invalid_max_turns".to_string(),
                message: "node max_turns must not be zero".to_string(),
                handle: Some("planner".to_string()),
                source_span: Some(crate::workflow_code::WorkflowCodeSourceSpan {
                    start_line: 7,
                    start_column: 3,
                    end_line: 7,
                    end_column: 3,
                }),
            }],
        },
        logs: "compiled".to_string(),
        source_spans: BTreeMap::new(),
    };
    let validate_response = LocalDaemonResponse::WorkflowCodeValidated {
        result: compile.clone(),
    };
    let apply_result = crate::workflow_code::WorkflowCodeCompileAndApplyResult {
        compile,
        apply: crate::workflow_code::WorkflowCodeApplyReport {
            workflow_id: "workflow-1".to_string(),
            schema_refs: BTreeMap::from([("final".to_string(), "final".to_string())]),
            node_ids: BTreeMap::from([("planner".to_string(), "node-1".to_string())]),
            agent_ids: BTreeMap::from([("planner".to_string(), "agent-1".to_string())]),
            edge_ids: BTreeMap::new(),
            endpoint_ids: BTreeMap::from([("entry".to_string(), "endpoint-1".to_string())]),
            queue_ids: BTreeMap::from([(
                "default".to_string(),
                "workflow-1:default".to_string(),
            )]),
            schedule_ids: BTreeMap::from([(
                "watchdog-1".to_string(),
                "watchdog-1".to_string(),
            )]),
            canvas_layout_applied: true,
            warnings: vec![crate::workflow_code::WorkflowCodeApplyWarning {
                code: "default_queue_created".to_string(),
                message: "workflow-code omitted queues; the kernel used the workflow default prompt queue".to_string(),
                handle: Some("default".to_string()),
            }],
        },
    };
    let apply_response = LocalDaemonResponse::WorkflowCodeApplied {
        result: apply_result.clone(),
        session: crate::session::RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        ),
    };
    let mut run_workflow =
        crate::session::WorkflowDefinition::new("workflow-1", Some("toy".to_string()));
    run_workflow.add_node(crate::session::WorkflowNodeDefinition::new(
        "node-1", "agent-1",
    ));
    let run_endpoint = crate::session::WorkflowEndpointDefinition::new(
        "endpoint-1",
        Some("entry".to_string()),
        "node-1",
    );
    run_workflow.add_endpoint(run_endpoint.clone());
    let run_response = LocalDaemonResponse::WorkflowCodeRun {
        result: crate::workflow_code::WorkflowCodeRunResult {
            apply: apply_result.clone(),
            invocation: crate::workflow_code::WorkflowCodeRunInvocation::Started {
                workflow_run: Box::new(crate::session::WorkflowRun::new(
                    "workflow-run-1",
                    "workflow-1",
                    "endpoint-1",
                    "node-1",
                    Some("Run this scripted workflow.".to_string()),
                    None,
                    Vec::new(),
                    Vec::new(),
                )),
                workflow: run_workflow,
                endpoint: run_endpoint,
            },
        },
        session: crate::session::RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        ),
    };
    let artifact_metadata = crate::workflow_code::WorkflowCodeArtifactMetadata {
        name: "toy-flow".to_string(),
        language: crate::workflow_code::WorkflowCodeLanguage::JavaScript,
        path: std::path::PathBuf::from("/workspace/.chariox/workflow-code/toy-flow.json"),
        source_sha256: "sha256".to_string(),
        source_bytes: 37,
        validation: crate::workflow_code::WorkflowCodeValidationReport {
            ok: true,
            diagnostics: Vec::new(),
        },
        provenance: crate::workflow_code::WorkflowCodeArtifactProvenance {
            created_by: crate::workflow_code::WorkflowCodeArtifactActor {
                user_id: "user-1".to_string(),
                metaagent_id: Some("meta-1".to_string()),
            },
            updated_by: crate::workflow_code::WorkflowCodeArtifactActor {
                user_id: "user-1".to_string(),
                metaagent_id: Some("meta-1".to_string()),
            },
        },
        history: vec![crate::workflow_code::WorkflowCodeArtifactHistoryEntry {
            action: crate::workflow_code::WorkflowCodeArtifactHistoryAction::Created,
            at_ms: 1_000,
            actor: crate::workflow_code::WorkflowCodeArtifactActor {
                user_id: "user-1".to_string(),
                metaagent_id: Some("meta-1".to_string()),
            },
            source_sha256: "sha256".to_string(),
            validation_ok: Some(true),
            workflow_id: None,
            warnings: Vec::new(),
        }],
        created_at_ms: 1_000,
        updated_at_ms: 2_000,
    };
    let artifact = crate::workflow_code::WorkflowCodeArtifact {
        metadata: artifact_metadata.clone(),
        source: "workflow.workflow({ alias: 'toy' })".to_string(),
        definition,
    };
    let package = crate::workflow_code::WorkflowCodeArtifactPackage {
        package_version: crate::workflow_code::WORKFLOW_CODE_ARTIFACT_PACKAGE_VERSION,
        name: "toy-flow".to_string(),
        language: crate::workflow_code::WorkflowCodeLanguage::JavaScript,
        source: "workflow.workflow({ alias: 'toy' })".to_string(),
        source_sha256: "sha256".to_string(),
        source_bytes: 37,
        definition_sha256: crate::workflow_code::workflow_code_definition_sha256_hex(
            &artifact.definition,
        ),
        definition: artifact.definition.clone(),
        validation: crate::workflow_code::WorkflowCodeValidationReport {
            ok: true,
            diagnostics: Vec::new(),
        },
        exported_at_ms: 3_000,
    };
    let import_artifact_request = LocalDaemonRequest::ImportWorkflowCodeArtifact(
        crate::local::ImportWorkflowCodeArtifactRequest {
            session_id: "session-1".to_string(),
            package: package.clone(),
            name: Some("imported-toy-flow".to_string()),
            overwrite: true,
            node_path: "/usr/local/bin/node".to_string(),
        },
    );
    let apply_artifact_request = LocalDaemonRequest::ApplyWorkflowCodeArtifact(
        crate::local::ApplyWorkflowCodeArtifactRequest {
            session_id: "session-1".to_string(),
            name: "imported-toy-flow".to_string(),
            provider_rebindings: vec![crate::workflow_code::WorkflowCodeProviderRebinding {
                node: "planner".to_string(),
                provider: "dev-stub".to_string(),
                model: Some("default".to_string()),
                effort: None,
                account_profile: None,
            }],
            agent_rebindings: vec![crate::workflow_code::WorkflowCodeAgentRebinding {
                node: "worker".to_string(),
                agent_ref: "agent-entry".to_string(),
            }],
        },
    );
    let run_artifact_request =
        LocalDaemonRequest::RunWorkflowCodeArtifact(crate::local::RunWorkflowCodeArtifactRequest {
            session_id: "session-1".to_string(),
            name: "imported-toy-flow".to_string(),
            provider_rebindings: Vec::new(),
            agent_rebindings: Vec::new(),
            endpoint: Some("entry".to_string()),
            queue_ref: Some("default".to_string()),
            prompt: "Run this saved workflow-code artifact.".to_string(),
        });
    let artifact_created_response = LocalDaemonResponse::WorkflowCodeArtifactCreated {
        artifact: artifact.clone(),
    };
    let artifact_updated_response = LocalDaemonResponse::WorkflowCodeArtifactUpdated {
        artifact: artifact.clone(),
    };
    let artifact_get_response = LocalDaemonResponse::WorkflowCodeArtifact {
        artifact: artifact.clone(),
    };
    let artifacts_listed_response = LocalDaemonResponse::WorkflowCodeArtifactsListed {
        artifacts: vec![artifact_metadata],
    };
    let artifact_deleted_response = LocalDaemonResponse::WorkflowCodeArtifactDeleted {
        name: "toy-flow".to_string(),
        path: std::path::PathBuf::from("/workspace/.chariox/workflow-code/toy-flow.json"),
    };
    let artifact_exported_response = LocalDaemonResponse::WorkflowCodeArtifactExported {
        package: package.clone(),
    };
    let artifact_imported_response = LocalDaemonResponse::WorkflowCodeArtifactImported {
        artifact: artifact.clone(),
    };
    let package_export_request = LocalDaemonRequest::ExportWorkflowCodePackage(
        crate::local::ExportWorkflowCodePackageRequest {
            session_id: "session-1".to_string(),
            name: "toy-flow".to_string(),
            target: Some(crate::local::WorkflowCodePackageExportTarget::Workflow {
                workflow_ref: "workflow-1".to_string(),
            }),
            agent_mode: crate::workflow_code::WorkflowCodeSourceExportAgentMode::PortableGenerated,
        },
    );
    let package_import_request = LocalDaemonRequest::ImportWorkflowCodePackage(
        crate::local::ImportWorkflowCodePackageRequest {
            session_id: "session-1".to_string(),
            package: package.clone(),
            name: Some("package-toy-flow".to_string()),
            overwrite: true,
            node_path: "/usr/local/bin/node".to_string(),
        },
    );
    let source_export_request = LocalDaemonRequest::ExportWorkflowCodeSource(
        crate::local::ExportWorkflowCodeSourceRequest {
            session_id: "session-1".to_string(),
            target: crate::local::WorkflowCodeSourceExportTarget::Artifact {
                name: "toy-flow".to_string(),
            },
            format: crate::workflow_code::WorkflowCodeSourceExportFormat::Directory,
            agent_mode: crate::workflow_code::WorkflowCodeSourceExportAgentMode::PortableGenerated,
        },
    );
    let workflow_source_export_request = LocalDaemonRequest::ExportWorkflowCodeSource(
        crate::local::ExportWorkflowCodeSourceRequest {
            session_id: "session-1".to_string(),
            target: crate::local::WorkflowCodeSourceExportTarget::Workflow {
                workflow_ref: "workflow-1".to_string(),
            },
            format: crate::workflow_code::WorkflowCodeSourceExportFormat::Inline,
            agent_mode: crate::workflow_code::WorkflowCodeSourceExportAgentMode::ExistingAgents,
        },
    );
    let registry_list_request =
        LocalDaemonRequest::ListWorkflowRegistry(crate::local::ListWorkflowRegistryRequest {
            session_id: "session-1".to_string(),
        });
    let registry_get_request = LocalDaemonRequest::GetWorkflowRegistryEntry(
        crate::local::GetWorkflowRegistryEntryRequest {
            session_id: "session-1".to_string(),
            name: "toy-flow".to_string(),
        },
    );
    let registry_add_request = LocalDaemonRequest::AddWorkflowRegistryEntry(
        crate::local::AddWorkflowRegistryEntryRequest {
            session_id: "session-1".to_string(),
            name: "toy-flow".to_string(),
            scope: Some(crate::workflow_code::WorkflowRegistrySourceScope::Workspace),
            source: crate::workflow_code::WorkflowRegistrySourceInput::SourceDirectory {
                files: vec![
                    crate::workflow_code::WorkflowCodeSourceExportFile {
                        path: "workflow.js".to_string(),
                        contents: "workflow.workflow({ alias: 'toy' })".to_string(),
                        sha256: "source-sha256".to_string(),
                    },
                    crate::workflow_code::WorkflowCodeSourceExportFile {
                        path: "schemas/final.json".to_string(),
                        contents: "{\n  \"type\": \"object\"\n}\n".to_string(),
                        sha256: "schema-sha256".to_string(),
                    },
                ],
            },
            node_path: "/usr/local/bin/node".to_string(),
        },
    );
    let registry_add_from_workflow_request =
        LocalDaemonRequest::AddWorkflowRegistryEntryFromWorkflow(
            crate::local::AddWorkflowRegistryEntryFromWorkflowRequest {
                session_id: "session-1".to_string(),
                name: "copied-toy-flow".to_string(),
                workflow_ref: "workflow-1".to_string(),
                scope: Some(crate::workflow_code::WorkflowRegistrySourceScope::User),
                agent_mode: crate::workflow_code::WorkflowCodeSourceExportAgentMode::ExistingAgents,
            },
        );
    let registry_delete_request = LocalDaemonRequest::DeleteWorkflowRegistryEntry(
        crate::local::DeleteWorkflowRegistryEntryRequest {
            session_id: "session-1".to_string(),
            name: "toy-flow".to_string(),
            scope: Some(crate::workflow_code::WorkflowRegistrySourceScope::Workspace),
        },
    );
    let registry_load_request = LocalDaemonRequest::LoadWorkflowRegistryEntry(
        crate::local::LoadWorkflowRegistryEntryRequest {
            session_id: "session-1".to_string(),
            name: "toy-flow".to_string(),
            parameters: std::collections::BTreeMap::from([(
                "worker_count".to_string(),
                serde_json::json!(3),
            )]),
            provider_rebindings: vec![crate::workflow_code::WorkflowCodeProviderRebinding {
                node: "planner".to_string(),
                provider: "dev-stub".to_string(),
                model: Some("default".to_string()),
                effort: None,
                account_profile: None,
            }],
            agent_rebindings: vec![crate::workflow_code::WorkflowCodeAgentRebinding {
                node: "worker".to_string(),
                agent_ref: "agent-entry".to_string(),
            }],
        },
    );
    let registry_run_request = LocalDaemonRequest::RunWorkflowRegistryEntry(
        crate::local::RunWorkflowRegistryEntryRequest {
            session_id: "session-1".to_string(),
            name: "toy-flow".to_string(),
            parameters: std::collections::BTreeMap::from([(
                "worker_count".to_string(),
                serde_json::json!(3),
            )]),
            provider_rebindings: Vec::new(),
            agent_rebindings: vec![crate::workflow_code::WorkflowCodeAgentRebinding {
                node: "worker".to_string(),
                agent_ref: "agent-entry".to_string(),
            }],
            endpoint: Some("entry".to_string()),
            queue_ref: Some("default".to_string()),
            prompt: "Run this registered workflow.".to_string(),
        },
    );
    let package_exported_response = LocalDaemonResponse::WorkflowCodePackageExported {
        package: package.clone(),
    };
    let package_imported_response = LocalDaemonResponse::WorkflowCodePackageImported { artifact };
    let source_exported_response = LocalDaemonResponse::WorkflowCodeSourceExported {
        export: crate::workflow_code::WorkflowCodeSourceExport {
            name: "toy-flow".to_string(),
            language: crate::workflow_code::WorkflowCodeLanguage::JavaScript,
            format: crate::workflow_code::WorkflowCodeSourceExportFormat::Directory,
            source_path: "workflow.js".to_string(),
            source: "async function defineWorkflow(workflow) {}\n".to_string(),
            source_sha256: "source-sha256".to_string(),
            source_bytes: 49,
            definition_sha256: package.definition_sha256.clone(),
            files: vec![crate::workflow_code::WorkflowCodeSourceExportFile {
                path: "schemas/final.json".to_string(),
                contents: "{\n  \"type\": \"object\"\n}\n".to_string(),
                sha256: "schema-sha256".to_string(),
            }],
        },
    };
    let registry_entry = crate::workflow_code::WorkflowRegistryEntryMetadata {
        name: "toy-flow".to_string(),
        source_scope: crate::workflow_code::WorkflowRegistrySourceScope::Workspace,
        source_kind: crate::workflow_code::WorkflowRegistrySourceKind::SourceDirectory,
        source_path: "workflow.js".to_string(),
        source_sha256: "source-sha256".to_string(),
        source_bytes: 37,
        definition_sha256: Some(package.definition_sha256.clone()),
        created_at_ms: 1_000,
        updated_at_ms: 2_000,
        validation: crate::workflow_code::WorkflowRegistryValidationSummary {
            ok: true,
            diagnostics: Vec::new(),
        },
        summary: Some(crate::workflow_code::WorkflowRegistryEntrySummary {
            endpoints: vec!["entry".to_string(), "review".to_string()],
            queues: vec!["urgent".to_string()],
            nodes: vec!["planner".to_string(), "reviewer".to_string()],
            default_endpoint: Some("entry".to_string()),
        }),
        parameters_schema: Some(serde_json::json!({
            "type": "object",
            "properties": {
                "worker_count": {
                    "type": "integer",
                    "minimum": 1,
                    "default": 3
                }
            },
            "additionalProperties": false
        })),
    };
    let registry_listed_response = LocalDaemonResponse::WorkflowRegistryListed {
        entries: vec![registry_entry.clone()],
    };
    let registry_entry_response = LocalDaemonResponse::WorkflowRegistryEntry {
        entry: registry_entry.clone(),
    };
    let registry_entry_added_response = LocalDaemonResponse::WorkflowRegistryEntryAdded {
        entry: registry_entry.clone(),
    };
    let registry_entry_deleted_response = LocalDaemonResponse::WorkflowRegistryEntryDeleted {
        name: "toy-flow".to_string(),
        path: std::path::PathBuf::from("/workspace/.chariox/workflows/toy-flow"),
    };
    let registry_entry_loaded_response = LocalDaemonResponse::WorkflowRegistryEntryLoaded {
        entry: registry_entry.clone(),
        result: apply_result.clone(),
        session: crate::session::RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        ),
    };
    let registry_entry_run_response = LocalDaemonResponse::WorkflowRegistryEntryRun {
        entry: registry_entry,
        result: crate::workflow_code::WorkflowCodeRunResult {
            apply: apply_result,
            invocation: crate::workflow_code::WorkflowCodeRunInvocation::Enqueued {
                queued_prompt: Box::new(crate::session::WorkflowQueuedPrompt::new(
                    crate::session::WorkflowQueuedPromptInput {
                        id: "queue-1".to_string(),
                        queue_id: "default".to_string(),
                        workflow_id: "workflow-1".to_string(),
                        endpoint_id: "endpoint-1".to_string(),
                        prompt: Some("Run this registered workflow.".to_string()),
                        publication_invocation: None,
                        source: crate::session::WorkflowQueuedPromptSource::Manual,
                        schedule_id: None,
                    },
                )),
                workflow: crate::session::WorkflowDefinition::new(
                    "workflow-1",
                    Some("toy".to_string()),
                ),
                endpoint: crate::session::WorkflowEndpointDefinition::new(
                    "endpoint-1",
                    Some("entry".to_string()),
                    "node-1",
                ),
            },
        },
        session: crate::session::RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        ),
    };

    let bind_source_request =
        LocalDaemonRequest::BindWorkflowCodeSource(crate::local::BindWorkflowCodeSourceRequest {
            session_id: "session-1".to_string(),
            workflow_ref: "workflow-1".to_string(),
            artifact_name: "workflow-source-session-1-workflow-1".to_string(),
            origin: crate::session::WorkflowCodeSourceOrigin::Generated,
            expected_workflow_revision: Some(7),
        });
    let mut bound_workflow =
        crate::session::WorkflowDefinition::new("workflow-1", Some("toy".to_string()));
    bound_workflow.bind_code_source(
        "workflow-source-session-1-workflow-1".to_string(),
        crate::workflow_code::WorkflowCodeLanguage::JavaScript,
        "sha256".to_string(),
        crate::session::WorkflowCodeSourceOrigin::Generated,
        crate::workflow_code::WorkflowCodeApplyReport::for_workflow("workflow-1"),
    );
    let bind_source_response = LocalDaemonResponse::WorkflowCodeSourceBound {
        workflow: bound_workflow,
        session: crate::session::RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        ),
    };
    let rebuild_preview_request = LocalDaemonRequest::RebuildWorkflowCodeSource(
        crate::local::RebuildWorkflowCodeSourceRequest {
            session_id: "session-1".to_string(),
            workflow_ref: "workflow-1".to_string(),
            expected_workflow_revision: 8,
            confirm: false,
        },
    );
    let rebuild_preview = crate::workflow_code::WorkflowCodeRebuildPreview {
        workflow_id: "workflow-1".to_string(),
        current_workflow_revision: 8,
        source_workflow_revision: 7,
        source_sha256: "sha256".to_string(),
        diverged: true,
        restored_schemas: 1,
        restored_nodes: 2,
        restored_edges: 1,
        restored_endpoints: 1,
        restored_queues: 1,
        restored_schedules: 1,
        changes: vec![crate::workflow_code::WorkflowCodeStructuralChange {
            resource: "nodes".to_string(),
            current_count: 3,
            source_count: 2,
            restore_missing: 0,
            remove_visual_only: 1,
            replace_existing: 2,
        }],
    };
    let rebuild_preview_response = LocalDaemonResponse::WorkflowCodeRebuildPreview {
        preview: rebuild_preview.clone(),
    };
    let rebuild_confirm_request = LocalDaemonRequest::RebuildWorkflowCodeSource(
        crate::local::RebuildWorkflowCodeSourceRequest {
            session_id: "session-1".to_string(),
            workflow_ref: "workflow-1".to_string(),
            expected_workflow_revision: 8,
            confirm: true,
        },
    );
    let rebuild_response = LocalDaemonResponse::WorkflowCodeSourceRebuilt {
        preview: rebuild_preview,
        result: crate::workflow_code::WorkflowCodeApplyReport::for_workflow("workflow-1"),
        session: crate::session::RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        ),
    };
    let update_source_preview_request = LocalDaemonRequest::UpdateWorkflowCodeSourceFromWorkflow(
        crate::local::UpdateWorkflowCodeSourceFromWorkflowRequest {
            session_id: "session-1".to_string(),
            workflow_ref: "workflow-1".to_string(),
            expected_workflow_revision: 9,
            expected_generated_source_sha256: None,
            confirm: false,
        },
    );
    let update_source_preview = crate::workflow_code::WorkflowCodeSourceUpdatePreview {
        workflow_id: "workflow-1".to_string(),
        workflow_revision: 9,
        previous_source_sha256: "old-sha256".to_string(),
        generated_source_sha256: "new-sha256".to_string(),
        changed: true,
        previous_line_count: 10,
        generated_line_count: 12,
        added_lines: 3,
        removed_lines: 1,
        generated_source: "export default workflow({});\n".to_string(),
    };
    let update_source_preview_response = LocalDaemonResponse::WorkflowCodeSourceUpdatePreview {
        preview: update_source_preview.clone(),
    };
    let update_source_confirm_request = LocalDaemonRequest::UpdateWorkflowCodeSourceFromWorkflow(
        crate::local::UpdateWorkflowCodeSourceFromWorkflowRequest {
            session_id: "session-1".to_string(),
            workflow_ref: "workflow-1".to_string(),
            expected_workflow_revision: 9,
            expected_generated_source_sha256: Some("new-sha256".to_string()),
            confirm: true,
        },
    );
    let update_source_response = LocalDaemonResponse::WorkflowCodeSourceUpdated {
        preview: update_source_preview,
        workflow: crate::session::WorkflowDefinition::new("workflow-1", Some("toy".to_string())),
        session: crate::session::RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        ),
    };

    let snapshot = serde_json::json!([
        validate_request,
        apply_request,
        run_request,
        create_artifact_request,
        update_artifact_request,
        get_artifact_request,
        list_artifacts_request,
        delete_artifact_request,
        export_artifact_request,
        import_artifact_request,
        validate_response,
        apply_response,
        run_response,
        artifact_created_response,
        artifact_updated_response,
        artifact_get_response,
        artifacts_listed_response,
        artifact_deleted_response,
        artifact_exported_response,
        artifact_imported_response,
        apply_artifact_request,
        run_artifact_request,
        package_export_request,
        package_import_request,
        source_export_request,
        workflow_source_export_request,
        package_exported_response,
        package_imported_response,
        source_exported_response,
        registry_list_request,
        registry_get_request,
        registry_add_request,
        registry_add_from_workflow_request,
        registry_delete_request,
        registry_load_request,
        registry_run_request,
        registry_listed_response,
        registry_entry_response,
        registry_entry_added_response,
        registry_entry_deleted_response,
        registry_entry_loaded_response,
        registry_entry_run_response,
        bind_source_request,
        bind_source_response,
        rebuild_preview_request,
        rebuild_preview_response,
        rebuild_confirm_request,
        rebuild_response,
        update_source_preview_request,
        update_source_preview_response,
        update_source_confirm_request,
        update_source_response
    ]);
    assert_eq!(
        snapshot.pointer("/0/ValidateWorkflowCode/session_id"),
        Some(&serde_json::json!("session-1"))
    );
    assert_eq!(
        snapshot.pointer("/0/ValidateWorkflowCode/language"),
        Some(&serde_json::json!("typescript"))
    );
    assert_eq!(
        snapshot.pointer("/0/ValidateWorkflowCode/provider_rebindings/0/provider"),
        Some(&serde_json::json!("dev-stub"))
    );
    assert_eq!(
        snapshot.pointer("/1/ApplyWorkflowCode/session_id"),
        Some(&serde_json::json!("session-1"))
    );
    assert_eq!(
        snapshot.pointer("/1/ApplyWorkflowCode/provider_rebindings/0/provider"),
        Some(&serde_json::json!("opencode"))
    );
    assert_eq!(
        snapshot.pointer("/2/RunWorkflowCode/prompt"),
        Some(&serde_json::json!("Run this scripted workflow."))
    );
    assert_eq!(
        snapshot.pointer("/2/RunWorkflowCode/provider_rebindings/0/account_profile"),
        Some(&serde_json::json!("work"))
    );
    assert_eq!(
        snapshot.pointer("/3/CreateWorkflowCodeArtifact/language"),
        Some(&serde_json::json!("java_script"))
    );
    assert_eq!(
        snapshot.pointer("/4/UpdateWorkflowCodeArtifact/name"),
        Some(&serde_json::json!("toy-flow"))
    );
    assert_eq!(
        snapshot.pointer("/4/UpdateWorkflowCodeArtifact/language"),
        Some(&serde_json::json!("typescript"))
    );
    assert_eq!(
        snapshot.pointer("/5/GetWorkflowCodeArtifact/name"),
        Some(&serde_json::json!("toy-flow"))
    );
    assert_eq!(
        snapshot.pointer("/6/ListWorkflowCodeArtifacts/session_id"),
        Some(&serde_json::json!("session-1"))
    );
    assert_eq!(
        snapshot.pointer("/7/DeleteWorkflowCodeArtifact/name"),
        Some(&serde_json::json!("toy-flow"))
    );
    assert_eq!(
        snapshot.pointer("/8/ExportWorkflowCodeArtifact/name"),
        Some(&serde_json::json!("toy-flow"))
    );
    assert_eq!(
        snapshot.pointer("/9/ImportWorkflowCodeArtifact/name"),
        Some(&serde_json::json!("imported-toy-flow"))
    );
    assert_eq!(
        snapshot.pointer("/9/ImportWorkflowCodeArtifact/package/package_version"),
        Some(&serde_json::json!(2))
    );
    assert!(snapshot
        .pointer("/9/ImportWorkflowCodeArtifact/package/definition_sha256")
        .and_then(|value| value.as_str())
        .is_some_and(|value| !value.is_empty()));
    assert_eq!(
        snapshot.pointer("/10/WorkflowCodeValidated/result/validation/ok"),
        Some(&serde_json::json!(false))
    );
    assert_eq!(
        snapshot.pointer(
            "/10/WorkflowCodeValidated/result/validation/diagnostics/0/source_span/start_line"
        ),
        Some(&serde_json::json!(7))
    );
    assert_eq!(
        snapshot.pointer("/10/WorkflowCodeValidated/result/definition/workflow/max_concurrent"),
        Some(&serde_json::json!(32))
    );
    assert_eq!(
        snapshot.pointer("/10/WorkflowCodeValidated/result/definition/workflow/prompt"),
        Some(&serde_json::json!("Run this scripted workflow."))
    );
    assert_eq!(
        snapshot.pointer("/11/WorkflowCodeApplied/result/apply/node_ids/planner"),
        Some(&serde_json::json!("node-1"))
    );
    assert_eq!(
        snapshot.pointer("/11/WorkflowCodeApplied/result/apply/warnings/0/code"),
        Some(&serde_json::json!("default_queue_created"))
    );
    assert_eq!(
        snapshot.pointer("/11/WorkflowCodeApplied/session/id"),
        Some(&serde_json::json!("session-1"))
    );
    assert_eq!(
        snapshot.pointer("/12/WorkflowCodeRun/result/invocation/kind"),
        Some(&serde_json::json!("started"))
    );
    assert_eq!(
        snapshot.pointer("/12/WorkflowCodeRun/result/invocation/workflow_run/id"),
        Some(&serde_json::json!("workflow-run-1"))
    );
    assert_eq!(
        snapshot.pointer("/12/WorkflowCodeRun/result/invocation/workflow/max_concurrent"),
        Some(&serde_json::json!(32))
    );
    assert_eq!(
        snapshot.pointer("/12/WorkflowCodeRun/result/apply/apply/endpoint_ids/entry"),
        Some(&serde_json::json!("endpoint-1"))
    );
    assert_eq!(
        snapshot.pointer("/13/WorkflowCodeArtifactCreated/artifact/metadata/name"),
        Some(&serde_json::json!("toy-flow"))
    );
    assert_eq!(
        snapshot.pointer(
            "/13/WorkflowCodeArtifactCreated/artifact/metadata/provenance/created_by/metaagent_id"
        ),
        Some(&serde_json::json!("meta-1"))
    );
    assert_eq!(
        snapshot.pointer("/13/WorkflowCodeArtifactCreated/artifact/metadata/history/0/action"),
        Some(&serde_json::json!("created"))
    );
    assert_eq!(
        snapshot.pointer("/14/WorkflowCodeArtifactUpdated/artifact/source"),
        Some(&serde_json::json!("workflow.workflow({ alias: 'toy' })"))
    );
    assert_eq!(
        snapshot.pointer("/15/WorkflowCodeArtifact/artifact/definition/workflow/alias"),
        Some(&serde_json::json!("toy"))
    );
    assert_eq!(
        snapshot.pointer("/16/WorkflowCodeArtifactsListed/artifacts/0/source_bytes"),
        Some(&serde_json::json!(37))
    );
    assert_eq!(
        snapshot.pointer("/17/WorkflowCodeArtifactDeleted/path"),
        Some(&serde_json::json!(
            "/workspace/.chariox/workflow-code/toy-flow.json"
        ))
    );
    assert_eq!(
        snapshot.pointer("/18/WorkflowCodeArtifactExported/package/source_bytes"),
        Some(&serde_json::json!(37))
    );
    assert_eq!(
        snapshot.pointer("/19/WorkflowCodeArtifactImported/artifact/metadata/name"),
        Some(&serde_json::json!("toy-flow"))
    );
    assert_eq!(
        snapshot.pointer("/20/ApplyWorkflowCodeArtifact/provider_rebindings/0/provider"),
        Some(&serde_json::json!("dev-stub"))
    );
    assert_eq!(
        snapshot.pointer("/21/RunWorkflowCodeArtifact/prompt"),
        Some(&serde_json::json!("Run this saved workflow-code artifact."))
    );
    assert_eq!(
        snapshot.pointer("/22/ExportWorkflowCodePackage/name"),
        Some(&serde_json::json!("toy-flow"))
    );
    assert_eq!(
        snapshot.pointer("/22/ExportWorkflowCodePackage/target/kind"),
        Some(&serde_json::json!("workflow"))
    );
    assert_eq!(
        snapshot.pointer("/22/ExportWorkflowCodePackage/target/workflow_ref"),
        Some(&serde_json::json!("workflow-1"))
    );
    assert_eq!(
        snapshot.pointer("/22/ExportWorkflowCodePackage/agent_mode"),
        Some(&serde_json::json!("portable_generated"))
    );
    assert_eq!(
        snapshot.pointer("/23/ImportWorkflowCodePackage/name"),
        Some(&serde_json::json!("package-toy-flow"))
    );
    assert_eq!(
        snapshot.pointer("/24/ExportWorkflowCodeSource/target/kind"),
        Some(&serde_json::json!("artifact"))
    );
    assert_eq!(
        snapshot.pointer("/24/ExportWorkflowCodeSource/format"),
        Some(&serde_json::json!("directory"))
    );
    assert_eq!(
        snapshot.pointer("/24/ExportWorkflowCodeSource/agent_mode"),
        Some(&serde_json::json!("portable_generated"))
    );
    assert_eq!(
        snapshot.pointer("/25/ExportWorkflowCodeSource/target/workflow_ref"),
        Some(&serde_json::json!("workflow-1"))
    );
    assert_eq!(
        snapshot.pointer("/25/ExportWorkflowCodeSource/agent_mode"),
        Some(&serde_json::json!("existing_agents"))
    );
    assert_eq!(
        snapshot.pointer("/26/WorkflowCodePackageExported/package/name"),
        Some(&serde_json::json!("toy-flow"))
    );
    assert_eq!(
        snapshot.pointer("/27/WorkflowCodePackageImported/artifact/metadata/name"),
        Some(&serde_json::json!("toy-flow"))
    );
    assert_eq!(
        snapshot.pointer("/28/WorkflowCodeSourceExported/export/source_path"),
        Some(&serde_json::json!("workflow.js"))
    );
    assert_eq!(
        snapshot.pointer("/28/WorkflowCodeSourceExported/export/files/0/path"),
        Some(&serde_json::json!("schemas/final.json"))
    );
    assert_eq!(
        snapshot.pointer("/29/ListWorkflowRegistry/session_id"),
        Some(&serde_json::json!("session-1"))
    );
    assert_eq!(
        snapshot.pointer("/30/GetWorkflowRegistryEntry/name"),
        Some(&serde_json::json!("toy-flow"))
    );
    assert_eq!(
        snapshot.pointer("/31/AddWorkflowRegistryEntry/scope"),
        Some(&serde_json::json!("workspace"))
    );
    assert_eq!(
        snapshot.pointer("/31/AddWorkflowRegistryEntry/source/kind"),
        Some(&serde_json::json!("source_directory"))
    );
    assert_eq!(
        snapshot.pointer("/31/AddWorkflowRegistryEntry/source/files/1/path"),
        Some(&serde_json::json!("schemas/final.json"))
    );
    assert_eq!(
        snapshot.pointer("/32/AddWorkflowRegistryEntryFromWorkflow/agent_mode"),
        Some(&serde_json::json!("existing_agents"))
    );
    assert_eq!(
        snapshot.pointer("/33/DeleteWorkflowRegistryEntry/scope"),
        Some(&serde_json::json!("workspace"))
    );
    assert_eq!(
        snapshot.pointer("/34/LoadWorkflowRegistryEntry/provider_rebindings/0/provider"),
        Some(&serde_json::json!("dev-stub"))
    );
    assert_eq!(
        snapshot.pointer("/35/RunWorkflowRegistryEntry/prompt"),
        Some(&serde_json::json!("Run this registered workflow."))
    );
    assert_eq!(
        snapshot.pointer("/36/WorkflowRegistryListed/entries/0/source_scope"),
        Some(&serde_json::json!("workspace"))
    );
    assert_eq!(
        snapshot.pointer("/36/WorkflowRegistryListed/entries/0/summary/endpoints/1"),
        Some(&serde_json::json!("review"))
    );
    assert_eq!(
        snapshot.pointer("/36/WorkflowRegistryListed/entries/0/summary/queues/0"),
        Some(&serde_json::json!("urgent"))
    );
    assert_eq!(
        snapshot.pointer("/36/WorkflowRegistryListed/entries/0/summary/default_endpoint"),
        Some(&serde_json::json!("entry"))
    );
    assert_eq!(
        snapshot.pointer("/37/WorkflowRegistryEntry/entry/source_kind"),
        Some(&serde_json::json!("source_directory"))
    );
    assert_eq!(
        snapshot.pointer("/38/WorkflowRegistryEntryAdded/entry/validation/ok"),
        Some(&serde_json::json!(true))
    );
    assert_eq!(
        snapshot.pointer("/39/WorkflowRegistryEntryDeleted/path"),
        Some(&serde_json::json!("/workspace/.chariox/workflows/toy-flow"))
    );
    assert_eq!(
        snapshot.pointer("/40/WorkflowRegistryEntryLoaded/result/apply/workflow_id"),
        Some(&serde_json::json!("workflow-1"))
    );
    assert_eq!(
        snapshot.pointer("/41/WorkflowRegistryEntryRun/result/invocation/kind"),
        Some(&serde_json::json!("enqueued"))
    );
    assert_eq!(
        snapshot.pointer("/42/BindWorkflowCodeSource/origin"),
        Some(&serde_json::json!("generated"))
    );
    assert_eq!(
        snapshot.pointer("/42/BindWorkflowCodeSource/expected_workflow_revision"),
        Some(&serde_json::json!(7))
    );
    assert_eq!(
        snapshot.pointer("/43/WorkflowCodeSourceBound/workflow/code_source/artifact_name"),
        Some(&serde_json::json!("workflow-source-session-1-workflow-1"))
    );
    assert_eq!(
        snapshot.pointer("/43/WorkflowCodeSourceBound/workflow/code_source/workflow_revision"),
        Some(&serde_json::json!(1))
    );
    assert_eq!(
        snapshot.pointer("/44/RebuildWorkflowCodeSource/confirm"),
        Some(&serde_json::json!(false))
    );
    assert_eq!(
        snapshot.pointer("/45/WorkflowCodeRebuildPreview/preview/changes/0/remove_visual_only"),
        Some(&serde_json::json!(1))
    );
    assert_eq!(
        snapshot.pointer("/46/RebuildWorkflowCodeSource/confirm"),
        Some(&serde_json::json!(true))
    );
    assert_eq!(
        snapshot.pointer("/47/WorkflowCodeSourceRebuilt/result/workflow_id"),
        Some(&serde_json::json!("workflow-1"))
    );
    assert_eq!(
        snapshot.pointer("/48/UpdateWorkflowCodeSourceFromWorkflow/confirm"),
        Some(&serde_json::json!(false))
    );
    assert_eq!(
        snapshot.pointer("/49/WorkflowCodeSourceUpdatePreview/preview/added_lines"),
        Some(&serde_json::json!(3))
    );
    assert_eq!(
        snapshot
            .pointer("/50/UpdateWorkflowCodeSourceFromWorkflow/expected_generated_source_sha256"),
        Some(&serde_json::json!("new-sha256"))
    );
    assert_eq!(
        snapshot.pointer("/51/WorkflowCodeSourceUpdated/preview/changed"),
        Some(&serde_json::json!(true))
    );
}
