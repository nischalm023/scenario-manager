//! `starting_tote`: the totes standing in the racks at world time 0, with what each holds.

use serde::{Deserialize, Serialize};

use super::SkuQuantity;

/// One tote standing in the racks at world time 0.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct StartingTote {
    /// The tote's id.
    pub id: String,
    /// What the tote holds, at least one SKU; several make a mixed tote.
    #[cfg_attr(test, schemars(length(min = 1)))]
    pub contents: Vec<SkuQuantity>,
}
