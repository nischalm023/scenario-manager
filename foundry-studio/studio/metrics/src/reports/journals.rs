//! Within `reports`, `journals` is THE RUN'S FACT JOURNALS REBUILT FROM THE RECORDING'S TABLE ROWS: the rows each delta
//! carries (and the end block's) decoded back into the observer's record structs, by the same readers the run model
//! uses (`dessim::metrics::worldview::journals`). Each journal is kept from the run's start on its own: one whose rows
//! do not read fails only the views that read it. A report window is handed only the records it reads, so building a
//! window costs its own records, not the run's. The D30s' and DA50s' interval rows are not read: their state and wait
//! series read the robot state channel.

use desgraph::StationId;
use dessim::metrics::streams::{Entries, EntryEncoding, LogStream, StreamDescriptor, StreamError};
use dessim::metrics::worldview::journals::{
    AISLE_STATES, COMPLETED_ORDERS, EXTRACTIONS, PICKS, VTM_OPERATIONS, declarations, read_aisle_states,
    read_completed_orders, read_picks, read_vtm_operations,
};
use dessim::model::DistributionCenterId;
use dessim::observer::output::{AisleStateRecord, CompletedOrderRecord, PickRecord, VtmOperationRecord};
use rustc_hash::FxHashMap;
use worldview::Descriptors;
use worldview::frames::tables::TableRows;

use crate::follower::{Unreadable, Unservable};

/// The journals the report tabs and the end-block series read.
pub(crate) struct Journals {
    pub vtm_operations: Journal<VtmOperationRecord>,
    pub picks: Journal<PickRecord>,
    pub completed_orders: Journal<CompletedOrderRecord>,
    pub aisle_states: AisleStates,
    /// The extraction stream, which rides the end block whole, as the extraction report reads it.
    pub extractions: Result<Option<LogStream>, Unservable>,
    /// The table ids the recording declared, by name.
    names: FxHashMap<u16, String>,
    /// The journals' own descriptors, the readers' key to a row's columns.
    declared: FxHashMap<String, StreamDescriptor>,
}

/// One journal of instants in simtime order, or why its rows stopped reading. A report states a row for every key the
/// records it reads name, so a window is handed, beside its own records, the earliest record naming each key.
pub(crate) struct Journal<T> {
    held: Result<Instants<T>, Unservable>,
}

struct Instants<T> {
    /// Every record, by `at`; records of one instant in the order taken.
    records: Vec<T>,
    /// For each key, the earliest record naming it.
    namers: Vec<T>,
    named: FxHashMap<Key, usize>,
}

/// The aisles' state records, each aisle's in simtime order, or why their rows stopped reading. The occupancy report
/// integrates an aisle's state from the one in force as a window begins.
pub(crate) struct AisleStates {
    held: Result<FxHashMap<u32, Vec<AisleStateRecord>>, Unservable>,
}

/// What a report states a row for once a record names it: an aisle, a pick station or a distribution centre.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Key {
    Aisle(u32),
    Station(StationId),
    Centre(DistributionCenterId),
}

/// A journal's record: when it is dated in simtime, and the keys it names.
pub(crate) trait Record: Clone {
    fn at(&self) -> u64;
    fn keys(&self) -> Vec<Key>;
}

impl<T: Record> Journal<T> {
    fn new() -> Self {
        Self { held: Ok(Instants { records: Vec::new(), namers: Vec::new(), named: FxHashMap::default() }) }
    }

    /// Every record, by `at`; or why the journal does not read.
    pub(crate) fn records(&self) -> Result<&[T], Unservable> {
        self.held.as_ref().map(|held| held.records.as_slice()).map_err(Clone::clone)
    }

    /// Every record dated by `end` in simtime.
    pub(crate) fn by(&self, end: u64) -> Result<&[T], Unservable> {
        self.records().map(|records| &records[..slot(records, end)])
    }

    /// What a report over `(start, end]` in simtime reads, to state the rows it states over every record dated by
    /// `end`: the records dated inside, and the earliest record naming each key named by `start`, which lies outside.
    pub(crate) fn window(&self, start: u64, end: u64) -> Result<Vec<T>, Unservable> {
        let held = self.held.as_ref().map_err(Clone::clone)?;
        let inside = &held.records[slot(&held.records, start)..slot(&held.records, end)];
        let named = held.namers.iter().filter(|record| record.at() <= start);
        Ok(named.chain(inside).cloned().collect())
    }

