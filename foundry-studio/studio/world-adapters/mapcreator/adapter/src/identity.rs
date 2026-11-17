//! Within the Map Creator adapter, `identity` is **where a converted record's
//! UUID comes from**: the site the host hands over, looked up through the
//! `external_ids` its entities already carry, and a fresh identity the host
//! issues only for a record nothing there matches.
//!
//! This module exists because the alternative is a hash, and a hash is a lie
//! about what an identity is. Daniel ruled it on 2026-09-11: *"minting entities
//! deterministically I think is bad. it sets the wrong precedent, and sets us up
//! for bad habits. These are UUIDs, not hashes. the mapcreator -> world proto
//! converter should first pull the world proto map in question so it can do the
//! external id lookups."* An identity is a fact about the world the site already
//! has. Hashing a program's namespace constant together with a legacy key would
//! let a renamed zone or a re-gridded cell silently become a different entity,
//! and it would let two converters that disagree about a key produce two
//! identities for one thing.
//!
//! So, in event order: the host hands the module the site it already has; this
//! module indexes every `external_ids` field set that names a Map Creator
//! construct; the import asks for one record's **lookup key**; exactly one
//! existing entity carrying that key means the record IS that entity and keeps
//! its id, none means a fresh identity from the host's `ids.fresh`, and more
//! than one is an ambiguity the import refuses by naming the entities. The consequence of a key is stated
//! plainly in `DECISIONS.md`: a record whose key Map Creator edits — a cell moved
//! to a new grid coordinate — resolves to nothing and becomes a new entity.

use crate::written::Usual;
use crate::dump::Error;
use crate::site::Held;
use crate::written;
use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::OnceLock;
use uuid::Uuid;
use worldcanonical::to_canonical_string;

/// The Map Creator constructs that get their own `external_ids` name, and the
/// fields that together name one record of each.
///
/// A construct's own name is its key's leading field, which is what keeps the
/// shapes apart: no shape's field set is a subset of another's, so an entity
/// matching one shape cannot also match a second (`no_shape_contains_another`
/// asserts it). That is why the floor frame's key is `mapcreator_floor` and a
/// cell's is `mapcreator_floor_id` plus `mapcreator_grid_coordinate` — one name
/// for the floor as a thing, another for the floor a cell is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyShape {
    /// The construct, as `DECISIONS.md` names it.
    pub construct: &'static str,
    /// The `external_ids` fields whose values, together, are the lookup key.
    pub fields: &'static [&'static str],
}

/// Every declared lookup key, one row per construct. `DECISIONS.md` carries the
/// same table in prose, with what each key costs when Map Creator edits it.
pub const SHAPES: &[KeyShape] = &[
    // The frames. `mapcreator_frame` names a frame whose role is fixed — the
    // site frame, the nominal legacy grid, the robot vendor's — and the floor
    // frame is keyed by the floor it is, since the export states a floor id.
    KeyShape { construct: "frame", fields: &["mapcreator_frame"] },
    KeyShape { construct: "floor", fields: &["mapcreator_floor"] },
    // A cell, by butler's own cell identity: the floor and the grid coordinate.
    // Not the barcode: a barcode is a sticker on the concrete, replaced when it
    // peels, and SPEC §2 keeps it out of reference position for that reason.
    // `DECISIONS.md` proposes the barcode as the secondary key it would take if
    // re-gridding turns out to be the commoner edit.
    KeyShape { construct: "cell", fields: &["mapcreator_floor_id", "mapcreator_grid_coordinate"] },
    // A station by its own `pps_id`. Its parts — points, bins, seats, lights —
    // name the station they belong to under a DIFFERENT external id,
    // `mapcreator_of_pps`, so that a part's two-field key is not a superset of
    // the station's one-field key: an entity is matched against every shape, and
    // a bin carrying `mapcreator_pps_id` would otherwise answer for its own
    // station as well (`no_shape_contains_another` is the assertion).
    KeyShape { construct: "station", fields: &["mapcreator_pps_id"] },
    // A stitch is not a Map Creator record: it is the subset of one cell's
    // `adjacency` that both ends permit travel across, so the protocol's own
    // reading of the floor. It still needs an identity that survives a
    // re-import, and the pair of grid coordinates is the only thing about it
    // that Map Creator states.
    KeyShape { construct: "stitch", fields: &["mapcreator_stitch"] },
    KeyShape { construct: "presentation_point", fields: &["mapcreator_of_pps", "mapcreator_point_id"] },
    KeyShape { construct: "station_point", fields: &["mapcreator_of_pps", "mapcreator_position_id"] },
    KeyShape { construct: "zone", fields: &["mapcreator_zone_id"] },
    KeyShape { construct: "sector", fields: &["mapcreator_sector"] },
    KeyShape { construct: "aisle", fields: &["mapcreator_aisle_id"] },
    KeyShape { construct: "block", fields: &["mapcreator_block_id"] },
    KeyShape { construct: "location_profile", fields: &["mapcreator_location_profile_id"] },
    // A robot class by its bot type, under the adapter's own name for it:
    // `butler_bot_version` is another system's name for the same string, which
    // the import writes beside this as plain data.
    KeyShape { construct: "robot_class", fields: &["mapcreator_bot_version"] },
    KeyShape { construct: "tote_storage", fields: &["mapcreator_tote_location"] },
    KeyShape { construct: "rack_type", fields: &["mapcreator_racktype_id"] },
    KeyShape { construct: "charger", fields: &["mapcreator_charger_id"] },
    KeyShape { construct: "dock_point", fields: &["mapcreator_dock_point_id"] },
    KeyShape { construct: "ods_exclusion", fields: &["mapcreator_ods_tuple"] },
    KeyShape { construct: "conveyor", fields: &["mapcreator_conveyor_id"] },
    // A mapping row by its conveyor and its step, two values: a key joined into
    // one string would let a conveyor id holding the separator meet another
    // row's key. The conveyor is under `mapcreator_of_conveyor`, not the
    // conveyor's own `mapcreator_conveyor_id`, which would make the conveyor's
    // one-field key a subset of the row's, and every row answer for its conveyor.
    KeyShape { construct: "conveyor_mapping", fields: &["mapcreator_of_conveyor", "mapcreator_step_id"] },
    KeyShape { construct: "sector_preference", fields: &["mapcreator_preferred_for"] },
    KeyShape { construct: "pps_bin", fields: &["mapcreator_of_pps", "mapcreator_bin_id"] },
    KeyShape { construct: "pps_bin_type", fields: &["mapcreator_bin_type_id"] },
    KeyShape { construct: "pps_seat", fields: &["mapcreator_of_pps", "mapcreator_seat_name"] },
    KeyShape { construct: "pptl", fields: &["mapcreator_peripheral_id"] },
    KeyShape { construct: "export", fields: &["mapcreator_export"] },
];

