//! Self-describing data streams: one entry representation, per-family
//! description.
//!
//! The protocol defines how to DESCRIBE a stream and how to REPRESENT its
//! entries; it never enumerates which streams exist. Producers — observers,
//! task implementations, resource types, report builders — emit streams, and
//! consumers discover the set of streams from what was emitted. Any run-level
//! index over streams is derived data, never a second source of truth.
//!
//! Every family shares one entry representation: concatenated MessagePack row
//! messages (one array per row, elements matching the declared columns by
//! position, no outer container header — see [`Entries`]). What
//! distinguishes families is their description:
//!
//! Every stream carries the same descriptor core ([`StreamDescriptor`]:
//! identity, column schema, open-world context); a family adds a semantic
//! extension only when it genuinely carries extra meaning:
//!
//! - [`LogStream`]: fact journals — the base family, core only.
//! - [`MetricStream`]: metric samples — core plus [`MetricSemantics`]
//!   (unit, aggregation class, denominator). Time series are rows per
//!   bucket; an overall scalar is one row whose bucket spans the run.
//!
//! A stream's family is structural — the wrapping type in memory, the frame
//! tag in the container — never a data field a descriptor could drift on.
//!
//! Producers author entries as plain Rust structs; the WRITE path is typed.
//! The universal READ path is dynamic: consumers build an in-memory table
//! from the descriptor and [`Entries::generic_rows`], never reconstituting
//! producer structs — a reader must render streams whose record types it has
//! never heard of. Typed read-back ([`Entries::records`]) exists for the
//! producing binary's own replay/tests, not as the consumer contract.
//!
//! Appending rows is byte concatenation, a live transport chunk splices into
//! the at-rest representation unchanged, and rows decode lazily.

use std::marker::PhantomData;

use rmp_serde::Deserializer;
use rmp_serde::decode::ReadReader;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::descriptor::MetricSemantics;

pub use dessim_stream_macros::StreamRecord;

/// A record type that is the single source of truth for its stream's column
/// schema. Derive it — `#[derive(StreamRecord)]` generates `entry_columns()`
/// from the struct's fields (names from field names, value types inferred or
/// declared, descriptions required per field) — so the descriptor can never
/// drift from the rows.
pub trait StreamRecord {
    fn entry_columns() -> Vec<EntryColumn>;
}

/// Declared type of one entry column, so generic consumers can render rows
/// without the producer's Rust types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EntryValueType {
    UInt,
    Int,
    Float,
    Bool,
    Text,
    /// Absolute simulation time (`dessim::SimTime`); encoded as u64
    /// milliseconds since simulation start.
    SimTime,
    /// Elapsed simulation time (`dessim::SimDuration`); encoded as u64
    /// milliseconds.
    SimDuration,
}

/// One declared column of a stream's entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryColumn {
    pub name: String,
    pub description: String,
    pub value_type: EntryValueType,
}

impl EntryColumn {
    pub fn new(name: impl Into<String>, description: impl Into<String>, value_type: EntryValueType) -> Self {
        Self { name: name.into(), description: description.into(), value_type }
    }
}

/// How an [`Entries`] byte block is encoded.
///
/// Declared rather than implied so the representation can evolve per stream
/// — e.g. a future columnar encoding for very large streams — without a
/// protocol break. Consumers must check this before decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryEncoding {
    /// Concatenated MessagePack arrays, one per row, positional per the
    /// stream's declared columns, no outer container header.
    RowsV1,
}

/// A scalar cell decoded from a row by a consumer without the record type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EntryValue {
    Bool(bool),
    UInt(u64),
    Int(i64),
    Float(f64),
    Text(String),
}

impl EntryValue {
    /// Whether this decoded cell is compatible with a declared column type.
    /// MessagePack integers decode through the `UInt` arm when non-negative,
    /// so `Int` columns accept both integer arms.
    pub fn matches(&self, declared: EntryValueType) -> bool {
        match declared {
            EntryValueType::UInt | EntryValueType::SimTime | EntryValueType::SimDuration => {
                matches!(self, Self::UInt(_))
            }
            EntryValueType::Int => matches!(self, Self::UInt(_) | Self::Int(_)),
            EntryValueType::Float => matches!(self, Self::Float(_)),
            EntryValueType::Bool => matches!(self, Self::Bool(_)),
            EntryValueType::Text => matches!(self, Self::Text(_)),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum StreamError {
    /// An encoded row's arity does not match the declared columns.
    ColumnMismatch { declared: usize, encoded: usize },
    /// An encoded cell's wire type is incompatible with its declared column.
    TypeMismatch { column: String, declared: EntryValueType },
    /// Rows could not be encoded or decoded as MessagePack.
    Codec(String),
    /// Chunks of different streams cannot be spliced together.
    ChunkMismatch,
}

impl std::fmt::Display for StreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ColumnMismatch { declared, encoded } => {
                write!(f, "row has {encoded} fields but the stream declares {declared} columns")
            }
            Self::TypeMismatch { column, declared } => {
                write!(f, "column {column:?} declares {declared:?} but the encoded cell is incompatible")
            }
            Self::Codec(message) => write!(f, "entry codec error: {message}"),
            Self::ChunkMismatch => write!(f, "cannot splice chunks of different streams"),
        }
    }
}

