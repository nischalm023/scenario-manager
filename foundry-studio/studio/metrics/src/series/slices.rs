//! Within `series`, `slices` is EVERY TIME SERIES' VALUE IN EACH SLICE, read off the slice's scalars by the formulas of
//! desvis's `metric_extract.rs` and `graph.rs`, and kept from the run's start, so a series can be cut into any
//! bucketing at any time. A delta's scalars section carries the slice's own contribution to each key, which is what
//! every formula reads; the cumulative series and the average sim speed read running sums kept from the run's start.
//! Durations stay in milliseconds and distances in millimetres, as the recording states them. A series reads only the
//! keys its formula names, so a key the recording does not declare is a fact the recording lacks. A series that
//! measures stations, rack storage slots or relays has no value in a world without any, as desvis says "Unavailable"
//! rather than draw zeros (`unavailable_reason`).

use desunits::{SimDuration, SimTime};
use rustc_hash::FxHashMap;
use studio_protocol::MetricPoint;
use worldview::Descriptors;
use worldview::frames::scalars::ScalarsDelta;

/// The scalar a slice states the wall clock it took to simulate in, a duration on the wall clock rather than simtime.
pub(crate) const SLICE_WALL_CLOCK: &str = "wall_clock_duration_ms";

use super::TimeSeries;

/// What the slices do not state and a formula needs: the world's station count, for presentations per station.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct World {
    pub(crate) stations: usize,
}

/// What a series measures that a world can have none of: stations, which the scenario block's station table counts;
/// rack storage slots and relays, which the slices count. A world has rack slots or relays from the first slice taken
/// that counts some, desvis's rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Structure {
    Stations,
    RackSlots,
    Relays,
}

impl Structure {
    /// The scalar a slice counts the structure in; none for stations.
    fn key(self) -> Option<&'static str> {
        match self {
            Self::Stations => None,
            Self::RackSlots => Some("rack_capacity"),
            Self::Relays => Some("relay_cells_total"),
        }
    }

    /// What a series measuring the structure says of a world without it, in its descriptor.
    pub(crate) fn absent(self) -> &'static str {
        match self {
            Self::Stations => "No value in a world without stations.",
            Self::RackSlots => "No value in a world without rack storage slots.",
            Self::Relays => "No value in a world without relays.",
        }
    }
}

