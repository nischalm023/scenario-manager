//! Within `worldview`, `journals` is the TABLE FORM of the run's fact
//! journals: each typed journal the observer keeps — the HTM and VTM legs,
//! the VTM cycles and operations, the picks, putaways, completed orders,
//! aisle snapshots and aisle flux — encoded as declared rows and read back
//! into the same typed records.
//!
//! **What it is for.** These journals used to leave a run only inside the
//! summary the recording carried at its end, so a run that died kept none of
//! them however long it had been going, and nothing could read a row until
//! the run was over. As tables they go out with the frame written once B has
//! passed the instant that makes them final (which rows those are is the
//! observer's to say: `observer::finality`), which is what the format
//! promises for every other fact.
//!
//! **How a row is written.** One flat entry struct per journal, its columns
//! declared by the `StreamRecord` derive and its cells scalars, because a
//! consumer that has never heard of these types reads them through the
//! declared columns. Two conventions follow from that, and both are the
//! extraction stream's own:
//!
//! - An absent value is the empty string, never MessagePack nil: nil is not
//!   a scalar cell and a generic reader cannot render it. So every optional
//!   id, zone or reason travels as text, empty when the producer recorded
//!   none.
//! - An endpoint that names a place and an id — a leg's `from` and `to` —
//!   travels as two columns, the kind of place and the id, so neither is
//!   hidden inside a tagged value.
//!
//! TODO(overstuffed): nine journals, nine near-identical encode/decode
//! pairs. The shape wants a derive of its own — an entry type generated from
//! the record type — rather than nine hand-written pairs to keep in step.

use std::fmt::Display;
use std::str::FromStr;

use desgraph::{RelayId, StationId};
use desunits::{OrderId, RackId, RobotId, SkuId};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::events::{TotePayloadState, ToteSource, VtmCycleDirection, VtmIdleReasonKind, VtmOperationKind};
use crate::metrics::reporting::extraction_stream;
use crate::metrics::streams::{LogStream, StreamDescriptor, StreamError, StreamRecord};
use crate::observer::finality::recording::SliceRows;
use crate::observer::output::{
    Activity, AisleFluxRecord, AisleStateRecord, CompletedOrderRecord, HtmHold, HtmJournalEndpoint, HtmLegRecord,
    HtmWaitCause, HtmWaitPosition, HtmWaitReason, PickRecord, PutawayRecord, VtmCycleRecord, VtmJournalEndpoint,
    VtmLegRecord, VtmOperationRecord,
};
use crate::{SimDuration, SimTime};

pub const HTM_LEGS: &str = "robots.htm_legs";
pub const VTM_LEGS: &str = "robots.vtm_legs";
pub const VTM_CYCLES: &str = "robots.vtm_cycles";
pub const VTM_OPERATIONS: &str = "robots.vtm_operations";
pub const PICKS: &str = "fulfillment.picks";
pub const PUTAWAYS: &str = "fulfillment.putaways";
pub const COMPLETED_ORDERS: &str = "fulfillment.completed_orders";
pub const AISLE_STATES: &str = "inventory.aisle_states";
pub const AISLE_FLUX: &str = "inventory.aisle_flux";
pub const EXTRACTIONS: &str = "fulfillment.extractions";

/// Every journal as an empty stream.
///
/// A file declares them all at its top, so a run that picked nothing still
/// states the picks stream and a reader can tell an empty journal from a
/// journal nobody wrote.
pub fn declarations() -> Result<Vec<LogStream>, StreamError> {
    Ok(vec![
        extraction_stream(&[])?,
        htm_leg_stream(&[])?,
        vtm_leg_stream(&[])?,
        vtm_cycle_stream(&[])?,
        vtm_operation_stream(&[])?,
        pick_stream(&[])?,
        putaway_stream(&[])?,
        completed_order_stream(&[])?,
        aisle_state_stream(&[])?,
        aisle_flux_stream(&[])?,
    ])
}

/// One slice's journal rows as the chunks the file carries. A journal with no
/// rows in the slice writes no chunk.
pub fn chunks(rows: &SliceRows) -> Result<Vec<LogStream>, StreamError> {
    let mut chunks = Vec::new();
    macro_rules! chunk {
        ($rows:expr, $encode:expr) => {
            if !$rows.is_empty() {
                chunks.push($encode(&$rows)?);
            }
        };
    }
    chunk!(rows.htm_legs, htm_leg_stream);
    chunk!(rows.vtm_legs, vtm_leg_stream);
    chunk!(rows.vtm_cycles, vtm_cycle_stream);
    chunk!(rows.vtm_operations, vtm_operation_stream);
    chunk!(rows.picks, pick_stream);
    chunk!(rows.putaways, putaway_stream);
    chunk!(rows.completed_orders, completed_order_stream);
    chunk!(rows.aisle_states, aisle_state_stream);
    chunk!(rows.aisle_flux, aisle_flux_stream);
    Ok(chunks)
}

/// Which column of a table's rows holds each row's instant, a leg's start.
/// Every journal states its instant first; the extraction stream reaches the
/// run model already encoded, with no typed rows, so it has none and rides
/// the end block whole.
pub fn instant_column(table: &str) -> Option<u16> {
    match table {
        EXTRACTIONS => None,
        _ => Some(0),
    }
}

// ---------------------------------------------------------------------------
// Cells that are not plain numbers
// ---------------------------------------------------------------------------

/// An optional id as its digits, or empty when the producer recorded none.
fn id_text<T: Display>(id: Option<T>) -> String {
    id.map(|id| id.to_string()).unwrap_or_default()
}

