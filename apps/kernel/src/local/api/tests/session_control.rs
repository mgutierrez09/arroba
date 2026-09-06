use super::*;

#[test]
fn project_lifecycle_archives_idle_sessions_and_restore_parks_them() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-project", "worktree-project")
                .with_project_selection(SessionProjectSelection::New),
        ))
        .expect("named project session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };

    let archived = match harness
        .dispatch(LocalDaemonRequest::ArchiveProject(ArchiveProjectRequest {
            project_id: session.project_id().to_string(),
        }))
        .expect("idle project should archive")
    {
        LocalDaemonResponse::ProjectArchived { project, sessions } => (project, sessions),
        other => panic!("unexpected local response: {other:?}"),
    };
    assert_eq!(
        archived.0.status(),
        crate::session::RuntimeProjectStatus::Archived
    );
    assert_eq!(archived.1.len(), 1);
    assert_eq!(archived.1[0].status(), crate::session::SessionStatus::Ended);

    let restored = match harness
        .dispatch(LocalDaemonRequest::RestoreProject(RestoreProjectRequest {
            project_id: session.project_id().to_string(),
        }))
        .expect("archived project should restore")
    {
        LocalDaemonResponse::ProjectRestored { project, sessions } => (project, sessions),
        other => panic!("unexpected local response: {other:?}"),
    };
    assert_eq!(
        restored.0.status(),
        crate::session::RuntimeProjectStatus::Active
    );
    assert_eq!(restored.1.len(), 1);
    assert_eq!(
        restored.1[0].status(),
        crate::session::SessionStatus::Parked
    );
    assert!(restored.1[0].active_provider_run_id().is_none());
}

#[test]
fn project_delete_cascades_when_last_session_removes_project_record() {
    let harness = LocalRouterTestHarness::new();
    let first = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-delete", "worktree-delete-1")
                .with_project_selection(SessionProjectSelection::New),
        ))
        .expect("named project session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    let second = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-delete", "worktree-delete-2")
                .with_project_selection(SessionProjectSelection::Existing {
                    project_id: first.project_id().to_string(),
                }),
        ))
        .expect("second project session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };

    let (project, sessions) = match harness
        .dispatch(LocalDaemonRequest::DeleteProject(DeleteProjectRequest {
            project_id: first.project_id().to_string(),
        }))
        .expect("project should delete")
    {
        LocalDaemonResponse::ProjectDeleted { project, sessions } => (project, sessions),
        other => panic!("unexpected local response: {other:?}"),
    };
    assert_eq!(project.id(), first.project_id());
    assert_eq!(sessions.len(), 2);
    assert!(sessions.iter().any(|session| session.id() == first.id()));
    assert!(sessions.iter().any(|session| session.id() == second.id()));

    let projects = match harness
        .dispatch(LocalDaemonRequest::ListProjects(ListProjectsRequest {
            include_archived: true,
        }))
        .expect("projects should list")
    {
        LocalDaemonResponse::ProjectsListed { projects } => projects,
        other => panic!("unexpected local response: {other:?}"),
    };
    assert!(projects.is_empty());
}

#[test]
fn project_requests_enforce_owner_and_named_numbering() {
    let harness = LocalRouterTestHarness::new();
    let create = |harness: &LocalRouterTestHarness| match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-numbering", "worktree-numbering")
                .with_project_selection(SessionProjectSelection::New),
        ))
        .expect("named project session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    let first = create(&harness);
    let second = create(&harness);
    let projects = match harness
        .dispatch(LocalDaemonRequest::ListProjects(ListProjectsRequest {
            include_archived: true,
        }))
        .expect("projects should list")
    {
        LocalDaemonResponse::ProjectsListed { projects } => projects,
        other => panic!("unexpected local response: {other:?}"),
    };
    assert_eq!(
        projects
            .iter()
            .find(|project| project.id() == first.project_id())
            .map(|project| project.name()),
        Some("Project-1")
    );
    assert_eq!(
        projects
            .iter()
            .find(|project| project.id() == second.project_id())
            .map(|project| project.name()),
        Some("Project-2")
    );

    let error = harness
        .dispatch_as_user(
            "another-user",
            LocalDaemonRequest::RenameProject(RenameProjectRequest {
                project_id: first.project_id().to_string(),
                name: "not-owned".to_string(),
            }),
        )
        .expect_err("non-owner project mutation should fail");
    assert!(error.to_string().contains("does not own project"));
}

