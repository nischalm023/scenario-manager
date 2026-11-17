//! RelayHtmIndex — per-relay HTM-to-relay travel-time approximation.
//!
//! Maintained as an [`AvailableHtmRegistry`] observer. Tracks whether each
//! relay's HTM access cell is currently held by a registered idle no-tote
//! HTM (a "resident"). Relay scoring (see
//! `select_relay_for_retrieve_locked_with_score`) reads `travel_time(relay)`
//! and adds it to the relay→station completion score: 0s for relays whose
//! access cell holds a resident, [`PICK_HTM_TO_RELAY_DEFAULT`] otherwise.
//!
//! # Why this exists
//!
//! Without this index, retrieve-relay scoring treats every empty access cell
//! the same as a cell with a resident no-tote HTM already in it — so an
//! HTM that just deposited a tote at relay R_A's cell and is sitting there
//! is no more valuable to a fresh from-rack chain than a fresh empty cell
//! at relay R_B in the same zone. The selector then picks R_B, and the
//! HTM at R_A drives away to R_B and pulls a tote from there. This is
//! exactly the "drop tote, drive to a different relay in the same zone,
//! pick up a new tote" pattern observed in desvis. The 0-vs-default split
//! makes the scoring see and prefer the resident case.
//!
//! # Cell-keyed, not relay-keyed
//!
//! The internal `cell_to_relays: CellId → Vec<RelayId>` map handles the
//! future "stacked relays" geometry where 2+ relay slots share one HTM
//! access cell — when a no-tote HTM occupies that cell, *all* relays
//! attached to it score 0s. Today's geometry is 1:1, so each Vec has
//! length 1, but the data shape is forward-compatible.
//!
//! # TODO(htm-to-relay-cost)
//!
//! `PICK_HTM_TO_RELAY_DEFAULT` is a single constant for "any non-resident
//! relay". A future improvement is to pre-materialize an A* travel time
//! per relay → its nearest no-tote HTM. The observer would then update the
//! per-relay travel time on every register/take instead of only flipping
//! between 0 and the constant. Out of scope for the current change; the
//! constant captures most of the lift with far less complexity.

use std::sync::{Arc, Mutex};

use desgraph::{CellId, RelayId};
use rustc_hash::FxHashMap;

use crate::model::htm::registry::{AvailableHtm, Observer};
use crate::model::{RobotId, SimDuration, SimTime};
use crate::pathfinding::Orientation;

/// Travel-time penalty applied to relays whose HTM access cell is *not*
/// currently held by an idle no-tote HTM. A coarse "average HTM travel from
/// elsewhere in the zone" estimate; resident relays get 0s instead.
pub const PICK_HTM_TO_RELAY_DEFAULT: SimDuration = SimDuration::seconds(8);

/// Approach summary for a relay: how long it takes an HTM to reach the
/// access cell and, if a resident no-tote HTM holds the cell, when that
/// HTM is action-ready. The relay scorer uses both.
#[derive(Clone, Copy, Debug)]
pub struct RelayHtmApproach {
    pub resident: Option<(RobotId, Orientation)>,
    /// HTM-to-relay travel-time estimate. 0s for resident relays;
    /// [`PICK_HTM_TO_RELAY_DEFAULT`] otherwise.
    pub htm_to_relay: SimDuration,
    /// `Some(action_ready_at)` for the resident no-tote HTM holding the
    /// access cell, else `None`. The scorer floors `path_start_at` at this
    /// time so a chain can't start before the resident HTM finishes its
    /// prior dwell, even if VTM deposit is earlier.
    pub resident_ready_at: Option<SimTime>,
}

impl RelayHtmApproach {
    fn nonresident() -> Self {
        Self { resident: None, htm_to_relay: PICK_HTM_TO_RELAY_DEFAULT, resident_ready_at: None }
    }

    fn resident(entry: &AvailableHtm) -> Self {
        Self {
            resident: Some((entry.robot(), entry.orientation)),
            htm_to_relay: SimDuration::ZERO,
            resident_ready_at: Some(entry.action_ready_at),
        }
    }
}

#[derive(Clone)]
pub struct RelayHtmIndex {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    /// HTM access cell → relays sharing that cell. Length 1 today, may be
    /// 2+ under future stacked-relay geometry.
    cell_to_relays: FxHashMap<CellId, Vec<RelayId>>,
    /// Per-relay current approach summary.
    approach: FxHashMap<RelayId, RelayHtmApproach>,
}

impl RelayHtmIndex {
    /// Build an empty index from `(relay, cell)` pairs. Each relay starts
    /// in the "non-resident" state; flips to "resident" once a no-tote HTM
    /// registers at the matching cell.
    pub fn new(relay_cells: impl IntoIterator<Item = (RelayId, CellId)>) -> Self {
        let mut cell_to_relays: FxHashMap<CellId, Vec<RelayId>> = FxHashMap::default();
        let mut approach: FxHashMap<RelayId, RelayHtmApproach> = FxHashMap::default();
        for (relay, cell) in relay_cells {
            cell_to_relays.entry(cell).or_default().push(relay);
            approach.insert(relay, RelayHtmApproach::nonresident());
        }
        Self { inner: Arc::new(Mutex::new(Inner { cell_to_relays, approach })) }
    }

    /// Read the approach summary for a relay. Returns the non-resident
    /// default for relays the index doesn't know about — a defensive
    /// fallback that should never fire in practice.
    pub fn approach(&self, relay: RelayId) -> RelayHtmApproach {
        self.inner.lock().unwrap().approach.get(&relay).copied().unwrap_or_else(RelayHtmApproach::nonresident)
    }

    fn set(&self, entry: &AvailableHtm, value: RelayHtmApproach) {
        if entry.tote.is_some() {
            return;
        }
        let mut inner = self.inner.lock().unwrap();
        let Some(relays) = inner.cell_to_relays.get(&entry.cell()).cloned() else {
            return;
        };
        for relay in relays {
            inner.approach.insert(relay, value);
        }
    }
}

impl Observer for RelayHtmIndex {
    fn on_register(&self, entry: &AvailableHtm) {
        // No-tote HTM at a relay access cell becomes that relay's resident.
        // Cell-reservation invariant guarantees no overlap with another
        // resident.
        self.set(entry, RelayHtmApproach::resident(entry));
    }

    fn on_take(&self, entry: &AvailableHtm) {
        // Resident leaves: approach flips back to non-resident default.
        self.set(entry, RelayHtmApproach::nonresident());
    }
}
