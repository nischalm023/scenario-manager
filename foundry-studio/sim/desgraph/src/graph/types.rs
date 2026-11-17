//! Within `graph`, `types` is WHAT A CELL AND AN ARC SAY ABOUT THEMSELVES: the
//! node and edge weights the graph carries, and their serialized twins.
//!
//! Separate from `builder` and the graph itself because these are the plain
//! data a map on disk is made of -- no petgraph, no traversal -- so a reader
//! asking what an arc restricts, or a writer emitting a map, comes here and not
//! to the structure that holds them.

use serde::{Deserialize, Serialize};

use crate::types::*;

// ---------------------------------------------------------------------------
// Cell (node weight)
// ---------------------------------------------------------------------------

/// A single navigable cell in the warehouse graph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    pub id: CellId,
    pub kind: CellKind,
    pub zone: ZoneId,
    pub sector: Option<SectorId>,
    /// Logical grid position. None for special/junction cells.
    pub grid_pos: Option<GridPos>,
    /// Physical world position in mm. None if not computed.
    pub world_pos: Option<WorldPos>,
    /// Physical barcode label. None for parametric maps.
    pub barcode: Option<String>,
    /// HTMs may translate through this cell but cannot turn in place here.
    #[serde(default, skip_serializing_if = "is_false")]
    pub htm_rotation_forbidden: bool,
    /// Rack orientation for Storage/Relay cells. None for non-rack cells.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rack_orientation: Option<RackOrientation>,
    /// Station that owns this cell. Set for Presentation and QueueCell kinds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub station_id: Option<StationId>,
}

// ---------------------------------------------------------------------------
// Edge (edge weight)
// ---------------------------------------------------------------------------

/// Which pathfind targets an edge admits, when it does not admit all of them.
///
/// One CELL for an approach into an operator's standing spot: one spot, one
/// claimant, and nobody else may drive the lane. A machine's HANDOVERS for the
/// arc out of its through route, because the robots that come out of a
/// machine's queue are bound for either end of its belt and no single cell
/// names both.
///
/// Serializes as a bare cell id in the one-cell case, so a map written before
/// the pair existed reads back unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DestGate {
    Cell(CellId),
    Handovers([CellId; 2]),
}

impl DestGate {
    /// Whether a robot pathfinding to `dest` may take the edge.
    pub fn admits(self, dest: CellId) -> bool {
        match self {
            DestGate::Cell(cell) => cell == dest,
            DestGate::Handovers(cells) => cells.contains(&dest),
        }
    }

    /// The gate that admits exactly these cells, or `None` for none of them.
    /// Two names one machine's pair of handovers; anything longer has no
    /// reading here, so the caller states at most two.
    pub fn of(cells: &[CellId]) -> Option<DestGate> {
        match cells {
            [] => None,
            [one] => Some(DestGate::Cell(*one)),
            [a, b] => Some(DestGate::Handovers([*a, *b])),
            _ => None,
        }
    }
}

/// A directed connection between two cells.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub traversability: EdgeTraversability,
    // TODO: upstream u64 to desmap config types (currently f64 from JSON)
    /// Physical distance in mm from source cell center to target cell center.
    pub distance_mm: u64,
    /// If set, only HTMs whose pathfind target the gate admits may traverse.
    /// Used to restrict station access cells to assigned HTMs only, and the way
    /// out of a machine's queue to the robots that hand a carrier over there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dest_only: Option<DestGate>,
    /// If set, only HTMs whose pathfind target is in this zone may traverse.
    /// Used to restrict relay zone cells so HTMs only enter a relay zone
    /// when their destination relay is in that zone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vtm_zone_gate: Option<ZoneId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curve: Option<CircularArc>,
    /// Translation relative to the HTM chassis. Other robot types ignore it.
    #[serde(default, skip_serializing_if = "HtmDriveDirection::is_forward")]
    pub htm_drive: HtmDriveDirection,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HtmDriveDirection {
    #[default]
    Forward,
    Reverse,
}

impl HtmDriveDirection {
    pub fn is_forward(&self) -> bool {
        *self == Self::Forward
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl Edge {
    /// Convenience constructor for uniform grids where all edges are fully traversable.
    pub fn uniform(distance_mm: u64) -> Self {
        Self {
            traversability: EdgeTraversability::fully_traversable(),
            distance_mm,
            dest_only: None,
            vtm_zone_gate: None,
            curve: None,
            htm_drive: HtmDriveDirection::Forward,
        }
    }
}

/// Circular travel, optionally joined to cell centres by straight tangent leads.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CircularArc {
    pub center: WorldPos,
    pub clockwise: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<WorldPos>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<WorldPos>,
}

impl CircularArc {
    pub fn reversed(self) -> Self {
        Self { clockwise: !self.clockwise, start: self.end, end: self.start, ..self }
    }

