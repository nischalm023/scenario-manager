//! Within `renderer`, `totes` is HOW A CONTAINER AND ITS CONTENTS ARE DRAWN:
//! one instanced shell per container, then one cube per physical unit inside
//! it, all sharing the container's own position, heading and pitch.
//!
//! The expansion from one container row to a shell and its units is a pure
//! function of that row, which is why the row is what the parity gate compares
//! and what a caller keeps. The instance buffer grows with the scene and is
//! never shrunk: a run that inducts units mid-flight would otherwise reallocate
//! every frame.

use std::mem::size_of;
use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use super::instances::ToteInstance;
use super::{DEPTH_FORMAT, multisample};

#[path = "container_mesh.rs"]
mod mesh;
use mesh::Vertex;

const WIDTH: f32 = 0.65;
const DEPTH: f32 = 0.45;
const UNIT_SIZE: f32 = 0.075;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
struct DrawInstance {
    origin: [f32; 3],
    heading: f32,
    scale: [f32; 3],
    color: [f32; 3],
    physical_type: u32,
    pitch: f32,
}

fn height(physical_type: u32) -> f32 {
    if physical_type == 1 { 0.025 } else { 0.15 }
}

fn shell_instance(tote: &ToteInstance) -> DrawInstance {
    let color = if tote.physical_type == 2 {
        [0.72, 0.49, 0.27]
    } else if tote.designation == 1 {
        [0.10, 0.62, 0.65]
    } else {
        [0.72, 0.53, 0.34]
    };
    DrawInstance {
        origin: [tote.world_x, tote.world_y, tote.world_z],
        heading: tote.heading,
        scale: [WIDTH, DEPTH, height(tote.physical_type)],
        color,
        physical_type: tote.physical_type,
        pitch: tote.pitch,
    }
}

