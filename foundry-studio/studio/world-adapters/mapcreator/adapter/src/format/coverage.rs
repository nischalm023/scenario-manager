//! Within `format`, `coverage` is **the disposition of every field the
//! export has**: one row per key path, saying whether the import carries it as
//! structure, carries it as data, drops it as runtime state, drops it as
//! redundant, or cannot carry it at all.
//!
//! It exists to make "nothing was dropped silently" a checkable claim rather than
//! a promise. `source`'s census counts every key path a dump actually contains;
//! this table says what happens to each one; and a path present in a dump with no
//! row here fails the run. That is the only mechanism keeping the ledger honest
//! as new site dumps arrive with fields nobody has seen — a field that appears
//! for the first time cannot slip past unmentioned, because the program does not
//! know what to say about it and says so.
//!
//! The **reading** column is the other half. `research/butler-representation.md`
//! separates what butler actually consumes from what Map Creator emits and
//! nothing reads, and the two are not the same severity: a field the protocol
//! cannot carry that butler keys on is a hole in the protocol, while one butler
//! never reads is a field the export should probably stop writing.

use std::collections::BTreeMap;

/// What the import does with a field family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    /// A nesting key with no content of its own.
    Container,
    /// Carried as structure: it becomes an entity, or a field of a component
    /// whose meaning SPEC §10 fixes.
    Structure,
    /// Carried as data: it rides in `external_ids`, joinable and indexable, never
    /// identity and never a reference target (SPEC §2).
    Data,
    /// Carried for some records and impedance for others; the ledger's mismatch
    /// list says which, and how many.
    Partial,
    /// The same fact, already carried from somewhere else in the export.
    Redundant,
    /// Live state. SPEC §1 separates a world from what is happening in it, so
    /// this is dropped by design rather than for want of a component.
    Runtime,
    /// The protocol has nowhere to put it. Never resolved here — logged, with a
    /// count and an example, as an input to the next spec conversation.
    Impedance,
}

/// Whether butler consumes the field, per `research/butler-representation.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reading {
    /// Butler reads it, keys on it, or validates against it.
    LoadBearing,
    /// Map Creator writes it and butler never reads it.
    EmittedOnly,
    /// The question does not arise: the field is carried, or it is a container.
    NotApplicable,
}

/// One field family's disposition.
#[derive(Debug, Clone, Copy)]
pub struct Field {
    pub path: &'static str,
    pub disposition: Disposition,
    pub reading: Reading,
    pub note: &'static str,
}

use Disposition::{Container, Data, Impedance, Partial, Redundant, Runtime, Structure};
use Reading::{EmittedOnly, LoadBearing, NotApplicable};

const fn f(path: &'static str, disposition: Disposition, reading: Reading, note: &'static str) -> Field {
    Field { path, disposition, reading, note }
}

