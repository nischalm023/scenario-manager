//! Inventory mutation events — emitted by InventoryService, consumed by
//! external indices (e.g., RelayInventoryIndex).

use crate::model::ids::ToteId;
use crate::model::location::{ToteDesignation, ToteLocation};
use crate::model::{SimTime, SkuId};

pub use super::error::{InventoryError, InventoryObserverError};

/// A mutation event from InventoryService.
#[derive(Clone, Debug)]
pub enum InventoryEvent {
    /// Tote location changed.
    LocationChanged {
        tote_id: ToteId,
        /// Which pool the tote belongs to — permanent, so the index can keep
        /// one frontier per pool without looking anything up.
        designation: ToteDesignation,
        old_location: ToteLocation,
        new_location: ToteLocation,
        is_empty: bool,
        at: SimTime,
    },
    /// Tote content changed (increment or decrement of a SKU).
    ContentsChanged {
        tote_id: ToteId,
        designation: ToteDesignation,
        sku: SkuId,
        delta: i32,
        /// Tote was empty before this mutation.
        was_empty: bool,
        /// Tote is empty after this mutation.
        is_empty: bool,
        /// Tote location at event time.
        location: ToteLocation,
        at: SimTime,
    },
}

/// Observer trait for inventory mutation events.
pub trait InventoryObserver: Send + Sync {
    fn on_inventory_event(&self, event: &InventoryEvent) -> Result<(), InventoryObserverError>;
}
