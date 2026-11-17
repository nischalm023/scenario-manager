//! Within `write`, `delta` is A SLICE'S RECORDS AS THE FORMAT'S: the robots'
//! steps, the totes' births and moves, the relay cells' holds, the stations'
//! depths, the heat, the motion and the robots' state spans a slice states,
//! and the journal rows due with it, each translated once, reading nothing but
//! the slice, its spans and its rows.
//! The end block's tables are journal rows too, and `finish` states them
//! through the same `table_rows` and `rows_of`.

use desgraph::types::{CellId, WorldPos};
use desunits::{RobotId, SimDuration, SimTime, ToteId};
use rustc_hash::FxHashMap;
use worldview::container::payload::Opaque;
use worldview::frames::heat::{HeatDelta, HeatLayerDelta};
use worldview::frames::motion::{MotionDelta, MotionProfile, MotionSpan as SpanRecord, RobotMotion};
use worldview::frames::playback::{
    MoverSteps, PlaybackDelta, Position, RelayTransition, StationChange, Step, ToteContents, ToteCreation,
    ToteLocation, ToteTransit, ToteTransition,
};
use worldview::frames::robot_state::{RobotSpans, RobotStateDelta, StateSpan};
use worldview::frames::tables::{TableDescriptor, TableId, TableRows};

use super::RecordingError;
use crate::metrics::profile::DynamicGrid;
use crate::metrics::streams::LogStream;
use crate::metrics::worldview::journals;
use crate::metrics::worldview::summary::document;
use crate::metrics::worldview::wire::{designation_byte, milliradians, step_kind, tote_location, tote_type_byte};
use crate::observer::finality::recording::SliceRows;
use crate::observer::robot_state::RobotSliceSpans;
use crate::observer::station_diag::StationDiagRecord;
use crate::observer::time_slice::{BotLocation, TimeSliceData};

/// The coordinates a recording states for a point the run model holds in
/// f64 millimetres: whole millimetres, halves away from zero.
///
/// The format carries no finer precision than this by ruling, so rounding
/// here is what every consumer then reads; the reader's own inverse widens
/// these back into the run model's f64 and gets the same number every time.
///
/// A recording states each coordinate as a 32-bit integer, so a run model
/// holding one that is not a number, or one past what that integer reaches,
/// has nothing to be written down as. `as i32` would turn the first into zero
/// and clamp the second to the far edge of the floor, putting a robot
/// somewhere it never was and saying nothing; the robot and the instant are
/// named instead and the write stops.
pub(super) fn whole_mm(pos: WorldPos, z_mm: f64, robot: u32, at_ms: u64) -> Result<Position, RecordingError> {
    let axis = |value: f64, axis: &'static str| match value.round() {
        rounded if rounded >= f64::from(i32::MIN) && rounded <= f64::from(i32::MAX) => Ok(rounded as i32),
        _ => Err(RecordingError::Coordinate { robot: RobotId(robot), at: SimTime::ms(at_ms), axis, value }),
    };
    Ok(Position::new(axis(pos.x_mm, "x")?, axis(pos.y_mm, "y")?, axis(z_mm, "z")?))
}

