//! Within `recording`, `overlays` is WHAT IS PAINTED OVER THE CELLS: how many
//! times a slice's robots stood on each cell, and how much of a cell's
//! exposure the HTMs spent waiting or crossing it and how fast.
//!
//! Both viewers paint these, so both derive them here. desvis counted visits
//! off the simulator's step journal and measured traffic off the observer's
//! motion spans; a browser has the same facts in a slice's own delta, and a
//! second derivation of either would be two answers to one question — which is
//! what the shared renderer exists to prevent.
//!
//! What a viewer still owns is WHEN: desvis measures from the run's start to
//! its cursor, and a browser measures from the keyframe its attachment opened
//! on. An attachment that did not open at zero has not been told about the
//! slices before it and cannot count them.

use desunits::{RobotId, SimTime};
use rustc_hash::{FxHashMap, FxHashSet};
use worldview::frames::motion::MotionSpan;
use worldview::frames::playback::{RobotClass, StepKind};
use worldview::{Delta, RunState};

use crate::scene::{BotType, HeatmapData, HeatmapMode, Scene, TrafficTints};

/// Duration-weighted speed bins: `[0, .5)`, `[.5, 1)`, … `[4.5, 5)`, `[5, ∞)`
/// metres per second. The simulator's own bin count, because the tints are
/// held against the numbers it reports.
pub const SPEED_BINS: usize = 11;

/// How many times each class of robot stood on each cell during one slice.
///
/// A robot counts once for the cell it began the slice on and once for every
/// cell a step took it to, which is what desvis has always counted — a visit,
/// not a dwell. Both viewers add to one of these.
pub struct VisitCounter {
    vtm: Vec<u32>,
    htm: Vec<u32>,
}

impl VisitCounter {
    pub fn over(cells: usize) -> Self {
        Self { vtm: vec![0; cells], htm: vec![0; cells] }
    }

    pub fn visited(&mut self, class: BotType, cell: u32) {
        let counts = match class {
            BotType::Vtm => &mut self.vtm,
            _ => &mut self.htm,
        };
        if let Some(count) = counts.get_mut(cell as usize) {
            *count += 1;
        }
    }

    pub fn finish(self) -> HeatmapData {
        let vtm_max = self.vtm.iter().copied().max().unwrap_or(0);
        let htm_max = self.htm.iter().copied().max().unwrap_or(0);
        HeatmapData { vtm_counts: self.vtm, htm_counts: self.htm, vtm_max, htm_max }
    }
}

/// One slice's visits, from the run's own frames: where each robot stood when
/// the slice opened, and every cell its steps named.
pub fn slice_visits(state: &RunState, slice: &Delta, cells: usize) -> HeatmapData {
    let mut counter = VisitCounter::over(cells);
    let moved = slice.playback.as_ref().map(|playback| &playback.movers);
    for (robot, held) in &state.playback.robots {
        let class = match held.class {
            RobotClass::Vtm => BotType::Vtm,
            RobotClass::Htm => BotType::Htm,
            RobotClass::Rtp => BotType::Rtp,
        };
        counter.visited(class, held.cell.0);
        for mover in moved.into_iter().flatten().filter(|mover| mover.robot == *robot) {
            for step in &mover.steps {
                counter.visited(class, step.cell.0);
            }
        }
    }
    counter.finish()
}

/// What one cell's HTM traffic adds up to: how far and how long robots moved
/// over it, how long they stood on it, and how that moving time is spread
/// across speeds.
#[derive(Clone, Debug, Default)]
pub struct CellTraffic {
    pub distance_mm: f64,
    pub moving_ms: f64,
    pub wait_ms: f64,
    pub rotation_ms: f64,
    pub observed_ms: f64,
    pub speed_ms: [f64; SPEED_BINS],
}

impl CellTraffic {
    pub fn moving_speed_m_s(&self) -> Option<f64> {
        (self.moving_ms > 0.0).then(|| self.distance_mm / self.moving_ms)
    }
}

