//! Within `worldview-render`, `renderer` is THE DRAWING ITSELF: the wgpu
//! pipelines for the cell layer, the robots, the containers, the edge arrows
//! and the rack steel, the instance rows each one is fed, and the order they
//! are drawn in.
//!
//! A host owns the surface and the render pass; this owns everything inside
//! it. [`SceneRenderer::new`] builds the pipelines and uploads the floor once
//! per map, [`SceneRenderer::write_cell_overlays`] re-ships the cell rows when
//! an overlay or a reservation changes, and a [`Moment`] — where the camera
//! stands and what the movers are doing at one instant — is uploaded and drawn
//! every frame.

mod arrows;
mod bots;
pub mod cell_layer;
mod conveyor;
mod grid_pipeline;
mod htm_mesh;
#[cfg(test)]
mod pinned_buffers;
mod rack_frames;
mod totes;

pub mod instances;

pub use cell_layer::CellInstance;

pub use arrows::ArrowGpuResources;
pub use bots::BotGpuResources;
pub use instances::{BotInstance, ToteInstance, build_bot_instances, build_tote_instances, place_pos, place_z};
pub use rack_frames::RackFrameGpuResources;
pub use totes::ToteGpuResources;

use crate::camera::CameraUniforms;
use crate::scene::{Overlays, Playback, Scene};
use crate::view::ArrowSet;

/// The depth format every pipeline here is built against. A host that opens
/// its own render pass gives it a depth attachment of this format.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Samples per pixel for every pipeline here. egui feathers its own 2D shapes,
/// but nothing feathers the 3D scene: at four samples the bot mesh's silhouette
/// and the rack frames' thin steel stop crawling as the camera moves.
///
/// These pipelines draw inside egui's render pass, so the number is not ours
/// alone — `gui_launch` hands the same one to eframe as
/// `NativeOptions::multisampling`, which sizes the multisampled colour and
/// depth targets the pass writes into. Disagreeing is a wgpu validation error,
/// not a softer picture.
pub const MSAA_SAMPLES: u32 = 4;

fn multisample() -> wgpu::MultisampleState {
    wgpu::MultisampleState { count: MSAA_SAMPLES, ..Default::default() }
}

/// Every GPU object one map's drawing needs: the cell pipeline and its
/// buffers, and the sub-resources that hang off the same camera bind group.
pub struct SceneRenderer {
    pub pipeline: wgpu::RenderPipeline,
    pub camera_bind_group: wgpu::BindGroup,
    pub camera_buffer: wgpu::Buffer,
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub index_count: u32,
    pub instance_buffer: wgpu::Buffer,
    pub instance_count: u32,
    pub bots: BotGpuResources,
    pub totes: ToteGpuResources,
    pub arrows: ArrowGpuResources,
    pub rack_frames: RackFrameGpuResources,
}

impl SceneRenderer {
    /// Ship one instant to the GPU: the camera, the cell rows with this
    /// instant's overlays folded in, and the movers. A host calls it once per
    /// frame, before the render pass that [`Self::draw`] writes into.
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, moment: &Moment) {
        queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&moment.camera_uniforms));
        // The cell prefix is rewritten in place; the composed underlay behind
        // it was uploaded once with the map and stays.
        queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&moment.cell_instances));
        self.bots.update_instances(queue, &moment.bot_instances, moment.htm_instance_count);
        self.totes.update_instances(device, queue, &moment.tote_instances);
    }
}

/// One drawn instant, as the GPU will be fed it: the camera, the rows for the
/// cells, the robots and the containers, and which arrow set is on.
///
/// A viewer never builds these rows itself — [`Scene::moment`] does, from the
/// run's own state — so two viewers of one run cannot disagree about where a
/// container on a conveyor is or how a robot is turned.
pub struct Moment {
    pub camera_uniforms: CameraUniforms,
    pub cell_instances: Vec<CellInstance>,
    pub bot_instances: Vec<BotInstance>,
    /// How many leading `bot_instances` are HTMs, from `build_bot_instances`.
    pub htm_instance_count: u32,
    pub tote_instances: Vec<ToteInstance>,
    /// Which arrow set to draw, as the caller's view asks: the arrow shader
    /// does not read the view mode, so the caller names the set.
    pub arrows: Option<ArrowSet>,
}

