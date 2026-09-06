use super::*;

#[test]
fn lifecycle_preserves_identity_and_reset_invalidates_runtime_handles() {
    let viewport = CanonicalViewport::new(1440, 900, 2, 2880, 1800).unwrap();
    let mut environment = RoomEnvironment::new("room-1", "environment-1", viewport).unwrap();

    assert_eq!(
        environment.snapshot().lifecycle,
        EnvironmentLifecycle::Stopped
    );
    assert_eq!(environment.snapshot().runtime_generation, 1);

    environment.start_runtime().unwrap();
    environment
        .transition_to(EnvironmentLifecycle::Ready)
        .unwrap();
    environment
        .transition_to(EnvironmentLifecycle::Stopping)
        .unwrap();
    environment
        .transition_to(EnvironmentLifecycle::Stopped)
        .unwrap();

    assert_eq!(environment.snapshot().environment_id, "environment-1");
    assert_eq!(environment.snapshot().runtime_generation, 1);

    environment.reset_runtime().unwrap();
    assert_eq!(environment.snapshot().environment_id, "environment-1");
    assert_eq!(environment.snapshot().runtime_generation, 2);
    assert_eq!(
        environment.snapshot().lifecycle,
        EnvironmentLifecycle::Starting
    );
}

#[test]
fn lifecycle_rejects_unsafe_transitions_and_invalid_viewports() {
    assert_eq!(
        CanonicalViewport::new(0, 900, 1, 1440, 900),
        Err(EnvironmentError::InvalidViewport)
    );

    let viewport = CanonicalViewport::new(1440, 900, 1, 1440, 900).unwrap();
    let mut environment = RoomEnvironment::new("room-1", "environment-1", viewport).unwrap();
    assert_eq!(
        environment.transition_to(EnvironmentLifecycle::Ready),
        Err(EnvironmentError::InvalidLifecycleTransition {
            from: EnvironmentLifecycle::Stopped,
            to: EnvironmentLifecycle::Ready,
        })
    );
    assert_eq!(
        environment.transition_to(EnvironmentLifecycle::Starting),
        Err(EnvironmentError::InvalidLifecycleTransition {
            from: EnvironmentLifecycle::Stopped,
            to: EnvironmentLifecycle::Starting,
        })
    );
}

#[test]
fn tab_identity_survives_reconciliation_and_navigation_invalidates_old_references() {
    let mut environment = ready_environment();
    let tab_id = environment
        .register_or_reconcile_tab("controller-target-1", "https://example.test", "Example")
        .unwrap();
    assert_eq!(tab_id, "tab-1");

    let reconciled_id = environment
        .register_or_reconcile_tab("controller-target-1", "https://example.test", "Example")
        .unwrap();
    assert_eq!(reconciled_id, tab_id);
    assert_eq!(environment.snapshot().tabs.len(), 1);

    environment
        .validate_tab_reference(1, &tab_id, 1)
        .expect("initial reference should be current");
    environment
        .record_navigation(&tab_id, "https://example.test/inbox", "Inbox")
        .unwrap();
    assert_eq!(environment.snapshot().tabs[0].document_revision, 2);
    assert_eq!(
        environment.validate_tab_reference(1, &tab_id, 1),
        Err(EnvironmentError::StaleDocumentRevision {
            tab_id: "tab-1".to_string(),
            expected: 2,
            actual: 1,
        })
    );
}

#[test]
fn closing_and_resetting_retire_runtime_tab_identity() {
    let mut environment = ready_environment();
    let tab_id = environment
        .register_or_reconcile_tab("controller-target-1", "https://example.test", "Example")
        .unwrap();
    environment.close_tab(&tab_id).unwrap();
    let replacement_id = environment
        .register_or_reconcile_tab("controller-target-1", "https://example.test", "Example")
        .unwrap();
    assert_eq!(replacement_id, "tab-2");

    environment
        .transition_to(EnvironmentLifecycle::Stopping)
        .unwrap();
    environment
        .transition_to(EnvironmentLifecycle::Stopped)
        .unwrap();
    environment.reset_runtime().unwrap();
    assert!(environment.snapshot().tabs.is_empty());
    assert_eq!(
        environment.validate_tab_reference(1, &replacement_id, 1),
        Err(EnvironmentError::StaleRuntimeGeneration {
            expected: 2,
            actual: 1,
        })
    );
}

