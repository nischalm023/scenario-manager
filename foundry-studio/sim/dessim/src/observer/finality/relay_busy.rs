//! Within `finality`, `relay_busy` reads a slice's relay busy time, how long
//! the relays' access cells were held across it, all relays together and
//! each relay zone's, from the holds navigation emits (`SimEvent::CellHeld`,
//! `SimEvent::CellReleased`), as the slice is made final, and notes the holds
//! that contradict each other.

use std::collections::VecDeque;

use desgraph::{CellId, ZoneId};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::events::{CellHeldEvent, CellReleasedEvent};
use crate::model::{SimDuration, SimTime};
use crate::observer::StreamViolation;
use crate::observer::time_slice::{MetricsConfig, passed_slice_end};

/// The holds on the relays' access cells that bear on the slices not yet
/// final.
pub(super) struct RelayBusy {
    width: SimDuration,
    /// Each relay's access cell, with the zone of its relay where the map
    /// puts the relay in one.
    access_cells: FxHashMap<CellId, Option<ZoneId>>,
    /// Every relay zone, each stated in every slice, an idle one as zero.
    zones: Vec<ZoneId>,
    /// The cells whose holds bound the horizon: the access cells and the
    /// stations' presentation cells.
    horizon_cells: FxHashSet<CellId>,
    /// When each access cell's hold began, while no path has ended it.
    open: FxHashMap<CellId, SimTime>,
    /// The time ended holds spent in each slice from `first` on.
    ended: VecDeque<SliceBusy>,
    /// The first slice not yet final.
    first: usize,
    /// The latest instant a hold on a horizon cell begins or ends: where the
    /// run's planned holds stop.
    horizon: SimTime,
    /// The first hold or release that contradicted the holds read before it,
    /// noted as it is read, for the observer to stop the run on.
    contradiction: Option<StreamViolation>,
}

/// How long the relays' access cells were held across one slice: all relays
/// together, and each relay zone's.
#[derive(Default)]
pub(super) struct SliceBusy {
    pub(super) all: SimDuration,
    pub(super) per_zone: FxHashMap<ZoneId, SimDuration>,
}

impl SliceBusy {
    fn add(&mut self, zone: Option<ZoneId>, held: SimDuration) {
        self.all += held;
        if let Some(zone) = zone {
            *self.per_zone.entry(zone).or_default() += held;
        }
    }
}

impl RelayBusy {
    pub(super) fn new(config: &MetricsConfig) -> Self {
        let access_cells: FxHashMap<CellId, Option<ZoneId>> = config
            .relay_htm_access_cells
            .values()
            .map(|&cell| (CellId(cell), config.relay_cell_zone.get(&cell).copied().map(ZoneId)))
            .collect();
        let mut zones: Vec<ZoneId> = access_cells.values().flatten().copied().collect();
        zones.sort_unstable();
        zones.dedup();
        let horizon_cells = access_cells.keys().copied().chain(config.station_cells.values().copied()).collect();
        Self {
            width: config.slice_width(),
            access_cells,
            zones,
            horizon_cells,
            open: FxHashMap::default(),
            ended: VecDeque::new(),
            first: 0,
            horizon: SimTime::ZERO,
            contradiction: None,
        }
    }

    /// Navigation held `hold.cell` for a robot. A hold that ends before it
    /// begins contradicts itself, and since no cell is held twice at once, an
    /// open hold of a cell an open hold still holds contradicts that hold.
    /// Either is noted before anything here changes.
    pub(super) fn held(&mut self, hold: &CellHeldEvent) {
        let (robot, cell, from) = (hold.robot_id, hold.cell, hold.from);
        if let Some(until) = hold.until.filter(|&until| until < from) {
            return self.note(StreamViolation::HoldEndsBeforeItBegins { robot, cell, from, until });
        }
        let zone = self.access_cells.get(&cell).copied();
        if let (Some(_), None, Some(&open_from)) = (zone, hold.until, self.open.get(&cell)) {
            return self.note(StreamViolation::CellHeldTwice { robot, cell, from, open_from });
        }
        if self.horizon_cells.contains(&cell) {
            self.horizon = self.horizon.max(from).max(hold.until.unwrap_or(from));
        }
        let Some(zone) = zone else {
            return;
        };
        match hold.until {
            Some(until) => self.end(zone, from, until),
            None => {
                self.open.insert(cell, from);
            }
        }
    }

