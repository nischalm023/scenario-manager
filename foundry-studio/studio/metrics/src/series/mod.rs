//! Within `studio-metrics`, `series` is THE SEVENTY-FOUR TIME SERIES of desvis's Time series tab
//! (`desvis/src/types/metrics.rs`), named once here, each with the stable name a viewer attaches by, the label desvis
//! shows, what it measures and how its values combine. `slices` computes each slice's value of every series, `trailing`
//! folds a series' slices into windows, and `view` cuts one series as an ask asks. A metric's name never changes once a
//! page has attached by it.

mod slices;
mod trailing;
mod view;

use desunits::SimDuration;
use studio_protocol::{Aggregation, Bucket, DurationClock, MetricDescriptor, MetricSource, MetricUnit, TrailingSpec};

use slices::SLICE_WALL_CLOCK;
pub(crate) use slices::Slices;
pub(crate) use slices::World;
pub(crate) use view::SeriesView;

/// One of desvis's time series, in its order there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimeSeries {
    PicksPerSlice,
    PicksPerHour,
    CumulativePicks,
    PutawaysPerSlice,
    CumulativePutaways,
    PickPresentationsPerSlice,
    CumulativePickPresentations,
    PutawayPresentationsPerSlice,
    CumulativePutawayPresentations,
    PutawayPresentationsDirectPerSlice,
    CumulativePutawayPresentationsDirect,
    PutawayPresentationsFromRackPerSlice,
    CumulativePutawayPresentationsFromRack,
    CumulativeOrders,
    OrdersPerHour,
    OrderExtractionsPerSlice,
    OrderExtractionsPerHour,
    ExtractedUnitsPerSlice,
    ExtractedUnitsPerHour,
    Presentations,
    CumulativePresentations,
    PresentationsPerStationHour,
    VtmIdle,
    VtmTraveling,
    VtmLoadingUnloading,
    HtmIdle,
    HtmTraveling,
    HtmRotating,
    HtmProcessing,
    HtmWaitLoadedInbound,
    HtmWaitLoadedOutbound,
    HtmPickAtStation,
    RelayReservationFailures,
    SimSpeed,
    SimSpeedAverage,
    WallClockPerSlice,
    VtmTravelDistance,
    HtmTravelDistance,
    VtmForwardTrips,
    VtmReturnTrips,
    HtmPickupTrips,
    HtmReturnTrips,
    VtmForwardCycle,
    VtmReturnCycle,
    HtmCycle,
    HtmOperatorWait,
    HtmRoundTripDistance,
    HtmRoundTripTime,
    HtmRoundTripSpeed,
    HtmReturnRelayRetries,
    OperatorQueueWaits,
    HtmToRelayDistance,
    HtmToStationDistance,
    HtmToReturnDistance,
    HtmToParkingDistance,
    RackOccupancy,
    Inventory,
    PipelineDepth,
    PickStationPicksPerHour,
    PickStationPresentations,
    InductedTotesPerHour,
    InductedUnitsPerHour,
    PutawayTotesStoredPerHour,
    CumulativePallets,
    HtmWaitAtRelay,
    HtmWaitAtRelayAverage,
    DivingBoardSlotWait,
    DivingBoardSlotWaitAverage,
    RackFillPercent,
    RackMaxFillPercent,
    RackFillMinAislePercent,
    RackFillMaxAislePercent,
    RelayOccupancyPercent,
    RelayReservationUtilPercent,
}

/// What a time series is called and how its values read; the formula is `slices`'.
struct Spec {
    name: &'static str,
    label: &'static str,
    description: &'static str,
    unit: MetricUnit,
    /// The clock of a duration or of a rate's hour: simtime, but for the wall clock a slice took to simulate.
    clock: Option<DurationClock>,
    /// `Additive` for a quantity each slice contributes, whose trailing windows sum; any other's windows average.
    aggregation: Aggregation,
    denominator: Option<&'static str>,
    /// Drawn smoothed by default in desvis, as a mean over 25 slices.
    smoothed: bool,
}

