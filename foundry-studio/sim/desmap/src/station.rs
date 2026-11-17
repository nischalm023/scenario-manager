//! Within desmap, `station` is the compiled MAP-ENTITY model of operator
//! endpoints: the pick/put/sort `Station` (the cells it presents at and the
//! queue before each, exit choreography, bin overrides) and the
//! `RelayPosition` cantilever shelf where VTMs and HTMs exchange totes. These
//! are the runtime shapes the simulator consumes; `design::types` holds the
//! authoring-format definitions that compile into them.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use crate::VtmPosition;
use crate::types::*;

/// An approach queue leading into one named cell.
///
/// The cell is stated rather than implied because a machine has several
/// handover points and they are not interchangeable: a queue into the dropoff
/// is not a queue into the collection cell, and a robot in the wrong one is
/// facing the wrong way.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HandoffQueue {
    /// The cell this queue feeds. The path's last step adjoins it.
    pub into: CellId,
    /// Ordered approach path, entry first.
    pub path: Vec<CellId>,
    /// The cell a robot drives the lane FROM: the one arc into the head of
    /// `path` that survives the corridor restriction.
    ///
    /// Not a step of the lane, and not derivable from `path` alone. A machine's
    /// lane is entered at a cell the lane does not name -- on an EasyLoop that
    /// is the dropoff, which the 2026-09-04 layout ruling put outside the
    /// lane's own path -- so a consumer asking "what must a robot cross to
    /// reach this handover" needs this cell as well as the steps. Only the
    /// graph can answer it, so `worldmap` fills it in once the corridors are
    /// resolved; a map compiled some other way, or read back from a file
    /// written before this field existed, states `None` and a consumer falls
    /// back to the path.
    #[serde(default)]
    pub entry_from: Option<CellId>,
    /// Whether the lane is an APPROACH into an operator's standing spot.
    ///
    /// True says one standing spot, one claimant: every forward edge is gated
    /// to `into`, which is what stops the queue becoming a shortcut across the
    /// floor. False says a machine's THROUGH ROUTE, where the gate moves to the
    /// door -- the one arc in from the floor outside, admitting the robots
    /// bound for the machine's handover cells -- and the chain itself stays
    /// open, because a robot that has set its carrier down and is leaving for
    /// somewhere else drives the rest of the lane to get out.
    #[serde(default = "approach_by_default", alias = "gated")]
    pub approach: bool,
}

/// A lane read back from a serialized map states nothing here if it was written
/// before the field existed, and every lane that existed then was an approach.
fn approach_by_default() -> bool {
    true
}

/// A cell a station presents at, and the queue before it when a lane feeds it.
///
/// A station presents at one cell or more, and a cell may have no queue. A
/// Dematic pick station presents at two: two mirrored lanes feed its one arm.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PresentationPoint {
    /// Cell where an HTM presents a carrier to the operator or arm.
    pub presentation_cell: CellId,
    /// Ordered approach path, entry first. Its last cell adjoins
    /// `presentation_cell`; it is empty when nothing states an approach.
    #[serde(default)]
    pub queue_path: Vec<CellId>,
}

impl PresentationPoint {
    /// Every cell of the point: the cell a station presents at, then its queue.
    pub fn cells(&self) -> impl Iterator<Item = CellId> + '_ {
        std::iter::once(self.presentation_cell).chain(self.queue_path.iter().copied())
    }
}

/// The process served by an outbound handoff, independent of its map position.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboundRole {
    RouteDispense,
    Bagging,
    Flex,
    SealerDropoff,
    SealedBoxPickup,
}

impl OutboundRole {
    pub fn label(self) -> &'static str {
        match self {
            Self::RouteDispense => "Route dispense",
            Self::Bagging => "Bagging",
            Self::Flex => "Flex",
            Self::SealerDropoff => "Sealer drop-off",
            Self::SealedBoxPickup => "Sealed-box pickup",
        }
    }
}