/// The inverse of [`id_text`].
fn text_id<T: FromStr>(text: &str, column: &str) -> Result<Option<T>, StreamError> {
    if text.is_empty() {
        return Ok(None);
    }
    text.parse().map(Some).map_err(|_| StreamError::Codec(format!("{column} is not an id: {text:?}")))
}

/// A unit-variant enum as the name its own serde form states.
fn enum_text<T: Serialize>(value: &T) -> Result<String, StreamError> {
    match serde_json::to_value(value) {
        Ok(Value::String(name)) => Ok(name),
        other => Err(StreamError::Codec(format!("a journal enum is not a plain name: {other:?}"))),
    }
}

/// The inverse of [`enum_text`], and empty text is no value at all.
fn text_enum<T: DeserializeOwned>(text: &str, column: &str) -> Result<Option<T>, StreamError> {
    if text.is_empty() {
        return Ok(None);
    }
    serde_json::from_value(Value::String(text.to_string()))
        .map(Some)
        .map_err(|_| StreamError::Codec(format!("{column} names no such value: {text:?}")))
}

/// A leg endpoint as the kind of place it is and the id of that place.
fn htm_endpoint_cells(endpoint: HtmJournalEndpoint) -> (String, String) {
    match endpoint {
        HtmJournalEndpoint::Parking => ("Parking".into(), String::new()),
        HtmJournalEndpoint::Relay(relay) => ("Relay".into(), relay.0.to_string()),
        HtmJournalEndpoint::PickStation(station) => ("PickStation".into(), station.0.to_string()),
        HtmJournalEndpoint::PutawayStation(station) => ("PutawayStation".into(), station.0.to_string()),
        HtmJournalEndpoint::ExtractionStation(station) => ("ExtractionStation".into(), station.0.to_string()),
    }
}

fn htm_endpoint_of(kind: &str, id: &str, column: &str) -> Result<HtmJournalEndpoint, StreamError> {
    let station = || -> Result<StationId, StreamError> {
        Ok(StationId(text_id(id, column)?.ok_or_else(|| StreamError::Codec(format!("{column} has no id")))?))
    };
    match kind {
        "Parking" => Ok(HtmJournalEndpoint::Parking),
        "Relay" => Ok(HtmJournalEndpoint::Relay(RelayId(
            text_id(id, column)?.ok_or_else(|| StreamError::Codec(format!("{column} has no id")))?,
        ))),
        "PickStation" => Ok(HtmJournalEndpoint::PickStation(station()?)),
        "PutawayStation" => Ok(HtmJournalEndpoint::PutawayStation(station()?)),
        "ExtractionStation" => Ok(HtmJournalEndpoint::ExtractionStation(station()?)),
        other => Err(StreamError::Codec(format!("{column} names no such place: {other:?}"))),
    }
}

fn vtm_endpoint_cells(endpoint: VtmJournalEndpoint) -> (String, String) {
    match endpoint {
        VtmJournalEndpoint::Aisle => ("Aisle".into(), String::new()),
        VtmJournalEndpoint::Relay(relay) => ("Relay".into(), relay.0.to_string()),
        VtmJournalEndpoint::Rack(rack) => ("Rack".into(), rack.0.to_string()),
    }
}

fn vtm_endpoint_of(kind: &str, id: &str, column: &str) -> Result<VtmJournalEndpoint, StreamError> {
    let id = || -> Result<u32, StreamError> {
        text_id(id, column)?.ok_or_else(|| StreamError::Codec(format!("{column} has no id")))
    };
    match kind {
        "Aisle" => Ok(VtmJournalEndpoint::Aisle),
        "Relay" => Ok(VtmJournalEndpoint::Relay(RelayId(id()?))),
        "Rack" => Ok(VtmJournalEndpoint::Rack(RackId(id()?))),
        other => Err(StreamError::Codec(format!("{column} names no such place: {other:?}"))),
    }
}

// ---------------------------------------------------------------------------
// HTM journey legs
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, StreamRecord)]
struct HtmLegEntry {
    #[stream(desc = "When the leg starts.")]
    start: SimTime,
    #[stream(desc = "How long the leg lasts.")]
    duration: SimDuration,
    #[stream(desc = "The D30 this leg belongs to.")]
    robot_id: u32,
    #[stream(text, desc = "Travel, Wait, Mechanical or Alignment.")]
    activity: Activity,
    #[stream(text, desc = "The kind of place the leg starts at.")]
    from_kind: String,
    #[stream(text, desc = "The id of the place the leg starts at.")]
    from_id: String,
    #[stream(text, desc = "The kind of place the leg ends at.")]
    to_kind: String,
    #[stream(text, desc = "The id of the place the leg ends at.")]
    to_id: String,
    #[stream(text, desc = "The aisle of the starting relay.")]
    from_zone_id: String,
    #[stream(text, desc = "The aisle of the ending relay.")]
    to_zone_id: String,
    #[stream(desc = "Straight-line distance travelled over the leg.")]
    distance_mm: u64,
    #[stream(text, desc = "The presentation cycle the leg belongs to.")]
    presentation_id: String,
    #[stream(desc = "Whether a tote was on board.")]
    carrying_cargo: bool,
    #[stream(desc = "Ninety-degree-equivalent turns over the leg.")]
    turn_count: u32,
    #[stream(text, desc = "The older two-name wait vocabulary, where it can name this hold.")]
    wait_reason: String,
    #[stream(text, desc = "What the schedule was holding the D30 for.")]
    hold_cause: String,
    #[stream(text, desc = "The kind of cell the hold left the D30 on.")]
    hold_position: String,
    #[stream(text, desc = "Free-text remarks.")]
    notes: String,
}

