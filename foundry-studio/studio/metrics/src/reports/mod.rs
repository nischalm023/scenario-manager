//! Within `studio-metrics`, `reports` is THE REPORT TABS COMPUTED FROM THE JOURNALS AS THE RUN GOES: the series of
//! desvis's DA50 throughput, Buffer & storage and Aisle distribution tabs, each built by the same builder desvis and the
//! workbook call (`dessim::metrics::reporting`), one trailing window at a time, and once more over the whole run at its
//! end, from the journal rows `journals` keeps since the run's start, each window handed only the records it reads. A
//! window is built once it is final: the follower has taken the slice past its end. Rows carry their dimensions as
//! keys, in the order the descriptor names them.

mod journals;

use dessim::events::VtmOperationKind;
use dessim::metrics::reporting::{
    Da50PayloadGroup, MetricScope, Ratio, ReportBuildError, ReportPeriod, ReportRange, ReportRequest, SupportStatus,
    TrailingSpec as ReportTrailing, aisle_occupancy_report, completed_work_report, da50_throughput_report,
};
use dessim::metrics::worldview::journals::{AISLE_STATES, COMPLETED_ORDERS, PICKS, VTM_OPERATIONS};
use dessim::observer::output::AisleStateRecord;
use desunits::{SimDuration, SimTime};
use studio_protocol::{
    Aggregation, Bucket, Dimension, MetricDescriptor, MetricPoint, MetricSource, MetricUnit, TrailingSpec,
};

use crate::follower::{Taken, Unreadable, Unrecorded, Unservable};

pub(crate) use journals::Journals;
use journals::{Journal, Record};

/// One report series: a tab's figures along one or more dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReportSeries {
    Da50Operations,
    StorageOccupied,
    StorageEmptyTote,
    StorageNonEmptyTote,
    RelayPositionsOccupied,
    PresentationsByAisleStation,
    OrdersByDcStation,
}

/// desvis's default report window: fifteen minutes at a five-minute step.
const DEFAULT_WINDOW: SimDuration = SimDuration::ms(900_000);
const DEFAULT_STEP: SimDuration = SimDuration::ms(300_000);