/// The shape one construct's key takes, or `None` for a construct nothing
/// declared — which is a defect in this adapter rather than in a dump, so the
/// caller that asks names the construct it asked for.
pub fn shape(construct: &str) -> Option<&'static KeyShape> {
    SHAPES.iter().find(|shape| shape.construct == construct)
}

/// One record's lookup key: the construct, and the value of each of its shape's
/// fields, in the shape's own order.
///
/// A key is indexed as these values, never as a string joined from them: no
/// separator is one a value cannot hold, so two records whose values joined
/// alike would otherwise be one key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LookupKey {
    pub construct: &'static str,
    pub values: Vec<String>,
}

impl LookupKey {
    /// The key of a construct whose shape is one field.
    pub fn one(construct: &'static str, value: impl Into<String>) -> LookupKey {
        LookupKey { construct, values: vec![value.into()] }
    }

    /// The key of a construct whose shape is two fields.
    pub fn two(construct: &'static str, first: impl Into<String>, second: impl Into<String>) -> LookupKey {
        LookupKey { construct, values: vec![first.into(), second.into()] }
    }

    /// The `external_ids` entries this key is spelled as, which the import
    /// writes onto the entity so the next import finds it again.
    pub fn external_ids(&self) -> Result<Vec<(&'static str, String)>, Error> {
        let shape = shape(self.construct)
            .ok_or_else(|| Error::Document(format!("no lookup key is declared for construct `{}`", self.construct)))?;
        if shape.fields.len() != self.values.len() {
            return Err(Error::Document(format!(
                "construct `{}` has a {}-field lookup key and was asked for with {} values",
                self.construct,
                shape.fields.len(),
                self.values.len()
            )));
        }
        Ok(shape.fields.iter().copied().zip(self.values.iter().cloned()).collect())
    }
}

/// A key as a sentence names it: "cell 1 [7,1]".
impl fmt::Display for LookupKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.construct, self.values.join(" "))
    }
}

/// Where a record's identity came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The site's existing map already had an entity under this key, and the
    /// record IS that entity.
    Existing,
    /// Nothing in the existing map carries this key, so the record is new and
    /// takes a fresh random UUID.
    Fresh,
}

/// Where an identity for a new record comes from: in the module, the host's
/// `ids.fresh`, and in a unit test, a list the test wrote down. Never a function
/// of the record it is for — the source is asked for one and is told nothing.
pub type Fresh<'a> = Box<dyn FnMut() -> Result<Uuid, Error> + 'a>;

