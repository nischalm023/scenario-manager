//! Within dessim, `sim_spec` is the RUN SPECIFICATION: the single JSON
//! document a run is launched from — warehouse design, scenario, and
//! simulation settings together — plus the `--settings key=value` overlay a
//! CLI applies on top of it. Overlay application is startup input handling,
//! so an unknown key or an unparseable value is refused as a
//! [`SettingsError`] before the clock starts, never quietly dropped. Besides
//! the listed keys, the overlay takes any dotted field path of the run config,
//! so a new figure needs no key of its own here. A value can parse and still
//! be one the simulation cannot use, such as a negative time or a speed of
//! zero; [`SimConfig::validate`] refuses those when the run is built, whether
//! the spec or the overlay stated them. A key that states a robot figure is
//! refused outright for a world whose robot entities give that fleet's
//! figures, and [`apply_cli_settings`] takes the world's figures so that no
//! caller can lay the overlay on without that check.

use std::any::type_name;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use desmap::{FleetConfig, WarehouseDesign};
use serde::{Deserialize, Deserializer};
use serde_json::{Number, Value};
use thiserror::Error;

use crate::composition::Composition;
use crate::config::VtmIsolationMode;
use crate::robots::RobotCatalog;
use crate::scenario::{BulkSourceScenario, CsvScenario};
use crate::{ScenarioConfig, SimConfig};

/// Default time-slice width in milliseconds (10 seconds).
pub const SLICE_WIDTH_MS: u64 = 10_000;

/// Every accepted `--settings` key, listed back to the operator when one
/// is misspelled.
const KNOWN_SETTINGS: &str = "composition, htm_only, queue_factor, nav_factor, num_htms, pick_time, no_watchdog, wall_timeout, fidelity, putaway, putaway.enabled, barrier.enabled, pick.embargo_secs, pick.sequencing, pick.pace_to_station, pick.delivery_max_wait_secs, order.max_inflight_picks_override, order.release_all_at_start, order.direct_extraction, order.extraction_secs, putaway.queue_depth, putaway.direct_bias_secs, putaway.forbid_rack_after_secs, putaway.ignore_hour_gate, order.close_out, putaway.max_presentations_per_hour, putaway.random_slots, vtm.isolation, vtm.ideal_cycle_secs, vtm.pick_secs, vtm.drop_secs, vtm.alignment_secs, vtm.per_zone, relay.htm_eligible_at_relay_start, max_orders, motion_model, htm.pickup_secs, htm.deposit_secs, htm.loaded_speed, htm.empty_speed, htm.loaded_constant_speed, htm.empty_constant_speed, htm.loaded_accel, htm.empty_accel, htm.loaded_rotate, htm.empty_rotate, orders_csv, items_csv, bulk.min_pallet_units, bulk.max_pallet_units";

/// Combined warehouse design, scenario config, and simulation settings.
///
/// Parsed from a single JSON file. `WarehouseDesign` is flattened so the
/// top-level keys merge. `scenario` and `settings` default when absent.
pub struct SimSpec {
    pub warehouse: WarehouseDesign,
    pub scenario: ScenarioConfig,
    pub settings: SimConfig,
}

impl<'de> Deserialize<'de> for SimSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        struct Document {
            #[serde(flatten)]
            warehouse: WarehouseDesign,
            #[serde(default)]
            scenario: ScenarioConfig,
            #[serde(default)]
            settings: SimConfig,
            #[serde(default)]
            composition: Option<Composition>,
        }
        let document = Document::deserialize(deserializer)?;
        let mut settings = document.settings;
        if let Some(composition) = document.composition {
            settings.composition = composition;
        }
        Ok(Self { warehouse: document.warehouse, scenario: document.scenario, settings })
    }
}

/// Why a `--settings key=value` override was refused. Every variant names
/// the key and the offending text, because the operator's next move is
/// editing that one argument.
#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("--settings '{0}': expected key=value")]
    NotKeyValue(String),
    #[error("--settings {key}: '{value}' is not a valid {expected}")]
    BadValue { key: String, value: String, expected: String },
    #[error("--settings {key}: {detail}")]
    Rejected { key: String, detail: String },
    #[error("--settings: unknown key '{key}'. Known: {}, and any field path of the run config", KNOWN_SETTINGS)]
    UnknownKey { key: String },
    #[error("--settings {key} applies only to {scenario} scenario configs")]
    WrongScenario { key: String, scenario: &'static str },
    #[error(
        "--settings {key}: a robot figure, and the world document {world} gives that fleet's robot figures \
         from its robot entities; change the robot entity there instead"
    )]
    RobotFigure { key: String, world: PathBuf },
}