impl std::error::Error for StreamError {}

/// The shared entry transport: an appendable block of positionally-encoded
/// rows. Rows stay opaque bytes so a million-row stream deserializes without
/// materializing per-cell values until a consumer asks, and appending —
/// during a run, or by splicing a live-transport chunk — is byte
/// concatenation. `entry_count` is authoritative; decoding reads exactly
/// that many rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entries {
    pub encoding: EntryEncoding,
    pub entry_count: u64,
    #[serde(with = "serde_bytes")]
    pub bytes: Vec<u8>,
}

impl Entries {
    pub fn empty() -> Self {
        Self { encoding: EntryEncoding::RowsV1, entry_count: 0, bytes: Vec::new() }
    }

    /// Encode and append rows, verifying the first row of an empty block
    /// against the declared columns — arity and per-cell type compatibility
    /// — so schema drift fails at the producer, not in a consumer.
    pub fn append_records<T: Serialize>(&mut self, declared: &[EntryColumn], records: &[T]) -> Result<(), StreamError> {
        for record in records {
            let row = rmp_serde::to_vec(record).map_err(|error| StreamError::Codec(error.to_string()))?;
            if self.entry_count == 0 && self.bytes.is_empty() {
                let cells: Vec<EntryValue> =
                    rmp_serde::from_slice(&row).map_err(|error| StreamError::Codec(error.to_string()))?;
                if cells.len() != declared.len() {
                    return Err(StreamError::ColumnMismatch { declared: declared.len(), encoded: cells.len() });
                }
                for (cell, column) in cells.iter().zip(declared) {
                    if !cell.matches(column.value_type) {
                        return Err(StreamError::TypeMismatch {
                            column: column.name.clone(),
                            declared: column.value_type,
                        });
                    }
                }
            }
            self.bytes.extend_from_slice(&row);
            self.entry_count += 1;
        }
        Ok(())
    }

    /// Splice another block of the same stream (e.g. a live-transport chunk).
    /// Bytes concatenate unchanged; chunked accumulation is byte-identical
    /// to one-shot encoding.
    pub fn append_chunk(&mut self, chunk: &Entries) -> Result<(), StreamError> {
        if chunk.encoding != self.encoding {
            return Err(StreamError::ChunkMismatch);
        }
        self.bytes.extend_from_slice(&chunk.bytes);
        self.entry_count += chunk.entry_count;
        Ok(())
    }

    /// Lazily decode rows back into the producing binary's record type —
    /// replay/test convenience, not the consumer contract (consumers use
    /// [`generic_rows`](Self::generic_rows)).
    pub fn iter_records<T: DeserializeOwned + 'static>(&self) -> impl Iterator<Item = Result<T, StreamError>> + '_ {
        RowIter {
            deserializer: Deserializer::new(self.bytes.as_slice()),
            remaining: self.entry_count,
            _marker: PhantomData,
        }
    }

    /// Decode all rows into the producing binary's record type. See
    /// [`iter_records`](Self::iter_records).
    pub fn records<T: DeserializeOwned + 'static>(&self) -> Result<Vec<T>, StreamError> {
        self.iter_records().collect()
    }

    /// The universal read path: decode rows as a dynamic in-memory table,
    /// zippable with the stream's declared columns. Works for record types
    /// this binary has never heard of; errors on cells that are not scalars
    /// or text.
    pub fn generic_rows(&self) -> Result<Vec<Vec<EntryValue>>, StreamError> {
        self.iter_records().collect()
    }
}

struct RowIter<'a, T> {
    deserializer: Deserializer<ReadReader<&'a [u8]>>,
    remaining: u64,
    _marker: PhantomData<T>,
}

impl<T: DeserializeOwned> Iterator for RowIter<'_, T> {
    type Item = Result<T, StreamError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        Some(T::deserialize(&mut self.deserializer).map_err(|error| StreamError::Codec(error.to_string())))
    }
}

