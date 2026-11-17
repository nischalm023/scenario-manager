//! Within the Map Creator adapter, `import` is **the dump becoming entities**:
//! one Map Creator export in, every entity the site should hold for it out,
//! with every field family landing somewhere and every identity looked up
//! rather than made up. Turning those entities into changes to the site is
//! `ingest`'s job, which compares them with what the site holds.
//!
//! The mapping is the one-way importer's, lifted here and extended. That mapping
//! follows from one fact about the source: a Map Creator export is a *survey*,
//! not a design. Every cell carries its own measured millimetre position and its
//! own four half-extents, the pitch varies cell to cell, and the grid indices are
//! sparse — so there is nothing for a region and a tiling to generate. Every cell
//! is declared, the export's `adjacency` becomes explicit stitches, and the legacy
//! grid is a declared, lossy, projection-only frame beside the real one.
//!
//! What this module adds to that mapping is the second home. Where the one-way
//! importer had 32 field families with nowhere to go and logged them, here each
//! one lands in a component of the vocabulary and comes back out. The rule that keeps
//! the two homes from arguing is `records`: one fact, one place, and where the
//! core carries a *derived* form — `cell.size` summed from four half-extents — the
//! vocabulary's component keeps the original and the export writes from the
//! original.
//!
//! Identity comes from `identity`, never from a hash. Read that module's doc
//! before changing anything here: the order of the passes below is the order the
//! references need, because a record can only be pointed at once it has an id.

use crate::written::Spacing as WrittenSpacing;
use crate::dump::{Dump, Error};
use crate::identity::{scalar, Existing, Fresh, LookupKey, Resolver};
use crate::kinds::FileKind;
use crate::layout;
use crate::ledger::Ledger;
use crate::format::coverage::Reading;
use crate::format::params::Params;
use crate::format::source::{Cell, LocationProfile, Site, MAX_WHOLE};
use crate::format::steps::{self, Length, Unkept};
use crate::ordering;
use crate::records;
use crate::records::{field, Field, Join, KeyText, RecordKind, Slot, Spelling};
use crate::redundancy;
use crate::source_index::{key_of, CellNamed, SourceIndex};
use crate::written::{self, Facts, Usual};
use rustc_hash::FxHashMap;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// One dump's import: every entity the site should hold for it, each an
/// `{id, name, components}` record in the steps of the site frame, and the
/// account of it.
pub struct Import {
    pub entities: Vec<Value>,
    pub ledger: Ledger,
    /// This adapter's entities of the site that no record claimed, in the order
    /// the host handed them over.
    pub unclaimed: Vec<Uuid>,
}

/// A straight-line fit of one axis, `value = intercept + slope * index`.
#[derive(Debug, Clone, Copy)]
struct Fit {
    intercept: f64,
    slope: f64,
}

/// One row of a flat record file, with the identity the first pass settled for
/// it, held until the second pass encodes it.
struct FlatRecord<'d> {
    /// The row's group in its file, which only `pptl.json` has.
    group: usize,
    /// Where the row stands in its file, as a pointer.
    pointer: String,
    /// Where the row stands in its file, for a sentence that names it.
    place: String,
    record: &'d Map<String, Value>,
    key: LookupKey,
    id: Uuid,
}

/// The one value the export entity's key takes: a site holds one Map Creator
/// export.
pub const EXPORT_KEY: &str = "site";

/// One record, encoded by its field map: what it states about the site, which
/// its component carries, and how its file wrote it, which its entry of
/// `import_formatting` carries.
struct Encoded {
    fields: Map<String, Value>,
    formatting: Map<String, Value>,
}

/// Everything one import pass needs to hand to the next.
struct Pass<'a> {
    site: &'a Site<'a>,
    dump: &'a Dump,
    resolver: Resolver<'a>,
    params: Params,
    index: SourceIndex,
    entities: Vec<Value>,
    ledger: Ledger,
    floor_frame: Uuid,
    site_frame: Uuid,
    /// Every legacy key one record used that named nothing in this dump.
    dangling: Vec<String>,
    /// The order each place of the files usually states an object's keys in,
    /// which a record's own order is recorded against.
    usual: Usual,
}

/// Import one dump against the site as it stands.
///
/// `existing` is the whole of the identity story: a record whose lookup key one
/// of its entities carries IS that entity and keeps its id, and a record nothing
/// matches is new and takes the identity `fresh` issues — the host's, in the
/// module. Pass [`Existing::none`] for a site that holds nothing of the export.
pub fn import(dump: &Dump, existing: &Existing, fresh: Fresh<'_>) -> Result<Import, Error> {
    layout::check(dump)?;
    let site = Site::from_files(dump.to_files()).map_err(Error::Import)?;
    let mut pass = Pass {
        site: &site,
        dump,
        resolver: Resolver::new(existing, fresh),
        params: Params::default(),
        index: SourceIndex::default(),
        entities: Vec::new(),
        ledger: Ledger::new(),
        floor_frame: Uuid::nil(),
        site_frame: Uuid::nil(),
        dangling: Vec::new(),
        usual: written::usual_orders(dump.shapes(), existing.key_orders()),
    };

    // The order is the order the references need. Nothing may be pointed at
    // before it has an id, and every pass below points only backwards.
    pass.declare_frames()?;
    pass.declare_robot_classes()?;
    pass.declare_location_profiles()?;
    pass.declare_zones()?;
    pass.declare_cells()?;
    redundancy::check(&site, dump, &pass.index)?;
    pass.declare_sectors()?;
    pass.declare_stitches()?;
    pass.declare_aisles()?;
    pass.declare_blocks()?;
    pass.declare_stations()?;
    pass.declare_flat_records()?;
    pass.declare_sector_preferences()?;
    pass.declare_export_entity()?;

    pass.account_for_identity(existing);
    pass.account_for_coverage();
    let unclaimed = pass.resolver.unclaimed();
    pass.ledger.invented_parameters = pass.params.used();
    pass.ledger.settle();

    // Every length above is in millimetres, because that is what the export
    // measures; the protocol counts steps of the primary frame (SPEC §5). One
    // pass at the end picks this site's unit, or keeps the one the site holds,
    // and rewrites the entities in it, and the site frame says what that unit is.
    let mut entities = Value::Array(pass.entities);
    let unit_mm = steps::to_steps(&mut entities, existing.unit_mm())
        .map_err(|unkept| unkept_unit(&entities, pass.site_frame, unkept))?;
    let Value::Array(mut entities) = entities else { unreachable!("the pass rewrites an array in place") };
    if unit_mm != 1.0 {
        let site_frame = pass.site_frame.to_string();
        for entity in &mut entities {
            if entity.get("id").and_then(Value::as_str) == Some(site_frame.as_str()) {
                entity["components"]["frame"]["unit_mm"] = json!(unit_mm);
            }
        }
        pass.ledger.notes.push(format!(
            "the primary frame declares unit_mm {unit_mm}: this site measures finer than a millimetre, so every \
             length counts {unit_mm} mm steps, and the export divides back"
        ));
    }

    Ok(Import { entities, ledger: pass.ledger, unclaimed })
}

