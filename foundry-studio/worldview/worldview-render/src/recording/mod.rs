//! Within `worldview-render`, `recording` is A RUN'S OWN WORDS TURNED INTO
//! WHAT IS DRAWN: the state a `.worldview` states at one instant, read as the
//! robots, containers, travels and reservations a [`Playback`] is built from.
//!
//! It exists so that the two viewers cannot derive the picture twice. desvis
//! reaches a run through the simulator's slice records and the browser viewer
//! through the recording's own frames; where those two paths would each decide
//! how far along its leg a robot is, what a container holds, or which way a
//! robot faces part-way through a turn, this decides once. A difference
//! between the two viewers' instance rows is a defect here.
//!
//! Where a place is concerned this module decides nothing itself: it asks
//! [`RunState::at`], which is the one answer to what a run says at an instant,
//! and the recording states that answer in millimetres, so the floor is never
//! consulted to find out where a robot is. What this adds is everything `at`
//! does not carry — the appearance a container has by then, which relay cells
//! are held, and the facing a robot is part-way through turning to.

mod heading;
pub mod overlays;

use std::array::from_fn;

use desunits::{RobotId, SimTime, ToteId};
use rustc_hash::FxHashMap;
use worldview::frames::playback::{MoverSteps, Position, RobotClass, Step, ToteTransit};
use worldview::frames::state::{HeatState, RobotAt, RobotPlace};
use worldview::{Delta, Error, Result, RunState};

use crate::scene::{
    BotHighlight, BotType, ContainerKind, Designation, HeatmapData, Overlays, Place, Playback, Robot, Scene, Tote,
    ToteAppearance,
};

/// The two layers a recording counts robot visits in, in the order the
/// overlay's two tint columns take them.
const VISIT_LAYERS: [&str; 2] = ["nav_visits_vtm", "nav_visits_htm"];

/// One instant of a run, in the renderer's words: everything [`Scene::moment`]
/// needs except the overlays a viewer chooses for itself.
///
/// The lists are owned because they are built here and borrowed by the
/// `Playback` that draws them; both are sorted by id, which is what makes a
/// container's shelf slot the same one frame to frame. The reservation flags
/// are dense by cell id, like the floor's own cell list.
#[derive(Debug, Default)]
pub struct Drawn {
    pub at_ms: u64,
    pub robots: Vec<Robot>,
    pub totes: Vec<Tote>,
    pub transits: Vec<ToteTransit>,
    pub reserved_cells: Vec<bool>,
}

impl Drawn {
    pub fn playback<'a>(&'a self, overlays: Overlays<'a>) -> Playback<'a> {
        Playback { at_ms: self.at_ms, robots: &self.robots, totes: &self.totes, transits: &self.transits, overlays }
    }
}

