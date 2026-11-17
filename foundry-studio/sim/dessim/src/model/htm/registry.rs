//! Registry of available (idle, between-chain) HTMs.
//!
//! After a task chain completes (or at simulator init), an HTM ends at some
//! cell with an open-ended `CellReservation`. The registry holds one entry
//! per idle HTM: the reservation and a snapshot of the tote it's carrying
//! (if any, with contents at registration time).
//!
//! Tote state is discovered at registration time by querying
//! `InventoryService::tote_on_robot`. This snapshot is stable until the
//! next chain claims the HTM.
//!
//! This registry is the HTM pool resource family's service (task-model-v2,
//! issue #423). [`ProposedHtmPickup`] is its candidacy surface: a chosen
//! robot held revocably until acceptance calls [`AvailableHtmRegistry::take`].
//! [`ProposedHtmClaim`] is the candidacy for one known stay, a productive
//! claim or a last resort (parking, a station's clearance); its acceptance
//! spends through [`AvailableHtmRegistry::take_exact`], which takes nothing
//! once the registry no longer holds that stay. [`ProposedLastResort`] is
//! the last resort with a `Vacate` on the stay's hold, so that an entrant's
//! need-by brings its decision forward. The registry also counts what pickup
//! candidacies cost to re-select, because their premise is deliberately
//! coarse and re-selection is the price of that choice.

use std::collections::HashSet;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustc_hash::{FxHashMap, FxHashSet};

use desgraph::{CellId, WorldPos};
use desmap::WarehouseMap;

use crate::model::cell::registry::Vacate;
use crate::model::ids::{PresentationId, RobotId, ToteId};
use crate::model::inventory::InventoryService;
use crate::model::navigation::Navigation;
use crate::model::parking::ParkingRegistry;
use crate::model::reservation::cell_reservation::CellReservation;
use crate::model::{SimDuration, SimTime, SkuId};
use crate::pathfinding::Orientation;
use crate::scheduler::task::proposal::{AcceptError, ProposalId, ProposedReservation};
use crate::signals::{Mut, Version};

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("HTM {0:?} already registered in AvailableHtmRegistry")]
    AlreadyRegistered(RobotId),
    #[error("HTM pickup candidacy read for its entry before acceptance claimed one")]
    NotAccepted,
}

#[derive(Clone)]
pub struct PickupHeadroom {
    protected_empty: Arc<dyn Fn() -> Option<usize> + Send + Sync>,
    changed: Version,
}

impl PickupHeadroom {
    pub fn new(protected_empty: usize, guard_required: Arc<dyn Fn() -> bool + Send + Sync>, changed: Version) -> Self {
        Self::dynamic(Arc::new(move || guard_required().then_some(protected_empty)), changed)
    }

    pub fn dynamic(protected_empty: Arc<dyn Fn() -> Option<usize> + Send + Sync>, changed: Version) -> Self {
        Self { protected_empty, changed }
    }

    pub fn version(&self) -> &Version {
        &self.changed
    }

    pub fn allows(&self, registry: &AvailableHtmRegistry, at: SimTime) -> bool {
        self.protected_empty().is_none_or(|protected_empty| {
            registry
                .inner
                .lock()
                .unwrap()
                .values()
                .filter(|entry| entry.tote.is_none() && entry.action_ready_at <= at)
                .count()
                > protected_empty
        })
    }

    /// Earliest time the currently registered empty fleet can satisfy this
    /// guard. Callers fold this into the pickup's departure anchor; no
    /// registry mutation occurs when a future-ready entry reaches that time.
    pub fn earliest_allowed_at(&self, registry: &AvailableHtmRegistry, after: SimTime) -> Option<SimTime> {
        let Some(protected_empty) = self.protected_empty() else {
            return Some(after);
        };
        let mut ready: Vec<_> = registry
            .inner
            .lock()
            .unwrap()
            .values()
            .filter(|entry| entry.tote.is_none())
            .map(|entry| entry.action_ready_at)
            .collect();
        if ready.len() <= protected_empty {
            return None;
        }
        ready.select_nth_unstable(protected_empty);
        Some(after.max(ready[protected_empty]))
    }

    fn protected_empty(&self) -> Option<usize> {
        (self.protected_empty)()
    }
}

/// Snapshot of a tote carried by an idle HTM.
#[derive(Clone, Debug)]
pub struct CarriedTote {
    pub tote_id: ToteId,
    pub contents: Vec<(SkuId, u32)>,
    /// Trip ID of the presentation cycle that left this tote on the bot.
    /// `Some` when the HTM was registered post-presentation (the store-chain
    /// case — a clearance's return reads it to continue the same Trip ID);
    /// `None` when registered with no cycle context (sim-start seed, parking).
    pub presentation_id: Option<PresentationId>,
}

impl CarriedTote {
    pub fn is_empty(&self) -> bool {
        self.contents.is_empty()
    }
    pub fn contains_sku(&self, sku: SkuId) -> bool {
        self.contents.iter().any(|&(s, qty)| s == sku && qty > 0)
    }
}

/// Snapshot of an available HTM: where it is, what it's carrying.
///
/// `cell_reservation.window().start` is when the HTM's leading edge enters
/// the cell, not when it is action-ready there. `available_at` is the
/// caller-supplied action-ready timestamp at the cell — separate from the
/// reservation window's start.
#[derive(Clone, Debug)]
pub struct AvailableHtm {
    availability_id: HtmAvailabilityId,
    pub cell_reservation: CellReservation,
    pub action_ready_at: SimTime,
    pub orientation: Orientation,
    pub tote: Option<CarriedTote>,
    pub at_idle_relay: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HtmAvailabilityId {
    pub robot: RobotId,
    pub epoch: u64,
}

impl AvailableHtm {
    pub fn availability_id(&self) -> HtmAvailabilityId {
        self.availability_id
    }

