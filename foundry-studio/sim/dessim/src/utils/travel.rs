//! Candidate selection helpers for "cheap prescreen, exact refinement".
//!
//! This module deliberately knows nothing about `FHSippPathfinder`, `Cached`,
//! relays, parking, or robots. Callers supply both the cheap lower-bound score
//! and the exact refinement closure. That keeps pathfinder/cache dependencies
//! at the workflow boundary while sharing the fallback algorithm.

/// Select the minimum exact-score candidate after a cheap lower-bound
/// prescreen.
///
/// Algorithm:
/// 1. Sort candidates by caller-provided lower-bound score.
/// 2. Refine the first `top_k` candidates with the caller's exact scorer.
/// 3. Return the refined candidate with the minimum exact score.
/// 4. If the promoted set produces no valid candidate, refine the remainder.
///
/// The fallback preserves correctness for disconnected or temporarily invalid
/// local clusters while bounding the common case to `top_k` exact refinements.
pub fn select_min_prescreened<C, Lower, LowerScore, Refine, Refined, ExactScore, Error>(
    mut candidates: Vec<C>,
    top_k: usize,
    mut lower_bound: Lower,
    mut refine: Refine,
    mut exact_score: impl FnMut(&Refined) -> ExactScore,
) -> Result<Option<Refined>, Error>
where
    C: Clone,
    Lower: FnMut(&C) -> LowerScore,
    LowerScore: Ord,
    Refine: FnMut(C) -> Result<Option<Refined>, Error>,
    ExactScore: Ord,
{
    if candidates.is_empty() {
        return Ok(None);
    }

    candidates.sort_by_key(|candidate| lower_bound(candidate));

    let split = candidates.len().min(top_k.max(1));
    let top = best_refined(&candidates[..split], &mut refine, &mut exact_score)?;
    if top.is_some() || split == candidates.len() {
        return Ok(top);
    }
    best_refined(&candidates[split..], &mut refine, &mut exact_score)
}

fn best_refined<C, Refine, Refined, ExactScore, Error>(
    candidates: &[C],
    refine: &mut Refine,
    exact_score: &mut impl FnMut(&Refined) -> ExactScore,
) -> Result<Option<Refined>, Error>
where
    C: Clone,
    Refine: FnMut(C) -> Result<Option<Refined>, Error>,
    ExactScore: Ord,
{
    let mut best: Option<Refined> = None;
    for candidate in candidates.iter().cloned() {
        let Some(refined) = refine(candidate)? else {
            continue;
        };
        if best.as_ref().map_or(true, |current| exact_score(&refined) < exact_score(current)) {
            best = Some(refined);
        }
    }
    Ok(best)
}
