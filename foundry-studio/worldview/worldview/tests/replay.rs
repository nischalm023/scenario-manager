//! The property the whole format rests on: reaching an instant by seeking to
//! it gives exactly what the run did at that instant.
//!
//! Two ways of checking it, because they fail differently. Against the
//! builder's own truth — it chose what happened and kept the answer — a
//! mistake in applying a delta shows up as a wrong position. Against a replay
//! from the first frame, a mistake in which frames a seek reads shows up even
//! where both paths apply deltas the same way.

mod support;

use std::collections::BTreeMap;

use desgraph::types::CellId;
use desunits::{RobotId, SimDuration, SimTime, ToteId};
use worldview::container::frame::Tag;
use worldview::frames::heat::{HeatKeyframe, HeatLayer};
use worldview::frames::playback::{
    MoverSteps, PlaybackDelta, PlaybackKeyframe, Position, RobotClass, RobotRecord, Step, StepKind, ToteContents,
    ToteCreation, ToteLocation, ToteRecord, ToteTransition,
};
use worldview::frames::state::{PlaybackAt, RunState, state_at};
use worldview::frames::{Delta, Keyframe, WriteFrames, instant, stamp};
use worldview::{Descriptors, End, Payload, Reader, Scenario, Writer};

use support::vocabulary::{
    BuiltRun, RunShape, SLICE_WIDTH_MS, Where, build, class_of, hand_built_scenario, ordered, scenario_of,
    start_heading, whole_delta, whole_keyframe,
};

/// The long way to an instant: the run's FIRST keyframe and every delta since,
/// which is the reading a seek must agree with without doing any of it.
fn replay(bytes: &[u8], descriptors: &Descriptors, t: SimTime) -> (RunState, PlaybackAt) {
    let mut reader = Reader::from_bytes(bytes).expect("the run reads");
    let records = reader.index().records().to_vec();
    let keyframe = records.iter().find(|record| record.tag == Tag::Keyframe).expect("a run opens with a keyframe");
    let sections = reader.sections_at(keyframe).expect("the keyframe reads");
    let decoded = Keyframe::decode(&sections, keyframe.offset).expect("the keyframe decodes");
    let mut state = RunState::from_keyframe(instant(keyframe.t), &decoded, descriptors)
        .expect("the keyframe is one the run declared");

    let deltas: Vec<_> =
        records.iter().filter(|record| record.tag == Tag::Delta && record.t <= stamp(t)).copied().collect();
    let Some((last, whole)) = deltas.split_last() else {
        let reached = state.reached;
        let at = state.at(reached, t.max(reached), &Delta::default()).expect("no slice to read");
        return (state, at);
    };
    for (index, record) in whole.iter().enumerate() {
        let sections = reader.sections_at(record).expect("a delta reads");
        let delta = Delta::decode(&sections, record.offset).expect("a delta decodes");
        let ends = instant(whole.get(index + 1).unwrap_or(last).t);
        state.apply(instant(record.t), ends, &delta).expect("a delta applies");
    }
    let sections = reader.sections_at(last).expect("the last delta reads");
    let delta = Delta::decode(&sections, last.offset).expect("the last delta decodes");
    // The state stands at this slice's start, so what the slice states about
    // its start — the headings — is part of it, the same way a seek takes it.
    let begins = instant(last.t);
    state.enter_slice(begins, &delta).expect("the slice begins where the state stands");
    let at = state.at(begins, t.max(begins), &delta).expect("the slice holds the instant");
    (state, at)
}

fn seek(run: &BuiltRun, t: SimTime) -> (RunState, PlaybackAt) {
    state_at(&mut run.reader(), t).expect("the seek reads")
}

#[test]
fn seeking_to_a_boundary_gives_the_run_the_builder_meant() {
    for cadence in [1, 2, 7, 30] {
        let run = build(RunShape { cadence, slices: 31, ..RunShape::default() });
        for (slice, boundary) in run.boundaries.iter().enumerate() {
            let (state, _) = seek(&run, *boundary);
            run.truth[slice].check(&state);
        }
    }
}

