//! Within `worldview`, `wire` is the VALUE TRANSLATION between the observer's
//! vocabulary and the format's records, in both directions.
//!
//! The two sides say the same things differently: the observer names a tote's
//! place with a struct variant carrying `u32` fields, a record with a kind
//! byte and two numbers; the observer records a chassis heading as radians in
//! an `f32`, a record as milliradians in an integer, because a record is
//! integers. Every one of those swaps lives here so the writing end and the
//! reading end cannot disagree about one.

use desgraph::types::CellId;
use desunits::RobotId;
use worldview::frames::playback::{StepKind, ToteLocation};
use worldview::frames::scalars::{AggregationClass, MetricDescriptor, MetricSemantics, MetricUnit};
use worldview::frames::tables::{ColumnEncoding, ColumnValueType, TableColumn};

use crate::metrics::profile::ScalarClass;
use crate::metrics::streams::{EntryColumn, EntryValueType};
use crate::model::{ToteDesignation, ToteType};
use crate::observer::time_slice::{BotStepKind, ToteVisualState};

/// What a metric key means: its class, as the flattening wrote the key with
/// it, and its unit, read off the key itself. A reading is a gauge, whose
/// slices a reader takes as they come; a sum is additive, whose slices it
/// adds up. The trailing `_ms`, `_ns` or `_mm` says what the number is
/// measured in, a key holding `_avg`, `_pct` or `_rate` with no such suffix
/// is a ratio, and anything else is a count.
pub fn metric_descriptor(name: &str, class: ScalarClass) -> MetricDescriptor {
    let unit = if name.ends_with("_ms") || name.ends_with("_ns") {
        MetricUnit::DurationMilliseconds
    } else if name.ends_with("_mm") {
        MetricUnit::DistanceMillimeters
    } else if ["_avg", "_pct", "_rate"].iter().any(|mark| name.contains(mark)) {
        MetricUnit::Ratio
    } else {
        MetricUnit::Count
    };
    let aggregation = match class {
        ScalarClass::Sum => AggregationClass::Additive,
        ScalarClass::Reading => AggregationClass::SnapshotGauge,
    };
    MetricDescriptor {
        name: name.to_owned(),
        label: name.to_owned(),
        description: String::new(),
        semantics: MetricSemantics { unit, denominator_description: None, aggregation },
    }
}

/// A journal's declared columns as the format states them. The descriptions
/// the observer's own column carries ride the producer document instead,
/// where the whole stream descriptor is kept for rebuilding.
pub fn table_columns(columns: &[EntryColumn]) -> Vec<TableColumn> {
    columns
        .iter()
        .map(|column| TableColumn {
            name: column.name.clone(),
            value_type: column_value_type(column.value_type),
            encoding: ColumnEncoding::Positional,
        })
        .collect()
}

pub fn column_value_type(value_type: EntryValueType) -> ColumnValueType {
    match value_type {
        EntryValueType::UInt => ColumnValueType::UInt,
        EntryValueType::Int => ColumnValueType::Int,
        EntryValueType::Float => ColumnValueType::Float,
        EntryValueType::Bool => ColumnValueType::Bool,
        EntryValueType::Text => ColumnValueType::Text,
        EntryValueType::SimTime => ColumnValueType::SimTime,
        EntryValueType::SimDuration => ColumnValueType::SimDuration,
    }
}

pub fn tote_location(state: &ToteVisualState) -> ToteLocation {
    match *state {
        ToteVisualState::AtCell { cell_id } => ToteLocation::AtCell(CellId(cell_id)),
        ToteVisualState::Extracted => ToteLocation::Extracted,
        ToteVisualState::InRack { cell_id, level } => ToteLocation::InRack { cell: CellId(cell_id), level },
        ToteVisualState::AtRelay { cell_id } => ToteLocation::AtRelay(CellId(cell_id)),
        ToteVisualState::OnBot { bot_id } => ToteLocation::OnBot(RobotId(bot_id)),
        ToteVisualState::OnConveyor { cell_id } => ToteLocation::OnConveyor(CellId(cell_id)),
    }
}

pub fn visual_state(location: ToteLocation) -> ToteVisualState {
    match location {
        ToteLocation::AtCell(cell) => ToteVisualState::AtCell { cell_id: cell.0 },
        ToteLocation::Extracted => ToteVisualState::Extracted,
        ToteLocation::InRack { cell, level } => ToteVisualState::InRack { cell_id: cell.0, level },
        ToteLocation::AtRelay(cell) => ToteVisualState::AtRelay { cell_id: cell.0 },
        ToteLocation::OnBot(robot) => ToteVisualState::OnBot { bot_id: robot.0 },
        ToteLocation::OnConveyor(cell) => ToteVisualState::OnConveyor { cell_id: cell.0 },
    }
}

