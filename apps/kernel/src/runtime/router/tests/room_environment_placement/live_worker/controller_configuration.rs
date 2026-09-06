use super::*;

pub(super) async fn check(
    fixture: &LiveWorker,
    token: &str,
    agent_id: &str,
    tab: &str,
    other_tab: &str,
) {
    let room = &fixture.rooms[0];
    let quiet = fixture._worker_state.root.join("quiet-permission-events");
    std::fs::write(&quiet, "do not simulate unrelated page navigation").unwrap();
    let tools = [
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
    ];
    let mut failures = Vec::new();
    for (scope, target) in [
        ("tab", json!({"kind":"browser_tab","id":tab})),
        ("other_origin", json!({"kind":"browser_tab","id":other_tab})),
        ("desktop", json!({"kind":"desktop"})),
    ] {
        dispatch_json(
            &fixture.home,
            json!({"RequestRoomEnvironmentInputTakeover":{
                "session_id":room,"target":target
            }}),
        )
        .await
        .unwrap();
        for (tool, args, _, counter) in &tools {
            let before = physical_count(fixture, counter);
            let result = fixture
                .home
                .runtime_state
                .dispatch_authenticated_runtime_tool_call(token, tool, args.clone())
                .await;
            let denied = *tool == "slice_browser_downloads" || scope != "other_origin";
            if denied {
                if !result
                    .as_ref()
                    .is_err_and(|error| error.to_string().contains("belongs to"))
                {
                    failures.push(format!("{tool} ignored {scope} ownership"));
                }
                if physical_count(fixture, counter) != before {
                    failures.push(format!(
                        "{tool} mutated Chromium while {scope} was human-owned"
                    ));
                }
            } else if !result.is_ok_and(|result| result.ok) {
                failures.push("unrelated-origin ownership blocked permission change".into());
            }
        }
        dispatch_json(
            &fixture.home,
            json!({"ReleaseRoomEnvironmentInput":{
                "session_id":room,"target":target
            }}),
        )
        .await
        .unwrap();
    }
    assert!(
        failures.is_empty(),
        "configuration ownership failures: {failures:?}"
    );

    for (tool, args, kind, counter) in tools {
        let before = physical_count(fixture, counter);
        let result = fixture
            .home
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(token, tool, args)
            .await
            .unwrap();
        assert!(result.ok);
        assert_eq!(physical_count(fixture, counter), before + 1);
        let state = dispatch_json(
            &fixture.home,
            json!({"GetRoomEnvironmentState":{
                "session_id":room
            }}),
        )
        .await
        .unwrap();
        let environment = &state["RoomEnvironmentState"]["environment"];
        let action = environment["actions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|action| action["kind"] == kind)
            .max_by_key(|action| action["sequence"].as_u64().unwrap())
            .expect("configuration action is attributed");
        assert_eq!(action["state"], "completed");
        assert_eq!(
            action["actor_id"],
            crate::session::agent_environment_actor_id(agent_id)
        );
        let targets = action["targets"].as_array().unwrap();
        assert!(targets.contains(&json!({"kind":"desktop"})));
        assert!(targets.contains(&json!({"kind":"browser_tab","id":tab})));
        assert_eq!(
            targets.contains(&json!({"kind":"browser_tab","id":other_tab})),
            kind == "download_configure"
        );
    }
    super::controller_configuration_queue::check(fixture, agent_id, tab).await;
    super::controller_configuration_cancellation::check(fixture, token, agent_id, tab).await;
    std::fs::remove_file(quiet).unwrap();
}

fn physical_count(fixture: &LiveWorker, key: &str) -> u64 {
    let state: Value = serde_json::from_slice(
        &std::fs::read(fixture._worker_state.root.join("chromium-state.json")).unwrap(),
    )
    .unwrap();
    state[key].as_u64().unwrap_or(0)
}
