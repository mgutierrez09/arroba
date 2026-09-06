use super::*;

pub(super) async fn check(fixture: &LiveWorker, agent_id: &str, tab_id: &str) {
    let runtime = &fixture.home.runtime_state;
    let room = &fixture.rooms[0];
    for permission in [false, true] {
        let environment = runtime.room_environment_snapshot(room).unwrap();
        let selected = environment
            .tabs
            .iter()
            .find(|tab| tab.tab_id == tab_id)
            .unwrap();
        let observations = environment
            .tabs
            .iter()
            .map(|tab| {
                let binding = runtime
                    .room_environment_controller_tab_binding(room, &tab.tab_id)
                    .unwrap();
                crate::session::EnvironmentTabObservation {
                    runtime_target_id: binding.runtime_target_id,
                    document_id: binding.document_id,
                    url: tab.url.clone(),
                    title: tab.title.clone(),
                }
            })
            .collect::<Vec<_>>();
        let selected_binding = runtime
            .room_environment_controller_tab_binding(room, tab_id)
            .unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let blocker_runtime = runtime.clone();
        let blocker_room = room.clone();
        let blocker_agent = agent_id.to_string();
        let blocker_tab = tab_id.to_string();
        let revision = selected.document_revision;
        let blocker = tokio::spawn(async move {
            blocker_runtime
                .execute_browser_mutation_as_agent(
                    &blocker_room,
                    &blocker_agent,
                    &blocker_tab,
                    revision,
                    "configuration-blocker",
                    None,
                    async {
                        let _ = started_tx.send(());
                        let _ = release_rx.await;
                        Ok(())
                    },
                )
                .await
        });
        started_rx.await.unwrap();
        let config_runtime = runtime.clone();
        let config_room = room.clone();
        let config_agent = agent_id.to_string();
        let config_tab = tab_id.to_string();
        let configuration = tokio::spawn(async move {
            if permission {
                config_runtime.set_browser_permission_as_agent(
                    &config_room, &config_agent, &config_tab,
                    crate::runtime::browser_controller_permission::BrowserPermissionName::Geolocation,
                    crate::runtime::browser_controller_permission::BrowserPermissionSetting::Denied,
                ).await.map(|_| ())
            } else {
                config_runtime
                    .configure_browser_downloads_as_agent(&config_room, &config_agent, &config_tab)
                    .await
                    .map(|_| ())
            }
        });
        let kind = if permission {
            "permission_set"
        } else {
            "download_configure"
        };
        let queued = timeout(Duration::from_secs(2), async {
            loop {
                if runtime
                    .room_environment_snapshot(room)
                    .unwrap()
                    .actions
                    .iter()
                    .any(|action| {
                        action.kind == kind
                            && action.state == crate::session::EnvironmentActionState::Queued
                    })
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await;
        let mut changed = observations.clone();
        changed.push(crate::session::EnvironmentTabObservation {
            runtime_target_id: "new-configuration-target".into(),
            document_id: "new-configuration-document".into(),
            url: selected.url.clone(),
            title: "New same-origin tab".into(),
        });
        let reconciled = runtime.reconcile_room_environment_controller_tabs(
            room,
            changed,
            Some(&selected_binding.runtime_target_id),
        );
        let _ = release_tx.send(());
        let blocker_result = blocker.await;
        let result = timeout(Duration::from_secs(3), configuration).await;
        runtime
            .reconcile_room_environment_controller_tabs(
                room,
                observations,
                Some(&selected_binding.runtime_target_id),
            )
            .unwrap();
        queued.expect("configuration waits behind the existing mutation");
        reconciled.unwrap();
        blocker_result.unwrap().unwrap();
        let error = result
            .unwrap()
            .unwrap()
            .expect_err("changed scope must not dispatch");
        assert!(error.to_string().contains("scope changed"), "{error}");
    }
}
