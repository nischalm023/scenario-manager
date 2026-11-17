//! Reads each robot entity independently into its own validated motion and
//! handling. Floor and rack-ladder chassis remain distinct; refusals name the
//! source entity and component rather than substituting run-wide figures.

use serde_json::Value;
use thiserror::Error;
use site_model::LoadError;
use site_model::entity::Entity;
use site_model::ids::EntityId;

use super::aspects::{Aspect, Drive, EndEffector, FloorDrive, FloorDriveKind, Lift, Profile, RackLadder};
use super::figures::HtmFigures;
use super::vtm::model::{AxisMotion, FloorVtmMotionModel, HandlingTimes, LadderMotionModel, VtmChassis, VtmMotion};
use crate::worldmap::WorldMap;

/// A robot entity whose aspects cannot become a usable model. The world
/// file is named by whoever loaded it.
#[derive(Debug, Error)]
#[error("robot entity {entity}{}: {refusal}", named(.name))]
pub struct RobotError {
    pub entity: EntityId,
    pub name: Option<String>,
    pub refusal: Refusal,
}

#[derive(Debug, Error)]
pub enum Refusal {
    #[error("`{component}` does not read: {message}")]
    Unreadable { component: &'static str, message: String },
    #[error("it states no {what}, and {why}")]
    Missing { what: String, why: &'static str },
    #[error("`{at}` is {value}, and a run needs {rule}")]
    Unusable { at: String, value: f64, rule: &'static str },
    #[error(
        "`{at}` states a deceleration of {decel} mm/s² beside an acceleration of {accel} mm/s², and the \
         simulator holds one figure for both; state no deceleration, or the acceleration's"
    )]
    Deceleration { at: String, accel: f64, decel: f64 },
    #[error("`lift` states {count} strokes from `{from}` to `{to}`, and a handoff reads one")]
    RepeatedStroke { from: &'static str, to: &'static str, count: usize },
    #[error("its `successor` names {of:?}, which this world does not carry")]
    SuccessorOfMissing { of: String },
    #[error("its `successor` names {of}, itself an override, and an override of an override is not read")]
    SuccessorOfSuccessor { of: EntityId },
    #[error("it succeeds {of} and replaces no component of it, so it overrides nothing")]
    SuccessorReplacesNothing { of: EntityId },
}

pub fn htm_figures(entity: &Entity) -> Result<HtmFigures, RobotError> {
    read_htm(&mut Reader::new(entity))
}

pub fn vtm_figures(entity: &Entity) -> Result<VtmMotion, RobotError> {
    read_vtm(&mut Reader::new(entity))
}

/// Studio cannot use the legacy settings fallback: each deployed class must
/// state its figures and each floor drive must state its physical model. The
/// world has already resolved successors and checked each model independently.
pub fn require_catalog_motion(world: &WorldMap) -> Result<(), String> {
    if world.map.fleet_config().htms > 0 && world.robot_catalog.htm.is_empty() {
        return Err("The deployed HTM fleet has no catalog motion: its robot classes must state floor_drive (including kind) and end_effector.".into());
    }
    if world.map.vtm_count() > 0 && world.robot_catalog.vtm.is_empty() {
        return Err("The deployed VTM fleet has no catalog motion: its robot classes must state rack_ladder or floor_drive, lift.profile and end_effector.".into());
    }
    for cohort in world.world.entities.values().filter_map(|entity| entity.components.get("cohort")) {
        if cohort.get("count").and_then(Value::as_u64) == Some(0) {
            continue;
        }
        let Some(class) = cohort.get("robot_class").and_then(Value::as_str) else { continue };
        let id: EntityId = class
            .split('@')
            .next()
            .unwrap_or(class)
            .parse()
            .map_err(|fault| format!("catalog class {class}: {fault}"))?;
        let entity =
            world.world.entities.get(&id).ok_or_else(|| format!("The world carries no catalog class {class}."))?;
        let role = entity.components.get("robot_class").and_then(|class| class.get("role")).and_then(Value::as_str);
        if !matches!(role, Some("htm" | "vtm")) {
            continue;
        }
        let read = Reader::new(entity);
        if role == Some("htm") || !entity.has("rack_ladder") {
            let drive: FloorDrive = read
                .required("a floor-driving catalog robot must state its motion")
                .map_err(|fault| fault.to_string())?;
            if drive.kind.is_none() {
                return Err(read
                    .missing("`floor_drive.kind`", "a Studio run takes its physical model from the catalog")
                    .to_string());
            }
        }
    }
    Ok(())
}

/// A belt handoff at a station is composed, which is our reading of the
/// 2026-09-21 directive pending the team's confirmation (docs/walmart-facts.md
/// section 9.12): a belt pick is the pick plus the `lift` stroke from `min` to
/// `cantilever_1`, and a belt drop is the drop plus the stroke from `max` to
/// `min`. No other stroke enters a handoff, so the M5F's pre-jacking stroke
/// adds nothing, and a robot without the stroke leaves the belt figure
/// unstated, so the relay figure stands in.
fn read_htm(read: &mut Reader) -> Result<HtmFigures, RobotError> {
    let drive: FloorDrive = read.required("an HTM drives on the floor")?;
    let effector: EndEffector = read.required("an HTM takes and places carriers with it")?;
    let lift: Option<Lift> = read.optional()?;
    let (loaded_speed, loaded_accel) = read.profile("floor_drive.loaded", &drive.loaded.profile)?;
    let (empty_speed, empty_accel) = read.profile("floor_drive.empty", &drive.empty.profile)?;
    let pickup_secs = read.zero_or_more("end_effector.pick_secs".into(), effector.pick_secs)?;
    let deposit_secs = read.zero_or_more("end_effector.drop_secs".into(), effector.drop_secs)?;
    let belt_pickup = read.stroke(lift.as_ref(), "min", "cantilever_1")?;
    let belt_deposit = read.stroke(lift.as_ref(), "max", "min")?;
    Ok(HtmFigures {
        kind: drive.kind,
        pickup_secs,
        deposit_secs,
        conveyor_pickup_secs: belt_pickup.map(|secs| pickup_secs + secs),
        conveyor_deposit_secs: belt_deposit.map(|secs| deposit_secs + secs),
        loaded_speed,
        empty_speed,
        loaded_constant_speed: read.constant_speed("floor_drive.loaded", &drive.loaded, drive.kind)?,
        empty_constant_speed: read.constant_speed("floor_drive.empty", &drive.empty, drive.kind)?,
        loaded_accel,
        empty_accel,
        loaded_rotate: read.quarter_turn("floor_drive.loaded", &drive.loaded)?,
        empty_rotate: read.quarter_turn("floor_drive.empty", &drive.empty)?,
        loaded_rotate_180: read.reversal("floor_drive.loaded", &drive.loaded)?,
        empty_rotate_180: read.reversal("floor_drive.empty", &drive.empty)?,
    })
}

/// A VTM composes its own chassis, lift and handling. An older floor VTM
/// without a kind keeps its historical trapezoidal assigned-aisle motion.
fn read_vtm(read: &mut Reader) -> Result<VtmMotion, RobotError> {
    let effector: EndEffector = read.required("a VTM takes and places totes with it")?;
    let lift: Lift = read.required("a VTM lifts totes between shelves")?;
    let lift_profile =
        lift.profile.ok_or_else(|| read.missing("`lift.profile`", "the VTM lifts on a speed profile"))?;
    let (lift_speed, lift_accel) = read.profile("lift.profile", &lift_profile)?;
    let chassis = match read.optional::<RackLadder>()? {
        Some(ladder) => {
            let (speed, accel) = read.profile("rack_ladder", &ladder.profile)?;
            VtmChassis::Ladder(LadderMotionModel { max_speed_m_s: speed, accel_m_s2: accel })
        }
        None => {
            let drive: FloorDrive = read
                .optional()?
                .ok_or_else(|| read.missing("`rack_ladder` or `floor_drive`", "a VTM must state its chassis motion"))?;
            VtmChassis::Floor(FloorVtmMotionModel {
                empty: read.axis("floor_drive.empty", &drive.empty, drive.kind)?,
                loaded: read.axis("floor_drive.loaded", &drive.loaded, drive.kind)?,
            })
        }
    };
    let why = "a VTM aligns before every fork insertion; a robot that does no scan states 0";
    let pick_alignment =
        effector.align_before_pick_secs.ok_or_else(|| read.missing("`end_effector.align_before_pick_secs`", why))?;
    let drop_alignment =
        effector.align_before_drop_secs.ok_or_else(|| read.missing("`end_effector.align_before_drop_secs`", why))?;
    Ok(VtmMotion {
        chassis,
        lift: AxisMotion::Trapezoidal { max_speed_m_s: lift_speed, accel_m_s2: lift_accel },
        handling: HandlingTimes {
            pick_secs: read.zero_or_more("end_effector.pick_secs".into(), effector.pick_secs)?,
            drop_secs: read.zero_or_more("end_effector.drop_secs".into(), effector.drop_secs)?,
            pick_alignment_secs: read.zero_or_more("end_effector.align_before_pick_secs".into(), pick_alignment)?,
            drop_alignment_secs: read.zero_or_more("end_effector.align_before_drop_secs".into(), drop_alignment)?,
        },
    })
}

/// Validates the fields used by this model. A turn must be positive because
/// a zero-duration turn would change facing without a rotation in the plan.
struct Reader<'a> {
    entity: &'a Entity,
}