    pub fn robot(&self) -> RobotId {
        self.cell_reservation.robot()
    }
    pub fn cell(&self) -> CellId {
        self.cell_reservation.cell()
    }
    pub fn available_at(&self) -> SimTime {
        self.action_ready_at
    }
}

/// Observer notified on registry mutations.
///
/// Mirrors the `InventoryObserver` pattern (see `model/inventory/events.rs`).
/// Implementors typically maintain a derived index that tracks something
/// derivable from registry contents (e.g. relay-cell residency for
/// HTM-to-relay travel scoring). Notifications fire after the registry's
/// internal state has been updated; receivers see a coherent post-commit
/// snapshot in `entry`.
pub trait Observer: Send + Sync {
    /// Called after a successful `register`. `entry` is the just-inserted
    /// snapshot (cell, tote, action_ready_at).
    fn on_register(&self, entry: &AvailableHtm);
    /// Called after a successful `take`. `entry` is the just-removed
    /// snapshot.
    fn on_take(&self, entry: &AvailableHtm);
}

/// Registry of available HTMs, keyed by bot id.
///
/// Clone is cheap (Arc bump). All clones share state.
///
/// The optional `parking` handle is the un-park lifecycle hook:
/// `take()` calls `parking.cancel(...)` so a claimed parked HTM's
/// reservation in the parking registry is released at the same logical
/// time the oracle's open-ended cell hold gets finalized by the next
/// path commit. Without this, the parking registry retains stale
/// open-ended entries indefinitely and `parking.find(...)` mis-allocates
/// already-occupied cells to other bots.
#[derive(Clone)]
pub struct AvailableHtmRegistry {
    inner: Arc<Mutex<FxHashMap<RobotId, AvailableHtm>>>,
    version: Version,
    next_availability: Arc<AtomicU64>,
    claims: Arc<Mutex<FxHashMap<HtmAvailabilityId, FxHashMap<ProposalId, ClaimRank>>>>,
    inventory: InventoryService,
    cell_positions: Arc<FxHashMap<CellId, WorldPos>>,
    idle_relay_cells: Arc<FxHashSet<CellId>>,
    parking: Option<ParkingRegistry>,
    observers: Arc<Mutex<Vec<Arc<dyn Observer>>>>,
    /// Selection churn: how many passes over the idle pool ran, and the
    /// wall time they burned. A pickup candidacy's premise is coarse
    /// enough that every fleet change sends its owner back to select
    /// again, so that cost is measured rather than assumed. Wall time is
    /// reporting only — no decision anywhere reads it.
    selection_runs: Arc<AtomicU64>,
    selection_nanos: Arc<AtomicU64>,
}

impl AvailableHtmRegistry {
    pub fn new(
        inventory: InventoryService,
        map: &WarehouseMap,
        idle_relay_cells: HashSet<CellId>,
        parking: Option<ParkingRegistry>,
    ) -> Self {
        let cell_positions = map.graph.cells().filter_map(|cell| cell.world_pos.map(|pos| (cell.id, pos))).collect();
        // Convert the externally-supplied HashSet to FxHashSet at the boundary;
        // internal lookups (`contains`) are on a per-tick hot path so the swap
        // pays for itself even after a one-shot conversion at construction.
        let idle_relay_cells: FxHashSet<CellId> = idle_relay_cells.into_iter().collect();
        Self {
            inner: Arc::new(Mutex::new(FxHashMap::default())),
            version: Version::new(),
            next_availability: Arc::new(AtomicU64::new(0)),
            claims: Arc::new(Mutex::new(FxHashMap::default())),
            inventory,
            cell_positions: Arc::new(cell_positions),
            idle_relay_cells: Arc::new(idle_relay_cells),
            parking,
            observers: Arc::new(Mutex::new(Vec::new())),
            selection_runs: Arc::new(AtomicU64::new(0)),
            selection_nanos: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn version(&self) -> &Version {
        &self.version
    }

    pub fn is_available(&self, id: HtmAvailabilityId) -> bool {
        self.inner.lock().unwrap().get(&id.robot).is_some_and(|entry| entry.availability_id == id)
    }

    fn claim(&self, entry: &AvailableHtm, owner: ProposalId, rank: ClaimRank) -> HtmClaim {
        let id = entry.availability_id;
        let previous = self.claims.lock().unwrap().entry(id).or_default().insert(owner, rank);
        assert!(previous.is_none(), "one proposal attempt registers its HTM claim once");
        HtmClaim { registry: self.clone(), id, owner }
    }

    fn ranked_claims(&self, id: HtmAvailabilityId, floor: SimTime) -> Option<Vec<ProposalId>> {
        let claims = self.claims.lock().unwrap();
        let claims = claims.get(&id)?;
        // A contest with no last resort is left to the earliest deadline.
        if claims.values().all(|rank| rank.kind == ClaimKind::Productive) {
            return None;
        }
        // The clearance starts at the operator's release, and no claim may take
        // the HTM off the station's cell later: one that would depart after the
        // clearance ranks behind every last resort. On a stay with no
        // clearance, every productive claim, however late, ranks ahead of the
        // last resort. Each start is floored at the barrier, the clearance's too.
        let release =
            claims.values().filter(|rank| rank.kind == ClaimKind::Clearance).map(|rank| rank.start.max(floor)).min();
        let mut ranked: Vec<_> = claims
            .iter()
            .map(|(&owner, rank)| {
                let start = rank.start.max(floor);
                ((release.is_some_and(|release| start > release), rank.kind != ClaimKind::Productive, start), owner)
            })
            .collect();
        ranked.sort_unstable();
        Some(ranked.into_iter().map(|(_, owner)| owner).collect())
    }

    /// A productive claim on one stay of an HTM, ranked against the other
    /// claims on that stay by its start.
    pub fn propose_productive(
        &self,
        entry: &AvailableHtm,
        owner: ProposalId,
        start: SimTime,
        clock: Mut<SimTime>,
    ) -> ProposedHtmClaim {
        self.propose_claim(entry, owner, start, clock, ClaimKind::Productive)
    }

    /// Parking's claim on an idle empty HTM's stay: it yields to every
    /// productive claim on the stay, however late that claim starts in simtime.
    pub fn propose_parking(
        &self,
        entry: &AvailableHtm,
        owner: ProposalId,
        start: SimTime,
        clock: Mut<SimTime>,
    ) -> ProposedHtmClaim {
        self.propose_claim(entry, owner, start, clock, ClaimKind::Parking)
    }

    /// A presentation's clearance's claim on the stay of the HTM the finished
    /// operation leaves on the station's cell: it yields to a productive claim
    /// that departs no later than it does (simtime), and outranks one that
    /// would depart later.
    pub fn propose_clearance(
        &self,
        entry: &AvailableHtm,
        owner: ProposalId,
        start: SimTime,
        clock: Mut<SimTime>,
    ) -> ProposedHtmClaim {
        self.propose_claim(entry, owner, start, clock, ClaimKind::Clearance)
    }

    fn propose_claim(
        &self,
        entry: &AvailableHtm,
        owner: ProposalId,
        start: SimTime,
        clock: Mut<SimTime>,
        kind: ClaimKind,
    ) -> ProposedHtmClaim {
        ProposedHtmClaim { claim: self.claim(entry, owner, ClaimRank { kind, start }), clock, start, accepted: None }
    }

    /// Record one pass over the idle pool and what it cost. See
    /// `selection_runs`.
    pub fn record_selection(&self, elapsed: Duration) {
        self.selection_runs.fetch_add(1, Ordering::Relaxed);
        self.selection_nanos.fetch_add(elapsed.as_nanos() as u64, Ordering::Relaxed);
    }

    /// End-of-run selection-churn line; `None` when no pass ever ran.
    pub fn selection_report(&self) -> Option<String> {
        let runs = self.selection_runs.load(Ordering::Relaxed);
        (runs > 0).then(|| {
            let ms = self.selection_nanos.load(Ordering::Relaxed) as f64 / 1_000_000.0;
            format!("htm pickup selection: {runs} runs, {ms:.1} ms total")
        })
    }

    /// Register an observer for register/take events.
    ///
    /// The observer is replayed `on_register` for every entry already in the
    /// registry at attach time, so an index attaching after seeding sees the
    /// full current state without a separate snapshot step.
    pub fn add_observer(&self, observer: Arc<dyn Observer>) {
        let snapshot: Vec<AvailableHtm> = self.inner.lock().unwrap().values().cloned().collect();
        for entry in &snapshot {
            observer.on_register(entry);
        }
        self.observers.lock().unwrap().push(observer);
    }

    /// Register an HTM as available. `action_ready_at` is when the bot is
    /// action-ready at `reservation.cell()` — separate from
    /// `reservation.window().start` (when its leading edge enters).
    /// `presentation_id` is `Some` only when the registration continues a
    /// presentation cycle (post-presentation, for the store chain to read);
    /// `None` for seed/parking registrations with no cycle context. Returns
    /// the stay it registered.
    pub fn register(
        &self,
        reservation: CellReservation,
        action_ready_at: SimTime,
        orientation: Orientation,
        presentation_id: Option<PresentationId>,
    ) -> Result<AvailableHtm, RegistryError> {
        let bot_id = reservation.robot();

        let tote = self.inventory.tote_on_robot(bot_id, action_ready_at).map(|(tote_id, contents)| CarriedTote {
            tote_id,
            contents,
            presentation_id,
        });

        let at_idle_relay = self.idle_relay_cells.contains(&reservation.cell());
        let mut guard = self.inner.lock().unwrap();
        if guard.contains_key(&bot_id) {
            return Err(RegistryError::AlreadyRegistered(bot_id));
        }
        let availability_id =
            HtmAvailabilityId { robot: bot_id, epoch: self.next_availability.fetch_add(1, Ordering::Relaxed) };
        let entry = AvailableHtm {
            availability_id,
            cell_reservation: reservation,
            action_ready_at,
            orientation,
            tote,
            at_idle_relay,
        };
        guard.insert(bot_id, entry.clone());
        drop(guard);
        self.version.bump();
        for observer in self.observers.lock().unwrap().iter() {
            observer.on_register(&entry);
        }
        Ok(entry)
    }

    /// Reinstate an entry exactly as `take` handed it out: same tote view,
    /// and the parking bay `take` cancelled is re-reserved. The version DOES
    /// bump — an actor that peeked during the take-window saw the HTM absent
    /// and went to sleep on the signal, so a silent restore is a lost wakeup
    /// (measured: the whole run parked idle). Callers whose failure is a
    /// LASTING condition must pre-check before `take` instead of leaning on
    /// this, or they wake themselves in a loop.
    pub fn put_back(&self, entry: AvailableHtm) -> Result<(), RegistryError> {
        let bot_id = entry.robot();
        let mut entries = self.inner.lock().unwrap();
        if entries.contains_key(&bot_id) {
            return Err(RegistryError::AlreadyRegistered(bot_id));
        }
        if entry.at_idle_relay
            && let Some(parking) = &self.parking
        {
            parking.reserve(entry.cell(), bot_id, entry.action_ready_at);
        }
        entries.insert(bot_id, entry.clone());
        drop(entries);
        self.version.bump();
        for observer in self.observers.lock().unwrap().iter() {
            observer.on_register(&entry);
        }
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    /// The stay the registry holds for the robot, if it has one.
    pub fn get(&self, bot_id: RobotId) -> Option<AvailableHtm> {
        self.inner.lock().unwrap().get(&bot_id).cloned()
    }

    pub fn contains(&self, bot_id: RobotId) -> bool {
        self.inner.lock().unwrap().contains_key(&bot_id)
    }

    /// The stay of `robot` while it is an idle empty HTM holding `cell`
    /// open-ended: a squatter that parking can move off a cell another task
    /// needs.
    pub fn squatter(&self, cell: CellId, robot: RobotId) -> Option<AvailableHtm> {
        self.get(robot)
            .filter(|stay| stay.cell() == cell && stay.tote.is_none() && stay.cell_reservation.is_open_ended())
    }

    /// Take an HTM out of the registry (claim it for a chain).
    /// Returns None if not registered (in an active chain or not seeded).
    ///
    /// As a side effect, also cancels the bot's parking reservation (if
    /// any) at `entry.action_ready_at`. `parking.cancel` is a no-op when
    /// the cell isn't parking-registered for this robot, so callers that
    /// claim non-parked bots see no change in behavior. Centralizing the
    /// cancel here means every claim path (`HtmPickup`,
    /// `feed_putaway_stations`, …) gets correct un-park lifecycle for
    /// free.
    pub fn take(&self, bot_id: RobotId) -> Option<AvailableHtm> {
        let entry = self.inner.lock().unwrap().remove(&bot_id)?;
        if let Some(parking) = &self.parking {
            parking.cancel(entry.cell(), entry.robot(), entry.action_ready_at);
        }
        for observer in self.observers.lock().unwrap().iter() {
            observer.on_take(&entry);
        }
        self.version.bump();
        Some(entry)
    }

    /// Take the robot only while the registry still holds the stay `id`
    /// names, so a claim written against a stay the registry no longer holds
    /// takes nothing.
    pub fn take_exact(&self, id: HtmAvailabilityId) -> Option<AvailableHtm> {
        if !self.is_available(id) {
            return None;
        }
        self.take(id.robot)
    }

    fn take_with_headroom(
        &self,
        bot_id: RobotId,
        at: SimTime,
        headroom: Option<&PickupHeadroom>,
    ) -> Option<AvailableHtm> {
        // Evaluate the external policy before taking the fleet lock. The
        // supplied Version invalidates proposals when that policy changes;
        // keeping the lock order one-way avoids coupling this generic pool to
        // the policy service's mutex.
        let protected_empty = headroom.and_then(PickupHeadroom::protected_empty);
        let mut entries = self.inner.lock().unwrap();
        if let Some(protected_empty) = protected_empty
            && entries.values().filter(|entry| entry.tote.is_none() && entry.action_ready_at <= at).count()
                <= protected_empty
        {
            return None;
        }
        let entry = entries.remove(&bot_id)?;
        drop(entries);
        if let Some(parking) = &self.parking {
            parking.cancel(entry.cell(), entry.robot(), entry.action_ready_at);
        }
        for observer in self.observers.lock().unwrap().iter() {
            observer.on_take(&entry);
        }
        self.version.bump();
        Some(entry)
    }

    /// Peek at all available HTMs matching a predicate.
    /// Does NOT consume — entries stay in the registry.
    pub fn peek_matching(&self, predicate: impl Fn(&AvailableHtm) -> bool) -> Vec<AvailableHtm> {
        let inner = self.inner.lock().unwrap();
        inner.values().filter(|entry| predicate(entry)).cloned().collect()
    }

    /// Find the nearest available HTMs matching a predicate, sorted by
    /// straight-line separation to `target` in whole millimetres. Returns at
    /// most `max_results` entries. Does NOT consume — caller must `take()`
    /// the winner.
    ///
    /// World positions rather than grid ordinals: a world-protocol map has no
    /// lattice, and ranking is all this is — commitment stays with the
    /// pathfinder (coding standards §8).
    pub fn find_nearest(
        &self,
        target: WorldPos,
        predicate: impl Fn(&AvailableHtm) -> bool,
        max_results: usize,
    ) -> Vec<AvailableHtm> {
        let inner = self.inner.lock().unwrap();
        let mut scored: Vec<(u64, AvailableHtm)> = inner
            .values()
            .filter(|entry| predicate(entry))
            .filter_map(|entry| {
                let p = self.cell_positions.get(&entry.cell()).copied()?;
                let d = ((p.x_mm - target.x_mm).powi(2) + (p.y_mm - target.y_mm).powi(2)).sqrt();
                Some((d.round() as u64, entry.clone()))
            })
            .collect();
        scored.sort_unstable_by_key(|(dist, entry)| (*dist, entry.robot()));
        scored.truncate(max_results);
        scored.into_iter().map(|(_, entry)| entry).collect()
    }
}

/// What a stay's contest ranks one claim by.
#[derive(Clone, Copy, Debug)]
struct ClaimRank {
    kind: ClaimKind,
    start: SimTime,
}

/// Why a claim wants the stay. Parking and a clearance are last resorts:
/// each exists so that an HTM nothing else wants still has its next task
/// decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClaimKind {
    /// Productive work: direct putaway supply, a pickup.
    Productive,
    /// Parking moves an idle empty HTM off the relay access cell it stands on.
    Parking,
    /// A presentation's clearance takes the HTM off the station's cell at
    /// the operator's release.
    Clearance,
}

struct HtmClaim {
    registry: AvailableHtmRegistry,
    id: HtmAvailabilityId,
    owner: ProposalId,
}

impl Drop for HtmClaim {
    fn drop(&mut self) {
        let mut families = self.registry.claims.lock().unwrap();
        if let Some(claims) = families.get_mut(&self.id) {
            claims.remove(&self.owner);
            if claims.is_empty() {
                families.remove(&self.id);
            }
        }
    }
}

pub struct ProposedHtmClaim {
    claim: HtmClaim,
    clock: Mut<SimTime>,
    start: SimTime,
    accepted: Option<(AvailableHtm, SimTime)>,
}

impl ProposedHtmClaim {
    pub fn into_accepted(mut self) -> Result<(AvailableHtm, SimTime), RegistryError> {
        self.accepted.take().ok_or(RegistryError::NotAccepted)
    }
}

impl fmt::Display for ProposedHtmClaim {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "htm-claim{{availability={:?} start={}}}", self.claim.id, self.start.max(self.clock.value()))
    }
}

impl ProposedReservation for ProposedHtmClaim {
    fn ranked_competitors(&self) -> Option<Vec<ProposalId>> {
        self.claim.registry.ranked_claims(self.claim.id, self.clock.value())
    }

