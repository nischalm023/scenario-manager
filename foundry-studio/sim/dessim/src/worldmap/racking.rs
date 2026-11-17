//! Within `worldmap`, `racking` is the SURVEY of a world's steel and of who
//! works it: which bays a run is made of, which handoff shelves sit on each bay
//! and which floor cell each of them binds, which aisle serves which face, and
//! how many climbing robots are assigned there.
//!
//! Everything here is read, not inferred. desmap had to rediscover this
//! structure from cell adjacency — a relay was any rack cell that happened to
//! touch open floor, a rack's side came from sharing a grid column with a relay
//! column — because its map format could not say it. A world document says it:
//! a `rack` enumerates its levels and the relay levels among them, the pipeline
//! binds each relay to the cell an HTM stands on, an `access` states which
//! region serves the storage face and who serves from it, and a `cohort` states
//! how many robots are assigned to the sector that names the run. This module
//! walks those statements and hands the result to `cells`, which mints the
//! indices, and to the zone and relay builders in the parent module.

use std::collections::{BTreeMap, BTreeSet};

use serde::de::DeserializeOwned;

use desgraph::{BlockSide, ZoneId};
use desmap::{FleetConfig, ZoneHolds};
// The deprecated `robot_class` still says which fleet a cohort joins, until
// that is read from the cohort's assignment.
#[allow(deprecated)]
use site_model::components::fleet::{Cohort, RobotClass};
use site_model::components::product::{CellAddress, Sector, Stamp};
use site_model::components::spatial::{Pose, Region};
use site_model::components::structure::Storage;
use site_model::components::structure::{Access, AccessCell, Rack, Relay, Side, Slot};
use site_model::derivation::KeySegment;
use site_model::entity::Entity;
use site_model::frames::FrameGraph;
use site_model::ids::EntityId;
use site_model::pipeline::World;
use site_model::units::{Coord, Degrees, Steps};

use super::{StepMm, WorldMapError};

/// The runs of a world, grouped the way dessim groups them: one entry per
/// region that an access is cut along.
pub(super) struct Survey {
    pub zones: Vec<ZoneDraft>,
}

/// One aisle and everything reached from it. This is desmap's "zone" without
/// the grid sandwich: a `LeftOnly` zone is one run beside this aisle, a `Dual`
/// zone is two runs facing each other across it, and both are one entry.
pub(super) struct ZoneDraft {
    pub aisle: EntityId,
    pub name: Option<String>,
    pub bays: Vec<BayDraft>,
    pub vtm_count: u32,
    /// What this aisle's racking holds, from the `storage` component on its
    /// bays. Unstated reads as inventory, which is what every document said
    /// before order buffers existed.
    ///
    /// `Order` only when EVERY bay of the aisle is: that aisle is an order
    /// buffer, worked exclusively and skipped wholesale by every reservation
    /// that is not a buffer release. An aisle with only SOME order bays -- the
    /// row of empty order totes at the south end of DFW6's storage block --
    /// is an inventory aisle that also holds the order-tote depot, and the
    /// discrimination for those bays happens per bay, not per zone.
    pub holds: ZoneHolds,
    /// The `configuration` of the robot class assigned here (gripper on a
    /// buffer aisle, vacuum on an inventory aisle). `None` when the document
    /// states no cohort for this zone.
    pub vtm_configuration: Option<String>,
    /// The robot entities the cohorts assigned here mint their VTMs from.
    pub vtm_classes: Vec<EntityId>,
    pub vtm_model_counts: BTreeMap<EntityId, usize>,
    /// How high the racking stands: the topmost shelf. See `PORT-NOTES.md`
    /// §5.4 for why the rack answers this and not the zone.
    pub height_mm: f64,
    /// Storage levels per bay, for the reports that still ask.
    pub levels: usize,
    /// The heights, bottom-up, of the stack of shelves a handoff position of
    /// this zone carries.
    pub board_floors_m: Vec<f64>,
}

/// One bay of a run, with the places dessim needs beside it: where its steel
/// stands, where in the aisle a climbing robot meets it, every storage position
/// in it, and every place somebody stands to reach its handoff shelves.
pub(super) struct BayDraft {
    pub rack: EntityId,
    pub centre: [f64; 2],
    pub aisle_centre: [f64; 2],
    pub side: BlockSide,
    /// What THIS BAY's steel is for, from its own `storage` component. A row
    /// of order racking can run across every aisle of a block without being an
    /// aisle itself, so the statement rides on the bay; the zone's `holds`
    /// above is the all-bays-agree summary of it.
    pub holds: ZoneHolds,
    pub stands: Vec<StandDraft>,
    pub slots: Vec<SlotDraft>,
    /// The steel's axis-aligned rectangle in millimetres: min corner and size,
    /// from the bay's own footprint pushed through its frame. This is what a
    /// renderer draws; everything else here is what a solver schedules.
    pub rect_mm: [f64; 4],
}

