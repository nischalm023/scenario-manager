//! The declared coordinate systems of a world, the one place a coordinate
//! changes frames, and the one place a step becomes a physical distance.
//!
//! SPEC §4's commitment is that conversion is defined in exactly one place — the
//! frame declarations — instead of being re-derived by each consumer from
//! constants that appear nowhere in the file. This module is that one place. It
//! resolves a frame's chain to the primary frame and pushes points through it,
//! so stitch matching and relay binding, which SPEC §5 evaluates in the primary
//! frame, never compare two numbers that were measured differently. A tiling's
//! exclusions ask nothing of it: a carve-out and the cells it removes are both
//! read in the tiling's own region frame, so that one comparison never crosses
//! frames at all.
//! Every conversion lands back on whole steps through SPEC §5's one rounding
//! rule, and the primary frame's `unit_mm` — declarable there and nowhere else —
//! is what turns the format's one surviving millimetre constant, a cell's
//! default footprint, into a step count.

use crate::components::spatial::{Frame, Region, DEFAULT_CELL_FOOTPRINT_MM};
use crate::entity::Entity;
use crate::error::LoadError;
use crate::ids::EntityId;
use crate::units::{Coord, Steps};
use std::collections::BTreeMap;

/// Every declared frame plus which one is primary.
#[derive(Debug, Clone)]
pub struct FrameGraph {
    primary: Option<EntityId>,
    frames: BTreeMap<EntityId, Frame>,
}

impl FrameGraph {
    /// Collect the frames out of a merged entity set, check that no frame is its
    /// own ancestor, and check that `unit_mm` sits on the primary frame and
    /// nowhere else. A cycle would make every position in the document
    /// unresolvable and a misplaced `unit_mm` would make a step mean two things,
    /// so both fail the load rather than the first conversion.
    pub fn build(
        entities: &BTreeMap<EntityId, Entity>,
        primary: Option<EntityId>,
    ) -> Result<FrameGraph, LoadError> {
        let mut frames = BTreeMap::new();
        for (id, entity) in entities {
            if let Some(frame) = entity.component::<Frame>("frame")? {
                frames.insert(id.clone(), frame);
            }
        }
        let graph = FrameGraph { primary, frames };
        for id in graph.frames.keys() {
            graph.chain(id)?;
        }
        for (id, frame) in &graph.frames {
            if frame.unit_mm.is_some() && Some(id) != graph.primary.as_ref() {
                return Err(LoadError::UnitOnNonPrimaryFrame { frame: id.clone() });
            }
        }
        if let Some(id) = &graph.primary {
            if let Some(frame) = graph.frames.get(id) {
                if frame.unit_mm.is_none() {
                    return Err(LoadError::PrimaryFrameWithoutUnit { frame: id.clone() });
                }
            }
        }
        Ok(graph)
    }

    pub fn primary(&self) -> Option<&EntityId> {
        self.primary.as_ref()
    }

    pub fn get(&self, id: &EntityId) -> Option<&Frame> {
        self.frames.get(id)
    }

    pub fn is_frame(&self, id: &EntityId) -> bool {
        self.frames.contains_key(id)
    }

    /// What one step of the primary frame is worth in millimetres (SPEC §4).
    pub fn unit_mm(&self) -> Option<f64> {
        self.primary
            .as_ref()
            .and_then(|id| self.frames.get(id))
            .and_then(|frame| frame.unit_mm)
    }

    /// The 900 mm a cell falls back to when nothing states its extent, as a
    /// count of the primary frame's steps.
    ///
    /// This is the one place the format's surviving physical constant meets the
    /// step lattice: SPEC §5 keeps the default in millimetres precisely because
    /// a default cannot be a step count until a document says what a step is, so
    /// a world whose primary frame says nothing cannot answer the question.
    pub fn default_cell_footprint(&self, cell: &EntityId) -> Result<[Steps; 2], LoadError> {
        let unit = self
            .unit_mm()
            .ok_or_else(|| LoadError::CellSizeWithoutUnit {
                entity: cell.clone(),
            })?;
        let side = Steps::round_half_up(DEFAULT_CELL_FOOTPRINT_MM / unit);
        Ok([side, side])
    }