    fn start_time(&self) -> Option<SimTime> {
        Some(self.start.max(self.clock.value()))
    }

    fn poisoned(&self) -> bool {
        !self.claim.registry.is_available(self.claim.id)
    }

    fn accept(&mut self) -> Result<(), AcceptError> {
        if self.poisoned() {
            return Err(AcceptError::Poisoned { detail: format!("HTM claim lost {:?}", self.claim.id) });
        }
        let entry = self.claim.registry.take_exact(self.claim.id).ok_or_else(|| AcceptError::ActualConflict {
            detail: format!("HTM claim lost {:?} during acceptance", self.claim.id),
        })?;
        self.accepted = Some((entry, self.start.max(self.clock.value())));
        Ok(())
    }
}

/// A last-resort claim on the stay of an HTM that would leave the hold it
/// stands on, plus the claim's `Vacate` on that hold: an entrant's need-by
/// brings the decision forward without moving the claim's start. Parking
/// proposes one, and so does a presentation's clearance.
pub struct ProposedLastResort {
    claim: ProposedHtmClaim,
    vacate: Vacate,
}

impl ProposedLastResort {
    /// Parking's, for an idle empty HTM, starting at the HTM's availability.
    pub fn parking(
        available: &AvailableHtmRegistry,
        navigation: &Navigation,
        entry: &AvailableHtm,
        owner: ProposalId,
        clock: Mut<SimTime>,
    ) -> Self {
        let claim = available.propose_parking(entry, owner, entry.action_ready_at, clock);
        Self::new(claim, navigation, entry, owner)
    }