impl ReportSeries {
    pub const ALL: [ReportSeries; 7] = [
        Self::Da50Operations,
        Self::StorageOccupied,
        Self::StorageEmptyTote,
        Self::StorageNonEmptyTote,
        Self::RelayPositionsOccupied,
        Self::PresentationsByAisleStation,
        Self::OrdersByDcStation,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Da50Operations => "da50_operations",
            Self::StorageOccupied => "storage_slots_occupied",
            Self::StorageEmptyTote => "storage_slots_empty_tote",
            Self::StorageNonEmptyTote => "storage_slots_non_empty_tote",
            Self::RelayPositionsOccupied => "relay_positions_occupied",
            Self::PresentationsByAisleStation => "presentations_by_aisle_station",
            Self::OrdersByDcStation => "orders_by_dc_station",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Da50Operations => "DA50 operations",
            Self::StorageOccupied => "Storage slots occupied, % by aisle",
            Self::StorageEmptyTote => "Empty totes in storage, % by aisle",
            Self::StorageNonEmptyTote => "Laden totes in storage, % by aisle",
            Self::RelayPositionsOccupied => "Relay positions occupied, % by aisle",
            Self::PresentationsByAisleStation => "Tote presentations by aisle and station",
            Self::OrdersByDcStation => "Completed orders by DC and station",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Da50Operations => {
                "Physical pickups and drops by aisle, endpoint and tote payload in the window; built from the DA50 operation journal."
            }
            Self::StorageOccupied => {
                "Slot-time holding any tote over the aisle's capacity slot-time in the window; built from the aisle state journal."
            }
            Self::StorageEmptyTote => {
                "Slot-time holding an empty container over the aisle's capacity slot-time in the window."
            }
            Self::StorageNonEmptyTote => {
                "Slot-time holding a laden container over the aisle's capacity slot-time in the window."
            }
            Self::RelayPositionsOccupied => {
                "Diving-board position-time holding a tote over the aisle's position-time in the window."
            }
            Self::PresentationsByAisleStation => {
                "Distinct completed tote presentations by source aisle and pick station in the window; built from the pick journal."
            }
            Self::OrdersByDcStation => {
                "Distinct completed orders by distribution centre and pick station in the window; built from the completed-order journal."
            }
        }
    }

    fn dimensions(self) -> Vec<Dimension> {
        let dimension =
            |name: &str, description: &str| Dimension { name: name.to_owned(), description: description.to_owned() };
        match self {
            Self::Da50Operations => vec![
                dimension("aisle", "The aisle's zone id, or `all` for the fleet."),
                dimension("operation", "Pickup or drop, at storage or a relay."),
                dimension("payload", "What the tote held: all, empty, non-empty, unknown."),
            ],
            Self::StorageOccupied
            | Self::StorageEmptyTote
            | Self::StorageNonEmptyTote
            | Self::RelayPositionsOccupied => {
                vec![dimension("aisle", "The aisle's zone id.")]
            }
            Self::PresentationsByAisleStation => {
                vec![dimension("aisle", "The source aisle's zone id."), dimension("station", "The pick station.")]
            }
            Self::OrdersByDcStation => {
                vec![dimension("dc", "The distribution centre."), dimension("station", "The pick station.")]
            }
        }
    }

    fn unit(self) -> (MetricUnit, Aggregation, Option<&'static str>) {
        match self {
            Self::Da50Operations => (MetricUnit::Count, Aggregation::Additive, None),
            Self::StorageOccupied | Self::StorageEmptyTote | Self::StorageNonEmptyTote => {
                (MetricUnit::Ratio, Aggregation::Ratio, Some("percent of the aisle's capacity slot-time"))
            }
            Self::RelayPositionsOccupied => {
                (MetricUnit::Ratio, Aggregation::Ratio, Some("percent of the aisle's diving-board position-time"))
            }
            Self::PresentationsByAisleStation | Self::OrdersByDcStation => {
                (MetricUnit::Count, Aggregation::DistinctCount, None)
            }
        }
    }

    /// The journal tables the series is built from, which a recording must declare.
    fn tables(self) -> &'static [&'static str] {
        match self {
            Self::Da50Operations => &[VTM_OPERATIONS],
            Self::StorageOccupied
            | Self::StorageEmptyTote
            | Self::StorageNonEmptyTote
            | Self::RelayPositionsOccupied => &[AISLE_STATES],
            Self::PresentationsByAisleStation | Self::OrdersByDcStation => &[PICKS, COMPLETED_ORDERS],
        }
    }

    /// Whether the journals the series reads still read; one that does not fails the series' views alone.
    fn reads(self, journals: &Journals) -> Result<(), Unservable> {
        self.tables().iter().try_for_each(|table| journals.reads(table))
    }
}

/// Which of a journal's records a report reads: a trailing window's, beginning at its start, or the whole run's.
#[derive(Clone, Copy)]
enum Read {
    Window(u64),
    Whole,
}

impl Read {
    fn of<T: Record>(self, journal: &Journal<T>, end: u64) -> Result<Vec<T>, Unservable> {
        match self {
            Self::Window(start) => journal.window(start, end),
            Self::Whole => journal.by(end).map(<[T]>::to_vec),
        }
    }

    fn aisle_states(self, journals: &Journals, end: u64) -> Result<Vec<AisleStateRecord>, Unservable> {
        match self {
            Self::Window(start) => journals.aisle_states.window(start, end),
            Self::Whole => journals.aisle_states.by(end),
        }
    }
}

/// The series' descriptor, as served in `bucket`.
pub fn descriptor(series: ReportSeries, bucket: Bucket) -> MetricDescriptor {
    let (unit, aggregation, denominator) = series.unit();
    MetricDescriptor {
        metric: series.name().to_owned(),
        label: series.label().to_owned(),
        description: series.description().to_owned(),
        unit,
        clock: None,
        aggregation,
        denominator: denominator.map(str::to_owned),
        source: MetricSource::Journals,
        bucket,
        dimensions: series.dimensions(),
    }
}

