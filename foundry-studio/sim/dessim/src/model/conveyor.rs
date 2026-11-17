//! A station's fixed conveyance: the belt a carrier rides between the cell an
//! HTM sets it down on and the cell another HTM lifts it off.
//!
//! Relative to `model`, this is a physical resource in the same sense a rack
//! or a relay is -- a place a tote can BE, which is what
//! `ToteLocation::OnConveyor` names. It differs from both in that **no robot
//! serves it**: a tote boards when an HTM departs the station's `dropoff_cell`
//! and comes off at the `collection_cell` one transit later, with nothing scheduled
//! in between. That gap is the point of an EasyLoop -- it decouples the
//! operator's picking from the robots' trips, so a robot is not held for the
//! duration of a pick.
//!
//! Order is FIFO by readiness. The loop is a closed racetrack with a single
//! ingress (confirmed by Kalpesh, 2026-08-07) and the document's own queues
//! declare `discipline: no_reverse`, so a tote cannot overtake one ahead of
//! it.

use std::sync::{Arc, Mutex};

use desgraph::StationId;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::model::ids::ToteId;
use crate::model::{SimDuration, SimTime};
use crate::signals::Version;

/// One EasyLoop's conveyor.
///
/// The machine, not the station. `specs/easyloop/DESIGN.md` §4 is emphatic
/// that an EasyLoop is "an assembly of two stations and one shared conveyor.
/// Not itself a station, and never called one here", and §5.2 puts the belt's
/// two handoff cells in the SHARED column: both of a machine's stations feed
/// one belt. Keying by station would give each half its own belt and double
/// the machine's capacity by accident.
///
/// The number is an index over the document's machines, assigned in sorted
/// order of the declaring entity so it is stable for a given document. It is
/// not a UUID because it travels through desmap, which is the plain-data layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ConveyorId(pub u32);

/// Daniel's September 7 ruling: eight totes over the entire EasyLoop conveyor,
/// including its two presentation positions and totes awaiting collection.
pub const EASYLOOP_CAPACITY: usize = 8;

/// Why a tote could not board.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoardRefusal {
    /// This station has no belt at all. The caller asked for something the map
    /// does not describe, which is a defect, not a transient.
    NoConveyor,
    /// Every position is taken. Back-pressure: the HTM keeps its tote and the
    /// deposit is retried, which is how a full loop reaches the robots feeding
    /// it.
    Full,
}

/// One tote's whole journey through a machine, computed when it boards.
///
/// An offline planner has no advancing clock -- `planner/mod.rs:11` says
/// `sim_time` is "the global planning barrier, not a per-task completion
/// timestamp", and it stays 0 all run. So nothing here polls "is it ready
/// yet"; the journey is solved at boarding time and callers SCHEDULE against
/// `available_at`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Journey {
    tote_id: ToteId,
    /// The seat this tote is drawn off to. A seat belongs to a station; a belt
    /// belongs to the machine, so which of the machine's two seats takes a
    /// tote is a RESOURCE question (DESIGN.md §5.3: "which inventory seat is
    /// free"), answered by whichever frees earliest.
    seat: StationId,
    seated_from: SimTime,
    /// When an HTM can lift it off at the collection cell.
    available_at: SimTime,
}

/// Every station's belt, keyed by station. Clone is a cheap Arc bump.
#[derive(Clone, Default)]
pub struct ConveyorLoops {
    inner: Arc<Mutex<Inner>>,
    /// Bumped on every board and take, so the loop actors wake when the
    /// belt's population changes instead of polling it.
    version: Version,
}

#[derive(Default)]
struct Inner {
    /// Which machine's belt a station feeds. Both stations of an EasyLoop map
    /// to one entry; a station absent here has no belt, which is every station
    /// that presents to an operator directly.
    of_station: FxHashMap<StationId, ConveyorId>,
    /// The seats a machine can draw a tote off to: one per station it serves.
    seats: FxHashMap<ConveyorId, Vec<StationId>>,
    /// How long one full circuit takes, per machine.
    transit: FxHashMap<ConveyorId, SimDuration>,
    /// How long an operator works a presented tote.
    dwell: SimDuration,
    /// How many carriers each belt holds at once.
    capacity: FxHashMap<ConveyorId, usize>,
    /// When each seat next comes free.
    seat_free_at: FxHashMap<StationId, SimTime>,
    /// Journeys solved and not yet lifted off. A POOL, not a queue: position
    /// on the belt is not modelled and no order is maintained.
    ///
    /// The machine re-orders carriers itself (Kristina, 2026-08-31), so a
    /// model that fixed their sequence would be claiming something the
    /// equipment does not promise -- and claiming it in the one place it
    /// hurts, since a sequence is also a head-of-line constraint the real belt
    /// does not impose.
    riding: FxHashMap<ConveyorId, Vec<Journey>>,
}

