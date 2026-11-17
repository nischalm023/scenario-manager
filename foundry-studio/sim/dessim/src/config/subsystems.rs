//! Within `config`, `subsystems` is the per-subsystem part of a run's
//! settings: relay, VTM scoring, orders, pick, the trailing barrier, station
//! operators and putaway, each a `#[serde(default)]` struct so a spec states
//! only the figures it moves off the default.

use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::OrderStrategy;
use super::robots::{InvalidFigure, above_zero};

/// Relay system parameters.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RelayConfig {
    /// VTM weight for relay selection scoring.
    pub vtm_weight: f64,
    /// HTM weight for relay selection scoring.
    pub htm_weight: f64,
    /// Zone load bucket size for relay load tracking.
    pub zone_load_bucket_size: usize,
    /// Discount factor for reserved-but-empty relay slots in pressure calculations.
    /// 0.0 = ignore reservations entirely, 1.0 = count reservations at full weight.
    /// Default 0.7 reflects that ~30% of reservations are transient ghost pressure.
    pub reservation_discount: f64,
    /// When true (default), an HTM pickup becomes eligible at the relay
    /// reservation's start (the VTM forward-cycle start) instead of
    /// `deposit_time - htm_to_relay`, finalizing the relay slot earlier in
    /// pass-order. Physical HTM timing is unchanged (JIT departure still
    /// targets deposit). Set false to restore the deposit-relative timing.
    pub htm_eligible_at_relay_start: bool,
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            vtm_weight: 0.6,
            htm_weight: 0.4,
            zone_load_bucket_size: 5,
            reservation_discount: 0.7,
            htm_eligible_at_relay_start: true,
        }
    }
}

/// VTM scoring parameters for unified proximity-weighted utility matching.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct VtmScoringConfig {
    /// Maximum travel distance for proximity normalization (mm).
    pub max_travel_dist: f64,
    /// Base return weight at zero pressure.
    pub return_base: f64,
    /// Return urgency multiplier for pressure-squared term.
    pub return_urgency: f64,
    /// Minimum free relay slots before forward work is blocked (RAW count).
    pub deadlock_headroom: u32,
    /// Discount for reserved-but-empty slots in soft scoring pressure.
    pub reservation_discount: f64,
    /// Time constant for starvation age bonus decay (seconds).
    pub starvation_decay_secs: f64,
    /// Maximum starvation bonus cap.
    pub starvation_max_bonus: f64,
    /// Contention penalty per VTM in-flight to the same zone.
    pub zone_contention_penalty: f64,
}

impl Default for VtmScoringConfig {
    fn default() -> Self {
        Self {
            max_travel_dist: 150_000.0,
            return_base: 0.3,
            return_urgency: 2.0,
            deadlock_headroom: 2,
            reservation_discount: 0.7,
            starvation_decay_secs: 60.0,
            starvation_max_bonus: 0.5,
            zone_contention_penalty: 0.15,
        }
    }
}

