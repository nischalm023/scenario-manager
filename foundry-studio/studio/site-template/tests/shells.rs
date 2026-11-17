//! The shell templates of the development seed (spec 05 section 5), each evaluated through the engine from its
//! documents on disk, with no server: at its defaults, at each knob's minimum and maximum, at each orientation
//! (section 5.5, acceptances 1 and 4, the map half: the floor compile and the flow validation are the API's), and
//! with its outline following the aisles and stations until someone sets it (D120).

mod support;

use std::collections::BTreeSet;
use std::path::Path;

use site_model::document::Document;
use site_model::pipeline::layers::LayerSet;
use site_model::pipeline::{World, derive};
use site_model::units::Coord;

use site_template::{
    Count, DocId, Evaluation, KnobDecl, KnobForm, KnobValue, KnobValues, Mint, Prior, ProblemCode, Severity,
    TemplateRead, evaluate,
};

const SHELLS: [&str; 5] = ["rectangle", "square", "l-shape", "u-shape", "i-shape"];

fn knobs_of(read: &TemplateRead) -> Vec<KnobDecl> {
    let root = read.scopes.iter().find(|scope| scope.scope == read.template).unwrap();
    root.entities
        .iter()
        .filter_map(|entity| Some(KnobDecl::read(entity.id, entity.components.get("template.knob")?).unwrap()))
        .collect()
}

fn run(read: &TemplateRead, knobs: &KnobValues) -> Evaluation {
    evaluate(read, knobs, &Prior::new(DocId::mint()), &mut Mint)
}

/// The evaluation's document loads in the simulator's reader, which finds the made primary frame as its own.
fn assert_loads(read: &TemplateRead, evaluation: &Evaluation, what: &str) {
    let map = dessim::worldmap::load_from_bytes(&evaluation.document(read).to_bytes())
        .unwrap_or_else(|fault| panic!("{what}: the simulator's reader refuses the document: {fault}"));
    let primary = map.world.frames.primary().map(ToString::to_string);
    assert_eq!(primary, evaluation.prologue.primary_frame.as_ref().map(ToString::to_string), "{what}");
}

/// Each shell's outline knobs, and the size their default expressions come to at the other knobs' defaults: the
/// integer defaults the seed held before D120, so the shells look as they did.
const OUTLINES: [(&str, &[(&str, i64)]); 5] = [
    ("rectangle", &[("width", 40800), ("depth", 27200)]),
    ("square", &[("side", 40800)]),
    ("l-shape", &[("width", 40800), ("depth", 34000)]),
    ("u-shape", &[("width", 47600), ("depth", 34000)]),
    ("i-shape", &[("width", 40800), ("depth", 27200)]),
];

/// Spec 05 section 5.5, acceptance 1 (the map half): at its defaults each shell evaluates with no problem, its
/// outline at the size its default expressions come to, and its document loads in foundry's reader with its primary
/// frame found.
#[test]
fn each_shell_at_its_defaults_evaluates_cleanly_and_loads() {
    for (shell, outline) in OUTLINES {
        let read = support::read_template(shell);
        let evaluation = run(&read, &KnobValues::new());
        assert!(evaluation.problems.is_empty(), "{shell}: {:?}", evaluation.problems);
        for (key, size) in outline {
            assert_eq!(evaluation.knobs.get(key), Some(KnobValue(*size)), "{shell}: {key}");
        }
        let mut derived: Vec<String> = evaluation.derived.iter().map(ToString::to_string).collect();
        derived.sort();
        let mut keys: Vec<&str> = outline.iter().map(|(key, _)| *key).collect();
        keys.sort();
        assert_eq!(derived, keys, "{shell}: only the outline follows the other knobs");
        assert_eq!(evaluation.census.by_knob["aisles"], Count::new(4), "{shell}");
        assert_eq!(evaluation.census.declared["station"], Count::new(6), "{shell}: four pick and two putaway stations");
        assert_loads(&read, &evaluation, shell);
    }
}

