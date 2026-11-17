//! Within `pathfinding`, `nearest` answers "which of these candidates is
//! closest?" — picking one cell out of many by real path cost rather than by
//! straight-line distance.
//!
//! Two-stage so the answer stays affordable: rank every candidate by Manhattan
//! grid time (cheap, no search), then exact-refine only the top
//! `SELECT_NEAREST_PRESCREEN_TOP_K` through the `Cached` pathfinder in
//! parallel. This preserves the existing single-model shortlist heuristic.
//! Heterogeneous callers use `select_all`: their execution costs need not
//! have the same ordering as a shared geometric shortlist.
//!
//! Directional: `select_nearest_to` treats the candidate as the path source,
//! `select_nearest_from` as the destination. Callers supply an eligibility
//! predicate and a scoring fn, so this module owns the search economics while
//! the caller owns what "best" means.

use desgraph::types::CellId;
use desmap::WarehouseMap;
use rayon::prelude::*;

use crate::model::{SimDuration, SimTime};

use super::estimate::{cell_point_mm, manhattan_mm};
use super::{Cached, Orientation, PathCacheKey, PathfinderError};

/// Number of eligible prescreened candidates to exact-refine per batch.
/// Each candidate in a batch performs one cached spatial path lookup; cache
/// misses run one A* search.
const SELECT_NEAREST_PRESCREEN_TOP_K: usize = 10;

/// Candidate with a floor cell used for nearest-path selection.
pub trait CellCandidate {
    /// Candidate floor cell: the path's source when
    /// [`Navigation::select_htm_to`](crate::model::navigation::Navigation::select_htm_to)
    /// or `select_nearest_to` chooses among candidates, and its destination
    /// when `select_nearest_from` does.
    fn cell(&self) -> CellId;
}

/// Exact path-scored result selected by a nearest-cell selector.
#[derive(Clone, Debug)]
pub struct CellSelection<T> {
    pub candidate: T,
    /// Exact cached floor travel time for the selected path leg.
    pub travel_time: SimDuration,
    /// Caller-defined score using `travel_time`. For the default relay cases
    /// this is `candidate.relay_ready_at() + candidate.pre_path_cost() +
    /// travel_time`; callers with task-level timing constraints can score
    /// differently while preserving the same prescreen/refine mechanics.
    pub(crate) completion_time: SimTime,
}

#[derive(Debug, thiserror::Error)]
pub enum PathSelectionError {
    #[error("robot model binding: {0}")]
    ModelBinding(String),
    #[error("no eligible reachable candidate for reference cell {reference:?}")]
    NoCandidate { reference: CellId },
    /// The cell carries neither a world position nor a grid position, so it
    /// cannot be prescreened at all. A cell with only one of the two is fine:
    /// see `pathfinding::estimate`.
    #[error("cell {cell:?} has no position (neither world nor grid)")]
    MissingCellPosition { cell: CellId },
    #[error(transparent)]
    Pathfinder(#[from] PathfinderError),
}

struct PrescreenedCandidate<T> {
    candidate: T,
    estimated_completion_time: SimTime,
}

pub(crate) struct NearestCellSelector<'a> {
    cache: &'a Cached,
    map: &'a WarehouseMap,
}

impl<'a> NearestCellSelector<'a> {
    /// Build a selector that quotes travel with the cache's
    /// [`travel_time`](Cached::travel_time), so quotes track real chain costs
    /// under the run's drive.
    pub(crate) fn new(cache: &'a Cached, map: &'a WarehouseMap) -> Self {
        Self { cache, map }
    }

