use std::collections::BTreeMap;
use std::fmt;
use std::ops::Deref;
use std::sync::{Arc, Mutex};

use desgraph::{CardinalDirection, CellId, StationId, StationType};

use crate::model::reservation::oracle::ReservationOracle;
use crate::model::{RobotId, SimDuration, SimOffset, SimTime};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StationDetails {
    pub id: StationId,
    pub station_type: StationType,
    pub cell: CellId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
pub enum StationArrivalSource {
    PutawayDirect,
    PutawayFromRack,

    PickFromRack,
}

impl StationArrivalSource {
    fn label(self) -> &'static str {
        match self {
            Self::PutawayDirect => "pa.dir",
            Self::PutawayFromRack => "pa.rack",
            Self::PickFromRack => "pick",
        }
    }
}

impl fmt::Display for StationArrivalSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct StationArrivalPlan {
    pub station_id: StationId,
    pub source: StationArrivalSource,
    pub target_arrival: SimTime,
    pub quoted_arrival: SimTime,

    pub htm_source_ready_at: Option<SimTime>,

    pub htm_pickup_from: Option<CellId>,

    pub htm_pickup_to: Option<CellId>,

    pub htm_pickup_travel: Option<SimDuration>,

    pub quoted_relay_pickup_at: Option<SimTime>,

    pub quoted_htm_to_relay: Option<SimDuration>,

    pub relay_pickup_at: Option<SimTime>,
}

impl StationArrivalPlan {
    pub fn new(
        station_id: StationId,
        source: StationArrivalSource,
        target_arrival: SimTime,
        quoted_arrival: SimTime,
    ) -> Self {
        Self {
            station_id,
            source,
            target_arrival,
            quoted_arrival,
            htm_source_ready_at: None,
            htm_pickup_from: None,
            htm_pickup_to: None,
            htm_pickup_travel: None,
            quoted_relay_pickup_at: None,
            quoted_htm_to_relay: None,
            relay_pickup_at: None,
        }
    }

    pub fn with_htm_source_ready_at(mut self, htm_source_ready_at: SimTime) -> Self {
        self.htm_source_ready_at = Some(htm_source_ready_at);
        self
    }
}

#[derive(Clone, Copy, Debug)]
struct StationArrivalActual {
    station_id: StationId,
    source: StationArrivalSource,
    robot_id: RobotId,
    target_arrival: SimTime,
    quoted_arrival: SimTime,
    actual_arrival: SimTime,
    htm_source_ready_at: SimTime,
    quoted_relay_pickup_at: Option<SimTime>,
    relay_pickup_at: Option<SimTime>,
}

#[derive(Clone, Default)]
pub struct StationArrivalDiagnostics {
    actuals: Arc<Mutex<Vec<StationArrivalActual>>>,
}

impl StationArrivalDiagnostics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&self, plan: StationArrivalPlan, robot_id: RobotId, actual_arrival: SimTime) {
        let actual = StationArrivalActual {
            station_id: plan.station_id,
            source: plan.source,
            robot_id,
            target_arrival: plan.target_arrival,
            quoted_arrival: plan.quoted_arrival,
            actual_arrival,
            htm_source_ready_at: plan.htm_source_ready_at.unwrap_or(plan.quoted_arrival),
            quoted_relay_pickup_at: plan.quoted_relay_pickup_at,
            relay_pickup_at: plan.relay_pickup_at,
        };
        self.actuals.lock().unwrap().push(actual);
    }

    pub(crate) fn status_report(&self) -> StationArrivalDiagnosticsReport<'_> {
        StationArrivalDiagnosticsReport { diagnostics: self }
    }
}

pub(crate) struct StationArrivalDiagnosticsReport<'a> {
    diagnostics: &'a StationArrivalDiagnostics,
}

pub struct Inner {
    pub id: StationId,
    pub station_type: StationType,
    pub presentation_cell: CellId,
    pub queue_path: Vec<CellId>,
    pub exit_direction: CardinalDirection,

    pub bins: usize,
}

impl Inner {
    pub fn queue_depth(&self) -> usize {
        self.queue_path.len()
    }

    pub fn details(&self) -> StationDetails {
        StationDetails { id: self.id, station_type: self.station_type, cell: self.presentation_cell }
    }
}

impl std::fmt::Debug for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Station")
            .field("id", &self.id)
            .field("type", &self.station_type)
            .field("cell", &self.presentation_cell)
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct Station(Arc<Inner>);

impl Station {
    pub fn new(
        id: StationId,
        station_type: StationType,
        presentation_cell: CellId,
        queue_path: Vec<CellId>,
        exit_direction: CardinalDirection,
        bins: usize,
    ) -> Self {
        Self(Arc::new(Inner { id, station_type, presentation_cell, queue_path, exit_direction, bins }))
    }

    pub fn details(&self) -> StationDetails {
        self.0.details()
    }
}

