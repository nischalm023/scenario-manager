//! Within `flowgraph`, the `site` is what the compiler reads of a loaded world besides its flows (spec 06 section
//! 3.1's member table and section 3.2's executor checks): sectors and their members, the facts each member states,
//! what the members offer each port, and the fleet's cohorts. It reads through dessim's own reader (`WorldMap`), so
//! a node's member count and the stations a run uses come from one resolver (research w3-01, section 7).

use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::Value;
use site_model::components::fleet::Cohort;
use site_model::components::product::{Port as WorldPort, Sector};
use site_model::components::spatial::Pose;
use site_model::entity::Entity;
use site_model::ids::EntityId;
use site_model::pipeline::World;

use crate::scenario_gen::RackUse;
use crate::worldmap::WorldMap;

/// A cohort as the run will mint it: how many, of which class, and for a hard assignment what it reaches.
#[derive(Debug, Clone)]
pub(super) struct CohortFacts {
    pub id: EntityId,
    pub count: usize,
    /// `robot_class.role`, which dessim's reader sorts a cohort into a fleet by (`htm`, `vtm`).
    pub role: Option<String>,
    /// Each target of a hard assignment, a sector standing for its members one level deep, as
    /// `worldmap/racking.rs` reads assignments; `None` for a free cohort.
    pub reach: Option<FxHashSet<EntityId>>,
}

/// A belt machine's stated places.
#[derive(Debug, Clone, Default)]
pub(super) struct MachineFacts {
    pub sinks: u32,
    pub sources: u32,
    pub order_presentations: u32,
    /// `go.easyloop.time_per_pick_secs`, the figure the map states for its operators.
    pub time_per_pick_secs: Option<f64>,
}

pub(super) struct Site<'w> {
    pub world: &'w World,
    bays_under: FxHashMap<EntityId, Vec<EntityId>>,
    bay_use: FxHashMap<EntityId, RackUse>,
    slots: FxHashMap<EntityId, u32>,
    relays: FxHashMap<EntityId, u32>,
    presentations: FxHashMap<EntityId, Vec<Option<String>>>,
    machines: Vec<EntityId>,
    machine_facts: FxHashMap<EntityId, MachineFacts>,
    cohorts: Vec<CohortFacts>,
}

fn raw_str<'a>(entity: &'a Entity, component: &str, field: &str) -> Option<&'a str> {
    entity.raw(component)?.get(field)?.as_str()
}

fn raw_id(entity: &Entity, component: &str, field: &str) -> Option<EntityId> {
    raw_str(entity, component, field)?.split('@').next()?.parse().ok()
}

impl<'w> Site<'w> {
    pub fn read(map: &'w WorldMap) -> Self {
        let world = map.world.as_ref();
        let mut site = Site {
            world,
            bays_under: FxHashMap::default(),
            bay_use: FxHashMap::default(),
            slots: FxHashMap::default(),
            relays: FxHashMap::default(),
            presentations: FxHashMap::default(),
            machines: Vec::new(),
            machine_facts: FxHashMap::default(),
            cohorts: Vec::new(),
        };
        for entity in world.entities.values() {
            site.note(entity);
        }
        for bays in site.bays_under.values_mut() {
            bays.sort();
        }
        site.machines.sort();
        site.count_order_presentations();
        // The racking use dessim's reader gave each storage position, carried up to the bay it is on.
        for (rack, slot) in &map.index.storage_slots {
            let bay = world.entities.get(slot).and_then(|slot| raw_id(slot, "slot", "rack"));
            if let (Some(bay), Some(column)) = (bay, map.racking.columns.get(rack.0 as usize)) {
                site.bay_use.entry(bay).or_insert(column.uses);
            }
        }
        site
    }

    fn note(&mut self, entity: &Entity) {
        if entity.has("rack") {
            let mut above = entity.parent.clone();
            while let Some(parent) = above {
                self.bays_under.entry(parent.clone()).or_default().push(entity.id.clone());
                above = self.world.entities.get(&parent).and_then(|up| up.parent.clone());
            }
        }
        if let Some(bay) = raw_id(entity, "slot", "rack") {
            *self.slots.entry(bay).or_default() += 1;
        }
        if let Some(bay) = raw_id(entity, "relay", "rack").filter(|_| entity.has("access_cell")) {
            *self.relays.entry(bay).or_default() += 1;
        }
        if let Some(station) = raw_id(entity, "presentation_point", "station") {
            let serves = raw_str(entity, "presentation_point", "for").map(str::to_owned);
            self.presentations.entry(station).or_default().push(serves);
        }
        if entity.has("go.easyloop") && entity.has("frame") {
            self.machines.push(entity.id.clone());
            let figure =
                entity.raw("go.easyloop").and_then(|belt| belt.get("time_per_pick_secs")).and_then(Value::as_f64);
            self.machine_facts.entry(entity.id.clone()).or_default().time_per_pick_secs = figure;
        }
        if let Ok(Some(port)) = entity.component::<WorldPort>("port") {
            let (machine, sink) = match port.direction.as_str() {
                "sink" => (port.into, true),
                _ => (port.from, false),
            };
            if let Some(machine) = machine {
                let facts = self.machine_facts.entry(machine).or_default();
                if sink {
                    facts.sinks += 1;
                } else {
                    facts.sources += 1;
                }
            }
        }
        if let Ok(Some(cohort)) = entity.component::<Cohort>("cohort") {
            let role =
                self.world.entities.get(&cohort.robot_class).and_then(|class| raw_str(class, "robot_class", "role"));
            let reach = cohort.assignment.as_ref().filter(|assignment| assignment.is_hard()).map(|assignment| {
                assignment
                    .to
                    .iter()
                    .flat_map(|target| self.sector(target).unwrap_or_else(|| vec![target.clone()]))
                    .collect()
            });
            self.cohorts.push(CohortFacts {
                id: entity.id.clone(),
                count: cohort.count,
                role: role.map(str::to_owned),
                reach,
            });
        }
    }