/// The slices desvis smooths a series over by default.
const SMOOTHING_SLICES: u64 = 25;

impl TimeSeries {
    pub const ALL: [TimeSeries; 74] = [
        Self::PicksPerSlice,
        Self::PicksPerHour,
        Self::CumulativePicks,
        Self::PutawaysPerSlice,
        Self::CumulativePutaways,
        Self::PickPresentationsPerSlice,
        Self::CumulativePickPresentations,
        Self::PutawayPresentationsPerSlice,
        Self::CumulativePutawayPresentations,
        Self::PutawayPresentationsDirectPerSlice,
        Self::CumulativePutawayPresentationsDirect,
        Self::PutawayPresentationsFromRackPerSlice,
        Self::CumulativePutawayPresentationsFromRack,
        Self::CumulativeOrders,
        Self::OrdersPerHour,
        Self::OrderExtractionsPerSlice,
        Self::OrderExtractionsPerHour,
        Self::ExtractedUnitsPerSlice,
        Self::ExtractedUnitsPerHour,
        Self::Presentations,
        Self::CumulativePresentations,
        Self::PresentationsPerStationHour,
        Self::VtmIdle,
        Self::VtmTraveling,
        Self::VtmLoadingUnloading,
        Self::HtmIdle,
        Self::HtmTraveling,
        Self::HtmRotating,
        Self::HtmProcessing,
        Self::HtmWaitLoadedInbound,
        Self::HtmWaitLoadedOutbound,
        Self::HtmPickAtStation,
        Self::RelayReservationFailures,
        Self::SimSpeed,
        Self::SimSpeedAverage,
        Self::WallClockPerSlice,
        Self::VtmTravelDistance,
        Self::HtmTravelDistance,
        Self::VtmForwardTrips,
        Self::VtmReturnTrips,
        Self::HtmPickupTrips,
        Self::HtmReturnTrips,
        Self::VtmForwardCycle,
        Self::VtmReturnCycle,
        Self::HtmCycle,
        Self::HtmOperatorWait,
        Self::HtmRoundTripDistance,
        Self::HtmRoundTripTime,
        Self::HtmRoundTripSpeed,
        Self::HtmReturnRelayRetries,
        Self::OperatorQueueWaits,
        Self::HtmToRelayDistance,
        Self::HtmToStationDistance,
        Self::HtmToReturnDistance,
        Self::HtmToParkingDistance,
        Self::RackOccupancy,
        Self::Inventory,
        Self::PipelineDepth,
        Self::PickStationPicksPerHour,
        Self::PickStationPresentations,
        Self::InductedTotesPerHour,
        Self::InductedUnitsPerHour,
        Self::PutawayTotesStoredPerHour,
        Self::CumulativePallets,
        Self::HtmWaitAtRelay,
        Self::HtmWaitAtRelayAverage,
        Self::DivingBoardSlotWait,
        Self::DivingBoardSlotWaitAverage,
        Self::RackFillPercent,
        Self::RackMaxFillPercent,
        Self::RackFillMinAislePercent,
        Self::RackFillMaxAislePercent,
        Self::RelayOccupancyPercent,
        Self::RelayReservationUtilPercent,
    ];

    /// Where this series sits in `ALL`, and in a slice's values: `ALL` lists the series in the order they are declared.
    pub fn index(self) -> usize {
        self as usize
    }