#[test]
fn seeking_inside_a_slice_gives_the_places_the_builder_meant() {
    let run = build(RunShape { cadence: 5, slices: 23, ..RunShape::default() });
    for (slice, probes) in run.probes.iter().enumerate() {
        for probe in probes {
            let (_, at) = seek(&run, probe.at);
            let positions: BTreeMap<RobotId, Where> =
                at.robots.iter().map(|(robot, held)| (*robot, Where { at: held.at, cell: held.cell })).collect();
            assert_eq!(positions, probe.positions, "positions {} ms into slice {slice}", probe.at.as_ms());
            let totes: BTreeMap<ToteId, ToteLocation> =
                at.totes.iter().map(|(tote, location)| (*tote, *location)).collect();
            assert_eq!(totes, probe.totes, "tote places {} ms into slice {slice}", probe.at.as_ms());
        }
    }
}

#[test]
fn seeking_agrees_with_replaying_from_the_first_frame() {
    for (cadence, slices, seed) in [(1u32, 9u32, 3u64), (4, 21, 11), (30, 40, 19), (1_000, 12, 5)] {
        let run = build(RunShape { cadence, slices, seed, ..RunShape::default() });
        let probes =
            [SimTime::ZERO, SimTime::ms(SLICE_WIDTH_MS / 3), run.end, run.end.saturating_add(SimDuration::ms(999))];
        let inside: Vec<SimTime> = run.probes.iter().flatten().map(|probe| probe.at).collect();
        for t in probes.into_iter().chain(inside) {
            let (by_seek, seen) = seek(&run, t);
            let (by_replay, replayed) = replay(&run.bytes, &run.descriptors(), t);
            assert_eq!(by_seek, by_replay, "the state at {} ms", t.as_ms());
            assert_eq!(seen, replayed, "the places at {} ms", t.as_ms());
        }
    }
}

/// What a robot is doing at an instant has one answer, whether a reader seeks to the keyframe before it or plays the
/// run from its first frame, and it is the state the builder gave the robot there: at every boundary, where a state
/// can begin with the slice and the keyframe written there must already state it, and inside every slice, where a
/// robot the slice does not name holds the state it began the slice in.
#[test]
fn a_robots_state_at_an_instant_is_the_one_answer_a_seek_and_a_replay_both_give() {
    for (cadence, seed) in [(1u32, 3u64), (4, 11), (30, 19)] {
        let run = build(RunShape { cadence, slices: 33, seed, robot_state: true, ..RunShape::default() });
        for (slice, boundary) in run.boundaries.iter().enumerate() {
            let (by_seek, _) = seek(&run, *boundary);
            run.truth[slice].check(&by_seek);
            let (by_replay, _) = replay(&run.bytes, &run.descriptors(), *boundary);
            assert_eq!(by_seek, by_replay, "the state at the boundary {} ms", boundary.as_ms());
        }
        for probe in run.probes.iter().flatten() {
            let (by_seek, seen) = seek(&run, probe.at);
            let (by_replay, replayed) = replay(&run.bytes, &run.descriptors(), probe.at);
            let states = ordered(seen.robot_states.as_ref().map(|held| &held.robots));
            assert_eq!(states, probe.states, "what each robot is doing {} ms in", probe.at.as_ms());
            assert_eq!(seen, replayed, "the places at {} ms", probe.at.as_ms());
            assert_eq!(by_seek, by_replay, "the state at {} ms", probe.at.as_ms());
        }
    }
}

#[test]
fn a_run_of_one_slice_seeks_to_its_only_instant() {
    let run = build(RunShape { cadence: 1, slices: 1, ..RunShape::default() });
    let (state, _) = seek(&run, SimTime::ZERO);
    run.truth[0].check(&state);
    let (late, at) = seek(&run, SimTime::ms(999_999));
    assert_eq!(late.reached, SimTime::ZERO, "a run past its end stands where its last frame left it");
    assert_eq!(at.at, SimTime::ms(999_999));
}

#[test]
fn heat_keyframes_equal_the_sum_of_the_increments_before_them() {
    let run = build(RunShape { cadence: 4, slices: 21, ..RunShape::default() });
    let mut reader = run.reader();
    let keyframes: Vec<_> =
        reader.index().records().iter().filter(|record| record.tag == Tag::Keyframe).copied().collect();
    assert!(keyframes.len() > 3, "the run carries several keyframes");
    for record in keyframes {
        let sections = reader.sections_at(&record).expect("the keyframe reads");
        let stated = Keyframe::decode(&sections, record.offset).expect("decodes").heat.expect("a heat section");
        let (replayed, _) = replay(&run.bytes, &run.descriptors(), instant(record.t));
        let summed = HeatKeyframe {
            layers: replayed
                .heat
                .layers
                .iter()
                .map(|(name, counts)| HeatLayer {
                    name: name.clone(),
                    counts: {
                        let mut counts: Vec<(CellId, u64)> = counts.iter().map(|(c, n)| (*c, *n)).collect();
                        counts.sort_unstable();
                        counts
                    },
                })
                .collect(),
        };
        assert_eq!(stated, summed.canonical(), "the keyframe at {} ms is the increments before it", record.t.0);
    }
}