/// The window a report tab is served in unless asked otherwise.
pub(crate) fn default_trailing() -> TrailingSpec {
    TrailingSpec { window_sim_ms: DEFAULT_WINDOW, step_sim_ms: DEFAULT_STEP }
}

/// One report series as one ask cuts it: its windows as each becomes final, then the whole run at its end.
pub(crate) struct ReportView {
    series: ReportSeries,
    spec: TrailingSpec,
    /// The end of the next window to build.
    next_end: u64,
}

impl ReportView {
    /// `series` in the trailing windows `asked`, or desvis's report window, which must be whole slices of a recording
    /// that declares the series' tables.
    pub(crate) fn opened(series: ReportSeries, asked: Option<TrailingSpec>, taken: &Taken) -> Result<Self, Unservable> {
        if let Some(table) = series.tables().iter().find(|table| !taken.journals.declares(table)) {
            return Err(Unservable::NotRecorded(Unrecorded::Table(table)));
        }
        series.reads(&taken.journals)?;
        let width = taken.slices.width();
        let spec = asked.unwrap_or_else(default_trailing);
        if !spec.fits(width) {
            return Err(Unservable::UnsupportedTrailing(spec));
        }
        let (window, step) = (spec.window_sim_ms.as_ms(), spec.step_sim_ms.as_ms());
        let next_end = window.div_ceil(step) * step;
        Ok(Self { series, spec, next_end })
    }

    pub(crate) fn descriptor(&self) -> MetricDescriptor {
        descriptor(self.series, Bucket::Trailing(self.spec))
    }

    /// Every window made final since the view last advanced, then, once the end block has been taken, the whole run.
    pub(crate) fn advance(&mut self, taken: &Taken) -> Result<Vec<MetricPoint>, Unservable> {
        let journals = &taken.journals;
        self.series.reads(journals)?;
        // Every window ending by the run's end is final once the end block, stamped there, has been taken.
        let mut rows = self.due(journals, taken.reached.as_ms())?;
        if let Some(ended) = &taken.ended {
            rows.extend(self.whole_run(journals, ended.run_end.as_ms())?);
        }
        Ok(rows)
    }

    /// Every window final with the recording read to `reached`: the follower has taken a frame at or past its end.
    fn due(&mut self, journals: &Journals, reached: u64) -> Result<Vec<MetricPoint>, Unservable> {
        let mut rows = Vec::new();
        while is_final(self.next_end, reached) {
            let end = self.next_end;
            let start = end - self.spec.window_sim_ms.as_ms();
            let trailing = ReportTrailing::new(end - start, end - start, end).map_err(Unreadable::because)?;
            let request = ReportRequest {
                scope: MetricScope::FullRun,
                range: ReportRange { start_ms: start, end_ms: end },
                period: ReportPeriod::Trailing(trailing),
            };
            rows.extend(self.build(journals, request, Read::Window(start))?);
            self.next_end = end + self.spec.step_sim_ms.as_ms();
        }
        Ok(rows)
    }

    /// The whole run, `[0, run_end]`, each row marked as the whole run's.
    fn whole_run(&self, journals: &Journals, run_end: u64) -> Result<Vec<MetricPoint>, Unservable> {
        let request = ReportRequest {
            scope: MetricScope::FullRun,
            range: ReportRange { start_ms: 0, end_ms: run_end },
            period: ReportPeriod::Overall,
        };
        let rows = self.build(journals, request, Read::Whole)?;
        Ok(rows.into_iter().map(|row| MetricPoint { whole_run: true, ..row }).collect())
    }