/// One place somebody stands to reach a bay's handoff shelves, and the stack of
/// shelves reachable from it.
///
/// This is what becomes a dessim relay — one per *standing position*, not one
/// per shelf. The protocol derives a relay entity per level per along-face
/// position and binds each to a cell of the floor, so cencora's two stacked
/// relay levels over three positions are six relay entities and three places to
/// stand. dessim exchanges at one height per zone and carries the rest of the
/// stack, so a dessim relay per protocol relay would double the zone's
/// reservable capacity and put half of it at the wrong height.
///
/// A bay whose run no region serves has shelves nobody can reach; the protocol
/// warns and binds none of them, and those produce no stand at all — the same
/// silence desmap kept by emitting no relay for a rack cell that touched no open
/// floor.
pub(super) struct StandDraft {
    /// The cell of the floor every shelf here binds — read from the relays'
    /// `access_cell`, which the pipeline filled in, never recomputed.
    pub cell: EntityId,
    /// Where the shelves actually sit, in the primary frame — the point a VTM
    /// travels to, which is not the cell an HTM stands on.
    pub exchange: [f64; 2],
    /// The stack at this position, bottom-up.
    pub board_floors_m: Vec<f64>,
    /// The protocol relays gathered here, in the order their heights run.
    pub shelves: Vec<EntityId>,
}

/// One storage position: which shelf, how high, and where along the run.
pub(super) struct SlotDraft {
    pub entity: EntityId,
    pub position: [f64; 2],
    pub floor_m: f64,
}

impl ZoneDraft {
    /// The aisle's usable travel, as the VTM cycle model measures it: the span
    /// of the very positions it normalises distances over, so the two cannot
    /// disagree.
    pub fn width_mm(&self) -> u64 {
        let positions = self.bays.iter().flat_map(|bay| {
            bay.slots.iter().map(|slot| slot.position).chain(bay.stands.iter().map(|stand| stand.exchange))
        });
        let mut lo = [f64::INFINITY; 2];
        let mut hi = [f64::NEG_INFINITY; 2];
        for p in positions {
            for axis in 0..2 {
                lo[axis] = lo[axis].min(p[axis]);
                hi[axis] = hi[axis].max(p[axis]);
            }
        }
        if !lo[0].is_finite() {
            return 0;
        }
        (hi[0] - lo[0]).max(hi[1] - lo[1]).round().max(0.0) as u64
    }
}

impl Survey {
    pub fn zone_aisles(&self) -> BTreeMap<ZoneId, EntityId> {
        self.zones.iter().enumerate().map(|(index, zone)| (ZoneId(index as u32 + 1), zone.aisle.clone())).collect()
    }

    /// The robot entities every zone's VTMs are minted from, sorted, each once:
    /// the entities that feed the simulator's VTM fleet.
    pub fn vtm_classes(&self) -> Vec<EntityId> {
        let mut classes: Vec<EntityId> = self.zones.iter().flat_map(|zone| zone.vtm_classes.iter().cloned()).collect();
        classes.sort();
        classes.dedup();
        classes
    }
}

