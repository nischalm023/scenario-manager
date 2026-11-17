//! Within `camera`, `input` is WHAT A GESTURE DOES TO THE VIEW: scroll zoom,
//! drag pan, orbit, the ground skim that keeps the eye above the floor, and
//! the key that resets it.
//!
//! WAIVER: handle_input is a single ~240-line method with tightly coupled
//! interaction logic (scroll zoom, drag pan, orbit, ground skim). Splitting
//! into smaller methods would obscure the sequential dependencies between
//! input phases. Keeping as one cohesive method for clarity.

use super::math::*;
use super::{
    CameraState, MAX_DISTANCE, MAX_ELEVATION, MIN_DISTANCE, MIN_ELEVATION, MIN_EYE_Z, ORBIT_SENSITIVITY,
    PERSPECTIVE_ELEVATION, SCROLL_ZOOM_SENSITIVITY, Viewport,
};

/// One frame of pointer and keyboard state, as the host saw it — the only
/// thing the camera reads. A viewer fills it in from whatever toolkit it draws
/// with, which is how the same gesture means the same thing in a window and in
/// a browser tab.
#[derive(Clone, Copy, Debug, Default)]
pub struct PointerInput {
    pub viewport: Viewport,
    pub hovered: bool,
    /// Where the pointer is, when it is over the viewport at all.
    pub hover_pos: Option<[f32; 2]>,
    pub dragged: bool,
    pub drag_delta: [f32; 2],
    /// Smoothed scroll since the last frame. The x component is read only when
    /// y is zero, for the platforms that turn a shifted wheel sideways.
    pub scroll_delta: [f32; 2],
    pub shift: bool,
    pub escape_pressed: bool,
}

