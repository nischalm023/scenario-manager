//! Within `recording`, these tests pin the conversion against a hand-written
//! slice: the one place the recording's words and the renderer's are held
//! side by side without a simulator, a browser or a GPU in the room.
//!
//! The real cross-check is the other one — the browser viewer's rows against
//! desvis's `--dump-render` over a whole cencora recording
//! (`studio/viewer/tests/dumped_rows.rs`). This says what the
//! conversion means; that says the two viewers agree about a real run.

use std::f32::consts::FRAC_PI_2;

use bytemuck::Zeroable;

use desgraph::CellId;
use desgraph::types::CellKind;
use desunits::{RobotId, SimDuration, SimTime, ToteId};
use worldview::frames::playback::{
    MoverSteps, PlaybackDelta, Position, RelayTransition, RobotClass, Step, StepKind, ToteContents, ToteCreation,
    ToteLocation,
};
use worldview::frames::state::{PlaybackState, RobotPlace, ToteState};
use worldview::{Delta, RunState};

use super::from_recording;
use crate::rack::RackVisual;
use crate::scene::{ContainerKind, Designation, Floor, Place, Scene};

/// Four cells in a row, 1,000 mm apart, at y = 500: the floor a browser is
/// sent, stated by centres alone.
fn scene() -> Scene {
    let floor =
        Floor::from_centres((0..4).map(|id| (id, CellKind::Highway, [500.0 + f64::from(id) * 1000.0, 500.0])), 1000.0);
    Scene::new(floor).with_rack(RackVisual::uniform(2))
}

/// A robot standing on cell 0 facing east, a container on its deck, and a
/// relay cell held — the state the slice below opens on.
fn opened_on() -> RunState {
    let mut playback = PlaybackState::default();
    playback.robots.insert(
        RobotId(1),
        RobotPlace {
            class: RobotClass::Htm,
            at: Position::new(500, 500, 0),
            heading_mrad: Some(0),
            carrying: Some(ToteId(7)),
            cell: CellId(0),
            toward: None,
        },
    );
    playback.totes.insert(
        ToteId(7),
        ToteState { tote_type: 1, designation: 0, location: ToteLocation::OnBot(RobotId(1)), contents_units: 3 },
    );
    playback.reserved_relay_cells.insert(CellId(3));
    RunState { reached: SimTime::ms(10_000), playback, ..RunState::default() }
}

/// The robot walks east onto cell 1 over four seconds, turns north for two,
/// then walks north over four; a carton is born at cell 3 two seconds in and
/// filled at five; cell 3 stops being held at three.
fn slice() -> Delta {
    let step = |cell: u32, x_mm: i32, y_mm: i32, dt_ms: u64, kind| Step {
        at: Position::new(x_mm, y_mm, 0),
        dt: SimDuration::ms(dt_ms),
        kind,
        cell: CellId(cell),
    };
    Delta {
        playback: Some(PlaybackDelta {
            movers: vec![MoverSteps {
                robot: RobotId(1),
                start_heading_mrad: Some(0),
                steps: vec![
                    step(1, 1_500, 500, 4_000, StepKind::Move),
                    step(1, 1_500, 500, 2_000, StepKind::Rotate),
                    step(2, 1_500, 1_500, 4_000, StepKind::Move),
                ],
                cell: CellId(2),
            }],
            tote_creations: vec![ToteCreation {
                offset: SimDuration::ms(2_000),
                tote: ToteId(9),
                tote_type: 2,
                designation: 1,
                location: ToteLocation::AtCell(CellId(3)),
            }],
            tote_contents: vec![ToteContents { offset: SimDuration::ms(5_000), tote: ToteId(9), units: 11 }],
            relay_transitions: vec![RelayTransition {
                cell: CellId(3),
                offset: SimDuration::ms(3_000),
                reserved: false,
            }],
            ..PlaybackDelta::default()
        }),
        ..Delta::default()
    }
}

/// Half way along its first leg the robot is a POSE there, not at either
/// centre, and it faces the way it is walking.
#[test]
fn a_robot_part_way_along_a_leg_stands_between_the_two_centres() {
    let drawn = from_recording(&scene(), &opened_on(), &slice(), SimTime::ms(12_000), &Default::default())
        .expect("the slice reads");
    assert_eq!(drawn.robots.len(), 1);
    assert_eq!(drawn.robots[0].at, Place { cell: 0, pos_mm: [1000.0, 500.0] });
    assert_eq!(drawn.robots[0].heading_rad, 0.0, "walking east");
}

