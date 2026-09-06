use super::*;
use std::sync::{Arc, Barrier};
use tokio::sync::Mutex;

#[test]
fn concurrent_owned_workflow_launches_preserve_single_run_admission() {
    // The admission race is timing dependent, so repeat the scenario several
    // times within a single test run. The pre-fix behavior misattributes the
    // Started outcome (reporting 0 Started / 32 Enqueued) on a large fraction of
    // iterations, so repetition makes a regression fail deterministically here.
    const SCENARIO_REPEATS: usize = 24;
    for iteration in 0..SCENARIO_REPEATS {
        run_single_run_admission_scenario(iteration);
    }
}

fn run_single_run_admission_scenario(iteration: usize) {
    const INVOCATION_COUNT: usize = 32;

    let (runtime, session_id, workflow_id, endpoint_id, _test_root) = runtime_with_idle_workflow();
    let barrier = Arc::new(Barrier::new(INVOCATION_COUNT));
    let mut handles = Vec::with_capacity(INVOCATION_COUNT);
    for index in 0..INVOCATION_COUNT {
        let runtime = runtime.clone();
        let barrier = Arc::clone(&barrier);
        let session_id = session_id.clone();
        let workflow_id = workflow_id.clone();
        let endpoint_id = endpoint_id.clone();
        handles.push(
            std::thread::Builder::new()
                .name(format!("owned-workflow-launch-{iteration}-{index}"))
                .stack_size(8 * 1024 * 1024)
                .spawn(move || {
                    barrier.wait();
                    runtime.owned.workflow_enqueue_prompt_and_maybe_start(
                        &session_id,
                        &workflow_id,
                        &endpoint_id,
                        Some(format!("concurrent owned invocation {index}")),
                        None,
                        None,
                    )
                })
                .expect("owned workflow launch thread should spawn"),
        );
    }

    let outcomes = handles
        .into_iter()
        .map(|handle| {
            handle
                .join()
                .unwrap_or_else(|payload| std::panic::resume_unwind(payload))
        })
        .collect::<Vec<_>>();
    let errors = outcomes
        .iter()
        .filter_map(|outcome| outcome.as_ref().err().map(ToString::to_string))
        .collect::<Vec<_>>();
    assert!(
        errors.is_empty(),
        "iteration {iteration}: concurrent invokes failed: {errors:?}"
    );
    let started = outcomes
        .iter()
        .filter(|outcome| {
            matches!(
                outcome,
                Ok((
                    crate::app::workflow_runtime::WorkflowLaunchOutcome::Started { .. },
                    _
                ))
            )
        })
        .count();
    let enqueued = outcomes
        .iter()
        .filter(|outcome| {
            matches!(
                outcome,
                Ok((
                    crate::app::workflow_runtime::WorkflowLaunchOutcome::Enqueued { .. },
                    _
                ))
            )
        })
        .count();
    let session = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .expect("session should remain available");
    let active_runs = session
        .workflow_runs()
        .iter()
        .filter(|workflow_run| {
            matches!(
                workflow_run.status(),
                crate::session::WorkflowRunStatus::Created
                    | crate::session::WorkflowRunStatus::Running
                    | crate::session::WorkflowRunStatus::Waiting
            )
        })
        .count();
    assert_eq!(
        started, 1,
        "iteration {iteration}: exactly one concurrent invoke should report Started \
         (started={started}, enqueued={enqueued}, active_runs={active_runs})"
    );
    assert_eq!(
        enqueued,
        INVOCATION_COUNT - 1,
        "iteration {iteration}: the losing invocations should report Enqueued"
    );
    assert_eq!(
        active_runs, 1,
        "iteration {iteration}: concurrent admission created extra runs"
    );
    assert_eq!(
        session.workflow_queued_prompts().len(),
        INVOCATION_COUNT - 1,
        "iteration {iteration}: exactly one prompt should be admitted"
    );
}