fn parse<T: FromStr>(key: &str, value: &str) -> Result<T, SettingsError> {
    value.parse().map_err(|_| SettingsError::BadValue {
        key: key.to_string(),
        value: value.to_string(),
        expected: type_name::<T>().to_string(),
    })
}

/// For value types whose own `FromStr` message lists the accepted spellings.
fn parse_named<T: FromStr<Err = String>>(key: &str, value: &str) -> Result<T, SettingsError> {
    value.parse().map_err(|detail| SettingsError::Rejected { key: key.to_string(), detail })
}

/// For keys where one reserved word (`none`, `auto`) means "leave unset".
fn optional<T: FromStr>(key: &str, value: &str, unset: &str) -> Result<Option<T>, SettingsError> {
    if value.eq_ignore_ascii_case(unset) { Ok(None) } else { Ok(Some(parse(key, value)?)) }
}

fn csv_mut<'a>(scenario: &'a mut ScenarioConfig, key: &str) -> Result<&'a mut CsvScenario, SettingsError> {
    match scenario {
        ScenarioConfig::Csv(csv) => Ok(csv),
        _ => Err(SettingsError::WrongScenario { key: key.to_string(), scenario: "Csv" }),
    }
}

fn bulk_mut<'a>(scenario: &'a mut ScenarioConfig, key: &str) -> Result<&'a mut BulkSourceScenario, SettingsError> {
    match scenario {
        ScenarioConfig::BulkSource(bulk) => Ok(bulk),
        _ => Err(SettingsError::WrongScenario { key: key.to_string(), scenario: "BulkSource" }),
    }
}

/// Apply a design's `fleet { ... }` block to the SimConfig.
///
/// When a design carries an explicit fleet block it is authoritative over
/// the legacy `settings.num_htms`/`num_rtps` duplicates. Call BEFORE
/// [`apply_cli_settings`] so CLI overrides (`--settings num_htms=N`) still
/// win. VTMs have no fleet parameter — the VTM fleet is map-derived
/// (`WarehouseMap::vtm_count`).
pub fn apply_fleet(config: &mut SimConfig, fleet: Option<&FleetConfig>) {
    if let Some(fleet) = fleet {
        config.num_htms = fleet.htms;
        config.num_rtps = fleet.rtps;
    }
}

