//! The order-tote close-out flow, measured rather than inferred.
//!
//! Every earlier claim that close-out ran came from arithmetic on the event
//! digest — total presentations minus picks minus putaways. That is a
//! plausible number, not a reading. This asserts the counters the flow's own
//! presentations increment.

use std::sync::{Arc, mpsc};
use std::time::Duration;

use dessim::events::EventSink;
use dessim::runner::RunnerHooks;
use dessim::{FHSippPathfinder, FidelityMode, SimDuration};
use native_modules as _;

use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

/// One load at the picking station and one extract at an extraction station,
/// per order that completes. Nothing else in the run produces either.
#[test]
#[cfg_attr(not(feature = "slow-gate"), ignore = "whole-fixture simulation; slow gate: scripts/test-slow-gate.sh")]
fn every_completed_order_is_loaded_and_extracted() {
    let loaded = dessim::world_spec::load(&fixture("walmart_dfw6_smoke_spec.json")).expect("spec loads");
    let mut config = loaded.settings;
    config.fidelity = FidelityMode::Ghost;
    // Long enough for five orders to finish picking and make the round trip;
    // the flow is what is under test, not the clock.
    config.sim_duration = Duration::from_secs(1800);

    let mut data = dessim::runner::resolve_scenario_with_racking(
        loaded.world.map.clone(),
        loaded.world.racking.clone(),
        &loaded.scenario,
        &config,
    )
    .expect("scenario generation");
    let pathfinder = Arc::new(FHSippPathfinder::new(
        &data.map,
        (config.vtm.speed * 1000.0) as u64,
        SimDuration::seconds(1),
        Default::default(),
    ));
    let (event_tx, event_rx) = mpsc::channel();
    let sink: Arc<dyn EventSink> = Arc::new(event_tx.clone());
    let (planner, model, bias) =
        dessim::composition::build(&mut data, &config, pathfinder, Some(sink)).expect("planner assembly");
    let (result, metrics) = dessim::runner::run(
        planner,
        model,
        &data,
        config,
        event_tx,
        event_rx,
        bias,
        RunnerHooks { live: None, subscribers: vec![], recording: None },
    )
    .expect("the run completes");

    let scalars = &metrics.cumulative_scalars;
    let orders_done = scalars.orders_completed;

    assert!(orders_done > 0, "no order completed, so close-out has nothing to prove");
    assert_eq!(
        scalars.order_load_presentations, orders_done,
        "one order-tote load per completed order (loads={}, orders={})",
        scalars.order_load_presentations, orders_done
    );
    assert_eq!(
        scalars.order_extract_presentations, orders_done,
        "one extraction per completed order (extracts={}, orders={})",
        scalars.order_extract_presentations, orders_done
    );

    // The arithmetic the earlier claim rested on, now a consequence rather
    // than the evidence.
    assert_eq!(
        scalars.presentations,
        scalars.pick_presentations
            + scalars.putaway_presentations
            + scalars.order_load_presentations
            + scalars.order_extract_presentations,
        "every presentation is accounted for by exactly one kind"
    );
    assert!(result.total_picks > 0, "the run must also pick");
}

