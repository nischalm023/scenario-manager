//! Within `site-template`, a `count` is how many of something the engine lays out or tallies: a placement's
//! instances along an axis or in all, a count knob's instances, the made entities holding one component; an
//! instance index is which one of a placement's instances along an axis. Lengths, coordinates and headings are the
//! protocol's own (`site_model::units`). Each is a bare integer only where JSON is read or written.

use std::fmt::{self, Display, Formatter};
use std::ops::{AddAssign, Mul};

use serde::Serialize;
use site_model::units::{Coord, Steps};

/// How many of something, zero included.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize)]
#[serde(transparent)]
pub struct Count(u64);

impl Count {
    pub const ONE: Count = Count(1);

    pub const fn new(count: u64) -> Self {
        Self(count)
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    /// A count as JSON writes it, `None` below zero.
    pub fn from_json(count: i64) -> Option<Count> {
        u64::try_from(count).ok().map(Count)
    }

    /// Every instance index below this count, in order.
    pub fn indexes(self) -> impl Iterator<Item = InstanceIndex> {
        (0..self.0).map(InstanceIndex)
    }
}

impl AddAssign for Count {
    fn add_assign(&mut self, more: Count) {
        self.0 += more.0;
    }
}

/// A grid's instances: its count along x times its count along y.
impl Mul for Count {
    type Output = Count;

    fn mul(self, other: Count) -> Count {
        Count(self.0 * other.0)
    }
}

impl Display for Count {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        write!(out, "{}", self.0)
    }
}

/// Which of a placement's instances along one axis: zero up to that axis's count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InstanceIndex(u64);

impl InstanceIndex {
    pub const fn new(index: u64) -> Self {
        Self(index)
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    /// Where this instance stands along its axis: `at`, then this many `pitch`es (spec 05 section 3.2). An index
    /// stands below a count read from JSON's signed integers, so it is one of them.
    pub fn marched(self, at: Coord, pitch: Steps) -> Coord {
        Coord(at.0 + self.0 as i64 * pitch.0)
    }
}

impl Display for InstanceIndex {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        write!(out, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A count is zero or more and its indexes run below it; an instance stands whole pitches from its placement's
    /// position; in JSON a count is a bare number.
    #[test]
    fn counts_index_and_instances_march() {
        assert_eq!(Count::from_json(-1), None);
        let three = Count::from_json(3).unwrap();
        assert_eq!(three.indexes().map(InstanceIndex::get).collect::<Vec<_>>(), [0, 1, 2]);
        assert_eq!(three * Count::new(2), Count::new(6));
        assert_eq!(InstanceIndex::new(2).marched(Coord(6800), Steps(7650)), Coord(22100));
        assert_eq!(serde_json::to_value(three).unwrap(), json!(3));
    }
}
