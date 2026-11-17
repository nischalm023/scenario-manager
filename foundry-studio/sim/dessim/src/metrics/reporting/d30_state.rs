//! Within `reporting`, `d30_state` partitions D30 FLEET-TIME: HTM journey-leg
//! facts (`htm_leg_log`) become state/wait-reason/travel-distance tables where
//! every measured bucket partitions each D30's time exactly once. "State"
//! means D30 operational state per the approved #261 taxonomy (Pickup, Drop,
//! Alignment, travel splits, Station processing, Waiting), not observer or
//! program state. Owns the `d30.*` metric descriptors. Surface-agnostic.
//!
//! Waiting is reported in TWO vocabularies over one classification. Each
//! elementary interval is classified once, from the journal facts, and lands
//! in both: [`D30WaitCategory`] is the ratified #272 taxonomy (Daniel,
//! 2026-07-30) and the one to build on; [`D30WaitReason`] is the vocabulary
//! that predates it, kept because the metrics panel and xlsx export still
//! render it, and retired when they adopt the categories. Both partition the
//! same Waiting total exactly, so neither can drift from the other.
//!
//! Attribution is POSITIONAL AND TASK-BASED interpretation of an already
//! solved schedule. This is an offline-planned simulator: there is no
//! present, and no waste or waiting except as rationalized in hindsight from
//! the schedule that was produced. A "wait" here is an interval the schedule
//! leaves between two committed actions, named from the facts that bound it
//! — never an observation of a robot idling, deciding, or being blocked.
//!
//! Fact-grounded caveats every consumer inherits:
//! - The journal's live rows are `Travel` legs and `Wait` holds, and inside a
//!   nav chain they partition its wall-clock: one Travel row per contiguous
//!   run of moves and rotations, one Wait row per hold between them.
//!   Pickup/Drop/endpoint-Alignment have no duration-bearing facts until
//!   #275 — their rows stay `Unsupported`, never a measured zero.
//! - Station processing is the recorded operator dwell and nothing else
//!   (Daniel's #272 ruling). The producer journals the pre-dwell hold as
//!   Presenting or In queue, leaving the dwell as the one stretch of a station
//!   visit no row covers; a visit the run stopped inside before any dwell was
//!   recorded is all pre-dwell hold. The stand after the dwell runs to the
//!   departure or, when the run stopped first, to the current world time.
//!   Station time past both the current world time and the dwell's end, with
//!   no departure to bound it, is impossible and fails report construction
//!   (ruling #49), rather than becoming waiting. A route hold on the
//!   presentation cell before or after the dwell is ordinary Presenting time
//!   and is not that shape.
//! - A no-leg gap bounded by rows at the SAME station endpoint is Station
//!   processing; every other no-leg gap is Waiting → Unknown. What remains
//!   in those gaps after the enrichment is dispatch seams and run-end
//!   truncation: intervals the rows on either side bound in time but say
//!   nothing about, where claiming a reason would fabricate attribution. This
//!   deliberately diverges from the DA50 family's gaps→NoTask convention: D30s
//!   have a real NoTask producer (explicit idle rows), so booking unfacted
//!   gaps to NoTask would poison a measured reason with fabricated time.
//! - A configured D30 the run never journaled is a different residual again:
//!   nothing bounds its window, so its whole share of fleet time is Waiting →
//!   No facts. Splitting it out of Unknown is what makes Unknown mean one
//!   thing — a gap the facts reach but under-determine.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::model::{RobotId, SimDuration, SimTime};
use crate::observer::output::{
    Activity, HtmHold, HtmJournalEndpoint, HtmLegRecord, HtmWaitCause, HtmWaitPosition, HtmWaitReason,
};

use super::{
    AggregationClass, MetricDescriptor, MetricUnit, MetricsReport, Ratio, ReportBuildError, ReportRequest,
    SupportStatus, TimeBucket, report_buckets,
};

/// Canonical report ordering of D30 states — the #261 §2.2 partition order.
/// Consumers must use this, never a private copy — the taxonomy has exactly
/// one home.
pub const D30_STATES: [D30State; 9] = [
    D30State::Pickup,
    D30State::Drop,
    D30State::Alignment,
    D30State::EmptyTravel,
    D30State::LadenTravel,
    D30State::TravelToPark,
    D30State::StationProcessing,
    D30State::Waiting,
    D30State::Unknown,
];

/// Canonical report ordering of the ratified #272 waiting taxonomy. Single
/// home; see above. The three residuals come last, ordered by how much the
/// facts gave: a bounded gap they under-determine, then a stretch they never
/// reached, then a stretch they contradict each other over. The span engine's
/// D30 wait definition lists these in this order, and beside them the one
/// category only the robot state channel states.
pub const D30_WAIT_CATEGORIES: [D30WaitCategory; 12] = [
    D30WaitCategory::NoTaskParked,
    D30WaitCategory::NoTaskElsewhere,
    D30WaitCategory::Presenting,
    D30WaitCategory::AwaitingStationDeparture,
    D30WaitCategory::AwaitingRelayDepositResident,
    D30WaitCategory::AwaitingRelayDepositDrove,
    D30WaitCategory::AwaitingRelaySlot,
    D30WaitCategory::InQueueStation,
    D30WaitCategory::Congestion,
    D30WaitCategory::Unknown,
    D30WaitCategory::NoFacts,
    D30WaitCategory::ConflictingFacts,
];

/// Canonical report ordering of the PRE-#272 wait vocabulary plus the
/// `ConflictingFacts` honesty bucket. Single home; see above.
pub const D30_WAIT_REASONS: [D30WaitReason; 8] = [
    D30WaitReason::NoTask,
    D30WaitReason::AwaitingDeposit,
    D30WaitReason::ScheduledRouteHold,
    D30WaitReason::StationQueueHold,
    D30WaitReason::PostPresentationReturnHold,
    D30WaitReason::AwaitingReturnSlot,
    D30WaitReason::Unknown,
    D30WaitReason::ConflictingFacts,
];

/// Canonical report ordering of travel-distance kinds. Single home; see above.
pub const D30_TRAVEL_KINDS: [D30TravelKind; 3] = [D30TravelKind::Empty, D30TravelKind::Laden, D30TravelKind::Parking];

/// Mutually exclusive D30 states used by duration and percentage reports —
/// the exhaustive partition Daniel approved on #261 (2026-07-29), including
/// `StationProcessing` as the ninth state.
///
/// `Pickup`, `Drop`, and `Alignment` intentionally remain in the stable
/// taxonomy even though no duration-bearing endpoint-service facts exist
/// until #275. Their rows carry `Unsupported`, not a fabricated zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum D30State {
    Pickup,
    Drop,
    /// Endpoint docking/final positioning. In-path chassis rotation is part
    /// of travel, never this state (#261 §2.2).
    Alignment,
    EmptyTravel,
    LadenTravel,
    /// Takes precedence over payload classification: a leg to a parking cell
    /// is never also counted as empty travel.
    TravelToPark,
    /// The recorded operator dwell at the presentation cell. A pre-dwell
    /// station hold is Waiting (Presenting or In queue), and so is the stand
    /// after it (Awaiting Departure (station)); station time past both the
    /// current world time and the dwell's end, with no departure to bound it,
    /// is an integrity error under ruling #49. In-path return rotation is
    /// Travel, never station processing.
    StationProcessing,
    Waiting,
    /// Conflicting or incomplete facts overlap this interval.
    Unknown,
}

/// The ratified #272 waiting taxonomy (Daniel, 2026-07-30): flat categories,
/// each a joint reading of a hold's task context and its position, plus the
/// honesty residuals. Categories describe the selected schedule; they are
/// never proof of congestion, unavoidable delay, or scheduler optimality, and
/// never a claim about what a robot was doing.
///
/// A residual is named for the FACT SITUATION that stranded the time, not for
/// a state the robot was in — the derivation Daniel ratified on 2026-07-31,
/// following the `ConflictingFacts` precedent. So the three residuals divide
/// by what the journal offered: facts that bound the interval but leave its
/// cause open (`Unknown`, rendered "Unattributed gap"), facts that never
/// reached it (`NoFacts`), and facts that contradict each other over it
/// (`ConflictingFacts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum D30WaitCategory {
    /// No task assigned, and the position facts put the D30 at a relay or
    /// charging cell.
    NoTaskParked,
    /// No task assigned, and the position facts put the D30 somewhere that is
    /// not a relay or charging cell. Positional only: "elsewhere" says where the
    /// schedule left the robot, nothing about behaviour.
    NoTaskElsewhere,
    /// Held at a station's presentation cell while nobody is serving the D30 —
    /// any committed hold there that the recorded dwell does not cover. Two
    /// shapes reach it: the arrival wait before the dwell opens, and the
    /// departure hold after it closes, where the path search parks the D30 on
    /// the cell until the corridor out of the station clears. Both are the same
    /// fact for the station: its one presentation cell is occupied by a D30
    /// nobody is serving.
    ///
    /// The determination is unchanged — position (a presentation cell) read
    /// jointly with the task facts that bound the hold. Only this gloss
    /// widened, and from evidence: issue #375 found pathfinder-committed
    /// departure holds at presentation cells, 198 of them in 15.4Ms of sim,
    /// which the earlier "pre-service wait" wording did not admit.
    Presenting,
    /// Standing on the presentation cell after the operator finished, until
    /// the next leg's admitted departure or, when the run stopped before one
    /// was scheduled, until the current world time, before which a departure
    /// scheduled later could not start. Bounded either way, so it is occupancy,
    /// distinct from the factless tail past both the current world time and the
    /// dwell's end that ruling #49 still refuses.
    ///
    /// Positive since the store leg's departure began flooring at the
    /// scheduler's settled frontier: a return admitted later than the
    /// presentation ended leaves the D30 genuinely standing here meanwhile.
    AwaitingStationDeparture,
    /// Standing anywhere but a station's presentation cell with its next move
    /// committed: from the simtime the barrier had reached when the move was
    /// committed to the move's departure. The journal files that stand as no
    /// task, so this report never lists the category; only the robot state
    /// channel states it, and the span engine's D30 wait definition charts it.
    AwaitingDepartureElsewhere,
    /// Held at a relay until the DA50 deposited the tote to fork up, by a
    /// D30 that was already standing on that relay's access cell when the
    /// chain chose it. It never drove for this wait: the interval is the
    /// DA50 cycle's lead time passing under a bot that was there anyway, so
    /// the time is near-free and the arrival quote was never wrong.
    AwaitingRelayDepositResident,
    /// Held at a relay until the DA50 deposited the tote to fork up, by a
    /// D30 that drove here to meet a quoted deposit instant and arrived
    /// before the tote. Unlike the resident case this is a real quote miss:
    /// the travel could have started later and the bot could have been
    /// elsewhere meanwhile.
    /// Pre-split profiles wrote the undivided category; they read as drove.
    #[serde(alias = "AwaitingRelayDeposit")]
    AwaitingRelayDepositDrove,
    /// Held at the return relay until its diving-board slot reservation
    /// became valid. The schedule does not delay the station departure for a
    /// slot, so this wait is always at the relay, never at the origin (#272's
    /// open question, resolved from the code on 2026-07-30). This is the
    /// model's current placement; entry to an occupied access cell is a known
    /// simplification pending an arrive-after path constraint (#153).
    AwaitingRelaySlot,
    /// Held on a station-owned queue cell.
    InQueueStation,
    /// Every other hold the selected route imposes partway along a leg. The
    /// "queue entrance" idea folds in here for v1.
    Congestion,
    /// A bounded gap whose facts under-determine attribution — rendered as
    /// "Unattributed gap". Journal rows on both sides fix when it started and
    /// when it ended, but nothing in them says why: a dispatch seam, run-end
    /// truncation, or a hold whose position facts did not place it.
    ///
    /// The Rust name stays `Unknown` because renaming it ripples through every
    /// consumer; the rendered name is the one that carries the meaning.
    Unknown,
    /// No journal facts cover this share of fleet time at all — currently the
    /// whole window of a configured robot that produced no journey facts.
    /// Distinct from `Unknown`: there, facts bound a gap but under-determine
    /// its cause.
    NoFacts,
    /// Multiple overlapping wait facts disagree.
    ConflictingFacts,
}