    pub fn name(self) -> &'static str {
        self.spec().name
    }

    /// A series whose bucket spans the run from its start: its rows are `[0, t]`, never smoothed.
    pub fn is_cumulative(self) -> bool {
        matches!(
            self,
            Self::CumulativePicks
                | Self::CumulativePutaways
                | Self::CumulativeOrders
                | Self::CumulativePresentations
                | Self::CumulativePickPresentations
                | Self::CumulativePutawayPresentations
                | Self::CumulativePutawayPresentationsDirect
                | Self::CumulativePutawayPresentationsFromRack
                | Self::CumulativePallets
                | Self::SimSpeedAverage
        )
    }

    /// Whether the trailing window a viewer asks for sums the slices (a count, a duration, a distance the slice
    /// contributed) or averages them (a rate, a reading, a quotient).
    pub fn trailing_sums(self) -> bool {
        self.spec().aggregation == Aggregation::Additive && !self.is_cumulative()
    }

    /// The trailing window a series is served in when the viewer names none: desvis's default smoothing, a mean over
    /// 25 slices at every slice, or none. A window of a quantity each slice contributes states the sum of its slices,
    /// never their mean, so such a series desvis smooths, Presentations, is served one row per slice, the raw line
    /// desvis draws its mean over; a viewer asks for its windows.
    pub fn default_trailing(self, slice_width: SimDuration) -> Option<TrailingSpec> {
        let window = slice_width * SMOOTHING_SLICES;
        let smoothed = self.spec().smoothed && !self.trailing_sums();
        smoothed.then_some(TrailingSpec { window_sim_ms: window, step_sim_ms: slice_width })
    }

    fn spec(self) -> Spec {
        let count = |name, label, description| Spec {
            name,
            label,
            description,
            unit: MetricUnit::Count,
            clock: None,
            aggregation: Aggregation::Additive,
            denominator: None,
            smoothed: false,
        };
        // A running total is a count from the run's start; `is_cumulative` says which counts run.
        let cumulative = count;
        let rate = |name, label, description| Spec {
            name,
            label,
            description,
            unit: MetricUnit::RatePerHour,
            clock: Some(DurationClock::Sim),
            aggregation: Aggregation::SnapshotGauge,
            denominator: None,
            smoothed: true,
        };
        let robots = |name, label, description| Spec {
            name,
            label,
            description,
            unit: MetricUnit::Count,
            clock: None,
            aggregation: Aggregation::SnapshotGauge,
            denominator: Some("robots, averaged over the slice"),
            smoothed: false,
        };
        // The time a slice's events took in all, which a window sums.
        let duration_total = |name, label, description| Spec {
            name,
            label,
            description,
            unit: MetricUnit::DurationMs,
            clock: Some(DurationClock::Sim),
            aggregation: Aggregation::Additive,
            denominator: None,
            smoothed: false,
        };
        // The time a slice's events took each, on average, which a window averages.
        let duration_mean = |name, label, description, smoothed| Spec {
            aggregation: Aggregation::SnapshotGauge,
            smoothed,
            ..duration_total(name, label, description)
        };
        let distance = |name, label, description| Spec {
            name,
            label,
            description,
            unit: MetricUnit::DistanceMm,
            clock: None,
            aggregation: Aggregation::Additive,
            denominator: None,
            smoothed: false,
        };
        let gauge = |name, label, description, denominator, smoothed| Spec {
            name,
            label,
            description,
            unit: MetricUnit::Ratio,
            clock: None,
            aggregation: Aggregation::SnapshotGauge,
            denominator: Some(denominator),
            smoothed,
        };
        match self {
            Self::PicksPerSlice => count("picks", "Picks", "Picks completed in the slice."),
            Self::PicksPerHour => {
                rate("picks_per_hour", "Picks/hr (rate)", "The slice's picks as a rate per simulated hour.")
            }
            Self::CumulativePicks => {
                cumulative("cumulative_picks", "Cumulative picks", "Picks completed since the run began.")
            }
            Self::PutawaysPerSlice => {
                count("inducted_units", "Inducted units", "Putaway units completed in the slice.")
            }
            Self::CumulativePutaways => cumulative(
                "cumulative_inducted_units",
                "Cumulative inducted units",
                "Putaway units completed since the run began.",
            ),
            Self::PickPresentationsPerSlice => {
                count("pick_presentations", "Pick pres", "Pick presentations in the slice.")
            }
            Self::CumulativePickPresentations => cumulative(
                "cumulative_pick_presentations",
                "Cumulative pick pres",
                "Pick presentations since the run began.",
            ),
            Self::PutawayPresentationsPerSlice => {
                count("putaway_presentations", "Putaway pres", "Putaway presentations in the slice.")
            }
            Self::CumulativePutawayPresentations => cumulative(
                "cumulative_putaway_presentations",
                "Cumulative putaway pres",
                "Putaway presentations since the run began.",
            ),
            Self::PutawayPresentationsDirectPerSlice => count(
                "putaway_presentations_direct",
                "Putaway pres (direct)",
                "Putaway station presentations dispatched direct, in the slice.",
            ),
            Self::CumulativePutawayPresentationsDirect => cumulative(
                "cumulative_putaway_presentations_direct",
                "Cumulative putaway pres (direct)",
                "Direct putaway station presentations since the run began.",
            ),
            Self::PutawayPresentationsFromRackPerSlice => count(
                "putaway_presentations_from_rack",
                "Putaway pres (from rack)",
                "Putaway station presentations from the rack, in the slice.",
            ),
            Self::CumulativePutawayPresentationsFromRack => cumulative(
                "cumulative_putaway_presentations_from_rack",
                "Cumulative putaway pres (from rack)",
                "Putaway station presentations from the rack since the run began.",
            ),
            Self::CumulativeOrders => cumulative(
                "cumulative_orders",
                "Cumulative fully picked orders",
                "Orders fully picked since the run began.",
            ),
            Self::OrdersPerHour => rate(
                "orders_per_hour",
                "Fully picked orders/hr",
                "The slice's fully picked orders as a rate per simulated hour.",
            ),
            Self::OrderExtractionsPerSlice => count(
                "order_extractions",
                "Completed order extractions",
                "Order extraction presentations completed in the slice.",
            ),
            Self::OrderExtractionsPerHour => rate(
                "order_extractions_per_hour",
                "Order extractions/hr",
                "The slice's completed order extractions as a rate per simulated hour.",
            ),
            Self::ExtractedUnitsPerSlice => {
                count("extracted_units", "Extracted units", "Units extracted for orders in the slice.")
            }
            Self::ExtractedUnitsPerHour => rate(
                "extracted_units_per_hour",
                "Extracted units/hr",
                "The slice's extracted units as a rate per simulated hour.",
            ),
            Self::Presentations => Spec {
                smoothed: true,
                ..count("presentations", "Presentations", "Presentations of every kind in the slice.")
            },
            Self::CumulativePresentations => {
                cumulative("cumulative_presentations", "Cumulative presentations", "Presentations since the run began.")
            }
            Self::PresentationsPerStationHour => Spec {
                smoothed: false,
                aggregation: Aggregation::SnapshotGauge,
                denominator: Some("stations"),
                ..rate(
                    "presentations_per_station_hour",
                    "Pres/stn/hr",
                    "The slice's presentations per station as a rate per simulated hour.",
                )
            },
            Self::VtmIdle => robots(
                "da50_idle",
                "DA50 idle",
                "DA50 robots in no duty, averaged over the slice: the fleet less those traveling or loading.",
            ),
            Self::VtmTraveling => {
                robots("da50_traveling", "DA50 traveling", "DA50 robots traveling, averaged over the slice.")
            }
            Self::VtmLoadingUnloading => robots(
                "da50_dropoff_collection",
                "DA50 dropoff/collection",
                "DA50 robots loading or unloading, averaged over the slice.",
            ),
            Self::HtmIdle => robots(
                "d30_idle",
                "D30 idle",
                "D30 robots in no duty, averaged over the slice: the fleet less those traveling, loading or processing.",
            ),
            Self::HtmTraveling => {
                robots("d30_traveling", "D30 traveling", "D30 robots traveling, averaged over the slice.")
            }
            Self::HtmRotating => robots(
                "d30_rotating",
                "D30 rotating",
                "D30 robots rotating, averaged over the slice; part of traveling.",
            ),
            Self::HtmProcessing => robots(
                "d30_processing",
                "D30 processing",
                "D30 robots processing at a station, averaged over the slice.",
            ),
            Self::HtmWaitLoadedInbound => robots(
                "d30_wait_loaded_inbound",
                "D30 wait-loaded inbound",
                "D30 robots holding a tote and waiting on their way to a station, averaged over the slice; part of traveling.",
            ),
            Self::HtmWaitLoadedOutbound => robots(
                "d30_wait_loaded_outbound",
                "D30 wait-loaded outbound",
                "D30 robots holding a tote and waiting on their way from a station to a return relay, averaged over the slice; part of traveling.",
            ),
            Self::HtmPickAtStation => robots(
                "d30_pick_at_station",
                "D30 pick-at-stn",
                "D30 robots held at a pick station by the operator's pick, averaged over the slice.",
            ),
            Self::RelayReservationFailures => {
                count("relay_failures", "Relay failures", "Relay reservations refused in the slice.")
            }
            Self::SimSpeed => gauge(
                "sim_speed",
                "Sim speed (slice)",
                "Simtime the slice covers over the wall clock it took: how many times faster than real time.",
                "wall clock",
                true,
            ),
            Self::SimSpeedAverage => gauge(
                "sim_speed_average",
                "Sim speed (avg)",
                "Simtime reached over the wall clock spent since the run began.",
                "wall clock",
                false,
            ),
            Self::WallClockPerSlice => Spec {
                clock: Some(DurationClock::Wall),
                ..duration_total(
                    "wall_clock_per_slice",
                    "Wall clock/slice",
                    "Wall clock the slice took to simulate; a wall-clock duration, not simtime.",
                )
            },
            Self::VtmTravelDistance => {
                distance("da50_travel_distance", "DA50 travel", "Distance DA50 robots traveled in the slice.")
            }
            Self::HtmTravelDistance => {
                distance("d30_travel_distance", "D30 travel", "Distance D30 robots traveled in the slice.")
            }
            Self::VtmForwardTrips => {
                count("da50_forward_trips", "DA50 fwd trips", "DA50 forward trips completed in the slice.")
            }
            Self::VtmReturnTrips => {
                count("da50_return_trips", "DA50 ret trips", "DA50 return trips completed in the slice.")
            }
            Self::HtmPickupTrips => {
                count("d30_pickup_trips", "D30 pickup trips", "D30 pickup trips completed in the slice.")
            }
            Self::HtmReturnTrips => {
                count("d30_return_trips", "D30 ret trips", "D30 return trips completed in the slice.")
            }
            Self::VtmForwardCycle => duration_mean(
                "da50_forward_cycle",
                "DA50 fwd cycle",
                "Mean DA50 forward cycle time over the trips completed in the slice.",
                false,
            ),
            Self::VtmReturnCycle => duration_mean(
                "da50_return_cycle",
                "DA50 ret cycle",
                "Mean DA50 return cycle time over the trips completed in the slice.",
                false,
            ),
            Self::HtmCycle => duration_mean(
                "d30_cycle",
                "D30 cycle",
                "Mean D30 cycle time over the pickup trips completed in the slice.",
                false,
            ),
            Self::HtmOperatorWait => duration_mean(
                "d30_operator_wait",
                "D30 op-wait",
                "Mean D30 wait for an operator over the waits ended in the slice.",
                false,
            ),
            Self::HtmRoundTripDistance => Spec {
                aggregation: Aggregation::SnapshotGauge,
                ..distance(
                    "d30_round_trip_distance",
                    "D30 round trip (m)",
                    "Mean D30 round-trip distance over the return trips completed in the slice.",
                )
            },
            Self::HtmRoundTripTime => duration_mean(
                "d30_round_trip_time",
                "D30 round trip (s)",
                "Mean D30 round-trip time over the return trips completed in the slice.",
                false,
            ),
            Self::HtmRoundTripSpeed => gauge(
                "d30_round_trip_speed",
                "D30 round trip (m/s)",
                "D30 round-trip distance over round-trip time in the slice, metres per second.",
                "metres per second of simtime",
                true,
            ),
            Self::HtmReturnRelayRetries => {
                count("d30_relay_retries", "D30 relay retries", "D30 return relay retries in the slice.")
            }
            Self::OperatorQueueWaits => {
                count("operator_queue_waits", "Operator queue waits", "Waits in an operator queue begun in the slice.")
            }
            Self::HtmToRelayDistance => distance(
                "d30_to_relay_distance",
                "D30 to relay",
                "Distance D30 robots traveled to a pickup relay in the slice.",
            ),
            Self::HtmToStationDistance => distance(
                "d30_to_station_distance",
                "D30 to station",
                "Distance D30 robots traveled to a station in the slice.",
            ),
            Self::HtmToReturnDistance => distance(
                "d30_to_return_distance",
                "D30 to return",
                "Distance D30 robots traveled to a return relay in the slice.",
            ),
            Self::HtmToParkingDistance => distance(
                "d30_to_parking_distance",
                "D30 to parking",
                "Distance D30 robots traveled to parking in the slice.",
            ),
            Self::RackOccupancy => Spec {
                aggregation: Aggregation::SnapshotGauge,
                ..count("rack_occupancy", "Rack occupancy", "Rack slots holding a laden tote at the slice's end.")
            },
            Self::Inventory => Spec {
                aggregation: Aggregation::SnapshotGauge,
                ..count("inventory_units", "Inventory (units)", "Units in the warehouse at the slice's end.")
            },
            Self::PipelineDepth => gauge(
                "pipeline_depth",
                "Pipeline depth (avg)",
                "Mean pipeline depth over the slice's snapshots.",
                "snapshots",
                false,
            ),
            Self::PickStationPicksPerHour => rate(
                "pick_station_picks_per_hour",
                "Pick stn picks/hr",
                "The slice's pick-station picks as a rate per simulated hour.",
            ),
            Self::PickStationPresentations => {
                count("pick_station_presentations", "Pick stn pres", "Pick-station presentations in the slice.")
            }
            Self::InductedTotesPerHour => rate(
                "inducted_totes_per_hour",
                "Inducted totes/hr (rate)",
                "The slice's putaway-station presentations as a rate per simulated hour.",
            ),
            Self::InductedUnitsPerHour => rate(
                "inducted_units_per_hour",
                "Inducted units/hr (rate)",
                "The slice's putaway-station picks as a rate per simulated hour.",
            ),
            Self::PutawayTotesStoredPerHour => rate(
                "putaway_totes_stored_per_hour",
                "Putaway totes stored/hr",
                "The slice's putaway totes stored as a rate per simulated hour.",
            ),
            Self::CumulativePallets => {
                cumulative("cumulative_pallets", "Cumulative pallets", "Pick-station orders since the run began.")
            }
            Self::HtmWaitAtRelay => duration_total(
                "d30_wait_at_relay",
                "D30 tote-wait@relay",
                "D30 time waiting at relays for a tote in the slice.",
            ),
            Self::HtmWaitAtRelayAverage => duration_mean(
                "d30_wait_at_relay_average",
                "D30 tote-wait@relay avg",
                "Mean D30 wait at a relay over the relay pickups in the slice.",
                true,
            ),
            Self::DivingBoardSlotWait => duration_total(
                "diving_board_slot_wait",
                "Diving-board slot wait",
                "Time relay reservations waited on a held diving-board slot in the slice.",
            ),
            Self::DivingBoardSlotWaitAverage => duration_mean(
                "diving_board_slot_wait_average",
                "Diving-board slot wait avg",
                "Mean diving-board slot wait over the presentations in the slice.",
                true,
            ),
            Self::RackFillPercent => gauge(
                "rack_fill_percent",
                "Rack fill %",
                "Rack slots holding a laden tote as a percentage of capacity, at the slice's end.",
                "percent of rack capacity",
                false,
            ),
            Self::RackMaxFillPercent => gauge(
                "rack_max_fill_percent",
                "Rack max fill %",
                "The slice's peak rack fill as a percentage of capacity.",
                "percent of rack capacity",
                false,
            ),
            Self::RackFillMinAislePercent => gauge(
                "rack_fill_min_aisle_percent",
                "Rack fill % (min aisle)",
                "The least filled aisle's fill percentage at the slice's end.",
                "percent of the aisle's capacity",
                false,
            ),
            Self::RackFillMaxAislePercent => gauge(
                "rack_fill_max_aisle_percent",
                "Rack fill % (max aisle)",
                "The most filled aisle's fill percentage at the slice's end.",
                "percent of the aisle's capacity",
                false,
            ),
            Self::RelayOccupancyPercent => gauge(
                "relay_occupancy_percent",
                "Relay occupancy %",
                "Relays holding a deposited tote as a percentage of relays, at the slice's end.",
                "percent of relays",
                true,
            ),
            Self::RelayReservationUtilPercent => gauge(
                "relay_reservation_util_percent",
                "Relay reservation util %",
                "Relay access cells' reserved time as a percentage of the relays' slice time.",
                "percent of relay time",
                true,
            ),
        }
    }
}