#[test]
fn project_workspace_membership_updates_through_the_local_api() {
    let harness = LocalRouterTestHarness::new();
    let first = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-primary", "worktree-primary")
                .with_project_selection(SessionProjectSelection::New),
        ))
        .expect("named project session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };

    let project = match harness
        .dispatch(LocalDaemonRequest::UpdateProjectWorkspaces(
            UpdateProjectWorkspacesRequest {
                project_id: first.project_id().to_string(),
                workspace_ids: vec![
                    "workspace-primary".to_string(),
                    "workspace-supporting".to_string(),
                ],
            },
        ))
        .expect("project Workspaces should update")
    {
        LocalDaemonResponse::ProjectWorkspacesUpdated { project } => project,
        other => panic!("unexpected local response: {other:?}"),
    };
    assert_eq!(
        project.workspace_ids(),
        &["workspace-primary", "workspace-supporting"]
    );

    let supporting = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-supporting", "worktree-supporting")
                .with_project_selection(SessionProjectSelection::Existing {
                    project_id: first.project_id().to_string(),
                }),
        ))
        .expect("supporting Workspace should create a session in the Project")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    assert_eq!(supporting.workspace_id(), "workspace-supporting");
    assert_eq!(supporting.project_id(), first.project_id());
}

#[test]
fn archived_default_project_rejects_default_session_creation_until_restored() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-default-archive", "worktree-default-archive"),
        ))
        .expect("default project session should create")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    harness
        .dispatch(LocalDaemonRequest::ArchiveProject(ArchiveProjectRequest {
            project_id: session.project_id().to_string(),
        }))
        .expect("default project should archive while idle");

    let error = harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-default-archive", "worktree-default-archive-2"),
        ))
        .expect_err("archived default project should reject session creation");
    assert!(error
        .to_string()
        .contains("restore it before creating a session"));
}