impl<'a> Reader<'a> {
    fn new(entity: &'a Entity) -> Self {
        Reader { entity }
    }

    fn refuse(&self, refusal: Refusal) -> RobotError {
        RobotError { entity: self.entity.id.clone(), name: self.entity.name.clone(), refusal }
    }

    fn optional<T: Aspect>(&self) -> Result<Option<T>, RobotError> {
        self.entity.component::<T>(T::NAME).map_err(|error| {
            let message = match LoadError::from(error) {
                LoadError::ComponentShape { source, .. } => source.to_string(),
                other => other.to_string(),
            };
            self.refuse(Refusal::Unreadable { component: T::NAME, message })
        })
    }

    fn required<T: Aspect>(&self, why: &'static str) -> Result<T, RobotError> {
        self.optional()?.ok_or_else(|| self.missing(&format!("`{}`", T::NAME), why))
    }

    fn missing(&self, what: &str, why: &'static str) -> RobotError {
        self.refuse(Refusal::Missing { what: what.to_string(), why })
    }

    /// A profile's top speed and acceleration, in m/s and m/s².
    fn profile(&mut self, at: &str, profile: &Profile) -> Result<(f64, f64), RobotError> {
        let speed = self.above_zero(format!("{at}.max_speed_mm_s"), profile.max_speed_mm_s)?;
        let accel = self.above_zero(format!("{at}.accel_mm_s2"), profile.accel_mm_s2)?;
        if let Some(decel) = profile.decel_mm_s2.filter(|decel| *decel != accel) {
            return Err(self.refuse(Refusal::Deceleration { at: at.to_string(), accel, decel }));
        }
        Ok((speed / 1000.0, accel / 1000.0))
    }