impl Deref for Station {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

struct StationSetInner {
    all: BTreeMap<StationId, Station>,
    pick: Vec<Station>,
    putaway: Vec<Station>,
    box_induction: Vec<Station>,
    extraction: Vec<Station>,
}

#[derive(Clone)]
pub struct StationSet(Arc<StationSetInner>);

pub(crate) struct StationStatusReport<'a> {
    stations: &'a StationSet,
    oracle: &'a dyn ReservationOracle,
    report_horizon_ms: u64,
}

impl StationSet {
    pub fn new(stations: Vec<Station>) -> Self {
        let mut all = BTreeMap::new();
        let mut pick = Vec::new();
        let mut putaway = Vec::new();
        let mut box_induction = Vec::new();
        let mut extraction = Vec::new();

        for s in stations {
            match s.station_type {
                StationType::Pick => pick.push(s.clone()),
                StationType::Putaway => putaway.push(s.clone()),
                // Bucketed so a flow module can find them; nothing reads
                // these lists yet (gaps-plan Gap 4).
                StationType::BoxInduction => box_induction.push(s.clone()),
                StationType::Extraction => extraction.push(s.clone()),
                StationType::ContainerPickup => {}
            }
            all.insert(s.id, s);
        }

        Self(Arc::new(StationSetInner { all, pick, putaway, box_induction, extraction }))
    }

    pub fn get(&self, id: StationId) -> Option<&Station> {
        self.0.all.get(&id)
    }

    pub fn pick_stations(&self) -> &[Station] {
        &self.0.pick
    }

    pub fn putaway_stations(&self) -> &[Station] {
        &self.0.putaway
    }

    /// Where an empty HTM is handed an empty order box. Unread so far.
    pub fn box_induction_stations(&self) -> &[Station] {
        &self.0.box_induction
    }

    /// Where an order's contents leave the building. Unread so far.
    pub fn extraction_stations(&self) -> &[Station] {
        &self.0.extraction
    }

    pub fn presentation_cells(&self) -> impl Iterator<Item = CellId> + '_ {
        self.0.all.values().map(|station| station.presentation_cell)
    }

    pub(crate) fn status_report<'a>(
        &'a self,
        oracle: &'a dyn ReservationOracle,
        report_horizon_ms: u64,
    ) -> StationStatusReport<'a> {
        StationStatusReport { stations: self, oracle, report_horizon_ms }
    }
}

impl fmt::Display for StationStatusReport<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn seconds(ms: u64) -> f64 {
            ms as f64 / 1000.0
        }

        fn open_or_seconds(has_open: bool, ms: u64) -> String {
            if has_open { "open".to_string() } else { format!("{:.1}", seconds(ms)) }
        }

        fn station_type_label(station_type: StationType) -> &'static str {
            match station_type {
                StationType::Pick => "pick",
                StationType::Putaway => "put",
                StationType::BoxInduction => "box",
                StationType::Extraction => "extract",
                StationType::ContainerPickup => "container pickup",
            }
        }

        writeln!(f, "stations:")?;
        writeln!(
            f,
            "  {:>3} {:>4} {:>6} {:>4} {:>4} {:>10} {:>6} {:>5} {:>9} {:>9} {:>9} {:>10}",
            "id",
            "type",
            "cell",
            "res",
            "open",
            "frontier_s",
            "duty%",
            "gaps",
            "gap_s",
            "max_gap_s",
            "iat_sd_s",
            "max_hold_s",
        )?;
        for station in self.stations.0.all.values() {
            let stats = self.oracle.cell_duty_stats(station.presentation_cell, self.report_horizon_ms);
            let iat_sd_ms = stats.start_interval_stddev_ms;
            writeln!(
                f,
                "  {:>3} {:>4} {:>6} {:>4} {:>4} {:>10} {:>6.1} {:>5} {:>9.1} {:>9.1} {:>9.1} {:>10}",
                station.id.0,
                station_type_label(station.station_type),
                station.presentation_cell.0,
                stats.reservations,
                stats.open_ended,
                open_or_seconds(stats.open_ended > 0, stats.frontier_ms),
                stats.duty_pct(),
                stats.gap_count,
                seconds(stats.gap_ms),
                seconds(stats.longest_gap_ms),
                seconds(iat_sd_ms.round() as u64),
                open_or_seconds(stats.open_ended > 0, stats.longest_hold_ms),
            )?;
        }
        Ok(())
    }
}