    pub(super) fn take(&mut self, records: Vec<T>) {
        let Ok(held) = &mut self.held else { return };
        for record in records {
            for key in record.keys() {
                match held.named.get(&key) {
                    Some(&namer) if held.namers[namer].at() <= record.at() => {}
                    Some(&namer) => held.namers[namer] = record.clone(),
                    None => {
                        held.named.insert(key, held.namers.len());
                        held.namers.push(record.clone());
                    }
                }
            }
            let at = slot(&held.records, record.at());
            held.records.insert(at, record);
        }
    }

    fn fail(&mut self, fault: &Unreadable) {
        if self.held.is_ok() {
            self.held = Err(fault.clone().into());
        }
    }
}

impl AisleStates {
    /// Every aisle's records dated by `end` in simtime; or why they do not read.
    pub(crate) fn by(&self, end: u64) -> Result<Vec<AisleStateRecord>, Unservable> {
        let held = self.held.as_ref().map_err(Clone::clone)?;
        Ok(held.values().flat_map(|records| &records[..slot(records, end)]).copied().collect())
    }

    /// What the occupancy report over `(start, end]` in simtime reads, to state the rows it states over every record
    /// dated by `end`: each aisle's state in force at `start`, then its records dated inside. The report reads each
    /// aisle's records apart, so their order across aisles states nothing.
    pub(crate) fn window(&self, start: u64, end: u64) -> Result<Vec<AisleStateRecord>, Unservable> {
        let held = self.held.as_ref().map_err(Clone::clone)?;
        let mut read = Vec::new();
        for records in held.values() {
            let from = slot(records, start);
            read.extend(records[from.saturating_sub(1)..slot(records, end)].iter().copied());
        }
        Ok(read)
    }

    pub(super) fn take(&mut self, records: Vec<AisleStateRecord>) {
        let Ok(held) = &mut self.held else { return };
        for record in records {
            let aisle = held.entry(record.zone_id).or_default();
            aisle.insert(slot(aisle, record.sim_time_ms), record);
        }
    }

    fn fail(&mut self, fault: &Unreadable) {
        if self.held.is_ok() {
            self.held = Err(fault.clone().into());
        }
    }
}

/// Where a record dated `at` goes among `records`: after every record dated by it.
fn slot<T: Record>(records: &[T], at: u64) -> usize {
    records.partition_point(|record| record.at() <= at)
}

impl Record for VtmOperationRecord {
    fn at(&self) -> u64 {
        self.sim_time_ms
    }
    fn keys(&self) -> Vec<Key> {
        vec![Key::Aisle(self.zone_id)]
    }
}

impl Record for AisleStateRecord {
    fn at(&self) -> u64 {
        self.sim_time_ms
    }
    fn keys(&self) -> Vec<Key> {
        vec![Key::Aisle(self.zone_id)]
    }
}

impl Record for PickRecord {
    fn at(&self) -> u64 {
        self.sim_time_ms
    }
    fn keys(&self) -> Vec<Key> {
        let aisle = self.source_zone_id.map(Key::Aisle);
        let centre = self.dc_id.clone().map(Key::Centre);
        [Some(Key::Station(self.station_id)), aisle, centre].into_iter().flatten().collect()
    }
}

impl Record for CompletedOrderRecord {
    fn at(&self) -> u64 {
        self.sim_time_ms
    }
    fn keys(&self) -> Vec<Key> {
        [self.station_id.map(Key::Station), self.dc_id.clone().map(Key::Centre)].into_iter().flatten().collect()
    }
}

impl Journals {
    pub(crate) fn new() -> Self {
        let declared = declarations()
            .map(|streams| {
                streams.into_iter().map(|stream| (stream.descriptor.id.clone(), stream.descriptor)).collect()
            })
            .unwrap_or_default();
        Self {
            vtm_operations: Journal::new(),
            picks: Journal::new(),
            completed_orders: Journal::new(),
            aisle_states: AisleStates { held: Ok(FxHashMap::default()) },
            extractions: Ok(None),
            names: FxHashMap::default(),
            declared,
        }
    }

