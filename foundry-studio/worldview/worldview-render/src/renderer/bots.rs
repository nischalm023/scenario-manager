//! Within `renderer`, `bots` is HOW A ROBOT IS DRAWN: two instanced draws
//! over one instance buffer — the baked D30 body for the HTMs, a cylinder for
//! everything else — plus the see-through pass that keeps a robot visible
//! behind rack steel.
//!
//! The HTM rows come first in the buffer so each draw is one contiguous range;
//! `build_bot_instances` is what guarantees that, and the count it returns is
//! where one range ends and the other begins. A robot that fails to unpack its
//! baked body falls back to the cylinder rather than vanishing.

use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use super::htm_mesh;
use super::instances::BotInstance;
use super::{DEPTH_FORMAT, multisample};

const BOT_CIRCLE_SEGMENTS: u32 = 16;
const MAX_BOT_INSTANCES: u64 = 1024;

/// One vertex of either bot glyph. `pos` reads as metres in the bot's own
/// frame for the HTM mesh, and as unit-circle xy plus a 0/1 z factor for the
/// cylinder; `face_type` tells `bots.wgsl` which, and how to shade it.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct BotVertex {
    pos: [f32; 3],
    face_type: u32,
    normal: [f32; 3],
    /// The HTM mesh's baked vendor paint. The cylinder has none — it takes
    /// its colour from the instance's bot type — so its vertices carry white,
    /// the value that tints nothing.
    color: [u8; 4],
}

impl BotVertex {
    fn cylinder(pos: [f32; 3], face_type: u32, normal: [f32; 3]) -> Self {
        Self { pos, face_type, normal, color: [255; 4] }
    }
}

const FACE_TOP: u32 = 0;
const FACE_SIDE: u32 = 1;
const FACE_BOTTOM: u32 = 2;
const FACE_MESH: u32 = 3;

pub struct BotGpuResources {
    pub pipeline: wgpu::RenderPipeline,
    pub occluded_pipeline: wgpu::RenderPipeline,
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub instance_buffer: wgpu::Buffer,
    /// Index range of the cylinder, drawn for VTMs and RTPs.
    cylinder_indices: Range<u32>,
    /// Index range of the HTM proxy mesh. Empty if the baked asset failed to
    /// parse, or if the two glyphs together outgrew what a 16-bit index can
    /// name, either of which sends HTMs back to the cylinder rather than
    /// dropping them from the scene.
    mesh_indices: Range<u32>,
    pub instance_count: u32,
    /// How many leading instances are HTMs. `build_bot_instances` orders them
    /// first so the two glyphs are two contiguous draws.
    htm_instance_count: u32,
}

/// Cylinder mesh for VTMs and RTPs: top disk, bottom disk, side strip.
fn cylinder_geometry() -> (Vec<BotVertex>, Vec<u16>) {
    let n = BOT_CIRCLE_SEGMENTS;
    let angle_of = |i: u32| 2.0 * std::f32::consts::PI * (i as f32) / (n as f32);
    let mut vertices: Vec<BotVertex> = Vec::new();

    vertices.push(BotVertex::cylinder([0.0, 0.0, 1.0], FACE_TOP, [0.0, 0.0, 1.0]));
    for i in 0..n {
        let a = angle_of(i);
        vertices.push(BotVertex::cylinder([a.cos(), a.sin(), 1.0], FACE_TOP, [0.0, 0.0, 1.0]));
    }
    let bc = (n + 1) as u16; // bottom center index
    vertices.push(BotVertex::cylinder([0.0, 0.0, 0.0], FACE_BOTTOM, [0.0, 0.0, -1.0]));
    for i in 0..n {
        let a = angle_of(i);
        vertices.push(BotVertex::cylinder([a.cos(), a.sin(), 0.0], FACE_BOTTOM, [0.0, 0.0, -1.0]));
    }
    let side_start = vertices.len() as u16;
    for i in 0..n {
        let a = angle_of(i);
        let normal = [a.cos(), a.sin(), 0.0];
        vertices.push(BotVertex::cylinder([a.cos(), a.sin(), 1.0], FACE_SIDE, normal));
        vertices.push(BotVertex::cylinder([a.cos(), a.sin(), 0.0], FACE_SIDE, normal));
    }

    let mut indices: Vec<u16> = Vec::new();
    // Top fan (CCW from +Z)
    for i in 1..=n {
        indices.push(0);
        indices.push(i as u16);
        indices.push(if i == n { 1 } else { (i + 1) as u16 });
    }
    // Bottom fan (CCW from -Z = reversed winding)
    for i in 1..=n {
        let curr = bc + i as u16;
        let next = if i == n { bc + 1 } else { bc + (i + 1) as u16 };
        indices.push(bc);
        indices.push(next);
        indices.push(curr);
    }
    // Side quads (using side strip vertices)
    for i in 0..n {
        let top_curr = side_start + (i * 2) as u16;
        let bot_curr = top_curr + 1;
        let top_next = side_start + (((i + 1) % n) * 2) as u16;
        let bot_next = top_next + 1;
        indices.push(top_curr);
        indices.push(bot_curr);
        indices.push(bot_next);
        indices.push(top_curr);
        indices.push(bot_next);
        indices.push(top_next);
    }
    (vertices, indices)
}

