//! Within `worldview-render`, `camera` is WHERE THE VIEWER STANDS and what a
//! pointer gesture does to that: the matrices the shaders receive, the passage
//! between a place on the floor and a place on the screen, and the pan, orbit,
//! zoom and fly-to a viewer drives it with.
//!
//! Orthographic at top-down, perspective when orbited. Drag = pan,
//! shift+drag = orbit, scroll = zoom, ESC/double-click = reset.
//!
//! It knows no toolkit: a host fills in a [`PointerInput`] from whatever it
//! reads its mouse with, so the same gesture means the same thing in a window
//! and in a browser tab.

mod input;
mod projection;

pub(crate) mod math;

pub use input::PointerInput;

use bytemuck::{Pod, Zeroable};
use math::lerp;

use crate::frame::RenderFrame;
use crate::view::ViewMode;

// Orbit defaults.
const DEFAULT_AZIMUTH: f32 = -std::f32::consts::FRAC_PI_2;
const DEFAULT_ELEVATION: f32 = std::f32::consts::FRAC_PI_2; // exact top-down
// Metres. A cell is ~0.86m, so these keep their index-space magnitudes; the
// far plane still clears the largest building we model (Dallas is ~92m x 70m)
// by two orders of magnitude, which keeps Depth32Float resolution ample.
const NEAR_PLANE: f32 = 0.1;
const FAR_PLANE: f32 = 10000.0;
const ORBIT_SENSITIVITY: f32 = 0.005; // radians per pixel
const MIN_ELEVATION: f32 = -0.3; // allow looking slightly upward (~-17 deg)
const MAX_ELEVATION: f32 = std::f32::consts::FRAC_PI_2; // allow exact top-down
const MIN_DISTANCE: f32 = 1.0;
const MAX_DISTANCE: f32 = 5000.0;
const SCROLL_ZOOM_SENSITIVITY: f32 = 0.002; // per scroll-delta unit
const MIN_EYE_Z: f32 = 0.5; // camera never goes below this height

// Ortho <-> perspective transition.
// Below this elevation, switch to perspective for foreshortening.
const PERSPECTIVE_ELEVATION: f32 = 1.2; // ~69 degrees
// FOV chosen so perspective matches ortho extent at center distance:
// half_h = distance * 0.5 = distance * tan(fov/2) -> fov = 2*atan(0.5) ~ 53.13 deg
const PERSPECTIVE_FOV_Y: f32 = 0.9273;

// Camera animation.
const ANIM_DURATION_SECS: f32 = 0.6;
const FLY_TO_DISTANCE: f32 = 25.0;
const FLY_TO_ELEVATION: f32 = 0.35; // ~20 degrees — slightly elevated side-on

/// The rectangle a viewport occupies on the host's screen, in its own pixels.
/// The camera needs both parts: the size to build its matrices, the corner to
/// turn a pointer position into a place on the floor.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Viewport {
    pub min: [f32; 2],
    pub size: [f32; 2],
}

impl Viewport {
    pub fn new(min: [f32; 2], size: [f32; 2]) -> Self {
        Self { min, size }
    }

    pub fn width(self) -> f32 {
        self.size[0]
    }

    pub fn height(self) -> f32 {
        self.size[1]
    }

    pub fn centre(self) -> [f32; 2] {
        [self.min[0] + self.size[0] * 0.5, self.min[1] + self.size[1] * 0.5]
    }
}

/// GPU-side camera uniforms. Must stay 16-byte aligned.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct CameraUniforms {
    pub projection: [[f32; 4]; 4],
    pub view: [[f32; 4]; 4],
    pub viewport_size: [f32; 2],
    pub cell_size: f32,
    /// The view the shaders switch on, as [`ViewMode::shader_code`] minted it.
    pub view_mode: u32,
}

impl CameraUniforms {
    /// Whether the robots and the containers are drawn in this view.
    pub fn draws_movers(&self) -> bool {
        self.view_mode == ViewMode::State.shader_code()
    }
}

/// Persistent camera state for pan/zoom/orbit interaction.
pub struct CameraState {
    /// The map's render geometry. Replaces the old `grid_width` /
    /// `grid_depth` cell counts, which were this same information in a form
    /// that only worked when a cell was exactly one world unit. The camera
    /// keeps it because it needs the map's extent to frame and clamp, and
    /// because it is the one thing that can convert a ground-plane hit into a
    /// cell without the camera owning any grid arithmetic itself.
    pub frame: RenderFrame,