/// Every field family the five analysed dumps contain, and what becomes of it.
///
/// Ordered by file, then by path, so a reader can walk one file's fate at a time.
pub const FIELDS: &[Field] = &[
    // ---- map.json: the spine -------------------------------------------------
    f("map.json", Container, NotApplicable, "the floor: cells, bot-capability profiles, and the VDA anchor"),
    f("map.json:[].floor_id", Structure, NotApplicable, "names the floor frame this document declares"),
    f("map.json:[].map_values", Container, NotApplicable, "the cell list"),
    f("map.json:[].map_values[].coordinate", Data, NotApplicable, "external_ids.mapcreator_grid_coordinate — butler's cell identity becomes data, and a UUID takes over identity"),
    f("map.json:[].map_values[].barcode", Data, NotApplicable, "external_ids.barcode — the floor sticker is a label, replaceable at runtime, never a key"),
    f("map.json:[].map_values[].world_coordinate", Structure, NotApplicable, "the cell's pose in the floor frame, millimetres"),
    f("map.json:[].map_values[].vda_world_coordinate", Data, EmittedOnly, "external_ids.vda_world_coordinate, beside a declared vendor frame that reproduces it"),
    f("map.json:[].map_values[].size_info", Structure, NotApplicable, "cell.size_mm — the two half-extents on each axis summed"),
    f("map.json:[].map_values[].blocked", Structure, NotApplicable, "cell.blocked"),
    f("map.json:[].map_values[].adjacency", Partial, LoadBearing, "an explicit stitch per traversable pair; directional and impassable pairs have no v0 form"),
    f("map.json:[].map_values[].neighbours", Partial, LoadBearing, "bit 1 decides whether an edge exists at all; the loaded/unloaded grade has no component"),
    f("map.json:[].map_values[].zone", Partial, LoadBearing, "external_ids.zone; the e-stop domain itself has no component (IDIOMS reserves safety_domain)"),
    f("map.json:[].map_values[].sector", Structure, NotApplicable, "membership of a sector entity, which SPEC §10 carries directly"),
    f("map.json:[].map_values[].block", Redundant, NotApplicable, "the inverse of block.json's coordinate lists, which agree row for row on every dump"),
    f("map.json:[].map_values[].location_profile_id", Structure, NotApplicable, "resolves to cell.traversable_by, and rides in external_ids besides"),
    f("map.json:[].map_values[].store_status", Impedance, LoadBearing, "the RTP rack-parking square butler turns into storage_info; no v0 component says a rack may stand here"),
    f("map.json:[].map_values[].special", Impedance, LoadBearing, "the bot re-initialization point; no v0 component"),
    f("map.json:[].map_values[].grid_attribute", Impedance, EmittedOnly, "butler recomputes gridinfo.attribute from eleven sources and overwrites this"),
    f("map.json:[].map_values[].composite_attribute_list", Impedance, EmittedOnly, "the second, list-shaped classification channel; butler only ever writes it itself"),
    f("map.json:[].map_values[].hasStorables", Impedance, EmittedOnly, "vestigial: the only value in any dump is 0, and no reader was found"),
    f("map.json:[].map_values[].conveyor_direction_attribute", Impedance, LoadBearing, "conveyor travel direction on a cell; conveyors have no v0 component"),
    f("map.json:[].map_values[].dockPoint", Redundant, NotApplicable, "duplicates dock_point.json row for row (symmetric difference 0 on site-c)"),
    f("map.json:[].map_values[].barcode_properties", Impedance, LoadBearing, "the per-cell bot-permission table; `{}` wherever present in these dumps"),
    f("map.json:[].map_values[].block_reasons", Runtime, NotApplicable, "why a cell is blocked right now"),
    f("map.json:[].map_values[].botid", Runtime, NotApplicable, "which robot is standing on the cell right now"),
    f("map.json:[].map_values[].floor", Redundant, NotApplicable, "repeats floor_id, which the file states once at the top"),
    f("map.json:[].map_values[].world_coordinate_reference_neighbour", Redundant, EmittedOnly, "the parent edge a legacy position was accumulated along; butler never reads it"),
    f("map.json:[].location_profiles", Container, NotApplicable, "the named permission bundles"),
    f("map.json:[].location_profiles[].location_profile_id", Structure, NotApplicable, "names the bundle each cell resolves through"),
    f("map.json:[].location_profiles[].bot_orientation_details", Partial, LoadBearing, "which bot types are permitted becomes cell.traversable_by; the per-orientation lift-height ranges and ground clearance have no component"),
    f("map.json:[].location_profiles[].edge_height_clearance", Impedance, LoadBearing, "per-direction clearance on the edge; `{}` in every dump, and no v0 component"),
    f("map.json:[].vda5050_reference", Redundant, NotApplicable, "one sample point offered in place of a transform; the declared vendor frame supersedes it"),
    f("map.json:[].vda5050_reference.barcode", Redundant, NotApplicable, "the cell the sample point names"),
    f("map.json:[].vda5050_reference.vda5050_coordinate", Redundant, NotApplicable, "that cell's vendor-frame position, which the declared frame reproduces"),
    // ---- zone.json -----------------------------------------------------------
    f("zone.json", Container, NotApplicable, "zone existence plus runtime pause and block flags"),
    f("zone.json:[].zonerec", Container, NotApplicable, "the record wrapper"),
    f("zone.json:[].zonerec.zone_id", Partial, LoadBearing, "carried per cell in external_ids; the e-stop domain has no v0 component to be an entity of"),
    f("zone.json:[].zonerec.blocked", Runtime, NotApplicable, "whether the zone is clear right now"),
    f("zone.json:[].zonerec.paused", Runtime, NotApplicable, "whether the zone is paused right now"),
    // ---- sectors.json / sector.json -----------------------------------------
    f("sectors.json", Container, NotApplicable, "sector to member cells"),
    f("sectors.json:[]", Redundant, EmittedOnly, "the inverse of the cell `sector` field, which the import reads instead; butler does not ingest this file at all"),
    f("sector.json", Container, NotApplicable, "the older spelling of sectors.json"),
    f("sector.json:[]", Redundant, EmittedOnly, "as sectors.json"),
    f("sectorMxUPreferences.json", Impedance, EmittedOnly, "carrier type to an ordered sector preference list; no v0 component, and no reader found in butler"),
    // ---- vtm_zone.json -------------------------------------------------------
    f("vtm_zone.json", Container, NotApplicable, "aisles as ordered lists of io-point cells"),
    f("vtm_zone.json:aisle_info", Container, NotApplicable, "the aisle list"),
    f("vtm_zone.json:aisle_info[].aisle_id", Data, NotApplicable, "external_ids.butler_aisle_id on the sector the aisle becomes"),
    f("vtm_zone.json:aisle_info[].display_name", Data, NotApplicable, "the entity's name, and external_ids besides"),
    f("vtm_zone.json:aisle_info[].io_point_coordinates", Structure, NotApplicable, "the sector's members"),
    f("vtm_zone.json:aisle_info[].additional_information", Container, NotApplicable, "the wrapper butler parses and discards"),
    f("vtm_zone.json:aisle_info[].additional_information.type", Data, EmittedOnly, "external_ids.aisle_type; butler parses it and throws it away"),
    // ---- block.json ----------------------------------------------------------
    f("block.json", Container, NotApplicable, "named cell groups used as queues and waiting areas"),
    f("block.json:[].entity_type", Partial, LoadBearing, "queue and sequenced_queue become a queue component, waiting becomes a waiting component, conveyor-point has nowhere to go"),
    f("block.json:[].entity_details", Container, NotApplicable, "the payload wrapper, schemaless in butler"),
    f("block.json:[].entity_details.block_id", Data, NotApplicable, "external_ids.butler_block_id"),
    f("block.json:[].entity_details.coordinates", Partial, LoadBearing, "a queue's path; a waiting area has no way to name its cells in v0"),
    f("block.json:[].entity_details.extra_details", Container, NotApplicable, "the sequenced-queue extras"),
    f("block.json:[].entity_details.extra_details.sync_point", Impedance, LoadBearing, "the cell a sequenced queue synchronises at; queue has no such field"),
    f("block.json:[].entity_details.extra_details.exit_point", Impedance, LoadBearing, "the cell a sequenced queue leaves by; queue has no such field"),
    // ---- pps.json ------------------------------------------------------------
    f("pps.json", Container, NotApplicable, "pick and put stations, in two incompatible versions"),
    f("pps.json:[].pps_id", Data, NotApplicable, "external_ids.butler_pps_id"),
    f("pps.json:[].location", Structure, NotApplicable, "the station's pose, via the cell it names"),
    f("pps.json:[].type", Structure, NotApplicable, "station.role, the product identity"),
    f("pps.json:[].allowed_modes", Partial, LoadBearing, "a single pick-or-put mode decides station.kind; a station allowing several has no single kind"),
    f("pps.json:[].version", Redundant, NotApplicable, "which export dialect this record is in; the document states its own proto_version"),
    f("pps.json:[].system", Impedance, LoadBearing, "relay versus RTP: the site generation has no component, and today a consumer sniffs it"),
    f("pps.json:[].eligible_system", Impedance, LoadBearing, "the v2 spelling of the same"),
    f("pps.json:[].orchestrator", Impedance, EmittedOnly, "which system drives the station; butler's ppsinfo has no field for it"),
    f("pps.json:[].status", Runtime, NotApplicable, "whether the station is connected right now"),
    f("pps.json:[].pps_url", Runtime, NotApplicable, "a localhost service address inside a map file"),
    f("pps.json:[].blocks", Redundant, NotApplicable, "the same list as entities.blocks, stated twice per station"),
    f("pps.json:[].entities", Container, NotApplicable, "the station's parts"),
    f("pps.json:[].entities.blocks", Impedance, LoadBearing, "which queues serve this station; station has no field naming its queues"),
    f("pps.json:[].entities.presentation_points", Container, NotApplicable, "the presentation point list"),
    f("pps.json:[].entities.presentation_points[].point_id", Data, NotApplicable, "external_ids.point_id"),
    f("pps.json:[].entities.presentation_points[].presented_for", Structure, NotApplicable, "presentation_point.for"),
    f("pps.json:[].entities.presentation_points[].presented_entity_allowed_types", Impedance, LoadBearing, "butler derives the point's ttp/rtp type from it; presentation_point has no carrier field"),
    f("pps.json:[].entities.presentation_points[].presenting_agent_type", Impedance, LoadBearing, "butler derives the point's sub_type from it; no v0 field"),
    f("pps.json:[].entities.presentation_points[].presenting_agent_id", Impedance, EmittedOnly, "no reader found in butler"),
    f("pps.json:[].entities.presentation_points[].light_curtain", Impedance, LoadBearing, "the optical safety barrier; no v0 component"),
    f("pps.json:[].entities.presentation_points[].attributes", Container, NotApplicable, "the attribute wrapper"),
    f("pps.json:[].entities.presentation_points[].attributes.barcode", Structure, NotApplicable, "resolves to the cell the point is posed at"),
    f("pps.json:[].entities.presentation_points[].attributes.world_coordinates", Redundant, NotApplicable, "a GRID coordinate under a name that says world; it names the same cell as the barcode, which the import checks"),
    f("pps.json:[].entities.presentation_points[].attributes.presentation_height", Impedance, LoadBearing, "the height a carrier is presented at; no v0 field"),
    f("pps.json:[].entities.presentation_points[].attributes.bot_type", Impedance, LoadBearing, "which robot class presents here; no v0 field"),
    f("pps.json:[].entities.presentation_points[].attributes.bot_orientation", Impedance, LoadBearing, "the heading the robot presents at; no v0 field"),
    f("pps.json:[].entities.presentation_points[].attributes.pick_direction", Impedance, LoadBearing, "the direction the operator reaches from; no v0 field"),
    f("pps.json:[].entities.presentation_points[].attributes.is_workbench", Impedance, LoadBearing, "whether this is where the operator works; butler reads it into sub_type"),
    f("pps.json:[].entities.presentation_points[].attributes.display_attributes", Container, NotApplicable, "the operator display wrapper"),
    f("pps.json:[].entities.presentation_points[].attributes.display_attributes.display_id", Impedance, EmittedOnly, "operator display hardware; no v0 component"),
    f("pps.json:[].entities.presentation_points[].attributes.display_attributes.direction", Impedance, EmittedOnly, "which way the display faces; no v0 component"),
    f("pps.json:[].associations", Container, NotApplicable, "the one place the export reifies a relationship"),
    f("pps.json:[].associations[].id", Impedance, LoadBearing, "butler stores ppsinfo.associations whole; the protocol has no station-internal transport graph"),
    f("pps.json:[].associations[].type", Impedance, LoadBearing, "the relation's kind, e.g. allowed_transport"),
    f("pps.json:[].associations[].entities", Impedance, LoadBearing, "station-local handles mixing a global and a local namespace"),
    f("pps.json:[].associations[].connections", Impedance, LoadBearing, "the typed connected graph over those handles"),
    f("pps.json:[].associations[].connections[].from", Impedance, LoadBearing, "one endpoint of a connection"),
    f("pps.json:[].associations[].connections[].to", Impedance, LoadBearing, "the other endpoint"),
    f("pps.json:[].pps_points", Container, NotApplicable, "the v2 spelling of presentation points"),
    f("pps.json:[].pps_points[].coordinate", Structure, NotApplicable, "resolves to the cell the point is posed at"),
    f("pps.json:[].pps_points[].position_id", Data, NotApplicable, "external_ids.position_id"),
    f("pps.json:[].pps_points[].type", Impedance, LoadBearing, "rtp versus ttp: the generation again, with no component"),
    f("pps.json:[].pick_position", Impedance, LoadBearing, "the v2 station's pick cell; butler keys tasks on it"),
    f("pps.json:[].pick_direction", Impedance, LoadBearing, "the direction the v2 station is picked from"),
    f("pps.json:[].queue_barcodes", Impedance, LoadBearing, "a bare list of queue cells with no stated order; a v0 queue path is ordered, and inventing that order would invent the queue"),
    f("pps.json:[].put_docking_positions", Impedance, LoadBearing, "where carriers dock for putting; no v0 component"),
    f("pps.json:[].docking_stations", Container, NotApplicable, "the v2 station's docking bays"),
    f("pps.json:[].docking_stations[].dock_station_id", Impedance, LoadBearing, "joins to dock_point.station_id; no v0 component"),
    f("pps.json:[].docking_stations[].dock_station_type", Impedance, LoadBearing, "what kind of bay it is"),
    f("pps.json:[].docking_stations[].docking_type", Impedance, LoadBearing, "manual or automatic docking"),
    f("pps.json:[].docking_stations[].group_id", Impedance, LoadBearing, "which group of bays it belongs to"),
    f("pps.json:[].docking_stations[].rollcage_type_id", Impedance, LoadBearing, "the carrier type the bay accepts; joins to tote_type.racktype_id"),
    f("pps.json:[].docking_stations[].tags", Impedance, LoadBearing, "selection tags on the bay"),
    // ---- tote_storage.json ---------------------------------------------------
    f("tote_storage.json", Container, NotApplicable, "every storage position in the building, as flat tuples"),
    f("tote_storage.json:[].location", Impedance, LoadBearing, "the TLOC allocation-counter id; a v0 slot exists only as something a rack derives, and the export declares no rack"),
    f("tote_storage.json:[].io_point", Container, NotApplicable, "the access cell wrapper"),
    f("tote_storage.json:[].io_point.coordinate", Impedance, LoadBearing, "the cell a robot stands on to reach the position"),
    f("tote_storage.json:[].io_point.bot_orientation", Impedance, LoadBearing, "the heading it stands at"),
    f("tote_storage.json:[].storage_direction", Impedance, LoadBearing, "which way the position lies from the access cell"),
    f("tote_storage.json:[].ndeep", Impedance, LoadBearing, "the depth index; SPEC's slot.depth would carry it, if a rack existed to derive the slot"),
    f("tote_storage.json:[].height", Impedance, LoadBearing, "millimetres above the floor; an entry of SPEC's rack.levels would carry the ladder"),
    f("tote_storage.json:[].relay_point", Impedance, LoadBearing, "the cell a VTM hands a tote to an HTM at; SPEC's relay, again derived from a rack"),
    f("tote_storage.json:[].storage_sector", Impedance, LoadBearing, "the storage-policy partition of the position, stated per position on site-b only"),
    // ---- tote_type.json ------------------------------------------------------
    f("tote_type.json", Container, NotApplicable, "the rack and carrier geometry template"),
    f("tote_type.json:[].racktype_id", Impedance, LoadBearing, "the template's name; nothing in the export instantiates it at a place, so no rack can be declared from it"),
    f("tote_type.json:[].rack_width", Impedance, LoadBearing, "centimetres here, while tote_storage.height is millimetres, in one dump"),
    f("tote_type.json:[].rack_depth", Impedance, LoadBearing, "as rack_width"),
    f("tote_type.json:[].rack_outer_width", Impedance, LoadBearing, "outer extent including structure"),
    f("tote_type.json:[].rack_outer_depth", Impedance, LoadBearing, "outer extent including structure"),
    f("tote_type.json:[].inter_leg_width", Impedance, LoadBearing, "the gap a robot drives between"),
    f("tote_type.json:[].inter_leg_depth", Impedance, LoadBearing, "the gap a robot drives between"),
    f("tote_type.json:[].initial_mass_of_the_rack", Impedance, LoadBearing, "mass; no v0 component"),
    f("tote_type.json:[].initial_cm_of_the_rack", Impedance, LoadBearing, "centre of mass; no v0 component"),
    f("tote_type.json:[].density_profile", Impedance, LoadBearing, "load distribution; no v0 component"),
    f("tote_type.json:[].replenishable", Impedance, LoadBearing, "whether the type may be replenished; a policy with no v0 component"),
    f("tote_type.json:[].floor_structure", Container, NotApplicable, "the shelf-level tree"),
    f("tote_type.json:[].floor_structure[].floor_id", Impedance, LoadBearing, "the shelf level's letter"),
    f("tote_type.json:[].floor_structure[].base_height", Impedance, LoadBearing, "the shelf's height; an entry of SPEC's rack.levels would carry it"),
    f("tote_type.json:[].floor_structure[].same_base_structure", Container, NotApplicable, "levels sharing a base height"),
    f("tote_type.json:[].floor_structure[].same_base_structure[].weight_limit_of_the_floor", Impedance, LoadBearing, "the shelf's load limit; no v0 component"),
    f("tote_type.json:[].floor_structure[].same_base_structure[].rack_face_structure", Container, NotApplicable, "the two faces"),
    f("tote_type.json:[].floor_structure[].same_base_structure[].rack_face_structure[].rack_face", Impedance, LoadBearing, "face 0 or 1; SPEC states sidedness on an access entity"),
    f("tote_type.json:[].floor_structure[].same_base_structure[].rack_face_structure[].slot_structure", Container, NotApplicable, "the slots on one face of one level"),
    f("tote_type.json:[].floor_structure[].same_base_structure[].rack_face_structure[].slot_structure[].slot_tags", Impedance, LoadBearing, "what may go in the slot"),
    f("tote_type.json:[].floor_structure[].same_base_structure[].rack_face_structure[].slot_structure[].slot_type", Impedance, LoadBearing, "the slot's kind"),
    f("tote_type.json:[].floor_structure[].same_base_structure[].rack_face_structure[].slot_structure[].slot_center", Impedance, LoadBearing, "the slot's centre in template-local units"),
    f("tote_type.json:[].floor_structure[].same_base_structure[].rack_face_structure[].slot_structure[].barcodes", Impedance, LoadBearing, "the slot's own labels"),
    f("tote_type.json:[].floor_structure[].same_base_structure[].rack_face_structure[].slot_structure[].height", Impedance, LoadBearing, "slot extent"),
    f("tote_type.json:[].floor_structure[].same_base_structure[].rack_face_structure[].slot_structure[].depth", Impedance, LoadBearing, "slot extent"),
    f("tote_type.json:[].floor_structure[].same_base_structure[].rack_face_structure[].slot_structure[].length", Impedance, LoadBearing, "slot extent"),
    f("tote_type.json:[].floor_structure[].same_base_structure[].rack_face_structure[].slot_structure[].x", Impedance, LoadBearing, "slot position along the face"),
    f("tote_type.json:[].floor_structure[].same_base_structure[].rack_face_structure[].slot_structure[].y", Impedance, LoadBearing, "slot position up the face"),
    // ---- charger.json --------------------------------------------------------
    f("charger.json", Container, NotApplicable, "chargers and their approach and re-init points"),
    f("charger.json:[].charger_id", Impedance, LoadBearing, "SPEC §11 puts charging behaviour outside v0 world data, so a charger has no component to be"),
    f("charger.json:[].charger_location", Impedance, LoadBearing, "the cell the charger stands at, by barcode"),
    f("charger.json:[].charger_direction", Impedance, LoadBearing, "which way a robot docks"),
    f("charger.json:[].charger_type", Impedance, LoadBearing, "a closed vendor-shaped enum in butler"),
    f("charger.json:[].entry_point_location", Impedance, LoadBearing, "the cell a robot approaches from"),
    f("charger.json:[].entry_point_direction", Impedance, LoadBearing, "the heading it approaches at"),
    f("charger.json:[].reinit_point_location", Impedance, LoadBearing, "the re-init cell, on the [500,500] diagonal"),
    f("charger.json:[].reinit_point_direction", Impedance, LoadBearing, "the heading it re-initialises at"),
    f("charger.json:[].mode", Impedance, LoadBearing, "the charger's operating mode"),
    f("charger.json:[].status", Runtime, NotApplicable, "connected or not, right now"),
    f("charger.json:[].charge", Runtime, NotApplicable, "the current charge reading"),
    f("charger.json:[].attached_butler_id", Runtime, NotApplicable, "which robot is docked right now"),
    f("charger.json:[].power_status", Runtime, NotApplicable, "the current power state"),
    f("charger.json:[].error_reason", Runtime, NotApplicable, "the current fault"),
    f("charger.json:[].blocked_by_butler_ids", Runtime, NotApplicable, "which robots are blocking it right now"),
    // ---- ods_excluded.json ---------------------------------------------------
    f("ods_excluded.json", Container, NotApplicable, "obstacle-detection exclusions per cell and heading"),
    f("ods_excluded.json:ods_excluded_list", Container, NotApplicable, "the exclusion list"),
    f("ods_excluded.json:ods_excluded_list[].ods_tuple", Impedance, LoadBearing, "a (cell, heading) pair butler gates its obstacle sensor on; IDIOMS reserves the mapping and v0 has no heading-aware policy component"),
    f("ods_excluded.json:ods_excluded_list[].excluded", Impedance, LoadBearing, "whether the exclusion is on"),
    // ---- dock_point.json -----------------------------------------------------
    f("dock_point.json", Container, NotApplicable, "fixed docking points for carriers at stations"),
    f("dock_point.json:[].id", Impedance, LoadBearing, "the dock point's identity, which is its barcode; no v0 component"),
    f("dock_point.json:[].dock_barcode", Redundant, NotApplicable, "byte-identical to id in 53 of 53 rows"),
    f("dock_point.json:[].dock_coordinate", Impedance, LoadBearing, "the cell the carrier is parked on"),
    f("dock_point.json:[].direction", Impedance, LoadBearing, "the heading it is parked at"),
    f("dock_point.json:[].display_name", Impedance, LoadBearing, "the operator-facing name"),
    f("dock_point.json:[].station_id", Impedance, LoadBearing, "the station it belongs to; joins to pps.docking_stations"),
    f("dock_point.json:[].station_type", Impedance, LoadBearing, "the kind of station"),
    f("dock_point.json:[].mtu_id", Runtime, NotApplicable, "which trolley is parked there right now"),
    f("dock_point.json:[].rack_id", Runtime, NotApplicable, "which rack is parked there right now"),
    f("dock_point.json:[].rack_status", Runtime, NotApplicable, "that rack's current state"),
    f("dock_point.json:[].reserved_by", Runtime, NotApplicable, "who holds it right now"),
    f("dock_point.json:[].status", Runtime, NotApplicable, "docked or not, right now"),
    f("dock_point.json:[].station_point_status", Runtime, NotApplicable, "the point's current state"),
    // ---- conveyor.json / conveyor_mapping.json -------------------------------
    f("conveyor.json", Container, NotApplicable, "conveyor topology as a step chain"),
    f("conveyor.json:version", Redundant, NotApplicable, "the file's own dialect version; the document states its proto_version"),
    f("conveyor.json:conveyor_data", Container, NotApplicable, "the conveyor list"),
    f("conveyor.json:conveyor_data[].conveyor_id", Impedance, LoadBearing, "conveyors have no v0 component; SPEC §11 keeps their mechanics outside world data"),
    f("conveyor.json:conveyor_data[].display_name", Impedance, LoadBearing, "the conveyor's operator-facing name"),
    f("conveyor.json:conveyor_data[].connected_conveyors", Impedance, LoadBearing, "the topology between conveyors"),
    f("conveyor.json:conveyor_data[].conveyor_steps", Container, NotApplicable, "the ordered steps"),
    f("conveyor.json:conveyor_data[].conveyor_steps[].location_id", Impedance, LoadBearing, "the step's own id"),
    f("conveyor.json:conveyor_data[].conveyor_steps[].coordinate", Impedance, LoadBearing, "the step's grid coordinate, always on the x==999 hatch"),
    f("conveyor.json:conveyor_data[].conveyor_steps[].world_coordinate", Impedance, LoadBearing, "the step's millimetre position"),
    f("conveyor.json:conveyor_data[].conveyor_steps[].location_type", Impedance, LoadBearing, "what the step is"),
    f("conveyor.json:conveyor_data[].entry_points", Container, NotApplicable, "where things get on"),
    f("conveyor.json:conveyor_data[].entry_points[].location_id", Impedance, LoadBearing, "the step the port serves"),
    f("conveyor.json:conveyor_data[].entry_points[].io_point", Impedance, LoadBearing, "the cell a robot stands on"),
    f("conveyor.json:conveyor_data[].entry_points[].io_point_type", Impedance, LoadBearing, "the kind of access"),
    f("conveyor.json:conveyor_data[].entry_points[].height", Impedance, LoadBearing, "the handoff height"),
    f("conveyor.json:conveyor_data[].entry_points[].direction", Impedance, LoadBearing, "the handoff direction"),
    f("conveyor.json:conveyor_data[].entry_points[].bot_orientation", Impedance, LoadBearing, "the heading the robot stands at"),
    f("conveyor.json:conveyor_data[].exit_points", Container, NotApplicable, "where things get off"),
    f("conveyor.json:conveyor_data[].exit_points[].location_id", Impedance, LoadBearing, "as the entry point"),
    f("conveyor.json:conveyor_data[].exit_points[].io_point", Impedance, LoadBearing, "as the entry point"),
    f("conveyor.json:conveyor_data[].exit_points[].io_point_type", Impedance, LoadBearing, "as the entry point"),
    f("conveyor.json:conveyor_data[].exit_points[].height", Impedance, LoadBearing, "as the entry point"),
    f("conveyor.json:conveyor_data[].exit_points[].direction", Impedance, LoadBearing, "as the entry point"),
    f("conveyor.json:conveyor_data[].exit_points[].bot_orientation", Impedance, LoadBearing, "as the entry point"),
    f("conveyor.json:hai_port_types", Impedance, LoadBearing, "vendor port classification butler ingests into hai_port_info; empty in every dump"),
    f("conveyor_mapping.json", Container, NotApplicable, "the three-frame join table"),
    f("conveyor_mapping.json:[].go_coordinates", Redundant, EmittedOnly, "the GO grid coordinate, already carried per cell; no reader found in butler"),
    f("conveyor_mapping.json:[].HAI_coordinates", Impedance, EmittedOnly, "the vendor frame's coordinate, UNDEFINED in every dump; no reader found"),
    f("conveyor_mapping.json:[].conveyor_coordinates", Container, NotApplicable, "the conveyor-local address"),
    f("conveyor_mapping.json:[].conveyor_coordinates.conveyor_id", Impedance, EmittedOnly, "which conveyor; no reader found"),
    f("conveyor_mapping.json:[].conveyor_coordinates.step_id", Impedance, EmittedOnly, "which step; no reader found"),
    f("conveyor_mapping.json:[].location_type", Impedance, EmittedOnly, "the kind of join; no reader found"),
    f("conveyor_mapping.json:[].pps_id", Impedance, EmittedOnly, "the station the port serves; no reader found"),
    // ---- the five empty files ------------------------------------------------
    f("elevator.json", Impedance, LoadBearing, "floor-to-floor transfer, `[]` on every site; butler ingests elevatorinfo, and multi-floor topology has no v0 component"),
    f("fire_emergency.json", Impedance, EmittedOnly, "evacuation behaviour per region, `[]` on every site; no component, and no reader found"),
    f("queue_data.json", Runtime, NotApplicable, "runtime queue occupancy, `[]` on every site"),
    f("opc_connection.json", Impedance, EmittedOnly, "the OPC-UA plant-equipment link, `[]` on every site; no component, and no reader found"),
    f("zone_controller.json", Impedance, LoadBearing, "the devices controlling a zone's gates and interlocks, `[]` on every site; part of the e-stop domain the protocol has no component for"),
    // ---- station interior ($WM only) ----------------------------------------
    f("pps_bin.json", Container, NotApplicable, "the put-wall bins, stored as a captured HTTP request"),
    f("pps_bin.json:url", Redundant, NotApplicable, "a request envelope field, not model data"),
    f("pps_bin.json:type", Redundant, NotApplicable, "a request envelope field, not model data"),
    f("pps_bin.json:header", Container, NotApplicable, "a request envelope field, not model data"),
    f("pps_bin.json:header.accept", Redundant, NotApplicable, "a request envelope field, not model data"),
    f("pps_bin.json:header.content-type", Redundant, NotApplicable, "a request envelope field, not model data"),
    f("pps_bin.json:data", Container, NotApplicable, "the bins themselves"),
    f("pps_bin.json:data[].bin_id", Impedance, LoadBearing, "a station's interior — bins, seats, lights — has no v0 component; a template with a local frame is where it would go"),
    f("pps_bin.json:data[].pps_id", Impedance, LoadBearing, "the station the bin belongs to"),
    f("pps_bin.json:data[].position", Impedance, LoadBearing, "the bin's [row, col] in the wall"),
    f("pps_bin.json:data[].coordinate", Impedance, LoadBearing, "the bin's station-local centimetres, derived from position and the bin type"),
    f("pps_bin.json:data[].direction", Impedance, LoadBearing, "which way the bin opens"),
    f("pps_bin.json:data[].group_id", Impedance, LoadBearing, "the bin's group"),
    f("pps_bin.json:data[].type", Impedance, LoadBearing, "the bin type, into pps_bintype"),
    f("pps_bin.json:data[].allowed_tags", Impedance, LoadBearing, "what may go in the bin"),
    f("pps_bin.json:data[].front_seat_name", Impedance, LoadBearing, "the operator position serving the bin from the front"),
    f("pps_bin.json:data[].back_seat_name", Impedance, LoadBearing, "and from the back"),
    f("pps_bin.json:data[].status", Runtime, NotApplicable, "the bin's current state"),
    f("pps_bintype.json", Container, NotApplicable, "bin geometry, stored as a captured HTTP request"),
    f("pps_bintype.json:url", Redundant, NotApplicable, "a request envelope field, not model data"),
    f("pps_bintype.json:type", Redundant, NotApplicable, "a request envelope field, not model data"),
    f("pps_bintype.json:header", Container, NotApplicable, "a request envelope field, not model data"),
    f("pps_bintype.json:header.accept", Redundant, NotApplicable, "a request envelope field, not model data"),
    f("pps_bintype.json:header.content-type", Redundant, NotApplicable, "a request envelope field, not model data"),
    f("pps_bintype.json:data", Container, NotApplicable, "the bin types"),
    f("pps_bintype.json:data[].type_id", Impedance, LoadBearing, "the bin type's name"),
    f("pps_bintype.json:data[].tag", Impedance, LoadBearing, "tags on the type"),
    f("pps_bintype.json:data[].dimension", Container, NotApplicable, "the type's extent"),
    f("pps_bintype.json:data[].dimension.height", Impedance, LoadBearing, "bin extent, centimetres"),
    f("pps_bintype.json:data[].dimension.length", Impedance, LoadBearing, "bin extent, centimetres"),
    f("pps_bintype.json:data[].dimension.breadth", Impedance, LoadBearing, "bin extent, centimetres"),
    f("pps_seat.json", Container, NotApplicable, "the operator seats at a station"),
    f("pps_seat.json:pps_seats", Container, NotApplicable, "the seat list"),
    f("pps_seat.json:pps_seats[].pps_id", Impedance, LoadBearing, "the station the seat belongs to"),
    f("pps_seat.json:pps_seats[].seat_name", Impedance, LoadBearing, "the seat's name, which bins reference"),
    f("pps_seat.json:pps_seats[].seat_type", Impedance, LoadBearing, "front or back"),
    f("pps_seat.json:pps_seats[].seat_data", Container, NotApplicable, "the seat's extras"),
    f("pps_seat.json:pps_seats[].seat_data.key", Impedance, LoadBearing, "an opaque seat key"),
    f("pptl.json", Container, NotApplicable, "put-to-light devices bound to bins"),
    f("pptl.json:[].pptls", Container, NotApplicable, "the device list"),
    f("pptl.json:[].pptls[].peripheral_id", Impedance, LoadBearing, "the light module's identity; station peripherals have no v0 component"),
    f("pptl.json:[].pptls[].peripheral_type", Impedance, LoadBearing, "the kind of peripheral"),
    f("pptl.json:[].pptls[].barcode", Impedance, LoadBearing, "the module's own label"),
    f("pptl.json:[].pptls[].pps_bin_id", Impedance, LoadBearing, "the bin it lights"),
    f("pptl.json:[].pptls[].pps_id", Impedance, LoadBearing, "the station it belongs to"),
];

/// The table, indexed by path.
pub fn table() -> BTreeMap<&'static str, Field> {
    FIELDS.iter().map(|field| (field.path, *field)).collect()
}

#[cfg(test)]
mod tests {
    use super::{table, Disposition, Reading, FIELDS};

    #[test]
    fn no_path_is_disposed_of_twice() {
        assert_eq!(table().len(), FIELDS.len(), "a path appears twice in FIELDS");
    }

    #[test]
    fn every_row_says_something() {
        for field in FIELDS {
            assert!(!field.note.is_empty(), "{} has no note", field.path);
            assert!(
                field.path.ends_with(".json") || field.path.contains(".json:"),
                "{} is not a path into an export file",
                field.path
            );
        }
    }

    /// A container is a nesting key. If one ever acquires a reading, the table
    /// has started arguing with itself about whether a wrapper is a fact.
    #[test]
    fn containers_carry_no_reading() {
        for field in FIELDS {
            if field.disposition == Disposition::Container {
                assert_eq!(field.reading, Reading::NotApplicable, "{} is a container with a reading", field.path);
            }
        }
    }
}
