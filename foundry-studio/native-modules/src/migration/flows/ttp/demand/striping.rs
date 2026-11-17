//! Destination pick-station **striping**: lays the destinations of a bulk
//! demand pool out as contiguous intervals over the physically-ordered pick
//! stations, interval length proportional to each destination's share of
//! total demand units. This is the static v1 stripe map of issue #245 —
//! computed once from the full pool when `LoadOrders` is scheduled (spec decision D1);
//! dynamic restriping later replaces [`StripeMap::compute`], not the
//! read-only interface. Phase 1 consumes it for reporting/visualization
//! only; putaway location selection joins in phase 2
//! (spec: specs/putaway_pick_locality/plan.md §4.1).
//!
//! All apportionment arithmetic is exact integer math (u128 cross
//! multiplication) — no floats, so results are deterministic and
//! reproducible across processes by construction.

use std::collections::BTreeMap;

use desgraph::StationId;
use desmap::WarehouseMap;
use rustc_hash::FxHashMap;

use crate::events::{EventSink, SimEvent, StripePlanEntry, StripePlanEvent};
use crate::model::Destination;
use crate::model::station::StationSet;
use crate::pathfinding::estimate::cell_point_mm;
use crate::scenario_gen::DemandPool;
use crate::scenario_gen::demand_pool::ProductLine;

/// Why a stripe map could not be computed. All variants are configuration
/// or scenario-shape problems the caller should surface, not runtime
/// conditions to retry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StripeError {
    /// No pick stations were supplied — nothing to stripe over.
    NoPickStations,
    /// The demand pool holds zero units — shares are undefined.
    NoDemand,
    /// Two pick stations project to the same coordinate on the dominant
    /// axis, so "physical order along the bank" is ambiguous. Striping
    /// refuses to guess; an explicit station-order config is the escape
    /// hatch when a layout genuinely needs one (spec §4.1).
    AmbiguousStationOrder { first: StationId, second: StationId },
}

impl std::fmt::Display for StripeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StripeError::NoPickStations => write!(f, "striping: no pick stations supplied"),
            StripeError::NoDemand => write!(f, "striping: demand pool holds zero units"),
            StripeError::AmbiguousStationOrder { first, second } => write!(
                f,
                "striping: stations {first:?} and {second:?} share a dominant-axis coordinate; \
                 physical order is ambiguous"
            ),
        }
    }
}

impl std::error::Error for StripeError {}

/// One destination's stripe: its interval on the cumulative-unit line and
/// the stations that interval overlaps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StripeEntry {
    pub destination: Destination,
    /// Total demand units for this destination in the pool.
    pub units: u64,
    /// Half-open interval `[cum_lo, cum_hi)` on the cumulative-unit line
    /// `[0, total_units)`, in destination (first-appearance) order.
    pub cum_lo: u64,
    pub cum_hi: u64,
    /// `(station, overlap)` for every station whose interval overlaps this
    /// destination's with nonzero length, in physical order; `overlap` is
    /// the overlap length in exact unit·N space (a station's share of the
    /// destination is `overlap / (units · station_count)`). Contiguous by
    /// construction. Empty iff `units == 0`.
    pub eligible: Vec<(StationId, u64)>,
}

impl StripeEntry {
    /// The eligible stations without their overlap lengths, in physical
    /// order.
    pub fn eligible_ids(&self) -> impl Iterator<Item = StationId> + '_ {
        self.eligible.iter().map(|&(station, _)| station)
    }
}

/// The computed stripe map: destinations in first-appearance order, each
/// with its eligible pick stations. Read-only after construction — this is
/// the interface #245 requires putaway location selection to consume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StripeMap {
    /// Pick stations in physical order along the bank (see
    /// [`order_stations_physically`]).
    stations: Vec<StationId>,
    /// Total units across the pool (the length of the cumulative-unit line).
    total_units: u64,
    /// One entry per destination, in first-appearance (product-sequence)
    /// order.
    entries: Vec<StripeEntry>,
    /// Destination → index into `entries`.
    index: BTreeMap<Destination, usize>,
}