/// Read the world's runs, accesses, racks and cohorts into the shape above.
pub(super) fn survey(world: &World, unit: StepMm) -> Result<Survey, WorldMapError> {
    let index = Index::build(world)?;
    let mut drafts: Vec<ZoneDraft> = Vec::new();

    for (run, access_id) in &index.access_by_run {
        let access: Access = component(world, access_id, "access")?;
        let aisle_mid = aisle_centre(world, &access.along, access_id, unit)?;
        let normal = world_normal_axis(world, &index, run, &access)?;
        let aisle_on_normal = aisle_mid[normal];

        let mut bays = Vec::new();
        let mut holds: Option<ZoneHolds> = None;
        let mut levels = 0usize;
        let mut height_mm = 0f64;
        let mut board_floors_m: Vec<f64> = Vec::new();

        for rack_id in index.bays_of(run).iter().cloned() {
            let rack: Rack = component(world, &rack_id, "rack")?;
            let frame = bay_frame(world, &rack_id)?;
            let centre = unit.point(frame.to_primary(world, [rack.footprint[0].0 / 2, rack.footprint[1].0 / 2])?);
            // Opposite corners through the same transform give the true
            // rectangle whatever the heading; a renderer wants min + size.
            let c0 = unit.point(frame.to_primary(world, [0, 0])?);
            let c1 = unit.point(frame.to_primary(world, [rack.footprint[0].0, rack.footprint[1].0])?);
            let rect_mm = [c0[0].min(c1[0]), c0[1].min(c1[1]), (c1[0] - c0[0]).abs(), (c1[1] - c0[1]).abs()];
            let mut aisle_pos = centre;
            aisle_pos[normal] = aisle_on_normal;
            let side = if centre[normal] < aisle_on_normal { BlockSide::Left } else { BlockSide::Right };

            // What the bay holds. SPEC fixes the vocabulary as `inventory` or
            // `order`; anything else is a document talking about something
            // this simulator does not model, and reading it as inventory would
            // silently seed SKUs into it.
            //
            // A RUN is one stamp of one template, so its bays cannot honestly
            // disagree; a template that makes them is refused. Whole AISLES
            // may disagree, and that is the order-tote row.
            let bay_entity = world.entities.get(&rack_id).ok_or_else(|| WorldMapError::Dangling {
                what: "bay",
                entity: run.clone(),
                target: rack_id.clone(),
            })?;
            let mut bay_holds = ZoneHolds::default();
            if let Some(storage) = optional::<Storage>(bay_entity, "storage")? {
                bay_holds = match storage.holds.as_str() {
                    "order" => ZoneHolds::Order,
                    "inventory" => ZoneHolds::Inventory,
                    other => {
                        return Err(WorldMapError::UnknownHolds { rack: rack_id.clone(), holds: other.to_string() });
                    }
                };
            }
            match holds {
                None => holds = Some(bay_holds),
                Some(seen) if seen != bay_holds => {
                    return Err(WorldMapError::MixedHolds { run: run.clone() });
                }
                Some(_) => {}
            }

            levels = levels.max(rack.storage_levels().count());
            height_mm = height_mm.max(top_shelf_mm(&rack, unit));

            let slots = bay_slots(world, &index, &rack_id, &rack, &frame, unit)?;
            let stands = bay_stands(world, &index, &rack_id, &rack, &frame, unit)?;
            for stand in &stands {
                if stand.board_floors_m.len() > board_floors_m.len() {
                    board_floors_m = stand.board_floors_m.clone();
                }
            }
            bays.push(BayDraft {
                rack: rack_id,
                centre,
                aisle_centre: aisle_pos,
                side,
                holds: bay_holds,
                stands,
                slots,
                rect_mm,
            });
        }

        drafts.push(ZoneDraft {
            aisle: access.along.clone(),
            name: world.entities.get(&access.along).and_then(|e| e.name.clone()),
            bays,
            vtm_count: 0,
            holds: holds.unwrap_or_default(),
            vtm_configuration: None,
            vtm_classes: Vec::new(),
            vtm_model_counts: BTreeMap::new(),
            height_mm,
            levels,
            board_floors_m,
        });
    }

    merge_shared_aisles(&mut drafts);

    // Storage in a zone no lane serves is storage dessim can never move a
    // tote into or out of: a VTM reaches the shelf and there is nowhere for
    // an HTM to stand and take the tote away. The protocol warns rather than
    // refusing, because 3,300 of its 3,609 converted runs are like this and
    // refusing them would refuse the installed base (SPEC §7); dessim is the
    // consumer that cannot operate such steel, and SPEC §7 says the refusal
    // is its own call to make.
    //
    // Scoped to (zone, SIDE), after the aisle merge, because that is the
    // service unit: `relays_for_rack` hands a rack every same-side relay of
    // its zone. A tunnel section with no boards of its own (boards at 394mm
    // would sit in the opening HTMs drive through) is served by the relay
    // bays beside it on the same side -- refusing per RUN would refuse that
    // legal steel. A SIDE with slots and no stands stays a refusal: the
    // other side's relays cannot reach across the aisle.
    for draft in &drafts {
        for side in [BlockSide::Left, BlockSide::Right] {
            let bays = draft.bays.iter().filter(|b| b.side == side);
            let (mut slots, mut stands) = (0usize, 0usize);
            for bay in bays {
                slots += bay.slots.len();
                stands += bay.stands.len();
            }
            if slots > 0 && stands == 0 {
                return Err(WorldMapError::StorageWithNoHandoff { run: draft.aisle.clone(), slots });
            }
        }
    }
    // Bays are ordered by where they stand, never by the identity the document
    // gives them. Two runs merge into one zone in whatever order the runs were
    // read, and the order decides `RackId` and `RelayId`; ordering by position
    // is what makes two documents describing one site run the same.
    for draft in &mut drafts {
        draft.bays.sort_by(|a, b| a.centre[0].total_cmp(&b.centre[0]).then(a.centre[1].total_cmp(&b.centre[1])));
    }
    drafts.sort_by(|a, b| {
        let corner = |z: &ZoneDraft| {
            z.bays.iter().fold([f64::INFINITY; 2], |acc, bay| [acc[0].min(bay.centre[0]), acc[1].min(bay.centre[1])])
        };
        let (left, right) = (corner(a), corner(b));
        left[1].total_cmp(&right[1]).then(left[0].total_cmp(&right[0])).then(a.aisle.cmp(&b.aisle))
    });
    assign_climbing_cohorts(world, &index, &mut drafts)?;
    Ok(Survey { zones: drafts })
}

/// A bay's own frame: where its minimum corner sits, which way it is turned,
/// and the coordinate frame all of that is measured in.
///
/// Everything inside a rack — a slot column's place along the face, a shelf's
/// along-face offset — is stated in the bay's local steps with `+x` along the
/// run and `+y` across it. This carries such a point out to the primary frame
/// the same way `site_model`'s own binding search carries a relay's point there,
/// step for step, so the exchange position dessim computes and the cell the
/// protocol bound cannot come from two different arithmetics.
struct BayFrame {
    anchor: [Coord; 2],
    heading: Degrees,
    frame: EntityId,
    /// The bay itself, so a frame-chain failure names the thing that was being
    /// placed rather than the frame it was placed in.
    bay: EntityId,
}