    /// An order presentation is a pick place posed in a machine's frame (`worldmap/machine.rs`: the machine IS a
    /// frame, and every cell posed in it is a member).
    fn count_order_presentations(&mut self) {
        for entity in self.world.entities.values() {
            if raw_str(entity, "presentation_point", "for") != Some("order_carrier") {
                continue;
            }
            let Some(frame) = entity.component::<Pose>("pose").ok().flatten().and_then(|pose| pose.frame) else {
                continue;
            };
            let ancestry = self.world.frames.ancestry(&frame).unwrap_or_default();
            if let Some(machine) = ancestry.iter().find(|frame| self.machines.binary_search(frame).is_ok()) {
                self.machine_facts.entry(machine.clone()).or_default().order_presentations += 1;
            }
        }
    }

    pub fn entity(&self, id: &EntityId) -> Option<&'w Entity> {
        self.world.entities.get(id)
    }

    pub fn name(&self, id: &EntityId) -> Option<&'w str> {
        self.entity(id).and_then(|entity| entity.name.as_deref())
    }

    pub fn sector(&self, id: &EntityId) -> Option<Vec<EntityId>> {
        self.entity(id)?.component::<Sector>("sector").ok().flatten().map(|sector| sector.members)
    }

    pub fn station(&self, id: &EntityId) -> Option<(&'w str, Option<&'w str>)> {
        let entity = self.entity(id)?;
        Some((raw_str(entity, "station", "kind")?, raw_str(entity, "station", "role")))
    }

    /// What each presentation point naming the station presents (`presentation_point.for`).
    pub fn presentations(&self, station: &EntityId) -> &[Option<String>] {
        self.presentations.get(station).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The stations the map holds whose kind is one of `kinds` and, when given, whose role is `role`.
    pub fn stations(&self, kinds: &[&str], role: Option<&str>) -> Vec<EntityId> {
        let mut out: Vec<EntityId> = (self.world.entities.values())
            .filter(|entity| raw_str(entity, "station", "kind").is_some_and(|kind| kinds.contains(&kind)))
            .filter(|entity| role.is_none() || raw_str(entity, "station", "role") == role)
            .map(|entity| entity.id.clone())
            .collect();
        out.sort();
        out
    }

    /// The bays a storage member stands for: itself when it is one, else every bay its expansion holds.
    pub fn bays_of(&self, member: &EntityId) -> Vec<EntityId> {
        match self.entity(member) {
            Some(entity) if entity.has("rack") => vec![member.clone()],
            _ => self.bays_under.get(member).cloned().unwrap_or_default(),
        }
    }

    /// A bay's `storage.holds`; unstated reads as inventory, as dessim's reader reads it.
    pub fn bay_holds(&self, bay: &EntityId) -> &'w str {
        self.entity(bay).and_then(|entity| raw_str(entity, "storage", "holds")).unwrap_or("inventory")
    }

    pub fn bay_use(&self, bay: &EntityId) -> Option<RackUse> {
        self.bay_use.get(bay).copied()
    }

    /// Every bay the run seeds with `uses`, in identity order.
    pub fn bays_used_as(&self, uses: RackUse) -> Vec<EntityId> {
        let mut out: Vec<EntityId> =
            self.bay_use.iter().filter(|(_, bay_use)| **bay_use == uses).map(|(bay, _)| bay.clone()).collect();
        out.sort();
        out
    }

    pub fn slots_on(&self, bay: &EntityId) -> u32 {
        self.slots.get(bay).copied().unwrap_or(0)
    }

    pub fn relays_on(&self, bay: &EntityId) -> u32 {
        self.relays.get(bay).copied().unwrap_or(0)
    }

    pub fn machines(&self) -> &[EntityId] {
        &self.machines
    }

    pub fn machine(&self, id: &EntityId) -> Option<&MachineFacts> {
        self.machines.binary_search(id).ok().and(self.machine_facts.get(id))
    }

    pub fn cohort(&self, id: &EntityId) -> Option<&CohortFacts> {
        self.cohorts.iter().find(|cohort| &cohort.id == id)
    }

    /// The hard-assigned cohorts above zero that reach `member`, or an entity it was expanded from, in identity
    /// order.
    pub fn assigned_to(&self, member: &EntityId) -> Vec<&CohortFacts> {
        let mut out: Vec<&CohortFacts> =
            self.cohorts.iter().filter(|cohort| cohort.count > 0 && self.reaches(cohort, member)).collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    pub fn reaches(&self, cohort: &CohortFacts, member: &EntityId) -> bool {
        let Some(reach) = &cohort.reach else { return false };
        let mut at = Some(member.clone());
        while let Some(id) = at {
            if reach.contains(&id) {
                return true;
            }
            at = self.entity(&id).and_then(|entity| entity.parent.clone());
        }
        false
    }
}
