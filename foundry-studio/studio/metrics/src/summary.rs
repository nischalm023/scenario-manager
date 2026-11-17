//! Within `studio-metrics`, `summary` is THE METRICS ONLY THE END BLOCK STATES: desvis's finished Status rows, read
//! from the summary statistics the simulator's harness computes as the run ends; its Order extraction tab, built
//! from the extraction stream, which rides the end block whole; and its Destinations tab's presentations by
//! destination, built from the stripe plan the summary carries and the pick journal. Each series has one bucket,
//! the whole run, and nothing before the end block comes. The same figures are the ones simrunner writes into an
//! attempt's `metrics.json` (`figures`), so a run's summary page and its streams never disagree.

use dessim::metrics::profile::SummaryStats;
use dessim::metrics::reporting::{
    MetricScope, ReportPeriod, ReportRange, ReportRequest, SupportStatus, destination_striping_report,
    extraction_report,
};
use dessim::metrics::worldview::journals::{EXTRACTIONS, PICKS};
use desunits::SimTime;
use studio_protocol::{
    Aggregation, Bucket, Dimension, DurationClock, MetricDescriptor, MetricPoint, MetricSource, MetricUnit,
    TrailingSpec,
};

use crate::follower::{Taken, Unreadable, Unrecorded, Unservable};
use crate::reports::Journals;

/// One of the end block's series.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SummarySeries {
    TotalPicks,
    ThroughputPicksPerHour,
    TotalOrders,
    TotalPutaways,
    ThroughputPutawayItemsPerHour,
    Da50IdlePercent,
    D30IdlePercent,
    FlowScore,
    AverageStationUtilizationPercent,
    PathfinderFailureRate,
    EventCount,
    WallClockMs,
    SimTimeReachedMs,
    D30SpeedLoaded,
    D30SpeedEmpty,
    D30SpeedCombined,
    StationUtilizationPercent,
    StationPickedUnitsPerHour,
    ExtractionsByStation,
    ExtractedUnitsByStation,
    ExtractionsPerHourByStation,
    PresentationsByDestination,
}

/// One figure of the summary as a declared metric: what simrunner writes and the stream sends.
#[derive(Debug, Clone, PartialEq)]
pub struct Figure {
    pub metric: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub unit: MetricUnit,
    pub clock: Option<DurationClock>,
    pub aggregation: Aggregation,
    pub denominator: Option<&'static str>,
    pub value: f64,
}

impl SummarySeries {
    pub const ALL: [SummarySeries; 22] = [
        Self::TotalPicks,
        Self::ThroughputPicksPerHour,
        Self::TotalOrders,
        Self::TotalPutaways,
        Self::ThroughputPutawayItemsPerHour,
        Self::Da50IdlePercent,
        Self::D30IdlePercent,
        Self::FlowScore,
        Self::AverageStationUtilizationPercent,
        Self::PathfinderFailureRate,
        Self::EventCount,
        Self::WallClockMs,
        Self::SimTimeReachedMs,
        Self::D30SpeedLoaded,
        Self::D30SpeedEmpty,
        Self::D30SpeedCombined,
        Self::StationUtilizationPercent,
        Self::StationPickedUnitsPerHour,
        Self::ExtractionsByStation,
        Self::ExtractedUnitsByStation,
        Self::ExtractionsPerHourByStation,
        Self::PresentationsByDestination,
    ];

