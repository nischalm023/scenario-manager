//! Within `renderer`, `rack_frames` is THE STEEL ITSELF: one instanced box per
//! upright, beam, slot divider and diving-board tab of
//! [`crate::rack::RackVisual::frames`], and the same pipeline draws the
//! station props a viewer adds to that list.
//!
//! The boxes are a map's own geometry, so the buffer is static per map and is
//! rebuilt only when the whole resource set is. Each box carries a yaw about
//! its own centre because a prop is not axis-aligned, and a kind the shader
//! colours by — which is why the kind discriminants are not free to reorder.
//!
//! Exchange cells are not drawn yet: a world-positioned HTM dock at a board-0
//! position belongs here as a dock pad, in the same frame the bay geometry
//! uses, and nothing draws one today.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use super::{DEPTH_FORMAT, multisample};
use crate::rack::RackBox;

/// Per-instance data: one axis-aligned box. Layout mirrors the vertex
/// attributes in rack_frames.wgsl.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct RackFrameInstance {
    min: [f32; 3],
    size: [f32; 3],
    kind: u32,
    /// Radians about the box's XY centre. Zero for rack steel; the EasyLoop
    /// prop's 45-degree seats and curve chords are why it exists.
    yaw: f32,
}

/// One row per box, in the boxes' own order — the buffer the steel is drawn
/// from, and what a golden test reads.
fn frame_instances(boxes: &[RackBox]) -> Vec<RackFrameInstance> {
    boxes.iter().map(|b| RackFrameInstance { min: b.min, size: b.size, kind: b.kind as u32, yaw: b.yaw }).collect()
}

/// The steel rows this scene draws, as bytes — how a golden test holds the
/// boxes a map derived to what they were.
#[cfg(test)]
pub(super) fn frame_instance_bytes(boxes: &[RackBox]) -> Vec<u8> {
    bytemuck::cast_slice(&frame_instances(boxes)).to_vec()
}

pub struct RackFrameGpuResources {
    pub pipeline: wgpu::RenderPipeline,
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub instance_buffer: wgpu::Buffer,
    pub index_count: u32,
    pub instance_count: u32,
}

impl RackFrameGpuResources {
    pub fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        bind_group_layout: &wgpu::BindGroupLayout,
        boxes: &[RackBox],
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rack_frame_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/rack_frames.wgsl").into()),
        });

        // Unit cube: 8 vertices, 36 indices (same topology as the tote cube).
        let vertices: [[f32; 3]; 8] = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
        ];
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("rack_frame_cube_verts"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let indices: [u16; 36] = [
            // bottom (-Z)
            0, 2, 1, 0, 3, 2, // top (+Z)
            4, 5, 6, 4, 6, 7, // front (+Y)
            3, 7, 6, 3, 6, 2, // back (-Y)
            0, 1, 5, 0, 5, 4, // right (+X)
            1, 2, 6, 1, 6, 5, // left (-X)
            0, 4, 7, 0, 7, 3,
        ];
        let index_count = indices.len() as u32;
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("rack_frame_cube_indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let instances = frame_instances(boxes);
        let instance_count = instances.len() as u32;
        // Zero-size buffers are invalid; keep one zeroed instance and draw none.
        let padded: Vec<RackFrameInstance> =
            if instances.is_empty() { vec![RackFrameInstance::zeroed()] } else { instances };
        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("rack_frame_instances"),
            contents: bytemuck::cast_slice(&padded),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rack_frame_pipeline_layout"),
            bind_group_layouts: &[bind_group_layout],
            push_constant_ranges: &[],
        });

        let cube_vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<[f32; 3]>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x3,
            }],
        };

        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<RackFrameInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                // box_min
                wgpu::VertexAttribute { offset: 0, shader_location: 1, format: wgpu::VertexFormat::Float32x3 },
                // box_size
                wgpu::VertexAttribute { offset: 12, shader_location: 2, format: wgpu::VertexFormat::Float32x3 },
                // kind
                wgpu::VertexAttribute { offset: 24, shader_location: 3, format: wgpu::VertexFormat::Uint32 },
                // yaw
                wgpu::VertexAttribute { offset: 28, shader_location: 4, format: wgpu::VertexFormat::Float32 },
            ],
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rack_frame_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[cube_vertex_layout, instance_layout],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
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

        Self { pipeline, vertex_buffer, index_buffer, instance_buffer, index_count, instance_count }
    }
}