    pub fn sweep(self, from: WorldPos, to: WorldPos) -> f64 {
        let (from, to) = (self.start.unwrap_or(from), self.end.unwrap_or(to));
        let a = (from.y_mm - self.center.y_mm).atan2(from.x_mm - self.center.x_mm);
        let b = (to.y_mm - self.center.y_mm).atan2(to.x_mm - self.center.x_mm);
        let sign = if self.clockwise { -1.0 } else { 1.0 };
        sign * (sign * (b - a)).rem_euclid(std::f64::consts::TAU)
    }

    pub fn length_mm(self, from: WorldPos, to: WorldPos) -> f64 {
        let (start, end) = (self.start.unwrap_or(from), self.end.unwrap_or(to));
        distance(from, start) + distance(start, self.center) * self.sweep(from, to).abs() + distance(end, to)
    }

    pub fn point(self, from: WorldPos, to: WorldPos, fraction: f64) -> WorldPos {
        if fraction <= 0.0 {
            return from;
        }
        if fraction >= 1.0 {
            return to;
        }
        let (a, b) = (self.start.unwrap_or(from), self.end.unwrap_or(to));
        let lead = distance(from, a);
        let radius = distance(a, self.center);
        let sweep = self.sweep(from, to);
        let arc_length = radius * sweep.abs();
        let along = self.length_mm(from, to) * fraction;
        let interpolate = |a: WorldPos, b: WorldPos, t: f64| WorldPos {
            x_mm: a.x_mm + (b.x_mm - a.x_mm) * t,
            y_mm: a.y_mm + (b.y_mm - a.y_mm) * t,
        };
        if along < lead {
            return interpolate(from, a, along / lead);
        }
        if along > lead + arc_length {
            return interpolate(b, to, (along - lead - arc_length) / distance(b, to));
        }
        let start = (a.y_mm - self.center.y_mm).atan2(a.x_mm - self.center.x_mm);
        let angle = start + sweep * (along - lead) / arc_length;
        WorldPos { x_mm: self.center.x_mm + radius * angle.cos(), y_mm: self.center.y_mm + radius * angle.sin() }
    }

    pub fn start_tangent(self, from: WorldPos) -> [f64; 2] {
        self.tangent(self.start.unwrap_or(from))
    }

    pub fn end_tangent(self, to: WorldPos) -> [f64; 2] {
        self.tangent(self.end.unwrap_or(to))
    }

    pub fn tangent(self, at: WorldPos) -> [f64; 2] {
        let dx = at.x_mm - self.center.x_mm;
        let dy = at.y_mm - self.center.y_mm;
        let sign = if self.clockwise { -1.0 } else { 1.0 };
        let radius = dx.hypot(dy);
        [-sign * dy / radius, sign * dx / radius]
    }
}

fn distance(a: WorldPos, b: WorldPos) -> f64 {
    (a.x_mm - b.x_mm).hypot(a.y_mm - b.y_mm)
}

#[cfg(test)]
mod curve_tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn tangent_leads_contribute_distance_and_interpolate_continuously_in_both_directions() {
        let p = |x_mm, y_mm| WorldPos { x_mm, y_mm };
        let (from, to) = (p(-600.0, 700.0), p(600.0, 900.0));
        let arc = CircularArc {
            center: p(0.0, 0.0),
            clockwise: false,
            start: Some(p(-600.0, 0.0)),
            end: Some(p(600.0, 0.0)),
        };
        let length = 700.0 + 600.0 * PI + 900.0;
        assert!((arc.length_mm(from, to) - length).abs() < 1e-9);
        for (along, expected) in [
            (0.0, from),
            (350.0, p(-600.0, 350.0)),
            (700.0, p(-600.0, 0.0)),
            (700.0 + 300.0 * PI, p(0.0, -600.0)),
            (700.0 + 600.0 * PI, p(600.0, 0.0)),
            (length, to),
        ] {
            assert!(distance(arc.point(from, to, along / length), expected) < 1e-8);
            assert!(distance(arc.reversed().point(to, from, 1.0 - along / length), expected) < 1e-8);
        }
        assert_eq!(arc.start_tangent(from), [0.0, -1.0]);
        assert_eq!(arc.end_tangent(to), [0.0, 1.0]);
        let encoded = serde_json::to_string(&arc).unwrap();
        assert_eq!(serde_json::from_str::<CircularArc>(&encoded).unwrap(), arc);
    }

