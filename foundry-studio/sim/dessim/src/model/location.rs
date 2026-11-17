//! Tote location variants and type classification.

use desgraph::{CellId, RelayId, StationId};
use serde::{Deserialize, Serialize};

use crate::model::conveyor::ConveyorId;
use crate::model::{RackId, RobotId};

/// Physical location of a tote in the warehouse.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ToteLocation {
    /// No location yet, including queries before a container's creation.
    Uninitialized,
    /// Container resting at a floor presentation or induction cell.
    AtCell {
        cell_id: CellId,
    },
    /// Container has left the modeled system; its identity and history remain.
    Extracted,
    /// Stored in a rack at a specific vertical slot.
    InRack {
        rack_id: RackId,
        z_index: u16,
    },
    Reserved {
        rack_id: RackId,
        z_index: u16, /* do we need any reservation info? */
    },
    /// Being carried by a robot (HTM or VTM). The tote's physical position
    /// is the robot's position — there is no separate "at station" or
    /// "traveling" state. The tote is ON the robot.
    WithRobot {
        robot_id: RobotId,
    },
    /// Sitting at a relay position (deposited by VTM or HTM, waiting for pickup).
    AtRelay {
        relay_id: RelayId,
    },
    /// In a conveyor machine's custody. Boarding does not choose which of its
    /// presentation positions will later serve this tote.
    OnConveyance {
        conveyor_id: ConveyorId,
    },
    /// In a station's care on an EasyLoop's conveyance: riding the belt out,
    /// sitting in that station's seat while it is worked, then riding back.
    /// The tote is on NO robot and in no rack -- an HTM set it down at the
    /// machine's `dropoff_cell` and left, and a later HTM lifts it off at the
    /// `collection_cell`.
    ///
    /// Named for the SEAT's station. A belt belongs to the machine and is
    /// shared by its two stations, but a seat belongs to one of them, and the
    /// seat is where the tote is actually worked. This is the one location
    /// whose position is not a robot's, a rack's or a relay's, and it exists
    /// because a loop's whole purpose is to decouple the operator's picking
    /// from the robots' trips.
    OnConveyor {
        station_id: StationId,
    },
}

/// Which pool a tote belongs to.
///
/// Order totes and inventory totes are the same physical tote; the difference
/// is a **permanent per-instance designation** (Vinay, 2026-08-07), and it is
/// permanent for anti-starvation: if either side could take from the other,
/// one of them eventually cannot work. The world protocol says the same thing
/// with `carrier_pool` — one `carrier_type`, two pools, a count each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ToteDesignation {
    /// Holds stock waiting to be picked.
    #[default]
    Inventory,
    /// Receives an order's picks and carries them to extraction.
    Order,
}

/// Physical container type — affects rendering height/shape.
///
/// The lowercase aliases read recordings written when an order's container
/// was its own snake_case enum.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToteType {
    /// Reusable shallow support, used for the provisional SIOC workflow.
    #[serde(alias = "tray")]
    Tray,
    /// Walled container with open top.
    #[default]
    #[serde(alias = "tote")]
    Tote,
    /// Shipping carton carried directly by an HTM.
    #[serde(alias = "carton")]
    Carton,
}