/// The shared descriptor core every stream family carries: identity, column
/// schema, and open-world context — everything a generic reader needs to
/// list and tabulate ANY stream.
///
/// `context` scopes the stream with pairs such as `("resource_type",
/// "da50")` or `("zone", "mezzanine-3")`. Consumers must read scope from
/// here, never parse it out of `id`.
///
/// The descriptor is deliberately family-blind: a stream's family (log,
/// metric, ...) is structural — the wrapping type in memory, the frame tag
/// in the container — because the family determines which semantics travel
/// alongside this core, and restating it as data here could only drift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamDescriptor {
    pub id: String,
    pub label: String,
    pub description: String,
    pub columns: Vec<EntryColumn>,
    pub context: Vec<(String, String)>,
}

/// One emitted log: a fact journal — the base stream family, a
/// [`StreamDescriptor`] core with no semantic extension.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogStream {
    pub descriptor: StreamDescriptor,
    pub entries: Entries,
}

impl LogStream {
    /// An empty stream ready for [`append_records`](Self::append_records).
    /// Emitting one is how a producer declares a log that captured nothing
    /// this run — discoverable, zero rows.
    pub fn empty(descriptor: StreamDescriptor) -> Self {
        Self { descriptor, entries: Entries::empty() }
    }

    pub fn from_records<T: Serialize>(descriptor: StreamDescriptor, records: &[T]) -> Result<Self, StreamError> {
        let mut stream = Self::empty(descriptor);
        stream.append_records(records)?;
        Ok(stream)
    }

    pub fn append_records<T: Serialize>(&mut self, records: &[T]) -> Result<(), StreamError> {
        self.entries.append_records(&self.descriptor.columns, records)
    }

    /// Splice a chunk of the same stream (e.g. from live transport).
    pub fn append_chunk(&mut self, chunk: &LogStream) -> Result<(), StreamError> {
        if chunk.descriptor != self.descriptor {
            return Err(StreamError::ChunkMismatch);
        }
        self.entries.append_chunk(&chunk.entries)
    }
}

/// One emitted metric sample stream: the shared [`StreamDescriptor`] core
/// plus the metric family's semantic extension ([`MetricSemantics`]).
///
/// Conventions for the core's columns: the first column is the bucket start
/// ([`EntryValueType::SimTime`]); dimension columns follow; value columns
/// come last. Metrics with ratio aggregation carry numerator and denominator
/// columns — never a precomputed percentage. A run-total scalar is a stream
/// with exactly one row whose bucket spans the run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricStream {
    pub descriptor: StreamDescriptor,
    pub semantics: MetricSemantics,
    pub entries: Entries,
}

impl MetricStream {
    /// An empty stream ready for [`append_samples`](Self::append_samples).
    pub fn empty(descriptor: StreamDescriptor, semantics: MetricSemantics) -> Self {
        Self { descriptor, semantics, entries: Entries::empty() }
    }

    pub fn from_samples<T: Serialize>(
        descriptor: StreamDescriptor,
        semantics: MetricSemantics,
        samples: &[T],
    ) -> Result<Self, StreamError> {
        let mut stream = Self::empty(descriptor, semantics);
        stream.append_samples(samples)?;
        Ok(stream)
    }

    pub fn append_samples<T: Serialize>(&mut self, samples: &[T]) -> Result<(), StreamError> {
        self.entries.append_records(&self.descriptor.columns, samples)
    }

    /// Splice a chunk of the same stream (e.g. from live transport).
    pub fn append_chunk(&mut self, chunk: &MetricStream) -> Result<(), StreamError> {
        if chunk.descriptor != self.descriptor || chunk.semantics != self.semantics {
            return Err(StreamError::ChunkMismatch);
        }
        self.entries.append_chunk(&chunk.entries)
    }
}

#[cfg(test)]
mod tests {
    use super::super::descriptor::{AggregationClass, MetricSemantics, MetricUnit};
    use super::*;

    use crate::{SimDuration, SimTime};

    /// Stand-in for a real observer record (same shape class as
    /// `VtmOperationRecord`: ids, a timestamp, and a unit-variant enum).
    /// The derive makes this struct the single source of the column schema;
    /// the `SimTime` field type alone makes its column a time column.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, StreamRecord)]
    struct SampleOperation {
        #[stream(desc = "Simulation time.")]
        time: SimTime,
        #[stream(desc = "Acting DA50.")]
        robot_id: u32,
        #[stream(desc = "Aisle of the endpoint.")]
        aisle: u32,
        #[stream(text, desc = "Tote payload state.")]
        payload: SamplePayload,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
    enum SamplePayload {
        Empty,
        NonEmpty,
    }

