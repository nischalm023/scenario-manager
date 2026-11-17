use std::any::Any;
use std::fmt;

use crate::model::SimTime;
use crate::model::ids::TaskId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProposalId {
    pub task: TaskId,
    pub attempt: u32,
}

impl fmt::Display for ProposalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.task, self.attempt)
    }
}

/// Whole-task candidacy at the trailing barrier. The earliest decision deadline
/// triggers acceptance of that proposal, or a contest ranked by its resource
/// family. The scheduler checks each contender's complete proposal before
/// accepting its reservations together.
pub trait Proposal: Any + fmt::Display {
    fn reservations(&mut self) -> Vec<&mut dyn ProposedReservation>;

    /// Family-ranked live attempts sharing a mutually exclusive claim. The
    /// due deadline triggers this contest; a higher-ranked attempt may start later.
    fn ranked_competitors(&self) -> Option<Vec<ProposalId>> {
        None
    }

    /// When an existing claim needs a decision. This can precede physical
    /// start when another task needs its release planned; it never changes
    /// the action's physical start time.
    fn resolve_by(&self) -> Option<SimTime> {
        self.start_time()
    }

    /// The sim instant this proposal's claim would start, when it
    /// carries one. Always a start, never an arrival or completion: the
    /// barrier must resolve this claim by the time its frontier reaches
    /// this instant. Family arbitration can accept it earlier; no claim
    /// may start behind the settled present.
    fn start_time(&self) -> Option<SimTime> {
        None
    }

    /// True once a conflicting state change killed this candidacy: it can
    /// never accept, so the scheduler evicts it and the owner re-selects
    /// under the changed state.
    fn poisoned(&self) -> bool {
        false
    }

    /// Whether the proposal's FULL claim could be granted right now —
    /// beyond the proposed reservation itself (e.g. zone inbound
    /// headroom). The scheduler never accepts an inadmissible candidacy
    /// (acceptance would refuse) and never advances the barrier past
    /// one; it stays pending until its family's conditions change.
    fn admissible(&self) -> bool {
        true
    }
}

// Acceptance converts a tentative into the real reservation IN PLACE,
// so the proposal's own concrete fields carry the accepted state back
// into `schedule` — no grant, no downcast. A refusal is a
// family-service bug and aborts the run; there is no unwind path. The
// premise: a tentative's validity is poison-swept on every conflicting
// state change, and the scheduler checks `poisoned`/`admissible` before
// accepting — so by accept time nothing refusable remains.
//
// Resource families define which claims compete and how they rank. The
// scheduler sees opaque proposal ids, never resource identities or task types.
// `resolve_by` triggers resolution; family preference may differ from time.
pub trait ProposedReservation: fmt::Display {
    fn accept(&mut self) -> Result<(), AcceptError>;

    fn ranked_competitors(&self) -> Option<Vec<ProposalId>> {
        None
    }

    fn resolve_by(&self) -> Option<SimTime> {
        self.start_time()
    }

    /// The claim's start instant, if any; feeds [`Proposal::start_time`]
    /// through the blanket impl.
    fn start_time(&self) -> Option<SimTime> {
        None
    }

    /// Feeds [`Proposal::poisoned`] through the blanket impl.
    fn poisoned(&self) -> bool {
        false
    }

    /// Feeds [`Proposal::admissible`] through the blanket impl.
    fn admissible(&self) -> bool {
        true
    }
}

impl<R: ProposedReservation + Any> Proposal for R {
    fn resolve_by(&self) -> Option<SimTime> {
        ProposedReservation::resolve_by(self)
    }

    fn ranked_competitors(&self) -> Option<Vec<ProposalId>> {
        ProposedReservation::ranked_competitors(self)
    }

    fn reservations(&mut self) -> Vec<&mut dyn ProposedReservation> {
        vec![self]
    }

    fn start_time(&self) -> Option<SimTime> {
        ProposedReservation::start_time(self)
    }

    fn poisoned(&self) -> bool {
        ProposedReservation::poisoned(self)
    }

    fn admissible(&self) -> bool {
        ProposedReservation::admissible(self)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AcceptError {
    #[error("tentative poisoned before acceptance: {detail}")]
    Poisoned { detail: String },
    #[error("acceptance conflict on the actual stratum: {detail}")]
    ActualConflict { detail: String },
    #[error("acceptance left torn state: {detail}")]
    TornState { detail: String },
}

pub struct EmptyProposal;

impl fmt::Display for EmptyProposal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EmptyProposal")
    }
}

impl Proposal for EmptyProposal {
    fn reservations(&mut self) -> Vec<&mut dyn ProposedReservation> {
        Vec::new()
    }
}

pub fn accept(proposal: &mut dyn Proposal) -> Result<(), AcceptError> {
    for reservation in proposal.reservations() {
        reservation.accept()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {

    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;

    #[derive(Default)]
    struct Ledger {
        accepted: Vec<u32>,
    }

    struct MockTentative {
        id: u32,
        refuse: bool,
        ledger: Rc<RefCell<Ledger>>,
    }

    impl fmt::Display for MockTentative {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "tentative#{}", self.id)
        }
    }

    impl ProposedReservation for MockTentative {
        fn accept(&mut self) -> Result<(), AcceptError> {
            if self.refuse {
                return Err(AcceptError::ActualConflict { detail: format!("tentative#{} refused", self.id) });
            }
            self.ledger.borrow_mut().accepted.push(self.id);
            Ok(())
        }
    }

    struct MockProposal {
        tentatives: Vec<MockTentative>,
    }

    impl fmt::Display for MockProposal {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "MockProposal({} tentatives)", self.tentatives.len())
        }
    }

    impl Proposal for MockProposal {
        fn reservations(&mut self) -> Vec<&mut dyn ProposedReservation> {
            self.tentatives.iter_mut().map(|t| t as &mut dyn ProposedReservation).collect()
        }
    }

    fn proposal(ids: &[(u32, bool)], ledger: &Rc<RefCell<Ledger>>) -> MockProposal {
        MockProposal {
            tentatives: ids.iter().map(|&(id, refuse)| MockTentative { id, refuse, ledger: ledger.clone() }).collect(),
        }
    }

    #[test]
    fn empty_proposal_accepts() {
        accept(&mut EmptyProposal).expect("empty accepts");
    }

    #[test]
    fn accepts_all_in_order() {
        let ledger = Rc::new(RefCell::new(Ledger::default()));
        let mut p = proposal(&[(1, false), (2, false), (3, false)], &ledger);
        accept(&mut p).expect("all accept");
        assert_eq!(ledger.borrow().accepted, vec![1, 2, 3]);
    }

    #[test]
    fn refusal_stops_the_walk() {
        let ledger = Rc::new(RefCell::new(Ledger::default()));
        let mut p = proposal(&[(1, false), (2, true), (3, false)], &ledger);
        let err = match accept(&mut p) {
            Ok(()) => panic!("tentative#2 must refuse"),
            Err(err) => err,
        };
        assert!(matches!(err, AcceptError::ActualConflict { .. }));
        assert_eq!(ledger.borrow().accepted, vec![1], "the walk stops at the refusal");
    }
}
