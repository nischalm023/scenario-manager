//! Within the Map Creator adapter, `ordering` is **the key that says which record of
//! a Map Creator file comes before which**: one whole number per record, bigger
//! meaning later, chosen so that re-importing an edited dump moves as few of them
//! as it can.
//!
//! It exists because of what a re-import is on the other side. An ingest answers
//! with a patch for every entity whose payload changed, and a patched entity is
//! an entity written, audited and re-validated. A file's record order has to
//! ride on the entities — a lens list comes back in
//! identity order and the identities are random — and carrying it as the record's
//! *position* made every record after an edit a changed record: deleting one cell
//! of site-c and re-importing rewrote 1,766 otherwise-identical cells, whose
//! only difference was that they had moved up one place in a file.
//!
//! So the key is an ordering key and not a position. A record the site already
//! has keeps the key it had, whenever keeping it preserves the order the dump
//! states; a record the site does not have takes a value strictly between its
//! neighbours'. A first import leaves wide gaps between consecutive records so
//! that there is room between any two of them for a record somebody inserts
//! later, and a run with no room left is renumbered — that run alone, not the
//! file.
//!
//! What the key never is, is meaningful in itself. Nothing reads it but a sort:
//! the export sorts each file's records by it and writes them out in that order,
//! so the first round-trip law cannot tell one set of keys from another as long
//! as the order is the same.
//!
//! Every key is inside `[0, MAX]` and every sum here is worked out in `i128`, so
//! no input produces one that is not. That is a correctness rule and not a
//! tidiness one: the arithmetic used to be unchecked `i64`, and a site whose
//! largest key was `i64::MAX` — which nothing stopped a producer writing — made
//! the next append overflow.

/// The distance a first import leaves between two consecutive records.
///
/// A million, so that a million records can be inserted between any two of them
/// before the run has to be renumbered, and a site's largest file — site-b's 21,737
/// storage positions — still ends well inside [`MAX`].
pub const STEP: i64 = 1 << 20;

/// The largest key a record may take: 2^53 − 1.
///
/// Not `i64::MAX`. A key travels as a JSON number and is read back by consumers
/// whose numbers are IEEE doubles, where every integer up to 2^53 is exact and
/// `2^53 + 1` is not — so a key above this can come back as a different key, and
/// two records can come back with one key between them. The schema
/// `vocabulary()` publishes for `record_index` states this as its `maximum`, so a
/// document carrying a larger one is refused at the server's door rather than
/// silently re-read as its neighbour.
pub const MAX: i64 = 9_007_199_254_740_991;

/// The ordering keys one file's records take, given the key each record's entity
/// already carried — `None` for a record the site does not have.
///
/// The records are in the order the dump states them, and so is the answer: the
/// keys strictly increase, the first is at least 0 and the last is at most
/// [`MAX`].
///
/// A key the site states outside `[0, MAX]` is read as no key at all, and the
/// record takes a fresh one: a key a consumer cannot read back exactly is not a
/// key this adapter will keep, and refusing the whole import over
/// one would make a site unimportable for a number nothing here wrote.
pub fn stable_indexes(prior: &[Option<i64>]) -> Vec<i64> {
    let mut kept = keepable(prior);
    loop {
        match assign(prior, &kept) {
            Ok(keys) => return keys,
            // The records after the last kept key have nowhere left to go below
            // the ceiling, so that key cannot be kept either: it joins the run
            // and the run is renumbered together. Giving up the last one and no
            // more is what keeps the rebase local — every key before it is
            // untouched.
            Err(crowded) => kept[crowded] = false,
        }
    }
}