    fn log_descriptor() -> StreamDescriptor {
        StreamDescriptor {
            id: "da50.operations".into(),
            label: "DA50 operations".into(),
            description: "One row per physical DA50 pickup or drop.".into(),
            columns: SampleOperation::entry_columns(),
            context: vec![("resource_type".into(), "da50".into())],
        }
    }

    #[test]
    fn derive_generates_columns_from_the_struct() {
        let columns = SampleOperation::entry_columns();
        let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["time", "robot_id", "aisle", "payload"]);
        assert_eq!(columns[0].value_type, EntryValueType::SimTime);
        assert_eq!(columns[1].value_type, EntryValueType::UInt);
        assert_eq!(columns[3].value_type, EntryValueType::Text);
        assert_eq!(columns[3].description, "Tote payload state.");
    }

    fn log_records() -> Vec<SampleOperation> {
        vec![
            SampleOperation { time: SimTime::ms(1_000), robot_id: 7, aisle: 3, payload: SamplePayload::Empty },
            SampleOperation { time: SimTime::ms(2_500), robot_id: 8, aisle: 4, payload: SamplePayload::NonEmpty },
        ]
    }

    #[test]
    fn typed_log_records_round_trip_losslessly() {
        let records = log_records();
        let stream = LogStream::from_records(log_descriptor(), &records).unwrap();
        assert_eq!(stream.entries.entry_count, 2);
        assert_eq!(stream.entries.records::<SampleOperation>().unwrap(), records);
    }

