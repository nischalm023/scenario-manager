//! Within `desmap::map`, `queue_edges` is the LANE DISCIPLINE: given a lane as a
//! list of cell ids, it cuts the graph down to the one way through it.
//!
//! Geometry-free on purpose -- it takes cells and gives back a graph, and every
//! question about where those cells are was answered before it was called. Lived
//! under the v3 compiler until the sunset; the world-protocol loader is its
//! consumer now.
//!
//! Two kinds of lane come through here, and they differ in WHERE the gate sits
//! rather than in whether there is one. An APPROACH into an operator's standing
//! spot gates every forward edge to that spot, so only the robot that claimed it
//! may drive the lane at all. A machine's through route gates the LAST arc, the
//! one out of the queue and onto the handover it serves, to the two cells a
//! carrier changes hands at; everything behind that arc is open.
//!
//! WHY THE MACHINE'S GATE IS AT THE FAR END. Moved here on 2026-09-05 and NOT
//! YET RATIFIED by the layout owner: the 2026-09-04 ruling put a machine's
//! dropoff outside its own lane, and the gate could not stay where it was once
//! it did.
//!
//! A machine's lane is a QUEUE for its collection cell: robots that have set a
//! carrier on the belt wait in it for one to come back, each holding a cell of
//! it open-ended. Two kinds of robot therefore have business inside -- one bound
//! for the handover and one bound for a cell of the queue -- and only a robot
//! MEANING TO COME OUT THE FAR END is passing through. The far end is the one
//! arc a passer-through must take and a waiting robot need not, so that is where
//! the gate belongs.
//! Gating the near end instead admits the collector and turns back the robot
//! that wants to wait, which leaves it standing on the handover; the machine's
//! only entrance is then held by an idle robot and the run grinds to a halt
//! (measured on `minimal_easyloop_collision_spec.json`: 235 s of sim in under a
//! second, 240 s in over 400).
//!
//! WHAT THAT PLACEMENT RULES OUT, said plainly: a robot standing IN a machine's
//! queue and then sent somewhere outside cannot leave, because the only way on
//! from the queue is the arc onto the handover and that arc admits only a robot
//! bound for a handover. Nothing does that today -- a robot in the queue is a
//! committed collector, held out of the available fleet until the belt hands it
//! a carrier, and its next destination is the collection cell it is queued for.
//! A flow that re-tasked a queued robot to somewhere else would strand it, and
//! this is the sentence to read first if one ever does.

use rustc_hash::{FxHashMap, FxHashSet};

use crate::graph::{DestGate, WarehouseGraph, WarehouseGraphBuilder};
use crate::types::*;

/// Enforce strict one-way queue paths and presentation cell access by
/// removing all edges except the allowed forward corridor.
///
/// For each station with a non-empty queue_path, the only edges permitted
/// that touch a queue or presentation cell are:
///
/// 1. **Entry**: one inbound edge into the first queue cell from the
///    tail neighbor (opposite the queue's forward direction).
/// 2. **Forward**: each queue cell to the next (`q[i]` -> `q[i+1]`).
/// 3. **Forward**: last queue cell to the presentation cell.
/// 4. **Exit**: one outbound edge from the presentation cell in the
///    station's `exit_direction`.
///
/// All other edges where either endpoint is a queue or presentation cell
/// are dropped entirely (removed from the graph).
///
/// Which of those edges carry a `dest_only` gate is [`CorridorAccess`]'s
/// answer, not this list's.
///
/// One station's queue corridor in CELL IDS: the geometry-free core of the
/// restriction, shared by the grid compiler (which derives entry/exit from
/// grid math) and the world-protocol loader (which derives them from world
/// positions). `entry_from`/`exit_to` name the single legal approach and
/// departure cells; `None` means that edge simply is not granted.
pub struct QueueCorridor {
    pub queue: Vec<CellId>,
    pub presentation: CellId,
    pub entry_from: Option<CellId>,
    pub exit_to: Option<CellId>,
    /// Who may drive the corridor, and which of its edges says so.
    pub access: CorridorAccess,
}

/// Who a corridor admits.
pub enum CorridorAccess {
    /// An APPROACH into an operator's standing spot. Every forward edge is
    /// gated to the presentation cell: one spot, one claimant, and the gate is
    /// what stops the queue becoming a shortcut across the floor.
    Approach,
    /// A machine's THROUGH ROUTE. The arc out of the queue and onto the cell it
    /// serves is gated to the machine's two handovers -- a robot with no carrier
    /// to hand over there has no business coming out the far end -- and every
    /// arc behind it is left open, so a robot may drive in to WAIT for the belt
    /// as well as to meet it.
    Machine { handovers: DestGate },
}