    /// The tables the recording declares, so a frame's rows are read by the right journal. A declared extraction table
    /// holds its stream from its declaration on, with no row until rows come, as the run model's reader keeps every
    /// declared table: a run that extracted nothing states no extraction, measured, as desvis's tab does.
    pub(crate) fn describe(&mut self, descriptors: &Descriptors) {
        self.names = descriptors.tables.iter().map(|table| (table.id.0, table.name.clone())).collect();
        if matches!(self.extractions, Ok(None)) && self.declares(EXTRACTIONS) {
            let declared = self.declared.get(EXTRACTIONS).cloned();
            self.extractions = Ok(declared.map(|descriptor| LogStream { descriptor, entries: Entries::empty() }));
        }
    }

    /// Whether the recording declares the table `name`.
    pub(crate) fn declares(&self, name: &str) -> bool {
        self.names.values().any(|declared| declared == name)
    }

    /// Whether the journal of the table `name` still reads: one that does not fails the views reading it.
    pub(crate) fn reads(&self, name: &str) -> Result<(), Unservable> {
        match name {
            VTM_OPERATIONS => self.vtm_operations.records().map(drop),
            PICKS => self.picks.records().map(drop),
            COMPLETED_ORDERS => self.completed_orders.records().map(drop),
            AISLE_STATES => self.aisle_states.held.as_ref().map(drop).map_err(Clone::clone),
            EXTRACTIONS => self.extractions.as_ref().map(drop).map_err(Clone::clone),
            _ => Ok(()),
        }
    }

    /// One frame's rows of every table, each taken into its journal; a table of no journal here is passed over, and
    /// rows that do not read fail their own journal alone.
    pub(crate) fn take(&mut self, tables: &[TableRows]) {
        for rows in tables {
            if rows.row_count == 0 {
                continue;
            }
            let Some(name) = self.names.get(&rows.table.0).cloned() else { continue };
            let Some(descriptor) = self.declared.get(&name).cloned() else { continue };
            let stream = LogStream {
                descriptor,
                entries: Entries {
                    encoding: EntryEncoding::RowsV1,
                    entry_count: u64::from(rows.row_count),
                    bytes: rows.rows.clone(),
                },
            };
            let read = |fault: StreamError| Unreadable::because(format!("the {name} rows do not read: {fault}"));
            match name.as_str() {
                VTM_OPERATIONS => match read_vtm_operations(&stream) {
                    Ok(records) => self.vtm_operations.take(records),
                    Err(fault) => self.vtm_operations.fail(&read(fault)),
                },
                PICKS => match read_picks(&stream) {
                    Ok(records) => self.picks.take(records),
                    Err(fault) => self.picks.fail(&read(fault)),
                },
                COMPLETED_ORDERS => match read_completed_orders(&stream) {
                    Ok(records) => self.completed_orders.take(records),
                    Err(fault) => self.completed_orders.fail(&read(fault)),
                },
                AISLE_STATES => match read_aisle_states(&stream) {
                    Ok(records) => self.aisle_states.take(records),
                    Err(fault) => self.aisle_states.fail(&read(fault)),
                },
                EXTRACTIONS => {
                    let appended = match &mut self.extractions {
                        Ok(Some(held)) => held.append_chunk(&stream).map_err(read),
                        Ok(held) => {
                            *held = Some(stream);
                            Ok(())
                        }
                        Err(_) => Ok(()),
                    };
                    if let Err(fault) = appended {
                        self.extractions = Err(fault.into());
                    }
                }
                _ => {}
            }
        }
    }

    /// A frame whose tables do not read at all: no journal can tell which of its rows were lost, so every one fails.
    pub(crate) fn fail(&mut self, fault: &Unreadable) {
        self.vtm_operations.fail(fault);
        self.picks.fail(fault);
        self.completed_orders.fail(fault);
        self.aisle_states.fail(fault);
        if self.extractions.is_ok() {
            self.extractions = Err(fault.clone().into());
        }
    }
}