    /// The sixteen one-figure series, in order: the Status rows simrunner also writes into `metrics.json`.
    pub const FIGURES: [SummarySeries; 16] = [
        Self::TotalPicks,
        Self::ThroughputPicksPerHour,
        Self::TotalOrders,
        Self::TotalPutaways,
        Self::ThroughputPutawayItemsPerHour,
        Self::Da50IdlePercent,
        Self::D30IdlePercent,
        Self::FlowScore,
        Self::AverageStationUtilizationPercent,
        Self::PathfinderFailureRate,
        Self::EventCount,
        Self::WallClockMs,
        Self::SimTimeReachedMs,
        Self::D30SpeedLoaded,
        Self::D30SpeedEmpty,
        Self::D30SpeedCombined,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::TotalPicks => "run_total_picks",
            Self::ThroughputPicksPerHour => "run_throughput_picks_per_hour",
            Self::TotalOrders => "run_total_orders",
            Self::TotalPutaways => "run_total_putaways",
            Self::ThroughputPutawayItemsPerHour => "run_throughput_putaway_items_per_hour",
            Self::Da50IdlePercent => "run_da50_idle_percent",
            Self::D30IdlePercent => "run_d30_idle_percent",
            Self::FlowScore => "run_flow_score",
            Self::AverageStationUtilizationPercent => "run_station_utilization_percent",
            Self::PathfinderFailureRate => "run_pathfinder_failure_rate",
            Self::EventCount => "run_event_count",
            Self::WallClockMs => "run_wall_clock",
            Self::SimTimeReachedMs => "run_sim_time",
            Self::D30SpeedLoaded => "run_d30_speed_loaded",
            Self::D30SpeedEmpty => "run_d30_speed_empty",
            Self::D30SpeedCombined => "run_d30_speed_combined",
            Self::StationUtilizationPercent => "station_utilization_percent",
            Self::StationPickedUnitsPerHour => "station_picked_units_per_hour",
            Self::ExtractionsByStation => "extractions_by_station",
            Self::ExtractedUnitsByStation => "extracted_units_by_station",
            Self::ExtractionsPerHourByStation => "extractions_per_hour_by_station",
            Self::PresentationsByDestination => "presentations_by_destination",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::TotalPicks => "Picks, whole run",
            Self::ThroughputPicksPerHour => "Throughput, picks/hr after the embargo",
            Self::TotalOrders => "Fully picked orders, whole run",
            Self::TotalPutaways => "Inducted units, whole run",
            Self::ThroughputPutawayItemsPerHour => "Inducted units/hr, whole run",
            Self::Da50IdlePercent => "DA50 idle %",
            Self::D30IdlePercent => "D30 idle %",
            Self::FlowScore => "Flow score",
            Self::AverageStationUtilizationPercent => "Average station utilization %",
            Self::PathfinderFailureRate => "Pathfinder failure %",
            Self::EventCount => "Event count",
            Self::WallClockMs => "Wall clock, whole run",
            Self::SimTimeReachedMs => "Simtime reached",
            Self::D30SpeedLoaded => "D30 effective speed, loaded (m/s)",
            Self::D30SpeedEmpty => "D30 effective speed, empty (m/s)",
            Self::D30SpeedCombined => "D30 effective speed, combined (m/s)",
            Self::StationUtilizationPercent => "Station utilization %, by station",
            Self::StationPickedUnitsPerHour => "Picked units/hr after the embargo, by station",
            Self::ExtractionsByStation => "Completed order extractions, by station",
            Self::ExtractedUnitsByStation => "Extracted units, by station",
            Self::ExtractionsPerHourByStation => "Order extractions/hr, by station",
            Self::PresentationsByDestination => "Picks by destination",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::TotalPicks => "Picks completed over the whole run, as the harness counted them at the end.",
            Self::ThroughputPicksPerHour => "Picks per simulated hour over the post-embargo window.",
            Self::TotalOrders => "Orders fully picked over the whole run.",
            Self::TotalPutaways => "Putaway units completed over the whole run, quantity-weighted.",
            Self::ThroughputPutawayItemsPerHour => "Putaway units per simulated hour over the whole run.",
            Self::Da50IdlePercent => {
                "The DA50 fleet's time in no duty, as a percentage of its time, time-weighted over the run."
            }
            Self::D30IdlePercent => {
                "The D30 fleet's time in no duty, as a percentage of its time, time-weighted over the run."
            }
            Self::FlowScore => "The harness's flow score for the run.",
            Self::AverageStationUtilizationPercent => {
                "Each station's operator-busy time as a percentage of its simtime from its first work to the run's end, averaged over the stations."
            }
            Self::PathfinderFailureRate => "Pathfinder queries that failed, as a percentage of queries.",
            Self::EventCount => "Events the simulator processed.",
            Self::WallClockMs => "Wall clock the run took; not simtime.",
            Self::SimTimeReachedMs => "The simtime the run reached.",
            Self::D30SpeedLoaded => "Distance-weighted mean D30 speed while carrying a tote, metres per second.",
            Self::D30SpeedEmpty => "Distance-weighted mean D30 speed while empty, metres per second.",
            Self::D30SpeedCombined => {
                "Distance-weighted mean D30 speed over all its travel, carrying a tote or empty, metres per second."
            }
            Self::StationUtilizationPercent => {
                "Each station's operator-busy time as a percentage of its simtime from its first work to the run's end."
            }
            Self::StationPickedUnitsPerHour => {
                "Each station's picked units per simulated hour over the post-embargo window, as the harness counted them."
            }
            Self::ExtractionsByStation => {
                "Container presentations whose extraction completed, by station and for the warehouse (`all`)."
            }
            Self::ExtractedUnitsByStation => "Units extracted, by station and for the warehouse (`all`).",
            Self::ExtractionsPerHourByStation => {
                "Completed extractions per simulated hour of covered time, by station and for the warehouse (`all`)."
            }
            Self::PresentationsByDestination => {
                "Pick presentations by the destination the stripe plan assigned them, over the whole run."
            }
        }
    }

    fn unit(self) -> (MetricUnit, Aggregation, Option<&'static str>) {
        match self {
            Self::TotalPicks | Self::TotalOrders | Self::TotalPutaways | Self::EventCount => {
                (MetricUnit::Count, Aggregation::Additive, None)
            }
            Self::ThroughputPicksPerHour
            | Self::ThroughputPutawayItemsPerHour
            | Self::StationPickedUnitsPerHour
            | Self::ExtractionsPerHourByStation => (MetricUnit::RatePerHour, Aggregation::SnapshotGauge, None),
            Self::Da50IdlePercent | Self::D30IdlePercent => {
                (MetricUnit::Ratio, Aggregation::Ratio, Some("percent of the fleet's time"))
            }
            Self::AverageStationUtilizationPercent | Self::StationUtilizationPercent => {
                (MetricUnit::Ratio, Aggregation::Ratio, Some("percent of the run"))
            }
            Self::PathfinderFailureRate => {
                (MetricUnit::Ratio, Aggregation::Ratio, Some("percent of pathfinder queries"))
            }
            Self::FlowScore => (MetricUnit::Ratio, Aggregation::SnapshotGauge, None),
            Self::WallClockMs | Self::SimTimeReachedMs => (MetricUnit::DurationMs, Aggregation::Interval, None),
            Self::D30SpeedLoaded | Self::D30SpeedEmpty | Self::D30SpeedCombined => {
                (MetricUnit::Ratio, Aggregation::SnapshotGauge, Some("metres per second of simtime"))
            }
            Self::ExtractionsByStation | Self::ExtractedUnitsByStation => {
                (MetricUnit::Count, Aggregation::Additive, None)
            }
            Self::PresentationsByDestination => (MetricUnit::Count, Aggregation::DistinctCount, None),
        }
    }

    /// The clock of a duration or of a rate's hour: simtime, but for the wall clock the run took.
    fn clock(self) -> Option<DurationClock> {
        match (self, self.unit().0) {
            (Self::WallClockMs, _) => Some(DurationClock::Wall),
            (_, MetricUnit::DurationMs | MetricUnit::RatePerHour) => Some(DurationClock::Sim),
            _ => None,
        }
    }

    fn dimensions(self) -> Vec<Dimension> {
        let dimension =
            |name: &str, description: &str| Dimension { name: name.to_owned(), description: description.to_owned() };
        match self {
            Self::StationUtilizationPercent | Self::StationPickedUnitsPerHour => {
                vec![dimension("station", "The station.")]
            }
            Self::ExtractionsByStation | Self::ExtractedUnitsByStation | Self::ExtractionsPerHourByStation => {
                vec![dimension("station", "The station, or `all` for the warehouse.")]
            }
            Self::PresentationsByDestination => {
                vec![dimension("destination", "The destination the stripe plan assigned.")]
            }
            _ => Vec::new(),
        }
    }

    /// The figure of a one-figure series, read off the summary.
    fn figure_of(self, summary: &SummaryStats) -> Option<f64> {
        Some(match self {
            Self::TotalPicks => summary.total_picks as f64,
            Self::ThroughputPicksPerHour => summary.throughput_picks_per_hour,
            Self::TotalOrders => summary.total_orders_completed as f64,
            Self::TotalPutaways => summary.total_putaways as f64,
            Self::ThroughputPutawayItemsPerHour => summary.throughput_putaway_items_per_hour,
            Self::Da50IdlePercent => percent(summary.vtm_idle_pct),
            Self::D30IdlePercent => percent(summary.htm_idle_pct),
            Self::FlowScore => summary.flow_score,
            Self::AverageStationUtilizationPercent => percent(summary.avg_station_utilization_pct),
            Self::PathfinderFailureRate => percent(summary.pathfinder_failure_rate),
            Self::EventCount => summary.event_count as f64,
            Self::WallClockMs => summary.wall_clock_secs * 1000.0,
            Self::SimTimeReachedMs => summary.sim_time_secs * 1000.0,
            Self::D30SpeedLoaded => summary.htm_effective_speed_loaded_mps,
            Self::D30SpeedEmpty => summary.htm_effective_speed_empty_mps,
            Self::D30SpeedCombined => summary.htm_effective_speed_combined_mps,
            _ => return None,
        })
    }
}

