// Bot marker shader for warehouse visualization.
// HTMs render the baked proxy of the real machine (see gpu/renderer/htm_mesh);
// VTMs and RTPs stay instanced cylinders, VTMs as thin slabs so a carried tote
// rides flush. Every glyph spins about +z by its instance heading, which is
// what makes an HTM's stop-and-rotate visible.

const VTM_HEIGHT: f32 = 0.02;
const HTM_HEIGHT: f32 = 0.12;

// Vertex face_type: which reading of `vertex_pos` and which shading applies.
const FACE_TOP: u32 = 0u;
const FACE_SIDE: u32 = 1u;
const FACE_BOTTOM: u32 = 2u;
const FACE_MESH: u32 = 3u;

// ── Camera uniform buffer (shared with grid shader) ─────────────────────────

struct CameraUniforms {
    projection: mat4x4<f32>,
    view: mat4x4<f32>,
    viewport_size: vec2<f32>,
    cell_size: f32,
    view_mode: u32,
};

@group(0) @binding(0)
var<uniform> camera: CameraUniforms;

// ── Vertex / Fragment IO ────────────────────────────────────────────────────

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) local_uv: vec2<f32>,
    @location(1) @interpolate(flat) bot_type: u32,
    @location(2) @interpolate(flat) face_type: u32,
    @location(3) @interpolate(flat) highlight: u32,
    @location(4) normal: vec3<f32>,
    @location(5) paint: vec3<f32>,
};

// ── Vertex shader ───────────────────────────────────────────────────────────

/// Turn a bot-local xy offset by the bot's heading, counter-clockwise about +z.
fn spin(v: vec2<f32>, heading: f32) -> vec2<f32> {
    let c = cos(heading);
    let s = sin(heading);
    return vec2<f32>(v.x * c - v.y * s, v.x * s + v.y * c);
}

@vertex
fn vs_main(
    // Per-vertex: mesh vertices carry metres; cylinder vertices carry
    // (cos, sin) on the unit circle plus a 0/1 z factor.
    @location(0) vertex_pos: vec3<f32>,
    @location(6) face_type: u32,
    @location(7) vertex_normal: vec3<f32>,
    @location(9) vertex_paint: vec4<f32>,
    // Per-instance attributes
    @location(1) world_x: f32,
    @location(2) world_y: f32,
    @location(3) world_z: f32,
    @location(4) bot_type: u32,
    @location(5) highlight: u32,
    @location(8) heading: f32,
) -> VertexOutput {
    // Selected bots render slightly larger so the highlight reads at a glance.
    let scale = select(1.0, 1.35, highlight == 1u);

    var local: vec3<f32>;
    if face_type == FACE_MESH {
        // Mesh vertices carry metres, and the highlight grows the whole body
        // rather than stretching its footprint.
        local = vertex_pos * scale;
    } else {
        // Physical radius in metres. The M5F chassis is 460 x 695mm, so a
        // ~0.35m disc is the turn envelope rather than a fraction of a cell.
        let radius = 0.35 * scale;
        let height = select(VTM_HEIGHT, HTM_HEIGHT, bot_type == 1u);
        local = vec3<f32>(vertex_pos.xy * radius, vertex_pos.z * height);
    }

    // Instance positions are world metres already; nothing scales by cell.
    let world_pos = vec2<f32>(world_x, world_y) + spin(local.xy, heading);

    var out: VertexOutput;
    out.clip_position = camera.projection * camera.view * vec4<f32>(world_pos, world_z + local.z, 1.0);
    out.local_uv = vertex_pos.xy;
    out.bot_type = bot_type;
    out.face_type = face_type;
    out.highlight = highlight;
    out.normal = vec3<f32>(spin(vertex_normal.xy, heading), vertex_normal.z);
    out.paint = vertex_paint.rgb;
    return out;
}

// ── Highlight tint ──────────────────────────────────────────────────────────
// Blend the base color toward a highlight color. Primary (1) = warm yellow,
// strong; secondary (2) = cyan, lighter. None (0) = unchanged.
fn apply_highlight(color: vec3<f32>, highlight: u32) -> vec3<f32> {
    if highlight == 1u {
        return mix(color, vec3<f32>(1.0, 0.85, 0.1), 0.6);
    }
    if highlight == 2u {
        return mix(color, vec3<f32>(0.2, 0.9, 1.0), 0.45);
    }
    return color;
}

// ── Bot color palette ───────────────────────────────────────────────────────
// One colour per bot type, for glyphs that have no paint of their own. The HTM
// entry is reached only when the baked mesh failed to parse and HTMs fall back
// to the cylinder.

fn bot_color(bt: u32) -> vec3<f32> {
    switch bt {
        case 0u: { return vec3<f32>(0.196, 0.392, 1.000); } // VTM: rgb(50, 100, 255)
        case 1u: { return vec3<f32>(1.000, 0.392, 0.196); } // HTM: rgb(255, 100, 50)
        default: { return vec3<f32>(0.392, 0.784, 0.392); } // RTP: rgb(100, 200, 100)
    }
}

/// What the body is painted before the highlight tint: the machine's own
/// vendor colours where the mesh carries them, the bot-type colour otherwise.
fn body_color(in: VertexOutput) -> vec3<f32> {
    let base = select(bot_color(in.bot_type), in.paint, in.face_type == FACE_MESH);
    return apply_highlight(base, in.highlight);
}

// ── Proxy mesh shading ──────────────────────────────────────────────────────
// A key light fixed in world space. The mesh's normals spin with the bot, so
// its faces brighten and darken as it turns: the shading reports the rotation
// even at zoom levels where the silhouette alone would be too small to read.
const KEY_LIGHT: vec3<f32> = vec3<f32>(0.35, -0.55, 0.76);

fn shade_mesh(color: vec3<f32>, normal: vec3<f32>) -> vec3<f32> {
    // Wrapped rather than clamped lambert: the mesh draws with culling off and
    // its baked normals only inherit the vendor model's winding, so a face
    // pointing away dims instead of going black.
    let key = clamp(0.5 + 0.5 * dot(normalize(normal), KEY_LIGHT), 0.0, 1.0);
    return color * (0.45 + 0.7 * key);
}

// ── Fragment shader ─────────────────────────────────────────────────────────

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = body_color(in);

    if in.face_type == FACE_MESH {
        return vec4<f32>(shade_mesh(color, in.normal), 1.0);
    }

    // Top face: circle with soft edge.
    if in.face_type == FACE_TOP {
        let dist = length(in.local_uv);
        if dist > 1.0 { discard; }
        let alpha = 1.0 - smoothstep(0.9, 1.0, dist);
        return vec4<f32>(color, alpha);
    }

    // Side face: slightly darkened.
    if in.face_type == FACE_SIDE {
        return vec4<f32>(color * 0.75, 1.0);
    }

    // Bottom face: darkened.
    let dist = length(in.local_uv);
    if dist > 1.0 { discard; }
    return vec4<f32>(color * 0.6, 1.0);
}

// ── Occluded bot fragment shader (behind racks, muted) ─────────────────────

@fragment
fn fs_occluded(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = body_color(in);

    if in.face_type == FACE_TOP || in.face_type == FACE_BOTTOM {
        let dist = length(in.local_uv);
        if dist > 1.0 { discard; }
    }

    // Highlighted bots stay legible through racks — a higher occluded alpha
    // so the selection/inbound tint reads even when behind geometry.
    let alpha = select(0.25, 0.55, in.highlight != 0u);
    return vec4<f32>(color, alpha);
}