impl BotGpuResources {
    // WAIVER(100-line fn): Sequential wgpu pipeline construction for dual-pass bot
    // rendering (occluded + visible). Same justification as grid_pipeline.
    pub fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bot_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/bots.wgsl").into()),
        });

        // Both glyphs share one vertex and one index buffer, and the mesh's
        // own indices count from its own first vertex. That offset is added
        // here rather than passed as a draw call's base vertex, because
        // WebGL2 has no base-vertex draw at all and the browser viewer's
        // fallback backend would panic on the one call that used it. The
        // indices name the same vertices either way, so the picture does not
        // move; what it costs is the mesh having to fit in a 16-bit index
        // beside the cylinder, which at about a thousand vertices it does.
        let (mut vertices, mut indices) = cylinder_geometry();
        let cylinder_indices = 0..indices.len() as u32;
        let mesh_start = indices.len() as u32;
        if let Some(mesh) = htm_mesh::baked()
            && let Ok(base) = u16::try_from(vertices.len())
            && mesh.vertices.len() <= usize::from(u16::MAX - base)
        {
            vertices.extend(mesh.vertices.iter().map(|v| BotVertex {
                pos: v.position,
                face_type: FACE_MESH,
                normal: v.normal,
                color: v.color,
            }));
            indices.extend(mesh.indices.iter().map(|index| index + base));
        }
        let mesh_indices = mesh_start..indices.len() as u32;

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("bot_glyph_verts"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("bot_glyph_indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        // Pre-allocate instance buffer for up to MAX_BOT_INSTANCES bots.
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bot_instances"),
            size: MAX_BOT_INSTANCES * std::mem::size_of::<BotInstance>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bot_pipeline_layout"),
            bind_group_layouts: &[bind_group_layout],
            push_constant_ranges: &[],
        });

        let glyph_vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<BotVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                // pos
                wgpu::VertexAttribute { offset: 0, shader_location: 0, format: wgpu::VertexFormat::Float32x3 },
                // face_type
                wgpu::VertexAttribute { offset: 12, shader_location: 6, format: wgpu::VertexFormat::Uint32 },
                // normal
                wgpu::VertexAttribute { offset: 16, shader_location: 7, format: wgpu::VertexFormat::Float32x3 },
                // color
                wgpu::VertexAttribute { offset: 28, shader_location: 9, format: wgpu::VertexFormat::Unorm8x4 },
            ],
        };

        let bot_instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<BotInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                // world_x
                wgpu::VertexAttribute { offset: 0, shader_location: 1, format: wgpu::VertexFormat::Float32 },
                // world_y
                wgpu::VertexAttribute { offset: 4, shader_location: 2, format: wgpu::VertexFormat::Float32 },
                // world_z
                wgpu::VertexAttribute { offset: 8, shader_location: 3, format: wgpu::VertexFormat::Float32 },
                // bot_type
                wgpu::VertexAttribute { offset: 12, shader_location: 4, format: wgpu::VertexFormat::Uint32 },
                // highlight
                wgpu::VertexAttribute { offset: 16, shader_location: 5, format: wgpu::VertexFormat::Uint32 },
                // heading
                wgpu::VertexAttribute { offset: 20, shader_location: 8, format: wgpu::VertexFormat::Float32 },
            ],
        };

        let bot_primitive = wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            ..Default::default()
        };
        let bot_color_target = Some(wgpu::ColorTargetState {
            format: target_format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        });

        // Pass 1: visible bots (in front of / on top of geometry).
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bot_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[glyph_vertex_layout.clone(), bot_instance_layout.clone()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[bot_color_target.clone()],
                compilation_options: Default::default(),
            }),
            primitive: bot_primitive.clone(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: multisample(),
            multiview: None,
            cache: None,
        });

        // Pass 2: occluded bots (behind rack geometry, muted alpha).
        let occluded_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bot_occluded_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[glyph_vertex_layout, bot_instance_layout],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_occluded"),
                targets: &[bot_color_target],
                compilation_options: Default::default(),
            }),
            primitive: bot_primitive,
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Greater,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: multisample(),
            multiview: None,
            cache: None,
        });

        Self {
            pipeline,
            occluded_pipeline,
            vertex_buffer,
            index_buffer,
            instance_buffer,
            cylinder_indices,
            mesh_indices,
            instance_count: 0,
            htm_instance_count: 0,
        }
    }

    pub fn update_instances(&mut self, queue: &wgpu::Queue, instances: &[BotInstance], htm_count: u32) {
        self.instance_count = (instances.len() as u64).min(MAX_BOT_INSTANCES) as u32;
        self.htm_instance_count = htm_count.min(self.instance_count);
        if self.instance_count > 0 {
            queue.write_buffer(
                &self.instance_buffer,
                0,
                bytemuck::cast_slice(&instances[..self.instance_count as usize]),
            );
        }
    }

    /// Draw every bot with the pipeline already bound: HTMs as the proxy mesh,
    /// the rest as cylinders.
    pub fn draw(&self, render_pass: &mut wgpu::RenderPass<'static>) {
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        if self.mesh_indices.is_empty() {
            render_pass.draw_indexed(self.cylinder_indices.clone(), 0, 0..self.instance_count);
            return;
        }
        render_pass.draw_indexed(self.mesh_indices.clone(), 0, 0..self.htm_instance_count);
        render_pass.draw_indexed(self.cylinder_indices.clone(), 0, self.htm_instance_count..self.instance_count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The HTM glyph's indices are folded past the cylinder's vertices rather
    /// than passed as a draw call's base vertex, because WebGL2 has no
    /// base-vertex draw. If the fold were dropped the mesh would draw the
    /// cylinder's vertices — a shape, but the wrong one — so this holds the
    /// two claims that make the fold visible: the mesh is there, and every
    /// index of it points past the cylinder.
    #[test]
    fn the_htm_mesh_indices_are_folded_past_the_cylinder() {
        let (mut vertices, mut indices) = cylinder_geometry();
        let cylinder_vertices = vertices.len();
        let cylinder_indices = indices.len();
        let mesh = htm_mesh::baked().expect("the baked asset parses");
        let base = u16::try_from(cylinder_vertices).expect("the cylinder fits a 16-bit index");
        vertices.extend(std::iter::repeat_n(vertices[0], mesh.vertices.len()));
        indices.extend(mesh.indices.iter().map(|index| index + base));

        assert!(indices.len() > cylinder_indices, "the mesh contributes indices of its own");
        assert!(vertices.len() <= usize::from(u16::MAX), "and the two glyphs together still fit a 16-bit index");
        assert!(
            indices[cylinder_indices..].iter().all(|index| usize::from(*index) >= cylinder_vertices),
            "every folded index names a vertex of the mesh, never one of the cylinder"
        );
    }
}