/// Apply CLI `--settings key=value` overrides to SimConfig.
///
/// `world` is the figures the run's world gives its fleets, from the document
/// at `world_path`. A key that states one of them is refused before any
/// override lands: the world's figure governs, and an override that ran anyway
/// would record a robot the world does not describe. Scenario keys
/// (`orders_csv`, `items_csv`, `bulk.*`) are accepted and ignored here;
/// [`apply_scenario_settings`] is what applies them. Any other key is read as
/// a field path of the run config, and one the config does not have is a
/// [`SettingsError::UnknownKey`] — a misspelled override that ran anyway would
/// produce a plausible-looking result for the wrong configuration.
pub fn apply_cli_settings(
    config: &mut SimConfig,
    cli_settings: &[String],
    world: &RobotCatalog,
    world_path: &Path,
) -> Result<(), SettingsError> {
    let mut keys = cli_settings.iter().filter_map(|setting| setting.split_once('=').map(|(key, _)| key));
    if let Some(key) = keys.find(|key| world.states(key)) {
        return Err(SettingsError::RobotFigure { key: key.to_string(), world: world_path.to_path_buf() });
    }
    for setting in cli_settings {
        let (key, value) = setting.split_once('=').ok_or_else(|| SettingsError::NotKeyValue(setting.clone()))?;
        let flag = value != "0" && value != "false";
        match key {
            "composition" => {
                let composition = Composition::Named(value.into());
                composition
                    .resolve()
                    .map_err(|e| SettingsError::Rejected { key: key.into(), detail: e.to_string() })?;
                config.composition = composition;
            }
            "htm_only" => config.htm_only = flag,
            "queue_factor" => config.queue_factor = parse(key, value)?,
            "nav_factor" => config.nav_factor = parse(key, value)?,
            "num_htms" => config.num_htms = parse(key, value)?,
            "pick_time" => config.operator.time_per_pick_secs = parse(key, value)?,
            "no_watchdog" => config.no_watchdog = flag,
            "wall_timeout" => config.wall_timeout_secs = Some(parse(key, value)?),
            "fidelity" => config.fidelity = parse_named(key, value)?,
            "putaway" | "putaway.enabled" => config.putaway.enabled = flag,
            "barrier.enabled" => config.barrier.enabled = flag,
            "pick.embargo_secs" => config.pick.embargo_secs = parse(key, value)?,
            "pick.sequencing" => config.pick.sequencing = parse_named(key, value)?,
            "pick.pace_to_station" => config.pick.pace_to_station = flag,
            "pick.delivery_max_wait_secs" => config.pick.delivery_max_wait_secs = optional(key, value, "none")?,
            "order.max_inflight_picks_override" => {
                config.order.max_inflight_picks_override = optional(key, value, "auto")?
            }
            "putaway.queue_depth" => config.putaway.queue_depth = Some(parse(key, value)?),
            "putaway.direct_bias_secs" => config.putaway.direct_bias_secs = optional(key, value, "auto")?,
            "putaway.forbid_rack_after_secs" => config.putaway.forbid_rack_after_secs = optional(key, value, "never")?,
            "putaway.ignore_hour_gate" => config.putaway.ignore_hour_gate = flag,
            // Pick-only: `order.close_out=false` ends an order at the station
            // that picked it -- no order tote is fetched, loaded, buffered or
            // extracted. Isolates pick traffic from order-bot traffic.
            "order.close_out" => config.order.close_out = flag,
            "order.release_all_at_start" => config.order.release_all_at_start = flag,
            "order.direct_extraction" => config.order.direct_extraction = flag,
            "order.extraction_secs" => config.order.extraction_secs = parse(key, value)?,
            "putaway.random_slots" => config.putaway.random_slots = flag,
            "putaway.max_presentations_per_hour" => config.putaway.max_presentations_per_hour = parse(key, value)?,
            // `vtm.approximation` is a deprecated alias for `vtm.isolation`.
            "vtm.isolation" | "vtm.approximation" => {
                config.vtm.isolation = match value {
                    "off" => VtmIsolationMode::Off,
                    "ideal_rate" => VtmIsolationMode::IdealRate,
                    "teleport" => VtmIsolationMode::Teleport,
                    other => {
                        let detail = format!("unknown mode '{other}'. Known: off, ideal_rate, teleport");
                        return Err(SettingsError::Rejected { key: key.to_string(), detail });
                    }
                }
            }
            "vtm.pick_secs" => config.vtm.pick_secs = parse(key, value)?,
            "vtm.drop_secs" => config.vtm.drop_secs = parse(key, value)?,
            "vtm.alignment_secs" => config.vtm.alignment_secs = parse(key, value)?,
            "vtm.ideal_cycle_secs" => config.vtm.ideal_cycle_secs = Some(parse(key, value)?),
            "vtm.per_zone" => {
                let n: usize = parse(key, value)?;
                if n == 0 {
                    let detail = "must be >= 1 (0 registers zero VTMs and deadlocks the run)".to_string();
                    return Err(SettingsError::Rejected { key: key.to_string(), detail });
                }
                config.vtm.per_zone_override = Some(n);
            }
            "relay.htm_eligible_at_relay_start" => config.relay.htm_eligible_at_relay_start = flag,
            "max_orders" => config.max_orders = Some(parse(key, value)?),
            "motion_model" => config.motion_model = parse_named(key, value)?,
            "htm.pickup_secs" => config.htm.pickup_secs = parse(key, value)?,
            "htm.deposit_secs" => config.htm.deposit_secs = parse(key, value)?,
            "htm.loaded_speed" => config.htm.loaded_speed = parse(key, value)?,
            "htm.empty_speed" => config.htm.empty_speed = parse(key, value)?,
            "htm.loaded_constant_speed" => config.htm.loaded_constant_speed = parse(key, value)?,
            "htm.empty_constant_speed" => config.htm.empty_constant_speed = parse(key, value)?,
            "htm.loaded_accel" => config.htm.loaded_accel = parse(key, value)?,
            "htm.empty_accel" => config.htm.empty_accel = parse(key, value)?,
            "htm.loaded_rotate" => config.htm.loaded_rotate = parse(key, value)?,
            "htm.empty_rotate" => config.htm.empty_rotate = parse(key, value)?,
            "orders_csv" | "items_csv" | "bulk.min_pallet_units" | "bulk.max_pallet_units" => {}
            _ => set_field_path(config, key, value)?,
        }
    }
    Ok(())
}