/// A PPS (pick/put/sort) station.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Station {
    pub id: StationId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outbound_role: Option<OutboundRole>,
    /// The cell the station occupies.
    pub position: CellId,
    /// The cells the station presents at, ordered by cell id, each with the
    /// queue of the lane that feeds it, if one does. Never empty: a station
    /// presents at one cell at least, and a serialized station that states
    /// none is refused.
    ///
    /// A station with a belt presents at the belt's `dropoff_cell`, and the
    /// lanes that serve its belt are `handoff_queues` below.
    #[serde(deserialize_with = "at_least_one_point")]
    pub presentation_points: Vec<PresentationPoint>,
    /// Which grid edge the station is on (parametric only; None for GSB).
    #[serde(default)]
    pub edge: Option<StationEdge>,
    /// Approach queues into a belt's handover cells, `dropoff_cell` and
    /// `collection_cell`, and into `order_presentation_cell`.
    ///
    /// The queues in `presentation_points` feed the cells the station presents
    /// at: the whole story for a station that presents to an operator
    /// directly. A machine with fixed conveyance has more handover points than
    /// that -- an EasyLoop hands inventory to its belt at `dropoff_cell` and
    /// takes it back at `collection_cell`, metres from where an order carrier
    /// presents -- and each is a single robot's standing spot with its own
    /// approach.
    ///
    /// Without these each handover is a bare cell: a robot waiting for a belt
    /// slot, a relay slot or its next task waits ON the handover, which is the
    /// machine's only door, and every delivery behind it is refused for as long
    /// as it stands there.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub handoff_queues: Vec<HandoffQueue>,
    /// Cardinal direction HTM departs after presenting.
    #[serde(default = "default_exit_direction")]
    pub exit_direction: CardinalDirection,
    /// Per-station bin count override. When `None`, the system default is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bins: Option<usize>,
    /// Operational station type. Default: Pick (backward compatible).
    #[serde(default)]
    pub station_type: StationType,
    /// Where an HTM sets a carrier down onto fixed conveyance and leaves --
    /// an EasyLoop's DROPOFF cell. `None` for a station with no belt, which is
    /// every station that presents to an operator directly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dropoff_cell: Option<CellId>,
    /// Where an HTM lifts a carrier back off that conveyance -- an EasyLoop's
    /// COLLECTION cell. Distinct from `dropoff_cell`: the belt runs between them, and
    /// the carrier is on neither robot in between.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection_cell: Option<CellId>,
    /// One circuit of that conveyance, in milliseconds. Milliseconds and not a
    /// duration type because desmap is the plain-data layer, the same reason
    /// its lengths are `_mm`; dessim converts at its own boundary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conveyor_transit_ms: Option<u64>,
    /// Where an ORDER carrier presents, when that is a different place from
    /// the cells in `presentation_points`. An EasyLoop hands inventory to a
    /// belt at one cell and presents an order carrier at another, metres apart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order_presentation_cell: Option<CellId>,
    /// What the machine's conveyance accepts, from the port's own `of`. A
    /// belt that carries inventory totes must not be handed an order tote:
    /// they are different populations with different journeys. A plain string
    /// because desmap is the plain-data layer; dessim maps it to a
    /// designation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conveyance_accepts: Option<String>,
    /// Which machine's conveyance this station feeds. An EasyLoop is one
    /// conveyor shared by TWO stations, so this is what keeps a machine's two
    /// halves from each being given a belt of their own. An index over the
    /// document's machines, stable for a given document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conveyor_group: Option<u32>,
}

impl Station {
    /// The cell the flows deliver to. A delivery needs one cell to drive to,
    /// and a station may present at several; this is the first of them until
    /// a policy chooses between a station's cells.
    pub fn delivery_cell(&self) -> CellId {
        self.presentation_points.first().expect("a station presents at one cell at least").presentation_cell
    }

