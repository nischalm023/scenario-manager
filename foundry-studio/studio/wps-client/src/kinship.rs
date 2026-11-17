//! Within `wps-client`, `kinship` is ruling D76's test (spec 04 section 3.7): a referrer and its target reference
//! each other at head when their scopes share a nearest boundary, the nearest ancestor-or-self carrying the flag; a
//! scope with no flagged ancestor shares one with nothing. Every path prefix counts as a scope.

use rustc_hash::FxHashMap;
use studio_protocol::{Kinship, ScopePath, Seq};

use crate::value::ScopeListing;

/// The prefixes of `scope`, longest first: `a/b/c`, `a/b`, `a`.
pub fn ancestry(scope: &ScopePath) -> Vec<ScopePath> {
    let slugs: Vec<&str> = scope.slugs().collect();
    (1..=slugs.len()).rev().filter_map(|depth| slugs[..depth].join("/").parse().ok()).collect()
}

/// The nearest ancestor-or-self of `scope` that `is_boundary` flags.
pub fn nearest_boundary(scope: &ScopePath, is_boundary: impl Fn(&ScopePath) -> bool) -> Option<ScopePath> {
    ancestry(scope).into_iter().find(|prefix| is_boundary(prefix))
}

/// Kin when both scopes are one, or share a nearest boundary; lateral otherwise.
pub fn classify(referrer: &ScopePath, target: &ScopePath, is_boundary: impl Fn(&ScopePath) -> bool) -> Kinship {
    if referrer == target {
        return Kinship::Kin;
    }
    match (nearest_boundary(referrer, &is_boundary), nearest_boundary(target, &is_boundary)) {
        (Some(mine), Some(theirs)) if mine == theirs => Kinship::Kin,
        _ => Kinship::Lateral,
    }
}

/// The scope tree as one listing read it: each scope's flag and head.
#[derive(Debug, Clone, Default)]
pub struct ScopeTree {
    scopes: FxHashMap<ScopePath, ScopeListing>,
}

impl ScopeTree {
    pub fn new(listing: impl IntoIterator<Item = ScopeListing>) -> Self {
        Self { scopes: listing.into_iter().map(|scope| (scope.path.clone(), scope)).collect() }
    }

    pub fn get(&self, scope: &ScopePath) -> Option<&ScopeListing> {
        self.scopes.get(scope)
    }

    pub fn head(&self, scope: &ScopePath) -> Option<Seq> {
        self.scopes.get(scope).map(|listing| listing.head)
    }

    pub fn is_boundary(&self, scope: &ScopePath) -> bool {
        self.scopes.get(scope).is_some_and(|listing| listing.boundary)
    }

    pub fn kinship(&self, referrer: &ScopePath, target: &ScopePath) -> Kinship {
        classify(referrer, target, |scope| self.is_boundary(scope))
    }

    /// Every scope under the boundary `root`, down to the next boundary: `root`'s unit, in path order.
    pub fn unit(&self, root: &ScopePath) -> Vec<&ScopeListing> {
        let mut unit: Vec<&ScopeListing> = self.scopes.values().filter(|listing| &listing.unit == root).collect();
        unit.sort_by(|a, b| a.path.cmp(&b.path));
        unit
    }

    /// The root of the unit `scope` stands in.
    pub fn unit_of(&self, scope: &ScopePath) -> Option<&ScopePath> {
        self.scopes.get(scope).map(|listing| &listing.unit)
    }

    /// The nearest prefix of `scope` that exists, or else the first scope of the tree in path order: a scope to ask
    /// WPS from on behalf of one not created yet.
    pub fn stand_in(&self, scope: &ScopePath) -> Option<ScopePath> {
        ancestry(scope)
            .into_iter()
            .find(|prefix| self.scopes.contains_key(prefix))
            .or_else(|| self.scopes.keys().min().cloned())
    }
}