/// A fraction the summary states in [0, 1] as the percentage desvis shows (its Status rows, `print_results`), which a
/// series whose denominator says "percent of" carries.
fn percent(fraction: f64) -> f64 {
    fraction * 100.0
}

/// The series' descriptor: one bucket, the whole run, from the end block.
pub fn descriptor(series: SummarySeries) -> MetricDescriptor {
    let (unit, aggregation, denominator) = series.unit();
    MetricDescriptor {
        metric: series.name().to_owned(),
        label: series.label().to_owned(),
        description: series.description().to_owned(),
        unit,
        clock: series.clock(),
        aggregation,
        denominator: denominator.map(str::to_owned),
        source: MetricSource::EndBlock,
        bucket: Bucket::Overall,
        dimensions: series.dimensions(),
    }
}

/// The sixteen Status figures of `summary` as declared metrics with their values: what simrunner writes.
pub fn figures(summary: &SummaryStats) -> Vec<Figure> {
    SummarySeries::FIGURES
        .iter()
        .filter_map(|series| {
            let (unit, aggregation, denominator) = series.unit();
            Some(Figure {
                metric: series.name(),
                label: series.label(),
                description: series.description(),
                unit,
                clock: series.clock(),
                aggregation,
                denominator,
                value: series.figure_of(summary)?,
            })
        })
        .collect()
}

