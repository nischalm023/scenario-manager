//! Resource contention event types.

/// Resource contention event.
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceEvent {
    pub kind: ResourceEventKind,
    /// Simulation time (ms) when event occurred.
    pub sim_time_ms: u64,
}

/// Kinds of resource contention events.
#[derive(Debug, Clone, PartialEq)]
pub enum ResourceEventKind {
    RelayReservationSuccess,
    RelayReservationFailure,
    ToteClaimSuccess,
    ToteClaimFailure,
    BackpressureTrigger,
    PathfinderQuery,
    PathfinderFailure,
    /// A VTM was matched to forward work (rack-to-relay).
    VtmScoreForwardSelected,
    /// A VTM was matched to return work (relay-to-rack).
    VtmScoreReturnSelected,
    /// A VTM was assigned work outside its zone.
    VtmCrossZoneAssignment,
    /// Timing of a VTM scoring pass (parallel matching).
    VtmScoringDuration {
        duration_ns: u64,
    },
    /// Zone relay utilization snapshot.
    ZonePressureSnapshot {
        zone_id: u32,
        pressure: f64,
    },
    /// Anti-starvation age bonus was applied during scoring.
    StarvationAgeBonusApplied,
    /// Deadlock safety headroom override fired.
    DeadlockHeadroomTriggered,
    /// VTM completed a forward cycle (rack→relay).
    VtmForwardCompleted {
        cycle_ms: u64,
    },
    /// VTM completed a return cycle (relay→rack).
    VtmReturnCompleted {
        cycle_ms: u64,
    },
    /// HTM completed a pickup cycle (relay→station→picks).
    HtmPickupCompleted {
        cycle_ms: u64,
    },
    /// HTM completed a return cycle (→return relay).
    HtmReturnCompleted {
        cycle_distance_mm: f64,
        cycle_ms: u64,
    },
    /// HTM operator wait completed.
    HtmOperatorWaitCompleted {
        wait_ms: u64,
    },
    /// HTM return relay retry (no relay available).
    HtmReturnRelayRetry,
    /// HTM queued waiting for operator.
    OperatorQueueWait,
}
