use serde::{Deserialize, Serialize};

use crate::runtime::browser_controller_process::{
    BrowserControllerProcessSnapshot, BrowserControllerReconciliation,
};
use crate::session::CanonicalViewport;

pub(crate) const ROOM_COMPUTER_SCROLL_MAX_STEPS: u16 = 120;
pub(crate) const ROOM_COMPUTER_KEYBOARD_TEXT_MAX_UTF8_BYTES: usize = 64 * 1024;
pub(crate) const ROOM_COMPUTER_KEYBOARD_KEY_MAX_UTF8_BYTES: usize = 128;
pub(crate) const ROOM_COMPUTER_KEYBOARD_KEY_MAX_REPEAT: u16 = 32;
pub(crate) const ROOM_COMPUTER_CLIPBOARD_MAX_UTF8_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RoomComputerPointerButton {
    Left,
    Middle,
    Right,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct RoomComputerSecretInput(String);

impl RoomComputerSecretInput {
    pub(crate) fn new(value: String) -> Self {
        Self(value)
    }

    pub(crate) fn from_zeroizing(mut value: zeroize::Zeroizing<String>) -> Self {
        Self(std::mem::take(&mut *value))
    }

    pub(crate) fn into_zeroizing(mut self) -> zeroize::Zeroizing<String> {
        zeroize::Zeroizing::new(std::mem::take(&mut self.0))
    }
}

impl Drop for RoomComputerSecretInput {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.0);
    }
}

impl std::fmt::Debug for RoomComputerSecretInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[redacted computer secret input]")
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct RoomComputerKeyboardInput(String);

impl RoomComputerKeyboardInput {
    pub(crate) fn new(value: String) -> Self {
        Self(value)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn into_zeroizing(mut self) -> zeroize::Zeroizing<String> {
        zeroize::Zeroizing::new(std::mem::take(&mut self.0))
    }
}

impl Drop for RoomComputerKeyboardInput {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.0);
    }
}

impl std::fmt::Debug for RoomComputerKeyboardInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[redacted computer keyboard input]")
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct RoomComputerClipboardText(String);

impl RoomComputerClipboardText {
    pub(crate) fn new(value: String) -> Self {
        Self(value)
    }

    pub(crate) fn from_zeroizing(mut value: zeroize::Zeroizing<String>) -> Self {
        Self(std::mem::take(&mut *value))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn into_zeroizing(mut self) -> zeroize::Zeroizing<String> {
        zeroize::Zeroizing::new(std::mem::take(&mut self.0))
    }
}

impl Drop for RoomComputerClipboardText {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.0);
    }
}

impl std::fmt::Debug for RoomComputerClipboardText {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[redacted computer clipboard text]")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum RoomComputerInputAction {
    PointerMove {
        x: u32,
        y: u32,
    },
    PointerDrag {
        from_x: u32,
        from_y: u32,
        to_x: u32,
        to_y: u32,
        button: RoomComputerPointerButton,
    },
    PointerScroll {
        x: u32,
        y: u32,
        horizontal_steps: i16,
        vertical_steps: i16,
    },
    KeyboardText {
        input: RoomComputerKeyboardInput,
    },
    KeyboardKey {
        input: RoomComputerKeyboardInput,
        repeat: u16,
    },
    ClipboardWrite {
        text: RoomComputerClipboardText,
    },
    PointerClick {
        x: u32,
        y: u32,
        button: RoomComputerPointerButton,
        click_count: u8,
    },
    SecretText {
        input: RoomComputerSecretInput,
    },
}