/// The run's HTM traffic up to an instant, and the tints it paints.
///
/// It is built up slice by slice because that is how a browser meets a run.
/// The whole slices go into [`Traffic::took`]; the one the cursor stands in is
/// measured into a copy each time the cursor moves, since the next move may be
/// backwards.
pub struct Traffic {
    whole: FxHashMap<u32, CellTraffic>,
    /// The whole slices plus however much of the current one the cursor has
    /// reached — what the tints are read from.
    now: FxHashMap<u32, CellTraffic>,
    max_wait_ms: f64,
    excluded: FxHashSet<u32>,
}

impl Traffic {
    /// Nothing measured yet, over a floor that says which cells the waiting
    /// tint leaves out.
    pub fn over(scene: &Scene) -> Self {
        let excluded = scene.cells().iter().flatten().filter(|cell| cell.wait_excluded).map(|cell| cell.id).collect();
        Self { whole: FxHashMap::default(), now: FxHashMap::default(), max_wait_ms: 0.0, excluded }
    }

    /// Take in a slice that the cursor has passed entirely.
    pub fn took(&mut self, begins: SimTime, ends: SimTime, state: &RunState, slice: &Delta) {
        measure(&mut self.whole, begins, ends, state, slice);
    }

    /// Measure up to `until` inside the slice beginning at `begins`, which the
    /// cursor stands in, and settle the tints.
    pub fn reaching(&mut self, begins: SimTime, until: SimTime, state: &RunState, slice: &Delta) {
        self.now.clone_from(&self.whole);
        measure(&mut self.now, begins, until, state, slice);
        self.max_wait_ms = self
            .now
            .iter()
            .filter(|(cell, _)| !self.excluded.contains(cell))
            .map(|(_, row)| row.wait_ms)
            .fold(0.0, f64::max);
    }

    pub fn cell(&self, cell: u32) -> Option<&CellTraffic> {
        self.now.get(&cell)
    }
}

/// The tints the two measured overlays paint, as the renderer asks for them —
/// desvis's own arithmetic, so the two viewers colour a cell alike.
impl TrafficTints for Traffic {
    fn intensity(&self, cell: u32, mode: HeatmapMode) -> f32 {
        let Some(row) = self.now.get(&cell) else {
            return -1.0;
        };
        match mode {
            HeatmapMode::HtmWait if !self.excluded.contains(&cell) && row.observed_ms > 0.0 => {
                (row.wait_ms / self.max_wait_ms.max(1.0)) as f32
            }
            HeatmapMode::HtmSpeed => row.moving_speed_m_s().map(|speed| (speed / 5.0).min(1.0) as f32).unwrap_or(-1.0),
            _ => -1.0,
        }
    }
}

/// Add one slice's HTM traffic, up to `until`, to what has been measured.
///
/// Robots are taken in ascending id because the sums are floating point and
/// the order they are added in is part of the answer.
fn measure(cells: &mut FxHashMap<u32, CellTraffic>, begins: SimTime, until: SimTime, state: &RunState, slice: &Delta) {
    if until <= begins {
        return;
    }
    let mut htms: Vec<_> = state
        .playback
        .robots
        .iter()
        .filter(|(_, held)| held.class == RobotClass::Htm)
        .map(|(robot, held)| (robot.0, held.cell.0))
        .collect();
    htms.sort_unstable();

    for (robot, start_cell) in htms {
        let id = RobotId(robot);
        for span in
            slice.motion.as_ref().and_then(|motion| motion.get(id)).map(|robot| &robot.spans).into_iter().flatten()
        {
            accumulate(span, begins, until, cells);
        }
        let steps = slice
            .playback
            .as_ref()
            .and_then(|playback| playback.movers.iter().find(|mover| mover.robot == id))
            .map(|mover| mover.steps.as_slice())
            .unwrap_or_default();

        let (mut time, mut cell) = (begins.as_ms(), start_cell);
        for step in steps {
            let end = time.saturating_add(step.dt.as_ms());
            let ms = end.min(until.as_ms()).saturating_sub(time) as f64;
            if ms > 0.0 && !step.kind.travels() {
                let row = cells.entry(step.cell.0).or_default();
                row.observed_ms += ms;
                match step.kind {
                    StepKind::Wait => row.wait_ms += ms,
                    _ => row.rotation_ms += ms,
                }
            }
            time = end;
            cell = step.cell.0;
            if time >= until.as_ms() {
                break;
            }
        }
        // Whatever is left of the slice, the robot spent standing where its
        // last step left it.
        if time < until.as_ms() {
            let row = cells.entry(cell).or_default();
            let ms = (until.as_ms() - time) as f64;
            row.observed_ms += ms;
            row.wait_ms += ms;
        }
    }
}