/// One slice's own records as the format's.
///
/// `next` is the slice after this one as B leaves it, which is where the
/// cell a mover is left in comes from: the observer holds a robot's cell
/// back until it reaches one, so the slice that follows a cut leg is the
/// only place that says which cell the mover was still in when this one
/// closed, and the leg was reported before B passed the boundary.
pub(super) fn playback_delta(
    slice: &TimeSliceData,
    next: Option<&TimeSliceData>,
    reach: Reach,
    headings: Vec<(RobotId, Option<i32>)>,
) -> Result<PlaybackDelta, RecordingError> {
    let mut movers: Vec<MoverSteps> = slice
        .bot_locations
        .iter()
        .filter(|(_, location)| !location.steps.is_empty())
        .map(|(&bot, location)| {
            Ok(MoverSteps {
                robot: RobotId(bot),
                start_heading_mrad: location.start_heading_rad.map(milliradians),
                steps: location
                    .steps
                    .iter()
                    .map(|step| {
                        Ok(Step {
                            at: whole_mm(step.pos, step.z_mm, bot, slice.start_ms)?,
                            dt: SimDuration::ms(step.delta_ms),
                            kind: step_kind(step.kind),
                            cell: CellId(step.cell_id),
                        })
                    })
                    .collect::<Result<Vec<_>, RecordingError>>()?,
                cell: match next.and_then(|next| next.bot_locations.get(&bot)) {
                    Some(found) => CellId(found.start_cell),
                    None => CellId(location.steps[location.steps.len() - 1].cell_id),
                },
            })
        })
        .collect::<Result<Vec<_>, RecordingError>>()?;
    movers.sort_by_key(|mover| mover.robot.0);

    let tote_creations = slice
        .tote_state
        .creations
        .iter()
        .map(|birth| {
            Ok(ToteCreation {
                offset: offset_in_slice("tote", u64::from(birth.tote_id), birth.offset_ms, slice, reach)?,
                tote: ToteId(birth.tote_id),
                tote_type: tote_type_byte(birth.tote_type),
                designation: designation_byte(birth.designation),
                location: birth_place(slice, birth.offset_ms, birth.tote_id)?,
            })
        })
        .collect::<Result<Vec<_>, RecordingError>>()?;

    Ok(PlaybackDelta {
        movers,
        headings,
        tote_creations,
        tote_transitions: slice
            .tote_state
            .transitions
            .iter()
            .map(|(offset, tote, state)| {
                Ok(ToteTransition {
                    offset: offset_in_slice("tote", u64::from(*tote), *offset, slice, reach)?,
                    tote: ToteId(*tote),
                    location: tote_location(state),
                })
            })
            .collect::<Result<Vec<_>, RecordingError>>()?,
        tote_transits: slice
            .tote_state
            .transits
            .iter()
            .map(|transit| ToteTransit {
                tote: ToteId(transit.tote_id),
                from_cell: CellId(transit.from_cell),
                to_cell: CellId(transit.to_cell),
                start: SimTime::ms(transit.start_ms),
                end: SimTime::ms(transit.end_ms),
            })
            .collect(),
        tote_contents: slice
            .tote_state
            .contents
            .iter()
            .map(|(offset, tote, units)| {
                Ok(ToteContents {
                    offset: offset_in_slice("tote", u64::from(*tote), *offset, slice, reach)?,
                    tote: ToteId(*tote),
                    units: *units,
                })
            })
            .collect::<Result<Vec<_>, RecordingError>>()?,
        relay_transitions: slice
            .relay_state
            .transitions
            .iter()
            .map(|(cell, offset, reserved)| {
                Ok(RelayTransition {
                    cell: CellId(*cell),
                    offset: offset_in_slice("relay cell", u64::from(*cell), *offset, slice, reach)?,
                    reserved: *reserved,
                })
            })
            .collect::<Result<Vec<_>, RecordingError>>()?,
        station_changes: station_changes(&slice.station_diag, slice, reach)?,
        station_display: display_document(&slice.station_diag)?,
    })
}

fn table_id(tables: &[TableDescriptor], name: &str) -> Option<TableId> {
    tables.iter().find(|table| table.name == name).map(|table| table.id)
}

/// One frame's journal rows as the tables the format carries. A table with
/// no rows there has no entry at all.
pub(super) fn table_rows(rows: &SliceRows, tables: &[TableDescriptor]) -> Result<Vec<TableRows>, RecordingError> {
    let chunks = journals::chunks(rows).map_err(RecordingError::Journals)?;
    chunks.iter().map(|stream| rows_of(stream, tables)).collect()
}

/// One stream's entries as the table rows the format carries. The two agree
/// byte for byte: both are positional rows back to back, and both count them.
pub(super) fn rows_of(stream: &LogStream, tables: &[TableDescriptor]) -> Result<TableRows, RecordingError> {
    let table = table_id(tables, &stream.descriptor.id)
        .ok_or_else(|| RecordingError::UndeclaredTable(stream.descriptor.id.clone()))?;
    let row_count = u32::try_from(stream.entries.entry_count)
        .map_err(|_| RecordingError::TooManyRows(stream.descriptor.id.clone()))?;
    Ok(TableRows { table, row_count, rows: stream.entries.bytes.clone() })
}

