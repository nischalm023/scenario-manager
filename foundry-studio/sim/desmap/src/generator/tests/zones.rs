//! Zone validity, relay ownership, and rack block tests.

use std::collections::{BTreeMap, HashSet};

use super::super::*;

#[test]
fn relay_system_produces_relays() {
    let config = GeneratorConfig { system_type: SystemType::Relay, ..Default::default() };
    let map = generate(&config).unwrap();

    assert_eq!(map.system_type, SystemType::Relay);
    assert!(!map.relays.is_empty(), "RELAY system should have relay positions");

    for relay in &map.relays {
        assert_eq!(relay.htm_access, relay.position, "HTM access should be at relay position");
        assert_ne!(relay.vtm_access.anchor(), relay.position, "VTM access should differ from relay position");
    }
}

#[test]
fn every_generated_relay_block_has_an_outer_htm_lane() {
    let map = generate(&GeneratorConfig { system_type: SystemType::Relay, ..Default::default() }).unwrap();
    for block in &map.blocks {
        let relays: Vec<_> = map.relays.iter().filter(|relay| relay.block == block.id).collect();
        let rows = block.bottom_right.y - block.top_left.y + 1;
        assert_eq!(
            relays.len() as u32,
            rows * 2,
            "every row on both rack faces of generated dual block {:?} needs a usable terminal relay",
            block.id,
        );
        assert!(relays.iter().any(|relay| relay.side == BlockSide::Left));
        assert!(relays.iter().any(|relay| relay.side == BlockSide::Right));
    }
}

#[test]
fn rack_blocks_have_correct_cell_ids() {
    let config = GeneratorConfig { system_type: SystemType::Relay, ..Default::default() };
    let map = generate(&config).unwrap();

    for block in &map.blocks {
        for &cell_id in &block.rack_cells {
            let cell = map.graph.cell(cell_id).unwrap();
            assert!(
                cell.kind == CellKind::Storage || cell.kind == CellKind::Relay,
                "rack cell {:?} should be Storage or Relay, got {:?}",
                cell_id,
                cell.kind
            );
        }
    }
}

#[test]
fn zones_are_generated() {
    let map = generate(&GeneratorConfig::default()).unwrap();
    assert!(!map.zones.is_empty(), "should have at least one zone");
    for zone in &map.zones {
        assert!(!zone.cells.is_empty(), "zone {:?} has no cells", zone.id);
    }
}

#[test]
fn all_cell_zone_ids_are_valid() {
    let map = generate(&GeneratorConfig::default()).unwrap();
    for cell in map.graph.cells() {
        assert!(
            map.zone(cell.zone).is_some(),
            "cell {:?} has zone {:?} which doesn't exist in map.zones",
            cell.id,
            cell.zone
        );
    }
}

#[test]
fn all_cell_zone_ids_are_valid_relay() {
    let config = GeneratorConfig { system_type: SystemType::Relay, ..Default::default() };
    let map = generate(&config).unwrap();
    for cell in map.graph.cells() {
        assert!(
            map.zone(cell.zone).is_some(),
            "cell {:?} has zone {:?} which doesn't exist in map.zones (relay)",
            cell.id,
            cell.zone
        );
    }
}

#[test]
fn zone_zero_exists_and_covers_non_aisle_cells() {
    let map = generate(&GeneratorConfig::default()).unwrap();
    let zone_0 = map.zone(ZoneId(0));
    assert!(zone_0.is_some(), "zone 0 must exist");

    let z0_set: HashSet<CellId> = zone_0.unwrap().cells.iter().copied().collect();
    for cell in map.graph.cells() {
        if cell.kind == CellKind::Highway || cell.kind == CellKind::StationArea {
            assert!(z0_set.contains(&cell.id), "cell {:?} (kind {:?}) should be in zone 0", cell.id, cell.kind);
        }
    }
}

#[test]
fn storage_cells_are_in_non_zero_zones() {
    let map = generate(&GeneratorConfig::default()).unwrap();
    if !map.blocks.is_empty() {
        for cell in map.graph.cells() {
            if cell.kind == CellKind::Storage {
                assert_ne!(cell.zone, ZoneId(0), "storage cell {:?} should not be in default zone 0", cell.id);
            }
        }
    }
}

#[test]
fn relay_positions_are_subset_of_rack_cells() {
    let config = GeneratorConfig { system_type: SystemType::Relay, ..Default::default() };
    let map = generate(&config).unwrap();
    assert!(!map.relays.is_empty(), "test requires relay positions");

    for relay in &map.relays {
        let block = map.block(relay.block).unwrap();
        let rack_set: HashSet<CellId> = block.rack_cells.iter().copied().collect();
        assert!(
            rack_set.contains(&relay.position),
            "relay {:?} position {:?} should be in block {:?} rack_cells",
            relay.id,
            relay.position,
            relay.block
        );
    }
}

#[test]
fn relay_zone_ownership_is_consistent() {
    let config = GeneratorConfig { system_type: SystemType::Relay, ..Default::default() };
    let map = generate(&config).unwrap();

    for zone in &map.zones {
        let zone_cell_set: HashSet<CellId> = zone.cells.iter().copied().collect();
        for &relay_id in &zone.relays {
            let relay = map.relay(relay_id).unwrap();
            assert!(
                zone_cell_set.contains(&relay.vtm_access.anchor()),
                "relay {:?} vtm_access {:?} is not in zone {:?}'s cells",
                relay_id,
                relay.vtm_access,
                zone.id
            );
        }
    }

    let mut relay_zone_count: BTreeMap<RelayId, u32> = BTreeMap::new();
    for zone in &map.zones {
        for &relay_id in &zone.relays {
            *relay_zone_count.entry(relay_id).or_default() += 1;
        }
    }
    for relay in &map.relays {
        let count = relay_zone_count.get(&relay.id).copied().unwrap_or(0);
        assert_eq!(count, 1, "relay {:?} should appear in exactly 1 zone, found {}", relay.id, count);
    }
}