    /// Exact refinement across every eligible candidate. Heterogeneous fleets
    /// cannot use a shared-speed top-k screen: a distant faster model may win.
    pub(crate) fn select_all<C, I, Score, Quote>(
        candidates: I,
        reference: CellId,
        score: Score,
        quote: Quote,
    ) -> Result<CellSelection<C>, PathSelectionError>
    where
        I: IntoIterator<Item = C>,
        C: Clone + Send + Sync,
        Score: Fn(&C, SimDuration) -> Option<SimTime> + Sync,
        Quote: Fn(&C) -> Result<SimDuration, PathSelectionError> + Sync,
    {
        let candidates: Vec<_> = candidates.into_iter().collect();
        let exact: Result<Vec<_>, PathSelectionError> = candidates
            .par_iter()
            .map(|candidate| {
                let travel_time = match quote(candidate) {
                    Ok(time) => time,
                    Err(PathSelectionError::Pathfinder(
                        PathfinderError::Unreachable { .. } | PathfinderError::NoStaticRoute { .. },
                    )) => return Ok(None),
                    Err(error) => return Err(error),
                };
                Ok(score(candidate, travel_time).map(|completion_time| CellSelection {
                    candidate: candidate.clone(),
                    travel_time,
                    completion_time,
                }))
            })
            .collect();
        exact?
            .into_iter()
            .flatten()
            .min_by_key(|candidate| candidate.completion_time)
            .ok_or(PathSelectionError::NoCandidate { reference })
    }

    /// Select the best `from` candidate for a given `to` destination.
    ///
    /// - `eligible`: medium-cost domain filter, run after geometric prescreen to build top-k items.
    /// - `score`: candidate ordering key; used for manhattan prescreen and Pathfinder refinement.
    ///   Return `None` when task-specific timing makes the candidate unusable.
    pub(crate) fn select_nearest_to<C, I, Score, FromOrientation>(
        &self,
        candidates: I,
        to: CellId,
        from_orientation: FromOrientation,
        carrying_cargo: bool,
        eligible: impl FnMut(&C) -> bool,
        score: Score,
    ) -> Result<CellSelection<C>, PathSelectionError>
    where
        I: IntoIterator<Item = C>,
        C: CellCandidate + Clone + Send + Sync,
        Score: Fn(&C, SimDuration) -> Option<SimTime> + Sync,
        FromOrientation: Fn(&C) -> Orientation + Sync,
    {
        let to_point = cell_point_mm(self.map, to).ok_or(PathSelectionError::MissingCellPosition { cell: to })?;
        let manhattan_time = |cell: CellId| -> Result<SimDuration, PathSelectionError> {
            let point = cell_point_mm(self.map, cell).ok_or(PathSelectionError::MissingCellPosition { cell })?;
            Ok(self.cache.minimum_travel_time(manhattan_mm(point, to_point), carrying_cargo))
        };

        // Pre-size from the iterator's size_hint and push directly into a
        // `Vec<PrescreenedCandidate>` — avoids both `Vec<Result<_, _>>` and
        // the collect-into-Result control flow that previously dominated
        // this function (8.3% wall time per profile).
        let iter = candidates.into_iter();
        let (lower, upper) = iter.size_hint();
        let mut candidates: Vec<PrescreenedCandidate<C>> = Vec::with_capacity(upper.unwrap_or(lower));
        for candidate in iter {
            let travel_time = manhattan_time(candidate.cell())?;
            if let Some(estimated_completion_time) = score(&candidate, travel_time) {
                candidates.push(PrescreenedCandidate { estimated_completion_time, candidate });
            }
        }

        if candidates.is_empty() {
            return Err(PathSelectionError::NoCandidate { reference: to });
        }

        self.select_prescreened(candidates, to, eligible, score, |candidate| {
            if candidate.cell() == to {
                return Ok(Some(SimDuration::ZERO));
            }

            match self.cache.travel_time(PathCacheKey {
                from: candidate.cell(),
                from_orientation: from_orientation(candidate),
                to,
                carrying_cargo,
            }) {
                Ok(travel_time) => Ok(Some(travel_time)),
                Err(PathfinderError::Unreachable { .. } | PathfinderError::NoStaticRoute { .. }) => Ok(None),
                Err(err) => Err(PathSelectionError::Pathfinder(err)),
            }
        })
    }

