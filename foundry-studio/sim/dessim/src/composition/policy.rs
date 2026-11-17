//! Pure calculations for task admission and timing, using task parameters and
//! the legacy scenario settings that still supply equipment properties/defaults.
//!
//! TODO(module): the tasks' own rules, frozen into core's API. Each task's own rule moves to the module that owns
//! it: `pick` and `pick_start` to `PickDemand`'s, `induction_period` to `PutawayInduction`'s, `direct_bias` to
//! `PutawaySupplyDemand`'s, and `echo_admission_caps` to the two that call it, `PickDemand` and
//! `PutawaySupplyDemand`. The duration validation, which `FulfillOrders` and the other tasks share, stays shared.
//! Core calls these too, to validate a composition's parameters and to compute a run's direct bias, so a module
//! must first be able to validate its own parameters.

use super::invalid;
use super::parameters::{InductionSequence, InductionTaskConfig, PickTaskConfig};
use crate::config::SimConfig;
use crate::metrics::fpi::fleet_pressure_index;
use crate::model::{StationFetchBounds, StationSet, station::Station};
use crate::prelude::*;
use crate::simtime::SimOffset;
use desmap::WarehouseMap;

#[derive(Clone, Copy)]
pub struct StationPolicy {
    pub admission: AdmissionLaw,
    pub pick_dwell: SimDuration,
    pub pallet_swap: SimDuration,
    pub aim_arrivals: bool,
}

pub fn pick(config: &SimConfig, params: &PickTaskConfig) -> Result<StationPolicy, PlanError> {
    let pick_dwell = duration("operator.time_per_pick_secs", config.operator.time_per_pick_secs)?;
    let pallet_swap = duration("operator.pallet_swap_secs", config.operator.pallet_swap_secs)?;
    let multiplier = params.queue_factor.unwrap_or(config.queue_factor);
    nonnegative("PickDemand.queue_factor", multiplier)?;
    let override_cap = params.max_inflight_picks.or(config.order.max_inflight_picks_override);
    if override_cap == Some(0) {
        return Err(invalid("PickDemand.max_inflight_picks must be positive"));
    }
    Ok(StationPolicy {
        admission: AdmissionLaw { multiplier, period: pick_dwell + pallet_swap, override_cap },
        pick_dwell,
        pallet_swap,
        aim_arrivals: params.pace_to_station.unwrap_or(config.pick.pace_to_station),
    })
}

pub fn pick_start(config: &SimConfig, params: &PickTaskConfig) -> Result<SimTime, PlanError> {
    let embargo = duration("PickDemand.embargo_secs", params.embargo_secs.unwrap_or(config.pick.embargo_secs))?;
    if embargo > SimDuration::ZERO && !config.barrier.enabled {
        return Err(invalid("PickDemand.embargo_secs requires barrier.enabled=true"));
    }
    Ok(SimTime::ms(embargo.as_ms()))
}

pub fn induction_period(config: &SimConfig, params: &InductionTaskConfig) -> Result<SimDuration, PlanError> {
    if let Some(secs) = params.lead_time_secs {
        duration("PutawayInduction.lead_time_secs", secs)?;
        if params.sequence != InductionSequence::OrderRelease {
            return Err(invalid("PutawayInduction.lead_time_secs requires sequence=order_release"));
        }
    }
    if params.units_per_tote == Some(0) {
        return Err(invalid("PutawayInduction.units_per_tote must be positive"));
    }
    let rate = params.max_presentations_per_hour.unwrap_or(config.putaway.max_presentations_per_hour);
    if !rate.is_finite() || rate <= 0.0 {
        return Err(invalid("PutawayInduction.max_presentations_per_hour must be finite and positive"));
    }
    let period = duration("PutawayInduction interval", 3600.0 / rate)?;
    if period == SimDuration::ZERO {
        return Err(invalid("PutawayInduction interval must be at least one millisecond"));
    }
    Ok(period)
}

