//! Within `camera`, `projection` is the passage between the map and the
//! screen, both ways: the view and projection matrices the shaders receive,
//! the screen position of a point already known in metres, and the ray a
//! pointer gesture casts back down onto the ground.
//!
//! Both directions build the same two matrices from the same camera state, so
//! what a gesture picks and what the GPU draws cannot drift apart. The
//! matrices are orthographic above `PERSPECTIVE_ELEVATION` and perspective
//! below it, which is also where the depth buffer's resolution changes
//! character.
//!
//! Because these matrices decide depth, the tests at the foot are also where
//! the renderer's coplanar tiers are held: they run `cell_layer`'s biases and
//! a bot's floor clearance through the real projections at every camera the
//! controls reach, and say in depth-buffer steps what survives.

use super::math::*;
use super::{CameraState, CameraUniforms, FAR_PLANE, NEAR_PLANE, PERSPECTIVE_ELEVATION, PERSPECTIVE_FOV_Y, Viewport};
use crate::view::ViewMode;

impl CameraState {
    /// Build the uniform buffer data for the current view (always perspective).
    pub fn build_uniforms(&self, viewport: Viewport, cell_size: f32, view: ViewMode) -> CameraUniforms {
        self.build_uniforms_impl(viewport.width(), viewport.height(), cell_size, view)
    }

    /// Millimetre position under the cursor, in the map's own frame.
    ///
    /// The picking path for gestures that need a DISTANCE rather than an index:
    /// see [`crate::RenderFrame::mm_at_render`].
    pub fn screen_to_mm(&self, screen_pos: [f32; 2], viewport: Viewport) -> Option<[f64; 2]> {
        let ground = self.cursor_to_ground(screen_pos, viewport)?;
        Some(self.frame.mm_at_render(ground[0], ground[1]))
    }

    /// Project a RENDER-space metre point (Y already flipped) to screen. The
    /// path for anything that already knows where it is in metres — bots,
    /// totes, free-form nodes — with no index in the middle.
    pub fn world_to_screen(&self, x_m: f32, y_m: f32, viewport: Viewport) -> [f32; 2] {
        self.grid_to_screen_impl(x_m, y_m, 0.0, viewport)
    }

    /// Same, with height. A label projected at z=0 lands where the FLOOR is
    /// under a tilted camera, not where the elevated thing is — a VTM riding
    /// a 3m rack top puts metres of screen space between the two.
    pub fn world_to_screen_z(&self, x_m: f32, y_m: f32, z_m: f32, viewport: Viewport) -> [f32; 2] {
        self.grid_to_screen_impl(x_m, y_m, z_m, viewport)
    }

    /// Ray-cast cursor position to z=0 ground plane, returning world [x, y].
    pub(super) fn cursor_to_ground(&self, screen_pos: [f32; 2], viewport: Viewport) -> Option<[f32; 2]> {
        let (ray_origin, ray_dir) = self.cursor_ray(screen_pos, viewport);
        if ray_dir[2].abs() < 1e-8 {
            return None;
        }
        let t = -ray_origin[2] / ray_dir[2];
        if t < 0.0 {
            return None;
        }
        Some([ray_origin[0] + t * ray_dir[0], ray_origin[1] + t * ray_dir[1]])
    }

    /// The pick ray under the cursor, in render space: origin and direction.
    /// Orthographic above `PERSPECTIVE_ELEVATION` (parallel rays offset by
    /// the cursor), perspective below it (rays fanning from the eye). The
    /// ground intersection above is one consumer; a 3D pick against standing
    /// geometry walks the whole ray instead.
    pub fn cursor_ray(&self, screen_pos: [f32; 2], viewport: Viewport) -> ([f32; 3], [f32; 3]) {
        let aspect = viewport.width() / viewport.height();
        let ndc_x = ((screen_pos[0] - viewport.min[0]) / viewport.width()) * 2.0 - 1.0;
        let ndc_y = 1.0 - ((screen_pos[1] - viewport.min[1]) / viewport.height()) * 2.0;

        let eye = spherical_to_eye(self.orbit_target, self.azimuth, self.elevation, self.distance);
        let forward = normalize(sub(self.orbit_target, eye));
        let up_hint = up_for_elevation(self.azimuth, self.elevation);
        let right = normalize(cross(forward, up_hint));
        let up = cross(right, forward);

        if self.elevation >= PERSPECTIVE_ELEVATION {
            let half_h = self.distance * 0.5;
            let half_w = half_h * aspect;
            let origin = [
                eye[0] + ndc_x * half_w * right[0] + ndc_y * half_h * up[0],
                eye[1] + ndc_x * half_w * right[1] + ndc_y * half_h * up[1],
                eye[2] + ndc_x * half_w * right[2] + ndc_y * half_h * up[2],
            ];
            (origin, forward)
        } else {
            let half_h = (PERSPECTIVE_FOV_Y * 0.5).tan();
            let half_w = half_h * aspect;
            let dir = normalize([
                ndc_x * half_w * right[0] + ndc_y * half_h * up[0] + forward[0],
                ndc_x * half_w * right[1] + ndc_y * half_h * up[1] + forward[1],
                ndc_x * half_w * right[2] + ndc_y * half_h * up[2] + forward[2],
            ]);
            (eye, dir)
        }
    }

