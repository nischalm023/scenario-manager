//! Within `crucible-viewer`, `canvas` is THE GPU BEHIND ONE `<canvas>`: the
//! wgpu device and queue the renderer is handed, the surface a frame is
//! presented to, and the two attachments a render pass writes into.
//!
//! It exists because the renderer draws into a pass somebody else opened, and
//! in a browser that somebody is this. desvis gets its device, its depth
//! buffer and its multisampling from eframe; there is no eframe here, so each
//! is made by hand — and made to the numbers `worldview-render` built its
//! pipelines against, because a pass whose sample count or depth format
//! disagrees with a pipeline is a validation error rather than a softer
//! picture.
//!
//! wasm32 only: `SurfaceTarget::Canvas` and the browser surface behind it
//! exist on no other target.

use wasm_bindgen::JsValue;
use web_sys::HtmlCanvasElement;
use worldview_render::{DEPTH_FORMAT, MSAA_SAMPLES};

pub(crate) struct Canvas {
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    /// Which of the two browser backends the tab gave us, for a shell that
    /// wants to say so and for a headless run that has to prove it drew.
    pub(crate) backend: &'static str,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    /// The multisampled colour target the pass writes into and resolves out
    /// of; the surface texture itself is single-sampled.
    colour: wgpu::TextureView,
    depth: wgpu::TextureView,
}

impl Canvas {
    pub(crate) async fn new(canvas: HtmlCanvasElement) -> Result<Self, JsValue> {
        let (width, height) = (canvas.width().max(1), canvas.height().max(1));
        // A tab can name `navigator.gpu` and still offer no WebGPU adapter (Chrome on Linux, a headless Chrome), and
        // an instance made for WebGPU then has no WebGL2 to fall back to; asking for an adapter first decides.
        let instance = wgpu::util::new_instance_with_webgpu_detection(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::BROWSER_WEBGPU | wgpu::Backends::GL,
            ..Default::default()
        })
        .await;
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
            .map_err(|failure| JsValue::from_str(&format!("this canvas takes no GPU surface: {failure}")))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or_else(|| JsValue::from_str("this tab offers neither WebGPU nor WebGL2"))?;
        let backend = match adapter.get_info().backend {
            wgpu::Backend::BrowserWebGpu => "webgpu",
            _ => "webgl",
        };
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor { required_limits: adapter.limits(), ..Default::default() }, None)
            .await
            .map_err(|failure| JsValue::from_str(&format!("no GPU device: {failure}")))?;

        // The shaders write colours that are already sRGB, so an sRGB surface
        // encodes them a second time and the whole picture comes out pale.
        // The two browser backends do not offer the same list — WebGPU leads
        // with a plain format here and WebGL2 with an sRGB one — so the plain
        // one is asked for by name rather than taken by position, which is
        // what makes the fallback draw the picture WebGPU draws.
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|format| !format.is_srgb())
            .or_else(|| caps.formats.first().copied())
            .ok_or_else(|| JsValue::from_str("this canvas's GPU surface offers no texture format at all"))?;
        let alpha_mode = caps
            .alpha_modes
            .first()
            .copied()
            .ok_or_else(|| JsValue::from_str("this canvas's GPU surface offers no alpha mode at all"))?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);
        let colour = target(&device, format, width, height, MSAA_SAMPLES);
        let depth = target(&device, DEPTH_FORMAT, width, height, MSAA_SAMPLES);
        Ok(Self { device, queue, backend, surface, config, colour, depth })
    }

    pub(crate) fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    /// The surface's size in device pixels.
    pub(crate) fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 || (width, height) == (self.config.width, self.config.height) {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.colour = target(&self.device, self.config.format, width, height, MSAA_SAMPLES);
        self.depth = target(&self.device, DEPTH_FORMAT, width, height, MSAA_SAMPLES);
    }

    /// Open a pass over this canvas cleared to `ground` (red, green, blue and
    /// alpha, already sRGB-encoded, as the palette keeps them), hand it to
    /// `paint`, and present what was drawn. Nil when the surface is being
    /// reconfigured under us, which is a frame to skip rather than a failure.
    pub(crate) fn painted(&self, ground: [f64; 4], paint: impl FnOnce(&mut wgpu::RenderPass<'static>)) -> Option<()> {
        let frame = self.surface.get_current_texture().ok()?;
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("the scene"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.colour,
                        resolve_target: Some(&view),
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: ground[0],
                                g: ground[1],
                                b: ground[2],
                                a: ground[3],
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                })
                .forget_lifetime();
            paint(&mut pass);
        }
        self.queue.submit(Some(encoder.finish()));
        frame.present();
        Some(())
    }
}

fn target(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    samples: u32,
) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("a render attachment"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}