impl StripeMap {
    /// Compute the static stripe map from the full product sequence and the
    /// pick stations (with their grid positions).
    ///
    /// Destination order is first appearance in `lines` — the same order the
    /// order builder walks, so stripe adjacency mirrors demand adjacency.
    /// Station order is physical (dominant-axis projection); raw station-id
    /// order is deliberately never used (#245).
    pub fn compute(lines: &[ProductLine], pick_stations: &[(StationId, (i64, i64))]) -> Result<Self, StripeError> {
        let stations = order_stations_physically(pick_stations)?;

        // Per-destination totals, kept in first-appearance order by the vector
        // itself so no later lookup can miss a destination it just recorded.
        let mut order: Vec<(Destination, u64)> = Vec::new();
        let mut position: FxHashMap<Destination, usize> = FxHashMap::default();
        for line in lines {
            let dest = Destination::DistributionCenter(line.distribution_center.clone());
            match position.get(&dest) {
                Some(&i) => order[i].1 += u64::from(line.qty),
                None => {
                    position.insert(dest.clone(), order.len());
                    order.push((dest, u64::from(line.qty)));
                }
            }
        }
        let total_units: u64 = order.iter().map(|&(_, units)| units).sum();
        if total_units == 0 {
            return Err(StripeError::NoDemand);
        }

        // Interval model (spec §4.1, exact integer arithmetic): destination
        // d owns [cum_lo, cum_hi) on the unit line [0, U); station k owns
        // [k·U/N, (k+1)·U/N). Overlap tests compare N·cum against k·U in
        // u128 so no division or floats ever happen.
        let n = stations.len() as u128;
        let u = u128::from(total_units);
        let mut entries: Vec<StripeEntry> = Vec::with_capacity(order.len());
        let mut cum: u64 = 0;
        for (dest, units) in order {
            let (cum_lo, cum_hi) = (cum, cum + units);
            cum = cum_hi;

            let (lo, hi) = (u128::from(cum_lo), u128::from(cum_hi));
            let mut eligible: Vec<(StationId, u64)> = Vec::new();
            // A zero-volume destination has an empty interval: no stations,
            // no primary — the fallback path (spec §4.1). Without this
            // guard, an empty interval strictly inside station k would pass
            // the strict-overlap predicate below with overlap length 0 and
            // fabricate an eligible station (Opus review, 2026-07-30).
            let stations_to_scan = if units == 0 { &[][..] } else { &stations[..] };
            for (k, &station) in stations_to_scan.iter().enumerate() {
                let (station_lo, station_hi) = ((k as u128) * u, (k as u128 + 1) * u);
                // Overlap of [lo, hi) with station k's [k·U/N, (k+1)·U/N),
                // in unit·N space: a boundary touched exactly does not extend
                // eligibility, and measuring both ends from the station's own
                // left edge bounds the length by that station's width
                // `u` = total_units, so it lands in u64 by construction.
                if lo * n >= station_hi || hi * n <= station_lo {
                    continue;
                }
                let start = (lo * n).max(station_lo) - station_lo;
                let end = (hi * n).min(station_hi) - station_lo;
                eligible.push((station, (end - start) as u64));
            }
            // No "primary" station: the allocation semantics are
            // proportional to the overlap shares (D13, Daniel 2026-07-30) —
            // a majority-station marker is a display concern, derived by
            // surfaces from the shares.
            entries.push(StripeEntry { destination: dest, units, cum_lo, cum_hi, eligible });
        }

        let index = entries.iter().enumerate().map(|(i, e)| (e.destination.clone(), i)).collect();
        Ok(Self { stations, total_units, entries, index })
    }

    /// Pick stations in physical order along the bank.
    pub fn stations(&self) -> &[StationId] {
        &self.stations
    }

    pub fn total_units(&self) -> u64 {
        self.total_units
    }

