use super::*;
use crate::local::{
    GetPromptSettingRequest, ListPromptSettingsRequest, PreviewPromptSettingRequest,
    PromptSettingVersion, ResetAllPromptSettingsRequest, ResetPromptSettingRequest,
    UpdatePromptSettingRequest,
};

fn setting() -> crate::prompt_assembly::PromptSettingRecord {
    crate::prompt_assembly::PromptSettingRecord {
        id: "workflow/turn".to_string(),
        title: "Workflow turn contract".to_string(),
        scope: "workflow".to_string(),
        audience: "workflow-agent".to_string(),
        provider_applicability: vec!["codex".to_string()],
        source: "bundled".to_string(),
        current: "Use {{DELIVERY_TOKEN}}".to_string(),
        default: "Use {{DELIVERY_TOKEN}}".to_string(),
        current_sha256: "current".to_string(),
        default_sha256: "default".to_string(),
        current_bytes: 24,
        default_bytes: 24,
        revision: 1,
        variables: vec!["DELIVERY_TOKEN".to_string()],
        editable: true,
        protected: false,
    }
}

#[test]
fn local_daemon_protocol_prompt_settings_shapes_are_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 308);
    let requests = vec![
        LocalDaemonRequest::ListPromptSettings(ListPromptSettingsRequest),
        LocalDaemonRequest::GetPromptSetting(GetPromptSettingRequest {
            id: "workflow/turn".to_string(),
        }),
        LocalDaemonRequest::UpdatePromptSetting(UpdatePromptSettingRequest {
            id: "workflow/turn".to_string(),
            markdown: "Use {{DELIVERY_TOKEN}}".to_string(),
            expected_revision: 7,
            expected_sha256: "abc".to_string(),
        }),
        LocalDaemonRequest::PreviewPromptSetting(PreviewPromptSettingRequest {
            id: "workflow/turn".to_string(),
            variables: [("DELIVERY_TOKEN".to_string(), "token".to_string())]
                .into_iter()
                .collect(),
        }),
        LocalDaemonRequest::ResetPromptSetting(ResetPromptSettingRequest {
            id: "workflow/turn".to_string(),
            expected_revision: 7,
            expected_sha256: "abc".to_string(),
        }),
        LocalDaemonRequest::ResetAllPromptSettings(ResetAllPromptSettingsRequest {
            expected: [(
                "workflow/turn".to_string(),
                PromptSettingVersion {
                    revision: 7,
                    sha256: "abc".to_string(),
                },
            )]
            .into_iter()
            .collect(),
        }),
    ];
    let encoded = requests
        .iter()
        .map(|request| serde_json::to_value(request).expect("prompt settings request encodes"))
        .collect::<Vec<_>>();
    assert_eq!(encoded[0], serde_json::json!({"ListPromptSettings": null}));
    assert_eq!(
        encoded[1],
        serde_json::json!({"GetPromptSetting": {"id": "workflow/turn"}})
    );
    assert_eq!(
        encoded[2],
        serde_json::json!({
            "UpdatePromptSetting": {
                "id": "workflow/turn",
                "markdown": "Use {{DELIVERY_TOKEN}}",
                "expected_revision": 7,
                "expected_sha256": "abc"
            }
        })
    );
    assert_eq!(
        encoded[3],
        serde_json::json!({
            "PreviewPromptSetting": {
                "id": "workflow/turn",
                "variables": {"DELIVERY_TOKEN": "token"}
            }
        })
    );
    assert_eq!(
        encoded[4],
        serde_json::json!({"ResetPromptSetting": {"id": "workflow/turn", "expected_revision": 7, "expected_sha256": "abc"}})
    );
    assert_eq!(
        encoded[5],
        serde_json::json!({"ResetAllPromptSettings": {"expected": {"workflow/turn": {"revision": 7, "sha256": "abc"}}}})
    );

    let responses = vec![
        LocalDaemonResponse::PromptSettingsListed {
            settings: vec![setting()],
        },
        LocalDaemonResponse::PromptSetting { setting: setting() },
        LocalDaemonResponse::PromptSettingPreview {
            id: "workflow/turn".to_string(),
            markdown: "Use token".to_string(),
            variables: [("DELIVERY_TOKEN".to_string(), "token".to_string())]
                .into_iter()
                .collect(),
        },
        LocalDaemonResponse::PromptSettingsReset {
            settings: vec![setting()],
        },
    ];
    let encoded = responses
        .into_iter()
        .map(|response| serde_json::to_value(response).expect("prompt settings response encodes"))
        .collect::<Vec<_>>();
    let expected_setting = serde_json::json!({
        "id": "workflow/turn",
        "title": "Workflow turn contract",
        "scope": "workflow",
        "audience": "workflow-agent",
        "provider_applicability": ["codex"],
        "source": "bundled",
        "current": "Use {{DELIVERY_TOKEN}}",
        "default": "Use {{DELIVERY_TOKEN}}",
        "current_sha256": "current",
        "default_sha256": "default",
        "current_bytes": 24,
        "default_bytes": 24,
        "revision": 1,
        "variables": ["DELIVERY_TOKEN"],
        "editable": true,
        "protected": false
    });
    assert_eq!(
        encoded[0],
        serde_json::json!({"PromptSettingsListed": {"settings": [expected_setting.clone()]}})
    );
    assert_eq!(
        encoded[1],
        serde_json::json!({"PromptSetting": {"setting": expected_setting}})
    );
    assert_eq!(
        encoded[2],
        serde_json::json!({
            "PromptSettingPreview": {
                "id": "workflow/turn",
                "markdown": "Use token",
                "variables": {"DELIVERY_TOKEN": "token"}
            }
        })
    );
    assert_eq!(
        encoded[3],
        serde_json::json!({"PromptSettingsReset": {"settings": [setting()]}})
    );
}