impl Pass<'_> {
    /// The ordering keys one file's records take, given their identities in the
    /// order the file states them.
    ///
    /// Every pass that writes records calls this between settling the identities
    /// and encoding, because the key a record takes depends on the key the entity
    /// it resolved to already had: `ordering`'s whole job is to leave that key
    /// alone wherever the dump's order allows, so that a re-import writes the
    /// records the dump changed and no others.
    fn ordering(&self, ids: &[Uuid]) -> Vec<i64> {
        let prior: Vec<Option<i64>> = ids.iter().map(|id| self.resolver.prior_index(*id)).collect();
        ordering::stable_indexes(&prior)
    }

    fn push(&mut self, id: Uuid, name: Option<String>, components: Value) {
        let mut record = Map::new();
        record.insert("id".into(), json!(id));
        if let Some(name) = name {
            record.insert("name".into(), json!(name));
        }
        record.insert("components".into(), components);
        self.entities.push(Value::Object(record));
    }

    // -----------------------------------------------------------------------
    // Frames
    // -----------------------------------------------------------------------

    /// The four frames: the site's, the floor's, the legacy grid as a declared
    /// lossy projection, and the robot vendor's.
    fn declare_frames(&mut self) -> Result<(), Error> {
        let floor_id = self.site.floor_id;
        self.site_frame = self.resolver.id(&LookupKey::one("frame", "site"))?;
        self.floor_frame = self.resolver.id(&LookupKey::one("floor", floor_id.to_string()))?;
        let site_frame = self.site_frame;
        let floor_frame = self.floor_frame;

        self.push(
            site_frame,
            Some("site".to_string()),
            json!({
                "frame": {
                    "dims": 3, "units": "mm", "unit_mm": 1,
                    "$comment": "The primary frame: world-fixed, 3-D, and the one frame that says what a step is worth. Every position in this document resolves here, as a whole number of those steps."
                },
                "mapcreator.frame": { "role": "site" },
                "external_ids": { "mapcreator_frame": "site" }
            }),
        );

        // The one sample point the export offers in place of a transform lives
        // here, as the cell it names rather than as a copy of that cell's vendor
        // coordinate — both halves of `vda5050_reference` are the named cell's own
        // fields, on every site seen.
        let reference = self.site.vda5050_reference.clone();
        let elevation = self.params.floor_elevation_mm;
        let note = self.params.note("floor_elevation_mm", elevation);
        let mut floor = Map::new();
        floor.insert("role".into(), json!("floor"));
        floor.insert("floor_id".into(), json!(floor_id));
        if let Some(reference) = &reference {
            let barcode = reference.get("barcode").and_then(Value::as_str).unwrap_or_default().to_string();
            floor.insert("vda5050_reference_barcode".into(), json!(barcode));
        }
        self.push(
            floor_frame,
            Some(format!("floor {floor_id}")),
            json!({
                "frame": {
                    "parent": site_frame, "dims": 2, "units": "mm",
                    "transform_to_parent": {
                        "translation": [0, 0, elevation], "scale": [1, 1], "$comment": note
                    },
                    "$comment": "The export is per floor: one map.json states one floor_id, and elevator.json — the only file that could relate two floors — is empty on every site seen."
                },
                "mapcreator.frame": Value::Object(floor),
                "external_ids": { "mapcreator_floor": floor_id.to_string() }
            }),
        );

        self.declare_grid_frame(floor_frame)?;
        self.declare_vendor_frame(floor_frame)?;
        Ok(())
    }

    /// The legacy grid frame, fitted, with the measurement of how badly it fits.
    ///
    /// SPEC §4 provides for exactly this: a nominal, lossy grid frame declared for
    /// projection only, with each cell's *recorded* legacy coordinate riding as
    /// data. The measurement is the point — a fit residual of one cell in three is
    /// a number about the installed base, not about the fit.
    ///
    /// The fit is least squares over every on-grid cell, which means it moves
    /// whenever any cell moves — and this frame is the one every cell's grid
    /// projection goes through, so a one-cell edit would otherwise rewrite the
    /// shared frame on every re-import. So a transform the site already has is
    /// kept whenever it projects every cell of the new dump to the same square the
    /// new fit would: two transforms that place every cell alike are the same
    /// statement about the floor, and writing the second one is churn.
    fn declare_grid_frame(&mut self, floor_frame: Uuid) -> Result<(), Error> {
        let on_grid: Vec<&Cell> = self.site.cells.iter().filter(|c| c.is_on_grid()).collect();
        let x = fit(on_grid.iter().map(|c| (c.grid[0] as f64, c.world[0])));
        let y = fit(on_grid.iter().map(|c| (c.grid[1] as f64, c.world[1])));
        let (Some(x), Some(y)) = (x, y) else {
            self.ledger.notes.push(
                "no legacy grid frame is declared: the site's grid indices do not vary on both \
                 axes, so no straight line relates them to millimetres"
                    .to_string(),
            );
            return Ok(());
        };
        let fresh = [
            Fit { intercept: round3(x.intercept), slope: round3(x.slope) },
            Fit { intercept: round3(y.intercept), slope: round3(y.slope) },
        ];
        // The frame is written as rounded, and a reader inverts its scale: a
        // slope that rounds to nothing would declare a frame nothing can be
        // projected into, so none is declared, and the note says why.
        if fresh.iter().any(|axis| axis.slope == 0.0 || !axis.slope.is_finite() || !axis.intercept.is_finite()) {
            self.ledger.notes.push(format!(
                "no legacy grid frame is declared: the fit's slope, {} mm per index across and {} mm per index \
                 along, rounds to {} and {} at the thousandth a frame is written to, and a frame of no scale \
                 projects nothing",
                x.slope, y.slope, fresh[0].slope, fresh[1].slope
            ));
            return Ok(());
        }
        let id = self.resolver.id(&LookupKey::one("frame", "legacy-grid"))?;

        // The transform the site already has, when keeping it changes no cell's
        // projected square. A transform read back off the site is in the steps of
        // the site frame that held it, so it is carried back into millimetres
        // before it is set against the fit above.
        //
        // Compared over **every** cell, not over the cells the fit was taken from.
        // The reader projects every cell of a document into every integer frame and
        // warns about the squares that hold two of them
        // (`pipeline/check.rs`'s `warn_lattice_projection_collisions`), so a
        // transform that agrees with the new fit on the on-grid cells and puts an
        // off-grid one in a different square is a transform under which the reader
        // says something different — and saying the same thing is the entire
        // argument for keeping the old one.
        let every: Vec<&Cell> = self.site.cells.iter().collect();
        let held_unit = self.resolver.prior_unit_mm().unwrap_or(1.0);
        let held = self.resolver.prior_transform(id).and_then(read_fit).map(|fit| in_millimetres(fit, held_unit));
        let (fitted, refitted) = match held {
            Some(held) if projects_alike(&every, &held, &fresh) => (held, false),
            Some(_) => (fresh, true),
            None => (fresh, false),
        };
        let [x, y] = fitted;
        let project = |cell: &Cell| -> [i64; 2] {
            [
                ((cell.world[0] - x.intercept) / x.slope + 0.5).floor() as i64,
                ((cell.world[1] - y.intercept) / y.slope + 0.5).floor() as i64,
            ]
        };
        let misplaced = on_grid.iter().filter(|cell| project(cell) != cell.grid).count();
        if refitted {
            self.ledger.notes.push(format!(
                "the legacy grid frame was refitted: the transform this site's map already carried \
                 projects at least one of its {} cells onto a different square than the new fit does \
                 — the fit itself is taken over the {} on-grid cells, and the comparison is over \
                 every cell, because that is the set the reader projects — so the frame entity is \
                 rewritten",
                every.len(),
                on_grid.len()
            ));
        }
        let note = self.params.note(
            "grid_frame_transform",
            format!(
                "translation [{}, {}], scale [{}, {}], fitted over {} on-grid cells",
                x.intercept,
                y.intercept,
                x.slope,
                y.slope,
                on_grid.len()
            ),
        );
        self.push(
            id,
            Some("legacy grid (Map Creator)".to_string()),
            json!({
                "frame": {
                    "parent": floor_frame, "dims": 2, "units": "index", "integer_only": true,
                    "transform_to_parent": {
                        "translation": [x.intercept, y.intercept], "scale": [x.slope, y.slope],
                        "$comment": note
                    },
                    "$comment": "Declared for projection only. Nothing in this document is positioned in this frame; each cell carries its recorded grid coordinate as external_ids.mapcreator_grid_coordinate instead."
                },
                "mapcreator.frame": { "role": "legacy_grid" },
                "external_ids": { "mapcreator_frame": "legacy-grid" }
            }),
        );
        self.ledger.cannot_state(
            "map.json:[].map_values[].coordinate against world_coordinate",
            Reading::LoadBearing,
            misplaced,
            format!(
                "A per-cell correction to a frame. The grid is not an affine image of the \
                 millimetre world, so the best straight-line fit over this site's {} on-grid cells \
                 still projects {misplaced} of them onto a different grid square than the export \
                 records. Nothing is lost — every cell carries its exact recorded coordinate as \
                 data, and the round trip writes that back — but the nominal frame cannot stand in \
                 for the recorded value, and the protocol has no way to say why.",
                on_grid.len()
            ),
        );
        Ok(())
    }

    /// The robot vendor's frame, recovered by regression over the site's own
    /// cells, and the count of cells it does not reproduce.
    fn declare_vendor_frame(&mut self, floor_frame: Uuid) -> Result<(), Error> {
        let mut offsets: BTreeMap<(i64, i64), usize> = BTreeMap::new();
        for cell in &self.site.cells {
            // Both positions are within `MAX_MM` of the origin, so an offset is
            // a whole number of millimetres an `i64` holds, and its negation too.
            if let Some(vda) = cell.vda {
                let key = (whole_mm(vda[0] - cell.world[0]), whole_mm(vda[1] + cell.world[1]));
                if let (Some(kx), Some(ky)) = key {
                    *offsets.entry((kx, ky)).or_default() += 1;
                }
            }
        }
        let unusable =
            self.site.cells.iter().filter(|cell| cell.vda.is_none() && cell.raw.contains_key("vda_world_coordinate"));
        let unusable = unusable.count();
        if unusable > 0 {
            self.ledger.notes.push(format!(
                "{unusable} cells state a vda_world_coordinate that is not two finite numbers of millimetres in \
                 range: each rides back as the file wrote it, and none is a vote in the vendor frame's fit"
            ));
        }
        let Some((&(kx, ky), &covered)) = offsets.iter().max_by_key(|(_, &count)| count) else {
            if unusable > 0 {
                self.ledger
                    .notes
                    .push("no vendor frame is declared: no cell states a vendor coordinate that measures".to_string());
            }
            return Ok(());
        };
        let stated: usize = offsets.values().sum();
        let note = self
            .params
            .note("vendor_frame_transform", format!("Kx {kx}, Ky {ky}, holding for {covered} of {stated} cells"));
        let id = self.resolver.id(&LookupKey::one("frame", "vendor/vda5050"))?;
        self.push(
            id,
            Some("vendor: VDA5050 frame".to_string()),
            json!({
                "frame": {
                    "parent": floor_frame, "dims": 2, "units": "mm", "vendor": "vda5050",
                    "transform_to_parent": {
                        "translation": [-kx, ky], "scale": [1, -1], "$comment": note
                    },
                    "$comment": "vda_x = world_x + Kx and vda_y = Ky - world_y, so the frame translates and flips y — the one detail a hand port gets wrong. Butler recomputes this per floor from a single reference cell; here it is declared once."
                },
                "mapcreator.frame": { "role": "vendor_vda5050" },
                "external_ids": { "mapcreator_frame": "vendor/vda5050" }
            }),
        );
        self.ledger.cannot_state(
            "map.json:[].map_values[].vda_world_coordinate against world_coordinate",
            Reading::EmittedOnly,
            stated - covered,
            format!(
                "A measured vendor position that the declared transform does not put there. This \
                 site's cells state {} different (Kx, Ky) pairs, so {} of {stated} recorded vendor \
                 coordinates differ from what one declared frame projects — the positions were \
                 accumulated cell by cell along different paths and drifted. Each cell's recorded \
                 value rides in external_ids and the round trip writes it back, so nothing is lost; \
                 what the protocol cannot say is which of the two a consumer should believe.",
                offsets.len(),
                stated - covered
            ),
        );
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Robot classes and location profiles
    // -----------------------------------------------------------------------

    /// One robot class per bot type the site's location profiles name.
    ///
    /// The resolver's refusals travel like every other pass's: two existing
    /// entities carrying one `mapcreator_bot_version` is an ambiguity a dump cannot
    /// settle, and swallowing it here would have skipped the class and reported
    /// success — a document with cells whose `traversable_by` names a class the
    /// document does not declare.
    fn declare_robot_classes(&mut self) -> Result<(), Error> {
        let mut types: BTreeSet<String> = BTreeSet::new();
        for profile in &self.site.location_profiles {
            types.extend(profile.bots.keys().cloned());
        }
        let roles: Vec<String> = types.iter().map(|bot| format!("{bot} -> {}", role_of(bot))).collect();
        let makers: Vec<String> =
            types.iter().filter_map(|bot| manufacturer_of(bot).map(|maker| format!("{bot} -> {maker}"))).collect();
        if !roles.is_empty() {
            self.params.note("robot_class_role", roles.join(", "));
        }
        if !makers.is_empty() {
            self.params.note("robot_class_manufacturer", makers.join(", "));
        }
        for bot in &types {
            let id = self.resolver.id(&LookupKey::one("robot_class", bot.clone()))?;
            let mut components = Map::new();
            components.insert(
                "robot_class".into(),
                json!({
                    "role": role_of(bot),
                    "model": model_of(bot),
                    "$comment": crate::format::params::provenance("robot_class_role")
                }),
            );
            if let Some(manufacturer) = manufacturer_of(bot) {
                components.insert(
                    "lineage".into(),
                    json!([{
                        "party": manufacturer, "role": "manufacturer", "designation": model_of(bot),
                        "$comment": crate::format::params::provenance("robot_class_manufacturer")
                    }]),
                );
            }
            components
                .insert("external_ids".into(), json!({ "mapcreator_bot_version": bot, "butler_bot_version": bot }));
            self.push(id, Some(bot.clone()), Value::Object(components));
        }
        let count = types.len();
        self.ledger.carry(
            "map.json location_profiles[].bot_orientation_details keys",
            "one robot_class entity per bot type",
            count,
        );
        Ok(())
    }

    /// One entity per location profile. The profile's own table — which bot types
    /// are permitted, at which headings, with which lift heights and ground
    /// clearance — is the richest thing in the export, and the core's
    /// `cell.traversable_by` is a flat list of classes, so the table rides whole
    /// and the flat list is read off it.
    fn declare_location_profiles(&mut self) -> Result<(), Error> {
        let profiles = self.site.location_profiles.clone();
        let mut ids: Vec<Uuid> = Vec::with_capacity(profiles.len());
        for profile in &profiles {
            let id = self.resolver.id(&LookupKey::one("location_profile", profile.id.clone()))?;
            self.index.add_profile(&profile.id, id);
            ids.push(id);
        }
        let order = self.ordering(&ids);
        // Each profile's own record, the first stating its id, found once
        // rather than by a scan of the list per profile.
        let site = self.site;
        let floor = site.files.get("map.json").and_then(|map| map.get(0));
        let listed = floor.and_then(|floor| floor.get("location_profiles")).and_then(Value::as_array);
        let mut raw_profiles: FxHashMap<&str, &Map<String, Value>> = FxHashMap::default();
        for raw in listed.into_iter().flatten().filter_map(Value::as_object) {
            if let Some(id) = raw.get("location_profile_id").and_then(Value::as_str) {
                raw_profiles.entry(id).or_insert(raw);
            }
        }
        for (at, profile) in profiles.iter().enumerate() {
            let raw = raw_profiles.get(profile.id.as_str()).map(|raw| (*raw).clone()).unwrap_or_default();
            let id = ids[at];
            let place = format!("map.json location profile {}", at + 1);
            let Encoded { fields: mut component, formatting } = self.encode(
                records::LOCATION_PROFILE,
                &raw,
                (FileKind::Map, &format!("/0/location_profiles/{at}"), "map.json[].location_profiles[]"),
                order[at],
                &place,
            )?;
            component.insert(
                "$comment".into(),
                json!(
                    "The permission table Map Creator states per bot version and per heading, kept whole. \
                 `cell.traversable_by` on every cell that names this profile is the flat list of \
                 roles read off it, which is a derived form: two models of one role can differ here \
                 and cannot differ there."
                ),
            );
            self.push(
                id,
                Some(format!("location profile {}", profile.id)),
                json!({
                    "location_profile": Value::Object(component),
                    "import_formatting": written::component(lens_of("location_profile"), formatting),
                    "external_ids": { "mapcreator_location_profile_id": profile.id.clone() }
                }),
            );
        }
        let count = profiles.len();
        self.ledger.carry("map.json location_profiles[]", "one location_profile entity each", count);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Zones
    // -----------------------------------------------------------------------

    /// One entity per zone: the e-stop and light-curtain domain every cell
    /// belongs to exactly one of.
    ///
    /// The one-way importer had nowhere to put this and carried the zone string
    /// per cell instead; `zone` is the entity, and the cell references
    /// it. Note what the string was: on one site the largest zone is
    /// named with a typo and a trailing space, and it was the join key for 925
    /// cells.
    fn declare_zones(&mut self) -> Result<(), Error> {
        let rows = self.dump.get(FileKind::Zone).and_then(Value::as_array).cloned().unwrap_or_default();
        let records: Vec<(String, Map<String, Value>)> = rows
            .iter()
            .map(|row| {
                let record = row.get("zonerec").and_then(Value::as_object).cloned().unwrap_or_default();
                let zone_id = record.get("zone_id").map(scalar).unwrap_or_default();
                (zone_id, record)
            })
            .collect();
        let mut ids: Vec<Uuid> = Vec::with_capacity(records.len());
        for (zone_id, _) in &records {
            let id = self.resolver.id(&LookupKey::one("zone", zone_id.clone()))?;
            self.index.add_zone(zone_id, id);
            ids.push(id);
        }
        let order = self.ordering(&ids);
        for (at, (zone_id, record)) in records.iter().enumerate() {
            let zone_id = zone_id.clone();
            let id = ids[at];
            let place = format!("zone.json record {}", at + 1);
            let stands = (FileKind::Zone, format!("/{at}"));
            let Encoded { fields, formatting } =
                self.encode(records::ZONE, record, (stands.0, &stands.1, "zone.json[]"), order[at], &place)?;
            self.push(
                id,
                Some(format!("zone {zone_id}")),
                json!({
                    "zone": Value::Object(fields),
                    "import_formatting": written::component(lens_of("zone"), formatting),
                    "external_ids": { "mapcreator_zone_id": zone_id }
                }),
            );
        }
        let count = rows.len();
        self.ledger.carry("zone.json []", "one zone entity each", count);
        self.ledger.carry_runtime("zone.json zonerec.blocked and .paused", "zone.runtime", count);
        self.ledger.cannot_state(
            "zone.json:[].zonerec.zone_id as a safety domain",
            Reading::LoadBearing,
            count,
            "What a zone IS. Butler pauses and blocks whole zones on physical safety events and \
             computes a buffer of cells within 1700 mm of one for pre-emptive stopping. IDIOMS \
             reserves the name `safety_domain` for the core component that would carry that \
             meaning; until it exists the zone is an entity with a Map Creator component on it, \
             which round-trips but tells a core consumer nothing.",
        );
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Cells
    // -----------------------------------------------------------------------

    /// One declared entity per cell of `map_values`.
    fn declare_cells(&mut self) -> Result<(), Error> {
        let profiles: BTreeMap<String, LocationProfile> =
            self.site.location_profiles.iter().map(|p| (p.id.clone(), p.clone())).collect();
        let floor_id = self.site.floor_id.to_string();
        let floor_frame = self.floor_frame;
        let site = self.site;
        let cells = &site.cells;

        let mut without_profile = 0usize;
        let mut collapsed = 0usize;
        let mut degenerate = 0usize;

        let mut ids: Vec<Uuid> = Vec::with_capacity(cells.len());
        for cell in cells {
            let id = self.resolver.id(&LookupKey::two("cell", floor_id.clone(), cell.coordinate.clone()))?;
            self.index.add_cell(&cell.coordinate, &cell.barcode, id, cell.world);
            ids.push(id);
        }
        let order = self.ordering(&ids);

        for (at, cell) in cells.iter().enumerate() {
            let id = ids[at];

            let mut roles: BTreeSet<String> = BTreeSet::new();
            match cell.location_profile_id.as_deref().and_then(|p| profiles.get(p)) {
                Some(profile) => {
                    for (bot, stated) in &profile.bots {
                        if *stated {
                            roles.insert(role_of(bot));
                        }
                    }
                    let mut per_role: BTreeMap<String, (bool, bool)> = BTreeMap::new();
                    for (bot, stated) in &profile.bots {
                        let seen = per_role.entry(role_of(bot)).or_insert((false, false));
                        if *stated {
                            seen.0 = true;
                        } else {
                            seen.1 = true;
                        }
                    }
                    if per_role.values().any(|(yes, no)| *yes && *no) {
                        collapsed += 1;
                    }
                }
                None => without_profile += 1,
            }

            let mut core = Map::new();
            core.insert("traversable_by".into(), json!(roles));
            core.insert("blocked".into(), json!(cell.blocked));
            // A cell stating no `size_info` states no footprint, and one whose
            // `size_info` sums to nothing on an axis has none to state.
            if let (Some(size), Some(parts)) = (cell.size_mm(), cell.size_parts()) {
                if size[0] > 0.0 && size[1] > 0.0 {
                    // Each axis as the two half-extents it sums, which the units
                    // pass holds to its rule one by one and counts as one sum.
                    core.insert("size".into(), json!(parts));
                } else {
                    degenerate += 1;
                }
            }

            // `map_values[].floor` restates the file's own `floor_id`, and the
            // coverage table calls it redundant on the strength of that. A cell
            // stating another floor is a cell in the wrong file: its pose would
            // put it on this file's floor, which the cell itself contradicts.
            if let Some(stated) = cell.raw.get("floor").filter(|stated| scalar(stated) != floor_id) {
                return Err(Error::Import(format!(
                    "map.json cell {} states `floor` {stated}, while the file states `floor_id` {floor_id}: one \
                     export is one floor, and the cell's pose would put it on the file's floor",
                    cell.coordinate
                )));
            }

            let mut external = Map::new();
            external.insert("barcode".into(), json!(cell.barcode));
            external.insert("mapcreator_floor_id".into(), json!(floor_id));
            external.insert("mapcreator_grid_coordinate".into(), json!(cell.coordinate));
            if let Some(vda) = cell.raw.get("vda_world_coordinate") {
                external.insert("vda_world_coordinate".into(), vda.clone());
            }
            if let Some(profile) = &cell.location_profile_id {
                if let Some(profile_id) = self.index.profile(profile) {
                    core.insert(
                        "$comment".into(),
                        json!(format!(
                            "traversable_by is read off location profile {profile_id}, which carries the \
                         permission table whole"
                        )),
                    );
                }
            }

            let Encoded { fields, formatting } = self.encode(
                records::CELL,
                &cell.raw,
                (FileKind::Map, &format!("/0/map_values/{at}"), "map.json[].map_values[]"),
                order[at],
                &format!("map.json cell {}", cell.coordinate),
            )?;
            // What Map Creator states of the cell beyond the core's four fields
            // rides on the same component, as the optional fields the protocol
            // states for a surveyed cell; no field map row lands on a core one.
            for (name, value) in fields {
                debug_assert!(!core.contains_key(&name), "`cell.{name}` is the core's and Map Creator's both");
                core.insert(name, value);
            }
            self.push(
                id,
                None,
                json!({
                    "pose": { "frame": floor_frame, "position": cell.world },
                    "cell": Value::Object(core),
                    "import_formatting": written::component(lens_of("cell"), formatting),
                    "external_ids": Value::Object(external)
                }),
            );
        }

        self.ledger.carry("map.json map_values[]", "one declared cell entity each", cells.len());
        self.ledger.carry("map.json map_values[].world_coordinate", "the cell's pose in the floor frame", cells.len());
        self.ledger.carry(
            "map.json map_values[].size_info",
            "cell.size_info whole, with cell.size the per-axis sum beside it",
            cells.len(),
        );
        self.ledger.carry_runtime("map.json map_values[].botid and .block_reasons", "cell.runtime", cells.len());
        self.ledger.cannot_state(
            "map.json:[].location_profiles[].bot_orientation_details per-model permission",
            Reading::LoadBearing,
            collapsed,
            "A permission that differs between two models of one role. `cell.traversable_by` names \
             agent classes and the validator matches it against `robot_class.role`, so where a \
             profile permits HTM_HAI_Q1_E1 and forbids HTM_HAI_Q2_E1 the role-level answer \
             `htm` is true of one and false of the other. The full table rides on the profile entity \
             and round-trips; what the CORE cannot say is the distinction.",
        );
        self.ledger.cannot_state(
            "map.json:[].map_values[] with no location_profile_id",
            Reading::LoadBearing,
            without_profile,
            "`no constraint`. Butler reads an absent permission as permissive; `cell.traversable_by` \
             is a positive list with no spelling for unrestricted, and writing every known class in \
             would grant a permission the export never granted. These cells come out with an empty \
             list.",
        );
        if degenerate > 0 {
            self.ledger.notes.push(format!(
                "{degenerate} cells state a size_info that sums to zero on an axis, so they carry no \
                 cell.size; the four half-extents still ride whole and come back out"
            ));
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Sectors and aisles
    // -----------------------------------------------------------------------

    /// One sector entity per sector key the cells name — `"undefined"` included.
    ///
    /// `undefined` is a real key: `sectors.json` lists it beside the numbered
    /// ones, and a cell that states it is a different fact from a cell that states
    /// nothing. Skipping it, as the one-way importer did, is what made the two
    /// indistinguishable.
    fn declare_sectors(&mut self) -> Result<(), Error> {
        let mut members: BTreeMap<String, (Value, Vec<Uuid>)> = BTreeMap::new();
        let mut order: Vec<String> = Vec::new();
        for cell in &self.site.cells {
            let Some(raw) = cell.raw.get("sector") else { continue };
            let key = scalar(raw);
            let Some(id) = self.index.cell_by_coordinate(&cell.coordinate) else { continue };
            let entry = members.entry(key.clone()).or_insert_with(|| {
                order.push(key.clone());
                (raw.clone(), Vec::new())
            });
            entry.1.push(id);
        }
        let mut ids: Vec<Uuid> = Vec::with_capacity(order.len());
        for key in &order {
            let id = self.resolver.id(&LookupKey::one("sector", key.clone()))?;
            self.index.add_sector(key, id, &members[key].0);
            ids.push(id);
        }
        let keys = self.ordering(&ids);
        for (at, key) in order.iter().enumerate() {
            let (spelling, cells) = &members[key];
            let id = ids[at];
            self.push(
                id,
                Some(format!("sector {key}")),
                json!({
                    "sector": { "members": cells },
                    // How the files spell the key: `map_values` writes a numbered
                    // sector as an integer and `undefined` as a string, while
                    // `sectors.json` keys on the string form of both, which
                    // `external_ids` holds. Keeping the JSON kind is what lets the
                    // export write each back as the file spells it.
                    "import_formatting": { (written::SECTOR): { "index": keys[at], "key": spelling.clone() } },
                    "external_ids": { "mapcreator_sector": key.clone() }
                }),
            );
        }
        let count = order.len();
        self.ledger.carry(
            "map.json map_values[].sector",
            "one sector entity per key, cells as explicit members; sectors.json is its inverse and is rebuilt from it",
            count,
        );
        Ok(())
    }

    /// One sector entity per aisle. `vtm_zone.json` states only which io-point
    /// cells belong to an aisle — no extent, no rack, no direction — so a sector,
    /// the format's shapeless named grouping, is what it is.
    fn declare_aisles(&mut self) -> Result<(), Error> {
        let aisles = self
            .dump
            .get(FileKind::VtmZone)
            .and_then(|v| v.get("aisle_info"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut ids: Vec<Uuid> = Vec::with_capacity(aisles.len());
        for aisle in &aisles {
            let aisle_id = aisle.get("aisle_id").map(scalar).unwrap_or_default();
            ids.push(self.resolver.id(&LookupKey::one("aisle", aisle_id))?);
        }
        let order = self.ordering(&ids);
        for (at, aisle) in aisles.iter().enumerate() {
            let raw = aisle.as_object().cloned().unwrap_or_default();
            let aisle_id = raw.get("aisle_id").map(scalar).unwrap_or_default();
            let display = raw.get("display_name").and_then(Value::as_str).unwrap_or(&aisle_id).to_string();
            let id = ids[at];
            let record = format!("vtm_zone.json aisle {}", at + 1);
            let mut cells: Vec<Uuid> = Vec::new();
            let mut unresolved: Vec<Value> = Vec::new();
            let mut as_pairs: Vec<usize> = Vec::new();
            for (place, point) in
                raw.get("io_point_coordinates").and_then(Value::as_array).into_iter().flatten().enumerate()
            {
                // A cell's own coordinate string, spelled exactly so, or its raw
                // pair: either is how the export writes the cell back, and which
                // of the two this entry was is recorded where it was a pair.
                let spelling = if point.is_string() { Spelling::Bracketed } else { Spelling::RawPair };
                match self.cell_named(point, spelling, &record, "io_point_coordinates")? {
                    Some(cell) => {
                        if point.is_array() {
                            as_pairs.push(place);
                        }
                        cells.push(cell);
                    }
                    None => unresolved.push(placed(place, point)),
                }
            }
            if !unresolved.is_empty() {
                self.dangling
                    .push(format!("aisle {aisle_id} names {} cells the map does not contain", unresolved.len()));
            }
            let Encoded { fields: mut component, mut formatting } = self.encode(
                records::AISLE,
                &raw,
                (FileKind::VtmZone, &format!("/aisle_info/{at}"), "vtm_zone.json.aisle_info[]"),
                order[at],
                &record,
            )?;
            if !unresolved.is_empty() {
                component.insert("io_point_coordinates_unresolved".into(), Value::Array(unresolved));
            }
            if !as_pairs.is_empty() {
                formatting.insert("io_point_coordinates_as_pairs".into(), json!(as_pairs));
            }
            self.push(
                id,
                Some(format!("aisle {display}")),
                json!({
                    "sector": { "members": cells },
                    "aisle": Value::Object(component),
                    "import_formatting": written::component(lens_of("aisle"), formatting),
                    "external_ids": { "mapcreator_aisle_id": aisle_id }
                }),
            );
        }
        let count = aisles.len();
        self.ledger.carry(
            "vtm_zone.json aisle_info[]",
            "one sector entity per aisle, its io-point cells as members",
            count,
        );
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Stitches
    // -----------------------------------------------------------------------

    /// One explicit stitch per adjacency both ends permit travel across.
    ///
    /// Butler decodes a `neighbours` triple as `[neighbour exists, bot may move,
    /// bot with load may move]`, so the second slot is what makes an adjacency an
    /// arc. A stitch is the protocol's reading of that, and it is *derived* from
    /// the adjacency the cell carries whole — which is why nothing here has to
    /// round-trip: the export writes `adjacency` and `neighbours` from the
    /// cell's own `cell`, not from these.
    fn declare_stitches(&mut self) -> Result<(), Error> {
        let site = self.site;
        let cells = &site.cells;
        let by_coordinate: BTreeMap<&str, &Cell> = cells.iter().map(|c| (c.coordinate.as_str(), c)).collect();
        let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
        let mut links = 0usize;
        let mut directional = 0usize;
        let mut impassable = 0usize;
        let mut graded = 0usize;
        let mut dangling = 0usize;
        let mut one_sided = 0usize;

        for cell in cells {
            for (direction, target) in cell.adjacency.iter().enumerate() {
                let Some(target) = target else { continue };
                let other_key = key_of(*target);
                let Some(other) = by_coordinate.get(other_key.as_str()) else {
                    dangling += 1;
                    continue;
                };
                // The grades at the other end are the ones toward this cell
                // only where that end's opposite slot names this cell back.
                let back = (direction + 2) % 4;
                if other.adjacency[back] != Some(cell.grid) {
                    one_sided += 1;
                    continue;
                }
                let pair = if cell.coordinate <= other_key {
                    (cell.coordinate.clone(), other_key.clone())
                } else {
                    (other_key.clone(), cell.coordinate.clone())
                };
                if !seen.insert(pair.clone()) {
                    continue;
                }
                let here = cell.neighbours[direction];
                let there = other.neighbours[back];
                match (here[1] == 1, there[1] == 1) {
                    (true, true) => {
                        if here[2] != 1 || there[2] != 1 {
                            graded += 1;
                        }
                        let (Some(a), Some(b)) =
                            (self.index.cell_by_coordinate(&pair.0), self.index.cell_by_coordinate(&pair.1))
                        else {
                            continue;
                        };
                        let key = format!("{}|{}", pair.0, pair.1);
                        let id = self.resolver.id(&LookupKey::one("stitch", key.clone()))?;
                        self.push(
                            id,
                            None,
                            json!({
                                "stitch": { "a": a, "b": b, "rule": "explicit", "pairs": [[a, b]] },
                                "external_ids": { "mapcreator_stitch": key }
                            }),
                        );
                        links += 1;
                    }
                    (true, false) | (false, true) => directional += 1,
                    (false, false) => impassable += 1,
                }
            }
        }

        self.ledger.carry(
            "map.json map_values[].adjacency both ends permit travel across",
            "one explicit stitch entity each, linking the two cells",
            links,
        );
        self.ledger.cannot_state(
            "map.json:[].map_values[].neighbours permitting travel from one end only",
            Reading::LoadBearing,
            directional,
            "A one-way lane. A stitch link is bidirectional, and SPEC §5 expresses a directional \
             seam by suppressing one direction's edge, which needs a link to exist first. Emitting \
             a bidirectional link here would grant travel the export withholds, so no link is \
             emitted and this adjacency is absent from the traversability graph — while the \
             adjacency itself rides on the cell and comes back out.",
        );
        self.ledger.cannot_state(
            "map.json:[].map_values[].neighbours == [1,0,0] at both ends",
            Reading::LoadBearing,
            impassable,
            "Two cells the export calls neighbours that no robot may travel between. The protocol \
             has one notion — a link is an arc — so `these are adjacent but you may not go` has no \
             core form. The adjacency rides on the cell and comes back out.",
        );
        self.ledger.cannot_state(
            "map.json:[].map_values[].neighbours third slot",
            Reading::LoadBearing,
            graded,
            "An edge a robot may cross empty but not loaded. A link carries no grade, so these \
             compile as ordinary two-way links and a core consumer cannot see the restriction. The \
             grades ride on the cell and come back out.",
        );
        self.ledger.cannot_state(
            "map.json:[].map_values[].adjacency naming a cell that does not name it back",
            Reading::LoadBearing,
            one_sided,
            "An adjacency one end states and the other does not: the named cell's opposite slot names \
             another cell or none, so its grades are toward that cell and say nothing of travel back. Reading \
             them as this pair's would grant travel the export withholds, so no link is emitted, as for a \
             one-way lane, while the adjacency rides on both cells and comes back out.",
        );
        self.ledger.cannot_state(
            "map.json:[].map_values[].adjacency naming a cell the map does not contain",
            Reading::LoadBearing,
            dangling,
            "An adjacency pointing at a coordinate with no cell. A dangling reference is a load \
             error in the protocol (SPEC §2), so it cannot become one — it stays data on the cell.",
        );
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Blocks
    // -----------------------------------------------------------------------

    /// Queues and waiting areas out of `block.json`.
    fn declare_blocks(&mut self) -> Result<(), Error> {
        let blocks = self.dump.get(FileKind::Block).and_then(Value::as_array).cloned().unwrap_or_default();
        let mut queues = 0usize;
        let mut waiting = 0usize;
        let mut other = 0usize;
        let mut sequenced = 0usize;

        let mut ids: Vec<Uuid> = Vec::with_capacity(blocks.len());
        for block in &blocks {
            let block_id = block.pointer("/entity_details/block_id").map(scalar).unwrap_or_default();
            let id = self.resolver.id(&LookupKey::one("block", block_id.clone()))?;
            self.index.add_block(&block_id, id);
            ids.push(id);
        }
        let order = self.ordering(&ids);

        for (at, block) in blocks.iter().enumerate() {
            let kind = block.get("entity_type").and_then(Value::as_str).unwrap_or_default().to_string();
            let details = block.get("entity_details").and_then(Value::as_object).cloned().unwrap_or_default();
            let block_id = details.get("block_id").map(scalar).unwrap_or_default();
            let id = ids[at];

            let mut path: Vec<Uuid> = Vec::new();
            let mut unresolved: Vec<Value> = Vec::new();
            let record = format!("block.json record {}", at + 1);
            for (place, point) in details.get("coordinates").and_then(Value::as_array).into_iter().flatten().enumerate()
            {
                match self.cell_named(point, Spelling::RawPair, &record, "coordinates")? {
                    Some(cell) => path.push(cell),
                    None => unresolved.push(placed(place, point)),
                }
            }
            if !unresolved.is_empty() {
                self.dangling
                    .push(format!("block {block_id} names {} cells the map does not contain", unresolved.len()));
            }

            let stands = format!("/{at}");
            let Encoded { fields: mut component, formatting } =
                self.encode(records::BLOCK, &details, (FileKind::Block, &stands, "block.json[]"), order[at], &record)?;
            component.insert("entity_type".into(), json!(kind));
            if !unresolved.is_empty() {
                component.insert("coordinates_unresolved".into(), Value::Array(unresolved));
            }
            if details.get("extra_details").is_some() {
                sequenced += 1;
            }

            let mut components = Map::new();
            match kind.as_str() {
                // The path is the whole of a queue: the run it admits on, so
                // nothing beside it says how much of the run counts (SPEC §10).
                kind if records::QUEUE_KINDS.contains(&kind) => {
                    components.insert("queue".into(), json!({ "path": path }));
                    queues += 1;
                }
                kind if records::WAITING_KINDS.contains(&kind) => {
                    components.insert("waiting".into(), json!({}));
                    // SPEC §10 gives a waiting area no way to name its cells, so
                    // the membership the block states rides here rather than
                    // nowhere.
                    component.insert("cells".into(), json!(path));
                    waiting += 1;
                }
                _ => {
                    component.insert("cells".into(), json!(path));
                    other += 1;
                }
            }
            components.insert("block".into(), Value::Object(component));
            components.insert(written::COMPONENT.into(), written::component(lens_of("block"), formatting));
            components.insert("external_ids".into(), json!({ "mapcreator_block_id": block_id.clone() }));
            self.push(id, Some(format!("block {block_id} ({kind})")), Value::Object(components));
        }

        self.ledger.carry(
            "block.json entity_type queue and sequenced_queue",
            "a queue component whose path is the block's cells",
            queues,
        );
        self.ledger.carry("block.json entity_type waiting", "a waiting component, with the cells on block", waiting);
        self.ledger.carry("block.json other entity_type", "a block entity with its cells", other);
        self.ledger.cannot_state(
            "block.json:[].entity_details.extra_details sync_point and exit_point",
            Reading::LoadBearing,
            sequenced,
            "The step a sequenced queue synchronises at and the step it leaves by. A core queue has \
             a path and a discipline, and no way to tell one step from another. The extras \
             ride on block and come back out.",
        );
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Stations
    // -----------------------------------------------------------------------

    /// Stations and their points, in both export dialects.
    fn declare_stations(&mut self) -> Result<(), Error> {
        let stations = self.dump.get(FileKind::Pps).and_then(Value::as_array).cloned().unwrap_or_default();
        // Two passes: every station gets its identity first, because
        // `dock_point.json` and `conveyor_mapping.json` point at one and
        // `pps.json` itself does not.
        let mut ids: Vec<Uuid> = Vec::with_capacity(stations.len());
        for station in &stations {
            let pps_id = station.get("pps_id").map(scalar).unwrap_or_default();
            let id = self.resolver.id(&LookupKey::one("station", pps_id.clone()))?;
            self.index.add_station(&pps_id, id);
            ids.push(id);
        }
        let order = self.ordering(&ids);

        let mut points = 0usize;
        let mut ambiguous = 0usize;
        let mut associations = 0usize;
        let mut geometry = 0usize;
        let mut unordered = 0usize;
        let floor_frame = self.floor_frame;

        for (at, station) in stations.iter().enumerate() {
            let raw = station.as_object().cloned().unwrap_or_default();
            let pps_id = raw.get("pps_id").map(scalar).unwrap_or_default();
            let id = ids[at];
            let v3 = raw.contains_key("entities");
            let map = if v3 { records::STATION_V3 } else { records::STATION_V2 };
            let kind_source = raw.get("type").and_then(Value::as_str).unwrap_or("").to_string();
            let modes: Vec<String> = raw
                .get("allowed_modes")
                .and_then(Value::as_array)
                .map(|m| m.iter().map(scalar).collect())
                .unwrap_or_default();

            let place = format!("pps.json record {}", at + 1);
            let stands = format!("/{at}");
            let Encoded { fields: mut component, mut formatting } =
                self.encode(map, &raw, (FileKind::Pps, &stands, "pps.json[]"), order[at], &place)?;
            // Which of the two dialect tables this record was read by. The export
            // used to guess it from the presence of `served_by`/`system`, which is
            // a guess about the shape of a record from the contents of one — and a
            // v3 station with neither came back out in the v2 spelling. `dialect`
            // beside it is the export's own `version` value, which is what the
            // file states and not what the shape is.
            formatting.insert("dialect_shape".into(), json!(if v3 { "v3" } else { "v2" }));
            // A station's `type` is its role, and a station that states none has
            // none: the export writes `type` back only from a role.
            if raw.contains_key("type") {
                component.insert("role".into(), json!(kind_source));
            }
            // A point list the record does not state, which the export would
            // otherwise write back empty.
            let points_list = if v3 { "entities.presentation_points" } else { "pps_points" };
            let stated =
                if v3 { raw.get("entities").and_then(|e| e.get("presentation_points")) } else { raw.get("pps_points") };
            if stated.is_none() {
                formatting.insert("absent".into(), json!([points_list]));
            }

            // The station's pose is the cell `at_cell` resolved to, read off that
            // very cell, so the reference and the pose cannot name two cells.
            let mut components = Map::new();
            if let Some(position) = self.position_of(component.get("at_cell")) {
                components.insert("pose".into(), json!({ "frame": floor_frame, "position": position }));
            }
            match station_kind(&kind_source, &modes) {
                Some(kind) => {
                    self.params.note("station_kind", kind);
                    let mut core = json!({ "kind": kind });
                    if raw.contains_key("type") {
                        core["role"] = json!(kind_source);
                    }
                    components.insert("station".into(), core);
                }
                None => ambiguous += 1,
            }

            // v3: the queues that serve the station, by identity.
            if let Some(blocks) = raw.get("entities").and_then(|e| e.get("blocks")).and_then(Value::as_array) {
                let mut served: Vec<Uuid> = Vec::new();
                let mut unresolved: Vec<Value> = Vec::new();
                for (place, block) in blocks.iter().enumerate() {
                    match self.index.joined(Join::Block, KeyText::Text, block, "") {
                        Some(block) => served.push(block),
                        None => unresolved.push(placed(place, block)),
                    }
                }
                component.insert("served_by".into(), json!(served));
                if !unresolved.is_empty() {
                    component.insert("served_by_unresolved".into(), Value::Array(unresolved));
                }
            }
            if raw.get("associations").and_then(Value::as_array).is_some_and(|a| !a.is_empty()) {
                associations += 1;
            }
            if raw.get("queue_barcodes").and_then(Value::as_array).is_some_and(|q| !q.is_empty()) {
                unordered += 1;
            }

            components.insert("mapcreator.station".into(), Value::Object(component));
            components.insert(written::COMPONENT.into(), written::component(lens_of("station"), formatting));
            let mut external = Map::new();
            external.insert("mapcreator_pps_id".into(), json!(pps_id.clone()));
            // The `pps_id`'s own JSON kind differs by dialect — an integer in v3
            // and a string in v2 — so the spelling rides on the component and the
            // external id is the string both index the same way.
            components.insert("external_ids".into(), Value::Object(external));
            self.push(id, Some(format!("pps {pps_id} ({kind_source})")), Value::Object(components));

            points += self.declare_station_points(&raw, &pps_id, v3, &mut geometry, (at, &place))?;
        }

        self.ledger.carry("pps.json []", "one station entity each, posed at the cell it names", stations.len());
        self.ledger.carry(
            "pps.json presentation_points[] and pps_points[]",
            "one presentation_point entity each",
            points,
        );
        self.ledger.cannot_state(
            "pps.json:[].allowed_modes naming more than one of pick and put",
            Reading::LoadBearing,
            ambiguous,
            "A station that is several kinds at once. SPEC §10 gives a station one kind, and picking \
             one of pick/put here would invent a restriction the site does not have. These entities \
             carry no `station` component; the type and the modes ride on mapcreator.station and \
             come back out.",
        );
        self.ledger.cannot_state(
            "pps.json:[].associations",
            Reading::LoadBearing,
            associations,
            "A station's internal transport graph: a typed connected graph over station-local \
             handles saying which transports between a station's parts are allowed. SPEC §2 wants \
             this shape with entity refs for endpoints, and the core has no component for it, so it \
             rides on mapcreator.station as the export spells it — handles and all.",
        );
        self.ledger.cannot_state(
            "pps.json:[].queue_barcodes",
            Reading::LoadBearing,
            unordered,
            "A queue whose order nobody stated. The v2 dialect lists six to eight cell barcodes with \
             no order, no entry or exit designation, and no relation to block.json. A core queue path \
             is ordered, so importing this list as one would invent the order — and the order is the \
             queue. The cells ride on mapcreator.station in file order and come back out in it.",
        );
        self.ledger.cannot_state(
            "pps.json:[].entities.presentation_points[].attributes geometry",
            Reading::LoadBearing,
            geometry,
            "The height a carrier is presented at, the robot class that presents it, the heading it \
             presents at, the direction the operator reaches from, and whether this is where the \
             operator works. The core's presentation_point carries one field, `for`; the rest ride \
             on the same presentation_point and come back out.",
        );
        Ok(())
    }

    /// The measured position of the cell one encoded reference names: the cell
    /// the reference resolved to, so a pose and a reference agree by
    /// construction.
    fn position_of(&self, reference: Option<&Value>) -> Option<[f64; 2]> {
        let cell = reference?.as_str()?.parse::<Uuid>().ok()?;
        self.index.position(cell)
    }

    /// A station's presentation points, in whichever dialect it states them.
    /// `station` is the station record's place in `pps.json`, as a number and as
    /// the words a sentence names it by.
    fn declare_station_points(
        &mut self,
        raw: &Map<String, Value>,
        pps_id: &str,
        v3: bool,
        geometry: &mut usize,
        station: (usize, &str),
    ) -> Result<usize, Error> {
        let (station_at, station) = station;
        let floor_frame = self.floor_frame;
        let mut count = 0usize;
        if v3 {
            let list = raw
                .get("entities")
                .and_then(|e| e.get("presentation_points"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let mut ids: Vec<Uuid> = Vec::with_capacity(list.len());
            for point in &list {
                let point_id = point.get("point_id").map(scalar).unwrap_or_default();
                ids.push(self.resolver.id(&LookupKey::two("presentation_point", pps_id.to_string(), point_id))?);
            }
            let order = self.ordering(&ids);
            for (at, point) in list.iter().enumerate() {
                let record = point.as_object().cloned().unwrap_or_default();
                let point_id = record.get("point_id").map(scalar).unwrap_or_default();
                let id = ids[at];
                let place = format!("{station}, presentation point {}", at + 1);
                let stands = format!("/{station_at}/entities/presentation_points/{at}");
                let stands_at = (FileKind::Pps, stands.as_str(), "pps.json[].entities.presentation_points[]");
                let Encoded { fields: mut component, mut formatting } =
                    self.encode(records::PRESENTATION_POINT, &record, stands_at, order[at], &place)?;
                // A point's station is not a field of its record — a point sits
                // inside the station's own record — so the reference is written
                // here, and `pps.json` rebuilds the nesting from it.
                if let Some(station) = self.index.station(pps_id) {
                    component.insert("station".into(), json!(station));
                }
                // The export writes `attributes.world_coordinates` back from the
                // cell the barcode names, which `redundancy` has held them to.
                // What that cell cannot give back is recorded instead: that the
                // point states none, and, where the barcode names no cell, the
                // coordinates as the file wrote them.
                if let Some(attributes) = record.get("attributes").and_then(Value::as_object) {
                    let named = component.get("attributes").and_then(|nested| nested.get("cell")).is_some();
                    match attributes.get("world_coordinates") {
                        None => {
                            if let Value::Array(fields) = formatting.entry("absent").or_insert_with(|| json!([])) {
                                fields.push(json!("attributes.world_coordinates"));
                            }
                        }
                        Some(stated) if !named => {
                            component.insert("world_coordinates_unresolved".into(), stated.clone());
                        }
                        Some(_) => {}
                    }
                }
                *geometry += 1;
                let mut components = Map::new();
                if let Some(position) = self.position_of(component.get("attributes").and_then(|a| a.get("cell"))) {
                    components.insert("pose".into(), json!({ "frame": floor_frame, "position": position }));
                }
                // One component: the core's `for`, and beside it what Map Creator
                // states of the point, as the optional fields the protocol
                // states for a presentation point.
                if let Some(purpose) = record.get("presented_for").and_then(Value::as_str) {
                    component.insert("for".into(), json!(purpose));
                }
                components.insert("presentation_point".into(), Value::Object(component));
                components
                    .insert(written::COMPONENT.into(), written::component(lens_of("presentation_point"), formatting));
                components.insert(
                    "external_ids".into(),
                    json!({ "mapcreator_of_pps": pps_id, "mapcreator_point_id": point_id }),
                );
                self.push(id, Some(format!("pps {pps_id} presentation point {point_id}")), Value::Object(components));
                count += 1;
            }
        } else {
            let list = raw.get("pps_points").and_then(Value::as_array).cloned().unwrap_or_default();
            let mut ids: Vec<Uuid> = Vec::with_capacity(list.len());
            for point in &list {
                let position_id = point.get("position_id").map(scalar).unwrap_or_default();
                ids.push(self.resolver.id(&LookupKey::two("station_point", pps_id.to_string(), position_id))?);
            }
            let order = self.ordering(&ids);
            for (at, point) in list.iter().enumerate() {
                let record = point.as_object().cloned().unwrap_or_default();
                let position_id = record.get("position_id").map(scalar).unwrap_or_default();
                let id = ids[at];
                let place = format!("{station}, point {}", at + 1);
                let stands = format!("/{station_at}/pps_points/{at}");
                let stands_at = (FileKind::Pps, stands.as_str(), "pps.json[].pps_points[]");
                let Encoded { fields: mut component, formatting } =
                    self.encode(records::STATION_POINT, &record, stands_at, order[at], &place)?;
                if let Some(station) = self.index.station(pps_id) {
                    component.insert("station".into(), json!(station));
                }
                let mut components = Map::new();
                if let Some(position) = self.position_of(component.get("cell")) {
                    components.insert("pose".into(), json!({ "frame": floor_frame, "position": position }));
                }
                components.insert("presentation_point".into(), json!({}));
                components.insert("station_point".into(), Value::Object(component));
                components.insert(written::COMPONENT.into(), written::component(lens_of("station_point"), formatting));
                components.insert(
                    "external_ids".into(),
                    json!({ "mapcreator_of_pps": pps_id, "mapcreator_position_id": position_id }),
                );
                self.push(id, Some(format!("pps {pps_id} point {position_id}")), Value::Object(components));
                count += 1;
            }
        }
        Ok(count)
    }

    // -----------------------------------------------------------------------
    // The field-map engine
    // -----------------------------------------------------------------------

    /// One record's own fields, placed by its field map.
    ///
    /// Every key the record carries must have a row in the map. A key with no row
    /// is a field family nobody has decided about, which is the one thing this
    /// crate refuses outright: a converter that silently carries an unknown field
    /// nowhere is a converter whose losslessness is a claim rather than a fact.
    ///
    /// `at` is where the record's whole object stands — the record itself, but
    /// for a zone and a block, whose fields sit one object down — as its file,
    /// its pointer in the file and its place. The formatting records that
    /// object's key orders and whole numbers written with a decimal point,
    /// where they are not what the file usually writes (`written`).
    fn encode(
        &mut self,
        map: &'static [Field],
        record: &Map<String, Value>,
        at: (FileKind, &str, &str),
        index: i64,
        place: &str,
    ) -> Result<Encoded, Error> {
        let mut encoded = self.encode_fields(map, record, place)?;
        let (file, pointer, stands) = at;
        if let Some(shape) = self.dump.shape(file).and_then(|shape| shape.at(pointer)) {
            Facts::of_record(shape, stands, &self.usual).stamp(&mut encoded.formatting);
        }
        // The record's ordering key in its file. A lens list comes back in
        // identity order and the identities are random, so the order a file states
        // its records in is a fact the document has to carry (spec §5). It is the
        // record's and nothing else's: a nested object inside one record is not a
        // record, and has no order of its own to state.
        encoded.formatting.insert("index".into(), json!(index));
        Ok(encoded)
    }

    /// One record's fields alone, without the ordering key — which is what a
    /// nested object of a record gets.
    fn encode_fields(
        &mut self,
        map: &'static [Field],
        record: &Map<String, Value>,
        place: &str,
    ) -> Result<Encoded, Error> {
        let mut out = Map::new();
        let mut formatting = Map::new();
        let mut runtime = Map::new();
        // A field the record does not state that the export would write back
        // from another, which an echo is: its absence is how the file wrote the
        // record, and the record's formatting carries it.
        let absent: Vec<&str> = map
            .iter()
            .filter(|field| matches!(field.slot, Slot::Echo { .. }) && !record.contains_key(field.source))
            .map(|field| field.source)
            .collect();
        if !absent.is_empty() {
            formatting.insert("absent".into(), json!(absent));
        }
        for (key, value) in record {
            let Some(field) = field(map, key) else {
                return Err(Error::Document(format!(
                    "a record carries `{key}`, which no row of this adapter's field map disposes of, so \
                     where it would go is undecided: a field family nobody has decided about is refused \
                     rather than carried nowhere"
                )));
            };
            match field.slot {
                Slot::Verbatim(_) => {
                    out.insert(field.target.to_string(), value.clone());
                }
                Slot::Runtime(_) => {
                    runtime.insert(field.target.to_string(), value.clone());
                }
                Slot::Written(_) => {
                    formatting.insert(field.target.to_string(), value.clone());
                }
                // A key rides in `external_ids`, which the caller writes from the
                // record's lookup key; nothing of it belongs in the component.
                Slot::Key { .. } | Slot::Elsewhere(_) => {}
                Slot::Cell(spelling) => self.place_cell(&mut out, field, value, spelling, place)?,
                Slot::Cells(spelling) => self.place_cells(&mut out, field, value, spelling, place)?,
                Slot::Join(join, text) => match self.index.joined(join, text, value, &station_of(record)) {
                    Some(named) => {
                        out.insert(field.target.to_string(), json!(named));
                    }
                    None => {
                        self.dangling.push(format!(
                            "`{}` = {value} names no {} of this dump",
                            field.source,
                            join.what()
                        ));
                        out.insert(format!("{}_unresolved", field.target), value.clone());
                    }
                },
                Slot::Blocks => {
                    let mut resolved: Vec<Value> = Vec::new();
                    let mut unresolved: Vec<Value> = Vec::new();
                    for (place, item) in value.as_array().map(Vec::as_slice).unwrap_or_default().iter().enumerate() {
                        match self.index.joined(Join::Block, KeyText::Text, item, "") {
                            Some(block) => resolved.push(json!(block)),
                            None => unresolved.push(placed(place, item)),
                        }
                    }
                    out.insert(field.target.to_string(), Value::Array(resolved));
                    if !unresolved.is_empty() {
                        out.insert(format!("{}_unresolved", field.target), Value::Array(unresolved));
                    }
                }
                // Carried only where it differs from the field it repeats.
                // Carrying it always would put one fact in two places on every row
                // of every site; dropping it would lose the row where a site's two
                // spellings have parted.
                Slot::Echo { of } => {
                    if record.get(of) != Some(value) {
                        out.insert(field.target.to_string(), value.clone());
                    }
                }
                // What a nested object's own fields say of how the file wrote
                // them rides in the formatting under the same name.
                Slot::Nested(inner) => {
                    let none = Map::new();
                    let nested = self.encode_fields(inner, value.as_object().unwrap_or(&none), place)?;
                    out.insert(field.target.to_string(), Value::Object(nested.fields));
                    if !nested.formatting.is_empty() {
                        formatting.insert(field.target.to_string(), Value::Object(nested.formatting));
                    }
                }
            }
        }
        if !runtime.is_empty() {
            out.insert("runtime".into(), Value::Object(runtime));
        }
        Ok(Encoded { fields: out, formatting })
    }

    /// The cell one reference names, through the one function every reference
    /// and every pose resolves through; `None` for a reference that names no
    /// cell, which is kept as written. A barcode two cells carry is refused,
    /// naming the cells and the record: which one it means would be a guess.
    fn cell_named(&self, value: &Value, spelling: Spelling, place: &str, field: &str) -> Result<Option<Uuid>, Error> {
        match self.index.cell(value, spelling) {
            CellNamed::One(cell) => Ok(Some(cell)),
            CellNamed::Nothing => Ok(None),
            CellNamed::Shared(barcode) => {
                let cells: Vec<&str> = self
                    .site
                    .cells
                    .iter()
                    .filter(|cell| cell.barcode == barcode)
                    .map(|cell| cell.coordinate.as_str())
                    .collect();
                Err(Error::Import(format!(
                    "{place} names the barcode {barcode} in `{field}`, which {} cells carry, {}: which of them it \
                     means would be a guess, and the import refuses rather than make it",
                    cells.len(),
                    cells.join(" and ")
                )))
            }
        }
    }

    fn place_cell(
        &mut self,
        out: &mut Map<String, Value>,
        field: &Field,
        value: &Value,
        spelling: Spelling,
        place: &str,
    ) -> Result<(), Error> {
        match self.cell_named(value, spelling, place, field.source)? {
            Some(cell) => {
                out.insert(field.target.to_string(), json!(cell));
            }
            None => {
                self.dangling.push(format!("`{}` = {value} names no cell of this dump", field.source));
                out.insert(format!("{}_unresolved", field.target), value.clone());
            }
        }
        Ok(())
    }

    fn place_cells(
        &mut self,
        out: &mut Map<String, Value>,
        field: &Field,
        value: &Value,
        spelling: Spelling,
        place: &str,
    ) -> Result<(), Error> {
        let mut resolved: Vec<Value> = Vec::new();
        let mut unresolved: Vec<Value> = Vec::new();
        for (at, item) in value.as_array().map(Vec::as_slice).unwrap_or_default().iter().enumerate() {
            match self.cell_named(item, spelling, place, field.source)? {
                Some(cell) => resolved.push(json!(cell)),
                None => {
                    self.dangling.push(format!("`{}` = {item} names no cell of this dump", field.source));
                    unresolved.push(placed(at, item));
                }
            }
        }
        out.insert(field.target.to_string(), Value::Array(resolved));
        if !unresolved.is_empty() {
            out.insert(format!("{}_unresolved", field.target), Value::Array(unresolved));
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // The flat record files, the preferences, and the export's own header
    // -----------------------------------------------------------------------

    /// Every record kind whose file is a flat list and whose fields the field map
    /// handles whole: chargers, dock points, exclusions, storage positions, rack
    /// types, conveyors, conveyor mappings, bins, bin types, seats and lights.
    ///
    /// Two passes, and the reason is a real defect the one-pass version had. A
    /// flat record may name a record of a file that is read later — a bin names
    /// its bin type and its two operator seats, and `pps_bintype.json` and
    /// `pps_seat.json` come after `pps_bin.json` — so encoding a record the
    /// moment its identity is settled left every one of site-e's 40 bins
    /// with no reference to either. The first pass therefore settles the
    /// identity of every record of every flat file and registers it; the second
    /// encodes, by which time every join table is full and no reference can
    /// resolve to nothing for want of reading order.
    fn declare_flat_records(&mut self) -> Result<(), Error> {
        let dump = self.dump;
        let mut allocated: Vec<(&'static RecordKind, Vec<FlatRecord<'_>>)> = Vec::new();
        for kind in records::KINDS {
            let Some(file) = dump.get(kind.file) else { continue };
            let mut rows: Vec<FlatRecord<'_>> = Vec::new();
            // How many devices each `pptl.json` group has had so far, counted
            // as the rows go rather than by a recount per row.
            let mut devices: FxHashMap<usize, usize> = FxHashMap::default();
            for (at, (group, record)) in layout::flat_rows(kind.file, file).into_iter().enumerate() {
                let key = self.flat_key(kind.construct, record)?;
                let id = self.resolver.id(&key)?;
                self.register_flat(kind.construct, record, id);
                // The row's place in the list that holds it: its group's, in
                // `pptl.json`, and the file's own everywhere else.
                let in_list = if kind.file == FileKind::Pptl {
                    let device = devices.entry(group).or_default();
                    *device += 1;
                    *device - 1
                } else {
                    at
                };
                let place = if kind.file == FileKind::Pptl {
                    format!("{} group {}, device {}", kind.file, group + 1, in_list + 1)
                } else {
                    format!("{} record {}", kind.file, at + 1)
                };
                let pointer = layout::record_pointer(kind.file, group, in_list);
                rows.push(FlatRecord { group, pointer, place, record, key, id });
            }
            allocated.push((kind, rows));
        }

        for (kind, rows) in &allocated {
            let order = self.ordering(&rows.iter().map(|row| row.id).collect::<Vec<Uuid>>());
            for (at, row) in rows.iter().enumerate() {
                let stands = layout::record_place(kind.file);
                let stands_at = (kind.file, row.pointer.as_str(), stands.as_str());
                let Encoded { fields: mut component, mut formatting } =
                    self.encode(kind.fields, row.record, stands_at, order[at], &row.place)?;
                if kind.file == FileKind::Pptl {
                    // `pptl.json` states its devices in groups, and a group is not
                    // one station's: site-e lists ten stations in twenty
                    // groups, the same ten twice over with different peripheral
                    // ids. So the group is the device's place in the file, which
                    // nothing in a record determines, and rides with its index.
                    formatting.insert("group".into(), json!(row.group));
                }
                self.extend_flat(kind.construct, row.record, &mut component, &row.place)?;
                let mut components = Map::new();
                components.insert(kind.component.to_string(), Value::Object(component));
                components.insert(written::COMPONENT.into(), written::component(kind.lens, formatting));
                let mut external = Map::new();
                for (name, value) in row.key.external_ids()? {
                    external.insert(name.to_string(), json!(value));
                }
                components.insert("external_ids".into(), Value::Object(external));
                self.push(row.id, self.flat_name(kind.construct, row.record), Value::Object(components));
            }
            if !rows.is_empty() {
                self.ledger.carry(
                    format!("{} []", kind.file.name()),
                    format!("one {} entity each", kind.component),
                    rows.len(),
                );
            }
        }
        Ok(())
    }

    /// One flat record's lookup key, per `DECISIONS.md`.
    fn flat_key(&self, construct: &str, record: &Map<String, Value>) -> Result<LookupKey, Error> {
        let stated = |name: &str| record.get(name).map(scalar).unwrap_or_default();
        let key = match construct {
            "charger" => LookupKey::one("charger", stated("charger_id")),
            "dock_point" => LookupKey::one("dock_point", stated("id")),
            "ods_exclusion" => LookupKey::one("ods_exclusion", stated("ods_tuple")),
            "tote_storage" => LookupKey::one("tote_storage", stated("location")),
            "rack_type" => LookupKey::one("rack_type", stated("racktype_id")),
            "conveyor" => LookupKey::one("conveyor", stated("conveyor_id")),
            "conveyor_mapping" => {
                let half = |name: &str| {
                    record.get("conveyor_coordinates").and_then(|c| c.get(name)).map(scalar).unwrap_or_default()
                };
                LookupKey::two("conveyor_mapping", half("conveyor_id"), half("step_id"))
            }
            "pps_bin" => LookupKey::two("pps_bin", stated("pps_id"), stated("bin_id")),
            "pps_bin_type" => LookupKey::one("pps_bin_type", stated("type_id")),
            "pps_seat" => LookupKey::two("pps_seat", stated("pps_id"), stated("seat_name")),
            "pptl" => LookupKey::one("pptl", stated("peripheral_id")),
            other => return Err(Error::Document(format!("no flat lookup key rule for `{other}`"))),
        };
        Ok(key)
    }

    fn register_flat(&mut self, construct: &str, record: &Map<String, Value>, id: Uuid) {
        let stated = |name: &str| record.get(name).map(scalar).unwrap_or_default();
        match construct {
            "pps_seat" => self.index.add_seat(&stated("pps_id"), &stated("seat_name"), id),
            "pps_bin" => self.index.add_bin(&stated("pps_id"), &stated("bin_id"), id),
            "pps_bin_type" => self.index.add_bin_type(&stated("type_id"), id),
            "conveyor" => self.index.add_conveyor(&stated("conveyor_id"), id),
            _ => {}
        }
    }

    /// What a flat record's field map cannot say: an exclusion's tuple unpacked
    /// into the cell and the heading it names.
    fn extend_flat(
        &mut self,
        construct: &str,
        record: &Map<String, Value>,
        component: &mut Map<String, Value>,
        place: &str,
    ) -> Result<(), Error> {
        match construct {
            "ods_exclusion" => {
                let tuple = record.get("ods_tuple").and_then(Value::as_str).unwrap_or_default();
                match tuple.rsplit_once("--") {
                    Some((barcode, heading)) => {
                        match self.cell_named(&json!(barcode), Spelling::Barcode, place, "ods_tuple")? {
                            Some(cell) => {
                                component.insert("cell".into(), json!(cell));
                            }
                            None => {
                                self.dangling.push(format!("obstacle-detection exclusion {tuple} names no cell"));
                                component.insert("cell_barcode_unresolved".into(), json!(barcode));
                            }
                        }
                        component.insert("heading".into(), json!(heading));
                    }
                    None => {
                        component.insert("tuple_unparsed".into(), json!(tuple));
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn flat_name(&self, construct: &str, record: &Map<String, Value>) -> Option<String> {
        let stated = |name: &str| record.get(name).map(scalar).unwrap_or_default();
        match construct {
            "charger" => Some(format!("charger {}", stated("charger_id"))),
            "dock_point" => Some(format!("dock point {}", stated("display_name"))),
            "rack_type" => Some(format!("rack type {}", stated("racktype_id"))),
            "conveyor" => Some(format!("conveyor {}", stated("display_name"))),
            "pps_bin" => Some(format!("pps {} bin {}", stated("pps_id"), stated("bin_id"))),
            "pps_bin_type" => Some(format!("bin type {}", stated("type_id"))),
            "pps_seat" => Some(format!("seat {}", stated("seat_name"))),
            "pptl" => Some(format!("put-to-light {}", stated("peripheral_id"))),
            _ => None,
        }
    }

    /// Carrier type to an ordered list of preferred sectors — a storage policy,
    /// and the kind of thing SPEC §7 keeps in an overlay document.
    fn declare_sector_preferences(&mut self) -> Result<(), Error> {
        // One object of carrier types, or that object as the one element of a
        // list, which the export entity's record of the file's form brings back.
        let file = self.dump.get(FileKind::SectorMxUPreferences);
        let table = file
            .and_then(|file| file.as_object().or_else(|| file.get(0).and_then(Value::as_object)))
            .cloned()
            .unwrap_or_default();
        let mut ids: Vec<Uuid> = Vec::with_capacity(table.len());
        for carrier in table.keys() {
            ids.push(self.resolver.id(&LookupKey::one("sector_preference", carrier.clone()))?);
        }
        let order = self.ordering(&ids);
        let in_a_list = file.is_some_and(Value::is_array);
        for (at, (carrier, sectors)) in table.iter().enumerate() {
            let id = ids[at];
            // How the file wrote the list, a sector key spelled `1.0` among it,
            // is the preference's, so it moves with the preference's key.
            let mut formatting = Map::new();
            let (pointer, place) = written::preference_list(carrier, in_a_list);
            let list = pointer.strip_prefix(FileKind::SectorMxUPreferences.name()).unwrap_or_default();
            if let Some(shape) = self.dump.shape(FileKind::SectorMxUPreferences).and_then(|shape| shape.at(list)) {
                Facts::of_record(shape, &place, &self.usual).stamp(&mut formatting);
            }
            formatting.insert("index".into(), json!(order[at]));
            let mut prefers: Vec<Value> = Vec::new();
            let mut unresolved: Vec<Value> = Vec::new();
            for (place, sector) in sectors.as_array().map(Vec::as_slice).unwrap_or_default().iter().enumerate() {
                // A preference may name a sector no cell belongs to — site-b
                // prefers sectors 1 and 2 and every one of its 3,177 cells states
                // `undefined` — and a sector with no cells is a sector with no
                // entity. The key then stays as the file wrote it, at its place
                // in the list, rather than minting an entity nothing else in the
                // document mentions, and so does a key spelled otherwise than the
                // cells spell it, which the export would write back in the cells'
                // spelling.
                match self.index.sector(sector) {
                    Some(sector_id) => prefers.push(json!(sector_id)),
                    None => unresolved.push(placed(place, sector)),
                }
            }
            if !unresolved.is_empty() {
                self.dangling.push(format!(
                    "the sector preference for {carrier} names {} sectors no cell states",
                    unresolved.len()
                ));
            }
            let mut component = json!({ "prefers": prefers });
            if !unresolved.is_empty() {
                component["prefers_unresolved"] = Value::Array(unresolved);
            }
            self.push(
                id,
                Some(format!("sector preference for {carrier}")),
                json!({
                    "sector_preference": component,
                    "import_formatting": written::component(written::SECTOR_PREFERENCE, formatting),
                    "external_ids": { "mapcreator_preferred_for": carrier.clone() }
                }),
            );
        }
        let count = table.len();
        self.ledger.carry("sectorMxUPreferences.json", "one sector_preference entity per carrier type", count);
        self.ledger.cannot_state(
            "sectorMxUPreferences.json as a storage policy",
            Reading::EmittedOnly,
            count,
            "A preference ordering over sectors for a carrier type. SPEC §7 keeps a policy in an \
             overlay document rather than in world data, and no core component states one; this \
             rides as a Map Creator component and comes back out. One site prefers sectors for a \
             rack type its own tote_type.json does not declare.",
        );
        Ok(())
    }

    /// The export's own table of contents: which files the site shipped, which of
    /// them were empty and in which form, and the per-file wrappers that are
    /// envelopes rather than model data.
    ///
    /// Nothing else can carry this. A file a site never shipped and a file it
    /// shipped empty are two different facts, and `[]` and `{}` are two more, so
    /// the export has to be told rather than guess.
    fn declare_export_entity(&mut self) -> Result<(), Error> {
        // A site holds one Map Creator export, so the key is the same for every
        // site. The crate this was ported from keyed it by the dump directory's
        // name, and an adapter is handed files and no directory.
        let id = self.resolver.id(&LookupKey::one("export", EXPORT_KEY))?;
        let mut files = Map::new();
        for kind in self.dump.present() {
            let form = match self.dump.get(kind) {
                Some(Value::Array(items)) if items.is_empty() => "empty_array",
                Some(Value::Object(map)) if map.is_empty() => "empty_object",
                Some(Value::Array(_)) => "array",
                Some(Value::Object(_)) => "object",
                _ => "other",
            };
            files.insert(kind.name().to_string(), json!(form));
            self.ledger.files.insert(kind.name().to_string(), form.to_string());
        }
        let mut component = Map::new();
        component.insert("files".into(), Value::Object(files));
        // How the files were written, beyond their values: each place's usual
        // key orders, which every record's own order is recorded against; the
        // key orders and whole numbers written with a decimal point of
        // everything outside the records; and each file's separators and the
        // whitespace around it, where they are not the compact form.
        if !self.usual.is_empty() {
            let orders: Map<String, Value> =
                self.usual.iter().map(|(place, orders)| (place.clone(), json!(orders))).collect();
            component.insert("key_orders".into(), Value::Object(orders));
        }
        let mut outside = Facts::default();
        let mut spacings = Map::new();
        for kind in self.dump.present() {
            if let Some(shape) = self.dump.shape(kind) {
                let mut facts = Facts::default();
                Facts::outside_records(kind.name(), shape, &self.usual, &mut facts);
                // Each preference's list is the preference's, which carries
                // how the file wrote it (`declare_sector_preferences`).
                if kind == FileKind::SectorMxUPreferences {
                    let file = self.dump.get(kind);
                    let table = file.and_then(|file| file.as_object().or_else(|| file.get(0)?.as_object()));
                    for carrier in table.into_iter().flat_map(Map::keys) {
                        facts.without(&written::preference_list(carrier, file.is_some_and(Value::is_array)).0);
                    }
                }
                outside.key_order.extend(facts.key_order);
                outside.with_point.extend(facts.with_point);
                outside.negative_zero.extend(facts.negative_zero);
            }
            if let Some(spacing) = self.dump.spacing(kind).and_then(WrittenSpacing::to_json) {
                spacings.insert(kind.name().to_string(), spacing);
            }
        }
        outside.stamp(&mut component);
        if !spacings.is_empty() {
            component.insert("spacing".into(), Value::Object(spacings));
        }
        // The places of the `pptl.json` groups that hold no device: a group is
        // rebuilt from its devices, and one with none has nothing else to say
        // it stood there.
        let empty_groups: Vec<usize> = self
            .dump
            .get(FileKind::Pptl)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
            .filter(|(_, group)| group.get("pptls").and_then(Value::as_array).is_some_and(Vec::is_empty))
            .map(|(at, _)| at)
            .collect();
        if !empty_groups.is_empty() {
            component.insert("pptl_empty_groups".into(), json!(empty_groups));
        }
        // An empty profile list `map.json` states, which no profile entity can
        // say: a floor object stating none and one stating an empty list are
        // two texts.
        let profiles =
            self.site.files.get("map.json").and_then(|map| map.get(0)).and_then(|floor| floor.get("location_profiles"));
        if profiles.and_then(Value::as_array).is_some_and(Vec::is_empty) {
            component.insert("location_profiles_empty".into(), json!(true));
        }
        // The conveyor file's own header, and the two files that are checked-in
        // HTTP request envelopes rather than models.
        if let Some(conveyor) = self.dump.get(FileKind::Conveyor).and_then(Value::as_object) {
            let mut header = Map::new();
            for key in ["version", "hai_port_types"] {
                if let Some(value) = conveyor.get(key) {
                    header.insert(key.to_string(), value.clone());
                }
            }
            if !header.is_empty() {
                component.insert("conveyor_header".into(), Value::Object(header));
            }
        }
        for (kind, target) in [(FileKind::PpsBin, "pps_bin_envelope"), (FileKind::PpsBinType, "pps_bin_type_envelope")]
        {
            if let Some(file) = self.dump.get(kind).and_then(Value::as_object) {
                let mut envelope = Map::new();
                for key in ["url", "type", "header"] {
                    if let Some(value) = file.get(key) {
                        envelope.insert(key.to_string(), value.clone());
                    }
                }
                if !envelope.is_empty() {
                    component.insert(target.to_string(), Value::Object(envelope));
                }
            }
        }
        self.push(
            id,
            Some("Map Creator export".to_string()),
            json!({
                "mapcreator.export": Value::Object(component),
                "external_ids": { "mapcreator_export": EXPORT_KEY }
            }),
        );
        Ok(())
    }

    // -----------------------------------------------------------------------
    // The account
    // -----------------------------------------------------------------------

    fn account_for_identity(&mut self, existing: &Existing) {
        self.ledger.identity.existing_entities = existing.len();
        self.ledger.identity.looked_up = self.resolver.looked_up;
        self.ledger.identity.fresh = self.resolver.minted;
        let unclaimed = self.resolver.unclaimed();
        self.ledger.identity.unclaimed_entities = unclaimed.len();
        self.ledger.identity.in_the_site_not_in_this_dump =
            unclaimed.iter().take(crate::ledger::INSTANCES).map(|id| existing.describe(&id.to_string())).collect();
        self.dangling.sort();
        self.dangling.dedup();
        self.ledger.identity.dangling_source_references = self.dangling.len();
        self.ledger.identity.dangling_instances =
            self.dangling.iter().take(crate::ledger::INSTANCES).cloned().collect();
    }

    /// Every field family the dump carries has a disposition, in the tool's table
    /// or in this adapter's overlay over it.
    fn account_for_coverage(&mut self) {
        let table = crate::format::coverage::table();
        let overlay = crate::coverage::overlay();
        for (path, count) in self.site.census() {
            if !table.contains_key(path.as_str()) && !overlay.contains_key(path.as_str()) {
                let times = if count == 1 { "once".to_string() } else { format!("{count} times") };
                self.ledger.uncovered.push(format!("{path} ({times})"));
            }
        }
    }
}

/// The straight-line fit a frame's `transform_to_parent` states, per axis, or
/// `None` for a transform of another shape.
///
/// The grid frame writes a two-element translation and a two-element scale, which
/// is a one-dimensional fit on each axis; anything else is a transform this
/// importer did not write, and the honest answer is not to read it as one.
/// The station a record names by its own `pps_id`, among whose parts a seat or
/// a bin it names is found.
/// The key one record kind's formatting rides under in `import_formatting`:
/// the lens its records are written back through.
fn lens_of(construct: &str) -> &'static str {
    records::kind(construct).map(|kind| kind.lens).expect("a record kind this adapter writes")
}

fn station_of(record: &Map<String, Value>) -> String {
    record.get("pps_id").map(scalar).unwrap_or_default()
}

/// The sentence refusing an export whose lengths no step, or not the site's
/// step, states exactly.
fn unkept_unit(entities: &Value, site_frame: Uuid, unkept: Unkept) -> Error {
    // The record a length is on, as a person finds it in the files.
    let on = |length: Length| {
        let entity = &entities[length.entity];
        match entity.pointer("/components/external_ids/mapcreator_grid_coordinate").and_then(Value::as_str) {
            Some(coordinate) => format!("map.json cell {coordinate}"),
            None => entity.get("name").and_then(Value::as_str).unwrap_or("an entity").to_string(),
        }
    };
    match unkept {
        Unkept::Unstated { count, worst, counted_mm } => Error::Import(format!(
            "{count} length(s) of the export land on no step this adapter counts in, 1, 0.5, 0.25 or 0.1 mm, so \
             the site could not state them without rounding: the worst is a length of {} mm on {}, which would be \
             counted as {counted_mm} mm",
            worst.mm,
            on(worst)
        )),
        Unkept::NoOneStep { one, step_mm, other } => Error::Import(format!(
            "the export states a length of {} mm on {}, which a step of {step_mm} mm states and a tenth of a \
             millimetre does not, and one of {} mm on {}, which a step of {step_mm} mm does not: no one step of 1, \
             0.5, 0.25 or 0.1 mm states both, and every length of a site is counted in one",
            one.mm,
            on(one),
            other.mm,
            on(other)
        )),
        Unkept::Finer { length, held_mm } => Error::Import(format!(
            "the export states a length of {} mm on {}, which the site's step of {held_mm} mm cannot state \
             exactly: every length in the site is counted in that step, other systems' lengths included, so the \
             adapter keeps it and refuses the export, which can go into a fresh site instead",
            length.mm,
            on(length)
        )),
        Unkept::Unknown { held_mm } => Error::Import(format!(
            "the site frame {site_frame} states `frame.unit_mm` {held_mm}, and this adapter counts lengths in steps \
             of 1, 0.5, 0.25 or 0.1 mm, each a whole number of them to a millimetre: in a step of {held_mm} mm no \
             length could be carried back into millimetres exactly"
        )),
    }
}

/// One entry of a list that named nothing, kept with its position in the list
/// the file states: the resolved entries of the list ride as references in
/// their own order, and the export writes this one back where it stood among
/// them.
fn placed(at: usize, value: &Value) -> Value {
    json!({ "at": at, "value": value })
}

fn read_fit(transform: &Value) -> Option<[Fit; 2]> {
    let axis = |key: &str, at: usize| -> Option<f64> { transform.get(key)?.as_array()?.get(at)?.as_f64() };
    let x = Fit { intercept: axis("translation", 0)?, slope: axis("scale", 0)? };
    let y = Fit { intercept: axis("translation", 1)?, slope: axis("scale", 1)? };
    if x.slope == 0.0 || y.slope == 0.0 {
        return None;
    }
    Some([x, y])
}

/// A fit read back off a site, carried from the steps of the site frame that
/// held it into millimetres: both the translation and the scale, because the
/// legacy grid frame's step is an index and its scale is a length per index.
fn in_millimetres(fit: [Fit; 2], unit_mm: f64) -> [Fit; 2] {
    let factor = steps::steps_per_mm(unit_mm);
    fit.map(|axis| Fit { intercept: axis.intercept / factor, slope: axis.slope / factor })
}

/// Whether two fits put every one of these cells in the same grid square.
///
/// Two transforms that agree on every cell say the same thing about the floor, so
/// the one already written down is as good as the new one — and it is the one no
/// consumer has to re-read.
///
/// The rounding is SPEC §5's one rule, `floor(v + 0.5)`, which is what
/// `Steps::round_half_up` does and so what the reader's own projection into this
/// frame does. Two implementations of one rounding rule would part company on a
/// half-boundary tie, which is precisely where a cell's square is in question.
fn projects_alike(cells: &[&Cell], left: &[Fit; 2], right: &[Fit; 2]) -> bool {
    let square = |fit: &[Fit; 2], cell: &Cell| -> [i64; 2] {
        [
            ((cell.world[0] - fit[0].intercept) / fit[0].slope + 0.5).floor() as i64,
            ((cell.world[1] - fit[1].intercept) / fit[1].slope + 0.5).floor() as i64,
        ]
    };
    cells.iter().all(|cell| square(left, cell) == square(right, cell))
}

/// Least squares through `(index, value)` pairs, or `None` when the index does
/// not vary.
fn fit(points: impl Iterator<Item = (f64, f64)> + Clone) -> Option<Fit> {
    let count = points.clone().count() as f64;
    if count < 2.0 {
        return None;
    }
    let mean_index = points.clone().map(|(i, _)| i).sum::<f64>() / count;
    let mean_value = points.clone().map(|(_, v)| v).sum::<f64>() / count;
    let covariance: f64 = points.clone().map(|(i, v)| (i - mean_index) * (v - mean_value)).sum();
    let variance: f64 = points.map(|(i, _)| (i - mean_index).powi(2)).sum();
    if variance == 0.0 || covariance == 0.0 {
        return None;
    }
    let slope = covariance / variance;
    Some(Fit { intercept: mean_value - slope * mean_index, slope })
}

/// A length rounded to whole millimetres, or `None` where it is not a finite
/// number an `i64` holds with its negation, so a cast never saturates.
fn whole_mm(mm: f64) -> Option<i64> {
    let rounded = mm.round();
    (rounded.is_finite() && rounded.abs() <= MAX_WHOLE as f64).then_some(rounded as i64)
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

/// `HTM_QT_X1` -> `htm`. The leading token is the role in every bot type any
/// dump carries.
pub fn role_of(bot_type: &str) -> String {
    bot_type.split('_').next().unwrap_or(bot_type).to_ascii_lowercase()
}

/// `VTM_HAI_Z9_E1` -> `hai`. The vendor tag sits inside the name and nowhere
/// else in the export.
pub fn manufacturer_of(bot_type: &str) -> Option<&'static str> {
    let upper = bot_type.to_ascii_uppercase();
    if upper.contains("HAI") {
        Some("hai")
    } else if upper.contains("QT") {
        Some("quicktron")
    } else {
        None
    }
}

/// The bot type with its role prefix removed, which is what is left to call a
/// model.
pub fn model_of(bot_type: &str) -> String {
    match bot_type.split_once('_') {
        Some((_, rest)) => rest.to_string(),
        None => bot_type.to_string(),
    }
}

/// SPEC §10's four station kinds, out of the export's product type and mode list.
pub fn station_kind(kind_source: &str, modes: &[String]) -> Option<&'static str> {
    let lowered = kind_source.to_ascii_lowercase();
    if lowered.contains("induction") || lowered.contains("induct") {
        return Some("induct");
    }
    if lowered.contains("extraction") || lowered.contains("extract") {
        return Some("extract");
    }
    // The distinct movement modes the list names: a mode named twice is one
    // mode, and no ambiguity between two kinds.
    let movement: BTreeSet<&str> =
        modes.iter().map(String::as_str).filter(|mode| *mode == "pick" || *mode == "put").collect();
    match movement.into_iter().collect::<Vec<_>>().as_slice() {
        ["pick"] => Some("pick"),
        ["put"] => Some("put"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{manufacturer_of, model_of, role_of, station_kind};

    #[test]
    fn a_bot_type_string_is_read_into_role_maker_and_model() {
        assert_eq!(role_of("HTM_QT_X1"), "htm");
        assert_eq!(role_of("VTM_HAI_Z9_E1"), "vtm");
        assert_eq!(manufacturer_of("HTM_QT_X1"), Some("quicktron"));
        assert_eq!(manufacturer_of("VTM_HAI_Z9_E1"), Some("hai"));
        assert_eq!(manufacturer_of("butler@vCleanZone"), None);
        assert_eq!(model_of("HTM_QT_X1"), "QT_X1");
    }

    #[test]
    fn a_station_kind_is_derived_only_where_the_export_decides_it() {
        let put = vec!["put".to_string(), "audit".to_string()];
        let both = vec!["put".to_string(), "pick".to_string(), "audit".to_string()];
        assert_eq!(station_kind("manual_tote_induction", &both), Some("induct"));
        assert_eq!(station_kind("tote_extraction", &both), Some("extract"));
        assert_eq!(station_kind("manual", &put), Some("put"));
        assert_eq!(station_kind("manual", &both), None);
        let twice = vec!["pick".to_string(), "pick".to_string()];
        assert_eq!(station_kind("manual", &twice), Some("pick"));
    }
}