pub fn htm_leg_stream(rows: &[HtmLegRecord]) -> Result<LogStream, StreamError> {
    let entries: Vec<HtmLegEntry> = rows
        .iter()
        .map(|row| {
            let (from_kind, from_id) = htm_endpoint_cells(row.from);
            let (to_kind, to_id) = htm_endpoint_cells(row.to);
            Ok(HtmLegEntry {
                start: row.start,
                duration: row.duration,
                robot_id: row.robot_id.0,
                activity: row.activity,
                from_kind,
                from_id,
                to_kind,
                to_id,
                from_zone_id: id_text(row.from_zone_id),
                to_zone_id: id_text(row.to_zone_id),
                distance_mm: row.distance_mm,
                presentation_id: id_text(row.presentation_id),
                carrying_cargo: row.carrying_cargo,
                turn_count: row.turn_count,
                wait_reason: row.wait_reason.as_ref().map(enum_text).transpose()?.unwrap_or_default(),
                hold_cause: row.hold.map(|hold| enum_text(&hold.cause)).transpose()?.unwrap_or_default(),
                hold_position: row.hold.map(|hold| enum_text(&hold.position)).transpose()?.unwrap_or_default(),
                notes: row.notes.clone(),
            })
        })
        .collect::<Result<_, StreamError>>()?;
    LogStream::from_records(
        StreamDescriptor {
            id: HTM_LEGS.into(),
            label: "HTM journey legs".into(),
            description: "One row per contiguous D30 travel run or stationary hold, with its endpoints and what the hold was for.".into(),
            columns: HtmLegEntry::entry_columns(),
            context: vec![("fleet".into(), "htm".into())],
        },
        &entries,
    )
}