    /// A robot's open hold of `release.cell` ended. A release of a cell no
    /// open hold holds, or one before the open hold began, contradicts the
    /// holds before it, and is noted before anything here changes.
    pub(super) fn released(&mut self, release: &CellReleasedEvent) {
        let (robot, cell, at) = (release.robot_id, release.cell, release.at);
        let ended = match self.access_cells.get(&cell) {
            None => None,
            Some(&zone) => match self.open.get(&cell) {
                None => return self.note(StreamViolation::ReleaseWithoutHold { robot, cell, at }),
                Some(&from) if at < from => {
                    return self.note(StreamViolation::ReleaseBeforeHoldBegan { robot, cell, from, at });
                }
                Some(&from) => Some((zone, from)),
            },
        };
        if self.horizon_cells.contains(&cell) {
            self.horizon = self.horizon.max(at);
        }
        if let Some((zone, from)) = ended {
            self.open.remove(&cell);
            self.end(zone, from, at);
        }
    }

    /// Keep the first contradiction read, for the observer to stop the run on.
    fn note(&mut self, violation: StreamViolation) {
        self.contradiction.get_or_insert(violation);
    }

    pub(super) fn take_contradiction(&mut self) -> Option<StreamViolation> {
        self.contradiction.take()
    }

    /// Book a hold over `[from, until)` of a relay in `zone` into the slices
    /// not yet final. What it spent in a final slice was read there as an open
    /// hold: B passes a hold's end only after the path that ends it is
    /// reported.
    fn end(&mut self, zone: Option<ZoneId>, from: SimTime, until: SimTime) {
        let mut index = self.first.max(self.slice_of(from));
        while self.slice_start(index) < until {
            let slice = self.slice_start(index);
            let held = until.min(slice + self.width) - from.max(slice);
            let offset = index - self.first;
            if self.ended.len() <= offset {
                self.ended.resize_with(offset + 1, SliceBusy::default);
            }
            self.ended[offset].add(zone, held);
            index += 1;
        }
    }

    /// The relay busy time of slice `index`, the first not yet final, all
    /// relays together and per zone, as it is made final with B at `barrier`:
    /// the time ended holds spent in it, and each open hold's from its start
    /// to the slice's end. B passes a hold's end only after the path that
    /// ends it is reported, so an open hold fills the rest of a slice B has
    /// passed; in a slice B never passed, it counts up to the horizon.
    pub(super) fn take_slice(&mut self, index: usize, barrier: SimTime) -> SliceBusy {
        assert_eq!(index, self.first, "slices are made final in order");
        let slice = self.slice_start(index);
        let end = slice + self.width;
        let until = if passed_slice_end(barrier, index, self.width) { end } else { end.min(self.horizon) };
        let mut busy = self.ended.pop_front().unwrap_or_default();
        for (cell, &from) in &self.open {
            busy.add(self.access_cells[cell], until - from.max(slice));
        }
        for &zone in &self.zones {
            busy.per_zone.entry(zone).or_default();
        }
        self.first += 1;
        busy
    }

    fn slice_of(&self, at: SimTime) -> usize {
        ((at - SimTime::ZERO) / self.width) as usize
    }

    fn slice_start(&self, index: usize) -> SimTime {
        SimTime::ZERO + self.width * index as u64
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, mpsc};

    use super::*;
    use crate::events::{OrderCompleteEvent, SimEvent};
    use crate::model::{OrderId, RobotId};
    use crate::observer::SimObserver;
    use crate::observer::finality::tests::Handed;
    use crate::sim_types::{StopReason, StopSignal};

    /// A hold's end reaches the horizon too: here the latest instant any hold
    /// begins or ends is cell 7's release, up to which cell 6's open hold
    /// counts in the slices B never passed.
    #[test]
    fn the_horizon_reaches_the_latest_release() {
        let mut config = MetricsConfig { slice_width_ms: 1_000, ..Default::default() };
        config.relay_htm_access_cells.extend([(0, 6), (1, 7)]);
        let mut busy = RelayBusy::new(&config);
        let robot_id = RobotId(1);
        busy.held(&CellHeldEvent { robot_id, cell: CellId(7), from: SimTime::ms(500), until: None });
        busy.held(&CellHeldEvent { robot_id, cell: CellId(6), from: SimTime::ms(600), until: None });
        busy.released(&CellReleasedEvent { robot_id, cell: CellId(7), at: SimTime::ms(2_500) });
        let barrier = SimTime::ms(1_001);
        let taken: Vec<u64> = (0..3).map(|index| busy.take_slice(index, barrier).all.as_ms()).collect();
        assert_eq!(taken, [500 + 400, 1_000 + 1_000, 500 + 500]);
    }