    /// The frames from `id` up to the root, `id` first.
    fn chain(&self, id: &EntityId) -> Result<Vec<EntityId>, LoadError> {
        let mut out: Vec<EntityId> = Vec::new();
        let mut cursor = Some(id.clone());
        while let Some(current) = cursor {
            if out.contains(&current) {
                return Err(LoadError::FrameCycle { frame: current });
            }
            cursor = match self.frames.get(&current) {
                Some(frame) => frame.parent.clone(),
                // An unknown frame ends the chain here; check.rs reports it as
                // the dangling reference it is, with the entity that wrote it.
                None => None,
            };
            out.push(current);
        }
        Ok(out)
    }

    /// Push a planar point from `frame` all the way to the primary frame,
    /// landing on whole steps of it by SPEC §5's one rounding rule.
    pub fn to_primary(&self, frame: &EntityId, p: [Coord; 2]) -> Result<[Steps; 2], LoadError> {
        let xy = self.push(frame, [p[0].0 as f64, p[1].0 as f64])?;
        Ok([Steps::round_half_up(xy[0]), Steps::round_half_up(xy[1])])
    }

    /// The same push, left unrounded — for a caller composing several steps of
    /// arithmetic that must round once at the end rather than at every joint.
    pub fn to_primary_unrounded(
        &self,
        frame: &EntityId,
        p: [f64; 2],
    ) -> Result<[f64; 2], LoadError> {
        self.push(frame, p)
    }

    /// Push a DIRECTION from `frame` to the primary frame.
    ///
    /// A direction has no origin, so the chain's translations do not touch it;
    /// its scales do, and a mirrored axis turns it round — under `scale
    /// [-1, 1]` a heading along local `+x` points along primary `-x`. This is
    /// why a heading stated in a nested frame cannot be read as though it had
    /// been stated in the primary one, and why the answer comes back as a
    /// vector rather than an angle: the caller decides what counts as close
    /// enough to an axis.
    ///
    /// The result is not normalized — the chain's scales are still in it — so a
    /// caller comparing directions divides by the length first.
    pub fn direction_to_primary(
        &self,
        frame: &EntityId,
        d: [f64; 2],
    ) -> Result<[f64; 2], LoadError> {
        let chain = self.chain(frame)?;
        let mut xy = d;
        for id in &chain {
            let Some(f) = self.frames.get(id) else {
                break;
            };
            let Some(t) = &f.transform_to_parent else {
                continue;
            };
            for (axis, coord) in xy.iter_mut().enumerate() {
                *coord *= t.scale.get(axis).copied().unwrap_or(1.0);
            }
        }
        Ok(xy)
    }

    /// The frames from `id` up to the root, `id` first — the same chain every
    /// conversion walks, and the same cycle check.
    ///
    /// Exposed so a consumer can ask which enclosing frame a posed thing sits
    /// in (which machine owns a cell, say) without walking `parent` itself and
    /// writing a second, weaker version of the cycle test.
    pub fn ancestry(&self, id: &EntityId) -> Result<Vec<EntityId>, LoadError> {
        self.chain(id)
    }

    fn push(&self, frame: &EntityId, p: [f64; 2]) -> Result<[f64; 2], LoadError> {
        let chain = self.chain(frame)?;
        let mut xy = p;
        for id in &chain {
            let Some(f) = self.frames.get(id) else {
                break;
            };
            let Some(t) = &f.transform_to_parent else {
                continue;
            };
            for (axis, coord) in xy.iter_mut().enumerate() {
                let scale = t.scale.get(axis).copied().unwrap_or(1.0);
                let translation = t.translation.get(axis).copied().unwrap_or(0.0);
                *coord = translation + scale * *coord;
            }
        }
        Ok(xy)
    }