    fn axis(&mut self, at: &str, drive: &Drive, kind: Option<FloorDriveKind>) -> Result<AxisMotion, RobotError> {
        let (max_speed_m_s, accel_m_s2) = self.profile(at, &drive.profile)?;
        let cruise = self.constant_speed(at, drive, kind)?;
        Ok(match kind {
            Some(FloorDriveKind::Constant) => {
                AxisMotion::Constant { speed_m_s: cruise.expect("constant cruise validated") }
            }
            _ => AxisMotion::Trapezoidal { max_speed_m_s, accel_m_s2 },
        })
    }

    fn quarter_turn(&mut self, at: &str, drive: &Drive) -> Result<f64, RobotError> {
        let at = format!("{at}.quarter_turn_secs");
        let secs = drive
            .quarter_turn_secs
            .ok_or_else(|| self.missing(&format!("`{at}`"), "an HTM turns in place on the floor"))?;
        self.above_zero(at, secs)
    }

    fn constant_speed(
        &mut self,
        at: &str,
        drive: &Drive,
        kind: Option<FloorDriveKind>,
    ) -> Result<Option<f64>, RobotError> {
        let at = format!("{at}.constant_speed_mm_s");
        match drive.constant_speed_mm_s {
            Some(speed) => self.above_zero(at, speed).map(|speed| Some(speed / 1000.0)),
            None if kind != Some(FloorDriveKind::Constant) => Ok(None),
            None => Err(self.missing(
                &format!("`{at}`"),
                "a constant floor drive states its execution cruise independently of its catalog maximum",
            )),
        }
    }