/// Physical controller operations only. The home retains Room/tab authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum RoomBrowserControllerCommand {
    Acquire,
    Reconcile {
        viewport: CanonicalViewport,
    },
    Snapshot {
        target_id: String,
        document_id: String,
    },
    Tab {
        target_id: String,
        document_id: String,
        action: crate::runtime::browser_controller_tab::BrowserTabAction,
    },
    History {
        target_id: String,
        document_id: String,
        action: crate::runtime::browser_controller_history::BrowserHistoryAction,
    },
    Navigate {
        target_id: String,
        document_id: String,
        url: crate::runtime::browser_controller_compatibility::BrowserNavigationUrl,
    },
    Wait {
        target_id: String,
        document_id: String,
        wait: crate::runtime::browser_controller_compatibility::BrowserCompatibilityWait,
        timeout_ms: u64,
    },
    Dialog {
        target_id: String,
        document_id: String,
        action: crate::runtime::browser_controller_action::BrowserDialogAction,
    },
    ConfigureDownloads {
        execution_id: String,
        target_id: String,
        document_id: String,
    },
    RecoverDownloadConfiguration {
        execution_id: String,
        target_id: String,
        document_id: String,
    },
    CancelDownload {
        cancellation: crate::runtime::browser_controller_file_transfer::BrowserDownloadCancellation,
    },
    Upload {
        execution_id: String,
        target_id: String,
        document_id: String,
        node_ref: String,
        files: crate::runtime::browser_controller_file_transfer::BrowserUploadFiles,
    },
    RecoverUpload {
        execution_id: String,
        target_id: String,
        document_id: String,
        node_ref: String,
        files: crate::runtime::browser_controller_file_transfer::BrowserUploadFiles,
    },
    Permission {
        execution_id: String,
        target_id: String,
        document_id: String,
        permission: crate::runtime::browser_controller_permission::BrowserPermissionName,
        setting: crate::runtime::browser_controller_permission::BrowserPermissionSetting,
    },
    RecoverPermission {
        execution_id: String,
        target_id: String,
        document_id: String,
        permission: crate::runtime::browser_controller_permission::BrowserPermissionName,
        setting: crate::runtime::browser_controller_permission::BrowserPermissionSetting,
    },
    PollEvents {
        browser_generation: u64,
        cursor: u64,
        limit: u16,
    },
    Action {
        execution_id: String,
        target_id: String,
        document_id: String,
        node_ref: String,
        action: crate::runtime::browser_controller_action::BrowserLocatorAction,
        timeout_ms: u64,
    },
    RecoverAction {
        execution_id: String,
        target_id: String,
        document_id: String,
        node_ref: String,
        action: crate::runtime::browser_controller_action::BrowserLocatorAction,
        timeout_ms: u64,
    },
    CancelAction {
        execution_id: String,
    },
    ComputerInput {
        action_id: String,
        actor_id: String,
        runtime_generation: u64,
        viewport_revision: u64,
        desktop_pixel_width: u32,
        desktop_pixel_height: u32,
        action: RoomComputerInputAction,
    },
    ComputerClipboardRead {
        actor_id: String,
        runtime_generation: u64,
    },
    Release,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum RoomBrowserControllerResult {
    RecoveryRequired {
        process: BrowserControllerProcessSnapshot,
    },
    ActionCancelled {
        controller_fenced: bool,
    },
    CancellationRequested {
        accepted: bool,
    },
    Action {
        result: Option<crate::runtime::browser_controller_action::BrowserControllerActionResult>,
    },
    ComputerInputApplied {
        action_id: String,
    },
    ComputerClipboard {
        content: RoomComputerClipboardText,
    },
    Snapshot {
        snapshot: Option<
            crate::runtime::browser_controller_snapshot::BrowserControllerStructuredSnapshot,
        >,
    },
    Tab {
        result: Option<crate::runtime::browser_controller_tab::BrowserControllerTabResult>,
    },
    History {
        result: Option<
            crate::runtime::browser_controller_history::BrowserControllerHistoryResult,
        >,
    },
    Navigation {
        result: Option<
            crate::runtime::browser_controller_compatibility::BrowserControllerNavigationResult,
        >,
    },
    Wait {
        result: Option<
            crate::runtime::browser_controller_compatibility::BrowserControllerCompatibilityWaitResult,
        >,
    },
    Dialog {
        result: Option<crate::runtime::browser_controller_action::BrowserControllerDialogResult>,
    },
    Downloads {
        result: Option<
            crate::runtime::browser_controller_file_transfer::BrowserControllerDownloadsResult,
        >,
    },
    DownloadCancellation {
        result: Option<crate::runtime::browser_controller_file_transfer::BrowserControllerDownloadCancellationResult>,
    },
    Upload {
        result:
            Option<crate::runtime::browser_controller_file_transfer::BrowserControllerUploadResult>,
    },
    Permission {
        result: Option<
            crate::runtime::browser_controller_permission::BrowserControllerPermissionResult,
        >,
    },
    Events {
        batch: Option<crate::runtime::browser_controller_event::BrowserControllerEventBatch>,
    },
    Process {
        snapshot: Option<BrowserControllerProcessSnapshot>,
    },
    Reconciled {
        reconciliation: Option<BrowserControllerReconciliation>,
    },
}