    /// A zone's figure is its own relays' holds, ended and open; a zone whose
    /// relays no hold held states zero, and the hold of a relay the map puts
    /// in no zone counts toward all relays alone.
    #[test]
    fn each_zone_carries_its_own_relays_holds_and_an_idle_zone_states_zero() {
        let mut config = MetricsConfig { slice_width_ms: 1_000, ..Default::default() };
        config.relay_htm_access_cells.extend([(0, 6), (1, 7), (2, 8), (3, 9)]);
        config.relay_cell_zone = Arc::new([(6, 10), (7, 10), (8, 11)].into_iter().collect());
        let mut busy = RelayBusy::new(&config);
        let (robot_id, ms) = (RobotId(1), SimTime::ms);
        busy.held(&CellHeldEvent { robot_id, cell: CellId(6), from: ms(100), until: Some(ms(400)) });
        busy.held(&CellHeldEvent { robot_id, cell: CellId(7), from: ms(500), until: None });
        busy.held(&CellHeldEvent { robot_id, cell: CellId(9), from: ms(0), until: Some(ms(200)) });
        let slice = busy.take_slice(0, ms(1_001));
        assert_eq!(slice.all.as_ms(), 300 + 500 + 200);
        let zone_10 = SimDuration::ms(300 + 500);
        assert_eq!(slice.per_zone, FxHashMap::from_iter([(ZoneId(10), zone_10), (ZoneId(11), SimDuration::ZERO)]));
    }

    /// A hold or a release that contradicts the holds is noted and changes
    /// nothing: the horizon stays where it was, and the open hold an early
    /// release names stays open.
    #[test]
    fn a_contradicting_hold_or_release_changes_nothing() {
        let mut config = MetricsConfig { slice_width_ms: 1_000, ..Default::default() };
        config.relay_htm_access_cells.insert(0, 7);
        let mut busy = RelayBusy::new(&config);
        let (robot_id, cell, ms) = (RobotId(1), CellId(7), SimTime::ms);
        busy.held(&CellHeldEvent { robot_id, cell, from: ms(800), until: None });
        busy.held(&CellHeldEvent { robot_id, cell, from: ms(2_500), until: Some(ms(2_000)) });
        assert!(busy.take_contradiction().is_some(), "the hold that ends before it begins");
        busy.released(&CellReleasedEvent { robot_id, cell, at: ms(600) });
        assert!(busy.take_contradiction().is_some(), "the release before its hold began");
        assert_eq!(busy.horizon, ms(800), "the horizon reaches the open hold's start alone");
        assert_eq!(busy.take_slice(0, ms(1_001)).all.as_ms(), 200, "the open hold still fills slice 0 from 0.8 s");
    }

    /// Read `events` with cell 7 a relay's access cell and the run's stop
    /// signal attached: what the observer returned, why the run was told to
    /// stop, and how many slices the recording took. Each stream makes slice
    /// 0 at 0.5 s and ends with B passing its end, which would make it final.
    fn observe(events: &[SimEvent]) -> (Result<(), StreamViolation>, Option<StopReason>, usize) {
        let mut config = MetricsConfig { slice_width_ms: 1_000, ..Default::default() };
        config.relay_htm_access_cells.insert(0, 7);
        let (tx, rx) = mpsc::channel();
        tx.send(SimEvent::OrderComplete(OrderCompleteEvent { order_id: OrderId(1), sim_time_ms: 500 })).unwrap();
        for event in events {
            tx.send(event.clone()).unwrap();
        }
        tx.send(SimEvent::BarrierAdvanced(SimTime::ms(1_001))).unwrap();
        drop(tx);
        let recording = Arc::new(Mutex::new(Handed::default()));
        let stop = StopSignal::new();
        let observer = SimObserver::new(config, rx).with_recorder(recording.clone());
        let observed = observer.with_failure_stop(stop.clone()).run().map(|_| ());
        let taken = recording.lock().unwrap().0.len();
        (observed, stop.take_reason(), taken)
    }

