//! Open-set priority queue entry for the SIPP-A\* search.
//!
//! `BinaryHeap` is a max-heap, so [`AStarEntry`]'s `Ord` is reversed: the
//! smallest `f` pops first. Tie-breaking is deterministic for offline
//! simulation reproducibility — equal `f` breaks on lowest `g`, then
//! lexicographic [`SearchKey`]. See spec §4 + FHSippPathfinder draft
//! §Goal Acceptance.

use std::cmp::Ordering;

use super::state::SearchKey;
use crate::SimDuration;

/// Open-set entry for the SIPP-A\* search. `idx` indexes [`super::search::Search::arena`];
/// the full [`SearchKey`] is recovered from the arena on pop. Keeping the
/// heap-resident entry at 12 bytes (vs 72 with an inline `SearchKey`) shrinks
/// the per-sift copies and avoids duplicating the key on every re-insertion.
///
/// `f`/`g` are bare `u32` milliseconds, a deliberate departure from
/// coding-standards §11: both are bounded by the planning horizon
/// (`max_wait`), far below the `u32`-ms ceiling (~49.7 days). They are an
/// ordering/staleness shadow only — the canonical `SimDuration` `g` lives in
/// the arena and feeds all downstream arithmetic.
pub(super) struct AStarEntry {
    pub f: u32,
    pub g: u32,
    pub idx: u32,
}

impl PartialEq for AStarEntry {
    fn eq(&self, other: &Self) -> bool {
        self.f == other.f && self.g == other.g && self.idx == other.idx
    }
}

impl Eq for AStarEntry {}

impl PartialOrd for AStarEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for AStarEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        // BinaryHeap is a max-heap; reverse the comparison so smallest-f
        // wins. Equal f → smallest g; equal g → arena insertion order
        // (`idx`) — a deterministic tie-break, no longer lexicographic
        // SearchKey.
        (other.f, other.g, other.idx).cmp(&(self.f, self.g, self.idx))
    }
}

// ---------------------------------------------------------------------------
// Bucket queue for spatial-only A*
// ---------------------------------------------------------------------------

/// Circular bucket queue for the spatial-only `find_path` open list.
///
/// A monotone cursor walks buckets indexed by `f mod capacity`, so the
/// structure only orders correctly while every live entry's f sits inside one
/// `capacity`-wide window starting at the cursor. That holds iff no single
/// action can raise f by `capacity` or more: children are pushed from a parent
/// popped at the cursor, so `max live f <= cursor + max_action_jump`.
///
/// The window is therefore a property of the *map and the drive*, not a
/// constant — see [`Self::capacity_for`], which `find_path` calls with each
/// search's drive. Sizing it as a fixed 2048 was a live correctness bug: the
/// cencora_v3 HTM in-place rotate is 2100ms, and a 2100ms jump into a
/// 2048-bucket ring folds onto a bucket the cursor has already passed. Measured
/// effect on that fixture: `find_path` returned paths up to 3.8s worse than
/// `reserve_path` (which uses [`AStarEntry`] + `BinaryHeap` and was unaffected)
/// on ~4.7% of HTM queries, always pessimistic, never optimistic.
///
/// Capacity is never clamped. A pathological map degrades allocation cost
/// visibly rather than silently returning wrong paths.
///
/// Stale entries are filtered by the caller via the standard lazy-deletion
/// pattern (check g against `g_score` map after popping).
pub(super) struct BucketQueue {
    buckets: Vec<Vec<(SearchKey, SimDuration)>>,
    /// `buckets.len() - 1`. Valid as an AND-mask because the length is a power
    /// of two, enforced by [`Self::with_capacity`].
    mask: usize,
    /// Monotonically advancing cursor tracking the current minimum f (ms).
    cursor: u64,
    len: usize,
}

/// Floor on bucket count. Maps whose largest action jump fits well inside this
/// keep the historical layout, so their allocation cost and pinned-trace
/// tie-break order are unchanged.
const MIN_BUCKETS: usize = 2048;

/// Ceiling on bucket count: 2^20 buckets is ~25 MB of empty `Vec` headers, which
/// corresponds to a single action costing over 17 minutes. No real warehouse
/// reaches it, so hitting it means the map is malformed (a garbage
/// `distance_mm`, or a speed near zero). Clamping keeps planning alive instead of
/// letting `next_power_of_two` overflow, and the clamp is announced on stderr
/// because past the ceiling the queue can mis-order exactly the way this type
/// exists to prevent.
const MAX_BUCKETS: usize = 1 << 20;