/// The host's `ids.fresh`, as a [`Fresh`] source: a version-4 UUID the host
/// draws and remembers, and refuses a new entity under any other.
pub fn from_host<'a>() -> Fresh<'a> {
    Box::new(|| {
        let issued = worldadapt_sdk::fresh_id();
        Uuid::parse_str(&issued)
            .map_err(|_| Error::Document(format!("the host issued {issued:?} as a new identity, and it is not a UUID")))
    })
}

/// The world-protocol map a site already has, indexed by every lookup key its
/// entities carry.
///
/// What this type is chiefly for is one question — which entity is this record —
/// and for that it reads an entity's identity and its `external_ids` and nothing
/// else. It reads three further things, and all three are there so that a
/// re-import changes only what the dump changed:
///
/// - the **record ordering key** each record's `import_formatting` entry carries,
///   so a record the site already has can keep the key it had (`ordering`);
/// - the **fitted transform** each `frame` component carries, so the legacy grid
///   frame is refitted only when the fit would project some cell differently;
/// - the **unit** the site frame this adapter declared states, because a held
///   transform is in that unit's steps and the fit it is compared with is in
///   millimetres.
///
/// None of the three is a fact about what the record means. All three are facts
/// about the site's own bookkeeping that only the previous import can supply, and
/// reading them is what turns a one-cell edit into a one-entity write.
#[derive(Debug, Default)]
pub struct Existing {
    /// Lookup key -> the entities carrying it. A key with two entities is the
    /// ambiguity the import refuses.
    by_key: FxHashMap<LookupKey, Vec<Uuid>>,
    /// Every entity that carries one of this adapter's lookup keys, in the order
    /// the host handed them over, with the constructs its keys name. These are
    /// the entities that are this adapter's: an entity no record of a dump claims
    /// is deleted when it is one of these and never touched when it is not.
    // ordered: the changes an ingest proposes come out in an order that depends
    // on its input alone.
    keyed: Vec<(Uuid, Vec<&'static str>)>,
    /// How many entities the site holds, keyed or not.
    count: usize,
    /// A printable name per identity, for the ambiguity refusal and the ledger.
    names: FxHashMap<String, String>,
    /// The record ordering key each entity's `import_formatting` entry carries.
    /// A record that finds its entity here keeps this key where the dump's order
    /// allows it (`ordering`).
    indexes: FxHashMap<Uuid, i64>,
    /// The `transform_to_parent` each entity's `frame` component carries. The
    /// legacy grid frame's is a least-squares fit over every cell, so it moves
    /// whenever any cell moves; keeping it where it still projects every cell to
    /// the same square is what stops a one-cell edit rewriting the frame every
    /// cell's grid projection goes through.
    transforms: FxHashMap<Uuid, Value>,
    /// What one step of the site frame this adapter declared is worth, when the
    /// site holds that frame and it says.
    unit_mm: Option<f64>,
    /// The orders the export entity says each place of the files usually
    /// gives a set of keys, which an ingest keeps while the set is still
    /// stated there (`written::usual_orders`): `None` for a site that holds no
    /// export entity, or one whose orders another tool spelled otherwise.
    key_orders: Option<Usual>,
}

impl Existing {
    /// No existing map: a first import, where every record is new.
    pub fn none() -> Existing {
        Existing::default()
    }