    /// Project a point from the primary frame back down into `frame` — the
    /// inverse of [`FrameGraph::to_primary`], applied down the chain instead of
    /// up it, and rounded by the same one rule.
    ///
    /// This is the direction the legacy lattice exists for: nothing is
    /// *authored* in that frame, but a legacy consumer needs to know which
    /// square a cell would land in. §4 calls the projection nominal and lossy by
    /// declaration, which is exactly why it is worth computing — two cells that
    /// land in one square are two cells a legacy exporter cannot tell apart.
    pub fn from_primary(&self, frame: &EntityId, p: [Steps; 2]) -> Result<[Coord; 2], LoadError> {
        let chain = self.chain(frame)?;
        let mut xy = [p[0].0 as f64, p[1].0 as f64];
        for id in chain.iter().rev() {
            let Some(f) = self.frames.get(id) else {
                continue;
            };
            let Some(t) = &f.transform_to_parent else {
                continue;
            };
            for (axis, coord) in xy.iter_mut().enumerate() {
                let scale = t.scale.get(axis).copied().unwrap_or(1.0);
                let translation = t.translation.get(axis).copied().unwrap_or(0.0);
                if scale == 0.0 {
                    return Err(LoadError::Unsupported {
                        entity: id.clone(),
                        message: format!(
                            "the frame's scale is 0 on the {} axis, so nothing can be projected \
                             back into it",
                            if axis == 0 { "x" } else { "y" }
                        ),
                    });
                }
                *coord = (*coord - translation) / scale;
            }
        }
        Ok([
            Coord(Steps::round_half_up(xy[0]).0),
            Coord(Steps::round_half_up(xy[1]).0),
        ])
    }

    /// The frames that hold integers: positions declared in one must be whole
    /// numbers, and projections into one round to the nearest (§4).
    pub fn integer_only(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.frames
            .iter()
            .filter(|(_, frame)| frame.integer_only)
            .map(|(id, _)| id.clone())
    }

    /// How an extent measured in `frame` stretches on the way to the primary
    /// frame. Signs are dropped: a footprint has no direction, and a mirrored
    /// axis (the legacy lattice's negative x) must not produce a negative width.
    pub fn extent_scale(&self, frame: &EntityId) -> Result<[f64; 2], LoadError> {
        let chain = self.chain(frame)?;
        let mut scale = [1.0, 1.0];
        for id in &chain {
            let Some(f) = self.frames.get(id) else {
                break;
            };
            let Some(t) = &f.transform_to_parent else {
                continue;
            };
            for (axis, s) in scale.iter_mut().enumerate() {
                *s *= t.scale.get(axis).copied().unwrap_or(1.0).abs();
            }
        }
        Ok(scale)
    }