#[test]
fn viewport_updates_are_actor_attributed_and_revision_guarded() {
    let mut environment = ready_environment();
    environment
        .register_actor(EnvironmentActor::new(
            "agent-1",
            EnvironmentActorKind::Agent,
            "Mara",
        ))
        .unwrap();
    let replacement = CanonicalViewport::new(1280, 720, 2, 2560, 1440).unwrap();

    environment
        .update_viewport("agent-1", 1, replacement)
        .expect("an actor may resize while no human owns input");
    assert_eq!(environment.snapshot().viewport.revision, 2);
    assert_eq!(
        environment.snapshot().viewport.last_actor_id.as_deref(),
        Some("agent-1")
    );

    let stale_replacement = CanonicalViewport::new(1024, 768, 1, 1024, 768).unwrap();
    assert_eq!(
        environment.update_viewport("agent-1", 1, stale_replacement),
        Err(EnvironmentError::StaleViewportRevision {
            expected: 2,
            actual: 1,
        })
    );
}

#[test]
fn viewport_updates_reject_unknown_actors() {
    let mut environment = ready_environment();
    let replacement = CanonicalViewport::new(1280, 720, 1, 1280, 720).unwrap();
    assert_eq!(
        environment.update_viewport("missing", 1, replacement),
        Err(EnvironmentError::UnknownActor {
            actor_id: "missing".to_string(),
        })
    );
}

#[test]
fn observations_run_concurrently_and_mutations_serialize_per_target() {
    let mut environment = ready_environment_with_agent();
    let tab_a = environment
        .register_or_reconcile_tab("target-a", "https://a.test", "A")
        .unwrap();
    let tab_b = environment
        .register_or_reconcile_tab("target-b", "https://b.test", "B")
        .unwrap();

    for _ in 0..2 {
        assert!(matches!(
            environment.submit_action(EnvironmentActionRequest::browser_observation(
                "agent-1", 1, "snapshot", &tab_a, 1,
            )),
            Ok(ActionAdmission::Accepted { .. })
        ));
    }

    let action_a = accepted_action_id(
        environment
            .submit_action(EnvironmentActionRequest::browser_mutation(
                "agent-1", 1, "click", &tab_a, 1,
            ))
            .unwrap(),
    );
    let action_b = accepted_action_id(
        environment
            .submit_action(EnvironmentActionRequest::browser_mutation(
                "agent-1", 1, "fill", &tab_b, 1,
            ))
            .unwrap(),
    );
    assert_ne!(action_a, action_b);
    assert!(matches!(
        environment
            .submit_action(EnvironmentActionRequest::browser_mutation(
                "agent-1", 1, "second-click", &tab_a, 1,
            ))
            .unwrap(),
        ActionAdmission::RejectedBusy {
            target: InputTarget::BrowserTab(ref tab_id),
            ..
        } if tab_id == &tab_a
    ));

    environment
        .finish_action(&action_a, EnvironmentActionTerminal::Completed)
        .unwrap();
    assert!(matches!(
        environment.submit_action(EnvironmentActionRequest::browser_mutation(
            "agent-1",
            1,
            "second-click",
            &tab_a,
            1,
        )),
        Ok(ActionAdmission::Accepted { .. })
    ));
}

#[test]
fn computer_mutation_reserves_desktop_before_the_focused_tab() {
    let mut environment = ready_environment_with_agent();
    let tab_id = environment
        .register_or_reconcile_tab("target-a", "https://a.test", "A")
        .unwrap();
    let action_id = accepted_action_id(
        environment
            .submit_action(EnvironmentActionRequest::computer_mutation(
                "agent-1",
                1,
                "pointer-click",
                Some(&tab_id),
            ))
            .unwrap(),
    );
    let action = environment
        .snapshot()
        .actions
        .into_iter()
        .find(|action| action.action_id == action_id)
        .unwrap();
    assert_eq!(
        action.targets,
        vec![
            InputTarget::Desktop,
            InputTarget::BrowserTab(tab_id.clone())
        ]
    );

    assert!(matches!(
        environment
            .submit_action(EnvironmentActionRequest::browser_mutation(
                "agent-1", 1, "click", &tab_id, 1,
            ))
            .unwrap(),
        ActionAdmission::RejectedBusy { .. }
    ));
}

