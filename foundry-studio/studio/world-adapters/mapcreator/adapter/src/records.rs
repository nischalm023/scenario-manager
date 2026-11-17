//! Within the Map Creator adapter, `records` is **the pairing table**: one row per
//! Map Creator record kind, and inside each row one entry per field of that
//! record saying where the field goes on the way in and where it comes from on
//! the way out.
//!
//! It exists so that the two directions cannot drift apart. An import written by
//! hand and an export written by hand agree until somebody edits one of them; a
//! single table read forwards by `import` and backwards by `serialize` has
//! nowhere to disagree. It is also the material §5 of the spec asks for: the
//! table of (record kind, lens, constructor, field map) from which a "put" law
//! could later be added to the lens language, if Daniel rules for one.
//!
//! Two rules give the table its shape. A field the core protocol already carries
//! is marked [`Slot::Elsewhere`] with the rule that owns it, so nothing is
//! carried twice — a cell's millimetre position is its `pose` and is not copied
//! into its `cell` beside the fields Map Creator adds there. A field that names another record by a legacy string —
//! a barcode, a grid coordinate, a station id — becomes an entity identity, and
//! the export rebuilds the legacy string from the identity, because a display
//! string is never load-bearing in a document (SPEC §2).

use crate::kinds::FileKind;

/// How the export spells a reference to a cell, which is how the import has to
/// read it and the export has to write it back.
///
/// Map Creator has three spellings for one cell and uses all three: `"[6,1]"` in
/// `map.json` and `tote_storage.json`, `[6, 1]` in `block.json` and `pps.json`
/// v3, and the floor sticker's barcode in `charger.json` and `pps.json` v2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spelling {
    /// `"[6,1]"` — the grid coordinate as a bracketed string.
    Bracketed,
    /// `[6, 1]` — the grid coordinate as a two-element array.
    RawPair,
    /// `"042.913"` — the cell's barcode.
    Barcode,
}

/// The JSON kind one carried field's value has, which is what a component's JSON
/// Schema states about it.
///
/// Read off the five dumps rather than guessed: every entry is the set of types
/// that field actually takes across all of them, which is why two of them are
/// unions — `botid` is the string `"null"` on three sites and JSON `null` on two,
/// and `hasStorables` is `0` or `null`. A union is a fact about the format, not a
/// tolerance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Text,
    Number,
    Flag,
    /// A JSON array. No element schema: the elements are the export's own nested
    /// shapes, and stating one would be inventing a rule Map Creator does not
    /// have.
    List,
    Object,
    /// A string or a JSON null, which are two different facts.
    TextOrNull,
    /// A number or a JSON null.
    NumberOrNull,
    /// A string or a number: an id one dialect states as one and another as the
    /// other.
    TextOrNumber,
    /// One entity identity — a reference, not a value.
    Reference,
    /// Entity identities in order.
    References,
}

impl Shape {
    /// The JSON Schema type (or types) this shape allows.
    pub fn json_types(self) -> &'static [&'static str] {
        match self {
            Shape::Text | Shape::Reference => &["string"],
            Shape::Number => &["number"],
            Shape::Flag => &["boolean"],
            Shape::List | Shape::References => &["array"],
            Shape::Object => &["object"],
            Shape::TextOrNull => &["string", "null"],
            Shape::NumberOrNull => &["number", "null"],
            Shape::TextOrNumber => &["string", "number"],
        }
    }
}

/// The JSON kind a legacy key is spelled in where the file states it, since
/// `external_ids` carries every one of them as a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyText {
    /// The file states it as a string: a zone name, a barcode, a TLOC id.
    Text,
    /// The file states it as a number: a charger id, a conveyor id, a v2 point's
    /// position id.
    Number,
}

/// A record that another record names by the first one's own key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Join {
    /// A station, by its `pps_id`.
    Station,
    /// A zone, by its `zone_id`.
    Zone,
    /// A location profile, by its `location_profile_id`.
    Profile,
    /// An operator seat, by its name among the seats of the naming record's own
    /// station.
    Seat,
    /// A put-wall bin, by its id among the bins of the naming record's own
    /// station.
    Bin,
    /// A bin geometry type, by its type id.
    BinType,
    /// A conveyor, by its `conveyor_id`.
    Conveyor,
    /// A block, by its `block_id`.
    Block,
}

