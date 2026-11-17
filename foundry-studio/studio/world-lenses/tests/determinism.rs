//! Within `worldlens`'s tests, `determinism` is THE PROMISE THAT ONE QUESTION
//! HAS ONE ANSWER: the same world and the same pattern give the same bindings,
//! in the same order, however the engine got there.
//!
//! The promise is worth testing because it is easy to lose by accident. The
//! engine indexes a world into maps, chooses a join order from candidate-set
//! sizes, and walks candidate lists — any of which could let a hash's iteration
//! order or an insertion order reach a result. So this file asks the question
//! twice over one world, and then asks it of the one choice the engine makes
//! that a caller never sees: the order the join visits variables in, which
//! decides which relation produces candidates and which merely filters them.
//! Each pattern is run under two orders this file picks and under the one the
//! planner picks, and the three have to agree.
//!
//! Writing a document's entities down in the opposite order is not a question
//! worth asking here: the reader sorts them into identity order before the
//! engine is handed anything, so a test of it would be a test of `Vec::reverse`.

mod fixtures;

use fixtures::{corners, document, Fixture};
use worldlens::{Binding, Library, Matcher, Pattern};
use site_model::pipeline::World;
use site_model::EntityId;

const LIBRARY: &str = foundry_world_lenses::LEGACY_WPS_LENS;

fn library() -> Library {
    Library::parse(LIBRARY).expect("the shipped pattern library parses")
}

/// Every binding of every pattern, printed, so two runs can be compared as text
/// and a difference reads as a difference rather than as a length.
fn answers(world: &World, library: &Library) -> Vec<String> {
    let matcher = Matcher::new(&world.entities);
    library
        .patterns()
        .flat_map(|pattern| {
            matcher.matches(pattern).into_iter().map(move |binding| format!("{}: {binding}", pattern.name()))
        })
        .collect()
}

/// Two lenses over one world answer identically, and each answer is sorted and
/// carries no binding twice.
#[test]
fn two_readings_of_one_world_agree() {
    let Fixture { world, .. } = document("cencora-v3");
    let library = library();
    assert_eq!(answers(&world, &library), answers(&world, &library));

    // The order is the reader's own order over identities, tuple by tuple in
    // the pattern's declaration order — which is not the alphabetical order of
    // the printed forms, because `EntityId` puts every declared identity before
    // every derived one.
    let matcher = Matcher::new(&world.entities);
    for pattern in library.patterns() {
        let bindings = matcher.matches(pattern);
        let keys: Vec<Vec<Option<EntityId>>> = bindings
            .iter()
            .map(|binding| pattern.variables().map(|variable| binding.get(variable).cloned()).collect())
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "{}: bindings come back in a total order", pattern.name());
        sorted.dedup();
        assert_eq!(sorted.len(), keys.len(), "{}: no binding twice", pattern.name());
    }
}

/// The order is the reader's order over identities and not the alphabetical
/// order of what they print as: a declared identity comes before every derived
/// one, whatever UUID either starts with. Saying so here means the rule the
/// README states is the rule a caller can rely on.
///
/// walmart-dallas is the document that states grids both ways: seven are
/// authored on the floor and two — the order queue and the waiting block — are
/// stamped out of the EasyLoop template, so the pattern binds a mixture and the
/// order it comes back in is a claim rather than an accident.
#[test]
fn declared_identities_sort_before_derived_ones() {
    let Fixture { world, .. } = document("walmart-dallas");
    let library = library();
    let bindings = Matcher::new(&world.entities).matches(library.get("cell_grid").expect("the pattern"));
    let declared = bindings.iter().filter(|b| !b.get("g").expect("bound").is_derived()).count();
    assert_eq!((declared, bindings.len()), (7, 9), "walmart-dallas states seven grids and stamps two");
    assert!(
        bindings[..declared].iter().all(|b| !b.get("g").expect("bound").is_derived()),
        "the declared grids come first"
    );
}