impl BayFrame {
    fn to_primary(&self, world: &World, local: [i64; 2]) -> Result<[Steps; 2], WorldMapError> {
        let turned = self.heading.rotate([local[0] as f64, local[1] as f64]);
        to_primary(
            &world.frames,
            &self.frame,
            [
                Coord(Steps::round_half_up(self.anchor[0].0 as f64 + turned[0]).0),
                Coord(Steps::round_half_up(self.anchor[1].0 as f64 + turned[1]).0),
            ],
            &self.bay,
        )
    }
}

fn bay_frame(world: &World, rack_id: &EntityId) -> Result<BayFrame, WorldMapError> {
    let pose: Pose = component(world, &rack_id, "pose")?;
    let (Some(frame), Some(anchor)) = (pose.frame.clone(), pose.xy()) else {
        return Err(WorldMapError::Unreadable {
            entity: rack_id.clone(),
            message: "a bay must be posed in a frame for its steel to have a place".into(),
        });
    };
    Ok(BayFrame { anchor, heading: pose.heading_deg, frame, bay: rack_id.clone() })
}

/// Every storage position of a bay, placed and given its shelf height.
///
/// The protocol gives a slot indices and no pose, so where a slot column sits
/// along the face has to be read off the steel around it: `slots_wide` columns
/// at `slot_pitch`, centred across the bay's own footprint. That reading
/// reproduces the surveyed column positions of the cencora prototype exactly
/// (320, 820, 1320, 1820 mm across a 2140 mm bay at a 500 mm pitch), which is
/// the check that it is a reading and not an invention. The deep axis reads
/// the same way: `slots_deep` rows at `slot_pitch[1]`, centred across the
/// footprint's depth — a one-deep bay lands on its depth centre (the old
/// behaviour, when no document was deeper than one), a double-deep bay puts
/// its back-to-back pair at the two half-depth centres.
fn bay_slots(
    world: &World,
    index: &Index,
    rack_id: &EntityId,
    rack: &Rack,
    frame: &BayFrame,
    unit: StepMm,
) -> Result<Vec<SlotDraft>, WorldMapError> {
    let wide = rack.slots_wide.max(1);
    let pitch = rack.slot_pitch.map_or(0, |p| p[0].0);
    let first = Steps::round_half_up((rack.footprint[0].0 - (wide as i64 - 1) * pitch) as f64 / 2.0);
    let deep = rack.slots_deep.max(1);
    let deep_pitch = rack.slot_pitch.map_or(0, |p| p[1].0);
    let deep_first = Steps::round_half_up((rack.footprint[1].0 - (deep as i64 - 1) * deep_pitch) as f64 / 2.0);

    // Ordered by the indices the protocol gives a slot, never by the identity it
    // gives one. `RackId` is this list's index, inventory is scattered over
    // those indices by a seeded shuffle, and ordering by UUID would place a
    // site's stock differently in two documents that describe it identically.
    let mut placed: Vec<(usize, usize, usize, EntityId)> = Vec::new();
    for slot_id in index.slots_of(rack_id).iter().cloned() {
        let slot: Slot = component(world, &slot_id, "slot")?;
        placed.push((slot.level, slot.col, slot.depth, slot_id.clone()));
    }
    placed.sort_unstable();

    let mut out = Vec::with_capacity(placed.len());
    for (level, col, depth, entity) in placed {
        let along = first.0 + col.min(wide - 1) as i64 * pitch;
        let out_pos = deep_first.0 + depth.min(deep - 1) as i64 * deep_pitch;
        out.push(SlotDraft {
            entity,
            position: unit.point(frame.to_primary(world, [along, out_pos])?),
            floor_m: shelf_height_mm(rack_id.clone(), rack, level, unit)? / 1000.0,
        });
    }
    Ok(out)
}