/// D120, the brief's Done when on the engine: with six aisles each shell's outline along x grows to fit them with no
/// problem; set back to its default size by hand, it holds, and the fit check reports under it.
#[test]
fn each_outline_follows_the_aisles_until_someone_sets_it() {
    let grown = [
        ("rectangle", "width", 56100, 40800, "15,300"),
        ("square", "side", 56100, 40800, "15,300"),
        ("l-shape", "width", 56100, 40800, "13,600"),
        ("u-shape", "width", 54400, 47600, "6,800"),
        ("i-shape", "width", 56100, 40800, "15,300"),
    ];
    for (shell, outline, follows, set, short) in grown {
        let read = support::read_template(shell);
        let six = run(&read, &KnobValues::new().with("aisles", 6));
        assert!(six.problems.is_empty(), "{shell}: {:?}", six.problems);
        assert_eq!(six.knobs.get(outline), Some(KnobValue(follows)), "{shell}");
        let held = run(&read, &KnobValues::new().with("aisles", 6).with(outline, set));
        let found: Vec<(Option<String>, &str)> =
            held.problems.iter().map(|p| (p.knob.as_ref().map(ToString::to_string), p.text.as_str())).collect();
        let text = format!("Error: The aisle row needs {short} mm more floor than the shell's width gives.");
        assert_eq!(found, [(Some(outline.to_owned()), text.as_str())], "{shell}");
    }
}

/// D120: an outline whose contents need more than its maximum stands at the maximum, an error under it saying so in
/// place of its fit checks; the Square's −y wall at 24 stations a pitch of 12,750 mm apart needs 309,400 mm.
#[test]
fn an_outline_its_contents_outgrow_is_held_at_its_maximum_with_a_problem() {
    let read = support::read_template("square");
    let crowded = KnobValues::new().with("pick_minus_y", 12).with("putaway_minus_y", 12).with("station_pitch", 12750);
    let evaluation = run(&read, &crowded);
    assert_eq!(evaluation.knobs.get("side"), Some(KnobValue(255000)));
    let found: Vec<(ProblemCode, Option<String>)> =
        evaluation.problems.iter().map(|p| (p.code, p.knob.as_ref().map(ToString::to_string))).collect();
    assert_eq!(
        found,
        [(ProblemCode::DerivedOutOfRange, Some("side".into())), (ProblemCode::CheckFailed, Some("htms".into()))]
    );
    let text =
        "Error: Side's default comes to 309,400 mm, more than its maximum of 255,000 mm, so it stays at 255,000 mm.";
    assert_eq!(evaluation.problems[0].text, text);
}

const E: Severity = Severity::Error;
const W: Severity = Severity::Warning;