    /// The site the host handed over: its own scope's entities and those of the
    /// scopes it pins, all of them readable and any of them a record's entity.
    ///
    /// Every entity is a declared one, named by a UUID: the host leaves derived
    /// entities out for a module that does not ask for them, and this one does
    /// not. An identity that does not parse as a UUID is refused rather than
    /// skipped, since the host promised otherwise.
    pub fn of_entities(entities: &[Held<'_>]) -> Result<Existing, Error> {
        let mut existing = Existing { count: entities.len(), ..Existing::default() };
        for entity in entities {
            let stated = entity.id;
            let id = stated.parse::<Uuid>().map_err(|_| {
                Error::Document(format!("the host handed over entity {stated:?}, whose identity is not a UUID"))
            })?;
            if let Some(name) = entity.name {
                existing.names.insert(stated.to_string(), name.to_string());
            }
            let components = entity.components;
            if let Some(transform) = components.get("frame").and_then(|frame| frame.get("transform_to_parent")) {
                existing.transforms.insert(id, transform.clone());
            }
            let Some(ids) = components.get("external_ids").and_then(Value::as_object) else {
                continue;
            };
            let constructs = constructs_of(ids);
            // The ordering key is the one in the record's own entry of its
            // formatting, never an `index` another entry carries: another
            // record kind's, or another importer's, which is another file's
            // order.
            if let Some(index) = constructs
                .first()
                .and_then(|(construct, _)| written::key_of(construct))
                .and_then(|key| components.get(written::COMPONENT)?.get(key)?.get("index")?.as_i64())
            {
                existing.indexes.insert(id, index);
            }
            for (_, key) in &constructs {
                existing.by_key.entry(key.clone()).or_default().push(id);
            }
            // The site frame this adapter declared is the one of its own frames
            // whose role says so.
            let role = components.get("mapcreator.frame").and_then(|frame| frame.get("role")).and_then(Value::as_str);
            if !constructs.is_empty() && role == Some("site") {
                existing.unit_mm =
                    components.get("frame").and_then(|frame| frame.get("unit_mm")).and_then(Value::as_f64);
            }
            if constructs.iter().any(|(construct, _)| *construct == "export") {
                existing.key_orders = components
                    .get("mapcreator.export")
                    .and_then(|export| export.get("key_orders"))
                    .and_then(|orders| serde_json::from_value(orders.clone()).ok());
            }
            if !constructs.is_empty() {
                existing.keyed.push((id, constructs.into_iter().map(|(construct, _)| construct).collect()));
            }
        }
        Ok(existing)
    }

    /// How many entities the site holds, keyed or not.
    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// What one step of the site frame this adapter declared is worth, as the
    /// site states it: `None` for a site that holds no such frame yet.
    pub fn unit_mm(&self) -> Option<f64> {
        self.unit_mm
    }

    /// The orders the site's export entity says each place of the files
    /// usually gives a set of keys, if it says.
    pub fn key_orders(&self) -> Option<&Usual> {
        self.key_orders.as_ref()
    }

    /// Every entity carrying one of this adapter's lookup keys, in the order the
    /// host handed them over, with the constructs its keys name.
    pub fn keyed(&self) -> &[(Uuid, Vec<&'static str>)] {
        &self.keyed
    }

    /// The ordering key one entity's formatting already carries, for a record
    /// that has resolved to it. `None` for an entity whose formatting states
    /// none — a frame, the export header — and for one this map does not hold.
    pub fn record_index(&self, id: Uuid) -> Option<i64> {
        self.indexes.get(&id).copied()
    }

    /// The transform one entity's `frame` component already carries.
    pub fn frame_transform(&self, id: Uuid) -> Option<&Value> {
        self.transforms.get(&id)
    }

    /// Every lookup key more than one of these entities carries, with the
    /// entities, in key order.
    ///
    /// A map with anything here is one this adapter's own import cannot be run
    /// against twice: the second run would refuse. So the laws assert it is empty
    /// over every converted document, which is the structural guard against an
    /// external id being written under a name that is some other construct's own
    /// key — thirty-three conveyor-mapping rows carrying `mapcreator_conveyor_id`
    /// was exactly that, and this is what found it.
    // ordered: the list is read by a person, and an Fx map's order shifts.
    pub fn ambiguities(&self) -> BTreeMap<LookupKey, Vec<Uuid>> {
        self.by_key
            .iter()
            .filter(|(_, entities)| entities.len() > 1)
            .map(|(key, entities)| (key.clone(), entities.clone()))
            .collect()
    }

    /// A printable handle for one existing entity, for a refusal or a ledger row.
    pub fn describe(&self, id: &str) -> String {
        match self.names.get(id) {
            Some(name) => format!("{id} (`{name}`)"),
            None => id.to_string(),
        }
    }
}

/// Every lookup key one entity's `external_ids` spells, with the construct each
/// names, in the order of [`SHAPES`]. An entity carrying no complete key shape
/// is none of this adapter's.
///
/// Every entity of a site is asked this several times a call, so the shapes
/// tried are only those that use a key the entity carries, and nothing is
/// allocated for a shape until the entity carries every field of it.
pub fn constructs_of(ids: &Map<String, Value>) -> Vec<(&'static str, LookupKey)> {
    let using = shapes_using();
    let mut candidates: u32 = ids.keys().filter_map(|key| using.get(key.as_str())).fold(0, |all, shapes| all | shapes);
    let mut found = Vec::new();
    while candidates != 0 {
        let shape = &SHAPES[candidates.trailing_zeros() as usize];
        candidates &= candidates - 1;
        if !shape.fields.iter().all(|field| ids.contains_key(*field)) {
            continue;
        }
        let values = shape.fields.iter().map(|field| scalar(&ids[*field])).collect();
        found.push((shape.construct, LookupKey { construct: shape.construct, values }));
    }
    found
}

/// Every field of a key shape, with the shapes that use it, as bits in the
/// order of [`SHAPES`].
fn shapes_using() -> &'static FxHashMap<&'static str, u32> {
    const _: () = assert!(SHAPES.len() <= 32, "the shapes using a field are one bit each of a u32");
    static USING: OnceLock<FxHashMap<&'static str, u32>> = OnceLock::new();
    USING.get_or_init(|| {
        let mut using: FxHashMap<&'static str, u32> = FxHashMap::default();
        for (at, shape) in SHAPES.iter().enumerate() {
            for field in shape.fields {
                *using.entry(*field).or_default() |= 1 << at;
            }
        }
        using
    })
}

/// The identities one import hands out, and the account of where each came from.
///
/// Resolution is a borrow of `Existing` plus the set of identities claimed so
/// far, because two things need saying at the end of an import that neither the
/// map nor the dump can say alone: how many records found an entity, and which
/// of the site's entities no record claimed.
pub struct Resolver<'a> {
    existing: &'a Existing,
    fresh: Fresh<'a>,
    claimed: FxHashSet<Uuid>,
    /// Key -> the identity this import gave it, so asking twice for one record
    /// gives one answer even when two passes reach the same construct.
    settled: FxHashMap<LookupKey, Uuid>,
    pub looked_up: usize,
    pub minted: usize,
}

impl<'a> Resolver<'a> {
    pub fn new(existing: &'a Existing, fresh: Fresh<'a>) -> Resolver<'a> {
        Resolver {
            existing,
            fresh,
            claimed: FxHashSet::default(),
            settled: FxHashMap::default(),
            looked_up: 0,
            minted: 0,
        }
    }

