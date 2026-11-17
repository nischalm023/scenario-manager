// Rack bay-frame and station-prop shader for warehouse visualization.
// Renders instanced boxes built CPU-side: the map's physical bay geometry
// (uprights, beams, slot dividers, diving-board tabs), which is axis-aligned,
// and station props (see easyloop_prop.rs), which carry a yaw about the box's
// own XY centre so a conveyor can turn a corner and a carrier can be
// presented at 45 degrees.

// ── Camera uniform buffer (shared with grid/bot/tote shaders) ───────────────

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
    @location(0) local_pos: vec3<f32>,
    @location(1) @interpolate(flat) kind: u32,
};

// ── Vertex shader ───────────────────────────────────────────────────────────

@vertex
fn vs_main(
    // Per-vertex: unit cube [0..1]^3
    @location(0) position: vec3<f32>,
    // Per-instance attributes: box min corner, box size, element kind, yaw.
    @location(1) box_min: vec3<f32>,
    @location(2) box_size: vec3<f32>,
    @location(3) kind: u32,
    @location(4) yaw: f32,
) -> VertexOutput {

    // All three axes are metres now, so the box needs no rescaling — the
    // XY-in-cell-units / Z-in-world-units split is gone.
    let unrotated = vec3<f32>(
        box_min.x + position.x * box_size.x,
        box_min.y + position.y * box_size.y,
        box_min.z + position.z * box_size.z,
    );

    // Yaw turns the box about its OWN XY centre, so an instance's min/size
    // still describe where it is; only its footprint rotates. Rack steel
    // passes yaw = 0 and takes the identity through here.
    let centre = box_min.xy + box_size.xy * 0.5;
    let c = cos(yaw);
    let s = sin(yaw);
    let d = unrotated.xy - centre;
    let world_pos = vec3<f32>(
        centre.x + d.x * c - d.y * s,
        centre.y + d.x * s + d.y * c,
        unrotated.z,
    );

    var out: VertexOutput;
    out.clip_position = camera.projection * camera.view * vec4<f32>(world_pos, 1.0);
    out.local_pos = position;
    out.kind = kind;
    return out;
}

// ── Fragment shader ─────────────────────────────────────────────────────────

fn kind_color(kind: u32) -> vec3<f32> {
    switch kind {
        case 0u: { return vec3<f32>(0.40, 0.43, 0.48); } // Upright: dark steel
        case 1u: { return vec3<f32>(0.56, 0.60, 0.66); } // Beam: mid steel
        case 2u: { return vec3<f32>(0.70, 0.72, 0.76); } // Divider: light steel
        case 3u: { return vec3<f32>(0.30, 0.55, 0.38); } // BoardTab: relay green
        // Station prop. Machinery greys so the two carrier colours and the
        // operator are the only things that read as figures in the scene.
        case 4u: { return vec3<f32>(0.52, 0.55, 0.60); } // ConveyorDeck
        case 5u: { return vec3<f32>(0.76, 0.79, 0.83); } // ConveyorGuard
        case 6u: { return vec3<f32>(0.33, 0.36, 0.40); } // ConveyorLeg
        case 7u: { return vec3<f32>(0.46, 0.49, 0.54); } // ConveyorRamp
        case 8u: { return vec3<f32>(0.42, 0.46, 0.51); } // ConveyorTransfer
        case 9u: { return vec3<f32>(0.58, 0.62, 0.67); } // PresentDeck
        case 10u: { return vec3<f32>(0.68, 0.72, 0.78); } // PresentChute
        case 11u: { return vec3<f32>(0.40, 0.44, 0.49); } // PresentStand
        case 12u: { return vec3<f32>(0.20, 0.47, 0.72); } // InventoryTote: blue
        case 13u: { return vec3<f32>(0.85, 0.52, 0.17); } // OrderTote: amber
        case 14u: { return vec3<f32>(0.28, 0.31, 0.35); } // OperatorPlatform
        case 15u: { return vec3<f32>(0.16, 0.49, 0.36); } // Operator: green
        case 16u: { return vec3<f32>(0.75, 0.23, 0.19); } // PickToLight: red
        case 17u: { return vec3<f32>(0.64, 0.67, 0.72); } // ConveyorRoller
        default: { return vec3<f32>(0.56, 0.60, 0.66); }
    }
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var color = kind_color(in.kind);

    // Simple top-face highlight / side darkening for depth perception.
    if in.local_pos.z > 0.99 {
        color *= 1.15;
    }
    let is_side = in.local_pos.z > 0.01 && in.local_pos.z < 0.99;
    if is_side {
        color *= 0.85;
    }

    return vec4<f32>(color, 1.0);
}