impl Scene {
    /// The rows one instant draws with. The only way either viewer turns a run
    /// into instances.
    pub fn moment(&self, camera_uniforms: CameraUniforms, arrows: Option<ArrowSet>, at: &Playback<'_>) -> Moment {
        let (bot_instances, htm_instance_count) = build_bot_instances(at.robots, self);
        let mut tote_instances = build_tote_instances(at.totes, at.robots, self);
        conveyor::apply_transits(&mut tote_instances, at.transits, at.at_ms, self.rack());
        Moment {
            camera_uniforms,
            cell_instances: cell_layer::rewrite_with_overlays(self, &at.overlays),
            bot_instances,
            htm_instance_count,
            tote_instances,
            arrows,
        }
    }

    /// The cell rows alone, for a caller that wants the numbers rather than a
    /// picture.
    pub fn cell_rows(&self, overlays: &Overlays<'_>) -> Vec<CellInstance> {
        cell_layer::rewrite_with_overlays(self, overlays)
    }
}

impl SceneRenderer {
    /// Draw the moment into a render pass the host opened.
    pub fn draw(&self, render_pass: &mut wgpu::RenderPass<'static>, moment: &Moment) {
        // Draw grid cells.
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.camera_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        render_pass.draw_indexed(0..self.index_count, 0, 0..self.instance_count);

        // Draw rack bay frames (uprights/beams/dividers/board tabs) in
        // every view mode — placed maps flatten their rack cells to
        // floor decals, so the frames ARE the racks. Drawn before totes
        // and bots so both depth-test against the steel (the bots'
        // occluded pass sees through frames like it did through
        // extruded cells).
        if self.rack_frames.instance_count > 0 {
            render_pass.set_pipeline(&self.rack_frames.pipeline);
            render_pass.set_bind_group(0, &self.camera_bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.rack_frames.vertex_buffer.slice(..));
            render_pass.set_vertex_buffer(1, self.rack_frames.instance_buffer.slice(..));
            render_pass.set_index_buffer(self.rack_frames.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            render_pass.draw_indexed(0..self.rack_frames.index_count, 0, 0..self.rack_frames.instance_count);
        }

        // Draw edge arrows: the set the caller asked for.
        let (arrow_start, arrow_count) = self.arrows.range_for(moment.arrows);
        if arrow_count > 0 {
            render_pass.set_pipeline(&self.arrows.pipeline);
            render_pass.set_bind_group(0, &self.camera_bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.arrows.vertex_buffer.slice(..));
            render_pass.set_vertex_buffer(1, self.arrows.instance_buffer.slice(..));
            render_pass.draw(0..3, arrow_start..arrow_start + arrow_count);
        }

        if moment.camera_uniforms.draws_movers() {
            draw_state_objects(render_pass, &self.camera_bind_group, &self.bots, &self.totes);
        }
    }
}

/// Rack see-through is painted before containers: otherwise that pass also
/// paints robots through opaque tote walls and floors. Visible robots still
/// depth-test against both the rack and the container geometry.
pub fn draw_state_objects(
    render_pass: &mut wgpu::RenderPass<'static>,
    camera_bind_group: &wgpu::BindGroup,
    bots: &BotGpuResources,
    totes: &ToteGpuResources,
) {
    render_pass.set_bind_group(0, camera_bind_group, &[]);
    if bots.instance_count > 0 {
        render_pass.set_pipeline(&bots.occluded_pipeline);
        bots.draw(render_pass);
    }
    if totes.instance_count > 0 {
        render_pass.set_pipeline(&totes.pipeline);
        totes.draw(render_pass);
    }
    if bots.instance_count > 0 {
        render_pass.set_pipeline(&bots.pipeline);
        bots.draw(render_pass);
    }
}