/// The descriptor a viewer is told at its attach: the series in `bucket`, as the server will send it.
pub fn descriptor(series: TimeSeries, bucket: Bucket) -> MetricDescriptor {
    let spec = series.spec();
    let description = match series.measures() {
        Some(structure) => format!("{} {}", spec.description, structure.absent()),
        None => spec.description.to_owned(),
    };
    MetricDescriptor {
        metric: spec.name.to_owned(),
        label: spec.label.to_owned(),
        description,
        unit: spec.unit,
        clock: spec.clock,
        aggregation: spec.aggregation,
        denominator: spec.denominator.map(str::to_owned),
        source: MetricSource::Scalars,
        bucket,
        dimensions: Vec::new(),
    }
}

/// Every time series, in desvis's order, each described in its own bucketing at a 10 s slice.
pub fn series_catalogue() -> Vec<MetricDescriptor> {
    TimeSeries::ALL.iter().map(|series| descriptor(*series, default_bucket(*series, SimDuration::ms(10_000)))).collect()
}

/// The bucketing a series is served in when the viewer names none: trailing windows for the ones desvis smooths
/// whose windows average (`default_trailing`), one bucket per slice for the rest.
pub fn default_bucket(series: TimeSeries, slice_width: SimDuration) -> Bucket {
    match series.default_trailing(slice_width) {
        Some(trailing) => Bucket::Trailing(trailing),
        None => Bucket::Slice { width_sim_ms: slice_width },
    }
}