    /// Select the best `to` candidate for a given `from` source.
    ///
    /// - `eligible`: medium-cost domain filter, run after geometric prescreen to build top-k items.
    /// - `score`: candidate ordering key; used for manhattan prescreen and Pathfinder refinement.
    ///   Return `None` when task-specific timing makes the candidate unusable.
    pub(crate) fn select_nearest_from<C, I, Score>(
        &self,
        candidates: I,
        from: CellId,
        from_orientation: Orientation,
        carrying_cargo: bool,
        eligible: impl FnMut(&C) -> bool,
        score: Score,
    ) -> Result<CellSelection<C>, PathSelectionError>
    where
        I: IntoIterator<Item = C>,
        C: CellCandidate + Clone + Send + Sync,
        Score: Fn(&C, SimDuration) -> Option<SimTime> + Sync,
    {
        let from_point = cell_point_mm(self.map, from).ok_or(PathSelectionError::MissingCellPosition { cell: from })?;
        let manhattan_time = |cell: CellId| -> Result<SimDuration, PathSelectionError> {
            let point = cell_point_mm(self.map, cell).ok_or(PathSelectionError::MissingCellPosition { cell })?;
            Ok(self.cache.minimum_travel_time(manhattan_mm(point, from_point), carrying_cargo))
        };

        // See `select_nearest_to` for the rationale; identical hot-path
        // prescreen, identical avoid-Result-in-Vec restructure.
        let iter = candidates.into_iter();
        let (lower, upper) = iter.size_hint();
        let mut candidates: Vec<PrescreenedCandidate<C>> = Vec::with_capacity(upper.unwrap_or(lower));
        for candidate in iter {
            let travel_time = manhattan_time(candidate.cell())?;
            if let Some(estimated_completion_time) = score(&candidate, travel_time) {
                candidates.push(PrescreenedCandidate { estimated_completion_time, candidate });
            }
        }

        if candidates.is_empty() {
            return Err(PathSelectionError::NoCandidate { reference: from });
        }

        self.select_prescreened(candidates, from, eligible, score, |candidate| {
            let to = candidate.cell();
            if from == to {
                return Ok(Some(SimDuration::ZERO));
            }

            match self.cache.travel_time(PathCacheKey { from, from_orientation, to, carrying_cargo }) {
                Ok(travel_time) => Ok(Some(travel_time)),
                Err(PathfinderError::Unreachable { .. } | PathfinderError::NoStaticRoute { .. }) => Ok(None),
                Err(err) => Err(PathSelectionError::Pathfinder(err)),
            }
        })
    }

    fn select_prescreened<C, Score, ExactTravel>(
        &self,
        mut candidates: Vec<PrescreenedCandidate<C>>,
        reference: CellId,
        mut eligible: impl FnMut(&C) -> bool,
        score: Score,
        exact_travel: ExactTravel,
    ) -> Result<CellSelection<C>, PathSelectionError>
    where
        C: CellCandidate + Clone + Send + Sync,
        Score: Fn(&C, SimDuration) -> Option<SimTime> + Sync,
        ExactTravel: Fn(&C) -> Result<Option<SimDuration>, PathSelectionError> + Sync,
    {
        // Stable sort by `estimated_completion_time` — keep this stable.
        // A previous partial-sort optimization (`select_nth_unstable_by_key` +
        // sort the prefix) traded the full O(n log n) for O(n) + O(K log K),
        // but `select_nth_unstable_by_key` reorders ties: when two candidates
        // have identical `estimated_completion_time` (common with shared
        // zones/vtm_floors/cycle costs), the post-partition order differs from
        // the original Vec order. Downstream, `best_exact_in_batch` picks via
        // `min_by_key` which returns the FIRST occurrence of the minimum, so
        // tie-resolution shifts which candidate (and therefore which relay/tote)
        // gets selected — observed empirically as +40s of htm_pickup pathfinding
        // when the selection trajectory diverged. Stable sort keeps the sim
        // deterministic across this code's optimization history.
        candidates.sort_by_key(|candidate| candidate.estimated_completion_time);

        let mut cursor = 0;
        while cursor < candidates.len() {
            let mut batch = Vec::with_capacity(SELECT_NEAREST_PRESCREEN_TOP_K);
            while cursor < candidates.len() && batch.len() < SELECT_NEAREST_PRESCREEN_TOP_K {
                let candidate = &candidates[cursor];
                cursor += 1;
                if eligible(&candidate.candidate) {
                    batch.push(candidate);
                }
            }

            if batch.is_empty() {
                continue;
            }

            if let Some(best) = Self::best_exact_in_batch(&batch, &score, &exact_travel)? {
                return Ok(best);
            }
        }

        Err(PathSelectionError::NoCandidate { reference })
    }

