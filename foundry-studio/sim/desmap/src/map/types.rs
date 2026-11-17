//! Data types used by WarehouseMap: fleet configuration and grid bounds.

use serde::{Deserialize, Serialize};

/// Fleet configuration: how many robots of each type to deploy.
///
/// For relay systems, specify `htms`. There is no VTM parameter: the VTM
/// fleet is derived from the map — each relay zone deploys its
/// [`crate::Zone::vtm_count`] (see `WarehouseMap::vtm_count`).
/// For RTP systems, specify `rtps`. All fields default to 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FleetConfig {
    /// Number of HTMs (Horizontal Transfer Modules). Relay systems only.
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub htms: usize,
    /// Number of RTPs (Racks-to-Picker robots). RTP systems only.
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub rtps: usize,
}

fn is_zero_usize(v: &usize) -> bool {
    *v == 0
}

impl FleetConfig {
    /// Default fleet for a relay system: 4 HTMs.
    pub fn default_relay() -> Self {
        Self { htms: 4, rtps: 0 }
    }

    /// Default fleet for an RTP system: 4 RTPs.
    pub fn default_rtp() -> Self {
        Self { htms: 0, rtps: 4 }
    }
}

/// Grid bounding rectangle for parametric maps.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GridBounds {
    /// Number of columns (X axis).
    pub width: u32,
    /// Number of rows (Y axis).
    pub depth: u32,
    /// Physical cell dimension in millimeters.
    pub cell_size_mm: f64,
}