    /// The series' rows over `request`'s span, as its builder states them over the records dated by its end. A
    /// builder states a row for every aisle, station or distribution centre its records name, so a record dated later,
    /// already taken when a window is built late, would give that window rows it lacks when built as it becomes final.
    fn build(&self, journals: &Journals, request: ReportRequest, read: Read) -> Result<Vec<MetricPoint>, Unservable> {
        let end = request.range.end_ms;
        let fault = |fault: ReportBuildError| Unservable::from(Unreadable::because(fault));
        let point = |start: u64, end: u64, keys: Vec<String>, value: Option<f64>, ratio: Option<Ratio>| MetricPoint {
            from_sim_ms: SimTime::ms(start),
            to_sim_ms: SimTime::ms(end),
            keys,
            value,
            numerator: ratio.map(|ratio| ratio.numerator as f64),
            denominator: ratio.map(|ratio| ratio.denominator as f64),
            whole_run: false,
        };
        let percent = |status: SupportStatus, ratio: Option<Ratio>| match status {
            SupportStatus::Measured => ratio.and_then(Ratio::percent),
            SupportStatus::Unknown | SupportStatus::Unsupported => None,
        };
        Ok(match self.series {
            ReportSeries::Da50Operations => da50_throughput_report(&read.of(&journals.vtm_operations, end)?, request)
                .map_err(fault)?
                .data
                .rows
                .into_iter()
                .map(|row| {
                    let aisle = row.zone_id.map_or_else(|| "all".to_owned(), |zone| zone.to_string());
                    let keys =
                        vec![aisle, operation_words(row.operation).to_owned(), payload_words(row.payload).to_owned()];
                    point(row.bucket.start_ms, row.bucket.end_ms, keys, Some(row.count as f64), None)
                })
                .collect(),
            ReportSeries::StorageOccupied
            | ReportSeries::StorageEmptyTote
            | ReportSeries::StorageNonEmptyTote
            | ReportSeries::RelayPositionsOccupied => {
                aisle_occupancy_report(&read.aisle_states(journals, end)?, request)
                    .map_err(fault)?
                    .data
                    .rows
                    .into_iter()
                    .map(|row| {
                        let ratio = match self.series {
                            ReportSeries::StorageOccupied => row.storage_occupied,
                            ReportSeries::StorageEmptyTote => row.storage_empty,
                            ReportSeries::StorageNonEmptyTote => row.storage_non_empty,
                            _ => row.diving_board_occupied,
                        };
                        point(
                            row.bucket.start_ms,
                            row.bucket.end_ms,
                            vec![row.zone_id.to_string()],
                            percent(row.status, ratio),
                            ratio,
                        )
                    })
                    .collect()
            }
            ReportSeries::PresentationsByAisleStation => completed_work_report(
                &read.of(&journals.picks, end)?,
                &read.of(&journals.completed_orders, end)?,
                request,
            )
            .map_err(fault)?
            .data
            .aisle_station_rows
            .into_iter()
            .map(|row| {
                point(
                    row.bucket.start_ms,
                    row.bucket.end_ms,
                    vec![row.zone_id.to_string(), row.station_id.to_string()],
                    Some(row.tote_count as f64),
                    None,
                )
            })
            .collect(),
            ReportSeries::OrdersByDcStation => completed_work_report(
                &read.of(&journals.picks, end)?,
                &read.of(&journals.completed_orders, end)?,
                request,
            )
            .map_err(fault)?
            .data
            .dc_station_rows
            .into_iter()
            .map(|row| {
                point(
                    row.bucket.start_ms,
                    row.bucket.end_ms,
                    vec![row.dc_id.clone(), row.station_id.to_string()],
                    Some(row.order_count as f64),
                    None,
                )
            })
            .collect(),
        })
    }
}

/// Whether a window ending at `end` is final with the recording read to `reached`, the instant of the newest slice or
/// end block taken: the follower has taken the slice that begins at the window's end, with the rows filed in it.
fn is_final(end: u64, reached: u64) -> bool {
    reached >= end
}

/// An operation's key, in desvis's words (`metrics_panel.rs`, `operation_label`).
fn operation_words(kind: VtmOperationKind) -> &'static str {
    match kind {
        VtmOperationKind::PickupStorage => "Pickup from storage",
        VtmOperationKind::DropRelay => "Drop to relay",
        VtmOperationKind::PickupRelay => "Pickup from relay",
        VtmOperationKind::DropStorage => "Drop to storage",
    }
}

