use super::action::{EnvironmentAction, EnvironmentActionState, InputTarget};
use super::ownership::InputOwnership;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentLifecycle {
    Stopped,
    Starting,
    Ready,
    Degraded,
    Saving,
    Restoring,
    Stopping,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalViewport {
    pub css_width: u32,
    pub css_height: u32,
    pub device_scale_factor: u32,
    pub desktop_pixel_width: u32,
    pub desktop_pixel_height: u32,
    pub revision: u64,
    pub last_actor_id: Option<String>,
}

impl CanonicalViewport {
    pub fn new(
        css_width: u32,
        css_height: u32,
        device_scale_factor: u32,
        desktop_pixel_width: u32,
        desktop_pixel_height: u32,
    ) -> Result<Self, EnvironmentError> {
        if [
            css_width,
            css_height,
            device_scale_factor,
            desktop_pixel_width,
            desktop_pixel_height,
        ]
        .contains(&0)
        {
            return Err(EnvironmentError::InvalidViewport);
        }
        Ok(Self {
            css_width,
            css_height,
            device_scale_factor,
            desktop_pixel_width,
            desktop_pixel_height,
            revision: 1,
            last_actor_id: None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentActorKind {
    Human,
    Agent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentActorPresence {
    Present,
    Away,
    Disconnected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentActor {
    pub actor_id: String,
    pub kind: EnvironmentActorKind,
    pub display_label: String,
    pub presence: EnvironmentActorPresence,
}

impl EnvironmentActor {
    pub fn new(
        actor_id: impl Into<String>,
        kind: EnvironmentActorKind,
        display_label: impl Into<String>,
    ) -> Self {
        Self {
            actor_id: actor_id.into(),
            kind,
            display_label: display_label.into(),
            presence: EnvironmentActorPresence::Present,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EnvironmentComponent {
    BrowserController,
    Browser,
    Desktop,
    Streamer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentComponentHealthState {
    Starting,
    Ready,
    Degraded,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentComponentHealth {
    pub component: EnvironmentComponent,
    pub state: EnvironmentComponentHealthState,
    pub diagnostic_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomEnvironmentSnapshot {
    pub session_id: String,
    pub environment_id: String,
    pub runtime_generation: u64,
    pub lifecycle: EnvironmentLifecycle,
    pub health: Vec<EnvironmentComponentHealth>,
    pub viewport: CanonicalViewport,
    pub actors: Vec<EnvironmentActor>,
    pub tabs: Vec<EnvironmentTab>,
    pub focused_tab_id: Option<String>,
    pub actions: Vec<EnvironmentAction>,
    pub input_ownership: Vec<InputOwnership>,
    pub event_cursor: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentTab {
    pub tab_id: String,
    pub url: String,
    pub title: String,
    pub document_revision: u64,
    pub focused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentError {
    InvalidViewport,
    EnvironmentAlreadyExists {
        session_id: String,
        environment_id: String,
    },
    EnvironmentNotFound {
        session_id: String,
    },
    RoomNotFound {
        session_id: String,
    },
    InvalidLifecycleTransition {
        from: EnvironmentLifecycle,
        to: EnvironmentLifecycle,
    },
    StaleRuntimeGeneration {
        expected: u64,
        actual: u64,
    },
    UnknownTab {
        tab_id: String,
    },
    StaleDocumentRevision {
        tab_id: String,
        expected: u64,
        actual: u64,
    },
    UnknownActor {
        actor_id: String,
    },
    StaleViewportRevision {
        expected: u64,
        actual: u64,
    },
    EnvironmentNotReady {
        lifecycle: EnvironmentLifecycle,
    },
    UnknownAction {
        action_id: String,
    },
    HumanActorRequired {
        actor_id: String,
    },
    InputOwnedByAnotherActor {
        target: InputTarget,
        actor_id: String,
    },
    InputNotOwned {
        target: InputTarget,
    },
    InvalidEventCapacity,
    IdempotencyConflict {
        idempotency_key: String,
    },
    ActionAlreadyTerminal {
        action_id: String,
        state: EnvironmentActionState,
    },
    ActorKindConflict {
        actor_id: String,
    },
}
