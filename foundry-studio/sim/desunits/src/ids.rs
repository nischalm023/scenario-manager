//! Domain identifier types. These are the fundamental IDs used everywhere.

use serde::{Deserialize, Serialize};

macro_rules! display_id {
    ($id:ty, $prefix:literal) => {
        impl std::fmt::Display for $id {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}{}", $prefix, self.0)
            }
        }
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ToteId(pub u32);
display_id!(ToteId, "tote");

/// Synthetic per-presentation-cycle id, minted once at chain birth and threaded
/// through the deliver/store legs as the journal's Trip ID. Unlike `ToteId`,
/// which recirculates across many cycles in a 10h sim, a `PresentationId` keys
/// exactly one presentation cycle — so all legs of one cycle group by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PresentationId(pub u64);
display_id!(PresentationId, "pres");

/// Per-VTM-cycle id, minted at each `VtmForward` / `VtmReturn` commit.
/// Unlike [`PresentationId`] (shared across a presentation's forward, HTM,
/// and return legs), this keys exactly one cycle, so overlapping geometric
/// cycles under Teleport stay separable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct VtmCycleId(pub u64);
display_id!(VtmCycleId, "vcyc");

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RobotId(pub u32);

// Debug emits `r(N)` so panic output from `assert_trace!` /
// `assert_waypoints!` pastes back into the fh_sipp test suite.
impl std::fmt::Debug for RobotId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "r({})", self.0)
    }
}
display_id!(RobotId, "r");

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RackId(pub u32);
display_id!(RackId, "rack");

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct OrderId(pub u32);
display_id!(OrderId, "order");

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SkuId(pub u32);
display_id!(SkuId, "sku");

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct StorageSpaceId(pub u32);
display_id!(StorageSpaceId, "space");

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TaskId(pub u32);
display_id!(TaskId, "task");

/// A specific vertical slot in a rack: (rack, level).
/// Not to be confused with desgraph::CellId which is a navigable grid cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RackSlot {
    pub rack_id: RackId,
    pub level: u16,
}

impl std::fmt::Display for RackSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.rack_id, self.level)
    }
}

impl From<ToteId> for StorageSpaceId {
    fn from(t: ToteId) -> Self {
        Self(t.0)
    }
}
