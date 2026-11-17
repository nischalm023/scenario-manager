//! What a **destination** is within the model: the abstract end-point that
//! demand (and, from phase 2 of the putaway-locality work, inducted
//! inventory) is ultimately headed for — as opposed to the physical stations
//! and cells it passes through on the way there. v1 defines a single kind,
//! the forward distribution center of a bulk demand row; future kinds (a
//! specific station, a wave, an external order) extend the enum without
//! re-plumbing consumers. The stripe map (destination pick-station striping,
//! `flows::ttp::demand::striping`) is keyed by this type, and a
//! `Destination` attached to inventory is called its *destination earmark*
//! (spec: specs/putaway_pick_locality/plan.md §4.2, decisions D9/D10).

use std::fmt;

/// String identifier for a forward distribution center (the "customer").
/// Kept as a string to match the source data (`087ABC004`-style codes)
/// without forcing a numeric remap.
pub type DistributionCenterId = String;

/// An end-point that demand or inventory is headed for.
///
/// `DistributionCenter` is the sole v1 variant (spec decision D10); the enum
/// exists so striping and earmarks generalize to other destination kinds
/// without changing their consumers.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
pub enum Destination {
    /// Forward distribution center of a bulk demand row (`dc_id` in the
    /// bulk CSV).
    DistributionCenter(DistributionCenterId),
}

impl fmt::Display for Destination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Destination::DistributionCenter(dc) => write!(f, "{dc}"),
        }
    }
}