/// Every place somebody stands to reach a bay's handoff shelves, with the stack
/// of shelves at each and the point on the face a VTM travels to.
///
/// The grouping is the whole of it. A relay entity is one shelf — one level, one
/// offset along the face — and the pipeline has already told each of them which
/// cell of the floor an HTM occupies to reach it (SPEC §5). Shelves that name
/// the same cell are one place to stand, so this gathers a bay's relays by the
/// cell they bound and hands each group over as one exchange point with the
/// stack behind it. cencora is the case it exists for: two relay levels over
/// three along-face positions derive six relays per bay, and both levels at a
/// position find the same nearest cell without being told to, so six shelves
/// become three exchange points — the number desmap compiles.
///
/// The grouping is per *bay*, not per cell across the world: where relay pitch
/// and cell pitch are incommensurate, neighbouring bays share the cell somebody
/// stands on, and their shelves are still on different steel.
fn bay_stands(
    world: &World,
    index: &Index,
    rack_id: &EntityId,
    rack: &Rack,
    frame: &BayFrame,
    unit: StepMm,
) -> Result<Vec<StandDraft>, WorldMapError> {
    let mut shelves: Vec<Shelf> = Vec::new();
    for relay_id in index.relays_of(rack_id).iter().cloned() {
        let Some(entity) = world.entities.get(&relay_id) else { continue };
        let relay: Relay = required(entity, "relay")?;
        // A run none of whose relays reaches a cell binds nothing and warns
        // (SPEC §7); those shelves simply have no place to stand.
        let Some(binding) = optional::<AccessCell>(entity, "access_cell")? else { continue };
        let cell = match binding.cell {
            // A position means "the binder searched from here", so by the time
            // a consumer reads it the stated cell is what the search found.
            CellAddress::At(_) => continue,
            CellAddress::Id(id) => id,
            CellAddress::Derived(raw) => raw
                .parse()
                .map_err(|source| WorldMapError::Unreadable {
                    entity: relay_id.clone(),
                    message: source.to_string(),
                })?
                .into(),
            CellAddress::Local(_) | CellAddress::Handle(_) => {
                return Err(WorldMapError::Unreadable {
                    entity: relay_id,
                    message: "the binding names a template-local cell outside any template".into(),
                });
            }
        };
        rack.levels.get(relay.level).and_then(|level| level.relay()).ok_or(WorldMapError::Unreadable {
            entity: relay_id.clone(),
            message: "the shelf names a level of its bay that carries no relays".into(),
        })?;
        // The level's positions are [along, out] offsets from the bay's
        // origin corner, in the bay's own frame.
        let _local = [relay.offset[0].0, relay.offset[1].0];
        // The VTM meets the board where it presents: at the bay's FACE, not at
        // the stand under it. A document that pulls its relay points inward
        // (so the stand binds under-rack floor) would otherwise put every
        // docked VTM inside the steel. The face is whichever across-bay edge
        // the offset leans toward.
        let face_out = if relay.offset[1].0 * 2 < rack.footprint[1].0 { 0 } else { rack.footprint[1].0 };
        let exchange_local = [relay.offset[0].0, face_out];
        shelves.push(Shelf {
            level: relay.level,
            index: relay.index,
            entity: relay_id,
            cell,
            height_mm: unit.length(relay.height),
            exchange: unit.point(frame.to_primary(world, exchange_local)?),
        });
    }
    shelves.sort_by_key(|shelf| (shelf.level, shelf.index));

    let mut out: Vec<StandDraft> = Vec::new();
    for shelf in shelves {
        match out.iter_mut().find(|stand| stand.cell == shelf.cell) {
            Some(stand) => {
                stand.shelves.push(shelf.entity);
                stand.board_floors_m.push(shelf.height_mm / 1000.0);
            }
            None => out.push(StandDraft {
                cell: shelf.cell,
                exchange: shelf.exchange,
                board_floors_m: vec![shelf.height_mm / 1000.0],
                shelves: vec![shelf.entity],
            }),
        }
    }
    for stand in &mut out {
        stand.board_floors_m.sort_by(f64::total_cmp);
        stand.board_floors_m.dedup();
    }
    // Exchange points are ordered by where they stand, like everything else the
    // survey hands out an index for.
    out.sort_by(|a, b| a.exchange[0].total_cmp(&b.exchange[0]).then(a.exchange[1].total_cmp(&b.exchange[1])));
    Ok(out)
}

/// One handoff shelf of a bay on its way into an exchange point: which level of
/// the steel it is, which cell the pipeline bound it to, and where it sits.
struct Shelf {
    level: usize,
    index: usize,
    entity: EntityId,
    cell: EntityId,
    height_mm: f64,
    exchange: [f64; 2],
}

/// How high one shelf sits above the floor at the bay's pose.
///
/// A rack's vertical truth is one enumerated list and a level index names an
/// entry of it, so this is a lookup rather than a calculation (SPEC §5). A
/// height crosses into millimetres on the primary frame's step alone: a 2-D
/// frame under a 3-D parent carries a two-element scale, so nothing in the
/// format stretches a vertical length.
fn shelf_height_mm(rack_id: EntityId, rack: &Rack, level: usize, unit: StepMm) -> Result<f64, WorldMapError> {
    rack.levels.get(level).map(|level| unit.length(level.height)).ok_or(WorldMapError::Unreadable {
        entity: rack_id,
        message: format!("a slot names level {level}, which the bay does not have"),
    })
}

/// How high the racking stands: its topmost shelf. The list is enumerated
/// bottom-up and strictly increasing, so the last entry is the highest.
fn top_shelf_mm(rack: &Rack, unit: StepMm) -> f64 {
    rack.levels.last().map_or(0.0, |level| unit.length(level.height))
}

