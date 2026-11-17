//! Within `types`, `metrics` is the LEGACY metric selector: the enumerated
//! `MetricKind` list driving the time-series line graph over the
//! string-keyed `SliceRecord` maps. This is the enumerated path the
//! descriptor contract replaces — it migrates to discovered metric streams
//! under issues #281/#300; the canonical metrics panel does not use it.

use eframe::egui;

/// How adjacent slices are aggregated into the smoothed line drawn on top of
/// the raw per-slice line. `Off` means render only the raw line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RollingKind {
    Off,
    Mean,
    Sum,
    TrailingSum,
    Min,
    Max,
    Median,
}

impl RollingKind {
    pub fn label(&self) -> &'static str {
        match self {
            RollingKind::Off => "raw",
            RollingKind::Mean => "mean",
            RollingKind::Sum => "sum",
            RollingKind::TrailingSum => "trailing sum",
            RollingKind::Min => "min",
            RollingKind::Max => "max",
            RollingKind::Median => "median",
        }
    }

    pub fn all() -> &'static [RollingKind] {
        &[
            RollingKind::Off,
            RollingKind::Mean,
            RollingKind::Sum,
            RollingKind::TrailingSum,
            RollingKind::Min,
            RollingKind::Max,
            RollingKind::Median,
        ]
    }

    pub fn smoothing_only() -> &'static [RollingKind] {
        &[
            RollingKind::Off,
            RollingKind::Mean,
            RollingKind::Sum,
            RollingKind::Min,
            RollingKind::Max,
            RollingKind::Median,
        ]
    }
}

/// Per-metric rolling-window configuration. `window` and `resolution` are in
/// source slices. Resolution is used only by the causal trailing-sum mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RollingConfig {
    pub kind: RollingKind,
    pub window: usize,
    pub resolution: usize,
}

impl RollingConfig {
    const fn smoothing(kind: RollingKind, window: usize) -> Self {
        Self { kind, window, resolution: 1 }
    }
}