/// What a run says at `t`, in the renderer's words.
///
/// `state` stands where `slice` begins — that is what [`RunState`] means by
/// standing — and `t` is an instant inside that slice. A viewer whose cursor
/// is at the state's own instant, with no slice read yet, passes a default
/// delta and `state.reached`.
pub fn from_recording(
    scene: &Scene,
    state: &RunState,
    slice: &Delta,
    t: SimTime,
    facing_before: &FxHashMap<RobotId, f32>,
) -> Result<Drawn> {
    let begins = state.reached;
    let at = state.at(begins, t, slice)?;
    let elapsed_ms = t.as_ms() - begins.as_ms();
    let empty = Default::default();
    let playback = slice.playback.as_ref().unwrap_or(&empty);

    let moved: FxHashMap<_, _> = playback.movers.iter().map(|mover| (mover.robot, mover)).collect();
    let mut robots: Vec<Robot> = Vec::with_capacity(at.robots.len());
    for (id, robot_at) in &at.robots {
        let Some(held) = state.playback.robots.get(id) else { continue };
        // A robot the run states no facing for keeps the one it was last drawn
        // with, and faces along +x if it has never been drawn — desvis's own
        // rule, and the reason the previous frame's facings are passed in.
        let opened_on = match robot_at.heading_mrad {
            Some(mrad) => radians(mrad),
            None => facing_before.get(id).copied().unwrap_or(0.0),
        };
        // A mover's own steps say more than the place `at` reduces them to:
        // where it is between two poses, and how far round a turn it has got.
        // A robot that stayed put has none, and its held place is the whole
        // answer.
        let walk = moved.get(id).map(|mover| Walk::of(mover, held, elapsed_ms));
        robots.push(Robot {
            id: *id,
            class: class_of(held.class),
            at: match walk.as_ref().and_then(Walk::placed) {
                Some(place) => place,
                None => standing(robot_at),
            },
            heading_rad: walk.as_ref().map_or(opened_on, |walk| walk.facing(opened_on)),
            highlight: BotHighlight::None,
        });
    }
    robots.sort_by_key(|robot| robot.id.0);

    let mut appearances = appearances(state)?;
    // A container born inside this slice states its two permanent facts as it
    // is placed, and its own contents records restate how full it is; both are
    // stamped at an offset, so neither is true before that offset is reached.
    for born in playback.tote_creations.iter().filter(|born| born.offset.as_ms() <= elapsed_ms) {
        let appearance = appearances.entry(born.tote).or_default();
        appearance.container = container_of(born.tote_type)?;
        appearance.designation = designation_of(born.designation)?;
    }
    for contents in playback.tote_contents.iter().filter(|contents| contents.offset.as_ms() <= elapsed_ms) {
        appearances.entry(contents.tote).or_default().contents_units = contents.units;
    }
    let mut totes: Vec<Tote> = at
        .totes
        .iter()
        .map(|(id, location)| Tote {
            id: *id,
            at: *location,
            appearance: appearances.get(id).copied().unwrap_or_default(),
        })
        .collect();
    totes.sort_by_key(|tote| tote.id.0);

    // The slice's own travels and no others, which is what desvis hands the
    // renderer from its slice record. The state also keeps the journeys still
    // in flight from earlier slices, and feeding those in as well would place
    // a container part-way along a belt it entered before the cursor's
    // slice — the better picture, and a change to BOTH viewers at once, since
    // the two are held byte for byte against each other. It belongs in the
    // round after this PR, not inside it.
    let transits = playback.tote_transits.clone();

    let mut reserved_cells = vec![false; scene.cells().len()];
    let mut held: Vec<_> = state.playback.reserved_relay_cells.iter().copied().collect();
    for relay in playback.relay_transitions.iter().filter(|relay| relay.offset.as_ms() <= elapsed_ms) {
        match relay.reserved {
            true => held.push(relay.cell),
            false => held.retain(|cell| *cell != relay.cell),
        }
    }
    for cell in held {
        if let Some(flag) = reserved_cells.get_mut(cell.0 as usize) {
            *flag = true;
        }
    }

    Ok(Drawn { at_ms: t.as_ms(), robots, totes, transits, reserved_cells })
}

/// The overlay the run's visit counters paint, over a floor of `cells` cells.
///
/// A recording counts visits in named layers; two of them are the robot
/// classes the overlay tints, and a run that counted neither paints nothing.
pub fn visit_counts(heat: &HeatState, cells: usize) -> HeatmapData {
    let [vtm_counts, htm_counts] = VISIT_LAYERS.map(|layer| {
        let mut counts = vec![0u32; cells];
        for (cell, count) in heat.layers.get(layer).into_iter().flatten() {
            if let Some(slot) = counts.get_mut(cell.0 as usize) {
                *slot = *count as u32;
            }
        }
        counts
    });
    let vtm_max = vtm_counts.iter().copied().max().unwrap_or(0);
    let htm_max = htm_counts.iter().copied().max().unwrap_or(0);
    HeatmapData { vtm_counts, htm_counts, vtm_max, htm_max }
}

/// One mover's slice as a path: where each of its steps leaves it, where it
/// opened, and which step the instant falls in.
///
/// Every point here comes from the recording, which states each step's
/// millimetres outright; the floor says where cells are and this asks it
/// nothing.
struct Walk<'a> {
    steps: &'a [Step],
    places: Vec<[f64; 2]>,
    opened_at: [f64; 2],
    opened_on: u32,
    active: Option<(usize, f32)>,
}

impl<'a> Walk<'a> {
    fn of(mover: &'a MoverSteps, stood: &RobotPlace, elapsed_ms: u64) -> Self {
        Self {
            steps: &mover.steps,
            places: mover.steps.iter().map(|step| point(step.at)).collect(),
            opened_at: point(stood.at),
            opened_on: stood.cell.0,
            active: active_step(&mover.steps, elapsed_ms),
        }
    }