trait Canonical {
    fn canonical(&self) -> Self;
}

impl Canonical for HeatKeyframe {
    /// A heat keyframe put through its own encoding, which is what orders its
    /// layers and counts — two of them are only comparable there.
    fn canonical(&self) -> Self {
        Self::from_bytes(&self.to_bytes().expect("encodes"), 0).expect("decodes")
    }
}

/// A tote born in a delta and left where it was born. A reader that seeks
/// lands on the keyframe after it and has the tote; one that replays meets
/// only the creation record, and reaches the same tote only if the creation
/// puts it somewhere.
#[test]
fn a_tote_created_mid_run_reaches_a_seek_and_a_replay_alike() {
    let born = ToteRecord {
        tote: ToteId(7),
        tote_type: 1,
        designation: 1,
        location: ToteLocation::AtCell(CellId(5)),
        contents_units: 12,
    };
    let mut bytes = Vec::new();
    {
        let mut writer = Writer::new(&mut bytes);
        writer.append_scenario_block(&hand_built_scenario()).expect("opens");
        writer.append_keyframe(SimTime::ZERO, &whole_keyframe(PlaybackKeyframe::default())).expect("an empty keyframe");
        let creating = whole_delta(PlaybackDelta {
            tote_creations: vec![ToteCreation {
                offset: SimDuration::ms(100),
                tote: born.tote,
                tote_type: born.tote_type,
                designation: born.designation,
                location: born.location,
            }],
            tote_contents: vec![ToteContents {
                offset: SimDuration::ms(200),
                tote: born.tote,
                units: born.contents_units,
            }],
            ..PlaybackDelta::default()
        });
        writer.append_delta(SimTime::ZERO, &creating).expect("the delta that creates it");
        let slice_one = SimTime::ms(SLICE_WIDTH_MS);
        let stated = whole_keyframe(PlaybackKeyframe { totes: vec![born], ..PlaybackKeyframe::default() });
        writer.append_keyframe(slice_one, &stated).expect("the keyframe that restates it");
        writer.append_delta(slice_one, &whole_delta(PlaybackDelta::default())).expect("an empty delta");
        writer.finish_with(slice_one, &End::default()).expect("closes");
    }

    let t = SimTime::ms(SLICE_WIDTH_MS);
    let (by_seek, _) = state_at(&mut Reader::from_bytes(&bytes).expect("reads"), t).expect("seeks");
    let held = by_seek.playback.totes[&born.tote];
    assert_eq!(held.location, born.location, "the keyframe states the tote");
    assert_eq!(held.contents_units, born.contents_units, "and the units the delta filled it with");

    // The replay never reads that keyframe: it meets the tote in the creation
    // record alone, and has to reach the same tote from it.
    let (by_replay, _) = replay(&bytes, &Descriptors::default(), t);
    assert_eq!(by_replay.playback.totes.get(&born.tote), by_seek.playback.totes.get(&born.tote));
    assert_eq!(by_seek, by_replay, "the creation record is the replay's only way to the tote");
}

/// Two runs of one shape are one run: the same records, the same ordering
/// rules, the same bytes. A recording that differed between two writes of the
/// same run would make every comparison of two runs meaningless.
#[test]
fn writing_one_run_twice_gives_the_same_bytes() {
    let shape = RunShape { cadence: 3, slices: 12, seed: 23, ..RunShape::default() };
    assert_eq!(build(shape).bytes, build(shape).bytes);
}