#[test]
fn human_takeover_waits_for_the_agent_action_to_be_terminal() {
    let mut environment = ready_environment_with_agent();
    environment
        .register_actor(EnvironmentActor::new(
            "user-1",
            EnvironmentActorKind::Human,
            "Miguel",
        ))
        .unwrap();
    let tab_id = environment
        .register_or_reconcile_tab("target-a", "https://a.test", "A")
        .unwrap();
    let action_id = accepted_action_id(
        environment
            .submit_action(EnvironmentActionRequest::browser_mutation(
                "agent-1", 1, "fill", &tab_id, 1,
            ))
            .unwrap(),
    );

    assert_eq!(
        environment
            .request_takeover("user-1", InputTarget::BrowserTab(tab_id.clone()))
            .unwrap(),
        TakeoverOutcome::CancellationRequired {
            action_ids: vec![action_id.clone()],
        }
    );
    assert!(environment.snapshot().input_ownership.is_empty());
    assert!(matches!(
        environment
            .submit_action(EnvironmentActionRequest::browser_mutation(
                "agent-1", 1, "click", &tab_id, 1,
            ))
            .unwrap(),
        ActionAdmission::RejectedTakeover {
            human_actor_id,
            ..
        } if human_actor_id == "user-1"
    ));

    environment
        .finish_action(&action_id, EnvironmentActionTerminal::Cancelled)
        .unwrap();
    assert_eq!(
        environment.snapshot().input_ownership,
        vec![InputOwnership {
            target: InputTarget::BrowserTab(tab_id.clone()),
            actor_id: "user-1".to_string(),
        }]
    );
    assert_eq!(
        environment
            .request_takeover("user-1", InputTarget::BrowserTab(tab_id.clone()))
            .unwrap(),
        TakeoverOutcome::Granted
    );
    environment
        .release_input("user-1", &InputTarget::BrowserTab(tab_id))
        .unwrap();
    assert!(environment.snapshot().input_ownership.is_empty());
}

#[test]
fn desktop_owner_exclusively_controls_viewport_updates() {
    let mut environment = ready_environment_with_agent();
    environment
        .register_actor(EnvironmentActor::new(
            "user-1",
            EnvironmentActorKind::Human,
            "Miguel",
        ))
        .unwrap();
    assert_eq!(
        environment.request_takeover("user-1", InputTarget::Desktop),
        Ok(TakeoverOutcome::Granted)
    );

    let agent_viewport = CanonicalViewport::new(1280, 720, 1, 1280, 720).unwrap();
    assert_eq!(
        environment.update_viewport("agent-1", 1, agent_viewport),
        Err(EnvironmentError::InputOwnedByAnotherActor {
            target: InputTarget::Desktop,
            actor_id: "user-1".to_string(),
        })
    );
    let human_viewport = CanonicalViewport::new(1280, 720, 1, 1280, 720).unwrap();
    environment
        .update_viewport("user-1", 1, human_viewport)
        .unwrap();
}

#[test]
fn reconnect_replays_ordered_events_or_requires_a_snapshot_after_a_gap() {
    let viewport = CanonicalViewport::new(1440, 900, 1, 1440, 900).unwrap();
    let mut environment =
        RoomEnvironment::new_with_event_capacity("room-1", "environment-1", viewport, 3).unwrap();
    environment.start_runtime().unwrap();
    environment
        .transition_to(EnvironmentLifecycle::Ready)
        .unwrap();
    environment
        .register_actor(EnvironmentActor::new(
            "agent-1",
            EnvironmentActorKind::Agent,
            "Mara",
        ))
        .unwrap();
    let cursor = environment.snapshot().event_cursor;

    let tab_id = environment
        .register_or_reconcile_tab("target-a", "https://a.test", "A")
        .unwrap();
    let action_id = accepted_action_id(
        environment
            .submit_action(EnvironmentActionRequest::browser_mutation(
                "agent-1", 1, "click", &tab_id, 1,
            ))
            .unwrap(),
    );
    environment
        .finish_action(&action_id, EnvironmentActionTerminal::Completed)
        .unwrap();

    let expected = environment.events_after(cursor);
    assert!(matches!(
        &expected,
        EnvironmentReplay::Events { events, next_cursor }
            if events.len() == 3
                && events.windows(2).all(|pair| pair[0].event_id + 1 == pair[1].event_id)
                && *next_cursor == environment.snapshot().event_cursor
    ));
    assert_eq!(environment.events_after(cursor), expected);
    assert!(matches!(
        environment.events_after(0),
        EnvironmentReplay::SnapshotRequired { snapshot }
            if snapshot.event_cursor == environment.snapshot().event_cursor
    ));
}