pub fn step_kind(kind: BotStepKind) -> StepKind {
    match kind {
        BotStepKind::Move => StepKind::Move,
        BotStepKind::Reverse => StepKind::Reverse,
        BotStepKind::Wait => StepKind::Wait,
        BotStepKind::Rotate => StepKind::Rotate,
    }
}

pub fn bot_step_kind(kind: StepKind) -> BotStepKind {
    match kind {
        StepKind::Move => BotStepKind::Move,
        StepKind::Reverse => BotStepKind::Reverse,
        StepKind::Wait => BotStepKind::Wait,
        StepKind::Rotate => BotStepKind::Rotate,
    }
}

pub fn tote_type_byte(tote_type: ToteType) -> u8 {
    match tote_type {
        ToteType::Tray => 0,
        ToteType::Tote => 1,
        ToteType::Carton => 2,
    }
}

/// A byte no container type answers to is damage, not a tote: reading it as
/// one would put a carton on a tray shelf and say nothing.
pub fn tote_type_of(byte: u8) -> Result<ToteType, String> {
    match byte {
        0 => Ok(ToteType::Tray),
        1 => Ok(ToteType::Tote),
        2 => Ok(ToteType::Carton),
        other => Err(format!("container type byte {other} names no kind of container")),
    }
}

pub fn designation_byte(designation: ToteDesignation) -> u8 {
    match designation {
        ToteDesignation::Inventory => 0,
        ToteDesignation::Order => 1,
    }
}

/// The same for the pool a container belongs to, which is permanent and
/// decides which flow may ever use it.
pub fn designation_of(byte: u8) -> Result<ToteDesignation, String> {
    match byte {
        0 => Ok(ToteDesignation::Inventory),
        1 => Ok(ToteDesignation::Order),
        other => Err(format!("designation byte {other} names no pool")),
    }
}

/// Headings travel as milliradians because a record's fields are integers;
/// the observer records radians as `f32`.
pub fn milliradians(radians: f32) -> i32 {
    (f64::from(radians) * 1_000.0).round() as i32
}

pub fn radians(milliradians: i32) -> f32 {
    (f64::from(milliradians) / 1_000.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The class a key is written with decides whether a keyframe restates
    /// its newest reading or the sum of every slice, whatever the name
    /// suggests: one name is a gauge written as a reading and additive written
    /// as a sum, and an average written as a sum is additive.
    #[test]
    fn the_class_not_the_name_decides_the_aggregation() {
        for (name, class, aggregation) in [
            ("some_key", ScalarClass::Reading, AggregationClass::SnapshotGauge),
            ("some_key", ScalarClass::Sum, AggregationClass::Additive),
            ("vtm_forward_cycle_avg_ms", ScalarClass::Sum, AggregationClass::Additive),
        ] {
            assert_eq!(metric_descriptor(name, class).semantics.aggregation, aggregation, "{name} as {class:?}");
        }
    }

    /// The unit is the key's own, whatever its class: slots held are a count,
    /// not a ratio, though they are a reading.
    #[test]
    fn the_unit_is_read_off_the_key() {
        for (name, class, unit) in [
            ("vtm_forward_cycle_avg_ms", ScalarClass::Reading, MetricUnit::DurationMilliseconds),
            ("htm_travel_distance_mm", ScalarClass::Sum, MetricUnit::DistanceMillimeters),
            ("pathfinder_failure_rate", ScalarClass::Reading, MetricUnit::Ratio),
            ("rack_occupancy", ScalarClass::Reading, MetricUnit::Count),
            ("picks_completed", ScalarClass::Sum, MetricUnit::Count),
        ] {
            assert_eq!(metric_descriptor(name, class).semantics.unit, unit, "{name}");
        }
    }

    #[test]
    fn every_tote_place_and_step_kind_survives_the_round_trip() {
        for state in [
            ToteVisualState::AtCell { cell_id: 1 },
            ToteVisualState::Extracted,
            ToteVisualState::InRack { cell_id: 2, level: 5 },
            ToteVisualState::AtRelay { cell_id: 3 },
            ToteVisualState::OnBot { bot_id: 4 },
            ToteVisualState::OnConveyor { cell_id: 5 },
        ] {
            assert_eq!(visual_state(tote_location(&state)), state);
        }
        for kind in [BotStepKind::Move, BotStepKind::Reverse, BotStepKind::Wait, BotStepKind::Rotate] {
            assert_eq!(bot_step_kind(step_kind(kind)), kind);
        }
        assert_eq!(radians(milliradians(1.571)), 1.571);
        for tote_type in [ToteType::Tray, ToteType::Tote, ToteType::Carton] {
            assert_eq!(tote_type_of(tote_type_byte(tote_type)).expect("a kind"), tote_type);
        }
        for designation in [ToteDesignation::Inventory, ToteDesignation::Order] {
            assert_eq!(designation_of(designation_byte(designation)).expect("a pool"), designation);
        }
        assert!(tote_type_of(3).is_err(), "a byte no container type answers to is damage");
        assert!(designation_of(2).is_err(), "and so is one no pool answers to");
    }
}
