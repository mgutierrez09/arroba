use super::*;
use crate::session::{EnvironmentActionState, EnvironmentTabObservation};

pub(super) async fn check(fixture: &LiveWorker, token: &str) {
    let runtime = &fixture.home.runtime_state;
    let room = &fixture.rooms[0];
    let status = runtime
        .dispatch_authenticated_runtime_tool_call(token, "slice_browser_status", json!({}))
        .await
        .unwrap();
    let tab = status.payload["tab_id"].as_str().unwrap();
    let agent = status.payload["agent_id"].as_str().unwrap();
    let snapshot = runtime
        .capture_browser_environment_snapshot(room, tab)
        .await
        .unwrap();
    let shadow = snapshot.shadow_roots.first().unwrap();
    let button = snapshot
        .dom_nodes
        .iter()
        .find(|node| {
            node.parent_ref.as_deref() == Some(shadow.element_ref.as_str())
                && node.node_name == "BUTTON"
        })
        .unwrap();
    runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_click",
            json!({"field_id":button.element_ref}),
        )
        .await
        .unwrap();
    runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_tab",
            json!({"tab_id":tab,"action":"activate"}),
        )
        .await
        .unwrap();
    let environment = runtime.room_environment_snapshot(room).unwrap();
    assert_eq!(environment.focused_tab_id.as_deref(), Some(tab));
    assert_eq!(environment.tabs.len(), 2);
    let revision = environment
        .tabs
        .iter()
        .find(|t| t.tab_id == tab)
        .unwrap()
        .document_revision;
    let observations = environment
        .tabs
        .iter()
        .map(|t| {
            let binding = runtime
                .room_environment_controller_tab_binding(room, &t.tab_id)
                .unwrap();
            EnvironmentTabObservation {
                runtime_target_id: binding.runtime_target_id,
                document_id: binding.document_id,
                url: t.url.clone(),
                title: t.title.clone(),
            }
        })
        .collect::<Vec<_>>();
    let other = environment.tabs.iter().find(|t| t.tab_id != tab).unwrap();
    let other_target = runtime
        .room_environment_controller_tab_binding(room, &other.tab_id)
        .unwrap()
        .runtime_target_id;

    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let blocker_runtime = runtime.clone();
    let blocker_room = room.clone();
    let blocker_agent = agent.to_string();
    let blocker_tab = tab.to_string();
    let blocker = tokio::spawn(async move {
        blocker_runtime
            .execute_browser_mutation_as_agent(
                &blocker_room,
                &blocker_agent,
                &blocker_tab,
                revision,
                "navigation-queue-blocker",
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
    let nav_runtime = runtime.clone();
    let nav_room = room.clone();
    let nav_agent = agent.to_string();
    let navigate = tokio::spawn(async move {
        nav_runtime
            .navigate_browser_environment_compatibility_as_agent(
                &nav_room,
                &nav_agent,
                "https://worker.test/queued-navigation",
            )
            .await
    });
    let queued = timeout(Duration::from_secs(2), async {
        loop {
            if runtime
                .room_environment_snapshot(room)
                .unwrap()
                .actions
                .iter()
                .any(|a| a.kind == "navigate" && a.state == EnvironmentActionState::Queued)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    // An external focus observation does not change the admitted Action target.
    let focus =
        runtime.reconcile_room_environment_controller_tabs(room, observations, Some(&other_target));
    let _ = release_tx.send(());
    let blocker_result = blocker.await;
    let result = timeout(Duration::from_secs(5), navigate).await;
    queued.expect("navigation queues behind the Tab reservation");
    focus.unwrap();
    blocker_result.unwrap().unwrap();
    let physical: Value = serde_json::from_slice(
        &std::fs::read(fixture._worker_state.root.join("chromium-state.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        physical["navigationSession"], "worker-cdp-session",
        "queued navigation must never switch to the newly focused popup"
    );
    result.unwrap().unwrap().unwrap();
    let after = runtime.room_environment_snapshot(room).unwrap();
    assert_eq!(
        after.tabs.iter().find(|t| t.tab_id == tab).unwrap().url,
        "https://worker.test/queued-navigation"
    );
    assert_eq!(
        after
            .tabs
            .iter()
            .find(|t| t.tab_id == other.tab_id)
            .unwrap()
            .url,
        "https://popup.worker.test/"
    );
    runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_tab",
            json!({"tab_id":other.tab_id,"action":"close"}),
        )
        .await
        .unwrap();
}
