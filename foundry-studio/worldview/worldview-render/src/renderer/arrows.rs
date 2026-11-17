//! Within `renderer`, `arrows` is WHICH WAY A ROBOT MAY GO: one glyph per
//! directed hop, drawn flat over the cells in the views that ask about
//! reachability rather than about the run.
//!
//! All three sets live in one instance buffer, laid out unladen-D30, laden-D30,
//! DA50, so a view draws its own contiguous run and the buffer is built once.
//! The pipeline writes no depth and always passes: arrows are an annotation
//! over the floor, never geometry standing on it.

use wgpu::util::DeviceExt;

use super::instances::{ArrowInstance, ArrowModeRanges, build_arrow_instances_by_mode};
use super::{DEPTH_FORMAT, multisample};
use crate::frame::RenderFrame;
use crate::scene::Edge;
use crate::view::ArrowSet;

pub struct ArrowGpuResources {
    pub pipeline: wgpu::RenderPipeline,
    pub vertex_buffer: wgpu::Buffer,
    pub instance_buffer: wgpu::Buffer,
    pub mode_ranges: ArrowModeRanges,
}

impl ArrowGpuResources {
    pub fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        bind_group_layout: &wgpu::BindGroupLayout,
        edges: &[Edge],
        frame: &RenderFrame,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("arrow_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/arrows.wgsl").into()),
        });

        // Arrow triangle: tip at +X, base at -X. In local space [-1, 1].
        let vertices: [[f32; 2]; 3] = [
            [1.0, 0.0],   // tip
            [-1.0, -1.0], // base left
            [-1.0, 1.0],  // base right
        ];
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("arrow_verts"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        // Build instances grouped by mode.
        let (instances, mode_ranges) = build_arrow_instances_by_mode(edges, frame);
        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("arrow_instances"),
            contents: if instances.is_empty() {
                &[0u8; 16] // dummy data for empty buffer
            } else {
                bytemuck::cast_slice(&instances)
            },
            usage: wgpu::BufferUsages::VERTEX,
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("arrow_pipeline_layout"),
            bind_group_layouts: &[bind_group_layout],
            push_constant_ranges: &[],
        });

        let vert_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<[f32; 2]>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x2,
            }],
        };

        let inst_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<ArrowInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                // from_x, from_y (metres)
                wgpu::VertexAttribute { offset: 0, shader_location: 1, format: wgpu::VertexFormat::Float32x2 },
                // dx, dy (metre delta to the target centre)
                wgpu::VertexAttribute { offset: 8, shader_location: 2, format: wgpu::VertexFormat::Float32x2 },
                // scale (local pitch, metres)
                wgpu::VertexAttribute { offset: 16, shader_location: 3, format: wgpu::VertexFormat::Float32 },
            ],
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("arrow_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[vert_layout, inst_layout],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: multisample(),
            multiview: None,
            cache: None,
        });

        Self { pipeline, vertex_buffer, instance_buffer, mode_ranges }
    }

    /// Where one set sits in the buffer, as (first instance, count). No set
    /// asked for is an empty range, which draws nothing.
    pub fn range_for(&self, set: Option<ArrowSet>) -> (u32, u32) {
        match set {
            Some(ArrowSet::UnladenHtm) => (self.mode_ranges.unladen_htm_start, self.mode_ranges.unladen_htm_count),
            Some(ArrowSet::LadenHtm) => (self.mode_ranges.laden_htm_start, self.mode_ranges.laden_htm_count),
            Some(ArrowSet::Vtm) => (self.mode_ranges.vtm_start, self.mode_ranges.vtm_count),
            None => (0, 0),
        }
    }
}