    /// The identity for one record.
    ///
    /// Exactly one existing entity under the key means the record IS that
    /// entity. None means a fresh identity from the source — the host's random
    /// draw, because the record is new to this site and nothing about a new
    /// thing is derivable from its name. More than one is refused: two entities
    /// claiming one legacy key is a statement about the site nobody can resolve
    /// from a dump, and guessing which one would move an identity silently.
    pub fn resolve(&mut self, key: &LookupKey) -> Result<(Uuid, Origin), Error> {
        if let Some(id) = self.settled.get(key) {
            return Ok((*id, if self.existing.by_key.contains_key(key) { Origin::Existing } else { Origin::Fresh }));
        }
        let (id, origin) = match self.existing.by_key.get(key).map(Vec::as_slice) {
            Some([only]) => {
                self.looked_up += 1;
                (*only, Origin::Existing)
            }
            Some(many) if many.len() > 1 => {
                return Err(Error::Document(format!(
                    "the site's existing map gives {} entities the {} key {} — {}. One legacy key \
                     naming two entities is an ambiguity a dump cannot settle, so this import \
                     refuses rather than move an identity by guess",
                    many.len(),
                    key.construct,
                    key.values.join(" "),
                    many.iter().map(|id| self.existing.describe(&id.to_string())).collect::<Vec<String>>().join(", ")
                )));
            }
            _ => {
                self.minted += 1;
                ((self.fresh)()?, Origin::Fresh)
            }
        };
        self.claimed.insert(id);
        self.settled.insert(key.clone(), id);
        Ok((id, origin))
    }

    /// The identity for one record, without caring where it came from.
    pub fn id(&mut self, key: &LookupKey) -> Result<Uuid, Error> {
        self.resolve(key).map(|(id, _)| id)
    }

    /// This adapter's entities — those carrying one of its lookup keys — that no
    /// record of this dump claimed, in the order the host handed them over.
    ///
    /// An ingest deletes these: the files are the format's whole content for the
    /// site, so an entity of this adapter's that no record names is one the files
    /// no longer hold. An entity carrying none of the keys is never here, whatever
    /// it is, because it is not this adapter's to delete.
    pub fn unclaimed(&self) -> Vec<Uuid> {
        self.existing.keyed.iter().map(|(id, _)| *id).filter(|id| !self.claimed.contains(id)).collect()
    }

    /// The ordering key the entity one record resolved to already carries.
    pub fn prior_index(&self, id: Uuid) -> Option<i64> {
        self.existing.record_index(id)
    }

    /// The transform the frame entity one frame resolved to already carries.
    pub fn prior_transform(&self, id: Uuid) -> Option<&Value> {
        self.existing.frame_transform(id)
    }

    /// What one step was worth in the site frame that held those transforms.
    pub fn prior_unit_mm(&self) -> Option<f64> {
        self.existing.unit_mm()
    }
}

/// A JSON scalar as the one string a key is indexed under: a string's own
/// text, and a number in the canonical spelling (`worldcanonical`). So an id
/// that is an integer in one dialect and a string in another indexes the same
/// way, and a whole number indexes as its integer however the file spelled
/// it: `31.0` and `31` are one key, as the laws' normalization reads them.
pub fn scalar(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(_) => to_canonical_string(value),
        other => other.to_string(),
    }
}

