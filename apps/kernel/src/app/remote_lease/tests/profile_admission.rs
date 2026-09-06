use super::*;
use std::future::{poll_fn, Future};
use std::task::Poll;

#[tokio::test]
async fn leased_prompt_replaces_idle_run_on_a_different_account() {
    assert_run_profile_reconciliation("account", None).await;
}

#[tokio::test]
async fn leased_prompt_replaces_idle_run_with_a_different_provider_model_or_effort() {
    for field in ["provider", "model", "effort"] {
        assert_run_profile_reconciliation(field, None).await;
    }
}

#[tokio::test]
async fn leased_prompt_preserves_busy_run_on_profile_mismatch() {
    for field in ["provider", "account", "model", "effort"] {
        assert_run_profile_reconciliation(field, Some(false)).await;
        assert_run_profile_reconciliation(field, Some(true)).await;
    }
}

// None means idle, Some(false) active, Some(true) queued for provider startup.
async fn assert_run_profile_reconciliation(field: &str, queued_work: Option<bool>) {
    let (mut app, lease) = leased_agent_fixture(false);
    let profile = crate::transport::relay_peer::RelayAgentExecutionProfile::from(
        &app.agents().get_agent(&lease.backing_agent_id).unwrap(),
    );
    let mut previous = profile.clone();
    match field {
        "provider" => previous.provider = "dev-stub".into(),
        "account" => previous.account_profile = "previous-account".into(),
        "model" => previous.model = Some("previous-model".into()),
        "effort" => previous.effort = Some("high".into()),
        _ => panic!("unknown profile field"),
    }
    let stale = app
        .providers_mut()
        .launch_run_detached(
            crate::provider::LaunchProviderRequest::new(
                &lease.backing_session_id,
                "dev-stub",
                &previous.provider,
                &previous.account_profile,
                previous.model.as_deref().unwrap(),
            )
            .with_agent_id(&lease.backing_agent_id)
            .with_variant(previous.effort.clone())
            .with_resume_state(
                crate::provider::ProviderResumeState::from_codex_thread_id(
                    "previous-account-thread",
                ),
            ),
        )
        .unwrap();
    if queued_work == Some(false) {
        sync_active_prompt(&mut app, &lease);
    } else if queued_work == Some(true) {
        app.prompt_owner_submit_prepared_prompt(
            &lease.backing_session_id,
            crate::session::PromptQueueItem::new(
                "retained-queued-prompt",
                &lease.backing_attachment_id,
                &lease.backing_agent_id,
                "preserve this queued work",
                crate::session::PromptStatus::Queued,
            ),
            true,
        )
        .unwrap();
    }
    let queued_before = app
        .prompt_owner_peek_next_queued_prompt(&lease.backing_session_id, &lease.backing_agent_id)
        .unwrap()
        .map(|prompt| serde_json::to_value(prompt).unwrap());
    let app = std::sync::Arc::new(tokio::sync::Mutex::new(app));
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(app.clone(), 2);
    let result = router
        .relay_submit_leased_prompt(
            &lease.id,
            profile.clone(),
            "use my selected account",
            "",
            Vec::new(),
            None,
            None,
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
        .await;
    let mut app = app.lock().await;
    if let Some(queued) = queued_work {
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("differs from the selected profile"));
        assert_eq!(
            app.providers().get_run(stale.id()).unwrap().state(),
            stale.state()
        );
        if queued {
            assert_eq!(
                serde_json::to_value(
                    app.prompt_owner_peek_next_queued_prompt(
                        &lease.backing_session_id,
                        &lease.backing_agent_id,
                    )
                    .unwrap()
                    .unwrap()
                )
                .unwrap(),
                queued_before.unwrap()
            );
        } else {
            assert_eq!(
                app.prompt_owner_active_prompt_for_agent(
                    &lease.backing_session_id,
                    &lease.backing_agent_id,
                )
                .unwrap()
                .unwrap()
                .id(),
                "active-prompt"
            );
        }
        return;
    }
    let (run_id, _) = result.unwrap();
    let actual = app.providers().get_run(&run_id).unwrap();
    assert_eq!(actual.provider(), profile.provider);
    assert_eq!(actual.model(), profile.model.as_deref().unwrap());
    assert_eq!(actual.variant(), profile.effort.as_deref());
    assert_eq!(
        actual.account_profile(),
        profile.account_profile,
        "a compatible tool catalog must not authorize reuse of a different account"
    );
    assert_ne!(run_id, stale.id());
    assert!(
        actual.resume_state().is_empty(),
        "a new account must not inherit the old account's provider session"
    );
    assert_eq!(
        app.providers().get_run(stale.id()).unwrap().state(),
        crate::provider::ProviderRunState::Ended
    );
}