/// The floor fleet: how many robots of each floor-driving role the world's
/// cohorts mint, and which robot entities mint the HTMs. Climbing robots are
/// absent on purpose — dessim derives the VTM fleet from the zones they are
/// assigned to, so they are counted there.
pub(super) struct FloorFleet {
    /// `None` when the document declares no floor cohort at all, which is a
    /// different statement from declaring one of size zero: unstated leaves
    /// dessim its own system-type default, stated-zero means a world with no
    /// floor robots in it.
    pub counts: Option<FleetConfig>,
    /// The robot entities the HTM cohorts mint from, sorted, each once: the
    /// entities that feed the simulator's HTM fleet.
    pub htm_classes: Vec<EntityId>,
    pub htm_model_counts: BTreeMap<EntityId, usize>,
}

#[allow(deprecated)]
pub(super) fn floor_fleet(world: &World) -> Result<FloorFleet, WorldMapError> {
    let mut fleet = FleetConfig::default();
    let mut htm_classes = Vec::new();
    let mut htm_model_counts = BTreeMap::new();
    let mut stated = false;
    for entity in world.entities.values() {
        let Some(cohort) = optional::<Cohort>(entity, "cohort")? else { continue };
        let Some(class) = world.entities.get(&cohort.robot_class) else {
            return Err(WorldMapError::Dangling {
                what: "cohort's robot class",
                entity: entity.id.clone(),
                target: cohort.robot_class,
            });
        };
        let Some(class) = optional::<RobotClass>(class, "robot_class")? else { continue };
        match class.role.as_str() {
            "htm" => {
                fleet.htms += cohort.count;
                if cohort.count > 0 {
                    htm_classes.push(cohort.robot_class.clone());
                    *htm_model_counts.entry(cohort.robot_class.clone()).or_default() += cohort.count;
                }
                stated = true;
            }
            "rtp" => {
                fleet.rtps += cohort.count;
                stated = true;
            }
            _ => {}
        }
    }
    htm_classes.sort();
    htm_classes.dedup();
    Ok(FloorFleet { counts: stated.then_some(fleet), htm_classes, htm_model_counts })
}

// ---------------------------------------------------------------------------
// The lookups the survey walks
// ---------------------------------------------------------------------------

/// The cross-references a survey needs in the direction it reads them: from a
/// run to the region it is worked from, from a run to its bays, from a bay to
/// its shelves and its storage.
struct Index {
    access_by_run: BTreeMap<EntityId, EntityId>,
    slots_by_rack: BTreeMap<EntityId, Vec<EntityId>>,
    relays_by_rack: BTreeMap<EntityId, Vec<EntityId>>,
    bays: BTreeMap<EntityId, Vec<EntityId>>,
}

impl Index {
    fn build(world: &World) -> Result<Index, WorldMapError> {
        let mut access_by_run: BTreeMap<EntityId, Vec<EntityId>> = BTreeMap::new();
        let mut children: BTreeMap<EntityId, Vec<EntityId>> = BTreeMap::new();
        let mut slots_by_rack: BTreeMap<EntityId, Vec<EntityId>> = BTreeMap::new();
        let mut relays_by_rack: BTreeMap<EntityId, Vec<EntityId>> = BTreeMap::new();

        for entity in world.entities.values() {
            if let Some(parent) = entity.parent.clone() {
                children.entry(parent).or_default().push(entity.id.clone());
            }
            if let Some(access) = optional::<Access>(entity, "access")? {
                access_by_run.entry(access.rack).or_default().push(entity.id.clone());
            }
            if let Some(slot) = optional::<Slot>(entity, "slot")? {
                slots_by_rack.entry(slot.rack).or_default().push(entity.id.clone());
            }
            if let Some(relay) = optional::<Relay>(entity, "relay")? {
                relays_by_rack.entry(relay.rack).or_default().push(entity.id.clone());
            }
        }
        for positions in slots_by_rack.values_mut() {
            positions.sort_unstable();
        }
        for shelves in relays_by_rack.values_mut() {
            shelves.sort_unstable();
        }

        let mut single = BTreeMap::new();
        for (run, accesses) in access_by_run {
            if accesses.len() > 1 {
                return Err(WorldMapError::RunWithTwoServingFaces { run, count: accesses.len() });
            }
            single.insert(run, accesses[0].clone());
        }

        let mut bays = BTreeMap::new();
        for run in single.keys() {
            bays.insert(run.clone(), run_bays(world, run.clone(), &children)?);
        }
        Ok(Index { access_by_run: single, slots_by_rack, relays_by_rack, bays })
    }

    fn bays_of(&self, run: &EntityId) -> &[EntityId] {
        self.bays.get(run).map(Vec::as_slice).unwrap_or(&[])
    }

    fn slots_of(&self, rack: &EntityId) -> &[EntityId] {
        self.slots_by_rack.get(rack).map(Vec::as_slice).unwrap_or(&[])
    }

    fn relays_of(&self, rack: &EntityId) -> &[EntityId] {
        self.relays_by_rack.get(rack).map(Vec::as_slice).unwrap_or(&[])
    }
}

