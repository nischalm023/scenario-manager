//! Readiness-ordered presentation intentions consume no physical capacity.
//! Only the selected approach becomes a slot claim. A pending earlier source
//! keeps priority even when a later source would need an earlier departure.

use std::sync::Arc;

use desgraph::StationId;
use rustc_hash::FxHashSet;

use super::{Conveyance, ConveyanceError, ConveyedTote, Inner, PresentationClaim, Shared, TotePhase, same};
use crate::model::{SimDuration, SimTime};
use crate::signals::Version;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PresentationApproach {
    pub station: StationId,
    pub travel: SimDuration,
    /// None means that the corresponding target position has no known
    /// departure yet. Source-only callers use Some(SimTime::ZERO).
    pub available_at: Option<SimTime>,
}

pub(super) struct IntentData {
    load: u64,
    inventory_ready: SimTime,
    order_ready: SimTime,
    approaches: Vec<PresentationApproach>,
}

#[derive(Clone)]
pub struct PresentationIntent(Arc<IntentHandle>);

struct IntentHandle {
    shared: Arc<Shared>,
    id: u64,
    tote: ConveyedTote,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PresentationQuote {
    pub station: StationId,
    pub inventory_ready: SimTime,
    pub depart_at: SimTime,
}

impl Conveyance {
    pub fn request_presentation(
        &self,
        tote: &ConveyedTote,
        inventory_ready: SimTime,
        order_ready: SimTime,
        approaches: Vec<PresentationApproach>,
    ) -> Result<PresentationIntent, ConveyanceError> {
        same(&self.shared, &tote.handle.shared)?;
        let mut inner = self.shared.inner.lock().unwrap();
        validate_approaches(&inner, &approaches)?;
        let load = &inner.loads[&tote.handle.id];
        let earliest = match load.visit.phase {
            TotePhase::AwaitingPresentation => load.visit.boarded_at.unwrap(),
            TotePhase::Returning { ready_at } if load.collection.is_none() => ready_at,
            _ => return Err(ConveyanceError::WrongState),
        };
        if inner.intents.values().any(|intent| intent.load == tote.handle.id) {
            return Err(ConveyanceError::WrongState);
        }
        let id = inner.id();
        inner.intents.insert(
            id,
            IntentData {
                load: tote.handle.id,
                inventory_ready: inventory_ready.max(earliest),
                order_ready,
                approaches,
            },
        );
        drop(inner);
        self.version().bump();
        Ok(PresentationIntent(Arc::new(IntentHandle { shared: self.shared.clone(), id, tote: tote.clone() })))
    }
}

fn validate_approaches(inner: &Inner, approaches: &[PresentationApproach]) -> Result<(), ConveyanceError> {
    if approaches.is_empty() {
        return Err(ConveyanceError::InvalidEquipment);
    }
    let mut stations = FxHashSet::default();
    for approach in approaches {
        if !inner.slots.contains_key(&approach.station) {
            return Err(ConveyanceError::UnknownPresentation(approach.station));
        }
        if !stations.insert(approach.station) {
            return Err(ConveyanceError::InvalidEquipment);
        }
    }
    Ok(())
}

impl PresentationIntent {
    pub fn version(&self) -> &Version {
        &self.0.shared.version
    }

    /// Refresh physical approach availability without changing the visit's
    /// source readiness or its place in the pending selection order.
    pub fn update_approaches(&self, approaches: Vec<PresentationApproach>) -> Result<(), ConveyanceError> {
        let mut inner = self.0.shared.inner.lock().unwrap();
        validate_approaches(&inner, &approaches)?;
        let intent = inner.intents.get_mut(&self.0.id).ok_or(ConveyanceError::SpentClaim)?;
        if intent.approaches == approaches {
            return Ok(());
        }
        intent.approaches = approaches;
        drop(inner);
        self.version().bump();
        Ok(())
    }

    /// Unselected callers wait for a version change; they do not publish an
    /// inadmissible early proposal that would prevent the clock advancing.
    pub fn quote(&self, now: SimTime) -> Option<PresentationQuote> {
        selected_quote(&self.0.shared.inner.lock().unwrap(), self.0.id, now)
    }

    pub fn accept(&self, quote: PresentationQuote, now: SimTime) -> Result<PresentationClaim, ConveyanceError> {
        let mut inner = self.0.shared.inner.lock().unwrap();
        let expected = PresentationQuote { depart_at: quote.depart_at.max(now), ..quote };
        if selected_quote(&inner, self.0.id, now) != Some(expected) {
            return Err(ConveyanceError::WrongState);
        }
        let claim = inner.reserve_presentation(&self.0.shared, &self.0.tote, quote.station, quote.inventory_ready);
        inner.intents.remove(&self.0.id);
        drop(inner);
        self.version().bump();
        Ok(claim)
    }
}

fn selected_quote(inner: &Inner, wanted: u64, now: SimTime) -> Option<PresentationQuote> {
    let mut intents: Vec<_> = inner.intents.iter().collect();
    intents.sort_unstable_by_key(|&(id, intent)| (intent.inventory_ready, intent.load, *id));
    let mut assigned = FxHashSet::default();
    for (&id, intent) in intents {
        let load = &inner.loads[&intent.load];
        if load.collection.is_some()
            || !matches!(load.visit.phase, TotePhase::AwaitingPresentation | TotePhase::Returning { .. })
        {
            continue;
        }
        let mut best = None;
        for approach in &intent.approaches {
            if assigned.contains(&approach.station) {
                continue;
            }
            let Some(order_available) = approach.available_at else { continue };
            let Some(source_at) = inner.slots[&approach.station].open_after(1, intent.inventory_ready.max(now)) else {
                continue;
            };
            let depart_at =
                source_at.max(order_available).saturating_sub(approach.travel).max(intent.order_ready).max(now);
            let Some(target_at) = depart_at.checked_add(approach.travel) else { continue };
            let source_at = source_at.max(depart_at);
            let rank = source_at.max(target_at);
            if best.as_ref().is_none_or(|&(previous, _)| rank < previous) {
                best = Some((
                    rank,
                    PresentationQuote { station: approach.station, inventory_ready: source_at, depart_at },
                ));
            }
        }
        if let Some((_, quote)) = best {
            if id == wanted {
                return Some(quote);
            }
            assigned.insert(quote.station);
        }
    }
    None
}

impl Drop for IntentHandle {
    fn drop(&mut self) {
        let removed = self.shared.inner.lock().unwrap().intents.remove(&self.id).is_some();
        if removed {
            self.shared.version.bump();
        }
    }
}

#[cfg(test)]
#[path = "intent_tests.rs"]
mod tests;
