//! Within the Map Creator adapter, `coverage` is **the extended disposition table**:
//! one row per key path whose fate the vocabulary's components change, layered
//! over the one-way importer's own table.
//!
//! It is an overlay rather than an edit, for two reasons. The importer's table is
//! its own committed opinion, which the importer's published ledgers were
//! measured against, and rewriting its dispositions would make those say
//! something nobody measured. And the delta is the deliverable: read the two
//! tables together and the overlay is exactly what the round trip bought, path
//! by path.
//!
//! The rule the importer set stays: a key path an export carries that neither
//! table disposes of **is refused**. That is the only mechanism keeping a claim
//! of losslessness honest as new sites arrive with fields nobody has seen — a
//! field that appears for the first time cannot slip past, because the adapter
//! does not know where to put it and says so.

use crate::format::coverage::{Disposition, Field, Reading};
use std::collections::BTreeMap;

use Disposition::{Container, Data, Impedance, Redundant, Structure};
use Reading::{EmittedOnly, LoadBearing, NotApplicable};

const fn f(path: &'static str, disposition: Disposition, reading: Reading, note: &'static str) -> Field {
    Field { path, disposition, reading, note }
}

/// Every path this adapter disposes of differently from the one-way importer,
/// plus every path no table had a row for.
///
/// `Structure` here means the same thing it means there — the path becomes an
/// entity, or a field of a component whose meaning is fixed — with one addition:
/// a component of the vocabulary has its meaning fixed by `vocabulary()`, which hands
/// out its JSON Schema and its prose line, so a field of one is structure and not
/// a bag.
pub const OVERRIDES: &[Field] = &[
    // ---- map.json: the eight the one-way importer had nowhere to put --------
    f("map.json:[].map_values[].store_status", Structure, LoadBearing, "cell.store_status — the square a movable rack may park on, kept as the export states it; the core still has no `a rack may stand here`, which the ledger says"),
    f("map.json:[].map_values[].special", Structure, LoadBearing, "cell.special — the bot re-initialization point, kept as stated; a charger points at one by name and the round trip preserves both ends"),
    f("map.json:[].map_values[].grid_attribute", Structure, EmittedOnly, "cell.grid_attribute — butler recomputes its own attribute from eleven sources and overwrites this, so it is carried and not read"),
    f("map.json:[].map_values[].composite_attribute_list", Structure, EmittedOnly, "cell.composite_attribute_list — the second, list-shaped classification channel"),
    f("map.json:[].map_values[].hasStorables", Structure, EmittedOnly, "cell.has_storables — vestigial (every value in every dump is 0 or null) and carried anyway, because the round trip decides nothing about what is worth keeping"),
    f("map.json:[].map_values[].conveyor_direction_attribute", Structure, LoadBearing, "cell.conveyor_direction_attribute — conveyor travel direction on a cell"),
    f("map.json:[].map_values[].barcode_properties", Structure, LoadBearing, "cell.barcode_properties — the per-cell bot-permission table, `{}` wherever present in these dumps and carried whole for the day it is not"),
    f("map.json:[].map_values[].world_coordinate_reference_neighbour", Structure, EmittedOnly, "cell.world_coordinate_reference_neighbour — the parent edge a legacy position was accumulated along; nothing reads it and the round trip writes it back"),
    f("map.json:[].map_values[].size_info", Structure, NotApplicable, "cell.size_info whole, with cell.size the per-axis sum beside it; the export writes from the four half-extents"),
    f("map.json:[].map_values[].adjacency", Structure, LoadBearing, "cell.adjacency whole, with one stitch entity per pair both ends permit travel across; the stitches are the derived form and the array is the original"),
    f("map.json:[].map_values[].neighbours", Structure, LoadBearing, "cell.neighbours whole, grades included; a stitch carries no grade and the array does"),
    f("map.json:[].map_values[].zone", Structure, LoadBearing, "a reference to the zone entity, so the zone string — typo, trailing space and all — stops being a join key"),
    f("map.json:[].map_values[].floor", Redundant, NotApplicable, "cell.floor — a second statement of the file's own floor_id, carried only because its PRESENCE is per-cell (1,198 cells of one site state it and 3,857 do not). The import checks the two agree and the ledger names any cell where they do not"),
    f("map.json:[].map_values[].dockPoint", Redundant, NotApplicable, "the dock point entity that names this cell; rebuilt from dock_point.json, whose rows match the flagged cells exactly on every site"),
    f("map.json:[].location_profiles[]", Structure, LoadBearing, "one location_profile entity per profile, the permission table whole"),
    f("map.json:[].location_profiles[].edge_height_clearance", Structure, LoadBearing, "location_profile.edge_height_clearance — `{}` in every dump and carried whole"),
    f("map.json:[].vda5050_reference", Redundant, NotApplicable, "mapcreator.frame.vda5050_reference_barcode on the floor frame; both halves of the reference are the named cell's own fields on every site, so the export rebuilds it from the cell"),
    f("map.json:[].map_values[].location_profile_id", Structure, LoadBearing, "a reference to the location_profile entity, and cell.traversable_by read off that profile beside it. The legacy string is NOT copied onto the cell: the reference is the home and the export rebuilds the string from the profile entity's own key, which is what keeps a display string out of reference position (SPEC §2)"),
    // ---- zone.json ---------------------------------------------------------
    f("zone.json:[].zonerec.zone_id", Structure, LoadBearing, "one zone entity per zone; the e-stop domain is an entity at last, though the core still has no `safety_domain` to make it mean anything"),
    f("zone.json:[].zonerec.blocked", Structure, NotApplicable, "zone.runtime.blocked — live state, carried under a named sub-object rather than dropped, so a later policy can drop it deliberately (spec §4)"),
    f("zone.json:[].zonerec.paused", Structure, NotApplicable, "zone.runtime.paused — as blocked"),
    // ---- sectors.json ------------------------------------------------------
    f("sectors.json:[]", Redundant, EmittedOnly, "the inverse of the cell `sector` field, rebuilt from the sector entities' members in map_values order; `undefined` is one of those entities, because a cell that states it is not a cell that states nothing"),
    f("sector.json:[]", Redundant, EmittedOnly, "as sectors.json"),
    f("sectorMxUPreferences.json", Structure, EmittedOnly, "one sector_preference entity per carrier type, preferring sector entities by identity"),
    // ---- vtm_zone.json -----------------------------------------------------
    f("vtm_zone.json:aisle_info[].additional_information.type", Structure, EmittedOnly, "aisle.additional_information, kept whole — butler parses it and throws it away, and the round trip does not"),
    // ---- block.json --------------------------------------------------------
    f("block.json:[].entity_type", Structure, LoadBearing, "block.entity_type, beside the core queue or waiting component the kind decides"),
    f("block.json:[].entity_details.coordinates", Structure, LoadBearing, "queue.path for a queue block, block.cells for a waiting one — which is the waiting-membership gap closed"),
    f("block.json:[].entity_details.extra_details.sync_point", Structure, LoadBearing, "block.extra_details.sync_point — a core queue cannot tell one step from another, and this carries the step"),
    f("block.json:[].entity_details.extra_details.exit_point", Structure, LoadBearing, "block.extra_details.exit_point — as sync_point"),
    // ---- pps.json ----------------------------------------------------------
    f("pps.json:[].version", Structure, NotApplicable, "mapcreator.station.dialect — not redundant: it decides the spelling of `location`, of the point list and of `pps_id` itself, so the export cannot write a record back without it"),
    f("pps.json:[].system", Structure, LoadBearing, "mapcreator.station.system — the product generation, which today a consumer sniffs by checking whether location_profiles exists"),
    f("pps.json:[].eligible_system", Structure, LoadBearing, "mapcreator.station.eligible_system — the v2 spelling of the same"),
    f("pps.json:[].orchestrator", Structure, EmittedOnly, "mapcreator.station.orchestrator"),
    f("pps.json:[].status", Structure, NotApplicable, "mapcreator.station.runtime.status"),
    f("pps.json:[].pps_url", Structure, NotApplicable, "mapcreator.station.runtime.url — a localhost service address inside a map file, carried as runtime state"),
    f("pps.json:[].blocks", Structure, NotApplicable, "mapcreator.station.blocks — NOT redundant with entities.blocks: the two lists differ in order on three stations of two sites and one of them is absent on two more, so both are stated"),
    f("pps.json:[].entities.blocks", Structure, LoadBearing, "mapcreator.station.served_by — which queues serve the station, by identity"),
    f("pps.json:[].entities.presentation_points[].presented_entity_allowed_types", Structure, LoadBearing, "presentation_point.presented_entity_allowed_types"),
    f("pps.json:[].entities.presentation_points[].presenting_agent_type", Structure, LoadBearing, "presentation_point.presenting_agent_type"),
    f("pps.json:[].entities.presentation_points[].presenting_agent_id", Structure, EmittedOnly, "presentation_point.presenting_agent_id"),
    f("pps.json:[].entities.presentation_points[].light_curtain", Structure, LoadBearing, "presentation_point.light_curtain — the optical safety barrier, present on some points and not others, so its absence is carried by being absent"),
    f("pps.json:[].entities.presentation_points[].attributes.world_coordinates", Redundant, NotApplicable, "a GRID coordinate under a name that says world, naming the cell the barcode names, which the import checks; the export rebuilds it from that cell, writes the copy the import kept where the barcode names no cell, and leaves it out where the point stated none"),
    f("pps.json:[].entities.presentation_points[].attributes.presentation_height", Structure, LoadBearing, "presentation_point.attributes.presentation_height"),
    f("pps.json:[].entities.presentation_points[].attributes.bot_type", Structure, LoadBearing, "presentation_point.attributes.bot_type"),
    f("pps.json:[].entities.presentation_points[].attributes.bot_orientation", Structure, LoadBearing, "presentation_point.attributes.bot_orientation"),
    f("pps.json:[].entities.presentation_points[].attributes.pick_direction", Structure, LoadBearing, "presentation_point.attributes.pick_direction"),
    f("pps.json:[].entities.presentation_points[].attributes.is_workbench", Structure, LoadBearing, "presentation_point.attributes.is_workbench"),
    f("pps.json:[].entities.presentation_points[].attributes.display_attributes.display_id", Structure, EmittedOnly, "presentation_point.attributes.display_attributes, kept whole"),
    f("pps.json:[].entities.presentation_points[].attributes.display_attributes.direction", Structure, EmittedOnly, "as display_id"),
    f("pps.json:[].associations[].id", Structure, LoadBearing, "mapcreator.station.associations, kept whole as the export spells it — handles and all, because the handles are the graph's own vertex names"),
    f("pps.json:[].associations[].type", Structure, LoadBearing, "as associations[].id"),
    f("pps.json:[].associations[].entities", Structure, LoadBearing, "as associations[].id"),
    f("pps.json:[].associations[].connections", Structure, LoadBearing, "as associations[].id"),
    f("pps.json:[].associations[].connections[].from", Structure, LoadBearing, "as associations[].id"),
    f("pps.json:[].associations[].connections[].to", Structure, LoadBearing, "as associations[].id"),
    f("pps.json:[].pps_points[].type", Structure, LoadBearing, "station_point.point_type"),
    f("pps.json:[].pick_position", Structure, LoadBearing, "mapcreator.station.pick_cell, by identity"),
    f("pps.json:[].pick_direction", Structure, LoadBearing, "mapcreator.station.pick_direction"),
    f("pps.json:[].queue_barcodes", Structure, LoadBearing, "mapcreator.station.queue_cells — the cells by identity in the order the file lists them, which is not a queue order and is not claimed to be one"),
    f("pps.json:[].put_docking_positions", Structure, LoadBearing, "mapcreator.station.put_docking_positions — `[]` in every dump and carried whole"),
    f("pps.json:[].docking_stations[].dock_station_id", Structure, LoadBearing, "mapcreator.station.docking_stations, kept whole; a bay is a sub-object of the station rather than an entity, because nothing in the export gives one a place on the floor"),
    f("pps.json:[].docking_stations[].dock_station_type", Structure, LoadBearing, "as docking_stations[].dock_station_id"),
    f("pps.json:[].docking_stations[].docking_type", Structure, LoadBearing, "as docking_stations[].dock_station_id"),
    f("pps.json:[].docking_stations[].group_id", Structure, LoadBearing, "as docking_stations[].dock_station_id"),
    f("pps.json:[].docking_stations[].rollcage_type_id", Structure, LoadBearing, "as docking_stations[].dock_station_id"),
    f("pps.json:[].docking_stations[].tags", Structure, LoadBearing, "as docking_stations[].dock_station_id"),
    // ---- tote_storage.json -------------------------------------------------
    f("tote_storage.json:[].location", Data, LoadBearing, "external_ids.mapcreator_tote_location, the position's lookup key — an allocation-counter id, which is why it is data and not identity"),
    f("tote_storage.json:[].io_point.coordinate", Structure, LoadBearing, "tote_storage.io_point.cell, by identity"),
    f("tote_storage.json:[].io_point.bot_orientation", Structure, LoadBearing, "tote_storage.io_point.bot_orientation"),
    f("tote_storage.json:[].storage_direction", Structure, LoadBearing, "tote_storage.storage_direction"),
    f("tote_storage.json:[].ndeep", Structure, LoadBearing, "tote_storage.ndeep — SPEC's slot.depth would carry it if a rack existed to derive the slot, and no export declares a rack"),
    f("tote_storage.json:[].height", Structure, LoadBearing, "tote_storage.height, millimetres above the floor"),
    f("tote_storage.json:[].relay_point", Structure, LoadBearing, "tote_storage.relay_cell, by identity"),
    f("tote_storage.json:[].storage_sector", Structure, LoadBearing, "tote_storage.storage_sector"),
    // ---- tote_type.json ----------------------------------------------------
    f("tote_type.json:[].racktype_id", Data, LoadBearing, "external_ids.mapcreator_racktype_id, the template's lookup key"),
    f("tote_type.json:[].floor_structure", Structure, LoadBearing, "rack_type.floor_structure, the shelf tree whole — nothing in the export places a rack anywhere, so the tree has no parts to be entities of and the round trip writes it back as one value"),
    // ---- charger.json ------------------------------------------------------
    f("charger.json:[].charger_id", Data, LoadBearing, "external_ids.mapcreator_charger_id, the charger's lookup key"),
    f("charger.json:[].charger_location", Structure, LoadBearing, "charger.at_cell, by identity rather than by the barcode the file writes"),
    f("charger.json:[].entry_point_location", Structure, LoadBearing, "charger.entry_cell, by identity"),
    f("charger.json:[].reinit_point_location", Structure, LoadBearing, "charger.reinit_cell, by identity — the cell on the [500,500] diagonal that also carries `special`"),
    f("charger.json:[].charger_direction", Structure, LoadBearing, "charger.at_direction"),
    f("charger.json:[].entry_point_direction", Structure, LoadBearing, "charger.entry_direction"),
    f("charger.json:[].reinit_point_direction", Structure, LoadBearing, "charger.reinit_direction"),
    f("charger.json:[].charger_type", Structure, LoadBearing, "charger.charger_type"),
    f("charger.json:[].mode", Structure, LoadBearing, "charger.mode"),
    // ---- ods_excluded.json -------------------------------------------------
    f("ods_excluded.json:ods_excluded_list[].ods_tuple", Structure, LoadBearing, "ods_exclusion.cell and .heading, split from the tuple; the tuple itself stays in external_ids as the lookup key, and 24 of one site's 180 name no cell and keep the barcode verbatim"),
    f("ods_excluded.json:ods_excluded_list[].excluded", Structure, LoadBearing, "ods_exclusion.excluded"),
    // ---- dock_point.json ---------------------------------------------------
    f("dock_point.json:[].id", Data, LoadBearing, "external_ids.mapcreator_dock_point_id, the dock point's lookup key"),
    f("dock_point.json:[].dock_coordinate", Structure, LoadBearing, "dock_point.at_cell, by identity"),
    f("dock_point.json:[].direction", Structure, LoadBearing, "dock_point.direction"),
    f("dock_point.json:[].display_name", Structure, LoadBearing, "dock_point.display_name, and the entity's own name"),
    f("dock_point.json:[].station_id", Structure, LoadBearing, "dock_point.docking_station_id, the string verbatim — NOT a reference: every row names a pps.docking_stations[].dock_station_id, which is a bay, and a bay is a sub-object of a station rather than an entity because nothing in the export gives one a place on the floor (records.rs says the same)"),
    f("dock_point.json:[].station_type", Structure, LoadBearing, "dock_point.station_type"),
    // ---- conveyor.json / conveyor_mapping.json -----------------------------
    f("conveyor.json:version", Structure, NotApplicable, "mapcreator.export.conveyor_header.version — the file's own dialect version, which the export has to write back"),
    f("conveyor.json:hai_port_types", Structure, LoadBearing, "mapcreator.export.conveyor_header.hai_port_types — empty in every dump and carried whole"),
    f("conveyor.json:conveyor_data[].conveyor_id", Data, LoadBearing, "external_ids.mapcreator_conveyor_id, the conveyor's lookup key"),
    f("conveyor.json:conveyor_data[].display_name", Structure, LoadBearing, "conveyor.display_name"),
    f("conveyor.json:conveyor_data[].connected_conveyors", Structure, LoadBearing, "conveyor.connected_conveyors"),
    f("conveyor.json:conveyor_data[].conveyor_steps", Structure, LoadBearing, "conveyor.steps whole, in file order; every step's coordinate names a real cell on the x==999 hatch and its world_coordinate equals that cell's, so the steps are a second reading of cells the document already declares"),
    f("conveyor.json:conveyor_data[].entry_points", Structure, LoadBearing, "conveyor.entry_points whole"),
    f("conveyor.json:conveyor_data[].exit_points", Structure, LoadBearing, "conveyor.exit_points whole"),
    f("conveyor_mapping.json:[].go_coordinates", Structure, EmittedOnly, "conveyor_mapping.cell, by identity"),
    f("conveyor_mapping.json:[].HAI_coordinates", Structure, EmittedOnly, "conveyor_mapping.vendor_coordinates — UNDEFINED in every dump"),
    f("conveyor_mapping.json:[].conveyor_coordinates.conveyor_id", Structure, EmittedOnly, "conveyor_mapping.conveyor_coordinates.conveyor, the conveyor by identity, and half the row's lookup key"),
    f("conveyor_mapping.json:[].conveyor_coordinates.step_id", Structure, EmittedOnly, "external_ids.mapcreator_step_id, half the row's lookup key: a handle into the conveyor's kept steps"),
    f("conveyor_mapping.json:[].location_type", Structure, EmittedOnly, "conveyor_mapping.location_type"),
    f("conveyor_mapping.json:[].pps_id", Structure, EmittedOnly, "conveyor_mapping.station, by identity"),
    // ---- the five empty files ----------------------------------------------
    f("elevator.json", Structure, LoadBearing, "mapcreator.export.files — the file's presence and its empty form are carried, so the export writes `[]` back rather than leaving the file out. Multi-floor topology still has no core component, and that is the finding the emptiness names"),
    f("fire_emergency.json", Structure, EmittedOnly, "mapcreator.export.files, as elevator.json"),
    f("queue_data.json", Structure, NotApplicable, "mapcreator.export.files, as elevator.json"),
    f("opc_connection.json", Structure, EmittedOnly, "mapcreator.export.files, as elevator.json"),
    f("zone_controller.json", Structure, LoadBearing, "mapcreator.export.files, as elevator.json"),
    // ---- station interior --------------------------------------------------
    f("pps_bin.json:url", Structure, NotApplicable, "mapcreator.export.pps_bin_envelope.url — a request envelope field, not model data, and carried only because the export has to write the file back byte for byte"),
    f("pps_bin.json:type", Structure, NotApplicable, "mapcreator.export.pps_bin_envelope.type, as url"),
    f("pps_bin.json:header.accept", Structure, NotApplicable, "mapcreator.export.pps_bin_envelope.header, as url"),
    f("pps_bin.json:header.content-type", Structure, NotApplicable, "mapcreator.export.pps_bin_envelope.header, as url"),
    f("pps_bin.json:data[].bin_id", Data, LoadBearing, "external_ids.mapcreator_bin_id, half the bin's lookup key"),
    f("pps_bin.json:data[].pps_id", Structure, LoadBearing, "pps_bin.station, by identity, which the export writes the station's id back from; external_ids.mapcreator_of_pps beside it is half the bin's lookup key"),
    f("pps_bin.json:data[].position", Structure, LoadBearing, "pps_bin.wall_position — the [row, col] in the wall, a place in the wall rather than a length"),
    f("pps_bin.json:data[].coordinate", Structure, LoadBearing, "pps_bin.station_local_coordinate — station-local centimetres, carried as stated (the units differ from everything else in the export, which is a finding rather than a conversion)"),
    f("pps_bin.json:data[].direction", Structure, LoadBearing, "pps_bin.direction"),
    f("pps_bin.json:data[].group_id", Structure, LoadBearing, "pps_bin.group_id"),
    f("pps_bin.json:data[].type", Structure, LoadBearing, "pps_bin.bin_type, the bin type entity by identity, which the export writes the type id back from"),
    f("pps_bin.json:data[].allowed_tags", Structure, LoadBearing, "pps_bin.allowed_tags"),
    f("pps_bin.json:data[].front_seat_name", Structure, LoadBearing, "pps_bin.front_seat, the seat entity by identity, which the export writes the seat name back from"),
    f("pps_bin.json:data[].back_seat_name", Structure, LoadBearing, "pps_bin.back_seat, as front_seat_name"),
    f("pps_bin.json:data[].status", Structure, NotApplicable, "pps_bin.runtime.status"),
    f("pps_bintype.json:url", Structure, NotApplicable, "mapcreator.export.pps_bin_type_envelope.url, as pps_bin.json's"),
    f("pps_bintype.json:type", Structure, NotApplicable, "mapcreator.export.pps_bin_type_envelope.type"),
    f("pps_bintype.json:header.accept", Structure, NotApplicable, "mapcreator.export.pps_bin_type_envelope.header"),
    f("pps_bintype.json:header.content-type", Structure, NotApplicable, "mapcreator.export.pps_bin_type_envelope.header"),
    f("pps_bintype.json:data[].type_id", Data, LoadBearing, "external_ids.mapcreator_bin_type_id, the type's lookup key"),
    f("pps_bintype.json:data[].tag", Structure, LoadBearing, "pps_bin_type.tag"),
    f("pps_bintype.json:data[].dimension.height", Structure, LoadBearing, "pps_bin_type.dimension, kept whole in the centimetres the file states"),
    f("pps_bintype.json:data[].dimension.length", Structure, LoadBearing, "as dimension.height"),
    f("pps_bintype.json:data[].dimension.breadth", Structure, LoadBearing, "as dimension.height"),
    f("pps_seat.json:pps_seats[].pps_id", Structure, LoadBearing, "pps_seat.station, by identity, which the export writes the station's id back from; external_ids.mapcreator_of_pps beside it is half the seat's lookup key"),
    f("pps_seat.json:pps_seats[].seat_name", Data, LoadBearing, "external_ids.mapcreator_seat_name, half the seat's lookup key"),
    f("pps_seat.json:pps_seats[].seat_type", Structure, LoadBearing, "pps_seat.seat_type"),
    f("pps_seat.json:pps_seats[].seat_data.key", Structure, LoadBearing, "pps_seat.seat_data, kept whole"),
    f("pptl.json:[].pptls[].peripheral_id", Data, LoadBearing, "external_ids.mapcreator_peripheral_id, the module's lookup key"),
    f("pptl.json:[].pptls[].peripheral_type", Structure, LoadBearing, "pptl.peripheral_type"),
    f("pptl.json:[].pptls[].barcode", Structure, LoadBearing, "pptl.barcode"),
    f("pptl.json:[].pptls[].pps_bin_id", Structure, LoadBearing, "pptl.bin, the bin entity by identity, which the export writes the bin id back from"),
    f("pptl.json:[].pptls[].pps_id", Structure, LoadBearing, "pptl.station, the station by identity, which the export writes the station's id back from"),
    // ---- the containers, so the overlay's own table is complete -------------
    f("pptl.json:[].pptls", Container, NotApplicable, "the device list; its grouping is rebuilt from the group place each device's formatting carries in import_formatting.mapcreator_pptl.group, and the places of the groups with no device from mapcreator.export.pptl_empty_groups"),
];

