//! Retrieve admission distinguishes work needed by an admitted consumer from
//! preparation for a future consumer. Both retain their inbound credit through
//! the relay pickup until the HTM's source hold has a committed end.

use std::sync::Arc;

use crate::signals::Version;

/// A live consumer-right predicate. Callers publish changes through `changed`;
/// promotion is re-read without rebuilding the owning retrieval task.
#[derive(Clone)]
pub struct RetrieveAdmission {
    productive: Arc<dyn Fn() -> bool + Send + Sync>,
    changed: Version,
}

impl RetrieveAdmission {
    pub fn new(productive: impl Fn() -> bool + Send + Sync + 'static, changed: Version) -> Self {
        Self { productive: Arc::new(productive), changed }
    }

    pub fn is_productive(&self) -> bool {
        (self.productive)()
    }

    pub fn version(&self) -> &Version {
        &self.changed
    }

    pub(super) fn protected_slots(&self) -> u32 {
        // In addition to the existing two return places, leave two inbound
        // places for admitted consumers' next order-container/inventory pair.
        if self.is_productive() { 0 } else { 2 }
    }
}