    fn open_hold(robot: u32, from: u64) -> SimEvent {
        let (robot_id, from) = (RobotId(robot), SimTime::ms(from));
        SimEvent::CellHeld(CellHeldEvent { robot_id, cell: CellId(7), from, until: None })
    }

    fn release(robot: u32, at: u64) -> SimEvent {
        SimEvent::CellReleased(CellReleasedEvent { robot_id: RobotId(robot), cell: CellId(7), at: SimTime::ms(at) })
    }

    /// A hold of a cell an open hold still holds stops the run as it is read,
    /// before B's next advance makes a slice final from the holds.
    #[test]
    fn a_cell_held_twice_at_once_stops_the_run_where_it_is_read() {
        let (observed, stopped, sent) = observe(&[open_hold(1, 500), open_hold(2, 800)]);
        let (robot, cell, from, open_from) = (RobotId(2), CellId(7), SimTime::ms(800), SimTime::ms(500));
        let violation = StreamViolation::CellHeldTwice { robot, cell, from, open_from };
        assert_eq!(observed, Err(violation.clone()));
        assert!(
            matches!(&stopped, Some(StopReason::Error(message)) if *message == violation.to_string()),
            "the scheduler is told to stop: {stopped:?}"
        );
        assert_eq!(sent, 0, "slice 0 never goes out");
    }

    /// A release of a cell no open hold holds stops the run as it is read,
    /// before B's next advance makes a slice final from the holds.
    #[test]
    fn a_release_of_a_cell_no_open_hold_holds_stops_the_run_where_it_is_read() {
        let (observed, stopped, sent) = observe(&[release(1, 600)]);
        let violation =
            StreamViolation::ReleaseWithoutHold { robot: RobotId(1), cell: CellId(7), at: SimTime::ms(600) };
        assert_eq!(observed, Err(violation.clone()));
        assert!(
            matches!(&stopped, Some(StopReason::Error(message)) if *message == violation.to_string()),
            "the scheduler is told to stop: {stopped:?}"
        );
        assert_eq!(sent, 0, "slice 0 never goes out");
    }

    /// A hold that ends before it begins stops the run as it is read, before
    /// B's next advance makes a slice final from the holds.
    #[test]
    fn a_hold_that_ends_before_it_begins_stops_the_run_where_it_is_read() {
        let (robot_id, cell, from, until) = (RobotId(1), CellId(7), SimTime::ms(800), SimTime::ms(500));
        let (observed, stopped, sent) =
            observe(&[SimEvent::CellHeld(CellHeldEvent { robot_id, cell, from, until: Some(until) })]);
        let violation = StreamViolation::HoldEndsBeforeItBegins { robot: robot_id, cell, from, until };
        assert_eq!(observed, Err(violation.clone()));
        assert!(
            matches!(&stopped, Some(StopReason::Error(message)) if *message == violation.to_string()),
            "the scheduler is told to stop: {stopped:?}"
        );
        assert_eq!(sent, 0, "slice 0 never goes out");
    }

    /// A release before the open hold it ends began stops the run as it is
    /// read, before B's next advance makes a slice final from the holds.
    #[test]
    fn a_release_before_its_hold_began_stops_the_run_where_it_is_read() {
        let (observed, stopped, sent) = observe(&[open_hold(1, 800), release(1, 600)]);
        let (robot, cell, from, at) = (RobotId(1), CellId(7), SimTime::ms(800), SimTime::ms(600));
        let violation = StreamViolation::ReleaseBeforeHoldBegan { robot, cell, from, at };
        assert_eq!(observed, Err(violation.clone()));
        assert!(
            matches!(&stopped, Some(StopReason::Error(message)) if *message == violation.to_string()),
            "the scheduler is told to stop: {stopped:?}"
        );
        assert_eq!(sent, 0, "slice 0 never goes out");
    }

    /// The run reached 1.001 s, where B last stood: slice 0 goes out as B passes it, and the stream's close sends
    /// slice 1, which the run's stop ends 1 ms in.
    #[test]
    fn holds_that_agree_stop_nothing() {
        let (observed, stopped, sent) = observe(&[open_hold(1, 500), release(1, 600), open_hold(2, 800)]);
        assert_eq!(observed, Ok(()));
        assert!(stopped.is_none(), "nothing stops the run: {stopped:?}");
        assert_eq!(sent, 2);
    }
}