impl Join {
    /// The `external_ids` field the named record's own key is under, which the
    /// export writes the reference back from.
    pub fn key_field(self) -> &'static str {
        match self {
            Join::Station => "mapcreator_pps_id",
            Join::Zone => "mapcreator_zone_id",
            Join::Profile => "mapcreator_location_profile_id",
            Join::Seat => "mapcreator_seat_name",
            Join::Bin => "mapcreator_bin_id",
            Join::BinType => "mapcreator_bin_type_id",
            Join::Conveyor => "mapcreator_conveyor_id",
            Join::Block => "mapcreator_block_id",
        }
    }

    /// The component the named record carries, which the server is told a
    /// reference to it expects (`vocabulary`).
    pub fn component(self) -> &'static str {
        match self {
            Join::Station => "mapcreator.station",
            Join::Zone => "zone",
            Join::Profile => "location_profile",
            Join::Seat => "pps_seat",
            Join::Bin => "pps_bin",
            Join::BinType => "pps_bin_type",
            Join::Conveyor => "conveyor",
            Join::Block => "block",
        }
    }

    /// What the named record is, for a sentence: "the station".
    pub fn what(self) -> &'static str {
        match self {
            Join::Station => "station",
            Join::Zone => "zone",
            Join::Profile => "location profile",
            Join::Seat => "operator seat",
            Join::Bin => "put-wall bin",
            Join::BinType => "bin geometry type",
            Join::Conveyor => "conveyor",
            Join::Block => "block",
        }
    }
}