/// The PRE-#272 waiting vocabulary, kept because the metrics panel and the
/// xlsx export still render it. Retired once they adopt
/// [`D30WaitCategory`]; until then both are filled from one classification
/// and both partition Waiting exactly.
///
/// `PostPresentationReturnHold` remains only for enum compatibility and is
/// unsupported: a positive interval is an integrity error. No variant here
/// names Presenting, which therefore projects to `Unknown`. Consumers wanting
/// the ratified split must read the categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum D30WaitReason {
    /// Idle with no assigned task — the explicit no-task wait fact journaled
    /// for each gap between a D30's tasks.
    /// Parked and elsewhere both land here; the ratified split is
    /// [`D30WaitCategory::NoTaskParked`] / `NoTaskElsewhere`.
    NoTask,
    /// Dwelling at the diving board awaiting the selected DA50 tote deposit.
    AwaitingDeposit,
    /// A hold the selected route imposes partway along a leg — the ratified
    /// taxonomy's Congestion. Measured since #272: the producer journals a
    /// `Wait` row per in-path hold instead of leaving it in the space
    /// between rows.
    ScheduledRouteHold,
    /// A hold on a station-owned queue cell. Measured since #272: the
    /// producer reads queue-cell membership off the map's own cell kinds, so
    /// a zero here is a measured zero — the schedule leaves no D30 on a queue
    /// cell — not an unmodelled blank.
    StationQueueHold,
    /// Compatibility row for a post-dwell station interval. Unsupported under
    /// ruling #49: the return departs at action-ready time, and an open-ended
    /// reservation is a wall-clock planning bookmark, not simulation-time
    /// occupancy. A positive producer fact fails report construction.
    PostPresentationReturnHold,
    /// Awaiting the return diving-board slot — the ratified taxonomy's
    /// Awaiting Relay Slot. Measured since #272 from `HtmRelayDepositEvent`,
    /// the per-robot dwell at the relay access cell. (The older
    /// `DivingBoardSlotWaitEvent` measures a larger, relay-attributed
    /// quantity that includes the D30's travel time and names no robot.)
    AwaitingReturnSlot,
    /// A no-leg gap no captured fact attributes (dispatch seams, run-end
    /// truncation), a hold whose position facts did not place it, or a hold
    /// this vocabulary has no name for — Presenting.
    Unknown,
    /// Multiple overlapping wait facts disagree on their reason.
    ConflictingFacts,
}

/// Travel-distance split. `Parking` is a destination split, not a payload
/// state — it wins over empty/laden exactly as [`D30State::TravelToPark`]
/// wins in the state partition, so the three kinds sum to total classified
/// navigation distance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum D30TravelKind {
    Empty,
    Laden,
    Parking,
}

impl D30State {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pickup => "Pickup",
            Self::Drop => "Drop",
            Self::Alignment => "Alignment",
            Self::EmptyTravel => "Empty travel",
            Self::LadenTravel => "Laden travel",
            Self::TravelToPark => "Travel to park",
            Self::StationProcessing => "Station processing",
            Self::Waiting => "Waiting",
            Self::Unknown => "Unknown",
        }
    }
}

impl D30WaitCategory {
    pub fn label(self) -> &'static str {
        match self {
            Self::NoTaskParked => "No task (relay/charger)",
            Self::NoTaskElsewhere => "No task (elsewhere)",
            Self::Presenting => "Presenting",
            Self::AwaitingStationDeparture => "Awaiting Departure (station)",
            Self::AwaitingDepartureElsewhere => "Awaiting Departure (elsewhere)",
            Self::AwaitingRelayDepositResident => "Awaiting Relay Deposit (resident)",
            Self::AwaitingRelayDepositDrove => "Awaiting Relay Deposit (drove)",
            Self::AwaitingRelaySlot => "Awaiting Relay Slot",
            Self::InQueueStation => "In Queue (station)",
            Self::Congestion => "Congestion",
            Self::Unknown => "Unattributed gap",
            Self::NoFacts => "No facts",
            Self::ConflictingFacts => "Conflicting facts",
        }
    }
}

impl D30TravelKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Empty => "Empty",
            Self::Laden => "Laden",
            Self::Parking => "Parking",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct D30StateDurationRow {
    pub bucket: TimeBucket,
    pub state: D30State,
    pub status: SupportStatus,
    /// `None` distinguishes unknown/unsupported from a measured zero.
    pub duration_ms: Option<u64>,
    /// State duration divided by D30 fleet-time in this bucket.
    pub fleet_share: Option<Ratio>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct D30WaitDurationRow {
    pub bucket: TimeBucket,
    pub reason: D30WaitReason,
    pub status: SupportStatus,
    pub duration_ms: Option<u64>,
    /// Reason duration divided by all Waiting time in this bucket.
    pub waiting_share: Option<Ratio>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct D30DistanceRow {
    pub bucket: TimeBucket,
    pub kind: D30TravelKind,
    pub status: SupportStatus,
    pub distance_mm: Option<u64>,
}

/// One bucket's waiting time under one ratified #272 category — the
/// [`D30WaitCategory`] counterpart of [`D30WaitDurationRow`], emitted per
/// bucket so a timeline can break its Waiting band down instead of drawing
/// one merged segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct D30WaitCategoryRow {
    pub bucket: TimeBucket,
    pub category: D30WaitCategory,
    pub status: SupportStatus,
    pub duration_ms: Option<u64>,
    /// Category duration divided by all Waiting time in this bucket.
    pub waiting_share: Option<Ratio>,
}

/// One observed D30's waiting time under one ratified #272 category in one
/// bucket — the per-vehicle drilldown of [`D30WaitCategoryRow`].
///
/// The D30-ness of the row is its own name; the id it carries is an ordinary
/// [`RobotId`] from the one robot id space, not a separate HTM numbering.
/// (The `htm_id: u32` on the older sibling rows is the same id, weakly typed.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct D30VehicleWaitCategoryRow {
    pub bucket: TimeBucket,
    pub robot_id: RobotId,
    pub category: D30WaitCategory,
    pub status: SupportStatus,
    pub duration_ms: Option<u64>,
    /// Category duration divided by this D30's Waiting time in the bucket.
    pub waiting_share: Option<Ratio>,
}

/// One observed D30's share of one state in one bucket — the per-vehicle
/// drilldown of [`D30StateDurationRow`]. Fleet rows equal the sum of these
/// plus the Waiting/NoTask backfill for configured-but-unobserved vehicles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct D30VehicleStateDurationRow {
    pub bucket: TimeBucket,
    pub htm_id: u32,
    pub state: D30State,
    pub status: SupportStatus,
    pub duration_ms: Option<u64>,
    /// State duration divided by this D30's included time in the bucket.
    pub vehicle_share: Option<Ratio>,
}

/// One observed D30's waiting time for one reason in one bucket — the
/// per-vehicle drilldown of [`D30WaitDurationRow`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct D30VehicleWaitDurationRow {
    pub bucket: TimeBucket,
    pub htm_id: u32,
    pub reason: D30WaitReason,
    pub status: SupportStatus,
    pub duration_ms: Option<u64>,
    /// Reason duration divided by this D30's Waiting time in the bucket.
    pub waiting_share: Option<Ratio>,
}

/// One observed D30's travel distance under one kind in one bucket — the
/// per-vehicle drilldown of [`D30DistanceRow`]; fleet rows equal the sum of
/// these (unobserved vehicles travel nowhere).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct D30VehicleDistanceRow {
    pub bucket: TimeBucket,
    pub htm_id: u32,
    pub kind: D30TravelKind,
    pub status: SupportStatus,
    pub distance_mm: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct D30StateData {
    pub fleet_size: u32,
    pub state_rows: Vec<D30StateDurationRow>,
    /// Waiting in the ratified #272 vocabulary. The one to build on.
    pub wait_category_rows: Vec<D30WaitCategoryRow>,
    /// Waiting in the pre-#272 vocabulary, over the same classification.
    pub wait_rows: Vec<D30WaitDurationRow>,
    pub distance_rows: Vec<D30DistanceRow>,
    pub vehicle_state_rows: Vec<D30VehicleStateDurationRow>,
    pub vehicle_wait_category_rows: Vec<D30VehicleWaitCategoryRow>,
    pub vehicle_wait_rows: Vec<D30VehicleWaitDurationRow>,
    pub vehicle_distance_rows: Vec<D30VehicleDistanceRow>,
}

/// The metric descriptors this producer owns and emits with every report.
/// Callable without data, so surfaces can enumerate D30 state metrics
/// before a run exists.
pub fn d30_state_descriptors() -> Vec<MetricDescriptor> {
    vec![
        MetricDescriptor::new(
            "d30.state.duration",
            "D30 state time",
            "Time attributed to each mutually exclusive D30 state.",
            MetricUnit::DurationMilliseconds,
            Some("D30 fleet availability time".into()),
            AggregationClass::Interval,
        ),
        MetricDescriptor::new(
            "d30.wait.category.duration",
            "D30 waiting time by category",
            "Journey-journal hold facts attributed post hoc to the ratified waiting taxonomy.",
            MetricUnit::DurationMilliseconds,
            Some("D30 waiting time".into()),
            AggregationClass::Interval,
        ),
        MetricDescriptor::new(
            "d30.wait.duration",
            "D30 waiting time",
            "The same attribution in the pre-#272 reason vocabulary.",
            MetricUnit::DurationMilliseconds,
            Some("D30 waiting time".into()),
            AggregationClass::Interval,
        ),
        MetricDescriptor::new(
            "d30.travel.distance.empty",
            "D30 empty travel distance",
            "Floor distance traveled without a tote on board, excluding travel to park.",
            MetricUnit::DistanceMillimeters,
            None,
            AggregationClass::Additive,
        ),
        MetricDescriptor::new(
            "d30.travel.distance.laden",
            "D30 laden travel distance",
            "Floor distance traveled with a tote on board, excluding travel to park.",
            MetricUnit::DistanceMillimeters,
            None,
            AggregationClass::Additive,
        ),
        MetricDescriptor::new(
            "d30.travel.distance.parking",
            "D30 parking travel distance",
            "Floor distance traveled on legs whose destination is a parking cell.",
            MetricUnit::DistanceMillimeters,
            None,
            AggregationClass::Additive,
        ),
    ]
}