    /// A presentation's clearance's, for the HTM a finished operation leaves
    /// on the station's cell, starting at the operator's release.
    pub fn clearance(
        available: &AvailableHtmRegistry,
        navigation: &Navigation,
        entry: &AvailableHtm,
        owner: ProposalId,
        clock: Mut<SimTime>,
    ) -> Self {
        let claim = available.propose_clearance(entry, owner, entry.action_ready_at, clock);
        Self::new(claim, navigation, entry, owner)
    }

    fn new(claim: ProposedHtmClaim, navigation: &Navigation, entry: &AvailableHtm, owner: ProposalId) -> Self {
        Self { claim, vacate: navigation.cells.vacate(&entry.cell_reservation, owner, entry.action_ready_at) }
    }

    /// The claimed stay and the departure acceptance fixed; the `Vacate` goes
    /// with the proposal.
    pub fn into_accepted(self) -> Result<(AvailableHtm, SimTime), RegistryError> {
        self.claim.into_accepted()
    }
}

impl fmt::Display for ProposedLastResort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "last-resort{{{}}}", self.claim)
    }
}

impl ProposedReservation for ProposedLastResort {
    fn ranked_competitors(&self) -> Option<Vec<ProposalId>> {
        ProposedReservation::ranked_competitors(&self.claim)
    }