/// Where one source field travels.
#[derive(Debug, Clone, Copy)]
pub enum Slot {
    /// The record's own legacy key. It rides in `external_ids`, which is where
    /// SPEC §2 puts the identifiers other systems know a thing by — as data,
    /// joinable, never identity and never a reference target — and the export
    /// writes it back in the JSON kind its file spells.
    Key { external: &'static str, text: KeyText },
    /// Copied as written into the component under the target name, with the JSON
    /// kind the dumps show it takes. The export copies it back.
    Verbatim(Shape),
    /// Live state — which robot is standing there, whether a zone is paused.
    /// Copied as written under the component's `runtime` sub-object, and marked
    /// in the ledger, so a later policy can drop it deliberately. SPEC §1
    /// separates a world from what is happening in it; this adapter's job is to
    /// round-trip, so nothing is dropped now (spec §4).
    Runtime(Shape),
    /// How the file wrote the record rather than anything about the site: a
    /// station's `version`, or its `pps_id` in the JSON kind its dialect
    /// spells it in while `external_ids` holds the key itself. Copied as
    /// written into the record's `import_formatting` entry, under the target
    /// name, and written back from there.
    Written(Shape),
    /// The value names one cell. The component holds that cell's identity;
    /// where the value names no cell of this dump the component holds the value
    /// verbatim under `<target>_unresolved` and the ledger counts it as a
    /// dangling source reference.
    Cell(Spelling),
    /// The value is a list naming cells, in order.
    Cells(Spelling),
    /// The value names one record of another kind by that record's own key,
    /// spelled in the JSON kind this field's file fixes. The component holds
    /// the record's identity and the export writes the key back from the
    /// named record; a value that names nothing, or is not spelled as the
    /// export would write it back, is kept verbatim under `<target>_unresolved`.
    Join(Join, KeyText),
    /// The value is a list naming blocks by their `block_id`, in order.
    Blocks,
    /// The same value as another source field of the same record, in every row of
    /// every dump seen. The export writes it from that field, and the component
    /// carries it verbatim under `<target>` **only** where the two differ — so a
    /// dump where they part company round-trips, and a dump where they agree does
    /// not carry the value twice.
    Echo { of: &'static str },
    /// An object whose own fields travel by this map.
    Nested(&'static [Field]),
    /// Not carried here: another rule owns the fact, and this names it. The
    /// export rebuilds the field from there.
    Elsewhere(&'static str),
}

/// One field of one record kind.
#[derive(Debug, Clone, Copy)]
pub struct Field {
    /// The key the export file spells.
    pub source: &'static str,
    /// The key the component spells. Snake case throughout, so
    /// `hasStorables` arrives as `has_storables` and nothing downstream has to
    /// know that Map Creator mixes two conventions.
    pub target: &'static str,
    pub slot: Slot,
}

const fn key(source: &'static str, external: &'static str, text: KeyText) -> Field {
    Field { source, target: "", slot: Slot::Key { external, text } }
}

const fn v(source: &'static str, target: &'static str, shape: Shape) -> Field {
    Field { source, target, slot: Slot::Verbatim(shape) }
}

const fn r(source: &'static str, target: &'static str, shape: Shape) -> Field {
    Field { source, target, slot: Slot::Runtime(shape) }
}

const fn w(source: &'static str, target: &'static str, shape: Shape) -> Field {
    Field { source, target, slot: Slot::Written(shape) }
}

const fn e(source: &'static str, owner: &'static str) -> Field {
    Field { source, target: "", slot: Slot::Elsewhere(owner) }
}

const fn cell(source: &'static str, target: &'static str, spelling: Spelling) -> Field {
    Field { source, target, slot: Slot::Cell(spelling) }
}

const fn cells(source: &'static str, target: &'static str, spelling: Spelling) -> Field {
    Field { source, target, slot: Slot::Cells(spelling) }
}

const fn join(source: &'static str, target: &'static str, join: Join, text: KeyText) -> Field {
    Field { source, target, slot: Slot::Join(join, text) }
}

/// One Map Creator record kind: what it becomes, and the field map that takes it
/// there and back.
#[derive(Debug, Clone, Copy)]
pub struct RecordKind {
    /// The construct's name, which is also its lookup-key row in
    /// `identity::SHAPES` and its row in `DECISIONS.md`.
    pub construct: &'static str,
    /// The component the record's own fields land in: a core component the
    /// protocol defines, where Map Creator's fields are its optional ones
    /// (`cell`, `presentation_point`); shared vocabulary where the name is
    /// otherwise undotted; and Map Creator's own `mapcreator.station` where the
    /// merge is not obvious (`vocabulary` says which).
    pub component: &'static str,
    /// The file the records come from and go back to.
    pub file: FileKind,
    /// The lens whose objects are these records, in `patterns/mapcreator.lens`.
    pub lens: &'static str,
    /// The field map a record of this kind travels by.
    pub fields: &'static [Field],
    /// Further field maps the same component spans, because the file states one
    /// record kind two ways.
    ///
    /// Only the station has any: `pps.json` has two dialects and they are **one
    /// component**, not two. A v2 record and a v3 record are the same station
    /// written twice — `pps_id` is a number in v3 and the string `"1"` in v2,
    /// `dialect` a string in v3 and a number in v2, and six fields belong to v2
    /// alone — so the component's schema is the union of both tables and a field
    /// both dialects carry accepts either JSON kind. A schema derived from v3
    /// alone refuses site-c's 20 and site-e's 10 stations at a server's
    /// door, which is what this field exists to prevent.
    pub dialects: &'static [&'static [Field]],
}

// ---------------------------------------------------------------------------
// map.json — the spine
// ---------------------------------------------------------------------------

/// One cell. The core takes the position, the footprint, the traversability and
/// the memberships; what is left rides on the same `cell` component, as the
/// optional fields the protocol states for a surveyed cell.
pub const CELL: &[Field] = &[
    key("coordinate", "mapcreator_grid_coordinate", KeyText::Text),
    key("barcode", "barcode", KeyText::Text),
    e("world_coordinate", "the cell's pose in the floor frame"),
    e("vda_world_coordinate", "external_ids.vda_world_coordinate, beside the declared vendor frame"),
    e("blocked", "cell.blocked"),
    e("sector", "membership of the sector entity, whose own key states the spelling"),
    e("block", "membership of the block entity's queue.path"),
    e("dockPoint", "the dock point entity that names this cell"),
    // The profile is an entity, so the cell references it and the export rebuilds
    // the legacy id from the referenced entity's own key — the same rule the zone
    // follows, and the reason neither string is a join key in a document (SPEC §2).
    // `cell.traversable_by` is read off the same profile and is the flat, lossy
    // form of it.
    join("location_profile_id", "location_profile", Join::Profile, KeyText::Text),
    // The four half-extents, kept whole. `cell.size` is their per-axis sum — a
    // derived form — so the original stays here and the export writes from it
    // (spec §4).
    v("size_info", "size_info", Shape::List),
    // The adjacency and its grades, kept whole for the same reason: the stitch
    // entities are the subset both ends permit travel across, which is derived
    // from these two and cannot state the rest.
    v("adjacency", "adjacency", Shape::List),
    v("neighbours", "neighbours", Shape::List),
    // The zone string is the join key for a whole e-stop domain — on one site
    // the largest is named with a typo and a trailing space, for 925 cells, the
    // trailing space load-bearing. The cell references the zone entity instead, and the
    // export rebuilds the string from it (SPEC §2).
    join("zone", "zone", Join::Zone, KeyText::Text),
    v("store_status", "store_status", Shape::Number),
    v("special", "special", Shape::Flag),
    v("grid_attribute", "grid_attribute", Shape::Text),
    v("composite_attribute_list", "composite_attribute_list", Shape::List),
    v("hasStorables", "has_storables", Shape::NumberOrNull),
    v("conveyor_direction_attribute", "conveyor_direction_attribute", Shape::Number),
    v("barcode_properties", "barcode_properties", Shape::Object),
    v("world_coordinate_reference_neighbour", "world_coordinate_reference_neighbour", Shape::Text),
    v("floor", "floor", Shape::Number),
    r("botid", "botid", Shape::TextOrNull),
    r("block_reasons", "block_reasons", Shape::List),
];

/// One named bundle of per-bot-type permissions.
pub const LOCATION_PROFILE: &[Field] = &[
    key("location_profile_id", "mapcreator_location_profile_id", KeyText::Text),
    v("bot_orientation_details", "bot_orientation_details", Shape::Object),
    v("edge_height_clearance", "edge_height_clearance", Shape::Object),
];

// ---------------------------------------------------------------------------
// The flat record files
// ---------------------------------------------------------------------------

/// One zone: an e-stop and light-curtain domain. `zonerec` is unwrapped on the
/// way in and re-wrapped on the way out — a record envelope is not a fact.
pub const ZONE: &[Field] = &[
    key("zone_id", "mapcreator_zone_id", KeyText::Text),
    r("blocked", "blocked", Shape::Flag),
    r("paused", "paused", Shape::Flag),
];

/// One aisle. Its io-point cells are the sector's members, which is the core's
/// own shapeless named grouping.
pub const AISLE: &[Field] = &[
    key("aisle_id", "mapcreator_aisle_id", KeyText::Text),
    e("io_point_coordinates", "sector.members, in the order the file states them"),
    v("display_name", "display_name", Shape::Text),
    v("additional_information", "additional_information", Shape::Object),
];

/// The block kinds whose cells are a core `queue`'s path. Every other block
/// keeps its cells on `block.cells`.
pub const QUEUE_KINDS: [&str; 2] = ["queue", "sequenced_queue"];

/// The block kinds that are a core `waiting` area.
pub const WAITING_KINDS: [&str; 2] = ["waiting", "waiting_block"];

/// One block: a named cell group butler uses as a queue or a waiting area.
pub const BLOCK: &[Field] = &[
    key("block_id", "mapcreator_block_id", KeyText::Text),
    e("coordinates", "queue.path for a queue block, block.cells for a waiting one"),
    v("extra_details", "extra_details", Shape::Object),
];

/// One charger. Three of its fields name cells by barcode, and all three become
/// identities.
pub const CHARGER: &[Field] = &[
    key("charger_id", "mapcreator_charger_id", KeyText::Number),
    cell("charger_location", "at_cell", Spelling::Barcode),
    cell("entry_point_location", "entry_cell", Spelling::Barcode),
    cell("reinit_point_location", "reinit_cell", Spelling::Barcode),
    v("charger_direction", "at_direction", Shape::Number),
    v("entry_point_direction", "entry_direction", Shape::Number),
    v("reinit_point_direction", "reinit_direction", Shape::Number),
    v("charger_type", "charger_type", Shape::Text),
    v("mode", "mode", Shape::Text),
    r("status", "status", Shape::Text),
    r("charge", "charge", Shape::Number),
    r("attached_butler_id", "attached_butler_id", Shape::Text),
    r("error_reason", "error_reason", Shape::Text),
    r("blocked_by_butler_ids", "blocked_by_butler_ids", Shape::List),
    r("power_status", "power_status", Shape::Text),
];

/// One docking point: a fixed floor position where a trolley is parked at a
/// station.
pub const DOCK_POINT: &[Field] = &[
    key("id", "mapcreator_dock_point_id", KeyText::Text),
    // The same string as `id`, byte for byte in every row of every dump seen — so
    // it is written back from the one key rather than stored twice. `Echo` is what
    // makes that safe rather than assumed: a row where the two differ carries the
    // barcode verbatim and comes back out differing.
    Field { source: "dock_barcode", target: "dock_barcode", slot: Slot::Echo { of: "id" } },
    cell("dock_coordinate", "at_cell", Spelling::Bracketed),
    v("direction", "direction", Shape::Number),
    v("display_name", "display_name", Shape::Text),
    // Not a `pps_id`: every one of site-c's 53 rows names a
    // `pps.docking_stations[].dock_station_id` — a bay, which is a sub-object of
    // a station and not an entity, because nothing in the export gives a bay a
    // place on the floor. So there is no identity to reference yet and the
    // legacy string stays.
    v("station_id", "docking_station_id", Shape::Text),
    v("station_type", "station_type", Shape::Text),
    r("mtu_id", "mtu_id", Shape::Text),
    r("rack_id", "rack_id", Shape::Text),
    r("rack_status", "rack_status", Shape::Text),
    r("reserved_by", "reserved_by", Shape::Text),
    r("status", "status", Shape::Text),
    r("station_point_status", "station_point_status", Shape::Text),
];

/// One obstacle-detection exclusion: a (cell, heading) pair where a robot's
/// obstacle sensor is suppressed because the charger fixture there would read as
/// an obstacle.
pub const ODS_EXCLUSION: &[Field] = &[
    e(
        "ods_tuple",
        "external_ids.mapcreator_ods_tuple, the exclusion's lookup key, and the cell and heading it splits into",
    ),
    v("excluded", "excluded", Shape::Flag),
];

/// One storage position, as a flat tuple with no rack behind it.
pub const TOTE_STORAGE: &[Field] = &[
    key("location", "mapcreator_tote_location", KeyText::Text),
    Field {
        source: "io_point",
        target: "io_point",
        slot: Slot::Nested(&[
            cell("coordinate", "cell", Spelling::Bracketed),
            v("bot_orientation", "bot_orientation", Shape::Number),
        ]),
    },
    cell("relay_point", "relay_cell", Spelling::Bracketed),
    v("storage_direction", "storage_direction", Shape::Number),
    v("ndeep", "ndeep", Shape::Number),
    v("height", "height", Shape::Number),
    v("storage_sector", "storage_sector", Shape::Number),
];

/// One rack and carrier geometry template. Nothing in the export places one
/// anywhere, so the template's own tree has no parts to be entities of and rides
/// whole.
pub const RACK_TYPE: &[Field] = &[
    key("racktype_id", "mapcreator_racktype_id", KeyText::Text),
    v("rack_width", "rack_width", Shape::Number),
    v("rack_depth", "rack_depth", Shape::Number),
    v("rack_outer_width", "rack_outer_width", Shape::Number),
    v("rack_outer_depth", "rack_outer_depth", Shape::Number),
    v("inter_leg_width", "inter_leg_width", Shape::Number),
    v("inter_leg_depth", "inter_leg_depth", Shape::Number),
    v("initial_mass_of_the_rack", "initial_mass_of_the_rack", Shape::Number),
    v("initial_cm_of_the_rack", "initial_cm_of_the_rack", Shape::List),
    v("density_profile", "density_profile", Shape::List),
    v("replenishable", "replenishable", Shape::Text),
    v("floor_structure", "floor_structure", Shape::List),
];

/// One conveyor: an ordered chain of steps with entry and exit ports.
pub const CONVEYOR: &[Field] = &[
    key("conveyor_id", "mapcreator_conveyor_id", KeyText::Number),
    v("display_name", "display_name", Shape::Text),
    v("connected_conveyors", "connected_conveyors", Shape::List),
    v("conveyor_steps", "steps", Shape::List),
    v("entry_points", "entry_points", Shape::List),
    v("exit_points", "exit_points", Shape::List),
];

/// One row of the three-frame join table. Its lookup key is the conveyor and
/// the step it names, two values in `external_ids`: `mapcreator_of_conveyor`
/// and `mapcreator_step_id`. The conveyor is also a reference, and the step,
/// a handle into the conveyor's kept `steps`, is the key alone.
pub const CONVEYOR_MAPPING: &[Field] = &[
    Field {
        source: "conveyor_coordinates",
        target: "conveyor_coordinates",
        slot: Slot::Nested(&[
            join("conveyor_id", "conveyor", Join::Conveyor, KeyText::Text),
            key("step_id", "mapcreator_step_id", KeyText::Text),
        ]),
    },
    cell("go_coordinates", "cell", Spelling::RawPair),
    v("HAI_coordinates", "vendor_coordinates", Shape::Text),
    v("location_type", "location_type", Shape::Text),
    join("pps_id", "station", Join::Station, KeyText::Number),
];

/// One put-wall bin at a station.
///
/// Its `pps_id` is two things at once, as a mapping row's `conveyor_id` is: a
/// reference to the station, and half of the key the bin is known by, which
/// `import::flat_key` reads off the file into `external_ids.mapcreator_of_pps`.
/// The file gets the reference. It is what the site states about the bin and
/// what a tool edits, and the key only finds the bin's entity again: once a
/// tool re-points the reference, the next ingest of the file this writes finds
/// no entity under the new key, declares the bin afresh and deletes the old
/// one, which is what an edit to a key costs (`DECISIONS.md`).
pub const PPS_BIN: &[Field] = &[
    key("bin_id", "mapcreator_bin_id", KeyText::Text),
    join("pps_id", "station", Join::Station, KeyText::Text),
    v("position", "wall_position", Shape::List),
    v("coordinate", "station_local_coordinate", Shape::List),
    v("direction", "direction", Shape::Text),
    v("group_id", "group_id", Shape::Text),
    join("type", "bin_type", Join::BinType, KeyText::Text),
    v("allowed_tags", "allowed_tags", Shape::List),
    join("front_seat_name", "front_seat", Join::Seat, KeyText::Text),
    join("back_seat_name", "back_seat", Join::Seat, KeyText::Text),
    r("status", "status", Shape::Text),
];

/// One bin geometry type.
pub const PPS_BIN_TYPE: &[Field] = &[
    key("type_id", "mapcreator_bin_type_id", KeyText::Text),
    v("tag", "tag", Shape::List),
    v("dimension", "dimension", Shape::Object),
];

/// One operator seat at a station. Its `pps_id` is a reference and half of its
/// key at once, and the file gets the reference, as for a bin.
pub const PPS_SEAT: &[Field] = &[
    key("seat_name", "mapcreator_seat_name", KeyText::Text),
    join("pps_id", "station", Join::Station, KeyText::Text),
    v("seat_type", "seat_type", Shape::Text),
    v("seat_data", "seat_data", Shape::Object),
];

/// One put-to-light module, bound one to one to a bin.
pub const PPTL: &[Field] = &[
    key("peripheral_id", "mapcreator_peripheral_id", KeyText::Text),
    e("$group", "import_formatting.mapcreator_pptl.group, the device's group in the file, which no field of a record determines"),
    v("peripheral_type", "peripheral_type", Shape::Text),
    v("barcode", "barcode", Shape::Text),
    join("pps_bin_id", "bin", Join::Bin, KeyText::Text),
    join("pps_id", "station", Join::Station, KeyText::Text),
];

// ---------------------------------------------------------------------------
// pps.json — two dialects under one component
// ---------------------------------------------------------------------------

/// One station in the v3 dialect: `location` is a raw grid pair and the parts
/// hang under `entities`.
pub const STATION_V3: &[Field] = &[
    // The key itself is `external_ids.mapcreator_pps_id`, as a string; the JSON
    // kind the file spells it in — an integer in v3, the string "1" on site-c
    // and the integer 1 on site-e in v2 — is how the file wrote it, and the
    // export writes back what it read.
    w("pps_id", "pps_id", Shape::Number),
    e("type", "station.role, the product identity"),
    w("version", "dialect", Shape::Text),
    cell("location", "at_cell", Spelling::RawPair),
    v("allowed_modes", "allowed_modes", Shape::List),
    v("orchestrator", "orchestrator", Shape::Text),
    v("system", "system", Shape::List),
    Field { source: "blocks", target: "blocks", slot: Slot::Blocks },
    e("entities", "the station's parts: its queues as blocks, its presentation points as entities of their own"),
    v("associations", "associations", Shape::List),
    r("status", "status", Shape::Text),
    r("pps_url", "url", Shape::Text),
];

/// One station in the v2 dialect: `location` is a barcode, the points are
/// `pps_points`, and the queue is a bare unordered list of barcodes.
pub const STATION_V2: &[Field] = &[
    w("pps_id", "pps_id", Shape::TextOrNumber),
    e("type", "station.role, the product identity"),
    w("version", "dialect", Shape::Number),
    cell("location", "at_cell", Spelling::Barcode),
    v("allowed_modes", "allowed_modes", Shape::List),
    v("eligible_system", "eligible_system", Shape::List),
    e("pps_points", "one presentation_point entity each"),
    cells("queue_barcodes", "queue_cells", Spelling::Barcode),
    cell("pick_position", "pick_cell", Spelling::Barcode),
    v("pick_direction", "pick_direction", Shape::Number),
    v("put_docking_positions", "put_docking_positions", Shape::List),
    v("docking_stations", "docking_stations", Shape::List),
    r("status", "status", Shape::Text),
    r("pps_url", "url", Shape::Text),
];

/// One v3 presentation point.
pub const PRESENTATION_POINT: &[Field] = &[
    key("point_id", "mapcreator_point_id", KeyText::Text),
    e("$station", "presentation_point.station, which the import writes and pps.json does not state: a point sits inside its station's own record"),
    e("presented_for", "presentation_point.for"),
    v("light_curtain", "light_curtain", Shape::Flag),
    v("presented_entity_allowed_types", "presented_entity_allowed_types", Shape::List),
    v("presenting_agent_type", "presenting_agent_type", Shape::Text),
    v("presenting_agent_id", "presenting_agent_id", Shape::Text),
    Field {
        source: "attributes",
        target: "attributes",
        slot: Slot::Nested(&[
            cell("barcode", "cell", Spelling::Barcode),
            e(
                "world_coordinates",
                "the cell the barcode names, whose own grid coordinate this repeats; kept verbatim as \
                 presentation_point.world_coordinates_unresolved where the barcode names no cell, and \
                 its absence listed in the point's import_formatting as absent",
            ),
            v("display_attributes", "display_attributes", Shape::Object),
            v("pick_direction", "pick_direction", Shape::Number),
            v("presentation_height", "presentation_height", Shape::Number),
            v("bot_type", "bot_type", Shape::Text),
            v("bot_orientation", "bot_orientation", Shape::Number),
            v("is_workbench", "is_workbench", Shape::Flag),
        ]),
    },
];

/// One v2 station point, which the older dialect calls a `pps_point`.
pub const STATION_POINT: &[Field] = &[
    key("position_id", "mapcreator_position_id", KeyText::Number),
    e("$station", "station_point.station, which the import writes and pps.json does not state: a point sits inside its station's own record"),
    cell("coordinate", "cell", Spelling::Bracketed),
    v("type", "point_type", Shape::Text),
];

// ---------------------------------------------------------------------------
// The table
// ---------------------------------------------------------------------------

/// Every record kind, in the order the files are written.
pub const KINDS: &[RecordKind] = &[
    RecordKind {
        construct: "zone",
        component: "zone",
        file: FileKind::Zone,
        lens: "mapcreator_zone",
        fields: ZONE,
        dialects: &[],
    },
    RecordKind {
        construct: "cell",
        component: "cell",
        file: FileKind::Map,
        lens: "mapcreator_cell",
        fields: CELL,
        dialects: &[],
    },
    RecordKind {
        construct: "location_profile",
        component: "location_profile",
        file: FileKind::Map,
        lens: "mapcreator_location_profile",
        fields: LOCATION_PROFILE,
        dialects: &[],
    },
    RecordKind {
        construct: "block",
        component: "block",
        file: FileKind::Block,
        lens: "mapcreator_block",
        fields: BLOCK,
        dialects: &[],
    },
    RecordKind {
        construct: "station",
        component: "mapcreator.station",
        file: FileKind::Pps,
        lens: "mapcreator_station",
        fields: STATION_V3,
        dialects: &[STATION_V2],
    },
    RecordKind {
        construct: "presentation_point",
        component: "presentation_point",
        file: FileKind::Pps,
        lens: "mapcreator_presentation_point",
        fields: PRESENTATION_POINT,
        dialects: &[],
    },
    RecordKind {
        construct: "station_point",
        component: "station_point",
        file: FileKind::Pps,
        lens: "mapcreator_station_point",
        fields: STATION_POINT,
        dialects: &[],
    },
    RecordKind {
        construct: "tote_storage",
        component: "tote_storage",
        file: FileKind::ToteStorage,
        lens: "mapcreator_tote_storage",
        fields: TOTE_STORAGE,
        dialects: &[],
    },
    RecordKind {
        construct: "rack_type",
        component: "rack_type",
        file: FileKind::ToteType,
        lens: "mapcreator_rack_type",
        fields: RACK_TYPE,
        dialects: &[],
    },
    RecordKind {
        construct: "conveyor",
        component: "conveyor",
        file: FileKind::Conveyor,
        lens: "mapcreator_conveyor",
        fields: CONVEYOR,
        dialects: &[],
    },
    RecordKind {
        construct: "conveyor_mapping",
        component: "conveyor_mapping",
        file: FileKind::ConveyorMapping,
        lens: "mapcreator_conveyor_mapping",
        fields: CONVEYOR_MAPPING,
        dialects: &[],
    },
    RecordKind {
        construct: "charger",
        component: "charger",
        file: FileKind::Charger,
        lens: "mapcreator_charger",
        fields: CHARGER,
        dialects: &[],
    },
    RecordKind {
        construct: "aisle",
        component: "aisle",
        file: FileKind::VtmZone,
        lens: "mapcreator_aisle",
        fields: AISLE,
        dialects: &[],
    },
    RecordKind {
        construct: "dock_point",
        component: "dock_point",
        file: FileKind::DockPoint,
        lens: "mapcreator_dock_point",
        fields: DOCK_POINT,
        dialects: &[],
    },
    RecordKind {
        construct: "ods_exclusion",
        component: "ods_exclusion",
        file: FileKind::OdsExcluded,
        lens: "mapcreator_ods_exclusion",
        fields: ODS_EXCLUSION,
        dialects: &[],
    },
    RecordKind {
        construct: "pps_bin",
        component: "pps_bin",
        file: FileKind::PpsBin,
        lens: "mapcreator_pps_bin",
        fields: PPS_BIN,
        dialects: &[],
    },
    RecordKind {
        construct: "pps_bin_type",
        component: "pps_bin_type",
        file: FileKind::PpsBinType,
        lens: "mapcreator_pps_bin_type",
        fields: PPS_BIN_TYPE,
        dialects: &[],
    },
    RecordKind {
        construct: "pps_seat",
        component: "pps_seat",
        file: FileKind::PpsSeat,
        lens: "mapcreator_pps_seat",
        fields: PPS_SEAT,
        dialects: &[],
    },
    RecordKind {
        construct: "pptl",
        component: "pptl",
        file: FileKind::Pptl,
        lens: "mapcreator_pptl",
        fields: PPTL,
        dialects: &[],
    },
];

/// The record kind one construct names.
pub fn kind(construct: &str) -> Option<&'static RecordKind> {
    KINDS.iter().find(|kind| kind.construct == construct)
}

/// The field map one construct's records travel by.
pub fn fields(construct: &str) -> Option<&'static [Field]> {
    kind(construct).map(|kind| kind.fields)
}

/// The entry for one source key of one field map.
pub fn field<'a>(fields: &'a [Field], source: &str) -> Option<&'a Field> {
    fields.iter().find(|field| field.source == source)
}