    #[test]
    fn older_circle_geometry_needs_no_tangent_points() {
        let arc: CircularArc = serde_json::from_str(r#"{"center":{"x_mm":0.0,"y_mm":0.0},"clockwise":false}"#).unwrap();
        let from = WorldPos { x_mm: -600.0, y_mm: 0.0 };
        let to = WorldPos { x_mm: 600.0, y_mm: 0.0 };
        assert_eq!((arc.start, arc.end), (None, None));
        assert!((arc.length_mm(from, to) - 600.0 * PI).abs() < 1e-9);
        assert!(distance(arc.point(from, to, 0.5), WorldPos { x_mm: 0.0, y_mm: -600.0 }) < 1e-8);
    }
}

// ---------------------------------------------------------------------------
// Serializable edge record (for desmap JSON format)
// ---------------------------------------------------------------------------

/// A directed edge as stored in the desmap JSON format.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct EdgeRecord {
    pub(super) from: CellId,
    pub(super) to: CellId,
    #[serde(flatten)]
    pub(super) edge: Edge,
}

#[cfg(test)]
mod gate_tests {
    use super::*;

    const DROPOFF: CellId = CellId(7);
    const COLLECT: CellId = CellId(9);
    const ELSEWHERE: CellId = CellId(11);

    /// What the way out of a machine's queue admits: EITHER end of its belt, and
    /// no other destination. A regression that let only one handover through
    /// would seal half the machine off, and one that admitted everything would
    /// reopen the through route to robots with no business inside.
    #[test]
    fn a_pair_admits_both_handovers_and_nothing_else() {
        let way_out = DestGate::Handovers([DROPOFF, COLLECT]);
        assert!(way_out.admits(DROPOFF), "a robot bound for the belt's near end may come out");
        assert!(way_out.admits(COLLECT), "and so may one bound for its far end");
        assert!(!way_out.admits(ELSEWHERE), "a robot bound anywhere else may not");

        let approach = DestGate::Cell(DROPOFF);
        assert!(approach.admits(DROPOFF));
        assert!(!approach.admits(COLLECT), "an approach admits its one standing spot and no other");
    }

    /// `of` is what turns a station's handover list into a gate, and the list
    /// is empty for a station with no belt and one long for a belt with one
    /// end. Longer than two has no reading: a gate that named three
    /// destinations would be a machine shape nothing here describes.
    #[test]
    fn a_gate_is_built_from_one_or_two_cells_and_no_other_count() {
        assert_eq!(DestGate::of(&[]), None);
        assert_eq!(DestGate::of(&[DROPOFF]), Some(DestGate::Cell(DROPOFF)));
        assert_eq!(DestGate::of(&[DROPOFF, COLLECT]), Some(DestGate::Handovers([DROPOFF, COLLECT])));
        assert_eq!(DestGate::of(&[DROPOFF, COLLECT, ELSEWHERE]), None);
    }

    /// The shape a map on disk carries. One cell is a BARE ID -- the same text
    /// the field held when it was an `Option<CellId>`, so a map written before
    /// the pair existed reads back as the gate it always was -- and a pair is a
    /// two-element array.
    #[test]
    fn a_gate_serializes_as_an_id_or_a_pair_and_reads_back_the_same() {
        let one = serde_json::to_value(DestGate::Cell(DROPOFF)).expect("a gate serializes");
        assert_eq!(one, serde_json::json!(7));
        let pair = serde_json::to_value(DestGate::Handovers([DROPOFF, COLLECT])).expect("a gate serializes");
        assert_eq!(pair, serde_json::json!([7, 9]));

        for gate in [DestGate::Cell(DROPOFF), DestGate::Handovers([DROPOFF, COLLECT])] {
            let mut edge = Edge::uniform(1000);
            edge.dest_only = Some(gate);
            let written = serde_json::to_value(edge).expect("an edge serializes");
            let read: Edge = serde_json::from_value(written.clone()).expect("an edge reads back");
            assert_eq!(read.dest_only, Some(gate), "the gate survives the round trip as the variant it was");
            // And it is the FIELD that carries the two shapes: an approach's
            // `dest_only` is the same bare number an `Option<CellId>` wrote,
            // so a map on disk from before the pair needs no migration.
            let expected = match gate {
                DestGate::Cell(_) => serde_json::json!(7),
                DestGate::Handovers(_) => serde_json::json!([7, 9]),
            };
            assert_eq!(written["dest_only"], expected);
        }
    }
}
