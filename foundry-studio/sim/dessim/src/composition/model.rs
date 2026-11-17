//! Publish immutable construction inputs and normalize declared fleet settings.
//! This module creates no managers and commits no simulation work.

use std::sync::Arc;

use desgraph::ZoneId;

use crate::config::SimConfig;
use crate::error::PlanError;
use crate::events::EventSink;
use crate::model::SimTime;
use crate::model::factories::WorldInputs;
use crate::pathfinding::Pathfinder;
use crate::scenario_gen::ScenarioData;
use crate::signals::Mut;
use crate::state::States;

/// An embargo requires a present: a pick embargo instant is consumed as a
/// barrier target, so configuring one with the barrier disabled is refused
/// before any planning, never silently floored.
pub fn validate_admission_config(config: &SimConfig) -> Result<(), PlanError> {
    if config.pick.embargo_secs > 0.0 && !config.barrier.enabled {
        return Err(PlanError::TaskGraphInvariant {
            context: "composition",
            detail: format!(
                "pick.embargo_secs={} with barrier.enabled=false: an embargo requires a present — enable the barrier or zero the embargo",
                config.pick.embargo_secs
            ),
        });
    }
    Ok(())
}

pub(super) fn configure_world(
    states: &States,
    data: &ScenarioData,
    config: &SimConfig,
    pathfinder: Arc<dyn Pathfinder>,
    event_sink: Option<Arc<dyn EventSink>>,
    clock: Mut<SimTime>,
) -> Result<(), PlanError> {
    states.configure(WorldInputs::new(Arc::new(data.clone()), config.clone(), pathfinder, event_sink, clock))?;
    Ok(())
}

pub(super) fn set_vtms_per_zone(data: &mut ScenarioData, n: usize) {
    use std::collections::{BTreeMap, BTreeSet};
    assert!(n >= 1, "vtm.per_zone must be >= 1 (0 registers zero VTMs and deadlocks the run)");
    let before = data.vtm_zone_assignments.len();
    let mut by_zone: BTreeMap<ZoneId, Vec<_>> = BTreeMap::new();
    for (zset, &pos) in data.vtm_zone_assignments.iter().zip(&data.vtm_initial_positions) {
        let zid = *zset.iter().next().expect("vtm assignment has empty zone set");
        by_zone.entry(zid).or_default().push(pos);
    }
    let mut new_assignments = Vec::with_capacity(by_zone.len() * n);
    let mut new_positions = Vec::with_capacity(by_zone.len() * n);
    for (&zid, positions) in &by_zone {
        for k in 0..n {
            let mut set = BTreeSet::new();
            set.insert(zid);
            new_assignments.push(set);
            new_positions.push(positions[k % positions.len()]);
        }
    }
    data.vtm_zone_assignments = new_assignments;
    data.vtm_initial_positions = new_positions;
    eprintln!(
        "[vtm] per_zone={n} — fleet {} → {} VTMs across {} zones",
        before,
        data.vtm_zone_assignments.len(),
        by_zone.len()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embargo_without_a_barrier_is_refused() {
        let mut config = SimConfig::default();
        config.pick.embargo_secs = 7200.0;
        config.barrier.enabled = false;
        let err = validate_admission_config(&config).expect_err("an embargo requires a present");
        assert!(matches!(err, PlanError::TaskGraphInvariant { context: "composition", .. }));
    }

    #[test]
    fn embargo_with_barrier_and_barrierless_without_embargo_both_pass() {
        let mut config = SimConfig::default();
        config.pick.embargo_secs = 7200.0;
        validate_admission_config(&config).expect("embargo + barrier is the production shape");
        config.pick.embargo_secs = 0.0;
        config.barrier.enabled = false;
        validate_admission_config(&config).expect("no embargo needs no present");
    }
}
