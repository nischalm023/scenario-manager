//! An empty composition plus one injected task performs no warehouse work.
//! Even a run a caller watches, polling its progress as it goes, must observe
//! the selected tasks without constructing equipment managers, and observer
//! teardown must still close cleanly.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use dessim::composition::{self, Composition, Definition};
use dessim::events::{EventSink, SimEvent};
use dessim::model::{BotService, InventoryService, RelayService};
use dessim::prelude::*;
use dessim::reporting::RunReports;
use dessim::runner::{RunnerHooks, RunnerLiveHandles};
use dessim::scenario::ScenarioConfig;
use dessim::sim_types::Counters;

#[derive(Display)]
#[display("NoWork")]
struct NoWork {
    ran: Arc<AtomicBool>,
    reports: RunReports,
}

#[async_trait(?Send)]
impl Task for NoWork {
    type Proposal = EmptyProposal;
    type Schedule = ();

    async fn propose(&self, _owner: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _proposal: EmptyProposal, _ctx: SchedCtx) -> Result<(), PlanError> {
        tokio::task::yield_now().await;
        assert!(self.reports.existing::<BotService>().is_none());
        assert!(self.reports.existing::<InventoryService>().is_none());
        assert!(self.reports.existing::<RelayService>().is_none());
        self.ran.store(true, Ordering::Relaxed);
        Ok(())
    }
}

#[test]
fn injected_fake_composition_governs_the_run() {
    run_no_work(false);
    run_no_work(true);
}

fn run_no_work(watched: bool) {
    let (world, mut scenario, mut config) = common::load_world_spec("minimal_pick_only_spec.json");
    let ScenarioConfig::Generated(generated) = &mut scenario else { panic!("generated fixture") };
    generated.num_skus = 1;
    config.composition = Composition::Inline(Definition { tasks: vec![], task_defaults: Default::default() });
    let mut data =
        dessim::runner::resolve_scenario_with_racking(world.map.clone(), world.racking.clone(), &scenario, &config)
            .expect("scenario gen");
    let pathfinder = common::fh_sipp_for_map(&data.map, &config);

    let (event_tx, event_rx) = mpsc::channel();
    let sink: Arc<dyn EventSink> = Arc::new(event_tx.clone());
    let (mut scheduler, reports, bias) =
        composition::build(&mut data, &config, pathfinder, Some(sink)).expect("empty composition");
    assert!(reports.existing::<InventoryService>().is_none());

    let (sub_tx, sub_rx) = mpsc::channel::<SimEvent>();
    let capture = std::thread::spawn(move || sub_rx.iter().collect::<Vec<_>>());

    let ran = Arc::new(AtomicBool::new(false));
    scheduler.add_task(NoWork { ran: ran.clone(), reports: reports.clone() });
    let (result, _metrics) = dessim::runner::run(
        scheduler,
        reports,
        &data,
        config,
        event_tx,
        event_rx,
        bias,
        RunnerHooks {
            live: watched.then(|| RunnerLiveHandles { counters: Some(Counters::new()), stop_signal: None }),
            subscribers: vec![sub_tx],
            recording: None,
        },
    )
    .expect("solver should halt cleanly with a no-work composition");

    assert!(ran.load(Ordering::Relaxed), "the scheduler never ran the injected task");
    assert_eq!(result.total_picks, 0, "injected composition must fully replace the Cencora set");
    let events = capture.join().unwrap();
    assert!(events.is_empty(), "no work was composed, so no physical events may be emitted");
}
