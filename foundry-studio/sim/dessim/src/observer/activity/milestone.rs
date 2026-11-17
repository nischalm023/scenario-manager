//! Within `activity`, a `milestone` is one boundary of a robot's work as the
//! stream reports it: a leg's departure, the arrival that opens a station
//! visit, or work whose extent its task knows. Milestones sort by their
//! chronology, never by the order the facts are read.

use desgraph::CellId;

use crate::events::TripEndpoint;
use crate::model::SimTime;

use super::ActivityKind;

/// What a leg's nav says of it: where in simtime it ends, and whether the bot
/// carries a tote along it.
#[derive(Debug, Clone, Copy)]
pub(in crate::observer) struct LegNav {
    pub(in crate::observer) arrival: SimTime,
    pub(in crate::observer) carrying: bool,
}

/// One reported boundary in a robot's work, with `known_from`: where B stood
/// when the fact reporting it was read.
///
/// A milestone may not lie behind its `known_from`, for B had passed it and
/// the slices it falls in were read without it. A leg's stand before the
/// operation the leg was sent for is the leg's only from that operation's
/// `known_from` on: B read what it passed before then as a gap.
#[derive(Debug, Clone, Copy)]
pub(in crate::observer) enum Milestone {
    /// A leg departed. It runs to the operation that follows it, or, with none
    /// following, to the end of its nav: a bot left standing where nothing
    /// waits for it is in a gap. `from` and `from_cell` are where the bot
    /// stood, read before the leg's nav moved the tracked position on, and
    /// `leaves_visit` marks the return leg, which closes a station visit.
    Departure {
        at: SimTime,
        nav: Option<LegNav>,
        leaves_visit: bool,
        from: TripEndpoint,
        from_cell: Option<CellId>,
        known_from: SimTime,
    },
    /// The bot arrived at its station's presentation cell, opening the visit.
    Arrival { at: SimTime, known_from: SimTime },
    /// Work whose extent the reporting task knows. `carries_after` is set only
    /// by the operations that change what the bot holds, and `resident_since`
    /// only by one its task found the bot already standing at: no leg of that
    /// task leads there, and the bot stood there in a gap from that instant,
    /// a stand the reporting fact journals itself.
    Span {
        kind: ActivityKind,
        start: SimTime,
        end: SimTime,
        carries_after: Option<bool>,
        resident_since: Option<SimTime>,
        known_from: SimTime,
    },
}

impl Milestone {
    pub(super) fn at(&self) -> SimTime {
        match self {
            Self::Departure { at, .. } | Self::Arrival { at, .. } => *at,
            Self::Span { start, .. } => *start,
        }
    }

    pub(super) fn known_from(&self) -> SimTime {
        match self {
            Self::Departure { known_from, .. } | Self::Arrival { known_from, .. } | Self::Span { known_from, .. } => {
                *known_from
            }
        }
    }

    /// Instant first; at a shared instant, an opener before whatever bounds
    /// it, so the pair collapses to an empty interval instead of one running
    /// past its bound. A departure on the return leg closes the visit an
    /// arrival opened; any other departure opens the leg the arrival ends.
    pub(super) fn order(&self) -> (SimTime, u8) {
        let rank = match self {
            Self::Departure { leaves_visit: false, .. } => 0,
            Self::Arrival { .. } => 1,
            Self::Departure { leaves_visit: true, .. } => 2,
            Self::Span { .. } => 3,
        };
        (self.at(), rank)
    }

    /// Where the stand of a leg that arrived at `arrival` becomes the leg's,
    /// if this is the operation the leg was sent for: at the arrival, or once
    /// the operation was reported if B had passed the arrival by then. `None`
    /// for what no leg is sent for, an operation its task found the bot
    /// already standing at included.
    pub(super) fn claims_stand_from(&self, arrival: SimTime) -> Option<SimTime> {
        match self {
            Self::Arrival { known_from, .. } | Self::Span { resident_since: None, known_from, .. } => {
                Some(arrival.max(*known_from))
            }
            _ => None,
        }
    }
}