impl ConveyorLoops {
    /// Build from what the document declares, plus the operator dwell.
    pub fn new(
        of_station: FxHashMap<StationId, ConveyorId>,
        transit: FxHashMap<ConveyorId, SimDuration>,
        dwell: SimDuration,
    ) -> Self {
        let capacity = transit.keys().map(|&c| (c, EASYLOOP_CAPACITY)).collect();
        let mut seats: FxHashMap<ConveyorId, Vec<StationId>> = FxHashMap::default();
        for (&station, &conveyor) in &of_station {
            seats.entry(conveyor).or_default().push(station);
        }
        // Fx order is arbitrary and seat choice reaches a schedule, so fix it.
        for list in seats.values_mut() {
            list.sort_by_key(|s| s.0);
        }
        Self {
            inner: Arc::new(Mutex::new(Inner {
                of_station,
                seats,
                transit,
                dwell,
                capacity,
                seat_free_at: FxHashMap::default(),
                riding: FxHashMap::default(),
            })),
            version: Version::new(),
        }
    }

    pub fn version(&self) -> &Version {
        &self.version
    }

    /// The belt this station feeds, if it has one.
    pub fn of_station(&self, station_id: StationId) -> Option<ConveyorId> {
        self.inner.lock().unwrap().of_station.get(&station_id).copied()
    }

    /// Does this station feed a belt at all?
    pub fn serves(&self, station_id: StationId) -> bool {
        self.inner.lock().unwrap().of_station.contains_key(&station_id)
    }

    /// One full circuit, or `None` for a machine with no belt.
    pub fn transit(&self, conveyor: ConveyorId) -> Option<SimDuration> {
        self.inner.lock().unwrap().transit.get(&conveyor).copied()
    }

    /// Whether one more carrier fits aboard right now.
    pub fn has_room(&self, conveyor: ConveyorId) -> bool {
        let inner = self.inner.lock().unwrap();
        let capacity = inner.capacity.get(&conveyor).copied().unwrap_or(EASYLOOP_CAPACITY);
        inner.riding.get(&conveyor).map_or(0, Vec::len) < capacity
    }

    /// Positions on this belt.
    pub fn capacity(&self, conveyor: ConveyorId) -> Option<usize> {
        self.inner.lock().unwrap().capacity.get(&conveyor).copied()
    }

    /// Journeys solved and not yet lifted off.
    pub fn riding_at(&self, conveyor: ConveyorId) -> usize {
        self.inner.lock().unwrap().riding.get(&conveyor).map_or(0, Vec::len)
    }

    /// Put a tote on the belt at `at` and solve its whole journey.
    ///
    /// The circuit is halved: out to the seat, back to the collection cell.
    /// `loop_transit_secs` being the FULL circuit rather than one leg is an
    /// assumption -- `specs/easyloop/DESIGN.md` §5.7 says the two readings
    /// differ by whether belt capacity binds at design load (~7.3 totes vs
    /// ~14.5 against 10) and that Kalpesh should be asked both numbers
    /// together. The circuit reading is the one that fits, and it is the one
    /// taken here.
    ///
    /// A tote whose seat is not free yet loiters on the belt until it is. The
    /// machine really recirculates it (Kalpesh: recirculation is "very
    /// minimalistic", a tote presents about once), which at this fidelity is
    /// the same delay by a different route.
    pub fn board(&self, conveyor: ConveyorId, tote_id: ToteId, at: SimTime) -> Result<Journeyed, BoardRefusal> {
        let mut inner = self.inner.lock().unwrap();
        let transit = *inner.transit.get(&conveyor).ok_or(BoardRefusal::NoConveyor)?;
        let capacity = inner.capacity.get(&conveyor).copied().unwrap_or(EASYLOOP_CAPACITY);
        if inner.riding.get(&conveyor).map_or(0, Vec::len) >= capacity {
            return Err(BoardRefusal::Full);
        }
        let leg = SimDuration::ms(transit.as_ms() / 2);
        let reaches_seat = at + leg;
        let dwell = inner.dwell;

        // Earliest-free seat among the machine's own.
        let seats = inner.seats.get(&conveyor).cloned().unwrap_or_default();
        let Some(seat) =
            seats.iter().copied().min_by_key(|s| inner.seat_free_at.get(s).copied().unwrap_or(SimTime::ms(0)).as_ms())
        else {
            return Err(BoardRefusal::NoConveyor);
        };
        let free_at = inner.seat_free_at.get(&seat).copied().unwrap_or(SimTime::ms(0));
        let seated_from = reaches_seat.max(free_at);
        let seated_until = seated_from + dwell;
        let available_at = seated_until + leg;

        inner.seat_free_at.insert(seat, seated_until);
        let journey = Journey { tote_id, seat, seated_from, available_at };
        inner.riding.entry(conveyor).or_default().push(journey);
        drop(inner);
        self.version.bump();
        Ok(Journeyed { seat, seated_from, available_at })
    }