pub fn read_htm_legs(stream: &LogStream) -> Result<Vec<HtmLegRecord>, StreamError> {
    stream
        .entries
        .records::<HtmLegEntry>()?
        .into_iter()
        .map(|entry| {
            let cause: Option<HtmWaitCause> = text_enum(&entry.hold_cause, "hold_cause")?;
            let position: Option<HtmWaitPosition> = text_enum(&entry.hold_position, "hold_position")?;
            Ok(HtmLegRecord {
                start: entry.start,
                activity: entry.activity,
                from: htm_endpoint_of(&entry.from_kind, &entry.from_id, "from")?,
                to: htm_endpoint_of(&entry.to_kind, &entry.to_id, "to")?,
                from_zone_id: text_id(&entry.from_zone_id, "from_zone_id")?,
                to_zone_id: text_id(&entry.to_zone_id, "to_zone_id")?,
                distance_mm: entry.distance_mm,
                duration: entry.duration,
                presentation_id: text_id(&entry.presentation_id, "presentation_id")?,
                robot_id: RobotId(entry.robot_id),
                carrying_cargo: entry.carrying_cargo,
                turn_count: entry.turn_count,
                wait_reason: text_enum::<HtmWaitReason>(&entry.wait_reason, "wait_reason")?,
                hold: match (cause, position) {
                    (Some(cause), Some(position)) => Some(HtmHold { cause, position }),
                    _ => None,
                },
                notes: entry.notes,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// VTM journey legs
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, StreamRecord)]
struct VtmLegEntry {
    #[stream(desc = "When the leg starts.")]
    start: SimTime,
    #[stream(desc = "How long the leg lasts.")]
    duration: SimDuration,
    #[stream(desc = "The DA50 this leg belongs to.")]
    vtm_id: u32,
    #[stream(text, desc = "Travel, Wait, Mechanical or Alignment.")]
    activity: Activity,
    #[stream(text, desc = "The kind of place the leg starts at.")]
    from_kind: String,
    #[stream(text, desc = "The id of the place the leg starts at.")]
    from_id: String,
    #[stream(text, desc = "The kind of place the leg ends at.")]
    to_kind: String,
    #[stream(text, desc = "The id of the place the leg ends at.")]
    to_id: String,
    #[stream(text, desc = "The aisle of the starting relay.")]
    from_zone_id: String,
    #[stream(text, desc = "The aisle of the ending relay.")]
    to_zone_id: String,
    #[stream(desc = "Rail distance travelled over the leg.")]
    distance_mm: u64,
    #[stream(text, desc = "The cycle the leg belongs to.")]
    vtm_cycle_id: String,
    #[stream(text, desc = "The tote the cycle carries.")]
    tote_id: String,
    #[stream(text, desc = "The presentation cycle the leg belongs to.")]
    presentation_id: String,
    #[stream(text, desc = "Forward or Return.")]
    direction: VtmCycleDirection,
    #[stream(desc = "Whether a tote was on the fork.")]
    carrying_tote: bool,
    #[stream(text, desc = "What the DA50 was waiting for, on a Wait leg.")]
    wait_reason: String,
    #[stream(text, desc = "Free-text remarks.")]
    notes: String,
}

pub fn vtm_leg_stream(rows: &[VtmLegRecord]) -> Result<LogStream, StreamError> {
    let entries: Vec<VtmLegEntry> = rows
        .iter()
        .map(|row| {
            let (from_kind, from_id) = vtm_endpoint_cells(row.from);
            let (to_kind, to_id) = vtm_endpoint_cells(row.to);
            Ok(VtmLegEntry {
                start: SimTime::ms(row.start_ms),
                duration: SimDuration::ms(row.duration_ms),
                vtm_id: row.vtm_id,
                activity: row.activity,
                from_kind,
                from_id,
                to_kind,
                to_id,
                from_zone_id: id_text(row.from_zone_id),
                to_zone_id: id_text(row.to_zone_id),
                distance_mm: row.distance_mm,
                vtm_cycle_id: id_text(row.vtm_cycle_id),
                tote_id: id_text(row.tote_id),
                presentation_id: id_text(row.presentation_id),
                direction: row.direction,
                carrying_tote: row.carrying_tote,
                wait_reason: row.wait_reason.as_ref().map(enum_text).transpose()?.unwrap_or_default(),
                notes: row.notes.clone(),
            })
        })
        .collect::<Result<_, StreamError>>()?;
    LogStream::from_records(
        StreamDescriptor {
            id: VTM_LEGS.into(),
            label: "VTM journey legs".into(),
            description: "One row per DA50 rail move, endpoint approach, fork operation or pre-cycle hold.".into(),
            columns: VtmLegEntry::entry_columns(),
            context: vec![("fleet".into(), "vtm".into())],
        },
        &entries,
    )
}

pub fn read_vtm_legs(stream: &LogStream) -> Result<Vec<VtmLegRecord>, StreamError> {
    stream
        .entries
        .records::<VtmLegEntry>()?
        .into_iter()
        .map(|entry| {
            Ok(VtmLegRecord {
                start_ms: entry.start.as_ms(),
                duration_ms: entry.duration.as_ms(),
                activity: entry.activity,
                from: vtm_endpoint_of(&entry.from_kind, &entry.from_id, "from")?,
                to: vtm_endpoint_of(&entry.to_kind, &entry.to_id, "to")?,
                from_zone_id: text_id(&entry.from_zone_id, "from_zone_id")?,
                to_zone_id: text_id(&entry.to_zone_id, "to_zone_id")?,
                distance_mm: entry.distance_mm,
                vtm_id: entry.vtm_id,
                vtm_cycle_id: text_id(&entry.vtm_cycle_id, "vtm_cycle_id")?,
                tote_id: text_id(&entry.tote_id, "tote_id")?,
                presentation_id: text_id(&entry.presentation_id, "presentation_id")?,
                direction: entry.direction,
                carrying_tote: entry.carrying_tote,
                wait_reason: text_enum::<VtmIdleReasonKind>(&entry.wait_reason, "wait_reason")?,
                notes: entry.notes,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// VTM cycles and operations
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, StreamRecord)]
struct VtmCycleEntry {
    #[stream(desc = "When the cycle completes.")]
    at: SimTime,
    #[stream(desc = "The DA50 that ran the cycle.")]
    vtm_id: u32,
    #[stream(text, desc = "The cycle's own id.")]
    vtm_cycle_id: String,
    #[stream(desc = "The aisle the cycle ran in.")]
    zone_id: u32,
    #[stream(text, desc = "Forward or Return.")]
    direction: VtmCycleDirection,
    #[stream(desc = "Empty travel within the cycle.")]
    empty_leg: SimDuration,
    #[stream(desc = "Loaded travel within the cycle.")]
    loaded_leg: SimDuration,
    #[stream(desc = "Fork movement within the cycle.")]
    fork: SimDuration,
    #[stream(desc = "Buffer transfer within the cycle.")]
    buffer: SimDuration,
    #[stream(desc = "Alignment at the rack.")]
    rack_alignment: SimDuration,
    #[stream(desc = "Alignment at the relay.")]
    relay_alignment: SimDuration,
    #[stream(desc = "Transfer at the relay.")]
    relay_transfer: SimDuration,
    #[stream(desc = "The whole cycle.")]
    total_cycle: SimDuration,
    #[stream(desc = "The relay the cycle served.")]
    relay_id: u32,
    #[stream(desc = "The rack the cycle served.")]
    rack_id: u32,
    #[stream(desc = "The tote the cycle moved.")]
    tote_id: u32,
    #[stream(text, desc = "The presentation cycle this cycle belongs to.")]
    presentation_id: String,
    #[stream(desc = "The hold before the cycle started.")]
    pre_cycle_wait: SimDuration,
    #[stream(text, desc = "What the DA50 was waiting for before the cycle.")]
    wait_reason: VtmIdleReasonKind,
    #[stream(text, desc = "Whether the tote came from a pick or a putaway.")]
    tote_source: ToteSource,
}

pub fn vtm_cycle_stream(rows: &[VtmCycleRecord]) -> Result<LogStream, StreamError> {
    let entries: Vec<VtmCycleEntry> = rows
        .iter()
        .map(|row| VtmCycleEntry {
            at: SimTime::ms(row.sim_time_ms),
            vtm_id: row.vtm_id,
            vtm_cycle_id: id_text(row.vtm_cycle_id),
            zone_id: row.zone_id,
            direction: row.direction,
            empty_leg: SimDuration::ms(row.empty_leg_ms),
            loaded_leg: SimDuration::ms(row.loaded_leg_ms),
            fork: SimDuration::ms(row.fork_ms),
            buffer: SimDuration::ms(row.buffer_ms),
            rack_alignment: SimDuration::ms(row.rack_alignment_ms),
            relay_alignment: SimDuration::ms(row.relay_alignment_ms),
            relay_transfer: SimDuration::ms(row.relay_transfer_ms),
            total_cycle: SimDuration::ms(row.total_cycle_ms),
            relay_id: row.relay_id,
            rack_id: row.rack_id,
            tote_id: row.tote_id,
            presentation_id: id_text(row.presentation_id),
            pre_cycle_wait: SimDuration::ms(row.pre_cycle_wait_ms),
            wait_reason: row.wait_reason,
            tote_source: row.tote_source,
        })
        .collect();
    LogStream::from_records(
        StreamDescriptor {
            id: VTM_CYCLES.into(),
            label: "VTM cycles".into(),
            description: "One committed DA50 cycle, split into its travel, alignment and transfer phases.".into(),
            columns: VtmCycleEntry::entry_columns(),
            context: vec![("fleet".into(), "vtm".into())],
        },
        &entries,
    )
}

pub fn read_vtm_cycles(stream: &LogStream) -> Result<Vec<VtmCycleRecord>, StreamError> {
    stream
        .entries
        .records::<VtmCycleEntry>()?
        .into_iter()
        .map(|entry| {
            Ok(VtmCycleRecord {
                sim_time_ms: entry.at.as_ms(),
                vtm_id: entry.vtm_id,
                vtm_cycle_id: text_id(&entry.vtm_cycle_id, "vtm_cycle_id")?,
                zone_id: entry.zone_id,
                direction: entry.direction,
                empty_leg_ms: entry.empty_leg.as_ms(),
                loaded_leg_ms: entry.loaded_leg.as_ms(),
                fork_ms: entry.fork.as_ms(),
                buffer_ms: entry.buffer.as_ms(),
                rack_alignment_ms: entry.rack_alignment.as_ms(),
                relay_alignment_ms: entry.relay_alignment.as_ms(),
                relay_transfer_ms: entry.relay_transfer.as_ms(),
                total_cycle_ms: entry.total_cycle.as_ms(),
                relay_id: entry.relay_id,
                rack_id: entry.rack_id,
                tote_id: entry.tote_id,
                presentation_id: text_id(&entry.presentation_id, "presentation_id")?,
                pre_cycle_wait_ms: entry.pre_cycle_wait.as_ms(),
                wait_reason: entry.wait_reason,
                tote_source: entry.tote_source,
            })
        })
        .collect()
}

#[derive(Serialize, Deserialize, StreamRecord)]
struct VtmOperationEntry {
    #[stream(desc = "When the operation completes.")]
    at: SimTime,
    #[stream(desc = "The DA50 that ran the operation.")]
    vtm_id: u32,
    #[stream(desc = "The cycle the operation belongs to.")]
    vtm_cycle_id: u64,
    #[stream(desc = "The aisle the operation ran in.")]
    zone_id: u32,
    #[stream(text, desc = "Which of the four fork operations this is.")]
    kind: VtmOperationKind,
    #[stream(desc = "The rack the operation served.")]
    rack_id: u32,
    #[stream(text, desc = "The rack level the operation served.")]
    rack_level: String,
    #[stream(desc = "The relay the operation served.")]
    relay_id: u32,
    #[stream(desc = "The tote the operation moved.")]
    tote_id: u32,
    #[stream(text, desc = "Whether the tote was empty, holding units, or unknown.")]
    payload: TotePayloadState,
    #[stream(text, desc = "The presentation cycle the operation belongs to.")]
    presentation_id: String,
}

pub fn vtm_operation_stream(rows: &[VtmOperationRecord]) -> Result<LogStream, StreamError> {
    let entries: Vec<VtmOperationEntry> = rows
        .iter()
        .map(|row| VtmOperationEntry {
            at: SimTime::ms(row.sim_time_ms),
            vtm_id: row.vtm_id,
            vtm_cycle_id: row.vtm_cycle_id,
            zone_id: row.zone_id,
            kind: row.kind,
            rack_id: row.rack_id,
            rack_level: id_text(row.rack_level),
            relay_id: row.relay_id,
            tote_id: row.tote_id,
            payload: row.payload,
            presentation_id: id_text(row.presentation_id),
        })
        .collect();
    LogStream::from_records(
        StreamDescriptor {
            id: VTM_OPERATIONS.into(),
            label: "VTM operations".into(),
            description: "One DA50 fork operation: a pickup from or a drop into a rack or a relay.".into(),
            columns: VtmOperationEntry::entry_columns(),
            context: vec![("fleet".into(), "vtm".into())],
        },
        &entries,
    )
}

pub fn read_vtm_operations(stream: &LogStream) -> Result<Vec<VtmOperationRecord>, StreamError> {
    stream
        .entries
        .records::<VtmOperationEntry>()?
        .into_iter()
        .map(|entry| {
            Ok(VtmOperationRecord {
                sim_time_ms: entry.at.as_ms(),
                vtm_id: entry.vtm_id,
                vtm_cycle_id: entry.vtm_cycle_id,
                zone_id: entry.zone_id,
                kind: entry.kind,
                rack_id: entry.rack_id,
                rack_level: text_id(&entry.rack_level, "rack_level")?,
                relay_id: entry.relay_id,
                tote_id: entry.tote_id,
                payload: entry.payload,
                presentation_id: text_id(&entry.presentation_id, "presentation_id")?,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Fulfillment: picks, putaways, completed orders
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, StreamRecord)]
struct PickEntry {
    #[stream(desc = "When the pick completes.")]
    at: SimTime,
    #[stream(desc = "The station the pick happened at.")]
    station_id: u32,
    #[stream(desc = "The SKU picked.")]
    sku: u32,
    #[stream(desc = "Units picked.")]
    quantity: u32,
    #[stream(text, desc = "The distribution centre the order belongs to.")]
    dc_id: String,
    #[stream(text, desc = "The order the pick belongs to.")]
    order_id: String,
    #[stream(text, desc = "The tote picked from.")]
    tote_id: String,
    #[stream(text, desc = "The aisle the tote came from.")]
    source_zone_id: String,
    #[stream(text, desc = "The relay the tote came from.")]
    source_relay_id: String,
    #[stream(text, desc = "The presentation cycle the pick belongs to.")]
    presentation_id: String,
}

pub fn pick_stream(rows: &[PickRecord]) -> Result<LogStream, StreamError> {
    let entries: Vec<PickEntry> = rows
        .iter()
        .map(|row| PickEntry {
            at: SimTime::ms(row.sim_time_ms),
            station_id: row.station_id.0,
            sku: row.sku.0,
            quantity: row.quantity,
            dc_id: row.dc_id.clone().unwrap_or_default(),
            order_id: id_text(row.order_id.map(|id| id.0)),
            tote_id: id_text(row.tote_id),
            source_zone_id: id_text(row.source_zone_id),
            source_relay_id: id_text(row.source_relay_id.map(|id| id.0)),
            presentation_id: id_text(row.presentation_id),
        })
        .collect();
    LogStream::from_records(
        StreamDescriptor {
            id: PICKS.into(),
            label: "Picks".into(),
            description: "One completed pick at a station, with the order and the tote it came from.".into(),
            columns: PickEntry::entry_columns(),
            context: vec![("work".into(), "pick".into())],
        },
        &entries,
    )
}

pub fn read_picks(stream: &LogStream) -> Result<Vec<PickRecord>, StreamError> {
    stream
        .entries
        .records::<PickEntry>()?
        .into_iter()
        .map(|entry| {
            Ok(PickRecord {
                sim_time_ms: entry.at.as_ms(),
                station_id: StationId(entry.station_id),
                sku: SkuId(entry.sku),
                quantity: entry.quantity,
                dc_id: (!entry.dc_id.is_empty()).then_some(entry.dc_id),
                order_id: text_id(&entry.order_id, "order_id")?.map(OrderId),
                tote_id: text_id(&entry.tote_id, "tote_id")?,
                source_zone_id: text_id(&entry.source_zone_id, "source_zone_id")?,
                source_relay_id: text_id(&entry.source_relay_id, "source_relay_id")?.map(RelayId),
                presentation_id: text_id(&entry.presentation_id, "presentation_id")?,
            })
        })
        .collect()
}

#[derive(Serialize, Deserialize, StreamRecord)]
struct PutawayEntry {
    #[stream(desc = "When the induction completes.")]
    at: SimTime,
    #[stream(desc = "The station the putaway happened at.")]
    station_id: u32,
    #[stream(desc = "The SKU inducted.")]
    sku: u32,
    #[stream(desc = "Units inducted.")]
    quantity: u32,
    #[stream(text, desc = "The presentation cycle the putaway belongs to.")]
    presentation_id: String,
}

pub fn putaway_stream(rows: &[PutawayRecord]) -> Result<LogStream, StreamError> {
    let entries: Vec<PutawayEntry> = rows
        .iter()
        .map(|row| PutawayEntry {
            at: SimTime::ms(row.sim_time_ms),
            station_id: row.station_id.0,
            sku: row.sku.0,
            quantity: row.quantity,
            presentation_id: id_text(row.presentation_id),
        })
        .collect();
    LogStream::from_records(
        StreamDescriptor {
            id: PUTAWAYS.into(),
            label: "Putaways".into(),
            description: "One completed putaway presentation at an induction station.".into(),
            columns: PutawayEntry::entry_columns(),
            context: vec![("work".into(), "putaway".into())],
        },
        &entries,
    )
}

pub fn read_putaways(stream: &LogStream) -> Result<Vec<PutawayRecord>, StreamError> {
    stream
        .entries
        .records::<PutawayEntry>()?
        .into_iter()
        .map(|entry| {
            Ok(PutawayRecord {
                sim_time_ms: entry.at.as_ms(),
                station_id: StationId(entry.station_id),
                sku: SkuId(entry.sku),
                quantity: entry.quantity,
                presentation_id: text_id(&entry.presentation_id, "presentation_id")?,
            })
        })
        .collect()
}

#[derive(Serialize, Deserialize, StreamRecord)]
struct CompletedOrderEntry {
    #[stream(desc = "When the order completes.")]
    at: SimTime,
    #[stream(desc = "The order.")]
    order_id: u32,
    #[stream(text, desc = "The station the order completed at.")]
    station_id: String,
    #[stream(text, desc = "The distribution centre the order belongs to.")]
    dc_id: String,
}

pub fn completed_order_stream(rows: &[CompletedOrderRecord]) -> Result<LogStream, StreamError> {
    let entries: Vec<CompletedOrderEntry> = rows
        .iter()
        .map(|row| CompletedOrderEntry {
            at: SimTime::ms(row.sim_time_ms),
            order_id: row.order_id.0,
            station_id: id_text(row.station_id.map(|id| id.0)),
            dc_id: row.dc_id.clone().unwrap_or_default(),
        })
        .collect();
    LogStream::from_records(
        StreamDescriptor {
            id: COMPLETED_ORDERS.into(),
            label: "Completed orders".into(),
            description: "One completed order, with the station and distribution centre it is attributed to.".into(),
            columns: CompletedOrderEntry::entry_columns(),
            context: vec![("work".into(), "order".into())],
        },
        &entries,
    )
}

pub fn read_completed_orders(stream: &LogStream) -> Result<Vec<CompletedOrderRecord>, StreamError> {
    stream
        .entries
        .records::<CompletedOrderEntry>()?
        .into_iter()
        .map(|entry| {
            Ok(CompletedOrderRecord {
                sim_time_ms: entry.at.as_ms(),
                order_id: OrderId(entry.order_id),
                station_id: text_id(&entry.station_id, "station_id")?.map(StationId),
                dc_id: (!entry.dc_id.is_empty()).then_some(entry.dc_id),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Inventory: aisle snapshots and aisle flux
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, StreamRecord)]
struct AisleStateEntry {
    #[stream(desc = "When the snapshot was taken.")]
    at: SimTime,
    #[stream(desc = "The aisle.")]
    zone_id: u32,
    #[stream(desc = "Rack slots in the aisle.")]
    storage_capacity: u32,
    #[stream(desc = "Rack slots holding a tote.")]
    storage_occupied: u32,
    #[stream(desc = "Rack slots holding an empty tote.")]
    storage_empty: u32,
    #[stream(desc = "Rack slots holding a tote with units in it.")]
    storage_non_empty: u32,
    #[stream(desc = "Rack slots whose tote contents are not known.")]
    storage_unknown: u32,
    #[stream(desc = "Diving-board slots in the aisle.")]
    diving_board_capacity: u32,
    #[stream(desc = "Diving-board slots holding a tote.")]
    diving_board_occupied: u32,
}

pub fn aisle_state_stream(rows: &[AisleStateRecord]) -> Result<LogStream, StreamError> {
    let entries: Vec<AisleStateEntry> = rows
        .iter()
        .map(|row| AisleStateEntry {
            at: SimTime::ms(row.sim_time_ms),
            zone_id: row.zone_id,
            storage_capacity: row.storage_capacity,
            storage_occupied: row.storage_occupied,
            storage_empty: row.storage_empty,
            storage_non_empty: row.storage_non_empty,
            storage_unknown: row.storage_unknown,
            diving_board_capacity: row.diving_board_capacity,
            diving_board_occupied: row.diving_board_occupied,
        })
        .collect();
    LogStream::from_records(
        StreamDescriptor {
            id: AISLE_STATES.into(),
            label: "Aisle snapshots".into(),
            description: "The rack and diving-board occupancy of one aisle, after a physical change in it.".into(),
            columns: AisleStateEntry::entry_columns(),
            context: vec![("subject".into(), "aisle".into())],
        },
        &entries,
    )
}

pub fn read_aisle_states(stream: &LogStream) -> Result<Vec<AisleStateRecord>, StreamError> {
    Ok(stream
        .entries
        .records::<AisleStateEntry>()?
        .into_iter()
        .map(|entry| AisleStateRecord {
            sim_time_ms: entry.at.as_ms(),
            zone_id: entry.zone_id,
            storage_capacity: entry.storage_capacity,
            storage_occupied: entry.storage_occupied,
            storage_empty: entry.storage_empty,
            storage_non_empty: entry.storage_non_empty,
            storage_unknown: entry.storage_unknown,
            diving_board_capacity: entry.diving_board_capacity,
            diving_board_occupied: entry.diving_board_occupied,
        })
        .collect())
}

#[derive(Serialize, Deserialize, StreamRecord)]
struct AisleFluxEntry {
    #[stream(desc = "When the tote crossed the aisle boundary.")]
    at: SimTime,
    #[stream(desc = "The aisle crossed into or out of.")]
    zone_id: u32,
    #[stream(desc = "The tote that carried the units across.")]
    tote_id: u32,
    #[stream(desc = "The SKU in the tote.")]
    sku: u32,
    #[stream(desc = "Units entering the aisle, negative when leaving it.")]
    delta: i32,
}

pub fn aisle_flux_stream(rows: &[AisleFluxRecord]) -> Result<LogStream, StreamError> {
    let entries: Vec<AisleFluxEntry> = rows
        .iter()
        .map(|row| AisleFluxEntry {
            at: SimTime::ms(row.sim_time_ms),
            zone_id: row.zone_id,
            tote_id: row.tote_id,
            sku: row.sku.0,
            delta: row.delta,
        })
        .collect();
    LogStream::from_records(
        StreamDescriptor {
            id: AISLE_FLUX.into(),
            label: "Aisle inventory flux".into(),
            description: "One tote's units entering or leaving an aisle.".into(),
            columns: AisleFluxEntry::entry_columns(),
            context: vec![("subject".into(), "aisle".into())],
        },
        &entries,
    )
}

pub fn read_aisle_flux(stream: &LogStream) -> Result<Vec<AisleFluxRecord>, StreamError> {
    Ok(stream
        .entries
        .records::<AisleFluxEntry>()?
        .into_iter()
        .map(|entry| AisleFluxRecord {
            sim_time_ms: entry.at.as_ms(),
            zone_id: entry.zone_id,
            tote_id: entry.tote_id,
            sku: SkuId(entry.sku),
            delta: entry.delta,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::streams::EntryValue;

    fn htm_leg() -> HtmLegRecord {
        HtmLegRecord {
            start: SimTime::ms(1_250),
            activity: Activity::Wait,
            from: HtmJournalEndpoint::Relay(RelayId(7)),
            to: HtmJournalEndpoint::PickStation(StationId(3)),
            from_zone_id: Some(2),
            to_zone_id: None,
            distance_mm: 4_100,
            duration: SimDuration::ms(900),
            presentation_id: Some(88),
            robot_id: RobotId(11),
            carrying_cargo: true,
            turn_count: 2,
            wait_reason: Some(HtmWaitReason::AwaitingDeposit),
            hold: Some(HtmHold { cause: HtmWaitCause::AwaitingSlot, position: HtmWaitPosition::Relay }),
            notes: "held".into(),
        }
    }

    fn vtm_leg() -> VtmLegRecord {
        VtmLegRecord {
            start_ms: 2_000,
            duration_ms: 350,
            activity: Activity::Travel,
            from: VtmJournalEndpoint::Rack(RackId(4)),
            to: VtmJournalEndpoint::Aisle,
            from_zone_id: None,
            to_zone_id: Some(5),
            distance_mm: 900,
            vtm_id: 6,
            vtm_cycle_id: Some(21),
            tote_id: Some(77),
            presentation_id: None,
            direction: VtmCycleDirection::Return,
            carrying_tote: false,
            wait_reason: Some(VtmIdleReasonKind::AwaitingSlot),
            notes: String::new(),
        }
    }

    /// Every cell a journal row states is a scalar a reader without these
    /// types can render: the enums are their names, and an absent id is
    /// empty text rather than a nil the generic reader cannot decode.
    #[test]
    fn a_journal_rows_cells_are_scalars_a_generic_reader_can_read() {
        let stream = htm_leg_stream(&[htm_leg()]).expect("encodes");
        let rows = stream.entries.generic_rows().expect("a generic reader reads every cell");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].len(), stream.descriptor.columns.len());
        assert_eq!(rows[0][3], EntryValue::Text("Wait".into()), "the activity is its own name");
        let to_zone = stream.descriptor.columns.iter().position(|c| c.name == "to_zone_id").expect("column");
        assert_eq!(rows[0][to_zone], EntryValue::Text(String::new()), "an absent zone is empty text");
        let vtm = vtm_leg_stream(&[vtm_leg()]).expect("encodes");
        assert!(vtm.entries.generic_rows().is_ok(), "and the same for a VTM leg with absent ids");
    }

    /// A journal written to a stream and read back is the same typed row,
    /// which is what the workbook needs of the file.
    #[test]
    fn every_journal_comes_back_the_row_it_went_in_as() {
        let legs = [htm_leg()];
        let read = read_htm_legs(&htm_leg_stream(&legs).expect("encodes")).expect("decodes");
        assert_eq!(format!("{:?}", read), format!("{:?}", legs));

        let vtm_legs = [vtm_leg()];
        let read = read_vtm_legs(&vtm_leg_stream(&vtm_legs).expect("encodes")).expect("decodes");
        assert_eq!(format!("{:?}", read), format!("{:?}", vtm_legs));

        let cycles = [VtmCycleRecord {
            sim_time_ms: 5_000,
            vtm_id: 1,
            vtm_cycle_id: Some(9),
            zone_id: 2,
            direction: VtmCycleDirection::Forward,
            empty_leg_ms: 10,
            loaded_leg_ms: 20,
            fork_ms: 30,
            buffer_ms: 40,
            rack_alignment_ms: 50,
            relay_alignment_ms: 60,
            relay_transfer_ms: 70,
            total_cycle_ms: 280,
            relay_id: 3,
            rack_id: 4,
            tote_id: 5,
            presentation_id: None,
            pre_cycle_wait_ms: 80,
            wait_reason: VtmIdleReasonKind::AwaitingTote,
            tote_source: ToteSource::Putaway,
        }];
        let read = read_vtm_cycles(&vtm_cycle_stream(&cycles).expect("encodes")).expect("decodes");
        assert_eq!(format!("{:?}", read), format!("{:?}", cycles));

        let operations = [VtmOperationRecord {
            sim_time_ms: 6_000,
            vtm_id: 1,
            vtm_cycle_id: 9,
            zone_id: 2,
            kind: VtmOperationKind::DropRelay,
            rack_id: 3,
            rack_level: Some(2),
            relay_id: 4,
            tote_id: 5,
            payload: TotePayloadState::NonEmpty,
            presentation_id: Some(12),
        }];
        let read = read_vtm_operations(&vtm_operation_stream(&operations).expect("encodes")).expect("decodes");
        assert_eq!(format!("{:?}", read), format!("{:?}", operations));

        let picks = [PickRecord {
            sim_time_ms: 7_000,
            station_id: StationId(2),
            sku: SkuId(44),
            quantity: 3,
            dc_id: Some("east".into()),
            order_id: Some(OrderId(8)),
            tote_id: Some(90),
            source_zone_id: Some(1),
            source_relay_id: Some(RelayId(6)),
            presentation_id: None,
        }];
        let read = read_picks(&pick_stream(&picks).expect("encodes")).expect("decodes");
        assert_eq!(format!("{:?}", read), format!("{:?}", picks));

        let putaways = [PutawayRecord {
            sim_time_ms: 8_000,
            station_id: StationId(3),
            sku: SkuId(45),
            quantity: 4,
            presentation_id: Some(13),
        }];
        let read = read_putaways(&putaway_stream(&putaways).expect("encodes")).expect("decodes");
        assert_eq!(format!("{:?}", read), format!("{:?}", putaways));

        let orders = [CompletedOrderRecord {
            sim_time_ms: 9_000,
            order_id: OrderId(8),
            station_id: Some(StationId(2)),
            dc_id: None,
        }];
        let read = read_completed_orders(&completed_order_stream(&orders).expect("encodes")).expect("decodes");
        assert_eq!(format!("{:?}", read), format!("{:?}", orders));

        let aisles = [AisleStateRecord {
            sim_time_ms: 10_000,
            zone_id: 1,
            storage_capacity: 100,
            storage_occupied: 40,
            storage_empty: 10,
            storage_non_empty: 30,
            storage_unknown: 0,
            diving_board_capacity: 4,
            diving_board_occupied: 1,
        }];
        let read = read_aisle_states(&aisle_state_stream(&aisles).expect("encodes")).expect("decodes");
        assert_eq!(format!("{:?}", read), format!("{:?}", aisles));

        let flux = [AisleFluxRecord { sim_time_ms: 11_000, zone_id: 1, tote_id: 90, sku: SkuId(44), delta: -3 }];
        let read = read_aisle_flux(&aisle_flux_stream(&flux).expect("encodes")).expect("decodes");
        assert_eq!(format!("{:?}", read), format!("{:?}", flux));
    }
}