    // Perspective orbit state (spherical coordinates).
    pub azimuth: f32,
    pub elevation: f32,
    pub distance: f32,
    pub orbit_target: [f32; 3],

    /// Last known viewport aspect ratio (updated each frame).
    aspect: f32,
    /// True until the first frame provides a real viewport aspect ratio.
    needs_fit: bool,

    // Camera animation state.
    anim: Option<CameraAnimation>,
}

struct CameraAnimation {
    start_azimuth: f32,
    start_elevation: f32,
    start_distance: f32,
    start_target: [f32; 3],
    end_azimuth: f32,
    end_elevation: f32,
    end_distance: f32,
    end_target: [f32; 3],
    elapsed: f32,
    duration: f32,
}

impl CameraState {
    pub fn new(frame: RenderFrame) -> Self {
        let (ew, ed) = frame.extent();
        let grid_center = [ew * 0.5, ed * 0.5, 0.0];
        let default_distance = Self::default_distance(ew, ed);
        Self {
            frame,
            azimuth: DEFAULT_AZIMUTH,
            elevation: DEFAULT_ELEVATION,
            distance: default_distance,
            orbit_target: grid_center,
            aspect: 1.0,
            needs_fit: true,
            anim: None,
        }
    }

    /// Clamp orbit_target so the camera center stays within the grid bounds
    /// (with a small margin). Prevents zooming/panning into empty space.
    fn clamp_orbit_target(&mut self) {
        let margin = self.distance * 0.5;
        let (w, h) = self.frame.extent();
        self.orbit_target[0] = self.orbit_target[0].clamp(-margin, w + margin);
        self.orbit_target[1] = self.orbit_target[1].clamp(-margin, h + margin);
        // Ensure eye stays above MIN_EYE_Z, but also relax orbit_target[2]
        // back toward 0 (ground plane) when there's room. Without this,
        // ground-skim lifts orbit_target[2] and it never comes back down.
        let eye_z = self.orbit_target[2] + self.distance * self.elevation.sin();
        if eye_z < MIN_EYE_Z {
            self.orbit_target[2] += MIN_EYE_Z - eye_z;
        } else if self.orbit_target[2] > 0.0 {
            let slack = eye_z - MIN_EYE_Z;
            self.orbit_target[2] -= self.orbit_target[2].min(slack);
        }
    }

    /// Compute the distance that fits the grid into a viewport with the given aspect ratio.
    /// In ortho mode, visible_height = distance and visible_width = distance * aspect.
    /// A 1% margin is added on each side so the grid doesn't touch the edges.
    pub fn fit_distance(extent_w: f32, extent_d: f32, aspect: f32) -> f32 {
        let (w, h) = (extent_w, extent_d);
        let margin = 1.02; // 1% padding on each side
        // Pick whichever axis is tighter.
        let dist = if aspect > 0.0 { (h * margin).max(w * margin / aspect) } else { h * margin };
        dist.clamp(MIN_DISTANCE, MAX_DISTANCE)
    }

    /// Fallback distance when aspect ratio is unknown (square assumption).
    pub fn default_distance(extent_w: f32, extent_d: f32) -> f32 {
        Self::fit_distance(extent_w, extent_d, 1.0)
    }

    /// Instantly snap to default top-down view (no animation).
    pub fn snap_to_default(&mut self) {
        self.azimuth = DEFAULT_AZIMUTH;
        self.elevation = DEFAULT_ELEVATION;
        self.distance = Self::fit_distance(self.frame.extent().0, self.frame.extent().1, self.aspect);
        self.orbit_target = [self.frame.extent().0 * 0.5, self.frame.extent().1 * 0.5, 0.0];
        self.anim = None;
    }

    /// Animate to the default top-down view.
    pub fn reset(&mut self) {
        self.animate_to(
            DEFAULT_AZIMUTH,
            DEFAULT_ELEVATION,
            Self::fit_distance(self.frame.extent().0, self.frame.extent().1, self.aspect),
            [self.frame.extent().0 * 0.5, self.frame.extent().1 * 0.5, 0.0],
        );
    }