fn random_unit(seed: u64) -> f32 {
    let mut value = seed.wrapping_add(0x9e3779b97f4a7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    ((value ^ (value >> 31)) >> 40) as f32 / 16_777_215.0
}

/// Stable overlapping offsets never spill over the walls as count grows.
/// Trays support the same cubes on their shallow floor, with no fictitious lid.
fn unit_offset(tote_id: u32, index: u64, physical_type: u32) -> [f32; 3] {
    let seed = (u64::from(tote_id) << 32) ^ index;
    let floor = height(physical_type) * 0.08;
    [
        (random_unit(seed) * 2.0 - 1.0) * (WIDTH * 0.5 - 0.015 - UNIT_SIZE * 0.5),
        (random_unit(seed ^ 0x517cc1b727220a95) * 2.0 - 1.0) * (DEPTH * 0.5 - 0.015 - UNIT_SIZE * 0.5),
        floor + random_unit(seed ^ 0x6eed0e9da4d94a4f) * (height(physical_type) - floor - UNIT_SIZE).max(0.0),
    ]
}

fn unit_instance(tote: &ToteInstance, index: u64) -> DrawInstance {
    let [x, local_y, local_z] = unit_offset(tote.tote_id, index, tote.physical_type);
    let (sp, cp) = tote.pitch.sin_cos();
    let y = local_y * cp - local_z * sp;
    let z = local_y * sp + local_z * cp;
    let (s, c) = tote.heading.sin_cos();
    let shade = 0.75 + random_unit(index ^ u64::from(tote.tote_id)) * 0.13;
    DrawInstance {
        origin: [tote.world_x + x * c - y * s, tote.world_y + x * s + y * c, tote.world_z + z],
        heading: tote.heading,
        scale: [UNIT_SIZE; 3],
        color: [shade, shade * 0.98, shade * 0.90],
        physical_type: tote.physical_type,
        pitch: tote.pitch,
    }
}

/// The bytes the GPU is handed for these containers -- what
/// `pinned_buffers` holds to the pre-move desvis's.
#[cfg(test)]
pub(super) fn draw_instance_bytes(totes: &[ToteInstance]) -> Vec<u8> {
    bytemuck::cast_slice(&draw_instances(totes)).to_vec()
}

fn draw_instances(totes: &[ToteInstance]) -> Vec<DrawInstance> {
    let count = totes
        .iter()
        .try_fold(totes.len() as u64, |count, tote| count.checked_add(tote.contents_units))
        .expect("container unit count fits u64");
    let count = usize::try_from(count).expect("container draw count fits address space");
    let mut instances = Vec::with_capacity(count);
    instances.extend(totes.iter().map(shell_instance));
    for tote in totes {
        instances.extend((0..tote.contents_units).map(|index| unit_instance(tote, index)));
    }
    instances
}

pub struct ToteGpuResources {
    pub pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    buffer_bytes: u64,
    shell_indices: Range<u32>,
    unit_indices: Range<u32>,
    pub instance_count: u32,
    draw_count: u32,
}

impl ToteGpuResources {
    pub fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tote_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/totes.wgsl").into()),
        });
        let mesh = mesh::build();
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("open_container_vertices"),
            contents: bytemuck::cast_slice(&mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("open_container_indices"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let buffer_bytes = 1024 * size_of::<DrawInstance>() as u64;
        let instance_buffer = Self::instance_buffer(device, buffer_bytes);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tote_pipeline_layout"),
            bind_group_layouts: &[bind_group_layout],
            push_constant_ranges: &[],
        });
        let mesh_layout = wgpu::VertexBufferLayout {
            array_stride: size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute { offset: 0, shader_location: 0, format: wgpu::VertexFormat::Float32x3 },
                wgpu::VertexAttribute { offset: 12, shader_location: 1, format: wgpu::VertexFormat::Float32x3 },
                wgpu::VertexAttribute { offset: 24, shader_location: 2, format: wgpu::VertexFormat::Uint32 },
            ],
        };
        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: size_of::<DrawInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute { offset: 0, shader_location: 3, format: wgpu::VertexFormat::Float32x3 },
                wgpu::VertexAttribute { offset: 12, shader_location: 4, format: wgpu::VertexFormat::Float32 },
                wgpu::VertexAttribute { offset: 16, shader_location: 5, format: wgpu::VertexFormat::Float32x3 },
                wgpu::VertexAttribute { offset: 28, shader_location: 6, format: wgpu::VertexFormat::Float32x3 },
                wgpu::VertexAttribute { offset: 40, shader_location: 7, format: wgpu::VertexFormat::Uint32 },
                wgpu::VertexAttribute { offset: 44, shader_location: 8, format: wgpu::VertexFormat::Float32 },
            ],
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("open_container_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[mesh_layout, instance_layout],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: None,
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
        Self {
            pipeline,
            vertex_buffer,
            index_buffer,
            instance_buffer,
            buffer_bytes,
            shell_indices: mesh.shell,
            unit_indices: mesh.contents,
            instance_count: 0,
            draw_count: 0,
        }
    }

    fn instance_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("container_and_contents_instances"),
            size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn update_instances(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, totes: &[ToteInstance]) {
        let instances = draw_instances(totes);
        self.instance_count = u32::try_from(totes.len()).expect("container count fits GPU draw range");
        self.draw_count = u32::try_from(instances.len()).expect("physical unit count fits GPU draw range");
        let bytes = bytemuck::cast_slice(&instances);
        if bytes.len() as u64 > self.buffer_bytes {
            self.buffer_bytes = (bytes.len() as u64).next_power_of_two();
            self.instance_buffer = Self::instance_buffer(device, self.buffer_bytes);
        }
        if !bytes.is_empty() {
            queue.write_buffer(&self.instance_buffer, 0, bytes);
        }
    }

    pub fn draw(&self, render_pass: &mut wgpu::RenderPass<'static>) {
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        render_pass.draw_indexed(self.shell_indices.clone(), 0, 0..self.instance_count);
        if self.draw_count > self.instance_count {
            render_pass.draw_indexed(self.unit_indices.clone(), 0, self.instance_count..self.draw_count);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tote(units: u64) -> ToteInstance {
        ToteInstance {
            tote_id: 42,
            contents_units: units,
            world_x: 3.0,
            world_y: 4.0,
            world_z: 0.15,
            tote_type: 3,
            designation: 1,
            heading: 0.7,
            pitch: 0.0,
            physical_type: 0,
        }
    }

    #[test]
    fn every_physical_unit_has_a_cube_and_offsets_stay_inside_the_container() {
        for physical_type in 0..3 {
            let mut container = tote(2000);
            container.physical_type = physical_type;
            let draw = draw_instances(&[container]);
            assert_eq!(draw.len(), 2001);
            for index in 0..container.contents_units {
                let offset = unit_offset(container.tote_id, index, physical_type);
                assert!(offset[0].abs() + UNIT_SIZE * 0.5 <= WIDTH * 0.5 - 0.015 + 1e-6);
                assert!(offset[1].abs() + UNIT_SIZE * 0.5 <= DEPTH * 0.5 - 0.015 + 1e-6);
                assert!(offset[2] >= height(physical_type) * 0.08);
                if physical_type != 1 {
                    assert!(offset[2] + UNIT_SIZE <= height(physical_type) + 1e-6);
                }
            }
        }
        assert_eq!(draw_instances(&[tote(0)]).len(), 1);
    }

    #[test]
    fn item_positions_follow_parent_yaw_and_are_stable_across_quantity_changes() {
        let mut container = tote(3);
        let first = draw_instances(&[container]);
        container.contents_units = 9;
        let later = draw_instances(&[container]);
        assert_eq!(&first[1..], &later[1..4]);
        container.heading += 1.3;
        let rotated = unit_instance(&container, 0);
        let offset = unit_offset(container.tote_id, 0, container.physical_type);
        let (s, c) = container.heading.sin_cos();
        assert!((rotated.origin[0] - container.world_x - (offset[0] * c - offset[1] * s)).abs() < 1e-6);
        assert!((rotated.origin[1] - container.world_y - (offset[0] * s + offset[1] * c)).abs() < 1e-6);
        assert_eq!(rotated.heading, container.heading);
        assert_eq!(rotated.origin[2], container.world_z + offset[2]);
        container.pitch = 0.4;
        let inclined = unit_instance(&container, 0);
        let (sp, cp) = container.pitch.sin_cos();
        assert!((inclined.origin[2] - container.world_z - (offset[1] * sp + offset[2] * cp)).abs() < 1e-6);
        assert_eq!(inclined.pitch, container.pitch);
    }
}
