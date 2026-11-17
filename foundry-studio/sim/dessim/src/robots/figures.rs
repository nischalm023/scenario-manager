//! Validated figures from one HTM catalog entity, with units made explicit
//! at the reader boundary. They construct that model's runtime drives and
//! handling; they are never reduced to one fleet-wide configuration.

use desgraph::RobotType;

use super::aspects::FloorDriveKind;
use crate::config::HtmConfig;

/// Every robot field of [`HtmConfig`], as one HTM robot entity gives them, in
/// the units `HtmConfig` holds: m/s, m/s² and seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct HtmFigures {
    /// The catalog's model; None preserves the legacy settings path.
    pub kind: Option<FloorDriveKind>,
    pub pickup_secs: f64,
    pub deposit_secs: f64,
    pub conveyor_pickup_secs: Option<f64>,
    pub conveyor_deposit_secs: Option<f64>,
    pub loaded_speed: f64,
    pub empty_speed: f64,
    pub loaded_constant_speed: Option<f64>,
    pub empty_constant_speed: Option<f64>,
    pub loaded_accel: f64,
    pub empty_accel: f64,
    pub loaded_rotate: f64,
    pub empty_rotate: f64,
    pub loaded_rotate_180: Option<f64>,
    pub empty_rotate_180: Option<f64>,
}

impl HtmFigures {
    /// Explicit adapter for settings-only and diagnostic models.
    pub fn from_legacy(htm: &HtmConfig) -> Self {
        Self {
            kind: None,
            pickup_secs: htm.pickup_secs,
            deposit_secs: htm.deposit_secs,
            conveyor_pickup_secs: htm.conveyor_pickup_secs,
            conveyor_deposit_secs: htm.conveyor_deposit_secs,
            loaded_speed: htm.loaded_speed,
            empty_speed: htm.empty_speed,
            loaded_constant_speed: Some(htm.loaded_constant_speed),
            empty_constant_speed: Some(htm.empty_constant_speed),
            loaded_accel: htm.loaded_accel,
            empty_accel: htm.empty_accel,
            loaded_rotate: htm.loaded_rotate,
            empty_rotate: htm.empty_rotate,
            loaded_rotate_180: htm.loaded_rotate_180,
            empty_rotate_180: htm.empty_rotate_180,
        }
    }
}

/// Legacy settings keys whose figures a catalog model supplies. Both the
/// spec reader and CLI override boundary consult this list.
const HTM_FIGURE_KEYS: [&str; 14] = [
    "pickup_secs",
    "load_time_secs",
    "deposit_secs",
    "unload_time_secs",
    "conveyor_pickup_secs",
    "conveyor_deposit_secs",
    "loaded_speed",
    "empty_speed",
    "loaded_accel",
    "empty_accel",
    "loaded_rotate",
    "empty_rotate",
    "loaded_rotate_180",
    "empty_rotate_180",
];
const VTM_FIGURE_KEYS: [&str; 9] = [
    "pick_secs",
    "drop_secs",
    "alignment_secs",
    "pick_alignment_secs",
    "drop_alignment_secs",
    "chassis_speed",
    "chassis_accel",
    "lift_speed",
    "lift_accel",
];

/// The fleet whose robot figure the dotted settings key `key` states, such as
/// `vtm.pick_secs`, or `None` for a key that is not a robot figure. The one
/// list both refusal sites read: `world_spec::load` for a spec's `settings`
/// block and `sim_spec::apply_cli_settings` for `--settings`.
pub fn fleet_of_figure_key(key: &str) -> Option<RobotType> {
    match key.split_once('.') {
        Some(("htm", field)) if HTM_FIGURE_KEYS.contains(&field) => Some(RobotType::Htm),
        Some(("vtm", field)) if VTM_FIGURE_KEYS.contains(&field) => Some(RobotType::Vtm),
        _ => None,
    }
}