impl BucketQueue {
    /// Bucket count for a search whose largest single-action f-increase is
    /// `max_action_jump_ms`.
    ///
    /// Must be a power of two **strictly greater** than the jump, hence the
    /// `+ 1` before rounding up: a jump of exactly `capacity` aliases onto the
    /// cursor's own bucket. Empirically confirmed on cencora_v3 by sweeping the
    /// HTM rotate time — 2040ms is correct at 2048 buckets, 2050ms is not.
    pub fn capacity_for(max_action_jump_ms: u64) -> usize {
        let needed = max_action_jump_ms.saturating_add(1).max(MIN_BUCKETS as u64);
        if needed > MAX_BUCKETS as u64 {
            eprintln!(
                "[pathfinder] BucketQueue: action jump {}ms needs more than the {} bucket ceiling; \
                 clamping. find_path may return suboptimal paths — check the map for a bad \
                 edge distance_mm or a near-zero speed.",
                max_action_jump_ms, MAX_BUCKETS,
            );
            return MAX_BUCKETS;
        }
        // Infallible below the ceiling: `needed` fits in usize on any 32-bit-or-wider target.
        (needed as usize).next_power_of_two()
    }

    pub fn with_capacity(buckets: usize) -> Self {
        debug_assert!(buckets.is_power_of_two() && buckets > 0, "bucket count must be a power of two, got {}", buckets);
        Self { buckets: vec![Vec::new(); buckets], mask: buckets - 1, cursor: 0, len: 0 }
    }

    /// Insert `key` with priority `f`. `g` is stored alongside for the
    /// caller's lazy-deletion check. The cursor SEEDS itself from a push into
    /// an empty queue and REWINDS when `f` lands below it — see the inline
    /// comments for why both exist.
    #[inline]
    pub fn push(&mut self, key: SearchKey, f: SimDuration, g: SimDuration) {
        let f = f.as_ms();
        if self.len == 0 {
            // The queue's only entry defines the minimum outright. Without
            // this, the cursor keeps its default 0 and the first pop leaves
            // it LAP-SHIFTED (true f mod ring): harmless when f is exactly
            // monotone (the shift is a constant offset and relative order
            // survives the mod), but fatal once the ε-rewind below needs to
            // compare absolute f against it.
            self.cursor = f;
        } else if f < self.cursor {
            // f is NOT monotone across pushes, from two separate causes, so
            // the cursor has to be able to walk back:
            //
            // 1. ε from integer flooring. A metric heuristic is truncated per
            //    node, so a child's f can land 1ms below its parent's — and
            //    the parent's f is where the cursor stands while it expands.
            //    Bounded at 1ms per push: with `edge_mm >= d_parent - d_child`
            //    (what `graph_is_rectilinear` enforces for the Manhattan arm)
            //    the algebra gives f_child - f_parent > -2.
            // 2. The seeding above, which is the larger effect and fires on
            //    LATTICE maps too. When a pop empties the queue the cursor is
            //    seeded from whatever the next push happens to be — often a
            //    rotate at parent + turn_duration — and the sibling move at
            //    the parent's own f then pulls it back down by up to one turn.
            //    Instrumenting this branch over `cargo test -p dessim --lib`
            //    counts 787 rewinds at depths 2100/1656/1000ms and none at
            //    1ms, including on pure-lattice fixtures whose cells carry
            //    only `grid_pos` (so `straight_line_h` never runs).
            //
            // Without the rewind an entry below the cursor sits a full ring
            // rotation away, and the search sequences the whole frontier
            // behind any detour cheaper than the ring width — the suboptimal
            // `find_path` results `find_path_optimality` caught on the
            // converted cencora_v3.
            //
            // Cost to the ring invariant, stated honestly: the live-f window
            // becomes `jump + cumulative drift` where the struct doc's
            // derivation assumes a monotone cursor and `capacity_for` only
            // guarantees `capacity > jump`. Measured slack on every fixture
            // in the tree is 1,048–1,996ms against realistic drift of
            // O(100–180ms) at O(200)-hop depth, and the tight-slack configs
            // are exactly the round ones (edge_ms exact, so no rewind at
            // all). Re-check this if a map ever lands with `jump` just under
            // `capacity`.
            self.cursor = f;
        }
        self.buckets[f as usize & self.mask].push((key, g));
        self.len += 1;
    }

