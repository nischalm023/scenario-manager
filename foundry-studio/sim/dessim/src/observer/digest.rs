//! Event-stream digest — the identity instrument for transformation PRs.
//!
//! The task-model-v2 program (`specs/task-model-v2/PLAN.md`) lands
//! behavior-preserving refactors gated on "the simulation's observable
//! behavior did not change" (gate G4). The event stream consumed by the
//! observer IS that observable behavior — every metric, slice, and export
//! derives from it — so the digest is computed here, on the observer
//! thread, in arrival order (single mpsc consumer: arrival order equals
//! emission order), touching nothing on the measured solver path.
//!
//! Three tiers, deliberately split by cost:
//!
//! - **Counts** (always on): total events + per-variant counts. One match
//!   and one increment per event — free at any event volume, and enough
//!   to catch most behavior drift (a lost or duplicated emission).
//! - **Hash** (`DES_DIGEST=1`): FNV-1a 64 over each event's `Debug`
//!   rendering, chained in arrival order. Sensitive to every payload
//!   field — waypoint chains, timing, ids — but pays a per-event format
//!   cost, so it is opt-in for A/B gate runs and off for ordinary runs.
//! - **Multiset** (`DES_DIGEST_SORTED=1`): the mechanism-cut verifier
//!   (task-model-v2 spec, verification ruling: cuts that move event
//!   emission points compare sim-time-sorted event streams, byte
//!   identity being unattainable by construction). Implemented as an
//!   order-independent accumulator — each event's rendering is hashed
//!   on its own, finalized, and summed — because two streams have equal
//!   multisets exactly when their sorted forms are equal (the sort key
//!   would be the full rendering, which embeds sim_time and kind), and
//!   the accumulator needs O(1) memory where a literal sort would buffer
//!   every rendered event of a multi-hour run. Summing (not XOR) keeps
//!   duplicated emissions visible. Gated separately from the chained
//!   hash so identity-cut `[gate]` lines stay byte-identical with
//!   binaries that predate this tier.
//!
//! FNV-1a and the splitmix64 finalizer are written out inline (no
//! dependency) so the pinned baseline in `perf/phase2-baseline/` can
//! never be invalidated by a hasher crate version bump.
//!
//! Two runs are compared by their `[gate]` digest lines byte-for-byte.
//! Identical hash ⇒ identical event stream (modulo 64-bit collision);
//! identical counts with differing hash ⇒ payload or ordering drift;
//! identical multiset with differing hash ⇒ pure emission-order drift —
//! the signature a faithful mechanism cut must show.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::events::SimEvent;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a 64 over one byte slice, from a fresh offset. The chained tier
/// inlines the same fold across events; this standalone form hashes one
/// event for the multiset tier.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET;
    for byte in bytes {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME);
    }
    hash
}

/// splitmix64 finalizer. Raw FNV values of near-identical renderings
/// differ mostly in low bits; wrapping-summed unmixed they would cancel
/// structure. The finalizer spreads each event hash over the full 64
/// bits so the sum behaves like a sum of independent uniform values.
fn mix(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// Accumulator owned by the observer; one `record` per event.
pub struct EventDigest {
    hash_enabled: bool,
    multiset_enabled: bool,
    hash: u64,
    multiset: u64,
    count: u64,
    by_kind: BTreeMap<&'static str, u64>,
}

impl EventDigest {
    /// Hash tier enabled by `DES_DIGEST=1`, multiset tier by
    /// `DES_DIGEST_SORTED=1` (any non-empty value each).
    pub fn from_env() -> Self {
        let non_empty = |var: &str| std::env::var_os(var).is_some_and(|v| !v.is_empty());
        Self::new(non_empty("DES_DIGEST"), non_empty("DES_DIGEST_SORTED"))
    }

    pub fn new(hash_enabled: bool, multiset_enabled: bool) -> Self {
        Self { hash_enabled, multiset_enabled, hash: FNV_OFFSET, multiset: 0, count: 0, by_kind: BTreeMap::new() }
    }

    pub fn record(&mut self, event: &SimEvent) {
        self.count += 1;
        *self.by_kind.entry(kind_name(event)).or_insert(0) += 1;
        if self.hash_enabled || self.multiset_enabled {
            let rendered = format!("{event:?}");
            if self.hash_enabled {
                for byte in rendered.as_bytes() {
                    self.hash = (self.hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME);
                }
            }
            if self.multiset_enabled {
                self.multiset = self.multiset.wrapping_add(mix(fnv1a(rendered.as_bytes())));
            }
        }
    }

    pub fn summary(&self) -> EventDigestSummary {
        EventDigestSummary {
            count: self.count,
            hash: self.hash_enabled.then_some(self.hash),
            multiset: self.multiset_enabled.then_some(self.multiset),
            by_kind: self.by_kind.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        }
    }
}

/// Final digest carried on [`crate::observer::output::MetricsOutput`].
///
/// `hash` is `None` when the run did not enable the hash tier — a pinned
/// baseline line always comes from a `DES_DIGEST=1` run.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EventDigestSummary {
    pub count: u64,
    pub hash: Option<u64>,
    /// Order-independent multiset hash (the mechanism-cut comparator).
    /// `None` when the tier was off — including in every summary
    /// deserialized from a run that predates the tier.
    pub multiset: Option<u64>,
    pub by_kind: BTreeMap<String, u64>,
}