#[test]
fn owned_launch_reports_started_when_prompt_admitted_by_concurrent_dispatch() {
    // Deterministically model the losing invocation: its prompt is enqueued and
    // then admitted into the single primary run by a *different* invocation's
    // dispatch loop before it runs its own. The owning invocation must still be
    // able to recover the Started attribution for its prompt instead of a
    // spurious Enqueued.
    let (runtime, session_id, workflow_id, endpoint_id, _test_root) = runtime_with_idle_workflow();
    let queued = runtime
        .owned
        .session_store
        .write()
        .enqueue_workflow_prompt(
            &session_id,
            &workflow_id,
            &endpoint_id,
            Some("owning invocation".to_string()),
            None,
            crate::session::WorkflowQueuedPromptSource::Manual,
            None,
        )
        .expect("prompt should enqueue");

    // A concurrent invocation's dispatch loop admits the oldest queued prompt.
    let (claimed_outcome, _dispatches) = runtime
        .owned
        .workflow_start_next_queued_prompt_for_response(&session_id)
        .expect("dispatch should succeed");
    let claimed_outcome =
        claimed_outcome.expect("the idle primary instance should admit exactly one run");
    let started_run = match claimed_outcome {
        crate::app::workflow_runtime::WorkflowLaunchOutcome::Started { workflow_run, .. } => {
            workflow_run
        }
        crate::app::workflow_runtime::WorkflowLaunchOutcome::Enqueued { .. } => {
            panic!("the admitted prompt should start")
        }
    };
    assert_eq!(started_run.queue_item_id(), Some(queued.id()));

    // The owning invocation recovers its Started run rather than mis-reporting.
    let recovered = runtime
        .owned
        .workflow_started_run_for_queued_prompt(&session_id, queued.id())
        .expect("started-run lookup should succeed")
        .expect("the owning invocation must observe its admitted run");
    assert_eq!(recovered.id(), started_run.id());
    assert_eq!(recovered.queue_item_id(), Some(queued.id()));

    runtime
        .owned
        .session_store
        .write()
        .cancel_workflow_run(&session_id, started_run.id())
        .expect("the admitted run should become terminal");
    assert!(runtime
        .owned
        .workflow_started_run_for_queued_prompt(&session_id, queued.id())
        .expect("terminal-run lookup should succeed")
        .is_none());

    // A prompt that was never admitted has no Started run.
    assert!(runtime
        .owned
        .workflow_started_run_for_queued_prompt(&session_id, "workflow-queued-prompt-missing")
        .expect("started-run lookup should succeed")
        .is_none());
}

#[test]
fn owned_launch_response_tracks_requested_prompt_when_older_prompt_starts() {
    let (runtime, session_id, workflow_id, endpoint_id, _test_root) = runtime_with_idle_workflow();
    let older_prompt = runtime
        .owned
        .session_store
        .write()
        .enqueue_workflow_prompt(
            &session_id,
            &workflow_id,
            &endpoint_id,
            Some("older queued invocation".to_string()),
            None,
            crate::session::WorkflowQueuedPromptSource::Manual,
            None,
        )
        .expect("older prompt should be queued");

    let (outcome, _dispatches) = runtime
        .owned
        .workflow_enqueue_prompt_and_maybe_start(
            &session_id,
            &workflow_id,
            &endpoint_id,
            Some("new requested invocation".to_string()),
            None,
            None,
        )
        .expect("new prompt should advance the older queued work");
    let requested_prompt = match outcome {
        crate::app::workflow_runtime::WorkflowLaunchOutcome::Enqueued { queued_prompt, .. } => {
            queued_prompt
        }
        crate::app::workflow_runtime::WorkflowLaunchOutcome::Started { .. } => {
            panic!("new invocation must not report the older prompt's run as its own")
        }
    };
    assert_ne!(requested_prompt.id(), older_prompt.id());
    assert_eq!(requested_prompt.prompt(), Some("new requested invocation"));
    let session = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .expect("session should remain available");
    assert_eq!(session.workflow_runs().len(), 1);
    assert_eq!(
        session.workflow_runs()[0].invocation_prompt(),
        Some("older queued invocation")
    );
    assert_eq!(
        session
            .workflow_queued_prompts()
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        vec![*requested_prompt]
    );
}

fn runtime_with_idle_workflow() -> (KernelRuntimeState, String, String, String, TestRoot) {
    let test_root = TestRoot(std::env::temp_dir().join(format!(
        "chariox-owned-workflow-admission-{}-{}-{:032x}",
        std::process::id(),
        crate::session::unix_epoch_ms(),
        rand::random::<u128>()
    )));
    std::fs::create_dir_all(&test_root.0).expect("workflow test root should be created");
    let test_root_path = test_root.0.to_string_lossy().to_string();
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            &test_root_path,
            &test_root_path,
        ))
        .expect("session should be created");
    let agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("owned-workflow-agent"),
        )
        .expect("workflow agent should be created");
    let workflow = app
        .sessions_mut()
        .create_workflow(session.id(), Some("owned-workflow".to_string()))
        .expect("workflow should be created");
    app.sessions_mut()
        .set_workflow_flush_agent_context_before_run(session.id(), workflow.id(), true)
        .expect("workflow should require fresh provider context");
    app.agents_mut()
        .set_agent_provider_resume_state(
            agent.id(),
            crate::provider::ProviderResumeState::from_external_provider_session(
                "opencode",
                "stale-opencode-session",
            ),
        )
        .expect("agent should retain a stale provider session for the drill");
    let node = app
        .sessions_mut()
        .add_workflow_node_owned(
            session.id(),
            workflow.id(),
            agent.id(),
            crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
            crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
            "Owned workflow node".to_string(),
        )
        .expect("workflow node should be added");
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");

    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "dev-stub",
        "dev-stub",
        "default",
        "workflow-test-idle",
    )
    .with_agent_id(agent.id());
    let mut provider_run = crate::provider::RuntimeProviderRun::new(
        "owned-workflow-provider",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "owned-workflow-provider".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("owned-workflow-provider".to_string()),
        },
    );
    provider_run.mark_running();
    app.providers_mut()
        .insert_run_for_test(provider_run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(provider_run.id().to_string()))
        .expect("provider run should become active");
    app.update_provider_run_projection(provider_run);

    let session_id = session.id().to_string();
    let workflow_id = workflow.id().to_string();
    let endpoint_id = endpoint.id().to_string();
    (
        runtime_state_from_app(app),
        session_id,
        workflow_id,
        endpoint_id,
        test_root,
    )
}

