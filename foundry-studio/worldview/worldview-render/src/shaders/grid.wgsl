// The CELL shader: within this crate's shaders, this is the one that paints the
// floor the solver runs on. One instanced box per cell id -- a flat tile on a
// placed map, an extruded rack in State view -- carrying the cell's kind
// colour, its own boundary (the edge darkening below IS the grid), its
// reservation state and its heatmap intensity.
//
// Every cell draws (ruling of 2026-09-04). The same pipeline also draws the
// composed underlay beneath them -- steel, equipment strips, the building's
// ground -- as instances carrying a depth bias; see
// `renderer::cell_layer`.

// Height per rack level in world units (cell_size = 1.0 = ~1 meter).
const LEVEL_HEIGHT: f32 = 0.25;

// ── Camera uniform buffer ───────────────────────────────────────────────────

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
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) cell_kind: u32,
    @location(2) heatmap_vtm: f32,
    @location(3) heatmap_htm: f32,
    @location(4) @interpolate(flat) face_id: u32,
    @location(5) @interpolate(flat) rack_levels: u32,
    @location(6) @interpolate(flat) reserved: u32,
};

// ── Vertex shader ───────────────────────────────────────────────────────────

@vertex
fn vs_main(
    // Per-vertex: box vertex position (x, y, z_factor) and face identifier.
    @location(0) position: vec3<f32>,
    @location(1) face_id: u32,
    // Per-instance attributes.
    @location(2) min_xy: vec2<f32>,
    @location(3) size_xy: vec2<f32>,
    @location(4) cell_kind: u32,
    @location(5) rack_levels: u32,
    @location(6) heatmap_vtm: f32,
    @location(7) heatmap_htm: f32,
    @location(8) reserved: u32,
    @location(9) depth_bias: f32,
    @location(10) heading: f32,
) -> VertexOutput {
    // Only extrude racks in State view (view_mode == 0).
    let is_3d = camera.view_mode == 0u && rack_levels > 0u;
    // Total height = (rack_levels + 1) levels: open floor + rack_levels storage shelves.
    let rack_height = select(0.0, f32(rack_levels + 1u) * LEVEL_HEIGHT, is_3d);

    // The tile's own metre rect. Per-instance because a rack row and a
    // highway lane are genuinely different widths; `position.xy` is the unit
    // quad corner in [0,1].
    let local = (position.xy - vec2<f32>(0.5)) * size_xy;
    let c = cos(heading);
    let s = sin(heading);
    let rotated = vec2<f32>(c * local.x - s * local.y, s * local.x + c * local.y);
    let world_xy = min_xy + size_xy * 0.5 + rotated;

    // Side walls start one level up — the ground floor is open for HTMs.
    // Side bottom vertices (z_factor=0, face_id 1-4) → LEVEL_HEIGHT.
    // Floor face (face_id=5) stays at z=0.
    // Top face and side/floor top vertices (z_factor=1) → rack_height.
    var world_z: f32;
    if is_3d && face_id >= 1u && face_id <= 4u && position.z < 0.5 {
        world_z = LEVEL_HEIGHT;
    } else {
        world_z = position.z * rack_height;
    }

    var out: VertexOutput;
    out.clip_position = camera.projection * camera.view * vec4<f32>(world_xy, world_z, 1.0);
    // Separate the coplanar tiers in the units the DEPTH BUFFER has, not in
    // metres: multiplying by w cancels the perspective divide, so `depth_bias`
    // is the same slice of the [0,1] depth range at every camera distance and
    // under both of the camera's projections. A metre offset is not -- the
    // perspective camera crushes a millimetre to nothing past about 60m, and
    // the underlay then fights the cells it stands under. See
    // `cell_layer::DEPTH_TIER`.
    //
    // ONLY the ground surface is biased. The tiers exist to order surfaces
    // that are coplanar at z=0, and nothing else is: a rack's walls start a
    // level up and its top is metres up, so pushing them back a tier buys
    // nothing and costs real occlusion -- a tier is worth about 0.8m of world
    // separation at a 200m camera, so a bot or tote just behind a rack wall
    // passed its later `LessEqual` test and drew IN FRONT of the rack. The
    // mesh's z_factor is exactly 0 or 1, so this comparison is exact, and it
    // keeps the extrusion's own floor face (face_id 5, always z=0) on its
    // tier along with every flat tile.
    let on_ground = world_z == 0.0;
    out.clip_position.z += select(0.0, depth_bias, on_ground) * out.clip_position.w;

    // UV: top/floor faces use position.xy (for grid lines).
    // Side faces: x = width along face, y = z_factor (for level lines).
    if face_id == 0u || face_id == 5u {
        out.uv = position.xy;
    } else if face_id <= 2u {
        out.uv = vec2<f32>(position.x, position.z);
    } else {
        out.uv = vec2<f32>(position.y, position.z);
    }

    out.cell_kind = cell_kind;
    out.face_id = face_id;
    out.rack_levels = rack_levels;
    out.heatmap_vtm = heatmap_vtm;
    out.heatmap_htm = heatmap_htm;
    out.reserved = reserved;
    return out;
}