/// Order processing parameters.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct OrderConfig {
    /// Order processing strategy.
    pub strategy: OrderStrategy,
    /// Safety-net periodic dispatch interval in seconds.
    /// This is a fallback timer; primary dispatch is event-driven.
    #[serde(alias = "bin_management_interval_secs")]
    pub dispatch_safety_net_interval_secs: f64,
    /// Inventory put algorithm name.
    pub put_algorithm: String,
    /// Maximum totes per order (set cover loop cap).
    pub max_totes_per_order: usize,
    /// PPF optimization target: target picks per face.
    pub target_picks_per_face: f64,
    /// Override max_inflight_picks for all stations. When `None` (default),
    /// depth is auto-derived from map geometry as
    /// `ceil(queue_factor * F_max / effective_P) + 2`.
    /// Useful in tests with small maps where the geometry-derived depth is too
    /// shallow to exercise the desired pipeline behavior.
    #[serde(skip_serializing_if = "Option::is_none", alias = "assignment_depth_override")]
    pub max_inflight_picks_override: Option<u32>,
    /// Minimum simulation-time hold after physical order-buffer storage,
    /// before retrieval becomes eligible. Retrieval can occur later if
    /// resources are unavailable. Applies only to orders whose `exit_path`
    /// is `buffer`, and only on a map that states buffer racking.
    ///
    /// Synthetic policy: the Walmart reader retains an interpreted cutoff as
    /// `OrderSpec::deadline`, but its meaning is unconfirmed and it does not control
    /// retrieval. A fixed hold does not model shared route/truck releases. The
    /// default 0.0 adds no intentional storage dwell, which models the
    /// buffer's TRAFFIC honestly while claiming nothing about its OCCUPANCY.
    pub buffer_dwell_secs: f64,
    /// Experiment control: release every order at T0 while retaining its stated arrival metadata.
    pub release_all_at_start: bool,
    /// Experiment control: route every completed order directly to extraction without rewriting its source exit path.
    pub direct_extraction: bool,
    /// Extraction service duration, independent of operator pick service.
    pub extraction_secs: f64,
    /// Whether a picked order becomes a physical tote that has to be loaded
    /// and carried out of the building.
    ///
    /// True is the real warehouse: `OrderCloseOut` fetches an empty order
    /// tote, loads the closed order at the station that picked it, and takes
    /// it to an extraction station (via the order buffer for orders that
    /// state that exit path).
    ///
    /// False is PICK-ONLY: an order ends at the Easy Loop and nothing leaves.
    /// Asked for by Vinay (2026-08-31) to measure traffic and congestion with
    /// and without order-bot movement, which needs the two flows separable.
    /// The pick side is untouched either way -- orders are already abstract
    /// while being picked, and the tote only becomes physical at close-out --
    /// so the difference between a `true` and a `false` run is exactly the
    /// order-bot traffic.
    ///
    /// A run with this false completes no order TOTE trips, so every
    /// order-tote metric reads zero by construction. That is the point, but
    /// it means the two runs' bot-hours are not comparable without saying
    /// which is which.
    pub close_out: bool,
}

impl Default for OrderConfig {
    fn default() -> Self {
        Self {
            strategy: OrderStrategy::default(),
            dispatch_safety_net_interval_secs: 30.0,
            put_algorithm: "greedy".to_owned(),
            max_totes_per_order: 10,
            target_picks_per_face: 3.0,
            max_inflight_picks_override: None,
            buffer_dwell_secs: 0.0,
            release_all_at_start: false,
            direct_extraction: false,
            extraction_secs: 9.0,
            close_out: true,
        }
    }
}

/// How a pick station selects which order-lines compete for the next tote
/// when binning keeps multiple orders open concurrently.
///
/// All modes then pick the winner by soonest quoted station arrival; they
/// differ only in how many candidate lines each open order contributes to
/// the comparison.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PickSequencing {
    /// Non-sequencing (default): each open order contributes **all** its
    /// undispatched lines. Dispatch whichever tote, across every open line,
    /// arrives soonest.
    #[default]
    AllTotes,
    /// Each open order contributes only its `next_undispatched()` line —
    /// totes within an order are dispatched in line order.
    Sequenced,
}

impl PickSequencing {
    /// All variants, in GUI-dropdown order.
    pub const ALL: [PickSequencing; 2] = [PickSequencing::Sequenced, PickSequencing::AllTotes];

    /// Human label for the GUI dropdown and run summaries.
    pub fn label(self) -> &'static str {
        match self {
            PickSequencing::Sequenced => "Sequenced",
            PickSequencing::AllTotes => "No Sequencing",
        }
    }
}

impl FromStr for PickSequencing {
    type Err = String;
    /// `--settings pick.sequencing=<v>`. Accepts the serde snake_case names
    /// plus friendly aliases for the GUI labels.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "all_totes" | "no_sequencing" => Ok(PickSequencing::AllTotes),
            "sequenced" => Ok(PickSequencing::Sequenced),
            _ => Err(format!("invalid pick.sequencing '{s}' (all_totes|sequenced)")),
        }
    }
}