#[test]
fn owned_workflow_launch_retires_idle_provider_and_suppresses_resume_state() {
    let (runtime, session_id, workflow_id, endpoint_id, _test_root) = runtime_with_idle_workflow();
    let old_provider_run_id = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .expect("session should resolve")
        .active_provider_run_id()
        .expect("idle provider should be active")
        .to_string();

    let (outcome, dispatches) = runtime
        .owned
        .workflow_enqueue_prompt_and_maybe_start(
            &session_id,
            &workflow_id,
            &endpoint_id,
            Some("fresh workflow invocation".to_string()),
            None,
            None,
        )
        .expect("workflow should launch");

    assert_eq!(dispatches.starting_provider_runs.len(), 1);
    assert_eq!(
        dispatches.provider_run_retirements,
        std::collections::BTreeMap::from([(
            dispatches.starting_provider_runs[0].clone(),
            vec![old_provider_run_id],
        )])
    );
    let new_provider_run = runtime
        .owned
        .provider_store
        .get_run(&dispatches.starting_provider_runs[0])
        .expect("fresh provider run should resolve");
    assert!(new_provider_run.resume_state().is_empty());
    assert!(new_provider_run.workflow_tools_enabled());

    let workflow_run = match outcome {
        crate::app::workflow_runtime::WorkflowLaunchOutcome::Started { workflow_run, .. } => {
            workflow_run
        }
        crate::app::workflow_runtime::WorkflowLaunchOutcome::Enqueued { .. } => {
            panic!("idle workflow invocation should start")
        }
    };
    let agent_id = workflow_run.node_runs()[0].agent_id();
    let session = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .expect("session should resolve");
    let queued_prompt = runtime
        .owned
        .prompt_state_owner
        .state_parts(&session, agent_id)
        .1
        .front()
        .cloned()
        .expect("workflow prompt should wait for the starting provider");
    assert!(!runtime
        .owned
        .workflow_prompt_requires_fresh_provider_context(&session_id, agent_id, &queued_prompt)
        .expect("freshness check should succeed"));
}