    fn build_uniforms_impl(
        &self,
        viewport_width: f32,
        viewport_height: f32,
        cell_size: f32,
        view: ViewMode,
    ) -> CameraUniforms {
        let aspect = viewport_width / viewport_height;

        let eye = spherical_to_eye(self.orbit_target, self.azimuth, self.elevation, self.distance);
        let up = up_for_elevation(self.azimuth, self.elevation);
        let look = look_at(eye, self.orbit_target, up);

        let projection = if self.elevation >= PERSPECTIVE_ELEVATION {
            // Top-down: orthographic (no foreshortening).
            let half_h = self.distance * 0.5;
            let half_w = half_h * aspect;
            orthographic(half_w, half_h, NEAR_PLANE, FAR_PLANE)
        } else {
            // Orbited: perspective (FOV matched to ortho extent at center).
            perspective(PERSPECTIVE_FOV_Y, aspect, NEAR_PLANE, FAR_PLANE)
        };

        CameraUniforms {
            projection,
            view: look,
            viewport_size: [viewport_width, viewport_height],
            cell_size,
            view_mode: view.shader_code(),
        }
    }

    fn grid_to_screen_impl(&self, world_x: f32, world_y: f32, world_z: f32, viewport: Viewport) -> [f32; 2] {
        let vw = viewport.width();
        let vh = viewport.height();
        let aspect = vw / vh;

        let eye = spherical_to_eye(self.orbit_target, self.azimuth, self.elevation, self.distance);
        let up = up_for_elevation(self.azimuth, self.elevation);
        let view = look_at(eye, self.orbit_target, up);
        let proj = if self.elevation >= PERSPECTIVE_ELEVATION {
            let half_h = self.distance * 0.5;
            let half_w = half_h * aspect;
            orthographic(half_w, half_h, NEAR_PLANE, FAR_PLANE)
        } else {
            perspective(PERSPECTIVE_FOV_Y, aspect, NEAR_PLANE, FAR_PLANE)
        };

        let world = [world_x, world_y, world_z, 1.0];

        // Apply view then projection.
        let view_pos = mat4_mul_vec4(view, world);
        let clip = mat4_mul_vec4(proj, view_pos);

        if clip[3].abs() < 1e-10 {
            return [f32::NAN, f32::NAN];
        }

        let ndc_x = clip[0] / clip[3];
        let ndc_y = clip[1] / clip[3];

        // NDC [-1,1] to screen. NDC Y-up, screen Y-down.
        let sx = viewport.min[0] + (ndc_x + 1.0) * 0.5 * vw;
        let sy = viewport.min[1] + (1.0 - ndc_y) * 0.5 * vh;

        [sx, sy]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::{MIN_DISTANCE, MIN_ELEVATION};
    use crate::frame::{FLOOR_CLEARANCE_M, RenderFrame};
    use crate::renderer::cell_layer::{MARKER_BIAS, TILE_BIAS, UNDERLAY_BIAS};

    /// The depth value the grid shader writes for a point on the ground
    /// carrying `depth_bias`, through the camera's own matrices and the
    /// shader's own arithmetic (`clip.z += bias * clip.w`, then the divide).
    fn depth(cam: &CameraState, world: [f32; 3], depth_bias: f32) -> f32 {
        let u = cam.build_uniforms(Viewport::new([0.0, 0.0], [1600.0, 1000.0]), 1.0, ViewMode::State);
        let view_pos = mat4_mul_vec4(u.view, [world[0], world[1], world[2], 1.0]);
        let clip = mat4_mul_vec4(u.projection, view_pos);
        (clip[2] + depth_bias * clip[3]) / clip[3]
    }

    /// Distance between two f32 depths, counted in the steps the depth buffer
    /// actually has. Zero means the buffer cannot tell them apart at all.
    fn ulps_between(a: f32, b: f32) -> i64 {
        i64::from(a.to_bits()) - i64::from(b.to_bits())
    }

    /// A camera the controls can actually reach, settled the way the input
    /// path settles it. `clamp_orbit_target` is the part that matters below:
    /// at a negative elevation the eye would be under the floor, so it LIFTS
    /// the orbit target until the eye clears `MIN_EYE_Z`, and the view then
    /// looks upward from near the ground. Setting the fields alone would build
    /// a camera the app never has.
    fn camera_at(distance: f32, elevation: f32) -> CameraState {
        let mut cam = CameraState::new(RenderFrame::from_extent_mm(200_000.0, 150_000.0));
        cam.distance = distance;
        cam.elevation = elevation;
        cam.orbit_target = [50.0, 50.0, 0.0];
        cam.clamp_orbit_target();
        cam
    }

    /// Every elevation the orbit control can reach, from exact top-down to the
    /// steepest upward tilt, coarse enough to run in the unit suite and fine
    /// enough to cross `PERSPECTIVE_ELEVATION` (1.2 rad) where the projection
    /// changes.
    const REACHABLE_ELEVATIONS: [f32; 8] =
        [std::f32::consts::FRAC_PI_2, 1.19, 0.6, 0.2, 0.0, -0.1, -0.2, MIN_ELEVATION];

    /// A tier step survives the depth buffer at every zoom the camera reaches,
    /// under BOTH projections, and the nearest tier stays clear of true z=0.
    ///
    /// This is the property a millimetre of world-space offset did not have,
    /// and its absence was the z-fight Daniel saw on DFW6 (2026-09-04).
    /// `Depth32Float` spends its mantissa near the near plane, so under the
    /// perspective camera a 1mm step measured 2 steps of the buffer at a 20m
    /// camera, one from 30m to 50m, and NONE at 80m and beyond — past which
    /// the underlay and the cell above it hold the same float and the winner
    /// is decided per pixel by interpolation noise.
    ///
    /// Eight steps is the bar: a couple of steps of interpolation error across
    /// a quad the size of a building must not be able to close the gap. What
    /// this actually holds today is 32 to 35 under the perspective camera and
    /// hundreds or thousands under the orthographic one.
    #[test]
    fn a_depth_tier_survives_the_depth_buffer_at_every_zoom() {
        const FLOOR: [f32; 3] = [50.0, 50.0, 0.0];
        for elevation in REACHABLE_ELEVATIONS {
            for distance in [MIN_DISTANCE, 2.0, 5.0, 20.0, 50.0, 120.0, 400.0, 1000.0, 5000.0] {
                let cam = camera_at(distance, elevation);
                let ground = depth(&cam, FLOOR, 0.0);
                let marker = depth(&cam, FLOOR, MARKER_BIAS);
                let tile = depth(&cam, FLOOR, TILE_BIAS);
                let under = depth(&cam, FLOOR, UNDERLAY_BIAS);
                let what = format!("at {distance}m, elevation {elevation}");
                assert!(marker > ground, "the marker tier must sit BEHIND unbiased z=0 geometry {what}");
                assert!(tile > marker, "a declared tile must sit BEHIND the markers standing in it {what}");
                assert!(under > tile, "the underlay must sit BEHIND the tiles {what}");
                for (nearer, farther, pair) in [
                    (ground, marker, "z=0 over marker"),
                    (marker, tile, "marker over tile"),
                    (tile, under, "tile over underlay"),
                ] {
                    let steps = ulps_between(farther, nearer);
                    assert!(steps >= 8, "{pair} is only {steps} depth steps apart {what}");
                }
            }
        }
    }

    /// From this camera distance out, a bot standing on ANY floor tier draws in
    /// front of it at every elevation the controls reach. Closer than it, and
    /// only while looking upward, see the residual below.
    const UPWARD_SAFE_M: f32 = 16.0;

    /// Nothing standing ON the floor is pushed behind it — with one measured
    /// exception, stated here rather than asserted away.
    ///
    /// The tiers only ever bias AWAY from the camera, and this is why. A bot is
    /// lifted `FLOOR_CLEARANCE_M` — 10mm — which stops meaning anything to the
    /// depth buffer at about the same distances a millimetre does. A tier
    /// biased TOWARD the camera would pass it: past a ~25m camera a marker cell
    /// would be drawn in front of a bot standing on it, and the bot would come
    /// out muted through `renderer::bots`' occluded pass, as though a rack were
    /// hiding it.
    ///
    /// Looking DOWN is not the only case, which is what the first version of
    /// this test missed (Codex review, 2026-09-04). `MIN_ELEVATION` is -0.3
    /// rad, and at a negative elevation the input path lifts the orbit target
    /// to keep the eye above `MIN_EYE_Z`; the camera then looks UPWARD from
    /// near the ground, where a higher point is FARTHER along the view
    /// direction. The bot's 10mm of clearance is then depth it LOSES, not depth
    /// it gains, and against a marker tier at zero bias it lost at every
    /// distance.
    ///
    /// **The residual.** Every tier is now at least one `DEPTH_TIER` behind
    /// true z=0, which buys back everything from 13.5m out. Measured through
    /// these matrices at `MIN_ELEVATION`, a bot draws in front of the marker
    /// tier from 13.5m, the tile tier from 9.6m and the underlay from 7.9m; the
    /// boundary falls with the tilt (10.3m / 7.5m / 6.2m at -0.2 rad, 7.2m /
    /// 5.1m / 4.2m at -0.1, and nothing at all at 0, where a lift costs no
    /// depth). So between `MIN_DISTANCE` and 13.5m, looking up at the steepest
    /// tilt the controls allow, a bot standing on a marker cell can still be
    /// drawn muted. Closing that needs the bot's clearance expressed in
    /// depth-buffer units like the tiers are, rather than in millimetres of
    /// world space — a change to the bot pipeline, not to these constants.
    #[test]
    fn a_bot_standing_on_a_cell_is_never_behind_it() {
        let tiers = [(MARKER_BIAS, "a marker"), (TILE_BIAS, "a tile"), (UNDERLAY_BIAS, "the underlay")];
        for elevation in REACHABLE_ELEVATIONS {
            for distance in [MIN_DISTANCE, 2.0, 5.0, UPWARD_SAFE_M, 20.0, 50.0, 200.0, 1000.0, 5000.0] {
                // The residual: looking up, close in, the 10mm of clearance is
                // worth more depth than the tier that answers it.
                if elevation < 0.0 && distance < UPWARD_SAFE_M {
                    continue;
                }
                let cam = camera_at(distance, elevation);
                let bot = depth(&cam, [50.0, 50.0, FLOOR_CLEARANCE_M], 0.0);
                let what = format!("at {distance}m, elevation {elevation}");
                for (bias, tier) in tiers {
                    assert!(bot <= depth(&cam, [50.0, 50.0, 0.0], bias), "{tier} is in front of a bot on it {what}");
                }
            }
        }
    }

    /// The residual is no wider than the doc comment above says.
    ///
    /// It measures the boundary rather than trusting the number: at the
    /// steepest upward tilt, the closest camera distance from which a bot
    /// stays in front of each tier. Written as an upper bound, so a smaller
    /// tier or a thinner clearance fails here — with the real boundary in the
    /// message — instead of quietly widening the band where a bot on a cell
    /// draws muted.
    #[test]
    fn the_upward_looking_residual_ends_where_the_tiers_say() {
        // Measured through these matrices on 2026-09-04, and quoted in
        // `a_bot_standing_on_a_cell_is_never_behind_it`.
        for (bias, tier, documented_m) in
            [(MARKER_BIAS, "a marker", 13.5_f32), (TILE_BIAS, "a tile", 9.6), (UNDERLAY_BIAS, "the underlay", 7.9)]
        {
            let mut boundary = f32::NAN;
            // Tenths of a metre, counted rather than accumulated, so the
            // boundary this reports is the grid point and not a drift off it.
            for tenths in (MIN_DISTANCE * 10.0) as u32..=300 {
                let distance = tenths as f32 / 10.0;
                let cam = camera_at(distance, MIN_ELEVATION);
                let bot = depth(&cam, [50.0, 50.0, FLOOR_CLEARANCE_M], 0.0);
                if bot <= depth(&cam, [50.0, 50.0, 0.0], bias) {
                    if boundary.is_nan() {
                        boundary = distance;
                    }
                } else {
                    boundary = f32::NAN;
                }
            }
            assert!(
                boundary <= documented_m,
                "looking up at {MIN_ELEVATION}, a bot only clears {tier} from {boundary}m -- the doc comment on \
                 a_bot_standing_on_a_cell_is_never_behind_it says {documented_m}m"
            );
            assert!(
                boundary < UPWARD_SAFE_M,
                "the residual for {tier} reaches {boundary}m, past the {UPWARD_SAFE_M}m the sweep above asserts from"
            );
        }
    }
}
