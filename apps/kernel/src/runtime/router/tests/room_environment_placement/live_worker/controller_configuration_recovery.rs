use super::*;

pub(super) async fn check(fixture: &LiveWorker, token: &str) {
    let runtime = &fixture.home.runtime_state;
    let room = &fixture.rooms[0];
    let quiet = fixture._worker_state.root.join("quiet-permission-events");
    std::fs::write(&quiet, "do not simulate unrelated page navigation").unwrap();
    for (tool, args, kind, counter) in [
        (
            "slice_browser_downloads",
            json!({}),
            "download_configure",
            "downloadConfigureCount",
        ),
        (
            "slice_browser_permission",
            json!({"permission":"geolocation","setting":"denied"}),
            "permission_set",
            "permissionCount",
        ),
    ] {
        let status = runtime
            .dispatch_authenticated_runtime_tool_call(token, "slice_browser_status", json!({}))
            .await
            .unwrap();
        let tab = status.payload["tab_id"].as_str().unwrap();
        let agent = status.payload["agent_id"].as_str().unwrap();
        let before = physical_count(fixture, counter);
        let pid = std::fs::read_to_string(fixture._worker_state.root.join("controller.pid"))
            .unwrap()
            .trim()
            .parse::<i32>()
            .unwrap();
        assert_eq!(unsafe { libc::kill(pid, libc::SIGKILL) }, 0);
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Enter at admission so a preliminary MCP status read cannot consume
        // the restart before the configuration mutation reaches its worker.
        let result = timeout(Duration::from_secs(5), async {
            if kind == "permission_set" {
                runtime.set_browser_permission_as_agent(
                    room, agent, tab,
                    crate::runtime::browser_controller_permission::BrowserPermissionName::Geolocation,
                    crate::runtime::browser_controller_permission::BrowserPermissionSetting::Denied,
                ).await.map(|_| ())
            } else {
                runtime.configure_browser_downloads_as_agent(room, agent, tab).await.map(|_| ())
            }
        }).await.expect("restart must not deadlock admitted configuration");
        let error = result.expect_err("pre-restart configuration must not execute");
        assert!(
            error
                .to_string()
                .contains("browser controller restarted before the operation"),
            "{error}"
        );
        assert_eq!(physical_count(fixture, counter), before);
        let environment = runtime.room_environment_snapshot(room).unwrap();
        let action = environment
            .actions
            .iter()
            .filter(|action| action.kind == kind)
            .max_by_key(|action| action.sequence)
            .unwrap();
        assert_eq!(action.state, crate::session::EnvironmentActionState::Failed);
        assert!(
            !environment.actions.iter().any(|action| action.kind == kind
                && matches!(
                    action.state,
                    crate::session::EnvironmentActionState::Running
                        | crate::session::EnvironmentActionState::Queued
                )),
            "restart must settle configuration reservations"
        );

        let refreshed = runtime
            .dispatch_authenticated_runtime_tool_call(token, "slice_browser_status", json!({}))
            .await
            .unwrap();
        assert_eq!(refreshed.payload["tab_id"], tab);
        assert_eq!(
            refreshed.payload["environment_id"],
            status.payload["environment_id"]
        );
        assert!(
            runtime
                .dispatch_authenticated_runtime_tool_call(token, tool, args)
                .await
                .unwrap()
                .ok
        );
        assert_eq!(
            physical_count(fixture, counter),
            before + 1,
            "fresh retry executes exactly once"
        );
    }
    std::fs::remove_file(quiet).unwrap();
}

fn physical_count(fixture: &LiveWorker, counter: &str) -> u64 {
    let state: Value = serde_json::from_slice(
        &std::fs::read(fixture._worker_state.root.join("chromium-state.json")).unwrap(),
    )
    .unwrap();
    state[counter].as_u64().unwrap_or(0)
}
