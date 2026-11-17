//! `sku`: the SKU ledger, the SKUs a run handles with every unit of each.

use serde::{Deserialize, Serialize};

/// One SKU the run handles, with every unit of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Sku {
    /// The SKU's id, by which the records of the other streams name it.
    pub id: String,
    /// Every unit of the SKU that stands in the racks at world time 0 or comes in during the run. Beside
    /// `starting_tote` or `inbound_lot` streams, the sum of their quantities of the SKU; with neither, the run racks
    /// a configured share at world time 0 and brings in the rest.
    pub units: u32,
}