pub fn duration(name: &str, secs: f64) -> Result<SimDuration, PlanError> {
    nonnegative(name, secs)?;
    if secs * 1000.0 >= u64::MAX as f64 {
        return Err(invalid(format!("{name} exceeds the supported simulation duration")));
    }
    Ok(SimDuration::ms((secs * 1000.0) as u64))
}

fn nonnegative(name: &str, value: f64) -> Result<(), PlanError> {
    if !value.is_finite() || value < 0.0 {
        return Err(invalid(format!("{name} must be finite and nonnegative")));
    }
    Ok(())
}

/// Log the derived ceilings next to the SimConfig echo, with the `F_max` and
/// `P` that produced them. The SimConfig echo cannot carry them — the caps
/// depend on the map, which is only resolved here — so without this a run's
/// transcript would not say what admission it actually ran under.
pub fn echo_admission_caps(
    stations: &StationSet,
    bounds: &StationFetchBounds,
    pick: Option<&AdmissionLaw>,
    putaway: Option<&AdmissionLaw>,
    ignored_queue_depth: Option<u32>,
) {
    eprintln!("=== Admission caps (derived: ceil(mult x F_max / P) + 2) ===");
    if let Some(pick) = pick {
        echo_cap_family("pick", pick, stations.pick_stations(), bounds);
    }
    let Some(putaway) = putaway else { return };
    let putaway_caps = echo_cap_family("putaway", putaway, stations.putaway_stations(), bounds);
    match (ignored_queue_depth, putaway_caps) {
        (Some(depth), Some((lo, hi))) => eprintln!(
            "[admission] WARNING putaway.queue_depth={depth} is IGNORED — the derived per-station caps {lo}..{hi} replace it"
        ),
        (Some(depth), None) => {
            eprintln!("[admission] WARNING putaway.queue_depth={depth} is IGNORED — this run has no putaway stations")
        }
        (None, _) => {}
    }
}

/// One line per station family; returns its min and max cap, or `None` when
/// the run has no station of that family.
fn echo_cap_family(
    family: &str,
    law: &AdmissionLaw,
    stations: &[Station],
    bounds: &StationFetchBounds,
) -> Option<(u32, u32)> {
    if stations.is_empty() {
        return None;
    }
    let caps: Vec<(SimDuration, u32)> = stations
        .iter()
        .map(|station| {
            let f_max = bounds.f_max(station.id);
            (f_max, law.cap(f_max))
        })
        .collect();
    let secs = |d: SimDuration| d.as_ms() as f64 / 1000.0;
    let cap_lo = caps.iter().map(|&(_, cap)| cap).min()?;
    let cap_hi = caps.iter().map(|&(_, cap)| cap).max()?;
    let cap_mean = caps.iter().map(|&(_, cap)| cap as f64).sum::<f64>() / caps.len() as f64;
    let pinned = match law.override_cap {
        Some(n) => format!("  [PINNED by override={n}, geometry ignored]"),
        None => String::new(),
    };
    eprintln!(
        "[admission] {family:<8} stations={:<3} mult={:.2}  P={:.2}s  F_max {:.1}s..{:.1}s  cap min/max/mean {}/{}/{:.1}{}",
        caps.len(),
        law.multiplier,
        secs(law.period),
        secs(caps.iter().map(|&(f_max, _)| f_max).min()?),
        secs(caps.iter().map(|&(f_max, _)| f_max).max()?),
        cap_lo,
        cap_hi,
        cap_mean,
        pinned,
    );
    Some((cap_lo, cap_hi))
}

pub fn direct_bias(has_putaway: bool, map: &WarehouseMap, secs: Option<f64>) -> Result<SimOffset, PlanError> {
    Ok(if !has_putaway {
        SimOffset::ZERO
    } else {
        match secs {
            // nan reads as neutral and inf wins every comparison, both silently — malformed config fails loud.
            Some(secs) if !secs.is_finite() => {
                return Err(PlanError::TaskGraphInvariant {
                    context: "composition",
                    detail: format!("putaway.direct_bias_secs must be finite, got {secs}"),
                });
            }
            Some(secs) => SimOffset::from_secs(secs),
            None => fleet_pressure_index(map).into(),
        }
    })
}