/// The clock a scalar the recording declares counts in, when its unit measures time: the wall clock for the wall clock
/// a slice took to simulate, simtime for every other duration or rate.
///
/// TODO(_zbdTH5Q): `vtm_scoring_duration_ns` is declared a duration and so read here as simtime, though it would time
/// the scorer on the wall clock; no code emits it yet, so it is always 0.
pub fn recorded_clock(name: &str, unit: MetricUnit) -> Option<DurationClock> {
    match unit {
        MetricUnit::DurationMs | MetricUnit::RatePerHour if name == SLICE_WALL_CLOCK => Some(DurationClock::Wall),
        MetricUnit::DurationMs | MetricUnit::RatePerHour => Some(DurationClock::Sim),
        MetricUnit::Count | MetricUnit::DistanceMm | MetricUnit::Ratio => None,
    }
}

/// The time series `name` names, if any.
pub fn find_series(name: &str) -> Option<TimeSeries> {
    TimeSeries::ALL.iter().copied().find(|series| series.name() == name)
}

#[cfg(test)]
mod tests {
    use rustc_hash::FxHashSet;

    use super::*;

    #[test]
    fn seventy_four_series_each_with_a_name_of_its_own_found_back_by_it() {
        let names: FxHashSet<&str> = TimeSeries::ALL.iter().map(|series| series.name()).collect();
        assert_eq!(names.len(), 74, "no two series share a name");
        for (index, series) in TimeSeries::ALL.iter().enumerate() {
            assert_eq!(series.index(), index);
            assert_eq!(find_series(series.name()), Some(*series), "{}", series.name());
            assert!(
                series.name().chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{}",
                series.name()
            );
        }
        assert_eq!(find_series("picks_per_hour"), Some(TimeSeries::PicksPerHour));
        assert_eq!(find_series("no such metric"), None);
        assert_eq!(series_catalogue().len(), 74);
    }