#[cfg(test)]
pub mod tests {
    use crate::dump::Error;
    use std::path::PathBuf;
    use super::{constructs_of, scalar, Existing, Fresh, LookupKey, Origin, Resolver, SHAPES};
    use crate::site::Held;
    use serde_json::{json, Map, Value};
    use std::collections::{BTreeMap, BTreeSet};
    use uuid::Uuid;

    /// Identities a test writes down, handed out in order: the stand-in for the
    /// host's `ids.fresh`, which a unit test compiled for the build machine has
    /// no host to ask. The list is the test's own, and nothing about a record
    /// decides which one it gets.
    pub fn issued(ids: &[&str]) -> Fresh<'static> {
        let mut ids: Vec<Uuid> = ids.iter().rev().map(|id| Uuid::parse_str(id).expect("a UUID")).collect();
        Box::new(move || ids.pop().ok_or_else(|| Error::Document("the test's list ran out".into())))
    }

    /// The site a test states as entity records, owned, so a [`Held`] view can
    /// borrow it.
    pub struct Stated(pub Vec<(String, Option<String>, BTreeMap<String, Value>)>);

    impl Stated {
        pub fn of(entities: Value) -> Stated {
            let entities = entities.as_array().expect("an array of entities");
            Stated(
                entities
                    .iter()
                    .map(|entity| {
                        let id = entity["id"].as_str().expect("an id").to_string();
                        let name = entity.get("name").and_then(Value::as_str).map(str::to_string);
                        let components = entity["components"]
                            .as_object()
                            .expect("components")
                            .iter()
                            .map(|(name, value)| (name.clone(), value.clone()))
                            .collect();
                        (id, name, components)
                    })
                    .collect(),
            )
        }

        pub fn held(&self) -> Vec<Held<'_>> {
            self.0
                .iter()
                .map(|(id, name, components)| Held { id, name: name.as_deref(), scope: "site", own: true, components })
                .collect()
        }
    }

    const FIRST: &str = "a0000000-0000-4000-8000-000000000001";
    const SECOND: &str = "a0000000-0000-4000-8000-000000000002";

    /// The shapes have to be mutually exclusive, because an entity is matched
    /// against every one of them. If one shape's fields were a subset of
    /// another's, a cell carrying both would answer for the floor frame too.
    #[test]
    fn no_shape_contains_another() {
        for outer in SHAPES {
            let a: BTreeSet<&str> = outer.fields.iter().copied().collect();
            for inner in SHAPES {
                if std::ptr::eq(outer, inner) {
                    continue;
                }
                let b: BTreeSet<&str> = inner.fields.iter().copied().collect();
                assert!(
                    !a.is_subset(&b),
                    "`{}`'s key fields are a subset of `{}`'s, so one entity would answer for both",
                    outer.construct,
                    inner.construct
                );
                assert_ne!(outer.construct, inner.construct, "two shapes for one construct");
            }
        }
    }

    /// The keys an entity spells are those of every shape whose fields it
    /// carries, in the order of the shapes, whatever other keys it carries
    /// beside them: checked for every shape alone, every two together, and
    /// every shape one field short.
    #[test]
    fn an_entity_spells_the_key_of_every_shape_whose_fields_it_carries() {
        let plainly = |ids: &Map<String, Value>| -> Vec<(&'static str, LookupKey)> {
            SHAPES
                .iter()
                .filter(|shape| shape.fields.iter().all(|field| ids.contains_key(*field)))
                .map(|shape| {
                    let values = shape.fields.iter().map(|field| scalar(&ids[*field])).collect();
                    (shape.construct, LookupKey { construct: shape.construct, values })
                })
                .collect()
        };
        let carrying = |fields: &[&str]| -> Map<String, Value> {
            let mut ids: Map<String, Value> =
                fields.iter().enumerate().map(|(at, field)| (field.to_string(), json!(at))).collect();
            ids.insert("barcode".into(), json!("042.913"));
            ids.insert("wms".into(), json!("W-9"));
            ids
        };
        let mut checked = 0;
        for one in SHAPES {
            for other in SHAPES {
                let fields: Vec<&str> = one.fields.iter().chain(other.fields).copied().collect();
                let ids = carrying(&fields);
                assert_eq!(constructs_of(&ids), plainly(&ids), "{} and {}", one.construct, other.construct);
                checked += 1;
            }
            for missing in one.fields {
                let fields: Vec<&str> = one.fields.iter().filter(|field| *field != missing).copied().collect();
                let ids = carrying(&fields);
                assert_eq!(constructs_of(&ids), plainly(&ids), "{} without {missing}", one.construct);
            }
        }
        assert_eq!(checked, SHAPES.len() * SHAPES.len());
    }

    #[test]
    fn every_shape_states_at_least_one_field() {
        for shape in SHAPES {
            assert!(!shape.fields.is_empty(), "`{}` has no key fields", shape.construct);
            for field in shape.fields {
                assert!(field.starts_with("mapcreator_"), "{field}");
            }
        }
    }

    fn one_cell_map() -> Stated {
        Stated::of(json!([
            { "id": "11111111-1111-4111-8111-111111111111", "name": "cell [7,1]",
              "components": { "external_ids": {
                  "mapcreator_floor_id": "1", "mapcreator_grid_coordinate": "[7,1]" } } },
            { "id": "22222222-2222-4222-8222-222222222222",
              "components": { "external_ids": { "mapcreator_zone_id": "Dock_West" } } },
        ]))
    }

    /// The whole of the identity rule in one test: a record the site already has
    /// keeps its id, a record it does not have gets the next one the source
    /// issues, and a second ask for the same record gives the same answer.
    #[test]
    fn a_known_record_keeps_its_id_and_a_new_one_takes_an_issued_one() {
        let site = one_cell_map();
        let existing = Existing::of_entities(&site.held()).expect("a map with entities");
        let mut resolver = Resolver::new(&existing, issued(&[FIRST]));
        let known = LookupKey::two("cell", "1", "[7,1]");
        let (id, origin) = resolver.resolve(&known).expect("one entity carries the key");
        assert_eq!(id.to_string(), "11111111-1111-4111-8111-111111111111");
        assert_eq!(origin, Origin::Existing);
        assert_eq!(resolver.resolve(&known).expect("asked twice").0, id);

        let fresh = LookupKey::two("cell", "1", "[8,1]");
        let (new_id, origin) = resolver.resolve(&fresh).expect("nothing carries the key");
        assert_eq!(origin, Origin::Fresh);
        assert_eq!(new_id.to_string(), FIRST, "a new record takes the identity the source issued");
        assert_eq!(resolver.resolve(&fresh).expect("asked twice").0, new_id, "and only one");
        assert_eq!(resolver.looked_up, 1);
        assert_eq!(resolver.minted, 1);
        // The zone nothing asked about is this adapter's, and no record claimed it.
        assert_eq!(
            resolver.unclaimed().iter().map(Uuid::to_string).collect::<Vec<_>>(),
            ["22222222-2222-4222-8222-222222222222"]
        );
    }

    /// A new record's identity is whatever the source issues, and nothing about
    /// the record: one key asked of two sources gets each source's own.
    #[test]
    fn a_new_identity_is_the_sources_and_not_the_records() {
        let nothing = Existing::none();
        let key = LookupKey::one("zone", "Dock_West");
        let first = Resolver::new(&nothing, issued(&[FIRST])).id(&key).expect("an issued id");
        let second = Resolver::new(&nothing, issued(&[SECOND])).id(&key).expect("an issued id");
        assert_eq!((first.to_string().as_str(), second.to_string().as_str()), (FIRST, SECOND));
    }

    #[test]
    fn two_entities_under_one_key_are_refused_by_name() {
        let site = Stated::of(json!([
            { "id": "11111111-1111-4111-8111-111111111111", "name": "zone A",
              "components": { "external_ids": { "mapcreator_zone_id": "Dock_West" } } },
            { "id": "33333333-3333-4333-8333-333333333333", "name": "zone A again",
              "components": { "external_ids": { "mapcreator_zone_id": "Dock_West" } } },
        ]));
        let existing = Existing::of_entities(&site.held()).expect("a map with entities");
        let refusal = Resolver::new(&existing, issued(&[]))
            .resolve(&LookupKey::one("zone", "Dock_West"))
            .err()
            .expect("two entities under one key");
        let message = refusal.to_string();
        assert!(message.contains("11111111"), "{message}");
        assert!(message.contains("33333333"), "{message}");
        assert!(message.contains("zone A again"), "{message}");
    }

    /// An entity that carries none of this adapter's keys matches nothing and is
    /// never in the unclaimed list: it is not this adapter's to delete.
    #[test]
    fn an_entity_without_a_key_is_neither_matched_nor_unclaimed() {
        let site = Stated::of(json!([
            { "id": "44444444-4444-4444-8444-444444444444",
              "components": { "cell": { "size": [900, 900] } } },
        ]));
        let existing = Existing::of_entities(&site.held()).expect("a map with entities");
        assert_eq!(existing.len(), 1);
        let mut resolver = Resolver::new(&existing, issued(&[FIRST]));
        assert_eq!(resolver.resolve(&LookupKey::two("cell", "1", "[7,1]")).expect("fresh").1, Origin::Fresh);
        assert!(resolver.unclaimed().is_empty());
    }

    /// A key is the same key whichever JSON kind the dialect spells it in: v3
    /// states `pps_id: 123` and v2 states `pps_id: "1"`. A number is the same
    /// key however it is spelled: a whole number is its integer, and any other
    /// number takes the canonical spelling.
    #[test]
    fn an_id_that_is_an_integer_in_one_dialect_indexes_as_the_string() {
        assert_eq!(scalar(&json!(123)), "123");
        assert_eq!(scalar(&json!("123")), "123");
        assert_eq!(scalar(&json!(123.0)), "123");
        assert_eq!(scalar(&json!(-0.0)), "0");
        assert_eq!(scalar(&json!(12.5)), "12.5");
        assert_eq!(scalar(&serde_json::from_str::<Value>("1.25e1").expect("a number")), "12.5");
        let site = Stated::of(json!([
            { "id": "55555555-5555-4555-8555-555555555555",
              "components": { "external_ids": { "mapcreator_pps_id": 123 } } },
        ]));
        let existing = Existing::of_entities(&site.held()).expect("a map with entities");
        let id = Resolver::new(&existing, issued(&[])).id(&LookupKey::one("station", "123")).expect("resolved");
        assert_eq!(id.to_string(), "55555555-5555-4555-8555-555555555555");
    }

    /// A key is its values, not a string joined from them: two seat keys whose
    /// values would join into one string are two keys.
    #[test]
    fn two_keys_whose_values_join_alike_are_two_keys() {
        let site = Stated::of(json!([
            { "id": "77777777-7777-4777-8777-777777777777",
              "components": { "external_ids": { "mapcreator_of_pps": "A\u{1}B", "mapcreator_seat_name": "C" } } },
        ]));
        let existing = Existing::of_entities(&site.held()).expect("a map with entities");
        let mut resolver = Resolver::new(&existing, issued(&[FIRST]));
        let (_, origin) = resolver.resolve(&LookupKey::two("pps_seat", "A", "B\u{1}C")).expect("resolved");
        assert_eq!(origin, Origin::Fresh, "another seat's key found this one's entity");
        let (_, origin) = resolver.resolve(&LookupKey::two("pps_seat", "A\u{1}B", "C")).expect("resolved");
        assert_eq!(origin, Origin::Existing);
    }

    /// The site frame this adapter declared says what a step is worth, which is
    /// what a held transform is read in.
    #[test]
    fn the_site_frame_states_the_unit() {
        let site = Stated::of(json!([
            { "id": "66666666-6666-4666-8666-666666666666",
              "components": { "frame": { "dims": 3, "units": "mm", "unit_mm": 0.1 },
                              "mapcreator.frame": { "role": "site" },
                              "external_ids": { "mapcreator_frame": "site" } } },
        ]));
        assert_eq!(Existing::of_entities(&site.held()).expect("a map").unit_mm(), Some(0.1));
        assert_eq!(Existing::none().unit_mm(), None);
    }

    /// The identity rule as a test rather than a reading: no source of this
    /// module calls a UUID constructor that draws an identity or derives one
    /// from content. A dependency can link `uuid`'s name-based constructor in,
    /// so the compiler alone would not notice a call. The patterns are spelled
    /// in pieces, so the test does not find itself.
    #[test]
    fn no_source_of_the_module_makes_an_identity() {
        let forbidden: Vec<String> = ["v3", "v4", "v5", "v8"]
            .iter()
            .map(|version| format!("new_{version}"))
            .chain([format!("Uuid::{}", "from_")])
            .collect();
        let mut directories = vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")];
        let (mut read, mut found) = (0usize, Vec::new());
        while let Some(directory) = directories.pop() {
            for entry in std::fs::read_dir(&directory).expect("a source directory") {
                let path = entry.expect("an entry").path();
                if path.is_dir() {
                    directories.push(path);
                    continue;
                }
                if path.extension().is_none_or(|extension| extension != "rs") {
                    continue;
                }
                read += 1;
                let text = std::fs::read_to_string(&path).expect("a source file");
                for (at, line) in text.lines().enumerate() {
                    for pattern in forbidden.iter().filter(|pattern| line.contains(pattern.as_str())) {
                        found.push(format!("{}:{}: {pattern}", path.display(), at + 1));
                    }
                }
            }
        }
        assert!(read > 10, "only {read} source files were read, so this proves little");
        assert!(found.is_empty(), "a source makes an identity rather than asking the host for one: {found:?}");
    }

    #[test]
    fn a_key_spells_itself_as_the_external_ids_the_entity_carries() {
        let entries = LookupKey::two("cell", "1", "[7,1]").external_ids().expect("a two-field shape");
        assert_eq!(
            entries,
            vec![("mapcreator_floor_id", "1".to_string()), ("mapcreator_grid_coordinate", "[7,1]".to_string()),]
        );
        assert!(LookupKey::one("cell", "[7,1]").external_ids().is_err(), "a two-field shape needs two values");
    }
}