/// How a time series reads a slice's scalars: the keys it names, and what it makes of their values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Formula {
    /// The slice's value of a key.
    Value(&'static str),
    /// A key's running total since the run's start.
    Running(&'static str),
    /// The slice's value per simulated hour.
    PerHour(&'static str),
    /// The slice's value per simulated hour per station of the world, which has some (`Structure::Stations`).
    PerStationHour(&'static str),
    /// A duty's milliseconds over the slice's width: how many robots it kept busy on average.
    Robots(&'static str),
    /// The robots the slice counts in a family of state counts, less those the duties kept busy, never below none.
    Idle { family: &'static str, duties: &'static [&'static str] },
    /// The first key's value over the second's, zero where the second is.
    Mean(&'static str, &'static str),
    /// A hundred times the first key's value over the second's, zero where the second is.
    Percent(&'static str, &'static str),
    /// A hundred times the reserved milliseconds over the cells' time in the slice, zero where there is no cell.
    Reserved { busy: &'static str, cells: &'static str },
    /// The slice's simulated width over the wall clock it took.
    Speed(&'static str),
    /// The simulated time so far over the wall clock taken so far.
    SpeedSoFar(&'static str),
}

impl Formula {
    /// Every key the formula reads by name; a family of state counts is read by its prefix and is no key.
    pub(crate) fn keys(self) -> Vec<&'static str> {
        match self {
            Self::Value(key)
            | Self::Running(key)
            | Self::PerHour(key)
            | Self::PerStationHour(key)
            | Self::Robots(key)
            | Self::Speed(key)
            | Self::SpeedSoFar(key) => vec![key],
            Self::Idle { duties, .. } => duties.to_vec(),
            Self::Mean(part, whole) | Self::Percent(part, whole) => vec![part, whole],
            Self::Reserved { busy, cells } => vec![busy, cells],
        }
    }

    /// The key whose running total the formula reads.
    fn running(self) -> Option<&'static str> {
        match self {
            Self::Running(key) | Self::SpeedSoFar(key) => Some(key),
            _ => None,
        }
    }
}

impl TimeSeries {
    /// How the series reads a slice's scalars.
    pub(crate) fn formula(self) -> Formula {
        use Formula::*;
        match self {
            Self::PicksPerSlice => Value("picks_completed"),
            Self::PicksPerHour => PerHour("picks_completed"),
            Self::CumulativePicks => Running("picks_completed"),
            Self::PutawaysPerSlice => Value("putaways_completed"),
            Self::CumulativePutaways => Running("putaways_completed"),
            Self::PickPresentationsPerSlice => Value("pick_presentations"),
            Self::CumulativePickPresentations => Running("pick_presentations"),
            Self::PutawayPresentationsPerSlice => Value("putaway_presentations"),
            Self::CumulativePutawayPresentations => Running("putaway_presentations"),
            Self::PutawayPresentationsDirectPerSlice => Value("putaway_station_presentations_direct"),
            Self::CumulativePutawayPresentationsDirect => Running("putaway_station_presentations_direct"),
            Self::PutawayPresentationsFromRackPerSlice => Value("putaway_station_presentations_from_rack"),
            Self::CumulativePutawayPresentationsFromRack => Running("putaway_station_presentations_from_rack"),
            Self::CumulativeOrders => Running("orders_completed"),
            Self::OrdersPerHour => PerHour("orders_completed"),
            Self::OrderExtractionsPerSlice => Value("order_extract_presentations"),
            Self::OrderExtractionsPerHour => PerHour("order_extract_presentations"),
            Self::ExtractedUnitsPerSlice => Value("order_extracted_units"),
            Self::ExtractedUnitsPerHour => PerHour("order_extracted_units"),
            Self::Presentations => Value("presentations"),
            Self::CumulativePresentations => Running("presentations"),
            Self::PresentationsPerStationHour => PerStationHour("presentations"),
            Self::VtmIdle => {
                Idle { family: "vtm_state_", duties: &["vtm_duty_traveling_ms", "vtm_duty_loading_unloading_ms"] }
            }
            Self::VtmTraveling => Robots("vtm_duty_traveling_ms"),
            Self::VtmLoadingUnloading => Robots("vtm_duty_loading_unloading_ms"),
            Self::HtmIdle => Idle {
                family: "htm_state_",
                duties: &["htm_duty_traveling_ms", "htm_duty_loading_unloading_ms", "htm_duty_processing_ms"],
            },
            Self::HtmTraveling => Robots("htm_duty_traveling_ms"),
            Self::HtmRotating => Robots("htm_duty_rotating_ms"),
            Self::HtmProcessing => Robots("htm_duty_processing_ms"),
            Self::HtmWaitLoadedInbound => Robots("htm_duty_wait_loaded_ms"),
            Self::HtmWaitLoadedOutbound => Robots("htm_duty_dwell_loaded_ms"),
            Self::HtmPickAtStation => Robots("htm_duty_pick_at_station_ms"),
            Self::RelayReservationFailures => Value("relay_reservation_failures"),
            Self::SimSpeed => Speed(SLICE_WALL_CLOCK),
            Self::SimSpeedAverage => SpeedSoFar(SLICE_WALL_CLOCK),
            Self::WallClockPerSlice => Value(SLICE_WALL_CLOCK),
            Self::VtmTravelDistance => Value("vtm_travel_distance_mm"),
            Self::HtmTravelDistance => Value("htm_travel_distance_mm"),
            Self::VtmForwardTrips => Value("vtm_forward_trips"),
            Self::VtmReturnTrips => Value("vtm_return_trips"),
            Self::HtmPickupTrips => Value("htm_pickup_trips"),
            Self::HtmReturnTrips => Value("htm_return_trips"),
            Self::VtmForwardCycle => Mean("vtm_forward_cycle_ms", "vtm_forward_trips"),
            Self::VtmReturnCycle => Mean("vtm_return_cycle_ms", "vtm_return_trips"),
            Self::HtmCycle => Mean("htm_cycle_ms", "htm_pickup_trips"),
            Self::HtmOperatorWait => Mean("htm_operator_wait_ms", "htm_operator_wait_count"),
            Self::HtmRoundTripDistance => Mean("htm_round_trip_distance_mm", "htm_return_trips"),
            Self::HtmRoundTripTime => Mean("htm_round_trip_ms", "htm_return_trips"),
            Self::HtmRoundTripSpeed => Mean("htm_round_trip_distance_mm", "htm_round_trip_ms"),
            Self::HtmReturnRelayRetries => Value("htm_return_relay_retries"),
            Self::OperatorQueueWaits => Value("operator_queue_waits"),
            Self::HtmToRelayDistance => Value("htm_to_relay_mm"),
            Self::HtmToStationDistance => Value("htm_to_station_mm"),
            Self::HtmToReturnDistance => Value("htm_to_return_mm"),
            Self::HtmToParkingDistance => Value("htm_to_parking_mm"),
            Self::RackOccupancy => Value("rack_occupancy"),
            Self::Inventory => Value("inventory_units"),
            Self::PipelineDepth => Value("pipeline_depth_avg"),
            Self::PickStationPicksPerHour => PerHour("pick_station_picks"),
            Self::PickStationPresentations => Value("pick_station_presentations"),
            Self::InductedTotesPerHour => PerHour("putaway_station_presentations"),
            Self::InductedUnitsPerHour => PerHour("putaway_station_picks"),
            Self::PutawayTotesStoredPerHour => PerHour("putaway_totes_stored"),
            Self::CumulativePallets => Running("pick_station_orders"),
            Self::HtmWaitAtRelay => Value("htm_wait_at_relay_ms"),
            Self::HtmWaitAtRelayAverage => Mean("htm_wait_at_relay_ms", "htm_relay_pickup_count"),
            Self::DivingBoardSlotWait => Value("diving_board_slot_wait_ms"),
            Self::DivingBoardSlotWaitAverage => Mean("diving_board_slot_wait_ms", "diving_board_presentation_count"),
            Self::RackFillPercent => Percent("rack_occupancy", "rack_capacity"),
            Self::RackMaxFillPercent => Percent("rack_occupancy_peak", "rack_capacity"),
            Self::RackFillMinAislePercent => Value("rack_fill_min_pct"),
            Self::RackFillMaxAislePercent => Value("rack_fill_max_pct"),
            Self::RelayOccupancyPercent => Percent("relay_cells_reserved", "relay_cells_total"),
            Self::RelayReservationUtilPercent => {
                Reserved { busy: "relay_reserved_busy_ms", cells: "relay_cells_total" }
            }
        }
    }

    /// What the series measures that a world can have none of, if anything: desvis's `unavailable_reason`.
    pub(crate) fn measures(self) -> Option<Structure> {
        match self {
            Self::PresentationsPerStationHour => Some(Structure::Stations),
            Self::RackOccupancy
            | Self::RackFillPercent
            | Self::RackMaxFillPercent
            | Self::RackFillMinAislePercent
            | Self::RackFillMaxAislePercent => Some(Structure::RackSlots),
            Self::RelayOccupancyPercent | Self::RelayReservationUtilPercent => Some(Structure::Relays),
            _ => None,
        }
    }

    /// Every key the series reads by name: its formula's, and the one its structure is counted in.
    pub(crate) fn keys(self) -> Vec<&'static str> {
        let mut keys = self.formula().keys();
        keys.extend(self.measures().and_then(Structure::key).filter(|key| !keys.contains(key)));
        keys
    }
}

/// One slice's value of every time series, in `TimeSeries::ALL`'s order, from its start to where the next frame says it
/// ends.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Slice {
    pub from: SimTime,
    pub to: SimTime,
    pub values: Box<[Option<f64>]>,
}

/// Every slice taken since the run's start, each with every time series' value once the next frame says where it ends,
/// and what the formulas carry from one slice to the next.
pub(crate) struct Slices {
    width: SimDuration,
    world: World,
    /// Each declared key's position in a slice's scalars.
    slots: FxHashMap<String, usize>,
    /// The running totals the formulas read, from the run's start.
    running: FxHashMap<&'static str, f64>,
    /// The most rack storage slots, and relays, a slice has counted since the run's start.
    rack_slots: f64,
    relays: f64,
    /// The newest slice taken, its start and its scalars, until the next frame says where it ends.
    open: Option<(SimTime, ScalarsDelta)>,
    taken: Vec<Slice>,
}

impl Slices {
    pub(crate) fn new(width: SimDuration, world: World, descriptors: &Descriptors) -> Self {
        let running = TimeSeries::ALL.iter().filter_map(|series| series.formula().running()).map(|key| (key, 0.0));
        let mut slices = Self {
            width,
            world,
            slots: FxHashMap::default(),
            running: running.collect(),
            rack_slots: 0.0,
            relays: 0.0,
            open: None,
            taken: Vec::new(),
        };
        slices.describe(descriptors);
        slices
    }

    /// The keys the recording declares so far, each at its position in a slice's scalars.
    pub(crate) fn describe(&mut self, descriptors: &Descriptors) {
        self.slots = descriptors.metrics.iter().enumerate().map(|(slot, metric)| (metric.name.clone(), slot)).collect();
    }

    pub(crate) fn declares(&self, key: &str) -> bool {
        self.slots.contains_key(key)
    }

    pub(crate) fn width(&self) -> SimDuration {
        self.width
    }

    pub(crate) fn taken(&self) -> &[Slice] {
        &self.taken
    }

    /// Whether the world has any of `structure`, as the slices taken count it.
    fn has(&self, structure: Structure) -> bool {
        match structure {
            Structure::Stations => self.world.stations > 0,
            Structure::RackSlots => self.rack_slots > 0.0,
            Structure::Relays => self.relays > 0.0,
        }
    }

    /// The slice beginning at `from`, whose scalars `delta` states: it ends the slice before it there, and is held
    /// until the next frame says where it ends in turn.
    pub(crate) fn take(&mut self, from: SimTime, delta: ScalarsDelta) {
        self.close(from);
        self.open = Some((from, delta));
    }

    /// The slice held, in a recording cut short with no frame after it, stated its whole width, as simrunner's stored
    /// series state it: where the slice ends, if one was held.
    pub(crate) fn close_whole(&mut self) -> Option<SimTime> {
        let to = self.open.as_ref().map(|(from, _)| *from + self.width)?;
        self.close(to);
        Some(to)
    }

    /// The slice held ends at `to`, the next frame's instant: every series' value in it kept, a rate taken over the
    /// slice's own width, and the running totals carried on. A recording made before slices stopped at the run's stop
    /// can begin its last slice at the end block; that slice is stated its whole width, as simrunner's stored series
    /// state it (`studio/simrunner/src/recording.rs`), so no two slices end at one instant.
    pub(crate) fn close(&mut self, to: SimTime) {
        let Some((from, delta)) = self.open.take() else { return };
        let to = if to > from { to } else { from + self.width };
        let get = |key: &str| self.slots.get(key).and_then(|slot| delta.values.get(*slot).copied().flatten());
        for (key, total) in &mut self.running {
            *total += get(key).unwrap_or(0.0);
        }
        self.rack_slots = self.rack_slots.max(get("rack_capacity").unwrap_or(0.0));
        self.relays = self.relays.max(get("relay_cells_total").unwrap_or(0.0));
        let family = |prefix: &str| {
            self.slots.keys().filter(|key| key.starts_with(prefix)).map(|key| get(key).unwrap_or(0.0)).sum::<f64>()
        };
        let width = (to - from).as_ms() as f64;
        let hours = width / 3_600_000.0;
        let quotient = |part: f64, whole: f64| if whole > 0.0 { part / whole } else { 0.0 };
        let values = TimeSeries::ALL
            .iter()
            .map(|series| match series.formula() {
                _ if series.measures().is_some_and(|structure| !self.has(structure)) => None,
                Formula::Value(key) => get(key),
                Formula::Running(key) => self.running.get(key).copied(),
                Formula::PerHour(key) => get(key).map(|value| quotient(value, hours)),
                Formula::PerStationHour(key) => {
                    get(key).map(|value| quotient(value, hours) / self.world.stations as f64)
                }
                Formula::Robots(key) => get(key).map(|duty| quotient(duty, width)),
                Formula::Idle { family: prefix, duties } => {
                    let busy = duties.iter().map(|duty| get(duty)).sum::<Option<f64>>()?;
                    Some((family(prefix) - busy / width).max(0.0))
                }
                Formula::Mean(part, whole) => Some(quotient(get(part)?, get(whole)?)),
                Formula::Percent(part, whole) => Some(100.0 * quotient(get(part)?, get(whole)?)),
                Formula::Reserved { busy, cells } => Some(100.0 * quotient(get(busy)?, get(cells)? * width)),
                Formula::Speed(wall) => get(wall).map(|wall| quotient(width, wall)),
                Formula::SpeedSoFar(wall) => self.running.get(wall).map(|wall| quotient(to.as_ms() as f64, *wall)),
            })
            .collect();
        self.taken.push(Slice { from, to, values });
    }
}

impl Slice {
    /// The slice's row of `series`: a cumulative series' bucket spans the run from its start.
    pub(crate) fn point(&self, series: TimeSeries) -> MetricPoint {
        MetricPoint {
            from_sim_ms: if series.is_cumulative() { SimTime::ZERO } else { self.from },
            to_sim_ms: self.to,
            keys: Vec::new(),
            value: self.values[series.index()],
            numerator: None,
            denominator: None,
            whole_run: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use worldview::frames::scalars::{AggregationClass, MetricDescriptor, MetricSemantics, MetricUnit};

    use super::*;

    /// A recording declaring `keys`, in order.
    fn declaring(keys: &[&str]) -> Descriptors {
        let semantics = MetricSemantics {
            unit: MetricUnit::Count,
            denominator_description: None,
            aggregation: AggregationClass::Additive,
        };
        let metric = |key: &&str| MetricDescriptor {
            name: (*key).to_owned(),
            label: (*key).to_owned(),
            description: String::new(),
            semantics: semantics.clone(),
        };
        Descriptors { metrics: keys.iter().map(metric).collect(), tables: Vec::new() }
    }

    /// The first slice counts no rack storage slot and no relay, as a world without them states; the second counts
    /// twenty slots and four relays. The rack and relay series have no value in the first and the slice's own in the
    /// second, as desvis decides "Unavailable" by what the slices count; presentations per station have none in a
    /// world of no station, and a series that measures no structure has its value in both.
    #[test]
    fn a_series_has_no_value_until_the_world_has_what_it_measures() {
        let keys = ["rack_occupancy", "rack_capacity", "relay_cells_reserved", "relay_cells_total", "presentations"];
        let mut slices = Slices::new(SimDuration::ms(10_000), World { stations: 0 }, &declaring(&keys));
        let scalars = |values: [f64; 5]| ScalarsDelta { values: values.map(Some).to_vec() };
        slices.take(SimTime::ZERO, scalars([0.0, 0.0, 0.0, 0.0, 3.0]));
        slices.take(SimTime::ms(10_000), scalars([5.0, 20.0, 1.0, 4.0, 3.0]));
        slices.close(SimTime::ms(20_000));
        let value = |slice: usize, series: TimeSeries| slices.taken()[slice].values[series.index()];
        for series in [TimeSeries::RackOccupancy, TimeSeries::RackFillPercent, TimeSeries::RelayOccupancyPercent] {
            assert_eq!(value(0, series), None, "{series:?} in a world without what it measures");
        }
        assert_eq!(value(1, TimeSeries::RackOccupancy), Some(5.0));
        assert_eq!(value(1, TimeSeries::RackFillPercent), Some(25.0));
        assert_eq!(value(1, TimeSeries::RelayOccupancyPercent), Some(25.0));
        assert_eq!(value(1, TimeSeries::PresentationsPerStationHour), None, "a world of no station");
        assert_eq!([value(0, TimeSeries::Presentations), value(1, TimeSeries::Presentations)], [Some(3.0); 2]);
    }
}