impl Default for RollingConfig {
    fn default() -> Self {
        Self { kind: RollingKind::Off, window: 5, resolution: 1 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetricKind {
    PicksPerSlice,
    PicksPerHourRate,
    CumulativePicks,
    PutawaysPerSlice,
    CumulativePutaways,
    PickPresentationsPerSlice,
    CumulativePickPresentations,
    PutawayPresentationsPerSlice,
    CumulativePutawayPresentations,
    PutawayPresentationsDirectPerSlice,
    CumulativePutawayPresentationsDirect,
    PutawayPresentationsFromRackPerSlice,
    CumulativePutawayPresentationsFromRack,
    CumulativeOrders,
    OrdersPerHourRate,
    OrderExtractionsPerSlice,
    OrderExtractionsPerHourRate,
    ExtractedUnitsPerSlice,
    ExtractedUnitsPerHourRate,
    Presentations,
    CumulativePresentations,
    CumulativePresPerStnHr,
    VtmIdle,
    VtmTraveling,
    VtmLoadingUnloading,
    HtmIdle,
    HtmTraveling,
    HtmRotating,
    HtmProcessing,
    /// KPI #4 Wait Loaded — loaded & not presenting yet (inbound). Overlay of
    /// travel: the loaded waits on the to-station leg.
    HtmWaitLoaded,
    /// KPI #5 Dwell Loaded — loaded & blocked returning (outbound). Overlay of
    /// travel: the loaded waits on the to-return leg.
    HtmDwellLoaded,
    /// KPI #6 Pick-at-station — operator pick dwell at pick stations. Overlay
    /// of station visits.
    HtmPickAtStation,
    RelayReservationFailures,
    SimSpeed,
    SimSpeedAvg,
    WallClockPerSlice,
    // -- New operational metrics --
    VtmTravelDistance,
    HtmTravelDistance,
    VtmForwardTrips,
    VtmReturnTrips,
    HtmPickupTrips,
    HtmReturnTrips,
    VtmForwardCycleAvg,
    VtmReturnCycleAvg,
    HtmCycleAvg,
    HtmOperatorWaitAvg,
    HtmRoundTripDistanceAvg,
    HtmRoundTripTimeAvg,
    HtmRoundTripSpeed,
    HtmReturnRelayRetries,
    OperatorQueueWaits,
    // -- Metrics V2 --
    HtmToRelayDistance,
    HtmToStationDistance,
    HtmToReturnDistance,
    HtmToParkingDistance,
    RackOccupancy,
    CurrentInventory,
    PipelineDepthAvg,
    PickStationPicksRate,
    PickStationPresentations,
    PutawaysPerHourRate,
    PutawayItemsPerHourRate,
    PutawayTotesStoredPerHour,
    CumulativePallets,
    /// Total D30 time spent waiting at relays for inbound pickup
    /// (seconds per slice). Sum of per-pickup wait gaps.
    HtmWaitAtRelay,
    /// Average D30 wait per HtmPickup commit (seconds). Renders as
    /// `htm_wait_at_relay_ms / htm_relay_pickup_count` per slice.
    HtmWaitAtRelayAvg,
    /// Total time committed relay reservations were delayed by the diving-
    /// board slot still holding the previous tote (seconds per slice).
    DivingBoardSlotWait,
    /// Average diving-board slot wait per committed reservation (seconds).
    /// Renders as `diving_board_slot_wait_ms / diving_board_presentation_count`.
    DivingBoardSlotWaitAvg,
    /// Rack fill: rack slots holding a non-empty tote, as a percentage of
    /// total tote-slot capacity. Empty totes parked in slots don't count, so
    /// this tracks pick-vs-replenish balance rather than being pinned at
    /// ~100% on fully-seeded scenarios. Renders as
    /// `100 * rack_occupancy / rack_capacity` (both from the per-slice map;
    /// capacity 0 → 0%).
    RackFillPercent,
    /// Peak rack fill: the within-slice maximum of the fill above, as a
    /// percentage of capacity ("max fill level"). Renders as
    /// `100 * rack_occupancy_peak / rack_capacity`.
    RackMaxFillPercent,
    /// Lowest per-aisle fill % at each slice. Plotted with `RackFillMaxPercent`
    /// around `RackFillPercent`, the gap is the aisle imbalance (one aisle
    /// draining while others stay full). Reads `rack_fill_min_pct`.
    RackFillMinPercent,
    /// Highest per-aisle fill % at each slice (upper edge of the band).
    /// Reads `rack_fill_max_pct`.
    RackFillMaxPercent,
    /// Relay occupancy: relays holding a deposited tote awaiting HTM pickup,
    /// as a percentage of total relays. Instantaneous per-slice snapshot of
    /// how much of the relay buffer is tied up. Renders as
    /// `100 * relay_cells_reserved / relay_cells_total` (both from the
    /// per-slice map; 0 relays → 0%).
    RelayOccupancyPercent,
    /// Relay-access *reservation* utilization: the fraction of the slice the
    /// relay HTM-access cells were reserved in the planner ledger (HTM
    /// approach + tote dwell), as a percentage of total relays. The
    /// time-series decomposition of the `relay_utilization_pct` summary line
    /// — integrates to the same overall number. Distinct from
    /// `RelayOccupancyPercent`, which is only the physical tote-dwell window.
    /// Renders as `100 * relay_reserved_busy_ms / (relay_cells_total *
    /// slice_width_ms)`.
    RelayReservationUtilPercent,
}

impl MetricKind {
    pub fn label(&self) -> &'static str {
        match self {
            MetricKind::PicksPerSlice => "Picks",
            MetricKind::PicksPerHourRate => "Picks/hr (rate)",
            MetricKind::CumulativePicks => "Cumulative picks",
            MetricKind::PutawaysPerSlice => "Inducted units",
            MetricKind::CumulativePutaways => "Cumulative inducted units",
            MetricKind::PickPresentationsPerSlice => "Pick pres",
            MetricKind::CumulativePickPresentations => "Cumulative pick pres",
            MetricKind::PutawayPresentationsPerSlice => "Putaway pres",
            MetricKind::CumulativePutawayPresentations => "Cumulative putaway pres",
            MetricKind::PutawayPresentationsDirectPerSlice => "Putaway pres (direct)",
            MetricKind::CumulativePutawayPresentationsDirect => "Cumulative putaway pres (direct)",
            MetricKind::PutawayPresentationsFromRackPerSlice => "Putaway pres (from rack)",
            MetricKind::CumulativePutawayPresentationsFromRack => "Cumulative putaway pres (from rack)",
            MetricKind::CumulativeOrders => "Cumulative fully picked orders",
            MetricKind::OrdersPerHourRate => "Fully picked orders/hr",
            MetricKind::OrderExtractionsPerSlice => "Completed order extractions",
            MetricKind::OrderExtractionsPerHourRate => "Order extractions/hr",
            MetricKind::ExtractedUnitsPerSlice => "Extracted units",
            MetricKind::ExtractedUnitsPerHourRate => "Extracted units/hr",
            MetricKind::Presentations => "Presentations",
            MetricKind::CumulativePresentations => "Cumulative presentations",
            MetricKind::CumulativePresPerStnHr => "Pres/stn/hr",
            MetricKind::VtmIdle => "DA50 idle",
            MetricKind::VtmTraveling => "DA50 traveling",
            MetricKind::VtmLoadingUnloading => "DA50 dropoff/collection",
            MetricKind::HtmIdle => "D30 idle",
            MetricKind::HtmTraveling => "D30 traveling",
            MetricKind::HtmRotating => "D30 rotating",
            MetricKind::HtmProcessing => "D30 processing",
            MetricKind::HtmWaitLoaded => "D30 wait-loaded",
            MetricKind::HtmDwellLoaded => "D30 dwell-loaded",
            MetricKind::HtmPickAtStation => "D30 pick-at-stn",
            MetricKind::RelayReservationFailures => "Relay failures",
            MetricKind::SimSpeed => "Sim speed (slice)",
            MetricKind::SimSpeedAvg => "Sim speed (avg)",
            MetricKind::WallClockPerSlice => "Wall clock/slice (s)",
            MetricKind::VtmTravelDistance => "DA50 travel (m)",
            MetricKind::HtmTravelDistance => "D30 travel (m)",
            MetricKind::VtmForwardTrips => "DA50 fwd trips",
            MetricKind::VtmReturnTrips => "DA50 ret trips",
            MetricKind::HtmPickupTrips => "D30 pickup trips",
            MetricKind::HtmReturnTrips => "D30 ret trips",
            MetricKind::VtmForwardCycleAvg => "DA50 fwd cycle (s)",
            MetricKind::VtmReturnCycleAvg => "DA50 ret cycle (s)",
            MetricKind::HtmCycleAvg => "D30 cycle (s)",
            MetricKind::HtmOperatorWaitAvg => "D30 op-wait (s)",
            MetricKind::HtmRoundTripDistanceAvg => "D30 round trip (m)",
            MetricKind::HtmRoundTripTimeAvg => "D30 round trip (s)",
            MetricKind::HtmRoundTripSpeed => "D30 round trip (m/s)",
            MetricKind::HtmReturnRelayRetries => "D30 relay retries",
            MetricKind::OperatorQueueWaits => "Operator queue waits",
            MetricKind::HtmToRelayDistance => "D30 to relay (m)",
            MetricKind::HtmToStationDistance => "D30 to station (m)",
            MetricKind::HtmToReturnDistance => "D30 to return (m)",
            MetricKind::HtmToParkingDistance => "D30 to parking (m)",
            MetricKind::RackOccupancy => "Rack occupancy",
            MetricKind::CurrentInventory => "Inventory (units)",
            MetricKind::PipelineDepthAvg => "Pipeline depth (avg)",
            MetricKind::PickStationPicksRate => "Pick stn picks/hr",
            MetricKind::PickStationPresentations => "Pick stn pres",
            MetricKind::PutawaysPerHourRate => "Inducted totes/hr (rate)",
            MetricKind::PutawayItemsPerHourRate => "Inducted units/hr (rate)",
            MetricKind::PutawayTotesStoredPerHour => "Putaway totes stored/hr",
            MetricKind::CumulativePallets => "Cumulative pallets",
            MetricKind::HtmWaitAtRelay => "D30 tote-wait@relay (s)",
            MetricKind::HtmWaitAtRelayAvg => "D30 tote-wait@relay avg (s)",
            MetricKind::DivingBoardSlotWait => "Diving-board slot wait (s)",
            MetricKind::DivingBoardSlotWaitAvg => "Diving-board slot wait avg (s)",
            MetricKind::RackFillPercent => "Rack fill %",
            MetricKind::RackMaxFillPercent => "Rack max fill %",
            MetricKind::RackFillMinPercent => "Rack fill % (min aisle)",
            MetricKind::RackFillMaxPercent => "Rack fill % (max aisle)",
            MetricKind::RelayOccupancyPercent => "Relay occupancy %",
            MetricKind::RelayReservationUtilPercent => "Relay reservation util %",
        }
    }

    pub fn color(&self) -> egui::Color32 {
        match self {
            MetricKind::PicksPerSlice => egui::Color32::from_rgb(46, 139, 87),
            MetricKind::PicksPerHourRate => egui::Color32::from_rgb(0, 128, 255),
            MetricKind::CumulativePicks => egui::Color32::from_rgb(34, 139, 34),
            MetricKind::PutawaysPerSlice => egui::Color32::from_rgb(255, 140, 0),
            MetricKind::CumulativePutaways => egui::Color32::from_rgb(210, 110, 20),
            MetricKind::PickPresentationsPerSlice => egui::Color32::from_rgb(60, 170, 110),
            MetricKind::CumulativePickPresentations => egui::Color32::from_rgb(40, 130, 80),
            MetricKind::PutawayPresentationsPerSlice => egui::Color32::from_rgb(245, 160, 60),
            MetricKind::CumulativePutawayPresentations => egui::Color32::from_rgb(200, 125, 40),
            // Direct (chained, no rack round-trip) — green tint: the efficient path.
            MetricKind::PutawayPresentationsDirectPerSlice => egui::Color32::from_rgb(120, 200, 100),
            MetricKind::CumulativePutawayPresentationsDirect => egui::Color32::from_rgb(90, 160, 75),
            // From-rack (VTM-staged at relay) — blue tint: the rack-involved path.
            MetricKind::PutawayPresentationsFromRackPerSlice => egui::Color32::from_rgb(100, 150, 220),
            MetricKind::CumulativePutawayPresentationsFromRack => egui::Color32::from_rgb(75, 115, 180),
            MetricKind::CumulativeOrders => egui::Color32::from_rgb(255, 140, 0),
            MetricKind::OrdersPerHourRate => egui::Color32::from_rgb(255, 195, 80),
            MetricKind::OrderExtractionsPerSlice => egui::Color32::from_rgb(160, 85, 200),
            MetricKind::OrderExtractionsPerHourRate => egui::Color32::from_rgb(125, 55, 175),
            MetricKind::ExtractedUnitsPerSlice => egui::Color32::from_rgb(30, 165, 160),
            MetricKind::ExtractedUnitsPerHourRate => egui::Color32::from_rgb(15, 120, 115),
            MetricKind::Presentations => egui::Color32::from_rgb(148, 103, 189),
            MetricKind::CumulativePresentations => egui::Color32::from_rgb(160, 32, 240),
            MetricKind::CumulativePresPerStnHr => egui::Color32::from_rgb(186, 85, 211),
            MetricKind::VtmIdle => egui::Color32::from_rgb(100, 149, 237),
            MetricKind::VtmTraveling => egui::Color32::from_rgb(30, 80, 200),
            MetricKind::VtmLoadingUnloading => egui::Color32::from_rgb(70, 130, 180),
            MetricKind::HtmIdle => egui::Color32::from_rgb(255, 99, 71),
            MetricKind::HtmTraveling => egui::Color32::from_rgb(220, 20, 60),
            MetricKind::HtmRotating => egui::Color32::from_rgb(255, 105, 97),
            MetricKind::HtmProcessing => egui::Color32::from_rgb(178, 34, 34),
            MetricKind::HtmWaitLoaded => egui::Color32::from_rgb(230, 90, 30),
            MetricKind::HtmDwellLoaded => egui::Color32::from_rgb(160, 40, 60),
            MetricKind::HtmPickAtStation => egui::Color32::from_rgb(110, 20, 20),
            MetricKind::RelayReservationFailures => egui::Color32::from_rgb(128, 0, 128),
            MetricKind::SimSpeed => egui::Color32::from_rgb(255, 165, 0),
            MetricKind::SimSpeedAvg => egui::Color32::from_rgb(200, 115, 0),
            MetricKind::WallClockPerSlice => egui::Color32::from_rgb(169, 169, 169),
            MetricKind::VtmTravelDistance => egui::Color32::from_rgb(65, 105, 225),
            MetricKind::HtmTravelDistance => egui::Color32::from_rgb(205, 92, 92),
            MetricKind::VtmForwardTrips => egui::Color32::from_rgb(0, 100, 200),
            MetricKind::VtmReturnTrips => egui::Color32::from_rgb(100, 160, 220),
            MetricKind::HtmPickupTrips => egui::Color32::from_rgb(200, 60, 60),
            MetricKind::HtmReturnTrips => egui::Color32::from_rgb(220, 120, 120),
            MetricKind::VtmForwardCycleAvg => egui::Color32::from_rgb(0, 80, 160),
            MetricKind::VtmReturnCycleAvg => egui::Color32::from_rgb(80, 140, 200),
            MetricKind::HtmCycleAvg => egui::Color32::from_rgb(180, 40, 40),
            MetricKind::HtmOperatorWaitAvg => egui::Color32::from_rgb(255, 100, 50),
            MetricKind::HtmRoundTripDistanceAvg => egui::Color32::from_rgb(230, 80, 130),
            MetricKind::HtmRoundTripTimeAvg => egui::Color32::from_rgb(200, 50, 100),
            MetricKind::HtmRoundTripSpeed => egui::Color32::from_rgb(170, 60, 90),
            MetricKind::HtmReturnRelayRetries => egui::Color32::from_rgb(200, 150, 0),
            MetricKind::OperatorQueueWaits => egui::Color32::from_rgb(180, 120, 60),
            MetricKind::HtmToRelayDistance => egui::Color32::from_rgb(255, 160, 122),
            MetricKind::HtmToStationDistance => egui::Color32::from_rgb(210, 105, 30),
            MetricKind::HtmToReturnDistance => egui::Color32::from_rgb(178, 102, 178),
            MetricKind::HtmToParkingDistance => egui::Color32::from_rgb(144, 238, 144),
            MetricKind::RackOccupancy => egui::Color32::from_rgb(70, 130, 70),
            MetricKind::CurrentInventory => egui::Color32::from_rgb(120, 180, 90),
            MetricKind::PipelineDepthAvg => egui::Color32::from_rgb(100, 100, 200),
            MetricKind::PickStationPicksRate => egui::Color32::from_rgb(0, 180, 120),
            MetricKind::PickStationPresentations => egui::Color32::from_rgb(50, 200, 150),
            MetricKind::PutawaysPerHourRate => egui::Color32::from_rgb(255, 165, 0),
            MetricKind::PutawayItemsPerHourRate => egui::Color32::from_rgb(230, 140, 30),
            MetricKind::PutawayTotesStoredPerHour => egui::Color32::from_rgb(65, 160, 110),
            MetricKind::CumulativePallets => egui::Color32::from_rgb(180, 100, 255),
            MetricKind::HtmWaitAtRelay => egui::Color32::from_rgb(200, 80, 130),
            MetricKind::HtmWaitAtRelayAvg => egui::Color32::from_rgb(160, 60, 100),
            MetricKind::DivingBoardSlotWait => egui::Color32::from_rgb(120, 160, 220),
            MetricKind::DivingBoardSlotWaitAvg => egui::Color32::from_rgb(80, 120, 180),
            MetricKind::RackFillPercent => egui::Color32::from_rgb(95, 160, 95),
            MetricKind::RackMaxFillPercent => egui::Color32::from_rgb(55, 125, 55),
            MetricKind::RackFillMinPercent => egui::Color32::from_rgb(200, 130, 60),
            MetricKind::RackFillMaxPercent => egui::Color32::from_rgb(120, 190, 120),
            MetricKind::RelayOccupancyPercent => egui::Color32::from_rgb(160, 90, 200),
            MetricKind::RelayReservationUtilPercent => egui::Color32::from_rgb(120, 60, 190),
        }
    }

    /// CLI-friendly kebab-case name for this metric.
    pub fn cli_name(&self) -> &'static str {
        match self {
            MetricKind::PicksPerSlice => "picks-per-slice",
            MetricKind::PicksPerHourRate => "picks-per-hour",
            MetricKind::CumulativePicks => "cumulative-picks",
            MetricKind::PutawaysPerSlice => "putaways-per-slice",
            MetricKind::CumulativePutaways => "cumulative-putaways",
            MetricKind::PickPresentationsPerSlice => "pick-pres-per-slice",
            MetricKind::CumulativePickPresentations => "cumulative-pick-pres",
            MetricKind::PutawayPresentationsPerSlice => "putaway-pres-per-slice",
            MetricKind::CumulativePutawayPresentations => "cumulative-putaway-pres",
            MetricKind::PutawayPresentationsDirectPerSlice => "putaway-pres-direct-per-slice",
            MetricKind::CumulativePutawayPresentationsDirect => "cumulative-putaway-pres-direct",
            MetricKind::PutawayPresentationsFromRackPerSlice => "putaway-pres-from-rack-per-slice",
            MetricKind::CumulativePutawayPresentationsFromRack => "cumulative-putaway-pres-from-rack",
            MetricKind::CumulativeOrders => "cumulative-orders",
            MetricKind::OrdersPerHourRate => "orders-per-hour",
            MetricKind::OrderExtractionsPerSlice => "order-extractions-per-slice",
            MetricKind::OrderExtractionsPerHourRate => "order-extractions-per-hour",
            MetricKind::ExtractedUnitsPerSlice => "extracted-units-per-slice",
            MetricKind::ExtractedUnitsPerHourRate => "extracted-units-per-hour",
            MetricKind::Presentations => "presentations",
            MetricKind::CumulativePresentations => "cumulative-presentations",
            MetricKind::CumulativePresPerStnHr => "avg-pres-per-stn-hr",
            MetricKind::VtmIdle => "vtm-idle",
            MetricKind::VtmTraveling => "vtm-traveling",
            MetricKind::VtmLoadingUnloading => "vtm-load-unload",
            MetricKind::HtmIdle => "htm-idle",
            MetricKind::HtmTraveling => "htm-traveling",
            MetricKind::HtmRotating => "htm-rotating",
            MetricKind::HtmProcessing => "htm-processing",
            MetricKind::HtmWaitLoaded => "htm-wait-loaded",
            MetricKind::HtmDwellLoaded => "htm-dwell-loaded",
            MetricKind::HtmPickAtStation => "htm-pick-at-station",
            MetricKind::RelayReservationFailures => "relay-failures",
            MetricKind::SimSpeed => "sim-speed",
            MetricKind::SimSpeedAvg => "sim-speed-avg",
            MetricKind::WallClockPerSlice => "wall-clock",
            MetricKind::VtmTravelDistance => "vtm-travel",
            MetricKind::HtmTravelDistance => "htm-travel",
            MetricKind::VtmForwardTrips => "vtm-fwd-trips",
            MetricKind::VtmReturnTrips => "vtm-ret-trips",
            MetricKind::HtmPickupTrips => "htm-pickup-trips",
            MetricKind::HtmReturnTrips => "htm-ret-trips",
            MetricKind::VtmForwardCycleAvg => "vtm-fwd-cycle",
            MetricKind::VtmReturnCycleAvg => "vtm-ret-cycle",
            MetricKind::HtmCycleAvg => "htm-cycle",
            MetricKind::HtmOperatorWaitAvg => "htm-op-wait",
            MetricKind::HtmRoundTripDistanceAvg => "htm-round-trip-dist",
            MetricKind::HtmRoundTripTimeAvg => "htm-round-trip-time",
            MetricKind::HtmRoundTripSpeed => "htm-round-trip-speed",
            MetricKind::HtmReturnRelayRetries => "htm-relay-retries",
            MetricKind::OperatorQueueWaits => "op-queue-waits",
            MetricKind::HtmToRelayDistance => "htm-to-relay",
            MetricKind::HtmToStationDistance => "htm-to-station",
            MetricKind::HtmToReturnDistance => "htm-to-return",
            MetricKind::HtmToParkingDistance => "htm-to-parking",
            MetricKind::RackOccupancy => "rack-occupancy",
            MetricKind::CurrentInventory => "current-inventory",
            MetricKind::PipelineDepthAvg => "pipeline-depth",
            MetricKind::PickStationPicksRate => "pick-stn-rate",
            MetricKind::PickStationPresentations => "pick-stn-pres",
            MetricKind::PutawaysPerHourRate => "putaways-per-hour",
            MetricKind::PutawayItemsPerHourRate => "putaway-items-per-hour",
            MetricKind::PutawayTotesStoredPerHour => "putaway-totes-stored-per-hour",
            MetricKind::CumulativePallets => "cumulative-pallets",
            MetricKind::HtmWaitAtRelay => "htm-wait-at-relay",
            MetricKind::HtmWaitAtRelayAvg => "htm-wait-at-relay-avg",
            MetricKind::DivingBoardSlotWait => "diving-board-slot-wait",
            MetricKind::DivingBoardSlotWaitAvg => "diving-board-slot-wait-avg",
            MetricKind::RackFillPercent => "rack-fill-pct",
            MetricKind::RackMaxFillPercent => "rack-max-fill-pct",
            MetricKind::RackFillMinPercent => "rack-fill-min-pct",
            MetricKind::RackFillMaxPercent => "rack-fill-max-pct",
            MetricKind::RelayOccupancyPercent => "relay-occupancy-pct",
            MetricKind::RelayReservationUtilPercent => "relay-reservation-util-pct",
        }
    }

    /// Parse a CLI-friendly name into a MetricKind.
    pub fn from_cli_name(name: &str) -> Option<MetricKind> {
        Self::all().iter().find(|k| k.cli_name() == name).copied()
    }

    /// Default rolling config applied at app init. Override per-metric where
    /// a non-`Off` smoothing makes the chart easier to read at a glance.
    pub fn default_rolling(&self) -> RollingConfig {
        match self {
            MetricKind::PicksPerSlice => RollingConfig::default(),
            MetricKind::PicksPerHourRate => RollingConfig::smoothing(RollingKind::Mean, 25),
            MetricKind::CumulativePicks => RollingConfig::default(),
            MetricKind::PutawaysPerSlice => RollingConfig::default(),
            MetricKind::CumulativePutaways => RollingConfig::default(),
            MetricKind::PickPresentationsPerSlice => RollingConfig::default(),
            MetricKind::CumulativePickPresentations => RollingConfig::default(),
            MetricKind::PutawayPresentationsPerSlice => RollingConfig::default(),
            MetricKind::CumulativePutawayPresentations => RollingConfig::default(),
            MetricKind::PutawayPresentationsDirectPerSlice => RollingConfig::default(),
            MetricKind::CumulativePutawayPresentationsDirect => RollingConfig::default(),
            MetricKind::PutawayPresentationsFromRackPerSlice => RollingConfig::default(),
            MetricKind::CumulativePutawayPresentationsFromRack => RollingConfig::default(),
            MetricKind::CumulativeOrders => RollingConfig::default(),
            MetricKind::OrdersPerHourRate => RollingConfig::smoothing(RollingKind::Mean, 25),
            MetricKind::OrderExtractionsPerSlice | MetricKind::ExtractedUnitsPerSlice => RollingConfig::default(),
            MetricKind::OrderExtractionsPerHourRate | MetricKind::ExtractedUnitsPerHourRate => {
                RollingConfig::smoothing(RollingKind::Mean, 25)
            }
            MetricKind::Presentations => RollingConfig::smoothing(RollingKind::Mean, 25),
            MetricKind::CumulativePresentations => RollingConfig::default(),
            MetricKind::CumulativePresPerStnHr => RollingConfig::default(),
            MetricKind::VtmIdle => RollingConfig::default(),
            MetricKind::VtmTraveling => RollingConfig::default(),
            MetricKind::VtmLoadingUnloading => RollingConfig::default(),
            MetricKind::HtmIdle => RollingConfig::default(),
            MetricKind::HtmTraveling => RollingConfig::default(),
            MetricKind::HtmRotating => RollingConfig::default(),
            MetricKind::HtmProcessing => RollingConfig::default(),
            MetricKind::HtmWaitLoaded => RollingConfig::default(),
            MetricKind::HtmDwellLoaded => RollingConfig::default(),
            MetricKind::HtmPickAtStation => RollingConfig::default(),
            MetricKind::RelayReservationFailures => RollingConfig::default(),
            MetricKind::SimSpeed => RollingConfig::smoothing(RollingKind::Mean, 25),
            MetricKind::SimSpeedAvg => RollingConfig::default(),
            MetricKind::WallClockPerSlice => RollingConfig::default(),
            MetricKind::VtmTravelDistance => RollingConfig::default(),
            MetricKind::HtmTravelDistance => RollingConfig::default(),
            MetricKind::VtmForwardTrips => RollingConfig::default(),
            MetricKind::VtmReturnTrips => RollingConfig::default(),
            MetricKind::HtmPickupTrips => RollingConfig::default(),
            MetricKind::HtmReturnTrips => RollingConfig::default(),
            MetricKind::VtmForwardCycleAvg => RollingConfig::default(),
            MetricKind::VtmReturnCycleAvg => RollingConfig::default(),
            MetricKind::HtmCycleAvg => RollingConfig::default(),
            MetricKind::HtmOperatorWaitAvg => RollingConfig::default(),
            MetricKind::HtmRoundTripDistanceAvg => RollingConfig::default(),
            MetricKind::HtmRoundTripTimeAvg => RollingConfig::default(),
            MetricKind::HtmRoundTripSpeed => RollingConfig::smoothing(RollingKind::Mean, 25),
            MetricKind::HtmReturnRelayRetries => RollingConfig::default(),
            MetricKind::OperatorQueueWaits => RollingConfig::default(),
            MetricKind::HtmToRelayDistance => RollingConfig::default(),
            MetricKind::HtmToStationDistance => RollingConfig::default(),
            MetricKind::HtmToReturnDistance => RollingConfig::default(),
            MetricKind::HtmToParkingDistance => RollingConfig::default(),
            MetricKind::RackOccupancy => RollingConfig::default(),
            MetricKind::CurrentInventory => RollingConfig::default(),
            MetricKind::PipelineDepthAvg => RollingConfig::default(),
            MetricKind::PickStationPicksRate => RollingConfig::smoothing(RollingKind::Mean, 25),
            MetricKind::PickStationPresentations => RollingConfig::default(),
            MetricKind::PutawaysPerHourRate => RollingConfig::smoothing(RollingKind::Mean, 25),
            MetricKind::PutawayItemsPerHourRate => RollingConfig::smoothing(RollingKind::Mean, 25),
            MetricKind::PutawayTotesStoredPerHour => RollingConfig::smoothing(RollingKind::Mean, 25),
            MetricKind::CumulativePallets => RollingConfig::default(),
            MetricKind::HtmWaitAtRelay => RollingConfig::default(),
            MetricKind::HtmWaitAtRelayAvg => RollingConfig::smoothing(RollingKind::Mean, 25),
            MetricKind::DivingBoardSlotWait => RollingConfig::default(),
            MetricKind::DivingBoardSlotWaitAvg => RollingConfig::smoothing(RollingKind::Mean, 25),
            MetricKind::RackFillPercent => RollingConfig::default(),
            MetricKind::RackMaxFillPercent => RollingConfig::default(),
            MetricKind::RackFillMinPercent => RollingConfig::default(),
            MetricKind::RackFillMaxPercent => RollingConfig::default(),
            MetricKind::RelayOccupancyPercent => RollingConfig::smoothing(RollingKind::Mean, 25),
            MetricKind::RelayReservationUtilPercent => RollingConfig::smoothing(RollingKind::Mean, 25),
        }
    }

    pub fn all() -> &'static [MetricKind] {
        &[
            MetricKind::PicksPerSlice,
            MetricKind::PicksPerHourRate,
            MetricKind::CumulativePicks,
            MetricKind::PutawaysPerSlice,
            MetricKind::CumulativePutaways,
            MetricKind::PickPresentationsPerSlice,
            MetricKind::CumulativePickPresentations,
            MetricKind::PutawayPresentationsPerSlice,
            MetricKind::CumulativePutawayPresentations,
            MetricKind::PutawayPresentationsDirectPerSlice,
            MetricKind::CumulativePutawayPresentationsDirect,
            MetricKind::PutawayPresentationsFromRackPerSlice,
            MetricKind::CumulativePutawayPresentationsFromRack,
            MetricKind::CumulativeOrders,
            MetricKind::OrdersPerHourRate,
            MetricKind::OrderExtractionsPerSlice,
            MetricKind::OrderExtractionsPerHourRate,
            MetricKind::ExtractedUnitsPerSlice,
            MetricKind::ExtractedUnitsPerHourRate,
            MetricKind::Presentations,
            MetricKind::CumulativePresentations,
            MetricKind::CumulativePresPerStnHr,
            MetricKind::VtmIdle,
            MetricKind::VtmTraveling,
            MetricKind::VtmLoadingUnloading,
            MetricKind::HtmIdle,
            MetricKind::HtmTraveling,
            MetricKind::HtmRotating,
            MetricKind::HtmProcessing,
            MetricKind::HtmWaitLoaded,
            MetricKind::HtmDwellLoaded,
            MetricKind::HtmPickAtStation,
            MetricKind::RelayReservationFailures,
            MetricKind::SimSpeed,
            MetricKind::SimSpeedAvg,
            MetricKind::WallClockPerSlice,
            MetricKind::VtmTravelDistance,
            MetricKind::HtmTravelDistance,
            MetricKind::VtmForwardTrips,
            MetricKind::VtmReturnTrips,
            MetricKind::HtmPickupTrips,
            MetricKind::HtmReturnTrips,
            MetricKind::VtmForwardCycleAvg,
            MetricKind::VtmReturnCycleAvg,
            MetricKind::HtmCycleAvg,
            MetricKind::HtmOperatorWaitAvg,
            MetricKind::HtmRoundTripDistanceAvg,
            MetricKind::HtmRoundTripTimeAvg,
            MetricKind::HtmRoundTripSpeed,
            MetricKind::HtmReturnRelayRetries,
            MetricKind::OperatorQueueWaits,
            MetricKind::HtmToRelayDistance,
            MetricKind::HtmToStationDistance,
            MetricKind::HtmToReturnDistance,
            MetricKind::HtmToParkingDistance,
            MetricKind::RackOccupancy,
            MetricKind::CurrentInventory,
            MetricKind::PipelineDepthAvg,
            MetricKind::PickStationPicksRate,
            MetricKind::PickStationPresentations,
            MetricKind::PutawaysPerHourRate,
            MetricKind::PutawayItemsPerHourRate,
            MetricKind::PutawayTotesStoredPerHour,
            MetricKind::CumulativePallets,
            MetricKind::HtmWaitAtRelay,
            MetricKind::HtmWaitAtRelayAvg,
            MetricKind::DivingBoardSlotWait,
            MetricKind::DivingBoardSlotWaitAvg,
            MetricKind::RackFillPercent,
            MetricKind::RackMaxFillPercent,
            MetricKind::RackFillMinPercent,
            MetricKind::RackFillMaxPercent,
            MetricKind::RelayOccupancyPercent,
            MetricKind::RelayReservationUtilPercent,
        ]
    }

    /// Headline metrics shown in the sidebar Charts list by default.
    /// Everything else lives behind the "+ Add chart" popup so the
    /// sidebar doesn't become a 40-checkbox scroll. Display order in
    /// the sidebar is alphabetical (sorted at render time) — the order
    /// here is irrelevant.
    pub fn featured() -> &'static [MetricKind] {
        &[
            MetricKind::PicksPerHourRate,
            MetricKind::PutawaysPerHourRate,
            MetricKind::PutawayTotesStoredPerHour,
            MetricKind::OrdersPerHourRate,
            MetricKind::OrderExtractionsPerHourRate,
            MetricKind::CumulativePicks,
            MetricKind::CumulativePutaways,
            MetricKind::Presentations,
            MetricKind::CurrentInventory,
            MetricKind::RelayReservationUtilPercent,
            MetricKind::SimSpeedAvg,
        ]
    }
}
