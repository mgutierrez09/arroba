use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Weak};

use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

/// Serialize profile changes with prompt admission, without blocking other leases.
#[derive(Clone, Default)]
pub(super) struct LeasedAgentOperations {
    lanes: Arc<Mutex<BTreeMap<String, Weak<AsyncMutex<()>>>>>,
}

impl LeasedAgentOperations {
    pub(super) async fn lock(&self, leased_agent_id: &str) -> OwnedMutexGuard<()> {
        let lane = {
            let mut lanes = self
                .lanes
                .lock()
                .expect("leased agent operation map poisoned");
            lanes.retain(|_, lane| lane.strong_count() > 0);
            if let Some(lane) = lanes.get(leased_agent_id).and_then(Weak::upgrade) {
                lane
            } else {
                let lane = Arc::new(AsyncMutex::new(()));
                lanes.insert(leased_agent_id.to_string(), Arc::downgrade(&lane));
                lane
            }
        };
        lane.lock_owned().await
    }
}