    /// The soonest-collectable tote aboard this belt, and when it can be
    /// lifted, without removing it. No "now" gate: the caller schedules a robot
    /// to arrive at that time.
    ///
    /// Searched over the pool rather than read off a maintained head, so no
    /// carrier can be stuck behind another. Ties break on tote id, which keeps
    /// the choice deterministic now that insertion order says nothing.
    pub fn next_available(&self, conveyor: ConveyorId) -> Option<(ToteId, SimTime)> {
        let inner = self.inner.lock().unwrap();
        inner
            .riding
            .get(&conveyor)?
            .iter()
            .min_by_key(|j| (j.available_at.as_ms(), j.tote_id.0))
            .map(|j| (j.tote_id, j.available_at))
    }

    /// Lift a named tote off. Named rather than "the head" so a caller that
    /// scheduled for a specific tote cannot silently take a different one.
    pub fn take(&self, conveyor: ConveyorId, tote_id: ToteId) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let Some(riding) = inner.riding.get_mut(&conveyor) else { return false };
        let Some(at) = riding.iter().position(|j| j.tote_id == tote_id) else { return false };
        riding.remove(at);
        drop(inner);
        self.version.bump();
        true
    }

    /// Totes aboard every belt. A run is not done while this is non-zero.
    pub fn riding_count(&self) -> usize {
        self.inner.lock().unwrap().riding.values().map(Vec::len).sum()
    }

    /// Machines currently carrying at least one tote, sorted so dispatch does
    /// not depend on hash order.
    pub fn occupied(&self) -> Vec<ConveyorId> {
        let inner = self.inner.lock().unwrap();
        let mut out: Vec<ConveyorId> = inner.riding.iter().filter(|(_, v)| !v.is_empty()).map(|(k, _)| *k).collect();
        out.sort_by_key(|c| c.0);
        out
    }
}