/// Build canonical D30 state, waiting-attribution, and travel-distance
/// tables from HTM journey-leg facts.
///
/// Every measured bucket partitions each D30's fleet-time exactly once.
/// Explicit `Wait` rows carry their reason; a no-leg gap bounded by legs at
/// the same station endpoint is `StationProcessing`; every other no-leg gap
/// is Waiting → Unknown (no fact attributes it). Conflicting overlapping
/// facts are `Unknown` instead of being double-counted. Waiting reasons
/// describe the chosen schedule post hoc; they do not prove congestion or
/// optimality.
///
/// Per-vehicle drilldown rows carry the same partition per observed D30:
/// each vehicle's states partition its own bucket time, and fleet rows are
/// the vehicle sums plus Waiting/NoTask backfill for configured-but-
/// unobserved vehicles.
///
/// The fleet denominator is the CONFIGURED eligible count (#261 §2.2's
/// "eligible D30 count × included duration"), never grown to fit the facts:
/// facts naming more distinct D30s than the config declares are an
/// integrity breach and fail hard, like every other reconciliation here.
/// (The DA50 family's `max()` denominator predates this adjudication and is
/// deliberately left untouched.) Overlapping same-robot travel facts also
/// fail hard: the state sweep would union them, but their distance would
/// double-count.
pub fn d30_state_report(
    records: &[HtmLegRecord],
    configured_fleet_size: u32,
    request: ReportRequest,
) -> Result<MetricsReport<D30StateData>, ReportBuildError> {
    let descriptors = d30_state_descriptors();
    let buckets = report_buckets(request)?;
    let observed_ids: BTreeSet<_> = records.iter().map(|record| record.robot_id).collect();
    let observed = observed_ids.len() as u32;
    if observed > configured_fleet_size {
        return Err(ReportBuildError::D30FleetExceeded { configured: configured_fleet_size, observed });
    }
    let fleet_size = configured_fleet_size;

    let mut state_rows = Vec::with_capacity(buckets.len() * D30_STATES.len());
    let mut wait_category_rows = Vec::with_capacity(buckets.len() * D30_WAIT_CATEGORIES.len());
    let mut wait_rows = Vec::with_capacity(buckets.len() * D30_WAIT_REASONS.len());
    let mut distance_rows = Vec::with_capacity(buckets.len() * D30_TRAVEL_KINDS.len());
    let mut vehicle_state_rows = Vec::new();
    let mut vehicle_wait_category_rows = Vec::new();
    let mut vehicle_wait_rows = Vec::new();
    let mut vehicle_distance_rows = Vec::new();
    let records_by_robot = records_by_robot(records, &observed_ids);
    check_travel_overlaps(&records_by_robot)?;
    check_post_dwell_station_holds(&records_by_robot)?;

    for bucket in buckets {
        let mut state_durations = zero_states();
        let mut category_durations = zero_wait_categories();
        let mut wait_durations = zero_wait_reasons();
        for (&robot_id, robot_records) in &records_by_robot {
            // The older sibling rows below still key on a raw `htm_id`.
            let htm_id = robot_id.0;
            let mut vehicle_states = zero_states();
            let mut vehicle_categories = zero_wait_categories();
            let mut vehicle_waits = zero_wait_reasons();
            accumulate_robot_time(
                robot_records,
                bucket,
                &mut vehicle_states,
                &mut vehicle_categories,
                &mut vehicle_waits,
            )?;

            // Each observed D30 partitions its own bucket time exactly once;
            // the fleet reconciliation below then holds by construction.
            let vehicle_total = checked_sum(vehicle_states.values().copied())?;
            if vehicle_total != bucket.coverage_ms {
                return Err(ReportBuildError::D30StateReconciliation {
                    bucket_start_ms: bucket.start_ms,
                    bucket_end_ms: bucket.end_ms,
                    measured_ms: vehicle_total,
                    fleet_time_ms: bucket.coverage_ms,
                });
            }
            let vehicle_waiting_ms = vehicle_states[&D30State::Waiting];
            // Both vocabularies partition this D30's Waiting exactly, so
            // neither can drift from the other or from the state partition.
            check_wait_partition(bucket, checked_sum(vehicle_categories.values().copied())?, vehicle_waiting_ms)?;
            check_wait_partition(bucket, checked_sum(vehicle_waits.values().copied())?, vehicle_waiting_ms)?;

            for state in D30_STATES {
                let measured_ms = vehicle_states[&state];
                add_duration(&mut state_durations, state, measured_ms)?;
                let status = d30_state_support(state);
                let duration_ms = (status == SupportStatus::Measured).then_some(measured_ms);
                vehicle_state_rows.push(D30VehicleStateDurationRow {
                    bucket,
                    htm_id,
                    state,
                    status,
                    duration_ms,
                    vehicle_share: duration_ms.map(|duration_ms| Ratio::new(duration_ms, bucket.coverage_ms)),
                });
            }
            for category in D30_WAIT_CATEGORIES {
                let measured_ms = vehicle_categories[&category];
                add_duration(&mut category_durations, category, measured_ms)?;
                vehicle_wait_category_rows.push(D30VehicleWaitCategoryRow {
                    bucket,
                    robot_id,
                    category,
                    status: SupportStatus::Measured,
                    duration_ms: Some(measured_ms),
                    waiting_share: Some(Ratio::new(measured_ms, vehicle_waiting_ms)),
                });
            }
            for reason in D30_WAIT_REASONS {
                let measured_ms = vehicle_waits[&reason];
                add_duration(&mut wait_durations, reason, measured_ms)?;
                let status = d30_wait_support(reason);
                let duration_ms = (status == SupportStatus::Measured).then_some(measured_ms);
                vehicle_wait_rows.push(D30VehicleWaitDurationRow {
                    bucket,
                    htm_id,
                    reason,
                    status,
                    duration_ms,
                    waiting_share: duration_ms.map(|duration_ms| Ratio::new(duration_ms, vehicle_waiting_ms)),
                });
            }
            for kind in D30_TRAVEL_KINDS {
                vehicle_distance_rows.push(D30VehicleDistanceRow {
                    bucket,
                    htm_id,
                    kind,
                    status: SupportStatus::Measured,
                    distance_mm: Some(distance_for_bucket(robot_records.iter().copied(), bucket, Some(kind))?),
                });
            }
        }

        let unobserved_robots = fleet_size.saturating_sub(records_by_robot.len() as u32);
        let unobserved_ms =
            bucket.coverage_ms.checked_mul(unobserved_robots as u64).ok_or(ReportBuildError::ArithmeticOverflow)?;
        add_duration(&mut state_durations, D30State::Waiting, unobserved_ms)?;
        // A configured-but-unobserved D30 produced no journey facts at all, so
        // no fact bounds this window and none names it — the fact situation
        // `NoFacts` exists for. It is not a gap the facts under-determine
        // (`Unknown`); the configuration is the only thing that says the robot
        // was there. The old vocabulary keeps calling it no task, which is what
        // the configuration alone supports.
        add_duration(&mut category_durations, D30WaitCategory::NoFacts, unobserved_ms)?;
        add_duration(&mut wait_durations, D30WaitReason::NoTask, unobserved_ms)?;

        let fleet_time_ms =
            bucket.coverage_ms.checked_mul(fleet_size as u64).ok_or(ReportBuildError::ArithmeticOverflow)?;
        let measured_total = checked_sum(state_durations.values().copied())?;
        if measured_total != fleet_time_ms {
            return Err(ReportBuildError::D30StateReconciliation {
                bucket_start_ms: bucket.start_ms,
                bucket_end_ms: bucket.end_ms,
                measured_ms: measured_total,
                fleet_time_ms,
            });
        }

        let waiting_ms = state_durations[&D30State::Waiting];
        check_wait_partition(bucket, checked_sum(category_durations.values().copied())?, waiting_ms)?;
        check_wait_partition(bucket, checked_sum(wait_durations.values().copied())?, waiting_ms)?;
        for state in D30_STATES {
            let status = d30_state_support(state);
            let duration_ms = (status == SupportStatus::Measured).then_some(state_durations[&state]);
            state_rows.push(D30StateDurationRow {
                bucket,
                state,
                status,
                duration_ms,
                fleet_share: duration_ms.map(|duration_ms| Ratio::new(duration_ms, fleet_time_ms)),
            });
        }
        for category in D30_WAIT_CATEGORIES {
            let duration_ms = category_durations[&category];
            wait_category_rows.push(D30WaitCategoryRow {
                bucket,
                category,
                // Every ratified category is attributable from the journal, so
                // each carries a real number — a zero here is a measured zero.
                status: SupportStatus::Measured,
                duration_ms: Some(duration_ms),
                waiting_share: Some(Ratio::new(duration_ms, waiting_ms)),
            });
        }
        for reason in D30_WAIT_REASONS {
            let status = d30_wait_support(reason);
            let duration_ms = (status == SupportStatus::Measured).then_some(wait_durations[&reason]);
            wait_rows.push(D30WaitDurationRow {
                bucket,
                reason,
                status,
                duration_ms,
                waiting_share: duration_ms.map(|duration_ms| Ratio::new(duration_ms, waiting_ms)),
            });
        }
        let mut split_mm = 0u64;
        for kind in D30_TRAVEL_KINDS {
            let distance_mm = distance_for_bucket(records, bucket, Some(kind))?;
            split_mm = split_mm.checked_add(distance_mm).ok_or(ReportBuildError::ArithmeticOverflow)?;
            distance_rows.push(D30DistanceRow {
                bucket,
                kind,
                status: SupportStatus::Measured,
                distance_mm: Some(distance_mm),
            });
        }
        // Empty + laden + parking must equal total classified navigation
        // distance — the distance analog of the state/wait checks above.
        let total_mm = distance_for_bucket(records, bucket, None)?;
        if split_mm != total_mm {
            return Err(ReportBuildError::D30DistanceReconciliation {
                bucket_start_ms: bucket.start_ms,
                bucket_end_ms: bucket.end_ms,
                split_mm,
                total_mm,
            });
        }
    }

    Ok(MetricsReport::new(
        request,
        descriptors,
        D30StateData {
            fleet_size,
            state_rows,
            wait_category_rows,
            wait_rows,
            distance_rows,
            vehicle_state_rows,
            vehicle_wait_category_rows,
            vehicle_wait_rows,
            vehicle_distance_rows,
        },
    ))
}

/// Reject a waiting vocabulary whose durations do not sum to the Waiting
/// state exactly. Both vocabularies run through here, so a divergence between
/// them fails as loudly as a divergence from the state partition.
fn check_wait_partition(bucket: TimeBucket, reason_ms: u64, waiting_ms: u64) -> Result<(), ReportBuildError> {
    if reason_ms == waiting_ms {
        return Ok(());
    }
    Err(ReportBuildError::D30WaitReconciliation {
        bucket_start_ms: bucket.start_ms,
        bucket_end_ms: bucket.end_ms,
        reason_ms,
        waiting_ms,
    })
}

fn checked_sum(mut values: impl Iterator<Item = u64>) -> Result<u64, ReportBuildError> {
    values.try_fold(0u64, |sum, value| sum.checked_add(value).ok_or(ReportBuildError::ArithmeticOverflow))
}

fn records_by_robot<'a>(
    records: &'a [HtmLegRecord],
    observed_ids: &BTreeSet<RobotId>,
) -> BTreeMap<RobotId, Vec<&'a HtmLegRecord>> {
    let mut by_robot: BTreeMap<_, Vec<_>> = observed_ids.iter().copied().map(|id| (id, Vec::new())).collect();
    for record in records {
        by_robot.entry(record.robot_id).or_default().push(record);
    }
    for robot_records in by_robot.values_mut() {
        robot_records.sort_by_key(|record| (record.start, record.duration));
    }
    by_robot
}