impl std::fmt::Display for EventDigestSummary {
    /// The stable one-line format the G4 gate compares byte-for-byte.
    /// BTreeMap keeps per-kind order lexicographic and run-independent.
    /// The multiset field appears only when its tier ran, so lines from
    /// `DES_DIGEST=1`-only runs stay byte-comparable across binaries
    /// that predate the tier.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "events={}", self.count)?;
        match self.hash {
            Some(h) => write!(f, " fnv1a={h:016x}")?,
            None => write!(f, " fnv1a=off")?,
        }
        if let Some(m) = self.multiset {
            write!(f, " multiset={m:016x}")?;
        }
        for (kind, n) in &self.by_kind {
            write!(f, " {kind}={n}")?;
        }
        Ok(())
    }
}

/// Variant name for per-kind counting, and for naming a fact in the
/// observer's errors. Names are part of the pinned baseline format —
/// renaming one invalidates `perf/phase2-baseline/`.
pub(super) fn kind_name(event: &SimEvent) -> &'static str {
    match event {
        SimEvent::Nav(_) => "Nav",
        SimEvent::VtmNav(_) => "VtmNav",
        SimEvent::CycleComplete(_) => "CycleComplete",
        SimEvent::VtmCycle(_) => "VtmCycle",
        SimEvent::VtmOperation(_) => "VtmOperation",
        SimEvent::Resource(_) => "Resource",
        SimEvent::InitialPositions(_) => "InitialPositions",
        SimEvent::OrderComplete(_) => "OrderComplete",
        SimEvent::OrderDispatch(_) => "OrderDispatch",
        SimEvent::HtmDispatch(_) => "HtmDispatch",
        SimEvent::PipelineSnapshot(_) => "PipelineSnapshot",
        SimEvent::RelayStateChange(_) => "RelayStateChange",
        SimEvent::ToteInitialStates(_) => "ToteInitialStates",
        SimEvent::ToteCreated(_) => "ToteCreated",
        SimEvent::ToteContentsChanged(_) => "ToteContentsChanged",
        SimEvent::StripePlan(_) => "StripePlan",
        SimEvent::ToteMoved(_) => "ToteMoved",
        SimEvent::ToteTransit(_) => "ToteTransit",
        SimEvent::ArrivalError(_) => "ArrivalError",
        SimEvent::WatchdogTimeout(_) => "WatchdogTimeout",
        SimEvent::TripStart(_) => "TripStart",
        SimEvent::Induction(_) => "Induction",
        SimEvent::VtmIdleReason(_) => "VtmIdleReason",
        SimEvent::InventoryDelta(_) => "InventoryDelta",
        SimEvent::InventoryMove(_) => "InventoryMove",
        SimEvent::HtmRelayPickup(_) => "HtmRelayPickup",
        SimEvent::HtmRelayDeposit(_) => "HtmRelayDeposit",
        SimEvent::DivingBoardSlotWait(_) => "DivingBoardSlotWait",
        SimEvent::HtmAssignment(_) => "HtmAssignment",
        SimEvent::HtmArrival(_) => "HtmArrival",
        SimEvent::DispatchRejection(_) => "DispatchRejection",
        SimEvent::BotPlanSnapshot(_) => "BotPlanSnapshot",
        SimEvent::BarrierAdvanced(_) => "BarrierAdvanced",
        SimEvent::CellHeld(_) => "CellHeld",
        SimEvent::CellReleased(_) => "CellReleased",
        SimEvent::OrderStationPhase(_) => "OrderStationPhase",
        SimEvent::PresentationReadiness(_) => "PresentationReadiness",
        SimEvent::InventoryVisitPhase(_) => "InventoryVisitPhase",
    }
}