/// The recording states one heading per robot per slice, so the turn has to
/// be read out of the steps: half way through the rotation the robot is half
/// way round, which is the whole reason this conversion reads them.
#[test]
fn a_robot_half_way_through_a_turn_faces_half_way_round() {
    let scene = scene();
    let state = opened_on();
    let slice = slice();
    let swept = |t_ms| {
        from_recording(&scene, &state, &slice, SimTime::ms(t_ms), &Default::default()).expect("reads").robots[0]
            .heading_rad
    };
    assert_eq!(swept(14_000), 0.0, "the turn has not begun");
    assert!((swept(15_000) - FRAC_PI_2 / 2.0).abs() < 1e-5, "half way round, got {}", swept(15_000));
    assert!((swept(16_000) - FRAC_PI_2).abs() < 1e-5, "facing the leg it departs on");
    assert!((swept(19_999) - FRAC_PI_2).abs() < 1e-5, "and still facing it while it walks");
}

/// A container is what its birth says only once its birth has happened, and
/// holds what its contents record says only once that has.
#[test]
fn a_container_born_mid_slice_is_nothing_until_it_is_born() {
    let (scene, state, slice) = (scene(), opened_on(), slice());
    let at = |t_ms| from_recording(&scene, &state, &slice, SimTime::ms(t_ms), &Default::default()).expect("reads");

    let early = at(11_000);
    assert_eq!(early.totes.len(), 1, "only the one riding the robot");
    assert_eq!(early.totes[0].id, ToteId(7));

    let born = at(12_000);
    assert_eq!(born.totes.len(), 2);
    let carton = born.totes.iter().find(|tote| tote.id == ToteId(9)).expect("the carton");
    assert_eq!(carton.appearance.container, ContainerKind::Carton);
    assert_eq!(carton.appearance.designation, Designation::Order);
    assert_eq!(carton.appearance.contents_units, 0, "nothing in it yet");
    assert_eq!(born.totes[0].id, ToteId(7), "sorted by id, so a shelf slot stays put");

    let filled = at(15_000);
    assert_eq!(
        filled.totes.iter().find(|tote| tote.id == ToteId(9)).expect("the carton").appearance.contents_units,
        11
    );
    assert_eq!(
        filled.totes.iter().find(|tote| tote.id == ToteId(7)).expect("the tote").appearance.container,
        ContainerKind::Tote,
        "the one the keyframe stated keeps its own kind"
    );
}

/// A relay cell is held until the instant the slice says it is let go.
#[test]
fn a_relay_cell_is_held_until_the_instant_it_is_let_go() {
    let (scene, state, slice) = (scene(), opened_on(), slice());
    let held = |t_ms| {
        from_recording(&scene, &state, &slice, SimTime::ms(t_ms), &Default::default()).expect("reads").reserved_cells
    };
    assert_eq!(held(12_999), vec![false, false, false, true]);
    assert_eq!(held(13_000), vec![false; 4]);
}

/// The instance rows the conversion feeds the renderer: a laden HTM half way
/// along its leg, and the container on its deck at the same millimetres.
#[test]
fn the_rows_the_renderer_is_fed_place_the_load_on_its_carrier() {
    let scene = scene();
    let drawn =
        from_recording(&scene, &opened_on(), &slice(), SimTime::ms(12_000), &Default::default()).expect("reads");
    let moment = scene.moment(Zeroable::zeroed(), None, &drawn.playback(Default::default()));

    assert_eq!(moment.bot_instances.len(), 1);
    assert_eq!(moment.htm_instance_count, 1);
    assert_eq!([moment.bot_instances[0].world_x, moment.bot_instances[0].world_y], [1.0, 0.5]);

    let carried = moment.tote_instances.iter().find(|tote| tote.tote_id == 7).expect("the carried tote");
    assert_eq!([carried.world_x, carried.world_y], [1.0, 0.5], "a load rides where its carrier is");
    assert_eq!(carried.tote_type, 3, "on an HTM");
    assert_eq!(carried.contents_units, 3);
    assert_eq!(moment.cell_instances.len(), 4);
}