/// One knob at one bound: the problems it makes, each as the knob it stands under, its severity and words of its
/// problem that name it among that knob's checks, or [`HELD`] for an outline its contents push past its range.
type Failing = (&'static str, &'static str, i64, &'static [(&'static str, Severity, &'static str)]);

/// The words of the problem under an outline knob whose default expression left its range (D120).
const HELD: &str = "default comes to";

/// Spec 05 section 5.5, acceptance 4: every check that fails with one knob at a bound, the others at their defaults
/// and the outline following them (D120), read off each check's expression at that geometry; a knob at a bound not
/// listed fails none. An outline's fit checks stand under it and report once it is set, so only an outline set
/// small fails them; the L's putaway row stands on its cut corner, which no default follows.
const FAILING: &[Failing] = &[
    ("rectangle", "width", 4250, &[("width", E, "aisle row"), ("width", E, "−y wall"), ("width", E, "+y wall")]),
    ("rectangle", "depth", 4250, &[("depth", E, "aisles need")]),
    ("rectangle", "htms", 1, &[("htms", W, "Fewer HTMs")]),
    ("rectangle", "pick_minus_y", 0, &[("pick_minus_y", E, "at least one pick")]),
    ("rectangle", "putaway_plus_y", 0, &[("putaway_plus_y", E, "at least one putaway")]),
    (
        "square",
        "side",
        4250,
        &[("side", E, "aisle row"), ("side", E, "−y wall"), ("side", E, "+y wall"), ("side", E, "aisles need")],
    ),
    ("square", "htms", 1, &[("htms", W, "Fewer HTMs")]),
    ("square", "pick_minus_y", 0, &[("pick_minus_y", E, "at least one pick")]),
    ("square", "putaway_plus_y", 0, &[("putaway_plus_y", E, "at least one putaway")]),
    ("l-shape", "width", 4250, &[("width", E, "arm along y"), ("width", E, "aisle row"), ("width", E, "pick row")]),
    ("l-shape", "depth", 4250, &[("depth", E, "aisles need"), ("cut_depth", E, "arm along x")]),
    ("l-shape", "cut_width", 255000, &[("width", E, HELD)]),
    ("l-shape", "cut_depth", 4250, &[("putaway_stations", E, "putaway row")]),
    ("l-shape", "cut_depth", 255000, &[("depth", E, HELD), ("cut_depth", E, "arm along x")]),
    ("l-shape", "station_pitch", 12750, &[("putaway_stations", E, "putaway row")]),
    ("l-shape", "htms", 1, &[("htms", W, "Fewer HTMs")]),
    ("l-shape", "putaway_stations", 12, &[("putaway_stations", E, "putaway row")]),
    (
        "u-shape",
        "width",
        4250,
        &[
            ("width", E, "aisle row"),
            ("width", E, "pick row"),
            ("width", E, "putaway row"),
            ("court_width", E, "Each arm"),
        ],
    ),
    ("u-shape", "depth", 4250, &[("depth", E, "aisles need")]),
    ("u-shape", "court_width", 255000, &[("width", E, HELD), ("court_width", E, "Each arm")]),
    ("u-shape", "court_depth", 255000, &[("depth", E, HELD)]),
    ("u-shape", "htms", 1, &[("htms", W, "Fewer HTMs")]),
    ("i-shape", "width", 4250, &[("width", E, "aisle row")]),
    ("i-shape", "depth", 4250, &[("depth", E, "aisles need"), ("depth", E, "pick row"), ("depth", E, "putaway row")]),
    ("i-shape", "htms", 1, &[("htms", W, "Fewer HTMs")]),
];

/// Spec 05 section 5.5, acceptance 4: each knob at its minimum and at its maximum, the others at their defaults,
/// fails exactly the checks [`FAILING`] lists for it, each at its severity, and nothing but checks and held
/// outlines; an evaluation with no error loads in the simulator's reader.
#[test]
fn each_knob_at_its_bounds_fails_the_checks_it_should() {
    let mut met = 0;
    for shell in SHELLS {
        let read = support::read_template(shell);
        let decls = knobs_of(&read);
        for decl in &decls {
            let (low, high) = decl.bounds();
            for value in [low.0, high.0] {
                let what = format!("{shell}, {} at {value}", decl.key);
                let row = FAILING.iter().find(|row| (row.0, row.1, row.2) == (shell, decl.key.as_str(), value));
                met += usize::from(row.is_some());
                let expected = row.map_or(&[][..], |row| row.3);
                let evaluation = run(&read, &KnobValues::new().with(decl.key.as_str(), value));
                assert!(
                    evaluation
                        .problems
                        .iter()
                        .all(|p| [ProblemCode::CheckFailed, ProblemCode::DerivedOutOfRange].contains(&p.code)),
                    "{what}: {:?}",
                    evaluation.problems
                );
                assert_eq!(evaluation.problems.len(), expected.len(), "{what}: {:?}", evaluation.problems);
                for (owner, severity, words) in expected {
                    let named = decls.iter().find(|d| d.key.as_str() == *owner).unwrap().checks.iter();
                    assert_eq!(
                        named.filter(|check| check.problem.contains(words)).count(),
                        usize::from(*words != HELD),
                        "{what}: `{words}` names one check of {owner}"
                    );
                    let failed = evaluation.problems.iter().any(|problem| {
                        problem.knob.as_ref().is_some_and(|knob| knob.as_str() == *owner)
                            && problem.severity == *severity
                            && problem.text.contains(words)
                    });
                    assert!(failed, "{what}: no {severity:?} on {owner} saying `{words}`: {:?}", evaluation.problems);
                }
                if !evaluation.has_errors() {
                    assert_loads(&read, &evaluation, &what);
                }
            }
        }
    }
    assert_eq!(met, FAILING.len(), "every listed knob and bound is one the shells have");
}

/// Spec 05 section 5.5, acceptance 4: each orientation code evaluates with no problem and loads.
#[test]
fn each_orientation_evaluates_cleanly_and_loads() {
    for shell in SHELLS {
        let read = support::read_template(shell);
        let Some(orientation) = knobs_of(&read).into_iter().find(|decl| decl.key.as_str() == "orientation") else {
            continue;
        };
        let KnobForm::Options(options) = &orientation.form else { panic!("{shell}: orientation is coded") };
        for option in options {
            let evaluation = run(&read, &KnobValues::new().with("orientation", option.value.0));
            assert!(evaluation.problems.is_empty(), "{shell} {}: {:?}", option.label, evaluation.problems);
            assert_loads(&read, &evaluation, &format!("{shell} {}", option.label));
        }
    }
}

fn world(read: &TemplateRead, evaluation: &Evaluation) -> World {
    let scenario = Document::from_value(Path::new("<shell>"), evaluation.document(read).as_value()).unwrap();
    derive(&LayerSet { scenario, layers: Vec::new() }).unwrap()
}

/// A position in the primary frame, in its steps.
type Point = [i64; 2];

/// Section 5.5's canonical keys for what a mirror moves: each cell by its centre in the primary frame, each station
/// by its position there and its kind.
fn canonical(world: &World) -> (BTreeSet<Point>, BTreeSet<(Point, String)>) {
    let cells = world.cells.values().map(|cell| [cell.centre[0].0, cell.centre[1].0]).collect();
    let mut stations = BTreeSet::new();
    for entity in world.entities.values() {
        let Some(station) = entity.components.get("station") else { continue };
        let pose = &entity.components["pose"];
        let frame = pose["frame"].as_str().unwrap().parse().unwrap();
        let at = |axis: usize| Coord(pose["position"][axis].as_i64().unwrap());
        let primary = world.frames.to_primary(&frame, [at(0), at(1)]).unwrap();
        stations.insert(([primary[0].0, primary[1].0], station["kind"].as_str().unwrap().to_owned()));
    }
    (cells, stations)
}

/// Spec 05 section 9: the L at each orientation code equals its default evaluation mirrored across the axes the
/// code names (x for an odd code, y from code 2), under canonical keys.
#[test]
fn the_l_at_each_orientation_is_its_default_mirrored() {
    let read = support::read_template("l-shape");
    let (width, depth) = (40800, 34000);
    let (cells, stations) = canonical(&world(&read, &run(&read, &KnobValues::new())));
    for code in 1..4 {
        let mirror = |p: [i64; 2]| {
            [if code % 2 == 1 { width - p[0] } else { p[0] }, if code / 2 == 1 { depth - p[1] } else { p[1] }]
        };
        let (turned_cells, turned_stations) =
            canonical(&world(&read, &run(&read, &KnobValues::new().with("orientation", code))));
        assert_eq!(turned_cells, cells.iter().map(|cell| mirror(*cell)).collect(), "orientation {code}");
        let expected: BTreeSet<_> = stations.iter().map(|(at, kind)| (mirror(*at), kind.clone())).collect();
        assert_eq!(turned_stations, expected, "orientation {code}");
    }
}

/// Spec 05 section 5.5's story under D120: the L-Shape with 6 aisles needs no wider shell by hand, its width
/// following to 56,100 mm, and makes 6 aisles of 2 VTMs each, 192 relays and 24 HTMs; its width set back to 40,800 mm
/// by hand holds, and needs 13,600 mm more.
#[test]
fn the_l_shape_story() {
    let read = support::read_template("l-shape");
    let short = run(&read, &KnobValues::new().with("aisles", 6).with("width", 40800));
    let texts: Vec<&str> = short.problems.iter().map(|problem| problem.text.as_str()).collect();
    assert_eq!(texts, ["Error: The aisle row needs 13,600 mm more floor than the shell's width gives."]);
    let wide = run(&read, &KnobValues::new().with("aisles", 6));
    assert!(wide.problems.is_empty(), "{:?}", wide.problems);
    assert_eq!(wide.knobs.get("width"), Some(KnobValue(56100)));
    assert_eq!(wide.census.by_knob["aisles"], Count::new(6));
    let world = world(&read, &wide);
    let robots = |role: &str| {
        world
            .entities
            .values()
            .filter(|entity| {
                entity.components.get("robot").is_some_and(|robot| {
                    let class = robot["robot_class"].as_str().unwrap();
                    world
                        .entities
                        .values()
                        .any(|e| e.id.to_string() == class && e.components["robot_class"]["role"] == role)
                })
            })
            .count()
    };
    let relays = world.entities.values().filter(|entity| entity.components.contains_key("relay")).count();
    eprintln!(
        "the L at 6 aisles and its width following to 56,100 mm: {relays} relays, {} VTMs, {} HTMs, {} floor cells",
        robots("vtm"),
        robots("htm"),
        world.cells.len()
    );
    assert_eq!((relays, robots("vtm"), robots("htm")), (192, 12, 24));
    assert_loads(&read, &wide, "the L story");
}