    /// desvis smooths fifteen series with a mean over 25 slices. Fourteen come as trailing windows of 250 s by
    /// default, each the mean of its slices; Presentations comes one row per slice, since a window of it sums its
    /// slices, which desvis's smoothing never does. A cumulative series never comes in windows.
    #[test]
    fn the_series_desvis_smooths_default_to_a_trailing_mean_and_presentations_to_its_slices() {
        let width = SimDuration::ms(10_000);
        let smoothed: Vec<TimeSeries> =
            TimeSeries::ALL.iter().copied().filter(|series| series.default_trailing(width).is_some()).collect();
        assert_eq!(smoothed.len(), 14, "{smoothed:?}");
        assert!(smoothed.contains(&TimeSeries::PicksPerHour) && smoothed.contains(&TimeSeries::SimSpeed));
        assert_eq!(
            default_bucket(TimeSeries::Presentations, width),
            Bucket::Slice { width_sim_ms: width },
            "desvis's raw line, its smoothing being a mean"
        );
        for series in TimeSeries::ALL {
            assert!(!(series.is_cumulative() && series.default_trailing(width).is_some()), "{series:?}");
            assert!(!(series.is_cumulative() && series.trailing_sums()), "{series:?} is already a running total");
        }
        assert_eq!(
            descriptor(TimeSeries::PicksPerHour, default_bucket(TimeSeries::PicksPerHour, width)).bucket,
            Bucket::Trailing(TrailingSpec { window_sim_ms: SimDuration::ms(250_000), step_sim_ms: width })
        );
    }