/// Reject overlapping same-robot duration-bearing Travel facts. The state
/// sweep unions overlaps (disagreements land in `Unknown`), but distance is
/// summed per record, so an overlap would double-count silently — and no
/// live producer emits one (one nav commit per leg, legs sequential per
/// robot), making this an integrity tripwire like the other hard errors.
fn check_travel_overlaps(records_by_robot: &BTreeMap<RobotId, Vec<&HtmLegRecord>>) -> Result<(), ReportBuildError> {
    for (&robot_id, robot_records) in records_by_robot {
        // Records are sorted by (start, duration); track the furthest travel
        // end seen and the start of the record that produced it.
        let mut furthest: Option<(SimTime, SimTime)> = None;
        for record in robot_records
            .iter()
            .filter(|record| record.activity == Activity::Travel && record.duration > SimDuration::ZERO)
        {
            let end = record.start + record.duration;
            if let Some((first_start, furthest_end)) = furthest {
                if record.start < furthest_end {
                    // `D30TravelOverlap` still names its times in raw ms.
                    return Err(ReportBuildError::D30TravelOverlap {
                        robot_id: robot_id.0,
                        first_start_ms: first_start.as_ms(),
                        second_start_ms: record.start.as_ms(),
                    });
                }
            }
            if furthest.is_none_or(|(_, furthest_end)| end > furthest_end) {
                furthest = Some((record.start, end));
            }
        }
    }
    Ok(())
}

/// Reject station time past both the current world time and the recorded
/// dwell's end, with no departure to bound it. What could linger there is the
/// open-ended station cell reservation, which the wall-clock planner leaves
/// unfinished and which has no simulation-time end at all. Reading occupancy
/// off it would invent time, so a row carrying that factless tail as
/// `PostStationService` fails here instead of inflating Waiting/Unknown
/// (ruling #49, 2026-07-30). Nothing journals one: a station visit ends at its
/// departure or, when the run stopped inside it, at the current world time,
/// before which a departure scheduled later could not start, or at its recorded
/// dwell's end if that is later; its stand after the dwell is
/// `AwaitingDeparture`.
///
/// The older reading of this rule — that HtmStore always starts the return at
/// the presentation action-ready time, so dwell and departure necessarily
/// meet — no longer holds: the store leg's departure floors at the
/// scheduler's settled frontier and can land later. That stand is bounded by
/// the departure that closed the visit, so `activity::visits` journals it as
/// `AwaitingDeparture` and it passes here as ordinary occupancy. Ruling #49
/// governs only the factless tail.
///
/// A hold the path search writes on the presentation cell at the head of the
/// return leg is NOT this shape: it is bounded by two of the D30's own
/// committed actions, so it is real occupancy and journals as a `RouteHold`
/// (issue #375).
fn check_post_dwell_station_holds(
    records_by_robot: &BTreeMap<RobotId, Vec<&HtmLegRecord>>,
) -> Result<(), ReportBuildError> {
    for (&robot_id, robot_records) in records_by_robot {
        if let Some(record) = robot_records.iter().find(|record| {
            record.activity == Activity::Wait
                && record.duration > SimDuration::ZERO
                && record.hold.is_some_and(|hold| hold.cause == HtmWaitCause::PostStationService)
        }) {
            return Err(ReportBuildError::D30PostDwellStationHold {
                robot_id,
                start: record.start,
                // `SimTime + SimDuration` saturates, so a row whose end runs
                // off the clock still names a time rather than overflowing.
                end: record.start + record.duration,
            });
        }
    }
    Ok(())
}

fn zero_states() -> BTreeMap<D30State, u64> {
    D30_STATES.into_iter().map(|state| (state, 0)).collect()
}

fn zero_wait_categories() -> BTreeMap<D30WaitCategory, u64> {
    D30_WAIT_CATEGORIES.into_iter().map(|category| (category, 0)).collect()
}

fn zero_wait_reasons() -> BTreeMap<D30WaitReason, u64> {
    D30_WAIT_REASONS.into_iter().map(|reason| (reason, 0)).collect()
}

/// What one elementary interval is attributed to: the state, plus — when that
/// state is Waiting — the same hold named in both vocabularies. Carrying the
/// pair together is what keeps the two waiting partitions from drifting:
/// there is one classification and two renderings of it, never two
/// classifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Attribution {
    state: D30State,
    category: D30WaitCategory,
    reason: D30WaitReason,
}

impl Attribution {
    /// A non-waiting state. The waiting fields are inert (the sweep reads
    /// them only for `Waiting`), and set to the residuals so a future
    /// mis-read shows up as unattributed rather than as a false claim.
    fn state(state: D30State) -> Self {
        Self { state, category: D30WaitCategory::Unknown, reason: D30WaitReason::Unknown }
    }

    fn waiting(category: D30WaitCategory, reason: D30WaitReason) -> Self {
        Self { state: D30State::Waiting, category, reason }
    }
}

/// Sweep one robot's records over one bucket. `records` is the robot's FULL
/// sorted journal, not a bucket slice: no-leg gaps are classified from their
/// bounding legs, and a station dwell that crosses an hour boundary is still
/// Station processing in both hours even though its bounding legs lie outside
/// the bucket.
fn accumulate_robot_time(
    records: &[&HtmLegRecord],
    bucket: TimeBucket,
    states: &mut BTreeMap<D30State, u64>,
    categories: &mut BTreeMap<D30WaitCategory, u64>,
    waits: &mut BTreeMap<D30WaitReason, u64>,
) -> Result<(), ReportBuildError> {
    each_attributed_stretch(records, bucket, |start_ms, end_ms, attribution| {
        let duration_ms = end_ms - start_ms;
        add_duration(states, attribution.state, duration_ms)?;
        if attribution.state == D30State::Waiting {
            add_duration(categories, attribution.category, duration_ms)?;
            add_duration(waits, attribution.reason, duration_ms)?;
        }
        Ok(())
    })
}

/// Each stretch of one D30's time over `[start_ms, end_ms)` as this report
/// partitions it from `records`, the robot's whole journal: its state and, in
/// Waiting, its ratified category. A stretch ends wherever a fact begins or
/// ends, so two that touch may carry one attribution. The robot state
/// channel's parity test reads it; it goes with the interval rows.
#[cfg(any(test, feature = "test-support"))]
pub fn d30_stretches(
    records: &[HtmLegRecord],
    start_ms: u64,
    end_ms: u64,
) -> Vec<(u64, u64, D30State, Option<D30WaitCategory>)> {
    let mut sorted: Vec<&HtmLegRecord> = records.iter().collect();
    sorted.sort_by_key(|record| (record.start, record.duration));
    let mut stretches = Vec::new();
    let walked = each_attributed_stretch(&sorted, TimeBucket::complete(start_ms, end_ms), |start, end, attribution| {
        let category = (attribution.state == D30State::Waiting).then_some(attribution.category);
        stretches.push((start, end, attribution.state, category));
        Ok(())
    });
    walked.expect("collecting a stretch fails nothing");
    stretches
}

/// Hand `take` one robot's time over `bucket`, in order, cut wherever one of
/// its facts begins or ends, each stretch attributed once; the first error
/// from `take` ends the walk.
fn each_attributed_stretch(
    records: &[&HtmLegRecord],
    bucket: TimeBucket,
    mut take: impl FnMut(u64, u64, Attribution) -> Result<(), ReportBuildError>,
) -> Result<(), ReportBuildError> {
    let relevant: Vec<_> = records
        .iter()
        .copied()
        .filter(|record| {
            record.duration.as_ms() > 0
                && record.start.as_ms() < bucket.end_ms
                && record.start.as_ms().saturating_add(record.duration.as_ms()) > bucket.start_ms
        })
        .collect();
    let mut boundaries = vec![bucket.start_ms, bucket.end_ms];
    for record in &relevant {
        boundaries.push(record.start.as_ms().max(bucket.start_ms));
        boundaries.push(record.start.as_ms().saturating_add(record.duration.as_ms()).min(bucket.end_ms));
    }
    boundaries.sort_unstable();
    boundaries.dedup();

    for pair in boundaries.windows(2) {
        let start_ms = pair[0];
        let end_ms = pair[1];
        if start_ms == end_ms {
            continue;
        }
        let active: Vec<_> = relevant
            .iter()
            .copied()
            .filter(|record| {
                record.start.as_ms() < end_ms && record.start.as_ms().saturating_add(record.duration.as_ms()) > start_ms
            })
            .collect();
        let attribution =
            if active.is_empty() { classify_gap(records, start_ms, end_ms) } else { classify_active(&active) };
        take(start_ms, end_ms, attribution)?;
    }
    Ok(())
}

/// Classify a no-leg gap from its bounding legs. The bounding lookup runs
/// over the robot's full journal (an empty elementary interval overlaps no
/// record, so every duration-bearing record ends at or before it or starts
/// at or after it). Zero-duration rows (rotation-only nav events) are
/// excluded exactly as the sweep excludes them: a turn-only row stamped at
/// a station dwell's opening instant must not outrank the arriving leg and
/// reclassify the dwell.
fn classify_gap(records: &[&HtmLegRecord], gap_start_ms: u64, gap_end_ms: u64) -> Attribution {
    let prev = records
        .iter()
        .copied()
        .filter(|record| {
            record.duration.as_ms() > 0 && record.start.as_ms().saturating_add(record.duration.as_ms()) <= gap_start_ms
        })
        .max_by_key(|record| (record.start.as_ms().saturating_add(record.duration.as_ms()), record.start.as_ms()));
    let next = records
        .iter()
        .copied()
        .filter(|record| record.duration.as_ms() > 0 && record.start.as_ms() >= gap_end_ms)
        .min_by_key(|record| (record.start.as_ms(), record.start.as_ms().saturating_add(record.duration.as_ms())));
    if let (Some(prev), Some(next)) = (prev, next) {
        if let Some(station) = station_endpoint(prev.to) {
            if station_endpoint(next.from) == Some(station) {
                return Attribution::state(D30State::StationProcessing);
            }
        }
    }
    // No covering fact and no same-station bounds: honest unattributed
    // waiting, not a fabricated NoTask claim — D30s have explicit NoTask
    // facts, so an uncovered gap is genuinely unattributed.
    Attribution::waiting(D30WaitCategory::Unknown, D30WaitReason::Unknown)
}

/// The station identity of an endpoint, if it is one. Pick and putaway
/// stations with the same id are distinct endpoints.
fn station_endpoint(endpoint: HtmJournalEndpoint) -> Option<HtmJournalEndpoint> {
    match endpoint {
        HtmJournalEndpoint::PickStation(_)
        | HtmJournalEndpoint::PutawayStation(_)
        | HtmJournalEndpoint::ExtractionStation(_) => Some(endpoint),
        HtmJournalEndpoint::Parking | HtmJournalEndpoint::Relay(_) => None,
    }
}