#[test]
fn process_loss_fails_only_running_actions_and_invalidates_runtime_handles() {
    let mut environment = ready_environment_with_agent();
    let tab_a = environment
        .register_or_reconcile_tab("target-a", "https://a.test", "A")
        .unwrap();
    let tab_b = environment
        .register_or_reconcile_tab("target-b", "https://b.test", "B")
        .unwrap();
    let completed_id = accepted_action_id(
        environment
            .submit_action(EnvironmentActionRequest::browser_mutation(
                "agent-1", 1, "click", &tab_a, 1,
            ))
            .unwrap(),
    );
    environment
        .finish_action(&completed_id, EnvironmentActionTerminal::Completed)
        .unwrap();
    let running_id = accepted_action_id(
        environment
            .submit_action(EnvironmentActionRequest::browser_mutation(
                "agent-1", 1, "fill", &tab_b, 1,
            ))
            .unwrap(),
    );

    let cursor = environment.snapshot().event_cursor;
    environment.invalidate_runtime_after_process_loss().unwrap();
    let snapshot = environment.snapshot();
    assert_eq!(snapshot.runtime_generation, 2);
    assert_eq!(snapshot.lifecycle, EnvironmentLifecycle::Starting);
    assert!(snapshot.tabs.is_empty());
    assert_eq!(
        snapshot
            .actions
            .iter()
            .find(|action| action.action_id == completed_id)
            .unwrap()
            .state,
        EnvironmentActionState::Completed
    );
    assert_eq!(
        snapshot
            .actions
            .iter()
            .find(|action| action.action_id == running_id)
            .unwrap()
            .state,
        EnvironmentActionState::Failed
    );
    assert!(matches!(
        environment.events_after(cursor),
        EnvironmentReplay::Events { events, .. }
            if matches!(events[events.len() - 2].kind, EnvironmentEventKind::RuntimeInvalidated)
                && matches!(
                    events[events.len() - 1].kind,
                    EnvironmentEventKind::LifecycleChanged {
                        lifecycle: EnvironmentLifecycle::Starting,
                    }
                )
    ));
}

#[test]
fn idempotency_reuses_the_original_action_and_rejects_conflicting_replays() {
    let mut environment = ready_environment_with_agent();
    let tab_id = environment
        .register_or_reconcile_tab("target-a", "https://a.test", "A")
        .unwrap();
    let request = EnvironmentActionRequest::browser_mutation("agent-1", 1, "send", &tab_id, 1)
        .with_idempotency_key("send-message-1");
    let action_id = accepted_action_id(environment.submit_action(request.clone()).unwrap());

    assert_eq!(
        environment.submit_action(request).unwrap(),
        ActionAdmission::Existing {
            action_id: action_id.clone(),
            state: EnvironmentActionState::Running,
        }
    );
    let conflicting =
        EnvironmentActionRequest::browser_mutation("agent-1", 1, "delete", &tab_id, 1)
            .with_idempotency_key("send-message-1");
    assert_eq!(
        environment.submit_action(conflicting),
        Err(EnvironmentError::IdempotencyConflict {
            idempotency_key: "send-message-1".to_string(),
        })
    );
}

#[test]
fn terminal_action_state_is_immutable() {
    let mut environment = ready_environment_with_agent();
    let action_id = accepted_action_id(
        environment
            .submit_action(EnvironmentActionRequest::computer_mutation(
                "agent-1",
                1,
                "key-chord",
                None,
            ))
            .unwrap(),
    );
    environment
        .finish_action(&action_id, EnvironmentActionTerminal::Completed)
        .unwrap();
    assert_eq!(
        environment.finish_action(&action_id, EnvironmentActionTerminal::Failed),
        Err(EnvironmentError::ActionAlreadyTerminal {
            action_id,
            state: EnvironmentActionState::Completed,
        })
    );
}