    fn best_exact_in_batch<C, Score, ExactTravel>(
        candidates: &[&PrescreenedCandidate<C>],
        score: &Score,
        exact_travel: &ExactTravel,
    ) -> Result<Option<CellSelection<C>>, PathSelectionError>
    where
        C: CellCandidate + Clone + Send + Sync,
        Score: Fn(&C, SimDuration) -> Option<SimTime> + Sync,
        ExactTravel: Fn(&C) -> Result<Option<SimDuration>, PathSelectionError> + Sync,
    {
        let exact: Result<Vec<_>, PathSelectionError> = candidates
            .par_iter()
            .map(|prescreened| {
                let candidate = &prescreened.candidate;
                let Some(travel_time) = exact_travel(candidate)? else {
                    return Ok(None);
                };
                let Some(completion_time) = score(candidate, travel_time) else {
                    return Ok(None);
                };
                Ok(Some(CellSelection { candidate: candidate.clone(), travel_time, completion_time }))
            })
            .collect();

        Ok(exact?.into_iter().flatten().min_by_key(|candidate| candidate.completion_time))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use desgraph::{Cell, CellKind, WarehouseGraphBuilder};
    use desgraph::{GridPos, WorldPos, ZoneId};
    use desmap::{SystemType, WarehouseMapBuilder};

    use super::*;
    use crate::pathfinding::Pathfinder;
    use crate::pathfinding::ReservationOracle;
    use crate::pathfinding::{CollisionConfig, Constant, DriveModel, HtmKinematics, Path, ReservedPath, WindowSink};
    use async_trait::async_trait;

    #[derive(Clone, Debug)]
    struct TestCandidate {
        id: u32,
        cell: CellId,
        ready: SimTime,
        pre_cost: SimDuration,
        eligible: bool,
    }

    impl TestCandidate {
        fn new(id: u32, cell: CellId) -> Self {
            Self { id, cell, ready: SimTime::ZERO, pre_cost: SimDuration::ZERO, eligible: true }
        }
    }

    impl CellCandidate for TestCandidate {
        fn cell(&self) -> CellId {
            self.cell
        }
    }

    fn default_score(candidate: &TestCandidate, travel: SimDuration) -> SimTime {
        candidate.ready + candidate.pre_cost + travel
    }

    fn default_score_opt(candidate: &TestCandidate, travel: SimDuration) -> Option<SimTime> {
        Some(default_score(candidate, travel))
    }

    struct FakePathfinder {
        travel: HashMap<(CellId, CellId), SimDuration>,
        calls: Arc<AtomicUsize>,
    }

    impl FakePathfinder {
        fn new(calls: Arc<AtomicUsize>) -> Self {
            Self { travel: HashMap::new(), calls }
        }

        fn with_travel(mut self, from: CellId, to: CellId, ms: u64) -> Self {
            self.travel.insert((from, to), SimDuration::ms(ms));
            self
        }
    }

    #[async_trait(?Send)]
    impl Pathfinder for FakePathfinder {
        fn find_path(
            &self,
            from: CellId,
            _from_orientation: Orientation,
            to: CellId,
            _drive: &dyn DriveModel,
            _start: SimTime,
        ) -> Result<Path, PathfinderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let Some(travel_time) = self.travel.get(&(from, to)).copied() else {
                return Err(PathfinderError::NoStaticRoute { from, to });
            };
            Ok(Path { waypoints: vec![], travel_time })
        }

        async fn reserve_path(
            &self,
            _from: CellId,
            _from_orientation: Orientation,
            _to: CellId,
            _drive: &dyn DriveModel,
            _oracle: &dyn ReservationOracle,
            _config: &CollisionConfig,
            _windows: &mut dyn WindowSink,
        ) -> Result<ReservedPath, PathfinderError> {
            unimplemented!("nearest selector tests use spatial find_path only")
        }

        fn traverse_duration(&self) -> SimDuration {
            SimDuration::ms(1)
        }
    }