/// A tote that is born and leaves inside one slice is never in a keyframe, so
/// the two records of that one delta are the only place it exists.
#[test]
fn a_tote_born_and_extracted_inside_one_slice_is_in_that_delta_and_in_no_keyframe() {
    let tote = ToteId(3);
    let mut bytes = Vec::new();
    {
        let mut writer = Writer::new(&mut bytes);
        writer.append_scenario_block(&hand_built_scenario()).expect("opens");
        writer.append_keyframe(SimTime::ZERO, &whole_keyframe(PlaybackKeyframe::default())).expect("an empty keyframe");
        let brief = whole_delta(PlaybackDelta {
            tote_creations: vec![ToteCreation {
                offset: SimDuration::ms(100),
                tote,
                tote_type: 2,
                designation: 1,
                location: ToteLocation::AtCell(CellId(9)),
            }],
            tote_transitions: vec![ToteTransition {
                offset: SimDuration::ms(5_000),
                tote,
                location: ToteLocation::Extracted,
            }],
            ..PlaybackDelta::default()
        });
        writer.append_delta(SimTime::ZERO, &brief).expect("the slice it lived in");
        let slice_one = SimTime::ms(SLICE_WIDTH_MS);
        writer
            .append_keyframe(slice_one, &whole_keyframe(PlaybackKeyframe::default()))
            .expect("a keyframe that states no totes");
        writer.append_delta(slice_one, &whole_delta(PlaybackDelta::default())).expect("an empty delta");
        writer.finish_with(slice_one, &End::default()).expect("closes");
    }

    let mut reader = Reader::from_bytes(&bytes).expect("reads");
    let (_, early) = state_at(&mut reader, SimTime::ms(1_000)).expect("seeks into the slice it lived in");
    assert_eq!(early.totes[&tote], ToteLocation::AtCell(CellId(9)), "it is there after its creation");
    let (_, gone) = state_at(&mut reader, SimTime::ms(6_000)).expect("seeks past its extraction");
    assert!(!gone.totes.contains_key(&tote), "and gone once it is extracted");
    let (after, _) = state_at(&mut reader, SimTime::ms(SLICE_WIDTH_MS)).expect("seeks past the slice");
    assert!(!after.playback.totes.contains_key(&tote), "no keyframe ever states it");
}

/// Which robot a tote is riding is stated twice — on the tote as its
/// location, and on the robot as the tote it carries — and a keyframe states
/// both. A run reached by applying deltas has to end up with the pair the
/// next keyframe would have written.
#[test]
fn a_tote_moving_between_robots_moves_which_robot_carries_it() {
    let bare = |robot: u32| RobotRecord {
        robot: RobotId(robot),
        class: RobotClass::Htm,
        at: Position::new(robot as i32 * 1_000, 0, 0),
        heading_mrad: None,
        carrying: None,
        cell: CellId(robot),
        toward: None,
    };
    let tote = ToteId(7);
    let onto = |location| {
        whole_delta(PlaybackDelta {
            tote_transitions: vec![ToteTransition { offset: SimDuration::ms(100), tote, location }],
            ..PlaybackDelta::default()
        })
    };

    let mut bytes = Vec::new();
    {
        let mut writer = Writer::new(&mut bytes);
        writer.append_scenario_block(&hand_built_scenario()).expect("opens");
        let start = whole_keyframe(PlaybackKeyframe {
            robots: vec![bare(0), bare(1)],
            totes: vec![ToteRecord {
                tote,
                tote_type: 1,
                designation: 0,
                location: ToteLocation::AtCell(CellId(5)),
                contents_units: 0,
            }],
            ..PlaybackKeyframe::default()
        });
        writer.append_keyframe(SimTime::ZERO, &start).expect("the keyframe");
        for (slice, location) in [
            (0, ToteLocation::OnBot(RobotId(0))),
            (1, ToteLocation::OnBot(RobotId(1))),
            (2, ToteLocation::AtCell(CellId(9))),
        ] {
            writer.append_delta(SimTime::ms(slice * SLICE_WIDTH_MS), &onto(location)).expect("a delta");
        }
        // The slice an instant falls in is read into rather than applied, so
        // the run needs the slice that begins where the last move ends.
        writer
            .append_delta(SimTime::ms(3 * SLICE_WIDTH_MS), &whole_delta(PlaybackDelta::default()))
            .expect("the slice after them");
        writer.finish_with(SimTime::ms(4 * SLICE_WIDTH_MS), &End::default()).expect("closes");
    }

    let mut reader = Reader::from_bytes(&bytes).expect("reads");
    let carrying = |state: &RunState, robot: u32| state.playback.robots[&RobotId(robot)].carrying;
    for (slice, expected) in [(1u64, (Some(tote), None)), (2, (None, Some(tote))), (3, (None, None))] {
        let (state, _) = state_at(&mut reader, SimTime::ms(slice * SLICE_WIDTH_MS)).expect("seeks");
        assert_eq!((carrying(&state, 0), carrying(&state, 1)), expected, "at slice {slice}");
    }
    let (_, inside) = state_at(&mut reader, SimTime::ms(SLICE_WIDTH_MS / 2)).expect("seeks into the first slice");
    assert_eq!(inside.totes[&tote], ToteLocation::OnBot(RobotId(0)), "the move inside a slice shows in its places");
}

