//! Within `viewer`, `projection` is HOW THE CAMERA LOOKS AT A FLOOR (ruling D119, on spec 03 section 3.6's viewer and
//! section 5's stage keys): orthographic, straight down, where a drag pans and nothing tilts, the default of every
//! floor view in Studio; or orbit, a perspective camera the person turns and tilts by drag and by keys, a drag with
//! Shift panning instead. `worldview-render`'s camera holds both already, orthographic at top-down and perspective
//! below `PERSPECTIVE_ELEVATION`; this names the two as a page offers them and keeps the camera inside the one chosen
//! after every input. Until a page chooses, the camera turns as desvis's does (Shift and a drag orbit), so a
//! recording's page is unchanged. Each move here jumps.

use std::f32::consts::{FRAC_PI_2, PI};
use std::ops::Mul;

use wasm_bindgen::prelude::*;
use worldview_render::CameraState;

use super::Viewer;

#[wasm_bindgen(typescript_custom_section)]
const TYPES: &str = r#"
/** How the camera looks at the floor (ruling D119): `"orthographic"`, straight down, the default, a drag panning; or `"orbit"`, a perspective camera a drag turns and tilts, Shift and a drag panning. */
export type Projection = "orthographic" | "orbit";
"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "Projection")]
    pub type ProjectionName;
}

/// The projection a page chose, or none yet.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Projection {
    #[default]
    Unchosen,
    Orthographic,
    Orbit,
}

/// An angle, as worldview-render keeps the camera's bearing and its height above the floor.
#[derive(Clone, Copy)]
struct Radians(f32);

/// How many steps of a turn or a tilt a page asks for: one a key press or a button, any number by `orbit_by`.
#[derive(Clone, Copy)]
struct Steps(f32);

impl Mul<Radians> for Steps {
    type Output = Radians;

    fn mul(self, step: Radians) -> Radians {
        Radians(self.0 * step.0)
    }
}

/// Where the orbit starts and how far it reaches above the floor: below 1.2 rad the renderer draws in perspective, so
/// the orbit stays under it, and above the floor by enough to keep the eye off it.
const ORBIT_START: Radians = Radians(0.75);
const ORBIT_HIGHEST: Radians = Radians(1.15);
const ORBIT_LOWEST: Radians = Radians(0.2);
/// One step's turn and tilt.
const TURN_STEP: Radians = Radians(PI / 12.0);
const TILT_STEP: Radians = Radians(0.1);
/// Straight down, and the default camera's bearing, worldview-render's, which top-down restores so north stays up.
const STRAIGHT_DOWN: Radians = Radians(FRAC_PI_2);
const NORTH_UP: Radians = Radians(-FRAC_PI_2);

impl Projection {
    pub(crate) fn orbits(self) -> bool {
        self == Self::Orbit
    }

    /// Whether a drag may turn the camera at all.
    pub(crate) fn may_turn(self) -> bool {
        self != Self::Orthographic
    }

    /// Keep `camera` inside this projection: straight down and north up, or tilted within the orbit's reach.
    pub(crate) fn hold(self, camera: &mut CameraState) {
        match self {
            Self::Unchosen => {}
            Self::Orthographic => {
                camera.elevation = STRAIGHT_DOWN.0;
                camera.azimuth = NORTH_UP.0;
            }
            Self::Orbit => camera.elevation = camera.elevation.clamp(ORBIT_LOWEST.0, ORBIT_HIGHEST.0),
        }
    }
}

#[wasm_bindgen]
impl Viewer {
    /// Look at the floor `"orthographic"`ly, straight down, or in `"orbit"`, tilted to start with; the point at the
    /// centre of the view and the distance to it stay. Anything else is refused by name.
    pub fn set_projection(&mut self, projection: ProjectionName) -> Result<(), JsValue> {
        let chosen = match JsValue::from(projection).as_string().as_deref() {
            Some("orthographic") => Projection::Orthographic,
            Some("orbit") => Projection::Orbit,
            other => {
                return Err(JsValue::from_str(&format!(
                    "`{}` is not a projection; they are orthographic and orbit",
                    other.unwrap_or("a value that is not text")
                )));
            }
        };
        let (target, distance, azimuth, elevation) =
            (self.camera.orbit_target, self.camera.distance, self.camera.azimuth, self.camera.elevation);
        self.settle_camera();
        self.camera.snap_to_default();
        (self.camera.orbit_target, self.camera.distance) = ([target[0], target[1], 0.0], distance);
        if chosen.orbits() && !self.projection.orbits() {
            (self.camera.azimuth, self.camera.elevation) = (NORTH_UP.0, ORBIT_START.0);
        } else {
            (self.camera.azimuth, self.camera.elevation) = (azimuth, elevation);
        }
        self.projection = chosen;
        self.projection.hold(&mut self.camera);
        self.settle_camera();
        Ok(())
    }

    /// The projection the page chose last, none until it chooses.
    pub fn projection(&self) -> Option<ProjectionName> {
        let name = match self.projection {
            Projection::Unchosen => return None,
            Projection::Orthographic => "orthographic",
            Projection::Orbit => "orbit",
        };
        Some(JsValue::from_str(name).unchecked_into())
    }

    /// Turn the orbit by whole steps about the centre of the view, a twelfth of a half turn each, positive
    /// clockwise as seen from above; and tilt it, positive toward straight down, within the orbit's reach. Under
    /// orthographic it does nothing, since nothing tilts there.
    pub fn orbit_by(&mut self, turn: f32, tilt: f32) {
        if !self.projection.orbits() || !turn.is_finite() || !tilt.is_finite() {
            return;
        }
        self.settle_camera();
        let (turn, tilt) = (Steps(turn), Steps(tilt));
        self.camera.azimuth -= (turn * TURN_STEP).0;
        self.camera.elevation += (tilt * TILT_STEP).0;
        self.projection.hold(&mut self.camera);
        self.settle_camera();
    }
}