#[test]
fn failed_owned_queue_launch_releases_its_workspace_claim() {
    let (runtime, session_id, workflow_id, endpoint_id, _test_root) = runtime_with_idle_workflow();
    let source = runtime
        .owned
        .agent_store
        .get_session_agents(&session_id)
        .into_iter()
        .find(|agent| agent.alias() == Some("owned-workflow-agent"))
        .expect("workflow agent should resolve");
    runtime
        .owned
        .session_store
        .write()
        .enqueue_workflow_prompt(
            &session_id,
            &workflow_id,
            &endpoint_id,
            Some("fail before provider dispatch".to_string()),
            None,
            crate::session::WorkflowQueuedPromptSource::Manual,
            None,
        )
        .expect("workflow prompt should enqueue");
    runtime
        .owned
        .workflow_ensure_dispatchable_runtime_instance(&session_id)
        .expect("idle workflow instance should be dispatchable");
    let (queued_prompt, workflow_run, workflow, endpoint) = runtime
        .owned
        .session_store
        .write()
        .dequeue_next_workflow_prompt_and_create_run(&session_id)
        .expect("queued prompt should be claimable")
        .expect("queued prompt should create a workflow run");
    let node_run = workflow_run
        .node_runs()
        .first()
        .expect("workflow run should retain its entry node");
    let claim_id =
        runtime
            .owned
            .workflow_dispatch_claim_id(&session_id, workflow_run.id(), node_run.id());
    runtime
        .owned
        .acquire_workflow_node_workspace_claim(
            &session_id,
            &claim_id,
            source.id(),
            workflow_run.id(),
            node_run.id(),
        )
        .expect("the launch should own its workspace claim");

    let source_worktree = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .expect("source session should retain its worktree")
        .worktree_id()
        .to_string();
    let blocked_agent = runtime
        .owned
        .agent_store
        .materialize_workflow_runtime_agent(source.clone(), &session_id, &source_worktree);
    let blocked_workflow = runtime
        .owned
        .session_store
        .write()
        .create_workflow(&session_id, Some("blocked-after-failed-launch".to_string()))
        .expect("blocked workflow should be created");
    runtime
        .owned
        .session_store
        .write()
        .set_workflow_flush_agent_context_before_run(&session_id, blocked_workflow.id(), false)
        .expect("blocked workflow should keep provider context");
    let blocked_node = runtime
        .owned
        .session_store
        .write()
        .add_workflow_node_owned(
            &session_id,
            blocked_workflow.id(),
            blocked_agent.id(),
            crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
            crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
            "Blocked workflow node".to_string(),
        )
        .expect("blocked workflow node should be created");
    let blocked_endpoint = runtime
        .owned
        .session_store
        .write()
        .create_workflow_endpoint(
            &session_id,
            blocked_workflow.id(),
            blocked_node.id(),
            Some("entry".to_string()),
        )
        .expect("blocked workflow endpoint should be created");
    let (blocked_outcome, _) = runtime
        .owned
        .workflow_enqueue_prompt_and_maybe_start(
            &session_id,
            blocked_workflow.id(),
            blocked_endpoint.id(),
            Some("wait for the failed launch claim".to_string()),
            None,
            None,
        )
        .expect("blocked workflow invocation should be admitted");
    let blocked_run = match blocked_outcome {
        crate::app::workflow_runtime::WorkflowLaunchOutcome::Started { workflow_run, .. } => {
            workflow_run
        }
        crate::app::workflow_runtime::WorkflowLaunchOutcome::Enqueued { .. } => {
            panic!("independent workflow should create a run before blocking on the claim")
        }
    };
    let blocked_node_run_id = blocked_run.node_runs()[0].id().to_string();
    assert_eq!(
        blocked_run.node_runs()[0].status(),
        crate::session::WorkflowNodeRunStatus::BlockedOnWorkspaceClaim,
    );

    runtime
        .owned
        .destroy_agent(source.id(), crate::session::DEFAULT_LOCAL_USER_ID)
        .expect("test should remove the node agent after claim acquisition");

    let mut failure_dispatches = WorkflowPromptDispatches::default();
    let failed_launch = runtime.owned.workflow_schedule_queued_prompt_run(
        &session_id,
        queued_prompt,
        workflow_run,
        workflow,
        endpoint,
        &mut failure_dispatches,
    );
    assert!(
        failed_launch.is_err(),
        "missing node agent should fail the queued launch"
    );

    assert!(
        !runtime.owned.prompt_workspace_claims.contains(&claim_id),
        "failed queue launch must not retain its workspace claim"
    );
    assert!(
        !failure_dispatches.is_empty(),
        "claim release must return the blocked workflow's retry dispatch"
    );
    let retried_run = runtime
        .owned
        .session_store
        .read()
        .resolve_workflow_run_ref(&session_id, blocked_run.id())
        .expect("retried workflow run should resolve");
    assert_ne!(
        retried_run
            .node_runs()
            .iter()
            .find(|node_run| node_run.id() == blocked_node_run_id)
            .expect("retried node should resolve")
            .status(),
        crate::session::WorkflowNodeRunStatus::BlockedOnWorkspaceClaim,
        "failed queue launch must immediately retry a node blocked on its released claim"
    );
}

#[test]
fn runtime_cleanup_removes_unreferenced_hidden_agents_and_worktrees() {
    let (runtime, session_id, _workflow_id, _endpoint_id, _test_root) =
        runtime_with_idle_workflow();
    let source = runtime
        .owned
        .agent_store
        .get_session_agents(&session_id)
        .into_iter()
        .find(|agent| agent.visible_in_freeform())
        .expect("visible source agent should exist");
    let initial_focus = runtime
        .owned
        .session_store
        .read()
        .get_session(&session_id)
        .expect("session should exist")
        .focused_agent_id()
        .map(str::to_string);
    let orphan_worktree = runtime
        .owned
        .config_projection
        .snapshot()
        .workflow_runtime_artifact_root()
        .join("instances")
        .join(&session_id)
        .join("unreferenced-instance");
    std::fs::create_dir_all(&orphan_worktree).expect("orphan worktree fixture should be created");
    let orphan = runtime
        .owned
        .agent_store
        .materialize_workflow_runtime_agent(
            source.clone(),
            &session_id,
            &orphan_worktree.to_string_lossy(),
        );
    assert!(!orphan.visible_in_freeform());
    assert!(runtime
        .owned
        .session_store
        .read()
        .get_session(&session_id)
        .expect("session should exist")
        .workflow_runtime_instances()
        .iter()
        .all(|instance| instance.worktree_id() != orphan_worktree.to_string_lossy()));

    runtime
        .owned
        .workflow_cleanup_runtime_instances_exclusive(&session_id)
        .expect("orphan cleanup should succeed");

    assert!(runtime.owned.agent_store.get_agent(orphan.id()).is_err());
    assert!(!orphan_worktree.exists());
    assert_eq!(
        runtime
            .owned
            .session_store
            .read()
            .get_session(&session_id)
            .expect("session should remain")
            .focused_agent_id(),
        initial_focus.as_deref()
    );
}