#[tokio::test]
async fn leased_prompt_admission_prevents_interleaved_profile_changes() {
    let mut config = crate::config::DaemonConfig::for_tests();
    // Hold provider initialization so a successful admission remains queued.
    config.provider_runtime_init_delay_ms = 60_000;
    let (app, lease) = leased_agent_fixture_with_config(false, config);
    let profile = crate::transport::relay_peer::RelayAgentExecutionProfile::from(
        &app.agents().get_agent(&lease.backing_agent_id).unwrap(),
    );
    let app = std::sync::Arc::new(tokio::sync::Mutex::new(app));
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(app.clone(), 2);
    let app_guard = app.lock().await;
    let submission = router.relay_submit_leased_prompt(
        &lease.id,
        profile.clone(),
        "keep my admitted profile",
        "",
        Vec::new(),
        None,
        None,
        Vec::new(),
        None,
        crate::extension::RemoteExtensionManifest::default(),
    );
    tokio::pin!(submission);
    poll_fn(|cx| {
        assert!(submission.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    let update = router.relay_update_leased_agent_profile(
        &lease.id,
        profile.provider.clone(),
        profile.account_profile.clone(),
        Some("interleaved-model".into()),
        Some("high".into()),
    );
    tokio::pin!(update);
    poll_fn(|cx| {
        assert!(update.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    drop(app_guard);
    let (submission, update) = tokio::join!(submission, update);
    let (run_id, outcome) = submission.unwrap();
    assert!(matches!(
        outcome,
        crate::session::PromptSubmissionOutcome::Queued { .. }
    ));
    assert!(update.unwrap_err().to_string().contains("queued prompt"));
    let confirmed = router
        .relay_update_leased_agent_profile(
            &lease.id,
            profile.provider.clone(),
            profile.account_profile.clone(),
            profile.model.clone(),
            profile.effort.clone(),
        )
        .await
        .unwrap();
    assert_eq!(confirmed.account_profile, profile.account_profile);
    let app = app.lock().await;
    let run = app.providers().get_run(&run_id).unwrap();
    assert_eq!(run.model(), profile.model.as_deref().unwrap());
    assert_eq!(run.account_profile(), profile.account_profile);
    assert_ne!(run.state(), crate::provider::ProviderRunState::Ended);
}

#[tokio::test]
async fn cancelling_leased_prompt_admission_releases_profile_operations() {
    let (app, lease) = leased_agent_fixture(false);
    let profile = crate::transport::relay_peer::RelayAgentExecutionProfile::from(
        &app.agents().get_agent(&lease.backing_agent_id).unwrap(),
    );
    let app = std::sync::Arc::new(tokio::sync::Mutex::new(app));
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(app.clone(), 2);
    let app_guard = app.lock().await;
    let mut submission = Box::pin(router.relay_submit_leased_prompt(
        &lease.id,
        profile.clone(),
        "cancel before admission",
        "",
        Vec::new(),
        None,
        None,
        Vec::new(),
        None,
        crate::extension::RemoteExtensionManifest::default(),
    ));
    poll_fn(|cx| {
        assert!(submission.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    drop(submission);
    drop(app_guard);
    let updated = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        router.relay_update_leased_agent_profile(
            &lease.id,
            profile.provider,
            profile.account_profile,
            Some("after-cancellation".into()),
            profile.effort,
        ),
    )
    .await
    .expect("cancelled admission must not retain its operation guard")
    .unwrap();
    assert_eq!(updated.model.as_deref(), Some("after-cancellation"));
    let mut app = app.lock().await;
    assert!(app
        .prompt_owner_active_prompt_for_agent(&lease.backing_session_id, &lease.backing_agent_id,)
        .unwrap()
        .is_none());
    assert!(app
        .prompt_owner_peek_next_queued_prompt(&lease.backing_session_id, &lease.backing_agent_id,)
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn leased_prompt_reconciles_durable_home_profile_after_lost_ack_and_restart() {
    let (mut app, lease) = leased_agent_fixture(false);
    RemoteLeaseRuntime::new(&mut app)
        .update_leased_agent_profile(
            &lease.id,
            lease.provider.clone(),
            "home-selected-account".into(),
            lease.model.clone(),
            lease.effort.clone(),
        )
        .unwrap();
    let home_profile = crate::transport::relay_peer::RelayAgentExecutionProfile::from(
        &app.agents().get_agent(&lease.backing_agent_id).unwrap(),
    );
    // Model the only profile state retained by the home kernel across a crash:
    // its last committed durable selection. The pending transition and response
    // acknowledgement are deliberately absent after this contract round-trip.
    let durable_home_profile = serde_json::to_vec(&home_profile).unwrap();
    drop(home_profile);
    // The worker applies an update, but its acknowledgement never reaches home.
    RemoteLeaseRuntime::new(&mut app)
        .update_leased_agent_profile(
            &lease.id,
            lease.provider.clone(),
            "unacknowledged-worker-account".into(),
            Some("unacknowledged-model".into()),
            Some("high".into()),
        )
        .unwrap();
    let home_profile: crate::transport::relay_peer::RelayAgentExecutionProfile =
        serde_json::from_slice(&durable_home_profile).unwrap();
    let app = std::sync::Arc::new(tokio::sync::Mutex::new(app));
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(app.clone(), 2);
    let (provider_run_id, _outcome) = router
        .relay_submit_leased_prompt(
            &lease.id,
            home_profile.clone(),
            "use the profile committed by home",
            "",
            Vec::new(),
            None,
            None,
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
        .await
        .unwrap();
    let app = app.lock().await;
    let run = app.providers().get_run(&provider_run_id).unwrap();
    assert_eq!(run.provider(), home_profile.provider);
    assert_eq!(run.account_profile(), home_profile.account_profile);
    assert_eq!(
        run.model(),
        home_profile.model.as_deref().unwrap(),
        "provider launch must use the model committed by home"
    );
    let actual = crate::transport::relay_peer::RelayAgentExecutionProfile::from(
        &app.agents().get_agent(&lease.backing_agent_id).unwrap(),
    );
    assert_eq!(
        actual, home_profile,
        "a prompt must not use an unacknowledged worker profile"
    );
}