/// A payload's key, in desvis's words (`MetricsPayloadSelection::label`).
fn payload_words(group: Da50PayloadGroup) -> &'static str {
    match group {
        Da50PayloadGroup::All => "All totes",
        Da50PayloadGroup::Empty => "Empty totes",
        Da50PayloadGroup::NonEmpty => "Non-empty totes",
        Da50PayloadGroup::Unknown => "Unknown contents",
    }
}

#[cfg(test)]
mod tests {
    use rustc_hash::FxHashSet;

    use desgraph::StationId;
    use dessim::events::TotePayloadState;
    use dessim::metrics::reporting::{DA50_OPERATION_KINDS, DA50_PAYLOAD_GROUPS};
    use dessim::model::{OrderId, SkuId};
    use dessim::observer::output::{CompletedOrderRecord, PickRecord, VtmOperationRecord};

    use super::*;

    #[test]
    fn seven_series_each_named_once_with_their_dimensions() {
        let names: FxHashSet<&str> = ReportSeries::ALL.iter().map(|series| series.name()).collect();
        assert_eq!(names.len(), 7);
        for series in ReportSeries::ALL {
            let described = descriptor(series, Bucket::Trailing(default_trailing()));
            assert!(!described.dimensions.is_empty(), "{series:?} has a dimension");
            assert_eq!(described.source, MetricSource::Journals);
        }
        assert_eq!(descriptor(ReportSeries::Da50Operations, Bucket::Overall).dimensions.len(), 3);
    }

    /// A window is final once the follower has taken the slice that begins at its end, and not before.
    #[test]
    fn a_window_is_final_once_the_slice_past_its_end_is_taken() {
        assert!(!is_final(900_000, 890_000), "the slice beginning at the window's end is not taken yet");
        assert!(is_final(900_000, 900_000));
    }

    /// A window is handed only the records it reads, yet states the rows its builder states over every record dated by
    /// its end, however late it is built and in whatever order the records came: an aisle, station or centre named
    /// before the window has its rows though nothing happens there inside it, one named only after the window has
    /// none, and an aisle's occupancy integrates from the state in force as the window begins. Records lie on window
    /// edges, and two come out of simtime order.
    #[test]
    fn a_window_reads_only_its_own_records_and_states_what_every_record_dated_by_its_end_states() {
        let mut journals = Journals::new();
        let operations = [(1, 100_000), (3, 900_000), (1, 1_100_000), (3, 950_000), (2, 1_900_000), (1, 1_200_000)];
        journals.vtm_operations.take(operations.map(|(aisle, at)| operation(aisle, at)).to_vec());
        let states = [(1, 0, 4), (2, 0, 1), (1, 600_000, 5), (2, 1_200_000, 2), (1, 1_200_000, 3), (4, 1_500_000, 1)];
        journals.aisle_states.take(states.map(|(aisle, at, occupied)| state(aisle, at, occupied)).to_vec());
        let picks = [(7, 1, "A", 0), (8, 2, "B", 900_000), (7, 2, "A", 1_150_000), (9, 3, "C", 1_850_000)];
        journals.picks.take(picks.map(|(station, aisle, centre, at)| pick(station, aisle, centre, at)).to_vec());
        let orders = [(7, "A", 300_000), (8, "D", 1_200_000), (9, "C", 1_900_000)];
        journals.completed_orders.take(orders.map(|(station, centre, at)| order(station, centre, at)).to_vec());

        for series in ReportSeries::ALL {
            let view = ReportView { series, spec: default_trailing(), next_end: 0 };
            for end in (900_000..=2_100_000).step_by(300_000) {
                let start = end - 900_000;
                let request = ReportRequest {
                    scope: MetricScope::FullRun,
                    range: ReportRange { start_ms: start, end_ms: end },
                    period: ReportPeriod::Trailing(ReportTrailing::new(900_000, 900_000, end).expect("a window")),
                };
                let read = view.build(&journals, request, Read::Window(start)).expect("built");
                let every = view.build(&journals, request, Read::Whole).expect("built");
                assert!(!read.is_empty(), "{series:?} to {end}");
                assert_eq!(read, every, "{series:?} over ({start}, {end}]");
            }
        }
        // Every record dated by an end, counted here apart from the journals: the records on window edges included.
        for end in [900_000, 1_200_000, 1_900_000] {
            let by = |dates: &[u64]| dates.iter().filter(|at| **at <= end).count();
            let held = journals.vtm_operations.by(end).expect("reads").len();
            assert_eq!(held, by(&operations.map(|(_, at)| at)), "operations by {end}");
            let states_by = journals.aisle_states.by(end).expect("reads").len();
            assert_eq!(states_by, by(&states.map(|(_, at, _)| at)), "aisle states by {end}");
            let orders_by = journals.completed_orders.by(end).expect("reads").len();
            assert_eq!(orders_by, by(&orders.map(|(_, _, at)| at)), "orders by {end}");
        }
        let first = ReportView { series: ReportSeries::Da50Operations, spec: default_trailing(), next_end: 0 };
        let request = ReportRequest {
            scope: MetricScope::FullRun,
            range: ReportRange { start_ms: 0, end_ms: 900_000 },
            period: ReportPeriod::Trailing(ReportTrailing::new(900_000, 900_000, 900_000).expect("a window")),
        };
        let rows = first.build(&journals, request, Read::Window(0)).expect("built");
        assert!(rows.iter().all(|row| row.keys[0] != "2"), "aisle 2 is named after the first window: {rows:?}");
    }