// ── Cell color palette ──────────────────────────────────────────────────────

fn cell_color(kind: u32) -> vec3<f32> {
    switch kind {
        case 0u: { return vec3<f32>(0.529, 0.808, 0.980); } // Highway
        case 1u: { return vec3<f32>(0.706, 0.863, 1.000); } // Aisle
        case 2u: { return vec3<f32>(0.851, 0.851, 0.851); } // Open
        case 3u: { return vec3<f32>(0.929, 0.788, 0.278); } // Storage
        case 4u: { return vec3<f32>(0.400, 0.780, 0.471); } // Relay
        case 5u: { return vec3<f32>(0.690, 0.490, 0.831); } // StationArea
        case 6u: { return vec3<f32>(0.902, 0.302, 0.302); } // Junction
        case 7u: { return vec3<f32>(1.000, 1.000, 1.000); } // Blank
        case 8u: { return vec3<f32>(1.000, 0.549, 0.000); } // Presentation (orange)
        case 9u: { return vec3<f32>(1.000, 0.784, 0.392); } // QueueCell (amber)
        case 11u: { return vec3<f32>(0.529, 0.808, 0.980); } // Charging (soft blue)
        case 12u: { return vec3<f32>(0.180, 0.545, 0.341); } // Exchange (sea green): belt handoff cells
        default: { return vec3<f32>(0.149, 0.149, 0.149); } // None / empty
    }
}

fn cell_color_heatmap(kind: u32) -> vec3<f32> {
    switch kind {
        case 4u: { return vec3<f32>(0.902, 0.961, 0.910); } // Relay — pale green
        case 5u: { return vec3<f32>(0.929, 0.890, 0.961); } // StationArea — pale purple
        case 12u: { return vec3<f32>(0.878, 0.941, 0.902); } // Exchange — pale sea green
        default: { return vec3<f32>(1.000, 1.000, 1.000); } // Everything else — white
    }
}

