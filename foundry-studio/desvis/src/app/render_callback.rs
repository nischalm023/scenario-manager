//! Within `app`, `render_callback` is the BRIDGE from egui's paint pass to the
//! shared renderer: egui hands over the device, the queue and a render pass
//! part-way through painting its own frame, and this asks `worldview-render`
//! to put one instant in it.
//!
//! It is the only place desvis's window and the crate's drawing meet, which is
//! what keeps egui out of the crate.

use eframe::egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use eframe::epaint::PaintCallbackInfo;
use eframe::wgpu;
use worldview_render::{Moment, SceneRenderer};

/// One instant on its way through egui's paint pass.
pub(crate) struct SceneCallback {
    pub(crate) moment: Moment,
}

impl CallbackTrait for SceneCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(renderer) = callback_resources.get_mut::<SceneRenderer>() {
            renderer.upload(device, queue, &self.moment);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &CallbackResources,
    ) {
        if let Some(renderer) = callback_resources.get::<SceneRenderer>() {
            renderer.draw(render_pass, &self.moment);
        }
    }
}
