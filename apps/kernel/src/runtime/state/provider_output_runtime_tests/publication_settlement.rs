use super::*;

async fn assert_completed_publication_output_settlement(
    adapter_key: &str,
    provider: &str,
    client_interface: crate::provider::ProviderClientInterface,
    waits_for_provider_completion: bool,
) {
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-publication-claim",
            "worktree-publication-claim",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "publication-settlement-test-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("test client should attach");
    let provider_run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                adapter_key,
                provider,
                "default",
                "gpt-test",
            )
            .with_agent_id(agent.id())
            .with_client_interface(client_interface),
        )
        .expect("provider run should launch");
    app.update_provider_run_projection(provider_run.clone());

    let workflow = app
        .sessions_mut()
        .create_workflow(session.id(), Some("published".to_string()))
        .expect("workflow should be created");
    let node = app
        .sessions_mut()
        .add_workflow_node(session.id(), workflow.id(), agent.id())
        .expect("workflow node should be added");
    app.sessions_mut()
        .set_workflow_node_can_complete_run(session.id(), workflow.id(), node.id(), true)
        .expect("workflow node should complete the run");
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");
    let publication_invocation = crate::session::WorkflowPublicationInvocationEnvelope {
        publication_id: "publication-1".to_string(),
        hook_id: Some("hook-1".to_string()),
        invocation_id: "request-1".to_string(),
        transport: "human_http".to_string(),
        endpoint_id: endpoint.id().to_string(),
        queue_ref: Some("default".to_string()),
        input: serde_json::json!({ "prompt": "render a dashboard" }),
        artifacts: Vec::new(),
        mode: Some("sync".to_string()),
        caller: serde_json::json!({ "type": "anonymous" }),
    };
    let workflow_run = app
        .sessions_mut()
        .invoke_workflow_endpoint_with_publication_invocation(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("render a dashboard".to_string()),
            Some(publication_invocation),
        )
        .expect("published workflow run should be created");
    let node_run_id = workflow_run.node_runs()[0].id().to_string();
    app.sessions_mut()
        .prepare_workflow_turn(
            session.id(),
            workflow_run.id(),
            &node_run_id,
            format!("workflow-ack:{node_run_id}"),
            "workflow prompt".to_string(),
            None,
            None,
        )
        .expect("workflow turn should be prepared");
    app.sessions_mut()
        .start_workflow_node_run(session.id(), workflow_run.id(), &node_run_id)
        .expect("workflow node should start");
    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
        agent.id(),
        "workflow prompt",
        crate::session::PromptStatus::Queued,
    )
    .with_workflow_context(workflow_run.id(), &node_run_id);
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("workflow prompt should start");
    let active_prompt_id = app
        .prompt_owner_active_prompt_for_agent(session.id(), agent.id())
        .expect("active prompt should load")
        .expect("workflow prompt should be active")
        .id()
        .to_string();
    let queued_prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "next queued prompt",
        crate::session::PromptStatus::Queued,
    );
    let queued = app
        .prompt_owner_submit_prepared_prompt(session.id(), queued_prompt, false)
        .expect("next prompt should queue");
    assert!(matches!(
        queued,
        crate::session::PromptSubmissionOutcome::Queued { .. }
    ));
    let claim_id = format!(
        "workflow-node:{}:{}:{}",
        session.id(),
        workflow_run.id(),
        node_run_id,
    );
    app.acquire_workflow_node_workspace_claim(
        session.id(),
        &claim_id,
        agent.id(),
        workflow_run.id(),
        &node_run_id,
    )
    .expect("workflow workspace claim should be acquired");
    crate::transport::flow_control::note_prompt_started(&mut app, provider_run.id());

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let context = crate::transport::runtime_tools::WorkflowRuntimeToolContext {
        session_id: session.id().to_string(),
        workflow_run_ref: workflow_run.id().to_string(),
        workflow_node_run_id: node_run_id.clone(),
        delivery_token: None,
        allowed_handoff_schema_refs: Vec::new(),
        workflow_run_output_schema_ref: None,
        workflow_intermediate_output_schema_ref: None,
        can_complete_workflow_run: true,
        can_emit_intermediate_workflow_run_output: true,
    };
    let (result, _) = runtime
        .owned
        .dispatch_workflow_runtime_tool_call(
            crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL
                .to_string(),
            serde_json::json!({ "workflow_output_json": "{\"status\":\"done\"}" }),
            context,
        )
        .expect("final publication output should settle");

    assert_eq!(result.payload["valid"], true);
    let durable_events = runtime
        .owned
        .durable_state_store
        .load_subject_events_by_kind(session.id(), "workflow.runtime.updated", 10)
        .expect("durable workflow runtime tool events should load");
    let latest_event = durable_events
        .last()
        .expect("workflow output submission should be durable before returning");
    assert_eq!(latest_event.payload["reason"], "workflow_runtime_tool");
    let durable_run = runtime
        .owned
        .durable_state_store
        .resolve_workflow_run(session.host_daemon_id(), session.id(), workflow_run.id())
        .expect("durable workflow run should load")
        .expect("durable workflow run should exist");
    assert_eq!(
        durable_run.status(),
        crate::session::WorkflowRunStatus::Completed
    );
    if waits_for_provider_completion {
        let hot_session = runtime
            .owned
            .session_store
            .read()
            .get_session(session.id())
            .expect("hot session should remain available");
        assert_eq!(
            hot_session
                .workflow_run(workflow_run.id())
                .expect("terminal workflow run must remain hot until its provider prompt settles")
                .status(),
            crate::session::WorkflowRunStatus::Completed,
        );
        assert!(
            hot_session
                .durable_runtime_snapshot()
                .workflow_run(workflow_run.id())
                .is_some(),
            "a crash before provider-prompt settlement must retain the referenced terminal run",
        );
        assert!(
            runtime.owned.prompt_workspace_claims.contains(&claim_id),
            "the provider-completion path retains the claim until prompt settlement",
        );
        let hot_session = runtime
            .owned
            .session_store
            .read()
            .get_session(session.id())
            .expect("hot session should remain available");
        assert_eq!(
            runtime
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&hot_session, agent.id())
                .as_ref()
                .map(|prompt| prompt.id()),
            Some(active_prompt_id.as_str()),
            "validated output must not activate the next queued prompt before the provider turn ends",
        );
        // The real provider settlement marks this reservation before clearing
        // the prompt, then awaits post-turn work. Another event may persist and
        // archive the room while that await is in flight.
        runtime
            .owned
            .session_store
            .write()
            .mark_workflow_run_settling(session.id(), workflow_run.id())
            .unwrap();
        let completion = runtime
            .owned
            .complete_local_prompt_without_advance_if_matches(
                session.id(),
                agent.id(),
                Some(provider_run.id()),
                Some(&active_prompt_id),
            )
            .unwrap()
            .expect("provider prompt should settle");
        runtime
            .owned
            .persist_workflow_runtime_session(session.id(), "interleaved_settlement_test")
            .unwrap();
        let hot = runtime
            .owned
            .session_store
            .get_session(session.id())
            .unwrap();
        assert!(
            hot.workflow_run(workflow_run.id()).is_some(),
            "archival must retain a terminal workflow during post-provider settlement"
        );
        assert!(
            hot.durable_runtime_snapshot()
                .workflow_run(workflow_run.id())
                .is_some(),
            "durable snapshot must retain the in-progress settlement"
        );
        runtime
            .owned
            .workflow_complete_prompt(
                session.id(),
                &completion.completion.completed,
                Some(provider_run.id()),
            )
            .expect("interleaved persistence must not lose the completing workflow");
        assert!(
            !runtime.owned.prompt_workspace_claims.contains(&claim_id),
            "completed provider settlement must release the node's workspace claim"
        );
        runtime
            .owned
            .session_store
            .write()
            .clear_workflow_run_settling(session.id(), workflow_run.id())
            .unwrap();
        runtime
            .owned
            .persist_workflow_runtime_session(session.id(), "settlement_test_finished")
            .unwrap();
        assert!(
            runtime
                .owned
                .session_store
                .get_session(session.id())
                .unwrap()
                .workflow_run(workflow_run.id())
                .is_none(),
            "finished settlement must not retain terminal runs indefinitely"
        );
    } else {
        assert!(
            !runtime.owned.prompt_workspace_claims.contains(&claim_id),
            "fast publication completion must release the workflow workspace claim",
        );
    }
}

#[tokio::test]
async fn completed_publication_output_releases_workflow_workspace_claim() {
    assert_completed_publication_output_settlement(
        "dev-stub",
        "codex",
        crate::provider::ProviderClientInterface::Chariox,
        false,
    )
    .await;
}

#[tokio::test]
async fn completed_publication_output_retains_terminal_run_until_provider_settlement() {
    assert_completed_publication_output_settlement(
        "codex",
        "codex",
        crate::provider::ProviderClientInterface::Chariox,
        true,
    )
    .await;
}

#[tokio::test]
async fn completed_headless_claude_publication_waits_for_stop_before_advancing_queue() {
    assert_completed_publication_output_settlement(
        "claude",
        "claude-headless",
        crate::provider::ProviderClientInterface::NativeTui,
        true,
    )
    .await;
}
