//! A watched run follows the scheduler's barrier B: the observer hands the
//! run's recording a slice, and the stock the slice states, once B has passed
//! the slice, while the run goes on. Each slice the recording takes carries
//! its relay busy time, read from the holds navigation emits.
//!
//! A held run here holds one task that reports a fact, waits for B to pass
//! it, and then holds the scheduler until the watcher sees the result. The run
//! cannot end meanwhile, so what the watcher sees went out while the run was
//! still going, and its event stream had gone quiet. The relay busy time is
//! checked on a small pick-only run that nothing holds.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use dessim::composition;
use dessim::events::{EventJournal, EventSink, InventoryDeltaEvent, OrderCompleteEvent, SimEvent};
use dessim::model::ids::SkuId;
use dessim::observer::finality::recording::{FinalSlice, RecordingFailed, SliceRecorder, SliceRows};
use dessim::observer::time_slice::TimeSliceData;
use dessim::prelude::*;
use dessim::runner::{RunnerHooks, resolve_scenario_with_racking};
use dessim::scenario::ScenarioConfig;
use native_modules as _;

/// A recording that keeps each slice it takes, as the observer handed it.
#[derive(Default)]
struct Kept(Vec<TimeSliceData>);

impl SliceRecorder for Kept {
    fn record(&mut self, slice: FinalSlice<'_>) -> Result<(), RecordingFailed> {
        self.0.push(slice.slice.clone());
        Ok(())
    }

    fn keep_rows_left(&mut self, _: SliceRows) {}
}

/// Reports `facts`, waits for B to reach `barrier`, then holds the scheduler
/// until `seen` holds or a minute of wall time has passed, and records which.
#[derive(Display)]
#[display("ReportAndHold")]
struct ReportAndHold {
    journal: EventJournal,
    facts: Vec<SimEvent>,
    barrier: SimTime,
    seen: Box<dyn FnMut() -> bool>,
    saw: Arc<AtomicBool>,
}

#[async_trait(?Send)]
impl Task for ReportAndHold {
    type Proposal = EmptyProposal;
    type Schedule = ();

    async fn propose(&self, _owner: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _proposal: EmptyProposal, ctx: SchedCtx) -> Result<(), PlanError> {
        for fact in self.facts.drain(..) {
            self.journal.emit(fact);
        }
        ctx.barrier().wait_until(self.barrier).await?;
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut saw = (self.seen)();
        while !saw && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
            saw = (self.seen)();
        }
        self.saw.store(saw, Ordering::Relaxed);
        Ok(())
    }
}

/// Run one `ReportAndHold`, recorded into `recording`, and say whether the
/// task saw what it waited for before it let the run end.
fn watched_run(
    facts: Vec<SimEvent>,
    barrier: SimTime,
    seen: Box<dyn FnMut() -> bool>,
    recording: Arc<Mutex<Kept>>,
) -> bool {
    let saw = Arc::new(AtomicBool::new(false));
    let task = |journal, _| ReportAndHold { journal, facts, barrier, seen, saw: saw.clone() };
    common::run_one_task(task, RunnerHooks { recording: Some(recording), ..RunnerHooks::default() })
        .expect("the held run completes");
    saw.load(Ordering::Relaxed)
}

/// The recording takes a slice once B has passed it, and only then.
#[test]
fn the_recording_takes_a_slice_once_the_barrier_has_passed_it() {
    let recording = Arc::new(Mutex::new(Kept::default()));
    let seen = {
        let recording = recording.clone();
        Box::new(move || {
            let taken: Vec<u64> = recording.lock().unwrap().0.iter().map(|slice| slice.start_ms).collect();
            let passed = |start: &u64| start + 10_000 < 30_000;
            assert!(taken.iter().all(passed), "a slice B has not passed went out: {taken:?}");
            taken.contains(&10_000)
        })
    };
    let order = SimEvent::OrderComplete(OrderCompleteEvent { order_id: OrderId(1), sim_time_ms: 25_000 });
    assert!(
        watched_run(vec![order], SimTime::ms(30_000), seen, recording),
        "slice 1 never reached the recording while the run was going"
    );
}