    fn reversal(&mut self, at: &str, drive: &Drive) -> Result<Option<f64>, RobotError> {
        drive.reversal_secs.map(|secs| self.above_zero(format!("{at}.reversal_secs"), secs)).transpose()
    }

    fn stroke(&mut self, lift: Option<&Lift>, from: &'static str, to: &'static str) -> Result<Option<f64>, RobotError> {
        let strokes: Vec<f64> = lift
            .into_iter()
            .flat_map(|lift| &lift.strokes)
            .filter(|stroke| stroke.from.as_deref() == Some(from) && stroke.to == to)
            .map(|stroke| stroke.secs)
            .collect();
        match strokes[..] {
            [] => Ok(None),
            [secs] => self.zero_or_more(format!("lift.strokes[{from} to {to}].secs"), secs).map(Some),
            _ => Err(self.refuse(Refusal::RepeatedStroke { from, to, count: strokes.len() })),
        }
    }

    fn above_zero(&mut self, at: String, value: f64) -> Result<f64, RobotError> {
        self.check(at, value, value.is_finite() && value > 0.0, "a finite figure above zero")
    }

    fn zero_or_more(&mut self, at: String, value: f64) -> Result<f64, RobotError> {
        self.check(at, value, value.is_finite() && value >= 0.0, "a finite figure, zero or more")
    }

    fn check(&mut self, at: String, value: f64, usable: bool, rule: &'static str) -> Result<f64, RobotError> {
        if !usable {
            return Err(self.refuse(Refusal::Unusable { at, value, rule }));
        }
        Ok(value)
    }
}