#[cfg(test)]
mod tests {
    use super::{Field, Slot, KINDS, STATION_V2, STATION_V3};
    use crate::identity::shape;
    use std::collections::BTreeSet;

    /// Every record kind's construct must have a lookup key, or the import has
    /// records it cannot give identities to.
    #[test]
    fn every_record_kind_has_a_lookup_key() {
        for kind in KINDS {
            assert!(shape(kind.construct).is_some(), "no lookup key for construct `{}`", kind.construct);
        }
    }

    /// One construct, one component, one lens. A component under two constructs
    /// would put two record kinds' fields in one place and the export could not
    /// tell them apart.
    #[test]
    fn constructs_components_and_lenses_are_one_to_one() {
        let mut constructs = BTreeSet::new();
        let mut components = BTreeSet::new();
        let mut lenses = BTreeSet::new();
        for kind in KINDS {
            assert!(constructs.insert(kind.construct), "`{}` twice", kind.construct);
            assert!(components.insert(kind.component), "`{}` twice", kind.component);
            assert!(lenses.insert(kind.lens), "`{}` twice", kind.lens);
            assert!(kind.lens.starts_with("mapcreator_"), "`{}` is not a mapcreator lens name", kind.lens);
        }
    }

    fn walk(fields: &'static [Field], each: &mut impl FnMut(&'static Field)) {
        for field in fields {
            each(field);
            if let Slot::Nested(inner) = field.slot {
                walk(inner, each);
            }
        }
    }

    /// A carried field needs a target name to be carried under, and an
    /// `Elsewhere` field needs the rule that owns it written out. Both halves
    /// are the table's whole value: a blank on either side is a field nobody can
    /// follow.
    #[test]
    fn every_field_either_names_a_target_or_names_its_owner() {
        for kind in KINDS {
            walk(kind.fields, &mut |field| match field.slot {
                Slot::Elsewhere(owner) => {
                    assert!(!owner.is_empty(), "{} {} says nothing about where it went", kind.construct, field.source);
                    assert!(
                        field.target.is_empty(),
                        "{} {} is elsewhere and still names a target",
                        kind.construct,
                        field.source
                    );
                }
                // A key's home is `external_ids`, under the name the slot carries,
                // so it has no component field of its own to name.
                Slot::Key { external, .. } => {
                    assert!(!external.is_empty(), "{} {} is a key with no external id", kind.construct, field.source);
                    assert!(
                        field.target.is_empty(),
                        "{} {} is a key and still names a target",
                        kind.construct,
                        field.source
                    );
                }
                _ => assert!(!field.target.is_empty(), "{} {} has no target name", kind.construct, field.source),
            });
        }
    }

    /// No source key twice in one map, in the same object: the import looks a
    /// key up by name and a second row would never be reached.
    #[test]
    fn no_source_key_appears_twice_in_one_object() {
        for kind in KINDS {
            let mut seen = BTreeSet::new();
            for field in kind.fields {
                assert!(seen.insert(field.source), "{} states `{}` twice", kind.construct, field.source);
            }
        }
    }

    /// The two station dialects are one component, so a key they both carry has
    /// to land in the same place under both — otherwise a v2 site and a v3 site
    /// would write two different documents for the same fact.
    #[test]
    fn the_two_station_dialects_agree_wherever_they_overlap() {
        for a in STATION_V3 {
            if let Some(b) = STATION_V2.iter().find(|b| b.source == a.source) {
                assert_eq!(a.target, b.target, "`{}` lands in two places", a.source);
            }
        }
    }

    /// Target names are snake case, because Map Creator mixes two conventions
    /// (`hasStorables` beside `store_status`) and a document should not.
    #[test]
    fn target_names_are_snake_case() {
        for kind in KINDS {
            walk(kind.fields, &mut |field| {
                assert!(
                    field.target.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                    "`{}` of {} is not snake case",
                    field.target,
                    kind.construct
                );
            });
        }
    }
}