/// Trailing-global-barrier parameters (docs/global-barrier.md).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct BarrierConfig {
    /// Run the trailing barrier: the planner's settled present, advanced
    /// at executor quiescence, floors new claims, and gates admission
    /// instants (the pick embargo). Disabling pins the frontier at zero;
    /// a run that also configures `pick.embargo_secs > 0` is refused at
    /// startup — an embargo requires a present.
    pub enabled: bool,
}

impl Default for BarrierConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Pick subsystem parameters.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct PickConfig {
    /// Earliest simulation time, in seconds, when new order picking may begin.
    /// Putaway continues during this embargo so the storage buffer can fill
    /// before staffed pick stations start receiving work.
    pub embargo_secs: f64,
    /// Candidate-line model for binning's cross-order soonest-arrival choice.
    pub sequencing: PickSequencing,
    /// Prototype: pace pick dispatch to each station's presentation-slot
    /// frontier (just-in-time arrival via `select_relay_for_retrieve_near`)
    /// instead of always-soonest. Off by default; opt in to A/B the
    /// entry-cell-serialization fix. `--settings pick.pace_to_station=true`.
    ///
    /// DIAGNOSTIC/EXPERIMENTAL — an open-loop JIT approximation (an optimistic
    /// quote books a tight slot and risks starve-not-block). Kept for A/B
    /// exploration; not a validated production lever.
    pub pace_to_station: bool,
    /// Optional bound (seconds) on the station-delivery leg's pathfinder
    /// `max_wait`. When `Some`, `HtmDeliver` **defers** (retries next pass)
    /// instead of committing a long `Wait` on a station-approach cell — so a
    /// delivering HTM waits un-committed rather than blocking the entry cell.
    /// `None` (default) keeps the current loose/strict wait + abort-on-horizon.
    /// `--settings pick.delivery_max_wait_secs=120` (or `none`).
    ///
    /// KNOWN-BROKEN, diagnostic only — enabling stalls the planner (the
    /// deferred mid-chain `HtmDeliver` keeps holding its tote, so the planner
    /// spins on retries and makes no progress). Kept for the record; do not
    /// enable.
    pub delivery_max_wait_secs: Option<f64>,
}

impl Default for PickConfig {
    fn default() -> Self {
        Self {
            embargo_secs: 0.0,
            sequencing: PickSequencing::default(),
            pace_to_station: false,
            delivery_max_wait_secs: None,
        }
    }
}

/// Station operator parameters.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct OperatorConfig {
    /// Time per pick in seconds (dwell time).
    pub time_per_pick_secs: f64,
    /// Bot-to-bot handoff gap at station in seconds.
    pub transition_time_secs: f64,
    /// Time variance coefficient (0.0 for Java parity, 0.15 for production log-normal).
    /// Superseded by `dwell_variance_pct` when that field is non-zero.
    pub time_variance: f64,
    /// Dwell time variance as a percentage (e.g. 15.0 = ±15%).
    /// Actual dwell sampled from Uniform(P*(1-v), P*(1+v)) where v = pct/100.
    /// When non-zero, takes precedence over `time_variance`.
    pub dwell_variance_pct: f64,
    /// Number of bins per station.
    pub num_bins: usize,
    /// Time to clear a finished order's pallet from its bin (remove/wrap/
    /// stage) before the position can accept a new order. The bin is released
    /// this long after the order's last pick, so a successor's picks can't
    /// start during the swap. Default 0 (release at last pick) until a
    /// client-supplied value exists. CLI: `--settings operator.pallet_swap_secs=N`.
    pub pallet_swap_secs: f64,
}

impl Default for OperatorConfig {
    fn default() -> Self {
        Self {
            time_per_pick_secs: 9.0,
            transition_time_secs: 3.0,
            time_variance: 0.0,
            dwell_variance_pct: 15.0,
            num_bins: 8,
            pallet_swap_secs: 0.0,
        }
    }
}

