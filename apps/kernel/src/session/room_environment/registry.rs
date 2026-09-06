use std::collections::BTreeMap;

use super::{CanonicalViewport, EnvironmentError, RoomEnvironment, RoomEnvironmentSnapshot};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RoomEnvironmentRegistry {
    environments_by_session: BTreeMap<String, RoomEnvironment>,
}

impl RoomEnvironmentRegistry {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    // The serialized lifecycle command lands in the next isolated protocol PR.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn create(
        &mut self,
        session_id: impl Into<String>,
        environment_id: impl Into<String>,
        viewport: CanonicalViewport,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        let session_id = session_id.into();
        if let Some(existing) = self.environments_by_session.get(&session_id) {
            return Err(EnvironmentError::EnvironmentAlreadyExists {
                session_id,
                environment_id: existing.snapshot().environment_id,
            });
        }
        let environment = RoomEnvironment::new(&session_id, environment_id, viewport)?;
        let snapshot = environment.snapshot();
        self.environments_by_session.insert(session_id, environment);
        Ok(snapshot)
    }

    // The serialized read projection lands in the next isolated protocol PR.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn snapshot(
        &self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.environments_by_session
            .get(session_id)
            .map(RoomEnvironment::snapshot)
            .ok_or_else(|| EnvironmentError::EnvironmentNotFound {
                session_id: session_id.to_string(),
            })
    }

    pub(crate) fn remove(&mut self, session_id: &str) -> Option<RoomEnvironmentSnapshot> {
        self.environments_by_session
            .remove(session_id)
            .map(|environment| environment.snapshot())
    }
}