/// The keys, given which records keep the one they have — or the position whose
/// kept key leaves the records after it no room below [`MAX`].
fn assign(prior: &[Option<i64>], kept: &[bool]) -> Result<Vec<i64>, usize> {
    let mut out = vec![0i64; prior.len()];
    // The last key handed out, which is the exclusive lower bound for the next,
    // and the position it came from. `None` before the first, so that a record
    // whose existing key is 0 can keep it.
    let mut low: Option<(usize, i64)> = None;
    let mut pending: Vec<usize> = Vec::new();
    let mut at = 0usize;
    while at < prior.len() {
        if !kept[at] {
            pending.push(at);
            at += 1;
            continue;
        }
        let hi = i128::from(prior[at].expect("a position is only kept when it states a prior key"));
        let floor = low.map_or(-1i128, |(_, key)| i128::from(key));
        let span = hi - floor;
        if span - 1 >= pending.len() as i128 {
            // Room between the last key and this one for everything waiting.
            let gap = span / (pending.len() as i128 + 1);
            for (j, position) in pending.drain(..).enumerate() {
                out[position] = fits(floor + gap * (j as i128 + 1));
            }
            out[at] = fits(hi);
            low = Some((at, out[at]));
            at += 1;
        } else {
            // No room. This record's own key cannot be kept either, so the run
            // grows by one and is renumbered together; the loop re-enters at the
            // same position, which is now a pending one.
            return Err(at);
        }
    }
    if pending.is_empty() {
        return Ok(out);
    }
    // The tail: the records after the last kept key, gapped by a full step where
    // the ceiling allows it and by the largest step that fits where it does not.
    // The spread is what a local rebase of the tail is: the keys before the last
    // kept one do not move.
    let anchor = low.map_or(0i128, |(_, key)| i128::from(key));
    let spans = if low.is_some() { pending.len() } else { pending.len() - 1 } as i128;
    let room = i128::from(MAX) - anchor;
    if spans > room {
        // Not even one integer per record above the last kept key. There is one
        // when nothing is kept at all — the tail then starts at 0 and a `Vec`
        // holding more than 2^53 records is 144 petabytes of `Option<i64>` — so
        // this is only ever reached with a key to give up.
        return Err(low.map_or(0, |(position, _)| position));
    }
    let step = if spans == 0 { i128::from(STEP) } else { i128::from(STEP).min(room / spans) };
    let start = low.map_or(0i128, |(_, key)| i128::from(key) + step);
    for (j, position) in pending.drain(..).enumerate() {
        out[position] = fits(start + step * j as i128);
    }
    Ok(out)
}

/// One computed key as an `i64`.
///
/// Every value [`assign`] computes is inside `[0, MAX]` by its own two bounds: a
/// key placed between neighbours sits strictly between a floor of at least −1 and
/// a kept key of at most [`MAX`], and a tail key comes from a step the ceiling
/// check already fitted. So this states that invariant rather than repairing it —
/// clamping a key that had escaped it would answer with two records under one
/// key, which is the one thing an ordering key may not do.
fn fits(key: i128) -> i64 {
    assert!((0..=i128::from(MAX)).contains(&key), "the ordering key {key} is outside [0, {MAX}]");
    key as i64
}

