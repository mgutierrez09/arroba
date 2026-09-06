use super::*;
use futures_util::FutureExt;

pub(super) async fn check(
    fixture: &LiveWorker,
    token: &str,
    field: &str,
    upload_path: &std::path::Path,
    target: &Value,
) {
    let root = &fixture._worker_state.root;
    let hold = root.join("hold-upload");
    let physical_count = || {
        let state: Value =
            serde_json::from_slice(&std::fs::read(root.join("chromium-state.json")).unwrap())
                .unwrap();
        state["uploadCount"].as_u64().unwrap_or(0)
    };
    let before = physical_count();
    std::fs::write(&hold, "hold file input inspection").unwrap();
    let home = Arc::clone(&fixture.home);
    let token_owned = token.to_string();
    let args = json!({"field_id":field,"files":[upload_path]});
    let request_args = args.clone();
    let upload = tokio::spawn(async move {
        home.runtime_state
            .dispatch_authenticated_runtime_tool_call(
                &token_owned,
                "slice_browser_upload",
                request_args,
            )
            .await
    });
    let result = std::panic::AssertUnwindSafe(async {
        wait_file(&root.join("upload-pending")).await;
        dispatch_json(
            &fixture.home,
            json!({"RequestRoomEnvironmentInputTakeover":{
                "session_id":fixture.rooms[0],"target":target
            }}),
        )
        .await
        .unwrap();
        wait_file(&root.join("upload-cancel-observed")).await;
        assert_eq!(
            physical_count(),
            before,
            "upload cannot mutate before cancellation cleanup"
        );
    })
    .catch_unwind()
    .await;
    std::fs::remove_file(&hold).unwrap();
    let outcome = timeout(Duration::from_secs(5), upload)
        .await
        .unwrap()
        .unwrap();
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
    let error = outcome.expect_err("takeover must cancel the pending upload");
    assert!(
        error.to_string().to_lowercase().contains("cancel"),
        "{error}"
    );
    assert_eq!(
        physical_count(),
        before,
        "cancelled upload must never expose files"
    );
    let snapshot = dispatch_json(
        &fixture.home,
        json!({"GetRoomEnvironmentState":{
            "session_id":fixture.rooms[0]
        }}),
    )
    .await
    .unwrap();
    let environment = &snapshot["RoomEnvironmentState"]["environment"];
    assert!(environment["actions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action["kind"] == "upload"
            && action["state"] == "cancelled"
            && action["targets"] == json!([target])));
    assert!(environment["input_ownership"]
        .as_array()
        .unwrap()
        .iter()
        .any(|owner| owner["target"] == *target
            && owner["actor_id"].as_str().unwrap().starts_with("user:")));
    dispatch_json(
        &fixture.home,
        json!({"ReleaseRoomEnvironmentInput":{
            "session_id":fixture.rooms[0],"target":target
        }}),
    )
    .await
    .unwrap();
    let retry = fixture
        .home
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(token, "slice_browser_upload", args)
        .await
        .unwrap();
    assert!(retry.ok);
    assert_eq!(
        physical_count(),
        before + 1,
        "a new upload may run exactly once after human release"
    );
}

async fn wait_file(path: &std::path::Path) {
    timeout(Duration::from_secs(2), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("controller reaches upload fault-injection point");
}