/// The recording takes the slice that states a change of stock once B has
/// passed it, with the stock the change left.
#[test]
fn the_recording_takes_a_slices_stock_once_the_barrier_has_passed_it() {
    let recording = Arc::new(Mutex::new(Kept::default()));
    let seen = {
        let recording = recording.clone();
        Box::new(move || recording.lock().unwrap().0.iter().any(|slice| slice.scalars.inventory_units == 7))
    };
    let delta = SimEvent::InventoryDelta(InventoryDeltaEvent {
        delta: 7,
        sku: SkuId(1),
        tote_id: ToteId(3),
        at: SimTime::ms(5_000),
    });
    assert!(
        watched_run(vec![delta], SimTime::ms(20_000), seen, recording),
        "the recording never took the stock B had passed while the run was going"
    );
}

/// Each slice the recording takes carries its relay busy time, read from the
/// holds navigation emits on the relays' access cells: as B passed it, or,
/// for a slice B never passed, up to the latest planned hold when the stream
/// closed. The run stops at 45 s, mid-flow, so B never passes the slice from
/// 40 s, which holds reservations planned past the stop.
#[test]
fn each_slice_the_recording_takes_carries_its_relay_busy_time() {
    let (world, mut scenario, mut config) = common::load_world_spec("minimal_pick_only_spec.json");
    config.sim_duration = Duration::from_secs(45);
    let ScenarioConfig::Generated(generated) = &mut scenario else { panic!("generated fixture") };
    generated.num_skus = 1;
    generated.total_inventory_units = 20;
    generated.num_orders = 3;
    generated.total_order_picks = 3;
    let mut data = resolve_scenario_with_racking(world.map.clone(), world.racking.clone(), &scenario, &config)
        .expect("scenario gen");
    let pathfinder = common::fh_sipp_for_map(&data.map, &config);
    let (event_tx, event_rx) = mpsc::channel();
    let sink: Arc<dyn EventSink> = Arc::new(event_tx.clone());
    let (scheduler, reports, bias) =
        composition::build(&mut data, &config, pathfinder, Some(sink)).expect("the fixture's composition");
    let recording = Arc::new(Mutex::new(Kept::default()));
    let hooks = RunnerHooks { recording: Some(recording.clone()), ..RunnerHooks::default() };
    dessim::runner::run(scheduler, reports, &data, config, event_tx, event_rx, bias, hooks).expect("the run completes");

    let received: Vec<u64> =
        recording.lock().unwrap().0.iter().map(|slice| slice.scalars.relay_reserved_busy_ms).collect();
    assert!(received[0] > 0, "the first slice, which B passed, held a relay access cell");
    assert!(received[4] > 0, "the slice from 40 s, which B never passed, holds one");

    // Each zone's figure, slice by slice. Every relay of the fixture is in a
    // zone, so the zones add up to all relays together; and the fixture's
    // figures are pinned, so a hold booked twice over or a relay put in no
    // zone fails here.
    let per_zone: Vec<Vec<(u32, u64)>> = recording
        .lock()
        .unwrap()
        .0
        .iter()
        .map(|slice| {
            let mut zones: Vec<(u32, u64)> =
                slice.scalars.relay_reserved_busy_per_zone.iter().map(|(zone, busy)| (zone.0, busy.as_ms())).collect();
            zones.sort_unstable();
            zones
        })
        .collect();
    for (index, (zones, all)) in per_zone.iter().zip(&received).enumerate() {
        assert_eq!(zones.iter().map(|&(_, ms)| ms).sum::<u64>(), *all, "slice {index}: the zones add up to all relays");
    }
    assert_eq!(per_zone, [[(1, 10_000)], [(1, 6_001)], [(1, 0)], [(1, 6_343)], [(1, 1_561)]], "zone 1's busy time");
}