    /// The fake ignores the drive it is handed, so any drives serve.
    fn drives() -> [Arc<dyn DriveModel>; 2] {
        let htm = HtmKinematics::uniform(1000, SimDuration::seconds(1));
        [Arc::new(Constant::htm(&htm, false)), Arc::new(Constant::htm(&htm, true))]
    }

    fn test_map(cells: impl IntoIterator<Item = CellId>) -> WarehouseMap {
        let mut builder = WarehouseGraphBuilder::new();
        for cell in cells {
            builder.add_cell(Cell {
                id: cell,
                kind: CellKind::Open,
                zone: ZoneId(0),
                sector: None,
                grid_pos: Some(GridPos { x: cell.0, y: 0 }),
                world_pos: None,
                barcode: None,
                rack_orientation: None,
                station_id: None,
                htm_rotation_forbidden: false,
            });
        }
        WarehouseMapBuilder::new(SystemType::Relay).graph(builder.build()).build()
    }

    #[test]
    fn select_nearest_to_paths_from_candidate_cell_to_target() {
        let calls = Arc::new(AtomicUsize::new(0));
        let map = test_map([CellId(0), CellId(5)]);
        let cache =
            Cached::new(FakePathfinder::new(calls.clone()).with_travel(CellId(5), CellId(0), 7), drives(), drives());
        let selector = NearestCellSelector::new(&cache, &map);

        let selected = selector
            .select_nearest_to(
                [TestCandidate::new(5, CellId(5))],
                CellId(0),
                |_| Orientation::East,
                true,
                |_| true,
                default_score_opt,
            )
            .unwrap();

        assert_eq!(selected.candidate.id, 5);
        assert_eq!(selected.travel_time, SimDuration::ms(7));
        assert_eq!(selected.completion_time, SimTime::ms(7));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn select_nearest_from_paths_from_reference_cell_to_candidate_cell() {
        let calls = Arc::new(AtomicUsize::new(0));
        let map = test_map([CellId(0), CellId(5)]);
        let cache =
            Cached::new(FakePathfinder::new(calls.clone()).with_travel(CellId(0), CellId(5), 7), drives(), drives());
        let selector = NearestCellSelector::new(&cache, &map);

        let selected = selector
            .select_nearest_from(
                [TestCandidate::new(5, CellId(5))],
                CellId(0),
                Orientation::East,
                true,
                |_| true,
                default_score_opt,
            )
            .unwrap();

        assert_eq!(selected.candidate.id, 5);
        assert_eq!(selected.travel_time, SimDuration::ms(7));
        assert_eq!(selected.completion_time, SimTime::ms(7));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn top_k_applies_after_eligibility_filtering() {
        let calls = Arc::new(AtomicUsize::new(0));
        let map = test_map((0..=12).map(CellId));
        let mut fake = FakePathfinder::new(calls.clone());
        fake = fake.with_travel(CellId(11), CellId(0), 11).with_travel(CellId(12), CellId(0), 12);
        let cache = Cached::new(fake, drives(), drives());
        let selector = NearestCellSelector::new(&cache, &map);
        let candidates = (1..=12)
            .map(|id| {
                let mut candidate = TestCandidate::new(id, CellId(id));
                candidate.eligible = id > 10;
                candidate
            })
            .collect::<Vec<_>>();

        let selected = selector
            .select_nearest_to(candidates, CellId(0), |_| Orientation::East, true, |c| c.eligible, default_score_opt)
            .unwrap();

        assert_eq!(selected.candidate.id, 11);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn falls_back_to_next_prescreen_batch_when_first_batch_has_no_route() {
        let calls = Arc::new(AtomicUsize::new(0));
        let map = test_map((0..=12).map(CellId));
        let mut fake = FakePathfinder::new(calls.clone());
        fake = fake.with_travel(CellId(11), CellId(0), 11).with_travel(CellId(12), CellId(0), 12);
        let cache = Cached::new(fake, drives(), drives());
        let selector = NearestCellSelector::new(&cache, &map);
        let candidates = (1..=12).map(|id| TestCandidate::new(id, CellId(id))).collect::<Vec<_>>();

        let selected = selector
            .select_nearest_to(candidates, CellId(0), |_| Orientation::East, true, |_| true, default_score_opt)
            .unwrap();

        assert_eq!(selected.candidate.id, 11);
        assert_eq!(calls.load(Ordering::SeqCst), 12);
    }

    /// A cell absent from the map has no position of any kind, so it still
    /// cannot be prescreened. Renamed from `missing_candidate_grid_position_is_fatal`:
    /// a missing *grid* position is no longer fatal on its own -- see
    /// `a_candidate_with_only_a_world_position_is_rankable`.
    #[test]
    fn a_candidate_with_no_position_at_all_is_fatal() {
        let calls = Arc::new(AtomicUsize::new(0));
        let map = test_map([CellId(0)]);
        let cache = Cached::new(FakePathfinder::new(calls), drives(), drives());
        let selector = NearestCellSelector::new(&cache, &map);

        let err = selector
            .select_nearest_to(
                [TestCandidate::new(5, CellId(5))],
                CellId(0),
                |_| Orientation::East,
                true,
                |_| true,
                default_score_opt,
            )
            .unwrap_err();

        assert!(matches!(err, PathSelectionError::MissingCellPosition { cell: CellId(5) }));
    }

    /// The blocker this change lifts. A free-form cell -- world position, no
    /// grid position -- is what a non-grid map is made of, and prescreening it
    /// used to be a hard error before the estimate module measured in
    /// millimetres.
    #[test]
    fn a_candidate_with_only_a_world_position_is_rankable() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut builder = WarehouseGraphBuilder::new();
        for (id, world) in [(0u32, (0.0, 0.0)), (1, (900.0, 0.0)), (2, (5000.0, 0.0))] {
            builder.add_cell(Cell {
                id: CellId(id),
                kind: CellKind::Open,
                zone: ZoneId(0),
                sector: None,
                // No grid position anywhere: this is the free-form case.
                grid_pos: None,
                world_pos: Some(WorldPos { x_mm: world.0, y_mm: world.1 }),
                barcode: None,
                rack_orientation: None,
                station_id: None,
                htm_rotation_forbidden: false,
            });
        }
        let map = WarehouseMapBuilder::new(SystemType::Relay).graph(builder.build()).build();
        let cache = Cached::new(
            FakePathfinder::new(calls).with_travel(CellId(1), CellId(0), 10).with_travel(CellId(2), CellId(0), 20),
            drives(),
            drives(),
        );
        let selector = NearestCellSelector::new(&cache, &map);

        let selected = selector
            .select_nearest_to(
                [TestCandidate::new(2, CellId(2)), TestCandidate::new(1, CellId(1))],
                CellId(0),
                |_| Orientation::East,
                true,
                |_| true,
                default_score_opt,
            )
            .expect("free-form cells prescreen by world position");

        assert_eq!(selected.candidate.id, 1, "cell 1 is 900mm out, cell 2 is 5000mm");
    }
}