    /// The DA50 operations rows are keyed in the words desvis's metrics panel labels the same rows with
    /// (`operation_label`, `MetricsPayloadSelection::label`), so a key names one row in both.
    #[test]
    fn the_operations_keys_are_desviss_words() {
        let operations = DA50_OPERATION_KINDS.map(operation_words);
        assert_eq!(operations, ["Pickup from storage", "Drop to relay", "Pickup from relay", "Drop to storage"]);
        let payloads = DA50_PAYLOAD_GROUPS.map(payload_words);
        assert_eq!(payloads, ["All totes", "Empty totes", "Non-empty totes", "Unknown contents"]);
    }

    fn state(zone_id: u32, sim_time_ms: u64, occupied: u32) -> AisleStateRecord {
        AisleStateRecord {
            sim_time_ms,
            zone_id,
            storage_capacity: 10,
            storage_occupied: occupied,
            storage_empty: 0,
            storage_non_empty: occupied,
            storage_unknown: 0,
            diving_board_capacity: 2,
            diving_board_occupied: occupied % 3,
        }
    }

    fn pick(station: u32, aisle: u32, centre: &str, sim_time_ms: u64) -> PickRecord {
        PickRecord {
            sim_time_ms,
            station_id: StationId(station),
            sku: SkuId(1),
            quantity: 1,
            dc_id: Some(centre.to_owned()),
            order_id: None,
            tote_id: None,
            source_zone_id: Some(aisle),
            source_relay_id: None,
            presentation_id: Some(sim_time_ms),
        }
    }

    fn order(station: u32, centre: &str, sim_time_ms: u64) -> CompletedOrderRecord {
        CompletedOrderRecord {
            sim_time_ms,
            order_id: OrderId(u32::try_from(sim_time_ms / 1_000).expect("small")),
            station_id: Some(StationId(station)),
            dc_id: Some(centre.to_owned()),
        }
    }

    fn operation(zone_id: u32, sim_time_ms: u64) -> VtmOperationRecord {
        VtmOperationRecord {
            sim_time_ms,
            vtm_id: 0,
            vtm_cycle_id: 0,
            zone_id,
            kind: VtmOperationKind::PickupStorage,
            rack_id: 0,
            rack_level: None,
            relay_id: 0,
            tote_id: 0,
            payload: TotePayloadState::Empty,
            presentation_id: None,
        }
    }
}
