use crate::model::SimTime;
use crate::utils::frontier_sorted_set::StaleOperation;

/// Error from an inventory observer callback.
#[derive(Debug, thiserror::Error)]
pub enum InventoryObserverError {
    #[error("stale index update: {0}")]
    StaleIndex(#[from] StaleOperation),

    #[error(transparent)]
    Inventory(Box<InventoryError>),
}

impl From<InventoryError> for InventoryObserverError {
    fn from(e: InventoryError) -> Self {
        Self::Inventory(Box::new(e))
    }
}

/// Inventory service error.
#[derive(Debug, thiserror::Error)]
pub enum InventoryError {
    #[error("not found in inventory")]
    NotFound,

    #[error("no tote IDs remain")]
    IdExhausted,

    #[error("tote mutation at {at:?} precedes creation at {created_at:?}")]
    BeforeCreation { at: SimTime, created_at: SimTime },

    #[error("insufficient quantity")]
    InsufficientQuantity,

    #[error(transparent)]
    Observer(#[from] InventoryObserverError),
}
