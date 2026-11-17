//! Reconstruct container positions, types and physical unit counts at the
//! cursor. Live slices and saved profiles use the same timed records.
//!
//! It is also where the simulator's words for a container become the
//! renderer's: the two name the same six places and the same three boxes, and
//! [`located`] and the two converters below are the only place they meet.

use desgraph::CellId;
use dessim::model::{ToteDesignation, ToteType};
use dessim::observer::time_slice::{ToteStateRecord, ToteTransitRecord, ToteVisualState};
use desunits::{RobotId, SimTime, ToteId};
use rustc_hash::FxHashMap;

use worldview_render::{ContainerKind, Designation, ToteAppearance, ToteLocation, ToteTransit};

/// Where a container is, in the renderer's terms.
pub(super) fn located(state: &ToteVisualState) -> ToteLocation {
    match *state {
        ToteVisualState::AtCell { cell_id } => ToteLocation::AtCell(CellId(cell_id)),
        ToteVisualState::Extracted => ToteLocation::Extracted,
        ToteVisualState::InRack { cell_id, level } => ToteLocation::InRack { cell: CellId(cell_id), level },
        ToteVisualState::AtRelay { cell_id } => ToteLocation::AtRelay(CellId(cell_id)),
        ToteVisualState::OnBot { bot_id } => ToteLocation::OnBot(RobotId(bot_id)),
        ToteVisualState::OnConveyor { cell_id } => ToteLocation::OnConveyor(CellId(cell_id)),
    }
}

/// One conveyor travel, in the renderer's terms. The recording states instants
/// where the slice record states milliseconds, and nothing else differs.
pub(super) fn travelling(travel: &ToteTransitRecord) -> ToteTransit {
    ToteTransit {
        tote: ToteId(travel.tote_id),
        from_cell: CellId(travel.from_cell),
        to_cell: CellId(travel.to_cell),
        start: SimTime::ms(travel.start_ms),
        end: SimTime::ms(travel.end_ms),
    }
}

/// Which physical box a container is, in the renderer's terms.
fn container_kind(tote_type: ToteType) -> ContainerKind {
    match tote_type {
        ToteType::Tote => ContainerKind::Tote,
        ToteType::Tray => ContainerKind::Tray,
        ToteType::Carton => ContainerKind::Carton,
    }
}

/// Which flow a container belongs to, in the renderer's terms.
fn designation(designation: ToteDesignation) -> Designation {
    match designation {
        ToteDesignation::Inventory => Designation::Inventory,
        ToteDesignation::Order => Designation::Order,
    }
}

#[cfg(test)]
fn positions_at(record: &ToteStateRecord, offset_ms: u64) -> Vec<(u32, ToteVisualState)> {
    ToteStateRecord::positions_at([(0, record)], offset_ms)
}

pub(super) fn appearances_at<'a>(
    slices: impl IntoIterator<Item = (u64, &'a ToteStateRecord)>,
    cursor_ms: u64,
) -> FxHashMap<u32, ToteAppearance> {
    let mut appearances = FxHashMap::default();
    for (start_ms, record) in slices {
        if start_ms > cursor_ms {
            continue;
        }
        // Legacy profiles describe reusable order totes once, on slice zero.
        for &id in &record.order_totes {
            appearances.entry(id).or_insert_with(ToteAppearance::default).designation = Designation::Order;
        }
        for created in &record.creations {
            if created.offset_ms <= cursor_ms - start_ms {
                let appearance = appearances.entry(created.tote_id).or_insert_with(ToteAppearance::default);
                appearance.container = container_kind(created.tote_type);
                appearance.designation = designation(created.designation);
            }
        }
        for &(at, tote_id, units) in &record.contents {
            if at <= cursor_ms - start_ms {
                appearances.entry(tote_id).or_insert_with(ToteAppearance::default).contents_units = units;
            }
        }
    }
    appearances
}

#[cfg(test)]
mod tests {
    use super::*;
    use dessim::observer::time_slice::ToteCreationRecord;