/// The robot state section of a slice's delta: each robot the slice names, its mark and its spans.
pub(super) fn robot_state_delta(named: &[RobotSliceSpans]) -> RobotStateDelta {
    let stated = named.iter().map(|robot| RobotSpans {
        robot: robot.robot,
        continues: robot.continues,
        spans: robot.spans.iter().map(|&(dt, state)| StateSpan { dt, state }).collect(),
    });
    RobotStateDelta(stated.collect())
}

pub(super) fn heat_delta(grid: &DynamicGrid) -> HeatDelta {
    let layers = grid
        .iter()
        .filter(|(_, counts)| !counts.is_empty())
        .map(|(name, counts)| HeatLayerDelta {
            name: name.clone(),
            increments: counts.iter().map(|(&cell, &count)| (CellId(cell), count)).collect(),
        })
        .collect();
    HeatDelta { layers }
}

/// Every robot that moved in the slice, with the speed profile the planner
/// drove each move by — what the traffic overlay bins and a step list cannot
/// say.
pub(super) fn motion_delta(bot_locations: &FxHashMap<u32, BotLocation>) -> MotionDelta {
    let robots = bot_locations
        .iter()
        .filter_map(|(&bot, location)| {
            let spans = location.motion.as_ref().filter(|spans| !spans.is_empty())?;
            Some(RobotMotion {
                robot: RobotId(bot),
                spans: spans
                    .iter()
                    .map(|span| SpanRecord {
                        source: CellId(span.source),
                        destination: CellId(span.destination),
                        start: SimTime::ms(span.start_ms),
                        end: SimTime::ms(span.end_ms),
                        distance_mm: span.distance_mm,
                        profile: span.profile.map(|p| MotionProfile {
                            entry_velocity_mm_s: p.entry_velocity_mm_s,
                            exit_velocity_mm_s: p.exit_velocity_mm_s,
                            peak_velocity_mm_s: p.peak_velocity_mm_s,
                            accel_time_s: p.accel_time_s,
                            cruise_time_s: p.cruise_time_s,
                            decel_time_s: p.decel_time_s,
                            accel_distance_mm: p.accel_distance_mm,
                            cruise_distance_mm: p.cruise_distance_mm,
                            decel_distance_mm: p.decel_distance_mm,
                            total_time_s: p.total_time_s,
                            total_distance_mm: p.total_distance_mm,
                            acceleration_mm_s2: p.acceleration_mm_s2,
                            deceleration_mm_s2: p.deceleration_mm_s2,
                        }),
                    })
                    .collect(),
            })
        })
        .collect();
    MotionDelta(robots)
}

/// An event's offset into its slice, refused when it falls outside the slice
/// it is being written into.
///
/// An offset is measured from the slice's start and a reader applies it
/// there, so one past the slice's end places the event in a slice that never
/// saw it — a tote moved minutes after the frame says, a station depth
/// applied to the wrong instant. The observer computes these by subtraction
/// and a wrong one is its arithmetic, not the file's, so it is named here
/// rather than carried.
pub(super) fn offset_in_slice(
    what: &'static str,
    which: u64,
    offset_ms: u64,
    slice: &TimeSliceData,
    reach: Reach,
) -> Result<SimDuration, RecordingError> {
    let offset = SimDuration::ms(offset_ms);
    match offset_ms < reach.width_ms || (reach.through_its_end && offset_ms == reach.width_ms) {
        true => Ok(offset),
        false => Err(RecordingError::PastItsSlice {
            what,
            which,
            offset,
            slice_start: SimTime::ms(slice.start_ms),
            width: SimDuration::ms(reach.width_ms),
        }),
    }
}

/// How far into a slice its records may lie: before the slice's end, or, in the run's last slice, which the stop cuts
/// or ends, up to and including the stop, the instant the run's last facts are stamped at, which no slice after it
/// states.
#[derive(Debug, Clone, Copy)]
pub(super) struct Reach {
    /// The slice's own width: the slice width, or less for the slice the stop cuts.
    pub(super) width_ms: u64,
    pub(super) through_its_end: bool,
}

/// Where a container stands the moment it is born: the move the observer
/// reports for it at that same instant, which every birth has because the
/// observer emits one with it.
pub(super) fn birth_place(slice: &TimeSliceData, offset_ms: u64, tote_id: u32) -> Result<ToteLocation, RecordingError> {
    slice
        .tote_state
        .transitions
        .iter()
        .find(|(offset, tote, _)| *offset == offset_ms && *tote == tote_id)
        .map(|(_, _, state)| tote_location(state))
        .ok_or(RecordingError::BirthWithNoPlace {
            tote: tote_id,
            offset: SimDuration::ms(offset_ms),
            slice_start: SimTime::ms(slice.start_ms),
        })
}