#[test]
fn restarting_a_stopped_runtime_invalidates_old_handles() {
    let mut environment = ready_environment();
    environment
        .register_or_reconcile_tab("target-a", "https://a.test", "A")
        .unwrap();
    environment
        .transition_to(EnvironmentLifecycle::Stopping)
        .unwrap();
    environment
        .transition_to(EnvironmentLifecycle::Stopped)
        .unwrap();

    environment.start_runtime().unwrap();
    assert_eq!(environment.snapshot().runtime_generation, 2);
    assert_eq!(
        environment.snapshot().lifecycle,
        EnvironmentLifecycle::Starting
    );
    assert!(environment.snapshot().tabs.is_empty());
}

#[test]
fn terminal_action_history_is_bounded_but_active_actions_are_retained() {
    let viewport = CanonicalViewport::new(1440, 900, 1, 1440, 900).unwrap();
    let mut environment =
        RoomEnvironment::new_with_event_capacity("room-1", "environment-1", viewport, 2).unwrap();
    environment.start_runtime().unwrap();
    environment
        .transition_to(EnvironmentLifecycle::Ready)
        .unwrap();
    environment
        .register_actor(EnvironmentActor::new(
            "agent-1",
            EnvironmentActorKind::Agent,
            "Mara",
        ))
        .unwrap();
    let tab_id = environment
        .register_or_reconcile_tab("target-a", "https://a.test", "A")
        .unwrap();

    let mut completed_ids = Vec::new();
    for sequence in 1..=3 {
        let action_id = accepted_action_id(
            environment
                .submit_action(
                    EnvironmentActionRequest::browser_mutation(
                        "agent-1",
                        1,
                        format!("click-{sequence}"),
                        &tab_id,
                        1,
                    )
                    .with_idempotency_key(format!("click-{sequence}")),
                )
                .unwrap(),
        );
        environment
            .finish_action(&action_id, EnvironmentActionTerminal::Completed)
            .unwrap();
        completed_ids.push(action_id);
    }
    let active_id = accepted_action_id(
        environment
            .submit_action(EnvironmentActionRequest::browser_observation(
                "agent-1", 1, "snapshot", &tab_id, 1,
            ))
            .unwrap(),
    );

    let retained_ids: Vec<_> = environment
        .snapshot()
        .actions
        .into_iter()
        .map(|action| action.action_id)
        .collect();
    assert_eq!(
        retained_ids,
        vec![
            completed_ids[1].clone(),
            completed_ids[2].clone(),
            active_id,
        ]
    );
}

#[test]
fn one_room_cannot_acquire_a_second_environment() {
    let mut environments = RoomEnvironmentRegistry::new();
    let first = environments
        .create(
            "room-1",
            "environment-1",
            CanonicalViewport::new(1280, 800, 1, 1280, 800).unwrap(),
        )
        .expect("the Room should acquire its first Environment");
    assert_eq!(first.session_id, "room-1");
    assert_eq!(first.environment_id, "environment-1");

    let error = environments
        .create(
            "room-1",
            "environment-2",
            CanonicalViewport::new(1440, 900, 1, 1440, 900).unwrap(),
        )
        .expect_err("the Room must not acquire a second Environment implicitly");
    assert_eq!(
        error,
        EnvironmentError::EnvironmentAlreadyExists {
            session_id: "room-1".to_string(),
            environment_id: "environment-1".to_string(),
        }
    );
    assert_eq!(
        environments
            .snapshot("room-1")
            .expect("the original Environment should remain")
            .environment_id,
        "environment-1"
    );
}

#[test]
fn removing_a_room_retires_its_environment_identity() {
    let mut environments = RoomEnvironmentRegistry::new();
    environments
        .create(
            "room-1",
            "environment-1",
            CanonicalViewport::new(1280, 800, 1, 1280, 800).unwrap(),
        )
        .expect("the Environment should be created");

    let retired = environments
        .remove("room-1")
        .expect("the Environment should be retired with its Room");
    assert_eq!(retired.environment_id, "environment-1");
    assert_eq!(
        environments
            .snapshot("room-1")
            .expect_err("a retired Environment must not remain addressable"),
        EnvironmentError::EnvironmentNotFound {
            session_id: "room-1".to_string(),
        }
    );
}

