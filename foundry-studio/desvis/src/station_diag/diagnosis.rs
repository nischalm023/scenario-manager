//! Station idle diagnosis: "why is this station idle (or serving)?" as-of the
//! cursor. Split from `mod.rs` — a descendant module, so it reads the store's
//! private `station_cycles` vec directly.

use desgraph::StationId;
use dessim::events::DispatchRejectionReason;
use dessim::model::{SimDuration, SimTime};

use super::StationDiagStore;

/// The inspector's answer to "why is this station idle (or not)?", as-of the
/// cursor. Precedence is resolved by [`StationDiagStore::station_idle_diagnosis`]:
/// serving beats inbound beats rejection-dominated beats no-signal.
pub enum StationDiagnosis {
    /// A cycle span covers the cursor — the station is actively serving, not
    /// idle. Carries when the covering cycle started.
    Serving { since: SimTime },
    /// Idle but with HTMs en-route: waiting on `count` inbound HTMs; `next_eta`
    /// is the soonest quoted arrival relative to the cursor. `overdue` marks
    /// the "HTM not in time" case (quoted arrival already passed).
    WaitingInbound { count: usize, next_eta: SimDuration, overdue: bool },
    /// Idle, dominated by a recent dispatch-rejection reason. Carries the
    /// plain-language cause, the dominant reason, its count, and last occurrence.
    Rejected { cause: &'static str, reason: DispatchRejectionReason, count: u64, last_at: SimTime },
    /// Idle with no dispatch signal in the window — cause unrecorded. Never a
    /// guess: the sim simply emitted nothing attributable this window. Carries
    /// the last cycle completion, if any.
    IdleUnrecorded { since: Option<SimTime> },
}

impl StationDiagStore {
    /// A station cycle covering `cursor` (dwell span `[start, start+dwell)`
    /// contains it), and separately the most recent cycle completion (dwell
    /// end) at or before `cursor`. Returns `(covering_start, last_completion)`.
    fn station_cycle_state(&self, station: StationId, cursor: SimTime) -> (Option<SimTime>, Option<SimTime>) {
        let mut covering = None;
        let mut last_completion = None;
        for record in &self.station_cycles {
            if record.start > cursor {
                break;
            }
            if record.station_id != station {
                continue;
            }
            let end = record.start + record.dwell;
            if record.start <= cursor && cursor < end {
                covering = Some(record.start);
            }
            if end <= cursor {
                last_completion = Some(end);
            }
        }
        (covering, last_completion)
    }

    /// Diagnose why `station` is idle (or confirm it's serving), as-of
    /// `cursor`. Precedence: Serving > WaitingInbound > Rejected > Unrecorded.
    /// `rejection_window` bounds the "recent" rejection scan.
    pub fn station_idle_diagnosis(
        &self,
        station: StationId,
        cursor: SimTime,
        rejection_window: SimDuration,
    ) -> StationDiagnosis {
        // 1. Serving — a cycle span covers the cursor.
        let (covering, last_completion) = self.station_cycle_state(station, cursor);
        if let Some(since) = covering {
            return StationDiagnosis::Serving { since };
        }

        // 2. Inbound en-route — assigned, not yet arrived.
        let inbound = self.inbound_htms_at(station, cursor);
        if let Some(next) = inbound.iter().min_by_key(|htm| htm.quoted_arrival) {
            let (eta, overdue) = if next.quoted_arrival >= cursor {
                (next.quoted_arrival - cursor, false)
            } else {
                (cursor - next.quoted_arrival, true)
            };
            return StationDiagnosis::WaitingInbound { count: inbound.len(), next_eta: eta, overdue };
        }

        // 3. Rejection-dominated — the highest-count recent reason.
        let rejections = self.rejections_in_window(station, cursor, rejection_window);
        if let Some(dominant) = rejections.into_iter().max_by_key(|summary| summary.count) {
            return StationDiagnosis::Rejected {
                cause: rejection_cause(dominant.reason),
                reason: dominant.reason,
                count: dominant.count,
                last_at: dominant.last_at,
            };
        }

        // 4. No signal in window — idle, cause unrecorded.
        StationDiagnosis::IdleUnrecorded { since: last_completion }
    }
}

/// Plain-language cause for a dominant rejection reason, for the station
/// inspector's "Status:" line. Groups the fetch-capacity reasons together —
/// the operator's question is "supply side vs demand side vs admission", not
/// the precise internal reservation stage.
fn rejection_cause(reason: DispatchRejectionReason) -> &'static str {
    match reason {
        DispatchRejectionReason::NoReadyOrders => "demand-starved (no ready orders)",
        DispatchRejectionReason::NoOpenLines => "waiting on in-flight lines",
        DispatchRejectionReason::OrderInventoryUnavailable => "inventory unavailable for open SKUs",
        DispatchRejectionReason::OrderAdmissionBlocked => "admission blocked (pipeline full at reservation time)",
        DispatchRejectionReason::ConwipCapSaturated => "admission cap (WIP saturated)",
        DispatchRejectionReason::CounterRace => "admission cap (WIP saturated)",
        DispatchRejectionReason::RelayNoCandidates
        | DispatchRejectionReason::RelayHeadroomExhausted
        | DispatchRejectionReason::AccessCellConflict
        | DispatchRejectionReason::NoProposal => "fetch capacity (relay / HTM side)",
        DispatchRejectionReason::TopupNoDispatch => "top-up started but nothing fetchable",
    }
}