/// A robot that turns where it stands is stated by the slice it turned in and
/// by nothing else, so the two ways to an instant have to agree about it: a
/// seek reads the keyframe before it and the deltas since, and a replay reads
/// every delta from the first frame.
#[test]
fn a_robot_that_turns_without_moving_agrees_between_a_seek_and_a_replay() {
    let robot = RobotId(0);
    let turning = |heading: Option<i32>| {
        whole_delta(PlaybackDelta { headings: vec![(robot, heading)], ..PlaybackDelta::default() })
    };
    let headings = [Some(0), Some(1_571), Some(3_142), None, Some(4_712)];

    let mut bytes = Vec::new();
    {
        let mut writer = Writer::new(&mut bytes);
        writer.append_scenario_block(&hand_built_scenario()).expect("opens");
        for (slice, heading) in headings.iter().enumerate() {
            let at = SimTime::ms(slice as u64 * SLICE_WIDTH_MS);
            if slice == 0 {
                let placed = PlaybackKeyframe {
                    robots: vec![RobotRecord {
                        robot,
                        class: RobotClass::Htm,
                        at: Position::new(4_000, 0, 0),
                        heading_mrad: *heading,
                        carrying: None,
                        cell: CellId(4),
                        toward: None,
                    }],
                    ..PlaybackKeyframe::default()
                };
                writer.append_keyframe(at, &whole_keyframe(placed)).expect("the one keyframe of the run");
            }
            writer.append_delta(at, &turning(*heading)).expect("a delta that only turns the robot");
        }
        let end = SimTime::ms(headings.len() as u64 * SLICE_WIDTH_MS);
        writer.append_delta(end, &whole_delta(PlaybackDelta::default())).expect("the slice after them");
        writer.finish_with(end.saturating_add(SimDuration::ms(SLICE_WIDTH_MS)), &End::default()).expect("closes");
    }

    for (slice, heading) in headings.iter().enumerate() {
        for offset in [0, SLICE_WIDTH_MS / 2] {
            let t = SimTime::ms(slice as u64 * SLICE_WIDTH_MS + offset);
            let (by_seek, seen) = state_at(&mut Reader::from_bytes(&bytes).expect("reads"), t).expect("seeks");
            let (by_replay, replayed) = replay(&bytes, &Descriptors::default(), t);
            assert_eq!(by_seek, by_replay, "the state at {} ms", t.as_ms());
            assert_eq!(seen, replayed, "the places at {} ms", t.as_ms());
            assert_eq!(
                by_seek.playback.robots[&robot].heading_mrad,
                *heading,
                "the robot faces where slice {slice} says, at {} ms",
                t.as_ms()
            );
        }
    }
}

/// A robot's heading travels in the delta beside its steps, so a seek that
/// reads one keyframe and many deltas must end holding the heading the last
/// delta stated. This run's cadence is longer than the run, so the only
/// keyframe is the first and every heading past it came out of a delta.
#[test]
fn a_heading_a_delta_states_reaches_a_seek_that_read_one_keyframe() {
    let shape = RunShape { cadence: 1_000, slices: 24, ..RunShape::default() };
    let run = build(shape);
    let t = run.boundaries.last().copied().expect("a run has slices");
    let (state, _) = seek(&run, t);
    let stated: Vec<Option<i32>> =
        (0..shape.robots).map(|robot| state.playback.robots[&RobotId(robot)].heading_mrad).collect();
    let unturned: Vec<Option<i32>> = (0..shape.robots).map(start_heading).collect();
    assert_ne!(stated, unturned, "no delta turned a robot, so the test proves nothing");
    run.truth[run.truth.len() - 1].check(&state);
}