    /// desvis offers a trailing sum for exactly the series whose slices each contribute a quantity, a count, a
    /// duration or a distance (`supports_trailing_sum`, `desvis/src/types/metric_extract.rs`): their windows sum here
    /// and their descriptors say `Additive`, wall clock, relay wait and diving-board wait per slice among them; every
    /// other series' windows average.
    #[test]
    fn the_windows_sum_exactly_the_series_desvis_offers_a_trailing_sum_for() {
        use TimeSeries::*;
        let mut desvis_sums = vec![
            PicksPerSlice,
            PutawaysPerSlice,
            PickPresentationsPerSlice,
            OrderExtractionsPerSlice,
            ExtractedUnitsPerSlice,
            PutawayPresentationsPerSlice,
            PutawayPresentationsDirectPerSlice,
            PutawayPresentationsFromRackPerSlice,
            Presentations,
            RelayReservationFailures,
            WallClockPerSlice,
            VtmTravelDistance,
            HtmTravelDistance,
            VtmForwardTrips,
            VtmReturnTrips,
            HtmPickupTrips,
            HtmReturnTrips,
            HtmReturnRelayRetries,
            OperatorQueueWaits,
            HtmToRelayDistance,
            HtmToStationDistance,
            HtmToReturnDistance,
            HtmToParkingDistance,
            PickStationPresentations,
            HtmWaitAtRelay,
            DivingBoardSlotWait,
        ];
        desvis_sums.sort_by_key(|series| series.index());
        let sums: Vec<TimeSeries> = TimeSeries::ALL.into_iter().filter(|series| series.trailing_sums()).collect();
        assert_eq!(sums, desvis_sums);
        let width = SimDuration::ms(10_000);
        for series in [WallClockPerSlice, HtmWaitAtRelay, DivingBoardSlotWait] {
            assert_eq!(descriptor(series, default_bucket(series, width)).aggregation, Aggregation::Additive);
        }
    }
}
