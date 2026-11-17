//! What `worldview-render` actually PUTS ON A SURFACE, checked on a real GPU
//! through the production container, robot and rack pipelines.
//!
//! It lives in desvis rather than in the crate because it is a native
//! harness — its own executor, a thread parked on adapter readback, and an
//! image decoder — and the crate has to stay something a browser can build.
//! Both tests are ignored by default: they need a GPU, and no artifact is
//! written unless CONTAINER_PREVIEW_PNG names its path.

use std::env;
use std::future::Future;
use std::sync::{Arc, mpsc};
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};
use std::time::Duration;

use image::RgbaImage;

use wgpu::util::DeviceExt;
use worldview_render::renderer::draw_state_objects;
use worldview_render::{
    BotGpuResources, BotInstance, CameraState, DEPTH_FORMAT, MSAA_SAMPLES, RackBox, RackBoxKind, RackFrameGpuResources,
    RenderFrame, ToteGpuResources, ToteInstance, ViewMode, Viewport, wgpu,
};

struct WakeThread(Thread);
impl Wake for WakeThread {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(WakeThread(thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => thread::park(),
        }
    }
}

fn render(
    totes: &[ToteInstance],
    bots: &[BotInstance],
    htm_count: u32,
    geometry: &[RackBox],
    clear: wgpu::Color,
) -> RgbaImage {
    const WIDTH: u32 = 1536;
    const HEIGHT: u32 = 1024;
    let gpu = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
    let adapter = block_on(gpu.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("GPU adapter");
    let (device, queue) =
        block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None)).expect("GPU device");
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let extent = wgpu::Extent3d { width: WIDTH, height: HEIGHT, depth_or_array_layers: 1 };
    let texture = |format, samples, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("container_preview"),
            size: extent,
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let resolved = texture(format, 1, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC);
    let color = texture(format, MSAA_SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT);
    let depth = texture(DEPTH_FORMAT, MSAA_SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT);
    let resolved_view = resolved.create_view(&Default::default());
    let color_view = color.create_view(&Default::default());
    let depth_view = depth.create_view(&Default::default());
    let mut camera = CameraState::new(RenderFrame::from_extent_mm(4500.0, 2500.0));
    camera.orbit_target = [0.0, 0.0, 0.1];
    camera.azimuth = -1.57;
    camera.elevation = 0.95;
    camera.distance = 4.2;
    let uniforms =
        camera.build_uniforms(Viewport::new([0.0, 0.0], [WIDTH as f32, HEIGHT as f32]), 1.0, ViewMode::State);
    let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("container_preview_camera"),
        contents: bytemuck::bytes_of(&uniforms),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("container_preview_camera_layout"),
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
    let camera_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("container_preview_camera_group"),
        layout: &camera_layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: camera_buffer.as_entire_binding() }],
    });
    let mut renderer = ToteGpuResources::new(&device, format, &camera_layout);
    renderer.update_instances(&device, &queue, totes);
    let mut bot_renderer = BotGpuResources::new(&device, format, &camera_layout);
    bot_renderer.update_instances(&queue, bots, htm_count);
    let rack_renderer = RackFrameGpuResources::new(&device, format, &camera_layout, geometry);
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("container_preview_readback"),
        size: u64::from(WIDTH * HEIGHT * 4),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("container_preview"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    resolve_target: Some(&resolved_view),
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(clear), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            })
            .forget_lifetime();
        if rack_renderer.instance_count > 0 {
            pass.set_pipeline(&rack_renderer.pipeline);
            pass.set_bind_group(0, &camera_group, &[]);
            pass.set_vertex_buffer(0, rack_renderer.vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, rack_renderer.instance_buffer.slice(..));
            pass.set_index_buffer(rack_renderer.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(0..rack_renderer.index_count, 0, 0..rack_renderer.instance_count);
        }
        draw_state_objects(&mut pass, &camera_group, &bot_renderer, &renderer);
    }
    encoder.copy_texture_to_buffer(
        resolved.as_image_copy(),
        wgpu::ImageCopyBuffer {
            buffer: &readback,
            layout: wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(WIDTH * 4), rows_per_image: Some(HEIGHT) },
        },
        extent,
    );
    queue.submit([encoder.finish()]);
    let (tx, rx) = mpsc::channel();
    readback.slice(..).map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
    device.poll(wgpu::Maintain::Wait);
    rx.recv_timeout(Duration::from_secs(10)).expect("GPU readback callback").expect("GPU readback");
    let bytes = readback.slice(..).get_mapped_range();
    RgbaImage::from_raw(WIDTH, HEIGHT, bytes.to_vec()).expect("RGBA image dimensions")
}

