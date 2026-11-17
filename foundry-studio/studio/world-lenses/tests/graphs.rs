//! Within `worldlens`'s tests, `graphs` is THE TWO INPUTS GIVING ONE ANSWER: a
//! pattern asked of a loaded world and asked of that world's entities alone,
//! and the demand that the two agree wherever they can agree at all.
//!
//! This is the claim the crate makes to a consumer that never loads a scenario
//! — a client handed a set of entities over a wire. If the answer it computes
//! for itself were not the answer the holder of the world computes, the whole
//! point of writing the predicates down once would be lost. So every pattern of
//! the shipped library is run both ways over every document, and the bindings
//! are compared as text.
//!
//! There is nowhere left for the two to disagree, and that is the point rather
//! than a gap in the file. Until 2026-09-16 a graph built from entities carried
//! no arcs and no rebuilt geometry, so some patterns could only be refused by
//! name; the relations that needed either were repealed, a `Graph` is now the
//! entities and nothing else, and both ways in build the same one. So this file
//! asks for agreement everywhere and accepts no refusal anywhere — and pins the
//! case that makes the cut worth its cost: a frame a generator stamped, which
//! no declaration states, is walked like any other.

mod fixtures;

use fixtures::{corners, documents, Fixture};
use std::collections::BTreeSet;
use worldlens::{Binding, Graph, Library, Matcher};
use site_model::EntityId;

const LIBRARY: &str = foundry_world_lenses::LEGACY_WPS_LENS;

fn library() -> Library {
    Library::parse(LIBRARY).expect("the shipped pattern library parses")
}

/// Bindings as text, so a disagreement reads as a disagreement rather than as a
/// length.
fn printed(bindings: &[Binding]) -> Vec<String> {
    bindings.iter().map(|binding| format!("{binding}")).collect()
}

/// A graph built from a world's entities alone answers every pattern exactly as
/// the world does — same bindings, same order, every pattern, no exceptions.
///
/// The entities are the only input, and every question this engine answers is a
/// question about them: which components an entity carries, what its fields
/// name, and how the derivation addresses nest. So there is nothing a world
/// carries that an entity set cannot answer from, and the two columns are the
/// one column.
#[test]
fn entities_alone_answer_what_a_world_answers() {
    let library = library();
    let mut compared = 0usize;
    for Fixture { name, world, .. } in documents() {
        let over_world = Matcher::new(&world.entities);
        let entities = Graph::of_entities(world.entities.clone());
        let over_entities = Matcher::over(&entities);

        for pattern in library.patterns() {
            assert_eq!(
                printed(&over_world.matches(pattern)),
                printed(&over_entities.matches(pattern)),
                "{name}: {} answers the same from entities alone",
                pattern.name()
            );
            compared += 1;
        }
    }
    assert!(compared >= 200, "only {compared} pattern-and-document pairs were compared");
}

/// A frame a generator made is walkable like any other, because the engine
/// reads what a document records rather than a table of the frames somebody
/// declared.
///
/// This is the change the cut of 2026-09-16 is for. The engine used to rebuild
/// a frame graph out of the DECLARED entities and leave every stamped
/// template's own frame out of it, so a chain that reached one stopped there —
/// and every stamped template has one. A multi-pass generator run projects
/// lenses over what earlier generators made, so an engine blind to a derived
/// frame could not run one.
///
/// The corner-case document stamps such a template once. The internal carries a
/// `frame` naming the declared floor as its parent, and the walk climbs from it
/// to the floor and on to the site — two entities the reader's own frame graph
/// would not have reached from there.
#[test]
fn a_frame_a_generator_made_is_walkable_like_any_other() {
    let Fixture { world, .. } = corners();
    let stamped: EntityId =
        "f0000000-0000-4000-8000-000000000008/instance:0/local:grid".parse().expect("the address parses");
    assert!(world.entities[&stamped].has("frame"), "expansion made an entity carrying a frame");
    assert!(
        world.frames.get(&stamped).is_none(),
        "and the reader's own frame graph, built from the declarations, does not hold it"
    );

    let library = Library::parse(r#"pattern above { f: frame, derived; up: frame; ref_walk(f, up, "frame.parent"); }"#)
        .expect("the pattern parses");
    let pattern = library.get("above").expect("the pattern");
    let over_world = Matcher::new(&world.entities);
    let rows = over_world.matches(pattern);
    assert_eq!(rows.len(), 2, "one derived frame, reaching two above it");
    for row in &rows {
        assert_eq!(row.get("f"), Some(&stamped), "the stamped internal is the only derived frame here");
    }
    let reached: BTreeSet<String> = rows.iter().filter_map(|row| row.get("up")).map(|id| id.to_string()).collect();
    assert_eq!(
        reached,
        BTreeSet::from([
            "22222222-2222-4222-8222-222222222222".to_string(),
            "11111111-1111-4111-8111-111111111111".to_string(),
        ]),
        "the floor it names, and the site above that"
    );

    // And the entities alone say the same, which is the claim read from the
    // other input.
    let rebuilt = Graph::of_entities(world.entities.clone());
    assert_eq!(printed(&Matcher::over(&rebuilt).matches(pattern)), printed(&rows));
}