impl OperatorConfig {
    /// Effective variance coefficient for dwell time sampling.
    /// Returns `dwell_variance_pct / 100.0` when non-zero, otherwise `time_variance`.
    pub fn effective_time_variance(&self) -> f64 {
        if self.dwell_variance_pct != 0.0 { self.dwell_variance_pct / 100.0 } else { self.time_variance }
    }
}

/// Putaway subsystem parameters.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct PutawayConfig {
    /// Whether putaway is enabled. When true, racks start with empty totes
    /// and a putaway phase precedes picking.
    pub enabled: bool,
    /// IGNORED: the supply service now derives each putaway station's staging
    /// depth from that station's own `F_max` and the induction period
    /// (`ceil(F_max / P) + 2`), so a hand-set count no longer has a meaning
    /// that survives a change of map. Kept as a parsed field so existing
    /// scenario JSON and `--settings putaway.queue_depth=N` still load; a
    /// supplied value draws one startup warning naming the derived caps that
    /// replaced it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_depth: Option<u32>,
    /// VESTIGIAL under feeding-decoupling: read by nothing on the timing path.
    /// Retained as a `#[serde(default)]` field only so existing scenario JSON
    /// that still carries `putaway.dwell_secs` (e.g. cencora_phase2.json) parses
    /// without surprise. Drop the field and JSON key together later.
    #[serde(default = "default_dwell_secs")]
    pub dwell_secs: f64,
    /// Per-station induction rate ceiling (presentations/hour). The demand
    /// generator (`PutawayInduction`) derives a pacing interval
    /// `3600/max_presentations_per_hour` seconds and admits at most one fill
    /// per station per interval. This is the real home of the rate cap that an
    /// inflated `dwell_secs` used to fake. 200/hr → an 18s interval.
    #[serde(default = "default_max_presentations_per_hour")]
    pub max_presentations_per_hour: f64,
    /// Selection-only handicap tilting the choice between direct empty-tote
    /// HTMs and from-rack relay chains. Applied to a direct candidate's timing
    /// error when picking the winning proposal; never alters the arrival
    /// estimate fed into scheduling — only the comparison.
    ///
    /// - `None` (default; `--settings putaway.direct_bias_secs=auto`):
    ///   derive the handicap from the map's Fleet Pressure Index (mean
    ///   nearest-relay→station fetch leg) at solver init. Auto-scales per map.
    /// - `Some(secs)`: fixed signed handicap — positive favors direct (it wins
    ///   unless from-rack beats it by more than this), negative penalizes
    ///   direct, 0 = neutral (purely earliest-arrival wins).
    #[serde(default)]
    pub direct_bias_secs: Option<f64>,
    /// Comparison lever, not policy: from this sim instant on, a putaway
    /// opening never stages an empty from the rack — it waits for a post-pick
    /// empty instead, so every staging past the instant is direct by
    /// construction. Exists to measure what pure direct circulation does to
    /// the putaway rate against the bias-driven choice. Set it a little
    /// before the pick embargo lifts so no rack chain committed just before
    /// the boundary dispatches after it. `None` (default;
    /// `--settings putaway.forbid_rack_after_secs=never`): the rack stays a
    /// source throughout.
    #[serde(default)]
    pub forbid_rack_after_secs: Option<f64>,
    /// The cap on HTMs bringing empty totes to each putaway station, as a
    /// multiple of the station's critical WIP, its uncontended fetch time
    /// `F_max` over its induction interval `P`: the putaway admission law caps
    /// a station's staged empty totes at `ceil(htm_wip_factor × F_max / P) + 2`.
    /// Set apart from the pick side's `queue_factor`. `None` (default): 1.0, a
    /// pipeline just deep enough to cover the uncontended fetch. A stated
    /// factor must be finite and above zero. CLI:
    /// `--settings putaway.htm_wip_factor=1.5`.
    #[serde(default)]
    pub htm_wip_factor: Option<f64>,
    /// When `true`, putaway ignores each lot's `PRD_HR`-derived net-time
    /// floor: lots induct in product-sequence order with no hourly gate
    /// (net-times loaded as `SimTime::ZERO`). Default `false` keeps the
    /// gate active — a lot may not be presented before its PRD_HR. Only
    /// meaningful for `BulkSource` scenarios (others carry no net-times);
    /// a no-op elsewhere. CLI: `--settings putaway.ignore_hour_gate=true`.
    pub ignore_hour_gate: bool,
    /// Load-spreading probe: pick the putaway deposit's destination relay
    /// uniformly at random among the candidates the selector would
    /// otherwise rank, instead of by its locality-and-frontier score. This
    /// deliberately reverses the placement bias, to measure how much of the
    /// observed hot-zone saturation is placement-driven — putaways are the
    /// large majority of presentations, so where they land is most of where
    /// the load lands. A measurement lever, NOT a production policy: it
    /// throws away the travel and readiness ranking the selector exists to
    /// compute. Default `false`. Draws come from the run's seeded PRNG, so
    /// a seed replays its choices. CLI:
    /// `--settings putaway.random_slots=true`.
    pub random_slots: bool,
}