#[cfg(test)]
mod tests {
    use desgraph::RelayId;

    use super::*;
    use crate::events::{HtmRelayPickupEvent, OrderCompleteEvent};
    use crate::model::ids::RobotId;
    use crate::model::{OrderId, SimDuration, SimTime};

    fn order_event(id: u64, at: u64) -> SimEvent {
        SimEvent::OrderComplete(OrderCompleteEvent { order_id: OrderId(id as u32), sim_time_ms: at })
    }

    #[test]
    fn identical_streams_identical_digests() {
        let events = vec![order_event(1, 10), order_event(2, 20)];
        let mut a = EventDigest::new(true, true);
        let mut b = EventDigest::new(true, true);
        for e in &events {
            a.record(e);
            b.record(e);
        }
        assert_eq!(a.summary(), b.summary());
        assert!(a.summary().hash.is_some());
        assert!(a.summary().multiset.is_some());
    }

    #[test]
    fn payload_and_order_changes_change_hash() {
        let mut base = EventDigest::new(true, false);
        base.record(&order_event(1, 10));
        base.record(&order_event(2, 20));

        // Same counts, different payload.
        let mut payload = EventDigest::new(true, false);
        payload.record(&order_event(1, 10));
        payload.record(&order_event(2, 21));
        assert_eq!(base.summary().count, payload.summary().count);
        assert_ne!(base.summary().hash, payload.summary().hash);

        // Same events, different order.
        let mut swapped = EventDigest::new(true, false);
        swapped.record(&order_event(2, 20));
        swapped.record(&order_event(1, 10));
        assert_ne!(base.summary().hash, swapped.summary().hash);
    }

    /// The mechanism-cut property: reordering emissions changes the chained
    /// hash and nothing else — the multiset hash must hold.
    #[test]
    fn multiset_ignores_order_and_sees_payload() {
        let mut base = EventDigest::new(true, true);
        base.record(&order_event(1, 10));
        base.record(&order_event(2, 20));

        let mut swapped = EventDigest::new(true, true);
        swapped.record(&order_event(2, 20));
        swapped.record(&order_event(1, 10));
        assert_ne!(base.summary().hash, swapped.summary().hash);
        assert_eq!(base.summary().multiset, swapped.summary().multiset);

        let mut payload = EventDigest::new(true, true);
        payload.record(&order_event(1, 10));
        payload.record(&order_event(2, 21));
        assert_ne!(base.summary().multiset, payload.summary().multiset);
    }

    /// Wrapping-sum (not XOR) combine: a duplicated emission must not
    /// cancel out.
    #[test]
    fn multiset_sees_duplicated_emissions() {
        let mut once = EventDigest::new(false, true);
        once.record(&order_event(1, 10));

        let mut thrice = EventDigest::new(false, true);
        for _ in 0..3 {
            thrice.record(&order_event(1, 10));
        }
        assert_ne!(once.summary().multiset, thrice.summary().multiset);
    }

    #[test]
    fn display_is_stable_and_kind_sorted() {
        let mut d = EventDigest::new(false, false);
        d.record(&SimEvent::HtmRelayPickup(HtmRelayPickupEvent {
            robot_id: RobotId(1),
            relay_id: RelayId(2),
            start: SimTime::ms(5),
            end: SimTime::ms(7),
            wait: SimDuration::ZERO,
            resident: false,
        }));
        d.record(&order_event(1, 10));
        // HtmRelayPickup sorts before OrderComplete; hash tier off renders
        // as `off`; the multiset tier off leaves NO field — the byte-compat
        // guarantee for identity-cut lines.
        assert_eq!(d.summary().to_string(), "events=2 fnv1a=off HtmRelayPickup=1 OrderComplete=1");

        let mut m = EventDigest::new(false, true);
        m.record(&order_event(1, 10));
        let line = m.summary().to_string();
        assert!(
            line.starts_with("events=1 fnv1a=off multiset=") && line.ends_with(" OrderComplete=1"),
            "multiset field renders between fnv1a and per-kind counts: {line}"
        );
    }
}
