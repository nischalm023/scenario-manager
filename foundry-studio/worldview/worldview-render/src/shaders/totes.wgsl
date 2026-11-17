// Opaque open shells, outward/downward carton flaps and physical unit cubes.
struct CameraUniforms {
    projection: mat4x4<f32>,
    view: mat4x4<f32>,
    viewport_size: vec2<f32>,
    cell_size: f32,
    view_mode: u32,
};

@group(0) @binding(0)
var<uniform> camera: CameraUniforms;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
};

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) part: u32,
    @location(3) origin: vec3<f32>,
    @location(4) heading: f32,
    @location(5) scale: vec3<f32>,
    @location(6) color: vec3<f32>,
    @location(7) physical_type: u32,
    @location(8) pitch: f32,
) -> VertexOutput {
    var local = position * scale;
    if part == 1u && physical_type != 2u {
        // Only cartons have flaps; collapse those triangles for plastic shells.
        local = vec3<f32>(0.0);
    }
    let c = cos(heading);
    let s = sin(heading);
    let cp = cos(pitch);
    let sp = sin(pitch);
    local = vec3<f32>(local.x, local.y * cp - local.z * sp, local.y * sp + local.z * cp);
    let xy = vec2<f32>(local.x * c - local.y * s, local.x * s + local.y * c);
    let scaled_normal = normalize(normal / scale);
    let transformed_normal = vec3<f32>(
        scaled_normal.x, scaled_normal.y * cp - scaled_normal.z * sp, scaled_normal.y * sp + scaled_normal.z * cp,
    );
    var out: VertexOutput;
    out.clip_position = camera.projection * camera.view * vec4<f32>(origin + vec3<f32>(xy, local.z), 1.0);
    out.normal = vec3<f32>(
        transformed_normal.x * c - transformed_normal.y * s,
        transformed_normal.x * s + transformed_normal.y * c,
        transformed_normal.z,
    );
    out.color = color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let light = normalize(vec3<f32>(-0.4, -0.6, 1.0));
    let shade = 0.64 + 0.42 * max(dot(normalize(in.normal), light), 0.0);
    return vec4<f32>(in.color * shade, 1.0);
}
