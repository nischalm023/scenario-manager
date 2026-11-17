//! Within `relay`, a pickup unit is the inbound unit a loaded HTM carries
//! from its relay pickup until the window onto its journey's target is
//! committed (scheduler time): the zone's inbound headroom counts it while it
//! rides the robot's open-ended hold. `Navigation` holds the ledger, so every
//! journey's windows move a unit onto the hold each window ends in, whatever
//! task made the journey; the relay records, counts and prunes the units.

use std::sync::Arc;

use desgraph::ZoneId;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

use crate::model::RobotId;
use crate::model::reservation::cell_reservation::CellReservation;

/// One unit per loaded robot, with the zone it counts in and the robot's hold
/// it rides: an HTM carries one tote, so its pickup replaces whatever unit a
/// zone has not yet pruned, and several robots' units from one relay can be
/// outstanding at once.
#[derive(Clone, Default)]
pub struct PickupUnits(Arc<Mutex<FxHashMap<RobotId, (ZoneId, CellReservation)>>>);

impl PickupUnits {
    pub(super) fn record(&self, zone: ZoneId, hold: CellReservation) {
        self.0.lock().insert(hold.robot(), (zone, hold));
    }

    pub(super) fn count(&self, zone: ZoneId, is_open: impl Fn(&CellReservation) -> bool) -> u32 {
        self.0.lock().values().filter(|(unit_zone, hold)| *unit_zone == zone && is_open(hold)).count() as u32
    }

    pub(super) fn prune(&self, zone: ZoneId, is_open: impl Fn(&CellReservation) -> bool) {
        self.0.lock().retain(|_, (unit_zone, hold)| *unit_zone != zone || is_open(hold));
    }

    /// A window closed `from`, the robot's open-ended hold, and ended in the
    /// open-ended hold `to`: a unit riding `from` rides `to`, so the zone
    /// counts it until the window onto the journey's target is committed, as
    /// when the journey was one window. A hold no unit rides changes nothing.
    pub fn follow(&self, from: &CellReservation, to: &CellReservation) {
        if let Some((_, unit)) = self.0.lock().get_mut(&from.robot())
            && unit.id() == from.id()
        {
            *unit = to.clone();
        }
    }
}