/// The bays of a run, in the order the stamp placed them. A stamp always
/// expands through `instance:{i}` (SPEC §3), so the order is the repeat's own.
fn run_bays(
    world: &World,
    run: EntityId,
    children: &BTreeMap<EntityId, Vec<EntityId>>,
) -> Result<Vec<EntityId>, WorldMapError> {
    let Some(entity) = world.entities.get(&run) else {
        return Err(WorldMapError::Dangling { what: "access's run", entity: run.clone(), target: run });
    };
    let stamp: Stamp = required(entity, "stamp")?;
    let count = stamp.repeat.as_ref().map_or(1, |r| r.count);
    let mut out = Vec::with_capacity(count);
    for bay in 0..count {
        let instance: EntityId = run.derive(KeySegment::parse(&format!("instance:{bay}")).expect("numeric bay index forms a key segment")).into();
        // Every rack the instance holds, not just the first: a template may
        // carry more than one bay (a relay bay and its plain neighbour, say),
        // and each is steel this run stands for. Sorted by identity, which for
        // template locals is their authored order (`local:rack` < `local:rack_b`)
        // and keeps the downstream RackId enumeration deterministic.
        let mut racks: Vec<EntityId> = children
            .get(&instance)
            .into_iter()
            .flatten()
            .filter(|id| world.entities.get(id).is_some_and(|e| e.has("rack")))
            .cloned()
            .collect();
        racks.sort();
        match racks.is_empty() {
            false => out.extend(racks),
            true => {
                return Err(WorldMapError::Unreadable {
                    entity: instance,
                    message: "a run's instance holds no entity with a `rack` component, so the bay \
                              has no steel to stand for"
                        .into(),
                });
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// The serving region's midpoint, used to identify rack sides and seed initial
/// VTM coordinates along the aisle. It does not describe equipment or a stop.
fn aisle_centre(world: &World, along: &EntityId, access: &EntityId, unit: StepMm) -> Result<[f64; 2], WorldMapError> {
    let Some(entity) = world.entities.get(along) else {
        return Err(WorldMapError::Dangling { what: "access's region", entity: access.clone(), target: along.clone() });
    };
    let region: Region = required(entity, "region")?;
    let Some(frame) = region.frame.clone() else {
        return Err(WorldMapError::Unreadable {
            entity: along.clone(),
            message: "a region an access names must be posed in a frame".into(),
        });
    };
    let min = to_primary(&world.frames, &frame, [Coord(region.origin[0].0), Coord(region.origin[1].0)], along)?;
    let corner = region.max_corner();
    let max = to_primary(&world.frames, &frame, [Coord(corner[0].0), Coord(corner[1].0)], along)?;
    Ok([unit.length(Steps((min[0].0 + max[0].0) / 2)), unit.length(Steps((min[1].0 + max[1].0) / 2))])
}

/// Which world axis the serving face looks along, once the run's heading has
/// turned it. Computed by pushing the face's own outward step through the frame
/// chain rather than by reasoning about signs, so a mirrored or scaled frame
/// cannot flip it behind the reader's back.
fn world_normal_axis(world: &World, index: &Index, run: &EntityId, access: &Access) -> Result<usize, WorldMapError> {
    let Some(rack_id) = index.bays_of(run).first() else {
        return Err(WorldMapError::Unreadable {
            entity: run.clone(),
            message: "a run with no bays has no face for an access to name".into(),
        });
    };
    let frame = bay_frame(world, rack_id)?;
    // Binding is center-on-center, so the side needs no axis helpers;
    // which axis a face is normal to is still plain geometry.
    let (normal_axis, outward) = match access.face {
        Side::PlusX => (0, 1),
        Side::MinusX => (0, -1),
        Side::PlusY => (1, 1),
        Side::MinusY => (1, -1),
    };
    let mut step = [0i64, 0i64];
    step[normal_axis] = outward;
    let origin = frame.to_primary(world, [0, 0])?;
    let ahead = frame.to_primary(world, step)?;
    let delta = [ahead[0].0 - origin[0].0, ahead[1].0 - origin[1].0];
    Ok(usize::from(delta[1].abs() > delta[0].abs()))
}

// ---------------------------------------------------------------------------
// Cohorts
// ---------------------------------------------------------------------------

/// Count each zone's climbing robots by following the references the document
/// wrote: a cohort names a class and an assignment, the assignment names a
/// sector, and the sector names the run.
#[allow(deprecated)]
fn assign_climbing_cohorts(world: &World, index: &Index, drafts: &mut [ZoneDraft]) -> Result<(), WorldMapError> {
    let mut zone_of_run: BTreeMap<EntityId, usize> = BTreeMap::new();
    for (position, draft) in drafts.iter().enumerate() {
        for (run, access) in &index.access_by_run {
            let served: Access = component(world, &access, "access")?;
            if served.along == draft.aisle {
                zone_of_run.insert(run.clone(), position);
            }
        }
    }

    for entity in world.entities.values() {
        let Some(cohort) = optional::<Cohort>(entity, "cohort")? else { continue };
        let Some(class_entity) = world.entities.get(&cohort.robot_class) else {
            return Err(WorldMapError::Dangling {
                what: "cohort's robot class",
                entity: entity.id.clone(),
                target: cohort.robot_class,
            });
        };
        let Some(class) = optional::<RobotClass>(class_entity, "robot_class")? else { continue };
        if class.role != "vtm" {
            continue;
        }
        let Some(assignment) = cohort.assignment.as_ref().filter(|a| a.is_hard()) else { continue };
        // A cohort works the zones its assignment reaches, and it is the same
        // robots in each. A `Dual` zone's sector names two runs across one
        // aisle, so counting per run would deploy the cohort twice.
        let mut reached: BTreeSet<usize> = BTreeSet::new();
        for target in &assignment.to {
            for run in members(world, target.clone())? {
                if let Some(&position) = zone_of_run.get(&run) {
                    reached.insert(position);
                }
            }
        }
        if cohort.count == 0 {
            continue;
        }
        for position in reached {
            drafts[position].vtm_count += cohort.count as u32;
            drafts[position].vtm_classes.push(cohort.robot_class.clone());
            *drafts[position].vtm_model_counts.entry(cohort.robot_class.clone()).or_default() += cohort.count;
            // Which hardware works this aisle. A gripper cannot do a vacuum
            // head's job or the reverse, so two cohorts of different
            // configuration over one aisle is a document describing a machine
            // that does not exist.
            match drafts[position].vtm_configuration.as_deref() {
                None => drafts[position].vtm_configuration = class.configuration.clone(),
                Some(seen) if Some(seen) != class.configuration.as_deref() => {
                    return Err(WorldMapError::MixedVtmConfiguration {
                        aisle: drafts[position].aisle.clone(),
                        first: seen.to_string(),
                        second: class.configuration.clone().unwrap_or_else(|| "unstated".into()),
                    });
                }
                Some(_) => {}
            }
        }
    }
    Ok(())
}

/// What an assignment target stands for: a sector stands for its members, and
/// anything else stands for itself.
fn members(world: &World, target: EntityId) -> Result<Vec<EntityId>, WorldMapError> {
    let Some(entity) = world.entities.get(&target) else {
        return Err(WorldMapError::Dangling { what: "assignment target", entity: target.clone(), target });
    };
    match optional::<Sector>(entity, "sector")? {
        Some(sector) if !sector.members.is_empty() => Ok(sector.members),
        _ => Ok(vec![target]),
    }
}

/// Two runs cut along one aisle are one zone — desmap's `Dual` sandwich, said
/// by two accesses naming one region instead of by a three-column footprint.
fn merge_shared_aisles(drafts: &mut Vec<ZoneDraft>) {
    let mut merged: Vec<ZoneDraft> = Vec::with_capacity(drafts.len());
    for draft in drafts.drain(..) {
        match merged.iter_mut().find(|existing| existing.aisle == draft.aisle) {
            Some(existing) => {
                existing.height_mm = existing.height_mm.max(draft.height_mm);
                existing.levels = existing.levels.max(draft.levels);
                // An aisle is an order BUFFER only if every run of it is. The
                // merge used to keep whichever run was read first and drop the
                // other's answer outright, so an aisle with one order run and
                // one inventory run called itself whichever the read order
                // happened to reach first -- and every zone-wide racking rule
                // downstream took that at face value.
                if draft.holds != ZoneHolds::Order {
                    existing.holds = draft.holds;
                }
                // The board stack must survive whichever draft arrives
                // first: an aisle whose first-merged run is a relay-less
                // bay group used to keep that run's EMPTY stack, and every
                // relay in the zone lost its board floors (and with them,
                // its rendered tabs).
                if draft.board_floors_m.len() > existing.board_floors_m.len() {
                    existing.board_floors_m = draft.board_floors_m;
                }
                existing.bays.extend(draft.bays);
            }
            None => merged.push(draft),
        }
    }
    *drafts = merged;
}

// ---------------------------------------------------------------------------
// Small readers
// ---------------------------------------------------------------------------

fn component<T: DeserializeOwned>(world: &World, id: &EntityId, name: &'static str) -> Result<T, WorldMapError> {
    let Some(entity) = world.entities.get(id) else {
        return Err(WorldMapError::Dangling { what: name, entity: id.clone(), target: id.clone() });
    };
    required(entity, name)
}

fn required<T: DeserializeOwned>(entity: &Entity, name: &'static str) -> Result<T, WorldMapError> {
    entity
        .require::<T>(name)
        .map_err(|source| WorldMapError::Unreadable { entity: entity.id.clone(), message: source.to_string() })
}

pub(super) fn optional<T: DeserializeOwned>(entity: &Entity, name: &str) -> Result<Option<T>, WorldMapError> {
    entity
        .component::<T>(name)
        .map_err(|source| WorldMapError::Unreadable { entity: entity.id.clone(), message: source.to_string() })
}

fn to_primary(
    frames: &FrameGraph,
    frame: &EntityId,
    point: [Coord; 2],
    entity: &EntityId,
) -> Result<[Steps; 2], WorldMapError> {
    frames
        .to_primary(frame, point)
        .map_err(|source| WorldMapError::Unreadable { entity: entity.clone(), message: source.to_string() })
}
