//! VTM model definitions. Will hold model-specific properties (max speed,
//! acceleration, dual-cycle rate) as the sim matures.

use serde::{Deserialize, Serialize};

/// Vertical Transfer Module hardware model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VtmModel {
    /// Quicktron A5 "Quick-Bin Ultra" — ladder-based, fixed to aisle.
    #[default]
    A5,
}