/// Integrate the observed part of one travel's acceleration, cruise and
/// deceleration, splitting at speed-bin boundaries and at the link's midpoint
/// so distance and duration are conserved however the run was sliced.
///
/// A link belongs to its source for the first half of its travel distance and
/// to its destination for the second, curves and map gaps included. This is
/// `dessim`'s `MotionSpan::accumulate` over the recording's own spans: the
/// simulator cannot be linked into a browser, and the two are held together by
/// the equality test that compares both viewers' tint columns.
fn accumulate(span: &MotionSpan, from: SimTime, until: SimTime, cells: &mut FxHashMap<u32, CellTraffic>) {
    let lo = from.as_ms().max(span.start.as_ms()) as f64;
    let hi = until.as_ms().min(span.end.as_ms()) as f64;
    let duration_s = span.end.as_ms().saturating_sub(span.start.as_ms()) as f64 / 1000.0;
    if hi <= lo || duration_s <= 0.0 || span.distance_mm <= 0.0 {
        return;
    }
    let phases = match span.profile.as_ref().filter(|p| p.total_time_s > 0.0 && p.total_distance_mm > 0.0) {
        Some(p) => {
            let time_scale = duration_s / p.total_time_s;
            let velocity_scale = span.distance_mm / p.total_distance_mm / time_scale;
            [
                (
                    p.accel_time_s * time_scale,
                    p.entry_velocity_mm_s * velocity_scale,
                    p.peak_velocity_mm_s * velocity_scale,
                ),
                (
                    p.cruise_time_s * time_scale,
                    p.peak_velocity_mm_s * velocity_scale,
                    p.peak_velocity_mm_s * velocity_scale,
                ),
                (
                    p.decel_time_s * time_scale,
                    p.peak_velocity_mm_s * velocity_scale,
                    p.exit_velocity_mm_s * velocity_scale,
                ),
            ]
        }
        None => {
            let v = span.distance_mm / duration_s;
            [(duration_s, v, v), (0.0, 0.0, 0.0), (0.0, 0.0, 0.0)]
        }
    };
    let mut phase_start = span.start.as_ms() as f64;
    let mut distance_before = 0.0;
    for (seconds, v0, v1) in phases {
        if seconds <= 0.0 {
            continue;
        }
        let acceleration = (v1 - v0) / seconds;
        let position = |t: f64| v0 * t + acceleration * t * t / 2.0;
        let left = ((lo - phase_start) / 1000.0).clamp(0.0, seconds);
        let right = ((hi - phase_start) / 1000.0).clamp(0.0, seconds);
        if right > left {
            let mut cuts = vec![left, right];
            if acceleration.abs() > 1e-9 {
                for bin in 1..SPEED_BINS {
                    let t = (bin as f64 * 500.0 - v0) / acceleration;
                    if t > left && t < right {
                        cuts.push(t);
                    }
                }
            }
            let half = span.distance_mm / 2.0 - distance_before;
            if half > position(left) && half < position(right) {
                let speed = (v0 * v0 + 2.0 * acceleration * half).max(0.0).sqrt();
                let t = 2.0 * half / (v0 + speed);
                if t > left && t < right {
                    cuts.push(t);
                }
            }
            cuts.sort_by(f64::total_cmp);
            for pair in cuts.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                let midpoint = (a + b) / 2.0;
                let cell = match distance_before + position(midpoint) < span.distance_mm / 2.0 {
                    true => span.source,
                    false => span.destination,
                };
                let row = cells.entry(cell.0).or_default();
                let ms = (b - a) * 1000.0;
                row.distance_mm += position(b) - position(a);
                row.moving_ms += ms;
                row.observed_ms += ms;
                let bin = (((v0 + acceleration * midpoint) / 500.0).max(0.0) as usize).min(SPEED_BINS - 1);
                row.speed_ms[bin] += ms;
            }
        }
        phase_start += seconds * 1000.0;
        distance_before += position(seconds);
    }
}