    /// All stripes, in destination first-appearance order.
    pub fn entries(&self) -> &[StripeEntry] {
        &self.entries
    }

    /// The stations eligible for `destination`, or `None` for a destination
    /// the pool never mentioned. `Some(vec![])` means the destination exists
    /// with zero volume — callers fall back exactly as for `None`.
    pub fn eligible(&self, destination: &Destination) -> Option<Vec<StationId>> {
        self.index.get(destination).map(|&i| self.entries[i].eligible_ids().collect())
    }

    /// Snapshot this map as a [`crate::events::StripePlanEvent`] taking
    /// effect at `sim_time_ms`, for emission and persistence on the run
    /// profile. Static v1 emits once at t=0; dynamic restriping later
    /// calls this again per recomputation.
    pub fn to_event(&self, sim_time_ms: u64) -> StripePlanEvent {
        StripePlanEvent {
            sim_time_ms,
            stations: self.stations.clone(),
            total_units: self.total_units,
            entries: self
                .entries
                .iter()
                .map(|e| StripePlanEntry {
                    destination: e.destination.clone(),
                    units: e.units,
                    eligible: e.eligible.clone(),
                })
                .collect(),
        }
    }
}

/// Order pick stations physically along the bank: project positions onto
/// the dominant axis (the one with the larger squared-deviation sum) and
/// sort by that coordinate. Two stations sharing a dominant-axis coordinate
/// make the order ambiguous → [`StripeError::AmbiguousStationOrder`], so a
/// wrong order can never be silently produced (spec §4.1; #245 forbids raw
/// id order silently becoming stripe order).
fn order_stations_physically(pick_stations: &[(StationId, (i64, i64))]) -> Result<Vec<StationId>, StripeError> {
    if pick_stations.is_empty() {
        return Err(StripeError::NoPickStations);
    }
    let n = pick_stations.len() as i128;
    let (sum_x, sum_y) =
        pick_stations.iter().fold((0i128, 0i128), |(sx, sy), &(_, (x, y))| (sx + i128::from(x), sy + i128::from(y)));
    // Compare N²·variance via Σ(N·c − Σc)² — integer-exact.
    let (ssd_x, ssd_y) = pick_stations.iter().fold((0i128, 0i128), |(ax, ay), &(_, (x, y))| {
        let dx = n * i128::from(x) - sum_x;
        let dy = n * i128::from(y) - sum_y;
        (ax + dx * dx, ay + dy * dy)
    });

    let mut ordered: Vec<(StationId, (i64, i64))> = pick_stations.to_vec();
    // Dominant coordinate first; the (x, y) tail keeps the sort total so
    // the ambiguity check below sees deterministic neighbors.
    let key = |&(_, (x, y)): &(StationId, (i64, i64))| if ssd_x >= ssd_y { (x, y) } else { (y, x) };
    ordered.sort_by_key(|s| (key(s), s.0));

    for pair in ordered.windows(2) {
        if key(&pair[0]).0 == key(&pair[1]).0 {
            return Err(StripeError::AmbiguousStationOrder { first: pair[0].0, second: pair[1].0 });
        }
    }
    Ok(ordered.into_iter().map(|(id, _)| id).collect())
}