/// The five walked rows of the shipped library and what the minimal EasyLoop
/// map answers them, so the claim below is made over rows that exist.
///
/// `ref_walk` is the one relation whose join can produce candidates from EITHER
/// end, which makes it the one an order could reach — and over a document where
/// every walked row answers zero, three orders agreeing is three empty vectors
/// agreeing. The minimal EasyLoop map is the document in this tree that authors
/// a machine cell by cell, so it is the one where they have rows at all.
const WALKED: [(&str, usize); 5] = [
    ("machine_cell", 28),
    ("machine_pick_place", 2),
    ("machine_port", 2),
    ("machine_waiting_cell", 12),
    ("machine_port_posed", 2),
];

/// The plan the join follows does not reach the answers. Each pattern is run
/// under the planner's own order and under two the test chooses — the variables
/// in the order the pattern declares them, and in the reverse of that — and all
/// three give the same bindings in the same order.
///
/// The two chosen orders keep the one property an answer depends on: every
/// optional variable comes after every required one, because an optional
/// variable visited before what constrains it can go absent where it would have
/// bound. Everything else about a plan is a matter of how much work the join
/// does to reach the same set.
#[test]
fn the_order_the_join_visits_variables_in_does_not_reach_the_answers() {
    let library = library();
    let mut walked_rows: Vec<(String, usize)> = Vec::new();
    for Fixture { name, world, .. } in [corners(), document("minimal-putaway2"), document("minimal-easyloop")] {
        let matcher = Matcher::new(&world.entities);
        let (mut differing, mut planner_chose_another) = (0usize, 0usize);
        for pattern in library.patterns() {
            let declared = visiting_order(pattern, false);
            let reversed = visiting_order(pattern, true);
            if declared != reversed {
                differing += 1;
            }
            let planned_order = matcher.join_order(pattern);
            if planned_order != declared && planned_order != reversed {
                planner_chose_another += 1;
            }
            let forwards: Vec<&str> = declared.iter().map(String::as_str).collect();
            let backwards: Vec<&str> = reversed.iter().map(String::as_str).collect();
            let planned = matcher.matches(pattern);
            assert_eq!(
                printed(&planned),
                printed(&matcher.matches_with_join_order(pattern, &forwards)),
                "{name}: {} answers the same under the order it declares its variables in",
                pattern.name()
            );
            assert_eq!(
                printed(&planned),
                printed(&matcher.matches_with_join_order(pattern, &backwards)),
                "{name}: {} answers the same under the reverse of that order",
                pattern.name()
            );
            if name == "minimal-easyloop" && WALKED.iter().any(|(row, _)| *row == pattern.name()) {
                walked_rows.push((pattern.name().to_string(), planned.len()));
            }
        }
        // Not vacuous, and asked of each document rather than of whichever the
        // loop ended on: most of the shipped library declares more than one
        // required variable, and for those the two chosen orders really are two
        // orders.
        assert!(differing >= 15, "{name}: only {differing} patterns had two distinguishable orders");
        assert!(
            planner_chose_another > 0,
            "{name}: the planner picked one of the two orders every time, so nothing was varied"
        );
    }
    // And the walked rows really had rows to compare, so the agreement above is
    // an agreement about bindings rather than about emptiness.
    let mut wanted: Vec<(String, usize)> = WALKED.iter().map(|(row, n)| ((*row).to_string(), *n)).collect();
    wanted.sort();
    walked_rows.sort();
    assert_eq!(walked_rows, wanted, "what the minimal EasyLoop map answers the walked rows, under the planner's order");
}

/// A join order over one pattern's variables: required ones first, optional ones
/// last, each group in the order the pattern declares them or the reverse of it.
fn visiting_order(pattern: &Pattern, reversed: bool) -> Vec<String> {
    let mut order: Vec<String> = Vec::new();
    for optional in [false, true] {
        let mut group: Vec<String> = pattern
            .variables()
            .filter(|variable| pattern.is_optional(variable) == optional)
            .map(str::to_string)
            .collect();
        if reversed {
            group.reverse();
        }
        order.extend(group);
    }
    order
}

/// Bindings as text, so a disagreement reads as a disagreement rather than as a
/// length.
fn printed(bindings: &[Binding]) -> Vec<String> {
    bindings.iter().map(|binding| format!("{binding}")).collect()
}