    /// Animate to a view of the given render-space point (a cell centre,
    /// resolved by the caller from the cell's own tile).
    /// When zoomed in close, preserves current angle and just re-centers.
    /// When zoomed out, swoops in with azimuth derived from click direction.
    pub fn fly_to_world(&mut self, wx: f32, wy: f32, keep_elevation: bool) {
        let target = [wx, wy, 0.0];

        let half_fit = Self::fit_distance(self.frame.extent().0, self.frame.extent().1, self.aspect) * 0.5;
        let zoomed_in = self.distance < half_fit;

        let dist = if zoomed_in { (self.distance * 1.2).min(FLY_TO_DISTANCE) } else { FLY_TO_DISTANCE };

        if keep_elevation {
            // Shift+double-click: preserve current angle, just re-center.
            self.animate_to(self.azimuth, self.elevation, dist, target);
        } else if zoomed_in {
            // Already close — preserve angle, re-center.
            self.animate_to(self.azimuth, self.elevation, dist, target);
        } else {
            // Zoomed out — swoop in from direction of click.
            let cx = self.frame.extent().0 * 0.5;
            let cy = self.frame.extent().1 * 0.5;
            let dx = target[0] - cx;
            let dy = target[1] - cy;
            let az = if dx.abs() > 1.0 || dy.abs() > 1.0 {
                dy.atan2(dx) + std::f32::consts::PI
            } else {
                -std::f32::consts::FRAC_PI_4 * 0.5
            };
            let el = if self.elevation >= PERSPECTIVE_ELEVATION {
                FLY_TO_ELEVATION
            } else {
                self.elevation.max(FLY_TO_ELEVATION)
            };
            // Snap azimuth before animating so rotation is visible during
            // the elevation drop (azimuth is invisible at top-down).
            if self.elevation >= PERSPECTIVE_ELEVATION {
                self.azimuth = az;
            }
            self.animate_to(az, el, dist, target);
        }
    }

    fn animate_to(&mut self, azimuth: f32, elevation: f32, distance: f32, target: [f32; 3]) {
        self.anim = Some(CameraAnimation {
            start_azimuth: self.azimuth,
            start_elevation: self.elevation,
            start_distance: self.distance,
            start_target: self.orbit_target,
            end_azimuth: azimuth,
            end_elevation: elevation,
            end_distance: distance,
            end_target: target,
            elapsed: 0.0,
            duration: ANIM_DURATION_SECS,
        });
    }

    /// Finish the flight in progress at once, the camera standing where it
    /// would have landed: for a host whose camera moves jump.
    pub fn land(&mut self) {
        if let Some(anim) = self.anim.take() {
            (self.azimuth, self.elevation, self.distance) = (anim.end_azimuth, anim.end_elevation, anim.end_distance);
            self.orbit_target = anim.end_target;
        }
    }

    /// Advance animation by dt seconds. Returns true if animating.
    pub fn tick(&mut self, dt: f32) -> bool {
        if let Some(ref mut anim) = self.anim {
            anim.elapsed += dt;
            let t = (anim.elapsed / anim.duration).min(1.0);
            // Smooth ease-in-out (cubic).
            let t = if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 };

            self.azimuth = lerp(anim.start_azimuth, anim.end_azimuth, t);
            self.elevation = lerp(anim.start_elevation, anim.end_elevation, t);
            self.distance = lerp(anim.start_distance, anim.end_distance, t);
            self.orbit_target = [
                lerp(anim.start_target[0], anim.end_target[0], t),
                lerp(anim.start_target[1], anim.end_target[1], t),
                lerp(anim.start_target[2], anim.end_target[2], t),
            ];

            if anim.elapsed >= anim.duration {
                self.anim = None;
            }
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flight landed at once stands where the flight would have ended,
    /// with nothing left for a frame to advance.
    #[test]
    fn a_landed_flight_stands_where_it_would_have_ended() {
        let mut flown = CameraState::new(RenderFrame::from_extent_mm(40_000.0, 30_000.0));
        let mut landed = CameraState::new(RenderFrame::from_extent_mm(40_000.0, 30_000.0));
        for camera in [&mut flown, &mut landed] {
            camera.fly_to_world(5.0, 7.0, false);
        }
        while flown.tick(0.1) {}
        landed.land();
        assert!(!landed.tick(0.1), "no flight left in progress");
        assert_eq!(landed.orbit_target, [5.0, 7.0, 0.0]);
        let pose = |camera: &CameraState| [camera.azimuth, camera.elevation, camera.distance];
        for (a, b) in pose(&landed).into_iter().zip(pose(&flown)) {
            assert!((a - b).abs() < 1e-4, "{:?} against {:?}", pose(&landed), pose(&flown));
        }
    }
}