    #[test]
    fn legacy_profiles_keep_order_tote_designation() {
        let record: ToteStateRecord = serde_json::from_str(
            r#"{"start_states":[[17,{"OnBot":{"bot_id":3}}]],"transitions":[],"order_totes":[17]}"#,
        )
        .unwrap();
        let appearances = appearances_at([(0, &record)], 10_000);
        assert_eq!(appearances[&17], ToteAppearance { designation: Designation::Order, ..Default::default() });
        assert_eq!(positions_at(&record, 0), [(17, ToteVisualState::OnBot { bot_id: 3 })]);
    }

    #[test]
    fn late_carton_creation_survives_recording_and_reverse_playback() {
        let initial = ToteStateRecord::default();
        let created = ToteStateRecord {
            creations: vec![ToteCreationRecord {
                offset_ms: 500,
                tote_id: 42,
                tote_type: ToteType::Carton,
                designation: ToteDesignation::Order,
            }],
            transitions: vec![
                (500, 42, ToteVisualState::AtCell { cell_id: 20 }),
                (750, 42, ToteVisualState::OnBot { bot_id: 7 }),
            ],
            ..Default::default()
        };
        let replayed: ToteStateRecord = serde_json::from_str(&serde_json::to_string(&created).unwrap()).unwrap();
        let later = ToteStateRecord {
            start_states: vec![(42, ToteVisualState::OnBot { bot_id: 7 })],
            transitions: vec![(500, 42, ToteVisualState::Extracted)],
            ..Default::default()
        };
        for record in [&created, &replayed] {
            for cursor in [1499, 1500, 1750, 2100, 1499] {
                let appearances = appearances_at([(0, &initial), (1000, record), (2000, &later)], cursor);
                if cursor < 1500 {
                    assert!(!appearances.contains_key(&42));
                    assert!(positions_at(record, cursor - 1000).is_empty());
                } else {
                    assert_eq!(appearances[&42].container, ContainerKind::Carton);
                    assert_eq!(appearances[&42].designation, Designation::Order);
                }
            }
            assert_eq!(positions_at(record, 500), [(42, ToteVisualState::AtCell { cell_id: 20 })]);
            assert_eq!(positions_at(record, 750), [(42, ToteVisualState::OnBot { bot_id: 7 })]);
        }
        assert_eq!(positions_at(&later, 499), [(42, ToteVisualState::OnBot { bot_id: 7 })]);
        assert_eq!(positions_at(&later, 500), [(42, ToteVisualState::Extracted)]);
        assert_eq!(positions_at(&later, 499), [(42, ToteVisualState::OnBot { bot_id: 7 })]);
    }

    #[test]
    fn explicit_startup_tray_type_overrides_legacy_order_roster() {
        let record = ToteStateRecord {
            order_totes: vec![17],
            creations: vec![ToteCreationRecord {
                offset_ms: 0,
                tote_id: 17,
                tote_type: ToteType::Tray,
                designation: ToteDesignation::Order,
            }],
            ..Default::default()
        };
        assert_eq!(appearances_at([(0, &record)], 0)[&17].container, ContainerKind::Tray);
    }

    #[test]
    fn physical_units_follow_transfer_extraction_and_reverse_playback() {
        let initial =
            ToteStateRecord { contents: vec![(0, 1, 7), (0, 42, 0)], order_totes: vec![42], ..Default::default() };
        let changes = ToteStateRecord {
            contents: vec![(100, 1, 5), (100, 42, 2), (300, 42, 0), (500, 1, 9)],
            ..Default::default()
        };
        let replayed: ToteStateRecord = serde_json::from_str(&serde_json::to_string(&changes).unwrap()).unwrap();
        for record in [&changes, &replayed] {
            for (cursor, inventory, order) in [(1099, 7, 0), (1100, 5, 2), (1300, 5, 0), (1500, 9, 0), (1100, 5, 2)] {
                let appearances = appearances_at([(0, &initial), (1000, record)], cursor);
                assert_eq!(appearances[&1].contents_units, inventory, "inventory at {cursor}");
                assert_eq!(appearances[&42].contents_units, order, "order container at {cursor}");
                assert_eq!(appearances[&42].designation, Designation::Order);
            }
        }
    }
}