/// One end-block series as an ask cuts it: nothing until the end block, then its rows over the whole run.
pub(crate) struct SummaryView {
    series: SummarySeries,
}

impl SummaryView {
    /// `series`, which takes no trailing windows, from a recording that declares the journal it reads, if any.
    pub(crate) fn opened(
        series: SummarySeries,
        asked: Option<TrailingSpec>,
        taken: &Taken,
    ) -> Result<Self, Unservable> {
        if let Some(asked) = asked {
            return Err(Unservable::UnsupportedTrailing(asked));
        }
        if let Some(table) = series.table() {
            if !taken.journals.declares(table) {
                return Err(Unservable::NotRecorded(Unrecorded::Table(table)));
            }
            taken.journals.reads(table)?;
        }
        Ok(Self { series })
    }

    pub(crate) fn descriptor(&self) -> MetricDescriptor {
        descriptor(self.series)
    }

    /// The series' rows once the end block has been taken, and nothing before.
    pub(crate) fn advance(&self, taken: &Taken) -> Result<Vec<MetricPoint>, Unservable> {
        let Some(ended) = &taken.ended else { return Ok(Vec::new()) };
        let summary = ended.summary.as_ref().map_err(Clone::clone)?;
        rows(self.series, summary, &taken.journals, ended.run_end.as_ms())
    }
}

