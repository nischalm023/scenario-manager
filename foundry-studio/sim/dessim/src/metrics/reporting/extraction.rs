//! Completed extraction facts and their descriptor-driven count report.
//! A reusable container emptying and a container leaving are both one extraction;
//! the source cycle does not distinguish them, so this report does not invent a split.

use std::collections::BTreeSet;

use desgraph::StationId;
use serde::{Deserialize, Serialize};

use crate::SimTime;
use crate::metrics::streams::{EntryValue, EntryValueType, LogStream, StreamDescriptor, StreamError, StreamRecord};
use crate::observer::output::ExtractionRecord;

use super::{
    AggregationClass, MetricDescriptor, MetricUnit, MetricsReport, ReportBuildError, ReportRequest, SupportStatus,
    TimeBucket, event_in_bucket, report_buckets,
};

const STREAM_ID: &str = "fulfillment.extractions";

#[derive(Serialize, StreamRecord)]
struct ExtractionEntry {
    /// Physical extraction completion time.
    #[stream(desc = "Physical extraction completion time.")]
    completed_at: SimTime,
    /// The extraction presentation's station identity.
    #[stream(desc = "The extraction presentation station identity.")]
    station_id: u32,
    /// The robot holding the container during extraction.
    #[stream(desc = "The robot holding the container during extraction.")]
    robot_id: u32,
    /// Actual units removed at completion.
    #[stream(desc = "Actual units removed at completion.")]
    extracted_units: u64,
    /// Order robot trip identity; empty when the producer did not record one.
    #[stream(desc = "Order robot trip identity, empty when unavailable.")]
    order_trip_id: String,
}