    fn start_time(&self) -> Option<SimTime> {
        ProposedReservation::start_time(&self.claim)
    }

    fn resolve_by(&self) -> Option<SimTime> {
        let start = ProposedReservation::start_time(&self.claim)?;
        Some(self.vacate.resolve_by(start, self.claim.clock.value()))
    }

    fn poisoned(&self) -> bool {
        ProposedReservation::poisoned(&self.claim)
    }

    fn accept(&mut self) -> Result<(), AcceptError> {
        self.claim.accept()
    }
}

/// One selection pass's winner: which HTM, what its drive to the relay
/// costs, and when that drive would start.
pub struct PickupSelection {
    pub htm: AvailableHtm,
    pub travel: SimDuration,
    pub depart_anchor: SimTime,
    /// The chosen HTM already holds the relay access cell, so the pickup
    /// needs no drive at all.
    pub resident: bool,
}

// A revocable claim on one idle HTM: the robot a selection pass chose and
// the instant its drive would start, held with the registry's version
// count as the premise. Nothing is spent until acceptance, which takes the
// entry out of the pool and hands it back through this type's own field.
// Its claim records candidacy only; dropping it never consumes physical capacity.
//
// The premise is coarse on purpose — ANY registry change kills it, not
// only one touching the chosen robot. A pass ranks the whole idle pool, so
// the choice that gets accepted must be the one computed after the last
// fleet event before its start arrives: if a nearer HTM finishes its chain
// while this candidacy waits, that newcomer has to win.
pub struct ProposedHtmPickup {
    claim: HtmClaim,
    registry: AvailableHtmRegistry,
    navigation: Arc<Navigation>,
    relay_cell: CellId,
    selection: PickupSelection,
    /// The registry version count the selection pass read — the premise.
    seen: u64,
    clock: Mut<SimTime>,
    accepted: Option<(AvailableHtm, SimTime)>,
    headroom: Option<(PickupHeadroom, u64)>,
    route_horizon: Option<SimDuration>,
}

impl ProposedHtmPickup {
    pub fn new(
        owner: ProposalId,
        registry: AvailableHtmRegistry,
        navigation: Arc<Navigation>,
        relay_cell: CellId,
        selection: PickupSelection,
        seen: u64,
        clock: Mut<SimTime>,
    ) -> Self {
        let claim = registry.claim(
            &selection.htm,
            owner,
            ClaimRank { kind: ClaimKind::Productive, start: selection.depart_anchor },
        );
        Self {
            claim,
            registry,
            navigation,
            relay_cell,
            selection,
            seen,
            clock,
            accepted: None,
            headroom: None,
            route_horizon: None,
        }
    }

    pub fn new_guarded(
        owner: ProposalId,
        registry: AvailableHtmRegistry,
        navigation: Arc<Navigation>,
        relay_cell: CellId,
        selection: PickupSelection,
        seen: u64,
        clock: Mut<SimTime>,
        headroom: PickupHeadroom,
    ) -> Self {
        let policy_seen = headroom.changed.seen();
        let mut proposal = Self::new(owner, registry, navigation, relay_cell, selection, seen, clock);
        proposal.headroom = Some((headroom, policy_seen));
        proposal
    }

    pub fn with_headroom(mut self, headroom: PickupHeadroom) -> Self {
        let seen = headroom.version().seen();
        self.headroom = Some((headroom, seen));
        self
    }

    pub fn with_route_horizon(mut self, max_wait: SimDuration) -> Self {
        self.route_horizon = Some(max_wait);
        self
    }

    /// Exact path time for the chosen HTM's drive to the relay; zero when
    /// it is already standing there.
    pub fn travel_estimate(&self) -> SimDuration {
        self.selection.travel
    }

    /// See [`PickupSelection::resident`].
    pub fn resident(&self) -> bool {
        self.selection.resident
    }

    /// The claimed entry and the departure instant acceptance settled on.
    pub fn into_accepted(mut self) -> Result<(AvailableHtm, SimTime), RegistryError> {
        self.accepted.take().ok_or(RegistryError::NotAccepted)
    }

    fn robot(&self) -> RobotId {
        self.selection.htm.robot()
    }

    fn start(&self) -> SimTime {
        self.selection.depart_anchor.max(self.clock.value())
    }

    fn destination_available(&self) -> bool {
        self.selection.resident
            || !self
                .navigation
                .oracle
                .reservations_for_cell(self.relay_cell)
                .into_iter()
                .any(|hold| hold.robot() != self.robot() && hold.is_open_ended())
    }

    fn headroom_available(&self) -> bool {
        let Some((headroom, _)) = &self.headroom else { return true };
        headroom.allows(&self.registry, self.start())
    }
}

impl fmt::Display for ProposedHtmPickup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "htm-pickup{{htm={:?} start={} seen={}}}", self.robot(), self.start(), self.seen)
    }
}

impl ProposedReservation for ProposedHtmPickup {
    fn ranked_competitors(&self) -> Option<Vec<ProposalId>> {
        self.registry.ranked_claims(self.claim.id, self.clock.value())
    }

    // Scored on the drive's start. Self-flooring: an accepted chain plans
    // its later legs ahead of the settled frontier, so by the time the
    // barrier reaches this candidacy its anchor can already lie behind B.
    // Reporting the floored instant keeps the claim startable — physically
    // the HTM just stands where it is until then.
    fn start_time(&self) -> Option<SimTime> {
        Some(self.start())
    }