fn classify_active(records: &[&HtmLegRecord]) -> Attribution {
    let classifications: Vec<_> = records.iter().map(|record| classify(record)).collect();
    let first = classifications[0];
    if classifications.iter().any(|other| other.state != first.state) {
        return Attribution::state(D30State::Unknown);
    }
    if first.state != D30State::Waiting {
        return first;
    }
    // Each vocabulary reports its OWN disagreement: overlapping holds can
    // differ in a distinction one vocabulary draws and the other does not,
    // and neither should be dragged into `ConflictingFacts` by the other's.
    let category = match classifications.iter().any(|other| other.category != first.category) {
        true => D30WaitCategory::ConflictingFacts,
        false => first.category,
    };
    let reason = match classifications.iter().any(|other| other.reason != first.reason) {
        true => D30WaitReason::ConflictingFacts,
        false => first.reason,
    };
    Attribution::waiting(category, reason)
}

fn classify(record: &HtmLegRecord) -> Attribution {
    match record.activity {
        Activity::Travel => Attribution::state(travel_state(record)),
        Activity::Wait => match record.hold {
            Some(hold) => hold_attribution(hold),
            // A row from a profile written before the enrichment: the cause
            // survives in the old two-name field, the position does not, so
            // the ratified vocabulary can only place what needs no position.
            None => match record.wait_reason {
                Some(HtmWaitReason::NoTask) => Attribution::waiting(D30WaitCategory::Unknown, D30WaitReason::NoTask),
                // An imported pre-split row carries only the old name, and
                // the resident/drove distinction is not recoverable from it.
                // It reads as drove — the population the split exists to
                // find — rather than being invented as resident.
                Some(HtmWaitReason::AwaitingDeposit) => {
                    Attribution::waiting(D30WaitCategory::AwaitingRelayDepositDrove, D30WaitReason::AwaitingDeposit)
                }
                None => Attribution::waiting(D30WaitCategory::Unknown, D30WaitReason::Unknown),
            },
        },
        // No live producer journals HTM Mechanical or Alignment rows (the
        // observer emits Travel and Wait only — verified against every
        // `htm_leg_log` push site). Should a foreign profile carry them,
        // they are unclassifiable time, not evidence for the Unsupported
        // endpoint-service states #275 owns.
        Activity::Mechanical | Activity::Alignment => Attribution::state(D30State::Unknown),
    }
}

/// Name one journaled hold in both vocabularies, from the task context and
/// the position the producer captured. This is the whole of #272's
/// attribution rule, and it is a total function of the two facts — nothing
/// here infers, and a fact that does not place the robot yields a residual
/// rather than a guess.
fn hold_attribution(hold: HtmHold) -> Attribution {
    use D30WaitCategory as Category;
    use D30WaitReason as Reason;
    match (hold.cause, hold.position) {
        (HtmWaitCause::NoTask, HtmWaitPosition::Charging | HtmWaitPosition::Relay) => {
            Attribution::waiting(Category::NoTaskParked, Reason::NoTask)
        }
        // The cause is known even when the position is not, so the old
        // vocabulary (which never looked at position) keeps its name and only
        // the ratified split degrades.
        (HtmWaitCause::NoTask, HtmWaitPosition::Unknown) => Attribution::waiting(Category::Unknown, Reason::NoTask),
        (HtmWaitCause::NoTask, _) => Attribution::waiting(Category::NoTaskElsewhere, Reason::NoTask),
        (HtmWaitCause::AwaitingDepositResident, _) => {
            Attribution::waiting(Category::AwaitingRelayDepositResident, Reason::AwaitingDeposit)
        }
        (HtmWaitCause::AwaitingDepositDrove, _) => {
            Attribution::waiting(Category::AwaitingRelayDepositDrove, Reason::AwaitingDeposit)
        }
        (HtmWaitCause::AwaitingSlot, _) => {
            Attribution::waiting(Category::AwaitingRelaySlot, Reason::AwaitingReturnSlot)
        }
        (HtmWaitCause::AwaitingStationService, HtmWaitPosition::StationQueue) => {
            Attribution::waiting(Category::InQueueStation, Reason::StationQueueHold)
        }
        // Presenting is the one category the old vocabulary cannot name.
        (HtmWaitCause::AwaitingStationService, HtmWaitPosition::Presentation) => {
            Attribution::waiting(Category::Presenting, Reason::Unknown)
        }
        // Waiting for station service somewhere neither cell kind covers:
        // both vocabularies name the position, and neither can.
        (HtmWaitCause::AwaitingStationService, _) => Attribution::waiting(Category::Unknown, Reason::Unknown),
        // The bounded post-dwell stand. The old vocabulary's own post-dwell
        // name is reserved for ruling #49's factless tail and carries
        // Unsupported support, so this projects to Unknown there — the same
        // degradation Presenting takes.
        (HtmWaitCause::AwaitingDeparture, _) => {
            Attribution::waiting(Category::AwaitingStationDeparture, Reason::Unknown)
        }
        // Unreachable for a positive interval: report construction rejects
        // this cause before classification under ruling #49. Keeping a total
        // mapping makes zero-duration/imported rows harmless without granting
        // the compatibility reason measured support.
        (HtmWaitCause::PostStationService, _) => {
            Attribution::waiting(Category::Unknown, Reason::PostPresentationReturnHold)
        }
        // A route hold at the presentation cell brackets the dwell on either
        // side: an inbound route can finish with one before Processing opens,
        // and an outbound route can open with one while the corridor out of
        // the station clears. Either way the station's presentation cell is
        // held by a D30 nobody is serving, which is Presenting. Ruling #49
        // reaches neither: it governs only the factless tail of an open-ended
        // reservation, and both of these are bounded by committed actions.
        (HtmWaitCause::RouteHold, HtmWaitPosition::Presentation) => {
            Attribution::waiting(Category::Presenting, Reason::Unknown)
        }
        // A queue cell is station-owned approach space. The live producer
        // exposes that wait as an in-path RouteHold, so position wins over the
        // generic route-hold Congestion label.
        (HtmWaitCause::RouteHold, HtmWaitPosition::StationQueue) => {
            Attribution::waiting(Category::InQueueStation, Reason::StationQueueHold)
        }
        (HtmWaitCause::RouteHold, _) => Attribution::waiting(Category::Congestion, Reason::ScheduledRouteHold),
    }
}

fn travel_state(record: &HtmLegRecord) -> D30State {
    // Travel-to-park takes precedence over payload classification (#261
    // §2.2), so a park leg is never also counted as empty travel.
    if record.to == HtmJournalEndpoint::Parking {
        D30State::TravelToPark
    } else if record.carrying_cargo {
        D30State::LadenTravel
    } else {
        D30State::EmptyTravel
    }
}

fn d30_state_support(state: D30State) -> SupportStatus {
    match state {
        // No duration-bearing endpoint-service facts until #275.
        D30State::Pickup | D30State::Drop | D30State::Alignment => SupportStatus::Unsupported,
        _ => SupportStatus::Measured,
    }
}

/// The compatibility post-dwell row is unsupported under ruling #49; every
/// other reason is attributable from the enriched hold journal.
fn d30_wait_support(reason: D30WaitReason) -> SupportStatus {
    match reason {
        D30WaitReason::PostPresentationReturnHold => SupportStatus::Unsupported,
        _ => SupportStatus::Measured,
    }
}

fn add_duration<K: Ord + Copy>(
    values: &mut BTreeMap<K, u64>,
    key: K,
    duration_ms: u64,
) -> Result<(), ReportBuildError> {
    let value = values.entry(key).or_default();
    *value = value.checked_add(duration_ms).ok_or(ReportBuildError::ArithmeticOverflow)?;
    Ok(())
}

/// Whether a Travel leg's distance belongs to `kind`. Exactly one kind
/// matches every Travel leg, so a per-kind split of any record set sums to
/// that set's total; the builder still re-checks this against the unfiltered
/// total per bucket ([`ReportBuildError::D30DistanceReconciliation`]).
fn travel_kind_matches(record: &HtmLegRecord, kind: D30TravelKind) -> bool {
    match travel_state(record) {
        D30State::TravelToPark => kind == D30TravelKind::Parking,
        D30State::LadenTravel => kind == D30TravelKind::Laden,
        _ => kind == D30TravelKind::Empty,
    }
}

/// Distance from Travel legs overlapping `bucket`, proportionally clipped;
/// `kind: None` is the unfiltered total the per-kind split must reconcile
/// to. Zero-duration rows cannot be proportionally split and are excluded
/// from BOTH the per-kind and total sums (symmetric, so the reconciliation
/// is unaffected) — mirroring the DA50 family, and honest: today's producer
/// stamps distance only on duration-bearing rows.
fn distance_for_bucket<'a>(
    records: impl IntoIterator<Item = &'a HtmLegRecord>,
    bucket: TimeBucket,
    kind: Option<D30TravelKind>,
) -> Result<u64, ReportBuildError> {
    records
        .into_iter()
        .filter(|record| {
            record.activity == Activity::Travel
                && record.duration.as_ms() > 0
                && kind.is_none_or(|kind| travel_kind_matches(record, kind))
        })
        .try_fold(0u64, |sum, record| {
            let leg_end_ms = record
                .start
                .as_ms()
                .checked_add(record.duration.as_ms())
                .ok_or(ReportBuildError::ArithmeticOverflow)?;
            let overlap_start_ms = record.start.as_ms().max(bucket.start_ms);
            let overlap_end_ms = leg_end_ms.min(bucket.end_ms);
            if overlap_start_ms >= overlap_end_ms {
                return Ok(sum);
            }
            let before_start = proportional_distance(
                record.distance_mm,
                overlap_start_ms - record.start.as_ms(),
                record.duration.as_ms(),
            )?;
            let before_end = proportional_distance(
                record.distance_mm,
                overlap_end_ms - record.start.as_ms(),
                record.duration.as_ms(),
            )?;
            sum.checked_add(before_end - before_start).ok_or(ReportBuildError::ArithmeticOverflow)
        })
}