/// What boarding settled: which seat, when it is worked, when it comes off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Journeyed {
    pub seat: StationId,
    pub seated_from: SimTime,
    pub available_at: SimTime,
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOOP: ConveyorId = ConveyorId(0);
    /// One EasyLoop, two stations, one seat each (DESIGN.md §5.2).
    const LEFT: StationId = StationId(1);
    const RIGHT: StationId = StationId(2);
    const CIRCUIT_MS: u64 = 45_000;
    const DWELL_MS: u64 = 9_000;

    fn loops() -> ConveyorLoops {
        let mut of_station = FxHashMap::default();
        of_station.insert(LEFT, LOOP);
        of_station.insert(RIGHT, LOOP);
        let mut transit = FxHashMap::default();
        transit.insert(LOOP, SimDuration::ms(CIRCUIT_MS));
        ConveyorLoops::new(of_station, transit, SimDuration::ms(DWELL_MS))
    }

    /// Out to the seat, worked, back to the collection cell -- the three steps of
    /// DESIGN.md §5.1, with the circuit halved into the two legs.
    #[test]
    fn a_journey_is_out_then_seated_then_back() {
        let l = loops();
        let j = l.board(LOOP, ToteId(7), SimTime::ms(0)).expect("boards");
        assert_eq!(j.seated_from, SimTime::ms(CIRCUIT_MS / 2), "half a circuit out");
        assert_eq!(j.available_at, SimTime::ms(CIRCUIT_MS / 2 + DWELL_MS + CIRCUIT_MS / 2), "out, worked, back");
    }

    /// The re-key's purpose: a machine's two stations share one belt.
    #[test]
    fn both_stations_of_a_machine_share_one_belt() {
        let l = loops();
        assert_eq!(l.of_station(LEFT), Some(LOOP));
        assert_eq!(l.of_station(RIGHT), Some(LOOP));
        l.board(LOOP, ToteId(1), SimTime::ms(0)).expect("boards");
        assert_eq!(l.riding_at(LOOP), 1);
        assert_eq!(l.occupied(), vec![LOOP]);
    }

    /// Two totes arriving together take DIFFERENT seats and are worked in
    /// parallel. This is where two stations are worth twice one; with a single
    /// seat the second would queue behind the first.
    #[test]
    fn two_totes_are_worked_in_parallel_at_the_two_seats() {
        let l = loops();
        let a = l.board(LOOP, ToteId(1), SimTime::ms(0)).expect("boards");
        let b = l.board(LOOP, ToteId(2), SimTime::ms(0)).expect("boards");
        assert_ne!(a.seat, b.seat, "the machine has two seats; use both");
        assert_eq!(a.seated_from, b.seated_from, "worked at the same time");
    }

    /// A third tote must wait for a seat, and its wait EMERGES from the seat
    /// timeline rather than being a constant.
    #[test]
    fn a_third_tote_waits_for_a_seat_to_free() {
        let l = loops();
        l.board(LOOP, ToteId(1), SimTime::ms(0)).expect("boards");
        l.board(LOOP, ToteId(2), SimTime::ms(0)).expect("boards");
        let third = l.board(LOOP, ToteId(3), SimTime::ms(0)).expect("boards");
        // Reaches the seats half a circuit in, but both are busy until a dwell
        // after that.
        assert_eq!(third.seated_from, SimTime::ms(CIRCUIT_MS / 2 + DWELL_MS));
    }

    #[test]
    fn a_station_with_no_belt_has_no_conveyor() {
        let l = loops();
        assert!(!l.serves(StationId(99)));
        assert_eq!(l.of_station(StationId(99)), None);
        assert_eq!(l.board(ConveyorId(9), ToteId(7), SimTime::ms(0)), Err(BoardRefusal::NoConveyor));
    }

    #[test]
    fn a_full_belt_refuses_the_next_tote_until_one_comes_off() {
        let l = loops();
        let cap = l.capacity(LOOP).expect("has a belt");
        assert_eq!(cap, EASYLOOP_CAPACITY);
        for i in 0..cap {
            l.board(LOOP, ToteId(i as u32), SimTime::ms(0)).expect("fits");
        }
        assert_eq!(l.riding_at(LOOP), cap);
        assert_eq!(
            l.board(LOOP, ToteId(999), SimTime::ms(0)),
            Err(BoardRefusal::Full),
            "an uncapped belt would absorb this and overstate throughput"
        );
        assert!(l.take(LOOP, ToteId(0)), "lift one off");
        assert!(l.board(LOOP, ToteId(999), SimTime::ms(0)).is_ok());
    }

    /// Nothing polls a clock: the belt states WHEN, and `next_available` hands
    /// that to a caller who schedules a robot to meet it. The planner's
    /// `sim_time` is a zero barrier all run, so a "is it ready now" gate would
    /// never open.
    #[test]
    fn next_available_states_a_time_and_does_not_consume() {
        let l = loops();
        let j = l.board(LOOP, ToteId(3), SimTime::ms(0)).expect("boards");
        assert_eq!(l.next_available(LOOP), Some((ToteId(3), j.available_at)));
        assert_eq!(l.next_available(LOOP), Some((ToteId(3), j.available_at)), "idempotent");
        assert_eq!(l.riding_at(LOOP), 1);
        assert!(l.take(LOOP, ToteId(3)));
        assert_eq!(l.next_available(LOOP), None);
    }

    /// `take` names its tote, so a caller that scheduled for one cannot
    /// silently be handed another.
    #[test]
    fn take_will_not_hand_over_a_different_tote() {
        let l = loops();
        l.board(LOOP, ToteId(1), SimTime::ms(0)).expect("boards");
        assert!(!l.take(LOOP, ToteId(42)), "not aboard");
        assert_eq!(l.riding_at(LOOP), 1);
    }

    #[test]
    fn totes_come_off_in_the_order_their_journeys_end() {
        let l = loops();
        l.board(LOOP, ToteId(1), SimTime::ms(0)).expect("boards");
        l.board(LOOP, ToteId(2), SimTime::ms(0)).expect("boards");
        l.board(LOOP, ToteId(3), SimTime::ms(0)).expect("boards");
        // 3 waited for a seat, so it comes off last.
        assert_eq!(l.next_available(LOOP).map(|(t, _)| t), Some(ToteId(1)));
        l.take(LOOP, ToteId(1));
        l.take(LOOP, ToteId(2));
        assert_eq!(l.next_available(LOOP).map(|(t, _)| t), Some(ToteId(3)));
    }

    #[test]
    fn occupied_names_only_belts_carrying_something() {
        let l = loops();
        assert!(l.occupied().is_empty());
        l.board(LOOP, ToteId(1), SimTime::ms(0)).expect("boards");
        assert_eq!(l.occupied(), vec![LOOP]);
    }
}