    #[test]
    fn unknown_consumer_tabulates_from_descriptor_alone() {
        let stream = LogStream::from_records(log_descriptor(), &log_records()).unwrap();
        // Simulate a consumer that has only bytes + descriptor: re-decode
        // through serde as the wire would.
        let wire = rmp_serde::to_vec_named(&stream).unwrap();
        let received: LogStream = rmp_serde::from_slice(&wire).unwrap();

        let rows = received.entries.generic_rows().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].len(), received.descriptor.columns.len());
        assert_eq!(rows[0][0], EntryValue::UInt(1_000));
        assert_eq!(rows[1][1], EntryValue::UInt(8));
        assert_eq!(rows[1][3], EntryValue::Text("NonEmpty".into()));
    }

    #[test]
    fn incremental_append_matches_one_shot_encoding() {
        let records = log_records();
        let one_shot = LogStream::from_records(log_descriptor(), &records).unwrap();

        let mut incremental = LogStream::empty(log_descriptor());
        incremental.append_records(&records[..1]).unwrap();
        incremental.append_records(&records[1..]).unwrap();

        // Byte-identical: appending during a run produces exactly the
        // at-rest representation, no re-encoding at finalization.
        assert_eq!(incremental, one_shot);
    }

    #[test]
    fn live_chunks_splice_into_the_at_rest_stream() {
        let records = log_records();
        let chunk_a = LogStream::from_records(log_descriptor(), &records[..1]).unwrap();
        let chunk_b = LogStream::from_records(log_descriptor(), &records[1..]).unwrap();

        let mut spliced = LogStream::empty(log_descriptor());
        spliced.append_chunk(&chunk_a).unwrap();
        spliced.append_chunk(&chunk_b).unwrap();

        let one_shot = LogStream::from_records(log_descriptor(), &records).unwrap();
        assert_eq!(spliced, one_shot);
    }

    #[test]
    fn lazy_iteration_decodes_without_materializing_all_rows() {
        let stream = LogStream::from_records(log_descriptor(), &log_records()).unwrap();
        let first: SampleOperation = stream.entries.iter_records().next().unwrap().unwrap();
        assert_eq!(first.robot_id, 7);
    }

    #[test]
    fn arity_drift_fails_at_the_producer() {
        let mut descriptor = log_descriptor();
        descriptor.columns.pop();
        let error = LogStream::from_records(descriptor, &log_records()).unwrap_err();
        assert_eq!(error, StreamError::ColumnMismatch { declared: 3, encoded: 4 });
    }

    #[test]
    fn lying_type_marker_fails_at_the_producer() {
        let mut descriptor = log_descriptor();
        // Claim the numeric robot_id column is text: the first appended row
        // must be rejected with the column named.
        descriptor.columns[1].value_type = EntryValueType::Text;
        let error = LogStream::from_records(descriptor, &log_records()).unwrap_err();
        assert_eq!(error, StreamError::TypeMismatch { column: "robot_id".into(), declared: EntryValueType::Text });
    }

    #[test]
    fn empty_stream_is_valid_and_discoverable() {
        let stream = LogStream::empty(log_descriptor());
        assert_eq!(stream.entries.entry_count, 0);
        assert!(stream.entries.generic_rows().unwrap().is_empty());
        assert_eq!(stream.descriptor.id, "da50.operations");
    }

    // ---- Metric streams: same transport, metric description ----

    /// One time-bucketed ratio sample: numerator and denominator travel
    /// raw so consumers sum before dividing (never precomputed percent).
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, StreamRecord)]
    struct OccupancySample {
        #[stream(desc = "Bucket start.")]
        bucket_start: SimTime,
        #[stream(desc = "Aisle.")]
        aisle: u32,
        #[stream(desc = "Occupied slot-time (numerator).")]
        occupied_slot_time: SimDuration,
        #[stream(desc = "Capacity slot-time (denominator).")]
        capacity_slot_time: SimDuration,
    }

    fn occupancy_stream() -> MetricStream {
        MetricStream::from_samples(
            StreamDescriptor {
                id: "relay.occupancy".into(),
                label: "Diving-board occupancy".into(),
                description: "Physical tote occupancy of relay positions, by aisle.".into(),
                columns: OccupancySample::entry_columns(),
                context: vec![("resource_type".into(), "relay".into())],
            },
            MetricSemantics {
                unit: MetricUnit::Ratio,
                denominator_description: Some("Available relay-position capacity-time".into()),
                aggregation: AggregationClass::Ratio,
            },
            &[
                OccupancySample {
                    bucket_start: SimTime::ZERO,
                    aisle: 1,
                    occupied_slot_time: SimDuration::ms(30_000),
                    capacity_slot_time: SimDuration::ms(60_000),
                },
                OccupancySample {
                    bucket_start: SimTime::ms(60_000),
                    aisle: 1,
                    occupied_slot_time: SimDuration::ms(45_000),
                    capacity_slot_time: SimDuration::ms(60_000),
                },
            ],
        )
        .unwrap()
    }

    #[test]
    fn metric_samples_ride_the_same_transport() {
        let stream = occupancy_stream();
        assert_eq!(stream.entries.entry_count, 2);

        // Typed read-back for the producing binary.
        let samples: Vec<OccupancySample> = stream.entries.records().unwrap();
        assert_eq!(samples[1].occupied_slot_time, SimDuration::ms(45_000));

        // The universal read path — a dynamic table for a binary without
        // the sample type; the ratio's numerator and denominator are
        // separate raw columns.
        let rows = stream.entries.generic_rows().unwrap();
        assert_eq!(rows[0][2], EntryValue::UInt(30_000));
        assert_eq!(rows[0][3], EntryValue::UInt(60_000));
        assert_eq!(stream.semantics.unit, MetricUnit::Ratio);
    }

    #[test]
    fn run_total_scalar_is_a_one_row_stream() {
        #[derive(Serialize)]
        struct TotalSample {
            bucket_start: SimTime,
            bucket_end: SimTime,
            completed: u64,
        }

        let stream = MetricStream::from_samples(
            StreamDescriptor {
                id: "completed_orders".into(),
                label: "Completed orders".into(),
                description: "Distinct completed orders over the whole run.".into(),
                columns: vec![
                    EntryColumn::new("bucket_start", "Bucket start.", EntryValueType::SimTime),
                    EntryColumn::new("bucket_end", "Bucket end.", EntryValueType::SimTime),
                    EntryColumn::new("completed", "Distinct completed orders.", EntryValueType::UInt),
                ],
                context: Vec::new(),
            },
            MetricSemantics {
                unit: MetricUnit::Count,
                denominator_description: None,
                aggregation: AggregationClass::DistinctCount,
            },
            &[TotalSample { bucket_start: SimTime::ZERO, bucket_end: SimTime::ms(14_400_000), completed: 328 }],
        )
        .unwrap();

        assert_eq!(stream.entries.entry_count, 1);
        let rows = stream.entries.generic_rows().unwrap();
        assert_eq!(rows[0][2], EntryValue::UInt(328));
    }

    #[test]
    fn metric_chunks_splice_like_log_chunks() {
        let full = occupancy_stream();
        let samples: Vec<OccupancySample> = full.entries.records().unwrap();

        let mut spliced = MetricStream::empty(full.descriptor.clone(), full.semantics.clone());
        let chunk_a =
            MetricStream::from_samples(full.descriptor.clone(), full.semantics.clone(), &samples[..1]).unwrap();
        let chunk_b =
            MetricStream::from_samples(full.descriptor.clone(), full.semantics.clone(), &samples[1..]).unwrap();
        spliced.append_chunk(&chunk_a).unwrap();
        spliced.append_chunk(&chunk_b).unwrap();

        assert_eq!(spliced, full);
    }
}