/// The class a keyframe states is the one the scenario block's id table
/// states, which is what lets a viewer draw a machine it seeked straight to.
#[test]
fn a_robots_class_is_the_same_in_the_id_table_and_in_every_keyframe() {
    let run = build(RunShape { slices: 3, cadence: 1, ..RunShape::default() });
    let scenario = Scenario::read_from(&mut run.reader()).expect("the scenario block reads");
    assert_eq!(scenario, scenario_of(run.shape));
    let (state, _) = seek(&run, run.boundaries[1]);
    for entry in &scenario.robots {
        assert_eq!(state.playback.robots[&entry.robot].class, entry.class);
    }
    assert_eq!(class_of(0), RobotClass::Htm);
    assert_eq!(Descriptors::from_scenario(&scenario).metrics.len(), run.shape.metrics);
}

/// A leg the planner curved arrives at one cell along an arc, and the
/// observer records it as several steps all naming that cell, each at its own
/// point on the curve. A scrub into the middle of such a leg has to land on
/// the arc, by the same reading whether it got there by seeking to a keyframe
/// or by replaying from the run's first frame.
#[test]
fn a_curve_recorded_as_three_steps_to_one_cell_reaches_a_seek_and_a_replay_alike() {
    let robot = RobotId(3);
    let corner = CellId(9);
    let arc = [Position::new(1_000, 200, 0), Position::new(1_600, 700, 0), Position::new(1_800, 1_500, 0)];
    let curve = whole_delta(PlaybackDelta {
        movers: vec![MoverSteps {
            robot,
            start_heading_mrad: None,
            steps: arc
                .iter()
                .map(|&at| Step { at, dt: SimDuration::ms(SLICE_WIDTH_MS / 3), kind: StepKind::Move, cell: corner })
                .collect(),
            cell: corner,
        }],
        ..PlaybackDelta::default()
    });

    let mut bytes = Vec::new();
    {
        let mut writer = Writer::new(&mut bytes);
        writer.append_scenario_block(&hand_built_scenario()).expect("opens");
        let start = whole_keyframe(PlaybackKeyframe {
            robots: vec![RobotRecord {
                robot,
                class: RobotClass::Htm,
                // The machine starts parked off its cell's centre, which is
                // what `InitialPositions` states.
                at: Position::new(400, 0, 0),
                heading_mrad: None,
                carrying: None,
                cell: CellId(4),
                toward: None,
            }],
            ..PlaybackKeyframe::default()
        });
        writer.append_keyframe(SimTime::ZERO, &start).expect("the keyframe");
        writer.append_delta(SimTime::ZERO, &curve).expect("the curve");
        writer
            .append_delta(SimTime::ms(SLICE_WIDTH_MS), &whole_delta(PlaybackDelta::default()))
            .expect("the slice after it");
        writer.finish_with(SimTime::ms(2 * SLICE_WIDTH_MS), &End::default()).expect("closes");
    }

    // A third of the way into the first step, which runs from the parked pose
    // to the arc's first point.
    let third = SimTime::ms(SLICE_WIDTH_MS / 9);
    let mut reader = Reader::from_bytes(&bytes).expect("reads");
    let (_, seen) = state_at(&mut reader, third).expect("seeks into the curve");
    let held = seen.robots[&robot];
    assert_eq!(
        held.at,
        Position::new(600, 67, 0),
        "a third of the way from where it was parked to the arc's first point, in whole millimetres"
    );
    assert_eq!(held.cell, CellId(4), "and still counted against the cell it has not left");

    let (by_replay_state, by_replay) = replay(&bytes, &Descriptors::default(), third);
    let (by_seek_state, _) = state_at(&mut reader, third).expect("seeks again");
    assert_eq!(by_seek_state, by_replay_state, "a seek and a replay hold the same state");
    assert_eq!(by_replay.robots[&robot], held, "and put the machine at the same point on the arc");

    // Once the slice has been taken in whole, the machine stands at the last
    // point of the arc, not at the cell's centre.
    let (after, _) = state_at(&mut reader, SimTime::ms(SLICE_WIDTH_MS)).expect("seeks past the curve");
    assert_eq!(after.playback.robots[&robot].at, arc[2], "the arc's last point is where the slice left it");
    assert_eq!(after.playback.robots[&robot].cell, corner, "in the cell the arc arrived at");
}
