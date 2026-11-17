use desgraph::CellKind;
use worldview_render::{CameraState, Floor, Scene, ViewMode, Viewport};

use super::*;

/// Three cells in a row and one apart, 1 m squares from the origin.
fn scene() -> Scene {
    let centres = [(0, [500.0, 500.0]), (1, [1_500.0, 500.0]), (2, [2_500.0, 500.0]), (3, [500.0, 2_500.0])];
    Scene::new(Floor::from_centres(centres.map(|(id, at)| (id, CellKind::Highway, at)), 1_000.0))
}

/// A camera looking straight down on the scene, its view square and 400 px a side.
fn screen(scene: &Scene) -> Screen {
    let viewport = Viewport::new([0.0, 0.0], [400.0, 400.0]);
    Screen::of(&CameraState::new(scene.frame().clone()).build_uniforms(viewport, 1.0, ViewMode::State))
}

/// What the camera draws at the centre of the floor is at the centre of the
/// screen, and the floor's x runs right and its y up the screen when looked
/// at from above, which is the drawing's north-up.
#[test]
fn the_floor_lands_where_the_camera_draws_it() {
    let scene = scene();
    let (screen, frame) = (screen(&scene), scene.frame());
    let (width, depth) = frame.extent();
    let middle = [f64::from(width) * 500.0, f64::from(depth) * 500.0];
    let ScreenPx(centre) = screen.at(frame, FloorMm(middle)).unwrap();
    assert!((centre[0] - 200.0).abs() < 1e-3 && (centre[1] - 200.0).abs() < 1e-3, "{centre:?}");
    assert_eq!(screen.centre(), ScreenPx([200.0, 200.0]));
    let ScreenPx(east) = screen.at(frame, FloorMm([middle[0] + 1_000.0, middle[1]])).unwrap();
    let ScreenPx(north) = screen.at(frame, FloorMm([middle[0], middle[1] + 1_000.0])).unwrap();
    assert!(east[0] > centre[0] && (east[1] - centre[1]).abs() < 1e-3);
    assert!(north[1] < centre[1] && (north[0] - centre[0]).abs() < 1e-3);
}

/// Spec 03 section 3.6: the named frame's axes are drawn with its name, each
/// arrow along the axis as it lies on the screen, so a frame whose y runs
/// down the drawing is drawn pointing down.
#[test]
fn a_frames_axes_point_the_way_it_lies_on_the_drawing() {
    let scene = scene();
    let (screen, frame) = (screen(&scene), scene.frame());
    let ground = FloorMm([1_500.0, 1_500.0]);
    let up = Axes { name: "shell".into(), x: [1.0, 0.0], y: [0.0, 1.0] };
    let AxesDrawn { segments, labels } = up.drawn(&screen, frame, ground).unwrap();
    assert_eq!(segments.len(), 6, "two arrows, each a shaft and two barbs");
    let [ScreenPx(origin), ScreenPx(x_tip)] = segments[0];
    let [_, ScreenPx(y_tip)] = segments[3];
    assert!(x_tip[0] > origin[0] && y_tip[1] < origin[1], "x right, y up the drawing");
    let texts: Vec<&str> = labels.iter().map(|label| label.text.as_str()).collect();
    assert_eq!(texts, ["x", "y", "shell"]);

    let mirrored = Axes { name: "floor1".into(), x: [1.0, 0.0], y: [0.0, -1.0] };
    let [ScreenPx(from), ScreenPx(to)] = mirrored.drawn(&screen, frame, ground).unwrap().segments[3];
    assert!(to[1] > from[1], "the mirrored frame's y points down the drawing");
}