impl CameraState {
    /// Process scroll, drag, double-click, and ESC input: primary-drag pans,
    /// shift+primary-drag orbits (when `allow_orbit`), scroll zooms, ESC resets.
    pub fn handle_input(&mut self, input: &PointerInput, allow_orbit: bool) {
        // Update stored aspect ratio from viewport.
        let viewport = input.viewport;
        if viewport.height() > 0.0 {
            self.aspect = viewport.width() / viewport.height();
            if self.needs_fit {
                self.distance = Self::fit_distance(self.frame.extent().0, self.frame.extent().1, self.aspect);
                self.needs_fit = false;
            }
        }

        // Cancel animation on drag or scroll (but NOT double-click — that starts animations).
        let cancel_anim = input.dragged || (input.hovered && input.scroll_delta[1] != 0.0);
        if cancel_anim {
            self.anim = None;
        }

        // Scroll = dolly (change distance). Only when hovering the canvas.
        // Use vertical scroll, but if shift converts it to horizontal
        // (macOS/some platforms), use horizontal instead.
        let scroll_y = if input.hovered {
            let v = input.scroll_delta[1];
            if v == 0.0 { input.scroll_delta[0] } else { v }
        } else {
            0.0
        };
        let shift_held = input.shift;
        if scroll_y != 0.0 {
            let old_distance = self.distance;
            let factor = (-scroll_y * SCROLL_ZOOM_SENSITIVITY).exp();
            let desired = self.distance * factor;

            // Compute cursor's world position on the z=0 ground plane via ray-cast.
            let cursor_ground = input.hover_pos.and_then(|hp| self.cursor_to_ground(hp, viewport));

            if shift_held {
                // Shift+scroll: slide toward/away from cursor at current elevation.
                let slide = old_distance - desired;
                if let Some(ground_pt) = cursor_ground {
                    // Direction from orbit_target toward cursor ground point.
                    let dx = ground_pt[0] - self.orbit_target[0];
                    let dy = ground_pt[1] - self.orbit_target[1];
                    let len = (dx * dx + dy * dy).sqrt();
                    if len > 0.01 {
                        self.orbit_target[0] += (dx / len) * slide;
                        self.orbit_target[1] += (dy / len) * slide;
                    } else {
                        self.orbit_target[0] += -self.azimuth.cos() * slide;
                        self.orbit_target[1] += -self.azimuth.sin() * slide;
                    }
                } else {
                    self.orbit_target[0] += -self.azimuth.cos() * (old_distance - desired);
                    self.orbit_target[1] += -self.azimuth.sin() * (old_distance - desired);
                }
            } else {
                self.distance = desired.clamp(MIN_DISTANCE, MAX_DISTANCE);

                // If distance hit MIN_DISTANCE and user is still zooming in,
                // convert excess zoom into forward slide along the ground.
                if scroll_y > 0.0 && desired < MIN_DISTANCE {
                    let excess = MIN_DISTANCE - desired;
                    self.orbit_target[0] += -self.azimuth.cos() * excess;
                    self.orbit_target[1] += -self.azimuth.sin() * excess;
                }

                // Cursor-centered zoom: shift orbit_target toward the cursor's
                // ground point so the world point under the cursor stays fixed.
                if let Some(ground_pt) = cursor_ground {
                    let t = 1.0 - self.distance / old_distance;
                    self.orbit_target[0] += (ground_pt[0] - self.orbit_target[0]) * t;
                    self.orbit_target[1] += (ground_pt[1] - self.orbit_target[1]) * t;
                }
            }

            // Incrementally rotate azimuth toward the cursor ground point
            // as we zoom in, so the camera naturally faces where we're going.
            if shift_held && scroll_y > 0.0 && self.elevation < PERSPECTIVE_ELEVATION {
                if let Some(ground_pt) = cursor_ground {
                    let dx = ground_pt[0] - self.orbit_target[0];
                    let dy = ground_pt[1] - self.orbit_target[1];
                    if dx * dx + dy * dy > 1.0 {
                        // Target azimuth: eye should be on opposite side of
                        // orbit_target from the cursor ground point.
                        let target_az = dy.atan2(dx) + std::f32::consts::PI;
                        // Shortest angular difference.
                        let mut diff = target_az - self.azimuth;
                        while diff > std::f32::consts::PI {
                            diff -= 2.0 * std::f32::consts::PI;
                        }
                        while diff < -std::f32::consts::PI {
                            diff += 2.0 * std::f32::consts::PI;
                        }
                        // Gentle nudge — 3% per scroll tick.
                        self.azimuth += diff * 0.03;
                    }
                }
            }

            self.clamp_orbit_target();
        }

        if input.dragged {
            let d = input.drag_delta;
            let shift = input.shift;
            let do_orbit = shift && allow_orbit;
            let do_pan = !do_orbit;
            if do_orbit {
                // Orbit around the world point under the cursor.
                // Strategy: compute where the pivot sits before and after the
                // azimuth/elevation change, then shift orbit_target to compensate
                // so the pivot stays fixed on screen.

                // 1. Compute pivot (world point under cursor on z=0 plane).
                let old_eye = spherical_to_eye(self.orbit_target, self.azimuth, self.elevation, self.distance);
                let fwd = normalize(sub(self.orbit_target, old_eye));
                let up_hint = up_for_elevation(self.azimuth, self.elevation);
                let rt = normalize(cross(fwd, up_hint));
                let u = cross(rt, fwd);
                let aspect = viewport.width() / viewport.height();
                let hh = self.distance * 0.5;
                let hw = hh * aspect;

                let pivot = if let Some(hp) = input.hover_pos {
                    let nx = ((hp[0] - viewport.min[0]) / viewport.width()) * 2.0 - 1.0;
                    let ny = 1.0 - ((hp[1] - viewport.min[1]) / viewport.height()) * 2.0;
                    [
                        self.orbit_target[0] + nx * hw * rt[0] + ny * hh * u[0],
                        self.orbit_target[1] + nx * hw * rt[1] + ny * hh * u[1],
                        0.0,
                    ]
                } else {
                    self.orbit_target
                };

                // 2. Apply rotation. Flip azimuth direction when cursor is
                // above viewport center so it feels like pushing the surface.
                let above_center = input.hover_pos.is_some_and(|hp| hp[1] < viewport.centre()[1]);
                let az_sign = if above_center { 1.0 } else { -1.0 };
                self.azimuth += az_sign * d[0] * ORBIT_SENSITIVITY;
                self.elevation = (self.elevation + d[1] * ORBIT_SENSITIVITY).clamp(MIN_ELEVATION, MAX_ELEVATION);

                // 3. Where would the pivot appear now (with old orbit_target)?
                //    Compute new camera basis and project pivot into new view.
                let new_eye = spherical_to_eye(self.orbit_target, self.azimuth, self.elevation, self.distance);
                let new_fwd = normalize(sub(self.orbit_target, new_eye));
                let new_up_hint = up_for_elevation(self.azimuth, self.elevation);
                let new_rt = normalize(cross(new_fwd, new_up_hint));
                let new_u = cross(new_rt, new_fwd);
                let new_hh = self.distance * 0.5;
                let new_hw = new_hh * aspect;

                // Pivot's offset from orbit_target in new camera space (NDC).
                let diff = sub(pivot, self.orbit_target);
                let new_ndc_x = if new_hw > 0.0 {
                    (diff[0] * new_rt[0] + diff[1] * new_rt[1] + diff[2] * new_rt[2]) / new_hw
                } else {
                    0.0
                };
                let new_ndc_y = if new_hh > 0.0 {
                    (diff[0] * new_u[0] + diff[1] * new_u[1] + diff[2] * new_u[2]) / new_hh
                } else {
                    0.0
                };

                // Pivot's original NDC position (from step 1).
                let old_diff = sub(pivot, self.orbit_target);
                let old_ndc_x =
                    if hw > 0.0 { (old_diff[0] * rt[0] + old_diff[1] * rt[1] + old_diff[2] * rt[2]) / hw } else { 0.0 };
                let old_ndc_y =
                    if hh > 0.0 { (old_diff[0] * u[0] + old_diff[1] * u[1] + old_diff[2] * u[2]) / hh } else { 0.0 };

                // 4. Shift orbit_target so pivot stays at its original screen position.
                let dndc_x = old_ndc_x - new_ndc_x;
                let dndc_y = old_ndc_y - new_ndc_y;
                self.orbit_target[0] += dndc_x * new_hw * new_rt[0] + dndc_y * new_hh * new_u[0];
                self.orbit_target[1] += dndc_x * new_hw * new_rt[1] + dndc_y * new_hh * new_u[1];
                // Don't shift z — keep orbit_target on the ground plane so
                // zoom distance stays consistent after orbiting.
                self.clamp_orbit_target();
            } else if do_pan {
                self.apply_pan(d, viewport.height());
            }
        }

        // Ground skim: after all input, ensure eye stays above MIN_EYE_Z.
        // If below, raise distance (keeping eye at MIN_EYE_Z) and convert
        // the excess into forward sliding along the ground.
        let eye = spherical_to_eye(self.orbit_target, self.azimuth, self.elevation, self.distance);
        if eye[2] < MIN_EYE_Z {
            let sin_el = self.elevation.sin();
            if sin_el.abs() > 0.001 {
                let needed_dist = (MIN_EYE_Z - self.orbit_target[2]) / sin_el;
                if needed_dist > 0.0 {
                    let slide = self.distance - needed_dist;
                    let fwd_x = -self.azimuth.cos();
                    let fwd_y = -self.azimuth.sin();
                    self.orbit_target[0] += fwd_x * slide;
                    self.orbit_target[1] += fwd_y * slide;
                    self.distance = needed_dist.max(MIN_DISTANCE);
                }
            }
            // Fallback: just lift orbit_target if math didn't fix it.
            let eye2 = spherical_to_eye(self.orbit_target, self.azimuth, self.elevation, self.distance);
            if eye2[2] < MIN_EYE_Z {
                self.orbit_target[2] += MIN_EYE_Z - eye2[2];
            }
        }

        // ESC = reset to top-down. Double-click handled by app.rs (fly-to-cell).
        if input.escape_pressed {
            self.reset();
        }
    }

    /// Pan by dragging the ground plane under the cursor: translate the orbit
    /// target along the camera's right/up basis. `d` is the screen-space drag
    /// delta in pixels; `viewport_height` scales pixels to world units.
    fn apply_pan(&mut self, d: [f32; 2], viewport_height: f32) {
        let eye = spherical_to_eye(self.orbit_target, self.azimuth, self.elevation, self.distance);
        let forward = normalize(sub(self.orbit_target, eye));
        let up_hint = up_for_elevation(self.azimuth, self.elevation);
        let right = normalize(cross(forward, up_hint));
        let up = cross(right, forward);
        // Ortho: full visible height = distance, so each drag pixel maps to
        // distance / viewport_height world units.
        let scale = if viewport_height > 0.0 { self.distance / viewport_height } else { 0.0 };
        // Screen right = world right (subtract for grab-and-drag); screen down =
        // world -up (add up for grab-and-drag).
        self.orbit_target[0] -= (d[0] * right[0] - d[1] * up[0]) * scale;
        self.orbit_target[1] -= (d[0] * right[1] - d[1] * up[1]) * scale;
        self.orbit_target[2] -= (d[0] * right[2] - d[1] * up[2]) * scale;
        self.clamp_orbit_target();
    }
}
