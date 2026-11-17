//! Within `renderer`, `grid_pipeline` is the BUILD step for the cell
//! pipeline: the shader module, the one box mesh every cell instance is drawn
//! from, the instance buffer holding the cell tiers and the underlay, the
//! hand-written vertex layout that must agree with `CellInstance`, and the
//! depth and blend state the whole floor draws under.
//!
//! It also owns the sub-resource sets that hang off the same camera bind
//! group — bots, totes, arrows and rack bay frames — because they are built
//! once, together, from the same scene.

use wgpu::util::DeviceExt;

use super::arrows::ArrowGpuResources;
use super::bots::BotGpuResources;
use super::cell_layer::{BoxVertex, CellInstance, build_cell_instances, compose_floor_instances};
use super::rack_frames::RackFrameGpuResources;
use super::totes::ToteGpuResources;
use super::{CameraUniforms, DEPTH_FORMAT, SceneRenderer, multisample};
use crate::scene::Scene;

impl SceneRenderer {
    // WAIVER(100-line fn): Sequential wgpu pipeline construction — shader compilation,
    // buffer creation, bind group layout, pipeline layout, render pipeline. Splitting
    // would scatter tightly coupled resource creation with no clarity gain.
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat, scene: &Scene) -> Self {
        let (cells, frame, rack, underlay) = (scene.cells(), scene.frame(), scene.rack(), scene.underlay());
        // Shader
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("grid_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/grid.wgsl").into()),
        });

        // Box mesh: 6 faces (top + 4 sides + floor), 24 vertices, 36 indices.
        // When rack_levels=0, z_factor*0=0 and sides/floor collapse to degenerate triangles.
        let box_vertices: [BoxVertex; 24] = [
            // Top face (face_id=0, z_factor=1)
            BoxVertex { pos: [0.0, 0.0, 1.0], face_id: 0 },
            BoxVertex { pos: [1.0, 0.0, 1.0], face_id: 0 },
            BoxVertex { pos: [1.0, 1.0, 1.0], face_id: 0 },
            BoxVertex { pos: [0.0, 1.0, 1.0], face_id: 0 },
            // Front face (face_id=1, Y=1 side)
            BoxVertex { pos: [0.0, 1.0, 0.0], face_id: 1 },
            BoxVertex { pos: [1.0, 1.0, 0.0], face_id: 1 },
            BoxVertex { pos: [1.0, 1.0, 1.0], face_id: 1 },
            BoxVertex { pos: [0.0, 1.0, 1.0], face_id: 1 },
            // Back face (face_id=2, Y=0 side)
            BoxVertex { pos: [1.0, 0.0, 0.0], face_id: 2 },
            BoxVertex { pos: [0.0, 0.0, 0.0], face_id: 2 },
            BoxVertex { pos: [0.0, 0.0, 1.0], face_id: 2 },
            BoxVertex { pos: [1.0, 0.0, 1.0], face_id: 2 },
            // Right face (face_id=3, X=1 side)
            BoxVertex { pos: [1.0, 1.0, 0.0], face_id: 3 },
            BoxVertex { pos: [1.0, 0.0, 0.0], face_id: 3 },
            BoxVertex { pos: [1.0, 0.0, 1.0], face_id: 3 },
            BoxVertex { pos: [1.0, 1.0, 1.0], face_id: 3 },
            // Left face (face_id=4, X=0 side)
            BoxVertex { pos: [0.0, 0.0, 0.0], face_id: 4 },
            BoxVertex { pos: [0.0, 1.0, 0.0], face_id: 4 },
            BoxVertex { pos: [0.0, 1.0, 1.0], face_id: 4 },
            BoxVertex { pos: [0.0, 0.0, 1.0], face_id: 4 },
            // Floor face (face_id=5, z_factor=0, faces up +Z)
            BoxVertex { pos: [0.0, 0.0, 0.0], face_id: 5 },
            BoxVertex { pos: [1.0, 0.0, 0.0], face_id: 5 },
            BoxVertex { pos: [1.0, 1.0, 0.0], face_id: 5 },
            BoxVertex { pos: [0.0, 1.0, 0.0], face_id: 5 },
        ];
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grid_box_verts"),
            contents: bytemuck::cast_slice(&box_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let box_indices: [u16; 36] = [
            0, 1, 2, 0, 2, 3, // top
            4, 5, 6, 4, 6, 7, // front
            8, 9, 10, 8, 10, 11, // back
            12, 13, 14, 12, 14, 15, // right
            16, 17, 18, 16, 18, 19, // left
            20, 21, 22, 20, 22, 23, // floor
        ];
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grid_box_indices"),
            contents: bytemuck::cast_slice(&box_indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        // Instance buffer: one instance per CELL ID, dense (index == id).
        // The prefix's order is load-bearing: `rewrite_with_overlays` rewrites
        // it by the same ids, and no row or column ever enters the contract.
        let mut instances = build_cell_instances(cells, frame, rack.uniform_levels);
        // Three tiers, one bias step apart in DEPTH-BUFFER units: markers over
        // cell tiles over a single COMPOSED base. The base is built CPU-side
        // as disjoint rectangles (steel > equipment > building, each minus
        // everything above it), so within that tier nothing overlaps and depth
        // never has to arbitrate coplanar quads. The tiles still float over the
        // base: their hairline float seams show the base colour through, which
        // is what draws the cell grid. The lattice prefix stays first
        // regardless: `rewrite_with_overlays` rewrites offset 0 positionally.
        instances.extend(compose_floor_instances(&underlay.steel_mm, &underlay.equipment, &underlay.building, frame));
        let instance_count = instances.len() as u32;
        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grid_instances"),
            contents: bytemuck::cast_slice(&instances),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        // Camera uniform buffer
        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grid_camera_uniform"),
            size: std::mem::size_of::<CameraUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("grid_camera_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("grid_camera_bg"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: camera_buffer.as_entire_binding() }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("grid_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        // Vertex buffer layouts
        let box_vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<BoxVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                // position (vec3: x, y, z_factor)
                wgpu::VertexAttribute { offset: 0, shader_location: 0, format: wgpu::VertexFormat::Float32x3 },
                // face_id
                wgpu::VertexAttribute { offset: 12, shader_location: 1, format: wgpu::VertexFormat::Uint32 },
            ],
        };

        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<CellInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            // Offsets are hand-written and have no compile-time link to
            // `CellInstance`; a mismatch reinterprets bit patterns silently
            // rather than erroring, so they are asserted in the tests below.
            attributes: &[
                // min_x, min_y (metres)
                wgpu::VertexAttribute { offset: 0, shader_location: 2, format: wgpu::VertexFormat::Float32x2 },
                // size_x, size_y (metres)
                wgpu::VertexAttribute { offset: 8, shader_location: 3, format: wgpu::VertexFormat::Float32x2 },
                // cell_kind
                wgpu::VertexAttribute { offset: 16, shader_location: 4, format: wgpu::VertexFormat::Uint32 },
                // rack_levels
                wgpu::VertexAttribute { offset: 20, shader_location: 5, format: wgpu::VertexFormat::Uint32 },
                // heatmap_vtm
                wgpu::VertexAttribute { offset: 24, shader_location: 6, format: wgpu::VertexFormat::Float32 },
                // heatmap_htm
                wgpu::VertexAttribute { offset: 28, shader_location: 7, format: wgpu::VertexFormat::Float32 },
                // reserved
                wgpu::VertexAttribute { offset: 32, shader_location: 8, format: wgpu::VertexFormat::Uint32 },
                // depth_bias
                wgpu::VertexAttribute { offset: 36, shader_location: 9, format: wgpu::VertexFormat::Float32 },
                wgpu::VertexAttribute { offset: 40, shader_location: 10, format: wgpu::VertexFormat::Float32 },
            ],
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("grid_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[box_vertex_layout, instance_layout],
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
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: multisample(),
            multiview: None,
            cache: None,
        });

        let bots = BotGpuResources::new(device, target_format, &bind_group_layout);
        let totes = ToteGpuResources::new(device, target_format, &bind_group_layout);
        let arrows = ArrowGpuResources::new(device, target_format, &bind_group_layout, scene.edges(), frame);
        let rack_frames = RackFrameGpuResources::new(device, target_format, &bind_group_layout, &rack.frames);

        Self {
            pipeline,
            camera_bind_group,
            camera_buffer,
            vertex_buffer,
            index_buffer,
            index_count: box_indices.len() as u32,
            instance_buffer,
            instance_count,
            bots,
            totes,
            arrows,
            rack_frames,
        }
    }
}