/// Set `key`, a dotted field path of the run config such as
/// `htm.loaded_rotate_180`, through the config's own serialized form. The path
/// must exist in that form, so a misspelling is still an unknown key. A field
/// reached this way skips the checks an explicit key above applies; the run's
/// own [`SimConfig::validate`] still judges it.
fn set_field_path(config: &mut SimConfig, key: &str, value: &str) -> Result<(), SettingsError> {
    let mut document = serde_json::to_value(&*config)
        .map_err(|error| SettingsError::Rejected { key: key.to_string(), detail: error.to_string() })?;
    let node = key
        .split('.')
        .try_fold(&mut document, |node, field| node.get_mut(field))
        .ok_or_else(|| SettingsError::UnknownKey { key: key.to_string() })?;
    *node = field_value(key, value, node)?;
    *config = serde_json::from_value(document).map_err(|error| SettingsError::BadValue {
        key: key.to_string(),
        value: value.to_string(),
        expected: format!("value for this field: {error}"),
    })?;
    Ok(())
}

/// `value` read as the kind of JSON the field already holds. An unset
/// `Option` holds `null`, which names no kind, so the value is read as the
/// first of an integer, a number, a bool and text that it parses as.
fn field_value(key: &str, value: &str, node: &Value) -> Result<Value, SettingsError> {
    let bad = |expected: &str| SettingsError::BadValue {
        key: key.to_string(),
        value: value.to_string(),
        expected: expected.to_string(),
    };
    let integer = || value.parse::<i64>().map(Value::from).or_else(|_| value.parse::<u64>().map(Value::from)).ok();
    // `str::parse` rounds correctly, which serde_json's own text parser need not.
    let number = || value.parse::<f64>().ok().and_then(Number::from_f64).map(Value::Number);
    match node {
        Value::Bool(_) => value.parse().map(Value::Bool).map_err(|_| bad("bool")),
        Value::Number(held) if held.is_f64() => number().ok_or_else(|| bad("number")),
        Value::Number(_) => integer().ok_or_else(|| bad("integer")),
        Value::Null => Ok(integer()
            .or_else(number)
            .or_else(|| value.parse().ok().map(Value::Bool))
            .unwrap_or_else(|| Value::from(value))),
        Value::String(_) => Ok(Value::from(value)),
        Value::Array(_) | Value::Object(_) => Err(bad("single figure; the key names a group of settings")),
    }
}