/// A depth record states the absolute instant it was taken; a delta states
/// the offset into the slice it closes, so the two are one subtraction apart.
///
/// The order is by offset and by nothing else, kept STABLE: two depths taken
/// at one instant happened in the order the observer emitted them, and a
/// reader applies the last one it reads. Breaking the tie by station id would
/// make the file say a different station was served last.
pub(super) fn station_changes(
    diag: &StationDiagRecord,
    slice: &TimeSliceData,
    reach: Reach,
) -> Result<Vec<StationChange>, RecordingError> {
    let mut changes: Vec<StationChange> = diag
        .pipeline_depths
        .iter()
        .map(|record| {
            let offset_ms = record.at.as_ms().saturating_sub(slice.start_ms);
            Ok(StationChange {
                offset: offset_in_slice("station", u64::from(record.station_id.0), offset_ms, slice, reach)?,
                station: record.station_id,
                pipeline_depth: record.depth,
            })
        })
        .collect::<Result<Vec<_>, RecordingError>>()?;
    changes.sort_by_key(|change| change.offset.as_ms());
    Ok(changes)
}

/// The station panel's own records, carried through for the workbook. An
/// empty one costs nothing rather than a document of defaults.
pub(super) fn display_document(diag: &StationDiagRecord) -> Result<Opaque, RecordingError> {
    match diag.is_empty() {
        true => Ok(Opaque::default()),
        false => document(diag, "station diagnostics"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A coordinate a recording cannot state, and what the refusal says. Each
    /// of these turns into a plausible-looking integer under `as i32` — a NaN
    /// into 0, an infinity or an out-of-range value into the far edge of the
    /// floor — so the robot would be drawn somewhere it never was with nothing
    /// said about it.
    #[test]
    fn a_coordinate_no_recording_can_state_names_the_robot_and_the_instant() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 3e9, -3e9] {
            let refused = whole_mm(WorldPos { x_mm: bad, y_mm: 0.0 }, 0.0, 7, 120_000)
                .expect_err("{bad} is not a whole millimetre a recording can state");
            assert!(
                matches!(refused, RecordingError::Coordinate { robot: RobotId(7), at, axis: "x", .. }
                    if at == SimTime::ms(120_000)),
                "the refusal names the robot, the instant and the axis: {refused}"
            );
        }
        for bad in [f64::NAN, 3e9] {
            assert!(whole_mm(WorldPos { x_mm: 0.0, y_mm: bad }, 0.0, 7, 0).is_err(), "y is checked too");
            assert!(whole_mm(WorldPos::default(), bad, 7, 0).is_err(), "and the height");
        }
    }

    /// In the slice the run's stop cuts, 4 s into it, a change at the stop is stated and one past it is refused: the
    /// observer leaves those out, and one reaching the writer is its arithmetic gone wrong, named rather than carried.
    #[test]
    fn the_slice_the_stop_cuts_states_a_change_at_the_stop_and_refuses_one_past_it() {
        let slice = TimeSliceData::default();
        let cut = Reach { width_ms: 4_000, through_its_end: true };
        assert!(offset_in_slice("relay cell", 7, 4_000, &slice, cut).is_ok(), "a change at the stop is stated");
        let refused = offset_in_slice("relay cell", 7, 6_000, &slice, cut).expect_err("one past the stop is refused");
        let said = refused.to_string();
        assert!(said.contains("relay cell 7 is stated 6s into the slice at 0s, which is 4s wide"), "{said}");
    }

    /// The edges that ARE statable: a coordinate that rounds exactly onto the
    /// ends of the integer a recording carries.
    #[test]
    fn the_far_corners_of_the_integer_a_position_is_stated_in_still_write() {
        let at = whole_mm(WorldPos { x_mm: f64::from(i32::MAX), y_mm: f64::from(i32::MIN) }, 0.0, 1, 0)
            .expect("both ends of the range are whole millimetres");
        assert_eq!((at.x_mm, at.y_mm), (i32::MAX, i32::MIN));
    }
}