#[test]
fn local_request_api_supports_session_attach_and_end() {
    let harness = LocalRouterTestHarness::new();
    let (session, _default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };

    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let detached = match harness
        .dispatch(LocalDaemonRequest::DetachFromSession(
            DetachFromSessionRequest {
                attachment_id: attachment.id().to_string(),
            },
        ))
        .expect("detach should succeed")
    {
        LocalDaemonResponse::SessionDetached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let ended = match harness
        .dispatch(LocalDaemonRequest::EndSession(EndSessionRequest {
            session_id: session.id().to_string(),
        }))
        .expect("end session should succeed")
    {
        LocalDaemonResponse::SessionEnded { session } => session,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(detached.id(), attachment.id());
    assert_eq!(ended.id(), session.id());
    harness.with_app(|app| {
        assert!(app.attachments().get_attachment(detached.id()).is_err());
    });
}

#[test]
fn session_attach_clears_a_missing_active_provider_run() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-stale-run", "worktree-stale-run"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };

    harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-after-worker-settlement".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("initial attachment should succeed");
    harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            account_profile: None,
            session_id: session.id().to_string(),
            alias: Some("remote-worker".to_string()),
            provider: Some("opencode".to_string()),
            model: Some("kimi-k2.6".to_string()),
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        }))
        .expect("second agent should make stale-run recovery exercise the multi-agent path");
    harness.with_app_mut(|app| {
        app.sessions_mut()
            .set_active_provider_run(session.id(), Some("provider-run-missing".to_string()))
            .expect("stale pointer should be installed");
        let stale = app
            .sessions()
            .get_session(session.id())
            .expect("session should remain available");
        assert_eq!(stale.active_provider_run_id(), Some("provider-run-missing"));
    });

    harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-after-worker-settlement".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should recover from a stale provider-run pointer");

    let attached = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("session state should load")
    {
        LocalDaemonResponse::SessionState { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(attached.active_provider_run_id(), None);
}

#[test]
fn session_attach_clears_a_projected_leased_run_for_an_unfocused_agent() {
    let harness = LocalRouterTestHarness::new();
    let (session, focused_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-projected-run", "worktree-projected-run"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-after-projected-worker-run".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("initial attachment should succeed");
    let remote_agent = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            account_profile: None,
            session_id: session.id().to_string(),
            alias: Some("remote-worker".to_string()),
            provider: Some("codex".to_string()),
            model: Some("gpt-5.6-sol".to_string()),
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        }))
        .expect("second agent should spawn")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };
    harness.with_app_mut(|app| {
        app.agents()
            .bind_remote_execution(
                remote_agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel".to_string(),
                    worker_machine_id: "worker-machine".to_string(),
                    execution_lease_id: "lease-1".to_string(),
                    leased_agent_id: "leased-agent-1".to_string(),
                    active_worker_provider_run_id: Some("provider-run-1".to_string()),
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .expect("agent should bind to remote execution");
        app.sessions_mut()
            .set_focused_agent(session.id(), Some(focused_agent.id().to_string()))
            .expect("first agent should be focused");
        let request =
            LaunchProviderRequest::new(session.id(), "codex", "codex", "default", "gpt-5.6-sol")
                .with_agent_id(remote_agent.id());
        let mut worker_run = RuntimeProviderRun::new(
            "provider-run-1",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "codex".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        );
        worker_run.mark_running();
        let (_, projected_run) = worker_run.project_leased_for_home_agent(
            "leased-agent-1",
            session.id(),
            remote_agent.id(),
        );
        let projected_run_id = projected_run.id().to_string();
        app.update_provider_run_projection(projected_run);
        app.sessions_mut()
            .set_active_provider_run(session.id(), Some(projected_run_id))
            .expect("projected run pointer should be installed");
    });

    harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-after-projected-worker-run".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should not try to park a projected worker run locally");

    let attached = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("session state should load")
    {
        LocalDaemonResponse::SessionState { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(attached.active_provider_run_id(), None);
}

#[test]
fn local_request_api_resolves_and_deletes_sessions_by_ref() {
    let harness = LocalRouterTestHarness::new();
    let (session, _agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1").with_alias("main"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let workspace_id = session.workspace_id().to_string();

    let resolved = match harness
        .dispatch(LocalDaemonRequest::ResolveSession(ResolveSessionRequest {
            session_ref: "mai".to_string(),
            workspace_id: Some(workspace_id.clone()),
        }))
        .expect("resolve should succeed")
    {
        LocalDaemonResponse::SessionResolved { session } => session,
        _ => panic!("unexpected local response"),
    };

    let deleted = match harness
        .dispatch(LocalDaemonRequest::DeleteSession(DeleteSessionRequest {
            session_ref: session.id()[..8].to_string(),
            workspace_id: Some(workspace_id.clone()),
        }))
        .expect("delete should succeed")
    {
        LocalDaemonResponse::SessionDeleted { session } => session,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(resolved.id(), session.id());
    assert_eq!(deleted.id(), session.id());
    assert_eq!(deleted.alias(), Some("main"));
    assert_eq!(deleted.status(), crate::session::SessionStatus::Ended);
    assert!(matches!(
        harness.dispatch(LocalDaemonRequest::ResolveSession(ResolveSessionRequest {
            session_ref: "main".to_string(),
            workspace_id: Some(workspace_id),
        })),
        Err(DaemonError::SessionNotFound { .. })
    ));
    let listed = match harness
        .dispatch(LocalDaemonRequest::ListSessions(ListSessionsRequest))
        .expect("list should succeed")
    {
        LocalDaemonResponse::SessionsListed { sessions } => sessions,
        _ => panic!("unexpected local response"),
    };
    assert!(listed.is_empty());
}

#[test]
fn local_request_api_manages_session_invites_and_members() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };

    let session_id = session.id().to_string();
    let invite_record = match harness
        .dispatch(LocalDaemonRequest::CreateSessionInvite(
            CreateSessionInviteRequest {
                session_id: session_id.clone(),
                expires_in_ms: None,
                max_uses: Some(1),
                collaboration_level: crate::session::CollaborationLevel::Private,
            },
        ))
        .expect("session invite create should succeed")
    {
        LocalDaemonResponse::SessionInviteCreated { invite, session } => {
            assert_eq!(session.id(), session_id);
            invite
        }
        _ => panic!("unexpected local response"),
    };

    let joined = match harness
        .dispatch(LocalDaemonRequest::JoinSessionInvite(
            JoinSessionInviteRequest {
                invite_token: invite_record.invite_token.clone(),
                user_id: "user-2".to_string(),
            },
        ))
        .expect("session invite join should succeed")
    {
        LocalDaemonResponse::SessionInviteJoined { member, session } => {
            assert!(session.has_member("user-2"));
            member
        }
        _ => panic!("unexpected local response"),
    };
    assert_eq!(joined.user_id(), "user-2");

    let (members, invites) = match harness
        .dispatch(LocalDaemonRequest::ListSessionMembers(
            ListSessionMembersRequest {
                session_id: session_id.clone(),
            },
        ))
        .expect("session members should list")
    {
        LocalDaemonResponse::SessionMembersListed { members, invites } => (members, invites),
        _ => panic!("unexpected local response"),
    };
    assert_eq!(members.len(), 2);
    assert_eq!(invites.len(), 1);
    assert_eq!(invites[0].used_count(), 1);

    let revoked = match harness
        .dispatch(LocalDaemonRequest::RevokeSessionInvite(
            RevokeSessionInviteRequest {
                session_id,
                invite_ref: invite_record.invite.invite_id().to_string(),
            },
        ))
        .expect("session invite revoke should succeed")
    {
        LocalDaemonResponse::SessionInviteRevoked { invite, .. } => invite,
        _ => panic!("unexpected local response"),
    };
    assert!(revoked.is_revoked());
}

#[test]
fn local_request_api_aliases_sessions() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };

    let aliased = match harness
        .dispatch(LocalDaemonRequest::AliasSession(AliasSessionRequest {
            session_id: session.id().to_string(),
            alias: "alpha".to_string(),
        }))
        .expect("alias should succeed")
    {
        LocalDaemonResponse::SessionAliased { session } => session,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(aliased.alias(), Some("alpha"));

    let resolved = match harness
        .dispatch(LocalDaemonRequest::ResolveSession(ResolveSessionRequest {
            session_ref: "alpha".to_string(),
            workspace_id: Some(aliased.workspace_id().to_string()),
        }))
        .expect("alias resolve should succeed")
    {
        LocalDaemonResponse::SessionResolved { session } => session,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(resolved.id(), session.id());
}

#[test]
fn local_request_api_spawns_and_focuses_agents() {
    let harness = LocalRouterTestHarness::new();
    let (session, default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };

    let spawned = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            account_profile: None,
            session_id: session.id().to_string(),
            alias: Some("  reviewer  ".to_string()),
            provider: Some("opencode".to_string()),
            model: Some("openai/gpt-5.4".to_string()),
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        }))
        .expect("spawn should succeed")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(spawned.alias(), Some("reviewer"));

    let (session_state, agent_activity) = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("session state should load")
    {
        LocalDaemonResponse::SessionState {
            session,
            agent_activity,
            ..
        } => (session, agent_activity),
        _ => panic!("unexpected local response"),
    };

    assert_eq!(session_state.agents().len(), 2);
    assert_eq!(
        agent_activity
            .get(default_agent.id())
            .expect("default agent activity should be projected")
            .status,
        crate::runtime::projection::AgentRuntimeStatus::Idle
    );
    assert_eq!(
        agent_activity
            .get(spawned.id())
            .expect("spawned agent activity should be projected")
            .status,
        crate::runtime::projection::AgentRuntimeStatus::Idle
    );
    assert_eq!(session_state.focused_agent_id(), Some(spawned.id()));
    assert_eq!(
        session_state
            .agents()
            .iter()
            .map(|agent| agent.id())
            .collect::<Vec<_>>(),
        vec![default_agent.id(), spawned.id()]
    );
    assert_eq!(
        session_state
            .agents()
            .iter()
            .find(|agent| agent.id() == default_agent.id())
            .expect("default agent should still exist")
            .state(),
        crate::agent::AgentState::Idle
    );
    assert_eq!(
        session_state
            .agents()
            .iter()
            .find(|agent| agent.id() == spawned.id())
            .expect("spawned agent should exist")
            .state(),
        crate::agent::AgentState::Focused
    );

    let renamed = match harness
        .dispatch(LocalDaemonRequest::AliasAgent(AliasAgentRequest {
            session_id: session.id().to_string(),
            agent_id: spawned.id().to_string(),
            alias: "web-reviewer".to_string(),
        }))
        .expect("agent alias update should succeed")
    {
        LocalDaemonResponse::AgentAliased { agent, session } => {
            assert_eq!(
                session
                    .agents()
                    .iter()
                    .find(|entry| entry.id() == spawned.id())
                    .and_then(|entry| entry.alias()),
                Some("web-reviewer")
            );
            agent
        }
        _ => panic!("unexpected local response"),
    };

    assert_eq!(renamed.alias(), Some("web-reviewer"));

    let alias_conflict = harness
        .dispatch(LocalDaemonRequest::AliasAgent(AliasAgentRequest {
            session_id: session.id().to_string(),
            agent_id: default_agent.id().to_string(),
            alias: "  WEB-REVIEWER  ".to_string(),
        }))
        .expect_err("agent aliases must remain unique within a session");
    assert!(matches!(
        alias_conflict,
        DaemonError::AgentAliasConflict {
            session_id: ref conflict_session_id,
            alias: ref conflict_alias,
        } if conflict_session_id == session.id() && conflict_alias == "WEB-REVIEWER"
    ));
    let reference_conflict = harness
        .dispatch(LocalDaemonRequest::AliasAgent(AliasAgentRequest {
            session_id: session.id().to_string(),
            agent_id: default_agent.id().to_string(),
            alias: spawned.agent_ref().to_string(),
        }))
        .expect_err("agent aliases must not shadow another agent reference");
    assert!(matches!(
        reference_conflict,
        DaemonError::AgentAliasConflict { .. }
    ));
    harness.with_app(|app| {
        assert_eq!(
            app.agents()
                .get_agent(default_agent.id())
                .expect("default agent should remain available")
                .alias(),
            default_agent.alias(),
        );
    });

    let profiled = match harness
        .dispatch(LocalDaemonRequest::UpdateAgentProfile(
            UpdateAgentProfileRequest {
                session_id: session.id().to_string(),
                agent_id: spawned.id().to_string(),
                provider: Some("codex".to_string()),
                account_profile: None,
                model: Some("gpt-5.4".to_string()),
                effort: Some("low".to_string()),
                clear_effort: false,
            },
        ))
        .expect("agent profile update should succeed")
    {
        LocalDaemonResponse::AgentProfileUpdated { agent, session } => {
            let entry = session
                .agents()
                .iter()
                .find(|entry| entry.id() == spawned.id())
                .expect("updated agent should remain in session snapshot");
            assert_eq!(entry.provider(), "codex");
            assert_eq!(entry.model(), Some("gpt-5.4"));
            assert_eq!(entry.effort(), Some("low"));
            agent
        }
        _ => panic!("unexpected local response"),
    };

    assert_eq!(profiled.provider(), "codex");
    assert_eq!(profiled.model(), Some("gpt-5.4"));
    assert_eq!(profiled.effort(), Some("low"));

    let cleared = match harness
        .dispatch(LocalDaemonRequest::UpdateAgentProfile(
            UpdateAgentProfileRequest {
                session_id: session.id().to_string(),
                agent_id: spawned.id().to_string(),
                provider: None,
                account_profile: None,
                model: None,
                effort: None,
                clear_effort: true,
            },
        ))
        .expect("agent profile clear should succeed")
    {
        LocalDaemonResponse::AgentProfileUpdated { agent, session } => {
            let entry = session
                .agents()
                .iter()
                .find(|entry| entry.id() == spawned.id())
                .expect("updated agent should remain in session snapshot");
            assert_eq!(entry.effort(), None);
            agent
        }
        _ => panic!("unexpected local response"),
    };

    assert_eq!(cleared.provider(), "codex");
    assert_eq!(cleared.model(), Some("gpt-5.4"));
    assert_eq!(cleared.effort(), None);

    let relocated = match harness
        .dispatch(LocalDaemonRequest::UpdateAgentConfig(
            UpdateAgentConfigRequest {
                session_id: session.id().to_string(),
                agent_id: spawned.id().to_string(),
                execution_mode: None,
                clear_execution_mode: false,
                permission_level: None,
                clear_permission_level: false,
                workspace_id: Some("/repo/feature".to_string()),
                clear_workspace_id: false,
                worktree_id: Some("/repo/feature-wt".to_string()),
                clear_worktree_id: false,
            },
        ))
        .expect("agent workspace update should succeed")
    {
        LocalDaemonResponse::AgentConfigUpdated { agent, session } => {
            let entry = session
                .agents()
                .iter()
                .find(|entry| entry.id() == spawned.id())
                .expect("updated agent should remain in session snapshot");
            assert_eq!(entry.workspace_id(), Some("/repo/feature"));
            assert_eq!(entry.worktree_id(), Some("/repo/feature-wt"));
            agent
        }
        _ => panic!("unexpected local response"),
    };

    assert_eq!(relocated.workspace_id(), Some("/repo/feature"));
    assert_eq!(relocated.worktree_id(), Some("/repo/feature-wt"));

    let focused_default = match harness
        .dispatch(LocalDaemonRequest::FocusAgent(FocusAgentRequest {
            session_id: session.id().to_string(),
            agent_id: default_agent.id().to_string(),
        }))
        .expect("focus should succeed")
    {
        LocalDaemonResponse::AgentFocused { agent } => agent,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(focused_default.id(), default_agent.id());

    let cycled = match harness
        .dispatch(LocalDaemonRequest::CycleAgentFocus(
            CycleAgentFocusRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("cycle should succeed")
    {
        LocalDaemonResponse::AgentFocusCycled { agent } => {
            agent.expect("cycle should return a focused agent")
        }
        _ => panic!("unexpected local response"),
    };

    assert_eq!(cycled.id(), spawned.id());

    let listed = match harness
        .dispatch(LocalDaemonRequest::ListAgents(ListAgentsRequest {
            session_id: session.id().to_string(),
        }))
        .expect("list should succeed")
    {
        LocalDaemonResponse::AgentsListed { agents } => agents,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(listed.len(), 2);
    assert_eq!(
        listed.iter().map(|agent| agent.id()).collect::<Vec<_>>(),
        vec![default_agent.id(), spawned.id()]
    );
    assert_eq!(
        listed
            .iter()
            .find(|agent| agent.id() == spawned.id())
            .expect("spawned agent should be listed")
            .state(),
        crate::agent::AgentState::Focused
    );
}

#[test]
fn same_agent_profile_update_keeps_active_provider_run() {
    let harness = LocalRouterTestHarness::new();
    let (session, agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let active_run_id = harness.with_app_mut(|app| {
        crate::test_support::authenticate_provider_account(
            &app.provider_account_profile_registry(),
            crate::session::DEFAULT_LOCAL_USER_ID,
            "codex",
            "default",
        )
        .expect("synthetic provider account should be authenticated");
        app.launch_provider(
            LaunchProviderRequest::new(session.id(), "dev-stub", "codex", "default", "gpt-5.4")
                .with_agent_id(agent.id()),
        )
        .expect("provider launch should succeed")
        .id()
        .to_string()
    });

    let updated_session = match harness
        .dispatch(LocalDaemonRequest::UpdateAgentProfile(
            UpdateAgentProfileRequest {
                session_id: session.id().to_string(),
                agent_id: agent.id().to_string(),
                provider: Some("codex".to_string()),
                account_profile: None,
                model: Some("gpt-5.4".to_string()),
                effort: None,
                clear_effort: false,
            },
        ))
        .expect("same profile update should succeed")
    {
        LocalDaemonResponse::AgentProfileUpdated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(
        updated_session.active_provider_run_id(),
        Some(active_run_id.as_str())
    );
    let run = harness.with_app(|app| {
        app.providers()
            .get_run(&active_run_id)
            .expect("active provider run should remain")
    });
    assert_eq!(run.state(), crate::provider::ProviderRunState::Running);
}

#[test]
fn detaching_one_attachment_keeps_the_session_open_for_others() {
    let harness = LocalRouterTestHarness::new();
    let (session, _default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };

    let first = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("first attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let second = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-2".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("second attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let detached = match harness
        .dispatch(LocalDaemonRequest::DetachFromSession(
            DetachFromSessionRequest {
                attachment_id: first.id().to_string(),
            },
        ))
        .expect("detach should succeed")
    {
        LocalDaemonResponse::SessionDetached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let state = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("state request should succeed")
    {
        LocalDaemonResponse::SessionState { session, .. } => session,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(detached.id(), first.id());
    assert_eq!(state.status().to_string(), "created");
    assert_eq!(state.attachment_ids().len(), 1);
    assert!(state.has_attachment(second.id()));
    assert!(harness.with_app(|app| app.attachments().get_attachment(second.id()).is_ok()));
}

#[test]
fn attaching_the_same_client_replaces_its_stale_attachment() {
    let harness = LocalRouterTestHarness::new();
    let (session, _default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };

    let first = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("first attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let second = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("second attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let state = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("state request should succeed")
    {
        LocalDaemonResponse::SessionState { session, .. } => session,
        _ => panic!("unexpected local response"),
    };

    assert_ne!(first.id(), second.id());
    assert_eq!(state.attachment_ids().len(), 1);
    assert!(state.has_attachment(second.id()));
    assert!(harness.with_app(|app| app.attachments().get_attachment(first.id()).is_err()));
}