    /// Where the step before `index` left the robot, and the cell it counts as
    /// standing on while it walks the one after.
    fn left_by(&self, index: usize) -> ([f64; 2], u32) {
        match index.checked_sub(1) {
            Some(before) => (self.places[before], self.steps[before].cell.0),
            None => (self.opened_at, self.opened_on),
        }
    }

    /// Where the robot is: part-way along the step the instant falls in, or
    /// wherever the slice's last step left it.
    fn placed(&self) -> Option<Place> {
        let Some((index, progress)) = self.active else {
            let last = self.steps.len().checked_sub(1)?;
            return Some(Place { cell: self.steps[last].cell.0, pos_mm: self.places[last] });
        };
        let (before, cell) = self.left_by(index);
        let along = f64::from(progress);
        let to = self.places[index];
        Some(Place { cell, pos_mm: from_fn(|axis| before[axis] + (to[axis] - before[axis]) * along) })
    }

    fn facing(&self, opened_on: f32) -> f32 {
        match self.active {
            Some((index, progress)) => {
                heading::part_way(self.steps, &self.places, self.opened_at, index, progress, opened_on)
            }
            None => heading::settled(self.steps, &self.places, self.opened_at, opened_on),
        }
    }
}

/// Where a robot that did not move this slice stands: the place the state
/// holds, which the recording stated in millimetres.
fn standing(robot: &RobotAt) -> Place {
    Place { cell: robot.cell.0, pos_mm: point(robot.at) }
}

/// Which step the instant falls in and how far through it, or nil once the
/// instant is past everything the slice states.
///
/// The fraction is worked out in f64 and then narrowed, which is desvis's own
/// arithmetic: a wider one puts a robot a few ten-thousandths of a millimetre
/// from where the other viewer puts it, and the two pictures are held to the
/// bit.
fn active_step(steps: &[Step], elapsed_ms: u64) -> Option<(usize, f32)> {
    let mut walked_ms = 0u64;
    for (index, step) in steps.iter().enumerate() {
        let step_ms = step.dt.as_ms();
        walked_ms += step_ms;
        if walked_ms > elapsed_ms && step_ms > 0 {
            let progress = (elapsed_ms + step_ms - walked_ms) as f64 / step_ms as f64;
            return Some((index, progress as f32));
        }
    }
    None
}

/// The floor-plane millimetres a recorded position stands for. The height
/// rides beside them in the recording and the renderer does not read it: a
/// VTM's drawn height is still the rack-top proxy `place_z` works out, so this
/// round changes nothing about what is drawn.
fn point(at: Position) -> [f64; 2] {
    [f64::from(at.x_mm), f64::from(at.y_mm)]
}

/// Every container's appearance as the slice opened, before this slice's own
/// births and transfers are applied to it.
fn appearances(state: &RunState) -> Result<FxHashMap<ToteId, ToteAppearance>> {
    state
        .playback
        .totes
        .iter()
        .map(|(id, tote)| {
            Ok((
                *id,
                ToteAppearance {
                    container: container_of(tote.tote_type)?,
                    designation: designation_of(tote.designation)?,
                    contents_units: tote.contents_units,
                },
            ))
        })
        .collect()
}

fn class_of(class: RobotClass) -> BotType {
    match class {
        RobotClass::Htm => BotType::Htm,
        RobotClass::Vtm => BotType::Vtm,
        RobotClass::Rtp => BotType::Rtp,
    }
}

/// The recording's container-type byte. A byte no container answers to is
/// damage: drawing it as a tote would put a carton on a shelf and say nothing.
fn container_of(byte: u8) -> Result<ContainerKind> {
    match byte {
        0 => Ok(ContainerKind::Tray),
        1 => Ok(ContainerKind::Tote),
        2 => Ok(ContainerKind::Carton),
        found => Err(Error::UnknownContainerByte { what: "container type", found }),
    }
}

fn designation_of(byte: u8) -> Result<Designation> {
    match byte {
        0 => Ok(Designation::Inventory),
        1 => Ok(Designation::Order),
        found => Err(Error::UnknownContainerByte { what: "pool", found }),
    }
}

/// The recording carries whole milliradians and nothing finer, which is the
/// round-2a ruling; this is the arithmetic the writer's own inverse uses.
fn radians(milliradians: i32) -> f32 {
    (f64::from(milliradians) / 1_000.0) as f32
}

#[cfg(test)]
mod tests;