    /// Remove and return the entry with the smallest f seen so far.
    /// Returns `None` when empty. The returned g may be stale — callers
    /// must check against `g_score`.
    #[inline]
    pub fn pop(&mut self) -> Option<(SearchKey, SimDuration)> {
        while self.len > 0 {
            let idx = self.cursor as usize & self.mask;
            if let Some(entry) = self.buckets[idx].pop() {
                self.len -= 1;
                return Some(entry);
            }
            self.cursor += 1;
        }
        None
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn clear(&mut self) {
        for b in &mut self.buckets {
            b.clear();
        }
        self.cursor = 0;
        self.len = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::super::state::{SippState, SpeedBucket};
    use super::*;
    use crate::Orientation;
    use crate::SimTime;
    use desgraph::types::CellId;

    #[test]
    fn capacity_is_strictly_greater_than_the_jump() {
        // Floor: anything that fits inside the historical ring keeps it.
        assert_eq!(BucketQueue::capacity_for(0), 2048);
        assert_eq!(BucketQueue::capacity_for(1000), 2048);
        // 2047 fits (2048 > 2047); 2048 does not, because a jump equal to the
        // capacity aliases onto the cursor's own bucket.
        assert_eq!(BucketQueue::capacity_for(2047), 2048);
        assert_eq!(BucketQueue::capacity_for(2048), 4096);
        // The empirically bisected boundary on cencora_v3.
        assert_eq!(BucketQueue::capacity_for(2040), 2048);
        assert_eq!(BucketQueue::capacity_for(2050), 4096);
        // The shipped cencora_v3 HTM rotate.
        assert_eq!(BucketQueue::capacity_for(2100), 4096);
        for jump in [0u64, 1, 999, 2040, 2050, 2100, 4095, 4096, 100_000] {
            let cap = BucketQueue::capacity_for(jump);
            assert!(cap.is_power_of_two(), "capacity {} not a power of two", cap);
            assert!(cap as u64 > jump, "capacity {} must exceed jump {}", cap, jump);
        }
    }

    #[test]
    fn absurd_jump_clamps_instead_of_overflowing() {
        // `next_power_of_two` would overflow near u64::MAX. A malformed map (garbage
        // `distance_mm`, near-zero speed) must degrade loudly, not wrap the capacity
        // to zero and underflow the mask.
        for jump in [MAX_BUCKETS as u64, u64::MAX / 2, u64::MAX - 1, u64::MAX] {
            let cap = BucketQueue::capacity_for(jump);
            assert_eq!(cap, MAX_BUCKETS, "jump {} should clamp to the ceiling", jump);
            assert!(cap.is_power_of_two());
        }
        // One below the ceiling still sizes exactly, so the clamp is not off by one.
        assert_eq!(BucketQueue::capacity_for(MAX_BUCKETS as u64 - 2), MAX_BUCKETS);
    }

    #[test]
    fn pops_in_f_order_across_a_jump_that_would_have_wrapped() {
        // f values 5640 and 7740 differ by 2100 — the cencora_v3 rotate jump.
        // At 2048 buckets they land on 1544 and 1596, so the larger f pops
        // first (the bug). Sized for the jump, order is preserved.
        let mut q = BucketQueue::with_capacity(BucketQueue::capacity_for(2100));
        let lo = SearchKey::ordinary(SippState {
            cell: CellId(1),
            orientation: Orientation::East,
            interval: (SimTime::ZERO, None),
            speed_bucket: SpeedBucket::ZERO,
        });
        let hi = SearchKey::ordinary(SippState {
            cell: CellId(2),
            orientation: Orientation::East,
            interval: (SimTime::ZERO, None),
            speed_bucket: SpeedBucket::ZERO,
        });
        q.push(hi, SimDuration::ms(7740), SimDuration::ms(7740));
        q.push(lo, SimDuration::ms(5640), SimDuration::ms(5640));
        assert_eq!(q.pop().map(|(_, g)| g), Some(SimDuration::ms(5640)));
        assert_eq!(q.pop().map(|(_, g)| g), Some(SimDuration::ms(7740)));
        assert_eq!(q.len(), 0);
    }
}