/// The overlay, indexed by path.
pub fn overlay() -> BTreeMap<&'static str, Field> {
    OVERRIDES.iter().map(|field| (field.path, *field)).collect()
}

/// The one-way importer's table with this adapter's overlay applied: what the round
/// trip actually does with each path.
pub fn table() -> BTreeMap<&'static str, Field> {
    let mut table = crate::format::coverage::table();
    table.extend(overlay());
    table
}

/// Every path the combined table still calls `Impedance`: a fact the protocol
/// genuinely cannot state even with the vocabulary's components in place.
///
/// The round trip does not lose these — a component of the vocabulary carries them
/// and the export writes them back — so what the list says is narrower and more
/// useful: a CORE consumer reading only the protocol's own vocabulary learns
/// nothing from them.
pub fn still_unrepresentable() -> Vec<Field> {
    table().into_values().filter(|field| field.disposition == Impedance).collect()
}

#[cfg(test)]
mod tests {
    use super::{overlay, still_unrepresentable, table, OVERRIDES};
    use crate::format::coverage::Disposition;

    #[test]
    fn no_path_is_overridden_twice() {
        assert_eq!(overlay().len(), OVERRIDES.len(), "a path appears twice in OVERRIDES");
    }

    #[test]
    fn every_override_says_where_the_field_went() {
        for field in OVERRIDES {
            assert!(!field.note.is_empty(), "{} has no note", field.path);
            assert!(
                field.path.ends_with(".json") || field.path.contains(".json:"),
                "{} is not a path into an export file",
                field.path
            );
        }
    }

    /// The overlay's whole purpose: the combined table calls far fewer paths
    /// impedance than the one-way importer's does. The numbers are pinned so a
    /// regression shows up as a number rather than as a feeling.
    #[test]
    fn the_overlay_resolves_most_of_the_one_way_importers_impedance() {
        let before = crate::format::coverage::table()
            .into_values()
            .filter(|f| f.disposition == Disposition::Impedance)
            .count();
        let after = still_unrepresentable().len();
        assert!(after < before, "the overlay resolved nothing: {before} before, {after} after");
        assert_eq!(before, 145, "the one-way importer's impedance count moved");
        assert_eq!(after, 39, "this adapter's remaining impedance count moved");
    }

    /// An override has to be an override of something, or a row for a path no
    /// table had. Both are fine; a typo in a path is not, and it looks exactly
    /// like the first.
    #[test]
    fn every_override_lands_on_a_path_the_combined_table_holds() {
        let combined = table();
        for field in OVERRIDES {
            assert!(combined.contains_key(field.path), "{} is in no table", field.path);
        }
    }
}