fn proportional_distance(distance_mm: u64, elapsed_ms: u64, duration_ms: u64) -> Result<u64, ReportBuildError> {
    let value = (distance_mm as u128).checked_mul(elapsed_ms as u128).ok_or(ReportBuildError::ArithmeticOverflow)?
        / duration_ms as u128;
    u64::try_from(value).map_err(|_| ReportBuildError::ArithmeticOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::reporting::{FixedPeriod, MetricScope, ReportPeriod};
    use desgraph::{RelayId, StationId};

    fn leg(
        start_ms: u64,
        duration_ms: u64,
        activity: Activity,
        from: HtmJournalEndpoint,
        to: HtmJournalEndpoint,
        carrying_cargo: bool,
        distance_mm: u64,
        wait_reason: Option<HtmWaitReason>,
    ) -> HtmLegRecord {
        HtmLegRecord {
            start: SimTime::ms(start_ms),
            activity,
            from,
            to,
            from_zone_id: None,
            to_zone_id: None,
            distance_mm,
            duration: SimDuration::ms(duration_ms),
            presentation_id: Some(1),
            robot_id: RobotId(1),
            carrying_cargo,
            turn_count: 0,
            wait_reason,
            hold: None,
            notes: String::new(),
        }
    }

    /// A journaled hold, as the live observer writes one: a stationary `Wait`
    /// row at a single endpoint carrying both #272 facts.
    fn hold_row(
        start_ms: u64,
        duration_ms: u64,
        at: HtmJournalEndpoint,
        cause: HtmWaitCause,
        position: HtmWaitPosition,
    ) -> HtmLegRecord {
        HtmLegRecord {
            hold: Some(HtmHold { cause, position }),
            ..leg(start_ms, duration_ms, Activity::Wait, at, at, false, 0, None)
        }
    }

    fn category_duration(report: &MetricsReport<D30StateData>, category: D30WaitCategory) -> u64 {
        report
            .data
            .wait_category_rows
            .iter()
            .find(|row| row.category == category)
            .and_then(|row| row.duration_ms)
            .unwrap()
    }

    fn state_duration(report: &MetricsReport<D30StateData>, state: D30State) -> u64 {
        report.data.state_rows.iter().find(|row| row.state == state).and_then(|row| row.duration_ms).unwrap()
    }

    fn wait_duration(report: &MetricsReport<D30StateData>, reason: D30WaitReason) -> u64 {
        report.data.wait_rows.iter().find(|row| row.reason == reason).and_then(|row| row.duration_ms).unwrap()
    }

    fn distance(report: &MetricsReport<D30StateData>, kind: D30TravelKind) -> u64 {
        report.data.distance_rows.iter().find(|row| row.kind == kind).and_then(|row| row.distance_mm).unwrap()
    }

    const RELAY: HtmJournalEndpoint = HtmJournalEndpoint::Relay(RelayId(2));
    const STATION: HtmJournalEndpoint = HtmJournalEndpoint::PickStation(StationId(4));

    /// One full presentation cycle as the live journal writes it: empty
    /// travel to the relay, AwaitingDeposit dwell, laden travel to the
    /// station, an un-facted station dwell, empty return travel, an explicit
    /// NoTask idle, and a park leg. Every state, reason, and distance kind
    /// reconciles to the 100 ms of fleet time.
    #[test]
    fn states_wait_reasons_and_distances_reconcile_to_fleet_time() {
        let records = vec![
            // Empty travel Parking → Relay: [0, 10).
            leg(0, 10, Activity::Travel, HtmJournalEndpoint::Parking, RELAY, false, 100, None),
            // AwaitingDeposit dwell at the relay: [10, 25).
            leg(10, 15, Activity::Wait, RELAY, RELAY, false, 0, Some(HtmWaitReason::AwaitingDeposit)),
            // Laden travel Relay → PickStation: [25, 45).
            leg(25, 20, Activity::Travel, RELAY, STATION, true, 200, None),
            // No fact covers [45, 60): bounded by legs at the same station
            // endpoint on both sides → StationProcessing.
            // Empty return PickStation → Relay: [60, 70).
            leg(60, 10, Activity::Travel, STATION, RELAY, false, 150, None),
            // Explicit NoTask idle at the relay: [70, 90).
            leg(70, 20, Activity::Wait, RELAY, RELAY, false, 0, Some(HtmWaitReason::NoTask)),
            // Park leg while (synthetically) laden: TravelToPark wins over
            // payload for state and distance alike: [90, 100).
            leg(90, 10, Activity::Travel, RELAY, HtmJournalEndpoint::Parking, true, 50, None),
        ];
        let report =
            d30_state_report(&records, 1, ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 100, 0))
                .unwrap();

        assert_eq!(state_duration(&report, D30State::EmptyTravel), 20);
        assert_eq!(state_duration(&report, D30State::LadenTravel), 20);
        assert_eq!(state_duration(&report, D30State::TravelToPark), 10);
        assert_eq!(state_duration(&report, D30State::StationProcessing), 15);
        assert_eq!(state_duration(&report, D30State::Waiting), 35);
        assert_eq!(state_duration(&report, D30State::Unknown), 0);
        assert_eq!(wait_duration(&report, D30WaitReason::AwaitingDeposit), 15);
        assert_eq!(wait_duration(&report, D30WaitReason::NoTask), 20);
        assert_eq!(wait_duration(&report, D30WaitReason::Unknown), 0);
        assert_eq!(
            report.data.state_rows.iter().filter_map(|row| row.duration_ms).sum::<u64>(),
            100,
            "measured states partition fleet time"
        );

        assert_eq!(distance(&report, D30TravelKind::Empty), 250);
        assert_eq!(distance(&report, D30TravelKind::Laden), 200);
        assert_eq!(distance(&report, D30TravelKind::Parking), 50);
        // Literal, not derived from `records`: a derived expectation would
        // inflate in lockstep with any double-counting defect.
        assert_eq!(
            distance(&report, D30TravelKind::Empty)
                + distance(&report, D30TravelKind::Laden)
                + distance(&report, D30TravelKind::Parking),
            500,
            "distance kinds reconcile to total classified navigation distance"
        );
    }

    /// Only a gap bounded by legs at the SAME station endpoint is station
    /// processing: relay-side gaps (fork seams, rotation) and gaps between
    /// different stations stay Waiting → Unknown, and a pick vs putaway
    /// station with the same id is a different endpoint.
    #[test]
    fn station_processing_requires_same_station_bounds() {
        let cases = [
            // Relay-bounded gap: Waiting → Unknown.
            (RELAY, RELAY, D30State::Waiting),
            // Same pick station: StationProcessing.
            (STATION, STATION, D30State::StationProcessing),
            // Different station ids: Waiting → Unknown.
            (STATION, HtmJournalEndpoint::PickStation(StationId(5)), D30State::Waiting),
            // Same id, different station kind: Waiting → Unknown.
            (STATION, HtmJournalEndpoint::PutawayStation(StationId(4)), D30State::Waiting),
        ];
        for (arrive_at, depart_from, expected) in cases {
            let records = vec![
                leg(0, 10, Activity::Travel, RELAY, arrive_at, true, 10, None),
                // Gap [10, 30).
                leg(30, 10, Activity::Travel, depart_from, RELAY, false, 10, None),
            ];
            let report = d30_state_report(
                &records,
                1,
                ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 40, 0),
            )
            .unwrap();
            assert_eq!(state_duration(&report, expected), 20, "gap bounded by {arrive_at:?} → {depart_from:?}");
            if expected == D30State::Waiting {
                assert_eq!(wait_duration(&report, D30WaitReason::Unknown), 20);
            }
        }
    }

    /// A station dwell that crosses an hour boundary stays StationProcessing
    /// in BOTH hourly buckets, even though its bounding legs lie entirely
    /// outside the second bucket (#272's hour-crossing acceptance case).
    #[test]
    fn station_processing_survives_hour_crossing() {
        let hour = 3_600_000;
        let records = vec![
            leg(hour - 20, 10, Activity::Travel, RELAY, STATION, true, 10, None),
            // Dwell [hour - 10, hour + 10) spans the boundary.
            leg(hour + 10, 10, Activity::Travel, STATION, RELAY, false, 10, None),
        ];
        let report = d30_state_report(
            &records,
            1,
            ReportRequest::for_run(
                MetricScope::FullRun,
                ReportPeriod::Fixed(FixedPeriod::new(hour, 0).unwrap()),
                hour + 20,
                0,
            ),
        )
        .unwrap();

        let processing: Vec<_> = report
            .data
            .state_rows
            .iter()
            .filter(|row| row.state == D30State::StationProcessing)
            .map(|row| row.duration_ms.unwrap())
            .collect();
        assert_eq!(processing, [10, 10], "10 ms of the dwell in each hour");
    }

    /// TravelToPark wins over payload classification: a laden park leg is
    /// neither laden travel nor laden distance.
    #[test]
    fn travel_to_park_takes_precedence_over_payload() {
        let records = [leg(0, 10, Activity::Travel, RELAY, HtmJournalEndpoint::Parking, true, 80, None)];
        let report =
            d30_state_report(&records, 1, ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 10, 0))
                .unwrap();

        assert_eq!(state_duration(&report, D30State::TravelToPark), 10);
        assert_eq!(state_duration(&report, D30State::LadenTravel), 0);
        assert_eq!(distance(&report, D30TravelKind::Parking), 80);
        assert_eq!(distance(&report, D30TravelKind::Laden), 0);
    }

    /// No records: modeled states report measured (truthful) zeros, while the
    /// endpoint-service states #275 owns carry their honest status and no
    /// value. Every legitimate wait reason is measurable since #272; the
    /// compatibility-only post-dwell station row is unsupported by ruling
    /// #49 and carries no value.
    #[test]
    fn unmeasurable_states_are_never_measured_zeros() {
        let report =
            d30_state_report(&[], 2, ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 10, 0))
                .unwrap();

        for state in [D30State::Pickup, D30State::Drop, D30State::Alignment] {
            assert!(report.data.state_rows.iter().any(|row| {
                row.state == state && row.status == SupportStatus::Unsupported && row.duration_ms.is_none()
            }));
        }
        assert!(report.data.state_rows.iter().any(|row| {
            row.state == D30State::StationProcessing
                && row.status == SupportStatus::Measured
                && row.duration_ms == Some(0)
        }));
        for reason in D30_WAIT_REASONS.into_iter().filter(|reason| *reason != D30WaitReason::PostPresentationReturnHold)
        {
            assert!(
                report
                    .data
                    .wait_rows
                    .iter()
                    .any(|row| { row.reason == reason && row.status == SupportStatus::Measured }),
                "{reason:?} is attributable from journal holds"
            );
        }
        assert!(report.data.wait_rows.iter().any(|row| {
            row.reason == D30WaitReason::PostPresentationReturnHold
                && row.status == SupportStatus::Unsupported
                && row.duration_ms.is_none()
        }));
        // Both unobserved D30s backfill into Waiting. The old vocabulary calls
        // that no task; the ratified one has no fact to read at all, so the
        // time lands in No facts rather than being called parked, or being
        // mixed in with the bounded gaps Unknown covers.
        assert_eq!(state_duration(&report, D30State::Waiting), 20);
        assert_eq!(wait_duration(&report, D30WaitReason::NoTask), 20);
        assert_eq!(category_duration(&report, D30WaitCategory::NoFacts), 20);
        assert_eq!(category_duration(&report, D30WaitCategory::Unknown), 0);
        assert_eq!(category_duration(&report, D30WaitCategory::NoTaskParked), 0);
        assert!(report.data.vehicle_state_rows.is_empty());
    }

    /// Every ratified #272 category, pinned from the journal facts the
    /// producer writes for it. One D30, one hold per category, back to back.
    #[test]
    fn every_ratified_category_is_attributed_from_its_hold_facts() {
        use HtmWaitCause as Cause;
        use HtmWaitPosition as Position;
        let cases = [
            (Cause::NoTask, Position::Charging, D30WaitCategory::NoTaskParked, D30WaitReason::NoTask),
            (Cause::NoTask, Position::Route, D30WaitCategory::NoTaskElsewhere, D30WaitReason::NoTask),
            (
                Cause::AwaitingStationService,
                Position::Presentation,
                D30WaitCategory::Presenting,
                // The one category the old vocabulary has no name for.
                D30WaitReason::Unknown,
            ),
            (
                Cause::AwaitingStationService,
                Position::StationQueue,
                D30WaitCategory::InQueueStation,
                D30WaitReason::StationQueueHold,
            ),
            (Cause::AwaitingStationService, Position::Route, D30WaitCategory::Unknown, D30WaitReason::Unknown),
            (Cause::RouteHold, Position::Presentation, D30WaitCategory::Presenting, D30WaitReason::Unknown),
            (
                Cause::AwaitingDepositResident,
                Position::Relay,
                D30WaitCategory::AwaitingRelayDepositResident,
                D30WaitReason::AwaitingDeposit,
            ),
            (
                Cause::AwaitingDepositDrove,
                Position::Relay,
                D30WaitCategory::AwaitingRelayDepositDrove,
                D30WaitReason::AwaitingDeposit,
            ),
            (
                Cause::AwaitingSlot,
                Position::Relay,
                D30WaitCategory::AwaitingRelaySlot,
                D30WaitReason::AwaitingReturnSlot,
            ),
            (
                Cause::RouteHold,
                Position::StationQueue,
                D30WaitCategory::InQueueStation,
                D30WaitReason::StationQueueHold,
            ),
            (Cause::RouteHold, Position::Route, D30WaitCategory::Congestion, D30WaitReason::ScheduledRouteHold),
            // A hold the position facts did not place: the cause still names
            // the old reason, the ratified split cannot be made.
            (Cause::NoTask, Position::Unknown, D30WaitCategory::Unknown, D30WaitReason::NoTask),
        ];
        for (cause, position, category, reason) in cases {
            // One hold covering the whole run, so nothing but it is being
            // classified.
            let records = [hold_row(0, 10, RELAY, cause, position)];
            let report = d30_state_report(
                &records,
                1,
                ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 10, 0),
            )
            .unwrap();
            assert_eq!(category_duration(&report, category), 10, "{cause:?} at {position:?} is {category:?}");
            assert_eq!(
                wait_duration(&report, reason),
                10,
                "{cause:?} at {position:?} reads as {reason:?} in the old vocabulary"
            );
        }
    }

    /// The two fact-poor residuals are different fact situations and stay
    /// apart in one report: the observed D30's hold is a real fact that names
    /// no position, so its time is a gap the facts under-determine (Unknown),
    /// while the configured D30 the run never journaled contributes no fact at
    /// all (No facts). Both are Waiting, and the two together still partition
    /// it exactly.
    #[test]
    fn an_unjournaled_robot_is_no_facts_and_a_placeless_hold_is_unknown() {
        let records = [hold_row(0, 10, RELAY, HtmWaitCause::AwaitingStationService, HtmWaitPosition::Unknown)];
        let report =
            d30_state_report(&records, 2, ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 10, 0))
                .unwrap();

        assert_eq!(state_duration(&report, D30State::Waiting), 20);
        assert_eq!(category_duration(&report, D30WaitCategory::Unknown), 10);
        assert_eq!(category_duration(&report, D30WaitCategory::NoFacts), 10);
        let categories: u64 =
            D30_WAIT_CATEGORIES.into_iter().map(|category| category_duration(&report, category)).sum();
        assert_eq!(categories, 20, "the categories partition Waiting exactly");
    }

    /// The compatibility-only post-service mapping remains explicit even
    /// though every positive live row is rejected before attribution.
    #[test]
    fn post_station_service_has_only_the_unsupported_compatibility_name() {
        assert_eq!(
            hold_attribution(HtmHold {
                cause: HtmWaitCause::PostStationService,
                position: HtmWaitPosition::Presentation,
            }),
            Attribution::waiting(D30WaitCategory::Unknown, D30WaitReason::PostPresentationReturnHold,)
        );
    }

    /// Daniel's #272 ruling: station processing is the recorded operator
    /// dwell and nothing else. With the producer journaling the pre-dwell
    /// hold, the gap rule finds exactly the dwell before return travel begins.
    #[test]
    fn station_processing_shrinks_to_the_recorded_dwell() {
        let records = vec![
            // Laden arrival at the station: [0, 10).
            leg(0, 10, Activity::Travel, RELAY, STATION, true, 100, None),
            // Held at the presentation cell before the dwell: [10, 15).
            hold_row(10, 5, STATION, HtmWaitCause::AwaitingStationService, HtmWaitPosition::Presentation),
            // The recorded dwell [15, 35) is the one stretch no row covers.
            // Return begins exactly when that dwell ends.
            leg(35, 10, Activity::Travel, STATION, RELAY, false, 100, None),
        ];
        let report =
            d30_state_report(&records, 1, ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 45, 0))
                .unwrap();

        assert_eq!(state_duration(&report, D30State::StationProcessing), 20, "the dwell only");
        assert_eq!(state_duration(&report, D30State::Waiting), 5);
        assert_eq!(category_duration(&report, D30WaitCategory::Presenting), 5);
        assert_eq!(wait_duration(&report, D30WaitReason::Unknown), 5, "Presenting has no old name");
        assert_eq!(category_duration(&report, D30WaitCategory::Unknown), 0);
        assert!(report.data.wait_rows.iter().any(|row| {
            row.reason == D30WaitReason::PostPresentationReturnHold
                && row.status == SupportStatus::Unsupported
                && row.duration_ms.is_none()
        }));
    }

    /// Ruling #49: an open-ended reservation is a wall-clock scheduling
    /// bookmark, never simulation-time occupancy past both the current world
    /// time and the dwell's end. A row carrying one is a hard integrity
    /// tripwire, never booked to a waiting residual.
    #[test]
    fn a_post_dwell_station_interval_fails_hard() {
        let record = hold_row(35, 5, STATION, HtmWaitCause::PostStationService, HtmWaitPosition::Presentation);
        let error =
            d30_state_report(&[record], 1, ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 40, 0))
                .unwrap_err();
        assert_eq!(
            error,
            ReportBuildError::D30PostDwellStationHold {
                robot_id: RobotId(1),
                start: SimTime::ms(35),
                end: SimTime::ms(40),
            }
        );
    }

    /// The sibling of the tripwire above, and the reason the two need
    /// separate names: a post-dwell stand bounded by the store leg's admitted
    /// departure, or by the current world time when the run stopped first, is
    /// real occupancy, so it must pass where the factless tail fails. Both are
    /// rows from the dwell's end on the presentation cell; only what bounds
    /// them tells them apart.
    ///
    /// The row covers `[35, 40)` and books exactly its own 5 ms. The 35 ms
    /// before it is an uncovered lead-in with no preceding record, so
    /// `classify_gap` finds no bounding pair and books it Unknown — also a
    /// Waiting state, which is why the window's Waiting total is the whole
    /// 40 and not the row's 5.
    #[test]
    fn a_bounded_post_dwell_stand_is_occupancy_not_a_tripwire() {
        let record = hold_row(35, 5, STATION, HtmWaitCause::AwaitingDeparture, HtmWaitPosition::Presentation);
        let report =
            d30_state_report(&[record], 1, ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 40, 0))
                .expect("a departure-bounded stand is ordinary occupancy");
        assert_eq!(category_duration(&report, D30WaitCategory::AwaitingStationDeparture), 5, "the row's own time");
        assert_eq!(category_duration(&report, D30WaitCategory::Unknown), 35, "the uncovered lead-in, not the row's");
        assert_eq!(state_duration(&report, D30State::Waiting), 40, "both are Waiting, and together they partition it");
    }

    /// The hold at the head of a return leg is the path search waiting out the
    /// corridor, bounded by the D30's own committed actions — real occupancy,
    /// and Presenting because the station's presentation cell is the thing it
    /// is holding. Ruling #49 is about station time past both the current
    /// world time and the dwell's end, and does not reach this.
    #[test]
    fn an_initial_outbound_hold_at_presentation_is_presenting() {
        let record = HtmLegRecord {
            from: STATION,
            to: RELAY,
            hold: Some(HtmHold { cause: HtmWaitCause::RouteHold, position: HtmWaitPosition::Presentation }),
            ..leg(35, 5, Activity::Wait, STATION, RELAY, true, 0, None)
        };
        let report =
            d30_state_report(&[record], 1, ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 40, 0))
                .unwrap();

        assert_eq!(category_duration(&report, D30WaitCategory::Presenting), 5);
        assert_eq!(state_duration(&report, D30State::StationProcessing), 0);
    }

    /// A hold journaled partway along a leg is Congestion, and it no longer
    /// leaves an unattributed gap: the producer splits the leg's Travel rows
    /// at the hold, so travel, hold, travel cover the chain end to end.
    #[test]
    fn an_in_path_hold_is_congestion_and_leaves_no_gap() {
        let records = vec![
            leg(0, 10, Activity::Travel, RELAY, STATION, true, 100, None),
            hold_row(10, 20, RELAY, HtmWaitCause::RouteHold, HtmWaitPosition::Route),
            leg(30, 10, Activity::Travel, RELAY, STATION, true, 100, None),
        ];
        let report =
            d30_state_report(&records, 1, ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 40, 0))
                .unwrap();

        assert_eq!(state_duration(&report, D30State::LadenTravel), 20);
        assert_eq!(category_duration(&report, D30WaitCategory::Congestion), 20);
        assert_eq!(wait_duration(&report, D30WaitReason::ScheduledRouteHold), 20);
        assert_eq!(wait_duration(&report, D30WaitReason::Unknown), 0);
    }

    /// A recording written before the enrichment has wait rows with no
    /// hold facts. The cause survives in the old field, so the old vocabulary
    /// keeps naming it; the ratified split needs a position and honestly
    /// reports that it has none.
    #[test]
    fn pre_enrichment_rows_keep_their_old_name_and_no_ratified_split() {
        let records = [
            leg(0, 10, Activity::Wait, RELAY, RELAY, false, 0, Some(HtmWaitReason::NoTask)),
            leg(10, 10, Activity::Wait, RELAY, RELAY, false, 0, Some(HtmWaitReason::AwaitingDeposit)),
        ];
        let report =
            d30_state_report(&records, 1, ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 20, 0))
                .unwrap();

        assert_eq!(wait_duration(&report, D30WaitReason::NoTask), 10);
        assert_eq!(category_duration(&report, D30WaitCategory::Unknown), 10);
        // Awaiting deposit needs no position, so it survives the round trip
        // — as `drove`, since a pre-split row cannot say it was resident.
        assert_eq!(category_duration(&report, D30WaitCategory::AwaitingRelayDepositDrove), 10);
    }

    /// Both vocabularies are emitted per bucket, so an hourly timeline can
    /// break its Waiting band down instead of drawing one merged segment —
    /// and each hour's categories partition that hour's Waiting exactly.
    #[test]
    fn wait_categories_are_emitted_per_bucket() {
        let hour = 3_600_000;
        let records = vec![
            hold_row(0, hour, RELAY, HtmWaitCause::RouteHold, HtmWaitPosition::Route),
            hold_row(hour, hour, RELAY, HtmWaitCause::NoTask, HtmWaitPosition::Charging),
        ];
        let report = d30_state_report(
            &records,
            1,
            ReportRequest::for_run(
                MetricScope::FullRun,
                ReportPeriod::Fixed(FixedPeriod::new(hour, 0).unwrap()),
                2 * hour,
                0,
            ),
        )
        .unwrap();

        let per_hour = |category: D30WaitCategory| -> Vec<u64> {
            report
                .data
                .wait_category_rows
                .iter()
                .filter(|row| row.category == category)
                .map(|row| row.duration_ms.unwrap())
                .collect()
        };
        assert_eq!(per_hour(D30WaitCategory::Congestion), [hour, 0]);
        assert_eq!(per_hour(D30WaitCategory::NoTaskParked), [0, hour]);
        for bucket_index in 0..2 {
            let bucket_ms: u64 = report
                .data
                .wait_category_rows
                .iter()
                .filter(|row| row.bucket.start_ms == bucket_index * hour)
                .filter_map(|row| row.duration_ms)
                .sum();
            assert_eq!(bucket_ms, hour, "hour {bucket_index} categories partition its waiting");
        }
    }

    /// The fleet denominator is the configured eligible count: facts naming
    /// more distinct D30s than configured are an integrity breach, not an
    /// invitation to silently grow the denominator.
    #[test]
    fn observed_robots_beyond_configured_fleet_fail_hard() {
        let first = leg(0, 10, Activity::Travel, HtmJournalEndpoint::Parking, RELAY, false, 10, None);
        let mut second = first.clone();
        second.robot_id = RobotId(2);
        let error = d30_state_report(
            &[first, second],
            1,
            ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 20, 0),
        )
        .unwrap_err();
        assert_eq!(error, ReportBuildError::D30FleetExceeded { configured: 1, observed: 2 });
    }

    /// Overlapping same-robot travel facts would double-count distance (the
    /// state sweep unions them, distance sums per record) — hard error, and
    /// an exact duplicate is the degenerate case.
    #[test]
    fn overlapping_travel_facts_fail_hard() {
        let first = leg(0, 10, Activity::Travel, RELAY, HtmJournalEndpoint::Parking, true, 80, None);
        let second = first.clone();
        let error = d30_state_report(
            &[first, second],
            1,
            ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 10, 0),
        )
        .unwrap_err();
        assert_eq!(error, ReportBuildError::D30TravelOverlap { robot_id: 1, first_start_ms: 0, second_start_ms: 0 });

        // Wait rows may legitimately overlap travel (the state sweep owns
        // that disagreement); only travel↔travel overlap is rejected.
        let travel = leg(0, 10, Activity::Travel, RELAY, STATION, false, 10, None);
        let wait = leg(5, 10, Activity::Wait, RELAY, RELAY, false, 0, Some(HtmWaitReason::NoTask));
        assert!(
            d30_state_report(
                &[travel, wait],
                1,
                ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 15, 0),
            )
            .is_ok()
        );
    }

    /// A rotation-only nav event journals a zero-duration Travel row stamped
    /// at its event time. One sitting exactly at a station dwell's opening
    /// instant (its leg context already pointing at the NEXT destination)
    /// must not outrank the arriving leg in the gap-bounding lookup and
    /// reclassify the dwell to Waiting → Unknown.
    #[test]
    fn zero_duration_rotation_row_does_not_break_station_dwell() {
        let records = vec![
            leg(0, 10, Activity::Travel, RELAY, STATION, true, 10, None),
            // Turn-only row of the departing leg at the dwell's opening
            // instant: duration 0, from the station toward the relay.
            leg(10, 0, Activity::Travel, STATION, RELAY, false, 0, None),
            // Dwell [10, 30), then the departing move.
            leg(30, 10, Activity::Travel, STATION, RELAY, false, 10, None),
        ];
        let report =
            d30_state_report(&records, 1, ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 40, 0))
                .unwrap();

        assert_eq!(state_duration(&report, D30State::StationProcessing), 20);
        assert_eq!(wait_duration(&report, D30WaitReason::Unknown), 0);
    }

    /// Overlapping facts that disagree on state are Unknown instead of being
    /// double-counted; overlapping wait facts that disagree on reason are
    /// Waiting → ConflictingFacts.
    #[test]
    fn conflicting_overlap_is_unknown_instead_of_double_counted() {
        let records = [
            leg(0, 10, Activity::Travel, RELAY, STATION, false, 10, None),
            leg(5, 10, Activity::Wait, RELAY, RELAY, false, 0, Some(HtmWaitReason::NoTask)),
        ];
        let report =
            d30_state_report(&records, 1, ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 15, 0))
                .unwrap();
        assert_eq!(state_duration(&report, D30State::EmptyTravel), 5);
        assert_eq!(state_duration(&report, D30State::Unknown), 5);
        assert_eq!(state_duration(&report, D30State::Waiting), 5);

        let records = [
            leg(0, 10, Activity::Wait, RELAY, RELAY, false, 0, Some(HtmWaitReason::NoTask)),
            leg(5, 10, Activity::Wait, RELAY, RELAY, false, 0, Some(HtmWaitReason::AwaitingDeposit)),
        ];
        let report =
            d30_state_report(&records, 1, ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 15, 0))
                .unwrap();
        assert_eq!(state_duration(&report, D30State::Waiting), 15);
        assert_eq!(wait_duration(&report, D30WaitReason::ConflictingFacts), 5);
        assert_eq!(category_duration(&report, D30WaitCategory::ConflictingFacts), 5);

        // Holds agreeing on cause but disagreeing on position conflict only in
        // the vocabulary that draws that distinction: one interval cannot be
        // both parked and elsewhere, but both rows still say "no task".
        let records = [
            hold_row(0, 10, RELAY, HtmWaitCause::NoTask, HtmWaitPosition::Charging),
            hold_row(5, 10, RELAY, HtmWaitCause::NoTask, HtmWaitPosition::Route),
        ];
        let report =
            d30_state_report(&records, 1, ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 15, 0))
                .unwrap();
        assert_eq!(category_duration(&report, D30WaitCategory::ConflictingFacts), 5);
        assert_eq!(wait_duration(&report, D30WaitReason::ConflictingFacts), 0);
        assert_eq!(wait_duration(&report, D30WaitReason::NoTask), 15);
    }

    /// No live producer journals HTM Mechanical/Alignment rows; a foreign
    /// profile's row lands in Unknown rather than fabricating a measurement
    /// for the Unsupported endpoint-service states #275 owns.
    #[test]
    fn foreign_mechanical_rows_land_in_unknown_not_pickup() {
        let records = [leg(0, 10, Activity::Mechanical, RELAY, RELAY, false, 0, None)];
        let report =
            d30_state_report(&records, 1, ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 10, 0))
                .unwrap();

        assert_eq!(state_duration(&report, D30State::Unknown), 10);
        assert!(report.data.state_rows.iter().find(|row| row.state == D30State::Pickup).unwrap().duration_ms.is_none());
    }

    #[test]
    fn fixed_buckets_split_duration_and_distance_without_rounding_drift() {
        let records = [leg(5, 10, Activity::Travel, HtmJournalEndpoint::Parking, RELAY, false, 101, None)];
        let report = d30_state_report(
            &records,
            1,
            ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Fixed(FixedPeriod::new(10, 0).unwrap()), 20, 0),
        )
        .unwrap();

        let empty_distances: Vec<_> = report
            .data
            .distance_rows
            .iter()
            .filter(|row| row.kind == D30TravelKind::Empty)
            .map(|row| row.distance_mm.unwrap())
            .collect();
        assert_eq!(empty_distances, [50, 51]);
        assert_eq!(empty_distances.iter().sum::<u64>(), 101);
        let empty_travel: Vec<_> = report
            .data
            .state_rows
            .iter()
            .filter(|row| row.state == D30State::EmptyTravel)
            .map(|row| row.duration_ms.unwrap())
            .collect();
        assert_eq!(empty_travel, [5, 5]);
    }

    #[test]
    fn taxonomy_arrays_are_exhaustive() {
        // A new enum variant must break these matches, forcing the canonical
        // ordering arrays to be updated with it.
        for state in D30_STATES {
            match state {
                D30State::Pickup
                | D30State::Drop
                | D30State::Alignment
                | D30State::EmptyTravel
                | D30State::LadenTravel
                | D30State::TravelToPark
                | D30State::StationProcessing
                | D30State::Waiting
                | D30State::Unknown => {}
            }
        }
        for reason in D30_WAIT_REASONS {
            match reason {
                D30WaitReason::NoTask
                | D30WaitReason::AwaitingDeposit
                | D30WaitReason::ScheduledRouteHold
                | D30WaitReason::StationQueueHold
                | D30WaitReason::PostPresentationReturnHold
                | D30WaitReason::AwaitingReturnSlot
                | D30WaitReason::Unknown
                | D30WaitReason::ConflictingFacts => {}
            }
        }
        for category in D30_WAIT_CATEGORIES {
            match category {
                D30WaitCategory::NoTaskParked
                | D30WaitCategory::NoTaskElsewhere
                | D30WaitCategory::Presenting
                | D30WaitCategory::AwaitingStationDeparture
                | D30WaitCategory::AwaitingRelayDepositResident
                | D30WaitCategory::AwaitingRelayDepositDrove
                | D30WaitCategory::AwaitingRelaySlot
                | D30WaitCategory::InQueueStation
                | D30WaitCategory::Congestion
                | D30WaitCategory::Unknown
                | D30WaitCategory::NoFacts
                | D30WaitCategory::ConflictingFacts => {}
                D30WaitCategory::AwaitingDepartureElsewhere => {
                    panic!("the journal files that stand as no task, so the report lists no row for it")
                }
            }
        }
        for kind in D30_TRAVEL_KINDS {
            match kind {
                D30TravelKind::Empty | D30TravelKind::Laden | D30TravelKind::Parking => {}
            }
        }
    }

    #[test]
    fn per_vehicle_rows_reconcile_to_fleet_rows() {
        let first = leg(0, 10, Activity::Travel, HtmJournalEndpoint::Parking, RELAY, false, 100, None);
        let mut second = first.clone();
        second.robot_id = RobotId(2);
        second.carrying_cargo = true;
        second.to = STATION;
        second.distance_mm = 250;
        // Fleet of 3 with 2 observed: fleet rows carry one unobserved
        // vehicle's Waiting/NoTask backfill on top of the vehicle sums.
        let report = d30_state_report(
            &[first, second],
            3,
            ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 20, 0),
        )
        .unwrap();

        let unobserved_backfill_ms = 20;
        assert_eq!(
            report.data.vehicle_state_rows.len(),
            2 * D30_STATES.len(),
            "one row per observed vehicle per state"
        );
        for state in D30_STATES {
            let fleet = report.data.state_rows.iter().find(|row| row.state == state).unwrap();
            let vehicle_sum: u64 = report
                .data
                .vehicle_state_rows
                .iter()
                .filter(|row| row.state == state)
                .filter_map(|row| row.duration_ms)
                .sum();
            let backfill = if state == D30State::Waiting { unobserved_backfill_ms } else { 0 };
            match fleet.status {
                SupportStatus::Measured => {
                    assert_eq!(fleet.duration_ms, Some(vehicle_sum + backfill))
                }
                _ => assert_eq!(vehicle_sum, 0, "unmeasured states carry no vehicle time"),
            }
        }
        for reason in D30_WAIT_REASONS {
            let fleet = report.data.wait_rows.iter().find(|row| row.reason == reason).unwrap();
            let vehicle_sum: u64 = report
                .data
                .vehicle_wait_rows
                .iter()
                .filter(|row| row.reason == reason)
                .filter_map(|row| row.duration_ms)
                .sum();
            let backfill = if reason == D30WaitReason::NoTask { unobserved_backfill_ms } else { 0 };
            match fleet.status {
                SupportStatus::Measured => {
                    assert_eq!(fleet.duration_ms, Some(vehicle_sum + backfill))
                }
                _ => assert_eq!(vehicle_sum, 0, "unsupported reasons carry no vehicle time"),
            }
        }
        for kind in D30_TRAVEL_KINDS {
            let fleet =
                report.data.distance_rows.iter().find(|row| row.kind == kind).and_then(|row| row.distance_mm).unwrap();
            let vehicle_sum: u64 = report
                .data
                .vehicle_distance_rows
                .iter()
                .filter(|row| row.kind == kind)
                .filter_map(|row| row.distance_mm)
                .sum();
            assert_eq!(fleet, vehicle_sum, "unobserved vehicles travel nowhere");
        }
    }
}