    /// SPEC §5: geometry is stated in the steps of the frame it belongs to, and a
    /// frame whose units are `index` is the legacy lattice — something to project
    /// into, never to author in. Refusing here is what the `_mm` suffix used to
    /// promise.
    pub(crate) fn refuse_geometry_in_lattice_frames(
        &self,
        entities: &BTreeMap<EntityId, Entity>,
    ) -> Result<(), LoadError> {
        for entity in entities.values() {
            let Some(region) = entity.component::<Region>("region")? else {
                continue;
            };
            let Some(frame_id) = region.frame.as_ref() else {
                continue;
            };
            let Some(frame) = self.get(frame_id) else {
                // A frame nobody declared is a dangling reference, reported with the
                // entity that wrote it in the check step.
                continue;
            };
            if !frame.measures_length() {
                return Err(LoadError::GeometryInLatticeFrame {
                    entity: entity.id.clone(),
                    frame: frame_id.clone(),
                    units: frame.units.clone(),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::spatial::Transform;
    use serde_json::{json, Value};
    use std::str::FromStr;
    use uuid::Uuid;

    fn id(s: &str) -> EntityId {
        EntityId::declared(Uuid::from_str(s).unwrap())
    }

    fn frame_entity(idstr: &str, body: Value) -> Entity {
        let mut e = Entity::new(id(idstr));
        e.components.insert("frame".into(), body);
        e
    }

    /// The vendor frame of the example: `vda = [126960 + x, 143540 - y]` against
    /// floor steps, written as one negative scale entry. Pushing a point the
    /// other way — from the vendor frame into the primary — must undo it.
    #[test]
    fn a_signed_scale_carries_the_vendor_frames_y_flip() {
        let site = "f0000000-0000-4000-8000-000000000001";
        let floor = "f0000000-0000-4000-8000-000000000002";
        let vda = "f0000000-0000-4000-8000-000000000004";
        let mut entities = BTreeMap::new();
        for e in [
            frame_entity(site, json!({ "dims": 3, "units": "mm", "unit_mm": 1 })),
            frame_entity(
                floor,
                json!({ "parent": site, "dims": 2, "units": "mm",
                        "transform_to_parent": { "translation": [0, 0, 0], "scale": [1, 1] } }),
            ),
            frame_entity(
                vda,
                json!({ "parent": floor, "dims": 2, "units": "mm",
                        "transform_to_parent": { "translation": [-126960, 143540],
                                                 "scale": [1, -1] } }),
            ),
        ] {
            entities.insert(e.id.clone(), e);
        }
        let graph = FrameGraph::build(&entities, Some(id(site))).unwrap();
        let p = graph
            .to_primary(&id(vda), [Coord(126960 + 2450), Coord(143540 - 5750)])
            .unwrap();
        assert_eq!(p, [Steps(2450), Steps(5750)]);
        assert_eq!(graph.extent_scale(&id(vda)).unwrap(), [1.0, 1.0]);
    }

    /// The legacy lattice of the converted documents: a 750-step pitch, mirrored
    /// on x, and half a pitch of translation because an index names a square and
    /// a square's place is its centre. An extent measured in it takes neither
    /// the sign nor the offset — a footprint has no direction and no origin.
    #[test]
    fn the_lattice_frame_scales_extents_without_sign() {
        let floor = "f0000000-0000-4000-8000-000000000002";
        let lattice = "f0000000-0000-4000-8000-000000000003";
        let mut entities = BTreeMap::new();
        for e in [
            frame_entity(floor, json!({ "dims": 2, "units": "mm", "unit_mm": 1 })),
            frame_entity(
                lattice,
                json!({ "parent": floor, "dims": 2, "units": "index", "integer_only": true,
                        "transform_to_parent": { "translation": [-375, 375],
                                                 "scale": [-750, 750] } }),
            ),
        ] {
            entities.insert(e.id.clone(), e);
        }
        let graph = FrameGraph::build(&entities, Some(id(floor))).unwrap();
        // -375 + (-750 x 2) = -1875, and 375 + (750 x 3) = 2625.
        assert_eq!(
            graph
                .to_primary(&id(lattice), [Coord(2), Coord(3)])
                .unwrap(),
            [Steps(-1875), Steps(2625)]
        );
        assert_eq!(graph.extent_scale(&id(lattice)).unwrap(), [750.0, 750.0]);
    }

    /// Projecting the other way inverts the chain — the half-pitch translation
    /// off, then the signed pitch out — and lands on whole indices: SPEC §4 says
    /// a projection into an integer frame rounds half-up, and §5 says every
    /// computed value does, which is the same rule read twice.
    #[test]
    fn projecting_into_the_lattice_frame_inverts_the_chain_and_rounds() {
        let floor = "edd0316d-a9bc-48f4-bb37-a1594ced8cd1";
        let lattice = "af1f165f-c886-42df-9766-bafb062d5e56";
        let mut entities = BTreeMap::new();
        for e in [
            frame_entity(floor, json!({ "dims": 2, "units": "mm", "unit_mm": 1 })),
            frame_entity(
                lattice,
                json!({ "parent": floor, "dims": 2, "units": "index", "integer_only": true,
                        "transform_to_parent": { "translation": [-375, 375],
                                                 "scale": [-750, 750] } }),
            ),
        ] {
            entities.insert(e.id.clone(), e);
        }
        let graph = FrameGraph::build(&entities, Some(id(floor))).unwrap();
        assert_eq!(graph.integer_only().collect::<Vec<_>>(), vec![id(lattice)]);
        // The centre of square [2, 3] comes back as [2, 3], and the mirror comes
        // back with it: (-1875 + 375) / -750 = 2, (2625 - 375) / 750 = 3.
        let p = graph
            .from_primary(&id(lattice), [Steps(-1875), Steps(2625)])
            .unwrap();
        assert_eq!(p, [Coord(2), Coord(3)]);
        // A point off a square's centre folds onto the nearest square:
        // (-1725 + 375) / -750 = 1.8 and (825 - 375) / 750 = 0.6 both round up,
        // which is what makes the projection many-to-one.
        let q = graph
            .from_primary(&id(lattice), [Steps(-1725), Steps(825)])
            .unwrap();
        assert_eq!(q, [Coord(2), Coord(1)]);
    }

    /// A heading is a direction, and a mirrored frame turns a direction round.
    /// Local `+x` under `scale [-1, 1]` comes back pointing along primary `-x`,
    /// which is the whole reason a heading cannot be read straight off a pose.
    #[test]
    fn a_mirrored_frame_turns_a_local_heading_round() {
        let floor = "f0000000-0000-4000-8000-000000000002";
        let mirrored = "f0000000-0000-4000-8000-000000000005";
        let mut entities = BTreeMap::new();
        for e in [
            frame_entity(floor, json!({ "dims": 2, "units": "mm", "unit_mm": 1 })),
            frame_entity(
                mirrored,
                json!({ "parent": floor, "dims": 2, "units": "mm",
                        "transform_to_parent": { "translation": [8000, 0], "scale": [-1, 1] } }),
            ),
        ] {
            entities.insert(e.id.clone(), e);
        }
        let graph = FrameGraph::build(&entities, Some(id(floor))).unwrap();
        // Local +x comes back as primary -x; the translation is no part of it.
        assert_eq!(
            graph.direction_to_primary(&id(mirrored), [1.0, 0.0]).unwrap(),
            [-1.0, 0.0]
        );
        // The other axis is untouched, so a mirror is not a rotation.
        assert_eq!(
            graph.direction_to_primary(&id(mirrored), [0.0, 1.0]).unwrap(),
            [0.0, 1.0]
        );
        // And the ancestry a consumer walks is the same chain, root last.
        assert_eq!(
            graph.ancestry(&id(mirrored)).unwrap(),
            vec![id(mirrored), id(floor)]
        );
    }

    #[test]
    fn a_frame_that_is_its_own_ancestor_fails_the_load() {
        let a = "f0000000-0000-4000-8000-00000000000a";
        let b = "f0000000-0000-4000-8000-00000000000b";
        let mut entities = BTreeMap::new();
        for e in [
            frame_entity(a, json!({ "parent": b, "dims": 2, "units": "mm" })),
            frame_entity(b, json!({ "parent": a, "dims": 2, "units": "mm" })),
        ] {
            entities.insert(e.id.clone(), e);
        }
        assert!(matches!(
            FrameGraph::build(&entities, None),
            Err(LoadError::FrameCycle { .. })
        ));
    }

    /// A step becomes physical in exactly one place (SPEC §4), so a second
    /// frame claiming to say what one is worth fails the load, and a primary
    /// frame that says nothing fails it too.
    #[test]
    fn only_the_primary_frame_may_say_what_a_step_is_worth() {
        let site = "f0000000-0000-4000-8000-000000000001";
        let floor = "f0000000-0000-4000-8000-000000000002";
        let build = |site_body: Value, floor_body: Value| {
            let mut entities = BTreeMap::new();
            for e in [
                frame_entity(site, site_body),
                frame_entity(floor, floor_body),
            ] {
                entities.insert(e.id.clone(), e);
            }
            FrameGraph::build(&entities, Some(id(site)))
        };

        let graph = build(
            json!({ "dims": 3, "units": "mm", "unit_mm": 0.1 }),
            json!({ "parent": site, "dims": 2, "units": "mm" }),
        )
        .unwrap();
        assert_eq!(graph.unit_mm(), Some(0.1));
        // 900 mm at a tenth of a millimetre is 9 000 steps.
        assert_eq!(
            graph.default_cell_footprint(&id(floor)).unwrap(),
            [Steps(9000), Steps(9000)]
        );

        assert!(matches!(
            build(
                json!({ "dims": 3, "units": "mm", "unit_mm": 0.1 }),
                json!({ "parent": site, "dims": 2, "units": "mm", "unit_mm": 0.1 }),
            ),
            Err(LoadError::UnitOnNonPrimaryFrame { .. })
        ));
        assert!(matches!(
            build(
                json!({ "dims": 3, "units": "mm" }),
                json!({ "parent": site, "dims": 2, "units": "mm" }),
            ),
            Err(LoadError::PrimaryFrameWithoutUnit { .. })
        ));
    }

    #[test]
    fn a_missing_transform_is_the_identity() {
        let root = "f0000000-0000-4000-8000-000000000001";
        let child = "f0000000-0000-4000-8000-000000000002";
        let mut entities = BTreeMap::new();
        for e in [
            frame_entity(root, json!({ "dims": 3, "units": "mm", "unit_mm": 1 })),
            frame_entity(child, json!({ "parent": root, "dims": 2, "units": "mm" })),
        ] {
            entities.insert(e.id.clone(), e);
        }
        let graph = FrameGraph::build(&entities, Some(id(root))).unwrap();
        assert_eq!(
            graph
                .to_primary(&id(child), [Coord(10), Coord(20)])
                .unwrap(),
            [Steps(10), Steps(20)]
        );
        let _ = Transform {
            translation: vec![],
            scale: vec![],
        };
    }
}