/// Which records can keep the key they have: the longest run, in dump order,
/// whose existing keys are already strictly increasing.
///
/// The longest rather than the first greedy one, because the rule is that a
/// record keeps its key *whenever that preserves the dump's order* — and a single
/// record that has moved backwards should cost one renumbering, not every
/// renumbering after it. This is the classic patience solitaire: `tails[n]` is
/// the position ending the smallest increasing run of length `n + 1` found so
/// far, and `parent` remembers what each position extends.
fn keepable(prior: &[Option<i64>]) -> Vec<bool> {
    let stated = |position: usize| -> Option<i64> { prior[position].filter(|key| (0..=MAX).contains(key)) };
    let mut tails: Vec<usize> = Vec::new();
    let mut parent: Vec<Option<usize>> = vec![None; prior.len()];
    for position in 0..prior.len() {
        let Some(key) = stated(position) else { continue };
        let at = tails.partition_point(|tail| stated(*tail).expect("a tail states a key") < key);
        parent[position] = at.checked_sub(1).map(|before| tails[before]);
        match tails.get_mut(at) {
            Some(tail) => *tail = position,
            None => tails.push(position),
        }
    }
    let mut kept = vec![false; prior.len()];
    let mut walk = tails.last().copied();
    while let Some(position) = walk {
        kept[position] = true;
        walk = parent[position];
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::{stable_indexes, MAX, STEP};

    /// A first import: nothing to keep, so the keys are gapped from zero.
    #[test]
    fn a_first_import_leaves_a_gap_between_every_pair() {
        assert_eq!(stable_indexes(&[None, None, None]), vec![0, STEP, 2 * STEP]);
        assert!(stable_indexes(&[]).is_empty());
    }

    /// A dump nothing changed: every record keeps its key, which is the whole
    /// point — a re-import of an unchanged site writes nothing.
    #[test]
    fn an_unchanged_file_keeps_every_key() {
        let prior = vec![Some(0), Some(STEP), Some(2 * STEP)];
        assert_eq!(stable_indexes(&prior), vec![0, STEP, 2 * STEP]);
    }

    /// One record deleted: the survivors keep their keys and nothing else moves.
    #[test]
    fn a_deletion_moves_no_surviving_key() {
        let prior = vec![Some(0), Some(2 * STEP), Some(3 * STEP)];
        assert_eq!(stable_indexes(&prior), vec![0, 2 * STEP, 3 * STEP]);
    }

    /// One record inserted between two others: it takes a key between theirs and
    /// every other record keeps its own.
    #[test]
    fn an_insertion_takes_a_key_between_its_neighbours() {
        let prior = vec![Some(0), None, Some(STEP), Some(2 * STEP)];
        let keys = stable_indexes(&prior);
        assert_eq!(keys[0], 0);
        assert_eq!(keys[2], STEP);
        assert_eq!(keys[3], 2 * STEP);
        assert!(keys[1] > 0 && keys[1] < STEP, "the new key is {} and should sit between 0 and {STEP}", keys[1]);
    }

    /// A record appended at the end takes the next gap, and one prepended takes a
    /// key below the first — which it cannot, since keys start at zero, so the
    /// first record's key is the one that moves.
    #[test]
    fn an_append_extends_the_run_and_a_prepend_renumbers_the_head() {
        let appended = stable_indexes(&[Some(0), Some(STEP), None]);
        assert_eq!(appended, vec![0, STEP, 2 * STEP]);
        let prepended = stable_indexes(&[None, Some(0), Some(STEP)]);
        assert!(prepended[0] < prepended[1], "{prepended:?}");
        assert!(prepended.windows(2).all(|pair| pair[0] < pair[1]), "{prepended:?}");
    }

    /// Two records inserted where only one integer is free: the run including its
    /// anchors is renumbered, and nothing outside the run is touched.
    #[test]
    fn a_run_with_no_room_left_is_renumbered_and_its_neighbours_are_not() {
        // Keys 10 and 12 have one free integer between them and two records want
        // it.
        let prior = vec![Some(0), Some(10), None, None, Some(12), Some(3 * STEP)];
        let keys = stable_indexes(&prior);
        assert_eq!(keys[0], 0, "the record before the crowded run keeps its key");
        assert_eq!(keys[5], 3 * STEP, "the record after it keeps its key");
        assert!(keys.windows(2).all(|pair| pair[0] < pair[1]), "{keys:?}");
    }

    /// A record that has moved backwards in the file costs one renumbering, not
    /// every renumbering after it: the longest increasing run is what is kept.
    #[test]
    fn one_record_moved_backwards_renumbers_only_itself() {
        // The fourth record used to be first. Keeping it would force the three
        // before it to be renumbered; keeping them costs one.
        let prior = vec![Some(STEP), Some(2 * STEP), Some(3 * STEP), Some(0)];
        let keys = stable_indexes(&prior);
        assert_eq!(keys[0..3], [STEP, 2 * STEP, 3 * STEP], "the run that was already in order keeps its keys");
        assert!(keys[3] > keys[2], "{keys:?}");
    }

    /// A file whose last record already sits at the ceiling: nothing changed, so
    /// every key is kept, the largest included.
    #[test]
    fn a_file_at_the_ceiling_keeps_every_key() {
        let prior = vec![Some(0), Some(STEP), Some(MAX)];
        assert_eq!(stable_indexes(&prior), vec![0, STEP, MAX]);
    }

    /// A record appended after a key at the ceiling: there is no key above the
    /// ceiling, so the record at it gives its key up and the two are renumbered
    /// together — the local rebase, rather than the overflow the unchecked
    /// arithmetic used to produce.
    #[test]
    fn an_append_past_the_ceiling_rebases_the_tail() {
        let keys = stable_indexes(&[Some(0), Some(STEP), Some(MAX), None]);
        assert_eq!(keys[0..2], [0, STEP], "the keys before the crowded tail do not move");
        assert!(keys.windows(2).all(|pair| pair[0] < pair[1]), "{keys:?}");
        assert!(keys.iter().all(|key| (0..=MAX).contains(key)), "{keys:?}");
        assert!(keys[2] < MAX, "the key at the ceiling had to move to make room above it: {keys:?}");
    }

    /// A run crowded against the ceiling: three records want the two integers
    /// above the last kept key, so the kept key goes too and the four are spread
    /// over what is left.
    #[test]
    fn a_crowded_run_at_the_ceiling_is_spread_over_what_is_left() {
        let keys = stable_indexes(&[Some(0), Some(MAX - 2), None, None, None]);
        assert_eq!(keys[0], 0, "the record before the crowded run keeps its key");
        assert!(keys.windows(2).all(|pair| pair[0] < pair[1]), "{keys:?}");
        assert!(keys.iter().all(|key| (0..=MAX).contains(key)), "{keys:?}");
    }

    /// A key the site states outside the range is no key at all: the
    /// record takes a fresh one rather than the import refusing the site or the
    /// arithmetic running past `i64`.
    ///
    /// `i64::MAX` is the reproduction — a site whose largest record index is
    /// that, one record appended — and a negative key is the other side of the
    /// same rule.
    #[test]
    fn a_key_outside_the_range_is_read_as_no_key() {
        for unreadable in [i64::MAX, MAX + 1, -1, i64::MIN] {
            let keys = stable_indexes(&[Some(0), Some(unreadable), None]);
            assert!(keys.windows(2).all(|pair| pair[0] < pair[1]), "{unreadable}: {keys:?}");
            assert!(keys.iter().all(|key| (0..=MAX).contains(key)), "{unreadable}: {keys:?}");
            assert_eq!(keys[0], 0, "{unreadable}: the readable key is kept");
        }
    }

    /// The answer is always strictly increasing, never below zero and never above
    /// the ceiling, whatever it was handed — which is what the export's sort, the
    /// schema's `minimum` and its `maximum` all rely on.
    #[test]
    fn the_keys_always_increase_and_stay_inside_the_range() {
        let awkward: Vec<Vec<Option<i64>>> = vec![
            vec![Some(5), Some(5), Some(5)],
            vec![Some(-3), None, Some(1)],
            vec![Some(2), Some(1), Some(0)],
            vec![None, Some(0), None, Some(1), None],
            vec![Some(0), Some(1), Some(2), None],
            vec![Some(MAX)],
            vec![Some(MAX), None],
            vec![None, Some(MAX)],
            vec![Some(MAX), Some(MAX), None, None],
            vec![Some(i64::MAX)],
            vec![Some(i64::MAX), None, Some(i64::MIN)],
            vec![Some(MAX - 1), Some(MAX), None, None, None],
        ];
        for prior in awkward {
            let keys = stable_indexes(&prior);
            assert!(keys.iter().all(|key| (0..=MAX).contains(key)), "{prior:?} -> {keys:?}");
            assert!(keys.windows(2).all(|pair| pair[0] < pair[1]), "{prior:?} -> {keys:?}");
        }
    }
}