#[test]
fn idempotency_survives_generation_change_without_repeating_work() {
    let mut environment = ready_environment_with_agent();
    let tab_id = environment
        .register_or_reconcile_tab("target-a", "https://a.test", "A")
        .unwrap();
    let request = EnvironmentActionRequest::browser_mutation("agent-1", 1, "send", &tab_id, 1)
        .with_idempotency_key("send-message-1");
    let action_id = accepted_action_id(environment.submit_action(request).unwrap());
    environment.invalidate_runtime_after_process_loss().unwrap();
    environment
        .transition_to(EnvironmentLifecycle::Ready)
        .unwrap();

    let retry = EnvironmentActionRequest::browser_mutation("agent-1", 2, "send", &tab_id, 99)
        .with_idempotency_key("send-message-1");
    assert_eq!(
        environment.submit_action(retry).unwrap(),
        ActionAdmission::Existing {
            action_id,
            state: EnvironmentActionState::Failed,
        }
    );
}

#[test]
fn actor_reconnect_preserves_identity_and_cannot_change_actor_kind() {
    let mut environment = ready_environment();
    environment
        .register_actor(EnvironmentActor::new(
            "user-1",
            EnvironmentActorKind::Human,
            "Miguel",
        ))
        .unwrap();
    environment
        .set_actor_presence("user-1", EnvironmentActorPresence::Disconnected)
        .unwrap();
    environment
        .register_actor(EnvironmentActor::new(
            "user-1",
            EnvironmentActorKind::Human,
            "Miguel G.",
        ))
        .unwrap();
    let actor = &environment.snapshot().actors[0];
    assert_eq!(actor.actor_id, "user-1");
    assert_eq!(actor.display_label, "Miguel G.");
    assert_eq!(actor.presence, EnvironmentActorPresence::Present);

    assert_eq!(
        environment.register_actor(EnvironmentActor::new(
            "user-1",
            EnvironmentActorKind::Agent,
            "Not Miguel",
        )),
        Err(EnvironmentError::ActorKindConflict {
            actor_id: "user-1".to_string(),
        })
    );
}

#[test]
fn component_health_projects_safe_diagnostic_codes() {
    let mut environment = ready_environment();
    environment.update_component_health(
        EnvironmentComponent::BrowserController,
        EnvironmentComponentHealthState::Ready,
        None,
    );
    environment.update_component_health(
        EnvironmentComponent::Streamer,
        EnvironmentComponentHealthState::Degraded,
        Some("encoder_restart_required"),
    );

    assert_eq!(
        environment.snapshot().health,
        vec![
            EnvironmentComponentHealth {
                component: EnvironmentComponent::BrowserController,
                state: EnvironmentComponentHealthState::Ready,
                diagnostic_code: None,
            },
            EnvironmentComponentHealth {
                component: EnvironmentComponent::Browser,
                state: EnvironmentComponentHealthState::Unavailable,
                diagnostic_code: None,
            },
            EnvironmentComponentHealth {
                component: EnvironmentComponent::Desktop,
                state: EnvironmentComponentHealthState::Unavailable,
                diagnostic_code: None,
            },
            EnvironmentComponentHealth {
                component: EnvironmentComponent::Streamer,
                state: EnvironmentComponentHealthState::Degraded,
                diagnostic_code: Some("encoder_restart_required".to_string()),
            },
        ]
    );
}

fn accepted_action_id(admission: ActionAdmission) -> String {
    match admission {
        ActionAdmission::Accepted { action_id } => action_id,
        other => panic!("expected accepted action, got {other:?}"),
    }
}

fn ready_environment_with_agent() -> RoomEnvironment {
    let mut environment = ready_environment();
    environment
        .register_actor(EnvironmentActor::new(
            "agent-1",
            EnvironmentActorKind::Agent,
            "Mara",
        ))
        .unwrap();
    environment
}

fn ready_environment() -> RoomEnvironment {
    let viewport = CanonicalViewport::new(1440, 900, 1, 1440, 900).unwrap();
    let mut environment = RoomEnvironment::new("room-1", "environment-1", viewport).unwrap();
    environment.start_runtime().unwrap();
    environment
        .transition_to(EnvironmentLifecycle::Ready)
        .unwrap();
    environment
}