/// The order buffer, with orders actually routed through it.
///
/// The smoke spec's generated orders state no exit path, so they all take the
/// straight-to-extraction route the test above pins. Here every order is
/// declared `ExitPath::Buffer`, which is the DFW6 majority case (14,626 of
/// 22,160 in the real Walmart extract), and the run has to make the detour:
/// pick station → buffer relay → buffer rack → dwell → buffer relay →
/// extraction.
#[test]
#[cfg_attr(not(feature = "slow-gate"), ignore = "whole-fixture simulation; slow gate: scripts/test-slow-gate.sh")]
fn a_buffer_bound_order_rests_in_the_buffer_and_still_reaches_extraction() {
    use std::collections::{BTreeMap, BTreeSet};

    use desgraph::{CellId, ZoneId};
    use desmap::ZoneHolds;
    use dessim::events::{SimEvent, VtmCycleDirection};
    use dessim::model::{ExitPath, RackId, ToteDesignation, ToteId, ToteLocation};

    let loaded = dessim::world_spec::load(&fixture("walmart_dfw6_smoke_spec.json")).expect("spec loads");
    let mut config = loaded.settings;
    config.fidelity = FidelityMode::Ghost;
    config.sim_duration = Duration::from_secs(1800);

    let mut data = dessim::runner::resolve_scenario_with_racking(
        loaded.world.map.clone(),
        loaded.world.racking.clone(),
        &loaded.scenario,
        &config,
    )
    .expect("scenario generation");

    // Which racks are the buffer's, before anything moves.
    let buffer_zones: BTreeSet<ZoneId> =
        data.map.zones.iter().filter(|z| z.holds == ZoneHolds::Order).map(|z| z.id).collect();
    assert!(!buffer_zones.is_empty(), "DFW6 states order-buffer racking");
    let cell_zone: BTreeMap<CellId, ZoneId> =
        data.zone_storage.iter().flat_map(|(&zid, cells)| cells.iter().map(move |&cid| (cid, zid))).collect();
    let buffer_racks: BTreeSet<RackId> = data
        .rack_cells
        .iter()
        .filter(|(_, cell)| cell_zone.get(cell).is_some_and(|z| buffer_zones.contains(z)))
        .map(|(&r, _)| r)
        .collect();
    assert!(!buffer_racks.is_empty(), "the buffer zones carry racking");

    // The buffer starts EMPTY. A slot holding a seeded tote is capacity a
    // closed order cannot use, and the buffer had none of it before this.
    assert!(
        !data.tote_placements.iter().any(|tp| matches!(tp.location, ToteLocation::InRack { rack_id, .. }
                               if buffer_racks.contains(&rack_id))),
        "no tote is seeded into buffer racking"
    );

    // Every order leaves via the buffer.
    for order in &mut data.orders {
        order.exit_path = ExitPath::Buffer;
    }
    let order_totes: BTreeSet<_> = data
        .tote_placements
        .iter()
        .filter(|tp| tp.designation == ToteDesignation::Order)
        .map(|tp| tp.tote_id)
        .collect();
    assert!(!order_totes.is_empty(), "the document declares an order-tote pool");

    let pathfinder = Arc::new(FHSippPathfinder::new(
        &data.map,
        (config.vtm.speed * 1000.0) as u64,
        SimDuration::seconds(1),
        Default::default(),
    ));
    let (event_tx, event_rx) = mpsc::channel();
    let sink: Arc<dyn EventSink> = Arc::new(event_tx.clone());
    let (planner, model, bias) =
        dessim::composition::build(&mut data, &config, pathfinder, Some(sink)).expect("planner assembly");
    let (tap_tx, tap_rx) = mpsc::channel::<SimEvent>();
    let (result, metrics) = dessim::runner::run(
        planner,
        model,
        &data,
        config,
        event_tx,
        event_rx,
        bias,
        RunnerHooks { live: None, subscribers: vec![tap_tx], recording: None },
    )
    .expect("the run completes");

    let cycles: Vec<(VtmCycleDirection, RackId, ToteId)> = tap_rx
        .try_iter()
        .filter_map(|e| match e {
            SimEvent::VtmCycle(c) => Some((c.direction, c.rack_id, c.tote_id)),
            _ => None,
        })
        .collect();
    let in_buffer: Vec<_> = cycles.iter().filter(|(_, r, _)| buffer_racks.contains(r)).collect();
    let filed = in_buffer.iter().filter(|(d, _, _)| *d == VtmCycleDirection::Return).count();
    let fetched = in_buffer.iter().filter(|(d, _, _)| *d == VtmCycleDirection::Forward).count();

    println!("[buffer] filed={filed} fetched={fetched} buffer_cycles={}", in_buffer.len());
    let scalars = &metrics.cumulative_scalars;
    assert!(scalars.orders_completed > 0, "no order completed, so the buffer has nothing to hold");
    assert!(result.total_picks > 0, "the run must also pick");

    // The detour happened: a gripper VTM filed closed orders into the buffer.
    assert!(filed > 0, "no order tote was filed into the buffer (buffer cycles: {})", in_buffer.len());
    // And they came back out. Fetches trail files by whatever is still resting
    // when the clock stops; they never lead.
    assert!(fetched <= filed, "more totes fetched from the buffer ({fetched}) than were ever filed into it ({filed})");
    assert!(fetched > 0, "every buffered order is still in the buffer at the end of the run");

    // ONLY order totes are ever in the buffer. It is not overflow storage for
    // stock, which is what it was before its zones' racking was made exclusive.
    for (direction, rack, tote) in &in_buffer {
        assert!(
            order_totes.contains(tote),
            "an inventory tote {tote:?} was moved {direction:?} in buffer rack {rack:?}"
        );
    }

    // The buffer is a detour, not a sink: an order that goes through it is
    // still loaded once and extracted once.
    assert_eq!(
        scalars.order_load_presentations, scalars.orders_completed,
        "one order-tote load per completed order (loads={}, orders={})",
        scalars.order_load_presentations, scalars.orders_completed
    );
    assert_eq!(
        scalars.order_extract_presentations as usize, fetched,
        "one extraction per tote fetched back out of the buffer (extracts={}, fetched={fetched})",
        scalars.order_extract_presentations
    );
}