/// Two corridors claim one arc and disagree about who may drive it.
///
/// Refused rather than resolved: the older reading let whichever corridor was
/// written last decide, and an ungated claim silently erased a gated one's
/// destination restriction -- a machine's through route quietly reopening an
/// operator's approach to every robot on the floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorridorConflict {
    pub from: CellId,
    pub to: CellId,
}

impl std::fmt::Display for CorridorConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the arc {:?} -> {:?} is claimed by two lanes that admit different robots; one lane's gate would \
             silently replace the other's",
            self.from, self.to
        )
    }
}

impl std::error::Error for CorridorConflict {}

/// Enforce strict one-way queue corridors: for each corridor, the only edges
/// permitted that touch a queue or presentation cell are the entry edge into
/// the first queue cell, the forward chain through the queue into the
/// presentation cell, and the exit edge. Every other edge touching those cells
/// is dropped.
pub fn restrict_queue_corridors(
    graph: &WarehouseGraph,
    corridors: &[QueueCorridor],
) -> Result<WarehouseGraph, CorridorConflict> {
    // Collected restricted cells: queue cells + presentation cells.
    let mut restricted_cells: FxHashSet<u32> = FxHashSet::default();
    // What each claimed arc admits: `None` for every robot, `Some(gate)` for
    // the robots the gate names. An arc claimed twice must be claimed the same
    // way both times, which is what `claim` refuses.
    let mut claims: FxHashMap<(u32, u32), Option<DestGate>> = FxHashMap::default();
    let mut conflict: Option<CorridorConflict> = None;
    let mut claim = |from: u32, to: u32, gate: Option<DestGate>, conflict: &mut Option<CorridorConflict>| match claims
        .get(&(from, to))
    {
        Some(existing) if *existing != gate => {
            conflict.get_or_insert(CorridorConflict { from: CellId(from), to: CellId(to) });
        }
        _ => {
            claims.insert((from, to), gate);
        }
    };

    for c in corridors {
        if c.queue.is_empty() {
            continue;
        }
        let pres_id = c.presentation.0;
        for q in &c.queue {
            restricted_cells.insert(q.0);
        }
        restricted_cells.insert(pres_id);

        // Three arcs of one lane, and which robots each admits: the arc in from
        // the floor, the chain between the queue's own cells, and the arc out of
        // the queue onto the cell it serves. An approach gates all three to the
        // standing spot it feeds -- one spot, one claimant. A machine gates only
        // the last, to the cells a carrier changes hands at; see the module doc
        // for why the gate is at that end and not the other.
        let (door_gate, forward_gate, serving_gate) = match &c.access {
            CorridorAccess::Approach => {
                let spot = Some(DestGate::Cell(c.presentation));
                (spot, spot, spot)
            }
            CorridorAccess::Machine { handovers } => (None, None, Some(*handovers)),
        };

        // Forward: consecutive queue cells, then last queue cell -> presentation.
        for pair in c.queue.windows(2) {
            claim(pair[0].0, pair[1].0, forward_gate, &mut conflict);
        }
        claim(c.queue[c.queue.len() - 1].0, pres_id, serving_gate, &mut conflict);

        if let Some(entry) = c.entry_from {
            claim(entry.0, c.queue[0].0, door_gate, &mut conflict);
        }
        if let Some(exit) = c.exit_to {
            // The way out is never gated: a robot leaving is by definition
            // bound for somewhere that is not this lane.
            claim(pres_id, exit.0, None, &mut conflict);
        }
    }

    if let Some(conflict) = conflict {
        return Err(conflict);
    }

    if restricted_cells.is_empty() {
        return Ok(clone_graph(graph));
    }

    let mut builder = WarehouseGraphBuilder::with_capacity(graph.cell_count(), graph.edge_count());
    for cell in graph.cells() {
        builder.add_cell(cell.clone());
    }

    for (src, tgt, edge) in graph.edges() {
        let src_restricted = restricted_cells.contains(&src.0);
        let tgt_restricted = restricted_cells.contains(&tgt.0);

        if !src_restricted && !tgt_restricted {
            // Neither endpoint is restricted — pass through unchanged.
            builder.add_edge(src, tgt, *edge);
        } else if let Some(&gate) = claims.get(&(src.0, tgt.0)) {
            let mut e = *edge;
            e.dest_only = gate;
            builder.add_edge(src, tgt, e);
        }
        // else: edge touches a restricted cell but is not claimed — drop it.
    }

    Ok(builder.build())
}

fn clone_graph(graph: &WarehouseGraph) -> WarehouseGraph {
    let mut builder = WarehouseGraphBuilder::with_capacity(graph.cell_count(), graph.edge_count());
    for cell in graph.cells() {
        builder.add_cell(cell.clone());
    }
    for (src, tgt, edge) in graph.edges() {
        builder.add_edge(src, tgt, *edge);
    }
    builder.build()
}
