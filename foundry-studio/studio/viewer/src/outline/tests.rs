use desgraph::CellKind;
use worldview_render::{CameraState, Floor, Scene, ViewMode, Viewport};

use super::*;

/// Three cells in a row and one apart, 1 m squares from the origin, and a
/// fifth on the fourth's own footprint.
fn scene() -> Scene {
    let centres = [
        (0, [500.0, 500.0]),
        (1, [1_500.0, 500.0]),
        (2, [2_500.0, 500.0]),
        (3, [500.0, 2_500.0]),
        (4, [500.0, 2_500.0]),
    ];
    Scene::new(Floor::from_centres(centres.map(|(id, at)| (id, CellKind::Highway, at)), 1_000.0))
}

/// A camera looking straight down on the scene, its view square and 400 px a side.
fn screen(scene: &Scene) -> Screen {
    let viewport = Viewport::new([0.0, 0.0], [400.0, 400.0]);
    Screen::of(&CameraState::new(scene.frame().clone()).build_uniforms(viewport, 1.0, ViewMode::State))
}

/// Each marked cell is outlined on its own sides, a shared side once, so two
/// cells on one footprint outline that footprint rather than cancel it
/// (reviews-impl/astra-t12.md finding 4).
#[test]
fn each_cell_is_outlined_on_its_own_sides_and_a_shared_side_once() {
    let scene = scene();
    let sides = |ids: &[u32]| cell_sides(&ids.iter().map(|&id| CellId(id)).collect::<Vec<_>>(), scene.cells()).len();
    assert_eq!(sides(&[0, 1, 2]), 10, "a row of three squares: twelve sides, two of them shared");
    assert_eq!(sides(&[0, 3]), 8, "two squares apart share none");
    assert_eq!(sides(&[3, 4]), 4, "two cells on one footprint outline it once");
    assert_eq!(sides(&[9]), 0, "a cell the floor does not have outlines nothing");
}

/// The outlines are drawn diff first, the agent's groups next and the
/// selection last, and re-marked on a new floor from the keys they were given.
#[test]
fn outlines_keep_their_keys_across_a_new_floor() {
    let scene = scene();
    let roll = Roll::of_census(vec![("station".into(), vec![0, 1, 2]), ("far".into(), vec![3])]);
    let mut outlines = Outlines::default();
    assert_eq!(outlines.highlight(vec![vec!["station".into(), "a sector".into()]], &roll, scene.cells()), [1]);
    assert_eq!(outlines.select(vec!["far".into()], &roll, scene.cells()), 1);
    assert_eq!(outlines.show_diff(vec![], vec!["far".into()], &roll, scene.cells()), [0, 1]);
    let inks: Vec<Ink> = outlines.strokes(scene.frame(), &screen(&scene)).iter().map(|stroke| stroke.ink).collect();
    assert_eq!(inks, [Ink::Added, Ink::Changed, Ink::Agent, Ink::Selected]);

    let moved = Roll::of_census(vec![("far".into(), vec![0])]);
    outlines.refloor(&moved, scene.cells());
    let strokes = outlines.strokes(scene.frame(), &screen(&scene));
    assert!(strokes[2].segments.is_empty(), "the station is not on the new floor");
    assert_eq!(strokes[3].segments.len(), 4, "the selection marks where its key stands now");
}

/// A selection that stands nowhere itself, a sector, is outlined by the
/// members it marks, as one union, and an empty one outlines nothing (spec 03
/// section 3.5; reviews-impl/astra-t14.md, the first P1).
#[test]
fn a_selection_outlines_the_union_of_what_it_marks() {
    let scene = scene();
    let roll = Roll::of_census(vec![("near".into(), vec![0]), ("next".into(), vec![1]), ("far".into(), vec![3])]);
    let mut outlines = Outlines::default();
    let marked = vec!["near".into(), "next".into(), "far".into(), "a member off the floor".into()];
    assert_eq!(outlines.select(marked, &roll, scene.cells()), 3);
    let strokes = outlines.strokes(scene.frame(), &screen(&scene));
    assert_eq!(strokes.len(), 1);
    assert_eq!(strokes[0].segments.len(), 11, "two squares sharing a side, and one apart: twelve sides, one shared");
    assert_eq!(outlines.select(Vec::new(), &roll, scene.cells()), 0);
    assert!(outlines.strokes(scene.frame(), &screen(&scene)).is_empty(), "nothing selected, nothing outlined");
}