// ── Fragment shader ─────────────────────────────────────────────────────────

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let heatmap_active = camera.view_mode >= 5u;
    let in_sim = camera.view_mode == 4u || heatmap_active;

    var color: vec3<f32>;
    if camera.view_mode >= 6u {
        color = vec3<f32>(0.80, 0.80, 0.80); // No exposure or excluded cell.
    } else if heatmap_active {
        color = cell_color_heatmap(in.cell_kind);
    } else {
        color = cell_color(in.cell_kind);
    }

    // Hoist fwidth() calls before non-uniform branches (face_id, cell_kind).
    // fwidth depends only on screen-space derivatives of UV, not on which
    // branch we're in, so the result is identical.
    let pw_uv_x = fwidth(in.uv.x);
    let pw_uv_y = fwidth(in.uv.y);

    // EVERY CELL DRAWS. Ruling of 2026-09-04: "All cells should be drawn.
    // entities bearing `region` components need not necessarily be drawn."
    // Open floor is a cell like any other and paints its own grey and its own
    // boundary here; only a REGION -- a rule, not a thing (ruling of
    // 2026-08-22) -- paints nothing, and it paints nothing on the CPU side,
    // where `cell_layer::compose_floor_instances` has no parameter to hand
    // one to.
    //
    // Open cells were discarded here between 2026-08-28 and 2026-09-04, to
    // stop them fighting the underlay beneath them. That hid one class of
    // fragment without fixing the stacking that made them fight; the stacking
    // is fixed at the source, in `cell_layer::DEPTH_TIER` and the `depth_bias`
    // the vertex shader applies above.

    // ── Floor face (visible through open rack bottom) ────────────────
    if in.face_id == 5u {
        // Distinct floor color per rack type and reservation state.
        if in.cell_kind == 4u {
            if in.reserved > 0u {
                color = vec3<f32>(0.95, 0.55, 0.35); // Reserved relay floor: warm orange
            } else {
                color = vec3<f32>(0.70, 0.85, 0.75); // Free relay floor: cool green-gray
            }
        } else {
            color = vec3<f32>(0.85, 0.80, 0.70); // Storage floor: warm beige
        }
        // Grid lines on floor.
        let edge_dist = min(min(in.uv.x, 1.0 - in.uv.x), min(in.uv.y, 1.0 - in.uv.y));
        let lw = max(pw_uv_x * 1.5, 0.02);
        let lf = 1.0 - smoothstep(0.0, lw, edge_dist);
        color = mix(color, color * 0.65, lf);
        return vec4<f32>(color, 1.0);
    }

    // ── Side faces (3D rack walls) ───────────────────────────────────
    if in.face_id >= 1u && in.face_id <= 4u {
        // Darken side faces for depth cue.
        color *= 0.70;

        // Horizontal level lines (rack_levels intervals = storage shelves).
        if in.rack_levels > 1u {
            let level_uv = in.uv.y * f32(in.rack_levels);
            let level_dist = min(fract(level_uv), 1.0 - fract(level_uv));
            let lw = max(pw_uv_y * f32(in.rack_levels) * 1.5, 0.02);
            let level_line = 1.0 - smoothstep(0.0, lw, level_dist);
            color = mix(color, color * 0.55, level_line);
        }

        // Vertical edge line at face boundary.
        let edge_x = min(in.uv.x, 1.0 - in.uv.x);
        let ew = max(pw_uv_x * 1.5, 0.02);
        let edge_line = 1.0 - smoothstep(0.0, ew, edge_x);
        color = mix(color, color * 0.55, edge_line);

        return vec4<f32>(color, 0.45);
    }

    // ── Top face (existing grid line + heatmap logic) ────────────────

    // Override relay cell color when reserved/occupied.
    if camera.view_mode < 6u && in.cell_kind == 4u && in.reserved > 0u {
        color = vec3<f32>(0.95, 0.55, 0.35); // Reserved relay: warm orange
    }

    let edge_dist = min(min(in.uv.x, 1.0 - in.uv.x), min(in.uv.y, 1.0 - in.uv.y));
    let line_width = max(pw_uv_x * 1.5, 0.02);
    let line_factor = 1.0 - smoothstep(0.0, line_width, edge_dist);

    if in.cell_kind != 7u {
        if in_sim {
            color = mix(color, vec3<f32>(0.902, 0.902, 0.902), line_factor);
        } else if in.cell_kind == 3u {
            color = mix(color, color * 0.65, line_factor);
        } else {
            color = mix(color, color * 0.80, line_factor);
        }
    }

    // ── Heatmap overlay (Simulation + heatmap active only) ───────────
    if camera.view_mode == 6u || camera.view_mode == 7u {
        if in.heatmap_htm >= 0.0 {
            let value = clamp(in.heatmap_htm, 0.0, 1.0);
            if camera.view_mode == 6u {
                color = mix(vec3<f32>(0.98, 0.98, 0.98), vec3<f32>(0.84, 0.15, 0.10), value);
            } else {
                color = mix(vec3<f32>(0.96, 0.53, 0.16), vec3<f32>(0.12, 0.40, 0.82), value);
            }
        }
        color = mix(color, vec3<f32>(0.65, 0.65, 0.65), line_factor);
    } else if heatmap_active {
        let vi = sqrt(max(in.heatmap_vtm, 0.0));
        let hi = sqrt(max(in.heatmap_htm, 0.0));

        if vi > 0.0 || hi > 0.0 {
            let center_offset = in.uv - vec2<f32>(0.5, 0.5);
            let dist = length(center_offset);

            if dist < 0.42 {
                let vtm_color = vec3<f32>(0.10, 0.30, 0.95);
                let htm_color = vec3<f32>(0.92, 0.15, 0.15);
                let total = vi + hi;
                let heat_color = (vtm_color * vi + htm_color * hi) / total;
                let alpha = max(vi, hi);
                color = heat_color * alpha + color * (1.0 - alpha);
            }
        }
    }

    return vec4<f32>(color, 1.0);
}
