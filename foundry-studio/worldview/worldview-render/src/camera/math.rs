//! Within `camera`, `math` is THE ARITHMETIC ITSELF: the two projection
//! matrices, the look-at, and the small vector operations the orbit and the
//! pick ray are built out of.
//!
//! They are separate from the camera because both directions use them — what
//! the GPU is sent and what a pointer gesture is resolved against — and a
//! second copy of a projection would be a second answer. Clip space is wgpu's
//! (Z in [0, 1]), the layout is column-major, and there is no Y flip anywhere:
//! a world document renders north-up.

/// Orthographic projection matrix for wgpu clip space (Z in [0,1]).
/// Standard Y (no flip) — grid Y-flip is handled by flipping grid_y in instance data.
/// Column-major layout.
pub(super) fn orthographic(half_w: f32, half_h: f32, near: f32, far: f32) -> [[f32; 4]; 4] {
    let range_inv = 1.0 / (near - far);
    [
        [1.0 / half_w, 0.0, 0.0, 0.0],
        [0.0, 1.0 / half_h, 0.0, 0.0], // no Y-flip
        [0.0, 0.0, range_inv, 0.0],
        [0.0, 0.0, near * range_inv, 1.0],
    ]
}

/// Perspective projection matrix for wgpu clip space (Z in [0,1]).
/// Standard Y (no flip) — ground at bottom, sky at top for side views.
/// Column-major layout.
pub(super) fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> [[f32; 4]; 4] {
    let f = 1.0 / (fov_y * 0.5).tan();
    let range_inv = 1.0 / (near - far);
    [
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0], // no Y-flip (natural 3D orientation)
        [0.0, 0.0, far * range_inv, -1.0],
        [0.0, 0.0, near * far * range_inv, 0.0],
    ]
}

/// Choose an up-hint vector that avoids degeneracy at top-down (elevation ~ pi/2).
/// When looking straight down, world_up (0,0,1) is parallel to the view direction,
/// so we use an azimuth-derived horizontal vector instead.
/// At default azimuth (-pi/2) this gives (0, 1, 0) so that screen X = world X.
pub(super) fn up_for_elevation(azimuth: f32, elevation: f32) -> [f32; 3] {
    if elevation > std::f32::consts::FRAC_PI_2 - 0.01 {
        // At top-down, forward=(0,0,-1). We need right=(1,0,0) so screen X = world X.
        // right = cross(forward, up_hint), which requires up_hint.y=1, up_hint.x=0
        // at the default azimuth (-pi/2). General formula:
        [-azimuth.cos(), -azimuth.sin(), 0.0]
    } else {
        [0.0, 0.0, 1.0]
    }
}

/// Compute eye position from spherical coordinates around a target.
pub(super) fn spherical_to_eye(target: [f32; 3], azimuth: f32, elevation: f32, distance: f32) -> [f32; 3] {
    let cos_elev = elevation.cos();
    let sin_elev = elevation.sin();
    let cos_az = azimuth.cos();
    let sin_az = azimuth.sin();

    [
        target[0] + distance * cos_elev * cos_az,
        target[1] + distance * cos_elev * sin_az,
        target[2] + distance * sin_elev,
    ]
}

/// Look-at view matrix. Column-major.
pub(super) fn look_at(eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> [[f32; 4]; 4] {
    let f = normalize(sub(target, eye));
    let r = normalize(cross(f, up));
    let u = cross(r, f);

    [
        [r[0], u[0], -f[0], 0.0],
        [r[1], u[1], -f[1], 0.0],
        [r[2], u[2], -f[2], 0.0],
        [-dot(r, eye), -dot(u, eye), dot(f, eye), 1.0],
    ]
}

/// Multiply a 4x4 column-major matrix by a vec4.
pub(super) fn mat4_mul_vec4(m: [[f32; 4]; 4], v: [f32; 4]) -> [f32; 4] {
    [
        m[0][0] * v[0] + m[1][0] * v[1] + m[2][0] * v[2] + m[3][0] * v[3],
        m[0][1] * v[0] + m[1][1] * v[1] + m[2][1] * v[2] + m[3][1] * v[3],
        m[0][2] * v[0] + m[1][2] * v[1] + m[2][2] * v[2] + m[3][2] * v[3],
        m[0][3] * v[0] + m[1][3] * v[1] + m[2][3] * v[2] + m[3][3] * v[3],
    ]
}

// ---------------------------------------------------------------------------
// Vec3 helpers
// ---------------------------------------------------------------------------

pub(super) fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(super) fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(super) fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

pub(super) fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len < 1e-10 {
        return [0.0, 0.0, 0.0];
    }
    [v[0] / len, v[1] / len, v[2] / len]
}

pub(super) fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