    // A newly occupied destination invalidates selection. Re-proposal keeps
    // the chosen HTM available while the caller waits for the holder, whose
    // next movement can require a later acceptance. An admissibility hold
    // here would pin the clock before that movement could be accepted.
    fn poisoned(&self) -> bool {
        self.registry.version().seen() != self.seen
            || self.headroom.as_ref().is_some_and(|(guard, seen)| guard.changed.seen() != *seen)
            || !self.destination_available()
            || self.navigation.htm_departure_is_blocked(
                self.robot(),
                self.selection.htm.cell(),
                self.relay_cell,
                false,
                self.start(),
            )
            || self.route_horizon.is_some_and(|max_wait| {
                self.navigation.htm_departure_exceeds_horizon(
                    self.robot(),
                    self.selection.htm.cell(),
                    self.relay_cell,
                    false,
                    self.start(),
                    max_wait,
                )
            })
    }

    fn admissible(&self) -> bool {
        self.headroom_available()
    }

    fn accept(&mut self) -> Result<(), AcceptError> {
        if self.poisoned() {
            return Err(AcceptError::Poisoned {
                detail: format!(
                    "htm pickup on {:?} lost its fleet, destination or departure premise (version {})",
                    self.robot(),
                    self.seen
                ),
            });
        }
        let depart = self.start();
        let robot = self.robot();
        let entry = self
            .registry
            .take_with_headroom(robot, depart, self.headroom.as_ref().map(|(guard, _)| guard))
            .ok_or_else(|| AcceptError::ActualConflict {
                detail: format!("htm {robot:?} was unavailable or protected by empty-fleet headroom at acceptance"),
            })?;
        self.accepted = Some((entry, depart));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    //! The pickup candidacy's contract: it spends nothing before
    //! acceptance, loses its premise to any fleet change at all,
    //! reselects when another robot occupies the destination, and claims
    //! its robot exactly once.
    use super::*;
    use crate::model::ids::TaskId;
    use crate::model::reservation::oracle::ReservationOracle;
    use crate::model::test_model::TestModel;
    use crate::model::test_model::{TestModelConfig, build_model, register_idle_htm};
    use crate::simtime::TimeRange;
    use desgraph::{CellId, WorldPos};
    use pathfinder::ReservationOracle as _;
    use std::sync::atomic::AtomicBool;

    /// A relay HTM access cell in the `test_model` line map.
    const RELAY_CELL: CellId = CellId(10);

    fn candidacy(
        model: &TestModel,
        robot: RobotId,
        anchor: SimTime,
        clock: Mut<SimTime>,
        resident: bool,
    ) -> ProposedHtmPickup {
        let htm = model.available_htm.peek_matching(|h| h.robot() == robot).pop().expect("registered entry");
        ProposedHtmPickup::new(
            ProposalId { task: TaskId(robot.0), attempt: 0 },
            model.available_htm.clone(),
            model.navigation.clone(),
            RELAY_CELL,
            PickupSelection { htm, travel: SimDuration::seconds(4), depart_anchor: anchor, resident },
            model.available_htm.version().seen(),
            clock,
        )
    }

    #[test]
    fn a_pickup_candidacy_spends_nothing_until_acceptance_claims_its_robot() {
        let (model, oracle) = build_model(TestModelConfig::default());
        let robot = RobotId(60);
        register_idle_htm(&model, &oracle, robot, CellId(5), SimTime::ZERO);

        let mut original = model.available_htm.take(robot).unwrap();
        original.orientation = Orientation::West;
        model.available_htm.put_back(original).unwrap();

        let mut proposal = candidacy(&model, robot, SimTime::seconds(10), Mut::new(SimTime::ZERO), false);
        assert!(!proposal.poisoned(), "a fresh candidacy holds its premise");
        assert!(model.available_htm.contains(robot), "publishing a candidacy leaves the robot in the pool");

        proposal.accept().expect("a live premise accepts");
        assert!(!model.available_htm.contains(robot), "acceptance is the claim");
        let (entry, depart) = proposal.into_accepted().expect("acceptance filled the entry slot");
        assert_eq!(entry.robot(), robot);
        assert_eq!(entry.orientation, Orientation::West, "restore and claim retain the chassis frontier");
        assert_eq!(depart, SimTime::seconds(10), "the accepted depart is the candidacy's start");
    }

    /// The premise is the whole fleet, not the chosen entry: a nearer HTM
    /// joining the pool while a candidacy waits has to be able to win it,
    /// so a newcomer that never touches the chosen robot still forces the
    /// owner back to select again.
    #[test]
    fn any_fleet_change_invalidates_a_pickup_candidacy() {
        let (model, oracle) = build_model(TestModelConfig::default());
        let chosen = RobotId(60);
        register_idle_htm(&model, &oracle, chosen, CellId(5), SimTime::ZERO);

        let proposal = candidacy(&model, chosen, SimTime::seconds(10), Mut::new(SimTime::ZERO), false);
        assert!(!proposal.poisoned());

        register_idle_htm(&model, &oracle, RobotId(61), CellId(7), SimTime::ZERO);
        assert!(proposal.poisoned(), "a newcomer elsewhere in the pool still forces re-selection");
    }

    /// A robot already claimed by another task can occupy the destination
    /// without changing the idle fleet. Its later departure needs the clock
    /// to advance, so the stale selection must be evicted rather than held.
    #[test]
    fn a_new_destination_holder_invalidates_selection_without_holding_the_clock() {
        let (model, oracle) = build_model(TestModelConfig::default());
        let robot = RobotId(60);
        register_idle_htm(&model, &oracle, robot, CellId(5), SimTime::ZERO);
        let mut proposal = candidacy(&model, robot, SimTime::seconds(10), Mut::new(SimTime::ZERO), false);
        let fleet_seen = model.available_htm.version().seen();
        assert!(!proposal.poisoned());

        oracle
            .table()
            .reserve_cell(RobotId(61), RELAY_CELL, TimeRange::open_ended(SimTime::ZERO))
            .expect("squatter hold");

        assert_eq!(model.available_htm.version().seen(), fleet_seen);
        assert!(proposal.poisoned(), "destination occupancy changed without an idle-fleet update");
        assert!(proposal.admissible(), "eviction must replace an earliest-start clock hold");
        assert!(matches!(proposal.accept(), Err(AcceptError::Poisoned { .. })));
        assert!(model.available_htm.contains(robot), "invalid selection claims no HTM");
    }

    #[test]
    fn unrelated_cell_holds_do_not_invalidate_pickup_selection() {
        let (model, oracle) = build_model(TestModelConfig::default());
        let robot = RobotId(60);
        register_idle_htm(&model, &oracle, robot, CellId(5), SimTime::ZERO);
        let proposal = candidacy(&model, robot, SimTime::seconds(10), Mut::new(SimTime::ZERO), false);
        oracle.table().reserve_cell(RobotId(61), CellId(30), TimeRange::open_ended(SimTime::ZERO)).unwrap();
        assert!(!proposal.poisoned());
    }

    #[test]
    fn future_exit_traffic_invalidates_a_pickup_before_it_claims_the_htm() {
        let (model, oracle) = build_model(TestModelConfig::default());
        let robot = RobotId(60);
        register_idle_htm(&model, &oracle, robot, CellId(5), SimTime::ZERO);
        let mut proposal = candidacy(&model, robot, SimTime::seconds(10), Mut::new(SimTime::ZERO), false)
            .with_route_horizon(SimDuration::seconds(300));
        assert!(!proposal.poisoned());
        let fleet_seen = model.available_htm.version().seen();
        for cell in [CellId(4), CellId(6)] {
            oracle
                .table()
                .reserve_cell(RobotId(61), cell, TimeRange::new(SimTime::ZERO, SimTime::seconds(400)))
                .unwrap();
        }
        assert_eq!(model.available_htm.version().seen(), fleet_seen);
        assert!(proposal.poisoned(), "finite exit traffic invalidates the departure premise");
        assert!(matches!(proposal.accept(), Err(AcceptError::Poisoned { .. })));
        assert!(model.available_htm.contains(robot));
        assert!(
            oracle.reservations_for_cell(CellId(5)).iter().any(|hold| hold.robot() == robot && hold.is_open_ended())
        );
    }

    #[test]
    fn a_resident_pickup_keeps_its_own_destination_hold() {
        let (model, oracle) = build_model(TestModelConfig::default());
        let robot = RobotId(60);
        register_idle_htm(&model, &oracle, robot, RELAY_CELL, SimTime::ZERO);
        let mut proposal = candidacy(&model, robot, SimTime::seconds(10), Mut::new(SimTime::ZERO), true);
        assert!(!proposal.poisoned());
        proposal.accept().unwrap();
        assert_eq!(proposal.into_accepted().unwrap().0.cell(), RELAY_CELL);
    }

    #[test]
    fn a_candidacy_start_floors_at_the_settled_frontier() {
        let (model, oracle) = build_model(TestModelConfig::default());
        let robot = RobotId(60);
        register_idle_htm(&model, &oracle, robot, CellId(5), SimTime::ZERO);

        let clock = Mut::new(SimTime::ZERO);
        let proposal = candidacy(&model, robot, SimTime::seconds(10), clock.clone(), false);
        assert_eq!(proposal.start_time(), Some(SimTime::seconds(10)), "an anchor ahead of B is reported as it is");

        clock.set(SimTime::seconds(25));
        assert_eq!(
            proposal.start_time(),
            Some(SimTime::seconds(25)),
            "an anchor B has passed rises with it, so acceptance never starts behind the settled present"
        );
    }

    #[test]
    fn guarded_pickups_atomically_preserve_empty_headroom() {
        let (model, oracle) = build_model(TestModelConfig::default());
        for (robot, cell) in [(60, 5), (61, 6), (62, 7)] {
            register_idle_htm(&model, &oracle, RobotId(robot), CellId(cell), SimTime::ZERO);
        }
        let required = Arc::new(AtomicBool::new(true));
        let policy_version = Version::new();
        let headroom = PickupHeadroom::new(
            2,
            {
                let required = required.clone();
                Arc::new(move || required.load(Ordering::Relaxed))
            },
            policy_version.clone(),
        );
        let mut first = candidacy(&model, RobotId(60), SimTime::ZERO, Mut::new(SimTime::ZERO), false)
            .with_headroom(headroom.clone());
        assert!(ProposedReservation::admissible(&first));
        first.accept().unwrap();

        let mut blocked =
            candidacy(&model, RobotId(61), SimTime::ZERO, Mut::new(SimTime::ZERO), false).with_headroom(headroom);
        assert!(!ProposedReservation::admissible(&blocked), "the two remaining empty HTMs are protected");
        assert!(blocked.accept().is_err(), "acceptance rechecks under the fleet mutex");
        assert!(model.available_htm.contains(RobotId(61)));

        required.store(false, Ordering::Relaxed);
        policy_version.bump();
        assert!(blocked.poisoned(), "policy changes force selection to republish");
    }

    #[test]
    fn future_empty_htms_do_not_satisfy_current_headroom() {
        let (model, oracle) = build_model(TestModelConfig::default());
        for (robot, cell) in [(60, 5), (61, 6), (62, 7)] {
            register_idle_htm(&model, &oracle, RobotId(robot), CellId(cell), SimTime::seconds((robot - 60).into()));
        }
        let headroom = PickupHeadroom::new(2, Arc::new(|| true), Version::new());
        assert!(!headroom.allows(&model.available_htm, SimTime::ZERO));
        assert_eq!(headroom.earliest_allowed_at(&model.available_htm, SimTime::ZERO), Some(SimTime::seconds(2)));
        assert!(headroom.allows(&model.available_htm, SimTime::seconds(2)));

        let mut proposal = candidacy(
            &model,
            RobotId(60),
            headroom.earliest_allowed_at(&model.available_htm, SimTime::ZERO).unwrap(),
            Mut::new(SimTime::ZERO),
            false,
        )
        .with_headroom(headroom);
        assert_eq!(ProposedReservation::start_time(&proposal), Some(SimTime::seconds(2)));
        proposal.accept().unwrap();
        assert_eq!(proposal.into_accepted().unwrap().1, SimTime::seconds(2));
    }

    #[test]
    fn dynamic_headroom_can_reduce_protection_then_bypass() {
        let (model, oracle) = build_model(TestModelConfig::default());
        for (robot, cell) in [(60, 5), (61, 6), (62, 7)] {
            register_idle_htm(&model, &oracle, RobotId(robot), CellId(cell), SimTime::ZERO);
        }
        let policy = Arc::new(Mutex::new(Some(2usize)));
        let changed = Version::new();
        let headroom = PickupHeadroom::dynamic(
            {
                let policy = policy.clone();
                Arc::new(move || *policy.lock().unwrap())
            },
            changed.clone(),
        );

        model.available_htm.take_with_headroom(RobotId(60), SimTime::ZERO, Some(&headroom)).unwrap();
        assert!(!headroom.allows(&model.available_htm, SimTime::ZERO));

        *policy.lock().unwrap() = Some(1);
        changed.bump();
        assert!(headroom.allows(&model.available_htm, SimTime::ZERO));
        model.available_htm.take_with_headroom(RobotId(61), SimTime::ZERO, Some(&headroom)).unwrap();

        *policy.lock().unwrap() = None;
        changed.bump();
        assert!(headroom.allows(&model.available_htm, SimTime::ZERO));
        model.available_htm.take_with_headroom(RobotId(62), SimTime::ZERO, Some(&headroom)).unwrap();
        assert_eq!(model.available_htm.len(), 0);
    }

    /// The idle empty HTM parking would move stands on a relay access cell,
    /// where nothing rules out a later departure: a productive claim outranks
    /// parking however late it starts.
    #[test]
    fn a_productive_claim_outranks_an_earlier_parking_claim_and_its_acceptance_poisons_it() {
        let (model, oracle) = build_model(TestModelConfig::default());
        let robot = RobotId(60);
        register_idle_htm(&model, &oracle, robot, CellId(5), SimTime::ZERO);
        let entry = model.available_htm.get(robot).unwrap();
        let clock = Mut::new(SimTime::ZERO);
        let parking_id = ProposalId { task: TaskId(1), attempt: 0 };
        let productive_id = ProposalId { task: TaskId(2), attempt: 0 };
        let parking = model.available_htm.propose_parking(&entry, parking_id, SimTime::seconds(1), clock.clone());
        let mut productive = model.available_htm.propose_productive(&entry, productive_id, SimTime::seconds(5), clock);
        assert_eq!(ProposedReservation::ranked_competitors(&parking), Some(vec![productive_id, parking_id]));
        assert!(!ProposedReservation::poisoned(&parking));

        ProposedReservation::accept(&mut productive).unwrap();
        let (taken, start) = productive.into_accepted().unwrap();
        assert_eq!(taken.availability_id(), entry.availability_id());
        assert_eq!(start, SimTime::seconds(5));
        assert!(ProposedReservation::poisoned(&parking));
        assert!(model.available_htm.get(robot).is_none());
    }

    /// The clearance starts at the operator's release, 10 s here. A productive
    /// claim that departs then outranks it, and one that would depart later
    /// ranks behind it. Once the barrier passes both starts, every claim would
    /// depart at the barrier, and the productive ones outrank the clearance.
    #[test]
    fn a_clearance_ranks_behind_a_productive_claim_departing_with_it_and_ahead_of_one_departing_later() {
        let (model, oracle) = build_model(TestModelConfig::default());
        let robot = RobotId(60);
        register_idle_htm(&model, &oracle, robot, CellId(5), SimTime::seconds(10));
        let entry = model.available_htm.get(robot).unwrap();
        let clock = Mut::new(SimTime::ZERO);
        let [clearance_id, late_id, on_time_id] = [1, 2, 3].map(|task| ProposalId { task: TaskId(task), attempt: 0 });
        let clearance =
            model.available_htm.propose_clearance(&entry, clearance_id, SimTime::seconds(10), clock.clone());
        let _late = model.available_htm.propose_productive(&entry, late_id, SimTime::seconds(15), clock.clone());
        let _on_time = model.available_htm.propose_productive(&entry, on_time_id, SimTime::seconds(10), clock.clone());
        assert_eq!(ProposedReservation::ranked_competitors(&clearance), Some(vec![on_time_id, clearance_id, late_id]));

        clock.set(SimTime::seconds(20));
        assert_eq!(ProposedReservation::ranked_competitors(&clearance), Some(vec![late_id, on_time_id, clearance_id]));
    }

    /// Robot 60 idles empty on cell 5, robot 61 carries a tote on cell 7, and
    /// robot 62's hold on cell 9 ends at 10 s: only robot 60 on cell 5 is a
    /// squatter.
    #[test]
    fn a_squatter_is_an_idle_empty_htm_holding_its_cell_open_ended() {
        use crate::model::location::{ToteDesignation, ToteLocation, ToteType};
        let (model, oracle) = build_model(TestModelConfig::default());
        register_idle_htm(&model, &oracle, RobotId(60), CellId(5), SimTime::ZERO);
        let carried = ToteLocation::WithRobot { robot_id: RobotId(61) };
        model.inventory.register_tote(ToteId(1), ToteType::Tote, ToteDesignation::Inventory, carried).unwrap();
        register_idle_htm(&model, &oracle, RobotId(61), CellId(7), SimTime::ZERO);
        let bounded = TimeRange::new(SimTime::ZERO, SimTime::seconds(10));
        let hold = oracle.table().reserve_cell(RobotId(62), CellId(9), bounded).unwrap();
        model.available_htm.register(hold, SimTime::ZERO, Orientation::East, None).unwrap();
        let idle = model.available_htm.get(RobotId(60)).unwrap().availability_id();
        assert_eq!(model.available_htm.squatter(CellId(5), RobotId(60)).map(|stay| stay.availability_id()), Some(idle));
        assert!(model.available_htm.squatter(CellId(6), RobotId(60)).is_none(), "robot 60 is not on cell 6");
        assert!(model.available_htm.squatter(CellId(7), RobotId(61)).is_none(), "robot 61 carries a tote");
        assert!(model.available_htm.squatter(CellId(9), RobotId(62)).is_none(), "robot 62's hold ends");
    }

    #[test]
    fn take_exact_finds_nothing_for_a_stay_the_registry_no_longer_holds() {
        let (model, oracle) = build_model(TestModelConfig::default());
        let robot = RobotId(60);
        register_idle_htm(&model, &oracle, robot, CellId(5), SimTime::ZERO);
        let first = model.available_htm.take(robot).unwrap();
        model
            .available_htm
            .register(first.cell_reservation.clone(), SimTime::seconds(20), first.orientation, None)
            .unwrap();
        let second = model.available_htm.get(robot).unwrap().availability_id();
        assert_ne!(second, first.availability_id());
        assert!(model.available_htm.take_exact(first.availability_id()).is_none());
        assert!(model.available_htm.contains(robot), "a stale claim takes nothing");
        assert_eq!(model.available_htm.take_exact(second).unwrap().availability_id(), second);
    }
}