    /// Every cell the station presents at, in the order of `presentation_points`.
    pub fn presentation_cells(&self) -> impl Iterator<Item = CellId> + '_ {
        self.presentation_points.iter().map(|point| point.presentation_cell)
    }

    /// Every cell of every presentation point, in order: each cell the station
    /// presents at, then the queue before it.
    pub fn presentation_and_queue_cells(&self) -> impl Iterator<Item = CellId> + '_ {
        self.presentation_points.iter().flat_map(|point| point.cells())
    }
}

/// Refuse a serialized station that states no presentation point: a station
/// presents at one cell at least, and `Station::delivery_cell` panics on one
/// that does not.
fn at_least_one_point<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<PresentationPoint>, D::Error> {
    let points = Vec::<PresentationPoint>::deserialize(deserializer)?;
    if points.is_empty() {
        return Err(D::Error::invalid_length(0, &"one presentation point at least"));
    }
    Ok(points)
}

fn default_exit_direction() -> CardinalDirection {
    CardinalDirection::East
}

/// A relay position -- cantilever shelf where VTMs and HTMs exchange totes.
/// Only present in RELAY system type.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RelayPosition {
    pub id: RelayId,
    /// The cell where the relay physically sits (on the rack block edge).
    pub position: CellId,
    /// Cell from which an HTM accesses this relay (same as position in Java).
    pub htm_access: CellId,
    /// VTM-side position, separate from the cell used to attribute its events.
    pub vtm_access: VtmPosition,
    /// Parent rack block.
    pub block: BlockId,
    /// Which side of the rack block this relay is on.
    pub side: BlockSide,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_round_trip_station() {
        let s = Station {
            id: StationId(1),
            outbound_role: None,
            position: CellId(10),
            presentation_points: vec![
                PresentationPoint { presentation_cell: CellId(11), queue_path: vec![CellId(9), CellId(10)] },
                PresentationPoint { presentation_cell: CellId(15), queue_path: vec![CellId(16)] },
            ],
            edge: Some(StationEdge::Bottom),
            handoff_queues: vec![],
            exit_direction: CardinalDirection::East,
            bins: None,
            dropoff_cell: Some(CellId(12)),
            collection_cell: Some(CellId(13)),
            order_presentation_cell: Some(CellId(14)),
            conveyance_accepts: Some("inventory".into()),
            conveyor_transit_ms: Some(45_000),
            conveyor_group: Some(0),
            station_type: StationType::Pick,
        };
        let json = serde_json::to_string(&s).unwrap();
        let s2: Station = serde_json::from_str(&json).unwrap();
        assert_eq!(s, s2);
    }

    #[test]
    fn station_stating_only_its_presentation_points_defaults_its_choreography() {
        let station: Station = serde_json::from_str(
            r#"{
                "id": 1,
                "position": 10,
                "presentation_points": [{ "presentation_cell": 11 }]
            }"#,
        )
        .unwrap();

        assert_eq!(
            station.presentation_points,
            vec![PresentationPoint { presentation_cell: CellId(11), queue_path: vec![] }]
        );
        assert_eq!(station.exit_direction, CardinalDirection::East);
        assert_eq!(station.station_type, StationType::Pick);
    }

    /// A station with no presentation point has no cell to deliver to, so
    /// deserializing one is refused.
    #[test]
    fn a_station_stating_no_presentation_point_is_refused() {
        let refusal = serde_json::from_str::<Station>(r#"{ "id": 1, "position": 10, "presentation_points": [] }"#)
            .expect_err("a station with no presentation point is read");
        assert!(refusal.to_string().contains("one presentation point at least"), "{refusal}");
    }

    #[test]
    fn serde_round_trip_relay() {
        let r = RelayPosition {
            id: RelayId(0),
            position: CellId(50),
            htm_access: CellId(50),
            vtm_access: VtmPosition::Cell(CellId(49)),
            block: BlockId(1),
            side: BlockSide::Left,
        };
        let json = serde_json::to_string(&r).unwrap();
        let r2: RelayPosition = serde_json::from_str(&json).unwrap();
        assert_eq!(r, r2);
    }
}