fn containers() -> Vec<ToteInstance> {
    // Back row: inventory empty/filled, order empty/filled. Front row:
    // carton empty/filled, loaded tray, loaded inventory tote on an incline.
    [
        (0, 0, 0, 0.0),
        (0, 0, 7, 0.0),
        (0, 1, 0, 0.0),
        (0, 1, 4, 0.0),
        (2, 1, 0, 0.0),
        (2, 1, 6, 0.0),
        (1, 1, 2, 0.0),
        (0, 0, 5, 0.28),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (physical_type, designation, contents_units, pitch))| ToteInstance {
        tote_id: index as u32,
        contents_units,
        world_x: (index % 4) as f32 - 1.5,
        world_y: if index < 4 { 0.6 } else { -0.6 },
        world_z: 0.15,
        tote_type: 3,
        designation,
        heading: 0.12,
        pitch,
        physical_type,
    })
    .collect()
}

#[test]
#[ignore = "requires a GPU and explicit CONTAINER_PREVIEW_PNG output path"]
fn render_open_container_preview() {
    let output = env::var_os("CONTAINER_PREVIEW_PNG").expect("set CONTAINER_PREVIEW_PNG to an explicit PNG path");
    let image = render(&containers(), &[], 0, &[], wgpu::Color { r: 0.94, g: 0.945, b: 0.955, a: 1.0 });
    assert!(image.pixels().any(|pixel| pixel[0] < 200), "real container pixels were rendered");
    image.save(output).expect("write explicitly requested preview PNG");
}

#[test]
#[ignore = "requires a GPU; checks real container/bot depth compositing without writing files"]
fn opaque_containers_hide_bots_while_rack_ghosts_remain_visible() {
    let mut containers = containers();
    for container in &mut containers {
        container.world_z = 0.5;
    }
    let bots: Vec<_> = containers
        .iter()
        .enumerate()
        .map(|(index, container)| BotInstance {
            world_x: container.world_x,
            world_y: container.world_y,
            world_z: 0.05,
            bot_type: u32::from(index < 4),
            highlight: (index % 3) as u32,
            heading: container.heading,
        })
        .collect();
    let rack = [RackBox { min: [-2.1, -1.25, 0.35], size: [4.2, 2.5, 0.05], yaw: 0.0, kind: RackBoxKind::Beam }];
    let clear = wgpu::Color::TRANSPARENT;
    let reference = render(&containers, &[], 0, &[], clear);
    let covered = render(&containers, &bots, 4, &rack, clear);
    let mut opaque_pixels = 0;
    for (expected, actual) in reference.pixels().zip(covered.pixels()) {
        if expected[3] == 255 {
            opaque_pixels += 1;
            assert_eq!(actual, expected, "a hidden robot or rack changed an opaque container pixel");
        }
    }
    assert!(opaque_pixels > 10_000, "the comparison covers the container surfaces");

    let rack_only = render(&[], &[], 0, &rack, clear);
    let rack_ghosts = render(&[], &bots, 4, &rack, clear);
    let ghost_pixels = rack_only
        .pixels()
        .zip(rack_ghosts.pixels())
        .filter(|(before, after)| before[3] == 255 && before != after)
        .count();
    assert!(ghost_pixels > 1000, "the rack still shows robots behind its opaque surface");
}