impl fmt::Display for StationArrivalDiagnosticsReport<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let actuals = self.diagnostics.actuals.lock().unwrap();
        if actuals.is_empty() {
            return Ok(());
        }

        #[derive(Default)]
        struct Acc {
            count: usize,
            quote_errors: Vec<SimOffset>,
            target_errors: Vec<SimOffset>,
            full_htm_ms: Vec<u64>,
            relay_to_station_ms: Vec<u64>,
            relay_pickup_errors: Vec<SimOffset>,
            last_robot: Option<RobotId>,
        }

        let mut rows: BTreeMap<(StationId, StationArrivalSource), Acc> = BTreeMap::new();
        for actual in actuals.iter() {
            let row = rows.entry((actual.station_id, actual.source)).or_default();
            row.count += 1;
            row.quote_errors.push(actual.actual_arrival.diff(actual.quoted_arrival));
            row.target_errors.push(actual.actual_arrival.diff(actual.target_arrival));
            row.full_htm_ms.push((actual.actual_arrival - actual.htm_source_ready_at).as_ms());
            if let Some(relay_pickup_at) = actual.relay_pickup_at {
                row.relay_to_station_ms.push((actual.actual_arrival - relay_pickup_at).as_ms());
                if let Some(quoted_relay_pickup_at) = actual.quoted_relay_pickup_at {
                    row.relay_pickup_errors.push(relay_pickup_at.diff(quoted_relay_pickup_at));
                }
            }
            row.last_robot = Some(actual.robot_id);
        }

        writeln!(f, "station_arrival_quotes (quote_err = actual − quoted; target_err = actual − target):")?;
        writeln!(
            f,
            "  {:>3} {:>6} {:>5} {:>12} {:>12} {:>12} {:>12} {:>12} {:>12} {:>9}",
            "id",
            "source",
            "n",
            "quote_err_s",
            "quote_sd_s",
            "target_err_s",
            "target_sd_s",
            "pickup_err_s",
            "full_htm_s",
            "rly_stn_s",
        )?;
        for ((station_id, source), row) in rows {
            writeln!(
                f,
                "  {:>3} {:>6} {:>5} {:>12.1} {:>12.1} {:>12.1} {:>12.1} {:>12.1} {:>12.1} {:>9.1}",
                station_id.0,
                source.label(),
                row.count,
                mean_offset(&row.quote_errors) / 1000.0,
                stddev_offset(&row.quote_errors) / 1000.0,
                mean_offset(&row.target_errors) / 1000.0,
                stddev_offset(&row.target_errors) / 1000.0,
                mean_offset(&row.relay_pickup_errors) / 1000.0,
                mean_u64(&row.full_htm_ms) / 1000.0,
                mean_u64(&row.relay_to_station_ms) / 1000.0,
            )?;
        }
        Ok(())
    }
}

fn mean_offset(values: &[SimOffset]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().map(|v| v.as_ms() as f64).sum::<f64>() / values.len() as f64
}

fn mean_u64(values: &[u64]) -> f64 {
    if values.is_empty() { 0.0 } else { values.iter().sum::<u64>() as f64 / values.len() as f64 }
}

fn stddev_offset(values: &[SimOffset]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let mean = mean_offset(values);
    let variance = values
        .iter()
        .map(|v| {
            let delta = v.as_ms() as f64 - mean;
            delta * delta
        })
        .sum::<f64>()
        / values.len() as f64;
    variance.sqrt()
}

#[cfg(test)]
pub fn test_station_set() -> StationSet {
    use desgraph::{CardinalDirection, CellId, StationId, StationType};
    StationSet::new(vec![
        Station::new(
            StationId(1),
            StationType::Pick,
            CellId(100),
            vec![CellId(97), CellId(98), CellId(99)],
            CardinalDirection::East,
            1,
        ),
        Station::new(
            StationId(2),
            StationType::Pick,
            CellId(200),
            vec![CellId(197), CellId(198), CellId(199)],
            CardinalDirection::East,
            1,
        ),
        Station::new(
            StationId(3),
            StationType::Putaway,
            CellId(300),
            vec![CellId(297), CellId(298), CellId(299)],
            CardinalDirection::North,
            1,
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::{Station, StationSet};
    use desgraph::{CardinalDirection, CellId, StationId, StationType};

    #[test]
    fn deref_gives_immutable_field_access() {
        let s = Station::new(
            StationId(1),
            StationType::Pick,
            CellId(10),
            vec![CellId(7), CellId(8), CellId(9)],
            CardinalDirection::East,
            1,
        );
        assert_eq!(s.id, StationId(1));
        assert_eq!(s.station_type, StationType::Pick);
        assert_eq!(s.presentation_cell, CellId(10));
        assert_eq!(s.queue_depth(), 3);
        assert_eq!(s.exit_direction, CardinalDirection::East);
    }

    #[test]
    fn details_snapshot() {
        let s = Station::new(StationId(1), StationType::Pick, CellId(10), vec![], CardinalDirection::East, 1);
        let d = s.details();
        assert_eq!(d.id, StationId(1));
    }

    #[test]
    fn station_set_uses_test_helper() {
        let set = super::test_station_set();
        assert_eq!(set.pick_stations().len(), 2);
        assert_eq!(set.putaway_stations().len(), 1);
        assert!(set.get(StationId(1)).is_some());
        assert!(set.get(StationId(99)).is_none());
    }
}
