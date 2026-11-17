//! Per-consumer limits for retrieval pipelines. The design estimate F_max
//! and service period size a CONWIP ceiling; queue_factor adds headroom.
//! Little's law relates observed mean WIP, cycle time and throughput. This
//! worst-distance sizing heuristic does not guarantee saturation or bound
//! completion time when traffic and resource queues add contention.

use crate::model::SimDuration;

/// The CONWIP ceiling as ingredients rather than a number, because every
/// station of a family shares the multiplier and the service period but has
/// its own `F_max`.
#[derive(Clone, Copy, Debug)]
pub struct AdmissionLaw {
    /// Dimensionless scaling of the derived pipeline (`queue_factor` on the
    /// pick side). Larger values provide more preparation headroom; actual
    /// WIP and starvation must be measured to tune this value.
    pub multiplier: f64,
    /// Service period `P` — how often the station consumes one presentation.
    pub period: SimDuration,
    /// Pins the ceiling regardless of geometry, for tests and experiments on
    /// maps whose derived depth is too shallow to exercise the behavior under
    /// study.
    pub override_cap: Option<u32>,
}

impl AdmissionLaw {
    /// `ceil(multiplier · F_max / P) + 2` — the derivation and the margin the
    /// pre-quarantine `OrderManager::set_spatial_data` shipped. The `+ 2`
    /// covers the tote presenting right now plus one chain's worth of quote
    /// error, and the inner `max(1)` keeps a station whose fetch bound rounds
    /// to nothing from being cut off from work entirely.
    pub fn cap(&self, f_max: SimDuration) -> u32 {
        if let Some(pinned) = self.override_cap {
            return pinned.max(1);
        }
        // A station with no service period has no rate to keep up with, so
        // Little's law says nothing; admit one chain and let it pace itself.
        if self.period.as_ms() == 0 {
            return 1;
        }
        let pipeline = self.multiplier * f_max.as_ms() as f64 / self.period.as_ms() as f64;
        pipeline.ceil().max(1.0) as u32 + 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn law(multiplier: f64, period_secs: u64) -> AdmissionLaw {
        AdmissionLaw { multiplier, period: SimDuration::seconds(period_secs), override_cap: None }
    }

    #[test]
    fn cap_is_the_pipeline_plus_two() {
        // 100s of worst-case fetch against a 7s pick: 15 chains cover the
        // pipeline, 2 more are the presenting tote and the quote margin.
        assert_eq!(law(1.0, 7).cap(SimDuration::seconds(100)), 17);
        // The multiplier scales the pipeline, not the margin.
        assert_eq!(law(4.0, 7).cap(SimDuration::seconds(100)), 60);
    }

    #[test]
    fn degenerate_inputs_still_admit_work() {
        assert_eq!(law(0.0, 7).cap(SimDuration::seconds(100)), 3, "a zero multiplier still admits the margin");
        assert_eq!(law(1.0, 7).cap(SimDuration::ZERO), 3, "a station at zero distance still admits the margin");
        assert_eq!(law(1.0, 0).cap(SimDuration::seconds(100)), 1, "no service period, no derivable pipeline");
    }

    #[test]
    fn override_pins_the_cap_against_geometry() {
        let pinned = AdmissionLaw { override_cap: Some(5), ..law(1.0, 7) };
        assert_eq!(pinned.cap(SimDuration::seconds(100)), 5);
        assert_eq!(AdmissionLaw { override_cap: Some(0), ..law(1.0, 7) }.cap(SimDuration::seconds(100)), 1);
    }
}