fn default_dwell_secs() -> f64 {
    12.0
}

fn default_max_presentations_per_hour() -> f64 {
    200.0
}

impl Default for PutawayConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            queue_depth: None,
            dwell_secs: default_dwell_secs(),
            max_presentations_per_hour: default_max_presentations_per_hour(),
            direct_bias_secs: None,
            forbid_rack_after_secs: None,
            htm_wip_factor: None,
            ignore_hour_gate: false,
            random_slots: false,
        }
    }
}

impl PutawayConfig {
    /// The multiplier the putaway admission law runs at: the stated
    /// `htm_wip_factor`, else 1.0.
    pub fn htm_wip_multiplier(&self) -> f64 {
        self.htm_wip_factor.unwrap_or(1.0)
    }

    /// Refuse a stated HTM WIP factor the law cannot use. The law floors a
    /// station's pipeline at one tote, so a factor of zero or less would not
    /// mean what it states, and an infinite one derives no cap.
    pub fn validate(&self) -> Result<(), InvalidFigure> {
        self.htm_wip_factor.map_or(Ok(()), |factor| above_zero("putaway.htm_wip_factor", factor))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::config::{InvalidFigure, SimConfig};

    /// Unstated, the putaway law runs at 1.0 whatever the pick side's queue
    /// factor; a factor a spec's settings block states replaces it.
    #[test]
    fn the_putaway_law_runs_at_one_until_a_settings_block_states_a_factor() {
        let unstated: SimConfig = serde_json::from_value(json!({ "queue_factor": 4.0 })).unwrap();
        assert_eq!(unstated.putaway.htm_wip_multiplier(), 1.0, "the queue factor sizes the pick side only");
        let stated: SimConfig =
            serde_json::from_value(json!({ "queue_factor": 4.0, "putaway": { "htm_wip_factor": 1.5 } })).unwrap();
        assert_eq!(stated.putaway.htm_wip_multiplier(), 1.5);
    }

    /// A stated factor of zero or less, or one that is not finite, is refused
    /// under its own key; an unstated factor and a usable one pass.
    #[test]
    fn an_htm_wip_factor_the_law_cannot_use_is_refused_under_its_own_key() {
        let validate_with = |factor: Option<f64>| {
            let mut config = SimConfig::default();
            config.putaway.htm_wip_factor = factor;
            config.validate()
        };
        for bad in [0.0, -1.5, f64::NAN, f64::INFINITY] {
            assert!(
                matches!(
                    validate_with(Some(bad)),
                    Err(InvalidFigure::MustBeAboveZero { key: "putaway.htm_wip_factor", .. })
                ),
                "putaway.htm_wip_factor={bad}"
            );
        }
        assert_eq!(validate_with(None), Ok(()));
        assert_eq!(validate_with(Some(1.5)), Ok(()));
    }
}