impl SummarySeries {
    /// The journal table the series reads beside the summary, if any.
    fn table(self) -> Option<&'static str> {
        match self {
            Self::ExtractionsByStation | Self::ExtractedUnitsByStation | Self::ExtractionsPerHourByStation => {
                Some(EXTRACTIONS)
            }
            Self::PresentationsByDestination => Some(PICKS),
            _ => None,
        }
    }
}

/// The series' rows over the whole run, `[0, run_end]`, from the end block's summary and, for a series that reads
/// one, the journals, each marked as the whole run's.
fn rows(
    series: SummarySeries,
    summary: &SummaryStats,
    journals: &Journals,
    run_end: u64,
) -> Result<Vec<MetricPoint>, Unservable> {
    let point = |keys: Vec<String>, value: Option<f64>| MetricPoint {
        from_sim_ms: SimTime::ZERO,
        to_sim_ms: SimTime::ms(run_end),
        keys,
        value,
        numerator: None,
        denominator: None,
        whole_run: true,
    };
    if let Some(figure) = series.figure_of(summary) {
        return Ok(vec![point(Vec::new(), Some(figure))]);
    }
    let request = ReportRequest {
        scope: MetricScope::FullRun,
        range: ReportRange { start_ms: 0, end_ms: run_end },
        period: ReportPeriod::Overall,
    };
    Ok(match series {
        SummarySeries::StationUtilizationPercent => summary
            .station_utilization_pct
            .iter()
            .map(|(station, fraction)| point(vec![station.to_string()], Some(percent(*fraction))))
            .collect(),
        SummarySeries::StationPickedUnitsPerHour => summary
            .presentations_per_station_per_hour
            .iter()
            .map(|(station, rate)| point(vec![station.to_string()], Some(*rate)))
            .collect(),
        SummarySeries::ExtractionsByStation
        | SummarySeries::ExtractedUnitsByStation
        | SummarySeries::ExtractionsPerHourByStation => {
            let Some(stream) = journals.extractions.clone()? else { return Ok(Vec::new()) };
            extraction_report(&[stream], request)
                .map_err(Unreadable::because)?
                .data
                .rows
                .into_iter()
                .map(|row| {
                    let station = row.station_id.map_or_else(|| "all".to_owned(), |station| station.to_string());
                    let value = match (row.status, series) {
                        (SupportStatus::Measured, SummarySeries::ExtractionsByStation) => {
                            row.extractions.map(|count| count as f64)
                        }
                        (SupportStatus::Measured, SummarySeries::ExtractedUnitsByStation) => {
                            row.extracted_units.map(|count| count as f64)
                        }
                        (SupportStatus::Measured, _) => row.extractions_per_hour(),
                        _ => None,
                    };
                    point(vec![station], value)
                })
                .collect()
        }
        SummarySeries::PresentationsByDestination => {
            destination_striping_report(&summary.stripe_plan_log, journals.picks.records()?, request)
                .map_err(Unreadable::because)?
                .data
                .presentation_rows
                .into_iter()
                .map(|row| point(vec![row.destination], Some(row.presentations as f64)))
                .collect()
        }
        _ => Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use rustc_hash::FxHashSet;

    use worldview::Descriptors;
    use worldview::frames::tables::{TableDescriptor, TableId};

    use super::*;

    #[test]
    fn every_series_is_named_once_and_the_figures_read_off_a_summary() {
        let names: FxHashSet<&str> = SummarySeries::ALL.iter().map(|series| series.name()).collect();
        assert_eq!(names.len(), 22);
        let summary = SummaryStats {
            total_picks: 12,
            throughput_picks_per_hour: 240.0,
            wall_clock_secs: 1.5,
            ..SummaryStats::default()
        };
        let figures = figures(&summary);
        assert_eq!(figures.len(), 16);
        let by_name = |name: &str| figures.iter().find(|figure| figure.metric == name).map(|figure| figure.value);
        assert_eq!(by_name("run_total_picks"), Some(12.0));
        assert_eq!(by_name("run_throughput_picks_per_hour"), Some(240.0));
        assert_eq!(by_name("run_wall_clock"), Some(1500.0), "milliseconds, as a duration travels");
        let rows = rows(SummarySeries::TotalPicks, &summary, &Journals::new(), 3_600_000).expect("rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].from_sim_ms, rows[0].to_sim_ms, rows[0].value),
            (SimTime::ZERO, SimTime::ms(3_600_000), Some(12.0))
        );
        assert_eq!(descriptor(SummarySeries::StationUtilizationPercent).dimensions.len(), 1);
        assert_eq!(descriptor(SummarySeries::TotalPicks).source, MetricSource::EndBlock);
    }

    /// A recording that declares the extraction table and carries no extraction row, a run that extracted nothing,
    /// states the warehouse's extractions as none, measured, as desvis's Order extraction tab does, not no row at all.
    #[test]
    fn a_run_that_extracted_nothing_states_no_extraction_for_the_warehouse() {
        let mut journals = Journals::new();
        let table =
            TableDescriptor { id: TableId(7), name: EXTRACTIONS.to_owned(), columns: Vec::new(), instant_column: None };
        journals.describe(&Descriptors { metrics: Vec::new(), tables: vec![table] });
        for series in [SummarySeries::ExtractionsByStation, SummarySeries::ExtractedUnitsByStation] {
            let rows = rows(series, &SummaryStats::default(), &journals, 3_600_000).expect("rows");
            let stated: Vec<(Vec<String>, Option<f64>)> = rows.into_iter().map(|row| (row.keys, row.value)).collect();
            assert_eq!(stated, [(vec!["all".to_owned()], Some(0.0))], "{series:?}");
        }
    }

    /// The summary states idle time, station utilization and pathfinder failures as fractions; desvis shows each a
    /// hundred times over, as a percentage (its Status rows, `print_results`). So does every series whose denominator
    /// says it is a percentage, station by station too, and `metrics.json` with them.
    #[test]
    fn every_percentage_is_a_hundred_times_the_summarys_fraction_as_desvis_shows_it() {
        let mut summary = SummaryStats {
            vtm_idle_pct: 0.625,
            htm_idle_pct: 0.5,
            avg_station_utilization_pct: 0.375,
            pathfinder_failure_rate: 0.0625,
            ..SummaryStats::default()
        };
        summary.station_utilization_pct = serde_json::from_str("[[1, 0.25], [2, 0.5]]")
            .expect("station ids and fractions, as a recording stores them");
        let stated = |series| -> Vec<f64> {
            let rows = rows(series, &summary, &Journals::new(), 3_600_000).expect("rows");
            rows.iter().map(|row| row.value.expect("a value")).collect()
        };
        assert_eq!(stated(SummarySeries::Da50IdlePercent), [62.5]);
        assert_eq!(stated(SummarySeries::D30IdlePercent), [50.0]);
        assert_eq!(stated(SummarySeries::AverageStationUtilizationPercent), [37.5]);
        assert_eq!(stated(SummarySeries::PathfinderFailureRate), [6.25]);
        assert_eq!(stated(SummarySeries::StationUtilizationPercent), [25.0, 50.0]);
        let percentages: Vec<SummarySeries> = SummarySeries::ALL
            .into_iter()
            .filter(|series| series.unit().2.is_some_and(|denominator| denominator.starts_with("percent of")))
            .collect();
        assert_eq!(percentages.len(), 5, "every percentage is checked above: {percentages:?}");
        let written = figures(&summary);
        let in_metrics_json =
            |name: &str| written.iter().find(|figure| figure.metric == name).map(|figure| figure.value);
        assert_eq!(in_metrics_json("run_da50_idle_percent"), Some(62.5));
        assert_eq!(in_metrics_json("run_station_utilization_percent"), Some(37.5));
    }
}