/// Apply CLI `--settings key=value` overrides to the scenario config.
///
/// Only the four scenario keys land here — `orders_csv` / `items_csv` on a
/// `Csv` scenario, `bulk.min_pallet_units` / `bulk.max_pallet_units` on a
/// `BulkSource` one; a key aimed at the wrong scenario variant is a
/// [`SettingsError::WrongScenario`]. Every other setting (and any malformed
/// one) is [`apply_cli_settings`]'s to accept or refuse.
pub fn apply_scenario_settings(scenario: &mut ScenarioConfig, cli_settings: &[String]) -> Result<(), SettingsError> {
    for setting in cli_settings {
        let Some((key, value)) = setting.split_once('=') else { continue };
        match key {
            "orders_csv" => csv_mut(scenario, key)?.orders_csv = value.to_string(),
            "items_csv" => csv_mut(scenario, key)?.items_csv = value.to_string(),
            "bulk.min_pallet_units" => bulk_mut(scenario, key)?.min_pallet_units = parse(key, value)?,
            "bulk.max_pallet_units" => bulk_mut(scenario, key)?.max_pallet_units = parse(key, value)?,
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use serde_json::{Value, json};
    use tempfile::tempdir;

    use super::{SettingsError, apply_cli_settings};
    use crate::SimConfig;
    use crate::robots::RobotCatalog;
    use crate::robots::aspects::COMPONENTS;
    use crate::runner::resolve_scenario_with_racking;
    use crate::world_spec;

    fn fixture(name: &str) -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    fn settings(overrides: &[&str]) -> Vec<String> {
        overrides.iter().map(|setting| setting.to_string()).collect()
    }

    /// The overlay for a run whose world gives no robot figures.
    fn overlay(config: &mut SimConfig, overrides: &[&str]) -> Result<(), SettingsError> {
        apply_cli_settings(config, &settings(overrides), &RobotCatalog::default(), Path::new("a world"))
    }

    /// A field path is set through the config's serialized form, so writing
    /// that form back must change nothing, including for the config the
    /// throughput spec runs on, robot figures and all.
    #[test]
    fn a_round_trip_through_the_serialized_config_changes_nothing() {
        let spec = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/walmart_dfw6_throughput_spec.json");
        let throughput = world_spec::load(&spec).expect("the throughput spec loads").settings;
        for config in [SimConfig::default(), throughput] {
            let before = serde_json::to_value(&config).unwrap();
            let after: SimConfig = serde_json::from_value(before.clone()).unwrap();
            assert_eq!(serde_json::to_value(&after).unwrap(), before);
        }
    }

    /// A key no arm names sets exactly the field its path names, read as the
    /// kind of value that field holds.
    #[test]
    fn a_field_path_sets_the_field_it_names_and_nothing_else() {
        let mut config = SimConfig::default();
        let overrides = [
            "htm.loaded_rotate_180=2.88",
            "htm.empty_rotate_180=2.5",
            "vtm.chassis_accel=0.5",
            "putaway.htm_wip_factor=1.5",
        ];
        overlay(&mut config, &overrides).expect("field paths of the run config");
        let mut expected = SimConfig::default();
        expected.htm.loaded_rotate_180 = Some(2.88);
        expected.htm.empty_rotate_180 = Some(2.5);
        expected.vtm.chassis_accel = 0.5;
        expected.putaway.htm_wip_factor = Some(1.5);
        assert_eq!(serde_json::to_value(&config).unwrap(), serde_json::to_value(&expected).unwrap());
    }

    #[test]
    fn a_field_path_the_run_config_lacks_is_an_unknown_key() {
        for key in ["htm.loaded_rotate_90=2.16", "htm.pickup_secs.fork=2", "robot.speed=1"] {
            let err = overlay(&mut SimConfig::default(), &[key]).unwrap_err();
            assert!(matches!(err, SettingsError::UnknownKey { .. }), "{key}: {err}");
        }
    }

    #[test]
    fn a_field_path_given_the_wrong_kind_of_value_is_refused() {
        for key in ["vtm.chassis_speed=fast", "num_threads=1.5", "htm.loaded_rotate_180=fast", "htm=1"] {
            let err = overlay(&mut SimConfig::default(), &[key]).unwrap_err();
            assert!(matches!(err, SettingsError::BadValue { .. }), "{key}: {err}");
        }
    }

    /// The minimal EasyLoop world, its robot entities stripped of every aspect
    /// when `aspects` is false, run from its flow spec with `edit` applied to
    /// the spec's JSON and `overrides` laid over it, loaded and built the way a
    /// run is.
    fn build_run(aspects: bool, edit: impl FnOnce(&mut Value), overrides: &[&str]) -> Result<(), String> {
        let dir = tempdir().unwrap();
        let mut world = fixture("minimal_easyloop.json");
        if !aspects {
            for entity in world["entities"].as_array_mut().unwrap() {
                let Some(components) = entity["components"].as_object_mut() else { continue };
                for name in COMPONENTS {
                    components.remove(name);
                }
            }
        }
        let world_path = dir.path().join("world.json");
        fs::write(&world_path, world.to_string()).unwrap();
        let mut spec = fixture("minimal_easyloop_flow_spec.json");
        spec["world"] = Value::from(world_path.to_string_lossy().into_owned());
        edit(&mut spec);
        let path = dir.path().join("spec.json");
        fs::write(&path, spec.to_string()).unwrap();
        let loaded = world_spec::load(&path).map_err(|error| error.to_string())?;
        let mut config = loaded.settings;
        apply_cli_settings(&mut config, &settings(overrides), &loaded.world.robot_catalog, &loaded.world_path)
            .map_err(|error| error.to_string())?;
        resolve_scenario_with_racking(loaded.world.map, loaded.world.racking, &loaded.scenario, &config).map(drop)
    }

    /// Once a fleet's robot entities carry aspects, its robot figures are the
    /// world's: a spec's `settings` block or `--settings` that states one is
    /// refused, under a serde alias too, and the refusal names the key and the
    /// world. A setting that is not a robot figure still passes.
    #[test]
    fn a_robot_figure_the_world_gives_is_refused_from_the_spec_and_from_the_command_line() {
        for (block, field) in [("vtm", "chassis_accel"), ("htm", "load_time_secs")] {
            let key = format!("{block}.{field}");
            let refused = build_run(true, |spec| spec["settings"][block][field] = json!(1.0), &[]).unwrap_err();
            assert!(refused.contains(&key) && refused.contains("world.json"), "{refused}");
        }
        let refused = build_run(true, |_| {}, &["htm.empty_rotate_180=2.88"]).unwrap_err();
        assert!(refused.contains("htm.empty_rotate_180") && refused.contains("world.json"), "{refused}");
        let vtm = json!({ "speed": 5.0, "load_time_secs": 3.0 });
        build_run(true, |spec| spec["settings"]["vtm"] = vtm, &["htm.loaded_constant_speed=5.0"])
            .expect("neither the grid speed nor a constant cruise is a robot figure");
    }

    /// A figure the simulation cannot use is refused when the run is built,
    /// whether the spec's `settings` block states it or `--settings` does, and
    /// the refusal names the key. The world's robot entities carry no aspects,
    /// so the robot figures here are the settings' own.
    #[test]
    fn an_unusable_figure_is_refused_from_the_spec_and_from_the_command_line() {
        let from_spec =
            build_run(false, |spec| spec["settings"]["vtm"] = json!({ "chassis_accel": 0 }), &[]).unwrap_err();
        assert!(from_spec.contains("vtm.chassis_accel"), "{from_spec}");
        let from_command_line = build_run(false, |_| {}, &["htm.empty_rotate_180=-1"]).unwrap_err();
        assert!(from_command_line.contains("htm.empty_rotate_180"), "{from_command_line}");
        let grid_speed = build_run(true, |spec| spec["settings"]["vtm"] = json!({ "speed": 0 }), &[]).unwrap_err();
        assert!(grid_speed.contains("vtm.speed"), "a setting is still judged beside the world's figures: {grid_speed}");
    }

    /// Both sources accept figures the simulation can use, down to a zero
    /// dwell where a dwell may be zero, for a world whose robot entities carry
    /// no aspects.
    #[test]
    fn usable_figures_pass_from_the_spec_and_from_the_command_line() {
        let vtm = json!({ "chassis_speed": 1.5, "chassis_accel": 0.5, "lift_speed": 1.5, "lift_accel": 0.5 });
        let overrides = [
            "vtm.pick_alignment_secs=3.56",
            "vtm.drop_alignment_secs=0",
            "htm.conveyor_pickup_secs=4.0",
            "htm.conveyor_deposit_secs=0",
            "htm.loaded_rotate_180=2.88",
            "htm.empty_rotate_180=2.88",
        ];
        build_run(false, |spec| spec["settings"]["vtm"] = vtm, &overrides)
            .expect("the simulation can use every figure here");
    }

    #[test]
    fn cli_settings_override_htm_chassis_kinematics() {
        // `--settings htm.*` overrides flow into config.htm for a run with no
        // world robot figures. Only the keys given change.
        let mut config = SimConfig::default();
        let overrides = [
            "htm.loaded_speed=4.2",
            "htm.empty_speed=4.0",
            "htm.loaded_constant_speed=3.0",
            "htm.loaded_accel=1.3",
            "htm.empty_accel=2.5",
        ];
        overlay(&mut config, &overrides).expect("well-formed htm.* overrides");
        assert_eq!(config.htm.loaded_speed, 4.2);
        assert_eq!(config.htm.empty_speed, 4.0);
        assert_eq!(config.htm.loaded_constant_speed, 3.0);
        assert_eq!(config.htm.loaded_accel, 1.3);
        assert_eq!(config.htm.empty_accel, 2.5);
    }

    #[test]
    fn bad_settings_are_refused_not_defaulted() {
        let mut config = SimConfig::default();
        let err = overlay(&mut config, &["num_htms=lots"]).unwrap_err();
        assert!(matches!(err, SettingsError::BadValue { .. }), "{err}");
        let err = overlay(&mut config, &["htm.loaded_speeed=4.2"]).unwrap_err();
        assert!(matches!(err, SettingsError::UnknownKey { .. }), "{err}");
        let err = overlay(&mut config, &["vtm.per_zone=0"]).unwrap_err();
        assert!(matches!(err, SettingsError::Rejected { .. }), "{err}");
        let err = overlay(&mut config, &["htm_only"]).unwrap_err();
        assert!(matches!(err, SettingsError::NotKeyValue(_)), "{err}");
    }
}
