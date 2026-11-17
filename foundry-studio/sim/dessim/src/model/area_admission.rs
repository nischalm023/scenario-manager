//! Identity-bearing, simulation-time capacity claims for physical areas.

use std::hash::Hash;
use std::sync::{Arc, Mutex};

use rustc_hash::FxHashMap;

use super::SimTime;
use crate::signals::Version;
use crate::state::{StateError, StateManager, States};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Membership {
    pub entry: SimTime,
    pub exit: Option<SimTime>,
    pub slot: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdmissionQuote {
    pub entry: SimTime,
    pub slot: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AreaAdmissionError {
    DuplicateArea,
    ZeroCapacity,
    UnknownArea,
    AlreadyClaimed,
    AlreadyMember,
    UnknownMember,
    SpentClaim,
    PrematureExit,
    AlreadyExited,
    CapacityChanged,
    PrematureEntry,
}

#[derive(Default)]
struct Calendar {
    changes: Vec<(SimTime, isize)>,
}

impl Calendar {
    fn record(&mut self, at: SimTime, delta: isize) {
        match self.changes.binary_search_by_key(&at, |&(time, _)| time) {
            Ok(index) => {
                self.changes[index].1 += delta;
                if self.changes[index].1 == 0 {
                    self.changes.remove(index);
                }
            }
            Err(index) => self.changes.insert(index, (at, delta)),
        }
    }

    fn count_at(&self, at: SimTime) -> usize {
        self.changes.iter().take_while(|&&(time, _)| time <= at).map(|&(_, delta)| delta).sum::<isize>() as usize
    }

    fn earliest_open(&self, capacity: usize, after: SimTime) -> Option<SimTime> {
        let mut count = self.count_at(after) as isize;
        let mut candidate = after;
        for &(at, delta) in self.changes.iter().filter(|&&(at, _)| at > after) {
            if count >= capacity as isize {
                candidate = at;
            }
            count += delta;
        }
        (count < capacity as isize).then_some(candidate)
    }
}

struct Area {
    slots: Vec<Calendar>,
}

enum MemberState {
    Provisional { claim: u64, entry: SimTime },
    Committed(Membership),
}

struct Inner<A, M> {
    next_claim: u64,
    areas: FxHashMap<A, Area>,
    members: FxHashMap<M, (A, MemberState)>,
}

pub struct AreaAdmission<A, M> {
    shared: Arc<Mutex<Inner<A, M>>>,
    version: Version,
}

impl<A, M> Clone for AreaAdmission<A, M> {
    fn clone(&self) -> Self {
        Self { shared: self.shared.clone(), version: self.version.clone() }
    }
}

impl<A, M> AreaAdmission<A, M>
where
    A: Clone + Eq + Hash,
    M: Clone + Eq + Hash,
{
    pub fn empty() -> Self {
        Self {
            shared: Arc::new(Mutex::new(Inner {
                next_claim: 0,
                areas: FxHashMap::default(),
                members: FxHashMap::default(),
            })),
            version: Version::new(),
        }
    }

    pub fn new(areas: impl IntoIterator<Item = (A, usize)>) -> Result<Self, AreaAdmissionError> {
        let service = Self::empty();
        for (key, capacity) in areas {
            service.declare_area(key, capacity)?;
        }
        Ok(service)
    }

    pub fn declare_area(&self, key: A, capacity: usize) -> Result<(), AreaAdmissionError> {
        if capacity == 0 {
            return Err(AreaAdmissionError::ZeroCapacity);
        }
        let mut inner = self.shared.lock().unwrap();
        if inner.areas.contains_key(&key) {
            return Err(AreaAdmissionError::DuplicateArea);
        }
        inner.areas.insert(key, Area { slots: (0..capacity).map(|_| Calendar::default()).collect() });
        drop(inner);
        self.version.bump();
        Ok(())
    }

    pub fn version(&self) -> &Version {
        &self.version
    }

    pub fn quote(&self, area: &A, after: SimTime) -> Result<Option<AdmissionQuote>, AreaAdmissionError> {
        let inner = self.shared.lock().unwrap();
        let area = inner.areas.get(area).ok_or(AreaAdmissionError::UnknownArea)?;
        Ok(area
            .slots
            .iter()
            .enumerate()
            .filter_map(|(slot, calendar)| calendar.earliest_open(1, after).map(|entry| AdmissionQuote { entry, slot }))
            .min_by_key(|quote| (quote.entry, quote.slot)))
    }

    /// Accept a published quote and take exclusive provisional custody of its
    /// slot. Call this before committing the physical route; dropping the
    /// returned claim rolls the slot back if route planning or commit fails.
    pub fn claim(&self, area: A, member: M, quote: AdmissionQuote) -> Result<AdmissionClaim<A, M>, AreaAdmissionError> {
        let mut inner = self.shared.lock().unwrap();
        if let Some((_, state)) = inner.members.get(&member) {
            return Err(match state {
                MemberState::Provisional { .. } => AreaAdmissionError::AlreadyClaimed,
                MemberState::Committed(_) => AreaAdmissionError::AlreadyMember,
            });
        }
        let area_state = inner.areas.get_mut(&area).ok_or(AreaAdmissionError::UnknownArea)?;
        let Some(slot) = area_state.slots.get_mut(quote.slot) else { return Err(AreaAdmissionError::CapacityChanged) };
        if slot.earliest_open(1, quote.entry) != Some(quote.entry) {
            return Err(AreaAdmissionError::CapacityChanged);
        }
        slot.record(quote.entry, 1);
        let claim = inner.next_claim;
        inner.next_claim += 1;
        inner.members.insert(member.clone(), (area.clone(), MemberState::Provisional { claim, entry: quote.entry }));
        drop(inner);
        self.version.bump();
        Ok(AdmissionClaim {
            shared: self.shared.clone(),
            version: self.version.clone(),
            area,
            member,
            claim,
            entry: quote.entry,
            slot: quote.slot,
            active: true,
        })
    }

    pub fn membership(&self, member: &M) -> Option<Membership> {
        let inner = self.shared.lock().unwrap();
        match &inner.members.get(member)?.1 {
            MemberState::Committed(membership) => Some(*membership),
            MemberState::Provisional { .. } => None,
        }
    }

    /// An accepted return right, including a provisional claim whose entry
    /// route is still being prepared. This does not assert physical residency.
    pub fn claimed_entry(&self, member: &M) -> Option<SimTime> {
        let inner = self.shared.lock().unwrap();
        match &inner.members.get(member)?.1 {
            MemberState::Committed(membership) => Some(membership.entry),
            MemberState::Provisional { entry, .. } => Some(*entry),
        }
    }

    pub fn memberships(&self, area: &A) -> Vec<(M, Membership)> {
        let inner = self.shared.lock().unwrap();
        inner
            .members
            .iter()
            .filter_map(|(member, (member_area, state))| match state {
                MemberState::Committed(membership) if member_area == area => Some((member.clone(), *membership)),
                _ => None,
            })
            .collect()
    }

    pub fn contains(&self, area: &A, member: &M, at: SimTime) -> bool {
        let inner = self.shared.lock().unwrap();
        let Some((member_area, MemberState::Committed(membership))) = inner.members.get(member) else { return false };
        member_area == area && membership.entry <= at && membership.exit.is_none_or(|exit| at < exit)
    }

    pub fn occupancy(&self, area: &A, at: SimTime) -> Result<usize, AreaAdmissionError> {
        let inner = self.shared.lock().unwrap();
        let area = inner.areas.get(area).ok_or(AreaAdmissionError::UnknownArea)?;
        Ok(area.slots.iter().filter(|calendar| calendar.count_at(at) != 0).count())
    }

    /// Record the area-clear time from an already committed egress plan.
    pub fn commit_exit(&self, area: &A, member: &M, clear_at: SimTime) -> Result<(), AreaAdmissionError> {
        let mut inner = self.shared.lock().unwrap();
        let (member_area, state) = inner.members.get_mut(member).ok_or(AreaAdmissionError::UnknownMember)?;
        if member_area != area {
            return Err(AreaAdmissionError::UnknownMember);
        }
        let MemberState::Committed(membership) = state else { return Err(AreaAdmissionError::UnknownMember) };
        if membership.exit.is_some() {
            return Err(AreaAdmissionError::AlreadyExited);
        }
        if clear_at < membership.entry {
            return Err(AreaAdmissionError::PrematureExit);
        }
        membership.exit = Some(clear_at);
        let slot = membership.slot;
        inner.areas.get_mut(area).unwrap().slots[slot].record(clear_at, -1);
        drop(inner);
        self.version.bump();
        Ok(())
    }
}

impl<A, M> StateManager for AreaAdmission<A, M>
where
    A: Clone + Eq + Hash + 'static,
    M: Clone + Eq + Hash + 'static,
{
    fn create(_: &States) -> Result<Self, StateError> {
        Ok(Self::empty())
    }
}

pub struct AdmissionClaim<A: Clone + Eq + Hash, M: Clone + Eq + Hash> {
    shared: Arc<Mutex<Inner<A, M>>>,
    version: Version,
    area: A,
    member: M,
    claim: u64,
    entry: SimTime,
    slot: usize,
    active: bool,
}

impl<A, M> AdmissionClaim<A, M>
where
    A: Clone + Eq + Hash,
    M: Clone + Eq + Hash,
{
    pub fn entry(&self) -> SimTime {
        self.entry
    }

    pub fn slot(&self) -> usize {
        self.slot
    }

    pub fn finalize(&mut self, actual_entry: SimTime) -> Result<Membership, AreaAdmissionError> {
        if !self.active {
            return Err(AreaAdmissionError::SpentClaim);
        }
        let mut inner = self.shared.lock().unwrap();
        if actual_entry < self.entry {
            return Err(AreaAdmissionError::PrematureEntry);
        }
        let Some((area, state)) = inner.members.get(&self.member) else {
            return Err(AreaAdmissionError::CapacityChanged);
        };
        if area != &self.area
            || !matches!(state, MemberState::Provisional { claim, entry } if *claim == self.claim && *entry == self.entry)
        {
            return Err(AreaAdmissionError::CapacityChanged);
        }
        let area = inner.areas.get_mut(&self.area).expect("a claimed area cannot disappear");
        area.slots[self.slot].record(self.entry, -1);
        area.slots[self.slot].record(actual_entry, 1);
        let membership = Membership { entry: actual_entry, exit: None, slot: self.slot };
        inner.members.get_mut(&self.member).unwrap().1 = MemberState::Committed(membership);
        self.active = false;
        drop(inner);
        self.version.bump();
        Ok(membership)
    }

    pub fn cancel(&mut self) -> Result<(), AreaAdmissionError> {
        if !self.active {
            return Err(AreaAdmissionError::SpentClaim);
        }
        self.rollback();
        self.active = false;
        Ok(())
    }

    fn rollback(&self) {
        let mut inner = self.shared.lock().unwrap();
        if matches!(inner.members.get(&self.member), Some((area, MemberState::Provisional { claim, .. })) if area == &self.area && *claim == self.claim)
        {
            inner.members.remove(&self.member);
            inner.areas.get_mut(&self.area).unwrap().slots[self.slot].record(self.entry, -1);
            drop(inner);
            self.version.bump();
        }
    }
}

impl<A: Clone + Eq + Hash, M: Clone + Eq + Hash> Drop for AdmissionClaim<A, M> {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        self.rollback();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(ms: u64) -> SimTime {
        SimTime::ms(ms)
    }

    fn q(ms: u64, slot: usize) -> Option<AdmissionQuote> {
        Some(AdmissionQuote { entry: t(ms), slot })
    }

    fn claim(
        service: &AreaAdmission<&'static str, i32>,
        area: &'static str,
        member: i32,
        after: SimTime,
    ) -> Result<AdmissionClaim<&'static str, i32>, AreaAdmissionError> {
        let quote = service.quote(&area, after)?.ok_or(AreaAdmissionError::CapacityChanged)?;
        service.claim(area, member, quote)
    }

    #[test]
    fn areas_are_independent_and_duplicate_identity_is_rejected() {
        let service = AreaAdmission::new([("left", 1), ("right", 1)]).unwrap();
        let left_quote = service.quote(&"left", t(10)).unwrap().unwrap();
        let right_quote = service.quote(&"right", t(0)).unwrap().unwrap();
        let mut left = service.claim("left", 7, left_quote).unwrap();
        assert_eq!(left.entry(), t(10));
        assert!(matches!(service.claim("left", 7, left_quote), Err(AreaAdmissionError::AlreadyClaimed)));
        let membership = left.finalize(t(10)).unwrap();
        assert_eq!(membership, Membership { entry: t(10), exit: None, slot: 0 });
        assert!(matches!(service.claim("right", 7, right_quote), Err(AreaAdmissionError::AlreadyMember)));
        assert!(service.contains(&"left", &7, t(10)));
        assert!(!service.contains(&"left", &7, t(9)));
    }

    #[test]
    fn committed_future_exit_enables_same_timestamp_replacement() {
        let service = AreaAdmission::new([("area", 1)]).unwrap();
        let mut first = claim(&service, "area", 1, t(10)).unwrap();
        first.finalize(t(10)).unwrap();
        assert_eq!(service.quote(&"area", t(20)).unwrap(), None);
        service.commit_exit(&"area", &1, t(100)).unwrap();

        let mut second = claim(&service, "area", 2, t(20)).unwrap();
        assert_eq!(second.entry(), t(100));
        second.finalize(t(100)).unwrap();
        assert_eq!(service.occupancy(&"area", t(99)).unwrap(), 1);
        assert_eq!(service.occupancy(&"area", t(100)).unwrap(), 1);
        assert!(!service.contains(&"area", &1, t(100)));
        assert!(service.contains(&"area", &2, t(100)));
    }

    #[test]
    fn canceled_proposal_restores_capacity_and_claim_can_be_repeated() {
        let service = AreaAdmission::new([("area", 1)]).unwrap();
        let mut proposal = claim(&service, "area", 1, t(5)).unwrap();
        assert_eq!(service.quote(&"area", t(5)).unwrap(), None);
        proposal.cancel().unwrap();
        assert_eq!(proposal.cancel().unwrap_err(), AreaAdmissionError::SpentClaim);
        assert_eq!(service.quote(&"area", t(5)).unwrap(), q(5, 0));
        drop(claim(&service, "area", 1, t(7)).unwrap());
        assert_eq!(service.quote(&"area", t(7)).unwrap(), q(7, 0));
    }

    #[test]
    fn dropping_a_provisional_claim_restores_capacity_and_signals_waiters() {
        let service = AreaAdmission::new([("area", 1)]).unwrap();
        let before = service.version().seen();
        let provisional = claim(&service, "area", 1, t(5)).unwrap();
        let claimed = service.version().seen();
        assert!(claimed > before);
        drop(provisional);
        assert!(service.version().seen() > claimed);
        assert_eq!(service.quote(&"area", t(5)).unwrap(), q(5, 0));
    }

    #[test]
    fn exits_are_explicit_and_cannot_be_premature_or_repeated() {
        let service = AreaAdmission::new([("area", 1)]).unwrap();
        assert_eq!(service.commit_exit(&"area", &9, t(10)).unwrap_err(), AreaAdmissionError::UnknownMember);
        let mut claim = claim(&service, "area", 9, t(20)).unwrap();
        claim.finalize(t(20)).unwrap();
        assert_eq!(service.commit_exit(&"area", &9, t(19)).unwrap_err(), AreaAdmissionError::PrematureExit);
        service.commit_exit(&"area", &9, t(20)).unwrap();
        assert_eq!(service.commit_exit(&"area", &9, t(21)).unwrap_err(), AreaAdmissionError::AlreadyExited);
    }

    #[test]
    fn chronological_booking_can_be_requested_out_of_order() {
        let service = AreaAdmission::new([("area", 2)]).unwrap();
        let mut late = claim(&service, "area", 1, t(200)).unwrap();
        let mut early = claim(&service, "area", 2, t(10)).unwrap();
        assert_eq!(early.entry(), t(10));
        assert_eq!(late.entry(), t(200));
        early.finalize(t(10)).unwrap();
        late.finalize(t(200)).unwrap();
        assert_eq!(service.occupancy(&"area", t(199)).unwrap(), 1);
        assert_eq!(service.occupancy(&"area", t(200)).unwrap(), 2);
    }

    #[test]
    fn published_quotes_do_not_poison_capacity_and_loser_fails_before_route_commit() {
        let service = AreaAdmission::new([("area", 1)]).unwrap();
        let later_quote = service.quote(&"area", t(20)).unwrap().unwrap();
        let earlier_quote = service.quote(&"area", t(10)).unwrap().unwrap();
        let mut later = service.claim("area", 1, later_quote).unwrap();
        assert!(matches!(service.claim("area", 2, earlier_quote), Err(AreaAdmissionError::CapacityChanged)));
        later.finalize(t(20)).unwrap();
        assert_eq!(service.occupancy(&"area", t(19)).unwrap(), 0);
        assert_eq!(service.occupancy(&"area", t(20)).unwrap(), 1);
    }

    #[test]
    fn committed_entry_may_narrow_a_quote_to_a_later_crossing() {
        let service = AreaAdmission::new([("area", 1)]).unwrap();
        let mut claim = claim(&service, "area", 1, t(10)).unwrap();
        assert_eq!(claim.finalize(t(25)).unwrap(), Membership { entry: t(25), exit: None, slot: 0 });
        assert_eq!(service.occupancy(&"area", t(24)).unwrap(), 0);
        assert_eq!(service.occupancy(&"area", t(25)).unwrap(), 1);
    }

    #[test]
    fn availability_mutations_bump_the_version_signal() {
        let service = AreaAdmission::<&'static str, i32>::empty();
        let initial = service.version().seen();
        service.declare_area("area", 1).unwrap();
        assert!(service.version().seen() > initial);

        let quote = service.quote(&"area", t(10)).unwrap().unwrap();
        let before_claim = service.version().seen();
        let mut claim = service.claim("area", 1, quote).unwrap();
        assert!(service.version().seen() > before_claim);
        let before_finalize = service.version().seen();
        claim.finalize(t(20)).unwrap();
        assert!(service.version().seen() > before_finalize);
        let before_exit = service.version().seen();
        service.commit_exit(&"area", &1, t(30)).unwrap();
        assert!(service.version().seen() > before_exit);

        let second_quote = service.quote(&"area", t(30)).unwrap().unwrap();
        let mut second = service.claim("area", 2, second_quote).unwrap();
        let before_cancel = service.version().seen();
        second.cancel().unwrap();
        assert!(service.version().seen() > before_cancel);

        let dropped_quote = service.quote(&"area", t(30)).unwrap().unwrap();
        let dropped = service.claim("area", 3, dropped_quote).unwrap();
        let before_drop = service.version().seen();
        drop(dropped);
        assert!(service.version().seen() > before_drop);
    }

    #[test]
    fn state_manager_constructs_empty_and_accepts_late_area_declarations() {
        type TestAdmission = AreaAdmission<&'static str, i32>;
        let states = States::default();
        let service = states.get::<TestAdmission>().unwrap();
        assert_eq!(service.quote(&"late", t(0)).unwrap_err(), AreaAdmissionError::UnknownArea);
        service.declare_area("late", 2).unwrap();
        assert_eq!(service.quote(&"late", t(0)).unwrap(), q(0, 0));
        assert!(Arc::ptr_eq(&service, &states.get::<TestAdmission>().unwrap()));
    }
}