/// Emit the physical station assignment for a bulk demand pool.
pub(crate) fn emit(
    stations: &StationSet,
    warehouse: &WarehouseMap,
    demand_pool: &Option<DemandPool>,
    events: Option<&dyn EventSink>,
) {
    if let Some(pool) = demand_pool.as_ref() {
        // Striping only ORDERS stations physically, so whole-millimetre
        // positions serve exactly as grid ordinals did. A pick station whose
        // presentation cell has no position of any kind is a map defect the
        // stripe plan cannot work around.
        let pick_positions: Vec<(StationId, (i64, i64))> = stations
            .pick_stations()
            .iter()
            .map(|s| {
                let p = cell_point_mm(warehouse, s.presentation_cell)
                    .unwrap_or_else(|| panic!("pick station {:?} presentation cell has no position of any kind", s.id));
                (s.id, (p.x_mm.round() as i64, p.y_mm.round() as i64))
            })
            .collect();
        match StripeMap::compute(pool.lines(), &pick_positions) {
            Ok(map) => {
                for e in map.entries() {
                    eprintln!(
                        "[striping] {} units={} stations={:?}",
                        e.destination,
                        e.units,
                        e.eligible_ids().map(|s| s.0).collect::<Vec<_>>(),
                    );
                }
                if let Some(events) = events {
                    events.send(SimEvent::StripePlan(map.to_event(0)));
                }
            }
            Err(StripeError::NoDemand) | Err(StripeError::NoPickStations) => {}
            // Ambiguous physical station order must fail loud, never a silently guessed stripe order.
            Err(e @ StripeError::AmbiguousStationOrder { .. }) => {
                panic!("destination striping: {e}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{SimTime, SkuId};

    fn line(dc: &str, qty: u32) -> ProductLine {
        ProductLine { putaway_net_time: SimTime::ZERO, distribution_center: dc.to_string(), sku: SkuId(1), qty }
    }

    fn dest(dc: &str) -> Destination {
        Destination::DistributionCenter(dc.to_string())
    }

    /// Stations on a horizontal bank: x varies, y constant.
    fn bank(n: u32) -> Vec<(StationId, (i64, i64))> {
        (0..n).map(|i| (StationId(100 + i), ((10 + 5 * i as i64), 3))).collect()
    }

    #[test]
    fn equal_volumes_split_contiguously() {
        let lines = vec![line("A", 4), line("B", 4)];
        let map = StripeMap::compute(&lines, &bank(4)).unwrap();
        assert_eq!(map.eligible(&dest("A")).unwrap(), &[StationId(100), StationId(101)]);
        assert_eq!(map.eligible(&dest("B")).unwrap(), &[StationId(102), StationId(103)]);
    }

    #[test]
    fn straddling_destination_gets_both_stations_with_proportional_shares() {
        // U=8 over 2 stations (width 4): A [0,3) → s0; B [3,6) straddles
        // the boundary at 4 → s0+s1 with overlaps 1:2 (in unit·N space,
        // 2:4) — allocation is proportional to these shares, there is no
        // primary (D13); C [6,8) → s1.
        let lines = vec![line("A", 3), line("B", 3), line("C", 2)];
        let map = StripeMap::compute(&lines, &bank(2)).unwrap();
        assert_eq!(map.eligible(&dest("A")).unwrap(), &[StationId(100)]);
        let b = &map.entries()[1];
        assert_eq!(b.destination, dest("B"));
        assert_eq!(b.eligible, [(StationId(100), 2), (StationId(101), 4)]);
        assert_eq!(map.eligible(&dest("C")).unwrap(), &[StationId(101)]);
    }

    #[test]
    fn exact_boundary_does_not_extend_eligibility() {
        // A is exactly one station wide: [0,4) of U=8, N=2 → s0 only.
        let lines = vec![line("A", 4), line("B", 4)];
        let map = StripeMap::compute(&lines, &bank(2)).unwrap();
        assert_eq!(map.eligible(&dest("A")).unwrap(), &[StationId(100)]);
        assert_eq!(map.eligible(&dest("B")).unwrap(), &[StationId(101)]);
    }

    #[test]
    fn zero_volume_destination_has_no_stations() {
        // The empty interval must yield no stations in BOTH positions: on a
        // station boundary (A=5 puts Z's point exactly at U/2) and strictly
        // inside a station (A=4 puts it interior to station 0) — the
        // interior case is where the strict-overlap predicate alone would
        // fabricate an eligible station with overlap 0 (Opus review).
        for first_units in [5u32, 4] {
            let lines = vec![line("A", first_units), line("Z", 0), line("B", 10 - first_units)];
            let map = StripeMap::compute(&lines, &bank(2)).unwrap();
            assert_eq!(map.eligible(&dest("Z")), Some(vec![]), "A={first_units}");
        }
        // And unknown destinations are None, distinguishable by callers.
        let lines = vec![line("A", 5), line("B", 5)];
        let map = StripeMap::compute(&lines, &bank(2)).unwrap();
        assert_eq!(map.eligible(&dest("NOPE")), None);
    }

    #[test]
    fn first_appearance_order_is_preserved() {
        let lines = vec![line("B", 1), line("A", 2), line("B", 3)];
        let map = StripeMap::compute(&lines, &bank(2)).unwrap();
        let order: Vec<&Destination> = map.entries().iter().map(|e| &e.destination).collect();
        assert_eq!(order, [&dest("B"), &dest("A")]);
        assert_eq!(map.entries()[0].units, 4);
    }

    #[test]
    fn more_destinations_than_stations_tile_without_gaps() {
        // 26 destinations of mixed volume over 10 stations — the cencora_v3
        // regime. Every destination gets 1–2 contiguous stations; the union
        // in destination order walks the bank monotonically with no gaps.
        let lines: Vec<ProductLine> = (0..26).map(|i| line(&format!("DC{i:02}"), 20 + (i % 7) * 13)).collect();
        let map = StripeMap::compute(&lines, &bank(10)).unwrap();
        let stations = map.stations().to_vec();
        let mut covered: Vec<StationId> = Vec::new();
        for entry in map.entries() {
            assert!(!entry.eligible.is_empty(), "{:?} got no stations", entry.destination);
            assert!(entry.eligible.len() <= 2, "{:?} spans {} stations", entry.destination, entry.eligible.len());
            // Contiguity within the entry, and monotone continuation of the
            // overall tiling (stripes may share a boundary station).
            for s in entry.eligible_ids() {
                let pos = stations.iter().position(|t| *t == s).unwrap();
                if let Some(last) = covered.last() {
                    let last_pos = stations.iter().position(|t| t == last).unwrap();
                    assert!(pos == last_pos || pos == last_pos + 1, "gap or backtrack at {:?}", entry.destination);
                }
                if covered.last() != Some(&s) {
                    covered.push(s);
                }
            }
        }
        assert_eq!(covered, stations, "every station must be covered exactly in order");
    }

    #[test]
    fn recompute_is_identical() {
        let lines: Vec<ProductLine> = (0..26).map(|i| line(&format!("DC{i:02}"), 20 + (i % 7) * 13)).collect();
        let a = StripeMap::compute(&lines, &bank(10)).unwrap();
        let b = StripeMap::compute(&lines, &bank(10)).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn vertical_bank_orders_by_y() {
        // x constant → dominant axis is y.
        let stations = vec![(StationId(1), (7, 30)), (StationId(2), (7, 10)), (StationId(3), (7, 20))];
        let lines = vec![line("A", 1), line("B", 1), line("C", 1)];
        let map = StripeMap::compute(&lines, &stations).unwrap();
        assert_eq!(map.stations(), &[StationId(2), StationId(3), StationId(1)]);
    }

    #[test]
    fn ambiguous_station_order_is_refused() {
        let stations = vec![(StationId(1), (10, 3)), (StationId(2), (10, 4)), (StationId(3), (20, 3))];
        let lines = vec![line("A", 1)];
        let err = StripeMap::compute(&lines, &stations).unwrap_err();
        assert!(matches!(err, StripeError::AmbiguousStationOrder { .. }), "got {err:?}");
    }

    #[test]
    fn empty_inputs_are_errors() {
        assert_eq!(StripeMap::compute(&[line("A", 1)], &[]).unwrap_err(), StripeError::NoPickStations);
        assert_eq!(StripeMap::compute(&[line("A", 0)], &bank(2)).unwrap_err(), StripeError::NoDemand);
        assert_eq!(StripeMap::compute(&[], &bank(2)).unwrap_err(), StripeError::NoDemand);
    }
}