pub fn extraction_stream(records: &[ExtractionRecord]) -> Result<LogStream, StreamError> {
    let mut records: Vec<_> = records.iter().collect();
    records.sort_by_key(|record| (record.sim_time_ms, record.station_id, record.robot_id, record.presentation_id));
    let entries: Vec<_> = records
        .into_iter()
        .map(|record| ExtractionEntry {
            completed_at: SimTime::ms(record.sim_time_ms),
            station_id: record.station_id.0,
            robot_id: record.robot_id.0,
            extracted_units: u64::from(record.quantity),
            order_trip_id: record.presentation_id.map(|id| id.to_string()).unwrap_or_default(),
        })
        .collect();
    LogStream::from_records(
        StreamDescriptor {
            id: STREAM_ID.into(),
            label: "Completed extractions".into(),
            description: "One physical extraction at dwell end, including reusable emptyings and whole-container removals without inferring a container split.".into(),
            columns: ExtractionEntry::entry_columns(),
            context: vec![("work".into(), "extraction".into())],
        },
        &entries,
    )
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractionRow {
    pub bucket: TimeBucket,
    /// None is the warehouse total; Some is one physical presentation.
    pub station_id: Option<StationId>,
    pub status: SupportStatus,
    pub extractions: Option<u64>,
    pub extracted_units: Option<u64>,
}

impl ExtractionRow {
    pub fn extractions_per_hour(&self) -> Option<f64> {
        self.rate(self.extractions)
    }

    pub fn extracted_units_per_hour(&self) -> Option<f64> {
        self.rate(self.extracted_units)
    }

    fn rate(&self, count: Option<u64>) -> Option<f64> {
        (self.bucket.coverage_ms > 0)
            .then(|| count.map(|count| count as f64 * 3_600_000.0 / self.bucket.coverage_ms as f64))
            .flatten()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractionData {
    pub rows: Vec<ExtractionRow>,
}

pub fn extraction_descriptors() -> Vec<MetricDescriptor> {
    [
        (
            "fulfillment.extractions",
            "Completed extractions",
            "Container presentations whose extraction dwell has completed.",
            MetricUnit::Count,
            AggregationClass::Additive,
        ),
        (
            "fulfillment.extracted_units",
            "Extracted units",
            "Already-picked units removed from order containers at extraction completion.",
            MetricUnit::Count,
            AggregationClass::Additive,
        ),
        (
            "fulfillment.extractions_per_hour",
            "Extractions/hr",
            "Completed extractions divided by the selected period's covered hours.",
            MetricUnit::RatePerHour,
            AggregationClass::Ratio,
        ),
        (
            "fulfillment.extracted_units_per_hour",
            "Extracted units/hr",
            "Extracted units divided by the selected period's covered hours.",
            MetricUnit::RatePerHour,
            AggregationClass::Ratio,
        ),
    ]
    .into_iter()
    .map(|(id, label, description, unit, aggregation)| {
        MetricDescriptor::new(
            id,
            label,
            description,
            unit,
            (unit == MetricUnit::RatePerHour).then(|| "Covered simulation hours in the selected period".into()),
            aggregation,
        )
    })
    .collect()
}

pub fn extraction_report(
    streams: &[LogStream],
    request: ReportRequest,
) -> Result<MetricsReport<ExtractionData>, ReportBuildError> {
    let mut matching = streams.iter().filter(|stream| stream.descriptor.id == STREAM_ID);
    let source = matching.next();
    if matching.next().is_some() {
        return Err(ReportBuildError::SourceStream("duplicate extraction streams".into()));
    }
    let facts = source.map(read_facts).transpose()?;
    // ordered: station rows must be stable across event emission orders.
    let stations: Vec<_> = std::iter::once(None)
        .chain(facts.iter().flatten().map(|fact| fact.1).collect::<BTreeSet<_>>().into_iter().map(Some))
        .collect();
    let mut rows = Vec::new();
    for bucket in report_buckets(request)? {
        for &station_id in &stations {
            let measured = facts.as_ref().map(|facts| {
                facts
                    .iter()
                    .filter(|&&(at, station, _)| {
                        event_in_bucket(at, bucket, request) && station_id.is_none_or(|id| id == station)
                    })
                    .fold((0, 0), |(count, units), &(_, _, qty)| (count + 1, units + qty))
            });
            rows.push(ExtractionRow {
                bucket,
                station_id,
                status: if measured.is_some() { SupportStatus::Measured } else { SupportStatus::Unknown },
                extractions: measured.map(|(count, _)| count),
                extracted_units: measured.map(|(_, units)| units),
            });
        }
    }
    Ok(MetricsReport::new(request, extraction_descriptors(), ExtractionData { rows }))
}

fn read_facts(stream: &LogStream) -> Result<Vec<(u64, StationId, u64)>, ReportBuildError> {
    let column = |name, value_type| {
        let mut matches = stream.descriptor.columns.iter().enumerate().filter(|(_, column)| column.name == name);
        let Some((index, column)) = matches.next() else {
            return Err(ReportBuildError::SourceStream(format!("extraction stream is missing {name}")));
        };
        if matches.next().is_some() || column.value_type != value_type {
            return Err(ReportBuildError::SourceStream(format!(
                "extraction column {name} is duplicate or has the wrong type"
            )));
        }
        Ok(index)
    };
    let (at, station, units) = (
        column("completed_at", EntryValueType::SimTime)?,
        column("station_id", EntryValueType::UInt)?,
        column("extracted_units", EntryValueType::UInt)?,
    );
    stream
        .entries
        .generic_rows()
        .map_err(|error| ReportBuildError::SourceStream(error.to_string()))?
        .into_iter()
        .map(|row| {
            let value = |index| match row.get(index) {
                Some(EntryValue::UInt(value)) => Ok(*value),
                _ => Err(ReportBuildError::SourceStream("extraction count/time must be unsigned".into())),
            };
            let station = u32::try_from(value(station)?)
                .map_err(|_| ReportBuildError::SourceStream("extraction station id exceeds u32".into()))?;
            Ok((value(at)?, StationId(station), value(units)?))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::metrics::reporting::{FixedPeriod, MetricScope, ReportPeriod};
    use crate::model::RobotId;

    use super::*;

    fn extraction(at: u64, station: u32, quantity: u32) -> ExtractionRecord {
        ExtractionRecord {
            sim_time_ms: at,
            station_id: StationId(station),
            robot_id: RobotId(4),
            quantity,
            presentation_id: Some(91),
        }
    }

    #[test]
    fn absent_facts_are_unknown_while_an_empty_recorded_stream_is_zero() {
        let request = ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 10_000, 0);
        let unknown = extraction_report(&[], request).unwrap();
        assert_eq!(unknown.data.rows[0].status, SupportStatus::Unknown);
        assert_eq!(unknown.data.rows[0].extractions, None);
        assert_eq!(unknown.data.rows[0].extractions_per_hour(), None);
        let empty = extraction_report(&[extraction_stream(&[]).unwrap()], request).unwrap();
        assert_eq!(empty.data.rows[0].status, SupportStatus::Measured);
        assert_eq!(empty.data.rows[0].extractions, Some(0));
        assert_eq!(empty.data.rows[0].extractions_per_hour(), Some(0.0));
    }

    #[test]
    fn extraction_counts_and_rates_use_completion_buckets_and_covered_time() {
        let stream = extraction_stream(&[extraction(15_000, 2, 3), extraction(10_000, 1, 2)]).unwrap();
        let request = ReportRequest::for_run(
            MetricScope::FullRun,
            ReportPeriod::Fixed(FixedPeriod::new(10_000, 0).unwrap()),
            15_000,
            0,
        );
        let report = extraction_report(&[stream], request).unwrap();
        let total: Vec<_> = report.data.rows.iter().filter(|row| row.station_id.is_none()).collect();
        assert_eq!(total.len(), 2);
        assert_eq!(total[0].extractions, Some(0));
        assert_eq!(total[1].extractions, Some(2));
        assert_eq!(total[1].extracted_units, Some(5));
        assert_eq!(total[1].extractions_per_hour(), Some(1440.0));
        assert_eq!(total[1].extracted_units_per_hour(), Some(3600.0));
    }

    #[test]
    fn descriptor_identity_is_validated_instead_of_decoding_a_known_struct() {
        let mut stream = extraction_stream(&[extraction(1, 2, 3)]).unwrap();
        stream.descriptor.columns[0].name = "some_other_time".into();
        let request = ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 10, 0);
        assert!(matches!(extraction_report(&[stream], request), Err(ReportBuildError::SourceStream(_))));
    }
    #[test]
    fn schema_and_completion_journal_are_truthful() {
        let rows = [extraction(20, 2, 3), extraction(10, 1, 1)];
        let stream = extraction_stream(&rows).unwrap();
        let decoded = stream.entries.generic_rows().unwrap();
        assert_eq!(decoded[0][0], EntryValue::UInt(10));
        assert_eq!(decoded[1][0], EntryValue::UInt(20));
        let request = ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 20, 0);
        let mut wrong = stream.clone();
        wrong.descriptor.columns[0].value_type = EntryValueType::SimDuration;
        assert!(extraction_report(&[wrong], request).is_err());
        let mut duplicate = stream;
        duplicate.descriptor.columns.push(duplicate.descriptor.columns[0].clone());
        assert!(extraction_report(&[duplicate], request).is_err());
    }
}
