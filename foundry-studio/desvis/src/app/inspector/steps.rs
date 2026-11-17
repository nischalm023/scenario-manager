//! Run-length aggregation of committed bot steps for the inspector's "next
//! waypoints" list: consecutive same-direction Moves collapse into one row,
//! consecutive Waits/Rotates coalesce with summed durations. Pure display
//! aggregation over existing steps — no new data.
//!
//! Limitation: `BotStepKind` has only Move/Wait/Rotate — a station/present
//! dwell is not distinguishable from any other Wait in the step data, so
//! dwells render as `Wait (Xs)` rather than a separate `Dwell` row.

use dessim::observer::time_slice::BotStepKind;

/// One step prepared for aggregation: its kind, destination cell, duration,
/// and the destination's centre in millimetres (`None` when the cell has no
/// position — breaks a Move run rather than guessing a direction).
pub(super) struct StepInfo {
    pub kind: BotStepKind,
    pub cell: u32,
    pub duration_ms: u64,
    pub pos_mm: Option<[f64; 2]>,
}

/// Cardinal sign of a millimetre delta. The deadband absorbs sub-millimetre
/// centre noise so a straight lane on a non-uniform map still reads straight.
fn direction_of(from: [f64; 2], to: [f64; 2]) -> (i32, i32) {
    let sign = |d: f64| {
        if d > 0.5 {
            1
        } else if d < -0.5 {
            -1
        } else {
            0
        }
    };
    (sign(to[0] - from[0]), sign(to[1] - from[1]))
}

/// One aggregated display row.
#[derive(Debug, PartialEq)]
pub(super) enum WaypointRun {
    /// A straight run of Moves: first→last destination, cell count, total time.
    Move { from_cell: u32, to_cell: u32, cells: usize, duration_ms: u64 },
    /// Coalesced consecutive Waits at one cell.
    Wait { duration_ms: u64 },
    /// Coalesced consecutive Rotates at one cell.
    Rotate { duration_ms: u64 },
}

/// Aggregate `steps` (in order) into display runs. Move runs extend while the
/// direction between consecutive destinations stays constant; a direction
/// change, kind change, or missing position starts a new run. Consecutive
/// Waits (and Rotates) coalesce by summing durations.
pub(super) fn aggregate_steps(steps: &[StepInfo]) -> Vec<WaypointRun> {
    let mut runs: Vec<WaypointRun> = Vec::new();
    // For an open Move run: (last position, run direction — None until the
    // 2nd member fixes it).
    let mut move_context: Option<([f64; 2], Option<(i32, i32)>)> = None;

    for step in steps {
        match step.kind {
            BotStepKind::Move | BotStepKind::Reverse => {
                let extended = match (runs.last_mut(), move_context, step.pos_mm) {
                    (
                        Some(WaypointRun::Move { to_cell, cells, duration_ms, .. }),
                        Some((last_pos, run_direction)),
                        Some(pos),
                    ) => {
                        let direction = direction_of(last_pos, pos);
                        match run_direction {
                            // Direction locked: extend only if it matches.
                            Some(d) if d == direction => {
                                *to_cell = step.cell;
                                *cells += 1;
                                *duration_ms += step.duration_ms;
                                move_context = Some((pos, Some(d)));
                                true
                            }
                            // Second member fixes the run direction.
                            None => {
                                *to_cell = step.cell;
                                *cells += 1;
                                *duration_ms += step.duration_ms;
                                move_context = Some((pos, Some(direction)));
                                true
                            }
                            Some(_) => false,
                        }
                    }
                    _ => false,
                };
                if !extended {
                    runs.push(WaypointRun::Move {
                        from_cell: step.cell,
                        to_cell: step.cell,
                        cells: 1,
                        duration_ms: step.duration_ms,
                    });
                    move_context = step.pos_mm.map(|pos| (pos, None));
                }
            }
            BotStepKind::Wait => {
                move_context = None;
                if let Some(WaypointRun::Wait { duration_ms }) = runs.last_mut() {
                    *duration_ms += step.duration_ms;
                } else {
                    runs.push(WaypointRun::Wait { duration_ms: step.duration_ms });
                }
            }
            BotStepKind::Rotate => {
                move_context = None;
                if let Some(WaypointRun::Rotate { duration_ms }) = runs.last_mut() {
                    *duration_ms += step.duration_ms;
                } else {
                    runs.push(WaypointRun::Rotate { duration_ms: step.duration_ms });
                }
            }
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(kind: BotStepKind, cell: u32, duration_ms: u64, grid: (i32, i32)) -> StepInfo {
        // Tests speak in unit cells; a unit is 1000mm.
        StepInfo { kind, cell, duration_ms, pos_mm: Some([grid.0 as f64 * 1000.0, grid.1 as f64 * 1000.0]) }
    }

    #[test]
    fn straight_run_collapses() {
        // Four cells east in a line -> one Move row.
        let steps: Vec<StepInfo> = (0..4).map(|i| step(BotStepKind::Move, 100 + i, 500, (i as i32, 0))).collect();
        let runs = aggregate_steps(&steps);
        assert_eq!(runs, vec![WaypointRun::Move { from_cell: 100, to_cell: 103, cells: 4, duration_ms: 2000 }]);
    }

    #[test]
    fn direction_change_starts_new_run() {
        // East, east, then south -> two Move rows.
        let steps = vec![
            step(BotStepKind::Move, 1, 500, (0, 0)),
            step(BotStepKind::Move, 2, 500, (1, 0)),
            step(BotStepKind::Move, 3, 500, (2, 0)),
            step(BotStepKind::Move, 4, 500, (2, 1)),
            step(BotStepKind::Move, 5, 500, (2, 2)),
        ];
        let runs = aggregate_steps(&steps);
        assert_eq!(
            runs,
            vec![
                WaypointRun::Move { from_cell: 1, to_cell: 3, cells: 3, duration_ms: 1500 },
                WaypointRun::Move { from_cell: 4, to_cell: 5, cells: 2, duration_ms: 1000 },
            ]
        );
    }

    #[test]
    fn waits_coalesce_and_split_moves() {
        let steps = vec![
            step(BotStepKind::Move, 1, 500, (0, 0)),
            step(BotStepKind::Wait, 1, 10_000, (0, 0)),
            step(BotStepKind::Wait, 1, 20_000, (0, 0)),
            step(BotStepKind::Rotate, 1, 1_000, (0, 0)),
            step(BotStepKind::Move, 2, 500, (1, 0)),
        ];
        let runs = aggregate_steps(&steps);
        assert_eq!(
            runs,
            vec![
                WaypointRun::Move { from_cell: 1, to_cell: 1, cells: 1, duration_ms: 500 },
                WaypointRun::Wait { duration_ms: 30_000 },
                WaypointRun::Rotate { duration_ms: 1_000 },
                WaypointRun::Move { from_cell: 2, to_cell: 2, cells: 1, duration_ms: 500 },
            ]
        );
    }

    #[test]
    fn single_step_is_one_run() {
        let runs = aggregate_steps(&[step(BotStepKind::Move, 7, 400, (3, 3))]);
        assert_eq!(runs, vec![WaypointRun::Move { from_cell: 7, to_cell: 7, cells: 1, duration_ms: 400 }]);
    }
}