fn named(name: &Option<String>) -> String {
    name.as_ref().map(|name| format!(" ({name:?})")).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use site_model::entity::Entity;
    use site_model::ids::EntityId;

    use super::{htm_figures, vtm_figures};
    use crate::robots::RobotCatalog;
    use crate::robots::vtm::model::{AxisMotion, VtmChassis};

    const A: &str = "00000000-0000-4000-8000-00000000000a";
    const B: &str = "00000000-0000-4000-8000-00000000000b";

    fn robot(id: &str, name: &str, components: Value) -> Entity {
        let mut entity = Entity::new(id.parse::<EntityId>().unwrap());
        entity.name = Some(name.to_string());
        entity.components = components.as_object().unwrap().clone().into_iter().collect();
        entity
    }

    /// A VTM that drives its aisle floor, as DFW6's do.
    fn vtm() -> Value {
        json!({
            "floor_drive": {
                "loaded": { "max_speed_mm_s": 1500, "accel_mm_s2": 500 },
                "empty": { "max_speed_mm_s": 1500, "accel_mm_s2": 500 }
            },
            "lift": { "profile": { "max_speed_mm_s": 1500, "accel_mm_s2": 500 } },
            "end_effector": {
                "pick_secs": 4.0,
                "drop_secs": 4.0,
                "align_before_pick_secs": 3.56,
                "align_before_drop_secs": 2.3
            }
        })
    }

    /// An HTM whose lift strokes compose both belt handoffs, each stroke timed
    /// apart from the others so a handoff that took the wrong one shows.
    fn htm() -> Value {
        let turns = json!({ "max_speed_mm_s": 4000, "accel_mm_s2": 1800, "quarter_turn_secs": 2.16 });
        json!({
            "floor_drive": { "loaded": with(turns.clone(), "/reversal_secs", json!(2.88)), "empty": turns },
            "lift": { "strokes": [
                { "from": "min", "to": "max", "secs": 5.28 },
                { "from": "min", "to": "cantilever_1", "secs": 2.0 },
                { "from": "max", "to": "min", "secs": 5.0 },
                { "to": "cantilever_1", "secs": 1.5 }
            ] },
            "end_effector": { "kind": "comb_platform", "pick_secs": 2.1, "drop_secs": 2.2, "$comment": "ignored" }
        })
    }

    /// `components` with the field at `pointer` set to `value`, or removed
    /// when `value` is null.
    fn with(mut components: Value, pointer: &str, value: Value) -> Value {
        let (parent, field) = pointer.rsplit_once('/').unwrap();
        let parent = components.pointer_mut(parent).unwrap().as_object_mut().unwrap();
        match value {
            Value::Null => parent.remove(field),
            value => parent.insert(field.to_string(), value),
        };
        components
    }

    #[test]
    fn an_htm_takes_its_belt_handoffs_from_two_named_strokes_and_no_other() {
        let figures = htm_figures(&robot(A, "K50H", htm())).unwrap();
        assert_eq!((figures.loaded_speed, figures.loaded_accel), (4.0, 1.8), "mm/s and mm/s² become m/s and m/s²");
        assert_eq!(figures.conveyor_pickup_secs, Some(2.1 + 2.0), "the pick and the lift from min to cantilever_1");
        assert_eq!(figures.conveyor_deposit_secs, Some(2.2 + 5.0), "the drop and the lower from max to min");
        assert_eq!((figures.loaded_rotate_180, figures.empty_rotate_180), (Some(2.88), None));
        let no_lift = htm_figures(&robot(A, "K50H", with(htm(), "/lift", Value::Null))).unwrap();
        assert_eq!((no_lift.conveyor_pickup_secs, no_lift.conveyor_deposit_secs), (None, None));
    }

    #[test]
    fn different_models_and_handling_remain_separate() {
        let mut constant = htm();
        constant["floor_drive"]["kind"] = json!("constant");
        for load in ["loaded", "empty"] {
            constant["floor_drive"][load]["constant_speed_mm_s"] = json!(6000);
        }
        let first = robot(A, "constant", constant);
        let mut trapezoidal = htm();
        trapezoidal["floor_drive"]["kind"] = json!("trapezoidal");
        trapezoidal["end_effector"]["pick_secs"] = json!(7.0);
        let second = robot(B, "trapezoidal", trapezoidal);
        let catalog = RobotCatalog::read(&[&first, &second], &[]).unwrap();
        assert_eq!(catalog.htm.len(), 2);
        assert_eq!(catalog.htm[&first.id].loaded_constant_speed, Some(6.0));
        assert_eq!(catalog.htm[&second.id].loaded_constant_speed, None, "trapezoidal requires no unused cruise");
        assert_eq!(catalog.htm[&second.id].pickup_secs, 7.0);
        let invalid = robot(B, "constant", with(htm(), "/floor_drive/kind", json!("constant")));
        assert!(htm_figures(&invalid).unwrap_err().to_string().contains("constant_speed_mm_s"));
    }

    #[test]
    fn vtm_keeps_its_chassis_kind_and_load_profiles() {
        let uneven = with(vtm(), "/floor_drive/empty/max_speed_mm_s", json!(1800));
        let model = vtm_figures(&robot(A, "floor", uneven.clone())).unwrap();
        let VtmChassis::Floor(floor) = model.chassis else { panic!("floor model") };
        assert_eq!(floor.empty, AxisMotion::Trapezoidal { max_speed_m_s: 1.8, accel_m_s2: 0.5 });
        assert_eq!(floor.loaded, AxisMotion::Trapezoidal { max_speed_m_s: 1.5, accel_m_s2: 0.5 });
        let ladder = with(uneven, "/rack_ladder", json!({ "max_speed_mm_s": 3000, "accel_mm_s2": 2000 }));
        let model = vtm_figures(&robot(A, "ladder", ladder)).unwrap();
        let VtmChassis::Ladder(ladder) = model.chassis else { panic!("ladder model") };
        assert_eq!((ladder.max_speed_m_s, ladder.accel_m_s2), (3.0, 2.0));
        assert_eq!(model.lift, AxisMotion::Trapezoidal { max_speed_m_s: 1.5, accel_m_s2: 0.5 });
        let mut constant = vtm();
        constant["floor_drive"]["kind"] = json!("constant");
        for load in ["loaded", "empty"] {
            constant["floor_drive"][load]["constant_speed_mm_s"] = json!(1000);
        }
        let model = vtm_figures(&robot(A, "constant", constant)).unwrap();
        let VtmChassis::Floor(floor) = model.chassis else { panic!("floor model") };
        assert_eq!(floor.empty, AxisMotion::Constant { speed_m_s: 1.0 });
    }

    /// Each rule the figures are read under refuses its own breach, naming the
    /// entity by id and name, and the component path once.
    #[test]
    fn a_figure_a_run_cannot_use_is_refused() {
        let vtm_refusals = [
            (with(vtm(), "/end_effector/align_before_pick_secs", Value::Null), "no `end_effector.align_before_pick"),
            (with(vtm(), "/lift/profile", Value::Null), "no `lift.profile`"),
            (with(vtm(), "/lift", json!({ "profile": 3 })), "`lift` does not read: invalid type"),
        ];
        for (components, refusal) in vtm_refusals {
            let refused = vtm_figures(&robot(A, "VTM", components)).unwrap_err().to_string();
            assert!(refused.starts_with(&format!("robot entity {A} (\"VTM\"): ")), "{refused}");
            assert!(refused.contains(refusal) && refused.matches(A).count() == 1, "{refused}");
        }
        let htm_refusals = [
            (with(htm(), "/floor_drive/loaded/quarter_turn_secs", Value::Null), "no `floor_drive.loaded.quarter_turn"),
            (
                with(htm(), "/floor_drive/loaded/quarter_turn_secs", json!(0)),
                "`floor_drive.loaded.quarter_turn_secs` is 0",
            ),
            (
                with(htm(), "/floor_drive/empty/quarter_turn_secs", json!(0)),
                "`floor_drive.empty.quarter_turn_secs` is 0",
            ),
            (
                with(htm(), "/floor_drive/empty/decel_mm_s2", json!(1500)),
                "`floor_drive.empty` states a deceleration of 1500",
            ),
            (with(htm(), "/floor_drive/empty/max_speed_mm_s", json!(0)), "`floor_drive.empty.max_speed_mm_s` is 0"),
            (with(htm(), "/floor_drive/loaded/reversal_secs", json!(0.0)), "`floor_drive.loaded.reversal_secs` is 0"),
            (with(htm(), "/lift/strokes/3/from", json!("min")), "2 strokes from `min` to `cantilever_1`"),
        ];
        for (components, refusal) in htm_refusals {
            let refused = htm_figures(&robot(A, "HTM", components)).unwrap_err().to_string();
            assert!(refused.contains(refusal) && refused.contains("(\"HTM\")"), "{refused}");
        }
        let decel_equal = with(htm(), "/floor_drive/empty/decel_mm_s2", json!(1800));
        assert!(htm_figures(&robot(A, "HTM", decel_equal)).is_ok(), "a deceleration equal to the acceleration");
    }

    #[test]
    fn only_a_fleet_without_any_aspects_uses_legacy_settings() {
        let bare = robot(A, "VTM", json!({ "robot_class": {} }));
        assert!(RobotCatalog::read(&[], &[&bare]).unwrap().vtm.is_empty());
        let incomplete = robot(A, "VTM", json!({ "lineage": [], "body": { "height_mm": 900 } }));
        assert!(RobotCatalog::read(&[], &[&incomplete]).unwrap_err().to_string().contains("end_effector"));
        let complete = robot(B, "C56", vtm());
        assert!(RobotCatalog::read(&[], &[&complete, &bare]).is_err());
    }
}