#[test]
fn deleting_session_removes_registered_workflow_runtime_worktrees() {
    let (runtime, session_id, workflow_id, endpoint_id, test_root) = runtime_with_idle_workflow();
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(&test_root.0)
        .output()
        .expect("git init should run");
    std::fs::write(
        test_root.0.join("README.md"),
        "workflow runtime cleanup fixture\n",
    )
    .expect("fixture should be written");
    for args in [
        vec!["config", "user.email", "tests@chariox.invalid"],
        vec!["config", "user.name", "Chariox Tests"],
        vec!["add", "README.md"],
        vec!["commit", "-m", "fixture"],
    ] {
        let output = std::process::Command::new("git")
            .args(&args)
            .current_dir(&test_root.0)
            .output()
            .expect("git fixture command should run");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let instance_id = "workflow-instance-delete-test";
    let instance_root = runtime
        .owned
        .config_projection
        .snapshot()
        .workflow_runtime_artifact_root()
        .join("instances")
        .join(&session_id);
    std::fs::create_dir_all(&instance_root).expect("instance root should be created");
    let instance_worktree = instance_root.join(instance_id);
    let placement = crate::agent::GitWorktreePlacement {
        target_directory: Some(instance_worktree.display().to_string()),
        branch: None,
        from_ref: Some("HEAD".to_string()),
    };
    let registered_worktree =
        crate::git_worktree_placement::prepare_workflow_runtime_worktree_or_reuse_directory(
            &placement,
            &test_root.0,
            None,
            "prepare session deletion test worktree",
        )
        .expect("runtime worktree should be created");
    let workflow = runtime
        .owned
        .session_store
        .read()
        .resolve_workflow_ref(&session_id, &workflow_id)
        .expect("workflow should resolve");
    let node_agent_ids = workflow
        .nodes()
        .iter()
        .map(|node| (node.id().to_string(), node.agent_id().to_string()))
        .collect::<std::collections::BTreeMap<_, _>>();
    runtime
        .owned
        .session_store
        .write()
        .register_workflow_runtime_instance(
            &session_id,
            crate::session::WorkflowEndpointRuntimeInstance::new(
                instance_id,
                &workflow_id,
                &endpoint_id,
                workflow.revision(),
                2,
                false,
                node_agent_ids,
                registered_worktree,
            ),
        )
        .expect("runtime instance should register");
    assert!(instance_worktree.exists());
    std::fs::write(instance_root.join("stale-runtime-note"), "obsolete")
        .expect("stray runtime artifact should be created");

    runtime
        .owned
        .delete_session_ref(&session_id, None)
        .expect("session deletion should succeed");

    assert!(!instance_worktree.exists());
    assert!(!instance_root.exists());
    let worktree_list = std::process::Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(&test_root.0)
        .output()
        .expect("git worktree list should run");
    assert!(worktree_list.status.success());
    assert!(!String::from_utf8_lossy(&worktree_list.stdout)
        .contains(&instance_worktree.display().to_string()));
}

#[tokio::test]
async fn pool_clone_binds_exact_stable_account_and_launch_ignores_later_default_change() {
    let (runtime, session_id, _workflow_id, _endpoint_id, _test_root) =
        runtime_with_idle_workflow();

    // Register two managed accounts; the first is the default at bind time.
    let first = runtime
        .owned
        .provider_account_profiles
        .create_managed(crate::session::DEFAULT_LOCAL_USER_ID, "codex", "Pool First")
        .expect("first managed account profile should register");
    let second = runtime
        .owned
        .provider_account_profiles
        .create_managed(
            crate::session::DEFAULT_LOCAL_USER_ID,
            "codex",
            "Pool Second",
        )
        .expect("second managed account profile should register");
    runtime
        .owned
        .provider_account_profiles
        .set_default(
            crate::session::DEFAULT_LOCAL_USER_ID,
            "codex",
            &first.profile_id,
        )
        .expect("initial default should be set");

    // Bind the visible node agent to the exact stable profile id.
    let source_id = {
        let app = runtime.app.lock().await;
        app.agents()
            .get_session_agents(&session_id)
            .into_iter()
            .find(|agent| agent.alias() == Some("owned-workflow-agent"))
            .expect("aliased node agent should exist")
            .id()
            .to_string()
    };
    {
        let app = runtime.app.lock().await;
        app.agents_mut()
            .set_agent_runtime_profile_with_account_profile(
                &source_id,
                "dev-stub",
                None,
                None,
                Some(first.profile_id.clone()),
                crate::provider::ProviderResumeState::default(),
            )
            .expect("stable account binding should apply");
    }
    let source = runtime
        .owned
        .agent_store
        .get_agent(&source_id)
        .expect("source should resolve");
    assert_eq!(source.account_profile(), Some(first.profile_id.as_str()));

    // The pool clone carries the exact stable binding, not a sentinel.
    let clone_a = runtime
        .owned
        .agent_store
        .materialize_workflow_runtime_agent(source.clone(), &session_id, "wt-clone-a");
    assert_eq!(clone_a.alias(), Some("owned-workflow-agent-2"));
    assert_eq!(clone_a.provider(), source.provider());
    assert_eq!(clone_a.model(), source.model());
    assert_eq!(
        clone_a.account_profile(),
        Some(first.profile_id.as_str()),
        "clone must preserve the exact stable profile binding"
    );

    // Launch through the production prompt-launch seam: the request uses the
    // clone's bound profile id.
    let run_id = {
        let mut app = runtime.app.lock().await;
        app.ensure_prompt_provider_run_for_agent(&session_id, clone_a.id())
            .expect("clone provider run should launch")
    };
    let launched = runtime
        .owned
        .provider_store
        .get_run(&run_id)
        .expect("launched run should resolve");
    assert_eq!(launched.account_profile(), first.profile_id);

    // The provider default changes AFTER creation...
    runtime
        .owned
        .provider_account_profiles
        .set_default(
            crate::session::DEFAULT_LOCAL_USER_ID,
            "codex",
            &second.profile_id,
        )
        .expect("default switch should succeed");

    // ...yet a pool clone provisioned afterwards still binds the original
    // stable id instead of following the moving default.
    let clone_c = runtime
        .owned
        .agent_store
        .materialize_workflow_runtime_agent(source.clone(), &session_id, "wt-clone-c");
    assert_eq!(clone_c.account_profile(), Some(first.profile_id.as_str()));
    assert_ne!(clone_c.account_profile(), Some(second.profile_id.as_str()));

    // And a fresh relaunch of the existing clone still uses the original id.
    {
        let mut app = runtime.app.lock().await;
        app.end_provider_run_for_workflow_context_flush(&session_id, clone_a.id())
            .expect("previous run should retire");
    }
    let relaunched_run_id = {
        let mut app = runtime.app.lock().await;
        app.ensure_prompt_provider_run_for_agent(&session_id, clone_a.id())
            .expect("relaunched clone provider run should start")
    };
    let relaunched = runtime
        .owned
        .provider_store
        .get_run(&relaunched_run_id)
        .expect("relaunched run should resolve");
    assert_eq!(relaunched.account_profile(), first.profile_id);
}

#[tokio::test]
async fn source_agent_profile_change_retires_idle_materialized_pool_copies() {
    let (runtime, session_id, workflow_id, endpoint_id, test_root) = runtime_with_idle_workflow();
    let source = runtime
        .owned
        .agent_store
        .get_session_agents(&session_id)
        .into_iter()
        .find(|agent| agent.alias() == Some("owned-workflow-agent"))
        .expect("source workflow agent should exist");
    let workflow = runtime
        .owned
        .session_store
        .read()
        .resolve_workflow_ref(&session_id, &workflow_id)
        .expect("workflow should resolve");
    let previous_revision = workflow.revision();
    let node_id = workflow
        .nodes()
        .first()
        .expect("workflow should have one node")
        .id()
        .to_string();
    let primary_node_agents =
        std::collections::BTreeMap::from([(node_id.clone(), source.id().to_string())]);
    runtime
        .owned
        .session_store
        .write()
        .register_workflow_runtime_instance(
            &session_id,
            crate::session::WorkflowEndpointRuntimeInstance::new(
                "instance-primary",
                &workflow_id,
                &endpoint_id,
                previous_revision,
                1,
                true,
                primary_node_agents,
                test_root.0.display().to_string(),
            ),
        )
        .expect("primary instance should register");
    let clone = runtime
        .owned
        .agent_store
        .materialize_workflow_runtime_agent(
            source.clone(),
            &session_id,
            &test_root.0.display().to_string(),
        );
    let clone_id = clone.id().to_string();
    runtime
        .owned
        .session_store
        .write()
        .register_workflow_runtime_instance(
            &session_id,
            crate::session::WorkflowEndpointRuntimeInstance::new(
                "instance-clone",
                &workflow_id,
                &endpoint_id,
                previous_revision,
                2,
                false,
                std::collections::BTreeMap::from([(node_id, clone_id.clone())]),
                test_root.0.display().to_string(),
            ),
        )
        .expect("clone instance should register");

    runtime
        .update_agent_profile(
            &session_id,
            source.id(),
            crate::session::DEFAULT_LOCAL_USER_ID,
            None,
            None,
            Some("updated-workflow-model".to_string()),
            None,
        )
        .await
        .expect("source profile should update and retire stale copies");

    let session = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .expect("session should remain");
    assert_eq!(
        session
            .workflow(&workflow_id)
            .expect("workflow should remain")
            .revision(),
        previous_revision + 1
    );
    assert_eq!(session.workflow_runtime_instances().len(), 1);
    let primary = session
        .workflow_runtime_instance("instance-primary")
        .expect("primary should remain");
    assert!(primary.primary());
    assert_eq!(primary.workflow_revision(), previous_revision + 1);
    assert!(runtime.owned.agent_store.get_agent(&clone_id).is_err());
}

#[tokio::test]
async fn source_agent_substitute_change_retires_idle_materialized_pool_copies() {
    let (runtime, session_id, workflow_id, endpoint_id, test_root) = runtime_with_idle_workflow();
    let source = runtime
        .owned
        .agent_store
        .get_session_agents(&session_id)
        .into_iter()
        .find(|agent| agent.alias() == Some("owned-workflow-agent"))
        .expect("source workflow agent should exist");
    let source = runtime
        .owned
        .agent_store
        .add_agent_substitute(
            source.id(),
            crate::agent::AgentSubstituteProfile::new(
                "codex",
                "gpt-5.6-sol",
                Some("high".to_string()),
            ),
        )
        .expect("source substitute should be configured");
    let workflow = runtime
        .owned
        .session_store
        .read()
        .resolve_workflow_ref(&session_id, &workflow_id)
        .expect("workflow should resolve");
    let previous_revision = workflow.revision();
    let node_id = workflow
        .nodes()
        .first()
        .expect("workflow should have one node")
        .id()
        .to_string();
    runtime
        .owned
        .session_store
        .write()
        .register_workflow_runtime_instance(
            &session_id,
            crate::session::WorkflowEndpointRuntimeInstance::new(
                "instance-primary",
                &workflow_id,
                &endpoint_id,
                previous_revision,
                1,
                true,
                std::collections::BTreeMap::from([(node_id.clone(), source.id().to_string())]),
                test_root.0.display().to_string(),
            ),
        )
        .expect("primary instance should register");
    let clone = runtime
        .owned
        .agent_store
        .materialize_workflow_runtime_agent(
            source.clone(),
            &session_id,
            &test_root.0.display().to_string(),
        );
    let clone_id = clone.id().to_string();
    runtime
        .owned
        .session_store
        .write()
        .register_workflow_runtime_instance(
            &session_id,
            crate::session::WorkflowEndpointRuntimeInstance::new(
                "instance-clone",
                &workflow_id,
                &endpoint_id,
                previous_revision,
                2,
                false,
                std::collections::BTreeMap::from([(node_id, clone_id.clone())]),
                test_root.0.display().to_string(),
            ),
        )
        .expect("clone instance should register");

    runtime
        .update_agent_substitutes(
            &session_id,
            source.id(),
            crate::session::DEFAULT_LOCAL_USER_ID,
            crate::local::AgentSubstituteAction::Activate {
                index: 0,
                reason: Some("resource exhausted".to_string()),
            },
        )
        .await
        .expect("source substitute activation should retire stale copies");

    let session = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .expect("session should remain");
    assert_eq!(
        session
            .workflow(&workflow_id)
            .expect("workflow should remain")
            .revision(),
        previous_revision + 1
    );
    assert_eq!(session.workflow_runtime_instances().len(), 1);
    let primary = session
        .workflow_runtime_instance("instance-primary")
        .expect("primary should remain");
    assert!(primary.primary());
    assert_eq!(primary.workflow_revision(), previous_revision + 1);
    assert!(runtime.owned.agent_store.get_agent(&clone_id).is_err());
}

#[test]
fn pool_aliases_and_ordinals_survive_durable_restart_without_collisions() {
    let (runtime, session_id, workflow_id, endpoint_id, _test_root) = runtime_with_idle_workflow();
    let source = runtime
        .owned
        .agent_store
        .get_session_agents(&session_id)
        .into_iter()
        .find(|agent| agent.alias() == Some("owned-workflow-agent"))
        .expect("aliased source agent should exist");
    let copy_a = runtime
        .owned
        .agent_store
        .materialize_workflow_runtime_agent(source.clone(), &session_id, "wt-restart-a");
    let copy_b = runtime
        .owned
        .agent_store
        .materialize_workflow_runtime_agent(source.clone(), &session_id, "wt-restart-b");
    assert_eq!(copy_a.alias(), Some("owned-workflow-agent-2"));
    assert_eq!(copy_b.alias(), Some("owned-workflow-agent-3"));

    let workflow = runtime
        .owned
        .session_store
        .read()
        .resolve_workflow_ref(&session_id, &workflow_id)
        .expect("workflow should resolve");
    let node_agent_ids = workflow
        .nodes()
        .iter()
        .map(|node| (node.id().to_string(), node.agent_id().to_string()))
        .collect::<std::collections::BTreeMap<_, _>>();
    for (ordinal, primary) in [(1u16, true), (2, false), (3, false)] {
        runtime
            .owned
            .session_store
            .write()
            .register_workflow_runtime_instance(
                &session_id,
                crate::session::WorkflowEndpointRuntimeInstance::new(
                    format!("workflow-instance-restart-{ordinal}"),
                    &workflow_id,
                    &endpoint_id,
                    workflow.revision(),
                    ordinal,
                    primary,
                    node_agent_ids.clone(),
                    format!("wt-restart-{ordinal}"),
                ),
            )
            .expect("runtime instance should register");
    }

    // Durable boundary: agents, instances, and the session cross restarts as
    // serde documents. Round-trip each and rebuild the post-restart world only
    // from those documents.
    let live_agents: Vec<crate::agent::AgentInstance> = vec![source.clone(), copy_a, copy_b];
    let restored_agents: Vec<crate::agent::AgentInstance> =
        serde_json::from_value(serde_json::to_value(&live_agents).expect("agents should encode"))
            .expect("agents should decode");
    let restored_instances: Vec<crate::session::WorkflowEndpointRuntimeInstance> =
        serde_json::from_value(
            serde_json::to_value(
                runtime
                    .owned
                    .session_store
                    .read()
                    .get_session(&session_id)
                    .expect("session should resolve")
                    .workflow_runtime_instances()
                    .to_vec(),
            )
            .expect("instances should encode"),
        )
        .expect("instances should decode");
    let restored_session: crate::session::RuntimeSession = serde_json::from_value(
        serde_json::to_value(
            runtime
                .owned
                .session_store
                .get_session(&session_id)
                .expect("session should resolve"),
        )
        .expect("session should encode"),
    )
    .expect("session should decode");

    // Ordinals continue after restart without collisions.
    assert_eq!(restored_instances.len(), 3);
    assert_eq!(
        restored_session.next_workflow_runtime_instance_ordinal(&workflow_id, &endpoint_id),
        4
    );

    // Aliases stay user-facing and collision-free after restart.
    let mut fresh_agents = crate::agent::AgentService::new();
    for agent in restored_agents {
        fresh_agents.restore_agent(agent);
    }
    let restored_source = fresh_agents
        .get_agent(&source_id_of(&live_agents))
        .expect("restored source should resolve");
    let next_copy = fresh_agents.materialize_workflow_runtime_agent(
        restored_source.clone(),
        &session_id,
        "wt-restart-c",
    );
    assert_eq!(next_copy.alias(), Some("owned-workflow-agent-4"));

    // A squatter occupying the next suffix (e.g. a user-named agent restored
    // from durable state) forces the sequence to skip the collision.
    let squatter = crate::agent::AgentInstance::new(
        "agent-squatter-restart",
        crate::agent::generate_agent_ref(),
        &session_id,
        Some("owned-workflow-agent-5".to_string()),
        "dev-stub",
        None,
        None,
        None,
        crate::agent::GridPosition::new(0, 0, 1, 1),
    );
    fresh_agents.restore_agent(squatter);
    let skipped_copy = fresh_agents.materialize_workflow_runtime_agent(
        restored_source,
        &session_id,
        "wt-restart-d",
    );
    assert_eq!(skipped_copy.alias(), Some("owned-workflow-agent-6"));
}

fn source_id_of(agents: &[crate::agent::AgentInstance]) -> String {
    agents
        .iter()
        .find(|agent| agent.visible_in_freeform())
        .expect("visible source should exist")
        .id()
        .to_string()
}

struct TestRoot(std::path::PathBuf);

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn runtime_state_from_app(app: DaemonApp) -> KernelRuntimeState {
    let config_projection = app.config_projection_store();
    let session_store = app.session_state_store();
    let agent_store = app.agents().clone();
    let attachment_store = app.attachments().clone();
    let provider_store = app.providers().clone();
    let provider_process_tracking = app.provider_process_tracking_store();
    let slice_store = app.slices();
    let session_projection = app.session_state_projection_store();
    let provider_run_projection = app.provider_run_projection_store();
    let operational_history_store = app.operational_history_store();
    let durable_state_store = app.durable_state_store();
    let prompt_state_owner = app.prompt_state_owner();
    let active_turns = app.active_turn_store();
    let prompt_activity = app.prompt_activity_store();
    let prompt_workspace_claims = app.prompt_workspace_claim_store();
    let structured_output_records = app.structured_output_record_store();
    let terminal_stream = app.terminal_stream_store();
    let workflow_design_events = app.workflow_design_event_store();
    let metaagent_events = app.metaagent_event_store();
    let workspace_coordinator = app.workspace_coordinator();
    KernelRuntimeState::new_with_owned_state(
        Arc::new(Mutex::new(app)),
        config_projection,
        session_store,
        agent_store,
        attachment_store,
        provider_store,
        provider_process_tracking,
        slice_store,
        session_projection,
        provider_run_projection,
        operational_history_store,
        durable_state_store,
        prompt_state_owner,
        active_turns,
        prompt_activity,
        prompt_workspace_claims,
        structured_output_records,
        terminal_stream,
        workflow_design_events,
        metaagent_events,
        workspace_coordinator,
    )
}
