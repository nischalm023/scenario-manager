//! Optional parameters owned by root tasks. Omitted fields inherit legacy
//! scenario settings while those settings migrate to tasks and physical things.

use serde::{Deserialize, Serialize};

use crate::config::PickSequencing;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InventoryTaskConfig {
    pub order_trays: u64,
    /// Fraction of each SKU preloaded, rounded up; the rest is inbound demand.
    /// The even distribution also spreads the preload over only this fraction
    /// of the inventory totes, leaving the others empty at simtime 0.
    pub preload_fraction: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PickTaskConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embargo_secs: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequencing: Option<PickSequencing>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pace_to_station: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue_factor: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_inflight_picks: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SupplyTaskConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direct_bias_secs: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InductionTaskConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_presentations_per_hour: Option<f64>,
    pub units_per_tote: Option<u32>,
    pub sequence: InductionSequence,
    /// Synthetic order-priority arrivals may become eligible this far before
    /// the corresponding order release. None makes all inbound stock eligible at T0.
    pub lead_time_secs: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InductionSequence {
    #[default]
    SkuDemand,
    OrderRelease,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CloseOutTaskConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buffer_dwell_secs: Option<f64>,
}
