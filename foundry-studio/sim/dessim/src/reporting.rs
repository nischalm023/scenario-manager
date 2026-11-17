//! Read the managers a run actually constructed for diagnostics and teardown.
//! Reporting never starts factories or commits warehouse work. Its registry
//! handle belongs to the runner, not to workflow tasks or their children.

use std::fmt::Write as _;
use std::sync::Arc;

use crate::error::PlanError;
use crate::model::bot::{BotKind, BotService};
use crate::model::htm::registry::AvailableHtmRegistry;
use crate::model::inventory::InventoryService;
use crate::model::navigation::Navigation;
use crate::model::relay::{RelayService, RelayUtilizationSummary, RelayUtilizationTimeline};
use crate::model::reservation::diagnostics::report_horizon_ms;
use crate::model::station::StationSet;
use crate::model::{SimTime, StationArrivalDiagnostics};
use crate::state::{StateManager, States};

#[derive(Clone)]
pub struct RunReports {
    states: States,
}

impl RunReports {
    pub(crate) fn new(states: States) -> Self {
        Self { states }
    }

    pub fn existing<M: StateManager>(&self) -> Option<Arc<M>> {
        self.states.existing()
    }

    pub fn planned_horizon(&self) -> SimTime {
        let mut horizon = SimTime::ZERO;
        if let Some(bots) = self.existing::<BotService>() {
            bots.for_each_bot(|_, _, plan| {
                if let Some((frontier, _)) = plan.availability() {
                    horizon = horizon.max(frontier);
                }
            });
        }
        horizon
    }

    fn relay_report_horizon(&self) -> u64 {
        let Some(navigation) = self.existing::<Navigation>() else { return 0 };
        let mut cells = Vec::new();
        if let Some(stations) = self.existing::<StationSet>() {
            cells.extend(stations.presentation_cells());
        }
        if let Some(relays) = self.existing::<RelayService>() {
            cells.extend(relays.htm_access_cells());
        }
        report_horizon_ms(navigation.oracle.as_ref(), cells)
    }

    pub fn status_report(&self) -> Option<String> {
        let navigation = self.existing::<Navigation>()?;
        let horizon = self.relay_report_horizon();
        if horizon == 0 {
            return None;
        }
        let mut out = String::new();
        let _ = writeln!(out, "\n=== Resource Status Report (offline planned intervals) ===");
        let _ = writeln!(out, "report_horizon_s={:.1}", horizon as f64 / 1000.0);
        let _ = writeln!(
            out,
            "note: 'open' means the reservation end is not planned; duty/gap percentages use report_horizon_s as the diagnostic cutoff."
        );
        if let Some(stations) = self.existing::<StationSet>() {
            let _ = write!(out, "{}", stations.status_report(navigation.oracle.as_ref(), horizon));
        }
        if let Some(arrivals) = self.existing::<StationArrivalDiagnostics>() {
            let _ = write!(out, "{}", arrivals.status_report());
        }
        if let Some(relays) = self.existing::<RelayService>() {
            let _ = write!(out, "{}", relays.status_report(navigation.oracle.as_ref(), horizon));
        }
        Some(out)
    }

    pub fn relay_utilization_summary(&self) -> Option<RelayUtilizationSummary> {
        let navigation = self.existing::<Navigation>()?;
        let relays = self.existing::<RelayService>()?;
        let horizon = self.relay_report_horizon();
        (horizon != 0).then(|| relays.utilization_summary(navigation.oracle.as_ref(), horizon))
    }

    pub fn relay_slot_occupancy_timeline(&self, slice_width_ms: u64) -> Option<RelayUtilizationTimeline> {
        let relays = self.existing::<RelayService>()?;
        let horizon = self.relay_report_horizon();
        (slice_width_ms != 0 && horizon != 0).then(|| relays.slot_occupancy_timeline(horizon, slice_width_ms))
    }

    pub fn stall_census(&self) -> String {
        let mut out = String::new();
        if let Some(available) = self.existing::<AvailableHtmRegistry>() {
            for entry in available.peek_matching(|_| true) {
                let _ = writeln!(
                    out,
                    "[census]   idle htm={:?} cell={:?} available_at={}",
                    entry.robot(),
                    entry.cell(),
                    entry.available_at()
                );
            }
        }
        if let Some(relays) = self.existing::<RelayService>() {
            let (per_relay, dir_counts) = relays.debug_relay_states();
            let _ = writeln!(out, "[census]   --- relay slots ---");
            for (rid, zid, unfinalized, avail, dir) in &per_relay {
                let _ = writeln!(
                    out,
                    "[census]   relay={rid:?} zone={zid:?} slot_unfinalized={unfinalized} avail_at={avail} dir={dir:?}"
                );
            }
            for ((zid, dir), count) in &dir_counts {
                let _ = writeln!(out, "[census]   direction_count zone={zid:?} dir={dir:?} = {count}");
            }
            if let Some(navigation) = self.existing::<Navigation>() {
                let _ = writeln!(out, "[census]   --- relay HTM-access cells (oracle) ---");
                for (rid, _, _, _, _) in &per_relay {
                    if let Some(cell) = relays.htm_access_cell(*rid) {
                        let _ = writeln!(
                            out,
                            "[census]   relay={rid:?} htm_cell={cell:?} oracle={:?}",
                            navigation.oracle.cell_status(cell)
                        );
                    }
                }
            }
            let _ = writeln!(
                out,
                "[census]   retrieve_tentatives={} fillable_relays={}",
                relays.retrieve_tentative_count(),
                relays.retrieve_fillable_relays().len()
            );
        }
        if let Some(inventory) = self.existing::<InventoryService>() {
            let horizon = self.planned_horizon();
            let _ = writeln!(out, "[census]   horizon={horizon} totes: {}", inventory.debug_census(horizon));
        }
        out
    }

    pub fn check_quiescence_invariants(&self) -> Result<(), PlanError> {
        let total = self.existing::<BotService>().map_or(0, |bots| bots.count_kind(BotKind::Htm));
        let idle = self.existing::<AvailableHtmRegistry>().map_or(0, |available| available.len());
        if idle != total {
            return Err(PlanError::TaskGraphInvariant {
                context: "conservation",
                detail: format!(
                    "quiescence: {idle} idle HTMs but {total} total (leaked {})",
                    total.saturating_sub(idle)
                ),
            });
        }
        Ok(())
    }
}