/// Pick-only: an order ends at the Easy Loop and nothing leaves the building.
///
/// Asked for by Vinay (2026-08-31) so traffic and congestion can be measured
/// with and without order-bot movement. The contract is subtractive: picking
/// is untouched, and every leg that exists only to move an order TOTE is
/// absent. Anything less than zero-order-legs would leave order traffic in the
/// A/B and make the comparison meaningless.
#[test]
#[cfg_attr(not(feature = "slow-gate"), ignore = "whole-fixture simulation; slow gate: scripts/test-slow-gate.sh")]
fn pick_only_picks_and_moves_no_order_tote() {
    let loaded = dessim::world_spec::load(&fixture("walmart_dfw6_smoke_spec.json")).expect("spec loads");
    let mut config = loaded.settings;
    config.fidelity = FidelityMode::Ghost;
    config.sim_duration = Duration::from_secs(1800);
    config.order.close_out = false;

    let mut data = dessim::runner::resolve_scenario_with_racking(
        loaded.world.map.clone(),
        loaded.world.racking.clone(),
        &loaded.scenario,
        &config,
    )
    .expect("scenario generation");
    let pathfinder = Arc::new(FHSippPathfinder::new(
        &data.map,
        (config.vtm.speed * 1000.0) as u64,
        SimDuration::seconds(1),
        Default::default(),
    ));
    let (event_tx, event_rx) = mpsc::channel();
    let sink: Arc<dyn EventSink> = Arc::new(event_tx.clone());
    let (planner, model, bias) =
        dessim::composition::build(&mut data, &config, pathfinder, Some(sink)).expect("planner assembly");
    let (result, metrics) = dessim::runner::run(
        planner,
        model,
        &data,
        config,
        event_tx,
        event_rx,
        bias,
        RunnerHooks { live: None, subscribers: vec![], recording: None },
    )
    .expect("the run completes");

    let scalars = &metrics.cumulative_scalars;
    assert!(result.total_picks > 0, "pick-only must still pick");
    assert!(scalars.pick_presentations > 0, "pick-only must still present totes");
    assert_eq!(scalars.order_load_presentations, 0, "no order tote is loaded");
    assert_eq!(scalars.order_extract_presentations, 0, "nothing reaches extraction");
    // The run must still be able to finish. Retaining completions that
    // close-out would have drained is what deadlocked the done-check when
    // `closed_pending()` was made unconditional, so this is the regression
    // that guard exists for, reached by the second of its two routes.
    assert_eq!(
        scalars.presentations,
        scalars.pick_presentations + scalars.putaway_presentations,
        "pick-only accounts for every presentation with no order-tote kind"
    );
}
